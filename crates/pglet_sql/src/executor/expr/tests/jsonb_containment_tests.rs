//! Tests for jsonb `@>` containment.

use super::super::json_ops::{jsonb_contains, jsonb_contains_nested};
use crate::types::{JsonbValue, parse_jsonb};

/// Parse a jsonb literal the way a query would (canonical form included).
fn j(s: &str) -> JsonbValue {
    parse_jsonb(s).expect("valid jsonb")
}

fn contains(container: &str, contained: &str) -> bool {
    jsonb_contains(&j(container), &j(contained))
}

#[test]
fn array_containment_is_subset_with_membership() {
    assert!(contains("[1,2,3]", "[1,3]"));
    assert!(contains("[1,2,3]", "[3,1]"), "element order is irrelevant");
    assert!(contains("[1]", "[1,1]"), "multiplicity is ignored");
    assert!(
        contains("[1,2]", "[]"),
        "an empty rhs is vacuously contained"
    );
    assert!(!contains("[1,2]", "[1,2,3]"));
}

#[test]
fn raw_scalar_exception_is_top_level_and_one_directional() {
    // "An array may contain a primitive value" — the documented exception.
    assert!(contains(r#"["a","b"]"#, r#""a""#));
    assert!(contains("[1,2,3]", "3"));
    // ...but a scalar never contains an array, and two scalars are equality.
    assert!(!contains(r#""a""#, r#"["a"]"#));
    assert!(contains(r#""a""#, r#""a""#));
    assert!(!contains(r#""a""#, r#""b""#));
    // The exception does not survive one level down: an object's value
    // comparison demands matching types, so the array `[1,2]` under key "a"
    // does *not* contain the scalar `1`.
    assert!(!contains(r#"{"a":[1,2]}"#, r#"{"a":1}"#));
    // Which is exactly the top-level/nested split — same operands, different
    // entry point.
    assert!(jsonb_contains(&j("[1,2]"), &j("1")));
    assert!(!jsonb_contains_nested(&j("[1,2]"), &j("1")));
}

#[test]
fn container_elements_match_only_containers() {
    // A container rhs element is matched against lhs *container* elements
    // (Postgres collects them into a temp array and gives up when empty).
    assert!(contains(r#"[{"a":1},{"b":2}]"#, r#"[{"a":1}]"#));
    assert!(contains(r#"[{"a":1,"b":2}]"#, r#"[{"b":2}]"#));
    assert!(!contains("[1,2,3]", "[[1]]"));
    assert!(contains("[[1,2]]", "[[1]]"));
}

#[test]
fn kinds_never_cross_and_types_must_match() {
    assert!(!contains(r#"{"a":1}"#, "[1]"));
    assert!(!contains("[1]", r#"{"a":1}"#));
    assert!(!contains(r#"{"a":1}"#, r#"{"a":"1"}"#), "1 is not \"1\"");
    assert!(!contains(r#"{"a":true}"#, r#"{"a":1}"#));
    assert!(!contains(r#"{"a":1}"#, r#"{"a":1,"b":2}"#));
    // A JSON null is a value that matches by equality, and numeric scale folds
    // (`PgNumeric` equality), both matching Postgres.
    assert!(contains("[null,1]", "[null]"));
    assert!(contains(r#"{"a":1.0}"#, r#"{"a":1}"#));
}

#[test]
fn object_containment_is_key_order_insensitive() {
    // Falls out of canonical form rather than the containment code: both sides
    // are sorted before the key lookup ever runs.
    assert!(contains(r#"{"b":2,"a":1}"#, r#"{"a":1,"b":2}"#));
    assert!(contains(r#"{"a":1,"b":2,"c":3}"#, r#"{"c":3,"a":1}"#));
}
