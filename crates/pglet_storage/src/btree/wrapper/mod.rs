//! Storage trait wrapper for `CowBtree`.
//!
//! `BtreeStorageWrapper` bridges the name-based `Storage` trait to the ID-based
//! `CowBtree`. It maintains a table name → `table_id` mapping and supports
//! both auto-commit and explicit transaction modes.
//!
//! # Usage
//!
//! ```ignore
//! let mut wrapper = BtreeStorageWrapper::new();
//!
//! // Auto-commit mode (default)
//! wrapper.create_table("users")?;
//! let row_id = wrapper.insert("users", vec![Value::Int4(1), Value::Text("Alice".into())])?;
//!
//! // Explicit transaction mode
//! wrapper.begin_transaction(conn_id)?;
//! wrapper.insert_in_tx(conn_id, "users", vec![Value::Int4(2)])?;
//! let rows = wrapper.scan_in_tx(conn_id, "users")?;  // Sees uncommitted insert
//! wrapper.commit_transaction(conn_id)?;  // Atomically commits all writes
//! ```
//!
//! # Transaction Model
//!
//! - **Auto-commit mode**: Each DML operation commits immediately.
//! - **Explicit transaction mode**: Use `begin_transaction()` to start, then `*_in_tx()`
//!   methods for operations. Reads see own uncommitted writes (read-your-own-writes).
//!   `commit_transaction()` atomically applies; `rollback_transaction()` discards.
//!
//! # Module layout
//!
//! `BtreeStorageWrapper` is a thin coordinator over cohesive submodules, each
//! contributing `impl BtreeStorageWrapper` blocks (over the struct's `pub(super)`
//! fields) plus the types it owns:
//!
//! - [`catalog`] — name ↔ id mapping, table-id/row-id allocators, `save_state` +
//!   counter snapshot/restore.
//! - [`index`] — [`IndexDef`], the `IndexKeying` helpers, and index maintenance
//!   (`emit_index_ops`).
//! - [`transaction`] — [`PendingRow`], [`BtreePendingWrites`],
//!   [`BtreeTransactionState`], and the begin/commit/rollback + `*_in_tx` engine.
//! - [`snapshot`] — [`BtreeSnapshot`], the lock-free read facade.
//!
//! The struct definition, the `Storage` trait impl (the auto-commit path), and the
//! cross-cutting coordinator methods (constructors, `read_snapshot`, vacuum/stats)
//! live here.

mod catalog;
mod index;
mod snapshot;
mod transaction;

pub use index::IndexDef;
pub use snapshot::BtreeSnapshot;
pub use transaction::{BtreePendingWrites, BtreeTransactionState, PendingRow};

use self::index::{
    encode_row_index_entries, encode_row_index_keys, gin_intersect_at_root, index_def_from_spec,
};
use super::cow::{BatchChanges, CowBtree, CowBtreeStats, VacuumResult};
use super::page::NULL_PAGE_ID;
use super::system::{decode_sys_table, encode_sys_table};
use super::{BtreeKey, SysKind};
use crate::index_keying::{encode_probe_key, index_range_bounds_for};
use crate::{RowId, Storage, StorageError};
use pglet_sql::executor::IndexSpec;
use pglet_sql::types::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Wrapper around `CowBtree` that implements the `Storage` trait.
///
/// Maps table names to internal table IDs and supports both auto-commit
/// and explicit transaction modes.
pub struct BtreeStorageWrapper {
    /// The underlying `CoW` B+tree (Arc for shared ownership in MVCC).
    pub(super) storage: Arc<CowBtree>,

    /// Mapping from table name to internal table ID.
    pub(super) name_to_id: HashMap<String, u32>,

    /// Next table ID to allocate.
    pub(super) next_table_id: AtomicU64,

    /// How many times a read of **another** connection's staged writes found one —
    /// the unique-key claim walk (`unique_key_claimed_elsewhere`) meeting a claim, or
    /// the table-id restore (`restore_next_table_id`) meeting another transaction's
    /// staged row. Every host lets one connection's block stage writes at a time (the
    /// writer: a block holds it from its first write to its end), so on a host that
    /// keeps that rule both arms answer from nothing and this stays `0`; a count above
    /// it is a write that staged without the writer ([`Self::cross_connection_hits`]).
    pub(super) cross_connection_hits: AtomicU64,

