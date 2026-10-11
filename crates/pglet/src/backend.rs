//! Storage backend enum and executor adapters.
//!
//! `StorageBackend` is the `Storage` carrier the executor is fed: it unifies the
//! in-memory and B+tree backends behind one type and exposes the transaction
//! lifecycle (`begin`/`commit`/`rollback`/`save_state`). The facade owns it so
//! both the embed API (`Database`) and any consumer that adds its own
//! concurrency layer (e.g. the native server) share a single definition.

use pglet_core::{Error, IndexSpec, RowId, Storage};
use pglet_sql::catalog::Schema;
use pglet_sql::types::Value;
#[cfg(feature = "memory_storage")]
use pglet_storage::MemoryStorage;
use pglet_storage::btree::{BtreeSnapshot, BtreeStorageWrapper, WriteFailure};
use std::sync::Arc;

/// Storage backend type.
pub enum StorageBackend {
    /// In-memory storage (supports transactions via clone). Behind the
    /// `memory_storage` feature, which the wasm build leaves off: its bindings run
    /// the B+tree only, and a variant it never builds would still cost it every arm
    /// that dispatches to one.
    #[cfg(feature = "memory_storage")]
    Memory(MemoryStorage),
    /// B+tree storage (`CoW`, single-writer with MVCC).
    Btree(BtreeStorageWrapper),
}

impl std::fmt::Debug for StorageBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => f.write_str("StorageBackend::Memory"),
            Self::Btree(b) => f.debug_tuple("StorageBackend::Btree").field(b).finish(),
        }
    }
}

