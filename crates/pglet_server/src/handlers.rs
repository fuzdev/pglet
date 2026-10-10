//! Frontend-message router and prepared-statement lifecycle.
//!
//! `handle_message` dispatches each protocol message; the Parse/Bind/Describe/
//! Close/Execute/Sync/Flush lifecycle lives here. Query execution itself is in
//! `execute` (this module routes Query / Execute into it).

use std::sync::Arc;

use pglet::is_transaction_exit_stmt;
use pglet_sql::catalog::{
    DEFAULT_SESSION_NAME, Schema, SessionSettings, is_deallocate_command, is_discard_command,
    is_reset_command, is_session_command, parse_deallocate, parse_discard, parse_reset,
    show_result_shape, startup_parameters,
};
use pglet_sql::executor::ResultShape;
use pglet_sql::planner::{Plan, bound_param_enum_legality, infer_param_targets};
use pglet_sql::types::TypeOid;
use pglet_wire::{BackendMessage, DescribeKind, FrontendMessage, SessionState, TransactionStatus};

use crate::connection::{Connection, Portal, PreparedStatement};
use crate::encoding::{
    decode_param, error_with_code, extended_error, fatal_with_code, format_code_at,
};
use crate::execute::{Protocol, execute_query_core, handle_query};

/// The result shape for a Describe reply: does this statement return a result set,
/// and with which columns.
///
/// Reuses the cached plan when its generation still matches the live schema (no
/// re-parse/re-plan); a generation mismatch (a DDL since Parse) or an absent cache
/// falls back to re-planning the SQL. A re-plan that *fails* yields no result set.
/// Swallowing it is right here and wrong at Parse: this fallback exists to refresh
/// a plan a DDL invalidated, and a statement that fails to re-plan still reports
/// its error — loud, and with the right SQLSTATE — when Execute runs it.
///
/// **What no path here does is revalidate a prepared statement's result type
/// against the DDL that changed it**, and that divergence is `EDGES.md` row 62 —
/// read the measurements, the class bounds and the controls there rather than from
/// this comment, which is its cross-reference home and not a second derivation. The
/// two kinds in short, both over `k (a INT, c INT)` with `ALTER TABLE k DROP COLUMN
/// a` between the Parse and the re-Describe. A re-plan that **fails** is loud but
/// late: `SELECT a FROM k` answers `NoData` here and raises `42703` at Execute,
/// where 18.6 raises that same `42703` at the Describe. A re-plan that **succeeds
/// against a changed shape** is silent: `SELECT *` re-describes as the new
/// one-column relation and answers its rows, where 18.6 raises `0A000 cached plan
/// must not change result type` — and that half is not confined to the Describe
/// path, since a client that Binds and Executes without re-Describing is sent rows
/// one field wide against the two-field descriptor it still holds. Neither kind is
/// bounded by its wire shape: run the DDL inside a transaction block and 18.6's
/// refusal **aborts** the block, so its COMMIT reports ROLLBACK and discards the DDL
/// and every statement after it, while the block here stays live and commits them —
/// measured on both kinds; on the loud kind it takes a route that stops short of
/// Execute (a re-Describe, a bare Bind, and a Bind + portal Describe each leave the
/// block live here), because nothing before Execute raises it here and pglet's own
/// 42703 there aborts this block too. Row 62's table is the record.
///
/// Three branches below answer [`ResultShape::NoResultSet`], and `row_description`
/// turns each into `NoData`: a plan that returns no result set (DDL, transaction
/// control, a DML with no `RETURNING`), a `parse_single` that returns `Err` (a failed
/// re-plan **and** the statements [`runs_outside_the_planner`] keeps from the planner,
/// `SHOW` apart, arrive through this one), and a `parse_single` that yields no statement
/// at all. `SHOW` is answered ahead of all three, by `catalog::show_result_shape`.
/// Which SQL fills each is enumerated in `EDGES.md` row 29.
///
/// **A plan that returns a result set of zero columns is not one of them.** `SELECT *`
/// over a relation whose last column was dropped is described `RowDescription`
/// (`fields=0`), as 18.6 describes it; that is `EDGES.md` row 59, and it is why this
/// returns a [`ResultShape`] rather than a column list the callers would have to read
/// `is_empty()` on.
fn describe_result_shape(
    query: &str,
    cached: Option<&Arc<Plan>>,
    plan_gen: u64,
    declared_param_types: &[Option<TypeOid>],
    schema: &Schema,
) -> ResultShape {
    // `SHOW` is the one statement `runs_outside_the_planner` keeps from the planner
    // that returns a result set, so it is the one whose shape the plan side cannot
    // produce. Left to the fallback below it answered `NoData` and then sent a
    // `DataRow` no descriptor covered — a binary client read the row as having no
    // columns and dropped the value. It also feeds the aborted-block Describe gate,
    // which keys on `is_result_set()`: with the shape missing, a `SHOW` prepared
    // while the block was healthy was described rather than refused in a failed one.
    if let Some(shape) = show_result_shape(query) {
        return shape;
    }
    if let Some(p) = cached
        && plan_gen == schema.generation
    {
        return p.result_shape(schema);
    }
    pglet_sql::parser::parse_single_with_param_types(query, schema, declared_param_types)
        .ok()
        .flatten()
        .map_or(ResultShape::NoResultSet, |p| p.result_shape(schema))
}

/// Whether `sql` executes through a path that never plans it — so the parser's
/// rejection of it is not the statement's error and Parse must not raise it.
///
/// `execute_query_core` intercepts the five session-hygiene statements **ahead of**
/// the parser (per statement after `split_statements`), through
/// `catalog::execute_session_command`. `SET` and `SHOW` are serviced from the
/// catalog, and the parser itself rejects them with `0A000` "not yet supported in
/// the native parser"; `DEALLOCATE`, `DISCARD` and `RESET` are serviced from
/// `conn.prepared` / `conn.sequence_session` and the parser answers `42601`, since
/// none of the three is a lexer keyword at all — which is also what keeps
/// `SELECT deallocate FROM t` working, as it does on 18.6 where all three words are
/// UNRESERVED. They are the entire exception list: every other statement the
/// extended protocol reaches is executed from a plan, so a plan error is that
/// statement's error. Because this predicate reads only the *first* statement's
/// keyword, the caller still owes the single-statement check that `parse_single`
/// would otherwise have made.
///
/// Their **Describe** half is modeled for the one member that has one:
/// [`describe_result_shape`] asks `catalog::show_result_shape` before falling back
/// to the plan side, so `Describe` of `SHOW server_version` answers
/// `RowDescription(1)` as Postgres does. The other four genuinely return no result
/// set, which the fallback already answers. Before that, having no plan, every
/// member reported no result set and a `SHOW`'s `Execute` sent a `DataRow` no field
/// descriptor covered — a binary client read the row as having **no columns** and
/// dropped the value, which is what the extended protocol's clients (node-postgres,
/// the fixture harness's binary leg) all are.
///
/// One member's **Parse-phase** half is also missing, measured over `client.prepare()`
/// (Parse + Describe + Sync, so the phase is unambiguous): `SHOW bogus_setting` is
/// `42704` at `Parse` on 18.6 — parse *analysis*, so `25P02` still outranks it in a
/// failed block — and `ParseComplete` here, with the error landing at `Execute`.
/// `DEALLOCATE`'s syntax errors take the opposite position (the grammar's, ahead of
/// the abort gate) and are checked above; `SET` has no Parse-time error to miss.
///
/// It reaches one gate too: the aborted-block Describe check keys on *does this
/// statement return data* (`psrc->resultDesc`), read here as
/// [`ResultShape::is_result_set`]. That gate was always right and its input was the
/// missing half — a `SHOW` prepared while the block was healthy was described rather
/// than refused in a failed one, where 18.6 refuses it. Feeding the gate the real
/// shape closed that too.
fn runs_outside_the_planner(sql: &str) -> bool {
    is_session_command(sql)
}

/// What Parse's plan attempt produced, in the three shapes the aborted-transaction
/// gate has to tell apart.
enum Parsed {
    /// A statement — with its cached plan, or `None` for the ones serviced outside
    /// the planner ([`runs_outside_the_planner`]).
    Statement {
        param_types: Vec<Option<TypeOid>>,
        /// Each `$N`'s **enum target**, as `infer_param_targets` settled it — empty
        /// for a statement serviced outside the planner, which takes no `$N` at all.
        param_enums: Vec<Option<String>>,
        plan: Option<Arc<Plan>>,
    },
    /// No statement at all: empty text, a lone `;`, comment-only. Postgres's
    /// `parsetree_list == NIL`, which skips its whole per-statement block — the
    /// aborted-transaction check included, so these still `ParseComplete` in a
    /// failed block.
    Empty { param_types: Vec<Option<TypeOid>> },
    /// The statement failed to parse/plan.
    Failed(pglet_sql::Error),
}

/// The Describe reply for a statement's result shape: `NoData` when it returns **no
/// result set**, else a `RowDescription` — including an empty one, `fields=0`, for a
/// result set of zero columns.
///
/// The criterion is result-set-ness and never the column count: 18.6 answers
/// `RowDescription(0)` for `SELECT *` over a relation whose last column was dropped
/// and `NoData` for a `RETURNING`-less `INSERT`, both measured, and reading
/// `columns.is_empty()` here collapsed the two (`EDGES.md` row 59).
fn row_description(shape: &ResultShape) -> BackendMessage {
    match shape {
        ResultShape::NoResultSet => BackendMessage::NoData,
        ResultShape::Columns(columns) => BackendMessage::RowDescription {
            columns: columns
                .iter()
                .map(|c| {
                    pglet_wire::FieldDescription::new(
                        &c.name,
                        c.type_oid,
                        c.type_modifier,
                        c.table_oid,
                        c.column_id,
                    )
                })
                .collect(),
        },
    }
}

/// Handle a frontend message and return response messages.
pub fn handle_message(conn: &mut Connection, msg: FrontendMessage) -> Vec<BackendMessage> {
    // After a failed durable write, nothing is served from the failed state on any
    // connection: its in-memory state may hold a commit the durable state does not.
    // 18.6's answer to a failed WAL write or flush is a `PANIC` — every other backend is
    // told `57P02` and ended, a new connection is refused `57P03` until crash recovery
    // finishes, and then it accepts again. Here the recovery is the server's: the next
    // connection it accepts is served from the store reopened from its durable slot
    // (`server::Server::connection_state`), so a client reconnects. A startup still meets
    // this only when it was accepted before the failure, or the reopen failed. Checked
    // first, so a copy-in or a skip-until-`Sync` window ends with the connection too.
    if let Some(reason) = conn.shared.write_failure_reason()
        && !matches!(msg, FrontendMessage::Terminate)
    {
        let startup = matches!(msg, FrontendMessage::Startup { .. });
        conn.session.state = SessionState::Terminated;
        return if startup {
            fatal_with_code(
                "57P03",
                &format!(
                    "the database system is in recovery mode: a durable write failed ({reason}); reconnect"
                ),
            )
        } else {
            fatal_with_code(
                "57P02",
                &format!(
                    "terminating connection because a durable write failed ({reason}); reconnect to be served from the durable state"
                ),
            )
        };
    }

    // COPY FROM STDIN sub-protocol: while a copy-in is active every inbound frame is
    // routed to the copy-in handler (append `CopyData`, ingest on `CopyDone`, abort
    // on `CopyFail`) until the stream ends. Checked first — before the
    // skip-until-Sync window — because copy-in is a distinct, simple-query mode that
    // owns the connection until it resolves. Mirrors the `skip_until_sync` flag.
    if conn.copy_in.is_some() {
        return crate::copy::handle_copy_in_frame(conn, msg);
    }

    // Extended-protocol error recovery (Postgres semantics): once any extended-query
    // message has errored, discard every following message until the next `Sync`,
    // which re-synchronizes the client with a single `ReadyForQuery`. Without this a
    // pipelined `Parse`/`Bind`/`Execute`/`Sync` whose `Bind` fails would emit a
    // cascade — the `Bind` error, then an `Execute` "portal does not exist" error —
    // where real Postgres emits one error and skips to `Sync`. `Sync` and
    // `Terminate` are always processed (Sync clears the state below).
    if conn.skip_until_sync && !matches!(msg, FrontendMessage::Sync | FrontendMessage::Terminate) {
        return Vec::new();
    }

    let responses = dispatch(conn, msg);

    // Arm skip-until-Sync on an *extended-protocol* error: an `ErrorResponse` with
    // no accompanying `ReadyForQuery`. The simple-query path always appends its own
    // `ReadyForQuery` (it self-recovers), so it never arms this; the extended path
    // defers `ReadyForQuery` to `Sync`, so its errors do. Checked here (not in each
    // arm) so it catches the extended-protocol error any `dispatch` arm returns, including
    // the many that
    // early-`return`. A frame the copy-in handler takes is not among them: it returns
    // above, ahead of this check, so a failure raised while a copy-in is active arms no
    // skip (a `COPY` that fails at its `Execute` is a `dispatch` arm's error and does).
    //
    // The same error **fails an open transaction block**, whichever phase raised it —
    // `Parse`, `Bind`, `Describe` or `Execute`. 18.6 aborts the transaction from its
    // one error-recovery path rather than per message, so a `Bind` that refuses a
    // parameter, or a `Describe` naming a statement that does not exist, leaves the
    // block `[E]` exactly as a failed `Execute` does (measured for each of those four
    // phases). Some raisers fail the block where they raise — `execute.rs`'s
    // `format_error`, and the copy-in handler's failures, which never reach here —
    // and this check fails it for the extended-protocol error any other `dispatch` arm
    // returns, so an arm that returns
    // an error does not have to remember to. `fail_transaction` is a no-op outside a
    // healthy block (an already aborted one included), so an error with no block open
    // leaves `[I]`, and a raiser that already failed the block is not failed twice.
    if !conn.skip_until_sync
        && responses
            .iter()
            .any(|m| matches!(m, BackendMessage::ErrorResponse { .. }))
        && !responses
            .iter()
            .any(|m| matches!(m, BackendMessage::ReadyForQuery { .. }))
    {
        conn.fail_transaction();
        conn.skip_until_sync = true;
    }

    responses
}

