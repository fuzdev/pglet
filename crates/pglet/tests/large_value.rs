//! A row of any size a client can send is stored, and reads back byte-identical, on
//! every host.
//!
//! A leaf value too long to keep inline goes to a chain of overflow pages
//! (`crates/pglet_storage/src/btree/CLAUDE.md` § Page Layout), so a row is no longer
//! bounded by the ~4 KB page. What is asserted here is the SQL-visible half: rows of
//! 5 000 characters and of a mebibyte, text and `bytea`, inserted, updated from one
//! long value to another and to a short one, deleted, rolled back, read from a
//! snapshot across their replacement, forked, and reopened — and a churn of long rows
//! that leaves a file-backed store no larger than one cycle of it needs, since a
//! replaced chain goes back to the free list. The chain's own mechanics (release,
//! snapshots, fork, corruption) are `cow/tests.rs`'s.
#![allow(clippy::panic, clippy::expect_used)]

use crate::common;

use common::{ScratchDir, open_database};
use pglet::Database;
use pglet_sql::types::Value;
use pglet_testing::block_io::MemBlockIo;
use pglet_testing::rng::Xorshift;

/// `len` pseudo-random letters, so no length is a lucky multiple of anything.
fn letters(seed: u64, len: usize) -> String {
    let mut rng = Xorshift::new(seed);
    (0..len)
        .map(|_| char::from(b'a' + rng.below(26) as u8))
        .collect()
}

/// `len` pseudo-random bytes, NULs included, as a `bytea` hex literal and its bytes.
fn bytes(seed: u64, len: usize) -> (String, Vec<u8>) {
    let mut rng = Xorshift::new(seed);
    let raw: Vec<u8> = (0..len).map(|_| rng.below(256) as u8).collect();
    let hex: String = raw.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
        out
    });
    (format!("'\\x{hex}'"), raw)
}

fn one(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    db.execute(sql)
        .unwrap_or_else(|e| panic!("`{}`: {e}", &sql[..sql.len().min(60)]))
        .remove(0)
        .rows
}

/// Insert, read back, update long to long and long to short, delete, reopen — the
/// same script on every store, each value compared whole.
fn exercise(mut db: Database, reopen: &dyn Fn(Database) -> Database) {
    let five_k = letters(1, 5_000);
    let mib = letters(2, 1 << 20);
    let (mib_bytea_sql, mib_bytea) = bytes(3, 1 << 20);
    db.execute("CREATE TABLE big (id INT, body TEXT, blob BYTEA)")
        .expect("create");
    db.execute(&format!("INSERT INTO big VALUES (1, '{five_k}', NULL)"))
        .expect("5 000 characters");
    db.execute(&format!(
        "INSERT INTO big VALUES (2, '{mib}', {mib_bytea_sql})"
    ))
    .expect("a mebibyte of each");
    db.execute("INSERT INTO big VALUES (3, 'short', '\\x00')")
        .expect("a short row beside them");

    let mut db = reopen(db);
    assert_eq!(
        one(&mut db, "SELECT body FROM big WHERE id = 1"),
        vec![vec![Value::Text(five_k.clone())]]
    );
    assert_eq!(
        one(&mut db, "SELECT body, blob FROM big WHERE id = 2"),
        vec![vec![Value::Text(mib), Value::Bytea(mib_bytea.clone())]]
    );

    // Long to long, long to short, short to long.
    let other = letters(4, 300_000);
    db.execute(&format!("UPDATE big SET body = '{other}' WHERE id = 2"))
        .expect("long to long");
    db.execute("UPDATE big SET body = 'tiny' WHERE id = 1")
        .expect("long to short");
    db.execute(&format!("UPDATE big SET body = '{five_k}' WHERE id = 3"))
        .expect("short to long");
    db.execute("DELETE FROM big WHERE blob = '\\x00' AND id = 999")
        .expect("a delete that matches nothing");

    let mut db = reopen(db);
    assert_eq!(
        one(&mut db, "SELECT id, length(body) FROM big ORDER BY id"),
        vec![
            vec![Value::Int4(1), Value::Int4(4)],
            vec![Value::Int4(2), Value::Int4(300_000)],
            vec![Value::Int4(3), Value::Int4(5_000)],
        ]
    );
    assert_eq!(
        one(&mut db, "SELECT body, blob FROM big WHERE id = 2"),
        vec![vec![Value::Text(other), Value::Bytea(mib_bytea)]]
    );
    db.execute("DELETE FROM big WHERE id = 2").expect("delete");
    let mut db = reopen(db);
    assert_eq!(
        one(&mut db, "SELECT count(*) FROM big"),
        vec![vec![Value::Int8(2)]]
    );
}

#[test]
fn a_long_row_round_trips_on_the_file_store() {
    let dir = ScratchDir::new("large_file");
    let path = dir.path.clone();
    let db = Database::open(&path).expect("open");
    exercise(db, &|db| {
        drop(db);
        Database::open(&path).expect("reopen")
    });
}

