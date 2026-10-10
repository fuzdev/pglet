use crate::index_keying::{
    IndexKeying, encode_probe_key, encode_range_bounds, gin_intersect_posting_lists,
};
use crate::{Result, RowId, Storage, StorageError};
use pglet_sql::executor::IndexSpec;
use pglet_sql::types::Value;
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

/// Table data with row ID tracking.
#[derive(Debug, Default, Clone)]
struct TableData {
    /// Rows indexed by their ID.
    rows: HashMap<RowId, Vec<Value>>,
    /// Next row ID to assign.
    next_id: RowId,
}

/// A registered secondary index over one table.
///
/// The backend-independent key/predicate logic lives in `keying` (shared with the
/// B+tree backend — see `crate::index_keying`); the only memory-specific field is
/// the in-RAM `entries` map.
#[derive(Debug, Clone)]
struct IndexDef {
    /// Owning table name.
    table: String,
    /// The shared row→key-bytes / row-in-index / parse logic, identical across both
    /// storage backends.
    keying: IndexKeying,
    /// `encoded value` -> row IDs holding that value, kept sorted by row ID.
    entries: BTreeMap<Vec<u8>, Vec<RowId>>,
}

impl IndexDef {
    /// Whether `row` participates in this index (delegates to the shared keying).
    fn row_in_index(&self, row: &[Value]) -> Result<bool> {
        self.keying.row_in_index(row)
    }
}

/// Build an empty `IndexDef` (parsing + caching the partial predicate and any
/// expression key parts via the shared `IndexKeying`) from a spec — the shared
/// half of `create_index` / `reattach_index`.
///
/// # Errors
///
/// Returns `ExprEval` if the partial predicate or a key expression fails to
/// parse/convert.
fn index_def_from_spec(spec: &IndexSpec) -> Result<IndexDef> {
    Ok(IndexDef {
        table: spec.table.clone(),
        keying: IndexKeying::from_spec(spec)?,
        entries: BTreeMap::new(),
    })
}

/// In-memory storage backend
///
/// Data lives only in RAM - useful for testing and ephemeral databases.
/// Each row is assigned a unique `RowId` on insert for efficient operations.
#[derive(Debug, Default, Clone)]
pub struct MemoryStorage {
    tables: HashMap<String, TableData>,
    /// Registered secondary indexes keyed by index id.
    indexes: HashMap<u32, IndexDef>,
    /// Savepoint stacks, **one per connection**: one whole-store snapshot per open
    /// savepoint, innermost last.
    ///
    /// This backend buffers nothing — every write lands immediately — so a
    /// snapshot is the only rollback channel it has, exactly as it is for the
    /// `BEGIN` frame `pglet::TransactionSession` keeps. That makes a savepoint
    /// **O(database)** here against O(pending writes) on the B+tree; acceptable
    /// because this backend is the test double and the B+tree is the default.
    ///
    /// Keyed by connection because a savepoint is its block's: one stack for the store
    /// let another connection's `SAVEPOINT`, block end or re-base move a writer's levels,
    /// so a `ROLLBACK TO` restored a level another block had pushed, or found the stack
    /// out of step with its own frames. The [`Storage`] trait's savepoint methods, which
    /// carry no connection, use [`TRAIT_CONN`]'s stack.
    savepoints: HashMap<i32, Vec<MemorySnapshot>>,
    /// How many transactions have committed on this store — what a `REPEATABLE READ`
    /// block compares against the count at its `BEGIN` to learn that something
    /// committed since (`pglet::StorageBackend::commit_stamp`).
    commits: u64,
}

/// The connection whose savepoint stack the [`Storage`] trait's connection-free
/// savepoint methods use.
const TRAIT_CONN: i32 = 0;

/// The whole of a [`MemoryStorage`]'s state, for one savepoint level.
#[derive(Debug, Clone)]
struct MemorySnapshot {
    tables: HashMap<String, TableData>,
    indexes: HashMap<u32, IndexDef>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many savepoints `conn_id` has open — the lockstep check
    /// `pglet::TransactionSession` asserts against its own frame count.
    #[must_use]
    pub fn savepoint_depth(&self, conn_id: i32) -> usize {
        self.savepoints.get(&conn_id).map_or(0, Vec::len)
    }