    /// How many whole-table reads a transaction has run (`scan_in_tx`) — the executor's
    /// own scan inside a block or an autocommit write, and every read built on it, the
    /// in-transaction range lookup (`index_range_lookup_in_tx`) among them
    /// ([`Self::table_scans_in_tx`]).
    pub(super) table_scans_in_tx: AtomicU64,

    /// Per-table row ID counters.
    pub(super) row_id_counters: HashMap<u32, AtomicU64>,

    /// Active transactions keyed by connection ID (`process_id`).
    pub(super) transactions: HashMap<i32, BtreeTransactionState>,

    /// Registered secondary indexes keyed by index id.
    pub(super) index_defs: HashMap<u32, IndexDef>,

    /// `table_id` -> the ids of indexes over that table, for fast maintenance
    /// lookups on the write path — ascending, which is creation order, the order 18.6
    /// enters a row's entries in ([`register_table_index`]).
    pub(super) table_indexes: HashMap<u32, Vec<u32>>,

    /// The catalog (postcard `Schema`) bytes last persisted to the tree as
    /// `SYS_CATALOG` chunks. `commit_metadata` rewrites the chunks only when the
    /// incoming schema differs from this, so a plain DML commit (schema unchanged)
    /// skips the catalog write entirely — keeping the autocommit-INSERT path at
    /// exactly two fsyncs. Seeded from the recovered catalog on open.
    ///
    /// Shared (`Arc`) so a reader takes it without copying the catalog, and replaced
    /// whole — never written in place — by each catalog write, so a reader holding one
    /// can tell by identity whether a later save wrote a catalog
    /// ([`Self::persisted_catalog`]).
    pub(super) last_schema: Mutex<Arc<Vec<u8>>>,

    /// The one connection whose transactions this store takes, for a store built for one
    /// ([`Self::for_one_connection`]); `None` where any connection may.
    pub(super) only_connection: Option<i32>,
}

impl std::fmt::Debug for BtreeStorageWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BtreeStorageWrapper")
            .field("tables", &self.name_to_id.keys().collect::<Vec<_>>())
            .field("next_table_id", &self.next_table_id.load(Ordering::SeqCst))
            .field("active_transactions", &self.transactions.len())
            .finish_non_exhaustive()
    }
}

/// Extract the raw payload bytes of a `0x02` system entry's value — one
/// `Value::Bytea` carrying the encoded chunk/meta. `None` if the value isn't the
/// expected single-`Bytea` shape (a corrupt entry the caller rejects loud).
fn sys_payload(value: &[Value]) -> Option<&[u8]> {
    match value.first() {
        Some(Value::Bytea(bytes)) => Some(bytes),
        _ => None,
    }
}

impl BtreeStorageWrapper {
    /// Build the `SYS_TABLE` key/value for `table_id` — `(name, current
    /// next_row_id)` carried as one `Bytea`. Appended to the data batch (insert,
    /// create, txn commit) so the table directory + row-id counter persist
    /// atomically with the rows they describe, in the same root swap and fsync.
    fn sys_table_kv(&self, table_id: u32, name: &str) -> (BtreeKey, Vec<Value>) {
        let next_row_id = self
            .row_id_counters
            .get(&table_id)
            .map_or(0, |c| c.load(Ordering::SeqCst));
        (
            BtreeKey::sys_table(table_id),
            vec![Value::Bytea(encode_sys_table(name, next_row_id))],
        )
    }

    /// Create a new wrapper with a fresh `CowBtree`.
    pub fn new() -> Self {
        Self {
            storage: Arc::new(CowBtree::new()),
            name_to_id: HashMap::new(),
            next_table_id: AtomicU64::new(1),
            cross_connection_hits: AtomicU64::new(0),
            table_scans_in_tx: AtomicU64::new(0),
            row_id_counters: HashMap::new(),
            transactions: HashMap::new(),
            index_defs: HashMap::new(),
            table_indexes: HashMap::new(),
            last_schema: Mutex::new(Arc::new(Vec::new())),
            only_connection: None,
        }
    }

