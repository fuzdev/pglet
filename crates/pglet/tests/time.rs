//! `TIME` column-type integration tests over the embed engine.
//!
//! Exercises the TIME lifecycle end-to-end (DDL → INSERT → SELECT → WHERE →
//! ORDER BY → CAST → index → arithmetic) without a server. The differential SQL
//! fixtures (`tests/fixtures/types/time_*.sql`) cover the same shapes byte-for-byte
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
    db.execute("CREATE TABLE events (id INT PRIMARY KEY, t TIME)")
        .expect("create");
    db.execute("INSERT INTO events VALUES (1, '13:45:06'), (2, '01:02:03'), (3, '23:59:59')")
        .expect("insert");
    db
}

#[test]
fn time_stores_and_renders_canonically() {
    let mut db = seeded();
    // A TIME value round-trips, rendered `HH:MM:SS` via `::text`.
    assert_eq!(
        one_text(&mut db, "SELECT t::text FROM events WHERE id = 1"),
        "13:45:06"
    );
    // The stored value is a real `Value::Time`, not text.
    let r = rows(&mut db, "SELECT t FROM events WHERE id = 2");
    assert!(matches!(r[0][0], Value::Time(_)), "got {:?}", r[0][0]);
}

#[test]
fn time_endpoint_and_input_forms() {
    let mut db = Database::with_btree();
    // The 24:00:00 endpoint is accepted and renders as itself.
    assert_eq!(
        one_text(&mut db, "SELECT '24:00:00'::time::text"),
        "24:00:00"
    );
    // AM/PM: 12 AM is midnight, 12 PM is noon, 4 PM is 16:00.
    assert_eq!(
        one_text(&mut db, "SELECT '12:00 AM'::time::text"),
        "00:00:00"
    );
    assert_eq!(
        one_text(&mut db, "SELECT '12:00 PM'::time::text"),
        "12:00:00"
    );
    assert_eq!(
        one_text(&mut db, "SELECT '4:00 PM'::time::text"),
        "16:00:00"
    );
    // All-digit ISO basic forms and fractional trimming.
    assert_eq!(one_text(&mut db, "SELECT '134506'::time::text"), "13:45:06");
    assert_eq!(one_text(&mut db, "SELECT '13:45'::time::text"), "13:45:00");
    assert_eq!(
        one_text(&mut db, "SELECT '00:00:00.100000'::time::text"),
        "00:00:00.1"
    );
}

#[test]
fn time_equality_and_range_filter() {
    let mut db = seeded();
    // Equality against a literal (coerced to time).
    let eq = rows(&mut db, "SELECT id FROM events WHERE t = '13:45:06'");
    assert_eq!(eq.len(), 1);
    assert_eq!(eq[0][0], Value::Int4(1));

    // Range: strictly after 12:00:00 → ids 1 and 3.
    let mut after: Vec<i32> = rows(&mut db, "SELECT id FROM events WHERE t > '12:00:00'")
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
fn time_order_by_is_chronological() {
    let mut db = seeded();
    let ids: Vec<i32> = rows(&mut db, "SELECT id FROM events ORDER BY t")
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("{o:?}"),
        })
        .collect();
    // 01:02:03 (id 2) < 13:45:06 (id 1) < 23:59:59 (id 3).
    assert_eq!(ids, vec![2, 1, 3]);
}

#[test]
fn time_casts_from_text_and_timestamp() {
    let mut db = Database::with_btree();
    // Text → time.
    assert_eq!(
        one_text(&mut db, "SELECT '15:30:45'::time::text"),
        "15:30:45"
    );
    // Timestamp → time truncates the date, keeping the time-of-day.
    assert_eq!(
        one_text(
            &mut db,
            "SELECT '2024-03-04 15:30:45.5'::timestamp::time::text"
        ),
        "15:30:45.5"
    );
}

#[test]
fn time_diff_and_interval_arithmetic() {
    let mut db = Database::with_btree();
    // `time - time → interval`, projected via a timestamp add to keep it wireable:
    // `'2000-01-01' + (t2 - t1)` shifts midnight by the µs difference.
    // Simpler: verify `time ± interval → time` and the wrap directly.
    assert_eq!(
        one_text(
            &mut db,
            "SELECT ('23:00:00'::time + INTERVAL '2 hours')::text"
        ),
        "01:00:00"
    );
    // Days are ignored: '01:00' + 3 days stays 01:00.
    assert_eq!(
        one_text(
            &mut db,
            "SELECT ('01:00:00'::time + INTERVAL '3 days')::text"
        ),
        "01:00:00"
    );
    // Subtraction wraps the other way.
    assert_eq!(
        one_text(
            &mut db,
            "SELECT ('01:00:00'::time - INTERVAL '2 hours')::text"
        ),
        "23:00:00"
    );
    // `date + time → timestamp`.
    assert_eq!(
        one_text(
            &mut db,
            "SELECT ('2024-06-15'::date + '13:30:00'::time)::text"
        ),
        "2024-06-15 13:30:00"
    );
}

