//! Durability under power loss and process death, through the whole engine.
//!
//! A double-meta commit is durable once its meta slot is fsynced, and everything a
//! reopen can adopt must still be on the medium as that slot's tree reached it. The
//! power-loss tests record every write the engine issues ([`RecordingBlockIo`]) and ask
//! two questions of the log:
//!
//! - **power loss** ([`power_loss_images`]): at every `sync` boundary, with the
//!   pending writes landing in every tried subset (a torn page included), does open
//!   adopt a state some acknowledged commit left, with a free list holding no page
//!   that state reaches, and do later writes leave it intact?
//! - **the durable window**: between one durable slot and the next, does any write
//!   land on a page the durable slot reaches — its tree pages, its overflow links,
//!   or the free-list and retired-log chain pages it names? A `kill -9` there leaves an
//!   image whose adopted slot reads overwritten pages.
//!
//! One test kills a real embedding process (`Database::open` over a file) with
//! `SIGKILL` inside DDL statements and reopens. The rest drive the engine over an
//! in-memory device and read what the connections and a reopen of the image then hold,
//! many with a write or `fsync` refused on purpose: what a refused write answers and
//! whether the store serves on, what a rollback saves (its restored catalog, its
//! counters) and what a reopen then holds, the table and index numbers a reopen hands
//! out, a retired-log read that fails, a read in flight when the store fails, the page
//! accounting after a reopen, and what a commit writes while a snapshot is held.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fmt::Write as _;

use pglet::{BoundExecError, Database};
use pglet_sql::catalog::Sequence;
use pglet_storage::btree::{BtreeKey, DoubleMetaStore, PAGE_SIZE, PageId};
use pglet_testing::block_io::{DATA_REGION_START, FaultPolicy, MemBlockIo};
use pglet_testing::db::{open_database, try_open_database};
use pglet_testing::power_loss::{
    IoOp, RecordingBlockIo, apply_op, audit_image, power_loss_images, reachable_pages,
};
use pglet_testing::rng::Xorshift;

/// A statement of a workload and whether it ends in an acknowledged commit (a
/// statement inside a block does not).
type Workload = Vec<(String, bool)>;

fn text(id: u64, len: usize) -> String {
    let c = char::from(b'a' + (id % 26) as u8);
    format!("{id}{}", c.to_string().repeat(len))
}

/// A seeded mix of short and long rows, deletes, updates, a block, and `CREATE` /
/// `DROP` — enough churn that the free list spans several chain pages and a DDL
/// statement runs with superseded pages past the vacuum threshold.
fn mixed_workload(seed: u64, steps: usize) -> Workload {
    let mut rng = Xorshift::new(seed);
    let mut w: Workload = vec![("CREATE TABLE t (id INT PRIMARY KEY, v TEXT)".into(), true)];
    let mut next_id = 1u64;
    for step in 0..steps {
        match rng.below(8) {
            0 | 1 => {
                let vals: Vec<String> = (0..=rng.below(12))
                    .map(|_| {
                        let len = match rng.below(3) {
                            0 => 10,
                            1 => 3000,
                            _ => 9000 + rng.below(5000) as usize,
                        };
                        let v = format!("({next_id}, '{}')", text(next_id, len));
                        next_id += 1;
                        v
                    })
                    .collect();
                w.push((format!("INSERT INTO t VALUES {}", vals.join(", ")), true));
            }
            2 => {
                let a = rng.below(next_id);
                w.push((
                    format!("DELETE FROM t WHERE id >= {a} AND id < {}", a + 10),
                    true,
                ));
            }
            3 => {
                let a = rng.below(next_id);
                let n = rng.below(9000) as usize;
                w.push((
                    format!(
                        "UPDATE t SET v = '{}' || v WHERE id >= {a} AND id < {}",
                        "u".repeat(n),
                        a + 5
                    ),
                    true,
                ));
            }
            4 => {
                w.push(("BEGIN".into(), false));
                w.push((
                    format!(
                        "INSERT INTO t VALUES ({next_id}, '{}')",
                        text(next_id, 5000)
                    ),
                    false,
                ));
                next_id += 1;
                w.push(("CREATE TABLE IF NOT EXISTS b (k INT)".into(), false));
                w.push(("UPDATE t SET v = v || 'z' WHERE id % 5 = 1".into(), false));
                w.push(("COMMIT".into(), true));
            }
            5 => {
                w.push(("CREATE TABLE IF NOT EXISTS s (k INT, w TEXT)".into(), true));
                w.push((
                    format!("INSERT INTO s VALUES ({step}, '{}')", "w".repeat(20_000)),
                    true,
                ));
            }
            6 => w.push(("DROP TABLE IF EXISTS s".into(), true)),
            _ => w.push(("DELETE FROM t WHERE id % 3 <> 0".into(), true)),
        }
    }
    w
}

/// A reliable trigger for a DDL statement run past the vacuum threshold:
/// a large `DROP` or rewrite, then a `CREATE TABLE`.
fn ddl_workload() -> Workload {
    let mut w: Workload = vec![("CREATE TABLE t (id INT PRIMARY KEY, v TEXT)".into(), true)];
    let rows: Vec<String> = (1..=40)
        .map(|i| format!("({i}, '{}')", text(i, 20)))
        .collect();
    w.push((format!("INSERT INTO t VALUES {}", rows.join(", ")), true));
    w.push(("CREATE TABLE big0 (id INT, v TEXT)".into(), true));
    for i in 0..12 {
        w.push((
            format!("INSERT INTO big0 VALUES ({i}, '{}')", text(i, 60_000)),
            true,
        ));
    }
    w.push(("DROP TABLE big0".into(), true));
    w.push(("CREATE TABLE big1 (id INT, v TEXT)".into(), true));
    w.push(("CREATE TABLE big0b (id INT, v TEXT)".into(), true));
    let rows: Vec<String> = (1..=400)
        .map(|i| format!("({i}, '{}')", text(i, 40)))
        .collect();
    w.push((
        format!("INSERT INTO big0b VALUES {}", rows.join(", ")),
        true,
    ));
    w.push(("CREATE INDEX big0b_v ON big0b (v)".into(), true));
    w.push(("ALTER TABLE big0b ADD COLUMN w INT DEFAULT 7".into(), true));
    w.push(("DROP TABLE big0b".into(), true));
    w.push(("CREATE TABLE big2 (id INT)".into(), true));
    w.push(("BEGIN".into(), false));
    w.push(("CREATE TABLE blk (id INT, v TEXT)".into(), false));
    w.push((
        format!("INSERT INTO blk VALUES (1, '{}')", text(1, 50_000)),
        false,
    ));
    w.push(("DROP TABLE big1".into(), false));
    w.push(("COMMIT".into(), true));
    w.push(("DROP TABLE blk".into(), true));
    w.push(("CREATE TABLE big3 (id INT)".into(), true));
    w
}

/// Every user table's rows, in a stable order, as one string.
fn contents(db: &mut Database) -> String {
    let tables = db
        .execute(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' ORDER BY table_name",
        )
        .map_or_else(
            |e| vec![format!("tables-err {e}")],
            |r| {
                r[0].rows
                    .iter()
                    .map(|row| format!("{:?}", row[0]))
                    .collect()
            },
        );
    let mut out = String::new();
    for t in &tables {
        let name = t.trim_start_matches("Text(\"").trim_end_matches("\")");
        let rows = db
            .execute(&format!("SELECT * FROM {name} ORDER BY 1, 2"))
            .or_else(|_| db.execute(&format!("SELECT * FROM {name} ORDER BY 1")));
        match rows {
            Ok(r) => writeln!(out, "{name}: {:?}", r[0].rows).unwrap(),
            Err(e) => writeln!(out, "{name}: err {e}").unwrap(),
        }
    }
    out
}

/// A 64-bit FNV-1a digest, so an acknowledged state is cheap to keep.
fn digest(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A workload's recorded run.
struct Recorded {
    /// Every operation the engine issued.
    ops: Vec<IoOp>,
    /// Each acknowledged state as `(op count at the ack, digest of every table's
    /// rows)`, the first being the empty store.
    acks: Vec<(usize, u64)>,
    /// The op count before each statement ran.
    starts: Vec<usize>,
}

impl Recorded {
    /// The statement op `i` was issued by.
    fn statement_of<'w>(&self, workload: &'w Workload, i: usize) -> &'w str {
        let k = self.starts.iter().rposition(|&s| s <= i).unwrap_or(0);
        workload
            .get(k)
            .map_or("(after the workload)", |(sql, _)| sql.as_str())
    }
}

/// Run `workload` over a recording device.
fn record(workload: &Workload) -> Recorded {
    let io = RecordingBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    let mut acks = vec![(io.op_count(), digest(&contents(&mut db)))];
    let mut starts = Vec::new();
    for (sql, commits) in workload {
        starts.push(io.op_count());
        db.execute(sql).unwrap_or_else(|e| panic!("{sql:.80}: {e}"));
        if *commits {
            acks.push((io.op_count(), digest(&contents(&mut db))));
        }
    }
    drop(db);
    Recorded {
        ops: io.ops(),
        acks,
        starts,
    }
}

/// Rows written and removed after a reopen, long enough to allocate many pages: a
/// free list holding a live page hands it out here and the write lands on it.
const CHURN: [&str; 2] = ["CREATE TABLE churn (id INT, v TEXT)", "DROP TABLE churn"];

fn churn(db: &mut Database) -> Result<(), String> {
    db.execute(CHURN[0]).map_err(|e| e.to_string())?;
    for i in 0..12 {
        db.execute(&format!(
            "INSERT INTO churn VALUES ({i}, '{}')",
            text(i, 9000)
        ))
        .map_err(|e| e.to_string())?;
    }
    db.execute("DELETE FROM churn WHERE id % 2 = 0")
        .map_err(|e| e.to_string())?;
    db.execute(CHURN[1]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Check every power-loss image of `workload`'s log. Returns the failures.
fn sweep(workload: &Workload, subsets: usize, seed: u64) -> (usize, Vec<String>) {
    let Recorded { ops, acks, .. } = record(workload);
    let images = power_loss_images(&ops, subsets, seed);
    let mut failures = Vec::new();
    for img in &images {
        let tag = format!("boundary {} ({})", img.boundary, img.landing);
        // An ack is durable once its last op — the commit's fsync — is at or before
        // the last sync this image keeps.
        let durable = acks
            .iter()
            .rposition(|(n, _)| *n <= img.durable_through + 1)
            .unwrap_or(0);
        let mut allowed = acks[durable..(durable + 2).min(acks.len())]
            .iter()
            .map(|(_, d)| *d);
        let audit = match audit_image(&img.image) {
            Ok(a) => a,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let live_free = audit.live_on_free_list();
        if !live_free.is_empty() {
            failures.push(format!("{tag}: free list holds live pages {live_free:?}"));
            continue;
        }
        let aliases = audit.free_list_aliases();
        if !aliases.is_empty() {
            failures.push(format!("{tag}: free list aliases {aliases:?}"));
            continue;
        }
        // What a kill leaves (every write landed) and what a crash before any write
        // leaves hold no torn state, so every allocated page is accounted for: a page
        // superseded but not yet collected is in the slot's retired-page log.
        if img.landing == "all" || img.landing == "none" {
            let leaked = audit.leaked();
            if !leaked.is_empty() {
                failures.push(format!("{tag}: {} pages leaked: {leaked:?}", leaked.len()));
                continue;
            }
        }
        let mut db = match try_open_database(Box::new(MemBlockIo::from_bytes(img.image.clone()))) {
            Ok(db) => db,
            Err(e) => {
                failures.push(format!("{tag}: open refused: {e}"));
                continue;
            }
        };
        let before = contents(&mut db);
        let adopted = digest(&before);
        if !allowed.any(|d| d == adopted) {
            failures.push(format!(
                "{tag}: adopted state matches no acknowledged commit: {before:.200}"
            ));
            continue;
        }
        if let Err(e) = churn(&mut db) {
            failures.push(format!("{tag}: a write after reopen failed: {e}"));
            continue;
        }
        let after = contents(&mut db);
        if after != before {
            failures.push(format!("{tag}: a write after reopen changed other rows"));
        }
    }
    (images.len(), failures)
}

fn assert_sweep(workload: &Workload, subsets: usize, seed: u64) {
    let (images, failures) = sweep(workload, subsets, seed);
    assert!(
        failures.is_empty(),
        "{} of {images} power-loss images failed; first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(8)]
    );
}

#[test]
fn every_power_loss_image_of_a_mixed_workload_recovers_an_acknowledged_state() {
    // Seed 4 over 30 steps is a workload whose free list spans chain pages a commit
    // rewrites: with the chain's checksums unchecked, 18 of its images recover a free
    // list naming live pages.
    assert_sweep(&mixed_workload(4, 30), 4, 4);
}

#[test]
fn every_power_loss_image_of_a_ddl_workload_recovers_an_acknowledged_state() {
    assert_sweep(&ddl_workload(), 4, 7);
}

/// The pages a durable image's adopted slot reaches or names: its tree pages and
/// overflow links, and its free-list and retired-log chain pages.
fn durable_footprint(image: &[u8]) -> BTreeSet<PageId> {
    let (store, meta) =
        DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image.to_vec()))).expect("opens");
    let mut pages = reachable_pages(&store, meta.root).expect("the durable tree reads");
    // The adopted slot's chain is the one it names; the alternate slot's chain is
    // the next commit's to rewrite.
    let [adopted, _] = store.persisted_chain_pages();
    pages.extend(adopted);
    pages
}

/// Replay `ops` and return every write that lands on a page the last durable slot
/// reaches before a newer slot is durable, as `(op index, page)`.
fn writes_into_durable_footprint(ops: &[IoOp]) -> Vec<(usize, PageId)> {
    let mut image = Vec::new();
    let mut footprint: BTreeSet<PageId> = BTreeSet::new();
    let mut slot_pending = false;
    let mut hits = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match op {
            IoOp::Write { offset, bytes } if *offset >= DATA_REGION_START => {
                let first = (offset - DATA_REGION_START) / PAGE_SIZE as u64;
                let last = (offset + bytes.len() as u64 - 1 - DATA_REGION_START) / PAGE_SIZE as u64;
                for page in first..=last {
                    if footprint.contains(&page) {
                        hits.push((i, page));
                    }
                }
            }
            IoOp::Write { .. } => slot_pending = true,
            IoOp::Sync if slot_pending => {
                slot_pending = false;
                apply_op(&mut image, op);
                footprint = durable_footprint(&image);
                continue;
            }
            _ => {}
        }
        apply_op(&mut image, op);
    }
    hits
}

#[test]
fn no_statement_writes_a_page_the_durable_slot_reaches() {
    for (name, workload) in [
        ("ddl", ddl_workload()),
        ("mixed 3", mixed_workload(3, 16)),
        ("mixed 8", mixed_workload(8, 16)),
    ] {
        let recorded = record(&workload);
        let hits = writes_into_durable_footprint(&recorded.ops);
        let named: Vec<String> = hits
            .iter()
            .take(4)
            .map(|&(i, page)| {
                format!(
                    "page {page} by `{:.60}`",
                    recorded.statement_of(&workload, i)
                )
            })
            .collect();
        assert!(
            hits.is_empty(),
            "{name}: {} writes landed on pages the durable slot reaches: {named:#?}",
            hits.len()
        );
    }
}

/// A commit whose meta-slot write fails leaves the older slot the durable one, and
/// the store writes nothing more until it is reopened: every statement after the
/// failure is refused, so no page that slot reaches is written over, and the reopen
/// adopts it.
#[test]
fn a_failed_commit_leaves_the_durable_slot_protected() {
    let io = RecordingBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    // `t`, then `big0` and its twelve long rows.
    for (sql, _) in &ddl_workload()[..15] {
        db.execute(sql).unwrap_or_else(|e| panic!("{sql:.80}: {e}"));
    }
    let durable = contents(&mut db);
    let failed_at = io.op_count();
    io.fail_slot_writes(true);
    let failed = db.execute("DELETE FROM big0");
    io.fail_slot_writes(false);
    assert!(failed.is_err(), "the injected slot failure is reported");
    let after_failure = io.op_count();
    for sql in [
        format!("INSERT INTO t VALUES (1000, '{}')", text(1000, 30_000)),
        "CREATE TABLE after1 (id INT, v TEXT)".to_string(),
    ] {
        let refused = db.execute(&sql).expect_err("a failed store serves nothing");
        assert_eq!(refused.sqlstate(), "57P02", "{sql:.60}: {refused}");
    }
    drop(db);
    let ops = io.ops();
    assert!(
        ops[after_failure..]
            .iter()
            .all(|op| !matches!(op, IoOp::Write { .. })),
        "a store that failed a slot write wrote again"
    );
    let hits: Vec<(usize, PageId)> = writes_into_durable_footprint(&ops)
        .into_iter()
        .filter(|&(i, _)| i >= failed_at)
        .collect();
    assert!(
        hits.is_empty(),
        "{} writes after the failed commit landed on pages the durable slot reaches, first {:?}",
        hits.len(),
        &hits[..hits.len().min(8)]
    );
    let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.image())));
    assert_eq!(
        contents(&mut reopened),
        durable,
        "the reopen adopts the durable slot"
    );
}

/// The rows of `t` as a list of ids, or the error's SQLSTATE.
fn ids(db: &mut Database, conn: i32) -> Result<Vec<i64>, String> {
    db.execute_as("SELECT id FROM t ORDER BY id", conn)
        .map(|r| {
            r[0].rows
                .iter()
                .map(|row| match &row[0] {
                    pglet_sql::types::Value::Int4(v) => i64::from(*v),
                    other => panic!("{other:?}"),
                })
                .collect()
        })
        .map_err(|e| e.sqlstate().to_string())
}

