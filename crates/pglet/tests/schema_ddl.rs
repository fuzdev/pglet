//! `DROP SCHEMA` / `CREATE SCHEMA` over the embed engine — the migration/test
//! reset idiom (`DROP SCHEMA public CASCADE; CREATE SCHEMA public`) fuz_app's test
//! harness (`testing/db.ts`'s `reset_pglite`) issues between tests, plus the
//! fail-loud boundaries.
//!
//! Runs over the in-memory CoW B+tree (`Database::with_btree`) — the same backend
//! the spine's `pglet_server` uses — so `DROP SCHEMA` exercises the real
//! per-table/index storage drop, not a memory no-op.
//!
//! The differential fixtures (`tests/fixtures/ddl/{drop,create}_schema_*.sql`)
//! assert the same shapes against a live Postgres; this is the in-process
//! counterpart.

use pglet::Database;
use pglet_sql::types::Value;

/// `DROP SCHEMA public CASCADE` clears every catalog object — tables, enums,
/// indexes, and sequences — and their storage, so the namespace is empty and
/// fully re-creatable afterward (the reset between two consecutive test runs).
#[test]
fn drop_schema_cascade_clears_everything() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE vis AS ENUM ('a', 'b')").unwrap();
    db.execute("CREATE TABLE t (id BIGSERIAL PRIMARY KEY, v vis, tags TEXT[])")
        .unwrap();
    db.execute("CREATE INDEX t_tags_gin ON t USING gin (tags)")
        .unwrap();
    db.execute("INSERT INTO t (v, tags) VALUES ('a', ARRAY['x'])")
        .unwrap();

    db.execute("DROP SCHEMA public CASCADE").unwrap();

    // The table is gone from the catalog (a query against it fails loud).
    assert!(db.execute("SELECT * FROM t").is_err());

    // The reset idiom's second half: re-create the (always-implicit) public schema.
    db.execute("CREATE SCHEMA public").unwrap();

    // The enum is gone — re-creating it (same name) succeeds with no duplicate
    // (DROP SCHEMA cleared `schema.enums`).
    db.execute("CREATE TYPE vis AS ENUM ('a', 'b')").unwrap();

    // The table name is free again, and the implicit SERIAL sequence restarted, so
    // the first insert's id is 1 (DROP SCHEMA cleared `schema.sequences`). No old
    // rows leak in (the storage drop ran), so the table holds exactly the new row.
    db.execute("CREATE TABLE t (id BIGSERIAL PRIMARY KEY, v vis)")
        .unwrap();
    db.execute("INSERT INTO t (v) VALUES ('b')").unwrap();
    let mut res = db.execute("SELECT id, v::text FROM t").unwrap();
    assert_eq!(
        res.remove(0).rows,
        vec![vec![Value::Int8(1), Value::Text("b".into())]]
    );
}

/// `CREATE SCHEMA public` (and the `IF NOT EXISTS` form) is a no-op success —
/// pglet's `public` is always present, and the reset idiom needs the create to
/// succeed after the preceding drop.
#[test]
fn create_schema_public_is_noop() {
    let mut db = Database::with_btree();
    db.execute("CREATE SCHEMA public").unwrap();
    db.execute("CREATE SCHEMA IF NOT EXISTS public").unwrap();
    // The schema is usable: a plain table round-trips.
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();
    let mut res = db.execute("SELECT id FROM t").unwrap();
    assert_eq!(res.remove(0).rows, vec![vec![Value::Int4(1)]]);
}

/// Postgres rejects `DROP SCHEMA public` (no `CASCADE`) while objects remain
/// (`2BP01`); pglet matches rather than silently emptying the catalog.
#[test]
fn drop_schema_nonempty_without_cascade_is_loud() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    let err = db.execute("DROP SCHEMA public").unwrap_err();
    assert_eq!(err.sqlstate(), "2BP01", "{err}");
    // The table survived the rejected drop.
    db.execute("INSERT INTO t VALUES (1)").unwrap();
    assert_eq!(
        db.execute("SELECT id FROM t").unwrap().remove(0).rows,
        vec![vec![Value::Int4(1)]]
    );
}

/// A sequence is an object of the schema on its own: over a `public` holding one and
/// nothing else, `DROP SCHEMA public` and its `RESTRICT` spelling are `2BP01`, and the
/// sequence keeps its counter — 18.6 answers `2` after them, where a drop that went
/// ahead would leave the name unresolvable (`42P01`).
#[test]
fn drop_schema_holding_only_a_sequence_without_cascade_is_loud() {
    let mut db = Database::with_btree();
    db.execute("CREATE SEQUENCE s").unwrap();
    db.execute("SELECT nextval('s')").unwrap();
    for sql in ["DROP SCHEMA public", "DROP SCHEMA public RESTRICT"] {
        let err = db.execute(sql).unwrap_err();
        assert_eq!(err.sqlstate(), "2BP01", "{sql}: {err}");
    }
    assert_eq!(
        db.execute("SELECT nextval('s')").unwrap().remove(0).rows,
        vec![vec![Value::Int8(2)]]
    );
}

/// An empty schema drops without `CASCADE` (Postgres allows it) — and a bare
/// `RESTRICT` is accepted (the default).
#[test]
fn drop_schema_empty_without_cascade_succeeds() {
    let mut db = Database::with_btree();
    db.execute("DROP SCHEMA public").unwrap();
    db.execute("DROP SCHEMA public RESTRICT").unwrap();
}

/// A non-`public` schema is "not found": `DROP SCHEMA other` fails loud (`3F000`),
/// while `DROP SCHEMA IF EXISTS other` is a no-op success (matching Postgres).
#[test]
fn drop_schema_nonpublic() {
    let mut db = Database::with_btree();
    let err = db.execute("DROP SCHEMA other").unwrap_err();
    assert_eq!(err.sqlstate(), "3F000", "{err}");
    db.execute("DROP SCHEMA IF EXISTS other").unwrap();
}

/// `CREATE SCHEMA other` fails loud — pglet models only the implicit `public`
/// schema (a deliberately stricter-than-Postgres boundary).
#[test]
fn create_schema_nonpublic_is_loud() {
    let mut db = Database::with_btree();
    let err = db.execute("CREATE SCHEMA other").unwrap_err();
    // NotImplemented → 0A000 (feature_not_supported).
    assert_eq!(err.sqlstate(), "0A000", "{err}");
}

/// The full reset round-trip the spine harness performs between tests: build a
/// schema, `DROP SCHEMA public CASCADE; CREATE SCHEMA public`, then rebuild and
/// query — proving a fresh server-instance-free reset works end-to-end.
#[test]
fn reset_idiom_roundtrips() {
    let mut db = Database::with_btree();
    // First "test" run: a schema with data.
    db.execute("CREATE TABLE account (id INT PRIMARY KEY, name TEXT)")
        .unwrap();
    db.execute("INSERT INTO account VALUES (1, 'old')").unwrap();

    // Reset (issued as one batch by the harness's `batch_execute`).
    db.execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .unwrap();

    // Second "test" run: the same DDL re-applies cleanly against the empty schema.
    db.execute("CREATE TABLE account (id INT PRIMARY KEY, name TEXT)")
        .unwrap();
    db.execute("INSERT INTO account VALUES (2, 'new')").unwrap();
    let mut res = db
        .execute("SELECT id, name FROM account ORDER BY id")
        .unwrap();
    assert_eq!(
        res.remove(0).rows,
        vec![vec![Value::Int4(2), Value::Text("new".into())]]
    );
}
