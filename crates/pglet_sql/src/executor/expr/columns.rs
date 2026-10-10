//! Column resolution: the `ColumnResolver` trait, `SimpleColumns` and the join column map.

use super::env::{EvalEnv, SubqueryExec};
use super::eval::eval_expr_full;
use crate::planner::{Expr, UsingMatch, match_names};
use crate::types::{ColumnDesc, TypeOid, Value};
use crate::{CatalogError, Error, Result};

// ============================================================================
// Column Resolution
// ============================================================================

/// Trait for resolving column references to row indices.
///
/// This allows the same expression evaluation logic to work with both
/// simple single-table queries and complex JOINs with table-qualified columns.
pub trait ColumnResolver {
    /// Resolve a column reference to an index into the row.
    fn resolve(&self, table: Option<&str>, name: &str) -> Result<usize>;

    /// Whether this scope's FROM clause includes a table/alias named `t`.
    ///
    /// Decides whether a *qualified* reference is inner (this scope owns it →
    /// resolve here) or correlated (defer to the outer scope).
    fn scope_has_table(&self, t: &str) -> bool;

    /// The column's **declared** `TypeOid`, when this scope knows it.
    ///
    /// Needed where a `Value` alone under-determines the rendering: `timestamptz`
    /// and `timestamp` share `Value::Timestamp`, and only the declared type says
    /// whether the text form carries the `+00` offset (see
    /// `value_to_text_with_oid`). `None` means "this resolver carries no type
    /// information" — callers then fall back to the value-only rendering.
    fn column_type(&self, _table: Option<&str>, _name: &str) -> Option<TypeOid> {
        None
    }

    /// The **ordered member labels** of the column's user `ENUM` type, when this
    /// scope knows the column declares one.
    ///
    /// The same under-determination [`Self::column_type`] closes, one type further:
    /// an enum value *is* a `Value::Text` and its `TypeOid` *is* `Text`, so neither
    /// says that `<`/`>`/`ORDER BY`/`MIN`/`MAX` over it order by the members'
    /// **positions** rather than the labels' bytes. The slice is the catalog's own
    /// `Schema.enums[name].labels` — borrowed, never copied, so there is one ordered
    /// member list and every ordering site reads it.
    ///
    /// `None` means "no enum here, or this resolver carries no enum information";
    /// the comparison then orders by the label bytes as any `text` pair does.
    fn column_enum_order(&self, _table: Option<&str>, _name: &str) -> Option<&[String]> {
        None
    }
}

/// One column of a **multi-relation** executor scope, addressed the way Postgres
/// addresses a join's namespace: by the `(relation, column)` **pair** rather than by a
/// synthesized `relation.column` string.
///
/// The string encoding it replaces is forgeable — a column a user quotes a dot into is
/// byte-identical to a name a join synthesized — and the forgery went both ways: a
/// relation the query never named became addressable, and the column the scan's own
/// alias would spell was never registered at all. See
/// `executor::query::scan`'s `plan_scope`, which derives these from the plan, and
/// `EDGES.md` for the measured pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRef {
    /// The relation a qualified reference must spell — the FROM alias when one was
    /// written, else the relation's own name. `None` for a column no qualifier can
    /// name: a `JOIN … USING` merge, which belongs to the join itself.
    pub relation: Option<String>,
    /// The column's own name, **verbatim** — never a synthesized spelling, so a dot a
    /// user quoted into a column name cannot manufacture a relation.
    pub name: String,
    /// Whether a **bare** reference can find this column. `false` for a side's `USING`
    /// column, which the merged column replaces in the join's own column list:
    /// `transformFromClauseItem` leaves the component RTEs qualifiable while hiding
    /// their columns from an unqualified lookup, which is why `SELECT c FROM a JOIN b
    /// USING (c)` is one column rather than `42702`.
    pub bare_visible: bool,
}

impl ColumnRef {
    /// A column of `relation`, findable both bare and qualified — the ordinary case.
    #[must_use]
    pub fn of(relation: &str, name: &str) -> Self {
        Self {
            relation: Some(relation.to_string()),
            name: name.to_string(),
            bare_visible: true,
        }
    }

    /// A `JOIN … USING` merged column: the join's own, so no qualifier names it, and
    /// bare-visible because it is what an unqualified reference must find.
    #[must_use]
    pub fn merged(name: &str) -> Self {
        Self {
            relation: None,
            name: name.to_string(),
            bare_visible: true,
        }
    }

