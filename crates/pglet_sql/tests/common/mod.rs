//! Shared harness for the `engine_*` integration tests.
//!
//! Thin re-export of [`pglet_testing::oracle`] (the `MemStorage` scan-and-filter
//! oracle, the `TestDb` driver, and the seed/setup helpers), declared once by
//! `tests/integration.rs`, while the implementations live in one place.

pub use pglet_testing::oracle::*;
