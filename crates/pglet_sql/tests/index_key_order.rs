//! A container value's index key orders as the comparator does.
//!
//! `Value::encode_index_key`'s `jsonb` and array arms are byte encodings of
//! `values_compare` — the order a `Sort`, a `Filter` and the stripped build answer by.
//! An index answers by the bytes, so `sign(memcmp(key a, key b)) == values_compare(a,
//! b)`, `Equal` included, is the condition under which the two build profiles agree
//! over a container key and a unique index's duplicates are `=`'s.
//!
//! This checks pglet against itself. What 18.6 answers for the same values is the
//! differential fixtures' (`tests/fixtures/container_key/`) and the measured ladders in
//! `pglet_core`'s `index_key` tests.

// A test module: a generator that fails to build its population should fail the test.
#![allow(clippy::expect_used)]

use std::cmp::Ordering;

use pglet_sql::executor::values_compare;
use pglet_sql::types::{
    ArrayValue, Infinite, JsonbValue, PgInterval, PgNumeric, TypeOid, Value, parse_jsonb,
};

/// A small deterministic generator (xorshift64).
struct Rng(u64);

impl Rng {
    const fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }

    const fn pick<'a, T>(&mut self, from: &'a [T]) -> &'a T {
        &from[self.below(from.len() as u64) as usize]
    }
}

/// Strings chosen to collide as prefixes and across the escape: empty, a NUL, a NUL
/// then more, `0xFF`-leading UTF-8, and a multi-byte character.
const STRINGS: &[&str] = &["", "a", "ab", "b", "\0", "\0a", "a\0", "\u{e9}", "A", "aa"];

/// Numbers in pairs that are one value under another scale or spelling, with neighbours
/// either side of each.
const NUMBERS: &[&str] = &[
    "0",
    "-0",
    "0.0",
    "1",
    "1.0",
    "1.00",
    "1e0",
    "10",
    "1e1",
    "100",
    "1e2",
    "-1",
    "-1.0",
    "-10",
    "0.1",
    "0.10",
    "0.01",
    "1.5",
    "1.50",
    "-1.5",
    "99999",
    "100000",
    "1e-7",
    "12345678901234567890",
];

fn numeric(text: &str) -> PgNumeric {
    PgNumeric::parse(text).expect("a numeric literal")
}

/// A random `jsonb` value at most `depth` containers deep, in canonical form.
fn random_jsonb(rng: &mut Rng, depth: u32) -> JsonbValue {
    let kinds = if depth == 0 { 4 } else { 6 };
    let value = match rng.below(kinds) {
        0 => JsonbValue::Null,
        1 => JsonbValue::Bool(rng.below(2) == 0),
        2 => JsonbValue::Number(numeric(rng.pick(NUMBERS))),
        3 => JsonbValue::String((*rng.pick(STRINGS)).to_string()),
        4 => JsonbValue::Array(
            (0..rng.below(4))
                .map(|_| random_jsonb(rng, depth - 1))
                .collect(),
        ),
        _ => JsonbValue::Object(
            (0..rng.below(4))
                .map(|_| {
                    (
                        (*rng.pick(STRINGS)).to_string(),
                        random_jsonb(rng, depth - 1),
                    )
                })
                .collect(),
        ),
    };
    value.canonical()
}

/// The measured 18.6 ladder plus a random nested population.
fn jsonb_population() -> Vec<Value> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut values: Vec<Value> = [
        "[]",
        "null",
        r#""""#,
        r#""a""#,
        "-1",
        "0",
        "100",
        "1e2",
        "false",
        "true",
        "[null]",
        r#"["a"]"#,
        "[1]",
        "[1.0]",
        "[true]",
        "[[]]",
        "[{}]",
        "[0,0]",
        "[[1],3]",
        "[[1,2],3]",
        "{}",
        r#"{"a":null}"#,
        r#"{"a":1}"#,
        r#"{"a":1.00}"#,
        r#"{"aa":1}"#,
        r#"{"b":1}"#,
        r#"{"a":1,"b":1}"#,
        r#"{"b":1,"aa":2}"#,
        r#"{"b":2,"aa":1}"#,
    ]
    .iter()
    .map(|text| Value::Jsonb(parse_jsonb(text).expect("valid json")))
    .collect();
    values.extend((0..400).map(|_| Value::Jsonb(random_jsonb(&mut rng, 3))));
    // Every string beside every other, where a value follows it: as the first of two
    // array elements and as an object's key. A string's end is only tested by what
    // comes after it, and the random population is too sparse to put each pair there.
    let string = |s: &&str| JsonbValue::String((*s).to_string());
    for a in STRINGS {
        for b in STRINGS.iter().step_by(3) {
            values.push(Value::Jsonb(JsonbValue::Array(vec![string(a), string(b)])));
            values.push(Value::Jsonb(JsonbValue::Object(vec![(
                (*a).to_string(),
                string(b),
            )])));
        }
    }
    values
}

