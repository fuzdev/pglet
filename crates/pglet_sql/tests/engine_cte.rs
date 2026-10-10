//! Engine integration tests: data-modifying CTEs —
//! `WITH x AS (UPDATE/DELETE/INSERT … RETURNING …) SELECT … FROM x [JOIN …]`.
//!
//! The CTE's DML runs once; its `RETURNING` rows become the relation the body
//! reads (here joined to an unrelated base table, the spine's
//! role_grant-supersede shape).

use crate::common;
use common::*;
use pglet_sql::types::Value;

/// `accounts(id, name)` + `grants(id, owner, revoked)` with two grants for
/// account 1 and one for account 2.
fn setup(db: &mut TestDb) {
    db.run("CREATE TABLE accounts (id INT, name TEXT)");
    db.run("INSERT INTO accounts VALUES (1, 'Alice'), (2, 'Bob')");
    db.run("CREATE TABLE grants (id INT, owner INT, revoked BOOL)");
    db.run("INSERT INTO grants VALUES (10, 1, false), (11, 1, false), (12, 2, false)");
}

#[test]
fn update_returning_cte_joined_to_base_table() {
    let mut db = TestDb::new();
    setup(&mut db);

    // Revoke account 1's grants, then join the revoked rows to `accounts`.
    let result = db.run(
        "WITH updated AS (
            UPDATE grants SET revoked = true
            WHERE owner = 1 AND revoked = false
            RETURNING id, owner
        )
        SELECT u.id AS grant_id, a.name
        FROM updated u
        JOIN accounts a ON a.id = u.owner
        ORDER BY u.id",
    );

    assert_eq!(result.columns().len(), 2);
    assert_eq!(result.rows.len(), 2, "two grants revoked");
    assert_eq!(result.rows[0][0], Value::Int4(10));
    assert_eq!(result.rows[0][1], Value::Text("Alice".to_string()));
    assert_eq!(result.rows[1][0], Value::Int4(11));
    assert_eq!(result.rows[1][1], Value::Text("Alice".to_string()));

    // The UPDATE actually persisted (and ran exactly once).
    let check = db.run("SELECT id FROM grants WHERE revoked = true ORDER BY id");
    assert_eq!(check.rows.len(), 2);
    assert_eq!(check.rows[0][0], Value::Int4(10));
    assert_eq!(check.rows[1][0], Value::Int4(11));
}

#[test]
fn update_returning_wildcard_in_body() {
    let mut db = TestDb::new();
    setup(&mut db);

    // `u.*` over the CTE relation — wildcard expansion against the captured columns.
    let result = db.run(
        "WITH updated AS (
            UPDATE grants SET revoked = true WHERE id = 12 RETURNING *
        )
        SELECT u.*, a.name
        FROM updated u
        JOIN accounts a ON a.id = u.owner",
    );

    // grants has 3 columns (id, owner, revoked) + accounts.name.
    assert_eq!(result.columns().len(), 4);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(12));
    assert_eq!(result.rows[0][1], Value::Int4(2));
    assert_eq!(result.rows[0][2], Value::Bool(true));
    assert_eq!(result.rows[0][3], Value::Text("Bob".to_string()));
}

#[test]
fn update_cte_zero_match_yields_no_rows() {
    let mut db = TestDb::new();
    setup(&mut db);

    // No grant for owner 99 → the UPDATE matches nothing → the join is empty.
    let result = db.run(
        "WITH updated AS (
            UPDATE grants SET revoked = true WHERE owner = 99 RETURNING id, owner
        )
        SELECT u.id, a.name FROM updated u JOIN accounts a ON a.id = u.owner",
    );
    assert_eq!(result.rows.len(), 0);
}

