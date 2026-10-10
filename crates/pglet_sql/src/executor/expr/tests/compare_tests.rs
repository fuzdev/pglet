//! Tests for `values_equal` and `values_compare`.

use super::super::compare::{values_compare, values_equal};
use crate::types::{JsonbValue, PgNumeric, Value};
use std::cmp::Ordering;
use time::OffsetDateTime;

fn uuid(b: u8) -> Value {
    let mut bytes = [0u8; 16];
    bytes[15] = b;
    Value::Uuid(bytes)
}

#[test]
fn distinct_uuids_are_not_equal() {
    // The live spine bug: two distinct UUIDs must NOT compare equal. Both the
    // equality path and the ordering-based equality (`== Some(Equal)`) must
    // agree they differ.
    assert_eq!(values_equal(&uuid(1), &uuid(2)), Some(false));
    assert_ne!(values_compare(&uuid(1), &uuid(2)), Some(Ordering::Equal));
    // ...and an identical UUID compares equal both ways.
    assert_eq!(values_equal(&uuid(1), &uuid(1)), Some(true));
    assert_eq!(values_compare(&uuid(1), &uuid(1)), Some(Ordering::Equal));
    // UUID order is lexicographic over the bytes.
    assert_eq!(values_compare(&uuid(1), &uuid(2)), Some(Ordering::Less));
}

#[test]
fn distinct_bytea_are_not_equal() {
    let a = Value::Bytea(vec![1, 2, 3]);
    let b = Value::Bytea(vec![1, 2, 4]);
    assert_eq!(values_equal(&a, &b), Some(false));
    assert_ne!(values_compare(&a, &b), Some(Ordering::Equal));
    assert_eq!(values_compare(&a, &b), Some(Ordering::Less));
    assert_eq!(values_compare(&a, &a), Some(Ordering::Equal));
}

#[test]
fn distinct_timestamps_are_not_equal() {
    let a = Value::timestamp(OffsetDateTime::from_unix_timestamp(1_000).unwrap());
    let b = Value::timestamp(OffsetDateTime::from_unix_timestamp(2_000).unwrap());
    assert_eq!(values_equal(&a, &b), Some(false));
    assert_eq!(values_compare(&a, &b), Some(Ordering::Less));
    assert_eq!(values_compare(&b, &a), Some(Ordering::Greater));
}

#[test]
fn distinct_jsonb_are_not_equal() {
    let a = Value::Jsonb(JsonbValue::Number(PgNumeric::from_i128(1)));
    let b = Value::Jsonb(JsonbValue::Number(PgNumeric::from_i128(2)));
    assert_eq!(values_equal(&a, &b), Some(false));
    assert_ne!(values_compare(&a, &b), Some(Ordering::Equal));
    // Equal jsonb compares Equal.
    assert_eq!(values_compare(&a, &a), Some(Ordering::Equal));
}

fn int_array(elems: &[Option<i32>]) -> Value {
    Value::Array(crate::types::ArrayValue::new(
        crate::types::TypeOid::Int4,
        elems
            .iter()
            .map(|e| e.map_or(Value::Null, Value::Int4))
            .collect(),
    ))
}

// Bug: a NULL array element sorted LESS; Postgres `array_cmp` sorts it GREATER
// than any non-NULL. Verified against live PG: `ARRAY[1,NULL] > ARRAY[1,2]`,
// `> ARRAY[1,0]`, `< ARRAY[1,0]` false; ORDER BY places `{1,NULL}` last.
#[test]
fn array_null_element_sorts_greatest() {
    let a = int_array(&[Some(1), None]);
    assert_eq!(
        values_compare(&a, &int_array(&[Some(1), Some(2)])),
        Some(Ordering::Greater)
    );
    assert_eq!(
        values_compare(&a, &int_array(&[Some(1), Some(0)])),
        Some(Ordering::Greater)
    );
    assert_eq!(
        values_compare(&int_array(&[Some(1), Some(0)]), &a),
        Some(Ordering::Less)
    );
    // A shorter prefix still sorts before either (length breaks the final tie).
    assert_eq!(
        values_compare(&int_array(&[Some(1)]), &a),
        Some(Ordering::Less)
    );
    // Equal (both NULL at the same slot) stays equal, consistent with `values_equal`.
    assert_eq!(values_compare(&a, &a), Some(Ordering::Equal));
    assert_eq!(values_equal(&a, &a), Some(true));
}

