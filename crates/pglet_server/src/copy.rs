//! `COPY … TO STDOUT` / `COPY … FROM STDIN` wire sub-protocol for the native server.
//!
//! COPY moves a bulk byte stream out-of-band — it can't ride the `QueryResult`
//! channel the normal query path uses — so it gets its own handling here, layered
//! over the pure engine functions (`execute_copy_to` / `execute_copy_from`) and the
//! wire `CopyInResponse` / `CopyOutResponse` / `CopyData` / `CopyDone` messages.
//!
//! # Both wire protocols
//!
//! psql / pg_dump drive COPY over the **simple query** protocol, but tokio-postgres
//! (and the fuz spine driver) prepare the statement and drive it over the
//! **extended** protocol. Both reach the same COPY sub-protocol once `CopyInResponse`
//! / `CopyOutResponse` is sent; the only difference is the trailing `ReadyForQuery`:
//! the simple path appends it after `CommandComplete`, while the extended path defers
//! it to the client's own `Sync`. That distinction is threaded as `extended`
//! everywhere a COPY exchange ends.
//!
//! # Two flows
//!
//! - **`COPY … TO STDOUT`** is a one-shot read: plan it, run `execute_copy_to`
//!   against the MVCC snapshot, and return `CopyOutResponse` + one `CopyData` per row
//!   + `CopyDone` + `CommandComplete` (+ `ReadyForQuery` when simple) in one batch.
//! - **`COPY … FROM STDIN`** needs the inbound stream: the dispatch returns
//!   `CopyInResponse` and sets [`Connection::copy_in`](crate::connection::Connection)
//!   (copy-in mode). While that is set, [`handle_copy_in_frame`] routes each inbound
//!   frame — `CopyData` appends to the buffer, `CopyDone` ingests it via
//!   `execute_copy_from` (persisting an autocommit write), `CopyFail` aborts.

use std::time::Instant;

use pglet_sql::Error;
use pglet_sql::catalog::Schema;
use pglet_sql::executor::{CopyParams, copy_column_count, execute_copy_from, execute_copy_to};
use pglet_sql::planner::{CopyDirection, CopyOptions, Plan};
use pglet_wire::{BackendMessage, FrontendMessage, SessionState, TransactionStatus};

use crate::connection::{Connection, CopyInState, Portal, SnapshotAdapter, StorageAdapter};
use crate::encoding::{error_response, error_with_code, extended_error};

/// Extract the resolved [`CopyOptions`] + direction from a plan, or `None` if it
/// isn't a `COPY` (the caller runs the normal path). The owned options flow into a
/// [`CopyInState`] (FROM) or are borrowed as the engine's [`CopyParams`] (TO).
fn copy_of_plan(plan: &Plan) -> Option<(CopyOptions, CopyDirection)> {
    let Plan::Copy { direction, options } = plan else {
        return None;
    };
    Some((options.clone(), *direction))
}

/// Intercept a **simple-query** `COPY` statement, returning its wire messages, or
/// `None` if `sql` is not a single `COPY` statement (the caller runs the normal
/// path).
///
/// A failed transaction (where only `ROLLBACK` is allowed) is left to
/// `execute_query_core`'s `25P02` guard. A parse error or a non-COPY plan also
/// returns `None` — `execute_query_core` re-parses and surfaces any error
/// identically, so error formatting stays single-sourced.
pub fn try_copy_simple(conn: &mut Connection, sql: &str) -> Option<Vec<BackendMessage>> {
    if conn.transaction_status() == TransactionStatus::Failed {
        return None;
    }
    if !leads_with_copy(sql) {
        return None;
    }
    // Plan under the schema lock. Only a single COPY statement is the sub-protocol;
    // anything else (parse error, multi-statement, non-COPY) falls back to `None`. A
    // plan that failed because of another connection's uncommitted catalog waits for that
    // hold to end and is planned again, as `execute::run_one_locked` does — else the
    // fallback would plan it afresh after the wait and run it outside the sub-protocol.
    let deadline = conn.writer_deadline();
    let plan = loop {
        let (planned, generation) = {
            let schema = conn.shared.lock_schema();
            // A trailing comment is no statement (`raw_statements`), so `COPY t TO
            // STDOUT; -- c` is still the sub-protocol, as it is on 18.6.
            let planned = match pglet_sql::parser::raw_statements(sql) {
                Ok(stmts) if stmts.len() == 1 => pglet_sql::parser::parse_single(stmts[0], &schema),
                _ => Ok(None),
            };
            (planned, schema.generation)
        };
        match planned {
            Err(e)
                if conn.plan_failure_is_the_writers(&e, generation, |committed| {
                    match pglet_sql::parser::raw_statements(sql) {
                        Ok(stmts) if stmts.len() == 1 => {
                            pglet_sql::parser::parse_single(stmts[0], committed)
                        }
                        _ => Ok(None),
                    }
                }) =>
            {
                if let Err(waited) = conn.wait_for_foreign_writer(deadline) {
                    return Some(copy_failure(conn, &waited, false));
                }
            }
            planned => break planned.ok().flatten(),
        }
    };
    let (options, direction) = copy_of_plan(&plan?)?;
    Some(dispatch_copy(conn, options, direction, false, deadline))
}