#[test]
fn a_long_row_round_trips_on_the_double_meta_store_over_an_image() {
    let io = MemBlockIo::new();
    let db = open_database(Box::new(io.clone()));
    exercise(db, &|db| {
        drop(db);
        open_database(Box::new(io.reopen()))
    });
}

#[test]
fn a_long_row_round_trips_on_the_in_memory_btree() {
    exercise(Database::with_btree(), &|db| db);
}

/// A transaction's long row is gone on rollback and there on commit, and a block
/// that began before another connection replaced a long value keeps reading the old
/// one until it ends.
#[test]
fn a_long_row_under_transactions_and_snapshots() {
    let dir = ScratchDir::new("large_txn");
    let mut db = Database::open(&dir.path).expect("open");
    let old = letters(5, 20_000);
    let new = letters(6, 30_000);
    db.execute("CREATE TABLE t (id INT, body TEXT)").unwrap();

    db.begin(1).unwrap();
    db.execute_as(&format!("INSERT INTO t VALUES (1, '{old}')"), 1)
        .unwrap();
    db.rollback(1).unwrap();
    assert_eq!(
        one(&mut db, "SELECT count(*) FROM t"),
        vec![vec![Value::Int8(0)]]
    );
    db.begin(1).unwrap();
    db.execute_as(&format!("INSERT INTO t VALUES (1, '{old}')"), 1)
        .unwrap();
    db.commit(1).unwrap();

    // Connection 2's block reads before connection 1 replaces the value.
    db.begin(2).unwrap();
    let read = |db: &mut Database, conn| {
        db.execute_as("SELECT body FROM t WHERE id = 1", conn)
            .unwrap()
            .remove(0)
            .rows
    };
    assert_eq!(read(&mut db, 2), vec![vec![Value::Text(old.clone())]]);
    db.execute_as(&format!("UPDATE t SET body = '{new}' WHERE id = 1"), 1)
        .unwrap();
    // Allocate after the replace, so a page freed too early would be overwritten now.
    db.execute_as(
        &format!("INSERT INTO t VALUES (2, '{}')", letters(9, 40_000)),
        1,
    )
    .unwrap();
    assert_eq!(
        read(&mut db, 2),
        vec![vec![Value::Text(old)]],
        "the open block keeps its snapshot's value"
    );
    db.commit(2).unwrap();
    assert_eq!(read(&mut db, 2), vec![vec![Value::Text(new.clone())]]);
    drop(db);
    let mut db = Database::open(&dir.path).expect("reopen");
    assert_eq!(read(&mut db, 1), vec![vec![Value::Text(new)]]);
}

/// A fork shares a long row with its source, and a write on either side leaves the
/// other's row whole.
#[test]
fn a_long_row_forks() {
    let mut base = Database::with_btree();
    let value = letters(7, 50_000);
    base.execute("CREATE TABLE t (id INT, body TEXT)").unwrap();
    base.execute(&format!("INSERT INTO t VALUES (1, '{value}')"))
        .unwrap();
    let mut fork = base.fork().expect("fork");
    fork.execute("UPDATE t SET body = 'forked' WHERE id = 1")
        .unwrap();
    base.execute(&format!(
        "UPDATE t SET body = '{}' WHERE id = 1",
        letters(8, 9_000)
    ))
    .unwrap();
    assert_eq!(
        one(&mut fork, "SELECT body FROM t"),
        vec![vec![Value::Text("forked".into())]]
    );
    let mut again = Database::with_btree();
    again.execute("CREATE TABLE t (id INT, body TEXT)").unwrap();
    again
        .execute(&format!("INSERT INTO t VALUES (1, '{value}')"))
        .unwrap();
    let mut early = again.fork().expect("fork");
    again.execute("DELETE FROM t").unwrap();
    assert_eq!(
        one(&mut early, "SELECT body FROM t"),
        vec![vec![Value::Text(value)]]
    );
}

/// Replacing and deleting long rows over and over leaves a file-backed store no
/// larger than a few cycles need: a superseded chain's pages are reused.
#[test]
fn a_churn_of_long_rows_reuses_its_pages() {
    let dir = ScratchDir::new("large_churn");
    let mut db = Database::open(&dir.path).expect("open");
    let file = dir.path.join("data.pglet");
    db.execute("CREATE TABLE t (id INT, body TEXT)").unwrap();
    let cycle = |db: &mut Database, n: u64| {
        db.execute(&format!(
            "INSERT INTO t VALUES (1, '{}')",
            letters(n, 200_000)
        ))
        .unwrap();
        db.execute(&format!(
            "UPDATE t SET body = '{}' WHERE id = 1",
            letters(n + 1, 200_000)
        ))
        .unwrap();
        db.execute("DELETE FROM t").unwrap();
    };
    cycle(&mut db, 10);
    cycle(&mut db, 20);
    let after_two = std::fs::metadata(&file).unwrap().len();
    for n in 3..20 {
        cycle(&mut db, n * 10);
    }
    let after_twenty = std::fs::metadata(&file).unwrap().len();
    assert!(
        after_twenty <= after_two + 64 * 1024,
        "the store grew from {after_two} to {after_twenty} bytes over 18 more cycles"
    );
}
