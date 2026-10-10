//! Real crash tests for the no-WAL `DoubleMetaStore`, driven through the full
//! engine (`Database` over `BtreeStorageWrapper` over `DoubleMetaStore`).
//!
//! Genuine mid-commit fault injection (not after-the-fact corruption of a
//! finished image): the doubles come from [`pglet_testing::block_io`] —
//! [`MemBlockIo`] over a drop-surviving `Arc<Mutex<Vec<u8>>>`, with an armable
//! [`FaultPolicy`](pglet_testing::block_io::FaultPolicy). We drive a real
//! auto-commit to a controlled mid-commit point, drop the store (abandoning
//! whatever the policy let through), reopen a fresh `Database` over the *same
//! bytes* ([`MemBlockIo::reopen`]), and assert the recovery invariant.
//!
//! The commit byte-sequence and why the policy gates on the write *offset* (not a
//! sync count) are documented on [`pglet_testing::block_io`]. The four hand-picked
//! tests target the known fault windows; the [`random_corruption_recovers_to_prefix_or_fails_loud`]
//! fuzz generalizes them across the whole image.

use std::panic::{AssertUnwindSafe, catch_unwind};

mod common;

use common::open_database;
use pglet_sql::types::Value;
use pglet_storage::btree::{DoubleMetaStore, PAGE_SIZE, PageStore};
use pglet_testing::block_io::{DATA_REGION_START, MemBlockIo};
// The recover-or-loud invariant lives in `pglet_testing` so this deterministic
// sweep and the coverage-guided fuzz target (`fuzz/`) share one definition.
use pglet_testing::durability::{build_pristine_image, is_valid_prefix, surviving_ids};

/// Sanity check: `pglet_testing`'s `DATA_REGION_START` matches the engine's actual
/// data-region boundary, so the slot-vs-data fault gate stays correct if the
/// layout ever changes. A freshly opened store reserves exactly the meta region,
/// so its length before any data page is written is `DATA_REGION_START`.
#[test]
fn data_region_start_matches_layout() {
    let io = MemBlockIo::new();
    let (store, _meta) = DoubleMetaStore::open(Box::new(io)).expect("open empty double-meta store");
    // `page_count` is `(len - DATA_REGION_START) / PAGE_SIZE`; an empty store has
    // zero data pages, so `len == DATA_REGION_START` exactly.
    assert_eq!(
        store.page_count(),
        0,
        "a fresh store has no data pages, so len == DATA_REGION_START"
    );
}

/// (c) Positive control: a clean, fully-acknowledged commit survives reopen.
///
/// Fault model: none. Invariant: with no fault injected, every acked row is
/// durable across a drop+reopen. This is the baseline that makes (a) and (b)
/// meaningful — it proves the harness reopens correctly when nothing is torn,
/// so a *missing* row in (a)/(b) is the injected fault, not a harness artifact.
#[test]
fn clean_acked_commit_survives_reopen() {
    let io = MemBlockIo::new();

    {
        // `clone()` shares the same buffer (and policy) with `io`.
        let mut db = open_database(Box::new(io.clone()));
        // Pure auto-commit (no explicit transaction): each statement commits.
        db.execute("CREATE TABLE t (id INT)").unwrap();
        db.execute("INSERT INTO t VALUES (10)").unwrap();
        db.execute("INSERT INTO t VALUES (20)").unwrap();
    }

    let mut db = open_database(Box::new(io.reopen()));
    let result = db.execute("SELECT * FROM t ORDER BY id").unwrap();
    let rows = &result[0].rows;
    assert_eq!(rows.len(), 2, "both cleanly-acked rows must survive reopen");
    assert_eq!(rows[0][0], Value::Int4(10));
    assert_eq!(rows[1][0], Value::Int4(20));
}

