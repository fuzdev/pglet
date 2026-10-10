//! Scan execution: `Scan`/`IndexScan`/`IndexRangeScan`/`VirtualScan` arms and the
//! single-table-name / inner-FROM column/table helpers used for alias-aware
//! resolution and correlated-subquery scoping.

use crate::Result;
use crate::catalog::{CatalogTable, InformationSchema, PgCatalog, Schema};
#[cfg(feature = "index_acceleration")]
use crate::planner::BoolContext;
use crate::planner::Plan;
#[cfg(feature = "index_acceleration")]
use crate::planner::{
    BinaryOp, Expr, IndexBound, ResolvedType, comparison_legality, comparison_symbol,
    mirrored_comparison_op,
};
#[cfg(feature = "index_acceleration")]
use crate::types::TypeOid;
use crate::types::Value;

use super::super::Storage;
use super::super::expr::ColumnRef;
#[cfg(feature = "index_acceleration")]
use super::super::expr::{
    EvalEnv, eval_binary_op_ordered, eval_expr_env, predicate_holds, values_compare,
    values_compare_ordered,
};
use super::super::helpers::{get_table_or_err, table_column_descs};
use super::super::types::ColumnDesc;
#[cfg(feature = "index_acceleration")]
use super::super::types::RowId;
#[cfg(feature = "index_acceleration")]
use super::resolve_limit_count;

/// Gate an index probe/bound value against its key column's type, then coerce it to
/// that type so the value's index-key bytes sort against the stored entries —
/// **`Ok(None)` when no exact key value represents it**, which tells the caller to
/// fall back to a filtered scan.
///
/// **This is what keeps an *inexact probe* from making the two build profiles
/// disagree.** An accelerated plan replaces `col OP <value>` with an index probe, so
/// it never reaches `eval_binary_op` and the comparison it stands for is decided by
/// *key bytes* instead. Two separate things can go wrong, and both used to:
///
/// 1. **No such operator.** `= true` against an `int4` column coerced to `= 1` and
///    returned rows, while the stripped (`--no-default-features`) build ran
///    `Filter(Scan)` and raised `42883`. The [`comparison_legality`] call fixes that.
/// 2. **An operator exists, but the coercion isn't faithful.** Past the gate the
///    coercion used to be best-effort, keeping the *original* value when it failed
///    and keeping a *lossy* result when it succeeded. `int_col > 1.5` coerced the
///    numeric `1.5` to int4 `2` and answered `> 2` (dropping the row `2`);
///    `time_col = INTERVAL '1 hour'` had no `Interval → Time` arm at all, so the
///    probe keyed on raw interval bytes and matched nothing. Both profiles disagreed,
///    and the accelerated one was the wrong answer.
///
/// The substitution of a coerced `c` for the written `v` is sound exactly when the
/// two decide every stored key the same way:
///
/// ```text
/// ∀ k of the column's type:  keycmp(k, c)  ==  crosscmp(k, v)
/// ```
///
/// where `keycmp` is the index's type-native key order and `crosscmp` is
/// [`values_compare`], the cross-type comparator `Filter(Scan)` would have used.
/// Note this is **not** the same as `c == v`: testing that through `values_compare`
/// is vacuous wherever the pair's cross-type comparison itself routes through `f64`,
/// because the coercion round-trips through that same coarse comparator and so always
/// reports `Equal`. Two conditions together give the invariant:
///
/// - **`c` is an exact stand-in for `v`** — checked with `values_compare`, which
///   catches the lossy coercions (`1.5 → int4 2`, an out-of-range or absent arm).
/// - **the comparator the substitution swaps in is no coarser than the one it swaps
///   out** — a `numeric` or `int8` key compares *exactly* as a key but through `f64`
///   against a float probe, so an `f64`-indistinguishable pair of keys
///   (`0.3` vs `0.30000000000000000001`, `2^53` vs `2^53 + 1`) would be separated by
///   the index and merged by `Filter(Scan)`. Those pairs are declined outright; every
///   other pair either shares its comparator with the key order or is already caught
///   by the exactness check (an `int2`/`int4` key is exact in `f64`, and a `float4` /
///   `float8` key compares as `f64` on both sides).
///
/// Postgres reaches the same answers by evaluating in the *wider* type (`int4 > 1.5`
/// compares as numeric, `numeric = float8` as float8); pglet can't do that through a
/// typed index, so it declines the index instead of approximating. So an *inexact* probe
/// cannot make the two build profiles disagree — which is what this gate contributes on
/// the coercion axis, not a property of index acceleration at large: two measured
/// exceptions reach a profile disagreement by other routes (`docs/indexes.md`
/// § Build profiles).
///
/// (A bare *string* literal never gets here: `resolve_expr_types` wraps it in a
/// coercing `Cast`, which isn't row-independent and so is never lifted into a probe.
/// Non-string literals and bound `$N` params are the reachable cases, and a `$N` has
/// no type until bind — which is why this lives on the resolved runtime value rather
/// than in index selection. Statically-typed operand pairs are already rejected by the
/// planner's `check_expr_semantics` operand gate, so in practice the gate below fires
/// for bound params.)
///
/// `op` is the comparison the probe stands in for and `column_first` whether the user
/// wrote the column on the left; together they reconstruct the predicate **as
/// written** for the error text, so the message matches the stripped profile's
/// `Filter(Scan)` byte for byte rather than merely sharing a SQLSTATE.
///
/// # Errors
///
/// [`pglet_core::Error::UndefinedFunction`] (`42883`) when the probe's type and the key column's
/// type have no comparison operator.
#[cfg(feature = "index_acceleration")]
fn probe_key_value(
    value: Value,
    key_column: &str,
    table_def: &crate::catalog::Table,
    op: BinaryOp,
    column_first: bool,
) -> Result<Option<Value>> {
    // NULL carries no type: it passes any gate, needs no coercion, and the callers'
    // null guards turn it into an empty result (`col OP NULL` is NULL for every row).
    if matches!(value, Value::Null) {
        return Ok(Some(value));
    }
    // Structurally unreachable — index selection resolved `key_column` against this
    // same catalog — but a missing column has no type to gate or coerce against.
    let Some(col) = table_def.columns.iter().find(|c| c.name == key_column) else {
        return Ok(Some(value));
    };
    if value.type_oid() == Some(col.type_oid) {
        return Ok(Some(value));
    }
    // Report the operands in the order the predicate was written.
    let (left, right, symbol) = if column_first {
        (
            Some(col.type_oid),
            value.type_oid(),
            comparison_symbol(op).unwrap_or("="),
        )
    } else {
        (
            value.type_oid(),
            Some(col.type_oid),
            comparison_symbol(mirrored_comparison_op(op)).unwrap_or("="),
        )
    };
    // Plain operands, and **not** because there is no identity to read: `col` is a
    // `catalog::Column`, which carries `enum_type` — the very field `build_column_type_map`
    // reads. What carries none is the runtime `Value` on the other side, so a pair built
    // here could only ever be half-identified. Two things make that cost nothing, and both
    // are properties of what reaches this call rather than of this line: index selection
    // (`planner::index_selection`'s `single_column_index_for`) declines an enum key column
    // outright, so no probe is lifted over one; and an enum column's `type_oid` **is**
    // `Text`, so a `Value::Text` probe takes the equal-OID early return above before it
    // gets here at all. The pair `EDGES.md` row 114 names is refused at plan time, above
    // both build profiles, so the two cannot disagree over it.
    comparison_legality(
        left.map(ResolvedType::plain),
        right.map(ResolvedType::plain),
        symbol,
    )?;
    // The key order is finer than the comparator `Filter(Scan)` would use, so no
    // coerced value can reproduce it — decline before the (here vacuous) exactness
    // check even looks.
    if comparator_is_coarser_than_key(col.type_oid, value.type_oid()) {
        return Ok(None);
    }
    // Exact-or-decline: only a coercion that round-trips to an *equal* value may
    // stand in for the original.
    let Ok(coerced) = super::super::coerce::coerce_value(value.clone(), col.type_oid) else {
        return Ok(None);
    };
    if values_compare(&coerced, &value) == Some(std::cmp::Ordering::Equal) {
        Ok(Some(coerced))
    } else {
        Ok(None)
    }
}

/// Whether [`values_compare`] decides `(key_type, probe_type)` through `f64` while
/// the index's key order for `key_type` is **exact** — the pairs no coerced probe
/// value can serve, because the index separates keys the scan comparator merges.
///
/// `values_compare` reaches `f64` for any pair it can't compare exactly: a `numeric`
/// against a float goes through the numeric's `f64` (Postgres's own
/// `numeric`-vs-`float8` operator does the same), and an integer against a float
/// widens both to `f64` (two integers compare as integers). Of those, only a
/// `numeric` or `int8` **key** orders more finely than `f64` — `int2`/`int4` are
/// exact in `f64`, and a `float4`/`float8` key already *is* an `f64` order on both
/// sides. So a `numeric` or `int8` column probed with a `float4`/`float8` value (a
/// driver binding an `f64`) is the whole set: `WHERE n = $1` over `0.3`, `0.30000000000000000001` must return both rows,
/// and an index probe on the exact numeric key would return one.
///
/// Everything else — same-type, `numeric` vs an integer, `date` vs `timestamp`,
/// `time` vs `interval` — either compares exactly on both sides or is caught by the
/// caller's exactness check.
#[cfg(feature = "index_acceleration")]
const fn comparator_is_coarser_than_key(key_type: TypeOid, probe_type: Option<TypeOid>) -> bool {
    matches!(key_type, TypeOid::Numeric | TypeOid::Int8)
        && matches!(probe_type, Some(TypeOid::Float4 | TypeOid::Float8))
}

