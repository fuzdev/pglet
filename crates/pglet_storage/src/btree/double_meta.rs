//! Single-file "double-meta" page store (no WAL).
//!
//! An LMDB-style durable layout that needs no write-ahead log: the file holds
//! two fixed meta slots followed by the CoW data pages. A commit writes the new
//! data pages, fsyncs them, then writes the full metadata snapshot into the
//! *alternate* meta slot and fsyncs again — the second fsync is the atomic
//! commit point. A crash mid-commit leaves the active slot untouched and the
//! half-written alternate is rejected by its CRC on reopen.
//!
//! # What keeps the durable slot's pages intact
//!
//! Copy-on-write alone does not: a statement can publish several versions before
//! its one `save_state`, and a page a later version superseded is still one the
//! durable slot's tree reaches. Two rules keep every page a reopen could adopt as
//! that slot wrote it until a newer slot is durable:
//!
//! - **the tree pages** — `CowBtree`'s garbage collection frees no page the
//!   [`durable_version`](PageStore::durable_version) reaches, and that version moves
//!   only once a slot's fsync has returned, so what collection puts on the free list
//!   is never one of them;
//! - **the free-list chain** — a commit writes its chain into pages only the slot it
//!   is replacing named, or into fresh ids past `next_page_id`, and the slot records the head page's checksum, each page its
//!   successor's. A chain page that did not land holds another commit's bytes and
//!   fails that check, so the walk stops there: a power loss between a chain write and
//!   the slot's fsync costs the unread tail of the free list (a leak), not a live
//!   page handed out — short of a stale page whose CRC32 happens to match.
//!
//! Pages superseded but not yet free (a snapshot still reads them, or the durable
//! version does) are logged by every save that describes a version past them
//! (`RetiredLog`): no root of the version saved reaches them, so after a reopen they
//! are free, and a close or a kill does not leak them. The log is kept apart from the
//! free list, which every save rewrites whole: a save logs what it retired, and a page
//! of the log is written once, so a long-held snapshot does not make every commit
//! rewrite every page superseded since.
//!
//! # A write the device refuses
//!
//! A refused page write or data `fsync` is answered [`PageStoreError::WriteRefused`] and
//! does not fail the store: the tree that issued it decides, from what its caller says
//! the batch belongs to (`CowBtree::apply_discardable_batch`). A refused save — a chain page, the slot, or
//! the `fsync` that is the commit point — fails the store, since the tree has published
//! the version being saved; one refused at the slot write or that `fsync` is answered
//! [`PageStoreError::SaveUnconfirmed`], since the slot may still have landed. A
//! failed store ([`WriteFailure`], recorded here or by the tree) refuses every page
//! read, page write and save after it — a page read in flight when it was recorded
//! included, checked again once its bytes are read — and only a reopen, which reads the
//! durable slot alone, clears it.
//!
//! # File layout
//!
//! ```text
//! ┌──────────────┬──────────────┬─────────────────────────────┐
//! │ meta slot A  │ meta slot B  │ data pages                  │
//! │ 64 KiB       │ 64 KiB       │ page_id * PAGE_SIZE + 128KiB │
//! │ offset 0     │ offset 64KiB │ offset 128 KiB ...          │
//! └──────────────┴──────────────┴─────────────────────────────┘
//! ```
//!
//! Each meta slot is `MAGIC(4) | version(8) | payload_len(4) | crc32(4) |
//! payload[payload_len]`, where the payload is the `PageStoreMetadata`
//! serialization (so there is exactly one metadata encoding) followed by the slot's
//! retired-log entries, `[count: u32][(PageId, freed_version); count]`
//! (`encode_retired_entries`), and the CRC32 covers `version | payload_len | payload`.
//!
//! # Commit = exactly two fsyncs
//!
//! - `flush()` → `io.sync()` (durable data pages)
//! - `save_state(meta)` → write the retired-log and free-list chain pages, then the
//!   alternate slot → `io.sync()` (the commit point) → flip the active-slot pointer
//!
//! # Write-through page cache
//!
//! Reads go through a write-through page cache (a [`ShardedPageCache`] of
//! `lru::LruCache<PageId, Page>` lanes, default ~2000 pages total). `get` serves
//! cache hits without touching the `BlockIo`; on a miss it reads + caches. `put`
//! writes to disk *and* updates the cache — write-THROUGH, so the durability
//! model and the two-fsyncs-per-commit count are unchanged (the cache only saves
//! reads, never absorbs or defers a write or an fsync). `free_pages` evicts the
//! freed ids: CoW reuses freed page ids for later allocations, so a stale cached
//! page under a reused id would be silent corruption. CoW never mutates a live
//! page in place (new versions get new ids), so caching immutable pages by id is
//! otherwise always safe.
//!
//! The cache is **sharded** so concurrent readers don't serialize on one global
//! mutex; see `page_cache.rs` for the sharding, hashing, and routing-correctness
//! detail.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use parking_lot::Mutex;

use crc32fast::Hasher;

use super::block_io::BlockIo;
use super::page::{NULL_PAGE_ID, PAGE_SIZE, Page, PageId};
use super::page_cache::ShardedPageCache;
use super::store::{
    DeviceError, PageStore, PageStoreError, PageStoreMetadata, PageStoreResult, WriteFailure,
};
use super::system::{
    FREELIST_IDS_PER_PAGE, RETIRED_PER_PAGE, RetiredEntry, decode_freelist_body,
    decode_retired_body, decode_retired_entries, encode_freelist_body, encode_retired_body,
    encode_retired_entries,
};

/// Compute a CRC32 checksum over `data` (used to detect torn meta slots).
#[inline]
fn crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// CRC32 of a serialized page over every byte except its own 4-byte checksum
/// field (page header bytes 28..32). Stamped on write (`put`) and verified on
/// read (`get`) so a torn or bit-rotted data page is caught, not trusted. Meta
/// slots carry their own CRC (see `write_slot`); this covers the data region.
fn page_checksum(buf: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(&buf[..28]);
    hasher.update(&buf[32..]);
    hasher.finalize()
}

/// Default page-cache size: ~8 MiB = 2000 pages at 4 KiB each (the total across
/// all cache shards).
const DEFAULT_CACHE_SIZE: usize = 2000;

/// Size of each meta slot in bytes. Under 4 KiB is used — the fixed
/// `PageStoreMetadata` record and at most [`RETIRED_INLINE_MAX`] retired-log entries;
/// the variable-size catalog/free-list moved into the tree at PGM5. Kept at the pre-PGM5 64 KiB so `DATA_REGION_START`
/// doesn't move (shrinking it is a separable format bump).
const META_SLOT_SIZE: u64 = 64 * 1024;

/// Number of meta slots (double-buffered: A and B).
const META_SLOT_COUNT: u64 = 2;

/// Offset where data pages begin (after both meta slots).
const DATA_REGION_START: u64 = META_SLOT_SIZE * META_SLOT_COUNT;

/// One past the highest page id the store can address: the page with id
/// `PAGE_ID_LIMIT - 1` is the last one whose bytes end at or below `i64::MAX`, the
/// largest offset a signed 64-bit file position (POSIX `off_t`) can name. Every id
/// the store reads or writes is below it — [`DoubleMetaStore::page_offset`] refuses
/// any other, so its multiply cannot overflow — and a meta slot whose
/// `next_page_id` is above it is refused at open (`slot_ids_are_addressable`). An
/// allocation can still reach the limit itself; the write of it is refused, and so is
/// every later `save_state`, whose `next_page_id` has then passed the limit.
const PAGE_ID_LIMIT: PageId = (i64::MAX as u64 - DATA_REGION_START) / PAGE_SIZE as u64;

/// Magic prefixing a meta slot: "PGMV" (PGlet Meta, on-disk format v30).
///
/// The magic is a fixed 4-byte tag. Versions ran `PGMT` (v1) then `PGM2`…`PGM9`
/// (v2…v9, the version digit in the 4th byte); v10 needs two digits, which no longer
/// fit, so the scheme rolls the 4th byte past `'9'` into `'A'` — `PGMA` = v10, `PGMB`
/// = v11, `PGMC` = v12, `PGMD` = v13, `PGME` = v14, `PGMF` = v15, `PGMG` = v16, `PGMH`
/// = v17, `PGMI` = v18, `PGMJ` = v19, and so on.
///
/// **A magic is never reused.** A bump takes the next 4th byte in the order `2`–`9`,
/// `A`–`Z`, `a`–`z`, skipping any byte an earlier version's magic holds — `T`, v1's — so
/// v29 is `PGMU`, not `PGMT`, which a store would refuse as v1, and v30 is `PGMV`. Past `z`
/// the 3rd byte
/// moves on (`N`, then `O`, …) and the 4th runs `0`–`9`, `A`–`Z`, `a`–`z` again. A reused
/// magic makes a store refuse itself as stale on its first reopen.
///
/// The on-disk format has changed several times. v2 (`PGM2`) moved the page codec
/// to variable-length, length-prefixed keys (data rows + secondary-index entries
/// share one tree). v3 (`PGM3`) added a per-segment NULL-presence tag to composite
/// index keys (`0x00` NULL / `0x01 ++ value`), so a NULL key stays distinct from an
/// empty-but-present value. v4 (`PGM4`) changed the persisted **catalog** (`Schema`)
/// format: CHECK and index predicates store the parsed planner `Expr` IR (postcard)
/// rather than source SQL. v5 (`PGM5`) lifted the ~64 KiB meta-slot ceiling: the
/// variable-size catalog, table mappings, and row-id counters moved **out** of the
/// slot into the B+tree as `0x02` system entries, so the slot holds only fixed
/// state — and the slot-payload bytes changed shape. v6 (`PGM6`) re-encoded a
/// persisted `Column.type_oid`: `TypeOid` now serializes as its **u32 OID** rather
/// than its serde variant ordinal, so the catalog (`SYS_CATALOG`) bytes changed
/// shape. v7 (`PGM7`) changed the `Value::Interval` payload: an interval is now a
/// 3-field `(months, days, micros)` span rather than a flat `i64` µs count, so a
/// persisted `INTERVAL '…'` literal inside a CHECK / partial-index `Expr` (postcard)
/// changes shape. v8 (`PGM8`) added the `Column.varchar_length` catalog field (a
/// `varchar(n)`/`char(n)` length cap) and a trailing `Expr::CastTypmod` variant, so a
/// persisted `SYS_CATALOG` — or a typmod'd cast inside a CHECK / partial-index `Expr`
/// — changes shape. v9 (`PGM9`) added the `Column.identity` catalog field
/// (`GENERATED … AS IDENTITY` generation), so a persisted `SYS_CATALOG` changes shape
/// again. v10 (`PGMA`) changed the `ForeignKey` catalog shape — the single `column` /
/// `parent_column` strings became `columns` / `parent_columns` `Vec`s for composite
/// (multi-column) foreign keys — so a persisted `SYS_CATALOG` carrying any FK changes
/// shape again. v11 (`PGMB`) added the `ForeignKey.on_update` catalog field (the
/// parent-side `ON UPDATE` referential action, previously parsed-and-ignored), so a
/// persisted `SYS_CATALOG` carrying any FK changes shape once more. v12 (`PGMC`)
/// changed the `JsonbValue::Number` payload from `f64` to the exact `PgNumeric` (so a
/// jsonb number preserves arbitrary precision, e.g. a 30-digit integer, rather than
/// f64-rounding), so a persisted `Value::Jsonb` row — and any jsonb literal inside a
/// CHECK / partial-index `Expr` (both postcard) — changes shape. v13 (`PGMD`) changed
/// the persisted **canonical form** of a jsonb object: keys are now deduplicated and
/// stored in Postgres's order (length first, then bytewise) rather than input order.
/// The postcard *shape* is unchanged, which is exactly why the bump is needed — an
/// old row would decode cleanly and then compare and render wrong, since equality,
/// `jsonb_compare`, and `jsonb_out` all walk objects in stored order. Failing loud on
/// reopen beats a store where `'{"a":1,"b":2}'` written yesterday is `<>` the same
/// value written today. v14 (`PGME`) added the constraint marker to the catalog
/// `Index`: an index's `Uniqueness` now records whether a declared `PRIMARY KEY` /
/// `UNIQUE` constraint owns it (so `DROP INDEX` can refuse it, `information_schema`
/// can report it, and the dump can re-emit it as `ADD CONSTRAINT`), which widens the
/// flat `{unique, enforced}` wire pair to a `{unique, enforced, constraint}` triple —
/// a postcard *field* addition, and therefore a shape change to every persisted
/// `SYS_CATALOG` carrying an index. v15 (`PGMF`) made the catalog `CheckConstraint`'s
/// `name` total — an `Option<String>` became a `String`, so a persisted `SYS_CATALOG`
/// carrying any CHECK changes shape. The bump is what lets the reconstruct-a-missing-
/// name fallback be **deleted** rather than kept forever: it could only search the
/// table's own preceding checks where the minting namer searches the whole schema
/// across all four kinds, so the same database dumped two different constraint names
/// depending on which build had written it. v16 (`PGMG`) made a catalog **sequence**
/// a record rather than an `i64`: `Schema.sequences` maps a name to a
/// `{last_value, start, owner}` struct, because the counter alone cannot say where a
/// bare `ALTER SEQUENCE … RESTART` returns to nor which column owns the sequence — so
/// a persisted `SYS_CATALOG` carrying any sequence changes shape. v17 (`PGMH`) is the
/// second bump of `PGMD`'s kind — the postcard *shape* is unchanged and the *meaning*
/// of the names in it moved. The engine now folds an unquoted identifier to lower case
/// where the parser reads it, as Postgres does, so a relation, column, index, sequence
/// or constraint name a pre-v17 build registered from unquoted mixed-case DDL
/// (`CREATE TABLE Users (Id INT)`) is no longer named by the SQL that created it. The
/// silent case is what forces the bump rather than the loud one: the table is still
/// reachable as `"Users"` and a `SELECT * FROM Users` is a plain `42P01`, but
/// `CREATE TABLE IF NOT EXISTS Users` now **succeeds** and mints a second, empty
/// `users` that shadows it for every later statement — a migration idiom turning into
/// silent data-invisibility, with nothing left in the store to detect it by.
/// v18 (`PGMI`) added the `ColumnDefault::Expr` catalog variant — a column `DEFAULT`
/// may now be **any** expression, stored unevaluated as a planner `Expr` and
/// evaluated per row — so a persisted `SYS_CATALOG` gains a shape no earlier build
/// can decode. It does **three** recorded jobs, and the second and third are what
/// make it a bump rather than an append. It fails loud on the **pre-v18** stores that
/// can carry NULL rows in a `NOT NULL` column: a `DEFAULT NULL` was accepted and
/// backfilled there, which is a `23502` on 18.6 and a state stock Postgres will not
/// restore, so those stores must not reopen at all rather than reopen and dump to a
/// script nothing reloads. (Pre-v18 and not pre-v17: a `PGMH` store written before
/// that acceptance was closed carries the shape too, so naming v17 as the boundary
/// would exempt exactly the newest stores that have it.) And it retires the two
/// **legacy-store belts** written for exactly that shape — the
/// `ColumnDefault::Literal(Value::Null)` arms in the dump's `emit_create_table` and in
/// `information_schema`'s `render_column_default`, each standing in for a store this
/// magic now refuses. A
/// v19 (`PGMJ`) gave a user `ENUM` type a **catalog OID**: `Schema.enums` maps a name
/// to an `{oid, labels}` record rather than to a bare label `Vec`, so a persisted
/// `SYS_CATALOG` carrying any enum changes shape. The OID is what `pg_type` and
/// `pg_enum` are joined on (`pg_enum.enumtypid = pg_type.oid`), and it has to be
/// persisted rather than derived per session for the same reason a table's is: two
/// reads of one database must name the same type by the same number. It shares the
/// `Schema::next_oid` counter with tables and indexes, which is why
/// `next_object_oid` — the shared seed every mint site reads — reads the enum map too.
/// v20 (`PGMK`) is the **third** bump of `PGMD`'s kind — the postcard shape is
/// unchanged and the meaning of what is in it moved. An enum value now orders by its
/// members' **positions** rather than its labels' bytes, and two things a pre-v20 store
/// holds were *derived* under the old order: the rows a `CHECK` over an enum column
/// admitted, and which rows a **partial** index whose predicate orders an enum was
/// built over. Neither is re-derived on open, so such a store carries rows its own
/// constraint now rejects and an index population the engine no longer agrees with —
/// and the partial-index case is the silent one, since a **unique** partial index built
/// under the old rule holds neither of two rows a later probe must find.
/// v21 (`PGML`) is the **fourth**, and it is v20's two populations again over a wider
/// set of stored expressions: the member order now reaches an enum under a `COALESCE`,
/// a `CASE`, a `greatest`/`least` or a `nullif`, so a v20 store's `CHECK` admitted — and
/// its partial index was built over — the rows the *label bytes* chose for every such
/// predicate, which are frequently the exact complement of the rows this build chooses.
/// v22 (`PGMM`) is the **fifth**, and it is the same two populations over the same kind
/// of widening once more: an `ARRAY[…]` expression, an `unnest(…)` of one, and an
/// `op ANY`/`op ALL` reaching the enum through its array operand now order by member
/// position, where a v21 store's `CHECK (ARRAY[m] < ARRAY['ok'::mood])` admitted, and its
/// partial index over `WHERE ARRAY[m] > ARRAY['sad'::mood]` was built over, whatever the
/// label bytes chose. Nothing re-derives either on open, so the reopen has to refuse.
/// v23 (`PGMN`) is a **shape** bump of `PGM8`'s and `PGM9`'s kind: the catalog `Column`
/// gained `temporal_precision`, the sub-second `(p)` of a
/// `timestamp`/`timestamptz`/`time`/`interval` column. `#[serde(default)]` on an
/// appended-last field buys nothing in a non-self-describing format — postcard reads
/// exactly the fields the struct declares, in order, with no per-struct length — so a
/// v22 `SYS_CATALOG` decoded by this build reads one field too many out of every
/// `Column` and takes the bytes from the next one in the `Vec`. Measured on a real v22
/// store: the reopen died with `failed to deserialize schema: Hit the end of buffer,
/// expected more data`, which is the loud shape of it and not the guaranteed one — a
/// `Vec<Column>` with the right bytes after it decodes a shifted catalog instead.
/// v24 (`PGMO`) is a **shape** bump: a `date` / `timestamp` value — a row value, or a
/// constant persisted inside a stored `Expr` — is written as the Postgres binary
/// format's day / microsecond count with its `infinity` sentinels rather than as `time`'s
/// field tuple, and a `date` / `timestamp` index key is that same count — where a v23 row
/// holds `time`'s field tuple and a v23 index entry a Julian-day or Unix-epoch key.
/// v25 (`PGMP`) is the **sixth** of `PGMD`'s meaning-moved kind: an `AND`/`OR` list of
/// more than 64 operands in a stored expression is built balanced under a list root
/// (`BinaryOp::AndList`), where a v24 store holds the left-deep chain (or balanced
/// nodes this build reads as nested lists), so a v24 store's partial-index predicate
/// does not equal the same text parsed fresh, and an `ON CONFLICT … WHERE` restating it
/// would be `42P10`.
/// v26 (`PGMQ`) is a **shape** bump of the page format rather than the catalog's: a
/// leaf value too long to keep inline is stored out of line, in a chain of overflow
/// pages (a page kind no earlier build writes), and the leaf holds a reference whose
/// value-length word is `page::OVERFLOW_TAG`. A v25 store would decode — none of its
/// value lengths carries the tag — but accepting it would be the compatibility branch
/// the policy rules out, so it is refused like every other superseded magic, and a v25
/// build meeting a v26 file fails loud on the unknown magic.
/// v27 (`PGMR`) is a **shape** bump of the slot and the free-list chain: the slot
/// payload gained `freelist_crc`, the checksum its head chain page carries, and each
/// chain page's body gained its successor's checksum before the id count — so a
/// power-loss image whose slot landed over a chain page that did not reads as a short
/// chain rather than a stale list naming live pages. A v26 slot is four bytes short
/// and its chain bodies read their count from the wrong offset.
/// v28 (`PGMS`) is a **shape** bump of the slot: the pages superseded and not yet freed
/// are no longer folded into the free-list chain, which every save rewrites whole, but
/// logged apart — the slot gained `retired_head`, `retired_crc`, `retired_max` and
/// `retired_floor` and carries the log's newest entries after its metadata, and the log
/// spills into chain pages of its own (`RetiredLog`). A v27 slot is 28 bytes short and
/// carries no entries.
/// v29 (`PGMU`) is the **seventh** of `PGMD`'s meaning-moved kind: a string read as
/// `bytea` is `byteain`'s two formats (the hex format only behind its `\x` prefix), and an
/// array literal is read by 18.6's array grammar (a backslash in an unquoted element is
/// an escape, a Unicode space is an element's character, and a literal that grammar
/// refuses is refused), so a stored expression holding one — `'cafe'::bytea`, a `bytea`
/// column's default string, `'{a\,b}'::text[]`, `'{a,,b}'::text[]` — evaluates to another
/// value, or to an error, than the one a v28 store's `CHECK`
/// admitted rows under and its partial indexes were built over. A v28 store decodes
/// cleanly; a unique partial index whose predicate holds one would accept a duplicate
/// beside a stored row.
/// v30 (`PGMV`) is the **eighth** of `PGMD`'s meaning-moved kind, over two evaluations at
/// once: `round(double precision)` rounds a tie to the even integer (C's `rint`, 18.6's
/// `dround`) where a v29 build rounded it away from zero, and a `real` / `double
/// precision` read from text, or converted from `numeric`, is refused `22003` outside the
/// type's range where a v29 build answered the `±Infinity` / `0` it rounds to. A stored
/// expression holding either — `CREATE UNIQUE INDEX … ON t (round(f))`, a `CHECK (r <
/// '1e39'::real)`, a `CHECK (r > 1e-50::numeric::real)` — evaluates to another value, or to
/// an error, than the one a v29 store's `CHECK` admitted rows under and its indexes
/// were built over: a unique expression index built under v29 over `round(f)` holding
/// `2.5` admits `2.4` beside it under this build. A v29 store decodes cleanly, so the
/// magic is what refuses it.
/// A stale-magic file would mis-read its index entries, catalog, or slot — or read a
/// stored expression as another value — so reopen
/// rejects every older magic (`PGMT` v1 … `PGMU` v29) with a clear stale-format error
/// instead of decoding incidental garbage.
const META_MAGIC: &[u8; 4] = b"PGMV";

