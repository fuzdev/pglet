//! Regression tests for the correct-or-loud fixes from the code-smell audit, over
//! the embed engine (parse → plan → execute).
//!
//! - An `UPDATE … FROM` whose `WHERE` errors propagates instead of being swallowed
//!   to a silent 0-row update (the swallow `execute_join` already fixed, missed in
//!   `collect_updated_from`).
//! - Projecting an INTERVAL now works (it is a full wire-encoded type): a
//!   `timestamp - timestamp` difference projects as `postgres`-style text
//!   regardless of which row's value is NULL.

// A test harness: the `rows` helper `.expect()`s so a setup failure surfaces loudly here.
#![allow(clippy::expect_used)]

use pglet::Database;
use pglet_sql::types::Value;

fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(results.len(), 1, "expected one result set for {sql:?}");
    results.remove(0).rows
}

/// An eval error in the `UPDATE … FROM … WHERE` propagates; the target is unchanged.
#[test]
fn update_from_where_eval_error_propagates() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE uw_tgt (id INT, n INT)").unwrap();
    db.execute("CREATE TABLE uw_src (id INT, k INT)").unwrap();
    db.execute("INSERT INTO uw_tgt VALUES (1, 10)").unwrap();
    db.execute("INSERT INTO uw_src VALUES (1, 5)").unwrap();

    // The join WHERE divides by zero on the matching pair; previously the Err was
    // swallowed to "no match" → a silent 0-row update.
    let err =
        db.execute("UPDATE uw_tgt t SET n = 99 FROM uw_src s WHERE t.id = s.id AND s.k / 0 = 1");
    assert!(
        err.is_err(),
        "an erroring UPDATE…FROM WHERE must propagate, not silently match 0 rows"
    );
    let after = rows(&mut db, "SELECT n FROM uw_tgt");
    assert_eq!(after, vec![vec![Value::Int4(10)]], "target row unchanged");
}

/// Projecting an interval-typed column now succeeds (interval is a full
/// wire-encoded type), even when the first row's value is NULL — the value that is
/// present in a later row projects as `postgres`-style text (`ts - ts` justifies
/// the ≥24h span into days).
#[test]
fn interval_projection_works_regardless_of_row_order() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE iv (a TIMESTAMP, b TIMESTAMP)")
        .unwrap();
    // Row 1: b NULL → (b - a) is NULL. Row 2: both present → (b - a) is 1 day.
    db.execute("INSERT INTO iv VALUES ('2020-01-01 00:00:00', NULL)")
        .unwrap();
    db.execute("INSERT INTO iv VALUES ('2020-01-01 00:00:00', '2020-01-02 00:00:00')")
        .unwrap();

    let rows = rows(&mut db, "SELECT b - a AS d FROM iv ORDER BY a");
    // The interval column projects: a NULL for the first row, the justified span
    // (`1 day`) for the second — no fail-loud, the value survives to output.
    assert_eq!(
        rows,
        vec![
            vec![Value::Null],
            vec![Value::Interval(pglet_sql::types::PgInterval::new(0, 1, 0))],
        ],
        "the interval difference must project (NULL then `1 day`)"
    );
}
