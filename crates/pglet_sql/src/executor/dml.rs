//! DML (Data Manipulation Language) execution.
//!
//! Handles INSERT, UPDATE, DELETE operations.
//!
//! # Architecture
//!
//! The executor handles predicate evaluation and calls storage with specific
//! row IDs. This separation enables:
//! - Efficient WAL logging (log row ID instead of full row values)
//! - Clean abstraction (storage doesn't need to understand predicates)
//! - Foundation for indexing (indexes return row IDs)

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use crate::builtins::{Clock, timestamp};
use crate::catalog::{
    Column, ColumnDefault, ForeignKey, ReferentialAction, Schema, Sequence, Table,
};
use crate::planner::{
    BoolContext, Expr, InsertRows, OnConflictAction, OnConflictClause, Plan, SequenceFunc,
};
use crate::types::{ColumnDesc, TypeOid, Value};
use crate::{Error, Result};
use pglet_core::{FkDirection, ForeignKeyViolationDetail, KeyDescription};

use super::Storage;
use super::constraints::{
    ActionRows, ArbiterConflict, PreparedIndex, TargetLessConflict, coerce_column_value,
    enforce_cascaded_index_entries, enforce_checks, enforce_foreign_key, enforce_foreign_keys,
    enforce_index_entries, enforce_not_null_row, enforce_row_checks, enforce_row_constraints,
    fk_child_positions, match_conflict_index, parent_tuple_exists, prepare_checks, prepare_indexes,
    probe_conflict, probe_target_less_conflict, validate_column,
};
use super::expr::{
    ColumnResolver, EvalEnv, JoinColumnMap, OuterFrame, ScopeIdentity, SimpleColumns, SubqueryExec,
    eval_expr, eval_expr_env, eval_expr_full, eval_expr_join, predicate_holds, values_equal,
};
use super::helpers::{finish_returning, get_table_or_err, table_column_descs};
use super::query::{eval_expr_subquery, execute_plan_node, get_table_name_from_plan};
use super::sequence::{SequenceSession, eval_sequence_fn};
use super::types::{QueryResult, RowId};

/// Resolve a column's `DEFAULT` to a value (used for omitted columns and the
/// explicit `DEFAULT` keyword). Mutates sequence state for `SERIAL`/`nextval`.
///
/// A `NextVal` default naming a sequence the catalog does not hold **fails loud**
/// (`42P01`), the SQLSTATE Postgres gives `nextval` on a name no relation holds.
/// Minting a fresh counter at 1 instead — which this once did — is the silent-wrong
/// shape the catalog is built to make impossible: it hands a second table's rows ids
/// that restart from 1 on top of the ones already there, and nothing in the read path
/// can tell. Postgres cannot reach the state at all (a cooked default records a
/// `pg_attrdef` → sequence dependency, so the sequence cannot be dropped out from
/// under it), and neither can pglet — `executor::dependencies` re-derives that edge
/// for `DROP TABLE` / `DROP COLUMN` and `execute_drop_sequence` refuses it directly.
/// The guard stands for the case where one of those routes is incomplete: it converts
/// a silent renumbering into an error naming the missing sequence.
///
/// # Errors
///
/// [`Error::SequenceNotFound`] (`42P01`) for an unregistered `NextVal` sequence, or a
/// builtin's error (e.g. `now()`/`gen_random_uuid()` on wasm).
pub(super) fn resolve_default(
    col: &Column,
    sequences: &mut BTreeMap<String, Sequence>,
) -> Result<Value> {
    let value = match &col.default {
        None => Value::Null,
        Some(ColumnDefault::Literal(v)) => v.clone(),
        // The clock family reads the clock the same spelling reads as an expression
        // (`executor::expr::eval::eval_builtin`), so a `DEFAULT now()` and a `now()` the
        // statement writes beside it are one instant.
        Some(
            default @ (ColumnDefault::Now
            | ColumnDefault::TransactionTimestamp
            | ColumnDefault::StatementTimestamp
            | ColumnDefault::ClockTimestamp
            | ColumnDefault::CurrentTimestamp),
        ) => timestamp(match default {
            ColumnDefault::StatementTimestamp => Clock::Statement,
            ColumnDefault::ClockTimestamp => Clock::Wall,
            _ => Clock::Transaction,
        })?,
        Some(ColumnDefault::CurrentDate) => crate::builtins::current_date()?,
        Some(ColumnDefault::CurrentTime) => crate::builtins::current_time()?,
        Some(ColumnDefault::RandomUuid) => crate::builtins::random_uuid()?,
        Some(ColumnDefault::NextVal(seq)) => draw_nextval(seq, sequences)?,
        Some(ColumnDefault::Expr(expr)) => {
            eval_default_expr(expr, sequences, SequenceDraw::Advance)?
        }
    };
    coerce_column_value(value, col)
}

/// Advance `seq`'s counter and take the value — the one place a `DEFAULT`'s
/// `nextval` draw happens, shared by the closed [`ColumnDefault::NextVal`] arm and
/// the `nextval`s inside a [`ColumnDefault::Expr`].
///
/// **It is a sequences-map-only lookup, so a name a *table* or *index* holds is
/// `42P01` here where `init_sequence` answers `42809`** — the shape `docs/names.md` names
/// as the trap this area springs, and it is unclosed rather than unreachable. The
/// three DDL registration sites gate the kind (`ensure_sequence_relation`), so every
/// route the engine's own accepted DDL offers is refused before a default naming a
/// wrong-kind relation can be persisted; this arm stands for one of those being
/// incomplete, exactly as its `SequenceNotFound` neighbour above does. It is not
/// fixed here because `relation_kind` is not reachable from this signature: the
/// INSERT, `COPY` and UPDATE callers pass `&mut schema.sequences` outright (the
/// `DO UPDATE` one by pass-through), and `execute_alter_table_add_column`'s two —
/// the uniform value and the per-row backfill — pass a **staged** map while `schema`
/// is mutably borrowed, so answering the right SQLSTATE needs the map replaced by a
/// resolver that carries the catalog — a change to `resolve_default`'s
/// contract rather than to this function. Recorded in `EDGES.md`'s row 47.
fn draw_nextval(seq: &str, sequences: &mut BTreeMap<String, Sequence>) -> Result<Value> {
    let sequence = sequences
        .get_mut(seq)
        .ok_or_else(|| Error::SequenceNotFound(seq.to_string()))?;
    sequence.last_value += 1;
    Ok(Value::Int8(sequence.last_value))
}

/// Whether a `nextval` inside a `DEFAULT` expression advances its sequence or stands
/// in for one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SequenceDraw {
    /// DDL time: substitute a representative value and leave every counter alone.
    /// Advancing there would issue an id no row will ever hold — on a statement that
    /// may still be refused, and against a counter `nextval`'s non-transactionality
    /// forbids walking back.
    Probe,
    /// Row time: advance the sequence and take the value, once per row.
    Advance,
}

/// Evaluate a [`ColumnDefault::Expr`] to its value.
///
/// A `DEFAULT` expression is **closed** — `executor::ddl`'s `check_default_expr_nodes`
/// refuses every column reference and every `$N` at DDL time — so it evaluates
/// against no row and no parameter list, which is why the column/type/row slices
/// handed to [`eval_expr`] are empty rather than the write's.
///
/// A sequence function is substituted **before** evaluation rather than handled
/// inside it, for two reasons that pull the same way: `eval_expr` deliberately
/// refuses a nested [`Expr::SequenceFn`] (pglet's read paths are side-effect-free,
/// so a sequence draw is a statement-level act), and the substitution is what lets
/// the same tree be evaluated once per row with a fresh draw each time — 18.6's
/// behaviour, measured: two `INSERT … DEFAULT VALUES` over
/// `a INT DEFAULT nextval('s') + 1` answer `2` then `3`.
///
/// **Its cost is the third fact about substitute-first, and it is not implied by the
/// two above: the walk is unconditional, so a `nextval` in a branch the evaluation
/// never takes is drawn anyway.** [`substitute_nextval`] rewrites the whole tree
/// before [`eval_expr`] sees it, and `eval_expr` is the only thing that knows which
/// `CASE` arm or `coalesce` argument is live. Measured against 18.6 over
/// `a INT DEFAULT CASE WHEN false THEN nextval('s') ELSE 0 END`: two inserts leave the
/// counter at `1` there (nothing drew) and at `2` here, so the next `nextval('s')`
/// answers `1` there and `3` here; `coalesce(1, nextval('s'))` is the same shape at
/// one draw. **The stored values agree** — `0` and `1` on both engines — so nothing a
/// row comparison can see is wrong; only the counter diverges, and only where an
/// unconditional walk and a lazy evaluation disagree about what runs. Closing it means
/// drawing lazily *inside* `eval_expr` (a statement-level sequence session threaded
/// through the evaluator), which is separate work; recorded in `EDGES.md`'s row 47.
///
/// Only `nextval` is substituted. `currval`/`setval` in a default reach
/// `eval_expr`'s refusal and so are `0A000` at DDL time — a pglet-only
/// over-rejection of two spellings 18.6 accepts, recorded in `EDGES.md`. Refusing them at DDL is
/// the safe direction: accepting would
/// record a default every write then failed on.
pub(super) fn eval_default_expr(
    expr: &Expr,
    sequences: &mut BTreeMap<String, Sequence>,
    draw: SequenceDraw,
) -> Result<Value> {
    let drawn = substitute_nextval(expr, sequences, draw)?;
    eval_expr(&drawn, &[], &[], &[], &[])
}

/// Replace every bare `nextval('s')` in `expr` with the constant it resolves to for
/// this row (or, at DDL time, with a stand-in), leaving every other node alone.
fn substitute_nextval(
    expr: &Expr,
    sequences: &mut BTreeMap<String, Sequence>,
    draw: SequenceDraw,
) -> Result<Expr> {
    crate::depth::check_stack()?;
    if let Expr::SequenceFn {
        func: SequenceFunc::Nextval,
        seq,
        args,
    } = expr
        && args.is_empty()
    {
        return Ok(Expr::Bound(match draw {
            // The probe must not move a counter, and it must not depend on one
            // existing either: a `CREATE TABLE` cooking `DEFAULT nextval('s') + 1`
            // registers `s` further down the same statement.
            SequenceDraw::Probe => Value::Int8(1),
            SequenceDraw::Advance => draw_nextval(seq, sequences)?,
        }));
    }
    let mut error = None;
    let substituted =
        expr.clone().map_children(
            &mut |child| match substitute_nextval(&child, sequences, draw) {
                Ok(replacement) => replacement,
                Err(e) => {
                    error.get_or_insert(e);
                    child
                }
            },
        );
    error.map_or(Ok(substituted), Err)
}

