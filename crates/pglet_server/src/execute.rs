//! Query execution shared between the simple and extended query protocols.
//!
//! `handlers::handle_message` routes Query / Execute here; the protocol router
//! and the prepared-statement lifecycle stay in `handlers`.
//!
//! # The aborted-block gate
//!
//! Which statements an aborted block still admits is decided by the facade's
//! classifier (`pglet::classify_transaction_exit`, reached through
//! `pglet::TransactionSession::aborted_block_action`), because four phases here and
//! the embed/wasm leg all have to admit the same set. It reads the **lexer** rather
//! than the raw text: a `starts_with` over the trimmed, ASCII-uppercased SQL answered
//! `25P02` to `/* c */ COMMIT` and `-- c⏎ROLLBACK` **and wedged the session** — every
//! statement after them, `BEGIN` included, was `25P02` until a bare exit arrived,
//! where 18.6 ends the block on all eight arrangements. That classifier's own doc
//! carries the admitted set, the divergences from `IsTransactionExitStmt`, and why
//! widening a string match is not the fix.
//!
//! What it decides is only which statements the `25P02` gate admits. Which statements
//! *open and end* a block is a different question, answered after execution by the
//! executor's `QueryResult::tx_command` signal
//! (`pglet::TransactionSession::apply_command`) — and by the signal rather than the
//! command tag, because `ROLLBACK TO SAVEPOINT` reports tag `ROLLBACK` and a
//! tag-keyed host reads it as a whole-block rollback.

use std::borrow::Cow;
use std::sync::Arc;

use pglet::AbortedBlockAction;
use pglet_sql::catalog::{Schema, execute_session_command, startup_parameters};
use pglet_sql::executor::{EvalEnv, QueryResult, TransactionCommand, execute_with_storage};
use pglet_sql::planner::Plan;
use pglet_sql::types::{TypeOid, Value};
use pglet_wire::protocol::FormatCode;
use pglet_wire::{BackendMessage, TransactionStatus};

use crate::connection::{Connection, SnapshotAdapter, StorageAdapter};
use crate::encoding::{
    error_response, extended_error, query_result_to_extended, query_result_to_messages,
    query_result_to_messages_no_ready,
};

/// Wire protocol variant (affects error formatting and `ReadyForQuery`).
#[derive(Clone)]
pub enum Protocol {
    /// Simple query protocol - includes `ReadyForQuery` after results
    Simple,
    /// Extended query protocol - no `ReadyForQuery` (comes with Sync)
    /// Carries `result_formats` from Bind.
    Extended { result_formats: Vec<FormatCode> },
}

impl Protocol {
    /// Whether this is the extended protocol (no trailing `ReadyForQuery`).
    const fn is_extended(&self) -> bool {
        matches!(self, Self::Extended { .. })
    }

    /// The result format codes (Extended only; empty for Simple = all text).
    // not const: `Vec`→slice deref coercion isn't allowed in a const fn
    #[allow(clippy::missing_const_for_fn)]
    fn result_formats(&self) -> &[FormatCode] {
        match self {
            Self::Simple => &[],
            Self::Extended { result_formats } => result_formats,
        }
    }
}

/// Format a `pglet_sql::Error` for `protocol`, failing the transaction first.
///
/// The simple-protocol `ReadyForQuery` is built with the post-failure
/// `conn.transaction_status()` directly (no after-the-fact patching); the extended
/// protocol emits no `ReadyForQuery` (it comes with Sync).
fn format_error(
    conn: &mut Connection,
    e: &pglet_sql::Error,
    protocol: &Protocol,
) -> Vec<BackendMessage> {
    conn.fail_transaction();
    if protocol.is_extended() {
        extended_error(e)
    } else {
        error_response(e, conn.transaction_status())
    }
}

/// Format a successful `QueryResult` for `protocol`.
///
/// The simple-protocol `ReadyForQuery` carries `conn.transaction_status()`
/// directly; the extended protocol emits no `ReadyForQuery`.
fn format_result(
    conn: &Connection,
    result: &QueryResult,
    tag: &str,
    protocol: &Protocol,
) -> Vec<BackendMessage> {
    if protocol.is_extended() {
        query_result_to_extended(result, tag, protocol.result_formats())
    } else {
        query_result_to_messages(result, tag, conn.transaction_status())
    }
}

/// The `ParameterStatus` frames a session command owes, given the reported values
/// as they stood **before** it ran.
///
/// Postgres's `GUC_REPORT` contract in one place: at connect every reported
/// parameter is sent, and after that one is sent exactly when its value changed.
/// Reading it as a diff over `catalog::startup_parameters` rather than as something
/// each statement announces is what keeps the two sides from drifting — the startup
/// block and this comparison render the same table, so a parameter added to it is
/// reported in both places or in neither.
///
/// This is **server-only by nature**: `ParameterStatus` is a wire frame, and the
/// embed facade and the wasm bindings over it have no wire. They carry the same
/// `SessionSettings`, so `SHOW` answers the same value there; what they cannot do is
/// tell a client the value moved.
fn changed_parameter_status(conn: &Connection, before: &[String]) -> Vec<BackendMessage> {
    startup_parameters(&conn.settings)
        .into_iter()
        .zip(before)
        .filter(|((_, after), was)| after.as_ref() != was.as_str())
        .map(|((name, after), _)| BackendMessage::ParameterStatus {
            name: name.into(),
            value: after.into(),
        })
        .collect()
}

/// Format one statement's successful result for `protocol` **without** a trailing
/// `ReadyForQuery`.
///
/// The multi-statement simple-`Query` path accumulates each statement's result set
/// and appends exactly one `ReadyForQuery` for the whole message (Postgres emits
/// every statement's result set, then a single `ReadyForQuery`). The extended path
/// never carries a `ReadyForQuery` here regardless (it comes with `Sync`).
fn format_result_no_ready(
    result: &QueryResult,
    tag: &str,
    protocol: &Protocol,
) -> Vec<BackendMessage> {
    if protocol.is_extended() {
        query_result_to_extended(result, tag, protocol.result_formats())
    } else {
        query_result_to_messages_no_ready(result, tag)
    }
}

/// An empty-query response for `protocol` (simple appends `ReadyForQuery`).
fn empty_response(conn: &Connection, protocol: &Protocol) -> Vec<BackendMessage> {
    if protocol.is_extended() {
        vec![BackendMessage::EmptyQueryResponse]
    } else {
        vec![
            BackendMessage::EmptyQueryResponse,
            BackendMessage::ReadyForQuery {
                status: conn.transaction_status(),
            },
        ]
    }
}

/// Apply a successful statement's transaction-lifecycle signal to `conn`'s block.
///
/// Reads `QueryResult::tx_command` and **not** the command tag: `ROLLBACK TO
/// SAVEPOINT` reports tag `ROLLBACK`, so a tag-keyed dispatch discards every write
/// since `BEGIN` and reports success. A statement with no signal is a no-op here.
///
/// # Errors
///
/// Returns the commit error if `COMMIT` fails to persist (e.g. the schema can't
/// be serialized) — the caller surfaces it to the client instead of reporting a
/// successful commit that silently dropped (or emptied) the catalog. Likewise
/// surfaces a `ROLLBACK` whose storage restore or save failed.
fn apply_transaction_command(
    conn: &mut Connection,
    result: &QueryResult,
) -> Result<(), pglet_sql::Error> {
    let Some(command) = &result.tx_command else {
        return Ok(());
    };
    conn.apply_transaction_command(command)
}

/// Handle a SQL query using simple query protocol (no bound parameters).
///
/// `COPY … TO STDOUT` / `… FROM STDIN` drive an out-of-band byte sub-protocol that
/// can't ride the `QueryResult` channel, so they are detected and handled before
/// the normal execution path (see `crate::copy`). A non-COPY statement returns
/// `None` from `try_copy_simple` and runs through `execute_query_core` as usual.
pub fn handle_query(conn: &mut Connection, sql: &str) -> Vec<BackendMessage> {
    if let Some(messages) = crate::copy::try_copy_simple(conn, sql) {
        return messages;
    }
    execute_query_core(conn, sql, &Protocol::Simple, &[], None)
}

/// Execute one resolved statement: capture the MVCC snapshot, choose the lock-free
/// read path vs the locked write path, take **the writer** for a write, and persist an
/// auto-commit write. The plan is produced by `resolve` **under the schema lock**, and
/// produced again under the lock the statement runs with when the catalog generation
/// moved in between — a write that waited for the writer re-plans against what the
/// holder committed or took back. Returns the statement's result, or the
/// already-formatted error messages to send back.
///
/// This is the shared per-statement core of both the simple-query parse loop
/// (`resolve` plans the parsed statement) and the extended-protocol cached fast
/// path (`resolve` reuses the cached plan when its generation still matches).
///
/// # The writer
///
/// A statement that takes it ([`Plan::takes_writer`]) and whose connection does not
/// hold it acquires it **before anything stages**, with no schema or storage guard and
/// no snapshot held while it waits ([`Connection::acquire_writer`]); a block that takes
/// it re-bases every frame on the state committed at that moment
/// (`pglet::TransactionSession::take_writer`). An auto-commit statement releases it at
/// its end, a block at its own ([`Connection::settle_writer`]).
fn run_one<'c, F>(
    conn: &mut Connection,
    protocol: &Protocol,
    env: EvalEnv<'_>,
    resolve: F,
) -> Result<QueryResult, Vec<BackendMessage>>
where
    F: Fn(&Schema) -> pglet_sql::Result<Cow<'c, Plan>>,
{
    let outcome = run_one_locked(conn, env, &resolve);
    conn.settle_writer();
    outcome.map_err(|e| format_error(conn, &e, protocol))
}

