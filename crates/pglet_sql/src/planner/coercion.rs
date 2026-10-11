//! Type resolution and coercion for the planner.
//!
//! This module handles inserting Cast nodes into the expression tree
//! for implicit type coercion (e.g., comparing INT column to '1' string).

use std::collections::HashMap;

use crate::Result;
use crate::catalog::Schema;
use crate::error::CatalogError;
use crate::types::{TypeOid, Value};

use super::common_type::{
    EnumIdentity, ResolvedType, nullif_result_type, select_common_type, variadic_common_type,
    variadic_common_type_oid,
};
use super::dml::{projects_wildcard, target_list_entry_with_input};
use super::expr::{
    AggregateFunc, ArgCertainty, BinaryOp, BoolContext, ColumnSizeOrigin, Expr, Literal,
    VariadicFunc,
};
use super::legality::{
    aggregate_legality, assignment_legality, branch_unification_legality, check_boolean_operand,
    comparison_legality, comparison_symbol, concat_legality, containment_legality,
    containment_symbol, json_get_legality, json_operator_legality, json_operator_symbol,
    like_legality, like_symbol, regex_legality, regex_symbol, starts_with_legality,
    unnest_legality,
};
use super::plan::{LeafScope, Plan, PlannerScope, TypeScope, expr_relations_resolvable};
use super::query::{collect_referenced_columns, contains_aggregate};

/// Resolve types in an expression, inserting Cast nodes where needed.
///
/// `scope` names the relations in scope and their column types. Returns a new
/// expression tree with Cast nodes for implicit coercions.
///
/// Resolution is **qualifier-aware** ([`ColumnScope::coercion_type`]): a qualified
/// `t.x` reads only relation `t`'s `x`, and a qualifier naming no relation here — a
/// correlated reference into an enclosing query, an `UPDATE … FROM` source pglet
/// doesn't register — declines rather than borrow a same-named local column's type.
/// The gate that runs after this pass ([`check_expr_semantics`]) resolves
/// through the same scope, so the two cannot disagree about what a reference
/// denotes: coercing `o.x = '1.1'` to a *sibling* relation's `float4` while the gate
/// typed `o.x` as `float8` is exactly how a comparison silently stopped matching.
///
/// The walker descends into **every** variant, so a comparison nested anywhere
/// (an `InList` element, a `CASE` branch, a `COALESCE` arg, …) still gets its
/// coercing cast — closing the silent divergence where, e.g.,
/// `WHERE int_col IN ('5')` matched zero rows because the string literal was never
/// coerced. Three sites add coercion beyond the bare comparison rule:
/// - **`IN (list)`** — a base table's column against two or more written constants is
///   converted to the type 18.6 builds its array at ([`in_list_array_type`]); in any
///   other list a string-literal item is coerced to the left column's type, as beside a
///   scalar `=`.
/// - **`CASE`/`COALESCE`** — string-literal *result/argument* branches are unified
///   to a sibling's concrete type (PG's unknown-literal → common-type rule), so
///   `COALESCE(int_col, '7')` parses `'7'` as int and `COALESCE(int_col, 'foo')`
///   raises the same runtime error PG does.
/// - **simple `CASE operand WHEN value`** — a string `value` label is coerced to
///   the operand's type (the `IN`-list rule applied to the implicit equality).
pub fn resolve_expr_types(mut expr: Expr, scope: &ColumnScope) -> Result<Expr> {
    crate::depth::check_stack()?;
    // A comparison's literal beside an aggregate is read from the operands as written,
    // before either is resolved.
    convert_literal_beside_aggregate(&mut expr, scope);
    // The chain-forming variants resolve their children in place from this frame, and
    // finish in one of their own, so a chain costs this frame a level (`crate::depth`).
    match &mut expr {
        Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } => {
            resolve_expr_types_in_place(left, scope)?;
            resolve_expr_types_in_place(right, scope)?;
        }
        Expr::Negate(child)
        | Expr::UnaryPlus(child)
        | Expr::Not(child)
        | Expr::IsNull(child)
        | Expr::IsNotNull(child)
        | Expr::Cast { expr: child, .. }
        | Expr::CastTypmod { expr: child, .. }
        | Expr::InSubquery { expr: child, .. }
        | Expr::BooleanTest { expr: child, .. } => resolve_expr_types_in_place(child, scope)?,
        Expr::InList {
            expr: child, list, ..
        } => {
            // Read from the operands as written, before either is resolved.
            let left = certain_in_list_left_type(child, list, scope);
            resolve_expr_types_in_place(child, scope)?;
            return resolve_in_list_items(expr, left, scope);
        }
        _ => return resolve_expr_types_rest(expr, scope),
    }
    Ok(resolve_chain_node(expr, scope))
}

/// Resolve a boxed child in place, reusing its allocation.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn resolve_expr_types_in_place(slot: &mut Box<Expr>, scope: &ColumnScope) -> Result<()> {
    let taken = std::mem::replace(&mut **slot, Expr::Default);
    **slot = resolve_expr_types(taken, scope)?;
    Ok(())
}

/// A node [`resolve_expr_types`] walks itself, its children resolved: a comparison
/// coerces a string literal against the other operand's column type, and a containment
/// its operands; every other one is already done.
#[inline(never)]
fn resolve_chain_node(expr: Expr, scope: &ColumnScope) -> Expr {
    let Expr::BinaryOp { left, op, right } = expr else {
        return expr;
    };
    let (left, right) = if is_comparison_op(op) {
        let (l, r) = coerce_comparison_operands(*left, *right, scope);
        (Box::new(l), Box::new(r))
    } else if matches!(op, BinaryOp::Containment | BinaryOp::ContainedBy) {
        let (l, r) = coerce_containment_operands(*left, *right, scope);
        (Box::new(l), Box::new(r))
    } else {
        (left, right)
    };
    Expr::BinaryOp { left, op, right }
}

#[inline(never)]
fn resolve_expr_types_rest(expr: Expr, scope: &ColumnScope) -> Result<Expr> {
    let recurse = |e: Expr| resolve_expr_types(e, scope);
    Ok(match expr {
        // Walked by `resolve_expr_types` itself: never here.
        Expr::BinaryOp { .. }
        | Expr::Negate(_)
        | Expr::UnaryPlus(_)
        | Expr::Not(_)
        | Expr::IsNull(_)
        | Expr::IsNotNull(_)
        | Expr::Cast { .. }
        | Expr::CastTypmod { .. }
        | Expr::InSubquery { .. }
        | Expr::AnyAll { .. }
        | Expr::InList { .. }
        | Expr::BooleanTest { .. } => resolve_expr_types(expr, scope)?,
        // read off the argument as written, before this pass rewrites it
        Expr::ColumnSize { arg, .. } => {
            let origin = classify_column_size(&arg, scope);
            Expr::ColumnSize {
                arg: Box::new(recurse(*arg)?),
                origin,
            }
        }
        Expr::Coalesce(args) => {
            let args: Vec<Expr> = args.into_iter().map(recurse).collect::<Result<_>>()?;
            // `COALESCE` resolves its arguments in source order (`transformCoalesceExpr`).
            let types = branch_types(&args, scope);
            Expr::Coalesce(unify_common_type_branches(args, &types, scope, "COALESCE"))
        }
        // `ARRAY[…]` unifies its elements exactly like `COALESCE` unifies its
        // arguments (`transformArrayExpr` calls the same `select_common_type`), and it
        // needs the casts for the same reason: without them `ARRAY[2::int4, 1::int8]`
        // declares `int4[]` — both type walkers read the *first* element — over a value
        // whose eval-time common type is `int8[]`.
        Expr::Array(elements) => {
            let elements: Vec<Expr> = elements.into_iter().map(recurse).collect::<Result<_>>()?;
            let types = branch_types(&elements, scope);
            Expr::Array(unify_common_type_branches(elements, &types, scope, "ARRAY"))
        }
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => resolve_case(operand, when_clauses, else_result, scope)?,
        // greatest/least/nullif unify their arguments to a common type: PG declares
        // (RowDescription) and returns (DataRow) that common type, so an int4/int8
        // mix must widen to int8 on BOTH sides or a binary client mis-decodes. Casting
        // every arg to the common type here makes the declared type (the type-walker
        // sees the casts) match the returned value (each arg is coerced to it). An
        // "unknown" string-literal arg coerces too, reproducing PG's runtime error
        // (`greatest('a', 1)` → `'a'::int` → 22P02). concat/concat_ws are text-valued
        // and need no unification.
        Expr::VariadicFn { func, args }
            if matches!(func, VariadicFunc::Greatest | VariadicFunc::Least) =>
        {
            let args: Vec<Expr> = args.into_iter().map(recurse).collect::<Result<_>>()?;
            Expr::VariadicFn {
                func,
                args: unify_variadic_args(args, scope),
            }
        }
        // `NULLIF` is **not** a common-type construct, despite living next to
        // `greatest`/`least` in the same IR node. Postgres resolves the `=` operator and
        // then declares *argument one's* type as the resolution left it, so
        // `NULLIF(int4_col, int8_col)` is `int4` where `GREATEST` of the same pair is
        // `int8`. Unifying both arguments to a common type — which this arm used to do —
        // widens the result past Postgres. Only argument one is cast, and only when the
        // operator resolution would have coerced it.
        Expr::VariadicFn {
            func: VariadicFunc::NullIf,
            args,
        } => {
            let args: Vec<Expr> = args.into_iter().map(recurse).collect::<Result<_>>()?;
            Expr::VariadicFn {
                func: VariadicFunc::NullIf,
                args: coerce_nullif_args(args, scope),
            }
        }
        Expr::Aggregate {
            func,
            arg,
            distinct,
            certainty,
        } => resolve_aggregate(func, *arg, distinct, certainty, scope)?,
        // Every other variant just descends into its children — including `ANY`/`ALL`,
        // which (unlike `IN`) gets *no* string-literal element coercion: PG resolves
        // `ARRAY['5']` to `text[]` and then errors on `int = text`, so coercing here
        // would be wrongly permissive. Leaves and self-contained sub-plans are returned
        // unchanged by `map_children`.
        other => other.try_map_children(recurse)?,
    })
}

/// Resolve an aggregate's argument and record the pass's verdict on it
/// ([`aggregate_arg_certainty`]) — the one producer of the fact every reader of the
/// aggregate's result type reads (`ArgCertainty`).
///
/// The verdict is taken on the aggregate's first run, from the argument as written, and
/// kept. The correlated re-run ([`super::correlation`]) runs this pass again over an
/// argument the first run already resolved, whose casts include those a set read at the
/// written reach chose from a guess — over a column `g.y` whose type is one,
/// `COALESCE(i, f + g.y)` comes back as `COALESCE(i, (f + g.y)::integer)` — so the
/// resolved form can read as certain where the written one did not. And readers at the enclosing level read the verdict
/// before that re-run: a scalar subquery's type is read as its level's branch types are
/// chosen, and a cast written there from one verdict is not revisited when another is
/// recorded. So an argument over an enclosing query's column — one the first run's scope
/// cannot type — is decided there, not certain, the answer an argument of unknown type
/// gets.
fn resolve_aggregate(
    func: AggregateFunc,
    arg: Expr,
    distinct: bool,
    certainty: ArgCertainty,
    scope: &ColumnScope,
) -> Result<Expr> {
    let certainty = match certainty {
        ArgCertainty::Undecided => aggregate_arg_certainty(func, &arg, scope, &no_subquery_type),
        decided => decided,
    };
    Ok(Expr::Aggregate {
        func,
        arg: Box::new(resolve_expr_types(arg, scope)?),
        distinct,
        certainty,
    })
}

/// Whether an `avg`'s or `sum`'s argument, **as written**, is certainly of an integer
/// type.
///
/// Certain is what the computed-reach branch reader ([`BranchReach::Computed`]) types:
/// a column whose type is certain ([`ColumnScope::column_type_certain`]), a typed
/// literal, a cast, a bound value, a scalar subquery `subquery` types, and the computed
/// and fold shapes over those. A column this scope cannot type — an enclosing query's,
/// on the first run — is not one. The argument is read before this pass resolves it and
/// before the pass writes a cast over a subquery in it ([`type_branch_subqueries`]), and
/// a `COALESCE`, `CASE` or `ARRAY[…]` that reader types is one every branch of which it
/// types or is of unknown type — exactly the sets [`set_reach`] reads at the computed
/// reach, whose target this pass then resolves and casts every branch to. So a certain
/// fold is one the pass resolved, by construction: a fold left at the written reach — a
/// branch over a column whose type is a guess, or over a scalar subquery whose column is
/// one — is not typed here, and its type is the walkers' guess.
/// `greatest`/`least` choose their
/// reach with [`arg_known_type`], which parts from this reader at an aggregate, an
/// `ARRAY[…]` or an alias argument: an `ARRAY[…]` is no integer, an alias cannot stand
/// inside an argument, and an argument holding an aggregate is recorded not certain
/// without asking any reader — the inner aggregate is still undecided here — and is
/// refused `42803` by the gate after this pass (`Error::NestedAggregate`).
///
/// `count`, `min` and `max` read no certainty, and are recorded as not certain.
fn aggregate_arg_certainty(
    func: AggregateFunc,
    arg: &Expr,
    scope: &ColumnScope,
    subquery: SubqueryLeaf<'_>,
) -> ArgCertainty {
    // An argument holding an aggregate is refused `42803` by the gate after this pass;
    // until then the inner aggregate is undecided, and no reader may ask its type.
    if !matches!(func, AggregateFunc::Avg | AggregateFunc::Sum) || contains_aggregate(arg) {
        return ArgCertainty::NotCertainInteger;
    }
    match branch_type_in(arg, scope, BranchReach::Computed, subquery) {
        Some(t @ (TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8)) => {
            ArgCertainty::CertainInteger(t)
        }
        _ => ArgCertainty::NotCertainInteger,
    }
}

/// Whether `op` is a comparison whose string-literal operand is coercible against
/// the other side's column type.
const fn is_comparison_op(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::IsDistinctFrom
            | BinaryOp::IsNotDistinctFrom
    )
}

/// Finish `expr [NOT] IN (list)`, its left operand resolved: recurse into every list
/// element, then convert the items to the type 18.6 compares the list at — where the
/// caller found every operand's type certain ([`certain_in_list_left_type`]) and handed
/// the left operand's as `left`.
///
/// Where [`in_list_array_type`] then answers, that is the list's common type: a string
/// literal is read as it, and under a float type an item of another type is cast to it,
/// since that conversion rounds. Under another type no cast is written — an integer
/// beside a wider integer or a `numeric`, a `date` beside a `timestamp`, are compared
/// with the left operand as 18.6 compares the converted item, measured — which keeps a
/// list already of the left operand's type planned as it was. A `NULL` is left as it is.
///
/// Everywhere else each item is compared with the left operand on its own, as a scalar
/// `=` is: a string literal beside a column is coerced to the column's type and nothing
/// else is touched (`EDGES.md` row 280). Either coercion is unconditional on the
/// literal's contents (same as the comparison path), so `int_col IN ('abc', '5')` errors
/// the whole query (`22P02`), consistent with `int_col = 'x'`.
#[inline(never)]
fn resolve_in_list_items(
    in_list: Expr,
    left: Option<TypeOid>,
    scope: &ColumnScope,
) -> Result<Expr> {
    let Expr::InList {
        expr,
        list,
        negated,
    } = in_list
    else {
        return Ok(in_list);
    };
    let col_type = match expr.as_ref() {
        Expr::Column { table, name } => scope.coercion_type(table.as_deref(), name),
        _ => None,
    };
    let mut list = list;
    for item in &mut list {
        *item = resolve_expr_types(std::mem::replace(item, Expr::Default), scope)?;
    }
    let target = left.and_then(|left| in_list_array_type(left, &list, scope));
    for item in &mut list {
        let mut taken = coerce_string_literal_to_target(
            std::mem::replace(item, Expr::Default),
            target.or(col_type),
        );
        if let Some(t @ (TypeOid::Float4 | TypeOid::Float8)) = target
            && branch_type_with_reach(&taken, scope, BranchReach::Written).is_some()
        {
            taken = cast_branch_to_target(taken, t, scope);
        }
        *item = taken;
    }
    Ok(Expr::InList {
        expr,
        list,
        negated,
    })
}

/// The left operand's type where `left [NOT] IN (list)`, as the statement wrote it, is a
/// list whose every operand has a type certain by construction — the only list converted
/// to 18.6's array form ([`resolve_in_list_items`]).
///
/// Certain is: the left operand is a plain reference to a column of a base table at this
/// query level ([`ColumnScope::base_column_type`]), and every item is a constant the
/// parser typed by its spelling — a literal, or a cast written directly over one. A type
/// this pass would have to infer — a computed operand, a parameter, a subquery, a derived
/// table's or an enclosing level's column — is one 18.6 can read differently, and a list
/// converted on a wrong reading matches rows 18.6's does not.
#[inline(never)]
fn certain_in_list_left_type(left: &Expr, list: &[Expr], scope: &ColumnScope) -> Option<TypeOid> {
    let Expr::Column { table, name } = left else {
        return None;
    };
    list.iter()
        .all(|item| match item {
            Expr::Literal(_) => true,
            Expr::Cast { expr, .. } | Expr::CastTypmod { expr, .. } => {
                matches!(**expr, Expr::Literal(_))
            }
            _ => false,
        })
        .then(|| scope.base_column_type(table.as_deref(), name))
        .flatten()
}

/// The element type of the array 18.6 builds from `left [NOT] IN (list)` over a left
/// operand of type `left` and items that are written constants.
///
/// `transformAExprIn` takes `select_common_type` over the left operand and the items
/// holding no column of the list's own level, when there are two or more of them,
/// converts each such item to that type and compares the left operand with the array.
/// So a `real` column beside `(0.1, 1.5)` is compared with two `real`s, where `x = 0.1`
/// compares at `float8` and does not match. A string literal and a `NULL` are 18.6's
/// `unknown`: they take the type without choosing it.
///
/// `None` keeps each item compared with the left operand on its own: 18.6's tree for a
/// single item and for a set with no common type. An enum's type is `text` here, under
/// which no conversion is written.
fn in_list_array_type(left: TypeOid, list: &[Expr], scope: &ColumnScope) -> Option<TypeOid> {
    if list.len() < 2 {
        return None;
    }
    let mut types = vec![left];
    for item in list {
        if !matches!(item, Expr::Literal(Literal::String(_) | Literal::Null)) {
            types.push(branch_type_with_reach(item, scope, BranchReach::Written)?);
        }
    }
    variadic_common_type_oid(&types)
}

/// Where `pg_column_size`'s argument `arg`, as written, takes its value from
/// ([`ColumnSizeOrigin`]), with the argument's scope in reach.
///
/// A constant is settled by its spelling ([`ColumnSizeOrigin::written`]). A stored value
/// is a bare column of one stored relation at this level ([`ColumnScope::stored_column`]).
/// Every other expression is one 18.6 computes, or reads through a node whose datum
/// pglet cannot place, and is [`ColumnSizeOrigin::Unsupported`].
fn classify_column_size(arg: &Expr, scope: &ColumnScope) -> ColumnSizeOrigin {
    match (arg, ColumnSizeOrigin::written(arg)) {
        (_, constant @ ColumnSizeOrigin::Constant { .. }) => constant,
        (Expr::Column { table, name }, _) => scope
            .stored_column(table.as_deref(), name)
            .unwrap_or(ColumnSizeOrigin::Unsupported),
        _ => ColumnSizeOrigin::Unsupported,
    }
}

/// Resolve a `CASE` expression, coercing string-literal labels (simple form) and
/// unifying the result branches (see [`resolve_expr_types`]).
fn resolve_case(
    operand: Option<Box<Expr>>,
    when_clauses: Vec<(Expr, Expr)>,
    else_result: Option<Box<Expr>>,
    scope: &ColumnScope,
) -> Result<Expr> {
    let operand = operand
        .map(|o| resolve_expr_types(*o, scope).map(Box::new))
        .transpose()?;
    // A simple `CASE operand WHEN value` compares each `value` to the operand for
    // equality, so a string `value` coerces to the operand's concrete type.
    let operand_type = operand
        .as_deref()
        .and_then(|o| concrete_branch_type(o, scope));

    let mut conditions = Vec::with_capacity(when_clauses.len());
    let mut results = Vec::with_capacity(when_clauses.len() + 1);
    for (cond, res) in when_clauses {
        let cond = resolve_expr_types(cond, scope)?;
        conditions.push(coerce_string_literal_to_target(cond, operand_type));
        results.push(resolve_expr_types(res, scope)?);
    }
    let has_else = else_result.is_some();
    if let Some(e) = else_result {
        results.push(resolve_expr_types(*e, scope)?);
    }

    // Unify the result branches (THEN results + ELSE) to their common type; the ELSE,
    // pushed last, is popped back off after.
    //
    // **The ELSE resolves FIRST.** `transformCaseExpr` builds its type list with
    // `resultexprs = lcons(newc->defresult, resultexprs)` — commented "the default
    // result is considered the most significant type" — so the `ELSE` branch leads.
    // That is only observable when two branch types are mutually implicit, and then it
    // is decisive: `CASE WHEN … THEN text_col ELSE varchar_col END` is `varchar`, while
    // swapping the two spellings gives `text`. Resolving in positional order gets both
    // backwards.
    let types = {
        let positional = branch_types(&results, scope);
        let mut ordered = Vec::with_capacity(positional.len());
        if has_else {
            // The ELSE was pushed last, so it is the final entry.
            ordered.extend(positional.last().cloned());
        }
        let then_count = positional.len() - usize::from(has_else);
        ordered.extend(positional.iter().take(then_count).cloned());
        ordered
    };
    let mut results = unify_common_type_branches(results, &types, scope, "CASE");
    let else_result = has_else.then(|| Box::new(results.pop().unwrap_or(Expr::Default)));
    let when_clauses = conditions.into_iter().zip(results).collect();
    Ok(Expr::Case {
        operand,
        when_clauses,
        else_result,
    })
}

/// Coerce a bare string literal to `target` when `target` is a string-coercible
/// scalar type ([`is_string_coercible_target`]) — the comparison / `IN` / simple-`CASE`
/// rule. The coercion is **unconditional on the literal's contents**: an unparseable
/// spelling still gets the coercing `Cast`, which fails loud at eval (matching
/// Postgres, which resolves the unknown literal to the target type and errors at run
/// time — `int_col IN ('abc')` → `22P02`, never a silent 0-row match). A non-string
/// expression, an absent target, or a non-coercible target (`text`, `bytea`, …) is
/// returned unchanged.
fn coerce_string_literal_to_target(expr: Expr, target: Option<TypeOid>) -> Expr {
    match (target, &expr) {
        (Some(t), Expr::Literal(Literal::String(_))) if is_string_coercible_target(t) => {
            cast_expr(expr, t)
        }
        _ => expr,
    }
}

/// Wrap `expr` in a coercing `Cast` to `target`.
fn cast_expr(expr: Expr, target: TypeOid) -> Expr {
    Expr::Cast {
        expr: Box::new(expr),
        target_type: target,
    }
}

/// Unify a set of `CASE`/`COALESCE`/`ARRAY[…]` branches to their Postgres common
/// type, casting **every** branch that isn't already it.
///
/// Casting every branch — not just the string literals the old rule reached — is what
/// makes the declared type and the returned value agree. `COALESCE(int4_col,
/// int8_col)` resolves to `int8`, and without a cast on the `int4` branch the
/// projection declares `int8` (8 bytes) while a row that takes the first branch sends
/// 4 — the same row set differing in width per row, which a binary client cannot
/// decode. Both type walkers then read the target off the casts, so neither needs its
/// own copy of this rule.
///
/// `types_in_pg_order` is the branch types **in the order Postgres resolves them**,
/// which is not always the order they appear in `branches`: `CASE` puts its `ELSE`
/// first (see [`resolve_case`]). The order is observable whenever two types are
/// mutually implicit — `CASE … THEN text_col ELSE varchar_col` is `varchar` and the
/// reverse is `text` — so the caller, not this function, owns it.
///
/// A branch set with no common type is left **unchanged**; the error is raised by
/// [`check_expr_semantics`] during the gated pass, so this pass stays infallible and
/// idempotent. That split is by design, not convenience — this pass is re-run over a
/// level-chained scope, and a pass that could error would have to decide whether a
/// not-yet-resolvable correlated branch is a failure (it isn't). The diagnosis
/// therefore only judges branch sets whose types are fully known, deferring exactly
/// where this unifier defers.
fn unify_common_type_branches(
    branches: Vec<Expr>,
    types_in_pg_order: &[Option<ResolvedType>],
    scope: &ColumnScope,
    context: &str,
) -> Vec<Expr> {
    // A branch that is a column this scope cannot type **defers** the whole
    // unification: its type is a candidate target, so choosing one without it can
    // pick the wrong one (`COALESCE(outer.f8, inner.i4, '5')` would cast to `int4`).
    // Declining leaves the branches untouched for the enclosing level's pass to
    // unify once the correlated column resolves — and makes this pass idempotent,
    // which the re-run over a chained scope depends on. See [`ColumnScope`]
    // § Level chaining.
    if branches.iter().any(|e| is_untyped_column(e, scope)) {
        return branches;
    }
    let Ok(Some(target)) = select_common_type(types_in_pg_order, context) else {
        return branches;
    };
    // **A target carrying an enum identity is cast to by nothing.** `Expr::Cast` names a
    // `TypeOid`, and no `TypeOid` names an enum, so the only cast this pass could write
    // for a `mood` target is one to `text` — which is 18.6's *other* type. Writing that
    // cast would be wrong; **not writing it is not the coercion either**.
    // 18.6 coerces the unknown-typed sibling to the enum,
    // and that coercion runs `enum_in` — the whole of a `22P02`, which skipping the cast
    // performs no part of. What performs it is `planner::enum_literal_legality`, at the
    // **gate** below rather than in this infallible pass, because a pass that could
    // error would have to decide whether a not-yet-resolvable correlated branch is a
    // failure (it is not). So this early return says only *do not write a cast to the
    // wrong type*; it says nothing about the literal, and the literal is judged.
    //
    // **Belt, and the brace is `EnumIdentity::Undecided`.** Removing these two lines
    // reddens nothing measured: the whole `enum/` fixture family, `crates/pglet`'s
    // `enum_identity_gates`, and the `dump` text of a `CHECK (greatest(m, 'ok') > 'sad')`
    // are all byte-identical with the cast written, because the gate reads that cast as
    // undecidable, `expr_enum_with` answers `None` for it either way, the declared type
    // is `text` on both spellings, and a stored expression is stored *as written* and so
    // never sees this pass at all. It stays because the tree it builds is the tree 18.6
    // builds, and a later narrowing of `erases_enum_identity` — the day a written
    // `x::mood` carries its name — would turn the cast into an over-refusal with nothing
    // red to say so.
    if target.enum_type.name().is_some() {
        return branches;
    }
    let reach = set_reach(&branches, |e| concrete_branch_type(e, scope));
    branches
        .into_iter()
        .map(|e| cast_branch_to_target_at(e, target.oid, scope, reach))
        .collect()
}

/// Cast one branch to the unification target, unless it already carries that type.
///
/// Skipping the no-op keeps the pass **idempotent** (the re-run over a level-chained
/// scope must not stack casts) and keeps an already-homogeneous branch set —
/// the spine's `COALESCE(scope_kind, 'GLOBAL')` — byte-identical to before.
fn cast_branch_to_target(expr: Expr, target: TypeOid, scope: &ColumnScope) -> Expr {
    cast_branch_to_target_at(expr, target, scope, BranchReach::Computed)
}

/// [`cast_branch_to_target`] with the no-op test read at `reach` — the reach the set's
/// target was chosen at, so a branch the target was chosen without is not taken as
/// already carrying it: a derived column whose type is a guess is typed at the written
/// reach and not at the computed one, and testing it at the other reach than its set's
/// cast it where the set's own reading leaves it.
fn cast_branch_to_target_at(
    expr: Expr,
    target: TypeOid,
    scope: &ColumnScope,
    reach: BranchReach,
) -> Expr {
    // An unknown-typed branch (bare string literal / NULL) has no type of its own and
    // always adopts the target — except that casting a string literal to `text` is the
    // identity, and adding the node would churn every text-branch plan for nothing.
    if branch_type_with_reach(&expr, scope, reach) == Some(target)
        || matches!(
            (&expr, target),
            (Expr::Literal(Literal::String(_)), TypeOid::Text)
        )
    {
        return expr;
    }
    cast_expr(expr, target)
}

/// The branch types of `branches`, in their own order, for [`unify_common_type_branches`].
/// **No catalog, and the pass cannot reach a shape that needs one.** A branch that is a
/// bare scalar subquery is [`is_untyped_column`]'s deferral, which
/// [`unify_common_type_branches`] checks *before* it folds — so the subquery leaf of
/// [`expr_enum_in`] is unreachable from this pass, and the gate below, which does hold a
/// catalog, is where such a branch is judged.
///
/// The set is read at [`set_reach`]'s reach: the computed shapes are typed only where
/// that types every branch.
fn branch_types(branches: &[Expr], scope: &ColumnScope) -> Vec<Option<ResolvedType>> {
    let reach = set_reach(branches, |e| concrete_branch_type(e, scope));
    set_types(branches, reach, scope, &|e, reach| {
        branch_type_with_reach(e, scope, reach)
    })
    .into_iter()
    .zip(branches)
    .map(|(branch_type, e)| {
        branch_type.map(|oid| ResolvedType {
            oid,
            enum_type: enum_identity_in(e, scope, None),
        })
    })
    .collect()
}

/// The types a branch set's target is chosen from: each branch's at the set's `reach`
/// ([`set_reach`]), as `typed` reads it there.
///
/// At the written reach one more type joins: `double precision`, for a branch the
/// computed reader types so. 18.6 chooses `double precision` for a set holding one
/// whatever numeric types stand beside it, so that branch decides the target and it
/// does not turn on which type the written reading holds for another — nor on whether
/// it holds one at all: with no branch typed at the written reach, the set's target is
/// still `double precision`. A column's value converts to the float as 18.6 converts
/// one, where the cast the other way rounds the float branch: `COALESCE(x, f + 0)` over
/// a column `x` this pass holds as `numeric` without vouching for it cast `f + 0` to
/// `numeric`, `0.12345678901234568` answering `0.123456789012346`, where 18.6 answers
/// the `double precision`.
///
/// It joins only a set of the numeric category: every branch the written reach types is
/// in it, and every branch neither reach types is in it by the declared-type walkers'
/// reading ([`walker_type`]) or is of unknown type. A branch the walkers read as `text`,
/// a `boolean` or a `date` leaves the set as the written reach reads it, the gates that
/// refuse such a set reading its branches uncast.
///
/// No other type joins. A `real` branch does not decide the target — beside an exact
/// branch 18.6 chooses `double precision`, which a set converted to `real` rounds away
/// from — and neither does a branch of an exact type: a guessed column cast to a wider
/// exact type rounds the value the guess mis-declares ([`set_reach`]).
fn set_types(
    branches: &[Expr],
    reach: BranchReach,
    scope: &ColumnScope,
    typed: &dyn Fn(&Expr, BranchReach) -> Option<TypeOid>,
) -> Vec<Option<TypeOid>> {
    let types: Vec<Option<TypeOid>> = branches.iter().map(|e| typed(e, reach)).collect();
    if reach != BranchReach::Written || !types.iter().flatten().all(|t| is_numeric_category(*t)) {
        return types;
    }
    let mut joined = types.clone();
    for (slot, e) in joined.iter_mut().zip(branches) {
        if slot.is_some() || is_unknown_branch(e) {
            continue;
        }
        if typed(e, BranchReach::Computed) == Some(TypeOid::Float8) {
            *slot = Some(TypeOid::Float8);
        } else if !walker_type(e, scope).is_some_and(is_numeric_category) {
            return types;
        }
    }
    joined
}

/// The declared-type walkers' reading of a branch neither reach types, with no catalog:
/// a guess, read for its **category** alone ([`set_types`]).
fn walker_type(expr: &Expr, scope: &ColumnScope) -> Option<TypeOid> {
    crate::planner::unknown_aware_type(
        expr,
        &ClauseLeafScope {
            columns: scope,
            schema: None,
        },
    )
}

/// Whether `expr` is a branch of **unknown** type, which takes its set's target: a bare
/// string literal, a `NULL`, and a `$N` no cast types — what 18.6's parse analysis holds
/// as `unknown` and converts to the type the set's other branches choose.
const fn is_unknown_branch(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Literal(Literal::String(_) | Literal::Null) | Expr::Parameter { .. }
    )
}

/// The reach a branch set's target is chosen at: [`BranchReach::Computed`] where that
/// reader (`typed`) types every branch that is not of unknown type
/// ([`is_unknown_branch`]), and [`BranchReach::Written`] otherwise.
///
/// A set the computed reader types only in part keeps the target the written reach gives
/// it. Typing one computed branch and not its sibling would choose the target from the
/// typed one and cast the untyped sibling to it: over a column `g.y` whose type is a
/// guess, `COALESCE(i + 0, f + g.y)` read at the computed reach casts the `float8`
/// branch to `integer`, `3.5` answering `4`, where the set read at the written reach is
/// left as it was and answers 18.6's value. The one computed branch a written-reach set
/// does take into its target is a `double precision` one ([`set_types`]).
fn set_reach<'a>(
    branches: impl IntoIterator<Item = &'a Expr>,
    typed: impl Fn(&Expr) -> Option<TypeOid>,
) -> BranchReach {
    let all_typed = branches
        .into_iter()
        .all(|e| typed(e).is_some() || is_unknown_branch(e));
    if all_typed {
        BranchReach::Computed
    } else {
        BranchReach::Written
    }
}