/// Intercept an **extended-protocol** `COPY` at `Execute`, returning its wire
/// messages, or `None` if the portal isn't a `COPY` (the caller runs the normal
/// path). tokio-postgres and the spine driver prepare COPY, so this is the path that
/// matters for them.
///
/// The cached plan (built at `Parse`) is used when present; otherwise the portal's
/// query is re-parsed (a COPY always plans). A failed transaction is left to
/// `execute_query_core`'s `25P02` guard.
pub fn try_copy_extended(conn: &mut Connection, portal: &Portal) -> Option<Vec<BackendMessage>> {
    if conn.transaction_status() == TransactionStatus::Failed {
        return None;
    }
    let (options, direction) = if let Some(plan) = portal.plan.as_deref() {
        copy_of_plan(plan)?
    } else {
        // No cached plan (rare — COPY plans at Parse): re-parse the query.
        if !leads_with_copy(&portal.query) {
            return None;
        }
        let schema = conn.shared.lock_schema();
        let plan = pglet_sql::parser::parse_single(&portal.query, &schema)
            .ok()
            .flatten()?;
        copy_of_plan(&plan)?
    };
    let deadline = conn.writer_deadline();
    Some(dispatch_copy(conn, options, direction, true, deadline))
}

/// Dispatch a resolved COPY by direction.
///
/// **This installs the ambient session** the expression evaluator reads — the native
/// server's COPY site, enumerated with the others on
/// [`pglet_sql::catalog::SessionSettings::install_ambient`], which is the one home
/// for that list. COPY reaches no other site: [`try_copy_simple`] returns before
/// `execute::execute_query_core` and [`try_copy_extended`] before it too, so without
/// this a COPY that is a connection's first statement resolves a stored
/// `DEFAULT current_user` / `CHECK (… = current_user)` against whatever the *thread*
/// last installed — and COPY **writes**, so that is a wrong row rather than a wrong
/// label.
///
/// **Why here and not [`begin_copy_in`]: a maintenance decision, not a measured one.**
/// Only the FROM direction needs it — [`copy_to`] runs `get_table_or_err` →
/// `resolve_columns` → `scan` → `value_to_text_with_oid` and evaluates no `Expr` at
/// all, and a `CopyOptions` carries no query source, so no session expression is
/// reachable in the TO direction. Moving the line into `begin_copy_in` keeps all
/// three tests below green (measured), and since TO evaluates nothing there is no
/// observable that could tell the two placements apart. It sits at the one funnel
/// both interceptors pass through because one site is cheaper to keep correct than
/// two, not because the second direction demands it.
///
/// **Why not [`handle_copy_in_frame`]** is the half that *is* measured: the ingest is
/// the same statement — nothing else can run on the connection between
/// [`begin_copy_in`] and its `CopyDone`, and both run on the connection's own thread,
/// so the install made here is still current when the rows are written.
/// `ambient_session_tests` drives the whole `CopyData`/`CopyDone` cycle and asserts
/// the written row, which is what makes that inheritance evidence rather than an
/// argument.
fn dispatch_copy(
    conn: &mut Connection,
    options: CopyOptions,
    direction: CopyDirection,
    extended: bool,
    deadline: Option<Instant>,
) -> Vec<BackendMessage> {
    conn.settings.install_ambient();
    // The statement starts here for the statement clock, as it does in
    // `execute::execute_query_core` for every statement that is not a `COPY`; the
    // ingest's rows are stamped by it.
    pglet_sql::builtins::start_statement();
    // A `COPY` takes the block's snapshot, as any planned statement does
    // (`execute::run_one_locked`): the first fixes the block's isolation level, in either
    // direction, and a `COPY … FROM` that is the block's first statement is its first
    // write, whose snapshot is this moment's — read here, at the statement's start,
    // before the ingest waits for the writer.
    let conn_id = conn.session.process_id;
    if conn.transaction_status() != TransactionStatus::Idle {
        let first_write_at = (direction == CopyDirection::From
            && !conn.tx.snapshot_taken(conn_id)
            && conn.settings.block_uses_transaction_snapshot())
        .then(|| conn.shared.lock_storage().commit_stamp());
        conn.tx
            .note_snapshot(conn_id, &mut conn.settings, first_write_at);
    }
    match direction {
        CopyDirection::To => copy_to(conn, &options, extended, deadline),
        CopyDirection::From => begin_copy_in(conn, options, extended, deadline),
    }
}