/// [`run_one`]'s body, answering the statement's error unformatted.
///
/// # A plan that fails on another connection's uncommitted catalog
///
/// The catalog is one object every connection reads in place, and a block's DDL is
/// applied to it at once — so a statement planned while another connection's block
/// holds the writer is planned against that block's uncommitted catalog. A plan that
/// **fails** there because of the block's change (a column the block dropped — the plan
/// succeeds against the catalog committed when the block took the writer) is not the
/// statement's answer: the statement waits for that hold to end and is planned again,
/// answering whatever that plan answers, bounded by the one `lock_timeout` deadline the
/// statement's waits share ([`Connection::plan_failure_is_the_writers`]). A failure of
/// the statement's own — one the committed catalog answers the same way — answers at
/// once. The wait covers **planning** only: a plan that succeeds against the block's
/// catalog runs against it, and an error raised while it runs (a table the block
/// dropped, looked up at execution) is answered from it (the repo's `EDGES.md` row 221).
fn run_one_locked<'c, F>(
    conn: &mut Connection,
    env: EvalEnv<'_>,
    resolve: &F,
) -> pglet_sql::Result<QueryResult>
where
    F: Fn(&Schema) -> pglet_sql::Result<Cow<'c, Plan>>,
{
    let conn_id = conn.session.process_id;
    let in_block = conn.transaction_status() != TransactionStatus::Idle;
    let deadline = conn.writer_deadline();
    // What the store has committed as this statement starts, for a `REPEATABLE READ`
    // block that has taken no snapshot yet: if this statement is its first write, its
    // snapshot is this moment's (`TransactionSession::note_snapshot`), read before any
    // wait below.
    let first_write_at = (in_block
        && !conn.tx.snapshot_taken(conn_id)
        && conn.settings.block_uses_transaction_snapshot())
    .then(|| conn.shared.lock_storage().commit_stamp());

    // Plan. A read on the lock-free path runs here and returns; anything else carries
    // its plan and the generation it was made against to the locked path below.
    let (planned, planned_at) = loop {
        // Only autocommit (Idle) + B+tree can use lock-free snapshots.
        let snapshot = if in_block {
            None
        } else {
            conn.shared.read_snapshot()
        };
        let planned = {
            let schema = conn.shared.lock_schema();
            // The statement's entry check (`Connection::catalog_depth`) comes before the
            // schema clone below, which walks the whole catalog.
            conn.catalog_depth.check(&schema)?;
            match resolve(&schema) {
                // Resolve the plan and snapshot the schema under one lock acquisition, so
                // the generation check (inside `resolve`) and the schema the plan runs
                // against are the same revision.
                Ok(planned) => {
                    let schema_clone =
                        (snapshot.is_some() && planned.is_read_only()).then(|| schema.clone());
                    Ok((planned, schema.generation, schema_clone))
                }
                Err(e) if conn.plan_failure_is_the_writers(&e, schema.generation, resolve) => {
                    Err(None)
                }
                Err(e) => Err(Some(e)),
            }
        };
        match (planned, snapshot) {
            (Ok((planned, _, Some(mut schema))), Some(snapshot)) => {
                // MVCC path: execute against the snapshot with no locks held. A
                // read-only plan never carries a top-level sequence function (those are
                // classified non-read-only precisely so they don't run against this
                // throwaway schema clone), so a scratch session that never records
                // anything is used.
                let mut storage_adapter = SnapshotAdapter::new(&snapshot);
                let mut scratch_session = pglet_sql::executor::SequenceSession::new();
                return execute_with_storage(
                    &planned,
                    &mut schema,
                    &mut storage_adapter,
                    env,
                    &mut scratch_session,
                );
            }
            (Ok((planned, generation, _)), _) => break (planned, generation),
            // Another connection's open hold of the writer changed the catalog this plan
            // failed against: wait, holding nothing, and plan again.
            (Err(None), _) => conn.wait_for_foreign_writer(deadline)?,
            (Err(Some(e)), _) => return Err(e),
        }
    };

    // The `25001` gate — `CREATE`/`DROP INDEX CONCURRENTLY` inside a transaction block,
    // from the one rule (`Plan::forbidden_in_transaction_block`), ahead of the writer: a
    // statement refused here stages nothing.
    if in_block && let Some(op) = planned.forbidden_in_transaction_block() {
        return Err(pglet_sql::Error::ActiveSqlTransaction { op });
    }
    // The writer, before anything stages — waited for with no guard and no snapshot
    // held. A statement already on a block that holds it takes nothing.
    let takes_writer = planned.takes_writer();
    // A statement the block runs takes its snapshot, the first fixing its isolation level.
    if in_block && !planned.is_transaction_control() {
        conn.tx.note_snapshot(
            conn_id,
            &mut conn.settings,
            first_write_at.filter(|_| takes_writer),
        );
    }
    if takes_writer && !conn.holds_writer() {
        conn.acquire_writer(deadline)?;
    }

    let (mut schema, mut storage) = conn.shared.lock_both();
    if takes_writer && in_block && !conn.tx.holds_writer() {
        // Nothing is staged yet: every frame the block holds is re-based on what is
        // committed now, which the writer keeps from moving under it — or, for a
        // `REPEATABLE READ` / `SERIALIZABLE` block something committed past, the write is
        // refused `40001` and the block, having staged nothing, gives the writer back.
        // Keyed on the session's own record, so the lock and the session's "holds the
        // writer" are one fact at every exit from here on.
        let uses_snapshot = conn.settings.block_uses_transaction_snapshot();
        if let Err(e) = conn
            .tx
            .take_writer(conn_id, uses_snapshot, &schema, &mut storage)
        {
            drop((schema, storage));
            conn.shared.writer.release(conn_id);
            return Err(e);
        }
    }
    if takes_writer {
        conn.note_writer_grant(&schema);
    }
    conn.catalog_depth.check(&schema)?;
    // A catalog that moved since the plan was made — while the statement waited, or
    // between the two guards — is planned against again.
    let replanned;
    let planned: &Plan = if schema.generation == planned_at {
        &planned
    } else {
        replanned = resolve(&schema)?;
        &replanned
    };

    // The sequence counter cells this statement replaces, with the value each held
    // before it — what a rollback puts back, and gone after the statement. Read before
    // it runs; outside this connection's block the session ignores it.
    let sequence_swaps = if in_block {
        planned.swapped_sequences(&schema)
    } else {
        Vec::new()
    };

    // `mutates_storage()` is exactly the DML/COPY/DDL set; it excludes
    // BEGIN/COMMIT/ROLLBACK (transaction control touches no page store), so only a
    // storage-mutating auto-commit statement is wrapped + persisted.
    let result = if !in_block && planned.mutates_storage() {
        // Whether a write the device refuses at the commit can drop the staged
        // writes and leave the store serving — asked before the statement runs.
        let discardable = pglet::autocommit_is_discardable(planned, &schema);
        // Wrap the auto-commit write in an implicit storage transaction so a
        // multi-row statement (INSERT/UPDATE/COPY) that violates a constraint
        // partway through rolls back its earlier rows atomically — Postgres's
        // per-statement atomicity — instead of leaving them applied. With a
        // transaction active for this conn_id, `StorageAdapter` routes every
        // insert/update to the pending buffer, so the rows stage and publish
        // in one CoW root swap at commit, exactly like an explicit BEGIN.
        // Read-your-own-writes within the statement (ON CONFLICT probes, an
        // intra-statement duplicate, `INSERT … SELECT`) stays correct because
        // the pending buffer is merged by the `*_in_tx` read paths. This is a
        // pure storage-layer bracket: it does NOT touch the connection's
        // `TransactionSession` (which tracks the *explicit* connection-level
        // transaction, orthogonal to this). A no-op bracket on memory storage.
        storage.begin(conn_id)?;
        let result = {
            let mut storage_adapter = StorageAdapter::new(&mut storage, conn_id);
            execute_with_storage(
                planned,
                &mut schema,
                &mut storage_adapter,
                env,
                &mut conn.sequence_session,
            )
        };
        match result {
            // Encode the catalog, then publish the staged writes atomically (one CoW
            // root swap), drop the schema lock, and flip the durable meta slot. Never
            // report success if the commit or the flip didn't reach disk.
            Ok(r) => Connection::publish_autocommit(conn_id, schema, &mut storage, discardable)
                .map(|()| r),
            // Discard the staged writes; surface the ORIGINAL error (a rollback
            // failure must not mask it).
            Err(e) => {
                let _ = storage.rollback(conn_id);
                Err(e)
            }
        }
    } else {
        // A statement inside a block, transaction control, or a read that cannot take
        // the lock-free path (memory storage, a `currval` projection): run directly.
        let mut storage_adapter = StorageAdapter::new(&mut storage, conn_id);
        execute_with_storage(
            planned,
            &mut schema,
            &mut storage_adapter,
            env,
            &mut conn.sequence_session,
        )
    };

    // Hand this statement's replaced counter cells to the session — the ones read
    // ahead, and the ones a completed `DO` reports in its result — which accumulates
    // them into the open frame and ignores them when no block is open on this
    // connection.
    conn.tx.record_statement(conn_id, sequence_swaps, &result);
    result
}

/// What the aborted-block gate decided about one statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbortedGate {
    /// The block is healthy or absent, or this statement is one it still admits
    /// (`ROLLBACK` and its spellings, which end the block through their command tag).
    Run,
    /// `COMMIT` / `END` was converted to a rollback: the block has ended and the reply
    /// is `CommandComplete { tag: "ROLLBACK" }`. Each caller appends its own
    /// `ReadyForQuery` (or not) per its protocol's convention.
    Converted,
    /// `25P02`.
    Refused,
}

/// Ask the session what to do with `sql`, performing the rollback when it converts.
///
/// `COMMIT` / `END` in an aborted transaction is an implicit `ROLLBACK` in Postgres:
/// no error, the transaction ends, and the reported command tag is `ROLLBACK` (not
/// `COMMIT`). Connection poolers / ORMs that unconditionally `COMMIT` at block end
/// rely on this to recover the session, so match it rather than wedging the
/// transaction open on `25P02`.
///
/// The conversion runs as the session's commit of an aborted block — that rollback,
/// answered as the commit the client asked for (`TransactionSession::commit`) — so a
/// refusal that fails the store past a save of the block's DDL says its outcome is
/// unknown, as a healthy block's refused `COMMIT` does.
///
/// # Errors
///
/// Returns a loud rollback refusal (a concurrent connection committed DDL), the refused
/// save of the restored catalog or of the counters, or an error building what that save
/// writes. The block is ended either way. After a loud rollback refusal, or an error
/// building the save, the connection stays usable and the error is informational; after a
/// refused save the store has failed, and the connection's next message ends it
/// (`57P02`).
fn gate_aborted_block(conn: &mut Connection, sql: &str) -> Result<AbortedGate, pglet_sql::Error> {
    match conn.tx.aborted_block_action(conn.session.process_id, sql) {
        AbortedBlockAction::Run => Ok(AbortedGate::Run),
        AbortedBlockAction::Refuse => Ok(AbortedGate::Refused),
        AbortedBlockAction::ImplicitRollback => {
            conn.apply_transaction_command(&TransactionCommand::Commit)?;
            Ok(AbortedGate::Converted)
        }
    }
}

/// An extended-protocol portal's cached plan, the schema generation it was built
/// against, and the parameter types its `Parse` declared — see [`execute_query_core`].
pub type CachedPlan<'a> = (&'a Arc<Plan>, u64, &'a [Option<TypeOid>]);

