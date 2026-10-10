//! An `IN` list is compared at the type 18.6's `transformAExprIn` chooses where every
//! operand's type is certain, at the in-process host.
//!
//! The converted lists are pinned against 18.6 by `tests/fixtures/in_list/`. What sits
//! outside that corpus:
//!
//! - **A list that is not converted where 18.6 converts it.** A fixture is recorded
//!   from 18.6, so a faithful one is red; the item-by-item answers and the reported
//!   parameter types are asserted here, each beside what 18.6 answers (`EDGES.md`
//!   row 280).
//! - **The backends and the block.** A fixture runs on the B+tree inside one
//!   transaction; the same statements are read here on memory storage too, in
//!   autocommit and in a block, a `DELETE` through the list included.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface an unexpected shape loudly
// right here.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pglet::{BoundExecError, Database};
use pglet_sql::types::{TypeOid, Value};

const SEED: [&str; 2] = [
    "CREATE TABLE r (id INT PRIMARY KEY, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, \
     s SMALLINT, t TEXT, v VARCHAR(8), dt DATE, ts TIMESTAMP)",
    "INSERT INTO r VALUES \
     (1, 0.1, 0.1, 0.1, 1, 1, 1, 'a', 'a', '2020-01-01', '2020-01-01 00:00:00'), \
     (2, 1.5, 1.5, 1.5, 2, 2, 2, 'b', 'b', '2020-01-02', '2020-01-02 10:00:00'), \
     (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3, 'c', 'c', \
      '2020-01-03', '2020-01-03 00:00:00')",
];

fn seeded(mut db: Database) -> Database {
    for sql in SEED {
        db.execute(sql).expect("seeds");
    }
    db
}

/// The types `sql`'s parameters are reported at, the statement never run.
fn reported(db: &mut Database, sql: &str) -> Vec<Option<TypeOid>> {
    let mut seen = Vec::new();
    let outcome = db.execute_bound(sql, 0, |types| {
        seen = types.to_vec();
        Err::<Vec<Option<Value>>, ()>(())
    });
    assert!(
        matches!(outcome, Err(BoundExecError::Bind(()))),
        "`{sql}` reaches its bind"
    );
    seen
}

/// The `id`s `sql` answers with `$N` bound to `values`, in order.
fn bound_ids(db: &mut Database, sql: &str, values: Vec<Value>) -> Vec<i32> {
    let result = db
        .execute_bound(sql, 0, |_| {
            Ok::<_, ()>(values.into_iter().map(Some).collect())
        })
        .unwrap_or_else(|_| panic!("`{sql}` answers"));
    ids(&result.rows)
}

fn ids(rows: &[Vec<Value>]) -> Vec<i32> {
    rows.iter()
        .map(|row| match row.first() {
            Some(Value::Int4(id)) => *id,
            other => panic!("an id, not {other:?}"),
        })
        .collect()
}

/// The `id`s the last statement of `sql` answers.
fn answer(db: &mut Database, sql: &str) -> Vec<i32> {
    let results = db.execute(sql).unwrap_or_else(|e| panic!("`{sql}`: {e}"));
    ids(&results.last().expect("a result").rows)
}

/// A `$N` item is reported at the left operand's type: a list holding a parameter is
/// compared item by item, whatever its other items are.
#[test]
fn a_parameter_item_is_reported_at_the_left_operands_type() {
    use TypeOid::{Date, Float4, Float8, Int2, Int4, Int8, Text};
    let mut db = seeded(Database::with_btree());
    for (sql, types) in [
        // As 18.6 reports: no typed item widens the list.
        ("SELECT id FROM r WHERE x IN ($1, $2)", vec![Float4, Float4]),
        ("SELECT id FROM r WHERE x IN ($1, 1.5)", vec![Float4]),
        ("SELECT id FROM r WHERE x IN ($1, NULL)", vec![Float4]),
        ("SELECT id FROM r WHERE x IN ($1)", vec![Float4]),
        ("SELECT id FROM r WHERE i IN ($1, $2)", vec![Int4, Int4]),
        ("SELECT id FROM r WHERE b IN ($1, 2)", vec![Int8]),
        ("SELECT id FROM r WHERE s IN ($1, 3::smallint)", vec![Int2]),
        ("SELECT id FROM r WHERE d IN ($1, 2.5::real)", vec![Float8]),
        ("SELECT id FROM r WHERE t IN ($1, 'b')", vec![Text]),
        (
            "SELECT id FROM r WHERE dt IN ($1, '2020-01-02')",
            vec![Date],
        ),
        ("SELECT id FROM r WHERE x IN ($1, d)", vec![Float4]),
        (
            "SELECT id FROM r WHERE i = $1 AND i IN ($1, 2.5)",
            vec![Int4],
        ),
        (
            "SELECT id FROM r WHERE d = $1 AND x IN ($1, 0.1)",
            vec![Float8],
        ),
        // Not as 18.6 reports — it reports the list's common type: `numeric`, `real`,
        // `integer`, `float8`, `float8`.
        ("SELECT id FROM r WHERE i IN ($1, 2.5)", vec![Int4]),
        ("SELECT id FROM r WHERE i IN ($1, 2.5::real)", vec![Int4]),
        ("SELECT id FROM r WHERE s IN ($1, 3)", vec![Int2]),
        (
            "SELECT id FROM r WHERE x IN ($1, 1.5::float8)",
            vec![Float4],
        ),
        (
            "SELECT id FROM r WHERE d = $2 AND x IN ($1, $2)",
            vec![Float4, Float8],
        ),
    ] {
        let expected: Vec<Option<TypeOid>> = types.into_iter().map(Some).collect();
        assert_eq!(reported(&mut db, sql), expected, "{sql}");
    }
}

