//! An `INSERT`'s `RETURNING` and `ON CONFLICT DO UPDATE` clauses refuse a subquery whose
//! own reference binds to nothing — **before** the row is written — over the embed engine
//! (parse → plan → execute) on the default CoW-B+tree backend.
//!
//! Both clauses lower a subquery from inside the clause's own parsing, so the nested
//! level's binding gate stays open (`scope_is_complete == false`: an expression subquery
//! encloses it) and only an enclosing `resolve_correlated_subplans` re-walk can close it.
//! There was no such re-walk for an `INSERT` — `lower_insert` never ran the pass and
//! `walk_level` treated `Plan::Insert` as a leaf — so nobody judged those levels, and with
//! the inner relation **empty** the eval-time resolver never fired either. Both statements
//! below wrote where 18.6 refuses.
//!
//! **These tests exist beside the differential fixtures rather than instead of them.** A
//! refusal fixture is `@expect-error`, which compares the SQLSTATE and nothing else — it
//! is green for a refusal raised *after* the rows are written. What is asserted here is
//! that the target rows are untouched. The SQLSTATEs themselves are pinned differentially
//! (`tests/fixtures/insert/returning_subquery_key_binds_to_nothing_loud.sql` and its three
//! siblings), since only a live 18.6 can say they are Postgres's.
//!
//! **`with_btree` is load-bearing**, not incidental: `MemoryStorage` has no autocommit
//! statement rollback, so a write applied before a refusal stands there and a guard
//! written against it would read a partially-applied statement as the same shape either
//! way. The default backend is what these tests must speak about.
//!
//! Read the pins in order: reaching a row assertion means the statement was refused (the
//! `is_err` pin above it panics otherwise), so what the row assertions guard is a refusal
//! placed *after* the writes — the shape this engine has shipped before.

// A test harness: setup `.unwrap()`s and the `rows` helper `.expect()`s, so a setup
// failure surfaces loudly here rather than as a confusing assertion mismatch.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pglet::Database;
use pglet_sql::types::Value;

fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(results.len(), 1, "expected one result set for {sql:?}");
    results.remove(0).rows
}

/// Seed an empty `icb_target(v)` beside an **empty** `icb_src(b)`.
///
/// `icb_src` carries no row deliberately: the eval-time resolver raises the same errors
/// per row, so a seeded source makes every statement here loud with the gate reverted and
/// the guard proves nothing. An inner relation with no row evaluates nothing, which is
/// what leaves the plan-time gate as the only verdict.
fn returning_seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE icb_target (v INT)").unwrap();
    db.execute("CREATE TABLE icb_src (b INT)").unwrap();
    db
}

/// Seed `ocb_target(id PRIMARY KEY, v)` = `{(1, 1)}` beside an **empty** `ocb_src(b)`.
///
/// The `ON CONFLICT` statements below insert `id = 2`, which collides with nothing — the
/// action is never evaluated, so the plan-time gate is again the only verdict, and the
/// write it must prevent is the *insert* of `(2, 2)`.
fn on_conflict_seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ocb_target (id INT PRIMARY KEY, v INT)")
        .unwrap();
    db.execute("CREATE TABLE ocb_src (b INT)").unwrap();
    db.execute("INSERT INTO ocb_target VALUES (1, 1)").unwrap();
    db
}

#[test]
fn a_returning_subquery_key_binding_to_nothing_writes_no_row() {
    let mut db = returning_seeded();
    assert!(
        db.execute(
            "INSERT INTO icb_target VALUES (1) RETURNING (SELECT b FROM icb_src ORDER BY zzz)"
        )
        .is_err(),
        "a RETURNING subquery's unbindable sort key must be refused"
    );
    assert!(
        rows(&mut db, "SELECT v FROM icb_target").is_empty(),
        "the refused INSERT must have written nothing"
    );
}

#[test]
fn a_returning_subquery_bogus_qualifier_writes_no_row() {
    let mut db = returning_seeded();
    assert!(
        db.execute(
            "INSERT INTO icb_target VALUES (1) RETURNING (SELECT b FROM icb_src WHERE zzz.q = 1)"
        )
        .is_err(),
        "a RETURNING subquery's unnamed qualifier must be refused"
    );
    assert!(
        rows(&mut db, "SELECT v FROM icb_target").is_empty(),
        "the refused INSERT must have written nothing"
    );
}

#[test]
fn an_on_conflict_set_subquery_key_binding_to_nothing_writes_no_row() {
    let mut db = on_conflict_seeded();
    assert!(
        db.execute(
            "INSERT INTO ocb_target VALUES (2, 2) \
             ON CONFLICT (id) DO UPDATE SET v = (SELECT b FROM ocb_src ORDER BY zzz)"
        )
        .is_err(),
        "an ON CONFLICT action subquery's unbindable sort key must be refused"
    );
    assert_eq!(
        rows(&mut db, "SELECT id, v FROM ocb_target"),
        vec![vec![Value::Int4(1), Value::Int4(1)]],
        "the refused INSERT must have left the seeded row alone and added none"
    );
}

