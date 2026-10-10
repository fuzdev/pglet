//! Boolean contexts, value comparison and equality, and the `LIKE` / regex family.

use super::arithmetic::{as_exact_numeric, value_as_i64, value_to_f64_opt};
use super::eval::option_to_value;
use crate::planner::{
    BoolContext, ResolvedType, like_legality, regex_legality, starts_with_legality,
};
use crate::types::{JsonbValue, Value, opt_type_oid_display_name};
use crate::{Error, Result};
use std::cmp::Ordering;

/// The **one** eval-time resolver for every boolean context.
///
/// The three-valued truth of `val` at a position that requires `boolean` (`None` =
/// SQL NULL), or the error 18.6 raises there.
///
/// The eval-time half of the gate whose plan-time half is
/// `planner::legality`'s [`boolean_context_legality`](crate::planner::boolean_context_legality);
/// both are needed because the plan-time reader answers `None` wherever it cannot be
/// certain — an unbound `$N`, a scalar subquery a scope could not type — and defers to
/// this. Every site that decides an expression's truth reads it, so no position can
/// substitute a truth value for a non-boolean: not FALSE, not TRUE, not a skipped
/// write.
///
/// A `Value::Text` reaching here is Postgres's `unknown` resolved at this position:
/// the plan-time gate lets a bare string literal through precisely so it can be
/// *converted*, and the conversion's own `22P02` is what 18.6 answers
/// (`WHERE 'abc'`), while `WHERE 't'` is a row. Every other type is `42804`.
///
/// # Errors
///
/// [`Error::Type`] (`42804`) for a non-boolean, non-text operand;
/// [`Error::InvalidInputSyntax`] (`22P02`) for text that is not a boolean spelling.
pub fn boolean_context_operand(val: &Value, ctx: BoolContext) -> Result<Option<bool>> {
    match val {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(*b)),
        Value::Text(s) => super::super::coerce::text_to_bool(s).map(Some),
        other => Err(Error::Type(format!(
            "argument of {} must be type boolean, not type {}",
            ctx.sql(),
            opt_type_oid_display_name(other.type_oid())
        ))),
    }
}

/// Whether a predicate **holds** — [`boolean_context_operand`] narrowed to the
/// two-valued question a filter asks, with SQL NULL answering `false`.
///
/// The shape every truth site takes, so "the predicate did not hold" and "the
/// predicate was not a boolean" cannot be spelled by the same `matches!`.
///
/// # Errors
///
/// [`boolean_context_operand`]'s.
pub fn predicate_holds(val: &Value, ctx: BoolContext) -> Result<bool> {
    Ok(boolean_context_operand(val, ctx)? == Some(true))
}

/// `NOT x` — the one context whose result is a value rather than a branch.
///
/// # Errors
///
/// [`boolean_context_operand`]'s, under `NOT`.
pub fn eval_not(val: &Value) -> Result<Value> {
    Ok(option_to_value(
        boolean_context_operand(val, BoolContext::Not)?.map(|b| !b),
    ))
}