/// Unify the arguments of `greatest`/`least` to their common type by inserting a `Cast`
/// on each argument whose known type isn't already it.
///
/// `NULLIF` shares the IR node but **not** this rule — it resolves an `=` operator
/// rather than a common type, and goes through [`coerce_nullif_args`].
///
/// The common type is resolved from the arguments' **known** types (a bare string
/// literal or NULL is "unknown" and adopts a sibling's type — Postgres's
/// unknown-literal rule). When no known type exists (all-unknown, e.g.
/// `greatest('a', 'b')`) the arguments are left unchanged (they stay homogeneous
/// text); when the known types are incompatible (`int` column vs `text` column) they
/// are also left unchanged, and the mismatch is then [`check_branch_unification_node`]'s
/// `42804` at **plan** time — `greatest(i, tx)` raises it over `WHERE false` and over an
/// empty table, as 18.6 does.
///
/// **"Known" here is [`arg_known_type`]'s answer, which is narrower than the gate's.**
/// The gate resolves the same arguments through [`concrete_branch_type`], which carries
/// `Array` and `Alias` arms this reader lacks — so over those two arms
/// this function folds the concrete sibling alone and casts an argument the gate would
/// have refused, and the gate then sees an agreeing `Cast`. **Only when there is a
/// sibling to fold**: with no argument known to this reader — or two that disagree —
/// [`variadic_common_type`] answers `None`, the `else` below returns the arguments
/// untouched, and the gate types the raw pair itself
/// (`greatest(ARRAY[1,2], set_config('x', text_col, false))` is `42804` on both engines).
/// See [`check_branch_unification_node`] and `EDGES.md` row 65's third sub-case.
fn unify_variadic_args(args: Vec<Expr>, scope: &ColumnScope) -> Vec<Expr> {
    // An argument this scope cannot type defers the whole unification — see the same
    // rule in [`unify_common_type_branches`].
    if args.iter().any(|a| is_untyped_column(a, scope)) {
        return args;
    }
    let reach = set_reach(&args, |a| arg_known_type(a, scope));
    // What the pass folds is each argument's type paired with its enum identity
    // ([`expr_enum_in`]), so a `greatest(m, 'ok')` resolves to the enum and wraps
    // nothing, where folding the OIDs alone resolved `text` and cast the literal to it.
    let known: Vec<ResolvedType> = set_types(&args, reach, scope, &|a, reach| {
        arg_known_type_with_reach(a, scope, reach)
    })
    .into_iter()
    .zip(&args)
    .filter_map(|(arg_type, a)| {
        arg_type.map(|oid| ResolvedType {
            oid,
            enum_type: enum_identity_in(a, scope, None),
        })
    })
    .collect();
    let Some(target) = variadic_common_type(&known) else {
        return args;
    };
    // See [`unify_common_type_branches`]: an enum target has no `Expr::Cast` spelling,
    // and writing the `text` one would erase the identity the gate reads.
    if target.enum_type.name().is_some() {
        return args;
    }
    args.into_iter()
        .map(|e| {
            if arg_known_type(&e, scope) == Some(target.oid) {
                e // already the target type — no redundant cast
            } else {
                Expr::Cast {
                    expr: Box::new(e),
                    target_type: target.oid,
                }
            }
        })
        .collect()
}

/// Coerce `NULLIF`'s **first** argument to the type Postgres declares for the whole
/// expression ([`nullif_result_type`]), leaving the second alone.
///
/// `NULLIF(a, b)` returns `a` (or NULL), so the declared type and the returned value
/// agree exactly when argument one carries the declared type — which it already does
/// whenever a registered `=` operator takes it on the left, and does not when the
/// operator resolution had to widen it (`NULLIF(int4_col, float4_col)` is `float8`).
/// The second argument only has to be *comparable*, which the `=` operand gate enforces
/// (at plan time where it can type the pair, at eval otherwise — below); widening it here
/// would be the bug this replaced.
///
/// `NULLIF` reaches no plan-time *branch-unification* check
/// ([`branch_unification_legality`]'s match does not name it), but it is not unguarded at
/// plan time: [`check_expr_semantics`]'s own `VariadicFunc::NullIf` arm types the pair
/// through the same `comparison_legality(…, "=")` rulebook, over
/// [`certain_operand_type`] — `nullif(int_col, text_col)` is
/// `42883 operator does not exist: integer = text` on both engines with no row evaluated
/// (`WHERE false` included). That gate reads the arguments **after** this function, as
/// the eval-time one does, so a cast inserted here satisfies it too. Over
/// `nullif(set_config('x', text_col, false), int_col)`, [`arg_known_type`] cannot see the
/// `Alias`, so [`nullif_result_type`] takes the sibling's `int4` and this function wraps
/// argument one in a `Cast` to it — and the gate, whose [`certain_operand_type`] *does*
/// carry an `Alias` arm, then reads that `Cast` rather than the `text` beneath it and has
/// an agreeing pair: pglet answers NULL where 18.6 refuses `42883`. Leave it nothing to
/// cast to and it fires on the pair it can read —
/// `nullif(set_config('x', text_col, false), set_config('y', int_col, false))` is
/// `42883 operator does not exist: text = integer` here with no row evaluated (pglet-only
/// — 18.6 refuses the inner `set_config(unknown, integer, boolean)`).
///
/// An `Expr::Array` argument is an arm [`certain_operand_type`] lacks as well, so the
/// gate cannot refuse that operand by either route. `nullif(ARRAY[1,2], text_col)` finds
/// `text` to coerce to, and the gate — which could not have typed the raw `ARRAY[…]`
/// either — reads the resulting `Cast` and has nothing to refuse, so pglet answers
/// `{1,2}` where 18.6 refuses `42883 operator does not exist: integer[] = text`. With no
/// target nothing is cast, and that same missing arm leaves the gate one operand it
/// cannot type, so the eval-time `=` gate is the only one left and the answer is a
/// property of the rows rather than of the statement:
/// `nullif(ARRAY[1,2], set_config('x', text_col, false))` is `42883` over a row and
/// `(0 rows)` over `WHERE false`, against 18.6's `42883` either way. Both spellings sit
/// in the arms [`arg_known_type`] lacks and [`concrete_branch_type`] carries —
/// `EDGES.md` row 65's third sub-case.
fn coerce_nullif_args(args: Vec<Expr>, scope: &ColumnScope) -> Vec<Expr> {
    // Same deferral as the branch unifiers: a column this scope cannot type yet.
    if args.iter().any(|a| is_untyped_column(a, scope)) {
        return args;
    }
    let [first, second] = args.as_slice() else {
        return args;
    };
    let reach = set_reach([first, second], |a| arg_known_type(a, scope));
    let types = set_types(&args, reach, scope, &|a, reach| {
        arg_known_type_with_reach(a, scope, reach)
    });
    let Some(target) = nullif_result_type(types[0], types[1]) else {
        return args;
    };
    let mut args = args;
    let first = args.remove(0);
    // `cast_branch_to_target`'s no-op skip keeps this idempotent across the re-run
    // over a level-chained scope. It is read at the pair's reach, as the target was.
    args.insert(0, cast_branch_to_target_at(first, target, scope, reach));
    args
}

/// `expr` with the scalar subqueries under its folds — a `COALESCE`, a `CASE`, an
/// `ARRAY[…]`, a `greatest`/`least`/`nullif` — wrapped in a `Cast` to their output
/// column's type: every one whose column is **certain**
/// ([`certain_scalar_subquery_type`]), wherever under the fold it stands (`in_fold`), and
/// one standing as a **branch** where that type is at least resolved
/// ([`resolved_subquery_type`]).
///
/// The branch readers ([`concrete_branch_type`], [`arg_known_type`]) hold no catalog, so a
/// scalar subquery is invisible to them, and an invisible branch beside a concrete one was
/// cast to that sibling's type: `COALESCE((SELECT 1.25), 2)` answered `integer` `1` where
/// 18.6 answers `numeric` `1.25`, and a `DELETE … WHERE COALESCE((SELECT n FROM t LIMIT
/// 1), 2) = 1` removed a row 18.6 keeps. A `Cast` is a shape both readers type, and it
/// converts nothing at the subquery's own type. A branch subquery left bare — its type
/// not resolved in `scope` — is untyped to [`is_untyped_column`], so the unification
/// defers rather than casting it to a sibling, and a re-run over a chained scope can
/// still type it. A subquery already under a `Cast` is left alone.
///
/// The certain reading is asked first, and for an **operand** under a branch it is the
/// only one. So the pass's readers type exactly the subqueries the reading that records
/// a certainty types ([`branch_type_in`]), and a fold that reading calls certain is one
/// this pass resolves: `COALESCE(-(SELECT avg(x) FROM v), f + 0)`, the subquery's column
/// a certain `numeric`, is a set the computed reach types whole and unifies to 18.6's
/// `double precision`, where the uncast operand left it unresolved and declared at its
/// first branch's `numeric` over `float8` values. It is also the cheaper reading: the
/// resolved one asks about a nested subquery more than once at each level (`EDGES.md`
/// row 310), and is not asked over a sub-plan whose column is certain.
///
/// The cast over a branch whose type is only resolved types the branch for its fold and
/// says nothing of the type being **certain**: it is written wherever the sub-plan's
/// column references resolve, over a sub-plan whose column is a guess. A reading that
/// records a certainty is taken before this runs and asks the sub-plan.
fn type_scalar_subquery_branches(
    mut expr: Expr,
    scope: &ColumnScope,
    schema: &Schema,
    in_fold: bool,
) -> Result<Expr> {
    crate::depth::check_stack()?;
    // A chain-forming node has no branch to type, and walks its children from this
    // frame, so a chain costs it a level (`crate::depth`); every other node is
    // `type_scalar_subquery_branches_rest`'s. Children in `map_children`'s order.
    match &mut expr {
        Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } => {
            type_scalar_subquery_in_place(left, scope, schema, in_fold)?;
            type_scalar_subquery_in_place(right, scope, schema, in_fold)?;
        }
        // A subquery already under a cast is typed by it, and is left as it is.
        Expr::Cast { expr: child, .. } | Expr::CastTypmod { expr: child, .. }
            if matches!(**child, Expr::Subquery { .. }) => {}
        Expr::Negate(child)
        | Expr::UnaryPlus(child)
        | Expr::Not(child)
        | Expr::IsNull(child)
        | Expr::IsNotNull(child)
        | Expr::Cast { expr: child, .. }
        | Expr::CastTypmod { expr: child, .. }
        | Expr::InSubquery { expr: child, .. }
        | Expr::BooleanTest { expr: child, .. } => {
            type_scalar_subquery_in_place(child, scope, schema, in_fold)?;
        }
        Expr::InList {
            expr: child, list, ..
        } => {
            type_scalar_subquery_in_place(child, scope, schema, in_fold)?;
            type_scalar_subquery_list(list, scope, schema, in_fold)?;
        }
        _ => return type_scalar_subquery_branches_rest(expr, scope, schema, in_fold),
    }
    Ok(expr)
}

/// [`type_scalar_subquery_branches`] over a boxed child in place, reusing its allocation.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn type_scalar_subquery_in_place(
    slot: &mut Box<Expr>,
    scope: &ColumnScope,
    schema: &Schema,
    in_fold: bool,
) -> Result<()> {
    let taken = std::mem::replace(&mut **slot, Expr::Default);
    **slot = type_scalar_subquery_branches(taken, scope, schema, in_fold)?;
    Ok(())
}

/// [`type_scalar_subquery_branches`] over an `IN` list's members, in place.
#[inline(never)]
fn type_scalar_subquery_list(
    list: &mut [Expr],
    scope: &ColumnScope,
    schema: &Schema,
    in_fold: bool,
) -> Result<()> {
    for item in list {
        let taken = std::mem::replace(item, Expr::Default);
        *item = type_scalar_subquery_branches(taken, scope, schema, in_fold)?;
    }
    Ok(())
}

/// [`type_scalar_subquery_branches`] for every node but a chain-forming one.
#[inline(never)]
fn type_scalar_subquery_branches_rest(
    expr: Expr,
    scope: &ColumnScope,
    schema: &Schema,
    in_fold: bool,
) -> Result<Expr> {
    let cast = |plan, target_type| Expr::Cast {
        expr: Box::new(Expr::Subquery { plan }),
        target_type,
    };
    // Under a fold, a subquery whose column is certain — wherever it stands.
    let expr = match expr {
        Expr::Subquery { plan } if in_fold => {
            return Ok(match certain_scalar_subquery_type(&plan, schema) {
                Some(target_type) => cast(plan, target_type),
                None => Expr::Subquery { plan },
            });
        }
        other => other,
    };
    // A branch the certain reading left bare, where its type is resolved.
    let branch = |e: Expr| match e {
        Expr::Subquery { plan } => match resolved_subquery_type(&plan, scope, schema) {
            Some(target_type) => cast(plan, target_type),
            None => Expr::Subquery { plan },
        },
        other => other,
    };
    let in_fold = in_fold
        || matches!(
            expr,
            Expr::Coalesce(_)
                | Expr::Array(_)
                | Expr::Case { .. }
                | Expr::VariadicFn {
                    func: VariadicFunc::Greatest | VariadicFunc::Least | VariadicFunc::NullIf,
                    ..
                }
        );
    let expr = expr
        .try_map_children(|child| type_scalar_subquery_branches(child, scope, schema, in_fold))?;
    Ok(match expr {
        Expr::Coalesce(args) => Expr::Coalesce(args.into_iter().map(branch).collect()),
        Expr::Array(elements) => Expr::Array(elements.into_iter().map(branch).collect()),
        Expr::VariadicFn {
            func: func @ (VariadicFunc::Greatest | VariadicFunc::Least | VariadicFunc::NullIf),
            args,
        } => Expr::VariadicFn {
            func,
            args: args.into_iter().map(branch).collect(),
        },
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => Expr::Case {
            operand,
            when_clauses: when_clauses
                .into_iter()
                .map(|(when, result)| (when, branch(result)))
                .collect(),
            else_result: else_result.map(|e| Box::new(branch(*e))),
        },
        other => other,
    })
}

/// The type of a scalar subquery's single output column when it is **resolved** and a
/// `Cast` to it carries the value whole — `None` otherwise.
///
/// Resolved means one of three things. A set operation with no column pending an
/// enclosing level, whose `column_types` are unified. Or a target-list entry every one of
/// whose column references resolves — in the subquery's own input, else in an enclosing
/// subquery's input, else in `scope`, the clause's namespace — and every scalar subquery
/// inside which is itself resolved, over an input no set operation with a pending column
/// feeds. Or **no** target-list entry at all, the projection being a wildcard, where the
/// column is whatever that wildcard stands over ([`wildcard_column_type`]).
/// `Plan::output_columns` is otherwise not asked for the type:
/// it answers `text` for a reference it cannot resolve, and a pending column's
/// provisional type, both guesses.
///
/// Resolved is not certain. The type a resolved entry is read at is the declared-type
/// walkers', which is a guess over a fold the sub-plan's pass left at the written reach,
/// an `ARRAY[…]` constructor's `unnest`, an aggregate over an enclosing query's column.
/// Whether a column's type is certain is the sub-plan's own record
/// (`ColumnDesc::type_certain`).
fn resolved_subquery_type(plan: &Plan, scope: &ColumnScope, schema: &Schema) -> Option<TypeOid> {
    match resolved_subquery_type_in(plan, &EnclosingLeaves::Clause(scope), schema) {
        // An enum the entry names through an enclosing level is beside the OID and not
        // in it, so a cast to the OID would erase it.
        SubqueryType::Castable(resolved) if resolved.enum_type.name().is_none() => {
            Some(resolved.oid)
        }
        SubqueryType::Castable(_) | SubqueryType::Uncastable(_) | SubqueryType::Deferred => None,
    }
}

/// The type of a scalar subquery's single output column when it is **resolved**, whether
/// or not a `Cast` could carry it — the reader
/// [`check_branch_unification_node`] resolves a bare subquery branch through.
///
/// The gate only compares types, so an enum column and the `name` type answer here where
/// they decline to [`resolved_subquery_type`]: `COALESCE((SELECT current_user), i)` is
/// 18.6's `42804` because this reader sees the `name`, and nothing casts to it. What the
/// gate then does with a `name` is
/// [`super::common_type`]'s business, not this reader's — it has `name`'s string-category
/// `pg_cast` rows, so the same subquery beside a `text` sibling **resolves**. An answer
/// here is not a refusal.
///
/// The enum arm answers with the column's `type_oid` **and** its
/// `ColumnDesc::enum_type`, paired as a [`ResolvedType`]: the OID for an enum is its
/// base `text`, so the identity beside it is the only thing that tells the gate `mood`
/// from `text` and from another enum. A `Cast` to it would erase that identity, which is
/// why that branch is `Uncastable` rather than a cast the pass inserts.
///
/// A [`SubqueryType::Castable`] answer carries an identity too, and it is the one thing
/// this reader must not read off the descriptor alone: `ColumnDesc::enum_type` is an
/// `Option` with no third state, and it is `None` both for a projection that is not an
/// enum and for one whose identity the descriptor could not derive — a **cast**, whose
/// enum name the parser dropped (`EDGES.md` row 104), being the one that survives now
/// that `Expr::to_column_desc` derives a projected expression's (row 107). Reading that
/// `None` as *not an enum* refused `m IN (SELECT 'ok'::mood)` and
/// `m IN (SELECT min(m) FROM w)`, which 18.6 answers. So the identity comes from
/// [`resolved_subquery_type_in`]'s own derivation over the projection, and the descriptor
/// is read only where it **names** one.
fn branch_subquery_type(plan: &Plan, scope: &ColumnScope, schema: &Schema) -> Option<ResolvedType> {
    match resolved_subquery_type_in(plan, &EnclosingLeaves::Clause(scope), schema) {
        SubqueryType::Castable(resolved) | SubqueryType::Uncastable(resolved) => Some(resolved),
        SubqueryType::Deferred => None,
    }
}

/// What [`resolved_subquery_type_in`] found at a scalar subquery's single output column.
#[derive(Clone)]
enum SubqueryType {
    /// Resolved, and a `Cast` to it carries the value whole — the cast naming the
    /// [`ResolvedType`]'s OID, the identity beside it being for a reader that
    /// *compares* rather than casts. The one exception is an enum the entry names
    /// through a nested subquery or an enclosing level, where the descriptor read first
    /// names none: the OID is the enum's `text`, and [`resolved_subquery_type`] — the
    /// reader that casts — declines it.
    Castable(ResolvedType),
    /// Resolved, and a `Cast` to it would **not**: an enum's identity is erased by one
    /// (`EDGES.md` row 104) and `name`/`name[]` is no cast's target. A reader
    /// that *compares* types may still have it — and for an enum it must have the
    /// identity too, which is why this arm carries a [`ResolvedType`] where
    /// [`Self::Castable`] carries the OID a `Cast` node can name.
    Uncastable(ResolvedType),
    /// Not resolved under these namespaces — an enclosing level's may still type it, and
    /// a re-run over a level-chained scope is what asks again.
    Deferred,
}

#[cfg(test)]
thread_local! {
    /// How many times this thread has asked [`resolved_subquery_type_in`] — what a test
    /// of the reading's cost over a nest of subqueries counts.
    pub(super) static RESOLVED_SUBQUERY_READS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    /// How many times this thread has asked [`certain_scalar_subquery_type`].
    pub(super) static CERTAIN_SUBQUERY_READS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// [`resolved_subquery_type`] under the namespaces `enclosing` holds.
fn resolved_subquery_type_in(
    plan: &Plan,
    enclosing: &EnclosingLeaves<'_>,
    schema: &Schema,
) -> SubqueryType {
    #[cfg(test)]
    RESOLVED_SUBQUERY_READS.with(|reads| reads.set(reads.get() + 1));
    let columns = plan.output_columns(schema);
    let [column] = columns.as_slice() else {
        return SubqueryType::Deferred;
    };
    if column.enum_type.is_some() || matches!(column.type_oid, TypeOid::Name | TypeOid::NameArray) {
        return SubqueryType::Uncastable(ResolvedType::new(
            column.type_oid,
            column.enum_type.clone(),
        ));
    }
    match set_op_first_column(plan) {
        SetOpColumn::Pending => return SubqueryType::Deferred,
        SetOpColumn::Unified {
            type_oid,
            left,
            right,
        } => {
            return type_oid.map_or(SubqueryType::Deferred, |oid| {
                SubqueryType::Castable(ResolvedType {
                    oid,
                    enum_type: set_op_enum_identity(left, right, enclosing, schema),
                })
            });
        }
        SetOpColumn::NotASetOp => {}
    }
    let Some((entry, input)) = target_list_entry_with_input(plan, 0) else {
        // No target-list entry at all: the projection is a wildcard, and this column's
        // type is whatever the wildcard stands over. Its identity is
        // `ResolvedType::plain` because a wildcard renames and computes nothing: the
        // descriptor a scan hands up *is* the catalog's, and the catalog's answer was
        // read above — reaching here means it named no enum.
        return wildcard_column_type(
            plan,
            &ResolvedType::plain(column.type_oid),
            enclosing,
            schema,
        );
    };
    if input.is_some_and(holds_pending_set_op) {
        return SubqueryType::Deferred;
    }
    let scoped = input.map(|p| p.output_columns_scoped(schema));
    let local = scoped
        .as_ref()
        .map_or(TypeScope::NONE, |(shape, relations)| TypeScope::Input {
            relations,
            columns: shape.columns(),
            enclosing: None,
        });
    let leaf = ResolvedLeafScope {
        local: PlannerScope::new(schema, local),
        enclosing,
        schema,
        nested: std::cell::RefCell::default(),
    };
    let mut refs = Vec::new();
    collect_referenced_columns(entry, &mut refs);
    if refs
        .iter()
        .any(|(table, name)| leaf.column_type(table.as_deref(), name).is_none())
        || !scalar_subqueries_resolved(entry, &leaf)
    {
        return SubqueryType::Deferred;
    }
    entry
        .infer_type(&leaf)
        .map_or(SubqueryType::Deferred, |oid| {
            SubqueryType::Castable(ResolvedType {
                oid,
                enum_type: entry_enum_identity(entry, &leaf),
            })
        })
}

/// The enum identity of a target-list entry, read under the namespaces `leaf` chains —
/// the sub-plan instantiation of [`enum_identity_in`], three-valued where the descriptor
/// `Expr::to_column_desc` builds is not (`EDGES.md` rows 104 and 107), and reaching the
/// enclosing levels a descriptor built under the sub-plan's own scope cannot.
///
/// Three-valued for the same reason every other identity reader is: the walk's `Some` is
/// the enum, and its `None` has to be split into *this projection is not an enum* and
/// *this projection's identity is not decidable here*, because a reader that answers the
/// second as the first asserts a negative it does not know — and that assertion refuses.
///
/// Two things produce the second, and neither is a list assembled here.
/// [`erases_enum_identity`] is one, whatever it comes to hold — today a cast, whose enum
/// name the parser dropped (`EDGES.md` row 104), and a spliced bound `text` value. The
/// **subquery leaf** is the other: its answer is this same reader one level down, so it
/// is undecidable where that level was. What makes those two the whole of it is
/// `expr_enum_with`'s own boundary statement — the `Cast` arm is the only arm of that
/// walk which drops an identity 18.6 can see — and not an arm count taken here. The
/// column leaf is [`ColumnScope::column_enum`]'s twin over this chain and answers on the
/// same terms the clause-side gates already accept it on.
///
/// **The subquery leaf's `Deferred` arm is structurally unreachable, and the match needs
/// it anyway.** This function has one caller, and it sits below
/// [`scalar_subqueries_resolved`], which walks the same entry through `for_each_child` —
/// a superset of the nodes `expr_enum_with` descends into — and requires every
/// `Expr::Subquery` in it to answer [`LeafScope::subquery_type`] under the same
/// [`EnclosingLeaves::Subquery`] chain this closure asks under, which a `Deferred` one
/// does not. The arm is stated rather than removed for that reason: were the guard above
/// to move, `Deferred` must keep setting the flag rather than reading as *not an enum*. An
/// `Uncastable` enum is reached, and its name is the identity read here.
fn entry_enum_identity(entry: &Expr, leaf: &ResolvedLeafScope<'_>) -> EnumIdentity {
    if entry_identity_undecidable(entry) {
        return EnumIdentity::Undecided;
    }
    let undecided = std::cell::Cell::new(false);
    let name = crate::executor::expr_enum_with(
        entry,
        &crate::executor::EnumLeaves {
            column: &|table, name| leaf.column_enum(table, name),
            subquery: &|plan| match leaf.nested_type(plan) {
                SubqueryType::Castable(resolved) | SubqueryType::Uncastable(resolved) => {
                    match resolved.enum_type {
                        EnumIdentity::Named(name) => Some(name),
                        EnumIdentity::Undecided => {
                            undecided.set(true);
                            None
                        }
                        EnumIdentity::NotAnEnum => None,
                    }
                }
                SubqueryType::Deferred => {
                    undecided.set(true);
                    None
                }
            },
        },
    );
    match name {
        Some(name) => EnumIdentity::Named(name),
        None if undecided.get() => EnumIdentity::Undecided,
        None => EnumIdentity::NotAnEnum,
    }
}

/// Whether a target-list entry's own identity cannot be decided — [`erases_enum_identity`]
/// at this position, with the one producer a **projection settles**.
///
/// An unresolved literal is 18.6's `unknown` only for as long as something can still
/// adopt it, and a target list is where that stops: `transformTargetList` resolves an
/// `unknown`-typed entry to `text` before the column leaves the subquery, so the outer
/// operator is offered `text` and nothing else. `m IN (SELECT 'ok')` is therefore
/// `42883 operator does not exist: mood = text` on 18.6, where the `m = 'ok'` spelling
/// of the same label is a membership test — the same split the `ARRAY[…]` constructor
/// makes over its own elements, at the other construct that settles an `unknown` before
/// a comparison sees it.
///
/// **The bare literal is the only spelling this arm decides**, and the neighbouring ones
/// that 18.6 also answers `mood = text` for are decided elsewhere, each because a pass
/// has already rewritten the literal: `m IN (SELECT 'ok' UNION SELECT 'sad')` at
/// [`set_op_enum_identity`] and `m IN (SELECT * FROM (VALUES ('sad')) v)` at
/// [`wildcard_column_type`]'s derived-table descent. `m IN (SELECT NULL)` reaches none of
/// the three — its entry has no type at all, so the reader defers and the pair is not
/// judged, which is row 114's open sub-plan-column residual rather than this arm's.
///
/// So the literal is decided here, and a **cast** is what is left undecidable: its enum
/// name was dropped in the parser (`EDGES.md` row 104), which is why
/// `m IN (SELECT 'ok'::mood)` defers to the OID rule and answers as 18.6 answers it.
fn entry_identity_undecidable(entry: &Expr) -> bool {
    match strip_alias(entry) {
        Expr::Literal(Literal::String(_) | Literal::Null) => false,
        other => erases_enum_identity(other),
    }
}

/// The identity a set operation's unified first column carries, from its two sides'.
///
/// The descriptor's own unification (`unified_enum_type`) has already been read and
/// named no enum, which is the `Option`'s single `None` standing for two facts again: two
/// sides that are not enums, and two whose identity the descriptor never derived. So each
/// side is asked through this reader, which does derive a projected expression's —
/// `SELECT min(m) FROM w UNION SELECT min(m) FROM w` is `mood` here and `None` there.
///
/// **A side this reader cannot decide contributes nothing, and the result is never
/// itself undecidable.** That is not the conservatism the other identity readers take,
/// and the reason is the node: a set operation's `unknown` branch columns are coerced to
/// the unified type by this engine's own pass before any gate sees them, which writes
/// the very `Cast` a user's `'ok'::mood` parses to (`EDGES.md` row 104) — so
/// `SELECT 'ok' UNION SELECT 'sad'`, which 18.6 refuses against an enum, and
/// `SELECT 'ok'::mood UNION SELECT 'sad'::mood`, which it answers, are the **same two
/// nodes** here. Deferring answers the first with rows; answering the decided side
/// refuses both, and the second is then a **loud** over-refusal rather than a silent
/// wrong row set.
fn set_op_enum_identity(
    left: &Plan,
    right: &Plan,
    enclosing: &EnclosingLeaves<'_>,
    schema: &Schema,
) -> EnumIdentity {
    let side = |plan| match resolved_subquery_type_in(plan, enclosing, schema) {
        SubqueryType::Castable(resolved) | SubqueryType::Uncastable(resolved) => {
            resolved.enum_type.name().map(ToOwned::to_owned)
        }
        SubqueryType::Deferred => None,
    };
    crate::executor::unify_enum_pair(side(left), side(right))
        .map_or(EnumIdentity::NotAnEnum, EnumIdentity::Named)
}

/// The type of output column 0 of a plan [`resolved_subquery_type_in`] found **no
/// target-list entry** for — a projection left to a wildcard — asked of whatever that
/// wildcard stands over.
///
/// A **scan**'s columns are the relation's, which `output_columns` read from the catalog
/// rather than guessed, so `passthrough` — the caller's own reading of column 0, through
/// arms that neither rename nor drop a column — is that answer. A **derived table**'s are
/// its own query's, so the question recurses into it under the same enclosing
/// namespaces. Anything else defers — including a **function scan**, which is why
/// `CASE WHEN i = 7 THEN (SELECT * FROM unnest(ARRAY[1.5::numeric])) ELSE i END` is
/// neither cast nor refused and sends a raw `int4` under a `numeric` descriptor
/// (`EDGES.md` row 152).
///
/// [`projects_wildcard`] matches a **qualified** `t.*` as well as a bare `*`, so the
/// `Project` arm's guard is not by itself the bare-`*` test it reads as. What keeps the
/// qualified spelling from reaching here is `expand_wildcards`, which expands every
/// qualified wildcard outright and leaves one unexpanded only where it is the projection's
/// sole entry over a FROM that is not a join. The `[e]` slice pattern is this arm's own
/// reason: a wildcard beside another entry would move the column index this answers for.
fn wildcard_column_type(
    plan: &Plan,
    passthrough: &ResolvedType,
    enclosing: &EnclosingLeaves<'_>,
    schema: &Schema,
) -> SubqueryType {
    let down = |input| wildcard_column_type(input, passthrough, enclosing, schema);
    match plan {
        Plan::Scan { .. } | Plan::VirtualScan { .. } => SubqueryType::Castable(passthrough.clone()),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { .. } | Plan::IndexRangeScan { .. } | Plan::GinScan { .. } => {
            SubqueryType::Castable(passthrough.clone())
        }
        Plan::Project { input, exprs, .. } if matches!(exprs.as_slice(), [e] if projects_wildcard(e)) => {
            down(input)
        }
        Plan::Filter { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input }
        | Plan::DistinctOn { input, .. } => down(input),
        // A **derived table**'s columns have been materialized before this reader sees
        // them, and this is the one descent that reads a column of one without reading
        // the reference that names it. `unify_values_columns` coerces an `unknown`
        // `VALUES` cell to the column's unified type — writing the very `Cast` node a
        // user's `'sad'::mood` parses to (`EDGES.md` row 104) — so an `Undecided` coming
        // back from here cannot be told from a written cast, and answering it as one let
        // `DELETE FROM t WHERE m IN (SELECT * FROM (VALUES ('sad')) v)` remove a row 18.6
        // keeps by refusing. So it is answered as the coercion it more often is: the
        // `(VALUES ('sad'::mood))` and `(SELECT 'ok'::mood)` spellings of a derived table
        // are a **loud** over-refusal here, where the direct `m IN (SELECT 'ok'::mood)`
        // — whose entry this reader does see — answers as 18.6 answers it.
        Plan::SubqueryScan { subquery, .. } => {
            match resolved_subquery_type_in(subquery, enclosing, schema) {
                SubqueryType::Castable(resolved) if resolved.enum_type.is_undecided() => {
                    SubqueryType::Castable(ResolvedType::plain(resolved.oid))
                }
                decided => decided,
            }
        }
        _ => SubqueryType::Deferred,
    }
}

/// Whether every scalar subquery in `expr` has a resolved type under `leaf` — an
/// `EXISTS` or `IN (SELECT …)` is `boolean` whatever its plan.
fn scalar_subqueries_resolved(expr: &Expr, leaf: &ResolvedLeafScope<'_>) -> bool {
    if let Expr::Subquery { plan } = expr {
        return leaf.subquery_type(plan).is_some();
    }
    let mut resolved = true;
    expr.for_each_child(|child| resolved = resolved && scalar_subqueries_resolved(child, leaf));
    resolved
}

/// The namespaces enclosing a subquery [`resolved_subquery_type_in`] types: the clause's,
/// behind the input of each subquery between it and the clause.
enum EnclosingLeaves<'a> {
    Clause(&'a ColumnScope),
    Subquery(&'a ResolvedLeafScope<'a>),
}

impl EnclosingLeaves<'_> {
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        match self {
            Self::Clause(scope) => scope.coercion_type(table, name),
            Self::Subquery(leaf) => leaf.column_type(table, name),
        }
    }

    /// The enum identity leaf beside [`Self::column_type`], reaching the same namespaces
    /// in the same order — so a correlated column's identity cannot be read from a
    /// different level than its type.
    fn column_enum(&self, table: Option<&str>, name: &str) -> Option<String> {
        match self {
            Self::Clause(scope) => scope.column_enum(table, name),
            Self::Subquery(leaf) => leaf.column_enum(table, name),
        }
    }
}

/// A subquery's own input scope with its enclosing namespaces behind it, for
/// [`resolved_subquery_type_in`].
struct ResolvedLeafScope<'a> {
    local: PlannerScope<'a>,
    enclosing: &'a EnclosingLeaves<'a>,
    schema: &'a Schema,
    /// Each nested subquery's answer under this leaf, read once: the entry is asked
    /// about it three times — resolved, typed, identity — and asking anew each time
    /// made one read over a nest geometric in its depth. Keyed on the sub-plan's
    /// address, which the entry, borrowed for this leaf's life, holds still.
    nested: std::cell::RefCell<Vec<(*const Plan, SubqueryType)>>,
}

impl LeafScope for ResolvedLeafScope<'_> {
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        self.local
            .own_column_type(table, name)
            .or_else(|| self.enclosing.column_type(table, name))
    }

    /// A nested subquery answers where a `Cast` could carry its type, and where its column
    /// is an enum: this type becomes the *enclosing* entry's, and an enum's is its base
    /// `text`, with the identity beside it read by [`entry_enum_identity`] — the entry is
    /// then an enum, which [`resolved_subquery_type`], the reader that casts, declines.
    /// Declined here, every entry over a nested enum column would defer, and `'zzz' IN
    /// (SELECT (SELECT m FROM te LIMIT 1) FROM t2)` compare the label as `text`.
    fn subquery_type(&self, plan: &Plan) -> Option<TypeOid> {
        match self.nested_type(plan) {
            SubqueryType::Castable(resolved) => Some(resolved.oid),
            SubqueryType::Uncastable(resolved) if resolved.enum_type.name().is_some() => {
                Some(resolved.oid)
            }
            SubqueryType::Uncastable(_) | SubqueryType::Deferred => None,
        }
    }
}

