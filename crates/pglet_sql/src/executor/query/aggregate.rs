//! Aggregate (`GROUP BY` / `HAVING` / aggregate-function) execution.

use std::cmp::Ordering;

use crate::catalog::Schema;
use crate::planner::{
    AggregateFunc, ArgCertainty, BinaryOp, BoolContext, Expr, Literal, aggregate_legality,
    aggregate_type_of,
};
use crate::types::{
    PgInterval, PgNumeric, TypeOid, Value, opt_type_oid_display_name, type_oid_display_name,
};
use crate::{Error, Result};

use super::super::Storage;
use super::super::coerce::coerce_value;
use super::super::expr::{
    EvalEnv, SimpleColumns, SubqueryExec, eval_binary_op, eval_expr_full, float4_add, float8_add,
    interval_add, interval_scale, literal_to_value, predicate_holds, value_as_i64,
    value_to_f64_opt, values_compare, values_compare_ordered,
};
use super::super::helpers::{
    ProjectionScope, certain_expr_type_oid, column_enum_orders, expr_column_descs, expr_column_info,
};
use super::super::types::ColumnDesc;
use super::execute_plan_node;

/// Execute a `Plan::Aggregate` — group the input rows and compute one output row
/// per group (applying `HAVING`).
///
/// Per group, the aggregates the group keeps ([`Collector`]) are computed
/// first, and then each expression above them — the `HAVING` predicate, each select-list
/// item and `ORDER BY` junk column — is evaluated by the **ordinary scalar walk**, which
/// reads each aggregate's value through the [`SubqueryExec::aggregate`] lookup this lends
/// it. The lookup is keyed by the `Expr::Aggregate` node's identity, so two textually equal
/// aggregates are two entries and an inner query's aggregate is never one of this
/// group's. The plan is never rewritten to carry a value.
///
/// The `HAVING` predicate's aggregates are computed and the predicate evaluated before the
/// select list's aggregates are computed, so a group `HAVING` discards computes none of
/// the select list's.
pub(super) fn execute_aggregate<S: Storage>(
    input: &crate::planner::Plan,
    group_by: &[Expr],
    (exprs, enclosing): (&[Expr], &crate::planner::EnclosingColumns),
    having: Option<&Expr>,
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let (rows, input_columns) = execute_plan_node(input, schema, storage, env)?;
    let col_names: Vec<&str> = input_columns.iter().map(|c| c.name.as_str()).collect();
    // Declared input types, so a grouped/aggregated `||` or `::text` over a
    // `timestamptz` keeps its `+00` offset (see `ColumnResolver::column_type`).
    let col_types: Vec<TypeOid> = input_columns.iter().map(|c| c.type_oid).collect();
    // ...and their enum member orders, so a grouped `HAVING m > 'sad'` and a
    // `min(m)`/`max(m)` order by member position (see `ColumnResolver::column_enum_order`).
    let col_enums = column_enum_orders(&input_columns, &schema.enums);

    // Every expression this node evaluates — a `GROUP BY` key, an aggregate's argument,
    // `HAVING`, the select list — reads the input's names through this one resolver, told
    // its level's FROM names so a qualifier the level owns binds here and never an
    // enclosing frame of the same name.
    let level = super::scan::level_scope(input, &input_columns, schema);
    let resolver = SimpleColumns::new(&col_names)
        .with_types(&col_types)
        .with_enum_orders(&col_enums)
        .within_level(&level);
    // The output descriptors, the aggregate operand check and `MIN`/`MAX`'s enum order
    // read a column through the same resolver, so each reference names the column its
    // value comes from — an owned qualifier included, which a scope rebuilt from the bare
    // names would bind by its `"t.name"` text instead.
    let enclosing = enclosing.entries_copy();
    let scope = ProjectionScope::new(&input_columns, &col_names, None)
        .with_schema(schema)
        .with_resolver(&resolver)
        .with_enclosing(&enclosing);
    let output_columns = expr_column_descs(exprs, scope);

    // The aggregates each phase computes, `HAVING`'s first: the lookup's slots are their
    // positions in that order. Which aggregates are kept depends on no row, so it is
    // decided once for the statement — and so is each aggregate's operand type, kept or
    // not, which the walk refuses here, ahead of every group. The select list is walked
    // before `HAVING`, the order 18.6 analyses them in, so a refusal in each is raised in
    // that order.
    let mut collector = Collector::new(scope);
    for expr in exprs {
        collector.collect(expr)?;
    }
    let mut select_kept = std::mem::take(&mut collector.kept);
    if let Some(having_expr) = having {
        collector.collect(having_expr)?;
    }
    let mut kept = collector.kept;
    let having_count = kept.len();
    kept.append(&mut select_kept);
    let slots = AggregateSlots::new(&kept);

    // One group of every row without a GROUP BY — even of no rows, which still answers
    // one row (`count(*)` = 0) — and otherwise one per set of equal key values.
    let groups = if group_by.is_empty() {
        vec![rows]
    } else {
        group_rows(rows, group_by.len(), |row| {
            group_by
                .iter()
                .map(|expr| eval_expr_full(expr, &resolver, row, SubqueryExec::NONE, env))
                .collect()
        })?
    };

    // Compute output rows (one per group)
    let mut output_rows = Vec::new();
    let mut values: Vec<Value> = Vec::with_capacity(kept.len());
    for group_rows in groups {
        // A non-aggregate expression reads the group's first row.
        let first_row = group_rows.first().map_or(&[][..], Vec::as_slice);
        let compute = |kept: &KeptAggregate<'_>| {
            compute_group_aggregate(kept, &resolver, &group_rows, scope, env)
        };
        values.clear();
        for node in &kept[..having_count] {
            values.push(compute(node)?);
        }
        if let Some(having_expr) = having {
            let value_of = |node: &Expr| slots.value(node, &values);
            let sub = SubqueryExec {
                aggregate: Some(&value_of),
                ..SubqueryExec::NONE
            };
            let having_result = eval_expr_full(having_expr, &resolver, first_row, sub, env)?;
            // Skip this group if HAVING condition is not true
            if !predicate_holds(&having_result, BoolContext::Having)? {
                continue;
            }
        }
        for node in &kept[having_count..] {
            values.push(compute(node)?);
        }
        let value_of = |node: &Expr| slots.value(node, &values);
        let sub = SubqueryExec {
            aggregate: Some(&value_of),
            ..SubqueryExec::NONE
        };
        let output_row = exprs
            .iter()
            .map(|expr| eval_expr_full(expr, &resolver, first_row, sub, env))
            .collect::<Result<Vec<_>>>()?;
        output_rows.push(output_row);
    }

    Ok((output_rows, output_columns))
}

/// Each kept aggregate node's slot in a group's value list, found by the node's address.
///
/// Identity rather than structure is the key: `sum(a)` written twice is two nodes and two
/// slots, and a node the grouping did not collect — an inner query's, or one under a
/// branch the [`Collector`] dropped — has no slot, so the scalar walk's
/// `Expr::Aggregate` arm refuses it rather than reading another node's value.
struct AggregateSlots {
    /// `(address, slot)`, ascending by address.
    by_address: Vec<(usize, usize)>,
}

impl AggregateSlots {
    fn new(kept: &[KeptAggregate<'_>]) -> Self {
        let mut by_address: Vec<(usize, usize)> = Vec::with_capacity(kept.len());
        for (slot, kept) in kept.iter().enumerate() {
            let address = std::ptr::from_ref(kept.node).addr();
            let at = by_address.partition_point(|&(a, _)| a < address);
            by_address.insert(at, (address, slot));
        }
        Self { by_address }
    }

    /// `node`'s value among `values` (the group's, in slot order, as far as they are
    /// computed), or `None` where `node` is not a kept aggregate or its phase has not
    /// computed it yet.
    fn value(&self, node: &Expr, values: &[Value]) -> Option<Value> {
        let address = std::ptr::from_ref(node).addr();
        let at = self
            .by_address
            .binary_search_by_key(&address, |&(a, _)| a)
            .ok()?;
        let &(_, slot) = self.by_address.get(at)?;
        values.get(slot).cloned()
    }
}

/// Collects the aggregates a group computes from the expressions it walks, in walk
/// order: every `Expr::Aggregate` reached, but none in a branch a **certain decider**
/// drops.
///
/// 18.6 computes every aggregate that survives its planner's constant folding for every
/// group, whether or not the expression above it reads the value — so `CASE WHEN
/// count(*) > 0 THEN max(a) ELSE sum(1/b) END` raises the division's `22012` over a row
/// with `b = 0`, while `CASE WHEN false THEN sum(1/b) END` answers `NULL`, the aggregate
/// having been dropped with its branch. Which deciders that planner folds is a property
/// of every function, operator, cast and type under them, and a decider taken for a
/// constant here that is none there drops an aggregate 18.6 computes: an answer where
/// 18.6 raises. So a branch is dropped only under a decider that is a constant **by its
/// shape alone** ([`Seen::Certain`]), and every other aggregate is kept — a fold of
/// 18.6's that this list does not hold costs a refusal where 18.6 answers, a failing
/// aggregate in the branch it dropped being computed here (`EDGES.md` row 89).
///
/// The certain deciders, each a constant to 18.6's parser or folded by
/// `eval_const_expressions` whatever the session:
///
/// - the literals `true`, `false` and `NULL`;
/// - a comparison (`=`, `<>`, `<`, `<=`, `>`, `>=`) of two integer or `numeric`
///   literals ([`literals_compare`]);
/// - `NOT`, `AND`, `OR`, `IS [NOT] NULL` and `IS [NOT] TRUE | FALSE | UNKNOWN` over
///   certain deciders.
///
/// What one drops is a branch the scalar walk does not evaluate either:
///
/// - an `AND` / `OR` whose **left** operand is certain and decides it drops its right
///   operand;
/// - a `CASE` drops an arm whose condition is certainly not true, and every arm and the
///   `ELSE` after one whose condition is certainly true — a simple `CASE` likewise,
///   where its operand and the arm's value are both integer or `numeric` literals;
/// - a `COALESCE` drops every argument after an integer, `numeric` or boolean literal
///   ([`is_non_null_literal`]).
///
/// Nothing is evaluated here but those literals: a constant is evaluated by the scalar
/// walk where it is reached, as in a query that groups nothing. An aggregate's own
/// argument is not walked — it is evaluated per row by the aggregate, and a nested
/// aggregate is refused at plan time. A window function and a sub-plan are leaves, as
/// in `contains_aggregate`.
///
/// **Dropping an aggregate drops its computation, not its operand's type check.** 18.6
/// resolves an aggregate against its argument's type in parse analysis, before it folds
/// anything, so `true OR sum(text_col) > 0` is `42883` there whatever the rows. A dropped
/// branch is therefore still walked, collecting nothing, and an aggregate the walk meets
/// that has no form for its operand's type — kept or dropped, and whether or not any
/// group is ever computed — is refused here, where that type needs no evaluation to know
/// ([`Self::operand_type`]).
struct Collector<'a, 'e> {
    scope: ProjectionScope<'a>,
    /// The aggregates collected so far.
    kept: Vec<KeptAggregate<'e>>,
    /// How many dropped branches the walk stands inside: above zero, it collects nothing.
    dropped: usize,
    /// Why the walk answered [`Seen::Failed`].
    failure: Option<Error>,
    /// How many nodes the walk visited, counted for the linearity test.
    #[cfg(test)]
    visited: usize,
}

/// What [`Collector::walk`] found a node to be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// Not a certain decider: its aggregates are collected.
    Kept,
    /// A certain decider and its value, `None` being `NULL`. The walk collected no
    /// aggregate under it.
    Certain(Option<bool>),
    /// The walk failed, and [`Collector::failure`] says why.
    Failed,
}

impl Seen {
    /// What a node that is no decider itself is, over an operand walked as `self`.
    const fn kept(self) -> Self {
        match self {
            Self::Failed => Self::Failed,
            Self::Kept | Self::Certain(_) => Self::Kept,
        }
    }

    /// A node that is a certain decider over a certain operand, its value `of` the
    /// operand's.
    fn map(self, of: impl FnOnce(Option<bool>) -> Option<bool>) -> Self {
        match self {
            Self::Certain(value) => Self::Certain(of(value)),
            other => other,
        }
    }
}

impl<'a, 'e> Collector<'a, 'e> {
    const fn new(scope: ProjectionScope<'a>) -> Self {
        Self {
            scope,
            kept: Vec::new(),
            dropped: 0,
            failure: None,
            #[cfg(test)]
            visited: 0,
        }
    }

    /// Walk `expr`, collecting its kept aggregates.
    ///
    /// # Errors
    ///
    /// The `42883` of an aggregate with no form for its operand's type, or the depth
    /// bound's `54001`.
    fn collect(&mut self, expr: &'e Expr) -> Result<()> {
        self.walk(expr);
        self.failure.take().map_or(Ok(()), Err)
    }

    /// Walk a branch a certain decider drops: its aggregates' operand types are checked,
    /// and none is collected.
    #[inline]
    fn walk_dropped(&mut self, expr: &'e Expr) -> Seen {
        self.dropped += 1;
        let seen = self.walk(expr);
        self.dropped -= 1;
        seen
    }

