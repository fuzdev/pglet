//! Plan node types for query execution.

use crate::catalog::{
    CatalogTable, ColumnDefault, IdentityGeneration, IndexKey, ReferentialAction, Schema,
};
use crate::executor::{DropTarget, IndexMethod, owned_sequences};
use crate::types::{ColumnDesc, ResultShape, TypeOid};
use serde::{Deserialize, Serialize};

use super::common_type::{EnumIdentity, ResolvedType};
use super::expr::{Expr, OrderKey};
use super::function_scan::FromFunction;

/// The effective resolvable name of a scan node — its FROM-clause `alias` if
/// present, else the real `table` name.
///
/// The single definition of "a scan's name", so the sites that compute it (planner
/// lowering + param typing, the join/scan executors) can't drift — a divergent copy
/// here is a column-resolution bug class (the kind the alias-aware scoping work
/// fixed). Applies to every scan variant (`Scan`/`IndexScan`/`IndexRangeScan`/`GinScan`).
#[must_use]
pub fn scan_effective_name(table: &str, alias: &Option<String>) -> String {
    alias.clone().unwrap_or_else(|| table.to_string())
}

/// The relation namespace an expression's `Column` leaves resolve their types
/// against — the second argument of [`Expr::infer_type`].
///
/// There are three, they answer differently, and picking the wrong one is a
/// declared-type bug rather than a missing one: the walk always produces an answer
/// (`to_column_desc` substitutes `text`, and an arithmetic node hands back the
/// *other* operand's type), so a scope that cannot see a column yields a confident
/// wrong OID over correct bytes. That is a binary client's decode failure.
///
/// - [`Self::Catalog`] keys the `Schema` by the column's own qualifier and then by
///   an optional fallback name. It has no FROM clause behind it: a stored `CHECK`
///   expression against its own table, a no-FROM `VALUES` row, a `$N` typed from the
///   column it is compared with.
/// - [`Self::Input`] resolves against the plan node's **input relations**, through
///   the same [`Plan::find_projected_column`] a bare projected column already uses.
///   It is the right scope for every projecting node, because a FROM-clause alias
///   (`FROM mw m`) names a relation the catalog does not have — and a `SubqueryScan`
///   or `CteScan` names one the catalog *cannot* have. The relations arrive
///   **already materialized** ([`RelationColumns`]), computed by the same walk that
///   produced `columns`; see that type for why re-deriving them from the plan node
///   was an exponential. A DML statement's `RETURNING` list uses it too, over
///   [`returning_relations`] — the target relation plus its `FROM` sources.
///
/// `RETURNING` used to pass `Catalog(Some(<target>))`, and the doc here used to
/// call that "exactly" right. It is not a namespace at all — a qualifier is looked
/// up **in the whole database** and only an unmatched one falls back to the target,
/// so a `Column` leaf resolved against a relation the statement never mentions.
/// `DELETE FROM ra WHERE id = 999 RETURNING rb.big + 0` declared 18.6's `int8` on
/// this walker and `int4` on the executor's, for a query 18.6 refuses outright
/// (`42P01`); `RETURNING zzz.b + 0` did the same by falling through to the target's
/// own `b`. Both are now bound at plan time (`lower_returning`), so a leaf reaching
/// this scope has already resolved in it.
///
/// The catalog scope's fallback name used to be fed the scan's *effective* name
/// (its alias when one was written), which reads as alias support and is the
/// opposite: `schema.get_table("m")` is `None`, so `SELECT m.s + m.i FROM mw m`
/// declared `text` while sending four `int4` bytes, and `SELECT m.b + 0 FROM mw m`
/// declared `int4` — the other operand's type — over eight `int8` bytes. Merely
/// writing an alias on the FROM item was enough; the columns did not have to be
/// qualified.
#[derive(Debug, Clone, Copy)]
pub enum TypeScope<'a> {
    /// The catalog, under an optional fallback table name for unqualified columns.
    Catalog(Option<&'a str>),
    /// A plan node's input relations, plus the descriptors that node's evaluator
    /// binds values through.
    Input {
        /// The relations the input introduces, by the name a qualifier must spell.
        relations: &'a RelationColumns,
        /// `input.output_columns(schema)`, materialized once by the caller.
        columns: &'a [ColumnDesc],
        /// The projecting node's record of the references an enclosing level binds
        /// ([`EnclosingColumns`]), where the scope is one.
        enclosing: Option<&'a EnclosingColumns>,
    },
}

impl TypeScope<'_> {
    /// The catalog scope with no fallback table — a no-FROM expression.
    pub const NONE: Self = Self::Catalog(None);

    /// The type of `[qualifier.]name` in this scope, or `None` when the scope
    /// cannot resolve it.
    pub(super) fn column_type(
        self,
        schema: &Schema,
        table: Option<&str>,
        name: &str,
    ) -> Option<TypeOid> {
        match self {
            Self::Catalog(fallback) => {
                let table_def = table
                    .and_then(|t| schema.get_table(t))
                    .or_else(|| fallback.and_then(|t| schema.get_table(t)))?;
                table_def
                    .columns
                    .iter()
                    .find(|c| c.name == name)
                    .map(|c| c.type_oid)
            }
            Self::Input {
                relations,
                columns,
                enclosing,
            } => Plan::find_projected_column(relations, columns, enclosing, table, name)
                .map(|c| c.type_oid),
        }
    }

    /// [`Self::column_type`] for a reference that **binds** in this scope — 18.6's level
    /// walk, whose qualified reference binds at the innermost level naming its qualifier.
    /// A qualifier no relation of this scope answers to is an enclosing level's, so its
    /// type is not this scope's to answer — not a catalog table's the qualifier spells, nor
    /// a projecting node's record of its enclosing references (`EDGES.md` row 159). A bare
    /// name binds here where the scope carries it.
    pub(super) fn own_column_type(
        self,
        schema: &Schema,
        table: Option<&str>,
        name: &str,
    ) -> Option<TypeOid> {
        let foreign = table.is_some_and(|qualifier| match self {
            Self::Catalog(fallback) => fallback != Some(qualifier),
            Self::Input { relations, .. } => relations.lookup(qualifier).is_none(),
        });
        if foreign {
            return None;
        }
        self.column_type(schema, table, name)
    }

    /// The **enum identity** of `[qualifier.]name` in this scope — [`Self::column_type`]'s
    /// other field, read off the same catalog row or the same descriptor.
    ///
    /// The `Column` leaf of this scope's `EnumLeaves` ([`PlannerScope::expr_enum`]). No
    /// `TypeOid` names an enum, so a rulebook reading only the OID cannot tell `mood`
    /// from `text` nor from another enum (`EDGES.md` rows 114 and 115).
    pub(super) fn column_enum(
        self,
        schema: &Schema,
        table: Option<&str>,
        name: &str,
    ) -> Option<String> {
        match self {
            Self::Catalog(fallback) => {
                let table_def = table
                    .and_then(|t| schema.get_table(t))
                    .or_else(|| fallback.and_then(|t| schema.get_table(t)))?;
                table_def
                    .columns
                    .iter()
                    .find(|c| c.name == name)
                    .and_then(|c| c.enum_type.as_ref())
                    .cloned()
            }
            Self::Input {
                relations,
                columns,
                enclosing,
            } => Plan::find_projected_column(relations, columns, enclosing, table, name)
                .and_then(|c| c.enum_type),
        }
    }
}

/// What the declared-type walk asks of a **leaf**.
///
/// [`Expr::infer_type`] answers 31 of its 33 variants from the node and its children
/// alone. Exactly two ask something of the outside world: a `Column` needs a namespace,
/// and a `Subquery` needs its plan's first output column. This trait is those two
/// questions — and having them behind it is what lets the walk stop carrying a
/// `&Schema`, since the catalog becomes one implementation's private business instead
/// of an argument every recursive call threads and every caller must have.
///
/// That matters because the catalog is exactly what the *other* declared-type walk
/// does not have. `executor::helpers`'s `expr_type_oid` answers the same two questions
/// from a projecting node's already-materialized `ColumnDesc`s, and being unable to
/// spell that difference is why the two walks were separate code — two open-coded rule
/// tables over the same 33 variants, which have drifted and, worse, have been
/// identically wrong (the hazard no pair-comparison can see; `docs/compatibility.md`'s Arithmetic
/// row carries the measured instance).
///
/// Both scopes implement this, so a rule that reads only the leaves can now be written
/// once: [`shared_leaf_cell_type`](super::shared_leaf_cell_type) is `Default`,
/// `Parameter` and `Subquery` — the three variants where that is *all* it takes — plus
/// `StringFn`'s `DateTrunc` **member**, whose variant keeps an arm on each walk for its
/// other members. Four more are merged into
/// [`shared_multi_operand_type`](super::shared_multi_operand_type) — `Array`,
/// `VariadicFn`, `Coalesce`, `Case` — where the two arms differed only through the
/// recursion, each walk passing its own operand reader. Every other variant stays
/// open-coded per walk for now, and what
/// bounds any further merge is the second half of the split — not the scope but what an
/// un-inferable operand is, since the executor walk is total (`TypeOid::Text`) where this
/// one answers `None`, a policy decided per rule against the oracle and never carried by
/// a refactor. Which of those actually bounds is a smaller set than the number of
/// differing arms, and it is measured there rather than restated here.
///
/// **Dispatch is dynamic on purpose, and the cost was measured rather than argued.** A
/// generic parameter would monomorphize the whole recursive walk once per scope type
/// and the wasm build is size-capped, while the vtable is reached only at the two
/// leaves — the arithmetic and branch recursion below them is an ordinary static call.
/// Against a generic spelling of the same walk the difference did not surface, and the
/// instrument's bounds are worth stating because they bound the claim: a `cargo test`
/// (unoptimized) build, five unwarmed iterations of the two `Describe` stress shapes at
/// depths 7 and 30, with parse, plan and `output_columns` folded into one timer and the
/// mean reported. That supports "no difference this instrument can see" and not a
/// per-millisecond figure, and the generic spelling no longer exists to re-measure.
/// What is standing rather than historical is narrower than the instrument: the two
/// `describe_over_a_*_is_not_exponential` tests run under `cargo test` at depth **7**
/// against a 10-second bound, and the property they hold is that the walk is **not
/// exponential** in depth — a quadratic walk would pass them; the depth-30 runs were
/// the instrument's own, not a gate.
///
/// `RelationColumns` names the other half of that cost contract: an implementation of
/// this trait must answer `column_type` from a namespace it already holds, **never** by
/// descending a `&Plan` to re-derive one. This signature is what makes that
/// unrepresentable rather than merely forbidden — `column_type` is handed no plan.
pub trait LeafScope {
    /// The type of `[table.]name` in this scope, or `None` when the scope cannot
    /// resolve it. `None` is always safe — it is what an un-inferable operand is
    /// spelled as on this walk.
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid>;

    /// The type of a scalar subquery's first output column, or `None` when it has none.
    fn subquery_type(&self, plan: &Plan) -> Option<TypeOid>;
}

/// The planner's [`LeafScope`]: a [`TypeScope`] paired with the catalog it resolves
/// against.
///
/// The pairing is not incidental — [`TypeScope::Catalog`] *is* a lookup in the schema,
/// so the two were always one object passed as two arguments.
#[derive(Debug, Clone, Copy)]
pub struct PlannerScope<'a> {
    schema: &'a Schema,
    scope: TypeScope<'a>,
}

impl<'a> PlannerScope<'a> {
    /// `scope` resolved against `schema`.
    #[must_use]
    pub const fn new(schema: &'a Schema, scope: TypeScope<'a>) -> Self {
        Self { schema, scope }
    }

    /// The catalog scope with no fallback table — a no-FROM expression.
    #[must_use]
    pub const fn none(schema: &'a Schema) -> Self {
        Self::new(schema, TypeScope::NONE)
    }

    /// The catalog scope under an optional fallback table name for unqualified
    /// columns.
    #[must_use]
    pub const fn catalog(schema: &'a Schema, table: Option<&'a str>) -> Self {
        Self::new(schema, TypeScope::Catalog(table))
    }
}

impl LeafScope for PlannerScope<'_> {
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        self.scope.column_type(self.schema, table, name)
    }

    /// A scalar subquery is typed by the same walk one level down — `output_columns`
    /// is `infer_type`'s own caller — so this is a recursion through the plan, not a
    /// namespace derivation, and the cost contract above does not bite.
    fn subquery_type(&self, plan: &Plan) -> Option<TypeOid> {
        plan.output_columns(self.schema).first().map(|c| c.type_oid)
    }
}

impl PlannerScope<'_> {
    /// The **enum identity** an expression carries in this scope — the catalog-side
    /// instantiation of `executor::expr`'s one exhaustive enum walk.
    ///
    /// The twin of `planner::coercion`'s clause-side `expr_enum_in`, and deliberately
    /// the *same walk*: the DDL-time operand rulebook and the plan-time one must read
    /// one arm list, or a `CHECK` would be judged by a rule the query over the same
    /// columns is not.
    #[must_use]
    pub fn expr_enum(&self, expr: &Expr) -> Option<String> {
        crate::executor::expr_enum_with(
            expr,
            &crate::executor::EnumLeaves {
                column: &|table, name| self.scope.column_enum(self.schema, table, name),
                subquery: &|plan| crate::executor::subquery_enum_name(plan, self.schema),
            },
        )
    }

    /// [`TypeScope::own_column_type`] in this scope: `[table.]name`'s type where the
    /// reference binds here, `None` where its qualifier names an enclosing level.
    #[must_use]
    pub fn own_column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        self.scope.own_column_type(self.schema, table, name)
    }

    /// One column's enum identity in this scope — the leaf behind [`Self::expr_enum`],
    /// exposed for the readers that walk a sub-plan's projection with their own leaves.
    #[must_use]
    pub fn column_enum(&self, table: Option<&str>, name: &str) -> Option<String> {
        self.scope.column_enum(self.schema, table, name)
    }

    /// [`Self::expr_enum`] with the **subquery leaf declining** — the instantiation
    /// `Expr::to_column_desc` builds a descriptor with. That leaf is the only difference:
    /// the same walk, the same column leaf, the same arm list.
    ///
    /// **Declining is a cost bound, and both sides of it are measured.** This walk runs
    /// *inside* `Plan::output_columns`, which `executor::subquery_enum_name` is a caller
    /// of, so a descending leaf re-enters — and `Expr::infer_type` is already descending
    /// there, making two `output_columns` walks per level where there was one. Over a
    /// projecting nest (`SELECT (…) FROM t WHERE id = 1` per level) on a release server,
    /// declining is **flat** — 32 / 32 / 31 / 33 ms at depth 12 / 16 / 18 / 20 — and
    /// descending is 50 / 316 / 1165 / 4507 ms at the same four, doubling per level, with
    /// depth 22 at ~17 s against 18.4's 39 ms. The per-row shape does not separate them
    /// (`SELECT count(*) FROM big WHERE m > (SELECT m FROM u WHERE id = 1)` over 20 000
    /// rows is 76–80 ms either way, medians of three), so nesting depth is the whole of
    /// the difference and it is the reason this declines.
    ///
    /// What it costs is one measured shape:
    /// `(SELECT (SELECT min(m) FROM u) FROM u) > 'sad'` is `f` here against 18.6's `t`,
    /// which with the no-FROM spelling beside it is `EDGES.md` row 107's residual. The
    /// **gate** path does not pay it, `enum_identity_in` asking
    /// `planner::coercion`'s `resolved_subquery_type_in` rather than a descriptor.
    #[must_use]
    pub fn expr_enum_shallow(&self, expr: &Expr) -> Option<String> {
        crate::executor::expr_enum_with(
            expr,
            &crate::executor::EnumLeaves {
                column: &|table, name| self.scope.column_enum(self.schema, table, name),
                subquery: &|_| None,
            },
        )
    }

    /// The identity a gate reads for `expr` in this scope — the catalog-side twin of
    /// `planner::coercion`'s `enum_identity_in`, over the same
    /// [`erases_enum_identity`](super::coercion::erases_enum_identity) predicate.
    #[must_use]
    pub fn enum_identity(&self, expr: &Expr) -> EnumIdentity {
        if super::coercion::erases_enum_identity(expr) {
            return EnumIdentity::Undecided;
        }
        self.expr_enum(expr)
            .map_or(EnumIdentity::NotAnEnum, EnumIdentity::Named)
    }

    /// `expr`'s declared type paired with that identity — the [`ResolvedType`] the
    /// comparison and common-type rulebooks take.
    #[must_use]
    pub fn resolved_type(&self, expr: &Expr) -> Option<ResolvedType> {
        expr.infer_type(self).map(|oid| ResolvedType {
            oid,
            enum_type: self.enum_identity(expr),
        })
    }
}

/// The relations a plan node puts in scope for the nodes above it: each one's output
/// columns under the name a qualifier must spell (its FROM alias when it has one, else
/// its table name).
///
/// The descriptor-level twin of the coercion pass's [`ColumnScope`](super::ColumnScope),
/// and the reason it is a **value carried out of the walk** rather than a function over
/// the plan node. A `qualifier.c` reference needs the named relation's columns, and
/// re-deriving them by descending the node cost one full inner `output_columns` **per
/// qualified leaf**: a nest of `L` qualified leaves per level cost `(L + 1) ^ depth`, so
/// `Describe` over 8 levels of 8 leaves took 42 s where 18.4 takes 31 ms, and a nest of
/// two-sided joins was the same curve with no wrapper node in it at all. Returning the
/// relations alongside the columns makes the whole walk one bottom-up pass — each node
/// builds its list from its children's *already computed* lists, and a qualified leaf is
/// a lookup.
///
/// It also cannot go stale the way a shape test can. The previous fix short-circuited
/// the lookup when the input **was** a `SubqueryScan`/`CteScan` whose alias matched, and
/// a single `Filter` between them (one per-level `WHERE`) put the plan back on the
/// exponential. Here every arm of [`Plan::output_columns_scoped`] must say what it puts
/// in scope, so a new node cannot silently omit itself — the compiler asks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RelationColumns(Vec<(String, Vec<ColumnDesc>)>);

impl RelationColumns {
    /// A node that introduces no relation of its own — every node that reshapes rows
    /// into something no qualifier can name (a `Project`, an `Aggregate`, a set
    /// operation, a `VALUES`).
    #[must_use]
    const fn none() -> Self {
        Self(Vec::new())
    }

    /// The single relation `name` with `columns` — a scan, a CTE reference, a derived
    /// table.
    #[must_use]
    pub(super) fn one(name: String, columns: Vec<ColumnDesc>) -> Self {
        Self(vec![(name, columns)])
    }

    /// This node's relations followed by `other`'s — a join's two sides, in that order.
    #[must_use]
    pub(super) fn chain(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        self
    }

    /// The columns of the relation named `qualifier`, or `None` when nothing here
    /// answers to that name.
    ///
    /// **First match wins**, which is a join's left side over its right — the order the
    /// per-node descent used (`left.or_else(right)`) and the order the executor's
    /// `JoinColumnMap` resolves in.
    #[must_use]
    pub(super) fn lookup(&self, qualifier: &str) -> Option<&[ColumnDesc]> {
        self.0
            .iter()
            .find(|(name, _)| name == qualifier)
            .map(|(_, columns)| columns.as_slice())
    }

    /// Every relation in FROM order, each with its columns in declaration order —
    /// what a `*` expansion walks.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&str, &[ColumnDesc])> {
        self.0
            .iter()
            .map(|(name, columns)| (name.as_str(), columns.as_slice()))
    }

    /// Every relation's columns concatenated in FROM order — the flat descriptor list
    /// a bare (unqualified) name is looked up in.
    #[must_use]
    pub(super) fn flatten(&self) -> Vec<ColumnDesc> {
        self.0.iter().flat_map(|(_, c)| c.iter().cloned()).collect()
    }
}

/// The relations a `FROM` source introduces, by the name a qualified reference must
/// spell, each with its columns in declaration order.
///
/// Read by everything that must agree about what an `UPDATE … FROM` source is called
/// and what it carries: `push_plan_relations` (the `WHERE` / `SET` scope, which drops
/// the order), and — through [`returning_relations`] — `lower_returning` (the
/// `RETURNING` binding gate **and** its wildcard expansion, which needs the order),
/// [`Plan::returning_output_columns`], and the parameter-type walk. The executor
/// derives the same relations from its own materialized rows
/// (`get_table_name_from_plan` + `project_returning`), which is a second derivation and
/// is why the two are pinned against each other by fixture rather than by sharing code.
///
/// # It must be exhaustive over the kinds a source can lower to
///
/// In the **open** `WHERE` / `SET` scope an unregistered relation can only *withhold* a
/// type, and withholding declines the coercion and the gate alike (see
/// [`ColumnScope`](super::ColumnScope)) — a missing arm costs a false rejection at
/// worst. In `RETURNING`'s **closed** scope it withholds a *claimant*, and a withheld
/// claimant makes a bare name look unambiguous and **answer a row**. The two readings
/// point opposite ways, so the walk is written to the stricter one.
///
/// That set is `parse_table_factor`'s whole range, and it is five: a base [`Plan::Scan`],
/// a [`Plan::VirtualScan`] (a catalog view), a [`Plan::CteScan`], a
/// [`Plan::SubqueryScan`] (a derived table or a FROM-clause `VALUES`), and a
/// [`Plan::FunctionScan`] (`unnest`, `generate_series`). Postgres's own answers, measured
/// on 18.6, are `42702` for a bare `RETURNING` name each of the last two claims —
/// `FROM information_schema.tables c … RETURNING table_name` and
/// `FROM unnest(ARRAY['x','y']) AS v … RETURNING v` — where pglet answered the target's
/// column.
///
/// It deliberately does **not** descend a `Join`: an `UPDATE … FROM` source is a single
/// table factor, so neither a row-shaping node (`Filter`, `Project`, …) nor a join can
/// appear between here and the leaf. Every multi-relation spelling is rejected upstream
/// — `FROM a JOIN b ON …` and its parenthesized form as `42601` (`parse_derived_table`
/// requires a `SELECT` inside the parens), a comma-separated from-list as `0A000` — so a
/// join arm would be unreachable code standing in for a shape the parser never builds.
///
/// **Exhaustive over kinds is not sufficient over claimants, and the `VirtualScan` arm is
/// where that bites.** The claimants a relation contributes are the names it would
/// *resolve*, which for a synthesized catalog view is `CatalogTable::columns()` — a
/// strict subset of 18.6's list for nine of the 17 relations pglet models.
/// `EDGES.md` carries the measured counts.
///
/// # Why this is not [`Plan::output_columns_scoped`]'s relation half
///
/// A `Scan` naming a table the catalog lacks registers **nothing** here, where
/// `output_columns_scoped` registers an empty relation, and the difference is
/// load-bearing. In a closed scope it decides which error a qualified reference to the
/// missing source gets: an empty relation answers `42703` at plan time, where the
/// relation being absent lets the executor's own `42P01` for the missing table win — the
/// answer 18.6 gives. A `FunctionScan` registers under its alias in both.
pub(super) fn plan_relations(plan: &Plan, schema: &Schema) -> RelationColumns {
    match plan {
        Plan::Scan { table, alias } if schema.get_table(table).is_some() => RelationColumns::one(
            scan_effective_name(table, alias),
            table_columns_desc(schema, table),
        ),
        // A catalog view has no `Table` in the schema, so its columns come from the
        // same synthesized list `collect_nonbase_into` reads, keyed the same way (its
        // alias, else the view's bare name — `information_schema.tables` is spelled
        // `tables`).
        Plan::VirtualScan {
            catalog_table,
            alias,
        } => RelationColumns::one(
            alias
                .clone()
                .unwrap_or_else(|| catalog_table.name().to_string()),
            catalog_table.columns(),
        ),
        Plan::CteScan { alias, columns, .. } => {
            RelationColumns::one(alias.clone(), columns.clone())
        }
        Plan::SubqueryScan {
            subquery,
            alias,
            column_aliases,
            cte_rank: _,
        } => RelationColumns::one(
            alias.clone(),
            alias_relation_columns(subquery.output_columns(schema), column_aliases),
        ),
        // A FROM-clause function: its column names carry Postgres's
        // `chooseScalarFunctionAlias` rule (see `parse_unnest_table`) and its types
        // `function_scan_column_types`'s, both already computed by `output_columns`.
        Plan::FunctionScan { alias, .. } => {
            RelationColumns::one(alias.clone(), plan.output_columns(schema))
        }
        _ => RelationColumns::none(),
    }
}

