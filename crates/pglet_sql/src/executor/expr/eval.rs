//! The core expression walk: `eval_expr_full`, the per-node evaluators it dispatches to,
//! and the declared-type scope it reads.

use super::arithmetic::{eval_cast, eval_concat};
use super::binary_op::{
    connective_decided_by, eval_binary_op, eval_binary_op_ordered, eval_negate,
};
use super::columns::{ColumnResolver, SimpleColumns};
use super::compare::{boolean_context_operand, eval_not, predicate_holds};
use super::enum_order::{operand_can_be_enum, operands_enum_order};
use super::env::{AGGREGATE_NOT_COMPUTED, EvalEnv, OuterSlot, SubqueryExec};
use super::math_fn::eval_math_fn;
use super::string_fn::{eval_enum_in, eval_string_fn, eval_variadic_fn, values_common_type};
use super::temporal_fn::eval_extract;
use crate::executor::coerce::coerce_value;
use crate::planner::{
    BinaryOp, BoolContext, BuiltinFn, Expr, Literal, Plan, StringFunc, VariadicFunc,
    arithmetic_legality, arithmetic_symbol, binary_op_result_type, math_fn_arg_legality,
    negate_legality,
};
use crate::types::{ArrayValue, JsonbValue, PgNumeric, TypeOid, Value, opt_type_oid_display_name};
use crate::{CatalogError, Error, Result};

// ============================================================================
// Expression Evaluation
// ============================================================================

/// Evaluate an expression with full subquery support (scalar, IN, and EXISTS).
///
/// The subquery-execution callbacks travel together in `sub` (a `Copy` bundle of
/// three pointers), threaded by value through every recursive call. Each field is
/// independently optional, so a caller may enable only a subset (e.g. scalar-only).
pub fn eval_expr_full<R: ColumnResolver>(
    expr: &Expr,
    resolver: &R,
    row: &[Value],
    sub: SubqueryExec<'_>,
    env: EvalEnv<'_>,
) -> Result<Value> {
    eval_expr_typed(expr, resolver, row, &sub, &env, None)
}

/// What the eval walk knows about a node's **declared** type without re-walking it.
///
/// The gates in [`eval_expr_full`] read an *operand's* declared type
/// ([`declared_type_oid`]), and reading it by walking the operand subtree from every
/// enclosing node is quadratic in expression depth — measured 6.4× at depth 32 over
/// 163 840 rows, against a chain of ordinary `int` additions. The verdict does not
/// depend on the row, so the bound to hold is **one derivation per node**: a node that
/// derives its own type hands it up, and its parent's gate reads that instead of
/// re-deriving it.
///
/// [`Self::NotDerived`] is not "un-inferable" — `Derived(None)` is that. It says the
/// walk did not derive this node's type, so a reader asks [`declared_type_oid`], the
/// same function for the same answer at the cost of one walk. An arm that defers is
/// therefore correct at a cost, which is why the arms that derive are the ones that can
/// **chain**; re-derive which those are from the `*out =` writes rather than from a list
/// here. And a node derives only when a reader **asked** — `out` is `None` otherwise —
/// so the shapes that ran through no gate before this existed run through none now
/// (measured: depth 1 unchanged against the pre-gate build).
///
/// A grouping query's expressions take this walk too (`executor::query::aggregate`), so
/// the same bound holds over a chain of operators above an aggregate; an
/// `Expr::Aggregate` itself defers, its declared type one walk over its argument.
#[derive(Debug, Clone, Copy)]
pub enum OperandType {
    /// `Expr::infer_type`'s answer for this node, derived from its operands' types.
    Derived(Option<TypeOid>),
    /// Not derived by the walk — ask [`declared_type_oid`].
    NotDerived,
}

impl OperandType {
    /// This operand's declared type, deriving it now if the walk did not.
    pub fn read(self, expr: &Expr, scope: &dyn crate::planner::LeafScope) -> Option<TypeOid> {
        match self {
            Self::Derived(t) => t,
            Self::NotDerived => declared_type_oid(expr, scope),
        }
    }
}

/// This node's declared type where its **one** operand's type is already in hand.
///
/// `Expr::infer_type`'s own per-node rule (`Expr::infer_type_with`) with the recursion
/// answered from `child_type` for `child` and by the full walk for anything else — so
/// this is not a fourth reader of the walker seam (`docs/declared_types.md`): the rule
/// stays in the one body and only *where its operand's type comes from* moves. The
/// `BinaryOp` sites — the ones on the measured path — do not come through here at all:
/// they call `binary_op_result_type`, that arm's rule lifted out, directly, which is the
/// asymmetry the invariant on *that* function is about.
pub fn derive_node_type(
    expr: &Expr,
    child: &Expr,
    child_type: Option<TypeOid>,
    scope: &dyn crate::planner::LeafScope,
) -> Option<TypeOid> {
    expr.infer_type_with(scope, &|e| {
        if std::ptr::eq(e, child) {
            child_type
        } else {
            e.infer_type(scope)
        }
    })
}

/// The executor's [`LeafScope`](crate::planner::LeafScope) — the two outward-facing leaves of the declared-type
/// walk, answered from an evaluating context rather than from a plan node's
/// materialized descriptors.
///
/// The sibling implementations are `ProjectionScope`'s (which resolves a column
/// against a projecting node's already-built `ColumnDesc`s) and the planner's
/// `PlannerScope` (which resolves against the catalog). This one exists because
/// [`declared_type_oid`] runs *inside* `eval_expr_full`, where neither of those objects
/// is in hand: the column leaf has to take the resolver-plus-frame-chain pairing
/// [`column_declared_type`] owns, and the subquery leaf has to take whatever catalog
/// this evaluation was lent.
struct EvalLeafScope<'a, R: ColumnResolver> {
    resolver: &'a R,
    env: EvalEnv<'a>,
}

impl<R: ColumnResolver> crate::planner::LeafScope for EvalLeafScope<'_, R> {
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        column_declared_type(self.resolver, self.env, table, name)
    }

    /// A scalar subquery is its plan's first output column, through the **planner's**
    /// `Plan::output_columns` — the same route `ProjectionScope` takes, and the same
    /// answer `Describe` reports.
    ///
    /// `None` when this evaluation was lent no catalog: the honest state, and safe,
    /// since `None` is what an un-inferable operand is spelled as on this walk.
    fn subquery_type(&self, plan: &Plan) -> Option<TypeOid> {
        let cols = plan.output_columns(self.env.schema()?);
        cols.first().map(|c| c.type_oid)
    }
}