impl ResolvedLeafScope<'_> {
    /// [`resolved_subquery_type_in`] over a subquery nested in this leaf's entry, under
    /// this leaf — derived at its first ask and read back at the rest.
    fn nested_type(&self, plan: &Plan) -> SubqueryType {
        let key = std::ptr::from_ref(plan);
        if let Some((_, found)) = self.nested.borrow().iter().find(|(p, _)| *p == key) {
            return found.clone();
        }
        let found = resolved_subquery_type_in(plan, &EnclosingLeaves::Subquery(self), self.schema);
        self.nested.borrow_mut().push((key, found.clone()));
        found
    }

    /// This subquery's own input, else the enclosing chain — the identity twin of
    /// [`LeafScope::column_type`]'s walk, keyed on **where the column resolves** rather
    /// than on where an enum is found, so a local `text` column cannot fall through to a
    /// same-named enum one level out.
    ///
    /// **The enclosing arm is reached, and what reaches it is not a correlated *shape* but
    /// a sub-plan whose own input registers no claimant for the name.** An ordinary
    /// correlated entry does not: `local` is a `PlannerScope` over
    /// `input.output_columns_scoped`, which already carries the enclosing relations such
    /// an entry names, so `enum/in_subquery_correlated_computed_column_*` and
    /// `enum/in_subquery_two_level_correlated_computed_column_accepts` are byte-identical
    /// with this arm replaced by `local` alone, at one level out and at two — which is
    /// what an earlier reading of those two runs generalised into a claim that nothing
    /// reached it. A FROM that introduces *only* its own relation is what leaves the name
    /// unclaimed locally: `SELECT m IN (SELECT m FROM (VALUES (1)) q) FROM t` fires the
    /// arm, measured on an instrumented build, and is `t, t, t` on both engines against
    /// the `42883 operator does not exist: mood = text` a `local`-only leaf answers.
    /// `enum/subquery_enclosing_column_projection_accepts` and its two `_loud` siblings
    /// are that guard, in both polarities and over the second enum.
    ///
    /// The **keying** is what the arm order is for, and it is unchanged: a shape reaching
    /// here is one where the local scope cannot type a column this walk was only asked
    /// about because the *type* leaf resolved it, and answering that from a different
    /// level than the type is the failure the local-first test exists to prevent.
    fn column_enum(&self, table: Option<&str>, name: &str) -> Option<String> {
        if self.local.own_column_type(table, name).is_some() {
            return self.local.column_enum(table, name);
        }
        self.enclosing.column_enum(table, name)
    }
}

/// What [`set_op_first_column`] found at a plan's first output column.
enum SetOpColumn<'a> {
    /// The plan is not a set operation, through the wrappers a parenthesized one carries.
    NotASetOp,
    /// A set operation's column still pending an enclosing level, so its type is
    /// provisional (`docs/scope.md` § A pending set-op column's type is never read).
    Pending,
    /// A set operation's unified column type — `None` for a node with no columns — with
    /// the two sides beside it, which the identity is read from and the type is not.
    Unified {
        type_oid: Option<TypeOid>,
        left: &'a Plan,
        right: &'a Plan,
    },
}

/// A set operation's first column, through the wrappers a parenthesized one carries.
fn set_op_first_column(plan: &Plan) -> SetOpColumn<'_> {
    match plan {
        Plan::SetOp {
            column_types,
            outer_pending,
            left,
            right,
            ..
        } => {
            if outer_pending.contains(&0) {
                SetOpColumn::Pending
            } else {
                SetOpColumn::Unified {
                    type_oid: column_types.first().copied(),
                    left,
                    right,
                }
            }
        }
        Plan::Filter { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input }
        | Plan::DistinctOn { input, .. } => set_op_first_column(input),
        _ => SetOpColumn::NotASetOp,
    }
}

