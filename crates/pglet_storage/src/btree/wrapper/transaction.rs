//! Transaction engine for `BtreeStorageWrapper`.
//!
//! Holds the buffered-write types (`PendingRow`, `BtreePendingWrites`), the
//! per-connection `BtreeTransactionState`, and the wrapper's transaction
//! lifecycle + read-your-own-writes (`*_in_tx`) operations as an
//! `impl super::BtreeStorageWrapper` block. Reads merge the transaction's pending
//! buffer over a snapshot root; `commit_transaction` applies everything in one
//! `apply_batch` (a single CoW root swap).

use super::BtreeStorageWrapper;
use super::index::{IndexDef, encode_row_index_entries, encode_row_index_key};
use crate::btree::BtreeKey;
use crate::btree::cow::{BatchChanges, Snapshot};
use crate::index_keying::{encode_probe_key, index_range_bounds_for};
use crate::{Result, RowId, StorageError};
use pglet_sql::executor::IndexMethod;
use pglet_sql::gin::gin_contains_keys;
use pglet_sql::types::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::Ordering;

/// Pending row state within a transaction.
#[derive(Debug, Clone)]
pub enum PendingRow {
    /// Row data (insert or update).
    Data(Vec<Value>),
    /// Row deleted.
    Deleted,
}

/// Buffered writes for a B+tree transaction.
///
/// Tracks inserts, updates, and deletes per table until commit.
///
/// `Clone` is what a savepoint costs on this backend: one copy of the *pending*
/// writes, not of the database (see [`BtreeSavepoint`]).
#[derive(Debug, Default, Clone)]
pub struct BtreePendingWrites {
    /// Per-table pending changes: `table_id` -> (`row_id` -> `PendingRow`).
    changes: HashMap<u32, BTreeMap<RowId, PendingRow>>,
    /// Tables dropped in this transaction. The destructive deletion (rows +
    /// `SYS_TABLE` + index entries) is deferred to `commit_transaction`, so a
    /// ROLLBACK simply discards this set and the table is left intact.
    dropped_tables: HashSet<u32>,
    /// Indexes dropped in this transaction (standalone `DROP INDEX` and the
    /// cascade from a dropped table). Deferred to commit like `dropped_tables`.
    dropped_indexes: HashSet<u32>,
}

impl BtreePendingWrites {
    /// Create empty pending writes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are no pending changes.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.dropped_tables.is_empty() && self.dropped_indexes.is_empty()
    }

    /// Insert a row.
    pub fn insert(&mut self, table_id: u32, row_id: RowId, values: Vec<Value>) {
        self.changes
            .entry(table_id)
            .or_default()
            .insert(row_id, PendingRow::Data(values));
    }

    /// Update a row.
    pub fn update(&mut self, table_id: u32, row_id: RowId, values: Vec<Value>) {
        self.changes
            .entry(table_id)
            .or_default()
            .insert(row_id, PendingRow::Data(values));
    }

    /// Delete a row.
    pub fn delete(&mut self, table_id: u32, row_id: RowId) {
        self.changes
            .entry(table_id)
            .or_default()
            .insert(row_id, PendingRow::Deleted);
    }

    /// Mark a table as dropped.
    pub fn drop_table(&mut self, table_id: u32) {
        self.dropped_tables.insert(table_id);
        // Remove any pending changes for this table
        self.changes.remove(&table_id);
    }

    /// Mark an index as dropped in this transaction (deferred to commit).
    pub fn drop_index(&mut self, index_id: u32) {
        self.dropped_indexes.insert(index_id);
    }

    /// Get pending state for a specific row.
    pub fn get(&self, table_id: u32, row_id: RowId) -> Option<&PendingRow> {
        if self.dropped_tables.contains(&table_id) {
            return Some(&PendingRow::Deleted);
        }
        self.changes.get(&table_id)?.get(&row_id)
    }

    /// Check if a table was dropped in this transaction.
    pub fn is_table_dropped(&self, table_id: u32) -> bool {
        self.dropped_tables.contains(&table_id)
    }

    /// Get all pending changes for a table.
    pub fn table_changes(&self, table_id: u32) -> Option<&BTreeMap<RowId, PendingRow>> {
        self.changes.get(&table_id)
    }

    /// Get all changes (for commit).
    pub const fn all_changes(&self) -> &HashMap<u32, BTreeMap<RowId, PendingRow>> {
        &self.changes
    }

    /// Get dropped tables (for commit).
    pub const fn dropped_tables(&self) -> &HashSet<u32> {
        &self.dropped_tables
    }

    /// Get dropped indexes (for commit).
    pub const fn dropped_indexes(&self) -> &HashSet<u32> {
        &self.dropped_indexes
    }
}

/// Per-connection transaction state for B+tree storage.
///
/// Holds the buffered writes and snapshot for a single transaction.
/// Created on `BEGIN`, consumed on `COMMIT` or `ROLLBACK`.
#[derive(Debug)]
pub struct BtreeTransactionState {
    /// Buffered writes for this transaction.
    pub pending: BtreePendingWrites,

    /// The tree version this transaction reads: taken at `BEGIN`, taken again when the
    /// block takes the writer ([`BtreeStorageWrapper::rebase_transaction`]), and moved
    /// onto what the transaction itself publishes by an index build or rebuild
    /// ([`BtreeStorageWrapper::advance_transaction_snapshot`]).
    pub snapshot: Snapshot,

    /// The table name → ID mapping a rollback restores — the one at `BEGIN`, re-taken
    /// with [`Self::snapshot`].
    pub name_to_id_snapshot: HashMap<String, u32>,

    /// The `next_table_id` a rollback restores, taken with [`Self::name_to_id_snapshot`].
    pub next_table_id_snapshot: u32,

    /// Count of this transaction's own catalog-allocator DDL ops
    /// (`create_table`/`drop_table`), recorded by `note_txn_ddl`. DDL is applied
    /// auto-commit even inside a transaction (it has no `conn_id`), so the adapter
    /// attributes each in-transaction DDL back to its connection here — which is what
    /// keeps a commit whose batch is not the whole of it from being dropped on a
    /// refused write.
    pub own_ddl_ops: u32,

    /// Index ids this transaction created (`CREATE INDEX`, and the auto-created
    /// PK/UNIQUE indexes of an in-transaction `CREATE TABLE`). The build is applied
    /// immediately to keep read-your-own-writes (a probe within the transaction sees
    /// the new index), so rollback must **undo** each: deregister the def and shed
    /// its entries. Recorded by `note_txn_created_index` via the adapter.
    pub created_indexes: HashSet<u32>,

    /// Pending index for O(1) equality/uniqueness probes over this transaction's
    /// buffered rows: `index_id` -> (encoded key -> row ids). B-tree indexes only (GIN
    /// is probed via `gin_lookup_in_tx`).
    ///
    /// Maintained by `insert_in_tx`, `update_in_tx` and `delete_in_tx` through the one
    /// [`restage_pending_index`](BtreeTransactionState::restage_pending_index) call,
    /// which retracts the row's *staged* image's keys before adding its new ones; the
    /// column-DDL paths write `pending` directly and mark `probe_scan_tables` instead
    /// (below). That completeness is what keeps a probe off the O(pending) scan: a
    /// single `UPDATE` would otherwise put its whole table on `probe_scan_tables` for the
    /// rest of the block.
    pub pending_index: HashMap<u32, HashMap<Vec<u8>, Vec<RowId>>>,

    /// Tables whose pending buffer took a write the pending index cannot follow —
    /// column add/drop (every row rewritten, and a dropped column shifts the positions
    /// a key encoding reads), or a key-encoding/predicate failure — so probe them via
    /// the full pending scan instead of the fast lookup.
    pub probe_scan_tables: HashSet<u32>,

    /// Open savepoints, innermost last. See [`BtreeSavepoint`].
    pub savepoints: Vec<BtreeSavepoint>,

