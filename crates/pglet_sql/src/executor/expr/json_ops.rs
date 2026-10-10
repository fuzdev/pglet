//! The jsonb operators: field access, key existence, path access and containment.

use super::compare::values_equal;
use crate::executor::array_parse::parse_array_literal;
use crate::planner::{containment_legality, json_get_legality, json_operator_legality};
use crate::types::{
    ArrayValue, JsonbValue, TypeOid, Value, opt_type_oid_display_name, parse_jsonb,
};
use crate::{Error, Result};

/// Evaluate JSON(B) field access: `->` (returns JSONB) or `->>` (returns TEXT).
///
/// - `as_text = false`: `json -> key` returns JSONB
/// - `as_text = true`: `json ->> key` returns TEXT
///
/// A NULL operand propagates to NULL. A `text` left operand is parsed as JSON
/// (a pglet leniency — Postgres defines `->` only on json/jsonb); malformed text
/// fails loud via `parse_jsonb` rather than silently yielding NULL.
///
/// Every **other** left operand fails loud (`42883`): Postgres registers only the
/// json/jsonb signatures, so `text_array_col -> 1` is an operator-resolution error
/// there, not the silent NULL pglet used to return.
pub(super) fn eval_json_get(json_val: &Value, key: &Value, as_text: bool) -> Result<Value> {
    if json_val.is_null() || key.is_null() {
        return Ok(Value::Null);
    }
    // The shared operand rule (also consulted by the DDL-time stored-expression
    // check, so the two cannot drift) decides the pair; only the accepted shapes
    // reach the lookup below.
    json_get_legality(json_val.type_oid(), key.type_oid(), as_text)?;
    match json_val {
        Value::Jsonb(j) => Ok(json_get(j, key, as_text)),
        Value::Text(s) => Ok(json_get(&parse_jsonb(s)?, key, as_text)),
        // Unreachable: `json_get_legality` accepts only jsonb/text on the left.
        _ => Err(Error::UndefinedFunction(format!(
            "operator does not exist: {} {} {}",
            opt_type_oid_display_name(json_val.type_oid()),
            if as_text { "->>" } else { "->" },
            opt_type_oid_display_name(key.type_oid())
        ))),
    }
}

/// Look up `key` in a `JsonbValue` — an object key for a text `key`, an array
/// index for an integer — returning the element as JSONB (`->`) or TEXT (`->>`),
/// or NULL if it is absent or the shapes don't match.
///
/// A **shape mismatch is NULL, not an error**, matching Postgres: a text key
/// against an array (`'[1,2]'::jsonb -> 'a'`) and an integer index against an
/// object (`'{"a":1}'::jsonb -> 0`) both yield NULL.
///
/// The **key's type**, by contrast, is gated by the caller's
/// `json_get_legality`: Postgres registers exactly `(jsonb, text)` and
/// `(jsonb, integer)`, so an `int2` widens to `int4` and is accepted, while
/// `int8`, `numeric`, `float8`, `bool`, … resolve to no operator and fail loud
/// (`42883`) before reaching here.
fn json_get(jsonb: &JsonbValue, key: &Value, as_text: bool) -> Value {
    let found = match key {
        Value::Text(k) => json_get_by_key(jsonb, k),
        Value::Int2(i) => json_get_by_index(jsonb, i64::from(*i)),
        Value::Int4(i) => json_get_by_index(jsonb, i64::from(*i)),
        // Unreachable: the key type is gated above.
        _ => None,
    };
    match found {
        Some(v) if as_text => jsonb_to_text(&v),
        Some(v) => Value::Jsonb(v),
        None => Value::Null,
    }
}

/// Get a value from a JSONB object by key.
fn json_get_by_key(json: &JsonbValue, key: &str) -> Option<JsonbValue> {
    match json {
        JsonbValue::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()),
        _ => None,
    }
}

/// Get a value from a JSONB array by index, counting a **negative** index from the
/// end as Postgres does (`'[1,2,3]'::jsonb -> -1` is `3`, `-4` is NULL) — shared
/// with `#>`/`#>>` via [`resolve_array_index`], so the two can't drift.
fn json_get_by_index(json: &JsonbValue, idx: i64) -> Option<JsonbValue> {
    match json {
        JsonbValue::Array(arr) => {
            resolve_array_index(arr.len(), idx).and_then(|i| arr.get(i).cloned())
        }
        _ => None,
    }
}

