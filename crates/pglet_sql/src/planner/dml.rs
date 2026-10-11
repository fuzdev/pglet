//! DML (Data Manipulation Language) lowering: INSERT, UPDATE, DELETE.
//!
//! The native parser (`parser::dml`) parses each statement into the raw lowered
//! inputs (`InsertRowSource`, assignment/filter `Expr`s) and calls these shared
//! `lower_*` helpers, which apply column mapping, `DEFAULT`-fill, per-column
//! coercion, and WHERE type-resolution against the catalog.

use std::collections::HashMap;

use crate::catalog::{Column, IdentityGeneration, Schema, Table};
use crate::executor::{EvalEnv, eval_expr_env, validate_column};
use crate::planner::ResolvedType;
use crate::{CatalogError, Error, Result};

use super::coercion::{
    AssignmentSource, ColumnScope, assignment_source_type, check_assignment, check_boolean_clause,
    coerce_to_type, column_type_map_for, resolve_expr_types_gated, resolve_expr_types_no_columns,
};
use super::correlation::resolve_correlated_subplans;
use super::expr::{BoolContext, Expr, Literal};
use super::legality::assignment_legality;
use super::plan::{
    CertainTypes, InsertRows, OnConflictAction, OnConflictClause, Overriding, Plan,
    RelationColumns, ResolvedTargets, TypeScope, expr_relations_resolvable,
    on_conflict_action_relations, plan_relations_resolvable, returning_relations,
};
use super::query::{dml_target_probe, push_plan_relations, sql99_target_index};

/// The row source of an INSERT, already lowered to planner IR — the parser-agnostic
/// input to [`lower_insert`].
pub enum InsertRowSource {
    /// `VALUES` tuples in *written* order (one inner `Vec` per tuple), pre-column-
    /// mapping. Each value is a lowered `Expr`; a `DEFAULT` keyword is `Expr::Default`.
    ///
    /// A tuple shorter than the table is widened with `Expr::Default` by
    /// [`lower_insert_rows`], so `INSERT … DEFAULT VALUES` is this arm carrying **one
    /// empty tuple** rather than a variant of its own — it lowers to the identical
    /// `Plan::Insert` its `VALUES (DEFAULT, …)` twin does.
    Values(Vec<Vec<Expr>>),
    /// An `INSERT … SELECT` source sub-plan (boxed — a `Plan` dwarfs the `Values` arm).
    Select(Box<Plan>),
}

/// Lower a parsed INSERT into a [`Plan::Insert`].
///
/// The native parser parses the row source and column list and calls this shared
/// lowering, so column mapping, `DEFAULT`-fill, per-column coercion, and
/// `INSERT … SELECT` target-index mapping have one implementation (the source/values
/// exprs and the column list are the only parser-specific input). `columns` is the
/// explicit `(col, …)` list (empty = positional). Both row sources feed the same
/// per-row insert loop downstream (DEFAULT-fill, constraints, ON CONFLICT, RETURNING).
///
/// # Clause order
///
/// The three clauses are lowered in the order `analyze.c`'s `transformInsertStmt`
/// transforms them — **row source, then `transformOnConflictClause`, then
/// `transformReturningClause`** — so when two of them are ill-typed the one Postgres
/// reports is the one raised here. It is the same source-order rule the per-node walk
/// enforces inside an expression, one level up, and it is why `ON CONFLICT` is lowered
/// here rather than where it is parsed.
///
/// The row source's own two halves keep their order too: the **target column list**
/// resolves before the cell expressions, because Postgres's `checkInsertTargets` runs
/// ahead of `transformInsertRow` (`INSERT INTO t (nosuchcol) VALUES (<ill-typed>)` is
/// the undefined column, not the cell). That mapping is this function's, which is why
/// the cells cannot simply be resolved where they are parsed either. The `VALUES` rows
/// are then lowered one at a time in list order, each whole before the next is read —
/// its cells, its length against the first row's, its length against the target list,
/// each cell's assignment — as `transformInsertStmt`'s loop over the lists does, so which
/// of two rows' plan-time errors wins is decided by the rows' order
/// ([`lower_insert_rows`]).
///
/// Ahead of all of it sits [`lowers_clauses`]: with no target relation nothing is
/// lowered and the executor's `42P01` stands. **Behind all of it** sits one gate that is
/// not a clause's at all — the `ON CONFLICT DO UPDATE` `SET` list's duplicate-target
/// refusal, which stands in for a check 18.6 makes in the rewriter and so runs after
/// every clause here; its own comment at the bottom of this function carries the
/// boundaries.
///
/// # Guards outside that order
///
/// The clause order above governs the **type resolution and operand gating** of the
/// three clauses, and nothing else. The **identity guards** sit outside it, and fire
/// earlier than the statement's real error, because Postgres raises them later than pglet
/// does. They are a known divergence, not a description of the order:
///
/// - The identity guards — [`apply_identity_rules`] (the `VALUES` rows, once every row
///   is lowered),
///   [`enforce_identity_select`] (an `INSERT … SELECT` target list) and
///   [`lower_on_conflict`]'s `DO UPDATE` check. Each runs *inside* its clause's
///   lowering, so it beats any error in a clause lowered after it. Postgres raises all
///   of them in the **rewriter** (`rewriteTargetListIU`, `rewrite/rewriteHandler.c`),
///   which runs after *all* of parse analysis — so there any parse-analysis error, in
///   any clause, outranks them. Measured: `INSERT INTO t (id, a) VALUES (1, 2)
///   RETURNING a LIKE 'z%'` on a `GENERATED ALWAYS` `id` is the `428C9` identity guard
///   here and the `RETURNING` clause's `42883` in Postgres. They do *not* outrank the
///   errors their **own** clause gates at plan time: the `VALUES` and `INSERT … SELECT`
///   guards run after that clause's values are gated, so a cell or source column pglet
///   gates at plan time wins on both sides — an assignment cast, which
///   `insert/assignment_cast_precedes_the_identity_guard.sql` pins, or
///   `COALESCE(1, 'x'::text)`; one it refuses only at execution — an unknown literal's
///   conversion, a bare name with no `FROM` to bind against, an operator pglet resolves
///   at eval — loses to the guard here and wins there. Nor do they outrank
///   an unresolvable **source relation**, which 18.6 reports from the range table ahead
///   of parse analysis and pglet raises at execution: `enforce_identity_select` takes
///   the same `plan_relations_resolvable` skip the assignment gate does
///   (`insert/identity_select_guard_yields_to_a_missing_source.sql`). [`lower_update`]'s
///   twin used to be listed here and no longer belongs: it runs last there now, after
///   every clause — which is *not* the same as matching the rewriter, and its own doc
///   says why.
///
/// # Errors
///
/// `CatalogError::ColumnNotFound` (`42703`) for an unknown target column,
/// `CatalogError::DuplicateColumnName` (`42701`) for a target column list naming one
/// twice ([`resolve_insert_columns`], reached through [`lower_insert_rows`]),
/// [`Error::Syntax`] (`42601`) for a row whose length the target list or the first
/// `VALUES` row refuses ([`row_length_refusal`]) and for an
/// `ON CONFLICT DO UPDATE` `SET` list assigning one column twice
/// ([`refuse_duplicate_assignments`]), and whatever the per-clause
/// lowering raises ([`lower_on_conflict`], [`lower_returning`]).
pub fn lower_insert(
    table: String,
    columns: &[String],
    source: InsertRowSource,
    returning: Vec<Expr>,
    on_conflict: Option<OnConflictClause>,
    overriding: Overriding,
    schema: &Schema,
) -> Result<Plan> {
    let lower = lowers_clauses(schema, &table);
    let rows = lower_insert_rows(&table, columns, source, overriding, schema)?;
    let on_conflict = on_conflict
        .map(|clause| {
            if lower {
                lower_on_conflict(clause, &table, schema)
            } else {
                Ok(clause)
            }
        })
        .transpose()?;
    // Expand `RETURNING *` / `RETURNING *, <expr>` to the target columns at plan
    // time, mirroring UPDATE so the executor never sees a bare `*` mixed with other
    // items. (A sole `*` is expanded too — the executor's `project_returning` sole-`*`
    // fast path then just sees explicit columns, which is equivalent.)
    //
    // The namespace it resolves against is [`insert_returning_relations`], which is where
    // the two `None`s — no alias, no source relation — are justified.
    let returning = if returning.is_empty() || !lower {
        returning
    } else {
        lower_returning(
            returning,
            &insert_returning_relations(&table, schema),
            schema,
        )?
    };
    let mut plan = Plan::Insert {
        table,
        rows,
        returning,
        on_conflict,
        overriding,
    };
    // A subquery inside `RETURNING` or inside the `ON CONFLICT` action was lowered
    // before either clause's scope existed, so its own binding gate stayed open
    // (`scope_is_complete == false` — an expression subquery encloses it). This is the
    // re-walk that closes it, and it is the same pass `lower_update` / `lower_delete`
    // run. Without it nobody judged those levels at all: over an **empty** `et`,
    // `INSERT INTO it VALUES (1) RETURNING (SELECT b FROM et ORDER BY zzz)` wrote its
    // row and answered NULL where 18.6 is `42703`, and the no-conflict spelling of
    // `ON CONFLICT (id) DO UPDATE SET v = (SELECT b FROM et ORDER BY zzz)` inserted —
    // the eval-time resolver never fires when the inner relation has no row, and the
    // action is never evaluated when nothing conflicts.
    //
    // Gated on `lower` for the reason [`lowers_clauses`] states: with no target
    // relation the clause namespaces are not derivable (an absent target has no
    // columns, and a closed scope would read that as `42703` for a column of a table
    // that is not there), and the executor's `42P01` must stand.
    //
    // A DML statement is always its own top level — there is no expression subquery it
    // could sit inside — so its chain is complete and the nested levels' gates may close.
    if lower {
        resolve_correlated_subplans(&mut plan, schema, true)?;
        // The `ON CONFLICT DO UPDATE SET` list is a set, and 18.6 says so from the
        // **rewriter**: `RewriteQuery` hands `onConflictSet` to the same
        // `rewriteTargetListIU` an `UPDATE`'s target list goes through, unconditionally
        // for a `DO UPDATE` action. So it fires whether or not a row conflicts —
        // measured, `INSERT INTO u VALUES (5, 1) ON CONFLICT (a) DO UPDATE SET b = 2,
        // b = 3` against a free key is `42601` on 18.6 and **inserted** here, which is
        // why the check cannot sit on the conflict path.
        //
        // Being a rewriter check it runs after every clause of this statement is
        // transformed, which is why it sits below `lower_returning` and below the
        // correlated re-walk: `… DO UPDATE SET b = 1, b = 2 RETURNING nosuch` is `42703`
        // on 18.6, as is the `RETURNING (SELECT b FROM et ORDER BY zzz)` spelling the
        // re-walk closes, and both were already that here. It also sits below the row
        // source's own lowering, which is where 18.6 rewrites the `INSERT`'s own target
        // list — before this one, on a separate `rewriteTargetListIU` call — so
        // `INSERT INTO g (id, b) VALUES (1, 2) ON CONFLICT (id) DO UPDATE SET b = 1,
        // b = 2` is the identity column's `428C9` on both engines.
        //
        // **Below the row source's *lowering* is not below the row source's *verdicts***,
        // and the identity guard is the misleading sample of that. A cell's implicit
        // assignment conversion and a `VALUES` cell's bare name are reached only per row,
        // at execution — below everything here — so this gate answers ahead of them where
        // 18.6 settles both in parse analysis: `INSERT INTO u VALUES (5, 'notanint', true)
        // ON CONFLICT (a) DO UPDATE SET b = 1, b = 2` is `22P02` on 18.6 and `42601` here,
        // the `nosuchcol` spelling of that cell `42703` there and `42601` here, at a free
        // key and a conflicting one alike, and the pre-gate binary agreed with 18.6 on
        // every one of them. Recorded, with the measurement that refuses the obvious
        // repair — moving the gate below those verdicts loses the zero-row source, which
        // 18.6 and this placement both refuse — in `docs/names.md` § What the refusals do
        // not carry. And before the executor's
        // arbiter inference, which 18.6 reaches later still, in the planner: `ON
        // CONFLICT (c) DO UPDATE SET b = 1, b = 2` over a `c` with no unique index is
        // `42601` there and was pglet's `42P10`.
        //
        // What it does *not* outrank here and does on 18.6 is the action's **own**
        // identity guard, `lower_on_conflict`'s pass 4 — `rewriteTargetListIU` merges
        // every entry before it looks at an identity column, so 18.6 answers
        // `DO UPDATE SET id = 1, id = 2` with `42601` where this answers `428C9`. That
        // is the identity guards' known position, not this gate's: the guard runs inside
        // its clause's lowering, which is also why it beats a later clause's error.
        if let Plan::Insert {
            on_conflict: Some(clause),
            ..
        } = &plan
            && let OnConflictAction::DoUpdate { assignments, .. } = &clause.action
        {
            refuse_duplicate_assignments(assignments)?;
        }
    }
    Ok(plan)
}

