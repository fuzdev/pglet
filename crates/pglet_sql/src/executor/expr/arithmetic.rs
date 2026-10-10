//! Arithmetic, temporal arithmetic, concatenation, casts and the exact-numeric promotion they share.

use super::compare::date_to_timestamp;
use crate::executor::coerce::coerce_value;
use crate::planner::{BinaryOp, concat_legality, concat_undefined};
use crate::types::{
    ArrayValue, Infinite, PgInterval, PgNumeric, TypeOid, Value, opt_type_oid_display_name,
    value_to_text_with_oid,
};
use crate::{Error, Result};

/// Convert a value to f64 if it's numeric.
pub fn value_to_f64_opt(v: &Value) -> Option<f64> {
    match v {
        Value::Int2(n) => Some(f64::from(*n)),
        Value::Int4(n) => Some(f64::from(*n)),
        Value::Int8(n) => Some(*n as f64),
        Value::Float4(n) => Some(f64::from(*n)),
        Value::Float8(n) => Some(*n),
        // Only reached for a numeric-vs-float comparison/arithmetic (numeric-vs-
        // numeric/int is handled exactly upstream); Postgres casts numeric→float8.
        Value::Numeric(nn) => Some(nn.to_f64()),
        _ => None,
    }
}

/// The four arithmetic operators, extracted from `BinaryOp` so the helpers below
/// dispatch over a closed set (and `int_arith` matches it exhaustively, no
/// catch-all that could silently mishandle a non-arithmetic op).
#[derive(Clone, Copy)]
pub(super) enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl ArithOp {
    const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
        }
    }
}

/// Integer widths supporting checked arithmetic for the SQL arithmetic operators.
/// Distinct method names (not the inherent `checked_*`) keep the trait dispatch
/// unambiguous.
trait CheckedArith: Copy {
    fn arith_add(self, rhs: Self) -> Option<Self>;
    fn arith_sub(self, rhs: Self) -> Option<Self>;
    fn arith_mul(self, rhs: Self) -> Option<Self>;
    fn arith_div(self, rhs: Self) -> Option<Self>;
    fn arith_rem(self, rhs: Self) -> Option<Self>;
    fn arith_is_zero(self) -> bool;
    fn arith_is_neg_one(self) -> bool;
    fn arith_zero() -> Self;
}

macro_rules! impl_checked_arith {
    ($($t:ty),+) => {$(
        impl CheckedArith for $t {
            fn arith_add(self, rhs: Self) -> Option<Self> { self.checked_add(rhs) }
            fn arith_sub(self, rhs: Self) -> Option<Self> { self.checked_sub(rhs) }
            fn arith_mul(self, rhs: Self) -> Option<Self> { self.checked_mul(rhs) }
            fn arith_div(self, rhs: Self) -> Option<Self> { self.checked_div(rhs) }
            fn arith_rem(self, rhs: Self) -> Option<Self> { self.checked_rem(rhs) }
            fn arith_is_zero(self) -> bool { self == 0 }
            fn arith_is_neg_one(self) -> bool { self == -1 }
            fn arith_zero() -> Self { 0 }
        }
    )+};
}
impl_checked_arith!(i16, i32, i64);

/// Checked integer arithmetic in the integer domain: overflow becomes a loud
/// `NumericOutOfRange` (Postgres `22003`) and division by zero a loud
/// `DivisionByZero` (`22012`) — never a wrap, a saturate, or `NaN as i64 == 0`.
/// `type_name` is the SQL type for the out-of-range message.
fn int_arith<T: CheckedArith>(op: ArithOp, l: T, r: T, type_name: &'static str) -> Result<T> {
    let result = match op {
        ArithOp::Add => l.arith_add(r),
        ArithOp::Sub => l.arith_sub(r),
        ArithOp::Mul => l.arith_mul(r),
        ArithOp::Div => {
            if r.arith_is_zero() {
                return Err(Error::DivisionByZero);
            }
            l.arith_div(r) // also catches i64::MIN / -1 overflow
        }
    };
    result.ok_or(Error::NumericOutOfRange(type_name))
}

/// Checked integer modulo (`%`): a zero divisor is a loud `DivisionByZero`
/// (`22012`), matching Postgres, and an overflowing remainder (`T::MIN % -1`) is
/// `NumericOutOfRange` (`22003`) rather than a wrap. `type_name` is the SQL type
/// for the out-of-range message.
fn int_rem<T: CheckedArith>(l: T, r: T, type_name: &'static str) -> Result<T> {
    if r.arith_is_zero() {
        return Err(Error::DivisionByZero);
    }
    // `x % -1 == 0` for every `x`; `checked_rem` alone returns `None` for `MIN % -1`
    // (the *division* overflows), so special-case it to 0 like Postgres's `int4mod`
    // et al., rather than the spurious `22003`.
    if r.arith_is_neg_one() {
        return Ok(T::arith_zero());
    }
    l.arith_rem(r).ok_or(Error::NumericOutOfRange(type_name))
}

