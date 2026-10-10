//! Nested-loop JOIN execution and its table-name resolution helper.

use crate::planner::{
    BinaryOp, BoolContext, Expr, JoinType, Plan, ResolvedType, UsingMatch, comparison_legality,
    using_column_match,
};
use crate::types::Value;
use crate::{CatalogError, Error, Result};

use super::super::expr::{EvalEnv, JoinColumnMap, eval_binary_op, eval_expr_join, predicate_holds};
use super::super::types::ColumnDesc;
use super::scan::{from_item_name, plan_scope};
use crate::catalog::Schema;

/// A resolved `JOIN … USING (…)`: how to match a row pair and how to shape the
/// merged output. Built once per join execution by [`resolve_using`].
struct UsingPlan {
    /// One `(left_index, right_index)` per USING column, as offsets into the
    /// combined `left ++ right` row — the columns whose pairwise equality is the
    /// join match, and whose `COALESCE` is the merged output column.
    pairs: Vec<(usize, usize)>,
    /// The merged column descriptors (one per USING column, in USING order),
    /// prepended to the raw per-side columns in the join's output.
    merged: Vec<ColumnDesc>,
}

/// Resolve a `USING` column list against the two sides' materialized columns.
///
/// Each name must denote exactly one column of each side (Postgres's rule) — matched
/// via [`using_column_match`], so a base column (`num`), a nested join's merged
/// column (`num`), and a derived side's dotted column (`x.num`) all resolve, while an
/// ambiguous / missing name fails loud (`42702` / `42703`). Returns `None` for an
/// ordinary (non-`USING`) join. Kept in sync with the plan-time validation
/// (`Parser::resolve_join_constraint`) — both consult the same matcher.
///
/// Each resolved pair is **operand-type gated on the two sides' declared types**,
/// once, here — not per row pair. Postgres resolves `USING (k)` to `left.k = right.k`
/// at parse time, so a `text` `k` against an `int` `k` is `42883` whatever the data
/// is: with no rows, with all-NULL rows, and with a multi-column `USING (k1, k2)`
/// whose *first* pair happens not to match. Gating on the runtime values instead made
/// all three silently return zero rows.
fn resolve_using(
    using: &[String],
    left_columns: &[ColumnDesc],
    right_columns: &[ColumnDesc],
    right_base: usize,
) -> Result<Option<UsingPlan>> {
    if using.is_empty() {
        return Ok(None);
    }
    let resolve = |cols: &[ColumnDesc], c: &str| -> Result<usize> {
        match using_column_match(cols, c) {
            UsingMatch::Found(i) => Ok(i),
            UsingMatch::Ambiguous => Err(Error::AmbiguousColumn {
                column: c.to_string(),
            }),
            UsingMatch::Missing => Err(CatalogError::ColumnNotFoundInExpr {
                column: c.to_string(),
            }
            .into()),
        }
    };
    let mut pairs = Vec::with_capacity(using.len());
    let mut merged = Vec::with_capacity(using.len());
    for c in using {
        let li = resolve(left_columns, c)?;
        let ri = resolve(right_columns, c)?;
        // `USING (c)` is `left.c = right.c`, and these descriptors carry the enum
        // identity the operator rulebook needs: a `mood` merged against a `text` — or
        // against a different enum — is 18.6's `42883` and not a merged column.
        comparison_legality(
            Some(ResolvedType::of_column(&left_columns[li])),
            Some(ResolvedType::of_column(&right_columns[ri])),
            "=",
        )?;
        // A merged USING column is a coalesce of both sides, built here with no
        // single-column wire metadata — where 18.6 keeps an agreeing modifier and, for an
        // inner, left or right join, a base column's provenance (`EDGES.md` row 155) —
        // but the **enum** is not wire metadata (see `unified_enum_type`):
        // it is what makes `ORDER BY m` and `WHERE m > 'sad'` over the merged name
        // order by member position, as they do over either side's own `m`.
        merged.push(ColumnDesc {
            enum_type: crate::executor::unified_enum_type(&left_columns[li], &right_columns[ri]),
            ..ColumnDesc::new(c.clone(), left_columns[li].type_oid)
        });
        pairs.push((li, right_base + ri));
    }
    Ok(Some(UsingPlan { pairs, merged }))
}

