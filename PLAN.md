# pglet plan

Roadmap and honest status. pglet is a small Postgres-compatible-ish engine; see
`README.md` for the pitch and `CLAUDE.md` for architecture. The long-term aim is
**full PostgreSQL conformance, delivered as optional compile-time layers** — a
build carries only what its consumer needs, and pglet is never positioned as a
replacement for upstream PostgreSQL (it serves the places a real server doesn't
fit: test suites, browsers, embedded/local-first apps). The short version of
where that stands:
the **size** and **startup** goals are both measured and met (the size/perf
thesis holds — see § 1), the fuz-stack Rust conformance is in place
(deps + lints + edition; see § 2), the durability and constraint mechanisms
(§ 3–4) are built, their open residuals recorded in `EDGES.md`, and the spine-compat SQL
surface (§ 5) backs the fuz data layer end-to-end on both pglet legs. The § 7
generic-conformance bar — the **test-fixture release** gate — is **met** and
locked by `tests/gates_baseline.txt`; what keeps the engine from real data is
accrued soak time and those residuals (see § Maturity and `EDGES.md`).

## Status

**Do not store data you can't lose yet — for maturity reasons.**
The commit path is crash-tested to a simulated power cut (§ 3); what keeps the warning in
force is accrued soak time on a young hand-rolled engine, browser/OPFS being best-effort
(crash-safety targets the **native** build). See § Maturity.

What works, and with what caveats, is [`docs/compatibility.md`](docs/compatibility.md) — the
single source of truth, deliberately not summarized here, since a second summary is a
second thing to keep true. Two standing ceilings it has no row for: results are **fully
materialized** (no streaming scans — the memory cliff at real-data scale), and there is
**no `EXPLAIN` and no cost-based planner** (rule-based index selection, nested-loop
joins, no stats). What is excluded for cause rather than unbuilt — a general regex engine (`~` /
`!~` answer an anchored literal prefix only), the jsonpath `@?`/`@@` language,
multi-dimensional arrays — is § 7.

Green is defined by the locked gate set: `cargo xtask gates --check` against
`tests/gates_baseline.txt`, which is also where the fixture, test and declared-type
counts live.

## Roadmap

### 1. Prove the thesis — benchmarking + size lock-in

Size and startup are **measured and the thesis holds**. The headline figures are
`README.md` (intro and § Goals, honestly measured) and `TRADEOFFS.md` (size, release
profile, startup, the `index_acceleration` size-floor lever); the recorded numbers are
`benches/*_results.json` and `benches/size_baseline.json`, and what each bench may and may
not be read as is `benches/README.md`. The harness is Deno tasks over `fuz_util`'s statistics
(reproduced in `benches/lib/bench_stats.ts` where it cannot be imported) plus
the `pglet_bench` crate (SQLite via `rusqlite` as its baseline), watch-tier: no gate but
size, no criterion, no new Cargo deps, no `cargo-bloat` (it does not support the wasm
target).

**Open in this track:** a B-tree index-accelerated lookup leg **vs SQLite** plus join and
transaction read shapes (`bench:gin` and `bench:index-accel` measure only the
pglet-vs-pglet ratio today), and an **OPFS-persistence perf leg** inside the existing
browser harness (`tests-web/browser/run.ts`). The bar those close against: beat pglite
decisively on startup and size, stay within an acceptable factor of SQLite on throughput
with no dubious shortcuts, and weigh every perf win against gzipped size.

### 2. fuz-stack Rust conformance + approved deps

In place: `CLAUDE.md` § fuz-stack conformance and § Conventions, `TRADEOFFS.md` § Dependency
footprint, and `CONTRIBUTING.md` § Running the checks. Beyond those, `pglet_wasm` carries an
explicit `[lints]` block with audited panic-point `#[allow]`s, and a bare `cargo clippy
--workspace` covers neither tests nor benches, so it passes locally where `check` does not.

### 3. Durability correctness

**Layout: settled — single-file double-meta, no WAL.** Two meta slots; a commit writes
the batch's CoW pages, fsyncs, writes the alternate slot and fsyncs again (the second
fsync is the commit point); recovery picks the higher-txid valid slot — CRC-valid, with page ids
the store can hold — and falls back on a torn write. Crash-safety is the **native** target; browser/OPFS is **best-effort**. Both
targets run one `DoubleMetaStore` core, differing only in their `BlockIo` shim — the
mechanism, the store versioning and the known bottlenecks are `docs/storage.md` and
`crates/pglet_storage/src/btree/CLAUDE.md`.