/// Collect `table`'s rows whose values match `filter`, evaluated with full subquery
/// support — a `WHERE … IN (SELECT …)` resolves against `schema` / `storage` exactly like
/// the SELECT `Plan::Filter` path (the spine's eviction `DELETE … WHERE id IN (SELECT …
/// ORDER BY … OFFSET $n)`). A `None` filter matches every row, and an eval error
/// **propagates** (correct-or-loud — never silently "no row matched").
///
/// **The candidates come from `target_probe` when the planner chose one and it
/// answers** — the read path's own equality probe, chosen at plan time (`Plan::Delete`'s
/// field), so a write keyed by an indexed equality reads the rows its key names rather
/// than the table. A range or containment `WHERE` takes no probe (`dml_target_probe`
/// says why) and scans.
/// Otherwise — no probe, or a probe declined at execution (`probe_dml_targets` says when)
/// — they are every row of a scan. Either way each candidate is tested against the whole
/// `filter`, unchanged, and the candidates are visited in **`RowId` order**, the order the
/// scan returns them in: it is the order the writes happen in, so it decides `RETURNING`'s
/// row order, the sequence values a `SET … = DEFAULT` draws, which row's unique or
/// foreign-key refusal is the one raised, and the order a cascade is seeded in.
///
/// `table_alias` is the effective name for alias-aware qualified-column resolution —
/// Postgres's RTE `eref`: for UPDATE the alias when one is written and the table's own
/// name when none is (`alias.unwrap_or(table)`), for DELETE the table name, since the
/// parser has no `DELETE FROM t [AS] x`. Its two call sites are `collect_updated_simple`
/// and `execute_delete`, and **both pass `Some(…)`** — so the `None` arm, which resolves
/// bare names only and whose cost is the write recorded in
/// `EDGES.md` (*Qualifier-blind name resolution*), is not reachable from either. It is still an
/// `Option` because that is what `SimpleColumns::with_table` and the correlated frame
/// take; narrowing it to `&str` here would be an engine change and is not one this doc
/// makes.
///
/// Reads only — `storage` is borrowed shared, and callers mutate after collecting the
/// targets, so the returned `RowId`s stay valid and no write is seen by the collection.
#[allow(clippy::too_many_arguments)]
fn scan_matching_rows<S: Storage>(
    storage: &S,
    table: &str,
    col_names: &[&str],
    table_alias: Option<&str>,
    filter: Option<&Expr>,
    target_probe: Option<&Plan>,
    schema: &Schema,
    env: EvalEnv<'_>,
) -> Result<Vec<(RowId, Vec<Value>)>> {
    // Declared column types of the scanned table, so a `WHERE` predicate that
    // stringifies a `timestamptz` (`'v=' || ts`, `ts::text`) keeps its `+00` offset.
    let col_types: Vec<TypeOid> = schema
        .get_table(table)
        .map(|t| t.columns.iter().map(|c| c.type_oid).collect())
        .unwrap_or_default();
    // ...and their enum member orders, so a `WHERE m > 'sad'` over an enum column
    // matches the rows 18.6 matches (member position, not label bytes).
    let col_enums: Vec<Option<&[String]>> = schema
        .get_table(table)
        .map(|t| {
            t.columns
                .iter()
                .map(|c| super::helpers::enum_order_of(&schema.enums, c.enum_type.as_deref()))
                .collect()
        })
        .unwrap_or_default();
    #[cfg(feature = "index_acceleration")]
    let probed = match (target_probe, filter) {
        (Some(probe), Some(predicate)) => super::query::probe_dml_targets(
            probe,
            predicate,
            get_table_or_err(schema, table)?,
            storage,
            env,
        )?,
        _ => None,
    };
    // The stripped planner never builds a probe.
    #[cfg(not(feature = "index_acceleration"))]
    let probed: Option<Vec<RowId>> = {
        let _ = target_probe;
        None
    };
    let candidates = match probed {
        Some(ids) => {
            let mut rows = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(row) = storage.get(table, id)? {
                    rows.push((id, row));
                }
            }
            rows
        }
        None => storage.scan(table)?,
    };
    let mut matching = Vec::new();
    for (row_id, row) in candidates {
        let keep = match filter {
            None => true,
            Some(predicate) => predicate_holds(
                &eval_expr_subquery(
                    predicate,
                    col_names,
                    &col_types,
                    &col_enums,
                    ScopeIdentity::from_table(table_alias),
                    &row,
                    schema,
                    storage,
                    env,
                )?,
                BoolContext::Where,
            )?,
        };
        if keep {
            matching.push((row_id, row));
        }
    }
    Ok(matching)
}

/// Execute INSERT (both the `VALUES` form and `INSERT … SELECT`).
///
/// For `InsertRows::Select`, the source query is executed first (a read against
/// the current snapshot), then its rows are lowered to full-width `Expr` rows —
/// mapped target columns carry the produced value as `Expr::Bound`, every other
/// column is `Expr::Default` — so they flow through the **same** per-row insert
/// loop as the `Values` form (DEFAULT-fill, enum validation, the row-scoped
/// NOT NULL/CHECK pair, ON CONFLICT, RETURNING all run unchanged).
#[allow(clippy::too_many_arguments)]
pub fn execute_insert<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    table: &str,
    rows: &InsertRows,
    returning: &[Expr],
    on_conflict: Option<&OnConflictClause>,
    env: EvalEnv<'_>,
    session: &mut SequenceSession,
) -> Result<QueryResult> {
    // Own the table definition so `schema.sequences` can be mutated while
    // resolving SERIAL/nextval defaults below (the borrow would otherwise clash).
    let table_def = get_table_or_err(schema, table)?.clone();

    // `INSERT … SELECT`: run the source query (a read against the current snapshot)
    // and lower its rows to full-width `Expr` rows. `Expr::Bound` carries each
    // produced value through the shared loop below exactly like a `VALUES` literal.
    let source_rows: Vec<Vec<Expr>>;
    let values: &[Vec<Expr>] = match rows {
        InsertRows::Values { rows, .. } => rows,
        InsertRows::Select { source, columns } => {
            let (rows, _cols) = execute_plan_node(source, schema, storage, env)?;
            let n_cols = table_def.columns.len();
            source_rows = rows
                .into_iter()
                .map(|src_row| {
                    let mut full: Vec<Expr> = vec![Expr::Default; n_cols];
                    for (src_idx, value) in src_row.into_iter().enumerate() {
                        if let Some(&target_idx) = columns.get(src_idx)
                            && target_idx < n_cols
                        {
                            full[target_idx] = Expr::Bound(value);
                        }
                    }
                    full
                })
                .collect();
            &source_rows
        }
    };

    let col_names: Vec<&str> = table_def.columns.iter().map(|c| c.name.as_str()).collect();
    // The parallel declared types — the scope a CHECK / SET-RHS / RETURNING
    // expression binds against (see `ColumnResolver::column_type`).
    let col_types: Vec<TypeOid> = table_def.columns.iter().map(|c| c.type_oid).collect();

    // Snapshot the enum members so member validation reads them while
    // `schema.sequences` is mutated for SERIAL/nextval defaults below.
    let enums = schema.enums.clone();
    // ...and each column's enum member order off that same snapshot, so a
    // `CHECK (m < 'ok')` keeps and refuses the rows 18.6 keeps and refuses (see
    // `enforce_checks`). Off the snapshot for the borrow reason above, not a second
    // opinion: it is the same map, and `CREATE TYPE` cannot run inside this statement.
    let col_enums = super::helpers::table_enum_orders(&table_def.columns, &enums);

    // Parse the table's CHECK predicates once per statement (not per row).
    let checks = prepare_checks(&table_def);
    // Prepare the table's indexes once per statement too (`enforce_index_entries`
    // runs per row; this hoists its predicate/expression-key parse out of the loop).
    let indexes = prepare_indexes(schema, table);

    // Only accumulate rows if RETURNING clause is present (saves memory for large inserts)
    let has_returning = !returning.is_empty();
    let mut result_rows: Vec<Vec<Value>> = Vec::new();
    let mut inserted_count = 0usize;

    // Resolve the ON CONFLICT arbiter index once (a *target-ful* clause only — its
    // target is row-independent). A target-less `ON CONFLICT [DO NOTHING]` has no
    // single arbiter; it is handled by the unique-violation fall-through below.
    // A target that matches no unique index fails loud here (SQLSTATE 42P10).
    let arbiter = match on_conflict.and_then(|oc| oc.target.as_ref()) {
        Some(target) => Some(match_conflict_index(schema, table, target)?),
        None => None,
    };

    // The foreign-key triggers of the rows a `DO UPDATE` statement writes. 18.6 queues each
    // row's — an inserted row's own key checks; a rewritten row's actions, then its own key
    // checks — as after-row triggers and fires them once the statement's plan has run, so
    // they are queued in row order and fired after the loop and after `RETURNING`
    // ([`FkTriggerQueue`]) — which is what lets a `RETURNING` error and a later row's own
    // `23514` / `23502` / `23505` outrank an earlier row's `23503` / `23001`, a row this
    // statement inserts supply a key a rewrite vacated (`NO ACTION`'s substitute), and a
    // row's key be checked against the actions queued before it. Each row's own keys are
    // also checked as it is written (`EDGES.md` row 166). Collected only for a `DO UPDATE`
    // over a table some foreign key references: without an action to run, the queued checks
    // would find what the written row's own did.
    let fk_queued = on_conflict
        .is_some_and(|oc| matches!(oc.action, OnConflictAction::DoUpdate { .. }))
        && !referencing_fks_on_update(schema, table).is_empty();
    let mut queued_rows: Vec<UpsertWrite> = Vec::new();
    // The rows a `DO UPDATE` statement has inserted or rewritten so far, by identity: a
    // conflict probe that lands on one of them is 18.6's `21000`
    // ([`Error::OnConflictRowAffectedTwice`]). Identity, not the key — a rewrite can move a
    // row onto a key a later proposal names, or off the key it was found by — and only this
    // statement's writes: a row an earlier statement of the same block wrote is updated as
    // any other. Kept only for a `DO UPDATE`: `DO NOTHING` skips a conflicting row whoever
    // wrote it, as 18.6 does.
    let mut written: Option<HashSet<RowId>> = on_conflict
        .is_some_and(|oc| matches!(oc.action, OnConflictAction::DoUpdate { .. }))
        .then(HashSet::new);

    let default_expr = Expr::Default;
    for row_exprs in values {
        // Build a full-width row in table-column order. `lower_insert` already
        // expanded omitted columns to `Expr::Default`; resolve those (and any
        // explicit `DEFAULT` keyword) against the column's catalog default.
        let mut row: Vec<Value> = Vec::with_capacity(table_def.columns.len());
        for (i, col) in table_def.columns.iter().enumerate() {
            let expr = row_exprs.get(i).unwrap_or(&default_expr);
            // Peel the planner's column-coercion `Cast` wrapper (added by
            // `coerce_to_type`) to reach a `nextval`/`setval` in a VALUES entry — the
            // trailing `coerce_column_value` re-applies the column type, so dropping
            // the intermediate cast is safe. A sequence function nested inside a
            // larger value expression is not top-level and fails loud in `eval_expr_env`.
            let seq_expr = match expr {
                Expr::Cast { expr: inner, .. } => inner.as_ref(),
                other => other,
            };
            let val = if matches!(expr, Expr::Default) {
                resolve_default(col, &mut schema.sequences)?
            } else if let Some((func, seq, seq_args)) = seq_expr.as_sequence_fn() {
                coerce_column_value(
                    eval_sequence_fn(func, seq, seq_args, schema, session, env)?,
                    col,
                )?
            } else {
                coerce_column_value(eval_expr_env(expr, env.with_schema(&*schema))?, col)?
            };
            // Enum membership on the resolved value. NOT NULL is row-scoped and runs
            // after the loop (`enforce_not_null_row`), where the whole tuple exists —
            // it still catches both a supplied NULL and an omitted column with no
            // default, which resolves to `Null` here.
            validate_column(table, col, &val, &enums)?;
            row.push(val);
        }

        // NOT NULL then row-local CHECK over the fully-assembled row, before any ON
        // CONFLICT branch (a plain insert and the upsert's pre-image both flow through
        // here). Both are row-scoped because 18.6 constrains a *formed* tuple —
        // `enforce_not_null_row`'s doc has the order this restores.
        enforce_not_null_row(table, &table_def.columns, &col_types, &row)?;
        enforce_checks(table, &checks, &col_names, &col_types, &col_enums, &row)?;

        // ON CONFLICT (target): probe the arbiter index for what holds the row's key.
        // (A target-less clause has no `arbiter` — its conflicts surface through the
        // unique-violation fall-through below.)
        //
        // Only `Row` is a conflict an action can act on. `ClaimedElsewhere` — another
        // connection's uncommitted write holds the key — is a row neither action can
        // reach: `DO UPDATE` has nothing to update, and `DO NOTHING` has nothing to
        // skip past, because the claiming write can still roll back and then 18.6
        // writes the row a skip would have silently dropped. Both fall through to the
        // shared enforcement below and take the same `23505` a committed holder would
        // — one error construction, so a claim-caused refusal is indistinguishable
        // from an ordinary one at the wire.
        if let (Some(on_conflict_clause), Some(arbiter)) = (on_conflict, &arbiter)
            && let ArbiterConflict::Row(conflict_id) = probe_conflict(storage, arbiter, &row)?
        {
            // The probe hit a stored key; fetch the row it belongs to. A miss
            // here (storage out of sync) just falls through to a normal insert.
            if let Some(existing_row) = storage.get(table, conflict_id)? {
                match &on_conflict_clause.action {
                    OnConflictAction::DoNothing => {
                        // Skip this row, don't insert.
                        continue;
                    }
                    OnConflictAction::DoUpdate {
                        assignments,
                        where_predicate,
                    } => {
                        // A row this statement already wrote. 18.6 refuses it where it
                        // locks the conflicting row, ahead of the action's `WHERE` — so a
                        // proposal whose `WHERE` is false is refused too.
                        if written.as_ref().is_some_and(|w| w.contains(&conflict_id)) {
                            return Err(Error::OnConflictRowAffectedTwice);
                        }
                        // DO UPDATE … WHERE: when it isn't true against the
                        // existing row (with EXCLUDED = the would-be-inserted
                        // row), leave the row unchanged and don't insert.
                        if let Some(pred) = where_predicate {
                            let keep = eval_do_update(
                                pred,
                                table,
                                &col_names,
                                &col_types,
                                &col_enums,
                                &existing_row,
                                &row,
                                env.with_schema(&*schema),
                            )?;
                            if !predicate_holds(&keep, BoolContext::Where)? {
                                continue;
                            }
                        }

                        // Update the existing row instead of inserting.
                        let updated_row = apply_do_update(
                            &existing_row,
                            &row,
                            assignments,
                            &col_names,
                            &table_def,
                            schema,
                            env,
                        )?;
                        let old_row = fk_queued.then_some(existing_row);

                        // Re-enforce constraints on the post-update row (DO UPDATE
                        // can write a non-member enum, a NULL, or break a CHECK the
                        // insert path didn't).
                        for (col, val) in table_def.columns.iter().zip(&updated_row) {
                            validate_column(table, col, val, &enums)?;
                        }
                        // CHECK + unique (excluding this row's own id, since the
                        // DO UPDATE can move it onto another row's key) + FK on the
                        // post-update row (DO UPDATE can break any of the three).
                        enforce_row_constraints(
                            storage,
                            schema,
                            table,
                            &table_def.columns,
                            &checks,
                            &col_names,
                            &col_types,
                            &indexes,
                            &updated_row,
                            Some(conflict_id),
                        )?;

                        // Update in storage by ID.
                        storage.update(table, conflict_id, updated_row.clone())?;
                        if let Some(written) = &mut written {
                            written.insert(conflict_id);
                        }
                        if let Some(old_row) = old_row {
                            queued_rows.push(UpsertWrite::Rewritten((
                                conflict_id,
                                old_row,
                                updated_row.clone(),
                            )));
                        }

                        if has_returning {
                            result_rows.push(updated_row);
                        }
                        inserted_count += 1;
                        continue;
                    }
                }
            }
        }

        // No conflict or no ON CONFLICT clause - normal insert. Enter the row into
        // every index first — its size refusals and its unique-index conflicts, index by
        // index in creation order (`enforce_index_entries`). An INSERT has no id yet,
        // so any existing holder of the key conflicts (`None`). Probing before
        // `storage.insert` and inserting row-by-row makes an intra-statement duplicate
        // (a second VALUES row with the same key) visible to the next row's probe via
        // read-your-own-writes.
        //
        // A *target-less* `ON CONFLICT DO NOTHING` arbitrates against **every**
        // unique / PK constraint (an explicit `ON CONFLICT (cols)` target was already
        // handled above via `probe_conflict`), and 18.6 asks that question of the whole
        // set **before** it enters the row into any index — so a readable conflict
        // skips the row even where an earlier index would have refused the row's size.
        // What it does is a property of the whole prepared set and not of the one
        // index `enforce_index_entries` stopped at — that index may hold a readable row
        // while a *different* one merely carries a claim, or the reverse.
        // `probe_target_less_conflict` re-asks the whole set: a readable row anywhere
        // already makes the insert a no-op under either outcome of every claim, so the
        // row is skipped, and only a claim that is the sole occupancy refuses.
        // Re-asking is off the uncontended path: only a row that already met a refusal
        // reaches it.
        let target_less_do_nothing = matches!(
            on_conflict,
            Some(oc) if oc.target.is_none()
                && matches!(oc.action, OnConflictAction::DoNothing)
        );
        match enforce_index_entries(storage, &indexes, &row, None) {
            Ok(()) => {}
            Err(
                e @ (Error::UniqueViolation { .. }
                | Error::BtreeIndexRowTooLarge(_)
                | Error::GinIndexRowTooLarge(_)
                | Error::IndexRowRequiresTooMuch { .. }),
            ) if target_less_do_nothing => {
                match probe_target_less_conflict(storage, &indexes, &row)? {
                    TargetLessConflict::Row => continue,
                    // The skip is licensed by a readable row and by nothing else, so a
                    // claim-only occupancy — and an occupancy that has since gone —
                    // both take the refusal already built. Where a claim is the reason,
                    // that error names a claim-only index: no index carries a readable
                    // row, so the one enforcement stopped at cannot either.
                    TargetLessConflict::ClaimedElsewhere | TargetLessConflict::Free => {
                        return Err(e);
                    }
                }
            }
            Err(e) => return Err(e),
        }
        // FK existence: every non-NULL referencing value must have a parent.
        enforce_foreign_keys(storage, schema, table, &row)?;

        let id = storage.insert(table, row.clone())?;
        if let Some(written) = &mut written {
            written.insert(id);
        }
        if fk_queued {
            queued_rows.push(UpsertWrite::Inserted(id));
        }
        inserted_count += 1;

        if has_returning {
            result_rows.push(row);
        }
    }

    // `RETURNING` is projected first: 18.6 projects it per row inside the statement and
    // runs the queued actions after, so a `RETURNING` subquery reads the children as the
    // actions have not yet moved them, and a `RETURNING` error outranks an action's.
    let result = finish_returning(
        &result_rows,
        &[(table, &table_column_descs(&table_def))],
        returning,
        schema,
        storage,
        env,
        format!("INSERT 0 {inserted_count}"),
    )?;
    let mut fk_queue = FkTriggerQueue::new(schema);
    for write in &queued_rows {
        match write {
            UpsertWrite::Rewritten((id, old_row, new_row)) => {
                fk_queue.queue_statement_row(table, *id, old_row, new_row);
            }
            UpsertWrite::Inserted(id) => fk_queue.queue_inserted_row(table, *id, storage)?,
        }
    }
    fk_queue.fire(storage)?;
    Ok(result)
}