/// Convert a JSONB value to TEXT (for ->> operator).
///
/// For scalar types (bool, number, string), returns the raw value.
/// For arrays and objects, returns JSON string representation.
fn jsonb_to_text(json: &JsonbValue) -> Value {
    let text = match json {
        JsonbValue::Null => return Value::Null,
        JsonbValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        // Exact Postgres `numeric_out` rendering (never scientific): `1e2` → `100`.
        JsonbValue::Number(n) => n.to_display_string(),
        JsonbValue::String(s) => s.clone(), // Return raw string, not quoted
        JsonbValue::Array(_) | JsonbValue::Object(_) => json.to_json_string(),
    };
    Value::Text(text)
}

/// Evaluate `jsonb ? text` — does the key/element exist? (LENIENT, mirroring
/// [`eval_json_get`].)
///
/// An operand pair outside [`json_operator_legality`]'s accept set is its `42883`,
/// checked before anything else. Past it, a NULL operand → NULL and a `Value::Text` left
/// auto-parses as JSON (malformed text fails loud via `parse_jsonb`). The membership
/// test itself is [`jsonb_exists_key`].
pub(super) fn eval_json_exists(json_val: &Value, key: &Value) -> Result<Value> {
    // The shared operand rule, ahead of the NULL short-circuit's siblings, so the
    // plan-time caller (`planner::coercion`), the DDL-time one
    // (`legality::check_expr_node_operand_types`) and this one cannot drift.
    json_operator_legality(json_val.type_oid(), key.type_oid(), "?", false)?;
    if json_val.is_null() || key.is_null() {
        return Ok(Value::Null);
    }
    let Value::Text(k) = key else {
        return Ok(Value::Null); // refused above unless the rule read it untyped
    };
    let found = match json_val {
        Value::Jsonb(j) => jsonb_exists_key(j, k),
        Value::Text(s) => jsonb_exists_key(&parse_jsonb(s)?, k),
        _ => return Ok(Value::Null), // refused above unless the rule read it untyped
    };
    Ok(Value::Bool(found))
}

/// The `?` membership test: an Object top-level **key**, an Array `String`
/// **element**, or — Postgres's scalar quirk — a top-level scalar `String` equal to
/// `key` directly (`'"a"'::jsonb ? 'a'` → true). A non-string scalar never matches.
fn jsonb_exists_key(json: &JsonbValue, key: &str) -> bool {
    match json {
        JsonbValue::Object(pairs) => pairs.iter().any(|(k, _)| k == key),
        JsonbValue::Array(elems) => elems
            .iter()
            .any(|e| matches!(e, JsonbValue::String(s) if s == key)),
        JsonbValue::String(s) => s == key,
        _ => false,
    }
}

/// Evaluate `jsonb ?| text[]` (`all = false`, any) / `jsonb ?& text[]` (`all =
/// true`, all). (LENIENT, mirroring [`eval_json_get`].)
///
/// An operand pair outside [`json_operator_legality`]'s accept set is its `42883`,
/// checked before anything else. Past it, a NULL operand → NULL, a `Value::Text` left
/// auto-parses as JSON, and a `Value::Text` right parses as an array literal
/// ([`as_text_array`]). The iteration (skipping NULL array elements) is
/// [`jsonb_exists_any_all`].
pub(super) fn eval_json_exists_any_all(json_val: &Value, keys: &Value, all: bool) -> Result<Value> {
    json_operator_legality(
        json_val.type_oid(),
        keys.type_oid(),
        if all { "?&" } else { "?|" },
        true,
    )?;
    if json_val.is_null() || keys.is_null() {
        return Ok(Value::Null);
    }
    let Some(arr) = as_text_array(keys)? else {
        return Ok(Value::Null); // refused above unless the rule read it untyped
    };
    let found = match json_val {
        Value::Jsonb(j) => jsonb_exists_any_all(j, &arr, all),
        Value::Text(s) => jsonb_exists_any_all(&parse_jsonb(s)?, &arr, all),
        _ => return Ok(Value::Null), // refused above unless the rule read it untyped
    };
    Ok(Value::Bool(found))
}