#[test]
fn time_column_is_indexable() {
    let mut db = seeded();
    // A B-tree index on the TIME column builds, and an equality probe uses it.
    db.execute("CREATE INDEX ix_t ON events(t)").expect("index");
    let r = rows(&mut db, "SELECT id FROM events WHERE t = '23:59:59'");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0][0], Value::Int4(3));

    // A UNIQUE time is enforced (a duplicate insert fails loud).
    db.execute("CREATE TABLE u (t TIME UNIQUE)")
        .expect("create u");
    db.execute("INSERT INTO u VALUES ('08:00:00')")
        .expect("first");
    let err = db
        .execute("INSERT INTO u VALUES ('08:00:00')")
        .expect_err("duplicate time must violate UNIQUE");
    assert_eq!(err.sqlstate(), "23505");
}

#[test]
fn time_null_orders_last() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE e (id INT, t TIME)")
        .expect("create");
    db.execute("INSERT INTO e VALUES (1, '13:45:06'), (2, NULL), (3, '01:02:03')")
        .expect("insert");
    // Default `ORDER BY t` puts NULL last (matching Postgres NULLS LAST for ASC).
    let ids: Vec<i32> = rows(&mut db, "SELECT id FROM e ORDER BY t")
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(ids, vec![3, 1, 2]);
}

#[test]
fn time_default_resolves() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE d (id INT, t TIME DEFAULT '09:30:00')")
        .expect("create");
    db.execute("INSERT INTO d (id) VALUES (1)").expect("insert");
    assert_eq!(one_text(&mut db, "SELECT t::text FROM d"), "09:30:00");
}

#[test]
fn time_array_round_trips() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, ts TIME[])")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1, '{08:00:00,17:30:00}')")
        .expect("insert");
    // A `time[]` renders as the Postgres `{...}` array literal.
    assert_eq!(
        one_text(&mut db, "SELECT ts::text FROM t"),
        "{08:00:00,17:30:00}"
    );
    // The element type is carried as Time (not Text).
    let r = rows(&mut db, "SELECT ts FROM t");
    match &r[0][0] {
        Value::Array(a) => {
            assert_eq!(a.element_type, pglet_sql::types::TypeOid::Time);
            assert!(matches!(a.elements[0], Value::Time(_)));
        }
        other => panic!("expected Array, got {other:?}"),
    }
}

#[test]
fn current_time_resolves_to_a_time() {
    let mut db = Database::with_btree();
    // CURRENT_TIME / LOCALTIME both yield a plain `time` within the valid range.
    let r = rows(&mut db, "SELECT CURRENT_TIME, LOCALTIME");
    assert!(matches!(r[0][0], Value::Time(_)), "got {:?}", r[0][0]);
    assert!(matches!(r[0][1], Value::Time(_)), "got {:?}", r[0][1]);
    // A deterministic predicate: the value sits within [00:00, 24:00].
    let r = rows(
        &mut db,
        "SELECT CURRENT_TIME >= '00:00:00'::time AND CURRENT_TIME <= '24:00:00'::time",
    );
    assert_eq!(r[0][0], Value::Bool(true));
}

#[test]
fn unsupported_time_input_fails_loud() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (t TIME)").expect("create");
    // A malformed time is a loud input-syntax error, never a silent NULL/misparse.
    // An unparseable spelling reports Postgres's `22007` (invalid_datetime_format);
    // a well-formed but out-of-range field reports `22008` (datetime_field_overflow).
    let err = db
        .execute("INSERT INTO t VALUES ('not-a-time')")
        .expect_err("malformed time must fail loud");
    assert_eq!(err.sqlstate(), "22007");
    // Out-of-range (past the 24:00:00 endpoint) fails loud with 22008.
    let out_of_range = db
        .execute("INSERT INTO t VALUES ('25:00:00')")
        .expect_err("25:00:00 is past the endpoint");
    assert_eq!(out_of_range.sqlstate(), "22008");
}
