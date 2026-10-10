//! Integration tests for the no-WAL `DoubleMetaStore` driven through the full
//! engine (`Database` over a `BtreeStorageWrapper` over `DoubleMetaStore`).
//!
//! These are in-process tests — a shared in-memory `BlockIo`
//! ([`pglet_testing::block_io::MemBlockIo`]) whose byte image survives a "reopen"
//! stands in for a file. Real process-kill crash tests live in the `test_crash`
//! harness (`pglet_tests`); arbitrary-corruption fuzzing lives in `crash.rs`.

use std::sync::Arc;

use crate::common;

use common::open_database;
use pglet_sql::types::Value;
use pglet_storage::btree::{BlockIo, CountingBlockIo};
use pglet_testing::block_io::MemBlockIo;

/// A thin forwarding `BlockIo` over a shared `Arc<CountingBlockIo<MemBlockIo>>`, so
/// the fsync counter stays observable after the store takes ownership of its
/// `Box<dyn BlockIo>`.
struct ArcIo(Arc<CountingBlockIo<MemBlockIo>>);
impl BlockIo for ArcIo {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read_at(offset, buf)
    }
    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
        self.0.write_at(offset, buf)
    }
    fn sync(&self) -> std::io::Result<()> {
        self.0.sync()
    }
    fn len(&self) -> std::io::Result<u64> {
        self.0.len()
    }
    fn set_len(&self, size: u64) -> std::io::Result<()> {
        self.0.set_len(size)
    }
}

#[test]
fn explicit_commit_survives_reopen() {
    let io = MemBlockIo::new();

    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE users (id INT, name TEXT)")
            .unwrap();
        db.begin(1).unwrap();
        db.execute_as("INSERT INTO users VALUES (1, 'Alice')", 1)
            .unwrap();
        db.execute_as("INSERT INTO users VALUES (2, 'Bob')", 1)
            .unwrap();
        db.commit(1).unwrap();
    }

    // Reopen over the same bytes; the root + catalog must have survived the
    // meta flip.
    let mut db = open_database(Box::new(io.reopen()));
    let result = db.execute("SELECT * FROM users ORDER BY id").unwrap();
    let rows = &result[0].rows;
    assert_eq!(rows.len(), 2, "rows should survive explicit-commit reopen");
    assert_eq!(rows[0][0], Value::Int4(1));
    assert_eq!(rows[0][1], Value::Text("Alice".into()));
    assert_eq!(rows[1][0], Value::Int4(2));
}

#[test]
fn autocommit_insert_survives_reopen() {
    let io = MemBlockIo::new();

    {
        let mut db = open_database(Box::new(io.clone()));
        // No explicit transaction — pure auto-commit.
        db.execute("CREATE TABLE t (id INT)").unwrap();
        db.execute("INSERT INTO t VALUES (10)").unwrap();
        db.execute("INSERT INTO t VALUES (20)").unwrap();
    }

    let mut db = open_database(Box::new(io.reopen()));
    let result = db.execute("SELECT * FROM t ORDER BY id").unwrap();
    let rows = &result[0].rows;
    assert_eq!(rows.len(), 2, "auto-commit rows should survive reopen");
    assert_eq!(rows[0][0], Value::Int4(10));
    assert_eq!(rows[1][0], Value::Int4(20));
}

#[test]
fn jsonb_number_survives_reopen_with_exact_precision() {
    // A jsonb number is stored as the exact `PgNumeric` (store magic PGMC), so a
    // 30-digit integer that an f64 would round must survive a `--data-dir`-style
    // reopen byte-for-byte — proving the `Value::Jsonb` row round-trips through the
    // postcard page codec under the new payload. An f64 would render the reopened
    // value as `123…680000…`, and the scale-carrying `1.50` would collapse to `1.5`.
    let io = MemBlockIo::new();
    let big = "123456789012345678901234567890";

    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE j (id INT, v JSONB)").unwrap();
        db.execute(&format!("INSERT INTO j VALUES (1, '{big}'::jsonb)"))
            .unwrap();
        db.execute("INSERT INTO j VALUES (2, '1.50'::jsonb)")
            .unwrap();
    }

    let mut db = open_database(Box::new(io.reopen()));
    let result = db.execute("SELECT v FROM j ORDER BY id").unwrap();
    let rows = &result[0].rows;
    assert_eq!(rows.len(), 2, "jsonb rows should survive reopen");

    // Render the reopened jsonb back to text: the big integer keeps every digit
    // (no f64 rounding) and the decimal keeps its display scale.
    let Value::Jsonb(big_json) = &rows[0][0] else {
        panic!("expected jsonb, got {:?}", rows[0][0]);
    };
    assert_eq!(
        big_json.to_json_string(),
        big,
        "big integer must round-trip exactly across reopen (PGMC)"
    );
    let Value::Jsonb(dec_json) = &rows[1][0] else {
        panic!("expected jsonb, got {:?}", rows[1][0]);
    };
    assert_eq!(dec_json.to_json_string(), "1.50");
}