/// Whether every relation this plan names exists in the catalog — the precondition a
/// **plan-time** gate standing in for a Postgres *parse-analysis* error must hold.
///
/// Postgres resolves the range table before it transforms anything else
/// (`transformFromClause` ahead of the target list, `setTargetTable` ahead of both), so
/// a statement naming a relation that does not exist is `42P01` whatever else is wrong
/// with it. pglet raises that at **execution**, which means any plan-time verdict
/// pre-empts it — and the assignment-cast gate does so with a type nothing in the
/// statement has, because [`Plan::output_columns`] substitutes `text` for a column of a
/// relation it cannot find. So each of those gates asks this first and declines for the
/// whole statement when the answer is `false`.
///
/// **It has to be the whole plan and not the immediate node**, which is what
/// [`plan_relations`] answers: measured on 18.6, `INSERT INTO t (i) SELECT n FROM
/// nosuchrel` and its `ORDER BY 1` / `LIMIT 1` / `DISTINCT` / derived-table / one-side-
/// of-a-join spellings are all `42P01` there, and each reaches the missing `Scan`
/// through a different chain of wrapper nodes. Expressions are walked too
/// ([`expr_relations_resolvable`]), because a scalar subquery carries a range table of
/// its own: `INSERT INTO t (b) VALUES ((SELECT n FROM nosuchrel))` and the `SET`
/// spelling of it were the same pre-emption reached without any FROM clause at all.
///
/// **The unlisted arms answer `true`, and that is the safe polarity rather than a
/// claim of completeness.** A node kind this match does not descend hides a missing
/// relation from the gate, which leaves a *loud* wrong SQLSTATE (`42804` where 18.6
/// says `42P01`) — never a silent write. A new query-plan node with a sub-plan belongs
/// in the list.
pub(super) fn plan_relations_resolvable(plan: &Plan, schema: &Schema) -> bool {
    let sub = |p: &Plan| plan_relations_resolvable(p, schema);
    let ex = |e: &Expr| expr_relations_resolvable(e, schema);
    match plan {
        Plan::Scan { table, .. } => schema.get_table(table).is_some(),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { table, .. }
        | Plan::IndexRangeScan { table, .. }
        | Plan::GinScan { table, .. } => schema.get_table(table).is_some(),
        Plan::Values { rows, .. } => rows.iter().flatten().all(ex),
        Plan::Filter { input, predicate } => sub(input) && ex(predicate),
        Plan::Project { input, exprs, .. } => sub(input) && exprs.iter().all(ex),
        Plan::ProjectUnnest { exprs } => exprs.iter().all(ex),
        Plan::Sort {
            input, order_by, ..
        } => sub(input) && order_by.iter().all(|k| ex(&k.expr)),
        Plan::Distinct { input } | Plan::Limit { input, .. } | Plan::JunkFilter { input, .. } => {
            sub(input)
        }
        Plan::DistinctOn { input, on_exprs } => sub(input) && on_exprs.iter().all(ex),
        Plan::Aggregate {
            input,
            group_by,
            exprs,
            having,
            ..
        } => {
            sub(input)
                && group_by.iter().all(ex)
                && exprs.iter().all(ex)
                && having.as_ref().is_none_or(ex)
        }
        Plan::Window {
            input,
            window_exprs,
            ..
        } => sub(input) && window_exprs.iter().all(ex),
        Plan::Join {
            left, right, on, ..
        } => sub(left) && sub(right) && ex(on),
        Plan::SetOp { left, right, .. } => sub(left) && sub(right),
        Plan::SubqueryScan { subquery, .. } => sub(subquery),
        Plan::FunctionScan { args, .. } => args.iter().all(ex),
        _ => true,
    }
}

/// [`plan_relations_resolvable`] over an expression: `false` when a subquery inside it
/// names a relation the catalog lacks.
///
/// The three plan-carrying variants are named explicitly because
/// [`Expr::for_each_child`] treats a self-contained sub-`Plan` as a **leaf** — it walks
/// scalar children only, so the `InSubquery` arm below hands it the left-hand side and
/// this function takes the plan.
pub(super) fn expr_relations_resolvable(expr: &Expr, schema: &Schema) -> bool {
    match expr {
        Expr::Subquery { plan } | Expr::Exists { plan, .. } | Expr::InSubquery { plan, .. }
            if !plan_relations_resolvable(plan, schema) =>
        {
            return false;
        }
        _ => {}
    }
    let mut resolvable = true;
    expr.for_each_child(|child| {
        resolvable = resolvable && expr_relations_resolvable(child, schema);
    });
    resolvable
}

/// The `RETURNING` namespace of a DML statement: the target relation under its
/// effective name (`relation` — its alias when one was written), then every relation
/// its `FROM` introduces, in FROM order.
///
/// One derivation. The readers known to consume it are the plan-time binding gate and
/// wildcard expansion (`lower_returning`), the planner-declared type walk
/// ([`Plan::returning_output_columns`]), and the prepared-statement parameter inference
/// (`Inference::walk_returning`). The executor's projection reaches the same relations
/// by its **own** derivation, off its materialized rows (`get_table_name_from_plan` +
/// `project_returning`) — it has no `Schema`-side list to read — so that pair is held
/// together by fixture, not by this function. The correlated re-walk of an
/// `UPDATE`/`DELETE` level is another derivation: it hands that level's `RETURNING`
/// items the level's own scope (`correlation::level_scope`), not this function's list.
///
/// **The reader list is the part to re-derive, not to inherit.** It was written here as
/// "all four" while the parameter walk was still reading
/// `TypeScope::Catalog(Some(<target>))`, which is a catalog lookup rather than a
/// namespace: over `dt(id, b BIGINT)` with `ds(id, b INT)` aliased `s`,
/// `RETURNING (s.b = $1)` reported `$1` as `int8` where 18.6 says `int4`. A count is
/// the claim that goes stale silently, because nothing recomputes it.
pub(super) fn returning_relations(
    table: &str,
    relation: &str,
    from: Option<&Plan>,
    schema: &Schema,
) -> RelationColumns {
    let mut relations =
        RelationColumns::one(relation.to_string(), table_columns_desc(schema, table));
    if let Some(source) = from {
        relations = relations.chain(plan_relations(source, schema));
    }
    relations
}

/// The `ON CONFLICT DO UPDATE` action's namespace: the insert target under its own
/// name, then the `EXCLUDED` pseudo-relation carrying the target's shape.
///
/// The [`RelationColumns`] spelling of the namespace
/// [`on_conflict_action_scope`](super::dml::on_conflict_action_scope) builds as a
/// [`ColumnScope`](super::ColumnScope) — one namespace, two types, because the lowering
/// needs a name→type map with claimant counting and the parameter walk needs ordered
/// [`ColumnDesc`]s. The two are deliberately adjacent in their doc comments: a change to
/// what the action can name has to move both, and only the pairing says so.
///
/// **`excluded` is the action's alone.** Measured on 18.6, the arbiter cannot name it —
/// `ON CONFLICT ((excluded.id)) DO UPDATE …` and `ON CONFLICT (id) WHERE excluded.n > 0
/// DO UPDATE …` are both `42P01 invalid reference to FROM-clause entry for table
/// "excluded"` — so `Inference::walk_on_conflict` sets this scope around the action's
/// two positions and not around the arbiter's.
///
/// Registering the target under its **own** name and not an effective one is a
/// statement about the *parser*, not about the rule: `gram.y`'s `insert_target` does
/// take an `AS ColId`, and 18.6 accepts `INSERT INTO t AS x … DO UPDATE SET v = x.v`
/// where pglet's parser is `42601` (measured) — so no `INSERT` reaching here carries a
/// target alias. That is the same measured gap
/// [`insert_returning_relations`](super::dml::insert_returning_relations) records for
/// the other `INSERT`-side namespace, and closing it in the parser means revisiting
/// both.
pub(super) fn on_conflict_action_relations(table: &str, schema: &Schema) -> RelationColumns {
    let columns = table_columns_desc(schema, table);
    RelationColumns::one(table.to_string(), columns.clone())
        .chain(RelationColumns::one("excluded".to_string(), columns))
}

/// One side of a [`Plan::IndexRangeScan`]'s key range.
///
/// The planner normalizes every range conjunct to `col OP value`, flipping a
/// value-first spelling (`5 < k` → `k > 5`), so `inclusive` and the lower/upper slot
/// describe the *normalized* form while `column_first` remembers the written one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexBound {
    /// The row-independent bound value (`Literal`/`Parameter`/`Bound`), evaluated
    /// once against an empty row at execute time.
    pub value: Box<Expr>,
    /// `>=` / `<=` → inclusive; `>` / `<` → exclusive.
    pub inclusive: bool,
    /// Whether the user wrote the column on the **left** (`col < v`) rather than
    /// flipped (`v > col`). Diagnostics only — it never changes which rows the scan
    /// returns, only the operand order and operator spelling an operand-type
    /// rejection reports, so the message matches the predicate as written (and
    /// matches what the stripped build's `Filter(Scan)` prints for the same query).
    pub column_first: bool,
}

/// The target-list positions a clause's keys **name**, under Postgres's
/// `findTargetlistEntrySQL92` rule — a positional ordinal, or a bare single-field name
/// matching an output column — and, for a key that rule does not take, the first entry the
/// key equals (`findTargetlistEntrySQL99`, `super::query`'s `sql99_target_index`).
///
/// Postgres's `addTargetToSortList` / `addTargetToGroupList` coerce a *named* entry's
/// `unknown` constant to `text` while leaving every other entry alone, so which entries
/// a `GROUP BY` / `ORDER BY` / `DISTINCT ON` claims is an observable fact about a
/// query: `INSERT INTO t (b, i) SELECT 't', 5 ORDER BY 2` is accepted on 18.6 and its
/// `ORDER BY 1` spelling is `42804`. `super::dml`'s `target_list_source_types` is the
/// reader, for an `INSERT … SELECT` source and for a set operation's side alike.
///
/// **It is recorded at the lowering because the lowering is the last place that knows
/// it.** Both clauses rewrite the key into the entry it named — a sort-below-projection
/// `ORDER BY 1` becomes the projection expression itself
/// (`rewrite_output_key_to_projection`), a `GROUP BY 1` becomes the target entry
/// (`group_by_target_entry`) — and two entries can be equal, so no walk over the
/// finished plan can recover the **position** from the rewritten key. Reading it back by
/// expression equality marks every equal entry, which refuses
/// `INSERT INTO t (txt, b) SELECT 't', 't' FROM s GROUP BY 1` — accepted on 18.6 — and
/// the obvious repair, taking the first entry that matches, then accepts the
/// `GROUP BY 2` spelling and **writes** the row 18.6 refuses. Positions are the only
/// answer that gets both. (The `findTargetlistEntrySQL99` match is the first equal entry
/// on 18.6 itself, so the claims of a key it takes are that rule and not this repair.)
///
/// A **window** key is not recorded here: it stays inside its `Expr::Window` rather than
/// being rewritten into an entry, so `super::dml`'s `window_key_claims` reads it off the
/// finished target list.
///
/// Empty means "this node claims no entry", which is also what every node that is not a
/// query level's own sort or grouping carries (a set-op's trailing `ORDER BY`, whose
/// entries the set operation has already resolved).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedTargets(Vec<usize>);

impl ResolvedTargets {
    /// The claims of a key list, each key's claimed position or `None`.
    #[must_use]
    pub(super) fn from_claims(claims: impl IntoIterator<Item = Option<usize>>) -> Self {
        Self(claims.into_iter().flatten().collect())
    }

    /// Whether the entry at `index` is one this node names.
    #[must_use]
    pub(super) fn claims(&self, index: usize) -> bool {
        self.0.contains(&index)
    }
}

/// The reason a [`Plan`] decode is refused — see [`Plan`]'s `Deserialize`.
///
/// **postcard does not carry it.** Its `Error` has no payload and renders every
/// `serde::de::Error::custom` as "Serde Deserialization Error", so this text documents
/// the intent in-source and reaches a caller only under a format that keeps custom
/// messages. A store holding plan bytes therefore surfaces as ordinary corruption.
const PLAN_NOT_PERSISTED: &str = "a Plan is never persisted: these bytes claim a plan-bearing Expr, \
     which no accepted DDL can store";

/// The type the coercion pass read each of a node's column-defining expressions at **as
/// written**, by position, and `None` where it typed none.
///
/// A position is a target-list entry, a `VALUES` column or a FROM `unnest`'s array, and
/// the reader is `planner::coercion`'s `certain_entry_type`.
///
/// The record of a computed column's type certainty, written while the plan is built
/// and read where the node's columns are derived ([`Plan::output_columns`]): a column is
/// described certain ([`ColumnDesc::type_certain`]) where the type the descriptor
/// declares is the type recorded for it here. An entry appended after the pass ran — a
/// junk column — has no record, and so is not certain.
///
/// The expression is read before the pass rewrites it because the rewritten form can
/// read as typed where the written one did not: a branch set the pass leaves at its
/// written reading casts a branch it could not type to a sibling's type, a guess, and
/// the pass types a branch subquery with a cast to a type its sub-plan need not vouch
/// for — each a typed branch to any later reader.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CertainTypes(Vec<Option<TypeOid>>);

impl CertainTypes {
    /// The record over `types`, one per position.
    #[must_use]
    pub const fn new(types: Vec<Option<TypeOid>>) -> Self {
        Self(types)
    }

    /// Whether position `index` was read as written at exactly `type_oid`.
    #[must_use]
    pub fn holds(&self, index: usize, type_oid: TypeOid) -> bool {
        self.0.get(index).copied().flatten() == Some(type_oid)
    }

    /// The record for position `index`, for a caller that rewrites the expression there
    /// into one whose type it knows to be certain.
    pub fn set(&mut self, index: usize, type_oid: Option<TypeOid>) {
        if self.0.len() <= index {
            self.0.resize(index + 1, None);
        }
        self.0[index] = type_oid;
    }
}

/// The identity of each column reference in a level's target list that an **enclosing**
/// level binds — type and enum, by spelling.
///
/// Written by the correlated re-run, the first point the enclosing levels exist, and read
/// where the level's columns are derived ([`Plan::output_columns`]) and where the executor
/// builds the same node's descriptors.
///
/// A target-list reference has one identity: the column 18.6's level walk binds it to.
/// The level's own input cannot say what that is for an enclosing level's column, so
/// without this a sub-plan's output descriptor took a same-named local column's identity,
/// or `text` (`EDGES.md` row 159), for every reader of it. A reference the record does
/// not hold is undecided, and the descriptor declines rather than read a local column.
///
/// Shared (not copied) by a clone, as [`SetOpOuterTypes`] is: the judgement of a nested
/// level runs on a copy at the statement's own clause walk
/// (`planner::correlation::judge_nested_level`), and its record is what the readers
/// between that walk and the statement's own re-run read. Each re-run rewrites it whole.
#[derive(Clone, Default)]
pub struct EnclosingColumns(std::sync::Arc<EnclosingState>);

/// One recorded reference: its qualifier, beside the descriptor of the column it binds to
/// (named as the reference is).
pub type EnclosingColumn = (Option<String>, ColumnDesc);

/// The recorded descriptor of `[qualifier.]name` among `entries`.
#[must_use]
pub fn find_enclosing<'a>(
    entries: &'a [EnclosingColumn],
    qualifier: Option<&str>,
    name: &str,
) -> Option<&'a ColumnDesc> {
    entries
        .iter()
        .find(|(q, c)| q.as_deref() == qualifier && c.name == name)
        .map(|(_, c)| c)
}

#[derive(Default)]
struct EnclosingState {
    /// Whether `entries` holds any — read without the lock, because a level's columns are
    /// derived far more often than its record is written, and nearly every record is empty.
    any: std::sync::atomic::AtomicBool,
    entries: std::sync::Mutex<Vec<EnclosingColumn>>,
}

impl EnclosingColumns {
    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<EnclosingColumn>> {
        self.0
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replace the record with `entries`, each a reference's qualifier beside the
    /// descriptor of the column it binds to (named as the reference is).
    pub(super) fn record(&self, entries: Vec<EnclosingColumn>) {
        if entries.is_empty() && self.is_empty() {
            return;
        }
        let mut held = self.entries();
        self.0
            .any
            .store(!entries.is_empty(), std::sync::atomic::Ordering::Release);
        *held = entries;
    }

    /// Whether no reference is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.0.any.load(std::sync::atomic::Ordering::Acquire)
    }

    /// The recorded identity of `[qualifier.]name`, or `None` where none is recorded.
    #[must_use]
    pub fn lookup(&self, qualifier: Option<&str>, name: &str) -> Option<ColumnDesc> {
        if self.is_empty() {
            return None;
        }
        find_enclosing(&self.entries(), qualifier, name).cloned()
    }

    /// A copy of the recorded entries, for a reader that borrows them across many lookups
    /// — empty, with nothing allocated, where none is recorded.
    #[must_use]
    pub fn entries_copy(&self) -> Vec<EnclosingColumn> {
        if self.is_empty() {
            return Vec::new();
        }
        self.entries().clone()
    }
}

/// Its entries are not printed: a plan's `Debug` reaches an error message, and the record
/// is derived from what that message already prints.
impl std::fmt::Debug for EnclosingColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnclosingColumns")
    }
}

/// **Excluded from a plan's equality**, as [`SetOpOuterTypes`] is: the record is derived
/// from the expressions beside it and the levels around them, so two nodes the gates call
/// the same expression carry the same identities whichever handle each holds.
impl PartialEq for EnclosingColumns {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// A projecting node's target list with the records beside it, as
/// [`Plan::project_output_columns`] reads them.
#[derive(Clone, Copy)]
struct Targets<'a> {
    exprs: &'a [Expr],
    certain: &'a CertainTypes,
    enclosing: &'a EnclosingColumns,
}