/// Float arithmetic, matching Postgres `float.h`'s per-operator range checks
/// (`float8_pl`/`_mi`/`_mul`/`_div`) rather than returning the raw IEEE result:
/// a finite-operand overflow to `±inf` and a nonzero-operand underflow to `0`
/// both fail loud (`22003`), and division by zero is `22012`. A NaN result (e.g.
/// `inf - inf`) and an already-infinite operand pass through — only a genuine
/// finite→inf overflow or nonzero→0 underflow errors, exactly as Postgres tests it.
fn float_arith(op: ArithOp, l: f64, r: f64) -> Result<f64> {
    let overflow = || Error::NumericOutOfRange("double precision");
    let underflow = || Error::NumericOutOfRange("double precision");
    match op {
        // Add/sub only check overflow (Postgres `float8_pl`/`float8_mi`).
        ArithOp::Add => {
            let result = l + r;
            if result.is_infinite() && l.is_finite() && r.is_finite() {
                return Err(overflow());
            }
            Ok(result)
        }
        ArithOp::Sub => {
            let result = l - r;
            if result.is_infinite() && l.is_finite() && r.is_finite() {
                return Err(overflow());
            }
            Ok(result)
        }
        // Mul checks both overflow and underflow (Postgres `float8_mul`).
        ArithOp::Mul => {
            let result = l * r;
            if result.is_infinite() && l.is_finite() && r.is_finite() {
                return Err(overflow());
            }
            if result == 0.0 && l != 0.0 && r != 0.0 {
                return Err(underflow());
            }
            Ok(result)
        }
        // Div: zero-divide first, then overflow (numerator finite) and underflow
        // (numerator nonzero, denominator finite) — Postgres `float8_div`. The
        // zero-divide guard skips a NaN numerator (`!isnan(val1)`), so `NaN / 0`
        // yields NaN (not `22012`), matching PG; a finite/inf numerator over 0 is
        // still a loud division by zero.
        ArithOp::Div => {
            if r == 0.0 && !l.is_nan() {
                return Err(Error::DivisionByZero);
            }
            let result = l / r;
            if result.is_infinite() && l.is_finite() {
                return Err(overflow());
            }
            if result == 0.0 && l != 0.0 && r.is_finite() {
                return Err(underflow());
            }
            Ok(result)
        }
    }
}

/// `float4 <op> float4`, computed in **f32** — Postgres's `float4_pl`/`_mi`/`_mul`/
/// `_div` (`utils/float.h`), which are `float4` arithmetic throughout.
///
/// Not a narrowing of the `f64` result: the two differ. `0.1::real + 0.2::real` is
/// `0.3` when added as f32 and `0.30000000447034836` when added as f64 and then
/// rendered, and Postgres prints the former. Computing wide and narrowing late would
/// also change the overflow point, since an f64 intermediate cannot overflow where an
/// f32 sum does.
///
/// The overflow/underflow/zero-divide checks mirror the f64 helper
/// ([`float_arith`]) arm for arm, differing only in the type they report.
fn float4_arith(op: ArithOp, l: f32, r: f32) -> Result<f32> {
    let overflow = || Error::NumericOutOfRange("real");
    let underflow = || Error::NumericOutOfRange("real");
    match op {
        ArithOp::Add => {
            let result = l + r;
            if result.is_infinite() && l.is_finite() && r.is_finite() {
                return Err(overflow());
            }
            Ok(result)
        }
        ArithOp::Sub => {
            let result = l - r;
            if result.is_infinite() && l.is_finite() && r.is_finite() {
                return Err(overflow());
            }
            Ok(result)
        }
        ArithOp::Mul => {
            let result = l * r;
            if result.is_infinite() && l.is_finite() && r.is_finite() {
                return Err(overflow());
            }
            if result == 0.0 && l != 0.0 && r != 0.0 {
                return Err(underflow());
            }
            Ok(result)
        }
        ArithOp::Div => {
            if r == 0.0 && !l.is_nan() {
                return Err(Error::DivisionByZero);
            }
            let result = l / r;
            if result.is_infinite() && l.is_finite() {
                return Err(overflow());
            }
            if result == 0.0 && l != 0.0 && r.is_finite() {
                return Err(underflow());
            }
            Ok(result)
        }
    }
}

/// One `float4_pl` step, for `sum(real)`'s f32 accumulator.
///
/// Shared with the aggregate executor so the aggregate's rounding and its
/// overflow behavior are the *same* code as `real + real`, not a second copy.
///
/// # Errors
///
/// `22003` when the sum overflows `real` from two finite operands.
pub fn float4_add(l: f32, r: f32) -> Result<f32> {
    float4_arith(ArithOp::Add, l, r)
}

/// One `float8_pl` step, for `sum(double precision)`'s accumulator — the `float8 +
/// float8` operator's own code, as [`float4_add`] is `real + real`'s.
///
/// # Errors
///
/// `22003` when the sum overflows `double precision` from two finite operands.
pub fn float8_add(l: f64, r: f64) -> Result<f64> {
    float_arith(ArithOp::Add, l, r)
}

/// The integer value of an `Int2`/`Int4`/`Int8`, widened to `i64`; `None` for any
/// other value (so a non-integer routes to the float path instead of silently
/// defaulting to 0). The single integer-widening helper, shared with the aggregate
/// executor (`SUM`/`AVG` integer accumulation).
pub fn value_as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Int2(n) => Some(i64::from(*n)),
        Value::Int4(n) => Some(i64::from(*n)),
        Value::Int8(n) => Some(*n),
        _ => None,
    }
}