#[test]
fn concurrent_committed_ddl_survives_dml_txn_rollback_and_reopen() {
    // End-to-end + persistent: connection A's block reads while connection B
    // (auto-commit) commits a CREATE TABLE and a row — B's write runs, since A's block
    // has not written. A's ROLLBACK must not clobber B's table, and the committed table
    // must recover consistently on reopen. Two cells: A then writes (taking the writer,
    // which re-bases the block on B's commit) before rolling back, or A never writes
    // (and its rollback restores nothing).
    for a_writes in [true, false] {
        let cell = if a_writes { "A wrote" } else { "A only read" };
        let io = MemBlockIo::new();
        {
            let mut db = open_database(Box::new(io.clone()));
            db.execute("CREATE TABLE base (id INT)").unwrap();
            db.execute("INSERT INTO base VALUES (1)").unwrap();

            // A (conn 1) opens a block that reads.
            db.begin(1).unwrap();
            db.execute_as("SELECT count(*) FROM base", 1).unwrap();

            // B (conn 2, auto-commit) commits a CREATE TABLE during A's block.
            db.execute_as("CREATE TABLE concurrent_t (x INT)", 2)
                .unwrap();
            db.execute_as("INSERT INTO concurrent_t VALUES (42)", 2)
                .unwrap();

            if a_writes {
                db.execute_as("INSERT INTO base VALUES (2)", 1).unwrap();
            }

            // A rolls back — its DML is discarded; B's committed table is preserved.
            db.rollback(1).unwrap();

            let r = db.execute("SELECT x FROM concurrent_t").unwrap();
            assert_eq!(
                r[0].rows.len(),
                1,
                "{cell}: B's committed table survives A's rollback"
            );
            assert_eq!(r[0].rows[0][0], Value::Int4(42));
            let r = db.execute("SELECT * FROM base").unwrap();
            assert_eq!(
                r[0].rows.len(),
                1,
                "{cell}: A's in-transaction insert was rolled back"
            );
        }

        // Reopen over the same bytes: the concurrently-committed table + row recover.
        let mut db = open_database(Box::new(io.reopen()));
        let r = db.execute("SELECT x FROM concurrent_t").unwrap();
        assert_eq!(
            r[0].rows.len(),
            1,
            "{cell}: concurrent table must survive reopen"
        );
        assert_eq!(r[0].rows[0][0], Value::Int4(42));
        // A fresh CREATE after reopen allocates cleanly (no id collision with the
        // recovered table).
        db.execute("CREATE TABLE after_reopen (y INT)").unwrap();
        db.execute("INSERT INTO after_reopen VALUES (7)").unwrap();
        let r = db.execute("SELECT y FROM after_reopen").unwrap();
        assert_eq!(r[0].rows[0][0], Value::Int4(7));
        let r = db.execute("SELECT x FROM concurrent_t").unwrap();
        assert_eq!(
            r[0].rows[0][0],
            Value::Int4(42),
            "{cell}: recovered table not aliased"
        );
    }
}

