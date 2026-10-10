//! Inline `FOREIGN KEY` capture on `ALTER TABLE … ADD COLUMN … REFERENCES …`,
//! end-to-end over the embed engine (parse → plan → execute).
//!
//! Mirrors the spine's auth v1 migration line
//! (`ALTER TABLE role_grant ADD COLUMN … source_offer_id UUID NULL REFERENCES
//! role_grant_offer(id) ON DELETE SET NULL`): the FK declared on the *added*
//! column must be captured onto the table's `foreign_keys` so the standard
//! write-time enforcement and `ON DELETE` engine cover it — no parallel
//! enforcement path. The differential SQL fixtures under `tests/fixtures/fk/`
//! (`alter_add_column_fk_*`) assert the same shapes against a live Postgres;
//! this is the in-process counterpart that runs without a server.

use pglet::Database;
use pglet_sql::catalog::ReferentialAction;

/// The ALTER-added FK lands on the table's catalog `foreign_keys` list, with the
/// expected auto-name / parent / on-delete action.
#[test]
fn alter_added_fk_is_captured_in_catalog() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE fk_parent (id INT PRIMARY KEY)")
        .unwrap();
    db.execute("CREATE TABLE fk_child (id INT PRIMARY KEY)")
        .unwrap();
    db.execute(
        "ALTER TABLE fk_child ADD COLUMN p_id INT REFERENCES fk_parent(id) ON DELETE SET NULL",
    )
    .unwrap();

    let table = db.schema().get_table("fk_child").expect("table exists");
    assert_eq!(
        table.foreign_keys.len(),
        1,
        "the ALTER-added FK is captured"
    );
    let fk = &table.foreign_keys[0];
    assert_eq!(fk.name, "fk_child_p_id_fkey");
    assert_eq!(fk.columns, vec!["p_id".to_string()]);
    assert_eq!(fk.parent_table, "fk_parent");
    assert_eq!(fk.parent_columns, vec!["id".to_string()]);
    assert_eq!(fk.on_delete, ReferentialAction::SetNull);
}

/// Inserting a child whose ALTER-added FK column points at a missing parent is
/// rejected (the standard `enforce_foreign_keys` path, unchanged).
#[test]
fn alter_added_fk_rejects_missing_parent() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE fk_parent (id INT PRIMARY KEY)")
        .unwrap();
    db.execute("CREATE TABLE fk_child (id INT PRIMARY KEY)")
        .unwrap();
    db.execute("INSERT INTO fk_parent VALUES (1)").unwrap();
    db.execute("ALTER TABLE fk_child ADD COLUMN p_id INT REFERENCES fk_parent(id)")
        .unwrap();

    // Parent 2 does not exist — the FK existence probe must reject this.
    let err = db.execute("INSERT INTO fk_child VALUES (10, 2)");
    assert!(
        err.is_err(),
        "insert referencing a missing parent should violate the ALTER-added FK"
    );

    // A matching parent (and a NULL FK value, which a FK always permits) inserts.
    db.execute("INSERT INTO fk_child VALUES (11, 1)").unwrap();
    db.execute("INSERT INTO fk_child VALUES (12, NULL)")
        .unwrap();
}

/// `ON DELETE SET NULL` on an ALTER-added FK column nulls the child when the
/// parent is deleted — the spine's `source_offer_id` shape.
#[test]
fn alter_added_fk_on_delete_set_null_nulls_the_child() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE fk_parent (id INT PRIMARY KEY)")
        .unwrap();
    db.execute("CREATE TABLE fk_child (id INT PRIMARY KEY)")
        .unwrap();
    db.execute("INSERT INTO fk_parent VALUES (1)").unwrap();
    db.execute(
        "ALTER TABLE fk_child ADD COLUMN p_id INT REFERENCES fk_parent(id) ON DELETE SET NULL",
    )
    .unwrap();
    db.execute("INSERT INTO fk_child VALUES (10, 1)").unwrap();

    db.execute("DELETE FROM fk_parent WHERE id = 1").unwrap();

    let mut results = db
        .execute("SELECT p_id FROM fk_child WHERE id = 10")
        .unwrap();
    let result = results.remove(0);
    assert_eq!(
        result.rows[0][0],
        pglet_sql::types::Value::Null,
        "ON DELETE SET NULL on the ALTER-added FK column nulls the child"
    );
}