    /// Drop every savepoint level `conn_id` holds, keeping the data — what the **end of
    /// a transaction block** does to this backend's stack.
    ///
    /// The B+tree gets this for free: its savepoints live inside the
    /// `BtreeTransactionState` that `commit_transaction` / `rollback_transaction`
    /// remove wholesale. This backend has no per-transaction state to remove, so a
    /// block that ended with a savepoint still open left its snapshot on the stack —
    /// and the next block's first `SAVEPOINT` then failed
    /// `pglet::TransactionSession`'s lockstep check, which is what found it.
    pub fn clear_savepoints(&mut self, conn_id: i32) {
        self.savepoints.remove(&conn_id);
    }

    /// Make every savepoint level `conn_id` holds a snapshot of the store as it stands
    /// now — what a block's savepoints become when the block takes the writer
    /// (`pglet::TransactionSession`'s re-base). A block takes it before its first write,
    /// so each level taken before then already equals its own state and differs from
    /// now only by what other connections committed meanwhile, which a `ROLLBACK TO`
    /// must not take back. Another connection's levels are its own and stay.
    pub fn rebase_savepoints(&mut self, conn_id: i32) {
        let Some(levels) = self.savepoints.get_mut(&conn_id) else {
            return;
        };
        for level in levels {
            level.tables.clone_from(&self.tables);
            level.indexes.clone_from(&self.indexes);
        }
    }

    /// Open a savepoint level for `conn_id`: a snapshot of the whole store.
    pub fn savepoint_for(&mut self, conn_id: i32) {
        let level = MemorySnapshot {
            tables: self.tables.clone(),
            indexes: self.indexes.clone(),
        };
        self.savepoints.entry(conn_id).or_default().push(level);
    }

    /// Put the store back to `conn_id`'s innermost savepoint, keeping it — `ROLLBACK
    /// TO` is repeatable and leaves the savepoint standing.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoSavepoint`] when `conn_id` holds none.
    pub fn rollback_to_savepoint_for(&mut self, conn_id: i32) -> Result<()> {
        let Some(top) = self
            .savepoints
            .get(&conn_id)
            .and_then(|levels| levels.last())
        else {
            return Err(StorageError::NoSavepoint);
        };
        self.tables = top.tables.clone();
        self.indexes = top.indexes.clone();
        Ok(())
    }

    /// Drop `conn_id`'s innermost savepoint, keeping everything written since it.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoSavepoint`] when `conn_id` holds none.
    pub fn release_savepoint_for(&mut self, conn_id: i32) -> Result<()> {
        let levels = self.savepoints.get_mut(&conn_id);
        if levels.and_then(Vec::pop).is_none() {
            return Err(StorageError::NoSavepoint);
        }
        Ok(())
    }

    /// A copy of the store's data — its tables and indexes, no savepoint level — for a
    /// block's `BEGIN` frame to put back with [`Self::restore_data`].
    #[must_use]
    pub fn data_snapshot(&self) -> Self {
        Self {
            tables: self.tables.clone(),
            indexes: self.indexes.clone(),
            savepoints: HashMap::new(),
            commits: self.commits,
        }
    }

    /// Put `snapshot`'s data back over the store, leaving every connection's savepoint
    /// levels and the commit count as they stand — a rollback takes back its own block's
    /// writes, never another block's savepoints.
    pub fn restore_data(&mut self, snapshot: Self) {
        self.tables = snapshot.tables;
        self.indexes = snapshot.indexes;
    }

    /// Count one committed transaction — see [`Self::commit_count`].
    pub const fn note_commit(&mut self) {
        self.commits += 1;
    }

    /// How many transactions have committed on this store.
    #[must_use]
    pub const fn commit_count(&self) -> u64 {
        self.commits
    }

    /// Refuse `row` where an index on `table` could not hold an entry it writes, before
    /// anything is mutated — this backend writes the row before its index entries, and
    /// has no statement rollback to undo a write whose entries are then refused.
    ///
    /// The indexes are asked in id (creation) order, 18.6's, so the index a refusal
    /// names does not depend on the map's iteration order.
    fn check_row_entries(&self, table: &str, row: &[Value]) -> Result<()> {
        let mut defs: Vec<(u32, &IndexDef)> = self
            .indexes
            .iter()
            .filter(|(_, def)| def.table == table)
            .map(|(&id, def)| (id, def))
            .collect();
        defs.sort_unstable_by_key(|&(id, _)| id);
        for (_, def) in defs {
            if def.row_in_index(row)? {
                def.keying.encode_row_entries(row)?;
            }
        }
        Ok(())
    }