/// Core query execution logic shared between simple and extended protocols.
///
/// `params` carries the extended-protocol portal's bound values (empty for the
/// simple-query path); they are threaded into execution as an [`EvalEnv`] so each
/// `$N` resolves at eval time — the plan is never mutated by binding.
///
/// `cached` is the extended-protocol portal's cached `(plan, generation, declared
/// parameter types)`: when present and still current it runs that one immutable plan
/// directly (no re-parse, no re-plan); a generation mismatch (a DDL changed the
/// schema) falls back to re-planning the SQL with the parameter types the client
/// declared, as its Parse planned it. `None` for the simple-query path.
pub fn execute_query_core(
    conn: &mut Connection,
    sql: &str,
    protocol: &Protocol,
    params: &[Option<Value>],
    cached: Option<CachedPlan<'_>>,
) -> Vec<BackendMessage> {
    // Install site 2 for the ambient session the expression evaluator reads: the one
    // that covers the native server's **extended** protocol, whose cached portals
    // return before the statement loop and so never reach
    // `catalog::execute_session_command`. The enumeration of sites lives in one place,
    // `SessionSettings::install_ambient`'s doc.
    conn.settings.install_ambient();
    // And where a statement starts for the statement clock: a simple-query message, or
    // an `Execute` (`pglet_sql::builtins::start_statement`).
    pglet_sql::builtins::start_statement();
    let env = EvalEnv::with_params(params);
    // The extended protocol's aborted-block gate: **one** test over the whole portal,
    // because that is what `exec_execute_message` does
    // (`!IsTransactionExitStmtList(portal->stmts)`) and a portal is one statement
    // anyway. The simple-query path gates *inside* the loop below instead, per
    // statement, which is `exec_simple_query`'s shape — see `gate_aborted_block`.
    if protocol.is_extended() {
        match gate_aborted_block(conn, sql) {
            Ok(AbortedGate::Run) => {}
            Ok(AbortedGate::Converted) => {
                return vec![BackendMessage::CommandComplete {
                    tag: "ROLLBACK".into(),
                }];
            }
            Ok(AbortedGate::Refused) => {
                return extended_error(&pglet_sql::Error::AbortedTransaction);
            }
            Err(e) => return format_error(conn, &e, protocol),
        }
    }

    // Extended-protocol fast path: reuse the portal's one cached plan (no re-parse,
    // no re-plan) when its schema generation is still current; re-plan the SQL only
    // if a DDL, or a rollback that took one back, has moved the generation since
    // Parse. The generation check happens inside `run_one`, under the schema lock,
    // so it is atomic with execution.
    if let Some((arc, plan_gen, declared_param_types)) = cached {
        let outcome = run_one(conn, protocol, env, |schema| {
            if plan_gen == schema.generation {
                Ok(Cow::Borrowed(arc.as_ref()))
            } else {
                // Schema changed since Parse — re-plan from the SQL.
                pglet_sql::parser::parse_single_with_param_types(sql, schema, declared_param_types)
                    .map(|p| Cow::Owned(p.unwrap_or(Plan::Empty)))
            }
        });
        return match outcome {
            Ok(result) => match apply_transaction_command(conn, &result) {
                Ok(()) => format_result(conn, &result, &result.command_tag, protocol),
                Err(e) => format_error(conn, &e, protocol),
            },
            Err(msgs) => msgs,
        };
    }

    // Parse + plan each statement fresh under the schema lock (so an earlier DDL is
    // visible to a later statement). `split_statements` slices the SQL at top-level
    // `;` (lexer-aware), then each substring is parsed+planned via `parse_single`.
    //
    // An extended-protocol portal is one statement — its `Parse` counted them as 18.6
    // does (`raw_statements`) — so it is split the same way here: a trailing comment
    // (`SET x TO 1; -- c`) is no second piece to run, where the simple-query loop still
    // runs one (`EDGES.md` row 18).
    let split = if protocol.is_extended() {
        pglet_sql::parser::raw_statements(sql)
    } else {
        pglet_sql::parser::split_statements(sql)
    };
    let statements = match split {
        Ok(stmts) => stmts,
        Err(e) => return format_error(conn, &e, protocol),
    };
    if statements.is_empty() {
        return empty_response(conn, protocol);
    }
    // A simple-`Query` message can carry multiple statements. Postgres executes each
    // and sends **every** statement's full result set (its own RowDescription +
    // DataRows + CommandComplete), then a **single** trailing ReadyForQuery — so
    // accumulate each statement's messages here (without a per-statement
    // ReadyForQuery) rather than returning only the last, which silently drops the
    // earlier result sets. The single-statement case is byte-identical: one result
    // set plus the one appended ReadyForQuery.
    //
    // Per-statement autocommit is unchanged — each `run_one` commits independently
    // (the documented divergence from Postgres's implicit single-transaction batch);
    // on a mid-batch error the earlier statements' result messages are still emitted,
    // then the error, then the one ReadyForQuery (the error path already carries it
    // for the simple protocol).
    let mut messages = Vec::new();
    for stmt_sql in &statements {
        // The aborted-block gate, **per statement**, because that is where
        // `exec_simple_query` puts it: inside `foreach(parsetree_list)`, so each parse
        // tree is tested on its own. Gating the whole `Query` text once instead cost
        // two answers, measured against 18.6 with a raw v3 client. `COMMIT; SELECT 1`
        // converts the `COMMIT` and then **runs the `SELECT`** there, where the
        // pre-loop gate converted the whole message and silently dropped every
        // statement after the exit — the same lost-statement class as the
        // leading-`SET` batch swallow this loop already fixes. And an unlexable
        // message (`SELECT 'abc`) is `42601` on both now, because `split_statements`
        // raises before the gate is reached, matching Postgres's raw grammar parse
        // running ahead of the abort test; a *parse-analysis* error still answers
        // `25P02` here against 18.6's `42601`, the same SQLSTATE-shaped approximation
        // of the stage split the `Parse` arm documents.
        //
        // One difference from `exec_simple_query` survives and is **not** this gate's
        // to close: its loop is over parse *trees*, so a piece holding no statement
        // (`/* x */`) never reaches the test and answers `EmptyQueryResponse`, while
        // `split_statements` slices raw text and hands such a piece here, where it is
        // `25P02`. Closing it means not *running* an empty piece — pglet answers a
        // comment-only simple query `CommandComplete "ALTER TABLE"` outside any
        // transaction (`Plan::Empty`'s placeholder tag, measured on both engines), so
        // skipping the gate alone would trade one divergence for another. Recorded in
        // `EDGES.md`.
        //
        // The extended protocol keeps its single pre-loop test above.
        match gate_aborted_block(conn, stmt_sql) {
            Ok(AbortedGate::Run) => {}
            Ok(AbortedGate::Converted) => {
                // The block ended; the trailing `ReadyForQuery` for the whole message
                // is appended once below, carrying the now-Idle status.
                messages.push(BackendMessage::CommandComplete {
                    tag: "ROLLBACK".into(),
                });
                continue;
            }
            Ok(AbortedGate::Refused) => {
                messages.extend(extended_error(&pglet_sql::Error::AbortedTransaction));
                // This arm returns rather than continuing, so it carries the message's
                // one trailing `ReadyForQuery` itself — under the simple protocol's
                // convention only. The extended protocol reaches this loop just for a
                // portal with no cached plan (`SET`/`SHOW`/empty), and its own gate ran
                // before the loop, so `Refused` here is unreachable for it; the
                // condition keeps the reply shape right by construction rather than by
                // that argument.
                if !protocol.is_extended() {
                    messages.push(BackendMessage::ReadyForQuery {
                        status: conn.transaction_status(),
                    });
                }
                return messages;
            }
            Err(e) => {
                // A loud rollback refusal, or its refused save; the session's commit has
                // already ended the block. After a refused save the store has failed and
                // the next message ends the connection.
                messages.extend(format_error(conn, &e, protocol));
                return messages;
            }
        }

        // The session-hygiene statements are serviced from
        // `catalog::execute_session_command`, not planned — the native parser
        // rejects every one of them — and the interception is **per statement**,
        // after `split_statements`. It used to prefix-test the whole `Query` text
        // ahead of the split, so a batch that *began* with a `SET` was answered as
        // one `SET` and every statement after it was never parsed or executed, with
        // no error: `SET TIME ZONE 'UTC'; INSERT INTO t VALUES (1)` — the standard
        // session-setup batch a driver sends — lost the `INSERT`. Each takes the
        // same shape as a planned statement here: its result set without a
        // `ReadyForQuery`, or its typed error (keeping the SQLSTATE — `42704` for an
        // unknown `SHOW` parameter) after the earlier statements' result sets.
        //
        // Three of the five are planner-invisible for a reason the catalog does not
        // cover: `DEALLOCATE` and `DISCARD` reach the namespace and session state
        // that live on the *connection* — `conn.prepared` and
        // `conn.sequence_session`, which nothing below the socket can see. Each
        // carries its tag on the result rather than beside it, since the forms
        // report different ones (`DEALLOCATE` / `DEALLOCATE ALL`, and one per
        // `DISCARD` target).
        let in_block = conn.transaction_status() != TransactionStatus::Idle;
        // Postgres's `GUC_REPORT` rule: a reported parameter is announced at connect
        // and then **only when its value moves** — measured on 18.6 with a raw v3
        // client, a second identical `SET search_path TO public` sends nothing, and so
        // does `SET client_encoding TO 'UTF8'` on a UTF8 connection. So the report is
        // a before/after comparison over the one table rather than something each
        // statement announces: `SET`, `RESET`, `RESET ALL` and the `RESET ALL` inside
        // `DISCARD ALL` all move values, and 18.6 reports every one of them.
        let before: Vec<String> = startup_parameters(&conn.settings)
            .into_iter()
            .map(|(_, value)| value.to_string())
            .collect();
        if let Some(outcome) = execute_session_command(
            stmt_sql,
            &mut conn.prepared,
            &mut conn.sequence_session,
            &mut conn.settings,
            in_block,
        ) {
            match outcome {
                Ok(result) => {
                    messages.extend(format_result_no_ready(
                        &result,
                        &result.command_tag,
                        protocol,
                    ));
                    // After the `CommandComplete` and before the message's trailing
                    // `ReadyForQuery`, which is where 18.6 puts it.
                    messages.extend(changed_parameter_status(conn, &before));
                }
                Err(e) => {
                    messages.extend(format_error(conn, &e, protocol));
                    return messages;
                }
            }
            continue;
        }
        match run_one(conn, protocol, env, |schema| {
            pglet_sql::parser::parse_single(stmt_sql, schema)
                .map(|p| Cow::Owned(p.unwrap_or(Plan::Empty)))
        }) {
            Ok(result) => {
                if let Err(e) = apply_transaction_command(conn, &result) {
                    // The already-accumulated result sets, then this statement's error
                    // (with the trailing ReadyForQuery for the simple protocol).
                    messages.extend(format_error(conn, &e, protocol));
                    return messages;
                }
                messages.extend(format_result_no_ready(
                    &result,
                    &result.command_tag,
                    protocol,
                ));
            }
            // `run_one` already formatted the error (with the simple protocol's
            // trailing ReadyForQuery); emit the earlier result sets before it.
            Err(msgs) => {
                messages.extend(msgs);
                return messages;
            }
        }
    }
    // One ReadyForQuery closes the whole simple-query message; the extended path
    // defers its ReadyForQuery to Sync.
    if !protocol.is_extended() {
        messages.push(BackendMessage::ReadyForQuery {
            status: conn.transaction_status(),
        });
    }
    messages
}

#[cfg(test)]
mod ambient_session_tests {
    //! The **extended**-protocol half of the ambient-session install (site 2 of the
    //! four `SessionSettings::install_ambient`'s doc enumerates).
    //!
    //! The differential fixture corpus cannot see this site: the harness's binary leg
    //! replays each fixture's setup over *simple* queries first, and that path
    //! installs the session through `catalog::execute_session_command` — so removing
    //! the install here leaves every `tests/fixtures/session/*` fixture green while a
    //! client that speaks **only** the extended protocol reads the packet-less
    //! default. Measured against a debug server with this line deleted: `psql -U
    //! alice` issuing `SELECT current_user \bind \g` as its first message answered
    //! `pglet` where the same connection's simple query answered `alice`.

    use std::sync::Arc;

    use pglet_sql::catalog::SessionSettings;
    use pglet_wire::BackendMessage;
    use pglet_wire::protocol::FormatCode;

    use super::{Protocol, execute_query_core};
    use crate::connection::{Connection, SharedState};

    /// The text of the first `DataRow`'s first column.
    fn first_cell(msgs: &[BackendMessage]) -> String {
        msgs.iter()
            .find_map(|m| match m {
                BackendMessage::DataRow { values } => values.first().and_then(|v| {
                    v.as_ref()
                        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                }),
                _ => None,
            })
            .expect("a DataRow with one column")
    }

    /// Run `sql` through the extended protocol on a **cached portal**, which is the
    /// route a real `Parse`/`Bind`/`Execute` takes and the only one that skips the
    /// statement loop — and with it `catalog::execute_session_command`, the *other*
    /// install site. Passing `cached: None` instead makes this test vacuous: it falls
    /// into that loop and installs, which is how the first version of it passed
    /// against a build with the install deleted.
    fn extended_cached(conn: &mut Connection, sql: &str) -> Vec<BackendMessage> {
        let (plan, generation) = {
            let schema = conn.shared.lock_schema();
            let plan = pglet_sql::parser::parse_single(sql, &schema)
                .expect("plan")
                .expect("a statement");
            (Arc::new(plan), schema.generation)
        };
        execute_query_core(
            conn,
            sql,
            &Protocol::Extended {
                result_formats: vec![FormatCode::Text],
            },
            &[],
            Some((&plan, generation, &[])),
        )
    }