/// The **declared** `TypeOid` of an expression — the same fact the `RowDescription`
/// declares — or `None` where the walk cannot answer.
///
/// **Not a reader of its own.** It is `Expr::infer_type`, the planner's complete
/// declared-type walk (the one the extended protocol's `Describe` reports), over
/// whichever [`LeafScope`](crate::planner::LeafScope) the caller hands it, so its two
/// outward leaves resolve against the scope of the walk that asked — the scalar eval
/// walk's [`EvalLeafScope`], a grouping query's expressions included. Re-derive which
/// scopes reach it from its callers — [`OperandType::read`] and the `Concat`/`ConcatWs`
/// arm's argument-type read, which builds its own [`EvalLeafScope`].
/// A reader that enumerated expression kinds stood here twice and was wrong both
/// times — first with no arms at all (`conrelid + 1` answered an integer), then with
/// four (`max(conrelid) + 1` and `(SELECT conrelid …) + 1` still did, and
/// `(SELECT max(tz) …)::text` dropped the `+00` offset) — because the set of shapes an
/// `oid` or a `timestamptz` reaches an operand position through is not enumerable from
/// this file.
///
/// The two consumers want the same fact for different reasons, which is why one
/// derivation serves both: `||` and `::text` need the one distinction a `Value` cannot
/// carry (`timestamptz` vs `timestamp`, whose text differs by the `+00` offset), and the
/// declaration-only gates need the type 18.6 resolves its operator against (an `oid`
/// *is* a `Value::Int4`, a `"char"` *is* a `Value::Text`).
///
/// A walk that guesses would be wrong to *gate* on in general — which is what
/// `certain_expr_type_oid` and `certain_operand_type` exist for, and why the comparison
/// and `LIKE` rules still read those. The exemption taken here rests on a reading of
/// `Expr::infer_type`'s non-leaf arms rather than on a measurement: its guessing arms
/// (`arith_result_type`'s operand fallback, the first-branch rule the multi-operand
/// cells take) each *propagate* an operand's type, so of the three types the two
/// consumers care about — `oid`, `"char"`, `timestamptz` — none is synthesized anywhere
/// but a leaf (a column, a cast target, a subquery's output column). **An arm that
/// invented one from operands carrying none would falsify that**, and nothing in the
/// tree compares the two; a new result-type rule over a catalog type is the shape to
/// re-check it against.
fn declared_type_oid(expr: &Expr, scope: &dyn crate::planner::LeafScope) -> Option<TypeOid> {
    expr.infer_type(scope)
}

/// The declared type of a column reference, resolved against the **same scope**
/// [`resolve_column`] takes the value from: the inner scope when it owns the
/// name, else the correlated outer-frame chain.
///
/// Keeping the two in lockstep is the point — pairing an inner value with an
/// outer type (or vice versa) would render one column as another's type. Where
/// the inner scope owns the name but carries no types, the answer stays `None`
/// (value-only rendering) rather than falling through to an outer frame that
/// happens to expose the same name.
///
/// So this takes [`EvalEnv::resolve_outer_slot`]'s **same four-way match**, and the
/// two arms where `resolve_column` raises answer `None` here: a type for a reference
/// the value path refuses is a type belonging to some other column. That pairing is
/// the whole reason these two functions exist rather than one, which is why they move
/// in one commit.
pub(super) fn column_declared_type<R: ColumnResolver>(
    resolver: &R,
    env: EvalEnv<'_>,
    table: Option<&str>,
    name: &str,
) -> Option<TypeOid> {
    if let Some(t) = table {
        if resolver.scope_has_table(t) {
            return resolver.column_type(Some(t), name);
        }
        return match env.resolve_outer_slot(Some(t), name) {
            OuterSlot::Found(frame, i) => frame.type_at(i),
            OuterSlot::Claimed | OuterSlot::Ambiguous => None,
            OuterSlot::Unclaimed => resolver.column_type(Some(t), name),
        };
    }
    match resolver.resolve(None, name) {
        Ok(_) => resolver.column_type(None, name),
        Err(Error::AmbiguousColumn { .. }) => None,
        Err(_) => match env.resolve_outer_slot(None, name) {
            OuterSlot::Found(frame, i) => frame.type_at(i),
            OuterSlot::Claimed | OuterSlot::Ambiguous | OuterSlot::Unclaimed => None,
        },
    }
}

/// [`eval_expr_full`]'s body, reporting this node's declared type in `out` when a
/// reader asked for one (see [`OperandType`]).
///
/// `out` is `None` from every caller that does not read a type, and the arms honour it,
/// so a query with no gate in it derives no types at all.
fn eval_expr_typed<R: ColumnResolver>(
    expr: &Expr,
    resolver: &R,
    row: &[Value],
    sub: &SubqueryExec<'_>,
    env: &EvalEnv<'_>,
    out: Option<&mut OperandType>,
) -> Result<Value> {
    crate::depth::check_stack()?;
    match expr {
        Expr::BinaryOp { left, op, right } => {
            // The operands' types are wanted when a gate below reads them, and when a
            // reader above asked for this node's — and in no other case, so an
            // ungated `a > 0` derives nothing.
            let gated = matches!(op, BinaryOp::Concat) || arithmetic_symbol(*op).is_some();
            let want = gated || out.is_some();
            let mut lty = OperandType::NotDerived;
            let mut rty = OperandType::NotDerived;
            let lval = eval_expr_typed(left, resolver, row, sub, env, want.then_some(&mut lty))?;
            // An `AND` / `OR` whose left operand decides it never evaluates the right —
            // 18.6's `EEOP_BOOL_AND_STEP` / `EEOP_BOOL_OR_STEP` steps of `ExecInterpExpr`
            // (`connective_decided_by`).
            if let Some(decided) = connective_decided_by(*op, &lval)? {
                if let Some(out) = out {
                    *out = OperandType::Derived(Some(TypeOid::Bool));
                }
                return Ok(Value::Bool(decided));
            }
            let rval = eval_expr_typed(right, resolver, row, sub, env, want.then_some(&mut rty))?;
            eval_binary_node(
                left, *op, right, &lval, &rval, lty, rty, want, resolver, *env, out,
            )
        }
        Expr::Cast {
            expr: inner,
            target_type,
        } => {
            let mut ity = OperandType::NotDerived;
            let val = eval_expr_typed(inner, resolver, row, sub, env, Some(&mut ity))?;
            eval_cast_node(expr, inner, val, ity, *target_type, resolver, *env, out)
        }
        Expr::IsNull(inner) => Ok(Value::Bool(
            eval_expr_typed(inner, resolver, row, sub, env, None)?.is_null(),
        )),
        Expr::IsNotNull(inner) => Ok(Value::Bool(
            !eval_expr_typed(inner, resolver, row, sub, env, None)?.is_null(),
        )),
        Expr::Not(inner) => {
            let val = eval_expr_typed(inner, resolver, row, sub, env, None)?;
            eval_not_node(&val)
        }
        // Unary `+` is value-preserving — the node exists only so `+1` is not
        // structurally a bare literal (see `Expr::UnaryPlus`). Postgres additionally
        // gates the operand to the numeric types; pglet does not, so `+'abc'` passes
        // the text through where Postgres raises `42883`.
        //
        // `out` is forwarded rather than re-derived: `Expr::infer_type` answers this
        // node with its operand's own type (the `Negate | UnaryPlus` arm), as it does
        // `Expr::Alias`.
        Expr::UnaryPlus(inner) | Expr::Alias { expr: inner, .. } => {
            eval_expr_typed(inner, resolver, row, sub, env, out)
        }
        // `x IS [NOT] TRUE|FALSE|UNKNOWN` — the three-valued test, whose result is
        // `boolean` for every operand including NULL.
        Expr::BooleanTest { expr: inner, test } => {
            let val = eval_expr_typed(inner, resolver, row, sub, env, None)?;
            eval_boolean_test_node(&val, *test)
        }
        Expr::CastTypmod {
            expr: inner,
            type_oid,
            typmod,
        } => {
            let mut ity = OperandType::NotDerived;
            let val = eval_expr_typed(inner, resolver, row, sub, env, Some(&mut ity))?;
            let target = (*type_oid, *typmod);
            eval_cast_typmod_node(expr, inner, val, ity, target, resolver, *env, out)
        }
        // The chain runs through the left operand of each of the next three; the rest
        // of each node is evaluated in its own frame.
        Expr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let val = eval_expr_typed(inner, resolver, row, sub, env, None)?;
            eval_in_list_node(&val, list, *negated, resolver, row, *sub, *env)
        }
        // The sub-plan runs before the left operand, as 18.6's `ExecHashSubPlan` runs it:
        // an empty result answers without the operand being evaluated, and the
        // sub-plan's error outranks the operand's. `ExecScanSubPlan` evaluates the
        // operand at the sub-plan's first row and stops at the first match; this runs
        // the sub-plan to its end first (`EDGES.md` row 271).
        Expr::InSubquery {
            expr: inner,
            plan,
            negated,
        } => {
            let Some(exec_fn) = sub.in_list else {
                // A position lent no `IN` callback cannot run the sub-plan (`EDGES.md`
                // row 89), so the left operand is evaluated first there, and a `NULL`
                // one answers `NULL` ahead of the refusal — 18.6's answer over a set with
                // a row in it, and not over an empty one (`EDGES.md` row 274).
                let val = eval_expr_typed(inner, resolver, row, sub, env, None)?;
                if val.is_null() {
                    return Ok(Value::Null);
                }
                return Err(Error::NotImplemented(
                    "IN subquery requires execution context".into(),
                ));
            };
            let set = exec_fn(plan)?;
            if set.is_empty() {
                return Ok(Value::Bool(*negated));
            }
            let val = eval_expr_typed(inner, resolver, row, sub, env, None)?;
            eval_in_subquery_node(&val, &set, *negated)
        }
        Expr::AnyAll {
            left,
            op,
            right,
            all,
            sub_select,
        } => {
            let lval = eval_expr_typed(left, resolver, row, sub, env, None)?;
            if *sub_select {
                // Both operands first, so their errors outrank the refusal, as 18.6's
                // own errors in them would: the refusal stands in for none.
                eval_expr_full(right, resolver, row, *sub, *env)?;
                return Err(Error::NotImplemented(
                    "ANY/ALL over a subquery is not supported".into(),
                ));
            }
            eval_any_all_node([left, right], *op, *all, &lval, resolver, row, *sub, *env)
        }
        _ => eval_expr_rest(expr, resolver, row, *sub, *env, out),
    }
}

