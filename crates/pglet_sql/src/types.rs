//! Value/type vocabulary, re-exported from the `pglet_core` leaf crate.
//!
//! The definitions live in `pglet_core::types`; this module re-exports them so
//! `crate::types::Value` and friends keep resolving across `pglet_sql`.

pub use pglet_core::types::*;