#[test]
fn delete_returning_cte() {
    let mut db = TestDb::new();
    setup(&mut db);

    let result = db.run(
        "WITH removed AS (
            DELETE FROM grants WHERE owner = 1 RETURNING id, owner
        )
        SELECT u.id, a.name FROM removed u JOIN accounts a ON a.id = u.owner ORDER BY u.id",
    );
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(10));
    assert_eq!(result.rows[1][0], Value::Int4(11));

    // The rows are actually gone.
    let check = db.run("SELECT id FROM grants ORDER BY id");
    assert_eq!(check.rows.len(), 1);
    assert_eq!(check.rows[0][0], Value::Int4(12));
}

#[test]
fn insert_returning_cte() {
    let mut db = TestDb::new();
    setup(&mut db);

    let result = db.run(
        "WITH added AS (
            INSERT INTO grants VALUES (20, 2, false) RETURNING id, owner
        )
        SELECT u.id, a.name FROM added u JOIN accounts a ON a.id = u.owner",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(20));
    assert_eq!(result.rows[0][1], Value::Text("Bob".to_string()));

    let check = db.run("SELECT id FROM grants WHERE id = 20");
    assert_eq!(check.rows.len(), 1);
}

#[test]
fn modifying_cte_runs_even_when_unreferenced() {
    let mut db = TestDb::new();
    setup(&mut db);

    // The body never selects from `updated`, but Postgres (and pglet) still run it.
    let result = db.run(
        "WITH updated AS (
            UPDATE grants SET revoked = true WHERE id = 10 RETURNING id
        )
        SELECT name FROM accounts WHERE id = 1",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Text("Alice".to_string()));

    // Side effect happened despite no reference.
    let check = db.run("SELECT revoked FROM grants WHERE id = 10");
    assert_eq!(check.rows[0][0], Value::Bool(true));
}

#[test]
fn select_star_over_cte_only() {
    let mut db = TestDb::new();
    setup(&mut db);

    // Bare `SELECT * FROM cte` (no join) — the executor fast path over a CteScan.
    let result = db.run(
        "WITH updated AS (
            UPDATE grants SET revoked = true WHERE id = 11 RETURNING id, owner
        )
        SELECT * FROM updated",
    );
    assert_eq!(result.columns().len(), 2);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(11));
    assert_eq!(result.rows[0][1], Value::Int4(1));
}

#[test]
fn cte_and_base_both_have_id_column_qualified_resolves() {
    // Mirrors the spine: both the CTE relation (`u`, from role_grant) and the base
    // table (`a`, actor) have an `id` column, and the body selects `u.id` while
    // joining `a.id = u.actor_id`. `u.id` must resolve to the CTE side, not `a.id`.
    let mut db = TestDb::new();
    db.run("CREATE TABLE actor (id INT, account_id INT)");
    db.run("INSERT INTO actor VALUES (100, 9001), (101, 9002)");
    db.run("CREATE TABLE role_grant (id INT, scope_id INT, actor_id INT, revoked BOOL)");
    db.run(
        "INSERT INTO role_grant VALUES (1, 5, 100, false), (2, 5, 101, false), (3, 7, 100, false)",
    );

    let result = db.run(
        "WITH updated AS (
            UPDATE role_grant SET revoked = true
            WHERE scope_id = 5 AND revoked = false
            RETURNING id, actor_id
        )
        SELECT u.id AS role_grant_id, a.account_id
        FROM updated u
        JOIN actor a ON a.id = u.actor_id
        ORDER BY u.id",
    );
    assert_eq!(result.rows.len(), 2);
    // role_grant_id must be the role_grant ids (1, 2), NOT the actor ids (100, 101).
    assert_eq!(
        result.rows[0][0],
        Value::Int4(1),
        "u.id is the role_grant id"
    );
    assert_eq!(result.rows[0][1], Value::Int4(9001));
    assert_eq!(result.rows[1][0], Value::Int4(2));
    assert_eq!(result.rows[1][1], Value::Int4(9002));
}