/// (a) Crash after the data fsync but before the meta flip → reopen adopts the
/// OLD slot, and the un-acked write is gone.
///
/// Fault model: row 1 commits cleanly (meta version 1). We then arm
/// `DropSlotWrites` and attempt a second auto-commit INSERT (row 2): its data
/// pages are written + fsynced, but the alternate-slot meta write and its fsync
/// are dropped — exactly the window between fsync #1 and the durable meta flip.
/// The active-slot flip is in-memory only and dies with the dropped store.
///
/// Invariant proven: recovery adopts the last *durable* meta slot (version 1),
/// the never-acknowledged row 2 rolls back cleanly, and the store reopens
/// consistent with no panic. CoW guarantees row 1's pages were never
/// overwritten, so the old snapshot is intact.
#[test]
fn crash_before_meta_flip_adopts_old_slot() {
    let io = MemBlockIo::new();
    let policy = io.policy();

    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT)").unwrap();
        // Row 1 commits cleanly and durably (meta version advances).
        db.execute("INSERT INTO t VALUES (1)").unwrap();

        // Crash window: the meta write for the next commit (and its fsync)
        // never reach disk.
        policy.arm_drop_slot_writes();
        // Row 2's data pages + data fsync happen, but the meta flip is lost.
        // The INSERT still returns Ok — the engine has no way to know the meta
        // write was silently dropped, which is precisely the crash being
        // modeled.
        db.execute("INSERT INTO t VALUES (2)").unwrap();
        // Drop the store: the in-memory active-slot flip is abandoned.
    }

    // Reopen over the same bytes; recovery must pick the last durable slot.
    let mut db = open_database(Box::new(io.reopen()));
    let result = db.execute("SELECT * FROM t ORDER BY id").unwrap();
    let rows = &result[0].rows;
    assert_eq!(
        rows.len(),
        1,
        "only the acked row 1 survives; the un-acked row 2 rolled back"
    );
    assert_eq!(rows[0][0], Value::Int4(1), "row 1 is the recovered state");
}

/// (b) Torn/partial alternate-meta write → CRC rejects the torn slot → reopen
/// falls back to the prior committed slot.
///
/// Fault model: row 1 commits cleanly (meta version 1). We then arm a one-shot
/// torn write that lets only a prefix of the *next* meta-slot write land
/// (header + part of the payload), then returns `Ok`. The slot's CRC covers
/// `version | payload_len | payload`, so a truncated payload fails validation.
///
/// Invariant proven: `read_slot` rejects the torn slot (`Ok(None)`), recovery
/// falls back to the prior valid slot (version 1), and the store reopens with
/// row 1 present and row 2 absent — no panic, no garbage decode.
#[test]
fn torn_alternate_meta_write_falls_back() {
    let io = MemBlockIo::new();
    let policy = io.policy();

    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT)").unwrap();
        // Row 1 commits cleanly and durably.
        db.execute("INSERT INTO t VALUES (100)").unwrap();

        // Tear the next meta-slot write to a 28-byte prefix: the 20-byte header
        // (magic + version + payload_len + crc) plus 8 bytes of payload. The
        // declared payload_len is intact but the actual payload is truncated, so
        // the CRC over the (now-missing) full payload fails → slot rejected.
        policy.arm_tear_next_slot_write(28);
        db.execute("INSERT INTO t VALUES (200)").unwrap();
    }

    let mut db = open_database(Box::new(io.reopen()));
    let result = db.execute("SELECT * FROM t ORDER BY id").unwrap();
    let rows = &result[0].rows;
    assert_eq!(
        rows.len(),
        1,
        "torn slot rejected by CRC; recovery falls back to version 1"
    );
    assert_eq!(rows[0][0], Value::Int4(100), "row 1 is the fallback state");
}

