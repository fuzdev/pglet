//! An `ON CONFLICT DO UPDATE` statement that meets a row it already wrote is refused
//! `21000`, and what the statement wrote before the refusal does not stand.
//!
//! The fixtures under `tests/fixtures/insert/` hold the refusal's code and the statements
//! it must not reach. What no fixture can hold is the state the refused statement leaves —
//! the harness stops at a failing `@test` — nor a block the refusal aborts, so those live
//! here, each against the in-memory B+tree and a file-backed store. The ground truth is
//! 18.6, measured: over `t` holding `(1, 'a')`, `INSERT INTO t VALUES (1, 'x'), (1, 'y') ON
//! CONFLICT (k) DO UPDATE SET v = excluded.v` is `21000` with the hint below and leaves
//! `(1, 'a')`; the same statement inside a block leaves the block aborted (`25P02`) and a
//! `ROLLBACK` restores the table as the block found it.

// A test harness: `.expect()`/`panic!` surface a setup failure or an unexpected cell
// shape loudly right here.
#![allow(clippy::expect_used, clippy::panic)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::types::Value;

const HINT: &str = "Ensure that no rows proposed for insertion within the same command have duplicate \
     constrained values.";

/// Every `(k, v)` row of `t`, ordered by `k` — a full scan.
fn rows(db: &mut Database) -> Vec<(i64, String)> {
    let mut results = db
        .execute("SELECT k, v FROM t ORDER BY k")
        .expect("scan should succeed");
    results
        .remove(0)
        .rows
        .iter()
        .map(|row| match (&row[0], &row[1]) {
            (Value::Int4(k), Value::Text(v)) => (i64::from(*k), v.clone()),
            other => panic!("unexpected row shape {other:?}"),
        })
        .collect()
}

/// The `v` the primary-key index reaches for `k`, or `None` — an indexed probe, so a heap
/// rolled back under a dangling index entry (or the reverse) shows as a disagreement with
/// [`rows`].
fn indexed_v(db: &mut Database, k: i64) -> Option<String> {
    let mut results = db
        .execute(&format!("SELECT v FROM t WHERE k = {k}"))
        .expect("indexed read should succeed");
    let result = results.remove(0);
    match result.rows.as_slice() {
        [] => None,
        [row] => match &row[0] {
            Value::Text(v) => Some(v.clone()),
            other => panic!("unexpected cell {other:?}"),
        },
        more => panic!("two rows under one primary key: {more:?}"),
    }
}

fn seed(db: &mut Database) {
    db.execute("CREATE TABLE t (k INT PRIMARY KEY, v TEXT)")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1, 'a')").expect("seed");
}

/// The refusal itself: the code, 18.6's message and its hint.
fn assert_refused(err: &pglet::Error) {
    assert_eq!(err.sqlstate(), "21000", "{err}");
    assert_eq!(
        err.to_string(),
        "ON CONFLICT DO UPDATE command cannot affect row a second time"
    );
    assert_eq!(err.hint().as_deref(), Some(HINT));
}

fn refused_autocommit_statement_leaves_nothing(db: &mut Database) {
    seed(db);
    // The first proposal updates the seeded row, and a new key's proposal inserts beside
    // it; the refused third proposal meets the row the first one updated.
    let err = db
        .execute(
            "INSERT INTO t VALUES (1, 'x'), (2, 'n'), (1, 'y') \
             ON CONFLICT (k) DO UPDATE SET v = excluded.v",
        )
        .expect_err("the second touch of row 1 must be refused");
    assert_refused(&err);
    assert_eq!(rows(db), vec![(1, "a".to_string())]);
    assert_eq!(indexed_v(db, 1).as_deref(), Some("a"));
    assert_eq!(indexed_v(db, 2), None);
}

#[test]
fn refused_autocommit_statement_leaves_nothing_on_the_btree() {
    refused_autocommit_statement_leaves_nothing(&mut Database::with_btree());
}

