//! The keyed path of duplicate elimination and grouping: `SELECT DISTINCT`, `UNION`,
//! `INTERSECT` / `EXCEPT` (with or without `ALL`), a recursive CTE's `UNION`, an
//! aggregate's `DISTINCT`, and `GROUP BY`.
//!
//! Each of those sites asks whether a row equals one it has already met, by a pairwise
//! relation — the row relation `rows_distinct_equal` (`values_equal` with two NULLs and
//! two float `NaN`s counted equal), or, for an aggregate's `DISTINCT`, `values_compare`'s
//! `Equal` with two NULLs counted equal. A dedup site keeps a row unless it equals a row
//! already kept; a set operation puts a left row, and `GROUP BY` an input row by its key
//! values, in the first group it equals. Asked of every kept row or group, that is
//! quadratic. [`DistinctKeys`] answers the same question from a hash of each row's
//! [`index_key_identity`], but only while the key relation is **certain**
//! to be the site's relation: the two part on a mixed-type pair (`values_equal` folds
//! `int4` `1` with `int8` `1`, whose keys differ; `float8` against `int8` is not
//! transitive near 2^53), so the key is used only while every column's non-NULL values
//! are of one [`key_class`]. The first row that breaks that sends the site back to its
//! pairwise relation for the rest of the input, against the rows (or groups) it has
//! kept so far — which are what the pairwise path would have kept, so the answer is the
//! same either way. That fallback is quadratic: each later row is asked of every row or
//! group kept, so a column that mixes classes costs time in the product of the rows left
//! and the rows or groups kept.
//!
//! Within one class, key equality is each site's equality, member by member (the
//! `parity_tests` below assert it over adversarial pairs of every class):
//!
//! - `bool`, each integer width, `text`, `bytea`, `uuid`, `date` and `time` key their
//!   value's exact bits or bytes, and both relations compare exactly those.
//! - `float4` and `float8` are one class: both relations compare the `float8` a value
//!   widens to under `float8_cmp_internal`'s total order, and the key is that order's —
//!   `-0` keys as `0`, every `NaN` as one key.
//! - `numeric` keys its normalized digits and weight, so display scale folds as both
//!   relations fold it (`1.5` = `1.50`), and `NaN` is one key.
//! - `interval` keys its comparison estimate, which is what both relations compare.
//! - `jsonb` keys its tree in stored order, a number by its `numeric` key — the
//!   structural equality `values_equal` reads, which `jsonb_compare` calls `Equal` on
//!   exactly the same pairs.
//! - `timestamp` keys the microsecond count, so it is the class only for a value that is
//!   microsecond-exact (the `Value::Timestamp` invariant, checked per value rather than
//!   assumed: two instants within one microsecond would share a key).
//! - An array compares element-wise with its length, two NULL elements equal, and keys
//!   each element self-delimited; it is the class only while every non-NULL element in
//!   the column is of one scalar class (`int4[]` beside `int8[]` folds under
//!   `values_equal` and keys apart).

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::types::Value;

use super::super::constraints::index_key_identity;

/// The array class: an array's own class, its elements' class tracked beside it.
const ARRAY_CLASS: u8 = 11;

/// The class a non-NULL value's key is equality-exact within, or `None` for a value
/// whose key is not certain to be its relation even within its type.
///
/// Two values of one class have equal keys iff the dedup sites call them equal — for
/// two arrays, while their non-NULL elements are of one class as well, which
/// [`DistinctKeys`] tracks beside it. Two values of different classes are never
/// compared by key.
fn key_class(value: &Value) -> Option<u8> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(_) => 1,
        Value::Int2(_) => 2,
        Value::Int4(_) => 3,
        Value::Int8(_) => 4,
        Value::Float4(_) | Value::Float8(_) => 5,
        Value::Text(_) => 6,
        Value::Bytea(_) => 7,
        Value::Timestamp(ts) => {
            if ts.finite().is_some_and(|t| t.nanosecond() % 1_000 != 0) {
                return None;
            }
            8
        }
        Value::Jsonb(_) => 9,
        Value::Uuid(_) => 10,
        Value::Array(_) => ARRAY_CLASS,
        Value::Interval(_) => 12,
        Value::Date(_) => 13,
        Value::Numeric(_) => 14,
        Value::Time(_) => 15,
    })
}

