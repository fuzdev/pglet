//! Read-concurrency scaling (the "reads" scenario): cache-warm point-gets across
//! N reader threads against a shared lock-free `Arc<CowBtree>` (no SQLite
//! comparison).
//!
//! Two read modes pin down *which* read-path lock a curve reflects:
//! - **`root_direct`** reads via a pre-captured root (`get_at_root`), so it
//!   isolates the **sharded per-page cache** (`DoubleMetaStore::cache`) — the lock
//!   the cache-sharding work targeted.
//! - **`per_statement_snapshot`** takes `snapshot()` + get + `release_snapshot()`
//!   per op (the realistic server-statement path), so it *adds* the
//!   `active_snapshots: RwLock<HashMap>` refcount write-lock pair (and the
//!   `pending_gc` count lock) on top. The gap between the two modes at high thread
//!   counts is the per-statement snapshot-refcount overhead — the data that says
//!   whether that remaining global lock is worth replacing next.
//!
//! Each (mode, thread count) cell reports the **median of [`REPEATS`] timed runs**
//! after one untimed warmup, so a single noisy run (frequency scaling, scheduler
//! jitter) can't skew the scaling curve.

use std::sync::Arc;
use std::time::Instant;

use pglet::StorageBackend;
use pglet_storage::btree::{BtreeKey, CowBtree, PageId};

use crate::common::{CREATE_SQL, insert_sql, open_pglet, pseudo_a};
use crate::stats::median;

/// Rows pre-loaded before the timed loop — fits the default 2000-page cache, so
/// every timed get is a hit (the working set `bench_concurrent_reads` warms).
const ROWS: u64 = 5_000;
/// Point-gets each reader thread issues in one timed run.
const OPS_PER_THREAD: u64 = 200_000;
/// Timed runs per (mode, thread count); the reported ops/sec is their median.
const REPEATS: usize = 5;
/// The first (only) user table created by `CREATE_SQL`.
const TABLE_ID: u32 = 1;

/// Which read path the timed loop drives — see the module docs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    /// Pre-captured root via `get_at_root`: isolates the sharded page cache,
    /// skipping the per-statement snapshot refcount.
    RootDirect,
    /// `snapshot()` + get + `release_snapshot()` per op: the realistic server
    /// statement path, which adds the `active_snapshots` RwLock write-lock pair.
    PerStatementSnapshot,
}

impl ReadMode {
    /// Stable identifier used in the printed header and the JSON `mode` field.
    pub const fn label(self) -> &'static str {
        match self {
            Self::RootDirect => "root_direct",
            Self::PerStatementSnapshot => "per_statement_snapshot",
        }
    }
}

/// One row of the read-concurrency scaling result (one mode, one thread count).
pub struct ReadConcRow {
    threads: usize,
    mode: &'static str,
    /// Median ops/sec across [`REPEATS`] timed runs.
    ops_per_sec: f64,
    /// Throughput relative to this mode's single-thread baseline (`1.0` at 1 thread).
    scaling: f64,
}

impl ReadConcRow {
    /// Serialize one scaling row. `scaling` is throughput vs the mode's 1-thread
    /// baseline; higher ops/sec (and scaling closer to the thread count) is better.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"mode\":\"{}\",\"threads\":{},\"ops_per_sec\":{:.1},\"scaling\":{:.4},\
\"units\":\"ops_per_sec\",\"higher_or_lower_is_better\":\"higher\"}}",
            self.mode, self.threads, self.ops_per_sec, self.scaling,
        )
    }
}