/// Logical plan node.
///
/// **A `Plan` serializes but never deserializes**, and the asymmetry is the point.
/// It carries `Serialize` because [`Expr`] does — `Expr::Subquery` / `InSubquery` /
/// `Exists` each box a `Plan`, and `Expr` is both persisted (stored `CHECK` /
/// `DEFAULT` / index-predicate / index-key trees) and, in memory, encoded as an
/// identity by `planner::coercion`'s `structural_identity`, which **must** succeed
/// over a plan-bearing tree (18.6 accepts `(SELECT 1) AS q, (SELECT 1) AS q` under a
/// bare `ORDER BY q` and rejects `(SELECT 1)`/`(SELECT 2)`; both verdicts are those
/// bytes compared). Its `Deserialize` is hand-written and **always fails**: no
/// accepted DDL can store a plan-bearing `Expr` — `parser::ddl`'s
/// `reject_persisted_expr_constructs` refuses a subquery in a CHECK / index predicate
/// / index expression (`0A000`) and `executor::ddl`'s `check_default_expr_nodes` does
/// the same for a column `DEFAULT` — so bytes claiming one are a corrupt store, not a
/// tree to rebuild.
///
/// Closing the read direction on the *type* rather than at each reader is what makes
/// it hold without an enumeration to keep current: every persisted-bytes reader
/// (`planner::deserialize_expr`, and the four `postcard::from_bytes::<Schema>` sites
/// across `pglet`, `pglet_wasm`, `pglet_bench` and `pglet_testing`) inherits the
/// refusal, at no per-commit cost.
///
/// There is no matching write-side closure. [`super::serialize_expr`] refuses one, but
/// its single production caller is the catalog → storage `IndexSpec` hand-off, so what
/// it covers is the *storage* copy of an index key expression or predicate. The catalog
/// copy of every route — CHECK, DEFAULT, index key and predicate alike — rides
/// `catalog::serialize_schema`'s ungated `Schema` encode. Those rest on **two**
/// independent pre-write refusals — `parser::ddl`'s ranked walk (or `executor::ddl`'s
/// positional one for a `DEFAULT`) and the stored-expression cook's probe eval, which
/// has no execution context for a sub-plan — and on this refusal at open. That last one
/// is the consequence worth knowing: if both ever go missing, the plan-bearing tree is
/// **written**, and the store then fails to open at all rather than carrying a CHECK
/// nobody can evaluate.
///
/// It is also the artifact's largest single size lever: the derived
/// `Plan::deserialize` was the second-largest function in the wasm, and dropping it
/// takes the `Plan`-only sub-types' decoders with it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Plan {
    /// Return literal values.
    Values {
        rows: Vec<Vec<Expr>>,
        /// The type each **column** is certainly of ([`CertainTypes`]): a no-`FROM`
        /// select's entry as it was read as written, a `VALUES` list's column where
        /// `unify_values_columns` found every cell so.
        #[serde(skip)]
        certain: CertainTypes,
        /// The identity of each reference an enclosing level binds ([`EnclosingColumns`])
        /// — every one a no-`FROM` level makes.
        #[serde(skip)]
        enclosing: EnclosingColumns,
    },

    /// Scan a table.
    ///
    /// `alias` is the FROM-clause alias (`FROM cell c` → `Some("c")`), retained
    /// so column resolution can disambiguate a qualified reference by its alias
    /// (the storage scan still keys on the real `table`). `None` for an unaliased
    /// scan, where the table name is itself the resolution qualifier.
    Scan {
        table: String,
        alias: Option<String>,
    },

    /// Index-accelerated point read: probe a single-column secondary index for
    /// the rows whose `key_column` equals `key_value`, instead of a full scan.
    ///
    /// Planned in place of `Filter(Scan)` when the WHERE clause is (or AND-contains)
    /// an equality `indexed_col = <row-independent value>` and a single-column index
    /// covers `(table, key_column)`. Any residual conjuncts stay in a wrapping
    /// `Filter`, so the node returns a superset of the final rows: the executor
    /// probes the index, fetches each matching row, and the outer `Filter` (if any)
    /// applies the rest. Output columns mirror `Scan` (the table's columns).
    ///
    /// `key_value` is row-independent by construction (a literal, a `$N`
    /// parameter, or a bound value — never another column), evaluated once against
    /// an empty row at execute time. `index_oid` is the catalog index's OID (cast
    /// to the storage `index_id`); `alias` mirrors `Scan`'s for resolution.
    #[cfg(feature = "index_acceleration")]
    IndexScan {
        table: String,
        alias: Option<String>,
        /// The catalog index's OID (`Index.oid`); cast to `u32` for the storage probe.
        index_oid: i32,
        /// The single indexed column the equality binds to (for diagnostics/clarity).
        key_column: String,
        /// The row-independent value to probe for (`Literal`/`Parameter`/`Bound`).
        key_value: Box<Expr>,
        /// Whether the user wrote the column on the **left** (`col = v`) rather than
        /// flipped (`v = col`). Diagnostics only — it never changes which rows the
        /// probe returns, only the operand order an operand-type rejection reports,
        /// so the message matches the predicate as written (and matches what the
        /// stripped build's `Filter(Scan)` prints for the same query).
        column_first: bool,
    },

    /// Index-accelerated **range** read: probe a single-column secondary index for
    /// the rows whose `key_column` falls in a bounded range, instead of a full scan.
    ///
    /// Planned in place of `Filter(Scan)` when the WHERE clause is (or AND-contains)
    /// one or more range comparisons `indexed_col {>,>=,<,<=} <row-independent value>`
    /// (or a desugared `BETWEEN`) on a single-column full index. All range conjuncts
    /// on the same indexed column merge into one node's `lower`/`upper` bounds; any
    /// residual conjuncts (other columns, non-range, `OR`) stay in a wrapping
    /// `Filter`, so the node returns a superset of the final rows. Output columns
    /// mirror `Scan` (the table's columns).
    ///
    /// Each bound is `Some((expr, inclusive))` (`>=`/`<=` → inclusive, `>`/`<` →
    /// exclusive) or `None` for unbounded that side; the bound `expr` is
    /// row-independent by construction (a literal, a `$N` parameter, or a bound
    /// value), evaluated once against an empty row at execute time. `index_oid` is
    /// the catalog index's OID (cast to the storage `index_id`); `alias` mirrors
    /// `Scan`'s for resolution.
    ///
    /// `ascending`/`limit` carry **ORDER-BY pushdown** (`Sort` elision). For a bare
    /// WHERE they are `true`/`None` — the row **set** is what matters and any
    /// `ORDER BY`/`Sort` runs unchanged on top. But when a single-key `ORDER BY` on
    /// this node's NOT-NULL single-column-indexed `key_column` lets the index supply
    /// the order, the planner drops the `Sort` and sets `ascending` from the sort
    /// direction; an accompanying `LIMIT n` (no `OFFSET`) is pushed into `limit` as
    /// a scan early-stop, while the outer `Plan::Limit` stays the correctness
    /// boundary. See `try_order_by_index_pushdown` in `planner/query.rs`.
    #[cfg(feature = "index_acceleration")]
    IndexRangeScan {
        table: String,
        alias: Option<String>,
        /// The catalog index's OID (`Index.oid`); cast to `u32` for the storage probe.
        index_oid: i32,
        /// The single indexed column the range bounds (for diagnostics/clarity).
        key_column: String,
        /// Lower bound, or `None` for unbounded below.
        lower: Option<IndexBound>,
        /// Upper bound, or `None` for unbounded above.
        upper: Option<IndexBound>,
        /// Index-key scan direction: `false` descends (an `ORDER BY … DESC`
        /// pushdown); `true` ascends (the default, and every bare WHERE).
        ascending: bool,
        /// Optional early-stop row cap, set by an `ORDER BY … LIMIT n` pushdown
        /// (no `OFFSET`); `None` for a bare WHERE. The outer `Plan::Limit` still
        /// enforces the final count — this only short-circuits the index walk.
        limit: Option<LimitCount>,
    },

    /// Index-accelerated **containment** read: probe a **GIN** index for the rows
    /// whose `key_column` `@>`-contains one of the decomposed `probes`, instead of a
    /// full scan.
    ///
    /// Planned in place of `Filter(Scan)` when the WHERE clause has, as a top-level
    /// `AND` conjunct, either:
    /// - a single `indexed_col @> <expr>` containment (the simple case → one probe,
    ///   no guards), or
    /// - an `OR` whose every disjunct is *either* a `indexed_col @> <expr>`
    ///   containment on the **same** GIN column (→ a `probes` entry, OR-unioned —
    ///   the multi-key audit `metadata @> a OR metadata @> b`) *or* a
    ///   scan-independent boolean `guard` (→ a `guards` entry — the spine's
    ///   param-null guard `$p IS NULL OR refs @> ARRAY[$p]`).
    ///
    /// Each `<expr>` must be independent of this scan's own rows (a literal, a `$N`
    /// parameter, or a **correlated outer-scope** column — never an inner column).
    /// Because GIN is **lossy** (JSONB containment, NULL array elements), the node
    /// returns a *superset* of the matches, so the planner **always** wraps it in a
    /// `Filter` carrying the original predicate (the `@>` recheck plus any other
    /// conjuncts). Output columns mirror `Scan` (the table's columns).
    ///
    /// At execution: any `guard` that evaluates to `true` (e.g. the param bound to
    /// `$p IS NULL` is null) makes the `OR` unconditionally true → a full scan (every
    /// row is a candidate; the recheck `Filter` passes them). Otherwise each `probe`
    /// is evaluated **once** against the evaluation environment (`EvalEnv`) — the same
    /// eval-time resolution `IndexRangeScan` bounds and correlated columns use, so a
    /// `$N` param or an outer-scope `f.hash` both resolve — decomposed
    /// (`gin::gin_decompose`) into query keys whose posting lists `gin_lookup`
    /// intersects, and the per-probe candidate row sets are **unioned**. `index_oid`
    /// is the catalog index's OID (cast to the storage `index_id`); `alias` mirrors
    /// `Scan`'s.
    #[cfg(feature = "index_acceleration")]
    GinScan {
        table: String,
        alias: Option<String>,
        /// The catalog index's OID (`Index.oid`); cast to `u32` for the storage probe.
        index_oid: i32,
        /// The single GIN-indexed column the containment binds to (diagnostics).
        key_column: String,
        /// The containment constants to decompose, probe, and **union** (each a
        /// `Literal`/`Parameter`/`Bound`/correlated `Column`/`Array`/`JsonbBuildObject`
        /// over those). One entry for the simple `col @> <const>` case; several for an
        /// OR-union. Always non-empty.
        probes: Vec<Expr>,
        /// Scan-independent boolean guards from an `OR` (`$p IS NULL`); if any is
        /// `true` at eval time the `OR` is unconditionally satisfied → full scan.
        /// Empty for the simple (AND) case.
        guards: Vec<Expr>,
    },

    /// Scan a virtual catalog table (`pg_class`, `pg_attribute`, etc.).
    VirtualScan {
        catalog_table: CatalogTable,
        /// Optional alias from the FROM clause (e.g., `FROM pg_type a`).
        alias: Option<String>,
    },

    /// Filter rows.
    Filter { input: Box<Self>, predicate: Expr },

    /// Project columns.
    Project {
        input: Box<Self>,
        exprs: Vec<Expr>,
        /// The type each entry was read at as written ([`CertainTypes`]).
        #[serde(skip)]
        certain: CertainTypes,
        /// The identity of each reference an enclosing level binds ([`EnclosingColumns`]).
        #[serde(skip)]
        enclosing: EnclosingColumns,
    },

    /// Projection containing a set-returning `unnest(<array>)` over **no FROM**.
    ///
    /// Built for the spine's `query_put_fact_refs` shape — `SELECT $1, unnest($2)`
    /// — where exactly one projection expr is an `Expr::Unnest`. Row-expands: the
    /// unnested array drives the row count, and every scalar sibling is evaluated
    /// once and broadcast across the emitted rows (an empty/NULL array yields zero
    /// rows, matching Postgres). Scalars and the unnest's inner array are evaluated
    /// against an empty input row (no column context), so a `$N` must already be
    /// bound. Scoped to the no-FROM single-unnest case; a real FROM clause, multiple
    /// independent unnests, or `WITH ORDINALITY` are unsupported.
    ProjectUnnest { exprs: Vec<Expr> },

    /// Create a table.
    CreateTable {
        name: String,
        columns: Vec<ColumnDef>,
        /// Table-scoped `CHECK` constraints (column-level checks normalized in), in
        /// **written** order across the column-level and table-level spellings, each
        /// still carrying its **declared** name or `None` — the executor mints the
        /// final name.
        checks: Vec<CheckConstraintDef>,
        /// Single-column PK/UNIQUE constraints to auto-create enforced unique
        /// indexes for (composite PK/UNIQUE are not captured here — see
        /// `UniqueConstraint`).
        unique_constraints: Vec<UniqueConstraint>,
        /// `FOREIGN KEY` constraints (inline `REFERENCES` and table-level `FOREIGN
        /// KEY`, single- or multi-column), each still carrying its **declared** name or
        /// `None` — the executor mints the final name. Stored on the `Table` and
        /// enforced at write/delete time.
        foreign_keys: Vec<ForeignKeyDef>,
        if_not_exists: bool,
    },

    /// Drop one or more tables. `cascade` decides what happens when a foreign key on
    /// **another** table references one of them: without it the drop is refused
    /// (`2BP01`), with it that constraint is dropped and the child table and its rows
    /// survive. `RESTRICT` (the SQL default) folds into `cascade: false`.
    ///
    /// `names` is the **written** list, duplicates and all: Postgres resolves every
    /// name positionally before deleting anything, and a repeated missing name notices
    /// once per element. A dependent declared on another table *in the list* does not
    /// block, which is why the list is one statement and not N.
    DropTable {
        names: Vec<String>,
        if_exists: bool,
        cascade: bool,
    },

    /// Truncate one or more tables (remove all rows). Under `cascade`, the set
    /// extends to every table that transitively FK-references one already in it;
    /// `restart_identity` resets the truncated tables' owned sequences.
    Truncate {
        tables: Vec<String>,
        cascade: bool,
        restart_identity: bool,
    },

    /// Create an index.
    CreateIndex {
        /// The declared name, or `None` for the unnamed spelling (`CREATE INDEX ON t
        /// (…)`), whose name the executor mints.
        name: Option<String>,
        table: String,
        /// The key parts, converted to the planner IR at plan time: bare columns
        /// (`IndexKey::Columns`, position-keyed) or key expressions
        /// (`IndexKey::Exprs`, e.g. `LOWER(email)`). Propagated verbatim to the
        /// catalog `Index.key` — no source SQL, no write-time re-parse.
        key: IndexKey,
        unique: bool,
        /// Whether a `unique` index enforces uniqueness at write time. True for
        /// every `unique` index — plain or expression-keyed, full or partial
        /// (`WHERE …`), `NULLS [NOT] DISTINCT`: the key tuple is recomputed
        /// identically at maintenance and probe time (positions for a plain
        /// index, `eval_expr` for an expression one), so the write-time probe
        /// always agrees with the stored entry. Always false when `unique` is
        /// false (see `Index::is_enforced`).
        enforced: bool,
        /// The partial index `WHERE` predicate, converted to the planner IR at plan
        /// time, or `None` for a full index. Propagated to the catalog `Index` —
        /// no source SQL, no write-time re-parse.
        predicate: Option<Expr>,
        /// `NULLS NOT DISTINCT` (the AST's `nulls_distinct == Some(false)`): index
        /// NULL keys so two NULLs conflict. `false` is the default `NULLS DISTINCT`.
        nulls_not_distinct: bool,
        /// The **declared** access method from `USING` (`gin` → `Gin`, absent/`btree`
        /// → `Btree`). Carried so `execute_create_index` can validate it against the
        /// key column's type (GIN requires a single array/JSONB column; a non-unique
        /// B-tree over one is refused). Not persisted — the catalog `Index`
        /// re-derives the method from the key shape and uniqueness as well as the
        /// column type, so a *unique* container key is a B-tree index (see
        /// `index_spec_from_catalog`).
        method: IndexMethod,
        if_not_exists: bool,
        /// The `CONCURRENTLY` keyword was written. It changes **nothing** about the
        /// index that gets built, and nothing about what the build blocks: pglet has
        /// no *table*-scoped lock for the concurrent form to weaken, and a build holds
        /// the process-wide schema + storage pair (`pglet_server`'s `SharedState`) for
        /// its whole duration. Measured over a 400 000-row build: an `INSERT` into an
        /// entirely unrelated table, and a `SELECT` from one, each return only once the
        /// build has finished, under both spellings. That is *stricter* than 18.6, which
        /// by its own `CREATE INDEX` documentation "locks out writes (but not reads)" on
        /// the **indexed** table for a plain build and takes no lock preventing an
        /// insert, update or delete for a concurrent one — measured both ways: during
        /// a plain build an unrelated table's `INSERT` runs straight through while the
        /// indexed table's waits, and under `CONCURRENTLY` the indexed table's runs
        /// straight through too. The hole `EDGES.md` row 103 recorded was therefore not a
        /// writer running beside the build but a write **staged before it** in another
        /// connection's open block, which the build's scan never sees; the build takes
        /// the writer, so on the native server it waits for that block and then refuses
        /// `23505`, as 18.6 does after blocking on the relation lock (row 103, closed) —
        /// the embedded `Database` refuses the build `55P03` instead. The field exists to
        /// carry the one thing the keyword *does* decide: Postgres refuses it inside a
        /// transaction block (`25001`), so the statement is not interchangeable with its
        /// plain spelling. See [`Plan::forbidden_in_transaction_block`].
        concurrently: bool,
    },

    /// Drop one or more indexes, resolved positionally like [`Plan::DropTable`]'s
    /// names. pglet models no dependent of an index, so the only per-target refusal is
    /// the `2BP01` an index its declared `PRIMARY KEY` / `UNIQUE` owns raises.
    DropIndex {
        names: Vec<String>,
        if_exists: bool,
        /// The `CONCURRENTLY` keyword was written — carried for the `25001` gate
        /// only, exactly as on [`Plan::CreateIndex`].
        concurrently: bool,
    },

    /// Create a user `ENUM` type (`CREATE TYPE name AS ENUM (...)`).
    CreateType { name: String, members: Vec<String> },

    /// Drop a user type (`DROP TYPE [IF EXISTS] name [CASCADE | RESTRICT]`).
    DropType {
        name: String,
        if_exists: bool,
        /// `CASCADE` was written — it drops the persisted expressions converting to the
        /// type (`executor::dependencies`).
        cascade: bool,
    },

    /// Drop the (implicit) `public` schema and all its objects
    /// (`DROP SCHEMA [IF EXISTS] name [CASCADE | RESTRICT]`). pglet models a
    /// single implicit `public` schema, so this clears the catalog (every table,
    /// index, enum, sequence) and its storage; a non-`public` name is "not found"
    /// (`3F000`, or a no-op under `IF EXISTS`). Without `CASCADE`, a non-empty
    /// schema fails loud (`2BP01`), matching Postgres. The migration/test reset
    /// idiom is `DROP SCHEMA public CASCADE; CREATE SCHEMA public`.
    DropSchema {
        name: String,
        if_exists: bool,
        cascade: bool,
    },

    /// Create a schema (`CREATE SCHEMA [IF NOT EXISTS] name`). pglet models a
    /// single always-present implicit `public` schema, so this is a no-op success
    /// for `public` (the second half of the reset idiom, after a `DROP SCHEMA
    /// public CASCADE`) and fails loud for any other name.
    CreateSchema { name: String, if_not_exists: bool },

    /// Add a column to a table.
    AlterTableAddColumn {
        table: String,
        column: ColumnDef,
        /// A single-column inline `REFERENCES parent(col) [ON DELETE action]` on the
        /// added column, captured the same way `CREATE TABLE` captures an inline FK
        /// (see `parser::ddl`'s `parse_alter_table_add` / `parse_inline_references`). `None`
        /// when the added column has no FK or a composite/missing parent-column list (skipped
        /// cleanly). Pushed onto the table's `foreign_keys` by the executor — which
        /// also mints its name — after which the standard `enforce_foreign_keys` /
        /// `ON DELETE` machinery covers it.
        foreign_key: Option<ForeignKeyDef>,
        /// Column-level `UNIQUE` / `PRIMARY KEY` constraints declared inline on the
        /// added column (`ADD COLUMN email text UNIQUE`) — each a single-column
        /// `UniqueConstraint`. The executor builds the same enforced unique index a
        /// `CREATE TABLE` inline UNIQUE/PK would (after validating the backfilled
        /// existing rows), rather than silently dropping the constraint.
        column_uniques: Vec<UniqueConstraint>,
        /// Column-level `CHECK` constraints declared inline on the added column
        /// (`ADD COLUMN val int CHECK (val > 0)`). Registered on the table + enforced
        /// on every future write like a `CREATE TABLE` inline CHECK (the backfilled
        /// existing rows are validated against the predicate).
        checks: Vec<CheckConstraintDef>,
        if_not_exists: bool,
        /// `ALTER TABLE IF EXISTS`: a name no relation holds skips the statement with
        /// a `NOTICE` instead of `42P01`. The same field, with the same meaning, rides
        /// every `ALTER TABLE` plan; the executor reads it before the action's first
        /// catalog read, which is where 18.6's `AlterTableLookupRelation` puts it.
        table_if_exists: bool,
    },

    /// Drop a column from a table. `cascade` decides what happens when a foreign key
    /// **references** the dropped column (from any table, including this one): without
    /// it the drop is refused (`2BP01`), with it that constraint is dropped too. The
    /// table's own CHECKs and child-side FKs over the column are shed either way.
    /// `RESTRICT` (the SQL default) folds into `cascade: false`.
    AlterTableDropColumn {
        table: String,
        column: String,
        /// `DROP COLUMN IF EXISTS` — the **column's** clause.
        if_exists: bool,
        cascade: bool,
        /// `ALTER TABLE IF EXISTS` — the table's; see [`Plan::AlterTableAddColumn`].
        table_if_exists: bool,
    },

    /// Add a table-level `CHECK` constraint (`ALTER TABLE … ADD CONSTRAINT … CHECK`).
    /// The executor validates existing rows against the predicate, then records it
    /// on the table's `checks` (enforced on every future write, and visible in
    /// `information_schema.table_constraints`). FK / PK / UNIQUE via `ADD CONSTRAINT`
    /// are unsupported — the planner errors loud.
    AlterTableAddCheck {
        table: String,
        check: CheckConstraintDef,
        /// `ALTER TABLE IF EXISTS`; see [`Plan::AlterTableAddColumn`].
        table_if_exists: bool,
    },

    /// Add a `PRIMARY KEY` / `UNIQUE` / single-column `FOREIGN KEY` constraint to an
    /// existing table (`ALTER TABLE [ONLY] t ADD CONSTRAINT n …`).
    ///
    /// Reuses the same enforcement machinery a `CREATE TABLE` inline constraint
    /// builds: PK/UNIQUE create an auto-named **enforced unique index** (after
    /// validating existing rows are unique — and, for PK, non-NULL — else a loud
    /// `23505`/`23502`), and a single-column FK is recorded on the table (after
    /// validating existing child rows reference live parents, else `23503`).
    /// `CHECK` keeps its own [`Plan::AlterTableAddCheck`] arm; a composite FK fails
    /// loud at parse time.
    AlterTableAddConstraint {
        table: String,
        constraint: AddConstraint,
        /// `ALTER TABLE IF EXISTS`; see [`Plan::AlterTableAddColumn`].
        table_if_exists: bool,
    },

    /// `ALTER TABLE [ONLY] t ALTER COLUMN c { SET DEFAULT expr | DROP DEFAULT }` —
    /// set or clear a column's catalog default.
    ///
    /// `default` is the resolved [`ColumnDefault`] for `SET DEFAULT` (a literal,
    /// `now()`/`gen_random_uuid()`, or `nextval(seq)` — the pg_dump-restore path
    /// re-attaches a SERIAL column's sequence default after the data load), or
    /// `None` for `DROP DEFAULT`. The executor rewrites the column's `default` in
    /// place so omitted-column INSERTs keep working.
    AlterTableSetDefault {
        table: String,
        column: String,
        default: Option<ColumnDefault>,
        /// `ALTER TABLE IF EXISTS`; see [`Plan::AlterTableAddColumn`].
        table_if_exists: bool,
    },

    /// `ALTER TABLE … ALTER [COLUMN] col SET NOT NULL` — enforce NOT NULL on an
    /// existing column. The executor validates every existing row has a non-NULL
    /// value (else `23502`) and flips the column's `nullable` flag, so later NULL
    /// inserts are rejected. (`DROP NOT NULL` and `… TYPE …` are unimplemented and
    /// fail loud rather than silently no-op — [`Plan::AlterTableUnperformed`].)
    AlterTableSetNotNull {
        table: String,
        column: String,
        /// `ALTER TABLE IF EXISTS`; see [`Plan::AlterTableAddColumn`].
        table_if_exists: bool,
    },

    /// `ALTER TABLE … ALTER [COLUMN] col ADD GENERATED { ALWAYS | BY DEFAULT } AS
    /// IDENTITY [( options )]` — make an existing column an identity column.
    ///
    /// This is the form **pg_dump emits** for an identity column (the inline
    /// `CREATE TABLE (… GENERATED …)` is not used in a dump). The executor sets the
    /// column's `identity`, registers the implicit `<table>_<col>_seq` sequence
    /// (seeded to `start - 1`) and sets the column's `NextVal` default — over a column
    /// already declared `NOT NULL`, and refused (`55000`) over one that is not.
    AlterTableAddIdentity {
        table: String,
        column: String,
        /// `ALWAYS` or `BY DEFAULT`.
        generation: IdentityGeneration,
        /// The `START WITH` value; the sequence seeds to `start - 1`.
        start: i64,
        /// The declared `SEQUENCE NAME <n>`, or `None` for the engine's minted
        /// `<table>_<column>_seq`. pg_dump emits this form **with** the name, so
        /// discarding it re-minted a name that only coincides while the dumped
        /// database used the default one.
        sequence_name: Option<String>,
        /// `ALTER TABLE IF EXISTS`; see [`Plan::AlterTableAddColumn`].
        table_if_exists: bool,
    },

    /// An `ALTER TABLE` operation pglet parses and does not perform: a clause it
    /// tolerates as a no-op (`OWNER TO`, `SET STATISTICS`, and the other `pg_dump`
    /// spellings with no data effect here), or one it refuses (`RENAME`,
    /// `DROP CONSTRAINT`, `ALTER COLUMN … TYPE`, `DROP NOT NULL`, …).
    ///
    /// It is a plan rather than a parse-time answer so that the relation is looked up
    /// first, as 18.6 looks it up before it analyses any action: under `IF EXISTS` a
    /// name no relation holds is the `NOTICE` and skip whatever the operation, and a
    /// refusal is reached only over a relation that exists — without `IF EXISTS`, a
    /// missing one is `42P01`. A **tolerated** clause does not look the relation up
    /// unless `IF EXISTS` asks it to, so `ALTER TABLE <no such table> OWNER TO x`
    /// still reports success (`EDGES.md` row 15).
    ///
    /// It is also the plan of an `ALTER TABLE IF EXISTS` whose relation name no
    /// relation pglet models can hold, whatever its operation —
    /// [`UnperformedAlter::Skipped`].
    AlterTableUnperformed {
        table: String,
        /// `ALTER TABLE IF EXISTS`; see [`Plan::AlterTableAddColumn`].
        table_if_exists: bool,
        /// What the statement answers once its relation is settled.
        answer: UnperformedAlter,
    },

    /// `CREATE SEQUENCE [IF NOT EXISTS] name [options]` — register a named sequence
    /// in the catalog so a later `DEFAULT nextval('name')` / `setval('name', …)`
    /// resolves it.
    ///
    /// `START WITH n`, `RESTART [[WITH] n]` and `OWNED BY` are honored (see
    /// `catalog::Sequence`); the remaining option words (`AS <type>`, `INCREMENT BY`,
    /// `[NO] MINVALUE/MAXVALUE`, `CACHE`, `CYCLE`) are parsed and their values
    /// discarded, since `nextval` is unconditionally `last_value + 1`. Produced by
    /// pg_dump for every SERIAL column.
    ///
    /// **Discarding a value is not accepting every spelling.** `AS <type>` reads a
    /// real type name, so refusals 18.6 raises while the *grammar* reduces escape the
    /// discard — `AS FLOAT(0)` / `AS FLOAT(54)` are `22023` on both engines and
    /// `AS timestamp(3,1)` / `AS interval(-1)` are `42601` on both — while refusals
    /// that need the *resolved* type do not: an in-range `AS FLOAT(1)`, `AS int4(3)`,
    /// `AS varchar(3)`, `AS nosuchtype` are each accepted here against an 18.6 error
    /// (`executor::resolve_column_type`'s doc carries the measured set; the repo's
    /// `EDGES.md` row 260 owns the dropped-`AS` class, and rows 116, 119 and 124 name
    /// members of it).
    CreateSequence {
        name: String,
        /// The `START WITH` value (default `1`); the counter is seeded to `start - 1`.
        start: i64,
        /// The `RESTART` clause, if written. `SeqOptElem` admits it on `CREATE`
        /// too, and `init_params` applies it exactly as it does on `ALTER` — it
        /// moves the **counter** and leaves `seqstart` alone, so `CREATE SEQUENCE s
        /// START WITH 10 RESTART WITH 5` hands out `5` and a later bare `RESTART`
        /// returns to `10` (measured on 18.6). A bare `RESTART` here is therefore a
        /// no-op: `isInit` already seeds the counter to `start`.
        restart: Option<SequenceRestart>,
        if_not_exists: bool,
        /// The `OWNED BY {table.column | NONE}` clause, if written.
        owned_by: Option<SequenceOwnerSpec>,
    },

    /// `ALTER SEQUENCE [IF EXISTS] name [options]`.
    ///
    /// Two option tails carry meaning — `START [WITH] n` / `RESTART [[WITH] n]` and
    /// `OWNED BY {table.column | NONE}`. Everything else in the tail (`OWNER TO`,
    /// `INCREMENT BY`, `CACHE`, `AS <type>`, `RENAME TO`, …) is tolerated with no
    /// effect, as it was when the whole statement was a no-op — but two things still
    /// bite. The **target is resolved**, so `ALTER SEQUENCE nosuch OWNER TO x` is
    /// `42P01` and `ALTER SEQUENCE <a table> …` is `42809`, matching Postgres. And
    /// `AS <type>` reads a real type name through the same `parse_sequence_options`
    /// loop `CREATE SEQUENCE` uses, so refusals 18.6 raises while the *grammar*
    /// reduces escape the tolerance: `ALTER SEQUENCE s AS FLOAT(0)` / `… AS FLOAT(54)`
    /// are `22023` and `… AS timestamp(3,1)` / `… AS interval(-1)` are `42601`, each on
    /// both engines. An in-range `AS FLOAT(1)` is accepted here against 18.6's
    /// `22023 sequence type must be smallint, integer, or bigint` — `EDGES.md` row
    /// 116's recorded position residual. No test or fixture reaches
    /// `ALTER SEQUENCE … AS` at all;
    /// `crates/pglet/tests/float_precision_selection.rs`'s position loop covers
    /// `CREATE SEQUENCE … AS` and the identity clause's `(AS …)`.
    AlterSequence {
        name: String,
        if_exists: bool,
        /// `START [WITH] n` — moves the recorded `START WITH`. Not a no-op even
        /// without a `RESTART`: a *later* bare `RESTART` returns to the new value
        /// (measured on 18.6), so tolerating this while honoring `RESTART` would be
        /// silently wrong.
        start: Option<i64>,
        /// The `RESTART` clause, if written. Applied **after** `start`, in either
        /// written order, as `init_params` does.
        restart: Option<SequenceRestart>,
        /// The `OWNED BY {table.column | NONE}` clause, if written.
        owned_by: Option<SequenceOwnerSpec>,
    },

    /// `DROP SEQUENCE [IF EXISTS] name [, …] [CASCADE | RESTRICT]`.
    ///
    /// `cascade` is load-bearing: a column whose `DEFAULT` is `nextval` of one of these
    /// sequences is a NORMAL dependent, so a bare drop is `2BP01` and `CASCADE` sheds
    /// **that column's default** (the column and its rows survive). An *identity*
    /// column's sequence is an INTERNAL dependent instead and is refused either way.
    /// `names` is the written list, resolved positionally like [`Plan::DropTable`]'s.
    DropSequence {
        names: Vec<String>,
        if_exists: bool,
        cascade: bool,
    },

    /// A pg_dump-preamble statement pglet parses for restore tolerance but models no
    /// semantic effect for — it returns only its command tag (`Noop::command_tag`).
    /// Covers `GRANT`, `REVOKE`, and `COMMENT ON …` (pglet has no roles, privileges,
    /// or object comments); `ALTER TABLE`'s tolerated clauses are
    /// [`Plan::AlterTableUnperformed`].
    Noop { tag: NoopCommand },

    /// Insert rows.
    ///
    /// `rows` is the row source — either literal `VALUES (...)` tuples or an
    /// `INSERT … SELECT` sub-plan with its column mapping (see `InsertRows`). Both
    /// forms feed the same per-row insert loop (DEFAULT-fill, constraints, ON
    /// CONFLICT, RETURNING all run unchanged).
    Insert {
        table: String,
        /// How this `INSERT` produces its rows (`VALUES` literals or `SELECT`).
        rows: InsertRows,
        /// RETURNING clause expressions (empty if not specified).
        returning: Vec<Expr>,
        /// ON CONFLICT clause for upsert behavior.
        on_conflict: Option<OnConflictClause>,
        /// The `OVERRIDING { SYSTEM | USER } VALUE` clause. Consumed at lowering
        /// (`lower_insert` decides per-row whether an identity column's supplied
        /// value is kept, replaced with the sequence default, or rejected `428C9`),
        /// so the executor doesn't re-consult it — it rides the plan for
        /// completeness / Describe symmetry.
        overriding: Overriding,
    },

    /// Empty result (for unsupported/no-op statements).
    Empty,

    /// Update rows in a table.
    Update {
        table: String,
        /// Target-table alias (`UPDATE t AS o …` / `UPDATE t o …`); `None` when
        /// unaliased. An alias fully replaces the table name in `SET`/`WHERE`/
        /// `RETURNING` resolution (Postgres semantics), so it drives alias-aware
        /// column lookup and the `o.*` RETURNING wildcard.
        alias: Option<String>,
        /// Optional `FROM` source joined to the target (`UPDATE t o SET … FROM
        /// other a WHERE …`). When present, the target is inner-joined to this plan
        /// and each matching pair drives an update; a target row is updated at most
        /// once (the first matching `from` row — Postgres leaves a multi-match
        /// update unspecified, so callers should join on a key). The `SET`/`WHERE`
        /// expressions resolve against the combined `target ++ from` namespace.
        from: Option<Box<Self>>,
        assignments: Vec<(String, Expr)>,
        filter: Option<Expr>,
        /// The index probe the target rows are collected through — see
        /// [`Plan::Delete`]'s field of the same name. Always `None` with a `from`.
        target_probe: Option<Box<Self>>,
        /// RETURNING clause expressions (empty if not specified). `*` / `<target>.*`
        /// wildcards are expanded to the target table's columns at lowering time.
        returning: Vec<Expr>,
    },

    /// Delete rows from a table.
    Delete {
        table: String,
        filter: Option<Expr>,
        /// The index probe the target rows are collected through, when lowering found
        /// one (`planner::query`'s `dml_target_probe`): an equality [`Plan::IndexScan`]
        /// over the target, chosen by the read path's own index selection. `None`
        /// collects by a full scan — on the stripped build always, and on the default
        /// build wherever the read path's choice is not an equality probe or the `WHERE`
        /// holds anything beside it and the residual conjuncts `dml_target_probe` admits. The probe only narrows the
        /// candidates: each is still tested against the whole `filter`, in `RowId` order.
        target_probe: Option<Box<Self>>,
        /// RETURNING clause expressions (empty if not specified).
        returning: Vec<Expr>,
    },

    /// `COPY <table> [(cols)] FROM STDIN | TO STDOUT [options]` — bulk text
    /// transfer between the table and an out-of-band byte stream.
    ///
    /// COPY has no `$N` parameters and no in-band result rows; the byte stream
    /// lives outside the `QueryResult` channel, so the engine's plain
    /// `execute_with_storage` path can't carry it. The plan is produced by the
    /// parser and consumed by the embed `Database::copy_in` / `copy_out` API (and
    /// the server's CopyData wire path), which call `execute_copy_from` /
    /// `execute_copy_to` directly with the data slice. Routing a `Plan::Copy`
    /// through `execute_with_storage` fails loud (no byte channel there).
    ///
    /// `direction` stays a sibling of [`CopyOptions`] because it drives the To/From
    /// dispatch and the read-only / `mutates_storage` classification; the rest of
    /// the resolved fields ride the shared `CopyOptions` (also embedded in the
    /// server's `CopyInState`, borrowed as the engine's `CopyParams`), so the field
    /// set lives in one place.
    Copy {
        /// Transfer direction (`FROM STDIN` ingests, `TO STDOUT` emits).
        direction: CopyDirection,
        /// The table, column list, format, and text-framing options.
        options: CopyOptions,
    },

    /// Sort rows by expressions.
    Sort {
        input: Box<Self>,
        /// The sort keys, each carrying its direction and NULL placement.
        order_by: Vec<OrderKey>,
        /// The target-list positions these keys **name** — see [`ResolvedTargets`].
        /// Recorded against the query level's own target list, whichever side of the
        /// projection this `Sort` ends up on.
        resolved_targets: ResolvedTargets,
    },

    /// `SELECT DISTINCT` — remove duplicate rows from the input, preserving
    /// first-occurrence order. Two NULLs count as equal (Postgres `DISTINCT`
    /// treats NULL as not-distinct), unlike `=`.
    Distinct { input: Box<Self> },

    /// `SELECT DISTINCT ON (e1[, e2…])` — keep the **first** row of each group
    /// whose `on_exprs` tuple is equal, in the input's row order. Two NULLs count
    /// as equal in the ON key (like `Distinct`), unlike `=`.
    ///
    /// The planner feeds this a `Sort` (directly, or through a `Project` when the
    /// sort key drops a projected column) whose leading keys are the `on_exprs`, so
    /// equal-ON rows are already adjacent — the executor keeps a row only when its
    /// ON key differs from the previous kept row's (an O(n) adjacent dedup). That
    /// one sort is also the query's visible order (unlike `Distinct`, which is
    /// order-independent and is wrapped *inside* the `Sort`); `Plan` is not
    /// persisted, so this ordering-dependent placement is free.
    DistinctOn {
        /// The `DISTINCT ON` key expressions, resolved against the input's output
        /// columns (a positional `DISTINCT ON (1)` already mapped to its column).
        on_exprs: Vec<Expr>,
        input: Box<Self>,
    },

    /// Drop the trailing **junk** columns a sort or dedup key needed — Postgres's
    /// junk filter (`execJunk.c`).
    ///
    /// An `ORDER BY` / `DISTINCT ON` key may name something the select list does not
    /// carry (`SELECT count(*) FROM t GROUP BY k ORDER BY k`), and the node that
    /// could evaluate it — an `Aggregate` whose rows have already collapsed, a
    /// `Window` whose functions are computed over the input order — sits *below* the
    /// key's consumer. Postgres's answer is a `resjunk` target-list entry: the node
    /// emits the key's value as an extra output column, the `Sort` / `DistinctOn`
    /// above reads it by name, and this node trims the row back to the select list's
    /// own width. `keep` is that width, so the junk columns are always the trailing
    /// ones.
    ///
    /// It must sit above **every** consumer of a junk column, which is why the
    /// planner places it rather than `build_sort`: a `DISTINCT ON` reads
    /// its key above the `Sort`.
    JunkFilter {
        input: Box<Self>,
        /// The number of leading columns that are the query's real output.
        keep: usize,
    },

    /// Limit and offset rows.
    ///
    /// `limit`/`offset` are `Option<LimitCount>`: `None` means "no limit" /
    /// "no offset", `Some(Const(n))` a literal count, and `Some(Param(i))` a `$i`
    /// placeholder resolved from the evaluation environment at execution time
    /// (`EvalEnv::limit_param`: NULL clears the bound, negative fails loud).
    Limit {
        input: Box<Self>,
        limit: Option<LimitCount>,
        offset: Option<LimitCount>,
    },

    /// Aggregate rows (GROUP BY with aggregate functions).
    Aggregate {
        input: Box<Self>,
        /// GROUP BY expressions (empty for aggregate over all rows).
        group_by: Vec<Expr>,
        /// Output expressions (mix of group columns and aggregates).
        exprs: Vec<Expr>,
        /// The `exprs` positions this node's `GROUP BY` keys **name** — see
        /// [`ResolvedTargets`].
        resolved_targets: ResolvedTargets,
        /// HAVING clause filter (applied after aggregation).
        having: Option<Expr>,
        /// The type each of `exprs` was read at as written ([`CertainTypes`]).
        #[serde(skip)]
        certain: CertainTypes,
        /// The identity of each reference an enclosing level binds ([`EnclosingColumns`]).
        #[serde(skip)]
        enclosing: EnclosingColumns,
    },

    /// Join two inputs.
    Join {
        left: Box<Self>,
        right: Box<Self>,
        join_type: JoinType,
        /// ON condition (e.g., `users.id = orders.user_id`). For a `USING`, `CROSS`,
        /// or comma join there is no user ON predicate, so this is
        /// `Expr::Literal(Literal::Bool(true))` and the row match is driven by
        /// `using` (empty `using` + `true` on = a Cartesian product).
        on: Expr,
        /// The `JOIN … USING (c1, c2, …)` column names, in USING order. Empty for an
        /// ordinary `ON`/`CROSS`/comma join. When non-empty the executor matches row
        /// pairs on these columns' pairwise equality (never via `on`) and emits a
        /// **merged** output: one COALESCE'd column per USING name — prepended, so an
        /// unqualified reference and a chained USING see it — ahead of the raw
        /// per-side columns (which stay qualifiable as `left.c` / `right.c`).
        using: Vec<String>,
    },

    /// Combine two subplans with a set operation (`UNION`/`INTERSECT`/`EXCEPT`,
    /// optionally `ALL`).
    ///
    /// The two sides have already been column-count-checked and per-position
    /// type-unified at plan time (`build_set_op`): `column_types` holds the common
    /// per-position type, and the executor casts each side's values to it
    /// **positionally** before combining (so both feed the combiner with identical
    /// column types). Result column **names** come from the `left` side (Postgres uses
    /// the leftmost SELECT); result column **types** are `column_types`. Whole-row
    /// duplicate handling counts two NULLs as equal, unlike `=` (matching Postgres
    /// set-op semantics), via the same comparator `Distinct` uses.
    ///
    /// Casting positionally (not via a by-name `Project`) is deliberate: a set-op side
    /// can have repeated output column names — unnamed exprs (all `?column?`),
    /// duplicate aliases, `SELECT col, col` — and by-name resolution would collapse
    /// every same-named column onto the first, silently corrupting values.
    ///
    /// `all` keeps duplicates: `UNION ALL` concatenates; `INTERSECT ALL` emits
    /// `min(left_count, right_count)` copies per distinct row; `EXCEPT ALL` emits
    /// `max(0, left_count − right_count)`. Without `all`, each side is treated as a
    /// set: `UNION` dedups the concatenation, `INTERSECT` keeps rows present in both,
    /// `EXCEPT` keeps rows present in the left but not the right.
    ///
    /// A trailing `ORDER BY`/`LIMIT` binds to the *combined* result — the planner
    /// wraps this node in `Limit(Sort(SetOp(…)))`, so no extra executor plumbing is
    /// needed. `Plan` is not persisted, so this variant's placement is free.
    SetOp {
        op: SetOpKind,
        /// `true` keeps duplicates (`… ALL`); `false` treats each side as a set.
        all: bool,
        left: Box<Self>,
        right: Box<Self>,
        /// The unified per-position column type both sides are cast to; also the
        /// result column types (paired with the left side's names).
        column_types: Vec<TypeOid>,
        /// The column positions whose type waits on a reference to an **enclosing**
        /// query level, which the correlated re-walk resolves — see
        /// `super::query::OuterTyping`. Empty once every column is unified; their
        /// `column_types` entries are provisional until then, and a plan the parser
        /// returns never carries one ([`SetOpOuterTypes`]).
        outer_pending: Vec<usize>,
        /// Where the re-walk records the enclosing-level types it unified the pending
        /// columns with — `Some` exactly when the node was built with a pending column.
        #[serde(skip)]
        outer_types: Option<SetOpOuterTypes>,
    },

    /// Begin a transaction, at the isolation level its mode list named last (`None`:
    /// the session's `default_transaction_isolation`).
    BeginTransaction {
        isolation: Option<crate::executor::IsolationLevel>,
    },

    /// Commit the current transaction.
    CommitTransaction,

    /// Rollback the current transaction.
    RollbackTransaction,

    /// Establish a savepoint in the current transaction block: `SAVEPOINT <name>`.
    ///
    /// The three savepoint plans carry an already-folded name (unquoted downcased,
    /// quoted verbatim — the parser's one name seam), because a savepoint name is
    /// an `IDENT` in `gram.y` and Postgres compares the folded bytes. Duplicates are
    /// legal and stack: `ROLLBACK TO`/`RELEASE` take the *most recent* frame of that
    /// name, so the older one survives to answer the next reference.
    Savepoint { name: String },

    /// Roll back to a savepoint: `ROLLBACK TO [SAVEPOINT] <name>`.
    ///
    /// Reports command tag `ROLLBACK` — the same tag a whole-block rollback reports,
    /// which is why the transaction lifecycle dispatches on the executor's
    /// [`QueryResult::tx_command`](crate::executor::QueryResult) signal and not on
    /// the tag.
    RollbackToSavepoint { name: String },

    /// Release a savepoint: `RELEASE [SAVEPOINT] <name>`.
    ReleaseSavepoint { name: String },

    /// Subquery scan - execute a subquery and use its results as a table.
    /// This is used for derived tables in FROM clause: `SELECT * FROM (SELECT ...) AS t`
    ///
    /// `column_aliases` is the FROM item's **column**-alias list, positionally replacing
    /// the subquery's own output names (`WITH c(x) AS (SELECT a FROM t)` — the ordinary
    /// read-only CTE is the one route that fills it today). It is applied by
    /// [`alias_relation_columns`] at **every** reader of this node's columns rather than
    /// baked into `subquery`, because the node's readers do not agree on where they read
    /// those columns from: the executor re-derives them by *executing* the subquery, so
    /// a rename inside the plan would reach the binding gate and `Describe` and leave
    /// the rows named as the body named them. Empty for a derived table and for a CTE
    /// written without a list. Never longer than the subquery's column count — the
    /// parser refuses that `42P10` at `WITH` time.
    ///
    /// `cte_rank` is `Some` where the node is an inlined read-only CTE: the CTE's place
    /// in its `WITH` list's analysis order ([`CtePlan::rank`]'s numbering). 18.6 analyses
    /// every `WITH` query before the statement's own clauses, whatever clause the
    /// reference sits in, and the one reader of the rank is parameter inference, which
    /// orders a `$N`'s uses as that analysis reaches them (`planner::param_types`).
    SubqueryScan {
        subquery: Box<Self>,
        alias: String,
        column_aliases: Vec<String>,
        cte_rank: Option<u32>,
    },

    /// A statement whose `WITH` list holds CTEs that must be **materialized** before
    /// the body runs: the **data-modifying** ones (`WITH x AS (UPDATE/DELETE/INSERT …
    /// RETURNING …) <body>`) and the genuinely **recursive** ones of a `WITH RECURSIVE`.
    ///
    /// The entries of `ctes` run in order — declaration order under a plain `WITH`, the
    /// dependency order `parse_with_clause` resolved under `WITH RECURSIVE` — each with
    /// every earlier entry bound into the evaluation environment, and `body` runs last
    /// with all of them bound. A [`Plan::CteScan`] reads an entry by name; the plan is
    /// never mutated to bind the rows (the materialized relation rides the `EvalEnv`,
    /// consistent with the parameter/outer-scope model). A data-modifying entry runs
    /// **exactly once** (so a CTE referenced twice — or zero times, which Postgres
    /// still runs — fires once); a recursive entry runs the fixed-point loop
    /// described at [`CteBody::Recursive`].
    ///
    /// Read-only non-recursive CTEs are *not* represented here — the parser inlines
    /// them as `SubqueryScan`s at their reference sites (Postgres may inline a
    /// non-modifying CTE too). A statement is a write (`mutates_storage` /
    /// `!is_read_only`) iff one of its entries or its body is, so a read-only
    /// `WITH RECURSIVE … SELECT` stays on the lock-free MVCC read path.
    ///
    /// **Snapshot caveat.** Postgres runs every part of the statement against the
    /// snapshot at statement start, so a sibling read does *not* see a modifying
    /// CTE's effects. pglet runs the CTEs, then the body, against the same storage,
    /// so a later part re-scanning a base table a sibling CTE wrote observes the
    /// post-write rows (read-your-own-writes). The parser refuses a body that writes
    /// beside a data-modifying entry, a recursive entry after one, and under
    /// `RECURSIVE` a second data-modifying entry or a body naming a table one can write
    /// (`Parser::wrap_with_ctes`). Under a plain `WITH` a later data-modifying entry or a
    /// `SELECT` body reading such a table stays — the first of them a write — and so do
    /// several data-modifying entries run in declaration order, where Postgres's
    /// pull-driven order decides which of two writers of one row takes it: documented
    /// divergences (`EDGES.md`).
    With {
        /// The CTEs to materialize, in the order they run.
        ctes: Vec<CtePlan>,
        /// The main query, executed after the CTEs with their rows bound in.
        body: Box<Self>,
    },

    /// A reference to a materialized data-modifying CTE in a FROM clause
    /// (`… FROM updated u`). Reads the rows the enclosing [`Plan::With`] captured
    /// for `name` from the evaluation environment — it never re-runs the DML.
    ///
    /// `alias` is the effective FROM name (the explicit alias, else the CTE name),
    /// keyed on by alias-aware resolution exactly like `Scan`'s alias. `columns`
    /// carries the CTE's output columns (computed from its `RETURNING` projection at
    /// plan time) so this node's [`Self::result_shape`] (`Describe`'s entry), its
    /// lossy [`Self::output_columns`] read, and `alias.*` expansion all work without
    /// the data.
    CteScan {
        /// The CTE name to look up in the environment's materialized results.
        name: String,
        /// The effective FROM name (explicit alias, else the CTE name).
        alias: String,
        /// The CTE's output columns (its `RETURNING` projection), for Describe and
        /// wildcard expansion.
        columns: Vec<ColumnDesc>,
    },

    /// A function call as a FROM table source — 18.6's `FunctionScan`, over the
    /// functions [`FromFunction`] names.
    ///
    /// `unnest(arr1, arr2, …)` expands its arrays into rows, zipping element-wise and
    /// NULL-padding shorter arrays (Postgres semantics), one output column per array.
    /// `generate_series(start, stop[, step])` answers one column of its integer overload.
    /// Each argument is evaluated once per run of the item, with no input row; there is no
    /// ordinality.
    FunctionScan {
        func: FromFunction,
        args: Vec<Expr>,
        columns: Vec<String>,
        /// The effective FROM name (explicit alias, else the function's name) — the name
        /// a qualified reference must spell, and the name under which
        /// `push_plan_relations` registers this relation. Carried on the node because
        /// a scope built from an assembled plan has no other way to learn it, and a
        /// relation the scope cannot name is one whose columns a **closed** scope
        /// silently fails to count as claimants.
        alias: String,
        /// The type each output column was read at as written ([`CertainTypes`]): an
        /// array's **element** type for `unnest`, the overload's for `generate_series`.
        #[serde(skip)]
        certain: CertainTypes,
    },

    /// Window function computation.
    /// Computes window function values for each row based on partitioning and ordering.
    Window {
        input: Box<Self>,
        /// Window function expressions to compute (these contain `Expr::Window`).
        window_exprs: Vec<Expr>,
        /// The type each of `window_exprs` was read at as written ([`CertainTypes`]).
        #[serde(skip)]
        certain: CertainTypes,
        /// The identity of each reference an enclosing level binds ([`EnclosingColumns`]).
        #[serde(skip)]
        enclosing: EnclosingColumns,
    },

    /// A `DO $$ … $$` anonymous PL/pgSQL block, supported in a **minimal** form:
    /// `BEGIN <statements>; [EXCEPTION WHEN <condition> [OR <condition>]* THEN
    /// <handler>;] END`. The body statements run in order; if one raises an error
    /// whose SQLSTATE matches the `EXCEPTION` clause, the rest of the body is skipped
    /// and the handler runs — so the error is swallowed. This covers the standard
    /// idempotent-`CREATE TYPE` idiom (`… EXCEPTION WHEN duplicate_object THEN NULL`,
    /// the fuz_app cell migration), since `CREATE TYPE` has no `IF NOT EXISTS`. Full
    /// PL/pgSQL (variables, `DECLARE`, control flow, multiple handlers) is
    /// deliberately out of scope — it fails loud at parse time, keeping the
    /// size-budget floor intact.
    ///
    /// The body is planned at parse time against the live schema, so an
    /// intra-block dependency (a later statement needing an earlier one's new
    /// object) isn't modeled; the spine's single-`CREATE TYPE` body has none. The
    /// handler **does** roll the body back, as Postgres's subtransaction does:
    /// `executor::execute_do_block` takes a storage savepoint plus a `Schema`
    /// snapshot around the block and, on a matched SQLSTATE, restores both before the
    /// handler runs — so an earlier completed statement, a partially applied multi-row
    /// one and DDL are all discarded (`EDGES.md` row 22). A `DO` that fails — no handler,
    /// an unmatched one, or a handler that raises — restores both before its error
    /// returns, since it is one statement (`EDGES.md` row 249).
    DoBlock {
        /// The statements between `BEGIN` and `EXCEPTION`/`END`, planned at parse time.
        body: Vec<Self>,
        /// The single `EXCEPTION WHEN … THEN …` handler, if present.
        exception: Option<DoExceptionHandler>,
    },

    /// A read statement that carries a row-locking clause — `FOR UPDATE`, `FOR NO KEY
    /// UPDATE`, `FOR SHARE` or `FOR KEY SHARE`, at any level of it — over the plan the
    /// statement would have without one. It changes no row the statement answers: pglet
    /// locks no row. What it carries is the one fact a host needs, that a block running
    /// the statement takes **the writer** ([`Plan::takes_writer`]), which is what makes
    /// a `SELECT … FOR UPDATE` then `UPDATE` in a block serialize against another
    /// connection's write as the row lock does on 18.6.
    ///
    /// It wraps a whole statement and nothing else: the parser adds it once, at the
    /// statement's top, and only over a plan that does not already write, so no walker
    /// below a statement's root meets it.
    LockRows { input: Box<Self> },
}

