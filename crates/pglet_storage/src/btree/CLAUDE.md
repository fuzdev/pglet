# B+tree Storage

**Status:** CoW B+tree is the primary storage engine. Persistence is a single
crash-safe layout — `DoubleMetaStore` over a `BlockIo` — shared by native (over
a file) and browser (over an OPFS handle); there is **no WAL**. A commit writes its
CoW pages, fsyncs, writes the alternate meta slot and fsyncs again (the commit point);
the slot is not the whole guarantee — collection frees no page the durable slot reaches,
and the free-list chain is checksum-linked from it (§ Single-file durable layout). The
in-memory structure and MVCC snapshot reads are solid. This file is the storage-engine implementation reference.

## Module Structure

```
btree/
├── mod.rs         # Module exports
├── key.rs         # BtreeKey: variable-length data-row + index-entry encoding
├── page.rs        # Page types and serialization
├── store.rs       # PageStore trait + PageStoreMetadata
├── block_io.rs    # BlockIo trait (positioned read/write/sync/len) + FileBlockIo + CountingBlockIo
├── mem_block_io.rs # In-memory BlockIo test double for block_io/double_meta unit tests (#[cfg(test)])
├── double_meta.rs # Single-file double-meta PageStore over a BlockIo (no WAL)
├── system.rs      # 0x02 system-entry codecs (SYS_CATALOG chunks / SYS_TABLE) + free-list chain (PGM5) + retired-page log (PGMS)
├── page_cache.rs  # Sharded write-through page cache (DoubleMetaStore's read cache)
├── cow.rs         # Copy-on-Write B+tree with MVCC
└── wrapper/       # Storage impl over CoW — mod (coordinator + Storage impl) +
                   #   catalog / index / transaction / snapshot submodules
```

(`cow.rs` and `key.rs` each have an adjacent `cow/tests.rs` / `key/tests.rs`
test module.)

The browser's OPFS `BlockIo` (`OpfsBlockIo`) lives in the `pglet_wasm` crate, behind its
opt-in `opfs` feature;
`DoubleMetaStore` works over any `BlockIo` (it holds a `Box<dyn BlockIo>`), so the
same store runs on both targets.

## Key Design