/// Total order over `f64` matching Postgres's `float8_cmp_internal`: `NaN` equals
/// itself and sorts **greater** than every finite value (and ±∞). Unlike IEEE
/// `partial_cmp` — which returns `None` for any `NaN` operand, collapsing to `Equal`
/// at the callsites and silently treating `NaN` as equal to a finite value — this is
/// a real total order. It is the shared float comparator for **both** [`values_compare`]
/// (ordering, `MIN`/`MAX`, `DISTINCT`, window partitioning) and [`values_equal`] (`=`,
/// join `ON`, `= ANY`, FK/conflict matchers), across every float width (a `float4`
/// promotes to `f64` exactly), so all of them agree `NaN = NaN` and `NaN > x`.
fn cmp_f64_total(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// A `date` promoted to a timestamp at midnight UTC (`date2timestamp`), an infinity to
/// the same infinity.
pub(super) fn date_to_timestamp(d: crate::types::PgDate) -> crate::types::PgTimestamp {
    d.map(|d| d.midnight().assume_utc())
}

/// Compare two values for equality.
/// Returns `None` if either value is NULL (SQL NULL propagation).
/// Float `NaN` equals itself (Postgres's total order), unlike IEEE `==`.
#[allow(clippy::match_same_arms)] // per-type arms kept separate; several share `l == r`
pub fn values_equal(left: &Value, right: &Value) -> Option<bool> {
    if left.is_null() || right.is_null() {
        return None;
    }
    // NUMERIC participates exactly against numeric/int; a float operand compares
    // via the numeric's f64 value (Postgres casts numeric→float8 there). Handled
    // before the per-type arms so numeric is never compared as a float by accident.
    if matches!(left, Value::Numeric(_)) || matches!(right, Value::Numeric(_)) {
        if let (Some(a), Some(b)) = (as_exact_numeric(left), as_exact_numeric(right)) {
            return Some(a == b);
        }
        let (l, r) = (value_to_f64_opt(left)?, value_to_f64_opt(right)?);
        return Some(cmp_f64_total(l, r) == Ordering::Equal);
    }
    // Any float operand → compare via `f64` with the NaN-aware total order (a `float4`
    // promotes to `f64` exactly; `NaN = NaN` is true). This covers same-width AND
    // cross-width float equality and int-vs-float, matching Postgres's mixed float
    // operators. Placed after the numeric check (numeric stays exact) and before the
    // integer arms, so pure int-vs-int never routes through `f64`.
    if matches!(left, Value::Float4(_) | Value::Float8(_))
        || matches!(right, Value::Float4(_) | Value::Float8(_))
    {
        let (l, r) = (value_to_f64_opt(left)?, value_to_f64_opt(right)?);
        return Some(cmp_f64_total(l, r) == Ordering::Equal);
    }
    // The integer widths cross-compare in **every** pairing (Postgres registers an
    // operator for each of the nine combinations), so widen to `i64` once instead of
    // spelling out arms: the explicit set covered int4↔int8 but not int2↔int4 or
    // int2↔int8, which fell through to the permissive `false` and made
    // `int2_col = int4_col` silently unequal. Reached only for two pure integers —
    // the numeric and float branches above claim any mixed pair.
    if let (Some(l), Some(r)) = (value_as_i64(left), value_as_i64(right)) {
        return Some(l == r);
    }
    Some(match (left, right) {
        (Value::Bool(l), Value::Bool(r)) => l == r,
        (Value::Text(l), Value::Text(r)) => l == r,
        // UUID / bytea / timestamp / jsonb scalar equality. UUID equality in
        // particular is load-bearing: the spine keys nearly every lookup on
        // `WHERE id = $1` (and `id = ANY($1)`) over uuid columns.
        (Value::Uuid(l), Value::Uuid(r)) => l == r,
        (Value::Bytea(l), Value::Bytea(r)) => l == r,
        (Value::Timestamp(l), Value::Timestamp(r)) => l == r,
        (Value::Date(l), Value::Date(r)) => l == r,
        (Value::Time(l), Value::Time(r)) => l == r,
        // Mixed DATE/TIMESTAMP: promote the date to midnight UTC and compare as
        // timestamps (Postgres `date2timestamp`), reusing the `date::timestamp` cast
        // logic (`coerce.rs`) so the implicit equality agrees with the explicit cast at
        // the midnight boundary. Two typed columns / an explicit `::timestamp` still
        // reach here as a mixed pair after plan-time literal coercion.
        (Value::Date(l), Value::Timestamp(r)) => date_to_timestamp(*l) == *r,
        (Value::Timestamp(l), Value::Date(r)) => *l == date_to_timestamp(*r),
        (Value::Jsonb(l), Value::Jsonb(r)) => l == r,
        // Element-wise array equality, two NULL elements equal and a NULL against a
        // non-NULL unequal — 18.6's `array_eq` (`arrayfuncs.c`: "We consider two NULLs
        // equal; NULL and not-NULL are unequal").
        (Value::Array(l), Value::Array(r)) => {
            l.elements.len() == r.elements.len()
                && l.elements.iter().zip(r.elements.iter()).all(|(a, b)| {
                    match (a.is_null(), b.is_null()) {
                        (true, true) => true,
                        (false, false) => values_equal(a, b) == Some(true),
                        _ => false,
                    }
                })
        }
        // Intervals compare on the Postgres estimate (`30 days` == `1 mon`); the
        // custom `PgInterval` `PartialEq` implements exactly that.
        (Value::Interval(l), Value::Interval(r)) => l == r,
        // Mixed TIME/INTERVAL: Postgres implicitly casts `time → interval`
        // (`pg_cast.dat`, `castcontext => 'i'`), i.e. an interval whose whole span is
        // the time's microsecond-of-day count — which is exactly its estimate. So
        // `TIME '01:00' = INTERVAL '1 hour'`, and `TIME '24:00' = INTERVAL '1 day'`
        // (the estimate collapses a day to 24 h).
        (Value::Time(l), Value::Interval(r)) => i128::from(*l) == r.estimate(),
        (Value::Interval(l), Value::Time(r)) => l.estimate() == i128::from(*r),
        _ => false,
    })
}

/// Compare two values, yielding a total order over each comparable type.
///
/// Returns `None` if either value is NULL (SQL NULL propagation). This backs both
/// ordering (`<`/`>`, `ORDER BY`, `MIN`/`MAX`) **and** the equality used by callers
/// that test `== Some(Ordering::Equal)` — scan-based `DISTINCT`, window
/// partitioning, and the scan-fallback `ON CONFLICT`/FK matchers. (The *common* FK
/// path is a covering-index lookup in `constraints.rs` that reads the storage index
/// directly and never routes through here, so a cross-type FK is not made to agree
/// by these arms — only the scan fallback is.) Every scalar variant therefore gets a
/// real arm: a missing one would collapse to `Equal` and silently treat two distinct
/// UUID/bytea/timestamp/jsonb/bool values as equal — a live conflict-matching bug.
/// UUID/bytea order lexicographically by bytes; jsonb orders by Postgres's B-tree
/// order (see `jsonb_compare`); a genuinely mixed-type pair (which well-typed SQL
/// never produces) falls back to a deterministic text comparison.
pub fn values_compare(left: &Value, right: &Value) -> Option<Ordering> {
    values_compare_ordered(left, right, None)
}

/// Where a label sits in its enum's declared member list, as a key that is a **total
/// order over every label**, member or not.
///
/// `Ok(position)` for a member and `Err(label)` for one the type does not hold, which
/// Rust's derived `Ord` on `Result` orders as *every member before every non-member,
/// non-members among themselves by their bytes*. Three properties the ordering sites
/// need and no interleaving rule has: it is **transitive** (`sort_by`'s requirement —
/// "members by position, the rest by bytes, mixed together" is not), it **cannot brick
/// a readable table** (a stored non-member still compares, as this repo's dump emitter
/// still exports a sequence whose owner it cannot resolve), and it keeps
/// `Equal ⟺ the labels are byte-equal`, which the callers that ask this comparator an
/// *equality* question (window peers, aggregate `DISTINCT`) depend on. 18.6 offers no
/// answer to copy here — its enum datum *is* a validated `pg_enum.oid`, so a label it
/// does not hold is an internal `elog(ERROR)` (`typcache.c`'s `find_enumitem`) rather
/// than a modelled SQLSTATE. The one route known to put a non-member into a stored row
/// is `DROP TYPE` + re-`CREATE TYPE` with other members under a column (`EDGES.md` row
/// 261, which 18.6 refuses `2BP01`); row 105 records the routes since closed.
fn enum_rank<'l>(order: &[String], label: &'l str) -> std::result::Result<usize, &'l str> {
    order.iter().position(|m| m == label).ok_or(label)
}

