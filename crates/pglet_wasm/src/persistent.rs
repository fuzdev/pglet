//! The OPFS-backed persistent database, behind the crate's `opfs` feature.
//!
//! The npm package's build leaves the feature off, so its wasm carries neither this binding
//! nor the OPFS `BlockIo` (`crate::opfs_block_io`); a build that turns it on — the browser
//! example and `test:browser` — exports `PgletBtreePersistent` beside `PgletBtree`.

use pglet::{Database, StorageBackend};
use pglet_sql::catalog::Schema;
use pglet_storage::btree::{BlockIo, BtreeStorageWrapper, CowBtree, DoubleMetaStore};
use wasm_bindgen::prelude::*;

use crate::call::ModuleCall;
use crate::database::{exec_bound, run_error_to_js};
use crate::error::pglet_error_to_js;
use crate::opfs_block_io::OpfsBlockIo;
use crate::result::{QueryResult, RowShape};

/// B+tree persistent database interface for JS.
///
/// This version uses OPFS with a B+tree storage engine for persistence.
///
/// **Durability:** every autocommit write (DDL and DML) is made durable
/// immediately by flipping the double-meta slot, exactly like the native server, so
/// it survives a reload with no explicit flush. To batch many writes into a single
/// durable commit (one meta flip instead of one per statement), wrap them in a
/// `BEGIN`/`COMMIT` transaction; [`sync`](Self::sync) forces a flush but autocommit
/// durability does not require it.
///
/// Persistence is single-file, crash-safe via the double-meta layout (two
/// alternating metadata slots; commit = write CoW pages → fsync → flip the
/// alternate meta slot → fsync). This is the same `DoubleMetaStore` the native
/// build uses, just over an OPFS handle instead of a file — no WAL.
///
/// # Important: Web Worker Required
///
/// This must be used from a Web Worker because the OPFS sync API
/// (`FileSystemSyncAccessHandle`) is only available in workers.
#[wasm_bindgen]
#[derive(Debug)]
pub struct PgletBtreePersistent {
    db: Database,
    /// The module call the close holds while `db` drops (`Drop`): declared after `db`, so
    /// it is released only once the database's own drop has run.
    closing: Option<ModuleCall>,
}

/// The close is the database's drop, and that drop writes: it rolls back a block left
/// open, and the rollback saves through the OPFS handle (`pglet::Database`'s `Drop`). The
/// generated `free` — and the `FinalizationRegistry` that calls it when JS lets the object
/// go — reaches it with no export's `ModuleCall` around it, so the close takes one itself:
/// held while the database drops, and where it is refused — a module an earlier call
/// never returned from, whose memory can no longer be trusted, or a call already running —
/// the database's drop writes nothing (`Database::skip_drop_rollback`), leaving the file
/// as the last save left it, as a kill would.
impl Drop for PgletBtreePersistent {
    fn drop(&mut self) {
        match ModuleCall::try_enter() {
            Ok(call) => self.closing = Some(call),
            Err(_) => self.db.skip_drop_rollback(),
        }
    }
}

#[wasm_bindgen]
impl PgletBtreePersistent {
    /// Opens or creates a persistent B+tree database.
    ///
    /// This is async because acquiring the OPFS sync handle requires promises.
    /// If data exists from a previous session, it is loaded. Crash recovery
    /// happens inside `DoubleMetaStore::open`, which selects the valid
    /// higher-version meta slot (falling back to the older slot if the newer one
    /// is torn) — there is no WAL replay.
    // `open()` awaits OPFS init while holding JS handles, so its future is `!Send` — correct and
    // irrelevant in the single-threaded browser runtime (same rationale as `opfs_block_io.rs`).
    #[allow(clippy::future_not_send)]
    #[wasm_bindgen]
    pub async fn open() -> Result<Self, JsValue> {
        // Acquire the single OPFS data-file handle and wrap it as a BlockIo, then
        // open the same double-meta store the native build uses.
        let opfs_io = OpfsBlockIo::open().await?;
        // Entered after the one await: from here the open runs to its end without
        // yielding, so no other call can interleave with it.
        let _call = ModuleCall::enter()?;
        let db = open_over(Box::new(opfs_io)).map_err(|e| JsValue::from_str(&e))?;
        Ok(Self { db, closing: None })
    }

