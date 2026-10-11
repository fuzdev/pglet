//! Query (SELECT) execution.
//!
//! Handles SELECT queries including scans, filters, projections, sorting, and limits.
//!
//! This module is organized as follows:
//! - `mod.rs` (here) — the `execute_query` entry point, the recursive
//!   `execute_plan_node` dispatch, the `unnest`/VALUES table-source handling, and
//!   small shared helpers (`resolve_limit_count`, `compare_order_values`).
//! - `scan.rs` — `Scan`/`IndexScan`/`IndexRangeScan`/`VirtualScan` execution and
//!   the single-table-name / inner-FROM column/table resolution helpers.
//! - `join.rs` — nested-loop JOIN execution.
//! - `aggregate.rs` — `GROUP BY`/`HAVING`/aggregate-function execution.
//! - `dedup.rs` — the keyed path of duplicate elimination and grouping (`DISTINCT`,
//!   `UNION`, `INTERSECT`, `EXCEPT`, a recursive CTE's `UNION`, an aggregate's
//!   `DISTINCT`, `GROUP BY`).
//! - `window.rs` — window-function (`OVER (...)`) execution.
//! - `correlated.rs` — correlated/non-correlated subquery support: outer-row
//!   substitution and the scalar/IN/EXISTS subquery executors.

mod aggregate;
mod correlated;
mod dedup;
mod join;
mod scan;
mod window;

pub use correlated::{eval_expr_subquery, eval_expr_subquery_with};
pub use join::get_table_name_from_plan;
#[cfg(feature = "index_acceleration")]
pub use scan::probe_dml_targets;

use std::cmp::Ordering;

use crate::catalog::Schema;
use crate::planner::{
    BoolContext, EnclosingColumns, Expr, FromFunction, LimitCount, Literal, MAX_SERIES_ROWS, Plan,
    SeriesType, SetOpKind, alias_relation_columns,
};
use crate::types::{ResultShape, TypeOid, Value};
use crate::{Error, Result};

use super::Storage;
use super::expr::{
    ColumnRef, EnumLeaves, EvalEnv, MaterializedCte, ScopeIdentity, SimpleColumns, SubqueryExec,
    eval_expr_env, eval_expr_full, expr_enum_order_with, predicate_holds, subquery_enum_order,
    values_compare_ordered, values_equal,
};
use super::helpers::{ProjectionScope, column_enum_orders, expr_column_descs, unified_enum_type};
use super::types::{ColumnDesc, QueryResult};

/// The scope identity `scan::scope_refs`'s two halves describe: a single relation's
/// effective name, else a join's `(relation, column)` pairs, else none.
///
/// The two are mutually exclusive by construction — `scope_refs` derives the pairs only
/// where there is no single relation — so this is the one place the pair is read as an
/// identity, and a caller cannot hold both.
const fn scope_identity<'a>(
    table: Option<&'a str>,
    refs: Option<&'a [ColumnRef]>,
) -> ScopeIdentity<'a> {
    match refs {
        Some(refs) => ScopeIdentity::Relations(refs),
        None => ScopeIdentity::from_table(table),
    }
}

/// Resolve a `LIMIT`/`OFFSET` count to a row count.
///
/// `None` → no bound; `Const(n)` → `n`; `Param(i)` → resolved from the evaluation
/// environment at execution time (NULL clears the bound, negative fails loud).
fn resolve_limit_count(count: Option<&LimitCount>, env: EvalEnv<'_>) -> Result<Option<usize>> {
    match count {
        None => Ok(None),
        Some(LimitCount::Const(n)) => Ok(Some(*n)),
        Some(LimitCount::Param(i)) => env.limit_param(*i),
    }
}

/// Evaluate one `ORDER BY` sort key against a result row.
///
/// A bare integer literal is SQL's **positional** reference (`ORDER BY <n>`) — the
/// 1-based Nth output column — resolved by *index* rather than routed through name
/// resolution. This matters for a set-op whose unnamed columns share the name
/// `?column?`: a by-name reference to one of them is ambiguous (`42702`), but a
/// positional reference is not. An ordinal outside `1..=row.len()` fails loud
/// (`42P10`, matching Postgres's `ORDER BY position <n> is not in select list`).
/// Every other key evaluates as an ordinary expression through `resolver`, the Sort's
/// scope over its input's names and its level's FROM names.
fn eval_order_key(
    expr: &Expr,
    resolver: &SimpleColumns<'_>,
    row: &[Value],
    env: EvalEnv<'_>,
) -> Result<Value> {
    if let Expr::Literal(Literal::Int(n)) = expr {
        return usize::try_from(*n)
            .ok()
            .filter(|&i| i >= 1 && i <= row.len())
            .map_or_else(
                || {
                    Err(Error::InvalidColumnPosition {
                        clause: "ORDER BY",
                        position: *n,
                    })
                },
                |i| Ok(row[i - 1].clone()),
            );
    }
    eval_expr_full(expr, resolver, row, SubqueryExec::NONE, env)
}

/// The enum member order an `ORDER BY` key's values are compared by — resolved once per
/// key, because the comparator sees only `Value`s.
///
/// **Mirrors [`eval_order_key`]'s own two-way split, and has to keep mirroring it.** A
/// bare integer literal is SQL's positional reference, so that key's values come from
/// `row[n - 1]` and its order is *that column's* — which no expression walk can see, and
/// a set operation's trailing `ORDER BY 2` is the shape that reaches it. Every other key
/// resolves through `scope`, by the one enum-passing walk (`expr_enum_order_with`).
///
/// **The `schema` is here for the walk's subquery leaf, whose answer nothing in this
/// position can observe today** — and that is deliberate rather than an oversight. A key
/// holding a scalar subquery is `0A000` at `eval_order_key`, which is lent no subquery
/// callback (`EDGES.md` row 89's class), so the leaf is asked and its answer thrown away
/// with the row. Threading it anyway is what keeps this an instantiation of the *one*
/// walk rather than a second, differently-shaped one: the alternative is a blind leaf
/// here that would answer wrongly the day that class closes, silently.
fn order_key_enum_order<'a>(
    expr: &Expr,
    col_enums: &[Option<&'a [String]>],
    scope: &SimpleColumns<'a>,
    schema: &'a Schema,
) -> Option<&'a [String]> {
    if let Expr::Literal(Literal::Int(n)) = expr {
        let i = usize::try_from(*n).ok()?;
        return *col_enums.get(i.checked_sub(1)?)?;
    }
    expr_enum_order_with(
        expr,
        &EnumLeaves {
            column: &|table, name| scope.scope_enum_order(table, name),
            subquery: &|plan| subquery_enum_order(plan, schema),
        },
    )
}