    /// Whether this block is **aborted**, which suspends the unique-key claims its
    /// pending writes hold against other connections' probes.
    ///
    /// Set by `pglet::TransactionSession::fail` (through
    /// [`suspend_claims`](BtreeStorageWrapper::suspend_claims)) and cleared by a
    /// successful `ROLLBACK TO SAVEPOINT`, which is the one statement that revives an
    /// aborted block. It gates the claims only — the block's *own* reads still see
    /// every pending write, because a `ROLLBACK TO SAVEPOINT` can hand them back.
    ///
    /// Measured on 18.6, both directions: `BEGIN; INSERT 'k'; SELECT 1/0;` leaves a
    /// second connection's `INSERT 'k'` free to run immediately (the failing statement
    /// aborted the top-level transaction, so its row is already dead), while
    /// `BEGIN; INSERT 'k'; SAVEPOINT s; SELECT 1/0;` does not — the error aborted only
    /// the subtransaction, and the pre-savepoint row is still live. Those are
    /// [`claimed_writes`](BtreeTransactionState::claimed_writes)' two arms. Behind the
    /// writer no other connection observes them: an aborted block keeps the writer until
    /// it ends, so another connection's write waits for its `ROLLBACK` on the native
    /// server, and is refused `55P03` by the embedded `Database`, where 18.6 lets it
    /// through at the error (the repo's `EDGES.md` row 230).
    pub claims_suspended: bool,
}

/// One savepoint level: everything a `rollback_to_savepoint` puts back.
///
/// # Why these fields and not the tree
///
/// A savepoint costs a clone of the transaction's **pending** state, never of the
/// database, because everything an open transaction writes is buffered: rows and
/// column DDL land in `pending`, and `DROP TABLE` / `DROP INDEX` are deferred into
/// it too. What is *not* buffered is the catalog allocator (`name_to_id`,
/// `next_table_id`, applied immediately by `create_table`) and a `CREATE INDEX`
/// (built immediately so a probe inside the transaction sees it) — the same two
/// exceptions a whole-block rollback already handles, restored the same way here.
///
/// `pending_index` and `probe_scan_tables` are derived accelerators over
/// `pending`, and they are restored **because** they are derived: leaving a
/// rolled-back row in the pending index makes the next unique probe answer
/// `23505` against a row no longer staged.
#[derive(Debug)]
pub struct BtreeSavepoint {
    pending: BtreePendingWrites,
    name_to_id: HashMap<String, u32>,
    next_table_id: u32,
    own_ddl_ops: u32,
    created_indexes: HashSet<u32>,
    pending_index: HashMap<u32, HashMap<Vec<u8>, Vec<RowId>>>,
    probe_scan_tables: HashSet<u32>,
}

/// One pending buffer read as a set of index-key holders — the shape both the
/// read-your-own-writes merge and the cross-connection claim walk probe.
///
/// The four fields are exactly what deciding "does this buffer hold key `k` in index
/// `i`?" needs, and they travel together because the fast path is only valid when all
/// four agree: `pending_index` answers in O(1) *while* it is complete for the table —
/// no column DDL, no key-encoding failure, no index created or dropped in the block —
/// and otherwise the buffer must be scanned and each row re-encoded. Splitting that
/// condition from the map it gates is how a probe silently reads a stale accelerator.
struct ClaimedWrites<'a> {
    pending: &'a BtreePendingWrites,
    pending_index: &'a HashMap<u32, HashMap<Vec<u8>, Vec<RowId>>>,
    probe_scan_tables: &'a HashSet<u32>,
    created_indexes: &'a HashSet<u32>,
}

impl ClaimedWrites<'_> {
    /// The row ids in this buffer whose **current** image encodes to `target` under
    /// `def`.
    ///
    /// The one derivation of "this buffer holds that key", reached from both probing
    /// positions: the merge half of
    /// [`index_lookup_in_tx`](BtreeStorageWrapper::index_lookup_in_tx) (the connection's
    /// own writes) and
    /// [`unique_key_claimed_elsewhere`](BtreeStorageWrapper::unique_key_claimed_elsewhere)
    /// (another connection's). One derivation is the point: a partial index's predicate
    /// (`IndexDef::row_in_index`) and `NULLS NOT DISTINCT` (folded into
    /// `encode_row_index_key`) are honoured here because they are honoured *once*, not
    /// because a second reader remembered to.
    ///
    /// # Errors
    ///
    /// Propagates a predicate-evaluation or key-encoding failure from `def`. Under a host
    /// that lets one block stage writes at a time the claim walk meets no foreign buffer
    /// holding a row, so this is not reached there; were it reached, the error would
    /// reach a **different** connection's statement, which is an open question rather
    /// than a known defect: the scan branch re-derives only
    /// the *probed* index's key, never the whole `pending_btree_index_keys` set an
    /// insert derives, so the foreign row has to fail the very index the prober is
    /// enforcing. Three routes were traced and each ends at a gate the SQL layer runs
    /// first: `prepared_unique_index_key` evaluates the partial predicate and the key
    /// expression before the write, and `encode_probe_key` refuses the encoder's own
    /// unsupported types — so on those the row never stages, and no reachable case was
    /// constructed. Measured nearby and **not** it: a staged row that fails a
    /// *different* index's encoding (a plain B-tree index over `float8`) routes its
    /// table to the scan path and propagates nothing, because that def is not the one
    /// re-derived here. It is not an `EDGES.md` row because nothing has been measured
    /// to diverge; it is here because this is the reader who would have to close it.
    fn matching_rows(&self, def: &IndexDef, index_id: u32, target: &[u8]) -> Result<Vec<RowId>> {
        let table_id = def.table_id;
        let fall_back_to_scan = self.probe_scan_tables.contains(&table_id)
            || !self.created_indexes.is_empty()
            || !self.pending.dropped_indexes().is_empty();
        if !fall_back_to_scan {
            // Fast path: the pending index is complete for this table, so it holds
            // exactly the matching pending rows. Deliberately not gated on the table
            // having pending changes: the accelerator is derived from `pending` and the
            // two are built and restored together, so the gate would be unreachable —
            // and if they ever did desync it would turn a spurious conflict into a
            // **missed** one, which is the wrong direction for a uniqueness gate.
            return Ok(self
                .pending_index
                .get(&index_id)
                .and_then(|by_key| by_key.get(target))
                .cloned()
                .unwrap_or_default());
        }
        // A `Deleted` entry contributes nothing; a partial index counts only
        // predicate-satisfying rows (`row_in_index`), like the committed side.
        let Some(changes) = self.pending.table_changes(table_id) else {
            return Ok(Vec::new());
        };
        let mut ids = Vec::new();
        for (row_id, pending_row) in changes {
            if let PendingRow::Data(row) = pending_row
                && def.row_in_index(row)?
                && let Some(bytes) = encode_row_index_key(def, row)?
                && bytes == target
            {
                ids.push(*row_id);
            }
        }
        Ok(ids)
    }
}

impl BtreeTransactionState {
    /// Create a new transaction state.
    pub fn new(
        snapshot: Snapshot,
        name_to_id_snapshot: HashMap<String, u32>,
        next_table_id_snapshot: u32,
    ) -> Self {
        Self {
            pending: BtreePendingWrites::new(),
            snapshot,
            name_to_id_snapshot,
            next_table_id_snapshot,
            own_ddl_ops: 0,
            created_indexes: HashSet::new(),
            pending_index: HashMap::new(),
            probe_scan_tables: HashSet::new(),
            savepoints: Vec::new(),
            claims_suspended: false,
        }
    }

