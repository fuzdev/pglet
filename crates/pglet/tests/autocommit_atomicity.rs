//! Auto-commit statement atomicity: a multi-row `INSERT`/`UPDATE`/`COPY` that
//! violates a constraint partway through must roll back its earlier rows, not
//! leave them applied.
//!
//! Postgres executes each statement atomically even outside an explicit
//! transaction — a mid-batch `duplicate key` aborts the whole statement, so none
//! of its rows survive. pglet's auto-commit write path used to apply each row
//! straight to the live tree, so a later row's error left the earlier rows
//! permanently committed (a data-integrity bug). The fix brackets an auto-commit
//! write in an implicit storage transaction (the same pending-buffer machinery an
//! explicit `BEGIN` uses), so the rows stage and publish atomically at commit and
//! a failure rolls the buffer back.
//!
//! Each check asserts the surviving state via **both** a full scan (`ORDER BY id`)
//! and an **indexed** lookup (`WHERE id = …` / `WHERE u = …`, served by the PK /
//! UNIQUE index), so an index-vs-heap desync (a heap row rolled back but a dangling
//! index entry left behind, or vice versa) would show as a disagreement.
//!
//! The differential SQL-fixture harness can't express "run a failing statement,
//! THEN assert the post-error state" in one `@test`, so this lives as a Rust
//! integration test; the ground-truth outcomes were confirmed against real
//! Postgres.

// A test harness: `.expect()`/`panic!` surface a setup failure or an unexpected cell
// shape loudly right here.
#![allow(clippy::expect_used, clippy::panic)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::types::Value;

/// Coerce an integer `Value` (`int4`/`int8`) to `i64` for assertions.
fn as_i64(v: &Value) -> i64 {
    match v {
        Value::Int2(n) => i64::from(*n),
        Value::Int4(n) => i64::from(*n),
        Value::Int8(n) => *n,
        other => panic!("expected an integer value, got {other:?}"),
    }
}

/// The `id` column of every row, ascending — a full heap scan (no index).
fn scan_ids(db: &mut Database) -> Vec<i64> {
    let mut results = db
        .execute("SELECT id FROM t ORDER BY id")
        .expect("scan should succeed");
    results
        .remove(0)
        .rows
        .iter()
        .map(|row| as_i64(&row[0]))
        .collect()
}

/// Run a single scalar-count query and return the count. Used with a `WHERE`
/// clause that the planner serves from an index, so it probes the index rather
/// than the heap.
fn count(db: &mut Database, sql: &str) -> i64 {
    let mut results = db.execute(sql).expect("count should succeed");
    as_i64(&results.remove(0).rows[0][0])
}

/// Seed a fresh in-memory btree db with `t(id pk, u unique, v)` holding (1,10,a)
/// and (2,20,b).
fn seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t(id INT PRIMARY KEY, u INT UNIQUE, v TEXT)")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1,10,'a'),(2,20,'b')")
        .expect("seed");
    db
}

/// Assert the table is exactly the untouched seed — via full scan AND both indexes.
fn assert_untouched_seed(db: &mut Database) {
    assert_eq!(
        scan_ids(db),
        vec![1, 2],
        "full scan must show only the seed rows"
    );
    // Indexed probes: the rolled-back rows must not be reachable by either index.
    assert_eq!(
        count(db, "SELECT count(*) FROM t WHERE id = 3"),
        0,
        "PK index desync"
    );
    assert_eq!(
        count(db, "SELECT count(*) FROM t WHERE id = 4"),
        0,
        "PK index desync"
    );
    assert_eq!(
        count(db, "SELECT count(*) FROM t WHERE u = 30"),
        0,
        "UNIQUE index desync"
    );
    assert_eq!(
        count(db, "SELECT count(*) FROM t WHERE u = 999"),
        0,
        "UNIQUE index desync"
    );
    // Both seed rows still reachable by index with their original values.
    assert_eq!(count(db, "SELECT count(*) FROM t WHERE id = 1"), 1);
    assert_eq!(count(db, "SELECT count(*) FROM t WHERE u = 10"), 1);
}

