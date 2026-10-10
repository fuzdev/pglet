//! The transaction and statement clocks on the **embed** host, at the boundaries no
//! differential fixture can see.
//!
//! The `tests/fixtures/**/clock_*` corpus compares the relations between reads against
//! a live 18.6, over the native server. What it cannot reach is here:
//!
//! - **This host's own entry points** — `Database::execute` (a call carrying several
//!   statements included), `execute_bound`, `copy_in` and the `begin` method — each of
//!   which has to take the statement's start itself (`pglet_sql::builtins`' module doc).
//! - **Two connections on one thread**, where the clocks are ambient: a statement on
//!   one must not read the other's block.
//! - **An aborted block**, which the fixture harness cannot express.
//!
//! No test sleeps. Where time has to pass between two reads, [`tick`] asks the engine's
//! own `clock_timestamp()` until it has moved.

// A test harness: `.unwrap()`/`.expect()`/`panic!` surface a setup failure — or a
// statement that unexpectedly succeeded — loudly right here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::{Infinite, Value};
use time::OffsetDateTime;

/// A finite timestamp cell.
fn instant(value: &Value) -> OffsetDateTime {
    match value {
        Value::Timestamp(Infinite::Finite(at)) => *at,
        other => panic!("not a finite timestamp: {other:?}"),
    }
}

/// The first row of a one-statement query on `conn_id`, each cell a timestamp.
fn instants_as(db: &mut Database, conn_id: i32, sql: &str) -> Vec<OffsetDateTime> {
    let mut results = db.execute_as(sql, conn_id).expect(sql);
    results.remove(0).rows[0].iter().map(instant).collect()
}

/// [`instants_as`] on connection 0.
fn instants(db: &mut Database, sql: &str) -> Vec<OffsetDateTime> {
    instants_as(db, 0, sql)
}

/// `now()` and `statement_timestamp()` as one statement on connection 0 reads them.
fn clocks(db: &mut Database) -> (OffsetDateTime, OffsetDateTime) {
    let read = instants(db, "SELECT now(), statement_timestamp()");
    (read[0], read[1])
}

/// The first cell of a one-statement query.
fn scalar(db: &mut Database, sql: &str) -> Value {
    db.execute(sql).expect(sql).remove(0).rows[0][0].clone()
}

/// Return once the wall clock has moved past where it stood on entry, so the next
/// statement's start is later than every instant read before the call.
fn tick(db: &mut Database) {
    let before = instants(db, "SELECT clock_timestamp()")[0];
    while instants(db, "SELECT clock_timestamp()")[0] <= before {}
}

