//! A FROM-clause `generate_series` at the in-process host, where the differential
//! fixtures (`tests/fixtures/generate_series/`) cannot reach.
//!
//! - **The row bound** is pglet's own: a series longer than `MAX_SERIES_ROWS` is refused
//!   `54000` from its evaluated arguments before a row is built, and the length is
//!   computed without overflowing at the integer types' edges. No fixture asks the
//!   oracle for such a series — it materializes a FROM function too, spilling to disk.
//! - **The refusals 18.6 does not share** are pinned to their SQLSTATE here, where an
//!   `@expect-error` fixture over a statement 18.6 accepts passes on any error.
//! - **The declared column type** of each overload, read off the executor's result.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface an unexpected verdict shape
// loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::cell::RefCell;

use pglet::{BoundExecError, Database};
use pglet_sql::executor::QueryResult;
use pglet_sql::types::{TypeOid, Value};

fn one(db: &mut Database, sql: &str) -> Result<QueryResult, String> {
    db.execute(sql)
        .map(|mut results| results.pop().expect("one statement"))
        .map_err(|e| e.sqlstate().to_string())
}

fn state(db: &mut Database, sql: &str) -> String {
    match one(db, sql) {
        Ok(result) => panic!("{sql}: answered {:?}", result.rows.len()),
        Err(code) => code,
    }
}

#[test]
fn a_series_at_the_bound_answers_and_one_past_it_is_refused() {
    let mut db = Database::with_btree();
    let at = one(
        &mut db,
        "SELECT count(*), min(g), max(g) FROM generate_series(1, 1000000) g",
    )
    .expect("at the bound");
    assert_eq!(
        at.rows,
        vec![vec![
            Value::Int8(1_000_000),
            Value::Int4(1),
            Value::Int4(1_000_000)
        ]]
    );
    let down = one(
        &mut db,
        "SELECT count(*) FROM generate_series(1000000, 1, -1)",
    )
    .expect("at the bound, descending");
    assert_eq!(down.rows, vec![vec![Value::Int8(1_000_000)]]);
    for sql in [
        "SELECT count(*) FROM generate_series(1, 1000001)",
        "SELECT count(*) FROM generate_series(0, 1000000)",
        "SELECT count(*) FROM generate_series(1000001, 1, -1)",
        "SELECT count(*) FROM generate_series(1::bigint, 1000001)",
    ] {
        assert_eq!(state(&mut db, sql), "54000", "{sql}");
    }
}

/// A length the arguments imply but no allocation could hold is refused as fast as one
/// past the bound, and a length computed by subtracting at the types' edges does not
/// overflow into a small or negative count that answers.
#[test]
fn a_series_too_long_to_allocate_is_refused_before_a_row_is_built() {
    let mut db = Database::with_btree();
    for sql in [
        "SELECT count(*) FROM generate_series(1, 2147483647)",
        "SELECT count(*) FROM generate_series(-2147483648, 2147483647)",
        "SELECT count(*) FROM generate_series(2147483647, -2147483648, -1)",
        "SELECT count(*) FROM generate_series(1::bigint, 9223372036854775807)",
        "SELECT count(*) FROM generate_series(-9223372036854775807, 9223372036854775807)",
        "SELECT count(*) FROM generate_series(9223372036854775807, -9223372036854775807, -1)",
        "SELECT count(*) FROM generate_series(-9223372036854775807 - 1, 9223372036854775807, 2)",
        "SELECT * FROM generate_series(1, 2147483647) LIMIT 1",
    ] {
        assert_eq!(state(&mut db, sql), "54000", "{sql}");
    }
    // The bound is the count of values, not the span: a stride over the whole type is
    // three rows.
    let stride = one(
        &mut db,
        "SELECT g FROM generate_series(-9223372036854775807, 9223372036854775807, \
         9223372036854775807) g",
    )
    .expect("a long stride");
    assert_eq!(
        stride.rows,
        vec![
            vec![Value::Int8(-9_223_372_036_854_775_807)],
            vec![Value::Int8(0)],
            vec![Value::Int8(9_223_372_036_854_775_807)],
        ]
    );
}

