//! Commit-latency sweep + commit-CPU attribution (the "commit" and "attribute"
//! scenarios): per-commit wall-clock latency across a batch-size sweep vs
//! SQLite, write-amplification counters, and the write/fsync/CPU breakdown,
//! plus the sub-phase attribution of the batched-commit CPU.

use std::time::Instant;

use pglet::{Database, StorageBackend};
use pglet_storage::btree::{CountingHandle, Page, PageId, PageStore};

use crate::common::{
    BENCH_SCHEMAS, BenchSchema, SQLITE_CACHE_SIZE, SQLITE_PAGE_SIZE, TEXT_B, bench_results_path,
    insert_sql, open_pglet, prepopulate_pglet, prepopulate_rows, pseudo_a, write_results,
};
use crate::reads::{ReadMode, bench_concurrent_reads, print_read_concurrency};
use crate::stats::{cv, percentile_sorted};

/// Batch sizes to sweep (inserts per commit). N=1 is autocommit. Both schema axes
/// use these — the enforcing path is now flat across batch size (no super-linear
/// cost to throttle around).
const BATCH_SIZES: &[u64] = &[1, 100, 1000, 10_000];

/// Choose the number of commits M for a given batch size N so each cell keeps a
/// percentile-stable sample count while doing a comparable total-work band
/// (~50k–300k inserts). Both schemas use the same counts; enforcing is only a
/// small constant factor slower per insert.
const fn commits_for_batch(n: u64) -> u64 {
    // an explicit batch→commits lookup table; the default arm duplicates the
    // 1_000 body on purpose so every documented size stays spelled out in order.
    #[allow(clippy::match_same_arms)]
    match n {
        1 => 5_000,   // 5k inserts, 5k commits
        100 => 1_000, // 100k inserts, 1k commits
        1_000 => 100, // 100k inserts, 100 commits
        10_000 => 30, // 300k inserts, 30 commits
        _ => 100,
    }
}

// ---------------------------------------------------------------------------
// Result row.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Row {
    engine: String,
    /// Schema axis this row was measured under (`indexfree` / `enforcing`).
    schema: &'static str,
    /// pragmas string for SQLite, or "double-meta" for pglet.
    baseline_config: String,
    batch_n: u64,
    commits: u64,
    p50_us: f64,
    p95_us: f64,
    cv: f64,
    fsyncs_per_commit: f64,
    fsyncs_measured: bool,
    writes_per_commit: f64,
    bytes_per_commit: f64,
    /// 3-way commit-time breakdown (pglet only), captured for the larger
    /// batch cells. `None` when not measured for this row.
    breakdown: Option<CommitBreakdown>,
    /// Optional condition note (e.g. an INFEASIBLE cap failure). `None` for a
    /// clean cell.
    note: Option<String>,
}

/// Average per-commit attribution of pglet commit wall-clock into the inner
/// `write_at` syscall time, the `sync` (fsync) time, and the residual CPU
/// (CoW build + `Page::serialize` + cache + everything else). All in
/// microseconds.
// The `_us` suffix marks the unit (microseconds), not a redundant field-name
// stutter; keeping it on every field is the point.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
struct CommitBreakdown {
    total_us: f64,
    write_us: f64,
    sync_us: f64,
    cpu_us: f64,
}

impl CommitBreakdown {
    fn pct(part: f64, total: f64) -> f64 {
        if total > 0.0 {
            100.0 * part / total
        } else {
            0.0
        }
    }
}

impl Row {
    fn to_json(&self) -> String {
        let note = self
            .note
            .as_ref()
            .map_or_else(|| "null".to_string(), |n| format!("{n:?}"));
        let breakdown = self.breakdown.as_ref().map_or_else(
            || "null".to_string(),
            |b| {
                let t = b.total_us;
                format!(
                    "{{\"total_us\":{:.3},\
\"cpu_us\":{:.3},\"cpu_pct\":{:.2},\
\"write_at_us\":{:.3},\"write_at_pct\":{:.2},\
\"fsync_us\":{:.3},\"fsync_pct\":{:.2}}}",
                    t,
                    b.cpu_us,
                    CommitBreakdown::pct(b.cpu_us, t),
                    b.write_us,
                    CommitBreakdown::pct(b.write_us, t),
                    b.sync_us,
                    CommitBreakdown::pct(b.sync_us, t),
                )
            },
        );
        format!(
            "{{\"engine\":{:?},\"schema\":{:?},\"layer\":\"L1\",\"durability_tier\":\"T1\",\
\"baseline_config\":{:?},\"batch_n\":{},\"commits\":{},\
\"p50_us\":{:.3},\"p95_us\":{:.3},\"cv\":{:.4},\
\"fsyncs_per_commit\":{:.3},\"fsyncs_source\":{:?},\
\"writes_per_commit\":{:.3},\"bytes_per_commit\":{:.1},\
\"commit_breakdown\":{},\
\"units\":\"microseconds\",\"higher_or_lower_is_better\":\"lower\",\"note\":{}}}",
            self.engine,
            self.schema,
            self.baseline_config,
            self.batch_n,
            self.commits,
            self.p50_us,
            self.p95_us,
            self.cv,
            self.fsyncs_per_commit,
            if self.fsyncs_measured {
                "measured"
            } else {
                "from_config"
            },
            self.writes_per_commit,
            self.bytes_per_commit,
            breakdown,
            note,
        )
    }
}

