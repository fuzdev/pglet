//! Tests for the string functions.

use super::super::eval::eval_expr;
use crate::parser::parse_expr;
use crate::types::Value;

/// Evaluate a literal-only scalar expression (no columns, no params).
fn eval_lit(sql: &str) -> Value {
    let expr = parse_expr(sql, &crate::catalog::Schema::new()).expect("parse");
    eval_expr(&expr, &[], &[], &[], &[]).expect("eval")
}

/// The crate error from a literal-only expression that must fail (at parse or eval).
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
fn strict_string_functions() {
    let text = |s: &str| Value::Text(s.to_string());
    assert_eq!(eval_lit("upper('abc')"), text("ABC"));
    // length is character-based (é is one character).
    assert_eq!(eval_lit("length('josé')"), Value::Int4(4));
    assert_eq!(eval_lit("replace('xxx', 'xx', 'y')"), text("yx"));
    // The substring negative-start clip gotcha.
    assert_eq!(eval_lit("substring('12345' from -2 for 5)"), text("12"));
    assert_eq!(eval_lit("substring('Thomas' from 2 for 3)"), text("hom"));
    assert_eq!(eval_lit("substring('Thomas' for 2)"), text("Th"));
    // trim is a character set, not a substring.
    assert_eq!(eval_lit("btrim('xyxtrimyyx', 'xyz')"), text("trim"));
    assert_eq!(eval_lit("ltrim('  hi  ')"), text("hi  "));
    // position/strpos are 1-based; an empty needle is 1.
    assert_eq!(eval_lit("position('om' in 'Thomas')"), Value::Int4(3));
    assert_eq!(eval_lit("strpos('Thomas', 'z')"), Value::Int4(0));
    assert_eq!(eval_lit("position('' in 'Thomas')"), Value::Int4(1));
    // lpad/rpad: cyclic fill, truncation, empty-fill.
    assert_eq!(eval_lit("lpad('hi', 5, 'xy')"), text("xyxhi"));
    assert_eq!(eval_lit("rpad('hi', 5, 'xy')"), text("hixyx"));
    assert_eq!(eval_lit("lpad('hello', 3)"), text("hel"));
    assert_eq!(eval_lit("lpad('hi', 5, '')"), text("hi"));
    // split_part: negative n from the end, out-of-range → ''.
    assert_eq!(eval_lit("split_part('a,b,c', ',', -1)"), text("c"));
    assert_eq!(eval_lit("split_part('a,b,c', ',', 2)"), text("b"));
    assert_eq!(eval_lit("split_part('a,b,c', ',', 9)"), text(""));
}

#[test]
fn strict_string_functions_propagate_null() {
    // A NULL argument makes the whole strict call NULL.
    assert_eq!(eval_lit("upper(NULL)"), Value::Null);
    assert_eq!(eval_lit("substring(NULL from 1 for 2)"), Value::Null);
    assert_eq!(eval_lit("split_part(NULL, ',', 1)"), Value::Null);
}

#[test]
fn variadic_functions_null_rules() {
    let text = |s: &str| Value::Text(s.to_string());
    // concat skips NULLs (all-NULL → ''); concat_ws's separator is strict.
    assert_eq!(eval_lit("concat('a', NULL, 'b')"), text("ab"));
    assert_eq!(eval_lit("concat(NULL, NULL)"), text(""));
    assert_eq!(eval_lit("concat_ws(',', 'a', NULL, 'b')"), text("a,b"));
    assert_eq!(eval_lit("concat_ws(NULL, 'a', 'b')"), Value::Null);
    // greatest/least skip NULLs; all-NULL → NULL.
    assert_eq!(eval_lit("greatest(1, NULL, 3)"), Value::Int4(3));
    assert_eq!(eval_lit("least(1, NULL, 3)"), Value::Int4(1));
    assert_eq!(eval_lit("greatest(NULL, NULL)"), Value::Null);
    // nullif: NULL when equal; a NULL argument returns the first arg.
    assert_eq!(eval_lit("nullif(5, 5)"), Value::Null);
    assert_eq!(eval_lit("nullif(5, 6)"), Value::Int4(5));
    assert_eq!(eval_lit("nullif(NULL, 5)"), Value::Null);
    assert_eq!(eval_lit("nullif(5, NULL)"), Value::Int4(5));
}

#[test]
fn string_functions_fail_loud() {
    // A negative substring length (22011), a zero split_part position (22023),
    // and a non-text argument / wrong arity (42883, undefined_function).
    assert_eq!(err_sqlstate("substring('x' from 1 for -1)"), "22011");
    assert_eq!(err_sqlstate("split_part('a,b,c', ',', 0)"), "22023");
    assert_eq!(err_sqlstate("length(5)"), "42883");
    assert_eq!(err_sqlstate("upper('a', 'b')"), "42883");
    // lower(non-text) matches upper's 42883 (not 0A000).
    assert_eq!(err_sqlstate("lower(5)"), "42883");
    // A pathological lpad length fails loud (54000) rather than allocating GBs.
    assert_eq!(err_sqlstate("lpad('x', 2000000000)"), "54000");
    // A non-text concat_ws separator resolves to no function (42883).
    assert_eq!(err_sqlstate("concat_ws(1, 'a', 'b')"), "42883");
}

#[test]
fn variadic_common_type_unification() {
    // greatest/least unify to the `select_common_type` result so the returned
    // value's type matches the declared one (no binary-protocol mis-decode).
    assert!(matches!(
        eval_lit("greatest(1::int4, 5::int8)"),
        Value::Int8(5)
    ));
    assert!(matches!(
        eval_lit("least(1::int4, 5::int8)"),
        Value::Int8(1)
    ));
    // `nullif` follows the *operator-resolution* rulebook instead, so it can differ
    // from its neighbours on the same pair. There is no `int4 = numeric` operator,
    // so argument one is coerced and the result is numeric — matching Postgres.
    assert!(matches!(eval_lit("nullif(5, 6.0)"), Value::Numeric(_)));
    // ...but a registered cross-type `=` leaves argument one alone, where
    // `greatest` of the same pair would widen to int8.
    assert!(matches!(
        eval_lit("nullif(5::int4, 6::int8)"),
        Value::Int4(5)
    ));
    assert!(matches!(
        eval_lit("nullif(5::int8, 6::int4)"),
        Value::Int8(5)
    ));
    // Incompatible greatest/least argument types fail loud (42804).
    assert_eq!(err_sqlstate("greatest('a'::text, 1)"), "42804");
}

#[test]
fn length_of_bytea_is_byte_count() {
    // length(bytea) is the byte count (Postgres has both length(text) and
    // length(bytea)); length('\xdead'::bytea) = 2.
    assert_eq!(eval_lit(r"length('\xdead'::bytea)"), Value::Int4(2));
}
