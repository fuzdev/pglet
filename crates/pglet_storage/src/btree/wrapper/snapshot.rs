//! Lock-free read snapshot facade for `BtreeStorageWrapper`.
//!
//! `BtreeSnapshot` pins a `(version, root)` of the underlying `CowBtree` plus the
//! table mapping and per-index NULL-distinctness flags captured at snapshot time,
//! so reads (scan/get/index probes) run with no locks held and observe an
//! isolated view. The snapshot releases its pin on `Drop`, allowing the pinned
//! pages to be garbage-collected.

use super::index::gin_intersect_at_root;
use crate::btree::BtreeKey;
use crate::btree::cow::{CowBtree, Snapshot};
use crate::index_keying::{encode_probe_key, index_range_bounds_for};
use crate::{Result, RowId, StorageError};
use pglet_sql::types::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Read-only snapshot handle for lock-free reads.
///
/// This struct holds a snapshot of the B+tree at a specific version, allowing
/// concurrent reads without holding any locks. Multiple connections can hold
/// snapshots simultaneously.
///
/// The snapshot is automatically released when dropped, allowing garbage
/// collection of old pages.
pub struct BtreeSnapshot {
    /// Shared reference to the B+tree.
    pub(super) btree: Arc<CowBtree>,
    /// The snapshot (version + root pointer).
    pub(super) snapshot: Snapshot,
    /// Table name → ID mapping at snapshot time.
    pub(super) name_to_id: HashMap<String, u32>,
    /// The store's live untoasted-table set (`BtreeStorageWrapper::untoasted`), read at
    /// this snapshot's ids: a table marked since the snapshot was taken only refuses.
    pub(super) untoasted: Arc<Mutex<HashSet<u32>>>,
    /// Registered index id → `nulls_not_distinct` at snapshot time. Presence is
    /// the probe-validity check; the flag selects the NULL-key probe encoding.
    pub(super) index_nnd: HashMap<u32, bool>,
}

impl BtreeSnapshot {
    /// Whether `table`'s every row is known to be within 18.6's TOAST threshold
    /// (`pglet_core::Storage::rows_untoasted`), by this snapshot's table ids.
    #[must_use]
    pub fn rows_untoasted(&self, table: &str) -> bool {
        super::untoasted_contains(&self.untoasted, &self.name_to_id, table)
    }

    /// Get the snapshot version.
    pub const fn version(&self) -> u64 {
        self.snapshot.version
    }

    /// Scan all rows in a table.
    ///
    /// Returns rows from the snapshot - does not see concurrent modifications.
    pub fn scan(&self, table: &str) -> Result<Vec<(RowId, Vec<Value>)>> {
        let table_id = self
            .name_to_id
            .get(table)
            .copied()
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        let entries = self
            .btree
            .scan_table_at_root(self.snapshot.root, table_id)
            .map_err(StorageError::from)?;

        Ok(entries
            .into_iter()
            .map(|(key, values)| (key.row_id(), values))
            .collect())
    }

    /// Get a specific row by ID.
    ///
    /// Returns the row from the snapshot - does not see concurrent modifications.
    /// A genuinely-absent row is `Ok(None)`; a real corruption/IO error propagates
    /// rather than being silently flattened to "not found".
    pub fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>> {
        let table_id = self
            .name_to_id
            .get(table)
            .copied()
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        let key = BtreeKey::new(table_id, id);
        self.btree
            .get_at_root(self.snapshot.root, &key)
            .map_err(StorageError::from)
    }

    /// Equality probe of a secondary index, **at this snapshot's root**.
    ///
    /// Returns the row IDs whose indexed column equals `value` as of the snapshot
    /// — the MVCC-consistent counterpart to `BtreeStorageWrapper::index_lookup`'s
    /// current-root probe. Because index entries share the data tree's root,
    /// reading at `self.snapshot.root` observes exactly the index state that
    /// matches data reads at the same root. A NULL `value` returns no rows under
    /// default `NULLS DISTINCT`; under `NULLS NOT DISTINCT` it probes the NULL key.
    /// For a partial index the committed entries already contain only
    /// predicate-satisfying rows, so no extra filtering is needed here.
    ///
    /// # Errors
    ///
    /// Returns an error if the index doesn't exist in this snapshot or the value
    /// is not B-tree-indexable.
    pub fn index_lookup(&self, index_id: u32, values: &[Value]) -> Result<Vec<RowId>> {
        let Some(&nulls_not_distinct) = self.index_nnd.get(&index_id) else {
            return Err(StorageError::IndexNotFound(index_id));
        };
        // NULL key columns: no match under default NULLS DISTINCT; probed under NND.
        let Some(encoded) = encode_probe_key(values, nulls_not_distinct)? else {
            return Ok(Vec::new());
        };
        self.btree
            .index_scan_at_root(self.snapshot.root, index_id, &encoded)
            .map_err(StorageError::from)
    }

    /// Ordered range probe of a secondary index, **at this snapshot's root**.
    ///
    /// The range counterpart to [`index_lookup`](Self::index_lookup): returns the
    /// row ids whose indexed value falls in `[lower, upper]` (per inclusivity) as of
    /// the snapshot, in index-key order (ascending when `ascending`), limited to
    /// `limit`. Because index entries share the data tree's root, reading at
    /// `self.snapshot.root` observes the index state matching data reads at the same
    /// root. A NULL bound is treated as no bound that side, and no range holds a NULL
    /// key: the walk starts at the least present key, above the NULL keys a `NULLS NOT
    /// DISTINCT` index holds (`index_keying::encode_range_bounds`).
    ///
    /// # Errors
    ///
    /// Returns an error if the index doesn't exist in this snapshot or a bound value
    /// is not B-tree-indexable.
    pub fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<RowId>> {
        if !self.index_nnd.contains_key(&index_id) {
            return Err(StorageError::IndexNotFound(index_id));
        }
        let (start, end) = index_range_bounds_for(index_id, lower, upper)?;
        self.btree
            .index_range_scan_at_root(self.snapshot.root, &start, &end, ascending, limit)
            .map_err(StorageError::from)
    }

    /// Containment (`@>`) probe of a GIN index, **at this snapshot's root**.
    ///
    /// The GIN counterpart to [`index_lookup`](Self::index_lookup): intersects the
    /// decomposed query `keys`' posting lists as of the snapshot, yielding the
    /// candidate **superset** the reader rechecks with the real `@>`. Index entries
    /// share the data tree's root, so reading at `self.snapshot.root` observes index
    /// state matching data reads at the same root.
    ///
    /// # Errors
    ///
    /// Returns an error if the index doesn't exist in this snapshot or a key is not
    /// indexable.
    pub fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>> {
        if !self.index_nnd.contains_key(&index_id) {
            return Err(StorageError::IndexNotFound(index_id));
        }
        gin_intersect_at_root(&self.btree, self.snapshot.root, index_id, keys)
    }

    /// Check if a table exists in this snapshot.
    pub fn table_exists(&self, table: &str) -> bool {
        self.name_to_id.contains_key(table)
    }
}

impl Drop for BtreeSnapshot {
    fn drop(&mut self) {
        self.btree.release_snapshot(&self.snapshot);
    }
}

impl std::fmt::Debug for BtreeSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BtreeSnapshot")
            .field("version", &self.snapshot.version)
            .field("root", &self.snapshot.root)
            .field("tables", &self.name_to_id.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}
