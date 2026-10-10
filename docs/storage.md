# Storage engine

> The storage-engine **design reference**. pglet's storage is a Copy-on-Write
> B+tree with single-writer concurrency and lock-free MVCC snapshot reads.
> Persistence is a single file with two in-band meta slots written alternately —
> **no WAL**. The durable on-disk layout and commit/recovery sequence are
> specified in `crates/pglet_storage/src/btree/CLAUDE.md`.

**Goal:** Scalable storage with single-writer concurrency (like SQLite).

## Storage Engine: Copy-on-Write B+tree

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Storage engine | CoW B+tree | ~2–5× amortized write amp (§ Write Amplification Analysis), read amp 1, natural MVCC — old roots *are* old snapshots — and a good OPFS fit (one file, one sync access handle) |
| Concurrency model | Single-writer | No concurrent page modification, so no write-conflict machinery; proven by SQLite |
| Reader isolation | Snapshot via old roots | Zero-cost MVCC |
| Page size | 4KB (fixed `PAGE_SIZE` constant) | Matches OS/OPFS page size |
| Key format | (table_id, row_id) | Schema-aware, compact |
| File layout | Single file: two in-band meta slots + CoW pages (no WAL) | One OPFS handle; the alternate slot's fsync is the commit |
| Vacuum strategy | Concurrent with readers | Old pages freed once neither a reader's snapshot nor the durable meta slot's version reaches them |

The key insight: **serializing writes** — one block's at a time, from its first write to
its end (§ Why Single-Writer?) — removes the page-level conflict detection and retry loops
a multi-writer engine needs, while leaving read concurrency intact — and CoW makes it
simpler still, since old roots already provide the snapshots. What it costs is
concurrency between writers that would not have conflicted (§ Why Single-Writer?, and
§ Isolation Guarantees for what each level keeps).

**Positioning:** SQLite's architecture (B-tree + single-writer) with Postgres's wire
protocol.

## Why Single-Writer?

Concurrent write transactions require conflict detection, first-committer-wins
logic, and read-set/predicate tracking — significant complexity for a scenario
that's rarely the bottleneck in pglet's audiences. SQLite's single-writer model
powers billions of devices: writes are serial by construction (no concurrent
page modification, no conflict-detection code, no surprising serialization
aborts) while readers stay concurrent on snapshots. The class comparison
against SQLite's journal/WAL modes and Postgres — and why single-writer is a
permanent axiom here rather than a stopgap — is `TRADEOFFS.md` § Concurrency.

**The writer is held for a block, not for a statement.** At most one connection's
block stages writes at a time: a block takes **the writer** at its first write — DML,
DDL, `COPY … FROM`, `DO`, a `nextval`/`setval`, a `SELECT … FOR UPDATE`/`SHARE` — or at its
first `pg_advisory_xact_lock`, whose lock the writer is (`EDGES.md` row 250), and holds it
until the block ends (`COMMIT`, `ROLLBACK`, a disconnect, a failed-store reopen, an
embedded `Database` dropped); an autocommit write or `pg_advisory_xact_lock` holds it for
its own statement, and any other read takes it only to lock rows. A host's own change while a block is open takes it too
— `Database::schema_mut`, `storage_mut`, `reattach_persisted_indexes` — and is the
block's (a `schema_mut` change its rollback takes back with its own); `Database::copy_in`
takes it as the `COPY … FROM` it runs. When a block takes it, every frame the block
holds is **re-based** on the state committed at that moment — its storage snapshot, the
directory and table-id counter it restores, each frame's catalog clone, the memory backend's clone — which is
sound because nothing is staged yet. So a block's writes are decided against what is
committed when it first writes, and nothing but that block changes the catalog or the
store until it ends. A second writer **waits** (the native server), bounded by its
session's `lock_timeout` — pglet's default `1s`, where 18.6's is `0`, set by the
server's `--lock-timeout` flag and by `SET`, `SET LOCAL` or the startup packet; past it
the statement answers `55P03 canceling statement due to lock timeout` and a block it ran
in is aborted. The embedded `Database` runs one statement at a time on one thread and
cannot wait, so it refuses `55P03` instead: another logical connection's write while a
block holds the writer, and its `BEGIN`/`COMMIT`/`ROLLBACK` while a block is open.

