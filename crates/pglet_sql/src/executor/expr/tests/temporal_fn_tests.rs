//! Tests for `EXTRACT`, `date_part` and `date_trunc`.

use super::super::eval::eval_expr;
use crate::parser::parse_expr;
use crate::types::{Value, value_to_text};

fn eval_lit(sql: &str) -> Value {
    let expr = parse_expr(sql, &crate::catalog::Schema::new()).expect("parse");
    eval_expr(&expr, &[], &[], &[], &[]).expect("eval")
}

/// The `numeric` display text (scale-carrying) of an `EXTRACT` result.
fn num_text(sql: &str) -> String {
    match eval_lit(sql) {
        Value::Numeric(n) => n.to_display_string(),
        other => panic!("expected numeric, got {other:?}"),
    }
}

/// The rendered text of a temporal (`date_trunc`) result.
fn ts_text(sql: &str) -> String {
    value_to_text(&eval_lit(sql)).expect("non-null temporal")
}

fn err_sqlstate(sql: &str) -> String {
    let schema = crate::catalog::Schema::new();
    match parse_expr(sql, &schema) {
        Err(e) => e.sqlstate().to_string(),
        Ok(expr) => eval_expr(&expr, &[], &[], &[], &[])
            .expect_err("expected an error")
            .sqlstate()
            .to_string(),
    }
}

#[test]
fn date_trunc_cascade_and_week_and_century() {
    assert_eq!(
        ts_text("date_trunc('month', '2024-03-15 12:34:56'::timestamp)"),
        "2024-03-01 00:00:00"
    );
    assert_eq!(
        ts_text("date_trunc('hour', '2024-03-15 12:34:56.789'::timestamp)"),
        "2024-03-15 12:00:00"
    );
    // `week` → Monday of the ISO week (2024-03-15 is a Friday → 03-11).
    assert_eq!(
        ts_text("date_trunc('week', '2024-03-15 12:34:56'::timestamp)"),
        "2024-03-11 00:00:00"
    );
    // "no year 0": century 21 is 2001–2100. A date implicit-casts to timestamp.
    assert_eq!(
        ts_text("date_trunc('century', '2024-03-15'::date)"),
        "2001-01-01 00:00:00"
    );
    // The interval variant truncates the (months, days, micros) cascade.
    assert_eq!(
        ts_text("date_trunc('day', INTERVAL '5 days 3 hours 27 minutes')"),
        "5 days"
    );
}

#[test]
fn extract_numeric_scales() {
    // second/epoch → scale 6; milliseconds → scale 3; integer fields → scale 0.
    assert_eq!(
        num_text("extract(second from '2024-03-15 13:45:06.789'::timestamp)"),
        "6.789000"
    );
    assert_eq!(
        num_text("extract(milliseconds from '2024-03-15 13:45:06.789'::timestamp)"),
        "6789.000"
    );
    assert_eq!(
        num_text("extract(microseconds from '2024-03-15 13:45:06.789'::timestamp)"),
        "6789000"
    );
    assert_eq!(
        num_text("extract(year from '2024-03-15 13:45:06.789'::timestamp)"),
        "2024"
    );
}

#[test]
fn extract_common_fields() {
    // dow is 0=Sunday..6=Saturday (Friday = 5); isodow is 1=Monday..7=Sunday.
    assert_eq!(num_text("extract(dow from '2024-03-15'::date)"), "5");
    assert_eq!(num_text("extract(isodow from '2024-03-15'::date)"), "5");
    assert_eq!(num_text("extract(quarter from '2024-08-15'::date)"), "3");
    assert_eq!(num_text("extract(doy from '2024-03-15'::date)"), "75");
    // interval epoch: 5 days 3 hours = 442800 s (scale 6).
    assert_eq!(
        num_text("extract(epoch from INTERVAL '5 days 3 hours')"),
        "442800.000000"
    );
    // time-of-day fields from a time value.
    assert_eq!(num_text("extract(hour from '13:45:06.789'::time)"), "13");
    assert_eq!(
        num_text("extract(second from '13:45:06.789'::time)"),
        "6.789000"
    );
}

#[test]
fn date_part_returns_float8_and_casts_date() {
    // date_part yields float8, not numeric.
    assert_eq!(
        eval_lit("date_part('hour', '2024-03-15 13:45:06'::timestamp)"),
        Value::Float8(13.0)
    );
    // date_part(text, date) casts date→timestamp, so a time field succeeds (= 0)
    // where EXTRACT(hour FROM date) fails.
    assert_eq!(
        eval_lit("date_part('hour', '2024-03-15'::date)"),
        Value::Float8(0.0)
    );
}

#[test]
fn temporal_fields_fail_loud() {
    // Unknown field → 22023; recognized-but-wrong-source → 0A000.
    assert_eq!(
        err_sqlstate("extract(nonsense from '2024-03-15'::date)"),
        "22023"
    );
    assert_eq!(
        err_sqlstate("extract(hour from '2024-03-15'::date)"),
        "0A000"
    );
    // date_trunc consults only the units tier: a special-only field (dow) is
    // *unrecognized* (22023), not merely unsupported.
    assert_eq!(
        err_sqlstate("date_trunc('dow', '2024-03-15 12:00:00'::timestamp)"),
        "22023"
    );
    // week is a unit but unsupported for an interval → 0A000.
    assert_eq!(
        err_sqlstate("date_trunc('week', INTERVAL '5 days')"),
        "0A000"
    );
    // NULL field / source → NULL (strict), never an error.
    assert_eq!(eval_lit("extract(year from NULL::timestamp)"), Value::Null);
    assert_eq!(
        eval_lit("date_trunc('month', NULL::timestamp)"),
        Value::Null
    );
}
