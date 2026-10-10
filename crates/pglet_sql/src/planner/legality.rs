//! Operand-type legality rules for the operators and aggregates pglet gates loud.
//!
//! Every rule here is a pure function of `TypeOid`s, so the **two** places that
//! need the verdict share one implementation and cannot drift:
//!
//! - **Eval time** (`executor::expr`, `executor::query::aggregate`) derives the
//!   operand types from the runtime `Value`s and gates each evaluation.
//! - **DDL time** ([`check_expr_node_operand_types`]) derives them statically from
//!   `Expr::infer_type` and gates a **stored** expression — an index partial
//!   predicate, an index key expression, or a CHECK constraint — before it is
//!   recorded, which is where Postgres reports the failure. Only the per-node
//!   verdict lives here; the traversal is its caller's, in `executor::ddl`.
//!
//! **The "cannot drift" is a property of the rules both places call, not of this file.**
//! [`unnest_legality`] lives here and is a **signature** rule rather than an operand one
//! — 18.6 raises it in `ParseFuncOrColumn`, not from `pg_operator` — and it has exactly
//! one consumer, `planner::coercion`'s `Expr::Unnest` arm. The DDL-time checker above
//! does not call it, and the drift is measured: at all four stored-expression sites a
//! non-array `unnest` is 18.6's `42883` and pglet's `0A000`, by two different routes with
//! two different messages. Before adding a rule here, ask which of the two callers will
//! consult it; one caller is a residual, not a rulebook. `EDGES.md` row 55 and
//! `docs/coercion.md` carry the members.
//!
//! Static checking is tractable for stored expressions specifically because they
//! carry **no bound parameters**: every leaf is a column, a literal, or a
//! constructed value, so inference resolves without a bind step. It is *not*
//! tractable for general query expressions, which is why eval-time gating remains
//! the primary mechanism and this module is the shared rulebook rather than a
//! replacement.
//!
//! `None` means "the type is not known" — a SQL NULL at eval time, an
//! unresolvable expression at DDL time — and is always **accepted**. Rejecting an
//! unknown would turn inference gaps into spurious errors.
//!
//! @module

use crate::planner::VariadicFunc;
use crate::planner::common_type::ResolvedType;
use crate::planner::expr::BoolContext;
use crate::planner::plan::{LeafScope, PlannerScope};
use crate::types::{TypeOid, opt_type_oid_display_name, type_oid_display_name};
use crate::{Error, Result};

use super::expr::{AggregateFunc, BinaryOp, Expr, Literal, MathFunc};

/// The Postgres operator symbol a `LIKE`-family `BinaryOp` resolves through, for
/// both the legality rule and the error message.
///
/// Returns `None` for every other operator, which is how the DDL-time walker
/// decides whether a node is `LIKE`-shaped at all.
#[must_use]
pub const fn like_symbol(op: BinaryOp) -> Option<&'static str> {
    Some(match op {
        BinaryOp::Like => "~~",
        BinaryOp::NotLike => "!~~",
        BinaryOp::Ilike => "~~*",
        BinaryOp::NotIlike => "!~~*",
        _ => return None,
    })
}

/// The Postgres operator symbol a POSIX-regex `BinaryOp` resolves through, for both the
/// legality rule and the error message; `None` for every other operator.
#[must_use]
pub const fn regex_symbol(op: BinaryOp) -> Option<&'static str> {
    Some(match op {
        BinaryOp::RegexMatch => "~",
        BinaryOp::NotRegexMatch => "!~",
        _ => return None,
    })
}

/// A POSIX-regex operator (`symbol` = `~` / `!~`).
///
/// 18.6 registers these as `text ~ text`, `name ~ text` and `bpchar ~ text`, so a
/// string-typed pair resolves (a `varchar` or `name` right operand reaching `text`
/// implicitly) and every other pair is `42883` — measured for `integer`, `bytea` and an
/// **enum** on the left (`operator does not exist: mood ~ unknown`) and an `integer` on the
/// right. The enum refusal needs the operand's identity, which only a plan-time caller has:
/// an eval-time caller passes [`ResolvedType::plain`] operands, and an enum value reaching
/// it reads as `text` (`EDGES.md` row 251). A side whose type is not known is accepted, as
/// everywhere in this rulebook.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] for an operand pair with no operator.
pub fn regex_legality(
    left: Option<&ResolvedType>,
    right: Option<&ResolvedType>,
    symbol: &str,
) -> Result<()> {
    let (Some(l), Some(r)) = (left, right) else {
        return Ok(());
    };
    let string = |t: &ResolvedType| is_string(t.oid) && t.enum_type.name().is_none();
    if string(l) && string(r) {
        Ok(())
    } else {
        Err(Error::UndefinedFunction(format!(
            "operator does not exist: {} {symbol} {}",
            l.display_name(),
            r.display_name()
        )))
    }
}

/// The Postgres operator symbol a comparison `BinaryOp` resolves through, for both
/// the legality rule and the error message.
///
/// `IS [NOT] DISTINCT FROM` maps to `=`: Postgres resolves it through the `=`
/// operator and reports the failure as `operator does not exist: text = integer`,
/// not as a distinct spelling.
///
/// Returns `None` for every other operator, which is how the DDL-time walker decides
/// whether a node is comparison-shaped at all.
#[must_use]
pub const fn comparison_symbol(op: BinaryOp) -> Option<&'static str> {
    Some(match op {
        BinaryOp::Eq | BinaryOp::IsDistinctFrom | BinaryOp::IsNotDistinctFrom => "=",
        BinaryOp::Ne => "<>",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        _ => return None,
    })
}

/// The SQL spelling of an **arithmetic** operator, or `None` for every other.
///
/// The shape test [`arithmetic_legality`]'s two callers dispatch on, so neither has to
/// re-list the arithmetic operators. A call-spelled operator (`mod(…)`, `power(…)`,
/// `pow(…)`) answers the symbol of the operator it evaluates as.
#[must_use]
pub const fn arithmetic_symbol(op: BinaryOp) -> Option<&'static str> {
    Some(match op.operator_form() {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Pow => "^",
        _ => return None,
    })
}

/// The comparison operator `op` becomes when its operands are swapped: `a < b` is
/// `b > a`, `a = b` is `b = a`.
///
/// Used where the planner **normalized** a predicate the user wrote value-first
/// (`5 < k` → `k > 5`): the normalized form drives execution, this recovers the
/// written spelling for the error message, so both build profiles report the operands
/// in the same order with the same symbol.
#[must_use]
pub const fn mirrored_comparison_op(op: BinaryOp) -> BinaryOp {
    match op {
        BinaryOp::Lt => BinaryOp::Gt,
        BinaryOp::Le => BinaryOp::Ge,
        BinaryOp::Gt => BinaryOp::Lt,
        BinaryOp::Ge => BinaryOp::Le,
        // `=` / `<>` (and the `IS [NOT] DISTINCT FROM` forms) are symmetric.
        other => other,
    }
}

/// `left <op> right` for a comparison operator (`=` `<>` `<` `<=` `>` `>=`, and the
/// `IS [NOT] DISTINCT FROM` forms that resolve through `=`).
///
/// Postgres registers a comparison operator per type plus a small set of **implicit
/// cross-type** ones, so a pair with no implicit path resolves to no operator at all:
/// `text = integer`, `integer = boolean`, `bytea = text`, `text[] = integer[]` all
/// raise `42883`. Without this gate they dispatched on the runtime `Value` variant
/// and fell through to a permissive `false` / text-order — so `WHERE name = 5` and a
/// mistyped join `ON a.int_id = b.uuid_col` silently returned **zero rows**.
///
/// The accepted pairs, verified against a live Postgres 18:
///
/// - **the same type** (every pglet type has its own comparison operators);
/// - the **numeric family** in every pairing (`int2`/`int4`/`int8`/`float4`/`float8`/
///   `numeric`);
/// - the **date/timestamp family** in every pairing (`date`/`timestamp`/`timestamptz`);
/// - the **string family** (`text`/`varchar` — one `Value::Text` in pglet, so this is
///   reachable only from a declared column type);
/// - **`time` vs `interval`**, either order, via Postgres's implicit `time → interval`
///   cast (`pg_cast.dat`, `castcontext => 'i'`).
///
/// Every other cross-type pair is rejected. Note that the neighbouring families do
/// **not** mix: `date = time`, `timestamp = interval`, and `time = timestamp` are all
/// `42883` in Postgres.
///
/// An **enum** operand resolves an operator only against the *same* enum. 18.6 registers
/// the comparison operators on `anyenum`, so both sides must be one type — `mood > text`,
/// `mood = character varying` and `mood > m3` are each `42883`, measured, and each is
/// unreachable from the OID alone, an enum's being `text` (`EDGES.md` rows 114 and 115).
/// That is why the operand currency here is a [`ResolvedType`] and not a `TypeOid`.
///
/// `None` (a SQL NULL at eval time, an unresolvable expression at DDL time) is
/// accepted, per this module's contract.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] (`42883`) for an operand pair with no operator.
pub fn comparison_legality(
    left: Option<ResolvedType>,
    right: Option<ResolvedType>,
    symbol: &str,
) -> Result<()> {
    let (Some(l), Some(r)) = (left, right) else {
        return Ok(());
    };
    if comparable_resolved(&l, &r) {
        Ok(())
    } else {
        Err(Error::UndefinedFunction(format!(
            "operator does not exist: {} {symbol} {}",
            l.display_name(),
            r.display_name()
        )))
    }
}

/// [`comparable_types`] over fully-resolved operands — the enum half in front of it.
///
/// An enum on either side admits only the identical enum: no `pg_cast` row reaches an
/// enum implicitly and every comparison operator it has is `anyenum`'s, so there is no
/// pair to widen through. An **array** of an enum is judged the same way, its identity
/// being its element's — `mood[] < text[]` and `mood[] < m3[]` are both `42883` on 18.6
/// and both refused here, measured over `ARRAY[m] < ARRAY[s]`, `ARRAY[m] < ARRAY[y]` and
/// `ARRAY[m] < ARRAY['ok']`, which is to say a written `ARRAY[…]` *does* reach this rule
/// — through the **complete** reader rather than the shallow one, `certain_operand_type`
/// having no `Expr::Array` arm at all. The `op ANY` / `op ALL` spelling reaches it at the
/// array's *element* width ([`array_element_resolved_type`]) and `IN (SELECT …)` at the
/// sub-plan column's.
///
/// What the written-`ARRAY[…]`, `op ANY`/`op ALL` and `IN (SELECT …)` spellings leave is
/// an operand whose identity is **undecidable** rather than one their readers cannot
/// type: a `Cast`, whose enum name the parser dropped, on an element
/// (`ARRAY['ok'::mood]`, which 18.6 answers and so does this) or on the whole right-hand
/// side. This engine's own passes write that node too, so a user's cast is answered as
/// the pass's output is, and which way that falls is the pass's doing rather than the
/// rule's: over an `unknown` **array element** beside a typed one it is a silent
/// under-refusal (`ARRAY[m, NULL::text]`), while over a **set operation's** branch column
/// and a **derived table's** cell it is a loud over-refusal
/// (`m IN (SELECT 'ok'::mood UNION SELECT 'sad'::mood)`). `EDGES.md` rows 114 and 104
/// carry the members.
fn comparable_resolved(l: &ResolvedType, r: &ResolvedType) -> bool {
    // An operand whose identity cannot be decided — a `text`-targeted cast, which is
    // what `x::mood` parses to, or an `unknown` literal — is judged on its OID, so
    // `greatest(m, t::mood)` and `m = 'ok'` answer as 18.6 answers them.
    if l.enum_type.is_undecided() || r.enum_type.is_undecided() {
        return comparable_types(l.oid, r.oid);
    }
    if l.enum_type.name().is_some() || r.enum_type.name().is_some() {
        return l == r;
    }
    comparable_types(l.oid, r.oid)
}

