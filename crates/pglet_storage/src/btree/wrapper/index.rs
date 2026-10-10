//! Secondary-index definitions and maintenance for `BtreeStorageWrapper`.
//!
//! Holds the `IndexDef` type and the `IndexKeying`-backed helpers
//! (`index_def_from_spec`, `encode_row_index_key`), plus the wrapper's
//! index-maintenance plumbing (`table_has_indexes`, `emit_index_ops`) as an
//! `impl super::BtreeStorageWrapper` block. The register/drop/lookup trait
//! methods live with the `Storage` impl in the coordinator module; this is the
//! shared key/maintenance machinery they build on.

use super::BtreeStorageWrapper;
use crate::btree::cow::{BatchChanges, CowBtree};
use crate::btree::{BtreeKey, PageId};
use crate::index_keying::{IndexKeying, gin_intersect_posting_lists};
use crate::{Result, RowId, StorageError};
use pglet_sql::executor::IndexSpec;
use pglet_sql::types::Value;

/// A registered secondary index over one B+tree table.
///
/// Index entries live in the same tree as data rows (`BtreeKey::Index`), so the
/// existing root seqlock makes index state atomic with the data it indexes. The
/// def records what to maintain: the backend-independent key/predicate logic lives
/// in `keying` (shared with the in-memory backend — see `crate::index_keying`);
/// the only B+tree-specific fields are the internal `table_id` and the table name.
#[derive(Debug, Clone)]
pub struct IndexDef {
    /// The internal id of the indexed table.
    pub table_id: u32,
    /// The indexed table's name (for diagnostics; the SQL catalog re-resolves it).
    pub table_name: String,
    /// The shared row→key-bytes / row-in-index / parse logic, identical across both
    /// storage backends (positions, key expressions, partial predicate, NULL
    /// distinctness, column names).
    pub keying: IndexKeying,
}

impl IndexDef {
    /// Whether `row` participates in this index (delegates to the shared keying).
    ///
    /// Pub as a pure diagnostic accessor: the soak harness's in-process
    /// index↔table consistency audit filters live rows by index membership to
    /// count a partial index's expected entries. Read-only, no behavior change.
    ///
    /// # Errors
    ///
    /// Propagates an evaluation error from the predicate.
    pub fn row_in_index(&self, row: &[Value]) -> Result<bool> {
        self.keying.row_in_index(row)
    }

    /// `NULLS NOT DISTINCT` for this index (shared keying field).
    pub(super) const fn nulls_not_distinct(&self) -> bool {
        self.keying.nulls_not_distinct
    }
}

/// Build an `IndexDef` (parsing + caching the partial predicate and any
/// expression key parts via the shared `IndexKeying`) from an `IndexSpec` and the
/// resolved internal `table_id`.
///
/// # Errors
///
/// Returns `Unindexable` if the partial predicate or a key expression fails to
/// parse/convert.
pub(super) fn index_def_from_spec(table_id: u32, spec: &IndexSpec) -> Result<IndexDef> {
    Ok(IndexDef {
        table_id,
        table_name: spec.table.clone(),
        keying: IndexKeying::from_spec(spec)?,
    })
}

/// Encode **all** of `row`'s index-entry key blobs (one for a B-tree key, many for
/// a GIN key — one per decomposed array element / JSONB key). The maintenance path:
/// every entry a row contributes. Delegates to the shared `IndexKeying` so a stored
/// entry and a probe for the same row always agree (see `crate::index_keying`).
///
/// # Errors
///
/// As `IndexKeying::encode_row_keys`: a key expression that fails to evaluate.
pub(super) fn encode_row_index_keys(def: &IndexDef, row: &[Value]) -> Result<Vec<Vec<u8>>> {
    def.keying.encode_row_keys(row)
}

/// Encode **all** of the entries `row` writes into the index, each refused where it is
/// too large to hold — the maintenance path for an entry being **added**
/// (`IndexKeying::encode_row_entries`). Retracting an entry keys through
/// [`encode_row_index_keys`] instead: an entry already stored was sized when it was
/// written.
///
/// # Errors
///
/// `StorageError::IndexEntry` for an oversized entry; as [`encode_row_index_keys`]
/// otherwise.
pub(super) fn encode_row_index_entries(def: &IndexDef, row: &[Value]) -> Result<Vec<Vec<u8>>> {
    def.keying.encode_row_entries(row)
}

/// Encode `row`'s **single** B-tree composite key blob, or `None` under a NULL key.
///
/// The probe-side counterpart used by the read-your-own-writes equality/range
/// scans (`index_lookup_in_tx` / `index_range_lookup_in_tx`), which compare one
/// composite key per row. B-tree only — a GIN index is probed via `gin_lookup`
/// (containment), never these: its entries are the value's decomposed elements, not
/// the whole value's key.
///
/// # Errors
///
/// As `IndexKeying::encode_row_key`: a key expression that fails to evaluate.
pub(super) fn encode_row_index_key(def: &IndexDef, row: &[Value]) -> Result<Option<Vec<u8>>> {
    def.keying.encode_row_key(row)
}