/// One timed run: `threads` readers each issue `OPS_PER_THREAD` random point-gets
/// against the shared tree via `mode`'s read path. Returns ops/sec.
fn timed_run(btree: &Arc<CowBtree>, root: PageId, threads: usize, mode: ReadMode) -> f64 {
    let start = Instant::now();
    std::thread::scope(|scope| {
        for t in 0..threads {
            let btree = Arc::clone(btree);
            scope.spawn(move || {
                // Per-thread key stream (no shared RNG, no cross-thread state).
                let mut x = (t as u64)
                    .wrapping_add(1)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15);
                for _ in 0..OPS_PER_THREAD {
                    x = pseudo_a(x);
                    let key = BtreeKey::new(TABLE_ID, x % ROWS);
                    match mode {
                        ReadMode::RootDirect => {
                            let _ = btree.get_at_root(root, &key);
                        }
                        ReadMode::PerStatementSnapshot => {
                            // Acquire + release a snapshot per op, as a server
                            // statement does — exercising the active_snapshots lock.
                            let snapshot = btree.snapshot();
                            let _ = btree.get_at_root(snapshot.root, &key);
                            btree.release_snapshot(&snapshot);
                        }
                    }
                }
            });
        }
    });
    let elapsed = start.elapsed().as_secs_f64();
    let total_ops = OPS_PER_THREAD * threads as u64;
    if elapsed > 0.0 {
        total_ops as f64 / elapsed
    } else {
        0.0
    }
}

/// Read-concurrency scaling for one read `mode`: for each thread count, warm up
/// once then take the median of [`REPEATS`] timed runs against a shared,
/// cache-warm `Arc<CowBtree>` with no concurrent writer. Ideal scaling is ~linear
/// in threads; the sharded cache makes `root_direct` *rise* with thread count.
pub fn bench_concurrent_reads(thread_counts: &[usize], mode: ReadMode) -> Vec<ReadConcRow> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("reads.pglet");
    let (mut db, _counters) = open_pglet(&path);
    db.execute(CREATE_SQL).expect("create table");
    db.begin(999).expect("begin prepopulate");
    for id in 0..ROWS {
        // Column `a` is BIGINT (signed i64) and is never read by this bench (gets
        // are keyed by `id`); `insert_sql` masks the (possibly > i64::MAX)
        // `pseudo_a` filler into i64 range so it doesn't fail loud as 22003.
        db.execute_as(&insert_sql(id, pseudo_a(id)), 999)
            .expect("prepopulate insert");
    }
    db.commit(999).expect("commit prepopulate");

    // The shared CoW B+tree + its committed root (cheap Arc clone; `db` keeps the
    // backing file alive for the whole bench).
    let btree: Arc<CowBtree> = match db.storage() {
        StorageBackend::Btree(w) => w.btree(),
        StorageBackend::Memory(_) => unreachable!("bench uses btree storage"),
    };
    let root = btree.current_root();

    // Warm the cache: pull every row once so the timed loop is all hits, and
    // assert the keys resolve (so we're benchmarking real reads, not misses).
    for id in 0..ROWS {
        let got = btree
            .get_at_root(root, &BtreeKey::new(TABLE_ID, id))
            .expect("warm get");
        assert!(got.is_some(), "row {id} missing after prepopulate");
    }

    let mut out = Vec::with_capacity(thread_counts.len());
    for &threads in thread_counts {
        // One untimed warmup run (stabilizes frequency scaling / first-touch), then
        // REPEATS timed runs reported as their median.
        let _ = timed_run(&btree, root, threads, mode);
        let samples: Vec<f64> = (0..REPEATS)
            .map(|_| timed_run(&btree, root, threads, mode))
            .collect();
        out.push(ReadConcRow {
            threads,
            mode: mode.label(),
            ops_per_sec: median(&samples),
            scaling: 0.0,
        });
    }
    // Scaling vs the single-thread baseline (first row), computed once.
    let baseline = out.first().map_or(0.0, |r| r.ops_per_sec);
    for r in &mut out {
        r.scaling = if baseline > 0.0 {
            r.ops_per_sec / baseline
        } else {
            0.0
        };
    }
    out
}

/// Print one mode's read-concurrency table with scaling vs its single-thread
/// baseline.
pub fn print_read_concurrency(rows: &[ReadConcRow]) {
    let mode = rows.first().map_or("", |r| r.mode);
    println!("\n=== read-concurrency scaling [{mode}] (cache-warm point-gets, no writer) ===");
    println!("median of {REPEATS} runs; ideal scaling ≈ threads (rising = healthy)");
    println!("{:>8} {:>16} {:>10}", "threads", "ops/sec", "scaling");
    for r in rows {
        println!(
            "{:>8} {:>16.0} {:>9.2}x",
            r.threads, r.ops_per_sec, r.scaling
        );
    }
}

