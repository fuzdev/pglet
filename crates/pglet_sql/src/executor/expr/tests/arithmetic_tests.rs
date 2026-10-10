//! Tests for arithmetic operator evaluation.

use super::super::binary_op::eval_binary_op;
use crate::planner::BinaryOp;
use crate::types::{PgInterval, Value};
use time::macros::datetime;

/// A pure-µs interval value, the shape the older tests used before the
/// 3-field refactor (`time - time`, `ts - ts` still produce this).
fn iv(micros: i64) -> Value {
    Value::Interval(PgInterval::from_micros(micros))
}

#[test]
fn integer_math_stays_exact_above_2_pow_53() {
    // 2^53 + 1 is not representable in f64; the old f64 path returned
    // 9007199254740992. Integer-domain math is exact.
    let a = Value::Int8(9_007_199_254_740_993);
    let r = eval_binary_op(BinaryOp::Add, &a, &Value::Int8(0)).unwrap();
    assert_eq!(r, Value::Int8(9_007_199_254_740_993));
    // Same magnitude as a mixed-width op (Int8 + Int4 → Int8).
    let r = eval_binary_op(BinaryOp::Add, &a, &Value::Int4(2)).unwrap();
    assert_eq!(r, Value::Int8(9_007_199_254_740_995));
}

#[test]
fn integer_overflow_is_loud_not_saturating() {
    // i32::MAX + 1 → "integer out of range" (22003), not a saturated i32::MAX.
    let e = eval_binary_op(BinaryOp::Add, &Value::Int4(i32::MAX), &Value::Int4(1))
        .expect_err("overflow must error");
    assert_eq!(e.sqlstate(), "22003");
    // i64::MAX + 1 → "bigint out of range".
    let e = eval_binary_op(BinaryOp::Add, &Value::Int8(i64::MAX), &Value::Int8(1))
        .expect_err("overflow must error");
    assert_eq!(e.sqlstate(), "22003");
    // i16::MIN - 1 → "smallint out of range".
    let e = eval_binary_op(BinaryOp::Sub, &Value::Int2(i16::MIN), &Value::Int2(1))
        .expect_err("overflow must error");
    assert_eq!(e.sqlstate(), "22003");
}

#[test]
fn division_by_zero_is_loud_for_int_and_float() {
    // Old code: f64::NAN as i64 == 0, silently returning 0.
    let e = eval_binary_op(BinaryOp::Div, &Value::Int4(1), &Value::Int4(0))
        .expect_err("int /0 must error");
    assert_eq!(e.sqlstate(), "22012");
    let e = eval_binary_op(BinaryOp::Div, &Value::Float8(1.0), &Value::Float8(0.0))
        .expect_err("float /0 must error");
    assert_eq!(e.sqlstate(), "22012");
}

#[test]
fn division_min_by_neg_one_overflows() {
    // The one case where integer *division* overflows — `T::MIN / -1` exceeds
    // `T::MAX` — and the reason `checked_div` is used over `/` (which would
    // panic in debug, wrap in release). Postgres errors "out of range" (22003).
    let e = eval_binary_op(BinaryOp::Div, &Value::Int8(i64::MIN), &Value::Int8(-1))
        .expect_err("i64::MIN / -1 must overflow");
    assert_eq!(e.sqlstate(), "22003");
    let e = eval_binary_op(BinaryOp::Div, &Value::Int4(i32::MIN), &Value::Int4(-1))
        .expect_err("i32::MIN / -1 must overflow");
    assert_eq!(e.sqlstate(), "22003");
    // Also via a mixed-width pair (int8 MIN / int4 -1 → bigint domain).
    let e = eval_binary_op(BinaryOp::Div, &Value::Int8(i64::MIN), &Value::Int4(-1))
        .expect_err("mixed-width MIN / -1 must overflow");
    assert_eq!(e.sqlstate(), "22003");
}

#[test]
fn non_numeric_operand_errors_not_silent_zero() {
    // `'abc' + 1` used to yield Float8(1.0) via the 0.0 fallback; now errors.
    assert!(eval_binary_op(BinaryOp::Add, &Value::Text("abc".into()), &Value::Int4(1)).is_err());
    assert!(eval_binary_op(BinaryOp::Mul, &Value::Bool(true), &Value::Int4(2)).is_err());
}

