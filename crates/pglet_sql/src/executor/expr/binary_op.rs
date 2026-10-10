//! Binary operators, connectives and the operators that evaluate beside them.

use super::arithmetic::{
    ArithOp, eval_arithmetic, eval_concat, eval_modulo, eval_operator_call, eval_power,
    interval_negate,
};
use super::compare::{
    boolean_context_operand, eval_ilike, eval_like, eval_regex, eval_starts_with,
    values_compare_ordered, values_equal,
};
use super::eval::option_to_value;
use super::json_ops::{
    containment, eval_json_exists, eval_json_exists_any_all, eval_json_get, eval_json_path_get,
};
use crate::planner::{BinaryOp, BoolContext, ResolvedType, comparison_legality, comparison_symbol};
use crate::types::{Infinite, Value};
use crate::{Error, Result};
use std::cmp::Ordering;

/// Evaluate a binary operation.
///
/// The comparison operators (and the `IS [NOT] DISTINCT FROM` forms that resolve
/// through `=`) are **operand-type gated** first, through the shared
/// [`comparison_legality`] rule: a pair Postgres has no operator for fails `42883`
/// rather than dispatching on the runtime `Value` variant and falling through to a
/// permissive `false` / text-order. Two NULLs — or one — carry no type, so they pass
/// the gate and NULL-propagate as before.
///
/// # Errors
///
/// Whatever the operator's evaluator or legality rule rejects the operands with.
pub fn eval_binary_op(op: BinaryOp, left: &Value, right: &Value) -> Result<Value> {
    eval_binary_op_ordered(op, left, right, None)
}

/// The value an `AND` / `OR` takes from its **left** operand alone, when that operand
/// decides it — `false` for `AND`, `true` for `OR` — and `None` for every other
/// operator and every other left value, NULL included.
///
/// 18.6 evaluates a boolean connective's operands left to right and stops at the first
/// that decides it (the `EEOP_BOOL_AND_STEP` / `EEOP_BOOL_OR_STEP` steps of
/// `ExecInterpExpr`, `execExprInterp.c`), so an operand after it is never evaluated and
/// cannot raise: `id = 1 OR 1/(id-1) = 0` is a row there. A NULL left operand decides
/// nothing — `NULL AND false` is `false` and `NULL OR true` is `true` — so the right is
/// still evaluated. The scalar walk (`eval_expr_typed`) asks this before it evaluates a
/// right operand — a grouping query's expressions included — and a long list's balanced
/// tree stops the same way, since a subtree that decides answers the deciding value up to
/// its parent. A grouping computes its aggregates before that walk runs, so an aggregate
/// under a right operand the walk skips is still computed, as 18.6's `Agg` computes it;
/// only a left operand that decides by its shape alone removes it, 18.6's planner having
/// folded the connective (`executor::query::aggregate`'s `Collector`). The GIN scan's
/// guard, a disjunct evaluated once ahead of every row, cannot ask it, and leaves a
/// failure to its recheck `Filter` instead (`execute_gin_scan`).
///
/// # Errors
///
/// [`boolean_context_operand`]'s over the left operand — the `42804` of a non-boolean
/// one, the `22P02` of a text one that is not a boolean spelling.
pub fn connective_decided_by(op: BinaryOp, left: &Value) -> Result<Option<bool>> {
    let Some(ctx) = op.operand_context() else {
        return Ok(None);
    };
    Ok(
        match (boolean_context_operand(left, ctx)?, op.connective()) {
            (Some(false), BinaryOp::And) => Some(false),
            (Some(true), BinaryOp::Or) => Some(true),
            _ => None,
        },
    )
}