/// Intersect the posting lists of a GIN `@>` probe at `root` (the committed/snapshot
/// probe primitive shared by the autocommit and snapshot read paths).
///
/// For each decomposed query `key`, scan its posting list at `root`
/// (`index_scan_at_root` — the same per-value scan an equality probe uses, sorted by
/// row id) and AND the lists together: a row survives only if it appears under
/// **every** key — the candidate **superset** the caller rechecks. Empty `keys` →
/// empty (the `x @> {}` all-match case is the caller's full scan, not this).
///
/// # Errors
///
/// Propagates an I/O/decode error, or `Unindexable` if a key can't be encoded.
pub(super) fn gin_intersect_at_root(
    storage: &CowBtree,
    root: PageId,
    index_id: u32,
    keys: &[Value],
) -> Result<Vec<RowId>> {
    // Posting lists come from the B+tree at `root`; the intersection loop itself
    // is shared with the in-memory backend via `gin_intersect_posting_lists`.
    gin_intersect_posting_lists(keys, |enc| {
        storage
            .index_scan_at_root(root, index_id, enc)
            .map_err(StorageError::from)
    })
}

impl BtreeStorageWrapper {
    /// Whether `table_id` has any registered secondary index.
    pub(super) fn table_has_indexes(&self, table_id: u32) -> bool {
        self.table_indexes
            .get(&table_id)
            .is_some_and(|ids| !ids.is_empty())
    }

    /// Queue a delete for every committed entry in `index_id`'s band into `batch`
    /// (the index's full key range at the current root). Shared by the auto-commit
    /// `drop_index` and the transaction commit's deferred index drops.
    ///
    /// # Errors
    ///
    /// Returns a storage error if the index band can't be scanned.
    pub(super) fn delete_index_band(&self, batch: &mut BatchChanges, index_id: u32) -> Result<()> {
        let start = BtreeKey::index_table_start(index_id);
        // Exclusive upper bound: the start of the next index id.
        let end = BtreeKey::index_table_end(index_id);
        let root = self.storage.current_root();
        let entries = self
            .storage
            .range_scan_at_root(root, &start, &end)
            .map_err(StorageError::from)?;
        for (key, _) in entries {
            batch.delete(key);
        }
        Ok(())
    }

    /// Remove an index's in-memory def and its `table_id` → index mapping. The
    /// caller deletes the entries separately (`delete_index_band`); this only drops
    /// the registration that drives maintenance and probe validation.
    pub(super) fn deregister_index(&mut self, index_id: u32) {
        if let Some(def) = self.index_defs.remove(&index_id)
            && let Some(ids) = self.table_indexes.get_mut(&def.table_id)
        {
            ids.retain(|id| *id != index_id);
        }
    }

    /// Emit the index key-ops for a single row change into `batch`.
    ///
    /// `old_row` (if present) is the row's current committed/visible values whose
    /// index entries must be removed; `new_row` (if present) is the post-change
    /// values whose index entries must be added. INSERT passes `old_row = None`;
    /// DELETE passes `new_row = None`; UPDATE passes both. Entries land in the
    /// SAME `batch` as the data row op, so they commit atomically under one root
    /// swap.
    ///
    /// # Errors
    ///
    /// Returns an error if an indexed value is not B-tree-indexable.
    pub(super) fn emit_index_ops(
        &self,
        batch: &mut BatchChanges,
        table_id: u32,
        row_id: RowId,
        old_row: Option<&[Value]>,
        new_row: Option<&[Value]>,
    ) -> Result<()> {
        let Some(index_ids) = self.table_indexes.get(&table_id) else {
            return Ok(());
        };
        for index_id in index_ids {
            let Some(def) = self.index_defs.get(index_id) else {
                continue;
            };
            if let Some(old) = old_row {
                // Asymmetric on UPDATE: delete the OLD entries only if the OLD row was
                // in the index — both the partial predicate held AND it had at least
                // one encodable key (no NULL under default NULLS DISTINCT; a non-empty
                // decomposition for GIN). A row with no key (or that failed the
                // predicate) was never indexed, so there is nothing to delete. A GIN
                // row contributes one entry per decomposed element/key.
                if def.row_in_index(old)? {
                    for encoded in encode_row_index_keys(def, old)? {
                        batch.delete(BtreeKey::index(*index_id, &encoded, row_id));
                    }
                }
            }
            if let Some(new) = new_row {
                // Insert the NEW entries only if the NEW row is in the index (its
                // partial predicate holds). A row can enter or leave a partial index
                // on update, so this is independent of the OLD-side decision above.
                if def.row_in_index(new)? {
                    for encoded in encode_row_index_entries(def, new)? {
                        // Index entries carry an empty stored value.
                        batch.insert(BtreeKey::index(*index_id, &encoded, row_id), Vec::new());
                    }
                }
            }
        }
        Ok(())
    }
}