fn bench_pglet(conn_id: i32, n: u64, prepopulate: u64, schema: BenchSchema) -> Row {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bench.pglet");
    let (mut db, counters) = open_pglet(&path);

    for stmt in schema.create_stmts() {
        db.execute(stmt).expect("create schema");
    }

    // Pre-populate in bounded chunks (setup, not measured).
    let mut next_id: u64 = prepopulate_pglet(&mut db, conn_id, prepopulate);

    let commits = commits_for_batch(n);
    let mut latencies = Vec::with_capacity(commits as usize);
    let mut total_syncs = 0u64;
    let mut total_writes = 0u64;
    let mut total_bytes = 0u64;
    // Per-commit deltas of the inner write_at / sync nanosecond counters, summed
    // across the measured commits so we can split commit wall-clock into
    // write-syscall / fsync / CPU. Captured for the larger batch cells.
    let measure_breakdown = matches!(n, 1_000 | 10_000);
    let mut total_write_nanos = 0u64;
    let mut total_sync_nanos = 0u64;
    let mut total_wall_nanos = 0u64;
    let mut note: Option<String> = None;

    'cell: for _ in 0..commits {
        let before = counters.stats();
        let start = Instant::now();
        let result: Result<(), pglet::Error> = if n == 1 {
            // Autocommit: a single insert, no explicit transaction. The facade
            // fsyncs (flush + meta-flip).
            let a = pseudo_a(next_id);
            let r = run_autocommit_insert(&mut db, conn_id, next_id, a);
            next_id += 1;
            r
        } else {
            db.begin(conn_id).expect("begin batch");
            for _ in 0..n {
                let a = pseudo_a(next_id);
                db.execute_as(&insert_sql(next_id, a), conn_id)
                    .expect("batch insert");
                next_id += 1;
            }
            db.commit(conn_id)
        };
        let elapsed = start.elapsed().as_nanos() as f64 / 1000.0; // microseconds
        match result {
            Ok(()) => {
                let after = counters.stats();
                latencies.push(elapsed);
                total_syncs += after.syncs - before.syncs;
                total_writes += after.writes - before.writes;
                total_bytes += after.bytes_written - before.bytes_written;
                if measure_breakdown {
                    total_write_nanos += after.write_nanos - before.write_nanos;
                    total_sync_nanos += after.sync_nanos - before.sync_nanos;
                    // Wall-clock in nanos for this same commit, so write/sync/CPU
                    // sum back to the measured total (not the p50 latency).
                    total_wall_nanos += (elapsed * 1000.0) as u64;
                }
            }
            Err(e) => {
                // Defensive: no commit is expected to fail here (PGM5's chained
                // free-list lifted the old meta-slot ceiling, and ids are unique
                // so the enforcing PK never collides). If one does, record how
                // far we got and stop this cell rather than crashing the sweep.
                note = Some(format!("INFEASIBLE after {} commits: {e}", latencies.len()));
                break 'cell;
            }
        }
    }

    // Build the 3-way breakdown (per-commit averages, in µs) from the summed
    // deltas. CPU is the residual: total wall-clock minus the inner write_at and
    // sync time. Computed on the same set of measured commits.
    let breakdown = if measure_breakdown && !latencies.is_empty() {
        let done_n = latencies.len() as f64;
        let total_us = total_wall_nanos as f64 / 1000.0 / done_n;
        let write_us = total_write_nanos as f64 / 1000.0 / done_n;
        let sync_us = total_sync_nanos as f64 / 1000.0 / done_n;
        let cpu_us = (total_us - write_us - sync_us).max(0.0);
        Some(CommitBreakdown {
            total_us,
            write_us,
            sync_us,
            cpu_us,
        })
    } else {
        None
    };

    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let done = latencies.len().max(1) as f64;
    let mut row = Row {
        engine: "pglet".to_string(),
        schema: schema.label(),
        baseline_config: "double-meta".to_string(),
        batch_n: n,
        commits: latencies.len() as u64,
        p50_us: percentile_sorted(&latencies, 0.50),
        p95_us: percentile_sorted(&latencies, 0.95),
        cv: cv(&latencies),
        fsyncs_per_commit: total_syncs as f64 / done,
        fsyncs_measured: true,
        writes_per_commit: total_writes as f64 / done,
        bytes_per_commit: total_bytes as f64 / done,
        breakdown,
        note,
    };
    if row.commits == 0 {
        // Not even one commit succeeded; zero the stats so JSON stays honest.
        row.p50_us = 0.0;
        row.p95_us = 0.0;
    }
    row
}

/// One autocommit insert, surfacing the storage error (the cap failure) instead
/// of panicking. `execute_as` wraps storage errors in `pglet::Error`, so we
/// inspect the message for the meta-slot-capacity signature.
fn run_autocommit_insert(
    db: &mut Database,
    conn_id: i32,
    id: u64,
    a: u64,
) -> Result<(), pglet::Error> {
    db.execute_as(&insert_sql(id, a), conn_id).map(|_| ())
}