#[test]
fn multi_row_insert_pk_violation_rolls_back_earlier_rows() {
    let mut db = seeded();

    // Row (3,…) inserts, row (1,…) collides on the PK — Postgres aborts the whole
    // statement, so neither (3,…) nor (4,…) may survive.
    let err = db
        .execute("INSERT INTO t VALUES (3,30,'c'),(1,40,'d'),(4,50,'e')")
        .expect_err("a mid-batch duplicate key must fail the statement");
    assert_eq!(err.sqlstate(), "23505", "expected unique_violation");

    assert_untouched_seed(&mut db);
}

#[test]
fn multi_row_insert_unique_violation_rolls_back_earlier_rows() {
    let mut db = seeded();

    // Row (5,50,…) inserts, row (6,10,…) collides on the UNIQUE(u) index (u=10 is
    // seed row 1's) — the whole statement aborts.
    let err = db
        .execute("INSERT INTO t VALUES (5,50,'x'),(6,10,'y')")
        .expect_err("a mid-batch unique(u) violation must fail the statement");
    assert_eq!(err.sqlstate(), "23505");

    assert_eq!(scan_ids(&mut db), vec![1, 2]);
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM t WHERE id = 5"),
        0,
        "PK index desync"
    );
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM t WHERE u = 50"),
        0,
        "UNIQUE index desync"
    );
}

#[test]
fn multi_row_update_unique_violation_rolls_back_earlier_rows() {
    let mut db = seeded();

    // id=1 → u=999 (ok), then id=2 → u=999 collides with the pending update of
    // id=1 — the whole statement aborts, so id=1 must keep its original u=10.
    let err = db
        .execute("UPDATE t SET u=999 WHERE id IN (1,2)")
        .expect_err("a mid-batch unique collision must fail the UPDATE");
    assert_eq!(err.sqlstate(), "23505");

    // Full scan: both rows present.
    assert_eq!(scan_ids(&mut db), vec![1, 2]);
    // Indexed: id=1's u is still 10 (NOT 999), and no dangling u=999 entry.
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM t WHERE u = 10"),
        1,
        "id=1 must keep u=10"
    );
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM t WHERE u = 20"),
        1,
        "id=2 must keep u=20"
    );
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM t WHERE u = 999"),
        0,
        "no partial update / index desync"
    );
    // Cross-check the actual stored value of id=1.
    let mut r = db.execute("SELECT u FROM t WHERE id = 1").unwrap();
    assert_eq!(as_i64(&r.remove(0).rows[0][0]), 10);
}

#[test]
fn copy_bad_line_rolls_back_earlier_lines() {
    let mut db = seeded();

    // Line 1 (7,…) valid, line 2 (1,…) duplicates the seed PK — the whole COPY
    // aborts (Postgres COPY is all-or-nothing), so line 1 must not survive.
    let data = b"7\t70\tp\n1\t80\tq\n8\t90\tr\n";
    let err = db
        .copy_in("COPY t FROM STDIN", data)
        .expect_err("a bad line mid-COPY must fail the ingest");
    assert_eq!(err.sqlstate(), "23505");

    assert_eq!(scan_ids(&mut db), vec![1, 2]);
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM t WHERE id = 7"),
        0,
        "PK index desync"
    );
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM t WHERE u = 70"),
        0,
        "UNIQUE index desync"
    );
}

// ---------------------------------------------------------------------------
// Positive checks: read-your-own-writes within the statement, and the happy
// path, still work through the implicit-transaction wrap.
// ---------------------------------------------------------------------------

#[test]
fn copy_all_valid_lines_commit() {
    let mut db = seeded();
    let n = db
        .copy_in("COPY t FROM STDIN", b"7\t70\tp\n8\t80\tq\n")
        .expect("a clean COPY must succeed");
    assert_eq!(n, 2);
    assert_eq!(scan_ids(&mut db), vec![1, 2, 7, 8]);
    // Reachable by index too (index maintained on the committed publish).
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE id = 7"), 1);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE u = 80"), 1);
}

#[test]
fn on_conflict_sees_pending_row_from_same_statement() {
    let mut db = seeded();

    // Both rows carry id=10: the first inserts (staged), the second must find the
    // pending row through the arbiter (PK) index — read-your-own-writes — and DO
    // NOTHING. Result: exactly one id=10 row, with the first row's u=100.
    db.execute("INSERT INTO t VALUES (10,100,'a'),(10,101,'b') ON CONFLICT (id) DO NOTHING")
        .expect("ON CONFLICT DO NOTHING must succeed");

    assert_eq!(scan_ids(&mut db), vec![1, 2, 10]);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE id = 10"), 1);
    let mut r = db.execute("SELECT u FROM t WHERE id = 10").unwrap();
    assert_eq!(
        as_i64(&r.remove(0).rows[0][0]),
        100,
        "first pending row wins"
    );
}