#[test]
fn each_overload_declares_its_type() {
    let mut db = Database::with_btree();
    for (sql, oid, first) in [
        (
            "SELECT * FROM generate_series(1, 2)",
            TypeOid::Int4,
            Value::Int4(1),
        ),
        (
            "SELECT * FROM generate_series(1::smallint, 2)",
            TypeOid::Int4,
            Value::Int4(1),
        ),
        (
            "SELECT * FROM generate_series(1, '2')",
            TypeOid::Int4,
            Value::Int4(1),
        ),
        (
            "SELECT * FROM generate_series(1, 2, 1::bigint)",
            TypeOid::Int8,
            Value::Int8(1),
        ),
        (
            "SELECT * FROM generate_series(1::bigint, '2')",
            TypeOid::Int8,
            Value::Int8(1),
        ),
        (
            "SELECT * FROM generate_series(2147483647, 2147483648)",
            TypeOid::Int8,
            Value::Int8(2_147_483_647),
        ),
    ] {
        let result = one(&mut db, sql).expect(sql);
        let columns = result.columns();
        assert_eq!(columns.len(), 1, "{sql}");
        assert_eq!(columns[0].name, "generate_series", "{sql}");
        assert_eq!(columns[0].type_oid, oid, "{sql}");
        assert_eq!(result.rows[0], vec![first], "{sql}");
    }
}

/// What 18.6 answers and pglet declines, each pinned to the code it declines with.
#[test]
fn what_is_declined_is_feature_not_supported() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE gs_t (id INT, n INT)",
        "INSERT INTO gs_t VALUES (1, 2), (2, 1)",
        "CREATE TABLE gs_i (n INT)",
        "INSERT INTO gs_i VALUES (1), (2)",
    ] {
        db.execute(sql).expect(sql);
    }
    for sql in [
        // A column of an earlier FROM item, bare and qualified.
        "SELECT * FROM gs_t, generate_series(1, n) g",
        "SELECT * FROM gs_t, generate_series(1, gs_t.n) g",
        "SELECT * FROM gs_t JOIN generate_series(1, gs_t.n) g ON true",
        // An enclosing query's column — and the name both levels carry, whose
        // enclosing-row reading would answer other counts silently.
        "SELECT id, (SELECT count(*) FROM generate_series(1, gs_t.n)) FROM gs_t",
        "SELECT id, (SELECT count(*) FROM gs_i, generate_series(1, n) g) FROM gs_t",
        "SELECT id FROM gs_t WHERE EXISTS (SELECT 1 FROM generate_series(1, n) g)",
        // A column under a cast, which would otherwise type the argument.
        "SELECT * FROM gs_t, generate_series(1, n::int8) g",
        // A sub-plan.
        "SELECT * FROM generate_series(1, (SELECT 2))",
        "SELECT * FROM generate_series(1, (SELECT max(id) FROM gs_t)::int)",
        // The overloads not modeled.
        "SELECT * FROM generate_series(1, 3.0)",
        "SELECT * FROM generate_series(1::numeric, 3)",
        "SELECT * FROM generate_series('2024-01-01'::timestamp, '2024-01-02', '1 day')",
        "SELECT * FROM generate_series(now(), now(), '1 day')",
        "SELECT * FROM generate_series(now(), now(), '1 day', 'UTC')",
        "SELECT * FROM generate_series('2024-01-01'::date, '2024-01-02', '1 day')",
        // An argument whose type is not known as written: an operator over a `$N`.
        "SELECT * FROM generate_series(1, $1 + 1)",
        // The `bigint` literal 18.6 reads as one constant, which pglet reads as a
        // negated `numeric`.
        "SELECT * FROM generate_series(-9223372036854775808, -9223372036854775807)",
    ] {
        assert_eq!(state(&mut db, sql), "0A000", "{sql}");
    }
}