/// Evaluate an arithmetic operation (`+`, `-`, `*`, `/`).
///
/// Integer operands are computed in the integer domain with checked arithmetic:
/// same-width operands keep their type (`Int4 + Int4 → Int4`), mixed widths
/// promote to `Int8`, and overflow or division by zero is a loud error — integer
/// math never wraps, saturates, or routes through `f64` (which loses precision
/// above 2^53). A float operand promotes both sides to `Float8`. A NULL operand
/// yields NULL; a non-numeric operand (text, uuid, bool) is an error, never a
/// silent 0.
pub(super) fn eval_arithmetic(op: ArithOp, left: &Value, right: &Value) -> Result<Value> {
    // NULL propagates: `NULL <op> x` and `x <op> NULL` are NULL.
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    // Temporal arithmetic. An `INTERVAL` is Postgres's 3-field `(months, days,
    // micros)` span, so `timestamp ± interval` runs the 3-pass calendar (month
    // shift with day-of-month clamp → flat day count → microseconds), not a flat
    // µs shift. `timestamp - timestamp → interval` (the spine's `expires_at -
    // NOW()`) yields a justified span (≥24h folded into days). `interval + ts` is
    // commutative; `interval - ts` is undefined and falls through to the loud
    // non-numeric error. Overflow fails loud (`22008`) rather than wrapping.
    match (op, left, right) {
        (ArithOp::Sub, Value::Timestamp(l), Value::Timestamp(r)) => {
            // `timestamp_mi` refuses an infinity less the same infinity, and answers an
            // infinite interval for any other infinite operand, which `PgInterval` does
            // not model: a loud `0A000` here.
            let (Infinite::Finite(l), Infinite::Finite(r)) = (l, r) else {
                return Err(if l == r {
                    Error::TemporalOutOfRange("interval")
                } else {
                    Error::NotImplemented("infinite interval".into())
                });
            };
            let micros = i64::try_from((*l - *r).whole_microseconds())
                .map_err(|_| Error::TemporalOutOfRange("interval"))?;
            // `timestamp_mi`: raw µs diff, then `justify_hours` folds ≥24h into days.
            return justify_hours(PgInterval::from_micros(micros)).map(Value::Interval);
        }
        (ArithOp::Add, Value::Timestamp(t), Value::Interval(i))
        | (ArithOp::Add, Value::Interval(i), Value::Timestamp(t)) => {
            return timestamp_add_interval(*t, i, false).map(Value::Timestamp);
        }
        (ArithOp::Sub, Value::Timestamp(t), Value::Interval(i)) => {
            return timestamp_add_interval(*t, i, true).map(Value::Timestamp);
        }
        // `interval ± interval` (`interval_pl`/`interval_mi`): field-wise, no
        // auto-justify (Postgres keeps `25h - 1h` as `24:00:00`, not `1 day`).
        (ArithOp::Add, Value::Interval(a), Value::Interval(b)) => {
            return interval_add(a, b).map(Value::Interval);
        }
        (ArithOp::Sub, Value::Interval(a), Value::Interval(b)) => {
            return interval_sub(a, b).map(Value::Interval);
        }
        // `interval × number` / `number × interval` (`interval_mul`, commutative)
        // and `interval ÷ number` (`interval_div`, not commutative). The number
        // operand casts to `float8`; fractions cascade down (month→days→µs). A
        // non-numeric operand or `interval × interval` is a loud type error;
        // division by zero is `22012`.
        (ArithOp::Mul, Value::Interval(iv), other) | (ArithOp::Mul, other, Value::Interval(iv)) => {
            let Some(factor) = value_to_f64_opt(other) else {
                return Err(Error::Type(format!(
                    "operator {} requires numeric operands",
                    op.symbol()
                )));
            };
            return interval_scale(iv, factor, true).map(Value::Interval);
        }
        (ArithOp::Div, Value::Interval(iv), other) => {
            let Some(factor) = value_to_f64_opt(other) else {
                return Err(Error::Type(format!(
                    "operator {} requires numeric operands",
                    op.symbol()
                )));
            };
            return interval_scale(iv, factor, false).map(Value::Interval);
        }
        // Date arithmetic (Postgres `pg_operator.dat`): `date ± int4 → date` (a day
        // shift, with the commutative `int4 + date`), `date - date → int4` (day
        // count), and `date ± interval → timestamp` (UTC midnight then the 3-pass
        // calendar — result is timestamp, never date). Only int2/int4 day counts —
        // Postgres has no `date + int8` operator, so a bigint (or any other RHS)
        // falls through to the loud non-numeric error below. Overflow fails loud.
        (ArithOp::Add, Value::Date(d), Value::Int2(n))
        | (ArithOp::Add, Value::Int2(n), Value::Date(d)) => return date_shift(*d, i64::from(*n)),
        (ArithOp::Add, Value::Date(d), Value::Int4(n))
        | (ArithOp::Add, Value::Int4(n), Value::Date(d)) => return date_shift(*d, i64::from(*n)),
        (ArithOp::Sub, Value::Date(d), Value::Int2(n)) => return date_shift(*d, -i64::from(*n)),
        (ArithOp::Sub, Value::Date(d), Value::Int4(n)) => return date_shift(*d, -i64::from(*n)),
        (ArithOp::Sub, Value::Date(l), Value::Date(r)) => {
            let (Infinite::Finite(l), Infinite::Finite(r)) = (l, r) else {
                return Err(Error::CannotSubtractInfiniteDates);
            };
            let days = l
                .to_julian_day()
                .checked_sub(r.to_julian_day())
                .ok_or(Error::NumericOutOfRange("integer"))?;
            return Ok(Value::Int4(days));
        }
        (ArithOp::Add, Value::Date(d), Value::Interval(i))
        | (ArithOp::Add, Value::Interval(i), Value::Date(d)) => {
            return timestamp_add_interval(date_to_timestamp(*d), i, false).map(Value::Timestamp);
        }
        (ArithOp::Sub, Value::Date(d), Value::Interval(i)) => {
            return timestamp_add_interval(date_to_timestamp(*d), i, true).map(Value::Timestamp);
        }
        // Time arithmetic (Postgres `pg_operator.dat`): `time - time → interval` (a
        // microsecond duration, possibly negative), `time ± interval → time` (shift
        // the time-of-day by the interval's µs field — the day/month fields are
        // ignored, matching `time_pl_interval` — wrapping into `[0, 24h)`), and
        // `date + time → timestamp`. Both `time` operands are in
        // `[0, 86_400_000_000]`, so the difference can't overflow i64.
        (ArithOp::Sub, Value::Time(l), Value::Time(r)) => {
            return Ok(Value::Interval(PgInterval::from_micros(l - r)));
        }
        (ArithOp::Add, Value::Time(t), Value::Interval(i))
        | (ArithOp::Add, Value::Interval(i), Value::Time(t)) => {
            return Ok(Value::Time(time_add_interval_wrap(*t, i.micros, false)));
        }
        (ArithOp::Sub, Value::Time(t), Value::Interval(i)) => {
            return Ok(Value::Time(time_add_interval_wrap(*t, i.micros, true)));
        }
        (ArithOp::Add, Value::Date(d), Value::Time(t))
        | (ArithOp::Add, Value::Time(t), Value::Date(d)) => {
            // `datetime_pl` (through `datetime_timestamp`) answers `22008 timestamp out of
            // range` past its range, and
            // the domain answers the same sentence past pglet's (`time '24:00'` on
            // `9999-12-31` is year 10000).
            // An infinite date stays infinite.
            return date_to_timestamp(*d)
                .try_map(|ts| {
                    ts.checked_add(time::Duration::microseconds(*t))
                        .ok_or(Error::TemporalOutOfRange("timestamp"))
                        .and_then(crate::types::check_timestamp_domain)
                })
                .map(Value::Timestamp);
        }
        _ => {}
    }
    // NUMERIC arithmetic: exact against numeric/int (result stays numeric, with
    // Postgres's per-operator scale rules); a float operand promotes both to
    // float8 via the fallback path below (Postgres casts numeric→float8).
    if (matches!(left, Value::Numeric(_)) || matches!(right, Value::Numeric(_)))
        && let (Some(a), Some(b)) = (as_exact_numeric(left), as_exact_numeric(right))
    {
        let r = match op {
            ArithOp::Add => a.add(&b)?,
            ArithOp::Sub => a.sub(&b)?,
            ArithOp::Mul => a.mul(&b)?,
            ArithOp::Div => a.div(&b)?,
        };
        return Ok(Value::Numeric(r));
    }
    // Same-width integer arithmetic keeps the width.
    match (left, right) {
        (Value::Int2(l), Value::Int2(r)) => {
            return int_arith(op, *l, *r, "smallint").map(Value::Int2);
        }
        (Value::Int4(l), Value::Int4(r)) => {
            return int_arith(op, *l, *r, "integer").map(Value::Int4);
        }
        (Value::Int8(l), Value::Int8(r)) => {
            return int_arith(op, *l, *r, "bigint").map(Value::Int8);
        }
        // Same-width **float** arithmetic likewise keeps its width. Postgres registers
        // `real <op> real → real` and computes it in f32; falling through to the f64
        // promotion below produced an 8-byte value under the `real` (4-byte) type both
        // walkers declare, so a binary client could not decode it — and it printed
        // `0.1::real + 0.2::real` as `0.30000000447034836` instead of `0.3`.
        (Value::Float4(l), Value::Float4(r)) => {
            return float4_arith(op, *l, *r).map(Value::Float4);
        }
        _ => {}
    }
    // Mixed integer widths promote to the **wider** operand type, matching
    // Postgres (`int2 + int4 → int4`; `int2/int4 + int8 → int8`). Operand order
    // preserved. The arithmetic runs in i64 (the inputs can't overflow it); for an
    // int4 result the i64 outcome is range-checked back into i32 and errors loud
    // (`22003`) if it doesn't fit, exactly as Postgres's `integer out of range`.
    if let (Some(l), Some(r)) = (value_as_i64(left), value_as_i64(right)) {
        let has_int8 = matches!(left, Value::Int8(_)) || matches!(right, Value::Int8(_));
        return if has_int8 {
            int_arith(op, l, r, "bigint").map(Value::Int8)
        } else {
            let wide = int_arith(op, l, r, "integer")?;
            i32::try_from(wide)
                .map(Value::Int4)
                .map_err(|_| Error::NumericOutOfRange("integer"))
        };
    }
    // At least one operand is a float: promote both to Float8. A non-numeric
    // operand (None) is an error rather than coercing to 0.
    let (Some(l), Some(r)) = (value_to_f64_opt(left), value_to_f64_opt(right)) else {
        return Err(Error::Type(format!(
            "operator {} requires numeric operands",
            op.symbol()
        )));
    };
    float_arith(op, l, r).map(Value::Float8)
}

