//! Tests for jsonb `->` / `->>` field access.

use super::super::json_ops::eval_json_get;
use crate::types::{JsonbValue, Value};

fn obj() -> Value {
    Value::Jsonb(JsonbValue::Object(vec![(
        "name".to_string(),
        JsonbValue::String("Alice".to_string()),
    )]))
}

#[test]
fn jsonb_object_field_as_text_and_jsonb() {
    // `->>` returns the raw text; `->` returns the JSONB scalar.
    let key = Value::Text("name".to_string());
    assert_eq!(
        eval_json_get(&obj(), &key, true).unwrap(),
        Value::Text("Alice".to_string())
    );
    assert_eq!(
        eval_json_get(&obj(), &key, false).unwrap(),
        Value::Jsonb(JsonbValue::String("Alice".to_string()))
    );
}

#[test]
fn missing_key_and_null_operand_yield_null() {
    let missing = Value::Text("absent".to_string());
    assert_eq!(eval_json_get(&obj(), &missing, true).unwrap(), Value::Null);
    assert_eq!(
        eval_json_get(&Value::Null, &Value::Text("k".into()), true).unwrap(),
        Value::Null
    );
}

#[test]
fn text_operand_parses_valid_json_but_fails_loud_on_malformed() {
    // A `text` left operand is parsed as JSON (pglet leniency). Valid text works…
    let valid = Value::Text(r#"{"name":"Bob"}"#.to_string());
    assert_eq!(
        eval_json_get(&valid, &Value::Text("name".into()), true).unwrap(),
        Value::Text("Bob".to_string())
    );
    // …malformed text fails loud (22P02), not a silent NULL (the old behavior).
    let bad = Value::Text(r#"{"name":@@bad}"#.to_string());
    let e = eval_json_get(&bad, &Value::Text("name".into()), true)
        .expect_err("malformed json text must fail loud");
    assert_eq!(e.sqlstate(), "22P02");
}