/// A bare `$N` takes the resolved overload's type through `execute_bound` — the wasm
/// hosts' `queryParams` door — and the inference is the binding's.
#[test]
fn a_bound_parameter_takes_the_overload_type() {
    let mut db = Database::with_btree();
    for (sql, oid, value, rows) in [
        (
            "SELECT * FROM generate_series(1, $1)",
            TypeOid::Int4,
            Value::Int4(3),
            3,
        ),
        (
            "SELECT * FROM generate_series($1, 1, -1)",
            TypeOid::Int4,
            Value::Int4(2),
            2,
        ),
        (
            "SELECT * FROM generate_series(1::bigint, $1)",
            TypeOid::Int8,
            Value::Int8(4),
            4,
        ),
    ] {
        let seen = RefCell::new(Vec::new());
        let result = db
            .execute_bound(sql, 0, |oids| {
                seen.replace(oids.to_vec());
                Ok::<_, ()>(vec![Some(value.clone())])
            })
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
        assert_eq!(seen.into_inner(), vec![Some(oid)], "{sql}");
        assert_eq!(result.rows.len(), rows, "{sql}");
        assert_eq!(result.columns()[0].type_oid, oid, "{sql}");
    }
    // Two bare parameters resolve no signature, before any value is bound.
    let bound = std::cell::Cell::new(false);
    match db.execute_bound("SELECT * FROM generate_series($1, $2)", 0, |_| {
        bound.set(true);
        Ok::<_, ()>(vec![Some(Value::Int4(1)), Some(Value::Int4(2))])
    }) {
        Err(BoundExecError::Engine(e)) => assert_eq!(e.sqlstate(), "42725"),
        other => panic!("answered {other:?}"),
    }
    assert!(!bound.get(), "refused before the binding");
}

/// Two FROM items sharing an effective name are 18.6's `42712`, a gate pglet does not
/// have (`EDGES.md` row 322). A bare `*` over them must not answer: a function item is
/// qualified by its name only where no other item at its level shares it, so both
/// items' columns stay unqualified and are `42702` — qualified by the shared name, the
/// first item answered for both. Each shape is pinned here because an `@expect-error`
/// fixture compares the code with 18.6's `42712` and is red on any `42702`.
#[test]
fn a_bare_star_over_items_sharing_a_name_does_not_answer() {
    let mut db = Database::with_btree();
    for (sql, code) in [
        (
            "SELECT * FROM generate_series(1, 2), generate_series(5, 6)",
            "42702",
        ),
        (
            "SELECT * FROM generate_series(1, 2) t, generate_series(3, 4) t",
            "42702",
        ),
        (
            "SELECT *, 1 FROM generate_series(1, 2) a, generate_series(1, 2) a",
            "42702",
        ),
        (
            "SELECT * FROM generate_series(1, 2) a JOIN generate_series(5, 6) a ON true",
            "42702",
        ),
        (
            "SELECT * FROM unnest(ARRAY[1, 2]) u, unnest(ARRAY[7, 8]) AS u",
            "42702",
        ),
        (
            "SELECT * FROM unnest(ARRAY[1, 2]), unnest(ARRAY[5, 6])",
            "42702",
        ),
        (
            "SELECT * FROM unnest(ARRAY[1, 2]) a JOIN unnest(ARRAY[7, 8]) a ON true",
            "42702",
        ),
        (
            "SELECT * FROM (SELECT 1 AS g) j, unnest(ARRAY[7, 8]) AS j(g)",
            "42702",
        ),
        (
            "WITH c AS (SELECT 5 AS u) SELECT * FROM c u, unnest(ARRAY[7, 8]) u",
            "42702",
        ),
        (
            "SELECT * FROM (SELECT 7 AS t) t, generate_series(1, 2) t",
            "42702",
        ),
        // The qualified wildcard reads the `nonbase` map, which a series sharing its
        // name stays out of.
        (
            "SELECT a.* FROM generate_series(1, 2) a, generate_series(5, 6) a",
            "0A000",
        ),
    ] {
        assert_eq!(state(&mut db, sql), code, "{sql}");
    }
    // The accept side: uniquely named items expand, each qualified by its own name.
    let unique = one(
        &mut db,
        "SELECT * FROM generate_series(1, 2) a, generate_series(5, 6) b ORDER BY 1, 2",
    )
    .expect("distinct names");
    assert_eq!(
        unique.rows,
        vec![
            vec![Value::Int4(1), Value::Int4(5)],
            vec![Value::Int4(1), Value::Int4(6)],
            vec![Value::Int4(2), Value::Int4(5)],
            vec![Value::Int4(2), Value::Int4(6)],
        ]
    );
    let qualified = one(
        &mut db,
        "SELECT a.*, b.* FROM generate_series(1, 2) a, generate_series(5, 5) b ORDER BY 1",
    )
    .expect("distinct names, qualified");
    assert_eq!(
        qualified.rows,
        vec![
            vec![Value::Int4(1), Value::Int4(5)],
            vec![Value::Int4(2), Value::Int4(5)]
        ]
    );
}
