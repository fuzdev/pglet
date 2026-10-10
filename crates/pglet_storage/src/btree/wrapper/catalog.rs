//! Catalog and ID-allocation facade for `BtreeStorageWrapper`.
//!
//! Owns the name → `table_id` mapping, the table-id and per-table row-id
//! allocators, and the metadata snapshot/restore path (`save_state` plus the
//! counter snapshot/restore helpers used by transaction rollback). These are
//! `impl super::BtreeStorageWrapper` blocks split out of the coordinator module;
//! they operate on the struct's `pub(super)` fields.

use super::super::BtreeKey;
use super::super::cow::BatchChanges;
use super::super::system::chunk_catalog;
use super::BtreeStorageWrapper;
use crate::{Result, RowId, StorageError};
use pglet_sql::types::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A catalog chunk index as the `u32` the `SYS_CATALOG` key carries.
///
/// Loud rather than clamping: the prior `unwrap_or(u32::MAX)` would have silently
/// collided every chunk past `u32::MAX` onto one key (a silent-corruption shape).
/// Unreachable at real schema sizes (a `u32::MAX`-chunk catalog is ~13 TB), but
/// the bound is enforced, not assumed.
fn chunk_index(i: usize) -> Result<u32> {
    u32::try_from(i)
        .map_err(|_| StorageError::Io("catalog has more chunks than a u32 key can address".into()))
}

impl BtreeStorageWrapper {
    /// Get the table name → ID mapping.
    pub const fn table_mappings(&self) -> &HashMap<String, u32> {
        &self.name_to_id
    }

    /// Get the next table ID (for persistence).
    pub fn next_table_id(&self) -> u64 {
        self.next_table_id.load(Ordering::SeqCst)
    }

    /// Get the row ID counters for persistence.
    ///
    /// Returns a list of (`table_id`, `next_row_id`) pairs for all tables.
    pub fn row_id_counters(&self) -> Vec<(u32, u64)> {
        self.row_id_counters
            .iter()
            .map(|(k, v)| (*k, v.load(Ordering::SeqCst)))
            .collect()
    }

    /// Commit the catalog + fixed slot metadata to durable storage.
    ///
    /// Two steps: (1) write the catalog (`SYS_CATALOG` chunks) into the tree — but
    /// **only if it changed** since the last persist, so a DML commit writes no catalog
    /// pages unless the live catalog changed since then — a sequence draw, or another
    /// connection's open block's DDL (`EDGES.md` row 211); (2) flip the meta slot via `PageStore::save_state`, passing
    /// only the logical root/version/next_table_id — the store owns `next_page_id`
    /// and persists the free-list as a page chain itself. The table directory +
    /// row-id counters (`SYS_TABLE`) are written eagerly with the data (see
    /// `sys_table_kv`), so they're not assembled here. Call after commits to ensure
    /// durability.
    pub fn commit_metadata(&self, schema_bytes: Vec<u8>) -> Result<()> {
        self.persist_catalog_if_changed(schema_bytes)?;

        self.storage
            .save_state(self.next_table_id() as u32)
            .map_err(StorageError::from)
    }

    /// The catalog bytes the tree last took from [`Self::commit_metadata`], or recovered
    /// on open — empty for a store no save has written a catalog to. While the store
    /// serves, a save's catalog batch is followed by its slot flip or fails the store, so
    /// on a store with durable state these are the catalog that state holds.
    ///
    /// Shared, not copied: every catalog write replaces the value whole, so two reads
    /// answer the same allocation (`Arc::ptr_eq`) exactly when no catalog write came
    /// between them — while the first is held, since only then can its allocation not be
    /// reused.
    ///
    /// # Errors
    ///
    /// A poisoned cache lock, as `StorageError::Io`.
    pub fn persisted_catalog(&self) -> Result<Arc<Vec<u8>>> {
        self.last_schema
            .lock()
            .map(|bytes| Arc::clone(&bytes))
            .map_err(|e| StorageError::Io(format!("last_schema lock poisoned: {e}")))
    }