/// Compare two values for an ORDER BY term, honoring the sort direction and the
/// (already-resolved) NULL placement.
///
/// The single shared comparator for every `ORDER BY` consumer — the `Plan::Sort`
/// executor, the window `ORDER BY`, `DISTINCT ON`'s effective sort, and a set-op
/// trailing `ORDER BY` — so their NULL handling can't drift. `nulls_first` is
/// already resolved to a concrete bool by [`crate::planner::OrderKey::nulls_first_resolved`], which
/// applies Postgres's direction default (`ASC` → NULLS LAST, `DESC` → NULLS FIRST)
/// when the query gave no explicit `NULLS FIRST/LAST`.
///
/// NULL placement is **independent** of the sort direction — Postgres applies
/// `NULLS FIRST/LAST` *after* `ASC/DESC` — so a NULL-vs-non-NULL comparison follows
/// `nulls_first` verbatim and is **not** reversed for `DESC`; only the
/// non-NULL-vs-non-NULL comparison flips. Non-NULL values delegate to
/// [`values_compare_ordered`], itself a total order over float `NaN` (NaN sorts
/// greatest and `NaN == NaN`, per `float8_cmp_internal`), so no NaN special-case is
/// needed here.
///
/// `enum_order` is the key's enum member list, resolved **once per key** by the caller
/// (the comparator sees only `Value`s, and a key's enum identity is a property of the
/// key *expression*). `None` for every non-enum key, and the comparison is then
/// byte-identical to what it was.
fn compare_order_values(
    a: &Value,
    b: &Value,
    ascending: bool,
    nulls_first: bool,
    enum_order: Option<&[String]>,
) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if nulls_first {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, true) => {
            if nulls_first {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, false) => {
            let ord = values_compare_ordered(a, b, enum_order).unwrap_or(Ordering::Equal);
            if ascending { ord } else { ord.reverse() }
        }
    }
}