/// [`values_compare`] over a pair whose **enum member order** the caller resolved.
///
/// `Some(order)` with two `Value::Text` operands compares them by their position in
/// `order` ([`enum_rank`]); every other shape is byte-identical to
/// [`values_compare`]. The enum arm runs before the numeric/float short-circuits below
/// for legibility only — neither claims a `Text` pair.
#[allow(clippy::match_same_arms)] // per-type arms kept separate; several share `l.cmp(r)`
pub fn values_compare_ordered(
    left: &Value,
    right: &Value,
    enum_order: Option<&[String]>,
) -> Option<Ordering> {
    if left.is_null() || right.is_null() {
        return None;
    }

    // An enum orders by its members' positions, not its labels' bytes. Both operands
    // take the same list: the one the caller resolved from the enum-typed side (see
    // `executor::expr`'s `expr_enum_order`), which is how a bare `'sad'` compared
    // against an enum column is ranked as the member 18.6 resolves it to.
    if let (Some(order), Value::Text(l), Value::Text(r)) = (enum_order, left, right) {
        return Some(enum_rank(order, l).cmp(&enum_rank(order, r)));
    }

    // NUMERIC first, exactly (never via `f64`, which loses precision): numeric/int
    // promote to `PgNumeric`; a float operand compares via the numeric's f64 value
    // (Postgres casts numeric→float8 for that mixed operator).
    if matches!(left, Value::Numeric(_)) || matches!(right, Value::Numeric(_)) {
        if let (Some(a), Some(b)) = (as_exact_numeric(left), as_exact_numeric(right)) {
            return Some(a.cmp_value(&b));
        }
        let (lf, rf) = (value_to_f64_opt(left)?, value_to_f64_opt(right)?);
        return Some(cmp_f64_total(lf, rf));
    }

    // Two integers compare as integers, in every width pairing: through `f64` two
    // `int8`s past 2^53 that differ by one compare equal, and `ORDER BY`, `MIN`,
    // `DISTINCT` and a `>` filter all answered by that.
    if let (Some(l), Some(r)) = (value_as_i64(left), value_as_i64(right)) {
        return Some(l.cmp(&r));
    }

    // A float operand (float4/8, cross-width, or against an integer, which 18.6 casts
    // to `float8`). Uses the NaN-aware total order so `NaN` sorts greatest and equals
    // itself.
    if let (Some(lf), Some(rf)) = (value_to_f64_opt(left), value_to_f64_opt(right)) {
        return Some(cmp_f64_total(lf, rf));
    }

    Some(match (left, right) {
        (Value::Bool(l), Value::Bool(r)) => l.cmp(r),
        (Value::Text(l), Value::Text(r)) => l.cmp(r),
        (Value::Uuid(l), Value::Uuid(r)) => l.cmp(r),
        (Value::Bytea(l), Value::Bytea(r)) => l.cmp(r),
        (Value::Timestamp(l), Value::Timestamp(r)) => l.cmp(r),
        (Value::Date(l), Value::Date(r)) => l.cmp(r),
        // Mixed DATE/TIMESTAMP: promote the date to midnight UTC and compare as
        // timestamps (Postgres `date2timestamp`), matching the `date::timestamp` cast
        // so the implicit ordering agrees with the explicit one across the midnight
        // boundary (without this the pair fell through to the ISO-text fallback below,
        // right only off-boundary by prefix luck).
        (Value::Date(l), Value::Timestamp(r)) => date_to_timestamp(*l).cmp(r),
        (Value::Timestamp(l), Value::Date(r)) => l.cmp(&date_to_timestamp(*r)),
        // A `time` compares as its microsecond-of-day count (order-preserving).
        (Value::Time(l), Value::Time(r)) => l.cmp(r),
        // Intervals order on the Postgres estimate (`30 days` == `1 mon`).
        (Value::Interval(l), Value::Interval(r)) => l.estimate().cmp(&r.estimate()),
        // Mixed TIME/INTERVAL: the implicit `time → interval` cast makes the time's
        // microsecond-of-day count its estimate (see `values_equal`).
        (Value::Time(l), Value::Interval(r)) => i128::from(*l).cmp(&r.estimate()),
        (Value::Interval(l), Value::Time(r)) => l.estimate().cmp(&i128::from(*r)),
        // JSONB orders by Postgres's B-tree order (`compareJsonbContainers`), not raw
        // JSON text: type rank first (null < string < number < bool < array < object),
        // then content — arrays by length-then-elementwise, objects by
        // pair-count-then-pairwise. See `jsonb_compare`. Equal values fast-path.
        (Value::Jsonb(l), Value::Jsonb(r)) => {
            if l == r {
                Ordering::Equal
            } else {
                jsonb_compare(l, r)
            }
        }
        (Value::Array(l), Value::Array(r)) => {
            if values_equal(left, right) == Some(true) {
                Ordering::Equal
            } else {
                // Element-wise order; a NULL element sorts GREATER than any non-NULL
                // (Postgres `array_cmp` in `arrayfuncs.c`: `ARRAY[1,NULL] > ARRAY[1,2]`).
                for (a, b) in l.elements.iter().zip(r.elements.iter()) {
                    match (a.is_null(), b.is_null()) {
                        (true, true) => {}
                        (true, false) => return Some(Ordering::Greater),
                        (false, true) => return Some(Ordering::Less),
                        // The **same** member list at every element: an array's enum
                        // is its element type's (`expr_enum_with`'s `Array` arm), so
                        // `ARRAY[m] < ARRAY['ok'::mood]` orders by member position
                        // rather than by the label bytes. Threading `enum_order` here is
                        // what makes that arm's answer readable at all.
                        (false, false) => match values_compare_ordered(a, b, enum_order) {
                            Some(Ordering::Equal) | None => {}
                            other => return other,
                        },
                    }
                }
                l.elements.len().cmp(&r.elements.len())
            }
        }
        // Mixed/unhandled type pair: well-typed SQL never reaches here. Compare by
        // text rendering so the result stays deterministic and equality-honest
        // rather than collapsing distinct values to `Equal`.
        _ => render_for_compare(left).cmp(&render_for_compare(right)),
    })
}