/// The superseded meta magics. A file whose slots still carry any of these is a
/// pre-`PGMV` store (`PGMT` = pre-variable-key v1, `PGM2` = pre-presence-tag v2,
/// `PGM3` = pre-serialized-`Expr`-catalog v3, `PGM4` = pre-system-entry catalog v4,
/// `PGM5` = pre-OID-encoded-`TypeOid` catalog v5, `PGM6` = pre-3-field-`Interval` v6,
/// `PGM7` = pre-`varchar_length`-catalog v7, `PGM8` = pre-`identity`-catalog v8,
/// `PGM9` = pre-composite-`ForeignKey` catalog v9, `PGMA` = pre-`on_update`-`ForeignKey`
/// catalog v10, `PGMB` = pre-`PgNumeric`-jsonb-number v11, `PGMC` =
/// pre-canonical-jsonb-key-order v12, `PGMD` = pre-constraint-marker-`Index`
/// catalog v13, `PGME` = pre-total-`CheckConstraint`-name catalog v14, `PGMF` =
/// pre-`Sequence`-record catalog v15, `PGMG` = pre-identifier-case-folding v16,
/// `PGMH` = pre-`ColumnDefault::Expr` catalog v17, `PGMI` = pre-enum-OID catalog v18,
/// `PGMJ` = pre-enum-member-order v19, `PGMK` = pre-wrapped-enum-member-order v20,
/// `PGML` = pre-array-expression-enum-member-order v21, `PGMM` =
/// pre-`temporal_precision`-catalog v22, `PGMN` = pre-infinity-temporal-count v23,
/// `PGMO` = pre-rooted-long-`AND`/`OR`-list v24, `PGMP` = pre-overflow-page v25,
/// `PGMQ` = pre-checksummed-free-list-chain v26, `PGMR` = pre-retired-page-log v27,
/// `PGMS` = pre-`byteain`-formats-and-array-element-escape v28, `PGMU` =
/// pre-`rint`-round-and-float-input-range v29);
/// reopening it fails loud with a stale-format message.
const STALE_MAGICS: [&[u8; 4]; 29] = [
    b"PGMT", b"PGM2", b"PGM3", b"PGM4", b"PGM5", b"PGM6", b"PGM7", b"PGM8", b"PGM9", b"PGMA",
    b"PGMB", b"PGMC", b"PGMD", b"PGME", b"PGMF", b"PGMG", b"PGMH", b"PGMI", b"PGMJ", b"PGMK",
    b"PGML", b"PGMM", b"PGMN", b"PGMO", b"PGMP", b"PGMQ", b"PGMR", b"PGMS", b"PGMU",
];

/// Meta slot header: magic(4) + version(8) + payload_len(4) + crc32(4).
const META_HEADER_SIZE: usize = 4 + 8 + 4 + 4;

/// Upper bound on a slot's `payload_len` — the physical slot capacity. A payload is
/// the fixed metadata and at most [`RETIRED_INLINE_MAX`] retired-log entries, under
/// 4 KiB, so this is purely a **corruption guard**: a slot
/// whose `payload_len` field claims more than one slot can hold is rejected on read
/// (it would otherwise drive a bogus allocation), never produced on write.
const MAX_META_PAYLOAD: usize = (META_SLOT_SIZE as usize) - META_HEADER_SIZE;

/// A slot's free-list chain as the store holds it: its pages, head first, and the
/// checksum the slot records for the head.
type SlotChain = (Vec<PageId>, u32);

/// A valid meta slot read from disk.
struct MetaSlot {
    version: u64,
    metadata: PageStoreMetadata,
    /// The retired-page log's newest entries, carried in the slot after the metadata.
    retired: Vec<RetiredEntry>,
}

/// How many retired-log entries a slot carries itself before a save spills them into
/// a chain page: 240 entries keep a slot write under one 4 KiB page.
const RETIRED_INLINE_MAX: usize = 240;

/// The retired-page log: every page the tree superseded and has not freed, persisted
/// so that a reopen — which no snapshot survives — finds them free. Its newest entries
/// ride in each slot; a save that would carry more than [`RETIRED_INLINE_MAX`] spills
/// them into chain pages written once and never rewritten, so a save writes the pages
/// it retired and not the whole log. A slot names the log's head and a floor: an entry
/// below the floor was freed since it was logged (it is on the free list, or live
/// again), so a reopen takes only the entries at or past it, and a walk stops at a page
/// whose largest entry is below it.
///
/// A save logs only versions past every version logged before it (the batches at or
/// past the durable version), and spills newest first, so along the chain each page's
/// entries are at or below every entry of the page before it. A walk under a floor
/// therefore reads a prefix of the chain, and a page below both slots' floors is at
/// its tail — which is where a save releases it.
#[derive(Debug)]
struct RetiredLog {
    /// The spilled chain's first page, its checksum and its largest entry, as the
    /// newest slot names them.
    head: PageId,
    head_crc: u32,
    head_max: u64,
    /// The entries the newest slot carries itself.
    inline: Vec<RetiredEntry>,
    /// Every spilled page a slot's walk reads, newest first, with its largest entry.
    pages: VecDeque<(PageId, u64)>,
    /// Each slot's head and floor (`[slot 0, slot 1]`), for the pages its walk reads.
    heads: [PageId; 2],
    floors: [u64; 2],
    /// The alternate slot's metadata as open found it, while its log's pages are
    /// unread ([`DoubleMetaStore::read_alternate_log`]).
    unread: Option<PageStoreMetadata>,
}

impl RetiredLog {
    const EMPTY: Self = Self {
        head: NULL_PAGE_ID,
        head_crc: 0,
        head_max: 0,
        inline: Vec::new(),
        pages: VecDeque::new(),
        heads: [NULL_PAGE_ID; 2],
        floors: [0; 2],
        unread: None,
    };

    /// The spilled pages `slot`'s walk reads: from its head, each page whose largest
    /// entry is at or past its floor.
    fn reach(&self, slot: usize) -> Vec<PageId> {
        self.pages
            .iter()
            .skip_while(|&&(id, _)| id != self.heads[slot])
            .take_while(|&&(_, max)| max >= self.floors[slot])
            .map(|&(id, _)| id)
            .collect()
    }
}

/// Single-file double-meta page store over a `BlockIo`.
///
/// Generic-free at the type level (holds a `Box<dyn BlockIo>`) so it slots into
/// the existing `Box<dyn PageStore>` plumbing. The active meta slot alternates
/// on every `save_state`, giving crash-atomic commits without a WAL.
pub struct DoubleMetaStore {
    io: Box<dyn BlockIo>,
    /// Which slot (0 = A, 1 = B) currently holds the committed metadata.
    active_slot: AtomicU8,
    /// Version of the active slot (monotonic; new commits get `version + 1`).
    active_version: AtomicU64,
    /// The tree version the durable slot holds: the adopted slot's at open, then
    /// each `save_state`'s once its fsync has returned. What
    /// [`PageStore::durable_version`] answers.
    durable_version: AtomicU64,
    /// Next page ID to allocate.
    next_page_id: AtomicU64,
    /// Free list of reusable page IDs.
    free_list: Mutex<Vec<PageId>>,
    /// The retired-page log ([`RetiredLog`]). Lock order: this before `free_list`.
    retired: Mutex<RetiredLog>,
    /// The page ids of the free-list chain currently persisted in each meta slot
    /// (`[slot 0, slot 1]`). When `save_state` rewrites a slot it **reuses that
    /// slot's** old chain pages for the new chain rather than fresh-allocating —
    /// safe because the slot being rewritten is always the *alternate* (a crash
    /// falls back to the active slot, whose chain is the disjoint other entry and
    /// is never touched). This bounds the file: `next_page_id` grows only when the
    /// free-list genuinely needs more chain pages than the slot last held, not once
    /// per commit. See `persist_freelist`. Beside the pages, the checksum the slot
    /// records for the head, which says what the head page holds.
    slot_chains: [Mutex<SlotChain>; 2],
    /// Write-through cache of immutable data pages, keyed by page id, sharded so
    /// concurrent readers don't serialize on one global mutex.
    cache: ShardedPageCache,
    /// The write or `fsync` that failed the store, once one has: a refused save, or a
    /// refused page write or `fsync` the tree could not drop (`CowBtree`). Once set,
    /// every page read, page write and save is refused (an audit's
    /// [`Self::persisted_chain_pages`] and `page_count` still read): the tree may have
    /// published a commit this
    /// store can no longer make durable, and whether a refused save landed is unknown.
    failure: WriteFailure,
}

impl std::fmt::Debug for DoubleMetaStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cache_len = self.cache.len();
        f.debug_struct("DoubleMetaStore")
            .field("active_slot", &self.active_slot.load(Ordering::SeqCst))
            .field(
                "active_version",
                &self.active_version.load(Ordering::SeqCst),
            )
            .field("next_page_id", &self.next_page_id.load(Ordering::SeqCst))
            .field("cache_len", &cache_len)
            .finish_non_exhaustive()
    }
}

impl DoubleMetaStore {
    /// Open a double-meta store over `io`, returning the store and the recovered
    /// metadata.
    ///
    /// An empty/new store is initialized with slot A holding an empty-DB
    /// metadata snapshot. An existing store reads both slots, validates each by
    /// CRC and by its page ids (`slot_ids_are_addressable`), and selects the valid
    /// slot with the higher version; if the higher-version slot is torn (bad CRC) or
    /// names a page id outside what the store addresses, it falls back to the other
    /// valid slot. Both slots invalid is a hard error, except on a file that is
    /// nothing but the zeros a crash inside creation leaves — no longer than the two
    /// slots, every byte zero — which never acknowledged a commit and is created
    /// afresh.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying I/O fails, or if a non-empty store has no
    /// valid meta slot and is not a creation cut short (unrecoverable corruption).
    pub fn open(io: Box<dyn BlockIo>) -> PageStoreResult<(Self, PageStoreMetadata)> {
        Self::open_with_cache_size(io, DEFAULT_CACHE_SIZE)
    }

    /// The empty store's metadata: no pages, no free list.
    const EMPTY: PageStoreMetadata = PageStoreMetadata {
        next_page_id: 0,
        root: NULL_PAGE_ID,
        version: 0,
        next_table_id: 1,
        freelist_head: NULL_PAGE_ID,
        freelist_crc: 0,
        retired_head: NULL_PAGE_ID,
        retired_crc: 0,
        retired_max: 0,
        retired_floor: 0,
    };

    /// Create the store: reserve the meta slots, write the empty store's metadata into
    /// slot A, and fsync.
    fn create(
        io: Box<dyn BlockIo>,
        cache_size: usize,
    ) -> PageStoreResult<(Self, PageStoreMetadata)> {
        let metadata = Self::EMPTY;
        // Reserve the data region so the first data page lands after the meta slots,
        // then write the initial meta into slot A and sync.
        io.set_len(DATA_REGION_START).map_err(map_io)?;
        write_slot(&*io, 0, 0, &metadata, &[])?;
        io.sync().map_err(map_io)?;
        Ok((
            Self::assemble(
                io,
                0,
                0,
                &metadata,
                (
                    Vec::new(),
                    [(Vec::new(), 0), (Vec::new(), 0)],
                    RetiredLog::EMPTY,
                ),
                cache_size,
            ),
            metadata,
        ))
    }

    fn assemble(
        io: Box<dyn BlockIo>,
        active_slot: u8,
        active_version: u64,
        metadata: &PageStoreMetadata,
        (free_list, slot_chains, retired): (Vec<PageId>, [SlotChain; 2], RetiredLog),
        cache_size: usize,
    ) -> Self {
        let [chain0, chain1] = slot_chains;
        Self {
            io,
            active_slot: AtomicU8::new(active_slot),
            active_version: AtomicU64::new(active_version),
            durable_version: AtomicU64::new(metadata.version),
            next_page_id: AtomicU64::new(metadata.next_page_id),
            free_list: Mutex::new(free_list),
            retired: Mutex::new(retired),
            slot_chains: [Mutex::new(chain0), Mutex::new(chain1)],
            cache: ShardedPageCache::new(cache_size),
            failure: WriteFailure::default(),
        }
    }

    /// Record a write or `fsync` the device refused as the store's failure, and answer
    /// it as [`PageStoreError::WriteFailed`]: from here the store refuses everything.
    fn write_failed(&self, error: DeviceError) -> PageStoreError {
        self.failure.record(error.reason.clone());
        PageStoreError::WriteFailed(error)
    }