    #[test]
    fn an_extended_only_connection_reads_its_own_session() {
        let mut conn = Connection::new(1, Arc::new(SharedState::with_btree()));
        conn.settings =
            SessionSettings::new(String::new()).with_identity("alice".into(), "wonderland".into());
        // No simple query first — this is the whole point: the extended protocol is
        // this connection's *first* statement.
        assert_eq!(
            first_cell(&extended_cached(&mut conn, "SELECT current_user")),
            "alice"
        );
        assert_eq!(
            first_cell(&extended_cached(&mut conn, "SELECT current_database()")),
            "wonderland"
        );
    }
}

#[cfg(test)]
mod aborted_block_gate_tests {
    //! [`execute_query_core`]'s aborted-block gate, driven end to end — a *second*
    //! site from the classifier's own tests (`pglet`'s `classifier_tests`), and the
    //! one that decides the implicit-rollback branch. Driven through
    //! `execute_query_core` over an in-memory CoW-btree engine (no socket), because
    //! the observable is the transaction *ending*.
    //!
    //! The asserted property is deliberately the block's state and command tag, not
    //! the SQLSTATE: `COMMıT` answers `25P02` here and `42601` on 18.6, which is the
    //! pre-parser-gate divergence a plain `SELEKT bogus` shows too (measured), so
    //! pinning the code would pin a divergence as though it were the requirement —
    //! and no `@expect-error` fixture can carry this for the same reason.

    use std::sync::Arc;

    use pglet_wire::{BackendMessage, TransactionStatus};

    use crate::connection::{Connection, SharedState};
    use crate::test_support::{run, sqlstate, tags};

    /// A connection whose block is open and failed.
    fn aborted() -> Connection {
        let mut c = Connection::new(1, Arc::new(SharedState::with_btree()));
        run(&mut c, "BEGIN");
        run(&mut c, "SELECT nosuchthing");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
        c
    }