impl<'de> Deserialize<'de> for Plan {
    /// Always fails — see [`Plan`]'s own docs for why a plan is not a decodable fact.
    ///
    /// The deserializer is dropped unread. postcard is not self-describing, so there
    /// is nothing to skip past: the caller's decode is over the moment this returns.
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(PLAN_NOT_PERSISTED))
    }
}

/// The `EXCEPTION WHEN … THEN …` handler of a minimal [`Plan::DoBlock`].
///
/// The PL/pgSQL condition names are resolved to their SQLSTATE codes at parse time
/// (`duplicate_object` → `42710`, …); a body error is caught when its
/// [`sqlstate`](crate::Error::sqlstate) matches one of `sqlstates` or `catch_all`
/// is set (the `OTHERS` condition).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DoExceptionHandler {
    /// The SQLSTATE codes this handler catches (resolved from the condition names).
    pub sqlstates: Vec<String>,
    /// `true` for the `OTHERS` catch-all — matches any error regardless of SQLSTATE.
    pub catch_all: bool,
    /// The handler statements, run when a matching error fires. A PL/pgSQL `NULL;`
    /// no-op lowers to an empty list.
    pub handler: Vec<Plan>,
}

/// The `RESTART` clause of an `ALTER SEQUENCE`, as written.
///
/// A sum type rather than an `Option<i64>` inside the plan's own `Option`, because
/// the two spellings mean different things and only one carries a value: bare
/// `RESTART` returns the counter to whatever the sequence's `START WITH` is *at that
/// moment* (so a `START WITH` in the same statement moves it first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SequenceRestart {
    /// `RESTART` — return to the sequence's `START WITH`.
    ToStart,
    /// `RESTART [WITH] n` — the next `nextval` returns `n`.
    To(i64),
}

/// The `OWNED BY` clause of a `CREATE`/`ALTER SEQUENCE`, as written.
///
/// The parse side, so it can say what the statement *said* — `NONE` (detach) is a
/// different fact from an absent clause (leave alone), which is why the executor
/// takes an `Option<SequenceOwnerSpec>` rather than an `Option<SequenceOwner>`.
/// The catalog side is `catalog::SequenceOwner`, resolved against the live schema
/// (`42P01` for a missing table, `42703` for a missing column).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum SequenceOwnerSpec {
    /// `OWNED BY NONE` — detach the sequence from any column.
    None,
    /// `OWNED BY <table>.<column>`.
    Column { table: String, column: String },
}

/// The command-tag identity of a [`Plan::Noop`] — a pg_dump-preamble statement
/// pglet tolerates with no semantic effect.
///
/// Each variant carries only the `CommandComplete` tag it reports; pglet models no
/// roles, privileges, or object comments, so these are correct no-ops on restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum NoopCommand {
    /// `GRANT …`.
    Grant,
    /// `REVOKE …`.
    Revoke,
    /// `COMMENT ON …`.
    Comment,
    /// `CREATE EXTENSION …` naming an extension pglet already provides.
    CreateExtension,
    /// `DROP EXTENSION …`.
    DropExtension,
}

impl NoopCommand {
    /// The Postgres `CommandComplete` tag this no-op reports.
    #[must_use]
    pub const fn command_tag(self) -> &'static str {
        match self {
            Self::Grant => "GRANT",
            Self::Revoke => "REVOKE",
            Self::Comment => "COMMENT",
            Self::CreateExtension => "CREATE EXTENSION",
            Self::DropExtension => "DROP EXTENSION",
        }
    }
}