/// The Postgres jsonb B-tree type rank: `null < string < number < bool < array <
/// object` (the relative order of the `jbvType` enum that `compareJsonbContainers`
/// reads for its cross-type "type-defined order"). Only the ordering matters, not
/// the absolute values.
const fn jsonb_type_rank(v: &JsonbValue) -> u8 {
    match v {
        JsonbValue::Null => 0,
        JsonbValue::String(_) => 1,
        JsonbValue::Number(_) => 2,
        JsonbValue::Bool(_) => 3,
        JsonbValue::Array(_) => 4,
        JsonbValue::Object(_) => 5,
    }
}

/// Whether a JSONB value is a scalar (non-container) — used only for the top-level
/// empty-array anomaly in [`jsonb_compare`].
const fn jsonb_is_scalar(v: &JsonbValue) -> bool {
    matches!(
        v,
        JsonbValue::Null | JsonbValue::Bool(_) | JsonbValue::Number(_) | JsonbValue::String(_)
    )
}

/// Compare two JSONB values in Postgres's B-tree order — the top-level entry point
/// backing `ORDER BY` / `DISTINCT` / `GROUP BY` over a `jsonb` column, and the order a
/// `jsonb` index key is the byte image of (`pglet_core`'s `jsonb_order_key`; a change
/// to one is a change to both, held by `tests/index_key_order.rs`). Faithful to
/// `compareJsonbContainers` in Postgres's `jsonb_util.c`, including the top-level
/// empty-array anomaly: a whole-document scalar is stored as a one-element "raw
/// scalar" array, so it sorts *greater* than an empty top-level array
/// (`'1'::jsonb > '[]'::jsonb`) even though its type rank is lower. The anomaly is
/// top-level only — nested values follow pure type rank via [`jsonb_compare_nested`].
fn jsonb_compare(a: &JsonbValue, b: &JsonbValue) -> Ordering {
    match (a, b) {
        // The one place type rank does not hold: an empty top-level array sorts
        // less than any top-level scalar.
        (JsonbValue::Array(e), s) if e.is_empty() && jsonb_is_scalar(s) => Ordering::Less,
        (s, JsonbValue::Array(e)) if e.is_empty() && jsonb_is_scalar(s) => Ordering::Greater,
        _ => jsonb_compare_nested(a, b),
    }
}