/// Whether Postgres resolves a comparison operator for the ordered pair `(l, r)`.
///
/// Deliberately **not** built on `common_type_step`: that helper serves
/// `greatest`/`least`/`nullif`/set-op unification and promotes through
/// `numeric_family_result`, which answers `Some(float8)` for `text` vs `float8` — a
/// widening the comparison operators do not have.
const fn comparable_types(l: TypeOid, r: TypeOid) -> bool {
    l as u32 == r as u32
        || is_numeric_family(l) && is_numeric_family(r)
        || is_temporal_family(l) && is_temporal_family(r)
        || is_string(l) && is_string(r)
        // `oid` compares with the **integer** types in either operand order (measured on
        // 18.6: `conrelid <> 0`, `conrelid > 100`, `conrelid = conindid` all resolve),
        // and with nothing wider — `conrelid > 1.5` is `42883 operator does not exist:
        // oid > numeric` there. It is a comparison rule and deliberately **not**
        // membership in [`is_numeric_family`], which would carry `oid` into the
        // assignment cast. **It does not by itself keep `oid` out of arithmetic** — this
        // engine has no arithmetic type gate at all, `eval_arithmetic` deciding on the
        // value pair, and an `oid` *is* a `Value::Int4`. That is
        // [`arithmetic_legality`]'s job, and reading this rule as covering it left
        // `conrelid + 1` silently answering an integer.
        || oid_comparable(l, r)
        || oid_comparable(r, l)
        // Postgres implicitly casts `time → interval`, so the pair compares as two
        // intervals in either operand order.
        || matches!(
            (l, r),
            (TypeOid::Time, TypeOid::Interval) | (TypeOid::Interval, TypeOid::Time)
        )
}

/// Whether `(l, r)` is `oid` against an integer type — the ordered half of the `oid`
/// comparison rule, applied in both orders by [`comparable_types`].
const fn oid_comparable(l: TypeOid, r: TypeOid) -> bool {
    matches!(l, TypeOid::Oid)
        && matches!(
            r,
            TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8 | TypeOid::Oid
        )
}

/// `left <arith> right` — the operand rule for the arithmetic operators (`+ - * / % ^`)
/// and for unary `-`.
///
/// This rulebook has no *general* arithmetic gate: arithmetic is decided by the value
/// pair in `executor::expr`'s `eval_arithmetic`, which is why `text + 1` is `42883`
/// there rather than here. That worked as long as every type pglet declared had a
/// value shape of its own. `oid` and `"char"` do not — an `oid` **is** a `Value::Int4`
/// and a `"char"` **is** a `Value::Text` (they are catalog *declaration* types, see
/// their `TypeOid` variants) — so `conrelid + 1` evaluated happily to an integer where
/// 18.6 refuses `42883 operator does not exist: oid + integer`. Measured on 18.6 for
/// `+`, `-`, `*`, `/`, `%` and unary `-`; `sum(oid)` was already loud, from
/// [`aggregate_legality`], which is what made the gap look closed.
///
/// So the gate is narrow on purpose: it refuses **only** an `oid` / `"char"` operand and
/// says nothing about any other pair, leaving `eval_arithmetic`'s verdicts — and the
/// walker rows recorded against them — exactly where they were.
///
/// # Two callers, and re-derive the list rather than inheriting it
///
/// [`check_expr_node_operand_types`] (DDL time) and `executor::expr`'s `eval_expr_full`
/// (the scalar walk). A grouping query's expressions take the scalar walk too, an
/// aggregate's value read through the lookup the grouping lends it, so the arithmetic
/// around an aggregate is gated where any other is: `max(conrelid) + 1` is `42883` as
/// `conrelid + 1` is. **What the operand type is read *through* is the other half**:
/// the eval-time caller takes the complete declared-type
/// walk (`Expr::infer_type` over its own scope) rather than a reader that enumerates
/// expression kinds — two such readers stood there and each missed the shapes the other
/// did.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] (`42883`) when either operand is `oid` or `"char"`, named
/// as 18.6 names it: the operator for `a % b`, the function for `mod(a, b)`.
pub fn arithmetic_legality(
    left: Option<TypeOid>,
    right: Option<TypeOid>,
    op: BinaryOp,
) -> Result<()> {
    if !left.is_some_and(is_declaration_only) && !right.is_some_and(is_declaration_only) {
        return Ok(());
    }
    let (l, r) = (
        opt_type_oid_display_name(left),
        opt_type_oid_display_name(right),
    );
    Err(Error::UndefinedFunction(op.call_name().map_or_else(
        || {
            format!(
                "operator does not exist: {l} {} {r}",
                arithmetic_symbol(op).unwrap_or("+")
            )
        },
        |name| format!("function {name}({l}, {r}) does not exist"),
    )))
}

/// `- operand` — the unary-minus half of [`arithmetic_legality`], whose message 18.6
/// words with the operator first (`operator does not exist: - oid`).
///
/// # Errors
///
/// [`Error::UndefinedFunction`] (`42883`) when the operand is `oid` or `"char"`.
pub fn negate_legality(operand: Option<TypeOid>) -> Result<()> {
    if !operand.is_some_and(is_declaration_only) {
        return Ok(());
    }
    Err(Error::UndefinedFunction(format!(
        "operator does not exist: - {}",
        opt_type_oid_display_name(operand)
    )))
}

/// `f(arg)` for a [`MathFunc`] — the same declaration-only refusal
/// [`arithmetic_legality`] makes, at the other position an `oid` reaches numerically.
///
/// Measured: `abs(conrelid)` is `42883 function abs(oid) does not exist` on 18.6, while
/// `sum(conrelid)` was already loud from [`aggregate_legality`].
///
/// **One caller**: the scalar eval walk, which a grouping query's expressions take too, so
/// `abs(max(conrelid))` is `42883` as `abs(conrelid)` is. [`check_expr_node_operand_types`]
/// has no `MathFn` arm, so this rule has no plan-time half to stay in step with.
/// `docs/coercion.md` carries the measurement.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] (`42883`) when the argument is `oid` or `"char"`.
pub fn math_fn_arg_legality(func: MathFunc, arg: Option<TypeOid>) -> Result<()> {
    if !arg.is_some_and(is_declaration_only) {
        return Ok(());
    }
    Err(Error::UndefinedFunction(format!(
        "function {}({}) does not exist",
        crate::deparse::math_fn_name(func),
        opt_type_oid_display_name(arg)
    )))
}

/// The types that carry another type's **value shape** and none of its operators: an
/// `oid` is a `Value::Int4`, a `"char"` a `Value::Text`. They compare (see
/// [`oid_comparable`] and [`is_string`]) and they concatenate or measure like their
/// shape, but they have no arithmetic on 18.6 and nothing here may infer one from the
/// value alone.
const fn is_declaration_only(oid: TypeOid) -> bool {
    matches!(oid, TypeOid::Oid | TypeOid::Char)
}

/// `column := <expr of type source>` — the **assignment** cast rule, which every DML
/// site that stores a value into a column takes (`transformAssignedExpr`).
///
/// Postgres decides this with `can_coerce_type(… COERCION_ASSIGNMENT)`, and the class it
/// refuses is **wider than `pg_cast`'s explicit-only rows**: a pair with no `pg_cast`
/// entry at all is refused too, so `text → integer` is `42804` exactly as `integer →
/// boolean` is. Read the other way round, the accepted set is what this function
/// enumerates, and it is four rules rather than a table:
///
/// - **the same type**;
/// - a `pg_cast` row whose `castcontext` is `'i'` (implicit) or `'a'` (assignment) —
///   among pglet's types those are precisely the **numeric family** in every pairing
///   (`int2`/`int4`/`int8`/`float4`/`float8`/`numeric`), the **date/timestamp family**
///   in every pairing (`date`/`timestamp`/`timestamptz`), `timestamp`/`timestamptz` →
///   `time` (one way only), and `time` ↔ `interval` (both ways);
/// - **I/O coercion into a string-category target** — `find_coercion_pathway` allows
///   `COERCION_PATH_COERCEVIAIO` when `ccontext >= COERCION_ASSIGNMENT` and the
///   *target* is `TYPCATEGORY_STRING`, so **every** type assigns into `text` /
///   `varchar` with no `pg_cast` row involved (`jsonb`, `uuid`, `bytea`, `boolean`,
///   `integer[]`, …). The mirror rule — a string-category *source* — needs
///   `COERCION_EXPLICIT`, which is why `text → integer` refuses;
/// - **element-wise** for two arrays (`COERCION_PATH_ARRAYCOERCE`), which is why
///   `integer[] → text[]` and `bigint[] → integer[]` assign while `text[] → uuid[]`
///   does not.
///
/// # Provenance
///
/// The four rules are read off a measured accept/reject matrix, not off the source:
/// every ordered pair of the 21 type spellings pglet models (17 scalars plus `text[]` /
/// `integer[]` / `uuid[]` / `bigint[]`) run as `INSERT INTO <col of target> VALUES
/// (NULL::<source>)` against a live 18.6 — 441 statements, 85 cross-type accepts, every
/// rejection `42804` and no other SQLSTATE. The four rules reproduce that matrix
/// exactly — **which is a statement about this function's accept set and not about the
/// engine**, since a typed `NULL` never reaches a conversion; five of the pairs both
/// admit die in the evaluator, and `the_four_rules_reproduce_the_measured_matrix` names
/// them. An `ENUM` target is outside the matrix entirely: pglet types an enum value as
/// `text`, so `'x'::text` into an enum column stores silently where 18.6 answers
/// `42804` (also recorded there). `pg_cast` alone does not, and by a wide margin: of the 40 accepted
/// string-target pairs it has a row for exactly four (`boolean → text`/`varchar`,
/// `text ↔ varchar`), so it would refuse the other 36 (`bytea → text`,
/// `jsonb → varchar`, `uuid → text`, `integer[] → text`, …), and it has **no** row for
/// any array-to-array pair, so it would refuse all five of those too (queried against
/// the live catalog, both directions).
///
/// **A typed `NULL` is refused like any other typed value** (`UPDATE t SET b =
/// NULL::int` is `42804` on 18.6), so the caller must not skip a NULL — what it must
/// skip is Postgres's `unknown`, a bare string constant, which is a *syntactic*
/// property of the expression and therefore not this function's to decide. See
/// `super::coercion::check_assignment`.
///
/// # Errors
///
/// [`Error::Type`] (`42804`) with Postgres's own sentence — the column name and both
/// type names — for a pair no assignment cast reaches.
pub fn assignment_legality(source: TypeOid, target: TypeOid, column: &str) -> Result<()> {
    if assignment_compatible(source, target) {
        Ok(())
    } else {
        Err(Error::Type(format!(
            "column \"{column}\" is of type {} but expression is of type {}",
            type_oid_display_name(target),
            type_oid_display_name(source)
        )))
    }
}

