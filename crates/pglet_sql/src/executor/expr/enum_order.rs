//! Enum identity and ordering: the enum-leaf walk and the operand predicates around it.

use super::columns::ColumnResolver;
use super::env::{EvalEnv, OuterSlot};
use crate::Error;
use crate::catalog::Schema;
use crate::planner::{AggregateFunc, Expr, Plan, VariadicFunc};
use crate::types::Value;

/// Whether an operand's **value** could be an enum label, and so whether the walk that
/// resolves a member order has an answer any comparison can consume.
///
/// An enum value is a `Value::Text`, and [`values_compare_ordered`](super::compare::values_compare_ordered)'s enum arm reads the
/// order only when the values it is comparing are both `Value::Text` — so this is that
/// arm's own precondition, asked *before* [`operands_enum_order`] rather than after.
/// The walk is not free: its subquery leaf costs a `Plan::output_columns` per
/// evaluation, which on a comparison operand is per row. Measured on a release server
/// over 20 000 rows, medians of three, against the same build with the walk's `Subquery`
/// arm cut out: `i > (SELECT i FROM u2 WHERE id = 1)` — an `int` comparison, with no
/// enum anywhere in the statement — took 38.0 ms unguarded against 29.9 ms guarded, and
/// `i > 5`, which walks no subquery at all, 5.21 ms against 3.82 ms; both for an answer
/// `values_compare_ordered` would then throw away.
///
/// **An array's elements are that same arm one level down**, so an array operand is
/// admitted when any element is: `values_compare_ordered` compares two arrays
/// element-wise, and its enum arm's precondition is two `Value::Text` operands — so an
/// element position that reaches the arm holds a `Value::Text` on each side, which forces
/// this predicate true on each side. The pair gate is therefore a superset of the arm by
/// construction, whatever decides the positions that never reach it (the NULL rule, the
/// length comparison, the mixed-pair fallback). An array holding no `Value::Text`
/// anywhere reaches the arm nowhere.
///
/// Each caller asks it at the width its own comparator has: both sides for a binary
/// comparison, both for `ANY`/`ALL` (the scalar side against the array's elements), and
/// any argument for `greatest`/`least` (which compares its arguments pairwise). A
/// superset is safe here and an undercount is not — the guard may only skip a walk whose
/// answer is unreadable. The superset matters here rather than being a formality: an
/// array carrying an enum reaches a comparison through positions no list of spellings
/// closes — `WHERE q.a < ARRAY['ok'::mood]` over a derived `SELECT ARRAY[m] AS a`
/// answers by member position on both engines, measured — so the gate asks the *value*.
///
/// **That recursion has a cost of its own, on the same axis the figures above are the
/// justification for.** Admitting an array that holds a `Value::Text` makes an ordered
/// comparison over a `text[]` operand walk where it used to skip (one holding none —
/// empty, or all-NULL — is still declined). Correctness does not move —
/// `values_compare` *is* `values_compare_ordered(_, _, None)` — so what moves is time.
/// Measured with psql's `\timing` inside one session (a warm connection, so no per-query
/// process spawn — wall-clock around `psql -c` is swamped by a ~35 ms round trip and
/// reports the wrong sign), 20 000 rows, medians of five, release server, in-memory
/// B+tree, default profile; HEAD against the same build with this function back to
/// `matches!(value, Value::Text(_))`:
/// `count(*) … WHERE ta < ARRAY['zzz']` 9.25 ms against 7.60 ms,
/// `… WHERE ta < ARRAY[(SELECT v FROM one)]` 33.11 ms against 28.71 ms,
/// `… WHERE ia < ARRAY[99]` 7.40 ms against 7.34 ms, and the non-array control
/// `… WHERE i > 5` 5.03 ms against 5.00 ms. Both builds answered identically over that
/// data. `Value::Jsonb` is not a `Value::Array` and is not on this path.
///
/// **Read the four pairs, not a summary of them.** A single band ("between a sixth and a
/// fifth") over the two `text[]` rows stood here and in `docs/enums.md`, and no reading of
/// it holds for both: against the narrower gate's time the literal row is +21.7% and the
/// subquery row +15.3%, against this build's own time +17.8% and +13.3%. They share a sign
/// and not a magnitude, and the magnitude is what decides whether the `TODO` below is
/// worth taking.
// TODO: a cheaper admission for the array arm — `matches!(a.element_type, TypeOid::Text)`,
// O(1) against a walk of the elements. More than one shape separates them, and the two
// the paragraph above already names as declined are ordinary: `build_array_value` types
// an array from its non-NULL, non-text peers and falls back to `TypeOid::Text` when it
// has none, so an all-NULL array and an empty one are both `element_type: Text` —
// measured, and this predicate answers `false` for both while the cheaper one answers
// `true`. Admitting those is safe under this function's own contract (a superset is).
// What is undecided is the array whose declared `element_type` disagrees with its
// contents, in either direction; neither its reachability nor which of the two a caller
// should believe is measured.
pub(super) fn operand_can_be_enum(value: &Value) -> bool {
    match value {
        Value::Text(_) => true,
        Value::Array(a) => a.elements.iter().any(operand_can_be_enum),
        _ => false,
    }
}