/// [`eval_binary_op`] over an operand pair whose **enum member order** the caller
/// resolved (`enum_order`), so `<`/`<=`/`>`/`>=` order by member position rather than
/// by label bytes. `None` is the ordinary non-enum case and is byte-identical to
/// [`eval_binary_op`].
///
/// Only the four ordering operators read it: `=`/`<>` (and the `IS [NOT] DISTINCT
/// FROM` forms) stay on [`values_equal`] deliberately — 18.6's `enum_eq`/`enum_ne` are
/// bare OID identity (`src/backend/utils/adt/enum.c`), and one member is one label is
/// one OID, so label-byte equality **is** OID equality and a position-aware equality
/// would be the same answer computed twice.
///
/// # Errors
///
/// Whatever the operator's evaluator or legality rule rejects the operands with.
pub fn eval_binary_op_ordered(
    op: BinaryOp,
    left: &Value,
    right: &Value,
    enum_order: Option<&[String]>,
) -> Result<Value> {
    if let Some(symbol) = comparison_symbol(op) {
        // Eval time reads its operands from `Value`s, which carry no enum identity at
        // all — so this half of the rulebook is structurally blind to `EDGES.md` row
        // 114's pair and the plan-time half is where it is refused.
        comparison_legality(
            left.type_oid().map(ResolvedType::plain),
            right.type_oid().map(ResolvedType::plain),
            symbol,
        )?;
    }
    match op {
        // Comparison operators: NULL propagates through
        BinaryOp::Eq => Ok(option_to_value(values_equal(left, right))),
        BinaryOp::Ne => Ok(option_to_value(values_equal(left, right).map(|b| !b))),
        BinaryOp::Lt => Ok(option_to_value(
            values_compare_ordered(left, right, enum_order).map(Ordering::is_lt),
        )),
        BinaryOp::Le => Ok(option_to_value(
            values_compare_ordered(left, right, enum_order).map(|o| !o.is_gt()),
        )),
        BinaryOp::Gt => Ok(option_to_value(
            values_compare_ordered(left, right, enum_order).map(Ordering::is_gt),
        )),
        BinaryOp::Ge => Ok(option_to_value(
            values_compare_ordered(left, right, enum_order).map(|o| !o.is_lt()),
        )),
        // Boolean operators: SQL three-valued logic
        // TRUE AND NULL = NULL, FALSE AND NULL = FALSE
        // TRUE OR NULL = TRUE, FALSE OR NULL = NULL
        // The walks that evaluate an expression stop at a left operand that decides
        // (`connective_decided_by`) and reach here only with both operands evaluated;
        // a value-only caller holding both still gets the full table.
        BinaryOp::And | BinaryOp::AndList | BinaryOp::AndListInner => {
            let l = boolean_context_operand(left, BoolContext::And)?;
            let r = boolean_context_operand(right, BoolContext::And)?;
            match (l, r) {
                (Some(false), _) | (_, Some(false)) => Ok(Value::Bool(false)),
                (Some(true), Some(true)) => Ok(Value::Bool(true)),
                _ => Ok(Value::Null), // At least one NULL and no FALSE
            }
        }
        BinaryOp::Or | BinaryOp::OrList | BinaryOp::OrListInner => {
            let l = boolean_context_operand(left, BoolContext::Or)?;
            let r = boolean_context_operand(right, BoolContext::Or)?;
            match (l, r) {
                (Some(true), _) | (_, Some(true)) => Ok(Value::Bool(true)),
                (Some(false), Some(false)) => Ok(Value::Bool(false)),
                _ => Ok(Value::Null), // At least one NULL and no TRUE
            }
        }
        BinaryOp::Add => eval_arithmetic(ArithOp::Add, left, right),
        BinaryOp::Sub => eval_arithmetic(ArithOp::Sub, left, right),
        BinaryOp::Mul => eval_arithmetic(ArithOp::Mul, left, right),
        BinaryOp::Div => eval_arithmetic(ArithOp::Div, left, right),
        // `%` and `^` are their own evaluators, not `ArithOp` arms — their operand
        // support differs (no float `%`; `^` is float8/numeric only, int→float8).
        BinaryOp::Mod => eval_modulo(left, right),
        BinaryOp::Pow => eval_power(left, right),
        BinaryOp::ModFn | BinaryOp::PowerFn | BinaryOp::PowFn => {
            eval_operator_call(op, left, right)
        }
        // Value-only entry: no declared types, so a `timestamptz` operand renders
        // offset-less. `eval_expr_full` intercepts `Concat` before reaching here and
        // supplies the declared types; this arm serves the callers that have only
        // values (and thus no timestamptz/timestamp distinction to preserve).
        BinaryOp::Concat => eval_concat(left, None, right, None),
        BinaryOp::Like => eval_like(left, right, "~~").map(option_to_value),
        BinaryOp::NotLike => eval_like(left, right, "!~~").map(|b| option_to_value(b.map(|v| !v))),
        BinaryOp::Ilike => eval_ilike(left, right, "~~*").map(option_to_value),
        BinaryOp::NotIlike => {
            eval_ilike(left, right, "!~~*").map(|b| option_to_value(b.map(|v| !v)))
        }
        BinaryOp::StartsWith => eval_starts_with(left, right).map(option_to_value),
        BinaryOp::RegexMatch => eval_regex(left, right, "~").map(option_to_value),
        BinaryOp::NotRegexMatch => {
            eval_regex(left, right, "!~").map(|b| option_to_value(b.map(|v| !v)))
        }
        BinaryOp::JsonGet => eval_json_get(left, right, false),
        BinaryOp::JsonGetText => eval_json_get(left, right, true),
        // `jsonb ? text`: key/element existence. `?|`/`?&`: any/all over a `text[]`.
        BinaryOp::JsonExists => eval_json_exists(left, right),
        BinaryOp::JsonExistsAny => eval_json_exists_any_all(left, right, false),
        BinaryOp::JsonExistsAll => eval_json_exists_any_all(left, right, true),
        // `jsonb #> text[]` (→ jsonb) / `#>>` (→ text): extract at a path.
        BinaryOp::JsonPathGet => eval_json_path_get(left, right, false),
        BinaryOp::JsonPathGetText => eval_json_path_get(left, right, true),
        // `a @> b`: a contains b. `a <@ b`: a is contained by b (swap operands).
        BinaryOp::Containment => containment(left, right, "@>"),
        BinaryOp::ContainedBy => containment(right, left, "<@"),
        BinaryOp::AtTimeZone => eval_at_time_zone(left, right),
        BinaryOp::ToChar => eval_to_char(left, right),
        // Null-safe comparison: ALWAYS a concrete boolean, never NULL. Both NULL
        // are "not distinct"; exactly one NULL is "distinct"; otherwise defer to
        // value equality (`values_equal` returns `Some` for two non-null operands,
        // so `.unwrap_or(false)` is total in the `(false, false)` branch).
        BinaryOp::IsNotDistinctFrom => Ok(Value::Bool(match (left.is_null(), right.is_null()) {
            (true, true) => true,
            (true, false) | (false, true) => false,
            (false, false) => values_equal(left, right).unwrap_or(false),
        })),
        BinaryOp::IsDistinctFrom => Ok(Value::Bool(match (left.is_null(), right.is_null()) {
            (true, true) => false,
            (true, false) | (false, true) => true,
            (false, false) => !values_equal(left, right).unwrap_or(false),
        })),
    }
}

