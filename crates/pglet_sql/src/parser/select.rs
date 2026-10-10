//! SELECT parsing, fused into the planner [`Plan`] IR.
//!
//! `SELECT <projection> FROM <from> [WHERE] [GROUP BY] [HAVING] [ORDER BY]
//! [LIMIT] [OFFSET]`, plus the no-FROM form (`SELECT 1`). The per-clause IR is
//! handed to the shared [`lower_select`], which owns index selection,
//! aggregate/window classification, ORDER BY pushdown (`Sort` elision), and
//! LIMIT/OFFSET — so SELECT parsing and lowering stay cleanly separated.
//!
//! Set operations (`UNION`/`INTERSECT`/`EXCEPT`, optionally `ALL`) chain leaf
//! selects at Postgres precedence (`INTERSECT` binds tighter than `UNION`/`EXCEPT`,
//! which are left-associative and equal), with one trailing `ORDER BY`/`LIMIT`
//! bound to the whole chain — built into a [`Plan::SetOp`] via
//! [`Parser::build_set_op`](super::Parser::build_set_op).

use crate::catalog::CatalogTable;
use crate::lexer::{Keyword, TokenKind};
use crate::planner::{
    CertainTypes, ColumnScope, EnclosingColumns, Expr, FromFunction, JoinType, LimitCount, Literal,
    OrderKey, Plan, PlannerScope, SelectParts, SetOpKind, TypeScope, UsingMatch,
    certain_unnest_element_type, contains_aggregate, contains_unnest, contains_window,
    lower_select, plan_no_from_distinct_on, plan_no_from_order_limit, plan_set_op_order_limit,
    resolve_correlated_subplans, resolve_entries_gated, unify_values_columns, using_column_match,
};
use crate::planner::{ResolvedType, select_common_type, select_common_typmod};
use crate::types::{ColumnDesc, TypeOid};
use crate::{CatalogError, Error, Result};

use super::Parser;

/// A FROM-clause table reference as `(name, alias)` — the unit of `SelectParts.tables`.
type FromTable = (String, Option<String>);

/// A parsed leaf `SELECT` (a `simple_select`, no trailing ORDER BY/LIMIT), the unit
/// a set-op chain combines. A FROM-bearing leaf is still un-lowered `SelectParts` so
/// the caller can either fill its own ORDER BY/LIMIT (a single select, preserving
/// index pushdown) or lower it with empty ones (a set-op operand); a no-FROM leaf
/// (`SELECT 1`) is already a finished `Plan`.
enum SelectLeaf {
    /// A FROM-bearing leaf, `order_by`/`limit` left for the caller to fill.
    FromParts(Box<SelectParts>),
    /// A no-FROM leaf (`SELECT 1`), already lowered to `Values`/`ProjectUnnest` — under
    /// the leaf's own quantifier where it carries one (`Plan::Distinct`, or the
    /// `Plan::DistinctOn` over a `Plan::Sort` that `plan_no_from_distinct_on` builds).
    /// The **trailing** `ORDER BY`/`LIMIT`/`OFFSET` is not in it: that binds to the whole
    /// select rather than to the leaf, so it is applied by the caller.
    NoFrom(Box<Plan>),
}

/// The clauses over a whole query that `gram.y`'s `insertSelectOptions` merges into the
/// query they follow when that query is parenthesized: to 18.6, `(SELECT … LIMIT 1) OFFSET
/// 1` is one query carrying both, and a clause both halves carry is refused
/// ([`Self::merge`]). This record is all that is merged here: the plan applies the
/// parenthesized query's own clauses before the ones written after it (`EDGES.md` row
/// 167).
#[derive(Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)] // independent facts, one per clause the merge reads
pub(super) struct QueryClauses {
    pub(super) order_by: bool,
    pub(super) offset: bool,
    /// A `LIMIT` — `LIMIT ALL` included — or a `FETCH`.
    pub(super) limit: bool,
    pub(super) with_ties: bool,
    pub(super) skip_locked: bool,
}

impl QueryClauses {
    /// `insertSelectOptions`: merge `outer`, the clauses written after a parenthesized
    /// query carrying `self`, into it — refusing, in its order, an `ORDER BY`, an
    /// `OFFSET` or a `LIMIT` both carry, a `WITH TIES` in `outer` that the merged query
    /// does not sort, and one beside a merged `SKIP LOCKED`. A query with no
    /// parenthesized query to merge into merges into the default, which refuses only
    /// the last two.
    ///
    /// # Errors
    ///
    /// `42601`, with 18.6's message.
    pub(super) fn merge(self, outer: Self) -> Result<Self> {
        let merged = Self {
            order_by: self.order_by || outer.order_by,
            offset: self.offset || outer.offset,
            limit: self.limit || outer.limit,
            with_ties: self.with_ties || outer.with_ties,
            skip_locked: self.skip_locked || outer.skip_locked,
        };
        let message = match (self, outer) {
            (a, b) if a.order_by && b.order_by => "multiple ORDER BY clauses not allowed",
            (a, b) if a.offset && b.offset => "multiple OFFSET clauses not allowed",
            (a, b) if a.limit && b.limit => "multiple LIMIT clauses not allowed",
            (_, b) if b.with_ties && !merged.order_by => {
                "WITH TIES cannot be specified without ORDER BY clause"
            }
            (_, b) if b.with_ties && merged.skip_locked => {
                "SKIP LOCKED and WITH TIES options cannot be used together"
            }
            _ => return Ok(merged),
        };
        Err(Error::Syntax {
            message: message.into(),
            position: None,
        })
    }
}

/// The distinct quantifier on a `SELECT` — an enum rather than a `bool` so
/// `DISTINCT ON` can carry its key expressions.
enum DistinctClause {
    /// No quantifier (every row kept).
    No,
    /// `SELECT DISTINCT` — dedup whole projected rows.
    All,
    /// `SELECT DISTINCT ON (e1[, e2…])` — keep the first row per ON-key group.
    On(Vec<Expr>),
}

/// The constraint following a `JOIN` factor, before it is resolved to the `(on,
/// using)` a `Plan::Join` carries (see [`Parser::resolve_join_constraint`]).
enum JoinConstraint {
    /// `JOIN … ON <predicate>` — the predicate is the row match.
    On(Expr),
    /// `JOIN … USING (c1, c2, …)` — the shared column names.
    Using(Vec<String>),
    /// `CROSS JOIN …` (and the comma-join) — a Cartesian product (`ON true`).
    Cross,
}

/// Validate that USING column `c` denotes exactly one column of a join side's
/// output. Missing → `42703`; two or more → ambiguous (`42702`) — Postgres's rule
/// that a USING name resolve to a single column per side.
fn check_using_side(cols: &[ColumnDesc], c: &str) -> Result<()> {
    match using_column_match(cols, c) {
        UsingMatch::Found(_) => Ok(()),
        UsingMatch::Ambiguous => Err(Error::AmbiguousColumn {
            column: c.to_string(),
        }),
        UsingMatch::Missing => Err(CatalogError::ColumnNotFoundInExpr {
            column: c.to_string(),
        }
        .into()),
    }
}

/// Which row-count clause is being parsed — selects the Postgres-matching SQLSTATE
/// for a negative count (`2201W` for LIMIT, `2201X` for OFFSET).
#[derive(Clone, Copy)]
enum LimitClause {
    Limit,
    Offset,
}

impl LimitClause {
    const fn name(self) -> &'static str {
        match self {
            Self::Limit => "LIMIT",
            Self::Offset => "OFFSET",
        }
    }

    const fn negative_error(self) -> Error {
        match self {
            Self::Limit => Error::NegativeLimit,
            Self::Offset => Error::NegativeOffset,
        }
    }
}

/// Parse a `LIMIT`/`OFFSET` integer literal to a non-negative row count, matching
/// Postgres's numeric→int8 coercion: a fractional literal rounds to nearest
/// (ties away from zero), and a value past the representable range fails loud (`22003`),
/// instead of the old `.parse::<usize>().ok()` that silently dropped both to
/// "unbounded". A negative literal is rejected earlier (the `-` token), so `raw`
/// here is unsigned.
fn parse_row_count(raw: &str) -> Result<usize> {
    // A plain non-negative integer within Postgres's bigint (i64) range. (Bounding
    // by i64 — not usize — keeps `LIMIT 9223372036854775808` out of range like PG,
    // and the cast can't truncate on wasm32 where usize is 32-bit.)
    if let Ok(n) = raw.parse::<i64>()
        && n >= 0
    {
        return usize::try_from(n).map_err(|_| Error::NumericOutOfRange("bigint"));
    }
    // A fractional literal rounds to nearest, ties away from zero, like Postgres's
    // numeric→int8 coercion (`0.5` → 1, `2.5` → 3). `f64::round` is half-away-from-zero.
    if raw.contains(['.', 'e', 'E'])
        && let Ok(f) = raw.parse::<f64>()
    {
        let rounded = f.round();
        if (0.0..=i64::MAX as f64).contains(&rounded) {
            return usize::try_from(rounded as i64).map_err(|_| Error::NumericOutOfRange("bigint"));
        }
    }
    Err(Error::NumericOutOfRange("bigint"))
}

