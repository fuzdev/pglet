//! A `jsonb` and an array index key over **committed** entries, on each storage backend.
//!
//! The differential fixtures (`tests/fixtures/container_key/`) hold 18.6's answers, and
//! each runs inside the harness's `BEGIN … ROLLBACK`, where an index entry is a pending
//! write and no commit publishes one. What is here is the other side of that: autocommit
//! writes on the in-memory B+tree, the file store and the plain in-memory store; an
//! ordered scan, a bound probe and a bound range over the entries a commit wrote; and a
//! file store reopened over them. The expected rows are 18.6's for the same statements
//! (measured, the fixtures' own corpora).

// A test harness: a failed setup statement should fail the test where it happens.
#![allow(clippy::panic, clippy::expect_used)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::types::{ArrayValue, TypeOid, Value, parse_jsonb};

/// `jsonb` keys in 18.6's order, measured: `[]` below every top-level scalar, type
/// rank, an array by length before its elements, an object by pair count before its
/// pairs. Each is the text `jsonb`'s output function writes.
const JSONB_ASCENDING: [&str; 27] = [
    "[]",
    "null",
    r#""""#,
    r#""a""#,
    r#""z""#,
    "-1",
    "0",
    "100",
    "false",
    "true",
    "[null]",
    r#"["a"]"#,
    "[1]",
    "[true]",
    "[[]]",
    "[{}]",
    "[0, 0]",
    "[[1], 3]",
    "[[1, 2], 3]",
    "{}",
    r#"{"a": null}"#,
    r#"{"a": 1}"#,
    r#"{"aa": 1}"#,
    r#"{"b": 1}"#,
    r#"{"a": 1, "b": 1}"#,
    r#"{"b": 1, "aa": 2}"#,
    r#"{"b": 2, "aa": 1}"#,
];

/// `int[]` keys in 18.6's order, measured: elements before length, a NULL element above
/// every value.
const ARRAY_ASCENDING: [&str; 10] = [
    "{}",
    "{-1,5,5}",
    "{1}",
    "{1,2}",
    "{1,2,3}",
    "{1,NULL}",
    "{2}",
    "{NULL}",
    "{NULL,1}",
    "{NULL,NULL}",
];

fn hosts(dir: &ScratchDir) -> Vec<(&'static str, Database)> {
    vec![
        ("btree", Database::with_btree()),
        ("file", Database::open(&dir.path).expect("open")),
        ("memory", Database::in_memory()),
    ]
}

/// The first column of each row `sql` returns, as text.
fn texts(db: &mut Database, sql: &str) -> Vec<String> {
    let result = db
        .execute(sql)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .remove(0);
    first_column(&result.rows)
}

fn first_column(rows: &[Vec<Value>]) -> Vec<String> {
    rows.iter()
        .map(|row| match &row[0] {
            Value::Text(s) => s.clone(),
            other => panic!("expected a text column, got {other:?}"),
        })
        .collect()
}

/// `sql`'s first column over `params`, bound through the embed door.
fn bound_texts(db: &mut Database, sql: &str, params: &[Value]) -> Vec<String> {
    let result = db
        .execute_bound(sql, 0, |_| {
            Ok::<_, ()>(params.iter().cloned().map(Some).collect())
        })
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    first_column(&result.rows)
}

fn jsonb(text: &str) -> Value {
    Value::Jsonb(parse_jsonb(text).expect("valid json"))
}

fn ints(elements: &[Option<i32>]) -> Value {
    Value::Array(ArrayValue::new(
        TypeOid::Int4,
        elements
            .iter()
            .map(|e| e.map_or(Value::Null, Value::Int4))
            .collect(),
    ))
}