/// Execute a SELECT query.
pub fn execute_query<S: Storage>(
    plan: &Plan,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<QueryResult> {
    // Execute the plan tree recursively to get rows
    let (rows, col_info) = execute_plan_node(plan, schema, storage, env)?;

    let row_count = rows.len();
    Ok(QueryResult {
        shape: ResultShape::Columns(col_info),
        rows,
        command_tag: format!("SELECT {row_count}"),
        notices: Vec::new(),
        tx_command: None,
        swapped_sequences: Vec::new(),
    })
}

/// A hard cap on recursive-CTE iterations. Postgres has no depth limit (it relies on
/// the query terminating), but pglet must not hang the process on a runaway
/// `UNION ALL` with no stop condition — exceeding this fails loud (`54000`,
/// `program_limit_exceeded`) rather than looping forever or silently truncating. Set
/// far above any realistic recursion depth.
const MAX_RECURSIVE_ITERATIONS: usize = 100_000;

/// Materialize one recursive CTE by the fixed-point (semi-naive) algorithm, with the
/// entries `bound` holds (the statement's earlier CTEs, already materialized) visible to
/// both terms.
///
/// (1) The non-recursive (seed) term seeds the **working table** and the accumulated
/// result. (2) The recursive term is re-evaluated with the CTE `name` bound to the
/// *current* working table (a [`MaterializedCte`] pushed onto `bound` for the one
/// evaluation and popped after it — the same relation-rebinding primitive a
/// data-modifying CTE uses); its new rows become the next working table and extend the
/// result, until an iteration produces zero new rows (or [`MAX_RECURSIVE_ITERATIONS`]
/// trips). Under `UNION` (`!all`) a new row is kept only if not already accumulated
/// (whole-row NULL-equal dedup — so a cyclic graph terminates); under `UNION ALL` every
/// row is kept. (3) The accumulated relation is returned for the caller to bind.
///
/// `bound` holds the earlier entries only, and is left as it was found: a later entry
/// reads this one's **finished** relation, never a working table.
///
/// The recursive term's rows are cast to the non-recursive column types each
/// iteration ([`cast_rows_to_types`]) — the asymmetric-typing rule (non-recursive
/// authoritative), already validated castable at plan time.
#[allow(clippy::too_many_arguments)]
pub fn materialize_recursive_cte<S: Storage>(
    name: &str,
    columns: &[ColumnDesc],
    non_recursive: &Plan,
    recursive: &Plan,
    all: bool,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
    bound: &mut Vec<MaterializedCte>,
) -> Result<MaterializedCte> {
    let target_types: Vec<TypeOid> = columns.iter().map(|c| c.type_oid).collect();

    // (1) Seed from the non-recursive term.
    let (seed_rows, seed_cols) =
        execute_plan_node(non_recursive, schema, storage, env.with_ctes(bound))?;
    let seed_rows = cast_rows_to_types(seed_rows, &seed_cols, &target_types)?;
    let mut accumulated: Vec<Vec<Value>> = Vec::new();
    let mut working: Vec<Vec<Value>> = Vec::new();
    // `UNION`'s dedup across every iteration, `dedup_rows`'s rule over `accumulated`.
    let mut keys = dedup::DistinctKeys::new(target_types.len());
    let mut fresh = |row: &[Value], accumulated: &[Vec<Value>]| {
        all || keys.first_sighting(row).unwrap_or_else(|| {
            !accumulated
                .iter()
                .any(|seen| rows_distinct_equal(seen, row))
        })
    };
    for row in seed_rows {
        if fresh(&row, &accumulated) {
            accumulated.push(row.clone());
            working.push(row);
        }
    }

    // (2) Iterate the recursive term until it yields no new rows.
    let mut iterations = 0usize;
    while !working.is_empty() {
        iterations += 1;
        if iterations > MAX_RECURSIVE_ITERATIONS {
            return Err(Error::ProgramLimitExceeded(format!(
                "recursive query \"{name}\" exceeded the maximum of \
                 {MAX_RECURSIVE_ITERATIONS} iterations"
            )));
        }
        bound.push(MaterializedCte {
            name: name.to_string(),
            columns: columns.to_vec(),
            rows: std::mem::take(&mut working),
        });
        let evaluated = execute_plan_node(recursive, schema, storage, env.with_ctes(bound));
        bound.pop();
        let (new_rows, new_cols) = evaluated?;
        let new_rows = cast_rows_to_types(new_rows, &new_cols, &target_types)?;
        for row in new_rows {
            if fresh(&row, &accumulated) {
                accumulated.push(row.clone());
                working.push(row);
            }
        }
    }

    // (3) The finished relation, for the caller to bind.
    Ok(MaterializedCte {
        name: name.to_string(),
        columns: columns.to_vec(),
        rows: accumulated,
    })
}

/// Execute a plan node recursively.
///
/// `env` threads the bound parameters (and, for correlated subqueries, the outer
/// scope) so an `Expr::Parameter` resolves at eval time rather than via plan
/// mutation. Forwarded unchanged to every child node.
///
/// It is also where the **catalog** joins the environment. `pg_get_constraintdef(oid)`
/// and the `::regclass` cast are functions of the catalog, not of the row, and this is
/// the one chokepoint every read path descends through — so lending the schema here
/// (rather than at each of the entry points that call it) is what keeps a subquery, a
/// join arm and a `RETURNING` list from disagreeing about whether those expressions can
/// be answered at all.
pub fn execute_plan_node<S: Storage>(
    plan: &Plan,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    crate::depth::check_stack()?;
    // the store too, for `pg_column_size`'s stored values
    let env = env.with_schema(schema).with_toast_status(storage);
    // The nodes a plan chain recurses through run their inputs from this frame and do
    // their own work in one of its own, so a chain costs this frame a level
    // (`crate::depth`); every other node is `execute_plan_node_rest`'s.
    match plan {
        Plan::Filter { input, predicate } => {
            let child = execute_plan_node(input, schema, storage, env)?;
            filter_materialized(child, input, predicate, schema, storage, env)
        }
        Plan::Project {
            input,
            exprs,
            enclosing,
            ..
        } => {
            let child = execute_plan_node(input, schema, storage, env)?;
            project_materialized(child, input, (exprs, enclosing), schema, storage, env)
        }
        Plan::Join {
            left,
            right,
            join_type,
            on,
            using,
        } => {
            let l = execute_plan_node(left, schema, storage, env)?;
            let r = execute_plan_node(right, schema, storage, env)?;
            join::execute_join(l, r, left, right, *join_type, on, using, schema, env)
        }
        Plan::SetOp {
            op,
            all,
            left,
            right,
            column_types,
            ..
        } => {
            let l = execute_plan_node(left, schema, storage, env)?;
            let r = execute_plan_node(right, schema, storage, env)?;
            setop_materialized(l, r, *op, *all, column_types)
        }
        Plan::SubqueryScan {
            subquery,
            column_aliases,
            ..
        } => {
            // Execute the subquery and use its results as a table. The executed columns
            // are the subquery's own, so a FROM-item column-alias list has to be applied
            // **here** — this reader derives the relation's names from the execution and
            // never from the plan-side node the binding gate and `Describe` read.
            let (rows, cols) = execute_plan_node(subquery, schema, storage, env)?;
            Ok((rows, alias_columns_outlined(cols, column_aliases)))
        }
        _ => execute_plan_node_rest(plan, schema, storage, env),
    }
}

/// A `Filter` over its input's rows, executed: each row the predicate holds for.
#[inline(never)]
fn filter_materialized<S: Storage>(
    child: (Vec<Vec<Value>>, Vec<ColumnDesc>),
    input: &Plan,
    predicate: &Expr,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let (rows, columns) = child;
    let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let col_types: Vec<TypeOid> = columns.iter().map(|c| c.type_oid).collect();
    // ...and their enum member orders, so `WHERE m > 'sad'` orders by member
    // position (see `ColumnResolver::column_enum_order`).
    let col_enums = column_enum_orders(&columns, &schema.enums);
    // The scope's relation identity: its single relation's effective name, else
    // the `(relation, column)` pairs of a join, else nothing (see `scope_refs`).
    let (table, refs) = scan::scope_refs(input, &columns, schema);
    let identity = scope_identity(table.as_deref(), refs.as_deref());

    let mut filtered: Vec<Vec<Value>> = Vec::new();
    for row in rows {
        let result = eval_expr_subquery(
            predicate, &col_names, &col_types, &col_enums, identity, &row, schema, storage, env,
        )?;
        if predicate_holds(&result, BoolContext::Where)? {
            filtered.push(row);
        }
    }

    Ok((filtered, columns))
}

/// A `Project` over its input's rows, executed.
#[inline(never)]
fn project_materialized<S: Storage>(
    child: (Vec<Vec<Value>>, Vec<ColumnDesc>),
    input: &Plan,
    (exprs, enclosing): (&[Expr], &EnclosingColumns),
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let (rows, input_columns) = child;
    let col_names: Vec<&str> = input_columns.iter().map(|c| c.name.as_str()).collect();
    // Declared input types, so a projected `||` / `::text` over a
    // `timestamptz` keeps its `+00` offset (see `ColumnResolver::column_type`).
    let col_types: Vec<TypeOid> = input_columns.iter().map(|c| c.type_oid).collect();
    let col_enums = column_enum_orders(&input_columns, &schema.enums);
    let (table, refs) = scan::scope_refs(input, &input_columns, schema);
    let identity = scope_identity(table.as_deref(), refs.as_deref());

    // Handle SELECT * specially
    if exprs.len() == 1
        && let Expr::Column { name, .. } = &exprs[0]
        && name == "*"
    {
        return Ok((rows, input_columns));
    }

    // Build output columns and project rows. The descriptor scope is the
    // (names, table) pair the per-row eval below binds through, so a projected
    // `t.x` is declared with the type of the column its value comes from — except
    // over a join, whose rows bind by `(relation, column)` pairs while this scope
    // reads the synthesized `relation.column` names (`EDGES.md` row 326).
    let enclosing = enclosing.entries_copy();
    let output_columns = expr_column_descs(
        exprs,
        ProjectionScope::new(&input_columns, &col_names, table.as_deref())
            .with_schema(schema)
            .with_enclosing(&enclosing),
    );

    let mut projected: Vec<Vec<Value>> = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut projected_row = Vec::with_capacity(exprs.len());
        for expr in exprs {
            let val = eval_expr_subquery(
                expr, &col_names, &col_types, &col_enums, identity, row, schema, storage, env,
            )?;
            projected_row.push(val);
        }
        projected.push(projected_row);
    }

    Ok((projected, output_columns))
}

/// A set operation over its two sides' rows, executed: each side's values cast to the
/// unified per-position `column_types` (positionally — a side may have repeated column
/// names, which by-name resolution would collapse), then combined. Result column names
/// come from the left side; types are `column_types`.
#[inline(never)]
fn setop_materialized(
    l: (Vec<Vec<Value>>, Vec<ColumnDesc>),
    r: (Vec<Vec<Value>>, Vec<ColumnDesc>),
    op: SetOpKind,
    all: bool,
    column_types: &[TypeOid],
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let ((left_rows, left_cols), (right_rows, right_cols)) = (l, r);
    let left_rows = cast_rows_to_types(left_rows, &left_cols, column_types)?;
    let right_rows = cast_rows_to_types(right_rows, &right_cols, column_types)?;
    let out_cols = left_cols
        .iter()
        .zip(&right_cols)
        .zip(column_types)
        .map(|((l, r), &type_oid)| ColumnDesc {
            enum_type: unified_enum_type(l, r),
            ..ColumnDesc::new(l.name.clone(), type_oid)
        })
        .collect();
    let combined = combine_set_op(op, all, left_rows, right_rows);
    Ok((combined, out_cols))
}

/// [`alias_relation_columns`], out of [`execute_plan_node`]'s frame.
#[inline(never)]
fn alias_columns_outlined(cols: Vec<ColumnDesc>, column_aliases: &[String]) -> Vec<ColumnDesc> {
    alias_relation_columns(cols, column_aliases)
}