    /// [`Self::new`] for `conn_id` alone, and every fork of it: `pglet`'s
    /// `StorageBackend::begin` refuses a transaction any other connection opens, so no
    /// other connection's block holds a version of this tree to compare a later one
    /// against. A constructor rather than a setter, so such a store is in memory and no
    /// transaction has been open on it before.
    #[must_use]
    pub fn for_one_connection(conn_id: i32) -> Self {
        let mut store = Self::new();
        store.only_connection = Some(conn_id);
        store
    }

    /// The one connection this store serves, where it was built for one
    /// ([`Self::for_one_connection`]).
    #[must_use]
    pub const fn only_connection(&self) -> Option<i32> {
        self.only_connection
    }

    /// Fork this wrapper into an independent one over a forked in-memory tree.
    ///
    /// Returns `None` if the backing store isn't in-memory forkable (a
    /// file/OPFS-backed store). The forked wrapper carries the source's table
    /// directory, row-id counters, and index definitions by value, and shares the
    /// tree's pages by `Arc` refcount until written — so it is a fully isolated
    /// copy of the source's committed state at fork time. It starts with **no**
    /// active transactions: any open transaction's pending writes on the source
    /// (which live here, not in the tree) are not carried.
    ///
    /// This is the per-test-fixture primitive: seed a base once, then fork a cheap
    /// isolated database per test instead of re-running the schema + seed SQL.
    pub fn fork(&self) -> Option<Self> {
        let forked_tree = self.storage.fork()?;
        Some(Self {
            storage: Arc::new(forked_tree),
            name_to_id: self.name_to_id.clone(),
            next_table_id: AtomicU64::new(self.next_table_id.load(Ordering::SeqCst)),
            cross_connection_hits: AtomicU64::new(0),
            table_scans_in_tx: AtomicU64::new(0),
            row_id_counters: self
                .row_id_counters
                .iter()
                .map(|(table_id, counter)| {
                    (*table_id, AtomicU64::new(counter.load(Ordering::SeqCst)))
                })
                .collect(),
            transactions: HashMap::new(),
            index_defs: self.index_defs.clone(),
            table_indexes: self.table_indexes.clone(),
            // A poisoned cache would only make the fork's next commit write the whole
            // catalog into its tree, moving the tree's version (its commit stamp), where
            // an unpoisoned one writes only a changed catalog — a store built for one
            // connection, handed none, writes nothing either way; fall back to empty
            // rather than propagate the poison.
            last_schema: Mutex::new(
                self.last_schema
                    .lock()
                    .map(|g| Arc::clone(&g))
                    .unwrap_or_default(),
            ),
            only_connection: self.only_connection,
        })
    }