/// The rows of `table` whose `key_column` satisfies every `(op, value)` predicate,
/// found by a full scan — the un-accelerated fallback for a probe/bound
/// [`probe_key_value`] declined.
///
/// This is deliberately the same evaluation the stripped profile's `Filter(Scan)`
/// performs: the comparison runs through [`eval_binary_op_ordered`] on the row's own value,
/// in the column's own type, so the cross-type semantics are Postgres's rather than
/// an index key's. A row whose key is NULL never satisfies a comparison and drops out.
///
/// `order_by_key` reproduces the index's ordering contract for the range path, whose
/// `ascending`/`limit` may be carrying an **ORDER BY pushdown** (the planner elided
/// the `Sort` on the promise that the scan returns key order). The sort is stable, so
/// equal keys keep scan order. The equality path passes `false`: all matched keys are
/// equal, and it never carries a pushdown.
///
/// Both the filter and the sort read the key column's **enum member order**, derived
/// here from `schema` by [`enum_key_order`] — not because either can reach an enum today
/// (`planner::index_selection`'s `single_column_index_for` declines an enum key column,
/// so no plan puts one here), but because *this function must not be correct only by
/// that decline*. It once was: it filtered through `eval_binary_op` and sorted through
/// `values_compare`, both label-byte, with nothing in either function naming the
/// coupling — so a later narrowing of the decline (allow equality, decline range and
/// order, say) would have silently reopened a mis-ordered read, on the accelerated
/// profile only. The derivation is **inside** rather than an argument the two callers
/// pass: as a parameter it was a wiring both call sites could quietly drop, and
/// dropping it reddened nothing.
///
/// # Errors
///
/// A storage failure, or an evaluation error from one of the comparisons.
#[cfg(feature = "index_acceleration")]
// The un-accelerated twin of `execute_index_range_scan`, so it carries the same
// (table, key, bounds, order, limit, storage) shape that one does.
#[allow(clippy::too_many_arguments)]
fn scan_filtered_by_key<S: Storage>(
    table: &str,
    key_column: &str,
    table_def: &crate::catalog::Table,
    predicates: &[(BinaryOp, Value)],
    order_by_key: bool,
    ascending: bool,
    limit: Option<usize>,
    schema: &Schema,
    storage: &S,
) -> Result<Vec<Vec<Value>>> {
    let key_enum_order = enum_key_order(schema, table_def, key_column);
    let Some(key_idx) = table_def.columns.iter().position(|c| c.name == key_column) else {
        // Unreachable for the same reason as in `probe_key_value`; with no key column
        // there is nothing to filter on.
        return Ok(storage.scan(table)?.into_iter().map(|(_, r)| r).collect());
    };
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for (_, row) in storage.scan(table)? {
        let mut keep = true;
        for (op, bound) in predicates {
            if !matches!(
                eval_binary_op_ordered(*op, &row[key_idx], bound, key_enum_order)?,
                Value::Bool(true)
            ) {
                keep = false;
                break;
            }
        }
        if keep {
            rows.push(row);
        }
    }
    if order_by_key {
        rows.sort_by(|a, b| {
            let ord = values_compare_ordered(&a[key_idx], &b[key_idx], key_enum_order)
                .unwrap_or(std::cmp::Ordering::Equal);
            if ascending { ord } else { ord.reverse() }
        });
    }
    if let Some(n) = limit {
        rows.truncate(n);
    }
    Ok(rows)
}

/// The enum member order of `table_def`'s `key_column` — the order
/// [`scan_filtered_by_key`] filters and sorts by.
///
/// Always `None` today, and deliberately *derived* rather than written as one:
/// `single_column_index_for` declines an enum key column, so no `IndexScan` /
/// `IndexRangeScan` is planned over one and this fallback never sees an enum. Deriving
/// it is what keeps the fallback's correctness from resting on that decline.
#[cfg(feature = "index_acceleration")]
fn enum_key_order<'s>(
    schema: &'s Schema,
    table_def: &crate::catalog::Table,
    key_column: &str,
) -> Option<&'s [String]> {
    let col = table_def.columns.iter().find(|c| c.name == key_column)?;
    super::super::helpers::enum_order_of(&schema.enums, col.enum_type.as_deref())
}

/// The effective table name a single-table plan resolves qualified references
/// against — its FROM-clause alias if present, else the real table name.
///
/// Threaded into `SimpleColumns::with_table` so an alias-qualified reference
/// (`s.open_signup`, `c.refs`) resolves — and into the correlated-subquery outer
/// frame, so a correlated `c.id` binds to the right outer scope. Returns `None`
/// for a JOIN or any multi/zero-source plan (those resolve via `JoinColumnMap`,
/// which is already alias-aware), descending pass-through nodes to the base scan.
///
/// **What the `None` means is the load-bearing part**, because both consumers read it
/// as *this scope has no relation identity* and take a different resolution rule on it:
/// `SimpleColumns::resolve` drops the qualifier and matches the bare name, and
/// `EvalEnv::resolve_outer_slot` treats the frame as addressing several relations, so it
/// answers the synthesized `"relation.column"` spelling a user's quoted `"a.b"` column is
/// byte-identical to. That is what a missing FROM-function arm cost: over
/// `unnest(ARRAY['hit','miss']) AS v("a.b")`, a correlated `… WHERE u2.yv = a.b` answered
/// rows where 18.6 raises `42P01`, and at the unnest's own level `SELECT zzz.b FROM
/// unnest(ARRAY['x','y']) AS v(b)` answered `b` against the same `42P01`.
///
/// **The obligation a missing arm carries has inverted, and it now costs a legitimate
/// query rather than a bogus one.** While the outer chain fell back to a qualifier-blind
/// bare match, a FROM kind with no arm here merely *over-accepted* — the reference found
/// the name anyway. The chain now stops at the frame that owns the qualifier and declines
/// where none does, so a scope this function cannot name owns nothing, and an ordinary
/// `alias.col` correlated reference into it is refused. Today no arm is missing for a
/// reachable shape: `Distinct` / `DistinctOn` / `SetOp` / `With` /
/// `ProjectUnnest` / `Values` have none, and none of them is a `Plan::Filter` or
/// `Plan::Project` input (a derived table wrapping any of them lowers to `SubqueryScan`,
/// which does). A parser change that makes one of them a FROM item directly must add its
/// arm in the same commit, and the symptom to expect is a `42P01` on a statement 18.6
/// answers — the opposite direction from the one this paragraph used to warn about.
///
/// The FROM-item arms, which live in [`from_item_name`], cover the kinds
/// `parse_table_factor` produces — base `Scan` (and its index-accelerated forms),
/// `VirtualScan`, `CteScan`, `SubqueryScan`, `FunctionScan` (`unnest`,
/// `generate_series`) — so a FROM shape the parser gains later needs one added there,
/// and [`level_relation_names`] reads the same arms. It is not the only walk over those
/// kinds and they do not all cover the same ones: the planner's `plan_relations` carries a `FunctionScan` arm
/// too; the planner's `leaf_qualifier` carries one for an item
/// whose name no other item at its level shares, and `collect_nonbase_into` one for a
/// `generate_series` item so named and none for an `unnest` (see the latter's comment for
/// what that costs). These are separate derivations, not one.
pub(super) fn single_table_name(plan: &Plan) -> Option<String> {
    match plan {
        Plan::Filter { input, .. }
        | Plan::Project { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Aggregate { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Window { input, .. } => single_table_name(input),
        _ => from_item_name(plan).map(str::to_owned),
    }
}

/// The name a FROM item puts in its level's namespace — Postgres's `eref`: a base
/// relation's alias when one is written, else its own name; a catalog view's alias, else
/// its bare name; and the alias of a derived table, a CTE reference and a FROM-clause
/// function — the last carrying Postgres's `chooseScalarFunctionAlias` name (the explicit
/// alias, else the function's), the same one `plan_relations` registers it under and
/// `get_table_name_from_plan` keys its join columns on. `None` for a plan that is not a
/// FROM item.
///
/// The leaf rule [`single_table_name`], [`level_relation_names`] and the join executor's
/// `get_table_name_from_plan` read, so they cannot disagree about what a FROM item is called.
pub(super) fn from_item_name(plan: &Plan) -> Option<&str> {
    match plan {
        Plan::Scan { table, alias } => Some(alias.as_deref().unwrap_or(table)),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { table, alias, .. }
        | Plan::IndexRangeScan { table, alias, .. }
        | Plan::GinScan { table, alias, .. } => Some(alias.as_deref().unwrap_or(table)),
        Plan::VirtualScan {
            catalog_table,
            alias,
        } => Some(alias.as_deref().unwrap_or_else(|| catalog_table.name())),
        Plan::SubqueryScan { alias, .. }
        | Plan::CteScan { alias, .. }
        | Plan::FunctionScan { alias, .. } => Some(alias),
        _ => None,
    }
}

/// The names the FROM items of `plan`'s own query level put in scope — every relation a
/// qualifier written at that level binds before any enclosing level is asked
/// (`refnameNamespaceItem` searches the levels innermost first and stops at the first one
/// carrying the name).
///
/// `plan` is any node of the level: the walk passes down through the nodes a level stacks
/// over its FROM clause (`Filter`, `Project`, `Sort`, `Limit`, `Aggregate`, `Window`,
/// `JunkFilter`, `Distinct`, `DistinctOn`, `LockRows`), takes **both** sides of a join,
/// and stops at each FROM item ([`from_item_name`]) — a derived table, a CTE reference or
/// a set operation is another level, and is never entered. A level with no FROM clause
/// (`Values`, `ProjectUnnest`) or topped by a set operation names nothing.
///
/// This is what a resolver over a node's **output** names is told about its level
/// ([`SimpleColumns::within_level`](super::super::expr::SimpleColumns::within_level)):
/// those rows carry no relation identity of their own — a `GROUP BY` / `ORDER BY` /
/// window key, an aggregate's argument, a grouped or windowed select list are evaluated
/// over the node's input names — and without the level's names such a resolver handed a
/// qualifier its own level owns to an enclosing frame carrying the same name.
///
/// The planner's `planner::correlation::push_level_relations` is the sibling derivation,
/// over the same level structure, for the plan-time gate; the two are separate walks, not
/// one. That one also passes through a `Plan::With` to its body, where this walk stops.
/// No caller reaches that difference today: a `With` is built only as a statement's top
/// node, so the nodes that ask this walk sit inside its body, and a `WITH` written inside
/// a derived table or a sub-select is refused by the parser.
pub(super) fn level_relation_names(plan: &Plan) -> Vec<&str> {
    let mut names = Vec::new();
    collect_level_relation_names(plan, &mut names);
    names
}

/// The [`LevelScope`](super::super::expr::LevelScope) a node evaluating over `input`'s
/// rows (`columns`, as executed) resolves an owned qualifier against: the level's FROM
/// names ([`level_relation_names`]), and — only where those name two or more items and
/// `input` is a join's rows — the input's `(relation, column)` pairs ([`plan_scope`],
/// kept only when they line up with `columns`, as [`scope_refs`] keeps them).
///
/// Whether the input is a join's is the plan's answer, never a column name's: a user's
/// quoted `"o.z"` column spells `o.` exactly as a join's synthesized name does. A
/// projection's output declines `plan_scope`, so above one the pairs are `None` and the
/// names are output names. One FROM item puts no pairs in scope, and is not walked for
/// them.
pub(super) fn level_scope<'a>(
    input: &'a Plan,
    columns: &[ColumnDesc],
    schema: &Schema,
) -> super::super::expr::LevelScope<'a> {
    let names = level_relation_names(input);
    let pairs = if names.len() > 1 {
        plan_scope(input, schema).filter(|refs| refs.len() == columns.len())
    } else {
        None
    };
    super::super::expr::LevelScope { names, pairs }
}