/// A row an `ON CONFLICT DO UPDATE` statement wrote, kept in the order it wrote them for
/// its foreign-key queue ([`FkTriggerQueue`]).
enum UpsertWrite {
    /// A row it inserted.
    Inserted(RowId),
    /// A row its `DO UPDATE` rewrote, `(id, old image, new image)`.
    Rewritten(UpdatedRow),
}

/// The relation alias `EXCLUDED.col` references inside a DO UPDATE — the
/// would-be-inserted row. Bound as an outer frame so a reference resolves at any
/// nesting depth (see [`eval_do_update`]).
const EXCLUDED_ALIAS: &str = "excluded";

/// Evaluate a DO UPDATE expression (an assignment RHS or the `WHERE` filter)
/// against the existing (conflicting) row, with `EXCLUDED` bound to the
/// would-be-inserted row.
///
/// The inner scope claims the target `table`, so a bare `col` or a `table.col`
/// reference reads the existing row; an `excluded`-named outer frame resolves
/// `EXCLUDED.col` — at any nesting depth (e.g. `t.x IS DISTINCT FROM EXCLUDED.x`). This
/// replaces the former top-level-only EXCLUDED special-case, which silently failed on a
/// nested `EXCLUDED.col`.
///
/// The unquoted spelling matches because the lexer folds it, not because the lookup is
/// loose: the frame's identity is `excluded`, `EXCLUDED` folds to it, and a **quoted**
/// `"EXCLUDED"` names no relation and is `42P01` — as on 18.6. A qualifier naming neither
/// the target nor `excluded` is `42P01` too, and it used to store the excluded value.
#[allow(clippy::too_many_arguments)]
fn eval_do_update(
    expr: &Expr,
    table: &str,
    col_names: &[&str],
    col_types: &[TypeOid],
    col_enums: &[Option<&[String]>],
    existing_row: &[Value],
    excluded_row: &[Value],
    env: EvalEnv<'_>,
) -> Result<Value> {
    // The frame carries the target's declared column types too (`EXCLUDED` shares
    // the target's shape), so `SET s = 'v=' || EXCLUDED.tz` stringifies the
    // `timestamptz` with its `+00` offset — the same rendering `uictz.tz` on the
    // existing-row side already got. Without them the upsert path *persisted* an
    // offset-less string.
    let excluded = OuterFrame::new(
        col_names,
        col_types,
        col_enums,
        Some(EXCLUDED_ALIAS),
        excluded_row,
        env.outer_scope(),
    );
    let env = env.push(&excluded);
    eval_expr_full(
        expr,
        &SimpleColumns::with_table(col_names, Some(table))
            .with_types(col_types)
            .with_enum_orders(col_enums),
        existing_row,
        SubqueryExec::NONE,
        env,
    )
}

/// Assemble the row a DO UPDATE writes over the existing (conflicting) row.
///
/// Every right-hand side reads `existing_row` — the conflicting row the arbiter probe
/// found, before any of this row's own assignments — plus `EXCLUDED.col` (the
/// would-be-inserted row), and never an earlier assignment's result; the new row is a
/// copy of `existing_row` with each assigned value written in. The rule and the
/// column-order evaluation are [`assign_row`]'s, and hold here for the same reason:
/// 18.6 plans a `DO UPDATE SET` list through the same `rewriteTargetListIU` and
/// projects it from one input tuple.
///
/// `existing_row` is never a row this statement wrote: a probe landing on one is refused
/// `21000` ahead of the action, as 18.6 refuses it ([`Error::OnConflictRowAffectedTwice`]).
///
/// A `SET col = DEFAULT` resolves to the column's catalog default (mirroring
/// INSERT/UPDATE, advancing a SERIAL/nextval sequence in `sequences`); every
/// other RHS is evaluated through [`eval_do_update`].
fn apply_do_update(
    existing_row: &[Value],
    excluded_row: &[Value],
    assignments: &[(String, Expr)],
    col_names: &[&str],
    table_def: &Table,
    schema: &mut Schema,
    env: EvalEnv<'_>,
) -> Result<Vec<Value>> {
    // The declared types the SET-RHS binds against, so `SET s = 'v=' || tz_col`
    // stores the `+00`-bearing text Postgres stores (see `enforce_checks`).
    let col_types: Vec<TypeOid> = table_def.columns.iter().map(|c| c.type_oid).collect();
    // ...and their enum member orders, so a `SET x = (m > 'sad')` reads the same order
    // the `WHERE` and the CHECK do. Off a snapshot for the same borrow reason as the
    // INSERT path's: `schema` is mutated below for a `SET col = DEFAULT`.
    let enums = schema.enums.clone();
    let col_enums = super::helpers::table_enum_orders(&table_def.columns, &enums);
    // Every target maps to its column position before any right-hand side runs. A
    // target the relation lacks is refused earlier, at plan time, by `lower_on_conflict`
    // (`42703`, as 18.6 refuses it in parse analysis — measured over a `"Val"` column,
    // `… DO UPDATE SET val = 'z'` is `42703` on both engines whether or not the row
    // conflicts; the conflicting spelling is
    // `tests/fixtures/identifier/on_conflict_set_target_is_exact_loud.sql`). The
    // comparison is exact: the assignment's target name and the catalog column name both
    // came through the parser's name fold.
    let targets = assignments
        .iter()
        .map(|(col_name, expr)| {
            let idx = col_names
                .iter()
                .position(|n| *n == col_name)
                .ok_or_else(|| crate::CatalogError::ColumnNotFound {
                    table: table_def.name.clone(),
                    column: col_name.clone(),
                })?;
            Ok((idx, expr))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut new_row = existing_row.to_vec();
    // Column order, not written order — see `assign_row`.
    for (idx, expr) in in_column_order(&targets, col_names.len()) {
        new_row[idx] = if matches!(expr, Expr::Default) {
            resolve_default(&table_def.columns[idx], &mut schema.sequences)?
        } else {
            coerce_column_value(
                eval_do_update(
                    expr,
                    &table_def.name,
                    col_names,
                    &col_types,
                    &col_enums,
                    existing_row,
                    excluded_row,
                    // Lent per evaluation — see `assign_row` for why it cannot be lent
                    // once for the function.
                    env.with_schema(&*schema),
                )?,
                &table_def.columns[idx],
            )?
        };
    }
    Ok(new_row)
}

/// A `SET` list's resolved `(column position, right-hand side)` pairs in the order 18.6
/// evaluates them: column order, which its rewriter rebuilds the list in
/// (`rewriteTargetListIU`), and written order among repeats of one column, which the
/// plan-time duplicate gate refuses where it runs. A scan per column rather than a sort:
/// measured on the wasm build, a stable sort at its two call sites costs more than eight
/// kilobytes.
fn in_column_order<'a>(
    targets: &'a [(usize, &'a Expr)],
    width: usize,
) -> impl Iterator<Item = (usize, &'a Expr)> {
    (0..width).flat_map(move |pos| targets.iter().copied().filter(move |(idx, _)| *idx == pos))
}

