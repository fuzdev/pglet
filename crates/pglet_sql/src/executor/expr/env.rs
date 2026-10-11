//! The evaluation environment: subquery callbacks, the `EvalEnv` frame chain and its outer scopes.

use super::columns::{ColumnRef, ScopeIdentity};
use crate::catalog::Schema;
use crate::planner::{Expr, Plan, UsingMatch};
use crate::types::{ColumnDesc, TypeOid, Value};
use crate::{Error, Result};
use pglet_core::ToastStatus;

/// Callback type for executing scalar subqueries.
///
/// The callback receives a Plan and should execute it, returning a single Value.
/// If the subquery returns no rows, the callback should return `Value::Null`.
/// If the subquery returns multiple rows/columns, the callback should return an error.
pub type SubqueryFn<'a> = Option<&'a dyn Fn(&Plan) -> Result<Value>>;

/// Callback type for executing IN subqueries.
///
/// The callback receives a Plan and should execute it, returning all values from
/// the first column of all rows. Used for `expr IN (SELECT ...)` evaluation.
pub type InSubqueryFn<'a> = Option<&'a dyn Fn(&Plan) -> Result<Vec<Value>>>;

/// Callback type for executing `EXISTS(SELECT ...)` subqueries.
///
/// The callback receives a Plan and should execute it, returning `true` iff the
/// sub-plan produced at least one row (the `NOT EXISTS` negation is applied by
/// the caller, not the callback). Used for `[NOT] EXISTS(SELECT ...)` evaluation.
pub type ExistsFn<'a> = Option<&'a dyn Fn(&Plan) -> Result<bool>>;

/// The refusal of an aggregate the walk reaches with no value lent for it.
pub(super) const AGGREGATE_NOT_COMPUTED: &str =
    "aggregate function evaluated where no aggregation computes it";

/// Callback type for reading an aggregate's value for the group being evaluated.
///
/// The callback receives an `Expr::Aggregate` node and answers the value the grouping
/// executor computed for **that node** over the current group — keyed by the node's
/// identity, never by its spelling — or `None` for a node it computed nothing for. The
/// expression above the aggregate is then evaluated by the ordinary walk, and the plan is
/// never rewritten to carry the value (`executor::query::aggregate`).
pub type AggregateFn<'a> = Option<&'a dyn Fn(&Expr) -> Option<Value>>;

/// The subquery-execution callbacks available while evaluating an expression.
///
/// Bundled so a new capability is added as one field here instead of re-threading
/// every `eval_expr_full` call site. Each field stays independently optional: a
/// caller may supply only the scalar callback (and leave IN/EXISTS `None`), so
/// this is a struct of `Option`s, not an `Option<SubqueryExec>`. **Correlated**
/// subqueries need no extra field here — the outer scope rides the [`EvalEnv`]
/// threaded alongside `sub`; the callback runs the unchanged inner plan and a
/// correlated column resolves against the env at eval time (`eval_expr_subquery`).
#[derive(Clone, Copy)]
pub struct SubqueryExec<'a> {
    /// Executes a scalar subquery `(SELECT …)` → a single `Value`.
    pub scalar: SubqueryFn<'a>,
    /// Executes an `IN (SELECT …)` subquery → its first-column values.
    pub in_list: InSubqueryFn<'a>,
    /// Executes `EXISTS (SELECT …)` → whether ≥1 row.
    pub exists: ExistsFn<'a>,
    /// Reads an aggregate's value for the current group → the computed `Value`.
    ///
    /// Lent only by the grouping executor, to the expressions above its aggregates. A
    /// sub-plan's evaluation builds its own `SubqueryExec`, so an inner query's
    /// aggregate is never answered from an outer query's group.
    pub aggregate: AggregateFn<'a>,
}

impl SubqueryExec<'_> {
    /// No subquery callbacks — encountering any subquery is then an error.
    pub const NONE: SubqueryExec<'static> = SubqueryExec {
        scalar: None,
        in_list: None,
        exists: None,
        aggregate: None,
    };
}

// ============================================================================
// Evaluation environment
// ============================================================================