/// Whether `sql` (ignoring leading whitespace) begins with the `COPY` keyword. A
/// cheap pre-filter so only COPY-shaped input pays for the real parse; a false
/// positive (e.g. `COPYING`) just costs one parse that returns a non-COPY plan.
fn leads_with_copy(sql: &str) -> bool {
    sql.trim_start()
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("COPY"))
}

/// Resolve a `COPY … TO STDOUT` and build its full message batch.
fn copy_to(
    conn: &mut Connection,
    options: &CopyOptions,
    extended: bool,
    deadline: Option<Instant>,
) -> Vec<BackendMessage> {
    let status = conn.transaction_status();
    match run_copy_to(conn, &CopyParams::from(options), deadline) {
        Ok((bytes, n_cols)) => copy_out_messages(&bytes, n_cols, options.header, status, extended),
        Err(e) => copy_failure(conn, &e, extended),
    }
}

/// Run `execute_copy_to` against the right storage view, returning the encoded COPY
/// bytes and the transferred-column count (for the `CopyOutResponse` header).
///
/// Autocommit over B+tree reads from a lock-free MVCC snapshot (matching the normal
/// read path); the memory backend and an in-transaction read take the locks and read
/// through the connection-aware adapter (read-your-own-writes within a txn).
///
/// The table and its column list are resolved first, under the rule a plan is
/// ([`resolve_copy_target`]): a resolution that fails because of another connection's
/// uncommitted catalog change waits for that hold to end, holding no guard, and resolves
/// again.
fn run_copy_to(
    conn: &Connection,
    params: &CopyParams<'_>,
    deadline: Option<Instant>,
) -> Result<(Vec<u8>, usize), Error> {
    let conn_id = conn.session.process_id;
    loop {
        if conn.transaction_status() == TransactionStatus::Idle
            && let Some(snapshot) = conn.shared.read_snapshot()
        {
            let schema = conn.shared.lock_schema();
            conn.catalog_depth.check(&schema)?;
            let Some(n_cols) = resolve_copy_target(conn, &schema, params)? else {
                drop(schema);
                drop(snapshot);
                conn.wait_for_foreign_writer(deadline)?;
                continue;
            };
            let adapter = SnapshotAdapter::new(&snapshot);
            let bytes = execute_copy_to(&adapter, &schema, params)?;
            return Ok((bytes, n_cols));
        }
        let (schema, mut storage) = conn.shared.lock_both();
        conn.catalog_depth.check(&schema)?;
        let Some(n_cols) = resolve_copy_target(conn, &schema, params)? else {
            drop((schema, storage));
            conn.wait_for_foreign_writer(deadline)?;
            continue;
        };
        let adapter = StorageAdapter::new(&mut storage, conn_id);
        let bytes = execute_copy_to(&adapter, &schema, params)?;
        return Ok((bytes, n_cols));
    }
}