    /// Open a double-meta store with a custom page-cache size.
    ///
    /// `cache_size` is the maximum number of pages held in the write-through
    /// read cache (4 KiB each; 2000 ≈ 8 MiB), spread across the cache shards. A
    /// `cache_size` of 0 is clamped to 1 (the cache is always present); pass a
    /// tiny value to effectively disable caching for A/B measurement.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying I/O fails, or if a non-empty store has no
    /// valid meta slot and is not a creation cut short (unrecoverable corruption).
    pub fn open_with_cache_size(
        io: Box<dyn BlockIo>,
        cache_size: usize,
    ) -> PageStoreResult<(Self, PageStoreMetadata)> {
        let len = io.len().map_err(map_io)?;

        if len == 0 {
            return Self::create(io, cache_size);
        }

        // Existing store: reject a stale on-disk format loudly before anything
        // else, so an old file fails with a clear message rather than looking
        // like incidental corruption.
        //
        // The magic list is **rendered from [`STALE_MAGICS`]**, not spelled out: a
        // hand-written copy went stale at `PGMB` and told the reader that a `PGMC` /
        // `PGMD` / `PGME` / `PGMF` file was not one of the formats being rejected, while
        // rejecting it. A message that contradicts the check is worse than a vague one.
        if file_is_stale_format(&*io)? {
            let magics: Vec<String> = STALE_MAGICS
                .iter()
                .map(|m| format!("'{}'", String::from_utf8_lossy(*m)))
                .collect();
            return Err(PageStoreError::Io(format!(
                "double-meta: stale on-disk format (superseded magic {}); re-bootstrap \
                 this dev database — the index-key page, catalog, or meta-slot format has changed",
                magics.join(", ")
            )));
        }

        // Read and validate both slots.
        let slot_a = read_slot(&*io, 0)?;
        let slot_b = read_slot(&*io, 1)?;

        let (active_slot, chosen, alternate) = match (slot_a, slot_b) {
            (Some(a), Some(b)) => {
                if b.version >= a.version {
                    (1u8, b, Some(a))
                } else {
                    (0u8, a, Some(b))
                }
            }
            (Some(a), None) => (0u8, a, None),
            (None, Some(b)) => (1u8, b, None),
            // A crash inside `create`: the length landed and slot A's write did not,
            // so the file is the zeros `set_len` wrote and nothing else. No data page
            // was ever written — a store writes its first one past both slots only
            // after creating it, and never shrinks its file — so it acknowledged
            // nothing, and is created afresh rather than refused on every open. Any
            // other byte — a foreign file, a slot damaged later — is the refusal
            // below: a store that acknowledged a commit cannot be told from damage.
            (None, None) if len <= DATA_REGION_START && is_all_zero(&*io, len)? => {
                return Self::create(io, cache_size);
            }
            (None, None) => {
                return Err(PageStoreError::Io(
                    "double-meta: both meta slots are invalid (unrecoverable corruption)".into(),
                ));
            }
        };

        let metadata = chosen.metadata.clone();
        let next_page_id = metadata.next_page_id;
        // Rebuild the in-memory free-list by walking the active slot's chain. A
        // torn, short or stale chain stops the walk early (the free-list is a
        // rebuildable optimization, not committed data) — the pages it would have
        // listed just aren't reused.
        let (mut free_list, active_chain) = read_freelist_chain(
            &*io,
            metadata.freelist_head,
            metadata.freelist_crc,
            next_page_id,
        )?;
        // The pages the active slot's retired log names are free now: no snapshot
        // survives a reopen, and no root of the adopted version reaches them.
        let active_reach =
            read_retired_log(&*io, &metadata, &chosen.retired, Some(&mut free_list))?;
        // The alternate slot's free-list chain is read for its page ids only (what it
        // lists belongs to the superseded version and may name pages live now), and held
        // for two-generation reuse: it is rewritten into on that slot's next save. Its
        // retired log is read then too, not here: its pages are needed only to release
        // them once that slot is rewritten, and a large one — the log of a big `DELETE`
        // or `DROP` just before a close — costs open a page read each.
        let alt_chain = match &alternate {
            Some(alt) => {
                read_freelist_chain(
                    &*io,
                    alt.metadata.freelist_head,
                    alt.metadata.freelist_crc,
                    next_page_id,
                )?
                .1
            }
            None => Vec::new(),
        };
        let mut log = RetiredLog::EMPTY;
        log.pages.extend(active_reach.iter().copied());
        let active = usize::from(active_slot);
        log.heads[active] = metadata.retired_head;
        log.floors[active] = metadata.retired_floor;
        if let Some(alt) = &alternate {
            log.heads[active ^ 1] = alt.metadata.retired_head;
            log.floors[active ^ 1] = alt.metadata.retired_floor;
            log.unread = Some(alt.metadata.clone());
        }
        log.head = metadata.retired_head;
        log.head_crc = metadata.retired_crc;
        log.head_max = metadata.retired_max;
        // Only what open itself took as free: an entry it would refuse is not carried on.
        log.inline = chosen
            .retired
            .into_iter()
            .filter(|&(id, freed_version)| {
                id < next_page_id
                    && freed_version < metadata.version
                    && freed_version >= metadata.retired_floor
            })
            .collect();
        // An id is handed out once: a listed repeat, or an id one of the chains is
        // written into, would be handed out while it is still in use. The store never
        // writes either; the check keeps a damaged list from acting on it. A sorted copy
        // answers whether either is there, so a well-formed list pays one sort, and only a
        // damaged one is rebuilt.
        let chain_pages: Vec<PageId> = active_chain
            .iter()
            .chain(&alt_chain)
            .copied()
            .chain(log.pages.iter().map(|&(id, _)| id))
            .collect();
        let mut sorted = free_list.clone();
        sorted.sort_unstable();
        let repeats = sorted.windows(2).any(|w| w[0] == w[1]);
        let names_a_chain_page = chain_pages
            .iter()
            .any(|id| sorted.binary_search(id).is_ok());
        if repeats || names_a_chain_page {
            let chain_pages: HashSet<PageId> = chain_pages.into_iter().collect();
            let mut listed = HashSet::with_capacity(free_list.len());
            free_list.retain(|id| !chain_pages.contains(id) && listed.insert(*id));
        }
        let active_chain = (active_chain, metadata.freelist_crc);
        let alt_chain = (
            alt_chain,
            alternate
                .as_ref()
                .map_or(0, |alt| alt.metadata.freelist_crc),
        );
        let slot_chains = if active_slot == 0 {
            [active_chain, alt_chain]
        } else {
            [alt_chain, active_chain]
        };
        let store = Self::assemble(
            io,
            active_slot,
            chosen.version,
            &metadata,
            (free_list, slot_chains, log),
            cache_size,
        );
        Ok((store, metadata))
    }

    /// Byte offset of a data page within the file, or `None` for an id at or past
    /// [`PAGE_ID_LIMIT`], which names no page the store can address — so no id, a
    /// damaged pointer's included, can overflow the offset or wrap it onto another
    /// page's bytes.
    const fn page_offset(page_id: PageId) -> Option<u64> {
        if page_id < PAGE_ID_LIMIT {
            Some(DATA_REGION_START + page_id * PAGE_SIZE as u64)
        } else {
            None
        }
    }

    /// [`Self::page_offset`] for a page the store is about to **write**: an id past
    /// the limit is the store out of addressable pages, refused before a byte moves.
    fn write_offset(page_id: PageId) -> PageStoreResult<u64> {
        Self::page_offset(page_id).ok_or_else(|| {
            PageStoreError::Io(format!(
                "double-meta: page {page_id} is past the last page the store can address"
            ))
        })
    }

    /// The chain pages the two meta slots name, the active slot's first — each slot's
    /// free-list chain and the retired-log pages its walk reads (a retired-log page
    /// both walks read is in both): pages that are neither in the tree nor on the free
    /// list, each held until its slot is rewritten. For an audit that accounts for
    /// every allocated page.
    pub fn persisted_chain_pages(&self) -> [Vec<PageId>; 2] {
        let active = usize::from(self.active_slot.load(Ordering::SeqCst));
        let mut retired = self.retired.lock();
        // An audit asks what the slots name, so the alternate log is read if it is not
        // yet; a read error leaves it unread, and its pages out of the answer.
        let _ = self.read_alternate_log(&mut retired);
        let slot = |i: usize| {
            let mut pages = self.slot_chains[i].lock().0.clone();
            pages.extend(retired.reach(i));
            pages
        };
        [slot(active), slot(active ^ 1)]
    }

    /// Stamp a serialized page's checksum field with the checksum `put` writes and
    /// `get` verifies — the one rule, for a tool or test that edits a store image by
    /// hand and needs its damage to reach past the checksum to the page codec and
    /// the tree. The store stamps every page it writes itself.
    pub fn stamp_page_checksum(page: &mut [u8; PAGE_SIZE]) {
        let crc = page_checksum(page);
        page[28..32].copy_from_slice(&crc.to_le_bytes());
    }

    /// Persist the free list as a chain of pages into `next_slot`'s generation,
    /// returning the chain head (`NULL_PAGE_ID` when the list is empty) and the
    /// checksum the head page carries (0 then).
    ///
    /// Breaks the free-list self-reference **without unbounded growth**: the chain
    /// pages are reused from the slot being overwritten (`slot_chains[next_slot]`),
    /// fresh-allocating only a shortfall. `next_slot` is always the *alternate* slot,
    /// so its old chain is safe to overwrite — a crash mid-commit falls back to the
    /// *active* slot, whose chain is the disjoint `slot_chains[active]` entry and is
    /// never touched. The persisted free-list never lists a live chain page (chain
    /// pages live in `slot_chains`, never the free-list), so there is no
    /// self-reference.
    ///
    /// When the list needs fewer pages than the slot's old chain held, the surplus
    /// pages are free — no slot will name them — and are **listed in the chain being
    /// written**, as well as put back on the in-memory free list: a close or a kill
    /// before another save would otherwise leave them named by nothing. Listing them
    /// can need a page more, which takes one back from the surplus, and a page kept
    /// that way with no id left for it is written as an empty chain page — an empty
    /// list keeps one page of a slot's old chain rather than dropping it unnamed.
    ///
    /// The pages are written tail first, each carrying the checksum of the page after
    /// it and the slot the head's, so the slot vouches for every page of its chain: a
    /// page this commit wrote but the device did not land still holds an earlier
    /// commit's list, and the open-time walk stops there instead of reading it.
    ///
    /// The earlier design fresh-allocated every chain and freed the prior one into
    /// the free-list, which leaked ~1 page per commit under steady-state churn: when
    /// data alloc/free is net-zero the freed chain pages accumulate and
    /// `next_page_id` grows every commit. Reusing the slot's own chain keeps
    /// `next_page_id` flat in steady state. (A free-list size that oscillates exactly
    /// across a chain-page boundary with zero net data allocation can still
    /// fresh-allocate per up-crossing — far slower than the old growth, and absorbed
    /// by any real data churn; a free-list pop fixpoint would close even that.)
    ///
    /// A save with nothing to list writes no chain page once the slot's chain is one
    /// page already holding the empty list — the page its previous save wrote empty,
    /// which nothing has written since (it is the slot's, off the free list), so the new
    /// slot names it with the checksum the old one recorded and it is not rewritten.
    /// Only the first save into each slot after the list empties writes that page.
    fn persist_freelist(&self, next_slot: u8) -> PageStoreResult<(PageId, u32)> {
        let pages_for = |ids: usize| ids.div_ceil(FREELIST_IDS_PER_PAGE);
        let (mut reuse, head_crc) =
            std::mem::take(&mut *self.slot_chains[next_slot as usize].lock());
        // Snapshot the list to persist, with the surplus of the slot's old chain folded
        // in. `pages` is how many chain pages the slot keeps: the fewest that hold the
        // list with the rest of the old chain listed on it. Listing a page can need one
        // more page than the list alone did, so it is found by counting up; where the
        // count lands on more pages than the ids fill, the last ones are written empty
        // rather than left named by nothing. By construction the list holds no live
        // chain page, so the chain pages chosen below are never in it.
        let (to_persist, pages): (Vec<PageId>, usize) = {
            let mut free = self.free_list.lock();
            let mut pages = pages_for(free.len());
            if pages < reuse.len() {
                while pages_for(free.len() + reuse.len() - pages) > pages {
                    pages += 1;
                }
                let surplus = reuse.split_off(pages);
                free.extend(surplus);
            }
            (free.clone(), pages)
        };
        // Nothing to list, and the slot's one chain page already lists nothing: keep it.
        if let ([page], true) = (reuse.as_slice(), to_persist.is_empty() && pages == 1)
            && head_crc == empty_chain_page_crc()
        {
            let page = *page;
            *self.slot_chains[next_slot as usize].lock() = (vec![page], head_crc);
            return Ok((page, head_crc));
        }
        let mut chunks: Vec<&[PageId]> = to_persist.chunks(FREELIST_IDS_PER_PAGE).collect();
        chunks.resize(pages, &[]);

        // Reuse the pages the slot we're overwriting currently holds; fresh-allocate
        // only the shortfall (the free-list grew past what the slot's chain held).
        let chain_ids: Vec<PageId> = (0..chunks.len())
            .map(|_| {
                reuse
                    .pop()
                    .unwrap_or_else(|| self.next_page_id.fetch_add(1, Ordering::SeqCst))
            })
            .collect();
        debug_assert!(
            reuse.is_empty(),
            "every page of the slot's old chain is kept or listed"
        );

        // Tail first, so each page carries its successor's checksum.
        let (mut next, mut next_crc) = (NULL_PAGE_ID, 0);
        for (&page_id, chunk) in chain_ids.iter().zip(&chunks).rev() {
            next_crc = write_freelist_page(&*self.io, page_id, next, next_crc, chunk)?;
            next = page_id;
        }

        // Record this slot's new chain so its next rewrite reuses these pages.
        *self.slot_chains[next_slot as usize].lock() = (chain_ids, next_crc);
        Ok((next, next_crc))
    }

    /// Read the alternate slot's retired log if open left it unread, adding each page
    /// its walk reads to `log.pages`: the alternate chain is the active one's past the
    /// active slot's newer pages, so what only it reads lies below the active floor, at
    /// the tail. A read that fails leaves the log unread and adds nothing.
    fn read_alternate_log(&self, log: &mut RetiredLog) -> PageStoreResult<()> {
        if let Some(meta) = &log.unread {
            let read = read_retired_log(&*self.io, meta, &[], None)?;
            let known: HashSet<PageId> = log.pages.iter().map(|&(id, _)| id).collect();
            for page in read {
                if !known.contains(&page.0) {
                    log.pages.push_back(page);
                }
            }
            log.unread = None;
        }
        Ok(())
    }

    /// Log the pages the tree has superseded and not freed into `next_slot`'s
    /// generation, returning the slot fields that name the log and the entries the slot
    /// carries itself.
    ///
    /// `superseded` is the tree's own pending set (`CowBtree::save_state`), each batch
    /// with its `freed_version`, in the order the tree published them — so the versions
    /// rise. The floor is its smallest `freed_version`, the first — collection
    /// frees every batch below a threshold at once, so an entry below the floor was
    /// freed since it was logged, and every entry at or past it is still retired. What
    /// is new since the last durable save — the batches at or past the durable version —
    /// joins the entries the newest slot carries; past [`RETIRED_INLINE_MAX`] they are
    /// spilled, newest first, into pages linked in front of the spilled chain. A spilled
    /// page is written once: a later save links past it or releases it, never rewrites
    /// it, so a save writes what it retired and not the whole log.
    ///
    /// A spilled page is allocated — from the free list, or a fresh id when the list is
    /// empty — before the free list is snapshotted, so it is never listed free, and goes
    /// back to the free list once neither slot's
    /// walk reads it, the same two-generation rule the free-list chain keeps.
    fn persist_retired(
        &self,
        log: &mut RetiredLog,
        next_slot: u8,
        version: u64,
        superseded: &[(Vec<PageId>, u64)],
    ) -> PageStoreResult<(Vec<RetiredEntry>, u64)> {
        self.read_alternate_log(log)?;
        // The pending set is in the order the tree published it, so its versions rise:
        // the floor is its first, and what is new since the durable save is a suffix.
        let durable = self.durable_version.load(Ordering::SeqCst);
        let floor = superseded
            .first()
            .map_or(version, |&(_, freed_version)| freed_version)
            .min(version);
        let new_since = superseded.partition_point(|&(_, freed_version)| freed_version < durable);
        let mut inline: Vec<RetiredEntry> = log
            .inline
            .iter()
            .copied()
            .filter(|&(_, freed_version)| freed_version >= floor)
            .chain(
                superseded[new_since..]
                    .iter()
                    .filter(|&&(_, freed_version)| freed_version < version)
                    .flat_map(|(ids, freed_version)| {
                        ids.iter().map(move |&id| (id, *freed_version))
                    }),
            )
            .collect();
        let (mut head, mut head_crc, mut head_max) = (log.head, log.head_crc, log.head_max);
        if inline.len() > RETIRED_INLINE_MAX {
            // Newest first, so a walk meets the pages in falling order of their entries.
            inline.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let chunks: Vec<&[RetiredEntry]> = inline.chunks(RETIRED_PER_PAGE).collect();
            let ids: Vec<PageId> = chunks.iter().map(|_| self.allocate()).collect();
            for (&id, chunk) in ids.iter().zip(&chunks).rev() {
                let max = chunk.iter().map(|&(_, v)| v).max().unwrap_or(0);
                let crc = write_retired_page(&*self.io, id, head, head_crc, head_max, chunk)?;
                log.pages.push_front((id, max));
                (head, head_crc, head_max) = (id, crc, max);
            }
            inline.clear();
        }
        log.head = head;
        log.head_crc = head_crc;
        log.head_max = head_max;
        // A page below both this save's floor and the active slot's is read by neither
        // walk once this slot is rewritten — the only slot that could still read it — so
        // it is released. Such pages are the chain's tail.
        let keep = floor.min(log.floors[usize::from(next_slot ^ 1)]);
        let mut released = Vec::new();
        while let Some(&(id, max)) = log.pages.back()
            && max < keep
        {
            log.pages.pop_back();
            released.push(id);
        }
        self.free_list.lock().extend(released);
        Ok((inline, floor))
    }
}

/// Map a `std::io::Error` into a `PageStoreError`.
// by-value so it's a direct `.map_err(map_io)` callback — `map_err` yields the error
// owned, and a `&io::Error` signature would force a closure wrapper at every call site
#[allow(clippy::needless_pass_by_value)]
fn map_io(e: std::io::Error) -> PageStoreError {
    PageStoreError::Io(e.to_string())
}

/// Map a refused **write or `fsync`** into [`PageStoreError::WriteRefused`], keeping
/// whether the device was full. Reads keep [`map_io`].
// by-value for the same reason as `map_io`
#[allow(clippy::needless_pass_by_value)]
fn map_write(e: std::io::Error) -> PageStoreError {
    PageStoreError::WriteRefused(DeviceError::from_io(&e))
}

/// A refused write or `fsync` of a save's meta slot as [`PageStoreError::SaveUnconfirmed`]:
/// the slot may have landed. Any other error passes through.
fn save_unconfirmed(e: PageStoreError) -> PageStoreError {
    match e {
        PageStoreError::WriteRefused(error) => PageStoreError::SaveUnconfirmed(error),
        other => other,
    }
}

/// Byte offset of meta slot `slot` (0 or 1).
const fn slot_offset(slot: u8) -> u64 {
    (slot as u64) * META_SLOT_SIZE
}

