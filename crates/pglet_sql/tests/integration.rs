//! The `engine_*` integration tests, compiled as one test binary.
//!
//! Each file beside this one is a module of it, so a change to `pglet_sql` relinks one
//! executable rather than one per file. Filter by module: `cargo test -p pglet_sql
//! --test integration engine_dml::`.

mod common;

mod dml_target_probe;
mod engine_arrays;
mod engine_constraints;
mod engine_cte;
mod engine_ddl;
mod engine_declared_params;
mod engine_dml;
mod engine_index;
mod engine_introspection;
mod engine_subqueries;
mod index_key_order;