/// The tree version `db`'s store has published.
fn tree_version(db: &Database) -> u64 {
    let pglet::StorageBackend::Btree(b) = db.storage() else {
        panic!("a B+tree database");
    };
    b.inner().current_version()
}

/// The pages `db`'s store holds in use: every id it has allocated and not listed free.
fn pages_in_use(db: &Database) -> u64 {
    let pglet::StorageBackend::Btree(b) = db.storage() else {
        panic!("a B+tree database");
    };
    let store = b.inner().store();
    store.next_page_id() - store.free_list_size() as u64
}

/// In a statement that does no DDL, a refused write fails the store exactly when a batch
/// the statement itself published is visible without being durable — the `SERIAL`
/// insert's catalog batch, written after its rows — or the refused write is the save's.
/// A write refused before the statement published anything, its staged rows being the
/// whole of it, is an ordinary error (`53100` for a full disk, `58030` otherwise, as
/// 18.6's `mdextend` answers): the rows are gone, nothing leaked, and the database
/// serves on. The rule for blocks with DDL is the tests after it.
///
/// Swept over where the refusal lands — each of the first data-page writes, a data
/// `fsync`, a full disk — in five statement shapes: a one-row and a multi-page
/// autocommit `INSERT`, one into a `SERIAL` table (whose sequence advance makes the
/// commit write a catalog batch after the rows), and the one-row and the `SERIAL` inserts
/// committed from a block. Whether the refusal landed before or after a batch published is read
/// off the tree version, and the verdict asserted is the relation between the two:
/// refused-and-serving iff nothing was published. Both halves are witnessed, so a rule
/// that failed on every refusal, or on none, is red.
#[test]
fn a_refused_write_without_ddl_fails_the_store_only_past_its_own_published_batch() {
    type Arm = (&'static str, fn(&FaultPolicy));
    let arms: [Arm; 7] = [
        ("first data write", |p| p.arm_fail_data_writes_after(0)),
        ("second data write", |p| p.arm_fail_data_writes_after(1)),
        ("third data write", |p| p.arm_fail_data_writes_after(2)),
        ("fourth data write", |p| p.arm_fail_data_writes_after(3)),
        ("fsync", FaultPolicy::arm_fail_syncs),
        ("full disk", |p| p.arm_fill_disk_after(0)),
        ("full disk after a write", |p| p.arm_fill_disk_after(1)),
    ];
    let long = text(2, 30_000);
    let shapes: [(&str, bool, String); 5] = [
        ("insert", false, "INSERT INTO t VALUES (2, 'b')".into()),
        (
            "long insert",
            false,
            format!("INSERT INTO t VALUES (2, '{long}')"),
        ),
        (
            "serial insert",
            false,
            "INSERT INTO s (v) VALUES ('b')".into(),
        ),
        ("commit", true, "INSERT INTO t VALUES (2, 'b')".into()),
        (
            "serial commit",
            true,
            "INSERT INTO s (v) VALUES ('b')".into(),
        ),
    ];
    let (mut dropped, mut failed) = (0, 0);
    for (shape, block, sql) in &shapes {
        for (fault, arm) in arms {
            let label = format!("{shape} under a refused {fault}");
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for setup in [
                "CREATE TABLE t (id INT, v TEXT)",
                "INSERT INTO t VALUES (1, 'a')",
                "CREATE TABLE s (id SERIAL, v TEXT)",
                "INSERT INTO s (v) VALUES ('a')",
            ] {
                db.execute(setup).unwrap();
            }
            let durable = contents(&mut db);
            if *block {
                db.execute("BEGIN").unwrap();
                db.execute(sql).unwrap();
            }
            let (version, in_use) = (tree_version(&db), pages_in_use(&db));
            let policy = io.policy();
            arm(&policy);
            let result = db.execute(if *block { "COMMIT" } else { sql });
            policy.disarm();
            let Err(refused) = result else {
                // The refusal landed past every write the statement made.
                continue;
            };
            let code = if fault.starts_with("full disk") {
                "53100"
            } else {
                "58030"
            };
            assert_eq!(refused.sqlstate(), code, "{label}: {refused}");

            if tree_version(&db) == version {
                // Nothing published: the statement is dropped and the database serves.
                dropped += 1;
                assert!(
                    matches!(refused, pglet_sql::Error::WriteRefused { .. }),
                    "{label}: a refusal before anything published is an ordinary error: {refused:?}"
                );
                assert_eq!(
                    pages_in_use(&db),
                    in_use,
                    "{label}: the batch's pages are handed back"
                );
                assert_eq!(contents(&mut db), durable, "{label}: the rows are gone");
                assert_eq!(ids(&mut db, 7), Ok(vec![1]), "{label}: another connection");
                // A block whose commit was refused has ended: this write is autocommit,
                // and another connection reads it.
                db.execute("INSERT INTO t VALUES (3, 'c')")
                    .unwrap_or_else(|e| panic!("{label}: the next statement: {e}"));
                assert_eq!(
                    ids(&mut db, 7),
                    Ok(vec![1, 3]),
                    "{label}: the next statement"
                );
                let served = contents(&mut db);
                drop(db);
                let image = io.snapshot_bytes();
                let audit = audit_image(&image).expect("the store opens");
                assert!(
                    audit.leaked().is_empty()
                        && audit.free_list_aliases().is_empty()
                        && audit.live_on_free_list().is_empty(),
                    "{label}: leaked {:?}, aliased {:?}, live on the free list {:?}",
                    audit.leaked(),
                    audit.free_list_aliases(),
                    audit.live_on_free_list()
                );
                let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(image)));
                assert_eq!(contents(&mut reopened), served, "{label}: the reopen");
            } else {
                // A batch published and cannot be made durable: the store fails.
                failed += 1;
                assert!(
                    matches!(refused, pglet_sql::Error::DurableWriteFailed { .. }),
                    "{label}: a refusal past a published batch fails the store: {refused:?}"
                );
                assert_eq!(
                    ids(&mut db, 0),
                    Err("57P02".into()),
                    "{label}: same connection"
                );
                assert_eq!(
                    ids(&mut db, 7),
                    Err("57P02".into()),
                    "{label}: another connection"
                );
                let refused = db
                    .execute("INSERT INTO t VALUES (3, 'c')")
                    .expect_err("a failed store serves nothing");
                assert_eq!(refused.sqlstate(), "57P02", "{label}: {refused}");
                drop(db);
                let mut reopened =
                    open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
                assert_eq!(contents(&mut reopened), durable, "{label}: the reopen");
            }
        }
    }
    assert!(
        dropped > 0 && failed > 0,
        "the sweep drives both halves: {dropped} refusals dropped, {failed} failed the store"
    );
}

/// A refused write that cannot be dropped fails the store, and from then on it serves
/// nothing on any connection (`57P02`): not the refused write, not a
/// `ROLLBACK` of the block whose `COMMIT` failed, not a later commit that would make any
/// of it durable. A reopen reads only what the durable slot holds.
///
/// The cells are the ways a statement's effect cannot be dropped: the meta slot's write
/// refused, in an autocommit write, a `COMMIT` and a `CREATE TABLE` (the save is the one
/// write that may land though refused); a `CREATE TABLE` whose second batch is refused
/// after its first published (witnessed by the tree version); a block whose `CREATE
/// TABLE` published before its `COMMIT`'s batch was refused; and three refused before
/// anything published, whose other effects no rollback takes back — a `CREATE TABLE`
/// refused at its first write and a `TRUNCATE … RESTART IDENTITY` at its commit (an
/// autocommit statement's catalog change is live in the session and stays), and a block
/// that dropped a table (the storage layer forgot the table's name and indexes as the
/// commit ran).
#[test]
fn a_refused_write_that_cannot_be_dropped_is_refused_until_a_reopen() {
    struct Cell {
        label: &'static str,
        arm: fn(&FaultPolicy),
        setup: &'static [&'static str],
        before: &'static [&'static str],
        sql: &'static str,
        code: &'static str,
        published: Option<bool>,
    }
    let fail = |p: &FaultPolicy| p.arm_fail_data_writes_after(0);
    let cells = [
        Cell {
            label: "autocommit, slot write",
            arm: FaultPolicy::arm_fail_slot_writes,
            setup: &[],
            before: &[],
            sql: "INSERT INTO t VALUES (2)",
            code: "58030",
            published: None,
        },
        Cell {
            label: "commit, slot write",
            arm: FaultPolicy::arm_fail_slot_writes,
            setup: &[],
            before: &["BEGIN", "INSERT INTO t VALUES (2)"],
            sql: "COMMIT",
            code: "58030",
            published: None,
        },
        Cell {
            label: "ddl, slot write",
            arm: FaultPolicy::arm_fail_slot_writes,
            setup: &[],
            before: &[],
            sql: "CREATE TABLE u (k INT)",
            code: "58030",
            published: None,
        },
        Cell {
            label: "ddl, second batch",
            arm: |p| p.arm_fill_disk_after(1),
            setup: &[],
            before: &[],
            sql: "CREATE TABLE u (k INT PRIMARY KEY)",
            code: "53100",
            published: Some(true),
        },
        Cell {
            label: "commit after ddl, data write",
            arm: fail,
            setup: &[],
            before: &[
                "BEGIN",
                "CREATE TABLE u (k INT)",
                "INSERT INTO t VALUES (2)",
            ],
            sql: "COMMIT",
            code: "58030",
            published: None,
        },
        Cell {
            label: "ddl, first write",
            arm: fail,
            setup: &[],
            before: &[],
            sql: "CREATE TABLE u (k INT)",
            code: "58030",
            published: Some(false),
        },
        Cell {
            label: "truncate restarting identity, commit",
            arm: fail,
            setup: &[
                "CREATE TABLE s (id SERIAL, v TEXT)",
                "INSERT INTO s (v) VALUES ('a'), ('b')",
            ],
            before: &[],
            sql: "TRUNCATE s RESTART IDENTITY",
            code: "58030",
            published: Some(false),
        },
        Cell {
            label: "commit after drop table, data write",
            arm: fail,
            setup: &[
                "CREATE TABLE x (id INT PRIMARY KEY)",
                "INSERT INTO x VALUES (1), (2)",
            ],
            before: &["BEGIN", "DROP TABLE x"],
            sql: "COMMIT",
            code: "58030",
            published: Some(false),
        },
    ];
    for cell in cells {
        let label = cell.label;
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        for sql in cell.setup {
            db.execute(sql).unwrap();
        }
        let durable = contents(&mut db);
        let durable_version = tree_version(&db);
        for sql in cell.before {
            db.execute(sql).unwrap();
        }
        let version = tree_version(&db);
        let policy = io.policy();
        (cell.arm)(&policy);
        let failed = db.execute(cell.sql);
        policy.disarm();
        let failed = failed.expect_err("the injected failure is reported");
        // A refused slot write may have landed part-way, so its answer is the unknown
        // outcome; every other cell refused before a slot naming its writes was written.
        if label.ends_with("slot write") {
            assert!(
                matches!(failed, pglet_sql::Error::DurableWriteUnconfirmed { .. }),
                "{label}: {failed:?}"
            );
        } else {
            assert!(
                matches!(failed, pglet_sql::Error::DurableWriteFailed { .. }),
                "{label}: {failed:?}"
            );
        }
        assert_eq!(failed.sqlstate(), cell.code, "{label}: {failed}");
        // What each cell stands for: whether a batch had published ahead of the durable
        // slot when the refusal came, where that is the cell's point.
        if let Some(published) = cell.published {
            assert_eq!(
                tree_version(&db) > durable_version,
                published,
                "{label}: the witness"
            );
        }
        if label == "commit after ddl, data write" {
            assert!(version > durable_version, "{label}: the witness");
        }

        assert_eq!(
            ids(&mut db, 0),
            Err("57P02".into()),
            "{label}: same connection"
        );
        assert_eq!(
            ids(&mut db, 7),
            Err("57P02".into()),
            "{label}: another connection"
        );
        for sql in [
            "ROLLBACK",
            "SELECT count(*) FROM u",
            "INSERT INTO t VALUES (3)",
        ] {
            let refused = db.execute(sql).expect_err(sql);
            assert_eq!(refused.sqlstate(), "57P02", "{label}: {sql}: {refused}");
        }
        let sync = db.sync().expect_err("a failed store persists nothing");
        assert_eq!(sync.sqlstate(), "57P02", "{label}: sync: {sync}");
        drop(db);

        let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
        assert_eq!(contents(&mut reopened), durable, "{label}: the reopen");
        let u = reopened.execute("SELECT count(*) FROM u");
        assert_eq!(
            u.map_err(|e| e.sqlstate().to_string()).err().as_deref(),
            Some("42P01"),
            "{label}: the reopen holds no `u`"
        );
        reopened.execute("INSERT INTO t VALUES (4)").unwrap();
        assert_eq!(
            ids(&mut reopened, 0),
            Ok(vec![1, 4]),
            "{label}: the reopen writes"
        );
    }
}

/// A refused write after which the statement may be durable is never answered as a
/// definite failure. The meta fsync refused after the slot's bytes reached the device
/// (a failed writeback, which leaves them readable) is the case: a reopen adopts the slot
/// and serves the statement, so the answer must say the outcome is unknown
/// (`DurableWriteUnconfirmed`) — told it failed, a client retrying it would write it
/// twice. The definite answer (`DurableWriteFailed`) stays where nothing was saved: a
/// `SERIAL` insert whose catalog batch after its rows is refused, which the reopen does
/// not hold.
///
/// Swept over every sync a statement issues, in an autocommit `INSERT`, a block's
/// `COMMIT`, and a `SERIAL` insert (rows, then its catalog batch, then the save), and over
/// a refused slot write. The relation asserted: a reopen that holds the statement never
/// follows a definite answer. Both answers are witnessed, and a reopen holding the row
/// after a refusal is too, so a rule answering either one always is red.
#[test]
fn a_refused_write_the_statement_may_survive_is_not_answered_as_failed() {
    let shapes: [(&str, &[&str], &str); 3] = [
        ("insert", &[], "INSERT INTO t VALUES (2)"),
        ("commit", &["BEGIN", "INSERT INTO t VALUES (2)"], "COMMIT"),
        (
            "serial insert",
            &[],
            "INSERT INTO s (id, v) VALUES (2, 'b')",
        ),
    ];
    let (mut unconfirmed, mut failed, mut survived) = (0, 0, 0);
    for (shape, before, sql) in shapes {
        for fault in 0..4_usize {
            let label = if fault == 3 {
                format!("{shape} under a refused slot write")
            } else {
                format!("{shape} under a refused sync {}", fault + 1)
            };
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for setup in [
                "CREATE TABLE t (id INT)",
                "INSERT INTO t VALUES (1)",
                "CREATE TABLE s (id INT, v TEXT, n SERIAL)",
                "INSERT INTO s (id, v) VALUES (1, 'a')",
            ] {
                db.execute(setup).unwrap();
            }
            for sql in before {
                db.execute(sql).unwrap();
            }
            let policy = io.policy();
            if fault == 3 {
                policy.arm_fail_slot_writes();
            } else {
                policy.arm_fail_syncs_after(fault);
            }
            let result = db.execute(sql);
            policy.disarm();
            let Err(refused) = result else {
                // The statement issued fewer syncs than this one.
                continue;
            };
            drop(db);
            let mut reopened = open_database(Box::new(io.reopen()));
            let table = if shape.starts_with("serial") {
                "s"
            } else {
                "t"
            };
            let rows = reopened
                .execute(&format!("SELECT id FROM {table} WHERE id = 2"))
                .unwrap()[0]
                .rows
                .len();
            // The discriminating assertion first: a statement the reopen holds was not
            // answered as failed.
            assert!(
                rows == 0 || !matches!(refused, pglet_sql::Error::DurableWriteFailed { .. }),
                "{label}: answered as failed, and the reopen holds it: {refused}"
            );
            assert_eq!(refused.sqlstate(), "58030", "{label}: {refused}");
            survived += rows;
            match refused {
                pglet_sql::Error::DurableWriteUnconfirmed { .. } => unconfirmed += 1,
                pglet_sql::Error::DurableWriteFailed { .. } => failed += 1,
                other => {
                    // A refusal dropped before anything was saved: the store serves on.
                    assert!(
                        matches!(other, pglet_sql::Error::WriteRefused { .. }),
                        "{label}: {other:?}"
                    );
                    assert_eq!(rows, 0, "{label}: a dropped statement");
                }
            }
        }
    }
    assert!(
        unconfirmed > 0 && failed > 0 && survived > 0,
        "the sweep drives both answers and a surviving statement: {unconfirmed} unknown, \
         {failed} failed, {survived} survived"
    );
}

/// The ids of the indexes `db`'s storage layer maintains, sorted.
fn index_registry(db: &Database) -> Vec<u32> {
    let pglet::StorageBackend::Btree(b) = db.storage() else {
        panic!("a B+tree database");
    };
    let mut ids: Vec<u32> = b.index_defs().keys().copied().collect();
    ids.sort_unstable();
    ids
}