/// Evaluate `left || right` (concatenation).
///
/// The operand-pair verdict comes from the shared [`concat_legality`] rule (which
/// the DDL-time stored-expression check consults too, so they cannot drift); an
/// accepted pair is one of the **text** forms, rendered here and joined.
///
/// A NULL operand propagates to NULL. **Divergence:** Postgres resolves the
/// operator before evaluating, so `NULL::int || NULL::int` is a type error there,
/// while pglet — gating at eval time, where a NULL carries no type — yields NULL.
///
/// `left_oid` / `right_oid` are the operands' **declared** types when the scope
/// knows them (see `eval::declared_type_oid`) — load-bearing only for `timestamptz`,
/// whose text form carries the `+00` offset that a plain `timestamp` does not.
///
/// # Errors
///
/// Whatever [`concat_legality`] rejects the operand pair with.
pub fn eval_concat(
    left: &Value,
    left_oid: Option<TypeOid>,
    right: &Value,
    right_oid: Option<TypeOid>,
) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    concat_legality(left.type_oid(), right.type_oid())?;
    match (
        concat_operand_text(left, left_oid),
        concat_operand_text(right, right_oid),
    ) {
        (Some(l), Some(r)) => Ok(Value::Text(l + &r)),
        // A legal pair with no text rendering: not reachable for the types pglet
        // models, but stays loud rather than silently yielding NULL.
        _ => Err(concat_undefined(left.type_oid(), right.type_oid())),
    }
}