    /// Add `row_id` under `row`'s indexed value for every index on `table`.
    ///
    /// For a partial index, a row that fails the predicate is not indexed.
    fn index_insert_row(&mut self, table: &str, row_id: RowId, row: &[Value]) -> Result<()> {
        for def in self.indexes.values_mut() {
            if def.table != table {
                continue;
            }
            if !def.row_in_index(row)? {
                continue;
            }
            // A B-tree row contributes one key; a GIN row one per decomposed element —
            // each refused where it is too large to hold, as the B+tree's are.
            for bytes in def.keying.encode_row_entries(row)? {
                let ids = def.entries.entry(bytes).or_default();
                if let Err(pos) = ids.binary_search(&row_id) {
                    ids.insert(pos, row_id);
                }
            }
        }
        Ok(())
    }

    /// Remove `row_id` (holding `row`) from every index on `table`.
    ///
    /// For a partial index, a row that fails the predicate was never indexed, so
    /// there is nothing to remove (the encode would also miss it).
    fn index_remove_row(&mut self, table: &str, row_id: RowId, row: &[Value]) -> Result<()> {
        for def in self.indexes.values_mut() {
            if def.table != table {
                continue;
            }
            if !def.row_in_index(row)? {
                continue;
            }
            // Remove every entry the row contributed (one for B-tree, N for GIN).
            for bytes in def.keying.encode_row_keys(row)? {
                if let Some(ids) = def.entries.get_mut(&bytes) {
                    if let Ok(pos) = ids.binary_search(&row_id) {
                        ids.remove(pos);
                    }
                    if ids.is_empty() {
                        def.entries.remove(&bytes);
                    }
                }
            }
        }
        Ok(())
    }
}

// `MemoryStorage` impls the single `pglet_core::Storage` trait directly. The
// method bodies are unchanged from the pre-unification single-trait impl; only
// the error type at the boundary moved. Internal calls (`index_insert_row`,
// `index_def_from_spec`, `encode_probe_key`, …) still return the backend's
// native `StorageError` (`crate::Result`), which the `?` operator converts to
// `pglet_core::Error` via `From<StorageError>`; the few bare-`Err` sites take an
// explicit `.into()`.
impl Storage for MemoryStorage {
    fn create_table(&mut self, name: &str) -> pglet_core::Result<()> {
        if self.tables.contains_key(name) {
            return Err(StorageError::TableExists(name.to_string()).into());
        }
        self.tables.insert(name.to_string(), TableData::default());
        Ok(())
    }

    fn drop_table(&mut self, name: &str) -> pglet_core::Result<()> {
        self.tables
            .remove(name)
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;
        // Drop entries of any index over this table (defs stay until drop_index).
        for def in self.indexes.values_mut() {
            if def.table == name {
                def.entries.clear();
            }
        }
        Ok(())
    }