/// What a [`Plan::AlterTableUnperformed`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum UnperformedAlter {
    /// A clause tolerated as a no-op (`OWNER TO`, `SET STATISTICS`, …): the
    /// `ALTER TABLE` tag, the relation looked up only under `IF EXISTS`.
    Tolerated,
    /// A clause refused `0A000` with this message over a relation that exists, and
    /// `42P01` without `IF EXISTS` over one that does not.
    Refused(&'static str),
    /// 18.6's skip, unconditionally: under `IF EXISTS`, a relation name whose qualifier
    /// names a schema no relation pglet models can be in (`parser::ddl`'s
    /// `qualifier_holds_no_relation`), so 18.6 finds no relation there either and
    /// answers the `NOTICE` and the tag before it analyses the operation. The plan's
    /// `table` is the name's last part, the one 18.6's notice prints.
    Skipped,
}

/// The `OVERRIDING { SYSTEM | USER } VALUE` clause of an INSERT, controlling how a
/// value supplied for an identity column is treated.
///
/// Applied at lowering ([`lower_insert`](crate::planner::lower_insert)): `System`
/// forces the literal (permitting a `GENERATED ALWAYS` explicit value); `User`
/// discards the supplied value and draws the sequence default; `None` is the
/// default (a `GENERATED ALWAYS` explicit value is then rejected `428C9`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Overriding {
    /// No `OVERRIDING` clause.
    None,
    /// `OVERRIDING SYSTEM VALUE` — the user's value overrides the identity default.
    System,
    /// `OVERRIDING USER VALUE` — the user's value for an identity column is ignored.
    User,
}

/// The kind of constraint an [`Plan::AlterTableAddConstraint`] adds to an existing
/// table — the PK/UNIQUE/FK forms that build real enforcement (CHECK has its own
/// [`Plan::AlterTableAddCheck`] arm).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum AddConstraint {
    /// `ADD [CONSTRAINT n] PRIMARY KEY (cols)` — validate existing rows are non-NULL
    /// and unique, set the columns NOT NULL, and create the enforced unique index
    /// under the declared name (else `<table>_pkey`).
    PrimaryKey {
        columns: Vec<String>,
        name: Option<String>,
    },
    /// `ADD [CONSTRAINT n] UNIQUE (cols)` — validate existing rows are unique and
    /// create the enforced unique index under the declared name (else
    /// `<table>_<cols>_key`).
    Unique {
        columns: Vec<String>,
        name: Option<String>,
    },
    /// `ADD … FOREIGN KEY (col) REFERENCES parent(col) [ON DELETE …]` — validate
    /// existing child rows reference live parents, then record the FK on the table.
    ForeignKey(ForeignKeyDef),
}

/// A `CHECK` constraint **as written**, before the executor mints its name.
///
/// The parse-side counterpart of the catalog [`crate::catalog::CheckConstraint`], differing only in
/// `name: Option<String>` — the same split, and for the same reason, as
/// [`ForeignKeyDef`] against `ForeignKey`. A `None` here means "no `CONSTRAINT <n>`
/// was written, generate one"; the catalog shape cannot express that, so nothing
/// unnamed can reach the store and no reader has to guess what an unnamed check is
/// called.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckConstraintDef {
    /// The `CONSTRAINT <n>` the user declared, or `None` to take Postgres's auto-name
    /// (`<table>_<column>_check` or `<table>_check`, chosen by the executor's namer).
    pub name: Option<String>,
    /// The predicate, parsed into the planner IR.
    pub expr: Expr,
}

/// A `FOREIGN KEY` **as written**, before the executor mints its name.
///
/// The catalog [`crate::catalog::ForeignKey`] carries a plain `String` name because it is persisted
/// and every stored constraint has one. This is the parse-side shape, and the only
/// difference that matters is `name: Option<String>` — which makes "the user declared
/// this name" and "the engine will generate one" two distinct states instead of one
/// indistinguishable `String`. Postgres treats them oppositely (`ATExecAddConstraint`):
/// a **declared** name that collides is the user's error (`42710`), an **auto** name
/// that collides is disambiguated with a digit and never fails a statement. Baking the
/// auto-name at parse time collapsed the two and cost both halves — an auto-name was
/// gated where Postgres disambiguates, and a declared name on an inline `REFERENCES`
/// was silently dropped.
///
/// Mirrors [`UniqueConstraint`], which has carried `Option<String>` all along.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForeignKeyDef {
    /// The `CONSTRAINT <n>` the user declared, or `None` to take Postgres's auto-name
    /// (`<table>_<columns>_fkey`, chosen by the executor's namer).
    pub name: Option<String>,
    /// The referencing (child) columns on this table, in declared order.
    pub columns: Vec<String>,
    /// The referenced (parent) table.
    pub parent_table: String,
    /// The referenced (parent) columns; empty means "the parent's PRIMARY KEY",
    /// resolved at execute time.
    pub parent_columns: Vec<String>,
    /// What a parent-row delete does to referencing child rows.
    pub on_delete: ReferentialAction,
    /// What an update to a referenced (parent-key) column does to referencing child
    /// rows.
    pub on_update: ReferentialAction,
}

/// One materialized common table expression in a [`Plan::With`]: its name, the
/// materialized-relation key the body's `CteScan`s resolve against, and how the
/// relation is produced.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CtePlan {
    /// The CTE name, used as the materialized-relation key the body's `CteScan`s
    /// resolve against.
    pub name: String,
    /// The CTE's place in its `WITH` list's **analysis order** — declaration order under
    /// a plain `WITH`, the dependency order `TopologicalSort` gives under `WITH
    /// RECURSIVE` — numbered across the whole list, read-only entries (which inline as
    /// [`Plan::SubqueryScan`]s carrying the same numbering) included.
    pub rank: u32,
    /// What produces the relation.
    pub body: CteBody,
}

/// How a [`CtePlan`]'s relation is produced.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum CteBody {
    /// A data-modifying CTE: a DML sub-plan (`Insert`/`Update`/`Delete`, with a
    /// `RETURNING` projection) run once to produce the relation.
    Modifying {
        /// The DML sub-plan (boxed — a `Plan` dwarfs the surrounding fields).
        plan: Box<Plan>,
        /// The CTE's `name(cols)` column-alias list, positionally replacing the
        /// `RETURNING` clause's own output names. Applied by the executor's materializer
        /// through [`alias_relation_columns`] — the same list the reference site's
        /// `Plan::CteScan` carries, because the two derive the relation's columns
        /// independently: the reference site from this plan's `output_columns`, the
        /// materializer from the rows the DML actually produced. Empty for a CTE written
        /// without a list.
        column_aliases: Vec<String>,
    },
    /// A genuinely recursive CTE — `name(cols) AS (<non-recursive> UNION [ALL]
    /// <recursive>)` — evaluated by the fixed-point (semi-naive) algorithm.
    ///
    /// The executor seeds a **working table** from `non_recursive`, then repeatedly
    /// evaluates `recursive` with the CTE name bound to the *current* working table
    /// (an [`EvalEnv`](crate::executor::EvalEnv)
    /// [`MaterializedCte`](crate::executor::MaterializedCte) beside the entries already
    /// materialized), accumulating the new rows and making them the next working table,
    /// until an iteration produces zero new rows. `all` selects `UNION ALL` (keep every
    /// row) vs `UNION` (dedup the new rows against everything seen so far, whole-row
    /// NULL-equal). The accumulated relation is then bound for every later entry and
    /// the body. A self-reference in `recursive` is a [`Plan::CteScan`] that reads the
    /// name from the environment.
    ///
    /// **Types are asymmetric — `non_recursive` is authoritative.** `columns` carries
    /// the relation's column descriptors (names from the `name(cols)` alias list, else
    /// the non-recursive term; types from the non-recursive term); the recursive term's
    /// rows are implicitly cast to those types each iteration (a non-castable pair fails
    /// loud `42804` at plan time).
    Recursive {
        /// The CTE relation's columns (alias-list names, non-recursive-term types) —
        /// the working/result [`MaterializedCte`](crate::executor::MaterializedCte)
        /// shape and the cast target.
        columns: Vec<ColumnDesc>,
        /// The non-recursive (seed) term — authoritative for the column types.
        non_recursive: Box<Plan>,
        /// The recursive term, re-evaluated each iteration against the working table.
        recursive: Box<Plan>,
        /// `true` for `UNION ALL` (keep every row); `false` for `UNION` (dedup).
        all: bool,
    },
}

impl CtePlan {
    /// The sub-plans this entry runs — the DML, or the two recursive terms.
    pub fn plans(&self) -> impl Iterator<Item = &Plan> {
        let (first, second) = match &self.body {
            CteBody::Modifying { plan, .. } => (plan.as_ref(), None),
            CteBody::Recursive {
                non_recursive,
                recursive,
                ..
            } => (non_recursive.as_ref(), Some(recursive.as_ref())),
        };
        std::iter::once(first).chain(second)
    }

    /// [`Self::plans`], mutably.
    pub fn plans_mut(&mut self) -> impl Iterator<Item = &mut Plan> {
        let (first, second) = match &mut self.body {
            CteBody::Modifying { plan, .. } => (plan.as_mut(), None),
            CteBody::Recursive {
                non_recursive,
                recursive,
                ..
            } => (non_recursive.as_mut(), Some(recursive.as_mut())),
        };
        std::iter::once(first).chain(second)
    }
}

/// Apply a FROM item's column-alias list to a relation's columns.
///
/// `aliases[i]` replaces column `i`'s **name**, and a column past the list's end keeps
/// the one the relation gave it. Types and wire metadata are untouched — Postgres's
/// `analyzeCTE` takes the types from `exprType(te->expr)` and never from the list, so
/// `WITH c(v) AS (SELECT 'abc'::text, 5::int8)` is `v text, int8 bigint`.
///
/// **The one derivation.** Every reader of a renamed relation's columns calls this: the
/// plan-side [`Plan::output_columns_scoped`] and [`plan_relations`] arms, the executor's
/// `SubqueryScan` arm and `Plan::With` materializer, `collect_nonbase_into`
/// (`planner/query.rs`, the plan-time binding gate's namespace and `alias.*` expansion)
/// and `push_level_relations` (`planner/correlation.rs`). A list longer than `cols`
/// truncates to `cols` rather than fabricating a column: the parser refuses that `42P10`,
/// but **defers** the refusal to the end of the `WITH` list, so an over-long list does
/// reach here on a statement that is about to be rejected.
#[must_use]
pub fn alias_relation_columns(cols: Vec<ColumnDesc>, aliases: &[String]) -> Vec<ColumnDesc> {
    if aliases.is_empty() {
        return cols;
    }
    cols.into_iter()
        .enumerate()
        .map(|(i, c)| match aliases.get(i) {
            Some(name) => ColumnDesc {
                name: name.clone(),
                ..c
            },
            None => c,
        })
        .collect()
}

/// How an `INSERT` produces its rows.
///
/// Modeling the two forms as a sum type makes the source/values/columns
/// incoherence unrepresentable: the `VALUES` form's rows are already mapped to their
/// columns and carry no mapping that could disagree with them, and the `SELECT` form
/// always pairs its source with one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum InsertRows {
    /// `INSERT … VALUES` — literal row expressions, each row full-width and in the
    /// **table's** column order, which is the order the executor stores them in.
    Values {
        rows: Vec<Vec<Expr>>,
        /// The order a row was **written** in, where that is not the rows' own: the
        /// table-column index of each cell, following the statement's column list.
        /// 18.6 analyses a row's cells in this order, so a reader of that order — the
        /// parameter walk's occurrence timing — reads the rows through it. Empty where
        /// the two orders are one: no column list, a list in the table's order, or a
        /// table the catalog lacks (the rows then stay as written).
        written: Vec<usize>,
    },
    /// `INSERT … SELECT` — a source sub-plan, plus the target table-column index
    /// for each source output column (in source order; from an explicit `(col, …)`
    /// list or positional `0..n`).
    Select {
        source: Box<Plan>,
        columns: Vec<usize>,
    },
}

/// A `LIMIT`/`OFFSET` count: a literal count or a `$N` parameter.
///
/// Postgres types `LIMIT`/`OFFSET` parameters as `bigint`. A `Param` is resolved
/// from the evaluation environment at execution time (the executor sees it and
/// looks up the bound value), not by a plan rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LimitCount {
    /// A literal row count.
    Const(usize),
    /// A `$N` placeholder (1-indexed), resolved from the bound params at execution.
    Param(usize),
}

/// A set-op column's left and right side's enclosing-level reference — the type of the
/// column it binds to and, where that is an enum, the enum's catalog OID — or `None` for a
/// side whose entry named none.
pub type SideReferenceTypes = [Option<(TypeOid, Option<i32>)>; 2];

/// The record a [`Plan::SetOp`] built with a column **pending** an enclosing query level
/// carries, and the parser holds a second handle to.
///
/// A pending column's `column_types` entry is provisional: the node is built while its
/// enclosing levels are still being parsed, so nothing can type the reference yet, and
/// every reader that asks the node for its columns before the correlated re-walk unifies
/// them is handed that provisional type — a scalar subquery's common type in `coalesce`,
/// an enclosing set operation's unification and a derived table's column were each
/// measured to keep it. Those readers are not enumerable, so none is made to wait. The record
/// is how the provisional type is kept from reaching a returned plan at all: the re-walk
/// writes each finalized column's enclosing-level reference types here, and the parser
/// re-parses the statement with them, building the node complete — no reader of that
/// second parse can see a provisional type, because no column of it is pending.
///
/// Shared (not copied) by a clone, so a plan the parser clones — a read-only CTE inlined
/// at each reference — reports to the one record. A second finalization that disagrees
/// with the first is recorded as a conflict rather than overwriting it.
#[derive(Clone)]
pub struct SetOpOuterTypes(std::sync::Arc<std::sync::Mutex<OuterTypesState>>);

struct OuterTypesState {
    /// Per column, the left and right side's enclosing-level reference the column was
    /// unified with ([`SideReferenceTypes`]).
    references: Vec<SideReferenceTypes>,
    /// Per column, whether it is unified — at build, or by the re-walk.
    finalized: Vec<bool>,
    /// Two finalizations of one column disagreed.
    conflict: bool,
}

impl SetOpOuterTypes {
    /// A record for a node of `width` columns, `pending` of them waiting.
    #[must_use]
    pub fn new(width: usize, pending: &[usize]) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(
            OuterTypesState {
                references: vec![[None, None]; width],
                finalized: (0..width).map(|i| !pending.contains(&i)).collect(),
                conflict: false,
            },
        )))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, OuterTypesState> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record the columns the re-walk has just unified, each with its sides' references.
    pub fn resolve(&self, unified: &[(usize, SideReferenceTypes)]) {
        let mut state = self.state();
        for &(column, references) in unified {
            let Some(finalized) = state.finalized.get(column).copied() else {
                state.conflict = true;
                continue;
            };
            if finalized {
                if state.references[column] != references {
                    state.conflict = true;
                }
                continue;
            }
            state.references[column] = references;
            state.finalized[column] = true;
        }
    }

    /// The per-column references to build the node complete with, once every column
    /// is unified and no two finalizations disagreed; `None` otherwise.
    #[must_use]
    pub fn hints(&self) -> Option<Vec<SideReferenceTypes>> {
        let state = self.state();
        (!state.conflict && state.finalized.iter().all(|&f| f)).then(|| state.references.clone())
    }
}

/// Its state is not printed, as [`EnclosingColumns`]'s is not: it is the parser's channel
/// to the statement's second parse, derived from what a plan's `Debug` already prints.
impl std::fmt::Debug for SetOpOuterTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SetOpOuterTypes")
    }
}

/// **Excluded from a plan's equality**: the record is the parser's channel to the
/// statement's second parse, not part of the query the node expresses.
///
/// `Plan` derives `PartialEq`, and the gates that ask whether two plans are the same
/// expression — `SELECT DISTINCT`'s sort keys against its target list, `GROUP BY`'s
/// against it, `DISTINCT ON`'s against `ORDER BY` — compare whole `Plan`s. A subquery
/// written twice in one statement builds a record per occurrence, so an identity
/// comparison made those two `(SELECT i EXCEPT SELECT '7')`s unequal and
/// `SELECT DISTINCT (…) FROM t ORDER BY (…)` was `42P10` where 18.6 answers. Which
/// record a node carries says nothing about the query it expresses, so the field is
/// excluded: every record compares equal to every other.
impl PartialEq for SetOpOuterTypes {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// The kind of set operation combined by a [`Plan::SetOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SetOpKind {
    /// `UNION` — rows in either side.
    Union,
    /// `INTERSECT` — rows in both sides.
    Intersect,
    /// `EXCEPT` — rows in the left side but not the right.
    Except,
}

impl SetOpKind {
    /// The SQL keyword for this set operation (for diagnostics / error messages).
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Union => "UNION",
            Self::Intersect => "INTERSECT",
            Self::Except => "EXCEPT",
        }
    }
}

/// Join type for `Plan::Join`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum JoinType {
    /// INNER JOIN - only matching rows from both sides.
    Inner,
    /// `LEFT [OUTER] JOIN` - all rows from left, matching from right (NULL if no match).
    Left,
    /// `RIGHT [OUTER] JOIN` - all rows from right, matching from left (NULL if no match).
    Right,
    /// `FULL [OUTER] JOIN` - all rows from both sides (NULL if no match on either side).
    Full,
}

/// Transfer direction of a [`Plan::Copy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CopyDirection {
    /// `COPY ... FROM STDIN` — ingest rows from the byte stream (a write).
    From,
    /// `COPY ... TO STDOUT` — emit rows to the byte stream (a read).
    To,
}

/// Wire format of a [`Plan::Copy`].
///
/// Only [`Text`](CopyFormat::Text) is executed; `Csv`/`Binary` parse into
/// the plan (so an explicit `FORMAT csv` is *recognized*, not mistaken for text) and
/// then fail loud at execution with `0A000` — the correct-or-loud contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CopyFormat {
    /// The default Postgres text format (tab-separated, `\N` null, backslash escapes).
    Text,
    /// `FORMAT csv` / bare `CSV` — parsed but not yet executed (fails loud `0A000`).
    Csv,
    /// `FORMAT binary` / bare `BINARY` — parsed but not yet executed (fails loud `0A000`).
    Binary,
}

/// The resolved, owned options of a [`Plan::Copy`] — every field except the transfer
/// `direction`.
///
/// The single owned definition of the COPY field set: embedded in [`Plan::Copy`] and
/// in the server's `CopyInState`, and borrowed as the engine's `CopyParams` (via its
/// `From<&CopyOptions>`). `columns` empty means "all columns, in catalog order". Only
/// [`CopyFormat::Text`] is executed; the parser accepts `csv`/`binary` into `format`
/// and the executor rejects them loud (`0A000`) — never silently treating csv as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CopyOptions {
    /// The target (`FROM`) / source (`TO`) table name.
    pub table: String,
    /// The explicit column list, or empty for all columns in catalog order.
    pub columns: Vec<String>,
    /// The on-wire format; only `Text` is executed.
    pub format: CopyFormat,
    /// The field-separator byte (default tab, `0x09`). Validated single-byte,
    /// non-newline at parse time.
    pub delimiter: u8,
    /// The token a NULL renders as / is detected by (default `\N`).
    pub null_marker: String,
    /// `HEADER`: on `TO`, emit a column-name header line; on `FROM`, skip the first
    /// input line.
    pub header: bool,
}

/// Column definition in CREATE TABLE.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColumnDef {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    /// Resolved `DEFAULT`; `None` = NULL. Always `None` when `implicit_sequence` is
    /// set — a `SERIAL` / identity column's `nextval` default is completed at execute
    /// time, once its sequence has a name.
    pub default: Option<ColumnDefault>,
    /// `true` for a `SERIAL` or `GENERATED … AS IDENTITY` column: the engine supplies
    /// the `nextval` default over a sequence **it** names.
    ///
    /// The parse/catalog split every auto-named object on this engine follows. A
    /// name the engine chose is disambiguated against the live namespace rather than
    /// refused, which a parser holding no catalog cannot do — and a plain `String`
    /// minted at parse time cannot even record *that* the engine chose it. So the
    /// parse side carries this flag with `default: None`, and `execute_create_table`
    /// / `execute_alter_table_add_column` mint through `mint_sequence_name`. A user's
    /// own `DEFAULT nextval('x')` is `default: Some(NextVal("x"))` with this `false`,
    /// and its name is never rewritten.
    pub implicit_sequence: bool,
    /// The identity generation if this column is `GENERATED … AS IDENTITY`, else
    /// `None`. Carried to the catalog `Column.identity`; drives the `428C9`
    /// explicit-value rule.
    pub identity: Option<IdentityGeneration>,
    /// The identity's `START WITH` value (its sequence seeds to `start - 1`), or
    /// `None` for a plain SERIAL / non-identity column (which seeds to `0`).
    pub identity_start: Option<i64>,
    /// The identity's **declared** `SEQUENCE NAME <n>`, or `None` for the engine's
    /// minted `<table>_<column>_seq`.
    ///
    /// The other half of the rule `implicit_sequence` implements. A declared name is
    /// the user's: taken verbatim and refused (`42P07`) if the relation namespace
    /// already holds it, where a minted one is disambiguated. `SERIAL` has no
    /// spelling for it, so this is `None` for every non-identity column.
    pub sequence_name: Option<String>,
}

/// A `PRIMARY KEY` / `UNIQUE` constraint captured at `CREATE TABLE`.
///
/// The executor auto-creates one enforced unique index per constraint (see
/// `execute_create_table`). Both single-column and composite (multi-column)
/// plain constraints are captured. An expression-column constraint is skipped
/// (not enforced as a plain key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UniqueConstraint {
    /// The constrained column names in declaration order (one for single-column,
    /// N for a composite `PRIMARY KEY (a, b)` / `UNIQUE (a, b)`).
    pub columns: Vec<String>,
    /// `true` for `PRIMARY KEY`, `false` for `UNIQUE`.
    pub is_primary: bool,
    /// The `CONSTRAINT <name>` the user declared, or `None` to take Postgres's
    /// auto-name (`<table>_pkey` / `<table>_<col1>_<col2>_…_key`). The constraint's
    /// index is created **under this name** — a constraint and its index share one
    /// name in Postgres, so carrying the declared name here is what makes
    /// `information_schema`, `DROP INDEX`, and the dump agree with it.
    pub name: Option<String>,
}

/// ON CONFLICT clause for INSERT.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OnConflictClause {
    /// The conflict arbiter, or `None` for a **target-less** `ON CONFLICT` — which
    /// arbitrates against *every* unique/PK constraint (Postgres permits only
    /// `DO NOTHING` with no target; the executor handles it via the unique-violation
    /// fall-through rather than a single matched index).
    pub target: Option<ConflictTarget>,
    /// Action to take on conflict.
    pub action: OnConflictAction,
}

/// The arbiter of an `ON CONFLICT (...)` — the unique index it must match.
///
/// Models the full Postgres index-inference target: the key parts (bare columns *or*
/// key expressions like `COALESCE(scope_kind, '')`) plus the
/// optional partial-index predicate (`WHERE revoked_at IS NULL`). The executor
/// matches this against the table's enforced unique indexes (`IndexKey` +
/// `Index.predicate`) to pick the arbiter. The expression/partial shape is exactly
/// what a general multi-dialect parser's bare-column-only conflict target cannot
/// represent — the conformance gap pglet's own parser was built to close.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictTarget {
    /// The arbiter key parts, in written order — and **matched as a set**, so neither
    /// that order nor a repeat reaches the match. A bare column is an `Expr::Column`;
    /// an expression part carries its parsed IR. Both this list and a candidate index's
    /// [`IndexKey`](crate::catalog::IndexKey) are split into their bare-column and
    /// expression halves and compared half by half; `executor::constraints`'
    /// `conflict_target_matches` is that rule, and `docs/indexes.md` states it.
    pub keys: Vec<Expr>,
    /// The partial-index predicate (`WHERE …`), matched against the candidate
    /// index's `predicate`; `None` targets a full (non-partial) index.
    pub predicate: Option<Expr>,
}

/// Action to take when a conflict is detected.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum OnConflictAction {
    /// DO NOTHING - skip the conflicting row.
    DoNothing,
    /// DO UPDATE SET ... - update the existing row.
    DoUpdate {
        /// Column assignments (`column_name`, expression).
        assignments: Vec<(String, Expr)>,
        /// The `DO UPDATE … WHERE` filter, evaluated against the existing
        /// (conflicting) row with `EXCLUDED` bound to the would-be-inserted row.
        /// When it is not true, the conflicting row is left unchanged and the new
        /// row is **not** inserted (Postgres semantics); `None` always applies.
        where_predicate: Option<Expr>,
    },
}

/// Locating a `JOIN … USING` column within one side's column list.
///
/// A base scan names its columns bare (`num`); a nested join's output prepends the
/// merged column bare (`num`) ahead of the raw dotted ones (`t1.num`). So an
/// **exact** bare-name match wins first (the merged / base column), then a **unique**
/// `.<c>` suffix match (a subquery / derived side whose columns arrive dotted). Two
/// or more matches at a tier is Postgres's ambiguous-column case (`42702`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsingMatch {
    /// The column resolves to this single index into the side's row.
    Found(usize),
    /// The column name matches two or more columns of the side (`42702`).
    Ambiguous,
    /// The column does not exist in the side.
    Missing,
}

