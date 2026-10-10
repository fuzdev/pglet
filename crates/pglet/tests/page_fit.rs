//! Every page the B+tree builds fits one page, on every host, whatever mix of
//! entries lands in it.
//!
//! A node that no longer fits is split into pieces that each do (`page::split_points`);
//! before that, a node was split at its entry-count midpoint and neither half was
//! checked again, so a half could hold more than a page. The file store then wrote
//! past its 4 096-byte page buffer (a panic, and a `SIGABRT` in a release server),
//! while the in-memory store held the page and answered. Each shape below is one that
//! did that: two rows whose sum outgrows a page, a row beside its own index entry,
//! two catalog chunks in one leaf, and an ordinary indexed insert/update workload of
//! values no longer than 1 000 characters.
//!
//! Each runs on three hosts: the embed's file store (`Database::open`), the same
//! double-meta store over an in-memory `BlockIo` (so a reopen reads the bytes back
//! through the page codec), and the in-memory B+tree (`Database::with_btree`), whose
//! page store refuses a page that would not fit rather than holding it — the reason
//! the fixture harness, which runs that store, guards this class at all.
#![allow(clippy::panic, clippy::expect_used)]

use crate::common;

use common::{ScratchDir, open_database};
use pglet::Database;
use pglet_sql::types::Value;
use pglet_testing::block_io::MemBlockIo;
use pglet_testing::rng::Xorshift;

/// A store a shape runs against, and how it is reopened.
enum Host {
    /// `Database::open` over a scratch directory.
    File(ScratchDir),
    /// The double-meta store over a shared in-memory `BlockIo`.
    MemImage(MemBlockIo),
    /// The in-memory B+tree, which has no reopen.
    Btree,
}

impl Host {
    fn all(tag: &str) -> Vec<Self> {
        vec![
            Self::File(ScratchDir::new(tag)),
            Self::MemImage(MemBlockIo::new()),
            Self::Btree,
        ]
    }

    const fn name(&self) -> &'static str {
        match self {
            Self::File(_) => "file",
            Self::MemImage(_) => "mem-image",
            Self::Btree => "btree",
        }
    }

    fn open(&self) -> Database {
        match self {
            Self::File(dir) => Database::open(&dir.path).expect("open file-backed database"),
            Self::MemImage(io) => Self::attach(open_database(Box::new(io.clone()))),
            Self::Btree => Database::with_btree(),
        }
    }

    /// Re-register the persisted indexes, as `Database::open` does.
    fn attach(mut db: Database) -> Database {
        db.reattach_persisted_indexes()
            .expect("re-register persisted indexes");
        db
    }

    /// Close `db` and open the same store again; the in-memory B+tree keeps `db`.
    fn reopen(&self, db: Database) -> Database {
        match self {
            Self::File(dir) => {
                drop(db);
                Database::open(&dir.path).expect("reopen file-backed database")
            }
            Self::MemImage(io) => {
                drop(db);
                Self::attach(open_database(Box::new(io.reopen())))
            }
            Self::Btree => db,
        }
    }
}

fn exec(db: &mut Database, host: &Host, sql: &str) {
    if let Err(e) = db.execute(sql) {
        panic!(
            "[{}] `{}` failed: {e}",
            host.name(),
            &sql[..sql.len().min(80)]
        );
    }
}

fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    db.execute(sql).expect("query").remove(0).rows
}