/// Every element type with a key encoder, each with the values its own order turns on.
fn element_populations() -> Vec<(TypeOid, Vec<Value>)> {
    let text = |s: &&str| Value::Text((*s).to_string());
    let interval = |months, days, micros| {
        Value::Interval(PgInterval {
            months,
            days,
            micros,
        })
    };
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    vec![
        (TypeOid::Bool, vec![Value::Bool(false), Value::Bool(true)]),
        (
            TypeOid::Int2,
            [i16::MIN, -1, 0, 1, 256, i16::MAX]
                .map(Value::Int2)
                .to_vec(),
        ),
        (
            TypeOid::Int4,
            [i32::MIN, -1, 0, 1, 256, 65536, i32::MAX]
                .map(Value::Int4)
                .to_vec(),
        ),
        (
            TypeOid::Int8,
            // Either side of 2^53, where an `f64` cannot tell neighbours apart.
            [
                i64::MIN,
                -9_007_199_254_740_993,
                -9_007_199_254_740_992,
                -1,
                0,
                1,
                9_007_199_254_740_992,
                9_007_199_254_740_993,
                i64::MAX - 1,
                i64::MAX,
            ]
            .map(Value::Int8)
            .to_vec(),
        ),
        (
            TypeOid::Float4,
            [
                f32::NEG_INFINITY,
                -1.5,
                -0.0,
                0.0,
                1.1,
                f32::INFINITY,
                f32::NAN,
                -f32::NAN,
            ]
            .map(Value::Float4)
            .to_vec(),
        ),
        (
            TypeOid::Float8,
            [
                f64::NEG_INFINITY,
                -1.5,
                -5e-324,
                -0.0,
                0.0,
                5e-324,
                1.1,
                f64::INFINITY,
                f64::NAN,
                -f64::NAN,
            ]
            .map(Value::Float8)
            .to_vec(),
        ),
        (
            TypeOid::Numeric,
            NUMBERS
                .iter()
                .map(|n| Value::Numeric(numeric(n)))
                .chain([Value::Numeric(PgNumeric::NaN)])
                .collect(),
        ),
        (TypeOid::Text, STRINGS.iter().map(text).collect()),
        (
            TypeOid::Bytea,
            [
                &b""[..],
                b"\0",
                b"\0\0",
                b"\0\xff",
                b"\xff",
                b"a",
                b"a\0",
                b"\x01",
            ]
            .iter()
            .map(|b| Value::Bytea(b.to_vec()))
            .collect(),
        ),
        (
            TypeOid::Uuid,
            [[0u8; 16], [1; 16], [0xFF; 16]].map(Value::Uuid).to_vec(),
        ),
        (
            TypeOid::Timestamp,
            vec![
                Value::Timestamp(Infinite::NegInfinity),
                Value::Timestamp(Infinite::from_pg_micros(-1).expect("in range")),
                Value::Timestamp(Infinite::from_pg_micros(0).expect("in range")),
                Value::Timestamp(Infinite::from_pg_micros(256).expect("in range")),
                Value::Timestamp(Infinite::Infinity),
            ],
        ),
        (
            TypeOid::Date,
            vec![
                Value::Date(Infinite::NegInfinity),
                Value::Date(Infinite::from_pg_days(-1).expect("in range")),
                Value::Date(Infinite::from_pg_days(0).expect("in range")),
                Value::Date(Infinite::from_pg_days(256).expect("in range")),
                Value::Date(Infinite::Infinity),
            ],
        ),
        (
            TypeOid::Time,
            [0, 1, 256, 86_400_000_000].map(Value::Time).to_vec(),
        ),
        (
            TypeOid::Interval,
            // `30 days` and `1 mon` are one value under the comparator's estimate.
            vec![
                interval(0, 0, -1),
                interval(0, 0, 0),
                interval(0, 30, 0),
                interval(1, 0, 0),
                interval(1, 0, 1),
                interval(-1, 0, 0),
            ],
        ),
        (
            TypeOid::Jsonb,
            (0..12)
                .map(|_| Value::Jsonb(random_jsonb(&mut rng, 2)))
                .chain([
                    Value::Jsonb(JsonbValue::Array(Vec::new())),
                    Value::Jsonb(JsonbValue::Null),
                ])
                .collect(),
        ),
    ]
}

