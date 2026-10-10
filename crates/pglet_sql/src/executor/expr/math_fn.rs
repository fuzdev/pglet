//! Numeric math functions (`abs`, `ceil`, `floor`, `round`, `trunc`, `sqrt`, `sign`).

use super::arithmetic::{as_exact_numeric, value_as_i64, value_to_f64_opt};
use crate::planner::MathFunc;
use crate::types::{PgNumeric, Value, opt_type_oid_display_name};
use crate::{Error, Result};

/// Evaluate a numeric math function (`abs`/`ceil`/`floor`/`round`/`trunc`).
///
/// `NUMERIC` is exact; a float argument computes in `float8`; an integer promotes
/// to `NUMERIC` (except `abs`, which keeps the integer type, erroring loud on the
/// `MIN`-value overflow like Postgres). A NULL argument yields NULL. `round`/`trunc`
/// take an optional integer scale; a float with an explicit scale fails loud
/// (Postgres has no `round(double precision, integer)`).
pub(super) fn eval_math_fn(func: MathFunc, args: &[Value]) -> Result<Value> {
    if args.iter().any(Value::is_null) {
        return Ok(Value::Null);
    }
    let arg = &args[0];
    let scale = match args.get(1) {
        Some(v) => Some(value_to_scale(v)?),
        None => None,
    };
    match func {
        MathFunc::Abs => eval_abs(arg),
        MathFunc::Ceil => eval_ceil_floor(arg, false),
        MathFunc::Floor => eval_ceil_floor(arg, true),
        MathFunc::Round => eval_round_trunc(arg, scale, false),
        MathFunc::Trunc => eval_round_trunc(arg, scale, true),
        MathFunc::Sqrt => eval_sqrt(arg),
        MathFunc::Sign => eval_sign(arg),
    }
}

/// `sqrt(x)` — only `(float8)`/`(numeric)` overloads: a `numeric` argument computes
/// exactly (Postgres `numeric_sqrt` scale), everything else (int **and** float)
/// computes in `float8`. A negative argument fails loud (`2201F`); `NaN → NaN`.
fn eval_sqrt(arg: &Value) -> Result<Value> {
    if let Value::Numeric(n) = arg {
        return n.sqrt().map(Value::Numeric);
    }
    let Some(f) = value_to_f64_opt(arg) else {
        return Err(Error::UndefinedFunction(format!(
            "function sqrt does not exist for argument type {}",
            opt_type_oid_display_name(arg.type_oid())
        )));
    };
    if f < 0.0 {
        return Err(Error::InvalidArgumentForPowerFunction(
            "cannot take square root of a negative number".into(),
        ));
    }
    // `sqrt(NaN)` is NaN (`f.sqrt()` already yields NaN, and `NaN < 0` is false).
    Ok(Value::Float8(f.sqrt()))
}

/// `sign(x)` — the sign with the argument's family: `(numeric)` for a numeric
/// argument (`NaN → NaN`), `(float8)` otherwise (int/float). The float form
/// reproduces Postgres `dsign`'s `>`/`<` test, so `sign(NaN) = 0` (a naive
/// `f64::signum()` would wrongly give `NaN`, and `signum(0.0)` would give `1`).
fn eval_sign(arg: &Value) -> Result<Value> {
    if let Value::Numeric(n) = arg {
        return Ok(Value::Numeric(numeric_sign(n)));
    }
    let Some(f) = value_to_f64_opt(arg) else {
        return Err(Error::UndefinedFunction(format!(
            "function sign does not exist for argument type {}",
            opt_type_oid_display_name(arg.type_oid())
        )));
    };
    // Postgres `dsign`: 0 when neither `>0` nor `<0` (covers both 0 and NaN).
    let s = if f > 0.0 {
        1.0
    } else if f < 0.0 {
        -1.0
    } else {
        0.0
    };
    Ok(Value::Float8(s))
}

/// `sign(numeric)` — `-1`/`0`/`1` as a `numeric` (Postgres `numeric_sign`); `NaN`
/// stays `NaN`.
fn numeric_sign(n: &PgNumeric) -> PgNumeric {
    if n.is_nan() {
        PgNumeric::NaN
    } else if n.is_zero() {
        PgNumeric::zero()
    } else if n.is_negative() {
        PgNumeric::from_i128(-1)
    } else {
        PgNumeric::from_i128(1)
    }
}

/// The integer scale argument of `round`/`trunc` (a non-integer fails loud).
fn value_to_scale(v: &Value) -> Result<i32> {
    value_as_i64(v)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| Error::Type("round/trunc scale must be an integer".to_string()))
}

