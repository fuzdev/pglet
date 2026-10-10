//! Value coercion for query execution: explicit `CAST` and implicit type
//! coercion between the engine's `Value` variants.

use crate::planner::Typmod;
use crate::types::{
    ArrayValue, PgNumeric, TypeOid, Value, parse_jsonb, round_interval_to_precision,
    round_time_to_precision, round_timestamp_to_precision,
};
use crate::{Error, Result};

use super::array_parse::parse_array_literal;

/// Apply a type modifier ([`Typmod`]) to an already-coerced value — the single
/// enforcer shared by the column-store path ([`super::constraints::coerce_column_value`],
/// `is_explicit = false`) and the explicit-cast path ([`Expr::CastTypmod`](crate::planner::Expr::CastTypmod),
/// `is_explicit = true`).
///
/// - **Numeric** (`numeric(p,s)`): round to `scale` and range-check, ignoring
///   `is_explicit` — Postgres enforces a numeric typmod identically for a column
///   store and an explicit cast (overflow → `22003`).
/// - **Length** (`varchar(n)` / `char(n)`, a `Value::Text`): counted in
///   **characters** (UTF-8), not bytes. Within `n` → unchanged. Over `n`: an
///   **explicit cast** (`is_explicit`) always truncates to the leading `n`
///   characters; a **column store** truncates iff every excess character is a blank
///   (space), else fails loud (`22001`, `Error::StringDataRightTruncation`).
///
/// - **Precision** (`timestamp(p)` / `timestamptz(p)` / `time(p)` / `interval(p)`):
///   round the value's microseconds to `p` fractional digits, half **away from
///   zero** about the value's own internal zero, ignoring `is_explicit` — 18.6 runs
///   the same `Adjust*ForTypmod` for a column store and an explicit cast. The arms
///   key on the **value variant**, not on `type_oid`: `Value::Timestamp` serves
///   `timestamp` and `timestamptz` alike (only the column OID carries the tz bit),
///   and `interval` rounds its `micros` field alone. Rounding can carry past the
///   representable range (`'9999-12-31 23:59:59.9999'::timestamp(3)`), which is
///   `22008` here and a year-10000 value on 18.6 — loud rather than a silently kept
///   unrounded value.
///
/// - **Array**: the modifier is the **element's** (`varchar(3)[]` is an array of
///   `varchar(3)`), so it is applied to each element under the same `is_explicit`
///   context — 18.6's `ArrayCoerceExpr`, which runs the element cast per element.
///   Measured on 18.6: `'{abcdefg,xy}'::varchar(3)[]` is `{abc,xy}` and the same
///   literal stored into a `varchar(3)[]` column is `22001`.
///
/// A value whose variant doesn't match the modifier (e.g. a NULL, or a non-text
/// value under a `Length` typmod) passes through unchanged.
///
/// # Errors
///
/// `Error::NumericOutOfRange` (`22003`) on a numeric precision overflow,
/// `Error::StringDataRightTruncation` (`22001`) on a column-store length over-run
/// with non-blank excess, or `Error::TemporalOutOfRange` (`22008`) when a precision
/// rounding carries out of range — from an element as readily as from a scalar.
pub fn apply_typmod(
    value: Value,
    _type_oid: TypeOid,
    typmod: Typmod,
    is_explicit: bool,
) -> Result<Value> {
    match (value, typmod) {
        (Value::Numeric(mut n), Typmod::Numeric { precision, scale }) => {
            n.apply_typmod(precision, scale)?;
            Ok(Value::Numeric(n))
        }
        (Value::Text(s), Typmod::Length(n)) => apply_length_typmod(s, n, is_explicit),
        (Value::Timestamp(dt), Typmod::Precision(p)) => {
            Ok(Value::Timestamp(round_timestamp_to_precision(dt, p)?))
        }
        (Value::Time(us), Typmod::Precision(p)) => Ok(Value::Time(round_time_to_precision(us, p)?)),
        (Value::Interval(iv), Typmod::Precision(p)) => {
            Ok(Value::Interval(round_interval_to_precision(iv, p)?))
        }
        (Value::Array(arr), typmod) => {
            let ArrayValue {
                element_type,
                elements,
            } = arr;
            let elements = elements
                .into_iter()
                .map(|e| apply_typmod(e, element_type, typmod, is_explicit))
                .collect::<Result<Vec<_>>>()?;
            Ok(Value::Array(ArrayValue::new(element_type, elements)))
        }
        // NULL, or a value variant that doesn't carry this modifier — leave as-is.
        (value, _) => Ok(value),
    }
}

/// Enforce a `varchar(n)` / `char(n)` character-length modifier on a text value.
/// See [`apply_typmod`] for the truncate-vs-error rules.
fn apply_length_typmod(s: String, n: u32, is_explicit: bool) -> Result<Value> {
    let limit = n as usize;
    // Length is character-based (UTF-8), not byte-based — count chars, don't index
    // by `str::len`.
    if s.chars().count() <= limit {
        return Ok(Value::Text(s));
    }
    // Over length. An explicit cast always truncates; a column store truncates only
    // when every excess character is a blank (space), else fails loud (`22001`).
    if is_explicit || s.chars().skip(limit).all(|c| c == ' ') {
        Ok(Value::Text(s.chars().take(limit).collect()))
    } else {
        Err(Error::StringDataRightTruncation { max_length: n })
    }
}