/// Render one `||` operand as Postgres's `anynonarray → text` **cast** does.
///
/// This is the value's `::text` spelling, which is not always its wire text form:
/// a `bool` casts to `true`/`false` (`booltext`) while the wire encoder emits
/// `t`/`f` (`boolout`), so `true || '!'` is `true!`, not `t!`.
///
/// `oid` is the operand's declared type where the scope knows it, so a
/// `timestamptz` keeps its `+00` offset (`… 12:30:45+00`, Postgres at
/// `TimeZone=UTC`) instead of rendering as the offset-less plain `timestamp` its
/// `Value::Timestamp` would otherwise imply. Without it, `'v=' || tz_col` hands a
/// driver a string it reparses in its own zone — a silent instant shift.
fn concat_operand_text(v: &Value, oid: Option<TypeOid>) -> Option<String> {
    match v {
        Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
        // An array never reaches the text forms (`anynonarray` excludes them). A
        // `Value::Array` whose element type has no array OID reports `None` from
        // `type_oid`, so `concat_legality` can't see it — refuse to stringify it
        // here rather than silently producing a joined literal.
        Value::Array(_) => None,
        other => oid.map_or_else(
            || crate::types::value_to_text(other),
            |o| value_to_text_with_oid(other, o),
        ),
    }
}

/// Coerce `val` to `target_type` for a `CAST` / `::`, honoring the source's
/// **declared** type where the scope knows it.
///
/// Only one distinction needs it: `timestamptz::text` keeps its `+00` offset
/// (Postgres's `timestamptz_out` at `TimeZone=UTC`), while a plain `timestamp`
/// renders without one — and both are `Value::Timestamp`, so the value alone
/// can't decide. A `timestamptz[]` source renders each element that way too, into
/// `text` / `varchar` and into `text[]` / `varchar[]`. Every other cast is value-only
/// and defers to `coerce_value`.
///
/// The scalar walk's (`eval_expr_full`) cast, which a grouping query's expressions take
/// too, so a cast over an aggregate result renders the same as a cast over a column.
///
/// # Errors
///
/// Propagates `coerce_value`'s error for an incompatible value/target pair.
pub fn eval_cast(val: Value, source_oid: Option<TypeOid>, target_type: TypeOid) -> Result<Value> {
    if matches!(target_type, TypeOid::Text | TypeOid::Varchar)
        && let Some(source @ (TypeOid::Timestamptz | TypeOid::TimestamptzArray)) = source_oid
        && let Some(s) = value_to_text_with_oid(&val, source)
    {
        return Ok(Value::Text(s));
    }
    if source_oid == Some(TypeOid::TimestamptzArray)
        && let Some(element @ (TypeOid::Text | TypeOid::Varchar)) = target_type.array_element()
        && let Value::Array(arr) = &val
    {
        let elements = arr
            .elements
            .iter()
            .map(|e| {
                value_to_text_with_oid(e, TypeOid::Timestamptz).map_or(Value::Null, Value::Text)
            })
            .collect();
        return Ok(Value::Array(ArrayValue::new(element, elements)));
    }
    coerce_value(val, target_type)
}

/// Evaluate `left % right` (modulo). Postgres has only `int2/4/8 % (same)` and
/// `numeric % numeric`, so a mixed `int`/`numeric` casts the int to numeric and
/// mixed integer widths promote to the wider; the remainder takes the **dividend's**
/// sign (`(-7) % 3 = -1`, `7 % (-3) = 1`). There is **no** float `%`: a `float4`/
/// `float8` operand on either side resolves to no operator (`42883`), never a
/// silent float modulo. `x % 0` is `22012`. A NULL operand yields NULL.
pub(super) fn eval_modulo(left: &Value, right: &Value) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    // No float `%` — a float operand on either side has no operator.
    if matches!(left, Value::Float4(_) | Value::Float8(_))
        || matches!(right, Value::Float4(_) | Value::Float8(_))
    {
        return Err(modulo_undefined(left, right));
    }
    // NUMERIC modulo (a `numeric` operand, no float): exact, dividend's sign.
    if (matches!(left, Value::Numeric(_)) || matches!(right, Value::Numeric(_)))
        && let (Some(a), Some(b)) = (as_exact_numeric(left), as_exact_numeric(right))
    {
        return a.rem(&b).map(Value::Numeric);
    }
    // Integer modulo — same-width keeps the width; mixed widths promote to the wider.
    match (left, right) {
        (Value::Int2(l), Value::Int2(r)) => return int_rem(*l, *r, "smallint").map(Value::Int2),
        (Value::Int4(l), Value::Int4(r)) => return int_rem(*l, *r, "integer").map(Value::Int4),
        (Value::Int8(l), Value::Int8(r)) => return int_rem(*l, *r, "bigint").map(Value::Int8),
        _ => {}
    }
    if let (Some(l), Some(r)) = (value_as_i64(left), value_as_i64(right)) {
        let has_int8 = matches!(left, Value::Int8(_)) || matches!(right, Value::Int8(_));
        return if has_int8 {
            int_rem(l, r, "bigint").map(Value::Int8)
        } else {
            // The remainder's magnitude is < the divisor's, so an int2/int4 result
            // always fits i32; the try_from guards it defensively.
            let rem = int_rem(l, r, "integer")?;
            i32::try_from(rem)
                .map(Value::Int4)
                .map_err(|_| Error::NumericOutOfRange("integer"))
        };
    }
    // A non-numeric operand (text, uuid, bool): no `%` operator.
    Err(modulo_undefined(left, right))
}

/// `mod(a, b)` / `power(a, b)` / `pow(a, b)` — the operator `op` evaluates as, over the
/// same values, with an operand pair the operator has no signature for named as 18.6
/// names a call's: `function mod(double precision, integer) does not exist`.
pub(super) fn eval_operator_call(op: BinaryOp, left: &Value, right: &Value) -> Result<Value> {
    let result = match op.operator_form() {
        BinaryOp::Mod => eval_modulo(left, right),
        _ => eval_power(left, right),
    };
    result.map_err(|e| match (e, op.call_name()) {
        (Error::UndefinedFunction(_), Some(name)) => Error::UndefinedFunction(format!(
            "function {name}({}, {}) does not exist",
            opt_type_oid_display_name(left.type_oid()),
            opt_type_oid_display_name(right.type_oid())
        )),
        (e, _) => e,
    })
}