#[test]
fn null_operand_propagates() {
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &Value::Null, &Value::Int4(1)).unwrap(),
        Value::Null
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Div, &Value::Int4(1), &Value::Null).unwrap(),
        Value::Null
    );
}

#[test]
fn mixed_width_subtraction_preserves_operand_order() {
    // Guards the operand-order trap in the old swapped mixed-width arms:
    // 10 - 3 must be 7, not 3 - 10.
    let r = eval_binary_op(BinaryOp::Sub, &Value::Int8(10), &Value::Int4(3)).unwrap();
    assert_eq!(r, Value::Int8(7));
    let r = eval_binary_op(BinaryOp::Sub, &Value::Int4(3), &Value::Int8(10)).unwrap();
    assert_eq!(r, Value::Int8(-7));
}

#[test]
fn mixed_width_promotes_to_the_wider_operand_type() {
    // Postgres: int2 + int4 -> int4; int4 + int8 -> int8 (the wider type),
    // not unconditionally int8. Operand order is preserved either way.
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &Value::Int2(2), &Value::Int4(3)).unwrap(),
        Value::Int4(5)
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &Value::Int4(3), &Value::Int2(2)).unwrap(),
        Value::Int4(5)
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &Value::Int4(3), &Value::Int8(10)).unwrap(),
        Value::Int8(13)
    );
}

#[test]
fn mixed_width_int4_result_overflow_is_loud() {
    // int4 + int2 -> int4: i32::MAX + 1 must error "integer out of range"
    // (22003), not silently widen to int8.
    let e = eval_binary_op(BinaryOp::Add, &Value::Int4(i32::MAX), &Value::Int2(1))
        .expect_err("int4 result overflow must error");
    assert_eq!(e.sqlstate(), "22003");
}

#[test]
fn mixed_width_covers_remaining_combos_and_operators() {
    // int2 + int8 -> int8 (the combo the other tests skip).
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &Value::Int2(2), &Value::Int8(40)).unwrap(),
        Value::Int8(42)
    );
    // int2 * int4 -> int4 (the int4-narrowing path under a non-Add operator).
    assert_eq!(
        eval_binary_op(BinaryOp::Mul, &Value::Int2(6), &Value::Int4(7)).unwrap(),
        Value::Int4(42)
    );
    // int4 / int2 -> int4 (integer division, operand order preserved: 17/5 = 3).
    assert_eq!(
        eval_binary_op(BinaryOp::Div, &Value::Int4(17), &Value::Int2(5)).unwrap(),
        Value::Int4(3)
    );
    // Mixed-width division by zero is still loud (22012).
    let e = eval_binary_op(BinaryOp::Div, &Value::Int2(1), &Value::Int4(0))
        .expect_err("mixed-width /0 must error");
    assert_eq!(e.sqlstate(), "22012");
}

#[test]
fn timestamp_plus_interval_shifts_by_microseconds() {
    // `ts + INTERVAL` advances the timestamp by the interval's microseconds;
    // `interval + ts` is commutative; `ts - INTERVAL` rewinds it. pglet's
    // interval is pure microseconds, so an hour shift is exactly 3.6e9 µs.
    let base = Value::timestamp(datetime!(2020-01-01 12:00:00 UTC));
    let one_hour = iv(3_600_000_000);
    let plus = Value::timestamp(datetime!(2020-01-01 13:00:00 UTC));
    let minus = Value::timestamp(datetime!(2020-01-01 11:00:00 UTC));
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &base, &one_hour).unwrap(),
        plus
    );
    // `interval + timestamp` (operand order swapped) yields the same result.
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &one_hour, &base).unwrap(),
        plus
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Sub, &base, &one_hour).unwrap(),
        minus
    );
}

#[test]
fn timestamp_interval_null_propagates() {
    // A NULL operand on either side yields NULL, not a shifted timestamp.
    let base = Value::timestamp(datetime!(2020-01-01 12:00:00 UTC));
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &base, &Value::Null).unwrap(),
        Value::Null
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Sub, &Value::Null, &iv(1)).unwrap(),
        Value::Null
    );
}