    /// This column with bare visibility removed when `using` names it — the side copy
    /// the merged column stands in for.
    #[must_use]
    pub(in super::super) fn hidden_by_using(mut self, using: &[String]) -> Self {
        if using.contains(&self.name) {
            self.bare_visible = false;
        }
        self
    }
}

/// What a query level's FROM clause puts in scope, in the form a resolver over a node's
/// **input names** needs it ([`SimpleColumns::within_level`]). Built from the plan by
/// `executor::query::scan`'s `level_scope`; nothing here is read off a column name.
#[derive(Debug)]
pub struct LevelScope<'a> {
    /// The effective names of the level's FROM items — the qualifiers the level owns.
    pub names: Vec<&'a str>,
    /// The `(relation, column)` pair of each input column, positionally parallel to the
    /// input's names, where the node reads a join's rows directly (`plan_scope` over the
    /// input, kept only when it lines up with the executed columns); `None` where the
    /// input is one relation's rows or a projection's output, whose names are columns of
    /// no relation the scope can name.
    pub pairs: Option<Vec<ColumnRef>>,
}

/// What a scope knows about the relations behind its columns — the fact both
/// [`SimpleColumns`] and [`OuterFrame`](super::env::OuterFrame) switch their resolution rules on.
///
/// It is an enum rather than an `Option<&str>` because the two "no single table" cases
/// are **not** the same claim and used to share one spelling: a join scope, whose
/// columns really do belong to several named relations, and a scope with no relation
/// identity at all (the no-FROM / `VALUES` path, the post-projection `ORDER BY` /
/// aggregate / window resolvers). Collapsing them is what let a scope answer for
/// relations the statement never named — see [`Self::Anonymous`].
#[derive(Debug, Clone, Copy)]
pub enum ScopeIdentity<'a> {
    /// No relation identity. A qualified reference cannot be validated against a FROM
    /// namespace here, so it falls to the loose bare/suffix match, and a synthesized
    /// `"t.c"` name is offered to a qualified lookup — the forgeable behaviour, kept
    /// because it is what a scope whose relations could not be derived still gets.
    Anonymous,
    /// Exactly one relation, under its effective name (its FROM alias when one was
    /// written). A qualifier naming anything else is `42P01`.
    Single(&'a str),
    /// Several relations, addressed by `(relation, column)` pairs. No string can forge
    /// a pair, so a qualified reference resolves exactly and a bare one counts exact
    /// claimants over the bare-visible columns.
    Relations(&'a [ColumnRef]),
}

impl<'a> ScopeIdentity<'a> {
    /// The identity a caller holding only an effective table name has.
    #[must_use]
    pub const fn from_table(table: Option<&'a str>) -> Self {
        match table {
            Some(t) => Self::Single(t),
            None => Self::Anonymous,
        }
    }

    /// This scope's single relation name, or `None` for the other two — what a caller
    /// that still speaks in `Option<&str>` reads.
    #[must_use]
    pub const fn single(&self) -> Option<&'a str> {
        match self {
            Self::Single(t) => Some(t),
            _ => None,
        }
    }

    /// Whether `t` names a relation of this scope.
    #[must_use]
    fn has_relation(&self, t: &str) -> bool {
        match self {
            Self::Anonymous => false,
            Self::Single(n) => *n == t,
            Self::Relations(refs) => refs.iter().any(|r| r.relation.as_deref() == Some(t)),
        }
    }
}

/// Simple column resolver for single-table queries.
///
/// Resolves an unqualified column by name; resolves a *qualified* column
/// alias-awarely against `table`, the scan's **effective** name (its FROM-clause
/// alias if present, else the real table name). `table` is `None` for the
/// no-FROM / `VALUES` path, which has no single-table identity and so preserves
/// the bare-only resolution behavior.
pub struct SimpleColumns<'a> {
    names: &'a [&'a str],
    /// What this scope knows about the relations behind `names` — one relation, several
    /// addressed by pair, or none (see [`ScopeIdentity`]).
    identity: ScopeIdentity<'a>,
    /// Declared type per column, positionally parallel to `names`, or empty when
    /// the caller has no descriptor list to hand (see
    /// [`ColumnResolver::column_type`]).
    types: &'a [TypeOid],
    /// Each column's enum member order, positionally parallel to `names`, or empty
    /// when the caller has none to hand (see [`ColumnResolver::column_enum_order`]).
    enum_orders: &'a [Option<&'a [String]>],
    /// The query level this scope evaluates in, when the scope's own identity cannot say
    /// it — `None` otherwise (see [`Self::within_level`]).
    level: Option<&'a LevelScope<'a>>,
}