/// The **ordered member labels** of the enum an operand list is typed by, or `None`
/// when no operand carries an enum identity this engine can see (and when two of them
/// carry *different* ones — a pair 18.6 answers `42883` for, which pglet leaves on the
/// label-byte order rather than inventing a verdict for).
///
/// The *operand* form of [`expr_enum_with`]: a comparison's enum is its operands'
/// unified one even though the comparison's own result is a boolean, so this cannot be
/// the node walk itself. Both unify through [`unify_enum`].
pub(super) fn operands_enum_order<'x, 'e, R: ColumnResolver>(
    operands: impl IntoIterator<Item = &'e Expr>,
    resolver: &'x R,
    env: EvalEnv<'x>,
) -> Option<&'x [String]> {
    unify_enum(
        operands,
        &EnumLeaves {
            column: &|table, name| column_enum_order(resolver, env, table, name),
            // The catalog this evaluation was lent, or `None` — the correct state, and
            // the same one `EvalLeafScope::subquery_type` answers with.
            subquery: &|plan| subquery_enum_order(plan, env.schema()?),
        },
    )
}

/// What [`expr_enum_with`] asks of the outside world, and every caller answers both.
///
/// The walk answers every node from the node and its children **except** two, and they
/// are the same two `planner::LeafScope` names for the same reason: a `Column` needs a
/// namespace, and a scalar `Subquery` needs its plan's first output column. Being a
/// **pair** is what closed the second: while the walk carried the column leaf alone,
/// `Subquery` had no way to answer and its arm was a bare `None`, so
/// `(SELECT m FROM u) < 'ok'` compared by label bytes — the reverse verdict for a `mood`
/// — and a `DELETE` under that predicate removed every row where 18.6 removes none.
///
/// `T` is the caller's fact about the enum, not the enum itself: the ordering sites
/// take the borrowed member list (`&[String]`), the descriptor site takes the type's
/// **name** (`String`). One walk, two instantiations — which is what keeps
/// `ColumnDesc::enum_type` and the order a projected expression sorts by from being
/// two lists that must agree (they were, and did not: `min(m)` was in the second and
/// not the first).
pub struct EnumLeaves<'f, T> {
    /// `[table.]name`'s enum in the caller's namespace.
    pub column: &'f dyn Fn(Option<&str>, &str) -> Option<T>,
    /// A scalar subquery's enum — its `Plan`'s first output column's, which is the
    /// planner-built descriptor and needs the caller's catalog to read
    /// ([`subquery_enum_order`] / [`subquery_enum_name`] are the two instantiations).
    pub subquery: &'f dyn Fn(&Plan) -> Option<T>,
}

/// A `dyn Fn` has no `Debug`, and the struct is public because the planner's two
/// instantiations live in `planner::coercion` and `planner::plan`; the fields carry no
/// state worth printing.
impl<T> std::fmt::Debug for EnumLeaves<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnumLeaves")
    }
}

/// [`EnumLeaves`] at the ordering sites' instantiation — the borrowed member list.
pub type EnumOrderLeaves<'x, 'f> = EnumLeaves<'f, &'x [String]>;

