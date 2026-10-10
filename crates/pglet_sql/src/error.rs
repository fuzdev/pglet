//! Error taxonomy, re-exported from the `pglet_core` leaf crate.
//!
//! The definitions live in `pglet_core::error`; this module re-exports them so
//! `crate::Error`/`crate::Result` and `pglet_sql::Error` keep resolving. The
//! `ErrorResponse` field builder (`pglet_core::error_fields`) rides along, because the
//! native server reaches its errors through `pglet_sql` and must not have to name the
//! leaf crate to send one.

pub use pglet_core::error::{
    CatalogError, ColumnDefaultDependency, DropTargetName, Error, IdentitySequenceDependency,
    RelationKind, Result, WrongTypeRoute,
};
pub use pglet_core::error_fields::{
    FailingRow, FkDirection, ForeignKeyViolationDetail, IMPLICIT_SCHEMA, KeyDescription,
    ObjectLocation, Severity, UniqueViolationDetail, plain_wire_fields,
};
