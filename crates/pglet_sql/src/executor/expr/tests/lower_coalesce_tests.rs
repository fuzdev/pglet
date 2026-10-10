//! Tests for `lower` and `COALESCE` evaluation.

use super::super::eval::eval_expr;
use crate::parser::parse_expr;
use crate::types::Value;

/// Evaluate a scalar expression's SQL against a single-column row binding.
fn eval(sql: &str, col: &str, value: Value) -> Value {
    let expr = parse_expr(sql, &crate::catalog::Schema::new()).expect("parse");
    eval_expr(&expr, &[col], &[], &[], &[value]).expect("eval")
}

#[test]
fn lower_lowercases_text_and_passes_null() {
    assert_eq!(
        eval("LOWER(email)", "email", Value::Text("A@X.Com".into())),
        Value::Text("a@x.com".into())
    );
    // NULL → NULL.
    assert_eq!(eval("LOWER(email)", "email", Value::Null), Value::Null);
}

#[test]
fn lower_on_non_text_fails_loud() {
    let expr = parse_expr("LOWER(n)", &crate::catalog::Schema::new()).unwrap();
    // An integer operand is not text → error, not a silent pass-through.
    assert!(eval_expr(&expr, &["n"], &[], &[], &[Value::Int4(7)]).is_err());
}

#[test]
fn coalesce_returns_first_non_null() {
    // First arg non-null → it wins.
    assert_eq!(
        eval(
            "COALESCE(scope, 'GLOBAL')",
            "scope",
            Value::Text("team".into())
        ),
        Value::Text("team".into())
    );
    // First arg NULL → the literal fallback.
    assert_eq!(
        eval("COALESCE(scope, 'GLOBAL')", "scope", Value::Null),
        Value::Text("GLOBAL".into())
    );
}

#[test]
fn coalesce_all_null_is_null() {
    assert_eq!(
        eval("COALESCE(scope, NULL)", "scope", Value::Null),
        Value::Null
    );
}