/// A typmod'd explicit cast (`x::varchar(3)`, `x::numeric(4,1)`) over its evaluated
/// operand: coerce to the target type, then apply the modifier with `is_explicit =
/// true` (a length over-run truncates silently; a numeric overflow still fails loud).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn eval_cast_typmod_node<R: ColumnResolver>(
    expr: &Expr,
    inner: &Expr,
    val: Value,
    ity: OperandType,
    (type_oid, typmod): (TypeOid, crate::planner::Typmod),
    resolver: &R,
    env: EvalEnv<'_>,
    out: Option<&mut OperandType>,
) -> Result<Value> {
    let scope = EvalLeafScope { resolver, env };
    let ity_oid = ity.read(inner, &scope);
    if let Some(out) = out {
        *out = OperandType::Derived(derive_node_type(expr, inner, ity_oid, &scope));
    }
    // Through `eval_cast` so a `timestamptz::varchar(n)` keeps its `+00` offset before
    // the length modifier applies, exactly as the plain `::text` form does.
    let coerced = eval_cast(val, ity_oid, type_oid)?;
    super::super::coerce::apply_typmod(coerced, type_oid, typmod, true)
}

/// `val [NOT] IN (list)`, `val` being the evaluated left operand.
#[inline(never)]
fn eval_in_list_node<R: ColumnResolver>(
    val: &Value,
    list: &[Expr],
    negated: bool,
    resolver: &R,
    row: &[Value],
    sub: SubqueryExec<'_>,
    env: EvalEnv<'_>,
) -> Result<Value> {
    // A NULL operand makes the answer NULL, a list never being empty, but the items are
    // still evaluated, as 18.6 evaluates each one over an operand its planner has not
    // folded to a constant NULL (`EDGES.md` row 273): an item's error is raised. An item
    // [`refused_by_this_walk`] holds is skipped: its refusal is pglet's own, and 18.6's
    // answer is this `NULL` unless the item raises there (`EDGES.md` row 274).
    if val.is_null() {
        for item in list {
            if !refused_by_this_walk(item, sub) {
                eval_expr_full(item, resolver, row, sub, env)?;
            }
        }
        return Ok(Value::Null);
    }
    let mut found_null = false;
    for item in list {
        let item_val = eval_expr_full(item, resolver, row, sub, env)?;
        if item_val.is_null() {
            found_null = true;
            continue;
        }
        // Compare using equality (an eval error propagates, not swallowed).
        if matches!(
            eval_binary_op(BinaryOp::Eq, val, &item_val)?,
            Value::Bool(true)
        ) {
            return Ok(Value::Bool(!negated));
        }
    }
    // SQL: if the list holds a NULL and nothing matched, the result is NULL.
    if found_null {
        Ok(Value::Null)
    } else {
        Ok(Value::Bool(negated))
    }
}

/// Whether `expr` holds a window function, which another executor evaluates, an
/// aggregate where `sub` lends no aggregate values, or a subquery of a kind `sub` lends no
/// callback for (`EDGES.md` row 89). A sequence function or a set-returning `unnest`, which
/// this walk refuses too, is not skipped: its refusal stands under a `NULL` operand
/// (`EDGES.md` row 180). A sub-plan is a leaf here, as in [`Expr::for_each_child`], and so
/// is an aggregate, whose argument this walk never evaluates.
///
/// Where `sub` does lend aggregate values, an aggregate is not refused here: the grouping
/// executor lends a value for every aggregate of a branch it kept, and the ones it lends
/// none for sit in a branch it dropped (`executor::query::aggregate`'s `Collector`),
/// which this walk does not take either.
fn refused_by_this_walk(expr: &Expr, sub: SubqueryExec<'_>) -> bool {
    let refused = match expr {
        Expr::Aggregate { .. } => return sub.aggregate.is_none(),
        Expr::Window { .. } => true,
        Expr::Subquery { .. } => sub.scalar.is_none(),
        Expr::Exists { .. } => sub.exists.is_none(),
        Expr::InSubquery { .. } => sub.in_list.is_none(),
        _ => false,
    };
    let mut found = refused;
    expr.for_each_child(|child| found = found || refused_by_this_walk(child, sub));
    found
}

/// `val [NOT] IN (subquery)` over the sub-plan's non-empty `set`, `val` being the
/// evaluated left operand. The empty set is the caller's: `false` for `IN` and `true`
/// for `NOT IN` whatever the operand, because no comparison is made.
#[inline(never)]
fn eval_in_subquery_node(val: &Value, set: &[Value], negated: bool) -> Result<Value> {
    // Over a set with a row in it, every comparison with a NULL operand is NULL.
    if val.is_null() {
        return Ok(Value::Null);
    }
    let mut found_null = false;
    for sv in set {
        if sv.is_null() {
            found_null = true;
            continue;
        }
        if matches!(eval_binary_op(BinaryOp::Eq, val, sv)?, Value::Bool(true)) {
            return Ok(Value::Bool(!negated));
        }
    }
    if found_null {
        Ok(Value::Null)
    } else {
        Ok(Value::Bool(negated))
    }
}

/// `lval op ANY|ALL (right)`, `lval` being the evaluated left operand.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn eval_any_all_node<R: ColumnResolver>(
    [left, right]: [&Expr; 2],
    op: BinaryOp,
    all: bool,
    lval: &Value,
    resolver: &R,
    row: &[Value],
    sub: SubqueryExec<'_>,
    env: EvalEnv<'_>,
) -> Result<Value> {
    let rval = eval_expr_full(right, resolver, row, sub, env)?;
    // The four ordering operators only, as in the `BinaryOp` arm. The gate also keeps
    // the resolver walk off `= ANY($1)`, the spine's bulk-lookup shape.
    //
    // **Both operands**, as in that arm: `eval_any_all` compares the scalar side against
    // each element, so an enum reached only through the array side
    // (`'ok' > ANY(ARRAY[m])`) types the comparison exactly as one on the scalar side
    // does. It read the scalar side alone while `expr_enum_with`'s `Array` arm answered
    // `None`, which made the array side unreadable and the comparison label-byte
    // ordered — a silent wrong `DELETE` among the measured spellings.
    let enum_order = (matches!(
        op,
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    ) && operand_can_be_enum(lval)
        && operand_can_be_enum(&rval))
    .then(|| operands_enum_order([left, right], resolver, env))
    .flatten();
    eval_any_all(op, lval, &rval, all, enum_order)
}