    /// Recover a wrapper from a reopened tree.
    ///
    /// Scans the `SYS_TABLE` band to rebuild the name→id map and row-id counters,
    /// and reassembles the `SYS_CATALOG` chunks into the postcard schema bytes
    /// (returned for the caller to decode into its catalog). The single recovery
    /// entry shared by the native and wasm open paths — it replaces the old
    /// slot-sourced mappings/schema/counters now that those live in the tree.
    ///
    /// `next_table_id` comes from the (fixed) meta slot and bounds the `SYS_TABLE`
    /// scan, so a table created-then-rolled-back (whose id is `>= next_table_id`)
    /// is structurally excluded from recovery. A rollback that leaves such an id below
    /// the counter, because rows sit under it, deletes its entry instead
    /// (`restore_next_table_id`): of two entries under one name the scan keeps the higher
    /// id, so an entry left there took a name from the committed table the catalog gives
    /// it.
    ///
    /// # Errors
    ///
    /// Returns a storage error if a system page can't be read (corruption — the
    /// page checksum fails) or a system entry's value is malformed.
    pub fn recover(storage: Arc<CowBtree>, next_table_id: u32) -> crate::Result<(Self, Vec<u8>)> {
        let mut name_to_id = HashMap::new();
        let mut row_id_counters = HashMap::new();
        let mut schema_bytes = Vec::new();

        let root = storage.current_root();
        if root != NULL_PAGE_ID {
            // SYS_TABLE band → (name, next_row_id) per table.
            let tables = storage
                .range_scan_at_root(
                    root,
                    &BtreeKey::sys_table_start(),
                    &BtreeKey::sys_table_end(next_table_id),
                )
                .map_err(StorageError::from)?;
            for (key, value) in tables {
                let Some((SysKind::Table, table_id)) = key.as_system() else {
                    continue;
                };
                let payload = sys_payload(&value)
                    .ok_or_else(|| StorageError::Io(format!("SYS_TABLE {table_id}: no payload")))?;
                let (name, next_row_id) = decode_sys_table(payload)
                    .ok_or_else(|| StorageError::Io(format!("SYS_TABLE {table_id}: malformed")))?;
                name_to_id.insert(name, table_id);
                row_id_counters.insert(table_id, AtomicU64::new(next_row_id));
            }
            // SYS_CATALOG band → the postcard schema blob, reassembled in order.
            let chunks = storage
                .range_scan_at_root(
                    root,
                    &BtreeKey::sys_catalog_start(),
                    &BtreeKey::sys_table_start(),
                )
                .map_err(StorageError::from)?;
            for (_key, value) in chunks {
                let payload = sys_payload(&value)
                    .ok_or_else(|| StorageError::Io("SYS_CATALOG: no payload".into()))?;
                schema_bytes.extend_from_slice(payload);
            }
        }

        let wrapper = Self {
            storage,
            name_to_id,
            next_table_id: AtomicU64::new(u64::from(next_table_id)),
            cross_connection_hits: AtomicU64::new(0),
            table_scans_in_tx: AtomicU64::new(0),
            row_id_counters,
            transactions: HashMap::new(),
            // Index defs are not persisted as system entries; the caller
            // re-registers them via `reattach_index` from the recovered catalog.
            index_defs: HashMap::new(),
            table_indexes: HashMap::new(),
            last_schema: Mutex::new(Arc::new(schema_bytes.clone())),
            only_connection: None,
        };
        Ok((wrapper, schema_bytes))
    }

    /// Get the underlying `CowBtree` for direct access.
    pub fn inner(&self) -> &CowBtree {
        &self.storage
    }

    /// Get shared access to the underlying `CowBtree`.
    ///
    /// Returns an Arc clone that can be used for lock-free snapshot reads.
    /// This is the primary method for MVCC support.
    pub fn btree(&self) -> Arc<CowBtree> {
        Arc::clone(&self.storage)
    }

    // ========================================================================
    // Vacuum / GC Management
    // ========================================================================

    /// Run garbage collection on the underlying B+tree.
    ///
    /// Frees pages that are no longer referenced by any active snapshot.
    /// This is the explicit VACUUM operation.
    pub fn vacuum(&self) -> VacuumResult {
        self.storage.vacuum()
    }

    /// Get statistics about the B+tree storage.
    ///
    /// Includes page counts, active snapshots, pending GC, and free list size.
    pub fn stats(&self) -> CowBtreeStats {
        self.storage.stats()
    }

    /// How many times a read of another connection's staged writes found one — see
    /// the `cross_connection_hits` field. `0` while one block at a time stages writes;
    /// a test or a probe reads it to say so of the workload it ran.
    pub fn cross_connection_hits(&self) -> u64 {
        self.cross_connection_hits.load(Ordering::Relaxed)
    }

    /// How many whole-table reads transactions have run — see the `table_scans_in_tx`
    /// field. A cost guard reads it to tell a write that probed an index inside its
    /// transaction from one that read the table.
    pub fn table_scans_in_tx(&self) -> u64 {
        self.table_scans_in_tx.load(Ordering::Relaxed)
    }

    /// Get the vacuum threshold (pending pages that trigger auto-vacuum).
    pub fn vacuum_threshold(&self) -> usize {
        self.storage.vacuum_threshold()
    }

    // ========================================================================
    // MVCC Snapshot Support (Lock-Free Reads)
    // ========================================================================