/// A scalar subquery's enum **member order**: its plan's first output column's enum
/// name, resolved in `schema`.
///
/// The reader that gives `Plan::output_columns_scoped`'s `ColumnDesc::enum_type` its
/// first consumer. Two consequences worth carrying: the planner's own set-operation and
/// `USING` arms (`unified_enum_type`) are live through this, where before nothing read
/// them; and a subquery projecting an **expression** rather than a passthrough column
/// answers too, `Expr::to_column_desc` deriving the identity where a passthrough merely
/// propagates it (`EDGES.md` row 107). The one projection it does not reach is a
/// **scalar subquery**, whose leaf that derivation declines for a cost reason stated
/// there, so `(SELECT (SELECT min(m) FROM u))` is still label-byte ordered.
///
/// **It costs a `Plan::output_columns` per evaluation**, which on a comparison operand is
/// per row — the same shape and the same frequency `EvalLeafScope::subquery_type` already
/// pays, so this doubles an existing walk rather than adding a new kind of one. Measured
/// on a **release** server over 20 000 rows, medians of three, against the same build
/// with this arm returning `None`:
/// `SELECT count(*) FROM big WHERE m > (SELECT m FROM u2 WHERE id = 1)` is 39.8 ms
/// against 34.0 ms, so the leaf is ~17 % of that shape where its answer is read.
///
/// **The cost was not on the enum's axis, which is why the guard is on the values.** The
/// walk ran for every `<`/`<=`/`>`/`>=` whatever the operands were typed, so before
/// [`operand_can_be_enum`] the same shape over an `int` column — no enum in the
/// statement at all — paid 38.0 ms against the 29.9 ms it pays now, and even `i > 5`,
/// with no subquery in it, paid 5.21 ms against 3.82 ms. With the guard the `int` shape
/// is at the arm-less floor (29.9 against 31.1, inside the noise), which is what a guard
/// on `values_compare_ordered`'s own precondition should do.
#[must_use]
pub fn subquery_enum_order<'x>(plan: &Plan, schema: &'x Schema) -> Option<&'x [String]> {
    let cols = plan.output_columns(schema);
    super::super::helpers::enum_order_of(&schema.enums, cols.first()?.enum_type.as_deref())
}

/// [`subquery_enum_order`] at the descriptor site's instantiation — the enum's **name**.
#[must_use]
pub fn subquery_enum_name(plan: &Plan, schema: &Schema) -> Option<String> {
    plan.output_columns(schema).first()?.enum_type.clone()
}