#[test]
fn refused_autocommit_statement_leaves_nothing_on_a_file_store() {
    let dir = ScratchDir::new("on_conflict_twice");
    {
        let mut db = Database::open(&dir.path).expect("open");
        refused_autocommit_statement_leaves_nothing(&mut db);
    }
    let mut db = Database::open(&dir.path).expect("reopen");
    assert_eq!(rows(&mut db), vec![(1, "a".to_string())]);
}

/// Inside a block the refusal aborts it, as any error does: the next statement is `25P02`,
/// and the `ROLLBACK` takes back the block's earlier write too.
fn refusal_aborts_the_block(db: &mut Database) {
    seed(db);
    db.execute("BEGIN").expect("begin");
    // An earlier statement of the block writes row 1 — which a later statement may update:
    // the refusal is one statement's, not the block's.
    db.execute("INSERT INTO t VALUES (1, 'b') ON CONFLICT (k) DO UPDATE SET v = excluded.v")
        .expect("a row an earlier statement wrote is updated");
    db.execute("INSERT INTO t VALUES (3, 'c')")
        .expect("insert in block");
    let err = db
        .execute(
            "INSERT INTO t VALUES (3, 'x'), (3, 'y') \
             ON CONFLICT (k) DO UPDATE SET v = excluded.v",
        )
        .expect_err("the second touch of row 3 must be refused");
    assert_refused(&err);
    let aborted = db
        .execute("SELECT 1")
        .expect_err("the block is aborted after the refusal");
    assert_eq!(aborted.sqlstate(), "25P02", "{aborted}");
    db.execute("ROLLBACK").expect("rollback");
    assert_eq!(rows(db), vec![(1, "a".to_string())]);
    assert_eq!(indexed_v(db, 3), None);
}

#[test]
fn refusal_aborts_the_block_on_the_btree() {
    refusal_aborts_the_block(&mut Database::with_btree());
}

#[test]
fn refusal_aborts_the_block_on_a_file_store() {
    let dir = ScratchDir::new("on_conflict_twice_block");
    let mut db = Database::open(&dir.path).expect("open");
    refusal_aborts_the_block(&mut db);
}

/// The accept side, pinned beside the refusal so an over-correction reddens here too: a
/// block's statements each update the row the one before wrote, and one statement's two
/// proposals of a key whose first write moved the row off it insert a second row.
#[test]
fn the_rows_other_statements_wrote_are_updated() {
    let mut db = Database::with_btree();
    seed(&mut db);
    db.execute("BEGIN").expect("begin");
    for v in ["b", "c", "d"] {
        db.execute(&format!(
            "INSERT INTO t VALUES (1, '{v}') ON CONFLICT (k) DO UPDATE SET v = excluded.v"
        ))
        .expect("each statement updates the row the previous one wrote");
    }
    db.execute(
        "INSERT INTO t VALUES (1, 'x'), (1, 'y') \
         ON CONFLICT (k) DO UPDATE SET k = t.k + 10, v = excluded.v",
    )
    .expect("the first proposal moves the row off key 1, so the second inserts");
    db.execute("COMMIT").expect("commit");
    assert_eq!(
        rows(&mut db),
        vec![(1, "y".to_string()), (11, "x".to_string())]
    );
}

/// A `VALUES` list whose rows differ in length is refused in planning, before any row is
/// executed — so the first row's `DEFAULT` draws nothing from its sequence. 18.6 refuses
/// it in parse analysis, and the next draw there is `1`.
#[test]
fn a_values_length_refusal_draws_no_sequence_value() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE s (id SERIAL, b INT)")
        .expect("create");
    let err = db
        .execute("INSERT INTO s VALUES (DEFAULT, 1), (2)")
        .expect_err("rows of different lengths must be refused");
    assert_eq!(err.sqlstate(), "42601", "{err}");
    assert_eq!(err.to_string(), "VALUES lists must all be the same length");
    let mut results = db.execute("SELECT nextval('s_id_seq')").expect("nextval");
    assert!(matches!(results.remove(0).rows[0][0], Value::Int8(1)));
}