// `1 * 3_600_000_000` reads as one hour beside its `23 *` / `22 *` / `2 *` siblings;
// collapsing it to the bare constant would make the table inconsistent.
#[allow(clippy::identity_op)]
#[test]
fn time_arithmetic_diff_and_interval_wrap() {
    // `time - time → interval` (µs duration, may be negative).
    let t_23 = Value::Time(23 * 3_600_000_000);
    let t_01 = Value::Time(1 * 3_600_000_000);
    assert_eq!(
        eval_binary_op(BinaryOp::Sub, &t_23, &t_01).unwrap(),
        iv(22 * 3_600_000_000)
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Sub, &t_01, &t_23).unwrap(),
        iv(-22 * 3_600_000_000)
    );
    // `'23:00' + INTERVAL '2 hours'` wraps to `01:00`.
    let two_hours = iv(2 * 3_600_000_000);
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &t_23, &two_hours).unwrap(),
        Value::Time(1 * 3_600_000_000)
    );
    // `interval + time` is commutative.
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &two_hours, &t_23).unwrap(),
        Value::Time(1 * 3_600_000_000)
    );
    // Days are ignored: `'01:00' + INTERVAL '3 days'` stays `01:00`.
    let three_days = iv(3 * 86_400_000_000);
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &t_01, &three_days).unwrap(),
        t_01.clone()
    );
    // `'01:00' - INTERVAL '2 hours'` wraps back to `23:00`.
    assert_eq!(
        eval_binary_op(BinaryOp::Sub, &t_01, &two_hours).unwrap(),
        t_23
    );
}

#[test]
fn date_plus_time_is_timestamp() {
    use time::macros::date;
    // `date + time → timestamp` at UTC midnight of the date plus the time-of-day.
    let d = Value::date(date!(2024 - 06 - 15));
    let t = Value::Time(13 * 3_600_000_000 + 30 * 60_000_000); // 13:30:00
    let expected = Value::timestamp(datetime!(2024-06-15 13:30:00 UTC));
    assert_eq!(eval_binary_op(BinaryOp::Add, &d, &t).unwrap(), expected);
    // Commutative: `time + date`.
    assert_eq!(eval_binary_op(BinaryOp::Add, &t, &d).unwrap(), expected);
}

#[test]
fn interval_minus_timestamp_is_loud() {
    // `interval - timestamp` is undefined in Postgres; it must fail loud
    // rather than silently producing a bogus value.
    let base = Value::timestamp(datetime!(2020-01-01 12:00:00 UTC));
    assert!(
        eval_binary_op(BinaryOp::Sub, &iv(1), &base).is_err(),
        "interval - timestamp must error"
    );
}

#[test]
fn interval_arith_overflow_and_div_by_zero_are_loud() {
    // `interval + interval` overflowing the month field fails loud (`22008`,
    // interval out of range), not wraps.
    let big = Value::Interval(PgInterval::new(i32::MAX, 0, 0));
    let e = eval_binary_op(BinaryOp::Add, &big, &big).expect_err("month overflow must error");
    assert_eq!(e.sqlstate(), "22008");
    // Same for the days field on subtraction of a negative extreme.
    let neg_days = Value::Interval(PgInterval::new(0, i32::MIN, 0));
    let pos_days = Value::Interval(PgInterval::new(0, i32::MAX, 0));
    let e =
        eval_binary_op(BinaryOp::Sub, &pos_days, &neg_days).expect_err("day overflow must error");
    assert_eq!(e.sqlstate(), "22008");
    // `interval / 0` is division by zero (`22012`).
    let e = eval_binary_op(BinaryOp::Div, &iv(3_600_000_000), &Value::Int4(0))
        .expect_err("interval /0 must error");
    assert_eq!(e.sqlstate(), "22012");
}

