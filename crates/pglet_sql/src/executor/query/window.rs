//! Window-function (`OVER (...)`) execution.

use std::cmp::Ordering;

use crate::Result;
use crate::catalog::Schema;
use crate::planner::{Expr, OrderKey, Plan, WindowFunc};
use crate::types::{TypeOid, Value};

use super::super::Storage;
use super::super::expr::{
    EnumLeaves, EvalEnv, SimpleColumns, SubqueryExec, eval_expr_full, expr_enum_order_with,
    subquery_enum_order, values_compare,
};
use super::super::helpers::{ProjectionScope, column_enum_orders, expr_column_descs};
use super::super::types::ColumnDesc;
use super::{compare_order_values, execute_plan_node};

/// A window function's parts: `(func, partition_by, order_by)`.
type WindowParts<'a> = (WindowFunc, &'a [Expr], &'a [OrderKey]);

/// Execute a Window plan node: compute every window expression's value for every
/// input row.
pub(super) fn execute_window<S: Storage>(
    input: &Plan,
    (window_exprs, enclosing): (&[Expr], &crate::planner::EnclosingColumns),
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let (rows, input_columns) = execute_plan_node(input, schema, storage, env)?;
    let col_names: Vec<&str> = input_columns.iter().map(|c| c.name.as_str()).collect();
    // Declared input types, so a window-scope `||` / `::text` over a `timestamptz`
    // keeps its `+00` offset (see `ColumnResolver::column_type`).
    let col_types: Vec<TypeOid> = input_columns.iter().map(|c| c.type_oid).collect();
    // ...and their enum member orders, so a window `ORDER BY m` ranks by member
    // position (see `ColumnResolver::column_enum_order`).
    let col_enums = column_enum_orders(&input_columns, &schema.enums);

    // Every key and select-list item reads the input's names through this one resolver,
    // told its level's FROM names so a qualifier the level owns binds here and never an
    // enclosing frame of the same name.
    let level = super::scan::level_scope(input, &input_columns, schema);
    let resolver = SimpleColumns::new(&col_names)
        .with_types(&col_types)
        .with_enum_orders(&col_enums)
        .within_level(&level);
    // The output descriptors read a column through the same resolver, so a reference's
    // declared type is the type of the column its value comes from.
    let enclosing = enclosing.entries_copy();
    let output_columns = expr_column_descs(
        window_exprs,
        ProjectionScope::new(&input_columns, &col_names, None)
            .with_schema(schema)
            .with_resolver(&resolver)
            .with_enclosing(&enclosing),
    );

    // One output cell per (row, window expr), filled column by column. A window
    // function is computed once across all rows (its keys precomputed); a
    // non-window projection evaluates per row.
    let mut output_rows: Vec<Vec<Value>> = vec![Vec::with_capacity(window_exprs.len()); rows.len()];
    for expr in window_exprs {
        if let Some((func, partition_by, order_by)) = as_window_function(expr) {
            let column =
                compute_window_column(func, partition_by, order_by, &rows, &resolver, schema, env)?;
            for (cell, value) in output_rows.iter_mut().zip(column) {
                cell.push(value);
            }
        } else {
            for (cell, row) in output_rows.iter_mut().zip(&rows) {
                cell.push(eval_expr_full(
                    expr,
                    &resolver,
                    row,
                    SubqueryExec::NONE,
                    env,
                )?);
            }
        }
    }

    Ok((output_rows, output_columns))
}

/// Peel an optional alias and, if the expression is a window function, return its
/// `(func, partition_by, order_by)` parts — the single "is this a window function"
/// decision shared by the bare and `AS alias` forms (which previously each carried
/// a full, divergent copy of the computation).
fn as_window_function(expr: &Expr) -> Option<WindowParts<'_>> {
    let inner = match expr {
        Expr::Alias { expr, .. } => expr.as_ref(),
        other => other,
    };
    match inner {
        Expr::Window {
            func,
            partition_by,
            order_by,
        } => Some((*func, partition_by, order_by)),
        _ => None,
    }
}