// ---------------------------------------------------------------------------
// SQLite engine via rusqlite (bundled).
// ---------------------------------------------------------------------------

struct SqliteConfig {
    label: &'static str,
    journal_mode: &'static str,
    synchronous: &'static str,
    /// Documented fsyncs/commit from config (NOT measured — rusqlite exposes no
    /// counter).
    fsyncs_per_commit_from_config: f64,
}

const SQLITE_CONFIGS: &[SqliteConfig] = &[
    SqliteConfig {
        label: "wal_normal",
        journal_mode: "WAL",
        synchronous: "NORMAL",
        fsyncs_per_commit_from_config: 1.0,
    },
    SqliteConfig {
        label: "delete_full",
        journal_mode: "DELETE",
        synchronous: "FULL",
        fsyncs_per_commit_from_config: 2.0,
    },
];

fn pragma_string(cfg: &SqliteConfig) -> String {
    format!(
        "journal_mode={};synchronous={};page_size={};cache_size={}",
        cfg.journal_mode, cfg.synchronous, SQLITE_PAGE_SIZE, SQLITE_CACHE_SIZE
    )
}

fn bench_sqlite(cfg: &SqliteConfig, n: u64, prepopulate: u64, schema: BenchSchema) -> Row {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bench.sqlite");
    let conn = rusqlite::Connection::open(&path).expect("open sqlite");

    // Pin pragmas. page_size must be set before any table is created.
    conn.pragma_update(None, "page_size", SQLITE_PAGE_SIZE)
        .expect("page_size");
    conn.pragma_update(None, "cache_size", SQLITE_CACHE_SIZE)
        .expect("cache_size");
    conn.pragma_update(None, "journal_mode", cfg.journal_mode)
        .expect("journal_mode");
    conn.pragma_update(None, "synchronous", cfg.synchronous)
        .expect("synchronous");

    for stmt in schema.create_stmts() {
        conn.execute_batch(stmt).expect("create schema");
    }

    // Pre-populate in ONE transaction. SQLite's B-tree updates in place (no CoW
    // re-descent), so a single big batch has none of the pglet-side pathology the
    // chunked prepopulate exists to avoid — setup isn't measured either way.
    let mut next_id: u64 = 0;
    conn.execute_batch("BEGIN").expect("begin prepopulate");
    {
        let mut stmt = conn
            .prepare("INSERT INTO bench VALUES (?1, ?2, ?3)")
            .expect("prepare prepopulate");
        for _ in 0..prepopulate {
            let a = pseudo_a(next_id);
            stmt.execute(rusqlite::params![next_id as i64, a as i64, TEXT_B])
                .expect("prepopulate insert");
            next_id += 1;
        }
    }
    conn.execute_batch("COMMIT").expect("commit prepopulate");

    let commits = commits_for_batch(n);
    let mut latencies = Vec::with_capacity(commits as usize);

    let mut stmt = conn
        .prepare("INSERT INTO bench VALUES (?1, ?2, ?3)")
        .expect("prepare insert");

    for _ in 0..commits {
        let start = Instant::now();
        if n == 1 {
            // Autocommit (no explicit BEGIN): each insert is its own commit.
            let a = pseudo_a(next_id);
            stmt.execute(rusqlite::params![next_id as i64, a as i64, TEXT_B])
                .expect("autocommit insert");
            next_id += 1;
        } else {
            conn.execute_batch("BEGIN").expect("begin batch");
            for _ in 0..n {
                let a = pseudo_a(next_id);
                stmt.execute(rusqlite::params![next_id as i64, a as i64, TEXT_B])
                    .expect("batch insert");
                next_id += 1;
            }
            conn.execute_batch("COMMIT").expect("commit batch");
        }
        let elapsed = start.elapsed().as_nanos() as f64 / 1000.0;
        latencies.push(elapsed);
    }
    drop(stmt);

    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Row {
        engine: format!("sqlite_{}", cfg.label),
        schema: schema.label(),
        baseline_config: pragma_string(cfg),
        batch_n: n,
        commits,
        p50_us: percentile_sorted(&latencies, 0.50),
        p95_us: percentile_sorted(&latencies, 0.95),
        cv: cv(&latencies),
        fsyncs_per_commit: cfg.fsyncs_per_commit_from_config,
        fsyncs_measured: false,
        writes_per_commit: 0.0,
        bytes_per_commit: 0.0,
        breakdown: None,
        note: None,
    }
}

// ---------------------------------------------------------------------------
// Commit-CPU attribution (the "attribute" scenario).
//
// Decomposes a batched commit's CPU — which `bench:commit` already shows is
// ~98% of commit wall-clock — into the named put-loop sub-phases plus the
// apply-loop residual:
//   (A) apply loop   — B+tree descent + copy-on-first-write page clones
//   (B) serialize    — `Page::serialize` (postcard over the leaf values)
//   (C) checksum     — `page_checksum` (CRC32 over the 4 KiB buffer)
//   (D) clone        — the `page.clone()` in the put loop
//   (E) write_at     — `io.write_at` (a full 4 KiB page each)
//
// Method (bench-side, zero production change): run real N-insert commits and
// read the per-commit page-write count + write_at/fsync nanos from the existing
// `CountingBlockIo`. Then micro-time serialize / clone / full-`put` on a
// representative dirtied page pulled back out of the store; checksum falls out
// of `put_total − serialize − write_at − clone − cache` by difference (the CRC
// crate isn't a bench dep, so difference is the honest isolation). The apply
// loop is the residual `commit_cpu − (serialize+checksum+clone+write_at)`.
// ---------------------------------------------------------------------------