    /// Everything this transaction has buffered, for its **own** read-your-own-writes
    /// probes.
    ///
    /// Deliberately not [`claimed_writes`](Self::claimed_writes): a connection sees all
    /// of its own pending writes whatever the block's state, because an aborted block
    /// that a `ROLLBACK TO SAVEPOINT` revives goes on reading them.
    const fn live_writes(&self) -> ClaimedWrites<'_> {
        ClaimedWrites {
            pending: &self.pending,
            pending_index: &self.pending_index,
            probe_scan_tables: &self.probe_scan_tables,
            created_indexes: &self.created_indexes,
        }
    }

    /// The writes this transaction currently holds unique-key claims for against
    /// *other* connections' probes, or `None` when it holds none.
    ///
    /// A healthy block claims everything it has buffered. An aborted one claims only
    /// what survives its abort — the innermost open savepoint's snapshot, because the
    /// failing statement aborted just that subtransaction — and nothing at all with no
    /// savepoint open, because then the failing statement aborted the whole block.
    /// [`claims_suspended`](Self::claims_suspended) carries the 18.6 measurements this
    /// mirrors.
    fn claimed_writes(&self) -> Option<ClaimedWrites<'_>> {
        if !self.claims_suspended {
            return Some(self.live_writes());
        }
        let frame = self.savepoints.last()?;
        Some(ClaimedWrites {
            pending: &frame.pending,
            pending_index: &frame.pending_index,
            probe_scan_tables: &frame.probe_scan_tables,
            created_indexes: &frame.created_indexes,
        })
    }

    /// Move one row's contribution to [`pending_index`](Self::pending_index) from
    /// `old_keys` to `new_keys` — the **one** maintenance point, reached from
    /// `insert_in_tx`, `update_in_tx` and `delete_in_tx`.
    ///
    /// `old_keys` is empty when nothing of this row is staged yet: the pending index
    /// never holds a row's *committed* image, so retracting it would remove an entry
    /// another staged row owns. `new_keys` is empty when the row is being deleted.
    /// Both come from `pending_btree_index_keys`, the same derivation
    /// `ClaimedWrites::matching_rows`' scan branch re-runs, which is why the two agree
    /// without a second notion of what a row's keys are.
    ///
    /// A table already on `probe_scan_tables` is left alone: its entries go unread, so
    /// maintaining them would be work with no reader — and a partially-maintained map
    /// behind a scan that is used anyway cannot be observed either way. The entries it
    /// leaves stale cannot outlive that scan: the one route off `probe_scan_tables` is
    /// [`rollback_to_savepoint`](BtreeStorageWrapper::rollback_to_savepoint), which
    /// replaces `pending`, this map and that set together from one snapshot.
    fn restage_pending_index(
        &mut self,
        table_id: u32,
        row_id: RowId,
        old_keys: &[(u32, Vec<u8>)],
        new_keys: Vec<(u32, Vec<u8>)>,
    ) {
        if self.probe_scan_tables.contains(&table_id) {
            return;
        }
        for (index_id, key) in old_keys {
            if let Some(by_key) = self.pending_index.get_mut(index_id)
                && let Some(ids) = by_key.get_mut(key)
            {
                ids.retain(|&id| id != row_id);
                if ids.is_empty() {
                    by_key.remove(key);
                }
            }
        }
        for (index_id, key) in new_keys {
            self.pending_index
                .entry(index_id)
                .or_default()
                .entry(key)
                .or_default()
                .push(row_id);
        }
    }

    /// Snapshot this transaction's restorable state for a new savepoint level.
    ///
    /// `name_to_id` / `next_table_id` come from the **wrapper's live** values rather
    /// than from this state's `*_snapshot` fields, which hold the values at `BEGIN`: a
    /// savepoint restores to where the statement stream stood when it was taken, not
    /// to the top of the block.
    fn snapshot(&self, name_to_id: &HashMap<String, u32>, next_table_id: u32) -> BtreeSavepoint {
        BtreeSavepoint {
            pending: self.pending.clone(),
            name_to_id: name_to_id.clone(),
            next_table_id,
            own_ddl_ops: self.own_ddl_ops,
            created_indexes: self.created_indexes.clone(),
            pending_index: self.pending_index.clone(),
            probe_scan_tables: self.probe_scan_tables.clone(),
        }
    }
}

impl BtreeStorageWrapper {
    /// Begin a new transaction for a connection.
    ///
    /// Snapshots the current state for isolation and potential rollback.
    pub fn begin_transaction(&mut self, conn_id: i32) -> Result<()> {
        if self.transactions.contains_key(&conn_id) {
            // Transaction already active - idempotent
            return Ok(());
        }

        let snapshot = self.storage.snapshot();
        let name_to_id_snapshot = self.name_to_id.clone();
        let next_table_id_snapshot = self.next_table_id.load(Ordering::SeqCst) as u32;

        self.transactions.insert(
            conn_id,
            BtreeTransactionState::new(snapshot, name_to_id_snapshot, next_table_id_snapshot),
        );

        Ok(())
    }

    /// Re-base `conn_id`'s transaction on the state committed now: a fresh tree
    /// snapshot, and the directory and table-id counter a rollback restores — for the
    /// block and for every savepoint it holds.
    ///
    /// What a block does when it takes **the writer** — the one connection whose block
    /// may stage writes, held from its first write to its end. Between its `BEGIN` and
    /// that write, other connections committed; re-based here, every rollback the block
    /// can make restores exactly what it changed, and its writes are decided against
    /// what is committed rather than what its `BEGIN` saw. Sound only because nothing is
    /// staged yet: a block re-bases once, before its first write. A no-op with no
    /// transaction open.
    pub fn rebase_transaction(&mut self, conn_id: i32) {
        let name_to_id = self.name_to_id.clone();
        let next_table_id = self.next_table_id.load(Ordering::SeqCst) as u32;
        let Some(tx) = self.transactions.get_mut(&conn_id) else {
            return;
        };
        debug_assert!(
            tx.pending.is_empty() && tx.created_indexes.is_empty() && tx.own_ddl_ops == 0,
            "a block re-bases before it stages anything"
        );
        // The new snapshot is registered before the old one is released, so no page
        // either reads is collectable in between.
        let old = std::mem::replace(&mut tx.snapshot, self.storage.snapshot());
        self.storage.release_snapshot(&old);
        for savepoint in &mut tx.savepoints {
            savepoint.name_to_id.clone_from(&name_to_id);
            savepoint.next_table_id = next_table_id;
        }
        tx.next_table_id_snapshot = next_table_id;
        tx.name_to_id_snapshot = name_to_id;
    }

    /// Move `conn_id`'s transaction onto the root published now, so that it reads what it
    /// has just published itself — the entries an index build or rebuild derived from the
    /// rows committed before the block, which its unique and foreign-key probes and its
    /// indexed reads read at `BtreeTransactionState::snapshot`. A no-op with no
    /// transaction open.
    ///
    /// The host calls it after such a publish. A transaction a session block or an
    /// autocommit statement opens takes **the writer** before it can publish, and that is
    /// what makes it sound: no other connection publishes while the writer is held, so the
    /// root published now differs from the transaction's snapshot only by the
    /// transaction's own eager publishes. The one storage transaction no session opened —
    /// the block `Database::into_parts` leaves open in the storage half (the repo's
    /// `EDGES.md` row 216) — holds no writer, and an advance there reads other
    /// connections' commits. The ones this
    /// wrapper's methods make for a statement inside a block — an index build or rebuild,
    /// a `CREATE TABLE`'s directory entry, a `ROLLBACK TO`'s undo — write index and
    /// system entries, the block's rows being staged, so the rows the transaction reads,
    /// and the old rows its commit derives index removals from, are the same at either
    /// root.
    ///
    /// Only the read root moves. What a rollback restores is not read from it: the
    /// directory and table-id counter are the `*_snapshot` fields and the savepoint
    /// frames, and an index the transaction built is taken back by shedding its entries
    /// ([`Self::rollback_transaction`], [`Self::rollback_to_savepoint`]).
    pub fn advance_transaction_snapshot(&mut self, conn_id: i32) {
        let Some(tx) = self.transactions.get_mut(&conn_id) else {
            return;
        };
        // The new snapshot is registered before the old one is released, so no page
        // either reads is collectable in between.
        let old = std::mem::replace(&mut tx.snapshot, self.storage.snapshot());
        self.storage.release_snapshot(&old);
    }

    /// End `conn_id`'s transaction **without restoring anything** — the end of a block
    /// that never took the writer, which staged nothing and whose `BEGIN`-time
    /// directory would clobber what other connections committed since. A no-op with no
    /// transaction open.
    pub fn end_unwritten_transaction(&mut self, conn_id: i32) {
        let Some(tx) = self.transactions.remove(&conn_id) else {
            return;
        };
        debug_assert!(
            tx.pending.is_empty() && tx.created_indexes.is_empty() && tx.own_ddl_ops == 0,
            "only a block that staged nothing ends unrestored"
        );
        self.storage.release_snapshot(&tx.snapshot);
    }

