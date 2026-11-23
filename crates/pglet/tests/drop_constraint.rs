//! `ALTER TABLE … DROP CONSTRAINT` — the halves the differential fixtures cannot hold:
//! the `NOTICE` an `IF EXISTS` skip sends (the harness compares no notice), the catalog
//! a failing list leaves (every fixture runs inside a `BEGIN … ROLLBACK`, which
//! restores it), a block's rollback, the portable dump, and a reopened store.

#![expect(
    clippy::unwrap_used,
    reason = "a failed setup statement should surface where it happens"
)]

use crate::common;

use common::ScratchDir;
use pglet::Database;

/// The schema every test starts from: a parent, and a child with a foreign key and a
/// `CHECK`.
const SETUP: &str = "CREATE TABLE p (id INT PRIMARY KEY); \
     CREATE TABLE c (id INT, pid INT REFERENCES p(id) ON DELETE CASCADE, CHECK (id > 0)); \
     INSERT INTO p VALUES (1); INSERT INTO c VALUES (1, 1)";

fn constraint_names(db: &Database, table: &str) -> (Vec<String>, Vec<String>) {
    let table = db.schema().get_table(table).unwrap();
    (
        table.checks.iter().map(|c| c.name.clone()).collect(),
        table.foreign_keys.iter().map(|f| f.name.clone()).collect(),
    )
}

fn untouched() -> (Vec<String>, Vec<String>) {
    (
        vec!["c_id_check".to_string()],
        vec!["c_pid_fkey".to_string()],
    )
}

/// Each `IF EXISTS` name no constraint of the table holds sends 18.6's notice, one per
/// written action — a name written twice under `IF EXISTS` drops once and notices once.
#[test]
fn an_if_exists_skip_notices_per_action() {
    let mut db = Database::with_btree();
    db.execute(SETUP).unwrap();
    let result = db
        .execute(
            "ALTER TABLE c DROP CONSTRAINT IF EXISTS nope, \
             DROP CONSTRAINT IF EXISTS c_pid_fkey, DROP CONSTRAINT IF EXISTS c_pid_fkey",
        )
        .unwrap()
        .remove(0);
    assert_eq!(result.command_tag, "ALTER TABLE");
    assert_eq!(
        result.notices,
        vec![
            "constraint \"nope\" of relation \"c\" does not exist, skipping".to_string(),
            "constraint \"c_pid_fkey\" of relation \"c\" does not exist, skipping".to_string(),
        ]
    );
    assert_eq!(
        constraint_names(&db, "c"),
        (vec!["c_id_check".to_string()], vec![])
    );
}

/// A list that fails on a later action — a missing name, or a kind pglet refuses —
/// drops nothing, on the autocommit path that cannot undo a catalog write: the earlier
/// actions' constraints are still recorded and still enforced.
#[test]
fn a_failing_list_leaves_the_catalog_untouched() {
    let mut db = Database::with_btree();
    db.execute(SETUP).unwrap();
    for (sql, code) in [
        (
            "ALTER TABLE c DROP CONSTRAINT c_pid_fkey, DROP CONSTRAINT c_id_check, \
             DROP CONSTRAINT nope",
            "42704",
        ),
        (
            "ALTER TABLE c DROP CONSTRAINT c_pid_fkey, DROP CONSTRAINT c_id_check, \
             DROP CONSTRAINT c_pid_fkey",
            "42704",
        ),
        (
            "ALTER TABLE c DROP CONSTRAINT c_pid_fkey, DROP CONSTRAINT c_id_check, \
             DROP CONSTRAINT IF EXISTS p_pkey, DROP CONSTRAINT c_id_not_null",
            "42704",
        ),
        ("ALTER TABLE p DROP CONSTRAINT p_pkey", "0A000"),
        ("ALTER TABLE p DROP CONSTRAINT p_id_not_null", "0A000"),
        (
            "ALTER TABLE c DROP CONSTRAINT c_pid_fkey, ADD COLUMN x INT",
            "0A000",
        ),
    ] {
        let err = db.execute(sql).unwrap_err();
        assert_eq!(err.sqlstate(), code, "{sql}");
        assert_eq!(constraint_names(&db, "c"), untouched(), "{sql}");
    }
    assert!(db.schema().get_index("p_pkey").is_some());
    assert_eq!(
        db.execute("INSERT INTO c VALUES (2, 99)")
            .unwrap_err()
            .sqlstate(),
        "23503"
    );
    assert_eq!(
        db.execute("INSERT INTO c VALUES (-1, 1)")
            .unwrap_err()
            .sqlstate(),
        "23514"
    );
}