/// `?|`/`?&` over a `text[]`: iterate the elements, **skipping** a non-`Value::Text`
/// (i.e. NULL) element rather than propagating it. `?&` (`all`) requires every
/// present key to exist (empty array → true); `?|` needs any present key to exist
/// (empty array → false).
fn jsonb_exists_any_all(json: &JsonbValue, keys: &ArrayValue, all: bool) -> bool {
    for elem in &keys.elements {
        let Value::Text(k) = elem else {
            continue; // a NULL (non-text) array element is skipped, not propagated
        };
        let found = jsonb_exists_key(json, k);
        if all && !found {
            return false;
        }
        if !all && found {
            return true;
        }
    }
    all
}

/// Evaluate `jsonb #> text[]` (`as_text = false`, → jsonb) / `jsonb #>> text[]`
/// (`as_text = true`, → text) — extract at a path. (LENIENT, mirroring
/// [`eval_json_get`].)
///
/// An operand pair outside [`json_operator_legality`]'s accept set is its `42883`,
/// checked before anything else. Past it, a NULL operand → NULL, a `Value::Text` left
/// auto-parses as JSON, and a `Value::Text` right parses as an array literal
/// ([`as_text_array`]). The traversal is [`json_path_get`].
pub(super) fn eval_json_path_get(json_val: &Value, path: &Value, as_text: bool) -> Result<Value> {
    json_operator_legality(
        json_val.type_oid(),
        path.type_oid(),
        if as_text { "#>>" } else { "#>" },
        true,
    )?;
    if json_val.is_null() || path.is_null() {
        return Ok(Value::Null);
    }
    let Some(arr) = as_text_array(path)? else {
        return Ok(Value::Null); // refused above unless the rule read it untyped
    };
    match json_val {
        Value::Jsonb(j) => Ok(json_path_get(j, &arr, as_text)),
        Value::Text(s) => Ok(json_path_get(&parse_jsonb(s)?, &arr, as_text)),
        _ => Ok(Value::Null), // refused above unless the rule read it untyped
    }
}

/// Resolve the array right-operand of `?|`/`?&`/`#>`/`#>>` to a `text[]`
/// [`ArrayValue`]. A real `Value::Array` passes through (cloned — these paths are
/// tiny); a `Value::Text` auto-parses as a Postgres array literal (`'{a,b}'`),
/// mirroring Postgres's implicit string→`text[]` cast (and [`eval_json_get`]'s
/// text→JSON leniency) — a malformed literal fails loud (`22P02`), matching
/// Postgres, rather than silently yielding NULL. Any other shape → `None`: the
/// callers' [`json_operator_legality`] refuses every such typed shape first, so their
/// `None` arm is a fallback for a value the rule reads as untyped.
fn as_text_array(v: &Value) -> Result<Option<ArrayValue>> {
    match v {
        Value::Array(arr) => Ok(Some(arr.clone())),
        Value::Text(s) => parse_array_literal(s, TypeOid::Text).map(Some),
        _ => Ok(None),
    }
}

/// Walk `path` (a `text[]`) into `json` for `#>` / `#>>`.
///
/// **Any NULL path element → the whole result is `Value::Null`** (checked before
/// traversal). An **empty path** → the whole document (`#>` as `Value::Jsonb`, `#>>`
/// as its full JSON text). Each step: an Object indexes by the key text (miss →
/// NULL); an Array parses the step as a **strict whole-string i64** (non-integer →
/// NULL) and resolves it negative-from-end + bounds via [`resolve_array_index`]
/// (out of range → NULL); a scalar (or any other) mid-path → NULL — there is **no**
/// `?`-style scalar quirk here. The final value wraps as `#>` `Value::Jsonb` or
/// `#>>` [`jsonb_to_text`] (which maps a JSON `null` → SQL NULL, whereas `#>` keeps
/// it as jsonb `null`).
fn json_path_get(json: &JsonbValue, path: &ArrayValue, as_text: bool) -> Value {
    // A NULL anywhere in the path makes the whole extraction NULL (checked first,
    // before any traversal, matching Postgres).
    if path.elements.iter().any(Value::is_null) {
        return Value::Null;
    }
    let mut current = json;
    for elem in &path.elements {
        let Value::Text(key) = elem else {
            // A non-text, non-null path element can't index anything → NULL.
            return Value::Null;
        };
        let next = match current {
            JsonbValue::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            JsonbValue::Array(arr) => key
                .parse::<i64>()
                .ok()
                .and_then(|i| resolve_array_index(arr.len(), i))
                .and_then(|idx| arr.get(idx)),
            // A scalar mid-path → NULL (no `?`-style scalar-as-1-elem-array quirk).
            _ => None,
        };
        match next {
            Some(v) => current = v,
            None => return Value::Null,
        }
    }
    if as_text {
        jsonb_to_text(current)
    } else {
        Value::Jsonb(current.clone())
    }
}