    /// Record that connection `conn_id` performed a catalog-allocator DDL
    /// (`create_table`/`drop_table`) inside its active transaction.
    ///
    /// DDL is applied auto-commit even within a transaction and the wrapper's DDL
    /// methods carry no `conn_id`, so the [`StorageAdapter`](crate) calls this right
    /// after routing an in-transaction `create_table`/`drop_table` to attribute it to
    /// this transaction. A no-op if the connection has no active transaction. See
    /// [`commit_transaction`](Self::commit_transaction), whose refused write drops a
    /// batch only when the batch is the whole transaction.
    pub fn note_txn_ddl(&mut self, conn_id: i32) {
        if let Some(tx) = self.transactions.get_mut(&conn_id) {
            tx.own_ddl_ops = tx.own_ddl_ops.saturating_add(1);
        }
    }

    /// Record that connection `conn_id` created index `index_id` inside its active
    /// transaction.
    ///
    /// `create_index` is applied immediately (so a probe within the transaction sees
    /// the new index — read-your-own-writes), so unlike a buffered drop its rollback
    /// is an *undo*: the adapter calls this right after a successful in-transaction
    /// `create_index` so [`rollback_transaction`](Self::rollback_transaction) can
    /// deregister the def and shed its entries. A no-op outside a transaction (an
    /// auto-commit `CREATE INDEX` is durable and needs no undo).
    pub fn note_txn_created_index(&mut self, conn_id: i32, index_id: u32) {
        if let Some(tx) = self.transactions.get_mut(&conn_id) {
            tx.created_indexes.insert(index_id);
        }
    }

    /// Drop a table within a transaction — **the destructive deletion is deferred
    /// to [`commit_transaction`](Self::commit_transaction)** (buffered).
    ///
    /// The name→id mapping is removed *now* (so read-your-own-writes sees the table
    /// gone and a `DROP TABLE t; CREATE TABLE t` recreates without colliding), and
    /// the drop is counted as own catalog DDL (`own_ddl_ops`); a rollback restores
    /// `name_to_id` as it does for the immediate-DDL path. But the table's rows, `SYS_TABLE` entry, and index
    /// entries are **not** touched here; they are recorded into the pending buffer
    /// and deleted at commit. This is what closes the DROP-in-transaction-rollback
    /// row-loss: with the deletion deferred, ROLLBACK discards the buffer and the
    /// table is intact (rows, counter, and indexes all preserved).
    ///
    /// Unlike a fully transactional DDL this does **not** isolate the catalog from
    /// concurrent connections — `name_to_id` (and the server/wasm `Schema`) is still
    /// mutated in place, so a concurrent reader resolves through the uncommitted drop
    /// (the repo's `EDGES.md` row 221): the catalog is not MVCC.
    ///
    /// # Errors
    ///
    /// [`StorageError::TableNotFound`] if `name` is unknown, or
    /// [`StorageError::NoTransaction`] if `conn_id` has no active transaction.
    pub fn drop_table_in_tx(&mut self, conn_id: i32, name: &str) -> Result<()> {
        let table_id = self.get_table_id(name)?;
        // a rollback restores the table, and its rows are then not known untoasted
        self.forget_untoasted(table_id);
        // Remove the name mapping now (RYOW + recreate-same-name) and count this as
        // own catalog DDL.
        self.name_to_id.remove(name);
        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;
        tx.own_ddl_ops = tx.own_ddl_ops.saturating_add(1);
        // Defer the destructive deletion (rows + SYS_TABLE + index entries) to commit.
        tx.pending.drop_table(table_id);
        Ok(())
    }

    /// Drop an index within a transaction — deferred to commit (buffered).
    ///
    /// The index def and its entries are left in place for the duration of the
    /// transaction (so a ROLLBACK restores them alongside the catalog) and recorded
    /// for deletion at [`commit_transaction`](Self::commit_transaction). Covers both
    /// a standalone `DROP INDEX` and the cascade from
    /// [`drop_table_in_tx`](Self::drop_table_in_tx) (`execute_drop_table` sheds a
    /// table's indexes before the table itself). Index DDL touches neither
    /// `name_to_id` nor `next_table_id`, so it is not counted toward `own_ddl_ops`.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoTransaction`] if `conn_id` has no active transaction.
    pub fn drop_index_in_tx(&mut self, conn_id: i32, index_id: u32) -> Result<()> {
        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;
        tx.pending.drop_index(index_id);
        Ok(())
    }

    /// Commit a transaction, atomically applying all buffered writes.
    ///
    /// All buffered changes are applied in one `apply_batch` (a single CoW root
    /// swap), so the commit is atomic — the durable commit point is the
    /// double-meta slot flip in `save_state`, not a write-ahead log. A page write or
    /// `fsync` the device refuses fails a file-backed store
    /// ([`StorageError::DurableWriteFailed`]); [`Self::commit_transaction_discardable`]
    /// is the form that may leave it serving.
    pub fn commit_transaction(&mut self, conn_id: i32) -> Result<()> {
        self.commit_transaction_with(conn_id, false)
    }

    /// [`Self::commit_transaction`] for a transaction whose host can take back every
    /// other effect of it (its catalog). When the transaction also holds no table or
    /// index it created or dropped at this layer, a savepoint rollback not having undone
    /// it — each of which changes this wrapper's own state before or beside its batch,
    /// and which nothing restores once the commit has consumed the transaction — its
    /// batch is the whole of it here, so a
    /// page write or `fsync` the device refuses drops the batch and answers
    /// [`StorageError::WriteRefused`]: the transaction is gone, its rows unwritten, and
    /// the store serves on (`CowBtree::apply_discardable_batch`), whatever another
    /// connection has published meanwhile. Otherwise as `commit_transaction`.
    ///
    /// # Errors
    ///
    /// Every error `commit_transaction` returns, and `WriteRefused` as above.
    pub fn commit_transaction_discardable(&mut self, conn_id: i32) -> Result<()> {
        self.commit_transaction_with(conn_id, true)
    }

    /// [`Self::commit_transaction`]'s body; `discardable` asks for
    /// [`Self::commit_transaction_discardable`]'s answer to a refused write.
    fn commit_transaction_with(&mut self, conn_id: i32, discardable: bool) -> Result<()> {
        let Some(tx) = self.transactions.remove(&conn_id) else {
            // No active transaction - no-op
            return Ok(());
        };

        // Release the snapshot
        self.storage.release_snapshot(&tx.snapshot);

        if tx.pending.is_empty() {
            return Ok(());
        }

        // Build batch changes
        let mut batch = BatchChanges::new();

        // Apply the buffered DDL drops first (deferred from `drop_*_in_tx`).
        //
        // Dropped indexes (standalone + the cascade from a dropped table): delete
        // every entry in each index's band and deregister its def, BEFORE the row
        // and DML work below so neither the dropped-table shed nor the pending-write
        // index maintenance touches a dropped index.
        for &index_id in tx.pending.dropped_indexes() {
            self.delete_index_band(&mut batch, index_id)?;
            self.deregister_index(index_id);
        }

        // Dropped tables: delete all rows and the SYS_TABLE directory entry. Index
        // entries were handled above (`execute_drop_table` records every index into
        // `dropped_indexes` before the table); a still-registered index here is a
        // straggler not in the catalog — shed it too so the tree can't keep dangling
        // entries past the table.
        for &table_id in tx.pending.dropped_tables() {
            if let Some(index_ids) = self.table_indexes.remove(&table_id) {
                for index_id in index_ids {
                    self.delete_index_band(&mut batch, index_id)?;
                    self.index_defs.remove(&index_id);
                }
            }
            let entries = self
                .storage
                .scan_table(table_id)
                .map_err(StorageError::from)?;
            for (key, _values) in entries {
                batch.delete(key);
            }
            batch.delete(BtreeKey::sys_table(table_id));
        }

        // Add row changes to batch. For indexed tables, read the OLD committed
        // value at the transaction's snapshot root so an update/delete removes the
        // stale index entry; the index ops land in the SAME batch (one root swap).
        // Each written table also re-emits its SYS_TABLE directory/counter entry in
        // the same batch, so the row-id counter the txn's inserts bumped rides the
        // one commit root swap / fsync — no separate metadata write.
        for (table_id, changes) in tx.pending.all_changes() {
            let indexed = self.table_has_indexes(*table_id);
            if let Some(name) = self
                .name_to_id
                .iter()
                .find(|&(_, id)| *id == *table_id)
                .map(|(n, _)| n.clone())
            {
                let (sk, sv) = self.sys_table_kv(*table_id, &name);
                batch.insert(sk, sv);
            }
            for (row_id, pending_row) in changes {
                let key = BtreeKey::new(*table_id, *row_id);
                let old_row = if indexed {
                    self.storage
                        .get_at_root(tx.snapshot.root, &key)
                        .ok()
                        .flatten()
                } else {
                    None
                };
                match pending_row {
                    PendingRow::Data(values) => {
                        if indexed {
                            self.emit_index_ops(
                                &mut batch,
                                *table_id,
                                *row_id,
                                old_row.as_deref(),
                                Some(values),
                            )?;
                        }
                        batch.insert(key, values.clone());
                    }
                    PendingRow::Deleted => {
                        if indexed {
                            self.emit_index_ops(
                                &mut batch,
                                *table_id,
                                *row_id,
                                old_row.as_deref(),
                                None,
                            )?;
                        }
                        batch.delete(key);
                    }
                }
            }
        }

        // Apply the batch atomically (one CoW root swap). Droppable on a refused write
        // only when the batch is the whole transaction: DDL changed this wrapper's own
        // state — an eager table or index, a name removed, the defs deregistered above —
        // which a dropped batch would leave standing. A dropped table is counted by
        // `own_ddl_ops`, which `drop_table_in_tx` bumps.
        let only_rows = tx.own_ddl_ops == 0
            && tx.created_indexes.is_empty()
            && tx.pending.dropped_indexes().is_empty();
        if discardable && only_rows {
            self.storage.apply_discardable_batch(batch)
        } else {
            self.storage.apply_batch(batch)
        }
        .map_err(StorageError::from)?;

        // Reclaim each dropped table's row-id counter (its `name_to_id` mapping was
        // already removed by `drop_table_in_tx`; its index defs above).
        for &table_id in tx.pending.dropped_tables() {
            self.remove_row_id_counter(table_id);
        }

        Ok(())
    }