/// The `RETURNING` namespace of an `INSERT`: the target relation under **its own name**,
/// with no alias and no source relation.
///
/// The one spelling of those two `None`s. Its readers are the clause's own lowering
/// ([`lower_returning`], via [`lower_insert`]), the correlated re-walk that hands the
/// clause's nested levels their outer scope (`super::correlation::walk_level`'s
/// `Plan::Insert` arm), and the prepared-statement parameter walk
/// (`super::param_types`'s `Inference::walk_returning`, from its `Plan::Insert` arm) —
/// read that as the readers there are, not as a count. The third arrived exactly the way
/// this doc warned one would: it spelled the namespace itself — the argument triple
/// `(t, t, None)` handed to a shared `RETURNING` walker, which then built
/// `returning_relations` from it — and so was invisible to a search for this function's
/// name. None of them is free to spell it differently — they must read the
/// same namespace or the outer items, the subqueries inside them, and the `$N` types
/// reported for them disagree about what the clause can name.
///
/// **Both `None`s are measured, not defaults.** The alias is `None` because this parser
/// rejects `INSERT INTO t AS p` (`42601`) — a gap, not Postgres's grammar, so closing it
/// means revisiting this function *and*
/// [`on_conflict_action_relations`](super::plan::on_conflict_action_relations), which
/// names the target the same way for the same reason. The source is `None` because an `INSERT … SELECT`'s
/// source is genuinely **not** in Postgres's `RETURNING` namespace: on 18.6
/// `INSERT INTO amb_t SELECT … FROM amb_s s RETURNING s.v` is `42P01` and
/// `RETURNING k` is `42703`.
pub(super) fn insert_returning_relations(table: &str, schema: &Schema) -> RelationColumns {
    returning_relations(table, table, None, schema)
}

/// Whether a DML statement's clauses should be resolved and gated at all — false when
/// the schema has no `table`.
///
/// Postgres resolves the target relation **first** (`setTargetTable`, ahead of every
/// `transform*Clause` call), so a statement naming a relation that does not exist is
/// `42P01` no matter what else is wrong with it. pglet raises that at execution, which
/// means a plan-time gate firing on an ill-typed clause pre-empts it — `UPDATE
/// nosuchtable SET t = COALESCE(1, 'x'::TEXT)` reported the `COALESCE`'s `42804`
/// instead. Declining to lower is the fix and it costs nothing: a statement with no
/// target relation cannot execute, so the type error it would have raised is one
/// Postgres never raises either.
///
/// `lower_insert_rows` already had this rule for the `VALUES` cells alone (it needs the
/// table to map columns at all); this is that rule stated once for every clause of
/// every DML statement.
pub(super) fn lowers_clauses(schema: &Schema, table: &str) -> bool {
    schema.get_table(table).is_some()
}

/// Lower an INSERT's row source into the executor-facing [`InsertRows`] — column
/// mapping, `DEFAULT`-fill, per-cell resolution/gating, and the identity rules.
///
/// The first of [`lower_insert`]'s three clauses; see its § Clause order.
fn lower_insert_rows(
    table: &str,
    columns: &[String],
    source: InsertRowSource,
    overriding: Overriding,
    schema: &Schema,
) -> Result<InsertRows> {
    match source {
        InsertRowSource::Select(source_plan) => {
            let target_columns =
                insert_select_target_indices(columns, table, schema, &source_plan)?;
            if let Some(table_def) = schema.get_table(table) {
                // The assignment cast runs **before** the identity guard, because 18.6
                // raises the `42804` in parse analysis and the `428C9` in the rewriter:
                // `INSERT INTO g (id, b) SELECT 5, 1` over a `GENERATED ALWAYS` `id` and
                // a `BOOL` `b` reports the column `b` there, in either column order.
                check_insert_select_assignments(&source_plan, &target_columns, table_def, schema)?;
                // And both yield to an unresolvable source relation, which 18.6 reports
                // from the range table ahead of parse analysis and pglet raises at
                // execution: `INSERT INTO g (id, v) SELECT 5, 1 FROM nosuchrel` is
                // `42P01` there and was this guard's `428C9` here.
                if plan_relations_resolvable(&source_plan, schema) {
                    enforce_identity_select(table_def, table, &target_columns, overriding)?;
                }
            }
            Ok(InsertRows::Select {
                source: source_plan,
                columns: target_columns,
            })
        }
        InsertRowSource::Values(rows) => {
            // Without the target table we can't honor the column list or fill
            // defaults; keep the values positionally and let the executor raise
            // table-not-found.
            let Some(table_def) = schema.get_table(table) else {
                return Ok(InsertRows::Values {
                    rows,
                    written: Vec::new(),
                });
            };

            // Map each provided value position to a table-column index. With an
            // explicit column list, map by name; without one, positional.
            let named = !columns.is_empty();
            let targets: Vec<usize> = if named {
                resolve_insert_columns(table_def, table, columns)?
            } else {
                (0..table_def.columns.len()).collect()
            };

            // The row whose length the statement refuses, if any. The lengths are known
            // before any cell is lowered, and a refusal certain from the start is what
            // licenses the probe below; with none, no row pays for it.
            let refusal = row_length_refusal(&rows, targets.len(), named);

            let mut values = Vec::with_capacity(rows.len());
            for (index, mut row) in rows.into_iter().enumerate() {
                let probe = refusal.as_ref().is_some_and(|(at, _)| index <= *at);
                // 18.6's `transformExpressionList`: every cell of the row is resolved on
                // its own, before the row's length is compared with anything.
                for cell in &mut row {
                    resolve_values_cell(cell, probe, schema)?;
                }
                if let Some((at, refusal)) = &refusal
                    && index == *at
                {
                    return Err(Error::syntax_sentence(*refusal));
                }
                values.push(lower_values_row(
                    row, &targets, table, table_def, probe, schema,
                )?);
            }
            // The identity rules (`OVERRIDING`, the `428C9` GENERATED-ALWAYS guard) are
            // the rewriter's in 18.6, so they read the rows after every row's own
            // parse-analysis checks — a later row's length included.
            for full in &mut values {
                apply_identity_rules(table_def, table, full, overriding)?;
            }

            // A column list in the table's own order says nothing the rows do not, and
            // is not recorded — which keeps `DEFAULT VALUES` and its spelled-out twins
            // one plan.
            Ok(InsertRows::Values {
                rows: values,
                written: if targets.windows(2).all(|pair| pair[0] < pair[1]) {
                    Vec::new()
                } else {
                    targets
                },
            })
        }
    }
}

/// 18.6's sentence for a `VALUES` row whose length differs from the first row's.
const VALUES_LISTS_LENGTH: &str = "VALUES lists must all be the same length";
/// 18.6's sentence for a row source longer than the target list.
const MORE_EXPRESSIONS: &str = "INSERT has more expressions than target columns";
/// 18.6's sentence for a row source shorter than an explicit target column list.
const MORE_TARGET_COLUMNS: &str = "INSERT has more target columns than expressions";

/// The first `VALUES` row an `INSERT` refuses for its length, and 18.6's sentence for it.
///
/// Row by row in list order, each row asked what 18.6's `transformInsertStmt` asks it:
/// its length against the first row's (`VALUES lists must all be the same length`), then
/// against the target list (`transformInsertRow`: longer than the targets is refused
/// either way, shorter only under an explicit column list — without one, the missing
/// trailing columns take their defaults). All three are `42601`, raised with no `P`: 18.6
/// points it at a cell, and a lowered row carries no spans.
///
/// Only *which row* is decided here. What outranks the refusal is every earlier row's
/// whole transformation and the refused row's own cells, which [`lower_insert_rows`]
/// reaches by lowering the rows in order up to it — and, for two verdicts it otherwise
/// reaches only at execution, by the probes ([`probe_bare_names`],
/// [`probe_literal_input`]). A verdict outside their shapes still loses to the refusal.
fn row_length_refusal(
    rows: &[Vec<Expr>],
    targets: usize,
    named: bool,
) -> Option<(usize, &'static str)> {
    let first = rows.first()?.len();
    rows.iter().enumerate().find_map(|(index, row)| {
        let sentence = if row.len() != first {
            VALUES_LISTS_LENGTH
        } else if row.len() > targets {
            MORE_EXPRESSIONS
        } else if named && row.len() < targets {
            MORE_TARGET_COLUMNS
        } else {
            return None;
        };
        Some((index, sentence))
    })
}

/// Resolve one `VALUES` cell on its own — 18.6's `transformExpressionList` step, which
/// runs over the whole row before the row's length is compared with anything.
///
/// The value expression is resolved and gated before any column coercion wraps it:
/// `coerce_to_type` only says what the value must become, never whether it is well-typed
/// on its own, so `VALUES (COALESCE(1, 'x'::text))` was accepted with no common type. A
/// `VALUES` list has no FROM, so this is the no-columns resolution — gating the
/// *unresolved* expression instead rejected `VALUES (1 = '1')`, whose unknown literal had
/// not yet adopted its sibling's type.
///
/// Under `probe` it also raises the cell's **bare column reference**, which 18.6 refuses
/// in that step (`42703`) and pglet otherwise refuses only as the executor evaluates the
/// row — see [`probe_bare_names`].
fn resolve_values_cell(cell: &mut Expr, probe: bool, schema: &Schema) -> Result<()> {
    if matches!(cell, Expr::Default) {
        return Ok(());
    }
    let resolved = resolve_expr_types_no_columns(std::mem::replace(cell, Expr::Default), schema)?;
    if probe {
        probe_bare_names(&resolved, schema)?;
    }
    *cell = resolved;
    Ok(())
}

/// Lower one `VALUES` row, its cells already resolved, into a full-width, table-ordered
/// row: each provided cell gated by the assignment rule and wrapped in its column's
/// coercion, in the row's written order; every column the row does not name
/// `Expr::Default` (the executor resolves the catalog default).
///
/// Under `probe` a bare string literal is also put through its column's input
/// conversion here, where 18.6's `transformAssignedExpr` runs it — see
/// [`probe_literal_input`].
fn lower_values_row(
    row: Vec<Expr>,
    targets: &[usize],
    table: &str,
    table_def: &Table,
    probe: bool,
    schema: &Schema,
) -> Result<Vec<Expr>> {
    let mut full = vec![Expr::Default; table_def.columns.len()];
    for (expr, &idx) in row.into_iter().zip(targets) {
        // Leave a DEFAULT raw so the executor resolves the catalog default; coerce only
        // real provided values to the column type.
        if matches!(expr, Expr::Default) {
            continue;
        }
        // The assignment cast decides whether the value may reach this column *at
        // all* — before `coerce_to_type` wraps it in an explicit cast that would
        // perform the conversion silently. `TypeScope::NONE` is the scope because a
        // `VALUES` row has no FROM clause to resolve a column reference against, the
        // same scope `Plan::Values::output_columns` passes.
        let column = &table_def.columns[idx];
        check_assignment(
            &expr,
            &column.name,
            column.type_oid,
            schema,
            TypeScope::NONE,
        )?;
        let unknown_literal = matches!(expr, Expr::Literal(Literal::String(_)));
        let lowered = coerce_to_type(expr, column.type_oid);
        if probe && unknown_literal {
            probe_literal_input(&lowered, table, column, schema)?;
        }
        full[idx] = lowered;
    }
    Ok(full)
}

/// Raise the first bare column reference in a `VALUES` cell, as the executor would.
///
/// A `VALUES` list has no relation in scope, so 18.6 refuses any column reference in a
/// cell while it transforms the row (`42703`; `42P01` for a qualified one). Here the cell
/// resolves at plan time and the reference is refused only when the executor evaluates
/// it, row by row — so where a later row's length is refused, the reference would lose
/// to it. Called only when that refusal is certain; each reference is evaluated alone,
/// against no scope, so the answer is the executor's own and nothing else in the cell
/// runs. A reference inside a subquery is the subquery's, and is not reached.
fn probe_bare_names(expr: &Expr, schema: &Schema) -> Result<()> {
    if let Expr::Column { name, .. } = expr {
        // `count(*)`'s synthesized star is not a column reference.
        if name == "*" {
            return Ok(());
        }
        return eval_expr_env(expr, EvalEnv::EMPTY.with_schema(schema)).map(drop);
    }
    let mut result = Ok(());
    expr.for_each_child(|child| {
        if result.is_ok() {
            result = probe_bare_names(child, schema);
        }
    });
    result
}

/// Put a bare string literal through its column's input conversion, as the executor
/// would, keeping only the verdict.
///
/// 18.6 runs the target type's input function over an `unknown` literal while it
/// transforms the row (`coerce_type`, with no length or precision modifier — those are
/// applied later, in the planner; `interval` alone takes its fields and precision
/// there), and an enum's input function is where a label the enum does not hold is
/// refused. Here both happen as the executor stores the row, so where a later row's
/// length is refused they would lose to it. Called only when that refusal is certain;
/// the value is discarded — the executor converts the cell again. The probe passes no
/// modifier, `interval`'s included, so an `interval(p)` literal whose rounding 18.6
/// refuses at input (`22008`) still loses to the refusal: `EDGES.md` row 226's residual.
fn probe_literal_input(
    lowered: &Expr,
    table: &str,
    column: &Column,
    schema: &Schema,
) -> Result<()> {
    let value = eval_expr_env(lowered, EvalEnv::EMPTY.with_schema(schema))?;
    validate_column(table, column, &value, &schema.enums)
}

