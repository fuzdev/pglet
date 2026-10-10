//! pglet's storage backends behind `pglet_core`'s `Storage` trait: the copy-on-write B+tree
//! (`btree`) — in memory, or a single double-meta file over a `BlockIo` — and the plain
//! `MemoryStorage`. The on-disk format is `btree`'s `CLAUDE.md`.

pub mod btree;
pub mod index_keying;
mod memory;

pub use index_keying::{IndexKeying, encode_probe_key, index_range_bounds_for};
pub use memory::MemoryStorage;

use thiserror::Error;

// `RowId`, `IndexSpec`, the `Value` representation, and the single `Storage`
// trait live in the `pglet_core` leaf crate; `RowId` and `Storage` are re-exported so a
// storage consumer names them without its own `pglet_core` dependency (the backends below
// impl `pglet_core::Storage` directly).
pub use pglet_core::{RowId, Storage};

/// Errors from storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("table not found: {0}")]
    TableNotFound(String),

    #[error("table already exists: {0}")]
    TableExists(String),

    #[error("row not found: {0}")]
    RowNotFound(RowId),

    #[error("io error: {0}")]
    Io(String),

    #[error("no active transaction for connection {0}")]
    NoTransaction(i32),

    #[error("index not found: {0}")]
    IndexNotFound(u32),

    #[error("value cannot be indexed: {0}")]
    Unindexable(String),

    /// A stored-expression evaluation error — a partial-index predicate or an
    /// index key expression — carried through storage **verbatim**.
    ///
    /// Index maintenance evaluates SQL expressions, so its failures are SQL-layer
    /// errors: an operator-resolution failure (`42883`), a malformed literal
    /// (`22P02`), an arithmetic overflow (`22003`). Folding them into
    /// [`Unindexable`](Self::Unindexable) relabeled every one as `0A000` "data
    /// type … has no default operator class for a B-tree index" — accurate only
    /// for the genuine opclass case (which still uses `Unindexable`) and
    /// misleading for the rest. This variant round-trips the original error, code
    /// and message intact.
    #[error("{0}")]
    ExprEval(Box<pglet_core::Error>),

    #[error("row too large: {0}")]
    ValueTooLarge(String),

    /// An index entry refused for its size during maintenance — one of 18.6's
    /// `54000`s, or pglet's own page bound on a key (`IndexKeying::encode_row_entries`)
    /// — carried through storage **verbatim**, its SQLSTATE, message and fields intact.
    #[error("{0}")]
    IndexEntry(Box<pglet_core::Error>),

    /// A `rollback_to_savepoint` / `release_savepoint` with no savepoint open on
    /// this backend.
    ///
    /// **Not the SQL-visible `3B001`.** A named savepoint the block does not hold is
    /// caught by `pglet::TransactionSession`, which owns the names, before storage
    /// is asked to do anything; this variant means the session's frame stack and the
    /// backend's snapshot stack disagree about their *depth*, which is a bug in the
    /// lockstep and not a user error. It surfaces as a loud execution error rather
    /// than restoring whichever frame happens to be on top.
    #[error("no storage savepoint to roll back to or release")]
    NoSavepoint,

    /// A write or `fsync` the device refused that did **not** fail the store
    /// (`btree::PageStoreError::WriteRefused`): the batch meeting it published nothing
    /// and is gone, and the store serves on.
    #[error("write refused: {0}")]
    WriteRefused(btree::DeviceError),

    /// A write or `fsync` the device refused that failed the store
    /// (`btree::PageStoreError::WriteFailed`): the store refuses everything after it.
    #[error("write failed: {0}")]
    DurableWriteFailed(btree::DeviceError),

    /// A save the device refused at or after its meta-slot write
    /// (`btree::PageStoreError::SaveUnconfirmed`): the store refuses everything after it,
    /// and whether the saved version is durable is unknown until a reopen.
    #[error("save unconfirmed: {0}")]
    DurableWriteUnconfirmed(btree::DeviceError),

    /// An operation refused because an earlier write failed
    /// (`btree::PageStoreError::StoreFailed`).
    #[error("the store refuses work since a write failed: {0}")]
    StoreFailed(String),
}

impl From<StorageError> for pglet_core::Error {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::TableNotFound(table) => {
                pglet_core::CatalogError::TableNotFound { table }.into()
            }
            StorageError::TableExists(table) => {
                pglet_core::CatalogError::TableAlreadyExists { table }.into()
            }
            StorageError::RowNotFound(id) => Self::Execution(format!("row not found: {id}")),
            // Both surface as a generic execution error at the SQL layer; the
            // distinction (oversized row vs IO) is carried by `StorageError`.
            StorageError::Io(msg) | StorageError::ValueTooLarge(msg) => Self::Execution(msg),
            StorageError::NoTransaction(conn_id) => {
                Self::Execution(format!("no active transaction for connection {conn_id}"))
            }
            StorageError::IndexNotFound(id) => Self::Execution(format!("index not found: {id}")),
            StorageError::Unindexable(msg) => Self::UnindexableType(msg),
            // The SQL-layer error passed through storage unchanged — its own
            // SQLSTATE and message are the accurate ones.
            StorageError::ExprEval(e) | StorageError::IndexEntry(e) => *e,
            StorageError::NoSavepoint => {
                Self::Execution("no storage savepoint to roll back to or release".to_string())
            }
            StorageError::WriteRefused(e) => Self::WriteRefused {
                reason: e.reason,
                disk_full: e.disk_full,
            },
            StorageError::DurableWriteFailed(e) => Self::DurableWriteFailed {
                reason: e.reason,
                disk_full: e.disk_full,
            },
            StorageError::DurableWriteUnconfirmed(e) => Self::DurableWriteUnconfirmed {
                reason: e.reason,
                disk_full: e.disk_full,
            },
            StorageError::StoreFailed(reason) => Self::DatabaseMustBeReopened(reason),
        }
    }
}

impl From<btree::CowBtreeError> for StorageError {
    fn from(e: btree::CowBtreeError) -> Self {
        match e {
            // An oversized row is a loud, typed error — not a generic IO failure —
            // so it surfaces as `ValueTooLarge` rather than being flattened into `Io`.
            btree::CowBtreeError::ValueTooLarge | btree::CowBtreeError::KeyTooLarge { .. } => {
                Self::ValueTooLarge(e.to_string())
            }
            btree::CowBtreeError::Storage(store) => store.into(),
            other => Self::Io(other.to_string()),
        }
    }
}

impl From<btree::PageStoreError> for StorageError {
    fn from(e: btree::PageStoreError) -> Self {
        match e {
            btree::PageStoreError::WriteRefused(e) => Self::WriteRefused(e),
            btree::PageStoreError::WriteFailed(e) => Self::DurableWriteFailed(e),
            btree::PageStoreError::SaveUnconfirmed(e) => Self::DurableWriteUnconfirmed(e),
            btree::PageStoreError::StoreFailed(reason) => Self::StoreFailed(reason),
            other => Self::Io(btree::CowBtreeError::Storage(other).to_string()),
        }
    }
}

/// Internal storage result over the backend-native `StorageError`.
///
/// Backends produce this internally; the `Storage` trait impls map it to
/// `pglet_core::Error` at the boundary via `From<StorageError>`.
pub type Result<T> = std::result::Result<T, StorageError>;
