//! `ALTER TABLE … RENAME TO` on a file-backed `Database`: the store's directory follows
//! the catalog across a reopen, and a block's rename taken back — whole or to a
//! savepoint — leaves the directory, the tree's entry and the catalog as they were, so
//! a reopen finds the table under its old name with its rows and indexes.

#![expect(
    clippy::unwrap_used,
    reason = "test setup `.unwrap()`s each statement so a failure surfaces where it happens"
)]

use crate::common;

use common::ScratchDir;
use pglet::Database;

/// `sql`'s last result's rows, each column's text joined by `|`.
fn rows(db: &mut Database, sql: &str) -> Vec<String> {
    let results = db.execute(sql).unwrap();
    results
        .last()
        .unwrap()
        .rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect()
}

#[test]
fn a_renamed_table_keeps_its_rows_indexes_and_references_across_a_reopen() {
    let dir = ScratchDir::new("rename_reopen");
    {
        let mut db = Database::open(&dir.path).unwrap();
        db.execute("CREATE TABLE ra (id SERIAL PRIMARY KEY, v TEXT CHECK (ra.v <> 'x'))")
            .unwrap();
        db.execute("CREATE INDEX ra_v ON ra (v) WHERE ra.v > 'a'")
            .unwrap();
        db.execute("CREATE TABLE rb (a INT REFERENCES ra (id))")
            .unwrap();
        db.execute("INSERT INTO ra (v) VALUES ('b'), ('c')")
            .unwrap();
        db.execute("INSERT INTO rb VALUES (1)").unwrap();
        db.execute("ALTER TABLE ra RENAME TO rz").unwrap();
        // A block's rename taken back whole, and one taken back to a savepoint.
        db.execute("BEGIN").unwrap();
        db.execute("ALTER TABLE rb RENAME TO ry").unwrap();
        db.execute("INSERT INTO ry VALUES (2)").unwrap();
        db.execute("ROLLBACK").unwrap();
        db.execute("BEGIN").unwrap();
        db.execute("ALTER TABLE rz RENAME TO ra2").unwrap();
        db.execute("SAVEPOINT s").unwrap();
        db.execute("ALTER TABLE ra2 RENAME TO ra3").unwrap();
        db.execute("ROLLBACK TO SAVEPOINT s").unwrap();
        db.execute("INSERT INTO ra2 (v) VALUES ('d')").unwrap();
        db.execute("COMMIT").unwrap();
    }
    let mut db = Database::open(&dir.path).unwrap();
    assert_eq!(
        rows(&mut db, "SELECT id, v FROM ra2 ORDER BY id"),
        [
            "Int4(1)|Text(\"b\")",
            "Int4(2)|Text(\"c\")",
            "Int4(3)|Text(\"d\")"
        ]
    );
    assert_eq!(rows(&mut db, "SELECT a FROM rb"), ["Int4(1)"]);
    for gone in ["ra", "rz", "ra3", "ry"] {
        let err = db.execute(&format!("SELECT * FROM {gone}")).unwrap_err();
        assert_eq!(err.sqlstate(), "42P01", "{gone}");
    }
    assert_eq!(
        db.execute("INSERT INTO rb VALUES (9)")
            .unwrap_err()
            .sqlstate(),
        "23503",
        "the foreign key references the renamed table"
    );
    assert_eq!(
        db.execute("INSERT INTO ra2 (v) VALUES ('x')")
            .unwrap_err()
            .sqlstate(),
        "23514"
    );
    assert_eq!(
        db.execute("INSERT INTO ra2 (id, v) VALUES (1, 'q')")
            .unwrap_err()
            .sqlstate(),
        "23505",
        "the primary key's index follows the table"
    );
    assert_eq!(
        rows(&mut db, "SELECT v FROM ra2 WHERE v > 'b' ORDER BY v"),
        ["Text(\"c\")", "Text(\"d\")"]
    );
    db.execute("INSERT INTO ra2 (v) VALUES ('e')").unwrap();
    assert_eq!(
        rows(&mut db, "SELECT pg_get_serial_sequence('ra2', 'id')"),
        ["Text(\"public.ra_id_seq\")"]
    );
}

/// A block's rename taken back — whole, or to a savepoint — leaves the storage index defs
/// reading the table under its old name: an in-block range or GIN read scans that table,
/// not one created later under the name the rename gave it. The defs' table name is what
/// those reads scan by, so the rollback puts it back with the directory.
#[test]
fn a_rolled_back_rename_leaves_in_block_index_reads_on_the_table() {
    for (savepoint, setup, insert, query, want) in [
        (
            false,
            "CREATE TABLE t (a INT); CREATE INDEX t_a ON t (a)",
            "INSERT INTO t VALUES (1), (7), (9)",
            "SELECT a FROM t WHERE a > 5 ORDER BY a",
            vec!["Int4(7)", "Int4(9)"],
        ),
        (
            true,
            "CREATE TABLE t (a INT); CREATE INDEX t_a ON t (a)",
            "INSERT INTO t VALUES (1), (7), (9)",
            "SELECT a FROM t WHERE a > 5 ORDER BY a",
            vec!["Int4(7)", "Int4(9)"],
        ),
        (
            true,
            "CREATE TABLE t (a INT, j JSONB); CREATE INDEX t_j ON t USING gin (j)",
            "INSERT INTO t VALUES (1, '{\"a\":1}'), (2, '{\"b\":2}')",
            "SELECT a FROM t WHERE j @> '{\"a\":1}'",
            vec!["Int4(1)"],
        ),
    ] {
        let mut db = Database::with_btree();
        db.execute(setup).unwrap();
        db.execute(insert).unwrap();
        db.execute("BEGIN").unwrap();
        if savepoint {
            db.execute("SAVEPOINT s").unwrap();
            db.execute("ALTER TABLE t RENAME TO u").unwrap();
            db.execute("ROLLBACK TO SAVEPOINT s").unwrap();
            db.execute("COMMIT").unwrap();
        } else {
            db.execute("ALTER TABLE t RENAME TO u").unwrap();
            db.execute("ROLLBACK").unwrap();
        }
        db.execute(&setup.replace(" t ", " u ").replace("t_", "u_"))
            .unwrap();
        db.execute(
            &insert
                .replace(" t ", " u ")
                .replace("(1", "(100")
                .replace("(2", "(200"),
        )
        .unwrap();
        db.execute("BEGIN").unwrap();
        assert_eq!(rows(&mut db, query), want, "{query}, savepoint {savepoint}");
        db.execute("ROLLBACK").unwrap();
    }
}