impl<'a> SimpleColumns<'a> {
    /// Resolver with no single-table identity — a *qualified* ref then resolves
    /// by the loose (bare/suffix) match, since there is no FROM scope to validate
    /// the qualifier against. Used on the no-FROM / `VALUES` path, for a stored
    /// expression over one table's columns, and — told their level's FROM names by
    /// [`Self::within_level`] — by the post-projection `ORDER BY` / `DISTINCT ON` /
    /// aggregate / window resolvers (over the node's input names, which carry no
    /// relation identity).
    pub const fn new(names: &'a [&'a str]) -> Self {
        Self {
            names,
            identity: ScopeIdentity::Anonymous,
            types: &[],
            enum_orders: &[],
            level: None,
        }
    }

    /// Resolver bound to an effective single-table name, enabling alias-aware
    /// resolution of a qualified reference.
    pub const fn with_table(names: &'a [&'a str], table: Option<&'a str>) -> Self {
        Self {
            names,
            identity: ScopeIdentity::from_table(table),
            types: &[],
            enum_orders: &[],
            level: None,
        }
    }

    /// Resolver over the scope `identity` describes — including
    /// [`ScopeIdentity::Relations`], a multi-relation namespace addressed by
    /// `(relation, column)` pairs. The pairs are positionally parallel to `names`; the
    /// names stay for the callers that still read them (the descriptors a client sees,
    /// the outer frame's own addressing), but nothing resolves through them there.
    #[must_use]
    pub const fn over_scope(names: &'a [&'a str], identity: ScopeIdentity<'a>) -> Self {
        Self {
            names,
            identity,
            types: &[],
            enum_orders: &[],
            level: None,
        }
    }

    /// Attach the scope's declared column types (positionally parallel to
    /// `names`), so [`ColumnResolver::column_type`] can answer. Callers that hold
    /// the input `ColumnDesc` list — the projection and `RETURNING` paths — thread
    /// them through; the rest leave them empty and get the value-only rendering.
    #[must_use]
    pub const fn with_types(mut self, types: &'a [TypeOid]) -> Self {
        self.types = types;
        self
    }

    /// Attach each column's enum member order (positionally parallel to `names`), so
    /// [`ColumnResolver::column_enum_order`] can answer. Threaded by every executor
    /// node that holds its input `ColumnDesc` list, beside `with_types` and derived
    /// from the same descriptors (`executor::column_enum_orders`); a caller with no
    /// descriptors leaves it empty and every enum in that scope orders lexically.
    #[must_use]
    pub const fn with_enum_orders(mut self, enum_orders: &'a [Option<&'a [String]>]) -> Self {
        self.enum_orders = enum_orders;
        self
    }

    /// Tell this scope the names of its query level's FROM items, so a qualifier the level
    /// owns is resolved **here** and never handed to an enclosing frame
    /// ([`ColumnResolver::scope_has_table`]).
    ///
    /// For a resolver over a node's *output* names — the `Sort`, `DistinctOn`,
    /// `Aggregate` and `Window` nodes evaluate keys, aggregate arguments, `HAVING` and
    /// the grouped or windowed select list over their input's names, which carry no
    /// relation identity. Without the level's names, `scope_has_table` was false there
    /// for every qualifier, and where an enclosing level's FROM item carried the same
    /// name `resolve_column` took that frame's value: over `t (y, n)` holding `10`, `20`
    /// and `ev (n)` holding `5`, `7`, `SELECT y, (SELECT max(o.n) FROM ev o) FROM t o`
    /// answered `10`, `20` against 18.6's `7`, `7`, and the same binding moved a `DELETE`'s
    /// row set and an `INSERT … SELECT`'s stored values. Postgres's
    /// `refnameNamespaceItem` searches the levels innermost first and stops at the first
    /// carrying the name.
    ///
    /// A qualifier the level owns then binds a column of that relation, decided by the
    /// level's structure rather than any column name's text: over a join's rows, the
    /// input's `(relation, column)` pair ([`LevelScope::pairs`]); otherwise the bare name,
    /// exactly — never a suffix match, which is another relation's column (see `resolve`).
    /// Above a projection the names are output names, and an owned qualifier binds
    /// whichever carries the name (`EDGES.md` row 325). The level comes from
    /// `executor::query::scan`'s `level_scope`.
    #[must_use]
    pub const fn within_level(mut self, level: &'a LevelScope<'a>) -> Self {
        self.level = Some(level);
        self
    }

    /// This scope's enum member order for a column reference, at the **scope's** own
    /// lifetime rather than the borrow's.
    ///
    /// [`ColumnResolver::column_enum_order`]'s body, and the reason it is here: a
    /// caller that resolves the order once and keeps it — the `ORDER BY` comparator,
    /// which sees only `Value`s — builds a temporary resolver, so the trait method's
    /// `&self`-tied answer would not outlive it. Same `resolve` either way, so the
    /// order lands on exactly the column the value came from.
    #[must_use]
    pub fn scope_enum_order(&self, table: Option<&str>, name: &str) -> Option<&'a [String]> {
        *self.enum_orders.get(self.resolve(table, name).ok()?)?
    }
}