/// `abs(x)` — preserves the argument's type; integer overflow (`abs(i32::MIN)`)
/// fails loud (`22003`) like Postgres.
fn eval_abs(arg: &Value) -> Result<Value> {
    match arg {
        Value::Numeric(n) => Ok(Value::Numeric(n.abs())),
        Value::Int2(x) => x
            .checked_abs()
            .map(Value::Int2)
            .ok_or(Error::NumericOutOfRange("smallint")),
        Value::Int4(x) => x
            .checked_abs()
            .map(Value::Int4)
            .ok_or(Error::NumericOutOfRange("integer")),
        Value::Int8(x) => x
            .checked_abs()
            .map(Value::Int8)
            .ok_or(Error::NumericOutOfRange("bigint")),
        Value::Float4(x) => Ok(Value::Float4(x.abs())),
        Value::Float8(x) => Ok(Value::Float8(x.abs())),
        other => Err(Error::NotImplemented(format!(
            "abs() requires a numeric argument, got {:?}",
            other.type_oid()
        ))),
    }
}

/// `ceil`/`floor` — a `NUMERIC` argument yields `NUMERIC`; **int and float both yield
/// `float8`** (Postgres resolves `ceil(int)` to the `double precision` overload, so
/// `ceil(5)` is `5.0`, matching the `math_fn_result_type` OID).
fn eval_ceil_floor(arg: &Value, toward_neg: bool) -> Result<Value> {
    match arg {
        Value::Numeric(n) => Ok(Value::Numeric(if toward_neg {
            n.floor()?
        } else {
            n.ceil()?
        })),
        Value::Float4(_) | Value::Float8(_) | Value::Int2(_) | Value::Int4(_) | Value::Int8(_) => {
            let f = value_to_f64_opt(arg).unwrap_or_default();
            Ok(Value::Float8(if toward_neg { f.floor() } else { f.ceil() }))
        }
        _ => Err(ceil_floor_type_err(arg)),
    }
}

fn ceil_floor_type_err(arg: &Value) -> Error {
    Error::NotImplemented(format!(
        "ceil/floor require a numeric argument, got {:?}",
        arg.type_oid()
    ))
}

/// `round`/`trunc` with an optional scale. A float with an explicit scale fails
/// loud (Postgres has no two-argument float form). A **one-argument** call on an
/// integer yields `float8` (`round(9)` → `9.0`, the `double precision` overload,
/// matching the `math_fn_result_type` OID); with an explicit scale an integer routes
/// to the numeric path (the only two-argument overload is `(numeric, integer)`).
///
/// The two overloads round a tie differently, each as 18.6's does: `round(double
/// precision)` is `dround`, which is C's `rint` — half to even under the default
/// rounding mode, so `round(2.5::float8)` is `2` and `round(-0.5::float8)` is `-0` —
/// while `round(numeric)` rounds half away from zero (`round(2.5)` is `3`).
fn eval_round_trunc(arg: &Value, scale: Option<i32>, truncate: bool) -> Result<Value> {
    let float_round = |f: f64| {
        if truncate {
            f.trunc()
        } else {
            f.round_ties_even()
        }
    };
    match arg {
        Value::Float4(_) | Value::Float8(_) => {
            if scale.is_some() {
                return Err(Error::NotImplemented(
                    "round/trunc with a scale requires a numeric argument".to_string(),
                ));
            }
            let f = value_to_f64_opt(arg).unwrap_or_default();
            Ok(Value::Float8(float_round(f)))
        }
        // One-argument round/trunc on an integer resolves to the double-precision
        // overload (no scale → `float8`); a scaled call falls through to numeric.
        Value::Int2(_) | Value::Int4(_) | Value::Int8(_) if scale.is_none() => {
            let f = value_to_f64_opt(arg).unwrap_or_default();
            Ok(Value::Float8(float_round(f)))
        }
        _ => {
            let n = as_exact_numeric(arg).ok_or_else(|| {
                Error::NotImplemented(format!(
                    "round/trunc require a numeric argument, got {:?}",
                    arg.type_oid()
                ))
            })?;
            let s = scale.unwrap_or(0);
            let result = if truncate { n.trunc(s) } else { n.round(s) };
            // Postgres `numeric_round`/`numeric_trunc` run `make_result` on the
            // result, so a round-up carry that crosses the weight cap (e.g.
            // `round(<max-weight nines>.5, 0)` → `10^131072`) fails loud (22003)
            // rather than storing an over-cap value.
            result.check_weight().map(Value::Numeric)
        }
    }
}