/// One per-page micro-timing (nanoseconds, averaged over the iteration count).
// The `_ns` suffix marks the unit on every field — that's the point, not a
// redundant name stutter (mirrors `CommitBreakdown`'s `_us`).
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
struct PerPage {
    serialize_ns: f64,
    clone_ns: f64,
    /// Full `store.put(page.clone())`: serialize + checksum + write_at + cache.
    put_total_ns: f64,
    /// `write_at` nanos attributable to one `put`, read from the counter delta
    /// across the `put` micro-loop (so it's the *real* syscall cost, not a
    /// separate write).
    write_at_ns: f64,
    /// `store.get(leaf)` on a cache-hot page: a deep `Page` clone out of the
    /// page cache — the per-descent-step cost the apply loop pays on every
    /// level of every row's root-to-leaf walk (diagnostic for the apply-loop
    /// dominance; not part of the put-loop sub-phases).
    store_get_ns: f64,
}

impl PerPage {
    /// Checksum (+ cache insert + the header re-encode inside `put`) by
    /// difference: `put` does serialize, checksum, write_at, and a cache put;
    /// subtracting the separately-timed serialize and the counter-measured
    /// write_at leaves the checksum-dominated residual. `clone` is *not*
    /// subtracted — `put` takes an already-owned page, the bench clones into it
    /// — so this residual is checksum + cache bookkeeping.
    fn checksum_ns(&self) -> f64 {
        (self.put_total_ns - self.serialize_ns - self.write_at_ns).max(0.0)
    }
}

/// Average per-commit attribution, in microseconds, plus the page/row context.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
struct Attribution {
    /// Schema axis this attribution was measured under (`indexfree` /
    /// `enforcing`) — the enforcing shape adds the PK + secondary index B+tree
    /// descents to the apply loop.
    schema: &'static str,
    batch_n: u64,
    /// Unique dirtied pages written per commit (from the write counter).
    pages_per_commit: f64,
    /// Measured commit wall-clock per commit.
    total_us: f64,
    /// Commit CPU = wall − write_at − fsync (the residual the breakdown calls
    /// `cpu`); this is what we decompose.
    commit_cpu_us: f64,
    apply_us: f64,
    serialize_us: f64,
    checksum_us: f64,
    clone_us: f64,
    write_at_us: f64,
    fsync_us: f64,
    /// Diagnostic: cost of one cache-hot `store.get` deep-clone (µs), the unit
    /// the apply loop's descent pays per level per row. `apply_us / this`
    /// estimates the deep-clone count behind the apply-loop dominance.
    store_get_us: f64,
}

impl Attribution {
    fn pct(part: f64, total: f64) -> f64 {
        if total > 0.0 {
            100.0 * part / total
        } else {
            0.0
        }
    }

    /// Per-row CPU in microseconds (commit CPU divided by the batch size) — the
    /// number to compare against SQLite's per-row write cost.
    fn cpu_per_row_us(&self) -> f64 {
        if self.batch_n > 0 {
            self.commit_cpu_us / self.batch_n as f64
        } else {
            0.0
        }
    }

    fn to_json(self) -> String {
        let t = self.total_us;
        format!(
            "{{\"schema\":{:?},\"batch_n\":{},\"pages_per_commit\":{:.2},\
\"total_us\":{:.3},\"commit_cpu_us\":{:.3},\"cpu_per_row_us\":{:.4},\
\"apply_us\":{:.3},\"apply_pct\":{:.2},\
\"serialize_us\":{:.3},\"serialize_pct\":{:.2},\
\"checksum_us\":{:.3},\"checksum_pct\":{:.2},\
\"clone_us\":{:.3},\"clone_pct\":{:.2},\
\"write_at_us\":{:.3},\"write_at_pct\":{:.2},\
\"fsync_us\":{:.3},\"fsync_pct\":{:.2},\
\"diag_store_get_us\":{:.4},\"diag_descent_clones_per_commit\":{:.1},\
\"pct_basis\":\"commit_total\",\"units\":\"microseconds\"}}",
            self.schema,
            self.batch_n,
            self.pages_per_commit,
            t,
            self.commit_cpu_us,
            self.cpu_per_row_us(),
            self.apply_us,
            Self::pct(self.apply_us, t),
            self.serialize_us,
            Self::pct(self.serialize_us, t),
            self.checksum_us,
            Self::pct(self.checksum_us, t),
            self.clone_us,
            Self::pct(self.clone_us, t),
            self.write_at_us,
            Self::pct(self.write_at_us, t),
            self.fsync_us,
            Self::pct(self.fsync_us, t),
            self.store_get_us,
            self.descent_clones_per_commit(),
        )
    }