    fn insert(&mut self, table: &str, row: Vec<Value>) -> pglet_core::Result<RowId> {
        if !self.tables.contains_key(table) {
            return Err(StorageError::TableNotFound(table.to_string()).into());
        }
        self.check_row_entries(table, &row)?;
        let data = self
            .tables
            .get_mut(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;
        let id = data.next_id;
        data.next_id += 1;
        data.rows.insert(id, row.clone());
        self.index_insert_row(table, id, &row)?;
        Ok(id)
    }

    fn scan(&self, table: &str) -> pglet_core::Result<Vec<(RowId, Vec<Value>)>> {
        let data = self
            .tables
            .get(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        // Return rows sorted by ID for deterministic ordering
        let mut rows: Vec<(RowId, Vec<Value>)> = data
            .rows
            .iter()
            .map(|(&id, row)| (id, row.clone()))
            .collect();
        rows.sort_by_key(|(id, _)| *id);
        Ok(rows)
    }

    fn get(&self, table: &str, id: RowId) -> pglet_core::Result<Option<Vec<Value>>> {
        let data = self
            .tables
            .get(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        Ok(data.rows.get(&id).cloned())
    }

    fn delete(&mut self, table: &str, id: RowId) -> pglet_core::Result<Option<Vec<Value>>> {
        let data = self
            .tables
            .get_mut(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        let Some(old) = data.rows.remove(&id) else {
            return Ok(None);
        };
        self.index_remove_row(table, id, &old)?;
        Ok(Some(old))
    }

    fn update(
        &mut self,
        table: &str,
        id: RowId,
        new_row: Vec<Value>,
    ) -> pglet_core::Result<Option<Vec<Value>>> {
        self.check_row_entries(table, &new_row)?;
        let data = self
            .tables
            .get_mut(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        // Only update if row exists
        let Some(old_slot) = data.rows.get_mut(&id) else {
            return Ok(None);
        };
        let old = std::mem::replace(old_slot, new_row.clone());
        // Move the index entry from the old value to the new value.
        self.index_remove_row(table, id, &old)?;
        self.index_insert_row(table, id, &new_row)?;
        Ok(Some(old))
    }

    fn add_column(&mut self, table: &str, default: Value) -> pglet_core::Result<()> {
        let data = self
            .tables
            .get_mut(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        for row in data.rows.values_mut() {
            row.push(default.clone());
        }
        // Appending a column never shifts an existing indexed position, so
        // indexes need no rebuild here.
        Ok(())
    }

    fn drop_column(&mut self, table: &str, column_index: usize) -> pglet_core::Result<()> {
        let data = self
            .tables
            .get_mut(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        for row in data.rows.values_mut() {
            if column_index < row.len() {
                row.remove(column_index);
            }
        }
        // NOTE: dropping a column at/below an indexed position shifts that
        // position; the index is not rebuilt for that case (the spine never drops
        // a column on an indexed table). Tracked as future work (position drift).
        Ok(())
    }

    fn create_index(&mut self, spec: IndexSpec) -> pglet_core::Result<()> {
        if !self.tables.contains_key(&spec.table) {
            return Err(StorageError::TableNotFound(spec.table).into());
        }
        let index_id = spec.index_id;
        let mut def = index_def_from_spec(&spec)?;
        // Back-fill from existing rows (partial predicate gates membership).
        let existing: Vec<(RowId, Vec<Value>)> = self
            .tables
            .get(&def.table)
            .map(|d| d.rows.iter().map(|(&id, row)| (id, row.clone())).collect())
            .unwrap_or_default();
        for (row_id, row) in existing {
            if !def.row_in_index(&row)? {
                continue;
            }
            // A B-tree row contributes one key; a GIN row one per decomposed element.
            for bytes in def.keying.encode_row_entries(&row)? {
                let ids = def.entries.entry(bytes).or_default();
                if let Err(pos) = ids.binary_search(&row_id) {
                    ids.insert(pos, row_id);
                }
            }
        }
        // Registered **last**, which is how this backend meets `Storage::create_index`'s
        // all-or-nothing contract: the def is a local until the back-fill is behind it,
        // so a refused entry leaves the registry untouched and the executor is
        // free to write its catalog afterwards. Nothing above the insert mutates `self`.
        self.indexes.insert(index_id, def);
        Ok(())
    }

    fn reattach_index(&mut self, spec: IndexSpec) -> pglet_core::Result<()> {
        if !self.tables.contains_key(&spec.table) {
            return Err(StorageError::TableNotFound(spec.table).into());
        }
        // No back-fill: the def alone is restored. Memory storage doesn't
        // persist, so registering with no existing entries leaves the index
        // empty; the reopen path (which has persisted data) is the B+tree's.
        let index_id = spec.index_id;
        self.indexes.insert(index_id, index_def_from_spec(&spec)?);
        Ok(())
    }

    fn rebuild_index(&mut self, spec: IndexSpec) -> pglet_core::Result<()> {
        // This backend's defs *own* their entries, so replacing the def is already a
        // full rebuild: `create_index` builds the new def — back-fill included — as a
        // local and inserts it over the old one only once it is whole. (`reattach_index`
        // would leave the def holding an empty entry map — right on reopen, where the
        // memory backend has no rows either, and silent data loss here.)
        //
        // Removing the old def *first* is what made a failed rebuild destructive: the
        // back-fill raises for a key that cannot be derived, and the index was then
        // gone from storage while the catalog still carried it. `Storage::rebuild_index`
        // is all-or-nothing, and this is where that had to be paid for.
        self.create_index(spec)
    }

    fn drop_index(&mut self, index_id: u32) -> pglet_core::Result<()> {
        self.indexes.remove(&index_id);
        Ok(())
    }

    fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> pglet_core::Result<Vec<RowId>> {
        let Some(def) = self.indexes.get(&index_id) else {
            return Err(StorageError::IndexNotFound(index_id).into());
        };
        // Posting lists come from this index's `entries` map; the intersection
        // loop itself is shared with the B+tree backend.
        gin_intersect_posting_lists(keys, |enc| {
            Ok(def.entries.get(enc).cloned().unwrap_or_default())
        })
        .map_err(Into::into)
    }

    fn index_lookup(&self, index_id: u32, values: &[Value]) -> pglet_core::Result<Vec<RowId>> {
        let def = self
            .indexes
            .get(&index_id)
            .ok_or(StorageError::IndexNotFound(index_id))?;
        // NULL key columns: no match under default NULLS DISTINCT; probed under NND.
        // The probe folds through the same `encode_probe_key` as maintenance, so a
        // probe tuple and a stored key for the same values agree exactly.
        let Some(bytes) = encode_probe_key(values, def.keying.nulls_not_distinct)? else {
            return Ok(Vec::new());
        };
        Ok(def.entries.get(&bytes).cloned().unwrap_or_default())
    }

    fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> pglet_core::Result<Vec<RowId>> {
        let def = self
            .indexes
            .get(&index_id)
            .ok_or(StorageError::IndexNotFound(index_id))?;

        // The map key is the tagged composite blob — the same bytes `encode_probe_key`
        // yields for a probe value — so a value range is a `BTreeMap::range` over those
        // blobs. The shared `encode_range_bounds` decides both ends, as it does for the
        // B+tree builder: the lower end is a bound's own blob or the least present key,
        // so the walk never starts below a NULL key's blob, and a NULL bound is no bound.
        let ((lower, lower_inclusive), upper) = encode_range_bounds(
            lower.as_ref().map(|(v, incl)| (v, *incl)),
            upper.as_ref().map(|(v, incl)| (v, *incl)),
        )?;

        // `>= a` is `Included`, `> a` is `Excluded`; symmetrically for the upper.
        let start: Bound<Vec<u8>> = if lower_inclusive {
            Bound::Included(lower)
        } else {
            Bound::Excluded(lower)
        };
        let end: Bound<Vec<u8>> = match upper {
            Some((blob, true)) => Bound::Included(blob),
            Some((blob, false)) => Bound::Excluded(blob),
            None => Bound::Unbounded,
        };

        // An interval that names no blob — a lower end above the upper, or equal ends not
        // both included (`k > 3 AND k < 3`, `k BETWEEN 4 AND 2`) — answers no id, as the
        // B+tree's half-open walk does. `BTreeMap::range` would panic on it instead, and
        // the comparison is the map's own, over the same blobs.
        if let (
            Bound::Included(lo) | Bound::Excluded(lo),
            Bound::Included(hi) | Bound::Excluded(hi),
        ) = (&start, &end)
            && (lo > hi
                || (lo == hi
                    && !matches!((&start, &end), (Bound::Included(_), Bound::Included(_)))))
        {
            return Ok(Vec::new());
        }

        // Walk the band in ascending key order; each blob's row ids are already
        // sorted, so the concatenation is in (key, row_id) order.
        let mut ids: Vec<RowId> = Vec::new();
        for (_, row_ids) in def.entries.range((start, end)) {
            ids.extend_from_slice(row_ids);
        }
        if !ascending {
            ids.reverse();
        }
        if let Some(n) = limit {
            ids.truncate(n);
        }
        Ok(ids)
    }

    fn savepoint(&mut self) -> pglet_core::Result<()> {
        self.savepoint_for(TRAIT_CONN);
        Ok(())
    }

    fn rollback_to_savepoint(&mut self) -> pglet_core::Result<()> {
        self.rollback_to_savepoint_for(TRAIT_CONN)
            .map_err(Into::into)
    }

    fn release_savepoint(&mut self) -> pglet_core::Result<()> {
        self.release_savepoint_for(TRAIT_CONN).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