/// Print the per-thread-count overhead of the snapshot path vs the cache-only
/// path — the cost of the `active_snapshots`/`pending_gc` per-statement locks,
/// isolated by subtracting the two modes at matching thread counts.
fn print_snapshot_overhead(root_rows: &[ReadConcRow], snap_rows: &[ReadConcRow]) {
    println!("\n=== per-statement snapshot-refcount overhead (per_statement vs root_direct) ===");
    println!("cost of the per-op snapshot acquire/release (active_snapshots refcount + GC gate)");
    println!(
        "{:>8} {:>16} {:>16} {:>12}",
        "threads", "root_direct", "snapshot", "slowdown"
    );
    for (rd, ss) in root_rows.iter().zip(snap_rows.iter()) {
        let pct = if rd.ops_per_sec > 0.0 {
            100.0 * (rd.ops_per_sec - ss.ops_per_sec) / rd.ops_per_sec
        } else {
            0.0
        };
        println!(
            "{:>8} {:>16.0} {:>16.0} {:>11.1}%",
            rd.threads, rd.ops_per_sec, ss.ops_per_sec, pct
        );
    }
}

/// The `reads` scenario: run the read-concurrency scaling sweep in both read
/// modes, print the tables + the snapshot-overhead comparison, and (on `--update`)
/// record `benches/reads_results.json` — mirroring `query::run_query`'s
/// print-then-write shape.
pub fn run_reads(update: bool) {
    use core::fmt::Write as _;

    // Sweep past the physical-core count (this box has 12 logical cores) so the
    // curve shows whether scaling is limited by lock contention (would flatten
    // early and more shards would lift it) or by cores/SMT (flattens near the core
    // count regardless of cache sharding).
    let thread_counts = [1usize, 2, 4, 6, 8, 12];
    let root_rows = bench_concurrent_reads(&thread_counts, ReadMode::RootDirect);
    print_read_concurrency(&root_rows);
    let snap_rows = bench_concurrent_reads(&thread_counts, ReadMode::PerStatementSnapshot);
    print_read_concurrency(&snap_rows);
    print_snapshot_overhead(&root_rows, &snap_rows);

    let mut json = String::new();
    json.push_str("{\n  \"meta\": {\n");
    json.push_str("    \"benchmark\": \"read_concurrency_scaling\",\n");
    json.push_str("    \"lane\": \"native_inprocess\",\n");
    json.push_str("    \"layer\": \"L1\",\n");
    let _ = writeln!(json, "    \"rows_prepopulated\": {ROWS},");
    let _ = writeln!(json, "    \"ops_per_thread\": {OPS_PER_THREAD},");
    let _ = writeln!(json, "    \"repeats\": {REPEATS},");
    json.push_str("    \"units\": \"ops_per_sec\",\n");
    json.push_str(
        "    \"note\": \"cache-warm point-gets, no concurrent writer; median of \
`repeats` timed runs. mode=root_direct isolates the sharded per-page cache \
(DoubleMetaStore::cache) — scaling should rise with thread count; \
mode=per_statement_snapshot adds the active_snapshots RwLock per op (the realistic \
server statement path), so the gap is the snapshot-refcount overhead\",\n",
    );
    json.push_str("    \"higher_or_lower_is_better\": \"higher\"\n  },\n");
    json.push_str("  \"rows\": [\n");
    let all: Vec<&ReadConcRow> = root_rows.iter().chain(snap_rows.iter()).collect();
    for (i, r) in all.iter().enumerate() {
        json.push_str("    ");
        json.push_str(&r.to_json());
        json.push_str(if i + 1 < all.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]\n}\n");

    crate::common::write_results(
        &crate::common::bench_results_path("reads_results.json"),
        &json,
        update,
    );

    println!("\n=== JSON ===");
    println!("{json}");
}