/// The enum an expression's **type** carries, resolved through `leaf`.
///
/// # The rule, and why the match is exhaustive
///
/// A node's enum is the **unified enum of the operands its own result type is drawn
/// from**; a node whose result type is not one of its operands' has none. 18.6 needs
/// no such walk — `COALESCE(m, 'sad')` simply *has type* `mood` by ordinary type
/// unification — and pglet needs one because an enum value is a `Value::Text` whose
/// `TypeOid` is `Text`, so the identity rides on the **column**
/// (`Column.enum_type` → `ColumnDesc::enum_type`) rather than on the value.
///
/// The match carries **no `_` arm**, deliberately. Its predecessor listed a column
/// reference and three value-preserving wrappers over one, with a `_ => None` catch-all,
/// and every expression kind that unifies its operands' types — `COALESCE`, `CASE`, `GREATEST`/`LEAST`,
/// `NULLIF` — fell through it and ordered by label bytes instead, in a read *and* in
/// the two write paths (`docs/enums.md`). A catch-all makes the next such kind another
/// silent wrong answer; an exhaustive match makes it a compile error, which is the
/// same reason `deparse.rs` is exhaustive.
///
/// # An array answers at its element's width
///
/// `ARRAY[…]`'s result type is `element[]`, not `element`, so read literally the rule
/// above would give it none. What the walk's answer is *for* settles it: every consumer
/// feeds the member list to [`values_compare_ordered`](super::compare::values_compare_ordered), which orders an array
/// **element-wise** and reads one list at every element — so the array node's answer is
/// its elements' unified enum, and the same list serves the scalar reading and the array
/// one. `Expr::Unnest` is the inverse and answers the same way for the same reason: its
/// result type *is* its operand's element type.
///
/// The arm this replaced was a `None` justified as *enum arrays have no column form
/// here*, which is true of a **column** (`ms mood[]` is `type "mood" is not supported`)
/// and says nothing about an `ARRAY[…]` **expression**, which pglet builds, orders, and
/// writes through. Measured against 18.6 over `mood AS ENUM ('sad','ok','happy')` before
/// this arm: `WHERE ARRAY[m] < ARRAY['ok'::mood]` selected the opposite row, a `DELETE`
/// under it reported the same `DELETE 1` while destroying a different row, a
/// `CHECK (ARRAY[m] < ARRAY['ok'::mood])` admitted the opposite row set with no error on
/// the row it took, and a unique partial index over `WHERE ARRAY[m] > ARRAY['sad'::mood]`
/// held a duplicate 18.6 refuses. It is also what `EDGES.md` row 111 turned on: widening
/// the `AnyAll` walk to both operands moved nothing while this arm answered `None`,
/// because `ARRAY[m]` was the operand that carried the identity.
///
/// # The boundary
///
/// A **cast's** identity is erased where the parser reads the type name: a member
/// literal's cast to an enum is built as the plain cast to `text`, so `'happy'::mood` is
/// byte-identical to `'happy'::text` in the plan and no walk can recover it, and any
/// other operand's is the conversion `StringFunc::EnumIn` under that same cast, which the
/// `StringFn` arm answers `None` for as it does every string function — the `Cast` /
/// `CastTypmod` arms below are `None` for that reason and not for a structural one
/// (`EDGES.md` row 104). It is the **only** boundary now — the only
/// arm that drops an identity 18.6 can see. A scalar subquery used to be a second, on a
/// `None` arm three places recorded as "measured unreachable" from a single position:
/// `ORDER BY (SELECT …)`, which is `0A000` here ("scalar subquery requires execution
/// context") because that site evaluates with no subquery callback lent to it — nothing
/// to do with enums. What refuses is that **class** and not a countable list of
/// positions (`EDGES.md` row 89, which states it): a node whose evaluation is lent no
/// subquery callback of its kind. That derivation has **two steps**, and compressing it
/// to one is what made it look like a grep for SQL positions: `grep SubqueryExec::NONE`
/// names the *evaluations* that lend none, and a SQL position is a member when its
/// expression reaches one of them carrying a subquery. Neither of the enum-free members
/// below appears in that grep's output, and both are members. **Today** the grep is the
/// whole predicate only because the constructions of [`SubqueryExec`](super::env::SubqueryExec) lend its three
/// sub-plan callbacks all or none: `executor::query::correlated`'s sets all three, and the
/// grouping's (`executor::query::aggregate`) lends only aggregate values, spelled
/// `..SubqueryExec::NONE` so the grep finds it — a construction lending a callback of one
/// kind and not another is one the all-`None` sentinel cannot name. The class has members carrying no enum at all
/// (`GROUP BY (SELECT n …)`, `INSERT … VALUES ((SELECT n …))`). The enum spellings here
/// are this walk's subject, not the class's shape, and this comment cannot be read as
/// the list. Measured reachable
/// and silent are a projected comparison, `WHERE`, `BETWEEN`, `COALESCE`, `CASE`,
/// `greatest`/`least`,
/// subquery-against-subquery, and a `DELETE` / `UPDATE` predicate. So the arm asks
/// [`EnumLeaves::subquery`], and what it cannot answer is stated there.
// The arms are grouped by *why* each answers what it does, and that grouping is the
// documentation: merging two same-bodied arms from different groups would erase the
// reason the next reader needs.
#[allow(clippy::match_same_arms)]
pub fn expr_enum_with<T: PartialEq>(expr: &Expr, leaf: &EnumLeaves<'_, T>) -> Option<T> {
    match expr {
        // The leaf: the one place a *column's* enum identity enters the walk.
        Expr::Column { table, name } => (leaf.column)(table.as_deref(), name),

        // Value-preserving wrappers — the result *is* the operand's value.
        Expr::Alias { expr, .. } | Expr::UnaryPlus(expr) => expr_enum_with(expr, leaf),

        // `min`/`max` return one of their argument's own values unchanged, so their
        // result carries the argument's enum. `count` and `avg` produce a number;
        // `sum` also reports its argument's type (`aggregate_result_type`) but is
        // operand-gated to the numeric/interval families, so it never sees an enum.
        Expr::Aggregate {
            func: AggregateFunc::Min | AggregateFunc::Max,
            arg,
            ..
        } => expr_enum_with(arg, leaf),
        Expr::Aggregate { .. } => None,

        // `COALESCE(a, b, …)` is `select_common_type` over every argument.
        Expr::Coalesce(args) => unify_enum(args, leaf),

        // `CASE` is `select_common_type` over the **results** only: the simple form's
        // operand and every `WHEN` condition feed the *comparison*, not the result.
        Expr::Case {
            when_clauses,
            else_result,
            ..
        } => unify_enum(
            when_clauses
                .iter()
                .map(|(_, r)| r)
                .chain(else_result.iter().map(Box::as_ref)),
            leaf,
        ),

        // `greatest`/`least` are `select_common_type` over their arguments. `nullif`'s
        // result is 18.6's `exprType(linitial(args))` — the first argument **after**
        // `make_op` coerced the pair to a common type, so an unknown-typed first
        // argument beside an enum second one is that enum; unifying both reproduces
        // that without a coercion pass. `concat`/`concat_ws` are text.
        Expr::VariadicFn {
            func: VariadicFunc::Greatest | VariadicFunc::Least | VariadicFunc::NullIf,
            args,
        } => unify_enum(args, leaf),
        Expr::VariadicFn {
            func: VariadicFunc::Concat | VariadicFunc::ConcatWs,
            ..
        } => None,

        // A cast's enum name is dropped in the parser (see § The boundary).
        Expr::Cast { .. } | Expr::CastTypmod { .. } => None,

        // The walk's second leaf: a scalar subquery's enum is its plan's first output
        // column's, which only the caller's catalog can read (§ The boundary).
        Expr::Subquery { plan } => (leaf.subquery)(plan),

        // Boolean-valued: a comparison, a quantified comparison, a null/boolean test,
        // a negation, an `IN` in either spelling, `EXISTS`. `BinaryOp`'s arithmetic and
        // `||` spellings are numeric/text. An operand's enum still decides such a
        // node's *comparison*, which is `operands_enum_order`'s job and not this walk's.
        Expr::BinaryOp { .. }
        | Expr::AnyAll { .. }
        | Expr::Not(_)
        | Expr::IsNull(_)
        | Expr::IsNotNull(_)
        | Expr::BooleanTest { .. }
        | Expr::InList { .. }
        | Expr::InSubquery { .. }
        | Expr::Exists { .. } => None,

        // Numeric- or text-valued by construction: negation, the math/string
        // families, `EXTRACT`/`date_part`, `LOWER`, `jsonb_build_object`, the niladic
        // builtins (`now()`, `gen_random_uuid()`), and the sequence functions (int8).
        // `Window`'s three functions (`row_number`/`rank`/`dense_rank`) are bigint.
        Expr::Negate(_)
        | Expr::MathFn { .. }
        | Expr::StringFn { .. }
        | Expr::Extract { .. }
        | Expr::Lower(_)
        | Expr::JsonbBuildObject(_)
        | Expr::Builtin(_)
        | Expr::SequenceFn { .. }
        | Expr::Window { .. } => None,

        // Array-valued, and answering at the **element** width for the reason
        // § An array answers at its element's width gives: `ARRAY[…]` is
        // `select_common_type` over its elements exactly as `COALESCE` is over its
        // arguments, and `unnest`'s result type *is* the array's element type, so it
        // passes its operand's answer through unchanged.
        Expr::Array(elems) => unify_enum(elems, leaf),
        Expr::Unnest(inner) => expr_enum_with(inner, leaf),

        // Carries no declared enum: a literal (an enum label is `Value::Text` and says
        // nothing about which type holds it), a `$N` parameter, a spliced `Bound`
        // value, and `DEFAULT`.
        Expr::Literal(_) | Expr::Parameter { .. } | Expr::Bound(_) | Expr::Default => None,
    }
}

