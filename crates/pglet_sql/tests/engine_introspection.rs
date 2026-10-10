//! Engine integration tests: catalog introspection (`information_schema`).

use crate::common;
use common::*;
use pglet_sql::types::Value;

/// The migration runner's `detect_old_tracker` probe shape — a scalar `EXISTS`
/// over `information_schema.columns`. It must see the columns that exist and not
/// the ones that don't.
#[test]
fn test_information_schema_columns_exists_probe() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE schema_version (namespace TEXT, name TEXT, sequence INTEGER)");

    let r = db.run(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'schema_version' AND column_name = 'namespace') AS has_namespace, EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'schema_version' AND column_name = 'version') AS has_version",
    );
    assert_eq!(r.rows[0][0], Value::Bool(true), "namespace column exists");
    assert_eq!(r.rows[0][1], Value::Bool(false), "version column absent");
}

/// Projecting the synthesized rows: names, ordinal positions, nullability, and
/// the canonical `data_type` mapping (incl. an array → `ARRAY`).
#[test]
fn test_information_schema_columns_rows() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT NOT NULL, name TEXT, tags TEXT[], created TIMESTAMPTZ)");

    let r = db.run(
        "SELECT column_name, data_type, is_nullable, ordinal_position FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 't' ORDER BY ordinal_position",
    );
    assert_eq!(r.rows.len(), 4);
    // id INT NOT NULL
    assert_eq!(r.rows[0][0], Value::Text("id".into()));
    assert_eq!(r.rows[0][1], Value::Text("integer".into()));
    assert_eq!(r.rows[0][2], Value::Text("NO".into()));
    assert_eq!(r.rows[0][3], Value::Int4(1));
    // name TEXT (nullable)
    assert_eq!(r.rows[1][0], Value::Text("name".into()));
    assert_eq!(r.rows[1][1], Value::Text("text".into()));
    assert_eq!(r.rows[1][2], Value::Text("YES".into()));
    // tags TEXT[] reports ARRAY
    assert_eq!(r.rows[2][1], Value::Text("ARRAY".into()));
    // created TIMESTAMPTZ reports the canonical SQL-standard name
    assert_eq!(r.rows[3][1], Value::Text("timestamp with time zone".into()));
}

/// A bare user table named `columns` resolves to itself, never the view — the
/// view requires the `information_schema.` qualifier.
#[test]
fn test_information_schema_does_not_shadow_user_table() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE columns (id INT)");
    db.run("INSERT INTO columns VALUES (42)");

    let r = db.run("SELECT id FROM columns");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Value::Int4(42));
}

/// The auth migration's idempotent-ADD-CONSTRAINT probe shape: look up a named
/// CHECK in `information_schema.table_constraints` (added via ALTER), then the
/// absent case (so the migration knows whether to ADD).
#[test]
fn test_table_constraints_check_lookup() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE rg (scope_kind TEXT, scope_id UUID)");
    db.run(
        "ALTER TABLE rg ADD CONSTRAINT rg_pair CHECK ((scope_kind IS NULL) = (scope_id IS NULL))",
    );

    let present = db.run(
        "SELECT constraint_type FROM information_schema.table_constraints WHERE table_name = 'rg' AND constraint_name = 'rg_pair'",
    );
    assert_eq!(present.rows.len(), 1);
    assert_eq!(present.rows[0][0], Value::Text("CHECK".into()));

    let absent = db.run(
        "SELECT 1 FROM information_schema.table_constraints WHERE table_name = 'rg' AND constraint_name = 'nope'",
    );
    assert!(absent.rows.is_empty());
}

/// FOREIGN KEY constraints appear with their declared name and type.
#[test]
fn test_table_constraints_foreign_key() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (id INT PRIMARY KEY)");
    db.run("CREATE TABLE child (id INT, parent_id INT, CONSTRAINT child_fk FOREIGN KEY (parent_id) REFERENCES parent(id))");

    let r = db.run(
        "SELECT constraint_type FROM information_schema.table_constraints WHERE table_name = 'child' AND constraint_name = 'child_fk'",
    );
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Value::Text("FOREIGN KEY".into()));
}