/// The "operator does not exist" error (`42883`) for a `%` with an operand type
/// the modulo operator isn't defined for (a float, or a non-numeric value).
fn modulo_undefined(left: &Value, right: &Value) -> Error {
    Error::UndefinedFunction(format!(
        "operator does not exist: {:?} % {:?}",
        left.type_oid(),
        right.type_oid()
    ))
}

/// Evaluate `left ^ right` (exponentiation). Postgres has only `float8 ^ float8`
/// and `numeric ^ numeric`, with `float8` winning the mix and `int ^ int` resolving
/// to `float8` (no integer operator exists). So a `float` operand — or two integers
/// — takes the `float8` path; otherwise (a `numeric` present, no float) the exact
/// `numeric` path. A NULL operand yields NULL.
pub(super) fn eval_power(left: &Value, right: &Value) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    let has_float = matches!(left, Value::Float4(_) | Value::Float8(_))
        || matches!(right, Value::Float4(_) | Value::Float8(_));
    // numeric ^ numeric (int operands cast to numeric): the exact path.
    if !has_float
        && (matches!(left, Value::Numeric(_)) || matches!(right, Value::Numeric(_)))
        && let (Some(base), Some(exp)) = (as_exact_numeric(left), as_exact_numeric(right))
    {
        return base.pow(&exp).map(Value::Numeric);
    }
    // float8 ^ float8 — the default (covers `int ^ int` and any float operand).
    let (Some(base), Some(exp)) = (value_to_f64_opt(left), value_to_f64_opt(right)) else {
        return Err(Error::UndefinedFunction(format!(
            "operator does not exist: {:?} ^ {:?}",
            left.type_oid(),
            right.type_oid()
        )));
    };
    eval_power_f64(base, exp).map(Value::Float8)
}

/// `float8 ^ float8` (Postgres `dpow`): POSIX `pow` NaN rules (`NaN^0 = 1`,
/// `1^NaN = 1`, else `NaN`), then the SQL-mandated domain errors — `0 ^ negative`
/// and negative-base `^` non-integer both `2201F` — and finally a finite-operand
/// overflow to `±inf` as `22003`.
// Exact float comparisons are intentional here (they mirror `dpow`'s `arg1 == 1.0`
// / `floor(arg2) != arg2` domain tests byte-for-byte), not accidental.
#[allow(clippy::float_cmp)]
fn eval_power_f64(base: f64, exp: f64) -> Result<f64> {
    // NaN handling (explicit, per POSIX — not trusting the platform `powf`).
    if base.is_nan() {
        return Ok(if exp.is_nan() || exp != 0.0 {
            f64::NAN
        } else {
            1.0
        });
    }
    if exp.is_nan() {
        return Ok(if base == 1.0 { 1.0 } else { f64::NAN });
    }
    // SQL-mandated domain errors (`2201F`), not a division-by-zero for `0 ^ -1`.
    if base == 0.0 && exp < 0.0 {
        return Err(Error::InvalidArgumentForPowerFunction(
            "zero raised to a negative power is undefined".into(),
        ));
    }
    if base < 0.0 && exp.floor() != exp {
        return Err(Error::InvalidArgumentForPowerFunction(
            "a negative number raised to a non-integer power yields a complex result".into(),
        ));
    }
    let result = base.powf(exp);
    // Finite operands producing a non-finite result is an overflow (`22003`).
    if result.is_infinite() && base.is_finite() && exp.is_finite() {
        return Err(Error::NumericOutOfRange("double precision"));
    }
    Ok(result)
}

/// Shift a time-of-day (µs since midnight) by an interval's microseconds and wrap
/// into `[0, 24h)`, matching Postgres `time_pl_interval` / `time_mi_interval`.
/// Only the interval's µs field participates — the day/month fields are ignored
/// (so `'01:00' + '3 days'` stays `01:00`). `subtract` negates the shift.
/// Computed in `i128` so a pathological interval (or `i64::MIN` under `subtract`)
/// can't overflow; the wrapped result is always in `[0, 86_400_000_000)`.
fn time_add_interval_wrap(t: i64, span_micros: i64, subtract: bool) -> i64 {
    const USECS_PER_DAY: i128 = 86_400_000_000;
    let span = if subtract {
        -i128::from(span_micros)
    } else {
        i128::from(span_micros)
    };
    let mut result = i128::from(t) + span;
    result -= result / USECS_PER_DAY * USECS_PER_DAY; // C trunc-toward-zero.
    if result < 0 {
        result += USECS_PER_DAY;
    }
    result as i64
}

/// Microseconds in a 24 h day (`USECS_PER_DAY`).
const USECS_PER_DAY: i64 = 86_400_000_000;

/// `interval_um`: negate every field of an interval (each overflow-checked). 18.6 also
/// refuses a result at its `infinity` field triple, and this does not (`EDGES.md` row 143).
pub(super) fn interval_negate(iv: &PgInterval) -> Result<PgInterval> {
    let oor = || Error::TemporalOutOfRange("interval");
    Ok(PgInterval {
        months: iv.months.checked_neg().ok_or_else(oor)?,
        days: iv.days.checked_neg().ok_or_else(oor)?,
        micros: iv.micros.checked_neg().ok_or_else(oor)?,
    })
}