    /// The type of an aggregate's operand `arg`, where it is certain and nothing is
    /// evaluated between it and the aggregate's verdict on 18.6.
    ///
    /// A column's declared type and a literal's are ([`certain_expr_type_oid`]); a bare
    /// `$N` and a computed argument have none. A cast's is its target — but a cast can itself be refused, which 18.6 raises ahead
    /// of the aggregate's verdict and pglet raises as it evaluates the argument per row.
    /// So a **kept** aggregate over a cast waits for `compute_aggregate`'s check, after
    /// its argument has been evaluated, and a **dropped** one, whose argument nothing
    /// evaluates, takes the verdict over the cast's target here.
    fn operand_type(&self, arg: &Expr) -> Option<TypeOid> {
        if self.dropped == 0 && matches!(arg, Expr::Cast { .. } | Expr::CastTypmod { .. }) {
            return None;
        }
        certain_expr_type_oid(arg, self.scope)
    }

    /// One node of the walk. The chain-forming nodes recurse from this frame, so a chain
    /// costs it alone a level (`crate::depth`); everything else is [`Self::walk_rest`]'s.
    fn walk(&mut self, expr: &'e Expr) -> Seen {
        if let Err(e) = crate::depth::check_stack() {
            self.failure = Some(e);
            return Seen::Failed;
        }
        #[cfg(test)]
        {
            self.visited += 1;
        }
        match expr {
            Expr::BinaryOp { left, op, right } => {
                let is_or = match op.connective() {
                    BinaryOp::And => false,
                    BinaryOp::Or => true,
                    _ => {
                        if let Some(holds) = literals_compare(left, *op, right) {
                            return Seen::Certain(Some(holds));
                        }
                        if self.walk(left) == Seen::Failed {
                            return Seen::Failed;
                        }
                        return self.walk(right).kept();
                    }
                };
                // `false AND …` and `true OR …` are the connective's value on their own,
                // and the right operand is dropped.
                let l = self.walk(left);
                if l == Seen::Failed {
                    return l;
                }
                if l == Seen::Certain(Some(is_or)) {
                    return match self.walk_dropped(right) {
                        Seen::Failed => Seen::Failed,
                        _ => l,
                    };
                }
                match (l, self.walk(right)) {
                    (_, Seen::Failed) => Seen::Failed,
                    (Seen::Certain(l), Seen::Certain(r)) => Seen::Certain(match (l, r) {
                        (_, Some(r)) if r == is_or => Some(is_or),
                        (Some(_), Some(r)) => Some(r),
                        _ => None,
                    }),
                    _ => Seen::Kept,
                }
            }
            Expr::Not(inner) => self.walk(inner).map(|value| value.map(|holds| !holds)),
            Expr::IsNull(inner) => self.walk(inner).map(|value| Some(value.is_none())),
            Expr::IsNotNull(inner) => self.walk(inner).map(|value| Some(value.is_some())),
            Expr::BooleanTest { expr: inner, test } => {
                self.walk(inner).map(|value| Some(test.apply(value)))
            }
            Expr::Negate(inner)
            | Expr::UnaryPlus(inner)
            | Expr::Cast { expr: inner, .. }
            | Expr::CastTypmod { expr: inner, .. } => self.walk(inner).kept(),
            Expr::InList {
                expr: operand,
                list,
                ..
            } => {
                let mut seen = self.walk(operand).kept();
                for item in list {
                    if seen == Seen::Failed {
                        break;
                    }
                    seen = self.walk(item).kept();
                }
                seen
            }
            _ => self.walk_rest(expr),
        }
    }

    /// [`Self::walk`]'s nodes that form no operator chain.
    #[inline(never)]
    fn walk_rest(&mut self, expr: &'e Expr) -> Seen {
        match expr {
            Expr::Literal(Literal::Bool(holds)) => Seen::Certain(Some(*holds)),
            Expr::Literal(Literal::Null) => Seen::Certain(None),
            Expr::Aggregate {
                func,
                arg,
                distinct,
                certainty,
            } => {
                if let Err(e) = aggregate_legality(*func, self.operand_type(arg)) {
                    self.failure = Some(e);
                    return Seen::Failed;
                }
                if self.dropped == 0 {
                    self.kept.push(KeptAggregate {
                        node: expr,
                        func: *func,
                        arg,
                        distinct: *distinct,
                        certainty: *certainty,
                    });
                }
                Seen::Kept
            }
            Expr::Window { .. } => Seen::Kept,
            Expr::Case {
                operand,
                when_clauses,
                else_result,
            } => self.case(operand.as_deref(), when_clauses, else_result.as_deref()),
            // The arguments after one that is certainly not `NULL` are dropped.
            Expr::Coalesce(args) => {
                let mut ended = false;
                for arg in args {
                    let seen = if ended {
                        self.walk_dropped(arg)
                    } else {
                        self.walk(arg)
                    };
                    if seen == Seen::Failed {
                        return Seen::Failed;
                    }
                    ended = ended || is_non_null_literal(arg);
                }
                Seen::Kept
            }
            _ => {
                let mut seen = Seen::Kept;
                expr.for_each_child(|child| {
                    if seen != Seen::Failed {
                        seen = self.walk(child).kept();
                    }
                });
                seen
            }
        }
    }

    /// A `CASE`: an arm a certain condition drops is dropped, and so is what follows an
    /// arm a certain condition takes.
    fn case(
        &mut self,
        operand: Option<&'e Expr>,
        when_clauses: &'e [(Expr, Expr)],
        else_result: Option<&'e Expr>,
    ) -> Seen {
        if let Some(operand) = operand
            && self.walk(operand) == Seen::Failed
        {
            return Seen::Failed;
        }
        // Whether an arm before this one is certainly taken: 18.6 makes that arm's result
        // the `CASE`'s default and drops what follows.
        let mut decided = false;
        for (condition, result) in when_clauses {
            if decided {
                if self.walk_dropped(condition) == Seen::Failed
                    || self.walk_dropped(result) == Seen::Failed
                {
                    return Seen::Failed;
                }
                continue;
            }
            // Whether the arm is certainly taken, certainly not, or neither.
            let taken = match operand {
                None => match self.walk(condition) {
                    Seen::Failed => return Seen::Failed,
                    Seen::Certain(value) => Some(value == Some(true)),
                    Seen::Kept => None,
                },
                Some(operand) => {
                    let taken = literals_compare(operand, BinaryOp::Eq, condition);
                    if taken.is_none() && self.walk(condition) == Seen::Failed {
                        return Seen::Failed;
                    }
                    taken
                }
            };
            let seen = if taken == Some(false) {
                self.walk_dropped(result)
            } else {
                self.walk(result)
            };
            if seen == Seen::Failed {
                return Seen::Failed;
            }
            decided = taken == Some(true);
        }
        match else_result {
            Some(else_result) if decided => self.walk_dropped(else_result).kept(),
            Some(else_result) => self.walk(else_result).kept(),
            None => Seen::Kept,
        }
    }
}

/// Whether `left op right` holds, where `op` is a comparison and both operands are
/// integer or `numeric` literals — and `None` for any other shape.
///
/// 18.6 folds such a comparison whatever the session: each operator is immutable, and
/// so is the conversion between the integer widths and `numeric` that brings two
/// literals of different types to one. A string literal is no member (its order is the
/// collation's there, `EDGES.md` row 43), and neither is a bound `$N`, which 18.6 folds
/// under a custom plan alone.
fn literals_compare(left: &Expr, op: BinaryOp, right: &Expr) -> Option<bool> {
    const fn is_number(literal: &Literal) -> bool {
        matches!(literal, Literal::Int(_) | Literal::Numeric(_))
    }
    let (Expr::Literal(left), Expr::Literal(right)) = (left, right) else {
        return None;
    };
    if !is_number(left)
        || !is_number(right)
        || !matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        )
    {
        return None;
    }
    match eval_binary_op(op, &literal_to_value(left), &literal_to_value(right)) {
        Ok(Value::Bool(holds)) => Some(holds),
        _ => None,
    }
}

/// Whether `expr` is a `COALESCE` argument that is certainly not `NULL`, to 18.6's
/// planner as to the scalar walk: an integer, `numeric` or boolean literal, bare or
/// under a cast to an integer type, `numeric` or a float — the cast the coercion pass
/// brings such a literal to the `COALESCE`'s type by, which is immutable on 18.6.
const fn is_non_null_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(literal) => matches!(
            literal,
            Literal::Bool(_) | Literal::Int(_) | Literal::Numeric(_)
        ),
        Expr::Cast {
            expr: inner,
            target_type,
        } => {
            matches!(
                &**inner,
                Expr::Literal(Literal::Int(_) | Literal::Numeric(_))
            ) && matches!(
                target_type,
                TypeOid::Int2
                    | TypeOid::Int4
                    | TypeOid::Int8
                    | TypeOid::Numeric
                    | TypeOid::Float4
                    | TypeOid::Float8
            )
        }
        _ => false,
    }
}

/// Partition `rows` into `GROUP BY` groups, each in input order, the groups in the
/// order their first rows arrive.
///
/// Rows group exactly where their key values (`key_of`, `width` of them) are equal
/// under the relation `SELECT DISTINCT` dedups by, `rows_distinct_equal` — 18.6 groups
/// and dedups by one equality, the type's default one, so `{"a":1.0}` and `{"a":1}`,
/// `ARRAY[1.0]` and `ARRAY[1.00]`, `ARRAY['-0'::float8]` and `ARRAY[0::float8]` are
/// one group, and two NULLs are one group. A row joins the first group whose key
/// values it equals. Keyed by [`super::dedup::DistinctKeys`] while that is certain to
/// be the relation, and pairwise against each group's first key values from the first
/// row where it is not.
///
/// A group's first row is the one a non-aggregate expression over it reads, so
/// `SELECT x … GROUP BY x` answers the spelling met first. That is 18.6's answer under
/// a hashed grouping; a sorted one answers the first of the group in its sort, which
/// is the first met where the sort keeps ties in input order — as its insertion sort,
/// below seven rows, does.
fn group_rows(
    rows: Vec<Vec<Value>>,
    width: usize,
    mut key_of: impl FnMut(&[Value]) -> Result<Vec<Value>>,
) -> Result<Vec<Vec<Vec<Value>>>> {
    use std::collections::hash_map::Entry;
    let mut keys = super::dedup::DistinctKeys::new(width);
    // Each group's first key values, beside the group's rows.
    let mut firsts: Vec<Vec<Value>> = Vec::new();
    let mut groups: Vec<Vec<Vec<Value>>> = Vec::new();
    for row in rows {
        let key = key_of(&row)?;
        let found = match keys.entry(&key) {
            Some(Entry::Occupied(held)) => super::dedup::held_group(held.get()),
            Some(Entry::Vacant(slot)) => {
                slot.insert(super::dedup::group_slot(groups.len()));
                None
            }
            None => firsts
                .iter()
                .position(|first| super::rows_distinct_equal(first, &key)),
        };
        if let Some(group) = found.and_then(|i| groups.get_mut(i)) {
            group.push(row);
        } else {
            firsts.push(key);
            groups.push(vec![row]);
        }
    }
    Ok(groups)
}

/// An `Expr::Aggregate` a group computes: the node, which keys its value, and its parts.
struct KeptAggregate<'e> {
    node: &'e Expr,
    func: AggregateFunc,
    arg: &'e Expr,
    distinct: bool,
    certainty: ArgCertainty,
}

/// Compute the aggregate `kept` over the group's rows.
fn compute_group_aggregate(
    kept: &KeptAggregate<'_>,
    resolver: &SimpleColumns<'_>,
    group_rows: &[Vec<Value>],
    scope: ProjectionScope<'_>,
    env: EvalEnv<'_>,
) -> Result<Value> {
    let arg = kept.arg;
    // Collect argument values from all rows
    let mut values: Vec<Value> = group_rows
        .iter()
        .map(|row| {
            if let Expr::Column { name, .. } = arg
                && name == "*"
            {
                // COUNT(*) counts rows, not specific column values
                return Ok(Value::Int8(1));
            }
            eval_expr_full(arg, resolver, row, SubqueryExec::NONE, env)
        })
        .collect::<Result<Vec<_>>>()?;

    // Handle DISTINCT
    if kept.distinct {
        values = deduplicate_values(values);
    }

    compute_aggregate(
        kept.func,
        &values,
        scope,
        arg,
        kept.distinct,
        kept.certainty,
    )
}