/// Recursive worker for [`jsonb_compare`]: pure Postgres type-rank order with no
/// top-level anomaly (nested containers never wrap scalars). Same-type values
/// compare by content — strings lexically, numbers by value, arrays by
/// length-then-elementwise, objects by pair-count-then-pairwise (key then value, in
/// stored order). Walking objects in stored order is faithful because *stored order
/// is canonical*: every jsonb value is built through [`JsonbValue::canonical`],
/// which sorts object keys Postgres's way, so both engines walk the same sequence.
/// Note the two orderings are deliberately different — keys are **stored** by
/// length-then-bytes but **compared** lexically here, matching Postgres, where the
/// storage order comes from `lengthCompareJsonbString` and the pairwise comparison
/// from `varstr_cmp`.
fn jsonb_compare_nested(a: &JsonbValue, b: &JsonbValue) -> Ordering {
    match (a, b) {
        (JsonbValue::Null, JsonbValue::Null) => Ordering::Equal,
        (JsonbValue::Bool(x), JsonbValue::Bool(y)) => x.cmp(y),
        // pglet stores jsonb numbers as exact `PgNumeric`, so compare with Postgres's
        // exact `numeric_cmp` (`cmp_value`) — no `f64` round-trip. `1e2` and `100`
        // fold equal; a 30-digit integer compares exactly.
        (JsonbValue::Number(x), JsonbValue::Number(y)) => x.cmp_value(y),
        (JsonbValue::String(x), JsonbValue::String(y)) => x.cmp(y),
        (JsonbValue::Array(xs), JsonbValue::Array(ys)) => {
            // Length first, then element-wise (Postgres compares `nElems` at the
            // container-start token, before any element) — so `[1] < [0,0]`.
            if xs.len() != ys.len() {
                return xs.len().cmp(&ys.len());
            }
            for (x, y) in xs.iter().zip(ys) {
                let o = jsonb_compare_nested(x, y);
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        }
        (JsonbValue::Object(xs), JsonbValue::Object(ys)) => {
            // Pair count first, then pairwise key (lexical) then value — so
            // `{"a":1} < {"a":1,"b":1}`.
            if xs.len() != ys.len() {
                return xs.len().cmp(&ys.len());
            }
            for ((kx, vx), (ky, vy)) in xs.iter().zip(ys) {
                let ko = kx.cmp(ky);
                if ko != Ordering::Equal {
                    return ko;
                }
                let vo = jsonb_compare_nested(vx, vy);
                if vo != Ordering::Equal {
                    return vo;
                }
            }
            Ordering::Equal
        }
        // Different types: Postgres's "type-defined order".
        _ => jsonb_type_rank(a).cmp(&jsonb_type_rank(b)),
    }
}

/// A deterministic text rendering of a value for the mixed-type compare fallback.
fn render_for_compare(v: &Value) -> String {
    crate::types::value_to_text(v).unwrap_or_default()
}

/// Both operands of a `LIKE`-family operator as `&str`, or a loud error naming
/// `symbol` (`~~` / `!~~` / `~~*` / `!~~*`).
///
/// The verdict itself comes from the shared [`like_legality`] rule — the same one
/// the DDL-time stored-expression check consults — so the two cannot drift; this
/// only turns the accepted pair into the `&str`s the matcher needs.
fn like_operands<'a>(
    value: &'a Value,
    pattern: &'a Value,
    symbol: &str,
) -> Result<(&'a str, &'a str)> {
    like_legality(value.type_oid(), pattern.type_oid(), symbol)?;
    match (value, pattern) {
        (Value::Text(v), Value::Text(p)) => Ok((v.as_str(), p.as_str())),
        // `like_legality` accepts only the string pair (and rejects everything
        // else loud), so the pair is text by construction here.
        _ => Err(Error::UndefinedFunction(format!(
            "operator does not exist: {} {symbol} {}",
            opt_type_oid_display_name(value.type_oid()),
            opt_type_oid_display_name(pattern.type_oid())
        ))),
    }
}