/// `n` copies of `c`.
fn run(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

/// A deterministic string of `n` letters from `a`..`j`, which no two calls share.
fn letters(rng: &mut Xorshift, n: usize) -> String {
    (0..n)
        .map(|_| char::from(b'a' + rng.below(10) as u8))
        .collect()
}

#[test]
fn two_rows_that_outgrow_a_page_together_fit_and_reopen() {
    for host in Host::all("rows") {
        let mut db = host.open();
        exec(&mut db, &host, "CREATE TABLE r (a TEXT)");
        exec(
            &mut db,
            &host,
            &format!("INSERT INTO r VALUES ('{}')", run('x', 3000)),
        );
        exec(
            &mut db,
            &host,
            &format!("INSERT INTO r VALUES ('{}')", run('y', 1100)),
        );
        let mut db = host.reopen(db);
        assert_eq!(
            rows(&mut db, "SELECT length(a) FROM r ORDER BY 1"),
            vec![vec![Value::Int4(1100)], vec![Value::Int4(3000)]],
            "[{}]",
            host.name()
        );
    }
}

#[test]
fn a_row_beside_its_own_index_entry_fits_and_reopens() {
    for host in Host::all("idx") {
        let mut db = host.open();
        exec(&mut db, &host, "CREATE TABLE r (a TEXT)");
        exec(&mut db, &host, "CREATE INDEX ri ON r (a)");
        let a = run('x', 2008);
        let b = run('y', 2040);
        exec(&mut db, &host, &format!("INSERT INTO r VALUES ('{a}')"));
        exec(&mut db, &host, &format!("INSERT INTO r VALUES ('{b}')"));
        let mut db = host.reopen(db);
        for (value, len) in [(&a, 2008), (&b, 2040)] {
            assert_eq!(
                rows(
                    &mut db,
                    &format!("SELECT length(a) FROM r WHERE a = '{value}'")
                ),
                vec![vec![Value::Int4(len)]],
                "[{}] the index probe must find the {len}-character row",
                host.name()
            );
        }
    }
}

#[test]
fn two_catalog_chunks_in_one_leaf_fit_and_reopen() {
    for host in Host::all("cat") {
        let mut db = host.open();
        exec(&mut db, &host, "CREATE TABLE f1 (a TEXT)");
        let long = run('x', 6000);
        exec(
            &mut db,
            &host,
            &format!("CREATE TABLE f2 (a TEXT CHECK (a <> '{long}'))"),
        );
        let mut db = host.reopen(db);
        // The constraint came back whole: it still refuses the one value it names.
        let err = db
            .execute(&format!("INSERT INTO f2 VALUES ('{long}')"))
            .expect_err("the reopened CHECK must refuse its own literal");
        assert_eq!(err.sqlstate(), "23514", "[{}] {err}", host.name());
        exec(&mut db, &host, "INSERT INTO f2 VALUES ('ok')");
    }
}

/// The ordinary workload that aborted a release `--data-dir` server: an indexed
/// column written by `INSERT`s of 1 to 1 000 random letters, each followed a quarter
/// of the time by an `UPDATE` of an earlier row to 1 to 1 000 `z`s. Checked against a
/// model of the table after a reopen, over twelve seeds per host.
#[test]
fn an_indexed_insert_update_workload_of_short_values_fits_and_reopens() {
    for seed in 1..=12u64 {
        for host in Host::all("work") {
            let mut rng = Xorshift::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut db = host.open();
            exec(&mut db, &host, "CREATE TABLE r (id INT, a TEXT)");
            exec(&mut db, &host, "CREATE INDEX ri ON r (a)");
            let mut model: Vec<String> = Vec::new();
            for _ in 0..150 {
                let len = 1 + rng.below(1000) as usize;
                let a = letters(&mut rng, len);
                exec(
                    &mut db,
                    &host,
                    &format!("INSERT INTO r VALUES ({}, '{a}')", model.len() + 1),
                );
                model.push(a);
                if model.len() > 1 && rng.below(4) == 0 {
                    let k = 1 + rng.below(model.len() as u64 - 1) as usize;
                    let z = run('z', 1 + rng.below(1000) as usize);
                    exec(
                        &mut db,
                        &host,
                        &format!("UPDATE r SET a = '{z}' WHERE id = {k}"),
                    );
                    model[k - 1] = z;
                }
            }
            let mut db = host.reopen(db);
            let got = rows(&mut db, "SELECT id, a FROM r ORDER BY id");
            let want: Vec<Vec<Value>> = model
                .iter()
                .enumerate()
                .map(|(i, a)| vec![Value::Int4(i as i32 + 1), Value::Text(a.clone())])
                .collect();
            assert!(
                got == want,
                "[{} seed {seed}] the table does not match its model",
                host.name()
            );
            // And the index agrees with the rows: every value is found through it.
            for (i, a) in model.iter().enumerate().step_by(7) {
                let hit = rows(&mut db, &format!("SELECT id FROM r WHERE a = '{a}'"));
                assert!(
                    hit.contains(&vec![Value::Int4(i as i32 + 1)]),
                    "[{} seed {seed}] row {} is missing from the index",
                    host.name(),
                    i + 1
                );
            }
        }
    }
}