/// Resolve a `COPY`'s table and column list against `schema` — the catalog resolution
/// its plan leaves to execution — to the transferred-column count, or `None` where the
/// resolution failed because of another connection's uncommitted catalog change
/// ([`Connection::plan_failure_is_the_writers`]): a column list naming a column that
/// connection's block dropped. Then the caller waits for that hold to end and resolves
/// again, as `execute::run_one_locked` waits for a plan; 18.6's `COPY` opens its table
/// under a lock the block's DDL holds before it resolves the list. A failure of the
/// statement's own — a column no catalog has — is its answer at once.
fn resolve_copy_target(
    conn: &Connection,
    schema: &Schema,
    params: &CopyParams<'_>,
) -> Result<Option<usize>, Error> {
    match copy_column_count(schema, params) {
        Ok(n_cols) => Ok(Some(n_cols)),
        Err(e)
            if conn.plan_failure_is_the_writers(&e, schema.generation, |committed| {
                copy_column_count(committed, params)
            }) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Build the `COPY … TO STDOUT` message batch from the encoded bytes: the
/// `CopyOutResponse` header, one `CopyData` per newline-terminated line, then
/// `CopyDone` + `CommandComplete` (+ `ReadyForQuery` when simple). The command tag
/// counts data rows, so a leading `HEADER` line is excluded.
fn copy_out_messages(
    bytes: &[u8],
    n_cols: usize,
    header: bool,
    status: TransactionStatus,
    extended: bool,
) -> Vec<BackendMessage> {
    let mut messages = vec![BackendMessage::CopyOutResponse {
        overall_format: 0,
        column_formats: vec![0; n_cols],
    }];
    // One CopyData per line. The encoder newline-terminates every line (header and
    // data) and escapes any field newline, so each raw `\n` is a true line end.
    let mut lines: u64 = 0;
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            messages.push(BackendMessage::CopyData(bytes[start..=i].to_vec()));
            start = i + 1;
            lines += 1;
        }
    }
    // Defensive: a trailing line with no newline is unreachable (every line is
    // terminated), but forward it so no byte is dropped.
    if start < bytes.len() {
        messages.push(BackendMessage::CopyData(bytes[start..].to_vec()));
    }
    let rows = lines.saturating_sub(u64::from(header));
    messages.push(BackendMessage::CopyDone);
    messages.push(BackendMessage::CommandComplete {
        tag: format!("COPY {rows}"),
    });
    if !extended {
        messages.push(BackendMessage::ReadyForQuery { status });
    }
    messages
}

/// Start a `COPY … FROM STDIN`: validate the table/columns (so a bad target — a missing
/// table, a missing column, a repeated one — errors here, before `CopyInResponse`, as
/// Postgres does; [`copy_column_count`] carries the codes), enter copy-in mode, and return the
/// `CopyInResponse` header. A resolution that fails because of another connection's
/// uncommitted catalog change waits for that hold to end and resolves again
/// ([`resolve_copy_target`]).
fn begin_copy_in(
    conn: &mut Connection,
    options: CopyOptions,
    extended: bool,
    deadline: Option<Instant>,
) -> Vec<BackendMessage> {
    // Resolve the column count under the schema lock, then drop the guard before any
    // `conn`-mutating error path (`copy_failure` borrows `conn` mutably).
    let params = CopyParams::from(&options);
    let n_cols = loop {
        let resolved = {
            let schema = conn.shared.lock_schema();
            conn.catalog_depth
                .check(&schema)
                .and_then(|()| resolve_copy_target(conn, &schema, &params))
        };
        match resolved {
            Ok(Some(n_cols)) => break Ok(n_cols),
            Ok(None) => {
                if let Err(waited) = conn.wait_for_foreign_writer(deadline) {
                    break Err(waited);
                }
            }
            Err(e) => break Err(e),
        }
    };
    let n_cols = match n_cols {
        Ok(n) => n,
        Err(e) => return copy_failure(conn, &e, extended),
    };
    conn.copy_in = Some(CopyInState {
        options,
        extended,
        buffer: Vec::new(),
    });
    vec![BackendMessage::CopyInResponse {
        overall_format: 0,
        column_formats: vec![0; n_cols],
    }]
}

/// Route one inbound frame while the connection is in `COPY … FROM STDIN` mode.
///
/// `CopyData` appends to the buffer (no response); `CopyDone` ingests the buffer and
/// resynchronizes; `CopyFail` aborts; `Terminate` ends the session; any other frame
/// is a protocol violation that aborts the COPY and resynchronizes loudly.
pub fn handle_copy_in_frame(conn: &mut Connection, msg: FrontendMessage) -> Vec<BackendMessage> {
    match msg {
        FrontendMessage::CopyData(bytes) => {
            if let Some(state) = conn.copy_in.as_mut() {
                state.buffer.extend_from_slice(&bytes);
            }
            Vec::new()
        }
        FrontendMessage::CopyDone => finish_copy_in(conn),
        // A `Flush`/`Sync` mid-COPY is ignored, matching Postgres (`CopyGetData`:
        // "Ignore Flush/Sync for the convenience of client libraries ... that may
        // send those without noticing that the command they just sent was COPY").
        // tokio-postgres pipelines `Bind, Execute, Sync` to start a copy-in, so this
        // Sync lands here before the data and must be a no-op — not a protocol
        // violation. The trailing `Sync` after `CopyDone` arrives once copy-in mode
        // has cleared, so it resynchronizes normally.
        FrontendMessage::Flush | FrontendMessage::Sync => Vec::new(),
        FrontendMessage::CopyFail(message) => {
            // 57014 query_canceled: the client aborted the in-progress COPY.
            copy_in_abort(conn, "57014", &format!("COPY from stdin failed: {message}"))
        }
        FrontendMessage::Terminate => {
            conn.copy_in = None;
            conn.session.state = SessionState::Terminated;
            Vec::new()
        }
        _ => {
            // A non-copy frame mid-COPY is a protocol violation: abort the COPY,
            // discard the partial buffer, and resynchronize with a loud error.
            copy_in_abort(conn, "08P01", "unexpected message during COPY from stdin")
        }
    }
}