/// The enum two already-derived facts unify to: the one they agree on, the one either
/// side alone carries, and `None` when they carry *different* ones.
///
/// **A side that carries no enum is no information, not a veto**, and that asymmetry is
/// what the rule turns on. It is 18.6's `select_common_type`: an `unknown`-typed literal or a
/// `NULL` beside an enum resolves to the enum, so `min(m) < 'ok'` compares by member
/// position and `SELECT m FROM t UNION ALL SELECT 'ok'` is a `mood` column. Two
/// *different* enums are a pair 18.6 refuses to unify at all — measured,
/// `UNION could not convert type weather to mood` for the set operation and
/// `operator does not exist: mood = weather` for a `USING` merge's equality, neither of
/// them the `types … cannot be matched` spelling the **text** pair takes — and pglet now
/// raises both, at the gates above this function (`EDGES.md` rows 115 and 109). The
/// `None` here is what keeps the executor from inventing an order for a pair those gates
/// cannot reach; it is not the answer to a statement any more.
///
/// **Sticky is [`unify_enum`]'s, not this function's, and the two descriptor sites do
/// not inherit it.** A nested set operation or `USING` chain folds *this* pairwise over
/// already-merged descriptors, and `None` reads as "no enum" whether the side carries
/// none or cancelled one below — so a third branch used to revive what the first two
/// cancelled: `mood ∪ weather` ordered by the label bytes and `mood ∪ weather ∪ mood` by
/// `mood`'s member positions, a different **row** set under a `WHERE`, and the 3-way
/// `JOIN … USING` spelling likewise. Reaching it takes two *different* enums — the only
/// pair this function cancels — and that pair is now refused above the fold at every
/// branch count, so the revival is unreachable and the third state on
/// `ColumnDesc::enum_type` this used to prescribe was never needed (`EDGES.md` row 109,
/// closed).
///
/// **The one rule the unifying sites read**, and the reason it is a function: the
/// others are [`unify_enum`] (this over an expression list) and
/// `executor::helpers::unified_enum_type` (this over two `ColumnDesc`s, for a set
/// operation's and a `USING` merge's output column). Written
/// out per site it drifts, and it *had*: `unified_enum_type` read the `(Some, None)`
/// arm as a **veto**, so a merged column whose second branch was a bare `'ok'` — the
/// pair that actually reaches it — ordered by label bytes, the exact reverse of 18.6
/// for a `mood`.
///
/// **What this rule is not asked about.** An enum against a plain `text` **column** is
/// `None` on both sides' descriptors here, indistinguishable from the unknown-typed
/// literal, and 18.6 refuses that pair outright (`operator does not exist: mood = text`,
/// `UNION types mood and text cannot be matched`). pglet refuses it too, at the gates
/// above this function, which read the declared identity rather than the descriptor's
/// `Option` (`EDGES.md` row 114, closed) — so what still reaches the `(Some, None)` arm
/// is the operand kind 18.6 also unifies, and the one this engine cannot tell from it, a
/// `::text`-cast side (row 104).
pub fn unify_enum_pair<T: PartialEq>(left: Option<T>, right: Option<T>) -> Option<T> {
    match (left, right) {
        (Some(l), Some(r)) => (l == r).then_some(l),
        (Some(o), None) | (None, Some(o)) => Some(o),
        (None, None) => None,
    }
}

