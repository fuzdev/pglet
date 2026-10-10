//! Query execution engine.
//!
//! This module is organized as follows:
//! - `types.rs` - `QueryResult`, Storage trait, `ColumnDesc`
//! - `expr/` - Expression evaluation (literals, columns, operators)
//! - `coerce.rs` - Value coercion (explicit `CAST` + implicit type coercion)
//! - `array_parse.rs` - Postgres array-literal text parsing
//! - `ddl.rs` - CREATE, DROP, ALTER execution
//! - `dependencies.rs` - DROP-time dependency resolution (auto-drop vs `2BP01`)
//! - `dml.rs` - INSERT, UPDATE, DELETE execution
//! - `query.rs` - SELECT execution (scans, filters, projections, aggregates)
//! - `constraints.rs` - Write-path constraint enforcement (enum, NOT NULL, CHECK, UNIQUE, FK)
//! - `index_row.rs` - 18.6's index-entry size and its `54000` refusals
//! - `type_resolution.rs` - DDL type-name → `TypeOid` resolution
//! - `helpers.rs` - Projection/descriptor helpers + OID generation

mod array_parse;
mod coerce;
mod constraints;
mod copy;
mod ddl;
mod dependencies;
mod dml;
mod expr;
mod helpers;
pub mod index_row;
mod md5;
mod query;
mod sequence;
mod type_resolution;
mod types;
#[cfg(test)]
mod walker_pins;

use crate::Result;
use crate::catalog::Schema;
use crate::planner::{CteBody, DoExceptionHandler, Plan, UnperformedAlter};

// Re-export public types. The `Storage` trait + `NoopStorage` live in the
// `pglet_core` leaf crate (one definition shared with the backends); re-export
// them so the `executor::Storage` / `executor::NoopStorage` paths keep resolving
// for internal callers and tests.
pub use coerce::coerce_value;
pub use coerce::text_to_bool;
pub use copy::{
    CopyParams, copy_column_count, copy_unescape_field, execute_copy_from, execute_copy_to,
    resolve_columns,
};
pub use expr::{
    EnumLeaves, EvalEnv, MaterializedCte, boolean_context_operand, eval_expr, eval_expr_env,
    expr_enum_with, predicate_holds, subquery_enum_name, unify_enum_pair, values_compare,
};
// The one read-only table lookup, re-exported for the server's COPY pre-flight — which
// resolves the target *before* the engine does and would otherwise answer `42P01` where
// this answers `42809` for a sequence.
pub use dependencies::default_sequence_refs;
// The owner edge a `DROP` removes a sequence over, read by `Plan::swapped_sequences` for the
// cells an owner drop replaces — the same derivation the drop removes them through.
pub(crate) use dependencies::{DropTarget, owned_sequences};
// The storage index specs follow `Schema.enums` wherever it moves; the DDL that moves
// it forward is in this crate, and the rollback that moves it back is `pglet`'s
// `TransactionSession` — so the resync it calls is re-exported here.
pub use ddl::{enums_moved_by_restore, rebuild_indexes_over_enums};
pub use helpers::{get_table_or_err, unified_enum_type};
pub use pglet_core::{NoopStorage, Storage};
pub use sequence::SequenceSession;
pub use types::{
    ColumnDesc, IndexMethod, IndexSpec, IsolationLevel, QueryResult, ResultShape, RowId,
    TransactionCommand, index_spec_from_catalog,
};
// `index_method_for` (the GIN/B-tree derivation) is crate-internal: the executor builds
// the `IndexSpec` with it, `pg_catalog` renders `indexdef` through it (`IndexMethod::name`),
// and the planner's index selection gates on it — so neither the reported method nor
// the one selection offers can drift from the one storage builds.
pub(crate) use types::index_method_for;

// Re-export `type_name_to_oid` (tests) and the two type-name resolvers (enum-aware
// name → `TypeOid`): `resolve_column_type` for `gram.y`'s `Typename` — the DDL
// executor and the parser's `CAST`/`::` path — and `resolve_constant_type` for
// `ConstTypename`, the typed-constant position. The two functions differ in the
// fixed-length default a **parenless** `char` takes and in nothing else; the
// productions they stand for differ in a good deal more, which `TypeNamePosition`
// states. The
// write-path constraint *enforcement* helpers (`enforce_*` / `prepare_*` /
// `PreparedIndex`) are NOT re-exported — sibling `executor` modules reach them via
// `super::constraints::…`, and nothing outside the crate consumes them. `validate_column`
// is re-exported to the crate alone, below.
pub use type_resolution::{
    ResolvedColumnType, resolve_column_type, resolve_constant_type, type_name_to_oid,
};
// The planner's `INSERT … VALUES` lowering puts a literal through its column's enum
// membership check when a later row's length refusal would otherwise outrank it — the
// same check the write makes.
pub(crate) use constraints::validate_column;

/// 18.6's sentence for an `Expr::Default` reaching a position that cannot hold one
/// (`42601`).
///
/// **One string, three raisers**, which is why it is a const rather than three
/// literals: the general evaluator (`SELECT default`), the stored-expression node
/// walk, and the stored-expression *binding* check all stand for the same 18.6
/// verdict, and a fixture comparing SQLSTATEs alone cannot tell them apart — so a
/// divergence in one spelling would be invisible until a client read the message.
pub(super) const DEFAULT_NOT_ALLOWED: &str = "DEFAULT is not allowed in this context";

