# Tradeoffs

pglet optimizes for **size** and **startup** over feature completeness. This is
the decision log for what's cut, what's kept, and where the budget actually
stands. Every number here is measured or labeled an estimate.

## Size (measured)

| Artifact | Raw | Gzipped (wire) | Gated? |
|---|---|---|---|
| WASM (`pglet_wasm_bg.wasm`) | **~1613 KB** | ~647 KB | yes — raw bytes, under the lower of baseline +2% and the 1650 KB cap (`benches/size_baseline.json` is the recorded figure) |
| Native (`pglet_server`) | ~2705 KB | ~1289 KB | no (secondary target) |

The size **gate** is on **raw WASM bytes** (`deno task size:check`) — the size you
build, deterministic across a fixed toolchain, so a regression is real code growth
rather than compressor noise. The gzipped figure is the over-the-wire /
download cost that matters for the embedded/browser use; it's reported alongside but
not gated. **The cap is 1650 KB**, moved only by an explicit decision; `size:check` runs
second in `cargo xtask gates`, so any full gate run enforces it — still worth running
deliberately after size-growing work if you are not running the set. The native binary
is the secondary target, reported only. The per-component breakdown below is estimated
from dependency sizes — cargo-bloat doesn't support the wasm target, so a precise
breakdown would have to be hand-rolled.

### Dependency footprint

- **the SQL front-end** — pglet's own Postgres-only lexer + parser, no
  third-party parser crate. (Replacing `sqlparser` reclaimed its whole
  `recursive`/`stacker`/`psm` + C-toolchain subtree — the wasm dropped 709 → 317.5
  KB gzip.) Its only added dep is **`phf`**, a compile-time keyword perfect-hash
  table (tiny).