/// Coerce a value to the target type.
///
/// This handles both explicit CAST and implicit type coercion.
/// Returns an error if the conversion is not possible.
pub fn coerce_value(val: Value, target: TypeOid) -> Result<Value> {
    // NULL stays NULL
    if val.is_null() {
        return Ok(Value::Null);
    }

    // If already the right type, return as-is
    if let Some(current_type) = val.type_oid()
        && current_type == target
    {
        return Ok(val);
    }

    match (val, target) {
        // Text to numeric types
        (Value::Text(s), TypeOid::Int2) => parse_int::<i16>(&s, "smallint").map(Value::Int2),
        (Value::Text(s), TypeOid::Int4) => parse_int::<i32>(&s, "integer").map(Value::Int4),
        (Value::Text(s), TypeOid::Int8) => parse_int::<i64>(&s, "bigint").map(Value::Int8),
        // `float4in` / `float8in`: a decimal outside the type's range is `22003`, never
        // the `±Infinity` / `0` it rounds to.
        (Value::Text(s), TypeOid::Float4) => crate::types::parse_float4_text(&s).map(Value::Float4),
        (Value::Text(s), TypeOid::Float8) => crate::types::parse_float8_text(&s).map(Value::Float8),
        (Value::Text(s), TypeOid::Bool) => parse_bool(&s).map(Value::Bool),

        // Text → varchar: pglet stores varchar as `Value::Text`, so this is
        // identity. The `Varchar` OID carries no length, so an explicit
        // `'x'::varchar(n)` applies no truncation (a non-spine edge vs Postgres).
        (Value::Text(s), TypeOid::Varchar) => Ok(Value::Text(s)),

        // Numeric to text
        (Value::Int2(n), TypeOid::Text | TypeOid::Varchar) => Ok(Value::Text(n.to_string())),
        (Value::Int4(n), TypeOid::Text | TypeOid::Varchar) => Ok(Value::Text(n.to_string())),
        (Value::Int8(n), TypeOid::Text | TypeOid::Varchar) => Ok(Value::Text(n.to_string())),
        // A float renders through the wire's own renderer, so `±Infinity` is spelled as
        // `float8out` spells it rather than Rust's `inf` — the text `date_part` of an
        // infinite date casts to.
        (v @ (Value::Float4(_) | Value::Float8(_)), TypeOid::Text | TypeOid::Varchar) => Ok(
            Value::Text(crate::types::value_to_text(&v).unwrap_or_default()),
        ),
        (Value::Bool(b), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(if b { "true" } else { "false" }.to_string()))
        }

        // Numeric type promotions
        (Value::Int2(n), TypeOid::Int4) => Ok(Value::Int4(i32::from(n))),
        (Value::Int2(n), TypeOid::Int8) => Ok(Value::Int8(i64::from(n))),
        (Value::Int2(n), TypeOid::Float4) => Ok(Value::Float4(f32::from(n))),
        (Value::Int2(n), TypeOid::Float8) => Ok(Value::Float8(f64::from(n))),

        (Value::Int4(n), TypeOid::Int8) => Ok(Value::Int8(i64::from(n))),
        (Value::Int4(n), TypeOid::Float4) => Ok(Value::Float4(n as f32)),
        (Value::Int4(n), TypeOid::Float8) => Ok(Value::Float8(f64::from(n))),

        (Value::Int8(n), TypeOid::Float8) => Ok(Value::Float8(n as f64)),
        // `int8 -> float4` is an implicit cast in `pg_cast` like its three neighbours,
        // and its absence made `CREATE TABLE t (x float4 DEFAULT 5)` a `42846` where
        // 18.6 accepts and reports `column_default` `5` — the literal is an `int4`
        // `Const` there and an `i64` here, so this pair is the one a bare integer
        // default reaches. `as f32` rounds to nearest, as the C cast does; no `i64`
        // overflows `f32`.
        (Value::Int8(n), TypeOid::Float4) => Ok(Value::Float4(n as f32)),

        (Value::Float4(n), TypeOid::Float8) => Ok(Value::Float8(f64::from(n))),

        // Numeric narrowing — correct-or-loud: an out-of-range value errors `22003`
        // (Postgres "<type> out of range") instead of the silent wrap/saturation an
        // `as` cast produces. Matches the arithmetic path (`eval_arithmetic`).
        (Value::Int8(n), TypeOid::Int4) => i32::try_from(n)
            .map(Value::Int4)
            .map_err(|_| Error::NumericOutOfRange("integer")),
        (Value::Int8(n), TypeOid::Int2) => i16::try_from(n)
            .map(Value::Int2)
            .map_err(|_| Error::NumericOutOfRange("smallint")),
        (Value::Int4(n), TypeOid::Int2) => i16::try_from(n)
            .map(Value::Int2)
            .map_err(|_| Error::NumericOutOfRange("smallint")),
        // float8 → real: precision loss is allowed, but a finite double beyond
        // `real`'s range (→ ±inf) is an overflow. inf/NaN inputs pass through.
        (Value::Float8(n), TypeOid::Float4) => {
            let f = n as f32;
            if n.is_finite() && f.is_infinite() {
                Err(Error::NumericOutOfRange("real"))
            } else {
                Ok(Value::Float4(f))
            }
        }
        (Value::Float8(n), TypeOid::Int8) => float_to_i64(n, "bigint").map(Value::Int8),
        (Value::Float8(n), TypeOid::Int4) => narrow_float_to_i32(n),
        (Value::Float4(n), TypeOid::Int4) => narrow_float_to_i32(f64::from(n)),

        // NUMERIC casts. int/text→numeric is exact. float8→numeric uses its own
        // 15-significant-digit spelling (Postgres `float8_numeric`'s `%.*g` with
        // `DBL_DIG`) and float4→numeric its own 6-digit one (`FLT_DIG`), so each
        // reflects the float's real precision, not the extra digits its shortest
        // round-trip carries — see `float8_to_numeric` / `float4_to_numeric`.
        // numeric→int rounds half-away then range-checks (loud `22003` on overflow).
        // numeric→float8 is the lossy double cast; numeric→float4 is a single
        // correctly-rounded decimal→real step (`numeric_to_float4`, loud `22003` on
        // overflow). numeric→numeric identity is handled by the same-type short-circuit
        // above; a column's `NUMERIC(p,s)` scale is enforced separately on store.
        (Value::Text(s), TypeOid::Numeric) => PgNumeric::parse(&s).map(Value::Numeric),
        (Value::Numeric(nn), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(nn.to_display_string()))
        }
        (Value::Int2(n), TypeOid::Numeric) => {
            Ok(Value::Numeric(PgNumeric::from_i128(i128::from(n))))
        }
        (Value::Int4(n), TypeOid::Numeric) => {
            Ok(Value::Numeric(PgNumeric::from_i128(i128::from(n))))
        }
        (Value::Int8(n), TypeOid::Numeric) => {
            Ok(Value::Numeric(PgNumeric::from_i128(i128::from(n))))
        }
        (Value::Float4(n), TypeOid::Numeric) => float4_to_numeric(n).map(Value::Numeric),
        (Value::Float8(n), TypeOid::Numeric) => float8_to_numeric(n).map(Value::Numeric),
        (Value::Numeric(nn), TypeOid::Int8) => nn.to_i128().and_then(|v| {
            i64::try_from(v)
                .map(Value::Int8)
                .map_err(|_| Error::NumericOutOfRange("bigint"))
        }),
        (Value::Numeric(nn), TypeOid::Int4) => nn.to_i128().and_then(|v| {
            i32::try_from(v)
                .map(Value::Int4)
                .map_err(|_| Error::NumericOutOfRange("integer"))
        }),
        (Value::Numeric(nn), TypeOid::Int2) => nn.to_i128().and_then(|v| {
            i16::try_from(v)
                .map(Value::Int2)
                .map_err(|_| Error::NumericOutOfRange("smallint"))
        }),
        (Value::Numeric(nn), TypeOid::Float8) => numeric_to_float8(&nn).map(Value::Float8),
        (Value::Numeric(nn), TypeOid::Float4) => numeric_to_float4(&nn).map(Value::Float4),

        // Text to timestamp. Both store as a UTC `Value::Timestamp` (timestamptz has
        // no distinct value), but the *input* handling of an explicit offset differs,
        // mirroring Postgres's `timestamp_in` vs `timestamptz_in`: a plain `timestamp`
        // DROPS any offset in the input (keeps the wall-clock fields), a `timestamptz`
        // APPLIES it (converts to UTC). This is the load-bearing input mirror of the
        // offset-aware text *output* (`value_to_text_with_oid`) — a plain-timestamp
        // target must not silently shift the instant.
        (Value::Text(s), TypeOid::Timestamp) => {
            crate::types::parse_timestamp_naive(&s).map(Value::Timestamp)
        }
        (Value::Text(s), TypeOid::Timestamptz) => {
            crate::types::parse_timestamp(&s).map(Value::Timestamp)
        }

        // A timestamp value satisfies both timestamp and timestamptz columns
        // (timestamptz shares the same UTC value representation) — e.g. `now()`.
        (Value::Timestamp(ts), TypeOid::Timestamp | TypeOid::Timestamptz) => {
            Ok(Value::Timestamp(ts))
        }

        // Timestamp to text
        (Value::Timestamp(ts), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(crate::types::format_timestamp_text(&ts)))
        }

        // Text ↔ date (`'2024-01-01'::date`, a date literal into a DATE column, `d::text`).
        (Value::Text(s), TypeOid::Date) => crate::types::parse_date(&s).map(Value::Date),
        (Value::Date(d), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(crate::types::format_date_text(&d)))
        }
        // Timestamp → date truncates the time-of-day (Postgres `ts::date`); date →
        // timestamp is midnight UTC (Postgres's implicit date→timestamp), so a DATE
        // compares against `now()`/timestamp columns.
        //
        // Both build a new value, so both judge the date domain on what they are handed:
        // a producer never hands them one outside it, but a value a store written without
        // the domain holds is read back as stored, and the `timestamp → date` half reads
        // the instant's UTC date rather than its fields at whatever offset it carries.
        // An infinity casts to the same infinity (`timestamp_date`, `date2timestamp`).
        (Value::Timestamp(ts), TypeOid::Date) => ts
            .try_map(|ts| crate::types::check_timestamp_domain(ts).map(time::OffsetDateTime::date))
            .map(Value::Date),
        (Value::Date(d), TypeOid::Timestamp | TypeOid::Timestamptz) => d
            .try_map(|d| crate::types::check_date_domain(d).map(|d| d.midnight().assume_utc()))
            .map(Value::Timestamp),

        // Text ↔ time (`'13:45:00'::time`, a time literal into a TIME column, `t::text`).
        (Value::Text(s), TypeOid::Time) => crate::types::parse_time(&s).map(Value::Time),
        (Value::Time(t), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(crate::types::format_time_text(t)))
        }
        // Timestamp → time truncates to the time-of-day µs (Postgres `ts::time`);
        // pglet is UTC-only, so `timestamptz::time` == `timestamp::time`. There is no
        // `time → timestamp` cast (Postgres has none), and no `time ↔ interval` cast
        // (out of scope) — both fall through to the loud catch-all below. An infinity
        // has no time of day and casts to NULL (`timestamp_time`).
        (Value::Timestamp(ts), TypeOid::Time) => {
            let Some(ts) = ts.finite() else {
                return Ok(Value::Null);
            };
            let micros = (i64::from(ts.hour()) * 60 + i64::from(ts.minute())) * 60_000_000
                + i64::from(ts.second()) * 1_000_000
                + i64::from(ts.microsecond());
            Ok(Value::Time(micros))
        }

        // Text ↔ interval (`'1 day'::interval`, an interval literal into an INTERVAL
        // column, `iv::text`). Parsing accepts the three Postgres grammars; a
        // malformed literal fails loud (`22007`).
        (Value::Text(s), TypeOid::Interval) => {
            crate::types::PgInterval::parse(&s).map(Value::Interval)
        }
        (Value::Interval(iv), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(crate::types::format_interval_text(&iv)))
        }

        // `time → interval` — Postgres's `time_interval`, an **implicit** cast (the only
        // one crossing the `D`/`T` category line). A time-of-day is a span of that many
        // microseconds from midnight, with no month or day component. It is what makes
        // `NULLIF(time_col, interval_col)` resolve to `interval` (the comparison
        // operator resolution coerces the `time` side), and what lets `time` and
        // `interval` compare at all.
        (Value::Time(micros), TypeOid::Interval) => Ok(Value::Interval(crate::types::PgInterval {
            months: 0,
            days: 0,
            micros,
        })),

        // Text to UUID
        (Value::Text(s), TypeOid::Uuid) => crate::types::parse_uuid_bytes(&s)
            .map(Value::Uuid)
            .ok_or(Error::InvalidInputSyntax {
                type_name: "uuid",
                value: s,
            }),

        // UUID to text
        (Value::Uuid(bytes), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(crate::types::format_uuid(&bytes)))
        }

        // Text to JSONB (parse JSON string)
        (Value::Text(s), TypeOid::Jsonb) => parse_jsonb(&s).map(Value::Jsonb),

        // JSONB to text (serialize JSON)
        (Value::Jsonb(j), TypeOid::Text | TypeOid::Varchar) => Ok(Value::Text(j.to_json_string())),

        // Text to BYTEA (`byteain`: the hex format behind `\x`, the escape format otherwise)
        (Value::Text(s), TypeOid::Bytea) => parse_bytea(&s).map(Value::Bytea),

        // BYTEA to text (hex format)
        (Value::Bytea(b), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(crate::types::format_bytea_hex(&b)))
        }

        // Bool conversions
        (Value::Bool(b), TypeOid::Int4) => Ok(Value::Int4(i32::from(b))),
        (Value::Int4(n), TypeOid::Bool) => Ok(Value::Bool(n != 0)),

        // Array → text: render the Postgres `{...}` array literal.
        (Value::Array(arr), TypeOid::Text | TypeOid::Varchar) => {
            Ok(Value::Text(arr.to_text_format()))
        }
        // Array → array: coerce each element to the target's element type.
        (Value::Array(arr), t) if t.is_array() => {
            let target_elem = t.array_element().unwrap_or(TypeOid::Text);
            let elements = arr
                .elements
                .into_iter()
                .map(|e| coerce_value(e, target_elem))
                .collect::<Result<Vec<_>>>()?;
            Ok(Value::Array(ArrayValue::new(target_elem, elements)))
        }
        // Text → array: parse the `{...}` literal into the target element type.
        (Value::Text(s), t) if t.is_array() => {
            let target_elem = t.array_element().unwrap_or(TypeOid::Text);
            parse_array_literal(&s, target_elem).map(Value::Array)
        }

        // `name`, `"char"` and `oid` are catalog **declaration** types with no value
        // shape of their own (see their `TypeOid` docs): a `name` and a `"char"` are a
        // `Value::Text`, an `oid` a `Value::Int4`. Coercing into `name` or `oid` is
        // therefore the identity — which is what makes `conrelid != 0` resolve, one of
        // the two comparisons the fuz spine's constraint snapshot writes.
        // Grouped per family for documentation; the identity arms share a body
        // deliberately — the point is that neither type changes the value's shape.
        #[allow(clippy::match_same_arms)]
        (v @ Value::Text(_), TypeOid::Name) => Ok(v),
        // `"char"` is **one character wide**, and `charin` truncates rather than
        // refusing: measured, `WHERE contype = 'nx'` matches every `'n'` row on 18.6 and
        // matched **none** here while this arm was the identity — silent zero rows on
        // the other comparison the spine's snapshot writes. 18.6 truncates to the first
        // **byte** (`'é'::"char"` is the lone `\303`); a `Value::Text` is a Rust
        // `String` and cannot hold one, so this takes the first *character* — identical
        // for every ASCII value, which is every value a catalog `"char"` column carries.
        // The multibyte case is recorded in `EDGES.md`.
        (Value::Text(s), TypeOid::Char) => Ok(Value::Text(
            s.chars().next().map(String::from).unwrap_or_default(),
        )),
        #[allow(clippy::match_same_arms)]
        (Value::Int2(n), TypeOid::Oid) => Ok(Value::Int4(i32::from(n))),
        #[allow(clippy::match_same_arms)]
        (v @ Value::Int4(_), TypeOid::Oid) => Ok(v),
        (Value::Int8(n), TypeOid::Oid) => i32::try_from(n)
            .map(Value::Int4)
            .map_err(|_| Error::NumericOutOfRange("oid out of range")),

        // Unsupported conversion.
        //
        // Both names are **SQL** spellings, `format_type`'s, because this message goes
        // to a client: a `Debug` rendering here would ship the Rust variant names and
        // the `Option` wrapper with them — `cannot cast type Some(Int8) to Float4` —
        // where 18.6's `42846` reads `cannot cast type <source> to <target>` in
        // `format_type`'s own spellings. **The format is 18.6's; pick the example from a
        // pair 18.6 actually refuses.** `bigint → real` is not one — that cast succeeds
        // there; a measured `42846` is `SELECT 1::bigint::boolean` → `cannot cast type bigint to boolean`
        // or `SELECT now()::real` → `cannot cast type timestamp with time zone to real`.
        // A value whose type is `None` is Postgres's `unknown`, which is what an untyped
        // NULL is there too.
        (val, target) => Err(Error::TypeCoercion {
            from: sql_type_name(val.type_oid()),
            to: sql_type_name(Some(target)),
        }),
    }
}

