//! `NUMERIC` column-type integration tests over the embed engine.
//!
//! Exercises the NUMERIC lifecycle end-to-end (DDL → INSERT → SELECT → WHERE →
//! ORDER BY → arithmetic → CAST → typmod → index) without a server. The
//! differential SQL fixtures (`tests/fixtures/types/numeric_*.sql`) cover the same
//! shapes byte-for-byte against a live Postgres; this is the in-process
//! counterpart, and — as with DATE — the place a `Value` wildcard fallthrough (a
//! comparison or index-probe arm the compiler can't force) would surface.

// A test harness: the `rows` helper `.expect()`s and the shape helpers `panic!` so a
// setup failure or an unexpected cell type surfaces loudly right here.
#![allow(clippy::expect_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::{TypeOid, Value};

/// Run one SQL string and return its single result set's rows.
fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(results.len(), 1, "expected one result set for {sql:?}");
    results.remove(0).rows
}

/// The single `text` cell of a one-row, one-column result (via `::text`).
fn one_text(db: &mut Database, sql: &str) -> String {
    let r = rows(db, sql);
    assert_eq!(r.len(), 1, "expected one row for {sql:?}");
    match &r[0][0] {
        Value::Text(s) => s.clone(),
        other => panic!("expected Text, got {other:?} for {sql:?}"),
    }
}

/// The `id` (Int4) column of every row, sorted ascending — the common projection.
fn ids(db: &mut Database, sql: &str) -> Vec<i32> {
    let mut v: Vec<i32> = rows(db, sql)
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("expected Int4 id, got {o:?}"),
        })
        .collect();
    v.sort_unstable();
    v
}

fn seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE prices (id INT PRIMARY KEY, amount NUMERIC)")
        .expect("create");
    db.execute(
        "INSERT INTO prices VALUES (1, 19.99), (2, 9.5), (3, 1234.5678), (4, -0.001), (5, 0)",
    )
    .expect("insert");
    db
}

#[test]
fn numeric_stores_exactly_and_keeps_scale() {
    let mut db = seeded();
    // A decimal literal keeps its exact scale (through the store, not via f64).
    assert_eq!(
        one_text(&mut db, "SELECT amount::text FROM prices WHERE id = 3"),
        "1234.5678"
    );
    // The stored value is a real `Value::Numeric`, not text or float.
    let r = rows(&mut db, "SELECT amount FROM prices WHERE id = 1");
    assert!(matches!(r[0][0], Value::Numeric(_)), "got {:?}", r[0][0]);
    // A trailing-zero literal preserves its display scale (`f64` would drop it).
    assert_eq!(one_text(&mut db, "SELECT 1.50::numeric::text"), "1.50");
}

#[test]
fn numeric_equality_and_range_filter_scan() {
    let mut db = seeded();
    // Equality against a decimal literal (no index on this un-keyed table → scan
    // + filter, exercising `values_equal`'s numeric arm, not the index probe).
    let eq = rows(&mut db, "SELECT id FROM prices WHERE amount = 9.5");
    assert_eq!(eq.len(), 1);
    assert_eq!(eq[0][0], Value::Int4(2));
    // A range against an *integer* literal: numeric-vs-int compares exactly.
    assert_eq!(
        ids(&mut db, "SELECT id FROM prices WHERE amount > 0"),
        vec![1, 2, 3]
    );
    assert_eq!(
        ids(&mut db, "SELECT id FROM prices WHERE amount < 10"),
        vec![2, 4, 5]
    );
}

#[test]
fn numeric_order_by_is_by_value() {
    let mut db = seeded();
    // -0.001 < 0 < 9.5 < 19.99 < 1234.5678 → ids 4,5,2,1,3.
    let ordered: Vec<i32> = rows(&mut db, "SELECT id FROM prices ORDER BY amount")
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(ordered, vec![4, 5, 2, 1, 3]);
}

#[test]
fn numeric_arithmetic_is_exact_with_pg_scales() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (a NUMERIC, b NUMERIC)")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1.50, 2.25)")
        .expect("insert");
    // add/sub scale = max; mul scale = sum; `a + <int>` promotes the int.
    assert_eq!(one_text(&mut db, "SELECT (a + b)::text FROM t"), "3.75");
    assert_eq!(one_text(&mut db, "SELECT (a * b)::text FROM t"), "3.3750");
    assert_eq!(one_text(&mut db, "SELECT (a + 5)::text FROM t"), "6.50");
    // division picks Postgres's result scale (≥16 significant digits).
    assert_eq!(
        one_text(&mut db, "SELECT (10.0 / 3.0)::text"),
        "3.3333333333333333"
    );
    // division by zero is loud.
    let err = db
        .execute("SELECT (1.0 / 0.0)::numeric")
        .expect_err("division by zero must fail loud");
    assert_eq!(err.sqlstate(), "22012");
}