/// Resolve a possibly-negative array index against `len`: a non-negative `i` is
/// itself; a negative `i` counts from the end (`-1` = last element). Returns `None`
/// when the resolved position is out of range.
///
/// Shared by `#>`/`#>>` (via [`json_path_get`]) and `->`/`->>` (via
/// [`json_get_by_index`]), so the two index paths can't drift.
pub(super) fn resolve_array_index(len: usize, i: i64) -> Option<usize> {
    let idx = if i < 0 {
        i64::try_from(len).ok()?.checked_add(i)?
    } else {
        i
    };
    usize::try_from(idx).ok().filter(|&u| u < len)
}

/// JSONB `@>` containment: does `container` contain `contained`?
///
/// A port of Postgres's `JsonbDeepContains` (`jsonb_util.c`). The rules, and the
/// one place the **top level** differs from every nested level:
///
/// - **Object ⊇ object** — every `(k, v)` of the rhs must appear in the lhs under
///   the same key, with the value *contained* (scalars by equality, containers
///   recursively). A key the lhs lacks fails. Structure must line up: an rhs
///   container value only matches an lhs container of the **same kind**.
/// - **Array ⊇ array** — every rhs element must be matched by *some* lhs element:
///   a scalar element by an equal lhs **scalar** element, a container element by
///   an lhs **container** element that contains it. Multiplicity is ignored
///   (`'[1]' @> '[1,1]'` is true) and element order is irrelevant.
/// - **Array ⊇ scalar** (top level only) — Postgres's documented exception, "an
///   array may contain a primitive value": `'["a","b"]' @> '"a"'`. It exists
///   because a top-level scalar is stored as a one-element *raw scalar*
///   pseudo-array, so this is really the array rule. The exception does **not**
///   apply nested — `'{"a":[1,2]}' @> '{"a":1}'` is false, because an object's
///   value comparison demands matching types.
/// - **Scalar ⊇ scalar** — equality (two raw scalars). A scalar never contains an
///   array or object (`'"a"' @> '["a"]'` is false), and object/array kinds never
///   cross.
///
/// This replaced an object-only implementation whose fallback was `a == b` for
/// every other pair — a **plausible wrong boolean** (not a loud failure) on array
/// subset, nested arrays, and the scalar exception, which is the whole workhorse
/// half of the operator.
///
/// The GIN path is unaffected in soundness: `gin_decompose` collects each key and
/// scalar leaf regardless of nesting, so `container @> contained` still implies
/// `decompose(contained) ⊆ decompose(container)` under these wider rules, and the
/// index remains a candidate filter over which this function is the exact recheck.
pub(super) fn jsonb_contains(container: &JsonbValue, contained: &JsonbValue) -> bool {
    match (container, contained) {
        // The top-level-only raw-scalar exception; every other pair is the same
        // rule a nested position uses.
        (JsonbValue::Array(items), scalar) if scalar.is_scalar() => {
            items.iter().any(|item| item == scalar)
        }
        _ => jsonb_contains_nested(container, contained),
    }
}