/// A data-modifying CTE's materialized result.
///
/// The `RETURNING` rows captured when the wrapping
/// [`Plan::With`](crate::planner::Plan::With) ran the DML once, plus their column
/// descriptors. A [`Plan::CteScan`](crate::planner::Plan::CteScan) in the body reads
/// these by name from the [`EvalEnv`].
#[derive(Debug, Clone)]
pub struct MaterializedCte {
    /// The CTE name (the `CteScan` lookup key).
    pub name: String,
    /// The relation's columns (from the DML's `RETURNING` projection).
    pub columns: Vec<ColumnDesc>,
    /// The captured rows.
    pub rows: Vec<Vec<Value>>,
}

/// Runtime context an expression resolves against that is **not** part of the
/// row being evaluated: the bound parameters and the enclosing (outer) query
/// scope for correlated subqueries.
///
/// Threaded by value through every recursive `eval_expr_full` call (it is `Copy`,
/// like [`SubqueryExec`]) and through `execute_plan_node`, so an `Expr::Parameter`
/// and a correlated `Expr::Column` resolve against this context **at eval time**
/// instead of being baked into the plan by mutation. No global/thread-local —
/// passed explicitly.
#[derive(Debug, Clone, Copy, Default)]
pub struct EvalEnv<'a> {
    /// Decoded bind parameters, indexed by `$N - 1`. `Expr::Parameter { index }`
    /// resolves to `params[index - 1]` (a `None` slot is a SQL NULL). Empty on the
    /// simple-query path (no params) — a `$N` then fails loud, exactly as the
    /// executor's prior unbound-parameter arm did.
    params: &'a [Option<Value>],
    /// Innermost enclosing query scope, or `None` at top level. A cons-list of
    /// frames (each links to its parent) so a doubly-nested subquery still sees
    /// every outer scope. A correlated `Expr::Column` that the inner scope doesn't
    /// claim resolves against these frames (see [`EvalEnv::resolve_outer_slot`]).
    outer: Option<&'a OuterFrame<'a>>,
    /// Materialized data-modifying-CTE results in scope (empty outside a
    /// [`Plan::With`](crate::planner::Plan::With) body). A
    /// [`Plan::CteScan`](crate::planner::Plan::CteScan) resolves its rows here by
    /// name — the DML ran once when the `With` arm built this slice.
    ctes: &'a [MaterializedCte],
    /// The live catalog, when the evaluating context has one to lend.
    ///
    /// Two expressions are functions **of the catalog** rather than of the row —
    /// `pg_get_constraintdef(oid)` and the `::regclass` cast — and neither can be
    /// answered from a `Value`. `None` is the correct state for a context that has no
    /// catalog to lend rather than a default: those two then fail loud (`0A000`)
    /// instead of guessing.
    ///
    /// **Lent at two seams, and it takes both.** `execute_plan_node` covers every read
    /// path, and `eval_expr_subquery_with` covers the DML clauses that never descend
    /// through a plan node — an `UPDATE`/`DELETE` `WHERE`, a `RETURNING` item. Three
    /// further sites lend it *per evaluation* rather than once, because the enclosing
    /// function also holds `&mut schema.sequences` for a `DEFAULT`: `INSERT … VALUES`,
    /// `UPDATE … SET` (`assign_row`), and `ON CONFLICT DO UPDATE`'s `SET` and
    /// `WHERE`. Covering only the first seam left every one of those `0A000` while the
    /// identical expression in a `SELECT` answered — a boundary neither the doc nor
    /// `EDGES.md` described. A site that evaluates an expression without lending is a
    /// silent re-opening of that gap. `catalog/regclass_in_dml_positions.sql` drives the
    /// `INSERT … VALUES`, `INSERT … SELECT`, `UPDATE … SET`, `UPDATE … WHERE` and
    /// `DELETE … WHERE` positions and `catalog/regclass_in_returning.sql` the `RETURNING`
    /// one; `ON CONFLICT DO UPDATE`'s `SET` and `WHERE` are lent (measured — the clause
    /// answers `'oc'::regclass::text` at both) and pinned by nothing.
    schema: Option<&'a Schema>,
    /// The store's answer to whether a table's rows are all within 18.6's TOAST
    /// threshold ([`pglet_core::Storage::rows_untoasted`]), which `pg_column_size` needs
    /// for a stored value. Lent where a plan node runs over a store
    /// ([`Self::with_toast_status`]); `None` elsewhere, where every table is "not known"
    /// and a stored value is refused rather than sized.
    toast: Option<ToastLens<'a>>,
}