    /// Create a read-only snapshot for lock-free reads.
    ///
    /// The returned `BtreeSnapshot` holds a reference to the B+tree at a specific
    /// version, allowing reads without holding any locks. The snapshot pins the
    /// version so pages aren't garbage collected while in use.
    ///
    /// # Usage
    ///
    /// ```ignore
    /// // Get snapshot (brief lock to read state)
    /// let snapshot = wrapper.read_snapshot();
    ///
    /// // Release the storage lock, keep snapshot
    /// drop(storage_guard);
    ///
    /// // Perform lock-free reads
    /// let rows = snapshot.scan("users")?;
    /// let row = snapshot.get("users", row_id)?;
    ///
    /// // Snapshot automatically released on drop
    /// ```
    pub fn read_snapshot(&self) -> BtreeSnapshot {
        BtreeSnapshot {
            btree: Arc::clone(&self.storage),
            snapshot: self.storage.snapshot(),
            name_to_id: self.name_to_id.clone(),
            index_nnd: self
                .index_defs
                .iter()
                .map(|(id, def)| (*id, def.nulls_not_distinct()))
                .collect(),
        }
    }

    /// Registered index definitions, keyed by index id (for diagnostics/tests).
    pub const fn index_defs(&self) -> &HashMap<u32, IndexDef> {
        &self.index_defs
    }
}

impl Default for BtreeStorageWrapper {
    fn default() -> Self {
        Self::new()
    }
}

// `BtreeStorageWrapper` impls the single `pglet_core::Storage` trait directly
// (its auto-commit path). The `*_in_tx` transaction-aware variants live in the
// inherent impl above. The method bodies are unchanged from the pre-unification
// single-trait impl; internal calls still return the backend's native
// `StorageError` (`crate::Result`), which the `?` operator converts to
// `pglet_core::Error` via `From<StorageError>`, with `.into()` / `.map_err(...)`
// at the few non-`?` sites.
impl Storage for BtreeStorageWrapper {
    fn create_table(&mut self, name: &str) -> pglet_core::Result<()> {
        if self.name_to_id.contains_key(name) {
            return Err(StorageError::TableExists(name.to_string()).into());
        }

        let table_id = self.allocate_table_id();
        self.name_to_id.insert(name.to_string(), table_id);
        self.init_row_id_counter(table_id);

        // Write the table's SYS_TABLE directory/counter entry (next_row_id = 0) to
        // the tree so the table is recoverable. DDL is auto-commit (applied
        // immediately); a CREATE rolled back inside a txn lands at an id >=
        // next_table_id (restored on rollback) and is excluded from recovery — or, where
        // rows sit under the id and the counter stays above it, the rollback deletes
        // this entry.
        let (sk, sv) = self.sys_table_kv(table_id, name);
        let mut batch = BatchChanges::new();
        batch.insert(sk, sv);
        self.storage
            .apply_batch(batch)
            .map_err(StorageError::from)?;

        Ok(())
    }

    fn drop_table(&mut self, name: &str) -> pglet_core::Result<()> {
        let table_id = self.get_table_id(name)?;

        // Remove from mapping
        self.name_to_id.remove(name);
        self.remove_row_id_counter(table_id);

        // Delete all rows for this table, shedding any index entries in the same
        // batch so the index can't dangle past the table. `emit_index_ops` no-ops
        // when the table has no index, so one batched path covers both cases.
        let entries = self
            .storage
            .scan_table(table_id)
            .map_err(StorageError::from)?;
        let mut batch = BatchChanges::new();
        for (key, values) in entries {
            let row_id = key.row_id();
            self.emit_index_ops(&mut batch, table_id, row_id, Some(&values), None)?;
            batch.delete(key);
        }
        // Remove the table's SYS_TABLE directory/counter entry in the same batch
        // (always present since `create_table`), so the batch is never empty.
        batch.delete(BtreeKey::sys_table(table_id));
        self.storage
            .apply_batch(batch)
            .map_err(StorageError::from)?;

        // Index entries are gone; the defs stay registered until `drop_index`.
        Ok(())
    }