/// Evaluate `ts AT TIME ZONE zone`.
///
/// pglet stores `Value::Timestamp` as UTC and has no distinct timestamptz, so
/// `AT TIME ZONE 'UTC'` is identity (the value is returned unchanged). Any other
/// zone fails loud rather than silently mis-rendering. NULL operand → NULL.
fn eval_at_time_zone(ts: &Value, zone: &Value) -> Result<Value> {
    if ts.is_null() || zone.is_null() {
        return Ok(Value::Null);
    }
    let Value::Text(z) = zone else {
        return Err(Error::NotImplemented(
            "AT TIME ZONE requires a text time-zone name".into(),
        ));
    };
    if !z.eq_ignore_ascii_case("UTC") {
        return Err(Error::NotImplemented(format!(
            "AT TIME ZONE only supports 'UTC' (got {z:?})"
        )));
    }
    match ts {
        Value::Timestamp(_) => Ok(ts.clone()),
        _ => Err(Error::NotImplemented(
            "AT TIME ZONE requires a timestamp left operand".into(),
        )),
    }
}

/// Evaluate `to_char(ts, format)`.
///
/// `ts` must be a timestamp and `format` a text literal; the timestamp is
/// rendered through `format_timestamp_to_char` (the spine's ISO-8601 token
/// subset). Unknown format tokens fail loud there. NULL operand → NULL.
fn eval_to_char(ts: &Value, format: &Value) -> Result<Value> {
    if ts.is_null() || format.is_null() {
        return Ok(Value::Null);
    }
    let Value::Text(fmt) = format else {
        return Err(Error::NotImplemented(
            "to_char requires a text format string".into(),
        ));
    };
    let Value::Timestamp(dt) = ts else {
        return Err(Error::NotImplemented(
            "to_char is only implemented for timestamp values".into(),
        ));
    };
    // An infinity renders as NULL, ahead of reading the format (`timestamp_to_char`).
    let Infinite::Finite(dt) = dt else {
        return Ok(Value::Null);
    };
    crate::types::format_timestamp_to_char(dt, fmt).map(Value::Text)
}

/// Evaluate numeric negation (-x).
/// Returns NULL if input is NULL.
pub fn eval_negate(val: &Value) -> Result<Value> {
    Ok(match val {
        Value::Null => Value::Null,
        // Checked so negating the MIN value fails loud (`22003`) like Postgres's
        // `int2um`/`int4um`/`int8um`, rather than wrapping to itself (mirrors
        // `eval_abs`/`interval_negate`).
        Value::Int2(n) => n
            .checked_neg()
            .map(Value::Int2)
            .ok_or(Error::NumericOutOfRange("smallint"))?,
        Value::Int4(n) => n
            .checked_neg()
            .map(Value::Int4)
            .ok_or(Error::NumericOutOfRange("integer"))?,
        Value::Int8(n) => n
            .checked_neg()
            .map(Value::Int8)
            .ok_or(Error::NumericOutOfRange("bigint"))?,
        Value::Float4(n) => Value::Float4(-n),
        Value::Float8(n) => Value::Float8(-n),
        Value::Numeric(nn) => Value::Numeric(nn.negate()),
        // Unary `-interval` (`interval_um`): negate every field independently.
        Value::Interval(iv) => Value::Interval(interval_negate(iv)?),
        _ => {
            return Err(Error::NotImplemented(format!(
                "negation not supported for type: {val:?}"
            )));
        }
    })
}
