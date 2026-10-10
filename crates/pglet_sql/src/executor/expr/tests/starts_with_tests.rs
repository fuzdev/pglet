//! Tests for `starts_with`.

use super::super::compare::eval_starts_with;
use crate::types::Value;

#[test]
fn literal_prefix_match_is_case_sensitive_and_not_wildcard() {
    let v = |s: &str| Value::Text(s.to_string());
    let ok = |a: &Value, b: &Value| eval_starts_with(a, b).expect("text operands");
    assert_eq!(ok(&v("docs/a"), &v("docs/")), Some(true));
    assert_eq!(ok(&v("other/b"), &v("docs/")), Some(false));
    // `%` is matched literally, not as a wildcard.
    assert_eq!(ok(&v("a%b"), &v("a%")), Some(true));
    assert_eq!(ok(&v("axb"), &v("a%")), Some(false));
    // Case-sensitive.
    assert_eq!(ok(&v("Docs"), &v("docs")), Some(false));
    // NULL operands → SQL NULL.
    assert_eq!(ok(&Value::Null, &v("x")), None);
    assert_eq!(ok(&v("x"), &Value::Null), None);
}

#[test]
fn non_text_argument_fails_loud() {
    // Postgres has only `starts_with(text, text)`: a non-text argument on
    // either side is `42883`, never a silent `false`.
    let v = |s: &str| Value::Text(s.to_string());
    let err = eval_starts_with(&Value::Int4(1), &v("x")).expect_err("no such function");
    assert_eq!(err.sqlstate(), "42883");
    let err = eval_starts_with(&v("x"), &Value::Int4(1)).expect_err("no such function");
    assert_eq!(err.sqlstate(), "42883");
}