/// Dispatch one frontend message to its handler arm. The skip-until-`Sync` error
/// recovery wraps this in [`handle_message`]; arms here are free to early-`return`
/// their error responses.
fn dispatch(conn: &mut Connection, msg: FrontendMessage) -> Vec<BackendMessage> {
    match msg {
        FrontendMessage::Startup { version, params } => {
            // Protocol version validation
            let major = version >> 16;
            let minor = version & 0xFFFF;

            if version != pglet_wire::protocol::PROTOCOL_VERSION_3_0 {
                eprintln!(
                    "Client requested protocol {major}.{minor}, using 3.0 (we only support 3.0)"
                );
                // TODO: answer a minor above 3.0 with `NegotiateProtocolVersion` (newest minor
                // 3.0, plus any unrecognized `_pq_.` options), as 18.6 does past its own 3.2
                // (`backend_startup.c`). Today any 3.x request is served as 3.0 and the client
                // is never told, so one that asked for 3.2 may assume 3.2 framing.
            }

            // The packet's settings, read as 18.6 reads them (`startup` module): its `options`
            // string's switches, then its other keys, the settings pglet models each checked as
            // 18.6 checks it, in that order, the last assignment of a setting being the session's
            // value — and then pglet's own refusals, judged on those final values (a
            // `search_path` gets its `set_config`'s verdict).
            //
            // The packet is the client's **request** channel, not only pglet's assertion:
            // libpq, JDBC and asyncpg set `client_encoding` here rather than with a statement.
            // Refusing an unservable one is the same call the `SET` route makes, at the only
            // phase this route has — measured over a packet asking for `LATIN1`, 18.6 sends
            // `63 61 66 e9` for `café` and pglet sends `63 61 66 c3 a9`, which that client
            // renders `cafÃ©`. 18.6 answers an encoding name it does not know with `FATAL 22023`
            // and `mule_internal` with `FATAL 0A000`, and closes; this answers the same, and
            // additionally refuses `LATIN1` (`22023`), which 18.6 serves by converting and pglet
            // cannot. `quote_all_identifiers` on is its
            // `SET`'s `0A000`, as a `FATAL`.
            //
            // `application_name` is the session's value **and** its `RESET` target — measured
            // on 18.6: after `SET application_name TO 'probe'`, its `DISCARD ALL` reports the
            // packet's value back, not a compile-time default.
            let steps = crate::startup::startup_steps(&params);
            let application_name = crate::startup::final_value(&steps, "application_name")
                .unwrap_or_default()
                .to_string();
            // The packet's `user` and `database` are the session's identity — what
            // `current_user` and `current_database()` answer. pglet authenticates
            // nobody, so echoing the client's claim is exactly what a real Postgres
            // under `trust` does; the fallback is the embed hosts' constant, reached
            // only by a client that omits `user` (which the protocol requires) or
            // `database` (which libpq defaults to the user name — so pglet does too,
            // matching what 18.6 would then be connected to).
            let packet = |key: &str| {
                params
                    .iter()
                    .rev()
                    .find(|(k, _)| k.eq_ignore_ascii_case(key))
                    .map(|(_, v)| v.clone())
            };
            let user = packet("user").unwrap_or_else(|| DEFAULT_SESSION_NAME.to_string());
            let database = packet("database").unwrap_or_else(|| user.clone());
            let mut settings = SessionSettings::new(application_name).with_identity(user, database);
            settings.set_default_lock_timeout(conn.shared.default_lock_timeout_ms());
            // `lock_timeout` and `default_transaction_isolation` are the session's values and
            // their `RESET` targets; one its type refuses is 18.6's `FATAL 22023`.
            if let Err(refusal) = crate::startup::apply_startup_steps(&mut settings, &steps) {
                conn.session.state = SessionState::Terminated;
                return fatal_with_code(refusal.code, &refusal.message);
            }
            conn.settings = settings;
            conn.session.params = params;
            conn.session.state = SessionState::Ready;

            // Report the runtime parameters a driver branches on at startup. The
            // list and its values are `catalog::startup_parameters()` — the same
            // table `SHOW` answers from, deliberately not a second copy: these
            // values were typed out twice, and `server_version` said `16.0` in both
            // places, so moving one would have left the other reporting a release
            // the engine no longer claims.
            let mut messages = vec![BackendMessage::AuthenticationOk];
            messages.extend(
                startup_parameters(&conn.settings)
                    .into_iter()
                    .map(|(name, value)| BackendMessage::ParameterStatus {
                        name: name.into(),
                        value: value.into(),
                    }),
            );
            messages.extend([
                BackendMessage::BackendKeyData {
                    process_id: conn.session.process_id,
                    secret_key: conn.session.secret_key,
                },
                BackendMessage::ReadyForQuery {
                    status: conn.transaction_status(),
                },
            ]);
            messages
        }

        FrontendMessage::Query { sql } => handle_query(conn, &sql),

        FrontendMessage::Terminate => {
            conn.session.state = SessionState::Terminated;
            vec![]
        }

        FrontendMessage::Parse {
            name,
            query,
            param_types,
        } => {
            // Plan the statement once here. The plan is cached (Arc-shared) so
            // Describe and Execute reuse it instead of re-parsing + re-planning,
            // and the same planned tree feeds parameter-type inference. A `$N` whose
            // type the client declared is planned as that type
            // (`parse_single_with_param_types`); every other `$N` is typed from its
            // usage context against the schema. Both are reported via Describe and used
            // to decode the Bind bytes.
            //
            // A plan **error** is raised right here, as Postgres does — parse
            // analysis is where it resolves names, types, and operators, so an
            // ill-typed clause is reported at Parse and never reaches Bind.
            // Deferring it was not merely late but wrong in *class*: with no plan
            // cached, `infer_param_types` never ran, so the statement advertised
            // **zero** parameters and Bind rejected the client's values with a
            // protocol-shaped `08P01` instead of the statement's own type error.
            //
            // This closes the phase gap only as far as pglet's planner reaches, and
            // the split is not per statement. Every column-resolution shape probed
            // raises right here at Parse, agreeing with Postgres in **phase** —
            // `SELECT nosuchcol FROM t` over a one- *or* two-column table, empty or
            // seeded; a missing `USING` column; and an `UPDATE`'s unresolved SET-list
            // column, each `42703` on both engines, so those agree in code too. Two
            // probed shapes agree in phase and **not** in code, because a grammar
            // rejection arrives *ahead* of column resolution and fires over a
            // resolvable column just the same: `count(*) OVER (PARTITION BY …)` is
            // pglet's `0A000` and a subquery `LIMIT` its `42601`, against 18.6's
            // `42703`. Measured with a raw v3 client sending Parse + Sync alone, so
            // no Describe can blur the phase: `E Z` on both engines, with no
            // `ParseComplete`. That is a statement about the shapes named here, not a
            // rule — an unnamed one is unprobed. One column spelling **defers**: an
            // `ON CONFLICT` arbiter (`ON CONFLICT (nosuchcol)`) completes Parse and
            // is `42P10` at Execute against 18.6's `42703` at Parse — the clause's
            // own residual, recorded at `EDGES.md` row 31.
            //
            // What defers is a **spelling** of an unresolved relation rather than
            // relation-ness. These raise at Parse instead, in three groups. Two are
            // not divergences at all, raising 18.6's own SQLSTATE at 18.6's own
            // phase: `nosuchschema.t` and an unowned qualifier (`nosuchalias.a`) are
            // `42P01` at Parse on **both** engines. Then the gates the **empty
            // expansion** feeds: a qualified or star-plus-item projection (`SELECT
            // t.* FROM nosuchtable t`, `SELECT *, 1 FROM nosuchtable`) is pglet's own
            // `0A000`, an ordinal `ORDER BY`/`GROUP BY` its `42P10`, a `DISTINCT ON`
            // over `*` its own `0A000` (the non-star spelling defers), and a `JOIN …
            // USING (a)` the *column* half's `42703` — the missing side contributing
            // no columns, so the name resolves nowhere. Last, a construct pglet does
            // not implement, which fires over a resolvable relation just the same and
            // so is not this residual: `NATURAL JOIN` and a window function at
            // `0A000`, `TABLE t` and a subquery `LIMIT` at `42601`.
            // A set operation is the same shape one gate further along, for every
            // operator and either arm: the arity check agrees with 18.6 (`SELECT a
            // FROM r1 UNION SELECT 1, 2` is `42601` on both), and the empty expansion
            // feeds it a wrong count — so `SELECT * FROM nosuchtable UNION SELECT 1`
            // raises `42601` here. Agreeing arities do not make it defer, since the
            // expansion feeds every select-list gate: `SELECT * FROM nosuchtable
            // UNION SELECT * FROM nosuchtable2` defers, and the same with a trailing
            // `ORDER BY 1` is `42P10`. Inside a CTE body the arity gate also beats the
            // unreferenced-CTE silence noted below.
            //
            // The rest defer, and their Describe reply follows `row_description`'s
            // rule and no other: `NoData` exactly when the statement returns **no
            // result set**, else a `RowDescription` — whose width, zero included, is
            // not what decides between them. The three code branches that answer
            // `NoResultSet` are named on `describe_result_shape` above; what fills the
            // first of them *in this family* is a DML with no `RETURNING` clause, and
            // that is all of it. `INSERT INTO nosuchtable VALUES (1)`,
            // `UPDATE nosuchtable SET x = 1` and `DELETE FROM nosuchtable` are each
            // `1 t n 2 n E Z` here against 18.6's bare `E Z` (re-measured). The
            // failed-re-plan branch reaches inside the family too, and what admits it
            // is whether the re-plan fails rather than how the reference is spelled:
            // over `rq (a INT, c INT)` with `nosuchtable` in the same FROM, dropping
            // `a` re-describes `SELECT rq.a FROM rq, nosuchtable` as `NoData` and the
            // bare `SELECT a FROM rq, nosuchtable` as a fabricated `a:text`, while both
            // `RETURNING` spellings of an `UPDATE … FROM nosuchtable` answer `NoData` —
            // a `RETURNING` name binds at plan time.
            //
            // Everything else here is a **fabricated** `RowDescription`, and the
            // wildcard spellings that expand to nothing are fabrications too — an
            // *empty* descriptor, not `NoData`: `SELECT * FROM nosuchtable`, the same
            // with a `WHERE` or a second missing relation, and
            // `INSERT`/`UPDATE`/`DELETE … RETURNING *` over a missing target are each
            // `1 t T[] 2 T[] E Z` (re-measured; they answered `n` while the descriptor
            // rule read a column count, which is `EDGES.md` row 59's defect and is
            // closed). The non-empty ones are not one kind either: `SELECT * FROM rz,
            // nosuchtable` and `UPDATE nosuchtable SET x = 1 FROM rz RETURNING *`
            // describe `rz`'s real columns, table OID and attnums included;
            // `SELECT 1 FROM nosuchtable` and `SELECT (SELECT 1 FROM nosuchtable)
            // FROM rz` describe leaves belonging to no relation; and `SELECT rz.a, x
            // FROM rz, nosuchtable` mixes both in one descriptor — all for statements
            // over a table that does not exist. `EDGES.md` row 59 records the same
            // descriptor rule over a zero-column relation that *exists*; the rule is
            // `row_description`'s and both rows are records over it.
            //
            // A fabricated item is **not** typed as though the relation existed. An
            // unresolved leaf is `text` in projection-leaf position, and an
            // expression over it takes the walkers' un-inferable-operand fallbacks,
            // which need not agree with what a `text` operand gives: `x + 1` declares
            // `int4` where `tx + 1` over a real `TEXT` column declares `text`, and
            // `x = 1` declares `bool` where `tx = 1` is `42883` at Parse on both
            // engines. Measured members are enumerated in `EDGES.md` row 29; the
            // fallback seam itself is `docs/declared_types.md` and row 54.
            //
            // The `42P01` then arrives at Execute — or nowhere. Five reachability
            // shapes silence it: an unreferenced CTE, a never-taken `CASE` arm (an
            // outer column may decide it), a `COALESCE` that short-circuits, an
            // `AND` / `OR` operand its left operand decides (`WHERE false AND EXISTS
            // (SELECT * FROM nosuchtable)`), and a subquery an **empty** outer scan
            // never runs (`SELECT * FROM rz WHERE EXISTS (SELECT * FROM nosuchtable)`
            // over an empty `rz` answers `SELECT 0`, and over a two-row `rz` is `42P01`
            // at Execute). Neither `LIMIT 0` nor an empty inner relation is one. A
            // `HAVING` that reaches the subquery takes a pglet gate's own `0A000 …
            // requires execution context` at Execute in place of the lookup, raised
            // over a resolvable relation just the same.
            // That residual is the planner's, not this handler's, and it is `EDGES.md`
            // rows 29 and 60.
            let aborted = conn.transaction_status() == TransactionStatus::Failed;
            let declared_param_types = param_types.clone();
            // One `lock_timeout` deadline for the waits below.
            let deadline = conn.writer_deadline();
            let (parsed, plan_gen) = loop {
                let (outcome, current_gen) = {
                    let schema = conn.shared.lock_schema();
                    let current_gen = schema.generation;
                    let outcome = if runs_outside_the_planner(&query) {
                        // The single-statement check is `parse_single`'s, and this
                        // branch bypasses `parse_single` — so it re-runs the check here
                        // over the split, or a `SET x; INSERT …` Parse advertises one
                        // statement and Execute runs both off one prepared statement,
                        // where 18.6 is `42601` "cannot insert multiple commands into a
                        // prepared statement" (the same class `parse_single` reports).
                        // Counted as 18.6 counts them: a trailing comment is no statement.
                        match pglet_sql::parser::raw_statements(&query) {
                            Ok(stmts) if stmts.len() > 1 => {
                                Parsed::Failed(pglet_sql::Error::syntax(
                                    "expected a single SQL statement".to_string(),
                                ))
                            }
                            // `DEALLOCATE`, `DISCARD` and `RESET` have `gram.y`
                            // grammars, so their syntax errors are raised by the **raw
                            // parse** on 18.6 — ahead of the aborted-block gate below,
                            // measured: in a failed block `Parse "DEALLOCATE a b"` is
                            // `42601` there while `Parse "DEALLOCATE nosuch"` is `25P02`.
                            // Deferring them to `Execute` (as `SET`/`SHOW` correctly do,
                            // their errors being *runtime* ones there — including
                            // `DISCARD ALL`'s `25001`, which is
                            // `PreventInTransactionBlock` inside `ProcessUtility`) put
                            // the whole family behind the gate. `Error::Syntax` is what
                            // carries it past the gate — see the stage note below.
                            Ok(_) if is_deallocate_command(&query) => {
                                match parse_deallocate(&query) {
                                    Ok(_) => Parsed::Statement {
                                        param_types: param_types.clone(),
                                        param_enums: Vec::new(),
                                        plan: None,
                                    },
                                    Err(e) => Parsed::Failed(e),
                                }
                            }
                            Ok(_) if is_discard_command(&query) => match parse_discard(&query) {
                                Ok(_) => Parsed::Statement {
                                    param_types: param_types.clone(),
                                    param_enums: Vec::new(),
                                    plan: None,
                                },
                                Err(e) => Parsed::Failed(e),
                            },
                            Ok(_) if is_reset_command(&query) => match parse_reset(&query) {
                                // `RESET TRANSACTION ISOLATION LEVEL`'s `0A000` is the one
                                // `reset_rest` production 18.6 parses and then refuses, so
                                // it arrives here as an `Err` and lands at `Parse` the way
                                // the grammar errors do rather than at `Execute`. 18.6
                                // raises it in `ProcessUtility`, one phase later; the
                                // placement is pglet's and the code is the oracle's.
                                Ok(_) => Parsed::Statement {
                                    param_types: param_types.clone(),
                                    param_enums: Vec::new(),
                                    plan: None,
                                },
                                Err(e) => Parsed::Failed(e),
                            },
                            Ok(_) => Parsed::Statement {
                                param_types: param_types.clone(),
                                param_enums: Vec::new(),
                                plan: None,
                            },
                            Err(e) => Parsed::Failed(e),
                        }
                    } else {
                        // A type the client declared is part of parse analysis, not only
                        // of the Bind decode: `parse_single_with_param_types`.
                        match pglet_sql::parser::parse_single_with_param_types(
                            &query,
                            &schema,
                            &param_types,
                        ) {
                            // A parameter left without a type is refused here, at
                            // `Parse`, with the planner's other refusals.
                            Ok(Some(plan)) => {
                                match infer_param_targets(&plan, &schema, &param_types) {
                                    Ok(targets) => Parsed::Statement {
                                        param_types: targets.types,
                                        param_enums: targets.enums,
                                        plan: Some(Arc::new(plan)),
                                    },
                                    Err(e) => Parsed::Failed(e),
                                }
                            }
                            Ok(None) => Parsed::Empty {
                                param_types: param_types.clone(),
                            },
                            Err(e) => Parsed::Failed(e),
                        }
                    };
                    (outcome, current_gen)
                };
                // A plan that failed because of another connection's uncommitted catalog
                // is not this statement's answer (`execute::run_one_locked`): wait for that
                // hold to end and parse again. A statement parsed outside the planner fails
                // for no catalog's reason, and an aborted block's answer is `25P02` below
                // whatever the catalog holds.
                if !aborted
                    && !runs_outside_the_planner(&query)
                    && let Parsed::Failed(e) = &outcome
                    && conn.plan_failure_is_the_writers(e, current_gen, |committed| {
                        // Inference with the plan, since a refusal it raises is one
                        // of the failures `e` can be.
                        pglet_sql::parser::parse_single_with_param_types(
                            &query,
                            committed,
                            &param_types,
                        )?
                        .map(|plan| infer_param_targets(&plan, committed, &param_types))
                        .transpose()
                    })
                {
                    if let Err(waited) = conn.wait_for_foreign_writer(deadline) {
                        break (Parsed::Failed(waited), current_gen);
                    }
                    continue;
                }
                break (outcome, current_gen);
            };

            // Postgres's `exec_parse_message` runs three stages in this order, and
            // the order is the whole contract in an aborted block: raw grammar parse
            // (plus the single-statement check), *then* the aborted-transaction
            // rejection, *then* parse analysis. So a failed block answers a grammar
            // error with the grammar error and everything else — an analysis error,
            // a perfectly valid statement, `SET`, `SHOW` — with `25P02`, and an input
            // that yields no statement at all still succeeds.
            //
            // pglet's front end is one pass (parse and plan are the same call), so
            // the stage split is approximated by SQLSTATE: `42601` is `Error::Syntax`,
            // which all but one of pglet's raise sites produce from the token stream.
            // Measured against 18.6, that is exact for every error Postgres's own
            // grammar raises. It lets through two kinds of `42601` Postgres would
            // answer `25P02`, and **both** are a pre-existing divergence surfaced
            // rather than a new one — each already reports a different code than
            // Postgres outside any transaction:
            //
            // - pglet's analysis-stage `Error::Syntax` — the `UNION` arity check, the
            //   only one raised outside the parser — and the constructs pglet's
            //   lexer/parser rejects that Postgres's grammar accepts and its analyzer
            //   then rejects (`SELECT 1e999999999999`, `LIMIT 'x'`).
            // - the statements pglet's grammar does not model at all, where Postgres's
            //   does: `END`, `ABORT`, `ROLLBACK TO SAVEPOINT`, `PREPARE TRANSACTION`,
            //   `PREPARE`, `EXPLAIN`. The first four are Postgres
            //   transaction-exit statements, and how a failed block answers them here
            //   splits by whether pglet's own classifier admits them: `END` and
            //   `ROLLBACK TO SAVEPOINT` pass the gate and get the parser's `42601`,
            //   while `ABORT` and `PREPARE TRANSACTION` do not and get `25P02` — two
            //   different codes for one gap, and the gap is the parser's rather than
            //   this gate's either way.
            if let Parsed::Failed(e) = &parsed
                && e.sqlstate() == "42601"
            {
                return extended_error(e);
            }
            if aborted
                && !matches!(parsed, Parsed::Empty { .. })
                && !is_transaction_exit_stmt(&query)
            {
                return extended_error(&pglet_sql::Error::AbortedTransaction);
            }
            let (inferred_types, inferred_enums, cached_plan) = match parsed {
                Parsed::Statement {
                    param_types,
                    param_enums,
                    plan,
                } => (param_types, param_enums, plan),
                Parsed::Empty { param_types } => (param_types, Vec::new(), None),
                Parsed::Failed(e) => {
                    // Postgres aborts an open transaction block on a Parse error —
                    // the next statement is `25P02` until ROLLBACK. `handle_message`
                    // fails the block and arms skip-until-`Sync` off the
                    // `ErrorResponse`, as it does for every extended-protocol error, so
                    // the client's pipelined Bind/Execute are discarded rather than
                    // cascading a second error.
                    return extended_error(&e);
                }
            };
            conn.prepared.insert(
                name,
                PreparedStatement {
                    query,
                    param_types: inferred_types,
                    param_enums: inferred_enums,
                    declared_param_types,
                    plan: cached_plan,
                    plan_gen,
                },
            );
            vec![BackendMessage::ParseComplete]
        }

        FrontendMessage::Bind {
            portal,
            statement,
            param_formats,
            params,
            result_formats,
        } => {
            let stmt = match conn.prepared.get(&statement) {
                Some(s) => s.clone(),
                None => {
                    return extended_error(&pglet_sql::Error::UndefinedPreparedStatement {
                        name: statement.clone(),
                    });
                }
            };

            // Postgres validates the supplied parameter count against the prepared
            // statement's parameter count *at Bind* — a mismatch is a protocol
            // violation (`08P01`), reported here before `BindComplete`, not deferred to
            // a wrong-class runtime error at Execute. `stmt.param_types.len()` is the
            // count `infer_param_types` derived (and reported to the client via
            // `ParameterDescription`), so the check agrees with what Describe advertised.
            let expected_params = stmt.param_types.len();
            if params.len() != expected_params {
                return error_with_code(
                    "08P01",
                    &format!(
                        "bind message supplies {} parameters, but prepared statement \"{statement}\" requires {expected_params}",
                        params.len()
                    ),
                );
            }

            // The aborted-block gate, third of the five phases Postgres gates. It sits
            // **after** the `08P01` count check above, as `exec_bind_message` does, and
            // it is the whole condition rather than its headline:
            //
            //   IsAbortedTransactionBlockState() &&
            //     (!(psrc->raw_parse_tree
            //        && IsTransactionExitStmt(psrc->raw_parse_tree->stmt))
            //      || numParams != 0)
            //
            // Both disjuncts carry a measured shape. Disjunct 1 refuses everything that
            // is not an exit statement — a comment-only statement included, because a
            // NULL `raw_parse_tree` fails the exit test rather than being exempted;
            // there is no `parsetree_list != NIL` exemption here, that test is
            // `exec_parse_message`'s alone, so copying the Parse arm's shape would
            // wrongly admit `/* x */`. Disjunct 2 then **un-exempts** an exit statement
            // that has parameters: `numParams` has already been checked equal to
            // `psrc->num_params` by the `08P01` above, so it reads as *this prepared
            // statement has parameters*, which an exit statement can only get by
            // declaring them (there is no `$N` to write in `ROLLBACK`). Postgres's own
            // comment gives the reason: it "can't risk calling user-defined I/O
            // functions" in an aborted transaction. Refusing here is also what stops
            // the portal from existing at all, which is why the `Describe` and
            // `Execute` that follow answer `34000` on both engines rather than needing
            // gates of their own.
            if conn.transaction_status() == TransactionStatus::Failed
                && (!is_transaction_exit_stmt(&stmt.query) || !stmt.param_types.is_empty())
            {
                return extended_error(&pglet_sql::Error::AbortedTransaction);
            }

            // Decode each parameter's wire bytes into a Value, using the inferred
            // type and the per-parameter format code. NULL params stay None. The
            // plan is bound from these values at Execute (no text substitution).
            let mut bound_values = Vec::with_capacity(params.len());
            for (i, raw) in params.iter().enumerate() {
                match raw {
                    None => bound_values.push(None),
                    Some(bytes) => {
                        let oid = stmt
                            .param_types
                            .get(i)
                            .and_then(|t| *t)
                            .unwrap_or(TypeOid::Text);
                        let format = format_code_at(&param_formats, i);
                        match decode_param(bytes, oid, format, i + 1) {
                            Ok(value) => bound_values.push(Some(value)),
                            Err(e) => return extended_error(&e),
                        }
                    }
                }
            }

            // The enum conversion of a bound value, at `Bind` — where 18.6 runs the
            // resolved parameter type's input function, once per value and before the
            // portal exists. Not at eval time: 18.6 refuses a non-member `$1` over an
            // **empty** table and inside a `WHERE false` alike, so nothing about the
            // refusal depends on a row, and the shapes it closes include writes (a
            // `DELETE … WHERE m < $1` that removed every row, an `INSERT … SELECT
            // greatest(m, $1)` that landed non-members in a `text` column).
            {
                let schema = conn.shared.lock_schema();
                if let Err(e) = bound_param_enum_legality(&stmt.param_enums, &bound_values, &schema)
                {
                    drop(schema);
                    return extended_error(&e);
                }
            }

            conn.portals.insert(
                portal,
                Portal {
                    query: stmt.query,
                    bound_values,
                    result_formats,
                    plan: stmt.plan,
                    plan_gen: stmt.plan_gen,
                    declared_param_types: stmt.declared_param_types,
                },
            );
            vec![BackendMessage::BindComplete]
        }

        FrontendMessage::Close { kind, name } => {
            match kind {
                DescribeKind::Statement => {
                    conn.prepared.remove(&name);
                }
                DescribeKind::Portal => {
                    conn.portals.remove(&name);
                }
            }
            vec![BackendMessage::CloseComplete]
        }

        FrontendMessage::Describe { kind, name } => {
            match kind {
                DescribeKind::Statement => {
                    // Get the prepared statement
                    let stmt = match conn.prepared.get(&name) {
                        Some(s) => s.clone(),
                        None => {
                            return extended_error(&pglet_sql::Error::UndefinedPreparedStatement {
                                name: name.clone(),
                            });
                        }
                    };

                    // Result shape via the cached plan (reused when current).
                    // TODO(mvcc): Could use snapshot for read-only describe
                    let shape = {
                        let schema = conn.shared.lock_schema();
                        describe_result_shape(
                            &stmt.query,
                            stmt.plan.as_ref(),
                            stmt.plan_gen,
                            &stmt.declared_param_types,
                            &schema,
                        )
                    };

                    // Describe's own aborted-block gate, and it is **not** the check
                    // `Parse` or `Bind` has: `exec_describe_statement_message` reads
                    // `psrc->resultDesc` — *does this statement return data* — rather
                    // than `parsetree_list != NIL` or `IsTransactionExitStmt`.
                    // Postgres's comment gives the reason: refusing every Describe
                    // "might break the ability of some clients to issue COMMIT or
                    // ROLLBACK commands, if they use code that blindly Describes
                    // whatever it does". So a no-data statement is still described in a
                    // failed block, on both engines, and handing this arm the Parse
                    // arm's condition would break exactly that recovery path. The gate
                    // precedes the `ParameterDescription`, as 18.6's does.
                    //
                    // `resultDesc` is a **pointer**, so the question is whether the
                    // statement returns a result set and not how wide it is: 18.6
                    // refuses a zero-column `SELECT *` here with `25P02` and describes a
                    // `RETURNING`-less `INSERT` (`t n Z(E)`), both measured. Reading a
                    // column count instead answered the first one `t n Z(E)` too.
                    //
                    // That widened this gate's reach with the rule: it refuses whatever
                    // `is_result_set` is true of, so every spelling that flipped to an
                    // empty `RowDescription` flipped here too, and no 18.6 answer bounds
                    // that half. `SELECT * FROM nosuchtable_ab` and
                    // `DELETE FROM nosuchtable_ab RETURNING *`, prepared while the block
                    // was healthy, are `25P02` here where they were `t n` before, while
                    // the `RETURNING`-less spelling is still described. 18.6 refuses all
                    // three at `Parse` (`42P01`), so none of them reaches its gate at all
                    // (`EDGES.md` rows 29 and 59).
                    if conn.transaction_status() == TransactionStatus::Failed
                        && shape.is_result_set()
                    {
                        return extended_error(&pglet_sql::Error::AbortedTransaction);
                    }

                    let row_desc = row_description(&shape);

                    // Convert Vec<Option<TypeOid>> to Vec<i32> for ParameterDescription
                    let param_type_oids: Vec<i32> = stmt
                        .param_types
                        .iter()
                        .map(|t| t.map_or(0, |oid| oid as i32))
                        .collect();

                    vec![
                        BackendMessage::ParameterDescription {
                            param_types: param_type_oids,
                        },
                        row_desc,
                    ]
                }
                DescribeKind::Portal => {
                    // For Portal: get bound query and describe it
                    let portal = match conn.portals.get(&name) {
                        Some(p) => p.clone(),
                        None => {
                            return extended_error(&pglet_sql::Error::UndefinedPortal {
                                name: name.clone(),
                            });
                        }
                    };

                    // TODO(mvcc): Could use snapshot for read-only describe
                    let shape = {
                        let schema = conn.shared.lock_schema();
                        describe_result_shape(
                            &portal.query,
                            portal.plan.as_ref(),
                            portal.plan_gen,
                            &portal.declared_param_types,
                            &schema,
                        )
                    };

                    // The portal leg of the same gate: `exec_describe_portal_message`
                    // reads `portal->tupDesc`, the same fact from the other function.
                    // The two legs must move together — patching one counterpart and
                    // not the other splits a statement's portal answer from its
                    // statement answer.
                    if conn.transaction_status() == TransactionStatus::Failed
                        && shape.is_result_set()
                    {
                        return extended_error(&pglet_sql::Error::AbortedTransaction);
                    }

                    vec![row_description(&shape)]
                }
            }
        }

        FrontendMessage::Execute { portal, .. } => {
            let p = match conn.portals.get(&portal) {
                Some(p) => p.clone(),
                None => {
                    return extended_error(&pglet_sql::Error::UndefinedPortal {
                        name: portal.clone(),
                    });
                }
            };

            // COPY drives an out-of-band byte sub-protocol that can't ride the
            // QueryResult channel. tokio-postgres and the spine driver prepare COPY,
            // so it arrives here (extended protocol) — intercept it before the normal
            // execution path (which would fail loud `0A000`).
            if let Some(messages) = crate::copy::try_copy_extended(conn, &p) {
                return messages;
            }

            // Execute the portal: reuse its cached plan (re-planning only if a DDL
            // changed the schema since Parse) and supply the decoded values via the
            // eval environment (no ReadyForQuery — that comes with Sync).
            execute_query_core(
                conn,
                &p.query,
                &Protocol::Extended {
                    result_formats: p.result_formats.clone(),
                },
                &p.bound_values,
                p.plan
                    .as_ref()
                    .map(|arc| (arc, p.plan_gen, p.declared_param_types.as_slice())),
            )
        }

        FrontendMessage::Flush => {
            // Flush requests immediate send of any buffered output.
            // Since we don't buffer responses, just return empty vec.
            vec![]
        }

        FrontendMessage::Sync => {
            // Sync ends the error-recovery window: re-synchronize with one
            // ReadyForQuery regardless of what was skipped.
            conn.skip_until_sync = false;
            vec![BackendMessage::ReadyForQuery {
                status: conn.transaction_status(),
            }]
        }

        FrontendMessage::SslRequest
        | FrontendMessage::GssEncRequest
        | FrontendMessage::CancelRequest { .. } => {
            // The negotiation/cancel special packets are handled at the connection
            // level (`run_connection_loop`): SSL/GSS are declined with `N`, a
            // CancelRequest drops the connection. Reaching dispatch would be a routing
            // bug; return nothing rather than treat them as queryable messages.
            vec![]
        }

        // A CopyData/CopyDone/CopyFail with no COPY in progress: while a copy-in is
        // active these are intercepted in `handle_message` before reaching dispatch,
        // so arriving here means the client sent one with no active COPY. Postgres
        // discards a stray copy frame (logs a warning); mirror that by ignoring it
        // (no response) rather than erroring the session.
        FrontendMessage::CopyData(_) | FrontendMessage::CopyDone | FrontendMessage::CopyFail(_) => {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    //! Direct tests of the extended-protocol router. The differential fixtures
    //! exercise the *happy* path through tokio-postgres; these drive the
    //! out-of-order / malformed / error-recovery sequences a hostile or buggy
    //! client can send, asserting the router fails loud with the right SQLSTATE and
    //! never panics — and that an extended error skips to `Sync` instead of
    //! cascading. The state machine had no direct coverage before.

    use std::sync::Arc;

    use pglet_wire::protocol::FormatCode;
    use pglet_wire::{
        BackendMessage, DescribeKind, FieldDescription, FrontendMessage, TransactionStatus,
    };

    use super::handle_message;
    use crate::connection::{Connection, SharedState};
    use crate::execute::{Protocol, execute_query_core};
    use crate::test_support::{cells, conn, has_error, prepared_stub, sqlstate, tags};
    use pglet_sql::types::TypeOid;

    /// A connection over a fresh in-memory CoW-btree engine (no sockets, no I/O).
    /// The SQLSTATE (`C` field) of the first `ErrorResponse`, if any.
    fn has_ready(msgs: &[BackendMessage]) -> bool {
        msgs.iter()
            .any(|m| matches!(m, BackendMessage::ReadyForQuery { .. }))
    }

    fn query(c: &mut Connection, sql: &str) -> Vec<BackendMessage> {
        handle_message(c, FrontendMessage::Query { sql: sql.into() })
    }

    /// Regression: a ROLLBACK must never move `Schema::generation` backward.
    ///
    /// The prepared-plan cache keys on `plan_gen == generation` (`execute.rs`), so a
    /// restored (repeated) stamp is an ABA — a plan cached at that stamp is reused
    /// against a *different* catalog and probes a since-freed `index_oid`, failing loud
    /// with "index not found" (the fuz_app pglet-server re-migration failure). Rollback
    /// must restore catalog *content* but advance the stamp monotonically — and keep it
    /// where the block changed no catalog, so a statement prepared before a DML-only
    /// block keeps its Parse-time plan after it, while one prepared over an index the
    /// rollback took back is planned again against the restored catalog.
    ///
    /// Red with every restore keeping the generation (the statement prepared inside the
    /// block probes the index the rollback dropped) and with every restore moving it
    /// (the generation after the DML-only block).
    #[test]
    fn rollback_keeps_schema_generation_monotonic() {
        let mut c = conn();
        assert!(!has_error(&query(
            &mut c,
            "CREATE TABLE t (id INT PRIMARY KEY, k INT)"
        )));
        assert!(!has_error(&query(&mut c, "CREATE INDEX idx ON t (k)")));
        assert!(!has_error(&query(&mut c, "INSERT INTO t VALUES (1, 2)")));
        let gen_before = c.shared.lock_schema().generation;
        let parse = |c: &mut Connection, name: &str| {
            let msgs = handle_message(
                c,
                FrontendMessage::Parse {
                    name: name.into(),
                    query: "SELECT id FROM t WHERE k = 2".into(),
                    param_types: vec![],
                },
            );
            assert!(!has_error(&msgs), "{msgs:?}");
            handle_message(c, FrontendMessage::Sync);
        };
        // What the prepared statement `name` answers: its rows, or its SQLSTATE.
        let execute = |c: &mut Connection, name: &str| {
            handle_message(
                c,
                FrontendMessage::Bind {
                    portal: String::new(),
                    statement: name.into(),
                    param_formats: vec![],
                    params: vec![],
                    result_formats: vec![],
                },
            );
            let msgs = handle_message(
                c,
                FrontendMessage::Execute {
                    portal: String::new(),
                    max_rows: 0,
                },
            );
            handle_message(c, FrontendMessage::Sync);
            sqlstate(&msgs).map_or_else(|| cells(&msgs), |code| vec![code])
        };
        parse(&mut c, "before");

        // A DML-only block's rollback keeps the stamp, and the Parse-time plan with it.
        assert!(!has_error(&query(&mut c, "BEGIN")));
        assert!(!has_error(&query(&mut c, "INSERT INTO t VALUES (2, 2)")));
        assert!(!has_error(&query(&mut c, "ROLLBACK")));
        assert_eq!(c.shared.lock_schema().generation, gen_before);
        assert_eq!(execute(&mut c, "before"), ["1"]);

        // A DDL transaction that rolls back — the generation-restoring path.
        assert!(!has_error(&query(&mut c, "BEGIN")));
        assert!(!has_error(&query(&mut c, "CREATE TABLE t2 (a INT)")));
        assert!(!has_error(&query(&mut c, "DROP INDEX idx")));
        assert!(!has_error(&query(&mut c, "CREATE INDEX idx2 ON t (k)")));
        parse(&mut c, "inside");
        assert_eq!(execute(&mut c, "inside"), ["1"]);
        assert!(!has_error(&query(&mut c, "ROLLBACK")));

        let gen_after = c.shared.lock_schema().generation;
        assert!(
            gen_after > gen_before,
            "rollback restored a prior generation (before={gen_before}, after={gen_after}) — ABA reopens the stale-plan bug",
        );
        assert_eq!(execute(&mut c, "inside"), ["1"]);
        assert_eq!(execute(&mut c, "before"), ["1"]);
    }

    /// A simple `Query` whose later statement names an operator pglet implements
    /// nothing for answers `42883`, as 18.6's analysis does, and the statements before
    /// it ran — per-statement autocommit, `EDGES.md` row 68, which is what keeps the
    /// rows 18.6's implicit transaction rolls back. The embed leg, with every form and
    /// the execution-error control, is `autocommit_atomicity.rs`.
    #[test]
    fn a_later_statement_naming_an_operator_pglet_lacks_answers_42883() {
        let mut c = conn();
        assert!(!has_error(&query(&mut c, "CREATE TABLE q (k INT)")));
        let rows = |c: &mut Connection| {
            query(c, "SELECT count(*) FROM q")
                .into_iter()
                .find_map(|m| match m {
                    BackendMessage::DataRow { values } => values.into_iter().next().flatten(),
                    _ => None,
                })
        };
        for later in ["1 ! 1", "1 & 1"] {
            for (before, kept, ran) in [
                ("INSERT INTO q VALUES (1)", &b"1"[..], 1),
                ("BEGIN; INSERT INTO q VALUES (1); COMMIT", &b"1"[..], 3),
                (
                    "INSERT INTO q VALUES (1); COMMIT; INSERT INTO q VALUES (2)",
                    &b"2"[..],
                    3,
                ),
            ] {
                assert!(!has_error(&query(&mut c, "DELETE FROM q")));
                let r = query(&mut c, &format!("{before}; SELECT {later}"));
                assert_eq!(sqlstate(&r).as_deref(), Some("42883"), "{later}: {r:?}");
                assert_eq!(tags(&r).len(), ran, "{before}; {later}: {r:?}");
                assert!(has_ready(&r), "{later}: {r:?}");
                assert_eq!(c.transaction_status(), TransactionStatus::Idle, "{r:?}");
                assert_eq!(rows(&mut c).as_deref(), Some(kept), "{before}; {later}");
            }
        }
    }

    fn bind_unnamed(statement: &str, params: Vec<Option<Vec<u8>>>) -> FrontendMessage {
        FrontendMessage::Bind {
            portal: String::new(),
            statement: statement.into(),
            param_formats: Vec::new(),
            params,
            result_formats: Vec::new(),
        }
    }

    // --- unknown statement / portal references all fail loud, never panic -------

    #[test]
    fn bind_unknown_statement_errors_26000() {
        let mut c = conn();
        let r = handle_message(&mut c, bind_unnamed("nope", Vec::new()));
        assert_eq!(sqlstate(&r).as_deref(), Some("26000"), "{r:?}");
    }

    #[test]
    fn execute_unknown_portal_errors_34000() {
        let mut c = conn();
        let r = handle_message(
            &mut c,
            FrontendMessage::Execute {
                portal: "ghost".into(),
                max_rows: 0,
            },
        );
        assert_eq!(sqlstate(&r).as_deref(), Some("34000"), "{r:?}");
    }

    #[test]
    fn describe_unknown_statement_and_portal_error() {
        let mut c = conn();
        let s = handle_message(
            &mut c,
            FrontendMessage::Describe {
                kind: DescribeKind::Statement,
                name: "nope".into(),
            },
        );
        assert_eq!(sqlstate(&s).as_deref(), Some("26000"), "{s:?}");
        // Sync to recover from the first error (else skip-until-Sync would, correctly,
        // discard the next message).
        handle_message(&mut c, FrontendMessage::Sync);
        let p = handle_message(
            &mut c,
            FrontendMessage::Describe {
                kind: DescribeKind::Portal,
                name: "nope".into(),
            },
        );
        assert_eq!(sqlstate(&p).as_deref(), Some("34000"), "{p:?}");
    }

    #[test]
    fn close_unknown_is_idempotent_no_error() {
        // Postgres makes Close a no-op on a missing target (CloseComplete, no error).
        let mut c = conn();
        for kind in [DescribeKind::Statement, DescribeKind::Portal] {
            let r = handle_message(
                &mut c,
                FrontendMessage::Close {
                    kind,
                    name: "absent".into(),
                },
            );
            assert!(
                !has_error(&r),
                "Close of a missing target must not error: {r:?}"
            );
            assert!(
                matches!(r.as_slice(), [BackendMessage::CloseComplete]),
                "{r:?}"
            );
        }
    }

    // --- malformed Bind params surface loud through the router ------------------

    #[test]
    fn bind_too_few_params_errors_08p01_at_bind() {
        // Postgres rejects a wrong parameter count *at Bind* with 08P01
        // (protocol_violation) — before BindComplete — not later at Execute. A
        // 2-param statement bound with 1 param must error here, and no BindComplete
        // may be emitted (a BindComplete would imply a successful bind).
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (id int)");
        query(&mut c, "INSERT INTO t VALUES (1)");
        let parse = handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT id FROM t WHERE id = $1 AND id = $2".into(),
                param_types: vec![Some(TypeOid::Int4), Some(TypeOid::Int4)],
            },
        );
        assert!(matches!(parse.as_slice(), [BackendMessage::ParseComplete]));
        // Supply only one of the two required params.
        let bind = handle_message(&mut c, bind_unnamed("", vec![Some(b"1".to_vec())]));
        assert_eq!(
            sqlstate(&bind).as_deref(),
            Some("08P01"),
            "too-few-params must fail loud at Bind with 08P01: {bind:?}"
        );
        assert!(
            !bind
                .iter()
                .any(|m| matches!(m, BackendMessage::BindComplete)),
            "a failed bind must not emit BindComplete: {bind:?}"
        );
    }

    #[test]
    fn bind_too_many_params_errors_08p01_at_bind() {
        // The mirror case: a 1-param statement bound with 2 params is the same 08P01.
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (id int)");
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT id FROM t WHERE id = $1".into(),
                param_types: vec![Some(TypeOid::Int4)],
            },
        );
        let bind = handle_message(
            &mut c,
            bind_unnamed("", vec![Some(b"1".to_vec()), Some(b"2".to_vec())]),
        );
        assert_eq!(sqlstate(&bind).as_deref(), Some("08P01"), "{bind:?}");
    }

    #[test]
    fn bind_correct_param_count_still_binds() {
        // The positive control: the exact param count still produces BindComplete.
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (id int)");
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT id FROM t WHERE id = $1 AND id = $2".into(),
                param_types: vec![Some(TypeOid::Int4), Some(TypeOid::Int4)],
            },
        );
        let bind = handle_message(
            &mut c,
            bind_unnamed("", vec![Some(b"1".to_vec()), Some(b"2".to_vec())]),
        );
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "{bind:?}"
        );
    }

    #[test]
    fn bind_zero_params_to_zero_param_statement_still_binds() {
        // A 0-param statement bound with 0 params is the common no-parameter case and
        // must still succeed (the count check treats 0 == 0 as valid).
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (id int)");
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT id FROM t".into(),
                param_types: Vec::new(),
            },
        );
        let bind = handle_message(&mut c, bind_unnamed("", Vec::new()));
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "{bind:?}"
        );
    }

    /// The `M` field of the first `ErrorResponse`, if any.
    fn error_message(msgs: &[BackendMessage]) -> Option<String> {
        msgs.iter().find_map(|m| match m {
            BackendMessage::ErrorResponse { fields } => fields
                .iter()
                .find(|(k, _)| *k == b'M')
                .map(|(_, v)| v.clone()),
            _ => None,
        })
    }

    #[test]
    fn a_declared_parameter_type_survives_a_re_plan() {
        // A type the client declares at `Parse` is part of the statement's parse
        // analysis, so a re-plan after DDL — at `Describe` and at `Execute` alike — must
        // be handed it again. Handed nothing, `coalesce($1, 1)` re-plans with `$1`
        // converted to the literal's `integer`: a `23` descriptor and the value `2`,
        // where the Parse-time plan and 18.6 answer `1700` and `1.5`.
        let mut c = conn();
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: "s".into(),
                query: "SELECT coalesce($1, 1) AS c".into(),
                param_types: vec![Some(TypeOid::Numeric)],
            },
        );
        handle_message(&mut c, FrontendMessage::Sync);
        // A DDL statement bumps the schema generation, which makes the cached plan stale.
        assert!(!has_error(&query(
            &mut c,
            "CREATE TABLE replan_bump (x INT)"
        )));
        let described = handle_message(
            &mut c,
            FrontendMessage::Describe {
                kind: DescribeKind::Statement,
                name: "s".into(),
            },
        );
        let described_types: Vec<i32> = described
            .iter()
            .find_map(|m| match m {
                BackendMessage::RowDescription { columns } => {
                    Some(columns.iter().map(|f| f.type_oid).collect())
                }
                _ => None,
            })
            .expect("a RowDescription");
        assert_eq!(
            described_types,
            vec![TypeOid::Numeric as i32],
            "{described:?}"
        );
        let bind = handle_message(
            &mut c,
            FrontendMessage::Bind {
                portal: String::new(),
                statement: "s".into(),
                param_formats: vec![FormatCode::Text],
                params: vec![Some(b"1.5".to_vec())],
                result_formats: vec![FormatCode::Text],
            },
        );
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "{bind:?}"
        );
        let executed = handle_message(
            &mut c,
            FrontendMessage::Execute {
                portal: String::new(),
                max_rows: 0,
            },
        );
        let value = executed.iter().find_map(|m| match m {
            BackendMessage::DataRow { values } => values.first().cloned().flatten(),
            _ => None,
        });
        assert_eq!(value.as_deref(), Some(&b"1.5"[..]), "{executed:?}");
    }

    #[test]
    fn a_declared_parameter_type_survives_a_re_plan_at_portal_describe() {
        // The portal leg of the test above: a portal bound **before** the DDL carries a
        // plan of the old generation, so `Describe` of the portal re-plans too, from the
        // declared types the portal keeps. Handed nothing, it describes `coalesce($1, 1)`
        // as `23`, where 18.6 and the statement's own `Describe` answer `1700`.
        let mut c = conn();
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: "s".into(),
                query: "SELECT coalesce($1, 1) AS c".into(),
                param_types: vec![Some(TypeOid::Numeric)],
            },
        );
        let bind = handle_message(
            &mut c,
            FrontendMessage::Bind {
                portal: "p".into(),
                statement: "s".into(),
                param_formats: vec![FormatCode::Text],
                params: vec![Some(b"1.5".to_vec())],
                result_formats: vec![FormatCode::Text],
            },
        );
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "{bind:?}"
        );
        // The DDL bumps the schema generation past the portal's plan.
        assert!(!has_error(&query(
            &mut c,
            "CREATE TABLE replan_portal_bump (x INT)"
        )));
        let described = handle_message(
            &mut c,
            FrontendMessage::Describe {
                kind: DescribeKind::Portal,
                name: "p".into(),
            },
        );
        let described_types: Vec<i32> = described
            .iter()
            .find_map(|m| match m {
                BackendMessage::RowDescription { columns } => {
                    Some(columns.iter().map(|f| f.type_oid).collect())
                }
                _ => None,
            })
            .expect("a RowDescription");
        assert_eq!(
            described_types,
            vec![TypeOid::Numeric as i32],
            "{described:?}"
        );
        let executed = handle_message(
            &mut c,
            FrontendMessage::Execute {
                portal: "p".into(),
                max_rows: 0,
            },
        );
        let value = executed.iter().find_map(|m| match m {
            BackendMessage::DataRow { values } => values.first().cloned().flatten(),
            _ => None,
        });
        assert_eq!(value.as_deref(), Some(&b"1.5"[..]), "{executed:?}");
    }

    #[test]
    fn a_binary_param_is_consumed_exactly_and_refused_at_bind_with_its_number() {
        // 18.6's `exec_bind_message` runs each parameter's `*recv` in order, then refuses
        // a payload the reader did not consume to the last byte with
        // `22P03 incorrect binary data format in bind parameter <n>`; a read past the end
        // is the message reader's `08P01`. The refused parameter here is the **second**,
        // behind a well-formed first, so a number that is not carried from the Bind
        // loop's own position cannot read as right. Each shape was measured on 18.6 over
        // a raw extended-protocol client.
        let mut c = conn();
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT $1::int4 AS a, $2::int4 AS b".into(),
                param_types: vec![Some(TypeOid::Int4), Some(TypeOid::Int4)],
            },
        );
        let bind_with = |c: &mut Connection, second: Vec<u8>| {
            handle_message(
                c,
                FrontendMessage::Bind {
                    portal: String::new(),
                    statement: String::new(),
                    param_formats: vec![FormatCode::Binary],
                    params: vec![Some(1i32.to_be_bytes().to_vec()), Some(second)],
                    result_formats: Vec::new(),
                },
            )
        };
        // (second parameter's payload, SQLSTATE, message)
        let cases: [(Vec<u8>, &str, &str); 3] = [
            (
                vec![0; 5],
                "22P03",
                "incorrect binary data format in bind parameter 2",
            ),
            (vec![0; 3], "08P01", "insufficient data left in message"),
            (Vec::new(), "08P01", "insufficient data left in message"),
        ];
        for (payload, code, message) in cases {
            let bind = bind_with(&mut c, payload);
            assert_eq!(sqlstate(&bind).as_deref(), Some(code), "{bind:?}");
            assert_eq!(error_message(&bind).as_deref(), Some(message), "{bind:?}");
            assert!(
                !bind
                    .iter()
                    .any(|m| matches!(m, BackendMessage::BindComplete)),
                "a failed bind must not emit BindComplete: {bind:?}"
            );
            handle_message(&mut c, FrontendMessage::Sync);
        }

        // The accept control: both parameters at exactly four bytes bind.
        let bind = bind_with(&mut c, 2i32.to_be_bytes().to_vec());
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "{bind:?}"
        );
    }

    /// An **undeclared** `$N` at an enum position is converted at `Bind`, where 18.6
    /// runs `enum_in` on the value the client sent — and the refusal reaches the wire
    /// there, before the portal exists.
    ///
    /// The differential corpus cannot hold this class: 18.6 resolves such a parameter to
    /// the enum's own OID and reports it in `ParameterDescription`, so the harness's
    /// client refuses to serialize a Rust `String` against it (`error serializing
    /// parameter 0`, measured) and the fixture's oracle leg never reaches the server.
    /// `crates/pglet/tests/enum_identity_gates.rs` covers the in-process host; this is
    /// the native server's leg, and the two hosts have their own decode paths.
    #[test]
    fn bind_undeclared_param_at_an_enum_position_converts_at_bind() {
        let mut c = conn();
        query(&mut c, "CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')");
        query(&mut c, "CREATE TABLE t (id INT, m mood NOT NULL)");
        query(
            &mut c,
            "INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy')",
        );

        // A read, a write, the branch fold, and a predicate no row reaches — each with
        // the parameter left for inference, which is what makes it 18.6's `unknown`.
        for sql in [
            "SELECT m > $1 FROM t",
            "SELECT greatest(m, $1) FROM t",
            "DELETE FROM t WHERE m < $1",
            "SELECT m > $1 FROM t WHERE false",
            // A cast of the parameter to the enum is the same conversion at `Bind`
            // (`EDGES.md` row 241's `$1::mq`), a predicate no row reaches included.
            "SELECT $1::mood::text FROM t WHERE false",
            "DELETE FROM t WHERE m = $1::mood",
            // …when the cast is the occurrence that fixes the parameter's type.
            "SELECT $1::mood::text, $1::text FROM t WHERE false",
        ] {
            assert!(!has_error(&parse(&mut c, sql, Vec::new())), "{sql}");
            let bind = handle_message(&mut c, bind_unnamed("", vec![Some(b"zzz".to_vec())]));
            assert_eq!(sqlstate(&bind).as_deref(), Some("22P02"), "{sql}: {bind:?}");
            handle_message(&mut c, FrontendMessage::Sync);
        }

        // The same two occurrences swapped: a `text` use fixes the parameter first, and the
        // cast converts a `text` value no row reaches — 18.6 binds it.
        assert!(!has_error(&parse(
            &mut c,
            "SELECT $1::text, $1::mood::text FROM t WHERE false",
            Vec::new()
        )));
        let bind = handle_message(&mut c, bind_unnamed("", vec![Some(b"zzz".to_vec())]));
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "a cast after a text use is not the Bind-time target: {bind:?}"
        );
        handle_message(&mut c, FrontendMessage::Sync);

        // The accept control, and the discriminator that says the value was converted
        // rather than the binding broken: a member binds, and the rows it selects are
        // the enum's **member positions** — only `happy` is greater than `ok`.
        assert!(!has_error(&parse(
            &mut c,
            "SELECT m > $1 FROM t WHERE m > $1",
            Vec::new()
        )));
        let bind = handle_message(&mut c, bind_unnamed("", vec![Some(b"ok".to_vec())]));
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "{bind:?}"
        );
        handle_message(&mut c, FrontendMessage::Sync);
        assert!(!has_error(&query(&mut c, "SELECT count(*) FROM t")));

        // And a **declared** `text` parameter is not this rule's: it is planned as a cast
        // to `text`, which `EDGES.md` row 114 records as an undecidable operand. Pinned
        // here as the over-refusal boundary — the gate must not reach it, whatever that
        // residual eventually becomes.
        assert!(!has_error(&parse(
            &mut c,
            "SELECT m > $1 FROM t",
            vec![Some(TypeOid::Text)]
        )));
        let bind = handle_message(&mut c, bind_unnamed("", vec![Some(b"zzz".to_vec())]));
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "a declared `text` parameter reached the undeclared rule: {bind:?}"
        );
    }

    #[test]
    fn bind_binary_jsonb_param_with_a_bad_version_byte_errors_xx000_at_bind() {
        // A binary-format `jsonb` param's first byte is its version, and only `1` is
        // supported — 18.6's `jsonb_recv` raises `unsupported jsonb version number <n>`
        // with no `errcode()`, so `errstart`'s elevel default makes it SQLSTATE `XX000`
        // (`Error::UnsupportedJsonbVersion`). This proves the refusal reaches the
        // wire at Bind rather than being swallowed: the payload below carries no version
        // byte at all, so its `{` (0x7b) is read as version 123, exactly as 18.6 reads it.
        let mut c = conn();
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT $1::jsonb".into(),
                param_types: vec![Some(TypeOid::Jsonb)],
            },
        );
        let bind = handle_message(
            &mut c,
            FrontendMessage::Bind {
                portal: String::new(),
                statement: String::new(),
                param_formats: vec![FormatCode::Binary],
                params: vec![Some(br#"{"a":1}"#.to_vec())],
                result_formats: Vec::new(),
            },
        );
        assert_eq!(
            sqlstate(&bind).as_deref(),
            Some("XX000"),
            "a bad jsonb version byte must fail loud at Bind: {bind:?}"
        );
        assert!(
            !bind
                .iter()
                .any(|m| matches!(m, BackendMessage::BindComplete)),
            "a failed bind must not emit BindComplete: {bind:?}"
        );

        // The accept control: the same JSON text behind its version byte binds.
        handle_message(&mut c, FrontendMessage::Sync);
        let bind = handle_message(
            &mut c,
            FrontendMessage::Bind {
                portal: String::new(),
                statement: String::new(),
                param_formats: vec![FormatCode::Binary],
                params: vec![Some([b"\x01".as_slice(), br#"{"a":1}"#].concat())],
                result_formats: Vec::new(),
            },
        );
        assert!(
            matches!(bind.as_slice(), [BackendMessage::BindComplete]),
            "{bind:?}"
        );
    }

    // --- error recovery: skip-until-Sync ---------------------------------------

    #[test]
    fn extended_error_skips_following_messages_until_sync() {
        let mut c = conn();
        // A failed Bind (unknown statement) arms the skip state…
        let bind = handle_message(&mut c, bind_unnamed("nope", Vec::new()));
        assert_eq!(sqlstate(&bind).as_deref(), Some("26000"));
        assert!(
            c.skip_until_sync,
            "an extended error must arm skip-until-Sync"
        );

        // …so a pipelined Execute is *discarded* (no cascading second error) —
        // before this fix it returned a spurious 34000 "portal does not exist".
        let exec = handle_message(
            &mut c,
            FrontendMessage::Execute {
                portal: String::new(),
                max_rows: 0,
            },
        );
        assert!(
            exec.is_empty(),
            "messages after an error are skipped until Sync: {exec:?}"
        );

        // Sync re-synchronizes with exactly one ReadyForQuery and clears the state.
        let sync = handle_message(&mut c, FrontendMessage::Sync);
        assert!(has_ready(&sync) && sync.len() == 1, "{sync:?}");
        assert!(!c.skip_until_sync, "Sync clears the skip state");

        // After Sync the router processes messages normally again.
        let again = handle_message(&mut c, bind_unnamed("still-nope", Vec::new()));
        assert_eq!(sqlstate(&again).as_deref(), Some("26000"), "{again:?}");
    }

    #[test]
    fn simple_query_error_does_not_arm_skip() {
        // A simple-Query error self-recovers (its response carries ReadyForQuery), so
        // it must NOT enter skip mode — a following message is processed normally.
        let mut c = conn();
        let bad = query(&mut c, "SELEC nonsense");
        assert!(
            has_error(&bad) && has_ready(&bad),
            "simple error carries its own RFQ: {bad:?}"
        );
        assert!(
            !c.skip_until_sync,
            "a simple-query error must not arm skip-until-Sync"
        );

        // Not discarded: this Execute is handled and returns its own (loud) error.
        let exec = handle_message(
            &mut c,
            FrontendMessage::Execute {
                portal: "ghost".into(),
                max_rows: 0,
            },
        );
        assert_eq!(sqlstate(&exec).as_deref(), Some("34000"), "{exec:?}");
    }

    #[test]
    fn full_extended_cycle_round_trips() {
        // Positive control: a well-formed Parse/Bind/Execute/Sync returns the
        // expected acknowledgements + a data row — proving the negative tests above
        // exercise a router that otherwise works, not one that errors on everything.
        let mut c = conn();
        query(&mut c, "CREATE TABLE t (id int, name text)");
        query(&mut c, "INSERT INTO t VALUES (1, 'alice'), (2, 'bob')");

        let parse = handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT name FROM t WHERE id = $1".into(),
                param_types: vec![Some(TypeOid::Int4)],
            },
        );
        assert!(matches!(parse.as_slice(), [BackendMessage::ParseComplete]));

        let bind = handle_message(&mut c, bind_unnamed("", vec![Some(b"2".to_vec())]));
        assert!(matches!(bind.as_slice(), [BackendMessage::BindComplete]));

        let exec = handle_message(
            &mut c,
            FrontendMessage::Execute {
                portal: String::new(),
                max_rows: 0,
            },
        );
        assert!(!has_error(&exec), "{exec:?}");
        let rows = exec
            .iter()
            .filter(|m| matches!(m, BackendMessage::DataRow { .. }))
            .count();
        assert_eq!(rows, 1, "id = 2 selects exactly one row: {exec:?}");
        assert!(
            exec.iter()
                .any(|m| matches!(m, BackendMessage::CommandComplete { .. })),
            "{exec:?}"
        );

        let sync = handle_message(&mut c, FrontendMessage::Sync);
        assert!(has_ready(&sync));
    }

    // --- Parse raises plan errors; the statements executed outside the planner don't -

    /// Parse the SQL as the unnamed statement and return the router's reply.
    fn parse(
        c: &mut Connection,
        sql: &str,
        param_types: Vec<Option<TypeOid>>,
    ) -> Vec<BackendMessage> {
        handle_message(
            c,
            FrontendMessage::Parse {
                name: String::new(),
                query: sql.into(),
                param_types,
            },
        )
    }

    /// Parse the SQL as a **named** prepared statement, no declared parameter types.
    fn parse_named(c: &mut Connection, name: &str, sql: &str) -> Vec<BackendMessage> {
        handle_message(
            c,
            FrontendMessage::Parse {
                name: name.into(),
                query: sql.into(),
                param_types: Vec::new(),
            },
        )
    }

    fn execute_unnamed(c: &mut Connection) -> Vec<BackendMessage> {
        handle_message(
            c,
            FrontendMessage::Execute {
                portal: String::new(),
                max_rows: 0,
            },
        )
    }

    /// The command tags of a message batch.
    #[test]
    fn parse_raises_a_gated_clause_error_instead_of_a_bind_count_violation() {
        // The regression this whole path exists for. `RETURNING i4 LIKE 'x%'` fails
        // the operator gate, so the statement never plans — and swallowing that at
        // Parse meant `infer_param_types` never ran, the statement advertised **zero**
        // parameters, and Bind answered a 1-parameter client with a protocol-shaped
        // `08P01`. Postgres raises the operator error at Parse (verified against 18.6),
        // so pglet must too: same SQLSTATE, same phase, and no ParseComplete.
        let mut c = conn();
        query(&mut c, "CREATE TABLE pg_t (i4 int, tx text)");
        let r = parse(
            &mut c,
            "UPDATE pg_t SET tx = $1 RETURNING i4 LIKE 'x%'",
            Vec::new(),
        );
        assert_eq!(sqlstate(&r).as_deref(), Some("42883"), "{r:?}");
        assert!(
            !r.iter().any(|m| matches!(m, BackendMessage::ParseComplete)),
            "a failed Parse must not also report ParseComplete: {r:?}"
        );
    }

    #[test]
    fn parse_refuses_a_parameter_left_without_a_type_unless_the_client_declared_it() {
        // 18.6 refuses these at `Parse` — `42P18` where nothing types the parameter,
        // `42P08` where a later occurrence does — and answers the same statement once
        // the Parse message declares the type (measured over a raw v3 client). No
        // `ParseComplete`, so no `Bind` can run the statement against
        // whatever type the client's value carries.
        let mut c = conn();
        query(&mut c, "CREATE TABLE pg_u (i4 int, tx text)");
        for (sql, state) in [
            ("SELECT tx FROM pg_u WHERE $1 IS NULL", "42P18"),
            ("SELECT tx FROM pg_u WHERE $1 IS NULL OR tx = $1", "42P08"),
        ] {
            let r = parse(&mut c, sql, Vec::new());
            assert_eq!(sqlstate(&r).as_deref(), Some(state), "{sql}: {r:?}");
            assert!(
                !r.iter().any(|m| matches!(m, BackendMessage::ParseComplete)),
                "{sql}: {r:?}"
            );
            let sync = handle_message(&mut c, FrontendMessage::Sync);
            assert!(has_ready(&sync));
            let r = parse(&mut c, sql, vec![Some(TypeOid::Text)]);
            assert!(
                r.iter().any(|m| matches!(m, BackendMessage::ParseComplete)),
                "{sql} with a declared type: {r:?}"
            );
            let sync = handle_message(&mut c, FrontendMessage::Sync);
            assert!(has_ready(&sync));
        }
    }

    #[test]
    fn a_parameter_refusal_at_parse_does_not_wait_on_another_blocks_catalog() {
        // The refusal is the statement's own: it is the same against the catalog another
        // connection's open block has changed and against the committed one, so `Parse`
        // answers it at once. Red with the committed-catalog replan stopping at the
        // plan — it then succeeds, the refusal reads as that block's doing, and `Parse`
        // waits for the block and answers `55P03`.
        let shared = Arc::new(SharedState::with_btree());
        let mut a = Connection::new(1, Arc::clone(&shared));
        let mut b = Connection::new(2, Arc::clone(&shared));
        query(&mut a, "CREATE TABLE pg_w (i4 int)");
        query(&mut a, "BEGIN");
        query(&mut a, "ALTER TABLE pg_w ADD COLUMN tx text");
        let r = parse(&mut b, "SELECT i4 FROM pg_w WHERE $1 IS NULL", Vec::new());
        assert_eq!(sqlstate(&r).as_deref(), Some("42P18"), "{r:?}");
    }

    #[test]
    fn parse_raises_an_ill_typed_set_rhs_at_parse() {
        // The sibling gate on a different clause: the `SET` right-hand side's
        // common-type fold is `42804` in Postgres, raised at Parse.
        let mut c = conn();
        query(&mut c, "CREATE TABLE pg_s (i4 int, tx text)");
        let r = parse(
            &mut c,
            "UPDATE pg_s SET tx = COALESCE(i4, tx) WHERE i4 = $1",
            Vec::new(),
        );
        assert_eq!(sqlstate(&r).as_deref(), Some("42804"), "{r:?}");
    }

    /// An error raised at **any** extended-protocol phase fails an open transaction
    /// block, as 18.6's one error-recovery path does: `Sync` reports `[E]`, the next
    /// statement is `25P02`, and `COMMIT` rolls the block back, the insert made before
    /// the error included. Measured on 18.6 over a raw extended-protocol client for
    /// each refusal below; before this, each left the block `[T]` and the `COMMIT` kept
    /// both inserts. The control: the same refusal with no block open leaves `[I]` and a
    /// usable session.
    #[test]
    fn an_error_at_any_extended_phase_fails_an_open_transaction_block() {
        // (label, the failing exchange, its SQLSTATE)
        type Exchange = fn(&mut Connection) -> Vec<BackendMessage>;
        let cases: [(&str, Exchange, &str); 5] = [
            (
                "binary param leftover",
                |c| {
                    parse(c, "INSERT INTO rb VALUES ($1)", vec![Some(TypeOid::Int4)]);
                    handle_message(
                        c,
                        FrontendMessage::Bind {
                            portal: String::new(),
                            statement: String::new(),
                            param_formats: vec![FormatCode::Binary],
                            params: vec![Some(vec![0; 5])],
                            result_formats: Vec::new(),
                        },
                    )
                },
                "22P03",
            ),
            (
                "text param",
                |c| {
                    parse(c, "INSERT INTO rb VALUES ($1)", vec![Some(TypeOid::Int4)]);
                    handle_message(
                        c,
                        FrontendMessage::Bind {
                            portal: String::new(),
                            statement: String::new(),
                            param_formats: Vec::new(),
                            params: vec![Some(b"x".to_vec())],
                            result_formats: Vec::new(),
                        },
                    )
                },
                "22P02",
            ),
            (
                "parameter count",
                |c| {
                    parse(c, "INSERT INTO rb VALUES ($1)", vec![Some(TypeOid::Int4)]);
                    handle_message(
                        c,
                        FrontendMessage::Bind {
                            portal: String::new(),
                            statement: String::new(),
                            param_formats: Vec::new(),
                            params: Vec::new(),
                            result_formats: Vec::new(),
                        },
                    )
                },
                "08P01",
            ),
            (
                "describe unknown statement",
                |c| {
                    handle_message(
                        c,
                        FrontendMessage::Describe {
                            kind: DescribeKind::Statement,
                            name: "nope".into(),
                        },
                    )
                },
                "26000",
            ),
            (
                "execute unknown portal",
                |c| {
                    handle_message(
                        c,
                        FrontendMessage::Execute {
                            portal: "nope".into(),
                            max_rows: 0,
                        },
                    )
                },
                "34000",
            ),
        ];
        let count = |c: &mut Connection| {
            let rows = query(c, "SELECT count(*) FROM rb");
            rows.iter().find_map(|m| match m {
                BackendMessage::DataRow { values } => values[0].clone(),
                _ => None,
            })
        };
        let ready = |msgs: &[BackendMessage]| {
            msgs.iter().find_map(|m| match m {
                BackendMessage::ReadyForQuery { status } => Some(*status),
                _ => None,
            })
        };
        for (label, exchange, code) in cases {
            let mut c = conn();
            query(&mut c, "CREATE TABLE rb (a int4)");
            query(&mut c, "BEGIN");
            assert!(!has_error(&query(&mut c, "INSERT INTO rb VALUES (1)")));
            let failed = exchange(&mut c);
            assert_eq!(
                sqlstate(&failed).as_deref(),
                Some(code),
                "{label}: {failed:?}"
            );
            let sync = handle_message(&mut c, FrontendMessage::Sync);
            assert_eq!(
                ready(&sync),
                Some(TransactionStatus::Failed),
                "{label}: {sync:?}"
            );
            let next = query(&mut c, "INSERT INTO rb VALUES (2)");
            assert_eq!(
                sqlstate(&next).as_deref(),
                Some("25P02"),
                "{label}: {next:?}"
            );
            assert_eq!(tags(&query(&mut c, "COMMIT")), vec!["ROLLBACK"], "{label}");
            assert_eq!(count(&mut c).as_deref(), Some(b"0".as_ref()), "{label}");

            // The control: no block open.
            let failed = exchange(&mut c);
            assert_eq!(
                sqlstate(&failed).as_deref(),
                Some(code),
                "{label} idle: {failed:?}"
            );
            let sync = handle_message(&mut c, FrontendMessage::Sync);
            assert_eq!(ready(&sync), Some(TransactionStatus::Idle), "{label} idle");
            assert!(!has_error(&query(&mut c, "SELECT 1")), "{label} idle");
        }
    }

    /// The over-abort controls: extended-protocol messages that answer with no error
    /// leave an open block healthy — a statement whose reply carries a `NoticeResponse`,
    /// a `Close` naming a statement or a portal that does not exist, and a `Flush`.
    /// Measured on 18.6 over a raw extended-protocol client: `Sync` answers `[T]` after
    /// each, the next insert succeeds, and `COMMIT` keeps both rows.
    #[test]
    fn a_reply_with_no_error_leaves_an_open_transaction_block_healthy() {
        type Exchange = fn(&mut Connection) -> Vec<BackendMessage>;
        let cases: [(&str, Exchange); 4] = [
            ("notice-bearing statement", |c| {
                parse(c, "DROP TABLE IF EXISTS nope_x", Vec::new());
                handle_message(c, bind_unnamed("", Vec::new()));
                handle_message(
                    c,
                    FrontendMessage::Execute {
                        portal: String::new(),
                        max_rows: 0,
                    },
                )
            }),
            ("close unknown statement", |c| {
                handle_message(
                    c,
                    FrontendMessage::Close {
                        kind: DescribeKind::Statement,
                        name: "nope".into(),
                    },
                )
            }),
            ("close unknown portal", |c| {
                handle_message(
                    c,
                    FrontendMessage::Close {
                        kind: DescribeKind::Portal,
                        name: "nope".into(),
                    },
                )
            }),
            ("flush", |c| handle_message(c, FrontendMessage::Flush)),
        ];
        for (label, exchange) in cases {
            let mut c = conn();
            query(&mut c, "CREATE TABLE rb (a int4)");
            query(&mut c, "BEGIN");
            assert!(!has_error(&query(&mut c, "INSERT INTO rb VALUES (1)")));
            let reply = exchange(&mut c);
            assert!(!has_error(&reply), "{label}: {reply:?}");
            if label == "notice-bearing statement" {
                assert!(
                    reply
                        .iter()
                        .any(|m| matches!(m, BackendMessage::NoticeResponse { .. })),
                    "the control needs its notice: {reply:?}"
                );
            }
            let sync = handle_message(&mut c, FrontendMessage::Sync);
            assert!(
                sync.iter().any(|m| matches!(
                    m,
                    BackendMessage::ReadyForQuery {
                        status: TransactionStatus::InTransaction
                    }
                )),
                "{label}: {sync:?}"
            );
            assert!(
                !has_error(&query(&mut c, "INSERT INTO rb VALUES (2)")),
                "{label}"
            );
            assert_eq!(tags(&query(&mut c, "COMMIT")), vec!["COMMIT"], "{label}");
        }
    }

    #[test]
    fn parse_error_aborts_an_open_transaction_block() {
        // Postgres aborts the transaction on a Parse error — the next statement is
        // `25P02` until ROLLBACK (verified against 18.6). Without failing the block
        // the next statement would *succeed*, which is the silently-wrong half of the
        // same bug.
        let mut c = conn();
        query(&mut c, "CREATE TABLE pg_x (i4 int, tx text)");
        assert!(!has_error(&query(&mut c, "BEGIN")));
        let r = parse(
            &mut c,
            "UPDATE pg_x SET tx = $1 RETURNING i4 LIKE 'x%'",
            Vec::new(),
        );
        assert!(has_error(&r), "{r:?}");
        handle_message(&mut c, FrontendMessage::Sync);
        let after = query(&mut c, "SELECT 1");
        assert_eq!(
            sqlstate(&after).as_deref(),
            Some("25P02"),
            "a Parse error must abort the block: {after:?}"
        );
    }

    /// A malformed `DEALLOCATE` is `42601` at **`Parse`**, and it beats the
    /// aborted-block gate.
    ///
    /// `DeallocateStmt` is in `gram.y`, so 18.6 raises its syntax errors from the raw
    /// parse — which `exec_parse_message` runs *before* the abort rejection. Measured
    /// there over `client.prepare()` (Parse + Describe + Sync, so the phase is
    /// unambiguous), in a failed block: `DEALLOCATE a b` is `42601` and
    /// `DEALLOCATE nosuch` is `25P02`. Routing the statement through
    /// `runs_outside_the_planner` without validating it put the whole family behind
    /// the gate — right for `SET`/`SHOW`, whose errors *are* runtime errors on 18.6,
    /// and wrong for this one.
    #[test]
    fn a_malformed_deallocate_is_a_parse_error_ahead_of_the_abort_gate() {
        for (sql, code) in [
            ("DEALLOCATE", "42601"),
            ("DEALLOCATE a b", "42601"),
            ("DEALLOCATE \"\"", "42601"),
            // Well-formed: the name is a *runtime* fact, so Parse accepts it.
            ("DEALLOCATE nosuch", ""),
            ("DEALLOCATE ALL", ""),
        ] {
            let mut c = Connection::new(1, Arc::new(SharedState::with_btree()));
            let msgs = parse(&mut c, sql, Vec::new());
            assert_eq!(sqlstate(&msgs).as_deref().unwrap_or(""), code, "{sql}");

            // The same statement in an aborted block: a grammar error still wins, and
            // only a well-formed one is refused `25P02`.
            let mut c = Connection::new(1, Arc::new(SharedState::with_btree()));
            execute_query_core(&mut c, "BEGIN", &Protocol::Simple, &[], None);
            execute_query_core(&mut c, "SELECT nosuchthing", &Protocol::Simple, &[], None);
            assert_eq!(c.transaction_status(), TransactionStatus::Failed);
            let aborted_code = if code.is_empty() { "25P02" } else { code };
            let msgs = parse(&mut c, sql, Vec::new());
            assert_eq!(
                sqlstate(&msgs).as_deref().unwrap_or(""),
                aborted_code,
                "{sql} in an aborted block"
            );
        }
    }

    /// `DEALLOCATE` over the **extended** protocol: `Parse` must route it through
    /// `runs_outside_the_planner`, or the native parser rejects it and the statement
    /// never reaches the namespace. The aborted-block half is the load-bearing part —
    /// while it was a `42601` the `Parse` arm's syntax-error short-circuit answered
    /// ahead of the abort gate, so a failed block reported "pglet cannot read this
    /// statement" where 18.6 reports that the block is dead.
    #[test]
    fn deallocate_parses_binds_and_executes_over_the_extended_protocol() {
        let mut c = Connection::new(1, Arc::new(SharedState::with_btree()));
        c.prepared.insert("victim".into(), prepared_stub());
        assert!(
            parse(&mut c, "DEALLOCATE victim", Vec::new())
                .iter()
                .any(|m| matches!(m, BackendMessage::ParseComplete)),
            "Parse must accept it"
        );
        handle_message(&mut c, bind_unnamed("", Vec::new()));
        let msgs = execute_unnamed(&mut c);
        assert!(
            msgs.iter().any(
                |m| matches!(m, BackendMessage::CommandComplete { tag } if tag == "DEALLOCATE")
            ),
            "{msgs:?}"
        );
        assert!(!c.prepared.contains_key("victim"));
    }

    #[test]
    fn set_and_show_still_parse_bind_and_execute() {
        // `execute_query_core` services SET/SHOW ahead of the parser, which rejects
        // both `0A000` — so Parse must NOT plan them. They are the entire exception
        // list, and this is the test that keeps `runs_outside_the_planner` honest: if
        // it ever stops covering one, that statement starts erroring at Parse.
        for (sql, tag) in [
            ("SET client_encoding = 'UTF8'", "SET"),
            ("set search_path = public", "SET"),
            ("SET LOCAL statement_timeout = 0", "SET"),
            ("SHOW server_version", "SHOW"),
        ] {
            let mut c = conn();
            let p = parse(&mut c, sql, Vec::new());
            assert!(
                matches!(p.as_slice(), [BackendMessage::ParseComplete]),
                "{sql:?} must still parse: {p:?}"
            );
            let b = handle_message(&mut c, bind_unnamed("", Vec::new()));
            assert!(
                matches!(b.as_slice(), [BackendMessage::BindComplete]),
                "{sql:?}: {b:?}"
            );
            let e = execute_unnamed(&mut c);
            assert!(!has_error(&e), "{sql:?}: {e:?}");
            assert_eq!(tags(&e), vec![tag.to_string()], "{sql:?}: {e:?}");
        }
    }

    #[test]
    fn statements_that_plan_but_execute_specially_still_parse() {
        // The rest of the deferred-execution suspects: each of these *does* plan, so
        // raising plan errors at Parse leaves them untouched. Pinned because the
        // enumeration is the risky part of the change, not the mechanism.
        for sql in [
            "BEGIN",
            "COMMIT",
            "ROLLBACK",
            "START TRANSACTION",
            "DO $$ BEGIN CREATE TABLE pg_do (a int); END $$",
            "COPY pg_c TO STDOUT",
            "COPY pg_c FROM STDIN",
            "CREATE TABLE pg_new (a int)",
            "DROP TABLE IF EXISTS pg_absent",
            "TRUNCATE pg_c",
        ] {
            let mut c = conn();
            query(&mut c, "CREATE TABLE pg_c (i4 int, tx text)");
            let p = parse(&mut c, sql, Vec::new());
            assert!(
                matches!(p.as_slice(), [BackendMessage::ParseComplete]),
                "{sql:?} must still parse: {p:?}"
            );
        }
    }

    #[test]
    fn an_empty_statement_still_parses_and_returns_empty_query_response() {
        // `parse_single` reports these as `Ok(None)` — a plan legitimately *absent*,
        // not a failure — so they keep the pre-change path: client hints kept, no plan
        // cached, `EmptyQueryResponse` at Execute (Postgres agrees).
        for sql in ["", ";", "   \n\t  "] {
            let mut c = conn();
            let p = parse(&mut c, sql, Vec::new());
            assert!(
                matches!(p.as_slice(), [BackendMessage::ParseComplete]),
                "{sql:?}: {p:?}"
            );
            handle_message(&mut c, bind_unnamed("", Vec::new()));
            let e = execute_unnamed(&mut c);
            assert!(
                e.iter()
                    .any(|m| matches!(m, BackendMessage::EmptyQueryResponse)),
                "{sql:?}: {e:?}"
            );
        }
    }

    #[test]
    fn a_multi_statement_parse_is_rejected_like_postgres() {
        // Postgres 18.6 rejects a multi-command Parse with `42601` "cannot insert
        // multiple commands into a prepared statement". pglet's `parse_single` reports
        // the same class ("expected a single SQL statement", `42601`), and raising it
        // here is what makes the two agree — before, pglet quietly executed *both*
        // statements off one prepared statement.
        let mut c = conn();
        let r = parse(&mut c, "SELECT 1; SELECT 2", Vec::new());
        assert_eq!(sqlstate(&r).as_deref(), Some("42601"), "{r:?}");
    }

    #[test]
    fn a_multi_statement_parse_outranks_a_too_deep_chain() {
        // 18.6's `exec_parse_message` counts the statements after its raw parse and
        // before analysing either, and `54001` is an analysis error there — so
        // `SELECT <chain>; SELECT 1` is `42601` "cannot insert multiple commands into a
        // prepared statement" on 18.6, measured with a 12 000-term chain. The refusal
        // used to be raised at the first statement, before the count. 1 000 terms is
        // past the test thread's limit in either profile (`pglet_sql::depth`).
        let chain = vec!["1"; 1000].join(" + ");
        let mut c = conn();
        let r = parse(&mut c, &format!("SELECT {chain}; SELECT 1"), Vec::new());
        assert_eq!(sqlstate(&r).as_deref(), Some("42601"), "{r:?}");
        handle_message(&mut c, FrontendMessage::Sync);
        // The control: the chain alone is the refusal.
        let r = parse(&mut c, &format!("SELECT {chain};"), Vec::new());
        assert_eq!(sqlstate(&r).as_deref(), Some("54001"), "{r:?}");
        handle_message(&mut c, FrontendMessage::Sync);
        // A piece holding no token is no statement: 18.6's raw parse derives none from
        // a trailing comment, so it counts one statement and answers the chain's
        // `54001` (measured, unnamed and named `Parse`). Counting the comment as a
        // second statement answered `42601` instead.
        for tail in ["; /* c */", "; -- c", "; ; /* c */ ;"] {
            let r = parse(&mut c, &format!("SELECT {chain}{tail}"), Vec::new());
            assert_eq!(sqlstate(&r).as_deref(), Some("54001"), "{tail:?}: {r:?}");
            handle_message(&mut c, FrontendMessage::Sync);
            let r = parse_named(&mut c, "chained", &format!("SELECT {chain}{tail}"));
            assert_eq!(sqlstate(&r).as_deref(), Some("54001"), "{tail:?}: {r:?}");
            handle_message(&mut c, FrontendMessage::Sync);
        }
    }

    #[test]
    fn a_trailing_comment_is_not_a_second_statement() {
        // Every single-statement count counts as 18.6's raw parse does, so a trailing
        // comment neither refuses a `Parse` nor runs as a second piece at `Execute`.
        // Measured on 18.6 with a raw v3 client: each of these is `ParseComplete`, and
        // `Execute` answers the one statement's `CommandComplete`.
        let mut c = conn();
        for (sql, tag) in [
            ("SET search_path TO public; -- c", "SET"),
            ("SET search_path TO public; /* c */ ;", "SET"),
            ("SHOW search_path; -- c", "SHOW"),
            ("DEALLOCATE ALL; -- c", "DEALLOCATE ALL"),
            ("SELECT 1; -- c", "SELECT 1"),
        ] {
            let r = parse(&mut c, sql, Vec::new());
            assert!(
                matches!(r.as_slice(), [BackendMessage::ParseComplete]),
                "{sql:?}: {r:?}"
            );
            let b = handle_message(&mut c, bind_unnamed("", Vec::new()));
            assert!(!has_error(&b), "{sql:?}: {b:?}");
            let e = handle_message(
                &mut c,
                FrontendMessage::Execute {
                    portal: String::new(),
                    max_rows: 0,
                },
            );
            assert_eq!(tags(&e), vec![tag.to_string()], "{sql:?}: {e:?}");
            handle_message(&mut c, FrontendMessage::Sync);
        }
        // The control that keeps the count honest: two statements are still two.
        let r = parse(
            &mut c,
            "SET search_path TO public; SELECT 1 -- c",
            Vec::new(),
        );
        assert_eq!(sqlstate(&r).as_deref(), Some("42601"), "{r:?}");
    }

    #[test]
    fn a_set_led_multi_statement_parse_is_rejected_like_postgres() {
        // `SET` runs outside the planner, so Parse skips `parse_single` and with it
        // the single-statement check above. Before, `SET x; INSERT …` advertised one
        // statement and Execute answered `SET` for the whole text (the batch swallow);
        // with the per-statement interception it would instead run *both* off one
        // prepared statement. 18.6 is `42601` for either spelling.
        let mut c = conn();
        query(&mut c, "CREATE TABLE pg_set_led (id int)");
        let r = parse(
            &mut c,
            "SET TIME ZONE 'UTC'; INSERT INTO pg_set_led VALUES (1)",
            Vec::new(),
        );
        assert_eq!(sqlstate(&r).as_deref(), Some("42601"), "{r:?}");
        handle_message(&mut c, FrontendMessage::Sync);
        let rows = query(&mut c, "SELECT count(*) FROM pg_set_led");
        let row = rows.iter().find_map(|m| match m {
            BackendMessage::DataRow { values } => values[0].clone(),
            _ => None,
        });
        assert_eq!(row.as_deref(), Some(b"0".as_ref()), "nothing ran: {rows:?}");
        // The show side of the same predicate.
        let r = parse(&mut c, "SHOW server_version; SELECT 1", Vec::new());
        assert_eq!(sqlstate(&r).as_deref(), Some("42601"), "{r:?}");
    }

    #[test]
    fn a_failed_parse_caches_no_statement() {
        // Postgres leaves no prepared statement behind after a failed Parse, so a Bind
        // naming it is `26000`. pglet's early return skips the `prepared.insert`, which
        // gets this for free — pinned so a later refactor can't insert a half-built
        // entry whose zero-parameter shape is exactly the bug.
        let mut c = conn();
        query(&mut c, "CREATE TABLE pg_n (i4 int, tx text)");
        let r = parse(
            &mut c,
            "UPDATE pg_n SET tx = $1 RETURNING i4 LIKE 'x%'",
            Vec::new(),
        );
        assert!(has_error(&r), "{r:?}");
        handle_message(&mut c, FrontendMessage::Sync);
        let b = handle_message(&mut c, bind_unnamed("", vec![Some(b"z".to_vec())]));
        assert_eq!(sqlstate(&b).as_deref(), Some("26000"), "{b:?}");
    }

    // --- Parse inside an ALREADY-aborted transaction block ---------------------

    /// A connection whose transaction block is open **and failed**, the state
    /// `postgres.c`'s `IsAbortedTransactionBlockState()` gates on.
    fn aborted_conn() -> Connection {
        let mut c = conn();
        assert!(!has_error(&query(
            &mut c,
            "CREATE TABLE ab_t (i4 int, tx text)"
        )));
        assert!(!has_error(&query(&mut c, "BEGIN")));
        let failed = query(&mut c, "SELECT * FROM ab_definitely_absent");
        assert!(has_error(&failed), "{failed:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
        c
    }

    /// Parse `sql` in an aborted block and return the reply's SQLSTATE, or `None`
    /// when it answered `ParseComplete`.
    fn parse_aborted(sql: &str) -> Option<String> {
        let mut c = aborted_conn();
        let r = parse(&mut c, sql, Vec::new());
        if !has_error(&r) {
            assert!(
                matches!(r.as_slice(), [BackendMessage::ParseComplete]),
                "{sql:?}: {r:?}"
            );
        }
        sqlstate(&r)
    }

    #[test]
    fn parse_in_an_aborted_block_reports_25p02_not_the_statements_own_error() {
        // The regression the Parse-raises-plan-errors change introduced: raising the
        // plan error jumped the aborted-transaction gate, which is Execute-only, so a
        // failed block answered `42883`/`42804`/`42704` where Postgres 18.6 answers
        // `25P02`. `postgres.c`'s `exec_parse_message` puts its abort check *before*
        // parse analysis for exactly this reason, and the wrong class is worse than
        // the wrong phase: a driver keys retry/rollback logic off `25P02`.
        for sql in [
            // gated operator — the shape that motivated raising plan errors at Parse
            "UPDATE ab_t SET tx = $1 RETURNING i4 LIKE 'x%'",
            // common-type fold (42804 outside a failed block)
            "UPDATE ab_t SET tx = COALESCE(i4, tx) WHERE i4 = 1",
            // unsupported type (42704 outside a failed block)
            "SELECT '1'::no_such_type_xyz",
        ] {
            assert_eq!(parse_aborted(sql).as_deref(), Some("25P02"), "{sql:?}");
        }
    }

    #[test]
    fn parse_of_a_valid_statement_in_an_aborted_block_is_25p02() {
        // The pre-existing half the same gate closes: a statement with nothing wrong
        // with it used to answer `ParseComplete` (and `ParameterDescription` /
        // `RowDescription` on Describe) inside a failed block, where Postgres refuses
        // it. `SET` and `SHOW` are in the list because the Parse arm routes them past
        // the planner entirely — the gate has to catch them on the way past, and
        // Postgres 18.6 does reject both here.
        for sql in [
            "SELECT i4 FROM ab_t",
            "SELECT i4 FROM ab_t WHERE i4 = $1",
            "UPDATE ab_t SET tx = 'q'",
            "SELECT bogus FROM ab_t",
            "SELECT * FROM ab_no_such_table",
            "SET client_encoding = 'UTF8'",
            "SHOW server_version",
            "BEGIN",
            "START TRANSACTION",
            "CREATE TABLE ab_new (a int)",
            "DROP TABLE IF EXISTS ab_absent",
            "DO $$ BEGIN END $$",
            "COPY ab_t TO STDOUT",
            "SELECT 1",
        ] {
            assert_eq!(parse_aborted(sql).as_deref(), Some("25P02"), "{sql:?}");
        }
    }

    #[test]
    fn a_syntax_error_in_an_aborted_block_reports_itself() {
        // Postgres runs the raw grammar parse — and the single-statement check —
        // *before* the abort gate, so a grammar error surfaces as itself even in a
        // failed block (measured on 18.6: `SELEC 1` and a multi-command Parse are
        // both `42601` there, not `25P02`). pglet's front end is one pass, so it
        // approximates the stage split by SQLSTATE: `42601` passes through.
        for sql in [
            "SELEC 1",
            "SELECT FROM FROM",
            "SELECT 'abc",
            "SELECT 1 /* nope",
            "SELECT 1; SELECT 2",
        ] {
            assert_eq!(parse_aborted(sql).as_deref(), Some("42601"), "{sql:?}");
        }
    }

    #[test]
    fn an_analysis_stage_42601_is_a_known_false_passthrough() {
        // The one measured cost of approximating Postgres's *stage* split with a
        // SQLSTATE: `each UNION query must have the same number of columns` is pglet's
        // only `Error::Syntax` raised outside the parser, and Postgres raises the same
        // message from parse *analysis* — so 18.6 answers `25P02` here and pglet
        // answers `42601`. Pinned as a divergence, not an assertion of correctness;
        // closing it means tagging pglet's errors by stage.
        assert_eq!(
            parse_aborted("SELECT 1 UNION SELECT 1, 2").as_deref(),
            Some("42601"),
        );
    }

    #[test]
    fn an_empty_parse_in_an_aborted_block_still_succeeds() {
        // Postgres's abort check sits *inside* `if (parsetree_list != NIL)`, so text
        // that yields no statement never reaches it — verified on 18.6, where all
        // four still answer `ParseComplete` in a failed block.
        for sql in ["", ";", "   \n\t  ", "-- nothing"] {
            assert_eq!(parse_aborted(sql), None, "{sql:?}");
        }
    }

    #[test]
    fn parse_of_a_transaction_exit_statement_in_an_aborted_block_succeeds() {
        // `IsTransactionExitStmt` is the abort gate's exemption, and it has to hold at
        // Parse or a pooled client that prepares its `COMMIT`/`ROLLBACK` can never end
        // the block. `is_transaction_exit_stmt` is shared with `execute_query_core`'s
        // gate, so the two phases admit the same set by construction.
        for sql in ["COMMIT", "ROLLBACK", "commit", "  ROLLBACK ;"] {
            assert_eq!(parse_aborted(sql), None, "{sql:?}");
        }
    }

    #[test]
    fn a_transaction_exit_statement_parsed_while_aborted_still_ends_the_block() {
        // The exemption is only worth having if the statement then works: Parse,
        // Bind, Execute a `COMMIT` from inside the failed block and the connection
        // comes back Idle with the `ROLLBACK` tag Postgres reports (18.6:
        // `ParseComplete` … `CommandComplete(ROLLBACK)`, `ReadyForQuery[I]`).
        let mut c = aborted_conn();
        assert_eq!(parse(&mut c, "COMMIT", Vec::new()).len(), 1);
        handle_message(&mut c, bind_unnamed("", Vec::new()));
        let e = execute_unnamed(&mut c);
        assert!(!has_error(&e), "{e:?}");
        assert_eq!(tags(&e), vec!["ROLLBACK".to_string()], "{e:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Idle);
    }

    #[test]
    fn a_25p02_parse_still_skips_pipelined_messages_to_sync() {
        // The gate returns a bare `ErrorResponse`, so `handle_message` arms
        // skip-until-`Sync` off it exactly as it does for a plan error: a client that
        // pipelined Parse/Bind/Execute/Sync gets **one** error, not a cascade, and the
        // block is still failed afterwards (18.6 answers the same single `25P02`).
        let mut c = aborted_conn();
        let p = parse(&mut c, "SELECT i4 FROM ab_t", Vec::new());
        assert_eq!(sqlstate(&p).as_deref(), Some("25P02"), "{p:?}");
        assert!(handle_message(&mut c, bind_unnamed("", Vec::new())).is_empty());
        assert!(execute_unnamed(&mut c).is_empty());
        let sync = handle_message(&mut c, FrontendMessage::Sync);
        assert!(has_ready(&sync), "{sync:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
    }

    #[test]
    fn the_aborted_gate_does_not_fire_outside_a_failed_block() {
        // The other direction: the gate keys on `TransactionStatus::Failed`, so a
        // healthy block and an autocommit connection are untouched — a plan error is
        // still the statement's own error there (the whole point of raising it at
        // Parse), and a valid statement still parses.
        let mut c = conn();
        query(&mut c, "CREATE TABLE ab_ok (i4 int, tx text)");
        assert!(!has_error(&query(&mut c, "BEGIN")));
        let good = parse(&mut c, "SELECT i4 FROM ab_ok", Vec::new());
        assert!(!has_error(&good), "{good:?}");
        let bad = parse(
            &mut c,
            "UPDATE ab_ok SET tx = $1 RETURNING i4 LIKE 'x%'",
            Vec::new(),
        );
        assert_eq!(sqlstate(&bad).as_deref(), Some("42883"), "{bad:?}");
    }

    /// A Bind reply reduced to a comparable shape — `BindComplete`, or the SQLSTATE it
    /// refused with.
    fn bind_shape(msgs: &[BackendMessage]) -> String {
        match sqlstate(msgs) {
            Some(code) => format!("ERROR {code}"),
            None if matches!(msgs, [BackendMessage::BindComplete]) => "BindComplete".to_owned(),
            None => format!("{msgs:?}"),
        }
    }

    /// Describe the unnamed portal.
    fn describe_unnamed_portal(c: &mut Connection) -> Vec<BackendMessage> {
        handle_message(
            c,
            FrontendMessage::Describe {
                kind: DescribeKind::Portal,
                name: String::new(),
            },
        )
    }

    #[test]
    // `c`/`p`/`b`/`d`/`e` are the connection and one binding per protocol message
    // (Parse/Bind/Describe/Execute); the message name is what carries the meaning.
    #[allow(clippy::many_single_char_names)]
    fn bind_in_an_aborted_block_is_refused_and_creates_no_portal() {
        // The gate-site enumeration's measured hole, now closed. `postgres.c` gates the
        // aborted block at **five** statement-processing phases — six
        // `IsAbortedTransactionBlockState()` tests, since Describe is two functions:
        // `exec_simple_query`, `exec_parse_message`, `exec_bind_message`,
        // `exec_describe_statement_message` + `exec_describe_portal_message`, and
        // `exec_execute_message`. pglet had three; this test is the Bind one, and the
        // Describe one is the `describe_in_an_aborted_block_…` test below, which asks a
        // different question rather than the same one a phase later.
        //
        // Which shapes reach Bind in a failed block is decided by pglet's *Parse* gate,
        // not by Postgres's: that gate exempts `Parsed::Empty` and
        // `is_transaction_exit_stmt`, and a statement parsed *before* the failure never
        // met it at all. Two are driven here. Measured against 18.6 with a raw v3
        // client that syncs and reads each phase — the `was` column on the in-memory
        // B+tree, `--storage memory` and stripped-B+tree legs identically, the `is`
        // column re-measured on the in-memory B+tree, where it matches 18.6:
        //
        //   phase                                        was              is / 18.6
        //   Bind a comment-only statement                BindComplete     25P02
        //   Describe that portal                         NoData           34000
        //   Execute that portal                          25P02            34000
        //   Bind a statement parsed before the failure   BindComplete     25P02
        //   Describe that portal                         RowDescription   34000
        //   Execute that portal                          25P02            34000
        //
        // The `Bind` rows are the ones the gate answers. The `Describe` and `Execute`
        // rows are consequences: 18.6 reports `34000` *because* its own Bind refused to
        // create the portal, and refusing here gets the same for free — which is why
        // neither needs a gate of its own, and why the criterion that says so about
        // `Execute` says it about `Describe` too. (Describe's *own* gate is a different
        // question and needs a portal that exists on both engines; that is the test
        // below.)
        //
        // A transaction-exit statement is the other thing pglet's Parse gate lets
        // through, and the exit spellings do **not** answer alike — no one sentence
        // covers them. The spellings measured against 18.6:
        //
        //   a bare `ROLLBACK`, no declared parameter types
        //     both engines: ParseComplete / BindComplete / NoData /
        //     CommandComplete(ROLLBACK), and the block is Idle afterwards. Guarded by
        //     `a_transaction_exit_statement_parsed_while_aborted_still_ends_the_block`.
        //     Same on both engines when it is parsed *before* the failure.
        //   `ROLLBACK` parsed with one declared parameter OID, one text parameter bound
        //     both engines: 25P02 at Bind, 34000 at Describe and Execute, and the block
        //     is still Failed. `exec_bind_message`'s `numParams != 0` disjunct. Guarded
        //     by `a_parameterized_exit_statement_is_refused_at_bind_and_the_block_stays_failed`.
        //   `/* trace */ ROLLBACK` (and the `-- trace`-newline spelling)
        //     both engines end the block: the classifier reads the lexer, so the comment
        //     is stripped before anything classifies. It used to be `25P02` at **Parse**
        //     here and it wedged the session; guarded in `execute.rs`'s
        //     `a_comment_prefixed_exit_ends_the_block_instead_of_wedging_the_session`.
        //
        // The relation asserted ahead of the pins is the one meant to outlive them:
        // 18.6 refuses **both** shapes at Bind, so whichever way this gate moves the two
        // have to move together. `exec_bind_message` has no `parsetree_list != NIL`
        // exemption — that test is `exec_parse_message`'s alone — so a Bind gate written
        // by copying the Parse arm's shape, `Parsed::Empty` exemption included, would
        // admit `cmt` and refuse `pre`. That is the wrong fix this forbids, and putting
        // the check ahead of the pins is what makes *this* message the one a fixer sees.
        let mut c = conn();
        assert!(!has_error(&query(&mut c, "CREATE TABLE ab_pre (i4 int)")));
        assert!(!has_error(&query(&mut c, "BEGIN")));

        // Prepared while the block is still healthy, so the Parse gate never sees it.
        let p = handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: "pre".into(),
                query: "SELECT i4 FROM ab_pre".into(),
                param_types: Vec::new(),
            },
        );
        assert!(!has_error(&p), "{p:?}");

        let failed = query(&mut c, "SELECT * FROM ab_definitely_absent");
        assert!(has_error(&failed), "{failed:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);

        // Shape 1 — a comment-only statement. It parses *inside* the failed block,
        // because the Parse gate exempts `Parsed::Empty` exactly as Postgres's abort
        // check sits inside `if (parsetree_list != NIL)`.
        let pc = handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: "cmt".into(),
                query: "/* x */".into(),
                param_types: Vec::new(),
            },
        );
        assert!(
            matches!(pc.as_slice(), [BackendMessage::ParseComplete]),
            "{pc:?}"
        );

        // A `Sync` after every phase, as the raw v3 probe does: a refused reply arms
        // skip-until-`Sync`, so without it the phases after it would read `[]` and the
        // failure would name a swallowed message instead of the refusal that caused it.
        let b_cmt = handle_message(&mut c, bind_unnamed("cmt", Vec::new()));
        handle_message(&mut c, FrontendMessage::Sync);
        let d_cmt = describe_unnamed_portal(&mut c);
        handle_message(&mut c, FrontendMessage::Sync);
        let e_cmt = execute_unnamed(&mut c);
        handle_message(&mut c, FrontendMessage::Sync);
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);

        // Shape 2 — the statement prepared before the failure.
        let b = handle_message(&mut c, bind_unnamed("pre", Vec::new()));
        handle_message(&mut c, FrontendMessage::Sync);
        let d = describe_unnamed_portal(&mut c);
        handle_message(&mut c, FrontendMessage::Sync);
        let e = execute_unnamed(&mut c);
        handle_message(&mut c, FrontendMessage::Sync);

        // Asserted first, deliberately: the relation is what a wrong fix breaks, and a
        // panicking pin pre-empts everything after it, so a re-pin that reorders this
        // file silently loses the discrimination.
        assert_eq!(
            bind_shape(&b_cmt),
            bind_shape(&b),
            "Bind must not distinguish a comment-only statement from a parsed one: 18.6 \
             refuses both",
        );

        assert_eq!(
            bind_shape(&b_cmt),
            "ERROR 25P02",
            "a comment-only statement is refused at Bind — its NULL `raw_parse_tree` \
             fails the exit-statement test rather than exempting it: {b_cmt:?}"
        );
        assert_eq!(sqlstate(&d_cmt).as_deref(), Some("34000"), "{d_cmt:?}");
        assert_eq!(sqlstate(&e_cmt).as_deref(), Some("34000"), "{e_cmt:?}");
        assert_eq!(bind_shape(&b), "ERROR 25P02", "{b:?}");
        assert_eq!(sqlstate(&d).as_deref(), Some("34000"), "{d:?}");
        assert_eq!(sqlstate(&e).as_deref(), Some("34000"), "{e:?}");

        // The half of this test's name nothing else asserts. The rows above reach the
        // portal only *through* phases the gate also changed, so "refused, but the
        // portal was created anyway" — a leak that hands a later `Execute` or a cursor a
        // portal 18.6 never made — would hide behind them. Ask the portal map directly.
        // `exec_bind_message` raises before `CreatePortal`, so this is `34000`.
        let d_again = describe_unnamed_portal(&mut c);
        assert_eq!(
            sqlstate(&d_again).as_deref(),
            Some("34000"),
            "a refused Bind must create no portal: {d_again:?}"
        );
    }

    #[test]
    // Same message-per-binding shape as the Bind gate test above.
    #[allow(clippy::many_single_char_names)]
    fn a_parameterized_exit_statement_is_refused_at_bind_and_the_block_stays_failed() {
        // The third shape the Bind test's comment lists, and the one an
        // `IsTransactionExitStmt`-only gate would leave behind. `exec_bind_message`
        // exempts an exit statement **and then un-exempts it** when the prepared
        // statement has parameters (`|| numParams != 0`, reachable because the
        // `numParams != psrc->num_params` `08P01` just above it has already made the
        // two counts equal). Postgres's own comment gives the reason: it "can't risk
        // calling user-defined I/O functions" in an aborted transaction. An exit
        // statement can only acquire a parameter by *declaring* one at Parse — there is
        // no `$N` to write in `ROLLBACK` — so this is exactly a `Parse` carrying a
        // param-type OID, which is what the raw v3 probe sends here.
        //
        // Measured against 18.6 after `BEGIN; SELECT * FROM <absent>` — the `was`
        // column on the in-memory B+tree, `--storage memory` and stripped-B+tree legs
        // identically, the `is` column re-measured on the in-memory B+tree:
        //
        //   phase                          was                          is / 18.6
        //   Parse `ROLLBACK` oids=[int4]   ParseComplete                ParseComplete
        //   Describe the statement         ParamDesc(1)[23] + NoData    same
        //   Bind, one text parameter       BindComplete                 25P02
        //   Describe the portal            NoData                       34000
        //   Execute                        CommandComplete(ROLLBACK)    34000
        //   the next statement             ran; ReadyForQuery Idle      25P02, still [E]
        //
        // The last row is the sharp one and is why the block's state is asserted: it
        // used to **end** here and not there, so a client's next statement ran against
        // a connection the two engines disagreed about.
        //
        // **The relation asserted ahead of the pins is the forbidder**, and its scope is
        // worth stating exactly. No assertion confined to *this* shape can separate an
        // `IsTransactionExitStmt`-only gate (the exit-only gate) from the whole condition,
        // because on this shape the exit-only gate and the pre-gate engine answer
        // identically. An assertion
        // spanning this shape and a **non**-exit one is not so confined, and separates
        // them at once, because the two fixes disagree about whether the two shapes may
        // part company:
        //
        //   build                                      comment-only  this shape   equal
        //   before the gate                            BindComplete  BindComplete  yes
        //   gate on `IsTransactionExitStmt` only        25P02         BindComplete  NO
        //   the whole `exec_bind_message` condition     25P02         25P02         yes
        //   18.6                                       25P02         25P02         yes
        //
        // Every row of that table was measured by mutation. The exit-only gate
        // (`!is_transaction_exit_stmt(&stmt.query)` alone) leaves the comment-only pin
        // green and reddens only the relation; `!stmt.param_types.is_empty()` alone —
        // gate on parameters, ignore exit-ness — reddens the relation from the other
        // side. (A gate on the aborted state alone satisfies the relation; that one is
        // rejected by
        // `a_transaction_exit_statement_parsed_while_aborted_still_ends_the_block`,
        // whose bare `COMMIT` Bind it would refuse.) Keep it first: a panicking pin
        // pre-empts everything after it, so a re-pin that reorders this file silently
        // loses the discrimination.
        let mut c = aborted_conn();

        // The non-exit shape, driven here so the two can be compared inside one test: a
        // comment-only statement, which pglet's Parse gate exempts through
        // `Parsed::Empty` and whose NULL `raw_parse_tree` makes Bind refuse it.
        let pc = handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: "cmt".into(),
                query: "/* x */".into(),
                param_types: Vec::new(),
            },
        );
        assert!(
            matches!(pc.as_slice(), [BackendMessage::ParseComplete]),
            "{pc:?}"
        );
        let b_cmt = handle_message(&mut c, bind_unnamed("cmt", Vec::new()));
        // Unconditional `Sync`: a refused Bind puts the router in skip-until-`Sync`, and
        // everything below has to be driven whatever this one answered — without it the
        // rest would be swallowed, and the failure would name a missing reply instead of
        // the shape split.
        handle_message(&mut c, FrontendMessage::Sync);

        let p = parse(&mut c, "ROLLBACK", vec![Some(TypeOid::Int4)]);
        assert!(
            matches!(p.as_slice(), [BackendMessage::ParseComplete]),
            "{p:?}"
        );

        // Describe the **statement**, before any Bind: `ParameterDescription` is the
        // reply that shows the declared OID became a parameter, and `numParams != 0` is
        // about exactly this count — so without it a zero-parameter `ROLLBACK` would be a
        // different shape wearing this one's name and the test would not notice. It is
        // also the control on Describe's own gate: a `ROLLBACK` returns no data, so
        // `psrc->resultDesc` is NULL and neither engine refuses it. Measured: emitting an
        // empty `ParameterDescription` from the Describe arm reddens this assertion and
        // nothing else in this crate's suite.
        let d_stmt = describe_synced(&mut c, DescribeKind::Statement, "");

        let b = handle_message(&mut c, bind_unnamed("", vec![Some(b"1".to_vec())]));
        handle_message(&mut c, FrontendMessage::Sync);
        let d = describe_synced(&mut c, DescribeKind::Portal, "");
        let e = execute_unnamed(&mut c);
        handle_message(&mut c, FrontendMessage::Sync);
        let status = c.transaction_status();

        // The forbidder, first.
        assert_eq!(
            bind_shape(&b_cmt),
            bind_shape(&b),
            "Bind must not distinguish a parameterized exit statement from a \
             comment-only one: 18.6 refuses both (a NULL `raw_parse_tree` fails the \
             exit-statement test, and a declared parameter un-exempts an exit statement), \
             so a gate here has to move them together — comment-only {b_cmt:?}, \
             parameterized exit {b:?}",
        );

        assert_eq!(
            d_stmt
                .iter()
                .find_map(|m| match m {
                    BackendMessage::ParameterDescription { param_types } =>
                        Some(param_types.clone()),
                    _ => None,
                })
                .as_deref(),
            Some([23].as_slice()),
            "the declared int4 OID must reach the client as this statement's one \
             parameter — 18.6 answers the same: {d_stmt:?}"
        );
        assert!(has_no_data(&d_stmt), "{d_stmt:?}");

        assert_eq!(
            bind_shape(&b),
            "ERROR 25P02",
            "an exit statement that declares parameters loses the exemption: {b:?}"
        );
        assert_eq!(
            sqlstate(&d).as_deref(),
            Some("34000"),
            "the refused Bind created no portal: {d:?}"
        );
        assert_eq!(sqlstate(&e).as_deref(), Some("34000"), "{e:?}");
        assert_eq!(
            status,
            TransactionStatus::Failed,
            "nothing ended the block — the Bind refused before the ROLLBACK could run",
        );
    }

    /// Bind a **named** portal from a named prepared statement, no parameters.
    fn bind_named(portal: &str, statement: &str) -> FrontendMessage {
        FrontendMessage::Bind {
            portal: portal.into(),
            statement: statement.into(),
            param_formats: Vec::new(),
            params: Vec::new(),
            result_formats: Vec::new(),
        }
    }

    /// Describe a named portal or prepared statement, then `Sync` so the next phase is
    /// read on its own rather than through skip-until-`Sync`.
    ///
    /// The `Sync` is load-bearing and its reply is not: the `Sync` arm clears
    /// `skip_until_sync` and returns `ReadyForQuery` unconditionally, so asserting on it
    /// would assert nothing.
    fn describe_synced(c: &mut Connection, kind: DescribeKind, name: &str) -> Vec<BackendMessage> {
        let r = handle_message(
            c,
            FrontendMessage::Describe {
                kind,
                name: name.to_owned(),
            },
        );
        handle_message(c, FrontendMessage::Sync);
        r
    }

    fn has_row_description(msgs: &[BackendMessage]) -> bool {
        msgs.iter()
            .any(|m| matches!(m, BackendMessage::RowDescription { .. }))
    }

    fn has_no_data(msgs: &[BackendMessage]) -> bool {
        msgs.iter().any(|m| matches!(m, BackendMessage::NoData))
    }

    /// A Describe reply reduced to a comparable shape, so two Describes can be compared
    /// for *what kind of answer* they gave rather than for a value.
    fn describe_shape(msgs: &[BackendMessage]) -> String {
        match sqlstate(msgs) {
            Some(code) => format!("ERROR {code}"),
            None if has_row_description(msgs) => "row description".to_owned(),
            None if has_no_data(msgs) => "no data".to_owned(),
            None => format!("{msgs:?}"),
        }
    }

    #[test]
    fn describe_in_an_aborted_block_gates_only_the_data_returning() {
        // Describe's own aborted-block gate, which is **not** the check Parse or Bind
        // has. `exec_describe_portal_message` and `exec_describe_statement_message` each
        // carry their own `IsAbortedTransactionBlockState()` test, discriminated by
        // `portal->tupDesc` / `psrc->resultDesc` — *does it return data* — rather than by
        // `parsetree_list != NIL` or `IsTransactionExitStmt`. `postgres.c` gives the
        // reason in both comments: refusing every Describe "might break the ability of
        // some clients to issue COMMIT or ROLLBACK commands, if they use code that
        // blindly Describes whatever it does".
        //
        // Reaching that gate needs a portal that exists on **both** engines, so all
        // three are bound while the block is still healthy — which puts Bind's own gate
        // behind them, exactly as the Bind test's second shape puts the Parse gate
        // behind it. Measured against 18.6 with a raw v3 client that syncs and reads
        // each phase — the `was` column on the in-memory B+tree, `--storage memory`
        // and stripped-B+tree legs, byte-identical on all three; the `is` column
        // re-measured on the in-memory B+tree:
        //
        //   phase                                        was                   is / 18.6
        //   Describe portal of `SELECT i4`               RowDescription(1)     25P02
        //   Describe portal of `INSERT … RETURNING i4`   RowDescription(1)     25P02
        //   Describe portal of `INSERT` (no RETURNING)   NoData                NoData
        //   Describe statement of `SELECT i4`            ParamDesc + RowDesc   25P02
        //   Describe statement of `INSERT … RETURNING`   ParamDesc + RowDesc   25P02
        //   Describe statement of `INSERT`               ParamDesc + NoData    same
        //
        // The `INSERT … RETURNING` rows are what make the stated mechanism observable.
        // Without them every data-returning row is a `SELECT`, so nothing separates
        // *gate what returns data* from *gate the `SELECT` kind*, and the assertions
        // below would be asserting a mechanism the test cannot see.
        //
        // **Two of the six rows already agreed before the gate**, which is what makes
        // this a forbidder and not only a conformance check. Three wrong fixes are on
        // offer and each breaks a relation asserted ahead of the per-row assertions, so
        // it fails on that relation's message. Measured by mutation:
        //
        //   * hand Describe the check the `Parse` arm has — the obvious reading of "the
        //     aborted block is gated one phase later than we thought" — and the two
        //     no-data rows become divergences, on precisely the COMMIT/ROLLBACK-recovery
        //     path that Postgres comment protects. Relation 1 fires;
        //   * gate the `SELECT` kind instead of the result descriptor, and the
        //     `INSERT … RETURNING` rows part company with the `SELECT` rows. **This is
        //     the one that used to land green**: with the two `SELECT` rows re-pinned to
        //     `25P02` and no RETURNING rows, the test passes; Relation 2 is what turns
        //     it red;
        //   * patch one `exec_describe_*` counterpart and not the other, and a
        //     statement's portal leg parts company with its statement leg. Relation 3 is
        //     a **message** improvement — the per-row assertion on the other leg reddens
        //     on that fix anyway; the relation names the leg mismatch instead of
        //     reporting an unexpected row description.
        let mut c = conn();
        assert!(!has_error(&query(&mut c, "CREATE TABLE dab (i4 int)")));
        assert!(!has_error(&query(&mut c, "BEGIN")));

        let ps = parse_named(&mut c, "ps", "SELECT i4 FROM dab");
        assert!(!has_error(&ps), "{ps:?}");
        let pr = parse_named(&mut c, "pr", "INSERT INTO dab VALUES (1) RETURNING i4");
        assert!(!has_error(&pr), "{pr:?}");
        let pi = parse_named(&mut c, "pi", "INSERT INTO dab VALUES (1)");
        assert!(!has_error(&pi), "{pi:?}");

        // Bound while healthy, so `exec_bind_message`'s gate never fires on either
        // engine and all three portals exist when the block goes bad. Nothing is ever
        // executed, so neither INSERT writes a row.
        for name in ["ps", "pr", "pi"] {
            let b = handle_message(&mut c, bind_named(name, name));
            assert!(
                matches!(b.as_slice(), [BackendMessage::BindComplete]),
                "{name}: {b:?}"
            );
        }

        let failed = query(&mut c, "SELECT * FROM dab_definitely_absent");
        assert!(has_error(&failed), "{failed:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);

        // `Sync` between the phases, as the raw v3 probe does: an `ErrorResponse` from
        // any one of them arms skip-until-`Sync`, so without this the refusals would
        // leave the rest reading `[]` and the failure would name a swallowed message
        // instead of the refusal that caused it.
        let d_portal_select = describe_synced(&mut c, DescribeKind::Portal, "ps");
        let d_portal_returning = describe_synced(&mut c, DescribeKind::Portal, "pr");
        let d_portal_insert = describe_synced(&mut c, DescribeKind::Portal, "pi");
        let d_stmt_select = describe_synced(&mut c, DescribeKind::Statement, "ps");
        let d_stmt_returning = describe_synced(&mut c, DescribeKind::Statement, "pr");
        let d_stmt_insert = describe_synced(&mut c, DescribeKind::Statement, "pi");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);

        // Relation 1 — the agreements. These are what a Parse-shaped gate would break.
        // Ordering matters: a panicking assertion pre-empts every later one, so the
        // relations go first and a rewrite that reorders this file silently loses the
        // discrimination.
        assert!(
            !has_error(&d_portal_insert) && has_no_data(&d_portal_insert),
            "18.6 answers NoData for a no-data portal in a failed block — its gate reads \
             `portal->tupDesc`, not the statement kind: {d_portal_insert:?}"
        );
        assert!(
            !has_error(&d_stmt_insert) && has_no_data(&d_stmt_insert),
            "18.6 answers ParameterDescription + NoData for a no-data statement in a \
             failed block — its gate reads `psrc->resultDesc`: {d_stmt_insert:?}"
        );

        // Relation 2 — the discriminator between *returns data* and *is a `SELECT`*.
        // Invariant under the correct gate (both refuse), violated by a kind-shaped one.
        // Two-sided, so a gate that refuses everything breaks it too.
        assert_eq!(
            describe_shape(&d_portal_returning),
            describe_shape(&d_portal_select),
            "Describe must not distinguish `INSERT … RETURNING` from `SELECT`: both \
             carry a result descriptor, which is the only thing 18.6's gate reads",
        );
        assert_eq!(
            describe_shape(&d_stmt_returning),
            describe_shape(&d_stmt_select),
            "same, for the statement leg: 18.6 reads `psrc->resultDesc`, not the kind",
        );
        assert_ne!(
            describe_shape(&d_portal_returning),
            describe_shape(&d_portal_insert),
            "…and it must still distinguish it from an `INSERT` with no RETURNING, or \
             the gate has stopped reading the result descriptor at all",
        );

        // Relation 3 — the two legs of one statement move together. `portal->tupDesc`
        // and `psrc->resultDesc` are the same fact seen from two functions, so a fix
        // that patches one `exec_describe_*` counterpart and not the other is wrong
        // here. It is the weakest of the three: the per-row assertions below catch that
        // fix anyway, so what this adds is the message.
        for (portal, stmt, what) in [
            (&d_portal_select, &d_stmt_select, "SELECT"),
            (&d_portal_returning, &d_stmt_returning, "INSERT … RETURNING"),
            (&d_portal_insert, &d_stmt_insert, "INSERT"),
        ] {
            assert_eq!(
                has_error(portal),
                has_error(stmt),
                "{what}: the portal and statement Describe legs must agree on whether \
                 they refuse — portal {portal:?}, statement {stmt:?}",
            );
        }

        // The four data-returning rows: `25P02`, agreeing with 18.6. The statement leg's
        // refusal carries **no** `ParameterDescription` — 18.6's gate precedes the
        // parameter reply, and this one does too.
        for (msgs, what) in [
            (&d_portal_select, "SELECT portal"),
            (&d_portal_returning, "INSERT … RETURNING portal"),
            (&d_stmt_select, "SELECT statement"),
            (&d_stmt_returning, "INSERT … RETURNING statement"),
        ] {
            assert_eq!(
                sqlstate(msgs).as_deref(),
                Some("25P02"),
                "{what}: a data-returning Describe is refused in a failed block: {msgs:?}"
            );
            assert!(
                !msgs
                    .iter()
                    .any(|m| matches!(m, BackendMessage::ParameterDescription { .. })),
                "{what}: the gate precedes the ParameterDescription: {msgs:?}"
            );
        }
    }

    // --- a zero-column result set is a result set (EDGES row 59) ---------------

    /// Build the zero-column relation both engines accept: create, seed two rows, drop
    /// the last column. The other route to one — a `RETURNING`-less data-modifying CTE
    /// the outer query references — 18.6 refuses outright (`EDGES.md` row 61).
    fn zero_column_relation(c: &mut Connection) {
        for sql in [
            "CREATE TABLE zc (x INT)",
            "INSERT INTO zc VALUES (1), (2)",
            "ALTER TABLE zc DROP COLUMN x",
        ] {
            let r = query(c, sql);
            assert!(!has_error(&r), "{sql}: {r:?}");
        }
    }

    /// The number of fields in a reply's `RowDescription`, or `None` when it carried
    /// none — the distinction the whole class turns on, which `has_row_description`
    /// alone cannot make.
    fn row_desc_width(msgs: &[BackendMessage]) -> Option<usize> {
        msgs.iter().find_map(|m| match m {
            BackendMessage::RowDescription { columns } => Some(columns.len()),
            _ => None,
        })
    }

    fn data_row_widths(msgs: &[BackendMessage]) -> Vec<usize> {
        msgs.iter()
            .filter_map(|m| match m {
                BackendMessage::DataRow { values } => Some(values.len()),
                _ => None,
            })
            .collect()
    }

    /// `SELECT *` over a zero-column relation on the **simple** leg: an empty
    /// `RowDescription` and both rows, not a bare `CommandComplete`.
    ///
    /// The differential fixture (`select/zero_column_relation_star.sql`) covers this
    /// same leg against 18.6 and is the primary guard; this pins the message *shape*
    /// the fixture reads only through its decoded rows, and the `fields=0` descriptor
    /// no JSON row can show.
    #[test]
    fn simple_query_describes_a_zero_column_result_set_and_sends_its_rows() {
        let mut c = conn();
        zero_column_relation(&mut c);

        let r = query(&mut c, "SELECT * FROM zc");
        assert!(!has_error(&r), "{r:?}");
        assert_eq!(
            row_desc_width(&r),
            Some(0),
            "18.6 sends RowDescription(fields=0) here; dropping it because the column \
             list is empty took the rows with it (EDGES row 59): {r:?}"
        );
        assert_eq!(
            data_row_widths(&r),
            vec![0, 0],
            "both rows go out, zero fields wide, under the `SELECT 2` tag: {r:?}"
        );
        assert_eq!(tags(&r), vec!["SELECT 2".to_owned()], "{r:?}");
    }

    /// The **extended** leg of the same statement, and the control that keeps the fix
    /// from becoming "always describe a `RowDescription`".
    ///
    /// The fixture harness cannot hold this half: its binary leg decodes the `DataRow`s
    /// that arrive after a `NoData` just as happily as the ones after a
    /// `RowDescription(0)`, so it was green on both sides of the fix. Only a raw-frame
    /// assertion sees it.
    #[test]
    fn describe_answers_a_zero_column_row_description_but_no_data_without_a_result_set() {
        let mut c = conn();
        zero_column_relation(&mut c);
        assert!(!has_error(&query(&mut c, "CREATE TABLE zi (a INT)")));

        for (name, sql) in [
            ("zs", "SELECT * FROM zc"),
            ("zn", "INSERT INTO zi VALUES (1)"),
        ] {
            let p = parse_named(&mut c, name, sql);
            assert!(!has_error(&p), "{name}: {p:?}");
            let b = handle_message(&mut c, bind_named(name, name));
            assert!(
                matches!(b.as_slice(), [BackendMessage::BindComplete]),
                "{name}: {b:?}"
            );
        }

        // The zero-column `SELECT *`: a result set, so both legs describe it.
        for kind in [DescribeKind::Statement, DescribeKind::Portal] {
            let d = describe_synced(&mut c, kind, "zs");
            assert_eq!(
                row_desc_width(&d),
                Some(0),
                "{kind:?}: 18.6 answers RowDescription(fields=0) — a zero-column result \
                 set is not the absence of one (EDGES row 59): {d:?}"
            );
        }

        // The control: a DML with no `RETURNING` returns no result set, so both legs
        // still answer `NoData`. A fix that keyed on "has a plan" rather than on
        // result-set-ness would move this, and 18.6 does not.
        for kind in [DescribeKind::Statement, DescribeKind::Portal] {
            let d = describe_synced(&mut c, kind, "zn");
            assert!(
                has_no_data(&d) && row_desc_width(&d).is_none(),
                "{kind:?}: a RETURNING-less INSERT returns no result set, so 18.6 \
                 answers NoData: {d:?}"
            );
        }
    }

    /// `SHOW` is described like any other result-returning statement, on both legs
    /// and in both block states.
    ///
    /// It never reaches the planner, so its shape has no plan-side producer and the
    /// fallback answered `NoResultSet`. Two things followed, and both are measured
    /// here against 18.6 with a raw v3 client (`Parse` + `Bind` while healthy, a
    /// failing `Query`, then `Describe` + `Sync` per leg):
    ///
    ///   phase                                    was                 is / 18.6
    ///   Describe statement of `SHOW` (healthy)   ParamDesc + NoData  ParamDesc + RowDescription(1)
    ///   Describe portal of `SHOW` (healthy)      NoData              RowDescription(1)
    ///   Describe statement of `SHOW` (aborted)   ParamDesc + NoData  25P02
    ///   Describe portal of `SHOW` (aborted)      NoData              25P02
    ///   Describe statement of `SET` (aborted)    ParamDesc + NoData  same
    ///   Describe portal of `SET` (aborted)       NoData              same
    ///
    /// The healthy rows are the silent half: `Execute` then sent a `DataRow` no field
    /// descriptor covered, and a binary client — which is what the extended protocol's
    /// clients all are — decoded it as a row with **no columns**, dropping the value.
    /// The differential fixtures hold that half now (a successful `SHOW` fixture is
    /// only readable on the binary leg once the descriptor is right, and both legs
    /// compare the declared type OID live against 18.6); this test holds the aborted
    /// rows, which no fixture can express.
    ///
    /// The `SET` rows are the control, and they are what a fix keyed on "runs outside
    /// the planner" rather than on *returns a result set* would break: `SET` really
    /// does return no result set, and 18.6 describes it in a failed block for the
    /// reason `postgres.c` gives — refusing every Describe would break a client that
    /// blindly Describes whatever it issues, including its own `ROLLBACK`.
    #[test]
    fn describe_of_a_show_carries_its_row_description_and_is_gated_when_aborted() {
        let mut c = conn();
        for (name, sql) in [
            ("dsh", "SHOW server_version"),
            ("dst", "SET client_encoding = 'UTF8'"),
        ] {
            let p = parse_named(&mut c, name, sql);
            assert!(!has_error(&p), "{name}: {p:?}");
            let b = handle_message(&mut c, bind_named(name, name));
            assert!(
                matches!(b.as_slice(), [BackendMessage::BindComplete]),
                "{name}: {b:?}"
            );
        }
        // Healthy: the `SHOW` carries a one-column descriptor on both legs, the `SET`
        // none.
        for kind in [DescribeKind::Statement, DescribeKind::Portal] {
            let d = describe_synced(&mut c, kind, "dsh");
            assert_eq!(
                row_desc_width(&d),
                Some(1),
                "{kind:?}: 18.6 answers RowDescription(1) for a SHOW; a NoData here                  sends the value in a DataRow no descriptor covers: {d:?}"
            );
            let d = describe_synced(&mut c, kind, "dst");
            assert!(
                has_no_data(&d) && row_desc_width(&d).is_none(),
                "{kind:?}: SET returns no result set: {d:?}"
            );
        }

        // Aborted: the gate reads the shape, so the `SHOW` is now refused and the
        // `SET` still described.
        let mut c = conn();
        assert!(!has_error(&query(&mut c, "BEGIN")));
        for (name, sql) in [
            ("ash", "SHOW server_version"),
            ("ast", "SET client_encoding = 'UTF8'"),
        ] {
            let p = parse_named(&mut c, name, sql);
            assert!(!has_error(&p), "{name}: {p:?}");
            let b = handle_message(&mut c, bind_named(name, name));
            assert!(
                matches!(b.as_slice(), [BackendMessage::BindComplete]),
                "{name}: {b:?}"
            );
        }
        assert!(has_error(&query(
            &mut c,
            "SELECT * FROM ash_definitely_absent"
        )));
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);
        for kind in [DescribeKind::Statement, DescribeKind::Portal] {
            let d = describe_synced(&mut c, kind, "ash");
            assert_eq!(
                sqlstate(&d).as_deref(),
                Some("25P02"),
                "{kind:?}: a SHOW returns data, so 18.6's aborted-block Describe gate                  refuses it: {d:?}"
            );
            let d = describe_synced(&mut c, kind, "ast");
            assert!(
                !has_error(&d) && has_no_data(&d),
                "{kind:?}: SET returns no result set, so 18.6 still describes it in a                  failed block: {d:?}"
            );
        }
    }

    /// The aborted-block `Describe` gate reads *does this statement return a result
    /// set* — `psrc->resultDesc` is a pointer, not a width.
    ///
    /// Measured on 18.6 with a raw v3 client, both statements prepared and bound while
    /// the block was healthy: the zero-column `SELECT *` is refused `25P02`, the
    /// `RETURNING`-less `INSERT` is described (`ParameterDescription` + `NoData`). The
    /// column-count reading answered the first one `NoData` too, letting a client
    /// describe in a failed block where 18.6 refuses.
    #[test]
    fn aborted_block_describe_refuses_a_zero_column_result_set() {
        let mut c = conn();
        zero_column_relation(&mut c);
        assert!(!has_error(&query(&mut c, "CREATE TABLE zi (a INT)")));
        assert!(!has_error(&query(&mut c, "BEGIN")));

        for (name, sql) in [
            ("zs", "SELECT * FROM zc"),
            ("zn", "INSERT INTO zi VALUES (1)"),
        ] {
            let p = parse_named(&mut c, name, sql);
            assert!(!has_error(&p), "{name}: {p:?}");
            let b = handle_message(&mut c, bind_named(name, name));
            assert!(
                matches!(b.as_slice(), [BackendMessage::BindComplete]),
                "{name}: {b:?}"
            );
        }

        let failed = query(&mut c, "SELECT * FROM zc_definitely_absent");
        assert!(has_error(&failed), "{failed:?}");
        assert_eq!(c.transaction_status(), TransactionStatus::Failed);

        for kind in [DescribeKind::Statement, DescribeKind::Portal] {
            let refused = describe_synced(&mut c, kind, "zs");
            assert_eq!(
                sqlstate(&refused).as_deref(),
                Some("25P02"),
                "{kind:?}: the gate reads `resultDesc != NULL`, and a zero-column \
                 `SELECT *` has one: {refused:?}"
            );
            let described = describe_synced(&mut c, kind, "zn");
            assert!(
                !has_error(&described) && has_no_data(&described),
                "{kind:?}: a RETURNING-less INSERT stays describable in a failed block \
                 — the COMMIT/ROLLBACK-recovery path 18.6's comment protects: \
                 {described:?}"
            );
        }
    }

    // --- RowDescription wire metadata: typmod + base-table provenance ----------

    fn row_desc_fields(msgs: &[BackendMessage]) -> Vec<FieldDescription> {
        msgs.iter()
            .find_map(|m| match m {
                BackendMessage::RowDescription { columns } => Some(columns.clone()),
                _ => None,
            })
            .expect("a SELECT emits a RowDescription")
    }

    #[test]
    fn row_description_reports_typmod_and_base_table_provenance() {
        // The differential fixture harness never compares RowDescription typmod /
        // table_oid / attnum, so this pins them directly: a driver's
        // getPrecision/getScale (and updatable-ResultSet provenance) read them.
        let mut c = conn();
        assert!(!has_error(&query(
            &mut c,
            "CREATE TABLE tt (amt numeric(10,2), nm varchar(20), body text, i int)"
        )));
        let msgs = query(&mut c, "SELECT amt, nm, body, i FROM tt");
        let cols = row_desc_fields(&msgs);
        assert_eq!(cols.len(), 4, "{cols:?}");

        // atttypmod: numeric(10,2) → ((10<<16)|2)+4 = 655366; varchar(20) → 24;
        // text / int carry no modifier → -1 (matches Postgres pg_attribute).
        assert_eq!(cols[0].type_modifier, 655_366, "numeric(10,2) typmod");
        assert_eq!(cols[1].type_modifier, 24, "varchar(20) typmod");
        assert_eq!(cols[2].type_modifier, -1, "text typmod");
        assert_eq!(cols[3].type_modifier, -1, "int typmod");

        // Base-table provenance: a real table OID + the 1-based attnum, propagated
        // through the projection.
        let table_oid = cols[0].table_oid;
        assert_ne!(table_oid, 0, "base-table column carries its table OID");
        for c in &cols {
            assert_eq!(c.table_oid, table_oid, "all columns from the same table");
        }
        assert_eq!(cols[0].column_id, 1, "amt is attnum 1");
        assert_eq!(cols[1].column_id, 2, "nm is attnum 2");
        assert_eq!(cols[2].column_id, 3, "body is attnum 3");
        assert_eq!(cols[3].column_id, 4, "i is attnum 4");
    }

    #[test]
    fn cast_expression_column_has_no_provenance() {
        // A computed/cast column is not a base-table column: no typmod, no
        // provenance (Postgres reports 0/-1 for an expression column).
        let mut c = conn();
        query(&mut c, "CREATE TABLE tp (amt numeric(10,2))");
        let msgs = query(&mut c, "SELECT amt + 1 AS s, amt::text AS a FROM tp");
        let cols = row_desc_fields(&msgs);
        for col in &cols {
            assert_eq!(
                col.type_modifier, -1,
                "computed column has no typmod: {col:?}"
            );
            assert_eq!(col.table_oid, 0, "computed column has no table: {col:?}");
            assert_eq!(col.column_id, 0, "computed column has no attnum: {col:?}");
        }
    }

    // --- startup ParameterStatus: the fields drivers branch on -----------------

    #[test]
    fn startup_reports_driver_parameter_status() {
        let mut c = conn();
        let msgs = handle_message(
            &mut c,
            FrontendMessage::Startup {
                version: pglet_wire::protocol::PROTOCOL_VERSION_3_0,
                params: vec![("user".to_string(), "postgres".to_string())],
            },
        );
        let params: std::collections::HashMap<&str, &str> = msgs
            .iter()
            .filter_map(|m| match m {
                BackendMessage::ParameterStatus { name, value } => {
                    Some((name.as_str(), value.as_str()))
                }
                _ => None,
            })
            .collect();
        // pglet is UTC-only, integer-datetimes-on, standard-conforming-strings-on,
        // so it asserts these fixed values a client branches on at startup.
        //
        // The **membership** is measured, not reasoned: 18.6's startup packet carries
        // `search_path` and `application_name` among the fifteen it sends — `search_path`
        // has carried `GUC_REPORT` since PostgreSQL 14. The six pglet omits
        // (`in_hot_standby`, `is_superuser`, `default_transaction_read_only`,
        // `scram_iterations`, `session_authorization`, `transaction_isolation`) are
        // ones it models no value for.
        //
        // `server_version` is here because no fixture can see the startup packet,
        // and because this string had a **second home**: it was typed out here and
        // again in `ShowVariable::report`, both saying `16.0`, so a client could
        // have been told two different releases by one connection. Both now render
        // `catalog::startup_parameters()`. Drivers and ORMs feature-gate on this
        // value, and it names the release pglet's differential corpus is asserted
        // against — the dialect target, not feature completeness.
        for (name, value) in [
            ("server_version", "18.6"),
            ("client_encoding", "UTF8"),
            ("server_encoding", "UTF8"),
            ("DateStyle", "ISO, MDY"),
            ("IntervalStyle", "postgres"),
            ("TimeZone", "UTC"),
            ("standard_conforming_strings", "on"),
            ("integer_datetimes", "on"),
            // `GUC_REPORT` since PostgreSQL 14, and 18.6 sends `"$user", public`
            // here — measured with a raw v3 client, which is the only way to see a
            // startup packet at all.
            ("search_path", "\"$user\", public"),
            // The packet's own value, echoed back: this connection sent none.
            ("application_name", ""),
        ] {
            assert_eq!(params.get(name), Some(&value), "ParameterStatus {name}");
        }
    }

    /// The startup packet's `application_name` is the session's value, and `SHOW`
    /// answers it.
    #[test]
    fn startup_seeds_the_session_application_name() {
        let mut c = conn();
        handle_message(
            &mut c,
            FrontendMessage::Startup {
                version: pglet_wire::protocol::PROTOCOL_VERSION_3_0,
                params: vec![
                    ("user".to_string(), "postgres".to_string()),
                    ("application_name".to_string(), "drizzle".to_string()),
                ],
            },
        );
        assert_eq!(c.settings.application_name(), "drizzle");
        let r = query(&mut c, "SHOW application_name");
        assert!(!has_error(&r), "{r:?}");
        assert_eq!(
            r.iter()
                .filter_map(|m| match m {
                    BackendMessage::DataRow { values } => values[0].clone(),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![b"drizzle".to_vec()],
            "{r:?}"
        );
    }

    /// The startup packet's `default_transaction_isolation` is the session's value and its
    /// `RESET` target — a block begun without naming a level runs at it — and a value that
    /// names no level refuses the connection `22023`, as 18.6's startup does.
    #[test]
    fn startup_seeds_the_session_default_isolation() {
        let startup = |value: &str| FrontendMessage::Startup {
            version: pglet_wire::protocol::PROTOCOL_VERSION_3_0,
            params: vec![
                ("user".to_string(), "postgres".to_string()),
                (
                    "default_transaction_isolation".to_string(),
                    value.to_string(),
                ),
            ],
        };
        let cell = |r: &[BackendMessage]| {
            r.iter()
                .find_map(|m| match m {
                    BackendMessage::DataRow { values } => values[0].clone(),
                    _ => None,
                })
                .map(|v| String::from_utf8_lossy(&v).into_owned())
        };
        let mut c = conn();
        handle_message(&mut c, startup("Repeatable Read"));
        let r = query(&mut c, "SHOW default_transaction_isolation");
        assert_eq!(cell(&r).as_deref(), Some("repeatable read"), "{r:?}");
        query(&mut c, "SET default_transaction_isolation = serializable");
        query(&mut c, "RESET default_transaction_isolation");
        query(&mut c, "BEGIN");
        let r = query(&mut c, "SHOW transaction_isolation");
        assert_eq!(cell(&r).as_deref(), Some("repeatable read"), "{r:?}");
        query(&mut c, "ROLLBACK");

        let mut c = conn();
        let r = handle_message(&mut c, startup("sometimes"));
        assert!(
            r.iter()
                .any(|m| matches!(m, BackendMessage::ErrorResponse { fields }
                if fields.iter().any(|(k, v)| *k == b'C' && v == "22023"))),
            "{r:?}"
        );
    }

    /// The startup packet's settings, as 18.6 reads them: `options` first — split by
    /// `pg_split_opts`, switches read by `getopt` — then the other keys in order, each checked
    /// as 18.6 checks it, the last assignment winning, and pglet's own refusals judged on the
    /// final values. Every answer below is 18.6's for the same packet over a raw v3 client
    /// (measured), except where pglet refuses a final value 18.6 honors — `client_encoding`
    /// other than UTF-8 (`22023`), `quote_all_identifiers` on (`0A000`, as its `SET`), a
    /// `TimeZone`, `DateStyle`, `IntervalStyle` or `standard_conforming_strings` other than the
    /// one pglet has (`0A000`, as its `SET`) and a
    /// `search_path` naming a schema pglet does not have (`0A000`, as its `set_config`) — and the
    /// `OPTIONS` key, which 18.6 refuses `42704` and pglet ignores (`EDGES.md` row 67). No
    /// fixture can hold this: the harness cannot write a startup packet.
    #[test]
    fn startup_reads_its_settings_as_18_6_does() {
        let start = |params: &[(&str, &str)]| {
            let mut c = conn();
            let mut all = vec![("user".to_string(), "postgres".to_string())];
            all.extend(
                params
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string())),
            );
            let msgs = handle_message(
                &mut c,
                FrontendMessage::Startup {
                    version: pglet_wire::protocol::PROTOCOL_VERSION_3_0,
                    params: all,
                },
            );
            let state = c.session.state;
            (c, state, sqlstate(&msgs))
        };
        let terminated = pglet_wire::SessionState::Terminated;
        let qai = "quote_all_identifiers";
        let long_dashes = format!("{}utf8", "-".repeat(60));
        let long_spaces = format!("utf8{}", " ".repeat(60));
        // byte 63 falls inside the 31st `é`: cut before it, as 18.6 cuts it, never inside
        let straddling = format!("ab{}utf8", "é".repeat(31));
        let straddling_option = format!("-c client_encoding={straddling}");
        let two_options: &[(&str, &str)] = &[
            (
                "options",
                "-c default_transaction_isolation=serializable -c application_name=first",
            ),
            ("options", "-c application_name=second"),
        ];
        for (params, code) in [
            (&[(qai, "on")][..], "0A000"),
            (&[("Quote_All_Identifiers", "yes")][..], "0A000"),
            (&[(qai, "maybe")][..], "22023"),
            (&[("options", "-c quote_all_identifiers=on")][..], "0A000"),
            (&[("options", "-cquote_all_identifiers=on")][..], "0A000"),
            (&[("options", "--quote-all-identifiers=on")][..], "0A000"),
            (&[("options", "-Ec quote_all_identifiers=on")][..], "0A000"),
            (&[("options", "-ec quote_all_identifiers=on")][..], "0A000"),
            (
                &[("options", "-c\x0bquote_all_identifiers=on")][..],
                "0A000",
            ),
            (
                &[(
                    "options",
                    "-c application_name=a\\ b -c quote_all_identifiers=1",
                )][..],
                "0A000",
            ),
            (
                &[("options", "-c quote_all_identifiers=maybe")][..],
                "22023",
            ),
            (&[("options", "-Ec client_encoding=LATIN1")][..], "22023"),
            (&[("options", "-c\x0bclient_encoding=LATIN1")][..], "22023"),
            // 18.6's own check of a value runs where the value is read, even one a later
            // assignment would override, and before any refusal of pglet's
            (
                &[(
                    "options",
                    "-c quote_all_identifiers=maybe -c quote_all_identifiers=off",
                )][..],
                "22023",
            ),
            (
                &[(
                    "options",
                    "-c client_encoding=NOSUCH -c client_encoding=UTF8",
                )][..],
                "22023",
            ),
            (
                &[(qai, "on"), ("options", "-c lock_timeout=abc")][..],
                "22023",
            ),
            (&[("options", "-c lock_timeout=abc")][..], "22023"),
            (
                &[("options", "-c default_transaction_isolation=bogus")][..],
                "22023",
            ),
            // what `getopt` refuses
            (&[("options", "-Z -c application_name=x")][..], "42601"),
            (&[("options", "-c application_name=x foo")][..], "42601"),
            (&[("options", "-c quote_all_identifiers")][..], "42601"),
            (&[("options", "--single")][..], "42601"),
            (&[("options", "-c statement_timeout=5 -c")][..], "42601"),
            // `mule_internal` is refused by 18.6 too, on the value the startup ends with
            (&[("client_encoding", "mule_internal")][..], "0A000"),
            (
                &[(
                    "options",
                    "-c client_encoding=UTF8 -c client_encoding=MULE-internal",
                )][..],
                "0A000",
            ),
            // an encoding name is truncated to 63 bytes before it is looked up
            (&[("client_encoding", long_dashes.as_str())][..], "22023"),
            (&[("client_encoding", straddling.as_str())][..], "22023"),
            (&[("options", straddling_option.as_str())][..], "22023"),
            // `search_path` gets its `set_config`'s verdict, on its final value (18.6 serves these)
            (&[("search_path", "nosuch")][..], "0A000"),
            (&[("options", "-c search_path=nosuch")][..], "0A000"),
            (
                &[
                    ("options", "-c search_path=public"),
                    ("search_path", "public, nosuch"),
                ][..],
                "0A000",
            ),
            // the last `options` key is the packet's (`ProcessStartupPacket` overwrites)
            (
                &[
                    ("options", "-c application_name=x"),
                    ("options", "-c quote_all_identifiers=on"),
                ][..],
                "0A000",
            ),
            // a parameter whose value pglet holds fixed gets its `SET`'s verdict, on its
            // final value (18.6 serves these)
            (&[("TimeZone", "America/New_York")][..], "0A000"),
            (&[("timezone", "GMT")][..], "0A000"),
            (&[("options", "-c timezone=Europe/Berlin")][..], "0A000"),
            (
                &[("options", "-c timezone=UTC"), ("TimeZone", "Asia/Tokyo")][..],
                "0A000",
            ),
            (&[("DateStyle", "German")][..], "0A000"),
            (&[("options", "-c datestyle=sql")][..], "0A000"),
            // `-e` is `datestyle=euro`
            (&[("options", "-e")][..], "0A000"),
            (&[("IntervalStyle", "iso_8601")][..], "0A000"),
            (&[("standard_conforming_strings", "off")][..], "0A000"),
            (
                &[("options", "-c standard_conforming_strings=maybe")][..],
                "22023",
            ),
        ] {
            let (_, state, code_seen) = start(params);
            assert_eq!(
                (state, code_seen),
                (terminated, Some(code.to_string())),
                "{params:?}"
            );
        }
        let show = |c: &mut Connection, what: &str| {
            query(c, &format!("SHOW {what}"))
                .iter()
                .find_map(|m| match m {
                    BackendMessage::DataRow { values } => values[0].clone(),
                    _ => None,
                })
                .map(|v| String::from_utf8_lossy(&v).into_owned())
        };
        // served, and each setting reaches the session through `options` as through its key
        for (params, setting, value) in [
            (&[(qai, "off")][..], "application_name", ""),
            (
                &[("options", "-d -cquote_all_identifiers=on")][..],
                "application_name",
                "",
            ),
            (
                &[(
                    "options",
                    "-c quote_all_identifiers=on -c quote_all_identifiers=off",
                )][..],
                "application_name",
                "",
            ),
            (
                &[("options", "-c quote_all_identifiers=on"), (qai, "off")][..],
                "application_name",
                "",
            ),
            (
                &[
                    ("options", "-c client_encoding=LATIN1"),
                    ("client_encoding", "UTF8"),
                ][..],
                "client_encoding",
                "UTF8",
            ),
            (
                &[(
                    "options",
                    "-c client_encoding=LATIN1 -c client_encoding=UTF8",
                )][..],
                "client_encoding",
                "UTF8",
            ),
            (
                &[("OPTIONS", "-c quote_all_identifiers=on")][..],
                "application_name",
                "",
            ),
            (&[("TimeZone", "UTC")][..], "TimeZone", "UTC"),
            (&[("timezone", "utc")][..], "TimeZone", "UTC"),
            (
                &[("options", "-c timezone=GMT"), ("TimeZone", "UTC")][..],
                "TimeZone",
                "UTC",
            ),
            (
                &[
                    ("options", "-e -c datestyle=iso"),
                    ("DateStyle", "ISO, MDY"),
                ][..],
                "DateStyle",
                "ISO, MDY",
            ),
            (&[("DateStyle", "ISO")][..], "DateStyle", "ISO, MDY"),
            (
                &[("IntervalStyle", "postgres")][..],
                "IntervalStyle",
                "postgres",
            ),
            (
                &[("standard_conforming_strings", "on")][..],
                "standard_conforming_strings",
                "on",
            ),
            (
                &[("options", "-c default_transaction_isolation=serializable")][..],
                "default_transaction_isolation",
                "serializable",
            ),
            (
                &[("options", "-c lock_timeout=123")][..],
                "lock_timeout",
                "123ms",
            ),
            (
                &[("options", "-c lock_timeout=5s"), ("lock_timeout", "7s")][..],
                "lock_timeout",
                "7s",
            ),
            (
                &[("options", "--application-name=hello")][..],
                "application_name",
                "hello",
            ),
            (
                &[("options", "-c application_name=a\\ b\\\\c")][..],
                "application_name",
                "a b\\c",
            ),
            (
                &[
                    ("options", "-c application_name=x"),
                    ("application_name", "k"),
                ][..],
                "application_name",
                "k",
            ),
            (
                &[
                    ("application_name", "k"),
                    ("options", "-c application_name=x"),
                ][..],
                "application_name",
                "k",
            ),
            (
                &[(
                    "options",
                    "-Eb -f s -t pa -P -D /tmp -c application_name=x --",
                )][..],
                "application_name",
                "x",
            ),
            (
                &[(
                    "options",
                    "-c client_encoding=mule_internal -c client_encoding=UTF8",
                )][..],
                "client_encoding",
                "UTF8",
            ),
            (
                &[("client_encoding", long_spaces.as_str())][..],
                "client_encoding",
                "UTF8",
            ),
            (
                &[
                    ("options", "-c search_path=nosuch"),
                    ("search_path", "public"),
                ][..],
                "application_name",
                "",
            ),
            (
                &[("search_path", "\"$user\", public")][..],
                "application_name",
                "",
            ),
            (&[("search_path", "")][..], "application_name", ""),
            // the last `options` key is the packet's: the first one's settings are dropped
            (two_options, "application_name", "second"),
            (
                two_options,
                "default_transaction_isolation",
                "read committed",
            ),
        ] {
            let (mut c, state, code) = start(params);
            assert_eq!(code, None, "{params:?}");
            assert_ne!(state, terminated, "{params:?}");
            assert_eq!(show(&mut c, setting).as_deref(), Some(value), "{params:?}");
        }
    }

    /// The startup packet's `client_encoding` is a **request**, and one pglet cannot
    /// serve refuses the connection.
    ///
    /// libpq, JDBC and asyncpg set `client_encoding` here rather than with a statement,
    /// so this — not `SET` — is the route the corruption actually takes: measured over a
    /// packet asking for `LATIN1`, 18.6 sends `63 61 66 e9` for `café` and pglet sends
    /// `63 61 66 c3 a9`, which that client renders `cafÃ©`. 18.6 answers an encoding it
    /// cannot serve with `FATAL 22023 invalid value for parameter "client_encoding":
    /// "…"` and closes; pglet answers the same shape and additionally refuses `LATIN1`,
    /// which 18.6 serves by converting and pglet cannot. No fixture can hold this: the
    /// harness cannot write a startup packet.
    #[test]
    fn startup_refuses_a_client_encoding_it_cannot_serve() {
        let start = |enc: &str| {
            let mut c = conn();
            let msgs = handle_message(
                &mut c,
                FrontendMessage::Startup {
                    version: pglet_wire::protocol::PROTOCOL_VERSION_3_0,
                    params: vec![
                        ("user".to_string(), "postgres".to_string()),
                        ("client_encoding".to_string(), enc.to_string()),
                    ],
                },
            );
            (c.session.state, msgs)
        };
        for enc in ["LATIN1", "NOSUCHENC", "WIN1252"] {
            let (state, msgs) = start(enc);
            assert_eq!(sqlstate(&msgs).as_deref(), Some("22023"), "{enc}: {msgs:?}");
            assert!(
                msgs.iter()
                    .any(|m| matches!(m, BackendMessage::ErrorResponse { fields }
                    if fields.iter().any(|(k, v)| *k == b'S' && v == "FATAL"))),
                "{enc} must be FATAL, not ERROR: {msgs:?}"
            );
            assert_eq!(
                state,
                pglet_wire::SessionState::Terminated,
                "{enc} must close the connection"
            );
        }
        // The servable ones still hand out a session. `SQL_ASCII` is "no conversion",
        // which is what pglet does — and what `pg_dump` of a `SQL_ASCII` database asks
        // for in its preamble.
        for enc in ["UTF8", "utf-8", "UNICODE", "SQL_ASCII", "sqlascii"] {
            let (state, msgs) = start(enc);
            assert!(!has_error(&msgs), "{enc}: {msgs:?}");
            assert_eq!(state, pglet_wire::SessionState::Ready, "{enc}");
        }
    }

    /// A `SET` that moves a reported parameter is followed by its `ParameterStatus`,
    /// and one that moves nothing is not.
    ///
    /// Postgres's `GUC_REPORT` rule, measured on 18.6 with a raw v3 client and
    /// reproduced here frame for frame:
    ///
    ///   statement                              18.6                     pglet
    ///   SET application_name TO 'probe'        CC SET, PS, Ready        same
    ///   SET application_name TO 'probe' again  CC SET, Ready            same
    ///   RESET application_name                 CC RESET, PS, Ready      same
    ///   SET statement_timeout = 0              CC SET, Ready            same
    ///   SET client_encoding TO 'UTF8'          CC SET, Ready            same
    ///
    /// pglet sent **no** `ParameterStatus` after any statement before this — it
    /// asserted eight values at connect and then never mentioned them again, so a
    /// pooler or driver that tracks `GUC_REPORT` parameters could not learn one had
    /// moved. `search_path` is the parameter 18.6 moves here that pglet does not:
    /// every value pglet accepts resolves the same way and its reported string is
    /// fixed, which is `EDGES.md`'s residual and not this rule's.
    #[test]
    fn a_set_that_moves_a_reported_parameter_reports_it() {
        let params_of = |msgs: &[BackendMessage]| -> Vec<(String, String)> {
            msgs.iter()
                .filter_map(|m| match m {
                    BackendMessage::ParameterStatus { name, value } => {
                        Some((name.clone(), value.clone()))
                    }
                    _ => None,
                })
                .collect()
        };
        let mut c = conn();
        assert_eq!(
            params_of(&query(&mut c, "SET application_name TO 'probe'")),
            vec![("application_name".to_string(), "probe".to_string())]
        );
        // On change only: the same value again announces nothing.
        assert!(
            params_of(&query(&mut c, "SET application_name TO 'probe'")).is_empty(),
            "a second identical SET must not report"
        );
        assert_eq!(
            params_of(&query(&mut c, "RESET application_name")),
            vec![("application_name".to_string(), String::new())]
        );
        // A parameter that is not reported, and one that is reported but unmoved.
        for sql in [
            "SET statement_timeout = 0",
            "SET client_encoding TO 'UTF8'",
            "SET search_path TO public",
            "SHOW server_version",
        ] {
            assert!(
                params_of(&query(&mut c, sql)).is_empty(),
                "{sql} must not report"
            );
        }
        // `DISCARD ALL` carries a `RESET ALL`, so it reports what that moved — 18.6
        // reports the same three it moved, measured.
        query(&mut c, "SET application_name TO 'probe'");
        assert_eq!(
            params_of(&query(&mut c, "DISCARD ALL")),
            vec![("application_name".to_string(), String::new())]
        );
    }

    // --- NoticeResponse: DROP … IF EXISTS of a missing object ------------------

    fn notice_messages(msgs: &[BackendMessage]) -> Vec<String> {
        msgs.iter()
            .filter_map(|m| match m {
                BackendMessage::NoticeResponse { fields } => fields
                    .iter()
                    .find(|(k, _)| *k == b'M')
                    .map(|(_, v)| v.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn drop_if_exists_missing_object_emits_notice_before_command_complete() {
        let mut c = conn();
        let msgs = query(&mut c, "DROP TABLE IF EXISTS nonexistent_xyz");
        assert!(
            !has_error(&msgs),
            "IF EXISTS skip is not an error: {msgs:?}"
        );
        assert_eq!(
            notice_messages(&msgs),
            vec!["table \"nonexistent_xyz\" does not exist, skipping".to_string()],
            "{msgs:?}"
        );
        // The NOTICE precedes the CommandComplete (Postgres ordering).
        let notice_pos = msgs
            .iter()
            .position(|m| matches!(m, BackendMessage::NoticeResponse { .. }));
        let cc_pos = msgs
            .iter()
            .position(|m| matches!(m, BackendMessage::CommandComplete { .. }));
        assert!(
            notice_pos < cc_pos,
            "NOTICE before CommandComplete: {msgs:?}"
        );
    }

    #[test]
    fn drop_table_existing_emits_no_notice() {
        // The positive control: dropping an existing table is silent (no NOTICE).
        let mut c = conn();
        query(&mut c, "CREATE TABLE present (id int)");
        let msgs = query(&mut c, "DROP TABLE IF EXISTS present");
        assert!(notice_messages(&msgs).is_empty(), "{msgs:?}");
    }

    /// A binary-format array's header names the **result column's** element type where its
    /// elements are sent as that type (`binary_array_header_and_element_bytes_are_one_type`
    /// holds the routes where they are not). A
    /// `timestamptz[]` built by `ARRAY[…]` holds `Value::Timestamp` elements, which
    /// `timestamp` shares, so the value alone records `timestamp`: its header said `1114`
    /// where 18.6's says `1184`, measured over a raw extended-protocol client for the
    /// constructor over a typed constant and over a column, a `COALESCE` and a subquery.
    /// The stored column and the two `timestamp[]` shapes are controls, and a `varchar[]`
    /// constructor, whose value records `text` (`25` before, `1043` on 18.6), is the same
    /// rule at another type.
    #[test]
    fn binary_array_header_carries_the_columns_element_type() {
        let mut c = conn();
        assert!(!has_error(&query(
            &mut c,
            "CREATE TABLE ah (tz timestamptz, tza timestamptz[], ts timestamp)"
        )));
        assert!(!has_error(&query(
            &mut c,
            "INSERT INTO ah VALUES ('2000-01-01 00:00:00+05', '{\"2000-01-01 00:00:00+05\"}', '2000-01-01')"
        )));
        handle_message(
            &mut c,
            FrontendMessage::Parse {
                name: String::new(),
                query: "SELECT ARRAY[timestamptz '2000-01-01 00:00:00+05'], ARRAY[tz], \
                        COALESCE(NULL, ARRAY[tz]), (SELECT ARRAY[tz] FROM ah), tza, \
                        ARRAY[ts], ARRAY[tz]::timestamp[], ARRAY['a'::varchar] FROM ah"
                    .into(),
                param_types: Vec::new(),
            },
        );
        handle_message(
            &mut c,
            FrontendMessage::Bind {
                portal: String::new(),
                statement: String::new(),
                param_formats: Vec::new(),
                params: Vec::new(),
                result_formats: vec![FormatCode::Binary],
            },
        );
        let r = handle_message(
            &mut c,
            FrontendMessage::Execute {
                portal: String::new(),
                max_rows: 0,
            },
        );
        let headers: Vec<u32> = r
            .iter()
            .find_map(|m| match m {
                BackendMessage::DataRow { values } => Some(
                    values
                        .iter()
                        .map(|v| {
                            let bytes = v.as_ref().expect("non-null array");
                            u32::from_be_bytes(bytes[8..12].try_into().expect("a header"))
                        })
                        .collect(),
                ),
                _ => None,
            })
            .expect("a row");
        assert_eq!(
            headers,
            [1184, 1184, 1184, 1184, 1184, 1114, 1114, 1043],
            "{r:?}"
        );
    }
    /// A binary-format array's header element OID and its element bytes are one type on
    /// each route below, including the routes where the result column's declared type is
    /// itself wrong.
    ///
    /// Each statement is `(sql, declared parameter type, bound text, header, element
    /// widths)`, the header and widths as 18.6 sends them, measured over a raw
    /// extended-protocol client. The first four put a Parse-declared `$N` under
    /// `ARRAY[…]`, a column typed the declared type's array as 18.6 types it, so the
    /// header names the declared element type. The fifth is a route whose column type
    /// pglet gets wrong: a `COALESCE` over a `NULL` `int4` operand and an `int8` one is typed
    /// `int4[]` (18.6 `int8[]`), and a header read from the column alone names `int4`
    /// over `int8` bytes there. A header read from the value alone is what the last three
    /// refuse — `timestamp` under `timestamptz` elements, `text` under a `varchar`
    /// constructor's, and `text` for an all-`NULL` `int4` array.
    #[test]
    fn binary_array_header_and_element_bytes_are_one_type() {
        // (sql, declared parameter type, bound text, header, element widths)
        type Case<'a> = (&'a str, Option<TypeOid>, &'a str, u32, &'a [i32]);
        let mut c = conn();
        for sql in [
            "CREATE TABLE hw (i int4, b int8)",
            "INSERT INTO hw VALUES (NULL, 7)",
        ] {
            assert!(!has_error(&query(&mut c, sql)), "{sql}");
        }
        let cases: [Case<'_>; 8] = [
            ("SELECT ARRAY[$1]", Some(TypeOid::Int4), "5", 23, &[4]),
            ("SELECT ARRAY[$1]", Some(TypeOid::Int2), "5", 21, &[2]),
            (
                "SELECT ARRAY[$1]",
                Some(TypeOid::Jsonb),
                "{\"a\": 1}",
                3802,
                &[9],
            ),
            (
                "SELECT ARRAY[$1]",
                Some(TypeOid::Timestamptz),
                "2000-01-01 00:00:00+05",
                1184,
                &[8],
            ),
            (
                "SELECT ARRAY[COALESCE(i + 0, b + 0)] FROM hw",
                None,
                "",
                20,
                &[8],
            ),
            (
                "SELECT ARRAY[timestamptz '2000-01-01 00:00:00+05']",
                None,
                "",
                1184,
                &[8],
            ),
            ("SELECT ARRAY['a'::varchar]", None, "", 1043, &[1]),
            ("SELECT ARRAY[NULL::int4]", None, "", 23, &[-1]),
        ];
        let mut wrong = Vec::new();
        for (sql, declared, bound, want_header, want_widths) in cases {
            handle_message(
                &mut c,
                FrontendMessage::Parse {
                    name: String::new(),
                    query: sql.into(),
                    param_types: declared.map(Some).into_iter().collect(),
                },
            );
            handle_message(
                &mut c,
                FrontendMessage::Bind {
                    portal: String::new(),
                    statement: String::new(),
                    param_formats: Vec::new(),
                    params: declared
                        .map(|_| Some(bound.as_bytes().to_vec()))
                        .into_iter()
                        .collect(),
                    result_formats: vec![FormatCode::Binary],
                },
            );
            let r = handle_message(
                &mut c,
                FrontendMessage::Execute {
                    portal: String::new(),
                    max_rows: 0,
                },
            );
            let bytes = r
                .iter()
                .find_map(|m| match m {
                    BackendMessage::DataRow { values } => values[0].clone(),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{sql}: a non-NULL array: {r:?}"));
            let int = |at: usize| i32::from_be_bytes(bytes[at..at + 4].try_into().expect("an int"));
            let header = u32::from_be_bytes(bytes[8..12].try_into().expect("a header"));
            let mut widths = Vec::new();
            let mut at = 20; // ndim, has-nulls, element OID, one (length, lower bound)
            while at < bytes.len() {
                let width = int(at);
                widths.push(width);
                at += 4 + usize::try_from(width).unwrap_or(0);
            }
            if (header, widths.as_slice()) != (want_header, want_widths) {
                wrong.push(format!("{sql} {declared:?}: {header} {widths:?}"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }
}