#[test]
fn an_on_conflict_set_subquery_bogus_qualifier_writes_no_row() {
    let mut db = on_conflict_seeded();
    assert!(
        db.execute(
            "INSERT INTO ocb_target VALUES (2, 2) \
             ON CONFLICT (id) DO UPDATE SET v = (SELECT b FROM ocb_src WHERE zzz.q = 1)"
        )
        .is_err(),
        "an ON CONFLICT action subquery's unnamed qualifier must be refused"
    );
    assert_eq!(
        rows(&mut db, "SELECT id, v FROM ocb_target"),
        vec![vec![Value::Int4(1), Value::Int4(1)]],
        "the refused INSERT must have left the seeded row alone and added none"
    );
}

/// The accept control the four refusals above are only meaningful beside: a correlated
/// `RETURNING` subquery that legitimately names the target still answers, and the write
/// still lands.
///
/// Its fixture twin (`insert/returning_subquery_names_the_target.sql`) pins the values
/// against 18.6; what this adds is the row-level assertion that the accepted statement
/// wrote, which an over-rejecting fix would lose along with the answer.
#[test]
fn a_returning_subquery_naming_the_target_still_writes_and_answers() {
    let mut db = returning_seeded();
    db.execute("INSERT INTO icb_src VALUES (5), (7)").unwrap();
    assert_eq!(
        rows(
            &mut db,
            "INSERT INTO icb_target VALUES (5) \
             RETURNING (SELECT count(*) FROM icb_src WHERE icb_src.b = icb_target.v)"
        ),
        vec![vec![Value::Int8(1)]],
    );
    assert_eq!(
        rows(&mut db, "SELECT v FROM icb_target"),
        vec![vec![Value::Int4(5)]],
    );
}

/// The `ON CONFLICT` accept control: a `SET` right-hand side naming `EXCLUDED` resolves,
/// and the non-conflicting insert lands.
///
/// The action is not evaluated (nothing collides with `id = 2`), which is what makes this
/// a control on the **gate** rather than on evaluation — pglet cannot evaluate a subquery
/// inside an executed action at all (`0A000`, a pre-existing gap unrelated to this rule).
#[test]
fn an_on_conflict_set_subquery_naming_excluded_still_writes() {
    let mut db = on_conflict_seeded();
    db.execute("INSERT INTO ocb_src VALUES (5)").unwrap();
    db.execute(
        "INSERT INTO ocb_target VALUES (2, 5) \
         ON CONFLICT (id) DO UPDATE SET v = (SELECT count(*) FROM ocb_src WHERE ocb_src.b = excluded.v)",
    )
    .expect("a SET right-hand side naming EXCLUDED must resolve");
    assert_eq!(
        rows(&mut db, "SELECT id, v FROM ocb_target ORDER BY id"),
        vec![
            vec![Value::Int4(1), Value::Int4(1)],
            vec![Value::Int4(2), Value::Int4(5)],
        ],
    );
}

/// The `ON CONFLICT` action's own `WHERE` is a **third** clause position the re-walk
/// serves, and the row-survival half of its guard.
///
/// Its fixture twin (`insert/on_conflict_where_subquery_key_binds_to_nothing_loud.sql`)
/// pins the SQLSTATE against 18.6. This is the assertion that fixture structurally cannot
/// make: the harness wraps every fixture in `BEGIN … ROLLBACK`, so an `@expect-error` is
/// green for a refusal raised *after* the row is written. Deleting the arm's
/// `where_predicate` visit left the whole fixture corpus and `cargo test --workspace`
/// green while this statement wrote its row, which is why the position needs a guard of
/// its own rather than riding the `SET` right-hand side's.
#[test]
fn an_on_conflict_where_subquery_key_binding_to_nothing_writes_no_row() {
    let mut db = on_conflict_seeded();
    assert!(
        db.execute(
            "INSERT INTO ocb_target VALUES (3, 3) \
             ON CONFLICT (id) DO UPDATE SET v = 1 \
             WHERE (SELECT b FROM ocb_src ORDER BY zzz) IS NULL"
        )
        .is_err(),
        "an ON CONFLICT action WHERE subquery's unbindable sort key must be refused"
    );
    assert_eq!(
        rows(&mut db, "SELECT id, v FROM ocb_target"),
        vec![vec![Value::Int4(1), Value::Int4(1)]],
        "the refused INSERT must have left the seeded row alone and added none"
    );
}