/// Every temporal arithmetic producer holds the date and timestamp domain at both
/// edges: the result on the edge is kept, one day or one microsecond past it is
/// `22008` with the producer's sentence. Where 18.6 answers a `BC` or year-10000
/// value here, pglet refuses — no differential fixture can pin that code, so this
/// test does.
#[test]
fn temporal_arithmetic_holds_the_domain_at_both_edges() {
    use time::macros::date;
    let refused = |op: BinaryOp, l: &Value, r: &Value, sentence: &str| {
        let e = eval_binary_op(op, l, r).expect_err("out of domain");
        assert_eq!(
            (e.sqlstate(), e.to_string().as_str()),
            ("22008", sentence),
            "{l:?} {op:?} {r:?}"
        );
    };
    let d = |x: time::Date| Value::date(x);
    let ts = |x: time::OffsetDateTime| Value::timestamp(x);
    let span = |months, days, micros| Value::Interval(PgInterval::new(months, days, micros));
    let first = date!(0001 - 01 - 01);
    let last = date!(9999 - 12 - 31);

    // `date ± int` — both argument orders of `+`, both widths.
    assert_eq!(
        eval_binary_op(BinaryOp::Sub, &d(date!(0001 - 01 - 02)), &Value::Int4(1)).unwrap(),
        d(first)
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &Value::Int2(1), &d(date!(9999 - 12 - 30))).unwrap(),
        d(last)
    );
    refused(
        BinaryOp::Sub,
        &d(first),
        &Value::Int4(1),
        "date out of range",
    );
    refused(
        BinaryOp::Add,
        &d(first),
        &Value::Int2(-1),
        "date out of range",
    );
    refused(
        BinaryOp::Add,
        &d(last),
        &Value::Int4(1),
        "date out of range",
    );
    refused(
        BinaryOp::Add,
        &Value::Int4(1),
        &d(last),
        "date out of range",
    );
    refused(
        BinaryOp::Sub,
        &d(first),
        &Value::Int4(i32::MAX),
        "date out of range",
    );

    // `timestamp ± interval`, one microsecond each side, through each pass.
    assert_eq!(
        eval_binary_op(
            BinaryOp::Sub,
            &ts(datetime!(0001-01-01 00:00:00.000001 UTC)),
            &span(0, 0, 1)
        )
        .unwrap(),
        ts(datetime!(0001-01-01 00:00:00 UTC))
    );
    assert_eq!(
        eval_binary_op(
            BinaryOp::Add,
            &span(0, 0, 1),
            &ts(datetime!(9999-12-31 23:59:59.999998 UTC))
        )
        .unwrap(),
        ts(datetime!(9999-12-31 23:59:59.999999 UTC))
    );
    let floor = ts(datetime!(0001-01-01 00:00:00 UTC));
    let ceiling = ts(datetime!(9999-12-31 23:59:59.999999 UTC));
    for step in [span(0, 0, 1), span(0, 1, 0), span(1, 0, 0)] {
        refused(BinaryOp::Sub, &floor, &step, "timestamp out of range");
        refused(BinaryOp::Add, &ceiling, &step, "timestamp out of range");
        refused(BinaryOp::Add, &step, &ceiling, "timestamp out of range");
    }
    // A pass may leave the domain on the way and come back: year 0 in between.
    assert_eq!(
        eval_binary_op(
            BinaryOp::Add,
            &ts(datetime!(0001-01-15 00:00:00 UTC)),
            &span(-1, 30, 0)
        )
        .unwrap(),
        ts(datetime!(0001-01-14 00:00:00 UTC))
    );

    // `date ± interval` → timestamp.
    refused(
        BinaryOp::Sub,
        &d(first),
        &span(0, 0, 1),
        "timestamp out of range",
    );
    refused(
        BinaryOp::Add,
        &span(0, 1, 0),
        &d(last),
        "timestamp out of range",
    );
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &d(last), &span(0, 0, 86_399_999_999)).unwrap(),
        ceiling
    );

    // `date + time` → timestamp; `24:00:00` on the last day is year 10000.
    assert_eq!(
        eval_binary_op(BinaryOp::Add, &d(last), &Value::Time(86_399_999_999)).unwrap(),
        ceiling
    );
    refused(
        BinaryOp::Add,
        &d(last),
        &Value::Time(86_400_000_000),
        "timestamp out of range",
    );
    refused(
        BinaryOp::Add,
        &Value::Time(86_400_000_000),
        &d(last),
        "timestamp out of range",
    );
}

