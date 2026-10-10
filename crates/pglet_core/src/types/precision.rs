//! Sub-second precision — the `(p)` the four temporal types carry.
//!
//! `timestamp(p)`, `timestamptz(p)`, `time(p)` and `interval(p)` all round the
//! **same** microsecond count the same way, so the rule lives here once and the
//! three carriers call it: [`round_micros_to_precision`] is 18.6's
//! `AdjustTimestampForTypmod` (`timestamp.c`), `AdjustTimeForTypmod` (`date.c`) and
//! `AdjustIntervalForTypmod` (`timestamp.c`)'s sub-second half, which are three
//! copies of one arithmetic in the C source.
//!
//! **Half away from zero, about the value's own zero — not the wall clock.** Each
//! adds half a scale and truncates toward zero, and the sign it branches on is the
//! *internal* representation's: microseconds since **2000-01-01** for a timestamp,
//! since midnight for a `time` (never negative), and the interval's own `micros`
//! field. That is why `'1999-12-31 23:59:59.9995'::timestamp(3)` is
//! `1999-12-31 23:59:59.999` on 18.6 where a naive half-up on the *fraction* gives
//! `2000-01-01 00:00:00` — the value is negative there, so the half goes the other
//! way (measured on both engines).
//!
//! `p = 6` is a no-op ([`MAX_TEMPORAL_PRECISION`], 18.6's `MAX_TIMESTAMP_PRECISION`),
//! and so is this — but that is one observable over **three different mechanisms** in
//! the C source, and only one of them is an early return. `AdjustTimestampForTypmod`
//! skips the arm outright (`(typmod != -1) && (typmod != MAX_TIMESTAMP_PRECISION)`).
//! `AdjustTimeForTypmod` runs it (`typmod >= 0 && typmod <= MAX_TIME_PRECISION`) with
//! `TimeOffsets[6] = 0` and `TimeScales[6] = 1`, an identity. `AdjustIntervalForTypmod`
//! skips its sub-second half only at `INTERVAL_FULL_PRECISION` (`0xFFFF`, the *absent*
//! precision), so a declared `interval(6)` runs the identity there too.

use crate::error::{Error, Result};

use super::{Infinite, PgInterval, PgTimestamp};

/// The largest sub-second precision the temporal types carry.
///
/// 18.6's `MAX_TIMESTAMP_PRECISION` / `MAX_TIME_PRECISION` / `MAX_INTERVAL_PRECISION`,
/// all `6`. A declared `p` above it is **clamped** to this (18.6 warns and records
/// `6`), never refused, and rounding at it is a no-op.
pub const MAX_TEMPORAL_PRECISION: u8 = 6;

/// `TimestampScales` / `TimestampOffsets` from `timestamp.c`, indexed by precision.
const SCALES: [i64; 7] = [1_000_000, 100_000, 10_000, 1_000, 100, 10, 1];
const OFFSETS: [i64; 7] = [500_000, 50_000, 5_000, 500, 50, 5, 0];

/// Round a microsecond count to `precision` fractional digits, half **away from
/// zero** — the one arithmetic all three of 18.6's `Adjust*ForTypmod` functions run.
///
/// `precision` at or above [`MAX_TEMPORAL_PRECISION`] is the identity — the same answer
/// 18.6 reaches at `6` by three different routes (see the module doc), and here also the
/// bound that keeps the table index in range: at exactly `6` the tables would give the
/// same answer (offset `0`, scale `1`), but above it there is no entry, so the guard is
/// load-bearing rather than decorative.
/// `resolve_column_type` clamps before this is reached, so a caller cannot normally
/// pass one; the guard is the floor under a direct call.
///
/// Returns `None` when taking the magnitude, or adding the half-scale to it, overflows
/// `i64`. 18.6's `AdjustIntervalForTypmod` checks the same arithmetic with
/// `pg_add_s64_overflow` / `pg_sub_s64_overflow` and raises
/// `22008 interval out of range`, and the two agree save at one negative count per
/// precision below `6`: `i64::MIN` plus the half-scale, which 18.6's subtraction reaches
/// without overflow and rounds, and whose magnitude here overflows (measured at `(0)`
/// and `(5)`). `AdjustTimestampForTypmod` does not check it,
/// because a `Timestamp` in range is nowhere near `i64::MAX`; the caller here gets
/// the same answer either way and a loud one instead of a wrap.
#[must_use]
pub fn round_micros_to_precision(micros: i64, precision: u8) -> Option<i64> {
    if precision >= MAX_TEMPORAL_PRECISION {
        return Some(micros);
    }
    let scale = SCALES[usize::from(precision)];
    let offset = OFFSETS[usize::from(precision)];
    // The C source branches on the sign and negates around the magnitude, which is
    // what makes the half go *away from zero* rather than toward positive infinity.
    let shifted = micros.checked_abs()?.checked_add(offset)?;
    let rounded = (shifted / scale) * scale;
    Some(if micros < 0 { -rounded } else { rounded })
}

