//! Memory-footprint measurement (the "memory" scenario).
//!
//! A **pglet-internal, longitudinal** metric — "did this change raise peak?" —
//! *not* a cross-engine ratio (a pglet/pglite memory comparison is meaningless:
//! pglite is whole-Postgres and WASM linear memory only grows). This file owns
//! the **native** surface: peak resident-set size (RSS) of the embed
//! `Database` over the file-backed double-meta store (the CoW B+tree + its LRU
//! page cache), across the same schema axes (`indexfree` / `enforcing`) and a
//! dataset sweep the write/query benches use, so the numbers sit next to those
//! baselines. The **WASM** surface (linear-memory high-water) is measured by
//! `benches/memory.ts`, which also merges both into `benches/memory_results.json`.
//!
//! **Method — "rough is fine" (the discipline's word).** Peak RSS via the
//! kernel's own `/proc/self/status` `VmHWM` high-water — no custom global
//! allocator (that's the deferred allocation-instrumentation), no `getrusage`
//! FFI (which would need `unsafe`), no new dependency. Because `VmHWM` is
//! monotonic over a process's life, one shared process would report the max
//! across all workloads; so the parent **spawns a fresh child per workload**
//! (`mem-child <schema> <rows>`) and each child reports its own baseline `VmRSS`
//! (before opening the DB) and final `VmHWM` (after the load). The delta
//! subtracts the fixed Rust/binary baseline, leaving the engine's incremental
//! footprint. The resulting **on-disk file size** rides along as context — it
//! grows with the dataset while the cache-bounded RSS should not, which is the
//! size-thesis check.
//!
//! Linux-only (reads `/proc`); a non-Linux run reports the kilobyte fields as 0
//! rather than failing.

use std::path::Path;
use std::process::Command;

use crate::common::{BenchSchema, open_pglet, prepopulate_pglet};

/// The dataset sweep (rows loaded), smallest first. Two sizes make dataset
/// scaling visible: a cache-bounded engine should hold RSS roughly flat while
/// the on-disk file grows. Fixed + repeatable — memory is longitudinal, so the
/// workload can't drift between runs.
const ROW_SIZES: &[u64] = &[10_000, 100_000];

/// One measured (schema, dataset) cell.
#[derive(Debug, Clone)]
struct MemRow {
    schema: &'static str,
    rows: u64,
    /// `VmRSS` sampled at child start, before the DB is opened — the fixed
    /// process/runtime baseline to subtract.
    baseline_rss_kb: u64,
    /// `VmRSS` after the load, DB still live — the steady-state resident set. A
    /// value well below `peak_rss_kb` says the peak was transient load churn
    /// (CoW page building + allocator high-water), not a retained working set.
    current_rss_kb: u64,
    /// `VmHWM` after the load — the peak resident set the process ever reached.
    peak_rss_kb: u64,
    /// On-disk double-meta file size after the load (context: this scales with
    /// the dataset while `peak_rss_kb` should stay cache-bounded).
    file_size_kb: u64,
}

impl MemRow {
    /// Engine footprint above the process baseline (`peak − baseline`), clamped
    /// at 0 (a load that never exceeds baseline reports 0, not underflow).
    const fn delta_kb(&self) -> u64 {
        self.peak_rss_kb.saturating_sub(self.baseline_rss_kb)
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"engine\":\"pglet\",\"surface\":\"native_embed\",\"schema\":{:?},\
\"rows\":{},\"baseline_rss_kb\":{},\"current_rss_kb\":{},\"peak_rss_kb\":{},\"delta_kb\":{},\
\"file_size_kb\":{},\"units\":\"kilobytes\",\"higher_or_lower_is_better\":\"lower\"}}",
            self.schema,
            self.rows,
            self.baseline_rss_kb,
            self.current_rss_kb,
            self.peak_rss_kb,
            self.delta_kb(),
            self.file_size_kb,
        )
    }
}

/// Read a `/proc/self/status` numeric kilobyte field (e.g. `VmHWM`, `VmRSS`).
/// Returns `None` off Linux or if the field is absent — the caller degrades to
/// 0 rather than aborting the sweep.
fn read_status_kb(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            // Line shape: "VmHWM:\t   12345 kB" — strip the colon, take the number.
            let rest = rest.trim_start_matches(':').trim();
            let kb = rest.split_whitespace().next()?;
            return kb.parse().ok();
        }
    }
    None
}

