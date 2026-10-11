# pglet

> fast and small embeddable postgres for speeding up your tests

pglet is a Postgres-compatible-ish database engine in Rust, built to start fast and stay small.
It speaks the Postgres wire protocol and a growing subset of the Postgres SQL dialect,
and runs in-process as WebAssembly ([`@fuzdev/pglet-wasm`](https://www.npmjs.com/package/@fuzdev/pglet-wasm))
or as a native server you build from source —
well under a megabyte over the wire.

**Today it's for tests.** A fresh database costs milliseconds, not a container boot
or pglite's second-plus init, and `fork()` branches a migrated, seeded database
into an isolated copy per test without re-running the setup SQL.
On [fuz_app](https://github.com/fuzdev/fuz_app)'s database tests,
pglet runs ~6.3x faster than pglite
at ~1/8 the gzipped size of pglite's wasm and data files (wasm-to-wasm, in memory),
and ~5.9x faster than a real PostgreSQL server
(pglet in-process versus Postgres over TCP; server-to-server it's ~4.0x) —
counting the time spent running the tests; vitest's whole run is ~4.4x faster than on pglite.

**Later, embedded and everywhere else.** The same engine is meant to grow into
a local-first, file-based store for apps and browsers —
it already persists to a file (or to OPFS in the browser, in an opt-in build) —
but you should not trust pglet with your production data, not yet.
Tests don't need this trust, they just gotta go fast.

pglet's position is that reimplementing Postgres is hubris -
PostgreSQL is more than its code,
it's a community of people with the collective design and implementation power
to maintain the software and steward its design.
Conformance is pglet's goal, and everything pglet supports
is differential-tested against a running PostgreSQL oracle.
Much of PostgreSQL is not supported yet, and what isn't aims to fail loudly
(so it should be obvious when pglet is at fault).
The strategy from here is to fill out the PostgreSQL featureset as optional layers,
measuring everything along the way —
a build carries only what its consumer needs,
so browsers and other size-sensitive hosts can take trimmer builds
that trade compatibility for file size and startup time.

pglet is derived from PostgreSQL's behavior by LLM;
it's a high-effort project but it may stink to high heaven, in spots.
It was made using the [fuz-stack skill](https://docs.fuz.dev/skills/fuz-stack),
which bakes in a lot of human opinions about good Rust projects and patterns.
pglet is as close to a "vibe coded" project as [@fuzdev](https://github.com/fuzdev) has -
a human did think through the high level architecture and design in detail
but as a dev-user not a db designer/developer.
The internals are LLM-generated from a running PostgreSQL and
the [official postgres](https://github.com/postgres/postgres) docs
in sometimes-slop unevenly-reviewed Rust.
This release is an early proof of concept:
speeding up fuz_app's tests makes pglet plausibly-real software,
so it's public despite being full of holes that your workloads may fail on.

## What it's for

pglet is for the places you wouldn't run a real Postgres server: test suites,
browsers, and embedded/local-first apps. It is not a replacement for upstream
PostgreSQL behind an app server and never will be — it's the same dialect made
cheap enough to start that a database can be a throwaway value.

Two audiences, on two clocks:

- **A fast test-fixture engine — this release.** pglet starts in milliseconds
  where pglite takes over a second (and Docker Postgres pays a container boot on top),
  so ORM/SQL suites get a real Postgres-dialect backend per test without the
  boot cost. The copy-on-write B+tree makes per-test isolation nearly free:
  `Database::fork()` (the in-repo Rust embed, not published) and the npm adapter's `fork()` branch a database's
  committed state into an isolated copy that shares seeded rows until written —
  migrate and seed a base once, then fork a fresh database per test instead of
  re-running the setup SQL (see `docs/storage.md` § Fork).
- **An embedded engine for production data — medium-term.** The same engine as
  a local-first, file-based store, once it earns the system-of-record trust bar
  (accrued maturity and soak time; the mechanism is built and torture-tested).
  It was born as the embedded backend for [fuz](https://github.com/fuzdev)'s
  data layer, and fuz_app's test suite is the production-shaped consumer it's
  proven on today.

The conformance strategy: **aim at full PostgreSQL conformance over time, and
make it optional.** Features land as compile-time-additive layers, so a build
carries only what its consumer needs and artifacts stay as small as today's — or
slimmer, when a dev doesn't need a feature (index acceleration is the first
working lever). Every feature that lands is differential-tested against a live
PostgreSQL, and whatever isn't implemented aims to fail **loud** rather than
approximate — `EDGES.md` records the exceptions found so far.

## Quick start

The npm package is the primary artifact —
[`@fuzdev/pglet-wasm`](https://www.npmjs.com/package/@fuzdev/pglet-wasm), an
in-process wasm database with a `pg`/PGlite-shaped adapter:

```bash
npm i @fuzdev/pglet-wasm
```

```js
import {create_pglet_db} from '@fuzdev/pglet-wasm';

const db = create_pglet_db();
await db.query('CREATE TABLE users (id BIGINT PRIMARY KEY, name TEXT)');
await db.query('INSERT INTO users VALUES ($1, $2)', [1, 'ada']);

// the test-fixture idiom: migrate + seed once, fork a fresh DB per test
const per_test = await db.fork(); // cheap CoW branch; writes are isolated
```

The [package README](crates/pglet_wasm/npm/README.md) has the full adapter API —
transactions, value coercion, and the kysely and drizzle recipes. (On Node the wasm
auto-initializes at import; in a browser/bundler build call `await init()`
first — the package README shows both.)

The native server speaks the Postgres wire protocol, so `psql` and ordinary
drivers connect:

```bash
# default storage is B+tree, in-memory (MVCC, no persistence)
cargo run -p pglet_server
psql -h 127.0.0.1 -p 5433 -U postgres

# persistent — but see the durability caveats below
# Takes an exclusive lock on the data dir: one process per --data-dir.
cargo run -p pglet_server -- --data-dir .pglet/data
```

The server listens on `127.0.0.1` only and accepts any credentials — run it on a trusted
machine. Each connection thread reserves 64 MiB of address space (512 MiB in a debug build);
a connection no thread can be spawned for is refused, as Postgres refuses one it cannot fork.

**Statement depth is bounded per host, and a statement past the bound is refused
`54001`** rather than overflowing the stack: 10 000 levels on a server connection thread,
2 000 in wasm, 2 600 (256 in a debug build) in the Rust embed on a thread whose stack it
knows. A store written under a larger bound is refused at `Database::open` on a thread
with a smaller one — export it with `pglet_server dump --data-dir <dir>`. The embed's knobs
are the `pglet` crate docs' (`set_thread_stack`, `set_thread_max_depth`); `EDGES.md` rows
171, 178, 179 and 182 have what each host answers and what can still abort.

For the raw wasm binding and OPFS persistence in the browser — an opt-in `opfs` build of
`crates/pglet_wasm`, which the npm package does not ship — see `examples/browser/`.

## Status: early, and not safe for real data

This is a prototype. Do not put anything you can't lose in it yet.

**What works, and with what caveats, is [`docs/compatibility.md`](docs/compatibility.md)**
— one line per feature, each linked to its full record. In outline: the SELECT /
INSERT / UPDATE / DELETE core with nested-loop joins, aggregates, subqueries (including
correlated `EXISTS`), CTEs — data-modifying and `WITH RECURSIVE` — set operations,
`DISTINCT ON` and window ranking; the column types bool / int / float / text / bytea /
uuid / timestamp / date / time / interval / numeric / jsonb / enum and 1-D arrays of
them; real prepared statements with `$N` type inference; transactions with savepoints
and Postgres's aborted-block semantics, one writing block at a time across connections
(below); `COPY … FROM STDIN` / `TO STDOUT`; a synthesized `information_schema` and a
partial `pg_catalog`; a `pg_dump … | psql` restore in and a portable dump out; and WASM in
the browser, with OPFS persistence as an opt-in build the npm package does not ship.

**Constraints are enforced at write time** — NOT NULL (`23502`), CHECK (`23514`),
PK / UNIQUE (`23505`, via auto-created unique indexes covering composite, partial,
expression-keyed and `NULLS NOT DISTINCT` shapes), and FK (`23503`, single-column
and composite, with both `ON DELETE` and `ON UPDATE` actions) — each decided against a
state no other connection is moving, since one connection's block writes at a time
(below).

**Concurrency** is many lock-free MVCC snapshot readers plus a single writer, by design
(the closest cousins are SQLite and LMDB) — not a stopgap toward multi-writer. A block
takes the writer at its first write and holds it until it ends; an autocommit write holds
it for its statement. On the native server a second writer waits, bounded by
`lock_timeout` (pglet's default `1s`, not PostgreSQL's `0`), then answers `55P03`; the
embedded `Database`, which cannot wait, refuses `55P03` at once. What
this costs against PostgreSQL 18.6 (the version the differential fixtures assert against,
"18.6" below and in the docs) — unrelated writes serialize, a `REPEATABLE READ` /
`SERIALIZABLE` block's first write is `40001` whenever the store moved past its snapshot,
another connection's read during a block's uncommitted `ADD COLUMN` sees a mismatched row
width (`EDGES.md` row 221) — is `docs/storage.md` § Isolation Guarantees. `READ ONLY` is
refused (`0A000`) rather than ignored; the `default_transaction_read_only` and
`transaction_read_only` settings are accepted and not applied (`EDGES.md` row 69).

**Durability** is a single file, no WAL: two in-band meta slots and CoW data
pages, with the second fsync as the commit point and CRCs on every page and slot.
Crash-safety targets the **native** build; browser/OPFS is best-effort. Tested by
fault injection, a real-process `kill -9` + soak suite against the server, a
`SIGKILL` of a real embedding process, every power-loss image of recorded DDL and DML
workloads (each tried subset of the writes between two fsyncs landing), a corruption
fuzz, a coverage-guided `cargo-fuzz` target, and a root-gated `dm-flakey` power-loss
harness whose device loses writes it acknowledged. Tested, not proven: each instrument
checks the workloads it runs. A write the device refuses is answered `53100`/`58030`;
where the statement cannot be dropped, every later one is refused `57P02` until the
database is reopened — the native server reopens for its next connection, an embedding
host reopens the `Database`. `docs/storage.md` for the mechanism, `PLAN.md` § 3 for its
status.

**Indexes are built, maintained, and used by the planner** — B-tree for
single-column equality, range predicates, and single-key `ORDER BY [LIMIT]` over a
`NOT NULL` column; GIN for JSONB / array `@>` containment, including under `OR`,
with a recheck. Composite, multi-key, expression-`ORDER BY` and `NULLS FIRST|LAST`
reads are deferred. A simple `UPDATE` or `DELETE` keyed by an indexed equality finds its
target rows through the same B-tree probe a `SELECT` with its `WHERE` would take, where the
rest of the `WHERE` is conjuncts that cannot fail; otherwise — a range among them — and for
an `UPDATE … FROM`, by a full scan of the table (an `INSERT … ON CONFLICT (…) DO UPDATE`
finds its conflicting row through the arbiter index).

**Not yet, and loud about it:** most of what pglet has not built it refuses (`0A000`,
`42601` or `42704`) rather than approximates — not all of it: some is accepted and
answers otherwise with no error, among them a sequence's `INCREMENT BY`, `MINVALUE` /
`MAXVALUE` and `CYCLE`, parsed and ignored so every draw is `+1` (`EDGES.md` row 215),
and an uncast string beside a `jsonb` or `bytea` operand of `||`, joined as text (row
293). A test suite meets some of the refused SQL early —
`generate_series` beyond its integer form in `FROM`, `CREATE TABLE … AS SELECT`, the aggregates beyond the basics,
`lag` / `lead` and window frames, views, domains, functions and triggers, array
subscripting (`a[1]`), general regular expressions. The npm package's README lists that
SQL in plain words, as
[Not supported yet](crates/pglet_wasm/npm/README.md#not-supported-yet), beside the
answers pglet gives differently with no error,
[Known differences from PostgreSQL](crates/pglet_wasm/npm/README.md#known-differences-from-postgresql).
`docs/compatibility.md` carries the rest per feature, and `EDGES.md` is the full ledger
of recorded divergences.

## Goals, honestly measured

- **Size:** the WASM artifact — the metric that matters for the embedded use — is
  ~1613 KB raw / ~647 KB gzipped over the wire — about a tenth of pglite's raw
  payload and an eighth of its gzipped one (`benches/pglite_size_comparison.json`).
  The gate is on raw bytes (deterministic; gzip is reported, not gated) against the
  recorded baseline +2% and a hard cap, re-validated on every full gate run. The recorded figures are `benches/size_baseline.json`; the budget and
  what has been cut to hold it are `TRADEOFFS.md`.
- **Startup (<100 ms): measured and met.** Native spawn→ready is ~1 ms p50 across
  storage modes; in-process WASM init is ~3.5 ms p50 for a full cold load
  (`benches/wasm_init_results.json`). On the head-to-head harness, whose cold init is
  each engine's fresh module load through its first query — for pglet, its npm
  package's — pglet takes ~12–16 ms against pglite's ~1.4 s across Deno / Node /
  Bun (`benches/wasm_compare_results.json`).
  Run `deno task bench:wasm-init` / `bench:wasm-compare` to re-measure.

## Milestones

pglet ships **test-time-first**: a fast, throwaway test-fixture engine now, a
production-data store later, full-but-optional PostgreSQL conformance as the long
arc.

- [x] **0.1 — the test-fixture engine** (this release): a real production app's
      full SQL surface (fuz_app — auth, sessions, and a fact/cell data layer) runs
      end-to-end on both the native-wire and in-process wasm legs, with the
      publicly checkable half of the same claim in `tests/fixtures/` — thousands of
      differential fixtures compared byte-for-byte against a live Postgres, in both
      wire formats. Constraints enforced, the durability mechanism built and
      crash-tested (open residuals in `EDGES.md`), `fork()` per-test isolation, the
      size and startup thesis proven, index acceleration as a compile-time size lever, the wider generic types (DATE / TIME / INTERVAL /
      NUMERIC), the generic-conformance bar of `PLAN.md` § 7, and the published
      **`@fuzdev/pglet-wasm`** adapter.
- [ ] **Embedded production data** — a trustworthy system of record; medium-term
  - [ ] Accrued large-dataset soak time
  - [ ] Accrued maturity on a young, hand-rolled engine
  - [x] Overflow pages for large values — a row is not bounded by one ~4 KB page
- [ ] **Toward full, optional PostgreSQL conformance** — the long arc
  - [ ] Broader Postgres coverage as opt-in, compile-time-additive layers (more
        types, PL/pgSQL, …) with the minimal build staying the gated floor —
        so a build carries only what its consumer needs
  - [ ] Cost-based planner + `EXPLAIN` — the arbitrary-SQL ceiling (today:
        rule-based, nested-loop joins, no stats)
  - [ ] Streaming scans — replace fully-materialized results, for real-data scale
        (the materialized-scan memory cliff)
- [ ] **Beyond** — speculative
  - [ ] Local-first sync — surface the CoW commit sequence as a changefeed
        primitive (full sync logic lives above the engine)
  - [ ] A first-party `drizzle-orm/pglet` dialect upstream (needs
        drizzle-maintainer buy-in)
  - [ ] Production daemon hardening — connection auth + TLS for `pglet_server`
        beyond a trusted loopback boundary

## Docs

- [`@fuzdev/pglet-wasm` README](crates/pglet_wasm/npm/README.md) — the npm
  adapter: API, value coercion, transactions, ORM recipes
- `PLAN.md` — roadmap and current state
- [`docs/compatibility.md`](docs/compatibility.md) — per-feature status: one line per
  feature over each feature's full record
- `EDGES.md` — the known-issues ledger: one stably-numbered row per recorded
  silent-wrong edge or divergence the differential harness does not hold
- `TRADEOFFS.md` — what's cut and kept, weighed against the size/perf goal
- `CLAUDE.md` — architecture, crate map, conventions, dev workflow (also the
  AI-agent context, which is a first-class audience here)
- `docs/` — design references: [storage](docs/storage.md) (single-file
  double-meta, no WAL), [parser](docs/parser.md) (native, Postgres-only),
  [parameter binding](docs/param_binding.md), [numeric](docs/numeric.md) (the
  in-house bignum), [temporal types](docs/temporal.md), [arrays](docs/arrays.md),
  [enums](docs/enums.md), [INSERT](docs/insert.md),
  [indexes & build profiles](docs/indexes.md), [declared
  types](docs/declared_types.md); and the engine rules in full —
  [names](docs/names.md), [sequences](docs/sequences.md), [scope](docs/scope.md),
  [coercion](docs/coercion.md), [DDL & error precedence](docs/ddl.md),
  [deparse — the two renderings](docs/deparse.md), [errors — the wire
  fields](docs/errors.md), [encoding — text in the server
  encoding](docs/encoding.md)

## Contributing

[fuz.dev/contributing](https://www.fuz.dev/contributing) — and read
`CONTRIBUTING.md` before running the tests: they issue `DROP SCHEMA public
CASCADE` against the Postgres they connect to.

## License [🐦](https://wikipedia.org/wiki/Free_and_open-source_software)

[MIT](LICENSE)