    /// Estimated cache-hot deep-clones behind the apply loop = apply-loop time
    /// divided by the measured cost of one such clone. A diagnostic, not a
    /// decomposition — descent isn't the *only* thing the apply loop does, but
    /// it's the repeated-per-row cost the clone count exposes.
    fn descent_clones_per_commit(&self) -> f64 {
        if self.store_get_us > 0.0 {
            self.apply_us / self.store_get_us
        } else {
            0.0
        }
    }
}

/// Pull a representative *dirtied* page out of the store: the page under
/// `page_id`, deserialized back from disk (so it's the real on-disk shape the
/// put loop rewrites). Falls back to scanning a small id window for a leaf so
/// we micro-time the dominant page kind.
fn representative_page(store: &dyn PageStore, hint: PageId) -> Page {
    if let Ok(Some(p)) = store.get(hint)
        && p.is_leaf()
    {
        return p;
    }
    // Scan a window of ids for a leaf (leaves dominate the dirtied set).
    for id in 0..256u64 {
        if let Ok(Some(p)) = store.get(id)
            && p.is_leaf()
            && p.key_count() > 0
        {
            return p;
        }
    }
    // Last resort: whatever the hint resolved to, or a fresh leaf.
    store
        .get(hint)
        .ok()
        .flatten()
        .unwrap_or_else(|| Page::new_leaf(hint))
}

/// Micro-time `serialize`, `clone`, and full `put` on `page`. `iters` controls
/// the averaging; `std::hint::black_box` keeps the optimizer from hoisting the
/// work out. The `put` loop reuses one allocated page id (`page`'s own id), so
/// it overwrites in place — the real put-loop write pattern.
fn micro_time_page(store: &dyn PageStore, counters: &CountingHandle, page: &Page) -> PerPage {
    const ITERS: u64 = 2_000;

    // (B) serialize — postcard over the leaf values into a fresh 4 KiB buffer.
    let start = Instant::now();
    let mut sink = 0u64;
    for _ in 0..ITERS {
        // A page the store read back fits by construction; an error here would
        // count as zero bytes rather than abort the measurement.
        let bytes = std::hint::black_box(page.serialize());
        sink = sink.wrapping_add(bytes.map_or(0, |b| b.len() as u64));
    }
    let serialize_ns = start.elapsed().as_nanos() as f64 / ITERS as f64;
    std::hint::black_box(sink);

    // (D) clone — the `page.clone()` the put loop pays per dirtied page.
    let start = Instant::now();
    let mut sink = 0u64;
    for _ in 0..ITERS {
        let c = std::hint::black_box(page.clone());
        sink = sink.wrapping_add(c.page_id());
    }
    let clone_ns = start.elapsed().as_nanos() as f64 / ITERS as f64;
    std::hint::black_box(sink);

    // Full `store.put` — serialize + checksum + write_at + cache insert. Read
    // the write_at counter delta across the loop to split out the syscall (E).
    let before = counters.stats();
    let start = Instant::now();
    for _ in 0..ITERS {
        store
            .put(std::hint::black_box(page.clone()))
            .expect("micro put");
    }
    let put_total_ns = start.elapsed().as_nanos() as f64 / ITERS as f64;
    let after = counters.stats();
    let write_at_ns = (after.write_nanos - before.write_nanos) as f64 / ITERS as f64;

    // `store.get` on a cache-hot leaf — the deep-clone the apply loop pays per
    // descent step (`BatchWorkingSet::get` → `DoubleMetaStore::get` clones out
    // of the cache). The `put` loop above primed the cache for `page`'s id.
    let page_id = page.page_id();
    let start = Instant::now();
    let mut sink = 0u64;
    for _ in 0..ITERS {
        let p = std::hint::black_box(store.get(page_id));
        sink = sink.wrapping_add(p.ok().flatten().map_or(0, |p| p.key_count() as u64));
    }
    let store_get_ns = start.elapsed().as_nanos() as f64 / ITERS as f64;
    std::hint::black_box(sink);

    PerPage {
        serialize_ns,
        clone_ns,
        put_total_ns,
        write_at_ns,
        store_get_ns,
    }
}