/// Child entry: load one (schema, rows) dataset into a fresh file-backed engine
/// and print one TSV line — `schema<TAB>rows<TAB>baseline_kb<TAB>peak_kb<TAB>file_kb`
/// — to stdout. TSV (not JSON) so the parent parses it without a JSON dep. Called
/// from `main` for the `mem-child` scenario.
pub fn run_mem_child(schema_label: &str, rows: u64) {
    let schema = BenchSchema::from_label(schema_label);

    // Baseline first — before any engine allocation touches pages.
    let baseline_rss_kb = read_status_kb("VmRSS").unwrap_or(0);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mem.pglet");
    let (mut db, _counters) = open_pglet(&path);
    for stmt in schema.create_stmts() {
        db.execute(stmt).expect("create schema");
    }
    // Chunked BEGIN/COMMIT load (setup shape shared with the write benches); each
    // chunk commit persists (flush + meta-flip), so the file reflects the dataset.
    let _ = prepopulate_pglet(&mut db, 1, rows);

    let file_size_kb = std::fs::metadata(&path).map_or(0, |m| m.len() / 1024);
    // Read both while `db` is still live, so its working set is resident:
    // current VmRSS (steady state) and VmHWM (peak over the load).
    let current_rss_kb = read_status_kb("VmRSS").unwrap_or(0);
    let peak_rss_kb = read_status_kb("VmHWM").unwrap_or(0);
    std::hint::black_box(&db);

    println!(
        "{schema_label}\t{rows}\t{baseline_rss_kb}\t{current_rss_kb}\t{peak_rss_kb}\t{file_size_kb}"
    );
}

/// Parse a child's TSV line back into a `MemRow`. Malformed output is dropped
/// (returns `None`) rather than poisoning the sweep.
fn parse_child_line(line: &str) -> Option<MemRow> {
    let mut f = line.split('\t');
    let schema = BenchSchema::from_label(f.next()?).label();
    let rows = f.next()?.trim().parse().ok()?;
    let baseline_rss_kb = f.next()?.trim().parse().ok()?;
    let current_rss_kb = f.next()?.trim().parse().ok()?;
    let peak_rss_kb = f.next()?.trim().parse().ok()?;
    let file_size_kb = f.next()?.trim().parse().ok()?;
    Some(MemRow {
        schema,
        rows,
        baseline_rss_kb,
        current_rss_kb,
        peak_rss_kb,
        file_size_kb,
    })
}

/// Spawn `mem-child <schema> <rows>` and collect its parsed `MemRow`. Uses the
/// current executable directly (not `cargo run`), so the child is a plain binary
/// exec with a clean, isolated address space.
fn measure_cell(exe: &Path, schema: BenchSchema, rows: u64) -> Option<MemRow> {
    let out = Command::new(exe)
        .args(["mem-child", schema.label(), &rows.to_string()])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().find_map(parse_child_line)
}

fn rows_json_array(rows: &[MemRow]) -> String {
    let mut json = String::from("[\n");
    for (i, r) in rows.iter().enumerate() {
        json.push_str("    ");
        json.push_str(&r.to_json());
        json.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]");
    json
}

fn print_table(rows: &[MemRow]) {
    println!("\n=== native embed engine — peak RSS (longitudinal, watch-tier) ===");
    println!(
        "file-backed double-meta (CoW B+tree + LRU page cache); load-only workload; \
peak = /proc VmHWM, baseline = VmRSS at child start; per-workload fresh process"
    );
    println!(
        "{:<11} {:>9} {:>12} {:>11} {:>10} {:>10} {:>11}",
        "schema", "rows", "baseline_kb", "current_kb", "peak_kb", "delta_kb", "file_kb"
    );
    println!("{}", "-".repeat(80));
    for r in rows {
        println!(
            "{:<11} {:>9} {:>12} {:>11} {:>10} {:>10} {:>11}",
            r.schema,
            r.rows,
            r.baseline_rss_kb,
            r.current_rss_kb,
            r.peak_rss_kb,
            r.delta_kb(),
            r.file_size_kb,
        );
    }
    println!("{}", "-".repeat(80));
    println!(
        "(kilobytes; peak/delta are pglet-internal tripwires — compare to a prior local run, \
never a committed absolute; on-disk file grows with the dataset while cache-bounded RSS should not)"
    );
}

/// The `memory` scenario (native surface): spawn one child per (schema, rows)
/// cell, collect peak RSS. `emit_json` prints ONLY the JSON array to stdout (for
/// the `benches/memory.ts` orchestrator, which owns `memory_results.json`);
/// otherwise a human table is printed. This mode never writes the results file —
/// the Deno orchestrator merges native + WASM and is the single writer.
pub fn run_memory(emit_json: bool) {
    let exe = std::env::current_exe().expect("current_exe");
    if !emit_json {
        eprintln!(
            "pglet_bench: memory footprint (native embed, peak RSS). \
schemas=[indexfree, enforcing]; rows={ROW_SIZES:?}; per-workload fresh process"
        );
    }

    let mut rows: Vec<MemRow> = Vec::new();
    // Smallest dataset first so a partial/interrupted run still yields the cheap cells.
    for &n in ROW_SIZES {
        for &schema in crate::common::BENCH_SCHEMAS {
            if !emit_json {
                eprintln!("  memory schema={} rows={n} ...", schema.label());
            }
            if let Some(row) = measure_cell(&exe, schema, n) {
                rows.push(row);
            }
        }
    }

    if emit_json {
        // Machine-readable output: just the array, nothing else on stdout.
        print!("{}", rows_json_array(&rows));
    } else {
        print_table(&rows);
    }
}