impl SimpleColumns<'_> {
    /// Resolve against a scope whose `(relation, column)` pairs are known — the join
    /// namespace, Postgres's own.
    ///
    /// A **qualified** reference is an exact pair match: a qualifier the scope names but
    /// whose column it lacks is `42703 "t.name"`, and one it does not name at all is
    /// `42P01`. A **bare** one counts exact claimants over the *bare-visible* columns
    /// (`scanRTEForColumn`): one resolves, two or more are `42702`, none is `42703`.
    ///
    /// Nothing here reads a column's *name string* as a `relation.column` spelling, which
    /// is the whole point — a name a user quoted a dot into is an ordinary column here.
    fn resolve_by_ref(refs: &[ColumnRef], table: Option<&str>, name: &str) -> Result<usize> {
        if let Some(t) = table {
            if let Some(i) = refs
                .iter()
                .position(|r| r.relation.as_deref() == Some(t) && r.name == name)
            {
                return Ok(i);
            }
            if refs.iter().any(|r| r.relation.as_deref() == Some(t)) {
                return Err(CatalogError::ColumnNotFoundInExpr {
                    column: format!("{t}.{name}"),
                }
                .into());
            }
            return Err(CatalogError::MissingFromClauseEntry {
                table: t.to_string(),
            }
            .into());
        }
        let mut found = None;
        for (i, r) in refs.iter().enumerate() {
            if r.bare_visible && r.name == name {
                if found.is_some() {
                    return Err(Error::AmbiguousColumn {
                        column: name.to_string(),
                    });
                }
                found = Some(i);
            }
        }
        found.ok_or_else(|| {
            CatalogError::ColumnNotFoundInExpr {
                column: name.to_string(),
            }
            .into()
        })
    }
}

