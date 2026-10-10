//! An index range read answers no row whose key is NULL, on each of the embed's stores.
//!
//! A `NULLS NOT DISTINCT` index holds an entry for a NULL key, and a comparison is not
//! true of a NULL, so a range the planner lifts into such an index must leave that entry
//! out — in autocommit, inside a block over the block's own pending rows, bound through
//! `execute_bound`, and from a file store reopened. The differential fixtures
//! (`index/range_null_key_*`) hold the answers against 18.6 on the native server's
//! B+tree; these hold them on the stores and doors a fixture does not reach, and
//! [`plans`] holds that the read still goes through the index — an answer cannot show
//! that.

#![allow(clippy::unwrap_used, clippy::panic)]

use crate::common::ScratchDir;
use pglet::Database;
use pglet_sql::types::Value;

/// `sql`'s last result as its command tag and rows, or `ERR <SQLSTATE>`.
fn answer(db: &mut Database, sql: &str) -> String {
    match db.execute(sql) {
        Ok(results) => results
            .last()
            .map_or_else(String::new, |r| format!("{} {:?}", r.command_tag, r.rows)),
        Err(e) => format!("ERR {}", e.sqlstate()),
    }
}

/// The text column of `sql`'s rows.
fn labels(db: &mut Database, sql: &str) -> Vec<String> {
    let results = db.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    results
        .last()
        .unwrap_or_else(|| panic!("{sql}: no result"))
        .rows
        .iter()
        .map(|row| match &row[0] {
            Value::Text(s) => s.clone(),
            other => panic!("{sql}: expected text, got {other:?}"),
        })
        .collect()
}

