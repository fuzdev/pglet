//! The transaction-block state a session reports and a host acts on.
//!
//! It lives in the leaf crate because three layers read the same three values and
//! must agree on them exactly: the embed facade's session
//! (`pglet::TransactionSession`), the native server's per-connection session, and
//! the wire codec, whose `ReadyForQuery` carries the status byte. Mirroring it —
//! one enum per layer plus a conversion — is the duplicated-rules seam this
//! workspace keeps paying for, and here the two copies would encode the same
//! Postgres state machine.

/// Where a session's transaction block stands, in Postgres's own three states.
///
/// The wire spelling is `ReadyForQuery`'s status byte: `I` / `T` / `E`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    /// No transaction block open — every statement autocommits.
    Idle,
    /// Inside a healthy block: writes stage until `COMMIT`.
    InTransaction,
    /// Inside an **aborted** block: a statement raised, so every statement but
    /// one that ends the block answers [`crate::Error::AbortedTransaction`]
    /// (`25P02`) and the eventual `COMMIT` is converted to a `ROLLBACK`.
    Failed,
}