/// Execute UPDATE, with optional target alias and `FROM` source.
///
/// Without `from`, each matching target row is updated in place (the simple
/// path). With `from`, the target is inner-joined to the `from` plan and each
/// matching `(target, from)` pair drives an update; a target row is updated at
/// most once (the first matching `from` row). In both cases `SET`, `WHERE` **and**
/// `RETURNING` read the one namespace the statement's `FROM` clause defines: the
/// target under its effective name plus the source under its own, so `RETURNING s.k`
/// answers the source's column exactly as `WHERE s.k = 1` filters on it.
#[allow(clippy::too_many_arguments)]
pub fn execute_update<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    table: &str,
    alias: Option<&str>,
    from: Option<&Plan>,
    assignments: &[(String, Expr)],
    filter: Option<&Expr>,
    target_probe: Option<&Plan>,
    returning: &[Expr],
    env: EvalEnv<'_>,
) -> Result<QueryResult> {
    // Own the table definition (like `execute_insert`) so `schema.sequences` can be
    // mutated while resolving a `SET col = DEFAULT` SERIAL/nextval default below —
    // a borrow of `schema` held in `UpdateCtx` would otherwise clash.
    let table_def = get_table_or_err(schema, table)?.clone();

    let col_names: Vec<&str> = table_def.columns.iter().map(|c| c.name.as_str()).collect();
    // The parallel declared types — the scope a CHECK / SET-RHS / RETURNING
    // expression binds against (see `ColumnResolver::column_type`).
    let col_types: Vec<TypeOid> = table_def.columns.iter().map(|c| c.type_oid).collect();
    // ...and each column's enum member order (see `ColumnResolver::column_enum_order`).
    let enums = schema.enums.clone();
    let col_enums = super::helpers::table_enum_orders(&table_def.columns, &enums);

    // Parse the table's CHECK predicates once per statement (not per row).
    let checks = prepare_checks(&table_def);
    // Prepare the table's indexes once per statement (hoisted out of the per-row
    // `enforce_index_entries`).
    let indexes = prepare_indexes(schema, table);

    // Build column index map for assignments
    let resolved: Vec<(usize, &Expr)> = assignments
        .iter()
        .map(|(col_name, expr)| {
            let idx = col_names
                .iter()
                .position(|n| *n == col_name)
                .ok_or_else(|| crate::CatalogError::ColumnNotFound {
                    table: table.to_string(),
                    column: col_name.clone(),
                })?;
            Ok((idx, expr))
        })
        .collect::<Result<Vec<_>>>()?;
    // Column order, not written order — see `assign_row`.
    let assignment_indices: Vec<(usize, Expr)> = in_column_order(&resolved, col_names.len())
        .map(|(idx, expr)| (idx, expr.clone()))
        .collect();

    let ctx = UpdateCtx {
        table,
        table_def: &table_def,
        col_names: &col_names,
        col_types: &col_types,
        col_enums: &col_enums,
        checks: &checks,
        indexes: &indexes,
        assignment_indices: &assignment_indices,
        filter,
        target_probe,
    };

    // The name this statement's target relation answers to — Postgres's RTE `eref`:
    // the alias when one is written, the table's own name when none is. Computed once
    // and handed to **both** collection paths, so an unaliased UPDATE cannot end up
    // with a scope that has no identity at all (see `collect_updated_simple`).
    let effective = alias.unwrap_or(table);

    // Collect the (id, old_row, new_row) triples read-only — no writes yet. The `FROM`
    // path also hands back the matched source rows, which `RETURNING` projects
    // alongside the target's.
    let (collected, from_image) = match from {
        None => (
            collect_updated_simple(&ctx, &*storage, schema, effective, env)?,
            None,
        ),
        Some(from_plan) => {
            let (collected, image) =
                collect_updated_from(&ctx, &*storage, schema, effective, from_plan, env)?;
            (collected, Some(image))
        }
    };

    // Write every target row first — each checked NOT NULL, CHECK and index entry by index
    // entry as it is written, sequentially so read-your-own-writes unique enforcement holds
    // across rows. 18.6 does the same and fires the foreign-key work — each row's actions,
    // then its own key checks — as after-row triggers once the statement's rows are
    // written, so a later target row's refusal outranks every refusal an action or a key
    // check would raise.
    for (id, _old_row, new_row) in &collected {
        write_updated_row(&ctx, storage, schema, *id, new_row)?;
    }

    // The foreign-key trigger queue, fired in row order ([`FkTriggerQueue`]). When this
    // table references and is referenced by no FK, or no key changed, nothing is queued.
    let mut fk_queue = FkTriggerQueue::new(schema);
    for (id, old_row, new_row) in &collected {
        fk_queue.queue_statement_row(table, *id, old_row, new_row);
    }
    fk_queue.fire(storage)?;

    let updated_rows: Vec<Vec<Value>> = collected.into_iter().map(|(_, _, row)| row).collect();
    let updated_count = updated_rows.len();

    // `RETURNING` projects against the statement's whole FROM namespace: the (updated)
    // target row, then the source row it matched. Without a `FROM` the namespace is the
    // target alone under its *effective* name (its alias if present) — so
    // `UPDATE t o … RETURNING o.id` binds `o`, and a bogus qualifier fails loud
    // (`42P01`) instead of silently loose-matching the bare column. With one, both
    // relations are named and a reference binds to the `(relation, column)` pair rather
    // than to a synthesized string (see `project_returning`).
    // The widened row is built only when there is a clause to project: it costs one
    // clone of the source half per updated row, which a `RETURNING`-less
    // `UPDATE … FROM` should not pay.
    let target_columns = if returning.is_empty() {
        Vec::new()
    } else {
        table_column_descs(&table_def)
    };
    let (returning_relations, returning_rows) = match &from_image {
        _ if returning.is_empty() => (Vec::new(), Vec::new()),
        None => (
            vec![(alias.unwrap_or(table), target_columns.as_slice())],
            updated_rows,
        ),
        Some(image) => (
            vec![
                (alias.unwrap_or(table), target_columns.as_slice()),
                (image.relation.as_str(), image.columns.as_slice()),
            ],
            updated_rows
                .into_iter()
                .zip(&image.rows)
                .map(|(mut target_row, from_row)| {
                    target_row.extend(from_row.iter().cloned());
                    target_row
                })
                .collect(),
        ),
    };
    finish_returning(
        &returning_rows,
        &returning_relations,
        returning,
        schema,
        storage,
        env,
        format!("UPDATE {updated_count}"),
    )
}

/// The `FROM` half of an `UPDATE … FROM`, kept for the `RETURNING` projection.
///
/// `rows` is positionally parallel to the collected targets — the source row each
/// target matched — and `columns` describes it. `relation` is the source's effective
/// FROM name, the qualifier a `RETURNING s.k` must spell.
struct FromImage {
    relation: String,
    columns: Vec<ColumnDesc>,
    rows: Vec<Vec<Value>>,
}

/// One collected UPDATE target, computed read-only before any write:
/// `(row id, old row image, new row image)`. The new image is written; the pair drives
/// the row's foreign-key triggers ([`FkTriggerQueue`]: an action finds the children of
/// the OLD key).
type UpdatedRow = (RowId, Vec<Value>, Vec<Value>);

/// The per-statement context shared by the UPDATE execution paths: the catalog
/// handles and the once-prepared CHECK/unique/assignment state. `schema` is
/// passed explicitly (not held here) so the mutation loop can take
/// `&mut schema.sequences` for a `SET col = DEFAULT` while `table_def` (an owned
/// clone) stays borrowed.
struct UpdateCtx<'a> {
    table: &'a str,
    table_def: &'a Table,
    col_names: &'a [&'a str],
    /// Declared types parallel to `col_names` — the scope a SET-RHS / `WHERE` /
    /// CHECK expression binds against (see `ColumnResolver::column_type`).
    col_types: &'a [TypeOid],
    /// Enum member orders parallel to `col_names` — what makes a SET-RHS / `WHERE` /
    /// CHECK comparison over an enum column order by member position (see
    /// `ColumnResolver::column_enum_order`).
    col_enums: &'a [Option<&'a [String]>],
    checks: &'a [(String, Expr)],
    indexes: &'a [PreparedIndex],
    assignment_indices: &'a [(usize, Expr)],
    filter: Option<&'a Expr>,
    /// The index probe the simple path collects its targets through (`Plan::Update`'s
    /// `target_probe`); the `FROM` path never has one.
    target_probe: Option<&'a Plan>,
}

/// Assemble the row an `UPDATE` writes: `old_row` with each `SET` assignment's value
/// written in, every right-hand side evaluated through `resolver`.
///
/// **Every right-hand side reads `old_row`** — the target row as the scan returned it,
/// before any of this row's own assignments — and never an earlier assignment's result,
/// because 18.6 projects the whole `SET` list from one input tuple. Measured on 18.6 over
/// `(10, 0)`: `SET a = a + 1, b = a` is `11 | 10`, and `SET a = b, b = a` swaps the two.
/// The new row is a separate copy, so an assignment cannot be read back by a later
/// right-hand side; only the assembled row is then checked, written and projected by
/// `RETURNING`.
/// `old_row` is the target row, or `target ++ source` on the `UPDATE … FROM` path (the
/// target columns lead it, so each assignment writes at its column's own index).
///
/// **The right-hand sides run in the target columns' order**, which
/// `ctx.assignment_indices` carries ([`in_column_order`]), not the order the list is
/// written in: 18.6's rewriter rebuilds the list in attribute order
/// (`rewriteTargetListIU`). Measured on 18.6: of two right-hand sides failing at
/// evaluation, the earlier *column*'s error answers whichever is written first, and two
/// `DEFAULT`s drawing on one sequence draw in column order.
///
/// A right-hand side whose error 18.6 raises **before execution** — a literal's conversion
/// to the column's type in parse analysis, or the planner's folding of an immutable
/// expression over constants — answers there before any right-hand side is evaluated.
/// Where pglet leaves that error to evaluation, it answers when its column's turn comes,
/// so an earlier column's evaluation error answers ahead of it. Measured over `(1, 0)`:
/// `SET b = 'notanint', a = a / b` and `SET b = 2147483647 + 1, a = a / b` are `22012`
/// here and `22P02` and `22003` on 18.6, while `SET b = 'notanint'::int, a = a / b` is
/// `22P02` on both, that cast being converted before any row here too — though not every written cast is (`'notanint'::varchar::int` waits). Those are
/// examples, not a list; `EDGES.md` rows 5 and 244 carry the measured members.
///
/// A `SET col = DEFAULT` resolves to the column's catalog default (mirroring
/// INSERT, advancing a SERIAL/nextval sequence in `schema.sequences`); every
/// other RHS evaluates normally, and an eval error **propagates** (correct-or-loud
/// — a SET-RHS error was previously swallowed and the column's old value silently
/// kept, diverging from Postgres which errors).
fn assign_row<R: ColumnResolver>(
    ctx: &UpdateCtx<'_>,
    resolver: &R,
    old_row: &[Value],
    schema: &mut Schema,
    env: EvalEnv<'_>,
) -> Result<Vec<Value>> {
    let mut new_row = old_row.to_vec();
    for (idx, expr) in ctx.assignment_indices {
        let col = &ctx.table_def.columns[*idx];
        let val = if matches!(expr, Expr::Default) {
            resolve_default(col, &mut schema.sequences)?
        } else {
            // Coerce the assigned value to the column type (+ NUMERIC typmod),
            // matching INSERT so a `SET n = <literal>` stores the column's type.
            coerce_column_value(
                eval_expr_full(
                    expr,
                    resolver,
                    old_row,
                    SubqueryExec::NONE,
                    // The catalog is lent per evaluation rather than once for the
                    // function, because the `DEFAULT` arm above needs
                    // `&mut schema.sequences` and the two borrows must not overlap.
                    // Lending is what makes a catalog-reading expression
                    // (`pg_get_constraintdef`, `::regclass`) answer in a `SET` the same
                    // way it answers in a `SELECT`.
                    env.with_schema(&*schema),
                )?,
                col,
            )?
        };
        // Enum membership on each assigned column's new value. NOT NULL is row-scoped
        // and runs over the assembled row in `execute_update`, before the FK plan.
        validate_column(ctx.table, col, &val, &schema.enums)?;
        new_row[*idx] = val;
    }
    Ok(new_row)
}

/// Check the post-update target `row` (NOT NULL / CHECK / each index entry) and write it.
/// `row` is the target row only (the `from` portion is dropped before this). Its foreign
/// keys are checked by `execute_update` once the statement's actions have run.
fn write_updated_row<S: Storage>(
    ctx: &UpdateCtx<'_>,
    storage: &mut S,
    schema: &Schema,
    id: RowId,
    row: &[Value],
) -> Result<()> {
    // Unique excludes the row's own id, so a key-preserving UPDATE doesn't self-conflict.
    enforce_row_checks(
        storage,
        schema,
        ctx.table,
        &ctx.table_def.columns,
        ctx.checks,
        ctx.col_names,
        ctx.col_types,
        ctx.indexes,
        row,
        Some(id),
    )?;
    storage.update(ctx.table, id, row.to_vec())?;
    Ok(())
}

