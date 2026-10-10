//! Parent-side FK `ON UPDATE` referential actions, end-to-end over the embed
//! engine (parse → plan → execute), plus the store-magic round-trip that the new
//! persisted `ForeignKey.on_update` field requires.
//!
//! The differential SQL fixtures under `tests/fixtures/fk/` (`on_update_*`) assert
//! the same shapes byte-for-byte against a live Postgres; this is the in-process
//! counterpart that runs without a server and additionally proves durability of the
//! `on_update` catalog field across a file-backed reopen (the `PGMB` store bump).

// A test harness: `.unwrap()`/`.expect()` surface a setup failure loudly here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::catalog::ReferentialAction;
use pglet_sql::types::Value;

/// Read a single scalar cell from a one-statement query.
fn scalar(db: &mut Database, sql: &str) -> Value {
    let mut results = db.execute(sql).expect("query");
    let result = results.remove(0);
    result.rows[0][0].clone()
}

/// The default (NO ACTION) FK: updating a parent's referenced key while a child
/// still points at the old value orphans the child → `23503`.
#[test]
fn default_no_action_parent_key_update_is_rejected() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE c (id INT PRIMARY KEY, parent_id INT REFERENCES p(id))")
        .unwrap();
    db.execute("INSERT INTO p VALUES (1)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

    let err = db
        .execute("UPDATE p SET id = 2 WHERE id = 1")
        .expect_err("orphaning a child must be rejected");
    assert_eq!(err.sqlstate(), "23503", "NO ACTION → foreign_key_violation");

    // Storage untouched: the parent key is unchanged and the child still resolves.
    assert_eq!(scalar(&mut db, "SELECT id FROM p"), Value::Int4(1));
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 10"),
        Value::Int4(1)
    );
}

/// A parent-key update with no referencing child succeeds.
#[test]
fn parent_key_update_with_no_child_succeeds() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE c (id INT PRIMARY KEY, parent_id INT REFERENCES p(id))")
        .unwrap();
    db.execute("INSERT INTO p VALUES (1)").unwrap();

    db.execute("UPDATE p SET id = 2 WHERE id = 1").unwrap();
    assert_eq!(scalar(&mut db, "SELECT id FROM p"), Value::Int4(2));
}

/// `ON UPDATE CASCADE` repoints the child's FK column to the new parent key.
#[test]
fn on_update_cascade_repoints_child() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute(
        "CREATE TABLE c (id INT PRIMARY KEY, parent_id INT REFERENCES p(id) ON UPDATE CASCADE)",
    )
    .unwrap();
    db.execute("INSERT INTO p VALUES (1)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1), (11, 1)").unwrap();

    db.execute("UPDATE p SET id = 2 WHERE id = 1").unwrap();

    assert_eq!(scalar(&mut db, "SELECT id FROM p"), Value::Int4(2));
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 10"),
        Value::Int4(2)
    );
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 11"),
        Value::Int4(2)
    );
}

/// `ON UPDATE RESTRICT` rejects with Postgres's distinct `23001` (not `23503`).
#[test]
fn on_update_restrict_rejects_with_23001() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute(
        "CREATE TABLE c (id INT PRIMARY KEY, parent_id INT REFERENCES p(id) ON UPDATE RESTRICT)",
    )
    .unwrap();
    db.execute("INSERT INTO p VALUES (1)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

    let err = db
        .execute("UPDATE p SET id = 2 WHERE id = 1")
        .expect_err("RESTRICT must reject");
    assert_eq!(err.sqlstate(), "23001", "RESTRICT → restrict_violation");
}

/// `ON UPDATE SET NULL` nulls the child's FK column.
#[test]
fn on_update_set_null_nulls_child() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute(
        "CREATE TABLE c (id INT PRIMARY KEY, parent_id INT REFERENCES p(id) ON UPDATE SET NULL)",
    )
    .unwrap();
    db.execute("INSERT INTO p VALUES (1)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

    db.execute("UPDATE p SET id = 2 WHERE id = 1").unwrap();

    assert_eq!(scalar(&mut db, "SELECT id FROM p"), Value::Int4(2));
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 10"),
        Value::Null
    );
}