/// Whether a set operation with a column pending an enclosing level feeds `plan`'s rows —
/// through its FROM items, joins, derived tables and set-operation sides — so a column
/// typed from them carries a provisional type. An arm not descended answers `false`, which
/// reaches only a statement's **first** parse — whose plan the parser discards whenever a
/// set operation was pending in it (`SetOpOuterTypes`) — at the cost that an error such a
/// mistyped branch raises there stands, there being no second parse to reach.
fn holds_pending_set_op(plan: &Plan) -> bool {
    match plan {
        Plan::SetOp {
            outer_pending,
            left,
            right,
            ..
        } => !outer_pending.is_empty() || holds_pending_set_op(left) || holds_pending_set_op(right),
        Plan::Join { left, right, .. } => holds_pending_set_op(left) || holds_pending_set_op(right),
        Plan::SubqueryScan {
            subquery: input, ..
        }
        | Plan::Filter { input, .. }
        | Plan::Project { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Distinct { input }
        | Plan::DistinctOn { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Aggregate { input, .. }
        | Plan::Window { input, .. } => holds_pending_set_op(input),
        _ => false,
    }
}

/// Whether `expr` is a bare column reference `scope` cannot type — the "this operand's
/// type belongs to a level not in this scope yet" test.
///
/// A `CASE`/`COALESCE`/`greatest` unification that includes one cannot pick a target
/// without guessing, so it defers rather than unify against the remaining siblings.
/// The distinction from a bare string literal matters: a literal is *unknown-typed* and
/// legitimately adopts a sibling's type, while a column has a concrete type this scope
/// simply hasn't been given.
fn is_untyped_column(expr: &Expr, scope: &ColumnScope) -> bool {
    match expr {
        Expr::Column { table, name } => scope.coercion_type(table.as_deref(), name).is_none(),
        // A bare scalar subquery is one [`type_scalar_subquery_branches`] could not type.
        Expr::Subquery { .. } => true,
        _ => false,
    }
}

/// The known `TypeOid` of a `greatest`/`least`/`nullif` argument, or `None` for an
/// "unknown" one (a bare string literal or NULL, which adopts a sibling's type) or a
/// computed expression outside [`BranchReach::Computed`]'s shapes, left out of target
/// selection.
///
/// An aggregate argument is typed as [`concrete_branch_type`] types it — the reader
/// [`check_branch_unification_node`] resolves the same arguments through — so the pass
/// folds an aggregate into the target rather than casting it to a sibling's type:
/// `least(sum(i), 5)` is `bigint` as on 18.6, and `greatest(sum(i), text_col)` reaches
/// the gate uncast and is its `42804`.
///
/// **Otherwise deliberately narrower than [`concrete_branch_type`], and the gap costs a
/// gate verdict rather than a declared type.** That reader carries `Array` and `Alias`
/// arms this one does not — so an argument invisible here and visible there gets cast to
/// its sibling's type by the pass and then passes the gate, **when a sibling is visible
/// here to fold**. When none is, there is no target and no cast, and the gate types the
/// raw pair as it always could: `greatest(ARRAY[1,2], text_col)` is the silent shape,
/// `greatest(ARRAY[1,2], set_config('x', text_col, false))` the `42804`-on-both one.
/// Widening the two to one arm list is the close; `EDGES.md` row 65's third sub-case
/// has the measured members.
fn arg_known_type(expr: &Expr, scope: &ColumnScope) -> Option<TypeOid> {
    arg_known_type_with_reach(expr, scope, BranchReach::Computed)
}

/// [`arg_known_type`] at a given [`BranchReach`].
fn arg_known_type_with_reach(
    expr: &Expr,
    scope: &ColumnScope,
    reach: BranchReach,
) -> Option<TypeOid> {
    match expr {
        // At the computed reach only a column whose type is certain, as
        // [`branch_type_with_reach`] reads one.
        Expr::Column { table, name }
            if reach == BranchReach::Computed
                && !scope.column_type_certain(table.as_deref(), name) =>
        {
            None
        }
        Expr::Column { table, name } => scope.coercion_type(table.as_deref(), name),
        Expr::Cast { target_type, .. } => Some(*target_type),
        // A typmod'd cast is concrete at its target type (like a plain cast) — so
        // `greatest(x::numeric(4,1), 0)` unifies the `0` to numeric.
        Expr::CastTypmod { type_oid, .. } => Some(*type_oid),
        // A bare string literal or NULL is "unknown" — it adopts a sibling's type.
        Expr::Literal(Literal::String(_) | Literal::Null) => None,
        Expr::Literal(lit) => lit.infer_type(),
        Expr::Aggregate { .. } => branch_type_with_reach(expr, scope, reach),
        // The computed shapes [`BranchReach::Computed`] types, read the same way here:
        // without them `least(i, f + 0)` unified to `integer` and cast `2.5` to `2`.
        _ if reach == BranchReach::Computed => {
            let operand = |e: &Expr| arg_known_type_with_reach(e, scope, reach);
            computed_reach_type(expr, &operand)
        }
        _ => None,
    }
}

/// [`resolve_expr_types`] followed by [`validate_const_casts`].
///
/// Inserts the coercing casts, then eagerly validates every constant string cast so an
/// unparseable literal fails loud **at plan time**. The `Result`-returning entry point
/// for the top-level plan sites (projection, `WHERE`, no-FROM row, DML filter).
///
/// The operand and branch-unification diagnoses deliberately live one step further on,
/// in [`resolve_expr_types_gated`]'s single [`check_expr_semantics`] walk — see the
/// note there.
///
/// # Errors
///
/// The coercion error of the first unparseable constant string cast (`22P02` /
/// `22007` / …) — identical to the eval-time cast.
pub fn resolve_expr_types_checked(expr: Expr, scope: &ColumnScope) -> Result<Expr> {
    let resolved = resolve_expr_types(expr, scope)?;
    validate_const_casts(&resolved)?;
    Ok(resolved)
}

/// The **branch-unification** diagnosis for one node, against this query's scope.
///
/// Delegates to the shared rulebook ([`branch_unification_legality`]) so the plan-time
/// walk and the DDL-time stored-expression walk apply the *same* rule — the two differ
/// only in how they resolve a branch's type, which is the closure each passes in.
///
/// **The closure is [`branch_type_with_subqueries`] for every one of the five constructs
/// [`branch_unification_legality`] matches — [`concrete_branch_type`] plus the one arm
/// that needs a catalog — and the blindness is per *branch*, not per
/// set.** With one exception, decided per set rather than per branch: where any branch's
/// widened read names an **enum**, every branch is read through [`complete_resolved`]
/// behind that closure, because an enum identity the exhaustive walk *does* derive was
/// otherwise discarded with the `None` the OID came back as (`EDGES.md` row 114, and
/// `UPDATE … SET m = greatest(m, lower(s))` is what it wrote). The branch list that
/// decision reads is `unification_branches`, the same one the rule folds, so the two
/// cannot come to disagree about which nodes are branch sets. Everything below describes
/// the reader in its unwidened state, which is what a set with no enum in it still gets.
///
/// [`select_common_type`](super::select_common_type) folds
/// `types.iter().flatten()`, so a branch this reader answers `None` for is dropped from
/// the fold and from the closing re-check alike: it is never compared against any
/// sibling, however many concrete siblings stand beside it.
///
/// **This closure is *not* uniformly what decides whether the coercion pass rewrites the
/// branches**, which is the trap: the pass reads `concrete_branch_type` only for
/// `COALESCE`/`CASE`/`ARRAY` — `greatest`/`least` go through [`unify_variadic_args`] and
/// `NULLIF` through [`coerce_nullif_args`], both over [`arg_known_type`], which lacks the
/// `Array` and `Alias` arms. So over a variadic the pass is the blinder of
/// the two readers, and it *runs first*: it folds the concrete sibling alone, wraps the
/// argument it cannot see in a `Cast` to that target, and this gate then reads that
/// `Cast` at an agreeing type. `greatest(ARRAY[1,2], text_col)` answers instead of
/// raising, where `greatest(int_array_col, text_col)` — the same pair through the
/// `Column` arm, which both readers carry — is `42804` as it should be.
///
/// **That inversion needs a concrete sibling for the pass to fold**, and stops without
/// one: with nothing concrete to [`arg_known_type`], [`variadic_common_type`] answers
/// `None`, [`unify_variadic_args`] returns its arguments untouched, and this closure then
/// types the raw pair itself — `greatest(ARRAY[1,2], set_config('x', text_col, false))`
/// is `42804` on both engines, with a row and over `WHERE false` alike. So the blindness
/// is the pass's *cast*, not the pass's reader. Three sub-cases
/// follow, and the set-wide one is only the narrowest:
///
/// - **No concrete member** — the fold, read at the written reach a set the computed
///   reader types only in part keeps (`set_reach`; `length` is typed there and `upper`
///   is not), is `[None, None]`, the answer `Ok(None)`, and nothing at all is compared. Nothing casts either, so the walkers declare the first
///   *inferable* branch while evaluation returns the branch it takes:
///   `COALESCE(length('ab'), upper('7'))` answers int4 `2` here and
///   `ARRAY[length('ab'), upper('7')]` answers `_int4` `{2,7}` where 18.6 refuses
///   `42804` at parse analysis. "No concrete member" is wider than "every branch
///   computed": this reader's `_` arm also swallows an `Expr::Bound`, so an `INTERVAL`
///   literal is one — `COALESCE(INTERVAL '1 day', length('ab'))` answers `1 day` under
///   `1186` where 18.6 refuses `42804`.
/// - **Some concrete member** — the concrete siblings resolve a target, the pass casts
///   the invisible branch to it, and the divergence is the same missing `42804` in far
///   more ordinary spellings: `COALESCE(i, upper(tx))` over `i = 7` answers `7`, and
///   `COALESCE(upper('7'), 1)` answers int4 `7` with no column at all. Here the
///   declaration and the value agree — the cast saw to that — so the answer is silent
///   wherever the cast succeeds or the branch is short-circuited past
///   (`COALESCE(1, upper('x'))` answers `1`), and `22P02` at eval where it does not
///   (`COALESCE(upper('x'), 1)`, `ARRAY[1, upper('x')]`, `greatest(i, upper('x'))`).
/// - **Reader mismatch**, the variadics only — the branch is invisible to the *pass* and
///   visible to *this* closure, and the pass's `Cast` is what blinds the gate. Over
///   `(i INT, tx TEXT)` holding `(7, '7')`: `greatest(ARRAY[1,2], tx)` answers text
///   `{1,2}`, `least` `7`, `nullif` `{1,2}`, and the `set_config('x', tx, false)`
///   spelling of each beside `i` answers int4 `7`, `7` and NULL, where 18.6 refuses each
///   `42804` (`nullif` `42883`). Silent wherever the pass's cast succeeds; where it
///   cannot, loud with pglet's own code rather than Postgres's — `42846` for an
///   `ARRAY[…]` argument against a non-string sibling, `22007`/`22P02` for an `Alias` one
///   whose value will not convert. An aggregate argument is no member: [`arg_known_type`]
///   types it as this closure does.
///
/// Controls. `COALESCE(i, tx)` — no invisible branch at all — is `42804` on both
/// engines, and says the gate exists. `COALESCE(i, tx, upper('7'))` is `42804` on both
/// too, and says the gate still fires over a *disagreeing concrete pair* with an
/// invisible branch sitting beside it: what is skipped is the comparison of that
/// branch, not the gate, which is what makes the rule per-branch rather than per-set.
/// `greatest(int_array_col, tx)` and `greatest(int_array_col::int[], tx)` are `42804` on
/// both and pin the third sub-case to the arm list rather than to the argument, and the
/// whole `COALESCE`/`CASE`/`ARRAY` family — one reader for pass and gate alike — is
/// untouched by the arms the two readers do not share.
/// All measured. `EDGES.md` row 65 — and widening
/// `concrete_branch_type` past everything it answers `None` for is the one change that
/// closes it together with row 54's fold band; the third sub-case closes on the same
/// change, by the two readers becoming one arm list. The numeric-category computed shapes
/// are widened (`BranchReach::Computed`), and a set they type only in part is judged at
/// the written reach, so the members above are unmoved.
fn check_branch_unification_node(expr: &Expr, scope: &ColumnScope, schema: &Schema) -> Result<()> {
    // The same widening the comparison pair takes, decided over the same branch list
    // the rule folds (`unification_branches`) rather than a second one: a set with an
    // enum in it is judged on the complete reader's types, a set without one keeps the
    // conservative reader and row 65's verdicts. `greatest(m, lower(s))` was the write
    // this closes — `UPDATE … SET m = greatest(m, lower(s))` rewrote a stored member
    // where 18.6 refuses `42804`.
    let widen = crate::planner::unification_branches(expr).is_some_and(|(branches, _)| {
        enum_pair_names_one(
            &branches
                .iter()
                .map(|e| {
                    branch_type_with_subqueries(e, scope, schema)
                        .or_else(|| complete_resolved(e, scope, schema))
                })
                .collect::<Vec<_>>(),
        )
    });
    // The set is judged at the reach the pass chose its target at (`set_reach`), so a
    // branch the computed reader cannot type leaves the gate the written reading — the
    // refusals it made before that reader existed — rather than a fold of the typed
    // branches alone that answers what neither reading would.
    let reach = crate::planner::unification_branches(expr).map_or(
        BranchReach::Computed,
        |(branches, _)| {
            set_reach(branches.iter().copied(), |e| {
                branch_type_with_subqueries(e, scope, schema).map(|t| t.oid)
            })
        },
    );
    branch_unification_legality(
        expr,
        &|e| {
            let shallow = match (reach, e) {
                (BranchReach::Written, Expr::Subquery { .. }) | (BranchReach::Computed, _) => {
                    branch_type_with_subqueries(e, scope, schema)
                }
                (BranchReach::Written, _) => branch_type_with_reach(e, scope, BranchReach::Written)
                    .map(|oid| ResolvedType {
                        oid,
                        enum_type: enum_identity_in(e, scope, Some(schema)),
                    }),
            };
            if widen {
                shallow.or_else(|| complete_resolved(e, scope, schema))
            } else {
                shallow
            }
        },
        &|e| is_untyped_branch(e, scope, schema),
        schema,
    )
}

/// [`concrete_branch_type`] with the one arm it cannot carry: a bare scalar subquery,
/// which holds no type of its own and needs a catalog to be read at all.
///
/// The coercion pass has already wrapped every branch subquery whose type a `Cast`
/// carries ([`type_scalar_subquery_branches`]), so what reaches here bare is a subquery
/// still deferring or one the pass could not wrap (an enum, `name`) — and that second
/// group is what this reader adds over the pass's: `COALESCE((SELECT current_user), i)` is `42804`
/// because the gate sees the `name` the pass could not wrap, while
/// `COALESCE((SELECT current_user), tx)` answers, because the fold carries `name`'s
/// string-category rows. A branch the gate can type is one the gate can *judge*; nothing
/// here says the verdict is a refusal.
fn branch_type_with_subqueries(
    expr: &Expr,
    scope: &ColumnScope,
    schema: &Schema,
) -> Option<ResolvedType> {
    match expr {
        Expr::Subquery { plan } => branch_subquery_type(plan, scope, schema),
        _ => concrete_branch_resolved(expr, scope, Some(schema)),
    }
}

/// [`is_untyped_column`] with that same arm — a bare scalar subquery defers the gate only
/// while its type is **not yet** resolved, never permanently.
///
/// Answering "untyped" for every bare subquery is what made the gate blind to the whole
/// branch set around one: `COALESCE((SELECT * FROM emp_i), i, d)` answered a row where
/// 18.6 refuses `42804` over `i` and `d` alone, and a set left unified by neither pass nor
/// gate declared one type over per-branch runtime values — a binary client's malformed
/// row.
fn is_untyped_branch(expr: &Expr, scope: &ColumnScope, schema: &Schema) -> bool {
    match expr {
        Expr::Subquery { plan } => branch_subquery_type(plan, scope, schema).is_none(),
        _ => is_untyped_column(expr, scope),
    }
}

/// [`resolve_expr_types_checked`] followed by the static semantic gate
/// ([`check_expr_semantics`]).
///
/// Both passes resolve a column reference through the same `scope`, so the gate
/// cannot reject a pair the coercion pass itself created. See [`ColumnScope`].
///
/// The gate is **one** walk, not two: operand legality and branch unification are
/// decided per node, operand first — see [`check_expr_semantics`] for why the ordering
/// has to live inside the walk rather than between two of them.
///
/// # Errors
///
/// A constant string cast that can't be coerced (`22P02` / `22007` / …), a comparison
/// with no operator for its operand types (`42883`), or a branch set with no common
/// type (`42804` / `42846`).
pub fn resolve_expr_types_gated(expr: Expr, scope: &ColumnScope, schema: &Schema) -> Result<Expr> {
    let expr = type_branch_subqueries(expr, scope, schema)?;
    resolve_and_gate(expr, scope, schema)
}

/// [`resolve_expr_types_gated`] over expressions that each **define a relation's
/// column**, with the type each is certainly of (`certain_entry_type`).
///
/// The expressions are a target list's entries or a `VALUES` row's cells, resolved in
/// order. Each entry's type is read from the entry as it was written: before the pass
/// resolves it, and before the pass writes its own cast over a branch subquery in it —
/// a cast that names a type the sub-plan need not vouch for
/// (`type_scalar_subquery_branches`).
///
/// # Errors
///
/// As [`resolve_expr_types_gated`], for the first entry that raises.
pub fn resolve_entries_gated(
    exprs: Vec<Expr>,
    scope: &ColumnScope,
    schema: &Schema,
) -> Result<(Vec<Expr>, Vec<Option<TypeOid>>)> {
    let mut certain = Vec::with_capacity(exprs.len());
    let mut resolved = Vec::with_capacity(exprs.len());
    for mut expr in exprs {
        certain.push(certain_entry_type(&mut expr, scope, schema));
        let expr = type_branch_subqueries(expr, scope, schema)?;
        resolved.push(resolve_and_gate(expr, scope, schema)?);
    }
    Ok((resolved, certain))
}

/// [`type_scalar_subquery_branches`], where a scalar subquery sits somewhere below.
///
/// The expression's aggregates are decided first ([`decide_aggregates`]). The casts this
/// writes are the pass's own, and a verdict is read off an argument as it was written:
/// the one function that writes them is therefore the one that sees no aggregate reach
/// them undecided.
fn type_branch_subqueries(mut expr: Expr, scope: &ColumnScope, schema: &Schema) -> Result<Expr> {
    if !crate::planner::expr::expr_contains_subquery(&expr) {
        return Ok(expr);
    }
    if contains_aggregate(&expr) {
        decide_aggregates(&mut expr, scope, schema);
    }
    type_scalar_subquery_branches(expr, scope, schema, false)
}

/// The two walks behind [`resolve_expr_types_gated`], over an expression whose branch
/// subqueries are typed.
fn resolve_and_gate(expr: Expr, scope: &ColumnScope, schema: &Schema) -> Result<Expr> {
    let resolved = resolve_expr_types_checked(expr, scope)?;
    check_expr_semantics(&resolved, scope, schema)?;
    Ok(resolved)
}

/// The type an expression that defines a relation's column is **certainly** of, or
/// `None` — a target-list entry, a `VALUES` cell, a FROM `unnest`'s array.
///
/// Certain is what the computed-reach branch reader ([`BranchReach::Computed`]) types
/// in the expression **as written**: the reader, and the reading, that decide an
/// aggregate argument's certainty ([`aggregate_arg_certainty`]). A column a relation
/// exposes is then certain where the expression defining it is
/// ([`ColumnScope::column_type_certain`]), so the one reader says so for a column at any
/// depth of derivation. A scalar subquery in the expression is certain where its
/// sub-plan's own column is ([`certain_scalar_subquery_type`]): the entry is not read
/// through the cast the pass writes over a branch subquery, which names a type whether
/// or not that column is a guess. A fold left at the written reach — a branch over a
/// column whose type is a guess, or over a scalar subquery whose column is one — is
/// typed by the walkers' guess, and is not typed here.
///
/// The expression's own aggregates are decided first, by that producer and from their
/// arguments as written — the verdict this pass records when it reaches them, and keeps
/// ([`resolve_aggregate`]). The reader types an aggregate from its recorded verdict, and
/// the pass has not reached these yet.
pub(super) fn certain_entry_type(
    expr: &mut Expr,
    scope: &ColumnScope,
    schema: &Schema,
) -> Option<TypeOid> {
    if contains_aggregate(expr) {
        decide_aggregates(expr, scope, schema);
    }
    branch_type_in(expr, scope, BranchReach::Computed, &|plan| {
        certain_scalar_subquery_type(plan, schema)
    })
}

/// The subquery leaf of [`branch_type_in`]: the type a scalar subquery is read at, or
/// `None`.
type SubqueryLeaf<'a> = &'a dyn Fn(&Plan) -> Option<TypeOid>;

/// The [`SubqueryLeaf`] of a reader that holds no catalog: a scalar subquery is not
/// typed.
const fn no_subquery_type(_: &Plan) -> Option<TypeOid> {
    None
}

/// The type a scalar subquery is **certainly** of, or `None` — the [`SubqueryLeaf`] of
/// the two readings that record a certainty ([`certain_entry_type`],
/// [`decide_aggregates`]), and what the pass writes its cast over a subquery under a
/// fold from ([`type_scalar_subquery_branches`]).
///
/// It is the type of the sub-plan's single output column where that column is certain
/// by its own record (`ColumnDesc::type_certain`, which [`Plan::output_columns`] derives
/// from what this pass read the sub-plan's defining expression at when the sub-plan was
/// lowered). The sub-plan's verdict is not taken again here, so it is the one the
/// sub-plan's own run took: a sub-plan whose column turns on an enclosing query's
/// column was read under a scope that could not type that column, and the correlated
/// re-run rewrites its expressions and leaves its record as it was.
///
/// An enum's column and a `name` one are not answered: no `Cast` names an enum, and
/// `name` is no cast's target, so a fold reading such a column as a typed branch could
/// not convert a sibling to it ([`SubqueryType::Uncastable`]).
///
/// The answer is the same wherever the subquery stands — the entry itself, an operand
/// of arithmetic or of a function, a branch of a fold — and reads the sub-plan's
/// descriptor alone, which [`Plan::output_columns`] derives in one descent of a nest of
/// subqueries.
fn certain_scalar_subquery_type(plan: &Plan, schema: &Schema) -> Option<TypeOid> {
    #[cfg(test)]
    CERTAIN_SUBQUERY_READS.with(|reads| reads.set(reads.get() + 1));
    let columns = plan.output_columns(schema);
    let [column] = columns.as_slice() else {
        return None;
    };
    (column.type_certain
        && column.enum_type.is_none()
        && !matches!(column.type_oid, TypeOid::Name | TypeOid::NameArray))
    .then_some(column.type_oid)
}

/// The element type a FROM-clause `unnest`'s array is certainly of, or `None`.
///
/// It is `certain_entry_type`'s reading of the array as the parser hands it over, with
/// no column in scope — the namespace the node's own columns are typed in
/// (`function_scan_column_types`).
///
/// An `ARRAY[…]` constructor is not read. The reader types one at its elements' common
/// type, and it is this pass that converts the elements to that type; the pass resolves a
/// FROM `unnest`'s array only on the correlated re-run over a sub-plan, so in a
/// statement's own `FROM` the constructor holds its elements as they were written —
/// `unnest(ARRAY[NULL::int, '7'])` a `text` `'7'` under an `integer` column — and its
/// column's type is the walkers' reading of the first element (`EDGES.md` row 308). What
/// the reader types beside a constructor is an array a cast converts. The array holds no
/// aggregate: the parser refuses one in a FROM function's argument before it asks this.
#[must_use]
pub fn certain_unnest_element_type(array: &Expr) -> Option<TypeOid> {
    if matches!(array, Expr::Array(_)) {
        return None;
    }
    branch_type_with_reach(array, &ColumnScope::default(), BranchReach::Computed)
        .and_then(TypeOid::array_element)
}

/// The type a FROM-clause function's argument is certainly of as written, or `None`.
///
/// The computed-reach reading with no column in scope — the reader
/// [`certain_unnest_element_type`] takes an array's element type from. The argument
/// holds no column, sub-plan or aggregate: `planner::resolve_generate_series` refuses
/// one before it asks this.
#[must_use]
pub fn certain_from_argument_type(arg: &Expr) -> Option<TypeOid> {
    branch_type_with_reach(arg, &ColumnScope::default(), BranchReach::Computed)
}

/// Record [`aggregate_arg_certainty`]'s verdict on every aggregate of `expr` that
/// carries none yet, at this query level — a sub-plan's are its own level's.
fn decide_aggregates(expr: &mut Expr, scope: &ColumnScope, schema: &Schema) {
    if let Expr::Aggregate {
        func,
        arg,
        certainty: certainty @ ArgCertainty::Undecided,
        ..
    } = expr
    {
        *certainty = aggregate_arg_certainty(*func, arg, scope, &|plan| {
            certain_scalar_subquery_type(plan, schema)
        });
    }
    expr.for_each_child_mut(&mut |child| decide_aggregates(child, scope, schema));
}

/// Resolve types throughout `expr` with **no columns in scope**, then validate — the
/// entry point for the **no-FROM** `SELECT` path (`SELECT true = 'yes'`, `SELECT CASE
/// … END`).
///
/// Runs [`resolve_expr_types_gated`] against an empty scope: comparison-operand
/// coercion (the concrete-literal-vs-string rule — `1 <> '1 '`, `'abc' = 1`),
/// `CASE`/`COALESCE` branch unification, `greatest`/`least`/`nullif` common-type
/// unification (else a `greatest(1::int4, NULL::int8)` would declare int8 but return an
/// int4 value — a binary-protocol mis-decode), and eager constant-cast validation (so
/// an unparseable *untaken* branch — `CASE WHEN 1=1 THEN 1 ELSE 'x' END`,
/// `COALESCE(NULL, 1, 'x')` — fails loud at plan time like Postgres, not just the
/// reached branch). Only literals and casts drive the types.
///
/// It also runs the static operand gate ([`check_expr_semantics`]) — with no
/// columns in scope every operand is a literal or a cast, so the gate is at its most
/// decisive here: `SELECT 1 = true` and `SELECT CASE 1 WHEN 1 THEN 'a' WHEN true THEN
/// 'b' END` both fail `42883` at plan time, exactly as Postgres does (and regardless
/// of which `WHEN` arm would have matched first).
///
/// **The schema goes through to the gate**, and that is not a detail of the plumbing: the
/// only operand here a scope cannot type is a scalar subquery, and a gate handed no
/// catalog defers on one forever. Running the branch pass separately and gating without a
/// schema left this clause the one place `SELECT COALESCE((SELECT current_user), 1)`
/// answered `postgres` where 18.6 refuses `42804` — the one clause where the gap could
/// not be seen from the FROM-bearing clauses that carry the schema already.
///
/// # Errors
///
/// A constant string cast that can't be coerced (`22P02` / `22007` / …), a
/// comparison with no operator for its operand types (`42883`), or a branch set with no
/// common type (`42804` / `42846`).
pub fn resolve_expr_types_no_columns(expr: Expr, schema: &Schema) -> Result<Expr> {
    let scope = ColumnScope::default();
    resolve_expr_types_gated(expr, &scope, schema)
}

/// One relation visible to the comparison gate: the name a **qualified** reference
/// must spell, plus that relation's `column → ResolvedType` map.
#[derive(Debug, Clone)]
struct ScopeRelation {
    /// The FROM alias if one was given, else the table name — Postgres hides the
    /// table name behind an alias (`FROM oc o` makes `oc.x` a `42P01`), so one name
    /// per relation is the whole qualifier surface.
    ///
    /// `None` is a relation this scope's builder could not name at all, which today is
    /// only a plan node the level walker has no model for. It matches **no** qualifier
    /// and, being non-exhaustive, makes every level it sits on uncertain — so the
    /// question "does this qualifier name nothing anywhere" can no longer be answered,
    /// which is exactly what an unnameable relation costs.
    name: Option<String>,
    columns: HashMap<String, ResolvedType>,
    /// Whether `columns` is **every** column this relation carries.
    ///
    /// `false` says a name absent from the map might still belong here, so resolution
    /// stops rather than walking to an enclosing level and binding to a relation that
    /// merely shares the name. Three kinds are non-exhaustive, and the list is the
    /// contract rather than the count — re-derive it from the two builders
    /// (`build_column_scope` and `push_level_relations`) rather than carrying it forward:
    /// a relation whose columns the builder could not enumerate at all (a set-op node, a
    /// plan node the level walker has no model for — [`ColumnScope::push_opaque_relation`],
    /// which also has no name); a scan of a table the schema has no entry for, DML target
    /// included (the *name* is known, the columns are not); and a **catalog view**, where
    /// pglet models a strict subset of 18.6's columns for nine of the 17 relations it has
    /// and binds seven of those non-exhaustively (`CatalogTable::columns_complete`) — so
    /// an exhaustive gate over one of the seven would turn a query 18.6 answers into a
    /// false `42703`.
    ///
    /// A FROM-clause `unnest(…)` is **not** one of them, at either builder: Postgres's
    /// function RTE has exactly its declared columns and both walks read them off the
    /// node's own `output_columns`. It was the fourth entry here until the flip, and the
    /// entry that did not belong — the map was not a subset of anything, only a list
    /// nobody had gone to fetch.
    exhaustive: bool,
    /// A **pseudo-relation** — today only `EXCLUDED`, the would-be-inserted row an
    /// `ON CONFLICT DO UPDATE` sees. It differs from a FROM relation in two ways.
    ///
    /// It is reachable only **qualified**, but not because Postgres binds a bare `col`
    /// there to the target row — Postgres puts *both* relations in scope, and since
    /// `excluded` carries the target's whole shape, a bare reference to any target
    /// column is `42702` (`SET n = n + 1` and `WHERE n > 45` alike). The parser rejects
    /// that spelling up front, so by the time a reference reaches this scope a bare
    /// name can only mean the target — which is why [`ColumnScope::bare_candidates`] skips
    /// pseudo-relations rather than counting them as a second claimant.
    ///
    /// Its name is also a fixed spelling rather than a user identifier, but it is
    /// matched **exactly** like every other qualifier — the parser downcases an
    /// unquoted `EXCLUDED` before this scope sees it, so `"EXCLUDED".n` correctly
    /// fails to resolve here as it does on Postgres (`42P01`).
    ///
    /// The flag survives because the *other* difference is real: [`ColumnScope::bare_candidates`]
    /// skips pseudo-relations.
    pseudo: bool,
    /// The **query level** this relation belongs to: `0` is the level being resolved,
    /// `1` its immediately enclosing query, and so on outward. Every scope built by a
    /// single query's lowering is entirely level `0`; only [`ColumnScope::nested_in`]
    /// mints higher levels. See [`ColumnScope`] § Level chaining.
    level: u16,
    /// Whether this is a **base table** registered with the catalog's own column types
    /// ([`ColumnScope::push_base_relation`]) — not a derived table, a CTE, a catalog
    /// view, an `unnest`, or a relation list whose kinds the builder does not carry.
    base: bool,
    /// The columns of a non-base relation whose types are **certain** — those its
    /// descriptors say so for (`ColumnDesc::type_certain`), which the plan derives from
    /// what this pass read each column's defining expression at as written
    /// ([`ColumnScope::push_derived_relation`], [`certain_entry_type`]). A base
    /// relation's are all certain.
    certain: std::collections::HashSet<String>,
    /// The catalog table whose **stored rows** this relation's values are read from — a
    /// FROM scan of a base table, or an `UPDATE` / `DELETE` target — which
    /// `pg_column_size` sizes as 18.6's heap holds them ([`ColumnScope::stored_column`]).
    /// `None` for every other relation, a base one included where its values are not
    /// read from a stored row: `RETURNING`'s target.
    stored: Option<StoredTable>,
}

/// The catalog table a [`ScopeRelation`] reads stored rows of, with the column facts
/// `pg_column_size` needs that its `ResolvedType`s do not carry.
#[derive(Debug, Clone)]
struct StoredTable {
    /// The catalog name, which keys the store — not the relation's effective name.
    table: String,
    /// The columns whose type is an enum: 18.6 stores those as four-byte OIDs.
    enums: std::collections::HashSet<String>,
    /// The columns with a character-length cap, which may be a `char(n)` 18.6 pads and
    /// pglet does not (`EDGES.md` row 95).
    capped: std::collections::HashSet<String>,
}

/// What a **closed** [`ColumnScope`] can say about one column reference.
///
/// Three outcomes and not two: beside *it binds* and *it does not*, a scope holding a
/// relation it could not name or could not enumerate must be able to say **it cannot
/// tell** — otherwise every catalog view, every scan of a table the schema lacks, and
/// every plan node the level walker has no model for turns an answerable query into a
/// false `42703`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binding {
    /// A relation in scope carries it.
    Bound,
    /// No level names the qualifier, and every level walked was name-known — `42P01`.
    NoRelation,
    /// The relation that answered the qualifier (or every level, for a bare name) is
    /// columns-exhaustive and has no such column — `42703`.
    NoColumn,
    /// Two relations at the innermost claiming level carry a bare name — `42702`.
    Ambiguous,
    /// A relation this scope could not name or enumerate might carry it; the verdict
    /// belongs to the executor.
    Uncertain,
}

/// The relations a column reference resolves against.
///
/// The **single** column-resolution mechanism the planner has, shared by the coercion
/// pass ([`resolve_expr_types`]) and the static comparison gate
/// ([`check_expr_semantics`]).
///
/// It is one mechanism on purpose. The coercion pass used to look a column up by its
/// **bare** name in a flat map merged across every FROM relation, where a later table
/// overwrote an earlier one on a name collision, while the gate resolved the same
/// reference qualifier-aware. Two relations sharing a column name was then enough for
/// the two to reach different conclusions about what `o.x` denotes, which cost answers
/// in both directions: the coercion pass cast a literal to a *sibling* relation's type
/// (`o.x = '1.1'` with `o.x float8` beside an `x float4` rounded the literal and
/// silently stopped matching — a lost row on a read and a lost row on an `UPDATE`),
/// and the gate had to be switched off wholesale over any shared-name scope to keep it
/// from rejecting the queries the miscast produced. Resolution is therefore
/// qualifier-aware, once, for both:
///
/// - a **qualified** `q.c` resolves only against the relation named `q`, so a
///   qualifier from an enclosing (or sibling) scope answers `None`;
/// - a **bare** `c` resolves per the caller's rule — [`Self::resolve`] for the gate
///   (one claimant), [`Self::coercion_type`] for the coercion pass (claimants agree).
///
/// Every FROM relation is registered under the name a qualified reference must spell —
/// though not always with its own columns: `build_column_scope` resolves that name
/// against the catalog first, so a FROM-clause `unnest(…)` aliased to a base table's
/// name registers the *table's* columns (a recorded divergence in both polarities,
/// `EDGES.md`). A FROM-clause `unnest(…)` used to be the exception — its columns were not
/// materialized here, so it registered under its alias with an empty map and then, once
/// the map was filled, still as a *subset* — and both readings cost answers. What
/// remains registered as a subset is listed on [`ScopeRelation::exhaustive`], and each
/// entry there is a relation whose real column list this planner genuinely cannot
/// produce, never one it merely has not been taught to walk.
///
/// `None` is the **conservative** side of a trade, not a no-op. Declining inserts no
/// cast and leaves the comparison to the eval-time gate — which is harmless only when
/// the column it declined to type is string-typed, since `text = text` is an operator
/// Postgres has. Decline on any other column and the gate sees `<column type> = text`,
/// finds no operator, and raises `42883` on a query Postgres runs (a correlated
/// `o.u = '<uuid>'` against a uuid `o.u` is the shape). So a missed resolution costs a
/// **false rejection**, not a silent wrong answer — the direction worth erring in, but
/// an error all the same. Widening what this can resolve narrows that over-rejection;
/// see `EDGES.md` for the clause positions it never reaches.
///
/// # Level chaining
///
/// A correlated subquery's expressions are lowered **before** the enclosing query's
/// scope exists (the parser is fused with the planner, so `EXISTS (…)` is fully
/// lowered while the outer query's clauses are still being parsed). The enclosing
/// relations are therefore attached afterwards, by [`Self::nested_in`], which stamps
/// this scope's relations level `0` and the enclosing chain's level `1`, `2`, … —
/// and the resolvers search **level by level, innermost first**.
///
/// That reproduces Postgres's `colNameToVar` (`parse_relation.c`) exactly: it searches
/// one level's namespace in full, raises `42702` on two claimants **at that level**,
/// and walks outward only when the level yields *zero*. So the innermost level wins
/// outright and cross-level ambiguity is not a thing — a bare `x` that the subquery's
/// own FROM carries is never in doubt because an outer relation also has one.
///
/// A level is **uncertain** when any relation on it is non-exhaustive
/// ([`ScopeRelation::exhaustive`]): a set-op node, a plan node this walker has no model
/// for, a scan of a table the schema has no entry for, or a catalog view whose columns
/// pglet models as a strict subset. A name absent from such a level might still belong to
/// it, so resolution **stops** there rather than walking outward and binding the reference
/// to an enclosing relation that merely shares the name — which would be a silently wrong
/// cast, the one outcome worse than the over-rejection this chaining exists to remove.
/// Stopping costs only the fix; it never resolves anything new.
///
/// Making that a per-**relation** fact rather than a per-level flag is what lets a
/// relation the scope can *name* and cannot fully enumerate answer its own qualifier
/// while still stopping the walk. It also removed the flag's only ordinary-SQL inhabitant:
/// a FROM-clause `unnest(…)` is a named relation with exactly-known columns, so it
/// registers rather than stopping anything, closing both halves of a recorded
/// over-rejection. **Both** scope builders read it that way now — `push_level_relations`
/// always did, `build_column_scope` since the flip that closed the own-level half of the
/// same class (`select/unnest_bogus_bare_name_own_level_loud.sql` and its siblings), so
/// there is no longer a shape whose verdict depends on which builder saw it.
///
/// # Closed scopes
///
/// A scope is **closed** ([`Self::closed`]) when it enumerates every relation and every
/// column a reference resolved through it may name. Resolution then answers a question
/// beside "what type is this" — *does this reference bind at all* — and one that does
/// not is an error rather than an unknown ([`Self::check_binding`]).
///
/// Open is the default, and not out of timidity: a scope built for a `WHERE` or a `SET`
/// right-hand side can be missing a relation it could not enumerate, so declining there
/// means "I don't know" and rejecting on it would refuse queries Postgres runs. Only a
/// caller that can say *this is the whole namespace* may close its scope. Today that is
/// `RETURNING`, whose namespace is the DML target under its effective name plus the
/// relations its `FROM` introduces — see `lower_returning`.
#[derive(Debug, Clone, Default)]
pub struct ColumnScope {
    relations: Vec<ScopeRelation>,
    /// Whether this scope is the **whole** namespace for the references resolved
    /// through it — see [`Self`] § Closed scopes.
    closed: bool,
    /// Whether this scope is a nested level's, chained onto its enclosing levels by
    /// [`Self::nested_in`] — whether or not they contribute a relation.
    nested: bool,
    /// Bare names a `JOIN … USING (c)` **merges**, so exactly one column carries each
    /// however many sides declare it.
    ///
    /// The relations in this scope are the join's *inputs*, each with its own `c`, so
    /// counting claimants over them calls a merged name ambiguous where Postgres has one
    /// column — `SELECT num FROM a JOIN b USING (num)` is ordinary SQL. The merge is a
    /// property of the FROM plan rather than of any relation in it, which is why it is
    /// recorded beside them instead of inside one.
    merged_names: Vec<(u16, String)>,
    /// The **output** column names of the query level this scope resolves a
    /// `ORDER BY` / `DISTINCT ON` key for, which a bare single-field key may name
    /// (`findTargetlistEntrySQL92`) even though no FROM relation carries it.
    ///
    /// A sibling of [`Self::merged_names`] rather than a second spelling of it, because
    /// the two shift differently: a `USING` merge is a fact about a *level's FROM* and
    /// travels outward with that level under [`Self::nested_in`], while a target list is
    /// visible only to its own level's sort clauses — measured, `SELECT v AS lbl FROM ot
    /// ORDER BY (SELECT w FROM ob ORDER BY lbl LIMIT 1)` is `42703` on 18.6 and here, so
    /// these must **not** shift. They are therefore
    /// level-0 only and unqualified: `findTargetlistEntrySQL92` takes its branch for a
    /// single-field name and nothing else.
    ///
    /// That statement is now a fixture
    /// (`select/nested_sort_key_may_not_name_the_enclosing_output.sql`) rather than only
    /// a sentence — but read what it guards narrowly, because it was measured and it is
    /// less than the sentence suggests. `super::correlation::key_scope`'s names are
    /// pushed only for a key that *is* a bare `ColumnRef`, and such a key carries no
    /// sub-plan for `descend_subplans` to recurse into, so no scope holding
    /// `output_names` reaches [`Self::nested_in`] at all today. Measured as two
    /// mutations: propagating them there alone reddens **nothing** (411 `select/`
    /// fixtures green), and doing it beside a revert of that shape gate reddens the
    /// fixture (`0A000` against 18.6's `42703`) plus the two the gate owns. So the rule
    /// here is a second lock on a door the shape gate already shuts, and the fixture pins
    /// the *answer* rather than either mechanism.
    ///
    /// The alternative this replaced — a blanket flag declining to judge *every* bare
    /// name — is what a key binding to nothing rode through
    /// `resolve_correlated_subplans`, and it wrote: `DELETE FROM dt WHERE a NOT IN
    /// (SELECT a FROM du ORDER BY bogus)` over an empty `du` removed every row of `dt`
    /// where 18.6 is `42703`, since a statement matching no row never reaches the
    /// eval-time resolver either. The same blanket stood at a level's **own** lowering
    /// until the keys moved below the projection, where the claimant set finally exists
    /// (`super::correlation::target_list_entries`); the writes it cost there are the
    /// closed members of `EDGES.md`' sort-key bullet.
    output_entries: Vec<OutputEntry>,
}

/// One target-list entry a bare single-field `ORDER BY` / `DISTINCT ON` key may name,
/// as `findTargetlistEntrySQL92` reads it: an output **name** (`resname`) beside the
/// entry's own **expression**, which is what decides whether two entries under one name
/// are ambiguous or one column written twice.
///
/// The pair is the whole point. Postgres scans the target list for `resname == name` and
/// raises `42702` only when two matching entries are not `equal()` — *"we allow SELECT a,
/// a FROM table ORDER BY a but not SELECT a AS b, b FROM table ORDER BY b"*, its own
/// worked example. Counting by **name** alone refuses the first of those: measured on
/// 18.6 over `ja2(c, x)`, `SELECT k FROM ot2 WHERE EXISTS (SELECT *, c FROM ja2 WHERE
/// false ORDER BY c)` answers `(0 rows)` there and was `42702` here, as were
/// `SELECT *, x … ORDER BY x`, `SELECT c AS c, c … ORDER BY c`, `SELECT ja2.c, c …
/// ORDER BY c`, `SELECT c, c FROM ja2 GROUP BY c ORDER BY c` and
/// `SELECT DISTINCT ON (c) ja2.c, c`.
#[derive(Debug, Clone)]
pub struct OutputEntry {
    /// The entry's output name — Postgres's `resname`, the only thing the name scan
    /// compares against.
    pub name: String,
    /// The entry's expression, or `None` when the node the key reads from is **not** a
    /// target list.
    ///
    /// The caller supplies the identity in the shape `equal()` compares — Postgres
    /// matches transformed `Var`s, so the `AS` label (which lives on the `TargetEntry`,
    /// not on the expression) is stripped and a bare column reference is bound to the
    /// relation carrying it, or `SELECT ja2.c, c` reads as two different columns.
    ///
    /// `None` is not "unknown": it is a row that is a **range table** row rather than a
    /// target list — a join or a scan under a sort placed below the projection — where
    /// two entries sharing a name are two distinct columns, so Postgres would compare two
    /// distinct `Var`s and `equal()` would be false. Each `None` therefore counts as its
    /// own claimant, which is what keeps a bare key two of a join's columns claim on
    /// `42702`.
    pub expr: Option<Expr>,
}

/// One target-list entry's identity as **bytes** — what `equal()` compares — or `None`
/// when it cannot be derived.
///
/// The bytes are [`super::expr::encode_expr`]'s, the single owner of the planner-IR
/// wire format — the **ungated** encoder, deliberately, not the persistence-bound
/// [`super::expr::serialize_expr`] that wraps it: an entry here is compared, never
/// written, and a plan-bearing one is ordinary (18.6 accepts two `equal()` `SubLink`s
/// under a bare `ORDER BY` and `42702`s two that differ, and both verdicts are these
/// bytes). It is a derived, field-by-field postcard encoding, so every field of every
/// nested value participates and two encodings are equal exactly when the trees are
/// structurally identical. That is Postgres's own test — `equal()` on a `Const` is
/// `equalConst` → `datumIsEqual`, a `memcmp` of the datum for a by-reference type and a
/// Datum-word compare for a by-value one (`nodes/equalfuncs.c`, `utils/adt/datum.c`) —
/// and it is deterministic here because nothing reachable from an `Expr` is a map: the
/// whole closure — `Plan` included, which an `Expr::Subquery` carries, and the
/// `pglet_core` value types under it — is `Box`/`Vec`/`Option`/scalars, **plus two
/// shapes that description used to elide** — a foreign `Serialize`, and a fixed-size
/// array — each still deterministic (a fixed field or element order, nothing iterating a
/// hashed container) but for its own reason rather than by being a scalar:
/// `Value::Timestamp` and `Value::Date` carry a hand-written `Serialize` (the Postgres
/// binary format's microsecond / day count, `pglet_core::types::Infinite`) rather than a
/// derive, and `Value::Uuid` a fixed `[u8; 16]`. A map's iteration order is what would make two equal
/// trees encode differently. That is the same reason `catalog::Schema`'s collections are
/// `BTreeMap`, and it is a property to re-check rather than inherit if a map is ever
/// added to the plan IR — **guarded now**, by `planner::expr`'s
/// `serialize_expr_survives_reserialization_byte_stable`, the analogue of
/// `catalog::schema`'s `serialize_schema_survives_reserialization_byte_stable`: a
/// `HashMap` added to any shape that guard's corpus reaches breaks the round-trip, since
/// deserializing rebuilds the map and re-serializing walks it in a fresh order. The guard is a **necessary**
/// condition and not the property itself — it shows that one tree re-encodes to its own
/// bytes, not that two independently built equal trees encode alike — which is why the
/// closure argument above stays written out rather than being replaced by the test. Its
/// **plan-bearing** half cannot take that route at all, since a `Plan` does not decode;
/// `plan_bearing_trees_encode_identically` covers those three variants by building each
/// tree twice, which is the property itself rather than the necessary condition.
///
/// **The rule this replaced, worth carrying: a Rust `PartialEq` that any value type has
/// customised for *semantic* equality is not Postgres's `equal()`, and adopting one as a
/// structural identity over-accepts in exactly the direction that writes.** `Expr`
/// derives `PartialEq`, but two of the types beneath it do not: `PgNumeric` compares by
/// value, so `1.5 == 1.50`, and `PgInterval` compares by Postgres's estimate, so
/// `INTERVAL '30 days' == INTERVAL '1 mon'`. Both are correct and deliberate for SQL
/// value semantics and wrong for this. Used here, they folded two entries 18.6 calls
/// ambiguous into one claimant and the `42702` was lost — measured, with `ja2(c, x)` and
/// `w1(a)` holding two rows: `DELETE FROM w1 WHERE NOT EXISTS (SELECT 1.5 AS q, 1.50 AS q
/// FROM ja2 WHERE false ORDER BY q)` was `DELETE 2` here against 18.6's `42702` and its
/// two surviving rows, and the same statement over `d + INTERVAL '1 mon' AS q, d +
/// INTERVAL '30 days' AS q` — two expressions that compute **different dates** — did the
/// same. The `UPDATE` twin wrote over every row.
fn structural_identity(expr: &Expr) -> Option<Vec<u8>> {
    super::expr::encode_expr(expr).ok()
}

impl ColumnScope {
    /// A scope over the one relation `name` with `columns` — the DELETE / UPDATE
    /// target, and a single-table SELECT.
    // internal planner helper; generalizing over the hasher is needless API churn
    #[allow(clippy::implicit_hasher)]
    #[must_use]
    pub fn single(name: String, columns: HashMap<String, ResolvedType>) -> Self {
        let mut scope = Self::default();
        scope.push_base_relation(name, columns);
        scope
    }

    /// Add a relation whose columns are known.
    // internal planner helper; generalizing over the hasher is needless API churn
    #[allow(clippy::implicit_hasher)]
    pub fn push_relation(&mut self, name: String, columns: HashMap<String, ResolvedType>) {
        self.push_scope_relation(Some(name), columns, true, false);
    }

    /// [`Self::push_relation`] for a **base table**, its `columns` the catalog's own —
    /// the one kind of relation whose column types are declared rather than derived
    /// ([`Self::base_column_type`]).
    // internal planner helper; generalizing over the hasher is needless API churn
    #[allow(clippy::implicit_hasher)]
    pub fn push_base_relation(&mut self, name: String, columns: HashMap<String, ResolvedType>) {
        self.push_relation(name, columns);
        if let Some(relation) = self.relations.last_mut() {
            relation.base = true;
        }
    }

    /// [`Self::push_base_relation`] for `table` under the effective name `name`, as a
    /// relation whose values are read from the table's stored rows — a FROM scan, an
    /// `UPDATE` / `DELETE` target.
    pub fn push_stored_relation(&mut self, name: String, table: &crate::catalog::Table) {
        self.push_base_relation(name, build_column_type_map(&table.columns));
        self.mark_stored(table);
    }

    /// Record that the relation pushed last reads `table`'s stored rows — for a scope
    /// whose builder pushed it as a base relation already, the `UPDATE` / `DELETE` /
    /// `ON CONFLICT` target. `pg_column_size` sizes such a relation's columns as 18.6's
    /// heap holds them.
    pub fn mark_stored(&mut self, table: &crate::catalog::Table) {
        if let Some(relation) = self.relations.last_mut() {
            let names = |keep: fn(&crate::catalog::Column) -> bool| {
                table
                    .columns
                    .iter()
                    .filter(|c| keep(c))
                    .map(|c| c.name.clone())
                    .collect()
            };
            relation.stored = Some(StoredTable {
                table: table.name.clone(),
                enums: names(|c| c.enum_type.is_some()),
                capped: names(|c| c.varchar_length.is_some()),
            });
        }
    }

    /// The origin of `pg_column_size`'s argument `[qualifier.]name` when it is a column
    /// of exactly one relation at this level, read from that relation's stored rows
    /// ([`ScopeRelation::stored`]). `None` for anything else — a name no relation of the
    /// level can be shown to own alone, an enclosing level's, a derived relation's, and a
    /// length-capped column, which may be a `char(n)` whose padding pglet does not hold.
    fn stored_column(&self, qualifier: Option<&str>, name: &str) -> Option<ColumnSizeOrigin> {
        let mut found = None;
        for relation in &self.relations {
            if relation.level != 0
                || (relation.pseudo && qualifier.is_none())
                || qualifier.is_some_and(|q| relation.name.as_deref() != Some(q))
            {
                continue;
            }
            if relation.columns.contains_key(name) {
                if found.is_some() {
                    return None;
                }
                found = Some(relation);
            } else if !relation.exhaustive {
                return None;
            }
        }
        let stored = found?.stored.as_ref()?;
        (!stored.capped.contains(name)).then(|| ColumnSizeOrigin::Stored {
            table: stored.table.clone(),
            is_enum: stored.enums.contains(name),
        })
    }

    /// [`Self::push_relation`] for a relation read off a plan node's output descriptors —
    /// a derived table, a CTE reference, a FROM-clause `unnest` — recording which of its
    /// columns the descriptors call certain ([`Self::column_type_certain`]).
    pub fn push_derived_relation(&mut self, name: String, columns: &[crate::types::ColumnDesc]) {
        self.push_relation(name, build_desc_type_map(columns));
        if let Some(relation) = self.relations.last_mut() {
            relation.certain = columns
                .iter()
                .filter(|c| c.type_certain)
                .map(|c| c.name.clone())
                .collect();
        }
    }

    /// Whether `[qualifier.]name` resolves, through the coercion pass's reader, to a
    /// column whose type is **certain**: the type this scope holds for it is the type its
    /// values have and the type 18.6's parse analysis gives it, by construction. That is
    /// a base table's column, and a derived relation's where the relation's descriptor
    /// says so (`ScopeRelation::certain`), which the plan derives from this pass's own
    /// reading of the expression defining the column (`certain_entry_type`). A bare
    /// name several relations claim is certain where they agree on the type
    /// (`Self::coercion_type`) and each is one of those: a `JOIN … USING` column merged
    /// from same-typed columns.
    ///
    /// The column leaf of the computed reach (`BranchReach::Computed`), and so of the
    /// one certainty verdict an aggregate's readers read (`aggregate_arg_certainty`) and
    /// of the one a relation's own columns carry outward.
    #[must_use]
    pub fn column_type_certain(&self, qualifier: Option<&str>, name: &str) -> bool {
        let certain = |r: &ScopeRelation| {
            r.columns.contains_key(name) && (r.base || r.certain.contains(name))
        };
        if self.coercion_type(qualifier, name).is_none() {
            return false;
        }
        if let Some(q) = qualifier {
            for level in self.levels() {
                if let Some(relation) = self.relation_at(level, q) {
                    return certain(relation);
                }
            }
            return false;
        }
        for level in self.levels() {
            let mut claimants = self
                .relations
                .iter()
                .filter(|r| r.level == level && !r.pseudo && r.columns.contains_key(name))
                .peekable();
            if claimants.peek().is_some() {
                return claimants.all(certain);
            }
        }
        false
    }

    /// Add a relation whose `columns` are a **subset** of what it really carries — a
    /// catalog view, or a scan of a table the schema has no entry for. Its name answers
    /// a qualifier; a name its map lacks is *unknown* rather than absent, so resolution
    /// stops at it instead of walking outward.
    // internal planner helper; generalizing over the hasher is needless API churn
    #[allow(clippy::implicit_hasher)]
    pub fn push_partial_relation(&mut self, name: String, columns: HashMap<String, ResolvedType>) {
        self.push_scope_relation(Some(name), columns, false, false);
    }

    /// Add a relation this builder could not name **or** enumerate — a plan node the
    /// level walker has no model for. It answers no qualifier and makes its level
    /// uncertain, which is the old per-level `seal` flag's effect expressed as a relation.
    pub fn push_opaque_relation(&mut self) {
        self.push_scope_relation(None, HashMap::new(), false, false);
    }

    /// The one constructor, so a new kind of relation cannot forget a field.
    fn push_scope_relation(
        &mut self,
        name: Option<String>,
        columns: HashMap<String, ResolvedType>,
        exhaustive: bool,
        pseudo: bool,
    ) {
        self.relations.push(ScopeRelation {
            name,
            columns,
            exhaustive,
            pseudo,
            level: 0,
            base: false,
            certain: std::collections::HashSet::new(),
            stored: None,
        });
    }

    /// Mark this scope as the **whole** namespace for the references resolved through
    /// it, so an unbindable reference is an error rather than an unknown. See [`Self`]
    /// § Closed scopes.
    #[must_use]
    pub const fn closed(mut self) -> Self {
        self.closed = true;
        self
    }

    /// Record the columns a `JOIN … USING (…)` in this scope's FROM merges, so the
    /// binding gate counts one claimant for each rather than one per side.
    pub fn push_merged_names(&mut self, names: impl IntoIterator<Item = String>) {
        self.merged_names.extend(names.into_iter().map(|n| (0, n)));
    }

    /// Record the **target-list entries** a bare single-field `ORDER BY` /
    /// `DISTINCT ON` key resolved through this scope may name — see
    /// [`Self::output_entries`] and [`OutputEntry`].
    pub fn push_output_entries(&mut self, entries: impl IntoIterator<Item = OutputEntry>) {
        self.output_entries.extend(entries);
    }

    /// The level-0 relation carrying `name`, when exactly one does — the binding a
    /// caller applies to a bare column reference before offering it as an
    /// [`OutputEntry`]'s identity, so `SELECT ja2.c, c` is one expression twice rather
    /// than two.
    ///
    /// Level 0 only, because a target-list entry's `Var` references *this* query level's
    /// range table. It answers `None` on **three** declines, and each leaves the reference
    /// bare — the conservative answer, since two entries then compare equal only if they
    /// were spelled the same way, which is an over-**rejection** and never a write. All
    /// three are live, each measured `(0 rows)` on 18.6 against `42702` here, nested
    /// inside an `EXISTS` so the plan-time gate is the judge:
    ///
    /// - **two or more claimants** — `SELECT *, c FROM ua JOIN ub USING (c) WHERE false
    ///   ORDER BY c`, and its `SELECT ua.c, c …` spelling. The `USING` merge makes one
    ///   column of the two, but both sides carry the name here;
    /// - **an uncertain level** — `SELECT t.table_name, table_name FROM
    ///   information_schema.tables t WHERE false ORDER BY table_name`, where pglet models
    ///   a strict subset of the view's columns so the level is not columns-exhaustive;
    /// - **no claimant** — `SELECT ot2.k AS q, k AS q FROM ja2 WHERE false ORDER BY q`,
    ///   where `k` belongs to the *enclosing* level and no level-0 relation claims it.
    ///
    /// Recorded in `EDGES.md` beside the fourth member of the
    /// same list, which is not this function's: 18.6 has no cast node to compare, so
    /// `SELECT c::int AS q, c AS q` is one entry there and two here.
    #[must_use]
    pub fn sole_relation_carrying(&self, name: &str) -> Option<&str> {
        if self.level_columns_uncertain(0) {
            return None;
        }
        let mut claimants = self
            .relations
            .iter()
            .filter(|r| r.level == 0 && !r.pseudo && r.columns.contains_key(name));
        let first = claimants.next()?;
        if claimants.next().is_some() {
            return None;
        }
        first.name.as_deref()
    }

    /// Whether a `JOIN … USING (…)` at `level` merges `name`, so its claimants there are
    /// one column rather than one per side.
    fn merged_at(&self, level: u16, name: &str) -> bool {
        self.merged_names
            .iter()
            .any(|(l, n)| *l == level && n == name)
    }

    /// Error when `[qualifier.]name` binds to nothing in a **closed** scope; `Ok(())`
    /// always in an open one.
    ///
    /// The two failures are Postgres's own and are not interchangeable: a qualifier no
    /// relation here answers to is `42P01` (`errorMissingRTE`), while a qualifier that
    /// does resolve, or no qualifier at all, over a name no relation carries is `42703`
    /// (`errorMissingColumn`). Reporting one for the other loses the distinction a
    /// client uses to tell a wrong table name from a wrong column name.
    ///
    /// A `*` is not a column reference — it is a wildcard the caller expands, and
    /// `count(*)`'s argument is spelled the same way — so only its **qualifier** is
    /// checked. By the time a `RETURNING q.*` reaches here its qualifier is known to name
    /// no relation of the statement's namespace (every spelling that does was expanded
    /// away), and this is the first of the two places that then reject it; the executor's
    /// `project_returning` is the second.
    ///
    /// A **bare** name is first matched against [`Self::output_entries`], the target-list
    /// entries a single-field `ORDER BY` / `DISTINCT ON` key may name — Postgres's
    /// `findTargetlistEntrySQL92` answers from the target list before it consults the
    /// range table, so a match there binds outright. Two matching entries are `42702`
    /// **only when they are different expressions** ([`Self::output_name_binding`]),
    /// which is both halves of Postgres's rule rather than either alone — though not yet
    /// the whole of it. **Four measured causes still make it refuse what 18.6 resolves**
    /// (four measured, not four proven exhaustive), each
    /// loud (`42702`) and each with more than one spelling: a **no-op cast** on an
    /// already-typed expression (18.6 has no cast node to compare — `coerce_type` returns
    /// the input unchanged — while this keeps the `Cast`); an **unknown-typed literal's
    /// cast** that parse analysis folds to a `Const` there and not here (`true` vs
    /// `'t'::bool`);
    /// an **`IN` list** that `transformAExprIn` normalizes into the `ScalarArrayOpExpr`
    /// over an `ArrayExpr` that `= ANY (ARRAY[…])` also produces, where this keeps a
    /// distinct `Expr::InList` (`c IN (1,2)` vs `c = ANY(ARRAY[1,2])`); and
    /// [`Self::sole_relation_carrying`]'s three declines. A fifth is **closed**, and the
    /// shape of its closure is worth carrying: **unary `-`** was not folded into a numeric
    /// constant here where `doNegate` folds it there, so `0` and `-0` were two entries —
    /// and the *cheap independent observable* of that same cause, `ORDER BY -1`, was not
    /// an over-rejection at all but a **silent write** (`42P10` there, rows here, and an
    /// ordinary filtering `DELETE` carrying that key removed a row). `parser::expr`'s
    /// `negate` folds it now, retiring the cause and the write together. This was recorded
    /// as a count of five shapes; the count was an undercount on arrival and is now an
    /// overcount — the members and
    /// their measurements are in `EDGES.md`, as causes, because
    /// another spelling of a listed cause reads as a falsified number and is not a
    /// finding. Taking the
    /// first match *wrote*: `DELETE FROM rt WHERE a NOT IN (SELECT ja.x FROM ja JOIN jb
    /// ON false ORDER BY n)` over a `ja(n, x)` and a `jb(n, y)` removed **every** row of
    /// `rt` where 18.6 is `42702` and leaves them, since the join matches nothing and no
    /// eval-time resolver ever sees the key. Counting by *name* then refused the other
    /// polarity: `SELECT *, c FROM ja2 WHERE false ORDER BY c` inside an `EXISTS` was
    /// `42702` here against 18.6's `(0 rows)`, the star's `c` and the explicit `c` being
    /// one column written twice.
    ///
    /// A **bare** name is additionally checked for Postgres's `42702`. That check is
    /// only ever needed in a closed scope, and for a reason worth stating: everywhere
    /// else pglet declines a two-claimant bare name and lets the *executor* raise the
    /// `42702` over its own wider namespace. A closed scope is the whole namespace, so
    /// declining here would not defer the verdict, it would drop it.
    ///
    /// # Errors
    ///
    /// [`CatalogError::MissingFromClauseEntry`] (`42P01`),
    /// [`crate::Error::AmbiguousColumn`] (`42702`), or
    /// [`CatalogError::ColumnNotFoundInExpr`] (`42703`).
    pub(super) fn check_binding(&self, qualifier: Option<&str>, name: &str) -> Result<()> {
        if !self.closed {
            return Ok(());
        }
        match self.binding(qualifier, name) {
            // `Uncertain` is the whole reason this is a three-way answer: a relation the
            // scope could not name or could not enumerate might carry the reference, so
            // declining leaves the verdict to the executor rather than inventing one.
            Binding::Bound | Binding::Uncertain => Ok(()),
            Binding::NoRelation => Err(CatalogError::MissingFromClauseEntry {
                table: qualifier.unwrap_or(name).to_string(),
            }
            .into()),
            Binding::NoColumn => Err(CatalogError::ColumnNotFoundInExpr {
                column: qualifier.map_or_else(|| name.to_string(), |q| format!("{q}.{name}")),
            }
            .into()),
            Binding::Ambiguous => Err(crate::Error::AmbiguousColumn {
                column: name.to_string(),
            }),
        }
    }

    /// This scope's verdict on one column reference — the one [`Self::check_binding`]
    /// raises from, and the one a literal compared with an aggregate asks of the
    /// aggregate's argument ([`certain_aggregate_comparison_type`]). A bare `*` binds.
    fn binding(&self, qualifier: Option<&str>, name: &str) -> Binding {
        // A `*` is not a column reference — only its qualifier is checked.
        match (qualifier, name) {
            (Some(q), "*") => self.binding_of_relation(q),
            (Some(q), _) => self.binding_qualified(q, name),
            (None, "*") => Binding::Bound,
            // Postgres tries the target list **first** for a bare single-field key
            // (`findTargetlistEntrySQL92` scans it whole), so an output column binds
            // outright and the range table is never consulted — but two entries carrying
            // the name are `42702` there *unless they are the same expression*, and
            // taking the first is how this arm resolved a join's duplicated column and
            // deleted every row.
            // `findTargetlistEntrySQL92` falls through to the range table when no
            // target-list entry carries the name — it does not decline to judge.
            (None, n) => self
                .output_name_binding(n)
                .unwrap_or_else(|| self.binding_bare(name)),
        }
    }

    /// `findTargetlistEntrySQL92`'s verdict on a bare `name` against this level's target
    /// list: `None` when no entry carries the name (the caller falls through to the range
    /// table), else [`Binding::Bound`] or [`Binding::Ambiguous`].
    ///
    /// This is a **distinct count** over [`structural_identity`], not the C loop's shape.
    /// The loop keeps `target_result` = the *first* match and `equal()`s every later match
    /// against that one only, never updating it; a distinct count agrees with that exactly
    /// while the equality is **transitive**, and byte equality over a deterministic
    /// serialization is an equivalence relation, so it is. An entry with no identity (see
    /// [`OutputEntry::expr`]) is always its own claimant, and so is one whose identity
    /// cannot be derived.
    fn output_name_binding(&self, name: &str) -> Option<Binding> {
        let mut distinct: Vec<Vec<u8>> = Vec::new();
        let mut anonymous = 0_usize;
        let mut matched = false;
        for entry in self.output_entries.iter().filter(|e| e.name == name) {
            matched = true;
            match entry.expr.as_ref().and_then(structural_identity) {
                None => anonymous += 1,
                Some(key) => {
                    if !distinct.contains(&key) {
                        distinct.push(key);
                    }
                }
            }
        }
        if !matched {
            return None;
        }
        Some(if distinct.len() + anonymous >= 2 {
            Binding::Ambiguous
        } else {
            Binding::Bound
        })
    }

    /// Whether some level of the chain names the relation `qualifier` — the `q.*` test,
    /// which asks nothing about columns.
    fn binding_of_relation(&self, qualifier: &str) -> Binding {
        for level in self.levels() {
            if self.relation_at(level, qualifier).is_some() {
                return Binding::Bound;
            }
            if !self.level_names_known(level) {
                return Binding::Uncertain;
            }
        }
        Binding::NoRelation
    }

    /// The chain's verdict on `qualifier.name`, mirroring `refnameRangeTblEntry` then
    /// `scanRTEForColumn`: find the **first** level naming the relation and look the
    /// column up in that relation alone.
    ///
    /// The two declines are asymmetric and load-bearing. Reporting *no such relation*
    /// needs every level walked to be **name-known** — one anonymous relation anywhere
    /// inside could be the one the qualifier means. Reporting *no such column* needs
    /// only the relation that answered to be **columns-exhaustive**, since the walk
    /// stops there either way.
    fn binding_qualified(&self, qualifier: &str, name: &str) -> Binding {
        for level in self.levels() {
            if let Some(relation) = self.relation_at(level, qualifier) {
                if relation.columns.contains_key(name) {
                    return Binding::Bound;
                }
                return if relation.exhaustive {
                    Binding::NoColumn
                } else {
                    Binding::Uncertain
                };
            }
            if !self.level_names_known(level) {
                return Binding::Uncertain;
            }
        }
        Binding::NoRelation
    }

    /// The chain's verdict on a **bare** `name`, mirroring `colNameToVar`: the innermost
    /// level with any claimant decides, and two claimants *there* are `42702`.
    ///
    /// A level with a non-exhaustive relation can neither be counted nor walked past:
    /// its unlisted columns could add a claimant here, or hide one from an enclosing
    /// level. One claimant beside such a relation still binds — the reference resolves,
    /// only its uniqueness is unproven — and since the sole verdict that would follow is
    /// `42702`, declining and binding are the same outcome.
    fn binding_bare(&self, name: &str) -> Binding {
        for level in self.levels() {
            if self.merged_at(level, name) {
                return Binding::Bound;
            }
            let claimants = self
                .relations
                .iter()
                .filter(|r| r.level == level && !r.pseudo && r.columns.contains_key(name))
                .count();
            if claimants >= 2 {
                return Binding::Ambiguous;
            }
            if claimants == 1 {
                return Binding::Bound;
            }
            if self.level_columns_uncertain(level) {
                return Binding::Uncertain;
            }
        }
        Binding::NoColumn
    }

    /// Whether every relation at `level` has a name — the test a `42P01` verdict needs,
    /// since an anonymous relation could be the one a qualifier names.
    fn level_names_known(&self, level: u16) -> bool {
        !self
            .relations
            .iter()
            .any(|r| r.level == level && r.name.is_none())
    }

    /// Whether this scope names no relation at all — the "nothing can be resolved
    /// through me" test the correlated-descent uses to skip re-resolving a level that
    /// has no enclosing relations to contribute.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.relations.is_empty()
    }

    /// This scope, re-stamped as the **innermost** level of a chain whose enclosing
    /// levels come from `outer`.
    ///
    /// `self`'s relations keep level `0`; every relation of `outer` shifts one level
    /// further out. `outer` is itself already a chain (the enclosing query's own
    /// level plus *its* enclosing levels), so nesting composes: a subquery two levels
    /// down sees `0 = its own FROM`, `1 = the query containing it`, `2 = the
    /// outermost`. See [`Self`] § Level chaining.
    #[must_use]
    pub fn nested_in(mut self, outer: &Self) -> Self {
        self.nested = true;
        self.relations.extend(outer.relations.iter().map(|r| {
            let mut r = r.clone();
            r.level = r.level.saturating_add(1);
            r
        }));
        // A `USING` merge is a fact about a level's FROM, so it shifts outward with that
        // level's relations. Dropping it turned a merged `id` into two claimants the
        // moment a correlated subquery looked at the enclosing level.
        self.merged_names.extend(
            outer
                .merged_names
                .iter()
                .map(|(l, n)| (l.saturating_add(1), n.clone())),
        );
        self
    }

    /// Whether this scope chains an enclosing level ([`Self::nested_in`]) — the scope of
    /// the correlated re-run alone, which resolves a level's expressions a second time.
    fn chains_enclosing_level(&self) -> bool {
        self.relations.iter().any(|r| r.level > 0)
    }

    /// Whether resolution must stop at `level` after finding no claimant there — the
    /// per-relation fact the per-level `sealed` flag used to stand in for.
    ///
    /// A level is uncertain when any relation on it is non-exhaustive: the name might
    /// belong to that relation, so walking outward could bind it to an enclosing
    /// relation that merely shares it.
    fn level_columns_uncertain(&self, level: u16) -> bool {
        self.relations
            .iter()
            .any(|r| r.level == level && !r.exhaustive)
    }

    /// The distinct levels present, innermost first — `0..=max_level`, which is all a
    /// resolver needs to walk since [`Self::nested_in`] assigns levels contiguously.
    fn levels(&self) -> std::ops::RangeInclusive<u16> {
        // An opaque relation is a relation, so a level that holds only one still
        // appears here — which is what the old `sealed` vector had to be consulted for.
        let max = self.relations.iter().map(|r| r.level).max().unwrap_or(0);
        0..=max
    }

    /// Add the `EXCLUDED` pseudo-relation — qualified-only.
    /// See [`ScopeRelation::pseudo`].
    // internal planner helper; generalizing over the hasher is needless API churn
    #[allow(clippy::implicit_hasher)]
    pub fn push_excluded(&mut self, columns: HashMap<String, ResolvedType>) {
        self.push_scope_relation(Some("excluded".to_string()), columns, true, true);
        if let Some(relation) = self.relations.last_mut() {
            relation.base = true;
        }
    }

    /// The type of `[qualifier.]name`, or `None` when this scope cannot be certain.
    ///
    /// The public face of [`Self::resolve`] — the **gate** resolver, so a bare name
    /// claimed by two relations declines rather than guessing. Callers must treat
    /// `None` as "unknown", never as "no such column".
    #[must_use]
    pub fn certain_column_type(&self, qualifier: Option<&str>, name: &str) -> Option<TypeOid> {
        self.resolve(qualifier, name).map(|t| t.oid)
    }

    /// The **enum identity** of `[qualifier.]name` — the same derivation
    /// [`Self::certain_column_type`] projects the OID out of, read at its other field.
    ///
    /// This is the `Column` leaf of the plan-time `EnumLeaves`, and the reason a
    /// relation's columns are `ResolvedType`s here rather than `TypeOid`s: no OID names
    /// an enum, so a gate reading this namespace could not tell `mood` from `text`
    /// (`EDGES.md` row 114).
    #[must_use]
    pub fn column_enum(&self, qualifier: Option<&str>, name: &str) -> Option<String> {
        Some(self.resolve(qualifier, name)?.enum_type.name()?.to_string())
    }

    /// Whether any relation in scope carries a column named `name` — the
    /// "is this a base column of the query" test, independent of its type.
    ///
    /// **Level `0` only.** This and its siblings ([`Self::contains_column_ref`],
    /// [`Self::sole_relation_with`]) answer structural questions about *this* query's
    /// own relations — which output column an `ORDER BY` key names, which relation an
    /// unqualified reference belongs to — and an enclosing query's relations are not
    /// candidates for any of them. Only the coercion/gate resolvers walk the chain.
    #[must_use]
    pub fn contains_column(&self, name: &str) -> bool {
        self.relations
            .iter()
            .any(|r| r.level == 0 && r.columns.contains_key(name))
    }

    /// Whether `[qualifier.]name` names a base column of a relation in scope —
    /// [`Self::contains_column`]'s **qualifier-aware** form.
    ///
    /// A bare name keeps the flat any-relation test. A qualified `q.name` asks only
    /// relation `q`, so `pb.x` is not answered by a sibling `pa.x`: Postgres resolves a
    /// qualified reference against its named relation alone
    /// (`findTargetlistEntrySQL92` takes the output-name branch only for a single-field
    /// name), and a bare-name test conflates the two sides of a join.
    #[must_use]
    pub fn contains_column_ref(&self, qualifier: Option<&str>, name: &str) -> bool {
        qualifier.map_or_else(
            || self.contains_column(name),
            |q| {
                self.relation_at(0, q)
                    .is_some_and(|r| r.columns.contains_key(name))
            },
        )
    }

    /// The name of the **one** relation in scope carrying a column `name`, or `None`
    /// when none or several do.
    ///
    /// Used to decide which relation an *unqualified* column reference must bind to:
    /// with a sole claimant the binding is unambiguous, and with several it is
    /// Postgres's `42702` (which the executor raises), so declining is right either way.
    /// Pseudo-relations are skipped — they are reachable only qualified.
    #[must_use]
    pub fn sole_relation_with(&self, name: &str) -> Option<&str> {
        let mut claimants = self
            .relations
            .iter()
            .filter(|r| r.level == 0 && !r.pseudo)
            .filter(|r| r.columns.contains_key(name));
        let first = claimants.next()?;
        claimants
            .next()
            .is_none()
            .then_some(first.name.as_deref())?
    }

    /// Every type a **bare** `name` could bind to at the **innermost level that claims
    /// it**, one per claiming relation there.
    ///
    /// The walk is Postgres's `colNameToVar`: search a level's namespace in full, and
    /// move outward only when it yields nothing. So an inner relation's `x` wins
    /// outright over an enclosing one's, and two claimants can only ever be at the
    /// *same* level (where they are that level's own ambiguity to judge). An uncertain
    /// level that yields nothing stops the walk — see [`Self`] § Level chaining.
    fn bare_candidates<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a ResolvedType> + 'a {
        let mut found = Vec::new();
        for level in self.levels() {
            found.extend(
                self.relations
                    .iter()
                    .filter(|r| r.level == level && !r.pseudo)
                    .filter_map(|r| r.columns.get(name)),
            );
            if !found.is_empty() || self.level_columns_uncertain(level) {
                break;
            }
        }
        found.into_iter()
    }

    /// The relation named `qualifier` at `level`, if any.
    ///
    /// An exact comparison, pseudo-relations included: the qualifier reached the
    /// planner through the parser's name fold, so an unquoted `EXCLUDED` is already
    /// `excluded` and a quoted `"EXCLUDED"` names a relation Postgres does not have
    /// either (`42P01` there).
    fn relation_at(&self, level: u16, qualifier: &str) -> Option<&ScopeRelation> {
        self.relations
            .iter()
            .find(|r| r.level == level && r.name.as_deref() == Some(qualifier))
    }

    /// The type of a **qualified** `qualifier.name`, searching innermost level
    /// outward, or `None` when no level in the chain names `qualifier` (or the one
    /// that does has no such column).
    ///
    /// Once a level names `qualifier` the search **stops there**, resolved or not:
    /// Postgres's `refnameRangeTblEntry` finds the range-table entry level by level
    /// and then looks the column up in *that* entry alone, so a missing column is
    /// `42703` on that relation, never a silent rebinding to a same-named relation
    /// further out. An uncertain level that names nothing also stops the walk.
    fn qualified<'a>(&'a self, qualifier: &str, name: &str) -> Option<&'a ResolvedType> {
        for level in self.levels() {
            if let Some(relation) = self.relation_at(level, qualifier) {
                return relation.columns.get(name);
            }
            if self.level_columns_uncertain(level) {
                return None;
            }
        }
        None
    }

    /// The type of `[qualifier.]name` for the **gate**, or `None` when this scope
    /// can't be certain.
    ///
    /// A bare name resolves only when exactly one relation claims it. Two claimants
    /// is Postgres's own `42702`, which the executor raises — declining here keeps
    /// that verdict rather than pre-empting it with a `42883` from whichever type
    /// the gate guessed.
    #[must_use]
    pub(super) fn column_type(&self, qualifier: Option<&str>, name: &str) -> Option<TypeOid> {
        self.resolve(qualifier, name).map(|t| t.oid)
    }

    /// [`Self::column_type`]'s body, and the one derivation both its projections read:
    /// the OID for every rule that needs no identity, the whole [`ResolvedType`] for
    /// the gates that do.
    #[must_use]
    fn resolve<'a>(&'a self, qualifier: Option<&str>, name: &'a str) -> Option<&'a ResolvedType> {
        if let Some(q) = qualifier {
            return self.qualified(q, name);
        }
        let mut candidates = self.bare_candidates(name);
        let first = candidates.next()?;
        candidates.next().is_none().then_some(first)
    }

    /// The type `[qualifier.]name` binds to where an **enclosing** level binds it —
    /// `None` where this level (`0`) does, or no level can be certain.
    ///
    /// 18.6's level walk: a qualifier binds at the innermost level naming it, a bare
    /// name at the innermost level carrying it. [`Self::resolve`] walks the same way, so
    /// this is that walk with the level-`0` binding answered as *not enclosing*.
    #[must_use]
    pub(super) fn enclosing_binding<'a>(
        &'a self,
        qualifier: Option<&str>,
        name: &'a str,
    ) -> Option<&'a ResolvedType> {
        let binds_here = qualifier.map_or_else(
            || self.contains_column(name),
            |q| self.relation_at(0, q).is_some(),
        );
        if binds_here {
            return None;
        }
        self.resolve(qualifier, name)
    }

    /// The type of `[qualifier.]name` for the **coercion pass**, or `None` when this
    /// scope can't be certain.
    ///
    /// Identical to [`Self::resolve`] on a qualified reference. It differs on a bare
    /// name with several claimants: the coercion target is the claimants' type when
    /// they **all agree**, where the gate declines. The two rules answer different
    /// questions and the split is load-bearing in both directions:
    ///
    /// - A `JOIN … USING (id)` merges the two `id` columns into one, so a bare `id`
    ///   is not ambiguous in Postgres and `WHERE id = '1'` must still coerce. The
    ///   claimants necessarily agree (Postgres requires a common type to merge on),
    ///   and the cast is right whichever one binds.
    /// - When the claimants disagree, no single cast can be right, so both rules
    ///   decline and the comparison falls through to the eval-time gate.
    /// - Where several claimants agree but Postgres would call the reference
    ///   ambiguous anyway, the coercion is harmless — the query errors `42702`
    ///   before the cast can change an answer.
    #[must_use]
    ///
    /// The **OID only**: the coercion pass writes `Expr::Cast` nodes, which name a
    /// `TypeOid` and nothing else. Where an enum identity is what a caller needs, it
    /// reads [`expr_enum_in`], whose column leaf is the *gate* resolver — the split
    /// stated there. The claimant agreement test below is over the whole
    /// [`ResolvedType`], so two claimants that share an OID and disagree about their
    /// enum decline rather than coercing to one of them.
    fn coercion_type<'a>(&'a self, qualifier: Option<&str>, name: &'a str) -> Option<TypeOid> {
        if let Some(q) = qualifier {
            return self.qualified(q, name).map(|t| t.oid);
        }
        let mut candidates = self.bare_candidates(name);
        let first = candidates.next()?;
        candidates.all(|t| t == first).then_some(first.oid)
    }

    /// The declared type of `[qualifier.]name` where the reference can only be a column
    /// of a **base table at this query level**: every relation of the level that carries
    /// the name (and answers the qualifier) is one, they agree on the type, and no
    /// relation of the level whose columns are not all known could carry it instead.
    /// `excluded` is one too — it carries the target's declared columns — and, like
    /// every reader here, answers only its qualified spelling.
    fn base_column_type(&self, qualifier: Option<&str>, name: &str) -> Option<TypeOid> {
        let mut found = None;
        for relation in &self.relations {
            if relation.level != 0
                || (relation.pseudo && qualifier.is_none())
                || qualifier.is_some_and(|q| relation.name.as_deref() != Some(q))
            {
                continue;
            }
            match relation.columns.get(name) {
                Some(column) if relation.base && found.is_none_or(|t| t == column.oid) => {
                    found = Some(column.oid);
                }
                Some(_) => return None,
                None if !relation.exhaustive => return None,
                None => {}
            }
        }
        found
    }
}