#[test]
fn insert_select_from_same_table_does_not_feed_back() {
    let mut db = seeded();

    // Reads the two seed rows and inserts shifted copies; the just-inserted rows
    // must NOT feed back into the source scan (no Halloween loop). Result: 4 rows.
    db.execute("INSERT INTO t SELECT id+100, u+100, v FROM t WHERE id <= 2")
        .expect("INSERT … SELECT from the same table must succeed");

    assert_eq!(scan_ids(&mut db), vec![1, 2, 101, 102]);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE id = 101"), 1);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE u = 120"), 1);
}

#[test]
fn intra_statement_duplicate_still_caught_and_atomic() {
    let mut db = seeded();

    // Two rows with the same id and no ON CONFLICT: the duplicate must be caught
    // (23505) AND nothing added (both rolled back).
    let err = db
        .execute("INSERT INTO t VALUES (20,200,'a'),(20,201,'b')")
        .expect_err("an intra-statement duplicate must fail loud");
    assert_eq!(err.sqlstate(), "23505");

    assert_eq!(scan_ids(&mut db), vec![1, 2]);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE id = 20"), 0);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE u = 200"), 0);
}

#[test]
fn happy_path_multi_row_insert_commits() {
    let mut db = seeded();
    db.execute("INSERT INTO t VALUES (3,30,'c'),(4,40,'d')")
        .expect("a clean multi-row insert must succeed");
    assert_eq!(scan_ids(&mut db), vec![1, 2, 3, 4]);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE id = 3"), 1);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE u = 40"), 1);
}

// ---------------------------------------------------------------------------
// DDL + durability: the wrap must not break auto-commit CREATE/INSERT persistence.
// ---------------------------------------------------------------------------

#[test]
fn autocommit_ddl_and_write_persist_across_reopen() {
    let dir = ScratchDir::new("atomicity");
    {
        let mut db = Database::open(&dir.path).expect("open file-backed db");
        // Plain auto-commit DDL (wrapped in begin/commit) must still persist.
        db.execute("CREATE TABLE t(id INT PRIMARY KEY, u INT UNIQUE, v TEXT)")
            .expect("create");
        db.execute("INSERT INTO t VALUES (1,10,'a'),(2,20,'b')")
            .expect("insert");
        // A failed statement rolls back and leaves the durable state consistent.
        assert!(
            db.execute("INSERT INTO t VALUES (3,30,'c'),(1,40,'d')")
                .is_err(),
            "the mid-batch dup must fail"
        );
    }
    // Reopen: the table + committed rows are present, the rolled-back row is not.
    let mut db = Database::open_existing(&dir.path).expect("reopen");
    assert_eq!(
        scan_ids(&mut db),
        vec![1, 2],
        "committed rows survive; rolled-back row absent"
    );
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE id = 3"), 0);
    assert_eq!(count(&mut db, "SELECT count(*) FROM t WHERE id = 1"), 1);
}

/// The operator spellings pglet's lexer once could not read, each in operator position.
///
/// 18.6's scanner lexes every one as a generic `Op`, and so does pglet's
/// (`lexer::TokenKind::Op`); what each answers is its **analysis**: `42883` for an
/// operator that does not exist, `42725` for `&` over two `unknown`s (pglet answers
/// `42883` there — it resolves no candidate list), and a row for the three 18.6
/// implements and pglet does not.
const GENERIC_OPS: [(&str, &str); 7] = [
    ("1 ` 1", "42883"),
    ("1 ! 1", "42883"),
    ("'a' & 'b'", "42883"),
    ("~ 'a'::text", "42883"),
    ("1 ~~~ 1", "42883"),
    ("1 # 1", "42883"),
    ("1 & 1", "42883"),
];

