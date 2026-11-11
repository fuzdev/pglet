# benches

pglet's benchmark harness. Watch-tier: every bench **prints** by default and
leaves the committed `*_results.json` untouched; pass `--update` (or the
`:update` task) to refresh the recorded numbers — the same `size:check` /
`size:update` idiom used across the repo. Only `size:check` gates; the rest are
evidence, not CI.

Two lanes:

- **Native, in-process** — the rigorous lane. Rust `pglet_bench`
  (`../crates/pglet_bench/`) times commit latency, read-concurrency, and
  commit-CPU attribution, and single-threaded query/scan latency, with **SQLite
  (`rusqlite`, bundled) as the baseline**. Real fsyncs, real temp files. See
  `bench:commit`, `bench:reads`, `bench:attribute`, `bench:query` — plus the
  pglet-vs-pglet modes `bench:gin` (GIN vs full-scan containment) and
  `bench:parse` (fused parse+plan latency).
- **WASM, in-memory** — the rough-figures lane. `wasm_compare.ts` compares
  **pglet vs pglite vs sqlite-wasm** on startup + basic query perf (see below).

**Memory** is the exception to the two-lane split: it is a **pglet-internal,
longitudinal** metric — "did this change raise peak?" — spanning *both* surfaces
(native peak RSS + WASM linear-memory high-water), and it is deliberately **not**
a cross-engine ratio (pglite is whole-Postgres and WASM linear memory only grows,
so a pglet/pglite memory comparison is meaningless). See `bench:memory` below.

Every script here is a plain Node script, run as `node benches/<name>.ts` (Node strips the
types); `wasm_compare.ts` runs under Bun as well. None runs on gro's loader, so nothing a
bench times in its own process, or in a child it starts, carries gro's startup or module
graph: the children (`index_accel.ts --measure`, `memory.ts --wasm-child`) are this
process's own `node` with no environment. Paths are found from each file, never the
working directory, so a hand-run works from anywhere: `node benches/startup.ts`. Bare
specifiers resolve as Node resolves them, through this directory's `package.json` and
`node_modules` (pglite, sqlite-wasm) and then the root's (`@fuzdev/fuz_util`).
`npm run check:benches` typechecks them (`tsconfig.json`, inside `npm run check`).

The size gate is the exception: a gro task (`src/tasks/size.task.ts` over
`src/lib/size_gate.ts`), since it gates rather than measures time. It and `lib/results.ts`
stamp results by the one rule in `src/lib/bench_results.ts`.

## Files

| File | What |
|---|---|
| `wasm_compare.ts` | cross-runtime in-memory WASM compare (pglet / pglite / sqlite-wasm) |
| `wasm_init.ts` | pglet's own WASM init phases (compile / construct / first query) |
| `memory.ts` | memory footprint — native peak RSS + WASM linear-memory high-water (orchestrates the `pglet_bench memory` mode; single writer of `memory_results.json`) |
| `exit_proof.ts` | fuz_app `db`-suite wall-time per engine leg — four legs: pglite / pglet-wasm (L1, in-process) and pglet-server / pg (L2, out-of-process), never compared across the line but by the one `ratios` entry marked `crosses_tiers`; records the README's multiples under `ratios`, `null` with a `null_reason` over a failed or unselected leg or two legs that passed different test counts (discovery refuses, before any leg runs, a fuz_app file mixing a single-driver suite into its four-leg ones), and under `engines` what the legs ran — the pglite fuz_app's install resolves (not `benches/`'), the npm package's version, fuz_app's commit (`bench:exit-proof`; single writer of `exit_proof_results.json`) |
| `startup.ts` | native server spawn→ready latency |
| `index_accel.ts` | index-acceleration ON vs OFF (wasm): size delta + read-latency ratio across table sizes (`bench:index-accel`; builds both profiles) |
| `size_compare.ts` | pglet-vs-pglite runtime payload, raw and gzip — the npm package's shipped wasm against pglite's wasm + data + initdb wasm at the lockfile's version; refuses a stale package or a pglite off the lockfile (`bench:size-compare`; single writer of `pglite_size_comparison.json`) |
| `lib/bench_stats.ts` | zero-dependency median/p95/CV stats, shared by the cross-runtime benches |
| `lib/results.ts` | `git_commit` + `write_json` — the envelope every recorded `*_results.json` carries — and `package_version`, an engine's; the stamp is `<commit>-dirty` when a tracked file is modified or an untracked one sits under `benches/`, `crates/` or `scripts/` (the recorded results files themselves excluded), `<commit>-dirty?` when that could not be read, and for another checkout (`git_commit(dir)`, fuz_app's) `-dirty` on any change. A `size:update` run before the change it measures is committed stamps `size_baseline.json` `<parent>-dirty` |
| `lib/exit_proof_ratios.ts` | the ratios `exit_proof.ts` records — which legs, which bucket, and the keys `cargo xtask measure`'s figures list reads |
| `lib/gzip_size.ts` | the gzip measurement (`gzip -c`, default level, the size gate's arguments too) behind `size_compare.ts` and `wasm_init.ts` |
| `lib/pglet_package.ts` | the npm package's location and the one rule for whether it is current, shared by `wasm_compare.ts` and `size_compare.ts` |
| `lib/pglite_install.ts` | which pglite a bench loads (resolved as Node resolves the import) and the one rule for whether `benches/` holds the lockfile's — applied by `wasm_compare.ts` and `size_compare.ts`, and asked by `cargo xtask measure`'s preflight before any timed step |
| `package.json` | `private`, dev-only npm deps for `wasm_compare.ts` (pglite, sqlite-wasm) and `size_compare.ts` (pglite) |
| `*_results.json` | committed evidence per bench |
| `pglite_size_comparison.json` | pglite-vs-pglet payload sizes and their ratios, written by `size_compare.ts` — the evidence behind the README's size ratio |
| `package-lock.json` | the lockfile for `package.json`'s npm deps |

## `wasm_compare.ts` — cross-runtime WASM compare

In-memory only — every engine runs in its no-persistence mode (pglet
`create_pglet_db()`, pglite ephemeral `new PGlite()`, sqlite `:memory:`), so the
numbers isolate **engine startup + query CPU** from OPFS / fsync / disk. Runs
**headless** (no browser) under **Node and Bun** — the same `.ts` file,
unmodified — so the figures get cross-engine (V8 vs JavaScriptCore) context.

Measured per engine: cold init (one-shot startup, module load through the first
query), bulk insert of N rows, point lookup, full scan, and an aggregate.

pglet has two rows, both over the npm package the task chain builds
(`npm run build:npm` → `crates/pglet_wasm/pkg-npm`):

- **`pglet`** — the package as a consumer loads it: its Node entry (`index.js`,
  which a bare `@fuzdev/pglet-wasm` import resolves to under both runtimes),
  `create_pglet_db()`, and `await db.query(sql)`'s `.rows`, the adapter's coerced
  object rows. This is the row a pglet figure quoted elsewhere comes from.
- **`pglet-raw`** — the same package's raw `PgletBtree` binding
  (`db.query(sql).rows`), read from the package's internal `pglet_wasm.js` (it is not an
  export), kept so its row path stays visible. A
  process holds one instance of the package's wasm module, so it runs on the
  instance the `pglet` row loaded: no cold init of its own (`—`, recorded
  `null`), and its one-shot bulk insert runs on an instance already warm.