/// A [`ColumnScope`] read as a [`LeafScope`], so the complete type walk
/// (`Expr::infer_type`) can run over a **clause's** namespace.
///
/// The boolean-context gate needs a reader that types `k + 1`, `length(t)` and a
/// scalar subquery — 18.6 refuses all three at a boolean position — where the
/// gate readers beside it ([`certain_operand_type`], [`arg_known_type`]) answer
/// `None` for each, deliberately, because *their* rules must not reject on a guess.
/// The complete reader already exists and is generic in this trait; what was missing
/// was the impl, not a second walk.
///
/// `schema` is what a scalar subquery's type needs (`Plan::output_columns`), and it
/// is optional because the no-FROM entry point ([`resolve_expr_types_no_columns`]) is
/// reached from the parser, which holds no catalog. A `None` there costs one shape —
/// a scalar subquery in a boolean position, over zero rows — and is the eval-time
/// resolver's to answer.
#[derive(Clone, Copy)]
struct ClauseLeafScope<'a> {
    columns: &'a ColumnScope,
    schema: Option<&'a Schema>,
}

impl LeafScope for ClauseLeafScope<'_> {
    /// The **gate**'s column reader ([`ColumnScope::resolve`]), not the coercion
    /// pass's: a bare name several relations claim is Postgres's `42702`, and
    /// declining here keeps that verdict rather than pre-empting it with a `42804`
    /// read off whichever claimant this guessed.
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        self.columns.resolve(table, name).map(|t| t.oid)
    }

    fn subquery_type(&self, plan: &Plan) -> Option<TypeOid> {
        self.schema
            .and_then(|s| plan.output_columns(s).first().map(|c| c.type_oid))
    }
}

/// The **clause-root** boolean gate: `expr` is the whole of a `WHERE` / `HAVING` /
/// `JOIN … ON` / `CHECK` / partial-index predicate, and must be `boolean`.
///
/// The node positions (`AND`/`OR`/`NOT`/searched `CASE WHEN`) are gated inside
/// [`check_expr_semantics`]'s walk; a clause root hangs on no node at all, so each
/// lowering site calls this where 18.6's `transformWhereClause` sits in that
/// statement's clause order. Both halves read the same rulebook
/// ([`check_boolean_operand`]) through the same scope adapter, so there is one
/// verdict with two call shapes rather than two rules.
///
/// # Errors
///
/// [`crate::Error::Type`] (`42804`) for a non-boolean clause;
/// [`crate::Error::InvalidInputSyntax`] (`22P02`) for an unresolved literal that is not
/// a boolean spelling.
#[inline(never)]
pub fn check_boolean_clause(
    expr: &Expr,
    ctx: BoolContext,
    scope: &ColumnScope,
    schema: &Schema,
) -> Result<()> {
    check_boolean_operand(
        expr,
        ctx,
        &ClauseLeafScope {
            columns: scope,
            schema: Some(schema),
        },
    )
}

/// Every plan-time expression check, in **one bottom-up, left-to-right walk**.
///
/// # Why this runs at plan time at all
///
/// The eval-time gate in `eval_binary_op` derives its operand types from the runtime
/// `Value`s, so it is **data-dependent**: over an empty table — or a column whose
/// every row is NULL — it never fires, and `WHERE text_col = 5` silently returned
/// zero rows where Postgres raises `42883` at parse time. Worse, the accelerated
/// build lifts such a predicate into an index probe, whose gate compares the
/// *declared* types and *does* fire on an empty table — so the same query answered
/// differently on the two build profiles. Gating on declared types here removes both:
/// the verdict no longer depends on the rows, and it is reached identically whether
/// or not the predicate is later lifted into a probe.
///
/// It is deliberately **conservative** ([`certain_operand_type`]): an operand whose
/// type isn't certain answers `None` and the pair falls through to the eval-time gate,
/// so this can only ever reject what Postgres also rejects.
///
/// # Why it is one walk and not two
///
/// The walk order is the whole point. Postgres resolves an expression in source order
/// during parse analysis, so when a query contains two independently-bad subtrees the
/// **leftmost, innermost** one is the error it reports. Two separate whole-tree passes
/// cannot reproduce that: whichever pass runs first wins wherever its error sits, so
/// `SELECT d LIKE 'x%' AND COALESCE(k, x) IS NULL` reported the `COALESCE` (a
/// later-in-source `42804`) instead of the `LIKE` (`42883`, and what Postgres says).
/// No *reordering* of two passes fixes it — one of the two orders is wrong for one of
/// the two queries. Only deciding both per node, in one walk, is right for both. This
/// is the same correction `executor/ddl.rs` made for stored expressions.
///
/// Descending **before** checking is what makes it bottom-up: an inner error outranks
/// an outer one, matching Postgres, which has already resolved the arguments by the
/// time it resolves the construct built from them.
///
/// Two families run here, in Postgres's own per-node order:
///
/// 1. **Operand legality** — the shared rulebook (`legality.rs`), the same one the
///    DDL-time stored-expression checker drives. It covers `LIKE`/`ILIKE`,
///    comparisons, containment, `||`, `->`/`->>`, and `starts_with`. Only the
///    comparison arm used to run at plan time; the rest fired at *eval*, so on an
///    empty table they never fired at all, where Postgres rejects the query outright.
/// 2. **Signature resolution** — a call whose argument list matches no `pg_proc`
///    signature. `unnest(<a non-array>)` ([`unnest_legality`]) is the member, and it is
///    listed apart from operand legality because the family is not the operator table:
///    an aggregate's operand gate is the same shape and still runs at eval, where its
///    own `EDGES.md` rows record the cost.
/// 3. **Branch unification** — `COALESCE`/`CASE`/`ARRAY` common types
///    ([`check_branch_unification_node`]).
///
/// Operand legality precedes unification within a node because Postgres builds a
/// construct's operator comparisons before folding its branch types
/// (`transformCaseExpr` makes each `WHEN`'s `=` before calling `select_common_type`).
/// The families are per-node and mutually exclusive by `Expr` variant today, so their
/// order is a contract with no live pair to test — except against the **recursion**,
/// which outranks all three: an argument's own binding failure beats the signature
/// refusal built on it (`unnest(nosuchcol)` is `42703`, `unnest(<ambiguous bare name>)`
/// `42702`, on both engines).
///
/// # Errors
///
/// The first failure in that order — `42883` for an absent operator, `0A000` for a
/// Postgres-legal form pglet doesn't evaluate, `42804`/`42846` for a branch set with no
/// common type.
fn check_expr_semantics(expr: &Expr, scope: &ColumnScope, schema: &Schema) -> Result<()> {
    crate::depth::check_stack()?;
    // Children first, in source order: bottom-up, left-to-right — and a child that
    // sits in a **boolean context** is gated the moment its own subtree is done,
    // before the next child is walked at all. That interleave is 18.6's
    // `transformBoolExpr` / `transformCaseExpr` loop, which transforms one argument
    // and coerces it before touching the next, and no pair of whole-subtree passes
    // reproduces it: `CHECK (a AND b LIKE 'x')` is `42804 argument of AND` while
    // `CHECK (b LIKE 'x' AND a)` is `42883`, and `CASE WHEN 1 THEN nosuchcol END` is
    // `42804` rather than the `THEN` arm's `42703`. Measured on 18.6, all three.
    //
    // The chain-forming variants are walked in this frame rather than through the
    // closure `try_for_each_child_ctx` takes, so a chain costs one frame a level
    // (`crate::depth`); their children and contexts are `for_each_child_ctx`'s.
    //
    // A sub-plan is a nested level, and its references are judged where this walk
    // reaches it — ahead of an `IN`'s left-hand operand, the order `transformSubLink`
    // analyzes the two in. Only a scope that is the whole namespace can judge it, and it
    // is judged once: a closed scope at a level's own lowering, never one the chained
    // re-run builds (`nested`), which judges the level below in its own descent.
    if let Expr::Subquery { plan } | Expr::Exists { plan, .. } | Expr::InSubquery { plan, .. } =
        expr
        && scope.closed
        && !scope.nested
    {
        super::correlation::judge_nested_level(plan, scope, schema)?;
    }
    match expr {
        Expr::Negate(child)
        | Expr::UnaryPlus(child)
        | Expr::IsNull(child)
        | Expr::IsNotNull(child)
        | Expr::Cast { expr: child, .. }
        | Expr::CastTypmod { expr: child, .. }
        | Expr::Alias { expr: child, .. }
        | Expr::InSubquery { expr: child, .. }
        | Expr::BooleanTest { expr: child, .. } => check_expr_semantics(child, scope, schema)?,
        Expr::BinaryOp { left, op, right } => {
            let ctx = op.operand_context();
            check_child_semantics(left, ctx, scope, schema)?;
            check_child_semantics(right, ctx, scope, schema)?;
        }
        Expr::AnyAll { left, right, .. } => {
            check_expr_semantics(left, scope, schema)?;
            check_expr_semantics(right, scope, schema)?;
        }
        Expr::InList {
            expr: child, list, ..
        } => {
            check_expr_semantics(child, scope, schema)?;
            check_list_semantics(list, scope, schema)?;
        }
        _ => expr.try_for_each_child_ctx(|child, ctx| {
            check_child_semantics(child, ctx, scope, schema)
        })?,
    }
    check_expr_node_semantics(expr, scope, schema)
}

/// [`check_expr_semantics`] over an `IN` list's members, in order — in no boolean
/// context (`Expr::chain_operands`).
#[inline(never)]
fn check_list_semantics(list: &[Expr], scope: &ColumnScope, schema: &Schema) -> Result<()> {
    list.iter()
        .try_for_each(|item| check_expr_semantics(item, scope, schema))
}

/// One child of [`check_expr_semantics`]: its subtree, then the boolean-context gate
/// its parent puts it in, if any.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn check_child_semantics(
    child: &Expr,
    ctx: Option<BoolContext>,
    scope: &ColumnScope,
    schema: &Schema,
) -> Result<()> {
    check_expr_semantics(child, scope, schema)?;
    ctx.map_or(Ok(()), |ctx| {
        check_boolean_clause(child, ctx, scope, schema)
    })
}

/// Every plan-time check for **one** node, no recursion — see [`check_expr_semantics`].
///
/// Column **binding** first (in a closed scope only), then operand legality, then
/// branch unification: the three are decided **per node**, which is the distinction the
/// single walk preserves. Postgres builds a construct's operator comparisons before
/// folding its branch types, so within a node the operand verdict outranks the
/// unification one — and it resolves the leaves before either, which is why a binding
/// failure cannot be a pass of its own. Over a table whose `b` is **`BIGINT`**,
/// `RETURNING bogus > 0 AND b LIKE 'x'` is `42703` while the reverse spelling
/// (`RETURNING b LIKE 'x' AND bogus > 0`) is `42883` — measured on 18.6, both engines
/// agreeing on both spellings — and no ordering of one whole-tree binding pass against
/// one whole-tree gate pass yields both: a leaf is checked when the walk reaches it, so
/// source order decides.
///
/// The example's schema is load-bearing and easy to lose: the `LIKE` operand has to be
/// something `~~` does not take. With `b TEXT` the operand is legal, so both spellings
/// are `42703` on both engines and the pair stops discriminating anything — which is a
/// statement about the example, not about the rule.
///
/// Three shapes reach the comparison rule, all of which Postgres resolves through the
/// `=` / `<` / … operators and rejects at parse time regardless of which row (or which
/// `WHEN` arm) would have been reached first:
///
/// - a comparison `left <op> right`, reported in the **written** operand order;
/// - a simple `CASE x WHEN v` label, which is Postgres's `x = v` — so
///   `CASE 1 WHEN 1 THEN … WHEN true THEN … END` fails even though the first arm
///   matches;
/// - an `IN (list)` element, which is Postgres's `x = elem` — the same
///   match-before-bad rule `types/compare_in_list_match_before_bad_loud.sql` locks
///   for the unparseable-literal case.
#[inline(never)]
fn check_expr_node_semantics(expr: &Expr, scope: &ColumnScope, schema: &Schema) -> Result<()> {
    let ty = |e: &Expr| certain_operand_type(e, scope);
    // The **comparison** rulebook's currency is a [`ResolvedType`] — the OID plus the
    // enum identity, which no OID can carry. Every other rule on this node takes the
    // OID alone, so `ty` stays beside it and the two are one derivation projected twice
    // ([`certain_operand_resolved`]).
    //
    // The pair is read through the shallow reader and, where that leaves a side
    // untyped, through the complete one — but the widened pair is *used* only when one
    // of its sides names an enum ([`complete_resolved`]). Then the operator's verdict,
    // and only then the `unknown` side's conversion to what the operator resolved: the
    // two in one helper, at every position 18.6 makes an `=`/`<`/… at whose right-hand
    // side is a **sibling `Expr`**, so no arm can come to hold half of it. The two
    // positions whose right-hand side is not take `compare_one_sided` below, for the
    // same reason spelled one way out.
    let compare = |l: &Expr, r: &Expr, symbol: &str| -> Result<()> {
        let (lt, rt) = operand_pair(l, r, scope, schema);
        comparison_legality(lt.clone(), rt.clone(), symbol)?;
        crate::planner::comparison_enum_literal_legality(lt.as_ref(), rt.as_ref(), l, r, schema)
    };
    // The two positions whose right-hand side is **not** a sibling `Expr`, so neither
    // can take `compare`: the operand is read from a sub-plan's output column or from
    // an array's element, and the conversion runs one way only (see
    // [`compare_against_one_sided`]).
    let compare_one_sided = |l: &Expr, rt: Option<ResolvedType>, symbol: &str| -> Result<()> {
        compare_against_one_sided(l, rt, symbol, scope, schema)
    };
    match expr {
        // A closed scope binds its leaves; an open one waves them through (the
        // engine-wide posture that column existence is decided at eval time).
        Expr::Column { table, name } => scope.check_binding(table.as_deref(), name)?,
        // An aggregate whose argument holds another is 18.6's `42803`, raised as it
        // transforms the outer call, after the call's arguments. In a top-level
        // `SELECT`, where every name binds at plan time, a column the argument cannot bind
        // is the `42703` that wins here too; inside a sub-plan or a DML statement this
        // refusal outranks errors 18.6 raises first (`EDGES.md` row 302). pglet reads
        // every aggregate inside a sub-plan as that sub-plan's — it has no outer-level
        // aggregates, where 18.6 assigns one to the lowest level its arguments name — so
        // a pair 18.6 places at two levels is refused here (`EDGES.md` row 303).
        Expr::Aggregate { arg, .. } if contains_aggregate(arg) => {
            return Err(crate::Error::NestedAggregate);
        }
        Expr::BinaryOp { left, op, right } => {
            // The same dispatch order as the DDL-time checker, over the same rulebook,
            // so the two cannot disagree about which operand pairs are legal.
            if let Some(symbol) = like_symbol(*op) {
                like_legality(ty(left), ty(right), symbol)?;
            } else if let Some(symbol) = regex_symbol(*op) {
                let (lt, rt) = operand_pair(left, right, scope, schema);
                regex_legality(lt.as_ref(), rt.as_ref(), symbol)?;
            } else if let Some(symbol) = comparison_symbol(*op) {
                compare(left, right, symbol)?;
            } else if let Some(symbol) = containment_symbol(*op) {
                containment_legality(ty(left), ty(right), symbol)?;
            } else {
                match op {
                    BinaryOp::Concat => concat_legality(ty(left), ty(right))?,
                    BinaryOp::JsonGet => json_get_legality(ty(left), ty(right), false)?,
                    BinaryOp::JsonGetText => json_get_legality(ty(left), ty(right), true)?,
                    _ if json_operator_symbol(*op).is_some() => {
                        let (symbol, array) = json_operator_symbol(*op).unwrap_or(("?", false));
                        json_operator_legality(ty(left), ty(right), symbol, array)?;
                    }
                    BinaryOp::StartsWith => starts_with_legality(ty(left), ty(right))?,
                    _ => {}
                }
            }
        }
        Expr::Case {
            operand: Some(operand),
            when_clauses,
            ..
        } => {
            for (condition, _) in when_clauses {
                compare(operand, condition, "=")?;
            }
        }
        Expr::InList {
            expr: subject,
            list,
            ..
        } => {
            for item in list {
                compare(subject, item, "=")?;
            }
        }
        // `x [NOT] IN (SELECT …)` **is** an `=`: `transformSubLink` rewrites a bare
        // `IN` into `= ANY (subquery)`, builds one `PARAM_SUBLINK` per non-junk
        // target-list entry carrying that entry's own `exprType`, and hands the pair
        // to `make_row_comparison_op` — so the operator is resolved in parse analysis,
        // over an empty subquery and inside a `WHERE false` alike. Without this arm the
        // pair was decided at eval time on `Value` kinds, where an enum is a
        // `Value::Text`: `DELETE FROM w WHERE m IN (SELECT s FROM w)` removed the rows
        // 18.6 keeps by refusing, and the same statement over a **second enum**
        // (`SELECT y`) did too.
        //
        // The sub-plan's column is read by [`branch_subquery_type`], the same reader a
        // bare scalar-subquery branch takes, and there is no second reader beside it —
        // see [`one_sided_operand_pair`] for why that makes the widening rule read
        // slightly differently here.
        Expr::InSubquery {
            expr: subject,
            plan,
            ..
        } => compare_one_sided(subject, branch_subquery_type(plan, scope, schema), "=")?,
        // `x op ANY/ALL (array)` is `make_scalar_array_op`, which resolves `op` between
        // the left-hand type and the **element** type of the right-hand array. The
        // element type is the array expression's own, settled before the operator:
        // `transformArrayExpr` with no target type folds its elements through
        // `select_common_type`, so an all-`unknown` element set is **`text`** and
        // `m = ANY(ARRAY['ok'])` is `42883` on 18.6 rather than a membership test.
        // That is why this position converts one way only.
        Expr::AnyAll {
            left, op, right, ..
        } => {
            if let Some(symbol) = comparison_symbol(*op) {
                compare_one_sided(left, array_element_resolved(right, scope, schema), symbol)?;
            }
        }
        // `NULLIF(a, b)` **is** an `=` comparison — Postgres builds the operator and
        // only then overwrites the result type. So a pair with no `=` is `42883`
        // (`nullif(time_col, timestamp_col)`), not the `42846` the common-type
        // constructs report for the same pair. Without this gate the result type fell
        // through to a fallback and the projection declared one type over another's
        // bytes.
        Expr::VariadicFn {
            func: VariadicFunc::NullIf,
            args,
        } => {
            if let [a, b] = args.as_slice() {
                compare(a, b, "=")?;
            }
        }
        // `unnest(<a non-array>)` — a **signature** refusal, so it sits where signature
        // resolution sits: on this node, after the recursion has already judged the
        // argument. That order is the contract at two scales and both were measured on
        // 18.6. Within the expression, the argument's own binding verdict outranks this,
        // in **both** its failing directions, because [`Self::check_binding`] runs at the
        // leaf that the recursion above reaches first: `unnest(nosuchcol)` is `42703` and
        // `SELECT unnest(id) FROM a, b` over two relations that both carry `id` is
        // `42702`, never `42883` — measured on both engines, and the second matters
        // because `arg_known_type` reads `coercion_type`, which *does* answer for a
        // multi-claimant name whose claimants agree. Between clauses, the whole target
        // list is analysed before the `WHERE`, so `SELECT unnest(i) FROM t WHERE
        // nosuchcol = 1` is `42883` — which pglet reproduces because `lower_select_level`
        // lowers the projection ahead of the predicate, item by item in source order.
        //
        // The argument is read through `arg_known_type` rather than `ty`
        // ([`certain_operand_type`]) for one reason: that reader answers `None` for a
        // bare string or `NULL` literal, which is Postgres's `unknown` at this position
        // and carries 18.6's *other* code (`42725`) — see [`unnest_legality`]. The rest
        // of the gap between the two readers costs a residual rather than a verdict: an
        // `Expr::Bound` constant and an `Alias` wrapper are typed by `certain_operand_type`
        // and not by this one, which is `EDGES.md` row 65's boundary. It is read at the
        // **written** reach: the computed shapes the common-type folds type are not this
        // signature-resolution gate's, whose residual (`EDGES.md` row 55) is recorded
        // over that narrower reader.
        Expr::Unnest(inner) => unnest_legality(arg_known_type_with_reach(
            inner,
            scope,
            BranchReach::Written,
        ))?,
        // `x IS [NOT] TRUE|FALSE|UNKNOWN` — boolean operand only, through the same
        // rulebook every other boolean context takes. Not interleaved, and that is
        // 18.6's shape rather than an omission: `transformBooleanTest` transforms the
        // whole argument and *then* coerces it, so this is an ordinary post-order node
        // check where `AND`/`OR`/`NOT`/`CASE WHEN` are per-argument.
        //
        // A bare string or `NULL` is Postgres's `unknown` here and is *resolved* to
        // boolean rather than refused, so `'t' IS TRUE` is `t` and `'x' IS TRUE` is the
        // conversion's `22P02`.
        Expr::BooleanTest { expr: inner, test } => {
            check_boolean_clause(inner, BoolContext::Test(*test), scope, schema)?;
        }
        _ => {}
    }
    // Then the construct-level common-type diagnosis for this node.
    check_branch_unification_node(expr, scope, schema)
}

