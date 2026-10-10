//! `DATE` column-type integration tests over the embed engine.
//!
//! Exercises the DATE lifecycle end-to-end (DDL → INSERT → SELECT → WHERE →
//! ORDER BY → CAST → index) without a server. The differential SQL fixtures
//! (`tests/fixtures/types/date_*.sql`) cover the same shapes byte-for-byte
//! against a live Postgres; this is the in-process counterpart.

// A test harness: the `rows` helper `.expect()`s and the shape helpers `panic!` so a
// setup failure or an unexpected cell type surfaces loudly right here.
#![allow(clippy::expect_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::Value;

/// Run one SQL string and return its single result set's rows.
fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(results.len(), 1, "expected one result set for {sql:?}");
    results.remove(0).rows
}

/// The single `text` cell of a one-row, one-column result (via `::text`).
fn one_text(db: &mut Database, sql: &str) -> String {
    let r = rows(db, sql);
    assert_eq!(r.len(), 1, "expected one row for {sql:?}");
    match &r[0][0] {
        Value::Text(s) => s.clone(),
        other => panic!("expected Text, got {other:?} for {sql:?}"),
    }
}

fn seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE events (id INT PRIMARY KEY, d DATE)")
        .expect("create");
    db.execute("INSERT INTO events VALUES (1, '2024-06-15'), (2, '2024-01-02'), (3, '2024-12-31')")
        .expect("insert");
    db
}

#[test]
fn date_stores_and_renders_as_iso() {
    let mut db = seeded();
    // A DATE value round-trips as itself, rendered `YYYY-MM-DD` via `::text`.
    assert_eq!(
        one_text(&mut db, "SELECT d::text FROM events WHERE id = 1"),
        "2024-06-15"
    );
    // The stored value is a real `Value::Date`, not text.
    let r = rows(&mut db, "SELECT d FROM events WHERE id = 2");
    assert!(matches!(r[0][0], Value::Date(_)), "got {:?}", r[0][0]);
}

#[test]
fn date_equality_and_range_filter() {
    let mut db = seeded();
    // Equality against an ISO literal.
    let eq = rows(&mut db, "SELECT id FROM events WHERE d = '2024-06-15'");
    assert_eq!(eq.len(), 1);
    assert_eq!(eq[0][0], Value::Int4(1));

    // Range: strictly after 2024-06-01 → ids 1 and 3.
    let mut after: Vec<i32> = rows(&mut db, "SELECT id FROM events WHERE d > '2024-06-01'")
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("{o:?}"),
        })
        .collect();
    after.sort_unstable();
    assert_eq!(after, vec![1, 3]);
}

#[test]
fn date_order_by_is_chronological() {
    let mut db = seeded();
    let ids: Vec<i32> = rows(&mut db, "SELECT id FROM events ORDER BY d")
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("{o:?}"),
        })
        .collect();
    // 2024-01-02 (id 2) < 2024-06-15 (id 1) < 2024-12-31 (id 3).
    assert_eq!(ids, vec![2, 1, 3]);
}

#[test]
fn date_casts_from_text_and_timestamp() {
    let mut db = Database::with_btree();
    // Text → date.
    assert_eq!(
        one_text(&mut db, "SELECT '2024-03-04'::date::text"),
        "2024-03-04"
    );
    // Timestamp → date truncates the time-of-day.
    assert_eq!(
        one_text(
            &mut db,
            "SELECT '2024-03-04 15:30:00'::timestamp::date::text"
        ),
        "2024-03-04"
    );
}

#[test]
fn date_column_is_indexable() {
    let mut db = seeded();
    // A B-tree index on the DATE column builds, and an equality probe uses it.
    db.execute("CREATE INDEX ix_d ON events(d)").expect("index");
    let r = rows(&mut db, "SELECT id FROM events WHERE d = '2024-12-31'");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0][0], Value::Int4(3));

    // A UNIQUE date is enforced (a duplicate insert fails loud).
    db.execute("CREATE TABLE u (d DATE UNIQUE)")
        .expect("create u");
    db.execute("INSERT INTO u VALUES ('2024-01-01')")
        .expect("first");
    let err = db
        .execute("INSERT INTO u VALUES ('2024-01-01')")
        .expect_err("duplicate date must violate UNIQUE");
    assert_eq!(err.sqlstate(), "23505");
}

#[test]
fn date_null_orders_last() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE e (id INT, d DATE)")
        .expect("create");
    db.execute("INSERT INTO e VALUES (1, '2024-06-15'), (2, NULL), (3, '2024-01-02')")
        .expect("insert");
    // Default `ORDER BY d` puts NULL last (matching Postgres NULLS LAST for ASC).
    let ids: Vec<i32> = rows(&mut db, "SELECT id FROM e ORDER BY d")
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(ids, vec![3, 1, 2]);
}

#[test]
fn date_default_resolves() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE d (id INT, dt DATE DEFAULT '2024-01-01')")
        .expect("create");
    db.execute("INSERT INTO d (id) VALUES (1)").expect("insert");
    assert_eq!(one_text(&mut db, "SELECT dt::text FROM d"), "2024-01-01");
}

#[test]
fn date_array_round_trips() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, ds DATE[])")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1, '{2024-01-01,2024-02-02}')")
        .expect("insert");
    // A `date[]` renders as the Postgres `{...}` array literal.
    assert_eq!(
        one_text(&mut db, "SELECT ds::text FROM t"),
        "{2024-01-01,2024-02-02}"
    );
    // The element type is carried as Date (not Text).
    let r = rows(&mut db, "SELECT ds FROM t");
    match &r[0][0] {
        Value::Array(a) => {
            assert_eq!(a.element_type, pglet_sql::types::TypeOid::Date);
            assert!(matches!(a.elements[0], Value::Date(_)));
        }
        other => panic!("expected Array, got {other:?}"),
    }
}

#[test]
fn unsupported_date_input_fails_loud() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (d DATE)").expect("create");
    // A malformed date is a loud input-syntax error, never a silent NULL/misparse.
    // An unparseable spelling reports Postgres's `22007` (invalid_datetime_format);
    // a well-formed but out-of-range field reports `22008` (datetime_field_overflow).
    let bad_format = db
        .execute("INSERT INTO t VALUES ('not-a-date')")
        .expect_err("malformed date must fail loud");
    assert_eq!(bad_format.sqlstate(), "22007");
    let out_of_range = db
        .execute("INSERT INTO t VALUES ('2026-13-01')")
        .expect_err("out-of-range date field must fail loud");
    assert_eq!(out_of_range.sqlstate(), "22008");
}