/// A list holding a parameter, at any depth of an item, is compared item by item: an
/// earlier use may have fixed the parameter's type, which is not known where the list
/// is planned. Behind a `float8` use 18.6's list is `float8` and it answers as here;
/// with no such use its list is `real` and it finds the stored `0.1`.
#[test]
fn a_list_holding_a_parameter_is_compared_item_by_item() {
    let mut db = seeded(Database::with_btree());
    for (sql, value, rows, as_18_6) in [
        (
            "SELECT id FROM r WHERE d < $1 AND x IN ($1, 0.1) ORDER BY id",
            Value::Float8(5.0),
            vec![],
            true,
        ),
        (
            "SELECT id FROM r WHERE d < $1 AND x NOT IN ($1, 0.1) ORDER BY id",
            Value::Float8(5.0),
            vec![1, 2],
            true,
        ),
        (
            "SELECT id FROM r WHERE d < $1 AND x IN ($1 + 0, 0.1) ORDER BY id",
            Value::Float8(5.0),
            vec![],
            true,
        ),
        (
            "SELECT id FROM r WHERE x IN ($1, 0.1::real) ORDER BY id",
            Value::Float4(7.0),
            vec![1],
            true,
        ),
        // 18.6 finds row `1` in each of these.
        (
            "SELECT id FROM r WHERE x IN ($1, 0.1) ORDER BY id",
            Value::Float4(7.0),
            vec![],
            false,
        ),
        (
            "SELECT id FROM r WHERE x IN ($1::real, 0.1) ORDER BY id",
            Value::Float4(7.0),
            vec![],
            false,
        ),
        (
            "SELECT id FROM r WHERE i < $1 AND x IN ($1 + 0, 0.1) ORDER BY id",
            Value::Int4(5),
            vec![],
            false,
        ),
        (
            "SELECT id FROM r WHERE i < $1 AND x IN (COALESCE($1, 1), 0.1) ORDER BY id",
            Value::Int4(5),
            vec![],
            false,
        ),
    ] {
        assert_eq!(
            bound_ids(&mut db, sql, vec![value]),
            rows,
            "{sql} (as 18.6 answers: {as_18_6})"
        );
    }
}

/// A list of constants is converted only under a plain column of a base table at its
/// own query level. Under any other left operand, and beside any item that is not a
/// written constant, it is compared item by item — which 18.6 answers differently in
/// each of these (its answer beside each).
#[test]
fn a_list_outside_the_certain_shapes_is_compared_item_by_item() {
    let mut db = seeded(Database::with_btree());
    for (sql, rows) in [
        // 18.6: `1`, `2` — a computed left operand.
        (
            "SELECT id FROM r WHERE abs(x) IN (0.1, 1.5) ORDER BY id",
            vec![2],
        ),
        // 18.6: `1`, `2` — a computed item.
        (
            "SELECT id FROM r WHERE x IN (0.05 + 0.05, 1.5) ORDER BY id",
            vec![2],
        ),
        // 18.6: `1`, `2` — a derived table's column, and a `WITH` query's.
        (
            "SELECT id FROM (SELECT id, x AS y FROM r) v WHERE y IN (0.1, 1.5) ORDER BY id",
            vec![2],
        ),
        (
            "WITH c AS (SELECT id, x AS y FROM r) SELECT id FROM c WHERE y IN (0.1, 1.5) \
             ORDER BY id",
            vec![2],
        ),
        // 18.6: `1`, `2` — the enclosing query's column.
        (
            "SELECT o.id FROM r o WHERE EXISTS \
             (SELECT 1 FROM r q WHERE q.id = 2 AND o.x IN (0.1, 1.5)) ORDER BY o.id",
            vec![2],
        ),
    ] {
        assert_eq!(answer(&mut db, sql), rows, "{sql}");
    }
    // 18.6 reads the strings as `integer` and answers `t`.
    let refused = db.execute("SELECT 1 IN ('1', '2')").expect_err("refused");
    assert_eq!(refused.sqlstate(), "42883");
}

/// The list is typed where the statement is planned, so the storage backend and the
/// transaction block see the same answers — and a `DELETE` through the list removes
/// the same rows.
#[test]
fn both_backends_answer_alike_in_autocommit_and_in_a_block() {
    for memory in [false, true] {
        for block in [false, true] {
            let mut db = seeded(if memory {
                Database::in_memory()
            } else {
                Database::with_btree()
            });
            let context = format!("memory storage {memory}, block {block}");
            if block {
                db.execute("BEGIN").expect("begins");
            }
            for (sql, rows) in [
                (
                    "SELECT id FROM r WHERE x IN (0.1, 1.5) ORDER BY id",
                    vec![1, 2],
                ),
                (
                    "SELECT id FROM r WHERE x NOT IN (0.1, 1.5) ORDER BY id",
                    vec![3],
                ),
                (
                    "SELECT id FROM r WHERE x IN (16777217, 5) ORDER BY id",
                    vec![3],
                ),
                ("SELECT id FROM r WHERE x IN (0.1) ORDER BY id", vec![]),
                (
                    "SELECT id FROM r WHERE id IN (1, 2.0, 7) ORDER BY id",
                    vec![1, 2],
                ),
                (
                    "DELETE FROM r WHERE x NOT IN (0.1, 1.5); SELECT id FROM r ORDER BY id",
                    vec![1, 2],
                ),
                (
                    "DELETE FROM r WHERE x IN (0.1, 7); SELECT id FROM r ORDER BY id",
                    vec![2],
                ),
            ] {
                assert_eq!(answer(&mut db, sql), rows, "{context}: {sql}");
            }
            if block {
                db.execute("COMMIT").expect("commits");
                assert_eq!(answer(&mut db, "SELECT id FROM r"), vec![2], "{context}");
            }
        }
    }
}