- **serde**, **time** (UTC only), the
  wasm-bindgen glue, postcard, thiserror, `parking_lot`, `lru` (the page cache) and
  `crc32fast` (the page/meta CRCs) — those three are the storage core's; all three ship
  natively and in the `opfs` wasm build, while the default wasm build carries `parking_lot`
  alone (`parking_lot` also backs the native server's locks) —
  plus `getrandom` (native-only: backs `gen_random_uuid()` and the wire session's secret
  key in `pglet_wire`; the wasm build, which carries no `pglet_wire`, takes randomness
  from the JS host). The `--data-dir` and gate-runner locks add no
  dep at all — `pglet_core`'s `file_lock` is `std::fs::File::try_lock`, behind a
  default-off feature so the wasm build never compiles it.

### Release profile

Already fully aggressive: `opt-level = "z"`, `lto = true`, `codegen-units = 1`,
`panic = "abort"`, `strip = true`, and **`wasm-opt -Oz` is on**. `-Oz` trades gzip for
raw — measured on the Rust 1.98.0 build at −8.1% raw and +5.6% gzip — so it stays on while
the raw gate binds and would come off if the gzip budget did. No custom allocator
(`wee_alloc` is unmaintained and leaks).

Measured and not pulled, each under 4 KB: dropping `console_error_panic_hook` from default
features (−2.2 KB; it costs the browser panic message, and the stripped profile re-adds the
feature deliberately) and `wasm-opt --converge` (−1.1 KB raw / −3.0 KB gzip at +23 s per
wasm build). Also not taken: `&dyn ColumnResolver` for the eval walk's generic resolver
(−15 107 B raw at no measured query or parse slowdown), because it cost stack — it lowered
the longest `SELECT 1+1+…` chain the wasm answered; under the statement-depth limit
(`pglet_sql::depth`) its cost is stack margin, not re-measured.

Pulled: `Plan`'s postcard **`Deserialize`** is a hand-written impl that always fails.
`Expr`'s boxed-subquery variants require the trait, but no accepted DDL stores a subquery,
and `Plan::deserialize` was the artifact's second-largest function when measured: −39 702 B
(−2.55 %). The `Serialize` half stays live for `planner::coercion`'s `structural_identity`,
which encodes a target-list entry as Postgres's `equal()` and must accept a plan-bearing one.

**The toolchain-and-flags pass** — historical: each row is the `wasm-pack` build of the row
above's tree with that row's change added, from a base of 1 687 657 B raw / 657 977 B gzip
under Rust 1.94.1, each gzip figure good to within a few bytes across clean builds. The
levers it took stand; the figures describe that pass's tree, not today's.

| Lever | Raw | Gzip |
|---|---|---|
| Rust 1.98.0 pinned (`rust-toolchain.toml`), from 1.94.1 | −120 135 B | −30 789 B |
| date/timestamp text by hand, not `time`'s `format_description!` | −9 514 B | −4 017 B |
| the wasm `Pglet` class removed; `StorageBackend::Memory` compiled out of the wasm (`pglet`'s `memory_storage` feature) | −32 639 B | −11 669 B |
| `wasm-opt --low-memory-unused` (the layout argument: `crates/pglet_wasm/Cargo.toml`) | −10 588 B | −4 674 B |
| **result** | **1 514 781 B** | **606 828 B** |

## Startup (measured)

Startup (<100 ms) is half the reason pglet exists, and it is now **measured and
met**: native spawn→ready is ~1 ms p50 across storage modes, and WASM init is
~3.3 ms on the first-ever compile, ~3.5 ms p50 for a full cold load, and ~0.5 ms
to instantiate from a cached module (`benches/wasm_init_results.json`) — all far under
the target. Those figures include the 4 MiB shadow stack (`crates/pglet_wasm/build.rs`),
whose three more MiB of initial memory measured side by side on one box at +0.17 ms to
instantiate and +0.16 ms to a full cold load. Head-to-head (`bench:wasm-compare`, pglet vs pglite vs sqlite-wasm in-memory
across Deno/Node/Bun, each engine's fresh module load through its first query — for pglet, its npm package's), pglet's cold init
is ~12–16 ms against pglite's ~1.4 s (`benches/wasm_compare_results.json`). The
harness is Deno tasks over `fuz_util`'s statistics (`benches/`; reproduced in
`benches/lib/bench_stats.ts` where it cannot be imported), not `xtask`; the Rust
`pglet_bench` crate adds the query microbench (point lookup / scan / aggregate)
alongside the rest of its scenario set (commit / reads / attribute / gin / parse /
memory — enumerated in `benches/README.md`).

## What's cut (and why)

Whole feature categories are excluded because the size/complexity cost dwarfs the
benefit for an embedded engine. Costs are **estimates** unless noted.

| Cut | Est. cost | Why |
|---|---|---|
| General regex (`~` / `!~` beyond a literal `^` prefix, `~*`, `SIMILAR TO`, `regexp_*`) | ~1.3 MB | full regex engine; `LIKE` and the `^` prefix test cover the common cases |
| Timezone database | multi-MB | tzdata is huge; UTC + fixed offsets only |
| ICU collation | multi-MB | locale-aware sort bloats the binary; binary/ASCII compare |
| General PL/pgSQL, stored procedures, triggers | 0.5–1 MB+ | interpreters/hooks; out of scope for an embedded store. One carve-out: a *minimal* `DO $$ … $$` block (the idempotent-`CREATE TYPE` migration idiom — `BEGIN <stmts>; [EXCEPTION …] END`, ~18 KB), **not** a general interpreter — everything else PL/pgSQL fails loud |
| Full-text search (`tsvector`/`tsquery`) | 0.5 MB+ | specialized; niche for embedded |
| Extensions | unbounded | opens an infinite API surface; no plugin system |
| Replication | — | single-instance engine |
| Roles / RLS | — | auth is an application-layer concern (fuz_app does authz above the DB) |

Hitting a cut feature should produce a clear error, not silent wrong behavior.

## Modularity — opt-in layers, not a plugin platform

The size budget is gated on the **minimal** build, and heavier features are meant to
land as opt-in, **compile-time-additive** layers the minimal/WASM build never pays
for: a Cargo feature compiled out costs zero bytes and zero indirection (it's
monomorphized, not dynamically dispatched). The working precedents, each by its own
mechanism:

- **`file_store`** (the std::fs `FileBlockIo`) is out of the wasm build because the
  workspace table pins `pglet` and `pglet_storage` `default-features = false` and only the
  native consumers re-add it on their dependency lines, so every native build carries it.
  The pin binds dependency edges, not packages: a member selected as a root (`cargo test
  --workspace`, the `test:rust` gate) takes its own default. The manifests' comments
  (`crates/pglet/Cargo.toml`, `crates/pglet_storage/Cargo.toml`) are the record.
- **`index_acceleration`** is forwarded through each consumer's own `default` list, so
  `--no-default-features` on a consumer drops it while `file_store` stays
  (`crates/pglet_server/Cargo.toml`). Read it off `cargo tree -p pglet_server -e normal -f
  "{p} [{f}]"` with and without the flag — the feature column is the point; `-e features`
  does not show `index_acceleration` at all.
- **`opfs`** (`pglet_wasm`'s OPFS-backed `PgletBtreePersistent` and its `BlockIo` shim) is
  additive and off by default on the leaf crate itself, so the build the npm package
  ships and `size:check` measures leaves it out, and the builds that want it name it
  (`deno task wasm:build:opfs`, for the browser example and `test:browser`). The
  `DoubleMetaStore`, page (de)serialization, the page cache and tree recovery, which only
  that binding reached, left the default build with it (read off a symbol-named build):
  measured −84 166 B raw / −48 556 B gzip on the build it was cut from.

What the acceleration drop removes is query index selection plus the
`IndexScan`/`IndexRangeScan`/`GinScan` read paths. An index is *intended* to change only a
query's speed, never its result — index objects stay built, maintained and introspectable,
PK/UNIQUE/FK enforcement is an independent probe, and a probe value substitutes only when
exactly equivalent. That equivalence is a property the accelerated read paths must uphold,
checked against a stripped build: a `@profile-lock` fixture subset on every
`test`, a stripped compile in `check`, and the full suite stripped by hand. Its measured
exceptions, the byte pair, the invocations and what the lock does and does not detect are
`docs/indexes.md` § Build profiles. The cost of the acceleration paths, as
`benches/index_accel_results.json` recorded it (on a 1 514 781 B wasm): **~2.4% of
the wasm**, and ~3.0% of the native server (a one-off measurement), startup and runtime
memory flat. It is a **size** lever, **not** a test-speed one — acceleration measured
~1.8–2.0× faster on reads from ~200 rows up — so the fixture/test audience keeps it on,
and the strip is for a size-constrained consumer that accepts slower reads.

So the heavy rows above (general PL/pgSQL, full-text search, one day richer types) are
cut from the *minimal* build, not ruled out forever — the direction is to make them
**opt-in layers**, not permanent non-goals. The row that stays a hard non-goal is
**runtime** Extensions: a plugin ABI / dynamic-dispatch registry / open type system
would cost the minimal build indirection *even when unused* (defeating
LTO/dead-code-elimination), which is the opposite axis from a compile-time feature.
A gate is only worth having if gating off removes the code — a feature gated off but
still *linked* is worse than none.

How the minimal build is kept small: the size gate
tracks the minimal profile (each layer asserts marginal-cost-when-off ≈ 0), a small
fixed set of *supported* build profiles each gets its own differential-test coverage
(the rest are "compiles, unsupported"), and core never references feature-gated code.
Today that coverage is a tagged fixture subset plus a compile check, not the full suite
per profile — the shape is in place, the breadth isn't yet.

## Float text: 18.6's digits, at a measured cost

A `float4` / `float8` is written as 18.6's `float4out` / `float8out` write it
(`pglet_core::types::float_text`), not as Rust's own shortest spelling, which differs at a
boundary, a tie, and the exponent threshold (`EDGES.md` row 255). The digits come from Rust's
shortest spelling plus an exact boundary-and-tie test in integer arithmetic; only a value that
fails the test (under one in a hundred over random values, measured) takes the longer search. Measured on the
native server's release build, 400 000 rows of random `real` and `double precision`, `psql
-At` wall time, medians of five alternating runs: Rust's shortest spelling 0.59 s, 18.6's
digits 0.66 s (1.00 s with the search alone, no test), PostgreSQL 18.4 itself 0.17 s (measured on 18.4); the encoder alone costs about 190–360 ns a value
against Rust's 70–190 ns. The npm package's default `float4` path renders this text and parses
it once per cell, so it carries the same cost. Kept: the text a client parses is 18.6's.

## JSONB objects: every key defined, at a measured cost

A jsonb object and a raw object row reach JS with every data key **defined** as an own property,
as `JSON.parse` defines it (`pglet_wasm`'s `result::Store`), not set: a set runs the prototype
chain, so `__proto__` replaced the object's prototype and a frozen `Object.prototype` silently
dropped a key naming one of its members. Measured on the release npm package under Node, 50 000
rows of a five-key jsonb read through the adapter, medians of nine, alternating builds: set
213 ms, define 275 ms (+29 %); an integer column is unchanged (44 ms). An array element is set
where that is sound and defined where a page put an index on the prototype chain, which the
store checks once per result: that check adds about 2.4 µs to every result (a one-row query
through the adapter, 11.5 → 13.9 µs); about 5 % of a fixture-shaped mix of small DDL,
parameterized inserts and point reads. A set element is also checked to have moved the array's
own `length`, which catches page code the build
itself calls (a replaced `TextDecoder.prototype.decode` or `Uint8Array.prototype.set`) changing
the chain mid-build: about 0.3 µs more a one-row result, about 3 % more on the same mix, and
nothing measurable on the 50 000-row reads. Kept: the object is the one `pg` hands back, and a
key or element that answers `false`, or stores nothing and leaves the array's `length` as it
was, fails the statement — whatever a page put on `Object.prototype` or `Array.prototype`, before
the statement or during it; page code on the chain that stores a value of its own choosing, or
moves `length` without storing, is not caught.

A result's field descriptors are not stored this way at all: each is one object literal, built
by a JS snippet the crate owns (`pglet_wasm`'s `result::field_def`), so its keys are defined —
no page member of `Object.prototype` is reached — and their names are the snippet's own strings,
never decoded out of the module's memory. Setting the eight keys one caught `Reflect.set` at a
time, each key string decoded, cost about 5.0 µs a field; the literal about 0.7 µs (a zero-row
raw read of 1 to 40 `int4` columns, the slope; decoding the key strings once instead, sets kept,
2.8 µs). A one-row read, a cell a column more, costs 0.8–0.9 µs a column with the literal and
5.0–5.4 µs with the sets.

The figures above measure the steps one at a time, against builds that differ by one step.
Every absolute figure in this section — a query's µs, a read's ms, the suite's seconds —
describes the pair of builds it was measured on, not the current build; what the section
records is the difference between each pair. What
the npm package costs the test-fixture audience as a whole, against the engine before it grew
the `pg`-shaped package (the object shape `pg` hands back, `types`, and the checks that keep a
page's prototype changes from silently altering a result): measured on release npm packages under
Node, alternating builds, two blocks on a box at load 2.5–9 — about 1 µs more a one-row query
through the adapter (about 9.6–9.9 → 10.7–11.1 µs; the raw binding's one-row read, which runs the
same once-per-result chain check, about 0.3 µs over base, and the rest of the adapter's delta not
attributed to a step), a ten-column one-row query about 10 µs *faster* (44.6–44.8 → 34.0–34.8 µs),
about 2.5–9 % more on two fixture-shaped mixes of small DDL, parameterized inserts and point reads
(+2.5 % and +6.6 % in one block, +8.0 % and +8.6 % in the other; a single round swings more than
that on this box), and fuz_app's `db` suite unchanged within noise — its total wall 0.7–2.5 %
lower and the sum of its per-test times 0.7–2.2 % lower (about 1.9 s either way; the files'
`beforeAll` hooks were not in that sum). The 50 000-row jsonb read is 25–29 % slower (the define),
and the 50 000-row integer read faster.

## Simplified behaviors (differ from Postgres)

Some of these are deliberate size tradeoffs; some are gaps not yet
closed. Both are flagged because they surprise callers either way.

| Behavior | pglet | Postgres | Status |
|---|---|---|---|
| Timezones | UTC only; `SET TIME ZONE '<zone>'` for a zone other than `UTC` refused `0A000` (`EDGES.md` row 69), a named zone in a value refused `22P02`, `AT TIME ZONE` with a non-UTC zone `0A000` | full tzdata | deliberate, and loud |
| Text collation | binary compare | ICU | deliberate |
| `TIMETZ` / unrecognized types | **fail loud** (`42704`); opt-in coerce-to-TEXT flag. DATE, TIME, INTERVAL, and NUMERIC are all real column types (indexable, comparable, casts + arithmetic; NUMERIC is an exact in-house bignum, its `NUMERIC(p,s)` enforced at the scalar column form, at a cast, and element-wise under a `numeric(p,s)[]`) | real types incl. `timetz` | deliberate (UTC-only); the `42704` is loud, and the `<type>(n)[]` and discarded-precision swallows are closed (`EDGES.md` rows 120 and 123) — a sub-second `(p)` is rounded and enforced at a cast and at a column store alike, `'03:04:05.6789'::time(3)` reading `03:04:05.679` and `'1.23456 secs'::interval(3)` reading `00:00:01.235` on both engines. Residuals a caller still designs around: a rounding carry past year 9999 is `22008 timestamp out of range` here against 18.6's `10000-01-01 00:00:00`, and the `WARNING` 18.6 sends when it clamps `p > 6` to `6` is not sent — row 123 carries the rest |
| Error fields | the structured fields 18.6 sends (`detail`, `hint`, `position`, the object fields) where pglet can fill them truthfully, the rest omitted (`docs/errors.md`) | all of them | deliberate: an omitted field is a gap a client can see, an invented one is a wrong answer |

## Concurrency (vs SQLite, Postgres)

pglet is **single-writer + lock-free MVCC snapshot reads**: one connection's block
stages writes at a time — it takes the writer at its first write and holds it until it
ends, an autocommit write holds it for its statement — and readers capture the root
atomically and traverse an immutable CoW tree (no locks, snapshot isolation). The closest
cousins are LMDB and SQLite's WAL mode (below). A second writer waits for the writer (the
native server, bounded by `lock_timeout`, `55P03` past it) or is refused `55P03` (the embedded
`Database`, which cannot wait), and a block that takes it is re-based on what is committed
then (`docs/storage.md` § Why Single-Writer?).

- **vs SQLite (rollback-journal, the default):** ahead — there readers and writers
  block each other; pglet's readers block neither the writer nor each other.
- **vs SQLite WAL mode:** the same class — WAL also gives readers a consistent snapshot
  concurrent with one writer, and holds its writer lock from a transaction's first write
  to its end, as pglet's writer is held. SQLite's busy timeout is pglet's `lock_timeout`.
  pglet's advantages are lock-free reads (no WAL-index read-mark), a single file
  (no `-wal`/`-shm` sidecars), and no checkpoint stalls; it pays instead in CoW
  write-amplification and old-version retention until GC.
- **vs Postgres:** behind — Postgres is multi-writer MVCC and locks rows; pglet (like
  SQLite) is single-writer, so unrelated writes serialize and a leaked written block
  stalls every writer until the bound. Acceptable for the audiences pglet serves (a test
  harness or a gateway is normally the one writer) — and single-writer is **permanent**: the CoW/root-versioned design is built around
  it (commit path, page layout).

## What pglet is good for today

Fast ORM/SQL test fixtures (spin up far quicker than Docker Postgres), in-process SQL in
the browser or Node from a small WASM, prototypes and scratch space. It is
**not** good for anything you can't lose — see `PLAN.md` § Status.

## Migration

Because pglet speaks the Postgres wire protocol, moving to real Postgres is a
connection-string change plus testing for the simplified behaviors above
(AVG type, UTC-only timezones, any features you worked around). The reverse —
pointing real Postgres SQL at pglet — can fail wherever `docs/compatibility.md`
says 🔲 or ⚠️, and at the exotic residuals its ✅ rows' records still name.