/// Executes a plan against the schema.
///
/// # Errors
///
/// Returns an error if execution fails (e.g., table not found, type mismatch).
pub fn execute(plan: &Plan, schema: &mut Schema) -> Result<QueryResult> {
    // A throwaway per-call session: the no-storage `execute` entry point has no
    // durable session, so `currval` across calls isn't meaningful here (the server
    // / embed drivers own a real per-connection `SequenceSession`).
    let mut session = SequenceSession::new();
    execute_with_storage(plan, schema, &mut NoopStorage, EvalEnv::EMPTY, &mut session)
}

/// Executes a plan against the schema with storage.
///
/// `env` carries the extended-protocol portal's bound parameters (`EvalEnv::EMPTY`
/// for the simple-query / embed / wasm paths). An `Expr::Parameter` resolves from
/// it at eval time — the plan is never mutated by binding.
///
/// `session` is the caller's per-connection [`SequenceSession`] — the sequence
/// functions' per-session state (`currval`), threaded at the **statement**
/// boundary, never into the read-only expression eval.
///
/// # Errors
///
/// Returns an error if execution fails (e.g., table not found, type mismatch).
pub fn execute_with_storage<S: Storage>(
    plan: &Plan,
    schema: &mut Schema,
    storage: &mut S,
    env: EvalEnv<'_>,
    session: &mut SequenceSession,
) -> Result<QueryResult> {
    crate::depth::check_stack()?;
    // A DDL statement changes the schema shape, so any plan held against the old
    // schema is now stale. Bump the generation here — the single chokepoint every
    // DDL path funnels through — so a plan held against the old schema (a server
    // prepared statement, a facade plan-cache entry) is planned again.
    // (Bumping before execution over-invalidates on a failed DDL, which is harmless.)
    if plan.mutates_schema() {
        schema.generation = schema.generation.wrapping_add(1);
    }
    if let Some((table, true)) = plan.alter_table_target()
        && schema.relation_kind(table).is_none()
    {
        return Ok(alter_table_skipped(table));
    }
    match plan {
        // The clause locks no row: what it carries is for the host (`Plan::takes_writer`).
        Plan::LockRows { input } => execute_with_storage(input, schema, storage, env, session),
        Plan::CreateTable {
            name,
            columns,
            checks,
            unique_constraints,
            foreign_keys,
            if_not_exists,
        } => ddl::execute_create_table(
            schema,
            storage,
            name,
            columns,
            checks,
            unique_constraints,
            foreign_keys,
            *if_not_exists,
        ),

        Plan::DropTable {
            names,
            if_exists,
            cascade,
        } => ddl::execute_drop_table(schema, storage, names, *if_exists, *cascade),

        Plan::CreateIndex {
            name,
            table,
            key,
            unique,
            enforced,
            predicate,
            nulls_not_distinct,
            method,
            if_not_exists,
            concurrently: _,
        } => ddl::execute_create_index(
            schema,
            storage,
            name.as_deref(),
            table,
            key.clone(),
            *unique,
            *enforced,
            predicate.clone(),
            *nulls_not_distinct,
            *method,
            *if_not_exists,
        ),

        Plan::DropIndex {
            names,
            if_exists,
            concurrently: _,
        } => ddl::execute_drop_index(schema, storage, names, *if_exists),

        Plan::CreateType { name, members } => {
            ddl::execute_create_type(schema, storage, name, members)
        }

        Plan::DropType {
            name,
            if_exists,
            cascade,
        } => ddl::execute_drop_type(schema, storage, name, *if_exists, *cascade),

        Plan::DropSchema {
            name,
            if_exists,
            cascade,
        } => ddl::execute_drop_schema(schema, storage, name, *if_exists, *cascade),

        Plan::CreateSchema {
            name,
            if_not_exists,
        } => ddl::execute_create_schema(name, *if_not_exists),

        Plan::AlterTableAddColumn {
            table,
            column,
            foreign_key,
            column_uniques,
            checks,
            if_not_exists,
            table_if_exists: _,
        } => ddl::execute_alter_table_add_column(
            schema,
            storage,
            table,
            column,
            foreign_key.as_ref(),
            column_uniques,
            checks,
            *if_not_exists,
        ),

        Plan::AlterTableDropColumn {
            table,
            column,
            if_exists,
            cascade,
            table_if_exists: _,
        } => ddl::execute_alter_table_drop_column(
            schema, storage, table, column, *if_exists, *cascade,
        ),

        Plan::AlterTableAddCheck { table, check, .. } => {
            ddl::execute_alter_table_add_check(schema, storage, table, check)
        }

        Plan::AlterTableAddConstraint {
            table, constraint, ..
        } => ddl::execute_alter_table_add_constraint(schema, storage, table, constraint),

        Plan::AlterTableSetDefault {
            table,
            column,
            default,
            ..
        } => ddl::execute_alter_table_set_default(schema, table, column, default.as_ref()),

        Plan::AlterTableSetNotNull { table, column, .. } => {
            ddl::execute_alter_table_set_not_null(schema, storage, table, column)
        }

        Plan::AlterTableAddIdentity {
            table,
            column,
            generation,
            start,
            sequence_name,
            table_if_exists: _,
        } => ddl::execute_alter_table_add_identity(
            schema,
            table,
            column,
            *generation,
            *start,
            sequence_name.as_deref(),
        ),

        Plan::AlterTableUnperformed { table, answer, .. } => match answer {
            UnperformedAlter::Tolerated => Ok(QueryResult::ddl("ALTER TABLE")),
            UnperformedAlter::Refused(_) if schema.relation_kind(table).is_none() => {
                Err(crate::CatalogError::TableNotFound {
                    table: table.clone(),
                }
                .into())
            }
            UnperformedAlter::Refused(refusal) => {
                Err(crate::Error::NotImplemented((*refusal).to_string()))
            }
            UnperformedAlter::Skipped => Ok(alter_table_skipped(table)),
        },

        Plan::CreateSequence {
            name,
            start,
            restart,
            if_not_exists,
            owned_by,
        } => ddl::execute_create_sequence(
            schema,
            name,
            *start,
            *restart,
            *if_not_exists,
            owned_by.as_ref(),
        ),

        Plan::AlterSequence {
            name,
            if_exists,
            start,
            restart,
            owned_by,
        } => ddl::execute_alter_sequence(
            schema,
            name,
            *if_exists,
            *start,
            *restart,
            owned_by.as_ref(),
        ),

        Plan::DropSequence {
            names,
            if_exists,
            cascade,
        } => ddl::execute_drop_sequence(schema, names, *if_exists, *cascade),

        // pg_dump-preamble tolerance statements: parsed, no semantic effect, report
        // only their command tag.
        Plan::Noop { tag } => Ok(QueryResult::ddl(tag.command_tag())),

        Plan::Insert {
            table,
            rows,
            returning,
            on_conflict,
            // `overriding` is consumed at lowering (identity-value handling); the
            // executor needs no further action.
            overriding: _,
        } => dml::execute_insert(
            schema,
            storage,
            table,
            rows,
            returning,
            on_conflict.as_ref(),
            env,
            session,
        ),

        Plan::Update {
            table,
            alias,
            from,
            assignments,
            filter,
            target_probe,
            returning,
        } => dml::execute_update(
            schema,
            storage,
            table,
            alias.as_deref(),
            from.as_deref(),
            assignments,
            filter.as_ref(),
            target_probe.as_deref(),
            returning,
            env,
        ),

        Plan::Delete {
            table,
            filter,
            target_probe,
            returning,
        } => dml::execute_delete(
            schema,
            storage,
            table,
            filter.as_ref(),
            target_probe.as_deref(),
            returning,
            env,
        ),

        Plan::Truncate {
            tables,
            cascade,
            restart_identity,
        } => ddl::execute_truncate(schema, storage, tables, *cascade, *restart_identity),

        // Materialized CTEs: run each entry once, in order — a data-modifying CTE's DML
        // against the write-path storage, a recursive CTE's fixed-point loop — each with
        // every earlier entry bound, then run the body with all of them bound. The DML
        // is never re-executed at a CTE's reference site (a `CteScan` reads the
        // materialized rows), and the plan is never mutated to bind them.
        Plan::With { ctes, body } => {
            let mut materialized: Vec<MaterializedCte> = Vec::with_capacity(ctes.len());
            for cte in ctes {
                let relation = match &cte.body {
                    CteBody::Modifying {
                        plan,
                        column_aliases,
                    } => {
                        let result = execute_with_storage(
                            plan,
                            schema,
                            storage,
                            env.with_ctes(&materialized),
                            session,
                        )?;
                        MaterializedCte {
                            name: cte.name.clone(),
                            // The materialized relation's columns come from the DML's own
                            // execution, so the CTE's `name(cols)` list is applied here —
                            // this is the executed-side reader, the one the reference
                            // site's `Plan::CteScan.columns` cannot stand in for
                            // (`Plan::CteScan`'s executor arm returns *these* columns,
                            // not the plan's).
                            columns: crate::planner::alias_relation_columns(
                                result.shape.into_columns(),
                                column_aliases,
                            ),
                            rows: result.rows,
                        }
                    }
                    CteBody::Recursive {
                        columns,
                        non_recursive,
                        recursive,
                        all,
                    } => query::materialize_recursive_cte(
                        &cte.name,
                        columns,
                        non_recursive,
                        recursive,
                        *all,
                        schema,
                        storage,
                        env,
                        &mut materialized,
                    )?,
                };
                materialized.push(relation);
            }
            // The body through the statement dispatcher, not the query one: a
            // data-modifying body (`WITH … INSERT INTO t SELECT … FROM c`) is a write
            // that reads the materialized relations. The parser admits a body that writes
            // only where no entry writes (`Parser::wrap_with_ctes`), so the entries it
            // follows are recursive ones, reads run before it as 18.6's snapshot has them.
            execute_with_storage(body, schema, storage, env.with_ctes(&materialized), session)
        }

        // COPY moves its rows over an out-of-band byte stream, which the in-band
        // `QueryResult` channel can't carry. The embed `Database::copy_in`/`copy_out`
        // API (and the server's CopyData path) call `execute_copy_from`/`copy_to`
        // directly with the data slice; routing a `Plan::Copy` through here is a
        // programming error, so fail loud rather than silently produce nothing.
        Plan::Copy { direction, .. } => Err(crate::Error::NotImplemented(format!(
            "COPY {} requires the streaming copy_in/copy_out API (no byte channel in execute)",
            match direction {
                crate::planner::CopyDirection::From => "FROM STDIN",
                crate::planner::CopyDirection::To => "TO STDOUT",
            }
        ))),

        Plan::Empty => Ok(QueryResult::ddl("ALTER TABLE")),

        // A minimal `DO $$ … $$` block: run the body statements, swallowing a
        // matching error via the optional EXCEPTION handler (the idempotent
        // `CREATE TYPE` idiom). See `execute_do_block`.
        Plan::DoBlock { body, exception } => {
            execute_do_block(body, exception.as_ref(), schema, storage, env, session)
        }

        // SELECT queries use a recursive execution model. The index-scan variants
        // exist only with acceleration on; they route through `execute_query` too.
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { .. } | Plan::IndexRangeScan { .. } | Plan::GinScan { .. } => {
            query::execute_query(plan, schema, storage, env)
        }
        // A top-level `SELECT nextval/currval/setval(...)` projection is the one
        // place a sequence function is evaluated — with `&mut Schema` + the session
        // state — and a top-level `SELECT pg_advisory_xact_lock(…)` the one place that
        // call answers, the host having taken the writer for it. Every other no-FROM
        // `VALUES` routes through `execute_query` (so its
        // projections evaluate with subquery context — required for a no-FROM
        // `SELECT EXISTS(SELECT ... FROM t ...)` like the spine's `query_has_fact`).
        Plan::Values { rows, .. } => {
            if plan.is_sequence_projection() || plan.is_advisory_lock_projection() {
                // Both predicates guarantee a single-row projection.
                sequence::execute_sequence_projection(&rows[0], schema, storage, session, env)
            } else {
                query::execute_query(plan, schema, storage, env)
            }
        }

        Plan::Scan { .. }
        | Plan::VirtualScan { .. }
        | Plan::Filter { .. }
        | Plan::Project { .. }
        | Plan::ProjectUnnest { .. }
        | Plan::Sort { .. }
        | Plan::Distinct { .. }
        | Plan::DistinctOn { .. }
        | Plan::JunkFilter { .. }
        | Plan::Limit { .. }
        | Plan::Aggregate { .. }
        | Plan::Join { .. }
        | Plan::SetOp { .. }
        | Plan::SubqueryScan { .. }
        | Plan::CteScan { .. }
        | Plan::FunctionScan { .. }
        | Plan::Window { .. } => query::execute_query(plan, schema, storage, env),

        // Transaction control — the block itself is the host's (see
        // `pglet::TransactionSession`), so these execute to nothing but a command tag
        // and the lifecycle signal the host applies afterwards. `ROLLBACK TO` reports
        // tag `ROLLBACK`, exactly as Postgres does, which is why the signal is a
        // field and not the tag.
        Plan::BeginTransaction { isolation } => Ok(QueryResult::transaction(
            "BEGIN",
            TransactionCommand::Begin(*isolation),
        )),
        Plan::CommitTransaction => Ok(QueryResult::transaction(
            "COMMIT",
            TransactionCommand::Commit,
        )),
        Plan::RollbackTransaction => Ok(QueryResult::transaction(
            "ROLLBACK",
            TransactionCommand::Rollback,
        )),
        Plan::Savepoint { name } => Ok(QueryResult::transaction(
            "SAVEPOINT",
            TransactionCommand::Savepoint(name.clone()),
        )),
        Plan::RollbackToSavepoint { name } => Ok(QueryResult::transaction(
            "ROLLBACK",
            TransactionCommand::RollbackTo(name.clone()),
        )),
        Plan::ReleaseSavepoint { name } => Ok(QueryResult::transaction(
            "RELEASE",
            TransactionCommand::Release(name.clone()),
        )),
    }
}