/// `x IS [NOT] TRUE|FALSE|UNKNOWN` over its evaluated operand — the three-valued test,
/// whose result is `boolean` for every operand including NULL.
#[inline(never)]
fn eval_boolean_test_node(val: &Value, test: crate::planner::BoolTest) -> Result<Value> {
    Ok(Value::Bool(test.apply(boolean_context_operand(
        val,
        BoolContext::Test(test),
    )?)))
}

/// `NOT` over its evaluated operand, out of [`eval_expr_typed`]'s frame.
#[inline(never)]
fn eval_not_node(val: &Value) -> Result<Value> {
    eval_not(val)
}

/// A cast over its evaluated operand: the operand's declared type read once, the
/// node's own derived for a reader that asked, then the cast.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn eval_cast_node<R: ColumnResolver>(
    expr: &Expr,
    inner: &Expr,
    val: Value,
    ity: OperandType,
    target_type: TypeOid,
    resolver: &R,
    env: EvalEnv<'_>,
    out: Option<&mut OperandType>,
) -> Result<Value> {
    let scope = EvalLeafScope { resolver, env };
    let ity_oid = ity.read(inner, &scope);
    if let Some(out) = out {
        *out = OperandType::Derived(derive_node_type(expr, inner, ity_oid, &scope));
    }
    eval_cast(val, ity_oid, target_type)
}

/// A binary operator over its evaluated operands: the declared-type gates, then the
/// operator, then the node's own declared type for a reader that asked.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn eval_binary_node<R: ColumnResolver>(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    lval: &Value,
    rval: &Value,
    lty: OperandType,
    rty: OperandType,
    want: bool,
    resolver: &R,
    env: EvalEnv<'_>,
    out: Option<&mut OperandType>,
) -> Result<Value> {
    // Each operand's declared type is read at most once, here.
    let (lt, rt) = if want {
        let scope = EvalLeafScope { resolver, env };
        (lty.read(left, &scope), rty.read(right, &scope))
    } else {
        (None, None)
    };
    // `||` stringifies its operands, so it needs their *declared* types:
    // `timestamptz` and `timestamp` share `Value::Timestamp` and differ only
    // in whether the text carries `+00`. Dropping the offset would silently
    // shift the instant for any driver that reparses in a local zone — the
    // same hazard `value_to_text_with_oid` exists to close on the wire.
    //
    // Arithmetic needs the *declared* types too, and for a second reason:
    // `eval_binary_op` sees only the value pair, and `oid` / `"char"` borrow
    // another type's value shape, so `conrelid + 1` reaches `eval_arithmetic`
    // as `Int4 + Int4` and answers where 18.6 raises.
    //
    // Two sites call the one rulebook for this: this seam and the plan-time
    // `check_expr_node_operand_types`. The arithmetic above an aggregate reaches
    // this seam too, a grouping query's expressions taking this walk, so
    // `max(conrelid) + 1` is gated here as `conrelid + 1` is.
    let result = if matches!(op, BinaryOp::Concat) {
        eval_concat(lval, lt, rval, rt)
    } else {
        // An enum orders by member position, so `<`/`<=`/`>`/`>=` need the
        // ordered member list of whichever operand is enum-typed. Resolved only
        // for those four operators — `=`/`<>` are label-byte equality, which is
        // already 18.6's OID identity (see `eval_binary_op_ordered`).
        let enum_order = (matches!(
            op,
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) && operand_can_be_enum(lval)
            && operand_can_be_enum(rval))
        .then(|| operands_enum_order([left, right], resolver, env))
        .flatten();
        if arithmetic_symbol(op).is_some() {
            // The value's own type backs the declared one up, so an integer
            // literal — which declares nothing — is named `integer` in the
            // message rather than 18.6's-`unknown`-but-not-here.
            arithmetic_legality(
                lt.or_else(|| lval.type_oid()),
                rt.or_else(|| rval.type_oid()),
                op,
            )?;
        }
        eval_binary_op_ordered(op, lval, rval, enum_order)
    };
    // The node's own declared type, from its operands' — so an enclosing gate
    // reads it instead of re-walking this subtree (see [`OperandType`]), and
    // through the one cell rather than the whole-`Expr` match.
    if let Some(out) = out {
        *out = OperandType::Derived(binary_op_result_type(op, lt, rt));
    }
    result
}