#[test]
fn numeric_casts_round_trip() {
    let mut db = Database::with_btree();
    // text/int → numeric are exact; numeric → int rounds half-away.
    assert_eq!(
        one_text(&mut db, "SELECT '1234.5678'::numeric::text"),
        "1234.5678"
    );
    assert_eq!(one_text(&mut db, "SELECT 42::numeric::text"), "42");
    assert_eq!(one_text(&mut db, "SELECT 2.5::numeric::int8::text"), "3");
    assert_eq!(
        one_text(&mut db, "SELECT (-2.5)::numeric::int8::text"),
        "-3"
    );
}

#[test]
fn numeric_typmod_rounds_and_overflows() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE money (id INT, amount NUMERIC(10, 2))")
        .expect("create");
    // 1.239 rounds to 1.24 on store; an integer value pads to the scale.
    db.execute("INSERT INTO money VALUES (1, 1.239), (2, 100)")
        .expect("insert");
    assert_eq!(
        one_text(&mut db, "SELECT amount::text FROM money WHERE id = 1"),
        "1.24"
    );
    assert_eq!(
        one_text(&mut db, "SELECT amount::text FROM money WHERE id = 2"),
        "100.00"
    );
    // A value whose integer part exceeds precision-scale fails loud (22003).
    let err = db
        .execute("INSERT INTO money VALUES (3, 123456789.0)")
        .expect_err("numeric field overflow must fail loud");
    assert_eq!(err.sqlstate(), "22003");
}

#[test]
fn numeric_aggregates_are_exact() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (amount NUMERIC(12, 2))")
        .expect("create");
    db.execute("INSERT INTO t VALUES (10.50), (20.25), (0.30)")
        .expect("insert");
    // SUM keeps the max input scale exactly.
    assert_eq!(
        one_text(&mut db, "SELECT SUM(amount)::text FROM t"),
        "31.05"
    );
    // The SUM result is a real Numeric value.
    let r = rows(&mut db, "SELECT SUM(amount) FROM t");
    assert!(matches!(r[0][0], Value::Numeric(_)), "got {:?}", r[0][0]);
}

#[test]
fn numeric_is_indexable_and_unique_collapses_scale() {
    let mut db = seeded();
    // The PRIMARY KEY on `amount` (numeric) built an index; an integer-literal
    // range bound must key as numeric (coerced), not int4 — else the upper bound
    // never lines up and out-of-range rows leak.
    assert_eq!(
        ids(
            &mut db,
            "SELECT id FROM prices WHERE amount > -1 AND amount < 500"
        ),
        vec![1, 2, 4, 5]
    );

    // A UNIQUE numeric collapses value-equal-but-scale-differing keys: inserting
    // 1.50 after 1.5 violates the constraint (the order-preserving key collides).
    db.execute("CREATE TABLE u (amount NUMERIC UNIQUE)")
        .expect("create u");
    db.execute("INSERT INTO u VALUES (1.5)").expect("first");
    let err = db
        .execute("INSERT INTO u VALUES (1.50)")
        .expect_err("1.50 equals 1.5 → UNIQUE violation");
    assert_eq!(err.sqlstate(), "23505");
}

#[test]
fn numeric_nan_sorts_high_and_equals_itself() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, a NUMERIC)")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1, 'NaN'), (2, 100), (3, -5)")
        .expect("insert");
    // NaN round-trips and renders as `NaN`.
    assert_eq!(
        one_text(&mut db, "SELECT a::text FROM t WHERE id = 1"),
        "NaN"
    );
    // NaN sorts above every finite value (id 1 last, ascending).
    let ordered: Vec<i32> = rows(&mut db, "SELECT id FROM t ORDER BY a")
        .into_iter()
        .map(|r| match r[0] {
            Value::Int4(n) => n,
            ref o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(ordered, vec![3, 2, 1]);
    // NaN = NaN is TRUE for numeric (unlike float NaN).
    let eq = rows(&mut db, "SELECT id FROM t WHERE a = 'NaN'::numeric");
    assert_eq!(eq.len(), 1);
    assert_eq!(eq[0][0], Value::Int4(1));
}

#[test]
fn numeric_default_resolves() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE d (id INT, amount NUMERIC(10,2) DEFAULT 9.99)")
        .expect("create");
    db.execute("INSERT INTO d (id) VALUES (1)").expect("insert");
    assert_eq!(one_text(&mut db, "SELECT amount::text FROM d"), "9.99");
}

