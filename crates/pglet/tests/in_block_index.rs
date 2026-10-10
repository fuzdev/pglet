//! An index a transaction block builds or rebuilds, read inside the same block over the
//! rows committed before it, on the in-memory CoW B+tree (`Database::with_btree`).
//!
//! The build's entries are published at once while the block reads at its own snapshot,
//! so the block's unique probes, foreign-key probes and indexed reads read those entries
//! only because the block's snapshot moves onto what it published
//! (`BtreeStorageWrapper::advance_transaction_snapshot`). Memory storage
//! (`Database::in_memory`) has no snapshot to move, so it is not the backend here. The
//! single-connection statements of the first three tests answer as 18.6 answered them over
//! psql; the second connection's `55P03` is this host's refusal (the repo's `EDGES.md`
//! row 229), not 18.6's; and the enum reorders, which 18.6 refuses at the `DROP TYPE`
//! (`2BP01`), are held to the answer the same statements give outside a block.

#![allow(clippy::unwrap_used, clippy::panic)]

use pglet::{Database, StorageBackend};

/// `sql` on `conn`: its last result's rows as text, or `ERR <SQLSTATE>`.
fn answer(db: &mut Database, sql: &str, conn: i32) -> String {
    match db.execute_as(sql, conn) {
        Ok(results) => results.last().map_or_else(String::new, |r| {
            r.rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect::<Vec<_>>()
                .join(",")
        }),
        Err(e) => format!("ERR {}", e.sqlstate()),
    }
}

/// An in-memory B+tree database with `sql` run on connection `0`.
fn seeded(sql: &str) -> Database {
    let mut db = Database::with_btree();
    db.execute(sql).unwrap();
    db
}

/// How many snapshot versions the store holds readers on.
fn held_snapshots(db: &Database) -> usize {
    let StorageBackend::Btree(btree) = db.storage() else {
        panic!("the in-memory B+tree");
    };
    btree.stats().active_snapshots
}

/// A unique index built inside a block holds the row committed before it: the block's
/// duplicate `INSERT`, key-moving `UPDATE` and `ON CONFLICT` arbiter all find it, and the
/// block's `ROLLBACK` takes the index back. Red at the first `23505` with the block's
/// snapshot left where it took the writer, where each of the three writes landed and the
/// third's `COMMIT` published two rows under the index.
#[test]
fn a_unique_index_built_in_a_block_holds_the_rows_committed_before_it() {
    let mut db =
        seeded("CREATE TABLE r (a INT, v TEXT); INSERT INTO r VALUES (1, 'old'), (2, 'b')");
    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    assert_eq!(answer(&mut db, "CREATE UNIQUE INDEX r_u ON r (a)", 1), "");
    assert_eq!(
        answer(&mut db, "INSERT INTO r VALUES (1, 'dup')", 1),
        "ERR 23505"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 1), "");

    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    assert_eq!(answer(&mut db, "CREATE UNIQUE INDEX r_u ON r (a)", 1), "");
    assert_eq!(
        answer(&mut db, "UPDATE r SET a = 1 WHERE a = 2", 1),
        "ERR 23505"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 1), "");

    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    assert_eq!(answer(&mut db, "CREATE UNIQUE INDEX r_u ON r (a)", 1), "");
    assert_eq!(
        answer(
            &mut db,
            "INSERT INTO r VALUES (1, 'new') ON CONFLICT (a) DO UPDATE SET v = excluded.v",
            1
        ),
        ""
    );
    assert_eq!(answer(&mut db, "COMMIT", 1), "");
    assert_eq!(
        answer(&mut db, "SELECT a, v FROM r ORDER BY a", 0),
        "Int4(1)|Text(\"new\"),Int4(2)|Text(\"b\")",
        "the committed row updated, not a second one inserted"
    );
    assert_eq!(
        answer(&mut db, "INSERT INTO r VALUES (2, 'dup')", 0),
        "ERR 23505",
        "the index committed with the block enforces the key"
    );
}

/// A foreign key through a unique index the block built finds the committed parent row,
/// and an indexed read through an index the block built answers the committed row. Red
/// with the snapshot left behind: the referencing `INSERT` is `23503` and the read, on
/// the accelerated build, answers nothing.
#[test]
fn a_foreign_key_and_an_indexed_read_through_an_index_built_in_the_block() {
    let mut db = seeded(
        "CREATE TABLE p (a INT); INSERT INTO p VALUES (1); CREATE TABLE q (a INT); INSERT INTO q VALUES (1)",
    );
    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    assert_eq!(answer(&mut db, "CREATE UNIQUE INDEX p_u ON p (a)", 1), "");
    assert_eq!(
        answer(&mut db, "CREATE TABLE c (a INT REFERENCES p (a))", 1),
        ""
    );
    assert_eq!(answer(&mut db, "INSERT INTO c VALUES (1)", 1), "");
    assert_eq!(answer(&mut db, "CREATE INDEX q_i ON q (a)", 1), "");
    assert_eq!(answer(&mut db, "SELECT a FROM q WHERE a = 1", 1), "Int4(1)");
    assert_eq!(answer(&mut db, "COMMIT", 1), "");
    assert_eq!(answer(&mut db, "SELECT count(*) FROM c", 0), "Int8(1)");
}