/// `ON UPDATE SET NULL` onto a NOT NULL child column aborts (`23502`), and the
/// statement's rollback leaves parent and child as they were.
#[test]
fn on_update_set_null_onto_not_null_aborts() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute(
        "CREATE TABLE c (id INT PRIMARY KEY, \
         parent_id INT NOT NULL REFERENCES p(id) ON UPDATE SET NULL)",
    )
    .unwrap();
    db.execute("INSERT INTO p VALUES (1)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

    let err = db
        .execute("UPDATE p SET id = 2 WHERE id = 1")
        .expect_err("SET NULL onto NOT NULL must abort");
    assert_eq!(err.sqlstate(), "23502", "not_null_violation");

    // The whole statement rolled back: parent key and child are unchanged.
    assert_eq!(scalar(&mut db, "SELECT id FROM p"), Value::Int4(1));
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 10"),
        Value::Int4(1)
    );
}

/// A composite `ON UPDATE CASCADE` repoints every paired FK column.
#[test]
fn composite_on_update_cascade_repoints_all_columns() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (x INT, y INT, PRIMARY KEY (x, y))")
        .unwrap();
    db.execute(
        "CREATE TABLE c (id INT PRIMARY KEY, a INT, b INT, \
         FOREIGN KEY (a, b) REFERENCES p (x, y) ON UPDATE CASCADE)",
    )
    .unwrap();
    db.execute("INSERT INTO p VALUES (1, 2)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1, 2)").unwrap();

    db.execute("UPDATE p SET x = 3, y = 4 WHERE x = 1 AND y = 2")
        .unwrap();

    assert_eq!(
        scalar(&mut db, "SELECT a FROM c WHERE id = 10"),
        Value::Int4(3)
    );
    assert_eq!(
        scalar(&mut db, "SELECT b FROM c WHERE id = 10"),
        Value::Int4(4)
    );
}

/// A self-referential FK hierarchy: updating an interior node's key cascades to the
/// rows that reference it (the updated node does not reference the old key itself).
#[test]
fn self_referential_hierarchy_on_update_cascade() {
    let mut db = Database::with_btree();
    db.execute(
        "CREATE TABLE t (id INT PRIMARY KEY, parent_id INT REFERENCES t(id) ON UPDATE CASCADE)",
    )
    .unwrap();
    // Root (1) with NULL parent; child (2) points at 1.
    db.execute("INSERT INTO t VALUES (1, NULL)").unwrap();
    db.execute("INSERT INTO t VALUES (2, 1)").unwrap();

    db.execute("UPDATE t SET id = 10 WHERE id = 1").unwrap();

    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM t WHERE id = 2"),
        Value::Int4(10)
    );
    assert_eq!(
        scalar(&mut db, "SELECT count(*) FROM t WHERE id = 10"),
        Value::Int8(1)
    );
}

/// Updating a non-referenced column (or a `SET id = id` no-op) never triggers the
/// parent-side FK path even with a live child.
#[test]
fn non_key_update_does_not_trigger_fk_action() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY, name TEXT)")
        .unwrap();
    db.execute("CREATE TABLE c (id INT PRIMARY KEY, parent_id INT REFERENCES p(id))")
        .unwrap();
    db.execute("INSERT INTO p VALUES (1, 'a')").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

    // Non-referenced column update and a no-op key update both succeed.
    db.execute("UPDATE p SET name = 'b' WHERE id = 1").unwrap();
    db.execute("UPDATE p SET id = id WHERE id = 1").unwrap();

    assert_eq!(
        scalar(&mut db, "SELECT name FROM p WHERE id = 1"),
        Value::Text("b".into())
    );
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 10"),
        Value::Int4(1)
    );
}

/// A multi-row parent UPDATE that cascades to each row's distinct children.
#[test]
fn multi_row_parent_update_cascades_each() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute(
        "CREATE TABLE c (id INT PRIMARY KEY, parent_id INT REFERENCES p(id) ON UPDATE CASCADE)",
    )
    .unwrap();
    db.execute("INSERT INTO p VALUES (1), (2)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1), (20, 2)").unwrap();

    // 1 -> 101, 2 -> 102 (no transient unique collision).
    db.execute("UPDATE p SET id = id + 100").unwrap();

    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 10"),
        Value::Int4(101)
    );
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 20"),
        Value::Int4(102)
    );
}

