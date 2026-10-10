//! Leaf vocabulary for pglet: the runtime `Value`/`TypeOid` representation and its codecs
//! (text, binary, index key) and JSON parser, the `Error`/`CatalogError` taxonomy, the
//! `Storage` trait with its `RowId`/`IndexSpec` identifiers, the server-encoding scan
//! (`encoding`), and the `TransactionStatus` a session reports.
//!
//! This crate sits at the bottom of the workspace dependency graph (only
//! serde/time/thiserror), so every other pglet crate can depend *down* on it
//! and share one definition of these types rather than re-rolling them.
//!
//! Being the one crate everything can reach, it is also where a primitive lands
//! that two otherwise-unrelated crates must implement identically:
//! [`file_lock`] is the workspace's single exclusive-file-lock rule, taken by
//! the `pglet_server` `--data-dir` guard and by the `xtask` run-lock. It is
//! behind the default-off `file_lock` feature, so the wasm build never compiles
//! its `std::fs` path. [`encoding`] is another: the server-encoding scan the
//! literal decoder, COPY, a bound parameter on the native server and on the wasm host,
//! and the dump's COPY payload reach.

pub mod encoding;
pub mod error;
pub mod error_fields;
#[cfg(feature = "file_lock")]
pub mod file_lock;
pub mod spec;
pub mod storage;
pub mod transaction;
pub mod types;

pub use error::*;
pub use error_fields::*;
pub use spec::*;
pub use storage::*;
pub use transaction::*;
pub use types::*;