/// A borrowed [`ToastStatus`], `Copy` like the environment that holds it.
#[derive(Clone, Copy)]
pub struct ToastLens<'a>(&'a dyn ToastStatus);

impl std::fmt::Debug for ToastLens<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ToastLens")
    }
}

/// One enclosing query scope captured when descending into a subquery: the outer
/// row plus the metadata needed to resolve a correlated reference against it.
#[derive(Debug, Clone, Copy)]
pub struct OuterFrame<'a> {
    /// The outer row's column names (its inner namespace).
    col_names: &'a [&'a str],
    /// The outer scope's **declared** column types, positionally parallel to
    /// `col_names`, or empty when the pushing scope has none to hand.
    ///
    /// Carried for the same reason [`ColumnResolver::column_type`](super::columns::ColumnResolver::column_type) exists: a
    /// correlated `'v=' || o.tz` / `o.tz::text` stringifies an *outer* column, and
    /// only its declared type says whether the text keeps the `+00` offset. Without
    /// it the outer reference rendered offset-less while the identical inner
    /// reference did not — a silent instant shift, and (under `||` in a predicate)
    /// a silently wrong result set.
    col_types: &'a [TypeOid],
    /// The outer scope's enum member orders, positionally parallel to `col_names`, or
    /// empty when the pushing scope has none to hand.
    ///
    /// Carried for the same reason [`Self::col_types`] is, one type further: a
    /// correlated `o.m > 'sad'` orders an *outer* enum column, and only its enum's
    /// member list says that is a position comparison rather than a byte one. Without
    /// it the outer reference ordered lexically while the identical inner reference did
    /// not.
    col_enums: &'a [Option<&'a [String]>],
    /// What this scope knows about the relations behind `col_names` (see
    /// [`ScopeIdentity`]) — what binds a qualified correlated ref (`c.id`) to the right
    /// frame, and what decides whether the frame is offered the forgeable synthesized
    /// addressing at all.
    identity: ScopeIdentity<'a>,
    /// The current outer row.
    row: &'a [Value],
    /// The next scope out (for multi-level correlation).
    parent: Option<&'a Self>,
}