/// `interval_pl`: field-wise interval addition, each field overflow-checked (no
/// auto-justify — Postgres keeps `25h + 0` as `25:00:00`). 18.6's infinite operands and
/// its refusal of a sum at an infinity field triple are not modelled (`EDGES.md` row 143).
pub fn interval_add(a: &PgInterval, b: &PgInterval) -> Result<PgInterval> {
    let oor = || Error::TemporalOutOfRange("interval");
    Ok(PgInterval {
        months: a.months.checked_add(b.months).ok_or_else(oor)?,
        days: a.days.checked_add(b.days).ok_or_else(oor)?,
        micros: a.micros.checked_add(b.micros).ok_or_else(oor)?,
    })
}

/// `interval_mi`: field-wise interval subtraction, each field overflow-checked. 18.6's
/// infinite operands and its refusal of a difference at an infinity field triple are not
/// modelled (`EDGES.md` row 143).
fn interval_sub(a: &PgInterval, b: &PgInterval) -> Result<PgInterval> {
    let oor = || Error::TemporalOutOfRange("interval");
    Ok(PgInterval {
        months: a.months.checked_sub(b.months).ok_or_else(oor)?,
        days: a.days.checked_sub(b.days).ok_or_else(oor)?,
        micros: a.micros.checked_sub(b.micros).ok_or_else(oor)?,
    })
}

/// `interval_justify_hours`: fold whole 24 h out of the µs field into `days`
/// (months untouched), matching Postgres's `timestamp - timestamp` post-processing.
fn justify_hours(iv: PgInterval) -> Result<PgInterval> {
    let oor = || Error::TemporalOutOfRange("interval");
    let wholeday = iv.micros / USECS_PER_DAY; // C TMODULO: trunc toward zero
    let mut micros = iv.micros % USECS_PER_DAY;
    // `days` (i32) + `wholeday` (≤ i64::MAX/86_400_000_000 ≈ 1e8) can't overflow i64.
    let mut day = i64::from(iv.days) + wholeday;
    // Same-sign fixup so `day` and `micros` never carry opposite signs.
    if day > 0 && micros < 0 {
        micros += USECS_PER_DAY;
        day -= 1;
    } else if day < 0 && micros > 0 {
        micros -= USECS_PER_DAY;
        day += 1;
    }
    Ok(PgInterval {
        months: iv.months,
        days: i32::try_from(day).map_err(|_| oor())?,
        micros,
    })
}

/// Julian day of `2000-01-01`, day 0 of a Postgres timestamp (`POSTGRES_EPOCH_JDATE`).
pub(super) const POSTGRES_EPOCH_JDATE: i64 = 2_451_545;

/// Julian day of `294277-01-01`, the first day past 18.6's timestamp range
/// (`TIMESTAMP_END_JULIAN`); the range starts at Julian day 0, `4714-11-24 BC`.
pub(super) const TIMESTAMP_END_JULIAN: i64 = 109_203_528;

/// `timestamp_pl_interval` (and `timestamp_mi_interval`, which negates the interval
/// first — `22008 interval out of range` for a field that cannot be negated, though not
/// for a negation landing on the field triple 18.6 reads as `infinity` and refuses
/// there, `PgInterval` having no infinity: `EDGES.md` row 143): three
/// passes, month shift with day-of-month clamp (leap-aware) → flat Julian day count →
/// microsecond shift, each re-deriving the calendar from the prior result and keeping
/// the time of day.
///
/// The passes run over plain integers — a proleptic Gregorian year and a Julian day
/// count — never over `time`'s types, whose years end at ±9999. 18.6 refuses a months
/// or days pass that lands outside its **own** timestamp range (Julian day
/// `0 .. TIMESTAMP_END_JULIAN`), and so does this: a pass into year 10000 is carried on
/// (`9999-12-31 + '1 day -1 microsecond'` is a value), a pass below `4714-11-24 BC` is
/// refused even when a later pass would come back. The microsecond pass's own range
/// check lies outside the domain on both sides, so the domain check on the result
/// (`22008 timestamp out of range`, the same sentence) stands in for it. The calendar is
/// read from `ts`'s fields at the offset it carries, as `docs/temporal.md` records for a
/// timestamp an older build stored off UTC.
///
/// An infinite `ts` is the answer, after the negation — which 18.6 also runs first, so
/// `'infinity'::timestamp - interval '-2147483648 days'` is its `22008` on both.
fn timestamp_add_interval(
    ts: crate::types::PgTimestamp,
    iv: &PgInterval,
    subtract: bool,
) -> Result<crate::types::PgTimestamp> {
    let oor = || Error::TemporalOutOfRange("timestamp");
    let span = if subtract { interval_negate(iv)? } else { *iv };
    let Infinite::Finite(ts) = ts else {
        return Ok(ts);
    };
    let in_range = |jd: i64| {
        if (0..TIMESTAMP_END_JULIAN).contains(&jd) {
            Ok(jd)
        } else {
            Err(oor())
        }
    };
    let date = ts.date();
    // Pass 1 — months: shift the year/month, then clamp the day-of-month to the
    // target month's length (Jan 31 + 1 mon → Feb 28/29, leap-aware).
    let mut jd = if span.months == 0 {
        i64::from(date.to_julian_day())
    } else {
        let total = i64::from(date.year()) * 12 + i64::from(u8::from(date.month())) - 1
            + i64::from(span.months);
        let (year, month) = (total.div_euclid(12), total.rem_euclid(12) + 1);
        let day = i64::from(date.day()).min(days_in_month(year, month));
        in_range(julian_day(year, month, day))?
    };
    // Pass 2 — days: a flat Julian day count (no calendar interpretation).
    if span.days != 0 {
        jd = in_range(jd + i64::from(span.days))?;
    }
    // Pass 3 — microseconds, over 18.6's µs-since-2000 count. A Julian day inside the
    // range keeps that count inside `i64`.
    let (hour, minute, second, micro) = ts.time().as_hms_micro();
    let time_of_day = ((i64::from(hour) * 60 + i64::from(minute)) * 60 + i64::from(second))
        * 1_000_000
        + i64::from(micro);
    let micros = ((jd - POSTGRES_EPOCH_JDATE) * USECS_PER_DAY + time_of_day)
        .checked_add(span.micros)
        .ok_or_else(oor)?;
    let day = i32::try_from(micros.div_euclid(USECS_PER_DAY) + POSTGRES_EPOCH_JDATE)
        .map_err(|_| oor())?;
    time::Date::from_julian_day(day)
        .map_err(|_| oor())?
        .midnight()
        .assume_utc()
        .checked_add(time::Duration::microseconds(
            micros.rem_euclid(USECS_PER_DAY),
        ))
        .ok_or_else(oor)
        .and_then(crate::types::check_timestamp_domain)
        .map(Infinite::Finite)
}