/// Evaluate LIKE pattern matching.
/// Returns `None` if either value is NULL (SQL NULL propagation).
/// Pattern uses: `%` = any sequence, `_` = single char.
///
/// A non-text operand fails loud (`42883`) via [`like_operands`]; `symbol` names
/// the Postgres operator for the message (`~~` for `LIKE`, `!~~` for `NOT LIKE`).
///
/// # Errors
///
/// Returns [`Error::UndefinedFunction`] when either operand is not text.
pub fn eval_like(value: &Value, pattern: &Value, symbol: &str) -> Result<Option<bool>> {
    if value.is_null() || pattern.is_null() {
        return Ok(None);
    }
    let (text, pat) = like_operands(value, pattern, symbol)?;
    Ok(Some(like_match(text, pat)))
}

/// Evaluate ILIKE — case-insensitive `LIKE`, folding both operands through the
/// same Unicode lowercasing `lower()` uses (so `x ILIKE p` matches
/// `lower(x) LIKE lower(p)`, Postgres's definition). Folding the pattern
/// leaves `%` / `_` / `\` untouched, so wildcard and escape semantics are
/// identical to `LIKE`.
///
/// A non-text operand fails loud (`42883`) via [`like_operands`]; `symbol` names
/// the Postgres operator (`~~*` for `ILIKE`, `!~~*` for `NOT ILIKE`).
///
/// # Errors
///
/// Returns [`Error::UndefinedFunction`] when either operand is not text.
pub fn eval_ilike(value: &Value, pattern: &Value, symbol: &str) -> Result<Option<bool>> {
    if value.is_null() || pattern.is_null() {
        return Ok(None);
    }
    let (text, pat) = like_operands(value, pattern, symbol)?;
    Ok(Some(like_match(&text.to_lowercase(), &pat.to_lowercase())))
}