/// Reads over `t` and `emp` that an index serves, each answer or error as text.
fn index_reads(db: &mut Database) -> String {
    let mut out = String::new();
    for sql in [
        "SELECT id, v FROM t ORDER BY id",
        "SELECT id FROM t WHERE v = 'b'",
        "SELECT id FROM t WHERE v = 'e'",
        "SELECT id FROM t WHERE v = 'later'",
        "SELECT id FROM emp WHERE v = 'later'",
        "SELECT id FROM emp WHERE id = 5",
    ] {
        match db.execute_as(sql, 7) {
            Ok(r) => writeln!(out, "{sql} => {:?}", r[0].rows).unwrap(),
            Err(e) => writeln!(out, "{sql} => {} {e}", e.sqlstate()).unwrap(),
        }
    }
    out
}

/// A block whose storage-layer DDL is an index drop or build, and whose `COMMIT` meets a
/// refused write, either fails the store or leaves the storage layer maintaining exactly
/// the indexes a reopen does, with the index-served reads it checks the same live and
/// after the reopen. The storage layer applies such DDL outside the commit's batch — a drop
/// deregisters the index as the commit runs, a build publishes its back-fill (none, over
/// an empty table) and registers it when it runs — so dropping the refused batch alone
/// leaves the registry out of step with the catalog the block's rollback restores: a
/// dropped index no write maintains, which a reopen reattaches without the rows written
/// meanwhile, or a built one maintained for no index the catalog holds.
///
/// Cells: `DROP INDEX`, `CREATE INDEX` over a table with rows and over an empty one, and
/// `ADD CONSTRAINT … UNIQUE` over an empty table, each with a row inserted into `t`, under
/// a refused first data write, a refused data `fsync` and a full disk. The relation is
/// what is asserted, so a commit that takes such DDL back and serves on passes too.
#[test]
fn a_refused_commit_after_index_ddl_leaves_no_index_a_reopen_lacks() {
    type Arm = (&'static str, fn(&FaultPolicy));
    let arms: [Arm; 3] = [
        ("data write", |p| p.arm_fail_data_writes_after(0)),
        ("fsync", FaultPolicy::arm_fail_syncs),
        ("full disk", |p| p.arm_fill_disk_after(0)),
    ];
    let cells: [(&str, &str); 4] = [
        ("drop index", "DROP INDEX t_v"),
        ("create index", "CREATE INDEX t_id_v ON t (id, v)"),
        (
            "create index on an empty table",
            "CREATE INDEX emp_v ON emp (v)",
        ),
        (
            "add unique on an empty table",
            "ALTER TABLE emp ADD CONSTRAINT emp_u UNIQUE (id)",
        ),
    ];
    for (cell, ddl) in cells {
        for (fault, arm) in arms {
            let label = format!("{cell} under a refused {fault}");
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for setup in [
                "CREATE TABLE t (id INT PRIMARY KEY, v TEXT)",
                "CREATE INDEX t_v ON t (v)",
                "INSERT INTO t VALUES (1, 'a'), (5, 'e')",
                "CREATE TABLE emp (id INT, v TEXT)",
            ] {
                db.execute(setup).unwrap();
            }
            let durable = contents(&mut db);
            let reads = index_reads(&mut db);
            for sql in ["BEGIN", ddl, "INSERT INTO t VALUES (2, 'b')"] {
                db.execute(sql)
                    .unwrap_or_else(|e| panic!("{label}: {sql}: {e}"));
            }
            let policy = io.policy();
            arm(&policy);
            let result = db.execute("COMMIT");
            policy.disarm();
            let refused = result.expect_err("the injected failure is reported");
            match refused {
                pglet_sql::Error::WriteRefused { .. } => {
                    // The store serves on: the block ended rolled back, so every read is
                    // what it was before the block, and the registry must be the one a
                    // reopen builds from the catalog.
                    assert_eq!(index_reads(&mut db), reads, "{label}: across the refusal");
                    for sql in [
                        "INSERT INTO t VALUES (77, 'later')",
                        "INSERT INTO emp VALUES (5, 'later')",
                    ] {
                        db.execute(sql)
                            .unwrap_or_else(|e| panic!("{label}: {sql}: {e}"));
                    }
                    let (registry, served) = (index_registry(&db), index_reads(&mut db));
                    drop(db);
                    let mut reopened = open_database(Box::new(io.reopen()));
                    assert_eq!(
                        index_registry(&reopened),
                        registry,
                        "{label}: the indexes maintained live and after a reopen"
                    );
                    assert_eq!(
                        index_reads(&mut reopened),
                        served,
                        "{label}: the reads live and after a reopen"
                    );
                }
                pglet_sql::Error::DurableWriteFailed { .. } => {
                    drop(db);
                    let mut reopened = open_database(Box::new(io.reopen()));
                    assert_eq!(contents(&mut reopened), durable, "{label}: the reopen");
                }
                other => panic!("{label}: {other:?}"),
            }
        }
    }
}

/// A block whose storage-layer DDL a savepoint rollback already undid serves on after its
/// `COMMIT` meets a refused write, like a block that never did it: the eager batches it
/// published are not the refused commit's, and what they left — nothing a reader or a
/// reopen tells from the pre-block state — the next save persists as it would have. The
/// store serving on, the reads it checks and the dump are unchanged across the refusal, a later
/// commit reads the same live and after a reopen, and the image audits clean. The cells:
/// `CREATE INDEX` over a table with rows, `CREATE TABLE … PRIMARY KEY` with a row, and
/// `ADD CONSTRAINT … UNIQUE`, each inside a savepoint rolled back, then a row inserted.
#[test]
fn a_refused_commit_after_ddl_a_savepoint_undid_serves_on() {
    let cells: [(&str, &[&str]); 3] = [
        ("create index", &["CREATE INDEX t_id_v ON t (id, v)"]),
        (
            "create table",
            &[
                "CREATE TABLE u (k INT PRIMARY KEY)",
                "INSERT INTO u VALUES (1)",
            ],
        ),
        (
            "add unique",
            &["ALTER TABLE t ADD CONSTRAINT t_v_u UNIQUE (v)"],
        ),
    ];
    for (cell, undone) in cells {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for setup in [
            "CREATE TABLE t (id INT PRIMARY KEY, v TEXT)",
            "CREATE INDEX t_v ON t (v)",
            "INSERT INTO t VALUES (1, \'a\'), (5, \'e\')",
            "CREATE TABLE emp (id INT, v TEXT)",
        ] {
            db.execute(setup).unwrap();
        }
        let (dump, reads) = (db.dump_sql().unwrap(), index_reads(&mut db));
        db.execute("BEGIN").unwrap();
        db.execute("SAVEPOINT a").unwrap();
        for sql in undone {
            db.execute(sql)
                .unwrap_or_else(|e| panic!("{cell}: {sql}: {e}"));
        }
        db.execute("ROLLBACK TO SAVEPOINT a").unwrap();
        db.execute("INSERT INTO t VALUES (2, \'b\')").unwrap();
        let policy = io.policy();
        policy.arm_fill_disk_after(0);
        let refused = db.execute("COMMIT");
        policy.disarm();
        let refused = refused.expect_err("the injected failure is reported");
        assert!(
            matches!(refused, pglet_sql::Error::WriteRefused { .. }),
            "{cell}: the block\'s own effects are all taken back: {refused:?}"
        );
        assert_eq!(
            db.dump_sql().unwrap(),
            dump,
            "{cell}: the dump across the refusal"
        );
        assert_eq!(
            index_reads(&mut db),
            reads,
            "{cell}: the reads across the refusal"
        );
        db.execute("INSERT INTO t VALUES (77, \'later\')").unwrap();
        let (registry, served) = (index_registry(&db), index_reads(&mut db));
        drop(db);
        let image = io.snapshot_bytes();
        let audit = audit_image(&image).expect("the store opens");
        assert!(
            audit.leaked().is_empty()
                && audit.free_list_aliases().is_empty()
                && audit.live_on_free_list().is_empty(),
            "{cell}: leaked {:?}, aliased {:?}, live on the free list {:?}",
            audit.leaked(),
            audit.free_list_aliases(),
            audit.live_on_free_list()
        );
        let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(image)));
        assert_eq!(
            index_registry(&reopened),
            registry,
            "{cell}: the registry after a reopen"
        );
        assert_eq!(
            index_reads(&mut reopened),
            served,
            "{cell}: the reads after a reopen"
        );
    }
}

/// A block whose `COMMIT` meets a write refused before anything published ends rolled
/// back, and the rollback takes back the block's catalog change with its rows: the store
/// serves on, as 18.6's does after a commit that raises an `ERROR`. Here the block made a
/// sequence and inserted a row; after the refusal the sequence is gone, the row is not
/// there, the connection is idle again, and a reopen holds the same.
#[test]
fn a_refused_commit_ends_its_block_rolled_back_with_its_catalog() {
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();
    let durable = contents(&mut db);
    db.execute("BEGIN").unwrap();
    db.execute("CREATE SEQUENCE q").unwrap();
    db.execute("INSERT INTO t VALUES (2)").unwrap();
    let version = tree_version(&db);
    let policy = io.policy();
    policy.arm_fill_disk_after(0);
    let refused = db.execute("COMMIT");
    policy.disarm();
    let refused = refused.expect_err("the injected failure is reported");
    assert!(
        matches!(refused, pglet_sql::Error::WriteRefused { .. }),
        "{refused:?}"
    );
    assert_eq!(refused.sqlstate(), "53100", "{refused}");
    assert_eq!(tree_version(&db), version, "the witness: nothing published");

    let q = db
        .execute("SELECT nextval('q')")
        .expect_err("the sequence is gone");
    assert_eq!(q.sqlstate(), "42P01", "{q}");
    assert_eq!(contents(&mut db), durable, "the block's row is gone");
    // Idle again: this write is autocommit, and another connection reads it.
    db.execute("INSERT INTO t VALUES (3)").unwrap();
    assert_eq!(ids(&mut db, 7), Ok(vec![1, 3]));
    let served = contents(&mut db);
    drop(db);
    let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
    assert_eq!(contents(&mut reopened), served, "the reopen");
    let q = reopened
        .execute("SELECT nextval('q')")
        .expect_err("no sequence after the reopen");
    assert_eq!(q.sqlstate(), "42P01", "{q}");
}

/// The setup `a_rolled_back_block_leaves_no_ddl_a_reopen_shows` and its neighbours build:
/// an indexed table, an enum over an indexed column, a sequence advanced once.
const ROLLED_BACK_SETUP: [&str; 9] = [
    "CREATE TABLE t (id INT PRIMARY KEY, v TEXT)",
    "CREATE INDEX t_v ON t (v)",
    "INSERT INTO t VALUES (1, 'a')",
    "CREATE TYPE mood AS ENUM ('x', 'y', 'z')",
    "CREATE TABLE e (id INT, m mood)",
    "INSERT INTO e VALUES (1, 'y'), (2, 'x')",
    "CREATE INDEX e_m ON e (m)",
    "CREATE SEQUENCE s1",
    "SELECT nextval('s1')",
];

/// Draw a value from a sequence with nothing saved: `sr_id_seq` created and saved, then
/// advanced by an autocommit `INSERT` that fails, whose staged write is dropped and whose
/// draw stands (as on 18.6) — a counter the durable catalog lags that no connection was
/// handed: a route that leaves a durable counter behind the live one on a healthy device.
fn draw_without_a_save(db: &mut Database) {
    db.execute("CREATE TABLE sr (x INT CHECK (x > 0), id SERIAL)")
        .unwrap();
    assert!(
        db.execute("INSERT INTO sr (x) VALUES (-1)").is_err(),
        "the draw's statement fails"
    );
}

/// The DDL a block rolls back in those tests, each a statement list. `ALTER SEQUENCE …
/// RESTART` leaves `Schema::generation` where it was (`Plan::mutates_schema`), so a save
/// keyed on the generation moving rather than on the restore having run skips it.
/// `RESTART` replaces a sequence's counter cell, so the counter is the change a rollback
/// takes back — where a counter advanced in place is not.
const ROLLED_BACK_DDL: [(&str, &[&str]); 7] = [
    ("create index", &["CREATE INDEX t_id ON t (id)"]),
    ("drop index", &["DROP INDEX t_v"]),
    (
        "enum re-created reversed",
        &["DROP TYPE mood", "CREATE TYPE mood AS ENUM ('z', 'y', 'x')"],
    ),
    (
        "create table",
        &[
            "CREATE TABLE u (k INT PRIMARY KEY)",
            "INSERT INTO u VALUES (1)",
        ],
    ),
    ("add column", &["ALTER TABLE t ADD COLUMN w INT DEFAULT 4"]),
    ("create sequence", &["CREATE SEQUENCE q"]),
    (
        "restart a sequence",
        &["ALTER SEQUENCE s1 RESTART WITH 100"],
    ),
];

/// What a reopen can show of a database: its dump, and an order its enum decides.
fn rolled_back_view(db: &mut Database) -> String {
    let order = db
        .execute("SELECT id FROM e ORDER BY m")
        .map(|r| format!("{:?}", r[0].rows));
    format!("{}\n-- ORDER BY m: {order:?}", db.dump_sql().unwrap())
}

/// Once a block's rollback has restored the catalog, no reopen shows the DDL it rolled
/// back — not after a crash right after the rollback returned, not after a clean close.
///
/// Every save but a rollback's counter save encodes the live catalog, and the catalog
/// holds an open block's DDL, so a save while the block is open — the block's own
/// `sync`, the one door that still writes one (the repo's `EDGES.md` row 211; another
/// connection's write now waits for, or is refused by, the block's writer) — makes that
/// DDL durable; the rollback then restored the catalog in memory alone. Measured before the
/// rollback saved: a reopen held an index the block created, lacked one it dropped, held
/// its reversed enum (an `ORDER BY` answered in the other order), its table, and its new
/// column with the existing row `NULL` — a state no statement built.
///
/// Seven DDL shapes × two ways a block ends in a rollback: `ROLLBACK`, and a `COMMIT` of
/// an aborted block × whether a `ROLLBACK TO SAVEPOINT` already undid the DDL after the
/// save — which restores the catalog in memory and saves nothing, so the image before the
/// end still holds the DDL, and the end's rollback has nothing left to restore that it
/// can see in the live catalog. A rollback that reads "no catalog change" off the
/// `ROLLBACK TO` skips its save and is red here. The block's own `sync` is the interleaved
/// save; witnessed per cell, an image taken just before the end reopens holding the DDL. Asserted: the database after the rollback
/// reads as one that never ran the block, and a reopen of the image taken as the rollback
/// returned, and of the one a clean close leaves, reads the same.
#[test]
fn a_rolled_back_block_leaves_no_ddl_a_reopen_shows() {
    let mut twin = open_database(Box::new(MemBlockIo::new()));
    for sql in ROLLED_BACK_SETUP {
        twin.execute(sql).unwrap();
    }
    let never_ran = rolled_back_view(&mut twin);
    drop(twin);
    for (shape, ddl) in ROLLED_BACK_DDL {
        for (end, undone) in [
            ("ROLLBACK", false),
            ("COMMIT of an aborted block", false),
            ("ROLLBACK", true),
            ("COMMIT of an aborted block", true),
        ] {
            let cell = format!(
                "{shape}, {end}{}",
                if undone {
                    ", the DDL undone by ROLLBACK TO after the save"
                } else {
                    ""
                }
            );
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for sql in ROLLED_BACK_SETUP {
                db.execute(sql).unwrap();
            }
            db.execute_as("BEGIN", 0).unwrap();
            if undone {
                db.execute_as("SAVEPOINT sp", 0).unwrap();
            }
            for sql in ddl {
                db.execute_as(sql, 0)
                    .unwrap_or_else(|e| panic!("{cell}: {sql}: {e}"));
            }
            db.sync().unwrap_or_else(|e| panic!("{cell}: sync: {e}"));
            if undone {
                db.execute_as("ROLLBACK TO sp", 0)
                    .unwrap_or_else(|e| panic!("{cell}: {e}"));
            }
            let before = io.snapshot_bytes();
            let answer = if end == "ROLLBACK" {
                db.execute_as("ROLLBACK", 0)
            } else {
                db.execute_as("SELECT 1 / 0", 0)
                    .expect_err("the block aborts");
                db.execute_as("COMMIT", 0)
            };
            answer.unwrap_or_else(|e| panic!("{cell}: {e}"));
            assert_eq!(db.transaction_depth(), 0, "{cell}: the block ended");
            let crash = io.snapshot_bytes();
            let live = rolled_back_view(&mut db);
            assert_eq!(live, never_ran, "{cell}: the rollback restored the catalog");
            drop(db);
            let closed = io.snapshot_bytes();

            let mut saved = open_database(Box::new(MemBlockIo::from_bytes(before)));
            assert_ne!(
                rolled_back_view(&mut saved),
                live,
                "{cell}: the witness — the interleaved save made the DDL durable"
            );
            for (image, what) in [(crash, "the rollback returned"), (closed, "a clean close")] {
                let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(image)));
                assert_eq!(
                    rolled_back_view(&mut reopened),
                    live,
                    "{cell}: a reopen after {what}"
                );
            }
        }
    }
}