/// The days in `month` (1–12) of a proleptic Gregorian `year`, inside `time`'s years or not.
pub(super) const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The Julian day of a proleptic Gregorian date, inside `time`'s years or not (`date2j`,
/// over floor division so a year before `-4800` stays exact).
pub(super) const fn julian_day(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era + 1_721_120
}

/// The interval scaling shared by `interval_mul` (`is_mul`) and `interval_div`:
/// the number operand is `factor`; fractions of the month and day products cascade
/// **down** (month→days via `DAYS_PER_MONTH`, day→seconds via `SECS_PER_DAY`),
/// never up. Follows `timestamp.c` byte-for-byte, including the `TSROUND`
/// microsecond rounding and the `>24h` fold, short of 18.6's infinite intervals, which
/// are not modelled (`EDGES.md` row 143). Overflow → `22008`; division by zero → `22012`.
pub fn interval_scale(span: &PgInterval, factor: f64, is_mul: bool) -> Result<PgInterval> {
    let oor = || Error::TemporalOutOfRange("interval");
    if !is_mul && factor == 0.0 {
        return Err(Error::DivisionByZero);
    }
    if !factor.is_finite() {
        return Err(oor());
    }
    let apply = |x: f64| if is_mul { x * factor } else { x / factor };
    let orig_month = f64::from(span.months);
    let orig_day = f64::from(span.days);

    let month_d = apply(orig_month);
    let month = f64_to_i32_trunc(month_d).ok_or_else(oor)?;
    let day_d = apply(orig_day);
    let mut day = f64_to_i32_trunc(day_d).ok_or_else(oor)?;

    let month_remainder_days = tsround((apply(orig_month) - f64::from(month)) * 30.0);
    let mut sec_remainder = tsround(
        (apply(orig_day) - f64::from(day) + month_remainder_days - month_remainder_days.trunc())
            * 86_400.0,
    );

    // A rounded/cascaded remainder ≥ 24h folds into whole days.
    if sec_remainder.abs() >= 86_400.0 {
        let extra = (sec_remainder / 86_400.0) as i32;
        day = day.checked_add(extra).ok_or_else(oor)?;
        sec_remainder -= f64::from(extra) * 86_400.0;
    }
    day = day
        .checked_add(month_remainder_days as i32)
        .ok_or_else(oor)?;

    let time_d = (apply(span.micros as f64) + sec_remainder * 1_000_000.0).round_ties_even();
    if !time_d.is_finite() || !fits_i64(time_d) {
        return Err(oor());
    }
    Ok(PgInterval {
        months: month,
        days: day,
        micros: time_d as i64,
    })
}

/// Postgres `TSROUND`: round to microsecond precision (`rint(x * 1e6) / 1e6`),
/// ties to even (C `rint`).
fn tsround(x: f64) -> f64 {
    (x * 1_000_000.0).round_ties_even() / 1_000_000.0
}

/// Truncate a float toward zero to `i32`, or `None` if it is NaN or outside
/// `i32`'s range — Postgres's `FLOAT8_FITS_IN_INT32` guard before an `(int32)` cast.
fn f64_to_i32_trunc(x: f64) -> Option<i32> {
    if x.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&x) {
        return None;
    }
    Some(x as i32)
}

/// Whether a float fits `i64`'s range — Postgres's `FLOAT8_FITS_IN_INT64` guard.
fn fits_i64(x: f64) -> bool {
    (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x)
}

/// Shift a `date` by a (possibly negative) day count, Julian-day based to match
/// the binary codec's epoch handling. A result outside the domain fails loud with
/// `22008 date out of range` — `date_pli` / `date_mii`'s own code and sentence where
/// 18.6's range ends, and the domain's refusal where only pglet's does
/// (`'0001-01-01'::date - 1` is `0001-12-31 BC` there). An infinity is left as it is.
fn date_shift(d: crate::types::PgDate, days: i64) -> Result<Value> {
    let oor = || Error::TemporalOutOfRange("date");
    d.try_map(|d| {
        let jd = i64::from(d.to_julian_day())
            .checked_add(days)
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(oor)?;
        time::Date::from_julian_day(jd)
            .map_err(|_| oor())
            .and_then(crate::types::check_date_domain)
    })
    .map(Value::Date)
}

/// Promote an integer or `NUMERIC` `Value` to an exact [`PgNumeric`] for numeric
/// comparison/arithmetic; `None` for a float or non-numeric value (which routes
/// through the `f64` path, matching Postgres's numeric→float8 mixed operators).
pub(super) fn as_exact_numeric(v: &Value) -> Option<PgNumeric> {
    match v {
        Value::Numeric(n) => Some(n.clone()),
        Value::Int2(n) => Some(PgNumeric::from_i128(i128::from(*n))),
        Value::Int4(n) => Some(PgNumeric::from_i128(i128::from(*n))),
        Value::Int8(n) => Some(PgNumeric::from_i128(i128::from(*n))),
        _ => None,
    }
}