/// The `ON CONFLICT DO UPDATE` action's namespace as a [`ColumnScope`] — the target
/// relation under its own name plus the `EXCLUDED` pseudo-relation carrying the target's
/// shape, **closed**.
///
/// Postgres builds one namespace for the whole action (`excluded` is set up before either
/// half is transformed), so this is one object. Its readers are [`lower_on_conflict`],
/// which gates the `SET` right-hand sides and the action's `WHERE` against it, and
/// `super::correlation::walk_level`'s `Plan::Insert` arm, which hands it to every nested
/// level either of those two positions embeds.
///
/// **It has a twin in the other namespace type**, and the pairing is the only thing that
/// says so: [`on_conflict_action_relations`](super::plan::on_conflict_action_relations)
/// builds the same target-plus-`excluded` namespace as a
/// [`RelationColumns`](super::plan::RelationColumns) for the prepared-statement
/// parameter walk, which needs ordered `ColumnDesc`s where this needs a name→type map
/// with claimant counting. A change to what the action can name moves both.
///
/// **What licenses the close is [`lowers_clauses`], not a check inside here.** A target
/// the catalog does not hold has an empty `column_type_map_for`, which a closed scope
/// would read as "this relation has no columns" — `42703` for a column of a table that is
/// not there, ahead of the execute-time `42P01` for the table itself, which is what 18.6
/// answers. Both callers test that predicate before reaching this.
pub(super) fn on_conflict_action_scope(table: &str, schema: &Schema) -> ColumnScope {
    let column_types = column_type_map_for(schema, table);
    let mut scope = ColumnScope::single(table.to_string(), column_types.clone());
    // the conflicting row is read from the heap
    if let Some(def) = schema.get_table(table) {
        scope.mark_stored(def);
    }
    scope.push_excluded(column_types);
    scope.closed()
}