impl Parser<'_> {
    /// Parse a `SELECT` query into a [`Plan`] (shared by top-level statements and
    /// scalar / `EXISTS` / `IN` subqueries). Depth-guarded.
    pub(super) fn parse_select(&mut self) -> Result<Plan> {
        self.enter()?;
        let result = self.parse_select_inner();
        self.leave();
        // A query nested in another is a unit the enclosing one walks — a derived
        // table's scope, a subquery's type — so it is measured here, before that walk
        // ([`crate::depth`]). A top-level one is measured with its whole statement.
        let plan = result?;
        if self.depth > 0 {
            crate::depth::check(crate::depth::plan_depth(&plan) + self.depth)?;
        }
        Ok(plan)
    }

    fn parse_select_inner(&mut self) -> Result<Plan> {
        // A leading `(` is a parenthesized select-with-parens as the first operand
        // (`(SELECT …) UNION …`, `(SELECT … ORDER BY … LIMIT …) EXCEPT …`). Parse it,
        // then any set-op chain / trailing ORDER BY / LIMIT binds to the whole result.
        //
        // With no set operation after it, the parenthesized query and the clauses after
        // it are one query to 18.6, so a clause both carry is `42601`
        // (`QueryClauses::merge`); the plan still applies the operand's own clauses first
        // (`EDGES.md` row 167).
        // What the query carries is read off `carried_clauses` before anything else can
        // parse a query and overwrite it.
        if self.ts.at(&TokenKind::LParen) {
            let first = self.parse_setop_operand()?;
            let inner = std::mem::take(&mut self.carried_clauses);
            let set_op = self.at_set_op();
            let tree = if set_op {
                self.parse_set_op_chain(first)?
            } else {
                first
            };
            let (order_by, limit, outer) = self.parse_query_clauses()?;
            self.carried_clauses = if set_op {
                outer
            } else {
                inner.merge(outer).map_err(|e| self.before_analysis(e))?
            };
            return plan_set_op_order_limit(tree, order_by, limit, self.schema);
        }

        let first = self.parse_select_leaf()?;

        // A set operator turns this into a chain: `q1 {UNION|INTERSECT|EXCEPT} [ALL]
        // q2 …`, with a single trailing ORDER BY/LIMIT bound to the whole result.
        if self.at_set_op() {
            let left = self.finalize_leaf(first)?;
            let tree = self.parse_set_op_chain(left)?;
            let (order_by, limit, clauses) = self.parse_query_clauses()?;
            self.carried_clauses = clauses;
            return plan_set_op_order_limit(tree, order_by, limit, self.schema);
        }

        // A single (non-set-op) select: finalize with its own trailing ORDER
        // BY/LIMIT, which flow through `lower_select` (index pushdown, Sort elision).
        match first {
            SelectLeaf::NoFrom(plan) => {
                // This arm used to consume and **discard** the `ORDER BY` and
                // `LIMIT`/`OFFSET` it parses, and to never reach the row-level locking
                // clause at all. Each was a measured divergence rather than a no-op —
                // `SELECT 1 AS q LIMIT 0` answered a row against 18.6's `(0 rows)` with
                // no error on either engine, `SELECT 1 AS q ORDER BY zzz` answered
                // against its `42703`, and `SELECT 1 FOR UPDATE` was `42601` against its
                // row. The bound's write polarities moved rows through all three of
                // `DELETE`/`UPDATE`/`INSERT … SELECT` and the sort key's through the two
                // measured (`DELETE`, `UPDATE`). The tail now routes through
                // `plan_no_from_order_limit` (which also *binds* the keys, since a Sort
                // over a `Values` judges them only where a row reaches the eval-time
                // resolver) and the locking clause is consumed as its sibling arms
                // consume it. The paren-leading arm two arms up is the control that
                // localised all three: `(SELECT 1 AS q) LIMIT 0`,
                // `(SELECT 1 AS q) ORDER BY 5` and `(SELECT 1 AS q) FOR UPDATE` each
                // already agreed with 18.6 (measured).
                let (order_by, limit, clauses) = self.parse_query_clauses()?;
                self.carried_clauses = clauses;
                let mut plan = plan_no_from_order_limit(
                    *plan,
                    order_by,
                    limit,
                    self.schema,
                    self.scope_is_complete(),
                )?;
                // A no-FROM select never reaches `lower_select`, so nothing else runs
                // the correlated pass over the sub-plans its projection may embed —
                // and a scalar subquery there is a nested level whose own scope was
                // provisional at its lowering. Without this, `SELECT (SELECT 1 FROM t
                // WHERE zzz.k = 1)` over an **empty** `t` answered a row where 18.6
                // raises `42P01`: the eval-time gate had nothing to evaluate and the
                // plan-time one never ran. This level's namespace is empty and it
                // encloses nothing, so its chain is complete by construction. It runs
                // over the *routed* plan so a sub-plan inside a sort key is reached too.
                resolve_correlated_subplans(&mut plan, self.schema, self.scope_is_complete())?;
                Ok(plan)
            }
            SelectLeaf::FromParts(mut parts) => {
                let clauses;
                (parts.order_by, parts.limit, clauses) = self.parse_query_clauses()?;
                self.carried_clauses = clauses;
                lower_select(*parts, self.schema)
            }
        }
    }

    /// Parse one leaf `SELECT` — a `simple_select` with **no** trailing ORDER
    /// BY/LIMIT (those bind to the enclosing set-op chain, or are applied by
    /// [`Self::parse_select_inner`] for a single select).
    ///
    /// A FROM-bearing leaf is returned as un-lowered [`SelectParts`] (its
    /// `order_by`/`limit` left empty for the caller to fill); a no-FROM leaf
    /// (`SELECT 1`) is already a finished [`Plan`].
    fn parse_select_leaf(&mut self) -> Result<SelectLeaf> {
        self.ts.expect_kw(Keyword::Select)?;
        let (distinct, distinct_on) = match self.parse_distinct()? {
            DistinctClause::No => (false, Vec::new()),
            DistinctClause::All => (true, Vec::new()),
            DistinctClause::On(exprs) => (false, exprs),
        };
        let projection = self.parse_projection()?;

        // No FROM: a single-row `Values` / projection `unnest`. The quantifier is applied
        // **here**, in the leaf, so both routes out of it carry it — this function's own
        // caller for a single select, and `finalize_leaf` for a set-op operand
        // (`SELECT DISTINCT unnest(ARRAY[2,1,2]) UNION ALL SELECT 9` is three rows on
        // 18.6).
        //
        // `DISTINCT` used to be **discarded** here, on the reasoning that a dedup over one
        // projected row is redundant. That is true of the one-row `Values` this arm was
        // written for and false of `Plan::ProjectUnnest`, which row-expands:
        // `SELECT DISTINCT unnest(ARRAY[2,1,2,1]) AS q` answered `2 1 2 1` against 18.6's
        // two rows, and `INSERT INTO n8 SELECT DISTINCT unnest(ARRAY[1,1,2])` wrote
        // **three** rows against its two — an extra row, silent on both engines. The node
        // is built unconditionally rather than only over a row-expanding projection: the
        // elision is *sound* over one row, so a shape test would only be a second rule to
        // keep in step with `plan_select_no_from`'s.
        //
        // The one exclusion is a **top-level sequence projection**, and it is an exclusion
        // from **both** quantifier branches rather than from this one — the plain
        // `DISTINCT` here, and the `DISTINCT ON` inside `plan_no_from_distinct_on`, which
        // runs its keys' gates and then declines to wrap. The reason is the plan-shape one
        // `plan_no_from_order_limit` states: the executor dispatches
        // `SELECT nextval/currval/setval(…)` on `Plan::is_sequence_projection`, which
        // matches a bare `Plan::Values`, so wrapping one turns an accepted statement into
        // `0A000` (`SELECT DISTINCT nextval('s')` and `SELECT DISTINCT ON (1) nextval('s')`
        // both answer a row on 18.6). Such a projection is a single row, which is exactly
        // the case both elisions are sound for — unlike the tail's `LIMIT`/`OFFSET`, whose
        // identical decline is a recorded divergence.
        //
        // `DISTINCT ON` is otherwise applied whether or not the projection row-expands, so
        // its ordinal is validated (`DISTINCT ON (5)` past the select list fails loud, as
        // in Postgres) rather than silently dropped — and so its keys are *bound* against
        // the target list, which nothing else does for a no-FROM select: the projection may
        // produce zero rows (`unnest(ARRAY[]::int[])`), and then no eval-time resolver ever
        // sees them. `scope_is_complete` is what keeps that gate off a level an expression
        // subquery encloses, whose keys may still name the enclosing query's relations.
        if !self.ts.eat_kw(Keyword::From)? {
            let scope_is_complete = self.scope_is_complete();
            let plan = plan_select_no_from(projection, self.schema)?;
            let plan = if !distinct_on.is_empty() {
                plan_no_from_distinct_on(plan, distinct_on, self.schema, scope_is_complete)?
            } else if distinct && !plan.is_sequence_projection() {
                Plan::Distinct {
                    input: Box::new(plan),
                }
            } else {
                plan
            };
            return Ok(SelectLeaf::NoFrom(Box::new(plan)));
        }

        let (from, tables) = self.parse_from()?;
        let selection = if self.ts.eat_kw(Keyword::Where)? {
            Some(self.parse_expr_bp(0)?)
        } else {
            None
        };
        let group_by = if self.ts.eat_kw(Keyword::Group)? {
            self.ts.expect_kw(Keyword::By)?;
            self.parse_expr_list()?
        } else {
            Vec::new()
        };
        let having = if self.ts.eat_kw(Keyword::Having)? {
            Some(self.parse_expr_bp(0)?)
        } else {
            None
        };

        Ok(SelectLeaf::FromParts(Box::new(SelectParts {
            from,
            tables,
            projection,
            distinct,
            distinct_on,
            selection,
            group_by,
            having,
            order_by: Vec::new(),
            limit: None,
            scope_is_complete: self.scope_is_complete(),
        })))
    }

    /// Lower a parsed leaf to a [`Plan`] for use as a set-op operand (its own ORDER
    /// BY/LIMIT are empty — those belong to the whole chain).
    ///
    /// The `NoFrom` arm runs the same correlated pass its [`Self::parse_select_inner`]
    /// twin runs over the identical leaf, and for the same reason: a no-FROM select
    /// never reaches [`lower_select`], so nothing else re-reads the sub-plans its
    /// projection embeds — and a level inside such a leaf keeps the *open* scope its
    /// own lowering gave it, which no enclosing level closes wherever the set op's own
    /// scope is already whole (top level and a CTE body, both measured). Without the
    /// call, `WITH c AS (SELECT 1 AS z UNION ALL SELECT (SELECT DISTINCT ON (bogusdon)
    /// 1 AS q)) DELETE FROM n8` destroyed every row against 18.6's `42703`, and the
    /// class needs no key at all — a plain correlated `zzz.k` in the leaf rode it too,
    /// against `42P01`.
    ///
    /// **Name the routes rather than count them.** Two callers reach this function —
    /// [`Self::parse_select_inner`] for a chain's leading leaf, [`Self::parse_setop_operand`]
    /// for every later one — but `parse_setop_operand` is itself called from
    /// [`Self::parse_recursive_cte_body`] as well, so a `WITH RECURSIVE`'s no-FROM seed
    /// and recursive term run this pass too. Measured benign and in agreement with 18.6:
    /// an ordinary `WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM r WHERE
    /// n < 3)` still answers `1, 2, 3`, and a correlated `zzz.k` inside a **sub-plan** of
    /// the seed or the term (`SELECT (SELECT k FROM et WHERE zzz.k = 1) UNION ALL …`) is
    /// `42P01` on both engines. The sub-plan spelling is the one this pass reaches — a
    /// bare-projection `SELECT zzz.k UNION ALL …` is not (the later paragraph's rule),
    /// and its errors arrive from other walks with other codes (`42804`/`42883`/`42703`
    /// by spelling — the UNION type unifier and the eval-time resolver — against 18.6's
    /// `42P01`).
    ///
    /// `scope_is_complete()` is what keeps the pass from *inventing* a refusal: a leaf
    /// an expression subquery encloses may legitimately name the enclosing query's
    /// relations, and there the enclosing level's own re-walk is the reader. Measured
    /// under a `true` in its place: `SELECT a FROM ct WHERE a IN (SELECT 1 AS z UNION
    /// ALL SELECT (SELECT k FROM et WHERE et.k = ct.a))` becomes `42P01` naming `ct`
    /// where both engines answer a row. The reference has to sit in a **sub-plan** of
    /// the leaf for that: a bare `SELECT ct.a` in the leaf's own projection is not
    /// reached by this pass and answers either way, so it is not the discriminator.
    ///
    /// # Errors
    ///
    /// Propagates [`lower_select`], and the correlated pass's binding verdicts
    /// (`42P01` / `42703` / `42702`) for a no-FROM leaf whose chain is whole.
    fn finalize_leaf(&self, leaf: SelectLeaf) -> Result<Plan> {
        match leaf {
            SelectLeaf::NoFrom(plan) => {
                let mut plan = *plan;
                resolve_correlated_subplans(&mut plan, self.schema, self.scope_is_complete())?;
                Ok(plan)
            }
            SelectLeaf::FromParts(parts) => lower_select(*parts, self.schema),
        }
    }

    /// Whether the cursor is at a set-operator keyword.
    pub(super) fn at_set_op(&self) -> bool {
        self.ts.at_kw(Keyword::Union)
            || self.ts.at_kw(Keyword::Intersect)
            || self.ts.at_kw(Keyword::Except)
    }

    /// Parse a set-op chain starting from an already-parsed leftmost operand `first`,
    /// at Postgres precedence: `INTERSECT` binds tighter than `UNION`/`EXCEPT`, and
    /// `UNION`/`EXCEPT` are equal precedence and left-associative.
    ///
    /// The `INTERSECT`-level sub-chain around `first` is folded first, then the
    /// lower-precedence `UNION`/`EXCEPT` operators fold left-to-right, each taking a
    /// fresh `INTERSECT`-level sub-chain as its right operand.
    fn parse_set_op_chain(&mut self, first: Plan) -> Result<Plan> {
        let mut left = self.parse_intersect_rest(first)?;
        let mut depth = SetOpDepth::default();
        loop {
            let op = if self.ts.at_kw(Keyword::Union) {
                SetOpKind::Union
            } else if self.ts.at_kw(Keyword::Except) {
                SetOpKind::Except
            } else {
                break;
            };
            self.ts.bump()?; // UNION | EXCEPT
            let all = self.parse_set_quantifier()?;
            let right_first = self.parse_setop_operand()?;
            let right = self.parse_intersect_rest(right_first)?;
            let right_depth = crate::depth::plan_depth(&right);
            left = self.build_set_op(op, all, left, right)?;
            depth.extend(&left, right_depth, self.depth)?;
        }
        Ok(left)
    }

    /// Fold a run of `INTERSECT [ALL]` operators (left-associative), starting from an
    /// already-parsed left operand.
    fn parse_intersect_rest(&mut self, first: Plan) -> Result<Plan> {
        let mut left = first;
        let mut depth = SetOpDepth::default();
        while self.ts.at_kw(Keyword::Intersect) {
            self.ts.bump()?; // INTERSECT
            let all = self.parse_set_quantifier()?;
            let right = self.parse_setop_operand()?;
            let right_depth = crate::depth::plan_depth(&right);
            left = self.build_set_op(SetOpKind::Intersect, all, left, right)?;
            depth.extend(&left, right_depth, self.depth)?;
        }
        Ok(left)
    }

    /// Parse a set-op operand: a parenthesized `( <select> )` (with its own optional
    /// ORDER BY/LIMIT), or a bare `SELECT` leaf.
    fn parse_setop_operand(&mut self) -> Result<Plan> {
        if self.ts.at(&TokenKind::LParen) {
            self.ts.expect(&TokenKind::LParen)?;
            let plan = self.parse_select()?;
            self.ts.expect(&TokenKind::RParen)?;
            return Ok(plan);
        }
        let leaf = self.parse_select_leaf()?;
        self.finalize_leaf(leaf)
    }

    /// Consume a set-op quantifier: `ALL` (keep duplicates) → `true`; `DISTINCT`
    /// (the default) or nothing → `false`.
    fn parse_set_quantifier(&mut self) -> Result<bool> {
        if self.ts.eat_kw(Keyword::All)? {
            Ok(true)
        } else {
            self.ts.eat_kw(Keyword::Distinct)?; // optional, default — a no-op
            Ok(false)
        }
    }

    /// Parse the FROM clause into `(plan, tables)` where `tables` lists each
    /// `(name, alias)` in order. Supports a base/catalog table, a derived
    /// `(SELECT …)`, a FROM-clause `unnest(…)`, a CTE reference, comma-joins
    /// (`FROM a, b, …`, an implicit CROSS JOIN), and explicit `JOIN`s (`ON` /
    /// `USING` / `CROSS`).
    ///
    /// `JOIN` binds **tighter** than comma, matching Postgres: the FROM clause is
    /// a comma-separated list of `table_ref`s, each a table factor plus its own
    /// left-deep `JOIN` chain scoped to itself (see [`Parser::parse_table_ref`]).
    /// The comma items combine into a top-level CROSS JOIN (`ON true`), so an
    /// inner JOIN's `ON`/`USING` can see only its own chain — `FROM c, a JOIN b ON
    /// c.num = b.num` fails loud (`42P01`) because `c` is a sibling comma item
    /// outside the `a JOIN b` scope, not the left of that join.
    fn parse_from(&mut self) -> Result<(Plan, Vec<FromTable>)> {
        let (mut plan, mut tables) = self.parse_table_ref()?;
        let mut depth = None;
        while self.ts.at(&TokenKind::Comma) {
            // `FROM a, b` — an implicit CROSS JOIN (Cartesian; a later WHERE
            // filters). Each comma operand is an independent `table_ref`, so its
            // own JOIN chain's scope stays isolated from the sibling operands.
            self.ts.bump()?;
            let (right_plan, right_tables) = self.parse_table_ref()?;
            depth = Some(join_depth(depth, &plan, &right_plan, None, self.depth)?);
            plan = Plan::Join {
                left: Box::new(plan),
                right: Box::new(right_plan),
                join_type: JoinType::Inner,
                on: Expr::Literal(Literal::Bool(true)),
                using: Vec::new(),
            };
            tables.extend(right_tables);
        }
        Ok((plan, tables))
    }

    /// Parse one `table_ref` — a table factor plus its own left-deep `JOIN` chain,
    /// scoped to itself (its `ON`/`USING` can't reach a sibling comma item, since
    /// `JOIN` binds tighter than comma). Returns the chain's plan and the
    /// `(name, alias)` of each relation it introduces, in order.
    fn parse_table_ref(&mut self) -> Result<(Plan, Vec<FromTable>)> {
        let (mut plan, first) = self.parse_table_factor()?;
        let mut tables = vec![first];
        let mut depth = None;
        while self.at_join_start() {
            let (join_type, right_plan, right_table, constraint) = self.parse_join()?;
            let (on, using) = self.resolve_join_constraint(constraint, &plan, &right_plan)?;
            depth = Some(join_depth(
                depth,
                &plan,
                &right_plan,
                Some(&on),
                self.depth,
            )?);
            plan = Plan::Join {
                left: Box::new(plan),
                right: Box::new(right_plan),
                join_type,
                on,
                using,
            };
            tables.push(right_table);
        }
        Ok((plan, tables))
    }

    /// Parse one `[INNER | LEFT [OUTER] | RIGHT [OUTER] | FULL [OUTER]] JOIN factor
    /// {ON condition | USING (cols)}` or a `CROSS JOIN factor`, returning
    /// `(join_type, right_plan, right_table, constraint)`. `NATURAL` joins and a
    /// bare-`JOIN`-without-`ON`/`USING` fail loud (`CROSS` is the only constraintless
    /// form).
    fn parse_join(&mut self) -> Result<(JoinType, Plan, FromTable, JoinConstraint)> {
        // `CROSS JOIN` — a Cartesian product; no `ON`/`USING` may follow.
        if self.ts.eat_kw(Keyword::Cross)? {
            self.ts.expect_kw(Keyword::Join)?;
            let (right_plan, right_table) = self.parse_table_factor()?;
            return Ok((
                JoinType::Inner,
                right_plan,
                right_table,
                JoinConstraint::Cross,
            ));
        }
        // `NATURAL JOIN` — out of scope; fail loud rather than silently degrade.
        if self.ts.at_kw(Keyword::Natural) {
            return Err(Error::NotImplemented("NATURAL JOIN not supported".into()));
        }

        let join_type = if self.ts.eat_kw(Keyword::Inner)? {
            self.ts.expect_kw(Keyword::Join)?;
            JoinType::Inner
        } else if self.ts.eat_kw(Keyword::Left)? {
            self.ts.eat_kw(Keyword::Outer)?;
            self.ts.expect_kw(Keyword::Join)?;
            JoinType::Left
        } else if self.ts.eat_kw(Keyword::Right)? {
            self.ts.eat_kw(Keyword::Outer)?;
            self.ts.expect_kw(Keyword::Join)?;
            JoinType::Right
        } else if self.ts.eat_kw(Keyword::Full)? {
            self.ts.eat_kw(Keyword::Outer)?;
            self.ts.expect_kw(Keyword::Join)?;
            JoinType::Full
        } else {
            // A bare `JOIN` is an inner join.
            self.ts.expect_kw(Keyword::Join)?;
            JoinType::Inner
        };

        let (right_plan, right_table) = self.parse_table_factor()?;

        let constraint = if self.ts.eat_kw(Keyword::On)? {
            JoinConstraint::On(self.parse_expr_bp(0)?)
        } else if self.ts.eat_kw(Keyword::Using)? {
            JoinConstraint::Using(self.parse_paren_name_list()?)
        } else {
            return Err(Error::NotImplemented(
                "JOIN without ON clause not supported".into(),
            ));
        };
        Ok((join_type, right_plan, right_table, constraint))
    }

    /// Turn a parsed [`JoinConstraint`] into the `(on, using)` pair a `Plan::Join`
    /// carries. `ON` keeps its predicate (no USING columns); `CROSS` is `ON true`;
    /// `USING (cols)` validates each column against both sides and drives the row
    /// match through `using` (its `on` is the trivial `true` — the executor never
    /// evaluates it for a USING join).
    ///
    /// # Errors
    ///
    /// A USING column absent from a side fails loud (`42703`); one that matches two
    /// or more of a side's relations is ambiguous (`42702`) — Postgres's rule that a
    /// USING name denote exactly one column per side.
    fn resolve_join_constraint(
        &self,
        constraint: JoinConstraint,
        left: &Plan,
        right: &Plan,
    ) -> Result<(Expr, Vec<String>)> {
        match constraint {
            JoinConstraint::On(expr) => Ok((expr, Vec::new())),
            JoinConstraint::Cross => Ok((Expr::Literal(Literal::Bool(true)), Vec::new())),
            JoinConstraint::Using(cols) => {
                if cols.is_empty() {
                    return Err(Error::syntax("JOIN/USING requires at least one column"));
                }
                let left_cols = left.output_columns(self.schema);
                let right_cols = right.output_columns(self.schema);
                for c in &cols {
                    check_using_side(&left_cols, c)?;
                    check_using_side(&right_cols, c)?;
                }
                Ok((Expr::Literal(Literal::Bool(true)), cols))
            }
        }
    }

    /// Parse an `ORDER BY a [ASC|DESC] [NULLS FIRST|LAST], …` clause if present,
    /// returning the [`OrderKey`] list (empty when absent).
    fn parse_order_by_clause(&mut self) -> Result<Vec<OrderKey>> {
        if !self.ts.eat_kw(Keyword::Order)? {
            return Ok(Vec::new());
        }
        self.ts.expect_kw(Keyword::By)?;
        self.parse_order_by()
    }

    /// Parse a comma-separated `expr [ASC|DESC] [NULLS FIRST|LAST]` list (the body
    /// of an `ORDER BY`, shared with a window's `ORDER BY`).
    ///
    /// The `NULLS FIRST|LAST` placement is **kept** (`Some(true)`/`Some(false)`),
    /// not discarded — an absent clause is `None`, resolved to Postgres's direction
    /// default (`ASC` → NULLS LAST, `DESC` → NULLS FIRST) at compare time.
    pub(super) fn parse_order_by(&mut self) -> Result<Vec<OrderKey>> {
        self.comma_list(|p| {
            let expr = p.parse_expr_bp(0)?;
            let asc = if p.ts.eat_kw(Keyword::Asc)? {
                true
            } else if p.ts.eat_kw(Keyword::Desc)? {
                false
            } else {
                true // default ASC
            };
            let nulls_first = if p.ts.eat_kw(Keyword::Nulls)? {
                if p.ts.eat_kw(Keyword::First)? {
                    Some(true)
                } else {
                    p.ts.expect_kw(Keyword::Last)?;
                    Some(false)
                }
            } else {
                None
            };
            Ok(OrderKey {
                expr,
                asc,
                nulls_first,
            })
        })
    }

    /// Parse a comma-separated expression list (GROUP BY / window PARTITION BY).
    pub(super) fn parse_expr_list(&mut self) -> Result<Vec<Expr>> {
        self.comma_list(|p| p.parse_expr_bp(0))
    }

    /// The clauses over a whole query — `ORDER BY`, then `LIMIT` / `OFFSET`, then a
    /// locking clause — and what they carry.
    #[allow(clippy::type_complexity)]
    fn parse_query_clauses(
        &mut self,
    ) -> Result<(
        Vec<OrderKey>,
        Option<(Option<LimitCount>, Option<LimitCount>)>,
        QueryClauses,
    )> {
        let order_by = self.parse_order_by_clause()?;
        let mut clauses = QueryClauses {
            order_by: !order_by.is_empty(),
            ..QueryClauses::default()
        };
        let limit = self.parse_limit_offset(&mut clauses)?;
        self.parse_for_locking()?;
        Ok((order_by, limit, clauses))
    }

    /// Parse a `LIMIT`/`OFFSET` clause (either order), returning `Some((limit,
    /// offset))` iff one was present (each bound may still be `None`, e.g.
    /// `LIMIT ALL`), and noting each in `clauses`.
    fn parse_limit_offset(
        &mut self,
        clauses: &mut QueryClauses,
    ) -> Result<Option<(Option<LimitCount>, Option<LimitCount>)>> {
        let mut present = false;
        let mut limit = None;
        let mut offset = None;
        loop {
            if self.ts.eat_kw(Keyword::Limit)? {
                present = true;
                clauses.limit = true;
                limit = self.parse_limit_count(LimitClause::Limit)?;
            } else if self.ts.eat_kw(Keyword::Offset)? {
                present = true;
                clauses.offset = true;
                offset = self.parse_limit_count(LimitClause::Offset)?;
                // optional `ROW`/`ROWS` noise word
                let _ = self.eat_ident("rows")? || self.eat_ident("row")?;
            } else {
                break;
            }
        }
        Ok(present.then_some((limit, offset)))
    }

    /// Parse one `LIMIT`/`OFFSET` count.
    ///
    /// A non-negative integer literal → `Const` (a fractional literal rounds to
    /// nearest, matching Postgres's numeric→int8 cast); a `$N` → `Param`;
    /// `ALL`/`NULL` → `None` (unbounded — Postgres treats both as no limit). A
    /// **negative** count fails loud (`2201W`/`2201X`) and an **overflowing** one
    /// fails loud (`22003`), matching Postgres — previously both silently became
    /// "unbounded" (returning every row). A non-constant expression, which pglet
    /// can't evaluate at plan time, also fails loud rather than silently dropping
    /// the bound.
    fn parse_limit_count(&mut self, clause: LimitClause) -> Result<Option<LimitCount>> {
        // A leading `-` makes a negative integer constant (`LIMIT -1`), which
        // Postgres rejects per-clause (`recompute_limits`, at execute time); catch it
        // before the general dispatch. This peek reads the raw token pair rather than
        // the folded constant `parser::expr`'s `negate` produces, so this clause does
        // not share that fold: `LIMIT -0` / `-0.4` / `- -1` diverge from 18.6, loudly.
        // `docs/compatibility.md`'s SELECT row records them, beside the parenthesized spellings the
        // number-or-param-only grammar below refuses independently of the sign.
        if matches!(self.ts.peek_kind(), TokenKind::Minus)
            && matches!(self.ts.peek2_kind(), TokenKind::Number)
        {
            return Err(clause.negative_error());
        }
        match self.ts.peek_kind() {
            TokenKind::Number => {
                let tok = self.ts.advance()?;
                Ok(Some(LimitCount::Const(parse_row_count(
                    self.ts.text(&tok),
                )?)))
            }
            TokenKind::Param(n) => {
                let index = *n as usize;
                self.ts.bump()?;
                Ok(Some(LimitCount::Param(index)))
            }
            // `ALL`/`NULL` → no bound (Postgres treats both as unbounded).
            TokenKind::Keyword(Keyword::All | Keyword::Null) => {
                self.ts.bump()?;
                Ok(None)
            }
            _ => Err(self.ts.error(&format!(
                "{} must be a constant, parameter, ALL, or NULL",
                clause.name()
            ))),
        }
    }

    /// Consume the optional distinct quantifier after `SELECT`: nothing
    /// ([`DistinctClause::No`]), `DISTINCT` ([`DistinctClause::All`] — dedup every
    /// projected row), or `DISTINCT ON (e1[, e2…])` ([`DistinctClause::On`]). The
    /// `ON` list is a parenthesized expression list (a Postgres extension), resolved
    /// against the output columns and merged with ORDER BY at lowering time.
    fn parse_distinct(&mut self) -> Result<DistinctClause> {
        if self.ts.eat_kw(Keyword::Distinct)? {
            if self.ts.eat_kw(Keyword::On)? {
                self.ts.expect(&TokenKind::LParen)?;
                let on_exprs = self.parse_expr_list()?;
                self.ts.expect(&TokenKind::RParen)?;
                return Ok(DistinctClause::On(on_exprs));
            }
            return Ok(DistinctClause::All);
        }
        Ok(DistinctClause::No)
    }

    /// Consume one or more trailing row-level locking clauses, each
    /// `FOR {UPDATE | NO KEY UPDATE | SHARE | KEY SHARE} [OF table[, …]]`, or the
    /// single `FOR READ ONLY` — and **refuse** a `NOWAIT` / `SKIP LOCKED` wait policy.
    ///
    /// Honoring a **valid** clause as a no-op (rather than failing loud) lets the spine's
    /// `SELECT … FOR UPDATE` find-and-claim queries run unchanged (`FOR UPDATE` bare and
    /// `FOR UPDATE OF <alias>` are both issued). A locking read inside a block takes the
    /// writer ([`Plan::takes_writer`]), which keeps two blocks' claims apart only by
    /// ordering the whole blocks, and that is the whole reason the wait policy is the one
    /// part of the clause that is not a no-op. Honored as no-ops, two connections inside
    /// `BEGIN` each running `… ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED` would not be
    /// handed rows 1 and 2, as 18.6 hands them: on the native server the second would
    /// queue behind the first block, and a `NOWAIT` there would wait for the first block —
    /// `55P03` only once `lock_timeout` passes — where 18.6 answers `55P03` at once; the
    /// embedded `Database` refuses the second `BEGIN` `55P03` while the first block is
    /// open. `SKIP LOCKED` and `NOWAIT` therefore fail loud (`0A000`) on every host;
    /// nothing in the workspace sends either.
    ///
    /// **What is parsed for real is the syntax, and only the syntax.** An unrecognized
    /// lock strength or trailing garbage (`FOR XYZ`, `FOR UPDATE foobar`) fails loud,
    /// matching Postgres, instead of being swallowed to the statement boundary — but
    /// every *semantic* refusal Postgres attaches to the clause is skipped, so this
    /// function accepts each of them silently. Postgres's `CheckSelectLocking` raises
    /// `0A000` for a locking clause over a set operation, `DISTINCT`/`DISTINCT ON`,
    /// `GROUP BY`, `HAVING`, an aggregate, a window function, or a set-returning function
    /// in the target list. Every one of those grounds has a spelling measured accepted
    /// here: the first six in their FROM-bearing form, and the seventh in the no-FROM
    /// projection form (`SELECT unnest(ARRAY[1,2]) AS q FOR UPDATE`), its FROM-bearing
    /// form being `0A000` here for an unrelated reason — a set-returning `unnest` is
    /// supported only in a projection. `transformLockingClause`'s `42P01` for an `OF`
    /// naming a relation the FROM clause lacks is accepted as well: the `OF` list's names
    /// are parsed and discarded rather than resolved. Those nine stay accepted — they
    /// are refusals of a clause pglet *does* honour as a no-op, so refusing them would
    /// reject spine SQL that runs correctly here; `EDGES.md` row 30 is where they live.
    /// The wait policy is the one member with a **behavioural** claim behind it, which
    /// is why it alone moved.
    pub(super) fn parse_for_locking(&mut self) -> Result<()> {
        // `gram.y:14345` `for_locking_clause: for_locking_items | FOR READ ONLY` — the
        // two are **alternatives**, so `FOR READ ONLY` is the whole clause and cannot
        // stand beside a locking item in either order. This flag is the whole of that
        // rule: the arm is offered on the first `FOR` only. Placed inside the loop with
        // no flag it was admitted at every iteration, and
        // `SELECT * FROM t FOR UPDATE FOR READ ONLY` answered rows against 18.6's
        // `42601 syntax error at or near "READ"` — a silent over-acceptance an
        // is-the-arm-present mutation cannot see, since the arm was present and merely
        // in the wrong position.
        let mut at_clause_start = true;
        while self.ts.eat_kw(Keyword::For)? {
            // `FOR READ ONLY` — the SQL92 spelling, a genuine no-op on 18.6 (it asserts
            // the cursor is not updatable) with no strength, no `OF` list and no wait
            // policy. Returning here is what makes the *other* order
            // (`FOR READ ONLY FOR UPDATE`) the statement's own `42601`, stopping at the
            // second `FOR` as 18.6 does (`syntax error at or near "FOR"`, and the same
            // for `FOR READ ONLY FOR READ ONLY`). After an item, `read` is left to
            // `parse_lock_strength`, whose `42601` lands at the same point in the parse
            // — not "on the same token": pglet sends no `P` and its message names none
            // (`EDGES.md` row 76), so which token it blames is not observable.
            if at_clause_start && self.eat_ident("read")? {
                self.expect_ident("only")?;
                return Ok(());
            }
            at_clause_start = false;
            self.parse_lock_strength()?;
            self.saw_locking_clause = true;
            // Optional `OF table[, …]` — the locked relations (a ColId list). The
            // names are validated then discarded (the `OF` list is a no-op for pglet).
            if self.eat_ident("of")? {
                self.comma_list(|p| p.expect_name("a table name in FOR ... OF"))?;
            }
            // Optional wait policy: `NOWAIT` or `SKIP LOCKED`. Both are **refused**
            // (`0A000`) — each names a behaviour that needs a **row lock** to mean
            // anything, and pglet takes none; the absence is invisible to one
            // connection, so accepting them as no-ops made a worker queue silently hand
            // every consumer the same row.
            // The refusal is *recorded*, not raised: the rest of the clause is consumed
            // first, so `FOR UPDATE SKIP` still reports `expected locked` and
            // `… SKIP LOCKED OF lt` still reports its boundary error, and
            // `Parser::lock_wait_policy_refusal` then fires it at the statement's end.
            if self.eat_ident("nowait")? {
                self.refuse_lock_wait_policy("NOWAIT");
            } else if self.eat_ident("skip")? {
                self.expect_ident("locked")?;
                self.saw_skip_locked = true;
                self.refuse_lock_wait_policy("SKIP LOCKED");
            }
        }
        Ok(())
    }

    /// Record a `SKIP LOCKED` / `NOWAIT` refusal, to be raised once the statement has
    /// finished parsing (see [`Parser::lock_wait_policy_refusal`]). One message per
    /// spelling: a single "wait policy" wording would not tell the author which of the
    /// two tokens to remove. The first refusal in a statement wins.
    fn refuse_lock_wait_policy(&mut self, spelling: &str) {
        self.lock_wait_policy_refusal
            .get_or_insert_with(|| Error::NotImplemented(format!("{spelling} not supported")));
    }

    /// Parse the lock strength after `FOR`: `UPDATE`, `NO KEY UPDATE`, `SHARE`, or
    /// `KEY SHARE` (`SHARE` is a contextual identifier, not a reserved keyword).
    fn parse_lock_strength(&mut self) -> Result<()> {
        if self.ts.eat_kw(Keyword::Update)? {
            Ok(())
        } else if self.ts.eat_kw(Keyword::No)? {
            self.ts.expect_kw(Keyword::Key)?;
            self.ts.expect_kw(Keyword::Update)?;
            Ok(())
        } else if self.ts.eat_kw(Keyword::Key)? {
            self.expect_ident("share")
        } else if self.eat_ident("share")? {
            Ok(())
        } else {
            Err(self
                .ts
                .expected("UPDATE, NO KEY UPDATE, SHARE, or KEY SHARE after FOR"))
        }
    }

    /// Parse the projection list (`*`, `expr`, `expr [AS] alias`). Shared with the
    /// `RETURNING` clause (which lowers identically to a projection).
    pub(super) fn parse_projection(&mut self) -> Result<Vec<Expr>> {
        self.comma_list(|p| {
            if p.ts.at(&TokenKind::Star) {
                p.ts.bump()?;
                Ok(Expr::Column {
                    table: None,
                    name: "*".to_string(),
                })
            } else {
                let expr = p.parse_expr_bp(0)?;
                p.parse_optional_alias(expr)
            }
        })
    }

    /// Wrap `expr` in an `Expr::Alias` if an `[AS] alias` follows.
    ///
    /// A column-list label is a Postgres `ColLabel`, so an explicit `AS` accepts
    /// **any** keyword (`… AS from`, `… AS left`); a bare (no-`AS`) label must be a
    /// real identifier, since a bare keyword would collide with clause markers
    /// (`SELECT a from t`) — that boundary stays fail-loud.
    fn parse_optional_alias(&mut self, expr: Expr) -> Result<Expr> {
        let aliased = self.ts.eat_kw(Keyword::As)?
            || matches!(
                self.ts.peek_kind(),
                TokenKind::Identifier | TokenKind::QuotedIdentifier
            );
        if aliased {
            let alias = self.expect_col_label()?;
            return Ok(Expr::Alias {
                expr: Box::new(expr),
                alias,
            });
        }
        Ok(expr)
    }

    /// Consume a Postgres `ColLabel` — a bare / quoted identifier or **any**
    /// keyword, reserved ones included — for an `AS` label. Wider than
    /// [`Self::expect_alias_name`], because a column label admits every keyword whereas a
    /// table alias is a `ColId`.
    ///
    /// The label is the **output column's name**, so it folds like any other
    /// unquoted identifier: `SELECT 1 AS Bar` labels the column `bar`, and only
    /// `AS "Baz"` keeps its case. That is what a client keying rows by column name
    /// reads back.
    fn expect_col_label(&mut self) -> Result<String> {
        let tok = self.ts.advance()?;
        match tok.kind {
            TokenKind::Identifier | TokenKind::QuotedIdentifier | TokenKind::Keyword(_) => {
                Ok(self.ts.name(&tok))
            }
            _ => Err(self.ts.expected("an alias")),
        }
    }

    /// Consume a **table** alias name — a Postgres `ColId` (bare / quoted
    /// identifier or a non-reserved keyword), narrower than a column label: a
    /// table alias rejects `TypeFuncName` words (`FROM t AS left` is a syntax
    /// error in Postgres too). Shares the ColId consumer [`Parser::expect_name`]
    /// (and so the [`super::is_colid_name`] predicate) with the other ColId
    /// positions; only the error label differs.
    fn expect_alias_name(&mut self) -> Result<String> {
        self.expect_name("an alias")
    }

    /// Parse a FROM table factor into `(Plan, (name, alias))`. Dispatches a derived
    /// `(SELECT …)` table, a FROM-clause `unnest(…)` or `generate_series(…)` table
    /// function, a CTE reference, a virtual catalog table (`Plan::VirtualScan`), or a
    /// regular table (`Plan::Scan`), in that precedence order.
    pub(super) fn parse_table_factor(&mut self) -> Result<(Plan, FromTable)> {
        // Derived table: `( SELECT … ) [AS] alias`.
        if self.ts.at(&TokenKind::LParen) {
            return self.parse_derived_table();
        }
        // FROM-clause `unnest(arr, …) [AS] alias[(cols)]`.
        if self.at_ident("unnest") && matches!(self.ts.peek2_kind(), TokenKind::LParen) {
            return self.parse_unnest_table();
        }
        // FROM-clause `generate_series(start, stop[, step]) [AS] alias[(col)]`.
        if self.at_ident("generate_series") && matches!(self.ts.peek2_kind(), TokenKind::LParen) {
            return self.parse_generate_series_table();
        }

        // A table name is a `ColId` (bare/quoted identifier or non-reserved
        // keyword), consistent with the INSERT/UPDATE/DELETE targets that resolve
        // through `expect_name` — routed through the same `is_colid_name` predicate
        // so it can't drift stricter (the regression the FROM-table-name fix closed:
        // pglet could `CREATE TABLE update` but not `SELECT … FROM update`).
        let first = self.ts.advance()?;
        let mut parts = if super::is_colid_name(&first.kind) {
            vec![self.ts.name(&first)]
        } else {
            return Err(self.ts.expected("a table name"));
        };
        while self.ts.at(&TokenKind::Dot) {
            self.ts.bump()?;
            let part = self.ts.advance()?;
            if super::is_colid_name(&part.kind) {
                parts.push(self.ts.name(&part));
            } else {
                return Err(self.ts.expected("a table name after '.'"));
            }
        }
        let qualified = parts.join(".");
        let alias = self.parse_table_alias()?;

        // An **unqualified** name may be a CTE, which shadows a table or catalog
        // view. A read-only CTE inlines as a `SubqueryScan` over its subplan; a
        // data-modifying CTE and a recursive one (its `Plan::CteScan` entry, registered
        // by `parse_recursive_cte_body` before its recursive term is parsed) resolve to a
        // `CteScan` that reads the rows a wrapping `Plan::With` materializes (never
        // re-executing at the reference site). A schema-qualified name can't name a CTE.
        if parts.len() == 1
            && let Some(cte) = self.ctes.get(&parts[0])
        {
            // A `CteScan` in the map is a recursive CTE — the placeholder of the one
            // being defined, or an earlier one already resolved — and a data-modifying
            // CTE reads its materialized rows too. Only the **name** being defined makes
            // this a self-reference.
            let is_self_ref = self.defining_cte.as_deref() == Some(parts[0].as_str());
            let use_cte_scan =
                matches!(cte.plan, Plan::CteScan { .. }) || cte.plan.mutates_storage();
            let table_name = parts[0].clone();
            let effective_alias = alias.unwrap_or(table_name);
            let plan = if use_cte_scan {
                // The `CteScan`'s `columns` are the plan-side answer only — the
                // executor reads the materialized relation's — so the alias list is
                // applied **here and again** at `Plan::With`'s materializer, both
                // through `alias_relation_columns`.
                Plan::CteScan {
                    name: parts[0].clone(),
                    alias: effective_alias.clone(),
                    columns: crate::planner::alias_relation_columns(
                        cte.plan.output_columns(self.schema),
                        &cte.column_aliases,
                    ),
                }
            } else {
                // A read-only CTE inlines, so the list rides on the `SubqueryScan`
                // and every reader of that node applies it — including the executor,
                // which is the reader a rename baked into the subplan would miss. The
                // inlined body reads its tables here, where the reference runs.
                if let Some(tables) = self.cte_tables.get(&parts[0]) {
                    self.touched_tables.extend(tables.iter().cloned());
                }
                Plan::SubqueryScan {
                    subquery: Box::new(crate::depth::clone_plan(&cte.plan)?),
                    alias: effective_alias.clone(),
                    column_aliases: cte.column_aliases.clone(),
                    cte_rank: Some(cte.rank),
                }
            };
            // Borrow of `self.ctes` (via `cte_plan`) ends above; count the
            // self-reference now that a mutable borrow is free.
            if is_self_ref {
                self.recursive_ref_count += 1;
            }
            return Ok((plan, (effective_alias.clone(), Some(effective_alias))));
        }
        // Under `WITH RECURSIVE` every name in the list is in scope everywhere, so a
        // list name not resolved yet must not fall through to a same-named table.
        if parts.len() == 1 && self.pending_ctes.contains(&parts[0]) {
            return Err(Error::NotImplemented(format!(
                "a reference to WITH query \"{}\" in a position the WITH RECURSIVE dependency scan does not read",
                parts[0]
            )));
        }

        // A catalog (virtual) table is matched on its full qualified name, so it
        // bypasses the implicit-schema reduction (`information_schema.columns`,
        // `pg_catalog.pg_class`, a bare `pg_tables`).
        if let Some(catalog_table) = CatalogTable::from_name(&qualified) {
            let effective = parts.last().cloned().unwrap_or_default();
            return Ok((
                Plan::VirtualScan {
                    catalog_table,
                    alias: alias.clone(),
                },
                (effective, alias),
            ));
        }

        // Otherwise a base table under the implicit `public` schema (a `public.`
        // prefix strips; any other qualifier or 3+ parts fails loud).
        let table_name = self.reduce_grammar_name(&parts)?;
        self.touched_tables.push(table_name.clone());
        Ok((
            Plan::Scan {
                table: table_name.clone(),
                alias: alias.clone(),
            },
            (table_name, alias),
        ))
    }

    /// Parse a derived table `( SELECT … ) [AS] alias` into a `Plan::SubqueryScan`,
    /// or a `( VALUES … ) [AS] alias[(cols)]` list-as-a-derived-table (dispatched to
    /// [`Self::parse_from_values`]). An absent `SELECT`-derived alias defaults to
    /// `"subquery"`.
    fn parse_derived_table(&mut self) -> Result<(Plan, FromTable)> {
        self.ts.expect(&TokenKind::LParen)?;
        // `( VALUES … )` — a VALUES list as a derived table.
        if self.ts.at_kw(Keyword::Values) {
            return self.parse_from_values();
        }
        // A derived table is a FROM item: its namespace does not include the enclosing
        // query's relations, so the chain-completeness depth resets rather than deepens.
        let subplan = self.in_from_item(Self::parse_select)?;
        self.ts.expect(&TokenKind::RParen)?;
        let alias_name = self
            .parse_table_alias()?
            .unwrap_or_else(|| "subquery".to_string());
        Ok((
            Plan::SubqueryScan {
                column_aliases: Vec::new(),
                cte_rank: None,
                subquery: Box::new(subplan),
                alias: alias_name.clone(),
            },
            (alias_name.clone(), Some(alias_name)),
        ))
    }

    /// Parse a FROM-clause `( VALUES (…), … ) [AS] alias[(col, …)]` — a VALUES list
    /// used as a derived table — into a `Plan::SubqueryScan` wrapping the existing
    /// `Plan::Values`. The opening `(` has already been consumed.
    ///
    /// Because the wrapper reuses the plan-generic `SubqueryScan`/`Values` path,
    /// JOIN/WHERE/ORDER BY composition and `alias.*`/`$N` typing come for free. This
    /// is the first **multi-row** `Plan::Values` consumer, so the per-column type is
    /// unified across **all** rows here (the same `common_type_step` fold set
    /// operations use) — `Plan::Values::output_columns` only reads row 0. The pre-pass
    /// therefore, at each position: casts every cell whose own type differs from the
    /// unified type to it (so every row encodes at the column type), and wraps row 0's
    /// cell in an `Expr::Alias` carrying the output name — the provided alias, else
    /// Postgres's `column{i + 1}` default (1-indexed).
    fn parse_from_values(&mut self) -> Result<(Plan, FromTable)> {
        self.ts.expect_kw(Keyword::Values)?;
        let rows = self.parse_values_rows()?;
        self.ts.expect(&TokenKind::RParen)?;

        // Every VALUES row must be the same width (Postgres `42601`).
        let width = rows.first().map_or(0, Vec::len);
        if rows.iter().any(|row| row.len() != width) {
            return Err(Error::syntax("VALUES lists must all be the same length"));
        }

        // Resolve and gate each **cell** on its own, before the per-column fold below —
        // Postgres transforms every `VALUES` expression and only then calls
        // `select_common_type` over the columns. The fold says what a cell must
        // *become*; it says nothing about whether the cell is well-typed on its own, so
        // `VALUES (COALESCE(1, 'x'::text))` unified to `integer` and ran where Postgres
        // reports `42804`. A FROM-clause `VALUES` has no relations in scope, which makes
        // this the no-columns resolution — the same one the `INSERT … VALUES` list uses,
        // and the one that supplies the unknown-literal coercion `(1 = '1')` needs.
        //
        // Each cell is read for the type it is certainly of as it is resolved
        // (`resolve_entries_gated`); the fold below turns those into each column's
        // record.
        let no_columns = ColumnScope::default();
        let mut written: Vec<Vec<Option<TypeOid>>> = Vec::with_capacity(rows.len());
        let rows: Vec<Vec<Expr>> = rows
            .into_iter()
            .map(|row| {
                let (cells, types) = resolve_entries_gated(row, &no_columns, self.schema)?;
                written.push(types);
                Ok(cells)
            })
            .collect::<Result<Vec<_>>>()?;

        let alias_name = self.parse_table_alias()?;
        // An optional column-alias list `(a, b, …)` — only after an explicit alias,
        // mirroring `unnest(...) AS t(a, b)`.
        let alias_columns = if alias_name.is_some() && self.ts.at(&TokenKind::LParen) {
            self.parse_paren_name_list()?
        } else {
            Vec::new()
        };
        let effective = alias_name.unwrap_or_else(|| "values".to_string());

        // More aliases than columns → `42P10`; fewer just leaves the trailing columns
        // with their `column{i + 1}` defaults (not an error), matching Postgres.
        if alias_columns.len() > width {
            return Err(Error::TableColumnCountMismatch {
                table: effective,
                available: width,
                specified: alias_columns.len(),
            });
        }

        // Per-column unified type across every row, each cell coerced to it — the unknown
        // cells converted at plan time (`unify_values_columns`).
        let mut rows = rows;
        let certain = unify_values_columns(&mut rows, &written, width, self.schema)?;

        // Give row 0's cells their output names via `Expr::Alias`.
        let rows: Vec<Vec<Expr>> = rows
            .into_iter()
            .enumerate()
            .map(|(r, row)| {
                row.into_iter()
                    .enumerate()
                    .map(|(i, cell)| {
                        let mut expr = cell;
                        if r == 0 {
                            let name = alias_columns
                                .get(i)
                                .cloned()
                                .unwrap_or_else(|| format!("column{}", i + 1));
                            expr = Expr::Alias {
                                expr: Box::new(expr),
                                alias: name,
                            };
                        }
                        expr
                    })
                    .collect()
            })
            .collect();

        Ok((
            Plan::SubqueryScan {
                column_aliases: Vec::new(),
                cte_rank: None,
                subquery: Box::new(Plan::Values {
                    rows,
                    certain,
                    enclosing: EnclosingColumns::default(),
                }),
                alias: effective.clone(),
            },
            (effective.clone(), Some(effective)),
        ))
    }

    /// Parse a FROM-clause `unnest(arr, …) [AS] alias[(col, …)]` into a
    /// `Plan::FunctionScan`.
    ///
    /// Output column names follow Postgres's `chooseScalarFunctionAlias`
    /// (`parse_relation.c`): the alias column list when one is written, else — **only
    /// when the RTE holds a single function** — the relation alias itself, else the
    /// function name `unnest`. Postgres expands a multi-argument `unnest` in FROM into
    /// one function per argument, so `nfuncs` here is `arrays.len()`, and the
    /// single-function arm is what makes `FROM unnest(a) AS v` name its column `v` as
    /// well as its relation. Measured on 18.6: `SELECT v FROM unnest(ARRAY[1,2]) AS v`
    /// answers and `SELECT unnest FROM unnest(ARRAY[1,2]) AS v` is `42703`, while over
    /// **two** arrays the columns are both `unnest` (so a bare `unnest` there is
    /// `42702`) and the alias names the composite row instead.
    ///
    /// The column name is not cosmetic: it is what a **closed** scope counts claimants
    /// with. Named `unnest` where Postgres says `v`, the relation contributed no
    /// claimant for `v`, and `UPDATE amb_t SET … FROM unnest(ARRAY[1,2]) AS v
    /// RETURNING v` answered the target's `v` where 18.6 says `42702`.
    ///
    /// This and [`Parser::parse_generate_series_table`] are the function calls a FROM
    /// clause holds, so they are the places besides [`Parser::parse_function_call`] that
    /// **read** a set quantifier — a second route to the same token, which is why reading
    /// it nowhere left `FROM unnest(ALL ARRAY[1,2]) AS u(x)` at `42601` here against
    /// `1, 2` on 18.6 while the select-list spelling agreed.
    fn parse_unnest_table(&mut self) -> Result<(Plan, FromTable)> {
        self.ts.bump()?; // `unnest`
        self.ts.expect(&TokenKind::LParen)?;
        // The **set quantifier** reaches the grammar here as well as at
        // `parse_function_call`, and this route reads it for the same reason that one
        // does: the token is `func_application`'s, so a route that does not read it
        // answers `42601` to spellings 18.6 accepts. The three rules split by route —
        // the grammar bound is shared, and the verdict on a written quantifier is
        // `planner::refuse_from_unnest_quantifier`'s (`transformRangeFunction`'s
        // suppressed expansion), not `build_call`'s.
        //
        // The two routes agree on **where** each of the two sits, and that is part of
        // the contract rather than a coincidence of layout: the grammar bound is the
        // parser's own and runs before the closing paren, while the verdict runs after
        // it, because 18.6 scans the whole call before parse analysis judges it. Raised
        // above the `expect` here, the verdict answered `42809` to
        // `FROM unnest(DISTINCT ARRAY[1, 2] junk)`, which is `42601` on both engines
        // once the paren is asked for first. The bound keeps its position, which is
        // what makes the two routes identical rather than merely both-correct: moving
        // it below the `expect` too changes no SQLSTATE and reddens no fixture
        // (measured), because it fires only over an empty argument list, and the list
        // is empty only when the next token *is* `)`.
        let quantifier = self.parse_func_set_quantifier()?;
        let arrays = self.comma_list_until(&TokenKind::RParen, |p| p.parse_expr_bp(0))?;
        self.bound_func_set_quantifier(quantifier, arrays.is_empty())?;
        self.ts.expect(&TokenKind::RParen)?;
        crate::planner::refuse_from_unnest_quantifier(quantifier, &arrays)?;
        // An empty list resolves no `unnest` signature on 18.6 — `unnest(anyarray)` is the
        // narrowest — so it is that `42883`, not a relation of no columns. A quantifier
        // beside the empty list never arrives: the grammar bound refused it above.
        if arrays.is_empty() {
            return Err(Error::UndefinedFunction(
                "function unnest() does not exist".into(),
            ));
        }

        let alias_name = self.parse_table_alias()?;
        let alias_columns = if alias_name.is_some() && self.ts.at(&TokenKind::LParen) {
            self.parse_paren_name_list()?
        } else {
            Vec::new()
        };
        // An aggregate in a FROM function's argument is 18.6's `42803`, raised as
        // `transformRangeFunction` transforms the argument: after the item's own grammar,
        // before its alias list is counted and before the target list. In a statement's
        // own `FROM` no pass decides an aggregate standing here (`planner::ArgCertainty`),
        // so evaluating one was pglet's own `0A000` and a debug build's readers refused
        // the undecided verdict by panicking. An aggregate 18.6 assigns to an enclosing
        // query answers there and is refused here too (`EDGES.md` row 303).
        if arrays.iter().any(contains_aggregate) {
            return Err(Error::AggregateNotAllowed {
                context: "functions in FROM",
            });
        }
        // More aliases than the FROM item provides is `42P10` — the same rule and the
        // same `parse_relation.c` wording the `(VALUES …) AS k(…)` route above uses.
        // Written there only, this route silently **truncated** the list instead
        // (`FROM unnest(ARRAY[1,2]) AS k(a,b)` answered one column named `a`).
        if alias_columns.len() > arrays.len() {
            return Err(Error::TableColumnCountMismatch {
                table: alias_name.unwrap_or_else(|| "unnest".to_string()),
                available: arrays.len(),
                specified: alias_columns.len(),
            });
        }
        let single_function = arrays.len() == 1;
        let columns: Vec<String> = (0..arrays.len())
            .map(|i| {
                alias_columns.get(i).cloned().unwrap_or_else(|| {
                    alias_name
                        .as_ref()
                        .filter(|_| single_function)
                        .map_or_else(|| "unnest".to_string(), Clone::clone)
                })
            })
            .collect();
        let table = alias_name.clone().unwrap_or_else(|| "unnest".to_string());
        // Read here, as written: the correlated pass resolves an array a sub-plan's
        // `FROM` holds, and the resolved form can read as typed where this one does not.
        let certain = CertainTypes::new(arrays.iter().map(certain_unnest_element_type).collect());
        Ok((
            Plan::FunctionScan {
                func: FromFunction::Unnest,
                args: arrays,
                columns,
                alias: table.clone(),
                certain,
            },
            (table, alias_name),
        ))
    }

    /// Parse a FROM-clause `generate_series(start, stop[, step]) [AS] alias[(col)]` into
    /// a `Plan::FunctionScan` over the integer overload it resolves to.
    ///
    /// The item's own grammar is read first; then the verdicts in
    /// `transformRangeFunction`'s order — an aggregate or window argument (`42803`,
    /// `42P20`), the overload and a `DISTINCT` (`planner::resolve_generate_series`, which
    /// refuses an argument naming a column), and an alias list longer than the one column
    /// (`42P10`). They are raised here, as the item is parsed, which is 18.6's order
    /// against the target list, `WHERE`, `ORDER BY` and later FROM items — its FROM
    /// clause is transformed before the target list, `WHERE` and `ORDER BY`, whose
    /// `42703`s pglet raises as it lowers the query — and **not** against the `WITH`
    /// clause, an earlier FROM item or the raw parse: a grammar error later in
    /// the statement, and an operator refusal an argument owes
    /// ([`Parser::owe_analysis_refusal`]), lose to these here and win on 18.6 (`EDGES.md`
    /// row 321). Owing them instead was measured to trade those members for the
    /// `42703`-after-the-item ones. The column is named by
    /// `chooseScalarFunctionAlias`, as [`Parser::parse_unnest_table`]'s single-array item
    /// is.
    fn parse_generate_series_table(&mut self) -> Result<(Plan, FromTable)> {
        const NAME: &str = "generate_series";
        self.ts.bump()?; // `generate_series`
        self.ts.expect(&TokenKind::LParen)?;
        let quantifier = self.parse_func_set_quantifier()?;
        let args = self.comma_list_until(&TokenKind::RParen, |p| p.parse_expr_bp(0))?;
        self.bound_func_set_quantifier(quantifier, args.is_empty())?;
        self.ts.expect(&TokenKind::RParen)?;
        let alias_name = self.parse_table_alias()?;
        let alias_columns = if alias_name.is_some() && self.ts.at(&TokenKind::LParen) {
            self.parse_paren_name_list()?
        } else {
            Vec::new()
        };
        crate::planner::transform_call_args(quantifier, &args)?;
        if args.iter().any(contains_aggregate) {
            return Err(Error::AggregateNotAllowed {
                context: "functions in FROM",
            });
        }
        if args.iter().any(contains_window) {
            return Err(Error::WindowingError {
                context: "functions in FROM",
            });
        }
        let (series, args) = crate::planner::resolve_generate_series(quantifier, args)?;
        let table = alias_name.clone().unwrap_or_else(|| NAME.to_string());
        if alias_columns.len() > 1 {
            return Err(Error::TableColumnCountMismatch {
                table,
                available: 1,
                specified: alias_columns.len(),
            });
        }
        let column = alias_columns
            .into_iter()
            .next()
            .unwrap_or_else(|| table.clone());
        Ok((
            Plan::FunctionScan {
                func: FromFunction::GenerateSeries(series),
                args,
                columns: vec![column],
                alias: table.clone(),
                certain: CertainTypes::new(vec![Some(series.type_oid())]),
            },
            (table, alias_name),
        ))
    }

    /// Parse a `WITH [RECURSIVE] name [(cols)] AS ( <body> ) [, …]` clause, planning
    /// each CTE body and registering it in the statement-scoped CTE context. An ordinary
    /// CTE body is a `SELECT` or a **data-modifying** `INSERT`/`UPDATE`/`DELETE …
    /// RETURNING`; a modifying CTE is recorded in [`Parser::materialized_ctes`] so the
    /// statement wraps it in a [`Plan::With`].
    ///
    /// Without `RECURSIVE` the bodies are parsed in declaration order and a CTE sees only
    /// the ones before it. Under `RECURSIVE` every name is visible to every body, and the
    /// list is resolved in dependency order ([`Parser::parse_with_recursive_list`]); a
    /// self-referencing CTE routes through [`Parser::parse_recursive_cte_body`]. A list
    /// the dependency pre-scan cannot read is walked in declaration order here, every
    /// body under `RECURSIVE` through [`Parser::parse_recursive_cte_body`].
    pub(super) fn parse_with_clause(&mut self) -> Result<()> {
        self.ts.expect_kw(Keyword::With)?;
        let recursive = self.ts.eat_kw(Keyword::Recursive)?;
        self.with_recursive = recursive;
        let resolved = if recursive {
            let resolved = self.parse_with_recursive_list();
            self.pending_ctes.clear();
            resolved?
        } else {
            false
        };
        if !resolved {
            self.parse_with_items_in_declaration_order(recursive)?;
        }
        // The whole list has parsed: fire the deferred verdict, if any. A duplicate name
        // outranks a `42P10` wherever both are pending, which is the order Postgres has
        // (`transformWithClause` before `analyzeCTE`) and the order this slot's
        // first-wins rule does **not** give on its own — the `42P10` can be recorded
        // first, by an earlier declaration than the one that duplicates a name.
        if let Some(err) = self.take_with_clause_verdict() {
            return Err(err);
        }
        Ok(())
    }

    /// Walk a `WITH` list's items in declaration order, each CTE recording itself into
    /// `self.ctes` (and `materialized_ctes` as appropriate).
    fn parse_with_items_in_declaration_order(&mut self, recursive: bool) -> Result<()> {
        self.comma_list(|p| {
            let name = p.expect_name("a CTE name")?;
            // Two `WITH` queries under one name is `42712`, and **the verdict is
            // recorded, not raised**. Postgres runs `transformWithClause`'s collision
            // scan over the whole raw list *after* the statement has parsed and *before*
            // any `analyzeCTE`, and that is a split pglet does not have — `parse_cte_body`
            // parses and plans in one act. Recording here and firing after the list ends
            // reproduces both halves of the ordering that pglet *can* express: a body's
            // own **syntax** error still wins (it is raised while the list is still being
            // walked, as Postgres's raw parse is), and every `42P10` the clause raises
            // loses (each is deferred alongside this one, as `analyzeCTE` is).
            //
            // The check is here rather than in `register_cte`: that function is also
            // what both `WITH RECURSIVE` degrade arms call, and by then this same
            // declaration has already put its own placeholder in `ctes` — a
            // `contains_key` test there refuses every list-bearing `WITH RECURSIVE` as
            // its own duplicate (measured: three fixtures and a unit test).
            //
            // What is left is one shape, `EDGES.md`'s: a body error pglet raises at
            // **plan** time, which Postgres raises in analysis after the scan and pglet
            // raises during the walk. Hoisting the scan ahead of the bodies — a
            // token-level pre-pass over the names — closes that shape and opens two
            // syntax-error ones in exchange, since no token scan can tell a well-formed
            // body from a malformed one; measured, it is a net loss.
            if p.ctes.contains_key(&name) {
                let dup = name.clone();
                p.with_clause_verdict
                    .get_or_insert(Error::DuplicateCteName { name: dup });
            }
            // CTE column aliases (`WITH x(a, b) AS …`). Applied for a recursive CTE
            // (they name the working-table columns a self-reference resolves against);
            // parsed and ignored for an ordinary one (columns keep their names).
            let col_aliases = if p.ts.at(&TokenKind::LParen) {
                p.parse_paren_name_list()?
            } else {
                Vec::new()
            };
            p.ts.expect_kw(Keyword::As)?;
            // `MATERIALIZED` / `NOT MATERIALIZED` are 18.6's inlining *hints*
            // (`gram.y`'s `opt_materialized`). pglet has no inlining decision to hint —
            // a read-only CTE always inlines as a `SubqueryScan` and a data-modifying
            // one always materializes — so both spellings are consumed as noise words
            // rather than refused, which is the accept side 18.6 has.
            if p.ts.eat_kw(Keyword::Not)? {
                p.expect_ident("materialized")?;
            } else {
                p.eat_ident("materialized")?;
            }
            p.ts.expect(&TokenKind::LParen)?;
            let mark = p.touched_tables.len();
            if recursive {
                p.parse_recursive_cte_body(name.clone(), &col_aliases)?;
            } else {
                let cte_plan = p.parse_cte_body()?;
                p.register_cte(name.clone(), cte_plan, col_aliases)?;
            }
            p.record_cte_tables(name, mark);
            p.ts.expect(&TokenKind::RParen)?;
            Ok(())
        })?;
        Ok(())
    }

    /// Take the `WITH` clause's deferred verdict: the duplicate-name `42712` if one was
    /// seen, else the first deferred `42P10`.
    ///
    /// Two slots rather than one, because the two are recorded in *declaration* order
    /// and must be reported in *Postgres's* order: `WITH c(a,b) AS (SELECT 1), c AS
    /// (SELECT 2)` records the arity failure at the first declaration and the duplicate
    /// at the second, and 18.6 answers `42712`.
    pub(super) fn take_with_clause_verdict(&mut self) -> Option<Error> {
        self.with_clause_verdict
            .take()
            .or_else(|| self.with_clause_arity_verdict.take())
    }

    /// Record the base tables CTE `name`'s body named — the part of
    /// [`Parser::touched_tables`] from `mark` — for each inlined reference to it.
    pub(super) fn record_cte_tables(&mut self, name: String, mark: usize) {
        let tables = self.touched_tables[mark..].to_vec();
        self.cte_tables.insert(name, tables);
    }

    /// Register an **ordinary** (non-placeholder) CTE under `name`: check its
    /// `name(cols)` list against the body's arity, record it in the data-modifying list
    /// if the body writes, and store it in [`Parser::ctes`].
    ///
    /// The one registration site for the three routes that produce an ordinary CTE —
    /// the plain `WITH … AS (…)` arm and both `WITH RECURSIVE` **degrade** arms (a body
    /// with no combinator, and a `UNION` body with no self-reference, each of which
    /// Postgres silently treats as an ordinary CTE). Written per route, the alias list
    /// reached one of the three and the arity check none of them.
    ///
    /// **Fallible in effect, not in signature.** The `42P10` for a list naming more
    /// columns than the body provides is *recorded* on
    /// [`Parser::with_clause_arity_verdict`] and fired by [`Parser::parse_with_clause`]
    /// once the whole list has parsed, so this function cannot fail. Postgres's
    /// `analyzeCTE` raises this while analysing the `WITH` clause, **before** the body
    /// is referenced, so an unreferenced CTE raises too — which is why the check lives
    /// here and not at the reference site. What it cannot reproduce is 18.6's ordering
    /// *against the body's own errors*: `analyzeCTE` analyses the body first, so
    /// `WITH c(w,x) AS (SELECT 1 FROM nosuchtbl) SELECT 1` is `42P01` there. pglet does
    /// not analyse an unreferenced CTE body at all (`EDGES.md`), so that statement is
    /// this `42P10` here — a loud answer where the pre-existing gap gave a silent
    /// success, and the ordering follows the gap rather than causing it.
    pub(super) fn register_cte(
        &mut self,
        name: String,
        plan: Plan,
        col_aliases: Vec<String>,
    ) -> Result<()> {
        if !col_aliases.is_empty() {
            let available = plan.output_columns(self.schema).len();
            if col_aliases.len() > available {
                // Recorded, not raised — `analyzeCTE` runs after `transformWithClause`'s
                // duplicate scan, so a `42P10` must not beat a `42712` recorded by a
                // *later* declaration. `parse_with_clause` fires whichever wins once the
                // whole list has parsed. Registration continues below with the list
                // applied as far as it reaches: the statement is going to be refused, and
                // leaving the name unregistered would make a later body's reference to it
                // raise something else first.
                let (bad, specified) = (name.clone(), col_aliases.len());
                self.with_clause_arity_verdict
                    .get_or_insert(Error::CteColumnCountMismatch {
                        name: bad,
                        available,
                        specified,
                    });
            }
        }
        // A data-modifying CTE body (`INSERT/UPDATE/DELETE`) is materialized once by the
        // wrapping `Plan::With`; record it (with its plan and alias list) in declaration
        // order. It also goes in `ctes` so a reference resolves to a `CteScan`.
        let rank = self.next_cte_rank;
        self.next_cte_rank += 1;
        if plan.mutates_storage() {
            self.materialized_ctes.push(crate::planner::CtePlan {
                name: name.clone(),
                rank,
                body: crate::planner::CteBody::Modifying {
                    plan: Box::new(crate::depth::clone_plan(&plan)?),
                    column_aliases: col_aliases.clone(),
                },
            });
        }
        self.ctes.insert(
            name,
            super::CteEntry {
                plan,
                column_aliases: col_aliases,
                rank,
            },
        );
        Ok(())
    }

    /// Parse the body of a CTE under `WITH RECURSIVE` — a `<non-recursive> UNION [ALL]
    /// <recursive>` whose recursive term references `name` — and register the result.
    ///
    /// A [`Plan::CteScan`] placeholder for `name` is registered **before** the seed
    /// (non-recursive) term is parsed, so the CTE name **shadows** any same-named real
    /// table throughout the body (Postgres semantics) and a self-reference resolves
    /// through the ordinary FROM path at any depth. Each FROM reference to `name` — the
    /// name, held in [`Parser::defining_cte`] while the body parses, not the kind of
    /// entry it resolves to — increments [`Parser::recursive_ref_count`] (it also counts
    /// refs inside expression sub-links a plan-walk can't see, and drives the degrade
    /// decision). The counter is zeroed around the seed parse and again around the
    /// recursive-term parse so the two are measured independently.
    ///
    /// Postgres's `42P19` (`invalid_recursion`) rules, each verified against
    /// `parse_cte.c`: a self-reference in the non-recursive term (`must not appear
    /// within its non-recursive term`), a combinator other than `UNION` between the
    /// terms (`does not have the form …`), a self-reference more than once (`must not
    /// appear more than once`), a self-reference on the nullable side of an outer join
    /// (`must not appear within an outer join`, via [`check_recursive_placement`]), or
    /// an aggregate in the recursive term. `UNION` with no self-reference degrades to
    /// an ordinary `UNION` CTE (Postgres's silent degrade); no combinator at all
    /// degrades to a plain single-term CTE. A well-formed recursive CTE is
    /// shape-checked ([`check_recursive_shape`]) and pushed onto
    /// [`Parser::materialized_ctes`], its placeholder left in `ctes` for every later
    /// reader. Under a list the dependency pre-scan read, the placement rules it can
    /// judge from the raw tokens have already run (`parser::with_recursive`), and a
    /// sub-link self-reference is refused there; the checks here are the ones that
    /// need the planned tree.
    ///
    /// Every error path removes the placeholder (it was inserted before the checks) so
    /// a half-registered name can't leak into the statement's CTE scope.
    ///
    /// # Errors
    ///
    /// The `42P19` cases above, plus `42601`/`42804`/`42P10` from the shape/column
    /// checks (and any parse error from the two operands).
    pub(super) fn parse_recursive_cte_body(
        &mut self,
        name: String,
        col_aliases: &[String],
    ) -> Result<()> {
        let saved = self.defining_cte.replace(name.clone());
        let parsed = self.parse_recursive_cte_terms(name, col_aliases);
        self.defining_cte = saved;
        parsed
    }

    /// [`Self::parse_recursive_cte_body`] with [`Parser::defining_cte`] set.
    fn parse_recursive_cte_terms(&mut self, name: String, col_aliases: &[String]) -> Result<()> {
        // Register a placeholder BEFORE the seed so the CTE name shadows any same-named
        // real table (Postgres semantics). Its provisional columns (alias-list names,
        // untyped) are only consumed if the seed self-references — which is rejected.
        self.ctes.insert(
            name.clone(),
            super::CteEntry {
                plan: Plan::CteScan {
                    name: name.clone(),
                    alias: name.clone(),
                    columns: provisional_recursive_columns(col_aliases),
                },
                // The placeholder's `columns` are already the alias list's names, so it
                // carries no pending rename.
                column_aliases: Vec::new(),
                rank: self.next_cte_rank,
            },
        );

        // Parse the seed with the self-ref counter zeroed, so a seed self-reference
        // (illegal) is distinguishable from a recursive-term one.
        let saved_count = std::mem::replace(&mut self.recursive_ref_count, 0);
        let non_recursive = self
            .parse_setop_operand()
            .map_err(|e| self.fail_recursive(&name, saved_count, e))?;
        if self.recursive_ref_count > 0 {
            let err = Error::InvalidRecursion(format!(
                "recursive reference to query \"{name}\" must not appear within its \
                 non-recursive term"
            ));
            return Err(self.fail_recursive(&name, saved_count, err));
        }

        // A non-`UNION` combinator is not a recursive-query form (42P19).
        if self.ts.at_kw(Keyword::Intersect) || self.ts.at_kw(Keyword::Except) {
            let err = invalid_recursion_form(&name);
            return Err(self.fail_recursive(&name, saved_count, err));
        }
        // No combinator → not recursive; register as a plain read-only CTE (overwriting
        // the placeholder). It never resolved as a self-reference, so a body reference
        // inlines as a `SubqueryScan`.
        if !self.ts.at_kw(Keyword::Union) {
            // A degrade is an **ordinary** CTE, alias list and all — it routes through
            // `register_cte` for exactly that reason: written inline, this arm and the
            // one below dropped the list that `recursive_cte_columns` applies on the
            // genuinely-recursive path, so `WITH RECURSIVE r(n) AS (SELECT 1) SELECT n
            // FROM r` was `42703` where the same body under a plain `WITH` now answers.
            self.register_cte(name, non_recursive, col_aliases.to_vec())?;
            self.recursive_ref_count = saved_count;
            return Ok(());
        }
        self.ts.bump()?; // UNION
        let all = self.parse_set_quantifier()?;

        // The working-table columns: the seed's output, renamed by the alias list.
        // Re-insert the placeholder with the real (seed-derived) columns before the
        // recursive term so `n` resolves to the seed's type.
        let columns = recursive_cte_columns(&name, &non_recursive, col_aliases, self.schema)
            .map_err(|e| self.fail_recursive(&name, saved_count, e))?;
        self.ctes.insert(
            name.clone(),
            super::CteEntry {
                plan: Plan::CteScan {
                    name: name.clone(),
                    alias: name.clone(),
                    columns: columns.clone(),
                },
                column_aliases: Vec::new(),
                rank: self.next_cte_rank,
            },
        );

        // Parse the recursive term with the counter zeroed again, then restore the
        // outer scope's count.
        // The term is `UNION`'s right operand, which an `INTERSECT` binds tighter than:
        // `… UNION ALL SELECT … FROM r INTERSECT SELECT 2` is `… UNION ALL (… INTERSECT
        // …)`, a recursive term 18.6 accepts (its context rule is
        // `check_recursive_placement`'s).
        self.recursive_ref_count = 0;
        let recursive = self
            .parse_setop_operand()
            .and_then(|term| self.parse_intersect_rest(term))
            .map_err(|e| self.fail_recursive(&name, saved_count, e))?;
        let ref_count = self.recursive_ref_count;
        self.recursive_ref_count = saved_count;

        // Only the two-term `<seed> UNION [ALL] <recursive>` form is modeled; a further
        // set-op is a chain — fail loud (Postgres also rejects a self-reference in a
        // compound non-recursive term with 42P19).
        if self.at_set_op() {
            let err = invalid_recursion_form(&name);
            return Err(self.fail_recursive(&name, saved_count, err));
        }
        // An `ORDER BY` / `OFFSET` / `LIMIT` / locking clause over the recursive query is
        // 18.6's `0A000`, raised from the list's pre-scan before any body parses; read
        // here too, it is a refusal wherever a body is parsed without that verdict ahead
        // of it — once the clauses have passed their grammar, whose `42601` the raw parse
        // reports first. A `FOR READ ONLY` alone locks nothing and is consumed.
        if ref_count > 0 && self.at_query_tail() {
            let refusal = self.parse_query_tail(QueryClauses::default());
            if let Some(err) = refusal.map_or_else(Some, |tail| tail.refusal()) {
                return Err(self.fail_recursive(&name, saved_count, err));
            }
        }

        // A self-reference on the nullable side of an outer join is illegal (42P19).
        if let Err(e) = check_recursive_placement(&recursive, &name) {
            return Err(self.fail_recursive(&name, saved_count, e));
        }

        if ref_count == 0 {
            // RECURSIVE but no self-reference → an ordinary `UNION` CTE (Postgres
            // degrades silently). Overwrite the placeholder with the combined plan.
            let plan = self
                .build_set_op(SetOpKind::Union, all, non_recursive, recursive)
                .map_err(|e| self.fail_recursive(&name, saved_count, e))?;
            self.register_cte(name, plan, col_aliases.to_vec())?;
            return Ok(());
        }
        // A self-reference appearing more than once is illegal (42P19).
        if ref_count > 1 {
            let err = Error::InvalidRecursion(format!(
                "recursive reference to query \"{name}\" must not appear more than once"
            ));
            return Err(self.fail_recursive(&name, saved_count, err));
        }

        let typmods = check_recursive_shape(&name, &recursive, &columns, self.schema)
            .map_err(|e| self.fail_recursive(&name, saved_count, e))?;
        // The CTE's own columns declare the **set operation's** modifier, which is the
        // seed's only where the recursive term carries the same one. The entry the
        // recursive term was parsed against kept the seed's, as 18.6's `ctecoltypmods`
        // does while it analyses that term; the entry the **body** resolves against is
        // re-registered here, so the plan-side reader (`Plan::CteScan` →
        // `Plan::result_shape` → `Describe`) and the executed one
        // (`MaterializedCte::columns`, from `CteBody::Recursive`) declare the same list.
        // Updating only one of them is what `cte_column_list`'s two-producer assertion
        // caught.
        //
        // The modifiers are read **positionally and never zipped**. `check_recursive_shape`
        // returns one per column and the column counts are equal by the time it does, but a
        // `zip` encodes that as a silent truncation: one short answer would drop the last
        // column from the relation the body resolves against, which is the half-added-column
        // shape — every `SELECT *` over the CTE losing a field at the wire with no error
        // anywhere. Indexing degrades to `-1`, which is the honest modifier for a column
        // nothing decided, and keeps the list whole. The column's certainty rides with
        // its modifier, and degrades to not certain.
        let columns: Vec<ColumnDesc> = columns
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let (type_modifier, type_certain) = typmods.get(i).copied().unwrap_or((-1, false));
                ColumnDesc {
                    type_modifier,
                    type_certain,
                    ..c
                }
            })
            .collect();
        self.ctes.insert(
            name.clone(),
            super::CteEntry {
                plan: Plan::CteScan {
                    name: name.clone(),
                    alias: name.clone(),
                    columns: columns.clone(),
                },
                column_aliases: Vec::new(),
                rank: self.next_cte_rank,
            },
        );
        let rank = self.next_cte_rank;
        self.next_cte_rank += 1;
        self.materialized_ctes.push(crate::planner::CtePlan {
            name,
            rank,
            body: crate::planner::CteBody::Recursive {
                columns,
                non_recursive: Box::new(non_recursive),
                recursive: Box::new(recursive),
                all,
            },
        });
        Ok(())
    }

    /// Remove the recursive-CTE placeholder for `name` and restore the self-ref
    /// counter to `saved_count`, then return `err`. The cleanup every error path in
    /// [`Self::parse_recursive_cte_body`] runs (the placeholder is inserted before the
    /// checks, so a leak would shadow a real table for the rest of the statement).
    fn fail_recursive(&mut self, name: &str, saved_count: usize, err: Error) -> Error {
        self.ctes.remove(name);
        self.recursive_ref_count = saved_count;
        err
    }

    /// Parse a CTE body: a `SELECT`, or a data-modifying `INSERT`/`UPDATE`/`DELETE`.
    ///
    /// A modifying body without a `RETURNING` clause exposes no columns to the outer
    /// query, so a reference to it yields a **zero-column** relation here.
    ///
    /// That is **not** what Postgres does, and the claim that it was is the reason this
    /// shape is accepted: measured on 18.6, referencing a `RETURNING`-less
    /// data-modifying CTE is `0A000 WITH query "c" does not have a RETURNING clause`,
    /// raised during parse analysis, and the body's write never runs. pglet accepts the
    /// statement and performs the write. Recorded as `EDGES.md` row 61; not fixed here.
    pub(super) fn parse_cte_body(&mut self) -> Result<Plan> {
        // A CTE body is a FROM item like a derived table — see `in_from_item`.
        let plan = self.in_from_item(|p| {
            if p.ts.at_kw(Keyword::Insert) {
                p.parse_insert()
            } else if p.ts.at_kw(Keyword::Update) {
                p.parse_update()
            } else if p.ts.at_kw(Keyword::Delete) {
                p.parse_delete()
            } else {
                p.parse_select()
            }
        })?;
        // Every later reference inlines a copy of this plan under its own query, so a
        // chain of CTEs each reading the last grows without the parser recursing.
        crate::depth::check(crate::depth::plan_depth(&plan) + self.depth)?;
        Ok(plan)
    }

    /// Parse an optional table alias (`[AS] alias`).
    pub(super) fn parse_table_alias(&mut self) -> Result<Option<String>> {
        let aliased = self.ts.eat_kw(Keyword::As)?
            || matches!(
                self.ts.peek_kind(),
                TokenKind::Identifier | TokenKind::QuotedIdentifier
            );
        if aliased {
            Ok(Some(self.expect_alias_name()?))
        } else {
            Ok(None)
        }
    }

    /// Whether a JOIN clause begins at the cursor.
    fn at_join_start(&self) -> bool {
        self.ts.at_kw(Keyword::Join)
            || self.ts.at_kw(Keyword::Inner)
            || self.ts.at_kw(Keyword::Left)
            || self.ts.at_kw(Keyword::Right)
            || self.ts.at_kw(Keyword::Full)
            || self.ts.at_kw(Keyword::Cross)
            || self.ts.at_kw(Keyword::Natural)
    }
}

