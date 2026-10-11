//! Correlated- and non-correlated-subquery support: the subquery executors
//! (scalar / IN / EXISTS) and the subquery-capable expression evaluator that
//! drives them.
//!
//! Correlation is resolved **at eval time** via the evaluation environment
//! ([`EvalEnv`]): descending into a subquery pushes the current scope as a new
//! outer frame, and a correlated `Expr::Column` the inner scope doesn't claim
//! resolves against that frame (see `resolve_column` in `executor::expr`). The
//! inner sub-plan runs unchanged under the child env — no per-outer-row clone.

use crate::Result;
use crate::catalog::Schema;
use crate::planner::{Expr, Plan};
use crate::types::{TypeOid, Value};

use super::super::Storage;
use super::super::expr::{
    ColumnResolver, EvalEnv, OuterFrame, ScopeIdentity, SimpleColumns, SubqueryExec, eval_expr_full,
};
use super::execute_plan_node;

/// Evaluate an expression with full subquery support (scalar / IN / EXISTS).
///
/// Provides the schema/storage context the embedded subqueries need. `identity` says
/// what the scope knows about the relations behind `col_names` (see `ScopeIdentity`),
/// enabling alias-aware resolution of a qualified column reference. `col_types` is the
/// scope's declared column types (positionally parallel to `col_names`), or empty when
/// the caller has none to hand — see `ColumnResolver::column_type`.
///
/// **`ScopeIdentity::Anonymous` is a claim about the scope, not a way of declining to
/// compute the name.** Both consumers read it as *this scope has no relation identity*
/// and switch resolution rules on it — `SimpleColumns::resolve` drops the qualifier and
/// matches the bare name, and the pushed `OuterFrame` is offered the synthesized
/// `"relation.column"` tier that a user's quoted `"a.b"` column is byte-identical to. A
/// caller that has a name and passes `Anonymous` therefore does not merely lose
/// alias-awareness; it makes the scope answer for relations the statement never named.
/// What the callers pass today: `Plan::Filter` / `Plan::Project` pass `scope_refs`'s
/// verdict — `Single` for a single-relation input, `Relations` for a join whose pairs
/// line up with the executed columns, `Anonymous` for the plan kinds that are not FROM
/// items and for a join scope whose pairs could not be derived; `scan_matching_rows`
/// passes its DML target's effective name; `project_returning` passes the sole relation
/// of a single-relation `RETURNING` namespace and that namespace's pairs for a
/// multi-relation one; and the no-scope callers (`sequence.rs`, the `ProjectUnnest` /
/// `Values` arms) pass an empty `col_names` beside it, so their frame claims nothing
/// either way.
///
/// Correlation: the current scope `(col_names, col_types, identity, row)` is pushed as
/// a new outer frame onto `env`, and the subquery callbacks run the **unmodified** inner
/// plan under that child env. A correlated `Expr::Column` inside the subquery the
/// inner scope doesn't own resolves against this frame at eval time — no per-row
/// plan clone, no outer-ref substitution. A non-correlated subquery simply never
/// reads the frame, so it behaves exactly as before. `env` also carries the bound
/// parameters, so a `$N` *inside* the subquery resolves too.
// The execution-context triple (schema/storage/env) plus the scope quadruple
// (expr/names/types/identity/row) is threaded individually, as across the executor —
// bundling it would diverge from every callsite's shape.
#[allow(clippy::too_many_arguments)]
pub fn eval_expr_subquery<S: Storage>(
    expr: &Expr,
    col_names: &[&str],
    col_types: &[TypeOid],
    col_enums: &[Option<&[String]>],
    identity: ScopeIdentity<'_>,
    row: &[Value],
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<Value> {
    eval_expr_subquery_with(
        expr,
        &SimpleColumns::over_scope(col_names, identity)
            .with_types(col_types)
            .with_enum_orders(col_enums),
        col_names,
        col_types,
        col_enums,
        identity,
        row,
        schema,
        storage,
        env,
    )
}

/// [`eval_expr_subquery`] over an explicit `resolver`, for a scope whose resolution
/// rule a `SimpleColumns` cannot spell.
///
/// The `RETURNING` clause of an `UPDATE … FROM` is the one such scope: its namespace is
/// relation-keyed (`(relation, column)` pairs — see `executor::helpers`'s
/// `project_returning`), because the flat `relation.column` name list a `SimpleColumns`
/// needs is forgeable by a quoted user column with a dot in it.
///
/// `col_names` / `col_types` / `identity` still describe the same row, and are what the
/// **outer frame** a correlated subquery resolves against is built from — so the frame
/// and `resolver` agree about what the scope is even where their resolution rules
/// differ.
// The execution-context triple plus the scope quintuple is threaded individually, as
// across the executor — bundling it would diverge from every callsite's shape.
#[allow(clippy::too_many_arguments)]
pub fn eval_expr_subquery_with<S: Storage, R: ColumnResolver>(
    expr: &Expr,
    resolver: &R,
    col_names: &[&str],
    col_types: &[TypeOid],
    col_enums: &[Option<&[String]>],
    identity: ScopeIdentity<'_>,
    row: &[Value],
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<Value> {
    // Lend the catalog here as well as at `execute_plan_node`: this is the eval entry a
    // DML clause takes (an `UPDATE`/`DELETE` `WHERE`, a `RETURNING` item), and those
    // never descend through a plan node, so a catalog-reading expression written there
    // would otherwise be `0A000` while the identical expression in a `SELECT` answered.
    // It goes on the **outer** env, not only on `child`: `child` is the frame a nested
    // subquery sees, and the expression itself is evaluated against `env`. The store
    // goes with it, for `pg_column_size` over an `UPDATE` / `DELETE` target's row.
    let env = env.with_schema(schema).with_toast_status(storage);
    let frame = OuterFrame::over_scope(
        col_names,
        col_types,
        col_enums,
        identity,
        row,
        env.outer_scope(),
    );
    let child = env.push(&frame);
    let scalar_fn = |plan: &Plan| execute_scalar_subquery(plan, schema, storage, child);
    let in_list_fn = |plan: &Plan| execute_in_subquery(plan, schema, storage, child);
    let exists_fn = |plan: &Plan| execute_exists_subquery(plan, schema, storage, child);
    eval_expr_full(
        expr,
        resolver,
        row,
        SubqueryExec {
            scalar: Some(&scalar_fn),
            in_list: Some(&in_list_fn),
            exists: Some(&exists_fn),
            aggregate: None,
        },
        env,
    )
}

/// Execute a scalar subquery and return a single value.
///
/// A scalar subquery must return at most one row with one column.
/// - If the subquery returns no rows, returns `Value::Null`.
/// - If the subquery returns one row with one column, returns that value.
/// - If the subquery returns multiple rows or columns, returns an error.
pub fn execute_scalar_subquery<S: Storage>(
    plan: &Plan,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<Value> {
    let (rows, columns) = execute_plan_node(plan, schema, storage, env)?;

    // Check column count
    if columns.len() > 1 {
        return Err(crate::Error::NotImplemented(
            "subquery must return only one column".into(),
        ));
    }

    // Check row count and return appropriate value
    match rows.len() {
        0 => Ok(Value::Null),
        1 => rows.first().map_or(Ok(Value::Null), |first_row| {
            Ok(first_row.first().cloned().unwrap_or(Value::Null))
        }),
        _ => Err(crate::Error::NotImplemented(
            "subquery returned more than one row".into(),
        )),
    }
}

/// Execute an IN subquery and return all values from the first column.
///
/// Used for `expr IN (SELECT ...)` evaluation. The subquery must return exactly
/// one column. Returns a Vec of all first-column values from all rows.
fn execute_in_subquery<S: Storage>(
    plan: &Plan,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<Vec<Value>> {
    let (rows, columns) = execute_plan_node(plan, schema, storage, env)?;

    // Check column count
    if columns.len() != 1 {
        return Err(crate::Error::NotImplemented(
            "IN subquery must return exactly one column".into(),
        ));
    }

    // Extract first column value from each row
    Ok(rows
        .into_iter()
        .map(|row| row.into_iter().next().unwrap_or(Value::Null))
        .collect())
}

/// Execute an `EXISTS(SELECT ...)` sub-plan and report whether it returned any
/// rows.
///
/// Position-agnostic: returns the bare existence (`!rows.is_empty()`), with the
/// `NOT EXISTS` negation applied by the caller. Unlike `execute_scalar_subquery`,
/// the inner query may project any number of columns — only row presence matters
/// (so `SELECT 1`, `SELECT *`, and multi-column inner queries all work).
pub fn execute_exists_subquery<S: Storage>(
    plan: &Plan,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<bool> {
    let (rows, _columns) = execute_plan_node(plan, schema, storage, env)?;
    Ok(!rows.is_empty())
}