/// A drop inside a block that rolls back comes back, enforcing again; one in a block
/// that commits stays dropped.
#[test]
fn a_rolled_back_drop_restores_the_constraint() {
    let mut db = Database::with_btree();
    db.execute(SETUP).unwrap();
    db.execute("BEGIN").unwrap();
    db.execute("ALTER TABLE c DROP CONSTRAINT c_pid_fkey, DROP CONSTRAINT c_id_check")
        .unwrap();
    db.execute("INSERT INTO c VALUES (-2, 99)").unwrap();
    db.execute("ROLLBACK").unwrap();
    assert_eq!(constraint_names(&db, "c"), untouched());
    assert_eq!(
        db.execute("INSERT INTO c VALUES (2, 99)")
            .unwrap_err()
            .sqlstate(),
        "23503"
    );

    db.execute("BEGIN").unwrap();
    db.execute("ALTER TABLE c DROP CONSTRAINT c_pid_fkey")
        .unwrap();
    db.execute("COMMIT").unwrap();
    db.execute("INSERT INTO c VALUES (2, 99)").unwrap();
}

/// The portable dump no longer emits a dropped constraint, and the constraint it still
/// emits is the one left.
#[test]
fn the_dump_leaves_a_dropped_constraint_out() {
    let mut db = Database::with_btree();
    db.execute(SETUP).unwrap();
    let before = db.dump_sql().unwrap();
    assert!(before.contains("c_pid_fkey"), "{before}");
    db.execute("ALTER TABLE c DROP CONSTRAINT c_pid_fkey")
        .unwrap();
    let after = db.dump_sql().unwrap();
    assert!(!after.contains("c_pid_fkey"), "{after}");
    assert!(after.contains("c_id_check"), "{after}");
}

/// A drop persists: a file-backed store reopened after an autocommit drop has neither
/// the record nor its enforcement, and deleting the parent no longer cascades.
#[test]
fn a_drop_survives_a_reopen() {
    let dir = ScratchDir::new("drop_constraint_reopen");
    {
        let mut db = Database::open(&dir.path).unwrap();
        db.execute(SETUP).unwrap();
        db.execute("ALTER TABLE c DROP CONSTRAINT IF EXISTS c_pid_fkey")
            .unwrap();
    }
    let mut db = Database::open(&dir.path).unwrap();
    assert_eq!(
        constraint_names(&db, "c"),
        (vec!["c_id_check".to_string()], vec![])
    );
    db.execute("INSERT INTO c VALUES (2, 99)").unwrap();
    db.execute("DELETE FROM p").unwrap();
    let rows = db
        .execute("SELECT id FROM c ORDER BY id")
        .unwrap()
        .remove(0);
    assert_eq!(rows.rows.len(), 2);
}

/// The drop moves the schema generation, the stamp a cached plan is keyed on, so a
/// plan held from before it is planned again.
#[test]
fn a_drop_moves_the_schema_generation() {
    let mut db = Database::with_btree();
    db.execute(SETUP).unwrap();
    let before = db.schema().generation;
    db.execute("ALTER TABLE c DROP CONSTRAINT c_pid_fkey")
        .unwrap();
    assert_ne!(db.schema().generation, before);
}

/// A name two constraints of the table hold — a `CHECK` declared under the name pglet
/// renders for a column's `NOT NULL` (`EDGES.md` row 290) — answers `0A000`, and both
/// constraints stand: the `CHECK` still enforces, and so does the `NOT NULL`. 18.6
/// named the `NOT NULL` `s_a_not_null1` and drops the `CHECK`. The lookup reaches the
/// `NOT NULL` first, whose own refusal answers here, so this pins the contract, not the
/// shared-name guard (no other same-name pair can be built today).
#[test]
fn a_name_two_constraints_hold_is_refused() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE s (a INT NOT NULL, CONSTRAINT s_a_not_null CHECK (a > 0))")
        .unwrap();
    let err = db
        .execute("ALTER TABLE s DROP CONSTRAINT s_a_not_null")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "0A000");
    assert_eq!(
        constraint_names(&db, "s"),
        (vec!["s_a_not_null".to_string()], vec![])
    );
    assert_eq!(
        db.execute("INSERT INTO s VALUES (-1)")
            .unwrap_err()
            .sqlstate(),
        "23514"
    );
    assert_eq!(
        db.execute("INSERT INTO s VALUES (NULL)")
            .unwrap_err()
            .sqlstate(),
        "23502"
    );
}