impl StorageBackend {
    /// The store's record of a write or `fsync` it could not complete — a handle that
    /// reads without this backend, for a host whose storage sits behind a lock.
    /// `None` for a backend that persists nothing.
    #[must_use]
    pub fn write_failure(&self) -> Option<WriteFailure> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => None,
            Self::Btree(btree) => btree.inner().store().write_failure().cloned(),
        }
    }

    /// The tree version the store's durable state holds — what a reopen adopts — or
    /// `None` for a backend that persists nothing. It moves only when a save's slot is
    /// durable, so two reads that differ bracket a save.
    #[must_use]
    pub fn durable_version(&self) -> Option<u64> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => None,
            Self::Btree(btree) => btree.inner().store().durable_version(),
        }
    }

    /// Refuse work once the store has failed a write: the in-memory state may then
    /// hold a commit the durable state does not, and only a reopen reads the durable
    /// state alone.
    ///
    /// # Errors
    ///
    /// [`Error::DatabaseMustBeReopened`] (`57P02`) carrying the failure's reason.
    pub fn check_writable(&self) -> Result<(), Error> {
        let failed = match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => None,
            Self::Btree(btree) => btree
                .inner()
                .store()
                .write_failure()
                .and_then(WriteFailure::get),
        };
        failed.map_or(Ok(()), |reason| {
            Err(Error::DatabaseMustBeReopened(reason.to_string()))
        })
    }

    /// Begin a transaction.
    ///
    /// For Btree storage, `conn_id` identifies the connection starting the transaction.
    ///
    /// # Errors
    ///
    /// The storage error, and a refusal on a B+tree private to another connection
    /// (`BtreeStorageWrapper::for_one_connection`).
    pub fn begin(&mut self, conn_id: i32) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => Ok(()), // Memory uses snapshot in the caller
            // `0A000`: what the store does not support is a second connection, which no
            // retry changes — not a lock another session will release.
            Self::Btree(btree) if btree.only_connection().is_some_and(|only| only != conn_id) => {
                Err(Error::NotImplemented(
                    "a transaction on a connection other than the one this database serves"
                        .to_owned(),
                ))
            }
            Self::Btree(btree) => btree.begin_transaction(conn_id).map_err(Error::from),
        }
    }

    /// Commit a transaction.
    ///
    /// For Btree storage, `conn_id` identifies the connection committing.
    pub fn commit(&mut self, conn_id: i32) -> pglet_storage::Result<()> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => {
                // Memory uses the snapshot in the caller; the commit is only counted.
                mem.note_commit();
                Ok(())
            }
            Self::Btree(btree) => btree.commit_transaction(conn_id),
        }
    }

    /// Commit a transaction whose host can take back every other effect of it — an
    /// auto-commit statement of no catalog-writing kind
    /// ([`crate::autocommit_is_discardable`]), or a block, whose rollback restores its
    /// catalog — so a page write or `fsync` the device refuses drops the transaction and
    /// answers
    /// `StorageError::WriteRefused`, leaving the store serving
    /// (`BtreeStorageWrapper::commit_transaction_discardable`). Otherwise as
    /// [`Self::commit`].
    pub fn commit_discardable(&mut self, conn_id: i32) -> pglet_storage::Result<()> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => {
                mem.note_commit();
                Ok(())
            }
            Self::Btree(btree) => btree.commit_transaction_discardable(conn_id),
        }
    }

    /// Rollback a transaction, restoring the catalog allocators it snapshotted — the end
    /// of an auto-commit statement's bracket, or of a block that took the writer.
    ///
    /// For Btree storage, `conn_id` identifies the connection rolling back.
    pub fn rollback(&mut self, conn_id: i32) -> pglet_storage::Result<()> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => Ok(()), // Memory uses snapshot in the caller
            Self::Btree(btree) => btree.rollback_transaction(conn_id),
        }
    }

    /// End `conn_id`'s transaction restoring nothing — a block that never took the
    /// writer, which staged nothing and whose `BEGIN`-time snapshots other connections'
    /// commits have since moved past (`BtreeStorageWrapper::end_unwritten_transaction`).
    /// A no-op for memory storage, which holds no per-connection state.
    pub fn end_unwritten(&mut self, conn_id: i32) {
        if let Self::Btree(btree) = self {
            btree.end_unwritten_transaction(conn_id);
        }
    }

    /// Re-base `conn_id`'s transaction on the state committed now — what a block's
    /// storage does when the block takes the writer: the B+tree's snapshot, directory
    /// and table-id counter, for the block and each savepoint
    /// (`BtreeStorageWrapper::rebase_transaction`); the memory store's savepoint levels
    /// (`MemoryStorage::rebase_savepoints`). The block's own `BEGIN` clone of the memory
    /// store is the session's, which re-takes it.
    pub fn rebase(&mut self, conn_id: i32) {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => mem.rebase_savepoints(conn_id),
            Self::Btree(btree) => btree.rebase_transaction(conn_id),
        }
    }

    /// A stamp of what has committed on this store: two reads that differ bracket a
    /// commit. The B+tree's is its published tree version, which moves with every
    /// non-empty batch — a committed write, a committed or rolled-back table creation or
    /// drop, a catalog write ([`Self::persists_catalog`]) — and not with a commit that
    /// changed no row and wrote no catalog; memory storage's counts its
    /// commits, empty ones included. What a `REPEATABLE READ` block compares at its first
    /// write against the stamp of its snapshot (`TransactionSession::take_writer`).
    #[must_use]
    pub fn commit_stamp(&self) -> u64 {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => mem.commit_count(),
            Self::Btree(btree) => btree.inner().current_version(),
        }
    }

    /// Suspend the unique-key claims `conn_id`'s staged writes hold against other
    /// connections, because its transaction block just went aborted.
    ///
    /// A no-op on memory storage, which stages no cross-connection pending writes and
    /// therefore has no claim to hold; see
    /// `pglet_storage::BtreeStorageWrapper::suspend_claims` for the rule and the 18.6
    /// measurements behind it.
    pub fn suspend_claims(&mut self, conn_id: i32) {
        if let Self::Btree(btree) = self {
            btree.suspend_claims(conn_id);
        }
    }

    /// Whether `conn_id` has an active explicit transaction.
    ///
    /// Memory storage has no per-connection transaction state at this layer
    /// (the host snapshots externally), so it always reports `false`.
    pub fn has_transaction(&self, conn_id: i32) -> bool {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => false,
            Self::Btree(btree) => btree.has_transaction(conn_id),
        }
    }

    /// Whether a commit must hand [`Self::save_state`] the catalog's encoding — whether
    /// anything reads what the catalog write leaves.
    ///
    /// Two things do. A store with durable state reads the catalog back on reopen. And a
    /// catalog write moves the B+tree's version, which is [`Self::commit_stamp`], so a
    /// commit that changes the catalog and no row — a `nextval`, a `CREATE SEQUENCE` —
    /// moves the stamp a `REPEATABLE READ` block on another connection compares at its
    /// first write. A B+tree built for one connection
    /// (`BtreeStorageWrapper::for_one_connection`) has neither: it is in memory, so no
    /// reopen, and no other connection's block. Memory storage keeps nothing, and counts
    /// its commits instead.
    pub const fn persists_catalog(&self) -> bool {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => false,
            Self::Btree(btree) => btree.only_connection().is_none(),
        }
    }

    /// The catalog bytes the store's durable state holds
    /// (`BtreeStorageWrapper::persisted_catalog`: empty before a save wrote one, and the
    /// same allocation until a save writes another), or `None` for a backend with no
    /// durable state — memory storage, and the in-memory B+tree, whose catalog copy no
    /// reopen can read.
    ///
    /// # Errors
    ///
    /// The B+tree's poisoned-lock error.
    pub fn persisted_catalog(&self) -> pglet_storage::Result<Option<Arc<Vec<u8>>>> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => Ok(None),
            Self::Btree(btree) if btree.inner().store().durable_version().is_none() => Ok(None),
            Self::Btree(btree) => btree.persisted_catalog().map(Some),
        }
    }

    /// Save state to persistent storage.
    ///
    /// For Btree storage, this persists all metadata including schema.
    /// For Memory storage, this is a no-op.
    pub fn save_state(&self, schema_bytes: Vec<u8>) -> pglet_storage::Result<()> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => Ok(()),
            Self::Btree(btree) => btree.commit_metadata(schema_bytes),
        }
    }

    /// Flush buffered page data to the underlying store (the data-page fsync that
    /// precedes a metadata write). A no-op for memory storage, which has no store.
    pub fn flush(&self) -> pglet_storage::Result<()> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => Ok(()),
            Self::Btree(btree) => btree
                .inner()
                .flush()
                .map_err(pglet_storage::StorageError::from),
        }
    }

    /// Open a savepoint level for `conn_id`.
    ///
    /// The two backends stage it differently and neither is the other's fallback:
    /// the B+tree clones this transaction's *pending writes*, while memory — which
    /// buffers nothing — clones the whole store, the same channel its `BEGIN`
    /// snapshot uses, onto `conn_id`'s own stack. Memory therefore needs no open
    /// transaction, which is also what makes a `DO` outside a block work there.
    ///
    /// # Errors
    ///
    /// The backend's error if it cannot stage one — for the B+tree, no active
    /// transaction on `conn_id`.
    pub fn savepoint(&mut self, conn_id: i32) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => {
                mem.savepoint_for(conn_id);
                Ok(())
            }
            Self::Btree(btree) => btree.savepoint(conn_id).map_err(Into::into),
        }
    }

    /// Discard everything written since `conn_id`'s innermost savepoint, keeping
    /// the savepoint (`ROLLBACK TO` is repeatable).
    ///
    /// # Errors
    ///
    /// The backend's error: no active transaction, no open savepoint, or a page the
    /// store cannot read or write.
    pub fn rollback_to_savepoint(&mut self, conn_id: i32) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => mem.rollback_to_savepoint_for(conn_id).map_err(Into::into),
            Self::Btree(btree) => btree.rollback_to_savepoint(conn_id).map_err(Into::into),
        }
    }

    /// Drop `conn_id`'s innermost savepoint, keeping everything written since it.
    ///
    /// # Errors
    ///
    /// The backend's error: no active transaction, or no open savepoint.
    pub fn release_savepoint(&mut self, conn_id: i32) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => mem.release_savepoint_for(conn_id).map_err(Into::into),
            Self::Btree(btree) => btree.release_savepoint(conn_id).map_err(Into::into),
        }
    }

    /// Drop every savepoint level `conn_id` holds — what the **end of a transaction
    /// block** does, and only that.
    ///
    /// It is deliberately *not* folded into [`Self::commit`] / [`Self::rollback`],
    /// which look like the same moment and are not: on the memory backend every
    /// auto-commit write is bracketed by that pair (`has_transaction` is always false
    /// there, so the implicit per-statement transaction always opens), so clearing
    /// from them wipes the savepoints of the enclosing *explicit* block — the mirror
    /// of the bug this method fixes, and reached by the very next statement.
    ///
    /// A no-op for the B+tree, whose savepoints live inside the per-connection state
    /// its own `commit`/`rollback` removes wholesale.
    pub fn discard_savepoints(&mut self, conn_id: i32) {
        #[cfg(feature = "memory_storage")]
        if let Self::Memory(mem) = self {
            mem.clear_savepoints(conn_id);
        }
        #[cfg(not(feature = "memory_storage"))]
        let _ = conn_id;
    }

    /// How many savepoint levels are open for `conn_id` — the backend's own count,
    /// which `TransactionSession` asserts its frame stack against.
    #[must_use]
    pub fn savepoint_depth(&self, conn_id: i32) -> usize {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(mem) => mem.savepoint_depth(conn_id),
            Self::Btree(btree) => btree.savepoint_depth(conn_id),
        }
    }

    /// Bring a reopened store's table directory and `schema`'s OID counter into step with
    /// what the tree holds beside the catalog: forget a table the catalog does not name
    /// (`BtreeStorageWrapper::forget_tables`), and seed the counter past every index id the
    /// tree holds an entry under (`BtreeStorageWrapper::index_id_floor`), so no number a
    /// reopen hands a new table or index keys another's rows or entries. A no-op for memory
    /// storage, which is never reopened.
    ///
    /// # Errors
    ///
    /// A page the store cannot read.
    pub fn reconcile_with_catalog(&mut self, schema: &mut Schema) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => Ok(()),
            Self::Btree(btree) => {
                btree.forget_tables(|name| schema.tables.contains_key(name));
                let floor = i32::try_from(btree.index_id_floor()?).unwrap_or(i32::MAX);
                schema.next_oid = schema.next_oid.max(floor);
                Ok(())
            }
        }
    }

    /// Capture a lock-free read snapshot for B+tree storage.
    ///
    /// Returns `None` for memory storage (which has no MVCC snapshot path).
    pub fn read_snapshot(&self) -> Option<BtreeSnapshot> {
        match self {
            Self::Btree(btree) => Some(btree.read_snapshot()),
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => None,
        }
    }
}