/// Every backend the facade runs: the clocks are the session's, not the store's.
fn backends() -> [(&'static str, Database); 2] {
    [
        ("btree", Database::with_btree()),
        ("memory", Database::in_memory()),
    ]
}

/// Inside a block every statement reads the block's start as the transaction clock,
/// while the statement clock is each statement's own; outside one the two agree, and
/// the next transaction has a start of its own.
///
/// The discriminating assertions are the two equalities across a [`tick`]: a clock read
/// per call, or per statement, fails the first; a statement clock frozen with the
/// block fails the second.
#[test]
fn a_block_reads_its_start_and_each_statement_reads_its_own() {
    for (backend, mut db) in backends() {
        db.execute("BEGIN").unwrap();
        let (first_tx, first_stmt) = clocks(&mut db);
        tick(&mut db);
        let (second_tx, second_stmt) = clocks(&mut db);
        assert_eq!(first_tx, second_tx, "{backend}: one block, one start");
        assert!(
            second_stmt > first_stmt,
            "{backend}: each statement its own"
        );
        assert!(
            first_tx < first_stmt,
            "{backend}: the block began before its first statement"
        );
        // The other transaction-start spellings read the same instant.
        let family = instants(
            &mut db,
            "SELECT transaction_timestamp(), CURRENT_TIMESTAMP, LOCALTIMESTAMP",
        );
        assert_eq!(family, vec![first_tx; 3], "{backend}");
        db.execute("COMMIT").unwrap();

        tick(&mut db);
        let (auto_tx, auto_stmt) = clocks(&mut db);
        assert_eq!(
            auto_tx, auto_stmt,
            "{backend}: a statement is its transaction"
        );
        assert!(
            auto_tx > second_stmt,
            "{backend}: a new transaction, a new start"
        );
        tick(&mut db);
        let (next_tx, _) = clocks(&mut db);
        assert!(next_tx > auto_tx, "{backend}: autocommit statements differ");
    }
}

/// A savepoint, a `ROLLBACK TO` and a `RELEASE` are inside the block's transaction: none
/// of them moves its start.
#[test]
fn a_savepoint_does_not_move_the_blocks_start() {
    for (backend, mut db) in backends() {
        db.execute("BEGIN").unwrap();
        let (start, _) = clocks(&mut db);
        for control in ["SAVEPOINT a", "ROLLBACK TO a", "SAVEPOINT b", "RELEASE b"] {
            tick(&mut db);
            db.execute(control).unwrap();
            assert_eq!(clocks(&mut db).0, start, "{backend}: after {control}");
        }
        db.execute("ROLLBACK").unwrap();
    }
}

/// A block that aborted ends at its `ROLLBACK` — or at the `COMMIT` that rolls it back —
/// and the block after it reads a start of its own.
#[test]
fn the_block_after_an_aborted_one_has_its_own_start() {
    for (backend, mut db) in backends() {
        for exit in ["ROLLBACK", "COMMIT"] {
            db.execute("BEGIN").unwrap();
            let (aborted_start, _) = clocks(&mut db);
            assert_eq!(
                db.execute("SELECT 1/0").unwrap_err().sqlstate(),
                "22012",
                "{backend}"
            );
            db.execute(exit).unwrap();
            tick(&mut db);
            db.execute("BEGIN").unwrap();
            let (next_start, _) = clocks(&mut db);
            assert!(next_start > aborted_start, "{backend}: after {exit}");
            tick(&mut db);
            assert_eq!(clocks(&mut db).0, next_start, "{backend}: after {exit}");
            db.execute("ROLLBACK").unwrap();
        }
    }
}

/// One `execute` call is this host's client message: the statements it carries read one
/// statement start, and a transaction one of them begins starts at it — as the
/// statements of one simple-query message do on 18.6.
#[test]
fn the_statements_of_one_call_read_one_statement_start() {
    let mut db = Database::with_btree();
    let results = db
        .execute(
            "SELECT now(), statement_timestamp(); \
             SELECT now(), statement_timestamp(); \
             BEGIN; \
             SELECT now(), statement_timestamp(); \
             COMMIT; \
             SELECT now(), statement_timestamp()",
        )
        .unwrap();
    let reads: Vec<OffsetDateTime> = results
        .iter()
        .filter(|result| !result.rows.is_empty())
        .flat_map(|result| result.rows[0].iter().map(instant))
        .collect();
    assert_eq!(reads.len(), 8);
    assert_eq!(reads, vec![reads[0]; 8]);
}

/// `execute_bound` is an entry point of its own: a bound statement reads the block's
/// start inside a block, its own start outside one, and a second execution of the same
/// text reads a later one — nothing of the first execution's instant is kept.
#[test]
fn a_bound_statement_reads_the_same_clocks() {
    let mut db = Database::with_btree();
    let bound = |db: &mut Database| -> (OffsetDateTime, OffsetDateTime) {
        let result = db
            .execute_bound("SELECT now(), statement_timestamp(), $1::int", 0, |_| {
                Ok::<_, ()>(vec![Some(Value::Int4(1))])
            })
            .unwrap_or_else(|_| panic!("the bound statement runs"));
        (instant(&result.rows[0][0]), instant(&result.rows[0][1]))
    };

    let (first_tx, first_stmt) = bound(&mut db);
    assert_eq!(first_tx, first_stmt, "outside a block");
    tick(&mut db);
    let (second_tx, _) = bound(&mut db);
    assert!(second_tx > first_tx, "a second execution, a later instant");

    db.execute("BEGIN").unwrap();
    let (start, _) = clocks(&mut db);
    tick(&mut db);
    let (bound_tx, bound_stmt) = bound(&mut db);
    assert_eq!(bound_tx, start, "inside a block");
    assert!(bound_stmt > start);
    db.execute("ROLLBACK").unwrap();
}

/// `copy_in` is an entry point of its own: the rows one ingest writes share one
/// transaction-start and one statement-start instant, which are the block's start and
/// the ingest's own inside a block and one instant outside it.
#[test]
fn a_copy_stamps_its_rows_with_its_statements_clocks() {
    let mut db = Database::with_btree();
    db.execute(
        "CREATE TABLE cc (id INT, tx TIMESTAMPTZ DEFAULT now(), \
         stmt TIMESTAMPTZ DEFAULT statement_timestamp())",
    )
    .unwrap();
    let stamps = |db: &mut Database, low: i32| -> Vec<OffsetDateTime> {
        instants(
            db,
            &format!(
                "SELECT min(tx), max(tx), min(stmt), max(stmt) FROM cc WHERE id >= {low} AND id < {low} + 10"
            ),
        )
    };

    let (before, _) = clocks(&mut db);
    tick(&mut db);
    db.copy_in("COPY cc (id) FROM STDIN", b"1\n2\n3\n").unwrap();
    let auto = stamps(&mut db, 0);
    assert_eq!(auto, vec![auto[0]; 4], "outside a block: one instant");
    assert!(auto[0] > before, "the ingest's own start");

    db.execute("BEGIN").unwrap();
    let (start, _) = clocks(&mut db);
    tick(&mut db);
    db.copy_in("COPY cc (id) FROM STDIN", b"11\n12\n13\n")
        .unwrap();
    let block = stamps(&mut db, 10);
    assert_eq!(block[..2], [start; 2], "inside a block: the block's start");
    assert_eq!(block[2], block[3], "one statement start");
    assert!(block[2] > start);
    db.execute("COMMIT").unwrap();
}

/// The `begin` method opens a block with no `BEGIN` statement: the call is where the
/// transaction starts.
#[test]
fn the_begin_method_starts_the_transaction_clock() {
    let mut db = Database::with_btree();
    let (before, _) = clocks(&mut db);
    tick(&mut db);
    db.begin(0).unwrap();
    tick(&mut db);
    let (first, first_stmt) = clocks(&mut db);
    assert!(first > before, "the block started at the call");
    assert!(first < first_stmt, "and before its first statement");
    tick(&mut db);
    assert_eq!(clocks(&mut db).0, first);
    db.rollback(0).unwrap();
}

/// The clocks are ambient, and two connections of one `Database` run on one thread: a
/// statement on a connection with no block reads its own start, and the block's
/// connection still reads the block's after it.
#[test]
fn another_connections_statement_does_not_read_the_block() {
    let mut db = Database::with_btree();
    db.execute_as("BEGIN", 1).unwrap();
    let start = instants_as(&mut db, 1, "SELECT now()")[0];
    tick(&mut db);
    let other = instants_as(&mut db, 2, "SELECT now(), statement_timestamp()");
    assert_eq!(other[0], other[1], "the other connection is in no block");
    assert!(other[0] > start);
    assert_eq!(
        instants_as(&mut db, 1, "SELECT now()")[0],
        start,
        "the block's connection reads its start again"
    );
    db.execute_as("ROLLBACK", 1).unwrap();
}

/// The mirror: `clock_timestamp()` is read at each call. Over the rows of one statement
/// it moves while `now()` and `statement_timestamp()` stand, and it is never before
/// either.
#[test]
fn the_wall_clock_still_moves_inside_a_statement() {
    for (backend, mut db) in backends() {
        db.execute("CREATE TABLE d (x INT)").unwrap();
        db.execute("INSERT INTO d VALUES (0), (1), (2), (3), (4), (5), (6), (7)")
            .unwrap();
        db.execute(
            "CREATE TABLE w (tx TIMESTAMPTZ DEFAULT now(), \
             stmt TIMESTAMPTZ DEFAULT statement_timestamp(), \
             clk TIMESTAMPTZ DEFAULT clock_timestamp(), id INT)",
        )
        .unwrap();
        db.execute(
            "INSERT INTO w (id) SELECT a.x * 512 + b.x * 64 + c.x * 8 + e.x \
             FROM d a, d b, d c, d e",
        )
        .unwrap();
        assert_eq!(
            scalar(
                &mut db,
                "SELECT count(DISTINCT tx) = 1 AND count(DISTINCT stmt) = 1 \
                 AND count(DISTINCT clk) > 1 AND min(clk) >= min(stmt) FROM w"
            ),
            Value::Bool(true),
            "{backend}"
        );
    }
}

/// A statement's start is taken as the call arrives, before the statement is parsed —
/// so a `clock_timestamp()` the statement goes on to evaluate is later, as it is on 18.6.
///
/// **It discriminates in a release build only** (`cargo test --release`): a start taken
/// where the statement first reads it sits in the same microsecond as that wall-clock
/// read there, and fails the first iterations, while an unoptimized build spends more
/// than a microsecond between the two reads and passes either way (both measured on one
/// idle host; the gap between the two reads turns on the host's speed as well as the
/// build).
#[test]
fn a_statements_start_is_before_its_wall_clock_reads() {
    let mut db = Database::with_btree();
    for _ in 0..2_000 {
        assert_eq!(
            scalar(&mut db, "SELECT now() < clock_timestamp()"),
            Value::Bool(true)
        );
    }
}