/// Run the commit-CPU attribution for batch size `n`.
fn attribute_commit(conn_id: i32, n: u64, prepopulate: u64, schema: BenchSchema) -> Attribution {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("attr.pglet");
    let (mut db, counters) = open_pglet(&path);
    for stmt in schema.create_stmts() {
        db.execute(stmt).expect("create schema");
    }

    // Prepopulate in bounded chunks (setup, not measured) for real B+tree depth.
    let mut next_id: u64 = prepopulate_pglet(&mut db, conn_id, prepopulate);

    // Measure real N-insert commits: per-commit wall-clock + write_at/fsync
    // nanos + pages written, summed across the measured commits.
    let commits = commits_for_batch(n).clamp(20, 60);
    let mut total_wall_ns = 0u64;
    let mut total_write_ns = 0u64;
    let mut total_sync_ns = 0u64;
    let mut total_writes = 0u64;
    let mut last_written_page: PageId = 0;
    for _ in 0..commits {
        let before = counters.stats();
        let start = Instant::now();
        db.begin(conn_id).expect("begin batch");
        for _ in 0..n {
            let a = pseudo_a(next_id);
            db.execute_as(&insert_sql(next_id, a), conn_id)
                .expect("batch insert");
            // Track an id we know is freshly inserted to find a dirtied leaf.
            last_written_page = next_id % 200;
            next_id += 1;
        }
        db.commit(conn_id).expect("commit batch");
        let wall = start.elapsed().as_nanos() as u64;
        let after = counters.stats();
        total_wall_ns += wall;
        total_write_ns += after.write_nanos - before.write_nanos;
        total_sync_ns += after.sync_nanos - before.sync_nanos;
        total_writes += after.writes - before.writes;
    }

    let done = commits as f64;
    let total_us = total_wall_ns as f64 / 1000.0 / done;
    let write_at_us = total_write_ns as f64 / 1000.0 / done;
    let fsync_us = total_sync_ns as f64 / 1000.0 / done;
    let commit_cpu_us = (total_us - write_at_us - fsync_us).max(0.0);
    let pages_per_commit = total_writes as f64 / done;

    // Per-page micro-timings on a representative dirtied leaf page.
    let btree = match db.storage() {
        StorageBackend::Btree(w) => w.btree(),
        StorageBackend::Memory(_) => unreachable!("attribution uses btree storage"),
    };
    let store = btree.store();
    let page = representative_page(store, last_written_page);
    let per_page = micro_time_page(store, &counters, &page);

    // Scale per-page costs by pages-per-commit (µs).
    let serialize_us = per_page.serialize_ns * pages_per_commit / 1000.0;
    let checksum_us = per_page.checksum_ns() * pages_per_commit / 1000.0;
    let clone_us = per_page.clone_ns * pages_per_commit / 1000.0;
    // The put-loop write_at is already in `write_at_us` (measured in the real
    // commit). The apply loop is the residual CPU after the put-loop sub-phases.
    let put_loop_cpu = serialize_us + checksum_us + clone_us;
    let apply_us = (commit_cpu_us - put_loop_cpu).max(0.0);

    Attribution {
        schema: schema.label(),
        batch_n: n,
        pages_per_commit,
        total_us,
        commit_cpu_us,
        apply_us,
        serialize_us,
        checksum_us,
        clone_us,
        write_at_us,
        fsync_us,
        store_get_us: per_page.store_get_ns / 1000.0,
    }
}

/// Print the attribution table for one batch size.
fn print_attribution(a: &Attribution) {
    let t = a.total_us;
    println!(
        "\n=== commit-CPU attribution (native L1, T1) — schema={}, N={} inserts/commit ===",
        a.schema, a.batch_n
    );
    println!(
        "pages dirtied & rewritten per commit: {:.1}  (full 4 KiB each = {:.0} KiB/commit)",
        a.pages_per_commit,
        a.pages_per_commit * 4096.0 / 1024.0
    );
    println!(
        "commit total = {:.2}us/commit;  commit CPU = {:.2}us ({:.1}%);  \
per-row CPU = {:.3}us",
        t,
        a.commit_cpu_us,
        Attribution::pct(a.commit_cpu_us, t),
        a.cpu_per_row_us(),
    );
    println!("{:<28} {:>12} {:>10}", "sub-phase", "us/commit", "% commit");
    println!("{}", "-".repeat(52));
    let row = |label: &str, us: f64| {
        println!(
            "{:<28} {:>12.3} {:>9.1}%",
            label,
            us,
            Attribution::pct(us, t)
        );
    };
    row("(A) apply loop (descent+CoW)", a.apply_us);
    row("(B) serialize (postcard)", a.serialize_us);
    row("(C) checksum (CRC32)", a.checksum_us);
    row("(D) page.clone()", a.clone_us);
    row("(E) write_at (4 KiB pages)", a.write_at_us);
    row("    fsync (2× per commit)", a.fsync_us);
    println!("{}", "-".repeat(52));
    let put_loop = a.serialize_us + a.checksum_us + a.clone_us + a.write_at_us;
    println!(
        "put loop (B+C+D+E) = {:.2}us ({:.1}%);  apply loop (A) = {:.2}us ({:.1}%)",
        put_loop,
        Attribution::pct(put_loop, t),
        a.apply_us,
        Attribution::pct(a.apply_us, t),
    );
    println!(
        "diag: one cache-hot store.get deep-clone = {:.4}us  →  apply loop ≈ {:.0} \
such clones/commit ({:.2}/row)\n      (the apply loop's per-descent-step cost; \
each row re-descends root→leaf, deep-cloning every page on the path)",
        a.store_get_us,
        a.descent_clones_per_commit(),
        if a.batch_n > 0 {
            a.descent_clones_per_commit() / a.batch_n as f64
        } else {
            0.0
        },
    );
}