// `StorageBackend` is itself a `Storage` impl: enum-dispatch to the chosen
// backend. Both backends now impl the single `pglet_core::Storage` directly, so
// these arms return the trait's `Result` with no error bridging — pure dispatch.
impl Storage for StorageBackend {
    fn create_table(&mut self, name: &str) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.create_table(name),
            Self::Btree(s) => s.create_table(name),
        }
    }

    fn drop_table(&mut self, name: &str) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.drop_table(name),
            Self::Btree(s) => s.drop_table(name),
        }
    }

    fn insert(&mut self, table: &str, row: Vec<Value>) -> Result<RowId, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.insert(table, row),
            Self::Btree(s) => s.insert(table, row),
        }
    }

    fn scan(&self, table: &str) -> Result<Vec<(RowId, Vec<Value>)>, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.scan(table),
            Self::Btree(s) => s.scan(table),
        }
    }

    fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.get(table, id),
            Self::Btree(s) => s.get(table, id),
        }
    }

    fn delete(&mut self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.delete(table, id),
            Self::Btree(s) => s.delete(table, id),
        }
    }

    fn update(
        &mut self,
        table: &str,
        id: RowId,
        new_row: Vec<Value>,
    ) -> Result<Option<Vec<Value>>, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.update(table, id, new_row),
            Self::Btree(s) => s.update(table, id, new_row),
        }
    }

    fn add_column(&mut self, table: &str, default: Value) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.add_column(table, default),
            Self::Btree(s) => s.add_column(table, default),
        }
    }

    fn drop_column(&mut self, table: &str, column_index: usize) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.drop_column(table, column_index),
            Self::Btree(s) => s.drop_column(table, column_index),
        }
    }

    fn create_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.create_index(spec),
            Self::Btree(s) => s.create_index(spec),
        }
    }

    fn reattach_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.reattach_index(spec),
            Self::Btree(s) => s.reattach_index(spec),
        }
    }

    fn rebuild_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.rebuild_index(spec),
            Self::Btree(s) => s.rebuild_index(spec),
        }
    }

    fn drop_index(&mut self, index_id: u32) -> Result<(), Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.drop_index(index_id),
            Self::Btree(s) => s.drop_index(index_id),
        }
    }

    fn index_lookup(&self, index_id: u32, values: &[Value]) -> Result<Vec<RowId>, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.index_lookup(index_id, values),
            Self::Btree(s) => s.index_lookup(index_id, values),
        }
    }

    fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<RowId>, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.index_range_lookup(index_id, lower, upper, ascending, limit),
            Self::Btree(s) => s.index_range_lookup(index_id, lower, upper, ascending, limit),
        }
    }

    fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>, Error> {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(s) => s.gin_lookup(index_id, keys),
            Self::Btree(s) => s.gin_lookup(index_id, keys),
        }
    }

    // A savepoint needs a connection identity and this dispatcher has none, so
    // both arms refuse — deliberately including memory, whose own `Storage` impl
    // does support them. Dispatching per backend here would mean a savepoint that
    // works on one backend and is `0A000` on the other over the *same* call, the
    // backend-shaped divergence this engine refuses; the usable entry points are
    // the inherent `StorageBackend::savepoint(conn_id)` above and `StorageAdapter`,
    // which carries a `conn_id`.
    fn savepoint(&mut self) -> Result<(), Error> {
        Err(no_conn_savepoint())
    }
    fn rollback_to_savepoint(&mut self) -> Result<(), Error> {
        Err(no_conn_savepoint())
    }
    fn release_savepoint(&mut self) -> Result<(), Error> {
        Err(no_conn_savepoint())
    }

    // The memory backend keeps the default: it does not watch, so it knows no table.
    fn rows_untoasted(&self, table: &str) -> bool {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => false,
            Self::Btree(btree) => btree.rows_untoasted(table),
        }
    }

    fn note_toasted_row(&self, table: &str) {
        match self {
            #[cfg(feature = "memory_storage")]
            Self::Memory(_) => {}
            Self::Btree(btree) => btree.note_toasted_row(table),
        }
    }
}