/// Compute an aggregate function over a list of values.
///
/// # Errors
///
/// Returns `NumericOutOfRange` (Postgres `22003`) if an integer `SUM` overflows
/// `i64` — integer aggregation stays in the integer domain (no `f64` precision
/// loss above 2^53) and fails loud rather than saturating.
///
/// `distinct` says `values` are a `DISTINCT` aggregate's, already deduplicated, which
/// 18.6 feeds its transition function in the argument type's sort order rather than in
/// input order. Where that order can move an answer — a float `sum`/`avg`, whose
/// rounding and overflow depend on it, and an `interval` one, whose overflow does — the
/// values are put in that order first ([`in_sort_order`]).
fn compute_aggregate(
    func: AggregateFunc,
    values: &[Value],
    scope: ProjectionScope<'_>,
    arg: &Expr,
    distinct: bool,
    certainty: ArgCertainty,
) -> Result<Value> {
    let observed = check_aggregate_operand(func, values, scope, arg)?;
    let sorted;
    let values = if distinct
        && matches!(func, AggregateFunc::Sum | AggregateFunc::Avg)
        && matches!(
            observed,
            Some(TypeOid::Float4 | TypeOid::Float8 | TypeOid::Interval)
        ) {
        sorted = in_sort_order(values, observed != Some(TypeOid::Interval));
        sorted.as_slice()
    } else {
        values
    };
    match func {
        AggregateFunc::Count => {
            // COUNT(*) counts all rows including nulls; COUNT(expr) excludes nulls
            let is_star = matches!(arg, Expr::Column { name, .. } if name == "*");
            let count = if is_star {
                values.len() as i64
            } else {
                values.iter().filter(|v| !v.is_null()).count() as i64
            };
            Ok(Value::Int8(count))
        }
        AggregateFunc::Sum => {
            let (_, arg_type) = expr_column_info(arg, scope);
            let result_type = aggregate_type_of(func, certainty, Some(arg_type));
            if result_type == Some(TypeOid::Int8) {
                // `sum(smallint|integer)` — 18.6's `int2_sum`/`int4_sum` keep an `int8`
                // running sum, exact above 2^53. A total past `i64` is `22003` here, never
                // a saturated or wrapped one; 18.6's addition is unchecked, and reaching
                // that total from `integer` inputs takes more than 2^32 rows.
                let mut sum: i64 = 0;
                let mut has_value = false;
                for v in values {
                    let Some(n) = exact_integer_input(func, arg_type, v)? else {
                        continue; // NULL
                    };
                    sum = sum
                        .checked_add(n)
                        .ok_or(Error::NumericOutOfRange("bigint"))?;
                    has_value = true;
                }
                Ok(if has_value {
                    Value::Int8(sum)
                } else {
                    Value::Null
                })
            } else if result_type == Some(TypeOid::Numeric) {
                // `sum(bigint)` and `sum(numeric)` — an exact `numeric`, whose display
                // scale is the inputs' largest, so a `bigint` total past `i64` widens
                // rather than failing.
                Ok(exact_sum(func, arg_type, values)?
                    .map_or(Value::Null, |(sum, _)| Value::Numeric(sum)))
            } else if observed == Some(TypeOid::Interval) {
                // `sum(interval)` — field-wise `interval_pl` accumulation, exactly
                // as Postgres's `interval_avg`/`interval_sum` transition does (no
                // justification pass), so `1 mon` + `1 day` stays `1 mon 1 day`.
                sum_interval(values)
            } else if arg_type == TypeOid::Float4 {
                // `sum(real)` accumulates in **f32**, not f64. `pg_aggregate.dat` gives
                // it `aggtransfn => float4pl, aggtranstype => float4`, so every partial
                // sum is rounded to f32 — which changes the answer, not just the width:
                // over `[2^24, 1, 1, 1, 1]` Postgres returns `16777216` (each `+1` is
                // absorbed) where an f64 accumulator returns `16777220`.
                //
                // The width mattered on its own: the result was a `Value::Float8` under
                // the `real` type `aggregate_result_type` declares, so a binary client
                // got 8 bytes for a 4-byte column.
                //
                // `avg(real)` is deliberately **not** here — Postgres's `float4_accum`
                // transitions through float8 and yields `double precision`, which the
                // f64 branch below already matches.
                // The transition state starts as the first input (`float4pl` is strict
                // with a NULL initial value), which an accumulator starting at `-0` is
                // exactly: `-0 + x` is `x` for every `x`, so a sum of `-0`s stays `-0`.
                let mut sum: f32 = -0.0;
                let mut has_value = false;
                for v in values {
                    // Coerce rather than pattern-match: the branch is chosen from the
                    // **static** `arg_type`, so a value that isn't already `Value::Float4`
                    // would have been dropped from the sum with no error — a silently
                    // wrong total. Nobody has produced a reachable desync, but the failure
                    // mode is bad enough to close by construction. `coerce_value` maps
                    // NULL to NULL (skipped, as every `SUM` skips NULLs) and fails loud on
                    // anything `real` cannot represent.
                    if let Value::Float4(n) = coerce_value(v.clone(), TypeOid::Float4)? {
                        sum = float4_add(sum, n)?;
                        has_value = true;
                    }
                }
                Ok(if has_value {
                    Value::Float4(sum)
                } else {
                    Value::Null
                })
            } else {
                // Float8 (or other numeric) SUM stays in f64, one `float8pl` step per input
                // — loud on an overflow from finite operands, as `real`'s is above — from
                // `-0`, which is the first input as above.
                let mut sum: f64 = -0.0;
                let mut has_value = false;
                for n in values.iter().filter_map(value_to_f64_opt) {
                    sum = float8_add(sum, n)?;
                    has_value = true;
                }
                Ok(if has_value {
                    Value::Float8(sum)
                } else {
                    Value::Null
                })
            }
        }
        AggregateFunc::Avg => {
            let (_, arg_type) = expr_column_info(arg, scope);
            if aggregate_type_of(func, certainty, Some(arg_type)) == Some(TypeOid::Numeric) {
                // `avg` over an integer or a `numeric` — the exact sum divided by the
                // count, both as `numeric`, the quotient at `select_div_scale`'s scale:
                // 18.6's `int8_avg` / `numeric_avg` finals, each a `numeric_div`.
                return Ok(match exact_sum(func, arg_type, values)? {
                    Some((sum, count)) => Value::Numeric(sum.div(&PgNumeric::from_i128(count))?),
                    None => Value::Null,
                });
            }
            if observed == Some(TypeOid::Interval) {
                // `avg(interval)` = `interval_sum / count` (Postgres's
                // `interval_avg` final function), reusing the `interval_div`
                // scaling so the fraction cascade matches byte-for-byte.
                let count = values.iter().filter(|v| !v.is_null()).count();
                if count == 0 {
                    return Ok(Value::Null);
                }
                let Value::Interval(sum) = sum_interval(values)? else {
                    return Ok(Value::Null);
                };
                #[allow(clippy::cast_precision_loss)]
                return interval_scale(&sum, count as f64, false).map(Value::Interval);
            }
            float8_avg(values)
        }
        AggregateFunc::Min => {
            // Minimum non-null value, by the argument's enum member order where it has
            // one — `min(m)` over an enum is its **first declared** member, not its
            // lexically smallest label.
            let enum_order = scope.expr_enum_order(arg);
            Ok(values
                .iter()
                .filter(|v| !v.is_null())
                .min_by(|a, b| values_compare_ordered(a, b, enum_order).unwrap_or(Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Null))
        }
        AggregateFunc::Max => {
            // Maximum non-null value (see `Min`).
            let enum_order = scope.expr_enum_order(arg);
            Ok(values
                .iter()
                .filter(|v| !v.is_null())
                .max_by(|a, b| values_compare_ordered(a, b, enum_order).unwrap_or(Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Null))
        }
    }
}

/// Accumulate `interval` values field-wise (`interval_pl`), returning NULL when
/// every input is NULL. Shared by `sum(interval)` and `avg(interval)`.
fn sum_interval(values: &[Value]) -> Result<Value> {
    let mut sum = PgInterval {
        months: 0,
        days: 0,
        micros: 0,
    };
    let mut has_value = false;
    for v in values {
        if let Value::Interval(iv) = v {
            sum = interval_add(&sum, iv)?;
            has_value = true;
        }
    }
    Ok(if has_value {
        Value::Interval(sum)
    } else {
        Value::Null
    })
}

/// The exact sum of an integer or `numeric` `sum`/`avg`'s inputs and how many
/// non-NULL inputs there were — `None` when there were none.
///
/// Integers are added in an `i128`, as 18.6's `int8` transitions add in an `int128`, and
/// `numeric` values in a [`PgNumeric`]; the two are folded once at the end, so an integer
/// argument pays no decimal addition per row. The display scale is the inputs' largest,
/// an integer's being `0`. No total of `i64` inputs overflows the `i128`: that would take
/// more than 2^63 rows.
///
/// A `numeric`-declared argument also takes an integer value, exactly — the implicit
/// `integer → numeric` conversion 18.6 applies to a branch it unifies to `numeric`, which
/// pglet can leave unconverted (`EDGES.md` row 152). Any other value is refused
/// ([`exact_integer_input`]).
fn exact_sum(
    func: AggregateFunc,
    arg_type: TypeOid,
    values: &[Value],
) -> Result<Option<(PgNumeric, i128)>> {
    let mut integers: i128 = 0;
    let mut decimals: Option<PgNumeric> = None;
    let mut count: i128 = 0;
    for v in values {
        match v {
            Value::Numeric(n) if arg_type == TypeOid::Numeric => {
                decimals = Some(match decimals {
                    Some(sum) => sum.add(n)?,
                    None => n.clone(),
                });
            }
            _ => {
                let Some(n) = exact_integer_input(func, arg_type, v)? else {
                    continue; // NULL
                };
                integers += i128::from(n);
            }
        }
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    let integers = PgNumeric::from_i128(integers);
    let sum = match decimals {
        Some(sum) => sum.add(&integers)?,
        None => integers,
    };
    Ok(Some((sum, count)))
}

/// One input of an exact `sum`/`avg` over an argument declared `arg_type`, as the
/// integer it holds — `None` for a NULL.
///
/// The accumulator is chosen by the argument's *declared* type and each value is read by
/// its own variant, and the two can disagree where pglet leaves a branch unconverted
/// (`EDGES.md` row 152). Every integer width holds its integer exactly, so any of them
/// is read; a value of another type is refused `0A000` rather than skipped or
/// converted, since the type 18.6 would have converted it to is one the declared type
/// does not say.
fn exact_integer_input(func: AggregateFunc, arg_type: TypeOid, v: &Value) -> Result<Option<i64>> {
    if v.is_null() {
        return Ok(None);
    }
    value_as_i64(v).map(Some).ok_or_else(|| {
        Error::NotImplemented(format!(
            "{}() argument declared as {} holds a value of type {}",
            if matches!(func, AggregateFunc::Sum) {
                "sum"
            } else {
                "avg"
            },
            type_oid_display_name(arg_type),
            opt_type_oid_display_name(v.type_oid()),
        ))
    })
}

/// `values` in the order 18.6 feeds a `DISTINCT` aggregate's transition function:
/// ascending by the argument type's sort order (`process_ordered_aggregate_single`).
///
/// The order is read off index keys, which encode it byte for byte: under `as_float` each
/// value is keyed as the `float8` it widens to — `float8_cmp_internal`'s order, `-0` as
/// `0` and every `NaN` one key above `Infinity` — whatever `Value` variant carries it, so
/// a bare `int4` in a float column (`EDGES.md` row 152) sorts by its value and not by its
/// own key's layout — where the caller sorts at all, which that row's bullet records as
/// turning on the argument's type; otherwise (an `interval`) by the value's own key, its comparison
/// estimate. A float `sum`/`avg` rounds and overflows by the order it adds in, and an
/// `interval` one overflows by it. The values are deduplicated already; two that key
/// alike keep their input order.
fn in_sort_order(values: &[Value], as_float: bool) -> Vec<Value> {
    let key_of = |v: &Value| {
        let widened = if as_float {
            value_to_f64_opt(v).map(Value::Float8)
        } else {
            None
        };
        widened
            .as_ref()
            .unwrap_or(v)
            .encode_index_key()
            .unwrap_or_default()
    };
    // the element type `index_row`'s size ordering sorts, so no new sort is instantiated
    let mut keyed: Vec<(Vec<u8>, usize)> = values
        .iter()
        .enumerate()
        .map(|(i, v)| (key_of(v), i))
        .collect();
    keyed.sort();
    keyed.into_iter().map(|(_, i)| values[i].clone()).collect()
}

/// `avg` over a float: `float8_accum`'s Youngs-Cramer transition, then `float8_avg`'s
/// `Sx / N`.
///
/// The sum of squares is kept only for the transition's overflow check, which is
/// 18.6's: a finite input that drives `Sx` or `Sxx` infinite is `22003`, so `avg` over
/// `1e200` and `-1e200` is refused where a plain sum would answer `0`. An infinite or
/// `NaN` input takes `Sxx` to `NaN` and passes, as there.
fn float8_avg(values: &[Value]) -> Result<Value> {
    let (mut n, mut sx, mut sxx) = (0.0_f64, 0.0_f64, 0.0_f64);
    for x in values.iter().filter_map(value_to_f64_opt) {
        let (prev_n, prev_sx) = (n, sx);
        n += 1.0;
        sx += x;
        if prev_n > 0.0 {
            // not `mul_add`: 18.6 rounds the product before the subtraction
            #[allow(clippy::suboptimal_flops)]
            let tmp = x * n - sx;
            // `float8_accum`'s update, as written there
            #[allow(clippy::suspicious_operation_groupings)]
            {
                sxx += tmp * tmp / (n * prev_n);
            }
            if sx.is_infinite() || sxx.is_infinite() {
                if !prev_sx.is_infinite() && !x.is_infinite() {
                    return Err(Error::NumericOutOfRange("double precision"));
                }
                sxx = f64::NAN;
            }
        } else if x.is_nan() || x.is_infinite() {
            sxx = f64::NAN;
        }
    }
    Ok(if n > 0.0 {
        Value::Float8(sx / n)
    } else {
        Value::Null
    })
}

/// Gate a value aggregate's operand through the shared [`aggregate_legality`]
/// rule, returning the operand type it gated on (`None` when neither the argument
/// expression nor the values determine one).
///
/// **The argument's static type wins when it is certainly knowable**
/// ([`certain_expr_type_oid`] — a column, literal, cast, or bound param), because
/// Postgres resolves the aggregate at *parse* time: `min(bool_col)` is `42883`
/// whether the table holds a thousand rows, all-NULL rows, or none. Gating on the
/// observed values alone made the same SQL over the same schema error or not
/// depending on the data — a suite green on an empty seed and red once populated,
/// the worst shape for a test-fixture engine.
///
/// Only when the static type is a *guess* (a subquery column, an operator result,
/// an unbound param) does it fall back to the observed values — the evidence
/// that's left. A group with zero non-NULL rows then has nothing observable and
/// stays un-gated; Postgres, resolving statically, still errors there.
///
/// The [`Collector`] asks the same rule of every aggregate ahead of the groups, where
/// the operand's type needs no evaluation; what is left to this call is a kept
/// aggregate over a cast and one whose operand type only its values show.
fn check_aggregate_operand(
    func: AggregateFunc,
    values: &[Value],
    scope: ProjectionScope<'_>,
    arg: &Expr,
) -> Result<Option<TypeOid>> {
    let observed =
        certain_expr_type_oid(arg, scope).or_else(|| values.iter().find_map(Value::type_oid));
    aggregate_legality(func, observed)?;
    Ok(observed)
}

/// Deduplicate values for DISTINCT aggregates: a value is kept, in first-occurrence
/// order, unless it is `values_compare`-`Equal` to a value already kept (two NULLs
/// counted equal) — a greedy rule, since that relation is not transitive over mixed
/// types.
///
/// Keyed by [`super::dedup::DistinctKeys`] while that is certain to be the relation,
/// and pairwise against the values kept so far from the first value where it is not —
/// `values_compare` folds pairs `values_equal` does not (`'1'` against `1`, through its
/// text fallback), which the gate leaves to the pairwise path.
fn deduplicate_values(values: Vec<Value>) -> Vec<Value> {
    let mut keys = super::dedup::DistinctKeys::new(1);
    let mut seen: Vec<Value> = Vec::new();
    for v in values {
        let fresh = keys
            .first_sighting(std::slice::from_ref(&v))
            .unwrap_or_else(|| {
                !seen.iter().any(|s| {
                    #[cfg(test)]
                    super::dedup::count_pairwise_ask();
                    // NULL != NULL in comparisons, but for DISTINCT they should be considered equal
                    if s.is_null() && v.is_null() {
                        return true;
                    }
                    matches!(values_compare(s, &v), Some(Ordering::Equal))
                })
            });
        if fresh {
            seen.push(v);
        }
    }
    seen
}

#[cfg(test)]
mod sum_tests {
    use super::{ProjectionScope, certain_expr_type_oid, compute_aggregate, expr_column_info};
    use crate::planner::{AggregateFunc, ArgCertainty, Expr, aggregate_result_type};
    use crate::types::{ColumnDesc, PgInterval, PgNumeric, TypeOid, Value};

    /// A column argument `v` of type `ty`.
    fn arg_of(ty: TypeOid) -> (Expr, Vec<ColumnDesc>, Vec<&'static str>) {
        (
            Expr::Column {
                table: None,
                name: "v".into(),
            },
            vec![ColumnDesc::new("v", ty)],
            vec!["v"],
        )
    }

    /// The verdict the coercion pass records for an argument of type `ty` whose type is
    /// certain, as a base table's column is.
    const fn certain(ty: TypeOid) -> ArgCertainty {
        match ty {
            TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8 => ArgCertainty::CertainInteger(ty),
            _ => ArgCertainty::NotCertainInteger,
        }
    }

    fn run(func: AggregateFunc, ty: TypeOid, values: &[Value]) -> crate::Result<Value> {
        let (arg, cols, names) = arg_of(ty);
        let scope = ProjectionScope::new(&cols, &names, None);
        compute_aggregate(func, values, scope, &arg, false, certain(ty))
    }

    /// An integer argument the coercion pass did not record as certain keeps the answers
    /// an argument of unknown type gets: `avg` is `double precision` over whatever values
    /// arrive — a `float8` among them, the case a guessed `integer` hides — and
    /// `sum(bigint)` an `int8`. Only a recorded certain integer takes the exact `numeric`
    /// rows, whatever the descriptor beneath says.
    #[test]
    fn a_guessed_integer_argument_keeps_the_float_avg_and_the_int8_sum() {
        let guessed = ArgCertainty::NotCertainInteger;
        let (arg, cols, names) = arg_of(TypeOid::Int4);
        let scope = ProjectionScope::new(&cols, &names, None);
        let values = [Value::Int4(1), Value::Float8(2.5), Value::Int4(3)];
        let r =
            compute_aggregate(AggregateFunc::Avg, &values, scope, &arg, false, guessed).unwrap();
        assert_eq!(r, Value::Float8(6.5 / 3.0));
        assert_eq!(
            expr_column_info(&agg(AggregateFunc::Avg, &arg, guessed), scope).1,
            TypeOid::Float8
        );
        let (arg, cols, names) = arg_of(TypeOid::Int8);
        let scope = ProjectionScope::new(&cols, &names, None);
        let r = compute_aggregate(
            AggregateFunc::Sum,
            &[Value::Int8(2), Value::Int8(3)],
            scope,
            &arg,
            false,
            guessed,
        )
        .unwrap();
        assert_eq!(r, Value::Int8(5));
        assert_eq!(
            expr_column_info(&agg(AggregateFunc::Sum, &arg, guessed), scope).1,
            TypeOid::Int8
        );
    }

    fn agg(func: AggregateFunc, arg: &Expr, certainty: ArgCertainty) -> Expr {
        Expr::Aggregate {
            func,
            arg: Box::new(arg.clone()),
            distinct: false,
            certainty,
        }
    }

    /// A `numeric` result as its text rendering — the display scale included, which
    /// is half of what `avg` over an integer has to get right.
    fn shown(v: &Value) -> String {
        match v {
            Value::Numeric(n) => n.to_display_string(),
            other => panic!("expected a numeric, got {other:?}"),
        }
    }

    #[test]
    fn integer_sum_is_exact_above_2_pow_53() {
        // 2^53 + 2^53 + 1 = 2^54 + 1, which f64 cannot represent (it rounds to
        // 2^54 = 18014398509481984); an `f64` accumulator returned that wrong value.
        // Under an `integer` declaration the total is an `int8`, under a `bigint` one a
        // `numeric`; either is exact.
        let values = [
            Value::Int8(9_007_199_254_740_992),
            Value::Int8(9_007_199_254_740_992),
            Value::Int4(1),
        ];
        let r = run(AggregateFunc::Sum, TypeOid::Int4, &values).unwrap();
        assert_eq!(r, Value::Int8(18_014_398_509_481_985));
        let r = run(AggregateFunc::Sum, TypeOid::Int8, &values).unwrap();
        assert_eq!(shown(&r), "18014398509481985");
    }

    #[test]
    fn integer_sum_overflow_is_loud_not_saturating() {
        // `sum(integer)` keeps an `int8` total: past `i64` is "bigint out of range"
        // (22003), not a saturated `i64::MAX`.
        let values = vec![Value::Int8(i64::MAX), Value::Int4(1)];
        let e = run(AggregateFunc::Sum, TypeOid::Int4, &values)
            .expect_err("sum(integer) overflow must error");
        assert_eq!(e.sqlstate(), "22003");
    }

    #[test]
    fn bigint_sum_widens_to_numeric_past_i64() {
        // `sum(bigint)` is `numeric`, so three `i64::MAX` are 18.6's exact total.
        let values = vec![Value::Int8(i64::MAX); 3];
        let r = run(AggregateFunc::Sum, TypeOid::Int8, &values).unwrap();
        assert_eq!(shown(&r), "27670116110564327421");
        let r = run(AggregateFunc::Avg, TypeOid::Int8, &values).unwrap();
        assert_eq!(shown(&r), "9223372036854775807");
    }

    #[test]
    fn integer_avg_is_the_exact_quotient_at_the_division_scale() {
        // 18.6's `numeric_div` answers, measured: the quotient's scale is
        // `select_div_scale`'s, never an integer's and never an `f64`'s digits.
        let ints = |xs: &[i32]| xs.iter().copied().map(Value::Int4).collect::<Vec<_>>();
        for (values, want) in [
            (ints(&[3, 4, 6]), "4.3333333333333333"),
            (ints(&[1, 2]), "1.5000000000000000"),
            (ints(&[1, 1, 1]), "1.00000000000000000000"),
            (ints(&[1_000_000, 2]), "500001.000000000000"),
            (ints(&[-7, 1, 1]), "-1.6666666666666667"),
        ] {
            let r = run(AggregateFunc::Avg, TypeOid::Int4, &values).unwrap();
            assert_eq!(shown(&r), want, "{values:?}");
        }
        let r = run(
            AggregateFunc::Avg,
            TypeOid::Int8,
            &vec![Value::Int8(9_007_199_254_740_993); 3],
        )
        .unwrap();
        assert_eq!(shown(&r), "9007199254740993.0000");
        let r = run(
            AggregateFunc::Avg,
            TypeOid::Int2,
            &[Value::Int2(2), Value::Int2(3)],
        )
        .unwrap();
        assert_eq!(shown(&r), "2.5000000000000000");
    }

    #[test]
    fn integer_sum_of_all_nulls_is_null() {
        for func in [AggregateFunc::Sum, AggregateFunc::Avg] {
            for ty in [
                TypeOid::Int2,
                TypeOid::Int4,
                TypeOid::Int8,
                TypeOid::Numeric,
            ] {
                let r = run(func, ty, &[Value::Null, Value::Null]).unwrap();
                assert_eq!(r, Value::Null, "{func:?}({ty:?})");
                assert_eq!(run(func, ty, &[]).unwrap(), Value::Null, "{func:?}({ty:?})");
            }
        }
    }

    #[test]
    fn integer_sum_skips_interspersed_nulls() {
        // NULLs among real values are skipped (not treated as 0, and they don't
        // suppress the result, nor count toward `avg`'s divisor).
        let values = vec![Value::Int8(10), Value::Null, Value::Int8(32)];
        let r = run(AggregateFunc::Sum, TypeOid::Int8, &values).unwrap();
        assert_eq!(shown(&r), "42");
        let r = run(AggregateFunc::Avg, TypeOid::Int8, &values).unwrap();
        assert_eq!(shown(&r), "21.0000000000000000");
        let r = run(AggregateFunc::Sum, TypeOid::Int4, &values).unwrap();
        assert_eq!(r, Value::Int8(42));
    }

    /// The exact accumulators are chosen by the argument's declared type and read each
    /// value by its own variant, and pglet can leave a branch unconverted so the two
    /// disagree (`EDGES.md` row 152). A value of another type used to be skipped — a
    /// silently short total. Every integer width is read exactly; anything else is
    /// refused.
    #[test]
    fn an_exact_accumulator_refuses_a_value_it_cannot_read_exactly() {
        let half = Value::Numeric(PgNumeric::parse("1.5").unwrap());
        for func in [AggregateFunc::Sum, AggregateFunc::Avg] {
            for ty in [TypeOid::Int2, TypeOid::Int4, TypeOid::Int8] {
                for foreign in [half.clone(), Value::Float8(1.5), Value::Text("2".into())] {
                    let values = vec![Value::Int4(1), foreign.clone(), Value::Int4(2)];
                    let e = run(func, ty, &values).expect_err("a foreign value must be loud");
                    assert_eq!(e.sqlstate(), "0A000", "{func:?}({ty:?}) over {foreign:?}");
                }
            }
            let e = run(func, TypeOid::Numeric, &[half.clone(), Value::Float8(1.5)])
                .expect_err("a float in a numeric accumulator must be loud");
            assert_eq!(e.sqlstate(), "0A000", "{func:?}(numeric)");
        }
    }

    /// ...and the conversion 18.6 *would* have applied to an unconverted integer in a
    /// `numeric` column, `integer → numeric`, is exact, so it is applied: the row-152
    /// shape that summed `1.0` and a bare `1` to `1.0` answers 18.6's `2.0`.
    #[test]
    fn a_numeric_accumulator_reads_an_integer_value_exactly() {
        let one = Value::Numeric(PgNumeric::parse("1.0").unwrap());
        let values = vec![one, Value::Int4(1), Value::Int8(i64::MAX), Value::Int2(-1)];
        let r = run(AggregateFunc::Sum, TypeOid::Numeric, &values).unwrap();
        assert_eq!(shown(&r), "9223372036854775808.0");
        let r = run(AggregateFunc::Avg, TypeOid::Numeric, &values[..2]).unwrap();
        assert_eq!(shown(&r), "1.00000000000000000000");
    }

    /// The value each `sum`/`avg`/`min`/`max`/`count` produces is the type
    /// `aggregate_result_type` declares for its argument — the rule the two protocols'
    /// descriptors read. A change to the accumulator alone, or to the rule alone, fails
    /// here before a fixture's declared-type leg sees it.
    #[test]
    fn every_aggregate_value_is_the_type_its_descriptor_declares() {
        let samples = [
            (TypeOid::Int2, Value::Int2(3)),
            (TypeOid::Int4, Value::Int4(3)),
            (TypeOid::Int8, Value::Int8(3)),
            (
                TypeOid::Numeric,
                Value::Numeric(PgNumeric::parse("3.5").unwrap()),
            ),
            (TypeOid::Float4, Value::Float4(3.5)),
            (TypeOid::Float8, Value::Float8(3.5)),
            (
                TypeOid::Interval,
                Value::Interval(PgInterval {
                    months: 1,
                    days: 2,
                    micros: 3,
                }),
            ),
        ];
        for func in [
            AggregateFunc::Count,
            AggregateFunc::Sum,
            AggregateFunc::Avg,
            AggregateFunc::Min,
            AggregateFunc::Max,
        ] {
            for (ty, value) in &samples {
                let r = run(func, *ty, &[value.clone(), value.clone()]).unwrap();
                assert_eq!(
                    r.type_oid(),
                    aggregate_result_type(func, Some(*ty), true),
                    "{func:?}({ty:?})"
                );
            }
        }
    }

    /// The executor's readers of an aggregate's type answer the rule's answer for
    /// `sum`/`avg` over every width: the type walk behind the simple protocol's
    /// descriptor and the certain-type gate. The gate's aggregate arm is reached from SQL
    /// only through an aggregate's own argument, which cannot be an aggregate, so no
    /// fixture sees it; this pin is what holds it to the rule.
    #[test]
    fn the_executor_readers_of_an_aggregate_type_ask_the_rule() {
        for func in [AggregateFunc::Sum, AggregateFunc::Avg] {
            for ty in [
                TypeOid::Int2,
                TypeOid::Int4,
                TypeOid::Int8,
                TypeOid::Numeric,
            ] {
                let (arg, cols, names) = arg_of(ty);
                let scope = ProjectionScope::new(&cols, &names, None);
                let agg = agg(func, &arg, certain(ty));
                let want = aggregate_result_type(func, Some(ty), true);
                assert_eq!(certain_expr_type_oid(&agg, scope), want, "{func:?}({ty:?})");
                assert_eq!(
                    Some(expr_column_info(&agg, scope).1),
                    want,
                    "{func:?}({ty:?})"
                );
            }
        }
    }

    /// Every executor reader of an aggregate's type — the descriptor walk, the
    /// certain-type gate and the accumulator — reads the certainty the coercion pass
    /// recorded, and none derives its own from what its scope shows. A recorded
    /// `bigint` over a descriptor reading `integer` is `sum`'s `numeric`, and the value
    /// is one; an argument recorded not certain over the same descriptor is `int8`, whatever the
    /// descriptor's provenance says.
    #[test]
    fn the_executor_readers_read_the_recorded_certainty() {
        let (arg, mut cols, names) = arg_of(TypeOid::Int4);
        let values = [Value::Int4(2), Value::Int8(3)];
        for provenance in [0, 16384] {
            cols[0].table_oid = provenance;
            let scope = ProjectionScope::new(&cols, &names, None);
            let recorded = ArgCertainty::CertainInteger(TypeOid::Int8);
            let sum = agg(AggregateFunc::Sum, &arg, recorded);
            assert_eq!(certain_expr_type_oid(&sum, scope), Some(TypeOid::Numeric));
            assert_eq!(expr_column_info(&sum, scope).1, TypeOid::Numeric);
            let r = compute_aggregate(AggregateFunc::Sum, &values, scope, &arg, false, recorded)
                .unwrap();
            assert_eq!(shown(&r), "5");
            let not_certain = agg(AggregateFunc::Sum, &arg, ArgCertainty::NotCertainInteger);
            assert_eq!(expr_column_info(&not_certain, scope).1, TypeOid::Int8);
            let r = compute_aggregate(
                AggregateFunc::Sum,
                &values,
                scope,
                &arg,
                false,
                ArgCertainty::NotCertainInteger,
            )
            .unwrap();
            assert_eq!(r, Value::Int8(5));
        }
    }

    fn float4_arg() -> (Expr, Vec<ColumnDesc>, Vec<&'static str>) {
        (
            Expr::Column {
                table: None,
                name: "v".into(),
            },
            vec![ColumnDesc::new("v", TypeOid::Float4)],
            vec!["v"],
        )
    }

    /// The `sum(real)` accumulator picks its branch from the **static** argument type,
    /// so it has to answer for a value that isn't a `Value::Float4`. The branch used to
    /// pattern-match `Value::Float4` and fall through on anything else, dropping the
    /// value from the sum with no error — a silently wrong total. These three lock the
    /// three outcomes the coercion replaced it with, in the order they matter:
    /// unrepresentable → loud, representable → summed, NULL → still skipped.
    ///
    /// No SQL reaches a desynced value today (that is the point of the static type), so
    /// a differential fixture cannot assert any of this; the unit is the only seam.
    #[test]
    fn float4_sum_is_loud_on_a_value_real_cannot_represent() {
        let (arg, cols, names) = float4_arg();
        let scope = ProjectionScope::new(&cols, &names, None);
        let values = vec![Value::Float4(1.0), Value::Text("not-a-real".into())];
        let e = compute_aggregate(
            AggregateFunc::Sum,
            &values,
            scope,
            &arg,
            false,
            ArgCertainty::NotCertainInteger,
        )
        .expect_err("a value `real` cannot represent must error, not be skipped");
        assert_eq!(e.sqlstate(), "22P02");
    }

    #[test]
    fn float4_sum_includes_a_coercible_foreign_value() {
        // The other half of the same rule: a value that *is* representable as `real`
        // joins the sum rather than being dropped. 1.5 + 3 + 2.5 = 7.
        let (arg, cols, names) = float4_arg();
        let scope = ProjectionScope::new(&cols, &names, None);
        let values = vec![Value::Float4(1.5), Value::Int4(3), Value::Float4(2.5)];
        let r = compute_aggregate(
            AggregateFunc::Sum,
            &values,
            scope,
            &arg,
            false,
            ArgCertainty::NotCertainInteger,
        )
        .unwrap();
        assert_eq!(r, Value::Float4(7.0));
    }

    /// The plain `sum(real)` coerces and adds each value in turn, so an overflow at an
    /// earlier value outranks a value `real` cannot represent after it.
    #[test]
    fn float4_sum_overflow_outranks_a_later_coercion_failure() {
        let (arg, cols, names) = float4_arg();
        let scope = ProjectionScope::new(&cols, &names, None);
        let values = vec![
            Value::Float4(3e38),
            Value::Float4(3e38),
            Value::Text("not-a-real".into()),
        ];
        let e = compute_aggregate(
            AggregateFunc::Sum,
            &values,
            scope,
            &arg,
            false,
            ArgCertainty::NotCertainInteger,
        )
        .expect_err("the overflow must error");
        assert_eq!(e.sqlstate(), "22003");
    }

    #[test]
    fn float4_sum_still_skips_nulls() {
        // `coerce_value` maps NULL to NULL, which the branch skips — so an
        // interspersed NULL neither errors nor suppresses `has_value`, and an
        // all-NULL group is still NULL.
        let (arg, cols, names) = float4_arg();
        let scope = ProjectionScope::new(&cols, &names, None);
        let values = vec![Value::Float4(1.5), Value::Null, Value::Float4(2.5)];
        let r = compute_aggregate(
            AggregateFunc::Sum,
            &values,
            scope,
            &arg,
            false,
            ArgCertainty::NotCertainInteger,
        )
        .unwrap();
        assert_eq!(r, Value::Float4(4.0));

        let all_null = vec![Value::Null, Value::Null];
        let r = compute_aggregate(
            AggregateFunc::Sum,
            &all_null,
            scope,
            &arg,
            false,
            ArgCertainty::NotCertainInteger,
        )
        .unwrap();
        assert_eq!(r, Value::Null);
    }
}

/// An aggregate's `DISTINCT` keeps today's pairwise answer — `values_compare`'s, which
/// folds mixed pairs the row sites' relation does not — on every input.
#[cfg(test)]
mod distinct_value_tests {
    use std::cmp::Ordering;

    use super::super::super::expr::values_compare;
    use super::deduplicate_values;
    use crate::types::{PgNumeric, Value};

    /// The pairwise algorithm, verbatim in shape.
    fn reference(values: &[Value]) -> Vec<Value> {
        let mut seen: Vec<Value> = Vec::new();
        for v in values {
            let duplicate = seen.iter().any(|s| {
                (s.is_null() && v.is_null()) || values_compare(s, v) == Some(Ordering::Equal)
            });
            if !duplicate {
                seen.push(v.clone());
            }
        }
        seen
    }

    fn num(s: &str) -> Value {
        Value::Numeric(PgNumeric::parse(s).expect("numeric"))
    }

    #[test]
    fn a_mixed_column_keeps_the_pairwise_answer() {
        for (input, expected) in [
            // `float8` 2^53 equals both `int8`s, which differ from each other
            (
                vec![
                    Value::Int8((1 << 53) + 1),
                    Value::Float8(9_007_199_254_740_992.0),
                    Value::Int8(1 << 53),
                ],
                vec![Value::Int8((1 << 53) + 1), Value::Int8(1 << 53)],
            ),
            (vec![Value::Int4(1), num("1.0")], vec![Value::Int4(1)]),
            // the text fallback folds `'1'` with `1`; the row relation does not
            (
                vec![Value::Text("1".into()), Value::Int4(1)],
                vec![Value::Text("1".into())],
            ),
            (
                vec![Value::Int4(1), Value::Null, Value::Int8(1), Value::Null],
                vec![Value::Int4(1), Value::Null],
            ),
        ] {
            assert_eq!(reference(&input), expected, "reference over {input:?}");
            assert_eq!(deduplicate_values(input.clone()), expected, "{input:?}");
        }
    }

    #[test]
    fn every_input_keeps_the_pairwise_answer() {
        let pool = [
            Value::Null,
            Value::Int4(1),
            Value::Int4(2),
            Value::Int8(1),
            Value::Int8((1 << 53) + 1),
            Value::Int8(1 << 53),
            Value::Float8(9_007_199_254_740_992.0),
            Value::Float8(-0.0),
            Value::Float8(0.0),
            num("1.0"),
            num("1.00"),
            Value::Text("1".into()),
            Value::Text("2".into()),
            Value::Text(String::new()),
        ];
        let sub_pools: [&[Value]; 5] = [&pool, &pool[..3], &pool[6..9], &pool[9..11], &pool[11..]];
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut below = |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        for round in 0..4000 {
            let sub = sub_pools[round % sub_pools.len()];
            let len = below(12);
            let input: Vec<Value> = (0..len).map(|_| sub[below(sub.len())].clone()).collect();
            assert_eq!(
                deduplicate_values(input.clone()),
                reference(&input),
                "{input:?}"
            );
        }
    }
}

/// `GROUP BY` groups by the relation `SELECT DISTINCT` dedups by, in first-seen order.
#[cfg(test)]
mod group_tests {
    use super::group_rows;
    use crate::types::{ArrayValue, PgInterval, PgNumeric, TypeOid, Value, parse_jsonb};
    use time::macros::{date, datetime};

    fn numeric(s: &str) -> Value {
        Value::Numeric(PgNumeric::parse(s).expect("numeric literal"))
    }

    fn jsonb(s: &str) -> Value {
        Value::Jsonb(parse_jsonb(s).expect("jsonb literal"))
    }

    fn array(ty: TypeOid, elements: Vec<Value>) -> Value {
        Value::Array(ArrayValue::new(ty, elements))
    }

    /// Rows compared by their `Debug` form, which tells `-0` from `0` and calls every
    /// `NaN` one — `Value`'s `PartialEq` calls a `NaN` unequal to itself.
    fn shown(groups: &[Vec<Vec<Value>>]) -> String {
        format!("{groups:?}")
    }

    /// The groups of `rows`, each row its own key.
    fn groups(rows: &[Vec<Value>]) -> Vec<Vec<Vec<Value>>> {
        let width = rows.first().map_or(0, Vec::len);
        group_rows(rows.to_vec(), width, |row| Ok(row.to_vec())).expect("grouping")
    }

    /// The pairwise algorithm: a row joins the first group whose first row it equals.
    fn reference(rows: &[Vec<Value>]) -> Vec<Vec<Vec<Value>>> {
        let mut out: Vec<Vec<Vec<Value>>> = Vec::new();
        for row in rows {
            match out
                .iter_mut()
                .find(|g| super::super::rows_distinct_equal(&g[0], row))
            {
                Some(group) => group.push(row.clone()),
                None => out.push(vec![row.clone()]),
            }
        }
        out
    }

    /// Each pair 18.6 groups as one, in both orders: one group, the first row first.
    #[test]
    fn equal_spellings_are_one_group_shown_by_the_first() {
        let f8 = |v: f64| array(TypeOid::Float8, vec![Value::Float8(v)]);
        let iv = |m, d, us| {
            array(
                TypeOid::Interval,
                vec![Value::Interval(PgInterval::new(m, d, us))],
            )
        };
        for (a, b) in [
            (jsonb(r#"{"a":1.0}"#), jsonb(r#"{"a":1}"#)),
            (
                array(TypeOid::Numeric, vec![numeric("1.0")]),
                array(TypeOid::Numeric, vec![numeric("1.00")]),
            ),
            (iv(0, 1, 0), iv(0, 0, 86_400_000_000)),
            (iv(1, 0, 0), iv(0, 30, 0)),
            (f8(0.0), f8(-0.0)),
            (f8(f64::NAN), f8(-f64::NAN)),
            (numeric("1.5"), numeric("1.50")),
            (Value::Float8(-0.0), Value::Float4(0.0)),
        ] {
            for (first, second) in [(&a, &b), (&b, &a)] {
                let rows = vec![vec![first.clone()], vec![second.clone()]];
                assert_eq!(
                    shown(&groups(&rows)),
                    shown(std::slice::from_ref(&rows)),
                    "{first:?} / {second:?}"
                );
            }
        }
    }

    /// Mixed-type key values group by `values_equal`, as `DISTINCT` dedups them — the
    /// integer widths, an integer and a `numeric`, a `date` and its midnight — but an
    /// integer is not the text that spells it (the aggregate `DISTINCT`'s relation
    /// folds those two, through its text fallback).
    #[test]
    fn mixed_types_group_by_distincts_relation() {
        let one =
            |rows: Vec<Value>| -> Vec<Vec<Value>> { rows.into_iter().map(|v| vec![v]).collect() };
        let midnight = Value::timestamp(datetime!(2024-01-01 00:00:00 UTC));
        for (input, expected) in [
            (vec![Value::Int4(1), Value::Int8(1)], vec![2]),
            (vec![Value::Int4(1), numeric("1.0")], vec![2]),
            (vec![Value::date(date!(2024 - 01 - 01)), midnight], vec![2]),
            (vec![Value::Int4(1), Value::Text("1".into())], vec![1, 1]),
            (vec![Value::Text("1".into()), Value::Int4(1)], vec![1, 1]),
            // `float8` 2^53 equals both `int8`s, which differ: it joins the first group
            (
                vec![
                    Value::Int8((1 << 53) + 1),
                    Value::Float8(9_007_199_254_740_992.0),
                    Value::Int8(1 << 53),
                ],
                vec![2, 1],
            ),
        ] {
            let rows = one(input);
            let got = groups(&rows);
            assert_eq!(
                got.iter().map(Vec::len).collect::<Vec<_>>(),
                expected,
                "{rows:?}"
            );
            assert_eq!(shown(&got), shown(&reference(&rows)), "{rows:?}");
        }
    }

    /// The groups come out in the order their first rows arrive, every input.
    #[test]
    fn groups_keep_first_seen_order() {
        let rows: Vec<Vec<Value>> = [3, 1, 3, 2, 1, 4]
            .into_iter()
            .map(|n| vec![Value::Int4(n)])
            .collect();
        let firsts: Vec<Value> = groups(&rows).into_iter().map(|g| g[0][0].clone()).collect();
        assert_eq!(firsts, [3, 1, 2, 4].map(Value::Int4).to_vec());
    }

    /// The keyed grouping answers the pairwise algorithm's groups on every input, over
    /// mixed and single-type pools of two-column rows.
    #[test]
    fn every_input_groups_as_the_pairwise_algorithm() {
        let pool = [
            Value::Null,
            Value::Int4(1),
            Value::Int4(2),
            Value::Int8(1),
            Value::Int8((1 << 53) + 1),
            Value::Int8(1 << 53),
            Value::Float8(9_007_199_254_740_992.0),
            Value::Float8(-0.0),
            Value::Float8(0.0),
            Value::Float8(f64::NAN),
            numeric("1.0"),
            numeric("1.00"),
            Value::Text("1".into()),
            Value::Text(String::new()),
            jsonb(r#"{"a":1.0}"#),
            jsonb(r#"{"a":1}"#),
            array(TypeOid::Numeric, vec![numeric("1.0")]),
            array(TypeOid::Numeric, vec![numeric("1.00")]),
            array(TypeOid::Int4, vec![Value::Int4(1)]),
            array(TypeOid::Int8, vec![Value::Int8(1)]),
        ];
        let sub_pools: [&[Value]; 6] = [
            &pool,
            &pool[..4],
            &pool[6..10],
            &pool[10..12],
            &pool[14..16],
            &pool[16..],
        ];
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut below = |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        for round in 0..4000 {
            let sub = sub_pools[round % sub_pools.len()];
            let len = below(10);
            let rows: Vec<Vec<Value>> = (0..len)
                .map(|_| vec![sub[below(sub.len())].clone(), sub[below(sub.len())].clone()])
                .collect();
            assert_eq!(shown(&groups(&rows)), shown(&reference(&rows)), "{rows:?}");
        }
    }
}

/// A `DISTINCT` float or `interval` `sum`/`avg` adds its values in the type's sort order;
/// every other aggregate keeps input order.
#[cfg(test)]
mod distinct_order_tests {
    use super::{ProjectionScope, compute_aggregate};
    use crate::planner::{AggregateFunc, ArgCertainty, Expr};
    use crate::types::{ColumnDesc, PgInterval, TypeOid, Value};
    use crate::{Error, Result};

    fn run(func: AggregateFunc, ty: TypeOid, values: &[Value], distinct: bool) -> Result<Value> {
        let arg = Expr::Column {
            table: None,
            name: "v".into(),
        };
        let cols = vec![ColumnDesc::new("v", ty)];
        let names = vec!["v"];
        let scope = ProjectionScope::new(&cols, &names, None);
        // a scan's column, whose type the coercion pass records as certain
        let certainty = match ty {
            TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8 => ArgCertainty::CertainInteger(ty),
            _ => ArgCertainty::NotCertainInteger,
        };
        compute_aggregate(func, values, scope, &arg, distinct, certainty)
    }

    fn f8(values: &[f64]) -> Vec<Value> {
        values.iter().copied().map(Value::Float8).collect()
    }

    fn overflow(r: &Result<Value>) -> bool {
        matches!(r, Err(Error::NumericOutOfRange(_)))
    }

    #[test]
    fn a_distinct_float_sum_adds_in_sort_order() {
        let v = f8(&[1e16, -1e16, 1.0]);
        let (sum, avg) = (AggregateFunc::Sum, AggregateFunc::Avg);
        assert_eq!(
            run(sum, TypeOid::Float8, &v, true).unwrap(),
            Value::Float8(0.0)
        );
        assert_eq!(
            run(avg, TypeOid::Float8, &v, true).unwrap(),
            Value::Float8(0.0)
        );
        // the plain aggregates keep input order
        assert_eq!(
            run(sum, TypeOid::Float8, &v, false).unwrap(),
            Value::Float8(1.0)
        );
        let r = vec![Value::Float4(1e8), Value::Float4(-1e8), Value::Float4(1.0)];
        assert_eq!(
            run(sum, TypeOid::Float4, &r, true).unwrap(),
            Value::Float4(0.0)
        );
        assert_eq!(
            run(sum, TypeOid::Float4, &r, false).unwrap(),
            Value::Float4(1.0)
        );
    }

    /// `NaN` sorts above every value, whatever its sign bit, so the finite values are
    /// added first and overflow — `partial_cmp` leaves the `NaN` where it was, and
    /// `f64::total_cmp` sorts a negative `NaN` first; either would answer `NaN`.
    #[test]
    fn a_nan_sorts_last_in_a_distinct_float_sum() {
        for nan in [f64::NAN, -f64::NAN] {
            let v = f8(&[nan, 1.7e308, 1e308]);
            assert!(
                overflow(&run(AggregateFunc::Sum, TypeOid::Float8, &v, true)),
                "{nan}"
            );
            assert!(
                overflow(&run(AggregateFunc::Avg, TypeOid::Float8, &v, true)),
                "{nan}"
            );
            let Value::Float8(plain) = run(AggregateFunc::Sum, TypeOid::Float8, &v, false).unwrap()
            else {
                panic!("float8 sum");
            };
            assert!(plain.is_nan());
        }
        let r = vec![
            Value::Float4(-f32::NAN),
            Value::Float4(3e38),
            Value::Float4(2e38),
        ];
        assert!(overflow(&run(
            AggregateFunc::Sum,
            TypeOid::Float4,
            &r,
            true
        )));
    }

    /// A float sum starts from its first input: a sum of `-0`s is `-0`; `avg` from `0`.
    #[test]
    fn a_float_sum_of_negative_zeros_is_negative_zero() {
        let neg = |v: Value| {
            matches!(v, Value::Float8(x) if x == 0.0 && x.is_sign_negative())
                || matches!(v, Value::Float4(x) if x == 0.0 && x.is_sign_negative())
        };
        let v = f8(&[-0.0, -0.0]);
        assert!(neg(
            run(AggregateFunc::Sum, TypeOid::Float8, &v, false).unwrap()
        ));
        assert!(neg(run(
            AggregateFunc::Sum,
            TypeOid::Float4,
            &[Value::Float4(-0.0)],
            false
        )
        .unwrap()));
        assert!(!neg(run(
            AggregateFunc::Sum,
            TypeOid::Float8,
            &f8(&[-0.0, 0.0]),
            false
        )
        .unwrap()));
        assert!(!neg(
            run(AggregateFunc::Avg, TypeOid::Float8, &v, false).unwrap()
        ));
    }

    /// `sum` is `float8pl` and `avg` `float8_accum`: overflow from finite inputs is
    /// loud, the sum of squares included, and an infinite or `NaN` input passes.
    #[test]
    fn float_overflow_is_loud_from_finite_inputs_only() {
        let (sum, avg) = (AggregateFunc::Sum, AggregateFunc::Avg);
        assert!(overflow(&run(
            sum,
            TypeOid::Float8,
            &f8(&[1e308, 1e308]),
            false
        )));
        assert!(overflow(&run(
            avg,
            TypeOid::Float8,
            &f8(&[1e200, -1e200]),
            false
        )));
        assert!(overflow(&run(
            avg,
            TypeOid::Float8,
            &f8(&[1e308, 1e308, f64::INFINITY]),
            false
        )));
        assert_eq!(
            run(
                avg,
                TypeOid::Float8,
                &f8(&[f64::INFINITY, 1e308, 1e308]),
                false
            )
            .unwrap(),
            Value::Float8(f64::INFINITY)
        );
        assert_eq!(
            run(
                sum,
                TypeOid::Float8,
                &f8(&[f64::INFINITY, 1e308, 1e308]),
                false
            )
            .unwrap(),
            Value::Float8(f64::INFINITY)
        );
        let Value::Float8(nan) =
            run(avg, TypeOid::Float8, &f8(&[f64::NAN, 1e308, 1e308]), false).unwrap()
        else {
            panic!("float8 avg");
        };
        assert!(nan.is_nan());
        assert!(overflow(&run(
            avg,
            TypeOid::Float8,
            &f8(&[1e308, -1e308]),
            false
        )));
        assert_eq!(
            run(avg, TypeOid::Float8, &f8(&[1e150, -1e150]), false).unwrap(),
            Value::Float8(0.0)
        );
    }

    /// A `DISTINCT` `interval` sum adds in `interval_cmp`'s order, where a partial sum
    /// of the input order would overflow the month count.
    #[test]
    fn a_distinct_interval_sum_adds_in_sort_order() {
        let years = |y: i32| Value::Interval(PgInterval::new(y * 12, 0, 0));
        let v = vec![years(178_000_000), years(100_000_000), years(-178_000_000)];
        for func in [AggregateFunc::Sum, AggregateFunc::Avg] {
            assert!(run(func, TypeOid::Interval, &v, true).is_ok(), "{func:?}");
            assert!(run(func, TypeOid::Interval, &v, false).is_err(), "{func:?}");
        }
        assert_eq!(
            run(AggregateFunc::Sum, TypeOid::Interval, &v, true).unwrap(),
            years(100_000_000)
        );
    }

    /// A bare `int4` in a `float8` column (the `EDGES.md` row 152 shape) sorts by its value:
    /// `-1e16`, `2`, `3` is 18.6's order, where the `int4`'s own key bytes put `3` before
    /// the `float8` `2` and round the sum differently.
    #[test]
    fn a_bare_integer_in_a_float_column_sorts_by_its_value() {
        let n = "CASE WHEN x = 7 THEN (SELECT * FROM unnest(ARRAY[-1e16::float8])) \
                 WHEN x = 8 THEN (SELECT * FROM unnest(ARRAY[2.0::float8])) ELSE x END";
        let sql = format!(
            "SELECT sum(DISTINCT n), avg(DISTINCT n) FROM (SELECT {n} AS n FROM (VALUES (7), (8), (3)) v(x)) s"
        );
        let mut schema = crate::catalog::Schema::new();
        let plan = crate::parser::parse(&sql, &schema)
            .expect("parse")
            .remove(0);
        let rows = crate::executor::execute(&plan, &mut schema)
            .expect("execute")
            .rows;
        // the column does carry a bare `int4`, or this asks nothing of the order
        let column = format!("SELECT {n} AS n FROM (VALUES (3)) v(x)");
        let plan = crate::parser::parse(&column, &schema)
            .expect("parse")
            .remove(0);
        let bare = crate::executor::execute(&plan, &mut schema)
            .expect("execute")
            .rows;
        assert_eq!(bare, vec![vec![Value::Int4(3)]]);
        assert_eq!(
            rows,
            vec![vec![
                Value::Float8(-9_999_999_999_999_996.0),
                Value::Float8(-3_333_333_333_333_332.0),
            ]]
        );
    }

    /// `avg` over an integer is exact, so a `DISTINCT` one has no order to keep:
    /// `2^53 + 1` survives the cancellation an `f64` accumulator rounded it into.
    #[test]
    fn a_distinct_integer_avg_is_exact() {
        let v = vec![
            Value::Int8(1 << 53),
            Value::Int8(1),
            Value::Int8(-(1 << 53)),
        ];
        let Value::Numeric(avg) = run(AggregateFunc::Avg, TypeOid::Int8, &v, true).unwrap() else {
            panic!("avg(bigint) is numeric");
        };
        assert_eq!(avg.to_display_string(), "0.33333333333333333333");
    }
}

#[cfg(test)]
mod slot_tests {
    use super::{AggregateSlots, KeptAggregate};
    use crate::planner::{AggregateFunc, ArgCertainty, Expr};
    use crate::types::Value;

    fn sum_of_a() -> Expr {
        Expr::Aggregate {
            func: AggregateFunc::Sum,
            arg: Box::new(Expr::Column {
                table: None,
                name: "a".into(),
            }),
            distinct: false,
            certainty: ArgCertainty::NotCertainInteger,
        }
    }

    fn kept(node: &Expr) -> KeptAggregate<'_> {
        let Expr::Aggregate {
            func,
            arg,
            distinct,
            certainty,
        } = node
        else {
            unreachable!("built as an aggregate")
        };
        KeptAggregate {
            node,
            func: *func,
            arg,
            distinct: *distinct,
            certainty: *certainty,
        }
    }

    /// The lookup is keyed by the node, not by its spelling: a second `sum(a)` — an inner
    /// query's, or one under a branch the grouping folded away — reads no value from the
    /// first one's slot, so the scalar walk refuses it rather than answering with
    /// another node's value.
    #[test]
    fn a_textually_equal_node_reads_no_value() {
        let collected = sum_of_a();
        let other = sum_of_a();
        assert_eq!(collected, other);
        let slots = AggregateSlots::new(&[kept(&collected)]);
        let values = [Value::Int8(5)];
        assert_eq!(slots.value(&collected, &values), Some(Value::Int8(5)));
        assert_eq!(slots.value(&other, &values), None);
    }

    /// Each node reads its own slot, whatever order the addresses sort in.
    #[test]
    fn each_node_reads_its_own_slot() {
        let nodes = [sum_of_a(), sum_of_a(), sum_of_a()];
        let kept: Vec<KeptAggregate<'_>> = nodes.iter().map(kept).collect();
        let slots = AggregateSlots::new(&kept);
        let values: Vec<Value> = (0..3).map(Value::Int8).collect();
        for (n, node) in (0..).zip(&nodes) {
            assert_eq!(slots.value(node, &values), Some(Value::Int8(n)));
        }
        // A slot whose phase has not computed it yet reads nothing.
        assert_eq!(slots.value(&nodes[2], &values[..2]), None);
    }
}

#[cfg(test)]
mod collector_tests {
    use super::{Collector, ProjectionScope, is_non_null_literal};
    use crate::catalog::Schema;
    use crate::executor::types::ColumnDesc;
    use crate::planner::{BinaryOp, Expr, Literal, Plan};
    use crate::types::{PgNumeric, TypeOid};

    /// `u (a INT, b INT, t TEXT, n NUMERIC, f FLOAT8, bo BOOL)`.
    fn schema() -> Schema {
        let mut schema = Schema::new();
        let create = "CREATE TABLE u (a INT, b INT, t TEXT, n NUMERIC, f FLOAT8, bo BOOL)";
        let plan = crate::parser::parse(create, &schema)
            .expect("parses")
            .remove(0);
        crate::executor::execute(&plan, &mut schema).expect("creates");
        schema
    }

    /// The grouping node of `plan`, under the nodes that project and sort its output.
    fn grouping(plan: &Plan) -> (&[Expr], Option<&Expr>) {
        match plan {
            Plan::Aggregate { exprs, having, .. } => (exprs, having.as_ref()),
            Plan::Project { input, .. }
            | Plan::Sort { input, .. }
            | Plan::JunkFilter { input, .. } => grouping(input),
            other => panic!("no grouping node under {other:?}"),
        }
    }

    /// How many aggregates the collector keeps from the statement `sql`, as planned and
    /// coerced — the select list, `HAVING` and `ORDER BY` junk together — and how many
    /// nodes it visits.
    fn collect(sql: &str, schema: &Schema) -> (usize, usize) {
        let plan = crate::parser::parse(sql, schema)
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
            .remove(0);
        let (exprs, having) = grouping(&plan);
        let mut collector = Collector::new(ProjectionScope::new(&[], &[], None));
        for expr in having.into_iter().chain(exprs) {
            collector.collect(expr).expect("collects");
        }
        (collector.kept.len(), collector.visited)
    }

    /// How many aggregates the collector keeps from `SELECT <item> FROM u`.
    fn kept(item: &str, schema: &Schema) -> usize {
        collect(&format!("SELECT {item} FROM u"), schema).0
    }

    /// Each certain decider drops the branch its value decides and keeps the one it does
    /// not: the same spelling with the other value keeps the aggregate.
    #[test]
    fn a_certain_decider_drops_exactly_the_branch_it_decides() {
        let schema = schema();
        for (item, expected) in [
            // A deciding left operand of a connective; `NULL` decides neither.
            ("true OR sum(1/b) > 0", 0),
            ("false OR sum(1/b) > 0", 1),
            ("NULL OR sum(1/b) > 0", 1),
            ("false AND sum(1/b) > 0", 0),
            ("true AND sum(1/b) > 0", 1),
            ("NULL AND sum(1/b) > 0", 1),
            // A comparison of two number literals, under each operator.
            ("1 = 1 OR sum(1/b) > 0", 0),
            ("1 = 2 OR sum(1/b) > 0", 1),
            ("1 <> 2 OR sum(1/b) > 0", 0),
            ("1 <> 1 OR sum(1/b) > 0", 1),
            ("1 < 2 OR sum(1/b) > 0", 0),
            ("2 < 2 OR sum(1/b) > 0", 1),
            ("2 <= 2 OR sum(1/b) > 0", 0),
            ("3 <= 2 OR sum(1/b) > 0", 1),
            ("3 > 2 OR sum(1/b) > 0", 0),
            ("2 > 2 OR sum(1/b) > 0", 1),
            ("2 >= 2 OR sum(1/b) > 0", 0),
            ("1 >= 2 OR sum(1/b) > 0", 1),
            ("1 = 1.0 OR sum(1/b) > 0", 0),
            ("1.5 < 1 OR sum(1/b) > 0", 1),
            ("10000000000 > 1 OR sum(1/b) > 0", 0),
            // Compared exactly, where a float would call the operands equal.
            ("16777217 = 16777216 OR sum(1/b) > 0", 1),
            ("1.0000000001 > 1 OR sum(1/b) > 0", 0),
            // `NOT` and the tests over a certain operand.
            ("NOT false OR sum(1/b) > 0", 0),
            ("NOT true OR sum(1/b) > 0", 1),
            ("NOT NULL OR sum(1/b) > 0", 1),
            ("NULL IS NULL OR sum(1/b) > 0", 0),
            ("true IS NULL OR sum(1/b) > 0", 1),
            ("true IS NOT NULL OR sum(1/b) > 0", 0),
            ("NULL IS NOT NULL OR sum(1/b) > 0", 1),
            ("(1 = 1) IS TRUE OR sum(1/b) > 0", 0),
            ("NULL IS TRUE OR sum(1/b) > 0", 1),
            ("false IS NOT TRUE OR sum(1/b) > 0", 0),
            ("true IS NOT TRUE OR sum(1/b) > 0", 1),
            ("false IS FALSE OR sum(1/b) > 0", 0),
            ("NULL IS FALSE OR sum(1/b) > 0", 1),
            ("NULL IS NOT FALSE OR sum(1/b) > 0", 0),
            ("false IS NOT FALSE OR sum(1/b) > 0", 1),
            ("NULL IS UNKNOWN OR sum(1/b) > 0", 0),
            ("true IS UNKNOWN OR sum(1/b) > 0", 1),
            ("true IS NOT UNKNOWN OR sum(1/b) > 0", 0),
            ("NULL IS NOT UNKNOWN OR sum(1/b) > 0", 1),
            // A connective of two certain operands is one.
            ("(false OR true) OR sum(1/b) > 0", 0),
            ("(false OR false) OR sum(1/b) > 0", 1),
            ("(NULL OR true) OR sum(1/b) > 0", 0),
            ("(NULL OR false) OR sum(1/b) > 0", 1),
            ("(true AND false) AND sum(1/b) > 0", 0),
            ("(true AND true) AND sum(1/b) > 0", 1),
            ("(NULL AND false) AND sum(1/b) > 0", 0),
            ("(true AND NULL) AND sum(1/b) > 0", 1),
            // ...by the three-valued table: `NULL AND true` and `NULL OR false` are
            // `NULL`, which `NOT` leaves `NULL`.
            ("CASE WHEN (NULL AND true) THEN sum(1/b) ELSE 0 END", 0),
            ("CASE WHEN NOT (NULL AND true) THEN sum(1/b) ELSE 0 END", 0),
            ("CASE WHEN (NULL OR false) THEN sum(1/b) ELSE 0 END", 0),
            ("CASE WHEN NOT (NULL OR false) THEN sum(1/b) ELSE 0 END", 0),
            ("CASE WHEN NOT (false OR NULL) THEN sum(1/b) ELSE 0 END", 0),
            // A searched `CASE`: a certainly untaken arm, and what follows a taken one.
            ("CASE WHEN false THEN sum(1/b) END", 0),
            ("CASE WHEN NULL THEN sum(1/b) END", 0),
            ("CASE WHEN true THEN sum(1/b) END", 1),
            ("CASE WHEN true THEN 0 ELSE sum(1/b) END", 0),
            ("CASE WHEN false THEN 0 ELSE sum(1/b) END", 1),
            (
                "CASE WHEN count(*) > 5 THEN 1 WHEN true THEN 2 ELSE sum(1/b) END",
                1,
            ),
            (
                "CASE WHEN count(*) > 5 THEN 1 WHEN false THEN 2 ELSE sum(1/b) END",
                2,
            ),
            (
                "CASE WHEN true THEN 0 WHEN count(*) > 5 THEN 1 ELSE sum(1/b) END",
                0,
            ),
            // A simple `CASE` over number literals.
            ("CASE 1 WHEN 1 THEN 0 ELSE sum(1/b) END", 0),
            ("CASE 1 WHEN 2 THEN 0 ELSE sum(1/b) END", 1),
            ("CASE 1 WHEN 2 THEN sum(1/b) ELSE 0 END", 0),
            ("CASE 1 WHEN 1 THEN sum(1/b) ELSE 0 END", 1),
            ("CASE 1 WHEN 1.0 THEN 0 ELSE sum(1/b) END", 0),
            (
                "CASE 1 WHEN count(*) THEN 3 WHEN 1 THEN 4 ELSE sum(1/b) END",
                1,
            ),
            // `COALESCE`: what follows a number or boolean literal, bare or under the
            // coercion pass's cast.
            ("coalesce(1, sum(1/b))", 0),
            ("coalesce(NULL, sum(1/b))", 1),
            ("coalesce(1.5, sum(1/b))", 0),
            ("coalesce(1, sum(n / b))", 0),
            ("coalesce(1, sum(f / b))", 0),
            ("coalesce(true, max(a) > 0)", 0),
            ("coalesce(max(a), 1, sum(1/b))", 1),
            ("coalesce(max(a), NULL, sum(1/b))", 2),
            ("coalesce(NULL, 1, sum(1/b))", 0),
            ("coalesce(10000000000, sum(1/b))", 0),
            // A drop nested in a drop ends without ending the outer one: what follows
            // it, still inside the outer drop, is dropped too.
            (
                "CASE WHEN false THEN coalesce(CASE WHEN false THEN 1 END, sum(1/b)) END",
                0,
            ),
            ("true OR ((false AND max(a) > 0) OR sum(1/b) > 0)", 0),
            ("false AND ((true OR max(a) > 0) AND sum(1/b) > 0)", 0),
            (
                "coalesce(1, CASE WHEN true THEN 2 ELSE max(a) END, sum(1/b))",
                0,
            ),
            // ...and a drop that has ended drops nothing after it.
            ("(false AND max(a) > 0) OR sum(1/b) > 0", 1),
            ("coalesce(CASE WHEN false THEN max(a) END, sum(1/b))", 1),
        ] {
            assert_eq!(kept(item, &schema), expected, "{item}");
        }
    }

    /// A decider that is a constant to 18.6's planner but not by its shape alone — and
    /// one that is none there at all — keeps every aggregate, whichever value it has.
    #[test]
    fn a_neighbour_of_a_certain_decider_keeps_every_aggregate() {
        let schema = schema();
        for decider in [
            // strings: their order is the collation's
            "'a' < 'b'",
            "'a' = 'a'",
            "'t'",
            // a bound parameter
            "$1",
            "$1 = 1",
            "$1::bool",
            // a computed constant
            "1 + 1 = 2",
            "7 % 2 = 1",
            "length('abc') = 3",
            "abs(-1) = 1",
            "coalesce(NULL, true)",
            "1 IN (1, 2)",
            "1 IS DISTINCT FROM 2",
            "(CASE WHEN true THEN true END)",
            // a cast, of anything
            "1::bool",
            "'yes'::bool",
            "NULL::bool",
            "NULL::int IS NULL",
            "1::int8 = 1",
            // a comparison of anything but two number literals
            "true = true",
            "NULL = 1",
            "1 = NULL",
            "'2020-01-01'::date < '2021-01-01'::date",
            // not a constant at all
            "now() IS NOT NULL",
            "current_user IS NOT NULL",
        ] {
            for item in [
                format!("{decider} OR sum(1/b) > 0"),
                format!("NOT ({decider}) AND sum(1/b) > 0"),
                format!("CASE WHEN {decider} THEN 0 ELSE sum(1/b) END"),
                format!("CASE WHEN NOT ({decider}) THEN sum(1/b) ELSE 0 END"),
            ] {
                assert_eq!(kept(&item, &schema), 1, "{item}");
            }
        }
        for item in [
            // The deciding constant on the right, which the scalar walk reaches last.
            "sum(1/b) > 0 OR true",
            "sum(1/b) > 0 AND false",
            // An `IN` list drops nothing.
            "1 IN (1, sum(1/b))",
            "1 NOT IN (1, sum(1/b))",
            "NULL IN (1, sum(1/b))",
            "NULL::int IN (sum(1/b))",
            // A strict operator or function over a `NULL`.
            "NULL::int + sum(1/b)",
            "nullif(NULL, sum(1/b))",
            // A simple `CASE` over anything but two number literals.
            "CASE 'a' WHEN 'a' THEN 0 ELSE sum(1/b) END",
            "CASE true WHEN true THEN 0 ELSE sum(1/b) END",
            "CASE NULL WHEN NULL THEN sum(1/b) ELSE 0 END",
            "CASE NULL::int WHEN 1 THEN sum(1/b) ELSE 0 END",
            "CASE 1 WHEN NULL THEN sum(1/b) ELSE 0 END",
            "CASE $1 WHEN 1 THEN 0 ELSE sum(1/b) END",
            "CASE 1 WHEN $1 THEN 0 ELSE sum(1/b) END",
            "CASE 1 + 1 WHEN 2 THEN 0 ELSE sum(1/b) END",
            "CASE 1 WHEN 1 + 0 THEN 0 ELSE sum(1/b) END",
            "CASE 1::int8 WHEN 1 THEN 0 ELSE sum(1/b) END",
            // A `COALESCE` argument that is no number or boolean literal.
            "coalesce('x', max(1/b)::text)",
            "coalesce($1, sum(1/b))",
            "coalesce($1::int, sum(1/b))",
            "coalesce(1 + 1, sum(1/b))",
            "coalesce(abs(1), sum(1/b))",
            "coalesce(coalesce(1), sum(1/b))",
            "coalesce(NULL::int, sum(1/b))",
            "coalesce(1::text, max(1/b)::text)",
            "coalesce('1'::int, sum(1/b))",
            "coalesce('2020-01-01'::date, max('2020-01-01'::date + 1/b))",
            // Other constructs over constants.
            "greatest(1, sum(1/b))",
            "nullif(1, sum(1/b))",
            "concat(NULL, sum(1/b))",
            "ARRAY[1, sum(1/b)]",
        ] {
            assert_eq!(kept(item, &schema), 1, "{item}");
        }
    }

    /// The statement `sql` collected over `u`'s columns as the grouping's input: how many
    /// aggregates are kept, or the refusal's message.
    fn checked(sql: &str, schema: &Schema) -> Result<usize, String> {
        let input = [
            ColumnDesc::new("a", TypeOid::Int4),
            ColumnDesc::new("b", TypeOid::Int4),
            ColumnDesc::new("t", TypeOid::Text),
            ColumnDesc::new("n", TypeOid::Numeric),
            ColumnDesc::new("f", TypeOid::Float8),
            ColumnDesc::new("bo", TypeOid::Bool),
        ];
        let names = ["a", "b", "t", "n", "f", "bo"];
        let plan = crate::parser::parse(sql, schema)
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
            .remove(0);
        let (exprs, having) = grouping(&plan);
        let mut collector = Collector::new(ProjectionScope::new(&input, &names, None));
        for expr in exprs.iter().chain(having) {
            collector.collect(expr).map_err(|e| {
                assert_eq!(e.sqlstate(), "42883", "{sql}: {e}");
                e.to_string()
            })?;
        }
        Ok(collector.kept.len())
    }

    /// Dropping an aggregate drops its computation, never its operand's type check: an
    /// aggregate with no form for its operand's type is refused at collection in a kept
    /// position and in each kind of dropped one, and one with a form is not.
    #[test]
    fn an_aggregate_operand_type_is_checked_kept_or_dropped() {
        let schema = schema();
        for (sql, refused) in [
            ("SELECT sum(t) FROM u", "sum(text)"),
            ("SELECT true OR sum(t) > 0 FROM u", "sum(text)"),
            ("SELECT false AND max(bo) FROM u", "max(boolean)"),
            (
                "SELECT true OR (count(*) > 0 AND avg(t) > 0) FROM u",
                "avg(text)",
            ),
            (
                "SELECT CASE WHEN false THEN min(bo) ELSE true END FROM u",
                "min(boolean)",
            ),
            (
                "SELECT CASE WHEN true THEN 1 WHEN max(bo) THEN 2 END FROM u",
                "max(boolean)",
            ),
            (
                "SELECT CASE WHEN true THEN 1 WHEN false THEN sum(t)::int END FROM u",
                "sum(text)",
            ),
            (
                "SELECT CASE WHEN true THEN 'x' ELSE sum(t)::text END FROM u",
                "sum(text)",
            ),
            (
                "SELECT CASE 1 WHEN 1 THEN true ELSE min(bo) END FROM u",
                "min(boolean)",
            ),
            (
                "SELECT CASE 1 WHEN 2 THEN min(bo) ELSE true END FROM u",
                "min(boolean)",
            ),
            (
                "SELECT CASE 1 WHEN 1 THEN 1 WHEN 3 THEN sum(t)::int END FROM u",
                "sum(text)",
            ),
            ("SELECT coalesce(1.5, avg(t)) FROM u", "avg(text)"),
            (
                "SELECT coalesce(1, max(a), sum(t)::int) FROM u",
                "sum(text)",
            ),
            (
                "SELECT a FROM u GROUP BY a HAVING false AND sum(t) > 0",
                "sum(text)",
            ),
            (
                "SELECT a FROM u GROUP BY a ORDER BY true OR sum(t) > 0, a",
                "sum(text)",
            ),
            // A dropped aggregate over a cast is judged by the cast's target.
            ("SELECT true OR sum(a::text) > 0 FROM u", "sum(text)"),
        ] {
            let message = checked(sql, &schema).expect_err(sql);
            assert!(message.contains(refused), "{sql}: {message}");
        }
        for (sql, kept) in [
            ("SELECT true OR sum(a) > 0 FROM u", 0),
            ("SELECT true OR max(t) > 'a' FROM u", 0),
            ("SELECT true OR count(bo) > 0 FROM u", 0),
            ("SELECT CASE WHEN false THEN max(t) END FROM u", 0),
            ("SELECT coalesce(1, max(a)) FROM u", 0),
            ("SELECT true OR min(a::text) > 'a' FROM u", 0),
            // No certain type without evaluation: no verdict here.
            ("SELECT true OR sum(upper(t)) > 0 FROM u", 0),
            ("SELECT sum(upper(t)) FROM u", 1),
            // A kept aggregate over a cast waits for its rows: the cast's own refusal,
            // raised as the argument is evaluated, comes first.
            ("SELECT max(a::bool) FROM u", 1),
            ("SELECT sum(a::text) FROM u", 1),
        ] {
            assert_eq!(checked(sql, &schema), Ok(kept), "{sql}");
        }
    }

    /// What ends a `COALESCE` is a number or boolean literal, bare or cast to a numeric
    /// type — asked of the predicate itself, since the coercion pass leaves some of
    /// these shapes beside no typed sibling.
    #[test]
    fn the_coalesce_stop_is_a_number_or_boolean_literal() {
        let cast = |inner: Expr, target_type: TypeOid| Expr::Cast {
            expr: Box::new(inner),
            target_type,
        };
        let int = || Expr::Literal(Literal::Int(1));
        let numeric = || Expr::Literal(Literal::Numeric(PgNumeric::parse("1.5").expect("numeric")));
        let string = || Expr::Literal(Literal::String("1".into()));
        for (expr, expected) in [
            (int(), true),
            (numeric(), true),
            (Expr::Literal(Literal::Bool(false)), true),
            (cast(int(), TypeOid::Int2), true),
            (cast(int(), TypeOid::Int8), true),
            (cast(int(), TypeOid::Numeric), true),
            (cast(numeric(), TypeOid::Float8), true),
            (cast(int(), TypeOid::Float4), true),
            (Expr::Literal(Literal::Null), false),
            (string(), false),
            (Expr::Parameter { index: 1 }, false),
            (cast(Expr::Literal(Literal::Null), TypeOid::Int8), false),
            (cast(string(), TypeOid::Int4), false),
            (cast(Expr::Parameter { index: 1 }, TypeOid::Int4), false),
            (cast(int(), TypeOid::Text), false),
            (cast(int(), TypeOid::Bool), false),
            (
                cast(Expr::Literal(Literal::Bool(true)), TypeOid::Int4),
                false,
            ),
            (cast(cast(int(), TypeOid::Int8), TypeOid::Numeric), false),
            (Expr::Negate(Box::new(int())), false),
        ] {
            assert_eq!(is_non_null_literal(&expr), expected, "{expr:?}");
        }
    }

    /// The collector evaluates nothing that can fail: a failing constant in a decider's
    /// neighbour, a kept branch, a dropped one or an aggregate's argument is left to the
    /// scalar walk, which raises it where it reaches it.
    #[test]
    fn collection_evaluates_no_constant() {
        let schema = schema();
        for (item, expected) in [
            ("1/0 + sum(a)", 1),
            ("1/0 > 0 OR sum(a) > 0", 1),
            ("CASE WHEN 1/0 > 0 THEN 1 ELSE sum(a) END", 1),
            ("CASE WHEN count(*) > 0 THEN sum(a) ELSE 1/0 END", 2),
            ("CASE WHEN true THEN 1/0 ELSE sum(a) END", 0),
            ("coalesce(1/0, sum(a))", 1),
            ("sum(1/0)", 1),
            ("sum(a + 1/0)", 1),
            ("1 IN (1/0, sum(a))", 1),
        ] {
            assert_eq!(kept(item, &schema), expected, "{item}");
        }
    }

    /// `HAVING` and an `ORDER BY` key are walked like the select list.
    #[test]
    fn every_clause_the_grouping_evaluates_is_collected() {
        let schema = schema();
        for (sql, expected) in [
            ("SELECT max(a) FROM u HAVING true OR sum(1/b) > 0", 1),
            ("SELECT max(a) FROM u HAVING false OR sum(1/b) > 0", 2),
            (
                "SELECT a FROM u GROUP BY a ORDER BY CASE WHEN 1 = 1 THEN count(*) ELSE sum(1/b) END",
                1,
            ),
            (
                "SELECT a FROM u GROUP BY a ORDER BY CASE WHEN 1 = 2 THEN count(*) ELSE sum(1/b) END",
                1,
            ),
            (
                "SELECT a FROM u GROUP BY a ORDER BY CASE WHEN a > 1 THEN count(*) ELSE sum(1/b) END",
                2,
            ),
        ] {
            assert_eq!(collect(sql, &schema).0, expected, "{sql}");
        }
    }

    /// The collector visits each node at most once, whatever the depth: a chain's
    /// decider is carried up from its operands, never re-derived from the top.
    #[test]
    fn a_chain_is_walked_in_time_linear_in_its_depth() {
        fn binary(left: Expr, op: BinaryOp, right: Expr) -> Expr {
            Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            }
        }
        fn node_count(expr: &Expr) -> usize {
            let mut count = 1;
            expr.for_each_child(|child| count += node_count(child));
            count
        }
        let number = || Expr::Literal(Literal::Int(1));
        for levels in [16, 96] {
            for (op, leaf) in [
                // No operand decides: every level's right operand is walked.
                (BinaryOp::And, Expr::Literal(Literal::Bool(true))),
                (BinaryOp::Or, Expr::Literal(Literal::Null)),
                (BinaryOp::Add, number()),
                (BinaryOp::Eq, number()),
            ] {
                let mut chain = leaf.clone();
                for _ in 0..levels {
                    chain = binary(chain, op, leaf.clone());
                }
                let mut collector = Collector::new(ProjectionScope::new(&[], &[], None));
                collector.collect(&chain).expect("collects");
                assert!(
                    collector.visited <= node_count(&chain),
                    "{op:?} chain of {levels}: {} visits of {} nodes",
                    collector.visited,
                    node_count(&chain)
                );
            }
        }
    }
}