/// Resolve `c` against a sequence of column `names` using the two-tier
/// exact-then-suffix rule of [`UsingMatch`].
///
/// The single source of the tiering, shared by the `USING` validation
/// ([`using_column_match`], over `ColumnDesc`s) and the executor's unqualified-column
/// resolver (`SimpleColumns::resolve`, over the raw `&str` output names), so the two
/// can't disagree on whether an unqualified name is present / ambiguous. An **exact**
/// bare-name match wins first — the merged `USING` column (a bare name prepended ahead
/// of the raw dotted per-side columns) or a base column — then a **unique** `.<c>`
/// suffix match (a dotted derived-side column). Two or more matches at *either* tier is
/// Postgres's ambiguous-column case (`42702`); zero at both is [`UsingMatch::Missing`].
/// The exact tier runs first (never a combined count) so a `USING` merged column stays
/// unambiguous even though the raw `a.<c>` / `b.<c>` also suffix-match.
#[must_use]
pub fn match_names<'a>(names: impl Iterator<Item = &'a str> + Clone, c: &str) -> UsingMatch {
    let exact: Vec<usize> = names
        .clone()
        .enumerate()
        .filter(|(_, n)| *n == c)
        .map(|(i, _)| i)
        .collect();
    match exact.len() {
        1 => return UsingMatch::Found(exact[0]),
        n if n >= 2 => return UsingMatch::Ambiguous,
        _ => {}
    }
    let suffix = format!(".{c}");
    let matches: Vec<usize> = names
        .enumerate()
        .filter(|(_, n)| n.ends_with(&suffix))
        .map(|(i, _)| i)
        .collect();
    match matches.len() {
        1 => UsingMatch::Found(matches[0]),
        0 => UsingMatch::Missing,
        _ => UsingMatch::Ambiguous,
    }
}

/// Resolve a `USING` column `c` to its index in `columns` (see [`UsingMatch`]).
///
/// Shared by plan-time validation (over [`Plan::output_columns`]) and the executor's
/// join (over the materialized side columns), so the two can't disagree on whether a
/// `USING` column is present / ambiguous. Delegates to [`match_names`].
#[must_use]
pub fn using_column_match(columns: &[ColumnDesc], c: &str) -> UsingMatch {
    match_names(columns.iter().map(|col| col.name.as_str()), c)
}

/// The `(name, type)` pairs of a FROM-clause function item ([`Plan::FunctionScan`]).
///
/// For `unnest(arr1, arr2, …)`, one pair per array argument, named by `columns`
/// (`chooseScalarFunctionAlias`: the explicit column alias, else `unnest`) and typed by
/// the array **expression's** statically inferred element type, `None` where inference
/// declines. For `generate_series`, one pair, named the same way (else
/// `generate_series`) and typed by the overload the parser resolved, which is never
/// `None`.
///
/// **One derivation for the two walkers that must agree about it.** They were separate,
/// and they drifted in exactly the way a `timestamptz` can: that type has no distinct
/// *value* in pglet — it is stored as a UTC `timestamp`, and only the column `TypeOid`
/// carries the tz/no-tz bit — so the executor, which derived the column type from the
/// evaluated `Value::Array`'s `element_type`, declared plain `timestamp` (1114) where
/// this walk declared `timestamptz` (1184) for the same statement. The declared type is
/// what the column-type-aware text encoder reads, so the simple-query leg then rendered
/// the value **without** its UTC `+00` offset and a text-protocol driver reparsed the
/// instant in its own local zone. 18.6 says 1184 with the offset, on both legs
/// (`types/unnest_timestamptz_declared_type.sql`, `types/unnest_timestamptz_text.sql`).
///
/// The two readers differ only in their fallback, which is why this returns an `Option`
/// rather than resolving it: [`Plan::output_columns`] has nothing else to offer and takes
/// `text`, while `executor::query`'s `execute_unnest` holds the evaluated array and takes
/// *its* element type — the branch that keeps a bound `$1` array typed by its value.
pub fn function_scan_column_types(
    func: FromFunction,
    args: &[Expr],
    columns: &[String],
    schema: &Schema,
) -> Vec<(String, Option<TypeOid>)> {
    let name = |i: usize| {
        columns
            .get(i)
            .cloned()
            .unwrap_or_else(|| func.name().to_string())
    };
    match func {
        FromFunction::Unnest => args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                (
                    name(i),
                    a.infer_type(&PlannerScope::none(schema))
                        .and_then(TypeOid::array_element),
                )
            })
            .collect(),
        FromFunction::GenerateSeries(series) => vec![(name(0), Some(series.type_oid()))],
    }
}

/// The output `ColumnDesc` list of base `table` from `schema`, or empty if the table
/// is unknown. The single place [`Plan::output_columns`] turns a catalog table into
/// its column descriptors — the `RETURNING *`, scan, and `qualifier.*` fan-outs.
fn table_columns_desc(schema: &Schema, table: &str) -> Vec<ColumnDesc> {
    schema.get_table(table).map_or_else(Vec::new, |t| {
        t.columns
            .iter()
            .map(|c| ColumnDesc {
                name: c.name.clone(),
                type_oid: c.type_oid,
                // Carry the wire metadata (typmod + base-table provenance) so the
                // Describe `RowDescription` matches the executor's; mirrors
                // `executor::table_column_descs`.
                type_modifier: c.pg_type_modifier(),
                table_oid: t.oid,
                column_id: c.position,
                enum_type: c.enum_type.clone(),
                // A base table's column: declared by the catalog.
                type_certain: true,
            })
            .collect()
    })
}

impl Plan {
    /// The expressions this node projects, or an empty slice for a node that projects
    /// none.
    ///
    /// The three projecting kinds share one target list in Postgres, and every reader
    /// that matches an `ORDER BY` / `DISTINCT ON` key against that list needs it
    /// whichever kind produced it. Written out per caller it drifts, and it drifts
    /// silently: a reader missing the `Window` arm sees an empty target list and
    /// decides every key is unmatched rather than failing.
    pub fn projection_exprs(&self) -> &[Expr] {
        match self {
            Self::Project { exprs, .. } | Self::Aggregate { exprs, .. } => exprs,
            Self::Window { window_exprs, .. } => window_exprs,
            _ => &[],
        }
    }

    /// Get the output columns for this plan without executing it.
    ///
    /// **The lossy read of [`Self::result_shape`]**, and the one every planner-internal
    /// caller wants: what columns does this (sub)plan expose. It answers an empty list
    /// both for a statement that returns no result set and for one that returns a
    /// zero-column result set. A caller that has to tell those apart — every wire reply
    /// does — must take [`Self::result_shape`] instead.
    pub fn output_columns(&self, schema: &Schema) -> Vec<ColumnDesc> {
        self.result_shape(schema).into_columns()
    }

    /// Whether this plan returns a result set, and its columns when it does.
    ///
    /// The fact the wire hosts read: `Describe`'s `NoData`-versus-`RowDescription`, the
    /// simple-`Query` reply's `RowDescription` + `DataRow`s, and the aborted-block
    /// `Describe` gate all key on [`ResultShape::is_result_set`], never on the column
    /// count (`EDGES.md` row 59). A `SELECT` — including one whose `*` expands over a
    /// relation with no columns — always returns a result set; DDL, transaction control
    /// and a DML with no `RETURNING` clause never do.
    pub fn result_shape(&self, schema: &Schema) -> ResultShape {
        self.output_columns_scoped(schema).0
    }

    /// [`Self::result_shape`] **plus** the relations this node puts in scope for the
    /// nodes above it.
    ///
    /// The two travel together because a projecting node needs both, and computing them
    /// separately is what made a qualified reference exponential in nesting depth — see
    /// [`RelationColumns`]. Every arm decides both; an arm that puts nothing nameable in
    /// scope says [`RelationColumns::none`]. **Every arm also decides result-set-ness**,
    /// which is why no *plan* decides it anywhere else: a second enumeration of these
    /// variants is a drift seam, and the one it would drift from is the list below. This
    /// is not the only producer of the fact, though — the executor builds
    /// `QueryResult::shape` at its own sites and nothing compares the two answers; see
    /// `docs/declared_types.md` for the live disagreement.
    ///
    /// **Three arms decide it by assumption rather than from the child.** `Join`,
    /// `SetOp` and `SubqueryScan` read their children through the lossy
    /// [`Self::output_columns`] / [`ResultShape::columns`] and re-wrap in
    /// [`ResultShape::Columns`], so a child answering [`ResultShape::NoResultSet`] would
    /// be silently promoted to a zero-column result set. What picks out *these* three is
    /// that their output column *list* is their children's — concatenated or `USING`-merged
    /// (`Join`), named by the leftmost arm (`SetOp`), passed straight through
    /// (`SubqueryScan`) — so a reader expects the child's shape to flow up with them.
    /// `Project`, `Aggregate` and `Window` do the same lossy read and are deliberately not
    /// on this list: there it resolves their *own* expression lists, which replace the
    /// child's. Nothing reaches *these three*
    /// today — a DML in a join side, a set-op arm or a derived table is `42601` at the
    /// parser — so the assumption is stated rather than guarded. **One spelling narrows
    /// that exclusion, at `Project` alone.** A lone bare `*` over a single relation is
    /// left *unexpanded* (`expand_wildcards`, `planner/query.rs`), and
    /// [`Self::project_output_columns`]'s wildcard branch — the only one of the three
    /// that has one — then extends by the input columns, so that `Project`'s list does
    /// not replace the child's, it *is* the child's, verbatim: the same substitution the
    /// three arms make, over `EDGES.md` row 59's own plan (`Scan` → `Project` of `*`).
    /// It is unreachable for the same reason they are — a `Project`'s child is its
    /// `FROM` plan, where a DML is that same `42601`, and the one data-modifying route a
    /// `FROM` does admit reaches it as a `CteScan` that has re-wrapped already.
    /// The promotion itself is not hypothetical: [`Self::CteScan`] re-wraps the same
    /// way, over a `columns` list the parser filled through the same lossy read
    /// (`parser/select.rs`), and a
    /// `RETURNING`-less data-modifying CTE reaches it — `WITH c AS (INSERT INTO t VALUES
    /// (…)) SELECT * FROM c` describes an empty `RowDescription` and performs the write,
    /// where 18.6 refuses the statement `0A000` at `Parse` (`EDGES.md` row 61). The three
    /// arms' assumption is worth stating because `With` had this same shape, *was*
    /// reachable (`WITH w AS (SELECT 1) DELETE FROM t` returns no result
    /// set and only the body knows it), and was changed to read the child's shape for
    /// exactly that reason.
    pub(crate) fn output_columns_scoped(&self, schema: &Schema) -> (ResultShape, RelationColumns) {
        // The arms that recurse, and only those, are here: each one's own work is in a
        // function of its own, so a plan chain costs this frame a level and not the
        // whole match's (`crate::depth`). The rest are in `output_columns_scoped_rest`.
        match self {
            // Filter/Sort/Distinct/Limit pass through from input — both the columns and
            // the relations, since none of them renames or hides a FROM item. This is
            // the arm a shape-keyed short-circuit missed, and the reason the relations
            // ride *with* the columns rather than being re-derived: adding a wrapper
            // here cannot leave the qualified lookup behind.
            Self::Filter { input, .. }
            | Self::Sort { input, .. }
            | Self::Distinct { input, .. }
            | Self::DistinctOn { input, .. }
            | Self::Limit { input, .. }
            | Self::LockRows { input } => input.output_columns_scoped(schema),
            Self::JunkFilter { input, keep } => {
                Self::junk_filter_output(input.output_columns_scoped(schema), *keep)
            }
            // A `Window`'s `window_exprs` are the same target list a `Project` carries
            // (see `project_output_scoped`).
            Self::Project {
                input,
                exprs,
                certain,
                enclosing,
            }
            | Self::Window {
                input,
                window_exprs: exprs,
                certain,
                enclosing,
            } => Self::project_output_scoped(
                input.output_columns_scoped(schema),
                Targets {
                    exprs,
                    certain,
                    enclosing,
                },
                schema,
            ),
            Self::Aggregate {
                input,
                exprs,
                certain,
                enclosing,
                ..
            } => Self::aggregate_output(
                input.output_columns_scoped(schema),
                Targets {
                    exprs,
                    certain,
                    enclosing,
                },
                schema,
            ),
            Self::Join {
                left, right, using, ..
            } => {
                let left = left.output_columns_scoped(schema);
                let right = right.output_columns_scoped(schema);
                Self::join_output(left, right, using)
            }
            Self::SetOp {
                left,
                right,
                column_types,
                ..
            } => {
                // `output_columns`, spelled so the recursion is this frame alone.
                let left = left.output_columns_scoped(schema).0.into_columns();
                let right = right.output_columns_scoped(schema).0.into_columns();
                Self::set_op_output(left, right, column_types)
            }
            Self::SubqueryScan {
                subquery,
                alias,
                column_aliases,
                cte_rank: _,
            } => Self::subquery_scan_output(
                subquery.output_columns_scoped(schema).0.into_columns(),
                alias,
                column_aliases,
            ),
            // A materialized-CTE statement returns the body's shape — including the
            // body's *result-set-ness*: `WITH w AS (…) DELETE FROM t` returns no result
            // set, and only the body knows that. A recursive entry's `CteScan` carries
            // the CTE's column names/types, so this resolves without the data.
            Self::With { body, .. } => (
                body.output_columns_scoped(schema).0,
                RelationColumns::none(),
            ),
            _ => self.output_columns_scoped_rest(schema),
        }
    }

    /// The junk filter is the one wrapper that *does* hide columns: it trims the
    /// trailing junk ones off. The relations pass through — a junk column is an extra
    /// target-list entry, not a FROM item, so nothing this node drops was ever nameable
    /// through a qualifier.
    #[inline(never)]
    fn junk_filter_output(
        (shape, relations): (ResultShape, RelationColumns),
        keep: usize,
    ) -> (ResultShape, RelationColumns) {
        (
            shape.map_columns(|mut columns| {
                columns.truncate(keep);
                columns
            }),
            relations,
        )
    }

    /// Project returns the projected expressions, and puts nothing nameable in scope —
    /// a bare projection is not a FROM item, and the `SubqueryScan` / `CteScan`
    /// wrapping it is what carries its alias.
    ///
    /// A `Window` returns columns from `window_exprs` (not input + `window_exprs`),
    /// which hold every SELECT item, the plain columns included — the same target list
    /// a `Project` carries, so described here too: a projected `[qualifier.]name` is
    /// looked up qualifier-aware and passes its descriptor through whole, aliased or
    /// not, which is what the executor's `execute_window` sends on the simple leg. A
    /// lookup of its own by bare name took the *first* input column carrying the name,
    /// so `b.k` over `FROM t a, t3 b` was described with `a.k`'s type OID, modifier and
    /// provenance.
    #[inline(never)]
    fn project_output_scoped(
        (input_shape, input_relations): (ResultShape, RelationColumns),
        targets: Targets<'_>,
        schema: &Schema,
    ) -> (ResultShape, RelationColumns) {
        (
            ResultShape::Columns(Self::project_output_columns(
                input_shape.columns(),
                &input_relations,
                targets,
                schema,
            )),
            RelationColumns::none(),
        )
    }

    /// Aggregate returns columns from the output expressions.
    ///
    /// An aggregate's result type follows its *argument's* type, and the
    /// argument may name a column of a derived relation the catalog can't
    /// type (`avg(d) FROM (SELECT d FROM sp) x`) or one behind a FROM alias
    /// (`avg(x.d) FROM sp x`). Resolving through the input scope keeps
    /// `avg(interval)` typed `interval` there instead of falling through to
    /// `float8` — which would advertise one OID and send another type's bytes.
    ///
    /// The items are described as a `Project`'s are ([`Self::project_output_columns`]),
    /// the rule the executor's walk takes for both nodes: a grouped **column** passes
    /// its input descriptor through — its modifier and its base-table provenance — and
    /// every other item goes through `to_column_desc`.
    #[inline(never)]
    fn aggregate_output(
        (input_shape, input_relations): (ResultShape, RelationColumns),
        targets: Targets<'_>,
        schema: &Schema,
    ) -> (ResultShape, RelationColumns) {
        (
            ResultShape::Columns(Self::project_output_columns(
                input_shape.columns(),
                &input_relations,
                targets,
                schema,
            )),
            RelationColumns::none(),
        )
    }

    /// Join returns columns from both sides. An ordinary (`ON`/`CROSS`/comma)
    /// join concatenates them. A `USING` join emits the Postgres-merged shape:
    /// one coalesced column per USING name first (in USING order), then the
    /// left's non-USING columns, then the right's — each USING name appearing
    /// once. (The executor additionally keeps the raw per-side columns
    /// addressable for qualified refs; this clean shape is what an enclosing
    /// `*` / subquery sees.)
    #[inline(never)]
    fn join_output(
        (left_shape, left_relations): (ResultShape, RelationColumns),
        (right_shape, right_relations): (ResultShape, RelationColumns),
        using: &[String],
    ) -> (ResultShape, RelationColumns) {
        let (left_cols, right_cols) = (left_shape.into_columns(), right_shape.columns());
        // Both sides' relations stay addressable by qualifier, left first — the
        // order `left.or_else(right)` had and the order the executor's
        // `JoinColumnMap` resolves in. A `USING` merge reshapes the *columns*,
        // never which relations a qualifier can name.
        let relations = left_relations.chain(right_relations);
        if using.is_empty() {
            let mut cols = left_cols;
            cols.extend_from_slice(right_cols);
            return (ResultShape::Columns(cols), relations);
        }
        let mut cols = Vec::new();
        for c in using {
            // The merged column's type comes from the left side (equal to the
            // right's for a well-typed USING); fall back to Text if unresolved.
            let (type_oid, enum_type) = match (
                using_column_match(&left_cols, c),
                using_column_match(right_cols, c),
            ) {
                (UsingMatch::Found(li), UsingMatch::Found(ri)) => (
                    left_cols[li].type_oid,
                    crate::executor::unified_enum_type(&left_cols[li], &right_cols[ri]),
                ),
                (UsingMatch::Found(li), _) => (left_cols[li].type_oid, None),
                _ => (TypeOid::Text, None),
            };
            // A merged USING column is a coalesce of two sides — not a plain passthrough —
            // and this builds it with no single-column wire metadata. 18.6 does carry some:
            // the sides' agreeing modifier, and for an inner, left or right join a base
            // column's provenance, where this sends `-1` and none (`EDGES.md` row 155). Its
            // **enum** is carried, for the reason `unified_enum_type` states. What a
            // *query* reads is the executor's own merge (`query::join`'s `resolve_using`);
            // this side is the second producer of the same fact, and it is read — a scalar
            // subquery over the merge asks this descriptor, so nulling this arm flips
            // `enum/subquery_over_join_using_member_position` while the set-op fixture
            // stays green. Measured, one arm at a time.
            //
            // Its type is the left side's, which is 18.6's merged type only where that is
            // the sides' common type (`EDGES.md` row 309), and it is left not certain
            // (`ColumnDesc::type_certain`): the reader of a merged name's certainty is
            // the coercion pass's own scope, over the join's inputs
            // (`ColumnScope::column_type_certain`), and a query that reads this join as a
            // relation reads the entry naming the column, which that scope judged.
            cols.push(ColumnDesc {
                enum_type,
                ..ColumnDesc::new(c.clone(), type_oid)
            });
        }
        // `output_columns` names are bare, so a side column is a USING column
        // iff its name is one of the USING names.
        let is_using = |col: &ColumnDesc| using.iter().any(|c| c == &col.name);
        cols.extend(left_cols.iter().filter(|c| !is_using(c)).cloned());
        cols.extend(right_cols.iter().filter(|c| !is_using(c)).cloned());
        (ResultShape::Columns(cols), relations)
    }

    /// A set operation takes its column names from the left side (Postgres names
    /// the combined result after the leftmost SELECT) and its types from the
    /// plan-time unified `column_types`. Neither side's relations survive it —
    /// a set-op result is nameless, as the per-node descent already had it.
    #[inline(never)]
    fn set_op_output(
        left: Vec<ColumnDesc>,
        right: Vec<ColumnDesc>,
        column_types: &[TypeOid],
    ) -> (ResultShape, RelationColumns) {
        (
            ResultShape::Columns(
                left.into_iter()
                    .zip(right)
                    .zip(column_types)
                    // A set-op result column is typed by the unified `column_types`,
                    // not a single source column — no provenance, on 18.6 too. Its
                    // modifier is `-1` here where 18.6 keeps the sides' agreeing one
                    // (`EDGES.md` row 155). Its **enum** is carried, and the executor's
                    // own build reads it through the same derivation.
                    //
                    // Its type is certain where both sides' are: the unified type is
                    // then 18.6's common type of two types that are 18.6's, and the
                    // executor casts each side's values to it.
                    .map(|((l, r), &type_oid)| ColumnDesc {
                        enum_type: crate::executor::unified_enum_type(&l, &r),
                        type_certain: l.type_certain && r.type_certain,
                        ..ColumnDesc::new(l.name, type_oid)
                    })
                    .collect(),
            ),
            RelationColumns::none(),
        )
    }

    /// SubqueryScan returns columns from the subquery — and is the derived
    /// table's alias, so it puts *one* relation in scope under that alias. The
    /// subquery's own relations stop here: `FROM (SELECT … FROM t) x` cannot be
    /// qualified `t.` from outside.
    #[inline(never)]
    fn subquery_scan_output(
        columns: Vec<ColumnDesc>,
        alias: &str,
        column_aliases: &[String],
    ) -> (ResultShape, RelationColumns) {
        let cols = alias_relation_columns(columns, column_aliases);
        (
            ResultShape::Columns(cols.clone()),
            RelationColumns::one(alias.to_owned(), cols),
        )
    }