/// Round a `timestamp` / `timestamptz` instant to `precision` fractional digits; an
/// infinity is left as it is (`AdjustTimestampForTypmod` skips a non-finite value).
///
/// The sign is taken from the **Postgres epoch** count ([`PgTimestamp::to_pg_micros`]),
/// not the Unix one: 18.6's `Timestamp` is microseconds since 2000-01-01, so `1969-07-20 20:17:40.0005` and
/// `1999-12-31 23:59:59.9995` are both *negative* there and round away from zero
/// downward (`…40` and `…59.999`), while `2000-01-01 00:00:00.0005` is positive and
/// rounds up to `.001`. All three measured on 18.6.
///
/// # Errors
///
/// [`Error::TemporalOutOfRange`] (`22008`, `timestamp out of range`) when the
/// rounded instant leaves the representable range — `'9999-12-31 23:59:59.9999'`
/// under `(3)` carries into year 10000, which 18.6 represents and this engine's
/// `OffsetDateTime` does not. Never silently keeps the unrounded value.
pub fn round_timestamp_to_precision(ts: PgTimestamp, precision: u8) -> Result<PgTimestamp> {
    if precision >= MAX_TEMPORAL_PRECISION || !matches!(ts, Infinite::Finite(_)) {
        return Ok(ts);
    }
    let oor = || Error::TemporalOutOfRange("timestamp");
    round_micros_to_precision(ts.to_pg_micros(), precision)
        .and_then(PgTimestamp::from_pg_micros)
        .ok_or_else(oor)?
        .try_map(super::check_timestamp_domain)
}

/// Round a `time` (microseconds since midnight) to `precision` fractional digits.
///
/// A `TimeADT` is never negative, so the sign split is unreachable — and the carry
/// is **not** wrapped: `'23:59:59.9999'::time(3)` is `24:00:00` on 18.6, which
/// `Value::Time`'s raw `i64` represents and the text encoder already prints.
///
/// # Errors
///
/// [`Error::TemporalOutOfRange`] (`22008`) if the half-scale add overflows `i64` —
/// unreachable from a parsed `time`, and a refusal rather than a wrap for a value
/// arriving over the binary protocol.
pub fn round_time_to_precision(micros: i64, precision: u8) -> Result<i64> {
    round_micros_to_precision(micros, precision).ok_or(Error::TemporalOutOfRange("time"))
}