/// Plan a `SELECT` with no FROM clause: a single-row `Values`, or `ProjectUnnest`
/// when a projection-level `unnest()` row-expands.
fn plan_select_no_from(row: Vec<Expr>, schema: &crate::catalog::Schema) -> Result<Plan> {
    if row.iter().any(Expr::is_bare_star) {
        return Err(Error::NotImplemented(
            "wildcard in SELECT without FROM not supported".into(),
        ));
    }
    // This path has no columns to resolve, but a no-FROM projection still needs the
    // full type-resolution pass with an empty column map: comparison-operand coercion
    // (`SELECT true = 'yes'`, `1 <> '1 '`, `'abc' = 1`), CASE/COALESCE branch
    // unification (`CASE WHEN … THEN 1 ELSE 'x' END`), greatest/least/nullif common-type
    // unification (else `greatest(1::int4, NULL::int8)` declares int8 but returns an int4
    // value — a binary-protocol mis-decode), and eager constant-cast validation (an
    // unparseable literal / untaken branch fails loud at plan time, like Postgres).
    //
    // Each entry is also read for the type it is certainly of, as an entry over a `FROM`
    // is (`lower_select`), for a query that reads this one as a relation.
    let (row, certain) = resolve_entries_gated(row, &ColumnScope::default(), schema)?;
    match row.iter().filter(|e| contains_unnest(e)).count() {
        0 => Ok(Plan::Values {
            rows: vec![row],
            certain: CertainTypes::new(certain),
            enclosing: EnclosingColumns::default(),
        }),
        1 => Ok(Plan::ProjectUnnest { exprs: row }),
        _ => Err(Error::NotImplemented(
            "multiple unnest() in a SELECT projection not supported".into(),
        )),
    }
}