/// `u` under a unique `NULLS NOT DISTINCT` index and `n` under a non-unique one, each
/// holding NULL keys.
fn seed(db: &mut Database) {
    for sql in [
        "CREATE TABLE r (u INT, n INT, v TEXT)",
        "CREATE UNIQUE INDEX r_u ON r (u) NULLS NOT DISTINCT",
        "CREATE INDEX r_n ON r (n) NULLS NOT DISTINCT",
        "INSERT INTO r VALUES (NULL, NULL, 'null'), (1, NULL, 'one'), (10, 3, 'ten'), (-4, 1, 'neg')",
    ] {
        db.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
}

/// Range reads over [`seed`]'s rows and the rows each answers, in order.
const READS: &[(&str, &[&str])] = &[
    ("SELECT v FROM r WHERE u < 5 ORDER BY v", &["neg", "one"]),
    ("SELECT v FROM r WHERE u <= 1 ORDER BY v", &["neg", "one"]),
    ("SELECT v FROM r WHERE 5 > u ORDER BY v", &["neg", "one"]),
    ("SELECT v FROM r WHERE u < -4", &[]),
    ("SELECT v FROM r WHERE u <= -4", &["neg"]),
    (
        "SELECT v FROM r WHERE u < 100 ORDER BY u",
        &["neg", "one", "ten"],
    ),
    (
        "SELECT v FROM r WHERE u < 100 ORDER BY u DESC LIMIT 1",
        &["ten"],
    ),
    (
        "SELECT v FROM r WHERE u < 5 ORDER BY u NULLS FIRST LIMIT 1",
        &["neg"],
    ),
    (
        "SELECT v FROM r WHERE u < 5 ORDER BY v LIMIT 5",
        &["neg", "one"],
    ),
    ("SELECT v FROM r WHERE u > 0 AND u < 5", &["one"]),
    ("SELECT v FROM r WHERE u > 5", &["ten"]),
    ("SELECT v FROM r WHERE n < 5 ORDER BY v", &["neg", "ten"]),
    ("SELECT v FROM r WHERE n <= 1", &["neg"]),
    ("SELECT v FROM r WHERE n < 1", &[]),
    // The rows a range leaves out are still the table's.
    ("SELECT v FROM r WHERE u IS NULL", &["null"]),
    (
        "SELECT v FROM r WHERE n IS NULL ORDER BY v",
        &["null", "one"],
    ),
    (
        "SELECT v FROM r WHERE u < 5 OR u IS NULL ORDER BY v",
        &["neg", "null", "one"],
    ),
    (
        "SELECT v FROM r ORDER BY u, v",
        &["neg", "one", "ten", "null"],
    ),
    (
        "SELECT v FROM r ORDER BY u DESC, v",
        &["null", "ten", "one", "neg"],
    ),
];

fn assert_reads(db: &mut Database, label: &str) {
    for (sql, want) in READS {
        assert_eq!(labels(db, sql), *want, "{label}: {sql}");
    }
    assert_eq!(
        answer(db, "SELECT count(*) FROM r WHERE u < 5"),
        "SELECT 1 [[Int8(2)]]",
        "{label}"
    );
    assert_eq!(
        answer(db, "SELECT EXISTS (SELECT 1 FROM r WHERE u < -4)"),
        "SELECT 1 [[Bool(false)]]",
        "{label}"
    );
}

fn in_memory_stores() -> Vec<(&'static str, Database)> {
    vec![
        ("memory", Database::in_memory()),
        ("btree", Database::with_btree()),
        (
            "btree, one connection",
            Database::with_btree_for_one_connection(0),
        ),
    ]
}

#[test]
fn a_range_read_answers_no_null_keyed_row_in_autocommit() {
    for (store, mut db) in in_memory_stores() {
        seed(&mut db);
        assert_reads(&mut db, store);
    }
}

#[test]
fn a_range_read_answers_no_null_keyed_row_bound() {
    for (store, mut db) in in_memory_stores() {
        seed(&mut db);
        // Twice: the second run takes the plan the first one cached.
        for round in 0..2 {
            for (sql, bound, want) in [
                (
                    "SELECT v FROM r WHERE u < $1 ORDER BY v",
                    Some(5),
                    vec!["neg", "one"],
                ),
                (
                    "SELECT v FROM r WHERE u <= $1 ORDER BY v",
                    Some(-4),
                    vec!["neg"],
                ),
                (
                    "SELECT v FROM r WHERE n < $1 ORDER BY v",
                    Some(5),
                    vec!["neg", "ten"],
                ),
                // A NULL bound compares with nothing.
                ("SELECT v FROM r WHERE u < $1 ORDER BY v", None, vec![]),
            ] {
                let result = db
                    .execute_bound(sql, 0, |_| Ok::<_, ()>(vec![bound.map(Value::Int4)]))
                    .unwrap_or_else(|e| panic!("{store}: {sql}: {e:?}"));
                let got: Vec<&Value> = result.rows.iter().map(|row| &row[0]).collect();
                let want: Vec<Value> = want.iter().map(|s| Value::Text((*s).into())).collect();
                assert_eq!(
                    got,
                    want.iter().collect::<Vec<_>>(),
                    "{store}, round {round}: {sql} bound {bound:?}"
                );
            }
        }
    }
}

/// `(column type, rows, the bound as SQL, the labels below the bound)`: for each key type,
/// a NULL key, the type's least value or one near it, and a bound above some of the rows.
/// The bound is evaluated to a `Value` and bound as `$1`, a spelling that reaches the
/// index for each of them — a fixture's `@param` has no spelling for some.
const TYPED_KEYS: &[(&str, &str, &str, &[&str])] = &[
    (
        "SMALLINT",
        "(NULL, 'null'), (-32768, 'least'), (7, 'seven')",
        "0::smallint",
        &["least"],
    ),
    (
        "INT",
        "(NULL, 'null'), (-2147483648, 'least'), (7, 'seven')",
        "0",
        &["least"],
    ),
    (
        "BIGINT",
        "(NULL, 'null'), (-9223372036854775808, 'least'), (7, 'seven')",
        "0::bigint",
        &["least"],
    ),
    (
        "REAL",
        "(NULL, 'null'), ('-Infinity', 'least'), (0, 'zero'), ('NaN', 'nan')",
        "'NaN'::real",
        &["least", "zero"],
    ),
    (
        "DOUBLE PRECISION",
        "(NULL, 'null'), ('-Infinity', 'least'), (0, 'zero'), ('NaN', 'nan')",
        "'NaN'::float8",
        &["least", "zero"],
    ),
    (
        "NUMERIC",
        "(NULL, 'null'), (-1e30, 'least'), (0, 'zero'), (2.5, 'more')",
        "1::numeric",
        &["least", "zero"],
    ),
    (
        "TEXT",
        "(NULL, 'null'), ('', 'least'), ('a', 'a'), ('b', 'b')",
        "'b'::text",
        &["a", "least"],
    ),
    (
        "VARCHAR(5)",
        "(NULL, 'null'), ('', 'least'), ('b', 'b')",
        "'b'::varchar",
        &["least"],
    ),
    (
        "BYTEA",
        "(NULL, 'null'), ('\\x', 'least'), ('\\x00', 'zero byte'), ('\\x01', 'one')",
        "'\\x01'::bytea",
        &["least", "zero byte"],
    ),
    (
        "BOOLEAN",
        "(NULL, 'null'), (false, 'least'), (true, 'yes')",
        "true",
        &["least"],
    ),
    (
        "DATE",
        "(NULL, 'null'), ('-infinity', 'least'), ('2024-01-01', 'new year'), ('2025-01-01', 'later')",
        "DATE '2024-06-01'",
        &["least", "new year"],
    ),
    (
        "TIME",
        "(NULL, 'null'), ('00:00:00', 'least'), ('12:00:00', 'noon')",
        "TIME '06:00:00'",
        &["least"],
    ),
    (
        "TIMESTAMP",
        "(NULL, 'null'), ('-infinity', 'least'), ('2024-01-01 00:00:00', 'new year'), ('2025-01-01 00:00:00', 'later')",
        "TIMESTAMP '2024-06-01 00:00:00'",
        &["least", "new year"],
    ),
    (
        "TIMESTAMPTZ",
        "(NULL, 'null'), ('-infinity', 'least'), ('2024-01-01 00:00:00+00', 'new year')",
        "TIMESTAMPTZ '2024-06-01 00:00:00+00'",
        &["least", "new year"],
    ),
    (
        "INTERVAL",
        "(NULL, 'null'), ('-1 day', 'yesterday'), ('1 hour', 'hour')",
        "INTERVAL '1 minute'",
        &["yesterday"],
    ),
    (
        "UUID",
        "(NULL, 'null'), ('00000000-0000-0000-0000-000000000000', 'least'), ('ffffffff-ffff-ffff-ffff-ffffffffffff', 'last')",
        "'11111111-1111-1111-1111-111111111111'::uuid",
        &["least"],
    ),
    (
        "JSONB",
        "(NULL, 'null'), ('[]', 'least'), ('null', 'json null'), ('1', 'one'), ('2', 'two')",
        "'2'::jsonb",
        &["json null", "least", "one"],
    ),
    (
        "INT[]",
        "(NULL, 'null'), ('{}', 'least'), ('{1}', 'one'), ('{5}', 'five')",
        "'{5}'::int[]",
        &["least", "one"],
    ),
    (
        "TEXT[]",
        "(NULL, 'null'), ('{}', 'least'), ('{\"\"}', 'empty string'), ('{b}', 'b')",
        "'{b}'::text[]",
        &["empty string", "least"],
    ),
];

#[test]
fn a_range_read_answers_no_null_keyed_row_for_each_key_type() {
    // Every case is run before any is judged, so a failure names each type it reaches.
    let mut wrong: Vec<String> = Vec::new();
    for (store, mut db) in in_memory_stores() {
        for (index, (column_type, rows, bound, want)) in TYPED_KEYS.iter().enumerate() {
            let label = format!("{store}, {column_type}");
            let table = format!("k{index}");
            // Unique, so a container key is a B-tree's: a non-unique index over one is GIN.
            for sql in [
                format!("CREATE TABLE {table} (c {column_type}, v TEXT)"),
                format!("CREATE UNIQUE INDEX {table}_c ON {table} (c) NULLS NOT DISTINCT"),
                format!("INSERT INTO {table} VALUES {rows}"),
            ] {
                db.execute(&sql)
                    .unwrap_or_else(|e| panic!("{label}: {sql}: {e}"));
            }
            let bound_value = db
                .execute(&format!("SELECT {bound}"))
                .unwrap_or_else(|e| panic!("{label}: {bound}: {e}"))
                .remove(0)
                .rows
                .remove(0)
                .remove(0);
            let sql = format!("SELECT v FROM {table} WHERE c < $1 ORDER BY v");
            #[cfg(feature = "index_acceleration")]
            assert!(
                plans::reads_through_an_index_range_scan(&db, &sql),
                "{label}: the range is read through the index"
            );
            for inside_a_block in [false, true] {
                if inside_a_block {
                    db.execute("BEGIN").unwrap();
                }
                let result = db
                    .execute_bound(&sql, 0, |_| Ok::<_, ()>(vec![Some(bound_value.clone())]))
                    .unwrap_or_else(|e| panic!("{label}: {sql}: {e:?}"));
                let got: Vec<String> = result
                    .rows
                    .iter()
                    .map(|row| match &row[0] {
                        Value::Text(s) => s.clone(),
                        other => panic!("{label}: expected text, got {other:?}"),
                    })
                    .collect();
                if got != *want {
                    wrong.push(format!(
                        "{label}, inside a block: {inside_a_block}: {got:?}, expected {want:?}"
                    ));
                }
                if inside_a_block {
                    db.execute("ROLLBACK").unwrap();
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_range_read_answers_no_null_keyed_row_inside_a_block() {
    for (store, mut db) in in_memory_stores() {
        seed(&mut db);
        db.execute("BEGIN").unwrap();
        // Before the block writes, over the committed NULL-keyed rows.
        assert_reads(&mut db, &format!("{store}, block before a write"));
        // The block's own pending rows: a NULL key under each index, and a present one.
        db.execute("INSERT INTO r VALUES (NULL, NULL, 'p-null'), (2, 0, 'p-two')")
            .unwrap_err();
        assert_eq!(
            answer(&mut db, "ROLLBACK"),
            "ROLLBACK []",
            "{store}: a second NULL under the unique index aborts the block"
        );
        db.execute("BEGIN").unwrap();
        db.execute("DELETE FROM r WHERE u IS NULL").unwrap();
        db.execute("INSERT INTO r VALUES (NULL, NULL, 'p-null'), (2, 0, 'p-two')")
            .unwrap();
        for (sql, want) in [
            (
                "SELECT v FROM r WHERE u < 5 ORDER BY v",
                vec!["neg", "one", "p-two"],
            ),
            (
                "SELECT v FROM r WHERE u < 5 ORDER BY u DESC LIMIT 1",
                vec!["p-two"],
            ),
            (
                "SELECT v FROM r WHERE n < 5 ORDER BY v",
                vec!["neg", "p-two", "ten"],
            ),
            ("SELECT v FROM r WHERE n <= 0", vec!["p-two"]),
            ("SELECT v FROM r WHERE u IS NULL", vec!["p-null"]),
        ] {
            assert_eq!(labels(&mut db, sql), want, "{store}, pending rows: {sql}");
        }
        // A row moved onto a NULL key leaves the range; one moved off enters it.
        db.execute("UPDATE r SET n = NULL WHERE v = 'neg'").unwrap();
        db.execute("UPDATE r SET n = 4 WHERE v = 'one'").unwrap();
        assert_eq!(
            labels(&mut db, "SELECT v FROM r WHERE n < 5 ORDER BY v"),
            vec!["one", "p-two", "ten"],
            "{store}: moved keys"
        );
        db.execute("COMMIT").unwrap();
        assert_eq!(
            labels(&mut db, "SELECT v FROM r WHERE n < 5 ORDER BY v"),
            vec!["one", "p-two", "ten"],
            "{store}: after the commit"
        );
        assert_eq!(
            labels(&mut db, "SELECT v FROM r WHERE u < 5 ORDER BY v"),
            vec!["neg", "one", "p-two"],
            "{store}: after the commit"
        );
    }
}

#[test]
fn a_range_read_writes_no_null_keyed_row() {
    // The range inside a write's source or subquery: a NULL-keyed row it answered would
    // be stored, removed or rewritten.
    for (store, mut db) in in_memory_stores() {
        seed(&mut db);
        db.execute("CREATE TABLE sink (v TEXT)").unwrap();
        assert_eq!(
            answer(&mut db, "INSERT INTO sink SELECT v FROM r WHERE u < 5"),
            "INSERT 0 2 []",
            "{store}"
        );
        assert_eq!(
            labels(&mut db, "SELECT v FROM sink ORDER BY v"),
            vec!["neg", "one"],
            "{store}"
        );
        db.execute("INSERT INTO sink VALUES ('null'), ('ten')")
            .unwrap();
        assert_eq!(
            answer(
                &mut db,
                "DELETE FROM sink WHERE v IN (SELECT v FROM r WHERE u <= 1)"
            ),
            "DELETE 2 []",
            "{store}"
        );
        assert_eq!(
            answer(
                &mut db,
                "UPDATE sink SET v = 'hit' WHERE EXISTS (SELECT 1 FROM r WHERE n < 1)"
            ),
            "UPDATE 0 []",
            "{store}"
        );
        assert_eq!(
            labels(&mut db, "SELECT v FROM sink ORDER BY v"),
            vec!["null", "ten"],
            "{store}"
        );
        // A write whose own `WHERE` is the range scans its target either way.
        assert_eq!(
            answer(&mut db, "DELETE FROM r WHERE u < 5 RETURNING v"),
            "DELETE 2 [[Text(\"one\")], [Text(\"neg\")]]",
            "{store}"
        );
        assert_eq!(
            labels(&mut db, "SELECT v FROM r ORDER BY v"),
            vec!["null", "ten"],
            "{store}"
        );
    }
}

#[test]
fn the_index_still_holds_and_enforces_its_null_key() {
    // What a range read leaves out is not what the index holds: a second NULL is still a
    // duplicate, and an upsert still arbitrates on the NULL key.
    for (store, mut db) in in_memory_stores() {
        seed(&mut db);
        assert_eq!(
            answer(&mut db, "INSERT INTO r VALUES (NULL, 7, 'dup')"),
            "ERR 23505",
            "{store}"
        );
        assert_eq!(
            answer(&mut db, "UPDATE r SET u = NULL WHERE u = 1"),
            "ERR 23505",
            "{store}"
        );
        assert_eq!(
            answer(
                &mut db,
                "INSERT INTO r VALUES (NULL, 7, 'x') ON CONFLICT (u) DO UPDATE SET v = 'upserted' RETURNING v"
            ),
            "INSERT 0 1 [[Text(\"upserted\")]]",
            "{store}"
        );
        // The non-unique index takes a second NULL key.
        assert_eq!(
            answer(&mut db, "INSERT INTO r VALUES (50, NULL, 'more')"),
            "INSERT 0 1 []",
            "{store}"
        );
        assert_eq!(
            labels(&mut db, "SELECT v FROM r WHERE n IS NULL ORDER BY v"),
            vec!["more", "one", "upserted"],
            "{store}"
        );
    }
}

#[test]
fn a_reopened_file_store_answers_no_null_keyed_row() {
    // The NULL key is in the stored index; leaving it out is the read's.
    let dir = ScratchDir::new("range_null_key");
    {
        let mut db = Database::open(&dir.path).unwrap();
        seed(&mut db);
        assert_reads(&mut db, "file store");
    }
    let mut db = Database::open(&dir.path).unwrap();
    assert_reads(&mut db, "file store, reopened");
    assert_eq!(
        answer(&mut db, "INSERT INTO r VALUES (NULL, 7, 'dup')"),
        "ERR 23505",
        "the reopened index still holds its NULL key"
    );
}

/// The plans behind the answers above. The stripped build plans a `Filter` over a scan
/// for each, so there is no accelerated node to assert.
#[cfg(feature = "index_acceleration")]
mod plans {
    use super::seed;
    use pglet::Database;
    use pglet_sql::planner::Plan;

    /// Whether `sql` reads its table through an `IndexRangeScan`.
    pub(super) fn reads_through_an_index_range_scan(db: &Database, sql: &str) -> bool {
        matches!(scan_node(db, sql), Plan::IndexRangeScan { .. })
    }

    /// The scan node `sql` reads its table through, under any `Project`, `Sort`,
    /// `Limit` and residual `Filter`.
    fn scan_node(db: &Database, sql: &str) -> Plan {
        let mut plan = pglet_sql::parser::parse_single(sql, db.schema())
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_else(|| panic!("{sql}: no statement"));
        loop {
            plan = match plan {
                Plan::Project { input, .. }
                | Plan::Sort { input, .. }
                | Plan::Limit { input, .. }
                | Plan::Filter { input, .. } => *input,
                other => return other,
            };
        }
    }

    #[test]
    fn a_range_without_a_lower_bound_is_an_index_range_scan() {
        let mut db = Database::with_btree();
        seed(&mut db);
        for (sql, index) in [
            ("SELECT v FROM r WHERE u < 5", "r_u"),
            ("SELECT v FROM r WHERE u <= 5 AND v <> 'x'", "r_u"),
            ("SELECT v FROM r WHERE 5 > u ORDER BY u LIMIT 1", "r_u"),
            ("SELECT v FROM r WHERE n < 5", "r_n"),
            ("SELECT v FROM r WHERE n <= $1", "r_n"),
        ] {
            let want = db.schema().get_index(index).unwrap().oid;
            match scan_node(&db, sql) {
                Plan::IndexRangeScan {
                    index_oid,
                    lower,
                    upper,
                    ..
                } => {
                    assert_eq!(index_oid, want, "{sql}");
                    assert!(lower.is_none() && upper.is_some(), "{sql}");
                }
                other => panic!("{sql}: expected an IndexRangeScan, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_ordered_scan_is_not_planned_over_a_nullable_key() {
        // The one range read with no bound is an `ORDER BY` served from an index, and it
        // answers every row only where no key is NULL: over a nullable column the sort
        // stays, whatever the index holds.
        let mut db = Database::with_btree();
        seed(&mut db);
        for sql in [
            "SELECT v FROM r ORDER BY u",
            "SELECT v FROM r ORDER BY u DESC",
            "SELECT v FROM r ORDER BY u NULLS FIRST",
            "SELECT v FROM r ORDER BY n LIMIT 1",
        ] {
            assert!(
                matches!(scan_node(&db, sql), Plan::Scan { .. }),
                "{sql}: a nullable key keeps the scan and its sort"
            );
        }
        db.execute("CREATE TABLE k (u INT NOT NULL, v TEXT)")
            .unwrap();
        db.execute("CREATE UNIQUE INDEX k_u ON k (u) NULLS NOT DISTINCT")
            .unwrap();
        assert!(
            matches!(
                scan_node(&db, "SELECT v FROM k ORDER BY u"),
                Plan::IndexRangeScan {
                    lower: None,
                    upper: None,
                    ..
                }
            ),
            "a NOT NULL key is served in index order"
        );
    }
}