impl<'a> OuterFrame<'a> {
    /// Build a frame for the scope `(col_names, col_types, table, row)`, linked to
    /// `parent` (the current `EvalEnv::outer`). Pass an empty `col_types` when the
    /// scope carries no declared types — a correlated reference into it then falls
    /// back to value-only rendering.
    #[must_use]
    pub const fn new(
        col_names: &'a [&'a str],
        col_types: &'a [TypeOid],
        col_enums: &'a [Option<&'a [String]>],
        table: Option<&'a str>,
        row: &'a [Value],
        parent: Option<&'a Self>,
    ) -> Self {
        Self {
            col_names,
            col_types,
            col_enums,
            identity: ScopeIdentity::from_table(table),
            row,
            parent,
        }
    }

    /// Build a frame for a **multi-relation** scope addressed by `(relation, column)`
    /// pairs — a join's namespace, and an `UPDATE … FROM`'s `RETURNING` namespace.
    ///
    /// `col_names` is still the row's own addressing (the descriptors a client sees, and
    /// what a `ProjectionScope` reads); `refs` is what resolution goes through, and is
    /// positionally parallel to it. The two spellings are not interchangeable: a
    /// synthesized `"a.b"` name and a column a user quoted a dot into are byte-identical,
    /// and only the pair tells them apart.
    #[must_use]
    pub const fn over_relations(
        col_names: &'a [&'a str],
        col_types: &'a [TypeOid],
        col_enums: &'a [Option<&'a [String]>],
        refs: &'a [ColumnRef],
        row: &'a [Value],
        parent: Option<&'a Self>,
    ) -> Self {
        Self {
            col_names,
            col_types,
            col_enums,
            identity: ScopeIdentity::Relations(refs),
            row,
            parent,
        }
    }

    /// Build a frame for the scope `identity` describes, linked to `parent` — the form
    /// a caller that already holds a [`ScopeIdentity`] uses, so the frame and the inner
    /// resolver cannot disagree about what the scope is.
    #[must_use]
    pub const fn over_scope(
        col_names: &'a [&'a str],
        col_types: &'a [TypeOid],
        col_enums: &'a [Option<&'a [String]>],
        identity: ScopeIdentity<'a>,
        row: &'a [Value],
        parent: Option<&'a Self>,
    ) -> Self {
        Self {
            col_names,
            col_types,
            col_enums,
            identity,
            row,
            parent,
        }
    }

    /// Whether this frame's namespace contains the relation `t` — the ownership test
    /// [`EvalEnv::resolve_outer_slot`] stops its walk on.
    ///
    /// A [`ScopeIdentity::Single`] frame *is* one relation, so it owns exactly its own
    /// name; a [`ScopeIdentity::Relations`] frame owns every relation its pairs name.
    /// A [`ScopeIdentity::Anonymous`] frame is offered the synthesized-name test — it
    /// owns every relation one of its `"t."`-prefixed names spells — which is forgeable
    /// by a user's quoted `"a.b"` column. That arm is what a scope whose relations could
    /// not be derived still gets; the collision is recorded in `EDGES.md` and closed by handing
    /// the frame its pairs, not by this
    /// predicate.
    fn owns(&self, t: &str) -> bool {
        match self.identity {
            ScopeIdentity::Single(name) => name == t,
            ScopeIdentity::Relations(refs) => refs.iter().any(|r| r.relation.as_deref() == Some(t)),
            ScopeIdentity::Anonymous => {
                let prefix = format!("{t}.");
                self.col_names.iter().any(|c| c.starts_with(&prefix))
            }
        }
    }

    /// This frame's verdict on the qualified reference `t.name`. Only meaningful where
    /// [`owns`](Self::owns) holds.
    ///
    /// A pair-addressed frame matches the `(relation, column)` pair, which no string can
    /// forge; the other two match the spelling a column of `t` takes in `col_names` —
    /// the bare name in a single-relation frame, the synthesized `"t.name"` in one
    /// addressing several.
    fn claim_qualified(&self, t: &str, name: &str) -> UsingMatch {
        if let ScopeIdentity::Relations(refs) = self.identity {
            return refs
                .iter()
                .position(|r| r.relation.as_deref() == Some(t) && r.name == name)
                .map_or(UsingMatch::Missing, UsingMatch::Found);
        }
        let key = match self.identity {
            ScopeIdentity::Single(_) => name.to_string(),
            _ => format!("{t}.{name}"),
        };
        self.claim_exact(&key)
    }

    /// This frame's verdict on a **bare** `name`: its position, [`UsingMatch::Ambiguous`]
    /// when two or more of its columns claim it, [`UsingMatch::Missing`] when none does.
    ///
    /// A pair-addressed frame counts the *bare-visible* columns whose own name is `name`
    /// — `scanRTEForColumn` over a join's namespace, so a join carrying one `w` answers a
    /// correlated bare `w` (18.6 does; matching the synthesized `"a.w"` string could not)
    /// and one carrying two is `42702`.
    ///
    /// The other two frames count **exact** matches on `col_names`. The suffix tier
    /// `match_names` adds for an inner join scope has no place there: a `USING`-merged
    /// `id` is one exact claimant sitting ahead of the raw `a.id` / `b.id`, so a merged
    /// join resolves, while a suffix match over a frame carrying `"a.c"` and `"b.c"` as
    /// *user* column names calls a bare `c` ambiguous where 18.6 resolves it further out.
    fn claim_bare(&self, name: &str) -> UsingMatch {
        if let ScopeIdentity::Relations(refs) = self.identity {
            let mut found = None;
            for (i, r) in refs.iter().enumerate() {
                if r.bare_visible && r.name == name {
                    if found.is_some() {
                        return UsingMatch::Ambiguous;
                    }
                    found = Some(i);
                }
            }
            return found.map_or(UsingMatch::Missing, UsingMatch::Found);
        }
        self.claim_exact(name)
    }

    /// The position of the single column named exactly `key`, `Ambiguous` for two or
    /// more, `Missing` for none.
    fn claim_exact(&self, key: &str) -> UsingMatch {
        let mut found = None;
        for (i, c) in self.col_names.iter().enumerate() {
            if *c == key {
                if found.is_some() {
                    return UsingMatch::Ambiguous;
                }
                found = Some(i);
            }
        }
        found.map_or(UsingMatch::Missing, UsingMatch::Found)
    }

    /// This frame's value at `i` — a slot past the row's end is a SQL NULL, matching
    /// the inner resolvers' own out-of-range handling.
    pub(super) fn value_at(&self, i: usize) -> Value {
        self.row.get(i).cloned().unwrap_or(Value::Null)
    }

    /// This frame's **declared** type at `i`, or `None` when it was pushed without
    /// types (the caller then falls back to value-only rendering).
    pub(super) fn type_at(&self, i: usize) -> Option<TypeOid> {
        self.col_types.get(i).copied()
    }

    /// This frame's enum member order at `i`, or `None` for a non-enum column (and for
    /// a frame pushed without them — the caller then orders by label bytes).
    pub(super) fn enum_order_at(&self, i: usize) -> Option<&'a [String]> {
        *self.col_enums.get(i)?
    }
}