/// The block's rollbacks still take back what it built, the moved snapshot
/// notwithstanding: after `ROLLBACK TO` the index is gone and a duplicate lands; after
/// `ROLLBACK` the index is gone and the committed row stands alone; and every snapshot the
/// block held is released when it ends.
#[test]
fn a_rollback_takes_back_the_index_the_block_read_through() {
    let mut db = seeded("CREATE TABLE r (a INT); INSERT INTO r VALUES (1)");
    let held = held_snapshots(&db);

    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    assert_eq!(answer(&mut db, "SAVEPOINT s", 1), "");
    assert_eq!(answer(&mut db, "CREATE UNIQUE INDEX r_u ON r (a)", 1), "");
    assert_eq!(answer(&mut db, "INSERT INTO r VALUES (1)", 1), "ERR 23505");
    assert_eq!(answer(&mut db, "ROLLBACK TO s", 1), "");
    assert_eq!(answer(&mut db, "INSERT INTO r VALUES (1)", 1), "");
    assert_eq!(answer(&mut db, "CREATE INDEX r_i ON r (a)", 1), "");
    assert_eq!(
        answer(&mut db, "SELECT count(*) FROM r WHERE a = 1", 1),
        "Int8(2)",
        "the committed row through the second index, and the block's own"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 1), "");
    assert_eq!(
        held_snapshots(&db),
        held,
        "the block's snapshots are released"
    );

    assert_eq!(answer(&mut db, "INSERT INTO r VALUES (1)", 0), "");
    assert_eq!(answer(&mut db, "SELECT count(*) FROM r", 0), "Int8(2)");
    assert_eq!(
        answer(
            &mut db,
            "SELECT count(*) FROM pg_indexes WHERE tablename = 'r'",
            0
        ),
        "Int8(0)"
    );
}

/// What makes the move sound: while the block holds the writer no other connection
/// publishes, so the root the block moves onto differs from its snapshot only by what it
/// published itself. On this host another logical connection's write is refused `55P03`
/// there — and one that ran before the block's first write is in the state the block
/// re-bases on, so the block's index holds it too.
#[test]
fn another_connections_write_cannot_land_between_the_build_and_the_read() {
    let mut db = seeded("CREATE TABLE r (a INT); INSERT INTO r VALUES (1)");
    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    assert_eq!(answer(&mut db, "SELECT count(*) FROM r", 1), "Int8(1)");
    assert_eq!(
        answer(&mut db, "INSERT INTO r VALUES (5)", 2),
        "",
        "a block that has only read holds no writer"
    );
    assert_eq!(answer(&mut db, "CREATE UNIQUE INDEX r_u ON r (a)", 1), "");
    assert_eq!(
        answer(&mut db, "INSERT INTO r VALUES (7)", 2),
        "ERR 55P03",
        "the block's build took the writer"
    );
    assert_eq!(
        answer(&mut db, "INSERT INTO r VALUES (5)", 1),
        "ERR 23505",
        "the other connection's row, committed before the block's first write"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 1), "");
    assert_eq!(
        answer(&mut db, "SELECT a FROM r ORDER BY a", 0),
        "Int4(1),Int4(5)"
    );
}

/// An enum reorder under a partial unique index rebuilds the index's entries, and the
/// block reads the rebuild as a statement outside a block does: the committed row the
/// reorder brought under the predicate refuses the block's duplicate `23505`. And a
/// `ROLLBACK TO` that takes the reorder back rebuilds the entries again, which the block
/// reads too: a key only the reorder made a duplicate is accepted afterwards, as it is
/// in a block that never reordered.
#[test]
fn a_block_reads_the_entries_an_enum_reorder_rebuilt() {
    let setup = "CREATE TYPE m AS ENUM ('sad', 'ok', 'happy'); \
                 CREATE TABLE e (k INT, m m); INSERT INTO e VALUES (1, 'sad'); \
                 CREATE UNIQUE INDEX e_u ON e (k) WHERE m > 'ok'";
    let reorder = [
        "DROP TYPE m",
        "CREATE TYPE m AS ENUM ('ok', 'happy', 'sad')",
    ];

    let mut outside = seeded(setup);
    for sql in reorder {
        assert_eq!(answer(&mut outside, sql, 0), "");
    }
    assert_eq!(
        answer(&mut outside, "INSERT INTO e VALUES (1, 'sad')", 0),
        "ERR 23505",
        "the control: the same statements with no block open"
    );

    let mut db = seeded(setup);
    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    for sql in reorder {
        assert_eq!(answer(&mut db, sql, 1), "");
    }
    assert_eq!(
        answer(&mut db, "INSERT INTO e VALUES (1, 'sad')", 1),
        "ERR 23505"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 1), "");
    assert_eq!(answer(&mut db, "SELECT count(*) FROM e", 0), "Int8(1)");

    let mut db = seeded(setup);
    assert_eq!(answer(&mut db, "BEGIN", 1), "");
    assert_eq!(answer(&mut db, "SAVEPOINT s", 1), "");
    for sql in reorder {
        assert_eq!(answer(&mut db, sql, 1), "");
    }
    assert_eq!(answer(&mut db, "ROLLBACK TO s", 1), "");
    assert_eq!(
        answer(&mut db, "INSERT INTO e VALUES (1, 'happy')", 1),
        "",
        "the committed 'sad' row is outside the predicate again"
    );
    assert_eq!(answer(&mut db, "COMMIT", 1), "");
    assert_eq!(answer(&mut db, "SELECT count(*) FROM e", 0), "Int8(2)");
}