**The WAL is rejected for the batched path**, and the verdict outlives its measurement: a
commit-time attribution showed the batched-write cost is ~98% CPU and ~0% fsync, so a
write-ahead log cannot improve throughput there and would only add size — and since a WAL
only moves fsync/IO placement, the verdict holds wherever the CPU goes. The enforcing
write path is flat across batch size at ~13–16× SQLite's WAL-mode p50 — the
in-transaction uniqueness probe merges the pending buffer through a per-transaction
index — O(log n) at the committed index plus an O(1) map lookup, rather than a table
scan — so an enforced bulk insert does not degrade with table or batch size.

**Tested, in five instruments** (not proven — each checks the workloads it runs): fault-injection crash tests (drop or tear the meta
write, reopen, assert the prior slot survives), every power-loss image of recorded DDL
and DML workloads with a real-process `SIGKILL` of an embedding host
(`crates/pglet/tests/durability.rs`), a real-process `kill -9` + soak leg
against a live `--data-dir` server (an acked commit always survives, an interrupted one
recovers atomically, and the data-dir `flock` is re-acquired after the hard kill), a
corruption sweep plus a coverage-guided `cargo-fuzz` target over a mutated valid image —
**stricter on purpose**: a panic on an untrusted image is a finding, not an acceptable
"loud" — and a root-gated `dm-flakey` `drop_writes` harness for *true* power loss, which a
`kill -9` cannot simulate (it cannot lose fsynced-but-cached data the way pulled power
can). It **passes** on its workload: every acked commit survives the simulated cut as a
clean prefix. It needs root, so it is opt-in (`deno task test:power-loss`) rather than part of
`deno task test`.

**A write the device refuses** is dropped with its statement where the host can take the
statement back (`53100`/`58030`, serving on), and otherwise answers the same code and fails
the store: every later statement is `57P02` until a reopen reads the durable slot — the
native server reopens for its next connection
(`docs/storage.md` § Crash Recovery). Open rows around the store include a recovered free
list trusted not to hold live ids (`EDGES.md` row 207), an emptied leaf never reclaimed
(209), a reopen that holds an open block's DDL (211), a drawn sequence value re-issued
after a reopen (214), and a block's staged rows carried across `Database::into_parts`
(216).

