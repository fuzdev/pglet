//! Expression evaluation for query execution.
//!
//! This module is organized as follows:
//! - `env.rs` - the evaluation environment: subquery callbacks, `EvalEnv`, outer frames
//! - `columns.rs` - `ColumnResolver`, `SimpleColumns`, the join column map and `eval_expr_join`
//! - `eval.rs` - the core walk (`eval_expr_full`, the `eval_*_node` evaluators), column and
//!   builtin resolution, the declared-type leaf scope
//! - `enum_order.rs` - enum identity and ordering: the enum-leaf walk and its predicates
//! - `binary_op.rs` - binary operators, connectives, `AT TIME ZONE`, `to_char`, negation
//! - `arithmetic.rs` - arithmetic, interval/timestamp arithmetic, concatenation, casts
//! - `compare.rs` - boolean contexts, equality and ordering, `LIKE` / regex
//! - `json_ops.rs` - the jsonb operators and containment
//! - `math_fn.rs` - numeric math functions
//! - `string_fn.rs` - string, catalog-information and variadic functions
//! - `temporal_fn.rs` - `EXTRACT`, `date_part`, `date_trunc`
//! - `tests/` - the unit tests, one file per concern

mod arithmetic;
mod binary_op;
mod columns;
mod compare;
mod enum_order;
mod env;
mod eval;
mod json_ops;
mod math_fn;
mod string_fn;
mod temporal_fn;

#[cfg(test)]
mod tests;

pub use arithmetic::{
    float4_add, float8_add, interval_add, interval_scale, value_as_i64, value_to_f64_opt,
};
pub use binary_op::eval_binary_op;
#[cfg(feature = "index_acceleration")]
pub use binary_op::eval_binary_op_ordered;
pub use columns::{
    ColumnRef, ColumnResolver, JoinColumnMap, LevelScope, ScopeIdentity, SimpleColumns,
    eval_expr_join,
};
pub use compare::{
    boolean_context_operand, predicate_holds, values_compare, values_compare_ordered, values_equal,
};
pub use enum_order::{
    EnumLeaves, expr_enum_order_with, expr_enum_with, subquery_enum_name, subquery_enum_order,
    unify_enum_pair,
};
pub use env::{EvalEnv, MaterializedCte, OuterFrame, SubqueryExec};
pub use eval::{eval_expr, eval_expr_env, eval_expr_full, literal_to_value};
pub use string_fn::eval_advisory_xact_lock;