    /// Rewrite the catalog as `SYS_CATALOG` chunks **iff** `schema_bytes` differs
    /// from the catalog last persisted (cached in `last_schema`).
    ///
    /// The catalog is the one `0x02` system write that does **not** ride a data
    /// batch (it's produced above storage at the persist boundary). Gating on a
    /// real change keeps it off most of the DML path: an autocommit `INSERT` that draws
    /// no sequence value changes no catalog bytes of its own, so this no-ops and the commit
    /// stays at two fsyncs, unless the live catalog holds a change no save has written —
    /// another connection's open block's DDL is one (`EDGES.md` row 211). A DDL commit pays the extra catalog batch (and its flush), and so
    /// does any write that draws `nextval` — a `SERIAL` or identity column's default
    /// included — because the sequence counters live in the catalog: such an insert
    /// rewrites the whole catalog. A store built for one connection
    /// ([`Self::for_one_connection`]) is handed no catalog, so it writes none. Frees
    /// superseded chunk keys when the catalog shrank.
    fn persist_catalog_if_changed(&self, schema_bytes: Vec<u8>) -> Result<()> {
        let mut last = self
            .last_schema
            .lock()
            .map_err(|e| StorageError::Io(format!("last_schema lock poisoned: {e}")))?;
        if **last == schema_bytes {
            return Ok(());
        }

        let new_chunks = chunk_catalog(&schema_bytes);
        let old_count = chunk_catalog(&last).len();

        let mut batch = BatchChanges::new();
        for (i, chunk) in new_chunks.iter().enumerate() {
            batch.insert(
                BtreeKey::sys_catalog(chunk_index(i)?),
                vec![Value::Bytea(chunk.clone())],
            );
        }
        // Catalog shrank: delete the now-superseded trailing chunks.
        for i in new_chunks.len()..old_count {
            batch.delete(BtreeKey::sys_catalog(chunk_index(i)?));
        }
        if !batch.is_empty() {
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }
        // Cache the just-persisted catalog (move; no clone) for the next change check, as
        // a new allocation — what tells a reader of `persisted_catalog` a write came.
        *last = Arc::new(schema_bytes);
        Ok(())
    }