    #[inline(never)]
    fn output_columns_scoped_rest(&self, schema: &Schema) -> (ResultShape, RelationColumns) {
        match self {
            // DDL statements return no result set at all
            Self::CreateTable { .. }
            | Self::DropTable { .. }
            | Self::Truncate { .. }
            | Self::CreateIndex { .. }
            | Self::DropIndex { .. }
            | Self::CreateType { .. }
            | Self::DropType { .. }
            | Self::DropSchema { .. }
            | Self::CreateSchema { .. }
            | Self::AlterTableAddColumn { .. }
            | Self::AlterTableDropColumn { .. }
            | Self::AlterTableAddCheck { .. }
            | Self::AlterTableAddConstraint { .. }
            | Self::AlterTableSetDefault { .. }
            | Self::AlterTableSetNotNull { .. }
            | Self::AlterTableAddIdentity { .. }
            | Self::CreateSequence { .. }
            | Self::AlterSequence { .. }
            | Self::DropSequence { .. }
            // pg_dump-preamble no-ops report only a command tag.
            | Self::Noop { .. }
            | Self::AlterTableUnperformed { .. }
            | Self::DoBlock { .. }
            // COPY has no in-band result columns (TO/FROM both move bytes
            // out-of-band, returning a `COPY <n>` command tag).
            | Self::Copy { .. }
            | Self::BeginTransaction { .. }
            | Self::CommitTransaction
            | Self::RollbackTransaction
            | Self::Savepoint { .. }
            | Self::RollbackToSavepoint { .. }
            | Self::ReleaseSavepoint { .. }
            | Self::Empty => (ResultShape::NoResultSet, RelationColumns::none()),

            // DML with RETURNING. The namespace is `returning_relations` — the target
            // relation under its effective name plus its `FROM` sources. UPDATE is the
            // only one of the three that can carry an alias, and the only one whose
            // grammar has a `FROM` (an `INSERT … SELECT`'s source is **not** in
            // Postgres's `RETURNING` namespace; `DELETE … USING` is a parser gap).
            Self::Insert {
                table, returning, ..
            }
            | Self::Delete {
                table, returning, ..
            } => (
                Self::returning_result_shape(table, table, None, returning, schema),
                RelationColumns::none(),
            ),
            Self::Update {
                table,
                alias,
                from,
                returning,
                ..
            } => (
                Self::returning_result_shape(
                    table,
                    alias.as_deref().unwrap_or(table),
                    from.as_deref(),
                    returning,
                    schema,
                ),
                RelationColumns::none(),
            ),

            // VALUES returns columns from the first row. A no-FROM level binds no column
            // reference, so each it makes is an enclosing level's, read off the record as
            // a projecting node reads one — and only a recorded column can lend an enum,
            // so the identity walk is skipped where the record is empty.
            Self::Values {
                rows,
                certain,
                enclosing,
            } => {
                let relations = RelationColumns::none();
                let scope = PlannerScope::new(
                    schema,
                    TypeScope::Input {
                        relations: &relations,
                        columns: &[],
                        enclosing: Some(enclosing),
                    },
                );
                let derive_enum = !schema.enums.is_empty() && !enclosing.is_empty();
                (
                    ResultShape::Columns(rows.first().map_or_else(Vec::new, |first_row| {
                        first_row
                            .iter()
                            .enumerate()
                            .map(|(i, e)| {
                                let type_oid = e.infer_type(&scope).unwrap_or(TypeOid::Text);
                                ColumnDesc {
                                    type_certain: certain.holds(i, type_oid),
                                    enum_type: if derive_enum {
                                        scope.expr_enum_shallow(e)
                                    } else {
                                        None
                                    },
                                    ..ColumnDesc::new(e.output_name(), type_oid)
                                }
                            })
                            .collect()
                    })),
                    relations,
                )
            }

            // Scan (and, with acceleration on, the index-scan variants) returns all
            // columns from the table (keyed on the real table name; the alias is for
            // resolution, not catalog lookup) — and puts that table in scope under its
            // effective name (its alias when it has one).
            //
            // A scan always returns a result set, and the empty column list has **two**
            // sources it cannot tell apart: a relation with no columns (`EDGES.md` row
            // 59) and one the catalog does not hold at all, whose `42P01` lands at
            // execute time (rows 29/60). Neither is the absence of a result set — a
            // `SELECT` has one whatever its `*` expands to — so both are
            // `ResultShape::Columns`, and what the fabricated descriptor of a missing
            // relation *contains* stays row 29's subject.
            Self::Scan { table, alias } => {
                let cols = table_columns_desc(schema, table);
                let name = scan_effective_name(table, alias);
                (
                    ResultShape::Columns(cols.clone()),
                    RelationColumns::one(name, cols),
                )
            }
            #[cfg(feature = "index_acceleration")]
            Self::IndexScan { table, alias, .. }
            | Self::IndexRangeScan { table, alias, .. }
            | Self::GinScan { table, alias, .. } => {
                let cols = table_columns_desc(schema, table);
                let name = scan_effective_name(table, alias);
                (
                    ResultShape::Columns(cols.clone()),
                    RelationColumns::one(name, cols),
                )
            }

            // VirtualScan returns the catalog table's columns
            Self::VirtualScan {
                catalog_table,
                alias,
            } => {
                let cols = catalog_table.columns();
                let name = alias
                    .clone()
                    .unwrap_or_else(|| catalog_table.name().to_string());
                (
                    ResultShape::Columns(cols.clone()),
                    RelationColumns::one(name, cols),
                )
            }

            // The arms that recurse are taken by `output_columns_scoped` in its own frame
            // (`crate::depth`): never here.
            Self::Filter { .. }
            | Self::Sort { .. }
            | Self::Distinct { .. }
            | Self::DistinctOn { .. }
            | Self::Limit { .. }
            | Self::LockRows { .. }
            | Self::JunkFilter { .. }
            | Self::Project { .. }
            | Self::Window { .. }
            | Self::Aggregate { .. }
            | Self::Join { .. }
            | Self::SetOp { .. }
            | Self::SubqueryScan { .. }
            | Self::With { .. } => self.output_columns_scoped(schema),

            // ProjectUnnest: one column per projection expr (no FROM, so no input
            // columns to resolve against). An `unnest(arr)` column takes the array's
            // element type; scalars take their own inferred type.
            Self::ProjectUnnest { exprs } => (
                ResultShape::Columns(
                    exprs
                        .iter()
                        .map(|e| e.to_column_desc(schema, TypeScope::NONE))
                        .collect(),
                ),
                RelationColumns::none(),
            ),

            // A CTE reference returns the columns captured from its RETURNING, under the
            // name the enclosing query refers to it by.
            Self::CteScan { alias, columns, .. } => (
                ResultShape::Columns(columns.clone()),
                RelationColumns::one(alias.clone(), columns.clone()),
            ),

            // A FROM function: `unnest`'s one column per array, typed by the array's
            // element type, or `generate_series`'s one column of its overload. An array
            // expression this walk cannot type falls back to `text` here; the executor
            // has the evaluated array and falls back to *its* element type instead,
            // which is the only thing that differs between the two readers of
            // [`function_scan_column_types`].
            //
            // It is a FROM item, so it puts itself in scope under its effective name, as
            // `plan_relations` does: a qualified `u.k` it did not register would be read as
            // an enclosing level's reference rather than this item's column.
            Self::FunctionScan {
                func,
                args,
                columns,
                alias,
                certain,
            } => {
                let cols: Vec<ColumnDesc> = function_scan_column_types(*func, args, columns, schema)
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, type_oid))| ColumnDesc {
                        type_certain: type_oid.is_some_and(|t| certain.holds(i, t)),
                        ..ColumnDesc::new(name, type_oid.unwrap_or(TypeOid::Text))
                    })
                    .collect();
                (
                    ResultShape::Columns(cols.clone()),
                    RelationColumns::one(alias.clone(), cols),
                )
            }

        }
    }

    /// The result shape of a DML statement: no result set for an empty `returning`
    /// list, else that list's columns ([`Self::returning_output_columns`]).
    ///
    /// **The list is post-lowering, so "empty" is not the same question as "the
    /// statement wrote no `RETURNING` clause"** — `lower_returning` replaces a
    /// wildcard with its expansion, and a `RETURNING *` over a relation with no
    /// columns expands to nothing. Such a statement is therefore answered `NoData` and
    /// no `DataRow`s here, which is *not* what 18.6 does with it: 18.6 refuses the
    /// spelling outright at parse analysis (`42601 RETURNING must have at least one
    /// column`, measured on `INSERT … DEFAULT VALUES RETURNING *`, `DELETE …
    /// RETURNING *` and `UPDATE … RETURNING *` over a zero-column relation). Only the
    /// first two of those three arrive here: an `UPDATE` over such a relation has no
    /// column to assign to, so it is refused `42703` on its `SET` target before the
    /// clause is reached. Recorded as `EDGES.md` row 63; a plan that cannot see the
    /// clause cannot decide it, and the gate that would belongs at lowering, not here.
    ///
    /// The `SELECT` side of the same shape *is* decided here and correctly: a scan
    /// always returns a result set, so `SELECT *` over that relation describes
    /// `RowDescription(0)` and sends its rows (row 59).
    fn returning_result_shape(
        table: &str,
        relation: &str,
        from: Option<&Self>,
        returning: &[Expr],
        schema: &Schema,
    ) -> ResultShape {
        if returning.is_empty() {
            return ResultShape::NoResultSet;
        }
        ResultShape::Columns(Self::returning_output_columns(
            table, relation, from, returning, schema,
        ))
    }

    /// Output columns of a DML statement's `RETURNING` list, resolved against the
    /// relations that clause can name ([`returning_relations`]).
    ///
    /// `relation` is the qualifier a target reference must spell (the target's alias
    /// when it has one); `table` is the catalog name its columns come from. They differ
    /// exactly when the statement wrote an alias, and conflating them is what let
    /// `UPDATE ra o … RETURNING ra.b` resolve — a `42P01` on 18.6, since an alias hides
    /// the table name.
    fn returning_output_columns(
        table: &str,
        relation: &str,
        from: Option<&Self>,
        returning: &[Expr],
        schema: &Schema,
    ) -> Vec<ColumnDesc> {
        if returning.is_empty() {
            return Vec::new();
        }
        let relations = returning_relations(table, relation, from, schema);
        let columns = relations.flatten();
        let scope = TypeScope::Input {
            relations: &relations,
            columns: &columns,
            enclosing: None,
        };
        let mut cols = Vec::new();
        for e in returning {
            // A bare `RETURNING *` fans out to the whole namespace. Only a statement
            // that skipped lowering (no target relation) reaches this — `lower_returning`
            // expands every wildcard it sees — and there `columns` is empty anyway.
            if e.is_bare_star() {
                cols.extend(columns.clone());
                continue;
            }
            cols.push(e.to_column_desc(schema, scope));
        }
        cols
    }

    /// Output columns of a `Project`: the descriptor each projection expr contributes.
    ///
    /// A bare `*` fans out to all of `input`'s columns and a `qualifier.*` to that
    /// table's columns (the longest single `output_columns` arm, lifted out). Every
    /// other expr resolves its type against `input`'s columns first (so a CTE /
    /// derived relation the catalog can't type still resolves), falling back to schema
    /// inference; an `Alias` over a column keeps the alias name but the resolved type.
    fn project_output_columns(
        input_cols: &[ColumnDesc],
        input_relations: &RelationColumns,
        Targets {
            exprs,
            certain,
            enclosing,
        }: Targets<'_>,
        schema: &Schema,
    ) -> Vec<ColumnDesc> {
        let scope = TypeScope::Input {
            relations: input_relations,
            columns: input_cols,
            enclosing: Some(enclosing),
        };
        let mut cols = Vec::new();
        for (i, e) in exprs.iter().enumerate() {
            // Star expansion: bare `*` → all input columns; `qualifier.*` → that table.
            if let Expr::Column { table, name } = e
                && name == "*"
            {
                match table {
                    None => cols.extend(input_cols.to_vec()),
                    Some(t) => cols.extend(table_columns_desc(schema, t)),
                }
                continue;
            }
            let infer = || e.to_column_desc(schema, scope);
            let find_input = |table: Option<&str>, name: &str| {
                Self::find_projected_column(
                    input_relations,
                    input_cols,
                    Some(enclosing),
                    table,
                    name,
                )
            };
            let col_desc = match e {
                Expr::Column { table, name } => {
                    find_input(table.as_deref(), name).unwrap_or_else(infer)
                }
                Expr::Alias { expr: inner, alias } => match inner.as_ref() {
                    Expr::Column { table, name } => {
                        // A passthrough column keeps the source column's wire
                        // metadata (type + typmod + provenance), only renaming.
                        find_input(table.as_deref(), name).map_or_else(infer, |c| ColumnDesc {
                            name: alias.clone(),
                            ..c
                        })
                    }
                    _ => infer(),
                },
                _ => infer(),
            };
            // Certain where the coercion pass read the entry, as written, at the type
            // declared for it — a column it passes through included, whose descriptor
            // says what its own relation found and not what this level's scope reads the
            // reference as.
            cols.push(ColumnDesc {
                type_certain: certain.holds(i, col_desc.type_oid),
                ..col_desc
            });
        }
        cols
    }

    /// The input descriptor a projected `[qualifier.]name` passes through, resolved
    /// **qualifier-aware**.
    ///
    /// A qualified `q.c` reads only relation `q`'s `c`. `output_columns` flattens a
    /// join's two sides into one bare-named list, so looking `q.c` up there by name
    /// alone merges the sides: over `a JOIN b` where both carry `x`, every reference —
    /// `a.x` and `b.x` alike — took the **first** `x`'s descriptor, and the extended
    /// protocol's `Describe` declared one relation's OID for the other relation's
    /// bytes (the executor resolves the reference correctly, so it sent the right
    /// value under the wrong declared type — a binary client's decode failure or
    /// garbage). This is the same defect the coercion pass's `ColumnScope` resolves
    /// one layer down, applied to the descriptors instead of the types.
    ///
    /// A **bare** name keeps the flat first-match rule: a name two relations both
    /// carry is Postgres's own `42702`, which the executor raises before the declared
    /// type can matter, and a `USING`-merged name resolves to the merged column
    /// `output_columns` already puts first.
    ///
    /// A reference this level does not bind — a qualifier no relation of the input
    /// answers to, or a bare name no input column carries — is an **enclosing** level's,
    /// and reads `enclosing`, the node's record of the column it binds to. Each of the
    /// kinds a FROM item lowers to registers itself, so a qualifier the input does not
    /// answer is never this level's, and one the record does not hold declines rather than
    /// read a same-named local column, whose type and enum are not the reference's:
    /// `(SELECT o.m FROM tt b) > 'ok'` must not read `tt.m`'s for `o.m`'s value
    /// (`EDGES.md` row 159).
    ///
    /// The lookup is a scan of an already-materialized list, so it costs nothing in the
    /// nesting depth. It used to descend the plan node instead, recomputing the named
    /// relation's `output_columns` once per qualified leaf — see [`RelationColumns`] for
    /// the exponential that produced and the two shapes that reached it.
    fn find_projected_column(
        relations: &RelationColumns,
        input_cols: &[ColumnDesc],
        enclosing: Option<&EnclosingColumns>,
        table: Option<&str>,
        name: &str,
    ) -> Option<ColumnDesc> {
        let local = table.map_or_else(
            || input_cols.iter().find(|c| c.name == name).map(Some),
            |qualifier| {
                relations
                    .lookup(qualifier)
                    .map(|cols| cols.iter().find(|c| c.name == name))
            },
        );
        match local {
            Some(found) => found.cloned(),
            None => enclosing?.lookup(table, name),
        }
    }

    /// Whether this is a top-level sequence-function projection: a **single-row**
    /// no-FROM `VALUES` in which some projected expression is a top-level
    /// `nextval`/`currval`/`setval` (a bare `SequenceFn` or an `Alias`-wrapped one).
    /// Such a plan is executed by `execute_sequence_projection` (with `&mut Schema`
    /// and the per-session state), not on an MVCC snapshot. A sequence function
    /// nested any deeper is not detected here; it fails loud in the read-only eval path.
    ///
    /// The single definition of this predicate — the executor's `Plan::Values` arm
    /// dispatches on it too, so detection and the read-only/persist classification
    /// can't drift.
    pub(crate) fn is_sequence_projection(&self) -> bool {
        matches!(self, Self::Values { rows, .. }
            if rows.len() == 1
                && rows[0].iter().any(|e| e.as_sequence_fn().is_some()))
    }

    /// Whether this is a top-level advisory-lock projection: a **single-row** no-FROM
    /// `VALUES` in which some projected expression is a top-level `pg_advisory_xact_lock`
    /// call (a bare one or an `Alias`-wrapped one) — `SELECT pg_advisory_xact_lock(k)`,
    /// the spelling kysely's migration lock issues.
    ///
    /// The one position the call answers in: [`Self::takes_writer`] reads this, so the
    /// host takes the writer before the statement runs, and the executor's `Plan::Values`
    /// arm evaluates the call on the projection path. A call anywhere else is reached by
    /// the ordinary evaluator, where a non-NULL one is `0A000` because nothing took the
    /// writer for it.
    #[must_use]
    pub fn is_advisory_lock_projection(&self) -> bool {
        matches!(self, Self::Values { rows, .. }
            if rows.len() == 1
                && rows[0].iter().any(|e| e.as_advisory_lock().is_some()))
    }

    /// Whether this sequence projection advances a counter (any top-level
    /// `nextval`/`setval`), so it must persist. `currval`-only projections read
    /// per-session state and write nothing.
    fn advances_sequence(&self) -> bool {
        matches!(self, Self::Values { rows, .. }
            if rows.len() == 1
                && rows[0]
                    .iter()
                    .filter_map(|e| e.as_sequence_fn())
                    .any(|(func, _, _)| func.advances()))
    }

    /// Check if this plan is read-only (no data modifications).
    ///
    /// Read-only plans can use MVCC snapshots for lock-free execution.
    /// Transaction control statements are NOT considered read-only since
    /// they modify connection state.
    // arms grouped by operation category (DDL / DML / transaction control) for documentation; several share the `false` body but are kept separate intentionally
    #[allow(clippy::match_same_arms)]
    pub fn is_read_only(&self) -> bool {
        match self {
            // A top-level sequence-function projection (`SELECT nextval/currval/
            // setval(...)`) is NOT read-only: it must run with `&mut Schema` + the
            // per-session state on the write path, never on a throwaway MVCC-snapshot
            // clone (which would advance a discarded sequence). A top-level
            // `pg_advisory_xact_lock` projection is not read-only either: it takes the
            // writer, which the lock-free path never does. A plain no-FROM `VALUES` is
            // read-only as before.
            Self::Values { .. } => {
                !self.is_sequence_projection() && !self.is_advisory_lock_projection()
            }
            // Pure read operations
            Self::Scan { .. }
            | Self::VirtualScan { .. }
            // A pg_dump-preamble no-op (GRANT/REVOKE/COMMENT) and an ALTER TABLE
            // operation pglet does not perform touch neither rows nor schema shape —
            // each reports a tag, a skip or a refusal.
            | Self::Noop { .. }
            | Self::AlterTableUnperformed { .. }
            | Self::Empty => true,
            // The index-scan variants exist only with acceleration on; all reads.
            #[cfg(feature = "index_acceleration")]
            Self::IndexScan { .. }
            | Self::IndexRangeScan { .. }
            | Self::GinScan { .. } => true,

            // Operators that pass through read-only status from input
            Self::Filter { input, .. }
            | Self::Project { input, .. }
            | Self::Sort { input, .. }
            | Self::Distinct { input, .. }
            | Self::DistinctOn { input, .. }
            | Self::Limit { input, .. }
            | Self::Aggregate { input, .. }
            | Self::JunkFilter { input, .. }
            | Self::SubqueryScan {
                subquery: input, ..
            }
            | Self::Window { input, .. } => input.is_read_only(),

            // A locking read is a read: an auto-commit `SELECT … FOR UPDATE` still runs
            // on the lock-free path, and it is inside a block that its clause takes the
            // writer (`Plan::takes_writer`).
            Self::LockRows { input } => input.is_read_only(),

            // A FROM function writes nothing.
            Self::FunctionScan { .. } => true,

            // A no-FROM projection-unnest reads only its (literal/bound) exprs.
            Self::ProjectUnnest { .. } => true,

            // A CTE reference reads its materialized rows.
            Self::CteScan { .. } => true,

            // A materialized-CTE statement is read-only iff every entry's sub-plans and
            // the body are: a data-modifying entry's DML makes it a write even though
            // the body is a SELECT, and a statement whose entries are all recursive
            // keeps a read-only `WITH RECURSIVE … SELECT` on the lock-free MVCC read path.
            Self::With { ctes, body } => {
                ctes.iter().all(|c| c.plans().all(Self::is_read_only)) && body.is_read_only()
            }

            // Joins are read-only if both inputs are
            Self::Join { left, right, .. } => left.is_read_only() && right.is_read_only(),

            // A set operation is read-only iff both sides are.
            Self::SetOp { left, right, .. } => left.is_read_only() && right.is_read_only(),

            // DDL operations modify schema
            Self::CreateTable { .. }
            | Self::DropTable { .. }
            | Self::Truncate { .. }
            | Self::CreateIndex { .. }
            | Self::DropIndex { .. }
            | Self::CreateType { .. }
            | Self::DropType { .. }
            | Self::DropSchema { .. }
            | Self::CreateSchema { .. }
            | Self::AlterTableAddColumn { .. }
            | Self::AlterTableDropColumn { .. }
            | Self::AlterTableAddCheck { .. }
            | Self::AlterTableAddConstraint { .. }
            | Self::AlterTableSetDefault { .. }
            | Self::AlterTableSetNotNull { .. }
            | Self::AlterTableAddIdentity { .. }
            | Self::CreateSequence { .. }
            | Self::AlterSequence { .. }
            | Self::DropSequence { .. } => false,

            // A DO block runs DDL/DML statements (it writes).
            Self::DoBlock { .. } => false,

            // DML operations modify data
            Self::Insert { .. } | Self::Update { .. } | Self::Delete { .. } => false,

            // COPY TO STDOUT only reads; COPY FROM STDIN writes rows.
            Self::Copy { direction, .. } => matches!(direction, CopyDirection::To),

            // Transaction control modifies connection state
            Self::BeginTransaction { .. }
            | Self::CommitTransaction
            | Self::RollbackTransaction
            | Self::Savepoint { .. }
            | Self::RollbackToSavepoint { .. }
            | Self::ReleaseSavepoint { .. } => false,
        }
    }

    /// The statement's name when Postgres refuses to run it **inside a transaction
    /// block** (`25001`), else `None`.
    ///
    /// The one rule, read by every host: `pglet::Database::run_planned_env` and
    /// `pglet_server::execute`'s `run_one` each ask it of the plan they are about to
    /// execute, having the block state their own way (a `TransactionSession` either
    /// way). Deciding it from the SQL text ahead of the parser is the mistake
    /// `TransactionSession::apply_command`'s doc records; deciding it inside the
    /// executor is not possible, because nothing below the host knows a block is
    /// open.
    ///
    /// `CONCURRENTLY` is the whole membership today, both directions of it, and the
    /// refusal is the *only* thing the keyword does here: pglet has no *table*-scoped
    /// lock for the concurrent form to weaken, and both spellings pass through the same
    /// process-wide locks (stated on the `concurrently` field of
    /// [`Plan::CreateIndex`]), so outside a block the index is built exactly as the
    /// plain spelling builds it and blocks what the plain spelling blocks — measured,
    /// the two produce the same `pg_indexes.indexdef`. Measured on 18.6:
    /// `BEGIN; CREATE INDEX CONCURRENTLY i ON t (c)` is
    /// `25001 CREATE INDEX CONCURRENTLY cannot run inside a transaction block`, as is
    /// the `DROP INDEX CONCURRENTLY` form — even the `IF EXISTS` spelling naming
    /// nothing.
    #[must_use]
    pub const fn forbidden_in_transaction_block(&self) -> Option<&'static str> {
        match self {
            Self::CreateIndex {
                concurrently: true, ..
            } => Some("CREATE INDEX CONCURRENTLY"),
            Self::DropIndex {
                concurrently: true, ..
            } => Some("DROP INDEX CONCURRENTLY"),
            _ => None,
        }
    }

    /// Whether this is transaction control — `BEGIN`, `COMMIT`, `ROLLBACK` and the
    /// savepoint statements — which a host applies to the block rather than runs, and
    /// which takes no snapshot on 18.6.
    #[must_use]
    pub const fn is_transaction_control(&self) -> bool {
        matches!(
            self,
            Self::BeginTransaction { .. }
                | Self::CommitTransaction
                | Self::RollbackTransaction
                | Self::Savepoint { .. }
                | Self::RollbackToSavepoint { .. }
                | Self::ReleaseSavepoint { .. }
        )
    }

    /// Whether a block running this statement takes **the writer** — the one
    /// connection whose block may stage writes, held from the block's first write to its
    /// end — before it runs: every statement that stages a write or writes the catalog
    /// ([`Self::mutates_storage`], a `nextval`/`setval` projection and a `DO` block
    /// among them), a read carrying a row-locking clause ([`Plan::LockRows`]), which
    /// on 18.6 holds a row lock the rest of the block writes under, and a top-level
    /// `pg_advisory_xact_lock` projection ([`Self::is_advisory_lock_projection`]), whose
    /// lock the writer is.
    ///
    /// A host asks it before the statement stages anything. An auto-commit statement
    /// takes the writer for its own duration when it stages a write. An auto-commit
    /// locking read takes it on the native server only where reads run under the write
    /// lock — `--storage memory` — and not on the B+tree's lock-free read path; the
    /// embedded `Database` claims it for every statement this answers `true` for, so there
    /// an auto-commit locking read is refused `55P03` while another logical connection's
    /// block holds it. An advisory-lock projection is not a read and takes the writer on every host,
    /// auto-commit included — for its statement there, as 18.6 holds the lock for the
    /// statement's implicit transaction.
    #[must_use]
    pub fn takes_writer(&self) -> bool {
        self.mutates_storage()
            || matches!(self, Self::LockRows { .. })
            || self.is_advisory_lock_projection()
    }

    /// Whether executing this plan writes data or schema to storage.
    ///
    /// True for DDL and DML (the cases that must persist their meta root on an
    /// auto-commit write); false for reads and for transaction-control
    /// statements (`BEGIN`/`COMMIT`/`ROLLBACK`), which touch only connection
    /// state and never the page store. This is a finer distinction than
    /// `is_read_only`: transaction control is non-read-only yet does not mutate
    /// storage, so an auto-commit persist must gate on `mutates_storage`, not on
    /// `!is_read_only`.
    // arms grouped by category for documentation; several share a body but are kept separate intentionally
    // Not `const`: the `Values` arm walks the projection expressions for a sequence
    // advance (`nextval`/`setval`), which const evaluation can't do over a `Vec`.
    #[allow(clippy::match_same_arms)]
    pub fn mutates_storage(&self) -> bool {
        match self {
            // DDL writes schema/catalog.
            Self::CreateTable { .. }
            | Self::DropTable { .. }
            | Self::CreateIndex { .. }
            | Self::DropIndex { .. }
            | Self::CreateType { .. }
            | Self::DropType { .. }
            // DROP SCHEMA clears the catalog; CREATE SCHEMA is a `public` no-op but
            // is classified with the other DDL (the redundant persist is harmless).
            | Self::DropSchema { .. }
            | Self::CreateSchema { .. }
            | Self::AlterTableAddColumn { .. }
            | Self::AlterTableDropColumn { .. }
            | Self::AlterTableAddCheck { .. }
            | Self::AlterTableAddConstraint { .. }
            | Self::AlterTableSetDefault { .. }
            | Self::AlterTableSetNotNull { .. }
            | Self::AlterTableAddIdentity { .. }
            // A new sequence is catalog state that must persist on auto-commit.
            | Self::CreateSequence { .. }
            | Self::AlterSequence { .. }
            // A DROP SEQUENCE removes a catalog record (and, under CASCADE, a column
            // default), which must persist on auto-commit.
            | Self::DropSequence { .. } => true,

            // DML writes rows; TRUNCATE clears them.
            Self::Insert { .. }
            | Self::Update { .. }
            | Self::Delete { .. }
            | Self::Truncate { .. }
            // A DO block can run DDL/DML (the idempotent `CREATE TYPE`), so its
            // effects must persist on the auto-commit write path.
            | Self::DoBlock { .. } => true,

            // A top-level `SELECT nextval/setval(...)` advances a sequence counter,
            // which is catalog state that must persist on the auto-commit write path
            // (the pg_dump-restore `setval` landmine); a `currval`-only or plain
            // `VALUES` writes nothing.
            Self::Values { .. } => self.advances_sequence(),

            // Reads.
            Self::Scan { .. }
            | Self::VirtualScan { .. }
            | Self::Empty
            | Self::Filter { .. }
            | Self::Project { .. }
            | Self::Sort { .. }
            | Self::Distinct { .. }
            | Self::DistinctOn { .. }
            | Self::JunkFilter { .. }
            | Self::Limit { .. }
            | Self::Aggregate { .. }
            | Self::SubqueryScan { .. }
            | Self::CteScan { .. }
            | Self::Window { .. }
            | Self::FunctionScan { .. }
            | Self::ProjectUnnest { .. }
            // A pg_dump-preamble no-op writes nothing.
            | Self::Noop { .. }
            | Self::AlterTableUnperformed { .. }
            // A set operation only reads (both sides are reads by construction).
            | Self::SetOp { .. }
            | Self::Join { .. } => false,

            // A materialized-CTE statement writes iff one of its entries' sub-plans or
            // its body does — a data-modifying entry's DML always does, a recursive
            // entry's SELECT terms do not.
            Self::With { ctes, body } => {
                ctes.iter().any(|c| c.plans().any(Self::mutates_storage)) || body.mutates_storage()
            }
            // The index-scan variants exist only with acceleration on; none write.
            #[cfg(feature = "index_acceleration")]
            Self::IndexScan { .. }
            | Self::IndexRangeScan { .. }
            | Self::GinScan { .. } => false,

            // COPY FROM STDIN writes rows (must persist on auto-commit); COPY TO
            // STDOUT only reads.
            Self::Copy { direction, .. } => matches!(direction, CopyDirection::From),

            // A locking clause locks no row here and writes nothing.
            Self::LockRows { input } => input.mutates_storage(),

            // Transaction control touches only connection state, never storage.
            //
            // A savepoint plan is transaction control by this test too, and
            // deliberately so even though `ROLLBACK TO` discards staged writes: the
            // predicate gates the auto-commit *persist* wrapper, and every savepoint
            // statement is `25P01` outside a block, so none of them is ever the
            // statement an implicit transaction is opened for.
            Self::BeginTransaction { .. }
            | Self::CommitTransaction
            | Self::RollbackTransaction
            | Self::Savepoint { .. }
            | Self::RollbackToSavepoint { .. }
            | Self::ReleaseSavepoint { .. } => false,
        }
    }

    /// The relation an `ALTER TABLE` plan names and whether the statement carried
    /// `IF EXISTS` — `None` for every other plan.
    ///
    /// Every `ALTER TABLE` plan carries the pair, and the executor reads it here, once,
    /// ahead of the operation: a name no relation holds under `IF EXISTS` is 18.6's
    /// `NOTICE` and skip, before any of the operation's catalog reads, which is where
    /// 18.6's `AlterTableLookupRelation` puts it.
    #[must_use]
    pub fn alter_table_target(&self) -> Option<(&str, bool)> {
        match self {
            Self::AlterTableAddColumn {
                table,
                table_if_exists,
                ..
            }
            | Self::AlterTableDropColumn {
                table,
                table_if_exists,
                ..
            }
            | Self::AlterTableAddCheck {
                table,
                table_if_exists,
                ..
            }
            | Self::AlterTableAddConstraint {
                table,
                table_if_exists,
                ..
            }
            | Self::AlterTableSetDefault {
                table,
                table_if_exists,
                ..
            }
            | Self::AlterTableSetNotNull {
                table,
                table_if_exists,
                ..
            }
            | Self::AlterTableAddIdentity {
                table,
                table_if_exists,
                ..
            }
            | Self::AlterTableUnperformed {
                table,
                table_if_exists,
                ..
            } => Some((table, *table_if_exists)),
            _ => None,
        }
    }

    /// Whether executing this plan changes the schema in a way that **invalidates a
    /// cached query plan**.
    ///
    /// True for the creation and drop of a table, an index, a type, a sequence or a
    /// schema, for the `ALTER TABLE` actions that change a table (not
    /// `AlterTableUnperformed`, which changes nothing), and for a `DO` block, which can
    /// run any of them. DML (which changes rows, not shape), transaction control and
    /// `ALTER SEQUENCE` are `false`: a cached plan stays valid across them. Drives the
    /// `Schema::generation` bump (and thus plan-cache invalidation). `AlterTableAddCheck`
    /// is included too — a cached write plan that re-reads checks from the catalog is
    /// unaffected, but bumping the generation keeps the invalidation conservative and
    /// consistent with the other `ALTER` arms.
    ///
    /// **This is not the "did this statement touch the catalog?" question** — see
    /// [`Plan::mutates_catalog`]. Anything that writes the catalog belongs in
    /// `mutates_catalog` whether or not it belongs here.
    #[must_use]
    pub const fn mutates_schema(&self) -> bool {
        matches!(
            self,
            Self::CreateTable { .. }
                | Self::DropTable { .. }
                | Self::CreateIndex { .. }
                | Self::DropIndex { .. }
                | Self::CreateType { .. }
                | Self::DropType { .. }
                | Self::DropSchema { .. }
                | Self::CreateSchema { .. }
                | Self::AlterTableAddColumn { .. }
                | Self::AlterTableDropColumn { .. }
                | Self::AlterTableAddCheck { .. }
                // ADD CONSTRAINT (PK/UNIQUE/FK) and SET/DROP DEFAULT change a table's
                // shape/defaults, so invalidate any cached plan.
                //
                // A new sequence's name is read where a statement is analyzed: parameter
                // inference refuses an untyped `$N` over a relation that exists, a
                // sequence included, and stands the refusal down over one that does not
                // (`param_types`'s `Inference::relation`). `AlterSequence` moves a
                // start, a counter or an owner, none of which analysis reads, so it is
                // excluded here — but it *is* a catalog write, so it is in
                // `mutates_catalog`.
                //
                // `DropSequence` changes more than a name: under `CASCADE` it clears a
                // column's `DEFAULT`, which is exactly why `AlterTableSetDefault` is
                // here, so a cached INSERT plan over that table must be invalidated.
                | Self::CreateSequence { .. }
                | Self::DropSequence { .. }
                | Self::AlterTableAddConstraint { .. }
                | Self::AlterTableSetDefault { .. }
                | Self::AlterTableSetNotNull { .. }
                | Self::AlterTableAddIdentity { .. }
                // A DO block can run DDL (the idempotent `CREATE TYPE`), so treat it
                // as schema-mutating for plan-cache invalidation.
                | Self::DoBlock { .. }
        )
    }

    /// Whether executing this plan may write the catalog — the question an auto-commit
    /// statement asks before it runs (`pglet::autocommit_is_discardable`): a write the
    /// device refuses at the statement's commit can be dropped, leaving the store
    /// serving, only when its staged writes are the whole of it, and a catalog change is
    /// what an auto-commit statement has no way to take back. A rollback does not read
    /// this: it restores the catalog unconditionally (`pglet::TransactionSession`).
    ///
    /// Everything [`Plan::mutates_schema`] accepts, **plus** the catalog write
    /// deliberately excluded there because it cannot stale a cached plan —
    /// `AlterSequence` (no analysis reads a sequence's start, counter or owner) —
    /// **plus** `Truncate`, which is in neither predicate and is answered from the
    /// catalog below. Calling it a *superset* of `mutates_schema` reads as a structural
    /// guarantee and is not one: the truncate arm `return`s before consulting that
    /// predicate, so a `Truncate` added there would make this one **narrower** instead
    /// of wider. `mutates_catalog_tests` pins the exclusion rather than leaving it to a
    /// comment. Keeping the two predicates separate is what lets the plan-cache
    /// optimization stay (the generation moves only when a cached plan could go stale)
    /// without correctness riding on it: a future catalog mutation that need not bump the
    /// generation belongs here and not in `mutates_schema`.
    ///
    /// **`TRUNCATE` is why this takes a `Schema`.** Whether it writes the catalog is not a
    /// property of the statement: `RESTART IDENTITY` rewrites the truncation closure's
    /// **owned** sequences, and over a table owning none it writes nothing at all. So the
    /// arm asks [`Schema::truncation_targets`], the same derivation `execute_truncate`
    /// writes from; deciding it twice would let this answer and the write disagree, and
    /// the closure makes that reachable in *both* directions (a `CASCADE`-pulled child's
    /// sequence is in the write set, so reading only the named tables under-claims).
    ///
    /// **The remaining over-claim is a class, and the *rule* is the record — a list of
    /// its members is not.** Every arm but `Truncate` answers by the statement **kind
    /// the parser produced**, so an execution of a catalog-writing kind that writes no
    /// catalog claims anyway: an `IF [NOT] EXISTS` skip, an idempotent `ALTER TABLE`
    /// action, a DML-only `DO` body. That is also why the class is not readable off the
    /// SQL text — `ALTER SEQUENCE s OWNER TO r` is in it and `ALTER TABLE t OWNER TO r`
    /// is not, the same clause on two statements, because the second parses to
    /// `Plan::AlterTableUnperformed`. A statement of a catalog-writing kind that *fails* is in the class
    /// deliberately: the catalog write can be partial (the `CREATE TABLE` phantom class),
    /// so the conservative answer is required there. What an over-claim costs is that
    /// answer: a refused write fails the store where it could have been dropped.
    ///
    /// A `DoBlock` carries its statements as `Vec<Plan>` (the body **and** the exception
    /// handler's), so this predicate could recurse with no executor condition to
    /// duplicate; recursing over the body alone would under-claim for a statement whose
    /// catalog the handler wrote. It deliberately does not recurse.
    #[must_use]
    pub fn mutates_catalog(&self, schema: &Schema) -> bool {
        if let Self::Truncate {
            tables,
            cascade,
            restart_identity,
        } = self
        {
            // Not in `mutates_schema` either (a truncate stales no cached plan), so
            // this arm is the whole answer for a `Truncate`. An `Err` — a non-CASCADE
            // truncate of an FK-referenced table — is *no*: the statement fails before
            // writing anything.
            return schema
                .truncation_targets(tables, *cascade, *restart_identity)
                .is_ok_and(|(_, restart_sequences)| !restart_sequences.is_empty());
        }
        self.mutates_schema() || matches!(self, Self::AlterSequence { .. })
    }

    /// The sequences this statement will **replace the counter cell of** — Postgres's
    /// relfilenode swap — with the `last_value` each holds right now.
    ///
    /// # Why a rollback needs this and `mutates_catalog` does not answer it
    ///
    /// A sequence counter is *not* transactional: `nextval` and `setval` write the
    /// current cell in place, so a `ROLLBACK` leaves them where they are (measured on
    /// 18.6 — `BEGIN; nextval → 1; ROLLBACK` then answers `2`). A statement that gives a
    /// sequence a **new** cell is different in kind, and rolling it back puts the old
    /// cell back — with whatever the in-place advances had written into it. The routes
    /// listed here are `ALTER SEQUENCE … RESTART`, `TRUNCATE … RESTART IDENTITY`,
    /// `DROP SEQUENCE`, and the drops that take an owned sequence with its owner:
    /// `DROP TABLE` and `ALTER TABLE … DROP COLUMN` (through `owned_sequences`, the
    /// derivation the drop itself removes them through), and `DROP SCHEMA public`, which
    /// clears every sequence.
    /// That is why the value is captured *before* the statement runs and why the
    /// snapshot's own `last_value` cannot stand in for it: `BEGIN; nextval; nextval;
    /// ALTER SEQUENCE … RESTART WITH 77; nextval; ROLLBACK` answers `3` on 18.6 — the
    /// old cell as the in-place advances left it, not the value at `BEGIN`. An owner drop
    /// followed by a re-create is the same shape: over a table holding ids `1`–`3`,
    /// `BEGIN; DROP TABLE t; CREATE TABLE t (id SERIAL …); INSERT …; ROLLBACK` then
    /// inserts `4` on 18.6, not the re-created cell's next value.
    ///
    /// The list is read before the statement runs, so a statement that then fails lists
    /// the cells its targets hold too. Its entry is read with no in-place advance after
    /// it: the failure aborts the block, so nothing draws before the rollback that reads
    /// the entry, a `ROLLBACK TO` truncates the log past it, and a `DO` body or handler
    /// stops at the failure and its block restores at once.
    ///
    /// Read by the host before executing the statement (`pglet::Database`, the native
    /// server) and handed to `pglet::TransactionSession::record_statement`, and by a
    /// `DO` block for each of its body's and handler's statements
    /// (`executor::execute_do_block`).
    ///
    /// **A `DoBlock` lists nothing here**, and that is not a gap in the list: which of its
    /// statements run — the handler's or not — and against which catalog is decided as it
    /// runs, so no reading ahead of it can say which cells it will replace. The `DO` block
    /// asks each of its statements as it runs them instead, and a `DO` that completes
    /// reports the cells it replaced in its result (`QueryResult::swapped_sequences`),
    /// which the host hands to `record_statement` beside this list and an enclosing `DO`
    /// adds to its own. Walking the body here would read every statement against the
    /// catalog as it stands *before* the `DO`, so a sequence its body creates and then
    /// restarts would be missed and one a handler that never runs replaces would be
    /// listed.
    #[must_use]
    pub fn swapped_sequences(&self, schema: &Schema) -> Vec<(String, i64)> {
        let names: Vec<String> = match self {
            Self::AlterSequence {
                name,
                restart: Some(_),
                ..
            } => vec![name.clone()],
            // Every named sequence gets a new cell, so every one of them is a swap —
            // the list, not its first element. A name the catalog does not hold is
            // filtered out below (an `IF EXISTS` skip, or a phase-1 failure that never
            // reaches execution).
            Self::DropSequence { names, .. } => names.clone(),
            Self::Truncate {
                tables,
                cascade,
                restart_identity,
            } => schema
                .truncation_targets(tables, *cascade, *restart_identity)
                .map(|(_, restart_sequences)| restart_sequences)
                .unwrap_or_default(),
            // The sequences each named table owns, which the drop takes with it. A
            // `CASCADE` pulls no further relation into the set (it sheds a constraint or
            // a default outside it), so the named tables are the whole of it.
            Self::DropTable { names, .. } => names
                .iter()
                .flat_map(|table| owned_sequences(schema, DropTarget::Table { table }))
                .collect(),
            Self::AlterTableDropColumn { table, column, .. } => {
                owned_sequences(schema, DropTarget::Column { table, column })
            }
            // `execute_drop_schema` clears every sequence of the one `public` schema, and
            // any other name drops nothing.
            Self::DropSchema { name, .. } if name == "public" => {
                schema.sequences.keys().cloned().collect()
            }
            _ => Vec::new(),
        };
        names
            .into_iter()
            .filter_map(|n| {
                let last_value = schema.sequences.get(&n)?.last_value;
                Some((n, last_value))
            })
            .collect()
    }
}