/// Compute a window function's value for every row.
///
/// Each row's partition key and order key are evaluated **once** up front
/// (propagating any eval error via `?` rather than the old
/// `unwrap_or(Value::Null)` swallow that turned an error into NULL), then each
/// row's value is derived by indexing — so the partition/sort/rank logic never
/// re-evaluates and never silently loses an error.
///
/// `schema` is the enum walk's subquery leaf's, and its answer is unobservable in this
/// position for the same reason `order_key_enum_order`'s is: `row_number() OVER (ORDER
/// BY (SELECT …))` is `0A000` at `eval_key_rows`, which is lent no subquery callback
/// (`EDGES.md` row 89's class) — measured, where 18.6 answers. It is threaded so that
/// the window's order keys read the one walk rather than a second one that would be
/// wrong the day that class closes.
fn compute_window_column<'a>(
    func: WindowFunc,
    partition_by: &[Expr],
    order_by: &[OrderKey],
    rows: &[Vec<Value>],
    key_scope: &SimpleColumns<'a>,
    schema: &'a Schema,
    env: EvalEnv<'_>,
) -> Result<Vec<Value>> {
    let partition_refs: Vec<&Expr> = partition_by.iter().collect();
    let order_refs: Vec<&Expr> = order_by.iter().map(|k| &k.expr).collect();
    let partition_keys = eval_key_rows(&partition_refs, rows, key_scope, env)?;
    let order_keys = eval_key_rows(&order_refs, rows, key_scope, env)?;
    // The resolved `(ascending, nulls_first)` per key, plus each key's enum member
    // order — resolved once per key, through the same `SimpleColumns` `eval_key_rows`
    // binds the key's *value* through.
    let placements: Vec<(bool, bool, Option<&[String]>)> = order_by
        .iter()
        .map(|k| {
            (
                k.asc,
                k.nulls_first_resolved(),
                expr_enum_order_with(
                    &k.expr,
                    &EnumLeaves {
                        column: &|table, name| key_scope.scope_enum_order(table, name),
                        subquery: &|plan| subquery_enum_order(plan, schema),
                    },
                ),
            )
        })
        .collect();

    let mut out = Vec::with_capacity(rows.len());
    for i in 0..rows.len() {
        // The current row's partition, sorted by the ORDER BY keys (a stable sort,
        // so peers keep their input order — matching Postgres's row_number among
        // peers being arbitrary-but-consistent).
        let mut partition: Vec<usize> = (0..rows.len())
            .filter(|&j| keys_equal(&partition_keys[j], &partition_keys[i]))
            .collect();
        partition.sort_by(|&a, &b| compare_order_keys(&order_keys[a], &order_keys[b], &placements));

        let pos = partition.iter().position(|&j| j == i).unwrap_or(0);
        let value = match func {
            WindowFunc::RowNumber => Value::Int8(pos as i64 + 1),
            WindowFunc::Rank => {
                // The rank is the position of the first peer (a row with the same
                // ORDER BY key) plus one, so ties share a rank and gaps follow.
                let first_peer = partition
                    .iter()
                    .position(|&j| keys_equal(&order_keys[j], &order_keys[i]))
                    .unwrap_or(pos);
                Value::Int8(first_peer as i64 + 1)
            }
            WindowFunc::DenseRank => {
                // Distinct ORDER BY keys from the partition start through the
                // current position (no gaps).
                let mut seen: Vec<&[Value]> = Vec::new();
                for &j in &partition[..=pos] {
                    let key = order_keys[j].as_slice();
                    if !seen.iter().any(|s| keys_equal(s, key)) {
                        seen.push(key);
                    }
                }
                Value::Int8(seen.len() as i64)
            }
        };
        out.push(value);
    }
    Ok(out)
}

/// Evaluate a list of key expressions against every row, returning one key tuple
/// per row. Any eval error propagates (no swallow).
fn eval_key_rows(
    exprs: &[&Expr],
    rows: &[Vec<Value>],
    resolver: &SimpleColumns<'_>,
    env: EvalEnv<'_>,
) -> Result<Vec<Vec<Value>>> {
    rows.iter()
        .map(|row| {
            exprs
                .iter()
                .copied()
                .map(|e| eval_expr_full(e, resolver, row, SubqueryExec::NONE, env))
                .collect::<Result<Vec<_>>>()
        })
        .collect()
}

/// Compare two order-key tuples, honoring each key's direction and (resolved) NULL
/// placement. `placements[i]` is the `(ascending, nulls_first, enum_order)` of the i-th
/// key.
fn compare_order_keys(
    a: &[Value],
    b: &[Value],
    placements: &[(bool, bool, Option<&[String]>)],
) -> Ordering {
    for ((av, bv), &(ascending, nulls_first, enum_order)) in a.iter().zip(b).zip(placements) {
        let ord = compare_order_values(av, bv, ascending, nulls_first, enum_order);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// Check if two key value vectors are equal (NULLs are equal here, for both
/// partitioning and peer/tie detection).
fn keys_equal(a: &[Value], b: &[Value]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (av, bv) in a.iter().zip(b.iter()) {
        if av.is_null() && bv.is_null() {
            continue;
        }
        if av.is_null() || bv.is_null() {
            return false;
        }
        if !matches!(values_compare(av, bv), Some(Ordering::Equal)) {
            return false;
        }
    }
    true
}
