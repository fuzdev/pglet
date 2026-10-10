//! Shared engine-assembly helpers for the `pglet` integration tests.
//!
//! Thin re-export of [`pglet_testing::db`], declared once by each test binary that
//! uses it (`tests/integration.rs`, `tests/crash.rs`), while the implementations live
//! in one place. Each binary compiles its own copy, so one using only a subset of the
//! re-exports would otherwise warn.
#![allow(unused_imports)]

pub use pglet_testing::db::{ScratchDir, open_database};
