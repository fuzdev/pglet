//! The embed facade's integration tests, compiled as one test binary.
//!
//! Each file beside this one is a module of it, so a change to a crate below `pglet`
//! relinks one executable rather than one per file. A file that needs a process of its
//! own is declared as its own `[[test]]` in `Cargo.toml` instead:
//!
//! - `crash.rs`, whose panic hook is process-wide;
//! - `catalog_depth.rs`, whose assertions read the process-wide stored-depth bound
//!   that any other test's catalog can raise;
//! - `stack_depth.rs`, whose assertions assume a thread spawned at a size runs in a
//!   mapping of that size. glibc hands a new thread a cached stack of up to four times
//!   the size it asks for, so the rest of the process's threads decide what it gets
//!   (measured: 1 run in 12 failed with every test in one process, 0 in 10 alone).
//!
//! Filter by module: `cargo test -p pglet --test integration fk_on_update::`.

mod common;

mod added_serial_column;
mod aggregate_walker_arms;
mod alter_add_column_backfill;
mod alter_add_column_fk;
mod alter_table_if_exists;
mod assignment_cast_verdicts;
mod autocommit_atomicity;
mod bool_list_identity;
mod boolean_context_verdicts;
mod case_folding;
mod clock;
mod column_default_verdicts;
mod comment_carriage_return;
mod container_key;
mod copy;
mod correct_or_loud_audit;
mod cte_column_list;
mod date;
mod descriptor_column_identity;
mod dml_target_probe_hosts;
mod do_block;
mod double_meta;
mod drop_dependencies;
mod durability;
mod enclosing_column_identity;
mod enum_identity_gates;
mod enum_reorder_index_specs;
mod fk_on_update;
mod fk_type_compatibility;
mod float_precision_selection;
mod generate_series;
mod identity_declaration;
mod in_block_index;
mod in_list_common_type;
mod index_registry_hygiene;
mod index_reopen;
mod index_row_size;
mod insert_clause_subplan_binding;
mod json_type_refused;
mod jsonb_reader_doors;
mod large_value;
mod numeric;
mod on_conflict_row_affected_twice;
mod page_fit;
mod parser_walls;
mod range_null_key;
mod regclass_cast_target;
mod returning_from_namespace;
mod returning_subquery;
mod schema_ddl;
mod scope_relation_identity;
mod sequence_catalog;
mod session_information;
mod set_quantifier_verdicts;
mod temporal_domain;
mod temporal_infinity;
mod time;
mod to_char;
mod type_namespace;
mod typmod_decomposition;
mod undetermined_param;
mod unnamed_index;
mod update_set_default;
mod writer;