/// The refusal the connection-less `Storage` dispatch answers with.
fn no_conn_savepoint() -> Error {
    Error::NotImplemented("a savepoint needs a connection identity".into())
}

/// Connection-aware adapter over a mutable `StorageBackend`.
///
/// With the single unified `Storage` trait, the error-bridging that an adapter
/// once carried is gone — but this wrapper stays because it carries genuine
/// logic the bare `StorageBackend` enum-dispatch cannot: per-connection
/// read-your-own-writes (RYOW) transaction routing. For Btree storage with an
/// active transaction (`has_transaction(conn_id)`), each operation routes to the
/// transaction's `*_in_tx` pending-writes buffer; otherwise it uses the backend's
/// committed (auto-commit) path. It borrows the backend mutably for the duration
/// of one statement; callers holding the backend behind a lock pass the guard
/// through.
#[derive(Debug)]
pub struct StorageAdapter<'a> {
    storage: &'a mut StorageBackend,
    conn_id: i32,
}

impl<'a> StorageAdapter<'a> {
    /// Create a new storage adapter over a borrowed backend.
    pub const fn new(storage: &'a mut StorageBackend, conn_id: i32) -> Self {
        Self { storage, conn_id }
    }

    /// Attribute an auto-commit DDL just applied through this adapter to the
    /// connection's open transaction (if any), so its commit knows its batch is not
    /// the whole of it. A no-op outside a B+tree transaction (memory storage has no
    /// per-connection transaction state).
    fn note_own_ddl(&mut self) {
        if let StorageBackend::Btree(btree) = &mut *self.storage
            && btree.has_transaction(self.conn_id)
        {
            btree.note_txn_ddl(self.conn_id);
        }
    }