/// The value [`DistinctKeys`]' map holds for group `index` — a grouping site keeps its
/// group index in the map type every keyed site shares, instantiating no map of its own.
pub(super) fn group_slot(index: usize) -> Vec<Value> {
    vec![Value::Int8(i64::try_from(index).unwrap_or(i64::MAX))]
}

/// The group index a [`group_slot`] holds.
pub(super) fn held_group(held: &[Value]) -> Option<usize> {
    match held {
        [Value::Int8(i)] => usize::try_from(*i).ok(),
        _ => None,
    }
}

/// Record `class` in `slot` (`0` = nothing seen yet), answering whether it agrees with
/// what the slot already holds.
const fn claim(slot: &mut u8, class: u8) -> bool {
    if *slot == 0 {
        *slot = class;
    }
    *slot == class
}

/// Duplicate detection by key, for as long as the key is certain to be the site's
/// relation (see the module doc).
///
/// A site asks [`DistinctKeys::entry`] for each row in input order. `Some` is the row's
/// slot in the map of keys seen; `None` means the keyed path no longer holds, from this
/// row on, and the site asks its pairwise relation against the rows it has kept.
pub(super) struct DistinctKeys {
    /// Per column, the class of its non-NULL values and the class of its arrays'
    /// non-NULL elements, each `0` while none has been seen.
    classes: Vec<[u8; 2]>,
    /// The keys seen, `None` once a row broke the class gate.
    seen: Option<HashMap<Vec<u8>, Vec<Value>>>,
}

impl DistinctKeys {
    /// A detector over rows of `width` columns.
    pub(super) fn new(width: usize) -> Self {
        Self {
            classes: vec![[0, 0]; width],
            seen: Some(HashMap::new()),
        }
    }

    /// `row`'s slot in the map of keys seen, or `None` — for this row and every later
    /// one — where the key is not certain to be the site's relation.
    pub(super) fn entry(&mut self, row: &[Value]) -> Option<Entry<'_, Vec<u8>, Vec<Value>>> {
        self.seen.as_ref()?;
        let key = if self.admit(row) {
            index_key_identity(row).ok()
        } else {
            None
        };
        let Some(key) = key else {
            self.seen = None;
            return None;
        };
        self.seen.as_mut().map(|seen| seen.entry(key))
    }

    /// Whether `row` is the first of its key — `Some(true)` the first, `Some(false)` a
    /// repeat — or `None` where the site's pairwise relation must answer.
    pub(super) fn first_sighting(&mut self, row: &[Value]) -> Option<bool> {
        Some(match self.entry(row)? {
            Entry::Vacant(slot) => {
                slot.insert(Vec::new());
                true
            }
            Entry::Occupied(_) => false,
        })
    }

    /// Fold `row` into the per-column classes, answering whether every column still
    /// holds one class.
    fn admit(&mut self, row: &[Value]) -> bool {
        row.len() == self.classes.len()
            && row.iter().zip(&mut self.classes).all(|(value, slot)| {
                if value.is_null() {
                    return true;
                }
                let Some(class) = key_class(value) else {
                    return false;
                };
                if !claim(&mut slot[0], class) {
                    return false;
                }
                let Value::Array(array) = value else {
                    return true;
                };
                array.elements.iter().filter(|e| !e.is_null()).all(|element| {
                    matches!(key_class(element), Some(c) if c != ARRAY_CLASS && claim(&mut slot[1], c))
                })
            })
    }
}