#[test]
fn update_with_table_alias_resolves_and_returns() {
    // `UPDATE t o SET … WHERE o.col … RETURNING o.*` — a target alias, alias-aware
    // WHERE, and the qualified RETURNING wildcard, all without a FROM clause (the
    // spine's `revoke_for_scope` shape).
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, v INT)");
    db.run("INSERT INTO t VALUES (1, 10), (2, 20)");

    let result = db.run("UPDATE t o SET v = 99 WHERE o.id = 1 RETURNING o.*");
    assert_eq!(result.command_tag, "UPDATE 1");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns().len(), 2, "o.* expands to both columns");
    assert_eq!(result.rows[0][0], Value::Int4(1));
    assert_eq!(result.rows[0][1], Value::Int4(99));

    // Row 2 untouched.
    let check = db.run("SELECT v FROM t WHERE id = 2");
    assert_eq!(check.rows[0][0], Value::Int4(20));
}

/// `offer(id, to_account, superseded)` joined to `actor(id, account_id)` — the
/// spine's `UPDATE role_grant_offer o … FROM actor a` shape, shrunk.
fn setup_offer_actor(db: &mut TestDb) {
    db.run("CREATE TABLE offer (id INT, to_account INT, superseded BOOL)");
    db.run("INSERT INTO offer VALUES (1, 9001, false), (2, 9002, false), (3, 9001, false)");
    db.run("CREATE TABLE actor (id INT, account_id INT)");
    db.run("INSERT INTO actor VALUES (100, 9001), (101, 9002)");
}

#[test]
fn update_from_joins_source_and_updates_matches() {
    let mut db = TestDb::new();
    setup_offer_actor(&mut db);

    // Supersede every offer whose to_account matches actor 100's account (9001).
    let result = db.run(
        "UPDATE offer o SET superseded = true
         FROM actor a
         WHERE a.id = 100 AND o.to_account = a.account_id
         RETURNING o.id",
    );
    assert_eq!(result.command_tag, "UPDATE 2");
    let mut ids: Vec<i32> = result
        .rows
        .iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref v => panic!("expected int id, got {v:?}"),
        })
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 3], "offers for account 9001 superseded");

    // Offer 2 (account 9002) is untouched.
    let check = db.run("SELECT superseded FROM offer WHERE id = 2");
    assert_eq!(check.rows[0][0], Value::Bool(false));
}

#[test]
fn update_from_no_source_match_updates_nothing() {
    let mut db = TestDb::new();
    setup_offer_actor(&mut db);

    // actor 999 doesn't exist → inner join is empty → no rows updated.
    let result = db.run(
        "UPDATE offer o SET superseded = true
         FROM actor a
         WHERE a.id = 999 AND o.to_account = a.account_id
         RETURNING o.id",
    );
    assert_eq!(result.command_tag, "UPDATE 0");
    assert_eq!(result.rows.len(), 0);
}

#[test]
fn update_from_inside_cte_joined_to_base_table() {
    // The full spine shape: a data-modifying `UPDATE … FROM … RETURNING o.*` CTE,
    // whose rows the body re-joins to a base table.
    let mut db = TestDb::new();
    setup_offer_actor(&mut db);
    db.run("CREATE TABLE account (id INT, label TEXT)");
    db.run("INSERT INTO account VALUES (9001, 'team-a'), (9002, 'team-b')");

    let result = db.run(
        "WITH updated AS (
            UPDATE offer o SET superseded = true
            FROM actor a
            WHERE a.id = 100 AND o.to_account = a.account_id
            RETURNING o.*
        )
        SELECT u.id AS offer_id, acc.label
        FROM updated u
        JOIN account acc ON acc.id = u.to_account
        ORDER BY u.id",
    );
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(1));
    assert_eq!(result.rows[0][1], Value::Text("team-a".into()));
    assert_eq!(result.rows[1][0], Value::Int4(3));
    assert_eq!(result.rows[1][1], Value::Text("team-a".into()));
}