/// Row 110's route (`EDGES.md`) is closed at its door: the row that made a rollback's enum
/// index-spec resync raise was another connection's, written while the block's reversed
/// order was live, so the partial index `WHERE m > 'ok'` skipped its key, `10 / d` over a
/// zero — and the restored order re-admitted it where that key cannot be evaluated. That write is now refused
/// `55P03` (the block holds the writer, and this host cannot make it wait), so the rollback
/// restores the order and answers without an error, and — past the block's own `sync`,
/// which made the reversed order durable (witnessed) — saves it: a reopen reads the order
/// the connections read.
#[test]
fn a_write_under_a_blocks_reordered_enum_is_refused_and_the_rollback_restores_the_order() {
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    for sql in [
        "CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')",
        "CREATE TABLE rb (id INT, d INT, m mood NOT NULL)",
        "CREATE INDEX a_f ON rb ((10 / d)) WHERE m > 'ok'",
    ] {
        db.execute(sql).unwrap();
    }
    db.execute_as("BEGIN", 0).unwrap();
    db.execute_as("DROP TYPE mood", 0).unwrap();
    db.execute_as("CREATE TYPE mood AS ENUM ('happy', 'ok', 'sad')", 0)
        .unwrap();
    let refused = db
        .execute_as("INSERT INTO rb VALUES (1, 0, 'happy')", 7)
        .expect_err("the block holds the writer");
    assert_eq!(refused.sqlstate(), "55P03", "{refused}");
    db.sync().unwrap();
    let before = io.snapshot_bytes();
    db.execute_as("ROLLBACK", 0)
        .unwrap_or_else(|e| panic!("no row the restored order cannot index: {e}"));
    assert_eq!(db.transaction_depth(), 0, "the block ended");
    let labels = "SELECT enumlabel FROM pg_enum ORDER BY enumsortorder";
    let order = |db: &mut Database| format!("{:?}", db.execute(labels).map(|r| r[0].rows.clone()));
    let live = order(&mut db);
    assert_eq!(
        live, r#"Ok([[Text("sad")], [Text("ok")], [Text("happy")]])"#,
        "the restored order"
    );
    let mut saved = open_database(Box::new(MemBlockIo::from_bytes(before)));
    assert_ne!(
        order(&mut saved),
        live,
        "the witness: the block's order was durable"
    );
    drop(db);
    let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
    assert_eq!(order(&mut reopened), live, "the reopen");
    assert_eq!(
        format!(
            "{:?}",
            reopened
                .execute("SELECT count(*) FROM rb")
                .map(|r| r[0].rows.clone())
        ),
        "Ok([[Int8(0)]])",
        "the refused row is nowhere"
    );
}

/// A catalog change no statement makes — a host writing the catalog through
/// `Database::schema_mut` inside an open block — is the block's (`schema_mut` takes the
/// block's writer), so the block's rollback takes it back whether or not a statement of the
/// block also wrote the catalog, and a save between the host's write and the rollback made
/// it durable, so the rollback must save the restored catalog. Measured before the save was keyed on the durable catalog's content: it
/// was keyed on a mark `record_statement` set at the block's first catalog-writing
/// statement, which the host's write never reaches, so a save before that statement read as
/// one that held none of the block — a reopen lacked the sequence the host removed while the
/// live database, rolled back, held it.
///
/// Cells: the host removes a sequence, or adds one; then a save (`Database::sync`); then a
/// `CREATE SEQUENCE`, or no
/// statement; then `ROLLBACK`, or the `COMMIT` of an aborted block. Witnessed per cell: an
/// image taken before the end reopens holding the host's change. Asserted: the database
/// after the end reads as before `BEGIN`, and a reopen of the image taken as the end
/// returned, and of the one a clean close leaves, reads as the live database.
#[test]
fn a_rollback_takes_back_a_catalog_change_no_statement_made() {
    type HostWrite = (&'static str, fn(&mut Database));
    let host_writes: [HostWrite; 2] = [
        ("remove a sequence", |db| {
            db.schema_mut().sequences.remove("s1");
        }),
        ("add a sequence", |db| {
            db.schema_mut()
                .sequences
                .insert("q0".into(), Sequence::new(1));
        }),
    ];
    for (write, host_write) in host_writes {
        for statement in [true, false] {
            for end in ["ROLLBACK", "COMMIT of an aborted block"] {
                let cell = format!(
                    "{write}, {}, {end}",
                    if statement {
                        "then a CREATE SEQUENCE"
                    } else {
                        "no statement"
                    }
                );
                let io = MemBlockIo::new();
                let mut db = open_database(Box::new(io.clone()));
                for sql in ROLLED_BACK_SETUP {
                    db.execute(sql).unwrap();
                }
                let before_begin = rolled_back_view(&mut db);
                db.execute_as("BEGIN", 0).unwrap();
                host_write(&mut db);
                db.sync().unwrap();
                if statement {
                    db.execute_as("CREATE SEQUENCE q", 0).unwrap();
                }
                let before = io.snapshot_bytes();
                let answer = if end == "ROLLBACK" {
                    db.execute_as("ROLLBACK", 0)
                } else {
                    db.execute_as("SELECT 1 / 0", 0)
                        .expect_err("the block aborts");
                    db.execute_as("COMMIT", 0)
                };
                answer.unwrap_or_else(|e| panic!("{cell}: {e}"));
                let crash = io.snapshot_bytes();
                let live = rolled_back_view(&mut db);
                assert_eq!(
                    live, before_begin,
                    "{cell}: the rollback restored the catalog"
                );
                drop(db);
                let closed = io.snapshot_bytes();

                let mut saved = open_database(Box::new(MemBlockIo::from_bytes(before)));
                assert_ne!(
                    rolled_back_view(&mut saved),
                    before_begin,
                    "{cell}: the witness — the save made the host's change durable"
                );
                for (image, what) in [(crash, "the end returned"), (closed, "a clean close")] {
                    let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(image)));
                    assert_eq!(
                        rolled_back_view(&mut reopened),
                        live,
                        "{cell}: a reopen after {what}"
                    );
                }
            }
        }
    }
}

/// The two polarities around a rollback's save. A block whose DDL no save followed, and that
/// advanced no sequence counter, writes no meta slot at its `ROLLBACK`: no durable slot holds its
/// DDL, so there is nothing to restore there — whether no save ran during the block at all, or one
/// ran only before the block's first catalog write (before the block's first statement, or after it
/// and before the DDL), when it wrote the very catalog the rollback restores. A save keyed on a
/// save since the block's `BEGIN` alone saves in those cells, and under a refused `fsync` the
/// needless save fails the whole store. The same holds of a sequence the block restarted after a
/// failed autocommit statement drew values from it, with no save since: the rollback restores the
/// cell as that draw left it, which no durable catalog need hold. (A sequence the block itself
/// advanced in place is saved at its rollback:
/// `a_value_nextval_handed_out_is_not_handed_out_again_after_a_close`.) And a save the device
/// refuses fails the store: the `ROLLBACK` answers the refusal (`58030`, or `53100` for a full
/// disk; the outcome unknown for the slot's own write), every later statement `57P02`, since the
/// durable catalog may hold DDL no connection sees.
#[test]
fn a_rollback_writes_no_slot_for_ddl_no_save_reached_and_a_refused_save_fails_the_store() {
    use pglet_sql::Error as E;
    type Arm = (&'static str, fn(&FaultPolicy));
    for (shape, ddl) in ROLLED_BACK_DDL {
        for save in ["none", "before the block", "between the INSERT and the DDL"] {
            let cell = format!("{shape}, the save {save}");
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for sql in ROLLED_BACK_SETUP {
                db.execute(sql).unwrap();
            }
            db.execute_as("BEGIN", 0).unwrap();
            if save == "before the block" {
                db.execute_as("INSERT INTO e VALUES (3, 'z')", 7).unwrap();
            }
            db.execute_as("INSERT INTO t VALUES (5, 'q')", 0).unwrap();
            if save == "between the INSERT and the DDL" {
                // The block holds the writer from its `INSERT` on, so the save here is
                // its own `sync`, which another connection's write no longer reaches.
                db.sync().unwrap();
            }
            for sql in ddl {
                db.execute_as(sql, 0).unwrap();
            }
            let slots =
                |io: &MemBlockIo| io.snapshot_bytes()[..DATA_REGION_START as usize].to_vec();
            let before = slots(&io);
            db.execute_as("ROLLBACK", 0).unwrap();
            assert_eq!(slots(&io), before, "{cell}: no slot written");
        }
    }
    // A counter advanced in place is no change a rollback takes back, and a replaced cell
    // is: a block that restarts `sr_id_seq`, which a failed autocommit `INSERT` advanced
    // before it with nothing saved, restores the cell as that advance left it. With no save
    // during the block, the durable catalog holds nothing of the block, though the restored
    // counter is one ahead of it; with a save before the restart, the durable catalog
    // already holds what the rollback restores. Neither rollback has anything to save.
    for (cell, save_between) in [
        ("an advance, then a restart, no save", false),
        ("an advance, a save, then a restart", true),
    ] {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in ROLLED_BACK_SETUP {
            db.execute(sql).unwrap();
        }
        draw_without_a_save(&mut db);
        db.execute_as("BEGIN", 0).unwrap();
        if save_between {
            db.execute_as("INSERT INTO e VALUES (3, 'z')", 7).unwrap();
        }
        db.execute_as("ALTER SEQUENCE sr_id_seq RESTART WITH 100", 0)
            .unwrap();
        let slots = |io: &MemBlockIo| io.snapshot_bytes()[..DATA_REGION_START as usize].to_vec();
        let before = slots(&io);
        db.execute_as("ROLLBACK", 0).unwrap();
        assert_eq!(slots(&io), before, "{cell}: no slot written");
    }
    let arms: [Arm; 3] = [
        ("data write", |p| p.arm_fail_data_writes_after(0)),
        ("full disk", |p| p.arm_fill_disk_after(0)),
        ("slot write", FaultPolicy::arm_fail_slot_writes),
    ];
    for (fault, arm) in arms {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in ROLLED_BACK_SETUP {
            db.execute(sql).unwrap();
        }
        db.execute_as("BEGIN", 0).unwrap();
        db.execute_as("ALTER TABLE t ADD COLUMN w INT DEFAULT 4", 0)
            .unwrap();
        db.sync().unwrap();
        let policy = io.policy();
        arm(&policy);
        let refused = db.execute_as("ROLLBACK", 0);
        policy.disarm();
        let refused = refused.expect_err("the refused save is the rollback's answer");
        let expected = if fault == "full disk" {
            "53100"
        } else {
            "58030"
        };
        assert_eq!(refused.sqlstate(), expected, "{fault}: {refused}");
        assert!(
            matches!(
                (fault, &refused),
                ("slot write", E::DurableWriteUnconfirmed { .. })
                    | ("data write" | "full disk", E::DurableWriteFailed { .. })
            ),
            "{fault}: {refused:?}"
        );
        for conn in [0, 7] {
            let next = db
                .execute_as("SELECT count(*) FROM t", conn)
                .expect_err("a failed store serves nothing");
            assert_eq!(
                next.sqlstate(),
                "57P02",
                "{fault}, connection {conn}: {next}"
            );
        }
    }
}

/// What a block leaves behind does not decide the next block's rollback or its refused
/// `COMMIT` on the same database. A first block replaces a sequence's counter cell and
/// creates a sequence, and commits. The second advances that sequence in place, saves (its
/// own `sync`), sets it back below where its `BEGIN` found it, inserts a row and
/// creates a sequence: no save follows its DDL, the in-place moves are no change a rollback
/// takes back, and a counter below its `BEGIN` value hands out nothing a save must keep, so
/// its `ROLLBACK` writes no meta slot, and its `COMMIT` meeting a full disk answers the
/// definite failure with a reopen holding none of the block. Red for state the first block
/// leaves standing that the second's decision reads — the first block's replaced cells,
/// which make the counter the save wrote read as a change of the second block's. (Set back
/// rather than advanced again, since an advance is saved at the rollback whatever the
/// decision reads.)
#[test]
fn a_block_after_a_catalog_writing_block_answers_as_its_own() {
    use pglet_sql::Error as E;
    for end in ["ROLLBACK", "COMMIT, the disk full"] {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in ROLLED_BACK_SETUP {
            db.execute(sql).unwrap();
        }
        for sql in [
            "BEGIN",
            "ALTER SEQUENCE s1 RESTART WITH 10",
            "CREATE SEQUENCE first_block",
            "COMMIT",
        ] {
            db.execute_as(sql, 0)
                .unwrap_or_else(|e| panic!("{end}: {sql}: {e}"));
        }
        db.execute_as("BEGIN", 0).unwrap();
        db.execute_as("SELECT nextval('s1')", 0).unwrap();
        db.sync().unwrap();
        db.execute_as("SELECT setval('s1', 5)", 0).unwrap();
        db.execute_as("INSERT INTO t VALUES (5, 'q')", 0).unwrap();
        db.execute_as("CREATE SEQUENCE q", 0).unwrap();
        let slots = |io: &MemBlockIo| io.snapshot_bytes()[..DATA_REGION_START as usize].to_vec();
        let before = slots(&io);
        if end == "ROLLBACK" {
            db.execute_as("ROLLBACK", 0).unwrap();
            assert_eq!(slots(&io), before, "{end}: no slot written");
            continue;
        }
        let policy = io.policy();
        policy.arm_fill_disk_after(0);
        let refused = db.execute_as("COMMIT", 0);
        policy.disarm();
        let refused = refused.expect_err("the full disk refuses the COMMIT");
        assert!(
            matches!(
                refused,
                E::DurableWriteFailed { .. } | E::WriteRefused { .. }
            ),
            "{end}: nothing of the block is durable, and the answer says so: {refused:?}"
        );
        assert_eq!(slots(&io), before, "{end}: no slot written");
        drop(db);
        let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
        assert!(
            reopened.execute("SELECT nextval('q')").is_err(),
            "{end}: the reopen holds no sequence q"
        );
        assert_eq!(
            ids(&mut reopened, 0),
            Ok(vec![1]),
            "{end}: nor the block's row"
        );
    }
}

/// A block no save reached is answered as one of which nothing is durable, whatever the
/// durable catalog already lacked when it opened. The durable catalog can differ from the
/// live one at `BEGIN` by what is no change of the block's: a counter a failed autocommit
/// statement drew from, which nothing saved and which handed no value out
/// (`draw_without_a_save`), a change a host made through `Database::schema_mut` that no save
/// has written. (A counter an earlier block advanced and rolled back is not among them: that
/// rollback saves it, `a_value_nextval_handed_out_is_not_handed_out_again_after_a_close`.)
/// With no save during the block — whose DDL replaces that very counter's cell, or creates
/// a sequence beside the host's change — its `ROLLBACK` writes no meta slot, and under a full disk answers without an error and
/// serves on; its `COMMIT` meeting a full disk answers the definite failure, and a reopen
/// holds none of the block. Witnessed per cell: an image taken at `BEGIN` reopens differing
/// from the live database.
///
/// Red for a decision that compares the durable catalog without first asking whether a save
/// has written one since `BEGIN`: the lagging counter, or the host's change, reads as the
/// block's, so the rollback saves a catalog — failing the store under the full disk — and
/// the refused `COMMIT` answers the unknown outcome.
#[test]
fn a_block_no_save_reached_answers_as_nothing_of_it_is_durable() {
    use pglet_sql::Error as E;
    type Before = (&'static str, fn(&mut Database), &'static [&'static str]);
    let befores: [Before; 2] = [
        (
            "a counter a failed autocommit statement drew from",
            draw_without_a_save,
            &[
                "ALTER SEQUENCE sr_id_seq RESTART WITH 50",
                "CREATE SEQUENCE q",
            ],
        ),
        (
            "a host change no save wrote",
            |db| {
                db.schema_mut().sequences.remove("s1");
            },
            &["CREATE SEQUENCE q"],
        ),
    ];
    for (before, prepare, ddl) in befores {
        for end in [
            "ROLLBACK",
            "ROLLBACK, the disk full",
            "COMMIT, the disk full",
        ] {
            let cell = format!("{before}, {end}");
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for sql in ROLLED_BACK_SETUP {
                db.execute(sql).unwrap();
            }
            prepare(&mut db);
            let at_begin = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())))
                .dump_sql()
                .unwrap();
            assert_ne!(
                at_begin,
                db.dump_sql().unwrap(),
                "{cell}: witness, the durable catalog differs at BEGIN"
            );
            db.execute_as("BEGIN", 0).unwrap();
            for sql in ddl {
                db.execute_as(sql, 0)
                    .unwrap_or_else(|e| panic!("{cell}: {sql}: {e}"));
            }
            let slots =
                |io: &MemBlockIo| io.snapshot_bytes()[..DATA_REGION_START as usize].to_vec();
            let slots_before = slots(&io);
            let policy = io.policy();
            if end.ends_with("the disk full") {
                policy.arm_fill_disk_after(0);
            }
            let answer = db.execute_as(end.split(',').next().unwrap(), 0);
            policy.disarm();
            if end.starts_with("ROLLBACK") {
                answer.unwrap_or_else(|e| panic!("{cell}: the rollback saves nothing: {e}"));
                assert_eq!(slots(&io), slots_before, "{cell}: no slot written");
                db.execute_as("SELECT 1", 7)
                    .unwrap_or_else(|e| panic!("{cell}: the store serves on: {e}"));
                continue;
            }
            let refused = answer.expect_err("the full disk refuses the COMMIT");
            assert!(
                matches!(
                    refused,
                    E::DurableWriteFailed { .. } | E::WriteRefused { .. }
                ),
                "{cell}: nothing of the block is durable, and the answer says so: {refused:?}"
            );
            drop(db);
            let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
            assert!(
                reopened.execute("SELECT nextval('q')").is_err(),
                "{cell}: the reopen holds no sequence q"
            );
            assert_eq!(
                reopened.dump_sql().unwrap(),
                at_begin,
                "{cell}: the reopen reads as the image at BEGIN"
            );
        }
    }
}