/// A query string whose later piece names an operator pglet implements nothing for runs
/// the pieces before it as pglet runs the pieces before **any** failing statement:
/// per-statement autocommit, which keeps what 18.6's implicit transaction rolls back
/// (`EDGES.md` row 68). The failing piece's own answer is 18.6's `42883`.
///
/// Measured on 18.6 over a raw v3 client, one simple `Query` each ending in
/// `SELECT <x>`: `INSERT (1)` keeps none there and 1 here; `BEGIN; INSERT (1); COMMIT`
/// keeps 1 on both; `INSERT (1); COMMIT; INSERT (2)` keeps 1 there and 1, 2 here;
/// `INSERT (1); ROLLBACK; INSERT (2)` keeps none there and 1, 2 here — the `ROLLBACK`
/// ends no block on pglet, where it rolls 18.6's implicit one back; and
/// `BEGIN; INSERT (1)` leaves the block aborted with nothing kept on both. The control
/// is `SELECT 1/0`, an execution error, which keeps exactly the same rows here as each
/// of these — that is what makes this row 68's divergence and not a rule of its own.
#[test]
fn a_later_piece_naming_an_operator_pglet_lacks_runs_the_pieces_before_it() {
    let forms: [(&str, &[i64], pglet::TransactionStatus); 6] = [
        (
            "INSERT INTO q VALUES (1)",
            &[1],
            pglet::TransactionStatus::Idle,
        ),
        (
            "BEGIN; INSERT INTO q VALUES (1); COMMIT",
            &[1],
            pglet::TransactionStatus::Idle,
        ),
        (
            "INSERT INTO q VALUES (1); COMMIT; INSERT INTO q VALUES (2)",
            &[1, 2],
            pglet::TransactionStatus::Idle,
        ),
        (
            "INSERT INTO q VALUES (1); BEGIN; INSERT INTO q VALUES (2); COMMIT; \
             INSERT INTO q VALUES (3)",
            &[1, 2, 3],
            pglet::TransactionStatus::Idle,
        ),
        (
            "INSERT INTO q VALUES (1); ROLLBACK; INSERT INTO q VALUES (2)",
            &[1, 2],
            pglet::TransactionStatus::Idle,
        ),
        (
            "BEGIN; INSERT INTO q VALUES (1)",
            &[],
            pglet::TransactionStatus::Failed,
        ),
    ];
    // The control runs the same forms: an ordinary execution error, whose rows are
    // this rule's whole content.
    for (later, sqlstate) in GENERIC_OPS.into_iter().chain([("1/0", "22012")]) {
        for (before, kept, status) in forms {
            let mut db = Database::with_btree();
            db.execute("CREATE TABLE q (k INT)").expect("create");
            let err = db
                .execute(&format!("{before}; SELECT {later}"))
                .expect_err(later);
            assert_eq!(err.sqlstate(), sqlstate, "{before}; {later}");
            assert_eq!(db.transaction_status(), status, "{before}; {later}");
            if status == pglet::TransactionStatus::Failed {
                db.execute("ROLLBACK").expect("rollback");
            }
            let rows: Vec<i64> = db
                .execute("SELECT k FROM q ORDER BY k")
                .expect("rows")
                .remove(0)
                .rows
                .iter()
                .map(|row| as_i64(&row[0]))
                .collect();
            assert_eq!(rows, kept, "{before}; {later}");
        }
    }
    // A piece 18.6's **raw parse** refuses is the other rule, unchanged: pglet parses
    // each piece in turn, so the pieces before it run and commit where 18.6 runs
    // nothing at all (`EDGES.md` row 175). A lone backtick is such a piece now — it
    // lexes, and the grammar refuses what follows it.
    for later in ["`", "1 ! 1 1", "(1 ! 1"] {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE q (k INT)").expect("create");
        let err = db
            .execute(&format!("INSERT INTO q VALUES (1); SELECT {later}"))
            .expect_err(later);
        assert_eq!(err.sqlstate(), "42601", "{later}");
        assert_eq!(count(&mut db, "SELECT count(*) FROM q"), 1, "{later}");
    }
    // The control: in the first piece the operator is that piece's own failure, and
    // nothing after it runs.
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE q (k INT)").expect("create");
    let err = db
        .execute("SELECT 1 & 1; INSERT INTO q VALUES (1)")
        .expect_err("first piece");
    assert_eq!(err.sqlstate(), "42883");
    assert_eq!(count(&mut db, "SELECT count(*) FROM q"), 0);
    // A piece every scanner lexes still runs ahead of a later failure as autocommit:
    // that is the per-statement autocommit `EDGES.md` row 68 records, not this rule.
    let _ = db.execute("INSERT INTO q VALUES (1); SELECT nosuch");
    assert_eq!(count(&mut db, "SELECT count(*) FROM q"), 1);
}
