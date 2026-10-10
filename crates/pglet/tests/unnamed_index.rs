//! An unnamed `CREATE INDEX ON t (…)`: what its chosen name costs a statement that
//! fails, a block that rolls back, and a store that is reopened.
//!
//! The differential fixtures (`tests/fixtures/ddl/create_index_unnamed_*.sql`) hold the
//! name itself to 18.6's. These hold what a fixture cannot reach: a statement that
//! errors mid-script, a rollback, and a reopen. The first two tests' answers are
//! 18.6's, measured statement by statement; the reopen is this engine's own.

#![allow(clippy::unwrap_used)]

use crate::common;

use common::ScratchDir;
use pglet::Database;

/// The table's index names, sorted.
fn index_names(db: &Database, table: &str) -> Vec<String> {
    let mut names: Vec<String> = db
        .schema()
        .indexes
        .values()
        .filter(|index| index.table_name == table)
        .map(|index| index.name.clone())
        .collect();
    names.sort();
    names
}

/// `sql`'s SQLSTATE, or `ok`.
fn code(db: &mut Database, sql: &str) -> String {
    db.execute(sql)
        .map_or_else(|e| e.sqlstate().to_string(), |_| "ok".to_string())
}

/// A build that fails takes no name with it: the unique build over duplicate rows is
/// `23505` and an expression key that raises on a row `22012`, and the next unnamed
/// statement over the same key takes the undigited name, on either backend.
#[test]
fn a_failed_unnamed_build_leaves_its_name_free() {
    for mut db in [Database::with_btree(), Database::in_memory()] {
        db.execute("CREATE TABLE t (a INT, b INT); INSERT INTO t VALUES (1, 0), (1, 5)")
            .unwrap();
        assert_eq!(code(&mut db, "CREATE UNIQUE INDEX ON t (a)"), "23505");
        assert_eq!(code(&mut db, "CREATE INDEX ON t ((10 / b))"), "22012");
        assert_eq!(index_names(&db, "t"), Vec::<String>::new());

        assert_eq!(code(&mut db, "CREATE INDEX ON t (a)"), "ok");
        assert_eq!(code(&mut db, "CREATE INDEX ON t ((b + 1))"), "ok");
        assert_eq!(index_names(&db, "t"), ["t_a_idx", "t_expr_idx"]);
    }
}

/// A rolled-back block gives its minted names back, and a `ROLLBACK TO` gives back the
/// ones minted after the savepoint: the name is chosen against the catalog as it stands,
/// so the statement after each rollback chooses what the rolled-back one chose.
#[test]
fn a_rollback_gives_a_minted_name_back() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (a INT)").unwrap();

    db.execute("BEGIN").unwrap();
    db.execute("CREATE INDEX ON t (a)").unwrap();
    db.execute("CREATE INDEX ON t (a)").unwrap();
    assert_eq!(index_names(&db, "t"), ["t_a_idx", "t_a_idx1"]);
    db.execute("ROLLBACK").unwrap();
    assert_eq!(index_names(&db, "t"), Vec::<String>::new());

    db.execute("BEGIN").unwrap();
    db.execute("CREATE INDEX ON t (a)").unwrap();
    db.execute("SAVEPOINT s").unwrap();
    db.execute("CREATE INDEX ON t (a)").unwrap();
    db.execute("ROLLBACK TO s").unwrap();
    db.execute("CREATE UNIQUE INDEX ON t (a)").unwrap();
    db.execute("COMMIT").unwrap();
    assert_eq!(index_names(&db, "t"), ["t_a_idx", "t_a_idx1"]);
    assert!(db.schema().get_index("t_a_idx1").unwrap().is_unique());
}

/// The minted name is a catalog fact: a reopened store holds the names the statements
/// chose, not names chosen again from the keys — the survivor of a dropped pair keeps its
/// digit — and the next unnamed statement steps around what the reopened catalog holds.
#[test]
fn a_minted_name_survives_a_reopen() {
    let dir = ScratchDir::new("unnamed_index");
    {
        let mut db = Database::open(&dir.path).unwrap();
        db.execute("CREATE TABLE t (b INT, a INT)").unwrap();
        db.execute("CREATE INDEX ON t (a)").unwrap();
        db.execute("CREATE INDEX ON t (a)").unwrap();
        db.execute("CREATE INDEX ON t (a)").unwrap();
        db.execute("DROP INDEX t_a_idx").unwrap();
    }
    let mut db = Database::open(&dir.path).unwrap();
    assert_eq!(index_names(&db, "t"), ["t_a_idx1", "t_a_idx2"]);
    db.execute("CREATE INDEX ON t (a)").unwrap();
    db.execute("CREATE INDEX ON t (a)").unwrap();
    assert_eq!(
        index_names(&db, "t"),
        ["t_a_idx", "t_a_idx1", "t_a_idx2", "t_a_idx3"]
    );
}
