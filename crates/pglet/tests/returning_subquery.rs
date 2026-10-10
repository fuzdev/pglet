//! Non-sole `*` projection mixed with a correlated scalar subquery in `RETURNING`
//! and `SELECT` — the fuz_app cell-query shape (`SELECT *, <grant_count>` /
//! `RETURNING *, <grant_count>`, where `grant_count` is a correlated `COUNT(*)`).
//!
//! Two engine capabilities together: `*` expands when it's one item among several
//! (INSERT's RETURNING expands at plan time like UPDATE's already did), and a
//! `RETURNING` item is evaluated with subquery context against the written row.
//! The differential fixtures (`{select,insert,update}/…_subquery.sql`) assert the
//! same against a live Postgres; this is the in-process counterpart.

// A test harness: the `seed` helper `.unwrap()`s so a setup failure surfaces loudly here.
#![allow(clippy::unwrap_used)]

use pglet::Database;
use pglet_sql::types::Value;

fn seed() -> Database {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE cell (id INT PRIMARY KEY, name TEXT)")
        .unwrap();
    db.execute("CREATE TABLE cell_grant (id INT, cell_id INT)")
        .unwrap();
    db.execute("INSERT INTO cell_grant VALUES (10, 1), (11, 1), (12, 1)")
        .unwrap();
    db
}

const GRANT_COUNT: &str =
    "(SELECT COUNT(*)::int FROM cell_grant WHERE cell_id = cell.id) AS grant_count";

#[test]
fn select_star_with_correlated_subquery() {
    let mut db = seed();
    db.execute("INSERT INTO cell VALUES (1, 'a')").unwrap();
    let sql = format!("SELECT *, {GRANT_COUNT} FROM cell ORDER BY id");
    let mut res = db.execute(&sql).unwrap();
    let r = res.remove(0);
    assert_eq!(
        r.columns()
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "name", "grant_count"]
    );
    assert_eq!(
        r.rows,
        vec![vec![
            Value::Int4(1),
            Value::Text("a".into()),
            Value::Int4(3)
        ]]
    );
}

#[test]
fn insert_returning_star_with_correlated_subquery() {
    let mut db = seed();
    let sql = format!("INSERT INTO cell (id, name) VALUES (1, 'a') RETURNING *, {GRANT_COUNT}");
    let mut res = db.execute(&sql).unwrap();
    let r = res.remove(0);
    assert_eq!(
        r.columns()
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "name", "grant_count"]
    );
    assert_eq!(
        r.rows,
        vec![vec![
            Value::Int4(1),
            Value::Text("a".into()),
            Value::Int4(3)
        ]]
    );
}

#[test]
fn update_returning_star_with_correlated_subquery() {
    let mut db = seed();
    db.execute("INSERT INTO cell VALUES (1, 'a')").unwrap();
    let sql = format!("UPDATE cell SET name = 'z' WHERE id = 1 RETURNING *, {GRANT_COUNT}");
    let mut res = db.execute(&sql).unwrap();
    let r = res.remove(0);
    assert_eq!(
        r.rows,
        vec![vec![
            Value::Int4(1),
            Value::Text("z".into()),
            Value::Int4(3)
        ]]
    );
}