/// Ingest the accumulated `COPY … FROM STDIN` buffer (on `CopyDone`): take the writer,
/// run `execute_copy_from`, persist an autocommit write, and resynchronize.
///
/// A door that stages writes outside `execute::run_one`, so it takes **the writer**
/// itself, before the ingest stages a row — waiting with no guard held, and re-basing a
/// block that takes it here — and settles it after, as `run_one` does
/// (`Connection::settle_writer`). A column default's `nextval` draws inside the ingest,
/// so without it a `COPY` drew from a sequence another connection's block was writing.
/// The table is resolved again by the ingest itself, so a catalog that moved while it
/// waited is read as it now stands.
fn finish_copy_in(conn: &mut Connection) -> Vec<BackendMessage> {
    let Some(state) = conn.copy_in.take() else {
        return Vec::new();
    };
    let outcome = ingest_copy_in(conn, &state);
    conn.settle_writer();
    match outcome {
        Ok(n) => {
            let mut messages = vec![BackendMessage::CommandComplete {
                tag: format!("COPY {n}"),
            }];
            if !state.extended {
                messages.push(BackendMessage::ReadyForQuery {
                    status: conn.transaction_status(),
                });
            }
            messages
        }
        Err(e) => copy_failure(conn, &e, state.extended),
    }
}