/// A rollback that saves writes the catalog it restored, counters included: every counter
/// advanced in place since the block's `BEGIN` — another connection's, committed while the
/// block had only read, and the block's own `nextval` — and the cell a `RESTART` replaced,
/// put back as those advances left it. The block's own `sync` holding its `CREATE SEQUENCE`
/// makes the rollback save; a reopen of the image it leaves answers `nextval('s1')` as the
/// live database does and holds no `q`. Witnessed per cell: the image before the `ROLLBACK`
/// reopens holding `q`. Red for a save of any catalog but the restored one — the `BEGIN`
/// catalog's encoding, which the comparison also computes, puts the counter back behind
/// ids already handed out.
#[test]
fn a_rollback_saves_the_counters_it_restored() {
    for (shape, restart) in [
        ("advances in place", false),
        ("advances, then a restart", true),
    ] {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in ROLLED_BACK_SETUP {
            db.execute(sql).unwrap();
        }
        db.execute_as("BEGIN", 0).unwrap();
        db.execute_as("SELECT nextval('s1')", 7).unwrap();
        db.execute_as("SELECT nextval('s1')", 0).unwrap();
        db.execute_as("SELECT nextval('s1')", 0).unwrap();
        if restart {
            db.execute_as("ALTER SEQUENCE s1 RESTART WITH 100", 0)
                .unwrap();
        }
        db.execute_as("CREATE SEQUENCE q", 0).unwrap();
        db.sync().unwrap();
        let mut before = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
        assert!(
            before.execute("SELECT nextval('q')").is_ok(),
            "{shape}: witness, the image before the ROLLBACK holds q"
        );
        db.execute_as("ROLLBACK", 0).unwrap();
        let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
        assert!(
            reopened.execute("SELECT nextval('q')").is_err(),
            "{shape}: the reopen holds no q"
        );
        let next = |db: &mut Database| {
            format!(
                "{:?}",
                db.execute("SELECT nextval('s1')")
                    .map(|r| r[0].rows.clone())
            )
        };
        assert_eq!(
            next(&mut reopened),
            next(&mut db),
            "{shape}: the reopen's counter is the live one"
        );
    }
}

/// A value `nextval` handed out is not handed out again after the database closes and
/// reopens. A block that draws from a sequence — `nextval`, and a `SERIAL` id an `INSERT …
/// RETURNING` hands back — and then ends without committing leaves the counter advanced, as
/// 18.6 does; the rollback saves it, so a reopen after the end returned, and after a clean
/// close, answers the next value, as the live database does. The ends: `ROLLBACK`, a failing
/// statement then `ROLLBACK`, a failing statement then `COMMIT` (18.6's implicit rollback), a
/// `ROLLBACK TO SAVEPOINT` then `ROLLBACK`, `Database::rollback`, and the `Database` dropped
/// with the block open, which rolls it back. Witnessed per cell: the image before the end
/// reopens handing the values out again. That witness is today's mechanism, not the
/// requirement — the image a kill with the block open leaves, the repo's `EDGES.md` row
/// 214 — and a repair making an advance durable before its block ends turns it red; the
/// assertions after it are the property.
///
/// Two polarities beside it. A save since the advance that already holds the counter — here
/// the block's own `sync` — leaves the rollback nothing to write: no meta slot. And the
/// rollback a refused `COMMIT` runs saves no counter, since the device has just refused a
/// write: under a full disk the `COMMIT` answers `WriteRefused` and the store serves on,
/// where a save there would fail it.
#[test]
fn a_value_nextval_handed_out_is_not_handed_out_again_after_a_close() {
    const HANDED: &[&str] = &[
        "SELECT nextval('s1')",
        "INSERT INTO sr (x) VALUES (2) RETURNING id",
    ];
    let setup = |db: &mut Database| {
        for sql in ROLLED_BACK_SETUP {
            db.execute(sql).unwrap();
        }
        db.execute("CREATE TABLE sr (x INT CHECK (x > 0), id SERIAL)")
            .unwrap();
        db.execute("INSERT INTO sr (x) VALUES (1)").unwrap();
    };
    let one = |db: &mut Database, sql: &str| -> i64 {
        match db.execute(sql).unwrap()[0].rows[0][0] {
            pglet_sql::types::Value::Int8(v) => v,
            pglet_sql::types::Value::Int4(v) => i64::from(v),
            ref other => panic!("{sql}: {other:?}"),
        }
    };
    // The next value each reopened image hands out, `nextval('s1')` then the next `sr` id.
    let next = |image: Vec<u8>| {
        let mut db = open_database(Box::new(MemBlockIo::from_bytes(image)));
        (
            one(&mut db, "SELECT nextval('s1')"),
            one(&mut db, "INSERT INTO sr (x) VALUES (3) RETURNING id"),
        )
    };
    for end in [
        "ROLLBACK",
        "a failing statement, then ROLLBACK",
        "a failing statement, then COMMIT",
        "ROLLBACK TO SAVEPOINT, then ROLLBACK",
        "Database::rollback",
        "the database dropped with the block open",
    ] {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        setup(&mut db);
        db.execute("BEGIN").unwrap();
        if end.starts_with("ROLLBACK TO") {
            db.execute("SAVEPOINT a").unwrap();
        }
        let handed = (one(&mut db, HANDED[0]), one(&mut db, HANDED[1]));
        if end.starts_with("a failing statement") {
            assert!(db.execute("SELECT * FROM no_such_table").is_err());
        }
        let before_end = io.snapshot_bytes();
        assert_eq!(
            next(before_end),
            handed,
            "{end}: witness, the image before the end hands the values out again"
        );
        match end {
            "Database::rollback" => db.rollback(0).unwrap(),
            "the database dropped with the block open" => {}
            _ => {
                if end.starts_with("ROLLBACK TO") {
                    db.execute("ROLLBACK TO SAVEPOINT a").unwrap();
                }
                let sql = if end.ends_with("COMMIT") {
                    "COMMIT"
                } else {
                    "ROLLBACK"
                };
                db.execute(sql).unwrap();
            }
        }
        let ended = io.snapshot_bytes();
        drop(db);
        let closed = io.snapshot_bytes();
        if !end.starts_with("the database dropped") {
            assert_eq!(
                next(ended),
                (handed.0 + 1, handed.1 + 1),
                "{end}: a reopen after the end returned"
            );
        }
        assert_eq!(
            next(closed),
            (handed.0 + 1, handed.1 + 1),
            "{end}: a reopen after a clean close"
        );
    }

    let slots = |io: &MemBlockIo| io.snapshot_bytes()[..DATA_REGION_START as usize].to_vec();
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    setup(&mut db);
    db.execute("BEGIN").unwrap();
    for sql in HANDED {
        db.execute(sql).unwrap();
    }
    db.sync().unwrap();
    let before = slots(&io);
    db.execute("ROLLBACK").unwrap();
    assert_eq!(
        slots(&io),
        before,
        "a save already holding the counters: no slot written"
    );

    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    setup(&mut db);
    db.execute("BEGIN").unwrap();
    for sql in HANDED {
        db.execute(sql).unwrap();
    }
    let policy = io.policy();
    policy.arm_fill_disk_after(0);
    let refused = db.execute("COMMIT");
    policy.disarm();
    assert!(
        matches!(refused, Err(pglet_sql::Error::WriteRefused { .. })),
        "a refused COMMIT's rollback saves no counter: {refused:?}"
    );
    db.execute_as("SELECT 1", 7)
        .unwrap_or_else(|e| panic!("the store serves on: {e}"));
}

/// What a database reopened over `image` answers to `sql`: its rows, or its error.
fn next_after_reopen(image: Vec<u8>, sql: &str) -> String {
    let mut db = open_database(Box::new(MemBlockIo::from_bytes(image)));
    format!("{:?}", db.execute(sql).map(|r| r[0].rows.clone()))
}