/// The Postgres-shaped `42P19` for a `WITH RECURSIVE` body that isn't
/// `non-recursive-term UNION [ALL] recursive-term`.
pub(super) fn invalid_recursion_form(name: &str) -> Error {
    Error::InvalidRecursion(format!(
        "recursive query \"{name}\" does not have the form \
         non-recursive-term UNION [ALL] recursive-term"
    ))
}

/// The working/result column descriptors of a recursive CTE: the seed (non-recursive)
/// term's output columns, **renamed** by the `name(cols)` alias list where given
/// (their types stay the seed's — the authoritative side). The names matter: a
/// recursive term's `SELECT n+1 FROM t` resolves `n` against these.
///
/// # A rebuilt descriptor carries what the seed's derived, field by field
///
/// This is the one producer of a genuinely-recursive CTE's descriptors, and both the
/// plan-side reader (the `Plan::CteScan` the body's `FROM` resolves to, whose
/// `output_columns_scoped` arm hands to the namespaces and gates around it) and the
/// eval-side one (`executor::MaterializedCte::columns`, which the comparator reads)
/// take them from here. So each field is decided here rather than defaulted, and
/// `ColumnDesc::new` is not the constructor for the job — it answers `None`/`-1` for
/// three of them:
///
/// - **`table_oid` / `column_id` are `0`** — a recursive CTE's column is the set
///   operation of its two terms, so it has no single source column. 18.6 sends `0`/`0`
///   here too (measured, against the *non*-recursive CTE beside it, where both engines
///   send the base table's oid and attnum).
/// - **`type_modifier` starts as the seed's and is then resolved against the recursive
///   term** by [`check_recursive_shape`], whose answer the caller writes back over this
///   list. Carried, a `VARCHAR(5)` seed is typmod `9` and a `NUMERIC(6,2)` one `393222`
///   on both engines, and dropping it made a width-deriving client read the column as
///   unbounded. But it is the seed's only while the recursive term declares the same
///   one: a `::varchar(8)` term makes the set operation's modifier `-1`, and keeping the
///   seed's there declared `character varying(5)` over a row of **eight** characters —
///   the wrong value, not the missing one. What this list holds is therefore the
///   *starting* value, and the working-table entry the recursive term binds against, as
///   18.6's `ctecoltypmods` is while it analyses that term.
/// - **`type_certain` starts as the seed's and is then resolved against the recursive
///   term**, as the modifier is: the column's type is the seed's, and it is certain
///   where the recursive term's is certain too ([`check_recursive_shape`]) — the set
///   operation's rule (`Plan::SetOp`'s columns). The entry the recursive term binds
///   against carries the seed's.
/// - **`enum_type` is the seed's.** Dropping it left the recursive CTE's column ordering
///   by its label bytes where 18.6 orders by member position — a wrong row set, not only
///   a wrong order — and turned the comparisons against a real enum column measured into
///   `42883 operator does not exist: mood > text`, which is `EDGES.md` row 114's class
///   at this door (`docs/enums.md`).
///
/// # Errors
///
/// More aliases than the seed produces columns → `42P10` (Postgres: `WITH query "t"
/// has N columns available but M columns specified`).
fn recursive_cte_columns(
    name: &str,
    non_recursive: &Plan,
    col_aliases: &[String],
    schema: &crate::catalog::Schema,
) -> Result<Vec<ColumnDesc>> {
    let base = non_recursive.output_columns(schema);
    if col_aliases.len() > base.len() {
        return Err(Error::CteColumnCountMismatch {
            name: name.to_string(),
            available: base.len(),
            specified: col_aliases.len(),
        });
    }
    Ok(base
        .into_iter()
        .enumerate()
        .map(|(i, c)| ColumnDesc {
            name: col_aliases.get(i).cloned().unwrap_or(c.name),
            type_oid: c.type_oid,
            type_modifier: c.type_modifier,
            table_oid: 0,
            column_id: 0,
            enum_type: c.enum_type,
            type_certain: c.type_certain,
        })
        .collect())
}