/// Round an `interval`'s **sub-second** field to `precision` fractional digits.
///
/// Months and days are untouched: only a *range* spec (`INTERVAL DAY TO SECOND`,
/// which pglet has no grammar for) zeroes higher-order fields in
/// `AdjustIntervalForTypmod`, and a bare `interval(p)` is `INTERVAL_FULL_RANGE`,
/// whose arm does nothing before the sub-second half runs.
///
/// 18.6 returns before rounding an interval at either field triple it reads as
/// `infinity` / `-infinity`. `PgInterval` has no infinity (`EDGES.md` row 143), so a
/// triple takes the arithmetic any value does: at `p = 6` it is returned unchanged, a
/// finite value, and below 6 its microseconds field overflows `i64`, so it is refused
/// where 18.6 answers `infinity` / `-infinity`.
///
/// # Errors
///
/// [`Error::TemporalOutOfRange`] (`22008`, `interval out of range`) when
/// [`round_micros_to_precision`] overflows, which is where 18.6 raises the same code,
/// short of the one negative count per precision that function names and of the two field
/// triples, which 18.6 answers as `infinity` / `-infinity` (above).
pub fn round_interval_to_precision(iv: PgInterval, precision: u8) -> Result<PgInterval> {
    let micros = round_micros_to_precision(iv.micros, precision)
        .ok_or(Error::TemporalOutOfRange("interval"))?;
    Ok(PgInterval {
        months: iv.months,
        days: iv.days,
        micros,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TEMPORAL_PRECISION, round_interval_to_precision, round_micros_to_precision,
        round_time_to_precision, round_timestamp_to_precision,
    };
    use crate::types::PgInterval;
    use time::OffsetDateTime;
    use time::macros::datetime;

    /// Ties go **away from zero**, not to even — and the negative side is a
    /// different arm, not the mirror of a symmetric rule. Each pair measured on
    /// 18.6 through `time(3)` / `interval(3)`.
    #[test]
    fn a_tie_rounds_away_from_zero_on_both_sides() {
        let r = |us| round_micros_to_precision(us, 3);
        assert_eq!(r(500), Some(1_000)); // .0005 -> .001
        assert_eq!(r(1_500), Some(2_000)); // .0015 -> .002
        assert_eq!(r(2_500), Some(3_000)); // .0025 -> .003 (half-to-even gives .002)
        assert_eq!(r(1_400), Some(1_000));
        assert_eq!(r(-500), Some(-1_000));
        assert_eq!(r(-1_500), Some(-2_000));
        assert_eq!(r(-2_500), Some(-3_000));
        assert_eq!(r(-1_400), Some(-1_000));
        assert_eq!(r(-400), Some(0));
    }

    /// `p = 6` returns the value untouched, which is why a `timestamp(6)` column is
    /// a *captured* modifier whose rounding is a no-op rather than an absent one.
    #[test]
    fn the_maximum_precision_is_the_identity() {
        assert_eq!(
            round_micros_to_precision(123_456, MAX_TEMPORAL_PRECISION),
            Some(123_456)
        );
        assert_eq!(
            round_micros_to_precision(-123_456, MAX_TEMPORAL_PRECISION),
            Some(-123_456)
        );
        assert_eq!(round_micros_to_precision(1, 0), Some(0));
    }

    /// The epoch trap: the sign is the **2000-01-01** count's, so an instant before
    /// that epoch rounds a tie downward in wall-clock terms. Each line is 18.6's
    /// answer, measured.
    #[test]
    fn the_rounding_sign_is_the_postgres_epochs_not_the_wall_clocks() {
        let round = |dt: OffsetDateTime| round_timestamp_to_precision(dt.into(), 3).unwrap();
        // `1999-12-31 23:59:59.9995` is negative in Postgres-epoch micros, so the
        // half goes *down*: 18.6 answers `1999-12-31 23:59:59.999`.
        assert_eq!(
            round(datetime!(1999-12-31 23:59:59.9995 UTC)),
            datetime!(1999-12-31 23:59:59.999 UTC).into()
        );
        // Positive by one half-milli: 18.6 answers `.001`.
        assert_eq!(
            round(datetime!(2000-01-01 00:00:00.0005 UTC)),
            datetime!(2000-01-01 00:00:00.001 UTC).into()
        );
        // Also negative: 18.6 answers `1969-07-20 20:17:40`, not `.001`.
        assert_eq!(
            round(datetime!(1969-07-20 20:17:40.0005 UTC)),
            datetime!(1969-07-20 20:17:40.000 UTC).into()
        );
        // The positive control on the same shape, well after the epoch.
        assert_eq!(
            round(datetime!(2020-01-02 03:04:05.6789 UTC)),
            datetime!(2020-01-02 03:04:05.679 UTC).into()
        );
    }

    /// A carry out of the day is **not** wrapped — 18.6 answers `24:00:00`.
    #[test]
    fn a_time_carry_reaches_the_day_endpoint() {
        // 23:59:59.9999 in micros since midnight.
        let us = ((23 * 3600 + 59 * 60 + 59) * 1_000_000) + 999_900;
        assert_eq!(
            round_time_to_precision(us, 3).unwrap(),
            24 * 3600 * 1_000_000
        );
    }

    /// Only the micros field moves; a precision is not a range spec.
    #[test]
    fn an_intervals_months_and_days_are_untouched() {
        let iv = PgInterval {
            months: 14,
            days: 3,
            micros: 1_234_560,
        };
        let out = round_interval_to_precision(iv, 3).unwrap();
        assert_eq!((out.months, out.days, out.micros), (14, 3, 1_235_000));
    }

    /// A rounding carry out of the representable range is loud (`22008`), never a
    /// silently-kept unrounded value. 18.6 answers `10000-01-01 00:00:00` here.
    #[test]
    fn a_rounding_overflow_is_loud() {
        let err = round_timestamp_to_precision(datetime!(9999-12-31 23:59:59.9999 UTC).into(), 3)
            .unwrap_err();
        assert_eq!(err.sqlstate(), "22008");
        assert_eq!(err.to_string(), "timestamp out of range");
    }

    /// The domain's lower edge holds across a rounding: `0001-01-01 00:00:00` is a whole
    /// second, so nothing at or above it rounds below it, and a source already below it
    /// — a value a store written without the domain holds — is refused, not rounded.
    #[test]
    fn a_rounding_holds_the_domain_lower_edge() {
        assert_eq!(
            round_timestamp_to_precision(datetime!(0001-01-01 00:00:00.0004 UTC).into(), 3)
                .unwrap(),
            datetime!(0001-01-01 00:00:00 UTC).into()
        );
        let err = round_timestamp_to_precision(datetime!(0000-12-31 12:00:00.4 UTC).into(), 0)
            .unwrap_err();
        assert_eq!(err.to_string(), "timestamp out of range");
    }

    /// The overflow guard on the half-scale add, at the one carrier whose field can
    /// reach `i64::MAX` — 18.6's `pg_add_s64_overflow` arm.
    #[test]
    fn an_interval_micros_overflow_is_loud() {
        let iv = PgInterval {
            months: 0,
            days: 0,
            micros: i64::MAX,
        };
        assert_eq!(
            round_interval_to_precision(iv, 3).unwrap_err().sqlstate(),
            "22008"
        );
    }
}