    /// Move the connection's open B+tree transaction onto the root it has just published
    /// index entries into, so its probes and indexed reads see them
    /// (`BtreeStorageWrapper::advance_transaction_snapshot`, whose doc carries why that is
    /// sound). A no-op outside a B+tree transaction.
    fn read_own_publish(&mut self) {
        if let StorageBackend::Btree(btree) = &mut *self.storage {
            btree.advance_transaction_snapshot(self.conn_id);
        }
    }
}

impl Storage for StorageAdapter<'_> {
    fn create_table(&mut self, name: &str) -> Result<(), Error> {
        // DDL always uses auto-commit semantics (applied immediately). When this
        // connection has an open transaction, attribute the table to it.
        let res = Storage::create_table(self.storage, name);
        if res.is_ok() {
            self.note_own_ddl();
        }
        res
    }

    fn drop_table(&mut self, name: &str) -> Result<(), Error> {
        // In a B+tree transaction, defer the destructive deletion (rows + SYS_TABLE +
        // index entries) to commit so a ROLLBACK restores the table; `drop_table_in_tx`
        // removes the name mapping now and does its own own-DDL bookkeeping. Outside a
        // transaction (or on memory storage), apply immediately (auto-commit).
        match self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .drop_table_in_tx(self.conn_id, name)
                .map_err(Error::from),
            _ => Storage::drop_table(self.storage, name),
        }
    }

    fn insert(&mut self, table: &str, row: Vec<Value>) -> Result<RowId, Error> {
        // Check if we have an active Btree transaction
        match self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .insert_in_tx(self.conn_id, table, row)
                .map_err(Error::from),
            _ => Storage::insert(self.storage, table, row),
        }
    }

    fn scan(&self, table: &str) -> Result<Vec<(RowId, Vec<Value>)>, Error> {
        // Check if we have an active Btree transaction
        match &*self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => {
                btree.scan_in_tx(self.conn_id, table).map_err(Error::from)
            }
            _ => Storage::scan(&*self.storage, table),
        }
    }

    fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
        // Check if we have an active Btree transaction
        match &*self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .get_in_tx(self.conn_id, table, id)
                .map_err(Error::from),
            _ => Storage::get(&*self.storage, table, id),
        }
    }

    fn delete(&mut self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
        // Check if we have an active Btree transaction
        match self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .delete_in_tx(self.conn_id, table, id)
                .map_err(Error::from),
            _ => Storage::delete(self.storage, table, id),
        }
    }

    fn update(
        &mut self,
        table: &str,
        id: RowId,
        new_row: Vec<Value>,
    ) -> Result<Option<Vec<Value>>, Error> {
        // Check if we have an active Btree transaction
        match self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .update_in_tx(self.conn_id, table, id, new_row)
                .map_err(Error::from),
            _ => Storage::update(self.storage, table, id, new_row),
        }
    }

    fn add_column(&mut self, table: &str, default: Value) -> Result<(), Error> {
        // Check if we have an active Btree transaction
        match self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .add_column_in_tx(self.conn_id, table, default)
                .map_err(Error::from),
            _ => Storage::add_column(self.storage, table, default),
        }
    }

    fn drop_column(&mut self, table: &str, column_index: usize) -> Result<(), Error> {
        // Check if we have an active Btree transaction
        match self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .drop_column_in_tx(self.conn_id, table, column_index)
                .map_err(Error::from),
            _ => Storage::drop_column(self.storage, table, column_index),
        }
    }

    fn create_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
        // Build + back-fill immediately so a probe within an open transaction sees the
        // new index (read-your-own-writes). When in a transaction, record the created
        // id so rollback can undo it (deregister the def + shed its entries) — without
        // this the def + entries would persist while the catalog layer drops the index,
        // leaving a phantom storage index whose table_id can later be reused. Also
        // covers `CREATE TABLE ... PRIMARY KEY` in a txn (its auto-created unique index).
        //
        // The `is_ok` gate is exactly right, and only because `Storage::create_index`
        // is **all-or-nothing**: a failed build registers nothing, so there is nothing
        // for the rollback to shed and noting the id would ask it to deregister a def
        // that was never installed. Under the older register-then-back-fill ordering
        // the same gate was the second half of a live defect — the def stood, the note
        // was never taken, and `ROLLBACK` restored the catalog over a storage index no
        // statement could name.
        //
        // The build's entries for the rows committed before the transaction are
        // published at once, so the transaction reads them from here on: a unique probe
        // that missed them accepted a duplicate of a committed row (the repo's
        // `EDGES.md` row 147).
        let index_id = spec.index_id;
        let res = Storage::create_index(self.storage, spec);
        if res.is_ok()
            && let StorageBackend::Btree(btree) = &mut *self.storage
            && btree.has_transaction(self.conn_id)
        {
            btree.note_txn_created_index(self.conn_id, index_id);
            self.read_own_publish();
        }
        res
    }

    fn reattach_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
        Storage::reattach_index(self.storage, spec)
    }

    fn rebuild_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
        // Neither branch of `drop_index`/`create_index` above applies: the rebuild is
        // one immediate operation on both backends (a deferred drop is exactly what it
        // must not take), and it creates nothing, so there is no id to note for the
        // rollback undo — a `ROLLBACK`'s catalog restore re-runs this instead. Its
        // entries are published at once, as a build's are, and read from here on.
        let res = Storage::rebuild_index(self.storage, spec);
        if res.is_ok() {
            self.read_own_publish();
        }
        res
    }

    fn drop_index(&mut self, index_id: u32) -> Result<(), Error> {
        // Buffer the drop inside a transaction (deferred to commit) so a ROLLBACK
        // restores the index; otherwise apply immediately. Covers both a standalone
        // DROP INDEX and the cascade from a dropped table (`execute_drop_table`).
        match self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .drop_index_in_tx(self.conn_id, index_id)
                .map_err(Error::from),
            _ => Storage::drop_index(self.storage, index_id),
        }
    }

    fn index_lookup(&self, index_id: u32, values: &[Value]) -> Result<Vec<RowId>, Error> {
        // Read-your-own-writes: with an active Btree transaction, the committed
        // index doesn't yet reflect this transaction's pending writes, so route
        // through the transaction-aware probe (scan-of-pending filtered by the
        // key column(s)) — identical rows to what `Filter(Scan)` would return.
        // Outside a transaction (autocommit), use the fast committed-index probe.
        match &*self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .index_lookup_in_tx(self.conn_id, index_id, values)
                .map_err(Error::from),
            _ => Storage::index_lookup(&*self.storage, index_id, values),
        }
    }

    fn unique_key_claimed_elsewhere(&self, index_id: u32, values: &[Value]) -> Result<bool, Error> {
        // The question is about the *other* connections, so unlike the probes above
        // this does not branch on whether this connection has a transaction of its own:
        // an auto-commit writer (which the hosts still bracket in an implicit storage
        // transaction) and an explicit block ask it identically. Memory storage buffers
        // no cross-connection pending writes and keeps the trait's `false`.
        match &*self.storage {
            StorageBackend::Btree(btree) => btree
                .unique_key_claimed_elsewhere(self.conn_id, index_id, values)
                .map_err(Error::from),
            #[cfg(feature = "memory_storage")]
            StorageBackend::Memory(_) => Ok(false),
        }
    }

    fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<RowId>, Error> {
        // Read-your-own-writes: with an active Btree transaction the committed index
        // lags this transaction's pending writes, so route through the
        // transaction-aware range probe (scan-of-pending filtered + sorted by the key)
        // — identical rows/order to what `Filter(Scan)` + sort would return. Outside a
        // transaction, use the committed-index range scan.
        match &*self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .index_range_lookup_in_tx(
                    self.conn_id,
                    index_id,
                    lower.as_ref().map(|(v, incl)| (v, *incl)),
                    upper.as_ref().map(|(v, incl)| (v, *incl)),
                    ascending,
                    limit,
                )
                .map_err(Error::from),
            _ => Storage::index_range_lookup(
                &*self.storage,
                index_id,
                lower,
                upper,
                ascending,
                limit,
            ),
        }
    }

    fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>, Error> {
        // Read-your-own-writes: with an active Btree transaction the committed index
        // lags this transaction's pending writes, so route through the
        // transaction-aware containment probe (scan-of-pending filtered by the same
        // candidate test). Outside a transaction, use the committed posting probe.
        match &*self.storage {
            StorageBackend::Btree(btree) if btree.has_transaction(self.conn_id) => btree
                .gin_lookup_in_tx(self.conn_id, index_id, keys)
                .map_err(Error::from),
            _ => Storage::gin_lookup(&*self.storage, index_id, keys),
        }
    }
    // The savepoint stack is per connection, so the adapter's `conn_id` is exactly
    // what the bare backend dispatch above lacks. This is the path a `DO` block takes,
    // and the reason the three methods are on the trait at all.
    fn savepoint(&mut self) -> Result<(), Error> {
        self.storage.savepoint(self.conn_id)
    }
    fn rollback_to_savepoint(&mut self) -> Result<(), Error> {
        self.storage.rollback_to_savepoint(self.conn_id)
    }
    fn release_savepoint(&mut self) -> Result<(), Error> {
        self.storage.release_savepoint(self.conn_id)
    }

    fn rows_untoasted(&self, table: &str) -> bool {
        self.storage.rows_untoasted(table)
    }

    fn note_toasted_row(&self, table: &str) {
        self.storage.note_toasted_row(table);
    }
}