    fn insert(&mut self, table: &str, row: Vec<Value>) -> pglet_core::Result<RowId> {
        let table_id = self.get_table_id(table)?;
        let row_id = self.allocate_row_id(table_id);

        let key = BtreeKey::new(table_id, row_id);
        // Data row + index entries + the table's bumped SYS_TABLE counter commit
        // atomically in one batch/root swap — so the row-id counter rides the same
        // fsync as the row it counts (the autocommit-INSERT 2-fsync path).
        // `emit_index_ops` no-ops when the table has no index.
        let mut batch = BatchChanges::new();
        self.emit_index_ops(&mut batch, table_id, row_id, None, Some(&row))?;
        batch.insert(key, row);
        let (sk, sv) = self.sys_table_kv(table_id, table);
        batch.insert(sk, sv);
        self.storage
            .apply_batch(batch)
            .map_err(StorageError::from)?;

        Ok(row_id)
    }

    fn scan(&self, table: &str) -> pglet_core::Result<Vec<(RowId, Vec<Value>)>> {
        let table_id = self.get_table_id(table)?;

        let entries = self
            .storage
            .scan_table(table_id)
            .map_err(StorageError::from)?;

        Ok(entries
            .into_iter()
            .map(|(key, values)| (key.row_id(), values))
            .collect())
    }

    fn get(&self, table: &str, id: RowId) -> pglet_core::Result<Option<Vec<Value>>> {
        let table_id = self.get_table_id(table)?;
        let key = BtreeKey::new(table_id, id);

        // A genuinely-absent row is `Ok(None)`; a real corruption/IO error
        // propagates rather than being silently flattened to "not found" (which
        // would also leave update/delete skipping their stale index-entry shed).
        self.storage
            .get(&key)
            .map_err(|e| StorageError::from(e).into())
    }

    fn delete(&mut self, table: &str, id: RowId) -> pglet_core::Result<Option<Vec<Value>>> {
        let table_id = self.get_table_id(table)?;
        let key = BtreeKey::new(table_id, id);

        // Get old row data first (for RETURNING support + index removal). A real
        // corruption/IO error propagates; only a genuinely-absent row is `None`
        // (so we don't skip shedding the old index entries on a swallowed error).
        let old_row = self.storage.get(&key).map_err(StorageError::from)?;

        if let Some(ref old) = old_row {
            // Data row + index entries shed atomically in one batch/root swap.
            // `emit_index_ops` no-ops when the table has no index.
            let mut batch = BatchChanges::new();
            self.emit_index_ops(&mut batch, table_id, id, Some(old), None)?;
            batch.delete(key);
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }

        Ok(old_row)
    }

    fn update(
        &mut self,
        table: &str,
        id: RowId,
        new_row: Vec<Value>,
    ) -> pglet_core::Result<Option<Vec<Value>>> {
        let table_id = self.get_table_id(table)?;
        let key = BtreeKey::new(table_id, id);

        // Get old row data first (for RETURNING support + index move). A real
        // corruption/IO error propagates; only a genuinely-absent row is `None`
        // (so we don't skip moving the old index entry on a swallowed error).
        let old_row = self.storage.get(&key).map_err(StorageError::from)?;

        if let Some(ref old) = old_row {
            // Move any index entry from the old value to the new value, atomic with
            // the data row overwrite in one batch. `emit_index_ops` no-ops when the
            // table has no index, so the batched insert just overwrites the row.
            let mut batch = BatchChanges::new();
            self.emit_index_ops(&mut batch, table_id, id, Some(old), Some(&new_row))?;
            batch.insert(key, new_row);
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }

        Ok(old_row)
    }

    fn add_column(&mut self, table: &str, default: Value) -> pglet_core::Result<()> {
        let table_id = self.get_table_id(table)?;

        // Get all existing rows
        let entries = self
            .storage
            .scan_table(table_id)
            .map_err(StorageError::from)?;

        if entries.is_empty() {
            return Ok(());
        }

        // Reinsert each row with the new column appended, in one batch (one root
        // swap). The data-row key is unchanged, so this is a pure value overwrite;
        // index entries (keyed on the indexed column value) are unaffected by an
        // appended column.
        let mut batch = BatchChanges::new();
        for (key, mut row) in entries {
            row.push(default.clone());
            batch.insert(key, row);
        }
        if !batch.is_empty() {
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }

        Ok(())
    }

