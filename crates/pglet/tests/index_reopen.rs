//! Reopen test: a secondary index built through `CREATE INDEX` must survive a
//! file-backed `Database::open` round-trip — both its persisted entries AND its
//! in-memory def re-registration.
//!
//! This drives the real native constructor (`Database::open`), the only path
//! that runs `reattach_persisted_indexes`. The `double_meta.rs`/`crash.rs`
//! helpers assemble a `Database` by hand and intentionally bypass it, so they
//! could not prove re-registration; this test must use `open` on a real dir.
//!
//! The maintenance proof is the key part: after reopen we INSERT a *new* row and
//! confirm the index reflects it. Old entries persisting in the tree would pass
//! a probe-only check even if re-registration were broken — but a post-reopen
//! insert is only indexed if the wrapper knows which column to maintain, which
//! is exactly what `reattach_index` restores.

// A test harness: the `probe` helper `panic!`s on the wrong backend and `.expect()`s the
// lookup so a setup failure surfaces loudly here.
#![allow(clippy::panic, clippy::expect_used)]

use crate::common;

use common::ScratchDir;
use pglet::{Database, StorageBackend};
use pglet_sql::types::Value;
// `index_lookup` on the wrapper is the `Storage` trait method, not inherent.
use pglet_storage::Storage;

/// Probe the index `oid` over the active backend for rows whose key equals `value`.
fn probe(db: &Database, oid: u32, value: &Value) -> Vec<u64> {
    let StorageBackend::Btree(wrapper) = db.storage() else {
        panic!("expected a btree backend");
    };
    let mut ids = wrapper
        .index_lookup(oid, std::slice::from_ref(value))
        .expect("index_lookup");
    ids.sort_unstable();
    ids
}

#[test]
fn index_survives_reopen_and_stays_maintained() {
    let dir = ScratchDir::new("maintained");

    // First open: build table + index, seed rows, then drop the Database
    // (auto-commit persists each statement to the meta slot).
    let oid = {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE t (id INT, k INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1, 10), (2, 20), (3, 10), (4, 30)")
            .unwrap();
        db.execute("CREATE INDEX ix_k ON t(k)").unwrap();

        let oid = db.schema().get_index("ix_k").expect("index in catalog").oid as u32;

        // Sanity: the index probes correctly before reopen.
        assert_eq!(probe(&db, oid, &Value::Int4(10)), vec![0, 2]);
        assert_eq!(probe(&db, oid, &Value::Int4(20)), vec![1]);
        oid
    };

    // Reopen over the same dir. `Database::open` restores the schema (incl. the
    // index) and re-registers the index def with the wrapper.
    let mut db = Database::open(&dir.path).expect("reopen");

    // The catalog index survived the schema round-trip.
    let reopened_oid = db.schema().get_index("ix_k").expect("index in catalog").oid as u32;
    assert_eq!(reopened_oid, oid, "index oid stable across reopen");

    // (a) Probe gets the right rows from the persisted entries.
    assert_eq!(
        probe(&db, oid, &Value::Int4(10)),
        vec![0, 2],
        "persisted index entries survive reopen"
    );
    assert_eq!(probe(&db, oid, &Value::Int4(30)), vec![3]);
    assert!(probe(&db, oid, &Value::Int4(99)).is_empty());

    // (b) INSERT a new row after reopen and confirm the index is MAINTAINED.
    // This only works if the wrapper re-registered the index def — a probe of
    // stale persisted entries alone would miss this new row.
    db.execute("INSERT INTO t VALUES (5, 10)").unwrap();
    assert_eq!(
        probe(&db, oid, &Value::Int4(10)),
        vec![0, 2, 4],
        "post-reopen insert must be indexed (re-registration, not just persistence)"
    );

    // UPDATE/DELETE after reopen are likewise maintained.
    db.execute("UPDATE t SET k = 20 WHERE id = 1").unwrap();
    assert_eq!(probe(&db, oid, &Value::Int4(10)), vec![2, 4]);
    assert!(probe(&db, oid, &Value::Int4(20)).contains(&0));

    db.execute("DELETE FROM t WHERE id = 3").unwrap();
    assert_eq!(probe(&db, oid, &Value::Int4(10)), vec![4]);
}

#[test]
fn expression_index_enforces_survives_reopen_and_stays_maintained() {
    let dir = ScratchDir::new("expr");

    // First open: a UNIQUE expression index over LOWER(email). Enforcement,
    // maintenance, and the computed-key probe must all work end-to-end.
    let oid = {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE acct (id INT, email TEXT)")
            .unwrap();
        db.execute("CREATE UNIQUE INDEX ux_email ON acct (LOWER(email))")
            .unwrap();
        db.execute("INSERT INTO acct VALUES (1, 'A@x'), (2, 'b@x')")
            .unwrap();

        // Enforcement keys on the COMPUTED value: 'a@x' collides with 'A@x'.
        let err = db
            .execute("INSERT INTO acct VALUES (3, 'a@x')")
            .expect_err("LOWER(email) collision must be rejected");
        assert!(
            format!("{err}").contains("ux_email"),
            "unique violation names the expression index, got: {err}"
        );

        let oid = db.schema().get_index("ux_email").expect("index").oid as u32;
        // The probe key is the computed (lowercased) value, so 'a@x' finds the
        // 'A@x' row.
        assert_eq!(probe(&db, oid, &Value::Text("a@x".into())), vec![0]);
        oid
    };

    // Reopen: `reattach_persisted_indexes` must restore the EXPRESSION def (its
    // key parts), not just position-resolve column names (which would silently
    // skip it, since "LOWER(email)" is not a column).
    let mut db = Database::open(&dir.path).expect("reopen");
    let reopened = db.schema().get_index("ux_email").expect("index");
    assert!(
        reopened.key.is_expression(),
        "expression key survives reopen"
    );
    assert_eq!(reopened.oid as u32, oid, "oid stable across reopen");

    // (a) Persisted computed-key entries survive reopen.
    assert_eq!(probe(&db, oid, &Value::Text("a@x".into())), vec![0]);
    assert_eq!(probe(&db, oid, &Value::Text("b@x".into())), vec![1]);

    // (b) Enforcement still works after reopen (re-registered def maintains it).
    let err = db
        .execute("INSERT INTO acct VALUES (4, 'A@X')")
        .expect_err("post-reopen LOWER collision must be rejected");
    assert!(format!("{err}").contains("ux_email"), "got: {err}");

    // (c) A distinct computed key inserts and is maintained. Rejected inserts
    // don't consume row IDs, so after the two committed rows (rowids 0,1) this is
    // rowid 2.
    db.execute("INSERT INTO acct VALUES (5, 'c@x')").unwrap();
    assert_eq!(probe(&db, oid, &Value::Text("c@x".into())), vec![2]);
}