/// (d) Torn data page → fails loud end-to-end (page CRC mismatch surfaces as a
/// query error, not wrong data).
///
/// Fault model: commit rows cleanly, then flip a body byte of a data page in the
/// shared buffer — away from the 28..32 checksum field and at/above
/// `DATA_REGION_START`. On reopen the page's recomputed checksum no longer
/// matches its stamped value, so `get` returns `None` → the row's page is
/// "not found" → the `SELECT` errors instead of returning corrupted rows.
///
/// This is the end-to-end companion to the store-level
/// `get_rejects_corrupted_data_page` unit test: it proves the loud-failure
/// propagates all the way out through the query path.
#[test]
fn torn_data_page_fails_loud() {
    let io = MemBlockIo::new();

    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT, name TEXT)").unwrap();
        db.execute("INSERT INTO t VALUES (1, 'alice')").unwrap();
        db.execute("INSERT INTO t VALUES (2, 'bob')").unwrap();
    }

    // Corrupt the body of the *root* data page — the one the recovered state
    // actually reads. Guessing a page id is unreliable: CoW leaves superseded
    // pages in the file that the current root never references, so corrupting an
    // arbitrary page may go unnoticed. Read the recovered root from a throwaway
    // open, then flip a body byte of that page (offset 100 into it — past the
    // 28..32 checksum field, inside the page body).
    let root = {
        let (_store, metadata) =
            DoubleMetaStore::open(Box::new(io.reopen())).expect("read recovered root");
        metadata.root
    };
    io.with_bytes_mut(|data| {
        let page_offset = DATA_REGION_START as usize + root as usize * PAGE_SIZE;
        let body_byte = page_offset + 100;
        assert!(
            body_byte < data.len(),
            "root page must be present in the data region"
        );
        // Stay clear of the checksum field (page_offset+28..+32) and inside the
        // page body.
        assert!(body_byte >= page_offset + 32);
        assert!(body_byte < page_offset + PAGE_SIZE);
        data[body_byte] ^= 0xFF;
    });

    // Loud, never wrong data — at one of two points now. Since PGM5, recovery
    // scans the tree at open to rebuild the catalog/table directory, so a torn page
    // in the recovered root is caught at OPEN (`recover`'s scan reads it) →
    // `open_database_checked` returns `None`. If the torn page is off the recovery
    // scan path and open still succeeds, the SELECT must fail loud instead.
    match pglet_testing::db::open_database_checked(Box::new(io.reopen())) {
        None => { /* loud at open (recover hit the torn page) — correct */ }
        Some(mut db) => {
            let result = db.execute("SELECT * FROM t ORDER BY id");
            assert!(
                result.is_err(),
                "a torn data page must fail the query loud (page CRC mismatch), \
                 not return wrong data; got {result:?}"
            );
        }
    }
}

/// A commit whose row spans a multi-page overflow chain, failed at each of its
/// data-page writes in turn, leaves the store at the state before it: the chain's
/// pages and the leaf that references them are written in no fixed order, but all of
/// them before the data fsync and the meta flip after it, so a failure at any page
/// write publishes nothing. Each reopen reads the
/// prior row whole and no trace of the failed one, and the store takes the same row
/// on the next try. The sweep runs past the last write, where the commit lands.
#[test]
fn a_chain_commit_failed_at_each_page_write_recovers_to_the_prior_state() {
    let long = |seed: u32| -> String {
        (0..30_000u32)
            .map(|i| char::from(b'a' + ((i.wrapping_mul(seed) >> 3) % 26) as u8))
            .collect()
    };
    let first = long(7919);
    let second = long(104_729);
    let base = {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT, body TEXT)").unwrap();
        db.execute(&format!("INSERT INTO t VALUES (1, '{first}')"))
            .unwrap();
        drop(db);
        io.snapshot_bytes()
    };
    let read = |io: &MemBlockIo| {
        let mut db = open_database(Box::new(io.reopen()));
        db.execute("SELECT id, body FROM t ORDER BY id")
            .unwrap()
            .remove(0)
            .rows
    };
    let mut failed_at = 0;
    for successes in 0.. {
        let io = MemBlockIo::from_bytes(base.clone());
        let policy = io.policy();
        let mut db = open_database(Box::new(io.clone()));
        policy.arm_fail_data_writes_after(successes);
        let result = db.execute(&format!("INSERT INTO t VALUES (2, '{second}')"));
        policy.disarm();
        drop(db);
        if result.is_ok() {
            assert_eq!(
                read(&io),
                vec![
                    vec![Value::Int4(1), Value::Text(first)],
                    vec![Value::Int4(2), Value::Text(second)],
                ]
            );
            break;
        }
        failed_at += 1;
        assert_eq!(
            read(&io),
            vec![vec![Value::Int4(1), Value::Text(first.clone())]],
            "a commit failed after {successes} page writes must leave the prior state"
        );
        // The store still takes the row.
        let mut db = open_database(Box::new(io.reopen()));
        db.execute(&format!("INSERT INTO t VALUES (2, '{second}')"))
            .unwrap();
        drop(db);
        assert_eq!(read(&io).len(), 2);
    }
    // 30 000 characters is eight chain pages, plus the one leaf the table holds; the
    // leaf it superseded is logged in the meta slot itself, which is no data-page write.
    assert_eq!(failed_at, 9, "the sweep must cross the whole chain");
}