    fn drop_column(&mut self, table: &str, column_index: usize) -> pglet_core::Result<()> {
        let table_id = self.get_table_id(table)?;

        // Get all existing rows
        let entries = self
            .storage
            .scan_table(table_id)
            .map_err(StorageError::from)?;

        if entries.is_empty() {
            return Ok(());
        }

        // Reinsert each row without the dropped column, in one batch (one root
        // swap). The data-row key is unchanged, so this is a pure value overwrite.
        let mut batch = BatchChanges::new();
        for (key, mut row) in entries {
            if column_index < row.len() {
                row.remove(column_index);
                batch.insert(key, row);
            }
        }
        if !batch.is_empty() {
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }

        // NOTE: dropping a column at/below an indexed position shifts that
        // position; the index is not rebuilt for that case (the spine never drops
        // a column on an indexed table). Tracked as future work (position drift).
        Ok(())
    }

    fn create_index(&mut self, spec: IndexSpec) -> pglet_core::Result<()> {
        let index_id = spec.index_id;
        let table_id = self.get_table_id(&spec.table)?;
        let def = index_def_from_spec(table_id, &spec)?;

        // Back-fill from the local `def`, then register it. The back-fill reads
        // nothing out of `self.index_defs` — `row_in_index` and
        // `encode_row_index_keys` take the def by reference and `BtreeKey::index`
        // takes the raw id — so registering first bought nothing and cost the
        // all-or-nothing contract: the back-fill is fallible (an entry its page
        // cannot hold, a predicate that raises) and left the def and the
        // `table_indexes` entry standing on the failure, maintaining an index the
        // catalog need not carry. As in `rebuild_index`, every step above the
        // `apply_batch` (the encode included) mutates nothing, and the registration
        // sits below it.
        let entries = self
            .storage
            .scan_table(table_id)
            .map_err(StorageError::from)?;

        let mut batch = BatchChanges::new();
        for (key, values) in entries {
            let row_id = key.row_id();
            // Add only the new index's entries. A row is indexed only when its
            // partial predicate holds AND it has at least one encodable key (no NULL
            // under default NULLS DISTINCT; for GIN, one entry per decomposed
            // element/key).
            if def.row_in_index(&values)? {
                for encoded in encode_row_index_entries(&def, &values)? {
                    batch.insert(BtreeKey::index(index_id, &encoded, row_id), Vec::new());
                }
            }
        }
        if !batch.is_empty() {
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }

        self.index_defs.insert(index_id, def);
        register_table_index(&mut self.table_indexes, table_id, index_id);

        Ok(())
    }

    fn reattach_index(&mut self, spec: IndexSpec) -> pglet_core::Result<()> {
        let index_id = spec.index_id;
        let table_id = self.get_table_id(&spec.table)?;

        // Reopen path: the index entries already live in the persisted tree, so
        // we only restore the in-memory def + table->index mapping that drives
        // maintenance and probe validation. No tree writes, no back-fill.
        let def = index_def_from_spec(table_id, &spec)?;
        self.index_defs.insert(index_id, def);
        register_table_index(&mut self.table_indexes, table_id, index_id);
        Ok(())
    }

    fn rebuild_index(&mut self, spec: IndexSpec) -> pglet_core::Result<()> {
        let index_id = spec.index_id;
        let table_id = self.get_table_id(&spec.table)?;
        let def = index_def_from_spec(table_id, &spec)?;

        // One batch: shed every committed entry in this index's band, then re-derive
        // the band from the current rows under the new def. Applied directly rather
        // than through `drop_index` + `create_index` — inside a transaction that drop
        // is deferred to commit, where it would deregister the def the create just
        // restored (see `Storage::rebuild_index`).
        let mut batch = BatchChanges::new();
        self.delete_index_band(&mut batch, index_id)?;

        let entries = self
            .storage
            .scan_table(table_id)
            .map_err(StorageError::from)?;
        for (key, values) in entries {
            if def.row_in_index(&values)? {
                for encoded in encode_row_index_keys(&def, &values)? {
                    batch.insert(
                        BtreeKey::index(index_id, &encoded, key.row_id()),
                        Vec::new(),
                    );
                }
            }
        }

        // The entries land before the def does, so a failed `apply_batch` leaves both
        // on the order they came in on rather than a def whose entries never moved —
        // `Storage::rebuild_index` is all-or-nothing, and every step above this one
        // (the encode included) mutates nothing.
        //
        // The def swap and the `table_id` registration are idempotent: this index is
        // already registered (rebuild, not create), and `deregister_index` is never
        // called here, so the `table_indexes` entry stands as it is.
        if !batch.is_empty() {
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }
        self.index_defs.insert(index_id, def);
        self.table_indexes.entry(table_id).or_default();
        Ok(())
    }