    /// Look up table ID by name.
    pub(super) fn get_table_id(&self, name: &str) -> Result<u32> {
        self.name_to_id
            .get(name)
            .copied()
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))
    }

    /// Allocate a new table ID.
    pub(super) fn allocate_table_id(&self) -> u32 {
        self.next_table_id.fetch_add(1, Ordering::SeqCst) as u32
    }

    /// Put `next_table_id` back to `to` for a rollback that took back the tables created
    /// since — as far as no id it would hand out again keys anything a table could read —
    /// and take the directory entry of every id it stays above out of the tree.
    ///
    /// A table the rollback takes back could hold rows it did not write while another
    /// connection could write during an open block: that connection reads the block's
    /// `CREATE TABLE` from the catalog (`docs/storage.md` § Schema Snapshot Semantics),
    /// and its write landed under the id, and handed out again the id gave the next table
    /// those rows, live and after a reopen. A block now holds the writer from its first
    /// write to its end, so such a write waits instead; the counter still stops above the
    /// highest of those ids that the tree holds a row under, or that another open
    /// transaction has a write staged for, a delete included, and such an id is never
    /// handed out again. The second of those arms reads other connections' staged writes,
    /// which no longer exist while the writer is held, so a hit on it is counted
    /// (`cross_connection_hits`). Where no id is written, the restore is whole — which is
    /// what keeps a rolled-back table's `SYS_TABLE` entry out of the recovery scan.
    ///
    /// Every id the counter stays above — each table the rollback takes back up to the
    /// highest written one, written or not — would be inside that scan, which gives a
    /// repeated name to the higher id: a block that dropped `t` and created it again handed
    /// a reopen the new `t`'s id under the name the restored catalog gives the committed `t`,
    /// which then read the other connection's rows in place of its own (measured). So the
    /// rollback deletes those ids' `SYS_TABLE` entries from the live tree, which every later
    /// save writes; their rows stay, under an id no name maps. Nothing writes the entries
    /// again: a table's directory entry is written under its id only through its name, and
    /// no name maps these ids once the rollback restores the directory.
    ///
    /// # Errors
    ///
    /// A page the store cannot read or write. The counter is stored first, so an id it
    /// stays above is never handed out again even then.
    pub(super) fn restore_next_table_id(&self, to: u32) -> Result<()> {
        let live = u32::try_from(self.next_table_id.load(Ordering::SeqCst)).unwrap_or(u32::MAX);
        let root = self.storage.current_root();
        let staged = |id: u32| {
            let hit = self
                .transactions
                .values()
                .any(|tx| tx.pending.table_changes(id).is_some());
            if hit {
                self.cross_connection_hits.fetch_add(1, Ordering::Relaxed);
            }
            hit
        };
        let written = |id: u32| {
            self.storage
                .range_scan_at_root_limited(
                    root,
                    &BtreeKey::table_start(id),
                    &BtreeKey::table_end(id),
                    Some(1),
                )
                .map_or(true, |rows| !rows.is_empty())
        };
        let floor = (to..live)
            .rev()
            .find(|&id| written(id) || staged(id))
            .map_or(to, |id| id + 1);
        self.next_table_id.store(u64::from(floor), Ordering::SeqCst);
        let mut batch = BatchChanges::new();
        for id in to..floor {
            batch.delete(BtreeKey::sys_table(id));
        }
        self.storage
            .apply_batch(batch)
            .map_err(StorageError::from)?;
        Ok(())
    }

    /// One past the greatest index id the tree holds an entry under, or `0` where it holds
    /// none: the lowest number a reopen may hand a new index.
    ///
    /// The catalog does not remember a number it has handed out — the host's counter is
    /// seeded from the catalog's greatest OID on open — and an index's entries can outlive
    /// its catalog entry in a saved tree: a `DROP INDEX` inside a block leaves them until
    /// its `COMMIT`, and a rollback sheds a block's own index in the live tree, leaving the
    /// entries in what a save made while the block was open wrote until a later save. A
    /// number handed out again then read those entries as its own. Read from the tree
    /// itself, so it bounds every entry a reopen can find.
    ///
    /// # Errors
    ///
    /// A page the store cannot read.
    pub fn index_id_floor(&self) -> Result<u32> {
        let last = self
            .storage
            .last_key_in_range_at_root(
                self.storage.current_root(),
                &BtreeKey::index_table_start(0),
                &BtreeKey::sys_catalog_start(),
            )
            .map_err(StorageError::from)?;
        Ok(match last {
            Some(BtreeKey::Index { index_id, .. }) => index_id.saturating_add(1),
            _ => 0,
        })
    }

    /// Forget every table the directory maps under a name `keep` refuses — a reopen's
    /// reconciliation with the catalog, which is written with the tree and names every
    /// table a statement committed.
    ///
    /// A table the directory holds and the catalog lacks is one no committed statement
    /// left that way: a block's `DROP TABLE`, deferred to `COMMIT`, was open when a save
    /// wrote the catalog without it. Left mapped, a `CREATE TABLE` under its name was
    /// handed the old id and its rows (the storage's `TableExists` is not an error to the
    /// executor). The rows stay in the tree, under an id no table is handed.
    ///
    /// It keeps every name the catalog holds, whichever id the directory maps it to, so it
    /// cannot repair a name the directory binds to the wrong table. A table a rollback
    /// took back under an id the counter stays above would be such a name where the block
    /// had dropped a committed table of the same name and created it again; the rollback
    /// deletes that table's entry instead ([`Self::restore_next_table_id`]).
    pub fn forget_tables(&mut self, keep: impl Fn(&str) -> bool) {
        let row_id_counters = &mut self.row_id_counters;
        self.name_to_id.retain(|name, id| {
            let kept = keep(name);
            if !kept {
                row_id_counters.remove(id);
            }
            kept
        });
    }

    /// Allocate a new row ID for a table.
    pub(super) fn allocate_row_id(&mut self, table_id: u32) -> RowId {
        self.row_id_counters
            .entry(table_id)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::SeqCst)
    }

    /// Initialize row ID counter for a new table.
    pub(super) fn init_row_id_counter(&mut self, table_id: u32) {
        self.row_id_counters
            .entry(table_id)
            .or_insert_with(|| AtomicU64::new(0));
    }

    /// Remove row ID counter for a dropped table.
    pub(super) fn remove_row_id_counter(&mut self, table_id: u32) {
        self.row_id_counters.remove(&table_id);
    }
}
