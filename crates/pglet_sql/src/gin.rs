//! GIN decomposition: a containment value → the set of index keys it contributes.
//!
//! The single source of truth shared by **maintenance** (decomposing a stored
//! row's array/JSONB value into posting entries) and **probing** (decomposing the
//! `@>` constant into the query keys to intersect), so the two always agree on the
//! key set. Storage encodes each returned `Value` with the same single-column key
//! encoder a B-tree index uses (`encode_probe_key`), so a GIN entry is byte-for-byte
//! an ordinary `BtreeKey::Index` entry — just many per row.

use crate::types::{JsonbValue, Value};

/// Decompose a value into the GIN index keys it contributes.
///
/// - **Array** (`text[]`, `uuid[]`, …): each distinct non-NULL element, as its
///   scalar `Value`. NULL elements are skipped — PG's `@>` never matches *via* a
///   NULL element (a containment with a NULL on either side is false), so skipping
///   them never drops a true match; the recheck preserves the exact semantics.
/// - **JSONB**: a synthetic `Value::Text` token per object key and per scalar leaf
///   value (string/bool/null), recursing nested objects and arrays. **Lossy but
///   sound** — if `container @> query` then `decompose(query) ⊆ decompose(container)`,
///   so an intersection probe never drops a true match, and the mandatory `@>`
///   recheck removes the false candidates. Number leaves contribute **no** value
///   token (their text form isn't canonical across producers, so a token could
///   miss a true match); the enclosing object key still keys them and the recheck
///   confirms the value.
///
/// Anything else (a scalar, NULL) decomposes to no keys.
#[must_use]
pub fn gin_decompose(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(arr) => {
            let mut keys: Vec<Value> = Vec::new();
            for elem in &arr.elements {
                if !elem.is_null() && !keys.contains(elem) {
                    keys.push(elem.clone());
                }
            }
            keys
        }
        Value::Jsonb(json) => {
            let mut tokens: Vec<Value> = Vec::new();
            jsonb_collect_tokens(json, &mut tokens);
            tokens
        }
        _ => Vec::new(),
    }
}

/// Collect a JSONB value's key and scalar-leaf tokens into `out`.
///
/// Tokens are prefixed (`k\0…` key, `vs\0…` string, `vb\0…` bool, `vn` null) so a
/// key `"x"` and a string value `"x"` occupy disjoint token spaces — aliasing would
/// only cost selectivity, not correctness, but the tag keeps probes tight. `\0`
/// can't appear in the prefix position of a real key/value, so the spaces stay
/// disjoint. Keys are **not** path-qualified (PG's `jsonb_ops` is likewise lossy),
/// so the recheck is load-bearing.
fn jsonb_collect_tokens(json: &JsonbValue, out: &mut Vec<Value>) {
    match json {
        JsonbValue::Object(pairs) => {
            for (k, v) in pairs {
                push_token(out, Value::Text(format!("k\u{0}{k}")));
                jsonb_collect_tokens(v, out);
            }
        }
        JsonbValue::Array(items) => {
            for item in items {
                jsonb_collect_tokens(item, out);
            }
        }
        JsonbValue::String(s) => push_token(out, Value::Text(format!("vs\u{0}{s}"))),
        JsonbValue::Bool(b) => push_token(out, Value::Text(format!("vb\u{0}{b}"))),
        JsonbValue::Null => push_token(out, Value::Text("vn".to_string())),
        // A number's text form isn't canonical across producers; skip the value
        // token (the enclosing object key still indexes it; recheck confirms it).
        JsonbValue::Number(_) => {}
    }
}

/// Push `token` if not already present (dedup keeps a row's posting entries unique).
fn push_token(out: &mut Vec<Value>, token: Value) {
    if !out.contains(&token) {
        out.push(token);
    }
}