/// A [`TypeOid`]'s SQL spelling for an error message — `format_type`'s, via
/// `deparse::catalog_type_name` — with `None` (an untyped value) rendering as
/// Postgres's `unknown`.
///
/// The one place a type name enters a client-facing coercion message, so a Rust `Debug`
/// spelling cannot leak back into one. A type the deparser cannot name falls back to
/// `unknown` rather than to its `Debug` form.
fn sql_type_name(oid: Option<TypeOid>) -> String {
    oid.and_then(|t| crate::deparse::catalog_type_name(t).ok())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Round a float to the nearest integer (ties to even, matching Postgres `rint`)
/// and convert to `i64`, erroring `22003` on NaN/infinity or a value outside
/// `i64`'s range — never the silent saturation / `NaN as i64 == 0` an `as` cast
/// gives. `type_name` is the SQL target type for the error message.
fn float_to_i64(n: f64, type_name: &'static str) -> Result<i64> {
    // `i64::MAX as f64` rounds up to 2^63, so the upper bound must be strict.
    const I64_MAX_PLUS_1: f64 = 9_223_372_036_854_775_808.0; // 2^63
    let r = n.round_ties_even();
    if !r.is_finite() || r < i64::MIN as f64 || r >= I64_MAX_PLUS_1 {
        return Err(Error::NumericOutOfRange(type_name));
    }
    Ok(r as i64)
}

/// Narrow a float to `int4`, erroring `22003` ("integer out of range") on
/// NaN/infinity or out-of-range — via [`float_to_i64`] then an `i32` range check.
fn narrow_float_to_i32(n: f64) -> Result<Value> {
    let v = float_to_i64(n, "integer")?;
    i32::try_from(v)
        .map(Value::Int4)
        .map_err(|_| Error::NumericOutOfRange("integer"))
}

/// `float4::numeric` — Postgres `float4_numeric`. Postgres renders the value with
/// `snprintf("%.*g", FLT_DIG, val)` (`FLT_DIG` = 6 significant digits, trailing
/// zeros stripped) and parses that into a numeric, so the result reflects the
/// float4's own 6-digit precision — not the f64-widening noise a `f64::from(n)`
/// capture would introduce (`0.1::float4::numeric` is `0.1`, not `0.10000000149…`).
///
/// NaN → numeric `NaN`. ±Infinity has no finite numeric form → loud `22003` — a
/// **divergence**: Postgres 14+ has numeric `±Infinity`, which pglet's `PgNumeric`
/// does not model.
fn float4_to_numeric(n: f32) -> Result<PgNumeric> {
    if n.is_nan() {
        return Ok(PgNumeric::NaN);
    }
    if n.is_infinite() {
        return Err(Error::NumericOutOfRange("numeric"));
    }
    // Rust's `{:.5e}` is 1 integer + 5 fractional = 6 significant digits.
    PgNumeric::parse(&strip_g_trailing_zeros(&format!("{n:.5e}")))
}

/// `float8::numeric` — Postgres `float8_numeric`. Postgres renders the value with
/// `snprintf("%.*g", DBL_DIG, val)` (`DBL_DIG` = 15 significant digits, trailing
/// zeros stripped) and parses that into a numeric, so the result reflects the
/// `float8`'s own 15-digit precision — not the 16-17 digits its shortest round-trip
/// spelling carries (`(1.0/3.0)::float8::numeric` is `0.333333333333333`, 15 threes,
/// not the 16 a `from_f64` capture gives). This is the float8 analogue of
/// [`float4_to_numeric`]'s 6-digit spelling.
///
/// NaN → numeric `NaN`. ±Infinity has no finite numeric form → loud `22003` — a
/// **divergence**: Postgres 14+ has numeric `±Infinity`, which pglet's `PgNumeric`
/// does not model.
fn float8_to_numeric(n: f64) -> Result<PgNumeric> {
    if n.is_nan() {
        return Ok(PgNumeric::NaN);
    }
    if n.is_infinite() {
        return Err(Error::NumericOutOfRange("numeric"));
    }
    // Rust's `{:.14e}` is 1 integer + 14 fractional = 15 significant digits.
    PgNumeric::parse(&strip_g_trailing_zeros(&format!("{n:.14e}")))
}

/// Strip the mantissa's trailing zeros from a Rust `{:.Ne}` scientific rendering
/// (`"mantissa e exp"`) — the value semantics of C `snprintf("%.*g", …)`, which drops
/// trailing zeros. The result is a scientific-notation decimal string
/// [`PgNumeric::parse`] accepts; dropping the zeros makes the parsed numeric's display
/// scale match Postgres (`2.0000002::float4` → `2`, not `2.00000`). Rust's `{:.Ne}` is
/// correctly rounded, ties to even — as C's default `FE_TONEAREST` — so the shared
/// `float4`/`float8` → numeric casts differ only in their significant-digit count.
fn strip_g_trailing_zeros(s: &str) -> String {
    let (mantissa, exp) = s.split_once('e').unwrap_or((s, "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.trim_end_matches('0').trim_end_matches('.')
    } else {
        mantissa
    };
    format!("{mantissa}e{exp}")
}

/// `numeric::float4` — Postgres `numeric_float4`. Postgres renders the numeric to
/// its exact decimal text (`numeric_out`) and parses that with `float4in`, a
/// **single** correctly-rounded decimal→`real` step. Going numeric→f64→f32 rounds
/// twice and can pick the wrong `f32` neighbour near a tie
/// (`1.00000017881…::numeric::float4` is `1.0000001`, not `1.0000002`), so pglet
/// parses the exact decimal straight to `f32` (Rust float parsing is correctly
/// rounded, round-to-nearest-even).
///
/// A finite numeric outside `real`'s range — beyond it, or nonzero and rounding to
/// zero — is `float4in`'s loud `22003`, its message naming the numeric's
/// text; a numeric `NaN` renders `"NaN"`, which reads as an `f32` NaN (kept, as
/// Postgres does).
fn numeric_to_float4(nn: &PgNumeric) -> Result<f32> {
    // `to_display_string` is the exact `numeric_out` decimal (all digits at the
    // display scale), so this is a single decimal→f32 rounding.
    crate::types::parse_float4_text(&nn.to_display_string())
}

/// `numeric::float8` — Postgres `numeric_float8`, which reads the numeric's `numeric_out`
/// text with `float8in`, so a finite numeric outside `double precision`'s range is that
/// reader's `22003` rather than the `±Infinity` / `0` it rounds to.
fn numeric_to_float8(nn: &PgNumeric) -> Result<f64> {
    crate::types::parse_float8_text(&nn.to_display_string())
}

/// Parse a string as an integer type. `type_name` is the SQL target type for the
/// `22P02` error (`'foo'::int` → "invalid input syntax for type integer").
fn parse_int<T: std::str::FromStr>(s: &str, type_name: &'static str) -> Result<T> {
    s.trim().parse().map_err(|_| Error::InvalidInputSyntax {
        type_name,
        value: s.to_string(),
    })
}

/// Parse a string as a boolean — Postgres's `boolin` spellings, or `22P02`.
///
/// Shared with `executor::expr`'s `boolean_context_operand` and with
/// `planner::legality`'s stored-expression gate, so an `unknown`-typed operand of
/// `IS TRUE` resolves through the **same** grammar a cast to `boolean` does rather
/// than a second, narrower one.
///
/// # Errors
///
/// [`Error::InvalidInputSyntax`] (`22P02`) for a spelling that is not a boolean.
pub fn text_to_bool(s: &str) -> Result<bool> {
    parse_bool(s)
}

/// Parse a string as a boolean — 18.6's `boolin` plus `parse_bool_with_len`
/// (`src/backend/utils/adt/bool.c`), which is a **prefix** grammar rather than a
/// closed list of spellings.
///
/// Any case-insensitive non-empty prefix of `true` / `false` / `yes` / `no` is
/// accepted, so `'tr'`, `'fals'` and `'ye'` are values 18.6 takes and a twelve-member
/// list refused. `on` / `off` are the exception 18.6 spells out — the comparison there
/// is over `max(len, 2)` characters, because a bare `'o'` cannot choose between them —
/// so `'of'` is `false` and `'o'` is `22P02`. `1` and `0` are accepted at length
/// **exactly** one, which is why `'01'` and `'10'` are refused.
///
/// The message names the **untrimmed** input, as `boolin`'s `errmsg` does (`in_str`,
/// not the advanced pointer): `'  zz  '::bool` reports `"  zz  "`.
fn parse_bool(s: &str) -> Result<bool> {
    let refuse = || Error::InvalidInputSyntax {
        type_name: "boolean",
        value: s.to_string(),
    };
    let t = trim_bool_input(s);
    let first = t.as_bytes().first().ok_or_else(refuse)?;
    // A prefix test that is `pg_strncasecmp(value, word, len)` with `value` NUL-padded:
    // a length past the word's own reaches that NUL and cannot match.
    let prefix_of = |word: &str| t.len() <= word.len() && word[..t.len()].eq_ignore_ascii_case(t);
    match first.to_ascii_lowercase() {
        b't' if prefix_of("true") => Ok(true),
        b'f' if prefix_of("false") => Ok(false),
        b'y' if prefix_of("yes") => Ok(true),
        b'n' if prefix_of("no") => Ok(false),
        // 18.6 compares `max(len, 2)` characters here, so length 1 always fails (the
        // padding byte meets `n`/`f`) and length 3 matches only `off` — `on`'s own
        // terminator stops it. Spelled as the two lengths that can match rather than
        // as a padded comparison.
        b'o' if t.len() == 2 && (t.eq_ignore_ascii_case("on") || t.eq_ignore_ascii_case("of")) => {
            Ok(t.eq_ignore_ascii_case("on"))
        }
        b'o' if t.len() == 3 && t.eq_ignore_ascii_case("off") => Ok(false),
        b'1' if t.len() == 1 => Ok(true),
        b'0' if t.len() == 1 => Ok(false),
        _ => Err(refuse()),
    }
}

/// The whitespace `boolin` strips: C `isspace` in the `C` locale — space, `\t`, `\n`,
/// `\v`, `\f`, `\r` — and nothing else.
///
/// Not Rust's `str::trim`, which strips every Unicode `White_Space` code point and so
/// accepted `U&'\00A0' || 't'` as `t` where 18.6 answers `22P02`; and not
/// `u8::is_ascii_whitespace`, which is the same set **minus** `\v`. Both misses were
/// measured against 18.6, in opposite directions.
fn trim_bool_input(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r'))
}

/// Read `s` as a `bytea`, as 18.6's `byteain` does.
///
/// A string that starts `\x` — lowercase, at its first byte — is the **hex format**: pairs
/// of hex digits, a space, tab, newline or carriage return skipped where a pair would
/// start and nowhere else. A character that is no hex digit, and a digit left without its
/// pair, are `22023`, the first one found winning.
///
/// Any other string is the **escape format**: each byte of the text is itself, `\\` is one
/// backslash, and `\ooo` — three octal digits, the first `0` to `3` — is the byte of that
/// value. A backslash before anything else, or ending the string, is `22P02`. Nothing is
/// trimmed, so whitespace is bytes, and `\X` is an invalid escape rather than a prefix.
fn parse_bytea(s: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(s.len());
    if let Some(hex) = s.strip_prefix("\\x") {
        let mut high = None;
        for c in hex.chars() {
            if high.is_none() && matches!(c, ' ' | '\n' | '\t' | '\r') {
                continue;
            }
            // `to_digit` yields a value below 16, so the narrowing keeps it whole
            let Some(digit) = c.to_digit(16).map(|d| d as u8) else {
                return Err(Error::InvalidHexData(format!(
                    "invalid hexadecimal digit: \"{c}\""
                )));
            };
            match high.take() {
                Some(h) => bytes.push(h << 4 | digit),
                None => high = Some(digit),
            }
        }
        if high.is_some() {
            return Err(Error::InvalidHexData(
                "invalid hexadecimal data: odd number of digits".to_string(),
            ));
        }
        return Ok(bytes);
    }
    let mut rest = s.as_bytes();
    while let Some((&first, tail)) = rest.split_first() {
        rest = match (first, tail) {
            (b'\\', [a @ b'0'..=b'3', b @ b'0'..=b'7', c @ b'0'..=b'7', tail @ ..]) => {
                bytes.push((a - b'0') << 6 | (b - b'0') << 3 | (c - b'0'));
                tail
            }
            (b'\\', [b'\\', tail @ ..]) => {
                bytes.push(b'\\');
                tail
            }
            (b'\\', _) => {
                return Err(Error::InvalidInputSyntax {
                    type_name: "bytea",
                    value: s.to_owned(),
                });
            }
            _ => {
                bytes.push(first);
                tail
            }
        };
    }
    Ok(bytes)
}

#[cfg(test)]
mod narrowing_tests {
    use super::*;

    #[test]
    fn int8_to_int4_overflow_is_loud() {
        // `9999999999::int4` must error `22003` (Postgres "integer out of range"),
        // not silently wrap via `as i32`.
        let e = coerce_value(Value::Int8(9_999_999_999), TypeOid::Int4)
            .expect_err("int8→int4 overflow must error");
        assert_eq!(e.sqlstate(), "22003");
        // An in-range value still narrows.
        assert_eq!(
            coerce_value(Value::Int8(42), TypeOid::Int4).unwrap(),
            Value::Int4(42)
        );
    }

    #[test]
    fn int_to_int2_overflow_is_loud() {
        assert_eq!(
            coerce_value(Value::Int8(40_000), TypeOid::Int2)
                .expect_err("int8→int2")
                .sqlstate(),
            "22003"
        );
        assert_eq!(
            coerce_value(Value::Int4(40_000), TypeOid::Int2)
                .expect_err("int4→int2")
                .sqlstate(),
            "22003"
        );
    }

    #[test]
    fn float_to_int_overflow_and_nan_are_loud() {
        // Out of range → 22003 (not a saturated `as` cast).
        assert_eq!(
            coerce_value(Value::Float8(1e30), TypeOid::Int8)
                .expect_err("1e30→int8")
                .sqlstate(),
            "22003"
        );
        assert_eq!(
            coerce_value(Value::Float8(1e30), TypeOid::Int4)
                .expect_err("1e30→int4")
                .sqlstate(),
            "22003"
        );
        // NaN / infinity → loud, not `NaN as i64 == 0`.
        assert!(coerce_value(Value::Float8(f64::NAN), TypeOid::Int8).is_err());
        assert!(coerce_value(Value::Float8(f64::INFINITY), TypeOid::Int4).is_err());
        // In-range rounds (ties to even, matching Postgres `rint`).
        assert_eq!(
            coerce_value(Value::Float8(2.5), TypeOid::Int4).unwrap(),
            Value::Int4(2)
        );
        assert_eq!(
            coerce_value(Value::Float8(3.5), TypeOid::Int4).unwrap(),
            Value::Int4(4)
        );
    }

    #[test]
    fn bytea_invalid_hex_is_22023() {
        // A non-hex digit / odd length in a `\x…` bytea literal is 22023
        // (invalid_parameter_value), matching Postgres's byteain hex path — not the
        // 22P02 the other scalar text parsers use.
        assert_eq!(
            coerce_value(Value::Text("\\xZZ".into()), TypeOid::Bytea)
                .expect_err("non-hex digit")
                .sqlstate(),
            "22023"
        );
        assert_eq!(
            coerce_value(Value::Text("\\xDEA".into()), TypeOid::Bytea)
                .expect_err("odd number of digits")
                .sqlstate(),
            "22023"
        );
        // Valid hex still round-trips.
        assert_eq!(
            coerce_value(Value::Text("\\xDEADBEEF".into()), TypeOid::Bytea).unwrap(),
            Value::Bytea(vec![0xde, 0xad, 0xbe, 0xef])
        );
    }

    /// `s` read as `bytea`: its bytes, or the SQLSTATE of the refusal.
    fn bytea_of(s: &str) -> std::result::Result<Vec<u8>, &'static str> {
        match coerce_value(Value::Text(s.into()), TypeOid::Bytea) {
            Ok(Value::Bytea(bytes)) => Ok(bytes),
            Ok(other) => panic!("{s:?} read as {other:?}"),
            Err(e) => Err(e.sqlstate()),
        }
    }

    // Each row is 18.6's answer, measured. The hex rows are the format's whole rule —
    // lowercase `\x` at the first byte, whitespace only where a pair would start — and the
    // escape rows are what a reader that still hex-decoded, trimmed, or read an octal
    // escape as anything but three octal digits gets wrong.
    #[test]
    fn bytea_input_is_hex_behind_its_prefix_and_the_escape_format_otherwise() {
        for (text, bytes) in [
            ("abcd", &b"abcd"[..]),
            ("abc", b"abc"),
            ("", b""),
            (" ", b" "),
            (" abcd\n", b" abcd\n"),
            ("x6162", b"x6162"),
            ("h\u{e9}", b"h\xc3\xa9"),
            ("\\141\\142", b"ab"),
            ("\\000\\377", b"\x00\xff"),
            ("\\1411", b"a1"),
            ("a\\\\b", b"a\\b"),
            ("\\\\x6162", b"\\x6162"),
            ("\\\\141", b"\\141"),
            ("\\x", b""),
            ("\\x6162", b"ab"),
            ("\\xDeadBEEF", b"\xde\xad\xbe\xef"),
            ("\\x6162 63", b"abc"),
            ("\\x 61\t62\n63\r64 ", b"abcd"),
        ] {
            assert_eq!(bytea_of(text), Ok(bytes.to_vec()), "{text:?}");
        }
        for (text, sqlstate) in [
            ("\\X6162", "22P02"),
            (" \\x6162", "22P02"),
            ("a\\b", "22P02"),
            ("a\\", "22P02"),
            ("\\400", "22P02"),
            ("\\12", "22P02"),
            ("\\128", "22P02"),
            ("\\x616", "22023"),
            ("\\xzz", "22023"),
            ("\\x6 162", "22023"),
            ("\\x61 6 ", "22023"),
            ("\\x61\u{b}62", "22023"),
            ("\\x61\u{c}62", "22023"),
            ("\\x6\u{e9}", "22023"),
        ] {
            assert_eq!(bytea_of(text), Err(sqlstate), "{text:?}");
        }
    }

    // Which of the hex format's two refusals is raised is the first one found, left to
    // right, and the digit one names the character.
    #[test]
    fn bytea_hex_refusals_carry_18_6s_messages() {
        let message = |s: &str| {
            coerce_value(Value::Text(s.into()), TypeOid::Bytea)
                .expect_err(s)
                .to_string()
        };
        assert_eq!(message("\\x6 1"), "invalid hexadecimal digit: \" \"");
        assert_eq!(message("\\x61z"), "invalid hexadecimal digit: \"z\"");
        assert_eq!(
            message("\\x6\u{e9}"),
            "invalid hexadecimal digit: \"\u{e9}\""
        );
        assert_eq!(
            message("\\x616"),
            "invalid hexadecimal data: odd number of digits"
        );
    }

    #[test]
    fn length_typmod_column_store_truncates_blanks_else_loud() {
        // Column store (`is_explicit = false`): within length → unchanged; over
        // length with all-blank excess → truncate; over length with non-blank excess
        // → 22001.
        let len3 = Typmod::Length(3);
        assert_eq!(
            apply_typmod(Value::Text("abc".into()), TypeOid::Varchar, len3, false).unwrap(),
            Value::Text("abc".into())
        );
        assert_eq!(
            apply_typmod(Value::Text("abc  ".into()), TypeOid::Varchar, len3, false).unwrap(),
            Value::Text("abc".into())
        );
        let e = apply_typmod(Value::Text("abcd".into()), TypeOid::Varchar, len3, false)
            .expect_err("non-blank excess must fail loud");
        assert_eq!(e.sqlstate(), "22001");
    }

    #[test]
    fn length_typmod_explicit_cast_always_truncates() {
        // Explicit cast (`is_explicit = true`) truncates silently even with non-blank
        // excess (`'abcd'::varchar(3)` → `'abc'`). Character-based, not byte-based.
        let len3 = Typmod::Length(3);
        assert_eq!(
            apply_typmod(Value::Text("abcd".into()), TypeOid::Varchar, len3, true).unwrap(),
            Value::Text("abc".into())
        );
        // Multibyte: 4 chars (é is one char) truncates to 3 chars, not 3 bytes.
        assert_eq!(
            apply_typmod(Value::Text("é234".into()), TypeOid::Varchar, len3, true).unwrap(),
            Value::Text("é23".into())
        );
    }

    #[test]
    fn an_array_takes_its_elements_modifier_element_wise() {
        // The modifier on an array target is the element's, and 18.6 applies it per
        // element (`ArrayCoerceExpr`), keeping the assignment-vs-explicit split:
        // `'{abcdefg,xy}'::varchar(3)[]` is `{abc,xy}` there, and the same literal
        // stored into a `varchar(3)[]` column is `22001`. Both measured on 18.6.
        let len3 = Typmod::Length(3);
        let arr = |xs: &[&str]| {
            Value::Array(ArrayValue::new(
                TypeOid::Varchar,
                xs.iter().map(|s| Value::Text((*s).into())).collect(),
            ))
        };
        assert_eq!(
            apply_typmod(arr(&["abcdefg", "xy"]), TypeOid::VarcharArray, len3, true).unwrap(),
            arr(&["abc", "xy"])
        );
        let e = apply_typmod(arr(&["abcdefg", "xy"]), TypeOid::VarcharArray, len3, false)
            .expect_err("a non-blank element over-run must fail loud");
        assert_eq!(e.sqlstate(), "22001");
        // A NULL element passes through rather than being counted.
        let with_null = Value::Array(ArrayValue::new(
            TypeOid::Varchar,
            vec![Value::Null, Value::Text("abcdefg".into())],
        ));
        assert_eq!(
            apply_typmod(with_null, TypeOid::VarcharArray, len3, true).unwrap(),
            Value::Array(ArrayValue::new(
                TypeOid::Varchar,
                vec![Value::Null, Value::Text("abc".into())],
            ))
        );
        // The numeric arm too, and its overflow still reaches the caller from inside
        // an element.
        let tm = Typmod::Numeric {
            precision: 4,
            scale: 1,
        };
        let nums = |xs: &[&str]| {
            Value::Array(ArrayValue::new(
                TypeOid::Numeric,
                xs.iter()
                    .map(|s| Value::Numeric(PgNumeric::parse(s).unwrap()))
                    .collect(),
            ))
        };
        assert_eq!(
            apply_typmod(nums(&["1.25", "2.5"]), TypeOid::NumericArray, tm, false).unwrap(),
            nums(&["1.3", "2.5"])
        );
        let e = apply_typmod(nums(&["1.25", "12345.6"]), TypeOid::NumericArray, tm, false)
            .expect_err("an element overflow must fail loud");
        assert_eq!(e.sqlstate(), "22003");
        // The polarity control: an array under a modifier its elements do not carry
        // (a length over an integer array) passes through untouched.
        let ints = Value::Array(ArrayValue::new(TypeOid::Int4, vec![Value::Int4(12345)]));
        assert_eq!(
            apply_typmod(ints.clone(), TypeOid::Int4Array, len3, false).unwrap(),
            ints
        );
    }

    #[test]
    fn numeric_typmod_is_context_insensitive() {
        // The numeric arm ignores `is_explicit` — round to scale + range-check the
        // same for a column store and an explicit cast (overflow → 22003).
        let tm = Typmod::Numeric {
            precision: 4,
            scale: 1,
        };
        let rounded = apply_typmod(
            Value::Numeric(PgNumeric::parse("12.345").unwrap()),
            TypeOid::Numeric,
            tm,
            true,
        )
        .unwrap();
        assert_eq!(rounded, Value::Numeric(PgNumeric::parse("12.3").unwrap()));
        for explicit in [false, true] {
            let e = apply_typmod(
                Value::Numeric(PgNumeric::parse("12345.6").unwrap()),
                TypeOid::Numeric,
                tm,
                explicit,
            )
            .expect_err("numeric overflow must fail loud");
            assert_eq!(e.sqlstate(), "22003");
        }
    }

    /// Assert a `float4::numeric` cast renders to the given decimal text.
    fn assert_f4_num(n: f32, expected: &str) {
        let v = coerce_value(Value::Float4(n), TypeOid::Numeric).expect("float4→numeric");
        let Value::Numeric(got) = v else {
            panic!("expected numeric, got {v:?}");
        };
        assert_eq!(got.to_display_string(), expected, "float4 {n} → numeric");
    }

    #[test]
    // `3.14` is a two-decimal `%.6g` probe, not an approximation of pi.
    #[allow(clippy::approx_constant)]
    fn float4_to_numeric_uses_own_precision() {
        // Postgres `float4_numeric` renders 6 significant digits (`%.6g`), so the
        // numeric reflects the float4's precision, not the f64-widened noise a
        // `f64::from(n)` capture gives (`0.1::float4::numeric` = `0.1`, not
        // `0.10000000149…`).
        assert_f4_num(0.1, "0.1");
        assert_f4_num(3.14, "3.14");
        assert_f4_num(1.5, "1.5");
        assert_f4_num(0.3, "0.3");
        assert_f4_num(1e20, "100000000000000000000");
        assert_f4_num(-0.0, "0"); // negative zero normalizes to zero, as Postgres
        // Trailing zeros dropped (dscale matches Postgres's `%g`): 6-sig rounding.
        assert_f4_num(123_456.7, "123457");
        assert_f4_num(8_388_609.0, "8388610");
        assert_f4_num(2.000_000_2, "2");
    }

    /// Assert a `float8::numeric` cast renders to the given decimal text.
    fn assert_f8_num(n: f64, expected: &str) {
        let v = coerce_value(Value::Float8(n), TypeOid::Numeric).expect("float8→numeric");
        let Value::Numeric(got) = v else {
            panic!("expected numeric, got {v:?}");
        };
        assert_eq!(got.to_display_string(), expected, "float8 {n} → numeric");
    }

    #[test]
    fn float8_to_numeric_uses_15_sig_figs() {
        // Postgres `float8_numeric` renders 15 significant digits (`%.15g`, DBL_DIG), so
        // the numeric reflects the double's precision — not the 16-17 digits its shortest
        // round-trip carries. Verified against live PG 18.6.
        assert_f8_num(0.1, "0.1");
        assert_f8_num(1.5, "1.5");
        assert_f8_num(1.0 / 3.0, "0.333333333333333"); // 15 threes, PG: not 16
        assert_f8_num(2.0 / 3.0, "0.666666666666667");
        assert_f8_num(1.234_567_890_123_456_7, "1.23456789012346"); // 15 sig, rounded
        assert_f8_num(123_456_789_012_345.67, "123456789012346");
        assert_f8_num(-0.0, "0"); // negative zero normalizes, as Postgres
    }

    #[test]
    fn float8_to_numeric_special_values() {
        // NaN → numeric NaN; ±Infinity has no PgNumeric form → loud 22003.
        let v = coerce_value(Value::Float8(f64::NAN), TypeOid::Numeric).expect("NaN→numeric");
        assert!(matches!(v, Value::Numeric(PgNumeric::NaN)));
        for f in [f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                coerce_value(Value::Float8(f), TypeOid::Numeric)
                    .expect_err("inf→numeric must fail loud")
                    .sqlstate(),
                "22003"
            );
        }
    }

    #[test]
    fn float4_to_numeric_special_values() {
        // NaN → numeric NaN (Postgres `make_result(&const_nan)`).
        let v = coerce_value(Value::Float4(f32::NAN), TypeOid::Numeric).expect("NaN→numeric");
        assert!(matches!(v, Value::Numeric(PgNumeric::NaN)));
        // ±Infinity has no `PgNumeric` form → loud 22003 (pglet-stricter than
        // Postgres 14+, which has numeric ±Infinity).
        for f in [f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                coerce_value(Value::Float4(f), TypeOid::Numeric)
                    .expect_err("inf→numeric must fail loud")
                    .sqlstate(),
                "22003"
            );
        }
    }

    #[test]
    fn numeric_to_float4_single_rounded_and_overflow_loud() {
        // A numeric beyond real's finite range → loud 22003 (Postgres `float4in`
        // overflow), mirroring the float8→float4 arm — not a silent ±inf.
        assert_eq!(
            coerce_value(
                Value::Numeric(PgNumeric::parse("1e300").unwrap()),
                TypeOid::Float4
            )
            .expect_err("1e300→real must fail loud")
            .sqlstate(),
            "22003"
        );
        // Ordinary values round-trip.
        assert_eq!(
            coerce_value(
                Value::Numeric(PgNumeric::parse("1.5").unwrap()),
                TypeOid::Float4
            )
            .unwrap(),
            Value::Float4(1.5)
        );
        // Single correctly-rounded decimal→real: this tie-boundary literal is
        // `1.0000001` in Postgres; a numeric→f64→f32 double-rounding would give
        // `1.0000002`.
        let dbl = "1.00000017881393421514957253748434595763683319091796875";
        assert_eq!(
            coerce_value(
                Value::Numeric(PgNumeric::parse(dbl).unwrap()),
                TypeOid::Float4
            )
            .unwrap(),
            Value::Float4(1.000_000_1)
        );
        // Numeric NaN → f32 NaN (no error).
        let v = coerce_value(Value::Numeric(PgNumeric::NaN), TypeOid::Float4).unwrap();
        assert!(matches!(v, Value::Float4(f) if f.is_nan()));
    }

    #[test]
    fn float8_to_float4_overflow_is_loud() {
        // A finite double beyond `real`'s range overflows → loud (Postgres errors).
        assert_eq!(
            coerce_value(Value::Float8(1e40), TypeOid::Float4)
                .expect_err("1e40→real")
                .sqlstate(),
            "22003"
        );
        // In-range narrows (precision loss is allowed, not an error).
        assert_eq!(
            coerce_value(Value::Float8(1.5), TypeOid::Float4).unwrap(),
            Value::Float4(1.5)
        );
    }

    /// The vertical tab, one member of `boolin`'s C `isspace` trim set. `E'\v'`,
    /// `E'\x0B'`, `E'\013'` and `E'\u000B'` each decode to U+000B on both engines, so
    /// the trim set's `\v` clause is differential in any of them and
    /// `bool_context/bool_input_trims_ascii_whitespace.sql` carries it. What this adds
    /// is the direct call on `text_to_bool`, and a mixed-padding spelling beside it.
    #[test]
    fn bool_input_trims_the_vertical_tab() {
        assert!(text_to_bool("\u{b}t\u{b}").unwrap());
        assert!(!text_to_bool("\u{b}\u{c}false\t\r\n ").unwrap());
    }

    /// And the code points it does **not** trim: `str::trim` strips every Unicode
    /// `White_Space`, which accepted values 18.6 refuses. The no-break space half is a
    /// fixture (`bool_context/bool_input_trims_only_c_isspace.sql`); the rest are here,
    /// because a fixture for each would be one statement per code point.
    #[test]
    fn bool_input_trims_no_unicode_whitespace() {
        for pad in ["\u{a0}", "\u{2000}", "\u{3000}", "\u{2028}", "\u{feff}"] {
            let input = format!("{pad}t");
            assert_eq!(
                text_to_bool(&input)
                    .expect_err("unicode padding")
                    .sqlstate(),
                "22P02",
                "{pad:?} must not be trimmed"
            );
        }
    }

    /// The prefix grammar at its boundaries, which the fixtures cover by spelling but not
    /// exhaustively: every non-empty prefix of each word is accepted, and the word plus
    /// one character is not.
    #[test]
    fn bool_input_accepts_every_prefix_and_refuses_one_character_more() {
        for (word, expected) in [
            ("true", true),
            ("false", false),
            ("yes", true),
            ("no", false),
        ] {
            for len in 1..=word.len() {
                assert_eq!(
                    text_to_bool(&word[..len]).unwrap_or_else(|e| panic!("{word}[..{len}]: {e}")),
                    expected,
                    "{word}[..{len}]"
                );
            }
            let too_long = format!("{word}x");
            assert_eq!(
                text_to_bool(&too_long)
                    .expect_err("one character past the word")
                    .sqlstate(),
                "22P02"
            );
        }
        // `on`/`off` share a first character, so 18.6 compares at least two — which is
        // what refuses a bare `o` and accepts `of`.
        assert_eq!(
            text_to_bool("o").expect_err("ambiguous").sqlstate(),
            "22P02"
        );
        assert!(text_to_bool("on").unwrap());
        assert!(!text_to_bool("of").unwrap());
        assert!(!text_to_bool("off").unwrap());
        assert_eq!(
            text_to_bool("offx").expect_err("past off").sqlstate(),
            "22P02"
        );
        // `1`/`0` are length-exact rather than prefixes.
        assert!(text_to_bool("1").unwrap());
        assert!(!text_to_bool("0").unwrap());
        for s in ["01", "10", "00", "11"] {
            assert_eq!(text_to_bool(s).expect_err("two digits").sqlstate(), "22P02");
        }
    }

    /// The message names the **untrimmed** input, as `boolin`'s `errmsg` does — it reports
    /// `in_str`, not the pointer it advanced past the leading whitespace.
    #[test]
    fn bool_input_error_names_the_untrimmed_input() {
        let err = text_to_bool("  zz  ").expect_err("not a boolean");
        assert!(
            err.to_string().contains("\"  zz  \""),
            "message must carry the untrimmed value: {err}"
        );
    }
}
