//! pglet's SQL front end and executor: the lexer and native parser, the planner, the executor,
//! and the catalog. Every host reaches it through the `pglet` crate's `Database`; the
//! engine-wide rules it implements are stated in the repository's `docs/`.

pub mod builtins;
pub mod catalog;
pub mod deparse;
pub mod depth;
pub mod error;
pub mod executor;
pub mod gin;
pub mod host;
pub mod lexer;
pub mod parser;
pub mod planner;
pub mod types;

pub use error::{
    CatalogError, ColumnDefaultDependency, DropTargetName, Error, FailingRow, FkDirection,
    ForeignKeyViolationDetail, IMPLICIT_SCHEMA, IdentitySequenceDependency, KeyDescription,
    ObjectLocation, RelationKind, Result, Severity, UniqueViolationDetail, WrongTypeRoute,
    plain_wire_fields,
};