/// Whether `source` reaches `target` by an implicit or assignment cast — the predicate
/// half of [`assignment_legality`], where its four rules are spelled.
fn assignment_compatible(source: TypeOid, target: TypeOid) -> bool {
    if source as u32 == target as u32 {
        return true;
    }
    // I/O coercion into a string-category target: every type assigns into text/varchar.
    if is_string(target) {
        return true;
    }
    if is_numeric_family(source) && is_numeric_family(target) {
        return true;
    }
    if is_temporal_family(source) && is_temporal_family(target) {
        return true;
    }
    // The two temporal pairs that are *not* symmetric families: `timestamp[tz] → time`
    // truncates to the time of day ('a'), and `time ↔ interval` casts both ways
    // ('i' one way, 'a' the other). Their mirrors have no row — `time → timestamp`,
    // `date → time` and `date ↔ interval` are all `42804` on 18.6.
    if matches!(
        (source, target),
        (
            TypeOid::Timestamp | TypeOid::Timestamptz | TypeOid::Interval,
            TypeOid::Time
        ) | (TypeOid::Time, TypeOid::Interval)
    ) {
        return true;
    }
    match (source.array_element(), target.array_element()) {
        (Some(s), Some(t)) => assignment_compatible(s, t),
        _ => false,
    }
}

/// The numeric types Postgres cross-compares in every pairing.
const fn is_numeric_family(oid: TypeOid) -> bool {
    matches!(
        oid,
        TypeOid::Int2
            | TypeOid::Int4
            | TypeOid::Int8
            | TypeOid::Float4
            | TypeOid::Float8
            | TypeOid::Numeric
    )
}

/// The date/timestamp types Postgres cross-compares in every pairing. `time` and
/// `interval` are **not** members — `date = time` and `timestamp = interval` have no
/// operator.
const fn is_temporal_family(oid: TypeOid) -> bool {
    matches!(
        oid,
        TypeOid::Date | TypeOid::Timestamp | TypeOid::Timestamptz
    )
}

/// `left || right` — the `||` operand rule.
///
/// Postgres registers `text || text`, `text || anynonarray`, `anynonarray || text`,
/// `bytea || bytea`, `jsonb || jsonb`, and the three array forms (`array||array`,
/// `array||element`, `element||array`).
///
/// pglet implements only the **text** forms. The array / `bytea` / `jsonb`
/// concatenations are Postgres-legal but unimplemented here, so they fail `0A000`
/// rather than claiming no such operator; every other operand pair (`int || int`,
/// `bool || bool`, `int[] || text`) truly resolves to no operator and fails
/// `42883`.
///
/// # Errors
///
/// [`Error::NotImplemented`] (`0A000`) for a Postgres-legal form pglet doesn't
/// evaluate; [`Error::UndefinedFunction`] (`42883`) for a pair with no operator.
pub fn concat_legality(left: Option<TypeOid>, right: Option<TypeOid>) -> Result<()> {
    let (Some(l), Some(r)) = (left, right) else {
        return Ok(());
    };
    let unimplemented = || {
        Err(Error::NotImplemented(format!(
            "operator {} || {} is not supported (only text concatenation)",
            type_oid_display_name(l),
            type_oid_display_name(r)
        )))
    };
    // Array concatenation in all three shapes: Postgres-legal when the element type
    // lines up, unimplemented in pglet. A mismatched element type (`int[] || text`)
    // has no operator at all and falls through to `42883`.
    if l.is_array() && r.is_array() {
        if l == r {
            return unimplemented();
        }
    } else if l.is_array() && l.array_element() == Some(r)
        || r.is_array() && r.array_element() == Some(l)
    {
        return unimplemented();
    } else if matches!(
        (l, r),
        (TypeOid::Bytea, TypeOid::Bytea) | (TypeOid::Jsonb, TypeOid::Jsonb)
    ) {
        // `bytea || bytea` (byte concat) and `jsonb || jsonb` (merge) likewise exist
        // in Postgres and are unimplemented here.
        return unimplemented();
    } else if (is_string(l) || is_string(r)) && !l.is_array() && !r.is_array() {
        // `text || text` and `text || anynonarray` / `anynonarray || text`.
        // `anynonarray` excludes arrays — the guard keeps a surviving array operand
        // (whose element type didn't match above) out of the stringifying path.
        return Ok(());
    }
    Err(concat_undefined(left, right))
}

/// The "operator does not exist" error (`42883`) for a `||` whose operand pair
/// matches no Postgres `||` signature.
#[must_use]
pub fn concat_undefined(left: Option<TypeOid>, right: Option<TypeOid>) -> Error {
    Error::UndefinedFunction(format!(
        "operator does not exist: {} || {}",
        opt_type_oid_display_name(left),
        opt_type_oid_display_name(right)
    ))
}

/// A `LIKE`-family operator (`symbol` = `~~` / `!~~` / `~~*` / `!~~*`).
///
/// Postgres registers these over the string types — `text`, `varchar`, `bpchar`,
/// `name` (all `Value::Text` in pglet) — so any other operand resolves to no
/// operator (`42883`). A wrong-typed operand must **fail loud** rather than
/// silently not matching: a permissive `false` made `int_col NOT LIKE 'x'` return
/// true for every row.
///
/// `bytea ~~ bytea` / `bytea !~~ bytea` are the one exception: Postgres *does*
/// define them (byte-wise, `%` = `0x25`, `_` = `0x5F`) and pglet's matcher works
/// over `&str`, so they are unimplemented rather than nonexistent — `0A000`, not
/// `42883`. There is no `bytea ~~* bytea` in Postgres, so case-insensitive bytea
/// stays `42883` like every other mismatch.
///
/// # Errors
///
/// [`Error::NotImplemented`] for the `bytea` forms; [`Error::UndefinedFunction`]
/// for every other non-string operand pair.
pub fn like_legality(left: Option<TypeOid>, right: Option<TypeOid>, symbol: &str) -> Result<()> {
    match (left, right) {
        (None, _) | (_, None) => Ok(()),
        (Some(l), Some(r)) if is_string(l) && is_string(r) => Ok(()),
        (Some(TypeOid::Bytea), Some(TypeOid::Bytea)) if matches!(symbol, "~~" | "!~~") => Err(
            Error::NotImplemented(format!("operator bytea {symbol} bytea is not supported")),
        ),
        _ => Err(Error::UndefinedFunction(format!(
            "operator does not exist: {} {symbol} {}",
            opt_type_oid_display_name(left),
            opt_type_oid_display_name(right)
        ))),
    }
}

/// `starts_with(value, prefix)` — Postgres has exactly one signature,
/// `starts_with(text, text)`, so a non-text argument on either side fails loud
/// rather than silently not matching.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] when either argument is not a string type.
pub fn starts_with_legality(left: Option<TypeOid>, right: Option<TypeOid>) -> Result<()> {
    match (left, right) {
        (None, _) | (_, None) => Ok(()),
        (Some(l), Some(r)) if is_string(l) && is_string(r) => Ok(()),
        _ => Err(Error::UndefinedFunction(format!(
            "function starts_with({}, {}) does not exist",
            opt_type_oid_display_name(left),
            opt_type_oid_display_name(right)
        ))),
    }
}

/// `json -> key` / `json ->> key`.
///
/// Postgres registers exactly `(jsonb, text)` and `(jsonb, integer)`, so an `int2`
/// widens to `int4` and is accepted, while `int8`, `numeric`, `float8`, `bool`, …
/// resolve to no operator. `int8` is deliberately **not** accepted — Postgres has
/// no `(jsonb, bigint)` signature, so accepting it would let a query pass here that
/// errors on the engine pglet conforms to.
///
/// On the **left**, `text` is a pglet leniency (the text is parsed as JSON);
/// Postgres defines `->` only over json/jsonb. Every other left operand fails loud.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] for any other operand pair.
pub fn json_get_legality(left: Option<TypeOid>, key: Option<TypeOid>, as_text: bool) -> Result<()> {
    let undefined = || {
        Error::UndefinedFunction(format!(
            "operator does not exist: {} {} {}",
            opt_type_oid_display_name(left),
            if as_text { "->>" } else { "->" },
            opt_type_oid_display_name(key)
        ))
    };
    if let Some(l) = left
        && !matches!(l, TypeOid::Jsonb | TypeOid::Text | TypeOid::Varchar)
    {
        return Err(undefined());
    }
    if let Some(k) = key
        && !matches!(
            k,
            TypeOid::Text | TypeOid::Varchar | TypeOid::Int2 | TypeOid::Int4
        )
    {
        return Err(undefined());
    }
    Ok(())
}

/// The Postgres operator symbol a **jsonb key/path** `BinaryOp` resolves through, and
/// whether its right operand is the `text[]` form — for both the legality rule and the
/// error message.
///
/// The five operators here are the ones Postgres registers with a *fixed* right operand
/// (`jsonb ? text`, `jsonb ?| text[]`, `jsonb ?& text[]`, `jsonb #> text[]`,
/// `jsonb #>> text[]`), which is why one rule serves all of them; `->` / `->>` take two
/// right forms and keep [`json_get_legality`].
///
/// Returns `None` for every other operator, which is how the two dispatchers decide
/// whether a node is jsonb-key-shaped at all.
#[must_use]
pub const fn json_operator_symbol(op: BinaryOp) -> Option<(&'static str, bool)> {
    Some(match op {
        BinaryOp::JsonExists => ("?", false),
        BinaryOp::JsonExistsAny => ("?|", true),
        BinaryOp::JsonExistsAll => ("?&", true),
        BinaryOp::JsonPathGet => ("#>", true),
        BinaryOp::JsonPathGetText => ("#>>", true),
        _ => return None,
    })
}

/// `jsonb ? key` / `?|` / `?&` / `#>` / `#>>` — the jsonb operators whose right operand
/// Postgres registers as one fixed type.
///
/// Postgres registers exactly `(jsonb, text)` for `?` and `(jsonb, text[])` for the
/// other four (plus the `json` left form, which pglet models no type for, and `?|`'s
/// three geometric forms). Every other operand pair resolves to **no operator**, so a
/// scalar right operand — `'{"a":1}'::jsonb ? 1` — is `42883` there, and here through
/// this rule, which each eval arm calls before it reads an operand.
///
/// The **leniencies are [`json_get_legality`]'s, deliberately**: `text`/`varchar` on the
/// left (parsed as JSON by the eval arm) and, for the `text[]` forms, a `text` right
/// operand (parsed as a Postgres array literal). Both are divergences pglet already
/// carries on `->`, and the rule that gates this family reproduces them rather than
/// closing one operator's half of a divergence and leaving its neighbour's open.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] for any other operand pair.
pub fn json_operator_legality(
    left: Option<TypeOid>,
    right: Option<TypeOid>,
    symbol: &str,
    right_is_array: bool,
) -> Result<()> {
    let undefined = || {
        Error::UndefinedFunction(format!(
            "operator does not exist: {} {symbol} {}",
            opt_type_oid_display_name(left),
            opt_type_oid_display_name(right)
        ))
    };
    if let Some(l) = left
        && !matches!(l, TypeOid::Jsonb | TypeOid::Text | TypeOid::Varchar)
    {
        return Err(undefined());
    }
    if let Some(r) = right {
        let ok = matches!(r, TypeOid::Text | TypeOid::Varchar)
            || (right_is_array && matches!(r, TypeOid::TextArray | TypeOid::VarcharArray));
        if !ok {
            return Err(undefined());
        }
    }
    Ok(())
}