/// Validate a genuinely-recursive CTE's shape against Postgres's rules: no aggregate
/// / window in the recursive term (`42P19`), equal column counts between the terms
/// (`42601`), and the recursive term implicitly castable to the non-recursive column
/// types (`42804`, non-recursive authoritative — carried in `columns`).
///
/// The two terms are folded by [`select_common_type`] under the `UNION` context, which
/// is where Postgres puts this too: a pair with **no** common type at all is the set
/// operation's own `42804`/`42846` (`UNION types integer and text cannot be matched`),
/// raised by `transformSetOperationTree` *before* `analyzeCTE` ever runs its
/// recursive-type comparison. Only a pair that does unify, to something other than the
/// non-recursive term's type, reaches the `has type … but type … overall` message.
///
/// # The modifier is **resolved** rather than compared, and that is a deliberate split
///
/// 18.6 tests type and modifier in one `if`
/// (`exprType(texpr) != lctyp || exprTypmod(texpr) != lctypmod`, against the set
/// operation's own resolved column), so a `VARCHAR(5)` seed beside a `::varchar(8)`
/// recursive term is `42804 recursive query "c" column 1 has type character varying(5)
/// in non-recursive term but type character varying overall` — measured, at that
/// spelling and at `::varchar(3)`, `::varchar`, a bare `'xy'`, `COALESCE(a, 'q')`,
/// `a || 'z'`, `numeric(10,5)`, `timestamp(6)` and an array's `text[]`.
///
/// **pglet resolves that modifier and does not raise on it**, returning
/// [`select_common_typmod`]'s answer per column for the caller to declare. The reason is
/// that pglet cannot compute 18.6's `exprTypmod`: a `::` cast's modifier is dropped
/// where the parser reads the type name and a value constructor does not propagate its
/// arms' (`EDGES.md` row 155), so the recursive term reads `-1` at spellings whose
/// modifier 18.6 knows exactly. Raising on the comparison was implemented and measured
/// against 18.6 before this: it refused a whole family of queries 18.6 **answers** —
/// a cast to the seed's *own* width (`a::varchar(5)`, `'xy'::varchar(5)`,
/// `1.5::numeric(6,2)`) and a value constructor over the working-table column
/// (`CASE`, `COALESCE`, `GREATEST`, `NULLIF`, a scalar subquery), each of which 18.6
/// declares at the seed's modifier. A gate over a value this engine cannot read is a
/// coin flip, and it landed wrong at every member measured. The members are the table in
/// `cte_column_list`'s `a_modifier_comparison_would_wrongly_refuse_these`, which is where
/// they are kept rather than counted here.
///
/// So the rule pglet aims at is the weaker, checkable half: **it declares no modifier its
/// own rows need not satisfy** — over the modifiers it reads. Where the two sides agree
/// the seed's rides out (a `VARCHAR(5)` seed beside its own working-table column is typmod
/// `9` on both engines); where they disagree the column declares `-1`, which every row
/// satisfies. The rule is only as good as the read: a term whose column reached it through
/// a lookup that took another relation's same-named column — a window query's projected
/// column, a qualified `unnest` column, both closed (`EDGES.md` row 158) — carried that
/// relation's modifier, and beside a seed of that modifier the fold kept it over rows that
/// violate it — measured, `varchar(5)` declared over an eight-character row.
///
/// What is left at the spellings 18.6 refuses is an **over-acceptance**, in two subsets
/// with different repairs (`docs/declared_types.md` § A recursive CTE's declared
/// modifier). Where the recursive term reads `-1` for a modifier 18.6 knows (a cast, a
/// value constructor) the fold is not 18.6's, and that subset — with the
/// **under-declaration** beside it, `-1` here against the modifier 18.6 declares — waits
/// on row 155. Where the term reads a modifier exactly and it differs (a `VARCHAR(8)`
/// column beside a `VARCHAR(5)` seed, a `TIMESTAMP(3)` column beside a `TIMESTAMPTZ(3)`
/// one) nothing in row 155 is missing; a refusal decided only where both sides are read
/// could close it, on the premise that a read modifier is the column's own — the premise
/// row 158 falsified until it closed.
///
/// The **type**-mismatch message below names the modifier
/// ([`format_type_with_typemod`](crate::deparse::format_type_with_typemod)) because 18.6
/// spells both its halves that way — `has type numeric(6,2) in non-recursive term but
/// type double precision overall`, byte-identical there and here, where the bare name
/// used to read `numeric`. That renderer is not part of the modifier fold and the fold
/// does not need it: the fold raises nothing. The two arrived together and are otherwise
/// unrelated.
///
/// # Returns
///
/// Per column, positionally: the set operation's resolved type modifier — what the CTE's
/// own columns declare — and whether the column's type is certain
/// (`ColumnDesc::type_certain`): where the seed's and the recursive term's both are, the
/// rule a set operation's column takes.
///
/// # Errors
///
/// `42P19` / `42601` / `42804` / `42846` as above.
fn check_recursive_shape(
    name: &str,
    recursive: &Plan,
    columns: &[ColumnDesc],
    schema: &crate::catalog::Schema,
) -> Result<Vec<(i32, bool)>> {
    // An aggregate / window in the recursive term is forbidden (42P19).
    if recursive_term_has_aggregate(recursive) {
        return Err(Error::InvalidRecursion(
            "aggregate functions are not allowed in a recursive query's recursive term".into(),
        ));
    }
    let rec_cols = recursive.output_columns(schema);
    // Equal column counts (42601) — the same message a plain UNION mismatch reports.
    if rec_cols.len() != columns.len() {
        return Err(Error::syntax(
            "each UNION query must have the same number of columns",
        ));
    }
    // The non-recursive term (its types are in `columns`) is authoritative; each
    // recursive column must implicitly widen to it — i.e. their common type *is* the
    // non-recursive type. Otherwise the "overall" type would differ (42804).
    //
    // A recursive column whose type pglet **could not infer** is skipped, because its
    // `ColumnDesc` fell back to `text` and judging that would report a type the query
    // never had. Postgres treats such a column as `unknown` and coerces it to the
    // non-recursive type, so `SELECT 1 UNION ALL SELECT 'x'` is a `22P02` coercion
    // failure there, not a `42804` mismatch. The reachable case here is a *compound*
    // expression over the working table — `val * val`, whose operands are working-table
    // columns the catalog cannot type — and skipping it restores exactly the queries the
    // previous (accidentally permissive) fold accepted, without the accident: the old
    // rule waved `numeric` vs `text` through because `numeric_family_result` answered
    // `Some(numeric)` whenever *either* side was numeric, whatever the other was.
    let rec_inferable = recursive_term_inferred_types(recursive, schema, rec_cols.len());
    // Which recursive-term entries are `unknown` — the set operation's own predicate,
    // read here so the two folds cannot disagree about what an unresolved literal is.
    let rec_unknown = crate::planner::unknown_output_columns(recursive, schema, rec_cols.len());
    let mut typmods = Vec::with_capacity(columns.len());
    for (i, (want, got)) in columns.iter().zip(&rec_cols).enumerate() {
        let certain = want.type_certain && got.type_certain;
        // The **modifier** is folded for every column, the skipped ones included: the
        // skip below says pglet could not read the recursive term's *type*, which is a
        // reason to declare no modifier rather than a reason to keep the seed's.
        let fold_typmod = |common: TypeOid| {
            select_common_typmod(
                &[
                    (want.type_oid, want.type_modifier),
                    (got.type_oid, got.type_modifier),
                ],
                common,
            )
        };
        if rec_inferable.get(i).copied() == Some(None) {
            typmods.push((fold_typmod(want.type_oid), certain));
            continue;
        }
        // Both sides are read as [`ResolvedType`]s: an enum's OID is `text`, so folding
        // the OIDs alone made `SELECT t … UNION ALL SELECT m …` a `text` column where
        // 18.6 refuses the pair `42804` (`EDGES.md` row 114).
        //
        // **An `unknown` entry is folded by the OIDs alone**, which is what this did
        // before the seed's descriptor carried an identity at all. 18.6 does not fold
        // such an entry at all — it *converts* it, running the target's input function,
        // so `… UNION ALL SELECT 'sad'` over a `mood` seed is two rows there and
        // `'zzz'` is `22P02`. This arm keeps the enum identity **out** of the fold for
        // that reason, and only for it: reading the identity here refused the member
        // literal 18.6 accepts, measured, before the arm existed.
        //
        // What the arm does **not** do is make the missing conversion quiet. Folding an
        // unresolved literal against the seed's type instead of converting it to it is
        // `EDGES.md` row 156, and it is *loud* — and wrong — at every seed type outside
        // the string category: an `int`, `date`, `timestamptz`, `bool`, `int[]`,
        // `jsonb`, `uuid` or `numeric` seed beside a bare `'7'` / `'2021-02-03'` / … is
        // `42804 UNION types <T> and text cannot be matched` here where 18.6 answers,
        // and a literal that does not convert is 18.6's `22P02`/`22007` against the same
        // `42804`. Its *string-category* face — `'zzz'` over a `mood` seed, where the
        // fold succeeds — is the conversion below, which an enum seed takes and a
        // string one needs no more of.
        let unknown_entry = rec_unknown.get(i).copied() == Some(true);
        let resolved_side = |c: &ColumnDesc| {
            if unknown_entry {
                ResolvedType::plain(c.type_oid)
            } else {
                ResolvedType::of_column(c)
            }
        };
        let nr = resolved_side(want);
        let rec = resolved_side(got);
        // A pair with no common type propagates the `UNION` error itself; only a
        // successful fold that lands somewhere other than `nr` is the recursive-query
        // complaint. Folding raw here reported the latter with an "unknown overall" for
        // the former, where Postgres reports the former outright.
        let overall = select_common_type(&[Some(nr.clone()), Some(rec.clone())], "UNION")?;
        // The folded `unknown` entry is then **converted** to the seed's type, as
        // `transformSetOperationTree` converts it — and an enum's conversion is its
        // input function, the membership test the set-operation door runs too. Without
        // it `… UNION ALL SELECT 'zzz' FROM r WHERE false` over a `mood` seed answered
        // the seed's rows, and wrote them through a data-modifying reader, where 18.6
        // refuses `22P02` in parse analysis whatever the rows.
        if unknown_entry && let Some(enum_name) = want.enum_type.as_deref() {
            crate::planner::unknown_entry_enum_legality(recursive, i, enum_name, schema)?;
        }
        // The **modifier** is the set operation's and is never compared: it is what the
        // CTE's column declares, and `-1` the moment the two sides disagree.
        let overall_typmod = overall.as_ref().map_or(-1, |t| fold_typmod(t.oid));
        typmods.push((overall_typmod, certain));
        if overall.as_ref() != Some(&nr) {
            // The catalog spelling, with the modifier at 18.6's own position — the two
            // halves of this sentence are otherwise the same words whenever it is the
            // modifier that differs.
            let spell = |oid: TypeOid, typmod: i32| {
                crate::deparse::format_type_with_typemod(oid, typmod)
                    .unwrap_or_else(|_| "unknown".to_string())
            };
            let overall_name = overall.map_or_else(
                || "unknown".to_string(),
                |t| {
                    t.enum_type
                        .name()
                        .map_or_else(|| spell(t.oid, overall_typmod), str::to_string)
                },
            );
            return Err(Error::Type(format!(
                "recursive query \"{name}\" column {} has type {} in non-recursive term \
                 but type {overall_name} overall",
                i + 1,
                nr.enum_type
                    .name()
                    .map_or_else(|| spell(nr.oid, want.type_modifier), str::to_string),
            )));
        }
    }
    Ok(typmods)
}