/// [`level_relation_names`]' walk. A join chain recurses a level per join, as
/// [`plan_scope`]'s walk over the same chain does.
fn collect_level_relation_names<'a>(plan: &'a Plan, names: &mut Vec<&'a str>) {
    match plan {
        Plan::Filter { input, .. }
        | Plan::Project { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Aggregate { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Window { input, .. }
        | Plan::Distinct { input }
        | Plan::DistinctOn { input, .. }
        | Plan::LockRows { input } => collect_level_relation_names(input, names),
        Plan::Join { left, right, .. } => {
            collect_level_relation_names(left, names);
            collect_level_relation_names(right, names);
        }
        _ => names.extend(from_item_name(plan)),
    }
}

/// The `(relation, column)` pairs a plan's output columns belong to — the executor's
/// namespace for a **multi-relation** scope, positionally parallel to the columns
/// [`execute_plan_node`](super::execute_plan_node) materializes for the same plan.
///
/// `None` where the shape cannot be derived, and a caller must then fall back to what it
/// had: [`single_table_name`]'s effective name, else no relation identity at all. A
/// decline is never a wrong pair — it is the absence of one.
///
/// # Why it is derived rather than carried
///
/// The executor's join scope has always *encoded* the pair in the column's name
/// (`format!("{relation}.{column}")`) and every consumer re-parsed it, which is forgeable
/// two ways: a column a user quoted a dot into is byte-identical to a synthesized name,
/// and a relation whose own name carries a dot splits at the wrong place. Both were
/// measured against 18.6 and both are silent (`EDGES.md`).
/// Deriving the pairs from the **plan** cannot forge either, because nothing here reads a
/// column's name as a spelling.
///
/// # The length check is what makes it safe
///
/// This is a second derivation of the executor's column shape, and the two agree only if
/// every arm is faithful. Callers therefore keep it **only when its length equals the
/// executed column count** — a mismatch anywhere in the recursion (a plan kind whose
/// executed width this does not model) shows up as a length mismatch at the top and
/// discards the whole list rather than misattributing a column.
///
/// # The obligation a new FROM kind inherits
///
/// The `_` arm asks [`single_table_name`], so a FROM kind that answers there is a single
/// relation here too and needs nothing added. A kind that puts **several** relations in
/// scope needs its own arm, or a join over it declines and its scope silently keeps the
/// forgeable string addressing.
pub fn plan_scope(plan: &Plan, schema: &Schema) -> Option<Vec<ColumnRef>> {
    // A join chain recurses from this frame and merges in one of its own, so a chain
    // costs this frame a level (`crate::depth`) — this walk cannot take the byte check.
    match plan {
        // Column-preserving wrappers pass their input's namespace up unchanged.
        Plan::Filter { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input }
        | Plan::DistinctOn { input, .. } => plan_scope(input, schema),

        // A join's namespace is Postgres's: the `USING` merges first (they belong to the
        // join itself, so no qualifier names them), then both sides in FROM order with
        // their own `USING` columns hidden from a bare lookup — still qualifiable, which
        // is what `transformFromClauseItem` does by leaving the component RTEs visible
        // and their columns not. The order mirrors `execute_join`'s own output exactly.
        Plan::Join {
            left, right, using, ..
        } => {
            let left_refs = plan_scope(left, schema)?;
            let right_refs = plan_scope(right, schema)?;
            Some(join_scope(left_refs, right_refs, using))
        }

        // Every FROM item that puts exactly one relation in scope, under the name
        // `single_table_name` gives it; anything that puts none declines.
        _ => {
            let name = single_table_name(plan)?;
            Some(
                plan.output_columns(schema)
                    .iter()
                    .map(|c| ColumnRef::of(&name, &c.name))
                    .collect(),
            )
        }
    }
}

/// One join's namespace from its sides' — see [`plan_scope`]'s `Join` arm.
#[inline(never)]
fn join_scope(
    left_refs: Vec<ColumnRef>,
    right_refs: Vec<ColumnRef>,
    using: &[String],
) -> Vec<ColumnRef> {
    let mut refs: Vec<ColumnRef> = using.iter().map(|c| ColumnRef::merged(c)).collect();
    refs.extend(left_refs.into_iter().map(|c| c.hidden_by_using(using)));
    refs.extend(right_refs.into_iter().map(|c| c.hidden_by_using(using)));
    refs
}

/// The scope identity to resolve a node's output columns through: its pairs when the
/// node puts several relations in scope and they line up with `columns`, else its single
/// relation's effective name, else nothing.
///
/// The `refs` it returns must outlive the identity, so the caller holds it — hence the
/// pair return rather than a bare [`ScopeIdentity`](super::super::expr::ScopeIdentity).
pub(super) fn scope_refs(
    plan: &Plan,
    columns: &[ColumnDesc],
    schema: &Schema,
) -> (Option<String>, Option<Vec<ColumnRef>>) {
    let table = single_table_name(plan);
    if table.is_some() {
        return (table, None);
    }
    let refs = plan_scope(plan, schema).filter(|refs| refs.len() == columns.len());
    (None, refs)
}

/// Execute a `Plan::Scan` — a full table scan.
///
/// The storage scan keys on the REAL `table`; the alias is presentation /
/// resolution-only, applied by the resolvers, not the catalog lookup.
pub(super) fn execute_scan<S: Storage>(
    table: &str,
    schema: &Schema,
    storage: &S,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let table_def = get_table_or_err(schema, table)?;

    // Scan returns (RowId, Vec<Value>) pairs; extract just the row values
    let rows: Vec<Vec<Value>> = storage
        .scan(table)?
        .into_iter()
        .map(|(_, row)| row)
        .collect();
    let columns = table_column_descs(table_def);

    Ok((rows, columns))
}

/// Execute a `Plan::IndexScan` — an index-accelerated point read.
///
/// Probe the secondary index for the row IDs whose `key_column` equals
/// `key_value`, then fetch each row. Returns the same `(rows, columns)` shape as
/// `Plan::Scan` — a wrapping `Filter` (built by the planner for any residual
/// conjuncts) narrows further. The probe is read-your-own-writes correct because
/// the transaction-aware storage adapter routes `index_lookup`/`get` through the
/// pending buffer (see `pglet::backend::StorageAdapter`).
///
/// A probe value the key column's type can't represent **exactly** (see
/// [`probe_key_value`]) falls back to [`scan_filtered_by_key`], so the equality is
/// answered in the column's own type instead of by approximated key bytes.
#[cfg(feature = "index_acceleration")]
// Mirrors the plan node's fields (table/index/key/probe/order) plus the shared
// schema/storage/env triple, like its `execute_index_range_scan` sibling.
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_index_scan<S: Storage>(
    table: &str,
    index_oid: i32,
    key_column: &str,
    key_value: &Expr,
    column_first: bool,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let table_def = get_table_or_err(schema, table)?;
    let columns = table_column_descs(table_def);
    let rows = match equality_probe_keys(key_value, key_column, column_first, table_def, env)? {
        // No exact key represents this value — evaluate the equality by scan.
        ProbeKeys::Inexact(predicates) => scan_filtered_by_key(
            table,
            key_column,
            table_def,
            &predicates,
            false,
            true,
            None,
            schema,
            storage,
        )?,
        keys => fetch_rows(
            table,
            probe_row_ids(index_oid, keys, true, None, storage)?,
            storage,
        )?,
    };
    Ok((rows, columns))
}

/// Execute a `Plan::IndexRangeScan` — an index-accelerated range read.
///
/// Probe the secondary index for the row IDs whose `key_column` falls in the
/// `lower`/`upper` range, then fetch each row. Returns the same `(rows, columns)`
/// shape as `Plan::Scan` — a wrapping `Filter` (built by the planner for any
/// residual conjuncts) narrows further, and an outer `Sort`/`Limit` runs
/// unchanged. The probe is read-your-own-writes correct because the
/// transaction-aware storage adapter routes `index_range_lookup`/`get` through
/// the pending buffer.
///
/// A bound the key column's type can't represent **exactly** (see
/// [`probe_key_value`]) drops the whole node to [`scan_filtered_by_key`] — the index
/// is declined rather than approximated, so the answer stays the one the stripped
/// profile and Postgres give. The fallback re-applies the key ordering and early-stop
/// because `ascending`/`limit` may be an ORDER-BY pushdown the planner already
/// elided the `Sort` for.
#[cfg(feature = "index_acceleration")]
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_index_range_scan<S: Storage>(
    table: &str,
    index_oid: i32,
    key_column: &str,
    lower: Option<&IndexBound>,
    upper: Option<&IndexBound>,
    ascending: bool,
    limit: Option<&crate::planner::LimitCount>,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let table_def = get_table_or_err(schema, table)?;
    let columns = table_column_descs(table_def);
    let rows = match range_probe_keys(lower, upper, key_column, table_def, env)? {
        // `col OP NULL` is NULL → no rows (matches Postgres).
        ProbeKeys::NoRow => Vec::new(),
        // Either bound inexact → no index probe can answer this range faithfully; fall
        // back to the scan the stripped profile would have run (keeping the key ordering
        // and early-stop the elided `Sort` promised).
        ProbeKeys::Inexact(predicates) => scan_filtered_by_key(
            table,
            key_column,
            table_def,
            &predicates,
            true,
            ascending,
            resolve_limit_count(limit, env)?,
            schema,
            storage,
        )?,
        // `ascending`/`limit` carry the ORDER-BY pushdown (an elided `Sort`):
        // for a bare WHERE they are `true`/`None`, but a `Sort` elided into an
        // ordered index scan sets the direction and an optional early-stop
        // count. Any outer `Plan::Limit` remains the correctness boundary; the
        // scan `limit` is purely an early-stop. A `$N` here is resolved from `env`
        // (the `ORDER BY … LIMIT $N` pushdown), matching the `Plan::Limit` arm.
        keys => {
            let scan_limit = resolve_limit_count(limit, env)?;
            fetch_rows(
                table,
                probe_row_ids(index_oid, keys, ascending, scan_limit, storage)?,
                storage,
            )?
        }
    };
    Ok((rows, columns))
}

/// What an equality or range probe's key values came to, before the index is asked —
/// the half [`execute_index_scan`] / [`execute_index_range_scan`] share with
/// [`probe_dml_targets`].
#[cfg(feature = "index_acceleration")]
enum ProbeKeys {
    /// An equality's one key, coerced to the key column's type.
    Equal(Value),
    /// A range's coerced bounds, each with its inclusivity; `None` is unbounded.
    Range {
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
    },
    /// A key or bound evaluated to NULL: `col OP NULL` holds for no row. Asking the index
    /// instead would be wrong both ways — `index_range_lookup` reads a NULL bound as
    /// unbounded, and a `NULLS NOT DISTINCT` index's `index_lookup` returns its
    /// NULL-keyed rows for a NULL key.
    NoRow,
    /// No exact key represents a probe value (see [`probe_key_value`]): the comparisons,
    /// as written, that a scan evaluates instead.
    Inexact(Vec<(BinaryOp, Value)>),
}

/// Evaluate an equality probe's key and gate it against the key column.
///
/// `key_value` is row-independent (literal / `$N`), so it is evaluated against an empty
/// row, resolving any `$N` from `env`. A NULL key is [`ProbeKeys::NoRow`]: `col = NULL`
/// is NULL for every row. Otherwise [`probe_key_value`] coerces it to the key column's
/// type so its index-key bytes match the stored entries (an integer literal `= 500`
/// against a `NUMERIC` column keys as numeric, not int4) — or reports it inexact.
///
/// # Errors
///
/// The key's evaluation error, or [`probe_key_value`]'s `42883`.
#[cfg(feature = "index_acceleration")]
fn equality_probe_keys(
    key_value: &Expr,
    key_column: &str,
    column_first: bool,
    table_def: &crate::catalog::Table,
    env: EvalEnv<'_>,
) -> Result<ProbeKeys> {
    let raw = eval_expr_env(key_value, env)?;
    if raw.is_null() {
        return Ok(ProbeKeys::NoRow);
    }
    let keyed = probe_key_value(
        raw.clone(),
        key_column,
        table_def,
        BinaryOp::Eq,
        column_first,
    )?;
    Ok(keyed.map_or_else(
        || ProbeKeys::Inexact(vec![(BinaryOp::Eq, raw)]),
        ProbeKeys::Equal,
    ))
}

/// Evaluate a range probe's bounds and gate each against the key column, lower first.
///
/// Each bound's expr is row-independent (literal / bound `$N`) and evaluated against an
/// empty row. A bound that evaluates to NULL makes `col OP NULL` NULL for every row, so
/// Postgres returns no rows — NOT an unbounded scan — and that is [`ProbeKeys::NoRow`]
/// whatever the other bound came to. The normalized operator (`col OP value`) comes from
/// the bound's slot and inclusivity; `column_first` recovers the spelling the user wrote
/// for the error text. A bound the key type can't represent exactly (see
/// [`probe_key_value`]) makes the whole range [`ProbeKeys::Inexact`].
///
/// # Errors
///
/// A bound's evaluation error, or [`probe_key_value`]'s `42883`.
#[cfg(feature = "index_acceleration")]
fn range_probe_keys(
    lower: Option<&IndexBound>,
    upper: Option<&IndexBound>,
    key_column: &str,
    table_def: &crate::catalog::Table,
    env: EvalEnv<'_>,
) -> Result<ProbeKeys> {
    let eval_bound = |bound: Option<&IndexBound>,
                      is_lower: bool|
     -> Result<Option<(BinaryOp, Value, Option<Value>)>> {
        let Some(b) = bound else { return Ok(None) };
        let op = match (is_lower, b.inclusive) {
            (true, true) => BinaryOp::Ge,
            (true, false) => BinaryOp::Gt,
            (false, true) => BinaryOp::Le,
            (false, false) => BinaryOp::Lt,
        };
        let raw = eval_expr_env(&b.value, env)?;
        let keyed = probe_key_value(raw.clone(), key_column, table_def, op, b.column_first)?;
        Ok(Some((op, raw, keyed)))
    };
    let lower_eval = eval_bound(lower, true)?;
    let upper_eval = eval_bound(upper, false)?;

    let is_null_bound = |e: &Option<(BinaryOp, Value, Option<Value>)>| {
        e.as_ref().is_some_and(|(_, raw, _)| raw.is_null())
    };
    if is_null_bound(&lower_eval) || is_null_bound(&upper_eval) {
        return Ok(ProbeKeys::NoRow);
    }
    let inexact = |e: &Option<(BinaryOp, Value, Option<Value>)>| {
        e.as_ref().is_some_and(|(_, _, keyed)| keyed.is_none())
    };
    if inexact(&lower_eval) || inexact(&upper_eval) {
        return Ok(ProbeKeys::Inexact(
            [lower_eval, upper_eval]
                .into_iter()
                .flatten()
                .map(|(op, raw, _)| (op, raw))
                .collect(),
        ));
    }
    let to_bound = |e: Option<(BinaryOp, Value, Option<Value>)>| {
        e.and_then(|(op, _, keyed)| keyed.map(|v| (v, matches!(op, BinaryOp::Ge | BinaryOp::Le))))
    };
    Ok(ProbeKeys::Range {
        lower: to_bound(lower_eval),
        upper: to_bound(upper_eval),
    })
}

/// Ask index `index_oid` for the row ids `keys` names, in the index's order
/// (`ascending` / `limit` apply to a range only). [`ProbeKeys::NoRow`] and
/// [`ProbeKeys::Inexact`] name no lookup and answer no ids — the callers branch on them
/// first.
///
/// The probe is read-your-own-writes correct because the transaction-aware storage
/// adapter routes `index_lookup` / `index_range_lookup` through the pending buffer (see
/// `pglet::backend::StorageAdapter`).
///
/// # Errors
///
/// A storage failure.
#[cfg(feature = "index_acceleration")]
fn probe_row_ids<S: Storage>(
    index_oid: i32,
    keys: ProbeKeys,
    ascending: bool,
    limit: Option<usize>,
    storage: &S,
) -> Result<Vec<RowId>> {
    match keys {
        // IndexScan stays single-column (composite IndexScan isn't planned), so the
        // probe tuple is a 1-element slice.
        ProbeKeys::Equal(value) => {
            Ok(storage.index_lookup(index_oid as u32, std::slice::from_ref(&value))?)
        }
        ProbeKeys::Range { lower, upper } => {
            Ok(storage.index_range_lookup(index_oid as u32, lower, upper, ascending, limit)?)
        }
        ProbeKeys::NoRow | ProbeKeys::Inexact(_) => Ok(Vec::new()),
    }
}

/// Fetch each of `ids`' rows from `table`, in `ids`' order, skipping an id with no row.
///
/// # Errors
///
/// A storage failure.
#[cfg(feature = "index_acceleration")]
fn fetch_rows<S: Storage>(table: &str, ids: Vec<RowId>, storage: &S) -> Result<Vec<Vec<Value>>> {
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(row) = storage.get(table, id)? {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// The candidate row ids of a simple `UPDATE` / `DELETE` whose plan carries a
/// `target_probe` (an equality `IndexScan` over the target), in **`RowId` order** — or
/// `Ok(None)`, a **decline**, which tells the caller to scan.
///
/// The caller tests every candidate against the whole `filter`; what this must
/// guarantee is that the probe leaves out no row that test would keep, and that leaving a
/// row out cannot change the statement's answer. The first is the read path's
/// exact-or-decline ([`probe_key_value`]) plus the index's own completeness: an inexact
/// key declines here, where the read path scans for it too. The second is the planner's
/// residual rule (`planner::index_selection`'s `dml_residual_is_certain`) plus one check
/// only execution can make: every parameter a conjunct compares a column with — the
/// probe's own key among them — must be bound and have a comparison operator against
/// that column's type ([`residual_parameters_admit_probe`]). Where one does not, the
/// scan raises the failure at the first row whose evaluation reaches the conjunct, and
/// over an empty table not at all; the probe would raise it before any row, or skip it
/// for every row it leaves out, so this declines instead.
///
/// After that check a key cannot fail to evaluate or to pass [`probe_key_value`]'s gate:
/// a parameter key was just evaluated and its operator asked, and a literal key's
/// operator was proved at plan time.
///
/// The ids are sorted (and deduplicated) here, because the scan visits rows in `RowId`
/// order and the writes must happen in the same one. Every backend's `index_lookup`
/// answers one key's ids in that order already — the B+tree's entries carry the row id
/// after the key, the memory index keeps each key's ids sorted, and the in-transaction
/// lookup sorts — so the sort keeps the write order from resting on that, rather than
/// reordering anything a backend returns today.
///
/// # Errors
///
/// A storage failure.
#[cfg(feature = "index_acceleration")]
pub fn probe_dml_targets<S: Storage>(
    probe: &Plan,
    filter: &Expr,
    table_def: &crate::catalog::Table,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<Option<Vec<RowId>>> {
    if !residual_parameters_admit_probe(filter, table_def, env) {
        return Ok(None);
    }
    let Plan::IndexScan {
        index_oid,
        key_column,
        key_value,
        column_first,
        ..
    } = probe
    else {
        return Ok(None);
    };
    let keys = equality_probe_keys(key_value, key_column, *column_first, table_def, env)?;
    if matches!(keys, ProbeKeys::Inexact(_)) {
        return Ok(None);
    }
    let mut ids = probe_row_ids(*index_oid, keys, true, None, storage)?;
    ids.sort_unstable();
    ids.dedup();
    Ok(Some(ids))
}

/// Whether every parameter `filter`'s top-level conjuncts compare a column with is bound
/// and has a comparison operator against that column's type — the execution-time half
/// of `dml_target_probe`'s residual rule (see [`probe_dml_targets`]).
///
/// A NULL carries no type and passes, as it passes the comparison's own gate.
#[cfg(feature = "index_acceleration")]
fn residual_parameters_admit_probe(
    filter: &Expr,
    table_def: &crate::catalog::Table,
    env: EvalEnv<'_>,
) -> bool {
    crate::planner::flatten_and(filter)
        .into_iter()
        .all(|conjunct| {
            let Expr::BinaryOp { left, op, right } = conjunct else {
                return true;
            };
            let Some(symbol) = comparison_symbol(*op) else {
                return true;
            };
            let ((Expr::Column { name: column, .. }, key)
            | (key, Expr::Column { name: column, .. })) = (&**left, &**right)
            else {
                return true;
            };
            if !key.any_node(&|e| matches!(e, Expr::Parameter { .. }).then_some(true)) {
                return true;
            }
            let Some(col) = table_def.columns.iter().find(|c| &c.name == column) else {
                return false;
            };
            eval_expr_env(key, env).is_ok_and(|value| {
                comparison_legality(
                    Some(ResolvedType::plain(col.type_oid)),
                    value.type_oid().map(ResolvedType::plain),
                    symbol,
                )
                .is_ok()
            })
        })
}

/// Execute a `Plan::GinScan` — a GIN-accelerated `@>` containment read, optionally
/// an OR-union of probes guarded by scan-independent conditions.
///
/// First evaluate the `guards` (resolving `$N` and correlated outer-scope columns
/// from `env`): if any is `true` (e.g. the param bound to `$p IS NULL` is null), the
/// originating `OR` is unconditionally satisfied for every row, so the candidate set
/// is **every** row and the recheck `Filter` passes them all. A guard that fails to
/// evaluate hands the `Filter` every row too, which raises the failure only for a row
/// whose `OR` reaches the guard.
///
/// Otherwise evaluate each `probe` once (same eval-time resolution the other index
/// scans use), decompose it into its GIN query keys, intersect their posting lists
/// via `gin_lookup`, and **union** the per-probe candidate row sets (deduplicated by
/// row id, fetched once). Returns the same `(rows, columns)` shape as `Plan::Scan` —
/// the planner **always** wraps this in a recheck `Filter` carrying the real
/// predicate (GIN is lossy), which narrows the candidate superset to the true
/// matches. RYOW-correct via the transaction-aware adapter.
///
/// An **empty** decomposition for any probe (the probe is NULL, or an empty
/// array/`{}`) can't be accelerated and the two cases differ (`x @> NULL` matches
/// nothing; `x @> '{}'` matches everything), so it returns **every** row as a
/// candidate and lets the recheck `Filter` decide — correct for both. A probe whose
/// type is not the key column's declines the same way (exact-or-decline): the
/// `Filter` then answers for the pair as the stripped build's does.
#[cfg(feature = "index_acceleration")]
// One argument per `Plan::GinScan` field beside the ones every scan executor takes.
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_gin_scan<S: Storage>(
    table: &str,
    index_oid: i32,
    key_column: &str,
    probes: &[Expr],
    guards: &[Expr],
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let table_def = get_table_or_err(schema, table)?;
    let columns = table_column_descs(table_def);
    let key_type = table_def
        .columns
        .iter()
        .find(|c| c.name == key_column)
        .map(|c| c.type_oid);

    // Every row as a candidate — the fallback when a guard fires or a probe can't
    // be decomposed; the recheck `Filter` does the real narrowing.
    let all_rows = || -> Result<Vec<Vec<Value>>> {
        Ok(storage
            .scan(table)?
            .into_iter()
            .map(|(_, row)| row)
            .collect())
    };

    // A satisfied guard makes the OR unconditionally true → every row is a candidate.
    for guard in guards {
        // A guard is a **disjunct** of the query's top-level `OR`, lifted out of the
        // predicate the recheck `Filter` still carries whole — so it is judged by the
        // same resolver, under the context it actually sits in. The two must move
        // together: widening the `Filter` alone loses every row admitted only by a
        // guard, because such a row matches no GIN probe and never reaches the
        // `Filter` at all — a silent row loss on the accelerated profile only.
        //
        // A guard that **fails** is not an answer here: it is evaluated once, ahead of
        // every row, where the `OR` it was lifted out of evaluates it per row and only
        // when no earlier disjunct decides the row (`connective_decided_by`). So a
        // failure hands the recheck `Filter` every row, and the `Filter` raises it
        // exactly where the stripped profile's `Filter(Scan)` does — for a row that
        // reaches it, and for no row of an empty table.
        let Ok(holds) =
            eval_expr_env(guard, env).and_then(|v| predicate_holds(&v, BoolContext::Or))
        else {
            return Ok((all_rows()?, columns));
        };
        if holds {
            return Ok((all_rows()?, columns));
        }
    }

    // Union each probe's posting-list intersection, deduplicated by row id so a row
    // matched by several probes is fetched once. (A single probe with no guards is
    // the simple `col @> <const>` case — the union is a no-op over one set.)
    let mut seen: std::collections::HashSet<_> = std::collections::HashSet::new();
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for probe in probes {
        let probe_value = eval_expr_env(probe, env)?;
        let keys = crate::gin::gin_decompose(&probe_value);
        // Empty decomposition → can't accelerate; hand the recheck `Filter` every row.
        // So does a probe of another type than the column's: its keys are not this
        // index's (`text[] @> ARRAY['1'::jsonb]` has no operator), and an empty
        // posting list would answer "no rows" where the `Filter` refuses the pair.
        if keys.is_empty() || probe_value.type_oid() != key_type {
            return Ok((all_rows()?, columns));
        }
        for id in storage.gin_lookup(index_oid as u32, &keys)? {
            if seen.insert(id)
                && let Some(row) = storage.get(table, id)?
            {
                rows.push(row);
            }
        }
    }
    Ok((rows, columns))
}

/// Execute a `Plan::VirtualScan` — a `pg_catalog` introspection table.
// Infallible, but returns `Result` to match the sibling scan executors and the
// `Result`-typed `execute_plan_node` dispatch arm.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn execute_virtual_scan(
    catalog_table: CatalogTable,
    schema: &Schema,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let pg_catalog = PgCatalog::new(schema);

    let rows = match catalog_table {
        CatalogTable::PgClass => pg_catalog.pg_class(),
        CatalogTable::PgNamespace => pg_catalog.pg_namespace(),
        CatalogTable::PgAttribute => pg_catalog.pg_attribute(),
        CatalogTable::PgType => pg_catalog.pg_type(),
        CatalogTable::PgTables => pg_catalog.pg_tables(),
        CatalogTable::PgIndexes => pg_catalog.pg_indexes(),
        CatalogTable::InformationSchemaTables => InformationSchema::new(schema).tables(),
        CatalogTable::InformationSchemaColumns => InformationSchema::new(schema).columns(),
        CatalogTable::InformationSchemaTableConstraints => {
            InformationSchema::new(schema).table_constraints()
        }
        CatalogTable::InformationSchemaKeyColumnUsage => {
            InformationSchema::new(schema).key_column_usage()
        }
        CatalogTable::InformationSchemaReferentialConstraints => {
            InformationSchema::new(schema).referential_constraints()
        }
        CatalogTable::InformationSchemaConstraintColumnUsage => {
            InformationSchema::new(schema).constraint_column_usage()
        }
        CatalogTable::PgConstraint => pg_catalog.pg_constraint(),
        CatalogTable::PgIndex => pg_catalog.pg_index(),
        CatalogTable::PgEnum => pg_catalog.pg_enum(),
        CatalogTable::InformationSchemaSequences => InformationSchema::new(schema).sequences(),
        CatalogTable::InformationSchemaCheckConstraints => {
            InformationSchema::new(schema).check_constraints()?
        }
    };

    let columns = catalog_table.columns();

    Ok((rows, columns))
}

#[cfg(test)]
mod scope_tests {
    use super::{level_relation_names, plan_scope, scope_refs};
    use crate::catalog::{Column, Schema, Table};
    use crate::planner::{Expr, JoinType, Literal, Plan};
    use crate::types::{ColumnDesc, TypeOid};

    /// A schema holding `a(id, w)` and `b(id, y)`.
    fn two_table_schema() -> Schema {
        let mut schema = Schema::new();
        for (name, second) in [("a", "w"), ("b", "y")] {
            let columns = ["id", second]
                .iter()
                .enumerate()
                .map(|(i, c)| Column {
                    name: (*c).to_string(),
                    type_oid: TypeOid::Int4,
                    nullable: true,
                    position: i as i16 + 1,
                    default: None,
                    enum_type: None,
                    numeric_typmod: None,
                    varchar_length: None,
                    temporal_precision: None,
                    identity: None,
                })
                .collect();
            schema.create_table(Table {
                name: name.to_string(),
                schema: "public".to_string(),
                oid: 0,
                checks: Vec::new(),
                foreign_keys: Vec::new(),
                columns,
            });
        }
        schema
    }

    fn scan(table: &str) -> Plan {
        Plan::Scan {
            table: table.to_string(),
            alias: None,
        }
    }

    fn join(using: &[&str]) -> Plan {
        Plan::Join {
            left: Box::new(scan("a")),
            right: Box::new(scan("b")),
            join_type: JoinType::Inner,
            on: Expr::Literal(Literal::Bool(true)),
            using: using.iter().map(|c| (*c).to_string()).collect(),
        }
    }

    fn spelled(plan: &Plan, schema: &Schema) -> Vec<String> {
        plan_scope(plan, schema)
            .expect("a join over two scans has derivable pairs")
            .iter()
            .map(|r| {
                format!(
                    "{}.{}{}",
                    r.relation.as_deref().unwrap_or("-"),
                    r.name,
                    if r.bare_visible { "" } else { "!" }
                )
            })
            .collect()
    }

    /// A join's pairs are both sides in FROM order under their own relation names, and a
    /// `USING` merge leads them: the merge belongs to the join (`-`), and each side's own
    /// copy stays qualifiable but hidden from a bare lookup (`!`).
    #[test]
    fn a_join_scope_is_its_sides_pairs_with_the_using_merge_leading() {
        let schema = two_table_schema();
        assert_eq!(spelled(&join(&[]), &schema), ["a.id", "a.w", "b.id", "b.y"]);
        assert_eq!(
            spelled(&join(&["id"]), &schema),
            ["-.id", "a.id!", "a.w", "b.id!", "b.y"]
        );
    }

    /// **The length check is what makes a derived list safe**, and nothing else in the
    /// engine can observe it: [`plan_scope`] models the executor's column shape a second
    /// time, so a plan kind it gets wrong shows up as a width disagreement at the top —
    /// and the list is then discarded rather than misattributing a column to a relation.
    /// No fixture reaches a disagreeing shape, which is exactly why this is a unit test.
    #[test]
    fn scope_refs_discards_pairs_that_do_not_span_the_executed_columns() {
        let schema = two_table_schema();
        let plan = join(&[]);
        let four: Vec<ColumnDesc> = (0..4)
            .map(|i| ColumnDesc::new(format!("c{i}"), TypeOid::Int4))
            .collect();
        let (table, refs) = scope_refs(&plan, &four, &schema);
        assert!(table.is_none() && refs.is_some_and(|r| r.len() == 4));
        let (_, refs) = scope_refs(&plan, &four[..3], &schema);
        assert!(refs.is_none(), "a width disagreement must discard the list");
    }

    /// A plan kind that puts no nameable relation in scope declines, and a join over it
    /// declines with it — the caller then keeps the addressing it had rather than a
    /// half-derived namespace.
    #[test]
    fn a_scope_with_no_nameable_relation_declines() {
        let schema = two_table_schema();
        let values = || Plan::Values {
            rows: Vec::new(),
            certain: crate::planner::CertainTypes::default(),
            enclosing: crate::planner::EnclosingColumns::default(),
        };
        assert!(plan_scope(&values(), &schema).is_none());
        let over_values = Plan::Join {
            left: Box::new(scan("a")),
            right: Box::new(values()),
            join_type: JoinType::Inner,
            on: Expr::Literal(Literal::Bool(true)),
            using: Vec::new(),
        };
        assert!(plan_scope(&over_values, &schema).is_none());
    }
    /// The names a level's FROM clause puts in scope, read from any node of the level:
    /// both sides of a join, every FROM item kind, through the nodes a level stacks over
    /// its FROM clause — and never a name from inside a derived table or a set
    /// operation's arms, which are other levels.
    #[test]
    fn level_relation_names_are_the_levels_from_items_only() {
        let schema = two_table_schema();
        let names = |sql: &str| -> Vec<String> {
            let plan = crate::parser::parse(sql, &schema).unwrap().remove(0);
            level_relation_names(&plan)
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        assert_eq!(
            names(
                "SELECT max(x.id) FROM a x JOIN b ON true GROUP BY x.w HAVING count(*) > 0 ORDER BY 1 LIMIT 1"
            ),
            ["x", "b"]
        );
        assert_eq!(
            names("SELECT id FROM a JOIN b USING (id) ORDER BY a.w"),
            ["a", "b"]
        );
        assert_eq!(
            names("SELECT s.id FROM (SELECT q.id FROM a q, b r) s ORDER BY s.id"),
            ["s"]
        );
        assert_eq!(
            names(
                "SELECT count(*) FROM unnest(ARRAY[1]) u(v), generate_series(1, 2) g(i), (VALUES (1)) v(n)"
            ),
            ["u", "g", "v"]
        );
        assert_eq!(
            names("SELECT DISTINCT ON (x.w) x.id FROM a x ORDER BY x.w, x.id"),
            ["x"]
        );
        assert_eq!(
            names("SELECT x.id, rank() OVER (ORDER BY x.w) FROM a x"),
            ["x"]
        );
        assert_eq!(names("SELECT DISTINCT \"Q\".id FROM a \"Q\""), ["Q"]);
        assert!(names("SELECT 1 ORDER BY 1").is_empty());
        assert!(names("SELECT id FROM a UNION SELECT id FROM b ORDER BY 1").is_empty());
    }
}

#[cfg(all(test, feature = "index_acceleration"))]
mod tests {
    use super::probe_key_value;
    use crate::catalog::{Column, Table};
    use crate::planner::BinaryOp;
    use crate::types::{PgInterval, PgNumeric, TypeOid, Value};

    /// A one-column table named `t` whose single column `k` has `type_oid`.
    fn table_with_key(type_oid: TypeOid) -> Table {
        Table {
            name: "t".to_string(),
            schema: "public".to_string(),
            oid: 1,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
            columns: vec![Column {
                name: "k".to_string(),
                type_oid,
                nullable: true,
                position: 1,
                default: None,
                enum_type: None,
                numeric_typmod: None,
                varchar_length: None,
                temporal_precision: None,
                identity: None,
            }],
        }
    }

    fn probe(
        value: Value,
        type_oid: TypeOid,
        op: BinaryOp,
        column_first: bool,
    ) -> super::Result<Option<Value>> {
        probe_key_value(value, "k", &table_with_key(type_oid), op, column_first)
    }

    #[test]
    fn probe_rejection_reports_the_written_operand_order() {
        // A bound `$N` is the one probe shape the planner's static gate can't reach
        // (a parameter has no type until bind), so the message this builds is what a
        // user sees for `WHERE $1 = k` / `WHERE $1 > k` — and it has to read the way
        // they wrote it, matching what the stripped build's `Filter(Scan)` prints.
        let written_column_first = probe(Value::Int4(5), TypeOid::Text, BinaryOp::Eq, true)
            .expect_err("text = integer has no operator");
        assert_eq!(
            written_column_first.to_string(),
            "operator does not exist: text = integer"
        );

        let written_value_first = probe(Value::Int4(5), TypeOid::Text, BinaryOp::Eq, false)
            .expect_err("integer = text has no operator");
        assert_eq!(
            written_value_first.to_string(),
            "operator does not exist: integer = text"
        );

        // A range predicate the planner normalized from `5 > k` to `k < 5` must report
        // the *written* `>`, not the normalized `<`.
        let normalized = probe(Value::Int4(5), TypeOid::Text, BinaryOp::Lt, false)
            .expect_err("integer > text has no operator");
        assert_eq!(
            normalized.to_string(),
            "operator does not exist: integer > text"
        );
        let as_written = probe(Value::Int4(5), TypeOid::Text, BinaryOp::Lt, true)
            .expect_err("text < integer has no operator");
        assert_eq!(
            as_written.to_string(),
            "operator does not exist: text < integer"
        );
    }

    #[test]
    fn probe_declines_an_inexact_coercion_and_keeps_an_exact_one() {
        // Exact: an int4 probe against a NUMERIC key coerces losslessly, so the index
        // may serve it (and must — that's the cross-type-literal acceleration).
        assert_eq!(
            probe(Value::Int4(500), TypeOid::Numeric, BinaryOp::Eq, true).unwrap(),
            Some(Value::Numeric(PgNumeric::from_i128(500))),
        );
        // Inexact: `1.5` rounds to an int4 key, which is a *different* value — the
        // probe must decline (`None`) rather than answer `> 2` for `> 1.5`.
        assert_eq!(
            probe(
                Value::Numeric(PgNumeric::parse("1.5").unwrap()),
                TypeOid::Int4,
                BinaryOp::Gt,
                true
            )
            .unwrap(),
            None,
        );
        // No coercion at all: `interval → time` has no arm, and the pair is legal
        // (Postgres casts the other way), so this must decline instead of keeping the
        // raw interval and keying the index on its bytes.
        assert_eq!(
            probe(
                Value::Interval(PgInterval::from_micros(3_600_000_000)),
                TypeOid::Time,
                BinaryOp::Eq,
                true
            )
            .unwrap(),
            None,
        );
        // NULL carries no type: it passes the gate untouched and the caller's null
        // guard turns it into an empty result.
        assert_eq!(
            probe(Value::Null, TypeOid::Int4, BinaryOp::Eq, true).unwrap(),
            Some(Value::Null),
        );
    }

    #[test]
    fn probe_declines_a_float_against_an_exactly_ordered_key() {
        // A float probe compares through `f64` while a NUMERIC / BIGINT key orders
        // exactly, so the index would separate keys `Filter(Scan)` merges. The
        // round-trip exactness check can't see it — it runs through the same coarse
        // comparator and always reports Equal — so these decline on the type pair.
        // Reachable from a driver binding an `f64` (`WHERE n = $1`).
        for probe_value in [Value::Float8(0.3), Value::Float4(0.3)] {
            assert_eq!(
                probe(probe_value.clone(), TypeOid::Numeric, BinaryOp::Eq, true).unwrap(),
                None,
            );
            assert_eq!(
                probe(probe_value, TypeOid::Int8, BinaryOp::Gt, true).unwrap(),
                None,
            );
        }
        // The exactness check *is* enough for an int2/int4 key (exact in `f64`) and
        // for a float key (an `f64` order on both sides), so those still accelerate.
        assert_eq!(
            probe(Value::Float8(3.0), TypeOid::Int4, BinaryOp::Eq, true).unwrap(),
            Some(Value::Int4(3)),
        );
        assert_eq!(
            probe(Value::Int4(3), TypeOid::Float8, BinaryOp::Eq, true).unwrap(),
            Some(Value::Float8(3.0)),
        );
        // A NUMERIC key against an *integer* probe compares exactly on both sides —
        // the cross-type-literal acceleration this gate must not swallow.
        assert_eq!(
            probe(Value::Int8(500), TypeOid::Numeric, BinaryOp::Ge, true).unwrap(),
            Some(Value::Numeric(PgNumeric::from_i128(500))),
        );
    }
}

#[cfg(test)]
mod virtual_scan_tests {
    use super::execute_virtual_scan;
    use crate::catalog::{
        CatalogTable, CheckConstraint, Column, ColumnDefault, ConstraintKind, EnumType, ForeignKey,
        IdentityGeneration, Index, IndexKey, NumericTypmod, ReferentialAction, Schema, Sequence,
        Table, Uniqueness,
    };
    use crate::planner::{BinaryOp, Expr, Literal};
    use crate::types::TypeOid;

    fn col(name: &str, type_oid: TypeOid, position: i16, nullable: bool) -> Column {
        Column {
            name: name.to_string(),
            type_oid,
            nullable,
            position,
            default: None,
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            temporal_precision: None,
            identity: None,
        }
    }

    /// A catalog exercising every shape the row producers branch on: an identity column
    /// (both generations), a SERIAL-style `nextval` default, a typmod'd `numeric` and
    /// `varchar`, an enum column, an array column, a NOT NULL column, a CHECK, a
    /// composite FK with non-default actions, a PRIMARY KEY, a composite UNIQUE, a plain
    /// index, an expression index, a partial index, an owned sequence and a bare one.
    fn rich_schema() -> Schema {
        let mut schema = Schema::new();
        schema.enums.insert(
            "mood".to_string(),
            EnumType {
                oid: 16500,
                labels: vec!["sad".to_string(), "ok".to_string()],
            },
        );

        let mut parent_id = col("id", TypeOid::Int4, 1, false);
        parent_id.identity = Some(IdentityGeneration::Always);
        parent_id.default = Some(ColumnDefault::NextVal("parent_id_seq".to_string()));
        let mut region = col("region", TypeOid::Varchar, 3, false);
        region.varchar_length = Some(8);
        schema.create_table(Table {
            name: "parent".to_string(),
            schema: "public".to_string(),
            oid: 16384,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
            columns: vec![parent_id, col("code", TypeOid::Text, 2, false), region],
        });

        let mut cid = col("cid", TypeOid::Int4, 1, false);
        cid.identity = Some(IdentityGeneration::ByDefault);
        cid.default = Some(ColumnDefault::NextVal("child_cid_seq".to_string()));
        let mut price = col("price", TypeOid::Numeric, 5, true);
        price.numeric_typmod = Some(NumericTypmod {
            precision: 10,
            scale: 2,
        });
        let mut m = col("m", TypeOid::Text, 6, true);
        m.enum_type = Some("mood".to_string());
        let mut sern = col("sern", TypeOid::Int4, 8, true);
        sern.default = Some(ColumnDefault::NextVal("child_sern_seq".to_string()));
        schema.create_table(Table {
            name: "child".to_string(),
            schema: "public".to_string(),
            oid: 16385,
            checks: vec![CheckConstraint {
                name: "child_qty_positive".to_string(),
                expr: Expr::BinaryOp {
                    left: Box::new(Expr::Column {
                        table: None,
                        name: "qty".to_string(),
                    }),
                    op: BinaryOp::Gt,
                    right: Box::new(Expr::Literal(Literal::Int(0))),
                },
            }],
            foreign_keys: vec![ForeignKey {
                name: "child_pid_fkey".to_string(),
                columns: vec!["pid".to_string()],
                parent_table: "parent".to_string(),
                parent_columns: vec!["id".to_string()],
                on_delete: ReferentialAction::Cascade,
                on_update: ReferentialAction::Restrict,
            }],
            columns: vec![
                cid,
                col("pid", TypeOid::Int4, 2, true),
                col("qty", TypeOid::Int4, 3, false),
                col("label", TypeOid::Text, 4, true),
                price,
                m,
                col("tags", TypeOid::TextArray, 7, true),
                sern,
            ],
        });

        let index =
            |name: &str, table: &str, oid: i32, key: IndexKey, uniqueness: Uniqueness| Index {
                name: name.to_string(),
                table_name: table.to_string(),
                key,
                uniqueness,
                predicate: None,
                nulls_not_distinct: false,
                oid,
            };
        schema.create_index(index(
            "parent_pkey",
            "parent",
            16390,
            IndexKey::Columns(vec!["id".to_string()]),
            Uniqueness::for_constraint(ConstraintKind::PrimaryKey),
        ));
        schema.create_index(index(
            "parent_code_region_uq",
            "parent",
            16391,
            IndexKey::Columns(vec!["code".to_string(), "region".to_string()]),
            Uniqueness::for_constraint(ConstraintKind::Unique),
        ));
        schema.create_index(index(
            "child_pid_idx",
            "child",
            16392,
            IndexKey::Columns(vec!["pid".to_string()]),
            Uniqueness::new(false, false),
        ));
        schema.create_index(index(
            "child_lower_label_idx",
            "child",
            16393,
            IndexKey::Exprs(vec![Expr::Lower(Box::new(Expr::Column {
                table: None,
                name: "label".to_string(),
            }))]),
            Uniqueness::new(false, false),
        ));
        let mut partial = index(
            "child_partial_idx",
            "child",
            16394,
            IndexKey::Columns(vec!["qty".to_string()]),
            Uniqueness::new(false, false),
        );
        partial.predicate = Some(Expr::BinaryOp {
            left: Box::new(Expr::Column {
                table: None,
                name: "qty".to_string(),
            }),
            op: BinaryOp::Gt,
            right: Box::new(Expr::Literal(Literal::Int(10))),
        });
        schema.create_index(partial);

        schema.sequences.insert(
            "parent_id_seq".to_string(),
            Sequence::owned_by(1, "parent", "id"),
        );
        schema.sequences.insert(
            "child_cid_seq".to_string(),
            Sequence::owned_by(1, "child", "cid"),
        );
        schema.sequences.insert(
            "child_sern_seq".to_string(),
            Sequence::owned_by(1, "child", "sern"),
        );
        schema
            .sequences
            .insert("plain_seq".to_string(), Sequence::new(5));
        schema
    }

    /// Every catalog relation's rows must be as wide as its declared column list.
    ///
    /// This is the `ResultShape` two-producer class in its catalog form: the descriptor
    /// comes from `CatalogTable::columns()` and the values from a `PgCatalog` /
    /// `InformationSchema` method, and **nothing in the engine compares them**. A row
    /// short of the list makes every column past the gap read SQL NULL — silently — and
    /// for a relation marked `columns_complete()` the plan-time binding gate would be
    /// binding a name its own row producer never fills.
    ///
    /// Run over a catalog carrying every shape the producers branch on, so an arm that
    /// pushes the wrong number of values on one path (an FK row, an expression index, an
    /// enum column) is reached rather than merely compiled.
    #[test]
    fn every_catalog_relation_emits_exactly_its_declared_columns() {
        let schema = rich_schema();
        for table in CatalogTable::ALL {
            let declared = table.columns();
            let (rows, columns) = execute_virtual_scan(table, &schema).unwrap();
            assert_eq!(
                columns.len(),
                declared.len(),
                "{}: the executor's descriptor must be `columns()`",
                table.name()
            );
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(
                    row.len(),
                    declared.len(),
                    "{} row {i} has {} values for {} declared columns",
                    table.name(),
                    row.len(),
                    declared.len()
                );
            }
        }
    }

    /// A relation marked `columns_complete()` must actually **produce rows**, or the
    /// exhaustive binding gate it turns on is refusing names over a relation nobody
    /// filled — which is how `pg_constraint` and `pg_index` came to accept any invented
    /// column as a zero-row `text` in the first place.
    #[test]
    fn every_complete_relation_produces_rows_for_a_populated_catalog() {
        let schema = rich_schema();
        for table in CatalogTable::ALL {
            if !table.columns_complete() {
                continue;
            }
            let (rows, _) = execute_virtual_scan(table, &schema).unwrap();
            assert!(
                !rows.is_empty(),
                "{} is marked complete but produced no rows over a catalog carrying \
                 tables, indexes, constraints, sequences and an enum",
                table.name()
            );
        }
    }
}

/// The un-accelerated key-scan fallback's **enum arm**, which no plan can reach.
///
/// `single_column_index_for` declines an enum key column, so nothing plans an
/// `IndexScan`/`IndexRangeScan` over one and [`scan_filtered_by_key`] never sees an
/// enum in production. That is exactly why these are unit tests over a row stub rather
/// than fixtures or engine tests: the arm is **unexecuted** by every other guard in the
/// repo, and an unexecuted arm is not a tested one. What they pin is that the fallback
/// is correct *without* the decline — the filter and the sort both read the member
/// order — so narrowing the decline later cannot silently reopen a mis-ordered read on
/// the accelerated profile only.
// A test harness: `panic!` on a value shape the fixtures below cannot produce, so a
// broken stub surfaces here rather than as a confusing assertion diff.
#[allow(clippy::panic)]
#[cfg(all(test, feature = "index_acceleration"))]
mod enum_key_fallback_tests {
    use super::{enum_key_order, scan_filtered_by_key};
    use crate::catalog::{Column, EnumType, Schema, Table};
    use crate::planner::BinaryOp;
    use crate::types::{TypeOid, Value};
    use pglet_core::{Error, IndexSpec, NoopStorage, RowId, Storage};

    /// A `Storage` that answers `scan` from a fixed row list and delegates the rest to
    /// [`NoopStorage`] — the read-only surface [`scan_filtered_by_key`] uses.
    struct Rows(Vec<Vec<Value>>);

    impl Storage for Rows {
        fn scan(&self, _table: &str) -> Result<Vec<(RowId, Vec<Value>)>, Error> {
            Ok(self
                .0
                .iter()
                .enumerate()
                .map(|(i, r)| (i as RowId, r.clone()))
                .collect())
        }
        fn create_table(&mut self, name: &str) -> Result<(), Error> {
            NoopStorage.create_table(name)
        }
        fn drop_table(&mut self, name: &str) -> Result<(), Error> {
            NoopStorage.drop_table(name)
        }
        fn insert(&mut self, table: &str, row: Vec<Value>) -> Result<RowId, Error> {
            NoopStorage.insert(table, row)
        }
        fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
            NoopStorage.get(table, id)
        }
        fn delete(&mut self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
            NoopStorage.delete(table, id)
        }
        fn update(
            &mut self,
            table: &str,
            id: RowId,
            new_row: Vec<Value>,
        ) -> Result<Option<Vec<Value>>, Error> {
            NoopStorage.update(table, id, new_row)
        }
        fn add_column(&mut self, table: &str, default: Value) -> Result<(), Error> {
            NoopStorage.add_column(table, default)
        }
        fn drop_column(&mut self, table: &str, column_index: usize) -> Result<(), Error> {
            NoopStorage.drop_column(table, column_index)
        }
        fn create_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
            NoopStorage.create_index(spec)
        }
        fn reattach_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
            NoopStorage.reattach_index(spec)
        }
        fn rebuild_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
            NoopStorage.rebuild_index(spec)
        }
        fn drop_index(&mut self, index_id: u32) -> Result<(), Error> {
            NoopStorage.drop_index(index_id)
        }
        fn index_lookup(&self, index_id: u32, values: &[Value]) -> Result<Vec<RowId>, Error> {
            NoopStorage.index_lookup(index_id, values)
        }
        fn index_range_lookup(
            &self,
            index_id: u32,
            lower: Option<(Value, bool)>,
            upper: Option<(Value, bool)>,
            ascending: bool,
            limit: Option<usize>,
        ) -> Result<Vec<RowId>, Error> {
            NoopStorage.index_range_lookup(index_id, lower, upper, ascending, limit)
        }
        fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>, Error> {
            NoopStorage.gin_lookup(index_id, keys)
        }
        fn savepoint(&mut self) -> Result<(), Error> {
            NoopStorage.savepoint()
        }
        fn rollback_to_savepoint(&mut self) -> Result<(), Error> {
            NoopStorage.rollback_to_savepoint()
        }
        fn release_savepoint(&mut self) -> Result<(), Error> {
            NoopStorage.release_savepoint()
        }
    }

    /// `mood AS ENUM ('sad','ok','happy')` — declared order is the reverse of the
    /// lexical one, so the two rules disagree on every pair.
    fn mood_schema() -> Schema {
        let mut schema = Schema::new();
        schema.enums.insert(
            "mood".to_string(),
            EnumType {
                oid: 16500,
                labels: vec!["sad".into(), "ok".into(), "happy".into()],
            },
        );
        schema
    }

    fn mood_table() -> Table {
        Table {
            name: "t".to_string(),
            schema: "public".to_string(),
            oid: 16384,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
            columns: vec![Column {
                name: "m".to_string(),
                type_oid: TypeOid::Text,
                nullable: false,
                position: 1,
                default: None,
                enum_type: Some("mood".to_string()),
                numeric_typmod: None,
                varchar_length: None,
                temporal_precision: None,
                identity: None,
            }],
        }
    }

    fn rows() -> Rows {
        Rows(vec![
            vec![Value::Text("sad".into())],
            vec![Value::Text("ok".into())],
            vec![Value::Text("happy".into())],
        ])
    }

    fn labels(rows: &[Vec<Value>]) -> Vec<String> {
        rows.iter()
            .map(|r| match &r[0] {
                Value::Text(s) => s.clone(),
                other => panic!("expected text, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn the_order_is_derived_from_the_column_not_assumed_absent() {
        let schema = mood_schema();
        let table = mood_table();
        assert_eq!(
            enum_key_order(&schema, &table, "m"),
            Some(&["sad".to_string(), "ok".to_string(), "happy".to_string()][..]),
        );
        // A column with no enum, and a name no column carries, both decline.
        assert_eq!(enum_key_order(&schema, &table, "nope"), None);
    }

    #[test]
    fn the_fallbacks_filter_reads_the_member_order() {
        let got = scan_filtered_by_key(
            "t",
            "m",
            &mood_table(),
            &[(BinaryOp::Gt, Value::Text("sad".into()))],
            false,
            true,
            None,
            &mood_schema(),
            &rows(),
        )
        .expect("scan");
        // By member position `sad` is first, so `ok` and `happy` survive. By label
        // bytes both sort *below* `sad` and the answer is empty.
        assert_eq!(labels(&got), vec!["ok".to_string(), "happy".to_string()]);
    }

    #[test]
    fn the_fallbacks_sort_reads_the_member_order() {
        let got = scan_filtered_by_key(
            "t",
            "m",
            &mood_table(),
            &[],
            true,
            true,
            None,
            &mood_schema(),
            &rows(),
        )
        .expect("scan");
        assert_eq!(
            labels(&got),
            vec!["sad".to_string(), "ok".to_string(), "happy".to_string()],
        );
    }

    /// The control: over a catalog that declares **no** enum for the key column, both
    /// halves fall back to the label bytes — which is what the two tests above would
    /// report if the derivation stopped happening. Without this the pair could pass over
    /// a comparator that ignored the order it was handed.
    #[test]
    fn without_an_enum_column_both_halves_are_label_bytes() {
        let plain = Schema::new();
        let filtered = scan_filtered_by_key(
            "t",
            "m",
            &mood_table(),
            &[(BinaryOp::Gt, Value::Text("sad".into()))],
            false,
            true,
            None,
            &plain,
            &rows(),
        )
        .expect("scan");
        assert!(filtered.is_empty());
        let sorted = scan_filtered_by_key(
            "t",
            "m",
            &mood_table(),
            &[],
            true,
            true,
            None,
            &plain,
            &rows(),
        )
        .expect("scan");
        assert_eq!(
            labels(&sorted),
            vec!["happy".to_string(), "ok".to_string(), "sad".to_string()],
        );
    }
}

/// [`probe_dml_targets`]'s ordering, against an index whose answer is **not** in row
/// order. No backend returns one today (each answers a key's ids in `RowId` order), which
/// is why a stub stands in: the write order must not rest on that.
#[cfg(all(test, feature = "index_acceleration"))]
mod dml_probe_order_tests {
    use super::probe_dml_targets;
    use crate::catalog::{Column, Table};
    use crate::planner::{BinaryOp, Expr, Literal, Plan};
    use crate::types::{TypeOid, Value};
    use pglet_core::{Error, IndexSpec, NoopStorage, RowId, Storage};

    /// A `Storage` whose `index_lookup` answers a fixed id list, in the order given.
    struct Ids(Vec<RowId>);

    impl Storage for Ids {
        fn index_lookup(&self, _index_id: u32, _values: &[Value]) -> Result<Vec<RowId>, Error> {
            Ok(self.0.clone())
        }
        fn scan(&self, table: &str) -> Result<Vec<(RowId, Vec<Value>)>, Error> {
            NoopStorage.scan(table)
        }
        fn create_table(&mut self, name: &str) -> Result<(), Error> {
            NoopStorage.create_table(name)
        }
        fn drop_table(&mut self, name: &str) -> Result<(), Error> {
            NoopStorage.drop_table(name)
        }
        fn insert(&mut self, table: &str, row: Vec<Value>) -> Result<RowId, Error> {
            NoopStorage.insert(table, row)
        }
        fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
            NoopStorage.get(table, id)
        }
        fn delete(&mut self, table: &str, id: RowId) -> Result<Option<Vec<Value>>, Error> {
            NoopStorage.delete(table, id)
        }
        fn update(
            &mut self,
            table: &str,
            id: RowId,
            new_row: Vec<Value>,
        ) -> Result<Option<Vec<Value>>, Error> {
            NoopStorage.update(table, id, new_row)
        }
        fn add_column(&mut self, table: &str, default: Value) -> Result<(), Error> {
            NoopStorage.add_column(table, default)
        }
        fn drop_column(&mut self, table: &str, column_index: usize) -> Result<(), Error> {
            NoopStorage.drop_column(table, column_index)
        }
        fn create_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
            NoopStorage.create_index(spec)
        }
        fn reattach_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
            NoopStorage.reattach_index(spec)
        }
        fn rebuild_index(&mut self, spec: IndexSpec) -> Result<(), Error> {
            NoopStorage.rebuild_index(spec)
        }
        fn drop_index(&mut self, index_id: u32) -> Result<(), Error> {
            NoopStorage.drop_index(index_id)
        }
        fn index_range_lookup(
            &self,
            index_id: u32,
            lower: Option<(Value, bool)>,
            upper: Option<(Value, bool)>,
            ascending: bool,
            limit: Option<usize>,
        ) -> Result<Vec<RowId>, Error> {
            NoopStorage.index_range_lookup(index_id, lower, upper, ascending, limit)
        }
        fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>, Error> {
            NoopStorage.gin_lookup(index_id, keys)
        }
        fn savepoint(&mut self) -> Result<(), Error> {
            NoopStorage.savepoint()
        }
        fn rollback_to_savepoint(&mut self) -> Result<(), Error> {
            NoopStorage.rollback_to_savepoint()
        }
        fn release_savepoint(&mut self) -> Result<(), Error> {
            NoopStorage.release_savepoint()
        }
    }

    #[test]
    fn the_candidates_come_back_in_row_id_order_whatever_the_index_answers() {
        let table = Table {
            name: "t".to_string(),
            schema: "public".to_string(),
            oid: 16384,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
            columns: vec![Column {
                name: "k".to_string(),
                type_oid: TypeOid::Int4,
                nullable: true,
                position: 1,
                default: None,
                enum_type: None,
                numeric_typmod: None,
                varchar_length: None,
                temporal_precision: None,
                identity: None,
            }],
        };
        let key = Expr::Literal(Literal::Int(1));
        let filter = Expr::BinaryOp {
            left: Box::new(Expr::Column {
                table: None,
                name: "k".to_string(),
            }),
            op: BinaryOp::Eq,
            right: Box::new(key.clone()),
        };
        let probe = Plan::IndexScan {
            table: "t".to_string(),
            alias: None,
            index_oid: 1,
            key_column: "k".to_string(),
            key_value: Box::new(key),
            column_first: true,
        };
        let ids = probe_dml_targets(
            &probe,
            &filter,
            &table,
            &Ids(vec![9, 2, 5, 2]),
            crate::executor::expr::EvalEnv::EMPTY,
        )
        .expect("probe");
        assert_eq!(ids, Some(vec![2, 5, 9]));
    }
}