/// [`eval_expr_typed`]'s arms for every node it does not evaluate itself — the
/// chain-forming ones are there, so a chain costs that lean frame a level
/// (`crate::depth`).
#[inline(never)]
fn eval_expr_rest<R: ColumnResolver>(
    expr: &Expr,
    resolver: &R,
    row: &[Value],
    sub: SubqueryExec<'_>,
    env: EvalEnv<'_>,
    out: Option<&mut OperandType>,
) -> Result<Value> {
    match expr {
        Expr::Literal(lit) => {
            // The literal's own rule, the one `Expr::infer_type` reads for this arm.
            if let Some(out) = out {
                *out = OperandType::Derived(lit.infer_type());
            }
            Ok(literal_to_value(lit))
        }
        Expr::Column { table, name } => resolve_column(resolver, env, row, table.as_deref(), name),
        Expr::Negate(inner) => {
            let mut ity = OperandType::NotDerived;
            let val = eval_expr_typed(inner, resolver, row, &sub, &env, Some(&mut ity))?;
            // Same declared-type reason as the arithmetic gate above: `- conrelid`
            // reaches `eval_negate` as an `Int4`, and so does `- max(conrelid)`.
            let scope = EvalLeafScope { resolver, env };
            let ity_oid = ity.read(inner, &scope);
            negate_legality(ity_oid)?;
            if let Some(out) = out {
                *out = OperandType::Derived(derive_node_type(expr, inner, ity_oid, &scope));
            }
            eval_negate(&val)
        }
        // Walked by `eval_expr_typed` itself: never here.
        Expr::BinaryOp { .. }
        | Expr::UnaryPlus(_)
        | Expr::Not(_)
        | Expr::IsNull(_)
        | Expr::IsNotNull(_)
        | Expr::BooleanTest { .. }
        | Expr::Cast { .. }
        | Expr::CastTypmod { .. }
        | Expr::InList { .. }
        | Expr::InSubquery { .. }
        | Expr::AnyAll { .. }
        | Expr::Alias { .. } => eval_expr_typed(expr, resolver, row, &sub, &env, out),
        // `DEFAULT` is valid only in an INSERT/UPDATE *value* position, where the
        // write path resolves it against the column's catalog default before it
        // ever reaches general expression eval (see `executor/dml.rs`). Reaching
        // here means `DEFAULT` appeared in a read context (`SELECT default`),
        // which Postgres rejects — so fail loud instead of silently yielding NULL.
        Expr::Default => Err(Error::syntax(super::super::DEFAULT_NOT_ALLOWED.to_string())),
        // `CASE`: return the first matching branch's result (searched: a TRUE
        // condition; simple: operand equals the WHEN value), else the ELSE result
        // or NULL. Only the matching branch's result is evaluated.
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            let operand_val = match operand {
                Some(op) => Some(eval_expr_full(op, resolver, row, sub, env)?),
                None => None,
            };
            for (condition, result) in when_clauses {
                let cond_val = eval_expr_full(condition, resolver, row, sub, env)?;
                // The simple form (`CASE x WHEN v`) is Postgres's `x = v`, so it goes
                // through `eval_binary_op` rather than a bare `values_equal` — that
                // is what applies the operand-type gate, making
                // `CASE text_col WHEN 5` fail `42883` as Postgres does.
                let matched = match operand_val.as_ref() {
                    Some(op_val) => matches!(
                        eval_binary_op(BinaryOp::Eq, op_val, &cond_val)?,
                        Value::Bool(true)
                    ),
                    None => predicate_holds(&cond_val, BoolContext::CaseWhen)?,
                };
                if matched {
                    return eval_expr_full(result, resolver, row, sub, env);
                }
            }
            else_result.as_deref().map_or(Ok(Value::Null), |else_expr| {
                eval_expr_full(else_expr, resolver, row, sub, env)
            })
        }
        // The value the grouping executor computed for this node over the current group
        // (`SubqueryExec::aggregate`). A position lent no such value — one no grouping
        // executor evaluates — is refused.
        Expr::Aggregate { .. } => sub
            .aggregate
            .and_then(|value_of| value_of(expr))
            .ok_or_else(|| Error::NotImplemented(AGGREGATE_NOT_COMPUTED.into())),
        Expr::Subquery { plan } => sub.scalar.map_or_else(
            || {
                Err(Error::NotImplemented(
                    "scalar subquery requires execution context".into(),
                ))
            },
            |exec_fn| exec_fn(plan),
        ),
        // `[NOT] EXISTS(...)`: run the sub-plan, then XOR row-presence with the
        // `negated` flag (`found ^ negated`) so `EXISTS` is true on a non-empty
        // result and `NOT EXISTS` is its negation. Always a concrete boolean.
        Expr::Exists { plan, negated } => sub.exists.map_or_else(
            || {
                Err(Error::NotImplemented(
                    "EXISTS subquery requires execution context".into(),
                ))
            },
            |exec_fn| Ok(Value::Bool(exec_fn(plan)? ^ negated)),
        ),
        Expr::Window { .. } => Err(Error::NotImplemented(
            "window functions must be evaluated via Window plan executor".into(),
        )),
        // Resolve `$N` against the evaluation environment at eval time (the
        // env-binding model — no plan mutation). Empty env → fails loud.
        Expr::Parameter { index } => env.param(*index),
        Expr::Bound(v) => Ok(v.clone()),
        Expr::Builtin(func) => eval_builtin(*func),
        Expr::Array(elems) => {
            let mut values = Vec::with_capacity(elems.len());
            for e in elems {
                values.push(eval_expr_full(e, resolver, row, sub, env)?);
            }
            build_array_value(values)
        }
        Expr::JsonbBuildObject(args) => {
            let mut values = Vec::with_capacity(args.len());
            for e in args {
                values.push(eval_expr_full(e, resolver, row, sub, env)?);
            }
            build_jsonb_object(values)
        }
        // `unnest(arr)` is a set-returning function: it row-expands and is only
        // legal inside a projection (lowered to `Plan::ProjectUnnest`, which
        // evaluates the inner array directly). Reaching it here means it appeared
        // in a scalar position (a WHERE/ON predicate, an arithmetic operand) —
        // unsupported, fail loud rather than silently collapse to one value.
        Expr::Unnest(_) => Err(Error::NotImplemented(
            "set-returning unnest() is only supported in a SELECT projection".into(),
        )),
        // `LOWER(text)` — lowercase the (text) operand; NULL → NULL. A non-text,
        // non-null operand resolves to no function (`42883`, like Postgres and like
        // `upper`), rather than silently passing through.
        Expr::Lower(inner) => {
            let val = eval_expr_full(inner, resolver, row, sub, env)?;
            match val {
                Value::Null => Ok(Value::Null),
                Value::Text(s) => Ok(Value::Text(s.to_lowercase())),
                other => Err(Error::UndefinedFunction(format!(
                    "function lower does not exist for argument type {}",
                    opt_type_oid_display_name(other.type_oid())
                ))),
            }
        }
        Expr::MathFn { func, args } => {
            // `Expr::infer_type` reads the first argument's type and no other, so only
            // that one is threaded; a second argument (`round(x, n)`) is gated below
            // through the full walk, as every argument was before.
            let mut first_ty = OperandType::NotDerived;
            let mut vals = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                let mut ty = OperandType::NotDerived;
                vals.push(eval_expr_typed(
                    a,
                    resolver,
                    row,
                    &sub,
                    &env,
                    (i == 0).then_some(&mut ty),
                )?);
                if i == 0 {
                    first_ty = ty;
                }
            }
            // `abs(conrelid)` is `42883 function abs(oid) does not exist` on 18.6, and
            // reaches `eval_math_fn` here as an `Int4` for the same reason arithmetic
            // does — the declared type is the only thing that can tell them apart.
            let scope = EvalLeafScope { resolver, env };
            for (i, arg) in args.iter().enumerate() {
                let ty = if i == 0 {
                    first_ty
                } else {
                    OperandType::NotDerived
                };
                math_fn_arg_legality(*func, ty.read(arg, &scope))?;
            }
            if let Some(out) = out {
                *out = OperandType::Derived(args.first().map_or_else(
                    || expr.infer_type(&scope),
                    |first| derive_node_type(expr, first, first_ty.read(first, &scope), &scope),
                ));
            }
            eval_math_fn(*func, &vals)
        }
        // Strict-NULL string function — the null check is hoisted in `eval_string_fn`.
        Expr::StringFn { func, args } => {
            // The conversion to an enum reads its operand alone: the name and the member
            // labels are the node's own literals, compared in place rather than
            // evaluated into values once per row.
            if matches!(func, StringFunc::EnumIn)
                && let Some((operand, name, labels)) = crate::planner::enum_in_arg_parts(args)
            {
                let value = eval_expr_full(operand, resolver, row, sub, env)?;
                return eval_enum_in(value, name, |label| {
                    crate::planner::enum_in_holds(labels, label)
                });
            }
            let vals = args
                .iter()
                .map(|a| eval_expr_full(a, resolver, row, sub, env))
                .collect::<Result<Vec<_>>>()?;
            eval_string_fn(*func, &vals, env.schema())
        }
        // Non-strict variadic function — each applies its own NULL rule.
        Expr::VariadicFn { func, args } => {
            let vals = args
                .iter()
                .map(|a| eval_expr_full(a, resolver, row, sub, env))
                .collect::<Result<Vec<_>>>()?;
            // `GREATEST`/`LEAST` order their arguments, and no other shape reaches
            // `greatest_least`, so this is the one site that resolves the order for it.
            let enum_order = (matches!(func, VariadicFunc::Greatest | VariadicFunc::Least)
                && vals.iter().any(operand_can_be_enum))
            .then(|| operands_enum_order(args.iter(), resolver, env))
            .flatten();
            // `concat`/`concat_ws` render each argument through its type's output
            // function, and a `timestamptz` — scalar or array element — shares
            // `Value::Timestamp` with `timestamp`, so only the declared type says it
            // carries `+00`.
            let arg_types = (matches!(func, VariadicFunc::Concat | VariadicFunc::ConcatWs)
                && vals
                    .iter()
                    .any(|v| matches!(v, Value::Timestamp(_) | Value::Array(_))))
            .then(|| {
                let scope = EvalLeafScope { resolver, env };
                args.iter()
                    .map(|a| declared_type_oid(a, &scope))
                    .collect::<Vec<_>>()
            });
            eval_variadic_fn(*func, &vals, enum_order, arg_types.as_deref())
        }
        // `EXTRACT(field FROM source)` / `date_part(field, source)` — a NULL field or
        // source propagates to NULL (strict), else the resolved field is pulled from
        // the temporal source (numeric for extract, float8 for date_part).
        Expr::Extract {
            source,
            field,
            retnumeric,
        } => {
            let field_val = eval_expr_full(field, resolver, row, sub, env)?;
            let source_val = eval_expr_full(source, resolver, row, sub, env)?;
            eval_extract(&field_val, &source_val, *retnumeric)
        }
        // `COALESCE(a, b, …)` — return the first argument that evaluates non-NULL,
        // else NULL. Short-circuits: later args are not evaluated once one is found.
        Expr::Coalesce(args) => {
            for arg in args {
                let val = eval_expr_full(arg, resolver, row, sub, env)?;
                if !val.is_null() {
                    return Ok(val);
                }
            }
            Ok(Value::Null)
        }
        // A sequence function mutates sequence / reads per-session state, so it is
        // evaluated at the statement level (`execute_sequence_projection`), never in
        // this read-only path. Reaching here means it appeared in a nested position
        // (a WHERE/ON predicate, an arithmetic operand, a subquery) — fail loud
        // rather than silently produce a value the surrounding read can't back.
        Expr::SequenceFn { func, .. } => Err(Error::NotImplemented(format!(
            "{}() is only supported as a top-level SELECT expression (and in INSERT … VALUES)",
            func.name()
        ))),
    }
}