    /// Rollback a transaction, discarding all buffered writes and restoring the
    /// directory and table-id counter it holds snapshots of.
    ///
    /// The **row-id counters are NOT rolled back** — they are monotonic allocators. A
    /// transaction's inserts are buffered (discarded here), never applied, so a
    /// rolled-back row id is simply skipped (a harmless gap; recovery reloads each
    /// table's `next_row_id` from its committed `SYS_TABLE` entry, not from this
    /// in-memory counter).
    ///
    /// **The catalog allocators (`name_to_id` / `next_table_id`) are restored
    /// unconditionally.** DDL is applied auto-commit even inside a transaction (no
    /// buffered `CREATE TABLE`), so the snapshot is the only undo channel, and the
    /// restore is exact because the snapshot is of the state this transaction's own
    /// writes started from: a block's host re-bases it when the block takes the writer
    /// ([`Self::rebase_transaction`]), and an auto-commit statement begins holding it.
    /// A block that never took the writer staged nothing and ends through
    /// [`Self::end_unwritten_transaction`] instead, since its `BEGIN`-time snapshot
    /// is older than what other connections committed. `next_table_id` is restored as
    /// far as no id it would hand out again has a row in the tree
    /// ([`restore_next_table_id`](Self::restore_next_table_id)), and the restore stays
    /// **load-bearing for recovery**: a created-then-rolled-back table's
    /// immediately-applied `SYS_TABLE` is excluded from the `[0, next_table_id)`
    /// recovery scan only by `next_table_id < its id`.
    ///
    /// **In-transaction `DROP TABLE` / `DROP INDEX` are buffered** (deferred to
    /// commit, see [`drop_table_in_tx`](Self::drop_table_in_tx)), so this rollback
    /// has nothing to undo for them at the tree level: discarding the pending buffer
    /// leaves the rows, `SYS_TABLE`, index entries, and defs intact, and the
    /// `name_to_id` restore brings the dropped name back.
    ///
    /// **In-transaction `CREATE INDEX` is undone here** (see
    /// [`undo_created_indexes`](Self::undo_created_indexes)). Unlike a drop it's built
    /// immediately (read-your-own-writes needs the live def + entries), so rollback
    /// sheds each created index's entries and deregisters its def — otherwise a phantom
    /// storage index would outlive the catalog's rollback. This also covers the
    /// auto-created PK/UNIQUE index of an in-transaction `CREATE TABLE`.
    ///
    /// # Errors
    ///
    /// A page the store cannot read or write while shedding a created index or
    /// deleting a taken-back table's directory entry.
    pub fn rollback_transaction(&mut self, conn_id: i32) -> Result<()> {
        let Some(tx) = self.transactions.remove(&conn_id) else {
            // No active transaction - no-op
            return Ok(());
        };

        // Release the snapshot
        self.storage.release_snapshot(&tx.snapshot);

        // Undo any index this transaction created (immediate build for read-your-own-
        // writes, so rollback sheds it).
        self.undo_created_indexes(&tx.created_indexes)?;

        self.name_to_id = tx.name_to_id_snapshot;
        self.restore_next_table_id(tx.next_table_id_snapshot)
    }

    /// Undo the indexes a rolling-back transaction created: shed every entry in each
    /// index's band, then deregister the def. Build-then-apply-then-deregister so a
    /// failed `apply_batch` leaves the defs registered (consistent with the still-
    /// present entries) rather than half-undone.
    fn undo_created_indexes(&mut self, created: &HashSet<u32>) -> Result<()> {
        if created.is_empty() {
            return Ok(());
        }
        let mut batch = BatchChanges::new();
        for &index_id in created {
            self.delete_index_band(&mut batch, index_id)?;
        }
        if !batch.is_empty() {
            self.storage
                .apply_batch(batch)
                .map_err(StorageError::from)?;
        }
        for &index_id in created {
            self.deregister_index(index_id);
        }
        Ok(())
    }

    /// Open a savepoint level on `conn_id`'s transaction.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoTransaction`] if `conn_id` has no active transaction. That
    /// is not reachable from SQL — `SAVEPOINT` outside a block is `25P01` from the
    /// session, which never calls this — but is the right answer for the embed and
    /// `DO`-block callers.
    pub fn savepoint(&mut self, conn_id: i32) -> Result<()> {
        let name_to_id = self.name_to_id.clone();
        let next_table_id = self.next_table_id.load(Ordering::SeqCst) as u32;
        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;
        let frame = tx.snapshot(&name_to_id, next_table_id);
        tx.savepoints.push(frame);
        Ok(())
    }

    /// Discard everything `conn_id`'s transaction wrote since its innermost
    /// savepoint, **keeping** that savepoint (`ROLLBACK TO` is repeatable).
    ///
    /// The catalog allocators are restored to the savepoint's unconditionally, for the
    /// reason [`rollback_transaction`](Self::rollback_transaction) restores them to the
    /// block's: a savepoint taken before the block's first write is re-based when the
    /// block takes the writer ([`Self::rebase_transaction`]), so every savepoint a
    /// written block holds is of state its own writes started from. A block that never
    /// took the writer staged nothing, and its host does not call this.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoTransaction`], [`StorageError::NoSavepoint`] if the stack
    /// is empty, or a page the store cannot read or write.
    pub fn rollback_to_savepoint(&mut self, conn_id: i32) -> Result<()> {
        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;
        let frame = tx.savepoints.last().ok_or(StorageError::NoSavepoint)?;

        // Indexes built since the savepoint are undone the way a whole-block
        // rollback undoes them: they were applied immediately for
        // read-your-own-writes, so the entries must be shed and the defs
        // deregistered. Collected before the pending restore because it reads the
        // live `created_indexes`.
        let undo: HashSet<u32> = tx
            .created_indexes
            .difference(&frame.created_indexes)
            .copied()
            .collect();

        tx.pending = frame.pending.clone();
        tx.pending_index = frame.pending_index.clone();
        tx.probe_scan_tables = frame.probe_scan_tables.clone();
        // The block is live again (`pglet::TransactionSession::rollback_to_savepoint`
        // clears the abort), so the writes just restored resume claiming their keys —
        // which is what 18.6 does, the pre-savepoint rows having belonged to the still
        // live top-level transaction all along.
        tx.claims_suspended = false;
        tx.own_ddl_ops = frame.own_ddl_ops;
        tx.created_indexes = frame.created_indexes.clone();
        let (name_to_id, next_table_id) = (frame.name_to_id.clone(), frame.next_table_id);

        self.undo_created_indexes(&undo)?;
        self.name_to_id = name_to_id;
        self.restore_next_table_id(next_table_id)
    }