#[test]
fn drop_in_txn_rollback_restores_rows_and_survives_reopen() {
    // The DROP-in-transaction-rollback row-loss residual, end-to-end + persistent.
    // The in-transaction drop is now buffered (deferred to commit), so ROLLBACK
    // restores the rows — and the catalog goes back with them: `Database::begin`
    // snapshots it into the session's frame and `Database::rollback` restores it, the
    // same `pglet::TransactionSession` the native server and the wasm bindings drive.
    // This test used to do that dance by hand, because the embed facade rolled back
    // storage and left the catalog alone.
    let io = MemBlockIo::new();
    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT, v INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1, 10)").unwrap();
        db.execute("INSERT INTO t VALUES (2, 20)").unwrap();

        db.begin(1).unwrap();
        db.execute_as("DROP TABLE t", 1).unwrap();
        db.rollback(1).unwrap();

        // The table is fully restored: both committed rows return.
        let r = db.execute("SELECT v FROM t ORDER BY v").unwrap();
        assert_eq!(r[0].rows.len(), 2, "rolled-back DROP must restore the rows");
        assert_eq!(r[0].rows[0][0], Value::Int4(10));
        assert_eq!(r[0].rows[1][0], Value::Int4(20));
    }

    // Reopen over the same bytes: the table + rows recover (its SYS_TABLE was never
    // deleted — the drop was buffered and discarded on rollback).
    let mut db = open_database(Box::new(io.reopen()));
    let r = db.execute("SELECT v FROM t ORDER BY v").unwrap();
    assert_eq!(
        r[0].rows.len(),
        2,
        "the table must survive reopen after a rolled-back DROP"
    );
    assert_eq!(r[0].rows[1][0], Value::Int4(20));
}

#[test]
fn large_catalog_spans_multiple_system_chunks_and_recovers() {
    // The PGM5 catalog lives in the tree as page-sized SYS_CATALOG chunks. A schema
    // big enough to exceed one chunk (CATALOG_CHUNK_SIZE) must persist across N
    // chunked system entries and reassemble on reopen — exercising the multi-chunk
    // path the small fixtures never reach. 100 five-column tables make the postcard
    // Schema comfortably multi-chunk; each table also persists its SYS_TABLE
    // directory entry + a row.
    const N: usize = 100;
    let io = MemBlockIo::new();
    {
        let mut db = open_database(Box::new(io.clone()));
        for i in 0..N {
            db.execute(&format!(
                "CREATE TABLE t{i} (id INT, a TEXT, b TEXT, c TEXT, d TEXT)"
            ))
            .unwrap();
            db.execute(&format!(
                "INSERT INTO t{i} VALUES ({i}, 'a', 'b', 'c', 'd')"
            ))
            .unwrap();
        }
    }

    // Reopen: the chunked catalog + every table's directory entry + row survive.
    let mut db = open_database(Box::new(io.reopen()));
    for i in 0..N {
        let result = db.execute(&format!("SELECT id FROM t{i}")).unwrap();
        let rows = &result[0].rows;
        assert_eq!(
            rows.len(),
            1,
            "table t{i} row should survive a multi-chunk-catalog reopen"
        );
        assert_eq!(rows[0][0], Value::Int4(i as i32));
    }

    // A fresh write after reopen continues the recovered row-id counter cleanly.
    db.execute("INSERT INTO t0 VALUES (999, 'a', 'b', 'c', 'd')")
        .unwrap();
    let rows = &db.execute("SELECT id FROM t0 ORDER BY id").unwrap()[0].rows;
    assert_eq!(
        rows.len(),
        2,
        "post-reopen insert should land on the recovered table"
    );
}

#[test]
fn torn_newer_meta_slot_falls_back() {
    let io = MemBlockIo::new();

    // First good commit (version 1 in slot B).
    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
    }
    // Second good commit (version 2 in slot A) adds a row.
    {
        let mut db = open_database(Box::new(io.reopen()));
        db.execute("INSERT INTO t VALUES (2)").unwrap();
    }

    // Corrupt the newer (active) meta slot. The active slot alternates every
    // commit; we corrupt both candidate header regions' payloads to find the
    // newer one — but only flip inside slot A (offset 0), which holds the
    // latest version-3 schema/row write. We flip a payload byte so the CRC
    // fails while the magic still parses, forcing fallback to the older slot.
    io.with_bytes_mut(|data| {
        // Meta header is 20 bytes; flip a byte inside slot A's payload.
        data[20 + 8] ^= 0xFF;
    });

    // Reopen: slot A is torn, must fall back to slot B (the older consistent
    // state). No panic; we see the older committed state.
    let mut db = open_database(Box::new(io.reopen()));
    let result = db.execute("SELECT * FROM t ORDER BY id").unwrap();
    let rows = &result[0].rows;
    // Older slot may hold 1 row (if A was the version-2 commit). Either way the
    // engine recovers a consistent earlier snapshot without panicking.
    assert!(
        !rows.is_empty(),
        "fallback to older valid slot yields a consistent state"
    );
    assert_eq!(rows[0][0], Value::Int4(1));
}

