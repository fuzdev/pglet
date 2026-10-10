//! Copy-on-Write B+tree storage engine.
//!
//! This module implements a B+tree optimized for single-writer concurrency
//! with natural MVCC via root versioning. Key design decisions:
//!
//! - **No version in key**: Unlike LSM, versions aren't stored in keys.
//!   MVCC is handled by retaining old root pointers.
//! - **Copy-on-write**: Pages are never modified in place. Writes create
//!   new pages and update parent pointers up to the root.
//! - **Single-writer**: Commits are serialized. No write conflict detection needed.
//! - **Snapshot isolation**: Readers capture root page ID at transaction start.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                          CowBtree                            │
//! │  root: AtomicU64 ──► latest committed tree (seqlock-paired   │
//! │    with version so a snapshot never sees a torn pair)        │
//! │  active_snapshots: {version -> refcount} ──► vacuum safety   │
//! │  write_lock: Mutex ──► single-writer serialization           │
//! └──────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌──────────────────────────────────────────────────────────────┐
//! │              PageStore (memory / DoubleMetaStore)            │
//! │  pages: in-memory map or single-file double-meta over        │
//! │    a BlockIo (native file / OPFS)                            │
//! │  free_list: Vec<PageId> ──► reclaimed pages                  │
//! │  next_page_id: u64 ──► allocation counter                    │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Key Format
//!
//! `BtreeKey` (see `key.rs` and this directory's `CLAUDE.md`) is a
//! variable-length, byte-comparable key over one keyspace with a leading
//! discriminant: data rows (`0x00` + table_id + row_id, big-endian), index
//! entries (`0x01` + index_id + an order-preserving escape-terminated value +
//! row_id), and system entries (`0x02`, the catalog/table directory). Byte
//! order equals logical order, so range scans work within every band.

mod block_io;
mod cow;
mod double_meta;
mod key;
#[cfg(test)]
mod mem_block_io;
mod page;
mod page_cache;
mod store;
mod system;
mod wrapper;

#[cfg(feature = "file_store")]
pub use block_io::FileBlockIo;
pub use block_io::{BlockIo, BlockIoStats, CountingBlockIo, CountingHandle};
pub use cow::{
    BatchChanges, CowBtree, CowBtreeError, CowBtreeStats, CowPageStore, Snapshot, VacuumResult,
    Version,
};
pub use double_meta::DoubleMetaStore;
// Re-export for custom page store implementations
pub use key::{BtreeKey, SysKind, encode_composite_index_key};
pub use page::{
    DATA_SIZE, LeafValue, MAX_KEY_LEN, NULL_PAGE_ID, OverflowRef, PAGE_SIZE, Page, PageFlags,
    PageHeader, PageId,
};
pub use store::{
    DeviceError, PageStore, PageStoreError, PageStoreMetadata, PageStoreResult, WriteFailure,
};
pub use wrapper::{
    BtreePendingWrites, BtreeSnapshot, BtreeStorageWrapper, BtreeTransactionState, PendingRow,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Verify module structure compiles and exports are accessible
        let key = BtreeKey::new(1, 100);
        assert_eq!(key.table_id(), Some(1));
        assert_eq!(key.row_id(), 100);
    }
}