/// An operand's type when it is **certainly** knowable from `scope`, `None` when it
/// is a guess — the planner-side twin of `certain_expr_type_oid` (which resolves
/// against the executor's materialized `ColumnDesc`s instead).
///
/// Shallow on purpose: a column, a literal, a cast, an already-bound value, and
/// `Alias` looking through. Anything else — an unbound `$N`, an operator result, a
/// subquery — is `None`, so the gate defers to eval time rather than rejecting on a
/// guess. [`operand_pair`] is where that deferral is *spent* rather than kept: an enum
/// pair is read through the complete walk instead, because the identity is derived by a
/// walk that answers for exactly the operands this one declines. A column reference keeps its qualifier: `o.x` is *not* the scope's bare
/// `x`, and answering `None` for an out-of-scope qualifier is what keeps a
/// correlated subquery (or an `UPDATE … FROM` source) from being typed against the
/// wrong relation.
fn certain_operand_type(expr: &Expr, scope: &ColumnScope) -> Option<TypeOid> {
    match expr {
        Expr::Column { table, name } => scope.resolve(table.as_deref(), name).map(|t| t.oid),
        Expr::Literal(lit) => lit.infer_type(),
        Expr::Cast { target_type, .. } => Some(*target_type),
        Expr::CastTypmod { type_oid, .. } => Some(*type_oid),
        Expr::Bound(v) => v.type_oid(),
        Expr::Alias { expr, .. } => certain_operand_type(expr, scope),
        _ => None,
    }
}

/// The **enum identity** an expression carries in this clause's namespace — the one
/// plan-time instantiation of `executor::expr`'s exhaustive walk.
///
/// The identity is derived by that walk and nowhere else, so the ordering sites, a
/// projected column's `ColumnDesc::enum_type` and these gates read one arm list rather
/// than three that must agree. The two leaves the walk cannot answer from a node are
/// this scope's columns ([`ColumnScope::column_enum`]) and a scalar subquery's first
/// output column (`executor::subquery_enum_name`, which needs the catalog — this reader
/// keeps its `Option` for the two coercion-**pass** callers that hold none, where the
/// column leaf still answers and only the subquery one declines).
///
/// Both of those leaves reach a `ColumnDesc`, and a **projected expression**'s descriptor
/// carries what `Expr::to_column_desc` derives for it rather than only what a passthrough
/// propagated — which is what makes the column leaf answer for a derived table's `v.z`
/// (`EDGES.md` row 107). A **gate** reads [`enum_identity_in`] rather than this, and its
/// subquery leaf asks [`resolved_subquery_type_in`] instead: the descriptor is built under
/// the sub-plan's own scope, so a projection naming an enclosing query's column is one
/// answer no descriptor can carry.
///
/// The column leaf is the **gate** resolver ([`ColumnScope::resolve`]), not the coercion
/// pass's: a bare name several relations claim declines here, so a merged `USING` column
/// referenced bare loses its identity and the pair below it is judged as `text` — the
/// under-refusal this reader keeps rather than pre-empting a `42702` with a `42804`.
fn expr_enum_in(expr: &Expr, scope: &ColumnScope, schema: Option<&Schema>) -> Option<String> {
    crate::executor::expr_enum_with(
        expr,
        &crate::executor::EnumLeaves {
            column: &|table, name| scope.column_enum(table, name),
            subquery: &|plan| schema.and_then(|s| crate::executor::subquery_enum_name(plan, s)),
        },
    )
}

/// Whether `expr`'s enum identity is **undecidable** rather than absent — the one
/// predicate behind every [`EnumIdentity::Undecided`] this engine produces.
///
/// Three producers, each for its own reason:
///
/// - A **cast to `text`**, in either spelling. A written `x::mood` reaches the planner
///   as exactly this node — the parser drops the enum name where it reads the type
///   (`EDGES.md` row 104) — so `t::mood` and `t::text` are the same `Expr`. A cast to
///   any *other* type is decidable and is not here: a `$N` the client declared
///   `varchar` is a cast to `varchar`, and it refuses against an enum as 18.6 refuses it.
/// - A **bare string or `NULL` literal**, which is 18.6's `unknown`: it adopts the
///   position's type, enum included, so `m = 'ok'` and `COALESCE(m, 'sad')` resolve to
///   `mood` there and must not refuse here.
/// - A **spliced `Bound` text value**, which carries no identity in either direction and
///   is read the conservative way for the same reason the first producer is.
///
/// `Alias` looks through, and an `ARRAY[…]` is undecidable when a **typed** element is:
/// the array's identity is its elements', by the same fold `executor::expr`'s
/// `Expr::Array` arm runs over them, so `ARRAY[t::mood]` inherits the cast's.
///
/// An `unknown` element ([`unknown_array_element`]) is **not** one of those, and that
/// asymmetry is `transformArrayExpr`'s: it folds the elements through
/// `select_common_type` before anything outside the constructor sees the array, and an
/// `unknown` contributes nothing to that fold — it *adopts* what the typed elements
/// resolve to, or, where they are all `unknown`, `text`. So `ARRAY[s,'ok']` is `text[]`
/// and `ARRAY[y,'ok']` is `m3[]` on 18.6, each decided by its one typed element, and
/// `ARRAY['ok']` is `text[]` with no typed element at all — none of the three
/// undecidable, where letting the literal erase made all three of them so and left
/// `m = ANY(ARRAY[s,'ok'])` answering rows 18.6 refuses.
pub(super) fn erases_enum_identity(expr: &Expr) -> bool {
    let text_shaped = |oid: TypeOid| matches!(oid, TypeOid::Text | TypeOid::TextArray);
    match expr {
        Expr::Cast { target_type, .. } => text_shaped(*target_type),
        Expr::CastTypmod { type_oid, .. } => text_shaped(*type_oid),
        Expr::Literal(Literal::String(_) | Literal::Null) => true,
        Expr::Bound(v) => v.type_oid().is_some_and(text_shaped),
        Expr::Alias { expr, .. } => erases_enum_identity(expr),
        Expr::Array(elements) => elements
            .iter()
            .any(|e| !unknown_array_element(e) && erases_enum_identity(e)),
        _ => false,
    }
}

/// Whether `expr` is one of the shapes 18.6 hands `select_common_type` as an `unknown`
/// inside an `ARRAY[…]` — a bare string literal, a `NULL`, or an **undeclared** `$N`
/// (a client-declared one is planned as a cast and carries its declared type).
///
/// Read only by [`erases_enum_identity`]'s array arm, and only to *exclude* an element:
/// what an `unknown` adopts is decided by the elements beside it, so it can neither
/// carry an identity into the fold nor take one out of it.
///
/// A bare `NULL` reaches the gate in **two** shapes and both are here. Alone in its
/// array it is the literal, the fold having no target to write; beside a typed element
/// it is a `Cast` to the unified target, written by this pass's own
/// [`unify_common_type_branches`] over the `ARRAY` branches before any gate reads the
/// node — and a cast is what [`erases_enum_identity`] answers `true` for, so
/// omitting this shape left `m = ANY(ARRAY[s, NULL])` answering rows 18.6 refuses while
/// `ARRAY[s, 'ok']` refused, the difference being which of the two the pass rewrote.
/// The cast it writes is **indistinguishable from a written `NULL::text`**, which 18.6
/// types `text` rather than `unknown` (`ARRAY[m, NULL::text]` is
/// `42804 ARRAY types mood and text cannot be matched` there, where `ARRAY[m, NULL]` is
/// `mood[]`), so that spelling is answered here as the pass's output is — row 104's
/// under-refusal, which it already had from the cast alone and does not gain here. A
/// `NULL::varchar(3)` element is a `CastTypmod` this arm does not match, so it erases as
/// before: `m = ANY(ARRAY[s, NULL::varchar(3)])` answers where 18.6 is
/// `42883 … mood = text`, measured, while its enum-element twin is
/// `42804 ARRAY types mood and character varying cannot be matched` on both.
fn unknown_array_element(expr: &Expr) -> bool {
    match strip_alias(expr) {
        Expr::Literal(Literal::String(_) | Literal::Null) | Expr::Parameter { .. } => true,
        Expr::Cast { expr, .. } => matches!(strip_alias(expr), Expr::Literal(Literal::Null)),
        _ => false,
    }
}

/// `expr` with any [`Expr::Alias`] wrappers taken off — the shape a node-kind test has
/// to ask about, an alias being a label rather than a construct.
fn strip_alias(expr: &Expr) -> &Expr {
    match expr {
        Expr::Alias { expr, .. } => strip_alias(expr),
        other => other,
    }
}

/// The identity a gate reads for `expr`: [`EnumIdentity::Undecided`] where
/// [`erases_enum_identity`] says so, else the walk's answer.
///
/// **Both of this reader's leaves are derivations now, and the subquery one is a second
/// derivation beside the descriptor.** A gate's operand may be a scalar subquery or a
/// column of a derived relation, and both resolve through a `ColumnDesc` — which carried
/// no identity for a *projected expression* (`EDGES.md` row 107), so `m > (SELECT min(m)
/// FROM w)` and `t.m = v.z` over `(SELECT min(m) …) v` read a `None` this function took
/// for *not an enum* and refused ordinary SQL 18.6 answers. `Expr::to_column_desc`
/// derives the identity at the one site that builds such a descriptor, which is upstream
/// of those namespaces rather than in any of them.
///
/// That leaves one thing a descriptor cannot carry however it is built, and the subquery
/// leaf here is why it does not have to: a descriptor is built under the sub-plan's
/// **own** scope, so a projection naming an *enclosing* query's column has no namespace
/// to resolve it in — `m > (SELECT m FROM (VALUES (1)) q)` refuses where 18.6 answers.
/// [`resolved_subquery_type_in`] chains the enclosing levels
/// ([`ResolvedLeafScope::column_enum`]), so asking it instead closes that shape, and it is
/// the same reader the `IN (SELECT …)` door already uses.
///
/// **Only its *decided* answer is taken, which is the posture [`wildcard_column_type`]
/// and [`set_op_enum_identity`] already hold at their own sub-plan doors.** That reader is
/// three-valued, and its middle state would fall an enum pair back to the OID rule —
/// which at a comparison is an *accept*: taking it was measured turning `DELETE FROM w
/// WHERE m = (SELECT s::text FROM w)`, which this engine refuses as 18.6 refuses it, into
/// a silent row removal. So an undecidable sub-plan column reads here as no enum and the
/// pair refuses, loud. What that costs is the cast whose enum name the parser dropped:
/// `m > (SELECT 'ok'::mood)` refuses where 18.6 answers, row 104's erasure, and it closes
/// where that does. The `IN (SELECT …)` door takes the opposite side of the same trade,
/// and that disagreement is row 114's, not this reader's to settle.
fn enum_identity_in(expr: &Expr, scope: &ColumnScope, schema: Option<&Schema>) -> EnumIdentity {
    if erases_enum_identity(expr) {
        return EnumIdentity::Undecided;
    }
    let Some(schema) = schema else {
        return expr_enum_in(expr, scope, None)
            .map_or(EnumIdentity::NotAnEnum, EnumIdentity::Named);
    };
    crate::executor::expr_enum_with(
        expr,
        &crate::executor::EnumLeaves {
            column: &|table, name| scope.column_enum(table, name),
            subquery: &|plan| match resolved_subquery_type_in(
                plan,
                &EnclosingLeaves::Clause(scope),
                schema,
            ) {
                SubqueryType::Castable(resolved) | SubqueryType::Uncastable(resolved) => {
                    resolved.enum_type.name().map(ToOwned::to_owned)
                }
                SubqueryType::Deferred => None,
            },
        },
    )
    .map_or(EnumIdentity::NotAnEnum, EnumIdentity::Named)
}

/// [`certain_operand_type`] paired with the identity [`enum_identity_in`] derives over
/// the same expression — the **operand** currency of the comparison rulebook.
///
/// The two arm lists agree wherever this answers at all, which is what makes the pairing
/// one fact rather than two: a `Column` reads both halves from this scope, a `Literal`,
/// `Bound` and both `Cast` spellings carry no identity in either (a cast erases it —
/// `EDGES.md` row 104), and `Alias` looks through in both. Every arm this reader answers
/// `None` for leaves the whole verdict `None`, so the walk's wider arms cannot
/// contribute an identity to a type this one declined to give.
fn certain_operand_resolved(
    expr: &Expr,
    scope: &ColumnScope,
    schema: &Schema,
) -> Option<ResolvedType> {
    certain_operand_type(expr, scope).map(|oid| ResolvedType {
        oid,
        enum_type: enum_identity_in(expr, scope, Some(schema)),
    })
}

/// [`concrete_branch_type`] paired with [`expr_enum_in`] — the **branch** currency of
/// the common-type rulebook, for the same reason [`certain_operand_resolved`] is the
/// operand one.
fn concrete_branch_resolved(
    expr: &Expr,
    scope: &ColumnScope,
    schema: Option<&Schema>,
) -> Option<ResolvedType> {
    concrete_branch_type(expr, scope).map(|oid| ResolvedType {
        oid,
        enum_type: enum_identity_in(expr, scope, schema),
    })
}

/// [`certain_operand_resolved`] / [`branch_type_with_subqueries`] with the **complete**
/// type walk behind them — the reach an enum identity needs, and the reach the two
/// readers beside it deliberately do not have.
///
/// A gate reader answers `None` for a computed operand on purpose: its verdict must not
/// rest on a guess, and what that costs over a non-enum pair is `EDGES.md` row 65. An
/// enum identity changed that trade at one pair and not at the others. The identity is
/// derived by a walk that *is* exhaustive ([`expr_enum_in`]), so a `mood` column inside
/// `lower(s)`'s sibling was read and then thrown away with the `None` its OID came back
/// as, and the whole of `EDGES.md` row 114 was reachable again one function call in —
/// `DELETE FROM w WHERE m > lower(s)` took the row 18.6 keeps by refusing, and
/// `UPDATE w SET m = greatest(m, lower(s))` rewrote a stored member.
///
/// So the complete reader stands **behind** the shallow one rather than replacing it:
/// every operand the shallow reader answers for keeps the answer it had, and the widening
/// is spent only where [`enum_pair_names_one`] finds an enum — in the comparison *pair*,
/// or anywhere in a branch *set*. The two differ in what else they move: a comparison
/// pair with no enum keeps its verdict, but inside a set that names one every branch is
/// widened, so a non-enum pair there is judged too — `COALESCE(n + 0, lower(s),
/// COALESCE(m, 'ok'))` answered before and is now 18.6's `42804 … integer and text`
/// (`EDGES.md` rows 65 and 114). Replacing it outright is a second, larger change — measured green over the
/// whole fixture corpus and closing most of the row-65 shapes measured against it, which
/// is row 65's close and not this one's.
fn complete_resolved(expr: &Expr, scope: &ColumnScope, schema: &Schema) -> Option<ResolvedType> {
    crate::planner::unknown_aware_type(
        expr,
        &ClauseLeafScope {
            columns: scope,
            schema: Some(schema),
        },
    )
    .map(|oid| ResolvedType {
        oid,
        enum_type: enum_identity_in(expr, scope, Some(schema)),
    })
}

/// Whether any of `types` names an enum — the predicate that spends
/// [`complete_resolved`]'s reach.
///
/// [`EnumIdentity::Undecided`] is **not** one: an operand a `::text` cast or a bare
/// literal leaves undecidable falls back to the OID rule (`EDGES.md` row 104), and a
/// widened read of its sibling could only turn that fallback into an over-refusal.
fn enum_pair_names_one<'a>(types: impl IntoIterator<Item = &'a Option<ResolvedType>>) -> bool {
    types
        .into_iter()
        .any(|t| t.as_ref().is_some_and(|t| t.enum_type.name().is_some()))
}

/// The operand pair the comparison gate judges: the shallow read, widened by
/// [`complete_resolved`] **only where the widened pair names an enum**.
///
/// One function rather than a reader applied twice, because the decision is the
/// *pair*'s: `lower(s) > (CASE WHEN … THEN m END)` has no enum in either shallow read
/// and one in the widened pair, so a per-operand rule could not see it. Widening both
/// sides and judging the widened types is right exactly when the verdict is the enum's
/// — a pair with no enum keeps the conservatism `EDGES.md` row 65 records, verdict for
/// verdict.
fn operand_pair(
    left: &Expr,
    right: &Expr,
    scope: &ColumnScope,
    schema: &Schema,
) -> (Option<ResolvedType>, Option<ResolvedType>) {
    let shallow = (
        certain_operand_resolved(left, scope, schema),
        certain_operand_resolved(right, scope, schema),
    );
    let widened = (
        shallow
            .0
            .clone()
            .or_else(|| complete_resolved(left, scope, schema)),
        shallow
            .1
            .clone()
            .or_else(|| complete_resolved(right, scope, schema)),
    );
    if enum_pair_names_one([&widened.0, &widened.1]) {
        widened
    } else {
        shallow
    }
}

/// [`operand_pair`] where the right-hand operand is **not** a sibling `Expr` — an
/// `IN (SELECT …)`'s sub-plan column or an `op ANY/ALL` array's element — and its type
/// therefore arrives already read.
///
/// The rule is [`operand_pair`]'s, with one difference that follows from the position
/// rather than from a choice: those two readers have no *shallow* half. A sub-plan's
/// column is read by [`branch_subquery_type`] and an array's element by
/// [`array_element_resolved`], both of which stand where [`complete_resolved`] stands —
/// so where the widened pair names no enum, the right side falls back to `None`, not to
/// a shallower answer. That keeps the widening spent only on the enum verdict, exactly
/// as [`operand_pair`] does: a pair with no enum in it is judged by the left-hand
/// operand alone and keeps the answer it had, `EDGES.md` rows 60 and 65 unmoved.
fn one_sided_operand_pair(
    left: &Expr,
    right: Option<ResolvedType>,
    scope: &ColumnScope,
    schema: &Schema,
) -> (Option<ResolvedType>, Option<ResolvedType>) {
    let shallow_left = certain_operand_resolved(left, scope, schema);
    let widened_left = shallow_left
        .clone()
        .or_else(|| complete_resolved(left, scope, schema));
    if enum_pair_names_one([&widened_left, &right]) {
        (widened_left, right)
    } else {
        (shallow_left, None)
    }
}

/// [`one_sided_operand_pair`] fed to the shared
/// [`one_sided_comparison_legality`](crate::planner::one_sided_comparison_legality) —
/// the one-sided twin of the `compare` closure in [`check_expr_node_semantics`], whose
/// rule (and the reason it converts one way only) is the rulebook's, so this walk and
/// the DDL-time one cannot come to hold different halves of it.
fn compare_against_one_sided(
    left: &Expr,
    right: Option<ResolvedType>,
    symbol: &str,
    scope: &ColumnScope,
    schema: &Schema,
) -> Result<()> {
    let (lt, rt) = one_sided_operand_pair(left, right, scope, schema);
    crate::planner::one_sided_comparison_legality(lt, rt.as_ref(), left, symbol, schema)
}

/// The **element** type of an `op ANY/ALL`'s right-hand array under this clause's
/// scope — the complete walk's read of the array operand, projected through the
/// rulebook's
/// [`array_element_resolved_type`](crate::planner::array_element_resolved_type), which
/// is where the projection rule and its identity clause live so the DDL-time walk reads
/// the same one.
fn array_element_resolved(
    expr: &Expr,
    scope: &ColumnScope,
    schema: &Schema,
) -> Option<ResolvedType> {
    crate::planner::array_element_resolved_type(&complete_resolved(expr, scope, schema)?)
}

/// Eagerly validate every **constant** string cast (`Cast(Literal::String → target)`)
/// in `expr`, failing loud if a literal can't be coerced.
///
/// This reproduces Postgres's parse-time handling of a constant coercion: PG resolves
/// and checks a constant `'literal'::type` (and every implicit cast the planner inserts
/// on a constant) eagerly, so the error is **independent of eval order or which branch
/// runs**.
///
/// It closes two gaps a lazy eval-time cast leaves:
/// - an unparseable element *after* a matching one in an `IN` list — `n IN (42,
///   'abc')` where `n = 42` matches short-circuits before `'abc'::int` is reached;
/// - an unparseable *untaken* `CASE`/`COALESCE` branch — `CASE WHEN 1=1 THEN 1 ELSE
///   'x' END`, `COALESCE(NULL, 1, 'x')`.
///
/// Only a **constant string** cast is checked. A runtime operator in an untaken branch
/// (`1/0`) is left to eval — matching Postgres, which folds the constant coercion but
/// does not eagerly evaluate the division.
///
/// A **typmod'd** cast ([`Expr::CastTypmod`]) is not a node kind this pass matches;
/// [`validate_arg_const_casts`] is the entry point that takes it, and its doc has the
/// measurement for why the two differ.
///
/// # Errors
///
/// The coercion error of the first unparseable constant string cast — identical to the
/// eval-time cast, since it calls the same [`crate::executor::coerce_value`].
pub fn validate_const_casts(expr: &Expr) -> Result<()> {
    validate_const_casts_inner(expr, false)
}

/// [`validate_const_casts`] over one more node kind — a **typmod'd** constant cast.
///
/// That is one `'lit'::<type>(…)` spelling per [`Typmod`](crate::planner::Typmod)
/// variant — `Numeric` (`numeric(p,s)`), `Length` (`varchar(n)`, `char(n)`) and
/// `Precision` (the sub-second `(p)` of `timestamp`/`timestamptz`/`time`/`interval`,
/// which `parser::expr`'s `a_temporal_precision_cast_builds_cast_typmod` pins as
/// reaching this node kind) — and the one caller is `planner::transform_call_args`,
/// 18.6's argument transform.
///
/// The two entry points share the walk and the node rule and differ only in whether an
/// [`Expr::CastTypmod`] counts, because the *modifier itself* is deliberately not
/// applied by either: `coerce_type` passes `inputTypeMod = -1` to `stringTypeDatum` for
/// every type but `interval` ("Any length constraint will be applied later by our
/// caller", `parse_coerce.c`), and that caller's `coerce_type_typmod` node is folded by
/// the **planner**, below every parse-analysis verdict. Measured on 18.6, the pair that
/// separates the input conversion from the modifier:
/// `concat(DISTINCT 'abc'::numeric(5,2))` is `22P02` — the input function's error, ahead
/// of the signature and the quantifier — while `concat(DISTINCT '12345'::numeric(3,1))`
/// is `42809`, the quantifier's, ahead of the modifier's own `22003` (which the
/// unquantified `concat('12345'::numeric(3,1))` does answer, on both engines). So
/// applying the modifier here would be a *third* answer neither engine gives, and the
/// controls that say so are `upper(DISTINCT 'abc'::varchar(2))` → `42809` and
/// `SELECT 'abcdef'::varchar(2)` → `ab`, both unchanged on both engines. 18.6's own
/// `interval` carve-out is **reachable** here: `ResolvedColumnType::typmod` — the one
/// conversion that decides [`Expr::Cast`] against [`Expr::CastTypmod`] — reads a
/// sub-second precision alongside a `numeric(p,s)` and a `varchar`/`char` length, so an
/// `interval(p)` cast arrives as the extra kind. Not applying its modifier is what two
/// measured spellings do not separate: `concat(DISTINCT 'xyz'::interval(3))` is `22007`
/// and `concat(DISTINCT '0.5 sec'::interval(0))` is `42809`, each on both engines.
///
/// **Why the plan-time pass does not take the extra kind**, which is a pglet fact rather
/// than an 18.6 one: [`validate_const_casts`] runs ahead of the range table, so widening
/// it moves an *unquantified* statement that agrees with 18.6 into a divergence —
/// measured, `SELECT 'abc'::numeric(5,2) FROM nosuchrel` is `42P01` on both engines and
/// becomes `22P02` here with the kind added there — on a build whose **whole fixture
/// corpus is green**, so this split is held by that probe and by nothing else. The
/// typmod'd cast's plan-time gap (its `22P02` deferred to eval) is recorded in
/// `EDGES.md` with the live silent members it leaves — the
/// `set_config` arm's discarded argument, and every route reaching
/// `validate_stored_expr`'s probe while its table holds no row (the routes and that
/// condition are recorded there rather than a count, which this line carried and which
/// has twice been wrong; a column `DEFAULT` is a persisted expression that is *not* in
/// that class — the plan-time pass answers its cooked expression).
///
/// # Errors
///
/// As [`validate_const_casts`], over the two cast spellings.
pub fn validate_arg_const_casts(expr: &Expr) -> Result<()> {
    validate_const_casts_inner(expr, true)
}

/// The shared walk behind [`validate_const_casts`] and [`validate_arg_const_casts`]:
/// one bottom-up traversal, one node rule, and one flag for whether a typmod'd cast is
/// a constant cast for this caller.
fn validate_const_casts_inner(expr: &Expr, typmod_casts: bool) -> Result<()> {
    crate::depth::check_stack()?;
    let cast = match expr {
        Expr::Cast {
            expr: inner,
            target_type,
        } => Some((inner, *target_type)),
        Expr::CastTypmod {
            expr: inner,
            type_oid,
            ..
        } if typmod_casts => Some((inner, *type_oid)),
        _ => None,
    };
    if let Some((inner, target)) = cast
        && let Expr::Literal(Literal::String(s)) = inner.as_ref()
    {
        crate::executor::coerce_value(Value::Text(s.clone()), target)?;
    }
    // A cast to an enum is this rule's too — its target is carried by the
    // `StringFunc::EnumIn` node under the `text` cast rather than by the cast's OID.
    crate::planner::enum_in_constant_legality(expr)?;
    // A chain is walked from this frame (`Expr::chain_operands`).
    let Some(operands) = expr.chain_operands() else {
        return expr.try_for_each_child(|child| validate_const_casts_inner(child, typmod_casts));
    };
    validate_const_casts_inner(operands.first, typmod_casts)?;
    if let Some(second) = operands.second {
        validate_const_casts_inner(second, typmod_casts)?;
    }
    operands
        .rest
        .iter()
        .try_for_each(|item| validate_const_casts_inner(item, typmod_casts))
}

/// The concrete `TypeOid` a branch contributes to unification, or `None` for an
/// "unknown" branch (a bare string literal or NULL) that should adopt a sibling's
/// type, and for one this reader cannot type. Conservative: a typed column, an
/// explicit cast, a non-string literal, and the computed shapes whose type is certain
/// once their operands' are ([`BranchReach::Computed`]) — anything else is left out
/// of target selection rather than guessed at.
fn concrete_branch_type(expr: &Expr, scope: &ColumnScope) -> Option<TypeOid> {
    branch_type_with_reach(expr, scope, BranchReach::Computed)
}

/// Which branch shapes [`branch_type_with_reach`] types.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchReach {
    /// The written shapes only — a column, a cast, a literal, and the aggregate, array
    /// and alias arms over those. What an `IN` list's conversion to 18.6's array form
    /// reads ([`in_list_array_type`]): it converts only where every operand's type is
    /// certain by construction (`EDGES.md` row 280).
    Written,
    /// The written shapes, plus the computed ones whose result is in the numeric
    /// category and certain once their operands' types are: `+ - * / %` and `^` (and
    /// their `mod` / `power` / `pow` calls), unary `-` / `+`, a math function, `length` /
    /// `strpos` / `position` / `octet_length`, `EXTRACT` / `date_part`, a `date` less a
    /// `date`, `random()`, and a nested `COALESCE` / `CASE` every branch
    /// of which this reader types or is of unknown type ([`is_unknown_branch`]) — and a
    /// column operand only where its type is certain
    /// ([`ColumnScope::column_type_certain`]), an `ARRAY[…]` only where every element is
    /// typed, a scalar subquery only to a reader handed a leaf for one
    /// ([`branch_type_in`]); and, outside the numeric category, a call whose result
    /// follows a typed argument ([`argument_following_type`]).
    /// Without them a computed branch was left out of target selection, so
    /// `COALESCE(i, f + 0)` unified to `integer` and cast the `float8` branch to it —
    /// `2.5` answered `2` — and `COALESCE(i + 0, f + 0)`, with no branch typed, was
    /// declared by its first branch's `integer` over `float8` values, where 18.6 takes
    /// the common type, `double precision`, for both.
    Computed,
}

/// The numeric-category types, the only results [`BranchReach::Computed`]'s computed
/// arms answer.
const fn is_numeric_category(t: TypeOid) -> bool {
    matches!(
        t,
        TypeOid::Int2
            | TypeOid::Int4
            | TypeOid::Int8
            | TypeOid::Numeric
            | TypeOid::Float4
            | TypeOid::Float8
    )
}

/// [`concrete_branch_type`] at a given [`BranchReach`], by a reader that holds no
/// catalog — a scalar subquery is not typed ([`no_subquery_type`]).
fn branch_type_with_reach(expr: &Expr, scope: &ColumnScope, reach: BranchReach) -> Option<TypeOid> {
    branch_type_in(expr, scope, reach, &no_subquery_type)
}