/// The reads a `jsonb` primary key answers, over whatever `db` holds of the ladder.
fn assert_jsonb_reads(host: &str, db: &mut Database) {
    assert_eq!(
        texts(db, "SELECT j::text AS t FROM kj ORDER BY j"),
        JSONB_ASCENDING,
        "[{host}] ascending"
    );
    let mut descending = JSONB_ASCENDING.to_vec();
    descending.reverse();
    assert_eq!(
        texts(db, "SELECT j::text AS t FROM kj ORDER BY j DESC LIMIT 5"),
        descending[..5],
        "[{host}] descending"
    );
    // A bound equality matches by `jsonb`'s equality, which folds a number's scale.
    assert_eq!(
        bound_texts(
            db,
            "SELECT j::text AS t FROM kj WHERE j = $1",
            &[jsonb(r#"{"a":1.0}"#)]
        ),
        [r#"{"a": 1}"#],
        "[{host}] equality"
    );
    // Above `[true]` and up to `{}`: the longer arrays, then the empty object.
    assert_eq!(
        bound_texts(
            db,
            "SELECT j::text AS t FROM kj WHERE j > $1 AND j <= $2 ORDER BY j",
            &[jsonb("[true]"), jsonb("{}")]
        ),
        JSONB_ASCENDING[14..20],
        "[{host}] range"
    );
    // Below the empty string: `null`, and below every scalar, the empty array.
    assert_eq!(
        bound_texts(
            db,
            "SELECT j::text AS t FROM kj WHERE j < $1 ORDER BY j",
            &[jsonb(r#""""#)]
        ),
        JSONB_ASCENDING[..2],
        "[{host}] below the lowest string"
    );
    for duplicate in ["1e2", "[1.0]", r#"{"a":1.00}"#, "-0", "[]"] {
        let err = db
            .execute(&format!("INSERT INTO kj VALUES (99, '{duplicate}')"))
            .expect_err("a duplicate key");
        assert_eq!(err.sqlstate(), "23505", "[{host}] {duplicate}: {err}");
    }
}

/// The reads an `int[]` unique key answers, over whatever `db` holds of the ladder.
fn assert_array_reads(host: &str, db: &mut Database) {
    assert_eq!(
        texts(db, "SELECT a::text AS t FROM ka ORDER BY a"),
        ARRAY_ASCENDING,
        "[{host}] ascending"
    );
    assert_eq!(
        bound_texts(
            db,
            "SELECT a::text AS t FROM ka WHERE a = $1",
            &[ints(&[Some(1), None])]
        ),
        ["{1,NULL}"],
        "[{host}] equality over a NULL element"
    );
    assert_eq!(
        bound_texts(
            db,
            "SELECT a::text AS t FROM ka WHERE a > $1 AND a < $2 ORDER BY a",
            &[ints(&[Some(1), Some(2)]), ints(&[None])]
        ),
        ARRAY_ASCENDING[4..7],
        "[{host}] range"
    );
    for duplicate in ["{1,NULL}", "{}", "{NULL,NULL}"] {
        let err = db
            .execute(&format!("INSERT INTO ka VALUES (99, '{duplicate}')"))
            .expect_err("a duplicate key");
        assert_eq!(err.sqlstate(), "23505", "[{host}] {duplicate}: {err}");
    }
}

/// Insert `values` into `table`'s second column one statement apiece, out of order: the
/// odd positions descending, then the even ones ascending.
fn insert_out_of_order(db: &mut Database, table: &str, values: &[&str]) {
    let odd = (0..values.len()).filter(|i| i % 2 == 1).rev();
    let even = (0..values.len()).filter(|i| i % 2 == 0);
    for i in odd.chain(even) {
        db.execute(&format!(
            "INSERT INTO {table} VALUES ({i}, '{}')",
            values[i]
        ))
        .unwrap_or_else(|e| panic!("{}: {e}", values[i]));
    }
}

#[test]
fn a_container_key_answers_over_committed_entries_on_every_backend() {
    let dir = ScratchDir::new("container_key_hosts");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE kj (v INT, j JSONB PRIMARY KEY)")
            .expect("kj");
        insert_out_of_order(&mut db, "kj", &JSONB_ASCENDING);
        assert_jsonb_reads(host, &mut db);

        db.execute("CREATE TABLE ka (id INT, a INT[] NOT NULL UNIQUE)")
            .expect("ka");
        insert_out_of_order(&mut db, "ka", &ARRAY_ASCENDING);
        assert_array_reads(host, &mut db);
    }
}

/// The entries a commit wrote are read by their bytes after a reopen: nothing decodes a
/// key, so the order and the equality are the stored bytes' own.
#[test]
fn a_container_key_survives_a_reopen() {
    let dir = ScratchDir::new("container_key_reopen");
    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE kj (v INT, j JSONB PRIMARY KEY)")
            .expect("kj");
        insert_out_of_order(&mut db, "kj", &JSONB_ASCENDING);
        db.execute("CREATE TABLE ka (id INT, a INT[] NOT NULL UNIQUE)")
            .expect("ka");
        insert_out_of_order(&mut db, "ka", &ARRAY_ASCENDING);
    }
    let mut db = Database::open(&dir.path).expect("reopen");
    assert_jsonb_reads("reopened", &mut db);
    assert_array_reads("reopened", &mut db);
    // And the reopened indexes take a write and its retraction.
    db.execute("INSERT INTO kj VALUES (50, '[2]')")
        .expect("a new key");
    db.execute("DELETE FROM kj WHERE j = '[2]'")
        .expect("its delete");
    db.execute("UPDATE ka SET a = '{7}' WHERE a = '{2}'")
        .expect("a moved key");
    db.execute("UPDATE ka SET a = '{2}' WHERE a = '{7}'")
        .expect("and back");
    assert_jsonb_reads("reopened, written", &mut db);
    assert_array_reads("reopened, written", &mut db);
}

/// A GIN index over `jsonb[]` over committed entries: the probe matches by `jsonb`'s
/// equality, and an update and a delete retract a row's entries.
#[test]
fn a_gin_index_over_jsonb_elements_answers_over_committed_entries() {
    let dir = ScratchDir::new("container_key_gin");
    for (host, mut db) in hosts(&dir) {
        for sql in [
            "CREATE TABLE g (id INT, arr JSONB[])",
            "CREATE INDEX g_ix ON g USING gin (arr)",
            "INSERT INTO g VALUES (1, ARRAY['1'::jsonb, '1.0'::jsonb]), \
             (2, ARRAY['{\"a\":1}'::jsonb]), (3, '{}'), (4, NULL), \
             (5, ARRAY['{\"a\":1}'::jsonb, '2'::jsonb])",
        ] {
            db.execute(sql)
                .unwrap_or_else(|e| panic!("[{host}] {sql}: {e}"));
        }
        let ids = |db: &mut Database, probe: &str| {
            texts(
                db,
                &format!(
                    "SELECT id::text AS t FROM g WHERE arr @> ARRAY['{probe}'::jsonb] ORDER BY id"
                ),
            )
        };
        assert_eq!(ids(&mut db, r#"{"a":1.0}"#), ["2", "5"], "[{host}]");
        assert_eq!(ids(&mut db, "1.00"), ["1"], "[{host}]");
        db.execute("UPDATE g SET arr = ARRAY['2'::jsonb] WHERE id = 1")
            .expect("update");
        db.execute("DELETE FROM g WHERE id = 5").expect("delete");
        assert_eq!(ids(&mut db, "1"), Vec::<String>::new(), "[{host}]");
        assert_eq!(ids(&mut db, "2.0"), ["1"], "[{host}]");
        assert_eq!(ids(&mut db, r#"{"a":1}"#), ["2"], "[{host}]");
    }
}