/// Read-only adapter for lock-free reads using a B+tree snapshot.
///
/// Wraps a `BtreeSnapshot` to provide the executor's Storage trait. Only read
/// operations are supported; write operations return `Error::NotImplemented`
/// (a snapshot is an immutable read view).
#[derive(Debug)]
pub struct SnapshotAdapter<'a> {
    snapshot: &'a BtreeSnapshot,
}

impl<'a> SnapshotAdapter<'a> {
    /// Create a new snapshot adapter.
    pub const fn new(snapshot: &'a BtreeSnapshot) -> Self {
        Self { snapshot }
    }
}

impl Storage for SnapshotAdapter<'_> {
    fn create_table(&mut self, _name: &str) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: create_table not supported".into(),
        ))
    }

    fn drop_table(&mut self, _name: &str) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: drop_table not supported".into(),
        ))
    }

    fn insert(&mut self, _table: &str, _row: Vec<Value>) -> Result<RowId, Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: insert not supported".into(),
        ))
    }

    fn scan(&self, table: &str) -> Result<Vec<(RowId, Vec<Value>)>, Error> {
        self.snapshot.scan(table).map_err(Error::from)
    }

    fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
        self.snapshot.get(table, id).map_err(Error::from)
    }

    fn delete(&mut self, _table: &str, _id: RowId) -> Result<Option<Vec<Value>>, Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: delete not supported".into(),
        ))
    }

    fn update(
        &mut self,
        _table: &str,
        _id: RowId,
        _new_row: Vec<Value>,
    ) -> Result<Option<Vec<Value>>, Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: update not supported".into(),
        ))
    }

    fn add_column(&mut self, _table: &str, _default: Value) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: add_column not supported".into(),
        ))
    }

    fn drop_column(&mut self, _table: &str, _column_index: usize) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: drop_column not supported".into(),
        ))
    }

    fn create_index(&mut self, _spec: IndexSpec) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: create_index not supported".into(),
        ))
    }

    fn reattach_index(&mut self, _spec: IndexSpec) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: reattach_index not supported".into(),
        ))
    }

    fn rebuild_index(&mut self, _spec: IndexSpec) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: rebuild_index not supported".into(),
        ))
    }

    fn drop_index(&mut self, _index_id: u32) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "SnapshotAdapter is read-only: drop_index not supported".into(),
        ))
    }

    fn index_lookup(&self, index_id: u32, values: &[Value]) -> Result<Vec<RowId>, Error> {
        // A snapshot is a read view; index probes are a read and are supported,
        // reading index state consistent with this snapshot's root.
        self.snapshot
            .index_lookup(index_id, values)
            .map_err(Error::from)
    }

    fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<RowId>, Error> {
        // A range probe is a read; serve it from the snapshot's root.
        self.snapshot
            .index_range_lookup(
                index_id,
                lower.as_ref().map(|(v, incl)| (v, *incl)),
                upper.as_ref().map(|(v, incl)| (v, *incl)),
                ascending,
                limit,
            )
            .map_err(Error::from)
    }

    fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>, Error> {
        // A containment probe is a read; serve it from the snapshot's root.
        self.snapshot
            .gin_lookup(index_id, keys)
            .map_err(Error::from)
    }
    // A read-only snapshot has nothing to stage or discard: it is handed to the
    // lock-free read path, which never writes, so a savepoint over it would be a
    // no-op that reads as support. Refuse instead.
    fn savepoint(&mut self) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "savepoints are not available on a read-only snapshot".into(),
        ))
    }
    fn rollback_to_savepoint(&mut self) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "savepoints are not available on a read-only snapshot".into(),
        ))
    }
    fn release_savepoint(&mut self) -> Result<(), Error> {
        Err(Error::NotImplemented(
            "savepoints are not available on a read-only snapshot".into(),
        ))
    }

    fn rows_untoasted(&self, table: &str) -> bool {
        self.snapshot.rows_untoasted(table)
    }
}