/// The interval passes' calendar is exact outside `time`'s years: the Julian day of a
/// date agrees with `time`'s wherever `time` holds the date, and lands on 18.6's own
/// constants at both ends of its timestamp range.
#[test]
fn interval_pass_calendar_matches_time_and_postgres() {
    use super::super::arithmetic::{
        POSTGRES_EPOCH_JDATE, TIMESTAMP_END_JULIAN, days_in_month, julian_day,
    };
    let mut date = time::Date::from_calendar_date(-9999, time::Month::January, 1).unwrap();
    while let Some(next) = date.checked_add(time::Duration::days(97)) {
        let (year, month) = (i64::from(date.year()), i64::from(u8::from(date.month())));
        assert_eq!(
            julian_day(year, month, i64::from(date.day())),
            i64::from(date.to_julian_day()),
            "{date}"
        );
        assert_eq!(
            days_in_month(year, month),
            i64::from(date.month().length(date.year())),
            "{date}"
        );
        date = next;
    }
    assert_eq!(julian_day(2000, 1, 1), POSTGRES_EPOCH_JDATE);
    assert_eq!(julian_day(-4713, 11, 24), 0);
    assert_eq!(julian_day(294_277, 1, 1), TIMESTAMP_END_JULIAN);
    assert_eq!(days_in_month(10_000, 2), 29);
    assert_eq!(days_in_month(10_100, 2), 28);
    assert_eq!(days_in_month(-4_800, 2), 29);
}

/// Passes that stay inside 18.6's timestamp range and end outside the date domain
/// are still refused on the result (`EDGES.md` row 142) — 18.6 answers
/// `11995-07-30 00:00:00` here, so no differential fixture can pin the code. And the
/// passes read an instant's calendar fields at the offset it carries, which is what
/// `docs/temporal.md` records for a timestamp an older build stored off UTC.
#[test]
fn interval_passes_refuse_a_result_outside_the_domain() {
    let span = |months, days, micros| Value::Interval(PgInterval::new(months, days, micros));
    let e = eval_binary_op(
        BinaryOp::Add,
        &Value::timestamp(datetime!(9999-12-31 00:00:00 UTC)),
        &span(284_000 * 12, -103_000_000, 0),
    )
    .expect_err("year 11995");
    assert_eq!(
        (e.sqlstate(), e.to_string().as_str()),
        ("22008", "timestamp out of range")
    );
    assert_eq!(
        eval_binary_op(
            BinaryOp::Add,
            &Value::timestamp(datetime!(2026-06-15 22:00:00 -5)),
            &span(0, 0, 0),
        )
        .unwrap(),
        Value::timestamp(datetime!(2026-06-15 22:00:00 UTC))
    );
}

/// `date_trunc` leaves the domain from inside it at one field: a `decade` over
/// years 1–9 is year 0, which 18.6 answers as `0001-01-01 00:00:00 BC` and pglet
/// once rendered `0000-01-01 00:00:00`. Every other field stays in — `0001-01-01`
/// is a Monday and the first day of its century and millennium. A source already
/// outside the domain (a value an older build stored) is refused too.
#[test]
fn date_trunc_holds_the_domain() {
    use time::macros::date;
    let field = |f: &str| Value::Text(f.to_string());
    for f in ["week", "millennium", "century", "year", "day"] {
        assert_eq!(
            super::super::temporal_fn::eval_date_trunc(
                &field(f),
                &Value::timestamp(datetime!(0001-01-03 00:00:00 UTC))
            )
            .unwrap(),
            Value::timestamp(
                datetime!(0001-01-01 00:00:00 UTC)
                    .replace_day(if f == "day" { 3 } else { 1 })
                    .unwrap()
            ),
            "{f}"
        );
    }
    assert_eq!(
        super::super::temporal_fn::eval_date_trunc(
            &field("decade"),
            &Value::date(date!(0010 - 01 - 01))
        )
        .unwrap(),
        Value::timestamp(datetime!(0010-01-01 00:00:00 UTC))
    );
    for source in [
        Value::timestamp(datetime!(0001-01-03 12:00:00 UTC)),
        Value::timestamp(datetime!(0009-12-31 23:59:59.999999 UTC)),
        Value::date(date!(0005 - 06 - 01)),
    ] {
        let e = super::super::temporal_fn::eval_date_trunc(&field("decade"), &source)
            .expect_err("year 0");
        assert_eq!(
            (e.sqlstate(), e.to_string().as_str()),
            ("22008", "timestamp out of range"),
            "{source:?}"
        );
    }
    for source in [
        Value::timestamp(datetime!(0000-12-31 12:00:00 UTC)),
        Value::date(date!(0000 - 12 - 31)),
    ] {
        let e = super::super::temporal_fn::eval_date_trunc(&field("day"), &source)
            .expect_err("out of domain");
        assert_eq!(e.sqlstate(), "22008", "{source:?}");
    }
}