/// The `DO` block's half of the `25001` gate — the **third** site that asks
/// [`Plan::forbidden_in_transaction_block`], beside the two hosts.
///
/// A `DO` body or handler runs through `execute_with_storage` directly, below both hosts,
/// so a statement wrapped in one reached neither host's check: `DO $$ BEGIN CREATE INDEX
/// CONCURRENTLY … END $$` built the index, **including inside an open transaction
/// block**, where the bare statement is correctly refused — and so did the same statement
/// in a handler after its body raised.
///
/// The refusal here is **unconditional**, and that is 18.6's rule rather than a
/// simplification: `PreventInTransactionBlock` refuses these statements from a
/// function whether or not a block is open. Only the message differs between the
/// two cases there — `cannot be executed from a function` with no block open,
/// `cannot run inside a transaction block` with one — and pglet renders the second
/// for both, so the SQLSTATE agrees everywhere and the text agrees in the case the
/// fixture harness (whose wrapper *is* a block) can reach.
fn check_allowed_in_do_block(plan: &Plan) -> Result<()> {
    plan.forbidden_in_transaction_block()
        .map_or(Ok(()), |op| Err(crate::Error::ActiveSqlTransaction { op }))
}

/// Execute a minimal `DO $$ … $$` block: run each body statement in order; on the
/// first error, if an `EXCEPTION` handler is present and its condition matches the
/// error's SQLSTATE (or it is the `OTHERS` catch-all), skip the rest of the body
/// and run the handler — swallowing the error. Without a matching handler the error
/// propagates. The block returns a `DO` command tag.
///
/// # A `DO` is one statement, and a failed one leaves nothing behind
///
/// On 18.6 a `DO` is a single statement, so one that fails is rolled back whole, as any
/// failed statement is: every table, index, sequence and type its body or handler created is gone
/// and every object it dropped is back with its rows. The block therefore takes a
/// storage savepoint and a `Schema` snapshot before its first statement and puts both
/// back on **every** exit on which one of its statements raised — a body error with no
/// handler, a body error the handler's conditions do not match, and an error a handler
/// statement raises. The two savepoint calls' own errors are not such exits: taking the
/// level fails before anything ran, and releasing it after a success can fail only on a
/// pairing bug here.
///
/// The host's own statement rollback is not enough, and the catalog is why. Storage
/// discards an auto-commit statement's staged writes, and an enclosing block's `ROLLBACK`
/// restores its frame, but `Schema` is the executor's own `&mut`, which no backend knows
/// about, and an auto-commit statement has no frame: a single DDL statement is correct
/// only because it stages everything fallible before its first catalog write
/// (`docs/ddl.md`), and a body's *earlier* statements have finished their writes by the
/// time a later one fails. Without the restore here those writes stood in the session —
/// the catalog naming a table whose storage the rollback had discarded — and the next
/// successful auto-commit write persisted them.
///
/// The error returned is the one the failing statement raised, with one exception: an
/// undo that itself fails part-way (see [`undo_do_block`]) returns its own error, since
/// the state it leaves is the one the caller must hear about.
///
/// # What every `DO` pays for it
///
/// The level and the snapshot are **copies**, taken before the block knows whether it
/// will fail, so a `DO` that succeeds pays them too: a `Schema` clone, O(catalog), and
/// the storage savepoint — on the B+tree a clone of the transaction's staged writes, so
/// O(what an enclosing block has staged) and next to nothing in auto-commit, and on memory
/// storage a clone of the whole store, O(database), in auto-commit as in a block. They are
/// the copies an explicit `SAVEPOINT` takes, and the ones a `DO` with an `EXCEPTION`
/// clause needs for its subtransaction in any case. A mark in place of a copy would need
/// an undo log over the staged buffer and the memory store, which neither has; a cheaper
/// copy — the memory store's tables shared and copied on write — would not, and is not
/// built. The
/// measured cost is `docs/storage.md` § Known bottlenecks / scaling limits.
///
/// # The `EXCEPTION` clause is a subtransaction
///
/// A PL/pgSQL block with an `EXCEPTION` clause runs its body inside a subtransaction, so
/// **catching the error discards everything the body wrote** — a completed earlier
/// statement, a partially applied multi-row one, and DDL alike. Measured on 18.6: a body
/// that inserts two rows and then violates a unique constraint leaves **no** rows, and a
/// `CREATE TABLE` in the same body leaves no table.
///
/// The body's subtransaction and the statement's own undo start from the same state, so
/// they share one savepoint level and one snapshot: a caught error rolls back **to** the
/// level and keeps it, the handler runs above it, and the level is released only once the
/// handler has finished. Three details the shape depends on:
///
/// - **The handler runs after the body's rollback**, so a write the handler makes
///   survives the catch — in Postgres the subtransaction has already aborted by the time
///   the handler body runs. It is still inside the *statement*, so a handler that fails
///   takes its own writes back with the body's.
/// - **The catalog is restored through `Schema::restore_from`** — the same function the
///   whole-block and savepoint rollbacks use, so the monotonic stamps and the
///   sequence-counter rule cannot drift between them. `nextval` inside a failed or caught
///   body is **not** rolled back (measured on 18.6), which falls out of that shared rule
///   rather than being restated here; a counter cell a statement *replaced* is restored
///   from the swaps recorded for each statement as it ran — its `Plan::swapped_sequences`
///   list, read before it, and what a nested `DO` reports it replaced, read after it
///   ([`run_do_statements`]).
/// - **The storage index specs are the other half of the catalog.** A backward
///   `Schema::restore_from` moves `Schema.enums`, and `IndexSpec::column_enum_orders` is
///   a copy of a member order that outlives the statement that took it, while
///   `Storage::rebuild_index` publishes at once and no storage rollback takes it back —
///   so every undo runs [`enums_moved_by_restore`] before the restore and
///   [`rebuild_indexes_over_enums`] after it, exactly as `pglet::transaction`'s two
///   rollbacks do. Without the pair a `DROP TYPE` + re-`CREATE TYPE` reorder inside a
///   caught body left the catalog restored and the specs on the block's order, and a
///   unique partial index over `WHERE m > 'ok'` took a duplicate with no error
///   anywhere. What it does **not** close is a resync that raises *part-way* — the
///   catalog is already restored, so the rebuild loop's unwind has nothing to put
///   back. That **shape** is `EDGES.md` row 110's and is every backward caller's,
///   this one included, since none of them stages. Its **reach** is measured at the
///   two `pglet::transaction` callers and not here: row 110's script needs a second
///   connection to commit a row the restored order re-admits into an index whose key
///   cannot be derived for it, and inside one statement there is no such window — a
///   `DoBlock` is never read-only, so the native server runs it under `lock_both()`
///   held across `execute_with_storage`, the embed and wasm hosts run a statement
///   behind `&mut self` so a second logical connection cannot interleave one, and
///   this block's own body rows are gone by then (the storage rollback runs before
///   the restore). The attempt that comes closest does not reach it either, and fails
///   one step earlier: with a `float8`-keyed partial index whose predicate a reorder
///   inside the body newly satisfies, the body's own *forward* rebuild raises first
///   and stages, so the undo restores a catalog that never moved and
///   [`enums_moved_by_restore`] finds nothing to resync — measured on the caught
///   form, and the unique index that follows still refuses its duplicate.
fn execute_do_block<S: Storage>(
    body: &[Plan],
    exception: Option<&DoExceptionHandler>,
    schema: &mut Schema,
    storage: &mut S,
    env: EvalEnv<'_>,
    session: &mut SequenceSession,
) -> Result<QueryResult> {
    storage.savepoint()?;
    let snapshot = schema.clone();
    let mut swaps: Vec<(String, i64)> = Vec::new();
    let mut outcome = run_do_statements(body, schema, storage, env, session, &mut swaps);

    if let (Err(e), Some(handler)) = (&outcome, exception)
        && (handler.catch_all || handler.sqlstates.iter().any(|s| s == e.sqlstate()))
    {
        // The body's subtransaction aborts: back to the level, which stays open for the
        // handler, whose own writes the statement can still take back. An undo that
        // fails here replaces the swallowed error, and the level is released below.
        outcome = match undo_do_block(schema, storage, snapshot.clone(), &swaps) {
            Ok(()) => {
                // A counter cell the body replaced is back; what the handler replaces
                // is the handler's own swap, read against the restored cell.
                swaps.clear();
                run_do_statements(&handler.handler, schema, storage, env, session, &mut swaps)
            }
            Err(undo) => {
                let _ = storage.release_savepoint();
                return Err(undo);
            }
        };
    }

    match outcome {
        Ok(()) => {
            storage.release_savepoint()?;
            // The cells this `DO` replaced and did not put back are its caller's to
            // restore on a later undo — an enclosing `DO`'s, or the host's block.
            Ok(QueryResult {
                swapped_sequences: swaps,
                ..QueryResult::ddl("DO")
            })
        }
        Err(e) => {
            let undone = undo_do_block(schema, storage, snapshot, &swaps);
            // The level comes off the backend's stack on every path out, so the next
            // session `ROLLBACK TO` restores the right one; a release failure would mask
            // the error the caller needs, and can only mean a bug in this pairing.
            let _ = storage.release_savepoint();
            undone.and(Err(e))
        }
    }
}