#[cfg(test)]
thread_local! {
    /// How many times a dedup site asked its pairwise relation on this thread — the
    /// cost guard's instrument: a column of one class must leave it at zero.
    pub(super) static PAIRWISE_ASKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count one pairwise ask (see [`PAIRWISE_ASKS`]).
#[cfg(test)]
pub(super) fn count_pairwise_ask() {
    PAIRWISE_ASKS.with(|n| n.set(n.get() + 1));
}

#[cfg(test)]
mod parity_tests {
    use std::cmp::Ordering;

    use super::{DistinctKeys, index_key_identity, key_class};
    use crate::executor::expr::values_compare;
    use crate::types::{ArrayValue, PgInterval, PgNumeric, TypeOid, Value, parse_jsonb};
    use time::macros::{date, datetime};

    fn numeric(s: &str) -> Value {
        Value::Numeric(PgNumeric::parse(s).expect("numeric literal"))
    }

    fn jsonb(s: &str) -> Value {
        Value::Jsonb(parse_jsonb(s).expect("jsonb literal"))
    }

    /// Adversarial values per class: pairs that one relation or the other might fold.
    fn classes() -> Vec<Vec<Value>> {
        vec![
            vec![Value::Bool(false), Value::Bool(true)],
            vec![
                Value::Int2(0),
                Value::Int2(-1),
                Value::Int2(i16::MIN),
                Value::Int2(1),
            ],
            vec![
                Value::Int4(0),
                Value::Int4(-1),
                Value::Int4(i32::MAX),
                Value::Int4(1),
            ],
            vec![
                Value::Int8(0),
                Value::Int8((1 << 53) + 1),
                Value::Int8(1 << 53),
                Value::Int8(i64::MIN),
            ],
            vec![
                Value::Float8(0.0),
                Value::Float8(-0.0),
                Value::Float4(-0.0),
                Value::Float8(f64::NAN),
                Value::Float8(-f64::NAN),
                Value::Float4(f32::NAN),
                Value::Float4(1.5),
                Value::Float8(1.5),
                Value::Float4(1.1),
                Value::Float8(1.1),
                Value::Float8(f64::from(1.1_f32)),
                Value::Float8(f64::INFINITY),
                Value::Float4(f32::NEG_INFINITY),
                Value::Float8(9_007_199_254_740_992.0),
            ],
            vec![
                Value::Text(String::new()),
                Value::Text("a".into()),
                Value::Text("a\0".into()),
                Value::Text("A".into()),
                Value::Text("é".into()),
                Value::Text("e\u{301}".into()),
            ],
            vec![
                Value::Bytea(Vec::new()),
                Value::Bytea(vec![0]),
                Value::Bytea(vec![0, 0]),
                Value::Bytea(vec![1]),
            ],
            vec![
                Value::timestamp(datetime!(2024-01-01 00:00:00 UTC)),
                Value::timestamp(datetime!(2024-01-01 01:00:00 +01:00)),
                Value::timestamp(datetime!(2024-01-01 00:00:00.000001 UTC)),
                Value::Timestamp(crate::types::Infinite::Infinity),
                Value::Timestamp(crate::types::Infinite::NegInfinity),
            ],
            vec![
                jsonb("1"),
                jsonb("1.0"),
                jsonb("1e0"),
                jsonb("\"1\""),
                jsonb("[]"),
                jsonb("[1.0]"),
                jsonb("[1]"),
                jsonb("{}"),
                jsonb(r#"{"a":1}"#),
                jsonb(r#"{"a":1.00}"#),
                jsonb(r#"{"a":"1"}"#),
                jsonb(r#"{"ab":1}"#),
                jsonb(r#"{"a":1,"b":2}"#),
                jsonb(r#"{"b":2,"a":1}"#),
                jsonb("null"),
                jsonb("true"),
                jsonb("[null]"),
            ],
            vec![Value::Uuid([0; 16]), Value::Uuid([1; 16])],
            vec![
                Value::Interval(PgInterval::new(1, 0, 0)),
                Value::Interval(PgInterval::new(0, 30, 0)),
                Value::Interval(PgInterval::new(0, 1, 0)),
                Value::Interval(PgInterval::from_micros(86_400_000_000)),
                Value::Interval(PgInterval::new(0, -1, 86_400_000_000)),
                Value::Interval(PgInterval::from_micros(1)),
            ],
            vec![
                Value::date(date!(2024 - 01 - 01)),
                Value::date(date!(2024 - 01 - 02)),
                Value::Date(crate::types::Infinite::Infinity),
            ],
            vec![
                numeric("1.5"),
                numeric("1.50"),
                numeric("0"),
                numeric("0.00"),
                numeric("-0"),
                numeric("100"),
                numeric("1e2"),
                numeric("-1.5"),
                numeric("NaN"),
                numeric("12345678901234567890.123"),
                numeric("12345678901234567890.1230"),
            ],
            vec![Value::Time(0), Value::Time(86_400_000_000), Value::Time(1)],
        ]
    }

    fn key_eq(a: &Value, b: &Value) -> bool {
        index_key_identity(std::slice::from_ref(a)).expect("key")
            == index_key_identity(std::slice::from_ref(b)).expect("key")
    }

    /// The row sites' relation on one column (`rows_distinct_equal`'s per-value arm).
    fn distinct_eq(a: &Value, b: &Value) -> bool {
        super::super::rows_distinct_equal(std::slice::from_ref(a), std::slice::from_ref(b))
    }

    /// The aggregate site's relation.
    fn compare_eq(a: &Value, b: &Value) -> bool {
        values_compare(a, b) == Some(Ordering::Equal)
    }

    fn assert_parity(values: &[Value]) {
        for a in values {
            for b in values {
                let k = key_eq(a, b);
                assert_eq!(k, distinct_eq(a, b), "row relation vs key: {a:?} / {b:?}");
                assert_eq!(
                    k,
                    compare_eq(a, b),
                    "aggregate relation vs key: {a:?} / {b:?}"
                );
            }
        }
    }

    #[test]
    fn within_one_class_key_equality_is_each_sites_equality() {
        for class in classes() {
            let first = key_class(&class[0]);
            assert!(first.is_some());
            assert!(
                class.iter().all(|v| key_class(v) == first),
                "one class: {class:?}"
            );
            assert_parity(&class);
        }
    }

    /// Arrays of each class's values: one-, two-element, NULL-holding and empty arrays,
    /// every pair compared under both relations against the key.
    #[test]
    fn within_one_element_class_array_key_equality_is_each_sites_equality() {
        for class in classes() {
            let ty = class[0].type_oid().unwrap_or(TypeOid::Text);
            let mut arrays = vec![
                Value::Array(ArrayValue::new(ty, Vec::new())),
                Value::Array(ArrayValue::new(ty, vec![Value::Null])),
                Value::Array(ArrayValue::new(ty, vec![Value::Null, Value::Null])),
            ];
            for a in &class {
                arrays.push(Value::Array(ArrayValue::new(ty, vec![a.clone()])));
                arrays.push(Value::Array(ArrayValue::new(
                    ty,
                    vec![a.clone(), Value::Null],
                )));
                arrays.push(Value::Array(ArrayValue::new(
                    ty,
                    vec![Value::Null, a.clone()],
                )));
                for b in class.iter().take(4) {
                    arrays.push(Value::Array(ArrayValue::new(
                        ty,
                        vec![a.clone(), b.clone()],
                    )));
                }
            }
            assert_parity(&arrays);
        }
    }

    /// A timestamp off the microsecond grid has no class: its key would fold it with
    /// the instant it truncates to, which neither relation does.
    #[test]
    fn a_sub_microsecond_timestamp_is_not_keyed() {
        let exact = Value::timestamp(datetime!(2024-01-01 00:00:00.000001 UTC));
        let off = Value::timestamp(datetime!(2024-01-01 00:00:00.0000015 UTC));
        assert!(key_eq(&exact, &off));
        assert!(!distinct_eq(&exact, &off));
        assert_eq!(key_class(&off), None);
        let mut keys = DistinctKeys::new(1);
        assert_eq!(
            keys.first_sighting(std::slice::from_ref(&exact)),
            Some(true)
        );
        assert_eq!(keys.first_sighting(std::slice::from_ref(&off)), None);
    }

    /// Each mixed pair a relation folds and the key keeps apart, or the reverse, is
    /// answered `None` — the pairwise path — at the row that mixes the column, and at
    /// every row after it.
    #[test]
    fn a_mixed_column_leaves_the_keyed_path_for_good() {
        let int_array = |v: Value| Value::Array(ArrayValue::new(TypeOid::Int4, vec![v]));
        for (first, mixed) in [
            (Value::Int4(1), Value::Int8(1)),
            (Value::Int8(1 << 53), Value::Float8(9_007_199_254_740_992.0)),
            (Value::Int4(1), numeric("1")),
            (Value::Text("1".into()), Value::Int4(1)),
            (
                Value::date(date!(2024 - 01 - 01)),
                Value::timestamp(datetime!(2024-01-01 00:00:00 UTC)),
            ),
            (
                Value::Time(86_400_000_000),
                Value::Interval(PgInterval::new(0, 1, 0)),
            ),
            (int_array(Value::Int4(1)), int_array(Value::Int8(1))),
        ] {
            let mut keys = DistinctKeys::new(1);
            assert_eq!(
                keys.first_sighting(std::slice::from_ref(&first)),
                Some(true)
            );
            assert_eq!(keys.first_sighting(std::slice::from_ref(&mixed)), None);
            // sticky: a row of the first class is not keyed again
            assert_eq!(keys.first_sighting(std::slice::from_ref(&first)), None);
        }
        // a nested array has no class
        let nested = Value::Array(ArrayValue::new(
            TypeOid::Int4,
            vec![int_array(Value::Int4(1))],
        ));
        assert_eq!(
            DistinctKeys::new(1).first_sighting(std::slice::from_ref(&nested)),
            None
        );
        // a row of another width
        assert_eq!(DistinctKeys::new(2).first_sighting(&[Value::Int4(1)]), None);
    }

    /// NULLs take no class, and the key tags a NULL apart from an empty value and
    /// frames each column, so `('ab','c')` is not `('a','bc')`.
    #[test]
    fn nulls_and_framing_key_apart() {
        let text = |s: &str| Value::Text(s.into());
        let mut keys = DistinctKeys::new(2);
        for (row, fresh) in [
            (vec![Value::Null, text("")], true),
            (vec![text(""), Value::Null], true),
            (vec![text(""), text("")], true),
            (vec![Value::Null, Value::Null], true),
            (vec![Value::Null, Value::Null], false),
            (vec![text("ab"), text("c")], true),
            (vec![text("a"), text("bc")], true),
            (vec![text("a"), text("bc")], false),
        ] {
            assert_eq!(keys.first_sighting(&row), Some(fresh), "{row:?}");
        }
    }
}

/// The sites, driven end to end and against today's pairwise algorithm.
#[cfg(test)]
mod site_tests {
    use super::PAIRWISE_ASKS;
    use crate::catalog::Schema;
    use crate::planner::SetOpKind;
    use crate::types::{ArrayValue, PgInterval, PgNumeric, TypeOid, Value};

    fn run(sql: &str) -> Vec<Vec<Value>> {
        let mut schema = Schema::new();
        let plan = crate::parser::parse(sql, &schema)
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .remove(0);
        crate::executor::execute(&plan, &mut schema)
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .rows
    }

    /// `sql`'s rows and how many pairwise asks it took.
    fn run_counted(sql: &str) -> (Vec<Vec<Value>>, usize) {
        PAIRWISE_ASKS.with(|n| n.set(0));
        let rows = run(sql);
        (rows, PAIRWISE_ASKS.with(std::cell::Cell::get))
    }

    /// `n` rows `1..=n` as `g(n)`.
    fn series(n: usize) -> String {
        format!("WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < {n}) ")
    }

    fn ints(rows: &[Vec<Value>]) -> Vec<i64> {
        rows.iter()
            .map(|r| match r.as_slice() {
                [Value::Int4(v)] => i64::from(*v),
                [Value::Int8(v)] => *v,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    /// The cost guard: over a column of one class, no site asks its pairwise relation
    /// once — where every site used to ask it of every kept row. Each answer is checked
    /// too, in first-occurrence order.
    #[test]
    fn a_column_of_one_class_never_asks_the_pairwise_relation() {
        let g = series(3000);
        let (rows, asks) = run_counted(&format!("{g}SELECT DISTINCT n % 1000 FROM g"));
        assert_eq!(asks, 0, "DISTINCT");
        assert_eq!(ints(&rows), (1..1000).chain([0]).collect::<Vec<_>>());

        let (rows, asks) = run_counted(&format!(
            "{g}SELECT n % 1000 FROM g UNION SELECT n % 1500 FROM g"
        ));
        assert_eq!(asks, 0, "UNION");
        assert_eq!(
            ints(&rows),
            (1..1000).chain([0]).chain(1000..1500).collect::<Vec<_>>()
        );

        let (rows, asks) = run_counted(&format!(
            "{g}SELECT n % 1000 FROM g INTERSECT ALL SELECT n % 1500 FROM g"
        ));
        assert_eq!(asks, 0, "INTERSECT ALL");
        // left: each residue 3 times; right: residues < 1000 twice, the rest once
        assert_eq!(rows.len(), 2000);

        let (rows, asks) = run_counted(&format!(
            "{g}SELECT n % 1000 FROM g EXCEPT ALL SELECT n % 1500 FROM g"
        ));
        assert_eq!(asks, 0, "EXCEPT ALL");
        assert_eq!(rows.len(), 1000);

        let (rows, asks) = run_counted(&format!(
            "{g}SELECT n % 1000 FROM g INTERSECT SELECT n % 500 FROM g"
        ));
        assert_eq!(asks, 0, "INTERSECT");
        assert_eq!(ints(&rows), (1..500).chain([0]).collect::<Vec<_>>());

        let (rows, asks) = run_counted(&format!(
            "{g}SELECT n % 1000 FROM g EXCEPT SELECT n % 500 FROM g"
        ));
        assert_eq!(asks, 0, "EXCEPT");
        assert_eq!(ints(&rows), (500..1000).collect::<Vec<_>>());

        let (rows, asks) = run_counted(
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION SELECT n % 2000 + 1 FROM c) \
             SELECT count(*) FROM c",
        );
        assert_eq!(asks, 0, "recursive UNION");
        assert_eq!(ints(&rows), vec![2000]);

        let (rows, asks) = run_counted(&format!("{g}SELECT count(DISTINCT n % 1000) FROM g"));
        assert_eq!(asks, 0, "count(DISTINCT)");
        assert_eq!(ints(&rows), vec![1000]);
    }

    /// The keyed path keeps first-occurrence order, so a `LIMIT` over a `DISTINCT` (which
    /// reads the dedup's order directly) answers the first values met.
    #[test]
    fn distinct_keeps_first_occurrence_order_under_a_limit() {
        let rows = run("SELECT DISTINCT x FROM (VALUES (3), (1), (3), (2), (1), (4)) v(x) LIMIT 3");
        assert_eq!(ints(&rows), vec![3, 1, 2]);
        let rows = run(
            "SELECT x FROM (VALUES (3), (1), (3)) a(x) UNION SELECT x FROM (VALUES (2), (1), (4)) b(x) LIMIT 4",
        );
        assert_eq!(ints(&rows), vec![3, 1, 2, 4]);
    }

    /// Every site's pairwise half, reached from SQL: a scalar-subquery `CASE` branch
    /// left bare beside an `int4` branch under one declared `numeric` (`EDGES.md` row
    /// 152) puts `int4` `1`, `numeric` `1.0`, `int4` `2`, `int4` `2` in one column. The
    /// class gate gives way at `1.0`, and from there the pairwise relation alone must
    /// find `1.0` equal to the kept `1` and the second `2` equal to the first.
    #[test]
    fn a_mixed_column_from_sql_takes_each_sites_pairwise_answer() {
        let m = "CASE WHEN x = 7 THEN (SELECT * FROM unnest(ARRAY[1.0::numeric])) ELSE x END";
        let v = "(VALUES (1), (7), (2), (2)) v(x)";
        let i = Value::Int4;
        for (sql, expected) in [
            (
                format!("SELECT DISTINCT {m} AS n FROM {v}"),
                vec![i(1), i(2)],
            ),
            (
                format!("SELECT {m} AS n FROM {v} UNION SELECT 3"),
                vec![
                    i(1),
                    i(2),
                    Value::Numeric(PgNumeric::parse("3").expect("numeric")),
                ],
            ),
            (
                format!("SELECT {m} AS n FROM {v} EXCEPT ALL SELECT 2"),
                vec![i(1), i(1), i(2)],
            ),
            (
                format!(
                    "SELECT {m} AS n FROM {v} INTERSECT ALL SELECT x FROM (VALUES (1), (2), (2)) w(x)"
                ),
                vec![i(1), i(2), i(2)],
            ),
            (
                format!("SELECT count(DISTINCT n) FROM (SELECT {m} AS n FROM {v}) s"),
                vec![Value::Int8(2)],
            ),
            (
                format!(
                    "WITH RECURSIVE r(n) AS (SELECT {m} FROM {v} UNION SELECT n + 0 FROM r WHERE n < 0) \
                     SELECT * FROM r"
                ),
                vec![i(1), i(2)],
            ),
        ] {
            let (rows, asks) = run_counted(&sql);
            let flat: Vec<Value> = rows.into_iter().flatten().collect();
            assert_eq!(flat, expected, "{sql}");
            assert!(asks > 0, "the pairwise relation answered nothing: {sql}");
        }
    }

    /// `GROUP BY`'s cost guard and order: over a key of one class no group is found
    /// pairwise, and the groups come out in the order their first rows arrive (a hash
    /// map's iteration order would move between runs).
    #[test]
    fn group_by_over_one_class_never_asks_and_keeps_first_seen_order() {
        let g = series(3000);
        let (rows, asks) = run_counted(&format!(
            "{g}SELECT n % 1000, count(*) FROM g GROUP BY n % 1000"
        ));
        assert_eq!(asks, 0, "GROUP BY");
        let keys: Vec<i64> = rows
            .iter()
            .map(|r| match r.as_slice() {
                [Value::Int4(k), Value::Int8(3)] => i64::from(*k),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(keys, (1..1000).chain([0]).collect::<Vec<_>>());
        let (rows, asks) = run_counted(
            "SELECT x, y, count(*) FROM (VALUES (2, 'b'), (1, 'a'), (2, 'b'), (1, NULL), (1, NULL)) \
             v(x, y) GROUP BY x, y",
        );
        assert_eq!(asks, 0, "two keys");
        let text = |s: &str| Value::Text(s.into());
        assert_eq!(
            rows,
            vec![
                vec![Value::Int4(2), text("b"), Value::Int8(2)],
                vec![Value::Int4(1), text("a"), Value::Int8(1)],
                vec![Value::Int4(1), Value::Null, Value::Int8(2)],
            ]
        );
    }

    /// `GROUP BY`'s pairwise half, reached from SQL through the mixed column of the
    /// test above: `int4` `1` and `numeric` `1.0` are one group, as are `int4` `1` and
    /// `int8` `1`, shown by the first row's value, as 18.6 (whose column is one type)
    /// answers `1 | 2`, `2 | 2`.
    #[test]
    fn a_mixed_group_by_key_from_sql_groups_pairwise() {
        let v = "(VALUES (1), (7), (2), (2)) v(x)";
        for branch in ["1.0::numeric", "1::int8"] {
            let m =
                format!("CASE WHEN x = 7 THEN (SELECT * FROM unnest(ARRAY[{branch}])) ELSE x END");
            let (rows, asks) = run_counted(&format!(
                "SELECT n, count(*) FROM (SELECT {m} AS n FROM {v}) s GROUP BY n"
            ));
            assert_eq!(
                rows,
                vec![
                    vec![Value::Int4(1), Value::Int8(2)],
                    vec![Value::Int4(2), Value::Int8(2)],
                ],
                "{branch}"
            );
            assert!(asks > 0, "the pairwise relation answered nothing: {branch}");
        }
    }

    /// The instrument's other polarity: a column that mixes classes does reach the
    /// pairwise relation, so a zero above is the keyed path and not a dead counter.
    #[test]
    fn a_mixed_column_asks_the_pairwise_relation() {
        PAIRWISE_ASKS.with(|n| n.set(0));
        let rows = super::super::dedup_rows(vec![
            vec![Value::Int4(1)],
            vec![Value::Int8(1)],
            vec![Value::Int8(2)],
        ]);
        assert_eq!(rows, vec![vec![Value::Int4(1)], vec![Value::Int8(2)]]);
        assert!(PAIRWISE_ASKS.with(std::cell::Cell::get) > 0);
    }

    /// Today's algorithms, verbatim in shape: the reference the keyed sites must agree
    /// with on every input, the mixed-class ones included.
    fn reference_dedup(rows: &[Vec<Value>]) -> Vec<Vec<Value>> {
        let mut out: Vec<Vec<Value>> = Vec::new();
        for row in rows {
            if !out
                .iter()
                .any(|seen| super::super::rows_distinct_equal(seen, row))
            {
                out.push(row.clone());
            }
        }
        out
    }

    fn reference_set_op(
        op: SetOpKind,
        all: bool,
        left: &[Vec<Value>],
        right: &[Vec<Value>],
    ) -> Vec<Vec<Value>> {
        let eq = super::super::rows_distinct_equal;
        let mut groups: Vec<(&Vec<Value>, usize)> = Vec::new();
        for row in left {
            if let Some(g) = groups.iter_mut().find(|(rep, _)| eq(rep, row)) {
                g.1 += 1;
            } else {
                groups.push((row, 1));
            }
        }
        let mut out = Vec::new();
        for (rep, lc) in groups {
            let rc = right.iter().filter(|r| eq(r, rep)).count();
            let emit = match (op, all) {
                (SetOpKind::Intersect, false) => usize::from(rc > 0),
                (SetOpKind::Intersect, true) => lc.min(rc),
                (SetOpKind::Except, false) => usize::from(rc == 0),
                (SetOpKind::Except, true) => lc.saturating_sub(rc),
                (SetOpKind::Union, _) => 0,
            };
            for _ in 0..emit {
                out.push(rep.clone());
            }
        }
        out
    }

    /// A pool of values that fold within their class (`1.5`/`1.50`, `-0`/`0`, `1 mon` /
    /// `30 days`) and across classes (`1` as `int4`, `int8`, `numeric`, `float8`; the
    /// float/`int8` pair near 2^53 that is not transitive).
    fn pool() -> Vec<Value> {
        let num = |s: &str| Value::Numeric(PgNumeric::parse(s).expect("numeric"));
        vec![
            Value::Null,
            Value::Int4(1),
            Value::Int4(2),
            Value::Int8(1),
            Value::Int8((1 << 53) + 1),
            Value::Int8(1 << 53),
            Value::Float8(9_007_199_254_740_992.0),
            Value::Float8(-0.0),
            Value::Float8(0.0),
            Value::Float4(1.0),
            num("1.0"),
            num("1.00"),
            num("2"),
            Value::Text("1".into()),
            Value::Text(String::new()),
            Value::Interval(PgInterval::new(1, 0, 0)),
            Value::Interval(PgInterval::new(0, 30, 0)),
            Value::Array(ArrayValue::new(TypeOid::Numeric, vec![num("1.0")])),
            Value::Array(ArrayValue::new(TypeOid::Numeric, vec![num("1.00")])),
            Value::Array(ArrayValue::new(TypeOid::Int4, vec![Value::Int4(1)])),
        ]
    }

    /// A small xorshift, so the sweep is reproducible.
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    /// Rows over a sub-pool: a narrow one stays inside one class (keyed throughout), a
    /// wide one mixes classes at a random row (keyed, then pairwise).
    fn rows(rng: &mut Rng, pool: &[Value], width: usize) -> Vec<Vec<Value>> {
        let len = rng.below(12);
        (0..len)
            .map(|_| {
                (0..width)
                    .map(|_| pool[rng.below(pool.len())].clone())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn every_site_agrees_with_the_pairwise_algorithm() {
        let pool = pool();
        let sub_pools: Vec<Vec<Value>> = vec![
            pool.clone(),
            pool[..3].to_vec(),
            vec![
                Value::Null,
                pool[7].clone(),
                pool[8].clone(),
                pool[6].clone(),
            ],
            vec![
                Value::Null,
                pool[10].clone(),
                pool[11].clone(),
                pool[12].clone(),
            ],
            vec![pool[15].clone(), pool[16].clone(), Value::Null],
            vec![pool[17].clone(), pool[18].clone(), pool[19].clone()],
            vec![
                pool[3].clone(),
                pool[4].clone(),
                pool[5].clone(),
                pool[6].clone(),
            ],
        ];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..4000 {
            let sub = &sub_pools[round % sub_pools.len()];
            let width = 1 + rng.below(2);
            let left = rows(&mut rng, sub, width);
            let right = rows(&mut rng, sub, width);
            assert_eq!(
                super::super::dedup_rows(left.clone()),
                reference_dedup(&left),
                "dedup {left:?}"
            );
            for op in [SetOpKind::Intersect, SetOpKind::Except] {
                for all in [false, true] {
                    assert_eq!(
                        super::super::set_op_multiset(op, all, &left, &right),
                        reference_set_op(op, all, &left, &right),
                        "{op:?} all={all} {left:?} / {right:?}"
                    );
                }
            }
        }
    }
}
