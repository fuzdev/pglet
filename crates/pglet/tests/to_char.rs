//! `to_char` + `AT TIME ZONE 'UTC'` integration tests over the embed engine.
//!
//! These exercise the spine's ISO-8601 timestamp read projection end-to-end
//! (parse → plan → execute), asserting the rendered wire text matches what real
//! Postgres produces for the same query. The differential SQL fixtures
//! (`tests/fixtures/types/to_char_iso8601.sql`,
//! `tests/fixtures/spine/cell/app_settings_iso8601.sql`) cover the same shapes
//! against a live Postgres; this is the in-process counterpart that runs without
//! a server.

// A test harness: the `rows` helper `.expect()`s so a setup failure surfaces loudly here.
#![allow(clippy::expect_used)]

use pglet::Database;
use pglet_sql::types::Value;

/// Run a single SQL string and return its one result set's rows.
fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(
        results.len(),
        1,
        "expected exactly one result set for {sql:?}"
    );
    results.remove(0).rows
}

#[test]
fn iso8601_projection_over_a_column() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE ts_proj (id INT, ts TIMESTAMPTZ)")
        .unwrap();
    db.execute(
        "INSERT INTO ts_proj (id, ts) VALUES \
         (1, '2026-06-19 12:30:45Z'), \
         (2, '2026-06-19 12:30:45.123456Z'), \
         (3, '2026-01-05 00:00:00Z')",
    )
    .unwrap();

    let mut results = db
        .execute(
            "SELECT id, to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS iso \
             FROM ts_proj ORDER BY id",
        )
        .unwrap();
    let result = results.remove(0);

    // The aliased column name is honored.
    assert_eq!(result.columns()[1].name, "iso");

    let iso: Vec<&str> = result
        .rows
        .iter()
        .map(|r| match &r[1] {
            Value::Text(s) => s.as_str(),
            other => panic!("expected text, got {other:?}"),
        })
        .collect();
    // Whole second; sub-second truncated (no fraction in the format); midnight.
    assert_eq!(
        iso,
        [
            "2026-06-19T12:30:45Z",
            "2026-06-19T12:30:45Z",
            "2026-01-05T00:00:00Z",
        ]
    );
}

#[test]
fn at_time_zone_utc_is_identity_and_unaliased_to_char_names_the_column() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE t (ts TIMESTAMPTZ)").unwrap();
    db.execute("INSERT INTO t (ts) VALUES ('2026-06-19 12:30:45Z')")
        .unwrap();

    // Unaliased `to_char(...)` projection — Postgres names the column "to_char".
    let mut results = db
        .execute("SELECT to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM t")
        .unwrap();
    let result = results.remove(0);
    assert_eq!(result.columns()[0].name, "to_char");
    assert_eq!(
        result.rows[0][0],
        Value::Text("2026-06-19T12:30:45Z".into())
    );
}

#[test]
fn non_utc_time_zone_fails_loud() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE t (ts TIMESTAMPTZ)").unwrap();
    db.execute("INSERT INTO t (ts) VALUES ('2026-06-19 12:30:45Z')")
        .unwrap();

    // A non-UTC zone must error, not silently mis-render.
    let err = db.execute("SELECT ts AT TIME ZONE 'America/New_York' FROM t");
    assert!(err.is_err(), "non-UTC AT TIME ZONE should fail loud");
}

#[test]
fn unknown_to_char_format_token_fails_loud() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE t (ts TIMESTAMPTZ)").unwrap();
    db.execute("INSERT INTO t (ts) VALUES ('2026-06-19 12:30:45Z')")
        .unwrap();

    // `US` (microseconds) is unsupported — must error rather than guess.
    let err = db.execute("SELECT to_char(ts AT TIME ZONE 'UTC', 'YYYY-US') FROM t");
    assert!(err.is_err(), "unknown to_char token should fail loud");
}

#[test]
fn null_timestamp_yields_null() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE t (ts TIMESTAMPTZ)").unwrap();
    db.execute("INSERT INTO t (ts) VALUES (NULL)").unwrap();

    let out = rows(
        &mut db,
        "SELECT to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM t",
    );
    assert_eq!(out[0][0], Value::Null);
}