/// Resolve an `Expr::Column` against the inner scope first, then the correlated
/// outer scope. Preserves the alias-aware precedence (exact-qualified →
/// qualifier-is-scope-alias → correlated-outer → inner resolve/error): an outer
/// reference is consulted only when the inner scope doesn't own the qualifier /
/// bare name, and it takes precedence over the inner scope's final resolve. For a
/// single-table-identity resolver a *qualifier* naming neither this scope nor any
/// outer frame is a `42P01` (missing FROM-clause entry), never a loose match; a
/// no-identity resolver still bare-matches (it has no FROM scope to validate
/// against). Which resolvers carry an identity is a property of their call sites and
/// not of this function. The DML scans pass one: `execute_delete` passes the target's
/// name and `execute_update` passes `alias.unwrap_or(table)`, so neither an aliased
/// nor an unaliased target reaches here with `None`. Callers that still pass `None`
/// are described at tier 3 of `SimpleColumns::resolve` (the no-FROM / `VALUES` path
/// and the post-projection resolvers among them), and what that costs is measured in
/// `EDGES.md` (*Qualifier-blind name resolution*).
///
/// Two arms of the outer verdict raise rather than fall through, and both used to
/// decide which rows a statement wrote. [`OuterSlot::Claimed`] — a frame owns the
/// qualifier and lacks the column — is `42703 "t.name"`, where the reference used to
/// rebind to another frame's bare column. And an ambiguity is **final at the level
/// that claims it**, inner scope included: `colNameToVar` raises `42702` at the
/// innermost claiming level and never walks past it, so an `Err(AmbiguousColumn)`
/// from the inner resolver is returned rather than escalated to the outer chain — it
/// used to escalate, and over `o/c/d` all carrying `v`, `DELETE FROM o WHERE EXISTS
/// (SELECT 1 FROM c JOIN d ON c.id = d.id WHERE v = 1)` deleted the outer row where
/// 18.6 answers `42702`.
///
/// With `env.outer == None` (no correlation) this is byte-identical to a plain
/// `resolver.resolve` — the outer chain is empty, so every lookup is `Unclaimed`.
fn resolve_column<R: ColumnResolver>(
    resolver: &R,
    env: EvalEnv<'_>,
    row: &[Value],
    table: Option<&str>,
    name: &str,
) -> Result<Value> {
    let at = |i: usize| row.get(i).cloned().unwrap_or(Value::Null);
    if let Some(t) = table {
        // The qualifier names this inner scope → it owns resolution (exact-
        // qualified or alias tier); a miss here is a genuine 42703.
        if resolver.scope_has_table(t) {
            return resolver.resolve(Some(t), name).map(at);
        }
        // The qualifier names no inner scope → the outer chain decides (the
        // cell-authz `c.id` case), and its verdict is final where a frame owns `t`.
        match env.resolve_outer_slot(Some(t), name) {
            OuterSlot::Found(frame, i) => Ok(frame.value_at(i)),
            OuterSlot::Claimed => Err(CatalogError::ColumnNotFoundInExpr {
                column: format!("{t}.{name}"),
            }
            .into()),
            OuterSlot::Ambiguous => Err(Error::AmbiguousColumn {
                column: name.to_string(),
            }),
            // The qualifier is unknown to this scope and to every outer frame. A
            // single-table-identity resolver raises `42P01` (missing FROM-clause
            // entry); a no-identity one (`ORDER BY` / VALUES) bare-matches.
            OuterSlot::Unclaimed => resolver.resolve(Some(t), name).map(at),
        }
    } else {
        // Unqualified: the inner scope wins if it has the bare name (standard
        // SQL shadowing)…
        match resolver.resolve(None, name) {
            Ok(i) => Ok(at(i)),
            // …an ambiguity *at this level* is the answer, not a reason to look
            // further out…
            Err(e @ Error::AmbiguousColumn { .. }) => Err(e),
            // …else a correlated outer reference (the orphan-fact `f.hash` /
            // bare-name case), else the inner scope's own 42703.
            Err(inner) => match env.resolve_outer_slot(None, name) {
                OuterSlot::Found(frame, i) => Ok(frame.value_at(i)),
                OuterSlot::Ambiguous => Err(Error::AmbiguousColumn {
                    column: name.to_string(),
                }),
                OuterSlot::Claimed | OuterSlot::Unclaimed => Err(inner),
            },
        }
    }
}

/// Build a `Value::Array` from evaluated element values.
///
/// The element type is the **common supertype** of the *concrete* (non-null,
/// non-text) elements (widest numeric, text-family unification — [`values_common_type`],
/// folding [`common_type_step`](crate::planner::common_type_step)); a `Value::Text`
/// element is treated as a coercible wildcard (by eval time an untyped string literal
/// has collapsed to `Value::Text`, so it adopts the concrete peers' type), and every
/// element — text included — is then coerced toward that type. When every element is
/// text or NULL the array is `text[]` (all-null → `text[]`). So `ARRAY[1, 2.5]` widens
/// to `numeric[]` (`{1,2.5}`, no `2.5`→`3` truncation), `ARRAY[1::int8, 2::int4]` to
/// `bigint[]`, and `ARRAY[1, '2']` / `ARRAY['2026-01-01'::date, '2026-02-02']` keep the
/// concrete peer's type (the string coerces toward it) — matching Postgres's
/// `select_common_type` over unknown-typed literals. A string that can't coerce
/// (`ARRAY[1, 'x']`) still fails loud (`22P02`/`42804`).
fn build_array_value(elements: Vec<Value>) -> Result<Value> {
    // **Multidimensional arrays are not supported**, and this is the backstop that
    // makes that a loud failure rather than a silent reinterpretation. pglet's
    // `ArrayValue` is one-dimensional by construction (`element_type` is a scalar
    // `TypeOid`), so an array element used to be coerced to `text` — rendering
    // `ARRAY[ARRAY[1,2],ARRAY[3,4]]` as `{"{1,2}","{3,4}"}` where Postgres gives
    // `{{1,2},{3,4}}`, and declaring it `text`/`text[]` depending on which walker
    // was asked.
    //
    // The planner rejects the statically-typed shape (`branch_unification_legality`);
    // this catches the rest, where the element type is only known once evaluated —
    // `ARRAY[(SELECT ARRAY[1,2])]`, `ARRAY[array_col]`. Deliberately pglet-stricter
    // than Postgres, which accepts all of these.
    if elements.iter().any(|v| matches!(v, Value::Array(_))) {
        return Err(Error::NotImplemented(
            "multidimensional arrays are not supported".into(),
        ));
    }
    // Only the concrete (non-text, non-null) elements drive the common type; a text
    // element is a wildcard that coerces to whatever they resolve to.
    let concrete: Vec<&Value> = elements
        .iter()
        .filter(|v| !v.is_null() && !matches!(v, Value::Text(_)))
        .collect();
    let element_type = values_common_type(&concrete, "ARRAY")?.unwrap_or(TypeOid::Text);
    let coerced = elements
        .into_iter()
        .map(|e| coerce_value(e, element_type))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Array(ArrayValue::new(element_type, coerced)))
}