    /// Executes a SQL statement without returning results.
    #[wasm_bindgen]
    pub fn exec(&mut self, sql: &str) -> Result<(), JsValue> {
        self.run(sql)?;
        Ok(())
    }

    /// Executes a SQL query and returns the results.
    #[wasm_bindgen]
    pub fn query(&mut self, sql: &str) -> Result<QueryResult, JsValue> {
        self.run(sql)
    }

    /// Syncs all pending writes to disk.
    ///
    /// This saves both the page data and metadata (schema, table mappings, etc.).
    #[wasm_bindgen]
    pub fn sync(&self) -> Result<(), JsValue> {
        let _call = ModuleCall::enter()?;
        self.db.sync().map_err(|e| pglet_error_to_js(&e))
    }

    /// Clears all data from the database.
    ///
    /// Note: This currently resets state in memory. Full clear requires
    /// reopening the database (OPFS files can't be cleared while handles are open).
    #[wasm_bindgen]
    pub fn clear(&mut self) -> Result<(), JsValue> {
        let _call = ModuleCall::enter()?;
        *self.db.schema_mut() = Schema::new();
        // Note: Can't fully clear OPFS with handles open
        // User should close and reopen the database for a full reset
        Ok(())
    }
}

/// Assemble the persistent binding's database over `io`: the same double-meta store the
/// native build opens, recovered from the tree. Kept apart from the OPFS handle and from
/// every JS value so the host target's tests can open it over bytes in memory.
fn open_over(io: Box<dyn BlockIo>) -> Result<Database, String> {
    let (page_store, metadata) =
        DoubleMetaStore::open(io).map_err(|e| format!("open double-meta store: {e}"))?;

    // Build the CowBtree over the recovered root/version. The store has
    // already picked the latest consistent meta slot, so there is nothing to
    // replay here.
    let btree = CowBtree::with_store(
        Box::new(page_store),
        metadata.root,
        metadata.version,
        100, // vacuum_threshold
    );

    // Recover the table directory, row-id counters, and catalog from the tree
    // (the PGM5 `0x02` system entries), not the meta slot — the same recovery
    // path as native open; OPFS durability re-validated by the browser tests.
    let (storage, schema_bytes) =
        BtreeStorageWrapper::recover(std::sync::Arc::new(btree), metadata.next_table_id)
            .map_err(|e| format!("recover catalog from tree: {e}"))?;

    // Restore schema from the reassembled SYS_CATALOG bytes (or empty if none).
    let schema = if schema_bytes.is_empty() {
        Schema::new()
    } else {
        pglet_sql::catalog::deserialize_schema(&schema_bytes)
            .map_err(|e| format!("Failed to deserialize schema: {e}"))?
    };

    // Hand the recovered parts to the facade; from here the parse → bind →
    // execute → persist driver lives in `pglet::Database`.
    let mut db = Database::with_storage(StorageBackend::Btree(storage), schema);
    // Re-register persisted indexes (entries are in the tree; only the in-memory
    // def map needs restoring) — the same step native `open` does, so a reopened
    // indexed OPFS database maintains/probes its indexes correctly.
    db.reattach_persisted_indexes()
        .map_err(|e| format!("re-register indexes: {e}"))?;

    Ok(db)
}

impl PgletBtreePersistent {
    /// Shared exec core: the transaction routing of [`PgletBtree`](crate::PgletBtree), through the same
    /// `exec_bound` with no parameters — this host exposes no bound-parameter method —
    /// over the OPFS-backed facade. Autocommit writes persist
    /// inside the facade (meta flip); an explicit `COMMIT` is the batched durable
    /// point.
    fn run(&mut self, sql: &str) -> Result<QueryResult, JsValue> {
        let _call = ModuleCall::enter()?;
        let result = exec_bound(&mut self.db, sql, &[]).map_err(run_error_to_js)?;
        QueryResult::from_pglet_result(result, RowShape::Objects)
    }
}

