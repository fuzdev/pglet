//! `UPDATE … SET col = DEFAULT` resolution + SET-RHS fail-loud, over the embed
//! engine (parse → plan → execute).
//!
//! Two behaviors, both previously silently-wrong divergences from Postgres:
//! - `SET col = DEFAULT` resolved to nothing (the old value was kept). It now
//!   resolves the column's catalog default, mirroring INSERT — including a
//!   SERIAL/nextval default, which advances the sequence — on both the plain
//!   UPDATE path and the `ON CONFLICT DO UPDATE` path.
//! - A SET-RHS eval error was swallowed (the assignment skipped, old value kept).
//!   It now propagates (correct-or-loud).
//!
//! The differential SQL fixtures (`tests/fixtures/update/set_default.sql`,
//! `…/set_rhs_error_loud.sql`) assert the same shapes against a live Postgres;
//! this is the in-process counterpart that runs without a server.

// A test harness: the `rows` helper `.expect()`s so a setup failure surfaces loudly here.
#![allow(clippy::expect_used)]

use pglet::Database;
use pglet_sql::types::Value;

/// Run a single SQL string and return its one result set's rows.
fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(results.len(), 1, "expected one result set for {sql:?}");
    results.remove(0).rows
}

/// `SET id = DEFAULT` (a SERIAL/nextval default) advances the sequence, and
/// `SET label = DEFAULT` (a literal default) resolves the literal.
#[test]
fn set_default_resolves_literal_and_advances_serial() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE d (id SERIAL, label TEXT DEFAULT 'def', n INT)")
        .unwrap();
    db.execute("INSERT INTO d (label, n) VALUES ('a', 1)")
        .unwrap(); // id=1
    db.execute("INSERT INTO d (label, n) VALUES ('b', 2)")
        .unwrap(); // id=2, seq at 2

    // SET id = DEFAULT calls nextval (seq 2 → 3); SET label = DEFAULT resolves the
    // literal 'def'. Previously both silently kept the old value (id=1, label='a').
    let returned = rows(
        &mut db,
        "UPDATE d SET id = DEFAULT, label = DEFAULT WHERE n = 1 RETURNING id, label",
    );
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0][0], Value::Int4(3), "nextval advanced 2 → 3");
    assert_eq!(returned[0][1], Value::Text("def".to_string()));

    // The untouched row keeps id=2; the new sequence value is 3, so the full set is
    // {2, 3} — proving the SERIAL default advanced rather than reusing the old id.
    let all = rows(&mut db, "SELECT id FROM d ORDER BY id");
    assert_eq!(all, vec![vec![Value::Int4(2)], vec![Value::Int4(3)]]);
}

/// `ON CONFLICT DO UPDATE SET col = DEFAULT` resolves the column's catalog default
/// too (the symmetric path), while a sibling assignment still sees `EXCLUDED`.
#[test]
fn set_default_in_on_conflict_do_update() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE c (id INT PRIMARY KEY, label TEXT DEFAULT 'dflt', v INT)")
        .unwrap();
    db.execute("INSERT INTO c (id, label, v) VALUES (1, 'orig', 10)")
        .unwrap();

    // Conflict on id=1 → DO UPDATE: label = DEFAULT resolves 'dflt' (not 'orig', not
    // the would-be-inserted 'new'); v = EXCLUDED.v takes the conflicting row's value.
    let returned = rows(
        &mut db,
        "INSERT INTO c (id, label, v) VALUES (1, 'new', 20) \
         ON CONFLICT (id) DO UPDATE SET label = DEFAULT, v = EXCLUDED.v \
         RETURNING id, label, v",
    );
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0][0], Value::Int4(1));
    assert_eq!(returned[0][1], Value::Text("dflt".to_string()));
    assert_eq!(returned[0][2], Value::Int4(20));
}

/// A SET-RHS eval error (integer division by zero) propagates — the statement
/// fails loud and writes nothing, instead of silently skipping the assignment.
#[test]
fn set_rhs_eval_error_propagates() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE e (n INT)").unwrap();
    db.execute("INSERT INTO e VALUES (5)").unwrap();

    let err = db.execute("UPDATE e SET n = n / 0");
    assert!(
        err.is_err(),
        "division by zero in a SET RHS must fail loud, not be swallowed"
    );

    // The row is unchanged — the error aborted the statement before any write.
    let after = rows(&mut db, "SELECT n FROM e");
    assert_eq!(after, vec![vec![Value::Int4(5)]]);
}

/// The same fail-loud applies on the `UPDATE … FROM other` path (it shares
/// `assign_row`): a type-erroring SET RHS over the joined row propagates.
#[test]
fn set_rhs_error_propagates_on_update_from() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE tgt (id INT, n INT)").unwrap();
    db.execute("CREATE TABLE src (id INT, k INT)").unwrap();
    db.execute("INSERT INTO tgt VALUES (1, 5)").unwrap();
    db.execute("INSERT INTO src VALUES (1, 0)").unwrap();

    // n / s.k with s.k = 0 → division by zero, evaluated against the joined row.
    let err = db.execute("UPDATE tgt t SET n = t.n / s.k FROM src s WHERE t.id = s.id");
    assert!(
        err.is_err(),
        "a SET-RHS error on the FROM path must propagate"
    );

    let after = rows(&mut db, "SELECT n FROM tgt");
    assert_eq!(after, vec![vec![Value::Int4(5)]]);
}