/// The Postgres operator symbol a containment `BinaryOp` resolves through, for both
/// the legality rule and the error message.
///
/// Unlike the `LIKE` and comparison families, the two spellings are **not**
/// interchangeable: the executor evaluates `<@` by swapping the operands into the
/// `@>` code, so the symbol is the only thing that still knows which one the user
/// wrote. Reporting `text @> text` for a `<@` query would name an operator that is
/// not in it.
///
/// Returns `None` for every other operator, which is how the DDL-time walker decides
/// whether a node is containment-shaped at all.
#[must_use]
pub const fn containment_symbol(op: BinaryOp) -> Option<&'static str> {
    Some(match op {
        BinaryOp::Containment => "@>",
        BinaryOp::ContainedBy => "<@",
        _ => return None,
    })
}

/// `a @> b` / `a <@ b` — containment, over the operands **as written** (the caller
/// has not swapped them).
///
/// Postgres registers containment over container types only: `jsonb @> jsonb` and
/// `anyarray @> anyarray` (plus the geometric, range, and `tsquery` families pglet
/// models no type for). So a scalar operand resolves to **no operator** and is
/// `42883`, exactly as `text @> text` is in Postgres — not `0A000`, which this
/// module reserves for an operator Postgres *has* and pglet has not implemented.
///
/// The two container families never cross: Postgres has no `jsonb @> anyarray`, so
/// a mixed pair is undefined too. Two arrays must be the **same** array type —
/// `anyarray` is resolved per-call, not unified across the operands, so Postgres
/// rejects even `integer[] @> bigint[]` at parse time rather than widening.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] (`42883`) for any operand pair that is not
/// jsonb-vs-jsonb or two arrays of the same element type.
pub fn containment_legality(
    left: Option<TypeOid>,
    right: Option<TypeOid>,
    symbol: &str,
) -> Result<()> {
    let (Some(l), Some(r)) = (left, right) else {
        return Ok(()); // an unknown operand is always accepted — see the module doc
    };
    if (l == TypeOid::Jsonb && r == TypeOid::Jsonb) || (l.is_array() && l == r) {
        return Ok(());
    }
    Err(Error::UndefinedFunction(format!(
        "operator does not exist: {} {symbol} {}",
        opt_type_oid_display_name(left),
        opt_type_oid_display_name(right)
    )))
}

/// A projection-position `unnest`'s argument, against Postgres's `unnest(anyarray)`
/// signature.
///
/// The overload set is read off the oracle rather than remembered:
/// `SELECT oid::regprocedure FROM pg_proc WHERE proname = 'unnest'` answers **three** on
/// 18.6 — `unnest(anyarray)`, `unnest(anymultirange)` and `unnest(tsvector)` — and pglet
/// models neither of the last two, so over the types it *does* model **every non-array
/// argument is `42883`**: `unnest(integer)`, `unnest(text)`, `unnest(jsonb)`,
/// `unnest(uuid)` and the rest, measured one per type on 18.6. (A `DOMAIN` over an array
/// is 18.6's fourth answer — it accepts — and pglet has no `CREATE DOMAIN`, so no
/// argument reaches here carrying one either.)
///
/// That is a signature refusal and nothing about rows: it is raised in
/// `check_expr_node_semantics`'s one plan-time walk, so it fires over an empty table,
/// over `WHERE false`, and inside a `CASE` branch nothing takes, exactly as 18.6 raises
/// it in parse analysis (measured: `Parse` on the extended leg, `E Z` with no
/// `ParseComplete`, on both engines).
///
/// **`None` is not "accept anything", it is "this is not the shape to refuse here".**
/// Three argument shapes reach it as `None`, for three different reasons, and each leaves
/// a residual `EDGES.md` row 55 records. All three share one polarity rule, which is the
/// closed half's exactly inverted: in a query position each is silent at `(0 rows)` and
/// `0A000` — the scalar-position refusal — once a row reaches the leaf, so a *no-FROM*
/// spelling of any of them reads "loud" only because it has a row by construction.
///
/// - a shape whose type is a **guess** for the caller's reader (`arg_known_type`) defers:
///   a computed expression, an unbound `$N`, an `ARRAY[…]` constructor. The list is
///   re-derived from the `Expr` variants that reader's `_` arm swallows rather than from
///   a remembered handful — `CASE`, `NULLIF`, `greatest`/`least` and a unary `-` are
///   members as much as `i + 0` is;
/// - a bare string or `NULL` literal is Postgres's **`unknown`** at this position, where
///   18.6 answers `42725 function unnest(unknown) is not unique` rather than `42883` — a
///   different code, and one pglet does not raise, so refusing it would trade a recorded
///   divergence for a differently-wrong one;
/// - an `Expr::Bound` constant **and an `Expr::Alias`** are invisible to that reader while
///   `certain_operand_type` types them both — row 65's boundary, not a decision here. So
///   `unnest(INTERVAL '1 day')` stays at pglet's own `0A000` where 18.6 is `42883`, and
///   `unnest(set_config('x', tx, false))`, which lowers to an `Alias` over its `value`
///   argument, answers where 18.6 is `42883 function unnest(text) does not exist`
///   (measured on both).
///
/// **The message names the type through `type_oid_display_name`, which is pglet's type
/// model and not 18.6's catalog.** Measured by sweeping the column-type spellings
/// `executor::resolve_column_type` accepts — each arm of `type_oid_from_name`, the
/// parser's `dec` and multiword forms, each length/precision modifier spelling, and a
/// user enum, one `CREATE TABLE` and one `unnest(c)` apiece against 18.6 — the model's
/// base types (`boolean`, `smallint`, `integer`, `bigint`, `real`, `double precision`,
/// `text`, `character varying`, `bytea`, `timestamp without time zone`,
/// `timestamp with time zone`, `date`, `time without time zone`, `interval`, `numeric`,
/// `uuid`, `jsonb`) match 18.6's spelling exactly, and so do the `int2`/`int4`/`int8`,
/// `serial`-family and `decimal` aliases, `varchar(n)`, the `timestamp(n)` /
/// `time(n)` / `interval(n)` precisions this parser keeps, and the `real` /
/// `double precision` a `FLOAT(n)` selects. What the sweep found parting:
///
/// - `char(n)` — and the parenless `char` / `character` — is `Varchar` here, so
///   `unnest(c)` over a `CHAR(3)` column reads
///   `function unnest(character varying) does not exist` where 18.6 says `character`.
///   The message follows a **substituted type**, not a relabelled one: 18.6's `bpchar`
///   blank-pads and compares blank-insensitively and this does neither (`EDGES.md`
///   row 95);
/// - an **enum** is text plus a catalog member set, so `unnest(<enum col>)` reads
///   `function unnest(text) does not exist` where 18.6 names the enum
///   (`function unnest(vis) does not exist`). This one is the message alone, and the
///   reason is this function's **signature** rather than the engine's model: the identity
///   is reachable — `comparison_legality` above takes a `ResolvedType` that carries it —
///   and `unnest_legality` deliberately stays on `Option<TypeOid>`, its rule being a
///   signature one with no enum verdict to give.
///
/// Same SQLSTATE and same shape refused in each. That sweep's domain is **column-type
/// spellings**, which is not the only way a type reaches this message — a bound
/// parameter's declared OID and a computed expression's result type arrive by other
/// routes and were not swept — so re-run it against the position at hand rather than
/// reading the list as closed. Being outside the base-type name list is not itself
/// parting: the declaration-only kinds a `pg_catalog` relation carries agree,
/// `unnest(conrelid)`, `unnest(contype)` and `unnest(typname)` reading `unnest(oid)`,
/// `unnest("char")` and `unnest(name)` byte-identically on the two engines.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] when `arg` is a known non-array type.
pub fn unnest_legality(arg: Option<TypeOid>) -> Result<()> {
    let Some(observed) = arg else {
        return Ok(());
    };
    if observed.array_element().is_some() {
        return Ok(());
    }
    Err(Error::UndefinedFunction(format!(
        "function unnest({}) does not exist",
        type_oid_display_name(observed)
    )))
}

/// A **boolean context**'s operand, which must be `boolean` — the one plan-time gate
/// behind 18.6's `coerce_to_boolean`.
///
/// 18.6 attempts a `COERCION_ASSIGNMENT` coercion to `boolean` there, and `pg_cast`
/// holds exactly two rows into `boolean` (`integer` and `jsonb`), **both
/// explicit-context**, so the rule is total and short: `boolean` itself, or
/// Postgres's `unknown`, and nothing else. That is exactly
/// [`assignment_compatible`] evaluated with `boolean` as the target, which is what
/// this asks — a second predicate here would be a second rulebook for one
/// `pg_cast` question.
///
/// `operand` is the type when it is knowable and `None` when it is not — an unbound
/// `$N`, or Postgres's `unknown` (a bare string / `NULL` literal), which this
/// position *resolves* to boolean rather than rejecting: `'t' IS TRUE` is `t` on
/// 18.6 and `'x' IS TRUE` is `22P02` from the input conversion, not `42804`. A
/// `None` therefore defers to the eval-time gate
/// (`executor::expr`'s `boolean_context_operand`), which sees the value.
///
/// The message names the construct, as 18.6 does — `argument of IS NOT TRUE must be
/// type boolean, not type integer` — which is why [`BoolContext`] carries its own
/// SQL text rather than the caller reconstructing it.
///
/// # Errors
///
/// [`Error::Type`] (`42804`) when `operand` is a known non-boolean type.
pub fn boolean_context_legality(operand: Option<TypeOid>, ctx: BoolContext) -> Result<()> {
    let Some(observed) = operand else {
        return Ok(());
    };
    if assignment_compatible(observed, TypeOid::Bool) {
        return Ok(());
    }
    Err(Error::Type(format!(
        "argument of {} must be type boolean, not type {}",
        ctx.sql(),
        type_oid_display_name(observed)
    )))
}