#[cfg(test)]
mod persistent_close_tests {
    //! What the persistent binding's close writes. Driven on the host target over the
    //! binding's own assembly (`open_over`) and a store held in memory — `free` and the
    //! finalizer are wasm-bindgen's generated drop of the object, and what they reach is
    //! this `Drop`.

    use std::sync::{Arc, Mutex, PoisonError};

    use pglet_storage::btree::BlockIo;

    use super::{PgletBtreePersistent, open_over};
    use crate::call::{ModuleCall, exclusive_module, reset_module};
    use crate::database::exec_bound;

    /// A store's bytes in memory, shared between clones so a test reads what was written.
    #[derive(Clone, Default)]
    struct Bytes(Arc<Mutex<Vec<u8>>>);

    impl Bytes {
        fn get(&self) -> Vec<u8> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl BlockIo for Bytes {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
            let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let start = usize::try_from(offset).map_or(bytes.len(), |o| o.min(bytes.len()));
            let n = buf.len().min(bytes.len() - start);
            buf[..n].copy_from_slice(&bytes[start..start + n]);
            Ok(n)
        }
        fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
            let mut bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let start = usize::try_from(offset).map_err(std::io::Error::other)?;
            if bytes.len() < start + buf.len() {
                bytes.resize(start + buf.len(), 0);
            }
            bytes[start..start + buf.len()].copy_from_slice(buf);
            Ok(())
        }
        fn sync(&self) -> std::io::Result<()> {
            Ok(())
        }
        fn len(&self) -> std::io::Result<u64> {
            Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).len() as u64)
        }
        fn set_len(&self, size: u64) -> std::io::Result<()> {
            let size = usize::try_from(size).map_err(std::io::Error::other)?;
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .resize(size, 0);
            Ok(())
        }
    }

    /// The next value `nextval('s')` answers on a database opened over `bytes`.
    fn next(bytes: Vec<u8>) -> String {
        let mut db = open_over(Box::new(Bytes(Arc::new(Mutex::new(bytes))))).expect("reopen");
        let result =
            exec_bound(&mut db, "SELECT nextval('s')", &[]).unwrap_or_else(|_| panic!("nextval"));
        format!("{:?}", result.rows)
    }

    /// A database whose block has handed out `nextval('s')` = 2, and the bytes before the
    /// close.
    fn open_block() -> (PgletBtreePersistent, Bytes, Vec<u8>) {
        let io = Bytes::default();
        let db = open_over(Box::new(io.clone())).expect("open");
        let mut persistent = PgletBtreePersistent { db, closing: None };
        for sql in [
            "CREATE SEQUENCE s",
            "SELECT nextval('s')",
            "BEGIN",
            "SELECT nextval('s')",
        ] {
            exec_bound(&mut persistent.db, sql, &[]).unwrap_or_else(|_| panic!("{sql}"));
        }
        let before = io.get();
        (persistent, io, before)
    }

    /// A close the module can trust rolls the open block back and saves the counter it
    /// handed out, as the embedded drop does: a reopen answers the next value. A close
    /// made once an earlier call never returned writes nothing — the file is the bytes
    /// before it, byte for byte — and one made while another call is running writes
    /// nothing either. Red for a close that reaches the database's drop without taking a
    /// call of its own.
    #[test]
    fn a_close_after_the_module_is_poisoned_writes_nothing() {
        let _module = exclusive_module();

        let (persistent, io, before) = open_block();
        drop(persistent);
        assert_ne!(
            io.get(),
            before,
            "a trusted close writes its rollback's save"
        );
        assert_eq!(
            next(io.get()),
            "[[Int8(3)]]",
            "a trusted close keeps the counter"
        );
        drop(ModuleCall::try_enter().expect("the close released its call"));

        let (persistent, io, before) = open_block();
        std::mem::forget(ModuleCall::try_enter().expect("a call that never returns"));
        drop(persistent);
        assert_eq!(io.get(), before, "a poisoned module's close writes nothing");
        reset_module();

        let (persistent, io, before) = open_block();
        let running = ModuleCall::try_enter().expect("a running call");
        drop(persistent);
        drop(running);
        assert_eq!(
            io.get(),
            before,
            "a close inside a running call writes nothing"
        );
    }
}
