//! Engine assembly + scratch directories for the `pglet` integration tests.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use pglet::{Database, StorageBackend};
use pglet_sql::catalog::Schema;
use pglet_storage::btree::{BlockIo, BtreeStorageWrapper, CowBtree, DoubleMetaStore};

/// A unique scratch dir under the system temp dir, removed on drop. No external
/// crate (`tempfile` is not a dependency); a pid + counter name is collision-free
/// enough for a single test process.
#[derive(Debug)]
pub struct ScratchDir {
    pub path: PathBuf,
}

impl ScratchDir {
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("pglet_test_{tag}_{}_{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create scratch dir");
        Self { path }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Build a `Database` over a `DoubleMetaStore` wrapping the given `BlockIo`.
///
/// `Database::open`'s assembly (open store → `CowBtree::with_store` → wrapper with
/// restored mappings → restore schema → re-register the persisted indexes), letting a
/// test inject a custom `BlockIo` (e.g. [`MemBlockIo`](crate::block_io::MemBlockIo))
/// without touching the filesystem. One step differs: the catalog is decoded with
/// `postcard` directly rather than through `Database::open`'s decoder
/// (`pglet_sql::catalog::deserialize_schema`).
pub fn open_database(io: Box<dyn BlockIo>) -> Database {
    open_database_checked(io).expect("open double-meta store")
}

/// Like [`open_database`], but returns `None` instead of panicking on a
/// *detected-corrupt* image.
///
/// The two corrupt outcomes are a failed `DoubleMetaStore::open` (a torn/CRC-bad
/// meta slot, a stale/garbage magic) or an undecodable schema blob. A recovery
/// harness (the crash sweep, the `double_meta_reopen` fuzz target) must treat both
/// as a clean "loud" rejection, distinct from a *panic* (which the fuzzer flags as a
/// robustness finding — the engine should fail loud, not panic, on an untrusted
/// image). `open_database` keeps the unwrapping shape for the common test path where
/// a non-openable image is itself the bug.
#[must_use]
pub fn open_database_checked(io: Box<dyn BlockIo>) -> Option<Database> {
    try_open_database(io).ok()
}

/// Like [`open_database_checked`], but the rejection carries the error's text — for
/// a harness that asks which check refused the image.
///
/// # Errors
///
/// The rendered error of the step that refused the image.
pub fn try_open_database(io: Box<dyn BlockIo>) -> Result<Database, String> {
    let (store, metadata) = DoubleMetaStore::open(io).map_err(|e| e.to_string())?;
    let btree = CowBtree::with_store(Box::new(store), metadata.root, metadata.version, 100);
    // Recover the table directory, row-id counters, and catalog from the tree (the
    // PGM5 system entries). A read error (corruption) or a malformed system entry is
    // a clean "loud" rejection, never a panic (the fuzz contract).
    let (wrapper, schema_bytes) =
        BtreeStorageWrapper::recover(std::sync::Arc::new(btree), metadata.next_table_id)
            .map_err(|e| e.to_string())?;
    let schema = if schema_bytes.is_empty() {
        Schema::new()
    } else {
        postcard::from_bytes(&schema_bytes).map_err(|e| e.to_string())?
    };
    let mut db = Database::with_storage(StorageBackend::Btree(wrapper), schema);
    db.reattach_persisted_indexes().map_err(|e| e.to_string())?;
    Ok(db)
}