/// Run a `DO` body's or handler's statements in order, stopping at the first error.
///
/// `swaps` gathers the counter cells each statement replaces: what `Plan::swapped_sequences`
/// lists for it, read before it runs — the value a replaced cell held is gone afterwards —
/// and, for a nested `DO` that completed, the cells that `DO` reports it replaced
/// (`QueryResult::swapped_sequences`), which no plan lists ahead. Without the second half
/// an inner `DO`'s replacement escaped the outer one's undo.
fn run_do_statements<S: Storage>(
    stmts: &[Plan],
    schema: &mut Schema,
    storage: &mut S,
    env: EvalEnv<'_>,
    session: &mut SequenceSession,
    swaps: &mut Vec<(String, i64)>,
) -> Result<()> {
    for stmt in stmts {
        check_allowed_in_do_block(stmt)?;
        swaps.extend(stmt.swapped_sequences(schema));
        let result = execute_with_storage(stmt, schema, storage, env, session)?;
        swaps.extend(result.swapped_sequences);
    }
    Ok(())
}

/// Put storage and the catalog back to the `DO` block's savepoint, keeping the level.
///
/// The catalog is restored even where the storage rollback fails, so the two never
/// disagree by more than the storage fault itself: the host's own statement rollback, or
/// the enclosing block's, discards the storage side again on the B+tree (memory storage's
/// auto-commit rollback is a no-op, the repo's `EDGES.md` row 40). The enum resync runs over a
/// store the rollback reached, so it is skipped when the rollback failed and that error
/// is returned instead.
fn undo_do_block<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    snapshot: Schema,
    swaps: &[(String, i64)],
) -> Result<()> {
    let rolled_back = storage.rollback_to_savepoint();
    // Read before the restore: it is a difference against the catalog the restore is
    // about to replace.
    let moved = enums_moved_by_restore(schema, &snapshot);
    schema.restore_from(snapshot, swaps);
    rolled_back?;
    rebuild_indexes_over_enums(schema, storage, &moved)
}