**A plan does not fail on another connection's uncommitted catalog.** The catalog is one
live structure and a block's DDL lands in it at once, so a statement planned while another
connection's block holds the writer plans against that block's changes. A plan that succeeds
there and takes the writer — a write, or a read that locks rows, an auto-commit one only
where the host runs reads under the write lock (`--storage memory` on the native server) —
waits for it and is planned again if the catalog moved (the embedded `Database` refuses it
`55P03`). A plan that *fails* there is planned again against the catalog committed when the
block took the writer: where it succeeds there, or fails otherwise — a column the block
dropped — the failure is the block's change, and the statement waits for the block to end
and is planned again, read or write (the embedded `Database`, which cannot wait, refuses it
`55P03`); where it fails the same way — a column no catalog has, a cast no catalog decides,
a grammar error — the failure is its own, answered at once. The SQLSTATE decides nothing:
pglet's parse and analysis are one pass, and a `42601` can be the catalog's (a set
operation's column counts over a `SELECT *`). That pass also orders the failures: a grammar
error it reaches only after a check the block's change decides — one whose failure, or whose
message, differs between the two catalogs — waits (the embedded `Database` refuses it
`55P03`), where 18.6's raw parse answers it at once, and one behind a check both catalogs
fail alike is answered with that check's error at once (`EDGES.md` row 238). A `COPY`'s
column list, which its plan leaves to execution, is resolved under the same rule before the
statement runs. This key is not 18.6's, which waits on the locks a statement takes. A
statement naming an object the block created, of whatever kind, finds it here: one that
takes the writer, or whose plan fails because of the object, waits (the embedded `Database`
refuses it `55P03`), and after the block's `COMMIT` is answered over the committed object by
its own gates (a `REPEATABLE READ` block's write `40001`, `EDGES.md` row 232): in the other
members measured a write lands, a sequence draws, a `DROP`, `IF EXISTS` or not, drops it,
where 18.6 answers the object's does-not-exist error or skips the `IF EXISTS` form at once.
18.6's answer varies by statement, and the class is not catalogued (`EDGES.md` row 240). One
failing the same way over a relation the block altered answers at once here and waits there
(`EDGES.md` row 238). What this covers is **planning**: a read whose plan succeeds still
reads the uncommitted catalog, and an error raised while a plan runs can come from it — a
read of a table the block dropped is `42P01` (`EDGES.md` row 221).

**What that costs, against 18.6.** One writer serializes *unrelated* writes, where 18.6
blocks only a conflicting row, table or index key: a write waits for any block that has
written, and times out at the bound where 18.6 would not have waited at all (`EDGES.md`
row 228; the embed's refusal is row 229); on the native server a client that holds a
written block open and awaits a write on a second connection waits for its own block until
the bound (row 231).
A block aborted by an error holds the writer until it ends, a `ROLLBACK TO` releasing
nothing (18.6 releases a failed transaction's locks at the error, and a failed
subtransaction's — a key written inside the aborted savepoint — at its error; row 230). A read inside a
block before its first write reads the block's `BEGIN` snapshot, where 18.6's `READ
COMMITTED` reads the state committed at each statement (`EDGES.md` row 219's read half).
A `SELECT … FOR UPDATE` outside a block takes no writer on the native server's B+tree and
reads the committed value at once, where 18.6 waits for a block holding the row; on
`--storage memory`, which has no lock-free read path, it takes the writer for its statement
and so waits for any written block; the embedded `Database` takes the writer for it, so it
is refused `55P03` while another logical connection's block holds the writer (measured on
the in-memory B+tree). And **write skew** stays: a decision a
client makes from a read taken before its first write is its own, as on 18.6 `READ
COMMITTED`.

## Isolation Guarantees

| Guarantee | Provided? | Notes |
|-----------|-----------|-------|
| No dirty reads | **Partial** | on the B+tree backend another connection's uncommitted rows are invisible, but the catalog is shared and live, so a read resolves through a block's uncommitted DDL — its column list, an enum's order, catalog views (`EDGES.md` row 221); a write and a `nextval` wait for the block instead on the native server, and the embedded `Database` refuses them `55P03` (`EDGES.md` row 229) |
| No non-repeatable reads | **Per level** | on the B+tree, a block reads its `BEGIN` snapshot until its first write; at `READ COMMITTED` that write re-bases it on what is committed, so a value read before it can read differently after (18.6's `READ COMMITTED` differs at every statement); at `REPEATABLE READ` / `SERIALIZABLE` the snapshot holds for the block, whose first write is refused `40001` if the store moved past it (`EDGES.md` row 232). `--storage memory` keeps no snapshot: its reads are live at every level |
| No phantom reads | **Per level** | as the row above, for a row set: another connection's committed insert appears to a `READ COMMITTED` block after its first write, and never to a `REPEATABLE READ` / `SERIALIZABLE` one that goes on writing |
| No lost updates | Yes, for a statement's own read-modify-write | a write waits for the block holding the writer (the embedded `Database` refuses it `55P03`, `EDGES.md` row 229) and then reads the latest committed row, and a block's first write re-bases it on what is committed; a value a client computed from a read taken before its write is its own, as on 18.6 `READ COMMITTED` |
| Indexes agree with their table across a concurrent write | Yes | a block's `COMMIT` derives each row's old index entries from the rows committed when it took the writer, which no other connection moved since |
| No write skew | **No** | no conflict detection — snapshot isolation, not serializable |
| Unique/PK enforced against a concurrent *pending* write | **Yes** | the later writer waits for the block holding the writer, then answers against its outcome: `23505` after a `COMMIT`, a write after a `ROLLBACK` — 18.6's answers, reached by waiting on the native server; the embedded `Database` refuses the later writer `55P03` while the block is open (`EDGES.md` row 229) |
| Unique/PK enforced against a key committed after a block's `BEGIN` | **Yes** | at `READ COMMITTED` the block's first write re-bases it on what is committed, so its write of that key is refused `23505`, and a statement after a committed key move matches the row now under the key; at `REPEATABLE READ` / `SERIALIZABLE` that write is refused `40001` instead, where 18.6 answers `23505` (`EDGES.md` row 232) |
| Foreign keys enforced against a concurrent write, pending or committed after a block's `BEGIN` | **Yes** | a pending write makes the other writer wait (the embedded `Database` refuses that writer `55P03`); a committed one is in the state a `READ COMMITTED` block takes the writer on, and refuses a `REPEATABLE READ` / `SERIALIZABLE` block's write `40001` |
| Serializable transactions | **Coarse** | a `REPEATABLE READ` / `SERIALIZABLE` block keeps its snapshot, noted by its first statement that takes one — a planned statement other than transaction control, or a `COPY`; not a `SET`, `SHOW`, `SAVEPOINT`, `DEALLOCATE` or `DISCARD` (the last two take one on 18.6, `EDGES.md` row 237) — as 18.6 takes it at its first such statement: that statement's start where it is the block's first write, else `BEGIN`'s, which the block's reads see — and its first write is refused `40001` if the store's commit stamp moved past it — so a block that writes read a state no commit moved, and the writer keeps one from moving while it writes; coarser than 18.6, which refuses only a write meeting a row changed since: an unrelated commit, another connection's `nextval` or `setval`, and a commit between `BEGIN` and a first-statement read refuse it too, and a key committed since is `40001` where 18.6 finds `23505` (`EDGES.md` row 232). A block that takes no writer — no write, no locking read — is never refused |

At `READ COMMITTED` this is 18.6's `READ COMMITTED` for what a block writes — one writer at
a time, each block writing against the state committed when it took the writer — with a
stronger read before the first write (the `BEGIN` snapshot) and a single writer where 18.6
locks rows. At `REPEATABLE READ` / `SERIALIZABLE` a block that writes wrote against the
snapshot it read, which no commit had moved, at the price of the over-refusal above; the
catalog is not snapshotted at any level (§ Schema Snapshot Semantics). The level is set by
`BEGIN` / `START TRANSACTION … ISOLATION LEVEL`, `SET TRANSACTION` (refused `25001`, as on
18.6, once the block has run a statement that takes a snapshot — any but transaction
control, `SET` and `SHOW`, a `COPY` included, though here not `DEALLOCATE` or `DISCARD`,
which 18.6 counts (`EDGES.md` row 237), nor an extended-protocol `Parse` (row 239) — or
inside a savepoint), `SET SESSION
CHARACTERISTICS`, `default_transaction_isolation` (by `SET` or the startup packet), and
reported by `SHOW transaction_isolation`; `READ UNCOMMITTED` runs as `READ COMMITTED`, as on
18.6. The `set_config` spelling of either setting cannot reach the session, so naming
`REPEATABLE READ` or `SERIALIZABLE` there is refused `0A000` rather than answered as set.

**A constraint is decided against a state no other connection is moving.** Postgres decides
a duplicate key at `READ COMMITTED` from the *live* index under a row lock rather than the
transaction's snapshot, waiting on a pending writer. pglet's writer gives the same answers
another way: a pending write belongs to the block holding the writer, so another writer
waits for that block to end (on the native server; the embedded `Database` refuses it
`55P03`, `EDGES.md` row 229) and then probes what it committed or took back, and a block's
first write re-bases its snapshot. The cross-connection claim walk
(`unique_key_claimed_elsewhere`) runs and, with one writer at a time, finds nothing; the
storage counts what it and the table-id restore's other-transaction arm find (`cross_connection_hits`), and a
count above `0` is a write that staged without the writer.

`READ ONLY` is the one transaction mode that is **not** accepted-and-inert: it is refused
(`0A000`) wherever it is written, on `BEGIN`/`START TRANSACTION` and on the `SET` spellings
alike. There is no read-only block here to enter, so accepting the request and then
executing the write it forbids would answer success where 18.6 answers `25006` — the
silent over-acceptance the engine's correct-or-loud rule forbids, and the one mode whose
inertness would be a *wrong answer* rather than a stronger one. Enforcing it instead
(a session flag plus a gate where the executor admits a write) is the better answer and is
open work. `[NOT] DEFERRABLE` is accepted and inert: it only bites in combination with
`SERIALIZABLE READ ONLY`.

## CoW B+tree Architecture

### Core Concept

Never modify pages in place. Copy, modify, write new page, update parent (recursively to root), atomic root swap.

```
Before INSERT key=50:

    Root v1: [30|60]
             /     \
        [10|20]   [40|55|70]

After INSERT (pages on write path copied):

    Root v1: [30|60]          Root v2: [30|60]'    ← new root
             /     \                   /     \
        [10|20]   [40|55|70]      [10|20]   [40|50|55|70]'  ← new leaf
                                     ↑
                              (shared, not copied)
```

**MVCC for free:** Readers holding Root v1 see old state. Readers on v2 see new state. No locks, no version filtering.

### Page Structure

```
┌─────────────────────────────────────────────────────────────┐
│ Page Header (32 bytes)                                       │
│ ┌─────────┬───────┬───────┬──────────┬──────────┬─────────┐ │
│ │ page_id │ flags │ count │ next_or_ │ reserved │ checksum│ │
│ │  (u64)  │ (u16) │ (u16) │ reserved │ (8 B pad)│  (u32)  │ │
│ │  0..8   │ 8..10 │ 10..12│  (u64)   │  20..28  │  28..32 │ │
│ │         │       │       │  12..20  │          │         │ │
│ └─────────┴───────┴───────┴──────────┴──────────┴─────────┘ │
├─────────────────────────────────────────────────────────────┤
│ Key/Pointer pairs (internal) or Key/Value pairs (leaf)      │
│                                                             │
│ Internal: [first_child] [(key, child_page_id), ...]         │
│ Leaf:     [(key, value_bytes | overflow_ref), ...]          │
│ Overflow: [payload bytes]                                   │
└─────────────────────────────────────────────────────────────┘
```

An internal page holds N keys and **N+1** children (the leading `first_child`
plus one child per key), and every key/value is length-prefixed (`u32`) because
`BtreeKey` is variable-length — see `btree/page.rs` for the byte-exact layout.

The `next_or_reserved` header field is per-kind: the leaf-chain `next_leaf`
pointer for `Leaf` pages, the next link for `Overflow` pages, the free-list next
pointer for `Free` pages, reserved otherwise.

**Page types:**
- `Internal` - Keys + child page IDs, no data
- `Leaf` - Keys + serialized row data + next-leaf pointer
- `Overflow` - One link of the chain holding a leaf value too long to keep inline
  (over a quarter page, or too long to fit beside its key); the leaf holds a
  reference to the chain's head. A chain is written, published, superseded and freed
  like any other page, so snapshots, `fork`, the double-meta commit and vacuum apply
  to it unchanged (`crates/pglet_storage/src/btree/CLAUDE.md` § Page Layout)
- `Free` - Reclaimed by vacuum, available for reuse (chained via the header's
  next pointer)

### File Layout

A single file (`data_dir/data.pglet`) — **no WAL, no separate meta file**: two
fixed-offset in-band meta slots followed by the CoW data pages.

```
data.pglet:
┌──────────┬──────────┬──────────┬──────────┬──────────┬─────┐
│ Meta A   │ Meta B   │ Page 0   │ Page 1   │ Page 2   │ ... │
│ (slot)   │ (slot)   │ (4KB)    │ (4KB)    │ (4KB)    │     │
└──────────┴──────────┴──────────┴──────────┴──────────┴─────┘
```

Each meta slot holds a small **fixed-size** record — `version`, root page ID,
`next_page_id`, `next_table_id`, `freelist_head` and the head chain page's checksum, and
the retired-page log's head, checksum, largest entry and floor — then at most 240 of
that log's newest entries, with a CRC over the slot.
The variable-size catalog, table directory, and row-id counters live **in the
B+tree** as `0x02` system entries (so a catalog change is atomic with the data it
describes, MVCC-covered for free), and the free-list lives in a **chained page
list** whose head and the head page's checksum are the only free-list state in the slot. The slot is therefore
bounded — no metadata size ceiling. A commit writes the new CoW pages,
fsyncs, then logs the pages it superseded, persists the free-list chain and writes the
metadata record into the *alternate* slot and fsyncs again (2 fsyncs; the second is the commit
point), and flips the active slot. Recovery picks the higher-version valid slot
(CRC-valid, with page ids the store can hold — § Crash Recovery), falling back to the
other on a torn write, then rebuilds the in-memory
catalog/directory/counters by scanning the system bands, and the free list by
walking the chain and adding the retired log's entries at or past the slot's floor. Copy-on-write alone does not keep the durable slot's pages as it
wrote them — a statement can publish several in-memory versions before its one slot
write — so collection frees no page the durable version reaches until a newer slot is
durable, and the free-list chain is read only as far as its pages carry the checksums
the slot, and each page, record for the next (§ Crash Recovery).

**OPFS** (the wasm crate's opt-in `opfs` feature; the npm package's build carries no
durable store): Same single-file layout over one `createSyncAccessHandle()`. The
on-disk format and commit/recovery sequence are specified in
`crates/pglet_storage/src/btree/CLAUDE.md` (`double_meta.rs`); native and browser
share one `DoubleMetaStore` core over a `BlockIo` shim.

### Key Encoding

One tree, one order-preserving keyspace, three key kinds under a leading
discriminant byte (`BtreeKey` in `crates/pglet_storage/src/btree/key.rs`, which
owns the full encoding spec):

```text
Data row     (0x00): [0x00][table_id u32 BE][row_id u64 BE]            // 13 bytes
Index entry  (0x01): [0x01][index_id u32 BE][escaped value … 0x00 0x00][row_id u64 BE]
System entry (0x02): [0x02][sub-kind u8][id u32 BE]                    // catalog chunk / table meta
```

Big-endian fields keep byte order == numeric order; the discriminant keeps the
bands disjoint (all data keys sort before all index keys, system entries last).
The index value is framed with an order-preserving escape-terminated encoding
(the FoundationDB/CockroachDB scheme) so byte order equals value order across
differing lengths — range-scan ready.

**No version in key:** MVCC is handled by root versioning, not key-level versions.

## Write Path

```
Transaction buffer → COMMIT: acquire write_lock →
  → Allocate new pages → Copy-modify-write path to root → fsync pages →
  → Write metadata snapshot to alternate meta slot → fsync (commit point) →
  → Flip active slot → Release lock →
  → Old root available for vacuum
```

### Detailed Steps

The design-level invariant: writes are buffered, applied in a **single CoW pass** under
the write lock, and made durable by **two fsyncs** — the new pages, then the alternate
meta slot (the second fsync — **before** the active-slot flip, which follows it — is
the commit point). The exact
step-by-step (page allocation, the copy-modify-write path to root, the free-list chain
persist, the slot flip, old-root registration for vacuum) is the **canonical
implementation reference** in `crates/pglet_storage/src/btree/CLAUDE.md` §
Single-file durable layout — not restated here.

### Write Amplification Analysis

For a single row update:
```
Leaf page: 4KB (full page, even if one row changed)
Internal pages: 4KB × tree_height (typically 3-4 levels)
Meta slot: ≤4KB written per commit (the fixed record + up to 240 retired-log entries, in a 64 KiB slot)

Total: ~16-20KB for a small update = ~20x for tiny writes
But: Pages hold many rows, so amortized is much lower

For bulk inserts (100 rows fitting in one leaf):
Pages: ~16KB
Meta slot: ≤4KB
Total: ~20KB for ~10KB logical = ~2x
```

**Amortized write amplification: 2-5x**

## Read Path

```
Get root page ID → Follow internal nodes → Read leaf → Return value
```

### Detailed Steps

1. **Snapshot** - Reader captures current root page ID (atomic read)
2. **Traverse** - Binary search keys at each level, follow child pointers
3. **Leaf read** - Find key in leaf, deserialize value
4. **No locks** - Old pages remain valid until vacuum

### Read Amplification

```
Tree height for various sizes:
- 1MB database: ~2 levels
- 100MB database: ~3 levels
- 10GB database: ~4 levels

Each level = 1 page read (with caching, often just 1 uncached read)
```

**Read amplification: 1** (single path, no merging multiple sources)

## MVCC via Root Versioning

### How It Works

The current `(root, version)` pair is published through a seqlock: a committing writer
bumps an odd generation, stores both fields, then bumps it even; a reader spins until it
observes an even generation unchanged across its two loads, so it never sees a torn
pair, and then traverses from that root. Because pages are copy-on-write, the captured root keeps pointing at
a complete, immutable tree — the reader holds no locks and sees a stable snapshot
for its entire scan. Active snapshots are ref-counted by version
(`active_snapshots` in `cow.rs`) so vacuum knows which old roots are still
reachable; the refcount is atomic counts under a shared read lock, so concurrent
readers register without serializing (see the implementation reference).

### Snapshot Isolation

| Scenario | Behavior |
|----------|----------|
| Reader starts, writer commits | Reader sees pre-commit state (has old root) |
| Writer commits, reader starts | Reader sees post-commit state (gets new root) |
| Long-running reader | Blocks vacuum of pages it references |

**No version filtering needed** - the entire tree snapshot is consistent by construction (MVCC via root versioning, no per-key versions to filter).

### Fork (per-test-fixture branch)

The same CoW immutability that makes a *read* snapshot free also makes a
**writable branch** cheap. `CowBtree::fork` (surfaced as `Database::fork` and the
wasm `PgletBtree.fork()`) clones the in-memory store's page map into an
independent tree at the current committed `(root, version)`: each `Page` clone is
an `Arc` refcount bump on its entry vector, not a row-value deep copy, so the fork
shares all seeded data until something is written. A write on *either* the base or
the fork copies only the leaf it touches (`Arc::make_mut` sees the shared refcount
and clones), leaving the other side untouched — the two trees have separate roots,
GC state, and page-id spaces.

This is the per-test-database primitive: seed a base once (schema + fixtures),
then fork a fully isolated copy per test instead of re-running the setup SQL —
far cheaper than replaying the seed, and orders of magnitude cheaper than a fresh
Postgres/pglite. It applies to the **in-memory** CoW store only; a file/OPFS-backed
`DoubleMetaStore` isn't forkable in memory and fails loud (test fixtures are
in-memory, so this is the intended scope); the plain `StorageBackend::Memory` backend
fails loud too, having no copy-on-write fork at all.

A fork reflects **committed** state, and the two layers say so differently.
`CowBtree::fork` branches at a pinned committed `(root, version)`, and an
in-flight transaction's pending writes live outside the tree in
`BtreeStorageWrapper`, whose `fork` starts the branch with no in-flight
transactions — so at the storage layer they are simply not carried.
`Database::fork` does not rely on that: it **refuses** with
`Error::ActiveSqlTransaction` (`25001`) unless the session is `Idle`, because
the catalog is the facade's and *is* cloned, so a
fork taken across an open block would hand the fork a catalog and a heap that
disagree, silently in both directions — a pending `DROP TABLE` hides a committed
table and its rows (`42P01`), a pending `ADD COLUMN` gives the fork a wider
descriptor than its rows carry — and the seed-once/fork-per-test caller would get
well-formed, empty fixtures. The fork itself starts with no active transaction.

## Vacuum (Garbage Collection)

### When Pages Become Garbage

A page is garbage when:
1. It was replaced by CoW (a newer version exists)
2. No active snapshot's version reaches it (no reader holds an older root)
3. The durable meta slot's version does not reach it — the version a crash reopens
   (`PageStore::durable_version`; a store that persists nothing has none, and only the
   first two apply)

### Vacuum Strategy

When CoW replaces a page, the old page can't be freed right away — a reader on an
older root may still reference it, and so may the durable meta slot, which a crash
reopens. Replaced pages are parked in `pending_gc`, batched by the version that
retired them. Vacuum computes the minimum version still held by an active snapshot,
bounds it by the store's durable version, and moves any batch older than that to the
free list, where new allocations reuse them (LIFO). Vacuum never rewrites data — it
only moves page IDs onto the free list, so it's cheap and never blocks readers.

### Vacuum Triggers

| Trigger | Action |
|---------|--------|
| `pending_gc` exceeds the threshold | Run vacuum |
| Host calls `vacuum()` on the store (`CowBtree::vacuum` / the storage wrapper) | Force vacuum |

There is no SQL `VACUUM` statement — it parses as a bare identifier and fails
`42601`; forcing a vacuum is a Rust-API affair.

## Crash Recovery

There is no WAL. On reopen the store reads both meta slots, validates each by its
CRC and by its page ids, and adopts the higher-version valid slot — falling back to
the other if the chosen one is torn (a crash between fsync #1 and the slot's fsync leaves
the alternate slot half-written and CRC-invalid) or names a page id outside the store:
a `next_page_id` past the last page a file offset can address, or a root, free-list head
or retired-log head at or past its own `next_page_id`. The CRC vouches for a slot's bytes, not for
what they claim: every fresh allocation starts at that `next_page_id`, and every chain
link a read accepts is below it. A file that is nothing but the zeros a crash inside
creation leaves — no longer than the two slots — opens as a new store, since no commit
was acknowledged; any other file without a valid slot is refused.

What keeps the adopted slot's pages as it wrote them is two rules, not copy-on-write
alone. **Collection frees no page the durable version reaches** (`CowBtree::try_gc`,
bounded by `PageStore::durable_version`, which moves only once a slot's fsync returns),
so a statement that publishes several versions before its slot write — a DDL
statement's batches — gets none of the durable version's pages from collection. **The free-list chain is authenticated by the slot**: the slot
records the head chain page's checksum and each page its successor's, so a chain page
a power cut did not land, still holding another commit's list, ends the walk — a leak
of the unread ids, not a live page handed out, short of a stale page whose CRC32 happens
to match. What the walk still trusts is an
authentic page's ids: open drops a repeat and a chain page's own id, and checks no id
against the tree (`EDGES.md` row 207). A page superseded but not yet collected is
logged by the save all the same, since no root of the saved version reaches it, so a
close or a kill does not leak it. The log is kept apart from the free list, which every
save rewrites whole: its newest entries ride in the slot, and past 240 of them a save
spills them into chain pages written once, so a commit writes what it retired, not
every page superseded since the oldest open snapshot.

**A write the device refuses fails the store only when the statement cannot be dropped.**
A commit is published in memory before its slot is written — and a statement can publish
a batch before its commit's — so a refused write can leave readers seeing what the durable
slot does not hold. The store answers a refused page write or data `fsync` as `WriteRefused`
and leaves the decision to the tree (`CowBtree::apply_discardable_batch`). What the refused
statement did decides it, not what else is published: a version published earlier and not
yet saved stays published, and the next save makes it durable.

- **Dropped, and the store serves on,** when the batch belongs to a statement or block whose
  other effects the host can take back — an autocommit statement of no catalog-writing kind
  (`Plan::mutates_catalog`; a sequence advance is not one, and it stands after a rollback on
  18.6 too), or a block, whose rollback restores its catalog — and that statement or block
  holds no table or index it created or dropped at the storage layer (one a savepoint
  rollback undid does not count). The batch is dropped, its page ids handed back, and the statement answers an
  ordinary `53100` (`ENOSPC`) or `58030`, as 18.6 answers a data file it cannot extend. A
  block whose `COMMIT` meets the refusal ends rolled back — whatever it published before,
  so long as the rollback takes it back: an index it rebuilt over an enum it re-created is
  re-derived — and the store fails only when that rollback returns an error: a re-derived
  index that cannot be rebuilt (a refusal of its own, say), or a refused catalog save.
- **Otherwise the store fails**: a refusal in an autocommit statement of a catalog-writing
  kind, in a block holding such a table or index, in a batch no commit issued (a DDL
  statement's eager build, a save's catalog batch), or in the save itself, whose slot write
  may have landed. `DoubleMetaStore` records it (`WriteFailure`) and refuses every page
  read, page write and save after it; a page read in flight when it was recorded is checked
  again once its bytes are read, so a store reopened over the file never lends it a page.
  The statement answers `53100` or `58030` — as `Error::DurableWriteUnconfirmed`, its
  outcome unknown until a reopen, when the refusal came at or after the slot write, which
  may have landed, or met a block's `COMMIT` (a healthy block's, or an aborted one's, which
  18.6 converts to a rollback) whose durable catalog, written by a save since the block
  took the writer, differs from the one it was re-based on, a counter advanced in place aside —
  what a save after a catalog change of the block's leaves (§ Schema Snapshot Semantics);
  as `Error::DurableWriteFailed` otherwise. Every later statement is `57P02`: the embed
  refuses it at `Database`'s entry check, and the native server ends each connection on
  the failed store at its next message.
- **Recovery is a reopen**, which reads the durable slot alone and decides — 18.6's
  `PANIC`-and-recover on a failed WAL write or flush. An embedding host reopens the
  database itself; the native server reopens for the next connection it accepts, serving
  it from the store reopened from its durable slot while the data-dir lock stays held
  (`pglet_server`'s `server.rs`). A startup that meets the failed store — accepted before
  the failure, or while a reopen fails — is `FATAL 57P03` (`EDGES.md` row 208).

The on-disk format and the
slot-alternation/torn-write tests live in
`crates/pglet_storage/src/btree/CLAUDE.md` (`double_meta.rs`).

## Connection Lifecycle

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                         CONNECTION STATE MACHINE                                 │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                  │
│                    ┌──────────────────────────────────────┐                     │
│                    │                                      │                     │
│                    ▼                                      │                     │
│  ┌─────────┐  BEGIN   ┌───────────────────┐  COMMIT/    │                     │
│  │  Idle   │ ───────► │  InTransaction    │  ROLLBACK   │                     │
│  └─────────┘          └───────────────────┘ ────────────┘                     │
│       │                       │                                                 │
│       │                       │ ERROR (constraint, etc.)                        │
│       │                       ▼                                                 │
│       │               ┌───────────────────┐                                     │
│       │               │  Failed           │ ─ ROLLBACK / COMMIT ─► Idle        │
│       │               │  (aborted)        │                                     │
│       │               └───────────────────┘                                     │
│       │                                                                         │
│       │ Auto-commit query                                                       │
│       └──────────────────────────────────────────────────────► Idle            │
│                                                                                  │
│  DISCONNECT (any state):                                                        │
│    1. Rollback active transaction (if any)                                      │
│    2. Unregister snapshot from active_snapshots                                 │
│    3. Release any held resources                                                │
│                                                                                  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

**State definitions:**
- **Idle:** No active transaction. Auto-commit queries run here.
- **InTransaction:** Active transaction. DML — and the destructive half of
  in-transaction `DROP TABLE`/`DROP INDEX` — buffered to the transaction's `pending`
  (`BtreePendingWrites`).
- **Failed:** The block met an error. Every statement is `25P02` except the exits:
  `ROLLBACK` ends the block, `COMMIT`/`END` ends it rolled back (tag `ROLLBACK`), and
  `ROLLBACK TO SAVEPOINT` clears the abort and leaves the block open (§ The transaction
  session).

### The socket and the response path

**The socket is configured, not merely accepted, and the response path is
buffered.** `set_nodelay(true)` is called on every accepted connection —
Postgres's own `pq_init` posture (FATAL when it fails): a response is several
small messages with no read between them, the exact shape Nagle + the peer's
delayed-ACK holds ~40 ms for, so a silently Nagle'd connection serves every
query ~70× slow (measured: fuz_app's `db` suite 19.66 s → 0.265 s; the whole
gate set ~940 s → ~46 s) — a failure to set it therefore loses the connection
rather than serving it. And the responses go through a **`BufWriter`, flushed
before each read of the socket** (measured 1.4–1.5× on the
fixture harness) — a `BufWriter` and deliberately not a response-sized `Vec`,
so a large `SELECT` streams as the buffer fills. Every path that stops writing
and waits for the client must flush first or the two ends deadlock; each such
path is marked in `run_connection_loop`.

## Transaction Lifecycle

Transactions are managed by `BtreeStorageWrapper` over the shared `CowBtree`;
per-connection state lives in `BtreeTransactionState` (a captured snapshot plus a
`BtreePendingWrites` buffer).

- **BEGIN** — capture the current root as the transaction's snapshot and open an
  empty pending-writes buffer.
- **The first write** — take the writer and re-base: a fresh snapshot, and the
  directory and table-id counter a rollback restores, for the block and each savepoint
  (`rebase_transaction`).
- **DML** — INSERT/UPDATE/DELETE buffer into pending writes; the tree is not
  touched yet.
- **DROP TABLE / DROP INDEX** — the catalog `name_to_id` mapping is removed
  immediately (read-your-own-writes; a `DROP TABLE t; CREATE TABLE t` recreates
  cleanly), but the *destructive* deletion (rows, the `SYS_TABLE` directory entry,
  and index entries) is buffered and applied at COMMIT — so the tree is not touched
  until commit and a ROLLBACK restores the table in full.
- **CREATE TABLE / CREATE INDEX** — applied immediately (read-your-own-writes: a
  later statement in the transaction sees the new table/index), so ROLLBACK *undoes*
  rather than discards. An index build's entries for the rows committed before the block
  are published at once, and the block's snapshot then moves onto that root
  (`advance_transaction_snapshot`, after a rebuild too), so its probes and reads through
  the index find those rows; sound because no other connection publishes while it holds
  the writer. A rolled-back `CREATE TABLE`'s `SYS_TABLE` lands at an id
  `>= next_table_id` (restored on rollback), so it's excluded from the recovery scan —
  unless rows sit under the id, which the restore then stops above, and the rollback
  deletes the `SYS_TABLE` entry of every id it stays above, so the scan finds none of them
  (`restore_next_table_id`; § Schema Snapshot Semantics). A
  rolled-back **successful** `CREATE INDEX` (including a `PRIMARY KEY`'s auto-created
  index) has its def deregistered and its entries shed, so no phantom index survives the
  rollback — in the live tree: a save made while the block was open holds the entries,
  and a reopen hands no new index their number. A `CREATE INDEX` whose **build fails**
  leaves nothing to shed, because it registered nothing: `Storage::create_index` is
  all-or-nothing, so `BtreeStorageWrapper::create_index` derives its def, back-fills, and
  only then inserts into `index_defs` / `table_indexes`, exactly as `rebuild_index` does.
  That is what makes `StorageAdapter`'s undo note — taken only when the call returns `Ok`
  — the right gate ([ddl.md](ddl.md) § A failed index build). Guards:
  `btree/wrapper/tests.rs`'s `create_index_in_tx_rollback_sheds_phantom` and
  `create_table_pk_in_tx_rollback_no_phantom_under_id_reuse` call
  `note_txn_created_index` unconditionally, hand-simulating the adapter's `Ok` branch,
  the only branch that reaches the undo; the `Err` branch is covered from plain SQL,
  where `pglet`'s `a_failed_index_build_registers_no_storage_def` reads the registry
  directly and `a_failed_create_index_build_inside_a_block_leaves_nothing_to_roll_back`
  drives a failed build inside `BEGIN … CREATE INDEX … ROLLBACK`.
- **SELECT** — read-your-own-writes: scan the snapshot root and merge the pending
  buffer over it.
- **COMMIT** — the block holds the writer (from its first write); apply the whole buffer
  through `apply_batch` (one CoW pass, one root swap), then persist (fsync pages → write
  the alternate meta slot → fsync, the commit point → flip the active slot). Auto-commit
  statements run their single write through the same persist path, so a bare `INSERT`
  survives restart. A block that never took the writer staged nothing, and publishes and
  saves nothing.
- **ROLLBACK** — discard the pending buffer (a buffered DROP's rows are never
  deleted) and undo any immediate in-transaction `CREATE INDEX` that **succeeded**
  (shed its entries in one `apply_batch`); a failed build registered nothing and so has
  nothing recorded and nothing to undo. The BEGIN snapshot root is never mutated
  (copy-on-write), so concurrent readers pinned to it stay consistent. The directory and
  table-id counter are restored unconditionally, from the state the block took the writer
  on; a block that never took it ends restoring nothing (`end_unwritten_transaction`),
  since its `BEGIN`-time values are older than what other connections committed. On a store with
  durable state the rollback may then save a catalog: the restored one, where a save
  during the block made a catalog change of the block's durable, or else the durable one
  with its sequence counters raised (§ Schema Snapshot Semantics).
- **SAVEPOINT / ROLLBACK TO / RELEASE** — a savepoint clones the transaction's
  **pending state**, never the database: the `BtreePendingWrites` buffer plus the
  catalog-allocator values (`name_to_id`, `next_table_id`, `own_ddl_ops`), the
  `created_indexes` set, and the two derived RYOW probe accelerators. Everything an open
  transaction writes is buffered, which is why that is enough; the two exceptions are the
  same two a whole-block rollback already handles — the catalog allocator, restored
  unconditionally to the *savepoint's* values (re-based with the block when it takes the
  writer), and an immediate `CREATE INDEX`, undone for the indexes built since the level
  opened. A block that never took the writer has nothing to restore at a `ROLLBACK TO`.
  `ROLLBACK TO` restores without popping (it is repeatable on Postgres, so the
  savepoint must survive to answer the next one); `RELEASE` pops without restoring.
  The stack is **anonymous and top-addressed** — savepoint *names* live in
  `pglet::TransactionSession`, which drives this stack one push per frame, so which
  frame a name means is decided once rather than in two places that can disagree.
  `MemoryStorage` implements the same three operations with a whole-store clone,
  the channel its `BEGIN` snapshot already uses, at O(database) rather than O(pending),
  on a stack **per connection**, so another connection's `SAVEPOINT`, block end or re-base
  never moves this one's levels (`EDGES.md` row 235). A restore replaces the store's data
  — never another connection's levels — and is sound only because the block re-takes its
  clones when it takes the writer and no other connection writes while it holds it; a
  block that never took it restores nothing. Every `DO` statement takes one such level
  too, with no name, for its own undo; what the copy costs is § Known bottlenecks.

### The transaction session — one state machine, every host drives it

The aborted block, the frame stack a rollback unwinds, whether the block holds the
writer and the unconditional restore that licenses, and the exit-statement classifier
live in `pglet`'s
`TransactionSession` — a **value type** with methods over
`(&mut Schema, &mut StorageBackend, conn_id)`, because the native server never
holds a `Database` (it calls `into_parts()` at startup and puts the halves
behind separate mutexes) while the embed facade and the wasm bindings do. Two
copies would disagree about **committed data**: a host that tracks no aborted block
runs `BEGIN; INSERT 1; INSERT 2; INSERT 1 (23505); SELECT 1; INSERT 3; COMMIT`'s
statements after the failure, answers tag `COMMIT` and leaves `1, 2, 3`, where 18.6
answers `25P02` twice, tag `ROLLBACK`, and no rows. The shape encodes six rules, the
sixth — what fails a block — after the list:

1. **The gate that admits a statement is not the dispatch that ends the block.** The
   gate runs ahead of parse analysis and reads the **lexer**, so a leading comment
   does not hide an exit (`classify_transaction_exit`); what an *executed* statement
   did to the block is decided after the parser has accepted it.
2. **That decision reads a carried signal, not the command tag.** `ROLLBACK TO
   SAVEPOINT` reports tag `ROLLBACK`, exactly as 18.6 does, so a tag-keyed host would
   discard every write since `BEGIN` and report success. `QueryResult` carries a
   `TransactionCommand` and the hosts dispatch on it; the npm adapter reads the
   engine's `transactionDepth()` rather than mirroring the tag.
3. **The frames are a stack**, one per open level: `SAVEPOINT` pushes, `RELEASE` pops,
   and `ROLLBACK TO` unwinds to one. The names live in the session and nowhere else,
   and the storage backends expose only an anonymous top-addressed stack the session
   drives in lockstep (above), so which frame a name means is decided once.
4. **A commit of an aborted block rolls it back**, in the session rather than only at
   the SQL gate, because a host can commit programmatically (`Database::commit`) and
   the two entry points must not disagree about what a `COMMIT` does.
5. **An abort ends nothing, and releases nothing another connection meets.** `COMMIT`,
   `ROLLBACK` and a disconnect drop the pending buffer and `ROLLBACK TO SAVEPOINT`
   restores it from the savepoint's snapshot, each releasing exactly the key claims whose
   writes stopped existing — but an abort ends nothing, because a `ROLLBACK TO SAVEPOINT`
   can still hand the buffer back, and the block keeps the writer until it ends, where
   18.6 releases a failed transaction's locks at the error (`EDGES.md` row 230).
   `TransactionSession::fail` carries the storage handle and suspends the claims as
   18.6's two arms release locks — an aborted block claims what its innermost open
   savepoint holds, and nothing with no savepoint open — which no other writer reaches
   while the writer is held.

**The session's settings ride the same frames.** The settings pglet acts on
(`lock_timeout`, `default_transaction_isolation`) and the block's isolation level live in
`SessionSettings`, beside the session rather than in it, and every host hands each
transaction command's outcome to `TransactionSession::carry_to_settings`: a `BEGIN` opens
their block at its level, a savepoint saves them, a `ROLLBACK TO` puts back what its
savepoint began with, a `RELEASE` keeps them, and a block's end keeps them only for a
`COMMIT` of a healthy block that succeeded — 18.6's `AtEOXact_GUC` / `AtEOSubXact_GUC`.
Each connection holds its own — a socket's on the native server, a logical connection's
on an embedded `Database` — so another connection's `SET` neither moves a block's settings
nor is taken back by its rollback. Two are outside this: `application_name` is not
transactional (`EDGES.md` row 236), and a multi-statement `Query` opens no block, so a
`SET` in one stands past a later statement's failure (row 68).

**What fails a block is an error, not a statement.** 18.6 aborts the
transaction from its one error-recovery path, so an error raised at *any*
extended-protocol phase fails an open block — a `Bind` refusing a parameter (a binary
or text value, the parameter count, an unknown statement), a `Describe` or `Execute`
naming a statement or portal that does not exist — just as a failed `Parse` or a
failed statement at `Execute` does: `Sync` answers `[E]`, the next statement is
`25P02`, and the `COMMIT` rolls back what the block wrote before the error. On the
native server a block is failed through `Connection::fail_transaction` (and so
`TransactionSession::fail`): `execute.rs`'s `format_error` and the copy-in handler call
it where they raise, and `handle_message` calls it for the error any other extended
`dispatch` arm returns, where it arms skip-until-`Sync`. It is a no-op with no healthy
block open, so the same error outside a block leaves `[I]`. A reply that carries no
error — a `NoticeResponse`, a `Close` naming nothing, a `Flush` — leaves the block
healthy on both engines. The embed
facade's `Database::execute_bound` — the wasm bindings' `queryParams` — does the same
when its host's binding callback fails, which is that host's `Bind`.
`handlers.rs`'s `an_error_at_any_extended_phase_fails_an_open_transaction_block` pins
each of those phases and the idle control,
`a_reply_with_no_error_leaves_an_open_transaction_block_healthy` the no-error replies,
and `pglet`'s
`a_binding_failure_fails_the_open_block` the facade's. None of this changes a pipeline
with no open block, whose `Execute`s pglet commits one at a time and 18.6 does not
(`EDGES.md` row 141).

**The session is one per `Database`, not one per `conn_id`.** The native server gives
each connection its own and the wasm bindings drive one connection; on one embedded
`Database`, while one logical connection holds a block, a second one's `BEGIN`,
`COMMIT` or `ROLLBACK` — as SQL through `execute_as` or through `Database::begin` /
`commit` / `rollback(conn_id)` — is refused `55P03` (`EDGES.md` row 210's residual), as
is its write while the block holds the writer (row 229). `Database::into_parts` ends no open block
either: its staged writes travel with the storage half (row 216). A `Database` built for one
connection (`Database::with_btree_for_one_connection` — the wasm `PgletBtree`'s, and every
fork of one) refuses another connection's `BEGIN` and write `0A000` where no `55P03` above
answers first, and runs its reads; its commits encode no catalog, since nothing reads one —
it is in memory, and no other connection's block compares the commit stamp a catalog write
moves. The session **settings**, unlike the session, are one per `conn_id` on an embedded
`Database` as well.

What a rollback does to a sequence counter — and what it deliberately leaves
alone — is [sequences.md](sequences.md)'s.

A `CREATE INDEX` inside an open block sizes each entry it enters, which evaluates
the key expression over every row the table holds, the block's own writes included, so
a poison row the block wrote raises at the `CREATE INDEX` — `22012` for a division by
zero, then `25P02` for what follows and the `COMMIT` tagged `ROLLBACK`, as 18.6
(`EDGES.md` row 97, closed; nothing pins the closure).

## Schema Snapshot Semantics

Row **data** reads use the per-transaction snapshot root, but the **catalog is not MVCC**:
the schema (`Mutex<Schema>`) and the storage name→id map are single shared structures
mutated in place across all connections. The schema clone taken at `BEGIN` exists only to
restore the catalog on **DDL rollback**, not to isolate reads — every table resolution reads
the *live* map. So a concurrent connection's DDL is visible immediately, even uncommitted:
an in-transaction DROP removes the shared name mapping before commit (a cross-connection
catalog dirty read). On the native server a write from another connection waits for the
block holding the writer and then resolves against the catalog it left, and so does any
statement whose plan fails *because of* the block's uncommitted catalog; the embedded
`Database` refuses both `55P03` (§ Why Single-Writer?); a **read** whose plan succeeds
resolves through a block's uncommitted DDL as it stands, a table the block dropped included
(`EDGES.md` row 221), per-transaction schema isolation being unimplemented.

The dirty read has a durable face: every save but a rollback's counter save (below) encodes
the live catalog, so a save while a block is open — the block's own `sync` on a host that
has one; another connection's write waits for the block, or is refused `55P03` on the
embedded `Database` — makes durable the DDL the live catalog then holds (what the block has
run so far, less what a `ROLLBACK TO SAVEPOINT` already undid), and a reopen before the
block ends holds it (`EDGES.md` row 211).
A rollback that ends the block — `ROLLBACK`, a `COMMIT` of an aborted block or one meeting a
refused write it could drop, a server client's disconnect, an embedded `Database` dropped
with the block open — restores the catalog and saves it when a save has written a catalog
since the block took the writer and the catalog the durable state holds differs both from
the one the block was re-based on and from the one just restored, so a reopen after it shows none of the
DDL it rolled back. The comparison is of the catalogs, not of which statements ran, so it
sees a change a host made through `Database::schema_mut` as it sees DDL. It sets aside a
sequence counter advanced in place: `nextval` and `setval` are not rolled back, so a durable
counter behind the live one is no change of the block's to take back; a counter cell a
statement of the block replaced — one `Plan::swapped_sequences` lists, or one a `DO`
statement reports it replaced — is compared. A save that wrote
none of the block's changes — before the first, or after a `ROLLBACK TO SAVEPOINT` undid
them — leaves the rollback no restored catalog to save, nor does a block no save reached,
even where the durable catalog already differed from the live one when it took the writer.

**A counter is saved by a rule of its own.** A `nextval` hands its value out as it runs,
so where no restored catalog is saved, the rollback saves the **durable** catalog with
each counter raised to the live one of its name that stands ahead of it — standing in for
18.6's `nextval`, which logs its advance ahead of use — and nothing else the live catalog
holds, so it writes none of the block's DDL into the durable catalog. It writes nothing where
no counter is raised; where the live catalog differs from the durable one in anything but
counters, since the save writes the current tree beside the catalog and a structure no
save has written — a change a host made through `Database::schema_mut` outside any block —
may have entries there (the footprint measured with the check removed, an enum re-created
with its labels reordered under a unique partial index and a reopen accepting a duplicate
under it, came through another connection's open block, a route the writer keeps closed);
and in the rollback a refused `COMMIT` runs, where the device has
just refused a write. The rule, its three choices and the footprints that make that check
necessary are [sequences.md](sequences.md) § A sequence counter is not rolled back.

A rollback's save, like every save, writes the store's current tree, which can hold index
entries the catalog it writes does not name: a block's `CREATE INDEX`'s that a
`ROLLBACK TO` undid after a save, and a committed index's, which a block's `DROP INDEX`
leaves until its `COMMIT`. A reopen seeds
its OID counter past every index number the tree holds an entry under
(`Database::reattach_persisted_indexes`), so no new index is handed such a number
(`EDGES.md` row 211).

A reopen after the rollback can still read otherwise than the connections
do: it lacks what no save has written since — the draws of a failed autocommit statement, a
host's change made before the block, among others. Where a reopen can hand an issued value out again is `EDGES.md`
row 214. The comparison runs only on a store with durable state, and there only past a save
since the block took the writer; the in-memory B+tree answers every one of these decisions "nothing durable"
without encoding a catalog. A refusal of the rollback's save fails the store (§ Crash
Recovery). `ROLLBACK TO SAVEPOINT` restores the catalog without saving it, so DDL it undid
that a save made durable stays on disk until the next save that encodes the live catalog —
the block's own `sync`, or the one the block's end makes — and a reopen in that window
holds it.

**The rollback restore is unconditional for a block that took the writer, and absent for
one that did not.** Every rollback — `ROLLBACK`, `ROLLBACK TO`, a caught `DO …
EXCEPTION` or a failed `DO`, a refused or aborted `COMMIT`, a disconnect, a drop — restores the frame's
catalog and the storage's directory and table-id counter whole: they were re-based when
the block took the writer, and nothing but the block has changed them since. A block that
never wrote restores nothing.

| Scenario | Behavior |
|----------|----------|
| Tx A creates table `foo`, commits | Every connection sees `foo` immediately |
| Tx B (started before A committed) queries `foo` | Works — the catalog is live, not snapshotted |
| Tx C drops table `bar` (even before commit) | An already-running scan completes (pinned data snapshot). C's next statement on `bar`, and another connection's next **read** of it, errors `42P01` — the shared name mapping is already gone (`EDGES.md` row 221); another connection's next **write** waits for C's block and runs against what C's end leaves: `42P01` after a `COMMIT`, the write after a `ROLLBACK` — on the native server; the embedded `Database` refuses that write `55P03` while C's block holds the writer (`EDGES.md` row 229) |
| Tx E drops table `bar`, **rolls back** | `bar` restored in full — rows and indexes intact (the drop was buffered) |
| Tx F creates table `t2`; another connection writes into it; F **rolls back** | on the native server the write waits for F's block (the writer), then answers `42P01`: `t2` is gone (the embedded `Database` refuses it `55P03` while F's block holds the writer). The storage rollback still puts the table-id counter back only as far as no id it would hand out again has a row, and deletes the `SYS_TABLE` entry of every id it stays above (`restore_next_table_id`) — a guard for a write staged without the writer, which `cross_connection_hits` counts |

## Store versioning — the `PGM*` magic

The persisted catalog/value shape carries a four-byte magic that bumps whenever
the postcard bytes' shape *or meaning* moves; a store written under an older
magic fails loud on reopen (pre-stable, no migration shims). **A magic is never
reused**: a bump takes the next fourth byte in the order `2`–`9`, `A`–`Z`, `a`–`z`,
skipping any byte an earlier magic holds (`T`, v1's `PGMT`), so `PGMS` was followed
by `PGMU`, `PGMU` by `PGMV`, and the next is `PGMW`; past `z` the third byte moves on and the fourth runs `0`–`9`, `A`–`Z`,
`a`–`z` again (`META_MAGIC`'s doc; `the_magic_scheme_never_reuses_a_magic` and
`the_current_magic_is_no_superseded_one` guard it). The ledger:

- `PGM7 → PGM8` — `Column.varchar_length` added (the `varchar(n)`/`char(n)`
  length typmod).
- `PGM9` — `Column.identity`.
- `PGMA` — composite FK: `ForeignKey`'s columns became a `Vec`.
- `PGMB` — `ForeignKey.on_update`.
- `PGMC` / v12 — `JsonbValue::Number` moved `f64` → `PgNumeric`.
- `PGMD` / v13 — canonical jsonb object key order: the **first** bump where the
  postcard *shape* is unchanged and the *meaning* of the bytes moved, so an old
  row would decode cleanly and compare wrong. The ordinals the later entries of
  that kind carry are counted from here.
- `PGME` / v14 — the catalog `Index`'s **constraint marker** (`Uniqueness`
  widened from `{unique, enforced}` to `{unique, enforced, constraint}`,
  recording whether a declared `PRIMARY KEY`/`UNIQUE` owns the index).
- `PGMF` / v15 — the catalog `CheckConstraint`'s `name` became a total
  `String`, so no reconstruct-a-missing-name compatibility branch exists
  ([names.md](names.md)).
- `PGMG` / v16 — the catalog **sequence** became a record
  (`{last_value, start, owner}`) rather than a bare `i64`
  ([sequences.md](sequences.md)).
- `PGMH` / v17 — the identifier case-fold changed which SQL text reaches a
  persisted name (the second bump of `PGMD`'s meaning-moved kind;
  [parser.md](parser.md)).
- `PGMI` / v18 — the catalog `ColumnDefault`'s `Expr` catch-all, a
  column `DEFAULT` that may be any expression stored unevaluated.
- `PGMJ` / v19 — a user `ENUM` type gained a catalog **OID**
  (`Schema.enums` maps a name to an `{oid, labels}` record rather than to a bare
  label `Vec`). `pg_type` and `pg_enum` are joined on it, so it has to be
  persisted rather than derived per session for the same reason a table's is:
  two reads of one database must name the same type by the same number. It
  shares the OID counter with tables and indexes — `Schema::next_oid`, runtime state
  seeded lazily, so the deciding site is `executor::helpers::next_object_oid`, the
  shared seed (`generate_object_oid` advances the counter through it) — which is why
  that seed reads the enum map too — leaving it out would let
  a table created after a reopen take an OID an enum already holds, and then
  `pg_class` and `pg_type` would answer for one OID with two objects.
- `PGMK` / v20 — an enum value's **order** moved from its labels' bytes
  to its members' declared positions ([enums.md](enums.md)). The third bump of
  `PGMD`'s meaning-moved kind, and the shape is unchanged: what a pre-v20 store holds
  is two populations *derived* under the old order and re-derived on no open — the
  rows a `CHECK` over an enum column admitted, and the rows a **partial** index whose
  predicate orders an enum was built over. The first leaves rows the table's own
  constraint now rejects; the second is the silent one, since a **unique** partial
  index built under the old rule holds neither of two rows a later probe must find,
  and so reports no conflict for a duplicate it should refuse.
- `PGML` / v21 — the same two populations again, over a **wider set of
  stored expressions**. The member order now reaches an enum under a `COALESCE`, a
  `CASE`, a `greatest`/`least` or a `nullif` ([enums.md](enums.md) § An expression's
  enum is its type's), so a v20 store's `CHECK` admitted — and its **partial** index was
  built over — whatever the *label bytes* chose for such a predicate, frequently the exact
  complement of what this build chooses. The fourth bump of `PGMD`'s meaning-moved kind,
  and the shape is unchanged.
- `PGMM` / v22 — the same two populations, over that same set widened once
  more. An `ARRAY[…]` expression answers with its elements' enum, an `unnest(…)` passes
  that through, and an `op ANY`/`op ALL` reads the array operand as well as the scalar one
  ([enums.md](enums.md) § An `ARRAY[…]` is not an enum array column) — so a v21 store's
  `CHECK (ARRAY[m] < ARRAY['ok'::mood])` admitted, and its partial index over
  `WHERE ARRAY[m] > ARRAY['sad'::mood]` was built over, whatever the label bytes chose.
  The fifth bump of `PGMD`'s meaning-moved kind, and the shape is unchanged.
- `PGMN` / v23 — the catalog `Column` gained `temporal_precision`, the
  sub-second `(p)` of a `timestamp`/`timestamptz`/`time`/`interval` column
  ([temporal.md](temporal.md) § Sub-second precision). A **shape** bump, of `PGM7 →
  PGM8`'s and `PGM8 → PGM9`'s kind rather than `PGMD`'s: `#[serde(default)]` on an
  appended-last field buys nothing in a non-self-describing format, since postcard reads
  exactly the fields the struct declares with no per-struct length, so a v22
  `SYS_CATALOG` decoded by this build takes one field too many out of every `Column` and
  reads it from the next one in the `Vec`. Measured on a real v22 store, the reopen died
  with `failed to deserialize schema: Hit the end of buffer, expected more data` — the
  loud shape of that, and not the guaranteed one.
- `PGMO` / v24 — a `date` / `timestamp` value — a row value, or a constant in a
  stored `Expr` — persists as the Postgres binary format's day / microsecond count, with
  its `infinity` sentinels, rather than as `time`'s field tuple, and a `date` / `timestamp`
  index key is that same count, where a v23 row holds `time`'s field tuple and a v23 index
  entry a Julian-day or Unix-epoch key. A **shape** bump ([temporal.md](temporal.md)
  § `infinity` and `-infinity`).
- `PGMP` / v25 — an `AND`/`OR` list of more than 64 operands in a stored
  expression is balanced under a list root, where a v24 store holds the left-deep chain,
  so its partial-index predicate does not equal the same text parsed fresh. The sixth
  bump of `PGMD`'s meaning-moved kind, and the shape is unchanged.
- `PGMQ` / v26 — a **shape** bump of the page format, not the catalog's. A
  leaf value too long to keep inline is stored in a chain of overflow pages, a page kind
  no earlier build writes, and the leaf holds a reference to it. A v25 store would decode,
  by construction (none of its value lengths carries the reference's tag), and is refused like every
  other superseded magic rather than kept as a compatibility branch.
- `PGMR` / v27 — a **shape** bump of the meta slot and the free-list
  chain. The slot gained `freelist_crc`, the head chain page's checksum, and each chain
  page's body its successor's checksum ahead of the id count, so a power-loss image
  whose slot landed over a chain page that did not reads as a short free list rather
  than a stale one naming live pages (`EDGES.md` row 202).
- `PGMS` / v28 — a **shape** bump of the meta slot. Pages superseded and
  not yet freed are logged apart from the free list rather than folded into its chain,
  which every save rewrites whole: the slot gained `retired_head`, `retired_crc`,
  `retired_max` and `retired_floor`, carries the log's newest entries after its
  metadata, and the log spills into chain pages written once (`EDGES.md` row 203).
- `PGMU` / v29 — a string read as `bytea` is `byteain`'s two formats, and an
  array literal is read by 18.6's array grammar — a backslash in an unquoted element is
  an escape, a Unicode space beside an element is the element's character rather than
  trimmed, a literal that grammar refuses (an empty element, a brace or a quote inside an
  unquoted element) is refused where a v28 build read it as elements, and a nested literal is
  refused (`'{{a,b}}'`) or, holding no element (`'{{}}'`), the empty array.
  So a stored expression holding one — `'cafe'::bytea`, a `bytea` default string,
  `'{a\,b}'::text[]`, `'{a,,b}'::text[]` — evaluates to another value, or to an error,
  than the one a v28 store's `CHECK` admitted rows under and its partial indexes were
  built over; measured on a v28 store, a unique partial index `WHERE b = 'cafe'::bytea`
  accepted a duplicate beside a stored row. The seventh bump of `PGMD`'s meaning-moved
  kind, and the shape is unchanged.
- `PGMV` / v30 — **current**: two evaluations moved at once. `round(double precision)`
  rounds a tie to the even integer (C's `rint`, 18.6's `dround`) where a v29 build rounded
  it away from zero, and a `real` / `double precision` read from text, or converted from
  `numeric`, is refused `22003` outside the type's range where a v29 build answered the
  `±Infinity` / `0` it rounds to ([numeric.md](numeric.md) § Math functions, `EDGES.md` row
  149). So a stored expression holding either — an expression index over `round(f)`, a
  `CHECK (r < '1e39'::real)`, a `CHECK (r > 1e-50::numeric::real)` — evaluates to another
  value, or to an error, than the one a v29 store's `CHECK` admitted rows under and its
  indexes were built over; measured on a v29 store, `CREATE UNIQUE INDEX … ON ru
  (round(f))` holding `2.5` admitted `2.4` beside it under the new build, where a fresh store
  answers `23505`. The eighth bump of `PGMD`'s meaning-moved kind, and the shape is
  unchanged.

**Which bumps moved meaning.** The meaning-moved ones — where the postcard *shape* is
unchanged, so an old row decodes cleanly and compares wrong — are `PGMD`, `PGMH`, `PGMK`,
`PGML`, `PGMM`, `PGMP`, `PGMU` and `PGMV`; each of those entries says so, and the rest name the field, type
or page format they moved. The **refusal** does not depend on the classification:
`STALE_MAGICS` lists every superseded magic (`PGMT` v1 through `PGMU` v29), and its guard
derives that expectation from `META_MAGIC` rather than from the array, so a superseded
version fails loud by name whether or not anything ever wrote it — rather than as
"unrecoverable corruption".

**What does not bump: a key for a value the encoder had no arm for.** An index entry's
key bytes are derived (`Value::encode_index_key`), not postcard, and a new arm of that
encoder — a float's, a `jsonb`'s, an array's — moves no stored byte's meaning as long as
no store holds a key segment of that type. That holds by the doors, not by inspection:
every write of an entry — a write's maintenance, a build's back-fill, a rebuild, a GIN
decomposition — asks the one encoder, a refusal fails the statement, and a failed write
or build commits nothing. The catalog does not move either: such an index's `Index`
record has the same shape whichever arms the encoder has. Changing the encoding of a type that already has one is the other
case, and is a meaning-moved bump ([indexes.md](indexes.md) § A float key, § A container
key).

## Configuration

There's no single config struct; knobs are set via the relevant constructors
(defaults): page size 4 KB (`PAGE_SIZE` constant); vacuum threshold ~100 pending
pages (`with_vacuum_threshold()`); block cache ~2000 pages / ~8 MB
(`open_with_cache_size()`).

## Open limits and the durability instruments

A long row is stored whole but written whole: an `UPDATE` of any column of a row held in an overflow
chain writes a fresh chain, and a chain's pages are read through the page cache.
Bulk-load optimization is open (write amplification
is measured — `deno task bench:commit` records fsyncs/writes/bytes per commit in
`benches/phase1_results.json`). On the durability side, fault-injection crash tests cover
mid-commit recovery (`crates/pglet/tests/crash.rs`), a real-process `kill -9` + soak
suite (`pglet_tests` `test_crash`) covers the production binary on a real file, a
randomized corruption fuzz asserts recover-or-fail-loud, and a coverage-guided
`cargo-fuzz` recovery target (`fuzz/`, `deno task fuzz`) mutates a valid double-meta
image against the same invariant — and, with the leaf and overflow pages' checksums
re-stamped so the mutation reaches the overflow-chain walk, against an error-not-a-panic
one. `crates/pglet/tests/durability.rs` replays power-loss images of recorded DDL and DML
workloads — at every `sync` boundary, the pending writes landing in every tried subset, a
torn page included — checks that no write between durable slots lands on a page the
durable slot reaches, and `SIGKILL`s a real embedding process inside DDL; and a root-gated harness
(`tools/durability/power_loss_test.sh` + the `test_power_loss` driver, `deno task
test:power-loss`) runs a loopback + `dm-flakey` `drop_writes` device that loses
writes it acknowledged, asserting every acked commit survives as a clean prefix.
Tested, not proven: each instrument checks the workloads it runs (`PLAN.md` § 3). (Data-page checksums verify on
read — `btree/double_meta.rs` checks each page's CRC before trusting its bytes.)

## Known bottlenecks / scaling limits

The CoW + double-meta + atomic-root architecture is sound; these are
*implementation-level* limits to weigh, not reasons to rethink the design — and the
embedded/WASM single-threaded target is largely unaffected by them, except the savepoint
copy below, whose B+tree column is the wasm build's backend too.

- **Read scaling is hardware-bound, not lock-bound.** Both read-path locks are off
  the hot path: the page cache is sharded (`ShardedPageCache`) and the snapshot
  refcount (`active_snapshots`) is atomic counts under a shared read lock — the
  mechanism and its vacuum-safety invariant are in the implementation reference
  (`crates/pglet_storage/src/btree/CLAUDE.md`). Concurrent-read throughput (`deno
  task bench:reads`) rises with thread count and peaks near the physical core
  count, then flattens on CPU/memory bandwidth — not on the cache.
- **Scans materialize fully.** `collect_range` returns the whole result as a
  `Vec` with a per-row `value.clone()` — no streaming at the storage layer, so a
  full-table `SELECT`/aggregate holds the whole table resident. Memory is linear
  in the rows scanned (order ~0.7–1 GB of server RSS per 1M-row full scan at a
  representative row width). The large-dataset soak (`deno task test:soak`, the
  `test_soak` harness) confirms every invariant holds through 1M rows at roughly
  0.8 GB peak RSS, so the fully-materialized scan is a *memory ceiling* far above
  the embedded / test-fixture working set (a single full-table aggregate reaching
  machine RAM, tens of millions of rows), not a correctness limit. Streaming scans
  are deferred until a consumer reaches that ceiling (the natural shape when one
  does is volcano-model iterator operators end-to-end — storage `scan` yielding
  an iterator, `QueryResult.rows` streamed to the wire as produced).
- **Input is buffered too.** The server reads each client message into a
  growing buffer capped at 64 MB (`pglet_server`'s `connection.rs`); a large
  `VALUES` INSERT parses in a flat iterative loop with no per-row recursion
  ([parser.md](parser.md)), so a big batch materializes its rows but never
  overflows the stack — there is no separate streaming path to keep in sync.
  Streaming `COPY FROM STDIN` (processing rows as they arrive rather than
  buffering the whole payload) is the natural follow-up if a consumer hits the
  cap.
- **Write path is CPU-bound and flat across batch size.** The apply loop copies a
  page at most once per batch (`Arc::make_mut` + in-place mutation of created pages),
  and the in-transaction uniqueness/equality probe (`index_lookup_in_tx`) is O(log n)
  — a committed-index probe at the snapshot root plus a per-transaction pending index,
  rather than a full table scan (which would be O(rows × table_size) for a batch of
  enforced-PK/UNIQUE inserts). It is maintained through updates and deletes as well as
  inserts, so one `UPDATE` does not put its table on the re-encoding scan for the rest of
  the block (an insert-only index would: measured through the cross-connection claim walk,
  which reads the same index and finds nothing under the writer, a 500-row insert beside a
  20 000-row updated block takes 1 158 ms against 4 ms). Three routes still
  fall back to that scan: column DDL and a key-encoding/predicate failure mark the one
  table, while an index created or dropped in the block falls **every** table in it
  back. One residual:
  `gin_lookup_in_tx` (the `@>` containment probe within a transaction) still full-scans
  — a GIN pending-merge over posting-list intersection is a follow-up, so a GIN probe
  inside a large write transaction is O(table_size).
- **A savepoint level is a copy, and every `DO` statement takes one.** `SAVEPOINT` clones
  what a `ROLLBACK TO` restores (§ Transaction Lifecycle): on the B+tree the
  transaction's staged writes, O(what the block has staged); on `MemoryStorage` the whole
  store, O(database); and on both the session's `Schema` frame, O(catalog). A `DO`
  statement takes the same level and a `Schema` snapshot before its first statement,
  because a failed one must be put back whole (`executor::execute_do_block`) — so a `DO`
  that succeeds pays them as well, in auto-commit as in a block; without that undo only a
  `DO` with an `EXCEPTION` clause would need them, for its subtransaction. In auto-commit on the B+tree
  the level is a copy of the statement's own bracket, which has staged nothing yet, so
  the difference there is the catalog clone — measured: over a one-table catalog an
  auto-commit `DO` costs what its `INSERT` costs (0.14 ms both).
  Measured (release native server, one connection over `psql`, a store holding a
  200 000-row table and 300 small keyed tables, each statement inserting one row into a
  small table; ms per statement, the round-trip included — in auto-commit the median of
  three runs of 200 statements, in the block the range over five timed statements in
  each of two runs):

  | | B+tree | `MemoryStorage` |
  | --- | --- | --- |
  | auto-commit `INSERT` | 0.33 | 0.13 |
  | auto-commit `DO` | 0.59 | 65 |
  | auto-commit `DO … EXCEPTION` | 0.58 | 66 |
  | in a block that has written 100 000 rows: `INSERT` | 0.03–0.09 | 0.04–0.29 |
  | … `DO` | 33–57 | 61–140 |
  | … `DO … EXCEPTION` | 36–40 | 63–102 |
  | … `SAVEPOINT` | 21–46 | 40–136 |

  Without the level a handler-less `DO` costs what its statements cost — 0.32 and 0.13 in
  auto-commit, 0.03–0.10 and 0.04–0.10 in the block, measured with the undo removed — and
  a failed one keeps its body's catalog writes (`EDGES.md` row 249). Replacing the copy with a mark — a position a rollback
  truncates back to — needs an undo log the staged buffer and the memory store do not
  keep: the buffer is keyed by row and overwrites a row's earlier staged image, and the
  memory store writes in place. A cheaper copy needs no log — `MemoryStorage`'s tables
  shared and copied on write would make its level O(tables) plus the tables written — and
  is not built.
- **An emptied leaf is never reclaimed.** A delete that empties a leaf (`DROP TABLE`,
  `TRUNCATE`, a `DELETE` of every row, a queue deleting its oldest row) writes the empty
  leaf back, and no path removes it from its parent or merges it into a sibling. A row's
  key is its table id and a row id that only grows, and a dropped table's id is not
  reused, so in the workloads measured no later write lands there: about 4 KiB of file
  per emptied leaf, which a per-test create-and-drop pattern accumulates. Space, not rows
  (`EDGES.md` row 209).
- **Two `dyn` hops per page** (`Box<dyn PageStore>` over `Box<dyn BlockIo>`).
  Negligible vs I/O; an enum over the closed `{memory, double-meta}` set would
  erase it if it ever shows up in a profile.