    fn drop_index(&mut self, index_id: u32) -> pglet_core::Result<()> {
        if !self.index_defs.contains_key(&index_id) {
            return Ok(());
        }

        // Remove every entry for this index (its full band) and deregister the def.
        let mut batch = BatchChanges::new();
        self.delete_index_band(&mut batch, index_id)?;
        self.deregister_index(index_id);
        if !batch.is_empty() {
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }

        Ok(())
    }

    fn index_lookup(&self, index_id: u32, values: &[Value]) -> pglet_core::Result<Vec<RowId>> {
        let Some(def) = self.index_defs.get(&index_id) else {
            return Err(StorageError::IndexNotFound(index_id).into());
        };
        // NULL key columns: no match under default NULLS DISTINCT; probed under NND.
        // The committed index only holds predicate-satisfying rows, so a partial
        // index needs no extra filtering here.
        let Some(encoded) = encode_probe_key(values, def.nulls_not_distinct())? else {
            return Ok(Vec::new());
        };

        let root = self.storage.current_root();
        self.storage
            .index_scan_at_root(root, index_id, &encoded)
            .map_err(|e| StorageError::from(e).into())
    }

    fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> pglet_core::Result<Vec<RowId>> {
        if !self.index_defs.contains_key(&index_id) {
            return Err(StorageError::IndexNotFound(index_id).into());
        }
        let (start, end) = index_range_bounds_for(
            index_id,
            lower.as_ref().map(|(v, incl)| (v, *incl)),
            upper.as_ref().map(|(v, incl)| (v, *incl)),
        )?;
        let root = self.storage.current_root();
        self.storage
            .index_range_scan_at_root(root, &start, &end, ascending, limit)
            .map_err(|e| StorageError::from(e).into())
    }

    fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> pglet_core::Result<Vec<RowId>> {
        if !self.index_defs.contains_key(&index_id) {
            return Err(StorageError::IndexNotFound(index_id).into());
        }
        // Intersect the query keys' posting lists at the current committed root —
        // MVCC-consistent with data reads at the same root.
        let root = self.storage.current_root();
        Ok(gin_intersect_at_root(&self.storage, root, index_id, keys)?)
    }
    // The auto-commit path carries no `conn_id`, so it cannot name a transaction to
    // stage a savepoint in. Every real caller reaches the savepoint stack through
    // `StorageAdapter`, which does carry one; this arm exists so the trait is total
    // and answers loud rather than silently accepting a savepoint it would never
    // roll back to.
    fn savepoint(&mut self) -> pglet_core::Result<()> {
        Err(StorageError::NoTransaction(0).into())
    }
    fn rollback_to_savepoint(&mut self) -> pglet_core::Result<()> {
        Err(StorageError::NoTransaction(0).into())
    }
    fn release_savepoint(&mut self) -> pglet_core::Result<()> {
        Err(StorageError::NoTransaction(0).into())
    }
}

/// Add `index_id` to `table_id`'s index list, kept ascending — so a list rebuilt by
/// reattaching a store's indexes on open, in whatever order the catalog hands them over,
/// is the same list the creations built.
fn register_table_index(table_indexes: &mut HashMap<u32, Vec<u32>>, table_id: u32, index_id: u32) {
    let ids = table_indexes.entry(table_id).or_default();
    if let Err(at) = ids.binary_search(&index_id) {
        ids.insert(at, index_id);
    }
}

#[cfg(test)]
mod tests;