/// 18.6's answer to an `ALTER TABLE IF EXISTS` over a name no relation holds — a
/// `NOTICE` naming the relation, and the `ALTER TABLE` tag — the statement's operation
/// never reached. Only a name **no relation** holds skips
/// ([`Plan::alter_table_target`]'s caller): over an index or a sequence the operation
/// runs and meets the wrong-kind refusal it meets without `IF EXISTS` (`ALTER TABLE
/// IF EXISTS <a sequence> ADD COLUMN …` is `42809` on 18.6, measured).
fn alter_table_skipped(table: &str) -> QueryResult {
    QueryResult::ddl_with_notice(
        "ALTER TABLE",
        format!("relation \"{table}\" does not exist, skipping"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{ColumnDef, Expr, Literal};
    use crate::types::TypeOid;

    // Executing a plan moves the schema generation on DDL and not on a read — the stamp
    // the server's prepared-statement plans and the facade's plan cache key on, whose
    // other movers `Schema::generation`'s doc names (a rollback's `Schema::restore_from`,
    // a host replacing the catalog).
    // A held plan of a stale shape reads wrong columns, so this gating is
    // load-bearing for both.
    #[test]
    fn ddl_bumps_schema_generation_reads_do_not() {
        let mut schema = Schema::new();
        assert_eq!(schema.generation, 0);

        // CREATE TABLE (DDL) bumps the generation.
        let create = Plan::CreateTable {
            name: "t".to_string(),
            columns: vec![ColumnDef {
                name: "a".to_string(),
                type_name: "integer".to_string(),
                nullable: true,
                default: None,
                implicit_sequence: false,
                identity: None,
                identity_start: None,
                sequence_name: None,
            }],
            checks: vec![],
            unique_constraints: vec![],
            foreign_keys: vec![],
            if_not_exists: false,
        };
        execute(&create, &mut schema).unwrap();
        assert_eq!(schema.generation, 1);

        // A read (no schema change) must NOT bump it — else the cache would be
        // invalidated on every query.
        let read = Plan::Values {
            rows: vec![vec![Expr::Literal(Literal::Int(1))]],
            certain: crate::planner::CertainTypes::default(),
            enclosing: crate::planner::EnclosingColumns::default(),
        };
        execute(&read, &mut schema).unwrap();
        assert_eq!(schema.generation, 1, "a read must not bump the generation");

        // Another DDL (DROP) bumps again.
        let drop = Plan::DropTable {
            names: vec!["t".to_string()],
            if_exists: false,
            cascade: false,
        };
        execute(&drop, &mut schema).unwrap();
        assert_eq!(schema.generation, 2);
    }

    // `generate_object_oid` advances the counter on every call, so it hands out a
    // fresh number even when the catalog max doesn't move — the shape a `CREATE TABLE`
    // that allocates and is then rolled back leaves. That matters because a **dropped**
    // object's number must never come back: reuse lets two logical indexes share one
    // storage `index_id`, so dropping one orphans the other → "index not found" (the
    // fuz_app pglet-server re-migration failure). A mint site that reads
    // `next_object_oid` instead does hand a *failed* statement's number back, which is a
    // different question and safe for the reason that function's doc states. Regression
    // guard for `generate_object_oid`.
    #[test]
    fn object_oid_allocation_is_monotonic() {
        use super::helpers::generate_object_oid;
        let mut schema = Schema::new();
        // Two allocations against an *unchanged* (empty) catalog — the rolled-back-
        // CREATE shape. The old `max+1` scheme would hand back the same OID both times.
        let a = generate_object_oid(&mut schema);
        let b = generate_object_oid(&mut schema);
        let c = generate_object_oid(&mut schema);
        assert!(
            a < b && b < c,
            "OIDs must advance monotonically even with a flat catalog max ({a}, {b}, {c})",
        );
    }

    // A `WITH RECURSIVE` counts up to the fixed point (the canonical 1..5). The
    // recursive term reads the working table from the `EvalEnv`, so `NoopStorage`
    // suffices — no base table is scanned.
    #[test]
    fn recursive_cte_counts_up() {
        let mut schema = Schema::new();
        let plan = crate::parser::parse(
            "WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM t WHERE n<5) \
             SELECT n FROM t ORDER BY n",
            &schema,
        )
        .unwrap()
        .remove(0);
        let result = execute(&plan, &mut schema).unwrap();
        let ns: Vec<_> = result.rows.iter().map(|r| r[0].clone()).collect();
        assert_eq!(
            ns,
            vec![
                crate::types::Value::Int4(1),
                crate::types::Value::Int4(2),
                crate::types::Value::Int4(3),
                crate::types::Value::Int4(4),
                crate::types::Value::Int4(5),
            ],
        );
        // The result column keeps the CTE's alias-list name `n`.
        assert_eq!(result.columns()[0].name, "n");
    }

    // A self-reference inside a plain FROM-clause derived table is legal (Postgres
    // permits it), and must execute correctly (1..5), not loop or misbehave — the
    // `SubqueryScan` threads the working-table binding down to the inner `CteScan`.
    #[test]
    fn recursive_cte_self_ref_in_derived_table() {
        let mut schema = Schema::new();
        let plan = crate::parser::parse(
            "WITH RECURSIVE t(n) AS (\
               SELECT 1 UNION ALL SELECT n + 1 FROM (SELECT n FROM t) sub WHERE n < 5\
             ) SELECT n FROM t ORDER BY n",
            &schema,
        )
        .unwrap()
        .remove(0);
        let result = execute(&plan, &mut schema).unwrap();
        let ns: Vec<_> = result.rows.iter().map(|r| r[0].clone()).collect();
        assert_eq!(
            ns,
            (1..=5).map(crate::types::Value::Int4).collect::<Vec<_>>()
        );
    }

    // A `UNION ALL` recursion with no stop condition is unbounded; rather than loop
    // forever, pglet trips the iteration cap and fails loud (`54000`). Postgres has no
    // cap and would hang, so this can't be a differential fixture — it lives here.
    #[test]
    fn recursive_cte_runaway_hits_iteration_cap() {
        let mut schema = Schema::new();
        let plan = crate::parser::parse(
            "WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM t) SELECT n FROM t",
            &schema,
        )
        .unwrap()
        .remove(0);
        let err = execute(&plan, &mut schema).unwrap_err();
        assert_eq!(err.sqlstate(), "54000");
    }

    #[test]
    fn test_create_table() {
        let mut schema = Schema::new();
        let plan = Plan::CreateTable {
            name: "users".to_string(),
            columns: vec![
                ColumnDef {
                    name: "id".to_string(),
                    type_name: "integer".to_string(),
                    nullable: false,
                    default: None,
                    implicit_sequence: false,
                    identity: None,
                    identity_start: None,
                    sequence_name: None,
                },
                ColumnDef {
                    name: "name".to_string(),
                    type_name: "text".to_string(),
                    nullable: true,
                    default: None,
                    implicit_sequence: false,
                    identity: None,
                    identity_start: None,
                    sequence_name: None,
                },
            ],
            checks: vec![],
            unique_constraints: vec![],
            foreign_keys: vec![],
            if_not_exists: false,
        };

        let result = execute(&plan, &mut schema).unwrap();
        assert_eq!(result.command_tag, "CREATE TABLE");
        assert!(schema.get_table("users").is_some());

        let table = schema.get_table("users").unwrap();
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].name, "id");
        assert_eq!(table.columns[0].type_oid, TypeOid::Int4);
    }

    #[test]
    fn test_create_table_if_not_exists() {
        let mut schema = Schema::new();
        let plan = Plan::CreateTable {
            name: "users".to_string(),
            columns: vec![],
            checks: vec![],
            unique_constraints: vec![],
            foreign_keys: vec![],
            if_not_exists: false,
        };

        execute(&plan, &mut schema).unwrap();

        // Try to create again without IF NOT EXISTS - should fail
        let result = execute(&plan, &mut schema);
        assert!(result.is_err());

        // Try with IF NOT EXISTS - should succeed
        let plan_if_not_exists = Plan::CreateTable {
            name: "users".to_string(),
            columns: vec![],
            checks: vec![],
            unique_constraints: vec![],
            foreign_keys: vec![],
            if_not_exists: true,
        };
        let result = execute(&plan_if_not_exists, &mut schema);
        assert!(result.is_ok());
    }

    #[test]
    fn test_type_name_to_oid() {
        assert_eq!(type_name_to_oid("integer").unwrap(), TypeOid::Int4);
        assert_eq!(type_name_to_oid("serial").unwrap(), TypeOid::Int4);
        assert_eq!(type_name_to_oid("text").unwrap(), TypeOid::Text);
        assert_eq!(type_name_to_oid("varchar(255)").unwrap(), TypeOid::Varchar);
        assert_eq!(type_name_to_oid("timestamp").unwrap(), TypeOid::Timestamp);
        assert_eq!(type_name_to_oid("date").unwrap(), TypeOid::Date);
        assert_eq!(type_name_to_oid("time").unwrap(), TypeOid::Time);
        assert_eq!(type_name_to_oid("boolean").unwrap(), TypeOid::Bool);
    }

    // An unsupported type fails loud by default (no silent TEXT coercion). `point`
    // is still unimplemented; `time`/`numeric` graduated to real column types.
    #[test]
    fn test_type_name_to_oid_unsupported_fails_loud() {
        assert!(matches!(
            type_name_to_oid("point"),
            Err(crate::Error::UnsupportedType { .. })
        ));
        assert!(matches!(
            type_name_to_oid("foobar"),
            Err(crate::Error::UnsupportedType { .. })
        ));
    }

    // With the coerce flag set, an unsupported type degrades to TEXT (legacy
    // behavior). Reset the thread-local afterward so the toggle doesn't leak into
    // sibling tests sharing this thread.
    #[test]
    fn test_type_name_to_oid_coerce_flag_restores_text() {
        crate::planner::set_coerce_unknown_types(true);
        let point = type_name_to_oid("point");
        let foobar = type_name_to_oid("foobar");
        crate::planner::set_coerce_unknown_types(false);
        assert_eq!(point.unwrap(), TypeOid::Text);
        assert_eq!(foobar.unwrap(), TypeOid::Text);
    }
}