/// The one enum a list of expressions shares, by [`expr_enum_with`] — [`unify_enum_pair`]
/// across the list, with a disagreement **sticky**.
///
/// Sticky is the one thing the list rule adds to the pair rule, and it is not a fold:
/// `COALESCE(mood_col, weather_col, mood_col)` is a pair 18.6 refuses, so the third
/// operand must not revive the enum the first two cancelled.
///
/// Reached from the node arms above and from [`operands_enum_order`].
fn unify_enum<'e, T: PartialEq>(
    exprs: impl IntoIterator<Item = &'e Expr>,
    leaf: &EnumLeaves<'_, T>,
) -> Option<T> {
    let mut found: Option<T> = None;
    for e in exprs {
        let next = expr_enum_with(e, leaf);
        let both_typed = found.is_some() && next.is_some();
        found = unify_enum_pair(found, next);
        if both_typed && found.is_none() {
            return None;
        }
    }
    found
}

/// [`expr_enum_with`] at the ordering sites' instantiation — the borrowed member list.
pub fn expr_enum_order_with<'x>(
    expr: &Expr,
    leaf: &EnumOrderLeaves<'x, '_>,
) -> Option<&'x [String]> {
    expr_enum_with(expr, leaf)
}

/// One column reference's enum member order — the inner scope's answer, else the
/// **outer** chain's, by the same walk [`column_declared_type`](super::eval::column_declared_type) takes for the declared
/// type. The two read one [`EvalEnv::resolve_outer_slot`] verdict, so a correlated
/// `o.m > 'sad'` cannot resolve its order against a different frame than its value.
pub(super) fn column_enum_order<'x, R: ColumnResolver>(
    resolver: &'x R,
    env: EvalEnv<'x>,
    table: Option<&str>,
    name: &str,
) -> Option<&'x [String]> {
    if let Some(t) = table {
        if resolver.scope_has_table(t) {
            return resolver.column_enum_order(Some(t), name);
        }
        return match env.resolve_outer_slot(Some(t), name) {
            OuterSlot::Found(frame, i) => frame.enum_order_at(i),
            OuterSlot::Claimed | OuterSlot::Ambiguous => None,
            OuterSlot::Unclaimed => resolver.column_enum_order(Some(t), name),
        };
    }
    match resolver.resolve(None, name) {
        Ok(_) => resolver.column_enum_order(None, name),
        Err(Error::AmbiguousColumn { .. }) => None,
        Err(_) => match env.resolve_outer_slot(None, name) {
            OuterSlot::Found(frame, i) => frame.enum_order_at(i),
            OuterSlot::Claimed | OuterSlot::Ambiguous | OuterSlot::Unclaimed => None,
        },
    }
}