/// Every shape of a damaged overflow chain, written into a file image **behind valid
/// page checksums** — so the damage reaches the chain walk rather than failing a
/// page's checksum first — is refused as a corrupt chain naming where the chain
/// stopped agreeing with its reference, at each door this drives: a row's chain on
/// the read, the `DELETE` and the `UPDATE`; the catalog's chain at open. No shape
/// aborts, panics, or walks without end — the one-page loops included, one of them
/// under a reference claiming a terabyte.
#[test]
fn a_chain_damaged_behind_valid_checksums_is_refused_at_every_door() {
    use pglet_storage::btree::{BtreeStorageWrapper, CowBtree, SysKind};
    use pglet_testing::store_image::{ChainDamage, corrupt_chain_message, reachable_chains};

    let base = {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        // A catalog longer than a leaf keeps inline, so it is stored in a chain.
        let columns: Vec<String> = (0..40)
            .map(|i| format!("a_rather_long_column_name_{i:02} TEXT"))
            .collect();
        db.execute(&format!("CREATE TABLE wide ({})", columns.join(", ")))
            .unwrap();
        db.execute("CREATE TABLE t (id INT, body TEXT)").unwrap();
        db.execute(&format!(
            "INSERT INTO t VALUES (1, '{}')",
            "q".repeat(10_000)
        ))
        .unwrap();
        db.execute("INSERT INTO t VALUES (2, 'short')").unwrap();
        drop(db);
        io.snapshot_bytes()
    };
    let (next_page_id, chains) = reachable_chains(&base);
    let row = chains
        .iter()
        .find(|c| c.key.table_id().is_some())
        .expect("the long row's chain");
    let catalog = chains
        .iter()
        .find(|c| matches!(c.key.as_system(), Some((SysKind::Catalog, _))))
        .expect("the catalog's chain");
    assert!(row.links.len() >= 3, "{row:?}");

    let recover = |image: &[u8]| {
        let (store, meta) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image.to_vec()))).unwrap();
        let btree = CowBtree::with_store(Box::new(store), meta.root, meta.version, 100);
        BtreeStorageWrapper::recover(std::sync::Arc::new(btree), meta.next_table_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let mut applied = 0;
    for shape in ChainDamage::ALL {
        for chain in [row, catalog] {
            let mut image = base.clone();
            let Some(page) = shape.apply(&mut image, chain, next_page_id) else {
                continue;
            };
            applied += 1;
            let refusal = corrupt_chain_message(chain.reference.head, page);
            let refused = |result: Result<(), String>| matches!(&result, Err(message) if message.contains(&refusal));
            if chain.key.table_id().is_none() {
                let opened = recover(&image);
                assert!(refused(opened.clone()), "{shape:?} at open: {opened:?}");
                assert!(
                    pglet_testing::db::open_database_checked(Box::new(MemBlockIo::from_bytes(
                        image
                    )))
                    .is_none(),
                    "{shape:?}"
                );
                continue;
            }
            assert_eq!(recover(&image), Ok(()), "{shape:?}");
            let mut db =
                pglet_testing::db::open_database_checked(Box::new(MemBlockIo::from_bytes(image)))
                    .expect("the catalog is whole");
            for sql in [
                "SELECT id, length(body) FROM t",
                "DELETE FROM t WHERE id = 1",
                "UPDATE t SET body = 'x' WHERE id = 1",
            ] {
                let result = db.execute(sql).map(|_| ()).map_err(|e| e.to_string());
                assert!(refused(result.clone()), "{shape:?}: {sql}: {result:?}");
            }
        }
    }
    // Every shape on the row's chain; the catalog's single link takes the shapes
    // that need no second link.
    assert_eq!(applied, 2 * ChainDamage::ALL.len() - 3);
}