/// Encode and write a meta slot. Fails loudly if the payload overflows the slot.
fn write_slot(
    io: &dyn BlockIo,
    slot: u8,
    version: u64,
    metadata: &PageStoreMetadata,
    retired: &[RetiredEntry],
) -> PageStoreResult<()> {
    // The slot payload is the fixed metadata (`PageStoreMetadata::SERIALIZED_LEN`)
    // and at most `RETIRED_INLINE_MAX` retired-log entries, so it stays under a page
    // and far under the slot; the old "catalog/free-list too large to inline" cap is
    // gone. The catalog/mappings/counters live in the tree and the free-list in a page
    // chain, leaving only the chain's head and its checksum (`freelist_head`,
    // `freelist_crc`) in the slot.
    let mut payload = metadata.serialize();
    payload.extend_from_slice(&encode_retired_entries(retired));
    #[allow(clippy::cast_possible_truncation)]
    let payload_len = payload.len() as u32;

    // CRC covers version | payload_len | payload.
    let mut crc_input = Vec::with_capacity(8 + 4 + payload.len());
    crc_input.extend_from_slice(&version.to_le_bytes());
    crc_input.extend_from_slice(&payload_len.to_le_bytes());
    crc_input.extend_from_slice(&payload);
    let crc = crc32(&crc_input);

    let mut buf = Vec::with_capacity(META_HEADER_SIZE + payload.len());
    buf.extend_from_slice(META_MAGIC);
    buf.extend_from_slice(&version.to_le_bytes());
    buf.extend_from_slice(&payload_len.to_le_bytes());
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&payload);

    io.write_at(slot_offset(slot), &buf).map_err(map_write)
}

/// Whether the first `len` bytes of the file are all zero — the file `create` leaves
/// when its length lands and its first slot write does not.
fn is_all_zero(io: &dyn BlockIo, len: u64) -> PageStoreResult<bool> {
    let mut buf = vec![0u8; usize::try_from(len).unwrap_or(0)];
    let n = io.read_at(0, &mut buf).map_err(map_io)?;
    Ok(n == buf.len() && buf.iter().all(|&b| b == 0))
}

/// Whether either meta slot still carries a superseded (pre-`PGMV`) magic.
///
/// A genuinely fresh or current-format file returns `false`; any pre-`PGMV` file
/// (`PGMT` v1 … `PGMU` v29 — [`STALE_MAGICS`] is the enumeration, and this sentence
/// has to name its current end rather than the end it had when written) returns
/// `true` so
/// [`open_with_cache_size`](DoubleMetaStore::open_with_cache_size) can fail loud
/// with a stale-format message rather than mis-reading its index entries, catalog,
/// or slot.
fn file_is_stale_format(io: &dyn BlockIo) -> PageStoreResult<bool> {
    for slot in [0u8, 1u8] {
        let mut magic = [0u8; 4];
        let n = io.read_at(slot_offset(slot), &mut magic).map_err(map_io)?;
        if n == 4 && STALE_MAGICS.contains(&&magic) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Read and CRC-validate a meta slot, returning `None` if invalid/torn.
fn read_slot(io: &dyn BlockIo, slot: u8) -> PageStoreResult<Option<MetaSlot>> {
    let mut header = [0u8; META_HEADER_SIZE];
    let n = io.read_at(slot_offset(slot), &mut header).map_err(map_io)?;
    if n < META_HEADER_SIZE {
        return Ok(None);
    }
    if &header[0..4] != META_MAGIC {
        return Ok(None);
    }

    let version = u64::from_le_bytes(header[4..12].try_into().unwrap_or([0; 8]));
    let payload_len = u32::from_le_bytes(header[12..16].try_into().unwrap_or([0; 4])) as usize;
    let stored_crc = u32::from_le_bytes(header[16..20].try_into().unwrap_or([0; 4]));

    if payload_len > MAX_META_PAYLOAD {
        return Ok(None);
    }

    let mut payload = vec![0u8; payload_len];
    let read = io
        .read_at(slot_offset(slot) + META_HEADER_SIZE as u64, &mut payload)
        .map_err(map_io)?;
    if read < payload_len {
        return Ok(None);
    }

    // Verify CRC over version | payload_len | payload.
    let mut crc_input = Vec::with_capacity(8 + 4 + payload_len);
    crc_input.extend_from_slice(&version.to_le_bytes());
    crc_input.extend_from_slice(&(payload_len as u32).to_le_bytes());
    crc_input.extend_from_slice(&payload);
    if crc32(&crc_input) != stored_crc {
        return Ok(None);
    }

    let Some(metadata) = PageStoreMetadata::deserialize(&payload) else {
        return Ok(None);
    };
    let Some(retired) = payload
        .get(PageStoreMetadata::SERIALIZED_LEN..)
        .and_then(decode_retired_entries)
    else {
        return Ok(None);
    };
    if !slot_ids_are_addressable(&metadata) {
        return Ok(None);
    }

    Ok(Some(MetaSlot {
        version,
        metadata,
        retired,
    }))
}

/// Whether a slot's page ids stay inside what the store can address and has
/// allocated: `next_page_id` at most [`PAGE_ID_LIMIT`], and the `root`, the
/// `freelist_head` and the `retired_head` each either the null link or an id below
/// `next_page_id`. `save_state`
/// reads `next_page_id` after the root and both chain heads are allocated, so the three
/// links hold by construction, and it refuses to write a slot that fails — which, from a store
/// of its own writing, is one whose `next_page_id` an allocation at the limit has carried
/// past it.
///
/// These are the slot's fields a page offset or an allocation is computed from:
/// `next_page_id` is where every fresh allocation starts and bounds every link a walk
/// accepts, `root` is the
/// first page a read fetches, `freelist_head` the first free-list page open reads, and
/// `retired_head` the first retired-log page open reads.
/// A slot that fails is treated as a torn one — the slot CRC vouches for the bytes,
/// not for what they claim — so open falls back to the other slot, and refuses when
/// neither is valid.
const fn slot_ids_are_addressable(metadata: &PageStoreMetadata) -> bool {
    let next = metadata.next_page_id;
    next <= PAGE_ID_LIMIT
        && (metadata.root == NULL_PAGE_ID || metadata.root < next)
        && (metadata.freelist_head == NULL_PAGE_ID || metadata.freelist_head < next)
        && (metadata.retired_head == NULL_PAGE_ID || metadata.retired_head < next)
}

/// Write one free-list chain page: the [`encode_freelist_body`] payload at offset
/// 0, then a CRC32 in the last 4 bytes over everything before it, which is returned —
/// the checksum the page's predecessor (or the slot, for the head) records for it.
/// `next_page` and `next_crc` link the chain (`NULL_PAGE_ID` and 0 at the tail).
fn write_freelist_page(
    io: &dyn BlockIo,
    page_id: PageId,
    next_page: PageId,
    next_crc: u32,
    ids: &[PageId],
) -> PageStoreResult<u32> {
    let body = encode_freelist_body(next_page, next_crc, ids);
    let mut buf = [0u8; PAGE_SIZE];
    // `ids.len() <= FREELIST_IDS_PER_PAGE` (the caller chunks), so the body fits
    // before the 4-byte CRC tail with margin.
    buf[..body.len()].copy_from_slice(&body);
    let crc = crc32(&buf[..PAGE_SIZE - 4]);
    buf[PAGE_SIZE - 4..].copy_from_slice(&crc.to_le_bytes());
    io.write_at(DoubleMetaStore::write_offset(page_id)?, &buf)
        .map_err(map_write)?;
    Ok(crc)
}

/// The checksum of a free-list chain page listing nothing and linking nowhere — the
/// page a slot's chain keeps once its list is empty ([`write_freelist_page`]'s bytes
/// for `NULL_PAGE_ID`, 0 and no ids, which do not depend on the page's id).
fn empty_chain_page_crc() -> u32 {
    let body = encode_freelist_body(NULL_PAGE_ID, 0, &[]);
    let mut buf = [0u8; PAGE_SIZE];
    buf[..body.len()].copy_from_slice(&body);
    crc32(&buf[..PAGE_SIZE - 4])
}

/// One free-list chain page as read: the next link, the checksum that link's page
/// must carry, the ids listed, and this page's own checksum.
type ChainPage = (PageId, u32, Vec<PageId>, u32);

/// Read one chain page — a free-list or a retired-log page — as its body (every byte
/// before the checksum) and the checksum it carries, or `None` if the page is
/// missing, short or fails that checksum.
fn read_chain_page(io: &dyn BlockIo, page_id: PageId) -> PageStoreResult<Option<(Vec<u8>, u32)>> {
    let Some(offset) = DoubleMetaStore::page_offset(page_id) else {
        return Ok(None);
    };
    let file_len = io.len().map_err(map_io)?;
    if offset + PAGE_SIZE as u64 > file_len {
        return Ok(None);
    }
    let mut buf = vec![0u8; PAGE_SIZE];
    let n = io.read_at(offset, &mut buf).map_err(map_io)?;
    if n < PAGE_SIZE {
        return Ok(None);
    }
    let stored = u32::from_le_bytes([
        buf[PAGE_SIZE - 4],
        buf[PAGE_SIZE - 3],
        buf[PAGE_SIZE - 2],
        buf[PAGE_SIZE - 1],
    ]);
    if crc32(&buf[..PAGE_SIZE - 4]) != stored {
        return Ok(None);
    }
    buf.truncate(PAGE_SIZE - 4);
    Ok(Some((buf, stored)))
}

/// Read one free-list chain page, or `None` if the page is missing/short or its CRC
/// fails — a torn chain page, which ends the recoverable walk (the free-list is a
/// rebuildable optimization, not committed data, so a torn tail just means some
/// reusable pages aren't recovered).
fn read_freelist_page(io: &dyn BlockIo, page_id: PageId) -> PageStoreResult<Option<ChainPage>> {
    Ok(read_chain_page(io, page_id)?.and_then(|(body, stored)| {
        decode_freelist_body(&body).map(|(next, next_crc, ids)| (next, next_crc, ids, stored))
    }))
}

/// Walk the free-list chain from `head`, whose page must carry `head_crc`, returning
/// the rebuilt free-list and the chain's page ids (held as that slot's chain, which its
/// next save rewrites in place — `persist_freelist`).
///
/// The walk is bounded loud-and-safe: a chain page id `>= next_page_id` (never
/// allocated) or a revisited id (a corrupt cycle) stops the walk, as does a torn
/// page (bad CRC), a page whose checksum is not the one its predecessor (or the slot)
/// recorded — a page left from another commit, whose ids may be live now — and a page
/// listing a free id `>= next_page_id`, an id no page was ever allocated under, which
/// `allocate` would otherwise hand out and a later fresh allocation hand out again. A
/// truncated, torn or stale chain recovers a prefix of the free-list: the unrecovered
/// pages simply aren't reused. What the walk cannot see is a checksum-valid list the
/// slot's own commit wrote wrong — it trusts the ids an authentic page lists.
fn read_freelist_chain(
    io: &dyn BlockIo,
    head: PageId,
    head_crc: u32,
    next_page_id: PageId,
) -> PageStoreResult<(Vec<PageId>, Vec<PageId>)> {
    let mut free_list = Vec::new();
    let mut chain_pages = Vec::new();
    let mut visited = HashSet::new();
    let mut cur = head;
    let mut expected = head_crc;
    while cur != NULL_PAGE_ID {
        if cur >= next_page_id || !visited.insert(cur) {
            break;
        }
        let Some((next, next_crc, ids, crc)) = read_freelist_page(io, cur)? else {
            break;
        };
        if crc != expected || ids.iter().any(|&id| id >= next_page_id) {
            break;
        }
        chain_pages.push(cur);
        free_list.extend(ids);
        cur = next;
        expected = next_crc;
    }
    Ok((free_list, chain_pages))
}

/// Write one retired-log chain page: the [`encode_retired_body`] payload, then a CRC32
/// in the last 4 bytes over everything before it, which is returned — the checksum
/// its predecessor (or the slot) records for it.
fn write_retired_page(
    io: &dyn BlockIo,
    page_id: PageId,
    next_page: PageId,
    next_crc: u32,
    next_max: u64,
    entries: &[RetiredEntry],
) -> PageStoreResult<u32> {
    let body = encode_retired_body(next_page, next_crc, next_max, entries);
    let mut buf = [0u8; PAGE_SIZE];
    buf[..body.len()].copy_from_slice(&body);
    let crc = crc32(&buf[..PAGE_SIZE - 4]);
    buf[PAGE_SIZE - 4..].copy_from_slice(&crc.to_le_bytes());
    io.write_at(DoubleMetaStore::write_offset(page_id)?, &buf)
        .map_err(map_write)?;
    Ok(crc)
}

/// Read a slot's retired-page log — `inline`, the entries it carries itself, and the
/// chain its `meta` names: return each spilled page the walk reads, in chain order with
/// its largest entry, and — when `free` is given, for the adopted slot — append every
/// entry at or past the slot's floor to it, since after a reopen no snapshot or root
/// reaches them.
///
/// Trusted as the free-list chain is, and bounded the same way: the walk reads a page
/// only while the largest entry recorded for it is at or past the floor, and stops at
/// a page id at or past `next_page_id`, a page reached twice, a torn page, a page whose
/// checksum is not the one recorded for it, whose largest entry is not the one
/// recorded, or that lists an id at or past `next_page_id` or a version at or past the
/// slot's own — each a page from another commit or damage, whose ids may be live. An
/// entry the slot carries itself is taken on the same terms.
fn read_retired_log(
    io: &dyn BlockIo,
    meta: &PageStoreMetadata,
    inline: &[RetiredEntry],
    mut free: Option<&mut Vec<PageId>>,
) -> PageStoreResult<Vec<(PageId, u64)>> {
    let (next_page_id, version, floor) = (meta.next_page_id, meta.version, meta.retired_floor);
    let well_formed =
        |&(id, freed_version): &RetiredEntry| id < next_page_id && freed_version < version;
    if let Some(free) = free.as_deref_mut() {
        free.extend(
            inline
                .iter()
                .filter(|e| well_formed(e) && e.1 >= floor)
                .map(|&(id, _)| id),
        );
    }
    let mut reach = Vec::new();
    let mut visited = HashSet::new();
    let (mut cur, mut expected, mut cur_max) =
        (meta.retired_head, meta.retired_crc, meta.retired_max);
    while cur != NULL_PAGE_ID && cur_max >= floor {
        if cur >= next_page_id || !visited.insert(cur) {
            break;
        }
        let Some(buf) = read_chain_page(io, cur)? else {
            break;
        };
        let Some((next, next_crc, next_max, entries)) = decode_retired_body(&buf.0) else {
            break;
        };
        let max = entries.iter().map(|&(_, v)| v).max().unwrap_or(0);
        if buf.1 != expected || max != cur_max || !entries.iter().all(well_formed) {
            break;
        }
        reach.push((cur, max));
        if let Some(free) = free.as_deref_mut() {
            free.extend(entries.iter().filter(|e| e.1 >= floor).map(|&(id, _)| id));
        }
        (cur, expected, cur_max) = (next, next_crc, next_max);
    }
    Ok(reach)
}

impl PageStore for DoubleMetaStore {
    fn allocate(&self) -> PageId {
        // Bind the pop before the `if let` so the lock guard drops at the end of
        // this statement, not across the branch body.
        let reused = self.free_list.lock().pop();
        if let Some(id) = reused {
            return id;
        }
        self.next_page_id.fetch_add(1, Ordering::SeqCst)
    }

    fn get(&self, page_id: PageId) -> PageStoreResult<Option<Page>> {
        self.failure.check()?;
        // Cache hit: serve without touching the BlockIo. Routed to the page id's
        // shard, so concurrent readers serialize only on a shared shard.
        if let Some(page) = self.cache.get(page_id) {
            return Ok(Some(page));
        }

        // Cache miss: read from the BlockIo, then cache the immutable page. An id past
        // every page the store can address is a pointer no write produced — damage,
        // refused rather than wrapped onto another page's bytes.
        let offset = Self::page_offset(page_id).ok_or(PageStoreError::Corruption(page_id))?;
        // A genuine I/O failure (EIO, a flaky fd / OPFS handle) is corruption-class,
        // not "page absent": propagate it loud rather than masking it as `Ok(None)`,
        // which a B+tree descent reads as a missing child/row → silent data loss.
        // `Ok(None)` stays reserved for a page legitimately past the written region.
        let file_len = self.io.len().map_err(map_io)?;
        if offset + PAGE_SIZE as u64 > file_len {
            return Ok(None);
        }
        let mut buf = [0u8; PAGE_SIZE];
        let n = self.io.read_at(offset, &mut buf).map_err(map_io)?;
        // Checked again after the read: a failure recorded while this read was in flight
        // may be followed by a store reopened over the same file, whose writes this read
        // can see. The reopen comes after the record, so a read that sees its bytes sees
        // the record here, and its bytes never reach an answer.
        self.failure.check()?;
        if n < PAGE_SIZE {
            return Ok(None);
        }
        // Verify the page checksum before trusting the bytes. A torn or
        // bit-rotted data page is corruption, not a missing page — fail loud
        // (a catchable `Corruption` error) and refuse to return wrong data
        // rather than silently decode garbage. CoW makes pages immutable once
        // written, so a mismatch is real damage, not a torn-write race.
        let stored = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]);
        let computed = page_checksum(&buf);
        if computed != stored {
            return Err(PageStoreError::Corruption(page_id));
        }
        // A checksum-valid page that does not decode is still corruption, never a
        // panic: the codec is fallible end to end.
        let page = Page::deserialize(&buf).ok_or(PageStoreError::Corruption(page_id))?;

        self.cache.put(page_id, page.clone());
        Ok(Some(page))
    }

    fn put(&self, page: Page) -> PageStoreResult<()> {
        // Write-THROUGH: the write still hits disk every time (durability and
        // the 2-fsyncs-per-commit count are unchanged); the cache only mirrors
        // it so a later read is served without a syscall.
        self.failure.check()?;
        let page_id = page.page_id();
        let offset = Self::write_offset(page_id)?;
        let mut data = page.serialize()?;
        // Stamp the page checksum over the serialized bytes (verified in `get`).
        let crc = page_checksum(&data);
        data[28..32].copy_from_slice(&crc.to_le_bytes());
        // A refused page write does not fail the store: the tree decides
        // (`PageStoreError::WriteRefused`).
        self.io.write_at(offset, &data).map_err(map_write)?;

        self.cache.put(page_id, page);
        Ok(())
    }

    fn free_pages(&self, page_ids: Vec<PageId>) {
        // Evict freed ids from the cache FIRST: CoW reuses freed page ids for
        // later allocations, so a stale cached page under a reused id would be a
        // silent corruption. Each id is popped from its own shard (the same shard
        // a later `get` for that id reads), so the eviction is never lost.
        for &id in &page_ids {
            self.cache.pop(id);
        }
        self.free_list.lock().extend(page_ids);
    }

    fn durable_version(&self) -> Option<u64> {
        Some(self.durable_version.load(Ordering::SeqCst))
    }

    fn page_count(&self) -> usize {
        let len = self.io.len().unwrap_or(DATA_REGION_START);
        let data_bytes = len.saturating_sub(DATA_REGION_START);
        (data_bytes / PAGE_SIZE as u64) as usize
    }

    fn free_list_size(&self) -> usize {
        self.free_list.lock().len()
    }

    fn next_page_id(&self) -> PageId {
        self.next_page_id.load(Ordering::SeqCst)
    }

    fn free_list(&self) -> Vec<PageId> {
        self.free_list.lock().clone()
    }

    fn flush(&self) -> PageStoreResult<()> {
        self.failure.check()?;
        // First fsync of a commit: data pages are durable. A refused one does not fail
        // the store: the tree decides (`PageStoreError::WriteRefused`).
        self.io.sync().map_err(map_write)
    }

    fn write_failure(&self) -> Option<&WriteFailure> {
        Some(&self.failure)
    }

    fn save_state(
        &self,
        root: PageId,
        version: u64,
        next_table_id: u32,
        superseded: &[(Vec<PageId>, u64)],
    ) -> PageStoreResult<()> {
        self.failure.check()?;
        // Any failure here fails the store, whatever the step. One before the slot write
        // (a refused chain write, a slot the store will not write) leaves the version
        // undurable; one at the slot write or the `fsync` after it may not have, since
        // the slot may have landed (`SaveUnconfirmed`).
        self.save_state_inner(root, version, next_table_id, superseded)
            .map_err(|e| match e {
                PageStoreError::WriteRefused(error) | PageStoreError::WriteFailed(error) => {
                    self.write_failed(error)
                }
                PageStoreError::SaveUnconfirmed(error) => {
                    self.failure.record(error.reason.clone());
                    PageStoreError::SaveUnconfirmed(error)
                }
                other => self.write_failed(DeviceError {
                    reason: other.to_string(),
                    disk_full: false,
                }),
            })
    }
}