**Large values are stored out of line.** A leaf value too long to keep inline goes to a
chain of overflow pages, written, fsynced, published, superseded and vacuumed like every
other CoW page, so a row is not bounded by a page: rows to a mebibyte are measured
stored and read back whole — through reopen, a `kill -9` mid-commit (`test_crash`'s long-row phase), a failure at each
page write of a chain's commit, an open snapshot across its replacement, and `fork`. Every
page the tree builds fits one page: a node that no longer fits is cut into as many pieces
as it takes for each to fit, and both page stores refuse a page that does not rather than
write or hold it (`crates/pglet_storage/src/btree/CLAUDE.md` § Page Layout).

**Index entries are sized as 18.6 sizes them.** Every entry a write, a foreign-key action or
an index build enters is laid out as 18.6's `index_form_tuple` lays it out and refused `54000`
at 18.6's limits, with its messages and in its order against the row's other refusals and
across the table's indexes (`docs/indexes.md` § An index entry is sized as 18.6 sizes it).
The residuals are each an older `EDGES.md` row's member: a `char(n)` key sized unpadded
(row 95) and an enum key part behind a cast sized by its label (row 104), both silent; the
entry a build names among several under `DESC` or the oracle's collation (rows 92, 43); the
row a build or an `ADD PRIMARY KEY`'s NOT NULL scan reads first once an `UPDATE` has moved
a row (row 194); and a multi-row `INSERT`'s foreign-key order (row 166). pglet refuses two kinds of key 18.6
holds, loudly: one 18.6 holds only because it compresses it, and one whose pglet encoding
outgrows a page (rows 191 and 192).

### 4. Constraint enforcement

Enforced on every write path: NOT NULL, CHECK, every unique index (single-column,
composite, partial, `NULLS NOT DISTINCT`, expression-keyed), and foreign keys — single and
composite, with every `ON DELETE` / `ON UPDATE` action. No
`DEFERRABLE`: every constraint is checked immediately, Postgres's non-deferrable default.
The action model, its measured limitations and per-feature caveats are
`docs/compatibility.md`'s *NOT NULL / CHECK constraints* and *PK / UNIQUE / FK constraints*
records and the `EDGES.md` rows they cite.

### 5. Spine-compat SQL features

The *spine* is the SQL fuz_app's auth, fact and cell layers issue. All of it runs
end-to-end on both pglet legs (native server + in-process wasm), validated byte-for-byte by
the differential fixtures, and so do the non-spine column types (DATE, TIME, INTERVAL,
NUMERIC). Per-feature status: `docs/compatibility.md`. The design principle beneath it —
parameters and correlated columns resolve at eval time against an `EvalEnv`, so a prepared
statement holds one immutable plan and nothing rewrites it per binding — is `docs/scope.md`.

### 6. Introspection depth

Grow `pg_catalog` / `information_schema` (and the catalog functions) only as the
spine and tooling actually need them — captured, not guessed. Synthesized from the
live catalog so far: `pg_class` / `pg_namespace` / `pg_attribute` / `pg_type`;
`pg_tables` / `pg_indexes`; `information_schema.tables` and
`columns` (with `column_default` rendered to Postgres's spelling); `table_constraints`
over **all four kinds** — CHECK and FOREIGN KEY off the tables, PRIMARY KEY and UNIQUE
off the constraint-marked indexes; and `key_column_usage` / `referential_constraints` /
`constraint_column_usage` (the first and last carrying the PK/UNIQUE rows as well as the
FK ones). The spine's `account_schema` schema-introspection tests run end-to-end.

Two rules govern what a name in these views means. A constraint's name is **read from the catalog, never re-derived** — a PK/UNIQUE
reports its index's name, which *is* the constraint's, so a declared `CONSTRAINT my_pk`
appears as `my_pk` rather than a conventional `<table>_pkey`; and every CHECK is reported
under the name the executor minted at DDL time, the same stored fact the violation message
and the dump read (`CheckConstraint.name` is a total `String` — see the mint rule). Handing an
introspecting client a plausible-but-fabricated name, or silently omitting a constraint it
has, are the same failure in two directions: `(table_name, constraint_name)` is unique by
construction in `pg_constraint`, so the client has nothing to compare against and cannot
detect either.

**`NOT NULL` is `contype = 'n'`, a kind of its own** — 18.6 does not materialize it as a
CHECK constraint at the `pg_constraint` level (`pg_constraint.h`'s `CONSTRAINT_NOTNULL`). Only
`information_schema.table_constraints` and `check_constraints` relabel it CHECK, which
is why the fuz spine filters `contype <> 'n'` on both legs: engines disagree about
whether to catalog it at all. pglet keeps a column flag and synthesizes the rows from it
in all three views.

`pg_constraint` and `pg_index` are populated and `information_schema.check_constraints` is
modelled; both catalogs bind **exhaustively**, so an unmodelled column is `42703` here
where 18.6 answers it — a deliberate over-refusal over a fabricated column. The columns
not modelled and why, and the positional `pg_constraint.oid`, are `docs/compatibility.md`'s
*pg_catalog introspection* record.

### 7. Generic-conformance bar

The bar for the test-fixture release: a first user pointing an existing Postgres suite at
pglet stops hitting *common* walls whose fix involves **no design decision and no size
cost**. Every such gap is implemented **to the Postgres spec** (the PostgreSQL source is
the oracle for *why*), with differential-fixture coverage (plus unit tests where the
harness cannot reach), on **forward-looking seams** — a real set-op plan node, one shared
typmod-capture path, never a point patch. **Met**, and locked by
`tests/gates_baseline.txt`; what it covered is carried per feature by
`docs/compatibility.md`.

**Deferred, loud, not silent:** a fractional numeric exponent and numeric `Infinity`. One
**silent** typmod residual stays: `char(n)` is length-enforced, not `bpchar`-blank-padded
(`EDGES.md` row 95 — the padding, the trailing-blank-insensitive comparator and the
index-key encoder must agree, and a real `TypeOid::Bpchar` is a store-magic bump). The
rules the bar closed on are stated in full elsewhere: a canonical type name decomposes once
and each part is decided by the type (`docs/coercion.md`; residuals in `EDGES.md` rows 116,
119, 120, 123 and 124), a string literal is assembled in 18.6's units (`docs/parser.md`;
rows 125 and 127), and text is validated in the server encoding where it enters
(`docs/encoding.md`; open doors rows 130 and 133).

**Excluded from the bar, each for cause:** a general regex engine and the jsonpath `@?`/`@@`
language (size), and multi-dimensional arrays (uncommon, and a foundational value-shape
change) — each fail-loud rather than silently approximated. `~` and `!~` answer one pattern
shape, `^` followed by literal characters (the prefix test a migrator's `nspname !~ '^pg_'`
needs), and refuse every other pattern `0A000` (`EDGES.md` row 251); `~*`, `!~*`,
`SIMILAR TO` and the `regexp_*` functions stay unimplemented.

## SQL compatibility

What works, and with what caveats, is [`docs/compatibility.md`](docs/compatibility.md):
a scannable table of the features it tracks, each with its status, and each feature's full
record below it. The recorded divergences are [`EDGES.md`](EDGES.md)'s rows; a reference of the form
"row 47" indexes that file.

## Non-goals

**Runtime** extensions/plugins, replication, and roles/RLS are hard non-goals.
Regex operators, ICU collation, and the timezone database are cut on size grounds.
Heavier SQL surface (general PL/pgSQL, stored procedures, triggers, full-text search)
is out of the **minimal** build today — the one exception is a *minimal* `DO $$ … $$`
block (the idempotent-`CREATE TYPE` idiom, `docs/compatibility.md`'s `DO` row), whose `EXCEPTION` handler is a
real subtransaction (transaction control inside the body is refused), deliberately
scoped so the floor doesn't pay for a PL/pgSQL interpreter. The modular direction is to make heavier
features opt-in compile-time layers rather than permanent non-goals — see
`TRADEOFFS.md` (Modularity, and the size reasoning).

Beyond these deliberate cuts, a range of Postgres **types absent by omission** —
composite/row, range/multirange, domain, geometric, network, bit-string
(`B'…'` / `X'…'`), XML, `money`, `pg_lsn`, `txid_snapshot`, `citext`, `json`, and the
`reg*` spellings other than `regclass` (`docs/compatibility.md`'s *pg_catalog
introspection* record) — are unimplemented and **fail loud** (`42704`, correct-or-loud) rather than silently
coercing. These are gaps rather than decided non-goals; they land on demand.
`42704` is the code for *using* such a type; a `CREATE`/`DROP` statement naming an
object kind the parser's dispatch does not carry — `CREATE DOMAIN` among them — answers `0A000` instead, per `docs/compatibility.md`'s row for them
and `EDGES.md` row 99.

`oid` and `regclass` are held out of that list, each for its own reason. `oid` (26) is a
**declaration** type rather than an absent one: it has no column form here, so
`CREATE TABLE t (a oid)` and `SELECT 1::oid` are `42704` (18.6 accepts both), while
`pg_constraint.conrelid` is declared `oid` and `unnest(conrelid)` names it exactly as
18.6 does — `docs/compatibility.md`'s pg_catalog row, where `"char"` sits beside it. And
`::regclass` / `CAST(… AS regclass)` is a **decided model**: `parse_cast_target`
intercepts the name ahead of the type resolver and answers the relation **name** as
`text` — rendered for an identifier position through `lexer::quote_ident`, so
`'"Odd"'::regclass` prints `"Odd"` as `regclassout` does — where a `regclass` *column*
is `42704`. `EDGES.md` row 85 is the record, and its (e) is the
relation kind that resolver's own list omits: a **sequence**'s name is `42P01` here
where 18.6 answers it. The intercept decides on the **canonical type name**, so a `[]`
suffix or a `(…)` modifier makes the target a different type and takes that same `42704`
— which closed the `'t'::regclass[]` write row 118 records. Each measured on both
engines.

## Maturity

Single-author prototype, actively developed, with a strong differential-testing
methodology. Crash testing
is real — in-process fault injection, a real-process `kill -9` + soak suite, a
randomized corruption fuzz (`crates/pglet/tests/crash.rs`, `test_crash`), every power-loss
image of recorded DDL and DML workloads plus a `SIGKILL` of a real embedding process
(`crates/pglet/tests/durability.rs`), and a coverage-guided
`cargo-fuzz` recovery target (`deno task fuzz`); *true* power-loss
(page-cache-bypassing) testing passes via the root-gated `dm-flakey` harness
(`deno task test:power-loss`). A large-dataset soak harness (`test_soak` /
`deno task test:soak`) drives sustained mixed churn with periodic `kill -9`/recover
against a rich multi-table schema (PK / UNIQUE / composite FK / CHECK / b-tree /
partial / GIN), asserting row-set, index↔table consistency, MVCC snapshot/vacuum,
and durability invariants continuously — plus an in-process leg that reads live
CoW-btree stats for the snapshot/vacuum-leak checks the wire can't see. A
**per-test-database re-migration leg** additionally regression-locks the
catalog↔storage **index-OID-reuse** class: a long-lived server is driven through
repeated idempotent re-migration + same-named index swaps (the shape a reuse-heavy
per-test-database suite provokes), and a within-transaction same-name `DROP/CREATE
INDEX` + an extended-protocol index-accelerated probe fails loud if a recreated
index's storage band is ever orphaned or reused (an `index not found` desync). A
band↔catalog **bijection** audit at each `kill -9` reopen catches an orphaned band —
a storage `index_id` with entries but no catalog owner — which the per-index
consistency scan is structurally blind to. Every invariant holds through 1M rows,
and the re-migration leg reproduces the index-OID desync when the monotonic-OID fix
is removed; what it leaves open is accrued soak time at larger tiers. Treat as a
research-grade embedded engine, not a store of record — yet.