// ============================================================================
// Randomized corruption fuzz
//
// The hand-picked tests above target the three known fault windows (drop slot,
// torn slot, torn page). This fuzz generalizes them: take a clean store image,
// apply random byte flips / runs / truncations across *both* the meta and data
// regions, reopen, and assert the one invariant that must hold for arbitrary
// corruption — **recover to a consistent committed prefix, or fail loud; never
// return a torn/garbage row set.**
//
// The image is built by inserting ids 1..=N, each in its own auto-commit, so
// every committed state is exactly the prefix {1, …, j}. A correct recovery
// therefore yields some prefix {1, …, j} (in practice the latest one or two
// intact meta slots); a *bug* would surface as a successful open returning a
// non-prefix set (a gap, a duplicate, an out-of-range id, or a non-Int4 value).
// Open/scan failures (engine errors or the test helper's `.expect` panics on an
// unreadable store) are the acceptable "fail loud" outcome.
//
// Hand-rolled xorshift PRNG, seeded from `PGLET_FUZZ_SEED` (printed at startup)
// — reproducible, no `rand`/`proptest` dependency. `PGLET_FUZZ_ITERS` controls
// the iteration count.
// ============================================================================

/// Seedable xorshift64 — keeps the fuzz reproducible with no dependency.
struct Xorshift(u64);

impl Xorshift {
    const fn new(seed: u64) -> Self {
        Self(seed | 1) // xorshift requires nonzero state
    }