/// The **conversion** half of a boolean context.
///
/// For the `unknown` [`boolean_context_legality`] deliberately lets through: a bare
/// string literal is converted here, through the same `boolean` input function a
/// `::bool` cast takes, so `WHERE 't'` is a row and `WHERE 'abc'` is `22P02`.
///
/// It is plan-time rather than eval-time because 18.6 runs it in parse analysis: the
/// eval-time resolver never reaches a filter over an empty table, so `WHERE 'abc'`
/// there would be a silent zero rows against 18.6's error. For a stored expression it
/// is load-bearing for a second reason — a `CHECK ('abc')` accepted here dumps to a
/// definition stock Postgres refuses to load.
///
/// # Errors
///
/// [`Error::InvalidInputSyntax`] (`22P02`) for a literal that is not a boolean spelling.
pub fn boolean_context_literal(expr: &Expr) -> Result<()> {
    if let Some(literal) = unknown_literal_spelling(expr) {
        crate::executor::text_to_bool(literal)?;
    }
    Ok(())
}

/// The whole plan-time verdict for one boolean operand.
///
/// [`boolean_context_legality`] and [`boolean_context_literal`] over `expr`, read
/// through [`unknown_aware_type`] against `scope` — in one call, so no caller can
/// take half of it.
///
/// # Errors
///
/// [`Error::Type`] (`42804`) for a non-boolean operand; [`Error::InvalidInputSyntax`]
/// (`22P02`) for an unresolved literal that is not a boolean spelling.
pub fn check_boolean_operand(expr: &Expr, ctx: BoolContext, scope: &dyn LeafScope) -> Result<()> {
    boolean_context_legality(unknown_aware_type(expr, scope), ctx)?;
    boolean_context_literal(expr)
}

/// A value aggregate's operand, against Postgres's accept-list.
///
/// These accept-lists are **hard-coded catalog facts** (one `pg_proc` entry per
/// input type), not derivable from "is this type comparable": `bool`, `jsonb`, and
/// `uuid` all have complete comparison operators, yet Postgres 18 registers no
/// `min`/`max` for any of them. So they are spelled out literally.
///
/// # Errors
///
/// [`Error::UndefinedFunction`] when the aggregate has no signature for `arg`.
pub fn aggregate_legality(func: AggregateFunc, arg: Option<TypeOid>) -> Result<()> {
    let Some(observed) = arg else {
        return Ok(());
    };
    let (name, accepted) = match func {
        // Every row counts regardless of type.
        AggregateFunc::Count => return Ok(()),
        // `sum`/`avg` are the numeric family plus `interval`. (Postgres also has
        // `sum(money)` — a type pglet doesn't model — and no `avg(money)`.)
        AggregateFunc::Sum | AggregateFunc::Avg => (
            if matches!(func, AggregateFunc::Sum) {
                "sum"
            } else {
                "avg"
            },
            matches!(
                observed,
                TypeOid::Int2
                    | TypeOid::Int4
                    | TypeOid::Int8
                    | TypeOid::Float4
                    | TypeOid::Float8
                    | TypeOid::Numeric
                    | TypeOid::Interval
            ),
        ),
        // `min`/`max` are registered for every sortable type Postgres 18 has *except*
        // `bool`, `jsonb`, and `uuid`, so the reject-list is the faithful encoding.
        // Version-sensitive: the 20devel source tree adds `min(uuid)`/`max(uuid)`, which
        // the 18.x oracle the fixtures compare against does not have — the oracle wins.
        AggregateFunc::Min | AggregateFunc::Max => (
            if matches!(func, AggregateFunc::Min) {
                "min"
            } else {
                "max"
            },
            !matches!(observed, TypeOid::Bool | TypeOid::Jsonb | TypeOid::Uuid),
        ),
    };
    if accepted {
        Ok(())
    } else {
        Err(Error::UndefinedFunction(format!(
            "function {name}({}) does not exist",
            type_oid_display_name(observed)
        )))
    }
}

/// Postgres's **string type category** (`TYPCATEGORY_STRING`) among the types pglet
/// models — what `LIKE` / `starts_with` accept, what compares against its own
/// category, and what everything assigns into.
///
/// pglet models `text`, `varchar` and `char(n)` all as `Value::Text`, and `varchar`
/// is the only one of those with a distinct `TypeOid`. `name` is the fourth member
/// and the one with no column form: it is only ever a *declared* type
/// (`current_user`, `current_schema`, `current_database()`), so it appears here as a
/// source and never as an assignment target. Measured on 18.6, all of which this
/// membership is what makes work: `current_user LIKE 'post%'`,
/// `starts_with(current_user, 'po')`, `owner = current_user` over a `text` column,
/// `current_user < 'z'`, `coalesce`/`greatest` against a `text` literal, and
/// `INSERT INTO t(txt) VALUES (current_user)`. And what it must *not* make work,
/// also measured: `current_user = 1` and `current_user + 1` are `42883` there, and
/// `INSERT INTO t(int_col) VALUES (current_user)` is `42804` — none of which this
/// predicate reaches, since it is the string category and not a wildcard.
///
/// `"char"` joins for the same reason and on the same measurements: 18.6 resolves
/// `contype = 'n'`, `contype > 'a'`, `contype LIKE 'n'` and `upper(contype)`, and
/// refuses `contype = 1`. One member of the family diverges — `contype || 'x'` is
/// `operator is not unique` there and concatenates here — recorded in `EDGES.md`
/// rather than carved out, because carving it out would need a second category
/// predicate for a shape no consumer writes.
const fn is_string(oid: TypeOid) -> bool {
    matches!(
        oid,
        TypeOid::Text | TypeOid::Varchar | TypeOid::Name | TypeOid::Char
    )
}

/// Whether `expr`'s `COALESCE` / `CASE` / `ARRAY` branches have a Postgres common
/// type — the construct-level half of the rulebook, beside the operand-level half in
/// [`check_expr_node_operand_types`].
///
/// `ty` resolves a branch's type and `defer` reports a branch whose type this caller
/// cannot know yet (a correlated column the scope hasn't been given). Both are
/// closures because the two callers resolve differently — the planner against a
/// `ColumnScope`, the DDL-time stored-expression checker against the catalog — while
/// the *rule* must stay single-sourced. A deferred branch set is skipped entirely: an
/// enclosing query level still has to resolve it, and judging it here would reject a
/// correlated subquery that is perfectly well-typed one level out.
///
/// Nested arrays are rejected here too (`0A000`): pglet's arrays are one-dimensional
/// by construction, and the alternative to rejecting is the silent 1-D-array-of-text
/// reinterpretation this replaced. That verdict is reached **after** the common-type
/// fold, never before it — see the `ARRAY` arm.
///
/// # Errors
///
/// `42804` when two branches are in different type categories, `42846` when they share
/// one but no conversion exists, `0A000` for a nested `ARRAY`.
pub fn branch_unification_legality(
    expr: &Expr,
    ty: &dyn Fn(&Expr) -> Option<ResolvedType>,
    defer: &dyn Fn(&Expr) -> bool,
    schema: &crate::catalog::Schema,
) -> Result<()> {
    let Some((branches, context)) = unification_branches(expr) else {
        return Ok(());
    };
    if branches.iter().any(|e| defer(e)) {
        return Ok(());
    }
    let types: Vec<Option<ResolvedType>> = branches.iter().map(|e| ty(e)).collect();
    let resolved = crate::planner::select_common_type(&types, context)?;
    // **After** the fold, because 18.6 coerces each branch to the type
    // `select_common_type` already picked: `GREATEST(m, s, 'zzz')` is the fold's
    // `42804` and not the literal's `22P02`, in either argument order, on both
    // engines. A target that names an enum is the only one this reaches — see
    // [`enum_literal_legality`].
    if let Some(name) = resolved.as_ref().and_then(|t| t.enum_type.name()) {
        for branch in &branches {
            enum_literal_legality(name, branch, schema)?;
        }
    }
    // **After** the fold, never before it. Postgres resolves `ARRAY[…]` through
    // `select_common_type` like every other construct, so an element set with no common
    // type is that fold's `42804`/`42846` whether or not one element happens to be an
    // array: `ARRAY[bool_col, int_array_col]` is "ARRAY types boolean and integer[]
    // cannot be matched", and `ARRAY[int_array_col, text_array_col]` is "ARRAY could not
    // convert type text[] to integer[]". Deciding `is_array` per element first reported
    // a nesting complaint for every such pair.
    //
    // Nesting is what is left once unification *succeeds* and lands on an array type —
    // `ARRAY[ARRAY[1,2], ARRAY[3,4]]`, which Postgres accepts as two-dimensional and
    // pglet does not model.
    if matches!(expr, Expr::Array(_)) && resolved.is_some_and(|t| t.oid.is_array()) {
        return Err(Error::NotImplemented(
            "multidimensional arrays are not supported".into(),
        ));
    }
    Ok(())
}

/// The branches [`branch_unification_legality`] folds, and the construct word 18.6
/// names in its message — `None` for a node that is no such position.
///
/// The **one** enumeration of these five constructs, so a caller that must decide
/// something about the branch set as a whole (the plan-time gate's reader choice)
/// reads the same list the rule folds rather than a second one that must agree.
///
/// `GREATEST`/`LEAST` are in it for a reason the other three do not need: they resolve
/// through the same `select_common_type` (`transformMinMaxExpr`), but their
/// *eval-time* fold drops NULL arguments before folding, so a row where the offending
/// argument happens to be NULL produced an answer instead of an error —
/// `GREATEST(int_col, text_col)` over `(7, NULL)` returned `7` where Postgres reports
/// `42804` regardless of the data. Static resolution is the only thing that sees it,
/// which also makes it right on an empty table, and stops a
/// `CHECK (GREATEST(t, i) IS NOT NULL)` being recorded and then failing every write
/// that supplies both columns.
#[must_use]
pub fn unification_branches(expr: &Expr) -> Option<(Vec<&Expr>, &'static str)> {
    match expr {
        Expr::Coalesce(args) => Some((args.iter().collect(), "COALESCE")),
        Expr::Array(elements) => Some((elements.iter().collect(), "ARRAY")),
        Expr::Case {
            when_clauses,
            else_result,
            ..
        } => {
            // ELSE first — Postgres prepends `defresult` before resolving.
            let mut branches: Vec<&Expr> = Vec::with_capacity(when_clauses.len() + 1);
            branches.extend(else_result.as_deref());
            branches.extend(when_clauses.iter().map(|(_, r)| r));
            Some((branches, "CASE"))
        }
        Expr::VariadicFn {
            func: func @ (VariadicFunc::Greatest | VariadicFunc::Least),
            args,
        } => Some((
            args.iter().collect(),
            if matches!(func, VariadicFunc::Greatest) {
                "GREATEST"
            } else {
                "LEAST"
            },
        )),
        _ => None,
    }
}