/// The simple UPDATE path (no `FROM`): scan, filter, and compute each matching row's
/// `(id, old_row, new_row)` — read-only, no writes (the caller writes the rows, then
/// fires their foreign-key triggers).
///
/// `effective` is the target's name for qualified resolution — its alias when one was
/// written (`UPDATE t o SET … WHERE o.col = …`, where the alias **hides** `t`), else the
/// table's own name. It is a `&str` and not an `Option` because *every* scope here has an
/// identity: this took the statement's `alias` verbatim, so an unaliased UPDATE handed
/// `None` to both the `SimpleColumns` and — through `scan_matching_rows` — the correlated
/// outer frame, where a `None` does not mean "unnamed" but "a join, addressed by
/// synthesized `relation.column` strings". Both readers then answered for relations the
/// statement does not name, on rows they went on to **write**: over `u1(id, "a.b")` and
/// `u2(yv)`, `UPDATE u1 SET id = 99 WHERE EXISTS (SELECT 1 FROM u2 WHERE u2.yv = a.b)`
/// updated a row where 18.6 raises `42P01`, and over `ex1(n, v)`,
/// `UPDATE ex1 SET v = bogusrel.n + 100` stored `101` — `ex1`'s own `n`, read through a
/// qualifier naming nothing. `execute_delete` passes `Some(table)` and had neither.
fn collect_updated_simple<S: Storage>(
    ctx: &UpdateCtx<'_>,
    storage: &S,
    schema: &mut Schema,
    effective: &str,
    env: EvalEnv<'_>,
) -> Result<Vec<UpdatedRow>> {
    let resolver = SimpleColumns::with_table(ctx.col_names, Some(effective))
        .with_types(ctx.col_types)
        .with_enum_orders(ctx.col_enums);
    let matching = scan_matching_rows(
        storage,
        ctx.table,
        ctx.col_names,
        Some(effective),
        ctx.filter,
        ctx.target_probe,
        schema,
        env,
    )?;

    let mut updated = Vec::with_capacity(matching.len());
    for (id, old_row) in matching {
        let new_row = assign_row(ctx, &resolver, &old_row, schema, env)?;
        // NOT NULL **per row, here** — the position `validate_column` held before the
        // check became row-scoped. 18.6 constrains each new tuple inside `ExecUpdate`
        // before moving to the next, so row i's null outranks row j's coercion failure:
        // `UPDATE y SET a = CASE WHEN k=1 THEN NULL ELSE 8 END, e = <bad enum on k=2>`
        // is `23502` there, and `22P02` if the gate waits for the whole collect pass
        // (measured on both engines, with the `varchar(3)` overflow spelling too).
        // Running it here also keeps it before the foreign-key queue and before any
        // write, which is what makes the target's null outrank a child's
        // `23001`/`23503`/`23514` and leaves the rollback-less memory backend untouched.
        enforce_not_null_row(ctx.table, &ctx.table_def.columns, ctx.col_types, &new_row)?;
        updated.push((id, old_row, new_row));
    }
    Ok(updated)
}

/// The `UPDATE … FROM` path: inner-join the target to `from_plan` and update each
/// target row that has a match. The `WHERE`/`SET` expressions resolve against the
/// combined `target ++ from` row via a `JoinColumnMap` keyed on the target's
/// `effective` name and the source's effective name. A target row is updated at
/// most once (the first matching `from` row); Postgres leaves a multi-match
/// update unspecified, so callers join on a key.
///
/// Returns the collected targets and the [`FromImage`] — the matched source rows,
/// positionally parallel, which `RETURNING` projects alongside the target's.
fn collect_updated_from<S: Storage>(
    ctx: &UpdateCtx<'_>,
    storage: &S,
    schema: &mut Schema,
    effective: &str,
    from_plan: &Plan,
    env: EvalEnv<'_>,
) -> Result<(Vec<UpdatedRow>, FromImage)> {
    let target_len = ctx.col_names.len();

    // Both inputs are reads; collect them (owned) so the writes stay out of this
    // read-only collection pass.
    let target_rows = storage.scan(ctx.table)?;
    let (from_rows, from_columns) = execute_plan_node(from_plan, schema, storage, env)?;
    // The source's effective name keys its columns in the map, and is the qualifier a
    // `WHERE s.k` or a `RETURNING s.k` must spell. It is derived here by the executor's
    // `get_table_name_from_plan`, whose answer must agree with the planner's
    // `plan_relations` for every kind `parse_table_factor` can produce — the two
    // disagreeing is what made a FROM-clause `unnest(…)` key on `"unknown"`, so
    // `WHERE u.u = 'x'` was `42P01` on a statement 18.6 answers.
    let from_name = get_table_name_from_plan(from_plan);

    // Combined namespace: target columns under `effective` (index 0..target_len),
    // source columns under the source's name (index target_len..).
    let mut col_map = JoinColumnMap::new();
    let target_cols = table_column_descs(ctx.table_def);
    col_map.add_columns(effective, &target_cols, 0, &schema.enums);
    col_map.add_columns(&from_name, &from_columns, target_len, &schema.enums);

    let mut updated = Vec::new();
    let mut matched_from_rows = Vec::new();
    for (id, target_row) in target_rows {
        // First matching source row wins (an inner join; empty source → no match).
        // A `WHERE` eval error **propagates** (correct-or-loud — not swallowed to a
        // silent no-match, the bug `execute_join` and the DELETE/UPDATE `WHERE` path
        // already fixed; an untyped FROM-source param decoded as text is the realistic
        // trigger).
        let mut chosen = None;
        for from_row in &from_rows {
            let mut combined = target_row.clone();
            combined.extend(from_row.iter().cloned());
            let matched = match ctx.filter {
                None => true,
                Some(f) => predicate_holds(
                    &eval_expr_join(f, &col_map, &combined, env)?,
                    BoolContext::Where,
                )?,
            };
            if matched {
                chosen = Some(combined);
                break;
            }
        }
        if let Some(combined) = chosen {
            let mut combined = assign_row(ctx, &col_map, &combined, schema, env)?;
            // Keep only the target portion as the new row; the source portion is the
            // `RETURNING` namespace's other half and is kept beside it, not in it —
            // everything downstream of the collect pass (the foreign-key queue, the
            // row write) indexes an `UpdatedRow` by *target* column position.
            let from_row = combined.split_off(target_len);
            // Per row, as in `collect_updated_simple` — but **after** the split, not
            // before: `combined` is `target ++ from` until here, and the `Failing row`
            // detail must describe the target alone.
            enforce_not_null_row(ctx.table, &ctx.table_def.columns, ctx.col_types, &combined)?;
            updated.push((id, target_row, combined));
            matched_from_rows.push(from_row);
        }
    }
    Ok((
        updated,
        FromImage {
            relation: from_name,
            columns: from_columns,
            rows: matched_from_rows,
        },
    ))
}

/// A child-row `SET NULL` queued by the delete plan: null **all** `columns` of row
/// `id` in `table` (a composite FK nulls every referencing column). Applied only if
/// the row isn't itself in the delete set (a delete wins).
struct SetNullEdit {
    table: String,
    id: RowId,
    columns: Vec<usize>,
}

/// One FK that references the table currently being deleted from, flattened with
/// its owning (child) table name so the cascade loop has everything it needs
/// without re-borrowing `schema` across the storage reads. `child_columns[i]` pairs
/// positionally with `parent_columns[i]` (single- or multi-column).
struct ReferencingFk {
    child_table: String,
    child_columns: Vec<String>,
    parent_columns: Vec<String>,
    fk_name: String,
    action: ReferentialAction,
}

impl ReferencingFk {
    /// Flatten one `(child table, FK)` pair from [`Schema::referencing_foreign_keys`]
    /// under the referential action for this direction — `on_delete` for the DELETE
    /// worklist, `on_update` for the UPDATE one. The only difference between the two
    /// collections, so they share this constructor rather than two near-identical
    /// walks.
    fn from_catalog(child: &Table, fk: &ForeignKey, action: ReferentialAction) -> Self {
        Self {
            child_table: child.name.clone(),
            child_columns: fk.columns.clone(),
            parent_columns: fk.parent_columns.clone(),
            fk_name: fk.name.clone(),
            action,
        }
    }
}

/// The fully-resolved effect of a DELETE, computed read-only before any mutation.
///
/// `deletes` is the cascade closure (every row that will be removed, in discovery
/// order); `set_nulls` is the FK columns to null on surviving rows. Building this
/// from reads alone means a violation the plan raises — RESTRICT, and a nulled row's
/// NOT NULL, CHECK, index entries' sizes and duplicates — aborts the statement
/// **before** storage is touched, on every backend. The plan does not ask the storage
/// maintenance path's own refusal of an entry — pglet's page bound on a key
/// (`EDGES.md` row 192) — so where a nulled row reaches
/// it, it is raised in the apply phase, after the statement's earlier writes, which a
/// backend with statement rollback undoes and `--storage memory`, having none, keeps.
struct DeletePlan {
    deletes: Vec<(String, RowId)>,
    set_nulls: Vec<SetNullEdit>,
}

/// Execute DELETE.
///
/// Beyond deleting the directly-matched rows, this applies the `ON DELETE` action
/// of every FK that **references** each deleted row (the parent side): `CASCADE`
/// deletes the referencing children (recursively, multi-level), `SET NULL` nulls
/// the child's FK column, and `NO ACTION`/`RESTRICT` aborts with a
/// `ForeignKeyViolation` if any child references the row.
///
/// The whole effect is planned read-only (`plan_delete_cascade`) before any write,
/// so the statement is atomic: a RESTRICT/NOT-NULL violation aborts with storage
/// untouched, and a success applies every delete + SET NULL in one go (the
/// embed/server layer persists a single meta root after the statement). The
/// `DELETE n` tag and `RETURNING` report only the directly-matched rows, matching
/// Postgres (cascaded rows aren't counted).
///
/// **There is no `alias` parameter because the parser has no `DELETE FROM t [AS] x`**
/// (`42601` here; 18.6 accepts it). That is why passing `table` as the scope identity to
/// `scan_matching_rows` is right today and would become wrong the moment the parser gains
/// the alias: Postgres's `eref` is the alias when one is written, and the alias **hides**
/// the table name — the same rule `execute_update` spells as `alias.unwrap_or(table)`.
pub fn execute_delete<S: Storage>(
    schema: &Schema,
    storage: &mut S,
    table: &str,
    filter: Option<&Expr>,
    target_probe: Option<&Plan>,
    returning: &[Expr],
    env: EvalEnv<'_>,
) -> Result<QueryResult> {
    // Check table exists and get column info
    let table_def = get_table_or_err(schema, table)?;

    let col_names: Vec<&str> = table_def.columns.iter().map(|c| c.name.as_str()).collect();

    // Find the directly-matched rows (the statement's own targets). Reads only —
    // the mutation phase is below, after the targets are collected.
    let matching_rows = scan_matching_rows(
        &*storage,
        table,
        &col_names,
        Some(table),
        filter,
        target_probe,
        schema,
        env,
    )?;

    // Plan the full cascade closure + SET NULL edits read-only; this raises on a
    // RESTRICT/NOT-NULL violation BEFORE any mutation (atomicity).
    let seeds: Vec<(RowId, Vec<Value>)> = matching_rows.clone();
    let plan = plan_delete_cascade(schema, storage, table, &seeds)?;

    // Apply phase (mutations only — the plan already validated). SET NULL first,
    // skipping any row that the closure will delete (the delete subsumes it).
    let delete_set: HashSet<(&str, RowId)> = plan
        .deletes
        .iter()
        .map(|(t, id)| (t.as_str(), *id))
        .collect();
    for edit in &plan.set_nulls {
        if delete_set.contains(&(edit.table.as_str(), edit.id)) {
            continue;
        }
        if let Some(mut row) = storage.get(&edit.table, edit.id)? {
            for &col in &edit.columns {
                if col < row.len() {
                    row[col] = Value::Null;
                }
            }
            storage.update(&edit.table, edit.id, row)?;
        }
    }
    for (t, id) in &plan.deletes {
        storage.delete(t, *id)?;
    }

    // RETURNING / command tag count only the directly-matched rows.
    let deleted_rows: Vec<Vec<Value>> = matching_rows.into_iter().map(|(_, row)| row).collect();
    let deleted_count = deleted_rows.len();

    // RETURNING / command tag count only the directly-matched rows.
    finish_returning(
        &deleted_rows,
        &[(table, &table_column_descs(table_def))],
        returning,
        schema,
        storage,
        env,
        format!("DELETE {deleted_count}"),
    )
}