/// [`branch_type_with_reach`], with the type a scalar subquery is read at for the
/// computed reach (`subquery`).
///
/// The leaf is asked for a subquery wherever it stands. The pass resolves a `COALESCE`,
/// a `CASE`, an `ARRAY[…]` and the variadics with a reader that holds no catalog, to
/// which a scalar subquery is typed only by a `Cast` written over it — and under a fold
/// the pass writes that cast over every subquery the certain leaf types
/// ([`type_scalar_subquery_branches`]), so a fold this reader types with that leaf is
/// one the pass's own reader types too. Outside a fold the pass rewrites nothing by a
/// subquery's type, having no reader that sees one.
fn branch_type_in(
    expr: &Expr,
    scope: &ColumnScope,
    reach: BranchReach,
    subquery: SubqueryLeaf<'_>,
) -> Option<TypeOid> {
    let recurse = |e: &Expr| branch_type_in(e, scope, reach, subquery);
    if reach == BranchReach::Computed
        && let Some(computed) = computed_reach_type(expr, &recurse)
    {
        return Some(computed);
    }
    match expr {
        // At the computed reach a column is typed only where its type is certain: a
        // derived table's column defined by an expression this pass did not type as
        // written carries the walkers' type for it, which can be a guess (`EDGES.md` row
        // 54's open band, row 152), and a target chosen from a guess casts its sibling at
        // the wrong type — `COALESCE(x * 1, b * 1)` over such an `integer` `x` holding
        // `3.5` answered `4`.
        Expr::Column { table, name }
            if reach == BranchReach::Computed
                && !scope.column_type_certain(table.as_deref(), name) =>
        {
            None
        }
        Expr::Column { table, name } => scope.coercion_type(table.as_deref(), name),
        Expr::Cast { target_type, .. } => Some(*target_type),
        // A typmod'd cast is concrete at its target type (like a plain cast) — so a
        // `COALESCE(x::numeric(4,1), 0)` branch unifies the `0` to numeric.
        Expr::CastTypmod { type_oid, .. } => Some(*type_oid),
        // A small int constant types as `int4` (else `int8`) — PG constant typing, so
        // `CASE WHEN 1=1 THEN 1 ELSE 'x' END` reports "integer" (not "bigint").
        Expr::Literal(Literal::Int(n)) => Some(int_literal_type(*n)),
        Expr::Literal(Literal::Float(_)) => Some(TypeOid::Float8),
        // A decimal/exponent literal is `numeric` under Postgres constant typing, and
        // it is *concrete* — it drives the target like any other typed branch. Omitting
        // it made `COALESCE(int_col, 1.5)` resolve to `integer` (the int branch alone)
        // where Postgres says `numeric`, then return a numeric value under an `int4`
        // declaration when the literal branch won.
        Expr::Literal(Literal::Numeric(_)) => Some(TypeOid::Numeric),
        Expr::Literal(Literal::Bool(_)) => Some(TypeOid::Bool),
        // An aggregate's type follows its argument's, and it is every bit as concrete
        // as a column's — without this a `HAVING COALESCE(min(i4), max(tx))` had two
        // *unknown* branches, so the no-common-type check had nothing to compare and
        // the clause was accepted where Postgres reports `42804`.
        // Its argument's certainty is the one this pass recorded on the node
        // ([`aggregate_arg_certainty`]) and every other reader reads, so `avg` of a
        // certain integer is `numeric` here exactly where the walkers declare it. At the
        // computed reach an argument this reader cannot type leaves the aggregate
        // untyped (`count` aside): `avg`'s `float8` for an unknown argument is a guess.
        Expr::Aggregate {
            func,
            arg,
            certainty,
            ..
        } => aggregate_branch_type(*func, *certainty, recurse(arg), reach),
        // An `ARRAY[…]` branch is concrete at its elements' common type, one dimension
        // up. It is **never** unknown: Postgres's `transformArrayExpr` resolves an
        // all-unknown element set to `text`, so `ARRAY['a','b']` is `text[]` and
        // `ARRAY[NULL]` is `text[]` too. Answering `None` here was the second half of the
        // nested-array misreport — an untyped array element let the enclosing set unify
        // to a *sibling's scalar* type and then cast the array to it, so
        // `ARRAY[true, ARRAY[1,2]]` failed at eval as an impossible cast instead of at
        // plan time as "types boolean and integer[] cannot be matched".
        //
        // An element set with no common type answers `None` rather than propagating: the
        // walk that consumes this is bottom-up, so the inner node has already raised its
        // own error by the time the enclosing one asks. An element this scope cannot type
        // defers the same way a bare one does ([`is_untyped_column`]) — the `text`
        // fallback is Postgres's rule for a genuinely *unknown* element, not a licence to
        // guess at a correlated column's.
        Expr::Array(elements) => {
            let mut types = Vec::with_capacity(elements.len());
            for e in elements {
                let element = recurse(e);
                // A scalar-subquery element defers likewise where this reader's subquery
                // leaf does not type it — which is every reader's but the one that holds
                // a catalog to ask the sub-plan with. And at the computed reach an element
                // this reader cannot type is not an unknown: the `text[]` an all-unknown
                // list resolves to would be a guess, and a set read at that reach would
                // cast a typed sibling array to it.
                if element.is_none()
                    && (matches!(e, Expr::Subquery { .. })
                        || is_untyped_column(e, scope)
                        || reach == BranchReach::Computed && !is_unknown_branch(e))
                {
                    return None;
                }
                types.push(element.map(ResolvedType::plain));
            }
            select_common_type(&types, "ARRAY")
                .ok()
                .and_then(|t| t.map_or(TypeOid::Text, |t| t.oid).array_of())
        }
        Expr::Alias { expr: inner, .. } => recurse(inner),
        // A scalar subquery is typed at the computed reach where its reader says so —
        // which is where its sub-plan's column is certain, for the reader that holds a
        // catalog to ask it with ([`certain_scalar_subquery_type`]). At the written
        // reach, and to every other reader, it has no type of its own.
        Expr::Subquery { plan } if reach == BranchReach::Computed => subquery(plan),
        _ => None,
    }
}

/// [`branch_type_in`]'s aggregate arm: the type an aggregate `func` is read at, over an
/// argument whose recorded certainty is `certainty` and which the reader types as
/// `arg_type`. At the computed reach an argument the reader cannot type leaves the
/// aggregate untyped, but for `count` and a certain integer argument.
fn aggregate_branch_type(
    func: AggregateFunc,
    certainty: ArgCertainty,
    arg_type: Option<TypeOid>,
    reach: BranchReach,
) -> Option<TypeOid> {
    if reach == BranchReach::Computed
        && arg_type.is_none()
        && !matches!(certainty, ArgCertainty::CertainInteger(_))
        && !matches!(func, AggregateFunc::Count)
    {
        return None;
    }
    super::expr::aggregate_type_of(func, certainty, arg_type)
}

/// Convert a bare string literal compared with an aggregate call to the aggregate's type
/// where [`certain_aggregate_comparison_type`] answers one — the cast a literal beside a
/// column of that type gets ([`coerce_comparison_operands`]' rule 1), written while both
/// operands are still as the statement wrote them. Out of line, so the recursive frame
/// of [`resolve_expr_types`] carries nothing across the walk of its children.
#[inline(never)]
fn convert_literal_beside_aggregate(expr: &mut Expr, scope: &ColumnScope) {
    let Expr::BinaryOp { left, op, right } = expr else {
        return;
    };
    let Some(target) = certain_aggregate_comparison_type(*op, left, right, scope) else {
        return;
    };
    let literal = if matches!(**left, Expr::Aggregate { .. }) {
        right
    } else {
        left
    };
    let taken = std::mem::replace(&mut **literal, Expr::Default);
    **literal = cast_expr(taken, target);
}

/// The type a bare string literal compared with an aggregate call is read at, decided
/// from the two operands **as written**, before the pass resolves either — or `None`,
/// leaving the literal as it is.
///
/// `op` is a comparison ([`is_comparison_op`]), one operand is a written string literal
/// — the node kind, so a `$N` and a `NULL` are never reached — and the other an
/// aggregate whose result type [`BranchReach::Computed`] types: the reader that decides
/// every certainty this pass records, over the argument as written, with the
/// aggregate's own recorded verdict ([`aggregate_arg_certainty`], taken here exactly as
/// [`resolve_aggregate`] is about to record it where the pass has not reached the node
/// yet). The literal is then converted as one beside a column of that type is.
///
/// It declines where the type would be a guess or the conversion would come ahead of a
/// refusal 18.6 raises first: an argument holding an aggregate (`42803` later), one with
/// a column reference its own level does not bind or with a sub-plan
/// ([`binds_at_its_own_level`]), an argument the aggregate does not accept
/// ([`aggregate_legality`]), a `text`/`varchar` result, and a scope chaining an
/// enclosing level. That last is the correlated re-run
/// ([`super::correlation`]), which hands this pass the first run's resolved operands —
/// their casts include those a fold read at the written reach chose from a guess, so
/// `max(COALESCE(i, f + o.y))` comes back as a certain `integer` — and the first run's
/// decision is the one that stands.
#[inline(never)]
fn certain_aggregate_comparison_type(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    scope: &ColumnScope,
) -> Option<TypeOid> {
    if !is_comparison_op(op) {
        return None;
    }
    let ((Expr::Literal(Literal::String(_)), aggregate)
    | (aggregate, Expr::Literal(Literal::String(_)))) = (left, right)
    else {
        return None;
    };
    let Expr::Aggregate {
        func,
        arg,
        certainty,
        ..
    } = aggregate
    else {
        return None;
    };
    if scope.chains_enclosing_level() {
        return None;
    }
    if contains_aggregate(arg) || !binds_at_its_own_level(arg, scope) {
        return None;
    }
    let certainty = match certainty {
        ArgCertainty::Undecided => aggregate_arg_certainty(*func, arg, scope, &no_subquery_type),
        decided => *decided,
    };
    let arg_type = concrete_branch_type(arg, scope);
    if aggregate_legality(*func, arg_type).is_err() {
        return None;
    }
    aggregate_branch_type(*func, certainty, arg_type, BranchReach::Computed)
        .filter(|&t| is_string_coercible_target(t))
}

/// Whether every column reference in an aggregate's `arg` binds in `scope`
/// ([`Binding::Bound`]) and `arg` holds no sub-plan, whose own references bind only on
/// the correlated re-run.
///
/// A reference that binds nowhere here is either a refusal 18.6 raises first (`42703`,
/// `42P01`, `42702`) or an enclosing level's column, which makes the aggregate 18.6's
/// enclosing level's (`EDGES.md` row 303); one the scope cannot judge
/// ([`Binding::Uncertain`]) is neither proven. `count` reads no argument type, so the
/// type reader alone cannot stand in for this.
fn binds_at_its_own_level(arg: &Expr, scope: &ColumnScope) -> bool {
    !arg.any_node(&|e| match e {
        Expr::Column { table, name } => {
            Some(scope.binding(table.as_deref(), name) != Binding::Bound)
        }
        Expr::Subquery { .. } | Expr::Exists { .. } | Expr::InSubquery { .. } => Some(true),
        _ => None,
    })
}

/// The type [`BranchReach::Computed`] reads a computed `expr` at: a numeric-category
/// [`computed_branch_type`], or the type of a call whose result follows an argument
/// ([`argument_following_type`]). `operand` is the reader itself.
fn computed_reach_type(expr: &Expr, operand: &dyn Fn(&Expr) -> Option<TypeOid>) -> Option<TypeOid> {
    argument_following_type(expr, operand)
        .or_else(|| computed_branch_type(expr, operand).filter(|t| is_numeric_category(*t)))
}

/// The result type of a string-function call whose result follows one of its arguments,
/// where that argument is typed: `substring` / `substr` over a `bytea` answer `bytea` and
/// over any other type `text` ([`super::expr::substring_result_type`]); `date_trunc`
/// answers its source's type ([`super::expr::date_trunc_result_type`]). The rule the two
/// declared-type walkers share ([`super::expr::shared_leaf_cell_type`]), read here only
/// where the deciding argument is typed, so a fold's unknown branch is converted to the
/// type the walkers then declare: `COALESCE(substr(b, 1), 'zz')` over a `bytea` `b` is
/// `\x7a7a`, as on 18.6, where the `'zz'` stayed text under a `bytea` declaration.
fn argument_following_type(
    expr: &Expr,
    operand: &dyn Fn(&Expr) -> Option<TypeOid>,
) -> Option<TypeOid> {
    use super::expr::{StringFunc, date_trunc_result_type, substring_result_type};
    match expr {
        Expr::StringFn {
            func: StringFunc::Substring | StringFunc::Substr,
            args,
        } => args
            .first()
            .and_then(operand)
            .map(|t| substring_result_type(Some(t))),
        Expr::StringFn {
            func: StringFunc::DateTrunc,
            args,
        } => args
            .get(1)
            .and_then(operand)
            .map(|t| date_trunc_result_type(Some(t))),
        _ => None,
    }
}

/// The computed arms of [`BranchReach::Computed`]: the type of `expr` where it is one of
/// those shapes and every operand the shape needs is typed, `None` otherwise. `operand`
/// is the reader itself.
fn computed_branch_type(
    expr: &Expr,
    operand: &dyn Fn(&Expr) -> Option<TypeOid>,
) -> Option<TypeOid> {
    use super::expr::{
        StringFunc, arith_result_type, math_fn_result_type, numeric_family_result, pow_result_type,
    };
    let numeric = |e: &Expr| operand(e).filter(|t| is_numeric_category(*t));
    match expr {
        // `mod(…)` / `power(…)` / `pow(…)` are typed as the operator each is.
        Expr::BinaryOp { left, op, right } => match op.operator_form() {
            // Both operands typed. An operand of unknown type — an untyped `NULL`, a `$N` —
            // leaves the operator untyped: nothing converts the unknown to its sibling's
            // type, so the value the operator returns is whatever the bound value makes
            // it, and a branch read at the sibling's type would be left uncast under a
            // declaration its value need not have.
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                match (operand(left)?, operand(right)?) {
                    // A date less a date is a count of days.
                    (TypeOid::Date, TypeOid::Date) if *op == BinaryOp::Sub => Some(TypeOid::Int4),
                    (l, r) if is_numeric_category(l) && is_numeric_category(r) => {
                        arith_result_type(*op, l, r)
                    }
                    _ => None,
                }
            }
            // 18.6 registers no float `%`, so only an exact pair is typed.
            BinaryOp::Mod => numeric(left)
                .zip(numeric(right))
                .and_then(|(l, r)| arith_result_type(BinaryOp::Mod, l, r))
                .filter(|t| !matches!(t, TypeOid::Float4 | TypeOid::Float8)),
            BinaryOp::Pow => numeric(left)
                .zip(numeric(right))
                .map(|(l, r)| pow_result_type(numeric_family_result(l, r))),
            _ => None,
        },
        Expr::Negate(inner) | Expr::UnaryPlus(inner) => numeric(inner),
        Expr::MathFn { func, args } => args
            .first()
            .and_then(numeric)
            .map(|t| math_fn_result_type(*func, Some(t), args.len() >= 2)),
        Expr::StringFn {
            func:
                StringFunc::Length | StringFunc::Strpos | StringFunc::Position | StringFunc::OctetLength,
            ..
        } => Some(TypeOid::Int4),
        // `EXTRACT` is `numeric`, `date_part` `double precision`, whatever the source.
        Expr::Extract { retnumeric, .. } => Some(if *retnumeric {
            TypeOid::Numeric
        } else {
            TypeOid::Float8
        }),
        // A bound value carries its type.
        Expr::Bound(value) => value.type_oid(),
        // A niladic builtin's type is its own (`random()` is `double precision`).
        Expr::Builtin(func) => Some(super::expr::builtin_fn_result_type(*func)),
        // `greatest`/`least` and `nullif` over arguments every one of which is typed, by
        // the rule the declared-type walkers read (`variadic_fn_result_type`) — so an
        // aggregate over one is certain where its arguments are
        // ([`aggregate_arg_certainty`]), and typed here as the walkers type it.
        Expr::VariadicFn {
            func: func @ (VariadicFunc::Greatest | VariadicFunc::Least | VariadicFunc::NullIf),
            args,
        } => {
            let types: Vec<Option<TypeOid>> = args
                .iter()
                .map(|a| {
                    operand(a)
                        .map(Some)
                        .or_else(|| is_unknown_branch(a).then_some(None))
                })
                .collect::<Option<_>>()?;
            types
                .iter()
                .any(Option::is_some)
                .then(|| super::expr::variadic_fn_result_type(*func, &types))
        }
        Expr::Coalesce(branches) => nested_fold_type(branches.iter(), operand),
        Expr::Case {
            when_clauses,
            else_result,
            ..
        } => nested_fold_type(
            else_result
                .iter()
                .map(AsRef::as_ref)
                .chain(when_clauses.iter().map(|(_, result)| result)),
            operand,
        ),
        _ => None,
    }
}

/// A nested `COALESCE`'s or `CASE`'s type as a branch: the common type of its branches
/// (a `CASE`'s `ELSE` first, as 18.6 folds it) where every branch is typed by `operand`
/// or is of unknown type ([`is_unknown_branch`]), and `None` otherwise.
fn nested_fold_type<'a>(
    branches: impl Iterator<Item = &'a Expr>,
    operand: &dyn Fn(&Expr) -> Option<TypeOid>,
) -> Option<TypeOid> {
    let mut types = Vec::new();
    for branch in branches {
        match operand(branch) {
            Some(t) => types.push(Some(ResolvedType::plain(t))),
            None if is_unknown_branch(branch) => types.push(None),
            None => return None,
        }
    }
    select_common_type(&types, "COALESCE")
        .ok()
        .flatten()
        .map(|t| t.oid)
}

/// Coerce operands in a comparison to compatible types.
///
/// 1. **column vs bare string literal** — cast the string to the column's type when
///    that type is string-coercible ([`is_string_coercible_target`]). The cast is
///    unconditional on the literal's contents: an unparseable spelling reaches the
///    eval-time `Cast` and fails loud (Postgres resolves `col <op> 'literal'` to the
///    column type and errors at run time — `WHERE n = 'abc'` → `22P02`), rather than
///    silently comparing a `Text` against the typed column and matching zero rows.
/// 2. **cast vs bare string literal** (`$1::numeric = '1.5'`) — rule 1 with the
///    anchor's type read off an explicit `Cast`/`CastTypmod` rather than the scope. That
///    is also where a parameter whose type the client declared at `Parse` lands, since
///    the parser reads one as a `Cast` over it
///    (`parser::parse_single_with_param_types`), so a declared `numeric` `$1 = '1.5'`
///    resolves the literal to `numeric`, as 18.6 resolves `unknown` beside a typed
///    operand, where it used to compare a `numeric` against a `text` and refuse `42883`.
///    Any other computed anchor (`length('ab') = '2'`) is still not one.
/// 3. **concrete literal vs bare string literal** (`SELECT true = 'yes'`) — the
///    unknown string adopts the *other* literal's concrete type (Postgres's
///    unknown-literal rule), so `1 <> '1 '` compares as integers and `'abc' = 1`
///    errors `22P02`. Two unknown strings (`'a' = 'b'`) have no concrete side and stay
///    text.
/// 4. Otherwise leave operands as-is (the executor handles cross-numeric comparisons).
///
/// A literal beside an aggregate is converted before the operands are resolved, so it
/// reaches here as a cast ([`convert_literal_beside_aggregate`]).
fn coerce_comparison_operands(left: Expr, right: Expr, scope: &ColumnScope) -> (Expr, Expr) {
    // Rule 1: a column with a known, string-coercible type on one side, a bare string
    // literal on the other → cast the literal to the column's type.
    if let (Expr::Column { table, name }, Expr::Literal(Literal::String(_))) = (&left, &right)
        && let Some(col_type) = scope.coercion_type(table.as_deref(), name)
        && is_string_coercible_target(col_type)
    {
        return (left, cast_expr(right, col_type));
    }
    if let (Expr::Literal(Literal::String(_)), Expr::Column { table, name }) = (&left, &right)
        && let Some(col_type) = scope.coercion_type(table.as_deref(), name)
        && is_string_coercible_target(col_type)
    {
        return (cast_expr(left, col_type), right);
    }

    // Rule 2: an explicit cast — a declared `$N` included — beside a bare string literal.
    if let (Some(t), Expr::Literal(Literal::String(_))) = (cast_target_type(&left), &right)
        && is_string_coercible_target(t)
    {
        return (left, cast_expr(right, t));
    }
    if let (Expr::Literal(Literal::String(_)), Some(t)) = (&left, cast_target_type(&right))
        && is_string_coercible_target(t)
    {
        return (cast_expr(left, t), right);
    }

    // Rule 3: a concrete (non-string) literal beside a bare string literal → the string
    // adopts the concrete side's type (PG's unknown-literal resolution). Restricted to
    // string-coercible target types; two unknown strings have no concrete side and fall
    // through to a text comparison.
    if let (Expr::Literal(Literal::String(_)), Expr::Literal(other)) = (&left, &right)
        && let Some(t) = concrete_literal_type(other).filter(|&t| is_string_coercible_target(t))
    {
        return (cast_expr(left, t), right);
    }
    if let (Expr::Literal(other), Expr::Literal(Literal::String(_))) = (&left, &right)
        && let Some(t) = concrete_literal_type(other).filter(|&t| is_string_coercible_target(t))
    {
        return (left, cast_expr(right, t));
    }

    // No coercion needed.
    (left, right)
}

/// The type an explicit `Cast` / `CastTypmod` node produces, for rule 2 of
/// [`coerce_comparison_operands`]; `None` for any other expression.
const fn cast_target_type(expr: &Expr) -> Option<TypeOid> {
    match expr {
        Expr::Cast { target_type, .. } => Some(*target_type),
        Expr::CastTypmod { type_oid, .. } => Some(*type_oid),
        _ => None,
    }
}

/// Coerce the operands of `@>` / `<@`: a bare string literal beside a **container**
/// operand (jsonb or an array) adopts that operand's type.
///
/// Postgres's containment operators are `jsonb @> jsonb` and `anyarray @> anyarray`,
/// so an unknown-typed literal resolves to the other operand's type — which is why
/// `WHERE data @> '{"k":1}'` is the spelling its own documentation uses. Without
/// this the literal stays `text` and containment fails loud with `42883`, an
/// over-rejection of the operator's most idiomatic form.
///
/// The anchor is any expression [`container_operand_type`] can type, **not** just a
/// column: Postgres resolves the unknown from the *operator signature*, so it accepts
/// `'{"a":1}'::jsonb @> '{"a":1}'`, `(d -> 'x') @> '{"a":1}'`, and
/// `jsonb_build_object('k','v') @> '{"k":"v"}'` exactly as readily as the column
/// form. Anchoring on `Expr::Column` alone left every one of those raising `42883`
/// — including the mirror of the cast spelling that was the documented workaround.
///
/// Kept separate from [`coerce_comparison_operands`] rather than folded into
/// [`is_comparison_op`]: the rule differs (the target must be a *container* type,
/// and there is no unknown-vs-concrete-literal case, since two bare literals give
/// containment nothing to resolve against), and that predicate's own contract is
/// about comparison operators.
fn coerce_containment_operands(left: Expr, right: Expr, scope: &ColumnScope) -> (Expr, Expr) {
    if matches!(right, Expr::Literal(Literal::String(_)))
        && let Some(target) = container_operand_type(&left, scope)
    {
        return (left, cast_expr(right, target));
    }
    if matches!(left, Expr::Literal(Literal::String(_)))
        && let Some(target) = container_operand_type(&right, scope)
    {
        return (cast_expr(left, target), right);
    }
    (left, right)
}

/// The **container** `TypeOid` an expression is known to produce, or `None`.
///
/// The anchor an unknown-typed containment literal resolves against. Deliberately
/// narrower than [`Expr::infer_type`], which needs a `Schema` this pass does not
/// carry: only the forms whose container type is decidable from the `ColumnScope`
/// plus the node itself. A `None` here costs nothing but the coercion — the operand
/// gate then reports the unresolved literal, as Postgres reports an unresolvable
/// `unknown`.
fn container_operand_type(expr: &Expr, scope: &ColumnScope) -> Option<TypeOid> {
    let oid = match expr {
        Expr::Column { table, name } => scope.coercion_type(table.as_deref(), name)?,
        Expr::Cast { target_type, .. } => *target_type,
        Expr::CastTypmod { type_oid, .. } => *type_oid,
        // The jsonb producers: the two jsonb-returning operators (`->` and `#>` —
        // their `->>`/`#>>` siblings return text and are correctly not containers)
        // and the one jsonb constructor.
        Expr::BinaryOp {
            op: BinaryOp::JsonGet | BinaryOp::JsonPathGet,
            ..
        }
        | Expr::JsonbBuildObject(_) => TypeOid::Jsonb,
        // An alias is a naming wrapper, not a value change — see through it.
        Expr::Alias { expr, .. } => return container_operand_type(expr, scope),
        _ => return None,
    };
    oid.is_container().then_some(oid)
}

/// Whether a bare string literal is *coerced* to `target` in a comparison / `IN`
/// list / simple-`CASE` label — a **type-only** predicate that does not inspect the
/// literal's contents. Postgres resolves an unknown-typed literal to the other
/// operand's type and parses it at run time, erroring on a bad spelling
/// (`WHERE n = 'abc'` → `22P02`, `WHERE iv = '1 mon'` → the parsed interval); pglet
/// reproduces both by always inserting the coercing `Cast` for a coercible target and
/// letting the eval-time cast either succeed or fail loud.
///
/// Returns **false** only for `text`/`varchar` — a bare unknown literal is *already*
/// text there, so the cast would be an identity no-op. Every other target (including
/// `bytea`, `jsonb`, and the array types) needs a real cast: Postgres resolves the
/// unknown literal through that type's input function, so `bytea_col = '\x41'`,
/// `jsonb_col <> '{"k": 2}'`, and `text_arr = '{a}'` all *work* — and a malformed
/// spelling raises the input function's own error (`22P02` for `jsonb_col = 'not
/// json'`), never "operator does not exist". Leaving them out made the comparison
/// gate see `jsonb <> text` and reject a legal query.
const fn is_string_coercible_target(target: TypeOid) -> bool {
    !matches!(target, TypeOid::Text | TypeOid::Varchar)
}

/// The concrete `TypeOid` a **non-string** literal resolves to, for the
/// unknown-string-literal rule in [`coerce_comparison_operands`]. A bare string or
/// NULL is "unknown" (→ `None`) and adopts the other side's type instead.
///
/// An integer constant types as `int4` when it fits, else `int8` — matching Postgres
/// constant typing, so the coerced string's reported type and error message match
/// (`1 = 'abc'` → "integer", `9999999999 = 'abc'` → "bigint").
const fn concrete_literal_type(lit: &Literal) -> Option<TypeOid> {
    match lit {
        Literal::Bool(_) => Some(TypeOid::Bool),
        Literal::Int(n) => Some(int_literal_type(*n)),
        Literal::Float(_) => Some(TypeOid::Float8),
        Literal::Numeric(_) => Some(TypeOid::Numeric),
        Literal::String(_) | Literal::Null => None,
    }
}

/// The `TypeOid` an integer constant types as under Postgres constant typing: `int4`
/// when it fits `i32`, else `int8`. The single classifier every literal-type path
/// shares ([`concrete_literal_type`], [`concrete_branch_type`], [`literal_type`]) so a
/// small `Literal::Int` reports "integer" (never "bigint") in a coercion error.
const fn int_literal_type(n: i64) -> TypeOid {
    if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
        TypeOid::Int4
    } else {
        TypeOid::Int8
    }
}

/// Check if a type is numeric.
const fn is_numeric_type(t: TypeOid) -> bool {
    matches!(
        t,
        TypeOid::Int2
            | TypeOid::Int4
            | TypeOid::Int8
            | TypeOid::Float4
            | TypeOid::Float8
            | TypeOid::Numeric
    )
}

/// Build a column type map from table columns, each carrying its enum identity.
pub fn build_column_type_map(columns: &[crate::catalog::Column]) -> HashMap<String, ResolvedType> {
    columns
        .iter()
        .map(|c| {
            (
                c.name.clone(),
                ResolvedType::new(c.type_oid, c.enum_type.clone()),
            )
        })
        .collect()
}

/// The scope map for a **projected relation**'s descriptors — the `ColumnDesc` twin of
/// [`build_column_type_map`].
///
/// Both builders exist so the two shapes a scope relation comes in (a catalog table's
/// `Column`s, a plan node's output `ColumnDesc`s) read the type *and* the enum identity
/// off one field each, rather than one of the two silently dropping the identity.
#[must_use]
pub fn build_desc_type_map(columns: &[crate::types::ColumnDesc]) -> HashMap<String, ResolvedType> {
    columns
        .iter()
        .map(|c| (c.name.clone(), ResolvedType::of_column(c)))
        .collect()
}

/// The `column_name → ResolvedType` map for a single named table (empty if absent).
///
/// The per-table coercion map the single-table lowering paths (`lower_update` /
/// `lower_delete`, `ON CONFLICT`) build.
pub fn column_type_map_for(schema: &Schema, table: &str) -> HashMap<String, ResolvedType> {
    schema
        .get_table(table)
        .map(|t| build_column_type_map(&t.columns))
        .unwrap_or_default()
}

/// Refuse an assignment whose right-hand side cannot reach `target` by an implicit or
/// assignment cast.
///
/// Postgres's `transformAssignedExpr` (`42804`), run **before** [`coerce_to_type`] at
/// every DML site that stores a value into a column — and at `executor::ddl`'s
/// `cook_column_default`, which is `cookDefault`'s own position for the same check.
///
/// **The order is the whole point, and it is why this cannot live at the store
/// finalizer.** `coerce_to_type` wraps the right-hand side in an `Expr::Cast` to the
/// column's type, so the evaluator converts it under *explicit*-cast rules and
/// `executor::constraints`'s `coerce_column_value` — which does hold an
/// assignment-context refusal — never sees the source type at all. Measured on a fresh
/// database, that made `INSERT INTO tb (b) VALUES (1)` over `tb(b BOOL)` store `true`
/// and `UPDATE ti SET a = true` over `ti(a INT)` store `1`, both silently, where 18.6
/// answers `42804`. Nor can the finalizer be widened to close it: it reads a `Value`,
/// and a legal unknown literal and an illegal typed `text` arrive as the same
/// `Value::Text`. Nor can the evaluator's cast table be narrowed — `SELECT 1::bool` is
/// `t` on 18.6, and deleting the arm is green across the whole corpus and takes that
/// statement with it (see `coerce_column_value`'s own note).
///
/// **What is skipped is Postgres's `unknown`, and it is a syntactic property.** A bare
/// string constant carries no type until something gives it one, so `INSERT INTO t (i)
/// VALUES ('5')` and `… (b) VALUES ('t')` are ordinary SQL on both engines — while
/// `VALUES ('5'::text)` is `42804` there. Both spellings infer `text` here, so the
/// carve-out keys on the *expression shape* ([`unknown_assignment_source`]) and not on
/// [`Expr::infer_type`]. It is deliberately narrow, because Postgres's is: an unknown
/// resolves to `text` the moment it enters a construct, so `CASE WHEN true THEN 't'
/// ELSE 'f' END`, `COALESCE('5','6')` and `ARRAY['1','2']` are each `42804` into a
/// non-text column on 18.6 — measured, all three.
///
/// A right-hand side with no type of its own ([`AssignmentSource::Unknown`] /
/// [`AssignmentSource::Opaque`] — a bare constant, a `$N` awaiting its bind, an
/// expression this walker cannot type) is accepted, per this rulebook's contract. A `$N`
/// the client declared at `Parse` is not one of those: the parser reads it as a `Cast`
/// (`parser::parse_single_with_param_types`), which types it, so `INSERT INTO t (b)
/// VALUES ($1)` with `$1` declared `int4` over a `bool` `b` is refused `42804`, as on
/// 18.6 (measured, with the `UPDATE … SET b = $1` and `INSERT … SELECT $1` spellings).
///
/// The `INSERT … SELECT` source is judged by the same rule against a plan's columns
/// rather than one expression (`super::dml`'s `target_list_source_types`), because a
/// construct enclosing the source *resolves* an `unknown` that the target column would
/// otherwise have taken.
///
/// # Errors
///
/// [`Error::Type`](crate::Error::Type) (`42804`) from [`assignment_legality`].
pub fn check_assignment(
    expr: &Expr,
    column: &str,
    target: TypeOid,
    schema: &Schema,
    scope: TypeScope<'_>,
) -> Result<()> {
    match assignment_source_type(expr, schema, scope) {
        AssignmentSource::Typed(source) => assignment_legality(source, target, column),
        AssignmentSource::Unknown | AssignmentSource::Opaque => Ok(()),
    }
}

/// What a right-hand side carries into an assignment — the three-way answer
/// [`assignment_source_type`] gives, so a site that reaches [`assignment_legality`] by
/// another route cannot answer it differently.
///
/// **`Unknown` and `Opaque` are both accepted and are still not the same fact**, which
/// is why one `Option<TypeOid>` cannot serve: an enclosing construct *resolves*
/// Postgres's `unknown` to `text` (`INSERT INTO t (b) SELECT 't' ORDER BY 1` is `42804`
/// on 18.6 while the bare `SELECT 't'` is ordinary SQL), and resolves nothing at all
/// about an expression the walker simply could not type. Collapsing them either
/// re-opens that `42804` or invents one for a `$N` and for a column of a relation the
/// catalog lacks — the shape `Plan::output_columns` substitutes `text` for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AssignmentSource {
    /// A concrete type — gate this value against its target column.
    Typed(TypeOid),
    /// Postgres's `unknown`: a bare string constant or a bare `NULL`, which the target
    /// column resolves — unless something upstream resolves it to `text` first.
    Unknown,
    /// Nothing this walker can name: a `$N` awaiting its bind, an expression it cannot
    /// type, or one reaching a relation the catalog lacks. Never gated, and never
    /// resolved to `text` either.
    Opaque,
}

impl AssignmentSource {
    /// This source as an enclosing construct leaves it — Postgres's `unknown` resolved
    /// to `text`, everything else untouched.
    ///
    /// `addTargetToSortList` / `addTargetToGroupList` coerce an `unknown` target-list
    /// entry to `text` when a sort or group key names it, and `transformDistinctClause`
    /// does the same for every entry. An `Opaque` entry has no `unknown` to resolve, so
    /// it stays out of the gate: `INSERT INTO t (i) SELECT n FROM nosuchrel ORDER BY 1`
    /// is `42P01` on 18.6, not `42804`.
    ///
    /// **`text` is this rule's answer and not a set operation's**, which is worth
    /// stating because the two are neighbours in the table above and one sentence used
    /// to cover both. A set op unifies its two sides through `select_common_type`, so an
    /// `unknown` there resolves to *the other side's* type — measured on 18.6,
    /// `SELECT 5 UNION ALL SELECT '6'` is `integer`, not `text` — and to `text` only
    /// where both sides are `unknown`. `super::query`'s `build_set_op` does that
    /// resolution when it builds the node, so a set operation reaching
    /// [`super::dml`]'s `target_list_source_types` fallback is already typed there.
    pub(super) const fn resolved(self) -> Self {
        match self {
            Self::Unknown => Self::Typed(TypeOid::Text),
            other => other,
        }
    }
}

/// What a right-hand side assigns with — the one derivation of that question.
///
/// Three answers, in the order they are decided. A **bare constant** is Postgres's
/// `unknown`, read off the expression's shape ([`unknown_assignment_source`]) because
/// its inferred type is indistinguishable from a `'5'::text`'s. An expression reaching
/// a relation the catalog lacks is `Opaque`, because the statement's real error is that
/// relation's `42P01` and this gate must not pre-empt it — measured on `INSERT INTO t
/// (b) VALUES ((SELECT n FROM nosuchrel))` and its `UPDATE … SET` twin, both `42P01` on
/// 18.6. Everything else is whatever [`Expr::infer_type`] answers, `Opaque` when it
/// answers nothing — which is where an unbound `$N` lands.
pub(super) fn assignment_source_type(
    expr: &Expr,
    schema: &Schema,
    scope: TypeScope<'_>,
) -> AssignmentSource {
    if unknown_assignment_source(expr) {
        return AssignmentSource::Unknown;
    }
    if !expr_relations_resolvable(expr, schema) {
        return AssignmentSource::Opaque;
    }
    expr.infer_type(&PlannerScope::new(schema, scope))
        .map_or(AssignmentSource::Opaque, AssignmentSource::Typed)
}

