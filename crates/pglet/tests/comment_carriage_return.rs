//! A `--` comment ends at a carriage return, on the **embed facade** (`Database::execute`).
//!
//! The differential fixtures carry the shape through the native server; this file is the
//! embed host's half, because its statement loop is its own (`tests/README.md` says what
//! a fixture run does not reach). 18.6's answers, measured over the wire: the `DELETE`
//! removes one row, the sum is `2`, and the two statements split at the `;` after the
//! carriage return.

// A test harness: `.expect()`/`.unwrap()` surface a setup failure loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use pglet::Database;
use pglet_sql::types::Value;

#[test]
fn a_where_after_a_carriage_return_is_read() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE crt (a INT); INSERT INTO crt VALUES (1), (2), (3)")
        .expect("seeds");
    db.execute("DELETE FROM crt -- only one\rWHERE a = 1")
        .expect("deletes");
    let rows = db
        .execute("SELECT a FROM crt ORDER BY a")
        .expect("reads")
        .pop()
        .expect("a result")
        .rows;
    assert_eq!(rows, vec![vec![Value::Int4(2)], vec![Value::Int4(3)]]);
}

#[test]
fn the_statement_split_reads_past_a_carriage_return() {
    let mut db = Database::with_btree();
    let results = db
        .execute("SELECT 1 --c\r+ 1; SELECT 2 --c\r; SELECT 3")
        .expect("runs");
    let values: Vec<Value> = results.iter().map(|r| r.rows[0][0].clone()).collect();
    assert_eq!(values, vec![Value::Int4(2), Value::Int4(2), Value::Int4(3)]);
    // The `;` inside the comment separates nothing, so `SELECT 2` continues `SELECT 1`.
    let err = db
        .execute("SELECT 1 --c;\rSELECT 2")
        .expect_err("one statement, malformed");
    assert_eq!(err.sqlstate(), "42601");
}
