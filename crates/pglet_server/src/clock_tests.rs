//! The transaction and statement clocks on the **native server**, at the boundaries no
//! differential fixture can see.
//!
//! The statement clock is read once per protocol message that executes
//! (`execute::execute_query_core`, and `copy::dispatch_copy` for a `COPY`), and an open
//! block carries its start in the connection's settings — `pglet_sql::builtins`' module
//! doc. The `tests/fixtures/**/clock_*` corpus compares the relations between reads
//! against a live 18.6; what it cannot express is here: a simple-query message carrying
//! several statements, a prepared statement executed twice, a `COPY … FROM STDIN`'s
//! ingest, an aborted block, and two connections served by one thread.
//!
//! No test sleeps. Where time has to pass between two reads, [`tick`] asks the engine's
//! own `clock_timestamp()` until it has moved.

use std::sync::Arc;

use pglet_wire::{BackendMessage, FrontendMessage};

use crate::connection::{Connection, SharedState};
use crate::handlers::handle_message;
use crate::test_support::{conn, sqlstate};

fn query(c: &mut Connection, sql: &str) -> Vec<BackendMessage> {
    handle_message(c, FrontendMessage::Query { sql: sql.into() })
}

/// Every cell of every `DataRow`, as text, in order.
fn cells(msgs: &[BackendMessage]) -> Vec<String> {
    msgs.iter()
        .filter_map(|m| match m {
            BackendMessage::DataRow { values } => Some(values),
            _ => None,
        })
        .flatten()
        .map(|v| {
            v.as_ref()
                .map_or_else(String::new, |b| String::from_utf8_lossy(b).into_owned())
        })
        .collect()
}

/// The cells one simple-query message answers.
fn read(c: &mut Connection, sql: &str) -> Vec<String> {
    let msgs = query(c, sql);
    assert_eq!(sqlstate(&msgs), None, "{sql}: {msgs:?}");
    cells(&msgs)
}

/// `now()` and `statement_timestamp()` as one statement reads them.
fn clocks(c: &mut Connection) -> (String, String) {
    let mut read = read(c, "SELECT now(), statement_timestamp()");
    let stmt = read.remove(1);
    (read.remove(0), stmt)
}

/// Whether the instant `later` renders is after the one `earlier` renders — decided by
/// the engine, since a rendered timestamp drops trailing zeros and does not sort as text.
fn after(c: &mut Connection, later: &str, earlier: &str) -> bool {
    read(
        c,
        &format!("SELECT '{later}'::timestamptz > '{earlier}'::timestamptz"),
    ) == ["t"]
}

/// Return once the wall clock has moved past where it stood on entry.
fn tick(c: &mut Connection) {
    let before = read(c, "SELECT clock_timestamp()").remove(0);
    while read(
        c,
        &format!("SELECT clock_timestamp() > '{before}'::timestamptz"),
    ) != ["t"]
    {}
}

/// Inside a block every statement reads the block's start; each reads its own statement
/// start; and the transaction after the block has a start of its own.
#[test]
fn a_block_reads_its_start_and_each_statement_reads_its_own() {
    let mut c = conn();
    query(&mut c, "BEGIN");
    let (first_tx, first_stmt) = clocks(&mut c);
    tick(&mut c);
    let (second_tx, second_stmt) = clocks(&mut c);
    assert_eq!(first_tx, second_tx, "one block, one start");
    assert!(after(&mut c, &second_stmt, &first_stmt));
    assert!(after(&mut c, &first_stmt, &first_tx));
    query(&mut c, "SAVEPOINT s");
    tick(&mut c);
    query(&mut c, "ROLLBACK TO s");
    assert_eq!(clocks(&mut c).0, first_tx, "a savepoint does not move it");
    query(&mut c, "COMMIT");

    tick(&mut c);
    let (auto_tx, auto_stmt) = clocks(&mut c);
    assert_eq!(auto_tx, auto_stmt, "a statement is its own transaction");
    assert!(after(&mut c, &auto_tx, &second_stmt));
}

/// One simple-query message is one statement start, and a transaction a statement of it
/// begins starts at that instant — measured on 18.6: `BEGIN; SELECT now() =
/// statement_timestamp()` sent as one message answers `t`, and as two answers `f`.
#[test]
fn one_simple_message_is_one_statement_start() {
    let mut c = conn();
    let one = read(
        &mut c,
        "SELECT now(), statement_timestamp(); SELECT now(), statement_timestamp(); \
         BEGIN; SELECT now(), statement_timestamp(); COMMIT; \
         SELECT now(), statement_timestamp()",
    );
    assert_eq!(one.len(), 8);
    assert_eq!(one, vec![one[0].clone(); 8]);

    tick(&mut c);
    query(&mut c, "BEGIN");
    tick(&mut c);
    assert_eq!(
        read(&mut c, "SELECT now() = statement_timestamp()"),
        ["f"],
        "sent as its own message, a block's statement starts after the block"
    );
    query(&mut c, "ROLLBACK");
}