A missing `pkg-npm`, or one older than any file under `crates/`, `scripts/`, `src/` or
the workspace manifests, stops the run before anything is measured, as does a pglite
other than the one `package-lock.json` pins (`lib/pglite_install.ts`). A failed
engine row is printed, and the run then exits 1 without writing.

> **Caveat (also printed by the bench).** pglet's WASM is built `opt-level = "z"`
> + LTO — tuned for *size*, not query speed. Read a
> slow pglet query number as "the build is small," not "the engine is slow."
> pglite is full Postgres compiled to WASM (startup-heavy, over a second cold);
> sqlite-wasm is the official SQLite build. Single-process, in-memory, rough
> figures — rigorous throughput is the native `pglet_bench`-vs-rusqlite lane.
>
> Cold init is a one-shot per run, so the absolute number is noisy (OS/JIT cache
> warmth); the *relative* ordering across engines is the signal.

### Setup

pglite + sqlite-wasm are dev-only deps in this directory's `package.json`
(`private`, never shipped — isolated from the workspace's tight runtime-dep
budget). Populate `benches/node_modules` once:

```bash
npm run bench:wasm-compare:install   # npm install --prefix benches
```

### Run

```bash
npm run bench:wasm-compare           # build the npm package once, then run under both (the normal one)
npm run bench:wasm-compare:node      # Node only
npm run bench:wasm-compare:bun       # Bun only
npm run bench:wasm-compare:update    # both, each merges its slice into wasm_compare_results.json
```

The two-runtime run is **fail-fast**: a missing runtime stops the chain (it assumes
both are installed). `wasm_compare_results.json` keys each runtime
(`node` / `bun`) separately, recording its version + the detected
engine versions, so a single-runtime `--update` refreshes only its own slice.

Knobs (env): `PGLET_BENCH_ROWS` (default 10000), `PGLET_BENCH_SAMPLES` (50),
`PGLET_BENCH_WARMUP` (5).

## `memory.ts` — footprint (pglet-internal longitudinal)

Peak memory across both surfaces, over the same `indexfree` / `enforcing` ×
`{10k, 100k}` **load-only** workload the write benches use (so the numbers sit
next to those baselines). "Rough is fine": the kernel/engine **high-water**, no
custom global allocator (the deferred allocation-instrumentation) and no new dep.

- **native embed** — peak RSS of the file-backed double-meta engine (CoW B+tree +
  LRU page cache) via `/proc/self/status` `VmHWM`. Owned by the Rust
  `pglet_bench memory` mode, which spawns a **fresh child process per workload**
  (the high-water is monotonic, so per-workload isolation needs a cold process)
  and also reports the baseline `VmRSS` (subtract the fixed binary/runtime floor)
  and the on-disk file size. Runnable bare — `cargo run -p pglet_bench -- memory`
  — without Node.
- **WASM in-memory** — linear-memory high-water
  (`initSync(…).memory.buffer.byteLength`) after the load into `PgletBtree` (the
  fuz_app wasm leg's in-memory engine). Linear memory never shrinks, so each
  workload runs in a **fresh subprocess** (cold module).

`memory.ts` orchestrates both (relaying the native mode's `--emit-json`) and is
the **single writer** of `memory_results.json` (native + wasm sections), mirroring
`wasm_compare.ts`.

```bash
npm run bench:memory          # print both surfaces
npm run bench:memory:update   # ...and refresh memory_results.json
cargo run -p pglet_bench -- memory   # native surface only, no Node
```

> **Longitudinal, watch-tier.** Read each number against a **prior local run's
> delta**, never the committed absolute (perf memory isn't a gated baseline — only
> `size:check` gates). `current≈peak` on the native surface is expected — glibc
> retains freed pages, so the peak is the load's *allocation* high-water, not a
> leak. These are runtime-memory numbers, **separate from the binary size figures**.