/// `COALESCE(a, b)` — the first non-NULL of the two (a USING column's merged value).
fn coalesce_pair(a: &Value, b: &Value) -> Value {
    if a.is_null() { b.clone() } else { a.clone() }
}

/// Execute a `Plan::Join` — a nested-loop join over the left/right inputs.
///
/// **Ordinary / CROSS / comma joins** (`using` empty) match on the `on` predicate,
/// **correct-or-loud**: the pair's value goes through `predicate_holds` at
/// [`BoolContext::JoinOn`], so `Value::Bool(true)` joins the pair while `false` and
/// NULL do not, a `Value::Text` is *converted* through the boolean input function
/// (`ON 't'` joins every pair, `ON 'abc'` is `22P02`), and any other type is
/// `42804 argument of JOIN/ON must be type boolean` rather than a silent non-match.
/// An *evaluation error* propagates instead of silently dropping the pair (a
/// `CROSS`/comma join's `on` is the constant `true`, so every pair joins).
///
/// **`JOIN … USING (…)`** (`using` non-empty) matches on the shared columns' pairwise
/// equality (NULL never matches — same as `ON left.c = right.c`) and emits the
/// Postgres-merged shape: one `COALESCE(left.c, right.c)` column per USING name,
/// **prepended** (so an unqualified `c`, a bare `*`, and a chained USING see the
/// single merged column) ahead of the raw per-side columns, which stay
/// dotted-and-qualifiable (`left.c` / `right.c`).
///
/// Takes both inputs already executed: `execute_plan_node` runs them in its own frame,
/// so a `JOIN` chain costs one frame of that function per level and this one only
/// once (`crate::depth`).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_join(
    l: (Vec<Vec<Value>>, Vec<ColumnDesc>),
    r: (Vec<Vec<Value>>, Vec<ColumnDesc>),
    left: &Plan,
    right: &Plan,
    join_type: JoinType,
    on: &Expr,
    using: &[String],
    schema: &Schema,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let ((left_rows, left_columns), (right_rows, right_columns)) = (l, r);

    // Build the column map + the combined output descriptors. Each side's columns
    // are registered under their `(table, column)` and named `table.column` so a
    // later Project/ON can resolve a qualified ref, distinguishing e.g. `users.id`
    // from `orders.id`. A nested-join input arrives already qualified, so it is
    // re-registered under its *original* aliases (see `register_input_columns`).
    let mut col_map = JoinColumnMap::new();
    let mut raw_columns = register_input_columns(&mut col_map, left, &left_columns, 0, schema);
    let right_output = register_input_columns(
        &mut col_map,
        right,
        &right_columns,
        left_columns.len(),
        schema,
    );
    raw_columns.extend(right_output);

    // A USING join resolves its shared columns to combined-row indices up front and
    // matches on their equality; an ordinary join evaluates `on`.
    let using_plan = resolve_using(using, &left_columns, &right_columns, left_columns.len())?;
    let right_len = right_columns.len();
    let left_len = left_columns.len();

    // The row-pair match predicate (an eval error propagates either way).
    let is_match = |combined: &[Value]| -> Result<bool> {
        match &using_plan {
            // A `USING` column pair is Postgres's `left.c = right.c`, so it goes
            // through `eval_binary_op` rather than a bare `values_equal` — keeping one
            // equality implementation, and catching a value-level mismatch the
            // declared-type gate in `resolve_using` can't see. (NULL still means "no
            // match", as `values_equal` did: an untyped NULL passes the gate and
            // `= NULL` is NULL, not `Bool(true)`.)
            Some(u) => {
                for &(li, ri) in &u.pairs {
                    if !matches!(
                        eval_binary_op(BinaryOp::Eq, &combined[li], &combined[ri])?,
                        Value::Bool(true)
                    ) {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            None => predicate_holds(
                &eval_expr_join(on, &col_map, combined, env)?,
                BoolContext::JoinOn,
            ),
        }
    };

    // Nested loop join
    let mut result_rows = Vec::new();

    match join_type {
        JoinType::Inner => {
            for left_row in &left_rows {
                for right_row in &right_rows {
                    let mut combined = left_row.clone();
                    combined.extend(right_row.clone());
                    if is_match(&combined)? {
                        result_rows.push(combined);
                    }
                }
            }
        }
        JoinType::Left => {
            for left_row in &left_rows {
                let mut found_match = false;
                for right_row in &right_rows {
                    let mut combined = left_row.clone();
                    combined.extend(right_row.clone());
                    if is_match(&combined)? {
                        result_rows.push(combined);
                        found_match = true;
                    }
                }
                // If no match, add left row with NULLs for right columns
                if !found_match {
                    let mut combined = left_row.clone();
                    combined.extend(vec![Value::Null; right_len]);
                    result_rows.push(combined);
                }
            }
        }
        JoinType::Right => {
            for right_row in &right_rows {
                let mut found_match = false;
                for left_row in &left_rows {
                    let mut combined = left_row.clone();
                    combined.extend(right_row.clone());
                    if is_match(&combined)? {
                        result_rows.push(combined);
                        found_match = true;
                    }
                }
                // If no match, add NULLs for left columns with right row
                if !found_match {
                    let mut combined = vec![Value::Null; left_len];
                    combined.extend(right_row.clone());
                    result_rows.push(combined);
                }
            }
        }
        JoinType::Full => {
            // Track which right rows have been matched
            let mut right_matched = vec![false; right_rows.len()];

            // First pass: like LEFT JOIN, track matched right rows
            for left_row in &left_rows {
                let mut found_match = false;
                for (right_idx, right_row) in right_rows.iter().enumerate() {
                    let mut combined = left_row.clone();
                    combined.extend(right_row.clone());
                    if is_match(&combined)? {
                        result_rows.push(combined);
                        found_match = true;
                        right_matched[right_idx] = true;
                    }
                }
                // If no match, add left row with NULLs for right columns
                if !found_match {
                    let mut combined = left_row.clone();
                    combined.extend(vec![Value::Null; right_len]);
                    result_rows.push(combined);
                }
            }

            // Second pass: add unmatched right rows with NULL left columns
            for (right_idx, right_row) in right_rows.iter().enumerate() {
                if !right_matched[right_idx] {
                    let mut combined = vec![Value::Null; left_len];
                    combined.extend(right_row.clone());
                    result_rows.push(combined);
                }
            }
        }
    }

    // An ordinary join emits the raw concatenation; a USING join prepends the
    // per-column `COALESCE` merges (in USING order) ahead of the raw columns.
    match using_plan {
        None => Ok((result_rows, raw_columns)),
        Some(u) => {
            let mut columns = u.merged;
            columns.extend(raw_columns);
            let rows = result_rows
                .into_iter()
                .map(|combined| {
                    let mut row = Vec::with_capacity(u.pairs.len() + combined.len());
                    for &(li, ri) in &u.pairs {
                        row.push(coalesce_pair(&combined[li], &combined[ri]));
                    }
                    row.extend(combined);
                    row
                })
                .collect();
            Ok((rows, columns))
        }
    }
}

/// Register one JOIN input's columns into `col_map` and produce its output column
/// descriptors.
///
/// A column whose name is already `table.column` — the output of a **nested** JOIN
/// (a left-deep `Join(Join(a, b), c)`) — is registered under that *original*
/// `(table, column)` and kept verbatim, so a qualified ref to an inner table
/// (`a.account_id` two joins deep) still resolves. Re-prefixing it under the
/// `"joined"` placeholder (the old behavior) produced `joined.a.account_id`, which no
/// qualified ref could match — the multi-table-alias resolution bug. A bare column
/// name (a base scan) is registered under `input`'s effective table/alias.
///
/// **The pair comes from the plan, not from the column's name.** Splitting `c.name` at
/// its first `.` cannot tell a nested join's synthesized spelling from a name a user
/// quoted a dot into, and it was wrong both ways: a base scan aliased `s` carrying a
/// column quoted into existence as `"zz.v"` filed it under the pair `("zz", "v")`, so a
/// relation the query never named became addressable, while the pair the scan's own
/// alias spells — `("s", "zz.v")` — was never registered at all and an `ON s."zz.v" = …`
/// was `column "s.zz.v" does not exist`. [`plan_scope`] derives the pairs from the plan
/// instead, which no spelling can forge, and it is right about a **nested** join too —
/// where the old fallback (`get_table_name_from_plan` answering `"joined"`) is what forced
/// the split in the first place.
///
/// The split survives only as the fallback for a plan shape `plan_scope` declines, where
/// it is what this call site had. The descriptors handed up keep their synthesized
/// `"{relation}.{column}"` names: they are what the planner's `*` expansion and
/// `SimpleColumns`'s qualified tier read, and the pair map is what resolves.
fn register_input_columns(
    col_map: &mut JoinColumnMap,
    input: &Plan,
    columns: &[ColumnDesc],
    base_idx: usize,
    schema: &Schema,
) -> Vec<ColumnDesc> {
    let input_table = get_table_name_from_plan(input);
    let refs = plan_scope(input, schema).filter(|refs| refs.len() == columns.len());
    columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            // A `USING` merge belongs to the nested join itself, so no qualifier names
            // it; it keeps the placeholder the fallback's `None` arm gives it.
            let (table, name) = refs.as_ref().and_then(|refs| refs.get(i)).map_or_else(
                || {
                    c.name
                        .split_once('.')
                        .map_or_else(|| (input_table.as_str(), c.name.as_str()), |(t, n)| (t, n))
                },
                |r| {
                    (
                        r.relation.as_deref().unwrap_or(input_table.as_str()),
                        r.name.as_str(),
                    )
                },
            );
            col_map.add_column(
                table,
                name,
                base_idx + i,
                c.type_oid,
                super::super::helpers::enum_order_of(&schema.enums, c.enum_type.as_deref())
                    .map(<[String]>::to_vec),
            );
            // A join side column is a passthrough of the source column — keep its
            // wire metadata (typmod + base-table provenance), only qualifying the
            // name — so a `SELECT a.x`-style projection propagates it.
            ColumnDesc {
                name: format!("{table}.{name}"),
                ..c.clone()
            }
        })
        .collect()
}

/// Get the table name from a plan node (for JOIN column mapping).
///
/// Reused by `UPDATE … FROM` to key the combined column map on the source's
/// effective name (its FROM-clause alias if present, else the real table).
pub fn get_table_name_from_plan(plan: &Plan) -> String {
    match plan {
        Plan::Filter { input, .. } | Plan::Project { input, .. } => get_table_name_from_plan(input),
        Plan::Join { .. } => "joined".to_string(), // Nested join - use placeholder
        // A FROM item's effective name — the alias where one is written, else the
        // relation's own name, and a `FunctionScan`'s `chooseScalarFunctionAlias` name —
        // is the scan module's one leaf rule, so this key agrees with the planner's
        // `plan_relations`: a self-join's `a.id`/`b.id` resolve to the right side, and
        // `UPDATE t SET … FROM unnest(ARRAY['x','y']) AS u WHERE u.u = 'x'` binds `u`.
        _ => from_item_name(plan).map_or_else(|| "unknown".to_string(), str::to_owned),
    }
}
