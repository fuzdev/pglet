//! Native in-process benchmark harness for pglet.
//!
//! Scenarios, mostly over pglet's double-meta storage:
//! - **commit** — per-commit wall-clock latency across a batch-size sweep at
//!   durability tier "T1", compared against SQLite (bundled, compiled from
//!   source), over two schema axes: `indexfree` (the optimistic ceiling — no
//!   PK, no index) and `enforcing` (a PRIMARY KEY probed per insert + a
//!   secondary index — the real write cost). Also reports fsyncs / writes /
//!   bytes per commit (write amplification) and a write/fsync/CPU breakdown.
//! - **reads** — read-concurrency scaling: cache-warm point-gets across N
//!   threads, isolating the sharded per-page cache (no SQLite comparison).
//! - **query** — single-threaded read-path latency vs SQLite: point lookup,
//!   full scan, and aggregate over a prepopulated table with no secondary index
//!   (every engine answers by sequential scan, so it stays apples-to-apples).
//! - **gin** — single-threaded `refs @> ARRAY['tok']` containment latency,
//!   GIN-accelerated vs full-scan (pglet-vs-pglet: SQLite has no `@>`/GIN). The
//!   same data and probes run with no index then with `USING gin(refs)`.
//! - **parse** — front-end (parse / plan / parse+plan) latency over a
//!   representative query set from fuz_app's SQL (pglet-only, no storage); the
//!   front-end baseline `parse_results.json` records.
//! - **attribute** — decompose the batched-commit CPU (most of what `commit`
//!   measures) into named sub-phases: apply loop (descent + CoW clones),
//!   `Page::serialize` (postcard), `page_checksum` (CRC32), `page.clone()`, and
//!   `io.write_at`, over both schema axes (`indexfree` / `enforcing`) so the
//!   index-maintenance cost is a visible delta. Bench-side micro-timing over the
//!   public `Page`/store APIs plus the `CountingBlockIo` counters — no
//!   production-path instrumentation.
//! - **memory** — peak RSS of the native embed over the file-backed store, one child
//!   process per workload (`memory.rs`; `benches/memory.ts` adds the wasm side).
//!
//! Deliberately minimal — no WASM, no pglite, no server-Postgres, no criterion;
//! real fsyncs, real temp files, and our own median/percentile/CV math.
//!
//! Usage: `cargo run --release -p pglet_bench [-- [commit|reads|attribute|query|gin|parse|memory] [--update]]`.
//! A plain run prints to stdout; `--update` refreshes the scenario's committed results
//! file under `benches/` (`commit` writes `phase1_results.json`). Env knobs: `PGLET_BENCH_PREPOPULATE` (setup
//! rows), `PGLET_BENCH_CACHE` (page-cache size).

// A dev-only bench binary: it prints results, and setup-phase failures (open
// store, run a fixed insert) are unrecoverable — `expect`/`unwrap` there is the
// intended panic, not a code smell to thread `Result` through.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

mod commit;
mod common;
mod gin;
mod memory;
mod parse;
mod query;
mod reads;
mod stats;

fn main() {
    // Flags mirror the repo's `size:check`/`size:update` idiom: a plain run prints
    // results and leaves the committed results files untouched; `--update` refreshes
    // the scenario's. A named scenario runs alone; with none, `commit` runs and then the
    // read-concurrency half of `reads`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let update = args.iter().any(|a| a == "--update");
    // Positional (non-flag) args, in order: [scenario, then scenario operands].
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let scenario = positional.first().map_or("all", |s| s.as_str());

    if scenario == "memory" {
        // Parent orchestrator: spawns a child per workload. `--emit-json` prints
        // just the JSON array (for benches/memory.ts); else a human table.
        memory::run_memory(args.iter().any(|a| a == "--emit-json"));
        return;
    }

    if scenario == "mem-child" {
        // Hidden child: `mem-child <schema> <rows>` — load one dataset, print its
        // peak-RSS TSV line. Spawned by the `memory` parent, one per workload.
        let schema = positional.get(1).map_or("indexfree", |s| s.as_str());
        let rows: u64 = positional
            .get(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10_000);
        memory::run_mem_child(schema, rows);
        return;
    }

    if scenario == "reads" {
        eprintln!("  pglet read-concurrency ...");
        reads::run_reads(update);
        return;
    }

    if scenario == "attribute" {
        commit::run_attribution(update);
        return;
    }

    if scenario == "query" {
        query::run_query(update);
        return;
    }

    if scenario == "parse" {
        parse::run_parse(update);
        return;
    }

    if scenario == "gin" {
        gin::run_gin(update);
        return;
    }

    commit::run_commit(update, scenario);
}