/// Per-column: whether the recursive term's projection expression has a type pglet can
/// actually infer (`Some(t)`), or none at all (`None`).
///
/// `Plan::output_columns` cannot answer this — it substitutes `text` for an
/// un-inferable expression, which is indistinguishable from a genuinely `text` one, and
/// the recursive-type check must not report a mismatch against a fallback.
///
/// It is the un-fallback-ed twin of `Plan::output_columns`'s `Project` arm and so
/// must resolve in the **same** namespace ([`TypeScope::Input`]); a scope that sees
/// fewer columns than `output_columns` does would report `None` — "not inferable" —
/// for a column `output_columns` typed exactly, and silently skip the check.
///
/// Only a direct `Project` is inspected; any other shape yields an empty vector and the
/// caller checks every column as before (a conservative default — it can only *add*
/// checks, never skip one). `count` is the expected column count, used to keep the
/// result positionally aligned with `output_columns` (a `*` projection fans out, so a
/// length mismatch means the positions don't correspond and the whole vector is dropped).
fn recursive_term_inferred_types(
    recursive: &Plan,
    schema: &crate::catalog::Schema,
    count: usize,
) -> Vec<Option<TypeOid>> {
    let Plan::Project { input, exprs, .. } = recursive else {
        return Vec::new();
    };
    if exprs.len() != count {
        return Vec::new();
    }
    let (input_shape, input_relations) = input.output_columns_scoped(schema);
    let scope = PlannerScope::new(
        schema,
        TypeScope::Input {
            relations: &input_relations,
            columns: input_shape.columns(),
            enclosing: None,
        },
    );
    exprs.iter().map(|e| e.infer_type(&scope)).collect()
}