impl ColumnResolver for SimpleColumns<'_> {
    fn resolve(&self, table: Option<&str>, name: &str) -> Result<usize> {
        if let ScopeIdentity::Relations(refs) = self.identity {
            return Self::resolve_by_ref(refs, table, name);
        }
        if let Some(t) = table {
            // The `"t.name"` spelling, built only for an error: the resolver runs per row,
            // and the tiers below compare without allocating it.
            let qn = || format!("{t}.{name}");
            // Whether `n` is the dotted spelling `"t.name"`.
            let is_qualified_name =
                |n: &str| n.strip_prefix(t).and_then(|r| r.strip_prefix('.')) == Some(name);
            // Tier 1: the qualifier IS our effective single table → match the
            // bare name. This is the alias case (`s.open_signup`, `c.refs`).
            if self.identity.single() == Some(t) {
                if let Some(i) = self.names.iter().position(|n| *n == name) {
                    return Ok(i);
                }
                // The qualifier names our table but the column is absent → a
                // genuine 42703; no loose fallback (it would mask a real error).
                return Err(CatalogError::ColumnNotFoundInExpr { column: qn() }.into());
            }
            // A qualifier this scope's query level owns (`Self::within_level`) names one
            // relation, and binds a column of it by the level's structure, never by a
            // column name's text: over a join's rows, the input's `(relation, column)`
            // pair; over one relation's rows or a projection's output, the bare name,
            // exactly. Ahead of tier 2, whose `"t.name"` spelling a user's quoted
            // `"o.n"` column forges. Over a projection's output the name belongs to no
            // relation the scope can name, so the bare match binds whichever output
            // carries it (`EDGES.md` row 325).
            if let Some(level) = self.level
                && level.names.contains(&t)
            {
                let found = level.pairs.as_ref().map_or_else(
                    || self.names.iter().position(|n| *n == name),
                    |pairs| {
                        pairs
                            .iter()
                            .position(|r| r.relation.as_deref() == Some(t) && r.name == name)
                    },
                );
                return found
                    .ok_or_else(|| CatalogError::ColumnNotFoundInExpr { column: qn() }.into());
            }
            // Tier 2: exact qualified match against prefixed storage names (the
            // JOIN case, where columns are stored as `"users.name"`).
            //
            // Gated on `self.table.is_none()`, and **after** the alias tier, because
            // the dotted name a join synthesizes is byte-identical to a column a user
            // quoted into existence as `"a.b"` — nothing in the name distinguishes
            // them. Consulted first and ungated, the synthesized-name tier let such a
            // column hijack a qualified reference that meant something else entirely:
            // `SELECT a.b FROM cl AS a` returned the `"a.b"` column instead of alias
            // `a`'s column `b`, and `SELECT a.b FROM dq` returned it where Postgres
            // raises `42P01`.
            //
            // `self.table` is the marker, and what it tests is exact: a scope that
            // *has* an effective table takes its names straight from that table, so
            // they are never dotted, and a dotted name reaching it is a user's
            // spelling. A qualified reference into such a scope that doesn't name
            // that table is therefore an unknown qualifier, and falls to tier 3's
            // `42P01` exactly as Postgres does.
            //
            // The converse does **not** hold, and reading it as if it did is how this
            // area goes wrong: `table: None` does not mean *join*. It also arrives
            // from the no-FROM / `VALUES` path and the post-projection resolvers
            // three paragraphs down — and it used to arrive from an unaliased UPDATE
            // and from a FROM-clause `unnest(…)`, both single-relation scopes with a
            // perfectly good name, where tier 3's loose match then answered a bogus
            // qualifier: over `ex1(n, v)`, `UPDATE ex1 SET v = 42 WHERE bogusrel.n = 1`
            // **wrote the row**, and `SELECT zzz.b FROM unnest(ARRAY['x','y']) AS v(b)`
            // answered `b`, against 18.6's `42P01` to both. Those two call sites now
            // pass a name; the ones listed at tier 3 still do not.
            //
            // `EvalEnv::resolve_outer_slot` resolves a correlated reference by the
            // same two tiers, in the same order, under the same `is_none()` gate —
            // and it carried the order without the gate, which is not the same
            // alignment: its dotted tier answered a qualified reference against a
            // frame whose relation it does not name, the `DELETE` that destroyed a
            // row quoted beside that gate.
            if self.identity.single().is_none()
                && let Some(i) = self.names.iter().position(|n| is_qualified_name(n))
            {
                return Ok(i);
            }
            // Tier 3: no exact spelling answered.
            if self.identity.single().is_some() {
                // This resolver has a single-table identity (a scan / `WHERE` /
                // `RETURNING` scope), so the qualifier is genuinely unknown → a
                // `42P01` (`missing FROM-clause entry for table "<qualifier>"`,
                // Postgres's `errorMissingRTE`), *never* the old silent loose
                // bare/suffix match (which returned a different column than the
                // one the qualifier named — the silent-wrong-column bug). A
                // correlated outer reference never reaches this arm: `resolve_
                // column` consults the outer `EvalEnv` frame chain before this
                // final resolve. Mirrors `JoinColumnMap::resolve`.
                return Err(CatalogError::MissingFromClauseEntry {
                    table: t.to_string(),
                }
                .into());
            }
            // No single-table identity, and a qualifier this scope's level does not own
            // (an owned one was settled above): the no-FROM / `VALUES` path, and a
            // resolver over a node's input names when no enclosing frame claims the
            // qualifier either (`resolve_column`'s `Unclaimed`). The qualifier can't be
            // validated against a FROM scope here, so the loose bare/suffix match
            // resolves it. A genuinely-unknown qualifier in one of these positions is a
            // live divergence, and not only an ordering one: reached through the aggregate
            // path it decides rows, since `validate_grouped_expr` waves an unknown
            // qualifier through and the `HAVING` / projection then resolves it here
            // (`SELECT ikey FROM gi2 GROUP BY ikey HAVING bogusrel.tag = 'z'` returns the
            // row where 18.6 is `42P01`; see `EDGES.md` for the measured set).
            // TODO: validate column qualifiers at plan time against each query
            // scope's FROM namespace.
            if let Some(i) = self
                .names
                .iter()
                .position(|n| *n == name || n.strip_suffix(name).is_some_and(|r| r.ends_with('.')))
            {
                return Ok(i);
            }
            return Err(CatalogError::ColumnNotFoundInExpr { column: qn() }.into());
        }
        // Unqualified: the same tiered exact-then-suffix match the `USING`
        // validation uses (`match_names`). A merged `USING` column is a bare name
        // (an exact match) prepended ahead of the raw dotted per-side columns, so
        // tier 1 returns it before the suffix tier sees `a.<c>` / `b.<c>` — USING
        // stays unambiguous. An unqualified name matching >1 relation of a join is
        // `42702` (Postgres), not a silent first-match.
        match match_names(self.names.iter().copied(), name) {
            UsingMatch::Found(i) => Ok(i),
            UsingMatch::Ambiguous => Err(Error::AmbiguousColumn {
                column: name.to_string(),
            }),
            UsingMatch::Missing => Err(CatalogError::ColumnNotFoundInExpr {
                column: name.to_string(),
            }
            .into()),
        }
    }

    fn scope_has_table(&self, t: &str) -> bool {
        // The relation namespace this scope knows: its single table's effective name,
        // or — for a join scope — any relation its pairs name; and, for a scope over a
        // node's output names, its query level's FROM items.
        self.identity.has_relation(t) || self.level.is_some_and(|l| l.names.contains(&t))
    }

    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        // Reuse `resolve` so the type lands on exactly the column the value came
        // from (alias tiers included); `types` is empty unless a caller attached it.
        self.types.get(self.resolve(table, name).ok()?).copied()
    }

    fn column_enum_order(&self, table: Option<&str>, name: &str) -> Option<&[String]> {
        self.scope_enum_order(table, name)
    }
}