    const fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    const fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Apply one random corruption in place, biased to hit both regions: roughly
/// half the time it targets the data region (offsets ≥ `DATA_REGION_START`),
/// otherwise anywhere (which, given the meta region dwarfs the data region, is
/// mostly a meta-slot corruption).
fn corrupt_once(bytes: &mut Vec<u8>, rng: &mut Xorshift) {
    if bytes.is_empty() {
        return;
    }
    let len = bytes.len() as u64;
    let target_data = len > DATA_REGION_START && rng.below(2) == 0;
    let (lo, hi) = if target_data {
        (DATA_REGION_START, len)
    } else {
        (0, len)
    };
    let pick = |rng: &mut Xorshift| (lo + rng.below(hi - lo)) as usize;

    match rng.below(4) {
        // Single-bit flip.
        0 => {
            let off = pick(rng);
            bytes[off] ^= 1 << (rng.below(8) as u32);
        }
        // Overwrite a run with random bytes.
        1 => {
            let off = pick(rng);
            let run = 1 + rng.below(64) as usize;
            for b in bytes.iter_mut().skip(off).take(run) {
                *b = rng.below(256) as u8;
            }
        }
        // Zero a run (models a dropped/short write).
        2 => {
            let off = pick(rng);
            let run = 1 + rng.below(128) as usize;
            for b in bytes.iter_mut().skip(off).take(run) {
                *b = 0;
            }
        }
        // Truncate the tail (models a torn final write / short file).
        _ => {
            let new_len = rng.below(len).max(1) as usize;
            bytes.truncate(new_len);
        }
    }
}

/// Fuzz: random corruption never yields a torn read — only a valid prefix or a
/// loud failure.
#[test]
fn random_corruption_recovers_to_prefix_or_fails_loud() {
    const N_ROWS: i32 = pglet_testing::durability::PRISTINE_ROWS;
    let seed = env_u64("PGLET_FUZZ_SEED", 0x5DEE_CE66_D9C2_8B1F);
    let iters = env_u64("PGLET_FUZZ_ITERS", 256);
    eprintln!("[fuzz] seed={seed} iters={iters} (override via PGLET_FUZZ_SEED / PGLET_FUZZ_ITERS)");

    let pristine = build_pristine_image(N_ROWS);
    let mut rng = Xorshift::new(seed);

    // Silence the expected panics from caught unreadable-store opens; restore
    // before the final assertion so a real failure prints normally.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut recovered = 0u64; // opened to a valid prefix
    let mut loud = 0u64; // failed loud (acceptable)
    let mut violation: Option<(u64, Vec<i32>)> = None;

    for iter in 0..iters {
        let mut image = pristine.clone();
        let n_corruptions = 1 + rng.below(3); // 1..=3 corruptions per image
        for _ in 0..n_corruptions {
            corrupt_once(&mut image, &mut rng);
        }
        // `surviving_ids` doesn't catch panics (the fuzz target wants them to
        // surface); here a decode panic on a corrupt image is an acceptable "loud"
        // outcome, so catch it → `None` under the silenced hook above.
        let ids = catch_unwind(AssertUnwindSafe(|| surviving_ids(image)))
            .ok()
            .flatten();
        match ids {
            None => loud += 1,
            Some(ids) if is_valid_prefix(&ids, N_ROWS) => recovered += 1,
            Some(ids) => {
                violation = Some((iter, ids));
                break;
            }
        }
    }

    std::panic::set_hook(prev_hook);

    if let Some((iter, ids)) = violation {
        panic!(
            "torn read after corruption (seed={seed}, iter={iter}): SELECT returned a \
             non-prefix set {ids:?} (expected a prefix of 1..={N_ROWS} or a loud failure). \
             Reproduce with PGLET_FUZZ_SEED={seed}."
        );
    }

    eprintln!("[fuzz] {recovered} recovered to a valid prefix, {loud} failed loud, 0 torn reads");
    // Sanity: the run must actually exercise both outcomes, else the corruption
    // generator or the harness is degenerate (e.g. all no-op corruptions).
    assert!(
        recovered > 0,
        "fuzz exercised no clean recoveries — generator too destructive?"
    );
    assert!(
        loud > 0,
        "fuzz exercised no loud failures — corruption not biting?"
    );
}

/// The corruption sweep again, **behind valid checksums**: random damage to the leaf
/// and overflow pages of the pristine image — a page's `count` and `next` fields half
/// the time, its body otherwise — with each such page's checksum re-stamped
/// afterwards, so the damage reaches the leaf codec, the leaf entries' chain
/// references and the chain walk rather than failing a checksum first. With the
/// checksum out of the way a changed value can read back as a different one, so the
/// prefix invariant is not asked; what is asked is that every image opens and scans
/// or fails with an error — no panic (none is caught here), no abort, no walk
/// without end — and that the sweep reaches the chain walk at all.
#[test]
fn random_damage_behind_valid_checksums_is_an_error_never_a_panic() {
    use pglet_testing::store_image::{
        is_leaf_or_overflow, page_ids, page_range, restamp_leaf_and_overflow_pages,
    };

    let seed = env_u64("PGLET_FUZZ_SEED", 0x5DEE_CE66_D9C2_8B1F);
    let iters = env_u64("PGLET_FUZZ_ITERS", 256);
    let pristine = build_pristine_image(pglet_testing::durability::PRISTINE_ROWS);
    let targets: Vec<u64> = page_ids(&pristine)
        .filter(|&id| is_leaf_or_overflow(&pristine, id))
        .collect();
    let mut rng = Xorshift::new(seed);
    let (mut scanned, mut chain_refusals, mut other_errors) = (0u64, 0u64, 0u64);
    for _ in 0..iters {
        let mut image = pristine.clone();
        for _ in 0..=rng.below(3) {
            let page = page_range(targets[rng.below(targets.len() as u64) as usize]);
            let (lo, hi) = if rng.below(2) == 0 {
                (page.start + 10, page.start + 20)
            } else {
                (page.start + 32, page.end)
            };
            let at = lo + rng.below((hi - lo) as u64) as usize;
            if rng.below(2) == 0 {
                image[at] ^= 1 << rng.below(8);
            } else {
                image[at] = rng.below(256) as u8;
            }
        }
        restamp_leaf_and_overflow_pages(&mut image);
        let outcome = pglet_testing::db::try_open_database(Box::new(MemBlockIo::from_bytes(image)))
            .and_then(|mut db| {
                db.execute("SELECT id FROM t ORDER BY id")
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
        match outcome {
            Ok(()) => scanned += 1,
            Err(message) if message.contains("overflow chain") => chain_refusals += 1,
            Err(_) => other_errors += 1,
        }
    }
    eprintln!(
        "[restamped] seed={seed} iters={iters}: {scanned} scanned, {chain_refusals} refused as a \
         corrupt chain, {other_errors} other errors"
    );
    assert!(chain_refusals > 0, "the sweep never reached the chain walk");
    assert!(scanned > 0, "the sweep never left an image that reads");
}