/// A value drawn from a sequence the block's `BEGIN` did not find is not handed out again
/// after the block rolls back and the database closes. The counter save compares each
/// counter with the durable one of its name, so the sequences it reaches are those a save
/// has written, not those the block opened on: one another connection creates after the
/// block's `BEGIN` and before its first write — `CREATE SEQUENCE`, or a `SERIAL` column's —
/// and one it drops and re-creates under a name `BEGIN` found at a higher counter (the
/// fresh draw `1` reads below the old `3`). Each cell hands out `1` and, after the
/// `ROLLBACK` or the drop with the block open, a reopen answers `2`, as the live database
/// does. Measured before the counter save read the durable counter: every cell reopened at
/// `1`.
#[test]
fn a_value_drawn_from_a_sequence_begin_did_not_find_is_not_handed_out_again() {
    type Cell = (&'static str, &'static [(i32, &'static str)], &'static str);
    const CELLS: [Cell; 3] = [
        (
            "another connection's CREATE SEQUENCE",
            &[(7, "CREATE SEQUENCE q"), (0, "SELECT nextval('q')")],
            "SELECT nextval('q')",
        ),
        (
            "another connection's SERIAL column",
            &[
                (7, "CREATE TABLE w (x INT, id SERIAL)"),
                (0, "INSERT INTO w (x) VALUES (1) RETURNING id"),
            ],
            "INSERT INTO w (x) VALUES (2) RETURNING id",
        ),
        (
            "a sequence another connection re-created under the name",
            &[
                (7, "DROP SEQUENCE s1"),
                (7, "CREATE SEQUENCE s1"),
                (0, "SELECT nextval('s1')"),
            ],
            "SELECT nextval('s1')",
        ),
    ];
    for (cell, block, probe) in CELLS {
        for end in ["ROLLBACK", "the drop with the block open"] {
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for sql in ROLLED_BACK_SETUP {
                db.execute(sql).unwrap();
            }
            db.execute("SELECT nextval('s1')").unwrap();
            db.execute("SELECT nextval('s1')").unwrap();
            db.execute_as("BEGIN", 0).unwrap();
            let mut handed = String::new();
            for (conn, sql) in block {
                let rows = db
                    .execute_as(sql, *conn)
                    .unwrap_or_else(|e| panic!("{cell}: {sql}: {e}"));
                handed = format!("{:?}", rows[0].rows);
            }
            assert!(
                handed == "[[Int8(1)]]" || handed == "[[Int4(1)]]",
                "{cell}: the block drew 1: {handed}"
            );
            if end == "ROLLBACK" {
                db.execute_as("ROLLBACK", 0)
                    .unwrap_or_else(|e| panic!("{cell}: {e}"));
            }
            drop(db);
            let next = next_after_reopen(io.snapshot_bytes(), probe);
            assert!(
                next == "Ok([[Int8(2)]])" || next == "Ok([[Int4(2)]])",
                "{cell}, {end}: a reopen after a clean close answers 2: {next}"
            );
        }
    }
}

/// A block that advances `s1` and also replaces another sequence's cell inside a `DO`
/// statement — its body's `DROP SEQUENCE` and `CREATE SEQUENCE`, or its `TRUNCATE …
/// RESTART IDENTITY` — reopens after its rollback at the old cell's counter: `4` for the
/// replaced sequence, as 18.6 answers after the rollback, and `s1`'s next value. Ends:
/// `ROLLBACK`, an aborted block's `COMMIT`, and the drop with the block open.
///
/// While the block's rollback read none of the `DO`'s replacements it left the replacing
/// cell's lower counter live, and this test guarded the counter save's raise-only rule
/// through that: measured when the save wrote the live catalog, the reopen answered `1`,
/// re-issuing the ids `1`–`3` rows already hold. With the replacements handed to the
/// block (the repo's `EDGES.md` row 213, closed) the live counter is the old cell's, so
/// the rule is no longer reached here: a counter save that writes a live counter below the
/// durable one leaves this test and the rest of the `pglet` integration suite green
/// (measured by mutation).
#[test]
fn a_rollback_over_a_do_replacement_reopens_at_the_old_cell() {
    const REPLACEMENTS: [(&str, &str); 2] = [
        (
            "DO $$ BEGIN DROP SEQUENCE s2; CREATE SEQUENCE s2; END $$",
            "SELECT nextval('s2')",
        ),
        (
            "DO $$ BEGIN TRUNCATE w RESTART IDENTITY; END $$",
            "INSERT INTO w (x) VALUES (9) RETURNING id",
        ),
    ];
    for (replacement, probe) in REPLACEMENTS {
        for end in ["ROLLBACK", "COMMIT of the aborted block", "the drop"] {
            let cell = format!("{replacement}, {end}");
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for sql in [
                "CREATE SEQUENCE s1",
                "CREATE SEQUENCE s2",
                "CREATE TABLE w (x INT, id SERIAL)",
                "INSERT INTO w (x) VALUES (1), (2), (3)",
            ] {
                db.execute(sql).unwrap();
            }
            for _ in 0..3 {
                db.execute("SELECT nextval('s1')").unwrap();
                db.execute("SELECT nextval('s2')").unwrap();
            }
            db.execute("BEGIN").unwrap();
            db.execute("SELECT nextval('s1')").unwrap();
            db.execute(replacement)
                .unwrap_or_else(|e| panic!("{cell}: {e}"));
            match end {
                "ROLLBACK" => {
                    db.execute("ROLLBACK").unwrap();
                }
                "COMMIT of the aborted block" => {
                    assert!(db.execute("SELECT 1 / 0").is_err());
                    db.execute("COMMIT").unwrap();
                }
                _ => {}
            }
            drop(db);
            let image = io.snapshot_bytes();
            let replaced = next_after_reopen(image.clone(), probe);
            assert!(
                replaced == "Ok([[Int8(4)]])" || replaced == "Ok([[Int4(4)]])",
                "{cell}: the replaced sequence reopens at its durable counter: {replaced}"
            );
            assert_eq!(
                next_after_reopen(image, "SELECT nextval('s1')"),
                "Ok([[Int8(5)]])",
                "{cell}: the advanced one at its next value"
            );
        }
    }
}

/// A rollback of a statement that drops a sequence with its owner puts the old counter cell
/// back durably: a reopen after the block ends hands out what 18.6 hands out after the
/// rollback. Over `w` and `k` holding ids `1`–`3`, a block that drops `w` and re-creates
/// it (its new cell drawing `1`, or `10` — above the old one), draws `4` and then drops
/// `w`, draws `4` and then drops `k.id`, or runs the reset idiom `DROP SCHEMA public
/// CASCADE` and re-creates `w`, reopens at `4`, `4`, `5`, `5` and `4`. The ends:
/// `ROLLBACK`, an aborted block's `COMMIT`, a `ROLLBACK TO` a savepoint taken before the
/// block's first statement then `COMMIT`, and the `Database` dropped with the block open;
/// the image is taken after a close, and after a `ROLLBACK` with the database still open
/// (a kill), where the live database's answer is read too. Each cell runs with and without
/// the block's own `sync`, which writes the replaced cell's counter into the durable
/// catalog before the end. Measured with the `Plan::swapped_sequences` owner-drop arm each
/// shape reaches removed — `DROP TABLE`'s for the re-created, landing-above and
/// draw-then-`DROP TABLE` shapes, `DROP COLUMN`'s for the draw-then-`DROP COLUMN` one,
/// `DROP SCHEMA`'s for the reset idiom — each removal reddening only the shapes its arm
/// reaches: every cell of the two draw-then-drop shapes answers one id low and every cell
/// of the landing-above shape `11`, the live answers included; the re-created and
/// reset-idiom shapes answer a colliding id live after the kill's `ROLLBACK`, with or
/// without the `sync`, and on reopen wherever a save followed the drop (the `sync`
/// cells, and both `ROLLBACK TO` cells, whose `COMMIT` saves).
#[test]
fn a_rollback_of_an_owner_drop_puts_back_the_old_cell_durably() {
    type Shape = (&'static str, &'static [&'static str], &'static str, i32);
    const SHAPES: [Shape; 5] = [
        (
            "re-created",
            &[
                "DROP TABLE w",
                "CREATE TABLE w (id SERIAL PRIMARY KEY, x INT)",
                "INSERT INTO w (x) VALUES (9)",
            ],
            "INSERT INTO w (x) VALUES (10) RETURNING id",
            4,
        ),
        (
            "re-created above the old cell",
            &[
                "DROP TABLE w",
                "CREATE TABLE w (id SERIAL PRIMARY KEY, x INT)",
                "INSERT INTO w (x) VALUES (1), (2), (3), (4), (5), (6), (7), (8), (9), (10)",
            ],
            "INSERT INTO w (x) VALUES (10) RETURNING id",
            4,
        ),
        (
            "a draw, then the drop",
            &["INSERT INTO w (x) VALUES (9) RETURNING id", "DROP TABLE w"],
            "INSERT INTO w (x) VALUES (10) RETURNING id",
            5,
        ),
        (
            "a draw, then DROP COLUMN",
            &[
                "INSERT INTO k (x) VALUES (9) RETURNING id",
                "ALTER TABLE k DROP COLUMN id",
            ],
            "INSERT INTO k (x) VALUES (10) RETURNING id",
            5,
        ),
        (
            "the reset idiom",
            &[
                "DROP SCHEMA public CASCADE",
                "CREATE SCHEMA public",
                "CREATE TABLE w (id SERIAL PRIMARY KEY, x INT)",
                "INSERT INTO w (x) VALUES (9)",
            ],
            "INSERT INTO w (x) VALUES (10) RETURNING id",
            4,
        ),
    ];
    const ENDS: [&str; 5] = [
        "ROLLBACK, close",
        "ROLLBACK, kill",
        "COMMIT of the aborted block, close",
        "ROLLBACK TO, COMMIT, close",
        "the drop with the block open",
    ];
    let mut wrong = Vec::new();
    for (shape, block, probe, next) in SHAPES {
        for end in ENDS {
            for sync in [false, true] {
                let cell = format!("{shape}, {end}, sync {sync}");
                let io = MemBlockIo::new();
                let mut db = open_database(Box::new(io.clone()));
                for sql in [
                    "CREATE TABLE w (id SERIAL PRIMARY KEY, x INT)",
                    "INSERT INTO w (x) VALUES (1), (2), (3)",
                    "CREATE TABLE k (x INT, id SERIAL)",
                    "INSERT INTO k (x) VALUES (1), (2), (3)",
                ] {
                    db.execute(sql).unwrap();
                }
                db.execute("BEGIN").unwrap();
                if end.starts_with("ROLLBACK TO") {
                    db.execute("SAVEPOINT sp").unwrap();
                }
                for sql in block {
                    db.execute(sql)
                        .unwrap_or_else(|e| panic!("{cell}: {sql}: {e}"));
                }
                if sync {
                    db.sync().unwrap();
                }
                match end {
                    "ROLLBACK, close" | "ROLLBACK, kill" => {
                        db.execute("ROLLBACK").unwrap();
                    }
                    "COMMIT of the aborted block, close" => {
                        assert!(db.execute("SELECT 1 / 0").is_err());
                        db.execute("COMMIT").unwrap();
                    }
                    "ROLLBACK TO, COMMIT, close" => {
                        db.execute("ROLLBACK TO sp").unwrap();
                        db.execute("COMMIT").unwrap();
                    }
                    _ => {}
                }
                // A kill takes the image with the database still open; a close drops it
                // first, and so does the end that is the drop.
                let image = if end.ends_with("kill") {
                    let image = io.snapshot_bytes();
                    let live = format!("{:?}", db.execute(probe).map(|r| r[0].rows.clone()));
                    if live != format!("Ok([[Int4({next})]])") {
                        wrong.push(format!("{cell}, live: {live}"));
                    }
                    image
                } else {
                    drop(db);
                    io.snapshot_bytes()
                };
                let reopened = next_after_reopen(image, probe);
                if reopened != format!("Ok([[Int4({next})]])") {
                    wrong.push(format!("{cell}: {reopened}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// An owner drop that is refused lists its table's cells all the same — the list is read
/// before the statement runs — and changes no answer: the entry names the live cell, a
/// `ROLLBACK TO` truncates the swap log past it, and the name it leaves among the replaced
/// cells only has that counter compared rather than set aside. Over `w` holding ids `1`–`3`
/// and a foreign key from `wc`, a block that runs `DROP TABLE w` under a savepoint
/// (`2BP01`), rolls back to it, draws `4` and `sync`s, then rolls back: the live database
/// and a reopen after a close or a kill both answer `5`, as 18.6 does (measured, a scratch
/// database). A control: it holds with the owner-drop arms of `Plan::swapped_sequences`
/// removed too.
#[test]
fn a_refused_owner_drop_changes_no_counter_a_rollback_leaves() {
    for kill in [false, true] {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in [
            "CREATE TABLE w (id SERIAL PRIMARY KEY, x INT)",
            "INSERT INTO w (x) VALUES (1), (2), (3)",
            "CREATE TABLE wc (pid INT REFERENCES w (id))",
        ] {
            db.execute(sql).unwrap();
        }
        db.execute("BEGIN").unwrap();
        db.execute("SAVEPOINT r").unwrap();
        let refused = db
            .execute("DROP TABLE w")
            .expect_err("a foreign key depends on w");
        assert_eq!(refused.sqlstate(), "2BP01", "{refused}");
        db.execute("ROLLBACK TO r").unwrap();
        assert_eq!(
            format!(
                "{:?}",
                db.execute("INSERT INTO w (x) VALUES (9) RETURNING id")
                    .unwrap()[0]
                    .rows
            ),
            "[[Int4(4)]]"
        );
        db.sync().unwrap();
        db.execute("ROLLBACK").unwrap();
        let probe = "INSERT INTO w (x) VALUES (10) RETURNING id";
        let image = if kill {
            let image = io.snapshot_bytes();
            assert_eq!(
                format!("{:?}", db.execute(probe).map(|r| r[0].rows.clone())),
                "Ok([[Int4(5)]])",
                "live"
            );
            image
        } else {
            drop(db);
            io.snapshot_bytes()
        };
        assert_eq!(
            next_after_reopen(image, probe),
            "Ok([[Int4(5)]])",
            "kill {kill}"
        );
    }
}

/// A database dropped with a block open rolls the block back rather than saving what the
/// live catalog holds: the block's `CREATE TABLE`, made durable by the block's own `sync`,
/// is gone from a reopen after the drop. Witnessed: the image
/// before the drop reopens holding the table. Red for a close that saves the live
/// catalog (`Database::sync`) in place of the rollback, which keeps the table durable.
#[test]
fn a_database_dropped_with_a_block_open_takes_back_its_ddl_a_save_made_durable() {
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    for sql in ROLLED_BACK_SETUP {
        db.execute(sql).unwrap();
    }
    db.execute_as("BEGIN", 0).unwrap();
    db.execute_as("CREATE TABLE ghost (i INT)", 0).unwrap();
    db.execute_as("SELECT nextval('s1')", 0).unwrap();
    db.sync().unwrap();
    assert_eq!(
        next_after_reopen(io.snapshot_bytes(), "SELECT count(*) FROM ghost"),
        "Ok([[Int8(0)]])",
        "witness: the save during the block made the table durable"
    );
    drop(db);
    let image = io.snapshot_bytes();
    let ghost = next_after_reopen(image.clone(), "SELECT count(*) FROM ghost");
    assert!(
        ghost.starts_with("Err"),
        "the drop took the table back: {ghost}"
    );
    assert_eq!(
        next_after_reopen(image, "SELECT nextval('s1')"),
        "Ok([[Int8(3)]])",
        "and kept the counter the block drew"
    );
}

/// A database dropped while its thread unwinds a panic writes nothing, whatever block it
/// holds open: the catalog may be half-written, as after a kill. The bytes after the
/// unwind are the bytes before the panic. Red for a drop that rolls back and saves during
/// the unwind.
#[test]
fn a_database_dropped_while_its_thread_panics_writes_nothing() {
    let io = MemBlockIo::new();
    let before = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut db = open_database(Box::new(io.clone()));
        for sql in ROLLED_BACK_SETUP {
            db.execute(sql).unwrap();
        }
        db.execute("BEGIN").unwrap();
        db.execute("SELECT nextval('s1')").unwrap();
        *before.lock().unwrap() = io.snapshot_bytes();
        panic!("unwinding with the block open");
    }));
    assert!(unwound.is_err(), "the closure panicked");
    assert_eq!(
        io.snapshot_bytes(),
        *before.lock().unwrap(),
        "the drop during the unwind wrote nothing"
    );
}

/// The setup the tests below share: `e` 7 8 9 behind a committed index `e_k` (so the
/// greatest index band is not the only one), `f` 1 2 3 unindexed, an empty `g` for the
/// other connection's save.
const LEFT_BEHIND_SETUP: [&str; 7] = [
    "CREATE TABLE e (i INT)",
    "INSERT INTO e VALUES (7), (8), (9)",
    "CREATE INDEX e_k ON e (i)",
    "CREATE TABLE f (j INT)",
    "INSERT INTO f VALUES (1), (2), (3)",
    "CREATE TABLE g (k INT)",
    "SELECT 1",
];

/// Whether the tree `db` stands over holds an entry under index number `oid`.
fn tree_holds_index_band(db: &Database, oid: u32) -> bool {
    let pglet::StorageBackend::Btree(btree) = db.storage() else {
        return false;
    };
    let tree = btree.inner();
    !tree
        .range_scan_at_root(
            tree.current_root(),
            &BtreeKey::index_table_start(oid),
            &BtreeKey::index_table_end(oid),
        )
        .unwrap()
        .is_empty()
}

/// An index a block built and dropped, or a committed index a block dropped, leaves its
/// entries in a tree a save writes while the block is open — the build is applied at once
/// and the drop is deferred to `COMMIT` — and a catalog that no longer names it. A reopen
/// seeded its OID counter from the catalog alone, so the next `CREATE INDEX` took that
/// number and served the entries as its own: over `f` holding 1, 2, 3, `j = 7` answered a
/// row with `j` 1 and the range scan `1 1 2 2 3 3` (measured, every cell below). The reopen
/// now seeds the counter past every index number the tree holds an entry under.
///
/// Cells: the block's `CREATE INDEX` then `DROP INDEX`, ended by a kill with the block
/// open, a `ROLLBACK` then a clean close (the rollback sheds the entries only in the live
/// tree, after the save wrote them), and the drop with the block open; and a committed
/// index dropped in the block, ended by a kill. The block's own `sync` is the save. Witnessed per cell: the reopened tree holds entries under the dropped index's
/// number and the catalog names no index by it. The read is asserted first.
#[test]
fn a_new_index_never_reads_the_entries_a_dropped_one_left_in_a_saved_tree() {
    for (cell, committed, block, end) in [
        (
            "built and dropped in the block",
            false,
            &["CREATE INDEX e_i ON e (i)", "DROP INDEX e_i"][..],
            "kill",
        ),
        (
            "built and dropped in the block",
            false,
            &["CREATE INDEX e_i ON e (i)", "DROP INDEX e_i"][..],
            "ROLLBACK, close",
        ),
        (
            "built and dropped in the block",
            false,
            &["CREATE INDEX e_i ON e (i)", "DROP INDEX e_i"][..],
            "drop with the block open",
        ),
        (
            "a committed index dropped in the block",
            true,
            &["DROP INDEX e_i"][..],
            "kill",
        ),
    ] {
        let cell = format!("{cell}, {end}");
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in LEFT_BEHIND_SETUP {
            db.execute(sql).unwrap();
        }
        let number = |db: &mut Database| {
            let rows = db
                .execute_as("SELECT oid::int FROM pg_class WHERE relname = 'e_i'", 0)
                .unwrap();
            match rows[0].rows.first().map(|r| r[0].clone()) {
                Some(pglet_sql::types::Value::Int4(n)) => Some(u32::try_from(n).unwrap()),
                _ => None,
            }
        };
        let mut oid = if committed {
            db.execute("CREATE INDEX e_i ON e (i)").unwrap();
            number(&mut db)
        } else {
            None
        };
        db.execute_as("BEGIN", 0).unwrap();
        for sql in block {
            db.execute_as(sql, 0).unwrap();
            oid = oid.or_else(|| number(&mut db));
        }
        let oid = oid.unwrap_or_else(|| panic!("{cell}: e_i's number"));
        db.sync().unwrap();
        let image = match end {
            "kill" => io.snapshot_bytes(),
            "ROLLBACK, close" => {
                db.execute_as("ROLLBACK", 0).unwrap();
                drop(db);
                io.snapshot_bytes()
            }
            _ => {
                drop(db);
                io.snapshot_bytes()
            }
        };
        let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(image)));
        let named = format!(
            "{:?}",
            reopened
                .execute(&format!("SELECT relname FROM pg_class WHERE oid = {oid}"))
                .unwrap()[0]
                .rows
        );
        reopened.execute("CREATE INDEX f_j ON f (j)").unwrap();
        let read = |db: &mut Database, sql: &str| format!("{:?}", db.execute(sql).unwrap()[0].rows);
        assert_eq!(
            read(&mut reopened, "SELECT j FROM f WHERE j = 7"),
            "[]",
            "{cell}"
        );
        assert_eq!(
            read(&mut reopened, "SELECT j FROM f WHERE j >= 0 ORDER BY j"),
            "[[Int4(1)], [Int4(2)], [Int4(3)]]",
            "{cell}: the new index holds its own rows only"
        );
        assert!(
            tree_holds_index_band(&reopened, oid),
            "{cell}: witness, the entries are in the saved tree"
        );
        assert_eq!(
            named, "[]",
            "{cell}: witness, the saved catalog names no index by their number"
        );
    }
}

/// A table a rollback takes back never hands its id to a later table. Another connection
/// reads an open block's `CREATE TABLE t2` from the catalog and wrote into it; the rollback
/// restored the table-id counter to its `BEGIN` value, so the next `CREATE TABLE` took
/// `t2`'s id and read `t2`'s row — live, and after a reopen (measured: `u` answered `42`).
/// That write is now refused `55P03` — the block holds the writer, and this host cannot
/// make a write wait — so no row lands under the id; the counter also stops above an id the
/// tree holds a row under (`pglet_storage`'s `restore_next_table_id`), which a write the
/// writer did not stop would reach.
///
/// Cells: the block ended by `ROLLBACK`, or by `ROLLBACK TO` a savepoint taken before the `CREATE
/// TABLE` and then `COMMIT`; the next table named `u` or `t2`; its rows read live, after a clean
/// close, and (the `ROLLBACK` end only) on a reopen of the image the rollback left, where the table
/// is created again. Connection 7's autocommit `INSERT` is the refused write. The counter's accept side — a
/// whole restore where no row is written — is `pglet_storage`'s
/// `create_table_pk_in_tx_rollback_no_phantom_under_id_reuse`.
#[test]
fn a_rolled_back_table_never_hands_its_rows_to_a_later_table() {
    for end in ["ROLLBACK", "ROLLBACK TO"] {
        for name in ["u", "t2"] {
            let cell = format!("{end}, then {name}");
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            for sql in LEFT_BEHIND_SETUP {
                db.execute(sql).unwrap();
            }
            db.execute_as("BEGIN", 0).unwrap();
            if end == "ROLLBACK TO" {
                db.execute_as("SAVEPOINT sp", 0).unwrap();
            }
            db.execute_as("CREATE TABLE t2 (i INT)", 0).unwrap();
            let refused = db
                .execute_as("INSERT INTO t2 VALUES (42)", 7)
                .expect_err("the block holds the writer");
            assert_eq!(refused.sqlstate(), "55P03", "{cell}: {refused}");
            if end == "ROLLBACK" {
                db.execute_as("ROLLBACK", 0).unwrap();
            } else {
                db.execute_as("ROLLBACK TO sp", 0).unwrap();
            }
            let rolled_back = io.snapshot_bytes();
            let create = format!("CREATE TABLE {name} (i INT)");
            let select = format!("SELECT i FROM {name}");
            db.execute_as(&create, 0).unwrap();
            if end == "ROLLBACK TO" {
                db.execute_as("COMMIT", 0).unwrap();
            }
            assert_eq!(
                format!("{:?}", db.execute_as(&select, 7).unwrap()[0].rows),
                "[]",
                "{cell}: live"
            );
            drop(db);
            assert_eq!(
                next_after_reopen(io.snapshot_bytes(), &select),
                "Ok([])",
                "{cell}: after a close"
            );
            if end == "ROLLBACK" {
                let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(rolled_back)));
                reopened.execute(&create).unwrap();
                assert_eq!(
                    format!("{:?}", reopened.execute(&select).unwrap()[0].rows),
                    "[]",
                    "{cell}: created after a reopen of the image the rollback left"
                );
            }
        }
    }
}

/// A table a block drops, saved by the block's own `sync` while it is open, leaves its
/// rows and its directory entry in the tree (the drop is deferred to `COMMIT`) under a
/// catalog that no longer names it; after a kill there, a reopen reads no `t` (row 211's
/// uncommitted `DROP`), and a `CREATE TABLE t` was handed the old id and answered the dropped
/// table's row `5` (measured). A reopen now forgets a directory name the catalog lacks, so
/// the new `t` is empty. Witnessed: the image holds the row under the old table.
#[test]
fn a_table_dropped_in_an_open_block_hands_its_rows_to_no_later_table() {
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    for sql in LEFT_BEHIND_SETUP {
        db.execute(sql).unwrap();
    }
    db.execute("CREATE TABLE t (i INT)").unwrap();
    db.execute("INSERT INTO t VALUES (5)").unwrap();
    db.execute_as("BEGIN", 0).unwrap();
    db.execute_as("DROP TABLE t", 0).unwrap();
    db.sync().unwrap();
    let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
    let pglet::StorageBackend::Btree(btree) = reopened.storage() else {
        panic!("a B+tree store");
    };
    let tree = btree.inner();
    let dropped_row_left = tree
        .range_scan_at_root(
            tree.current_root(),
            &BtreeKey::table_start(0),
            &BtreeKey::index_table_start(0),
        )
        .unwrap()
        .iter()
        .any(|(_, row)| *row == [pglet_sql::types::Value::Int4(5)]);
    assert!(
        reopened.execute("SELECT i FROM t").is_err(),
        "the image holds the uncommitted drop"
    );
    reopened.execute("CREATE TABLE t (i INT)").unwrap();
    assert_eq!(
        format!("{:?}", reopened.execute("SELECT i FROM t").unwrap()[0].rows),
        "[]",
        "the new t holds no row of the dropped one"
    );
    assert!(
        dropped_row_left,
        "witness: the dropped table's row is in the tree"
    );
}

/// Whether the tree `db` stands over holds a table row equal to `row`, under any table id.
fn tree_holds_row(db: &Database, row: &[pglet_sql::types::Value]) -> bool {
    let pglet::StorageBackend::Btree(btree) = db.storage() else {
        return false;
    };
    let tree = btree.inner();
    tree.range_scan_at_root(
        tree.current_root(),
        &BtreeKey::table_start(0),
        &BtreeKey::index_table_start(0),
    )
    .unwrap()
    .iter()
    .any(|(_, values)| values == row)
}

/// A block that drops a committed table and creates it again, then rolls back, leaves a
/// reopen the committed table and its rows. When another connection could write into the
/// block's new tables, the rollback kept the table-id counter above each one it wrote, and
/// each id it stayed above was inside the reopen's directory scan, which gives a repeated
/// name to the higher id: the new `t` took the name from the committed one, so after a
/// close `t` answered `42` — the other connection's row — while `1 2 3` were unreachable
/// (measured). That write is now refused `55P03` (the block holds the writer, and this host
/// cannot make a write wait), and the rollback restores the counter whole.
///
/// Cells: connection 7's refused `INSERT` into the new `t`, the block ended by a `ROLLBACK`
/// and a close, by `ROLLBACK TO` a savepoint taken before the `DROP` and a `COMMIT`, and by
/// a `ROLLBACK` and a kill; and the refused insert into a later table `t3` instead, ended by
/// a `ROLLBACK` and a close.
#[test]
fn a_rolled_back_re_creation_leaves_a_reopen_the_committed_table() {
    for (written, end) in [
        ("t", "ROLLBACK, close"),
        ("t", "ROLLBACK TO, COMMIT, close"),
        ("t", "ROLLBACK, kill"),
        ("t3", "ROLLBACK, close"),
    ] {
        let cell = format!("{written} written, {end}");
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in LEFT_BEHIND_SETUP {
            db.execute(sql).unwrap();
        }
        db.execute("CREATE TABLE t (i INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();
        db.execute_as("BEGIN", 0).unwrap();
        if end.starts_with("ROLLBACK TO") {
            db.execute_as("SAVEPOINT sp", 0).unwrap();
        }
        for sql in [
            "DROP TABLE t",
            "CREATE TABLE t (i INT)",
            "CREATE TABLE t3 (i INT)",
        ] {
            db.execute_as(sql, 0).unwrap();
        }
        let refused = db
            .execute_as(&format!("INSERT INTO {written} VALUES (42)"), 7)
            .expect_err("the block holds the writer");
        assert_eq!(refused.sqlstate(), "55P03", "{cell}: {refused}");
        if end.starts_with("ROLLBACK TO") {
            db.execute_as("ROLLBACK TO sp", 0).unwrap();
            db.execute_as("COMMIT", 0).unwrap();
        } else {
            db.execute_as("ROLLBACK", 0).unwrap();
        }
        let committed = "[[Int4(1)], [Int4(2)], [Int4(3)]]";
        assert_eq!(
            format!(
                "{:?}",
                db.execute_as("SELECT i FROM t ORDER BY i", 7).unwrap()[0].rows
            ),
            committed,
            "{cell}: live"
        );
        // A kill takes the image with the database still open; a close drops it first.
        if !end.ends_with("kill") {
            drop(db);
        }
        let image = io.snapshot_bytes();
        let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(image)));
        let read = |db: &mut Database| {
            format!(
                "{:?}",
                db.execute("SELECT i FROM t ORDER BY i").unwrap()[0].rows
            )
        };
        assert_eq!(read(&mut reopened), committed, "{cell}: after a reopen");
        reopened.execute("INSERT INTO t VALUES (4)").unwrap();
        assert_eq!(
            read(&mut reopened),
            "[[Int4(1)], [Int4(2)], [Int4(3)], [Int4(4)]]",
            "{cell}: a new row lands beside the committed ones"
        );
        assert!(
            !tree_holds_row(&reopened, &[pglet_sql::types::Value::Int4(42)]),
            "{cell}: the refused row is nowhere in the tree"
        );
    }
}

/// Every table a rollback takes back keeps its rows from the tables created after it,
/// however many the block created. When another connection could write into them, the
/// table-id counter stopped above the **highest** written id; stopped above the lowest, the
/// next `CREATE TABLE` took the second written table's id and answered its row `43`, live
/// and after a reopen (measured). Those writes are now refused `55P03` — the block holds
/// the writer — so nothing lands under either id. Two tables, two refused writes, then two
/// tables created: each reads nothing.
#[test]
fn every_table_a_rollback_takes_back_keeps_its_rows_from_the_next_tables() {
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    for sql in LEFT_BEHIND_SETUP {
        db.execute(sql).unwrap();
    }
    db.execute_as("BEGIN", 0).unwrap();
    db.execute_as("CREATE TABLE t2 (i INT)", 0).unwrap();
    db.execute_as("CREATE TABLE t3 (i INT)", 0).unwrap();
    for sql in ["INSERT INTO t2 VALUES (42)", "INSERT INTO t3 VALUES (43)"] {
        let refused = db
            .execute_as(sql, 7)
            .expect_err("the block holds the writer");
        assert_eq!(refused.sqlstate(), "55P03", "{sql}: {refused}");
    }
    db.execute_as("ROLLBACK", 0).unwrap();
    db.execute_as("CREATE TABLE u (i INT)", 0).unwrap();
    db.execute_as("CREATE TABLE w (i INT)", 0).unwrap();
    let reads = |db: &mut Database| {
        ["u", "w"].map(|name| {
            format!(
                "{:?}",
                db.execute_as(&format!("SELECT i FROM {name}"), 0).unwrap()[0].rows
            )
        })
    };
    assert_eq!(reads(&mut db), ["[]", "[]"], "live");
    drop(db);
    let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
    assert_eq!(reads(&mut reopened), ["[]", "[]"], "after a reopen");
}

/// A block whose `COMMIT` is refused, when a save during the block may have made its catalog
/// changes durable, is not told the `COMMIT` definitely failed. Every save but a rollback's counter
/// save encodes the live catalog, which holds an open block's DDL, so a save after the block's DDL
/// — the block's own `sync`; not another connection's write, which this host refuses `55P03` while
/// the block holds the writer, as the native server makes it wait — makes that DDL durable; a
/// refused `COMMIT` that then fails the store
/// — its rollback's save refused under a fault that persists, or a block that created a table,
/// which no rollback can drop — leaves it there, the block's row not. Measured before this answer
/// changed: the definite `DurableWriteFailed`, and a reopen holding the sequence, the column or the
/// table — on a healthy block's `COMMIT`, and later on an aborted block's, whose `COMMIT` 18.6
/// converts to a rollback.
///
/// Five shapes × four ends × the faults × where the save falls — another connection's write
/// before the block's first write, the block's own `sync` after it.
/// The shapes: a `CREATE SEQUENCE`, an `ADD COLUMN`, a `CREATE TABLE`, a `CREATE INDEX` —
/// whose rollback writes, shedding the entries its build published — and a sequence the
/// host adds through `Database::schema_mut`, which no statement records. Where the save
/// falls: after the block's DDL; before the block; after the block's first statement, an
/// `INSERT`, and before its DDL — the one that separates the block's first catalog write
/// from its first statement; and with the DDL inside a savepoint a `ROLLBACK TO` undoes,
/// after the save — the DDL durable though the live catalog no longer holds it — or before
/// it, the save then writing a catalog without it. The block then writes the catalog once
/// more, so its commit's save writes a catalog batch. The ends: the healthy block's
/// `COMMIT`, and the `COMMIT` of the block aborted by a failing statement through each door
/// a host converts it at. The faults: a full disk, a refused data write and a refused
/// `fsync`, each persisting; a data write refused once — the healthy block's commit batch,
/// after which its rollback's save lands, or the aborted block's rollback save itself;
/// and, on the healthy end only, the second `fsync` refused, so the commit's batch lands
/// and the save's catalog batch does not. (On the aborted ends the second `fsync` is the
/// rollback save's slot `fsync`, whose refusal answers the unknown outcome, the slot
/// having landed — measured, a reopen holds none of the block — so it has no place in an
/// exactly-when relation.)
///
/// The discriminating assertion comes first, and it is a relation: the answer claims
/// nothing of the block persisted — `DurableWriteFailed`, `WriteRefused` with the store
/// serving, or the aborted block's rollback answered without an error — exactly when the
/// reopen holds none of the block, its DDL or its row. So an answer left definite past
/// the save is red, and so is one made unknown where nothing of the block is durable.
/// Every kind of answer is witnessed.
#[test]
fn a_refused_commit_past_a_save_of_its_ddl_is_not_answered_as_failed() {
    use pglet_sql::Error as E;
    type Arm = (&'static str, fn(&FaultPolicy));
    type Durable = fn(&mut Database) -> bool;
    // A shape's change, as SQL — or, for `HOST_WRITE`, a sequence the host adds through
    // `Database::schema_mut` — and whether a reopen holds it.
    const HOST_WRITE: &str = "a sequence added through schema_mut";
    let shapes: [(&str, &str, Durable); 5] = [
        ("create sequence", "CREATE SEQUENCE q", |db| {
            db.execute("SELECT nextval('q')").is_ok()
        }),
        (
            "add column",
            "ALTER TABLE t ADD COLUMN w INT DEFAULT 4",
            |db| db.execute("SELECT w FROM t").is_ok(),
        ),
        ("create table", "CREATE TABLE u (k INT)", |db| {
            db.execute("SELECT k FROM u").is_ok()
        }),
        // An index build publishes its entries as it runs, so the rollback of it writes
        // even when it saves nothing.
        ("create index", "CREATE INDEX t_id ON t (id)", |db| {
            db.execute("SELECT indexname FROM pg_indexes WHERE indexname = 't_id'")
                .is_ok_and(|r| !r[0].rows.is_empty())
        }),
        (HOST_WRITE, HOST_WRITE, |db| {
            db.execute("SELECT nextval('q')").is_ok()
        }),
    ];
    let arms: [Arm; 5] = [
        ("full disk", |p| p.arm_fill_disk_after(0)),
        ("data write", |p| p.arm_fail_data_writes_after(0)),
        ("every fsync", FaultPolicy::arm_fail_syncs),
        ("one data write", |p| p.arm_fail_one_data_write_after(0)),
        ("the second fsync", |p| p.arm_fail_syncs_after(1)),
    ];
    // Where the other connection's saved write falls. The last two put the DDL inside a
    // savepoint a `ROLLBACK TO` then undoes: after the save, which made it durable, or
    // before it, when the save wrote a catalog without it.
    let saves = [
        "after the DDL",
        "before the block",
        "between the INSERT and the DDL",
        "after the DDL, which a ROLLBACK TO then undid",
        "after a ROLLBACK TO undid the DDL",
    ];
    // The healthy block's `COMMIT`, then the aborted block's through each door a host
    // converts it at: SQL through `execute_as`, SQL through `execute_bound` (the wasm
    // bindings' route), and the embed API's `Database::commit`.
    let ends = [
        "COMMIT",
        "aborted COMMIT",
        "COMMIT, bound",
        "Database::commit",
    ];
    let (mut unknown, mut failed, mut refused, mut rolled_back) = (0, 0, 0, 0);
    for (shape, ddl, durable) in shapes {
        for end in ends {
            let aborted = end != "COMMIT";
            for (fault, arm) in arms {
                if aborted && fault == "the second fsync" {
                    continue;
                }
                for save in saves {
                    let cell = format!(
                        "{shape}, {} block, {end}, {fault}, the save {save}",
                        if aborted { "aborted" } else { "healthy" }
                    );
                    let io = MemBlockIo::new();
                    let mut db = open_database(Box::new(io.clone()));
                    for sql in ROLLED_BACK_SETUP {
                        db.execute(sql).unwrap();
                    }
                    // Before the block's first write, another connection's write is the
                    // save; after it, the block holds the writer, and the save is its own
                    // `sync` — the door another connection's write no longer reaches.
                    let other_saves = |db: &mut Database| {
                        db.execute_as("INSERT INTO e VALUES (3, 'z')", 7).unwrap();
                    };
                    let own_save = |db: &mut Database| db.sync().unwrap();
                    db.execute_as("BEGIN", 0).unwrap();
                    if save == "before the block" {
                        other_saves(&mut db);
                    }
                    db.execute_as("INSERT INTO t VALUES (5, 'q')", 0).unwrap();
                    if save == "between the INSERT and the DDL" {
                        own_save(&mut db);
                    }
                    let savepoint = save.contains("ROLLBACK TO");
                    if savepoint {
                        db.execute_as("SAVEPOINT sp", 0).unwrap();
                    }
                    if ddl == HOST_WRITE {
                        db.schema_mut()
                            .sequences
                            .insert("q".into(), Sequence::new(1));
                    } else {
                        db.execute_as(ddl, 0).unwrap();
                    }
                    if save.starts_with("after the DDL") {
                        own_save(&mut db);
                    }
                    if savepoint {
                        db.execute_as("ROLLBACK TO sp", 0).unwrap();
                    }
                    if save == "after a ROLLBACK TO undid the DDL" {
                        own_save(&mut db);
                    }
                    // A catalog write past the save, so the commit's own save writes a
                    // catalog batch for the second `fsync` to refuse.
                    db.execute_as("CREATE SEQUENCE later", 0).unwrap();
                    if aborted {
                        db.execute_as("SELECT 1 / 0", 0)
                            .expect_err("the block aborts");
                    }
                    let policy = io.policy();
                    arm(&policy);
                    let answer = match end {
                        "Database::commit" => db.commit(0),
                        "COMMIT, bound" => db
                            .execute_bound("COMMIT", 0, |_| Ok::<_, ()>(Vec::new()))
                            .map(|_| ())
                            .map_err(|e| match e {
                                BoundExecError::Engine(e) => e,
                                BoundExecError::Bind(()) => unreachable!("nothing to bind"),
                            }),
                        _ => db.execute_as("COMMIT", 0).map(|_| ()),
                    };
                    policy.disarm();
                    let next = db.execute_as("SELECT count(*) FROM t", 7);
                    drop(db);

                    let mut reopened =
                        open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
                    let ddl_durable = durable(&mut reopened);
                    let row_durable = ids(&mut reopened, 0) != Ok(vec![1]);
                    let claims_none = matches!(
                        answer,
                        Ok(()) | Err(E::DurableWriteFailed { .. } | E::WriteRefused { .. })
                    );
                    assert_eq!(
                        claims_none,
                        !(ddl_durable || row_durable),
                        "{cell}: an answer that claims nothing of the block persisted exactly \
                         when the reopen holds none of it: {answer:?}"
                    );
                    let Err(answer) = answer else {
                        assert!(aborted, "{cell}: a healthy block's COMMIT is refused");
                        rolled_back += 1;
                        assert!(next.is_ok(), "{cell}: the store serves on: {next:?}");
                        continue;
                    };
                    let code = if fault == "full disk" {
                        "53100"
                    } else {
                        "58030"
                    };
                    assert_eq!(answer.sqlstate(), code, "{cell}: {answer}");
                    match answer {
                        E::WriteRefused { .. } => {
                            refused += 1;
                            assert!(next.is_ok(), "{cell}: the store serves on: {next:?}");
                        }
                        E::DurableWriteFailed { .. } | E::DurableWriteUnconfirmed { .. } => {
                            if claims_none {
                                failed += 1;
                            } else {
                                unknown += 1;
                            }
                            let next = next.expect_err("a failed store serves nothing");
                            assert_eq!(next.sqlstate(), "57P02", "{cell}: {next}");
                        }
                        other => panic!("{cell}: {other:?}"),
                    }
                }
            }
        }
    }
    assert!(
        unknown > 0 && failed > 0 && refused > 0 && rolled_back > 0,
        "every answer witnessed: {unknown} unknown, {failed} failed, {refused} refused, \
         {rolled_back} rolled back"
    );
}

/// A healthy `COMMIT` whose save is refused after its catalog batch landed and before its
/// slot answers the definite failure, and a reopen holds none of the block. Once superseded
/// pages have reached the free list, a save writes the retired log's and the free list's
/// chain pages between its catalog batch and its slot, and a refusal there fails the store
/// with the slot unwritten — so nothing of the block is durable, and
/// `DurableWriteUnconfirmed` would be false caution. A block that creates a sequence and
/// commits, over a store churned until its free list holds pages, with each data-write fault
/// armed after each of 0 to 5 successes. Witnessed: in some cell the refusal came past the
/// catalog batch, which moved the catalog the store reports durable.
///
/// Red for a `COMMIT` that reads the durable catalog after its save failed rather than
/// before the save started: the batch has by then replaced it, so it reads as a save of
/// the block's change and the answer turns unknown.
#[test]
fn a_commit_refused_past_its_catalog_batch_answers_the_definite_failure() {
    use pglet_sql::Error as E;
    type Arm = (&'static str, fn(&FaultPolicy, usize));
    let arms: [Arm; 3] = [
        (
            "a data write refused once",
            FaultPolicy::arm_fail_one_data_write_after,
        ),
        (
            "data writes refused",
            FaultPolicy::arm_fail_data_writes_after,
        ),
        ("a full disk", FaultPolicy::arm_fill_disk_after),
    ];
    let mut past_the_batch = 0;
    for (fault, arm) in arms {
        for successes in 0..6 {
            let cell = format!("{fault} after {successes}");
            let io = MemBlockIo::new();
            let mut db = open_database(Box::new(io.clone()));
            db.execute("CREATE TABLE t (id INT, pad TEXT)").unwrap();
            for i in 0..60 {
                db.execute(&format!(
                    "INSERT INTO t VALUES ({i}, '{}')",
                    "x".repeat(300)
                ))
                .unwrap();
            }
            for i in 0..30 {
                db.execute(&format!("UPDATE t SET pad = 'y{i}' WHERE id = {i}"))
                    .unwrap();
            }
            db.execute("DELETE FROM t WHERE id >= 40").unwrap();
            db.execute("BEGIN").unwrap();
            db.execute("CREATE SEQUENCE q").unwrap();
            let durable_before = db.storage().persisted_catalog().unwrap().unwrap();
            let policy = io.policy();
            arm(&policy, successes);
            let answer = db.execute("COMMIT");
            policy.disarm();
            let durable_after = db.storage().persisted_catalog().unwrap().unwrap();
            let Err(refused) = answer else {
                continue;
            };
            let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
            assert!(
                reopened.execute("SELECT nextval('q')").is_err(),
                "{cell}: the reopen holds none of the block"
            );
            assert!(
                matches!(
                    refused,
                    E::DurableWriteFailed { .. } | E::WriteRefused { .. }
                ),
                "{cell}: nothing of the block is durable, and the answer says so: {refused:?}"
            );
            if !std::sync::Arc::ptr_eq(&durable_before, &durable_after) {
                past_the_batch += 1;
            }
        }
    }
    assert!(
        past_the_batch > 0,
        "witness: some refusal came past the catalog batch"
    );
}

/// A read that fails inside the alternate slot's retired-log walk leaves that log unread,
/// so the next read adds its pages: an audit that met one failed read
/// (`DoubleMetaStore::persisted_chain_pages`) answers in full the next time, rather than
/// under-reporting the chain pages that slot names for as long as the store is open.
///
/// The image holds a snapshot across 150 commits, so the alternate slot names a spilled
/// retired log; the failure is the first data-region read after open, which that walk
/// makes. Witnessed: the injected failure fired, and it cut the first answer short.
#[test]
fn a_failed_read_of_the_alternate_retired_log_is_read_again() {
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    db.execute("CREATE TABLE t (id INT PRIMARY KEY, v TEXT)")
        .unwrap();
    for i in 0..600 {
        db.execute(&format!(
            "INSERT INTO t VALUES ({i}, \'{}\')",
            "a".repeat(100)
        ))
        .unwrap();
    }
    db.execute_as("BEGIN", 2).unwrap();
    db.execute_as("SELECT count(*) FROM t", 2).unwrap();
    for r in 0..150 {
        let lo = (r * 37) % 560;
        db.execute(&format!(
            "UPDATE t SET v = \'{}\' WHERE id >= {lo} AND id < {}",
            "b".repeat(100 + r % 7),
            lo + 40
        ))
        .unwrap();
    }
    db.execute_as("COMMIT", 2).unwrap();
    // One save after the snapshot's release, which makes the spilled log the alternate
    // slot's. A block that only read saves nothing at its `COMMIT`, so the save is the
    // host's own.
    db.sync().unwrap();
    drop(db);
    let image = io.snapshot_bytes();
    let (pristine, _) = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image.clone())))
        .expect("the image opens");
    let whole = pristine.persisted_chain_pages();
    drop(pristine);

    let io = MemBlockIo::from_bytes(image);
    let policy = io.policy();
    let (store, _) = DoubleMetaStore::open(Box::new(io)).expect("the image opens");
    policy.arm_fail_data_reads(1);
    let first = store.persisted_chain_pages();
    assert_eq!(
        policy.failing_data_reads_left(),
        0,
        "the injected read failure fired"
    );
    assert_ne!(first, whole, "the failed read cut the first answer short");
    assert_eq!(
        store.persisted_chain_pages(),
        whole,
        "the next read of the alternate log adds every page it names"
    );
}

/// A read served by a store that failed while the read was in flight answers what the
/// durable state held or refuses — never bytes a store reopened over the same file wrote.
///
/// The failed store's reads check the failure before the page cache, and the reopen
/// that follows the failure can rewrite a page a read that passed that check is about to
/// read. Each cell opens the image cold, lets the statement's `skip` earlier data reads
/// pass, and at the next one runs, on the reading thread
/// (`FaultPolicy::arm_before_data_read`): the failure recorded, a second store opened
/// over the same bytes, and commits on it until that page's bytes change. Every cold
/// read of three statements is gated in turn — an index lookup, a primary-key lookup
/// and a scan. Witnessed per cell: the gated read happened and its page was rewritten.
/// Measured with the check after the read removed: the scan's last gated read answered
/// 190 rows of the table's 200.
#[test]
fn a_read_in_flight_when_its_store_fails_never_answers_with_a_reopened_stores_page() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let io = MemBlockIo::new();
    {
        let mut db = open_database(Box::new(io.clone()));
        for sql in [
            "CREATE TABLE t (id INT PRIMARY KEY, v TEXT)",
            "CREATE INDEX t_v ON t (v)",
            "INSERT INTO t VALUES (1, 'a'), (5, 'e')",
            "CREATE TABLE big (id INT PRIMARY KEY, pad TEXT)",
        ] {
            db.execute(sql).unwrap();
        }
        let rows: Vec<String> = (1..=200)
            .map(|k| format!("({k}, '{}')", text(k, 300)))
            .collect();
        db.execute(&format!("INSERT INTO big VALUES {}", rows.join(", ")))
            .unwrap();
    }
    let base = io.snapshot_bytes();
    let answer = |db: &mut Database, sql: &str| -> Result<String, String> {
        db.execute(sql)
            .map(|r| format!("{:?}", r[0].rows))
            .map_err(|e| e.sqlstate().to_string())
    };
    let mut cells = 0;
    for sql in [
        "SELECT id FROM t WHERE v = 'e'",
        "SELECT id, length(pad) FROM big WHERE id = 150",
        "SELECT count(*), sum(id) FROM big",
    ] {
        let expected = answer(
            &mut open_database(Box::new(MemBlockIo::from_bytes(base.clone()))),
            sql,
        );
        assert!(expected.is_ok(), "{sql}: {expected:?}");
        for skip in 0.. {
            let label = format!("{sql}, cold read {skip}");
            let io = MemBlockIo::from_bytes(base.clone());
            let mut old = open_database(Box::new(io.clone()));
            let failure = old.storage().write_failure().expect("a file-format store");
            let rewritten = Arc::new(AtomicBool::new(false));
            let hook_rewritten = Arc::clone(&rewritten);
            let shared = io.reopen();
            let policy = io.policy();
            policy.arm_before_data_read(
                skip,
                Box::new(move |offset| {
                    // What another connection's refused save and the server's reopen do.
                    failure.record("a refused save on another connection".into());
                    let page = |bytes: &[u8]| {
                        let at = usize::try_from(offset).unwrap();
                        bytes.get(at..at + PAGE_SIZE).map(<[u8]>::to_vec)
                    };
                    let before = page(&shared.snapshot_bytes());
                    let mut new = open_database(Box::new(shared.reopen()));
                    new.execute("DROP TABLE big").unwrap();
                    new.execute("CREATE TABLE x (id INT PRIMARY KEY, pad TEXT)")
                        .unwrap();
                    for i in 0..100 {
                        let rows: Vec<String> = (0..20)
                            .map(|j| format!("({}, '{}')", i * 20 + j, text(j, 300)))
                            .collect();
                        new.execute(&format!("INSERT INTO x VALUES {}", rows.join(", ")))
                            .unwrap();
                        if page(&shared.snapshot_bytes()) != before {
                            hook_rewritten.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                }),
            );
            let got = answer(&mut old, sql);
            if policy.before_data_read_armed() {
                // The statement made fewer cold reads than `skip + 1`: every one is done.
                assert!(skip > 0, "{sql}: the statement reads a page cold");
                break;
            }
            cells += 1;
            assert!(
                rewritten.load(Ordering::SeqCst),
                "{label}: the reopened store rewrote the page the read was about to read"
            );
            assert!(
                got == expected || got == Err("57P02".to_string()),
                "{label}: answered {got:?}, where the durable state holds {expected:?}"
            );
        }
    }
    assert!(cells >= 3, "every statement gated at least once: {cells}");
}

/// Every page below a closed store's `next_page_id` is live, free, or a chain page a slot names
/// (free-list or retired-log) after the reopen: a page superseded but not yet collected when the
/// store closed is free on reopen, not leaked.
#[test]
fn a_reopened_store_accounts_for_every_page() {
    // The per-test fixture pattern: create, fill, drop, thirty times — its free list
    // empties and refills, which is where a slot's old chain pages can go unnamed.
    let mut create_fill_drop: Workload = Vec::new();
    for i in 0..30 {
        create_fill_drop.push(("CREATE TABLE x (id INT, v TEXT)".into(), true));
        let rows: Vec<String> = (0..20)
            .map(|k| format!("({k}, '{}')", text(i, 300)))
            .collect();
        create_fill_drop.push((format!("INSERT INTO x VALUES {}", rows.join(", ")), true));
        create_fill_drop.push(("DROP TABLE x".into(), true));
    }
    for (name, workload) in [
        ("ddl", ddl_workload()),
        ("mixed 3", mixed_workload(3, 16)),
        ("mixed 8", mixed_workload(8, 16)),
        ("create, fill, drop", create_fill_drop),
    ] {
        let io = RecordingBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for (sql, _) in &workload {
            db.execute(sql).unwrap_or_else(|e| panic!("{sql:.80}: {e}"));
        }
        drop(db);
        let audit = audit_image(&io.image()).expect("the closed store opens");
        let leaked = audit.leaked();
        assert!(
            leaked.is_empty(),
            "{name}: {} of {} allocated pages are leaked: {:?}",
            leaked.len(),
            audit.metadata.next_page_id,
            &leaked[..leaked.len().min(16)]
        );
    }
}

/// Bytes written by the last 100 of `commits` one-row `UPDATE`s, over a 200-row table,
/// with another connection holding an open block — a snapshot — for the whole run when
/// `idle` is set.
fn commit_bytes(commits: usize, idle: bool) -> u64 {
    use pglet_storage::btree::CountingBlockIo;
    let io = CountingBlockIo::new(MemBlockIo::new());
    let counts = io.handle();
    let mut db = open_database(Box::new(io));
    db.execute("CREATE TABLE t (id INT PRIMARY KEY, v TEXT)")
        .unwrap();
    let rows: Vec<String> = (0..200)
        .map(|i| format!("({i}, '{}')", text(i, 100)))
        .collect();
    db.execute(&format!("INSERT INTO t VALUES {}", rows.join(", ")))
        .unwrap();
    if idle {
        db.execute_as("BEGIN", 2).unwrap();
        db.execute_as("SELECT count(*) FROM t", 2).unwrap();
    }
    for i in 0..commits {
        if i == commits - 100 {
            counts.reset();
        }
        db.execute(&format!("UPDATE t SET v = v WHERE id = {}", i % 200))
            .unwrap();
    }
    counts.stats().bytes_written
}

/// A commit writes what it changed, its slot and the free list, not every page superseded
/// since the oldest open snapshot: with a block held open on another connection for 800
/// commits, the last 100 write no more than half as much again as they do with none
/// held. A store that folds the pages a held snapshot pins into the free list it
/// rewrites every commit writes them all again each time, and grows by them.
#[test]
fn a_held_snapshot_does_not_grow_what_a_commit_writes() {
    let quiet = commit_bytes(800, false);
    let held = commit_bytes(800, true);
    assert!(
        held * 2 <= quiet * 3,
        "the last 100 commits wrote {held} bytes with a snapshot held, {quiet} without"
    );
}

/// The environment variable that turns [`embed_kill_child`] into the child process
/// [`a_killed_embedding_process_loses_no_acknowledged_row`] kills: `<data dir>` then
/// `<statement file>`, a tab between.
const EMBED_CHILD: &str = "PGLET_DURABILITY_EMBED_CHILD";

/// The child side of the kill test: open the database under the named directory with
/// `Database::open` and run each statement of the named file (statements separated by
/// a line holding `;;`), printing `start <i>` before and `ack <i>` after each. Does
/// nothing unless [`EMBED_CHILD`] is set.
#[test]
fn embed_kill_child() {
    use std::io::Write;
    let Ok(spec) = std::env::var(EMBED_CHILD) else {
        return;
    };
    let (dir, file) = spec.split_once('\t').expect("dir<TAB>file");
    let statements = std::fs::read_to_string(file).expect("statement file");
    let mut db = Database::open(std::path::Path::new(dir)).expect("open");
    let mut out = std::io::stdout();
    for (i, sql) in statements.split("\n;;\n").enumerate() {
        writeln!(out, "start {i}").unwrap();
        out.flush().unwrap();
        db.execute(sql).unwrap_or_else(|e| panic!("{sql:.60}: {e}"));
        writeln!(out, "ack {i}").unwrap();
        out.flush().unwrap();
    }
}

/// Run `statements` in a child process over `dir` and time the last one, from its
/// `start` line to its `ack`; with `kill` set, `SIGKILL` the child that long after the
/// last statement starts instead.
fn run_child(
    dir: &std::path::Path,
    statements: &[String],
    kill: Option<std::time::Duration>,
) -> std::time::Duration {
    use std::io::BufRead;
    let file = dir.join("statements.sql");
    std::fs::write(&file, statements.join("\n;;\n")).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "durability::embed_kill_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(
            EMBED_CHILD,
            format!("{}\t{}", dir.display(), file.display()),
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let last = statements.len() - 1;
    let (start, ack) = (format!("start {last}"), format!("ack {last}"));
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
    for line in lines.by_ref() {
        if line.unwrap() == start {
            break;
        }
    }
    let t0 = std::time::Instant::now();
    let mut ran = std::time::Duration::ZERO;
    if let Some(delay) = kill {
        while t0.elapsed() < delay {
            std::hint::spin_loop();
        }
        child.kill().unwrap();
    }
    for line in lines.by_ref() {
        if line.unwrap() == ack {
            ran = t0.elapsed();
        }
    }
    let status = child.wait().unwrap();
    assert!(
        kill.is_some() || status.success(),
        "the child failed: {status}"
    );
    ran
}

/// A real embedding process — `Database::open` over a file — killed with `SIGKILL`
/// inside a `CREATE TABLE` that runs after a large `DROP` in the same process, at
/// delays swept across the statement's own duration: every reopen adopts the state
/// the `DROP` acknowledged or the one the `CREATE` did.
#[test]
fn a_killed_embedding_process_loses_no_acknowledged_row() {
    let scratch = pglet_testing::db::ScratchDir::new("embed_kill");
    let dir = |name: &str| {
        let d = scratch.path.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    };
    // `t` and its forty rows, then `big0` and its twelve long rows.
    let base = dir("base");
    let setup: Vec<String> = ddl_workload()[..15]
        .iter()
        .map(|(s, _)| s.clone())
        .collect();
    run_child(&base, &setup, None);
    let copy_base = |to: &std::path::Path| {
        std::fs::copy(base.join("data.pglet"), to.join("data.pglet")).unwrap();
    };
    let state = |d: &std::path::Path| match Database::open(d) {
        Ok(mut db) => contents(&mut db),
        Err(e) => format!("open refused: {e}"),
    };
    let target = vec![
        "DROP TABLE big0".to_string(),
        "CREATE TABLE big1 (id INT, v TEXT)".to_string(),
    ];
    let dropped = dir("dropped");
    copy_base(&dropped);
    run_child(&dropped, &target[..1], None);
    let created = dir("created");
    copy_base(&created);
    // How long the `CREATE` runs uninterrupted, to sweep the kill across it.
    let span = run_child(&created, &target, None);
    let allowed = [state(&dropped), state(&created)];
    let attempts = 60u32;
    let mut wrong = Vec::new();
    for k in 0..attempts {
        let att = dir(&format!("att{k}"));
        copy_base(&att);
        let delay = span * k / attempts;
        run_child(&att, &target, Some(delay));
        let got = state(&att);
        if !allowed.contains(&got) {
            wrong.push(format!("delay {delay:?}: {got:.160}"));
        }
        let _ = std::fs::remove_dir_all(&att);
    }
    assert!(
        wrong.is_empty(),
        "{} of {attempts} kills adopted no acknowledged state (statement span {span:?}): {wrong:#?}",
        wrong.len()
    );
}

/// The row a failing `INSERT` writes, and how its batch is made to fail.
type Arm = (u64, fn(&FaultPolicy));

/// A batch refused after allocating page ids — at a page write, or at its data `fsync`
/// — hands them back: the statement is an ordinary error that leaves the store serving
/// (nothing had published), and after a later commit and a close every allocated page
/// is live, free or a chain page — the ids the refused batch took among them.
#[test]
fn a_failed_batch_leaks_no_page() {
    let arms: [Arm; 2] = [
        (2, FaultPolicy::arm_fail_syncs),
        (3, |p| p.arm_fail_data_writes_after(1)),
    ];
    for (id, arm) in arms {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT, v TEXT)").unwrap();
        db.execute(&format!("INSERT INTO t VALUES (1, '{}')", text(1, 20_000)))
            .unwrap();
        let policy = io.policy();
        arm(&policy);
        let failed = db.execute(&format!(
            "INSERT INTO t VALUES ({id}, '{}')",
            text(id, 30_000)
        ));
        policy.disarm();
        let failed = failed.expect_err("the injected failure is reported");
        assert!(
            matches!(failed, pglet_sql::Error::WriteRefused { .. }),
            "row {id}: nothing had published, so the store serves on: {failed:?}"
        );
        db.execute("INSERT INTO t VALUES (4, 'short')")
            .unwrap_or_else(|e| panic!("row {id}: the next commit: {e}"));
        drop(db);
        let image = io.snapshot_bytes();
        let audit = audit_image(&image).expect("the store opens");
        let leaked = audit.leaked();
        assert!(
            leaked.is_empty(),
            "row {id}: {} of {} allocated pages are leaked: {leaked:?}",
            leaked.len(),
            audit.metadata.next_page_id
        );
        let mut reopened = open_database(Box::new(MemBlockIo::from_bytes(image)));
        assert_eq!(
            ids(&mut reopened, 0),
            Ok(vec![1, 4]),
            "row {id}: the reopen"
        );
    }
}