/// Build a `Value::Jsonb(Object)` from `jsonb_build_object`'s evaluated arguments.
///
/// `values` is the flat `[k1, v1, k2, v2, …]` list (even length, enforced at
/// conversion). Each key must be non-null text (Postgres: "argument N: key must
/// not be null"); each value is mapped to a `JsonbValue` via `value_to_jsonb`,
/// with a NULL value kept as a `JsonbValue::Null` member (matching PG). Keys are
/// inserted in argument order.
fn build_jsonb_object(values: Vec<Value>) -> Result<Value> {
    let mut pairs: Vec<(String, JsonbValue)> = Vec::with_capacity(values.len() / 2);
    let mut iter = values.into_iter();
    let mut arg_index = 1;
    while let Some(key) = iter.next() {
        let key_str = match key {
            Value::Text(s) => s,
            Value::Null => {
                return Err(Error::Execution(format!(
                    "argument {arg_index}: key must not be null"
                )));
            }
            _ => {
                // PG requires the key to be text; reject anything else loudly
                // rather than silently coercing.
                return Err(Error::Type(format!(
                    "jsonb_build_object key (argument {arg_index}) must be text"
                )));
            }
        };
        // Even length is guaranteed by the planner, so `next()` is always `Some`.
        let value = iter
            .next()
            .ok_or_else(|| Error::Execution("jsonb_build_object: odd argument count".into()))?;
        pairs.push((key_str, value_to_jsonb(&value)));
        arg_index += 2;
    }
    // Canonicalized like every other jsonb producer: `jsonb_build_object` returns
    // **jsonb**, and last-wins key dedup is a jsonb invariant, not a property of
    // the text parser. Without it `jsonb_build_object('a',1,'a',2)` built the
    // shape `{"a": 1, "a": 2}` that no `::jsonb` cast can produce — and the
    // first-match `->`/`?`/`@>` readers then disagreed with the text path's
    // last-wins, an internal inconsistency on top of the divergence from
    // Postgres's `{"a": 2}`.
    Ok(Value::Jsonb(JsonbValue::Object(pairs).canonical()))
}

/// Map a scalar `Value` to a `JsonbValue` per Postgres `to_jsonb` semantics.
///
/// The load-bearing case is text: a text value becomes a JSON *string* (so
/// `jsonb_build_object('kind', 'doc')` is `{"kind": "doc"}`, value quoted), it is
/// **not** parsed as JSON. An existing `Value::Jsonb` passes through unchanged.
/// Booleans and numbers map to their JSON kinds; `Null` is a JSON null. Other
/// scalars (uuid/timestamp/bytea) render through their text-output form as a JSON
/// string, matching PG's `to_jsonb` text rendering — the spine only needs
/// text/null, but this keeps the rest correct rather than lossy.
///
/// Numbers store as an exact [`PgNumeric`] (Postgres `datum_to_jsonb`), so a big
/// integer or scale-carrying decimal round-trips without loss — `1.5::numeric`
/// becomes the JSON *number* `1.5`, not the string `"1.5"` the old f64 fallback
/// produced. A non-finite float/numeric (`NaN`/`±Infinity`) has no JSON number
/// form, so Postgres emits it as a JSON *string* (`"NaN"`/`"Infinity"`); pglet
/// matches via [`float_to_jsonb`].
fn value_to_jsonb(value: &Value) -> JsonbValue {
    match value {
        Value::Null => JsonbValue::Null,
        Value::Bool(b) => JsonbValue::Bool(*b),
        // Integers map to an exact `PgNumeric` — arbitrary precision, no f64 rounding.
        Value::Int2(n) => JsonbValue::Number(PgNumeric::from_i128(i128::from(*n))),
        Value::Int4(n) => JsonbValue::Number(PgNumeric::from_i128(i128::from(*n))),
        Value::Int8(n) => JsonbValue::Number(PgNumeric::from_i128(i128::from(*n))),
        Value::Float4(n) => float_to_jsonb(f64::from(*n)),
        Value::Float8(n) => float_to_jsonb(*n),
        // A NaN numeric has no JSON number form (Postgres strings it); a finite
        // numeric keeps its exact value and display scale (`1.50` stays `1.50`).
        Value::Numeric(n) if n.is_nan() => JsonbValue::String("NaN".to_string()),
        Value::Numeric(n) => JsonbValue::Number(n.clone()),
        Value::Text(s) => JsonbValue::String(s.clone()),
        Value::Jsonb(j) => j.clone(),
        // uuid / timestamp / bytea / array → PG renders these to_jsonb as a JSON
        // string of their text output. `value_to_text` is None only for Null,
        // already handled above, so the fallback is defensive.
        other => JsonbValue::String(crate::types::value_to_text(other).unwrap_or_default()),
    }
}

/// Map an `f64` to a `JsonbValue` per Postgres `datum_to_jsonb`: a finite float is
/// an exact JSON number; `NaN`/`±Infinity` have no JSON number form, so Postgres
/// emits the JSON *string* `"NaN"`/`"Infinity"`/`"-Infinity"` (the old code wrote a
/// bare `NaN`/`inf` — invalid JSON). `from_f64` only errors on non-finite input,
/// already handled here, so the finite branch can't fail.
fn float_to_jsonb(f: f64) -> JsonbValue {
    if f.is_nan() {
        JsonbValue::String("NaN".to_string())
    } else if f.is_infinite() {
        JsonbValue::String(if f > 0.0 { "Infinity" } else { "-Infinity" }.to_string())
    } else {
        // `f` is finite, so `from_f64` cannot fail; the `Err` fallback (the shortest
        // decimal spelling) is defensive and unreachable.
        PgNumeric::from_f64(f)
            .map_or_else(|_| JsonbValue::String(f.to_string()), JsonbValue::Number)
    }
}

/// Evaluate `left op ANY(array)` / `left op ALL(array)`.
///
/// An **empty** array short-circuits to ANY=false / ALL=true (vacuously) *before*
/// the NULL-left check, matching Postgres: `NULL = ANY('{}')` is `false` and
/// `NULL = ALL('{}')` is `true`, not NULL — the quantifier over zero elements never
/// evaluates the comparison, so the unknown left operand is irrelevant. A NULL left
/// over a **non-empty** array still yields NULL. NULL or non-array right → NULL /
/// error. ANY is true if any element compares true (false if none and no NULLs, else
/// NULL); ALL is false if any element compares false (true if none and no NULLs, else
/// NULL).
fn eval_any_all(
    op: BinaryOp,
    left: &Value,
    right: &Value,
    all: bool,
    enum_order: Option<&[String]>,
) -> Result<Value> {
    let arr = match right {
        Value::Null => return Ok(Value::Null),
        Value::Array(a) => a,
        _ => {
            return Err(Error::NotImplemented(
                "ANY/ALL requires an array on the right-hand side".into(),
            ));
        }
    };
    // An empty array is vacuous: ANY→false, ALL→true, regardless of the left operand
    // (even NULL). Checked before the NULL-left short-circuit so `NULL = ANY('{}')`
    // is `false`, not NULL.
    if arr.elements.is_empty() {
        return Ok(Value::Bool(all));
    }
    if left.is_null() {
        return Ok(Value::Null);
    }
    // TODO: only the array RHS form is built (`x = ANY(ARRAY[...])`). Postgres
    // also allows a subquery RHS (`x = ANY(SELECT ...)`); that path currently
    // errors here. The spine uses the array form; `IN (SELECT ...)` covers the
    // subquery case until a consumer needs `op ANY(subquery)`.
    let mut saw_null = false;
    for elem in &arr.elements {
        match eval_binary_op_ordered(op, left, elem, enum_order)? {
            Value::Bool(true) if !all => return Ok(Value::Bool(true)),
            Value::Bool(false) if all => return Ok(Value::Bool(false)),
            Value::Null => saw_null = true,
            _ => {}
        }
    }
    if saw_null {
        Ok(Value::Null)
    } else {
        // ANY exhausted with no match → false; ALL exhausted with no mismatch → true.
        Ok(Value::Bool(all))
    }
}