/// One column of a [`JoinColumnMap`]: where it sits in the joined row, and the two
/// declared facts a `Value` under-determines (see [`ColumnResolver::column_type`] and
/// [`ColumnResolver::column_enum_order`]).
///
/// The enum order is **owned** here, unlike [`SimpleColumns`]'s borrowed slice: a
/// `JoinColumnMap` outlives the borrow of the catalog its builders hold.
pub struct JoinColumn {
    /// The relation a qualified reference must spell (the FROM alias when written).
    pub table: String,
    /// The column's own name.
    pub name: String,
    /// Its position in the joined row.
    pub index: usize,
    /// Its declared type.
    pub type_oid: TypeOid,
    /// Its enum's ordered member labels, when it declares one.
    pub enum_order: Option<Vec<String>>,
}

/// Column metadata for JOIN evaluation.
/// Maps (`table_name`, `column_name`) to a [`JoinColumn`].
pub struct JoinColumnMap {
    /// All columns, in row order.
    pub columns: Vec<JoinColumn>,
}

impl JoinColumnMap {
    /// Create a new column map.
    pub const fn new() -> Self {
        Self {
            columns: Vec::new(),
        }
    }

    /// Register one column at `index`, resolvable as `table.name`.
    pub fn add_column(
        &mut self,
        table: &str,
        name: &str,
        index: usize,
        type_oid: TypeOid,
        enum_order: Option<Vec<String>>,
    ) {
        self.columns.push(JoinColumn {
            table: table.to_string(),
            name: name.to_string(),
            index,
            type_oid,
            enum_order,
        });
    }