/// Statically type-check **one node**'s operands against `schema` + `current_table`.
///
/// Only a gated operator or aggregate has anything to check; every other node is
/// `Ok`. Sub-expressions are not visited — the caller drives the traversal.
///
/// This is the DDL-time half of the shared rules above — the half that catches what
/// a value probe structurally cannot. A probe evaluates one synthetic row, so any
/// intermediate that yields SQL NULL makes every downstream gate NULL-propagate and
/// never fire: `CREATE INDEX … WHERE data->'k' LIKE 'x%'` probes as
/// `NULL LIKE 'x%'` for *every* representative `data`, because `->` on an object
/// without key `k` is NULL — and the probe cannot know which key to synthesize.
/// Static inference has no such hole: `data->'k'` is `jsonb` whatever the value is,
/// and `jsonb ~~ text` resolves to no operator.
///
/// Errors propagate with the rule's own SQLSTATE (`42883` / `0A000`), which is
/// where Postgres reports them too — at `CREATE INDEX` / `CREATE TABLE` /
/// `ALTER TABLE`, not on some later write to a now-unwritable table.
///
/// # Node-local on purpose
///
/// `executor::ddl` walks a stored expression **bottom-up, left-to-right**,
/// interleaving this verdict at each node with its own DDL-specific binding check
/// (`DEFAULT` / `$N` / a column the table does not have). That interleaving is only
/// expressible as one traversal: run as two whole-tree passes, whichever pass went
/// first would win regardless of *where* in the expression each error sits, and
/// Postgres resolves strictly in source order during parse analysis — `CHECK (a LIKE
/// 'x' AND bogus > 0)` is `42883` while `CHECK (bogus > 0 AND a LIKE 'x')` is
/// `42703`, and no pass ordering yields both. So the recursion lives with the walk
/// and this module stays a pure per-node rulebook, with nothing DDL-specific in it.
///
/// # Errors
///
/// Whatever legality rule this node violates.
pub fn check_expr_node_operand_types(
    expr: &Expr,
    schema: &crate::catalog::Schema,
    current_table: Option<&str>,
) -> Result<()> {
    let scope = PlannerScope::catalog(schema, current_table);
    let ty = |e: &Expr| e.infer_type(&scope);
    // The **comparison** rulebook reads the enum identity beside the OID, and at DDL
    // time that identity comes from the catalog through the same exhaustive walk the
    // plan-time gate uses ([`PlannerScope::expr_enum`]). Every other rule here takes the
    // OID alone, so `ty` stays and this rides beside it.
    let rty = |e: &Expr| scope.resolved_type(e);
    // The operator's legality and then the `unknown` side's conversion to what the
    // operator resolved — one helper rather than two calls per arm, so the plan-time
    // twin and this one cannot come to hold different halves of the pair.
    let compare = |l: &Expr, r: &Expr, symbol: &str| -> Result<()> {
        let (lt, rt) = (rty(l), rty(r));
        comparison_legality(lt.clone(), rt.clone(), symbol)?;
        comparison_enum_literal_legality(lt.as_ref(), rt.as_ref(), l, r, schema)
    };
    match expr {
        Expr::BinaryOp { left, op, right } => {
            if let Some(symbol) = like_symbol(*op) {
                like_legality(ty(left), ty(right), symbol)?;
            } else if let Some(symbol) = regex_symbol(*op) {
                regex_legality(rty(left).as_ref(), rty(right).as_ref(), symbol)?;
            } else if let Some(symbol) = comparison_symbol(*op) {
                compare(left, right, symbol)?;
            } else if let Some(symbol) = containment_symbol(*op) {
                containment_legality(ty(left), ty(right), symbol)?;
            } else {
                match op {
                    BinaryOp::Concat => concat_legality(ty(left), ty(right))?,
                    _ if arithmetic_symbol(*op).is_some() => {
                        // Unreachable through a stored expression today — no DDL
                        // spelling produces an `oid` / `"char"` operand — but this is
                        // the rulebook's plan-time half and it must call the same rule
                        // the eval-time half does, or the two drift the moment a
                        // catalog type becomes reachable from one.
                        arithmetic_legality(ty(left), ty(right), *op)?;
                    }
                    BinaryOp::JsonGet => json_get_legality(ty(left), ty(right), false)?,
                    BinaryOp::JsonGetText => json_get_legality(ty(left), ty(right), true)?,
                    // Measured **redundant** at every stored-expression position a
                    // fixture reaches: a `CHECK` over a jsonb operator is refused by the
                    // plan-time pass or by the value probe's eval-time gate whichever way
                    // this arm is removed, so no fixture reddens on removing it alone —
                    // the standing of the `arithmetic_legality` arm above. It is here for
                    // the same reason: this is the rulebook's DDL-time half, and a half
                    // that calls a different rule drifts the moment one of the other
                    // callers stops reaching a position.
                    _ if json_operator_symbol(*op).is_some() => {
                        let (symbol, array) = json_operator_symbol(*op).unwrap_or(("?", false));
                        json_operator_legality(ty(left), ty(right), symbol, array)?;
                    }
                    BinaryOp::StartsWith => starts_with_legality(ty(left), ty(right))?,
                    _ => {}
                }
            }
        }
        Expr::Negate(inner) => negate_legality(ty(inner))?,
        Expr::Aggregate { func, arg, .. } => aggregate_legality(*func, ty(arg))?,
        // The DDL-time half of the constant enum cast, the plan-time half being
        // `validate_const_casts`: a stored `DEFAULT`, `CHECK` or index expression holding
        // `'q'::mq` is refused at the DDL over zero rows, where the value probe beside this
        // walk swallows a `22P02` as a value-dependent outcome.
        Expr::StringFn {
            func: super::StringFunc::EnumIn,
            ..
        } => enum_in_constant_legality(expr)?,
        // A simple `CASE x WHEN v` label and an `IN` element are both Postgres's `x = v`,
        // and they are here because the plan-time walk has them: the two callers of this
        // rulebook must judge one arm list, or a `CHECK` is judged by a rule the query
        // over the same columns is not. They were missing, and what that cost is the
        // persisted half — `CHECK (m IN ('ok','zzz'))` and `CHECK (CASE m WHEN s THEN …)`
        // were recorded here and refused there, so the catalog dumped to SQL stock 18.6
        // will not load. The **value probe** beside this walk hid it for every pair whose
        // operands differ in shape (`i IN (s)` is its `42883`) and could not for an enum,
        // whose value is a `Value::Text` like the `text` it is being compared to.
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
        // The operand type is read through the unknown-aware reader, not `ty`: at
        // this position Postgres *resolves* an `unknown` to boolean instead of
        // refusing it, so `CHECK ('t' IS TRUE)` is accepted on 18.6 (and stored
        // folded to `CHECK ((true IS TRUE))`) while `CHECK ('x' IS TRUE)` is the
        // input conversion's `22P02`. Typing the literal `text` here would answer
        // `42804` for both.
        // …and, for the `unknown` the gate deliberately lets through, the conversion
        // itself. A stored expression is validated once, at DDL time, and never
        // again: without it `CHECK ('x' IS TRUE)` is accepted here and `22P02` on
        // 18.6, and the accepted catalog then **dumps to a definition stock Postgres
        // refuses to load** — the invariant a persisted expression exists under.
        //
        // The four *interleaved* contexts (`AND`/`OR`/`NOT`/searched `CASE WHEN`) are
        // not here: 18.6 coerces each of their arguments as it transforms it, before
        // the next argument is transformed at all, so their gate belongs to the walk
        // rather than to a per-node rulebook — see `Expr::for_each_child_ctx`.
        Expr::BooleanTest { expr: inner, test } => {
            check_boolean_operand(inner, BoolContext::Test(*test), &scope)?;
        }
        // `NULLIF` resolves an `=` operator, so a pair with none is `42883` here too —
        // kept in step with the plan-time walk, which drives this same rulebook.
        Expr::VariadicFn {
            func: VariadicFunc::NullIf,
            args,
        } => {
            if let [a, b] = args.as_slice() {
                compare(a, b, "=")?;
            }
        }
        // `x op ANY/ALL (array)` is `make_scalar_array_op`, resolving `op` between the
        // left-hand type and the array's **element** type — and it is a position a
        // stored expression reaches, which is why it is in *both* walks: `CHECK (m =
        // ANY(ARRAY[s]))` and the same predicate on a partial index were recorded here
        // and refused there, so the catalog dumped to SQL stock 18.6 will not load.
        // The `IN (SELECT …)` half of the plan-time walk's pair has no twin here:
        // a subquery is refused in every persisted-expression context by both engines
        // (`0A000 cannot use subquery in check constraint` / `… in index predicate`),
        // so no stored expression can carry one.
        Expr::AnyAll {
            left, op, right, ..
        } => {
            if let Some(symbol) = comparison_symbol(*op) {
                let element = rty(right).as_ref().and_then(array_element_resolved_type);
                one_sided_comparison_legality(rty(left), element.as_ref(), left, symbol, schema)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// An operand's type as this rulebook's callers read it, with Postgres's `unknown`
/// answered as `None`.
///
/// `Expr::infer_type` types a bare string literal `text`, which is right for a
/// projection's output column and wrong for an operand gate: at an operand position
/// a bare string / `NULL` is `unknown` and adopts the position's type.
///
/// It is generic in its [`LeafScope`] so the DDL-time walk (a [`PlannerScope`] over the
/// probe schema) and the plan-time one (a clause's `ColumnScope`) read an operand
/// through **one** derivation rather than two lists that must agree. The narrower
/// readers beside it — `planner::coercion`'s `arg_known_type` and `certain_operand_type`
/// — answer `None` for an operator result, which is right for a gate that must not
/// reject on a guess and wrong here: `WHERE k + 1` is `42804` on 18.6 whatever the row,
/// and `k + 1` is `integer` for certain.
pub fn unknown_aware_type(expr: &Expr, scope: &dyn LeafScope) -> Option<TypeOid> {
    match expr {
        Expr::Literal(Literal::String(_) | Literal::Null) => None,
        Expr::Alias { expr, .. } => unknown_aware_type(expr, scope),
        _ => expr.infer_type(scope),
    }
}

/// The **source spelling** of an `unknown` string literal operand, looking through
/// an alias — the shape whose conversion the position's target type decides. `None`
/// for a `NULL` literal (which converts to every type) and for anything with a type
/// of its own.
///
/// Two rules read it, because 18.6 resolves both through the same `coerce_type` of an
/// `unknown` `Const`: the boolean contexts ([`boolean_context_literal`], whose target
/// is `boolean`) and an enum target ([`enum_literal_legality`]). A persisted
/// expression must prove either conversion at DDL time or it dumps to SQL stock 18.6
/// refuses to load.
fn unknown_literal_spelling(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Literal(Literal::String(s)) => Some(s),
        Expr::Alias { expr, .. } => unknown_literal_spelling(expr),
        _ => None,
    }
}

/// 18.6's coercion of an `unknown` literal to an **enum** target — `enum_in`, which is
/// the whole of the `22P02`.
///
/// A fold or an operator resolution that lands on an enum does not leave the literal
/// alone: `coerce_type` runs the target's input function on the `Const`, so
/// `greatest(m, 'zzz')` is `invalid input value for enum mood: "zzz"` in parse
/// analysis, whatever the rows and whatever branch would have been taken. pglet holds
/// an enum value as a `Value::Text`, so the coercion *is* the membership test and
/// nothing else — but skipping it is not that coercion, which is what let a
/// non-member label reach a projection, a `text` column, and a persisted `CHECK` or
/// partial index whose dump then restored into stock 18.6 with neither, psql exiting
/// `0`.
///
/// `schema` is not optional: every caller of `resolve_expr_types_gated` passes it —
/// `resolve_expr_types_no_columns` included, the whole gate chain above this rule — so
/// an `Option` would stand for a case no statement reaches while offering a silent
/// decline to every future one. An enum the catalog **does not hold** is a different
/// decline and is still here: the conversion is left to the write path's own gate,
/// which only a `DROP TYPE` this engine does not yet refuse can reach (`2BP01` under a
/// dependent column on 18.6, accepted here; the repo's `EDGES.md` row 261 —
/// `docs/enums.md`).
///
/// Anything but an unresolved literal is `Ok` here, and the two shapes that reach it
/// are not one fact: a `::text` cast is `EDGES.md` row 104's undecidable operand, and
/// so is a **client-declared** `$N`, which is planned as such a cast. An
/// *undeclared* `$N` is neither — it is 18.6's `unknown`, whose conversion runs at
/// `Bind` on the value rather than at plan time on the node, and that is
/// [`bound_param_enum_legality`]'s.
///
/// # Errors
///
/// [`Error::InvalidEnumValue`] (`22P02`) for a literal the enum does not hold.
pub fn enum_literal_legality(
    enum_name: &str,
    operand: &Expr,
    schema: &crate::catalog::Schema,
) -> Result<()> {
    let Some(label) = unknown_literal_spelling(operand) else {
        return Ok(());
    };
    enum_label_legality(enum_name, label, schema)
}

/// The membership test itself — 18.6's `enum_in` over a **label**, with no `Expr`
/// around it.
///
/// The one predicate behind every conversion of an `unknown` to an enum, so the two
/// things that can arrive as one cannot come to be judged differently: a literal the
/// SQL text spells ([`enum_literal_legality`]) and a value the client binds
/// ([`bound_param_enum_legality`]). An enum the catalog does not hold leaves the
/// conversion to the write path's own gate rather than refusing here; that decline is
/// only reachable through a `DROP TYPE` this engine does not yet refuse (see
/// [`enum_literal_legality`]).
///
/// # Errors
///
/// [`Error::InvalidEnumValue`] (`22P02`) for a label the enum does not hold.
pub fn enum_label_legality(
    enum_name: &str,
    label: &str,
    schema: &crate::catalog::Schema,
) -> Result<()> {
    let Some(enum_type) = schema.enums.get(enum_name) else {
        return Ok(());
    };
    if enum_type.labels.iter().any(|l| l == label) {
        return Ok(());
    }
    Err(Error::InvalidEnumValue {
        enum_type: enum_name.to_string(),
        value: label.to_string(),
    })
}

/// The membership test of a cast to an enum — 18.6's `enum_in` over `label`.
///
/// It reads the member list a [`StringFunc::EnumIn`](crate::planner::StringFunc::EnumIn)
/// node carries rather than the catalog's, so it is decidable where no catalog is lent.
///
/// # Errors
///
/// [`Error::InvalidEnumValue`] (`22P02`) for a label `labels` does not hold.
pub fn enum_in_label_legality(enum_name: &str, labels: &[Expr], label: &str) -> Result<()> {
    if crate::planner::enum_in_holds(labels, label) {
        return Ok(());
    }
    Err(Error::InvalidEnumValue {
        enum_type: enum_name.to_string(),
        value: label.to_string(),
    })
}

/// A cast of a **string literal** to an enum, decided where 18.6 decides it.
///
/// That is parse analysis, whatever the rows, since an `unknown` constant is converted in
/// place by the target type's input function (`coerce_type` → `stringTypeDatum` →
/// `enum_in`).
///
/// One node rule with two walkers, the same pair every other constant cast has: the
/// plan-time [`validate_const_casts`](crate::planner::validate_const_casts) for a
/// statement's clauses, and the DDL-time [`check_expr_node_operand_types`] for a
/// `DEFAULT`, a `CHECK`, and an index's predicate and key — so `SELECT 'q'::mq FROM e`
/// over an empty `e`, `… WHERE false`, an untaken `CASE` branch and
/// `CREATE TABLE … CHECK (m <> 'q'::mq)` are each `22P02`, as on 18.6. Any other node, and
/// an `EnumIn` over a non-literal operand, is `Ok` here: that conversion runs on the
/// value, at evaluation (or, for an undeclared `$N`, at `Bind`).
///
/// # Errors
///
/// [`Error::InvalidEnumValue`] (`22P02`) for a literal the enum does not hold.
pub fn enum_in_constant_legality(expr: &Expr) -> Result<()> {
    match crate::planner::enum_in_parts(expr) {
        Some((Expr::Literal(Literal::String(label)), name, labels)) => {
            enum_in_label_legality(name, labels, label)
        }
        _ => Ok(()),
    }
}

/// [`enum_label_legality`] over a statement's **bound** parameters — `enum_in` where
/// 18.6 runs it, at `Bind`, once per value rather than once per row.
///
/// `enums` is `ParamTargets::enums`, the enum target plan time recorded for each `$N`;
/// `values` is what this host's decode produced for them, positionally. A `NULL`
/// parameter converts to every type and is passed over, as 18.6 passes over one
/// without calling the input function at all. A value that is not a `Value::Text` did
/// not arrive as 18.6's `unknown` and is not this rule's: a client-declared parameter
/// is planned as a cast and never carries an enum target in the first place.
///
/// Both hosts call this — the native server's `Bind` and `Database::execute_bound` —
/// because the target is a property of the statement and not of the transport, and a
/// gate on one host alone would leave the other writing rows the dump cannot restore.
///
/// # Errors
///
/// [`Error::InvalidEnumValue`] (`22P02`) for the first parameter holding a label its
/// target enum does not hold, in `$N` order.
pub fn bound_param_enum_legality(
    enums: &[Option<String>],
    values: &[Option<crate::types::Value>],
    schema: &crate::catalog::Schema,
) -> Result<()> {
    for (target, value) in enums.iter().zip(values) {
        let (Some(name), Some(crate::types::Value::Text(label))) = (target.as_ref(), value) else {
            continue;
        };
        enum_label_legality(name, label, schema)?;
    }
    Ok(())
}

/// [`enum_literal_legality`] at a **comparison** position, where the target is the
/// other operand's type.
///
/// 18.6 resolves the operator first and coerces the `unknown` side to what it
/// resolved, so this runs *after* [`comparison_legality`] and never instead of it:
/// `m > lower(s) AND m > 'zzz'` is the operator's `42883` and the reverse spelling is
/// this rule's `22P02`, each on both engines. Only a side whose identity is
/// [`EnumIdentity::Named`](crate::planner::EnumIdentity::Named) supplies a target — an
/// undecidable one falls back to the OID rule here exactly as it does there.
///
/// # Errors
///
/// [`Error::InvalidEnumValue`] (`22P02`) for a literal the enum does not hold.
pub fn comparison_enum_literal_legality(
    left: Option<&ResolvedType>,
    right: Option<&ResolvedType>,
    left_expr: &Expr,
    right_expr: &Expr,
    schema: &crate::catalog::Schema,
) -> Result<()> {
    if let Some(name) = left.and_then(|t| t.enum_type.name()) {
        enum_literal_legality(name, right_expr, schema)?;
    }
    if let Some(name) = right.and_then(|t| t.enum_type.name()) {
        enum_literal_legality(name, left_expr, schema)?;
    }
    Ok(())
}

/// The **element** type of an `op ANY`/`op ALL`'s right-hand array.
///
/// One array suffix off the OID, as `make_scalar_array_op` reads it, with the enum
/// identity already at the element's width (a named identity on an array
/// [`ResolvedType`] *is* the element's).
///
/// `None` for a type with no element, which defers: pglet refuses a non-array
/// right-hand side at eval time on its own account.
///
/// # The element's identity is the array's, and nothing here folds it
///
/// `transformArrayExpr` runs `select_common_type` over the elements *before*
/// `make_scalar_array_op` sees the array, so the element type this reads is already
/// settled and this function takes one array suffix off it. The identity beside it is
/// settled by the same fold, one layer down: an array's identity **is** its elements'
/// (`executor::expr`'s `Expr::Array` arm unifies them, and
/// [`erases_enum_identity`](super::coercion::erases_enum_identity)'s array arm asks
/// which of them can erase it), so there is no element rule here to state twice.
///
/// What that buys at this position, measured on both engines over a `mood` column `m`,
/// a `text` column `s` and a second enum `y`: `m = ANY(ARRAY['ok'])` and
/// `m = ANY(ARRAY[NULL])` are `42883 operator does not exist: mood = text` — an
/// all-`unknown` array is `text[]` and no enclosing position can retype it, where the
/// `m = 'ok'` and `m IN ('ok')` spellings of the same label are membership tests;
/// `ARRAY[s,'ok']` and `ARRAY[y,'ok']` refuse for the same reason one element decides
/// them (`mood = text`, `mood = m3`), an `unknown` beside a typed element contributing
/// nothing to the fold; and `ARRAY['ok'::mood]` **answers**, its one element being a
/// cast whose enum name the parser dropped (`EDGES.md` row 104), which leaves the array
/// undecidable rather than `text[]`.
#[must_use]
pub fn array_element_resolved_type(array: &ResolvedType) -> Option<ResolvedType> {
    Some(ResolvedType {
        oid: array.oid.array_element()?,
        enum_type: array.enum_type.clone(),
    })
}

/// The operator's verdict and then the `unknown` conversion, one-sided.
///
/// The two comparison positions whose right-hand operand is **not a sibling `Expr`**:
/// an `IN (SELECT …)`'s sub-plan column and an `op ANY`/`op ALL`'s array element.
///
/// One-sided is the whole of the difference from [`comparison_enum_literal_legality`],
/// and it is 18.6's shape rather than a shortcut: at both positions the right-hand side
/// has **already been resolved to a concrete type** by the time the operator is built —
/// a `PARAM_SUBLINK` carries its target-list entry's `exprType`, and an `A_ArrayExpr`
/// with no target type has folded its elements through `select_common_type`. Neither
/// can still be an `unknown` for the operator's resolution to convert, so
/// `m IN (SELECT 'zzz')` and `m = ANY(ARRAY['zzz'])` are `42883 operator does not
/// exist: mood = text` where the `IN ('zzz')` spelling of the same label is a `22P02`
/// membership test. Only the left-hand side has a literal to convert, and
/// `'zzz' IN (SELECT m FROM w)` is its `22P02`.
///
/// # Errors
///
/// `42883` for a pair with no operator, then `22P02` for a left-hand literal the
/// right-hand enum does not hold.
pub fn one_sided_comparison_legality(
    left: Option<ResolvedType>,
    right: Option<&ResolvedType>,
    left_expr: &Expr,
    symbol: &str,
    schema: &crate::catalog::Schema,
) -> Result<()> {
    comparison_legality(left, right.cloned(), symbol)?;
    if let Some(name) = right.and_then(|t| t.enum_type.name()) {
        enum_literal_legality(name, left_expr, schema)?;
    }
    Ok(())
}

#[cfg(test)]
mod assignment_legality_tests {
    use super::{assignment_compatible, assignment_legality};
    use crate::types::TypeOid;

    /// The 21 type spellings pglet models that the accept matrix was measured over —
    /// 17 scalars plus four arrays, enough to reach every rule and each array direction.
    const MEASURED: &[TypeOid] = &[
        TypeOid::Bool,
        TypeOid::Int2,
        TypeOid::Int4,
        TypeOid::Int8,
        TypeOid::Float4,
        TypeOid::Float8,
        TypeOid::Numeric,
        TypeOid::Text,
        TypeOid::Varchar,
        TypeOid::Bytea,
        TypeOid::Timestamp,
        TypeOid::Timestamptz,
        TypeOid::Date,
        TypeOid::Time,
        TypeOid::Interval,
        TypeOid::Jsonb,
        TypeOid::Uuid,
        TypeOid::TextArray,
        TypeOid::Int4Array,
        TypeOid::UuidArray,
        TypeOid::Int8Array,
    ];

    /// Every **cross-type** pair 18.6 accepts in assignment context, transcribed from the
    /// measurement rather than derived: `INSERT INTO <col of target> VALUES
    /// (NULL::<source>)` run for all 441 ordered pairs of [`MEASURED`] against a live
    /// 18.6, with the 21 same-type pairs dropped. Every one of the remaining 335
    /// rejections was `42804`.
    ///
    /// This list is the provenance for [`assignment_compatible`]'s four rules, so it is
    /// **not** generated from them — a table derived from the code it checks proves
    /// nothing. Regenerate it against a live server, do not hand-edit it.
    const ACCEPTED: &[(TypeOid, TypeOid)] = &[
        (TypeOid::Int4, TypeOid::Int2),
        (TypeOid::Int8, TypeOid::Int2),
        (TypeOid::Float4, TypeOid::Int2),
        (TypeOid::Float8, TypeOid::Int2),
        (TypeOid::Numeric, TypeOid::Int2),
        (TypeOid::Int2, TypeOid::Int4),
        (TypeOid::Int8, TypeOid::Int4),
        (TypeOid::Float4, TypeOid::Int4),
        (TypeOid::Float8, TypeOid::Int4),
        (TypeOid::Numeric, TypeOid::Int4),
        (TypeOid::Int2, TypeOid::Int8),
        (TypeOid::Int4, TypeOid::Int8),
        (TypeOid::Float4, TypeOid::Int8),
        (TypeOid::Float8, TypeOid::Int8),
        (TypeOid::Numeric, TypeOid::Int8),
        (TypeOid::Int2, TypeOid::Float4),
        (TypeOid::Int4, TypeOid::Float4),
        (TypeOid::Int8, TypeOid::Float4),
        (TypeOid::Float8, TypeOid::Float4),
        (TypeOid::Numeric, TypeOid::Float4),
        (TypeOid::Int2, TypeOid::Float8),
        (TypeOid::Int4, TypeOid::Float8),
        (TypeOid::Int8, TypeOid::Float8),
        (TypeOid::Float4, TypeOid::Float8),
        (TypeOid::Numeric, TypeOid::Float8),
        (TypeOid::Int2, TypeOid::Numeric),
        (TypeOid::Int4, TypeOid::Numeric),
        (TypeOid::Int8, TypeOid::Numeric),
        (TypeOid::Float4, TypeOid::Numeric),
        (TypeOid::Float8, TypeOid::Numeric),
        (TypeOid::Bool, TypeOid::Text),
        (TypeOid::Int2, TypeOid::Text),
        (TypeOid::Int4, TypeOid::Text),
        (TypeOid::Int8, TypeOid::Text),
        (TypeOid::Float4, TypeOid::Text),
        (TypeOid::Float8, TypeOid::Text),
        (TypeOid::Numeric, TypeOid::Text),
        (TypeOid::Varchar, TypeOid::Text),
        (TypeOid::Bytea, TypeOid::Text),
        (TypeOid::Timestamp, TypeOid::Text),
        (TypeOid::Timestamptz, TypeOid::Text),
        (TypeOid::Date, TypeOid::Text),
        (TypeOid::Time, TypeOid::Text),
        (TypeOid::Interval, TypeOid::Text),
        (TypeOid::Jsonb, TypeOid::Text),
        (TypeOid::Uuid, TypeOid::Text),
        (TypeOid::TextArray, TypeOid::Text),
        (TypeOid::Int4Array, TypeOid::Text),
        (TypeOid::UuidArray, TypeOid::Text),
        (TypeOid::Int8Array, TypeOid::Text),
        (TypeOid::Bool, TypeOid::Varchar),
        (TypeOid::Int2, TypeOid::Varchar),
        (TypeOid::Int4, TypeOid::Varchar),
        (TypeOid::Int8, TypeOid::Varchar),
        (TypeOid::Float4, TypeOid::Varchar),
        (TypeOid::Float8, TypeOid::Varchar),
        (TypeOid::Numeric, TypeOid::Varchar),
        (TypeOid::Text, TypeOid::Varchar),
        (TypeOid::Bytea, TypeOid::Varchar),
        (TypeOid::Timestamp, TypeOid::Varchar),
        (TypeOid::Timestamptz, TypeOid::Varchar),
        (TypeOid::Date, TypeOid::Varchar),
        (TypeOid::Time, TypeOid::Varchar),
        (TypeOid::Interval, TypeOid::Varchar),
        (TypeOid::Jsonb, TypeOid::Varchar),
        (TypeOid::Uuid, TypeOid::Varchar),
        (TypeOid::TextArray, TypeOid::Varchar),
        (TypeOid::Int4Array, TypeOid::Varchar),
        (TypeOid::UuidArray, TypeOid::Varchar),
        (TypeOid::Int8Array, TypeOid::Varchar),
        (TypeOid::Timestamptz, TypeOid::Timestamp),
        (TypeOid::Date, TypeOid::Timestamp),
        (TypeOid::Timestamp, TypeOid::Timestamptz),
        (TypeOid::Date, TypeOid::Timestamptz),
        (TypeOid::Timestamp, TypeOid::Date),
        (TypeOid::Timestamptz, TypeOid::Date),
        (TypeOid::Timestamp, TypeOid::Time),
        (TypeOid::Timestamptz, TypeOid::Time),
        (TypeOid::Interval, TypeOid::Time),
        (TypeOid::Time, TypeOid::Interval),
        (TypeOid::Int4Array, TypeOid::TextArray),
        (TypeOid::UuidArray, TypeOid::TextArray),
        (TypeOid::Int8Array, TypeOid::TextArray),
        (TypeOid::Int8Array, TypeOid::Int4Array),
        (TypeOid::Int4Array, TypeOid::Int8Array),
    ];

    /// The rulebook reproduces the measured matrix exactly — both directions, so a rule
    /// that is too narrow and one that is too wide both redden.
    ///
    /// **It is a claim about [`assignment_compatible`], not about the engine.** The
    /// matrix bound `NULL::<source>`, a value that never reaches a conversion, so it
    /// measures which pairs 18.6 *admits* and which this rulebook admits — and nothing
    /// about what pglet then does with a real one. Re-run with real values, 436 of the
    /// 441 pairs still agree and **five** die in `executor::coerce`'s cast table with
    /// `42846` after this function has accepted them, each storing on 18.6:
    /// `int8 → float4`, `float4 → int2`, `float4 → int8`, `float8 → int2` and
    /// `interval → time` (`EDGES.md`). A later widening of
    /// this table has to re-run the real-value form; the NULL form cannot see that
    /// class at all.
    #[test]
    fn the_four_rules_reproduce_the_measured_matrix() {
        assert_eq!(
            ACCEPTED.len(),
            85,
            "the measured accept set is 85 cross-type pairs"
        );
        for &source in MEASURED {
            for &target in MEASURED {
                let expected = source as u32 == target as u32
                    || ACCEPTED.iter().any(|&(s, t)| s == source && t == target);
                assert_eq!(
                    assignment_compatible(source, target),
                    expected,
                    "{source:?} -> {target:?}"
                );
            }
        }
    }

    /// The refusal carries Postgres's own sentence, with `format_type`'s **display**
    /// spelling on both sides — not the DDL spelling, which says `varchar` where 18.6
    /// says `character varying` and has no name at all for an array.
    #[test]
    fn the_message_is_postgres_s_sentence_in_display_spelling() {
        let message = |source, target| {
            assignment_legality(source, target, "b")
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            message(TypeOid::Int4, TypeOid::Bool),
            "type error: column \"b\" is of type boolean but expression is of type integer"
        );
        assert_eq!(
            message(TypeOid::Date, TypeOid::Time),
            "type error: column \"b\" is of type time without time zone but expression is of type date"
        );
        assert_eq!(
            message(TypeOid::TextArray, TypeOid::UuidArray),
            "type error: column \"b\" is of type uuid[] but expression is of type text[]"
        );
        // The control that identifies the display table rather than the sentence: a
        // string-category target never refuses at all, so `jsonb` reaches no message here.
        assert!(assignment_legality(TypeOid::Jsonb, TypeOid::Varchar, "b").is_ok());
    }

    /// SQLSTATE `42804` (`datatype_mismatch`) — the code 18.6 answers, and the one an
    /// `@expect-error` fixture compares.
    #[test]
    fn the_refusal_is_42804() {
        assert_eq!(
            assignment_legality(TypeOid::Text, TypeOid::Int4, "i")
                .unwrap_err()
                .sqlstate(),
            "42804"
        );
    }
}

#[cfg(test)]
mod unnest_legality_tests {
    use super::unnest_legality;
    use crate::types::{TypeOid, type_oid_display_name};

    /// `unnest` takes an array and nothing else, and the accept side is the whole array
    /// family rather than a list — [`unnest_legality`]'s reader is
    /// [`TypeOid::array_element`], the same derivation the two declared-type walkers use,
    /// so the rule and the element type it implies cannot come apart.
    ///
    /// The `None` case is asserted here because it is a *decision*, not an absence:
    /// declining is what leaves the `unknown` literals to 18.6's `42725` and a computed
    /// argument to the walkers (`EDGES.md` row 55).
    #[test]
    fn unnest_takes_an_array_and_declines_an_unknown() {
        for oid in [
            TypeOid::Int2,
            TypeOid::Int4,
            TypeOid::Int8,
            TypeOid::Numeric,
            TypeOid::Float4,
            TypeOid::Text,
            TypeOid::Varchar,
            TypeOid::Bool,
            TypeOid::Timestamptz,
            TypeOid::Uuid,
            TypeOid::Jsonb,
            TypeOid::Bytea,
            TypeOid::Date,
            TypeOid::Interval,
        ] {
            let err = unnest_legality(Some(oid)).unwrap_err();
            assert_eq!(err.sqlstate(), "42883", "{oid:?}");
            assert!(
                err.to_string().ends_with(&format!(
                    "function unnest({}) does not exist",
                    type_oid_display_name(oid)
                )),
                "{oid:?}: {err}"
            );
        }
        for oid in [
            TypeOid::Int4Array,
            TypeOid::TextArray,
            TypeOid::UuidArray,
            TypeOid::TimestamptzArray,
        ] {
            assert!(unnest_legality(Some(oid)).is_ok(), "{oid:?}");
        }
        assert!(unnest_legality(None).is_ok());
    }
}