    /// Drop `conn_id`'s innermost savepoint, **keeping** everything written since
    /// it — the sub-work merges into the enclosing level, so nothing is restored.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoTransaction`], or [`StorageError::NoSavepoint`] if the
    /// stack is empty.
    pub fn release_savepoint(&mut self, conn_id: i32) -> Result<()> {
        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;
        tx.savepoints.pop().ok_or(StorageError::NoSavepoint)?;
        Ok(())
    }

    /// How many savepoints `conn_id`'s transaction holds — `0` with no transaction
    /// at all. The lockstep check `pglet::TransactionSession` asserts against its
    /// own frame count.
    pub fn savepoint_depth(&self, conn_id: i32) -> usize {
        self.transactions
            .get(&conn_id)
            .map_or(0, |tx| tx.savepoints.len())
    }

    /// Check if a connection has an active transaction.
    pub fn has_transaction(&self, conn_id: i32) -> bool {
        self.transactions.contains_key(&conn_id)
    }

    /// Get the snapshot version for a transaction (for debugging/testing).
    pub fn transaction_snapshot_version(&self, conn_id: i32) -> Option<u64> {
        self.transactions
            .get(&conn_id)
            .map(|tx| tx.snapshot.version)
    }

    /// The B-tree index keys `row` contributes, as `(index_id, encoded_key)` — the
    /// maintenance side of the per-transaction
    /// [`pending_index`](BtreeTransactionState::pending_index), for every write that
    /// moves it: an insert's and an update's new keys directly, and the keys an update
    /// or delete retracts through
    /// [`staged_btree_index_keys`](Self::staged_btree_index_keys). Skips GIN
    /// indexes (probed via `gin_lookup_in_tx`, never `index_lookup_in_tx`),
    /// partial-excluded rows, and NULL keys (not indexed under NULLS DISTINCT), so
    /// the stored set matches exactly what the committed index holds and what
    /// `index_lookup_in_tx` probes for.
    fn pending_btree_index_keys(
        &self,
        table_id: u32,
        row: &[Value],
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut out = Vec::new();
        // The table's indexes in id (creation) order, 18.6's, so the index a size
        // refusal names does not depend on a map's iteration order.
        let Some(index_ids) = self.table_indexes.get(&table_id) else {
            return Ok(out);
        };
        for &index_id in index_ids {
            let Some(def) = self.index_defs.get(&index_id) else {
                continue;
            };
            if !def.row_in_index(row)? {
                continue;
            }
            match def.keying.method {
                // The entry the commit will write, sized as it will be sized there.
                IndexMethod::Btree => {
                    out.extend(
                        encode_row_index_entries(def, row)?
                            .into_iter()
                            .map(|key| (index_id, key)),
                    );
                }
                // Not in the pending index, but refused here rather than at `COMMIT`.
                IndexMethod::Gin => {
                    encode_row_index_entries(def, row)?;
                }
            }
        }
        Ok(out)
    }

    /// The B-tree index keys this connection's **staged** image of `row_id` currently
    /// contributes — empty when nothing of the row is staged.
    ///
    /// Empty is the load-bearing answer: a row whose only image is the committed one
    /// has no entry in the pending index, so a write that replaces it must retract
    /// nothing there. Reading the committed image here instead would retract keys the
    /// pending index never held.
    ///
    /// # Errors
    ///
    /// Propagates a predicate-evaluation or key-encoding failure from the staged row.
    fn staged_btree_index_keys(
        &self,
        conn_id: i32,
        table_id: u32,
        row_id: RowId,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let Some(tx) = self.transactions.get(&conn_id) else {
            return Ok(Vec::new());
        };
        match tx.pending.get(table_id, row_id) {
            Some(PendingRow::Data(row)) => self.pending_btree_index_keys(table_id, row),
            Some(PendingRow::Deleted) | None => Ok(Vec::new()),
        }
    }

    /// Insert a row within a transaction.
    pub fn insert_in_tx(&mut self, conn_id: i32, table: &str, row: Vec<Value>) -> Result<RowId> {
        let table_id = self.get_table_id(table)?;
        let row_id = self.allocate_row_id(table_id);
        // Compute the fast-path index keys before the mutable transaction borrow
        // (encoding reads `index_defs`). An encode/predicate failure here just falls
        // the table back to the scan path — the same error then surfaces at the
        // probe or at commit, exactly as before, so the insert itself isn't failed.
        let index_keys = self.pending_btree_index_keys(table_id, &row);
        // An entry too large to hold is the statement's refusal, not a reason to fall
        // back to the scan path: the commit would refuse the same entry.
        if let Err(StorageError::IndexEntry(e)) = index_keys {
            return Err(StorageError::IndexEntry(e));
        }

        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;

        tx.pending.insert(table_id, row_id, row);
        match index_keys {
            // A freshly allocated row id stages nothing yet, so there is nothing to
            // retract.
            Ok(keys) => tx.restage_pending_index(table_id, row_id, &[], keys),
            Err(_) => {
                tx.probe_scan_tables.insert(table_id);
            }
        }
        Ok(row_id)
    }