/// The `attribute` scenario: decompose the batched-commit CPU into named
/// sub-phases for the configured batch sizes (default N=1000, the run-sheet's
/// attribution target). Prints by default; `--update` refreshes the recorded
/// JSON alongside the commit results.
pub fn run_attribution(update: bool) {
    use core::fmt::Write as _;
    let prepopulate = prepopulate_rows();
    // N=1000 is the run-sheet's attribution target; also do 100 and 10000 so the
    // breakdown's batch-scaling is visible (apply loop grows with depth/CoW). Both
    // schema axes at every size — the enforcing per-row cost is now flat across
    // batch size (the O(batch²) it once showed is closed), so the enforcement cost
    // is a clean constant-factor delta over index-free.
    let attr_batch_sizes: &[u64] = &[100, 1_000, 10_000];
    eprintln!(
        "pglet_bench: commit-CPU attribution (native L1, T1). \
prepopulate={prepopulate} rows; schemas=[indexfree, enforcing]"
    );

    let mut attrs: Vec<Attribution> = Vec::new();
    let mut conn_id = 1;
    for &schema in BENCH_SCHEMAS {
        for &n in attr_batch_sizes {
            eprintln!("  attribute schema={} N={n} ...", schema.label());
            let a = attribute_commit(conn_id, n, prepopulate, schema);
            print_attribution(&a);
            attrs.push(a);
            conn_id += 1;
        }
    }

    // JSON block (same print-by-default / --update idiom).
    let mut json = String::new();
    json.push_str("{\n  \"meta\": {\n");
    json.push_str("    \"benchmark\": \"commit_cpu_attribution\",\n");
    json.push_str("    \"lane\": \"native_inprocess\",\n");
    json.push_str("    \"layer\": \"L1\",\n");
    json.push_str("    \"durability_tier\": \"T1\",\n");
    let _ = writeln!(json, "    \"prepopulate_rows\": {prepopulate},");
    json.push_str("    \"schemas\": [\"indexfree\", \"enforcing\"],\n");
    json.push_str(
        "    \"method\": \"real commits for page-count + write_at/fsync nanos; per-page serialize/clone/put micro-timed on a representative dirtied leaf; checksum + apply by difference\",\n",
    );
    json.push_str("    \"units\": \"microseconds\",\n");
    json.push_str("    \"higher_or_lower_is_better\": \"lower\"\n  },\n");
    json.push_str("  \"attributions\": [\n");
    for (i, a) in attrs.iter().enumerate() {
        json.push_str("    ");
        json.push_str(&a.to_json());
        json.push_str(if i + 1 < attrs.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]\n}\n");

    write_results(
        &bench_results_path("attribution_results.json"),
        &json,
        update,
    );

    println!("\n=== JSON ===");
    println!("{json}");
}