    #[test]
    fn a_dotless_i_commit_does_not_end_an_aborted_block() {
        let mut c = aborted();

        // U+0131 uppercases to ASCII `I`. Under a Unicode fold this took the
        // implicit-rollback branch: tag `ROLLBACK`, status Idle, and the next
        // statement ran. Nothing folds here now — the lexer reads it as an ordinary
        // identifier — so the answer is reached without any fold of our own.
        let msgs = run(&mut c, "COMMıT");
        assert!(tags(&msgs).is_empty(), "{msgs:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);

        // The accept side: the ASCII spelling still ends the block, as `ROLLBACK`.
        let msgs = run(&mut c, "commit");
        assert_eq!(tags(&msgs), vec!["ROLLBACK".to_string()], "{msgs:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Idle);
    }

    #[test]
    fn a_comment_prefixed_exit_ends_the_block_instead_of_wedging_the_session() {
        // The wedge, at the site that produced it. Under the `starts_with` classifier
        // every one of these answered `25P02` **and left the block failed**, so the
        // `SELECT` after it was `25P02`, the next `BEGIN` was `25P02`, and only a bare
        // exit recovered the session — while 18.6 ends the block on all of them.
        // Red on revert: put the text match back and each `assert_eq!` on the tag
        // fails with an empty tag list.
        for exit in [
            "/* trace */ COMMIT",
            "-- trace\nCOMMIT",
            "/* trace */ ROLLBACK",
            "-- trace\nROLLBACK",
            "/* a /* b */ c */ COMMIT",
        ] {
            let mut c = aborted();
            let msgs = run(&mut c, exit);
            assert_eq!(
                tags(&msgs),
                vec!["ROLLBACK".to_string()],
                "must end the block: {exit:?} -> {msgs:?}"
            );
            assert_eq!(c.transaction_status(), TransactionStatus::Idle);
            // The session is usable, not wedged: an ordinary statement answers and a
            // fresh block opens.
            let after = run(&mut c, "SELECT 1");
            assert!(sqlstate(&after).is_none(), "{exit:?} -> {after:?}");
            assert_eq!(tags(&run(&mut c, "BEGIN")), vec!["BEGIN".to_string()]);
            assert_eq!(c.transaction_status(), TransactionStatus::InTransaction);
        }
    }

    #[test]
    fn an_exit_inside_a_batch_ends_the_block_and_the_rest_of_the_batch_runs() {
        // The gate is per **statement**, as `exec_simple_query`'s is (its test sits inside
        // `foreach(parsetree_list)`). Gating the whole `Query` text once converted the
        // `COMMIT` and then silently dropped every statement after it — measured against
        // 18.6 with a raw v3 client, which answers `CommandComplete(ROLLBACK)`, then the
        // `SELECT`'s `RowDescription` + `DataRow` + `CommandComplete`, then one
        // `ReadyForQuery[I]`. Same lost-statement class as the leading-`SET` batch swallow.
        let mut c = aborted();
        let msgs = run(&mut c, "COMMIT; SELECT 1");
        assert_eq!(
            tags(&msgs),
            vec!["ROLLBACK".to_string(), "SELECT 1".to_string()],
            "the statement after the exit must run: {msgs:?}"
        );
        assert_eq!(c.transaction_status(), TransactionStatus::Idle);
        assert_eq!(
            msgs.iter()
                .filter(|m| matches!(m, BackendMessage::ReadyForQuery { .. }))
                .count(),
            1,
            "exactly one trailing ReadyForQuery for the whole message: {msgs:?}"
        );
        assert!(
            matches!(
                msgs.last(),
                Some(BackendMessage::ReadyForQuery {
                    status: TransactionStatus::Idle
                })
            ),
            "{msgs:?}"
        );

        // The other polarity, so a gate that admitted everything would redden: a
        // non-exit first statement still refuses the whole message, and the one after it
        // does not run (18.6 answers the same `25P02`).
        let mut c = aborted();
        let msgs = run(&mut c, "SELECT 1; COMMIT");
        assert_eq!(sqlstate(&msgs).as_deref(), Some("25P02"), "{msgs:?}");
        assert!(tags(&msgs).is_empty(), "{msgs:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
    }

    #[test]
    fn a_non_exit_statement_is_still_refused_however_it_is_spelled() {
        // The other polarity, so a classifier that admitted everything would redden
        // too: a comment ahead of an ordinary statement buys it nothing, and the block
        // stays failed.
        for sql in ["SELECT 1", "/* trace */ SELECT 1", "-- trace\nBEGIN"] {
            let mut c = aborted();
            let msgs = run(&mut c, sql);
            assert_eq!(sqlstate(&msgs).as_deref(), Some("25P02"), "{sql:?}");
            assert_eq!(c.transaction_status(), TransactionStatus::Failed);
        }
    }
}

#[cfg(test)]
mod multi_statement_result_tests {
    //! A simple-`Query` message carrying multiple statements must emit **every**
    //! statement's full result set (each its own RowDescription + DataRows +
    //! CommandComplete), then exactly ONE trailing ReadyForQuery — matching Postgres.
    //! Before the fix only the *last* statement's result set was returned, silently
    //! dropping the earlier ones (a driver batching statements lost data). Driven
    //! through `execute_query_core` over an in-memory CoW-btree engine (no socket).

    use pglet_wire::{BackendMessage, TransactionStatus};

    use crate::test_support::{conn, run, tags};

    fn count<F: Fn(&BackendMessage) -> bool>(msgs: &[BackendMessage], f: F) -> usize {
        msgs.iter().filter(|m| f(m)).count()
    }

    fn data_rows(msgs: &[BackendMessage]) -> Vec<Vec<Option<Vec<u8>>>> {
        msgs.iter()
            .filter_map(|m| match m {
                BackendMessage::DataRow { values } => Some(values.clone()),
                _ => None,
            })
            .collect()
    }

    fn row_desc_first_names(msgs: &[BackendMessage]) -> Vec<String> {
        msgs.iter()
            .filter_map(|m| match m {
                BackendMessage::RowDescription { columns } => {
                    columns.first().map(|c| c.name.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn two_selects_emit_both_result_sets_in_order_with_one_ready() {
        let mut c = conn();
        let msgs = run(&mut c, "SELECT 1 AS a; SELECT 2 AS b");

        assert_eq!(
            count(&msgs, |m| matches!(
                m,
                BackendMessage::RowDescription { .. }
            )),
            2,
            "each SELECT gets its own RowDescription: {msgs:?}"
        );
        assert_eq!(
            row_desc_first_names(&msgs),
            vec!["a".to_string(), "b".to_string()],
            "result sets appear in statement order: {msgs:?}"
        );

        let rows = data_rows(&msgs);
        assert_eq!(rows.len(), 2, "both result sets' rows: {msgs:?}");
        assert_eq!(rows[0][0].as_deref(), Some(b"1".as_ref()), "{msgs:?}");
        assert_eq!(rows[1][0].as_deref(), Some(b"2".as_ref()), "{msgs:?}");

        assert_eq!(
            count(&msgs, |m| matches!(
                m,
                BackendMessage::CommandComplete { .. }
            )),
            2,
            "one CommandComplete per statement: {msgs:?}"
        );
        assert_eq!(
            count(&msgs, |m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            1,
            "exactly one trailing ReadyForQuery for the whole message: {msgs:?}"
        );
        assert!(
            matches!(
                msgs.last(),
                Some(BackendMessage::ReadyForQuery {
                    status: TransactionStatus::Idle
                })
            ),
            "the single ReadyForQuery is last and carries the final status: {msgs:?}"
        );
    }

    #[test]
    fn mixed_ddl_dml_select_batch_emits_each_tag_and_the_select_rows() {
        let mut c = conn();
        let msgs = run(
            &mut c,
            "CREATE TABLE m (id int); INSERT INTO m VALUES (7); SELECT id FROM m",
        );

        let tags = tags(&msgs);
        assert_eq!(tags.len(), 3, "one CommandComplete per statement: {msgs:?}");
        assert!(tags[0].starts_with("CREATE TABLE"), "{tags:?}");
        assert!(tags[1].starts_with("INSERT"), "{tags:?}");
        assert!(tags[2].starts_with("SELECT"), "{tags:?}");

        let rows = data_rows(&msgs);
        assert_eq!(rows.len(), 1, "the SELECT's single row: {msgs:?}");
        assert_eq!(rows[0][0].as_deref(), Some(b"7".as_ref()), "{msgs:?}");
        assert_eq!(
            count(&msgs, |m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            1,
            "exactly one trailing ReadyForQuery: {msgs:?}"
        );
    }

    #[test]
    fn single_statement_is_byte_identical_one_result_set_one_ready() {
        // The refactor must not add or drop messages for the common single-statement
        // case: RowDescription, DataRow, CommandComplete, ReadyForQuery — exactly.
        let mut c = conn();
        let msgs = run(&mut c, "SELECT 1 AS a");
        assert_eq!(msgs.len(), 4, "{msgs:?}");
        assert!(
            matches!(msgs[0], BackendMessage::RowDescription { .. }),
            "{msgs:?}"
        );
        assert!(
            matches!(msgs[1], BackendMessage::DataRow { .. }),
            "{msgs:?}"
        );
        assert!(
            matches!(msgs[2], BackendMessage::CommandComplete { .. }),
            "{msgs:?}"
        );
        assert!(
            matches!(msgs[3], BackendMessage::ReadyForQuery { .. }),
            "{msgs:?}"
        );
    }

    #[test]
    fn error_midbatch_emits_earlier_result_then_error_then_one_ready() {
        // Per-statement autocommit (the documented divergence) is unchanged: the first
        // SELECT's result set is emitted, then the second statement's error, then a
        // single ReadyForQuery closes the message.
        let mut c = conn();
        let msgs = run(&mut c, "SELECT 1 AS a; SELECT 1/0");

        assert!(
            msgs.iter()
                .any(|m| matches!(m, BackendMessage::RowDescription { .. }))
                && !data_rows(&msgs).is_empty(),
            "the first statement's result set must still be emitted: {msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| matches!(m, BackendMessage::ErrorResponse { .. })),
            "the erroring second statement must surface an ErrorResponse: {msgs:?}"
        );
        assert_eq!(
            count(&msgs, |m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            1,
            "exactly one trailing ReadyForQuery even on a mid-batch error: {msgs:?}"
        );
        assert!(
            matches!(msgs.last(), Some(BackendMessage::ReadyForQuery { .. })),
            "the ReadyForQuery is the final message: {msgs:?}"
        );
    }

    #[test]
    fn a_leading_set_does_not_swallow_the_rest_of_the_batch() {
        // The standard driver session-setup batch. Before the per-statement
        // interception this answered one `SET` and the INSERT never ran — no error,
        // and `count(*)` stayed 0. Red on revert.
        let mut c = conn();
        run(&mut c, "CREATE TABLE ss (id int)");
        let msgs = run(&mut c, "SET TIME ZONE 'UTC'; INSERT INTO ss VALUES (1)");
        assert_eq!(
            tags(&msgs),
            vec!["SET".to_string(), "INSERT 0 1".to_string()],
            "{msgs:?}"
        );
        assert_eq!(
            count(&msgs, |m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            1,
            "exactly one trailing ReadyForQuery: {msgs:?}"
        );
        let rows = data_rows(&run(&mut c, "SELECT count(*) FROM ss"));
        assert_eq!(rows[0][0].as_deref(), Some(b"1".as_ref()), "the INSERT ran");
    }

    #[test]
    fn set_and_show_mid_batch_take_their_turn_in_order() {
        // Not the leading position: before, a mid-batch `SET`/`SHOW` reached the
        // parser and was `0A000` (loud, and it dropped the statements after it).
        let mut c = conn();
        let msgs = run(
            &mut c,
            "SELECT 1 AS a; SET client_encoding TO 'UTF8'; SHOW server_version; SELECT 2 AS b",
        );
        let tags = tags(&msgs);
        assert_eq!(tags.len(), 4, "one CommandComplete per statement: {msgs:?}");
        assert!(tags[0].starts_with("SELECT"), "{tags:?}");
        assert_eq!(tags[1], "SET", "{tags:?}");
        assert_eq!(tags[2], "SHOW", "{tags:?}");
        assert!(tags[3].starts_with("SELECT"), "{tags:?}");
        assert_eq!(
            row_desc_first_names(&msgs),
            vec![
                "a".to_string(),
                "server_version".to_string(),
                "b".to_string()
            ],
            "the SHOW's result set sits between the two SELECTs': {msgs:?}"
        );
        assert_eq!(data_rows(&msgs).len(), 3, "{msgs:?}");
        assert_eq!(
            count(&msgs, |m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            1,
            "{msgs:?}"
        );
    }

    #[test]
    fn a_lone_set_is_byte_identical_command_complete_then_ready() {
        // The single-statement shape every driver sends on connect must not grow a
        // RowDescription (a zero-column result) or lose its ReadyForQuery.
        let mut c = conn();
        let msgs = run(&mut c, "SET TIME ZONE 'UTC'");
        assert_eq!(msgs.len(), 2, "{msgs:?}");
        assert!(
            matches!(&msgs[0], BackendMessage::CommandComplete { tag } if tag == "SET"),
            "{msgs:?}"
        );
        assert!(
            matches!(
                msgs[1],
                BackendMessage::ReadyForQuery {
                    status: TransactionStatus::Idle
                }
            ),
            "{msgs:?}"
        );
    }

    #[test]
    fn a_show_error_mid_batch_keeps_its_sqlstate_after_the_earlier_result_sets() {
        // The typed error survives the move into the loop: the earlier SELECT's
        // result set, then `42704`, then the one ReadyForQuery — and the statement
        // after the error never runs.
        let mut c = conn();
        run(&mut c, "CREATE TABLE se (id int)");
        let msgs = run(
            &mut c,
            "SELECT 1 AS a; SHOW no_such_parameter_here; INSERT INTO se VALUES (1)",
        );
        assert_eq!(data_rows(&msgs).len(), 1, "{msgs:?}");
        let code = msgs.iter().find_map(|m| match m {
            BackendMessage::ErrorResponse { fields } => fields
                .iter()
                .find(|(k, _)| *k == b'C')
                .map(|(_, v)| v.clone()),
            _ => None,
        });
        assert_eq!(code.as_deref(), Some("42704"), "{msgs:?}");
        assert_eq!(
            count(&msgs, |m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            1,
            "{msgs:?}"
        );
        assert!(
            matches!(msgs.last(), Some(BackendMessage::ReadyForQuery { .. })),
            "{msgs:?}"
        );
        let rows = data_rows(&run(&mut c, "SELECT count(*) FROM se"));
        assert_eq!(
            rows[0][0].as_deref(),
            Some(b"0".as_ref()),
            "the INSERT did not run"
        );
    }
}

#[cfg(test)]
mod deallocate_tests {
    //! `DEALLOCATE` over the connection's prepared-statement namespace.
    //!
    //! The SQL surface — the four spellings, the `26000`, the syntax errors — is
    //! pinned by `tests/fixtures/commands/deallocate_*`, differentially against 18.6.
    //! What no fixture can reach is the namespace itself: the harness never names a
    //! prepared statement, so every fixture runs against an empty one and could not
    //! tell a working `DEALLOCATE` from one that removes nothing.

    use pglet_wire::TransactionStatus;

    use crate::test_support::{conn, prepared_stub, run, sqlstate, tags};

    /// `DEALLOCATE` reaches the connection's own namespace, which no fixture can
    /// address: the differential harness never names a prepared statement, so every
    /// SQL-level fixture for this statement runs against an *empty* one. These are
    /// the only guards that a name actually goes away.
    #[test]
    fn deallocate_removes_a_named_prepared_statement() {
        let mut c = conn();
        c.prepared.insert("ps".into(), prepared_stub());
        assert_eq!(tags(&run(&mut c, "DEALLOCATE ps")), vec!["DEALLOCATE"]);
        assert!(!c.prepared.contains_key("ps"));
        // The second one is `26000`, which is what separates this statement from the
        // wire's `Close` over the same store.
        assert_eq!(
            sqlstate(&run(&mut c, "DEALLOCATE ps")).as_deref(),
            Some("26000")
        );
    }

    /// `DEALLOCATE ALL` clears the **named** statements and leaves the unnamed slot.
    ///
    /// Postgres keeps the unnamed statement in a separate `unnamed_stmt_psrc` rather
    /// than in `prepared_queries`, so no `DEALLOCATE` can reach it; pglet keys it
    /// `""` in the same map, so the exemption is a rule this engine has to write
    /// down. Without it a driver's session-reset `DEALLOCATE ALL` would close the
    /// statement behind a live unnamed portal.
    #[test]
    fn deallocate_all_spares_the_unnamed_statement() {
        let mut c = conn();
        c.prepared.insert(String::new(), prepared_stub());
        c.prepared.insert("a".into(), prepared_stub());
        c.prepared.insert("b".into(), prepared_stub());
        assert_eq!(tags(&run(&mut c, "DEALLOCATE ALL")), vec!["DEALLOCATE ALL"]);
        assert_eq!(c.prepared.keys().collect::<Vec<_>>(), vec![""]);
    }

    /// The name folds as a `ColId`, so an unquoted spelling reaches a statement
    /// created under any casing and a quoted one reaches only the verbatim name —
    /// which is how a driver's `Parse`-generated `"S_1"` is addressed. The fixtures
    /// can only compare SQLSTATEs, and both spellings are `26000` over an empty
    /// namespace, so the fold is invisible to them.
    #[test]
    fn deallocate_folds_its_name_like_an_identifier() {
        let mut c = conn();
        c.prepared.insert("foo".into(), prepared_stub());
        c.prepared.insert("S_1".into(), prepared_stub());
        assert_eq!(tags(&run(&mut c, "DEALLOCATE FoO")), vec!["DEALLOCATE"]);
        assert!(!c.prepared.contains_key("foo"));
        // Unquoted cannot reach the verbatim name, quoted can.
        assert_eq!(
            sqlstate(&run(&mut c, "DEALLOCATE S_1")).as_deref(),
            Some("26000")
        );
        assert_eq!(
            tags(&run(&mut c, r#"DEALLOCATE "S_1""#)),
            vec!["DEALLOCATE"]
        );
        assert!(c.prepared.is_empty());
    }

    /// An aborted block refuses it — `DeallocateStmt` is not a `TransactionStmt`, so
    /// `IsTransactionExitStmt` does not admit it. The gate has to sit ahead of the
    /// interception rather than beside it. No fixture can carry this: the failing
    /// statement would be scored as the fixture's own result.
    #[test]
    fn deallocate_in_an_aborted_block_is_refused() {
        let mut c = conn();
        run(&mut c, "BEGIN");
        run(&mut c, "SELECT nosuchthing");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
        assert_eq!(
            sqlstate(&run(&mut c, "DEALLOCATE ALL")).as_deref(),
            Some("25P02")
        );
    }

    /// It is not transactional: `prepared_queries` lives in `TopMemoryContext` with
    /// no `AtEOXact` callback, so a `ROLLBACK` does not put the statement back.
    #[test]
    fn deallocate_is_not_rolled_back() {
        let mut c = conn();
        c.prepared.insert("ps".into(), prepared_stub());
        run(&mut c, "BEGIN");
        assert_eq!(tags(&run(&mut c, "DEALLOCATE ps")), vec!["DEALLOCATE"]);
        run(&mut c, "ROLLBACK");
        assert!(!c.prepared.contains_key("ps"), "the rollback restored it");
    }
}

#[cfg(test)]
mod discard_tests {
    //! `DISCARD` over a live connection — the two things it reaches that no
    //! differential fixture can address.
    //!
    //! The SQL surface (the five words, the tags, the `25001`, the syntax errors) is
    //! pinned by `tests/fixtures/commands/discard_*`, differentially against 18.6.
    //! What a fixture cannot reach is the same pair `DEALLOCATE`'s tests name: the
    //! harness never names a prepared statement, so every fixture runs against an
    //! empty namespace and could not tell a working `DISCARD ALL` from one that
    //! removes nothing.

    use pglet_wire::TransactionStatus;

    use crate::test_support::{conn, prepared_stub, run, sqlstate, tags};

    #[test]
    fn discard_all_clears_the_connections_named_prepared_statements() {
        let mut c = conn();
        c.prepared.insert(String::new(), prepared_stub());
        c.prepared.insert("s1".into(), prepared_stub());
        c.prepared.insert("s2".into(), prepared_stub());
        assert_eq!(tags(&run(&mut c, "DISCARD ALL")), vec!["DISCARD ALL"]);
        // The unnamed slot survives, for the reason `DEALLOCATE ALL`'s does:
        // Postgres keeps it outside `prepared_queries`, so the same
        // `DropAllPreparedStatements` `DISCARD ALL` calls cannot reach it.
        assert_eq!(c.prepared.keys().collect::<Vec<_>>(), vec![""]);
        // And a name that is gone is `26000` on the next reference — the observable
        // consequence a pool's check-in `DISCARD ALL` has.
        assert_eq!(
            sqlstate(&run(&mut c, "DEALLOCATE s1")).as_deref(),
            Some("26000")
        );
    }

    #[test]
    fn the_other_three_targets_leave_the_namespace_alone() {
        // `DISCARD PLANS` in particular: Postgres's `ResetPlanCache` throws away
        // cached plans and *keeps* the statements holding them, so a
        // `PREPARE p …; DISCARD PLANS; EXECUTE p` still runs there. Clearing the
        // namespace here would have been the plausible wrong reading of the name.
        for (sql, tag) in [
            ("DISCARD PLANS", "DISCARD PLANS"),
            ("DISCARD SEQUENCES", "DISCARD SEQUENCES"),
            ("DISCARD TEMP", "DISCARD TEMP"),
            ("DISCARD TEMPORARY", "DISCARD TEMP"),
        ] {
            let mut c = conn();
            c.prepared.insert("s1".into(), prepared_stub());
            assert_eq!(tags(&run(&mut c, sql)), vec![tag.to_string()], "{sql}");
            assert!(c.prepared.contains_key("s1"), "{sql} dropped the statement");
        }
    }

    #[test]
    fn discard_all_is_refused_inside_a_transaction_block() {
        // `PreventInTransactionBlock`, and the block survives the refusal as an
        // *aborted* one — the refusal is an ordinary statement error, not a
        // transaction-control statement.
        let mut c = conn();
        run(&mut c, "BEGIN");
        assert_eq!(
            sqlstate(&run(&mut c, "DISCARD ALL")).as_deref(),
            Some("25001")
        );
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
    }

    #[test]
    fn an_aborted_block_refuses_discard_ahead_of_its_own_gate() {
        // `DiscardStmt` is not a `TransactionStmt`, so `IsTransactionExitStmt` does
        // not admit it and `25P02` outranks the `25001` this same statement would
        // otherwise take. The gate has to sit ahead of the interception, not beside
        // it.
        let mut c = conn();
        run(&mut c, "BEGIN");
        run(&mut c, "SELECT nosuchthing");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
        assert_eq!(
            sqlstate(&run(&mut c, "DISCARD ALL")).as_deref(),
            Some("25P02")
        );
        assert_eq!(
            sqlstate(&run(&mut c, "RESET ALL")).as_deref(),
            Some("25P02")
        );
    }

    #[test]
    fn discard_sequences_clears_this_connections_currval() {
        // The one target with an observable effect on session state, and the
        // connection's `sequence_session` is where it lives — invisible to
        // everything below the socket.
        let mut c = conn();
        run(&mut c, "CREATE SEQUENCE sq");
        run(&mut c, "SELECT nextval('sq')");
        assert!(sqlstate(&run(&mut c, "SELECT currval('sq')")).is_none());
        assert_eq!(
            tags(&run(&mut c, "DISCARD SEQUENCES")),
            vec!["DISCARD SEQUENCES"]
        );
        assert_eq!(
            sqlstate(&run(&mut c, "SELECT currval('sq')")).as_deref(),
            Some("55000")
        );
    }
}

#[cfg(test)]
mod with_dispatch_tests {
    //! Which autocommit path a `Plan::With` takes is decided by
    //! `Plan::is_read_only` / `Plan::mutates_storage`, and a `WITH` statement's
    //! entries do not decide it alone: a `WITH RECURSIVE` list of recursive CTEs is
    //! read-only while its body may write. The fixture harness runs a fixture inside
    //! `BEGIN … ROLLBACK` unless it opts out (`@no-txn`), and inside a block a statement
    //! takes neither the lock-free read path nor the wrapped autocommit write, so these
    //! drive the dispatch itself.

    use pglet_wire::BackendMessage;

    use crate::test_support::{conn, run, sqlstate, tags};

    fn first_cell(msgs: &[BackendMessage]) -> Option<String> {
        msgs.iter().find_map(|m| match m {
            BackendMessage::DataRow { values } => values
                .first()
                .and_then(|v| v.as_ref().map(|b| String::from_utf8_lossy(b).into_owned())),
            _ => None,
        })
    }

    const RECURSIVE: &str =
        "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3)";

    /// A body that writes takes the write path — the lock-free path's snapshot adapter
    /// refuses the write (`0A000`) — and the wrapped one, whose storage transaction
    /// puts back the rows a statement wrote before it failed.
    #[test]
    fn a_recursive_with_whose_body_writes_takes_the_wrapped_write_path() {
        let mut c = conn();
        run(&mut c, "CREATE TABLE s (x INT CHECK (x < 3))");
        // The third row fails its CHECK after the first two are written.
        let failed = run(
            &mut c,
            &format!("{RECURSIVE} INSERT INTO s SELECT n FROM r"),
        );
        assert_eq!(sqlstate(&failed).as_deref(), Some("23514"), "{failed:?}");
        let counted = run(&mut c, "SELECT count(*) FROM s");
        assert_eq!(
            first_cell(&counted).as_deref(),
            Some("0"),
            "a failed statement leaves no row: {counted:?}"
        );
        let inserted = run(
            &mut c,
            &format!("{RECURSIVE} INSERT INTO s SELECT n FROM r WHERE n < 3"),
        );
        assert_eq!(tags(&inserted), ["INSERT 0 2"], "{inserted:?}");
    }

    /// The same list over a `SELECT` body is a read, and answers.
    #[test]
    fn a_recursive_with_whose_body_reads_answers() {
        let mut c = conn();
        let msgs = run(&mut c, &format!("{RECURSIVE} SELECT count(*) FROM r"));
        assert_eq!(first_cell(&msgs).as_deref(), Some("3"), "{msgs:?}");
    }
}

#[cfg(test)]
mod writer_tests {
    //! The writer on the native server, driven in process: two connections over one
    //! [`SharedState`], the second on a thread of its own so that it can wait. The
    //! socket-level twins — 18.6's answers after each wait, every writing door, the
    //! `lock_timeout` bound — are `pglet_tests`' `test_concurrent`; these are the ones
    //! that read what only the process can see: the storage's cross-connection counter,
    //! the store failing under a waiter, and a waiter's plan after the holder took its
    //! catalog back.

    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    use pglet::StorageBackend;
    use pglet_testing::block_io::MemBlockIo;
    use pglet_testing::db::open_database;

    use crate::connection::{Connection, SharedState};
    use crate::test_support::{cells, run, sqlstate};

    /// Run `statements` on a new connection `pid` over `shared` on a thread of its own,
    /// sending each one's SQLSTATE (`None`: success) as it answers.
    fn spawn(
        shared: &Arc<SharedState>,
        pid: i32,
        statements: &[&str],
    ) -> mpsc::Receiver<Option<String>> {
        let shared = Arc::clone(shared);
        let statements: Vec<String> = statements.iter().map(|s| (*s).to_string()).collect();
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let mut conn = Connection::new(pid, shared);
            for sql in statements {
                if send.send(sqlstate(&run(&mut conn, &sql))).is_err() {
                    return;
                }
            }
        });
        receive
    }

    /// Long enough that a statement answering at once has answered.
    const SETTLE: Duration = Duration::from_millis(200);

    /// The storage's count of reads of another connection's staged writes that found
    /// one — the claim walk and the table-id restore's other-transaction arm.
    fn cross_connection_hits(shared: &SharedState) -> u64 {
        match &*shared.lock_storage() {
            StorageBackend::Btree(btree) => btree.cross_connection_hits(),
            StorageBackend::Memory(_) => 0,
        }
    }

    /// A second writer waits for the block holding the writer and then answers against
    /// its outcome — so no statement ever reads another connection's staged writes: the
    /// storage counter of such reads stays `0` across the shapes the answer-at-once
    /// design answered from them (a staged key under an `INSERT`, an `ON CONFLICT`, an
    /// `UPDATE` onto it; a table a block created, written into). Red with the wait
    /// removed: the second writer answers from the claim at once, and the count moves.
    #[test]
    fn a_second_writer_waits_and_reads_no_staged_write() {
        let shared = Arc::new(SharedState::with_btree());
        let mut a = Connection::new(1, Arc::clone(&shared));
        run(&mut a, "CREATE TABLE w (k TEXT PRIMARY KEY, v INT)");
        run(&mut a, "INSERT INTO w VALUES ('other', 0)");
        for (end, statement, answer) in [
            ("COMMIT", "INSERT INTO w VALUES ('dup', 2)", Some("23505")),
            ("ROLLBACK", "INSERT INTO w VALUES ('dup', 2)", None),
            (
                "COMMIT",
                "INSERT INTO w VALUES ('dup', 3) ON CONFLICT (k) DO NOTHING",
                None,
            ),
            (
                "COMMIT",
                "UPDATE w SET k = 'dup' WHERE k = 'other'",
                Some("23505"),
            ),
        ] {
            run(&mut a, "DELETE FROM w WHERE k = 'dup'");
            run(&mut a, "BEGIN");
            run(&mut a, "INSERT INTO w VALUES ('dup', 1)");
            let b = spawn(&shared, 2, &[statement]);
            assert!(
                b.recv_timeout(SETTLE).is_err(),
                "{statement}: the second writer waits for the block"
            );
            assert_eq!(sqlstate(&run(&mut a, end)), None, "{end}");
            let got = b.recv_timeout(Duration::from_secs(5)).expect("answered");
            assert_eq!(got.as_deref(), answer, "{statement}, the holder's {end}");
        }
        // A table a block created, and a write into it from another connection.
        run(&mut a, "BEGIN");
        run(&mut a, "CREATE TABLE made (i INT)");
        let b = spawn(&shared, 3, &["INSERT INTO made VALUES (42)"]);
        assert!(b.recv_timeout(SETTLE).is_err(), "the write into it waits");
        assert_eq!(sqlstate(&run(&mut a, "ROLLBACK")), None);
        assert_eq!(
            b.recv_timeout(Duration::from_secs(5))
                .expect("answered")
                .as_deref(),
            Some("42P01"),
            "re-planned against the catalog the rollback restored"
        );
        assert_eq!(
            cross_connection_hits(&shared),
            0,
            "no statement read another connection's staged writes"
        );
    }

    /// A block's first write re-bases it on what is committed: a key another connection
    /// committed after the block's `BEGIN` refuses the block's insert `23505`, and the
    /// block's rollback keeps the other connection's committed table.
    /// Red without the re-base: the insert commits a second row under the primary key
    /// (its snapshot is the `BEGIN` one), and the rollback restores the `BEGIN` catalog
    /// over the committed table.
    #[test]
    fn a_blocks_first_write_rebases_it_on_what_is_committed() {
        let shared = Arc::new(SharedState::with_btree());
        let mut a = Connection::new(1, Arc::clone(&shared));
        let mut b = Connection::new(2, Arc::clone(&shared));
        run(&mut a, "CREATE TABLE k (k INT PRIMARY KEY)");
        run(&mut a, "BEGIN");
        run(&mut a, "SELECT count(*) FROM k");
        assert_eq!(sqlstate(&run(&mut b, "INSERT INTO k VALUES (1)")), None);
        assert_eq!(sqlstate(&run(&mut b, "CREATE TABLE y (i INT)")), None);
        assert_eq!(
            sqlstate(&run(&mut a, "INSERT INTO k VALUES (1)")).as_deref(),
            Some("23505"),
            "a key committed after BEGIN refuses the block's insert"
        );
        assert_eq!(sqlstate(&run(&mut a, "ROLLBACK")), None);
        assert_eq!(
            sqlstate(&run(&mut b, "SELECT count(*) FROM y")),
            None,
            "the rollback keeps the committed table"
        );
        assert_eq!(sqlstate(&run(&mut a, "INSERT INTO k VALUES (2)")), None);
    }

    /// A plan failing because of another connection's uncommitted catalog change is not the
    /// statement's answer: an `UPDATE` of a column another connection's open block dropped
    /// — which plans against the catalog committed when the block took the writer — **waits** for that
    /// block and is planned again against what it left — it succeeds after the block's
    /// `ROLLBACK` (the column is back) and answers `42703` after its `COMMIT`, 18.6's two
    /// answers (its parse analysis waits on the `ALTER`'s lock). So do a `DELETE` and an
    /// `INSERT … SELECT` reading the column, and a read of it. A syntax error planning
    /// reaches first (`SELEC 1`, `UPDATE d SET = 1`), and a failure against a catalog the
    /// holder has not changed (a block that only wrote rows), answer at once. Red with the plan's failure answered before the wait: the `UPDATE`
    /// answers `42703` at once, although the column is back after the rollback.
    #[test]
    fn a_plan_failing_against_a_holders_uncommitted_catalog_waits_and_plans_again() {
        for shared in [SharedState::with_btree(), SharedState::in_memory()] {
            let shared = Arc::new(shared);
            let mut a = Connection::new(1, Arc::clone(&shared));
            run(&mut a, "CREATE TABLE d (id INT, c INT)");
            run(&mut a, "INSERT INTO d VALUES (1, 1)");
            for (end, answer) in [("ROLLBACK", None), ("COMMIT", Some("42703"))] {
                for statement in [
                    "UPDATE d SET c = 9",
                    "DELETE FROM d WHERE c = 7",
                    "INSERT INTO d SELECT 2, c FROM d WHERE c = 7",
                    "SELECT c FROM d",
                ] {
                    run(&mut a, "ALTER TABLE d ADD COLUMN IF NOT EXISTS c INT");
                    run(&mut a, "BEGIN");
                    run(&mut a, "ALTER TABLE d DROP COLUMN c");
                    let b = spawn(&shared, 2, &[statement]);
                    assert!(
                        b.recv_timeout(SETTLE).is_err(),
                        "{statement}: waits for the block that dropped the column"
                    );
                    assert_eq!(sqlstate(&run(&mut a, end)), None, "{end}");
                    let got = b.recv_timeout(Duration::from_secs(5)).expect("answered");
                    assert_eq!(got.as_deref(), answer, "{statement}, after the {end}");
                }
            }
            // The holder changed rows only: a failing plan answers at once.
            run(&mut a, "BEGIN");
            run(&mut a, "INSERT INTO d VALUES (3)");
            let b = spawn(&shared, 3, &["UPDATE d SET nosuch = 1", "SELEC 1"]);
            for want in ["42703", "42601"] {
                assert_eq!(
                    b.recv_timeout(SETTLE).expect("at once").as_deref(),
                    Some(want)
                );
            }
            // And a syntax error answers at once over a changed catalog too.
            run(&mut a, "ALTER TABLE d ADD COLUMN e INT");
            let b = spawn(&shared, 4, &["UPDATE d SET = 1"]);
            assert_eq!(
                b.recv_timeout(SETTLE).expect("at once").as_deref(),
                Some("42601")
            );
            run(&mut a, "ROLLBACK");
        }
    }

    /// A `42601` is not exempted from the wait by its SQLSTATE: a set operation's column
    /// counts are pglet's analysis, and a `SELECT *` side's width is the catalog's — so
    /// behind a block that dropped or added a column, `SELECT * FROM d UNION ALL SELECT
    /// 7, 8` fails `42601` against the block's uncommitted catalog and succeeds against
    /// the committed one. It waits and is planned again (rows after the `ROLLBACK`; the
    /// `42601` after a `COMMIT` that leaves the widths unequal), as 18.6's parse analysis
    /// waits on the `ALTER`'s lock. A grammar error planning reaches first still answers
    /// at once (`a_plan_failing_against_a_holders_uncommitted_catalog_waits_and_plans_again`);
    /// one behind column counts the block's change decides waits too (the repo's `EDGES.md`
    /// row 238).
    /// Red with every `42601` taken as the statement's own: the set operation answers
    /// `42601` at once.
    #[test]
    fn a_set_operations_arity_failing_against_a_holders_catalog_waits() {
        for shared in [SharedState::with_btree(), SharedState::in_memory()] {
            let shared = Arc::new(shared);
            let mut a = Connection::new(1, Arc::clone(&shared));
            run(&mut a, "CREATE TABLE d (id INT, c INT)");
            run(&mut a, "INSERT INTO d VALUES (1, 1)");
            let statement = "SELECT * FROM d UNION ALL SELECT 7, 8";
            for (ddl, end, answer) in [
                ("ALTER TABLE d DROP COLUMN c", "ROLLBACK", None),
                ("ALTER TABLE d ADD COLUMN e INT", "ROLLBACK", None),
                ("ALTER TABLE d DROP COLUMN c", "COMMIT", Some("42601")),
            ] {
                run(&mut a, "BEGIN");
                run(&mut a, ddl);
                let b = spawn(&shared, 2, &[statement]);
                assert!(
                    b.recv_timeout(SETTLE).is_err(),
                    "{ddl}: waits for the block"
                );
                assert_eq!(sqlstate(&run(&mut a, end)), None, "{end}");
                let got = b.recv_timeout(Duration::from_secs(5)).expect("answered");
                assert_eq!(got.as_deref(), answer, "{ddl}, after the {end}");
            }
        }
    }

    /// A plan that fails for its own reason answers at once whatever another block has
    /// done to the catalog — here a table it created, which none of these statements
    /// names: a column no catalog has, a cast no catalog decides, a function no catalog
    /// has — each answered at once with what it answers with no block open, as 18.6
    /// answers them, its parse analysis taking no lock the block holds. So do they behind a block that changed the catalog
    /// and then went aborted. The failure the block caused still waits
    /// (`a_plan_failing_against_a_holders_uncommitted_catalog_waits_and_plans_again`).
    /// Red with every non-syntax plan failure behind a changed catalog taken as the
    /// block's: each waits out the block.
    #[test]
    fn a_plans_own_failure_answers_at_once_behind_another_blocks_catalog_change() {
        for shared in [SharedState::with_btree(), SharedState::in_memory()] {
            let shared = Arc::new(shared);
            let mut a = Connection::new(1, Arc::clone(&shared));
            run(&mut a, "CREATE TABLE pu (i INT)");
            run(&mut a, "INSERT INTO pu VALUES (1)");
            let statements = [
                "SELECT nosuchcol FROM pu",
                "SELECT 'abc'::int",
                "SELECT no_such_fn(1)",
                "UPDATE pu SET nosuchcol = 1",
            ];
            // Each statement's own answer, with no other block open — `42703`, `22P02`,
            // pglet's `0A000` for the unknown function (18.6's `42883`, a divergence of
            // its own), `42703`.
            let own: Vec<(&str, String)> = statements
                .iter()
                .map(|sql| (*sql, sqlstate(&run(&mut a, sql)).expect("fails")))
                .collect();
            for (pid, label, holder) in [
                (
                    2,
                    "a table created",
                    &["BEGIN", "CREATE TABLE zz_unrelated (x INT)"][..],
                ),
                (
                    3,
                    "a table created, then aborted",
                    &["BEGIN", "CREATE TABLE zz_aborted (x INT)", "SELECT 1/0"][..],
                ),
            ] {
                for sql in holder {
                    run(&mut a, sql);
                }
                let b = spawn(&shared, pid, &statements);
                for (sql, want) in &own {
                    assert_eq!(
                        b.recv_timeout(SETTLE)
                            .unwrap_or_else(|_| panic!("{label}: {sql} answers at once"))
                            .as_deref(),
                        Some(want.as_str()),
                        "{label}: {sql}"
                    );
                }
                assert_eq!(sqlstate(&run(&mut a, "ROLLBACK")), None, "{label}");
            }
        }
    }

    /// A `REPEATABLE READ` / `SERIALIZABLE` block's snapshot is taken by its first
    /// statement, not by `BEGIN` — 18.6's rule (`mvcc.sgml`: "as of the start of the first
    /// non-transaction-control statement"). So a block whose first statement is its write
    /// is re-based on what committed between its `BEGIN` and that statement, not refused:
    /// another connection's committed `c = 5` is read by the block's `UPDATE … c * 10`,
    /// which answers `UPDATE 1` and leaves `50`, 18.6's answers. Both levels, both storage
    /// backends. Red with the check measured from `BEGIN`: `40001`, final `5`. And a block
    /// whose first statement is a read keeps the stamp of the snapshot that read saw — red
    /// with every first statement stamped at its start: the write lands over `5`.
    #[test]
    fn a_blocks_first_statement_write_is_rebased_not_refused() {
        for (snapshots, shared) in [
            (true, SharedState::with_btree()),
            (false, SharedState::in_memory()),
        ] {
            let shared = Arc::new(shared);
            let mut a = Connection::new(1, Arc::clone(&shared));
            let mut b = Connection::new(2, Arc::clone(&shared));
            run(&mut a, "CREATE TABLE f (id INT PRIMARY KEY, c INT)");
            for begin in [
                "BEGIN ISOLATION LEVEL REPEATABLE READ",
                "BEGIN ISOLATION LEVEL SERIALIZABLE",
            ] {
                run(&mut a, "DELETE FROM f");
                run(&mut a, "INSERT INTO f VALUES (1, 0)");
                run(&mut a, begin);
                assert_eq!(
                    sqlstate(&run(&mut b, "UPDATE f SET c = 5")),
                    None,
                    "{begin}"
                );
                assert_eq!(
                    sqlstate(&run(&mut a, "UPDATE f SET c = c * 10")),
                    None,
                    "{begin}: the first statement takes the snapshot"
                );
                assert_eq!(sqlstate(&run(&mut a, "COMMIT")), None, "{begin}");
                assert_eq!(cells(&run(&mut b, "SELECT c FROM f")), ["50"], "{begin}");
                // A block whose first statement is a read reads the storage snapshot its
                // `BEGIN` took (18.6 reads the commit: the repo's `EDGES.md` row 219), so its
                // write is checked against that stamp and refused — loud, where re-basing
                // it would write over a value the block never read.
                run(&mut a, "UPDATE f SET c = 0");
                run(&mut a, begin);
                assert_eq!(
                    sqlstate(&run(&mut b, "UPDATE f SET c = 5")),
                    None,
                    "{begin}"
                );
                // Memory storage keeps no snapshot: its reads are live.
                let read = if snapshots { "0" } else { "5" };
                assert_eq!(cells(&run(&mut a, "SELECT c FROM f")), [read], "{begin}");
                assert_eq!(
                    sqlstate(&run(&mut a, "UPDATE f SET c = 50")).as_deref(),
                    Some("40001"),
                    "{begin}: the read's snapshot is BEGIN's"
                );
                run(&mut a, "ROLLBACK");
                assert_eq!(cells(&run(&mut b, "SELECT c FROM f")), ["5"], "{begin}");
            }
        }
    }

    /// A `COPY … TO` in a block takes its snapshot as any other statement does, so a
    /// `SET TRANSACTION ISOLATION LEVEL` after it is `25001`, 18.6's answer (its
    /// `FirstSnapshotSet`); a `SET` or `SHOW` before it takes none, and the level still
    /// changes. Red with `dispatch_copy` noting no snapshot: the `SET TRANSACTION` after
    /// the `COPY` is accepted.
    #[test]
    fn a_blocks_copy_to_fixes_its_isolation_level() {
        for shared in [SharedState::with_btree(), SharedState::in_memory()] {
            let shared = Arc::new(shared);
            let mut a = Connection::new(1, Arc::clone(&shared));
            run(&mut a, "CREATE TABLE ct (i INT)");
            run(&mut a, "BEGIN");
            run(&mut a, "SHOW transaction_isolation");
            assert_eq!(
                sqlstate(&run(
                    &mut a,
                    "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"
                )),
                None,
                "a SHOW takes no snapshot"
            );
            assert_eq!(
                sqlstate(&super::handle_query(&mut a, "COPY ct TO STDOUT")),
                None,
                "the COPY"
            );
            assert_eq!(
                sqlstate(&run(&mut a, "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")).as_deref(),
                Some("25001"),
                "the COPY took the snapshot"
            );
            run(&mut a, "ROLLBACK");
        }
    }

    /// A `REPEATABLE READ` / `SERIALIZABLE` block keeps its snapshot and is refused
    /// `40001` at its first write when another connection committed since it began
    /// (S13RR: the second read is `0`, the `UPDATE` `40001`, the final value `5` — 18.6's
    /// answers), gives the writer back at once (another connection's write does not wait
    /// on it), and at `READ COMMITTED` the same block re-bases and writes over the commit
    /// (`55`). With nothing committed since, the `REPEATABLE READ` block writes. Both
    /// storage backends. Red with the check removed from `take_writer`: the `UPDATE`
    /// writes `50` over the committed `5`, the lost update.
    #[test]
    fn a_repeatable_read_block_is_refused_40001_past_a_commit_and_holds_no_writer() {
        for shared in [SharedState::with_btree(), SharedState::in_memory()] {
            let shared = Arc::new(shared);
            let mut a = Connection::new(1, Arc::clone(&shared));
            let mut b = Connection::new(2, Arc::clone(&shared));
            run(&mut a, "CREATE TABLE v (id INT PRIMARY KEY, c INT)");
            for (begin, committed, answer, last) in [
                (
                    "BEGIN ISOLATION LEVEL REPEATABLE READ",
                    true,
                    Some("40001"),
                    "5",
                ),
                (
                    "BEGIN ISOLATION LEVEL SERIALIZABLE",
                    true,
                    Some("40001"),
                    "5",
                ),
                ("BEGIN ISOLATION LEVEL REPEATABLE READ", false, None, "50"),
                ("BEGIN", true, None, "55"),
            ] {
                let cell = format!("{begin}, a commit between {committed}");
                run(&mut a, "DELETE FROM v");
                run(&mut a, "INSERT INTO v VALUES (1, 0)");
                run(&mut a, begin);
                assert_eq!(cells(&run(&mut a, "SELECT c FROM v")), ["0"], "{cell}");
                if committed {
                    assert_eq!(sqlstate(&run(&mut b, "UPDATE v SET c = 5")), None, "{cell}");
                }
                assert_eq!(
                    sqlstate(&run(&mut a, "UPDATE v SET c = c + 50")).as_deref(),
                    answer,
                    "{cell}"
                );
                if answer.is_some() {
                    assert!(
                        !shared.writer.is_held_by(1),
                        "{cell}: the writer given back"
                    );
                    assert_eq!(
                        sqlstate(&run(&mut b, "INSERT INTO v VALUES (9, 9)")),
                        None,
                        "{cell}: another write does not wait on the refused block"
                    );
                    run(&mut b, "DELETE FROM v WHERE id = 9");
                    run(&mut a, "ROLLBACK");
                } else {
                    run(&mut a, "COMMIT");
                }
                assert_eq!(cells(&run(&mut b, "SELECT c FROM v")), [last], "{cell}");
            }
        }
    }

    /// A connection's savepoint levels are its own on `--storage memory`, whose store
    /// held one stack for every connection: another connection's read-only block ending
    /// and a third's `SAVEPOINT` no longer move the writer's levels, so its `ROLLBACK TO`
    /// restores its own savepoint (`1`, 18.6's answer) where it restored the third's level
    /// and committed `1, 2`, and with the third absent it no longer answers `22000
    /// savepoint stacks out of step`; and the writer's whole `ROLLBACK`, which puts its
    /// `BEGIN` copy of the store back, leaves the third's level standing. The B+tree, whose
    /// levels always lived per connection, is the control. Red with the memory store's
    /// stack shared again, and with the rollback putting back the whole store, levels and
    /// all.
    #[test]
    fn a_connections_savepoints_are_its_own_on_every_backend() {
        type Backend = (&'static str, fn() -> SharedState);
        let backends: [Backend; 2] = [
            ("btree", SharedState::with_btree),
            ("memory", SharedState::in_memory),
        ];
        for (backend, make) in backends {
            for (third, end, rows) in [
                (true, "COMMIT", &["1"][..]),
                (false, "COMMIT", &["1"][..]),
                (true, "ROLLBACK", &[][..]),
            ] {
                let cell = format!("{backend}, a third connection's savepoint {third}, {end}");
                let shared = Arc::new(make());
                let mut b = Connection::new(1, Arc::clone(&shared));
                let mut a = Connection::new(2, Arc::clone(&shared));
                let mut c = Connection::new(3, Arc::clone(&shared));
                run(&mut b, "CREATE TABLE sp (i INT)");
                run(&mut a, "BEGIN");
                run(&mut a, "SELECT count(*) FROM sp");
                run(&mut b, "BEGIN");
                run(&mut b, "INSERT INTO sp VALUES (1)");
                run(&mut b, "SAVEPOINT b");
                run(&mut b, "INSERT INTO sp VALUES (2)");
                assert_eq!(sqlstate(&run(&mut a, "COMMIT")), None, "{cell}");
                if third {
                    run(&mut c, "BEGIN");
                    assert_eq!(sqlstate(&run(&mut c, "SAVEPOINT x")), None, "{cell}");
                }
                run(&mut b, "INSERT INTO sp VALUES (3)");
                assert_eq!(sqlstate(&run(&mut b, "ROLLBACK TO b")), None, "{cell}");
                assert_eq!(sqlstate(&run(&mut b, end)), None, "{cell}");
                if third {
                    assert_eq!(sqlstate(&run(&mut c, "ROLLBACK TO x")), None, "{cell}");
                    run(&mut c, "ROLLBACK");
                }
                assert_eq!(
                    cells(&run(&mut a, "SELECT i FROM sp ORDER BY i")),
                    rows,
                    "{cell}"
                );
            }
        }
    }

    /// A writer waiting on a state whose store then fails is answered `57P02` — it is
    /// never left waiting on a holder that may send nothing again, nor served from the
    /// failed state. The wait here has no bound (`lock_timeout = 0`), so red with both
    /// wakes removed (the wait's look at the store, and the reopen's
    /// `WriterLock::fail_waiters`) is a waiter still waiting.
    #[test]
    fn a_waiter_on_a_store_that_fails_is_answered_57p02() {
        let io = MemBlockIo::new();
        let shared = Arc::new(SharedState::from_database(open_database(Box::new(
            io.clone(),
        ))));
        let mut a = Connection::new(1, Arc::clone(&shared));
        run(&mut a, "CREATE TABLE f (i INT)");
        run(&mut a, "BEGIN");
        run(&mut a, "INSERT INTO f VALUES (1)");
        let b = spawn(
            &shared,
            2,
            &["SET lock_timeout = 0", "INSERT INTO f VALUES (2)"],
        );
        assert_eq!(b.recv_timeout(Duration::from_secs(5)).expect("SET"), None);
        assert!(b.recv_timeout(SETTLE).is_err(), "the insert waits");
        // A refused slot write fails the store (a refused data write the commit can
        // drop would leave it serving).
        io.policy().arm_fail_slot_writes();
        let committed = run(&mut a, "COMMIT");
        io.policy().disarm();
        assert!(
            shared.write_failure_reason().is_some(),
            "the COMMIT failed the store: {committed:?}"
        );
        let got = b
            .recv_timeout(Duration::from_secs(3))
            .expect("the waiter is answered, not left waiting");
        assert_eq!(got.as_deref(), Some("57P02"));
    }
}

#[cfg(test)]
mod failed_do_tests {
    //! A failed `DO` statement leaves nothing behind on the native server's auto-commit
    //! path — the same `execute_do_block` undo the embed's tests pin, reached through this
    //! host's own statement bracket (`storage.begin` → execute → `storage.rollback`),
    //! which rolls back only the staged storage writes. Without the block's own undo the
    //! catalog kept the body's table over storage the bracket had discarded: `CREATE
    //! TABLE` of its name answered `42P07` and an `INSERT` into it `42P01`.

    use crate::test_support::{cells, conn, run, sqlstate, tags};

    /// This host hands a completed `DO`'s replaced counter cells to the session with the
    /// statement's result (`TransactionSession::record_statement`), so a block's rollback
    /// puts the sequence back on its own cell: over `s` drawn to `3`, a block whose `DO`
    /// restarts it — in its body, or in a `DO` nested in its body — answers `4` after the
    /// `ROLLBACK`, as 18.6 does. Handed nothing, the rollback left the restarted cell's
    /// `100`.
    #[test]
    fn a_block_rollback_restores_the_cell_a_completed_do_replaced() {
        let mut c = conn();
        for sql in [
            "CREATE SEQUENCE s",
            "SELECT nextval('s'), nextval('s'), nextval('s')",
        ] {
            assert_eq!(sqlstate(&run(&mut c, sql)), None, "{sql}");
        }
        for (restart, next) in [
            ("DO $$ BEGIN ALTER SEQUENCE s RESTART WITH 100; END $$", "4"),
            (
                "DO $o$ BEGIN DO $i$ BEGIN ALTER SEQUENCE s RESTART WITH 100; END $i$; END $o$",
                "5",
            ),
        ] {
            for sql in ["BEGIN", restart, "ROLLBACK"] {
                assert_eq!(sqlstate(&run(&mut c, sql)), None, "{sql}");
            }
            assert_eq!(
                cells(&run(&mut c, "SELECT nextval('s')")),
                [next],
                "{restart}"
            );
        }
    }

    #[test]
    fn a_failed_do_leaves_no_catalog_change_behind() {
        let mut c = conn();
        for sql in [
            "CREATE TABLE keep (id INT PRIMARY KEY)",
            "INSERT INTO keep VALUES (1), (2)",
        ] {
            assert_eq!(sqlstate(&run(&mut c, sql)), None, "{sql}");
        }
        let failed = run(
            &mut c,
            "DO $$ BEGIN CREATE TABLE ft (a INT); CREATE TYPE fe AS ENUM ('a'); \
             DROP TABLE keep; INSERT INTO nosuch VALUES (1); END $$",
        );
        assert_eq!(sqlstate(&failed).as_deref(), Some("42P01"));
        for sql in [
            "CREATE TABLE ft (a INT)",
            "INSERT INTO ft VALUES (1)",
            "CREATE TYPE fe AS ENUM ('z')",
        ] {
            assert_eq!(sqlstate(&run(&mut c, sql)), None, "{sql}");
        }
        assert_eq!(
            tags(&run(&mut c, "SELECT id FROM keep")),
            ["SELECT 2"],
            "the dropped table is back with its rows"
        );
    }
}