/// The error for a parent DELETE/UPDATE blocked by a referencing child, keyed on the
/// FK's action so the SQLSTATE and message match Postgres: **NO ACTION** →
/// `ForeignKeyViolation` (`23503`, `violates foreign key constraint`); **RESTRICT** →
/// `RestrictViolation` (`23001`, `violates RESTRICT setting of foreign key
/// constraint`). Postgres separates the two, so a differential `@expect-error` fixture
/// on an explicit `RESTRICT` requires the distinct SQLSTATE. `action` is only ever
/// `NoAction`/`Restrict` here (the caller's match arm binds those two).
///
/// `key` is the **parent's** referenced tuple, which is what 18.6 puts in the DETAIL
/// on this side (`Key (id)=(1) is still referenced from table "c"`), while `t` names
/// the **child** in both directions — see `ForeignKeyViolationDetail`.
fn fk_block_violation(
    action: ReferentialAction,
    parent_table: &str,
    fk_name: &str,
    child_table: &str,
    key: KeyDescription,
) -> Error {
    let detail = Box::new(ForeignKeyViolationDetail {
        child_table: child_table.to_string(),
        parent_table: parent_table.to_string(),
        constraint: fk_name.to_string(),
        direction: FkDirection::Delete,
        key,
    });
    if matches!(action, ReferentialAction::Restrict) {
        Error::RestrictViolation(detail)
    } else {
        Error::ForeignKeyViolation(detail)
    }
}

/// Render a parent row's referenced-key tuple for a `Key (…)=(…)` DETAIL, honoring
/// each referenced column's declared type.
fn parent_key_description(
    parent_def: &Table,
    parent_columns: &[String],
    positions: &[usize],
    key: &[Value],
) -> KeyDescription {
    KeyDescription::new(
        parent_columns.to_vec(),
        key.iter()
            .enumerate()
            .map(|(i, v)| {
                positions
                    .get(i)
                    .and_then(|&p| parent_def.columns.get(p))
                    .map_or_else(
                        || crate::types::value_to_text(v),
                        |c| crate::types::value_to_text_with_oid(v, c.type_oid),
                    )
            })
            .collect(),
    )
}

/// Read-only: compute the full set of rows a DELETE removes (cascade closure) and
/// the FK columns it nulls, validating RESTRICT and every nulled row.
///
/// A BFS over the FK graph seeded by `seeds` (the directly-matched rows of
/// `seed_table`). For each row removed, every FK that **references** its table
/// drives an action on the children whose FK column equals the removed row's
/// referenced key — the children as the actions have left them so far
/// ([`find_referencing_children`] over [`ActionState`]'s rows):
/// - `Cascade`: enqueue each child for deletion (recurses). A `(table, id)`
///   visited-set both dedups overlapping fan-in and breaks a cascade cycle
///   (a re-encountered row is skipped — the cycle guard, so the BFS terminates).
/// - `SetNull`: null the FK columns of the child's current image — so a row two
///   `SET NULL` keys reach is nulled twice, the second over the first — and check the
///   image as 18.6 checks the row its action writes (`ActionState::rewrite`: NOT NULL,
///   CHECK, then each index entry, a duplicate probed against the storage entries the
///   actions have not superseded and against the images they have planned), so a SET
///   NULL that would leave a constraint violated aborts the whole DELETE (matching
///   Postgres at delete time).
/// - `NoAction`/`Restrict`: a referencing child raises `ForeignKeyViolation`
///   UNLESS that child is already in `visited` (the cascade closure built so far)
///   when the edge is checked — Postgres lets a row and its referenced parent be
///   deleted together.
///
/// **Order-dependence caveat (RESTRICT/NO ACTION + CASCADE fan-in).** The block
/// decision uses `visited` *at the moment the edge is checked*, not the final
/// closure. Because BFS visits a deleted row's direct children before descending,
/// this matches Postgres for the common shapes — a child cascaded from the same
/// row/seed that the RESTRICT edge originates from is already `visited` and so does
/// not block, and a child reachable only by a *deeper* cascade chain is correctly
/// seen as not-yet-deleted and blocks (Postgres rejects that fan-in too, via its
/// immediate-RESTRICT / per-row RI-trigger order). It is **not** a faithful model of
/// Postgres's trigger semantics in every topology, so an exotic mixed
/// RESTRICT+CASCADE fan-in could diverge. The spine never combines RESTRICT (or
/// NO ACTION) with CASCADE on a shared child, so this is not a spine-correctness
/// gap; a faithful model would replicate Postgres's RI-trigger queue, which is out
/// of scope here. (Tracked in `PLAN.md`.)
///
/// Performs no writes, so any raised error leaves storage untouched.
///
/// # Errors
///
/// `Error::ForeignKeyViolation` / `Error::RestrictViolation` / `Error::NotNullViolation`
/// / `Error::CheckViolation` / `Error::UniqueViolation` / an index-size refusal; a
/// storage error from a scan.
fn plan_delete_cascade<S: Storage>(
    schema: &Schema,
    storage: &S,
    seed_table: &str,
    seeds: &[(RowId, Vec<Value>)],
) -> Result<DeletePlan> {
    let mut visited: HashSet<(String, RowId)> = HashSet::new();
    let mut deletes: Vec<(String, RowId)> = Vec::new();
    let mut set_nulls: Vec<SetNullEdit> = Vec::new();
    // The rows the actions have nulled or removed so far, beside the storage they have not
    // been written to: a second `SET NULL` reaching a row nulls the first one's image, and
    // each nulled image is checked as 18.6 checks the row its action writes
    // (`ActionState::rewrite`).
    let mut state = ActionState::default();

    // BFS worklist of rows to remove: (table, id, row values).
    let mut queue: VecDeque<(String, RowId, Vec<Value>)> = VecDeque::new();
    for (id, row) in seeds {
        if visited.insert((seed_table.to_string(), *id)) {
            state.rows_mut(seed_table).remove(*id);
            queue.push_back((seed_table.to_string(), *id, row.clone()));
        }
    }

    while let Some((parent_table, parent_id, parent_row)) = queue.pop_front() {
        deletes.push((parent_table.clone(), parent_id));

        // Every FK referencing this table (collected up front; `schema` is read-only).
        let referencing: Vec<ReferencingFk> = schema
            .referencing_foreign_keys(&parent_table)
            .map(|(child, _, fk)| ReferencingFk::from_catalog(child, fk, fk.on_delete))
            .collect();

        for ReferencingFk {
            child_table,
            child_columns,
            parent_columns,
            fk_name,
            action,
        } in referencing
        {
            let Some(child_def) = schema.get_table(&child_table) else {
                continue;
            };
            let Some(parent_def) = schema.get_table(&parent_table) else {
                continue;
            };
            // The referenced-key tuple from the deleted parent row (in the FK's
            // `parent_columns` order, paired with the child columns below).
            let Some(parent_positions) = parent_columns
                .iter()
                .map(|pc| parent_def.columns.iter().position(|c| &c.name == pc))
                .collect::<Option<Vec<usize>>>()
            else {
                continue;
            };
            let key_tuple: Vec<Value> = parent_positions
                .iter()
                .map(|&p| parent_row.get(p).cloned().unwrap_or(Value::Null))
                .collect();
            // A NULL in the referenced tuple is never matched by a MATCH SIMPLE child.
            if key_tuple.iter().any(Value::is_null) {
                continue;
            }
            let Some(child_positions) = child_columns
                .iter()
                .map(|cc| child_def.columns.iter().position(|c| &c.name == cc))
                .collect::<Option<Vec<usize>>>()
            else {
                continue;
            };

            // The children as the actions have left them so far: a row the statement
            // removes is not among them, and a nulled row is read by its planned image.
            let children = find_referencing_children(
                storage,
                state.rows(&child_table),
                &child_table,
                &child_positions,
                &key_tuple,
            )?;

            match action {
                ReferentialAction::Cascade => {
                    for (child_id, child_row) in children {
                        // The visited guard dedups fan-in and breaks any cascade cycle.
                        if visited.insert((child_table.clone(), child_id)) {
                            state.rows_mut(&child_table).remove(child_id);
                            queue.push_back((child_table.clone(), child_id, child_row));
                        }
                    }
                }
                ReferentialAction::SetNull => {
                    for (child_id, child_row) in children {
                        // The post-SET-NULL image (all FK columns nulled) of the row as
                        // it stands, an earlier action's rewrite included, checked as
                        // 18.6 checks the row the action writes.
                        let mut nulled = child_row;
                        for &pos in &child_positions {
                            if pos < nulled.len() {
                                nulled[pos] = Value::Null;
                            }
                        }
                        state.rewrite(schema, storage, child_def, child_id, nulled)?;
                        set_nulls.push(SetNullEdit {
                            table: child_table.clone(),
                            id: child_id,
                            columns: child_positions.clone(),
                        });
                    }
                }
                action @ (ReferentialAction::NoAction | ReferentialAction::Restrict) => {
                    // A child referencing this row blocks the delete — UNLESS that
                    // child is already in the cascade closure when this edge is
                    // checked (Postgres lets a row and its referenced parent be
                    // deleted together). `children` leaves out every row removed so far
                    // — which, because BFS visits a deleted row's direct children before
                    // descending, covers the case Postgres accepts (a child cascaded
                    // from the same row/seed). See the order-dependence note on the fn.
                    if !children.is_empty() {
                        return Err(fk_block_violation(
                            action,
                            &parent_table,
                            &fk_name,
                            &child_table,
                            parent_key_description(
                                parent_def,
                                &parent_columns,
                                &parent_positions,
                                &key_tuple,
                            ),
                        ));
                    }
                }
            }
        }
    }

    Ok(DeletePlan { deletes, set_nulls })
}

/// The values an `ON UPDATE` action assigns to the referencing columns at
/// `child_positions` — `new_key`'s under `CASCADE`, NULL under `SET NULL` (`new_key`
/// `None`) — each converted to its child column as any assignment is:
/// [`coerce_column_value`] (the column's type and modifier), then [`validate_column`].
/// Parallel to `child_positions`.
///
/// 18.6's action is `UPDATE ONLY child SET col = $1 … WHERE …` through SPI, so the
/// parent's key reaches the child through the child column's assignment conversion: a
/// `bigint` child of an `int` key stores an `int8`, an `int` child of a `numeric` key
/// rounds `2.6` to `3`, and a `varchar(3)` child refuses `'abcdef'` (`22001`). The
/// columns convert in the child's column order, the order 18.6's target list takes.
///
/// It runs once per action event, when the event's turn in the queue comes and ahead of
/// the lookup of the rows the action rewrites, so a refused conversion refuses the
/// statement whether or not a child row references the old key — 18.6's answer while the
/// action's SPI plan is a custom plan, which folds the conversion over the bound key when
/// the action is planned. Where 18.6's plan cache has switched a key's action to a generic
/// plan, later in a session, it refuses only where a child row is rewritten (`EDGES.md`
/// row 200).
///
/// # Errors
///
/// A conversion refusal of [`coerce_column_value`] (`22001`, `22003`, `42804`, …) or an
/// enum-membership refusal of [`validate_column`].
fn action_assigned_values(
    schema: &Schema,
    child_def: &Table,
    child_positions: &[usize],
    new_key: Option<&[Value]>,
) -> Result<Vec<Value>> {
    let mut assigned = vec![Value::Null; child_positions.len()];
    let mut order: Vec<usize> = (0..child_positions.len()).collect();
    order.sort_by_key(|&i| child_positions[i]);
    for i in order {
        let Some(col) = child_def.columns.get(child_positions[i]) else {
            continue;
        };
        let value = new_key
            .and_then(|key| key.get(i).cloned())
            .unwrap_or(Value::Null);
        let value = coerce_column_value(value, col)?;
        validate_column(&child_def.name, col, &value, &schema.enums)?;
        assigned[i] = value;
    }
    Ok(assigned)
}

/// Find child rows in `child_table` whose FK columns (at `child_positions`) equal the
/// referenced-key `key_tuple` — pairwise, single- or multi-column — as the statement's
/// actions have left them so far (`rows`): a row the statement removes is not among them,
/// and a row an action has rewritten is matched, and returned, by its planned image.
///
/// Scans the child table (read-your-own-writes correct via the adapter). A scan,
/// not an index probe, so the plan phase needs no index and stays correct even
/// when the FK columns are unindexed. A row with a NULL in any FK column never
/// matches (MATCH SIMPLE: `values_equal` returns `None` for a NULL operand, and
/// `key_tuple` is NULL-free), so it correctly references nothing.
///
/// # Errors
///
/// A storage error from `scan`.
fn find_referencing_children<S: Storage>(
    storage: &S,
    rows: Option<&ActionRows>,
    child_table: &str,
    child_positions: &[usize],
    key_tuple: &[Value],
) -> Result<Vec<(RowId, Vec<Value>)>> {
    let mut out = Vec::new();
    for (id, stored) in storage.scan(child_table)? {
        if rows.is_some_and(|r| r.is_removed(id)) {
            continue;
        }
        let row = rows
            .and_then(|r| r.image(id))
            .map_or(stored, <[Value]>::to_vec);
        // Equality must be type-correct for the FK key (commonly a UUID): `fk_key_matches`
        // reads `values_equal`, not `values_compare` — the latter's ordering
        // fallback returns `Equal` for any two non-numeric/non-text values, which
        // would match every child row regardless of its key.
        let matches = child_positions.iter().zip(key_tuple).all(|(&pos, want)| {
            row.get(pos)
                .is_some_and(|v| super::constraints::fk_key_matches(want, v))
        });
        if matches {
            out.push((id, row));
        }
    }
    Ok(out)
}