/// Whether `expr` is Postgres's `unknown` — a **bare** string constant or a bare
/// `NULL`, seen through an `AS alias`.
///
/// These are the constants the target column resolves: `INSERT
/// INTO t (b) VALUES ('t')` and `… (i) VALUES ('5')` are ordinary SQL on both engines
/// while `VALUES ('5'::text)` is `42804` there. Both spellings infer `text` here, so
/// the distinction has to be read off the expression rather than off its inferred type.
/// Only the alias is transparent, and that is measured: `SELECT 't' AS z` assigns into
/// a `BOOL` column on 18.6 exactly as `SELECT 't'` does, while every construct that
/// *resolves* an unknown is not — `CASE WHEN true THEN 't' ELSE 'f' END`,
/// `COALESCE('5','6')`, `ARRAY['1','2']` and `SELECT 't' ORDER BY 1` are each `42804`
/// there. Listing `NULL` is not the same as skipping a *typed* null: `SET b =
/// NULL::int` is `42804` on 18.6 and is refused here, the cast being what carries the
/// type — and a bare `NULL` really is an unknown that a sort key resolves, measured:
/// `INSERT INTO t (b) SELECT NULL ORDER BY 1` is `42804` on 18.6 where the bare
/// `SELECT NULL` is accepted.
///
/// **A `$N` is deliberately not here**, and the reason is worth stating because the arm
/// looks like it belongs: an unbound parameter must reach
/// [`AssignmentSource::Opaque`] — never `Unknown`, or a construct that resolves an
/// unknown would resolve it to `text` and refuse an assignment 18.6 accepts — and it
/// does, through [`Expr::infer_type`], whose `Parameter` arm answers `None`
/// unconditionally. An arm here answering `Opaque` is therefore behaviourally dead;
/// measured as that mutation, deleting one reddens nothing (1791/0/8, 1434/0). The
/// dependency on `infer_type`'s arm is real and is named rather than duplicated.
fn unknown_assignment_source(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(Literal::String(_) | Literal::Null) => true,
        Expr::Alias { expr, .. } => unknown_assignment_source(expr),
        _ => false,
    }
}

/// Coerce an expression to a target type by wrapping in Cast if needed.
///
/// Returns the expression unchanged if it's already the right type or is a column.
// arms grouped by reason (column / already-cast / NULL) for documentation; several share the `expr` body but are kept separate intentionally
#[allow(clippy::match_same_arms)]
pub fn coerce_to_type(expr: Expr, target_type: TypeOid) -> Expr {
    match &expr {
        // Don't wrap columns - they'll be coerced at evaluation if needed
        Expr::Column { .. } => expr,
        // Don't double-wrap — a cast (with or without a typmod) already targets a type.
        Expr::Cast { .. } | Expr::CastTypmod { .. } => expr,
        // NULL doesn't need coercion
        Expr::Literal(Literal::Null) => expr,
        // Check if literal needs coercion
        Expr::Literal(lit) => {
            let lit_type = literal_type(lit);
            if lit_type == target_type || types_compatible(lit_type, target_type) {
                expr
            } else {
                Expr::Cast {
                    expr: Box::new(expr),
                    target_type,
                }
            }
        }
        // Other expressions - wrap if types might not match
        _ => Expr::Cast {
            expr: Box::new(expr),
            target_type,
        },
    }
}

/// Get the inferred type of a literal.
// arms are kept one-per-variant for documentation; Null and String both infer Text but are distinct cases
#[allow(clippy::match_same_arms)]
const fn literal_type(lit: &Literal) -> TypeOid {
    match lit {
        Literal::Null => TypeOid::Text,
        Literal::Bool(_) => TypeOid::Bool,
        // `int4` when it fits (PG constant typing); a numeric target treats int4/int8
        // as compatible, so this stays behaviorally a no-op for `coerce_to_type`.
        Literal::Int(n) => int_literal_type(*n),
        Literal::Float(_) => TypeOid::Float8,
        Literal::String(_) => TypeOid::Text,
        Literal::Numeric(_) => TypeOid::Numeric,
    }
}

/// Check if two types are compatible without explicit cast.
const fn types_compatible(from: TypeOid, to: TypeOid) -> bool {
    // Same type (compare discriminants; `==` on the enum is not const)
    if from as u32 == to as u32 {
        return true;
    }
    // Numeric types are compatible with each other
    if is_numeric_type(from) && is_numeric_type(to) {
        return true;
    }
    // Text and varchar are compatible
    if matches!(
        (from, to),
        (TypeOid::Text, TypeOid::Varchar) | (TypeOid::Varchar, TypeOid::Text)
    ) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    /// [`resolve_expr_types`], which only a too-deep tree fails.
    fn resolve_ok(expr: Expr, scope: &ColumnScope) -> Expr {
        super::resolve_expr_types(expr, scope).expect("resolves")
    }

    use super::{ColumnScope, resolve_expr_types_no_columns, validate_const_casts};
    use crate::catalog::Schema;
    use crate::planner::{BinaryOp, Expr, Literal, ResolvedType};
    use crate::types::TypeOid;

    /// A one-relation scope named `t` over `columns`.
    fn scope_of(columns: &[(&str, TypeOid)]) -> ColumnScope {
        ColumnScope::single(
            "t".to_string(),
            columns
                .iter()
                .map(|(n, t)| ((*n).to_string(), ResolvedType::plain(*t)))
                .collect(),
        )
    }

    fn int_col_types() -> ColumnScope {
        scope_of(&[("i", TypeOid::Int4), ("label", TypeOid::Text)])
    }

    fn col(name: &str) -> Expr {
        Expr::Column {
            table: None,
            name: name.to_string(),
        }
    }

    fn str_lit(s: &str) -> Expr {
        Expr::Literal(Literal::String(s.to_string()))
    }

    fn cast_int(inner: Expr) -> Expr {
        Expr::Cast {
            expr: Box::new(inner),
            target_type: TypeOid::Int4,
        }
    }

    fn aggregate(func: crate::planner::AggregateFunc, arg: Expr) -> Expr {
        Expr::Aggregate {
            func,
            arg: Box::new(arg),
            distinct: false,
            certainty: crate::planner::ArgCertainty::Undecided,
        }
    }

    /// The producer of an aggregate's certainty never asks the type of an aggregate it
    /// has not decided yet: an argument holding one — `avg(avg(i))`, `sum(i + max(i))` —
    /// reads the inner aggregate before this pass reaches it, and is recorded not
    /// certain without asking (the gate after the pass refuses it `42803`). Reading it
    /// asked `aggregate_type_of` of an undecided verdict, which debug builds refuse.
    #[test]
    fn an_argument_holding_an_aggregate_is_not_certain_and_asks_no_reader() {
        use crate::planner::{AggregateFunc, ArgCertainty};
        let scope = int_col_types();
        for outer in [AggregateFunc::Avg, AggregateFunc::Sum] {
            for arg in [
                aggregate(AggregateFunc::Avg, col("i")),
                Expr::BinaryOp {
                    left: Box::new(col("i")),
                    op: BinaryOp::Add,
                    right: Box::new(aggregate(AggregateFunc::Max, col("i"))),
                },
                Expr::Coalesce(vec![
                    aggregate(AggregateFunc::Count, col("i")),
                    Expr::Literal(Literal::Int(0)),
                ]),
            ] {
                assert_eq!(
                    super::aggregate_arg_certainty(outer, &arg, &scope, &super::no_subquery_type),
                    ArgCertainty::NotCertainInteger,
                    "{outer:?}({arg:?})"
                );
            }
        }
        // the accept side: a certain integer argument with no aggregate in it
        assert_eq!(
            super::aggregate_arg_certainty(
                AggregateFunc::Avg,
                &col("i"),
                &scope,
                &super::no_subquery_type
            ),
            ArgCertainty::CertainInteger(TypeOid::Int4)
        );
    }

    #[test]
    fn in_list_coerces_string_elements_to_column_type() {
        // `i IN ('5', '7')` → the string literals become casts to the column's int
        // type (PG treats IN like a scalar `=`).
        let expr = Expr::InList {
            expr: Box::new(col("i")),
            list: vec![str_lit("5"), str_lit("7")],
            negated: false,
        };
        let Expr::InList { list, .. } = resolve_ok(expr, &int_col_types()) else {
            panic!("expected InList");
        };
        assert_eq!(list, vec![cast_int(str_lit("5")), cast_int(str_lit("7"))]);
    }

    #[test]
    fn coalesce_unifies_string_to_concrete_sibling() {
        // `COALESCE(i, '99')` → '99' coerces to the int sibling; the same-type
        // `COALESCE(label, 'x')` is left untouched (a text target adds no cast).
        let expr = Expr::Coalesce(vec![col("i"), str_lit("99")]);
        assert_eq!(
            resolve_ok(expr, &int_col_types()),
            Expr::Coalesce(vec![col("i"), cast_int(str_lit("99"))]),
        );
        let same_type = Expr::Coalesce(vec![col("label"), str_lit("x")]);
        assert_eq!(resolve_ok(same_type.clone(), &int_col_types()), same_type,);
    }

    #[test]
    fn case_unifies_result_branches_and_descends_into_conditions() {
        // `CASE WHEN i IN ('5') THEN i ELSE '0' END`: the nested `IN ('5')` coerces
        // (walker descent), and the ELSE '0' unifies to the int THEN branch.
        let expr = Expr::Case {
            operand: None,
            when_clauses: vec![(
                Expr::InList {
                    expr: Box::new(col("i")),
                    list: vec![str_lit("5")],
                    negated: false,
                },
                col("i"),
            )],
            else_result: Some(Box::new(str_lit("0"))),
        };
        let Expr::Case {
            when_clauses,
            else_result,
            ..
        } = resolve_ok(expr, &int_col_types())
        else {
            panic!("expected Case");
        };
        // Nested IN coerced.
        let Expr::InList { list, .. } = &when_clauses[0].0 else {
            panic!("expected InList condition");
        };
        assert_eq!(list, &vec![cast_int(str_lit("5"))]);
        // ELSE unified to int.
        assert_eq!(else_result, Some(Box::new(cast_int(str_lit("0")))));
    }

    #[test]
    fn nested_comparison_inside_coalesce_is_coerced() {
        // The walker descends into COALESCE args, so a nested comparison still gets
        // its coercing cast.
        let expr = Expr::Coalesce(vec![Expr::BinaryOp {
            left: Box::new(col("i")),
            op: BinaryOp::Eq,
            right: Box::new(str_lit("5")),
        }]);
        let Expr::Coalesce(args) = resolve_ok(expr, &int_col_types()) else {
            panic!("expected Coalesce");
        };
        assert_eq!(
            args[0],
            Expr::BinaryOp {
                left: Box::new(col("i")),
                op: BinaryOp::Eq,
                right: Box::new(cast_int(str_lit("5"))),
            },
        );
    }

    #[test]
    fn comparison_coerces_unparseable_literal_unconditionally() {
        // `i = 'abc'` now casts the *unparseable* literal to the column's int type —
        // the coercion is type-only, so the eval-time cast fails loud (PG's `22P02`)
        // instead of a silent 0-row match. (Previously the parse-gate skipped the cast
        // and the `Text` compared as false against every int row.)
        let expr = Expr::BinaryOp {
            left: Box::new(col("i")),
            op: BinaryOp::Eq,
            right: Box::new(str_lit("abc")),
        };
        let Expr::BinaryOp { right, .. } = resolve_ok(expr, &int_col_types()) else {
            panic!("expected BinaryOp");
        };
        assert_eq!(*right, cast_int(str_lit("abc")));
    }

    #[test]
    fn case_small_int_branch_types_as_int4_not_int8() {
        // `CASE WHEN true THEN 1 ELSE 'x' END`: the THEN `1` types as int4 (PG constant
        // typing), so the unparseable ELSE 'x' casts to int4 — its eval error reads
        // "integer", not "bigint". A branch value overflowing int4 types as int8.
        let empty = ColumnScope::default();
        let small = Expr::Case {
            operand: None,
            when_clauses: vec![(
                Expr::Literal(Literal::Bool(true)),
                Expr::Literal(Literal::Int(1)),
            )],
            else_result: Some(Box::new(str_lit("x"))),
        };
        let Expr::Case { else_result, .. } = resolve_ok(small, &empty) else {
            panic!("expected Case");
        };
        assert_eq!(
            else_result,
            Some(Box::new(Expr::Cast {
                expr: Box::new(str_lit("x")),
                target_type: TypeOid::Int4,
            })),
        );

        let big = Expr::Case {
            operand: None,
            when_clauses: vec![(
                Expr::Literal(Literal::Bool(true)),
                Expr::Literal(Literal::Int(9_999_999_999)),
            )],
            else_result: Some(Box::new(str_lit("x"))),
        };
        let Expr::Case { else_result, .. } = resolve_ok(big, &empty) else {
            panic!("expected Case");
        };
        assert_eq!(
            else_result,
            Some(Box::new(Expr::Cast {
                expr: Box::new(str_lit("x")),
                target_type: TypeOid::Int8,
            })),
        );
    }

    #[test]
    fn interval_column_comparison_coerces_string_literal() {
        // An INTERVAL column vs a bare string literal was never coerced before (no
        // `Interval` arm in the gate), so `iv > '15 days'` text-compared. Now the
        // literal casts to interval and compares as an interval.
        let m = scope_of(&[("iv", TypeOid::Interval)]);
        let expr = Expr::BinaryOp {
            left: Box::new(col("iv")),
            op: BinaryOp::Gt,
            right: Box::new(str_lit("15 days")),
        };
        let Expr::BinaryOp { right, .. } = resolve_ok(expr, &m) else {
            panic!("expected BinaryOp");
        };
        assert_eq!(
            *right,
            Expr::Cast {
                expr: Box::new(str_lit("15 days")),
                target_type: TypeOid::Interval,
            }
        );
    }

    #[test]
    fn literal_vs_literal_string_adopts_concrete_type() {
        let empty = ColumnScope::default();
        // no-FROM `true = 'yes'`: the unknown string adopts the bool sibling's type; it
        // parses, so the checked path returns Ok.
        let bool_eq = Expr::BinaryOp {
            left: Box::new(Expr::Literal(Literal::Bool(true))),
            op: BinaryOp::Eq,
            right: Box::new(str_lit("yes")),
        };
        let Ok(Expr::BinaryOp { right, .. }) =
            resolve_expr_types_no_columns(bool_eq, &Schema::new())
        else {
            panic!("expected Ok(BinaryOp)");
        };
        assert_eq!(
            *right,
            Expr::Cast {
                expr: Box::new(str_lit("yes")),
                target_type: TypeOid::Bool,
            }
        );

        // `'abc' = 1`: the small int literal types as **int4** (PG constant typing), so
        // the string adopts int4 — inspect the target via the pure resolver...
        let str_eq_int = Expr::BinaryOp {
            left: Box::new(str_lit("abc")),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Literal::Int(1))),
        };
        let Expr::BinaryOp { left, .. } = resolve_ok(str_eq_int.clone(), &empty) else {
            panic!("expected BinaryOp");
        };
        assert_eq!(
            *left,
            Expr::Cast {
                expr: Box::new(str_lit("abc")),
                target_type: TypeOid::Int4,
            }
        );
        // ...and the checked path fails loud (22P02), like PG.
        assert_eq!(
            resolve_expr_types_no_columns(str_eq_int, &Schema::new())
                .expect_err("'abc' = 1 must fail loud")
                .sqlstate(),
            "22P02"
        );

        // `'a' = 'b'`: two unknown strings have no concrete side → stay text (no cast),
        // and the checked path returns Ok (no casts to validate).
        let both_str = Expr::BinaryOp {
            left: Box::new(str_lit("a")),
            op: BinaryOp::Eq,
            right: Box::new(str_lit("b")),
        };
        assert_eq!(
            resolve_expr_types_no_columns(both_str.clone(), &Schema::new()).unwrap(),
            both_str,
        );
    }

    #[test]
    fn coercion_never_wraps_a_bare_column() {
        // The load-bearing invariant of the pass: a bare `Expr::Column` is returned
        // **as itself**, never wrapped in a `Cast`. Several consumers silently depend
        // on it and degrade quietly if it breaks — `try_order_by_index_pushdown` gates
        // ORDER BY index elision on the key being a bare column (a wrapped one just
        // stops eliding, a perf regression invisible at the SQL level), and
        // `rewrite_scan_for_order` / `build_sort`'s `collect_referenced_columns` read
        // column identity out of the same shape.
        //
        // Checked both bare and qualified, and in the two positions that *do* insert a
        // cast on the other operand — so the assertion is that the column side is
        // untouched while the coercion demonstrably fired.
        let m = int_col_types();
        for column in [
            col("i"),
            Expr::Column {
                table: Some("t".to_string()),
                name: "i".to_string(),
            },
        ] {
            assert_eq!(
                resolve_ok(column.clone(), &m),
                column,
                "a bare column must resolve to itself",
            );

            let cmp = Expr::BinaryOp {
                left: Box::new(column.clone()),
                op: BinaryOp::Eq,
                right: Box::new(str_lit("5")),
            };
            let Expr::BinaryOp { left, right, .. } = resolve_ok(cmp, &m) else {
                panic!("expected BinaryOp");
            };
            assert_eq!(*left, column, "the column operand must be untouched");
            assert_eq!(*right, cast_int(str_lit("5")), "the literal must be cast");

            let in_list = Expr::InList {
                expr: Box::new(column.clone()),
                list: vec![str_lit("5")],
                negated: false,
            };
            let Expr::InList { expr, list, .. } = resolve_ok(in_list, &m) else {
                panic!("expected InList");
            };
            assert_eq!(*expr, column, "the IN subject column must be untouched");
            assert_eq!(list, vec![cast_int(str_lit("5"))]);
        }
    }

    /// A one-relation scope named `name` over `columns`.
    fn named_scope(name: &str, columns: &[(&str, TypeOid)]) -> ColumnScope {
        ColumnScope::single(
            name.to_string(),
            columns
                .iter()
                .map(|(n, t)| ((*n).to_string(), ResolvedType::plain(*t)))
                .collect(),
        )
    }

    #[test]
    fn chained_scope_resolves_the_innermost_level_that_claims_a_name() {
        // Level 0 (`inner`) carries `a`; level 1 (`outer`) carries `a` **and** `b`.
        // Postgres's `colNameToVar` searches a level in full and walks outward only on
        // zero matches, so `a` is the inner `int4` and `b` falls through to the outer.
        let chained = named_scope("inner", &[("a", TypeOid::Int4)]).nested_in(&named_scope(
            "outer",
            &[("a", TypeOid::Uuid), ("b", TypeOid::Uuid)],
        ));
        assert_eq!(chained.coercion_type(None, "a"), Some(TypeOid::Int4));
        assert_eq!(chained.coercion_type(None, "b"), Some(TypeOid::Uuid));
        assert_eq!(
            chained.resolve(None, "a").map(|t| t.oid),
            Some(TypeOid::Int4)
        );
        // A qualifier resolves against the relation it names, at whichever level.
        assert_eq!(
            chained.qualified("outer", "a").map(|t| t.oid),
            Some(TypeOid::Uuid)
        );
        assert_eq!(
            chained.qualified("inner", "a").map(|t| t.oid),
            Some(TypeOid::Int4)
        );
        // A name no level carries stays unresolved — the walk running out of levels
        // must not fall back to anything.
        assert_eq!(chained.coercion_type(None, "missing"), None);
        assert_eq!(chained.qualified("nowhere", "a").map(|t| t.oid), None);
    }

    #[test]
    fn a_qualifier_stops_at_the_level_that_names_it() {
        // Both levels have a relation spelled `r`; the inner one lacks `x`. Postgres
        // finds the range-table entry level by level and then looks the column up in
        // *that* entry alone, so this is `42703` on the inner `r` — never a silent
        // rebinding to the outer relation's `x`.
        let chained = named_scope("r", &[("y", TypeOid::Text)])
            .nested_in(&named_scope("r", &[("x", TypeOid::Uuid)]));
        assert_eq!(chained.qualified("r", "x").map(|t| t.oid), None);
        assert_eq!(
            chained.qualified("r", "y").map(|t| t.oid),
            Some(TypeOid::Text)
        );
    }

    #[test]
    fn an_uncertain_level_stops_the_walk_outward() {
        // A level holding a relation the scope builder could not enumerate must not let
        // a name escape to an enclosing level that happens to share it — that is how a
        // chained resolution would turn a safe over-rejection into a wrong cast.
        let mut inner = named_scope("inner", &[("a", TypeOid::Int4)]);
        inner.push_opaque_relation();
        let chained = inner.nested_in(&named_scope("outer", &[("b", TypeOid::Uuid)]));
        // The uncertain level's own columns still resolve...
        assert_eq!(chained.coercion_type(None, "a"), Some(TypeOid::Int4));
        // ...but nothing escapes past it, bare or qualified.
        assert_eq!(chained.coercion_type(None, "b"), None);
        assert_eq!(chained.qualified("outer", "b"), None);
        // With every relation enumerated, the same chain resolves the enclosing name.
        let open = named_scope("inner", &[("a", TypeOid::Int4)])
            .nested_in(&named_scope("outer", &[("b", TypeOid::Uuid)]));
        assert_eq!(open.coercion_type(None, "b"), Some(TypeOid::Uuid));
    }

    /// A **named** relation whose columns are a subset stops the walk exactly as an
    /// unnamed one does, and that is the widening the per-relation facts buy: a catalog
    /// view is a relation the scope can name and cannot enumerate, which the per-level
    /// `sealed` bool had no way to say.
    #[test]
    fn a_partial_relation_answers_its_qualifier_and_still_stops_the_walk() {
        let mut inner = ColumnScope::default();
        inner.push_partial_relation(
            "v".to_string(),
            std::iter::once(("a".to_string(), ResolvedType::plain(TypeOid::Int4))).collect(),
        );
        let chained = inner.nested_in(&named_scope("outer", &[("b", TypeOid::Uuid)]));
        // Its own column resolves, qualified and bare.
        assert_eq!(
            chained.qualified("v", "a").map(|t| t.oid),
            Some(TypeOid::Int4)
        );
        assert_eq!(chained.coercion_type(None, "a"), Some(TypeOid::Int4));
        // A column it does not list is *unknown*, not absent — so nothing escapes.
        assert_eq!(chained.qualified("v", "nosuch").map(|t| t.oid), None);
        assert_eq!(chained.coercion_type(None, "b"), None);
    }

    #[test]
    fn nesting_composes_across_three_levels() {
        // `nested_in` shifts an already-chained scope as a whole, so a two-deep nest
        // sees its own FROM, then the middle query's, then the outermost's.
        let chain = named_scope("l1", &[("b", TypeOid::Text)])
            .nested_in(&named_scope("l2", &[("c", TypeOid::Uuid)]));
        let chain = named_scope("l0", &[("a", TypeOid::Int4)]).nested_in(&chain);
        assert_eq!(chain.coercion_type(None, "a"), Some(TypeOid::Int4));
        assert_eq!(chain.coercion_type(None, "b"), Some(TypeOid::Text));
        assert_eq!(chain.coercion_type(None, "c"), Some(TypeOid::Uuid));
        assert_eq!(
            chain.qualified("l2", "c").map(|t| t.oid),
            Some(TypeOid::Uuid)
        );
    }

    #[test]
    fn level_zero_only_helpers_ignore_enclosing_relations() {
        // `contains_column` and friends answer structural questions about *this*
        // query's own relations (which output column an ORDER BY key names, which
        // relation an unqualified reference belongs to). An enclosing query's
        // relations are not candidates for any of them.
        let chained = named_scope("inner", &[("a", TypeOid::Int4)])
            .nested_in(&named_scope("outer", &[("b", TypeOid::Uuid)]));
        assert!(chained.contains_column("a"));
        assert!(!chained.contains_column("b"));
        assert!(chained.contains_column_ref(Some("inner"), "a"));
        assert!(!chained.contains_column_ref(Some("outer"), "b"));
        assert_eq!(chained.sole_relation_with("a"), Some("inner"));
        assert_eq!(chained.sole_relation_with("b"), None);
    }

    #[test]
    fn an_untyped_column_branch_defers_the_whole_unification() {
        // `COALESCE(<correlated float8>, i, '1.5')`: with the correlated column
        // untypeable here, picking `int4` from the sibling would narrow it and cast
        // the literal to int — permanently, since the enclosing level's pass would
        // then see a `Cast`. Declining leaves every branch untouched.
        let correlated = Expr::Column {
            table: Some("outer".to_string()),
            name: "amt".to_string(),
        };
        let expr = Expr::Coalesce(vec![correlated, col("i"), str_lit("1.5")]);
        assert_eq!(resolve_ok(expr.clone(), &int_col_types()), expr);
        // With the correlated relation in scope, the same expression unifies on its
        // real type.
        let chained = int_col_types().nested_in(&named_scope("outer", &[("amt", TypeOid::Float8)]));
        let Expr::Coalesce(args) = resolve_ok(expr, &chained) else {
            panic!("expected Coalesce");
        };
        assert_eq!(
            args[2],
            Expr::Cast {
                expr: Box::new(str_lit("1.5")),
                target_type: TypeOid::Float8,
            },
        );
    }

    #[test]
    fn validate_const_casts_errors_on_unparseable_regardless_of_position() {
        // A resolved `i IN (42, 'abc')`: the bad element is *second*, but validation
        // walks the whole tree and errors (order-independent) — matching PG's parse-time
        // constant folding, not the eval-time short-circuit that returns on the match.
        let in_list = Expr::InList {
            expr: Box::new(col("i")),
            list: vec![Expr::Literal(Literal::Int(42)), cast_int(str_lit("abc"))],
            negated: false,
        };
        assert_eq!(
            validate_const_casts(&in_list)
                .expect_err("bad IN element must fail loud")
                .sqlstate(),
            "22P02"
        );
        // A list of parseable constant casts validates clean.
        let ok_list = Expr::InList {
            expr: Box::new(col("i")),
            list: vec![cast_int(str_lit("5")), cast_int(str_lit("7"))],
            negated: false,
        };
        assert!(validate_const_casts(&ok_list).is_ok());

        // An untaken CASE branch (`CASE WHEN <cond> THEN 1 ELSE 'x'::int END`) is a
        // constant cast, so it fails loud at plan time regardless of the branch taken.
        let case = Expr::Case {
            operand: None,
            when_clauses: vec![(col("i"), Expr::Literal(Literal::Int(1)))],
            else_result: Some(Box::new(cast_int(str_lit("x")))),
        };
        assert_eq!(
            validate_const_casts(&case)
                .expect_err("bad untaken branch must fail loud")
                .sqlstate(),
            "22P02"
        );
    }

    /// `agg op lit`, or `lit op agg` when `literal_left`.
    fn compare(agg: Expr, op: BinaryOp, other: Expr, literal_left: bool) -> Expr {
        let (left, right) = if literal_left {
            (other, agg)
        } else {
            (agg, other)
        };
        Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    /// A written string literal compared with an aggregate whose type is certain is read
    /// at that type, on either side, under every comparison operator — and the rule is
    /// keyed on the literal's node kind and the operator, so a `$N`, a `NULL`, a `text`
    /// result and a non-comparison operator plan exactly as before. Those four are
    /// pinned here because the answers cannot tell them apart: a parameter or a `NULL`
    /// cast to the type its inference already gives answers the same, and so does a
    /// literal cast to `text`.
    #[test]
    fn a_literal_compared_with_a_certain_aggregate_is_read_at_its_type() {
        use crate::planner::AggregateFunc;
        let scope = int_col_types();
        for (func, ty) in [
            (AggregateFunc::Count, TypeOid::Int8),
            (AggregateFunc::Max, TypeOid::Int4),
            (AggregateFunc::Min, TypeOid::Int4),
            (AggregateFunc::Sum, TypeOid::Int8),
            (AggregateFunc::Avg, TypeOid::Numeric),
        ] {
            for op in [
                BinaryOp::Eq,
                BinaryOp::Ne,
                BinaryOp::Lt,
                BinaryOp::Le,
                BinaryOp::Gt,
                BinaryOp::Ge,
                BinaryOp::IsDistinctFrom,
                BinaryOp::IsNotDistinctFrom,
            ] {
                for literal_left in [false, true] {
                    let Expr::BinaryOp { left, right, .. } = resolve_ok(
                        compare(aggregate(func, col("i")), op, str_lit("1"), literal_left),
                        &scope,
                    ) else {
                        panic!("expected a comparison");
                    };
                    let literal = if literal_left { left } else { right };
                    assert_eq!(
                        *literal,
                        Expr::Cast {
                            expr: Box::new(str_lit("1")),
                            target_type: ty,
                        },
                        "{func:?} {op:?}, literal on the left: {literal_left}"
                    );
                }
            }
        }
        let max_i = || aggregate(AggregateFunc::Max, col("i"));
        let unchanged = |other: Expr, op: BinaryOp, agg: Expr| {
            let Expr::BinaryOp { right, .. } =
                resolve_ok(compare(agg, op, other.clone(), false), &scope)
            else {
                panic!("expected a binary operator");
            };
            assert_eq!(*right, other, "{op:?}");
        };
        unchanged(Expr::Parameter { index: 1 }, BinaryOp::Eq, max_i());
        unchanged(Expr::Literal(Literal::Null), BinaryOp::Eq, max_i());
        unchanged(
            str_lit("b"),
            BinaryOp::Eq,
            aggregate(AggregateFunc::Max, col("label")),
        );
        unchanged(str_lit("1"), BinaryOp::Concat, max_i());
        unchanged(str_lit("1"), BinaryOp::Add, max_i());
    }

    /// The correlated re-run resolves a level's expressions a second time, from the
    /// first run's output, under a scope chaining the enclosing levels; it converts no
    /// literal beside an aggregate, so the first run's decision is the one that stands.
    /// The aggregate here is certain at its own level, which is what isolates the
    /// re-run's refusal from the reader's.
    #[test]
    fn the_correlated_rerun_reads_no_literal_at_an_aggregate_type() {
        use crate::planner::AggregateFunc;
        let chained = int_col_types().nested_in(&named_scope("o", &[("y", TypeOid::Float8)]));
        let Expr::BinaryOp { right, .. } = resolve_ok(
            compare(
                aggregate(AggregateFunc::Max, col("i")),
                BinaryOp::Eq,
                str_lit("4"),
                false,
            ),
            &chained,
        ) else {
            panic!("expected a comparison");
        };
        assert_eq!(*right, str_lit("4"));
    }

    /// `count` reads no argument type, so binding — not the type reader — is what keeps
    /// a literal beside it `text` where its argument does not bind at the aggregate's own
    /// level: an enclosing query's column (`EDGES.md` row 303's aggregate), a column or
    /// relation nothing carries, a bare name two relations claim — `max` too, the two
    /// claimants agreeing on the type — and a sub-plan. A qualified reference to one of
    /// the claimants, and `count(*)`, bind.
    #[test]
    fn a_literal_beside_an_aggregate_whose_argument_does_not_bind_stays_text() {
        use crate::planner::AggregateFunc;
        let mut two = int_col_types();
        two.push_base_relation(
            "u".to_string(),
            std::iter::once(("i".to_string(), ResolvedType::plain(TypeOid::Int4))).collect(),
        );
        let qualified = |table: &str, name: &str| Expr::Column {
            table: Some(table.to_string()),
            name: name.to_string(),
        };
        let literal_after = |agg: Expr, scope: &ColumnScope| {
            let Expr::BinaryOp { right, .. } =
                resolve_ok(compare(agg, BinaryOp::Gt, str_lit("x"), false), scope)
            else {
                panic!("expected a comparison");
            };
            *right
        };
        let count = |arg| aggregate(AggregateFunc::Count, arg);
        for (agg, scope) in [
            (count(qualified("o", "z")), &int_col_types()),
            (count(col("nosuch")), &int_col_types()),
            (count(qualified("t", "nosuch")), &int_col_types()),
            (
                count(Expr::BinaryOp {
                    left: Box::new(col("i")),
                    op: BinaryOp::Add,
                    right: Box::new(col("nosuch")),
                }),
                &int_col_types(),
            ),
            (count(col("i")), &two),
            (aggregate(AggregateFunc::Max, col("i")), &two),
        ] {
            assert_eq!(literal_after(agg.clone(), scope), str_lit("x"), "{agg:?}");
        }
        for agg in [
            count(qualified("u", "i")),
            count(col("*")),
            aggregate(AggregateFunc::Max, qualified("t", "i")),
        ] {
            assert!(
                matches!(literal_after(agg.clone(), &two), Expr::Cast { .. }),
                "{agg:?}"
            );
        }
    }

    /// The pair is read before the pass resolves it: a fold the pass unifies at the
    /// written reach from a column whose type is a guess comes back with a cast over that
    /// column, which the computed reach then reads as certain — so `max(COALESCE(x, 1.5e0))`
    /// over such an `x` keeps its literal `text`, where the resolved form would read a
    /// certain `double precision`.
    #[test]
    fn a_literal_beside_an_aggregate_is_decided_on_the_written_argument() {
        use crate::planner::AggregateFunc;
        let mut scope = ColumnScope::default();
        scope.push_relation(
            "v".to_string(),
            std::iter::once(("x".to_string(), ResolvedType::plain(TypeOid::Int4))).collect(),
        );
        let fold = Expr::Coalesce(vec![col("x"), Expr::Literal(Literal::Float(1.5))]);
        let Expr::BinaryOp { left, right, .. } = resolve_ok(
            compare(
                aggregate(AggregateFunc::Max, fold),
                BinaryOp::Gt,
                str_lit("1"),
                false,
            ),
            &scope,
        ) else {
            panic!("expected a comparison");
        };
        assert_eq!(*right, str_lit("1"));
        // The pass did write the written-reach cast the resolved form would read.
        let Expr::Aggregate { arg, .. } = *left else {
            panic!("expected the aggregate");
        };
        assert!(
            matches!(&*arg, Expr::Coalesce(branches) if matches!(branches[0], Expr::Cast { .. })),
            "{arg:?}"
        );
    }
}