#[test]
fn numeric_math_functions() {
    let mut db = Database::with_btree();
    // abs keeps the display scale; ceil/floor/round/trunc round exactly.
    assert_eq!(one_text(&mut db, "SELECT abs(-1.50)::text"), "1.50");
    assert_eq!(one_text(&mut db, "SELECT ceil(1.1)::text"), "2");
    assert_eq!(one_text(&mut db, "SELECT floor(-1.5)::text"), "-2");
    assert_eq!(one_text(&mut db, "SELECT round(2.567, 2)::text"), "2.57");
    assert_eq!(one_text(&mut db, "SELECT round(-2.5)::text"), "-3"); // half-away
    assert_eq!(one_text(&mut db, "SELECT trunc(1.789, 1)::text"), "1.7");
    // abs preserves an integer argument's type; round(float) → float8.
    assert_eq!(rows(&mut db, "SELECT abs(-7)")[0][0], Value::Int4(7));
    // A NULL argument propagates.
    let r = rows(&mut db, "SELECT round(NULL::numeric, 2)");
    assert!(r[0][0].is_null());
}

#[test]
fn numeric_negative_scale_typmod_rounds_to_power_of_ten() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, a NUMERIC(5,-2))")
        .expect("create");
    // A negative scale rounds to hundreds on store (Postgres numeric(5,-2)).
    db.execute("INSERT INTO t VALUES (1, 12345.6), (2, 999)")
        .expect("insert");
    assert_eq!(
        one_text(&mut db, "SELECT a::text FROM t WHERE id = 1"),
        "12300"
    );
    assert_eq!(
        one_text(&mut db, "SELECT a::text FROM t WHERE id = 2"),
        "1000"
    );
}

#[test]
fn coalesce_unifies_int_literal_to_numeric() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, a NUMERIC(10,2))")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1, 5.50), (2, NULL)")
        .expect("insert");
    // COALESCE(numeric, 0) unifies the int fallback to numeric — the selected `0`
    // must carry the numeric type, not int4 (else a wire type mismatch).
    let r = rows(&mut db, "SELECT COALESCE(a, 0) FROM t ORDER BY id");
    assert!(matches!(r[0][0], Value::Numeric(_)));
    assert!(
        matches!(r[1][0], Value::Numeric(_)),
        "fallback must be numeric"
    );
    assert_eq!(
        one_text(&mut db, "SELECT COALESCE(a, 0)::text FROM t WHERE id = 2"),
        "0"
    );
}

#[test]
fn numeric_array_round_trips() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, ns NUMERIC[])")
        .expect("create");
    db.execute("INSERT INTO t VALUES (1, '{1.5,2.25,-3}')")
        .expect("insert");
    // A `numeric[]` renders as the Postgres `{...}` array literal.
    assert_eq!(one_text(&mut db, "SELECT ns::text FROM t"), "{1.5,2.25,-3}");
    // The element type is carried as Numeric (not Text).
    let r = rows(&mut db, "SELECT ns FROM t");
    match &r[0][0] {
        Value::Array(a) => {
            assert_eq!(a.element_type, TypeOid::Numeric);
            assert!(matches!(a.elements[0], Value::Numeric(_)));
        }
        other => panic!("expected Array, got {other:?}"),
    }
}

/// A `--data-dir` store written before `round(double precision)` rounded a tie to even,
/// and before a float read from text was range-checked, cannot be reopened.
///
/// Both moved what a stored expression evaluates to with the postcard shape unchanged:
/// a unique expression index over `round(f)` that a `PGMU` build filled holding `2.5`
/// would admit `2.4` beside it. The `PGMU` → `PGMV` bump is what refuses such a store,
/// and this asserts it moved — a bump left out of `STALE_MAGICS`, or not applied at all,
/// makes the old file open cleanly.
#[test]
fn a_pre_rint_round_store_magic_is_rejected() {
    use pglet_storage::btree::DoubleMetaStore;
    use pglet_testing::block_io::MemBlockIo;

    let mut bytes = vec![0u8; 2 * 64 * 1024];
    bytes[0..4].copy_from_slice(b"PGMU");
    let err = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes)))
        .expect_err("a PGMU store must not reopen");
    assert!(
        format!("{err:?}").contains("stale on-disk format"),
        "expected a stale-format error, got {err:?}"
    );
}