/// What a statement's foreign-key actions have planned so far, and the per-child-table
/// preparations their checks reuse.
#[derive(Default)]
struct ActionState {
    /// Each child table's rows as the actions have left them ([`ActionRows`]).
    rows: HashMap<String, ActionRows>,
    /// Each child table's CHECK predicates, parsed once.
    checks: HashMap<String, Vec<(String, Expr)>>,
    /// Each child table's indexes, prepared once.
    indexes: HashMap<String, Vec<PreparedIndex>>,
}

impl ActionState {
    /// The table's rows as the actions have left them, if an action has reached it.
    fn rows(&self, table: &str) -> Option<&ActionRows> {
        self.rows.get(table)
    }

    fn rows_mut(&mut self, table: &str) -> &mut ActionRows {
        self.rows.entry(table.to_string()).or_default()
    }

    /// Check the image an action writes over child row `id` as 18.6 checks the row the
    /// action's own `UPDATE` writes — NOT NULL, CHECK, then each index entry in creation
    /// order ([`enforce_cascaded_index_entries`]) — and record it as the row's image.
    ///
    /// 18.6 runs a `CASCADE` and a `SET NULL` alike as an `UPDATE` of the child, which
    /// constrains each row in full before it takes the next: a repointed key can be
    /// NULL (a nullable referenced key moved to NULL), and can collide under a unique
    /// index that is not the referenced key's (an expression index, a composite key's
    /// other part), so neither action is exempt from any of the three.
    ///
    /// # Errors
    ///
    /// `Error::NotNullViolation`, `Error::CheckViolation`, `Error::UniqueViolation` or an
    /// index-size refusal; an evaluation or storage error from a check or a probe.
    fn rewrite<S: Storage>(
        &mut self,
        schema: &Schema,
        storage: &S,
        child_def: &Table,
        id: RowId,
        image: Vec<Value>,
    ) -> Result<()> {
        let table = &child_def.name;
        let types: Vec<TypeOid> = child_def.columns.iter().map(|c| c.type_oid).collect();
        enforce_not_null_row(table, &child_def.columns, &types, &image)?;
        let names: Vec<&str> = child_def.columns.iter().map(|c| c.name.as_str()).collect();
        let enums = super::helpers::table_enum_orders(&child_def.columns, &schema.enums);
        let checks = self
            .checks
            .entry(table.clone())
            .or_insert_with(|| prepare_checks(child_def));
        enforce_checks(table, checks, &names, &types, &enums, &image)?;
        let indexes = self
            .indexes
            .entry(table.clone())
            .or_insert_with(|| prepare_indexes(schema, table));
        let rows = self.rows.entry(table.clone()).or_default();
        enforce_cascaded_index_entries(storage, indexes, &image, id, rows)?;
        rows.record(id, image);
        Ok(())
    }
}

/// The referenced-key tuple from `row` at `positions` (`Null` for a missing column).
fn parent_key_tuple(row: &[Value], positions: &[usize]) -> Vec<Value> {
    positions
        .iter()
        .map(|&p| row.get(p).cloned().unwrap_or(Value::Null))
        .collect()
}

/// Whether two referenced-key tuples are equal element-wise, type-correctly (via
/// `values_equal`). A NULL element compares unequal (NULL is never "the same key"),
/// so an unchanged NULL parent key reads as "changed" — the caller separately skips a
/// NULL old key, which is the only place this matters.
fn key_tuples_equal(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| values_equal(x, y) == Some(true))
}

/// Collect every FK that **references** `parent_table` (the parent side), carrying its
/// `on_update` action, in the order 18.6 fires their action triggers.
///
/// 18.6 fires a row's triggers in trigger-name order, and an action trigger is named
/// `RI_ConstraintTrigger_a_<oid>` after the OID minted when its key is created, so the
/// actions run in the order their foreign keys were created while those OIDs have one number
/// of digits (a name compares as text). pglet keeps no creation stamp on a foreign key; the referencing table's own
/// OID, then the key's place in that table's list, is that order wherever every key was
/// declared with its table — an `ALTER TABLE … ADD FOREIGN KEY` on a table created before
/// another referencing table is ordered by the table (`EDGES.md` row 198).
fn referencing_fks_on_update(schema: &Schema, parent_table: &str) -> Vec<ReferencingFk> {
    let mut fks: Vec<(i32, usize, ReferencingFk)> = schema
        .referencing_foreign_keys(parent_table)
        .map(|(child, i, fk)| {
            (
                child.oid,
                i,
                ReferencingFk::from_catalog(child, fk, fk.on_update),
            )
        })
        .collect();
    fks.sort_by_key(|(oid, i, _)| (*oid, *i));
    fks.into_iter().map(|(_, _, fk)| fk).collect()
}

/// A foreign key that references a table, resolved to the column positions of both of its
/// tables — what an action event of [`FkTriggerQueue`] fires.
struct ActionFk {
    fk: ReferencingFk,
    /// The referenced columns' positions in the parent table.
    parent_positions: Vec<usize>,
    /// The referencing columns' positions in the child table.
    child_positions: Vec<usize>,
}

/// One event of a statement's foreign-key trigger queue ([`FkTriggerQueue`]).
enum FkEvent {
    /// A referenced key moved: fire `fk`'s `ON UPDATE` action over the rows that reference
    /// `old_key` when the event fires.
    Action {
        parent_table: String,
        fk: Rc<ActionFk>,
        old_key: Vec<Value>,
        new_key: Vec<Value>,
    },
    /// Check a written row's own foreign key — the `fk`-th of `table`'s — over the row as it
    /// stands, unless an action has rewritten the row since: `version` counts the action
    /// rewrites the row had when the event was queued.
    Check {
        table: String,
        id: RowId,
        fk: usize,
        version: u32,
    },
}

/// A child table's CHECK predicates and indexes, prepared once per statement for the rows
/// an action rewrites there.
struct PreparedTable {
    checks: Vec<(String, Expr)>,
    indexes: Vec<PreparedIndex>,
}

/// The foreign-key work of an `UPDATE` and of an `ON CONFLICT DO UPDATE`, run as 18.6 runs
/// it: as after-row triggers, queued as each row is written and fired in order, one queue,
/// once the statement's rows are written.
///
/// 18.6 (`trigger.c`, `ri_triggers.c`):
/// - A written row queues its triggers in trigger-name order — every action on a foreign
///   key that references the row's table (`RI_ConstraintTrigger_a_*`), then every check of
///   the row's own foreign keys (`RI_ConstraintTrigger_c_*`). An action is queued only when
///   the referenced key changed and the old one holds no NULL
///   (`RI_FKey_pk_upd_check_required`); a check only when the new key holds no NULL and
///   either the key changed or the row's old version is the transaction's own write
///   (`RI_FKey_fk_upd_check_required`). An inserted row queues a check of each of its own
///   keys that holds no NULL, which no key change gates (`RI_FKey_check_ins`).
/// - An action runs its own `UPDATE` of the child through SPI with `fire_triggers = false`,
///   so the value it assigns reaches each referencing column through that column's
///   assignment conversion ([`action_assigned_values`]), each child row it rewrites is
///   constrained as that `UPDATE` writes it — NOT NULL, CHECK, then each index entry — and
///   queues its own triggers at the **end** of the same list. `AfterTriggerEndQuery` fires
///   the list until nothing is left: first in, first out.
/// - A check of a version an action has since rewritten is skipped (`RI_FKey_check`'s
///   `SnapshotSelf` test); the rewrite queued its own.
/// - `NO ACTION` first asks whether a row of the referenced table now holds the old key
///   (`ri_Check_Pk_Match`), `RESTRICT` never; each then refuses if a row still references
///   it.
///
/// So a row's own key is checked before the next row's actions run, a key that moves twice
/// in one statement moves its referencing rows twice, and every refusal is raised by the
/// event whose turn it is. Each event reads the tables as the statement has left them so
/// far — storage, which every earlier event has already written.
///
/// Some places this and 18.6 part: whether a row's old version is its transaction's own write
/// from an **earlier** statement, which a row this statement wrote answers and one an
/// earlier statement of the same block wrote does not (`EDGES.md` row 165's residual); the
/// order of the actions on the keys referencing one table, the referencing table's and then
/// its keys' where 18.6 takes its triggers' names ([`referencing_fks_on_update`], row 198);
/// and an `INSERT`'s and an `ON CONFLICT DO UPDATE`'s own check of each row as they write
/// it, ahead of the queue (row 166) — an `ON CONFLICT DO UPDATE` over a referenced table
/// queues its rows here as well, so that check can only add a refusal. And two of the
/// action's own: whether a referenced key changed, which pglet asks of its value and 18.6 of
/// its bytes (row 199); and a conversion refused over a key no child row references, which
/// 18.6 refuses only while the action's plan is a custom one ([`action_assigned_values`],
/// row 200).
///
/// A refusal leaves the statement's earlier writes to its rollback, which `--storage
/// memory` does not have (`EDGES.md` row 40).
struct FkTriggerQueue<'a> {
    schema: &'a Schema,
    events: VecDeque<FkEvent>,
    /// The rows the statement itself wrote, by table — a later action rewriting one of them
    /// checks every one of its keys, changed or not. Kept only for a table some foreign key
    /// references, the only kind an action can reach.
    statement_rows: HashMap<String, HashSet<RowId>>,
    /// How many times an action has rewritten each row, by table (absent: none).
    versions: HashMap<String, HashMap<RowId, u32>>,
    /// Each table's referencing foreign keys, in firing order.
    actions: HashMap<String, Rc<[Rc<ActionFk>]>>,
    /// Each table's own foreign keys' referencing-column positions, parallel to its
    /// `foreign_keys` (`None` for a key naming no column).
    own_fks: HashMap<String, Rc<[Option<Vec<usize>>]>>,
    /// Each child table an action has rewritten a row of, prepared once.
    prepared: HashMap<String, Rc<PreparedTable>>,
}

impl<'a> FkTriggerQueue<'a> {
    fn new(schema: &'a Schema) -> Self {
        Self {
            schema,
            events: VecDeque::new(),
            statement_rows: HashMap::new(),
            versions: HashMap::new(),
            actions: HashMap::new(),
            own_fks: HashMap::new(),
            prepared: HashMap::new(),
        }
    }

    /// The foreign keys referencing `table`, in firing order, resolved.
    fn actions_for(&mut self, table: &str) -> Rc<[Rc<ActionFk>]> {
        if let Some(fks) = self.actions.get(table) {
            return Rc::clone(fks);
        }
        let schema = self.schema;
        let resolved: Rc<[Rc<ActionFk>]> = schema.get_table(table).map_or_else(
            || Rc::from(Vec::new()),
            |parent_def| {
                referencing_fks_on_update(schema, table)
                    .into_iter()
                    .filter_map(|fk| {
                        let child_def = schema.get_table(&fk.child_table)?;
                        let parent_positions = fk
                            .parent_columns
                            .iter()
                            .map(|pc| parent_def.columns.iter().position(|c| &c.name == pc))
                            .collect::<Option<Vec<usize>>>()?;
                        let child_positions = fk
                            .child_columns
                            .iter()
                            .map(|cc| child_def.columns.iter().position(|c| &c.name == cc))
                            .collect::<Option<Vec<usize>>>()?;
                        Some(Rc::new(ActionFk {
                            fk,
                            parent_positions,
                            child_positions,
                        }))
                    })
                    .collect()
            },
        );
        self.actions.insert(table.to_string(), Rc::clone(&resolved));
        resolved
    }

    /// `table`'s own foreign keys' referencing-column positions.
    fn own_fks_for(&mut self, table: &str) -> Rc<[Option<Vec<usize>>]> {
        if let Some(fks) = self.own_fks.get(table) {
            return Rc::clone(fks);
        }
        let resolved: Rc<[Option<Vec<usize>>]> = self.schema.get_table(table).map_or_else(
            || Rc::from(Vec::new()),
            |table_def| {
                table_def
                    .foreign_keys
                    .iter()
                    .map(|fk| fk_child_positions(table_def, fk))
                    .collect()
            },
        );
        self.own_fks.insert(table.to_string(), Rc::clone(&resolved));
        resolved
    }

    /// How many times an action has rewritten the row.
    fn version(&self, table: &str, id: RowId) -> u32 {
        self.versions
            .get(table)
            .and_then(|rows| rows.get(&id))
            .copied()
            .unwrap_or(0)
    }