/// [`jsonb_contains`] at a nested position — no raw-scalar exception.
///
/// Split out because Postgres's recursion re-enters `JsonbDeepContains` at a point
/// where both operands are known containers, so the "an array may contain a
/// primitive" rule is unreachable below the top level.
pub(super) fn jsonb_contains_nested(container: &JsonbValue, contained: &JsonbValue) -> bool {
    match (container, contained) {
        (JsonbValue::Object(outer), JsonbValue::Object(inner)) => {
            // Cheap reject Postgres also makes — sound only because object keys are
            // deduplicated (`JsonbValue::canonical`), so pair counts are key counts.
            inner.len() <= outer.len()
                && inner.iter().all(|(k, v)| {
                    outer
                        .iter()
                        .find(|(ok, _)| ok == k)
                        .is_some_and(|(_, ov)| jsonb_contains_nested(ov, v))
                })
        }
        (JsonbValue::Array(outer), JsonbValue::Array(inner)) => inner.iter().all(|rhs| {
            if rhs.is_scalar() {
                // Postgres looks a scalar element up among the lhs *scalars*
                // (`findJsonbValueFromContainer` compares scalar values), so a
                // container element can never satisfy one.
                outer.iter().any(|lhs| lhs == rhs)
            } else {
                outer
                    .iter()
                    .any(|lhs| !lhs.is_scalar() && jsonb_contains_nested(lhs, rhs))
            }
        }),
        // Two scalars: equality, which also enforces Postgres's type check (a
        // number never equals the string spelling it). Numbers compare exactly via
        // `PgNumeric`, so `1.0` contains `1` as it does in Postgres.
        (a, b) if a.is_scalar() && b.is_scalar() => a == b,
        // Any remaining pair mixes kinds (object vs array, container vs scalar):
        // never contained.
        _ => false,
    }
}

/// Containment: true if `container` holds `contained`.
///
/// Backs `@>` (`container @> contained`) and, with operands swapped, `<@` — so
/// `symbol` (`@>` / `<@`) is the only record of which one the query wrote, and the
/// operand-type gate takes its operands **unswapped** to keep the error message
/// faithful. This is both the standalone (scan-based) evaluator AND the **recheck** a
/// `Plan::GinScan` applies to its candidate superset (a GIN index narrows which rows
/// reach here, but the exact answer is still this function). NULL operand → NULL.
/// Arrays use element subset; jsonb delegates to [`jsonb_contains`], the full
/// `JsonbDeepContains` port.
pub(super) fn containment(container: &Value, contained: &Value, symbol: &str) -> Result<Value> {
    let (left, right) = if symbol == "@>" {
        (container, contained)
    } else {
        (contained, container)
    };
    containment_legality(left.type_oid(), right.type_oid(), symbol)?;
    if container.is_null() || contained.is_null() {
        return Ok(Value::Null);
    }
    match (container, contained) {
        (Value::Array(a), Value::Array(b)) => {
            // O(n*m) nested scan. For a GIN-indexed column this runs only on the
            // candidate rows the `gin_lookup` posting-list intersection returns, not
            // the whole table; for an unindexed column it scans every row.
            //
            // NULL elements match Postgres's `array_contain_compare` exactly, and
            // fall out of `values_equal` rather than needing a special case: it
            // returns `None` for any NULL operand, so a NULL *in `contained`* fails
            // its `any` and sinks the whole `all` (PG breaks with `result = false`),
            // while a NULL *in `container`* simply never satisfies an element (PG
            // `continue`s past it). Both directions are fixture-locked.
            let held = b
                .elements
                .iter()
                .all(|x| a.elements.iter().any(|y| values_equal(x, y) == Some(true)));
            Ok(Value::Bool(held))
        }
        // Full jsonb containment — object key/value, array subset, nested containers,
        // and the top-level raw-scalar exception (see `jsonb_contains`). `<@` reaches
        // here with operands already swapped.
        (Value::Jsonb(a), Value::Jsonb(b)) => Ok(Value::Bool(jsonb_contains(a, b))),
        // `containment_legality` accepts only the jsonb pair and the same-typed
        // array pair (and rejects everything else loud), so the pair is one of those
        // by construction here; re-raise its own verdict rather than invent a second.
        _ => Err(Error::UndefinedFunction(format!(
            "operator does not exist: {} {symbol} {}",
            opt_type_oid_display_name(left.type_oid()),
            opt_type_oid_display_name(right.type_oid())
        ))),
    }
}