    /// Scan a table within a transaction.
    ///
    /// Returns committed rows at the transaction's snapshot merged with
    /// uncommitted writes from the transaction's pending buffer.
    pub fn scan_in_tx(&self, conn_id: i32, table: &str) -> Result<Vec<(RowId, Vec<Value>)>> {
        let table_id = self.get_table_id(table)?;
        self.table_scans_in_tx.fetch_add(1, Ordering::Relaxed);

        let tx = self
            .transactions
            .get(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;

        // Check if table was dropped in this transaction
        if tx.pending.is_table_dropped(table_id) {
            return Ok(Vec::new());
        }

        // Get committed rows from snapshot
        let committed = self
            .storage
            .scan_table_at_root(tx.snapshot.root, table_id)
            .map_err(StorageError::from)?;

        // Merge with pending changes
        let mut result: BTreeMap<RowId, Vec<Value>> = BTreeMap::new();

        // Add committed rows
        for (key, values) in committed {
            result.insert(key.row_id(), values);
        }

        // Apply pending changes
        if let Some(changes) = tx.pending.table_changes(table_id) {
            for (row_id, pending_row) in changes {
                match pending_row {
                    PendingRow::Data(values) => {
                        result.insert(*row_id, values.clone());
                    }
                    PendingRow::Deleted => {
                        result.remove(row_id);
                    }
                }
            }
        }

        Ok(result.into_iter().collect())
    }

    /// Equality probe within a transaction — **read-your-own-writes correct**.
    ///
    /// The committed index is only maintained at COMMIT (`apply_batch`), so a
    /// current-root index probe alone would miss a row inserted in this
    /// transaction, return a row this transaction deleted, or report a stale value
    /// for a row this transaction updated. Correctness is restored by probing the
    /// committed index at the snapshot root and then merging **only the pending
    /// buffer** — dropping committed candidates the transaction has modified and
    /// adding pending inserts/updates that match — rather than scanning the whole
    /// table. The result is identical to a pending-merged `Filter(Scan)` but costs
    /// O(log n + pending) instead of O(table_size); the former made an enforced
    /// PK/UNIQUE insert scan the whole table, so a batch was O(rows × table_size).
    ///
    /// Comparison is on the index's own key encoding (`encode_index_key`), so it
    /// matches the index's notion of equality exactly. A NULL probe returns no
    /// rows under default `NULLS DISTINCT`, but conflicts under `NULLS NOT
    /// DISTINCT` (two NULLs collide). For a **partial** index, only rows satisfying
    /// the predicate are counted (they alone are in the index). Result row IDs are
    /// sorted, matching the committed probe's order.
    ///
    /// # Errors
    ///
    /// Returns `IndexNotFound` if the index isn't registered, `NoTransaction` if
    /// the connection has no active transaction, or `Unindexable` if a value can't
    /// be index-encoded.
    pub fn index_lookup_in_tx(
        &self,
        conn_id: i32,
        index_id: u32,
        values: &[Value],
    ) -> Result<Vec<RowId>> {
        // Clone the def's fields so the borrows below don't conflict, and so
        // `row_in_index` / `encode_row_index_key` (a method / free fn over the def)
        // stay callable while `self` is otherwise borrowed.
        let def = self
            .index_defs
            .get(&index_id)
            .ok_or(StorageError::IndexNotFound(index_id))?
            .clone();
        // NULL key columns: no match under default NULLS DISTINCT; encoded (so two
        // NULLs collide) under NULLS NOT DISTINCT.
        let Some(target) = encode_probe_key(values, def.nulls_not_distinct())? else {
            return Ok(Vec::new());
        };

        // The def carries its table's internal id directly — no name lookup needed.
        let table_id = def.table_id;
        let tx = self
            .transactions
            .get(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;
        let table_changes = tx.pending.table_changes(table_id);

        // Probe the **committed index at the snapshot root** — O(log n), not a full
        // table scan. The old implementation ran `scan_in_tx` (scan every committed
        // row + re-encode each row's index key) per probe, so an enforced-unique
        // insert was O(table_size); a batch of them O(rows × table_size) — the
        // dominant cost of the enforcing write path. Read-your-own-writes
        // correctness is preserved by merging *only* the pending buffer, not the
        // whole table:
        //   (a) drop any committed candidate this transaction has since modified
        //       (delete or update) — its current value is re-evaluated below, and
        //   (b) add pending inserts/updates whose current value matches the key.
        // The result set is identical to the old full scan, at O(log n + pending).
        let committed = self
            .storage
            .index_scan_at_root(tx.snapshot.root, index_id, &target)
            .map_err(StorageError::from)?;
        let mut ids: Vec<RowId> = committed
            .into_iter()
            .filter(|id| table_changes.is_none_or(|c| !c.contains_key(id)))
            .collect();

        // The read-your-own-writes half — the pending matches, through the one
        // buffer-holds-this-key derivation the cross-connection claim walk also reads
        // (`ClaimedWrites::matching_rows`, which picks the O(1) pending index or the
        // correct O(pending) scan).
        ids.extend(tx.live_writes().matching_rows(&def, index_id, &target)?);

        ids.sort_unstable();
        Ok(ids)
    }

    /// Whether a connection **other than** `conn_id` holds an uncommitted write whose
    /// key tuple in `index_id` is `values` — the cross-connection half of a uniqueness
    /// decision.
    ///
    /// [`index_lookup_in_tx`](Self::index_lookup_in_tx) and the committed-root probe
    /// both answer only for the asking connection, so without a wait an open block's
    /// staged row would be invisible to every other writer's `23505` gate and two
    /// connections would commit the same key. This walks the *other* live transactions'
    /// buffers and reports whether any claims the key. Every host lets one block stage
    /// writes at a time (the writer, taken before a statement stages anything), so under
    /// such a host this finds nothing; it is kept as the storage layer's own guard, and a
    /// hit is counted (`cross_connection_hits`).
    ///
    /// The claim is derived from the buffered rows themselves — never from a second
    /// registry of keys — so it cannot drift from what those writes will actually
    /// insert, and it is released by the writes ceasing to exist: `COMMIT` and
    /// `ROLLBACK` drop the whole transaction state, `ROLLBACK TO SAVEPOINT` restores
    /// the buffer from the savepoint's snapshot, and a client disconnect rolls the
    /// block back. Only the aborted-block release needs a mark of its own
    /// ([`BtreeTransactionState::claims_suspended`]).
    ///
    /// The `index_id` is the caller's: this asks whether that index's key is claimed
    /// and never re-decides *which* indexes enforce uniqueness, so the SQL layer's
    /// enforced set stays the one such predicate.
    ///
    /// # Errors
    ///
    /// [`StorageError::IndexNotFound`] if the index isn't registered, or a
    /// predicate-evaluation / key-encoding failure from a foreign buffer's row.
    pub fn unique_key_claimed_elsewhere(
        &self,
        conn_id: i32,
        index_id: u32,
        values: &[Value],
    ) -> Result<bool> {
        let def = self
            .index_defs
            .get(&index_id)
            .ok_or(StorageError::IndexNotFound(index_id))?;
        // NULL key columns: no claim under default NULLS DISTINCT (such a row is never
        // a uniqueness candidate); encoded, so two NULLs collide, under NULLS NOT
        // DISTINCT. The same `encode_probe_key` call the self-probe makes.
        let Some(target) = encode_probe_key(values, def.nulls_not_distinct())? else {
            return Ok(false);
        };
        for (&other_conn, tx) in &self.transactions {
            if other_conn == conn_id {
                continue;
            }
            let Some(claimed) = tx.claimed_writes() else {
                continue;
            };
            if !claimed.matching_rows(def, index_id, &target)?.is_empty() {
                // Unreachable while one block at a time stages writes — the writer every
                // host takes before a statement stages anything — so counted, not
                // relied on: see `cross_connection_hits`.
                self.cross_connection_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Suspend `conn_id`'s unique-key claims because its transaction block just went
    /// aborted; a no-op when it has no open block.
    ///
    /// The one release point a claim does not get from the writes themselves — see
    /// [`BtreeTransactionState::claims_suspended`] for the 18.6 behaviour it mirrors
    /// and [`rollback_to_savepoint`](Self::rollback_to_savepoint) for the resume.
    pub fn suspend_claims(&mut self, conn_id: i32) {
        if let Some(tx) = self.transactions.get_mut(&conn_id) {
            tx.claims_suspended = true;
        }
    }

    /// Containment (`@>`) probe within a transaction — **read-your-own-writes
    /// correct**.
    ///
    /// The GIN counterpart to [`index_lookup_in_tx`](Self::index_lookup_in_tx): the
    /// committed index isn't maintained until COMMIT, so a current-root posting
    /// intersection would miss this transaction's pending inserts (or surface
    /// deleted/updated rows). It computes the candidate set from
    /// [`scan_in_tx`](Self::scan_in_tx) (pending-merged) instead — a row is a
    /// candidate when it's in the (partial) index AND its value's decomposition holds
    /// **every** query key (`gin_contains_keys`, the same superset the committed
    /// posting-list intersection returns). The caller rechecks with the real `@>`.
    ///
    /// # Errors
    ///
    /// Returns `IndexNotFound` if the index isn't registered, or `NoTransaction` if
    /// the connection has no active transaction.
    pub fn gin_lookup_in_tx(
        &self,
        conn_id: i32,
        index_id: u32,
        keys: &[Value],
    ) -> Result<Vec<RowId>> {
        let def = self
            .index_defs
            .get(&index_id)
            .ok_or(StorageError::IndexNotFound(index_id))?
            .clone();
        // `x @> {}` matches everything (the caller full-scans instead); never here.
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut ids: Vec<RowId> = Vec::new();
        for (id, row) in self.scan_in_tx(conn_id, &def.table_name)? {
            // A row not in the (partial) index can't be a match.
            if !def.row_in_index(&row)? {
                continue;
            }
            // The single GIN key value (the array/JSONB column); contains all keys?
            let values = def.keying.key_values(&row)?;
            if values
                .first()
                .is_some_and(|value| gin_contains_keys(value, keys))
            {
                ids.push(id);
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    /// Ordered range probe within a transaction — **read-your-own-writes correct**.
    ///
    /// The range counterpart to [`index_lookup_in_tx`](Self::index_lookup_in_tx):
    /// the committed index isn't maintained until COMMIT, so a current-root range
    /// scan would miss this transaction's pending inserts, surface deleted rows, or
    /// use a stale value for an updated row. To stay byte-for-byte identical to what
    /// `Filter(Scan)` + sort would return, it computes the result from
    /// [`scan_in_tx`](Self::scan_in_tx) (pending-merged) rather than the committed
    /// index:
    ///
    /// 1. build the `[start, end)` `BtreeKey` bounds from the value bounds — the bounds
    ///    the committed scan walks (`index_range_bounds_for`),
    /// 2. for each pending-merged row in the (partial) index, build its index key
    ///    (`index(index_id, encode_row_index_key(row), row_id)`) and keep it when it
    ///    falls in `[start, end)` — the same half-open `BtreeKey` order the committed
    ///    scan uses, so a NULL key (which has no encoding under default `NULLS
    ///    DISTINCT`, and one below `start` under `NULLS NOT DISTINCT`) and an
    ///    out-of-range row drop out identically,
    /// 3. sort by the index key (`scan_in_tx` yields row-id order, not value order),
    /// 4. apply `ascending` (reverse for descending) and `limit`.
    ///
    /// Step 2 reads every row of the table, so a range or an `ORDER BY … LIMIT` pushdown
    /// inside an open block costs a whole-table read where the autocommit path walks the
    /// committed index.
    ///
    /// # Errors
    ///
    /// Returns `IndexNotFound` if the index isn't registered, `NoTransaction` if the
    /// connection has no active transaction, or `Unindexable` if a value can't be
    /// index-encoded.
    // TODO(perf): merge the committed index's range with the transaction's pending rows
    // instead of re-encoding the whole table — every range read inside a block pays it
    // (a test harness that wraps each test in a transaction does), and it is why a DML
    // target probe takes equality keys only.
    pub fn index_range_lookup_in_tx(
        &self,
        conn_id: i32,
        index_id: u32,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<RowId>> {
        let def = self
            .index_defs
            .get(&index_id)
            .ok_or(StorageError::IndexNotFound(index_id))?
            .clone();
        let (start, end) = index_range_bounds_for(index_id, lower, upper)?;

        // `scan_in_tx` is read-your-own-writes but yields row-id order; collect the
        // in-range rows as (index key, row_id) so we can sort by value (key) order.
        let mut hits: Vec<(BtreeKey, RowId)> = Vec::new();
        for (id, row) in self.scan_in_tx(conn_id, &def.table_name)? {
            // A row not in the (partial) index can't be a match.
            if !def.row_in_index(&row)? {
                continue;
            }
            // A row whose key doesn't encode (NULL under default NULLS DISTINCT) has no
            // entry for the committed scan to find either. Under NULLS NOT DISTINCT a
            // NULL key does encode, below `start`, and the comparison leaves it out.
            let Some(bytes) = encode_row_index_key(&def, &row)? else {
                continue;
            };
            let key = BtreeKey::index(index_id, &bytes, id);
            if key >= start && key < end {
                hits.push((key, id));
            }
        }
        // Index-key order (the committed scan's order); ties already break by row_id
        // since the key carries it.
        hits.sort_by(|a, b| a.0.cmp(&b.0));
        let mut ids: Vec<RowId> = hits.into_iter().map(|(_, id)| id).collect();
        if !ascending {
            ids.reverse();
        }
        if let Some(n) = limit {
            ids.truncate(n);
        }
        Ok(ids)
    }

    /// Get a row within a transaction.
    pub fn get_in_tx(&self, conn_id: i32, table: &str, id: RowId) -> Result<Option<Vec<Value>>> {
        let table_id = self.get_table_id(table)?;

        let tx = self
            .transactions
            .get(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;

        // Check if table was dropped
        if tx.pending.is_table_dropped(table_id) {
            return Ok(None);
        }

        // Check pending writes first (read-your-own-writes)
        if let Some(pending_row) = tx.pending.get(table_id, id) {
            return match pending_row {
                PendingRow::Data(data) => Ok(Some(data.clone())),
                PendingRow::Deleted => Ok(None),
            };
        }

        // Fall back to committed storage at snapshot. A genuinely-absent row is
        // `Ok(None)`; only a real corruption/IO error propagates (rather than
        // being silently flattened to "not found").
        let key = BtreeKey::new(table_id, id);
        self.storage
            .get_at_root(tx.snapshot.root, &key)
            .map_err(StorageError::from)
    }

    /// Delete a row within a transaction.
    pub fn delete_in_tx(
        &mut self,
        conn_id: i32,
        table: &str,
        id: RowId,
    ) -> Result<Option<Vec<Value>>> {
        let table_id = self.get_table_id(table)?;

        // Get old row data first (for RETURNING support)
        let old_row = self.get_in_tx(conn_id, table, id)?;

        if old_row.is_some() {
            // The staged image's keys, before the mutable borrow: a delete retracts
            // them and adds nothing. A committed-only row staged nothing, so this is
            // empty and the pending index is untouched — the committed side already
            // drops a candidate this transaction has modified.
            let staged_keys = self.staged_btree_index_keys(conn_id, table_id, id);
            let tx = self
                .transactions
                .get_mut(&conn_id)
                .ok_or(StorageError::NoTransaction(conn_id))?;
            tx.pending.delete(table_id, id);
            match staged_keys {
                Ok(old) => tx.restage_pending_index(table_id, id, &old, Vec::new()),
                Err(_) => {
                    tx.probe_scan_tables.insert(table_id);
                }
            }
        }

        Ok(old_row)
    }

    /// Update a row within a transaction.
    pub fn update_in_tx(
        &mut self,
        conn_id: i32,
        table: &str,
        id: RowId,
        new_row: Vec<Value>,
    ) -> Result<Option<Vec<Value>>> {
        let table_id = self.get_table_id(table)?;

        // Get old row data first (for RETURNING support)
        let old_row = self.get_in_tx(conn_id, table, id)?;

        if old_row.is_some() {
            // Both key sets before the mutable borrow (encoding reads `index_defs`):
            // the staged image's, to retract, and the new row's, to add. Either failing
            // routes the table to the scan fallback rather than leaving the pending
            // index half-maintained.
            let staged_keys = self.staged_btree_index_keys(conn_id, table_id, id);
            let new_keys = self.pending_btree_index_keys(table_id, &new_row);
            // An entry too large to hold is the statement's refusal (see `insert_in_tx`).
            if let Err(StorageError::IndexEntry(e)) = new_keys {
                return Err(StorageError::IndexEntry(e));
            }
            let tx = self
                .transactions
                .get_mut(&conn_id)
                .ok_or(StorageError::NoTransaction(conn_id))?;
            tx.pending.update(table_id, id, new_row);
            match (staged_keys, new_keys) {
                (Ok(old), Ok(new)) => tx.restage_pending_index(table_id, id, &old, new),
                _ => {
                    tx.probe_scan_tables.insert(table_id);
                }
            }
        }

        Ok(old_row)
    }

    /// Add a column to all rows within a transaction.
    // `default` is cloned into each existing row below; taking `&Value` would only move the
    // needless-by-value lint onto the `add_column` forwarders (server + wasm) without removing a clone.
    #[allow(clippy::needless_pass_by_value)]
    pub fn add_column_in_tx(&mut self, conn_id: i32, table: &str, default: Value) -> Result<()> {
        let table_id = self.get_table_id(table)?;

        // Get all existing rows (committed + pending)
        let rows = self.scan_in_tx(conn_id, table)?;

        if rows.is_empty() {
            return Ok(());
        }
        // `add_column`'s reason: 18.6 reads these rows' default from `attmissingval`
        if !default.is_null() {
            self.forget_untoasted(table_id);
        }

        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;

        // Column DDL re-writes every row: route this table's probes to the scan
        // fallback for the rest of the transaction.
        tx.probe_scan_tables.insert(table_id);
        // Update each row with the new column
        for (row_id, mut row) in rows {
            row.push(default.clone());
            tx.pending.update(table_id, row_id, row);
        }

        Ok(())
    }

    /// Drop a column from all rows within a transaction.
    pub fn drop_column_in_tx(
        &mut self,
        conn_id: i32,
        table: &str,
        column_index: usize,
    ) -> Result<()> {
        let table_id = self.get_table_id(table)?;
        // `drop_column`'s reason: 18.6's rows keep the dropped column
        self.forget_untoasted(table_id);

        // Get all existing rows (committed + pending)
        let rows = self.scan_in_tx(conn_id, table)?;

        if rows.is_empty() {
            return Ok(());
        }

        let tx = self
            .transactions
            .get_mut(&conn_id)
            .ok_or(StorageError::NoTransaction(conn_id))?;

        // Column DDL re-writes every row: route this table's probes to the scan
        // fallback for the rest of the transaction.
        tx.probe_scan_tables.insert(table_id);
        // Update each row without the dropped column
        for (row_id, mut row) in rows {
            if column_index < row.len() {
                row.remove(column_index);
                tx.pending.update(table_id, row_id, row);
            }
        }

        Ok(())
    }
}