/// [`execute_plan_node`]'s arms for every node it does not execute itself.
#[inline(never)]
fn execute_plan_node_rest<S: Storage>(
    plan: &Plan,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let env = env.with_schema(schema).with_toast_status(storage);
    match plan {
        Plan::Scan { table, alias: _ } => scan::execute_scan(table, schema, storage),

        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan {
            table,
            index_oid,
            key_column,
            key_value,
            column_first,
            ..
        } => scan::execute_index_scan(
            table,
            *index_oid,
            key_column,
            key_value,
            *column_first,
            schema,
            storage,
            env,
        ),

        #[cfg(feature = "index_acceleration")]
        Plan::IndexRangeScan {
            table,
            index_oid,
            key_column,
            lower,
            upper,
            ascending,
            limit,
            ..
        } => scan::execute_index_range_scan(
            table,
            *index_oid,
            key_column,
            lower.as_ref(),
            upper.as_ref(),
            *ascending,
            limit.as_ref(),
            schema,
            storage,
            env,
        ),

        #[cfg(feature = "index_acceleration")]
        Plan::GinScan {
            table,
            index_oid,
            key_column,
            probes,
            guards,
            ..
        } => scan::execute_gin_scan(
            table, *index_oid, key_column, probes, guards, schema, storage, env,
        ),

        Plan::VirtualScan { catalog_table, .. } => {
            scan::execute_virtual_scan(*catalog_table, schema)
        }

        // Taken by `execute_plan_node` in its own frame (`crate::depth`): never here.
        Plan::Filter { .. }
        | Plan::Project { .. }
        | Plan::Join { .. }
        | Plan::SetOp { .. }
        | Plan::SubqueryScan { .. } => execute_plan_node(plan, schema, storage, env),

        Plan::Values {
            rows, enclosing, ..
        } => execute_values_internal((rows, enclosing), schema, storage, env),

        Plan::Sort {
            input, order_by, ..
        } => {
            let (rows, columns) = execute_plan_node(input, schema, storage, env)?;
            let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
            // Declared input types, so an ORDER BY key that stringifies a
            // `timestamptz` keeps its `+00` offset (see `ColumnResolver::column_type`).
            let col_types: Vec<TypeOid> = columns.iter().map(|c| c.type_oid).collect();
            let col_enums = column_enum_orders(&columns, &schema.enums);
            // The keys read the input's names, which carry no relation identity, so the
            // scope is told its level's FROM names: a qualifier the level owns binds here,
            // never an enclosing frame of the same name.
            let level = scan::level_scope(input, &columns, schema);
            let key_scope = SimpleColumns::new(&col_names)
                .with_types(&col_types)
                .with_enum_orders(&col_enums)
                .within_level(&level);
            // Each key's enum member order, resolved **once per key** rather than per
            // comparison, through the same `SimpleColumns` the key's *value* binds
            // through — so a key that reads an enum column cannot be ordered by a
            // different column's members than the value came from.
            let key_enum_orders: Vec<Option<&[String]>> = order_by
                .iter()
                .map(|key| order_key_enum_order(&key.expr, &col_enums, &key_scope, schema))
                .collect();

            // Precompute each row's ORDER BY key tuple, **propagating** eval errors
            // (correct-or-loud: a key that errors — e.g. a non-projected column the
            // Sort can't see — fails the query rather than silently collapsing to
            // NULL → insertion order). This also evaluates each key once (O(n)),
            // not once per comparison (O(n log n)).
            let mut keyed: Vec<(Vec<Value>, Vec<Value>)> = Vec::with_capacity(rows.len());
            for row in rows {
                let mut keys = Vec::with_capacity(order_by.len());
                for key in order_by {
                    keys.push(eval_order_key(&key.expr, &key_scope, &row, env)?);
                }
                keyed.push((keys, row));
            }

            // Sort by the precomputed keys, each honoring its direction and NULL
            // placement (explicit `NULLS FIRST/LAST`, else the Postgres default).
            keyed.sort_by(|(a_keys, _), (b_keys, _)| {
                for (i, key) in order_by.iter().enumerate() {
                    let ord = compare_order_values(
                        &a_keys[i],
                        &b_keys[i],
                        key.asc,
                        key.nulls_first_resolved(),
                        key_enum_orders[i],
                    );
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                Ordering::Equal
            });

            let rows = keyed.into_iter().map(|(_, row)| row).collect();
            Ok((rows, columns))
        }

        Plan::Limit {
            input,
            limit,
            offset,
        } => {
            let (rows, columns) = execute_plan_node(input, schema, storage, env)?;
            let skip = resolve_limit_count(offset.as_ref(), env)?.unwrap_or(0);
            let take = resolve_limit_count(limit.as_ref(), env)?.unwrap_or(rows.len());
            let limited: Vec<Vec<Value>> = rows.into_iter().skip(skip).take(take).collect();
            Ok((limited, columns))
        }

        // Trim the junk columns an `ORDER BY` / `DISTINCT ON` key needed off every
        // row — Postgres's junk filter. The junk columns are the trailing ones by
        // construction (the planner appends them), so `keep` is a truncation. It is a
        // planner invariant that `keep <= columns.len()` and that every row is as wide
        // as the descriptors; either shortfall is a planner/executor width
        // disagreement, and `Vec::truncate` is a silent no-op on a short input, so
        // **both** widths are asserted rather than clamped. Only the *column* half was
        // ever written as code — this sentence claimed the row half for four commits,
        // which is the shape of miss a comment carries indefinitely. Nothing is known to
        // have reached it: no plan produces a row narrower than its descriptors, so the
        // row check has no red-on-revert guard and stands as an assertion of the
        // invariant, not as a fix for an observed state.
        Plan::JunkFilter { input, keep } => {
            let (rows, mut columns) = execute_plan_node(input, schema, storage, env)?;
            let width = columns.len();
            if width < *keep {
                return Err(Error::Execution(format!(
                    "junk filter keeps {keep} columns of {width}"
                )));
            }
            columns.truncate(*keep);
            let trimmed = rows
                .into_iter()
                .map(|mut row| {
                    if row.len() < *keep {
                        return Err(Error::Execution(format!(
                            "junk filter keeps {keep} columns of a {}-value row",
                            row.len()
                        )));
                    }
                    row.truncate(*keep);
                    Ok(row)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((trimmed, columns))
        }

        Plan::Distinct { input } => {
            let (rows, columns) = execute_plan_node(input, schema, storage, env)?;
            // Dedup preserving first-occurrence order, with NULL == NULL (Postgres
            // `DISTINCT` treats NULLs as not-distinct) — `UNION`'s dedup exactly.
            Ok((dedup_rows(rows), columns))
        }

        Plan::DistinctOn { on_exprs, input } => {
            let (rows, columns) = execute_plan_node(input, schema, storage, env)?;
            let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
            let col_types: Vec<TypeOid> = columns.iter().map(|c| c.type_oid).collect();
            let col_enums = column_enum_orders(&columns, &schema.enums);
            // The ON keys read the input's names; the scope is told its level's FROM
            // names, as the Sort's is.
            let level = scan::level_scope(input, &columns, schema);
            let key_scope = SimpleColumns::new(&col_names)
                .with_types(&col_types)
                .with_enum_orders(&col_enums)
                .within_level(&level);
            // The planner sorts the input so equal-ON rows are adjacent (the ON exprs
            // lead the sort key), so this keeps the first row of each group with an
            // O(n) adjacent dedup: emit a row only when its ON-key tuple differs from
            // the previous kept row's. NULL == NULL in the key (Postgres `DISTINCT ON`
            // treats NULLs as not-distinct), unlike `=`. Key-eval errors propagate
            // (correct-or-loud: an ON expr the projected rows can't resolve fails the
            // query rather than silently collapsing to NULL).
            let mut result: Vec<Vec<Value>> = Vec::new();
            let mut prev_key: Option<Vec<Value>> = None;
            for row in rows {
                let mut key = Vec::with_capacity(on_exprs.len());
                for e in on_exprs {
                    key.push(eval_expr_full(
                        e,
                        &key_scope,
                        &row,
                        SubqueryExec::NONE,
                        env,
                    )?);
                }
                let is_new_group = prev_key
                    .as_ref()
                    .is_none_or(|prev| !rows_distinct_equal(prev, &key));
                if is_new_group {
                    result.push(row);
                    prev_key = Some(key);
                }
            }
            Ok((result, columns))
        }

        Plan::Aggregate {
            input,
            group_by,
            exprs,
            having,
            enclosing,
            ..
        } => aggregate::execute_aggregate(
            input,
            group_by,
            (exprs, enclosing),
            having.as_ref(),
            schema,
            storage,
            env,
        ),

        // A reference to a data-modifying CTE: return the rows the enclosing
        // `Plan::With` materialized for `name` (cloned — the relation may be
        // referenced more than once). The DML already ran; this never writes.
        Plan::CteScan { name, .. } => {
            let cte = env.cte(name).ok_or_else(|| {
                Error::Execution(format!(
                    "common table expression \"{name}\" not materialized"
                ))
            })?;
            Ok((cte.rows.clone(), cte.columns.clone()))
        }

        Plan::FunctionScan {
            func: FromFunction::Unnest,
            args,
            columns,
            ..
        } => execute_unnest(args, columns, schema, env),
        Plan::FunctionScan {
            func: FromFunction::GenerateSeries(series),
            args,
            columns,
            ..
        } => execute_generate_series(*series, args, columns, env),

        Plan::ProjectUnnest { exprs } => execute_project_unnest(exprs, schema, storage, env),

        Plan::Window {
            input,
            window_exprs,
            enclosing,
            ..
        } => window::execute_window(input, (window_exprs, enclosing), schema, storage, env),

        _ => Err(Error::NotImplemented(format!(
            "plan node not supported in query: {plan:?}"
        ))),
    }
}

/// Execute `unnest(arr1, arr2, ...)` as a table source.
///
/// Each array expression is evaluated with no input row, against the `EvalEnv` frame
/// chain, which resolves a bound parameter and an enclosing query's column in it.
/// Output is one column per array; rows zip the arrays element-wise, NULL-padding
/// shorter ones to the longest length.
///
/// **The output column's name and type come from `function_scan_column_types`, the same
/// derivation `Plan::FunctionScan`'s `output_columns` reads**, so the simple-query
/// `RowDescription` and the extended protocol's `Describe` cannot disagree about them.
/// They did, and the drift had a silent value consequence: this function used to take
/// the type from the *evaluated* `Value::Array`'s `element_type`, which cannot carry the
/// tz/no-tz bit — a `timestamptz` is stored as a UTC `timestamp` and only a column
/// `TypeOid` distinguishes them — so an `unnest(ARRAY[…::timestamptz])` column was
/// declared 1114 here against `Describe`'s 1184, and the column-type-aware text encoder
/// then dropped the UTC `+00` offset from every rendered value.
///
/// The evaluated array's element type stays as the **fallback** for an argument the static
/// derivation cannot type, and that fallback is the only difference between the two
/// readers. A bound parameter the client **declared** at `Parse` is not such an argument:
/// the parser reads it as a `Cast` (`parser::parse_single_with_param_types`), so
/// `SELECT * FROM unnest($1) AS u(v)` declared `int4[]` (1007) or `timestamptz[]` (1185)
/// is typed 23 / 1184 by both readers, as 18.6 types it. An **uncast, undeclared** one is
/// (`Expr::Parameter::infer_type` is `None`): pglet accepts that spelling at `Parse`
/// where 18.6 answers `42725 function unnest(unknown) is not unique`, and the parameter,
/// decoded as text, is refused below. `unnest(NULL)` reaches the fallback too
/// (`Literal::Null::infer_type` is `None`); pglet answers 0 rows where 18.6 is that same
/// `42725`. The cast spelling `unnest($1::int[])` agrees with 18.6 and is the control.
/// Recorded in `EDGES.md`.
// TODO: args are evaluated against an empty row and the enclosing frames, so `unnest`
// reads an enclosing query's column (the correlated form) and cannot read an earlier
// item of its own `FROM` — no lateral form, written `LATERAL` or not (`EDGES.md` row
// 312). `WITH ORDINALITY` is also unsupported.
fn execute_unnest(
    arrays: &[Expr],
    columns: &[String],
    schema: &Schema,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let declared =
        crate::planner::function_scan_column_types(FromFunction::Unnest, arrays, columns, schema);
    let mut element_lists: Vec<Vec<Value>> = Vec::with_capacity(arrays.len());
    let mut output_columns: Vec<ColumnDesc> = Vec::with_capacity(arrays.len());
    let mut max_len = 0;

    // `function_scan_column_types` returns exactly one pair per array, so the zip is total and
    // the naming rule has no second spelling here — it lives in that one derivation.
    for (array_expr, (name, static_type)) in arrays.iter().zip(declared) {
        let (element_type, elements) = match eval_expr_env(array_expr, env)? {
            Value::Array(arr) => (arr.element_type, arr.elements),
            Value::Null => (TypeOid::Text, Vec::new()),
            other => {
                return Err(Error::NotImplemented(format!(
                    "unnest requires array arguments, got {other:?}"
                )));
            }
        };
        max_len = max_len.max(elements.len());
        output_columns.push(ColumnDesc::new(name, static_type.unwrap_or(element_type)));
        element_lists.push(elements);
    }

    let mut rows = Vec::with_capacity(max_len);
    for i in 0..max_len {
        let row: Vec<Value> = element_lists
            .iter()
            .map(|elems| elems.get(i).cloned().unwrap_or(Value::Null))
            .collect();
        rows.push(row);
    }

    Ok((rows, output_columns))
}

/// Execute a FROM-clause `generate_series(start, stop[, step])` over its integer
/// overload — 18.6's `generate_series_step_int4` / `_int8`.
///
/// The arguments are evaluated once, with no input row, as `unnest`'s are; the parser
/// admits none that names a column. The function is strict, so a NULL argument is zero
/// rows ahead of the step's verdict; a zero step is `22023`. A series runs from `start`
/// toward `stop` and never past it, so a value at the type's edge ends it rather than
/// wrapping. Its length is computed from the three values before a row is built, and a
/// series longer than [`MAX_SERIES_ROWS`] is refused `54000`.
// TODO: 18.6 does not run a FROM item under a constant-false qual or a `LIMIT 0`
// (measured), so `… generate_series(1, 2, 0) WHERE false` is zero rows there and `22023`
// here, and a series past the bound `54000` (`EDGES.md` row 320).
fn execute_generate_series(
    series: SeriesType,
    args: &[Expr],
    columns: &[String],
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let name = columns
        .first()
        .cloned()
        .unwrap_or_else(|| FromFunction::GenerateSeries(series).name().to_string());
    let output = vec![ColumnDesc::new(name, series.type_oid())];
    let mut bounds = [0_i64, 0, 1];
    let mut strict_null = false;
    for (slot, arg) in bounds.iter_mut().zip(args) {
        match eval_expr_env(arg, env)? {
            Value::Null => strict_null = true,
            Value::Int2(n) => *slot = i64::from(n),
            Value::Int4(n) => *slot = i64::from(n),
            Value::Int8(n) => *slot = n,
            other => {
                return Err(Error::Execution(format!(
                    "generate_series argument is not an integer: {other:?}"
                )));
            }
        }
    }
    if strict_null {
        return Ok((Vec::new(), output));
    }
    let [start, stop, step] = bounds;
    if step == 0 {
        return Err(Error::InvalidParameterValue {
            message: "step size cannot equal zero".into(),
            hint: None,
        });
    }
    // The count of values from `start` toward `stop`, computed unsigned so no
    // difference overflows at the type's edges.
    let len = if (step > 0 && start <= stop) || (step < 0 && start >= stop) {
        (stop.abs_diff(start) / step.unsigned_abs()).saturating_add(1)
    } else {
        0
    };
    if len > MAX_SERIES_ROWS {
        return Err(Error::ProgramLimitExceeded(format!(
            "generate_series would return {len} rows, more than the {MAX_SERIES_ROWS} \
             pglet materializes"
        )));
    }
    let mut rows = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    let mut n = start;
    for _ in 0..len {
        // Every value lies between `start` and `stop`, each the overload's type; the
        // step past the last one may wrap, and is never read.
        rows.push(vec![match series {
            SeriesType::Int4 => Value::Int4(i32::try_from(n).map_err(|_| {
                Error::Execution(format!("generate_series value {n} out of range"))
            })?),
            SeriesType::Int8 => Value::Int8(n),
        }]);
        n = n.wrapping_add(step);
    }
    Ok((rows, output))
}

/// One projection slot in `execute_project_unnest`: a pre-evaluated scalar
/// (broadcast across every emitted row), a scalar holding a volatile call (its first
/// evaluation, which the first emitted row reads, and the expression each later row
/// evaluates again), or the unnested array's elements (the expansion axis that drives
/// the row count).
enum ProjectUnnestSlot<'e> {
    Scalar(Value),
    Volatile(&'e Expr, Value),
    Unnest(Vec<Value>),
}

/// Execute a no-FROM projection containing one set-returning `unnest(<array>)`.
///
/// Row-expands: the unnested array drives the row count. Every slot is evaluated once, in
/// target-list order, before any row is emitted — whether or not the array yields one —
/// as 18.6's `ExecProjectSRF` evaluates every target-list item at its first call, so a
/// sibling's error is raised over an empty array and the first failing item in the list
/// is the one reported. A non-volatile scalar is then broadcast across the emitted rows;
/// one holding a volatile call is evaluated again for each row after the first, as
/// `ExecProjectSRF` evaluates it at each call (`SELECT unnest(ARRAY[1, 2, 3]), random()`
/// draws three values there). An empty or NULL array
/// yields zero rows (matching Postgres). Each expression is evaluated against an
/// empty input row (no column context) but with subquery support, so a `$N` must
/// already be bound and a nested scalar subquery still works.
///
/// The single `Expr::Unnest` is located by peeling any wrapping `Expr::Alias`; its
/// inner array expr is evaluated to a `Value::Array` and split into elements, while
/// the other projection slots stay scalar. Column descriptors come from
/// `expr_column_info`, consistent with `Plan::ProjectUnnest::output_columns`.
fn execute_project_unnest<S: Storage>(
    exprs: &[Expr],
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    // Output column descriptors (one per projection slot, no input columns).
    let columns = expr_column_descs(exprs, ProjectionScope::empty().with_schema(schema));

    // Evaluate each slot: the one containing `unnest` becomes the element list (the
    // expansion axis); every other slot is a single scalar value, broadcast.
    let mut slots: Vec<ProjectUnnestSlot> = Vec::with_capacity(exprs.len());
    let mut row_count: Option<usize> = None;
    for expr in exprs {
        if let Some(array_expr) = unnest_inner(expr) {
            let elements = match eval_expr_subquery(
                array_expr,
                &[],
                &[],
                &[],
                ScopeIdentity::Anonymous,
                &[],
                schema,
                storage,
                env,
            )? {
                Value::Array(arr) => arr.elements,
                // NULL array → zero rows (PG semantics), mirroring `execute_unnest`.
                Value::Null => Vec::new(),
                other => {
                    return Err(Error::NotImplemented(format!(
                        "unnest requires an array argument, got {other:?}"
                    )));
                }
            };
            row_count = Some(elements.len());
            slots.push(ProjectUnnestSlot::Unnest(elements));
        } else {
            let val = eval_expr_subquery(
                expr,
                &[],
                &[],
                &[],
                ScopeIdentity::Anonymous,
                &[],
                schema,
                storage,
                env,
            )?;
            slots.push(if expr.contains_volatile_call() {
                ProjectUnnestSlot::Volatile(expr, val)
            } else {
                ProjectUnnestSlot::Scalar(val)
            });
        }
    }

    // `plan_select_no_from` guarantees exactly one unnest slot, so `row_count`
    // is always `Some`; default to 0 defensively.
    let n = row_count.unwrap_or(0);
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(n);
    for i in 0..n {
        let row = slots
            .iter()
            .map(|slot| match slot {
                ProjectUnnestSlot::Scalar(v) | ProjectUnnestSlot::Volatile(_, v) if i == 0 => {
                    Ok(v.clone())
                }
                ProjectUnnestSlot::Scalar(v) => Ok(v.clone()),
                ProjectUnnestSlot::Volatile(expr, _) => eval_expr_subquery(
                    expr,
                    &[],
                    &[],
                    &[],
                    ScopeIdentity::Anonymous,
                    &[],
                    schema,
                    storage,
                    env,
                ),
                ProjectUnnestSlot::Unnest(elems) => {
                    Ok(elems.get(i).cloned().unwrap_or(Value::Null))
                }
            })
            .collect::<Result<Vec<Value>>>()?;
        rows.push(row);
    }

    Ok((rows, columns))
}

/// If `expr` is an `Expr::Unnest` (possibly wrapped in an `Expr::Alias`), return
/// its inner array expression; otherwise `None`.
fn unnest_inner(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Unnest(inner) => Some(inner),
        Expr::Alias { expr, .. } => unnest_inner(expr),
        _ => None,
    }
}

/// Internal VALUES execution returning raw rows and columns.
///
/// `schema`/`storage` are threaded through so projection expressions evaluate via
/// the subquery-capable path (`eval_expr_subquery`), not the callback-less
/// `eval_expr`. This is what makes a no-FROM `SELECT EXISTS(SELECT ... FROM t ...)`
/// (the `query_has_fact` spine shape, planned to `Plan::Values`) work: the inner
/// sub-plan is run via the `exists_fn` callback. Each VALUES row has no input row
/// (the `&[]`/`&[]` resolver context); a column reference here would still error,
/// matching the prior behavior — only subqueries gain context.
fn execute_values_internal<S: Storage>(
    (rows, enclosing): (&[Vec<Expr>], &EnclosingColumns),
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    if rows.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // Infer column names and types from the first row (handles aliases, e.g.
    // `SELECT 1 AS num`).
    let enclosing = enclosing.entries_copy();
    let columns = expr_column_descs(
        &rows[0],
        ProjectionScope::empty()
            .with_schema(schema)
            .with_enclosing(&enclosing),
    );

    let result_rows: Vec<Vec<Value>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|expr| {
                    eval_expr_subquery(
                        expr,
                        &[],
                        &[],
                        &[],
                        ScopeIdentity::Anonymous,
                        &[],
                        schema,
                        storage,
                        env,
                    )
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((result_rows, columns))
}

/// Cast every row of a set-op side to the unified per-position `targets` type,
/// **positionally** (by column index, never by name — a side can carry repeated
/// column names, which by-name resolution would collapse onto the first). Only the
/// positions whose source type differs from the target are coerced; a NULL casts to
/// NULL under any target. Reuses `coerce_value`, the same correct-or-loud value cast
/// `Expr::Cast` uses, so an impossible coercion fails loud.
fn cast_rows_to_types(
    rows: Vec<Vec<Value>>,
    source_cols: &[ColumnDesc],
    targets: &[TypeOid],
) -> Result<Vec<Vec<Value>>> {
    // A per-position "needs a cast" mask; if none do, the rows pass through untouched.
    let needs: Vec<bool> = source_cols
        .iter()
        .zip(targets)
        .map(|(c, &t)| c.type_oid != t)
        .collect();
    if needs.iter().all(|n| !n) {
        return Ok(rows);
    }
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .enumerate()
                .map(|(i, v)| {
                    if needs.get(i).copied().unwrap_or(false) {
                        super::coerce_value(v, targets[i])
                    } else {
                        Ok(v)
                    }
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect()
}

/// Combine the materialized rows of a set operation's two sides.
///
/// `UNION` concatenates (deduping unless `all`); `INTERSECT`/`EXCEPT` group both
/// sides by whole-row equality and emit per the Postgres `nodeSetOp` arithmetic.
/// Two NULLs count as equal throughout (`rows_distinct_equal`), unlike `=`, matching
/// Postgres set-op semantics. First-occurrence order from the **left** side is
/// preserved for determinism.
fn combine_set_op(
    op: SetOpKind,
    all: bool,
    left_rows: Vec<Vec<Value>>,
    right_rows: Vec<Vec<Value>>,
) -> Vec<Vec<Value>> {
    match op {
        SetOpKind::Union => {
            let mut out = left_rows;
            out.extend(right_rows);
            if all { out } else { dedup_rows(out) }
        }
        SetOpKind::Intersect | SetOpKind::Except => {
            set_op_multiset(op, all, &left_rows, &right_rows)
        }
    }
}

/// Dedup rows preserving first-occurrence order, counting two NULLs as equal
/// (`DISTINCT`, and `UNION` without `ALL`): a row is kept, where it stood, unless it
/// equals under [`rows_distinct_equal`] a row already kept. Where that relation is not
/// transitive — float8 against int8 near 2^53, say — the kept rows are not the first of
/// each set of mutually equal values.
///
/// Keyed by [`dedup::DistinctKeys`] while that is certain to be the relation, and
/// pairwise against the rows kept so far from the first row where it is not.
fn dedup_rows(rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    let mut keys = dedup::DistinctKeys::new(rows.first().map_or(0, Vec::len));
    let mut out: Vec<Vec<Value>> = Vec::new();
    for row in rows {
        let fresh = keys
            .first_sighting(&row)
            .unwrap_or_else(|| !out.iter().any(|seen| rows_distinct_equal(seen, &row)));
        if fresh {
            out.push(row);
        }
    }
    out
}

/// `INTERSECT`/`EXCEPT` (with or without `ALL`) via group-and-count over whole-row
/// equality. Iterates the left side's distinct row-groups in first-occurrence order,
/// counts each group's multiplicity on both sides, and emits per the `nodeSetOp`
/// rule: `INTERSECT` → 1 iff present on both; `INTERSECT ALL` → `min(left, right)`;
/// `EXCEPT` → 1 iff present on the left but not the right; `EXCEPT ALL` →
/// `max(0, left − right)`. A right-only row emits nothing under either op, so only
/// the left's groups are kept.
///
/// A row finds its group through [`dedup::DistinctKeys`] — one detector over both
/// sides, so a right row of another class than the left's leaves the keyed path too —
/// and, from the first row where that is not certain to be the relation, by
/// [`rows_distinct_equal`] against every group: a left row joins the first group it
/// equals, and a right row counts toward every group it equals — each group's count of
/// the right rows equal to it, which a relation that is not transitive can give one
/// right row in two groups.
fn set_op_multiset(
    op: SetOpKind,
    all: bool,
    left_rows: &[Vec<Value>],
    right_rows: &[Vec<Value>],
) -> Vec<Vec<Value>> {
    use std::collections::hash_map::Entry;
    let mut keys = dedup::DistinctKeys::new(left_rows.first().map_or(0, Vec::len));
    // Distinct left row-groups: representative, left count, right count.
    let mut groups: Vec<(&Vec<Value>, usize, usize)> = Vec::new();
    for row in left_rows {
        let found = match keys.entry(row) {
            Some(Entry::Occupied(held)) => dedup::held_group(held.get()),
            Some(Entry::Vacant(slot)) => {
                slot.insert(dedup::group_slot(groups.len()));
                None
            }
            None => groups
                .iter()
                .position(|(rep, _, _)| rows_distinct_equal(rep, row)),
        };
        match found.and_then(|i| groups.get_mut(i)) {
            Some(group) => group.1 += 1,
            None => groups.push((row, 1, 0)),
        }
    }
    for row in right_rows {
        match keys.entry(row) {
            Some(Entry::Occupied(held)) => {
                if let Some(group) = dedup::held_group(held.get()).and_then(|i| groups.get_mut(i)) {
                    group.2 += 1;
                }
            }
            Some(Entry::Vacant(_)) => {}
            None => {
                for group in &mut groups {
                    if rows_distinct_equal(row, group.0) {
                        group.2 += 1;
                    }
                }
            }
        }
    }

    let mut out: Vec<Vec<Value>> = Vec::new();
    for (rep, left_count, right_count) in groups {
        let emit = match (op, all) {
            (SetOpKind::Intersect, false) => usize::from(right_count > 0),
            (SetOpKind::Intersect, true) => left_count.min(right_count),
            (SetOpKind::Except, false) => usize::from(right_count == 0),
            (SetOpKind::Except, true) => left_count.saturating_sub(right_count),
            // UNION is handled by `combine_set_op` without counting.
            (SetOpKind::Union, _) => 0,
        };
        for _ in 0..emit {
            out.push(rep.clone());
        }
    }
    out
}

/// Whether two rows are equal for `DISTINCT`: same length and every column
/// pairwise equal under [`values_distinct_equal`] (two NULLs, and two float `NaN`s,
/// counted as equal — Postgres `DISTINCT` treats both as not-distinct, unlike `=`).
///
/// Shared by plain `DISTINCT`, `DISTINCT ON` (on the ON-key tuple), and set-op
/// duplicate elimination.
fn rows_distinct_equal(a: &[Value], b: &[Value]) -> bool {
    #[cfg(test)]
    dedup::count_pairwise_ask();
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_distinct_equal(x, y))
}

/// Whether two values are equal under `DISTINCT`-family semantics (not the `=`
/// operator): two NULLs are equal, and two float4/float8 `NaN`s are equal —
/// Postgres treats `NaN` as not-distinct-from `NaN` for `DISTINCT`/grouping even
/// though `NaN = NaN` is false. (`NUMERIC` `NaN` already equals itself via
/// [`values_equal`], so only the machine floats need the override.)
fn values_distinct_equal(x: &Value, y: &Value) -> bool {
    match (x.is_null(), y.is_null()) {
        (true, true) => return true,
        (true, false) | (false, true) => return false,
        (false, false) => {}
    }
    match (x, y) {
        (Value::Float4(a), Value::Float4(b)) if a.is_nan() && b.is_nan() => true,
        (Value::Float8(a), Value::Float8(b)) if a.is_nan() && b.is_nan() => true,
        _ => values_equal(x, y) == Some(true),
    }
}

#[cfg(test)]
mod order_compare_tests {
    use super::compare_order_values;
    use crate::planner::OrderKey;
    use crate::types::Value;
    use std::cmp::Ordering;

    fn col(name: &str) -> crate::planner::Expr {
        crate::planner::Expr::Column {
            table: None,
            name: name.to_string(),
        }
    }

    /// NULL placement is honored independently of direction: a NULL sorts *first*
    /// under `nulls_first = true` and *last* under `false`, for **both** ASC and
    /// DESC (Postgres applies NULLS FIRST/LAST after ASC/DESC — the placement is not
    /// reversed by direction).
    #[test]
    fn null_placement_is_independent_of_direction() {
        let one = Value::Int4(1);
        let null = Value::Null;
        for &ascending in &[true, false] {
            // NULLS FIRST → NULL sorts before a non-NULL (Less), regardless of dir.
            assert_eq!(
                compare_order_values(&null, &one, ascending, true, None),
                Ordering::Less,
                "NULLS FIRST, ascending={ascending}: NULL before value"
            );
            assert_eq!(
                compare_order_values(&one, &null, ascending, true, None),
                Ordering::Greater
            );
            // NULLS LAST → NULL sorts after a non-NULL (Greater), regardless of dir.
            assert_eq!(
                compare_order_values(&null, &one, ascending, false, None),
                Ordering::Greater,
                "NULLS LAST, ascending={ascending}: NULL after value"
            );
            assert_eq!(
                compare_order_values(&one, &null, ascending, false, None),
                Ordering::Less
            );
        }
        // Two NULLs are always equal.
        assert_eq!(
            compare_order_values(&null, &null, true, true, None),
            Ordering::Equal
        );
    }

    /// A non-NULL-vs-non-NULL comparison *is* flipped by direction, but is unaffected
    /// by the null placement.
    #[test]
    fn non_null_comparison_flips_with_direction_only() {
        let (one, two) = (Value::Int4(1), Value::Int4(2));
        for &nulls_first in &[true, false] {
            assert_eq!(
                compare_order_values(&one, &two, true, nulls_first, None),
                Ordering::Less,
                "ASC: 1 < 2"
            );
            assert_eq!(
                compare_order_values(&one, &two, false, nulls_first, None),
                Ordering::Greater,
                "DESC: 1 sorts after 2"
            );
        }
    }

    /// The default resolution (via [`OrderKey::nulls_first_resolved`]) reproduces
    /// Postgres's direction default: `ASC` → NULLS LAST, `DESC` → NULLS FIRST — the
    /// behavior the engine had before explicit `NULLS FIRST/LAST` was honored.
    #[test]
    fn default_resolution_matches_pg_direction_default() {
        let asc = OrderKey::new(col("v"), true);
        let desc = OrderKey::new(col("v"), false);
        // ASC default → NULLS LAST (nulls_first == false).
        assert!(!asc.nulls_first_resolved());
        // DESC default → NULLS FIRST (nulls_first == true).
        assert!(desc.nulls_first_resolved());

        let (one, null) = (Value::Int4(1), Value::Null);
        // ASC + default: NULL last.
        assert_eq!(
            compare_order_values(&null, &one, asc.asc, asc.nulls_first_resolved(), None),
            Ordering::Greater
        );
        // DESC + default: NULL first.
        assert_eq!(
            compare_order_values(&null, &one, desc.asc, desc.nulls_first_resolved(), None),
            Ordering::Less
        );
    }

    /// An explicit `NULLS` clause overrides the direction default.
    #[test]
    fn explicit_placement_overrides_default() {
        // ASC NULLS FIRST — the non-default combination.
        let asc_nf = OrderKey {
            expr: col("v"),
            asc: true,
            nulls_first: Some(true),
        };
        assert!(asc_nf.nulls_first_resolved());
        // DESC NULLS LAST — the other non-default combination.
        let desc_nl = OrderKey {
            expr: col("v"),
            asc: false,
            nulls_first: Some(false),
        };
        assert!(!desc_nl.nulls_first_resolved());
    }
}