#[cfg(test)]
mod mutates_catalog_tests {
    //! `TRUNCATE`'s arm of [`Plan::mutates_catalog`], plus the structural fact the arm's
    //! `return` depends on (`Truncate` is absent from `mutates_schema`) and the kind-only
    //! answer of the other arms. The arm reads the same derivation as
    //! `Plan::swapped_sequences`, which decides what a rollback restores of a sequence;
    //! that half's differential coverage is `transaction/rollback_truncate_restart_identity`,
    //! `rollback_restart_identity_of_a_cascade_child_is_undone`,
    //! `rollback_restart_identity_without_an_owner_keeps_a_sequence_advance` and
    //! `rollback_plain_truncate_keeps_a_sequence_advance`.
    use super::Plan;
    use crate::catalog::{ForeignKey, ReferentialAction, Schema, Sequence, SequenceOwner, Table};

    fn table(name: &str, foreign_keys: Vec<ForeignKey>) -> Table {
        Table {
            name: name.to_string(),
            schema: "public".to_string(),
            columns: Vec::new(),
            oid: 0,
            checks: Vec::new(),
            foreign_keys,
        }
    }

    fn fk(parent: &str) -> ForeignKey {
        ForeignKey {
            name: format!("c_{parent}_fkey"),
            columns: vec!["pid".to_string()],
            parent_table: parent.to_string(),
            parent_columns: vec!["id".to_string()],
            on_delete: ReferentialAction::NoAction,
            on_update: ReferentialAction::NoAction,
        }
    }

    fn owned_sequence(table: &str) -> Sequence {
        Sequence {
            last_value: 0,
            start: 1,
            owner: Some(SequenceOwner {
                table: table.to_string(),
                column: "id".to_string(),
            }),
        }
    }

    fn truncate(tables: &[&str], cascade: bool, restart_identity: bool) -> Plan {
        Plan::Truncate {
            tables: tables.iter().map(|t| (*t).to_string()).collect(),
            cascade,
            restart_identity,
        }
    }

    /// Four tables, each isolating one cell: `p` owns nothing and nothing references
    /// it; `q` owns `q_seq`; `r` owns nothing but is FK-referenced by `c`, which owns
    /// `c_seq`. Keeping `p` free of children matters — a table with children answers
    /// *no* under a bare `TRUNCATE` for the wrong reason (the `0A000`, not the empty
    /// restart set), which would make the headline cell test nothing.
    fn schema() -> Schema {
        let mut schema = Schema::default();
        for name in ["p", "q", "r"] {
            schema.tables.insert(name.to_string(), table(name, vec![]));
        }
        schema
            .tables
            .insert("c".to_string(), table("c", vec![fk("r")]));
        schema
            .sequences
            .insert("q_seq".to_string(), owned_sequence("q"));
        schema
            .sequences
            .insert("c_seq".to_string(), owned_sequence("c"));
        schema
    }

    #[test]
    fn restart_identity_over_a_sequenceless_table_writes_no_catalog() {
        // The bug this arm exists for: two keywords apart from a plain TRUNCATE, over a
        // table owning no sequence, `execute_truncate` writes nothing — so claiming a
        // catalog write made ROLLBACK revert an unrelated `nextval`.
        assert!(!truncate(&["p"], false, true).mutates_catalog(&schema()));
        assert!(!truncate(&["p"], true, true).mutates_catalog(&schema()));
    }

    #[test]
    fn restart_identity_over_an_owned_sequence_writes_the_catalog() {
        assert!(truncate(&["q"], false, true).mutates_catalog(&schema()));
        // ...and one owner anywhere in the list is enough.
        assert!(truncate(&["p", "q"], false, true).mutates_catalog(&schema()));
    }

    #[test]
    fn restart_identity_sees_a_cascade_pulled_childs_sequence() {
        // `r` owns nothing, but CASCADE pulls in `c`, whose sequence is restarted — so
        // a gate reading only the *named* tables would under-claim and let that restart
        // outlive its own ROLLBACK. The closure is the write set on both sides.
        assert!(truncate(&["r"], true, true).mutates_catalog(&schema()));
    }

    #[test]
    fn plain_truncate_never_writes_the_catalog() {
        for tables in [&["p"][..], &["q"][..], &["p", "q"][..]] {
            assert!(!truncate(tables, false, false).mutates_catalog(&schema()));
            assert!(!truncate(tables, true, false).mutates_catalog(&schema()));
        }
    }

    #[test]
    fn a_truncate_of_a_name_that_does_not_resolve_writes_no_catalog() {
        // The gate is asked before the executor resolves anything, so it is asked about
        // names that need not exist. A missing table contributes no owned sequence, and
        // the statement is `42P01` before any write — so *no* is both what this answers
        // and what the execution does. The mixed list is the discriminating half: one
        // real owner anywhere still claims, so a missing name cannot suppress a
        // restart that will happen.
        assert!(!truncate(&["nosuch"], false, true).mutates_catalog(&schema()));
        assert!(!truncate(&["nosuch"], true, true).mutates_catalog(&schema()));
        assert!(truncate(&["nosuch", "q"], false, true).mutates_catalog(&schema()));
    }

    #[test]
    fn truncate_is_absent_from_mutates_schema() {
        // `mutates_catalog`'s truncate arm `return`s before consulting `mutates_schema`,
        // so this predicate is only *wider* than that one while `Truncate` stays out of
        // it: adding `Truncate` there (for plan-cache invalidation, say) would make a
        // plain `TRUNCATE` claim there and still answer *no* here — narrower, not
        // wider, and in the direction both of this pair's recorded silent bugs came
        // from. The doc says so; nothing but this coupled it.
        for restart_identity in [false, true] {
            assert!(!truncate(&["q"], false, restart_identity).mutates_schema());
        }
    }

    #[test]
    fn a_truncate_that_cannot_run_writes_no_catalog() {
        // `c` references `r` and CASCADE was not given, so the statement is `0A000`
        // before it reaches the restart loop — even though the closure CASCADE *would*
        // have built owns a sequence (the line below).
        assert!(!truncate(&["r"], false, true).mutates_catalog(&schema()));
        assert!(truncate(&["r"], true, true).mutates_catalog(&schema()));
    }

    #[test]
    fn the_other_arms_stay_kind_only() {
        // A schema-mutating plan and a generation-neutral catalog write both claim
        // without consulting the catalog; a read does not.
        assert!(
            Plan::DropSequence {
                names: vec!["q_seq".to_string()],
                if_exists: false,
                cascade: false,
            }
            .mutates_catalog(&schema())
        );
        let alter = Plan::AlterSequence {
            name: "q_seq".to_string(),
            if_exists: false,
            start: None,
            restart: None,
            owned_by: None,
        };
        assert!(!alter.mutates_schema());
        assert!(alter.mutates_catalog(&schema()));
        assert!(!Plan::BeginTransaction { isolation: None }.mutates_catalog(&schema()));
    }
}

#[cfg(test)]
mod match_names_tests {
    use super::{UsingMatch, match_names, using_column_match};
    use crate::types::{ColumnDesc, TypeOid};

    fn m(names: &[&str], c: &str) -> UsingMatch {
        match_names(names.iter().copied(), c)
    }

    #[test]
    fn single_table_bare_names_never_ambiguous() {
        // A single scan's schema-unique bare names → tier 1 gives 0 or 1.
        assert_eq!(m(&["id", "name", "num"], "num"), UsingMatch::Found(2));
        assert_eq!(m(&["id", "name"], "missing"), UsingMatch::Missing);
    }

    #[test]
    fn dotted_join_columns_collide_ambiguously() {
        // Both sides carry `num` (dotted) → suffix tier sees two `.num` → 42702.
        assert_eq!(m(&["a.num", "b.num"], "num"), UsingMatch::Ambiguous);
        // Only one side has `name` → a single suffix match resolves.
        assert_eq!(m(&["a.num", "b.name"], "name"), UsingMatch::Found(1));
    }

    #[test]
    fn using_merged_bare_short_circuits_before_suffix() {
        // The `USING` merged column is a bare name prepended ahead of the raw
        // per-side dotted columns — tier 1 (exact) returns it before the suffix
        // tier ever counts the two `.num`, so USING stays unambiguous.
        assert_eq!(m(&["num", "a.num", "b.num"], "num"), UsingMatch::Found(0));
    }

    #[test]
    fn two_exact_bare_names_are_ambiguous() {
        assert_eq!(m(&["num", "num"], "num"), UsingMatch::Ambiguous);
    }

    #[test]
    fn using_column_match_delegates_over_column_descs() {
        let cols = vec![
            ColumnDesc::new("num", TypeOid::Int4),
            ColumnDesc::new("a.num", TypeOid::Int4),
            ColumnDesc::new("b.num", TypeOid::Int4),
        ];
        // Same exact-first tiering as `match_names`, over `ColumnDesc`s.
        assert_eq!(using_column_match(&cols, "num"), UsingMatch::Found(0));
        assert_eq!(using_column_match(&cols[1..], "num"), UsingMatch::Ambiguous);
    }
}

#[cfg(test)]
mod set_op_outer_types_tests {
    use super::SetOpOuterTypes;
    use crate::types::TypeOid;

    /// A record hands the second parse its reference types only once **every** pending
    /// column is unified: a column the re-walk never reached has no type to build with,
    /// and building the node complete without it would put a guess where the enclosing
    /// level's answer belongs.
    #[test]
    fn a_record_hints_only_a_fully_unified_set_op() {
        let record = SetOpOuterTypes::new(2, &[1]);
        assert_eq!(record.hints(), None);
        record.resolve(&[(1, [Some((TypeOid::Numeric, None)), None])]);
        assert_eq!(
            record.hints(),
            Some(vec![[None, None], [Some((TypeOid::Numeric, None)), None]])
        );
    }

    /// Two finalizations of one column that disagree are a conflict, not an overwrite —
    /// the plan the record belongs to was cloned, and neither answer is the node's.
    #[test]
    fn a_record_that_is_finalized_twice_differently_hints_nothing() {
        let record = SetOpOuterTypes::new(1, &[0]);
        record.resolve(&[(0, [Some((TypeOid::Numeric, None)), None])]);
        assert!(record.hints().is_some());
        record.resolve(&[(0, [Some((TypeOid::Int4, None)), None])]);
        assert_eq!(record.hints(), None);
    }
}