/// Two-level ON UPDATE CASCADE is **transitive**: `a ← b ← cc`, updating `a`'s key
/// repoints `b`, and `b`'s changed key in turn repoints `cc` (no silent grandchild
/// orphan).
#[test]
fn two_level_cascade_is_transitive() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE a (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE b (aid INT PRIMARY KEY REFERENCES a(id) ON UPDATE CASCADE)")
        .unwrap();
    db.execute("CREATE TABLE cc (bid INT PRIMARY KEY REFERENCES b(aid) ON UPDATE CASCADE)")
        .unwrap();
    db.execute("INSERT INTO a VALUES (1)").unwrap();
    db.execute("INSERT INTO b VALUES (1)").unwrap();
    db.execute("INSERT INTO cc VALUES (1)").unwrap();

    db.execute("UPDATE a SET id = 2").unwrap();

    assert_eq!(scalar(&mut db, "SELECT id FROM a"), Value::Int4(2));
    assert_eq!(scalar(&mut db, "SELECT aid FROM b"), Value::Int4(2));
    assert_eq!(
        scalar(&mut db, "SELECT bid FROM cc"),
        Value::Int4(2),
        "the grandchild must follow transitively, not stay orphaned at 1"
    );
}

/// Three-level ON UPDATE CASCADE chain propagates all the way down.
#[test]
fn three_level_cascade_is_transitive() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE a (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE b (aid INT PRIMARY KEY REFERENCES a(id) ON UPDATE CASCADE)")
        .unwrap();
    db.execute("CREATE TABLE cc (bid INT PRIMARY KEY REFERENCES b(aid) ON UPDATE CASCADE)")
        .unwrap();
    db.execute("CREATE TABLE d (cid INT PRIMARY KEY REFERENCES cc(bid) ON UPDATE CASCADE)")
        .unwrap();
    db.execute("INSERT INTO a VALUES (1)").unwrap();
    db.execute("INSERT INTO b VALUES (1)").unwrap();
    db.execute("INSERT INTO cc VALUES (1)").unwrap();
    db.execute("INSERT INTO d VALUES (1)").unwrap();

    db.execute("UPDATE a SET id = 9").unwrap();

    assert_eq!(scalar(&mut db, "SELECT aid FROM b"), Value::Int4(9));
    assert_eq!(scalar(&mut db, "SELECT bid FROM cc"), Value::Int4(9));
    assert_eq!(scalar(&mut db, "SELECT cid FROM d"), Value::Int4(9));
}

/// SET NULL at a middle level propagates: `a ← b (SET NULL) ← cc (CASCADE)` — nulling
/// `b`'s key cascades that change (OLD → NULL) onto the grandchild.
#[test]
fn set_null_middle_level_propagates() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE a (id INT PRIMARY KEY)").unwrap();
    db.execute(
        "CREATE TABLE b (bid INT PRIMARY KEY, aid INT UNIQUE REFERENCES a(id) ON UPDATE SET NULL)",
    )
    .unwrap();
    db.execute(
        "CREATE TABLE cc (cid INT PRIMARY KEY, bref INT REFERENCES b(aid) ON UPDATE CASCADE)",
    )
    .unwrap();
    db.execute("INSERT INTO a VALUES (1)").unwrap();
    db.execute("INSERT INTO b VALUES (10, 1)").unwrap();
    db.execute("INSERT INTO cc VALUES (100, 1)").unwrap();

    db.execute("UPDATE a SET id = 2").unwrap();

    assert_eq!(
        scalar(&mut db, "SELECT aid FROM b WHERE bid = 10"),
        Value::Null
    );
    assert_eq!(
        scalar(&mut db, "SELECT bref FROM cc WHERE cid = 100"),
        Value::Null,
        "the grandchild follows b.aid from 1 to NULL"
    );
}

/// A CASCADE repoint that would violate the child's CHECK is rejected (`23514`) before
/// the child is written, and the statement's rollback leaves the parent as it was.
#[test]
fn cascade_repoint_violating_check_is_rejected() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE c (pid INT REFERENCES p(id) ON UPDATE CASCADE CHECK (pid < 100))")
        .unwrap();
    db.execute("INSERT INTO p VALUES (5)").unwrap();
    db.execute("INSERT INTO c VALUES (5)").unwrap();

    let err = db
        .execute("UPDATE p SET id = 200 WHERE id = 5")
        .expect_err("repoint to 200 violates CHECK (pid < 100)");
    assert_eq!(err.sqlstate(), "23514", "check_violation");

    // Storage untouched: parent key and child unchanged.
    assert_eq!(scalar(&mut db, "SELECT id FROM p"), Value::Int4(5));
    assert_eq!(scalar(&mut db, "SELECT pid FROM c"), Value::Int4(5));
}