/// Evaluate a POSIX-regex match (`~`; the caller negates for `!~`), **narrowed** to
/// the one pattern shape pglet answers: `^` followed by characters none of which is an
/// ARE metacharacter (`. [ ] ( ) * + ? { } | ^ $ \`). Such a pattern matches exactly the
/// strings that begin with the characters after the `^` — 18.6's answer, a
/// case-sensitive prefix test — so that is what runs. Every other pattern is refused
/// `0A000`, decided here on the pattern's **value**, so a pattern bound as a parameter
/// (kysely binds `'^pg_'` as `$1`) is judged as a literal one is. Returns `None` (SQL
/// NULL) when either operand is NULL, as 18.6's strict operator does.
///
/// # Errors
///
/// `42883` for an operand pair with no operator ([`regex_legality`]); `0A000` for a
/// pattern outside the narrowed shape (a malformed one included, which 18.6 answers
/// `2201B`).
pub fn eval_regex(value: &Value, pattern: &Value, symbol: &str) -> Result<Option<bool>> {
    if value.is_null() || pattern.is_null() {
        return Ok(None);
    }
    regex_legality(
        value.type_oid().map(ResolvedType::plain).as_ref(),
        pattern.type_oid().map(ResolvedType::plain).as_ref(),
        symbol,
    )?;
    let (Value::Text(text), Value::Text(pat)) = (value, pattern) else {
        return Err(Error::UndefinedFunction(format!(
            "operator does not exist: {} {symbol} {}",
            opt_type_oid_display_name(value.type_oid()),
            opt_type_oid_display_name(pattern.type_oid())
        )));
    };
    let prefix = pat
        .strip_prefix('^')
        .filter(|rest| !rest.contains(['.', '[', ']', '(', ')', '*', '+', '?', '{', '}', '|', '^', '$', '\\']))
        .ok_or_else(|| {
            Error::NotImplemented(format!(
                "regular expression {pat:?}: only a pattern of ^ followed by literal characters is supported"
            ))
        })?;
    Ok(Some(text.starts_with(prefix)))
}