/// Arrays over `elements`: the empty one, every one- and two-element array with a NULL
/// in each position, and a random population of up to four elements.
fn array_population(element_type: TypeOid, elements: &[Value], rng: &mut Rng) -> Vec<Value> {
    let mut with_null: Vec<Value> = elements.to_vec();
    with_null.push(Value::Null);
    let array = |items: Vec<Value>| Value::Array(ArrayValue::new(element_type, items));
    let mut arrays = vec![array(Vec::new())];
    for a in &with_null {
        arrays.push(array(vec![a.clone()]));
        for b in with_null.iter().step_by(3) {
            arrays.push(array(vec![a.clone(), b.clone()]));
        }
    }
    // Every element beside every other in a three-element array's middle: an element's
    // end is only tested by what comes after it.
    for a in &with_null {
        for b in with_null.iter().step_by(2) {
            arrays.push(array(vec![elements[0].clone(), a.clone(), b.clone()]));
        }
    }
    for _ in 0..60 {
        arrays.push(array(
            (0..rng.below(5))
                .map(|_| rng.pick(&with_null).clone())
                .collect(),
        ));
    }
    arrays
}

/// The pairs of `values` whose key bytes order differently from `values_compare`.
fn mismatches(values: &[Value]) -> Vec<String> {
    let keys: Vec<Vec<u8>> = values
        .iter()
        .map(|v| v.encode_index_key().expect("every value has a key"))
        .collect();
    let mut found = Vec::new();
    for (i, a) in values.iter().enumerate() {
        for (j, b) in values.iter().enumerate() {
            let by_key = keys[i].cmp(&keys[j]);
            let by_comparator = values_compare(a, b);
            if by_comparator != Some(by_key) {
                found.push(format!(
                    "{a:?} vs {b:?}: key {by_key:?}, comparator {by_comparator:?}"
                ));
            }
        }
    }
    found
}

#[test]
fn a_jsonb_key_orders_as_the_comparator_does() {
    let found = mismatches(&jsonb_population());
    assert!(
        found.is_empty(),
        "{} mismatches, first: {}",
        found.len(),
        found[0]
    );
}

#[test]
fn an_array_key_orders_as_the_comparator_does() {
    let mut rng = Rng(0xD6E8_FEB8_6659_FD93);
    for (element_type, elements) in element_populations() {
        let arrays = array_population(element_type, &elements, &mut rng);
        let found = mismatches(&arrays);
        assert!(
            found.is_empty(),
            "{element_type:?}[]: {} mismatches, first: {}",
            found.len(),
            found[0]
        );
    }
}

/// The populations hold what each order turns on — or the two tests above pass over a
/// corpus that never asks. Each pair named here is one a wrong encoding answers
/// differently: equal values under two spellings, a prefix, a NULL element, the
/// top-level empty array.
#[test]
fn the_populations_hold_the_deciding_pairs() {
    let jsonb = jsonb_population();
    let equal_pairs = jsonb
        .iter()
        .enumerate()
        .flat_map(|(i, a)| jsonb[i + 1..].iter().map(move |b| (a, b)))
        .filter(|(a, b)| format!("{a:?}") != format!("{b:?}"))
        .filter(|(a, b)| values_compare(a, b) == Some(Ordering::Equal))
        .count();
    assert!(equal_pairs > 0, "no two spellings of one jsonb value");
    let empty = Value::Jsonb(JsonbValue::Array(Vec::new()));
    assert!(jsonb.contains(&empty));
    assert!(
        jsonb
            .iter()
            .any(|v| matches!(v, Value::Jsonb(JsonbValue::Array(items)) if items.len() > 1))
    );
    let mut rng = Rng(1);
    for (element_type, elements) in element_populations() {
        let arrays = array_population(element_type, &elements, &mut rng);
        let has = |want: &dyn Fn(&[Value]) -> bool| {
            arrays
                .iter()
                .any(|v| matches!(v, Value::Array(a) if want(&a.elements)))
        };
        assert!(has(&|e| e.is_empty()), "{element_type:?}[]: no empty array");
        assert!(
            has(&|e| e.len() == 2 && e[0].is_null()),
            "{element_type:?}[]: no leading NULL element"
        );
        assert!(
            has(&|e| e.len() > 2),
            "{element_type:?}[]: no array past two elements"
        );
    }
}