/// The `commit` scenario (also the default `all` path): run the commit-latency
/// sweep + write-amp counters vs SQLite, print the table + ratios, and — unless
/// the scenario is exactly `commit` — append the read-concurrency micro-bench.
pub fn run_commit(update: bool, scenario: &str) {
    use core::fmt::Write as _;
    let prepopulate = prepopulate_rows();
    eprintln!(
        "pglet_bench: Phase-1 commit-latency sweep (native L1, T1). \
prepopulate={prepopulate} rows; batch sizes={BATCH_SIZES:?}; schemas=[indexfree, enforcing]"
    );

    let mut rows: Vec<Row> = Vec::new();

    // Use a distinct conn_id per pglet run to keep transaction routing clean.
    let mut conn_id: i32 = 1;

    // Two schema axes: the index-free ceiling and the enforcing (PK + secondary
    // index) shape, so the honest write cost is measured alongside its delta.
    for &schema in BENCH_SCHEMAS {
        for &n in BATCH_SIZES {
            eprintln!("  pglet  schema={} N={n} ...", schema.label());
            rows.push(bench_pglet(conn_id, n, prepopulate, schema));
            conn_id += 1;

            for cfg in SQLITE_CONFIGS {
                eprintln!("  sqlite_{} schema={} N={n} ...", cfg.label, schema.label());
                rows.push(bench_sqlite(cfg, n, prepopulate, schema));
            }
        }
    }

    // --- ratios: pglet p50 / sqlite_wal_normal p50, per (schema, N) ---
    // (schema, n, pglet_p50, sqlite_wal_p50, ratio)
    let mut ratios: Vec<(&'static str, u64, f64, f64, f64)> = Vec::new();
    for &schema in BENCH_SCHEMAS {
        let s = schema.label();
        for &n in BATCH_SIZES {
            let pglet_p50 = rows
                .iter()
                .find(|r| r.engine == "pglet" && r.schema == s && r.batch_n == n)
                .map_or(0.0, |r| r.p50_us);
            let sqlite_wal_p50 = rows
                .iter()
                .find(|r| r.engine == "sqlite_wal_normal" && r.schema == s && r.batch_n == n)
                .map_or(0.0, |r| r.p50_us);
            let ratio = if sqlite_wal_p50 > 0.0 {
                pglet_p50 / sqlite_wal_p50
            } else {
                0.0
            };
            ratios.push((s, n, pglet_p50, sqlite_wal_p50, ratio));
        }
    }

    // --- human-readable table ---
    println!("\n=== Phase-1 commit-latency (native L1, durability T1) ===");
    println!(
        "indexfree = (id BIGINT, a BIGINT, b TEXT[{} chars]); enforcing adds id PRIMARY KEY + \
a secondary index on a; prepopulate={prepopulate} both; SQLite page_size={SQLITE_PAGE_SIZE} \
cache_size={SQLITE_CACHE_SIZE}",
        TEXT_B.len(),
    );
    for &schema in BENCH_SCHEMAS {
        let s = schema.label();
        println!("\n-- schema = {s} --");
        println!(
            "{:<20} {:>7} {:>8} {:>12} {:>12} {:>7} {:>10} {:>9} {:>11}",
            "engine", "N", "commits", "p50_us", "p95_us", "cv", "fsync/cm", "write/cm", "bytes/cm"
        );
        println!("{}", "-".repeat(108));
        for &n in BATCH_SIZES {
            for r in rows.iter().filter(|r| r.schema == s && r.batch_n == n) {
                let fsync = if r.fsyncs_measured {
                    format!("{:.2}", r.fsyncs_per_commit)
                } else {
                    format!("{:.1}*", r.fsyncs_per_commit)
                };
                println!(
                    "{:<20} {:>7} {:>8} {:>12.2} {:>12.2} {:>7.3} {:>10} {:>9.2} {:>11.1}",
                    r.engine,
                    r.batch_n,
                    r.commits,
                    r.p50_us,
                    r.p95_us,
                    r.cv,
                    fsync,
                    r.writes_per_commit,
                    r.bytes_per_commit,
                );
                if let Some(b) = &r.breakdown {
                    let t = b.total_us;
                    println!(
                        "    └─ commit breakdown (avg/commit): total={:.2}us  cpu={:.2}us ({:.1}%)  \
write_at={:.2}us ({:.1}%)  fsync={:.2}us ({:.1}%)",
                        t,
                        b.cpu_us,
                        CommitBreakdown::pct(b.cpu_us, t),
                        b.write_us,
                        CommitBreakdown::pct(b.write_us, t),
                        b.sync_us,
                        CommitBreakdown::pct(b.sync_us, t),
                    );
                }
                if let Some(note) = &r.note {
                    println!("    └─ {note}");
                }
            }
            println!("{}", "-".repeat(108));
        }
    }
    println!("(* = fsyncs/commit from_config, not measured — SQLite exposes no counter)");

    println!("\n=== pglet / SQLite-WAL p50 ratio (decision input; lower pglet is better) ===");
    println!(
        "{:<11} {:>7} {:>14} {:>16} {:>10}",
        "schema", "N", "pglet_p50_us", "sqlite_wal_p50", "ratio"
    );
    for (s, n, p, sq, r) in &ratios {
        println!("{s:<11} {n:>7} {p:>14.2} {sq:>16.2} {r:>10.3}");
    }

    // --- read-concurrency scaling (separate read-only micro-bench) ---
    if scenario != "commit" {
        eprintln!("  pglet read-concurrency ...");
        print_read_concurrency(&bench_concurrent_reads(&[1, 2, 4, 8], ReadMode::RootDirect));
    }

    // --- JSON output ---
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str("  \"meta\": {\n");
    json.push_str("    \"benchmark\": \"phase1_commit_latency\",\n");
    json.push_str("    \"lane\": \"native_inprocess\",\n");
    json.push_str("    \"layer\": \"L1\",\n");
    json.push_str("    \"durability_tier\": \"T1\",\n");
    let _ = writeln!(json, "    \"prepopulate_rows\": {prepopulate},");
    let _ = write!(
        json,
        "    \"schemas\": {{\"indexfree\": \"CREATE TABLE bench (id BIGINT, a BIGINT, b TEXT)\", \
\"enforcing\": \"id BIGINT PRIMARY KEY + CREATE INDEX bench_a_idx ON bench (a)\"}},\n    \"text_b_len\": {},\n",
        TEXT_B.len()
    );
    let _ = write!(
        json,
        "    \"sqlite_page_size\": {SQLITE_PAGE_SIZE},\n    \"sqlite_cache_size\": {SQLITE_CACHE_SIZE},\n"
    );
    json.push_str("    \"units\": \"microseconds\",\n");
    json.push_str("    \"higher_or_lower_is_better\": \"lower\"\n");
    json.push_str("  },\n");

    json.push_str("  \"ratios_pglet_over_sqlite_wal\": [\n");
    for (i, (schema, n, p, s, r)) in ratios.iter().enumerate() {
        let _ = write!(
            json,
            "    {{\"schema\": {schema:?}, \"batch_n\": {n}, \"pglet_p50_us\": {p:.3}, \"sqlite_wal_p50_us\": {s:.3}, \"ratio\": {r:.4}}}"
        );
        json.push_str(if i + 1 < ratios.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ],\n");

    json.push_str("  \"rows\": [\n");
    for (i, r) in rows.iter().enumerate() {
        json.push_str("    ");
        json.push_str(&r.to_json());
        json.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]\n}\n");

    // benches/phase1_results.json is the headline commit-latency evidence; written
    // only under --update (the size:check/update idiom — see `write_results`).
    write_results(&bench_results_path("phase1_results.json"), &json, update);

    println!("\n=== JSON ===");
    println!("{json}");
}