/// Evaluate `starts_with(value, prefix)` — case-sensitive literal prefix
/// match. Unlike LIKE, `prefix` is matched verbatim (no `%` / `_` wildcards).
/// Returns `None` (SQL NULL) when either operand is NULL, mirroring Postgres.
///
/// A non-text argument on either side fails loud (`42883`) via the shared
/// [`starts_with_legality`] rule, rather than silently not matching.
///
/// # Errors
///
/// Returns [`Error::UndefinedFunction`] when either argument is not text.
pub fn eval_starts_with(value: &Value, prefix: &Value) -> Result<Option<bool>> {
    if value.is_null() || prefix.is_null() {
        return Ok(None);
    }
    starts_with_legality(value.type_oid(), prefix.type_oid())?;
    let (Value::Text(text), Value::Text(pat)) = (value, prefix) else {
        // Unreachable: `starts_with_legality` accepts only the string pair.
        return Err(Error::UndefinedFunction(format!(
            "function starts_with({}, {}) does not exist",
            opt_type_oid_display_name(value.type_oid()),
            opt_type_oid_display_name(prefix.type_oid())
        )));
    };
    Ok(Some(text.starts_with(pat.as_str())))
}

/// Match a string against a LIKE pattern using dynamic programming.
///
/// `%` matches any sequence of characters (including empty).
/// `_` matches exactly one character.
///
/// Time complexity: O(n * m) where n = text length, m = pattern length.
/// Space complexity: O(m) - uses a single row of the DP table.
/// One element of a compiled LIKE pattern.
enum LikeElem {
    /// `%` — matches zero or more characters.
    Any,
    /// `_` — matches exactly one character.
    Single,
    /// A literal character (an escaped `%`/`_`/`\` lands here too).
    Lit(char),
}

/// Compile a LIKE `pattern` into elements, honoring the SQL default escape `\`:
/// a `\` makes the following character literal (so `\%` matches a literal `%`).
/// A trailing `\` is treated as a literal backslash.
fn compile_like(pattern: &str) -> Vec<LikeElem> {
    let mut elems = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => elems.push(LikeElem::Lit(chars.next().unwrap_or('\\'))),
            '%' => elems.push(LikeElem::Any),
            '_' => elems.push(LikeElem::Single),
            other => elems.push(LikeElem::Lit(other)),
        }
    }
    elems
}

fn like_match(text: &str, pattern: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let pattern = compile_like(pattern);
    let m = pattern.len();

    // dp[j] = can pattern[0..j] match text[0..current_i]?
    let mut dp = vec![false; m + 1];
    dp[0] = true;

    // Handle leading % wildcards (they can match empty string)
    for (j, pe) in pattern.iter().enumerate() {
        if matches!(pe, LikeElem::Any) {
            dp[j + 1] = dp[j];
        } else {
            break;
        }
    }

    // Process each character in text
    for &tc in &text {
        let mut prev = dp[0]; // dp[i-1][j-1] from previous row
        dp[0] = false; // Empty pattern can't match non-empty text

        for (j, pe) in pattern.iter().enumerate() {
            let temp = dp[j + 1]; // Save before overwriting
            dp[j + 1] = match pe {
                LikeElem::Any => dp[j + 1] || dp[j], // Skip char in text OR skip % in pattern
                LikeElem::Single => prev,            // Match any single char
                LikeElem::Lit(c) => prev && tc == *c, // Match literal char
            };
            prev = temp;
        }
    }

    dp[m]
}