/// The provisional working-table columns registered for a recursive-CTE placeholder
/// **before** the seed is parsed — the `name(cols)` alias-list names, typed `text`.
/// They exist only so the name shadows a same-named real table; they're consumed only
/// if the seed self-references, which is then rejected (`42P19`), so their (untyped)
/// type never reaches a kept plan. Empty when there is no alias list.
fn provisional_recursive_columns(col_aliases: &[String]) -> Vec<ColumnDesc> {
    col_aliases
        .iter()
        .map(|name| ColumnDesc::new(name.clone(), TypeOid::Text))
        .collect()
}

/// Reject a self-reference to `name` on the **nullable** side of an outer join in the
/// recursive term — Postgres's `42P19` `recursive reference to query "%s" must not
/// appear within an outer join`. A `LEFT` join's right subtree is nullable, a `RIGHT`
/// join's left, a `FULL` join's both; an `INNER` join nulls neither (but is descended
/// for nested joins). Descends the row-shaping wrappers, subquery scans, and set-op
/// sides too, so an outer join nested anywhere in the term is checked — and a set
/// operation's arm is judged by its operator on the way down, `… within INTERSECT` /
/// `… within EXCEPT` as 18.6 words them.
///
/// A self-reference inside a plain FROM-clause **derived table** (no enclosing outer
/// join) is *not* rejected — Postgres allows it (`FROM (SELECT … FROM t) sub`). Only
/// the outer-join placement is illegal. (Self-references inside expression sub-links
/// live in `Expr`s, not plan children, so they aren't reached here — the list's
/// pre-scan refuses those with 18.6's `within a subquery`.)
///
/// # Errors
///
/// `42P19` when a self-reference sits on a nullable outer-join side or in an arm a set
/// operation closes.
fn check_recursive_placement(plan: &Plan, name: &str) -> Result<()> {
    crate::depth::check_stack()?;
    match plan {
        Plan::Join {
            left,
            right,
            join_type,
            ..
        } => {
            let (left_nullable, right_nullable) = match join_type {
                JoinType::Inner => (false, false),
                JoinType::Left => (false, true),
                JoinType::Right => (true, false),
                JoinType::Full => (true, true),
            };
            if (left_nullable && plan_contains_self_ref(left, name))
                || (right_nullable && plan_contains_self_ref(right, name))
            {
                return Err(Error::InvalidRecursion(format!(
                    "recursive reference to query \"{name}\" must not appear within an outer join"
                )));
            }
            check_recursive_placement(left, name)?;
            check_recursive_placement(right, name)?;
        }
        // `checkWellFormedSelectStmt`'s contexts: both arms of an `INTERSECT ALL`, both
        // of an `EXCEPT ALL` and the right arm of an `EXCEPT` refuse a self-reference; a
        // `UNION` and a plain `INTERSECT` refuse none, and the arms they leave open are
        // judged further down.
        Plan::SetOp {
            op,
            all,
            left,
            right,
            ..
        } => {
            let (closed, left_closed, right_closed) = match op {
                SetOpKind::Union => ("", false, false),
                SetOpKind::Intersect => ("INTERSECT", *all, *all),
                SetOpKind::Except => ("EXCEPT", *all, true),
            };
            for (arm, arm_closed) in [(left, left_closed), (right, right_closed)] {
                if arm_closed && plan_contains_self_ref(arm, name) {
                    return Err(Error::InvalidRecursion(format!(
                        "recursive reference to query \"{name}\" must not appear within {closed}"
                    )));
                }
                check_recursive_placement(arm, name)?;
            }
        }
        Plan::SubqueryScan {
            subquery: input, ..
        }
        | Plan::Filter { input, .. }
        | Plan::Project { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Distinct { input, .. }
        | Plan::DistinctOn { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Aggregate { input, .. }
        | Plan::Window { input, .. } => check_recursive_placement(input, name)?,
        _ => {}
    }
    Ok(())
}

/// Whether a self-reference to the recursive CTE `name` (a [`Plan::CteScan`] with that
/// name) appears anywhere in `plan`'s **structure** — descending joins, set-op sides,
/// subquery scans, and the row-shaping wrappers. Does not look inside `Expr`s (a
/// sub-link there is counted by `recursive_ref_count` instead). Used to locate a
/// self-reference on a nullable outer-join side ([`check_recursive_placement`]).
fn plan_contains_self_ref(plan: &Plan, name: &str) -> bool {
    match plan {
        Plan::CteScan { name: n, .. } => n == name,
        Plan::Join { left, right, .. } | Plan::SetOp { left, right, .. } => {
            plan_contains_self_ref(left, name) || plan_contains_self_ref(right, name)
        }
        Plan::SubqueryScan {
            subquery: input, ..
        }
        | Plan::Filter { input, .. }
        | Plan::Project { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Distinct { input, .. }
        | Plan::DistinctOn { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Aggregate { input, .. }
        | Plan::Window { input, .. } => plan_contains_self_ref(input, name),
        _ => false,
    }
}

/// Whether a recursive term's lowered plan performs a top-level aggregate or window
/// computation — which Postgres forbids in a recursive term (`42P19`). Descends the
/// row-shaping wrappers (project/filter/sort/distinct/limit) to the query's
/// aggregate/window node; a deeper aggregate inside an *un*correlated subquery isn't
/// reached (Postgres permits those), so this is a conservative, cheap check.
fn recursive_term_has_aggregate(plan: &Plan) -> bool {
    match plan {
        Plan::Aggregate { .. } | Plan::Window { .. } => true,
        Plan::Project { input, .. }
        | Plan::Filter { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input, .. }
        | Plan::DistinctOn { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Limit { input, .. } => recursive_term_has_aggregate(input),
        _ => false,
    }
}

/// The depth of a `Join` over `left` (whose depth is `known`, once measured) and
/// `right`, refused past the thread's limit before the node is built — a join chain
/// grows one level per join without the parser recursing, and every walker over the
/// `FROM` recurses down it ([`crate::depth`]).
fn join_depth(
    known: Option<usize>,
    left: &Plan,
    right: &Plan,
    on: Option<&Expr>,
    nesting: usize,
) -> Result<usize> {
    let left_depth = known.unwrap_or_else(|| crate::depth::plan_depth(left));
    let on_depth = on.map_or(1, crate::depth::expr_depth);
    let depth = 1 + left_depth
        .max(crate::depth::plan_depth(right))
        .max(on_depth);
    crate::depth::check(depth + nesting)?;
    Ok(depth)
}

/// The depth of a set-operation chain as it grows, refused past the thread's limit
/// ([`crate::depth`]).
///
/// `build_set_op` can push a coercion into either side's projection as well as adding
/// the `SetOp` node, so one step deepens the chain by at most two levels over its
/// deeper side. The bound is carried by that arithmetic and re-measured exactly only
/// when it crosses the limit — so a chain the limit admits is measured a logarithmic
/// number of times, not once per operand.
#[derive(Default)]
struct SetOpDepth {
    bound: Option<usize>,
}

impl SetOpDepth {
    fn extend(&mut self, built: &Plan, right_depth: usize, nesting: usize) -> Result<()> {
        let mut bound = self.bound.map_or_else(
            || crate::depth::plan_depth(built),
            |left| left.max(right_depth) + 2,
        );
        if bound + nesting > crate::depth::max_depth() {
            bound = crate::depth::plan_depth(built);
        }
        crate::depth::check(bound + nesting)?;
        self.bound = Some(bound);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_row_count;
    use crate::catalog::Schema;
    use crate::parser::parse;

    fn parses(sql: &str) -> bool {
        parse(sql, &Schema::default()).is_ok()
    }

    #[test]
    fn for_locking_valid_forms_parse() {
        // Every lock strength + `OF` list parses (a host takes the writer for it inside a
        // block), and so does the SQL92 `FOR READ ONLY` — 18.6's other
        // `for_locking_clause` production, a no-op there too.
        for sql in [
            "SELECT * FROM t FOR UPDATE",
            "SELECT * FROM t FOR NO KEY UPDATE",
            "SELECT * FROM t FOR SHARE",
            "SELECT * FROM t FOR KEY SHARE",
            "SELECT * FROM t FOR UPDATE OF t",
            "SELECT * FROM t FOR UPDATE OF t, t",
            "SELECT * FROM t FOR UPDATE FOR SHARE",
            "SELECT * FROM t FOR READ ONLY",
            "SELECT 1 FOR READ ONLY",
        ] {
            assert!(parses(sql), "should parse: {sql}");
        }
    }

    /// The wait policy is the one part of a locking clause pglet cannot honour as a
    /// no-op, so both spellings are refused `0A000` — on every one of the four
    /// `parse_for_locking` call sites (paren-leading, set-op chain, no-FROM leaf,
    /// `FromParts`) and from inside a CTE body.
    ///
    /// The refusal is **deferred** to the statement's end, which is what the last three
    /// cases pin: 18.6 accepts both spellings, so every error it *does* raise on such a
    /// statement outranks a refusal it has no counterpart for. `expected locked` is the
    /// clause's own boundary error, and the trailing-junk case is the statement's.
    #[test]
    fn for_locking_wait_policy_is_refused_after_the_statement_parses() {
        let state = |sql: &str| parse(sql, &Schema::default()).err().map(|e| e.sqlstate());
        for sql in [
            "SELECT * FROM t FOR UPDATE NOWAIT",
            "SELECT * FROM t FOR UPDATE SKIP LOCKED",
            "SELECT * FROM t FOR UPDATE OF t, t NOWAIT",
            "(SELECT 1) FOR UPDATE NOWAIT",
            "SELECT 1 UNION SELECT 2 FOR UPDATE SKIP LOCKED",
            "SELECT 1 FOR UPDATE NOWAIT",
            "WITH c AS (SELECT 1 FOR UPDATE SKIP LOCKED) SELECT * FROM c",
        ] {
            assert_eq!(state(sql), Some("0A000"), "should refuse: {sql}");
        }
        // The first refusal in a statement is the one reported.
        let err = parse(
            "SELECT * FROM t FOR UPDATE NOWAIT FOR SHARE SKIP LOCKED",
            &Schema::default(),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "not implemented: NOWAIT not supported");
        // A syntax error anywhere in the statement still wins.
        assert_eq!(state("SELECT * FROM t FOR UPDATE SKIP"), Some("42601"));
        assert_eq!(
            state("SELECT * FROM t FOR UPDATE SKIP LOCKED zzz"),
            Some("42601")
        );
        assert_eq!(
            state("SELECT * FROM t FOR READ ONLY FOR UPDATE"),
            Some("42601")
        );
    }

    /// `gram.y`'s `for_locking_clause: for_locking_items | FOR READ ONLY` — alternatives,
    /// so `FOR READ ONLY` is the whole clause and cannot stand beside a locking item in
    /// **either** order. The `FOR UPDATE FOR READ ONLY` direction is the one an
    /// is-the-arm-present check cannot see: the arm was there, one loop iteration too
    /// late, and the statement answered rows.
    #[test]
    fn for_read_only_is_the_whole_clause_or_nothing() {
        let state = |sql: &str| parse(sql, &Schema::default()).err().map(|e| e.sqlstate());
        assert!(
            parses("SELECT * FROM t FOR READ ONLY"),
            "alone, it is accepted"
        );
        for sql in [
            "SELECT * FROM t FOR UPDATE FOR READ ONLY",
            "SELECT * FROM t FOR READ ONLY FOR UPDATE",
            "SELECT * FROM t FOR READ ONLY FOR READ ONLY",
            // The wait-policy refusal is deferred, so this reports the syntax error too.
            "SELECT * FROM t FOR UPDATE NOWAIT FOR READ ONLY",
        ] {
            assert_eq!(state(sql), Some("42601"), "should refuse: {sql}");
        }
    }

    /// A `DO $$ … $$` body is parsed by a **sub-parser**, whose deferred state does not
    /// flow out with its plan: `parse` fires `lock_wait_policy_refusal` off the outer
    /// parser only, so the body's refusal was recorded and dropped. The one route out of
    /// eight that was not loud, and the reason `Parser::adopt_deferred_refusals` exists.
    #[test]
    fn a_do_block_bodys_wait_policy_is_refused_too() {
        let state = |sql: &str| parse(sql, &Schema::default()).err().map(|e| e.sqlstate());
        assert_eq!(
            state(
                "DO $$ BEGIN UPDATE m SET a = 1 WHERE a IN (SELECT a FROM t FOR UPDATE NOWAIT);                  END $$"
            ),
            Some("0A000")
        );
        assert_eq!(
            state("DO $$ BEGIN INSERT INTO m SELECT a FROM t FOR UPDATE SKIP LOCKED; END $$"),
            Some("0A000")
        );
        // The control: the same body without a wait policy parses.
        assert!(parses(
            "DO $$ BEGIN INSERT INTO m SELECT a FROM t FOR UPDATE; END $$"
        ));
    }

    #[test]
    fn qualified_table_name_model() {
        // `public.` strips (the implicit schema); the SELECT, INSERT, and DDL paths
        // all route through the same reduction.
        assert!(parses("SELECT * FROM public.t"));
        assert!(parses("INSERT INTO public.t VALUES (1)"));
        assert!(parses("CREATE TABLE public.foo (id INT)"));
        // A bare name is unchanged.
        assert!(parses("SELECT * FROM t"));
    }

    #[test]
    fn qualified_table_name_loud() {
        let state = |sql: &str| parse(sql, &Schema::default()).err().map(|e| e.sqlstate());
        // A non-`public` schema qualifier → 42P01 (relation does not exist).
        assert_eq!(state("SELECT * FROM noschema.t"), Some("42P01"));
        assert_eq!(state("INSERT INTO noschema.t VALUES (1)"), Some("42P01"));
        // 3+ dotted parts → 42601 (improper qualified name).
        assert_eq!(state("SELECT * FROM a.b.c"), Some("42601"));
        assert_eq!(state("SELECT * FROM a.b.c.d"), Some("42601"));
    }

    #[test]
    fn for_locking_garbage_fails_loud() {
        // An unrecognized lock strength or trailing junk is a syntax error (42601),
        // not silently swallowed.
        for sql in [
            "SELECT * FROM t FOR XYZ",
            "SELECT * FROM t FOR UPDATE foobar",
            "SELECT * FROM t FOR NO UPDATE",
            "SELECT * FROM t FOR KEY UPDATE",
        ] {
            let err = parse(sql, &Schema::default()).unwrap_err();
            assert_eq!(err.sqlstate(), "42601", "should fail loud: {sql}");
        }
    }

    #[test]
    fn row_count_plain_integer() {
        assert_eq!(parse_row_count("0").unwrap(), 0);
        assert_eq!(parse_row_count("5").unwrap(), 5);
        assert_eq!(
            parse_row_count("9223372036854775807").unwrap(),
            i64::MAX as usize
        );
    }

    #[test]
    fn row_count_fractional_rounds_half_away_from_zero() {
        // Matches Postgres's numeric→int8 cast (round half away from zero): `2.5` → 3
        // and `0.5` → 1, not the ties-to-even 2 / 0.
        assert_eq!(parse_row_count("1.5").unwrap(), 2);
        assert_eq!(parse_row_count("2.5").unwrap(), 3);
        assert_eq!(parse_row_count("0.5").unwrap(), 1);
        assert_eq!(parse_row_count("2.4").unwrap(), 2);
        assert_eq!(parse_row_count("2.6").unwrap(), 3);
        assert_eq!(parse_row_count("1e2").unwrap(), 100);
    }

    #[test]
    fn row_count_overflow_is_out_of_range() {
        // Past bigint range → 22003 (not a silent unbounded clause).
        assert!(parse_row_count("9223372036854775808").is_err()); // i64::MAX + 1
        assert!(parse_row_count("99999999999999999999").is_err());
        assert!(parse_row_count("1e30").is_err());
    }
}

#[cfg(test)]
mod join_tests {
    use crate::catalog::{Column, Schema, Table};
    use crate::parser::parse;
    use crate::types::TypeOid;

    /// A schema with `t1(num, name)`, `t2(num, value)`, `t3(num, note)` — every
    /// `num` shared, so a USING resolves and the cross-join left is ambiguous.
    fn schema() -> Schema {
        let col = |name: &str, position| Column {
            name: name.to_string(),
            type_oid: TypeOid::Int4,
            nullable: true,
            position,
            default: None,
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            temporal_precision: None,
            identity: None,
        };
        let table = |name: &str, cols: Vec<Column>, oid| Table {
            name: name.to_string(),
            schema: "public".to_string(),
            columns: cols,
            oid,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
        };
        let mut schema = Schema::new();
        schema.create_table(table("t1", vec![col("num", 1), col("name", 2)], 1));
        schema.create_table(table("t2", vec![col("num", 1), col("value", 2)], 2));
        schema.create_table(table("t3", vec![col("num", 1), col("note", 2)], 3));
        schema
    }

    fn ok(sql: &str) -> bool {
        parse(sql, &schema()).is_ok()
    }

    fn state(sql: &str) -> Option<&'static str> {
        parse(sql, &schema()).err().map(|e| e.sqlstate())
    }

    #[test]
    fn cross_comma_and_using_parse() {
        assert!(ok("SELECT * FROM t1 CROSS JOIN t2"));
        assert!(ok("SELECT * FROM t1, t2"));
        assert!(ok("SELECT * FROM t1, t2, t3"));
        assert!(ok("SELECT * FROM t1 JOIN t2 USING (num)"));
        // Chained USING: the second one's `num` is the first join's merged column,
        // not ambiguous.
        assert!(ok(
            "SELECT * FROM t1 JOIN t2 USING (num) JOIN t3 USING (num)"
        ));
    }

    #[test]
    fn natural_join_fails_loud() {
        // NATURAL is out of scope — a loud `0A000`, never a silent degrade.
        assert_eq!(state("SELECT * FROM t1 NATURAL JOIN t2"), Some("0A000"));
    }

    #[test]
    fn using_missing_column_is_42703() {
        // `value` is only on t2 (missing on the left); `nope` is on neither side.
        assert_eq!(
            state("SELECT * FROM t1 JOIN t2 USING (value)"),
            Some("42703")
        );
        assert_eq!(
            state("SELECT * FROM t1 JOIN t2 USING (nope)"),
            Some("42703")
        );
    }

    #[test]
    fn using_ambiguous_left_is_42702() {
        // The left side `t1 CROSS JOIN t2` carries `num` twice → ambiguous, matching
        // Postgres's "common column name ... appears more than once in left table".
        assert_eq!(
            state("SELECT * FROM t1 CROSS JOIN t2 JOIN t3 USING (num)"),
            Some("42702"),
        );
    }

    #[test]
    fn comma_join_using_left_is_scoped() {
        // `JOIN` binds tighter than comma, so USING's left side is just `t2` (not
        // `t1, t2`). `t1` also having `num` no longer makes the left ambiguous —
        // Postgres accepts this, where the old single-accumulator parse wrongly
        // raised 42702.
        assert!(ok("SELECT * FROM t1, t2 JOIN t3 USING (num)"));
        // A `num`-bearing comma item on the *left* is likewise not one of USING's
        // operands (only `t2`/`t3` are), so it stays unambiguous.
        assert!(ok(
            "SELECT * FROM t1, t2 JOIN t3 USING (num) JOIN t2 t2b USING (num)"
        ));
    }
}

#[cfg(test)]
mod recursive_tests {
    use crate::catalog::Schema;
    use crate::parser::parse;

    fn state(sql: &str) -> Option<&'static str> {
        parse(sql, &Schema::default()).err().map(|e| e.sqlstate())
    }

    fn ok(sql: &str) -> bool {
        parse(sql, &Schema::default()).is_ok()
    }

    #[test]
    fn well_formed_recursive_ctes_parse() {
        // The canonical count-up and a self-join tree traversal both plan cleanly.
        assert!(ok(
            "WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM t WHERE n<5) \
             SELECT * FROM t"
        ));
        assert!(ok(
            "WITH RECURSIVE t(n) AS (SELECT 1 UNION SELECT n+1 FROM t WHERE n<5) \
             SELECT sum(n) AS s FROM t"
        ));
    }

    #[test]
    fn recursive_without_self_reference_degrades() {
        // `RECURSIVE` with no self-reference is an ordinary CTE (Postgres degrades
        // silently), not an error — both the bare and the two-term UNION shapes.
        assert!(ok("WITH RECURSIVE t AS (SELECT 1) SELECT * FROM t"));
        assert!(ok(
            "WITH RECURSIVE t AS (SELECT 1 UNION ALL SELECT 2) SELECT * FROM t"
        ));
    }

    #[test]
    fn non_union_combinator_is_invalid_recursion() {
        // A combinator other than UNION between the terms → 42P19.
        assert_eq!(
            state("WITH RECURSIVE t(n) AS (SELECT 1 INTERSECT SELECT n+1 FROM t) SELECT * FROM t"),
            Some("42P19")
        );
        assert_eq!(
            state("WITH RECURSIVE t(n) AS (SELECT 1 EXCEPT SELECT n+1 FROM t) SELECT * FROM t"),
            Some("42P19")
        );
    }

    #[test]
    fn aggregate_in_recursive_term_is_invalid_recursion() {
        // An aggregate in the recursive term → 42P19 (Postgres forbids it).
        assert_eq!(
            state(
                "WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT count(*)::int FROM t) \
                 SELECT * FROM t"
            ),
            Some("42P19")
        );
    }

    #[test]
    fn type_mismatch_is_datatype_mismatch() {
        // The non-recursive term is authoritative: an int seed with a numeric-widening
        // recursive term → 42804 ("... type integer in non-recursive term but numeric
        // overall"), while the reverse (numeric seed, int recursive) is well-typed.
        assert_eq!(
            state(
                "WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT n+1::numeric FROM t WHERE n<5) \
                 SELECT * FROM t"
            ),
            Some("42804")
        );
        assert!(ok(
            "WITH RECURSIVE t(n) AS (SELECT 1::numeric UNION ALL SELECT n+1 FROM t WHERE n<5) \
             SELECT * FROM t"
        ));
    }

    #[test]
    fn column_count_mismatch_is_syntax_error() {
        // Unequal column counts between the terms → 42601.
        assert_eq!(
            state(
                "WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT n, n+1 FROM t) SELECT * FROM t"
            ),
            Some("42601")
        );
    }

    #[test]
    fn too_many_column_aliases_is_invalid_column_reference() {
        // More aliases than the seed produces columns → 42P10.
        assert_eq!(
            state(
                "WITH RECURSIVE t(n, m) AS (SELECT 1 UNION ALL SELECT n+1 FROM t WHERE n<3) \
                 SELECT * FROM t"
            ),
            Some("42P10")
        );
    }

    /// Build a schema by running each DDL statement, so a recursive-CTE name can be
    /// tested against a same-named (or sibling) real table.
    fn schema_with(ddls: &[&str]) -> Schema {
        let mut schema = Schema::new();
        for sql in ddls {
            let plan = parse(sql, &schema).unwrap().remove(0);
            crate::executor::execute(&plan, &mut schema).unwrap();
        }
        schema
    }

    fn state_in(schema: &Schema, sql: &str) -> Option<&'static str> {
        parse(sql, schema).err().map(|e| e.sqlstate())
    }

    #[test]
    fn double_self_reference_is_invalid_recursion() {
        // The self-reference appearing twice (a self-join to the recursive name) → 42P19
        // — without this, pglet silently returned rows Postgres rejects.
        assert_eq!(
            state(
                "WITH RECURSIVE t(n) AS (\
                   SELECT 1 UNION ALL \
                   SELECT (t1.n + t2.n)::int FROM t t1, t t2 WHERE t1.n < 4\
                 ) SELECT n FROM t"
            ),
            Some("42P19")
        );
    }

    #[test]
    fn seed_shadowing_a_real_table_is_invalid_recursion() {
        // The CTE name shadows a same-named real table throughout the body, so a
        // self-reference in the non-recursive term → 42P19 (not a read of the real
        // table). Without the placeholder-before-seed shadow, pglet silently read the
        // base table's rows.
        let schema = schema_with(&["CREATE TABLE t (n INT)"]);
        assert_eq!(
            state_in(
                &schema,
                "WITH RECURSIVE t(n) AS (\
                   SELECT n FROM t UNION ALL SELECT n + 1 FROM t WHERE n < 103\
                 ) SELECT n FROM t"
            ),
            Some("42P19")
        );
    }

    #[test]
    fn seed_referencing_a_different_real_table_is_unaffected() {
        // A seed that references some *other* real table (not the CTE name) is a
        // normal recursive CTE — the shadow only covers the CTE's own name.
        let schema = schema_with(&["CREATE TABLE base (x INT)"]);
        assert!(
            parse(
                "WITH RECURSIVE t(n) AS (\
                   SELECT x FROM base UNION ALL SELECT n + 1 FROM t WHERE n < 5\
                 ) SELECT n FROM t",
                &schema,
            )
            .is_ok()
        );
    }

    #[test]
    fn self_ref_on_nullable_outer_join_side_is_invalid_recursion() {
        // A self-reference on the nullable (right) side of a LEFT JOIN → 42P19. Under
        // UNION this can terminate with rows Postgres rejects, so it must fail loud.
        let schema = schema_with(&["CREATE TABLE base (x INT)"]);
        assert_eq!(
            state_in(
                &schema,
                "WITH RECURSIVE t(n) AS (\
                   SELECT 1 UNION \
                   SELECT base.x FROM base LEFT JOIN t ON base.x = t.n WHERE base.x < 5\
                 ) SELECT n FROM t"
            ),
            Some("42P19")
        );
    }

    #[test]
    fn self_ref_in_from_derived_table_is_allowed() {
        // Postgres permits a self-reference inside a plain FROM-clause derived table
        // (no enclosing outer join); it must not be rejected.
        assert!(ok("WITH RECURSIVE t(n) AS (\
               SELECT 1 UNION ALL SELECT n + 1 FROM (SELECT n FROM t) sub WHERE n < 5\
             ) SELECT n FROM t"));
    }

    #[test]
    fn inner_join_self_ref_on_preserved_side_is_allowed() {
        // A self-reference on the preserved (left) side of a LEFT JOIN, and in an INNER
        // join, are both fine — only the nullable outer-join side is illegal.
        let schema = schema_with(&["CREATE TABLE base (x INT)"]);
        assert!(
            parse(
                "WITH RECURSIVE t(n) AS (\
                   SELECT 1 UNION ALL \
                   SELECT t.n + 1 FROM t LEFT JOIN base ON base.x = t.n WHERE t.n < 3\
                 ) SELECT n FROM t",
                &schema,
            )
            .is_ok()
        );
    }
}