/// Whether `container`'s decomposition holds **every** key in `keys` — the GIN
/// candidate test for `container @> <query>` where `keys = gin_decompose(<query>)`.
///
/// This is the **same superset** an inverted-index posting-list intersection
/// returns (a row is a candidate iff it appears under every query key), so the
/// read-your-own-writes and oracle paths can compute it directly from a row's value
/// without materialized posting lists, and agree with the committed-index probe. It
/// is *lossy* (matches `gin_decompose`), so the caller still rechecks with the real
/// `@>`. Empty `keys` (an `x @> {}` probe) is vacuously contained — true for every
/// row.
///
/// A key is matched as the index matches it — by its encoded key — which a `Value`'s
/// own `==` reproduces for every element but one: a float NaN is unequal to itself
/// there and is one key in the index, so a NaN key is matched by any NaN element.
#[must_use]
pub fn gin_contains_keys(container: &Value, keys: &[Value]) -> bool {
    const fn is_nan(v: &Value) -> bool {
        match v {
            Value::Float4(f) => f.is_nan(),
            Value::Float8(f) => f.is_nan(),
            _ => false,
        }
    }
    if keys.is_empty() {
        return true;
    }
    let decomposed = gin_decompose(container);
    keys.iter()
        .all(|k| decomposed.contains(k) || (is_nan(k) && decomposed.iter().any(is_nan)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The candidate test agrees with the index's key equality for a float: a NaN key
    /// is held by an array holding a NaN of either sign, and `-0` by one holding `0`.
    #[test]
    fn a_float_key_is_contained_as_the_index_keys_it() {
        let arr = |elements: Vec<Value>| Value::Array(ArrayValue::new(TypeOid::Float8, elements));
        let row = arr(vec![Value::Float8(-f64::NAN), Value::Float8(0.0)]);
        assert!(gin_contains_keys(&row, &[Value::Float8(f64::NAN)]));
        assert!(gin_contains_keys(&row, &[Value::Float8(-0.0)]));
        assert!(!gin_contains_keys(&row, &[Value::Float8(1.5)]));
        assert!(!gin_contains_keys(
            &arr(vec![Value::Float8(1.5)]),
            &[Value::Float8(f64::NAN)]
        ));
    }
    use crate::types::{ArrayValue, PgNumeric, TypeOid};

    fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    #[test]
    fn array_decomposes_to_distinct_non_null_elements() {
        let arr = Value::Array(ArrayValue::new(
            TypeOid::Text,
            vec![text("a"), text("b"), text("a"), Value::Null],
        ));
        let keys = gin_decompose(&arr);
        assert_eq!(keys, vec![text("a"), text("b")]);
    }

    #[test]
    fn jsonb_object_decomposes_to_key_and_value_tokens() {
        let json = Value::Jsonb(JsonbValue::Object(vec![(
            "kind".to_string(),
            JsonbValue::String("note".to_string()),
        )]));
        let keys = gin_decompose(&json);
        assert_eq!(keys, vec![text("k\u{0}kind"), text("vs\u{0}note")]);
    }

    #[test]
    fn jsonb_number_leaf_contributes_only_its_key() {
        let json = Value::Jsonb(JsonbValue::Object(vec![(
            "n".to_string(),
            JsonbValue::Number(PgNumeric::from_i128(1)),
        )]));
        let keys = gin_decompose(&json);
        assert_eq!(keys, vec![text("k\u{0}n")]);
    }

    #[test]
    fn containment_query_keys_are_a_subset_of_a_containing_object() {
        // `{"kind":"note","title":"x"}` @> `{"kind":"note"}` is true, so the query's
        // decomposition must be a subset of the container's (the soundness invariant).
        let container = gin_decompose(&Value::Jsonb(JsonbValue::Object(vec![
            ("kind".to_string(), JsonbValue::String("note".to_string())),
            ("title".to_string(), JsonbValue::String("x".to_string())),
        ])));
        let query = gin_decompose(&Value::Jsonb(JsonbValue::Object(vec![(
            "kind".to_string(),
            JsonbValue::String("note".to_string()),
        )])));
        assert!(query.iter().all(|k| container.contains(k)));
    }

    #[test]
    fn scalar_and_null_decompose_to_nothing() {
        assert!(gin_decompose(&Value::Int4(1)).is_empty());
        assert!(gin_decompose(&Value::Null).is_empty());
    }
}