    /// Add a relation's columns from their descriptors, indexed from `base_idx`,
    /// resolving each one's enum order against `schema` — the same derivation
    /// `SimpleColumns`'s callers take (`executor::column_enum_orders`).
    pub fn add_columns(
        &mut self,
        table: &str,
        cols: &[ColumnDesc],
        base_idx: usize,
        enums: &std::collections::BTreeMap<String, crate::catalog::EnumType>,
    ) {
        for (i, c) in cols.iter().enumerate() {
            self.add_column(
                table,
                &c.name,
                base_idx + i,
                c.type_oid,
                super::super::helpers::enum_order_of(enums, c.enum_type.as_deref())
                    .map(<[String]>::to_vec),
            );
        }
    }

    /// Find a column by name (optionally qualified with table).
    // The `0` (not found) and `_` (ambiguous) arms share a `None` body but are kept
    // separate so the ambiguity case stays legible — `resolve` re-counts the bare
    // matches to raise `42702` for it, so `None` here means "no single match".
    #[allow(clippy::match_same_arms)]
    pub fn find(&self, table: Option<&str>, name: &str) -> Option<(usize, TypeOid)> {
        self.find_column(table, name).map(|c| (c.index, c.type_oid))
    }

    /// The single [`JoinColumn`] a reference names, by the same rules as [`Self::find`]
    /// — its one implementation, so the index, the declared type and the enum order can
    /// never land on different columns.
    pub fn find_column(&self, table: Option<&str>, name: &str) -> Option<&JoinColumn> {
        // If table qualifier is given, match exactly
        if let Some(tbl) = table {
            return self
                .columns
                .iter()
                .find(|c| c.table == tbl && c.name == name);
        }

        // No table qualifier - match by name (error if ambiguous)
        let mut matches = self.columns.iter().filter(|c| c.name == name);
        let first = matches.next()?;
        // Ambiguous - would need better error handling
        matches.next().is_none().then_some(first)
    }
}

impl Default for JoinColumnMap {
    fn default() -> Self {
        Self::new()
    }
}

impl ColumnResolver for JoinColumnMap {
    fn resolve(&self, table: Option<&str>, name: &str) -> Result<usize> {
        if let Some((idx, _)) = self.find(table, name) {
            return Ok(idx);
        }
        // An out-of-scope *table qualifier* is a 42P01 (`missing FROM-clause
        // entry for table "<qualifier>"`), distinct from a bad *column* on an
        // in-scope table (42703) — matching Postgres. The comma-vs-JOIN scope
        // split (JOIN binds tighter than comma) surfaces this: `FROM c, a JOIN b
        // ON c.num = b.num` reaches here with `c` outside the inner join's scope.
        // A correlated outer reference never lands here — `resolve_column`
        // consults the outer frame before this final resolve for a not-in-scope
        // qualifier.
        if let Some(t) = table {
            if !self.scope_has_table(t) {
                return Err(CatalogError::MissingFromClauseEntry {
                    table: t.to_string(),
                }
                .into());
            }
        } else if self.columns.iter().filter(|c| c.name == name).count() >= 2 {
            // A bare column in a JOIN `ON` predicate (e.g. `… ON num > 0`) matching
            // more than one joined relation is ambiguous — `find` returned `None` for
            // the ≥2 case (it can't pick a side). Postgres: `42702`. A single match
            // returned `Ok` above; zero falls through to the `42703` below.
            return Err(Error::AmbiguousColumn {
                column: name.to_string(),
            });
        }
        Err(CatalogError::ColumnNotFoundInExpr {
            column: table.map_or_else(|| name.to_string(), |t| format!("{t}.{name}")),
        }
        .into())
    }

    fn scope_has_table(&self, t: &str) -> bool {
        // A join scope owns a qualifier if any of its joined inputs is named `t`.
        self.columns.iter().any(|c| c.table == t)
    }

    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        // A join map already carries each column's type alongside its index.
        self.find_column(table, name).map(|c| c.type_oid)
    }

    fn column_enum_order(&self, table: Option<&str>, name: &str) -> Option<&[String]> {
        self.find_column(table, name)?.enum_order.as_deref()
    }
}

/// Evaluate an expression using a `JoinColumnMap` for column resolution.
///
/// This is a convenience wrapper around `eval_expr_full` for JOIN queries.
/// Does not support subqueries (`SubqueryExec::NONE`).
pub fn eval_expr_join(
    expr: &Expr,
    col_map: &JoinColumnMap,
    row: &[Value],
    env: EvalEnv<'_>,
) -> Result<Value> {
    eval_expr_full(expr, col_map, row, SubqueryExec::NONE, env)
}