impl DoubleMetaStore {
    /// [`PageStore::save_state`]'s body, whose every error fails the store.
    fn save_state_inner(
        &self,
        root: PageId,
        version: u64,
        next_table_id: u32,
        superseded: &[(Vec<PageId>, u64)],
    ) -> PageStoreResult<()> {
        // The commit rewrites the ALTERNATE slot; `persist_freelist` reuses that
        // slot's old chain pages (safe — a crash falls back to the active slot,
        // whose chain is untouched), so compute the target slot first.
        let current_slot = self.active_slot.load(Ordering::SeqCst);
        let next_slot = current_slot ^ 1;
        let next_version = self.active_version.load(Ordering::SeqCst) + 1;

        // Log the superseded pages first — a spilled page is allocated (from the free
        // list, or fresh), and a released one returned to it — then persist the free list as a page
        // chain in next_slot's generation (either may bump next_page_id), so read
        // next_page_id and the chain heads AFTER, into the slot.
        let mut log = self.retired.lock();
        let (inline, retired_floor) =
            self.persist_retired(&mut log, next_slot, version, superseded)?;
        let (freelist_head, freelist_crc) = self.persist_freelist(next_slot)?;
        let metadata = PageStoreMetadata {
            next_page_id: self.next_page_id.load(Ordering::SeqCst),
            root,
            version,
            next_table_id,
            freelist_head,
            freelist_crc,
            retired_head: log.head,
            retired_crc: log.head_crc,
            retired_max: log.head_max,
            retired_floor,
        };

        // A slot open would refuse is not written: open would fall back to the other
        // slot and silently lose this commit. From a store of its own writing that is a
        // `next_page_id` an allocation at the limit has carried past it.
        if !slot_ids_are_addressable(&metadata) {
            return Err(PageStoreError::Io(format!(
                "double-meta: next page id {} is past the last page the store can address",
                metadata.next_page_id
            )));
        }

        // Write the fixed slot into the ALTERNATE slot, then fsync — that fsync is
        // the commit point — then flip the active-slot pointer. The chain pages
        // written just above ride the same fsync, so the device may land the slot
        // without one of them; the checksum each records for the next is what makes
        // that a short free list on reopen rather than a stale one (the data is
        // already durable from the apply_batch flush).
        // From the slot write on, a refusal does not say the slot is absent: a write
        // refused part-way, or an `fsync` refused after its bytes reached the device's
        // cache, can leave the slot to be adopted by the next open.
        write_slot(&*self.io, next_slot, next_version, &metadata, &inline)
            .map_err(save_unconfirmed)?;
        // Second fsync of a commit: the meta flip is durable. This is the
        // atomic commit point — a crash before it leaves the old active slot.
        self.io.sync().map_err(|e| save_unconfirmed(map_write(e)))?;
        log.inline = inline;
        log.heads[next_slot as usize] = metadata.retired_head;
        log.floors[next_slot as usize] = retired_floor;
        drop(log);

        // Flip the active slot only after the alternate is durable, and only then let
        // collection free the pages the older durable version reaches. A failure above
        // leaves the older slot durable, and its pages held.
        self.active_slot.store(next_slot, Ordering::SeqCst);
        self.active_version.store(next_version, Ordering::SeqCst);
        self.durable_version.fetch_max(version, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::mem_block_io::MemBlockIo;

    #[test]
    fn open_empty_initializes_slot_a() {
        let (store, meta) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        assert_eq!(meta.root, NULL_PAGE_ID);
        assert_eq!(meta.version, 0);
        assert_eq!(store.active_slot.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn page_roundtrip_through_blockio() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let id = store.allocate();
        store.put(Page::new_leaf(id)).unwrap();
        let page = store.get(id).unwrap().unwrap();
        assert_eq!(page.page_id(), id);
        assert!(page.is_leaf());
    }

    #[test]
    fn get_rejects_corrupted_data_page() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let id = store.allocate();
        store.put(Page::new_leaf(id)).unwrap();
        store.flush().unwrap();
        let bytes = store_bytes(&store);

        // Cold reopen reads page `id` from disk; its stamped checksum verifies.
        let (clean, _) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes.clone()))).unwrap();
        assert!(clean.get(id).unwrap().is_some());

