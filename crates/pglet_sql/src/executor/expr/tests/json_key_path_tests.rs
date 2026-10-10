//! Tests for jsonb key existence and path access.

use super::super::json_ops::{
    eval_json_exists, eval_json_exists_any_all, eval_json_path_get, resolve_array_index,
};
use crate::types::{ArrayValue, JsonbValue, TypeOid, Value, parse_jsonb};

/// A jsonb `Value` from JSON source text.
fn jb(s: &str) -> Value {
    Value::Jsonb(parse_jsonb(s).unwrap())
}
fn txt(s: &str) -> Value {
    Value::Text(s.to_string())
}
/// A `text[]` array `Value` (a `Value::Null` element models a NULL entry).
fn text_arr(items: Vec<Value>) -> Value {
    Value::Array(ArrayValue::new(TypeOid::Text, items))
}

#[test]
fn exists_object_array_and_scalar_quirk() {
    assert_eq!(
        eval_json_exists(&jb(r#"{"a":1}"#), &txt("a")).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        eval_json_exists(&jb(r#"{"a":1}"#), &txt("x")).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        eval_json_exists(&jb(r#"["a","b"]"#), &txt("b")).unwrap(),
        Value::Bool(true)
    );
    // PG's scalar quirk: a top-level scalar String equals the key directly…
    assert_eq!(
        eval_json_exists(&jb(r#""a""#), &txt("a")).unwrap(),
        Value::Bool(true)
    );
    // …but a non-string scalar never matches (no quirk).
    assert_eq!(
        eval_json_exists(&jb("1"), &txt("1")).unwrap(),
        Value::Bool(false)
    );
    // A NULL operand → NULL (not false).
    assert_eq!(
        eval_json_exists(&Value::Null, &txt("a")).unwrap(),
        Value::Null
    );
}

#[test]
fn any_all_skip_null_elements_and_empty_array() {
    let j = jb(r#"{"a":1}"#);
    // A NULL array element is SKIPPED (not propagated to NULL) — unlike `#>`,
    // where a NULL path element makes the whole result NULL.
    assert_eq!(
        eval_json_exists_any_all(&j, &text_arr(vec![txt("a"), Value::Null]), false).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        eval_json_exists_any_all(&j, &text_arr(vec![txt("a"), Value::Null]), true).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        eval_json_exists_any_all(&j, &text_arr(vec![txt("z"), Value::Null]), true).unwrap(),
        Value::Bool(false)
    );
    // Empty array: `?|` → false, `?&` → true.
    assert_eq!(
        eval_json_exists_any_all(&j, &text_arr(vec![]), false).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        eval_json_exists_any_all(&j, &text_arr(vec![]), true).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn path_get_negative_index_scalar_mid_null_path_and_json_null_target() {
    // Nested object → array → negative-from-end index.
    assert_eq!(
        eval_json_path_get(
            &jb(r#"{"a":{"b":[10,20,30]}}"#),
            &text_arr(vec![txt("a"), txt("b"), txt("-1")]),
            false
        )
        .unwrap(),
        jb("30")
    );
    // A scalar mid-path → SQL NULL (no `?`-style scalar quirk here).
    assert_eq!(
        eval_json_path_get(
            &jb(r#"{"a":5}"#),
            &text_arr(vec![txt("a"), txt("b")]),
            false
        )
        .unwrap(),
        Value::Null
    );
    // A NULL path element → the whole result is NULL (propagated, unlike `?|`/`?&`).
    assert_eq!(
        eval_json_path_get(
            &jb(r#"{"a":1}"#),
            &text_arr(vec![txt("a"), Value::Null]),
            false
        )
        .unwrap(),
        Value::Null
    );
    // An empty path → the whole document.
    assert_eq!(
        eval_json_path_get(&jb(r#"{"a":1}"#), &text_arr(vec![]), false).unwrap(),
        jb(r#"{"a":1}"#)
    );
    // A JSON-null target: `#>` keeps jsonb `null`, `#>>` gives SQL NULL.
    assert_eq!(
        eval_json_path_get(&jb(r#"{"a":null}"#), &text_arr(vec![txt("a")]), false).unwrap(),
        Value::Jsonb(JsonbValue::Null)
    );
    assert_eq!(
        eval_json_path_get(&jb(r#"{"a":null}"#), &text_arr(vec![txt("a")]), true).unwrap(),
        Value::Null
    );
}

#[test]
fn text_right_operand_parses_as_array_literal() {
    // Postgres implicitly casts a string literal to `text[]` for `#>`/`?|`/`?&`;
    // pglet mirrors that at eval time (a `Value::Text` right operand parses as a
    // `{…}` array literal), so `data #> '{a,b}'` works, not just `ARRAY['a','b']`.
    assert_eq!(
        eval_json_path_get(&jb(r#"{"a":{"b":42}}"#), &txt("{a,b}"), false).unwrap(),
        jb("42")
    );
    // The empty-array literal `'{}'` is the empty path → the whole document.
    assert_eq!(
        eval_json_path_get(&jb(r#"{"a":1}"#), &txt("{}"), false).unwrap(),
        jb(r#"{"a":1}"#)
    );
    // `?|`/`?&` accept the text form too.
    assert_eq!(
        eval_json_exists_any_all(&jb(r#"{"a":1}"#), &txt("{a,x}"), false).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn resolve_array_index_bounds() {
    assert_eq!(resolve_array_index(3, 0), Some(0));
    assert_eq!(resolve_array_index(3, 2), Some(2));
    assert_eq!(resolve_array_index(3, 3), None); // out of range (== len)
    assert_eq!(resolve_array_index(3, -1), Some(2)); // last
    assert_eq!(resolve_array_index(3, -3), Some(0)); // first
    assert_eq!(resolve_array_index(3, -4), None); // past the front
}
