//! Query planning IR + the lowering helpers the native parser (`parser`) fuses into.
//!
//! The parser emits these `Plan`/`Expr` nodes directly; this module owns the IR
//! types and the shared lowering/selection logic both the parser and executor
//! depend on.
//!
//! This module is organized as follows:
//! - `plan.rs` - Plan enum and `ColumnDef` types
//! - `expr.rs` - Expr, Literal, `BinaryOp`, `AggregateFunc` IR + `build_call`
//! - `ddl.rs` - `is_serial_type` (the rest of DDL planning lives in the parser)
//! - `dml.rs` - INSERT/UPDATE/DELETE lowering (`lower_insert`/`lower_update`/`lower_delete`)
//! - `query.rs` - SELECT lowering (`lower_select`: index selection, aggregates, ORDER BY)
//! - `index_selection.rs` - Predicate + schema -> chosen index / bounds / keys
//! - `coercion.rs` - Type resolution and implicit casts
//! - `correlation.rs` - Correlated-subquery re-resolution against the enclosing scope

mod coercion;
// Postgres's two type-resolution rulebooks: `select_common_type` (COALESCE / CASE /
// GREATEST / ARRAY / set-ops) and NULLIF's operator-resolution rule, which disagree.
mod common_type;
// Re-resolution of a nested query level's expressions against the enclosing
// query's relations (`Expr::Subquery`/`Exists`/`InSubquery` sub-plans).
mod correlation;
pub use correlation::resolve_correlated_subplans;
mod ddl;
mod dml;
mod expr;
mod function_scan;
// Query-time index selection (`col = v` → IndexScan, ranges → IndexRangeScan,
// `@>` → GinScan). Off in the stripped "testing" profile — the planner then emits
// `Filter(Scan)` for every predicate (intended same rows, full scan).
#[cfg(feature = "index_acceleration")]
mod index_selection;
#[cfg(feature = "index_acceleration")]
pub(crate) use index_selection::flatten_and;
mod legality;
mod param_types;
mod plan;
mod query;
mod unknown_types;

// Re-export public types
pub use coercion::{
    ColumnScope, build_column_type_map, build_desc_type_map, certain_from_argument_type,
    certain_unnest_element_type, check_assignment, coerce_to_type, column_type_map_for,
    resolve_entries_gated, resolve_expr_types, resolve_expr_types_checked,
    resolve_expr_types_gated, resolve_expr_types_no_columns, validate_arg_const_casts,
    validate_const_casts,
};
pub use common_type::{
    EnumIdentity, ResolvedType, TypeCategory, common_type_step, comparison_operand_type,
    foreign_key_types_compatible, implicitly_coercible, nullif_result_type, resolved_category,
    resolved_coercible, select_common_type, select_common_typmod, variadic_common_type,
    variadic_common_type_oid,
};
pub use expr::{
    AggregateFunc, ArgCertainty, BinaryOp, BoolContext, BoolTest, BuiltinFn, ColumnSizeOrigin,
    Expr, ExtractField, Literal, MathFunc, OrderKey, SequenceFunc, SqlValueFunction, StringFunc,
    Typmod, VariadicFunc, WindowFunc, aggregate_result_type, aggregate_type_of, arith_result_type,
    binary_op_result_type, build_enum_cast, builtin_fn_result_type, date_trunc_result_type,
    deserialize_expr, enum_in_arg_parts, enum_in_holds, enum_in_parts, expr_contains_subquery,
    int_width_result, math_fn_result_type, numeric_family_result, pow_result_type, serialize_expr,
    shared_leaf_cell_type, shared_multi_operand_type, string_fn_result_type, temporal_arith_type,
    variadic_fn_result_type,
};
pub use function_scan::{FromFunction, MAX_SERIES_ROWS, SeriesType, resolve_generate_series};
pub use legality::{
    aggregate_legality, arithmetic_legality, arithmetic_symbol, array_element_resolved_type,
    assignment_legality, boolean_context_legality, boolean_context_literal,
    bound_param_enum_legality, branch_unification_legality, check_boolean_operand,
    check_expr_node_operand_types, comparison_enum_literal_legality, comparison_legality,
    comparison_symbol, concat_legality, concat_undefined, containment_legality, containment_symbol,
    enum_in_constant_legality, enum_in_label_legality, enum_label_legality, enum_literal_legality,
    json_get_legality, json_operator_legality, json_operator_symbol, like_legality, like_symbol,
    math_fn_arg_legality, mirrored_comparison_op, negate_legality, one_sided_comparison_legality,
    regex_legality, regex_symbol, starts_with_legality, unification_branches, unknown_aware_type,
};
pub use param_types::{ParamTargets, infer_param_targets, infer_param_types};
pub use plan::{
    AddConstraint, CertainTypes, CheckConstraintDef, ColumnDef, ConflictTarget, CopyDirection,
    CopyFormat, CopyOptions, CteBody, CtePlan, DoExceptionHandler, EnclosingColumn,
    EnclosingColumns, ForeignKeyDef, IndexBound, InsertRows, JoinType, LeafScope, LimitCount,
    NoopCommand, OnConflictAction, OnConflictClause, Overriding, Plan, PlannerScope,
    SequenceOwnerSpec, SequenceRestart, SetOpKind, SetOpOuterTypes, SideReferenceTypes, TypeScope,
    UniqueConstraint, UnperformedAlter, UsingMatch, alias_relation_columns, find_enclosing,
    function_scan_column_types, match_names, scan_effective_name, using_column_match,
};
// Reused by the native parser (`parser`): the shared function-call dispatch
// (`build_call`), the INSERT/UPDATE/DELETE/SELECT lowering helpers, and the
// type-resolution / serial-detection utilities.
pub(crate) use ddl::is_serial_type;
pub(crate) use dml::{
    InsertRowSource, claim_target_column, lower_delete, lower_insert, lower_update,
};
pub(crate) use expr::{
    SetQuantifier, build_call, refuse_from_unnest_quantifier, refuse_window_arguments,
    transform_call_args,
};
pub(crate) use query::{
    SelectParts, SetOpTyping, build_set_op, contains_aggregate, contains_unnest, contains_window,
    lower_select, plan_no_from_distinct_on, plan_no_from_order_limit, plan_set_op_order_limit,
    unify_values_columns, unknown_entry_enum_legality, unknown_output_columns,
};
pub use unknown_types::{coerce_unknown_types, set_coerce_unknown_types, unsupported_type_oid};

// `clippy::panic` has no test-allowance in this clippy (only unwrap/expect do), so the
// panic!-based assertions below need an explicit allow on the test module.
#[cfg(test)]
#[allow(clippy::panic)]
mod tests;