- **BtreeKey**: **variable-length**, byte-comparable, over a single keyspace
  with a leading discriminant byte. Three kinds share one tree (one `root`), so
  the MVCC seqlock covers index + system state with no new roots/snapshots:
  - **Data row** (`0x00`): `[0x00][table_id: u32 BE][row_id: u64 BE]` — 13
    bytes; the `(table_id, row_id)` layout under the data discriminant.
  - **Index entry** (`0x01`): `[0x01][index_id: u32 BE][escaped value … 0x00 0x00]
    [row_id: u64 BE]` — a secondary-index entry with an empty stored value. The
    value is **order-preserving escape-terminated** (FoundationDB/Cockroach style:
    `0x00`→`0x00 0xFF`, terminate `0x00 0x00`), so byte-order == value-order even
    across different value lengths. The executor uses both equality probes and
    single-column ordered/range scans — the order-preserving framing enabled the
    latter with no format break (`index_range_start` / `index_range_end` map a bound's
    blob and inclusivity to the half-open `[start, end)` ends, driven by
    `index_range_lookup`; the start is never the band's own, which sorts below a NULL
    key — `index_keying::encode_range_bounds`). An
    equality probe scans a value over `[index_start(id, val) .. exclusive upper]`
    (the upper bound is `index(id, val ++ 0x00, 0)`, since `val` is a strict prefix
    of `val++byte`).
  - **System entry** (`0x02`): `[0x02][sub-kind: u8][id: u32 BE]` — 6 bytes; the
    catalog/table state, kept in the tree rather than the meta slot so it is atomic
    with the data it describes. Sub-kind `0x00` = a **catalog chunk** (`id` = chunk
    index): a slice of at most `CATALOG_CHUNK_SIZE` bytes of the postcard `Schema`
    blob — stored out of line like any long leaf value — reassembled on open. Sub-kind `0x01` = **table meta**
    (`id` = `table_id`): `(name, next_row_id)` — the table directory and its row-id
    counter in one entry. `SYS_TABLE` rides the data batch (insert / create / txn
    commit) so the directory + row-id counter persist in the same root swap and
    fsync as the rows; `SYS_CATALOG` is written by `commit_metadata` only when the
    catalog bytes it is handed changed — a DDL, a sequence draw (a `SERIAL` default's
    included), or a `setval` — keeping an autocommit `INSERT` that draws none at two fsyncs; a store
    built for one connection is handed none and writes none. On open,
    `BtreeStorageWrapper::recover` scans the two bands to rebuild the name→id map,
    counters, and catalog bytes.
  - The discriminant keeps the kinds disjoint (data < index < system), and the
    hand-written `Ord` is plain lexicographic on the encoded bytes. `BtreeKey` is
    **not `Copy`** (it owns a `Vec<u8>` for index values); clone where a move is
    required.

  Index **maintenance** and the **probe primitive** are built: the
  `BtreeStorageWrapper` holds index defs (`index_id` → table + key-column
  positions + `unique`) and emits index key-ops alongside data-row ops in the same
  `apply_batch` (one root swap, atomic). `index_lookup` / the snapshot's
  `index_lookup` probe at the current / snapshot root respectively. A **GIN** index
  (`IndexMethod::Gin`, derived from a **non-unique** single bare array/JSONB key
  column — uniqueness and key shape decide it too) is the same keyspace
  used as an **inverted index**: maintenance decomposes the row's array/JSONB value
  into **many** entries — one per element / JSONB key+value (`gin::gin_decompose`),
  each encoded exactly like a single-column key — and `gin_lookup` intersects the
  query keys' posting lists (`index_scan_at_root` per key + a sorted-merge `&`),
  returning a candidate superset the executor rechecks with the real `@>`. The
  value→bytes encoding is `Value::encode_index_key` (in `pglet_core::types`, shared
  by the storage probe and the executor): sign-flipped BE for signed
  ints/timestamps, raw bytes for text/uuid/bytea, a float as the `float8` it widens to in
  `float8_ops` order (`-0` keyed as `0`, every NaN as one key above `Infinity`), an
  array as its tagged elements in `array_ops` order, a jsonb in
  `jsonb_ops` order (the repo's `docs/indexes.md` § A container key).
  A container column is **not** GIN-only: GIN cannot enforce uniqueness, so a declared
  `UNIQUE` / `PRIMARY KEY` over a jsonb or array column mints a B-tree index over it
  (measured: `CREATE TABLE dj (id INT, j JSONB UNIQUE)` renders `… USING btree (j)`
  byte-identically here and on 18.6), keyed by that encoding, as a composite or
  expression key holding one is. `CREATE INDEX` refuses one shape at DDL time — a
  non-unique B-tree over a single bare container column, which the method derivation
  reads as GIN.
  The backend-independent
  row→key-bytes / row-in-index / parse logic lives **once** in
  `pglet_storage::index_keying::IndexKeying`, embedded by both the B+tree
  `IndexDef` (which adds `table_id`) and the in-memory one (which adds `entries`),
  so the two backends compute an index key through one implementation. Each
  composite-key segment carries a **presence tag** (`0x00` NULL / `0x01 ++ value
  bytes` present) so a NULL key stays distinct from an empty-but-present value
  (`''`, empty bytea) — Postgres semantics under `NULLS NOT DISTINCT`; the tag is
  order-preserving (`0x00 < 0x01`). Index
  defs are **not persisted in the meta slot** — on reopen the catalog re-registers each
  through `reattach_index` (`Database::reattach_persisted_indexes`), which restores the
  in-memory def and the table's index set and writes nothing: the entries are already in
  the tree, so there is no back-fill. The same step seeds the host's OID counter past the
  greatest index id the tree holds an entry under (`BtreeStorageWrapper::index_id_floor`,
  one right-to-left walk that stops at the first key it finds,
  `CowBtree::last_key_in_range_at_root`) — a band a dropped or
  rolled-back index left in a saved tree is under no catalog index, and must not be handed
  to a new one — and forgets a `SYS_TABLE` name the catalog lacks
  (`BtreeStorageWrapper::forget_tables`). An entry being **added** is keyed
  through `IndexKeying::encode_row_entries`, which refuses it `54000` where 18.6 would for
  its size (`pglet_sql::executor::index_row`) and where its key would not fit an internal
  page as a separator (`MAX_KEY_LEN`) — on the insert, update and new-index back-fill paths
  of both backends (a rebuild re-derives entries already sized), inside a
  transaction at the statement; retracting an entry keys through `encode_row_keys`
  (`docs/indexes.md` § An index entry is sized as 18.6 sizes it).
- **No version in key**: MVCC via root versioning
- **Single-writer**: Writes serialized via Mutex
- **Lock-free reads**: the `(root, version)` pair is published and read through a
  **seqlock** (a `generation` counter), so a snapshot never observes a torn pair

## Page Layout

- **Size**: 4KB (configurable via PAGE_SIZE constant)
- **Header**: 32 bytes (page_id, flags, count, next/reserved, checksum)
- **Types**: Free, Internal, Leaf, Overflow
- **Variable-length keys**: each on-page key is **length-prefixed** with a
  `u32` (Internal: `[key_len][key][child]`; Leaf:
  `[key_len][key][value_len][value]`).
- **Overflow chains**: a leaf value whose postcard encoding is over
  `INLINE_VALUE_MAX` (a quarter page), or whose entry would not fit a page with it
  inline, is stored **out of line** — a chain of `Page::Overflow` pages, each
  carrying up to `DATA_SIZE` bytes of the encoding (`count` is the payload length,
  `next_or_reserved` the next link) — and the leaf holds an `OverflowRef`: a value
  length of `OVERFLOW_TAG` (`u32::MAX`, which no inline value reaches) followed by
  the head page and the total length. In memory a leaf entry's value is a
  `LeafValue`, `Inline(Vec<Value>)` or `Overflow(OverflowRef)`, so an entry cannot be
  both. Per entry, not per column: a row is materialized whole anyway, so a chain
  carries the whole row. The chain is written by `apply_batch` into its working set
  (`write_chain`) and so is put, fsynced and published with the leaf that
  references it; a delete or a replace **releases** it (`release_chain`) — a
  pre-batch link superseded, freed by the same GC rule that frees the leaf, so a
  snapshot that still reads the old leaf still reads its chain; a link this batch
  created and dropped is never written. `get` and `collect_range` resolve a
  reference (`read_overflow`), so every caller above `CowBtree` reads `Vec<Value>`
  as before. A read and a release walk the chain the same way (`walk_chain`): the
  chain must be exactly `OverflowRef::page_count` links, full but the last, the last
  ending the chain, and the walk is bounded by the store's **allocation counter**, not by
  the reference — a reference claiming more links than the store has allocated pages is
  refused before a link is read, every link must be a page id below `next_page_id`, and a
  revisited link is a cycle — so a damaged reference or link cannot make the walk visit a
  page twice or gather more than one page's payload per page the store holds. That bound
  is the meta slot's word, so it holds because open checks the word: a slot whose
  `next_page_id` is past the last page the store can address, or whose `root`,
  `freelist_head` or `retired_head` is at or past its `next_page_id`, is refused as a torn slot is (the
  other slot is read, or open fails), and `page_offset` refuses any id past that last
  page — so two ids are two pages, and no id wraps onto another's bytes. Every other
  mismatch is `PageStoreError::CorruptChain` too. A release refuses a pre-batch link its
  batch has already released (two entries' chains sharing a link), as the walk refuses a
  link it revisits, and those two refusals keep a damaged chain's page from reaching the
  free list twice within a batch. What leaves a refused release harmless is `apply_batch`, not the walk
  preceding the release: it returns the error before it stores the working set, and the
  superseded ids go with it. The one release that reaches past the working set is a
  created link's, whose id `discard_created` returns to the free list at once — a page
  never written, which no root reaches. A link two chains share, released in **different**
  batches, is not caught — the store trusts that each page has one referrer. A chain
  is shared between the leaf versions that reference it
  and between a store and its fork, both by id; it is never mutated.
- **Byte-aware split**: because keys vary in length, *whether* a node splits is
  **byte-budgeted** (`leaf_entries_fit` / `internal_entries_fit` measure the
  serialized page against `DATA_SIZE`), with a hard `MAX_ENTRIES_PER_PAGE` count cap
  as a secondary bound, and so is *where*: `split_points` takes the most
  byte-balanced two-way cut when one fits and otherwise fills pieces greedily left to
  right, so a node splits into as many pieces as it takes for each to fit — a two-way
  cut cannot always, once an entry is over half a page. An internal node promotes the
  entry at each cut, so a piece may hold only its `first_child`, and the root grows as
  many levels as its separators need in one batch. What makes every piece fit is
  `apply_batch`'s up-front refusal of an entry that does not fit a page alone and of a
  key over `MAX_KEY_LEN` (4 044 bytes — the most an internal page can carry as a
  separator beside its `first_child`). **Neither page store accepts a page that does
  not fit**: `CowPageStore::put` checks the size it never serializes
  (`Page::check_fits`), `Page::serialize` bounds every write and returns
  `PageStoreError::Overfull`, and `apply_batch` hands a commit's pages to
  `PageStore::put_batch`, which checks every page before it stores any — once per page,
  the in-memory store's override skipping `put`'s repeat of the check — so a refusal
  leaves no page written and the batch's fresh ids go back to the free list. The per-entry byte size uses `postcard::serialized_size` (counts bytes,
  no allocation) — this runs per entry on every insert's fit check, so an allocating
  measure dominated the batched-write apply loop.
- **`Arc`-wrapped entry vectors**: `Page::Internal`/`Leaf` hold their entries as
  `Arc<Vec<…>>`, so cloning a page (every descent step clones the page it
  navigates through) is an O(1) refcount bump, not a deep copy of all values.
  CoW pays the deep copy only on the leaf/internal node it actually mutates, via
  `Arc::make_mut` (which copies iff an old root or the cache still shares the
  vector — keeping MVCC snapshots immutable). Within one `apply_batch`, a page the
  batch already copied is mutated **in place**: the batch working set hands a
  mutator the sole reference (`take_for_write` removes it from the working map, so
  its `Arc` refcount is 1 and `make_mut` doesn't re-copy), then re-inserts it —
  so a page is deep-copied at most once per batch, not once per insert that lands
  in it. The page codec is hand-rolled, so the `Arc` is invisible on disk — no
  format change, no magic bump.
- **On-disk format (v30)**: the meta magic is `PGMV`. A bump takes the next magic and a magic
  is never reused (`META_MAGIC`'s doc states the rule; the next is `PGMW`); the ledger of the
  bumps since `PGM7` is `docs/storage.md` § Store versioning. Reopen rejects every superseded
  magic loud (`double-meta: stale on-disk format …`) — re-bootstrap dev DBs.

  **`PGMD` is the odd one out — the postcard shape did not change.** An old store's jsonb
  rows decode perfectly; they are simply in a key order that equality, `jsonb_compare`, and
  `jsonb_out` no longer expect, so `'{"a":1,"b":2}'` written before the change would compare
  `<>` the identical value written after it. The bump converts that silent wrong answer into
  a loud reopen failure. When the *meaning* of persisted bytes changes, that is a format
  break even though every byte still parses.

  **`PGMF` is the case that would have been tempting to skip.** The `Option<String>` a
  `CheckConstraint` used to hold could stay, with `None` meaning "written before the namer,
  reconstruct it". But the reconstruction can only see the table's
  own preceding checks, while the namer that replaced it searches the whole schema across all
  four constraint kinds, so the two disagree on a reachable input and one database yields two
  different dumps depending on which build wrote it. A compatibility branch that answers
  differently from the code it stands in for is not compatibility. Deleting it is a format break
  even though nothing about the old bytes is unreadable.

  **`PGMG` is the case where a *widening* was unavoidable.** A sequence used to be its
  counter — the fact and the storage were the same `i64` — and two behaviours could not be
  built on that: a bare `ALTER SEQUENCE … RESTART` returns the counter to the sequence's
  `START WITH`, and `OWNED BY` names the column a sequence dies with. Neither is derivable
  from the counter, so the record grew and the bytes moved. The trap it left behind is not
  in the format at all: every site that *updated* a counter did so by inserting a fresh
  value, and a naive port of that insert writes a whole fresh **record** — silently
  resetting `start` and dropping `owner`, invisibly to any query that reads `nextval`.

  **`PGME` is the ordinary case, and it is the one to reason from.** Postcard is
  positional and non-self-describing, so a **new field** on a persisted struct is never
  backward-compatible — `#[serde(default)]` does nothing for it (that attribute only
  helps self-describing formats). An old `Index` whose `Uniqueness` carries two bytes
  where three are now read consumes the following field's byte and desynchronizes the
  rest of the catalog. Adding a field to anything reachable from `Schema` is a magic
  bump, every time.
- **The planner `Expr` enum's variant *order* is on-disk format.** Since `PGM4` the catalog
  stores CHECK predicates, index key expressions, and index partial predicates as
  postcard-encoded `Expr`, and a serde-derived enum encodes its variant as a **positional**
  index — so inserting a variant anywhere but the end shifts every later tag and an existing
  store silently decodes persisted expressions as the wrong variant. New variants go **last**,
  and `expr_postcard_variant_tags_are_pinned` (`planner/expr.rs`) fails the build if one
  doesn't. `TypeOid` is exempt by design — it encodes as its numeric OID (`PGM6`), so it can
  be reordered freely; `Expr` has no such external numbering. A variant that must move needs a
  magic bump, which turns the silent misdecode into a loud stale-format error.

## CoW Implementation (cow.rs)

```
CowBtree
├── store: Box<dyn PageStore>    # Page storage (trait object for flexibility)
├── root: AtomicU64              # Current root (atomic for readers)
├── version: AtomicU64           # Version counter
├── generation: AtomicU64        # Seqlock guarding the (root, version) pair
├── write_lock: Mutex            # Single-writer serialization
├── active_snapshots: parking_lot::RwLock<HashMap<Version, AtomicUsize>>
│                                # Version -> ref count; read-lock + atomic on the
│                                #   hot path (write lock only to add/remove a version)
├── pending_gc: Mutex            # Old pages awaiting GC
├── pending_gc_pages: AtomicUsize # lock-free shadow of pending-page total (GC gate)
└── vacuum_threshold: usize      # Auto-vacuum trigger threshold
```

**Snapshot refcount (read-path concurrency).** `active_snapshots` tracks how many
readers hold each old version so vacuum never frees a page a reader can still
reach (`min_active_version` = the smallest version with a `> 0` count, always `<=`
every active snapshot's version). The counts are **atomics under a
`parking_lot::RwLock`**: `snapshot`/`release` for an already-tracked version take
only the *shared* read lock + a `fetch_add`/`fetch_sub`, so concurrent readers do
not serialize; the exclusive write lock is taken only to insert a never-seen
version or remove a drained one (re-checked under the lock so a concurrently
revived version is never dropped). `snapshot` registers its refcount and then
re-validates the version is still current before returning — `apply_batch`
publishes the next version *before* pushing the retiring pages to `pending_gc`, so
an unchanged version means its pages cannot have been freed yet, closing the
acquire-vs-vacuum window. `release` gates GC on the lock-free `pending_gc_pages`
shadow rather than locking `pending_gc` every time.

### Constructors
```rust
// In-memory (default)
let tree = CowBtree::new();

// Custom page store (e.g., OPFS for browser persistence)
let tree = CowBtree::with_store(Box::new(opfs_store), root, version, threshold);
```

### Write Path
1. Acquire write_lock
2. Find path from root to target leaf
3. Clone leaf, apply modification
4. Clone parents up to root (updating child pointers)
5. Publish the new `(root, version)` pair via the seqlock (`publish_root` — the
   sole mutation point, so no publish site can drift)
6. Register old pages for GC
7. Release lock

### Read Path (lock-free)
1. Capture `(root, version)` consistently via the seqlock (`snapshot()` retries
   while a publish is in progress), register the snapshot's refcount, then
   re-validate the version is still current (retry if a commit advanced it) —
   shared read lock + atomic, no per-statement write lock
2. Traverse using the captured root
3. Pages are immutable - no locks needed

**Note:** Range scans traverse the tree structure rather than following leaf `next`
pointers. Leaf chains are not maintained in CoW mode because updating a leaf would
require CoW-ing all preceding leaves to update their `next` pointers (O(n) cost).

### Snapshots
- `snapshot()` returns (version, root) and increments ref count
- `release_snapshot()` decrements ref count, triggers auto-vacuum if threshold exceeded
- Pages freed only when no snapshot references them and the store's durable version
  does not either (`PageStore::durable_version`)

## Vacuum

### How It Works

Pages become garbage when:
1. They are replaced by CoW (new version written)
2. No active snapshot references their version
3. The durable meta slot does not reference it either — the version a crash reopens

```rust
// Explicit vacuum
let result = tree.vacuum();
// result.pages_freed, result.pages_pending, result.min_active_version

// Auto-vacuum on snapshot release (when pending > threshold)
tree.release_snapshot(&snapshot);  // Triggers GC if needed

// Combined release + force vacuum
let result = tree.release_snapshot_and_vacuum(&snapshot);
```

### Statistics

```rust
let stats = tree.stats();
stats.pending_gc_batches  // Number of GC batches pending
stats.pending_gc_pages    // Total pages pending GC
stats.free_list_size      // Pages available for reuse
stats.active_snapshots    // Count of active snapshots
```

### Configuration

```rust
// Default: 100 pending pages triggers auto-vacuum
let tree = CowBtree::new();

// Custom threshold
let tree = CowBtree::with_vacuum_threshold(50);
```

### Page Lifecycle

```
Allocated → Active → Pending GC → Free List → Reused
    │          │          │            │
    │          │          │            └─► New allocations try free list first
    │          │          │
    │          │          └─► Vacuum moves here when no snapshot refs
    │          │
    │          └─► CoW creates new page, old goes to pending
    │
    └─► First allocation from counter
```

## Storage Wrapper (`wrapper/`)

```
BtreeStorageWrapper
├── storage: Arc<CowBtree>             # Underlying B+tree (Arc for MVCC)
├── name_to_id: HashMap<String, u32>   # Table name → ID mapping
├── next_table_id: AtomicU64           # Table ID allocator
├── row_id_counters: HashMap<u32, AtomicU64>  # Per-table row IDs
├── transactions: HashMap<i32, BtreeTransactionState>  # Active txns; read *across*
│                                                     #   connections by the unique-key
│                                                     #   claim probe, not just by owner
├── cross_connection_hits: AtomicU64   # Times a probe found another txn's staged state
│                                      #   (the claim walk, the table-id restore); 0 under
│                                      #   the host's writer
├── index_defs / table_indexes         # Registered index defs + per-table index sets
├── last_schema: Mutex<Arc<Vec<u8>>>   # Last-persisted catalog bytes (SYS_CATALOG rewrite dedup)
└── only_connection: Option<i32>       # The one connection an in-memory store built for one
                                       #   serves (for_one_connection); pglet's facade then
                                       #   hands it no catalog
```

### Transaction Support
- `begin_transaction(conn_id)` - Captures snapshot, creates pending buffer
- `rebase_transaction(conn_id)` - Re-takes the snapshot and the name-map / table-id
  snapshots (the block's and each savepoint's) at the block's first write, when the host
  hands it the writer; nothing is staged yet
- `advance_transaction_snapshot(conn_id)` - Moves the snapshot onto the root the
  transaction just published into — an index build's or rebuild's entries, which its
  probes then read; called by the host's adapter, only the read root moves
- `commit_transaction(conn_id)` - Applies pending writes atomically
- `rollback_transaction(conn_id)` - Discards pending and restores the name map and the
  table-id allocator unconditionally: the host's writer keeps every other connection's
  write out between the re-base and the rollback
- `end_unwritten_transaction(conn_id)` - Ends a transaction that never took the writer,
  restoring nothing: its snapshots are older than other connections' commits since
- `unique_key_claimed_elsewhere(conn_id, index_id, values)` - Whether **another**
  connection's open block stages a write on this index key — the write path's second
  occupancy question, beside its own index probe. The one storage operation that reads
  another connection's `transactions` entry (every other access is keyed by the caller's
  own `conn_id`): it walks every entry but `conn_id` and asks each block's *claimed*
  writes for a match under the index's own key encoding. A block whose claims are
  suspended answers from its last savepoint frame instead, and one with no frame
  answers nothing. Under the host's writer no other block has a staged write when a
  connection writes, so a hit is unexpected and counted (`cross_connection_hits`)
- `suspend_claims(conn_id)` - Mark that connection's claims suspended, because its block
  just went aborted — the one release a claim does not get from the writes themselves,
  since an aborted block ends nothing until it exits; `rollback_to_savepoint` resumes
  them
- `*_in_tx(conn_id, ...)` - Read-your-own-writes within transaction

  `index_lookup_in_tx` (the RYOW equality/uniqueness probe — an enforced PK/UNIQUE
  insert calls it per row) probes the **committed index at the snapshot root**
  (O(log n)) and merges the pending buffer, rather than scanning the table (which would make
  a batch of enforced inserts O(rows × table_size)). The
  pending merge itself is O(1) in the pending row count: the transaction maintains
  a **pending index** (`pending_index`: `index_id` → encoded-key → row ids, B-tree
  indexes only), so the probe looks the key up instead of scanning. `insert_in_tx`,
  `update_in_tx` and `delete_in_tx` maintain it through one `restage_pending_index`
  call that retracts the row's *staged* image's keys before adding its new ones, which
  is what keeps a single `UPDATE` from putting the whole table on the scan path.
  Column DDL (every row rewritten, and a dropped column shifts the positions a key
  encoding reads) or a key-encoding/predicate failure mark the table in
  `probe_scan_tables`; index DDL in the txn is a separate condition that falls **every**
  table in the block back. Either way the probe runs a full pending scan (correct,
  O(pending)).
  `gin_lookup_in_tx` still full-scans (its pending merge over posting-list intersection
  is a follow-up).

### MVCC Snapshot API
- `read_snapshot()` - Returns `BtreeSnapshot` for lock-free reads
- `btree()` - Get `Arc<CowBtree>` for shared access

### Vacuum API
- `vacuum()` - Explicit garbage collection, returns `VacuumResult`
- `stats()` - Get `CowBtreeStats` including vacuum info
- `vacuum_threshold()` - Get current auto-vacuum threshold

## BtreeSnapshot (Lock-Free Reads)

```
BtreeSnapshot
├── btree: Arc<CowBtree>               # Shared reference to B+tree
├── snapshot: Snapshot                 # (version, root) at capture time
├── name_to_id: HashMap<String, u32>   # Table mappings at capture time
└── index_nnd: HashMap<u32, bool>      # Per-index NULLS NOT DISTINCT flag at capture time
```

### Usage
```rust
// Get snapshot (brief lock, immediately released)
let snapshot = wrapper.read_snapshot();

// Read without any locks held
let rows = snapshot.scan("users")?;
let row = snapshot.get("users", row_id)?;

// Snapshot auto-released on drop
```

### Methods
- `scan(table)` - Scan all rows using `scan_table_at_root()`
- `get(table, row_id)` - Get single row using `get_at_root()`
- `table_exists(table)` - Check if table exists in snapshot
- `version()` - Get snapshot version number

### Pending Writes
```
BtreePendingWrites
├── changes: HashMap<table_id, BTreeMap<row_id, PendingRow>>
├── dropped_tables: HashSet<u32>
└── dropped_indexes: HashSet<u32>
```

## Tests

Unit tests (in the crate) cover key encoding/ordering (data + index kinds,
cross-kind ordering, index range bounds), page serialization (incl.
index-key round-trip, the **large-index-key page-fill** guard that the byte-aware
fit check refuses a page its entries would overflow, `split_points`' every-piece-fits
property, and the fallible codec — `encoded_len` equal to what `serialize` writes, a page
one byte over refused, malformed bytes refused on decode), the **page-budget audit**
(random row-and-index trees up to the key bound, every reachable page checked to fit and
every entry read back; a three-way leaf cut; a multi-level root growth; the in-memory
store refusing an overfull page), **overflow chains** (a long value out of line and
read back; a replaced or deleted chain reclaimed once no snapshot reads it, with a
snapshot reading the old value across an allocation that would reuse an early-freed
page; a chain replaced inside its own batch never written; a fork's chains; eleven
damaged-chain shapes refused as `CorruptChain` naming the page, on a read, a scan and a
release, each within the descent's page reads plus the chain's own links, through a
read-counting store double; a chain two entries name refused rather than released
twice; a store's overfull refusal publishing nothing and freeing the batch's pages; the
overflow page and reference codec), PageStore metadata, core B+tree ops,
CoW page creation & snapshots, the Storage trait, transaction isolation
(concurrent txns, rollback, commit), vacuum/GC, `DoubleMetaStore` (slot
alternation, reopen recovery, torn-slot fallback, **stale-format** fail-loud over
every superseded magic, and the **free-list page chain** — a multi-page chain
round-trips on reopen, chain pages reuse the rewritten slot's prior generation so
steady-state churn doesn't grow the file, an empty free-list persists a null head;
a commit's pages refused whole by `put_batch`; a checksum-valid page that does not
decode refused as `Corruption`; a CRC-valid slot whose page ids leave the store —
`next_page_id` past the last addressable page, a `root` or `freelist_head` never
allocated — refused with a fallback to the other slot, and open refused when both carry
it; a slot at the limit opening and refusing the write past it; under such a slot, a
chain link that would wrap onto page 1 refused as a corrupt chain and an allocation that
would wrap to the null link not handed out; a read or write of an id past the store refused rather than wrapped; a free-list
page naming an unallocated id not recovered; and an honest store whose `next_page_id`
has passed its file's end reopening, the reason the slot is not checked against the
file; a slot landing over a stale chain page, head or tail, recovering no id it lists;
creation cut short opening as a new store, and a store that held a page refused when
both slots are lost; a recovered list handing each id out once; collection freeing no
page the durable version reaches; a superseded, uncollected page free after a reopen)
+ the tree walks (a page naming an ancestor refused by a point read, a scan and a
write's descent; a child two pages name refused by a scan; a 60 000-deep chain of pages
walked without overflowing the stack)
+ its sharded write-through page cache (`page_cache.rs`: shard-routing /
sequential-id dispersion / exact capacity carving / tiny-cache-collapse unit
tests, plus a `double_meta` concurrent get-vs-put/free consistency hammer that
asserts a read never returns the wrong page under shard contention), and
BtreeSnapshot MVCC reads. `pglet`'s `tests/double_meta.rs` drives the full engine
over a shared in-memory `BlockIo` (explicit-commit + auto-commit reopen durability,
a **multi-chunk catalog** reopen, torn-slot fallback, the 2-fsync-per-commit
oracle), `tests/page_fit.rs` and `tests/large_value.rs` drive the split and the
overflow chains through SQL on the file store, the store over an in-memory image and
the in-memory tree, `tests/durability.rs` rebuilds every power-loss image of recorded DDL and DML
workloads (`pglet_testing::power_loss`) and checks each adopts an acknowledged state
with a free list disjoint from its tree — and, where every pending write or none
landed, no page leaked — checks no statement writes
a page the durable slot reaches (after a failed slot write too), and `SIGKILL`s a real
embedding process inside a `CREATE TABLE`; and `tests/crash.rs` adds
mid-commit fault injection (including a failure at each page write of a chain's
commit) + a randomized corruption fuzz over an image that holds chains, then drives
chain damage **behind valid checksums** (`pglet_testing::store_image`, which edits an
image and re-stamps each page's checksum with the store's own rule): each
`ChainDamage` shape refused at the doors it drives, and a second randomized sweep over
the leaf and overflow pages that asks for an error rather than a panic and counts its
chain refusals. The engine-level shared
in-memory / fault-injecting `BlockIo` double (`MemBlockIo`) lives in the dev-only
`pglet_testing` crate (`pglet_testing::block_io`); `pglet_storage` keeps its own
in-crate `mem_block_io.rs` for its `#[cfg(test)]` unit tests (a distinct trait
instance it can't borrow from `pglet_testing`). Real-process **kill -9 + soak**
durability testing is the `pglet_tests` `test_crash` binary (`deno task
test:crash`), which spawns `pglet_server` over a real file and pulls the rug out
with `SIGKILL`. A coverage-guided `cargo-fuzz`/libFuzzer target (`fuzz/`, `deno task
fuzz`) mutates a valid double-meta image against the **same** recover-or-loud
invariant — shared with `crash.rs` via `pglet_testing::durability` — but
coverage-guided and stricter (a panic on an untrusted image is a finding, not an
acceptable "loud"), then opens and scans the same image with its leaf and overflow
pages' checksums re-stamped, so a mutation there reaches the chain walk
(`fuzz/README.md`). Browser (Playwright) tests cover OPFS persistence and commit
durability across a reload (the meta flip is the durable commit point — no WAL).

## Server Integration

**In-memory (default; `--storage btree` names it):**
```bash
cargo run -p pglet_server
```

**File-backed persistence:**
```bash
cargo run -p pglet_server -- --data-dir ./data
# Creates: data/data.pglet — a single file (two meta slots + CoW data pages)
```

The server supports B+tree storage via:
- `StorageBackend::Btree` variant in `crates/pglet/src/backend.rs`
- `SharedState::with_btree()` constructor (in-memory)
- `Server::with_file_btree()` constructor (file-backed, wraps `Database::open` + the
  data-dir process lock, which the server holds across the reopens that follow a
  failed store, `pglet_server`'s `server.rs`)

## Single-file durable layout (`double_meta.rs`)

Persistence is a single file holding two fixed metadata slots followed by the
CoW data pages. A commit:
1. writes the new CoW pages (`put`) — including the `0x02` system entries
   (`SYS_TABLE` always, `SYS_CATALOG` when the catalog changed),
2. fsyncs them (`flush` — fsync #1),
3. logs the superseded pages (spilling retired-log pages when the slot's share is
   full), persists the free-list page chain, then writes the **fixed-size** metadata
   record (root, `version`, `next_page_id`, `next_table_id`, `freelist_head`, `freelist_crc`, the
   four `retired_*` fields) and the retired log's newest entries into the *alternate*
   meta slot and fsyncs again (`save_state` — fsync #2, the durable commit point); the
   chain pages ride this fsync,
4. flips the active-slot pointer, and only then moves the store's
   `durable_version` to the version saved.

Copy-on-write alone does not keep the durable slot's pages intact: a statement can
publish several versions before its `save_state` (a DDL statement's eager wrapper
batch, the commit batch, the catalog batch), and a page a later version superseded is
still one the durable slot reaches. So `CowBtree::try_gc` frees no page whose
`freed_version` is at or past `PageStore::durable_version()` — pinned until a newer
slot's fsync returns, a failed `save_state` included. The free-list chain is
authenticated from the slot: `freelist_crc` is the head page's checksum, and each chain
body carries its successor's (`[next_page][next_crc][count][ids]`), so a chain page a
power cut did not land ends the open-time walk instead of handing out another commit's
ids. Superseded pages still in `pending_gc` are handed to the save by the tree, their one
owner (`CowBtree::save_state` → `PageStore::save_state`'s `superseded`), and logged apart
from the free list: the slot carries the log's newest entries (at most 240) after its
metadata, and past that a save spills them into retired-log chain pages
(`[next_page][next_crc][next_max][count][(id, freed_version)…]`), written once and
released once both slots' floors pass them. A reopen takes every logged entry at or past
the slot's floor as free, so a close or a kill does not leak them, and a held snapshot
does not make each commit rewrite them. A page write or data `fsync` the device refuses
is `PageStoreError::WriteRefused`, and the tree decides: a batch its caller can drop whole
(`CowBtree::apply_discardable_batch`) is dropped and the store serves on, whatever another
connection has published unsaved; any other refusal, and every refused save, fails the
store (`WriteFailure`, `PageStoreError::WriteFailed`, or `SaveUnconfirmed` for a refused
slot write or the `fsync` after it, whose slot may have landed): every later tree-page read, write and save is
refused (`PageStoreError::StoreFailed`) until a reopen, since the tree may already hold a
commit the durable slot does not (`docs/storage.md` § Crash Recovery). A crash mid-commit leaves the active
slot untouched; the half-written alternate is rejected by its CRC on reopen, and a file
that is only creation's zeros (no longer than the two slots) opens as a new store. No WAL. Every data page is likewise
CRC32-checksummed — stamped on `put` (over the page buffer, into the header's
`checksum` field) and verified on `get` — so a torn or bit-rotted page fails loud
(`PageStoreError::Corruption`, distinct from a missing page) and refuses to decode
rather than returning garbage.

`PgletBtreePersistent` (pglet_wasm, `opfs` feature) uses the **same** `DoubleMetaStore` over an
OPFS `BlockIo` (`OpfsBlockIo`), so native and browser share one page-store path.

### BlockIo (`block_io.rs`)

`DoubleMetaStore` works over the `BlockIo` seam (a `Box<dyn BlockIo>`) — positioned
`read_at`/`write_at`, `sync` (fsync), and `len`/`set_len`:
- `FileBlockIo` — native `std::fs::File` (`pread`/`pwrite` + `sync_all`); gated
  behind the `file_store` cargo feature so it never enters the wasm32 build.
- `OpfsBlockIo` — browser OPFS `FileSystemSyncAccessHandle` (in `pglet_wasm`, `opfs` feature).
- `CountingBlockIo` — a pass-through decorator that counts fsyncs/reads/writes
  (the 2-fsync commit oracle).

### PageStore Trait (`store.rs`)
```rust
pub trait PageStore: Send + Sync {
    fn allocate(&self) -> PageId;
    fn get(&self, page_id: PageId) -> PageStoreResult<Option<Page>>;
    fn put(&self, page: Page) -> PageStoreResult<()>;
    fn put_batch(&self, pages: Vec<Page>) -> PageStoreResult<()> { /* check all, then put each */ }
    fn free_pages(&self, page_ids: Vec<PageId>);
    fn write_failure(&self) -> Option<&WriteFailure> { None }
    fn durable_version(&self) -> Option<u64> { None }
    fn page_count(&self) -> usize;
    fn free_list_size(&self) -> usize;
    fn next_page_id(&self) -> PageId;
    fn free_list(&self) -> Vec<PageId>;
    fn flush(&self) -> PageStoreResult<()> { Ok(()) }
    fn save_state(&self, _root: PageId, _version: u64, _next_table_id: u32, _superseded: &[(Vec<PageId>, u64)]) -> PageStoreResult<()> { Ok(()) }
    fn fork_in_memory(&self) -> Option<Box<dyn PageStore>> { None }
}
```

### Implementations
- `CowPageStore` — in-memory (default, tests)
- `DoubleMetaStore` — single-file double-meta over a `BlockIo` (native + OPFS)

### Write-through page cache (sharded)

`DoubleMetaStore` reads go through a write-through page cache, **sharded** into N
independent `Mutex<LruCache>` lanes (`page_cache.rs`, `ShardedPageCache`; default
~2000 pages ≈ 8 MiB total) so concurrent readers don't serialize on one global
mutex — an `LruCache` needs `&mut` per `get`, so it can't be a plain `RwLock`.
`get` serves cache hits without touching the `BlockIo`; `put` writes to disk
*and* updates the cache (so durability and the 2-fsyncs-per-commit count are
unchanged); `free_pages` evicts freed ids before they can be reused by CoW. The
lane is chosen by a Fibonacci hash of the page id (high bits, so sequential /
LIFO-reused ids disperse instead of clustering), and `get`/`put`/`pop` all route
through **one** shard-index function so an eviction always lands in the lane a
later `get` reads — the page-id-reuse invariant can't be subtly broken by
mismatched routing. The total capacity is carved across the lanes (min one page
each; a tiny cache collapses toward a single lane).

## Native persistence & features

- Single-file double-meta store; `--data-dir` CLI flag
- Schema (chunked), table directory, and `row_id_counters` persisted **in the tree**
  as `0x02` system entries; the meta slot holds fixed-size state (root, `version`,
  `next_page_id`, `next_table_id`, `freelist_head`, `freelist_crc`, the four
  `retired_*` fields) and at most 240 retired-log entries
- Transaction routing (read-your-own-writes) via the wrapper's `*_in_tx` methods,
  dispatched per-connection by the facade's `StorageAdapter` (`pglet::backend`)
- Write-through LRU page cache for hot pages (~8MB default)
- MVCC lock-free reads via `BtreeSnapshot` (Arc-wrapped `CowBtree`)

**Future optimizations:**
- [ ] Bulk loading optimization
- [ ] Reusing an unchanged row's chain across an `UPDATE` (today every write of a
      long row writes a fresh chain), and a cache-bypassing read for chain pages