// Bug: no `(Date, Timestamp)` cross arm — a DATE and a TIMESTAMP at the same
// midnight instant compared unequal. Postgres promotes the date to midnight UTC
// (`date2timestamp`): `'2024-01-01'::date = '2024-01-01 00:00:00'::timestamp`.
#[test]
fn date_timestamp_cross_compare_at_midnight() {
    use time::macros::{date, datetime};
    let d = Value::date(date!(2024 - 01 - 01));
    let midnight = Value::timestamp(datetime!(2024-01-01 00:00:00 UTC));
    let just_after = Value::timestamp(datetime!(2024-01-01 00:00:01 UTC));
    let just_before = Value::timestamp(datetime!(2023-12-31 23:59:59 UTC));
    // Equal at the exact midnight boundary, both directions and both functions.
    assert_eq!(values_equal(&d, &midnight), Some(true));
    assert_eq!(values_equal(&midnight, &d), Some(true));
    assert_eq!(values_compare(&d, &midnight), Some(Ordering::Equal));
    assert_eq!(values_compare(&midnight, &d), Some(Ordering::Equal));
    // Ordering just around midnight.
    assert_eq!(values_compare(&d, &just_after), Some(Ordering::Less));
    assert_eq!(values_compare(&d, &just_before), Some(Ordering::Greater));
    assert_eq!(values_compare(&just_after, &d), Some(Ordering::Greater));
    assert_eq!(values_equal(&d, &just_after), Some(false));
}

fn jb(s: &str) -> Value {
    Value::Jsonb(crate::types::parse_jsonb(s).unwrap())
}

// Bug: jsonb compared by raw JSON text, not Postgres's B-tree order. Type rank
// is `null < string < number < bool < array < object` (verified live).
#[test]
fn jsonb_btree_type_rank_order() {
    // The full ladder, in Postgres's sorted order.
    let ladder = [
        "null",
        "\"a\"",
        "\"b\"",
        "1",
        "2",
        "false",
        "true",
        "[1]",
        "[1,2]",
        "{\"a\":1}",
        "{\"a\":1,\"b\":2}",
    ];
    for w in ladder.windows(2) {
        assert_eq!(
            values_compare(&jb(w[0]), &jb(w[1])),
            Some(Ordering::Less),
            "{} should sort before {}",
            w[0],
            w[1]
        );
        assert_eq!(
            values_compare(&jb(w[1]), &jb(w[0])),
            Some(Ordering::Greater)
        );
    }
    // Cross-type rank, not text order: number > string (`1 > "z"`, text order
    // would put `"` before `1`).
    assert_eq!(
        values_compare(&jb("1"), &jb("\"z\"")),
        Some(Ordering::Greater)
    );
}

// Bug: text order gets the array-length-first and object-paircount-first
// tie-breaks wrong.
#[test]
fn jsonb_array_length_and_object_paircount_tiebreaks() {
    // Arrays: length first (`[1,2] < [1,2,0]`), then element-wise (`[1,2] < [1,3]`,
    // `[10] < [9]` is false — 10 > 9). `[1] < [0,0]` (shorter wins regardless).
    assert_eq!(
        values_compare(&jb("[1,2]"), &jb("[1,2,0]")),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare(&jb("[1,2]"), &jb("[1,3]")),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare(&jb("[10]"), &jb("[9]")),
        Some(Ordering::Greater)
    );
    assert_eq!(
        values_compare(&jb("[1]"), &jb("[0,0]")),
        Some(Ordering::Less)
    );
    // Nested: length-first applies at every level (`[[1]] < [[0,0]]`).
    assert_eq!(
        values_compare(&jb("[[1]]"), &jb("[[0,0]]")),
        Some(Ordering::Less)
    );
    // Objects: pair count first (`{"a":1} < {"a":1,"b":1}`), then key then value.
    assert_eq!(
        values_compare(&jb("{\"a\":1}"), &jb("{\"a\":1,\"b\":1}")),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare(&jb("{\"a\":1}"), &jb("{\"b\":0}")),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare(&jb("{\"a\":2}"), &jb("{\"a\":3}")),
        Some(Ordering::Less)
    );
}

// Bug: the top-level empty-array anomaly — a whole-document scalar is stored as a
// raw-scalar array, so it sorts GREATER than an empty top-level array (verified
// live: `'1'::jsonb > '[]'::jsonb`, `'"a"'::jsonb > '[]'`). The anomaly is
// top-level only: nested `[1] < [[]]` follows pure type rank.
#[test]
fn jsonb_top_level_empty_array_anomaly() {
    for scalar in ["null", "\"a\"", "1", "false", "true"] {
        assert_eq!(
            values_compare(&jb(scalar), &jb("[]")),
            Some(Ordering::Greater),
            "scalar {scalar} should sort after top-level []"
        );
        assert_eq!(values_compare(&jb("[]"), &jb(scalar)), Some(Ordering::Less));
    }
    // But an empty top-level OBJECT keeps type rank (scalar < object).
    assert_eq!(values_compare(&jb("1"), &jb("{}")), Some(Ordering::Less));
    // And nested empty arrays follow pure type rank (number < array).
    assert_eq!(
        values_compare(&jb("[1]"), &jb("[[]]")),
        Some(Ordering::Less)
    );
    // Non-empty top-level array keeps type rank too (scalar < array).
    assert_eq!(values_compare(&jb("1"), &jb("[0]")), Some(Ordering::Less));
}

#[test]
fn null_operand_propagates() {
    assert_eq!(values_equal(&uuid(1), &Value::Null), None);
    assert_eq!(values_compare(&uuid(1), &Value::Null), None);
}