        // Flip a body byte (well past the 28..32 checksum field). The recomputed
        // checksum no longer matches, so the page is rejected as a loud,
        // catchable `Corruption` error rather than decoded (or silently treated
        // as a missing page).
        let mut corrupted = bytes;
        let body_byte = DoubleMetaStore::page_offset(id).unwrap() as usize + 100;
        corrupted[body_byte] ^= 0xFF;
        let (damaged, _) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(corrupted))).unwrap();
        assert!(matches!(
            damaged.get(id),
            Err(PageStoreError::Corruption(corrupt_id)) if corrupt_id == id
        ));
    }

    /// A leaf page too full to serialize: two 3 000-byte inline values.
    fn overfull_leaf(id: PageId) -> Page {
        use crate::btree::page::{LeafEntry, LeafValue};
        use pglet_sql::types::Value;
        let mut page = Page::new_leaf(id);
        if let Page::Leaf { entries, .. } = &mut page {
            let entries = std::sync::Arc::make_mut(entries);
            for row_id in 0..2 {
                entries.push(LeafEntry {
                    key: crate::btree::BtreeKey::new(1, row_id),
                    value: LeafValue::Inline(vec![Value::Text("x".repeat(3000))]),
                });
            }
        }
        page
    }

    /// A commit's pages are refused whole, before any is written: the page that fits,
    /// ahead of the one that does not, is not on disk either.
    #[test]
    fn put_batch_refuses_an_overfull_page_before_writing_any() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let fits = store.allocate();
        let over = store.allocate();
        let len = store.io.len().unwrap();
        assert!(matches!(
            store.put_batch(vec![Page::new_leaf(fits), overfull_leaf(over)]),
            Err(PageStoreError::Overfull { page_id, .. }) if page_id == over
        ));
        assert_eq!(store.io.len().unwrap(), len, "no page may reach the file");
        assert!(store.get(fits).unwrap().is_none());
        store
            .put_batch(vec![Page::new_leaf(fits), Page::new_leaf(over)])
            .unwrap();
        assert!(store.get(over).unwrap().is_some());
    }

    /// A page whose checksum verifies but whose bytes do not decode — damage written
    /// with a valid checksum — is `Corruption`, never a panic.
    #[test]
    fn get_refuses_a_checksum_valid_page_that_does_not_decode() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let leaf = store.allocate();
        let link = store.allocate();
        store.put(Page::new_leaf(leaf)).unwrap();
        store
            .put(Page::new_overflow(link, vec![7; 10], NULL_PAGE_ID))
            .unwrap();
        store.flush().unwrap();
        let mut bytes = store_bytes(&store);
        // The leaf claims five entries it does not carry; the link claims a payload
        // longer than a page's.
        let at = |id: PageId| DoubleMetaStore::page_offset(id).unwrap() as usize;
        bytes[at(leaf) + 10..at(leaf) + 12].copy_from_slice(&5u16.to_le_bytes());
        bytes[at(link) + 10..at(link) + 12].copy_from_slice(&u16::MAX.to_le_bytes());
        for id in [leaf, link] {
            let page: &mut [u8; PAGE_SIZE] =
                (&mut bytes[at(id)..at(id) + PAGE_SIZE]).try_into().unwrap();
            DoubleMetaStore::stamp_page_checksum(page);
        }
        let (damaged, _) = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
        for id in [leaf, link] {
            let got = damaged.get(id);
            assert!(
                matches!(got, Err(PageStoreError::Corruption(c)) if c == id),
                "{id}: {got:?}"
            );
        }
    }

    #[test]
    fn save_state_alternates_slots() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        assert_eq!(store.active_slot.load(Ordering::SeqCst), 0);
        let root = store.allocate();

        store.save_state(root, 1, 2, &[]).unwrap();
        assert_eq!(store.active_slot.load(Ordering::SeqCst), 1);

        store.save_state(root, 2, 2, &[]).unwrap();
        assert_eq!(store.active_slot.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn reopen_recovers_latest_slot() {
        let bytes = {
            let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
            let id = store.allocate();
            store.put(Page::new_leaf(id)).unwrap();
            store.flush().unwrap();
            store.save_state(id, 1, 2, &[]).unwrap(); // root = id, version 1, next_table_id 2
            store_bytes(&store)
        };

        let (store, meta) = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
        assert_eq!(meta.version, 1);
        assert_eq!(meta.root, 0);
        assert_eq!(meta.next_table_id, 2);
        // page survived
        assert!(store.get(0).unwrap().is_some());
    }

    /// Helper to extract the underlying byte image from a store built over a
    /// `MemBlockIo` (test-only; downcast-free by re-reading via the trait).
    fn store_bytes(store: &DoubleMetaStore) -> Vec<u8> {
        let len = store.io.len().unwrap();
        let mut buf = vec![0u8; len as usize];
        store.io.read_at(0, &mut buf).unwrap();
        buf
    }

    #[test]
    fn torn_newer_slot_falls_back_to_older() {
        // Build a valid two-commit image, then corrupt the newer slot.
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let id = store.allocate();
        store.put(Page::new_leaf(id)).unwrap();

        store.flush().unwrap();
        store.save_state(id, 1, 2, &[]).unwrap(); // now in slot B (active, version 1)
        store.save_state(id, 2, 3, &[]).unwrap(); // now in slot A (active, version 2)
        assert_eq!(store.active_slot.load(Ordering::SeqCst), 0);

        let mut bytes = store_bytes(&store);
        // Corrupt slot A's payload region (offset 0). Flip a byte well inside
        // the payload so the CRC fails but the magic still parses.
        bytes[META_HEADER_SIZE + 8] ^= 0xFF;

        let (_store, meta) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
        // Falls back to the older valid slot B (version 1).
        assert_eq!(meta.version, 1);
        assert_eq!(meta.next_table_id, 2);
    }

    #[test]
    fn both_slots_invalid_on_nonempty_errors() {
        let mut bytes = vec![0u8; DATA_REGION_START as usize];
        // Garbage in both slots (no valid magic).
        for b in bytes.iter_mut().take(64) {
            *b = 0xAB;
        }
        let result = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes)));
        assert!(matches!(result, Err(PageStoreError::Io(_))));
    }

    /// An image whose committed tree is one leaf holding key `(1, 1)` with its value
    /// out of line — a chain of one link, page 1, the leaf naming `head` as that link —
    /// committed normally into slot B (version 1, `next_page_id` 2), and slot A then
    /// overwritten with `crafted` at version 99, CRC valid.
    fn image_with_crafted_slot(head: PageId, crafted: &PageStoreMetadata) -> Vec<u8> {
        use crate::btree::BtreeKey;
        use crate::btree::page::{LeafEntry, LeafValue, OverflowRef};
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let leaf = store.allocate();
        let link = store.allocate();
        let bytes =
            postcard::to_allocvec(&vec![pglet_sql::types::Value::Text("chained".into())]).unwrap();
        let mut page = Page::new_leaf(leaf);
        if let Page::Leaf { entries, .. } = &mut page {
            std::sync::Arc::make_mut(entries).push(LeafEntry {
                key: BtreeKey::new(1, 1),
                value: LeafValue::Overflow(OverflowRef {
                    head,
                    len: bytes.len() as u64,
                }),
            });
        }
        store.put(page).unwrap();
        store
            .put(Page::new_overflow(link, bytes, NULL_PAGE_ID))
            .unwrap();
        store.flush().unwrap();
        store.save_state(leaf, 1, 1, &[]).unwrap();
        assert_eq!(store.active_slot.load(Ordering::SeqCst), 1);
        write_slot(&*store.io, 0, 99, crafted, &[]).unwrap();
        store_bytes(&store)
    }

    /// The well-formed slot [`image_with_crafted_slot`] commits.
    const WELL_FORMED: PageStoreMetadata = PageStoreMetadata {
        next_page_id: 2,
        root: 0,
        version: 1,
        next_table_id: 1,
        freelist_head: NULL_PAGE_ID,
        freelist_crc: 0,
        retired_head: NULL_PAGE_ID,
        retired_crc: 0,
        retired_max: 0,
        retired_floor: 0,
    };

    /// A meta slot whose CRC is valid but whose page ids leave what the store
    /// addresses or has allocated — `next_page_id` past [`PAGE_ID_LIMIT`], a `root` or
    /// a `freelist_head` at or past `next_page_id` — is refused as a torn slot is:
    /// open reads the other slot when that one is valid, and refuses when it is not.
    #[test]
    fn a_slot_naming_a_page_id_the_store_cannot_hold_is_refused() {
        let crafted = [
            (
                "next_page_id one past the limit",
                PageStoreMetadata {
                    next_page_id: PAGE_ID_LIMIT + 1,
                    ..WELL_FORMED
                },
            ),
            (
                "next_page_id at the null link",
                PageStoreMetadata {
                    next_page_id: u64::MAX,
                    ..WELL_FORMED
                },
            ),
            (
                "a root never allocated",
                PageStoreMetadata {
                    root: WELL_FORMED.next_page_id,
                    ..WELL_FORMED
                },
            ),
            (
                "a free-list head never allocated",
                PageStoreMetadata {
                    freelist_head: WELL_FORMED.next_page_id,
                    ..WELL_FORMED
                },
            ),
        ];
        for (name, meta) in &crafted {
            let bytes = image_with_crafted_slot(1, meta);
            let (_, got) =
                DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes.clone()))).unwrap();
            assert_eq!(
                (got.version, got.next_page_id, got.root, got.freelist_head),
                (
                    WELL_FORMED.version,
                    WELL_FORMED.next_page_id,
                    WELL_FORMED.root,
                    WELL_FORMED.freelist_head
                ),
                "{name}: open did not fall back to the well-formed slot"
            );

            // The same slot in both places: nothing valid to fall back to.
            let io = MemBlockIo::from_bytes(bytes);
            write_slot(&io, 1, 100, meta, &[]).unwrap();
            let result = DoubleMetaStore::open(Box::new(io));
            assert!(
                matches!(result, Err(PageStoreError::Io(_))),
                "{name}: {:?}",
                result.map(|(_, m)| m)
            );
        }
    }

    /// The limit itself is a `next_page_id` a slot may carry: the store opens, and the
    /// allocation it then hands out — the first id past the limit — is refused when
    /// written rather than placed past the addressable file.
    #[test]
    fn a_slot_at_the_page_id_limit_opens_and_writes_nothing_past_it() {
        let at_limit = PageStoreMetadata {
            next_page_id: PAGE_ID_LIMIT,
            ..WELL_FORMED
        };
        let io = MemBlockIo::from_bytes(image_with_crafted_slot(1, &at_limit));
        write_slot(&io, 1, 100, &at_limit, &[]).unwrap();
        let (store, meta) = DoubleMetaStore::open(Box::new(io)).unwrap();
        assert_eq!(meta.next_page_id, PAGE_ID_LIMIT);
        let id = store.allocate();
        assert_eq!(id, PAGE_ID_LIMIT);
        let put = store.put(Page::new_leaf(id));
        assert!(matches!(put, Err(PageStoreError::Io(_))), "{put:?}");
        assert!(matches!(
            store.get(id),
            Err(PageStoreError::Corruption(c)) if c == id
        ));
        // Below the limit and past the file is an absent page, not an error.
        assert!(matches!(store.get(PAGE_ID_LIMIT - 1), Ok(None)));
    }

    /// A commit after an allocation has reached the limit is refused rather than written:
    /// its `next_page_id` is past the limit, a slot open refuses, so a reopen would fall
    /// back to the other slot and lose the commit without a word.
    #[test]
    fn a_commit_past_the_page_id_limit_is_refused_not_written() {
        let at_limit = PageStoreMetadata {
            next_page_id: PAGE_ID_LIMIT,
            ..WELL_FORMED
        };
        let io = MemBlockIo::from_bytes(image_with_crafted_slot(1, &at_limit));
        write_slot(&io, 1, 100, &at_limit, &[]).unwrap();
        let (store, meta) = DoubleMetaStore::open(Box::new(io)).unwrap();
        assert_eq!(store.allocate(), PAGE_ID_LIMIT);
        let before = store_bytes(&store);
        let saved = store.save_state(meta.root, meta.version + 1, meta.next_table_id, &[]);
        assert!(
            matches!(saved, Err(PageStoreError::WriteFailed(_))),
            "{saved:?}"
        );
        assert_eq!(store_bytes(&store), before, "no slot byte moved");
        // The store is failed: the tree's published version cannot be made durable.
        let again = store.save_state(meta.root, meta.version + 1, meta.next_table_id, &[]);
        assert!(
            matches!(again, Err(PageStoreError::StoreFailed(_))),
            "{again:?}"
        );
        // And reads too, for a reader no host gate stands in front of.
        let read = store.get(meta.root);
        assert!(
            matches!(read, Err(PageStoreError::StoreFailed(_))),
            "{read:?}"
        );
        let (_, reopened) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(store_bytes(&store)))).unwrap();
        assert_eq!(reopened.version, meta.version);
    }

    /// No id reaches the page-offset multiply unchecked: a read of an id past every
    /// page the store addresses — one that would wrap onto page 1's bytes, the top of
    /// the id space, the limit itself — is `Corruption` naming it, never a panic nor
    /// another page's bytes, and a write of one is refused before a byte moves.
    #[test]
    fn a_page_id_past_the_store_is_refused_not_wrapped() {
        let bytes = image_with_crafted_slot(1, &WELL_FORMED);
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
        assert!(store.get(1).unwrap().is_some());
        for id in [PAGE_ID_LIMIT, 1 + (1 << 52), NULL_PAGE_ID - 1, NULL_PAGE_ID] {
            let got = store.get(id);
            assert!(
                matches!(got, Err(PageStoreError::Corruption(c)) if c == id),
                "{id}: {got:?}"
            );
        }
        let put = store.put(Page::new_leaf(NULL_PAGE_ID - 1));
        assert!(matches!(put, Err(PageStoreError::Io(_))), "{put:?}");
        assert!(store.get(1).unwrap().is_some());
    }

    /// A crafted slot claiming every id is allocated (`next_page_id` at the null link,
    /// CRC valid, the newer version) is refused at open and the well-formed slot read, so a
    /// chain link past the page-offset domain is a corrupt chain and an insert reads
    /// back beside the committed row. Read as the store's word, that slot passes such a
    /// link through the walk's allocated-id check — with `page_offset` unchecked too, a
    /// multiply overflow in a debug build — and hands out the null link as a fresh
    /// allocation.
    #[test]
    fn a_crafted_slot_cannot_alias_a_chain_link_or_wrap_an_allocation() {
        use crate::btree::BtreeKey;
        use crate::btree::cow::CowBtree;
        use pglet_sql::types::Value;
        let crafted = PageStoreMetadata {
            next_page_id: u64::MAX,
            ..WELL_FORMED
        };
        let open_tree = |head: PageId| {
            let bytes = image_with_crafted_slot(head, &crafted);
            let (store, meta) =
                DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
            assert_eq!(meta.next_page_id, WELL_FORMED.next_page_id);
            CowBtree::with_store(Box::new(store), meta.root, meta.version, 100)
        };
        for link in [1 + (1 << 52), NULL_PAGE_ID - 1] {
            let got = open_tree(link).get(&BtreeKey::new(1, 1));
            assert!(
                matches!(
                    got,
                    Err(crate::btree::CowBtreeError::Storage(PageStoreError::CorruptChain { head, page }))
                        if head == link && page == link
                ),
                "{link}: {got:?}"
            );
        }

        let tree = open_tree(1);
        tree.insert(BtreeKey::new(1, 2), vec![Value::Int4(7)])
            .unwrap();
        assert_eq!(
            tree.get(&BtreeKey::new(1, 1)).unwrap(),
            Some(vec![Value::Text("chained".into())])
        );
        assert_eq!(
            tree.get(&BtreeKey::new(1, 2)).unwrap(),
            Some(vec![Value::Int4(7)])
        );
    }

    /// A free-list page that lists an id at or past `next_page_id` — an id no page was
    /// ever allocated under — ends the open-time walk as a torn page does, so the id is
    /// never handed out (and never handed out a second time when the store's fresh
    /// allocations reach it).
    #[test]
    fn a_free_list_page_naming_an_unallocated_id_is_not_recovered() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let ids: Vec<PageId> = (0..3).map(|_| store.allocate()).collect();
        for &id in &ids {
            store.put(Page::new_leaf(id)).unwrap();
        }
        store.free_pages(vec![ids[2]]);
        store.flush().unwrap();
        store.save_state(ids[0], 1, 1, &[]).unwrap();
        let bytes = store_bytes(&store);
        let (clean, meta) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes.clone()))).unwrap();
        assert_eq!(
            clean.free_list(),
            vec![ids[2]],
            "the control recovers the id"
        );

        // The page rewritten to list an id never allocated, and the slot re-recording
        // its checksum, so what refuses it is the listed-id check and not the chain's.
        let io = MemBlockIo::from_bytes(bytes);
        let (next, next_crc, _, _) = read_freelist_page(&io, meta.freelist_head)
            .unwrap()
            .unwrap();
        let crc = write_freelist_page(
            &io,
            meta.freelist_head,
            next,
            next_crc,
            &[meta.next_page_id + 5],
        )
        .unwrap();
        let recrafted = PageStoreMetadata {
            freelist_crc: crc,
            ..meta
        };
        write_slot(&io, 1, 1, &recrafted, &[]).unwrap();
        let (store, reopened) = DoubleMetaStore::open(Box::new(io)).unwrap();
        assert_eq!(reopened.freelist_crc, crc, "open read the re-recorded slot");
        assert!(store.free_list().is_empty(), "{:?}", store.free_list());
    }

    /// Power lost with the slot landing over a free-list chain page that did not: the
    /// page still holds the list from two commits back, whose ids a commit between has
    /// handed out again. Whether the stale page is the chain's head or its tail, open
    /// recovers none of those ids — the stale page fails the checksum its predecessor
    /// (or the slot) records for it, and the walk stops there.
    #[test]
    fn a_slot_landing_over_a_stale_chain_page_recovers_no_live_id() {
        let per = FREELIST_IDS_PER_PAGE as u64;
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        // v1: a two-page free list, into slot B's chain.
        let first: Vec<PageId> = (0..per + 40).map(|_| store.allocate()).collect();
        store.free_pages(first.clone());
        store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
        // v2: every one of those ids handed out again, live from here on; slot A.
        for _ in &first {
            store.allocate();
        }
        assert!(store.free_list().is_empty());
        store.save_state(NULL_PAGE_ID, 2, 1, &[]).unwrap();
        // v3: another two-page list, written over slot B's chain pages.
        let second: Vec<PageId> = (0..per + 40).map(|_| store.allocate()).collect();
        store.free_pages(second.clone());
        let before = store_bytes(&store);
        store.save_state(NULL_PAGE_ID, 3, 1, &[]).unwrap();
        let after = store_bytes(&store);
        let [chain, _] = store.persisted_chain_pages();
        assert_eq!(chain.len(), 2, "a head and a tail");

        for (stale, name, recovered) in [(0, "head", 0), (1, "tail", per as usize)] {
            let at = DoubleMetaStore::page_offset(chain[stale]).unwrap() as usize;
            assert_ne!(before[at..at + PAGE_SIZE], after[at..at + PAGE_SIZE]);
            let mut image = after.clone();
            image[at..at + PAGE_SIZE].copy_from_slice(&before[at..at + PAGE_SIZE]);
            let (reopened, meta) =
                DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image))).unwrap();
            assert_eq!(meta.version, 3, "{name}: open adopts the slot that landed");
            let free = reopened.free_list();
            let live: Vec<&PageId> = free.iter().filter(|id| first.contains(id)).collect();
            assert!(live.is_empty(), "stale {name}: live ids recovered {live:?}");
            assert!(
                free.iter().all(|id| second.contains(id)),
                "stale {name}: {free:?}"
            );
            assert_eq!(free.len(), recovered, "stale {name}: the prefix before it");
        }
    }

    /// A crash inside creation — the length landed, slot A's write did not — leaves a
    /// file of zeros no longer than the two slots, and it opens as a new store that
    /// takes a commit.
    #[test]
    fn a_creation_cut_short_opens_as_a_new_store() {
        for len in [DATA_REGION_START as usize, 4096, 1] {
            let io = std::sync::Arc::new(MemBlockIo::from_bytes(vec![0; len]));
            let (store, meta) =
                DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(&io)))).unwrap();
            assert_eq!(
                (meta.version, meta.root, meta.next_page_id),
                (0, NULL_PAGE_ID, 0),
                "{len}"
            );
            let id = store.allocate();
            store.put(Page::new_leaf(id)).unwrap();
            store.flush().unwrap();
            store.save_state(id, 1, 2, &[]).unwrap();
            let (_, reopened) = DoubleMetaStore::open(Box::new(SharedIo(io))).unwrap();
            assert_eq!((reopened.version, reopened.root), (1, id), "{len}");
        }
    }

    /// The other side of the creation rule: a store that has held a data page and lost
    /// both slots is refused, never opened empty — even when the slots are zeros, the
    /// bytes creation leaves.
    #[test]
    fn a_store_that_held_a_page_is_refused_when_both_slots_are_lost() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let id = store.allocate();
        store.put(Page::new_leaf(id)).unwrap();
        store.flush().unwrap();
        store.save_state(id, 1, 2, &[]).unwrap();
        let mut bytes = store_bytes(&store);
        bytes[..DATA_REGION_START as usize].fill(0);
        let result = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes)));
        assert!(
            matches!(result, Err(PageStoreError::Io(ref m)) if m.contains("both meta slots")),
            "{:?}",
            result.map(|(_, m)| m)
        );
    }

    /// A recovered free list hands each id out once: an authentic chain page listing
    /// an id twice recovers it once, and one listing the chain page itself does not
    /// recover that page — each damage alone, since either is enough to act on.
    #[test]
    fn a_recovered_free_list_hands_each_id_out_once() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let ids: Vec<PageId> = (0..6).map(|_| store.allocate()).collect();
        store.free_pages(vec![ids[3], ids[4]]);
        store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
        let (_, meta) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(store_bytes(&store)))).unwrap();
        let head = meta.freelist_head;
        for (damage, listed) in [
            ("a repeated id", vec![ids[3], ids[3], ids[4]]),
            ("the chain page's own id", vec![ids[3], ids[4], head]),
        ] {
            let io = MemBlockIo::from_bytes(store_bytes(&store));
            let crc = write_freelist_page(&io, head, NULL_PAGE_ID, 0, &listed).unwrap();
            write_slot(
                &io,
                1,
                1,
                &PageStoreMetadata {
                    freelist_crc: crc,
                    ..meta.clone()
                },
                &[],
            )
            .unwrap();
            let (reopened, _) = DoubleMetaStore::open(Box::new(io)).unwrap();
            let mut free = reopened.free_list();
            free.sort_unstable();
            assert_eq!(free, vec![ids[3], ids[4]], "{damage}");
        }
    }

    /// Every tree page `root` reaches in `store`.
    fn tree_pages(store: &dyn PageStore, root: PageId) -> HashSet<PageId> {
        let mut seen = HashSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if id == NULL_PAGE_ID || !seen.insert(id) {
                continue;
            }
            if let Some(Page::Internal {
                first_child,
                entries,
                ..
            }) = store.get(id).unwrap()
            {
                stack.push(first_child);
                stack.extend(entries.iter().map(|e| e.child));
            }
        }
        seen
    }

    /// A tree over a double-meta store on `io`, auto-collection off.
    fn tree_on(io: &std::sync::Arc<MemBlockIo>) -> crate::btree::cow::CowBtree {
        let (store, meta) =
            DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(io)))).unwrap();
        crate::btree::cow::CowBtree::with_store(
            Box::new(store),
            meta.root,
            meta.version,
            usize::MAX,
        )
    }

    /// Commit a batch writing `rows` rows of `width`-character values, and save it
    /// when `save` is set.
    fn write_rows(tree: &crate::btree::cow::CowBtree, rows: u64, width: usize, save: bool) {
        use crate::btree::BtreeKey;
        use crate::btree::cow::BatchChanges;
        use pglet_sql::types::Value;
        let mut batch = BatchChanges::new();
        for r in 0..rows {
            batch.insert(BtreeKey::new(1, r), vec![Value::Text("v".repeat(width))]);
        }
        tree.apply_batch(batch).unwrap();
        if save {
            tree.save_state(1).unwrap();
        }
    }

    /// A `BlockIo` over a shared buffer that refuses data-region writes (`writes`) or
    /// `fsync`s (`syncs`) while its switch is on, as a full disk refuses them (`ENOSPC`).
    struct RefusingIo {
        inner: std::sync::Arc<MemBlockIo>,
        writes: std::sync::Arc<std::sync::atomic::AtomicBool>,
        syncs: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl BlockIo for RefusingIo {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read_at(offset, buf)
        }
        fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
            if offset >= DATA_REGION_START && self.writes.load(Ordering::SeqCst) {
                return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "full"));
            }
            self.inner.write_at(offset, buf)
        }
        fn sync(&self) -> std::io::Result<()> {
            if self.syncs.load(Ordering::SeqCst) {
                return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "full"));
            }
            self.inner.sync()
        }
        fn len(&self) -> std::io::Result<u64> {
            self.inner.len()
        }
        fn set_len(&self, size: u64) -> std::io::Result<()> {
            self.inner.set_len(size)
        }
    }

    /// A refused page write or `fsync` is dropped with its batch — the store serving on —
    /// exactly when the batch is declared droppable (`apply_discardable_batch`), whatever
    /// else the tree has published past the durable version (another connection's open
    /// block, say): that version stays published and the next save makes it durable, and
    /// the dropped batch is in neither — each for a refused page write and for a refused
    /// `fsync`, the device's being full carried with the refusal. From a batch not
    /// declared droppable and from a bare `flush`, with writes and syncs both refused, it
    /// fails the store.
    #[test]
    fn a_refused_write_is_dropped_exactly_from_a_discardable_batch() {
        use crate::btree::BtreeKey;
        use crate::btree::cow::{BatchChanges, CowBtree, CowBtreeError};
        use pglet_sql::types::Value;
        use std::sync::atomic::AtomicBool;
        let batch = |row: u64| {
            let mut batch = BatchChanges::new();
            batch.insert(BtreeKey::new(1, row), vec![Value::Text("w".repeat(300))]);
            batch
        };
        for refusing in ["page write", "fsync"] {
            let io = std::sync::Arc::new(MemBlockIo::new());
            let (writes, syncs) = (
                std::sync::Arc::new(AtomicBool::new(false)),
                std::sync::Arc::new(AtomicBool::new(false)),
            );
            let switch = if refusing == "fsync" { &syncs } else { &writes };
            let (store, meta) = DoubleMetaStore::open(Box::new(RefusingIo {
                inner: std::sync::Arc::clone(&io),
                writes: std::sync::Arc::clone(&writes),
                syncs: std::sync::Arc::clone(&syncs),
            }))
            .unwrap();
            let tree = CowBtree::with_store(Box::new(store), meta.root, meta.version, usize::MAX);
            write_rows(&tree, 20, 40, true);
            let failed = |tree: &CowBtree| tree.store().write_failure().unwrap().get().is_some();

            // Nothing published past the durable version: dropped, and the store serves.
            let version = tree.current_version();
            switch.store(true, Ordering::SeqCst);
            let refused = tree.apply_discardable_batch(batch(100)).unwrap_err();
            switch.store(false, Ordering::SeqCst);
            assert!(
                matches!(
                    &refused,
                    CowBtreeError::Storage(PageStoreError::WriteRefused(DeviceError {
                        disk_full: true,
                        ..
                    }))
                ),
                "{refusing}: {refused:?}"
            );
            assert!(!failed(&tree), "{refusing}: the store serves on");
            assert_eq!(
                tree.current_version(),
                version,
                "{refusing}: nothing published"
            );
            tree.apply_discardable_batch(batch(100)).unwrap();
            tree.save_state(1).unwrap();

            // A version published and not yet durable that is not the refused batch's: the
            // refusal drops the batch alone, and the store serves on.
            tree.apply_batch(batch(101)).unwrap();
            let published = tree.current_version();
            switch.store(true, Ordering::SeqCst);
            let refused = tree.apply_discardable_batch(batch(102)).unwrap_err();
            switch.store(false, Ordering::SeqCst);
            assert!(
                matches!(
                    &refused,
                    CowBtreeError::Storage(PageStoreError::WriteRefused(DeviceError {
                        disk_full: true,
                        ..
                    }))
                ),
                "{refusing}, past a published version: {refused:?}"
            );
            assert!(
                !failed(&tree),
                "{refusing}, past a published version: the store serves on"
            );
            assert_eq!(
                tree.current_version(),
                published,
                "{refusing}, past a published version: it stays published"
            );
            tree.save_state(1).unwrap();
            let len = io.len().unwrap();
            let mut image = vec![0u8; usize::try_from(len).unwrap()];
            io.read_at(0, &mut image).unwrap();
            let (reopened, meta) =
                DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image))).unwrap();
            let reopened =
                CowBtree::with_store(Box::new(reopened), meta.root, meta.version, usize::MAX);
            let held = |row: u64| reopened.get(&BtreeKey::new(1, row)).unwrap().is_some();
            assert!(
                held(101) && !held(102),
                "{refusing}, past a published version: the save holds the published batch and not the dropped one"
            );
        }

        // A batch not declared droppable, and a bare flush, fail the store with nothing
        // published.
        for how in ["batch", "flush"] {
            let io = std::sync::Arc::new(MemBlockIo::new());
            let (writes, syncs) = (
                std::sync::Arc::new(AtomicBool::new(false)),
                std::sync::Arc::new(AtomicBool::new(false)),
            );
            let (store, meta) = DoubleMetaStore::open(Box::new(RefusingIo {
                inner: std::sync::Arc::clone(&io),
                writes: std::sync::Arc::clone(&writes),
                syncs: std::sync::Arc::clone(&syncs),
            }))
            .unwrap();
            let tree = CowBtree::with_store(Box::new(store), meta.root, meta.version, usize::MAX);
            write_rows(&tree, 20, 40, true);
            writes.store(true, Ordering::SeqCst);
            syncs.store(true, Ordering::SeqCst);
            let refused = if how == "batch" {
                tree.apply_batch(batch(100)).unwrap_err()
            } else {
                tree.flush().unwrap_err()
            };
            assert!(
                matches!(
                    &refused,
                    CowBtreeError::Storage(PageStoreError::WriteFailed(_))
                ),
                "{how}: {refused:?}"
            );
            assert!(
                tree.store().write_failure().unwrap().get().is_some(),
                "{how}: the store fails"
            );
        }
    }

    /// Collection frees no page the durable version reaches, whatever the snapshots
    /// say: versions published in memory since the last save are not what a crash
    /// reopens. Once a newer version is durable, the pages are free.
    #[test]
    fn collection_frees_no_page_the_durable_version_reaches() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        write_rows(&tree, 200, 40, true);
        let durable = tree_pages(tree.store(), tree.current_root());
        // Two in-memory versions past the durable one, and a vacuum with no snapshot.
        write_rows(&tree, 200, 50, false);
        write_rows(&tree, 200, 60, false);
        tree.vacuum();
        let free = tree.store().free_list();
        let overlap: Vec<&PageId> = free.iter().filter(|id| durable.contains(id)).collect();
        assert!(
            overlap.is_empty(),
            "durable pages on the free list: {overlap:?}"
        );

        tree.save_state(1).unwrap();
        tree.vacuum();
        let free = tree.store().free_list();
        assert!(
            durable.iter().all(|id| free.contains(id)),
            "once a newer version is durable, the older one's pages are free"
        );
    }

    /// The pin is the durable slot's **tree** version, not the slot counter: the two
    /// move apart — a save with no batch (`Database::sync`) moves only the slot counter,
    /// several batches before one save (a DDL statement) move only the tree version —
    /// and a pin keyed on the wrong one frees a durable page when the slot counter runs
    /// ahead, and never frees an older durable version when it runs behind. Each half is
    /// checked both ways — no durable page on the free list while its version is the
    /// durable one, every one of them there once a newer version is durable — after a
    /// reopen too, which seeds the pin from the adopted slot.
    #[test]
    fn the_pin_is_the_tree_version_not_the_slot_counter() {
        let check = |tree: &crate::btree::cow::CowBtree, label: &str| {
            let durable = tree_pages(tree.store(), tree.current_root());
            write_rows(tree, 200, 50, false);
            write_rows(tree, 200, 60, false);
            tree.vacuum();
            let free: HashSet<PageId> = tree.store().free_list().into_iter().collect();
            let held: Vec<&PageId> = durable.iter().filter(|id| free.contains(id)).collect();
            assert!(
                held.is_empty(),
                "{label}: durable pages on the free list: {held:?}"
            );
            tree.save_state(1).unwrap();
            tree.vacuum();
            let free: HashSet<PageId> = tree.store().free_list().into_iter().collect();
            let kept: Vec<&PageId> = durable.iter().filter(|id| !free.contains(id)).collect();
            assert!(
                kept.is_empty(),
                "{label}: pages of an older durable version not freed: {kept:?}"
            );
        };
        // The slot counter ahead of the tree version: twenty saves with no batch.
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        for _ in 0..20 {
            tree.save_state(1).unwrap();
        }
        write_rows(&tree, 200, 40, true);
        check(&tree, "slot counter ahead");
        drop(tree);
        let reopened = tree_on(&io);
        write_rows(&reopened, 200, 45, true);
        check(&reopened, "slot counter ahead, reopened");

        // The slot counter behind: three batches to every save.
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        for width in [40, 41, 42, 43, 44, 45] {
            write_rows(&tree, 200, width, false);
            write_rows(&tree, 200, width + 10, false);
            write_rows(&tree, 200, width + 20, true);
        }
        check(&tree, "slot counter behind");
        drop(tree);
        let reopened = tree_on(&io);
        write_rows(&reopened, 200, 47, false);
        write_rows(&reopened, 200, 48, true);
        check(&reopened, "slot counter behind, reopened");
    }

    /// Every page below `next_page_id` of the store `io` holds is live in its tree, free,
    /// or a chain page one of its slots names, after a reopen; the reopened store's
    /// metadata comes back for the caller's own checks.
    fn assert_every_page_accounted(
        io: &std::sync::Arc<MemBlockIo>,
        label: &str,
    ) -> PageStoreMetadata {
        let (reopened, meta) =
            DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(io)))).unwrap();
        let live = tree_pages(&reopened, meta.root);
        let free: HashSet<PageId> = reopened.free_list().into_iter().collect();
        let chains: HashSet<PageId> = reopened
            .persisted_chain_pages()
            .concat()
            .into_iter()
            .collect();
        let leaked: Vec<PageId> = (0..meta.next_page_id)
            .filter(|id| !live.contains(id) && !free.contains(id) && !chains.contains(id))
            .collect();
        assert!(leaked.is_empty(), "{label}: leaked {leaked:?}");
        let aliased: Vec<&PageId> = free.iter().filter(|id| live.contains(id)).collect();
        assert!(
            aliased.is_empty(),
            "{label}: live pages on the free list {aliased:?}"
        );
        // A page a slot's chain or retired walk still reads is not free either: handed
        // out, it is written over while that slot can still be adopted, and a later
        // release puts it on the list a second time.
        let named: Vec<&PageId> = free.iter().filter(|id| chains.contains(id)).collect();
        assert!(
            named.is_empty(),
            "{label}: pages a slot's chain or retired walk reads are on the free list {named:?}"
        );
        meta
    }

    /// A snapshot held across many saves makes the retired log outgrow what a slot
    /// carries, so saves spill it into chain pages. A store closed then — no snapshot
    /// survives it — takes every spilled entry as free and leaks nothing; and once the
    /// snapshot is gone and both slots are rewritten past it, the spilled pages are
    /// released rather than kept, and still nothing leaks.
    #[test]
    fn a_spilled_retired_log_is_read_at_reopen_and_released_after() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        write_rows(&tree, 200, 400, true);
        let held = tree.snapshot();
        for width in 401..420 {
            write_rows(&tree, 200, width, true);
        }
        let spilled = assert_every_page_accounted(&io, "held snapshot, closed");
        assert_ne!(
            spilled.retired_head, NULL_PAGE_ID,
            "the witness: the log spilled"
        );

        tree.release_snapshot(&held);
        write_rows(&tree, 200, 470, true);
        write_rows(&tree, 200, 471, true);
        tree.vacuum();
        // Two saves past the collection, one into each slot.
        write_rows(&tree, 200, 472, true);
        write_rows(&tree, 200, 473, true);
        drop(tree);
        assert_every_page_accounted(&io, "snapshot released, closed");
        let (reopened, _) =
            DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(&io)))).unwrap();
        // Both slots' walks, the alternate's included (read by the audit call).
        let _ = reopened.persisted_chain_pages();
        let named = reopened.retired.lock().pages.len();
        assert_eq!(named, 0, "the spilled pages are released, not kept named");
    }

    /// The slot the store on `io` adopts, read as open reads it: the valid slot with the
    /// higher version.
    fn adopted_slot(io: &dyn BlockIo) -> MetaSlot {
        match (read_slot(io, 0).unwrap(), read_slot(io, 1).unwrap()) {
            (Some(a), Some(b)) => {
                if b.version >= a.version {
                    b
                } else {
                    a
                }
            }
            (Some(slot), None) | (None, Some(slot)) => slot,
            (None, None) => panic!("no valid slot"),
        }
    }

    /// Every byte of the store on `io`.
    fn image_of(io: &MemBlockIo) -> Vec<u8> {
        let mut buf = vec![0u8; usize::try_from(io.len().unwrap()).unwrap()];
        io.read_at(0, &mut buf).unwrap();
        buf
    }

    /// A slot's retired log is authenticated link by link, as its free-list chain is:
    /// a checksum-valid retired page the slot did not record — another commit's page
    /// left where a spill write did not land, or a crafted one — is not trusted, so
    /// open takes none of its entries as free. Checked at the chain's head (the checksum
    /// the slot records) and at its second page (the one the head records), each
    /// crafted to list live pages under the largest entry its predecessor records, so
    /// the recorded-max check alone does not stop it; a walk that checks only the head,
    /// or only the max, or skips the check when open reads the log, puts live pages on
    /// the free list.
    #[test]
    fn a_retired_page_from_another_commit_recovers_no_live_id() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        write_rows(&tree, 200, 400, true);
        let held = tree.snapshot();
        for width in 401..440 {
            write_rows(&tree, 200, width, true);
        }
        drop(tree);
        let image = image_of(&io);
        let slot = adopted_slot(&*io);
        let meta = slot.metadata;
        let (head, head_crc, head_max) = (meta.retired_head, meta.retired_crc, meta.retired_max);
        assert_ne!(head, NULL_PAGE_ID, "the witness: the log spilled");
        let (body, _) = read_chain_page(&*io, head).unwrap().expect("the head page");
        let (second, second_crc, second_max, _) = decode_retired_body(&body).unwrap();
        assert_ne!(second, NULL_PAGE_ID, "the witness: the log spans two pages");
        let (body, _) = read_chain_page(&*io, second)
            .unwrap()
            .expect("the second page");
        let (third, third_crc, third_max, _) = decode_retired_body(&body).unwrap();
        let (pristine, _) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image.clone()))).unwrap();
        let live: Vec<PageId> = tree_pages(&pristine, meta.root)
            .into_iter()
            .take(20)
            .collect();
        assert!(
            pristine.free_list().iter().all(|id| !live.contains(id)),
            "the pristine image lists no live page"
        );
        drop(pristine);

        for (at, max, (next, next_crc, next_max), recorded, link) in [
            (
                head,
                head_max,
                (second, second_crc, second_max),
                head_crc,
                "head",
            ),
            (
                second,
                second_max,
                (third, third_crc, third_max),
                second_crc,
                "second page",
            ),
        ] {
            let crafted = MemBlockIo::from_bytes(image.clone());
            let entries: Vec<RetiredEntry> = live.iter().map(|&id| (id, max)).collect();
            let crc = write_retired_page(&crafted, at, next, next_crc, next_max, &entries).unwrap();
            assert_ne!(
                crc, recorded,
                "{link}: the crafted page is not the one recorded"
            );
            let (store, _) = DoubleMetaStore::open(Box::new(crafted)).unwrap();
            let listed: Vec<PageId> = store
                .free_list()
                .into_iter()
                .filter(|id| live.contains(id))
                .collect();
            assert!(
                listed.is_empty(),
                "{link}: live pages on the free list {listed:?}"
            );
        }
        let _ = held;
    }

    /// A spilled retired-log page is released only once **both** slots' walks pass it,
    /// not once the slot being written does: the older slot is still the one a torn save
    /// falls back to, and open lists the new slot's free pages without reading the older
    /// slot's log. Here a snapshot held across a spill is released and exactly one save
    /// follows — so the new slot's floor has passed the spilled pages and the older
    /// slot's has not — then the store closes: every page is accounted for, and none the
    /// older slot's walk reads is on the free list (released early, it is handed out
    /// while that walk still reads it, and a later release lists it a second time).
    #[test]
    fn a_page_the_older_slots_retired_walk_reads_is_not_released() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        write_rows(&tree, 200, 400, true);
        let held = tree.snapshot();
        for width in 401..420 {
            write_rows(&tree, 200, width, true);
        }
        tree.release_snapshot(&held);
        tree.vacuum();
        write_rows(&tree, 200, 470, true);
        drop(tree);
        let adopted = adopted_slot(&*io);
        let older = [0, 1]
            .into_iter()
            .filter_map(|slot| read_slot(&*io, slot).unwrap())
            .find(|slot| slot.version < adopted.version)
            .expect("the older slot is valid");
        assert!(
            older.metadata.retired_head != NULL_PAGE_ID
                && older.metadata.retired_floor < adopted.metadata.retired_floor,
            "the witness: the older slot's walk still reads spilled pages the new floor passed"
        );
        assert_every_page_accounted(&io, "one save after the release");
    }

    /// A retired-log page can hold entries on both sides of a later floor: collection
    /// frees the older ones while the page is still read for the newer. Those older
    /// pages are handed out again and live in the saved tree, so a reopen must take
    /// only the entries at or past the floor — here, after two snapshots held in
    /// turn, a close leaves no live page on the free list and none leaked.
    #[test]
    fn a_retired_log_entry_below_its_floor_is_not_taken_free() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        write_rows(&tree, 200, 400, true);
        let first = tree.snapshot();
        for width in 401..410 {
            write_rows(&tree, 200, width, true);
        }
        let second = tree.snapshot();
        for width in 410..420 {
            write_rows(&tree, 200, width, true);
        }
        // Free everything below the second snapshot, and hand those pages out again.
        tree.release_snapshot(&first);
        tree.vacuum();
        for width in 420..424 {
            write_rows(&tree, 200, width, true);
        }
        let meta = assert_every_page_accounted(&io, "closed with the second snapshot held");
        assert_ne!(
            meta.retired_head, NULL_PAGE_ID,
            "the witness: the log spilled"
        );
        tree.release_snapshot(&second);
    }

    /// A page superseded but not yet collected — here, never: auto-collection is off —
    /// is on the free list a reopen recovers, so a close does not leak it; the page
    /// the saved version reaches is not.
    #[test]
    fn a_superseded_page_is_free_after_a_reopen() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let tree = tree_on(&io);
        write_rows(&tree, 200, 40, true);
        let first = tree_pages(tree.store(), tree.current_root());
        write_rows(&tree, 200, 50, true);
        let second = tree_pages(tree.store(), tree.current_root());
        assert!(tree.store().free_list().is_empty(), "nothing was collected");
        drop(tree);
        let (reopened, meta) = DoubleMetaStore::open(Box::new(SharedIo(io))).unwrap();
        let free: HashSet<PageId> = reopened.free_list().into_iter().collect();
        assert!(
            first.iter().all(|id| free.contains(id)),
            "the superseded pages"
        );
        assert!(second.iter().all(|id| !free.contains(id)), "the live pages");
        let chains = reopened.persisted_chain_pages().concat();
        assert_eq!(
            second.len() + free.len() + chains.len(),
            meta.next_page_id as usize,
            "every allocated page is live, free or a chain page"
        );
    }

    /// A shared handle onto one [`MemBlockIo`], so a test can read the image of a store
    /// a tree owns.
    struct SharedIo(std::sync::Arc<MemBlockIo>);

    impl BlockIo for SharedIo {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read_at(offset, buf)
        }
        fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
            self.0.write_at(offset, buf)
        }
        fn sync(&self) -> std::io::Result<()> {
            self.0.sync()
        }
        fn len(&self) -> std::io::Result<u64> {
            self.0.len()
        }
        fn set_len(&self, size: u64) -> std::io::Result<()> {
            self.0.set_len(size)
        }
    }

    /// A well-formed store's `next_page_id` can pass the end of its file: a batch can
    /// allocate an id it never writes (a chain it writes and drops again in the same
    /// batch is never written, and `discard_created` hands the id straight back), so a
    /// slot can record ids past every page the file holds. So open bounds the slot by
    /// [`PAGE_ID_LIMIT`] and not by the file's length — this store reopens. The store
    /// calls are the ones such a batch makes: a reused id written, a fresh one handed
    /// out and handed back unwritten, and a save whose free-list chain reuses the page
    /// its slot held, so nothing past the fresh id is written either.
    #[test]
    fn a_well_formed_store_can_allocate_past_its_file_and_reopens() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let (store, _) =
            DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(&io)))).unwrap();
        // A page written and freed, saved into both slots: each slot's chain holds a page.
        let page = store.allocate();
        store.put(Page::new_leaf(page)).unwrap();
        store.free_pages(vec![page]);
        store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
        store.save_state(NULL_PAGE_ID, 2, 1, &[]).unwrap();
        // The batch: the free page reused and written, a fresh one dropped unwritten.
        let reused = store.allocate();
        assert_eq!(reused, page);
        store.put(Page::new_leaf(reused)).unwrap();
        let dropped = store.allocate();
        store.free_pages(vec![dropped]);
        store.save_state(reused, 3, 1, &[]).unwrap();
        let next_page_id = store.next_page_id();
        assert!(
            next_page_id > store.page_count() as u64,
            "the witness: next_page_id {next_page_id} is past the file's {} pages",
            store.page_count()
        );
        let (reopened_store, reopened) = DoubleMetaStore::open(Box::new(SharedIo(io))).unwrap();
        assert_eq!(reopened.next_page_id, next_page_id);
        assert!(reopened_store.free_list().contains(&dropped));
    }

    /// The 4th bytes of the `PGM` magics after v1's, in the order the scheme
    /// [`META_MAGIC`] documents hands them out: `2`–`9`, `A`–`Z`, `a`–`z`, without `T`,
    /// which v1's magic holds.
    fn pgm_fourth_bytes() -> Vec<u8> {
        (b'2'..=b'9')
            .chain(b'A'..=b'Z')
            .chain(b'a'..=b'z')
            .filter(|&c| c != b'T')
            .collect()
    }

    /// The 4th bytes of every later 3rd byte's magics: `0`–`9`, `A`–`Z`, `a`–`z`.
    fn later_fourth_bytes() -> Vec<u8> {
        (b'0'..=b'9')
            .chain(b'A'..=b'Z')
            .chain(b'a'..=b'z')
            .collect()
    }

    /// The magic for on-disk format version `v`, per the scheme [`META_MAGIC`]
    /// documents: `PGMT` is v1, `PGM2`…`PGM9` are v2…v9, then the 4th byte rolls past
    /// `'9'` into `'A'` so `PGMA` is v10, `PGMB` v11, and so on — skipping `T`, v1's, and
    /// moving to the next 3rd byte past `z`.
    fn magic_for_version(v: u32) -> [u8; 4] {
        if v == 1 {
            return *b"PGMT";
        }
        let pgm = pgm_fourth_bytes();
        let i = usize::try_from(v - 2).expect("version fits usize");
        if let Some(&last) = pgm.get(i) {
            return [b'P', b'G', b'M', last];
        }
        let later = later_fourth_bytes();
        let i = i - pgm.len();
        let third = b'N' + u8::try_from(i / later.len()).expect("the 3rd byte stays a byte");
        [b'P', b'G', third, later[i % later.len()]]
    }

    /// The version [`META_MAGIC`] denotes — the inverse of [`magic_for_version`].
    fn version_of_magic(magic: [u8; 4]) -> u32 {
        assert_eq!(&magic[0..2], b"PG", "magic must carry the PG prefix");
        let v = if &magic == b"PGMT" {
            1
        } else if magic[2] == b'M' {
            let i = pgm_fourth_bytes()
                .iter()
                .position(|&c| c == magic[3])
                .expect("a PGM magic's 4th byte is in the scheme");
            i + 2
        } else {
            let later = later_fourth_bytes();
            let i = later
                .iter()
                .position(|&c| c == magic[3])
                .expect("a magic's 4th byte is in the scheme");
            2 + pgm_fourth_bytes().len() + usize::from(magic[2] - b'N') * later.len() + i
        };
        u32::try_from(v).expect("version fits u32")
    }

    /// The current magic is not a superseded one — the check that fails when a bump
    /// reuses a magic an earlier version wrote, as `PGMT` would have at v29: a store
    /// written under it would refuse itself as stale on its first reopen.
    #[test]
    fn the_current_magic_is_no_superseded_one() {
        assert!(
            !STALE_MAGICS.contains(&META_MAGIC),
            "{} is a superseded magic: a store written under it refuses itself as stale",
            String::from_utf8_lossy(META_MAGIC)
        );
    }

    /// The scheme never hands out one magic twice, past the end of each 3rd byte's
    /// letters too, and every magic maps back to its version.
    #[test]
    fn the_magic_scheme_never_reuses_a_magic() {
        let mut seen = HashSet::new();
        for v in 1..=200 {
            let magic = magic_for_version(v);
            assert!(
                seen.insert(magic),
                "v{v}'s magic {} is an earlier version's",
                String::from_utf8_lossy(&magic)
            );
            assert_eq!(version_of_magic(magic), v);
        }
        assert_eq!(&magic_for_version(29), b"PGMU", "v29 skips v1's `T`");
        assert_eq!(&magic_for_version(30), b"PGMV");
    }

    #[test]
    fn stale_magics_lists_every_superseded_version() {
        // `stale_format_fails_loud` below iterates `STALE_MAGICS`, so it is structurally
        // blind to the one mutation that matters: bumping `META_MAGIC` and *forgetting*
        // to add the superseded magic to the array removes the entry from both the check
        // and the test, and both stay green. The degradation is not silent but it is the
        // wrong loud — with `PGMF` missing, a `PGMF` file fails the magic compare in both
        // slots and reports "both meta slots are invalid (unrecoverable corruption)",
        // which is exactly the message-contradicts-the-check failure the rendered magic
        // list was introduced to fix.
        //
        // So derive the expected set from `META_MAGIC` rather than reading the array.
        let current = version_of_magic(*META_MAGIC);
        let expected: Vec<[u8; 4]> = (1..current).map(magic_for_version).collect();
        let actual: Vec<[u8; 4]> = STALE_MAGICS.iter().map(|m| **m).collect();
        assert_eq!(
            actual,
            expected,
            "STALE_MAGICS must be exactly v1..v{current} — bumping META_MAGIC to {} means \
             adding {} to the array",
            String::from_utf8_lossy(META_MAGIC),
            String::from_utf8_lossy(&magic_for_version(current - 1))
        );
        // ...and the current magic is not among them, or every fresh store reads as stale.
        assert!(
            !STALE_MAGICS.contains(&META_MAGIC),
            "the current magic must not be listed as superseded"
        );
        // The scheme itself round-trips, so neither helper can drift into agreeing wrongly.
        // The bound is `current`, never a literal: written as one it stops covering the
        // magic in use at the very bump that introduces it, and stays one short forever
        // after — the round-trip's whole point is the magic this build writes.
        for v in 1..=current {
            assert_eq!(version_of_magic(magic_for_version(v)), v);
        }
    }

    #[test]
    fn stale_format_fails_loud() {
        // A non-empty file whose slot still carries any superseded magic must
        // fail loud on reopen, not look like corruption or a fresh store. Every
        // pre-`PGMV` magic qualifies (`PGMT`/…/`PGMS`/`PGMU`) — e.g. a `PGMF` file
        // encodes each catalog sequence as a bare `i64` where a three-field
        // `{last_value, start, owner}` record is now read, so the postcard decode
        // consumes the next field's bytes and desynchronizes the rest of the catalog,
        // while a `PGMG` file decodes cleanly and holds names the engine no longer
        // resolves the same way (see `META_MAGIC`).
        for stale in STALE_MAGICS {
            let mut bytes = vec![0u8; DATA_REGION_START as usize];
            bytes[0..4].copy_from_slice(stale);
            let result = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes)));
            match result {
                Err(PageStoreError::Io(msg)) => {
                    assert!(
                        msg.contains("stale on-disk format"),
                        "expected stale-format message for magic {stale:?}, got: {msg}"
                    );
                }
                other => {
                    panic!("expected loud stale-format error for magic {stale:?}, got {other:?}")
                }
            }
        }
    }

    #[test]
    fn freelist_chain_spans_multiple_pages_and_round_trips() {
        // A free-list far larger than one chain page (FREELIST_IDS_PER_PAGE) must
        // persist across a multi-page chain and rebuild exactly on reopen — the old
        // 64 KiB meta-slot ceiling is gone.
        let n = FREELIST_IDS_PER_PAGE * 3 + 7;
        let bytes = {
            let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
            // Realistic free pages: allocate (bumping the high-water past them) then
            // free, so every freed id is < next_page_id like a genuinely freed page.
            let ids: Vec<PageId> = (0..n).map(|_| store.allocate()).collect();
            store.free_pages(ids.clone());
            store.flush().unwrap();
            store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
            let mut recovered = store.free_list();
            recovered.sort_unstable();
            assert_eq!(recovered, ids);
            store_bytes(&store)
        };

        let (store, meta) = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
        assert_ne!(
            meta.freelist_head, NULL_PAGE_ID,
            "a non-empty free-list has a chain"
        );
        let mut recovered = store.free_list();
        recovered.sort_unstable();
        assert_eq!(
            recovered.len(),
            n,
            "the whole multi-page free-list rebuilds on reopen"
        );
        assert_eq!(recovered, (0..n as u64).collect::<Vec<_>>());
    }

    #[test]
    fn freelist_chain_pages_are_fresh_allocated_not_from_the_free_list() {
        // The self-reference breaker: on the FIRST rewrite of a slot (no prior chain
        // to reuse), chain pages come from fresh next_page_id bumps, never from the
        // free-list being persisted — else a chain page would alias a page the slot
        // also lists as free, corrupting on reuse.
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let count = FREELIST_IDS_PER_PAGE + 5;
        let freed: Vec<PageId> = (0..count).map(|_| store.allocate()).collect();
        store.free_pages(freed.clone());
        let next_before = store.next_page_id();
        store.flush().unwrap();
        store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();

        // next_page_id advanced by the number of chain pages (all fresh, beyond the
        // prior high-water), and none of those chain ids overlap the persisted set.
        let next_after = store.next_page_id();
        assert!(
            next_after > next_before,
            "chain pages bumped the high-water"
        );
        let chain_ids: HashSet<PageId> = (next_before..next_after).collect();
        let persisted: HashSet<PageId> = freed.into_iter().collect();
        assert!(
            chain_ids.is_disjoint(&persisted),
            "chain pages must not alias any persisted free page"
        );
    }

    #[test]
    fn freelist_chain_reuse_bounds_file_under_steady_state_churn() {
        // Regression for the chain-churn leak: with a stable free-list and net-zero
        // data alloc/free (a constant-size table under update churn), two-generation
        // reuse must keep next_page_id FLAT after the two-commit warmup — the earlier
        // fresh-every-commit design grew it ~1 page per commit, unboundedly.
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        // Seed a stable non-empty free-list (well under one chain page).
        let seed: Vec<PageId> = (0..50).map(|_| store.allocate()).collect();
        store.free_pages(seed);

        let mut warm = 0;
        for round in 0..1000u64 {
            // Net-zero data churn: pop K, free the same K back.
            let tmp: Vec<PageId> = (0..5).map(|_| store.allocate()).collect();
            store.free_pages(tmp);
            store.flush().unwrap();
            store.save_state(NULL_PAGE_ID, round + 1, 1, &[]).unwrap();
            if round == 10 {
                warm = store.next_page_id();
            }
        }
        // After warmup both slots' chains are established and reused in place, so the
        // high-water does not advance with commit count.
        assert_eq!(
            store.next_page_id(),
            warm,
            "steady-state churn must not grow the file (chain pages are reused, not fresh-allocated)"
        );
    }

    #[test]
    fn freelist_survives_alternating_reopen_churn() {
        // Reopen seeds BOTH slots' chains (active from its head, alternate from its
        // own), so post-reopen commits keep reusing chain pages rather than orphaning
        // them. Drive several reopen+commit cycles and confirm the free-list stays
        // exactly recoverable and the file stays bounded.
        let mut bytes = {
            let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
            let seed: Vec<PageId> = (0..FREELIST_IDS_PER_PAGE as u64 + 20)
                .map(|_| store.allocate())
                .collect();
            store.free_pages(seed);
            store.flush().unwrap();
            store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
            store_bytes(&store)
        };
        let mut last_high_water = 0;
        for round in 0..6u64 {
            let (store, _) =
                DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
            // Net-zero churn then commit.
            let tmp: Vec<PageId> = (0..3).map(|_| store.allocate()).collect();
            store.free_pages(tmp);
            store.flush().unwrap();
            store.save_state(NULL_PAGE_ID, round + 2, 1, &[]).unwrap();
            // The free-list still rebuilds to the same size every reopen.
            assert_eq!(
                store.free_list().len(),
                FREELIST_IDS_PER_PAGE + 20,
                "free-list size preserved across reopen churn"
            );
            let hw = store.next_page_id();
            if round >= 2 {
                // One-time per-reopen fresh-alloc is bounded; the high-water must not
                // keep climbing once both slots' chains are seeded on open.
                assert!(
                    hw <= last_high_water + (FREELIST_IDS_PER_PAGE as u64 + 4),
                    "per-reopen high-water growth must stay bounded (got {hw} vs {last_high_water})"
                );
            }
            last_high_water = hw;
            bytes = store_bytes(&store);
        }
    }

    /// A save whose free list needs fewer chain pages than its slot's old chain held
    /// lists the surplus pages in the chain it writes, so a close right after it leaks
    /// none of them: every id below `next_page_id` is handed out, free, or a chain page
    /// either slot names. The list shrinks across two chain-page boundaries here, from
    /// three pages to one.
    #[test]
    fn a_shrinking_free_list_leaks_no_chain_page_at_close() {
        let io = std::sync::Arc::new(MemBlockIo::new());
        let (store, _) =
            DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(&io)))).unwrap();
        let ids: Vec<PageId> = (0..1500).map(|_| store.allocate()).collect();
        store.free_pages(ids);
        store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
        store.save_state(NULL_PAGE_ID, 2, 1, &[]).unwrap();
        let chain_before = store.persisted_chain_pages()[0].len();
        let handed_out: HashSet<PageId> = (0..1400).map(|_| store.allocate()).collect();
        store.save_state(NULL_PAGE_ID, 3, 1, &[]).unwrap();
        assert!(
            store.persisted_chain_pages()[0].len() < chain_before,
            "the witness: the chain shrank"
        );
        drop(store);
        let (reopened, meta) = DoubleMetaStore::open(Box::new(SharedIo(io))).unwrap();
        let free: HashSet<PageId> = reopened.free_list().into_iter().collect();
        let chains: HashSet<PageId> = reopened
            .persisted_chain_pages()
            .concat()
            .into_iter()
            .collect();
        let leaked: Vec<PageId> = (0..meta.next_page_id)
            .filter(|id| !handed_out.contains(id) && !free.contains(id) && !chains.contains(id))
            .collect();
        assert!(leaked.is_empty(), "leaked at close: {leaked:?}");
    }

    /// The same, for a list that empties: every page of the slot's old chain is still
    /// named after the save — kept as an empty chain page — so a close right after it
    /// leaks none.
    #[test]
    fn an_emptied_free_list_leaks_no_chain_page_at_close() {
        for listed in [1, 700, 1500] {
            let io = std::sync::Arc::new(MemBlockIo::new());
            let (store, _) =
                DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(&io)))).unwrap();
            let ids: Vec<PageId> = (0..listed).map(|_| store.allocate()).collect();
            store.free_pages(ids);
            store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
            store.save_state(NULL_PAGE_ID, 2, 1, &[]).unwrap();
            let handed_out: HashSet<PageId> = (0..listed).map(|_| store.allocate()).collect();
            store.save_state(NULL_PAGE_ID, 3, 1, &[]).unwrap();
            drop(store);
            let (reopened, meta) = DoubleMetaStore::open(Box::new(SharedIo(io))).unwrap();
            let free: HashSet<PageId> = reopened.free_list().into_iter().collect();
            let chains: HashSet<PageId> = reopened
                .persisted_chain_pages()
                .concat()
                .into_iter()
                .collect();
            let leaked: Vec<PageId> = (0..meta.next_page_id)
                .filter(|id| !handed_out.contains(id) && !free.contains(id) && !chains.contains(id))
                .collect();
            assert!(
                leaked.is_empty(),
                "{listed} listed: leaked at close: {leaked:?}"
            );
        }
    }

    /// A save with nothing to list writes no chain page: once the list is empty, each
    /// slot's one chain page is written empty at that slot's next save and never again,
    /// so every later save writes its slot alone, as a fresh store's does — before and
    /// after a reopen, which reads from each slot's checksum that its chain already
    /// lists nothing. Every page is still accounted for at a close.
    #[test]
    fn a_save_with_nothing_to_list_writes_no_chain_page() {
        use crate::btree::block_io::CountingBlockIo;
        let io = std::sync::Arc::new(MemBlockIo::new());
        let mut handed_out: HashSet<PageId> = HashSet::new();
        let drain = |store: &DoubleMetaStore, handed_out: &mut HashSet<PageId>| {
            while store.free_list_size() > 0 {
                handed_out.insert(store.allocate());
            }
        };
        let mut version = 0;
        for reopen in [false, true] {
            let counting = CountingBlockIo::new(SharedIo(std::sync::Arc::clone(&io)));
            let counts = counting.handle();
            let (store, _) = DoubleMetaStore::open(Box::new(counting)).unwrap();
            // First, what a chain page left unwritten must never do: vouch for a list
            // it no longer holds, whose ids are in use.
            let listed: Vec<PageId> = store
                .free_list()
                .into_iter()
                .filter(|id| handed_out.contains(id))
                .collect();
            assert!(
                listed.is_empty(),
                "reopened {reopen}: ids in use are listed free: {listed:?}"
            );
            if !reopen {
                // A two-page list over both slots, then every id handed out.
                let ids: Vec<PageId> = (0..FREELIST_IDS_PER_PAGE as u64 + 40)
                    .map(|_| store.allocate())
                    .collect();
                store.free_pages(ids);
                for _ in 0..2 {
                    version += 1;
                    store.save_state(NULL_PAGE_ID, version, 1, &[]).unwrap();
                }
                // Each save lists the surplus of the chain it shrinks; drain those too.
                for _ in 0..4 {
                    drain(&store, &mut handed_out);
                    version += 1;
                    store.save_state(NULL_PAGE_ID, version, 1, &[]).unwrap();
                }
                assert_eq!(store.free_list_size(), 0, "the list is empty");
            }
            for _ in 0..8 {
                counts.reset();
                version += 1;
                store.save_state(NULL_PAGE_ID, version, 1, &[]).unwrap();
                assert_eq!(
                    counts.stats().writes,
                    1,
                    "reopened {reopen}: save {version} writes its slot alone"
                );
            }
            drop(store);
            let (reopened, meta) =
                DoubleMetaStore::open(Box::new(SharedIo(std::sync::Arc::clone(&io)))).unwrap();
            let free: HashSet<PageId> = reopened.free_list().into_iter().collect();
            let chains: HashSet<PageId> = reopened
                .persisted_chain_pages()
                .concat()
                .into_iter()
                .collect();
            let leaked: Vec<PageId> = (0..meta.next_page_id)
                .filter(|id| !handed_out.contains(id) && !free.contains(id) && !chains.contains(id))
                .collect();
            assert!(
                leaked.is_empty(),
                "reopened {reopen}: leaked at close: {leaked:?}"
            );
        }
    }

    #[test]
    fn empty_freelist_persists_a_null_head() {
        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        let next_before = store.next_page_id();
        store.flush().unwrap();
        store.save_state(NULL_PAGE_ID, 1, 1, &[]).unwrap();
        // No free-list → no chain pages allocated, NULL head.
        assert_eq!(store.next_page_id(), next_before);
        let bytes = store_bytes(&store);
        let (reopened, meta) =
            DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes))).unwrap();
        assert_eq!(meta.freelist_head, NULL_PAGE_ID);
        assert!(reopened.free_list().is_empty());
    }

    #[test]
    fn concurrent_get_with_put_and_free_is_consistent() {
        // Hammer `get` from many threads across the cache shards while a single
        // writer (the single-writer model) churns the same ids through
        // `free_pages` (the reuse-eviction path) + write-through `put`. Page `id`
        // is always a leaf whose own id is `id`, so a correct cache can only ever
        // return that page for `id`: a returned page with a different `page_id`, a
        // `None` (every id is durably on disk), or an error means the shard routing
        // dropped, misrouted, or torn a read. The eviction-routing and
        // get/put-agreement bugs sharding could introduce all surface here.
        use std::sync::atomic::AtomicBool;

        const N: u64 = 512;
        const READERS: usize = 8;
        const ITERS: u64 = 20_000;

        let (store, _) = DoubleMetaStore::open(Box::new(MemBlockIo::new())).unwrap();
        for id in 0..N {
            // `put` writes disk (extending the file to cover the offset) + cache.
            store.put(Page::new_leaf(id)).unwrap();
        }
        store.flush().unwrap();

        let failed = AtomicBool::new(false);
        std::thread::scope(|scope| {
            for t in 0..READERS {
                let store = &store;
                let failed = &failed;
                scope.spawn(move || {
                    // Per-thread xorshift key stream (no shared RNG); the seed
                    // constant is the golden-ratio mixer, only to decorrelate streams.
                    let mut x = (t as u64)
                        .wrapping_add(1)
                        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    for _ in 0..ITERS {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let id = x % N;
                        match store.get(id) {
                            Ok(Some(page)) if page.page_id() == id => {}
                            _ => failed.store(true, Ordering::SeqCst),
                        }
                    }
                });
            }
            // One writer: free (evict from the id's shard) then re-put the same
            // page (write-through repopulates that shard). Identical bytes each
            // time, so disk reads never tear regardless of interleaving.
            let store = &store;
            scope.spawn(move || {
                let mut y = 0xDEAD_BEEF_u64;
                for _ in 0..ITERS {
                    y ^= y << 13;
                    y ^= y >> 7;
                    y ^= y << 17;
                    let id = y % N;
                    store.free_pages(vec![id]);
                    store.put(Page::new_leaf(id)).unwrap();
                }
            });
        });
        assert!(
            !failed.load(Ordering::SeqCst),
            "a concurrent read returned the wrong page, a spurious miss, or an error"
        );
    }
}