/// Lower an `ON CONFLICT` clause's `DO UPDATE` action: the ambiguity and identity
/// guards, then the same resolve-and-gate every other DML clause gets.
///
/// The second of [`lower_insert`]'s three clauses. It runs **here** rather than in the
/// parser because `transformOnConflictClause` runs after the row source: gating the
/// action at parse time (before the `VALUES` cells are lowered at all) made an
/// ill-typed `SET` outrank an ill-typed cell, where Postgres reports the cell.
///
/// A `DO NOTHING` (or a target-less clause) has nothing to lower and passes through.
/// The **arbiter target** is deliberately untouched: pglet never resolves or gates
/// `ON CONFLICT (<expr>)`, and doing so would change how an arbiter matches an
/// expression index.
///
/// The scope both halves of the action resolve against is
/// [`on_conflict_action_scope`] — the target relation plus the `EXCLUDED`
/// pseudo-relation, which carries the target's shape, so `t.col` and `EXCLUDED.col` both
/// resolve, in the `SET` right-hand sides exactly as in the `WHERE`. That namespace is
/// **complete by construction**, so it closes; the state that would make closing wrong
/// (a target the catalog does not hold) is unreachable because [`lower_insert`] skips
/// every clause lowering when the target is unknown. The guards for **that** gate
/// (`insert/on_conflict_missing_target_stays_a_table_error.sql` and
/// `a_missing_on_conflict_target_reports_the_table_and_not_its_columns`) are written in
/// the spellings that would move if it went — a *bare* name, or one qualified by the
/// target or by `excluded`, never a bogus qualifier, which is `42P01` under a closed
/// scope too.
///
/// **Closing it is what makes the action's name errors row-independent**, and until it
/// did, four shapes wrote their row: an INSERT that hits **no conflict** never evaluates
/// the action, so `SET nosuchcol = 'z'`, `SET v = nosuchcol`, `SET v = 'z' WHERE
/// nosuchcol > 0` and `SET v = excluded.nosuchcol` each inserted where 18.6 raises
/// `42703` — the same statement erroring or writing depending on whether the row already
/// existed. The bogus-qualifier spellings (`SET v = zzz.v`, `SET v = "EXCLUDED".v`)
/// wrote too, and on a *conflicting* row they stored the **excluded** value; the
/// executor's own outer-frame rule refuses those at eval time, and this refuses them
/// before any row is looked at. Both are wanted: neither alone covers both row counts.
///
/// **The checks run in Postgres's order, which is not the order they are written
/// in.** `transformUpdateTargetList` transforms *every* right-hand side before assigning
/// any of them to a column, then walks the targets resolving each name and coercing that
/// assignment in the same iteration; the action's `WHERE` is transformed after all of
/// that, and the identity guard is a rewriter check that runs after all of parse
/// analysis. Measured on 18.6, each pair discriminating: `SET nosuchcol = zzz.v` is
/// `42P01` (a right-hand side beats its own target) and `SET nosuchcol = 'y', v = zzz.v`
/// is `42P01` too (the *second* right-hand side beats the *first* target — which no
/// per-assignment loop can produce); over a `BOOL` `b`, `SET b = 1, nosuch = 1` is
/// `42804` and `SET nosuch = 1, b = 1` is `42703` (so the name and the **type** verdict
/// interleave per target rather than running as two whole-list passes), while
/// `SET b = 1, i = nosuchcol` is `42703` (every right-hand side is still transformed
/// ahead of both); `SET nosuchcol = 'z' WHERE zzz.v = 'b'` is `42703` (a target beats the
/// `WHERE`) while `SET v = zzz.v WHERE nosuchcol = 1` is `42P01` (a right-hand side
/// beats it too); and over a `GENERATED ALWAYS` `n`, `SET n = 5 WHERE nosuchcol = 1` is
/// `42703` and `SET n = 5, nosuchcol = 'y'` is `42703`, so the identity guard is last.
///
/// # Errors
///
/// [`Error::AmbiguousColumn`] (`42702`) for a bare reference to a target column,
/// [`CatalogError::ColumnNotFound`] (`42703`) for an assignment to a column the target
/// lacks, [`Error::Type`] (`42804`) for an assignment no cast context carries (see
/// [`check_assignment`]), [`Error::GeneratedAlwaysUpdate`] (`428C9`) for a non-`DEFAULT`
/// assignment to a `GENERATED ALWAYS` identity column, or whatever
/// [`resolve_expr_types_gated`] raises.
fn lower_on_conflict(
    clause: OnConflictClause,
    table: &str,
    schema: &Schema,
) -> Result<OnConflictClause> {
    let OnConflictAction::DoUpdate {
        assignments,
        where_predicate,
    } = clause.action
    else {
        return Ok(clause);
    };

    let table_def = schema.get_table(table);
    let column_types = column_type_map_for(schema, table);
    let action_scope = on_conflict_action_scope(table, schema);
    // The descriptor-level twin of `action_scope` — the same target-plus-`excluded`
    // namespace, which pass 2's assignment gate reads through `Expr::infer_type`. The
    // pairing is the one `on_conflict_action_scope` names: a change to what the action
    // can name moves both.
    let action_relations = on_conflict_action_relations(table, schema);

    // Pass 1 — every right-hand side, in written order, before any target name.
    let resolved: Vec<(String, Expr)> = assignments
        .into_iter()
        .map(|(col, expr)| {
            reject_ambiguous_bare_column(&expr, &column_types)?;
            // A `DEFAULT` RHS stays bare so the executor resolves the column's catalog
            // default; coercing it would hide it from that path (mirrors the
            // INSERT/UPDATE lowering).
            if matches!(expr, Expr::Default) {
                return Ok((col, expr));
            }
            // Resolve and gate the right-hand side before wrapping it in the column
            // coercion, the same pass the `WHERE` leg below gets. `coerce_to_type` only
            // says what the value must become, never whether it is well-typed on its
            // own, so `DO UPDATE SET tx = COALESCE(t.i4, t.tx)` was accepted with no
            // common type at all — and *silently*, because a statement that hits no
            // conflict never evaluates the branch, so even the eval-time gate never
            // fired. Resolution travels with the gate so `SET b = (EXCLUDED.i4 = '10')`
            // still coerces its unknown literal instead of being falsely rejected.
            Ok((col, resolve_expr_types_gated(expr, &action_scope, schema)?))
        })
        .collect::<Result<Vec<_>>>()?;

    // Pass 2 — every target name, **and that assignment's own type verdict**, per target
    // in written order. `attnameAttNum`'s error names the relation, unlike the right-hand
    // sides' `42703`; `transformAssignedExpr` follows it immediately in the same loop
    // iteration, so measured on 18.6 over a `BOOL` `b`, `DO UPDATE SET b = 1, nosuch = 1`
    // is `42804` while `SET nosuch = 1, b = 1` is `42703`. Both outrank the action's
    // `WHERE` (pass 3, `transformOnConflictClause` transforms the target list first) and
    // the identity guard (pass 4, a rewriter check) — `SET b = 1 WHERE nosuchcol = 1` is
    // `42804` there, and here.
    let action_columns = action_relations.flatten();
    let action_type_scope = TypeScope::Input {
        relations: &action_relations,
        columns: &action_columns,
        enclosing: None,
    };
    for (col, expr) in &resolved {
        let Some(target_type) = column_types.get(col).map(|t| t.oid) else {
            return Err(CatalogError::ColumnNotFound {
                table: table.to_string(),
                column: col.clone(),
            }
            .into());
        };
        if !matches!(expr, Expr::Default) {
            check_assignment(expr, col, target_type, schema, action_type_scope)?;
        }
    }

    // Pass 3 — the action's `WHERE`, gated like every other DML filter rather than only
    // coercion-checked.
    let where_predicate = where_predicate
        .map(|predicate| {
            reject_ambiguous_bare_column(&predicate, &column_types)?;
            let predicate = resolve_expr_types_gated(predicate, &action_scope, schema)?;
            // 18.6 spells this clause `"WHERE"` too (`analyze.c`'s `transformInsertStmt`
            // hands `transformWhereClause` that name), and it sits here — after the
            // action's own `SET` list — because that is the order `transformOnConflictClause`
            // runs them in. The arbiter's inference `WHERE` is **not** a member: 18.6
            // transforms it with a bare `transformExpr` and no boolean coercion, so
            // `ON CONFLICT (u) WHERE k DO NOTHING` is accepted there.
            check_boolean_clause(&predicate, BoolContext::Where, &action_scope, schema)?;
            Result::Ok(predicate)
        })
        .transpose()?;

    // Pass 4 — the identity guard (a rewriter check on 18.6, so it runs after every
    // parse-analysis error above) and the store coercion. A `GENERATED ALWAYS` identity
    // column can only be updated to DEFAULT (`428C9`); DO UPDATE never routes through
    // `lower_update`, so the guard is applied here too (`BY DEFAULT` is allowed).
    let assignments = resolved
        .into_iter()
        .map(|(col, expr)| {
            if matches!(expr, Expr::Default) {
                return Ok((col, expr));
            }
            let is_always_identity = table_def
                .and_then(|t| t.columns.iter().find(|c| c.name == col))
                .is_some_and(|c| matches!(c.identity, Some(IdentityGeneration::Always)));
            if is_always_identity {
                return Err(Error::GeneratedAlwaysUpdate {
                    table: table.to_string(),
                    column: col,
                });
            }
            let expr = match column_types.get(&col) {
                Some(target_type) => coerce_to_type(expr, target_type.oid),
                None => expr,
            };
            Ok((col, expr))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(OnConflictClause {
        target: clause.target,
        action: OnConflictAction::DoUpdate {
            assignments,
            where_predicate,
        },
    })
}

/// Reject a **bare** column reference inside an `ON CONFLICT DO UPDATE` action.
///
/// Postgres puts *both* the target relation and the `excluded` pseudo-relation in
/// scope there, and `excluded` carries the target's whole shape — so every target
/// column name has two claimants and a bare reference to one is `42702`, in the `SET`
/// right-hand side (`SET n = n + 1`) exactly as in the `WHERE` (`WHERE n > 45`). The
/// reference has to say which row it means: `t.n` for the stored one, `excluded.n` for
/// the would-be-inserted one. pglet used to accept both spellings and silently bind
/// them to the **target** row — right by luck for the increment idiom, a silent no-op
/// for a `WHERE` meant to read `excluded`.
///
/// A name no target column carries is not ambiguous; it falls through to the
/// undefined-column error raised downstream. Sub-plans are not descended into
/// (`Expr::Subquery` / `Expr::Exists` are leaves to the child walk), because a
/// correlated subquery's own `FROM` introduces a scope where a bare name binds
/// normally.
///
/// # Errors
///
/// [`Error::AmbiguousColumn`] (`42702`) for the first bare reference to a target column.
fn reject_ambiguous_bare_column(
    expr: &Expr,
    column_types: &HashMap<String, ResolvedType>,
) -> Result<()> {
    crate::depth::check_stack()?;
    if let Expr::Column { table: None, name } = expr
        && column_types.contains_key(name)
    {
        return Err(Error::AmbiguousColumn {
            column: name.clone(),
        });
    }
    expr.try_for_each_child(|child| reject_ambiguous_bare_column(child, column_types))
}

/// Apply the identity-column rules to a fully-built (table-ordered) `VALUES` row.
///
/// For each identity column carrying an explicitly supplied (non-`DEFAULT`) value:
/// - `OVERRIDING USER VALUE` discards it (the sequence default is drawn instead);
/// - `GENERATED ALWAYS` without `OVERRIDING SYSTEM VALUE` is rejected `428C9`;
/// - otherwise (`BY DEFAULT`, or `ALWAYS` + `OVERRIDING SYSTEM VALUE`) it is kept.
///
/// An omitted column (`Expr::Default`) always draws the sequence value, so it is
/// skipped. A non-identity column is untouched.
fn apply_identity_rules(
    table_def: &Table,
    table: &str,
    row: &mut [Expr],
    overriding: Overriding,
) -> Result<()> {
    for (idx, col) in table_def.columns.iter().enumerate() {
        let Some(generation) = col.identity else {
            continue;
        };
        if matches!(row[idx], Expr::Default) {
            continue;
        }
        match overriding {
            Overriding::User => row[idx] = Expr::Default,
            Overriding::System => {}
            Overriding::None => {
                if matches!(generation, IdentityGeneration::Always) {
                    return Err(Error::GeneratedAlwaysInsert {
                        table: table.to_string(),
                        column: col.name.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Enforce the identity rules for an `INSERT … SELECT` whose target list names an
/// identity column (the source supplies its value).
///
/// A `GENERATED ALWAYS` target without `OVERRIDING SYSTEM VALUE` is `428C9`, as
/// Postgres rejects an `INSERT … SELECT` that feeds a value into a GENERATED ALWAYS
/// column. `OVERRIDING USER VALUE` over a SELECT source (the copy-between-tables
/// idiom) would require dropping the source column and is not modeled — it fails
/// loud (`0A000`) rather than silently using the user value.
fn enforce_identity_select(
    table_def: &Table,
    table: &str,
    target_columns: &[usize],
    overriding: Overriding,
) -> Result<()> {
    for &idx in target_columns {
        let Some(col) = table_def.columns.get(idx) else {
            continue;
        };
        let Some(generation) = col.identity else {
            continue;
        };
        match overriding {
            Overriding::System => {}
            Overriding::User => {
                return Err(Error::NotImplemented(
                    "OVERRIDING USER VALUE for an identity column in INSERT … SELECT \
                     is not supported"
                        .into(),
                ));
            }
            Overriding::None => {
                if matches!(generation, IdentityGeneration::Always) {
                    return Err(Error::GeneratedAlwaysInsert {
                        table: table.to_string(),
                        column: col.name.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Refuse a `SET` list that assigns one column more than once
/// (`42601 multiple assignments to same column "…"`).
///
/// The same *is a set* invariant the target column lists carry, over a different
/// statement shape and — because 18.6 raises it from a different place — a different
/// SQLSTATE. `process_matched_tle` (`rewrite/rewriteHandler.c`) merges a repeated
/// assignment or refuses it, and pglet parses no assignment it could merge (`SET foo[2]
/// = 42, foo[4] = 43` is `42601 syntax error` here), so every repeat is the refusal.
///
/// **The rule is "more than one assignment", not "conflicting assignments":**
/// `SET a = 1, a = 1` is refused too, measured. 18.6 keys it on the resolved attribute;
/// keying on the folded name here is the same key, because a relation's column names are
/// a set and the name was resolved against that set before this runs.
///
/// It carries no order either. Its callers run it where 18.6's **rewriter** runs — after
/// every clause of the statement is transformed **and** below the correlated re-walk that
/// closes a nested sub-plan's binding gate — and each states the boundaries it measured.
///
/// What that position buys is an ordering against the errors pglet itself raises **while
/// it lowers**, and against nothing else. **Outside that set, what answers is decided by
/// whether the statement reaches this gate at all — a property of the call site, never of
/// the verdict it would otherwise stand in front of.** It is reached only below every
/// verdict the same lowering raises above it, only with the target relation in the
/// catalog ([`lowers_clauses`]), and — on the `UPDATE` route alone — only with the
/// `source_gated` skip in [`lower_update`] left on. What pglet would answer later, and
/// what 18.6 answers at all, are read at none of them, so neither predicts the outcome.
/// These are instances of that rather than the set:
///
/// - One clause, one 18.6 verdict, opposite sides. `RETURNING (SELECT 1 FROM nosuchrel)`
///   beside a repeat is `42P01` on 18.6 under either statement; the `UPDATE` route's skip
///   turns this gate off and pglet's own `42P01` answers at execution over a matching row
///   (nothing does over none), while [`lower_insert`]'s `ON CONFLICT DO UPDATE` call site
///   carries no such skip and this gate answers `42601` — at a conflicting key and a free
///   one alike.
/// - A relation the catalog lacks in a `SET` right-hand side is outside that skip's clause
///   list, so the gate runs there too: `SET a = (SELECT 1 FROM nosuchrel), a = 2` is
///   `42601` here against `42P01` there, at either row count.
/// - A gate that runs answers even with no verdict of pglet's own to stand in front of: a
///   non-boolean `WHERE` is 18.6's `42804` and is raised here **nowhere**
///   (`UPDATE t SET a = 1 WHERE a` runs and matches no row), so `SET a = 1, a = 2 WHERE a`
///   is this gate's `42601` standing in front of nothing.
///
/// Which statements reach it is not enumerated and each bound written for that has gone
/// short; the measured residuals are `docs/names.md` § What the refusals do not carry.
///
/// # Errors
///
/// [`Error::Syntax`] (`42601`, no position — 18.6 sends none for this one) naming the
/// first column assigned twice, in written order.
fn refuse_duplicate_assignments(assignments: &[(String, Expr)]) -> Result<()> {
    let mut assigned: Vec<&str> = Vec::with_capacity(assignments.len());
    for (col, _) in assignments {
        if assigned.contains(&col.as_str()) {
            return Err(Error::syntax_sentence(format!(
                "multiple assignments to same column \"{col}\""
            )));
        }
        assigned.push(col);
    }
    Ok(())
}

/// Record `position` as claimed by a statement's target column list, refusing
/// (`42701 column "…" specified more than once`) one the list already claimed.
///
/// The one predicate behind *a statement's target column list is a set*. It is keyed on
/// the **resolved position**, as both of 18.6's implementations are — `checkInsertTargets`
/// (`parser/parse_target.c`) tests a `Bitmapset` of attnums, `CopyGetAttnums`
/// (`commands/copy.c`) a `list_member_int` over the attnums it has collected. What that
/// buys is not the quoted spelling, which the parser's fold settles before this runs
/// (`INSERT INTO t ("a", a)` is one column on both engines either way); it is that a name
/// which never resolves never claims anything, so `(zz, zz)` is the resolver's `42703`.
///
/// **One `Vec<usize>` stands in for `checkInsertTargets`' *two* bitmapsets, on a premise
/// this parser supplies.** That function keeps `wholecols` and `partialcols` and branches
/// on the target's shape: a whole column refuses an attnum in either set, a **partial** one
/// (`a[1]`) refuses only one in `wholecols` — so two partial claims on one attnum are
/// deliberately allowed. Measured over `ar (a INT[], b INT)`: `INSERT INTO ar (a[1], a[2])` and
/// `INSERT INTO ar (a[1], a[1])` are accepted on 18.6 while `INSERT INTO ar (a, a[1])` and
/// `INSERT INTO ar (a[1], a)` are `42701`. Nothing diverges because this parser reads no
/// indirection target at all — all four are `42601` here — the same premise
/// [`refuse_duplicate_assignments`] states for the `SET` route, and the one a parser that
/// gained `a[1]` targets would have to revisit at both gates together.
///
/// It carries no order: **where** a family runs it is that family's decision, made at the
/// call site, because the families do not agree. The `INSERT` and `COPY` lists claim one
/// name as soon as they resolve it, so a repeat outranks a later unknown name; an `UPDATE`
/// `SET` list meets its duplicate check a whole phase later and never does. That is
/// [`resolve_insert_columns`]' and `executor::resolve_columns`' rule, not this
/// function's — and [`refuse_duplicate_assignments`] is the `SET` list's separate
/// predicate, over a separate SQLSTATE.
///
/// # Errors
///
/// [`CatalogError::DuplicateColumnName`] (`42701`) when `position` is already claimed.
pub fn claim_target_column(claimed: &mut Vec<usize>, position: usize, name: &str) -> Result<()> {
    if claimed.contains(&position) {
        return Err(CatalogError::DuplicateColumnName {
            column: name.to_string(),
        }
        .into());
    }
    claimed.push(position);
    Ok(())
}

/// Resolve an INSERT's target column list to table-column indices, in written order,
/// refusing a name the list already holds.
///
/// The two verdicts **interleave**, one list item at a time, because
/// `checkInsertTargets` interleaves them: it resolves a name and tests it against the
/// columns it has already claimed before it reads the next one. So a repeat at item 2
/// outranks an unknown name at item 3 (`INSERT INTO t (a, a, zz)` is `42701`), while an
/// unknown name at item 1 outranks a repeat after it (`INSERT INTO t (zz, a, a)` is
/// `42703`) — measured on 18.6, both. No pair of whole-list passes reproduces that.
///
/// The interleave is this family's own. An `UPDATE`'s `SET` list resolves every name in
/// parse analysis and meets its duplicate check in the rewriter a whole phase later, so
/// there an unknown name wins from *either* position; `COPY`'s list interleaves as this
/// one does, through its own loop over the same
/// [`claim_target_column`]. A helper shared across the families with the ordering baked
/// in would report `42701` for `UPDATE t SET a = 1, a = 2, zz = 3`, which is `42703`.
///
/// Its callers are the ones `checkInsertTargets` has — the `VALUES` row source's column
/// mapping and the `INSERT … SELECT` one — and in each it runs ahead of the row's own
/// work, which is where `checkInsertTargets` runs: ahead of `transformInsertRow`'s arity
/// check and ahead of every cell's coercion. So `INSERT INTO t (a, a) VALUES (1)` and
/// `INSERT INTO t (a, a) VALUES (1, 'zz')` are 18.6's `42701` rather than the arity error
/// or the `22P02`, and `INSERT INTO t (a, a) SELECT 1` rather than the source-arity
/// error.
///
/// **"The row's own work" is the bound, and the `INSERT … SELECT` source *query* is
/// outside it.** 18.6 runs `checkInsertTargets` ahead of the source's own
/// `transformStmt` (both in `analyze.c`'s `transformInsertStmt`); pglet lowers the
/// source `SELECT` first and reports whatever that lowering raises. Measured: `INSERT INTO t (a, a) SELECT 1,
/// nosuchcol FROM t` is `42701` there and `42703` here, `… SELECT COALESCE(1, 'x'::text),
/// 2` is `42701` there and `42804` here, and `… SELECT 1, 2 ORDER BY nosuchcol` is
/// `42701` there and `42703` here. What bounds the bound: a source relation the catalog
/// lacks is pglet's `42P01` at execution, so `INSERT INTO t (a, a) SELECT 1 FROM
/// nosuchrel` is `42701` on both. `docs/names.md` § What the refusals do not carry
/// records the residual.
///
/// # Errors
///
/// [`CatalogError::ColumnNotFound`] (`42703`) for a name the relation lacks,
/// [`CatalogError::DuplicateColumnName`] (`42701`) for one the list already holds —
/// whichever the interleaved walk reaches first.
fn resolve_insert_columns(
    table_def: &Table,
    table: &str,
    columns: &[String],
) -> Result<Vec<usize>> {
    let mut targets = Vec::with_capacity(columns.len());
    for name in columns {
        let position = resolve_insert_column(table_def, table, name)?;
        claim_target_column(&mut targets, position, name)?;
    }
    Ok(targets)
}

/// Resolve an INSERT target column name to its table-column index.
///
/// **Exact.** Both sides came through the parser's name fold, so an unquoted
/// spelling on either side is already lowercase; a case-insensitive fallback here
/// would let `INSERT INTO ci (userid) …` write to a `"UserId"` column that the
/// matching `SELECT userid FROM ci` refuses (`42703` on 18.6, `checkInsertTargets`).
fn resolve_insert_column(table_def: &Table, table: &str, name: &str) -> Result<usize> {
    table_def
        .columns
        .iter()
        .position(|c| c.name == name)
        .ok_or_else(|| {
            // `42703`, Postgres's own SQLSTATE and wording for `checkInsertTargets`.
            // The bare `Error::Execution` this used to raise was `22000`; the fallback
            // above kept it nearly unreachable, so removing the fallback is what makes
            // the code observable.
            CatalogError::ColumnNotFound {
                table: table.to_string(),
                column: name.to_string(),
            }
            .into()
        })
}

/// Map an `INSERT … SELECT`'s source output columns to target-table column indices.
///
/// With an explicit column list (`INSERT INTO t (a, b) SELECT …`), each named
/// column resolves to its table index, in list order;
/// the source must produce exactly that many columns. Without a list, the source
/// columns map positionally onto the leading table columns (`0..n`). Returns one
/// target index per source output column, in source order.
///
/// A length mismatch is 18.6's `42601`, raised here ahead of the source's assignment
/// gate and its relation resolution, where 18.6 transforms the source whole first — so
/// a source error pglet reaches only after this check loses to it (`INSERT INTO t SELECT
/// nosuch, 1, 2` into a two-column `t` is 18.6's `42703` and `42601` here): a known
/// divergence, `EDGES.md` row 226's residual.
fn insert_select_target_indices(
    columns: &[String],
    table: &str,
    schema: &Schema,
    source_plan: &Plan,
) -> Result<Vec<usize>> {
    let source_cols = source_plan.output_columns(schema);
    let Some(table_def) = schema.get_table(table) else {
        // No target table: map positionally; the executor raises table-not-found.
        return Ok((0..source_cols.len()).collect());
    };

    if columns.is_empty() {
        if source_cols.len() > table_def.columns.len() {
            return Err(Error::syntax_sentence(MORE_EXPRESSIONS));
        }
        return Ok((0..source_cols.len()).collect());
    }

    // The column list resolves **before** the source arity is compared, because
    // `checkInsertTargets` runs ahead of `transformInsertRow`: measured on 18.6,
    // `INSERT INTO t (a, a) SELECT 1` is `42701` and `INSERT INTO t (a, zz) SELECT 1` is
    // `42703`, where both were this arity error here.
    let targets = resolve_insert_columns(table_def, table, columns)?;

    if source_cols.len() > targets.len() {
        return Err(Error::syntax_sentence(MORE_EXPRESSIONS));
    }
    if source_cols.len() < targets.len() {
        return Err(Error::syntax_sentence(MORE_TARGET_COLUMNS));
    }

    Ok(targets)
}

/// Gate every `INSERT … SELECT` source column against the column it is assigned to
/// (`42804`), the `SELECT`-source spelling of the check the `VALUES` cells get.
///
/// A source naming a relation the catalog lacks turns the gate off for the whole
/// statement, the same rule [`lower_update`] states for an `UPDATE … FROM`: 18.6
/// resolves the range table first, so `INSERT INTO t (i) SELECT n FROM nosuchrel` is
/// `42P01` there whatever the types, and pglet raises that at execution.
///
/// # Errors
///
/// [`Error::Type`] (`42804`) for the first source column in source order whose type no
/// assignment cast carries into its target column.
fn check_insert_select_assignments(
    source_plan: &Plan,
    target_columns: &[usize],
    table_def: &Table,
    schema: &Schema,
) -> Result<()> {
    if !plan_relations_resolvable(source_plan, schema) {
        return Ok(());
    }
    let source_types = target_list_source_types(source_plan, schema, ParameterSource::Opaque);
    for (i, &idx) in target_columns.iter().enumerate() {
        let (Some(&AssignmentSource::Typed(source)), Some(column)) =
            (source_types.get(i), table_def.columns.get(idx))
        else {
            continue;
        };
        assignment_legality(source, column.type_oid, &column.name)?;
    }
    Ok(())
}

/// What each output column of a query level's target list carries into an enclosing
/// construct — the [`assignment_source_type`] question asked of a plan's columns rather
/// than of one expression.
///
/// Two readers, and they differ only in what a bare, undeclared `$N` is
/// ([`ParameterSource`]): an `INSERT … SELECT` source, whose target column types such a
/// parameter, and a set operation's side (`super::query`'s `build_set_op`), where 18.6's
/// `constructSetOpTargetlist` treats an `unknown` `Param` exactly as it treats an
/// `unknown` `Const`.
///
/// # The rule this reproduces
///
/// Postgres leaves a bare constant `unknown` and lets the **target column** resolve it,
/// so `INSERT INTO t (b) SELECT 't'` assigns a string constant into a `BOOL` column.
/// What takes that away is any construct that resolves the entry to `text` *first*, and
/// the set of those is neither "the wrapper nodes" nor "everything but a projection" —
/// it is the claimants of `findTargetlistEntrySQL92` and `findTargetlistEntrySQL99` plus
/// the whole-list resolvers. Measured
/// on 18.6, `INSERT INTO f1t (b) SELECT 't' …`:
///
/// | resolves (`42804`) | does not resolve (accepted) |
/// | --- | --- |
/// | `ORDER BY 1`, `ORDER BY z` | `LIMIT 1`, `OFFSET 0`, `LIMIT ALL` |
/// | `GROUP BY 1` | `GROUP BY n`, `HAVING count(*) > 0` |
/// | `DISTINCT` | `DISTINCT ON (n)`, `ORDER BY n` |
/// | a set operation, a derived table | a `WHERE`, a join, a `FROM` derived table |
///
/// So the walk descends the wrappers that resolve nothing, and each node that *does*
/// resolve marks the **positions it names** — [`AssignmentSource::resolved`] — rather
/// than the whole list. Both halves of that are measured. Judging the source's
/// **immediate** node alone — everything else falling to the `text`-substituting
/// fallback — refuses these spellings of `INSERT INTO f5t (b) SELECT 't' …`, each
/// accepted by 18.6 and each re-measured against the code as it stands (a *count* of
/// them is what goes stale, so this is a list):
///
/// - `LIMIT 1`, `OFFSET 0`, `LIMIT ALL` — a [`Plan::Limit`];
/// - `FROM s ORDER BY n LIMIT 1` — a `Limit` again, over the sort;
/// - `FROM s GROUP BY n ORDER BY n`, and a window projection with `ORDER BY n` — a
///   [`Plan::Sort`], which an `Aggregate` and a `Window` keep *above* them;
/// - `FROM s GROUP BY n ORDER BY count(*)` — a [`Plan::JunkFilter`] over that sort;
/// - `DISTINCT ON (n) … FROM s` — a [`Plan::DistinctOn`].
///
/// The spellings it gets right, it gets right for a different reason: a bare
/// `ORDER BY n`, a `WHERE`, a join, a `FROM` derived table, a CTE source, a bare
/// `GROUP BY n`, a `HAVING` and a bare window projection each already top out at a
/// target-list node. (The `GROUP BY n`, `HAVING` and window members this list used to
/// carry were three of those — the enumeration was written from the arms rather than
/// measured, and it disagreed with both counts written about it: eight members under a
/// sentence saying nine, under a doc line saying six — the mis-count `docs/coercion.md`
/// records.) And marking a
/// sort's whole list refuses `SELECT 't', 5 ORDER BY 2`, whose *second* entry is the only
/// one a key names.
///
/// # Where the positions come from
///
/// **Not from the finished plan** — both claiming clauses rewrite the key into the entry
/// it named, and two entries can be equal, so the position is recorded at the lowering
/// ([`ResolvedTargets`], on [`Plan::Sort`] and [`Plan::Aggregate`]) and read here. The
/// two shapes that force it are measured on 18.6 and pinned:
/// `SELECT 't' AS p, 't' AS q FROM src ORDER BY p, n` (a sort-**below**-projection
/// layout, whose `Plan::Sort` sits *under* the `Project` carrying the target list) and
/// `SELECT 't', 't' FROM src GROUP BY 1`.
///
/// # What each arm is
///
/// [`Plan::Project`], [`Plan::Values`] (a no-FROM `SELECT`, which lowers to a single
/// row), [`Plan::Aggregate`] and [`Plan::Window`] are the nodes carrying a query's own
/// target list 1:1, so each expression is judged as itself, and each marks the entries
/// its own window keys equal ([`window_key_claims`]). `Aggregate` additionally marks the
/// entries its `GROUP BY` names. `Project` marks the entries a `Plan::Sort`
/// **beneath** it names, which is the sort-below-projection layout `build_sort` chooses
/// when a key reads a base column the output drops (`ORDER BY 1, n`); the `Sort` there
/// carries the *enclosing* target list's positions, not its own input's columns.
/// [`Plan::Sort`] marks them in the sort-above layout, where it sits on top of that same
/// target list. [`Plan::Distinct`] marks every entry, because plain `DISTINCT` groups on
/// all of them — the one claimant set that needs no position, which is why it is decided
/// here rather than at the lowering. [`Plan::Limit`], [`Plan::Filter`],
/// [`Plan::JunkFilter`] and [`Plan::DistinctOn`] resolve nothing and pass through — the
/// last because its ON keys are merged into the `Sort` beneath it
/// (`merge_distinct_on_order_by`), which is what marks them. That group is
/// enumerated by the **rule** (a node claiming no target-list entry) rather than by
/// reachability: `Filter` is in it because a `WHERE` claims none, and no shape has been
/// found that puts one on top of an `INSERT … SELECT` source. Including it cannot
/// over-accept — a node that claims nothing leaves every entry as its input left it —
/// while omitting it would fall to the `text`-substituting fallback.
///
/// Everything else takes [`Plan::output_columns`]' answer, which is right *because*
/// those shapes have already resolved their columns: a set operation to its common type
/// when it was built (`super::query`'s `build_set_op`), so an all-`unknown` one
/// (`SELECT 't' UNION SELECT 'f'`) is `text`, and a derived table
/// (`SELECT * FROM (VALUES ('t')) v`) likewise — both `42804` into a `bool` column on
/// both engines. A **wildcard** projection takes it for a different
/// reason: `*` expands to its input's columns, so the expression list no longer indexes
/// the output columns at all.
///
/// # Why the answer is three-way
///
/// `Plan::output_columns` **substitutes `text`** for an expression it cannot type, so a
/// two-way answer would gate a `$N` and a column of a relation the catalog lacks against
/// a type nothing in the statement has. [`AssignmentSource::Opaque`] is what keeps those
/// out, *and* keeps them out through a resolving node: `SELECT n FROM nosuchrel ORDER BY
/// 1` must stay the `42P01` both engines report. (The plan-level
/// [`plan_relations_resolvable`] gate above covers the same class from the other side
/// and neither subsumes the other — it reaches a missing relation this walk never
/// descends to, such as one side of a join.)
pub(super) fn target_list_source_types(
    plan: &Plan,
    schema: &Schema,
    parameters: ParameterSource,
) -> Vec<AssignmentSource> {
    // The target-list nodes share this: each expression judged against the scope its
    // input puts in place, then the entries `claimed` names resolved to `text`.
    let target_list = |input: &Plan, exprs: &[Expr], claimed: &ResolvedTargets| {
        let (shape, relations) = input.output_columns_scoped(schema);
        let scope = TypeScope::Input {
            relations: &relations,
            columns: shape.columns(),
            enclosing: None,
        };
        let window_claims = window_key_claims(exprs);
        exprs
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let source = parameters.source_type(e, schema, scope);
                if claimed.claims(i) || window_claims.claims(i) {
                    source.resolved()
                } else {
                    source
                }
            })
            .collect::<Vec<_>>()
    };
    match plan {
        Plan::Project { input, exprs, .. } if !exprs.iter().any(projects_wildcard) => {
            // A `Plan::Sort` **below** this projection carries this target list's own
            // claims — `build_sort`'s sort-below layout, taken when a key reads a base
            // column the output drops.
            match input.as_ref() {
                Plan::Sort {
                    resolved_targets, ..
                } => target_list(input, exprs, resolved_targets),
                _ => target_list(input, exprs, &ResolvedTargets::default()),
            }
        }
        // A no-FROM `SELECT <exprs>` lowers to a single-row `Plan::Values`, which has no
        // input to resolve a column against — the same `TypeScope::NONE` its own
        // `output_columns` passes.
        Plan::Values { rows, .. } if rows.len() == 1 && !rows[0].iter().any(projects_wildcard) => {
            rows[0]
                .iter()
                .map(|e| parameters.source_type(e, schema, TypeScope::NONE))
                .collect()
        }
        Plan::Aggregate {
            input,
            exprs,
            resolved_targets,
            ..
        } if !exprs.iter().any(projects_wildcard) => target_list(input, exprs, resolved_targets),
        Plan::Window {
            input,
            window_exprs,
            ..
        } if !window_exprs.iter().any(projects_wildcard) => {
            target_list(input, window_exprs, &ResolvedTargets::default())
        }
        Plan::Filter { input, .. } | Plan::Limit { input, .. } | Plan::DistinctOn { input, .. } => {
            target_list_source_types(input, schema, parameters)
        }
        Plan::JunkFilter { input, keep } => {
            let mut types = target_list_source_types(input, schema, parameters);
            types.truncate(*keep);
            types
        }
        Plan::Sort {
            input,
            resolved_targets,
            ..
        } => {
            let mut types = target_list_source_types(input, schema, parameters);
            for (i, source) in types.iter_mut().enumerate() {
                if resolved_targets.claims(i) {
                    *source = source.resolved();
                }
            }
            types
        }
        Plan::Distinct { input } => target_list_source_types(input, schema, parameters)
            .into_iter()
            .map(AssignmentSource::resolved)
            .collect(),
        other => other
            .output_columns(schema)
            .into_iter()
            .map(|c| AssignmentSource::Typed(c.type_oid))
            .collect(),
    }
}

/// The entries of a target list its **window** keys name — every `PARTITION BY` and
/// window `ORDER BY` key of every window function the list carries, each claiming the
/// first entry it equals ([`sql99_target_index`]).
///
/// 18.6's `transformWindowDefinitions` hands both key lists to `transformSortClause` /
/// `transformGroupClause` with SQL99 rules forced, so a key equal to an `unknown` entry
/// resolves that entry to `text` as a `GROUP BY` naming it does: `SELECT 1, 1 UNION SELECT
/// '2', row_number() OVER (PARTITION BY '2') FROM t` is `42804`, and so is `OVER (ORDER BY
/// '2')` and `PARTITION BY NULL` over a `NULL` entry, measured. Unlike a sort or grouping
/// key, a window key is never rewritten into the entry it names — it stays inside the
/// `Expr::Window` — so the claim is read off the finished list rather than recorded at the
/// lowering ([`ResolvedTargets`] says why those two cannot be). A window in a nested
/// subquery belongs to that level, and [`Expr::for_each_child`] does not descend into one.
fn window_key_claims(exprs: &[Expr]) -> ResolvedTargets {
    fn collect(e: &Expr, targets: &[Expr], claims: &mut Vec<Option<usize>>) {
        if let Expr::Window {
            partition_by,
            order_by,
            ..
        } = e
        {
            let keys = order_by.iter().map(|k| &k.expr).chain(partition_by);
            claims.extend(keys.map(|k| sql99_target_index(k, targets)));
        }
        e.for_each_child(|child| collect(child, targets, claims));
    }
    let mut claims = Vec::new();
    for e in exprs {
        collect(e, exprs, &mut claims);
    }
    ResolvedTargets::from_claims(claims)
}

/// What a bare `$N` with no declared type is to [`target_list_source_types`].
///
/// A parameter the client declared at `Parse` never reaches this question: the parser
/// reads it as a `Cast` (`parser::parse_single_with_param_types`), which is typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ParameterSource {
    /// [`AssignmentSource::Opaque`], the `INSERT … SELECT` reading — the target column
    /// types the parameter, so neither the gate nor a resolving node may.
    Opaque,
    /// [`AssignmentSource::Unknown`], the set-operation reading — 18.6's
    /// `constructSetOpTargetlist` coerces an `unknown` `Param` to the column's common
    /// type as it coerces an `unknown` `Const`, and a sort, grouping, `DISTINCT` or
    /// window key naming it resolves it to `text` first, as it resolves the constant.
    Unknown,
}

impl ParameterSource {
    /// [`assignment_source_type`], with a bare `$N` (seen through an `AS alias`) read as
    /// this reading says.
    pub(super) fn source_type(
        self,
        expr: &Expr,
        schema: &Schema,
        scope: TypeScope<'_>,
    ) -> AssignmentSource {
        if self == Self::Unknown && is_bare_parameter(expr) {
            return AssignmentSource::Unknown;
        }
        assignment_source_type(expr, schema, scope)
    }
}

/// Whether `expr` is a bare `$N`, seen through an `AS alias`.
fn is_bare_parameter(expr: &Expr) -> bool {
    match expr {
        Expr::Parameter { .. } => true,
        Expr::Alias { expr, .. } => is_bare_parameter(expr),
        _ => false,
    }
}

/// The target-list expression behind output column `index` of `plan` and the input its
/// column references resolve against — `None` for a no-FROM `SELECT`, whose row has no
/// input — reached by [`target_list_source_types`]' descent, wildcard guards included.
/// `None` for any other node.
pub(super) fn target_list_entry_with_input(
    plan: &Plan,
    index: usize,
) -> Option<(&Expr, Option<&Plan>)> {
    match plan {
        Plan::Project { input, exprs, .. }
        | Plan::Aggregate { input, exprs, .. }
        | Plan::Window {
            input,
            window_exprs: exprs,
            ..
        } if !exprs.iter().any(projects_wildcard) => {
            exprs.get(index).map(|e| (e, Some(input.as_ref())))
        }
        Plan::Values { rows, .. } if rows.len() == 1 && !rows[0].iter().any(projects_wildcard) => {
            rows[0].get(index).map(|e| (e, None))
        }
        Plan::Filter { input, .. }
        | Plan::Limit { input, .. }
        | Plan::DistinctOn { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input } => target_list_entry_with_input(input, index),
        Plan::JunkFilter { input, keep } if index < *keep => {
            target_list_entry_with_input(input, index)
        }
        _ => None,
    }
}

/// The target-list expression behind output column `index` of `plan`, reached by the
/// descent [`target_list_source_types`] takes — for a caller rewriting an entry that walk
/// answered [`AssignmentSource::Unknown`] for — beside the record of the types that
/// target list was read at as written ([`CertainTypes`]), which a rewrite that changes
/// the entry's type has to move with it.
///
/// The arms are that walk's arms, with its wildcard guards dropped: such a guard there
/// only sends an entry to the typed fallback, which never answers `Unknown`, so an entry
/// this is asked for always sits under an arm both walks take. `None` for any other node.
pub(super) fn target_list_entry_mut(
    plan: &mut Plan,
    index: usize,
) -> Option<(&mut Expr, &mut CertainTypes)> {
    match plan {
        Plan::Project { exprs, certain, .. }
        | Plan::Aggregate { exprs, certain, .. }
        | Plan::Window {
            window_exprs: exprs,
            certain,
            ..
        } => exprs.get_mut(index).map(|entry| (entry, certain)),
        Plan::Values { rows, certain, .. } if rows.len() == 1 => {
            rows[0].get_mut(index).map(|entry| (entry, certain))
        }
        Plan::Filter { input, .. }
        | Plan::Limit { input, .. }
        | Plan::DistinctOn { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input } => target_list_entry_mut(input, index),
        Plan::JunkFilter { input, keep } if index < *keep => target_list_entry_mut(input, index),
        _ => None,
    }
}

/// Whether a projection item is a `*` or `qualifier.*`, which expands to several output
/// columns and so breaks the 1:1 index [`target_list_source_types`] needs.
///
/// Only a **sole bare** `*` over a single relation survives the planner unexpanded
/// (`expand_wildcards` leaves it for the executor's fast path), so that is the one
/// spelling this ever sees — and the one the discriminating fixture uses. Its source has
/// to be **empty**: over a non-empty one the store finalizer refuses the same pair, so a
/// seeded fixture is green with this predicate broken
/// (`insert/assignment_cast_select_wildcard_source_column_loud.sql`, measured both ways).
pub(super) fn projects_wildcard(expr: &Expr) -> bool {
    matches!(expr, Expr::Column { name, .. } if name == "*")
}

/// Lower a parsed UPDATE into a [`Plan::Update`].
///
/// The native parser parses the assignments + WHERE into raw lowered IR and calls
/// this, so per-column assignment coercion and WHERE type-resolution have one
/// implementation. `assignments` and `filter` are raw lowered IR (pre-coercion /
/// pre-resolution); only the table-column type context is applied here.
///
/// With no target relation ([`lowers_clauses`]) nothing is lowered and the executor's
/// `42P01` stands.
///
/// # Clause order
///
/// `analyze.c`'s `transformUpdateStmt` runs FROM, then the `WHERE`, then `RETURNING`,
/// and only then `transformUpdateTargetList` — which itself splits into three passes
/// (every right-hand side, then per target the name and its assignment cast) — and the
/// identity guard is a rewriter check after all of it. This function is written in that
/// order, and each boundary is measured against 18.6 at its own site.
///
/// # Errors
///
/// [`CatalogError::ColumnNotFound`] (`42703`) for a `SET` target the relation lacks,
/// [`Error::Type`] (`42804`) for an assignment no cast context carries (see
/// [`check_assignment`]), [`Error::Syntax`] (`42601`) for a `SET` list assigning one
/// column twice ([`refuse_duplicate_assignments`]),
/// [`Error::GeneratedAlwaysUpdate`] (`428C9`) for a non-`DEFAULT`
/// assignment to a `GENERATED ALWAYS` identity column, and whatever
/// [`resolve_expr_types_gated`] / [`lower_returning`] raise.
pub fn lower_update(
    table: String,
    alias: Option<String>,
    from: Option<Plan>,
    assignments: Vec<(String, Expr)>,
    filter: Option<Expr>,
    returning: Vec<Expr>,
    schema: &Schema,
) -> Result<Plan> {
    if !lowers_clauses(schema, &table) {
        return Ok(Plan::Update {
            table,
            alias,
            from: from.map(Box::new),
            assignments,
            filter,
            target_probe: None,
            returning,
        });
    }
    let column_types = column_type_map_for(schema, &table);

    // The scope is the target relation under the name a qualified reference must spell
    // (its alias if it has one), **plus** every relation an `UPDATE … FROM src s`
    // introduces. Registering the source is what makes `s.x` read as the source's `x`
    // rather than the target's same-named column: with only the target in scope, `s.x =
    // '1.1'` cast the literal to the *target*'s type — a `float4` round that silently
    // stopped matching, so the UPDATE reported success and wrote nothing.
    let mut scope = ColumnScope::single(
        alias.clone().unwrap_or_else(|| table.clone()),
        column_types.clone(),
    );
    if let Some(def) = schema.get_table(&table) {
        scope.mark_stored(def);
    }
    if let Some(source) = &from {
        push_plan_relations(&mut scope, source, schema);
    }
    // No `push_merged_names` here, and the reason is a parser gap rather than a rule:
    // this parser's `UPDATE … FROM` takes a single relation (`… FROM a JOIN b USING (id)`
    // is `42601`), so there is no merge for the closed scope's bare-name count to get
    // wrong. A parser that gains joins there needs the merged names, exactly as
    // `lower_select_level` passes them.

    // Target plus `FROM` is the whole namespace here — no enclosing level to chain to —
    // so the scope closes and the `WHERE` / `SET` name errors stop being
    // row-count-dependent: `UPDATE t SET v = zzz.v WHERE 1 = 0` was `UPDATE 0`, silently,
    // where 18.6 raises `42P01`. A source relation the catalog lacks registers **nothing**
    // (see `plan_relations`), which is deliberate: the qualifier then reports `42P01` here
    // rather than a `42703` for the columns of a table that is not there.
    let scope = scope.closed();
    // **The `SET` list resolves LAST** — `analyze.c`'s `transformUpdateStmt` runs FROM,
    // then `transformWhereClause`, then `transformReturningClause`, and only then
    // `transformUpdateTargetList`, under a comment saying the target list comes after
    // the SELECT-like processing is done. So of two ill-typed clauses Postgres reports
    // the `WHERE`, and of a `SET`/`RETURNING` pair it reports the `RETURNING`. The same
    // source-order rule the per-node walk enforces inside an expression, applied one
    // level up between the clauses of a statement.
    let filter = filter
        .map(|f| resolve_expr_types_gated(f, &scope, schema))
        .transpose()?;
    // The clause-root boolean gate, above the `RETURNING` items and the `SET` list for
    // the reason the paragraph above gives: `UPDATE t SET nosuch = 1 WHERE k` and
    // `… WHERE k RETURNING nosuchcol` are both `42804` on 18.6.
    if let Some(f) = &filter {
        check_boolean_clause(f, BoolContext::Where, &scope, schema)?;
    }
    // One relation list for the whole statement: Postgres builds `p_namespace` once from
    // the target plus its `FROM` and every clause reads it, so the `SET` gate below and
    // the `RETURNING` items here are two readers of one namespace rather than two
    // spellings of it (`docs/scope.md`'s "a clause's namespace is one object").
    let relations = returning_relations(
        &table,
        alias.as_deref().unwrap_or(&table),
        from.as_ref(),
        schema,
    );
    let returning = lower_returning(returning, &relations, schema)?;
    // Then each `SET` right-hand side, through the same resolve-and-gate the `WHERE`
    // gets. `coerce_to_type` only inserts the cast to the target column's type; it says
    // nothing about whether the expression itself is well-typed, so
    // `SET tx = COALESCE(i4, tx)` was accepted with no common type at all. Gating the
    // *unresolved* expression instead would falsely reject `SET b = (i4 = '10')`, whose
    // unknown literal has not yet adopted the column's type — which is why resolution
    // and the gate travel together here, exactly as they do for the filter.
    //
    // **The list takes three passes, and the split is `transformUpdateTargetList`'s
    // own.** It calls `transformTargetList` over *every* right-hand side before it looks
    // at a single target name, then walks the targets in written order resolving each
    // name (`attnameAttNum`) and immediately coercing that assignment
    // (`transformAssignedExpr`). Measured on 18.6, each pair discriminating: `SET b = 1,
    // i = nosuchcol` over a `BOOL` `b` is `42703` (a *later* right-hand side beats an
    // earlier target's `42804`), while `SET b = 1, nosuch = 1` is `42804` and `SET
    // nosuch = 1, b = 1` is `42703` (so the name and the type verdict interleave per
    // target rather than running as two whole-list passes).
    let resolved: Vec<(String, Expr)> = assignments
        .into_iter()
        .map(|(col, expr)| {
            // A `DEFAULT` RHS stays bare `Expr::Default` — the executor resolves the
            // column's catalog default (which coerces to the column type itself), so
            // wrapping it in a coerce-cast would hide it from that path and it would
            // reach general eval and fail loud (mirrors `lower_insert`).
            if matches!(expr, Expr::Default) {
                return Ok((col, expr));
            }
            Ok((col, resolve_expr_types_gated(expr, &scope, schema)?))
        })
        .collect::<Result<Vec<_>>>()?;

    let type_scope_columns = relations.flatten();
    let type_scope = TypeScope::Input {
        relations: &relations,
        columns: &type_scope_columns,
        enclosing: None,
    };
    // A relation this statement names that the catalog lacks leaves the assignment gate
    // — and the identity guard below it — off for the whole statement. Postgres resolves
    // a relation where it meets it, ahead of the target list in every case: the `FROM`
    // in `transformFromClause`, a subquery's own range table as its clause is
    // transformed. Measured on 18.6, each `42P01` there and each a plan-time `42804`
    // here without this: `UPDATE t SET b = 1 FROM nosuchrel`, `… FROM (SELECT n FROM
    // nosuchrel) s`, `… WHERE EXISTS (SELECT 1 FROM nosuchrel)`, `… WHERE id IN (SELECT
    // id FROM nosuchrel)` and `… RETURNING (SELECT 1 FROM nosuchrel)`. pglet raises that
    // `42P01` at **execution**, downstream of every plan-time verdict.
    //
    // **The clause list is the contract and it is this statement's, not a general one.**
    // `transformUpdateStmt` runs FROM, `WHERE`, `RETURNING`, *then* the target list, so
    // all three outrank the assignment cast — while `transformInsertStmt` runs its row
    // source **first**, so the mirrored `INSERT … VALUES (1) RETURNING (SELECT 1 FROM
    // nosuchrel)` and its `ON CONFLICT … DO UPDATE SET b = 1 WHERE EXISTS (…)` twin are
    // 18.6's `42804`, and those sites deliberately carry no such skip (measured, both).
    // A per-expression skip would not do for any of them, because a right-hand side can
    // be perfectly typed (`= 1`) while the statement's real error is still elsewhere.
    //
    // The `FROM` half used to be `relations.iter().count() > 1` — the target plus one
    // source — standing in for "`plan_relations` registered the source at all". It is
    // **subsumed**: this parser's `UPDATE … FROM` takes a single table factor,
    // `plan_relations` has an arm for every kind `parse_table_factor` produces, and the
    // one arm that can decline is a `Plan::Scan` whose table the catalog lacks — which
    // is exactly what this walk refuses. Measured as the mutation `> 1` → `>= 1`:
    // nothing reddens. It is *not* the other way round — the count cannot see through a
    // derived table.
    //
    // **The three gates these two skips feed do not all take the same half of it, and
    // the split is not "which clause 18.6 resolves first" but "when does pglet's own
    // `42P01` actually fire".** (The assignment cast is just below; the duplicate `SET`
    // target and the identity guard are at the bottom of this function, under the
    // correlated re-walk.)
    // Only the `FROM` is a plan node the executor runs whatever the row count. A
    // `WHERE` predicate and a `RETURNING` item are evaluated **per target row**, so over
    // an empty table neither is reached: measured, `UPDATE gi SET id = 5 WHERE EXISTS
    // (SELECT 1 FROM nosuchrel)` and its `RETURNING (SELECT 1 FROM nosuchrel)` spelling
    // both report `UPDATE 0` here against 18.6's `42P01`, while `… FROM nosuchrel` is
    // `42P01` here with no rows and with one.
    //
    // So a skip on those two clauses buys 18.6's own SQLSTATE when a row exists and
    // costs silence when none does, and whether that trade is payable depends on what
    // the gate was standing in front of. For the **assignment cast** it is: the same
    // statement with a well-typed right-hand side reports `UPDATE 0` too, gate or no
    // gate — a pre-existing zero-row hole (`EDGES.md`) the
    // gate neither opened nor can close by refusing with the wrong code. For the
    // **duplicate `SET` target** it is too, and for the same shape of reason: the
    // statement the wide skip turns that gate off for had no refusal before it either,
    // and it does not gain a duplicate write — over a matching row the skipped clause
    // raises the `42P01` and the autocommit wrap rolls the statement back, and over no
    // row nothing is written at all. (`RETURNING`'s spelling writes first and unwinds,
    // so on `MemoryStorage`, which has no statement rollback, the write stands — that is
    // row 40's pre-existing leak, reached the same way with or without this gate.) What
    // it does cost is the zero-row silence itself, and unlike the assignment cast's that
    // silence is against an 18.6 that refuses: measured over an empty table, `SET a = 1,
    // a = 2 WHERE EXISTS (SELECT 1 FROM nosuchrel)` and its `RETURNING (SELECT 1 FROM
    // nosuchrel)` spelling report `UPDATE 0` against `42P01`, before this gate and after
    // it. For the
    // **identity guard** the trade is not payable: `UPDATE gi SET id = 5 WHERE 1=0
    // RETURNING (SELECT 1 FROM nosuchrel)` answers `428C9`, so a skip there *removes a
    // refusal the statement had*, and a loud wrong code beats a silent success. The
    // identity guard therefore takes the `FROM` half only, and pays for it in the
    // row-matching `WHERE` / `RETURNING` spellings, which answer `428C9` against 18.6's
    // `42P01` — recorded rather than traded for a silence nobody sees.
    let from_relations_resolvable = from
        .as_ref()
        .is_none_or(|source| plan_relations_resolvable(source, schema));
    let source_gated = from_relations_resolvable
        && filter
            .as_ref()
            .is_none_or(|f| expr_relations_resolvable(f, schema))
        && returning
            .iter()
            .all(|item| expr_relations_resolvable(item, schema));
    let assignments: Vec<(String, Expr)> = resolved
        .into_iter()
        .map(|(col, expr)| {
            // The target name first. The executor checks it too — and must, for the
            // un-lowered path — but only as a whole-list pass ahead of the row loop,
            // which cannot report a *later* name after an earlier assignment's `42804`.
            let Some(target_type) = column_types.get(&col).map(|t| t.oid) else {
                return Err(CatalogError::ColumnNotFound {
                    table: table.clone(),
                    column: col,
                }
                .into());
            };
            if matches!(expr, Expr::Default) {
                return Ok((col, expr));
            }
            if source_gated {
                check_assignment(&expr, &col, target_type, schema, type_scope)?;
            }
            Ok((col, coerce_to_type(expr, target_type)))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut plan = Plan::Update {
        table,
        alias,
        from: from.map(Box::new),
        assignments,
        filter,
        target_probe: None,
        returning,
    };
    // A correlated subquery in the `WHERE` (or a `SET` RHS) was lowered before this
    // scope existed, so it is re-resolved against the target relation here — the same
    // pass `lower_select` runs. See `correlation.rs`.
    // A DML statement is always its own top level — there is no expression subquery it
    // could sit inside — so its chain is complete and the nested levels' gates may close.
    resolve_correlated_subplans(&mut plan, schema, true)?;
    // The target probe is chosen from the `WHERE` as it will execute, after the re-walk.
    // An `UPDATE … FROM` collects through its join and takes none.
    if let Plan::Update {
        table,
        alias,
        from: None,
        filter: Some(filter),
        target_probe,
        ..
    } = &mut plan
    {
        *target_probe = dml_target_probe(table, alias.as_deref(), filter, schema);
    }

    // **The two rewriter-position gates run below that re-walk**, which is what makes
    // them rewriter-position at all: the re-walk stands in for parse analysis — it is
    // where a correlated sub-plan's own binding gate closes — and a rewriter check
    // outranks nothing 18.6 refuses in parse analysis. `lower_insert` places its `ON
    // CONFLICT DO UPDATE SET` twin below the same call for the same reason. Run above
    // it, each beat a `42703` the re-walk raises, at zero rows and at one alike:
    // measured, `SET a = 1, a = 2 RETURNING (SELECT b FROM et ORDER BY zzz)`, its
    // `WHERE (SELECT b FROM et ORDER BY zzz) = 1` spelling and `SET a = (SELECT b FROM
    // et ORDER BY zzz), a = 2` are `42703` on 18.6 and answered the duplicate gate's
    // `42601`, while `SET id = 5 RETURNING (SELECT b FROM et ORDER BY zzz)` and its
    // `WHERE` spelling are `42703` on 18.6 and answered the identity guard's `428C9`.
    //
    // What the move leaves alone is the two skips below, whose subject is a relation the
    // catalog lacks: pglet raises its `42P01` for one at execution, downstream of every
    // plan-time verdict, and the re-walk does not raise it either — measured over an
    // empty table, `SET a = 1, a = 2 WHERE EXISTS (SELECT 1 FROM nosuchrel WHERE
    // nosuchrel.x = t.a)` and its `RETURNING` spelling report `UPDATE 0`, the qualified
    // inner reference included.
    if let Plan::Update {
        table, assignments, ..
    } = &plan
    {
        // First the `SET` list's own *is a set* check. It sits **above** the identity
        // guard because `rewriteTargetListIU` merges every target entry in its first
        // loop and only reaches the identity columns in its second: measured on 18.6
        // over a `GENERATED ALWAYS` `id`, `SET id = 1, id = 2` is `42601` and
        // `SET b = 1, b = 2, id = 3` is the `42601` too, where both were `428C9` here.
        //
        // It takes the **whole** `source_gated` skip rather than the identity guard's
        // `FROM` half, and the two differ because what they stand in front of differs.
        // Skipping the guard removes a refusal the statement had; skipping this one does
        // not — a statement whose `WHERE` or `RETURNING` names a relation the catalog
        // lacks either matches a row, reaches that clause and raises pglet's own `42P01`
        // (measured at one row: `SET a = 1, a = 2 WHERE EXISTS (SELECT 1 FROM
        // nosuchrel)` and its `RETURNING (SELECT 1 FROM nosuchrel)` spelling are `42P01`
        // on both engines), or matches none and writes nothing. So the wide skip costs
        // no duplicate write; what it costs is the refusal itself over an empty table,
        // where both spellings report `UPDATE 0` against 18.6's `42P01` — the same
        // silence the single-assignment `SET a = 1` spelling reports there, measured, so
        // it is the zero-row reachability hole rather than this gate's. Recorded in
        // `docs/names.md` § What the refusals do not carry.
        if source_gated {
            refuse_duplicate_assignments(assignments)?;
        }

        // Last, the identity guard: a `GENERATED ALWAYS AS IDENTITY` column can only be
        // `SET … = DEFAULT` (UPDATE has no `OVERRIDING`), so a non-`DEFAULT` assignment
        // to one is `428C9`. It runs **after** every clause because Postgres raises it in
        // the rewriter (`rewriteTargetListIU`), which runs after all of parse analysis —
        // measured on 18.6 over a `GENERATED ALWAYS` `id`: `SET id = 5 WHERE nosuchcol =
        // 1` and `SET id = 5 RETURNING nosuchcol` are `42703`, `SET id = 5, a =
        // COALESCE(1, 'x'::text)` is the `COALESCE`'s error, and `SET id = 5, b = 1` over
        // a `BOOL` `b` is that assignment's `42804`. Run at the top of this function, as
        // it was, it beat all four.
        //
        // **After every clause is still not after every *Postgres* error**, which is why
        // it shares the `FROM` half of the skip: pglet's `42P01` for an unresolvable
        // `FROM` source is raised later still, at execution, and a `FROM` is a plan node
        // the executor runs whatever the row count. Measured, `UPDATE g SET id = 5 FROM
        // nosuchrel` is `42P01` on 18.6 and `42P01` here — the skip is what buys that;
        // with `from_relations_resolvable` forced true it is `428C9` (measured as a
        // mutation). It deliberately does **not** take the `WHERE` or `RETURNING` half;
        // see `from_relations_resolvable`.
        if from_relations_resolvable && let Some(table_def) = schema.get_table(table) {
            for (col, expr) in assignments {
                if matches!(expr, Expr::Default) {
                    continue;
                }
                let is_always_identity = table_def
                    .columns
                    .iter()
                    .find(|c| &c.name == col)
                    .is_some_and(|c| matches!(c.identity, Some(IdentityGeneration::Always)));
                if is_always_identity {
                    return Err(Error::GeneratedAlwaysUpdate {
                        table: table.clone(),
                        column: col.clone(),
                    });
                }
            }
        }
    }
    Ok(plan)
}

/// The `RETURNING` clause's namespace as a [`ColumnScope`] — every relation of
/// [`returning_relations`], in FROM order, in a **closed** scope.
///
/// The projection from the namespace object to the object its readers gate against, in
/// one place. Its readers are [`lower_returning`], which gates the clause's own items,
/// and `super::correlation::walk_level`'s `Plan::Insert` arm, which hands the same scope
/// to every nested level a `RETURNING` item embeds. A second
/// spelling here would let the outer item and the subquery inside it disagree about what
/// the clause can name — the seam `docs/scope.md`'s "a clause's namespace is one object"
/// rule names.
///
/// The scope **closes** because that relation list is the whole namespace: there is no
/// enclosing level a `RETURNING` reference may reach, so a reference it cannot bind is
/// `42P01` / `42703` at plan time rather than an unknown deferred to eval.
pub(super) fn returning_scope(relations: &RelationColumns) -> ColumnScope {
    let mut scope = ColumnScope::default();
    for (name, columns) in relations.iter() {
        // The list's first relation is the DML target, a base table; the rest are an
        // `UPDATE … FROM`'s, of any kind.
        if scope.is_empty() {
            scope.push_base_relation(
                name.to_string(),
                crate::planner::build_desc_type_map(columns),
            );
        } else {
            scope.push_derived_relation(name.to_string(), columns);
        }
    }
    scope.closed()
}

/// Wildcard expansion ([`returning_wildcard`]) followed by the same resolve-and-gate every other
/// clause gets ([`resolve_expr_types_gated`]).
///
/// `RETURNING` is a projection — it declares output columns a client decodes against —
/// and it was the last clause reaching neither pass. Both halves matter, per row:
///
/// - **unresolved**, an unknown literal never adopts its sibling's type, so
///   `RETURNING i4 = '10'` compared `integer` against `text` and failed loud at *eval*
///   (or matched nothing on an empty result) where Postgres coerces and answers;
/// - **ungated**, a branch set with no common type was accepted and the two type
///   walkers each declared whatever the first branch happened to be —
///   `RETURNING COALESCE(i4, i8)` declared `int4` (4 bytes) over an `int8` value
///   (8 bytes), which a binary client mis-decodes, and `RETURNING ARRAY[i4, i8]` sent a
///   different element OID per row.
///
/// # The `RETURNING` namespace
///
/// `relations` is **not** the caller's own scope — it is [`returning_relations`], the DML
/// target under its effective name (its alias when one was written) followed by every
/// relation an `UPDATE … FROM src s` introduces, in FROM order. That is Postgres's own
/// namespace for the clause (`transformReturningClause` runs over the statement's whole
/// range table). It is passed in rather than derived here because the correlated re-walk
/// needs the identical object for an `INSERT`'s nested levels, and the derivation must be
/// one function ([`returning_scope`] turns it into the gated scope). The
/// spellings below were measured on both engines with `amb_t(id, v)` the target and
/// `amb_s(id, v, k)` the source aliased `s`, and answer identically:
///
/// | spelling | answer |
/// |---|---|
/// | `RETURNING *` | five columns — the target's, then the source's, in FROM order |
/// | `RETURNING s.*` | the source's three |
/// | `RETURNING s.k` / bare `RETURNING k` | `7` |
/// | `RETURNING v` (both relations carry it) | `42702` |
/// | `RETURNING amb_s.k` (the alias hides the table name) | `42P01` |
/// | `RETURNING s.nosuch` | `42703` |
///
/// The scope is **closed** ([`ColumnScope::closed`]) — that list is the whole namespace,
/// so a reference it cannot bind is `42P01` / `42703` here rather than an unknown
/// deferred to eval.
///
/// The `42702` for a two-claimant bare name is raised **here and in the executor**, from
/// two derivations of the same relation list, and with a matching row the executor's
/// fires whether or not this one exists — so removing this count leaves
/// `update/returning_from_source_column_ambiguous.sql` green. What this count is the
/// only verdict for is the **zero-row** statement, where the executor evaluates no
/// projection item at all: without it,
/// `UPDATE t SET … FROM s WHERE 1 = 0 RETURNING <ambiguous>` answers `(0 rows)` where
/// 18.6 says `42702` — measured under exactly that removal, and pinned by
/// `update/returning_from_source_column_ambiguous_no_match.sql`.
///
/// # Why the wildcard expands to *qualified* references
///
/// Postgres expands a `*` into per-relation qualified references (`ExpandSingleTable`),
/// and the qualifier is load-bearing *because* the expansion goes through the same
/// resolve-and-gate as a written item: expanded unqualified, `RETURNING <target>.*` over
/// a source carrying a same-named column would be re-checked as a list of bare names,
/// find two claimants, and answer `42702` — turning the spine's own `RETURNING o.*` into
/// an error. The bare `*` expands to every relation's columns in turn, each qualified by
/// that relation's name, which is what makes the duplicate names 18.6 emits
/// (`id | v | id | v | k`) reachable at all.
///
/// A `<q>.*` whose `q` names no relation in the namespace is left untouched and the
/// closed scope then rejects it (`42P01`); the executor's `project_returning` rejects it
/// a second time.
///
/// # What this clause still refuses that Postgres accepts
///
/// `RETURNING count(*)` — an aggregate in `RETURNING` is `42803` on 18.6 and pglet
/// answers row-count-dependently (`EDGES.md`). And a
/// `DELETE … USING` source is out of the namespace here only because the clause is a
/// parser gap; nothing in this function is `UPDATE`-specific.
///
/// It excludes `EXCLUDED` for a different reason — there pglet and Postgres agree.
/// Postgres puts that pseudo-relation in scope for the `ON CONFLICT DO UPDATE` action
/// only, so `RETURNING EXCLUDED.x` is `42P01` on both. And an `INSERT … SELECT`'s
/// source is genuinely not in Postgres's namespace either: measured on 18.6,
/// `INSERT INTO amb_t SELECT … FROM amb_s s RETURNING s.v` is `42P01` and
/// `RETURNING k` is `42703` — so `lower_insert` passes no source at all.
///
/// # Errors
///
/// `CatalogError::MissingFromClauseEntry` (`42P01`) for a qualifier the namespace
/// does not name, `Error::AmbiguousColumn` (`42702`) for a bare name two relations
/// claim, and `CatalogError::ColumnNotFoundInExpr` (`42703`) for a name none carries,
/// all via `ColumnScope::check_binding`; otherwise whatever
/// [`resolve_expr_types_gated`] raises.
fn lower_returning(
    returning: Vec<Expr>,
    relations: &RelationColumns,
    schema: &Schema,
) -> Result<Vec<Expr>> {
    let scope = returning_scope(relations);
    let mut out = Vec::with_capacity(returning.len());
    // One left-to-right pass over the items, expanding and resolving each before
    // looking at the next — `transformReturningClause` resolves in source order, so
    // `RETURNING nosuch, <target>.*` reports the `42703` on the first item and
    // `RETURNING <target>.nosuch, *` the one on its own. Two whole-list passes cannot
    // produce both.
    for e in returning {
        // A `<q>.*` whose qualifier names nothing falls through to the resolve-and-gate
        // below, which raises its `42P01`.
        let expanded = match returning_wildcard(&e) {
            Some(ReturningWildcard::Bare) => Some(relations.iter().collect::<Vec<_>>()),
            Some(ReturningWildcard::Qualified(q)) => {
                relations.lookup(q).map(|cols| vec![(q, cols)])
            }
            None => None,
        };
        match expanded {
            Some(items) => {
                for (relation, columns) in items {
                    for c in columns {
                        let item = Expr::Column {
                            table: Some(relation.to_string()),
                            name: c.name.clone(),
                        };
                        out.push(resolve_expr_types_gated(item, &scope, schema)?);
                    }
                }
            }
            None => out.push(resolve_expr_types_gated(e, &scope, schema)?),
        }
    }
    Ok(out)
}

/// A `RETURNING` wildcard's spelling — which relations of the namespace it expands to.
enum ReturningWildcard<'a> {
    /// A bare `*`: every relation in the namespace, in FROM order.
    Bare,
    /// A `q.*`: the one relation named `q`, or `42P01` when nothing answers to it.
    Qualified(&'a str),
}

/// Whether `e` is a `RETURNING` wildcard, and which spelling — `None` for anything else.
///
/// Expanding at plan time is what makes the plan-side column answer (`output_columns`,
/// the lossy read a `CteScan` capture takes, and `Plan::result_shape`, `Describe`'s
/// entry) and the executor's projection agree without either re-deriving the wildcard;
/// a bare `*` mixed with other items (`RETURNING *, <grant_count>`) expands in place,
/// the other items following.
///
/// Leaving a qualified star for the executor alone was what went wrong before: the
/// sole-`*` fast path there is reached ahead of every resolver, and it answered a
/// `RETURNING <any qualifier>.*` with the **target's** whole row — `UPDATE ra SET … FROM
/// rb … RETURNING rb.*` returned `ra`'s columns, names and values, where 18.6 returns
/// `rb`'s.
fn returning_wildcard(e: &Expr) -> Option<ReturningWildcard<'_>> {
    if e.is_bare_star() {
        return Some(ReturningWildcard::Bare);
    }
    match e {
        Expr::Column {
            table: Some(qualifier),
            name,
        } if name == "*" => Some(ReturningWildcard::Qualified(qualifier.as_str())),
        _ => None,
    }
}

/// Lower a parsed DELETE into a [`Plan::Delete`].
///
/// The native parser parses the WHERE into raw lowered IR and calls this, so WHERE
/// type-resolution has one implementation. `filter` is raw lowered IR
/// (pre-resolution); the table-column type context is applied here.
///
/// With no target relation ([`lowers_clauses`]) nothing is lowered and the executor's
/// `42P01` stands.
///
/// # Errors
///
/// A constant string cast in the WHERE that can't be coerced (`22P02` / `22007` / …),
/// or a comparison with no operator for its operand types (`42883`), fails loud at
/// plan time (see [`resolve_expr_types_gated`]).
pub fn lower_delete(
    table: String,
    filter: Option<Expr>,
    returning: Vec<Expr>,
    schema: &Schema,
) -> Result<Plan> {
    if !lowers_clauses(schema, &table) {
        return Ok(Plan::Delete {
            table,
            filter,
            target_probe: None,
            returning,
        });
    }
    let column_types = column_type_map_for(schema, &table);
    // The target table's own name is the qualifier, because pglet's parser accepts no
    // DELETE alias — not because Postgres has none. `DELETE FROM t AS d … RETURNING
    // d.a` (and the bare `DELETE FROM t d` spelling) both answer on 18.6 and are
    // `42601` here, a pre-existing parser gap; closing it means revisiting
    // `lower_returning`, whose namespace is the target's *effective* name.
    // The target alone is the whole namespace a `WHERE` here may name — there is no
    // `USING` clause in this parser and no enclosing level — so the scope closes and a
    // reference binding to nothing is an error at plan time. Without that the verdict was
    // row-count-dependent: `DELETE FROM t WHERE zzz.c = 1` over an **empty** table matched
    // nothing, evaluated nothing, and reported `DELETE 0` where 18.6 raises `42P01`.
    let mut scope = ColumnScope::single(table.clone(), column_types);
    if let Some(def) = schema.get_table(&table) {
        scope.mark_stored(def);
    }
    let scope = scope.closed();
    let filter = filter
        .map(|f| resolve_expr_types_gated(f, &scope, schema))
        .transpose()?;
    // Above the `RETURNING` items, where `transformDeleteStmt` puts it:
    // `DELETE FROM t WHERE k RETURNING nosuchcol` is `42804` on 18.6.
    if let Some(f) = &filter {
        check_boolean_clause(f, BoolContext::Where, &scope, schema)?;
    }
    // Expand `RETURNING *` / `RETURNING *, <expr>` to the target columns at plan
    // time, mirroring INSERT/UPDATE so the executor never sees a bare `*` mixed with
    // other items. `DELETE … USING` is a parser gap, so there is no source relation to
    // put in the namespace — 18.6 accepts the clause and its `RETURNING *` answers the
    // joined row, so closing that gap means passing the source here (see
    // `lower_returning`, which is not `UPDATE`-specific).
    let returning = lower_returning(
        returning,
        &returning_relations(&table, &table, None, schema),
        schema,
    )?;

    let mut plan = Plan::Delete {
        table,
        filter,
        target_probe: None,
        returning,
    };
    // See `lower_update` — a correlated subquery in the `WHERE` is re-resolved against
    // the target relation here.
    // A DML statement is always its own top level — there is no expression subquery it
    // could sit inside — so its chain is complete and the nested levels' gates may close.
    resolve_correlated_subplans(&mut plan, schema, true)?;
    // As in `lower_update`, from the `WHERE` as it will execute.
    if let Plan::Delete {
        table,
        filter: Some(filter),
        target_probe,
        ..
    } = &mut plan
    {
        *target_probe = dml_target_probe(table, None, filter, schema);
    }
    Ok(plan)
}