/// A prepared statement holds no instant: two executions of one cached plan read two
/// statement starts, and inside a block both read the block's.
#[test]
fn a_prepared_statement_reads_a_new_instant_at_each_execution() {
    let mut c = conn();
    let parse = handle_message(
        &mut c,
        FrontendMessage::Parse {
            name: "clk".into(),
            query: "SELECT now(), statement_timestamp()".into(),
            param_types: Vec::new(),
        },
    );
    assert!(matches!(parse.as_slice(), [BackendMessage::ParseComplete]));
    let execute = |c: &mut Connection| -> (String, String) {
        handle_message(
            c,
            FrontendMessage::Bind {
                portal: String::new(),
                statement: "clk".into(),
                param_formats: Vec::new(),
                params: Vec::new(),
                result_formats: Vec::new(),
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
        let mut read = cells(&msgs);
        assert_eq!(read.len(), 2, "{msgs:?}");
        let stmt = read.remove(1);
        (read.remove(0), stmt)
    };

    // Outside a block the two clocks are not compared: 18.6 starts the transaction at an
    // earlier message than the `Execute` that restamps the statement, which this server
    // does not reproduce (`EDGES.md` row 292).
    let (first_tx, _) = execute(&mut c);
    tick(&mut c);
    let (second_tx, _) = execute(&mut c);
    assert!(
        after(&mut c, &second_tx, &first_tx),
        "a second execution, a later instant"
    );

    query(&mut c, "BEGIN");
    let (start, _) = clocks(&mut c);
    tick(&mut c);
    let (block_tx, block_stmt) = execute(&mut c);
    assert_eq!(block_tx, start, "inside a block");
    assert!(after(&mut c, &block_stmt, &start));
    tick(&mut c);
    assert_eq!(
        execute(&mut c).0,
        start,
        "and at the block's next execution"
    );
    query(&mut c, "ROLLBACK");
}

/// A `COPY … FROM STDIN` reaches neither `execute_query_core` nor the statement loop:
/// its rows are stamped by the clocks `dispatch_copy` read — one instant outside a
/// block, the block's start and the `COPY`'s own inside one.
#[test]
fn a_copy_stamps_its_rows_with_its_statements_clocks() {
    let mut c = conn();
    query(
        &mut c,
        "CREATE TABLE cc (id INT, tx TIMESTAMPTZ DEFAULT now(), \
         stmt TIMESTAMPTZ DEFAULT statement_timestamp())",
    );
    let copy = |c: &mut Connection, data: &[u8]| {
        let start = query(c, "COPY cc (id) FROM STDIN");
        assert!(
            matches!(start.as_slice(), [BackendMessage::CopyInResponse { .. }]),
            "{start:?}"
        );
        handle_message(c, FrontendMessage::CopyData(data.to_vec()));
        handle_message(c, FrontendMessage::CopyDone);
    };
    let stamps = |c: &mut Connection, low: i32| -> Vec<String> {
        read(
            c,
            &format!(
                "SELECT min(tx), max(tx), min(stmt), max(stmt) FROM cc \
                 WHERE id >= {low} AND id < {low} + 10"
            ),
        )
    };

    // The wall clock as the statement before the `COPY` read it — later than that
    // statement's own start, so a `COPY` that took no start of its own, and read the one
    // the thread last kept, stamps its rows earlier than this.
    let mark = read(&mut c, "SELECT clock_timestamp()").remove(0);
    copy(&mut c, b"1\n2\n3\n");
    let auto = stamps(&mut c, 0);
    assert_eq!(
        auto,
        vec![auto[0].clone(); 4],
        "outside a block: one instant"
    );
    assert!(after(&mut c, &auto[0], &mark), "the COPY's own start");

    query(&mut c, "BEGIN");
    let (start, _) = clocks(&mut c);
    tick(&mut c);
    let mark = read(&mut c, "SELECT clock_timestamp()").remove(0);
    copy(&mut c, b"11\n12\n13\n");
    let block = stamps(&mut c, 10);
    assert_eq!(block[..2], [start.clone(), start], "the block's start");
    assert_eq!(block[2], block[3], "one statement start");
    assert!(after(&mut c, &block[2], &mark), "the COPY's own start");
    query(&mut c, "COMMIT");
}

/// A block that aborted ends at its `ROLLBACK`, and the block after it reads a start of
/// its own.
#[test]
fn the_block_after_an_aborted_one_has_its_own_start() {
    let mut c = conn();
    query(&mut c, "BEGIN");
    let (aborted_start, _) = clocks(&mut c);
    assert_eq!(
        sqlstate(&query(&mut c, "SELECT 1/0")).as_deref(),
        Some("22012")
    );
    query(&mut c, "ROLLBACK");
    tick(&mut c);
    query(&mut c, "BEGIN");
    let (next_start, _) = clocks(&mut c);
    assert!(after(&mut c, &next_start, &aborted_start));
    tick(&mut c);
    assert_eq!(clocks(&mut c).0, next_start);
    query(&mut c, "ROLLBACK");
}

/// The clocks are ambient, and a thread can serve a second connection: a statement on a
/// connection with no block reads its own start, and the block's connection reads the
/// block's again after it.
#[test]
fn another_connections_statement_does_not_read_the_block() {
    let shared = Arc::new(SharedState::with_btree());
    let mut in_block = Connection::new(1, Arc::clone(&shared));
    let mut other = Connection::new(2, Arc::clone(&shared));
    query(&mut in_block, "BEGIN");
    let (start, _) = clocks(&mut in_block);
    tick(&mut other);
    let (other_tx, other_stmt) = clocks(&mut other);
    assert_eq!(other_tx, other_stmt, "the other connection is in no block");
    assert!(after(&mut other, &other_tx, &start));
    assert_eq!(clocks(&mut in_block).0, start);
    query(&mut in_block, "ROLLBACK");
}