/// A CASCADE repoint that still satisfies the child's CHECK succeeds.
#[test]
fn cascade_repoint_satisfying_check_succeeds() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE c (pid INT REFERENCES p(id) ON UPDATE CASCADE CHECK (pid < 100))")
        .unwrap();
    db.execute("INSERT INTO p VALUES (5)").unwrap();
    db.execute("INSERT INTO c VALUES (5)").unwrap();

    db.execute("UPDATE p SET id = 50 WHERE id = 5").unwrap();
    assert_eq!(scalar(&mut db, "SELECT pid FROM c"), Value::Int4(50));
}

/// ON UPDATE RESTRICT never accepts a substitute row: `p = {1, 2}`, `c` references 1;
/// a swap where row 2 refills key 1 still raises `23001` (unlike NO ACTION).
#[test]
fn restrict_rejects_even_with_substitute_row() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE c (id INT PRIMARY KEY, pid INT REFERENCES p(id) ON UPDATE RESTRICT)")
        .unwrap();
    db.execute("INSERT INTO p VALUES (1), (2)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

    // Row 1 vacates key 1; row 2 refills it. RESTRICT rejects regardless.
    let err = db
        .execute("UPDATE p SET id = CASE id WHEN 1 THEN 3 WHEN 2 THEN 1 END")
        .expect_err("RESTRICT never substitutes");
    assert_eq!(err.sqlstate(), "23001");
}

/// NO ACTION *does* accept a substitute row (the swap that leaves every child with a
/// parent) — must not regress.
#[test]
fn no_action_accepts_substitute_row() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE c (id INT PRIMARY KEY, pid INT REFERENCES p(id))")
        .unwrap();
    db.execute("INSERT INTO p VALUES (1), (2)").unwrap();
    db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

    // Row 1 vacates key 1; row 2 refills it. NO ACTION accepts (child still has parent).
    db.execute("UPDATE p SET id = CASE id WHEN 1 THEN 3 WHEN 2 THEN 1 END")
        .unwrap();
    // The child still resolves to a live parent keyed 1.
    assert_eq!(
        scalar(&mut db, "SELECT pid FROM c WHERE id = 10"),
        Value::Int4(1)
    );
    assert_eq!(
        scalar(&mut db, "SELECT count(*) FROM p WHERE id = 1"),
        Value::Int8(1)
    );
}

/// The persisted `ForeignKey.on_update` field survives a file-backed reopen (the
/// `PGMB` store bump), and a post-reopen parent-key UPDATE still fires the cascade —
/// proving the catalog field round-trips through the double-meta store.
#[test]
fn on_update_action_survives_reopen_and_still_cascades() {
    let dir = ScratchDir::new("fk_on_update_reopen");

    // First open: declare an ON UPDATE CASCADE FK and seed a child.
    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE p (id INT PRIMARY KEY)").unwrap();
        db.execute(
            "CREATE TABLE c (id INT PRIMARY KEY, \
             parent_id INT REFERENCES p(id) ON UPDATE CASCADE)",
        )
        .unwrap();
        db.execute("INSERT INTO p VALUES (1)").unwrap();
        db.execute("INSERT INTO c VALUES (10, 1)").unwrap();

        // The catalog carries the on_update action before reopen.
        let table = db.schema().get_table("c").expect("child table");
        assert_eq!(table.foreign_keys[0].on_update, ReferentialAction::Cascade);
    }

    // Reopen over the same dir: the catalog (incl. on_update) is restored from the
    // persisted SYS_CATALOG bytes.
    let mut db = Database::open(&dir.path).expect("reopen");
    let table = db
        .schema()
        .get_table("c")
        .expect("child table after reopen");
    assert_eq!(
        table.foreign_keys[0].on_update,
        ReferentialAction::Cascade,
        "on_update survives the store round-trip"
    );

    // And it still fires: a post-reopen parent-key UPDATE cascades to the child.
    db.execute("UPDATE p SET id = 2 WHERE id = 1").unwrap();
    assert_eq!(
        scalar(&mut db, "SELECT parent_id FROM c WHERE id = 10"),
        Value::Int4(2)
    );
}