    /// Record a row the statement wrote, so an action that rewrites it checks every one of
    /// its keys. Kept only for a table some foreign key references, the only kind an action
    /// can reach.
    fn note_statement_row(&mut self, table: &str, id: RowId) {
        if !self.actions_for(table).is_empty() {
            self.statement_rows
                .entry(table.to_string())
                .or_default()
                .insert(id);
        }
    }

    /// Queue the triggers of a row the statement rewrote, `old` → `new`: its actions, then
    /// its own key checks.
    fn queue_statement_row(&mut self, table: &str, id: RowId, old: &[Value], new: &[Value]) {
        self.note_statement_row(table, id);
        self.queue_row(table, id, old, new, false, Some(0));
    }

    /// Queue the triggers of a row the statement inserted: a check of each of its own keys
    /// that holds no NULL (`RI_FKey_check_ins`, which no key change gates).
    ///
    /// # Errors
    ///
    /// A storage error reading the row.
    fn queue_inserted_row<S: Storage>(
        &mut self,
        table: &str,
        id: RowId,
        storage: &S,
    ) -> Result<()> {
        self.note_statement_row(table, id);
        let Some(row) = storage.get(table, id)? else {
            return Ok(());
        };
        self.queue_row(table, id, &row, &row, true, Some(0));
        Ok(())
    }

    /// Queue a written row's triggers: an action for each referencing foreign key whose
    /// referenced key moved, then — when `check_version` is given — a check of each of the
    /// row's own keys that holds no NULL and changed, or of every such key when
    /// `written_before` (the old version is the statement's own write).
    fn queue_row(
        &mut self,
        table: &str,
        id: RowId,
        old: &[Value],
        new: &[Value],
        written_before: bool,
        check_version: Option<u32>,
    ) {
        for fk in self.actions_for(table).iter() {
            let old_key = parent_key_tuple(old, &fk.parent_positions);
            if old_key.iter().any(Value::is_null) {
                continue;
            }
            let new_key = parent_key_tuple(new, &fk.parent_positions);
            if key_tuples_equal(&old_key, &new_key) {
                continue;
            }
            self.events.push_back(FkEvent::Action {
                parent_table: table.to_string(),
                fk: Rc::clone(fk),
                old_key,
                new_key,
            });
        }
        let Some(version) = check_version else {
            return;
        };
        for (fk, positions) in self.own_fks_for(table).iter().enumerate() {
            let Some(positions) = positions else {
                continue;
            };
            // MATCH SIMPLE: a NULL in the new key satisfies the constraint.
            if positions
                .iter()
                .any(|&p| new.get(p).is_none_or(Value::is_null))
            {
                continue;
            }
            if !written_before
                && key_tuples_equal(
                    &parent_key_tuple(old, positions),
                    &parent_key_tuple(new, positions),
                )
            {
                continue;
            }
            self.events.push_back(FkEvent::Check {
                table: table.to_string(),
                id,
                fk,
                version,
            });
        }
    }

    /// Fire the queue, first in first out, until no event is left.
    ///
    /// # Errors
    ///
    /// `Error::ForeignKeyViolation` / `Error::RestrictViolation` / `Error::NotNullViolation`
    /// / `Error::CheckViolation` / `Error::UniqueViolation` / an index-size refusal / an
    /// action's conversion refusal ([`action_assigned_values`]) — the first the queue
    /// raises; a storage or evaluation error.
    fn fire<S: Storage>(mut self, storage: &mut S) -> Result<()> {
        while let Some(event) = self.events.pop_front() {
            match event {
                FkEvent::Check {
                    table,
                    id,
                    fk,
                    version,
                } => self.fire_check(&*storage, &table, id, fk, version)?,
                FkEvent::Action {
                    parent_table,
                    fk,
                    old_key,
                    new_key,
                } => self.fire_action(storage, &parent_table, &fk, &old_key, &new_key)?,
            }
        }
        Ok(())
    }

    /// A row's own key check: skipped when an action has rewritten the row since it was
    /// queued, else [`enforce_foreign_key`] over the row as it stands.
    fn fire_check<S: Storage>(
        &mut self,
        storage: &S,
        table: &str,
        id: RowId,
        fk: usize,
        version: u32,
    ) -> Result<()> {
        if self.version(table, id) != version {
            return Ok(());
        }
        let schema = self.schema;
        let Some(table_def) = schema.get_table(table) else {
            return Ok(());
        };
        let (Some(fk_def), Some(Some(positions))) = (
            table_def.foreign_keys.get(fk),
            self.own_fks_for(table).get(fk).cloned(),
        ) else {
            return Ok(());
        };
        let Some(row) = storage.get(table, id)? else {
            return Ok(());
        };
        enforce_foreign_key(storage, schema, table_def, fk_def, &positions, &row)
    }

    /// An `ON UPDATE` action over the rows referencing `old_key` as they stand: `CASCADE`
    /// repoints each to `new_key` and `SET NULL` nulls it, the assigned value converted to
    /// each referencing column once, before the rows are looked up
    /// ([`action_assigned_values`]), and each rewrite constrained and written before the next
    /// row's ([`FkTriggerQueue::rewrite`]); `NO ACTION` refuses unless a row of the
    /// referenced table now holds `old_key`, and `RESTRICT` refuses.
    fn fire_action<S: Storage>(
        &mut self,
        storage: &mut S,
        parent_table: &str,
        fk: &ActionFk,
        old_key: &[Value],
        new_key: &[Value],
    ) -> Result<()> {
        let schema = self.schema;
        let edge = &fk.fk;
        match edge.action {
            ReferentialAction::Cascade | ReferentialAction::SetNull => {
                let Some(child_def) = schema.get_table(&edge.child_table) else {
                    return Ok(());
                };
                let cascade = matches!(edge.action, ReferentialAction::Cascade);
                let assigned = action_assigned_values(
                    schema,
                    child_def,
                    &fk.child_positions,
                    cascade.then_some(new_key),
                )?;
                let children = find_referencing_children(
                    &*storage,
                    None,
                    &edge.child_table,
                    &fk.child_positions,
                    old_key,
                )?;
                for (child_id, child_row) in children {
                    let mut image = child_row.clone();
                    for (&pos, value) in fk.child_positions.iter().zip(&assigned) {
                        if let Some(slot) = image.get_mut(pos) {
                            *slot = value.clone();
                        }
                    }
                    self.rewrite(storage, child_def, child_id, &child_row, image)?;
                }
                Ok(())
            }
            action @ (ReferentialAction::NoAction | ReferentialAction::Restrict) => {
                if matches!(action, ReferentialAction::NoAction)
                    && parent_tuple_exists(
                        &*storage,
                        schema,
                        parent_table,
                        &edge.parent_columns,
                        old_key,
                    )?
                {
                    return Ok(());
                }
                let referenced = !find_referencing_children(
                    &*storage,
                    None,
                    &edge.child_table,
                    &fk.child_positions,
                    old_key,
                )?
                .is_empty();
                match schema.get_table(parent_table) {
                    Some(parent_def) if referenced => Err(fk_block_violation(
                        action,
                        parent_table,
                        &edge.fk_name,
                        &edge.child_table,
                        parent_key_description(
                            parent_def,
                            &edge.parent_columns,
                            &fk.parent_positions,
                            old_key,
                        ),
                    )),
                    _ => Ok(()),
                }
            }
        }
    }

    /// Write the image an action gives child row `id`, constrained as 18.6 constrains the row
    /// the action's own `UPDATE` writes — NOT NULL, CHECK, then each index entry in creation
    /// order ([`enforce_row_checks`]) — and queue the rewritten row's own triggers behind
    /// every event already queued.
    fn rewrite<S: Storage>(
        &mut self,
        storage: &mut S,
        child_def: &Table,
        id: RowId,
        old: &[Value],
        image: Vec<Value>,
    ) -> Result<()> {
        let table = child_def.name.as_str();
        let prepared = if let Some(prepared) = self.prepared.get(table) {
            Rc::clone(prepared)
        } else {
            let prepared = Rc::new(PreparedTable {
                checks: prepare_checks(child_def),
                indexes: prepare_indexes(self.schema, table),
            });
            self.prepared
                .insert(table.to_string(), Rc::clone(&prepared));
            prepared
        };
        let names: Vec<&str> = child_def.columns.iter().map(|c| c.name.as_str()).collect();
        let types: Vec<TypeOid> = child_def.columns.iter().map(|c| c.type_oid).collect();
        enforce_row_checks(
            &*storage,
            self.schema,
            table,
            &child_def.columns,
            &prepared.checks,
            &names,
            &types,
            &prepared.indexes,
            &image,
            Some(id),
        )?;
        let written_before = self.version(table, id) > 0
            || self
                .statement_rows
                .get(table)
                .is_some_and(|rows| rows.contains(&id));
        let version = {
            let slot = self
                .versions
                .entry(table.to_string())
                .or_default()
                .entry(id)
                .or_insert(0);
            *slot += 1;
            *slot
        };
        self.queue_row(table, id, old, &image, written_before, Some(version));
        storage.update(table, id, image)?;
        Ok(())
    }
}

#[cfg(test)]
mod default_resolution_tests {
    use std::collections::BTreeMap;

    use super::resolve_default;
    use crate::catalog::{Column, ColumnDefault, Sequence};
    use crate::types::{TypeOid, Value};

    fn nextval_column(sequence: &str) -> Column {
        Column {
            name: "id".to_string(),
            type_oid: TypeOid::Int8,
            nullable: false,
            position: 1,
            default: Some(ColumnDefault::NextVal(sequence.to_string())),
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            temporal_precision: None,
            identity: None,
        }
    }

    /// A `NextVal` default naming a sequence the catalog lacks is `42P01`, never a
    /// freshly minted counter at 1.
    ///
    /// **This guard had no SQL reproduction when it was written, and that claim was
    /// wrong** — four routes reached the state, and the claim is worth keeping in view
    /// because of *how* it was wrong. The reasoning was sound as far as it went: every
    /// route that records such a default registers the sequence in the same statement,
    /// and every route that removes a sequence either takes the default with it or
    /// refuses. What it missed is that the second half held only for a column the
    /// resolver could *see*. `Dependents::resolve` exempted every **identity** column
    /// from the `pg_attrdef` edge — right about Postgres, which stores no such row —
    /// while pglet models the link as an ordinary `NextVal` default that two statements
    /// could take apart (`ALTER SEQUENCE … OWNED BY`, `ALTER COLUMN … SET/DROP
    /// DEFAULT`). Detach the link and the sequence is nobody's dependent; drop it and
    /// the default names nothing. Every write was then `42P01` — this guard, doing its
    /// job on a state it was documented as unable to reach — and the dump emitted the
    /// `DEFAULT nextval(…)` with no `CREATE SEQUENCE`, which stock Postgres refuses and
    /// pglet's own restore silently renumbers from 1.
    ///
    /// Both statements are now refused and the exemption is keyed on being the
    /// sequence's *own* column rather than on the identity flag, so the four routes are
    /// closed and every removal site is accounted for (`executor::dependencies`,
    /// `execute_drop_sequence`, `DROP SCHEMA public CASCADE`, which clears the tables
    /// with the sequences). One route remains and is not a parser route: a store that
    /// already holds the state holds it on reopen. That window is narrow, and bounded
    /// at both ends by the same store magic: the state is not *representable* until a
    /// sequence becomes a record carrying an `owner`, which is the bump to the current
    /// magic — and that is also the only magic that still opens, every superseded one
    /// failing loud. So the store must carry today's magic and predate the gates.
    ///
    /// So the guard is kept, and the general form of what it stands for is the lesson:
    /// "no route reaches this" is a claim about an enumeration, and an enumeration over
    /// *removal sites* silently assumed the resolver reports every dependent. It
    /// converts a second table's ids silently restarting from 1 into an error naming
    /// the missing sequence. A test that could only be written *after* such a hole
    /// existed would be a test written too late — which is what happened here, and the
    /// test was already in place.
    #[test]
    fn a_missing_sequence_is_loud_rather_than_minted_at_one() {
        let mut sequences: BTreeMap<String, Sequence> = BTreeMap::new();
        let column = nextval_column("gone");

        let err = resolve_default(&column, &mut sequences)
            .expect_err("an unregistered sequence must not be minted");
        assert_eq!(err.sqlstate(), "42P01");
        assert!(
            sequences.is_empty(),
            "the failed resolution must not register the sequence either"
        );

        // The registered case is unchanged: the counter advances from `start - 1`.
        sequences.insert("gone".to_string(), Sequence::new(7));
        assert_eq!(
            resolve_default(&column, &mut sequences).unwrap(),
            Value::Int8(7)
        );
    }
}