/// [`finish_copy_in`]'s ingest, from the writer to the durable flip.
fn ingest_copy_in(conn: &mut Connection, state: &CopyInState) -> Result<u64, Error> {
    let conn_id = conn.session.process_id;
    let params = CopyParams::from(&state.options);
    if !conn.holds_writer() {
        conn.acquire_writer(conn.writer_deadline())?;
    }
    {
        let (mut schema, mut storage) = conn.shared.lock_both();
        if conn.transaction_status() != TransactionStatus::Idle && !conn.tx.holds_writer() {
            // As `execute::run_one_locked`: the block re-bases, or — a `REPEATABLE READ`
            // block past the snapshot `dispatch_copy` noted — is refused `40001` and gives
            // the writer back. An auto-commit ingest holds the writer for itself.
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
        conn.note_writer_grant(&schema);
        // Wrap an auto-commit COPY ingest in an implicit storage transaction so a bad
        // line partway through the stream rolls back the already-ingested lines
        // atomically (Postgres COPY is all-or-nothing) instead of leaving them applied.
        // With a transaction active the ingest stages every row in the pending buffer
        // (via the adapter's `insert_in_tx`) — intra-batch uniqueness still catches a
        // duplicate — and publishes in one CoW root swap at commit. Inside an explicit
        // transaction the ingest stays buffered and the COMMIT persists it.
        let autocommit = !storage.has_transaction(conn_id);
        // The statement's entry check, at the point the ingest runs
        // (`Connection::catalog_depth`).
        let begun = conn.catalog_depth.check(&schema).and_then(|()| {
            if autocommit {
                storage.begin(conn_id)
            } else {
                Ok(())
            }
        });
        begun.and_then(|()| {
            let count = {
                let mut adapter = StorageAdapter::new(&mut storage, conn_id);
                execute_copy_from(&mut adapter, &mut schema, &params, &state.buffer)
            };
            match count {
                // Publish the staged rows atomically, then the durable meta flip.
                // A `COPY … FROM` writes no catalog fact but the sequence counters a
                // column default advances, which no rollback puts back — so a refused
                // write can drop its rows.
                Ok(n) if autocommit => {
                    Connection::publish_autocommit(conn_id, schema, &mut storage, true).map(|()| n)
                }
                // Inside an explicit transaction: stays buffered, the COMMIT persists.
                Ok(n) => Ok(n),
                // Discard the staged rows; surface the original error.
                Err(e) => {
                    if autocommit {
                        let _ = storage.rollback(conn_id);
                    }
                    Err(e)
                }
            }
        })
    }
}

/// Abort an in-progress copy-in with a COPY-specific SQLSTATE: clear the mode,
/// discard the partial buffer, fail the transaction, and resynchronize.
fn copy_in_abort(conn: &mut Connection, code: &str, message: &str) -> Vec<BackendMessage> {
    let extended = conn.copy_in.as_ref().is_some_and(|s| s.extended);
    conn.copy_in = None;
    conn.fail_transaction();
    if extended {
        // Extended: no `ReadyForQuery` (the client's `Sync` resynchronizes). No
        // skip-until-Sync is armed: `handle_message` returns a copy-in frame's reply
        // before its arming check, so a message pipelined after this failure is still
        // processed.
        error_with_code(code, message)
    } else {
        let mut messages = error_with_code(code, message);
        messages.push(BackendMessage::ReadyForQuery {
            status: conn.transaction_status(),
        });
        messages
    }
}

/// Format a COPY engine error for the active protocol, failing the transaction first
/// (so the `ReadyForQuery` status is correct on the simple path).
fn copy_failure(conn: &mut Connection, e: &Error, extended: bool) -> Vec<BackendMessage> {
    conn.fail_transaction();
    if extended {
        extended_error(e)
    } else {
        error_response(e, conn.transaction_status())
    }
}

#[cfg(test)]
mod ambient_session_tests {
    //! The **COPY** routes' half of the ambient-session install (the native COPY site
    //! of the four `SessionSettings::install_ambient`'s doc enumerates).
    //!
    //! The install is per statement, and COPY is the family of statements that
    //! reaches no other site: `try_copy_simple` returns before `execute_query_core`,
    //! `try_copy_extended` likewise, and the `CopyData`/`CopyDone` ingest is routed by
    //! `handle_copy_in_frame` — so a COPY
    //! that is a connection's **first** statement used to evaluate a stored
    //! `DEFAULT current_user` / `CHECK (… = current_user)` against whatever the
    //! *thread's* last-installed session was. It **writes**, which is what makes this
    //! the one COPY-shaped hole worth its own module.
    //!
    //! Both properties are load-bearing and neither alone reproduces it: a **fresh**
    //! session (nothing of its own installed yet) **and** COPY as its first statement.
    //! `test_copy`'s differential harness cannot see it — every fixture there creates
    //! its table on the same connection first, which installs — and neither can
    //! `execute::ambient_session_tests`, which never enters copy-in mode. The two
    //! connections here share one `SharedState` and run on one thread, which is the
    //! sharper form of the production shape (thread-per-connection): the writer's
    //! session must win over the *creator's*, not merely over the default.

    use std::sync::Arc;

    use pglet_sql::catalog::SessionSettings;
    use pglet_wire::{BackendMessage, FrontendMessage};

    use crate::connection::{Connection, SharedState};
    use crate::handlers::handle_message;

    /// A connection over `shared`, identified as `user`.
    fn conn_as(shared: &Arc<SharedState>, user: &str) -> Connection {
        let mut c = Connection::new(1, Arc::clone(shared));
        c.settings =
            SessionSettings::new(String::new()).with_identity(user.into(), "wonderland".into());
        c
    }

    fn query(c: &mut Connection, sql: &str) -> Vec<BackendMessage> {
        handle_message(c, FrontendMessage::Query { sql: sql.into() })
    }

    /// Every `DataRow`'s cells as text.
    fn rows(msgs: &[BackendMessage]) -> Vec<Vec<String>> {
        msgs.iter()
            .filter_map(|m| match m {
                BackendMessage::DataRow { values } => Some(
                    values
                        .iter()
                        .map(|v| {
                            v.as_ref().map_or_else(String::new, |b| {
                                String::from_utf8_lossy(b).into_owned()
                            })
                        })
                        .collect(),
                ),
                _ => None,
            })
            .collect()
    }

    /// Feed one line and finish the copy-in.
    fn copy_line(c: &mut Connection, line: &[u8]) -> Vec<BackendMessage> {
        handle_message(c, FrontendMessage::CopyData(line.to_vec()));
        handle_message(c, FrontendMessage::CopyDone)
    }

    #[test]
    fn a_simple_copy_as_a_connections_first_statement_writes_its_own_session() {
        let shared = Arc::new(SharedState::with_btree());
        let mut creator = conn_as(&shared, "bob");
        query(
            &mut creator,
            "CREATE TABLE cpt (i INT, u TEXT DEFAULT current_user)",
        );

        // A *different*, fresh connection whose first statement is the COPY.
        let mut writer = conn_as(&shared, "alice");
        let start = query(&mut writer, "COPY cpt (i) FROM STDIN");
        assert!(
            matches!(start.as_slice(), [BackendMessage::CopyInResponse { .. }]),
            "{start:?}"
        );
        copy_line(&mut writer, b"1\n");

        assert_eq!(
            rows(&query(&mut creator, "SELECT i, u FROM cpt")),
            vec![vec!["1".to_string(), "alice".to_string()]],
            "the COPY's own session must resolve the stored DEFAULT"
        );
    }

    #[test]
    fn an_extended_copy_as_a_connections_first_statement_writes_its_own_session() {
        let shared = Arc::new(SharedState::with_btree());
        let mut creator = conn_as(&shared, "bob");
        query(
            &mut creator,
            "CREATE TABLE cpx (i INT, u TEXT DEFAULT current_user)",
        );

        let mut writer = conn_as(&shared, "alice");
        handle_message(
            &mut writer,
            FrontendMessage::Parse {
                name: String::new(),
                query: "COPY cpx (i) FROM STDIN".into(),
                param_types: Vec::new(),
            },
        );
        handle_message(
            &mut writer,
            FrontendMessage::Bind {
                portal: String::new(),
                statement: String::new(),
                param_formats: Vec::new(),
                params: Vec::new(),
                result_formats: Vec::new(),
            },
        );
        let start = handle_message(
            &mut writer,
            FrontendMessage::Execute {
                portal: String::new(),
                max_rows: 0,
            },
        );
        assert!(
            start
                .iter()
                .any(|m| matches!(m, BackendMessage::CopyInResponse { .. })),
            "{start:?}"
        );
        copy_line(&mut writer, b"1\n");

        assert_eq!(
            rows(&query(&mut creator, "SELECT i, u FROM cpx")),
            vec![vec!["1".to_string(), "alice".to_string()]]
        );
    }

    /// The loud half of the same hole: a `CHECK` over the session role **refuses** a
    /// copy-first row that is in fact correct, which is a rejected write rather than
    /// a mislabelled one.
    #[test]
    fn a_copy_first_check_over_the_session_role_admits_its_own_rows() {
        let shared = Arc::new(SharedState::with_btree());
        let mut creator = conn_as(&shared, "bob");
        query(
            &mut creator,
            "CREATE TABLE cck (i INT, o TEXT CHECK (o = current_user))",
        );

        let mut writer = conn_as(&shared, "alice");
        query(&mut writer, "COPY cck FROM STDIN");
        let done = copy_line(&mut writer, b"1\talice\n");
        assert!(
            !done
                .iter()
                .any(|m| matches!(m, BackendMessage::ErrorResponse { .. })),
            "the writer's own role must satisfy the CHECK: {done:?}"
        );
        assert_eq!(
            rows(&query(&mut creator, "SELECT i, o FROM cck")),
            vec![vec!["1".to_string(), "alice".to_string()]]
        );
    }
}

#[cfg(test)]
mod tests {
    //! In-process drives of the **simple-query** COPY state machine through the real
    //! router (`handle_message`) — no sockets. The wire-level byte conformance is
    //! covered by the `test_copy` differential harness; these pin the connection
    //! state machine (copy-in mode entry/exit, buffering, error recovery) directly.

    use pglet_wire::{BackendMessage, FrontendMessage};

    use crate::connection::Connection;
    use crate::handlers::handle_message;
    use crate::test_support::{conn, has_error};

    fn query(c: &mut Connection, sql: &str) -> Vec<BackendMessage> {
        handle_message(c, FrontendMessage::Query { sql: sql.into() })
    }

    fn command_tag(msgs: &[BackendMessage]) -> Option<&str> {
        msgs.iter().find_map(|m| match m {
            BackendMessage::CommandComplete { tag } => Some(tag.as_str()),
            _ => None,
        })
    }

    #[test]
    fn a_trailing_comment_leaves_a_simple_copy_the_sub_protocol() {
        // `COPY t TO STDOUT; -- c` is one statement to 18.6's raw parse, which runs the
        // sub-protocol and answers `COPY 1` (measured). Counting the comment as a second
        // statement fell back to the planner, whose `COPY … TO STDOUT` is `0A000`.
        let mut c = conn();
        query(&mut c, "CREATE TABLE ct (a int)");
        query(&mut c, "INSERT INTO ct VALUES (1)");
        for sql in ["COPY ct TO STDOUT; -- c", "COPY ct TO STDOUT; /* c */"] {
            let out = query(&mut c, sql);
            assert!(
                matches!(out.first(), Some(BackendMessage::CopyOutResponse { .. })),
                "{sql:?}: {out:?}"
            );
            assert_eq!(command_tag(&out), Some("COPY 1"), "{sql:?}: {out:?}");
            assert!(!has_error(&out), "{sql:?}: {out:?}");
        }
    }

    #[test]
    fn simple_copy_from_then_to_round_trips() {
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (a int, b text)");

        // COPY FROM STDIN enters copy-in mode: a lone CopyInResponse, no RFQ.
        let start = query(&mut c, "COPY t FROM STDIN");
        assert!(
            matches!(start.as_slice(), [BackendMessage::CopyInResponse { .. }]),
            "COPY FROM should return just CopyInResponse: {start:?}"
        );
        assert!(c.copy_in.is_some(), "copy-in mode must be active");

        // CopyData accumulates with no response.
        assert!(
            handle_message(&mut c, FrontendMessage::CopyData(b"1\thello\n".to_vec())).is_empty()
        );
        assert!(
            handle_message(&mut c, FrontendMessage::CopyData(b"2\tworld\n".to_vec())).is_empty()
        );

        // CopyDone ingests the buffer and resynchronizes (simple → trailing RFQ).
        let done = handle_message(&mut c, FrontendMessage::CopyDone);
        assert_eq!(command_tag(&done), Some("COPY 2"), "{done:?}");
        assert!(
            done.iter()
                .any(|m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            "simple-protocol CopyDone must append ReadyForQuery: {done:?}"
        );
        assert!(c.copy_in.is_none(), "copy-in mode must clear at CopyDone");

        // COPY TO STDOUT emits the rows back: CopyOutResponse + 2 CopyData + CopyDone
        // + CommandComplete + ReadyForQuery.
        let out = query(&mut c, "COPY t TO STDOUT");
        assert!(matches!(
            out.first(),
            Some(BackendMessage::CopyOutResponse { .. })
        ));
        let data: Vec<&Vec<u8>> = out
            .iter()
            .filter_map(|m| match m {
                BackendMessage::CopyData(b) => Some(b),
                _ => None,
            })
            .collect();
        assert_eq!(data.len(), 2, "one CopyData per row: {out:?}");
        assert_eq!(data[0].as_slice(), b"1\thello\n");
        assert_eq!(data[1].as_slice(), b"2\tworld\n");
        assert_eq!(command_tag(&out), Some("COPY 2"));
    }

    #[test]
    fn copy_from_bad_value_fails_loud_at_copy_done() {
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (a int)");
        let start = query(&mut c, "COPY t FROM STDIN");
        assert!(matches!(
            start.as_slice(),
            [BackendMessage::CopyInResponse { .. }]
        ));

        // A non-integer value is buffered without complaint…
        assert!(
            handle_message(&mut c, FrontendMessage::CopyData(b"notanint\n".to_vec())).is_empty()
        );
        // …then fails loud at CopyDone (22P02), and copy-in mode clears.
        let done = handle_message(&mut c, FrontendMessage::CopyDone);
        assert!(has_error(&done), "bad int must error at CopyDone: {done:?}");
        assert!(
            c.copy_in.is_none(),
            "a failed ingest must still clear copy-in mode"
        );
    }

    #[test]
    fn copy_from_missing_table_errors_before_copy_in() {
        // A COPY into a nonexistent table errors immediately (no CopyInResponse, no
        // copy-in mode) — matching Postgres.
        let mut c = conn();
        let r = query(&mut c, "COPY nope FROM STDIN");
        assert!(has_error(&r), "missing table must error: {r:?}");
        assert!(
            !r.iter()
                .any(|m| matches!(m, BackendMessage::CopyInResponse { .. })),
            "no CopyInResponse for a missing table: {r:?}"
        );
        assert!(c.copy_in.is_none(), "no copy-in mode on a failed start");
    }

    #[test]
    fn copy_fail_aborts_and_resynchronizes() {
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (a int)");
        query(&mut c, "COPY t FROM STDIN");
        handle_message(&mut c, FrontendMessage::CopyData(b"1\n".to_vec()));

        let aborted = handle_message(&mut c, FrontendMessage::CopyFail("client gave up".into()));
        assert!(
            has_error(&aborted),
            "CopyFail must surface an error: {aborted:?}"
        );
        assert!(
            aborted
                .iter()
                .any(|m| matches!(m, BackendMessage::ReadyForQuery { .. })),
            "simple-protocol CopyFail must resynchronize: {aborted:?}"
        );
        assert!(c.copy_in.is_none(), "CopyFail clears copy-in mode");

        // The aborted COPY left no rows.
        let count = query(&mut c, "COPY t TO STDOUT");
        assert!(
            !count
                .iter()
                .any(|m| matches!(m, BackendMessage::CopyData(_))),
            "an aborted COPY must insert nothing: {count:?}"
        );
    }
}