/// Evaluate a niladic builtin scalar function (`now()`, `gen_random_uuid()`).
///
/// # Errors
///
/// Propagates the builtin's error (e.g. `NotImplemented` on the wasm target).
pub fn eval_builtin(func: BuiltinFn) -> Result<Value> {
    use crate::builtins::{Clock, timestamp};
    match func {
        // Which clock each spelling reads is `crate::builtins`' module doc: the
        // transaction's start for every one of these but `statement_timestamp()` (the
        // statement's) and `clock_timestamp()` (the wall clock, at each call — the one
        // whose per-row-ness an `ADD COLUMN` backfill has to reproduce, see
        // `executor::ddl::is_per_row_volatile`). `LOCALTIMESTAMP` is `now()`'s value
        // under another inferred `TypeOid` (`timestamp` vs `timestamptz`), which the
        // text encoder uses to keep or drop the `+00` offset; the rest of the
        // timestamp spellings differ only in their output column name and their
        // `column_default` / dump rendering.
        BuiltinFn::Now
        | BuiltinFn::LocalTimestamp
        | BuiltinFn::TransactionTimestamp
        | BuiltinFn::CurrentTimestamp => timestamp(Clock::Transaction),
        BuiltinFn::StatementTimestamp => timestamp(Clock::Statement),
        BuiltinFn::ClockTimestamp => timestamp(Clock::Wall),
        BuiltinFn::RandomUuid => crate::builtins::random_uuid(),
        BuiltinFn::Random => crate::builtins::random(),
        BuiltinFn::CurrentDate => crate::builtins::current_date(),
        // Both `CURRENT_TIME` and `LOCALTIME` return the transaction start's UTC
        // time-of-day as a plain `time` (pglet is UTC-only, so it does not model
        // `CURRENT_TIME`'s `timetz`); they stay distinct builtins only so their output
        // column names differ (`current_time` vs `localtime`).
        BuiltinFn::CurrentTime | BuiltinFn::LocalTime => crate::builtins::current_time(),
        // The session-information forms. Each reads the **ambient** session
        // (`SessionSettings::install_ambient`) rather than a threaded argument, which
        // is what lets a stored `DEFAULT CURRENT_USER` and a `CHECK` resolve at the
        // sites the executor reaches them from.
        //
        // The four role spellings answer one value: pglet models no roles, so
        // `current_user` and `session_user` cannot differ the way they do on 18.6
        // after a `SET ROLE` (which stays in the unmodelled GUC-echo family here).
        BuiltinFn::CurrentUser
        | BuiltinFn::SessionUser
        | BuiltinFn::User
        | BuiltinFn::CurrentRole => Ok(Value::Text(
            crate::catalog::SessionSettings::ambient()
                .user()
                .to_string(),
        )),
        BuiltinFn::CurrentCatalog | BuiltinFn::CurrentDatabase => Ok(Value::Text(
            crate::catalog::SessionSettings::ambient()
                .database()
                .to_string(),
        )),
        // A constant: pglet's search path never moves (`EDGES.md` row 66), so the
        // schema an unqualified name resolves to is always `public`.
        BuiltinFn::CurrentSchema | BuiltinFn::CurrentSchemaFn => {
            Ok(Value::Text(crate::catalog::CURRENT_SCHEMA.to_string()))
        }
        BuiltinFn::Version => Ok(crate::builtins::version()),
    }
}

/// Evaluate an expression against a row (simple single-table queries).
///
/// A convenience wrapper over [`eval_expr_full`] for the common case of
/// single-table queries with unambiguous column names and no parameters / outer
/// scope. Does not support subqueries (`SubqueryExec::NONE`) — use
/// [`eval_expr_full`] with a populated `SubqueryExec` for those.
///
/// `col_types` is the scope's declared column types, positionally parallel to
/// `col_names` (empty when the caller has none). It is what keeps a stringified
/// `timestamptz` — `'v=' || ts`, `ts::text` — carrying its `+00` offset; see
/// [`ColumnResolver::column_type`]. `col_enums` is the same shape one type further:
/// each column's enum member order, which is what makes `<`/`>`/`MIN`/`MAX` over an
/// enum order by member position (see [`ColumnResolver::column_enum_order`]).
pub fn eval_expr(
    expr: &Expr,
    col_names: &[&str],
    col_types: &[TypeOid],
    col_enums: &[Option<&[String]>],
    row: &[Value],
) -> Result<Value> {
    eval_expr_full(
        expr,
        &SimpleColumns::new(col_names)
            .with_types(col_types)
            .with_enum_orders(col_enums),
        row,
        SubqueryExec::NONE,
        EvalEnv::EMPTY,
    )
}

/// Evaluate an expression that reads **no row** with a parameter environment (no
/// subquery).
///
/// The param-bearing sibling of [`eval_expr`] for an expression with no column scope —
/// a `nextval` argument, an index probe value, a `VALUES` cell, a literal's input
/// conversion: an `Expr::Parameter` resolves from `env`, and a column reference can bind
/// only through the frame chain.
///
/// It takes no column names on purpose. A node that evaluates over a row builds its own
/// resolver — over its input's names and its query level's FROM names
/// (`SimpleColumns::within_level`) — and passes it to `eval_expr_full`; a resolver
/// built here over a row's names would know no level, and a qualifier its level owns
/// would reach an enclosing frame of the same name.
pub fn eval_expr_env(expr: &Expr, env: EvalEnv<'_>) -> Result<Value> {
    eval_expr_full(expr, &SimpleColumns::new(&[]), &[], SubqueryExec::NONE, env)
}

/// Convert a literal to a value.
///
/// Integer literals are typed based on their value (matches Postgres behavior):
/// - Values in i32 range → Int4
/// - Values outside i32 range → Int8
pub fn literal_to_value(lit: &Literal) -> Value {
    match lit {
        Literal::Null => Value::Null,
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Int(n) => {
            // Use Int4 if value fits, otherwise Int8 (matches Postgres literal typing)
            if i32::try_from(*n).is_ok() {
                #[allow(clippy::cast_possible_truncation)]
                Value::Int4(*n as i32)
            } else {
                Value::Int8(*n)
            }
        }
        Literal::Float(f) => Value::Float8(*f),
        Literal::String(s) => Value::Text(s.clone()),
        Literal::Numeric(n) => Value::Numeric(n.clone()),
    }
}

/// Convert `Option<bool>` to `Value` (`None` -> `Null`, `Some(b)` -> `Bool(b)`).
pub const fn option_to_value(opt: Option<bool>) -> Value {
    match opt {
        Some(b) => Value::Bool(b),
        None => Value::Null,
    }
}