/// What the outer-scope chain says about one correlated column reference — the
/// four-way verdict [`EvalEnv::resolve_outer_slot`] returns, so its two readers (the
/// value in `resolve_column`, the declared type in `column_declared_type`) cannot
/// disagree about which frame answered or whether one did.
///
/// The middle two arms are the ones a two-state `Option` could not spell, and both
/// decided rows before this existed: a reference an owning frame cannot answer used to
/// rebind to a *different* frame's bare column, and an inner level's ambiguity used to
/// escalate outward instead of raising.
#[derive(Debug)]
pub(super) enum OuterSlot<'a> {
    /// A frame claims the reference at this position in its row.
    Found(&'a OuterFrame<'a>, usize),
    /// A frame owns the qualifier and has no such column — `42703 "t.name"`.
    Claimed,
    /// Two or more exact claimants at the innermost claiming level — `42702`.
    Ambiguous,
    /// No frame claims it; the inner resolver's own verdict stands.
    Unclaimed,
}

impl<'a> EvalEnv<'a> {
    /// No parameters, no outer scope — the simple-query / no-FROM / constraint
    /// context.
    pub const EMPTY: EvalEnv<'static> = EvalEnv {
        params: &[],
        outer: None,
        ctes: &[],
        schema: None,
        toast: None,
    };

    /// Build an environment over the portal's bound parameter values.
    #[must_use]
    pub const fn with_params(params: &'a [Option<Value>]) -> Self {
        Self {
            params,
            outer: None,
            ctes: &[],
            schema: None,
            toast: None,
        }
    }

    /// Derive an environment that lends `schema` to the catalog-reading expressions
    /// (`pg_get_constraintdef`, `::regclass`), keeping everything else.
    #[must_use]
    pub const fn with_schema<'b>(&self, schema: &'b Schema) -> EvalEnv<'b>
    where
        'a: 'b,
    {
        EvalEnv {
            params: self.params,
            outer: self.outer,
            ctes: self.ctes,
            schema: Some(schema),
            toast: self.toast,
        }
    }

    /// Derive a child environment that lends `status` — the store the evaluating plan
    /// node reads — to `pg_column_size` ([`Self::rows_untoasted`]).
    #[must_use]
    pub const fn with_toast_status<'b>(&self, status: &'b dyn ToastStatus) -> EvalEnv<'b>
    where
        'a: 'b,
    {
        EvalEnv {
            params: self.params,
            outer: self.outer,
            ctes: self.ctes,
            schema: self.schema,
            toast: Some(ToastLens(status)),
        }
    }

    /// Whether `table`'s every row is known to be within 18.6's TOAST threshold — `false`
    /// where no store is lent, which is "not known".
    #[must_use]
    pub fn rows_untoasted(&self, table: &str) -> bool {
        self.toast
            .is_some_and(|ToastLens(status)| status.rows_untoasted(table))
    }

    /// The live catalog, or `None` where the evaluating context has none.
    #[must_use]
    pub const fn schema(&self) -> Option<&'a Schema> {
        self.schema
    }

    /// The innermost enclosing scope (for building a child frame's `parent`).
    #[must_use]
    pub const fn outer_scope(&self) -> Option<&'a OuterFrame<'a>> {
        self.outer
    }

    /// Derive a child environment that adds `frame` as the new innermost outer
    /// scope (keeping the same params). Used when descending into a correlated
    /// subquery — the analogue of pushing the outer row onto the env.
    #[must_use]
    pub const fn push<'b>(&self, frame: &'b OuterFrame<'b>) -> EvalEnv<'b>
    where
        'a: 'b,
    {
        EvalEnv {
            params: self.params,
            outer: Some(frame),
            ctes: self.ctes,
            schema: self.schema,
            toast: self.toast,
        }
    }

    /// Derive a child environment that exposes `ctes` as the materialized
    /// data-modifying-CTE results (keeping the same params and outer scope). Built
    /// once by the [`Plan::With`](crate::planner::Plan::With) executor before running
    /// the body, so a [`Plan::CteScan`](crate::planner::Plan::CteScan) can read them.
    #[must_use]
    pub const fn with_ctes<'b>(&self, ctes: &'b [MaterializedCte]) -> EvalEnv<'b>
    where
        'a: 'b,
    {
        EvalEnv {
            params: self.params,
            outer: self.outer,
            ctes,
            schema: self.schema,
            toast: self.toast,
        }
    }

    /// Look up a materialized data-modifying CTE by name (the `CteScan` resolution).
    #[must_use]
    pub fn cte(&self, name: &str) -> Option<&'a MaterializedCte> {
        self.ctes.iter().find(|c| c.name == name)
    }

    /// Locate a correlated column reference in the outer-scope chain, mirroring
    /// Postgres's `colNameToVar`: the walk runs innermost-out, and **the first frame
    /// that owns the reference stops it, resolved or not** — a reference an owning
    /// frame cannot answer is an error, never a rebinding further out.
    ///
    /// A frame **owns** a qualifier `t` when it *is* the relation `t` (an identity
    /// frame — `table == Some(t)`) or when it addresses `t` among several (an
    /// identity-less frame — a join scope or a multi-relation `RETURNING` scope —
    /// carrying a synthesized name starting `"t."`). An owning frame answers its
    /// column, or [`OuterSlot::Claimed`], which the caller reports as
    /// `42703 "t.name"`. No frame owning `t` at any level is
    /// [`OuterSlot::Unclaimed`], and the caller falls back to the inner resolver's own
    /// verdict — `42P01` for a resolver that has a relation identity.
    ///
    /// Rebinding past an owner is what this rule removes, and it **wrote**: over
    /// `a(id)` and `b(v)`, `DELETE FROM a WHERE EXISTS (SELECT 1 FROM b WHERE EXISTS
    /// (SELECT 1 FROM k WHERE k.k = a.v))` deleted every row here — frame `a` owns the
    /// qualifier and lacks `v`, so the reference fell to the bare loop and took frame
    /// `b`'s — where 18.6 answers `42703 column a.v does not exist`, hinting `b.v`.
    ///
    /// **The identity split decides which addressing a frame is offered, and says
    /// nothing about why a frame lacks an identity** — the exactness
    /// `SimpleColumns::resolve` gates its own dotted tier with, for the same reason. A
    /// frame with a `table` takes its names straight from that relation, so a dotted
    /// entry there is a user's quoted `"a.b"` column and *not* this addressing; the
    /// two are byte-identical. Ungated, such a column answered a qualified reference
    /// that meant something else: over `q1(id, "a.b")` and `q2(yv)`, `DELETE FROM q1
    /// WHERE EXISTS (SELECT 1 FROM q2 WHERE q2.yv = a.b)` **deleted a row** where 18.6
    /// raises `42P01` and both rows survive. The converse does not hold — a frame can
    /// lack an identity because its call site never worked the name out, and then it
    /// answers for relations the statement never named. Nothing here can detect that;
    /// `single_table_name`'s doc carries the obligation a new FROM kind inherits.
    ///
    /// A **bare** name resolves at the innermost frame carrying it, counting *exact*
    /// matches only (see [`OuterFrame::claim_bare`]): one is the answer, two or more are
    /// [`OuterSlot::Ambiguous`] (`42702`, `scanRTEForColumn`). Frames at different
    /// levels are shadowing rather than ambiguity, so the walk continues past a frame
    /// with no exact match — `SELECT … FROM a WHERE EXISTS (SELECT 1 FROM b WHERE
    /// EXISTS (SELECT 1 FROM k WHERE k.k = v))` takes `b`'s `v`, as on 18.6.
    ///
    /// The single scoping rule behind the value and the declared type
    /// (`resolve_column` and [`column_declared_type`](super::eval::column_declared_type)), so the two can't bind to
    /// different frames — and both take the *same* four-way match, since a `Some`
    /// where the value path raises renders one column as another's type.
    pub(super) fn resolve_outer_slot(&self, table: Option<&str>, name: &str) -> OuterSlot<'a> {
        let mut frame = self.outer;
        while let Some(f) = frame {
            let claim = match table {
                // An owning frame stops the walk whatever its answer; a frame that
                // owns nothing here is simply skipped.
                Some(t) if f.owns(t) => f.claim_qualified(t, name),
                Some(_) => UsingMatch::Missing,
                None => f.claim_bare(name),
            };
            match claim {
                UsingMatch::Found(i) => return OuterSlot::Found(f, i),
                UsingMatch::Ambiguous => return OuterSlot::Ambiguous,
                UsingMatch::Missing => {
                    if table.is_some_and(|t| f.owns(t)) {
                        return OuterSlot::Claimed;
                    }
                }
            }
            frame = f.parent;
        }
        OuterSlot::Unclaimed
    }

    /// Resolve `$index` (1-indexed) to its bound `Value`; a `None` slot is
    /// `Value::Null`.
    pub(super) fn param(&self, index: usize) -> Result<Value> {
        if index == 0 || index > self.params.len() {
            return Err(Error::Execution(format!(
                "bind message supplies {} parameters, but statement requires ${index}",
                self.params.len()
            )));
        }
        Ok(self.params[index - 1].clone().unwrap_or(Value::Null))
    }

    /// Resolve a `LIMIT`/`OFFSET` `$index` to a row count: a NULL value clears the
    /// bound (Postgres treats NULL `LIMIT`/`OFFSET` as "no limit"/"no offset"),
    /// and a non-integer or negative value fails loud.
    pub(crate) fn limit_param(&self, index: usize) -> Result<Option<usize>> {
        match self.param(index)? {
            Value::Null => Ok(None),
            v => value_to_count(&v).map(Some),
        }
    }
}

/// Coerce an integer value to a non-negative row count for `LIMIT`/`OFFSET`.
fn value_to_count(value: &Value) -> Result<usize> {
    let n: i64 = match value {
        Value::Int2(n) => i64::from(*n),
        Value::Int4(n) => i64::from(*n),
        Value::Int8(n) => *n,
        other => {
            return Err(Error::Execution(format!(
                "LIMIT/OFFSET must be an integer, got {:?}",
                other.type_oid()
            )));
        }
    };
    usize::try_from(n).map_err(|_| Error::Execution("LIMIT/OFFSET must not be negative".into()))
}