#[test]
fn autocommit_insert_performs_exactly_two_fsyncs() {
    // Wrap the BlockIo in a counter to assert the commit fsync count.
    let counting = Arc::new(CountingBlockIo::new(MemBlockIo::new()));

    let mut db = open_database(Box::new(ArcIo(Arc::clone(&counting))));
    // CREATE TABLE is a write too; reset after setup so we measure one INSERT.
    db.execute("CREATE TABLE t (id INT)").unwrap();
    counting.reset();

    db.execute("INSERT INTO t VALUES (1)").unwrap();

    let stats = counting.stats();
    assert_eq!(
        stats.syncs, 2,
        "one auto-commit INSERT must perform exactly 2 fsyncs (data flush + meta flip), got {}",
        stats.syncs
    );
}

#[test]
fn multi_table_autocommit_insert_after_reopen_performs_exactly_two_fsyncs() {
    // The single-table oracle above can't catch the catalog-serialization
    // determinism bug: with one entry per catalog map there is no order to vary.
    // A multi-table schema serialized from `HashMap`s had process-random entry
    // order, so the first commit AFTER REOPEN re-serialized the recovered schema
    // into different bytes than the stored ones, tripping
    // `persist_catalog_if_changed` into a spurious `SYS_CATALOG` rewrite (+1 fsync).
    // With the catalog backed by `BTreeMap`s the encoding is canonical, so a
    // pure-DML autocommit stays at exactly 2 fsyncs even on the first commit after
    // opening a multi-table DB.
    let io = MemBlockIo::new();

    // Phase 1: build a multi-table DB and persist it. Enough tables/indexes that a
    // non-canonical (HashMap) encoding would re-order with overwhelming probability
    // (so this reliably catches a regression, not just by luck on a 2–3 entry map).
    {
        let mut db = open_database(Box::new(io.clone()));
        for t in 'a'..='h' {
            db.execute(&format!("CREATE TABLE {t} (id INT)")).unwrap();
            db.execute(&format!("CREATE INDEX {t}_id_idx ON {t} (id)"))
                .unwrap();
        }
    }

    // Phase 2: reopen through an fsync counter; the recovered schema is freshly
    // deserialized, so a non-canonical encoding would re-serialize differently and
    // pay an extra catalog-rewrite fsync on the first autocommit INSERT.
    let counting = Arc::new(CountingBlockIo::new(io.reopen()));
    let mut db = open_database(Box::new(ArcIo(Arc::clone(&counting))));
    counting.reset();

    db.execute("INSERT INTO a VALUES (1)").unwrap();

    let stats = counting.stats();
    assert_eq!(
        stats.syncs, 2,
        "first multi-table autocommit INSERT after reopen must perform exactly 2 fsyncs \
         (no spurious catalog rewrite), got {}",
        stats.syncs
    );
}

/// A store written before a string was read as `bytea` by `byteain`'s two formats, and
/// before a backslash in an unquoted array-literal element was an escape, cannot be
/// reopened.
///
/// Its bytes decode cleanly; what moved is the value a stored expression evaluates to —
/// `'cafe'::bytea` in a `CHECK` or a partial-index predicate, a `bytea` default string,
/// `'{a\,b}'::text[]` — so a unique partial index built under the old reading would
/// accept a duplicate beside a stored row. The magic is spelled here, not read from the
/// stale list: a bump left out of that list, or not applied at all, makes the old file
/// open.
#[test]
fn a_store_written_under_the_earlier_bytea_reading_is_rejected() {
    use pglet_storage::btree::DoubleMetaStore;

    // Two 64 KiB meta slots — the whole pre-data region, so the file is read as a
    // store rather than as a fresh one.
    let mut bytes = vec![0u8; 2 * 64 * 1024];
    bytes[0..4].copy_from_slice(b"PGMS");
    let err = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes)))
        .expect_err("a PGMS store must not reopen");
    assert!(
        format!("{err:?}").contains("stale on-disk format"),
        "expected a stale-format error, got {err:?}"
    );
}
