//! The four parser walls a ported SQL suite hits in its first ten minutes, driven
//! over the **embed facade** (`Database::execute`).
//!
//! The parser is shared by every host, but the two decisions these walls carry that
//! are *not* the parser's are per-host: the `25001` transaction-block refusal is
//! asked by each host's execute-one-statement choke point
//! (`Plan::forbidden_in_transaction_block`), and the differential fixtures only ever
//! exercise the native server's. So the embed leg gets one statement of each wall
//! here, and the block gate gets both of its directions.
//!
//! What a fixture covers and this file deliberately does not: the pglet-vs-18.6
//! values and SQLSTATEs. Those live in `tests/fixtures/{select,ddl,types}/` and are
//! compared against a live Postgres; these assert only that the embed host reaches
//! the same verdicts the server does.

// A test harness: `.expect()`/`.unwrap()` surface a setup failure loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use pglet::Database;
use pglet_sql::types::Value;

/// The single scalar a one-row, one-column `@test`-shaped statement returns.
fn scalar(db: &mut Database, sql: &str) -> Value {
    let results = db.execute(sql).expect("statement should succeed");
    let last = results.last().expect("a result set");
    last.rows
        .first()
        .and_then(|row| row.first())
        .cloned()
        .expect("one row, one column")
}

/// The SQLSTATE a statement fails with, or `None` when it succeeds.
fn sqlstate(db: &mut Database, sql: &str) -> Option<&'static str> {
    db.execute(sql).err().map(|e| e.sqlstate())
}

#[test]
fn wall_1_typed_constants_evaluate_on_the_embed_host() {
    let mut db = Database::with_btree();
    assert!(
        matches!(scalar(&mut db, "SELECT DATE '2024-06-15'"), Value::Date(_)),
        "a typed constant is typed, not text"
    );
    assert_eq!(scalar(&mut db, "SELECT TEXT 'x'"), Value::Text("x".into()));
    assert_eq!(scalar(&mut db, "SELECT BOOLEAN 'true'"), Value::Bool(true));
    // The multi-word type name, which takes the speculative parse.
    assert_eq!(
        scalar(&mut db, "SELECT DOUBLE PRECISION '1.5'"),
        Value::Float8(1.5)
    );
    // The literal's own conversion, with the code the equivalent cast raises.
    assert_eq!(sqlstate(&mut db, "SELECT DATE 'nope'"), Some("22007"));
    assert_eq!(sqlstate(&mut db, "SELECT INT4 'x'"), Some("22P02"));
    // An identifier followed by a string is a **type** name, never a column — even
    // when a column of that name is in scope (18.6: `42704`, not `42703`).
    db.execute("CREATE TABLE tc (foo TEXT); INSERT INTO tc VALUES ('v')")
        .expect("seeds");
    assert_eq!(sqlstate(&mut db, "SELECT foo 'x' FROM tc"), Some("42704"));
    assert_eq!(
        scalar(&mut db, "SELECT foo FROM tc"),
        Value::Text("v".into()),
        "a bare column of the same name still resolves"
    );
}

#[test]
fn wall_1_an_interval_qualifier_sets_a_bare_number_s_unit() {
    let mut db = Database::with_btree();
    // The silent wrong this closes: unparsed, `'90'` was ninety *seconds* under an
    // output alias `minute`.
    assert_eq!(
        scalar(&mut db, "SELECT INTERVAL '90' MINUTE"),
        Value::Interval(pglet_sql::types::PgInterval::new(0, 0, 5_400_000_000))
    );
    assert_eq!(
        scalar(&mut db, "SELECT INTERVAL '1' DAY"),
        Value::Interval(pglet_sql::types::PgInterval::new(0, 1, 0))
    );
    // A field *range* is refused rather than answered wrong: it re-reads the
    // literal's unqualified fields, which this parser's colon grammar does not do.
    assert_eq!(
        sqlstate(&mut db, "SELECT INTERVAL '3:04' MINUTE TO SECOND"),
        Some("0A000")
    );
}

#[test]
fn wall_2_the_boolean_test_family_answers_and_gates_its_operand() {
    let mut db = Database::with_btree();
    assert_eq!(scalar(&mut db, "SELECT NULL IS TRUE"), Value::Bool(false));
    assert_eq!(scalar(&mut db, "SELECT NULL IS UNKNOWN"), Value::Bool(true));
    assert_eq!(
        scalar(&mut db, "SELECT NULL::bool IS NOT FALSE"),
        Value::Bool(true)
    );
    // `IS` binds looser than `=`, so this is `(1 = 1) IS TRUE`.
    assert_eq!(scalar(&mut db, "SELECT 1 = 1 IS TRUE"), Value::Bool(true));
    // A typed non-boolean operand is `42804`; an `unknown` literal is *converted*,
    // so a bad one is the conversion's `22P02` and a good one answers.
    assert_eq!(sqlstate(&mut db, "SELECT 1 IS TRUE"), Some("42804"));
    assert_eq!(sqlstate(&mut db, "SELECT 'x' IS TRUE"), Some("22P02"));
    assert_eq!(scalar(&mut db, "SELECT 't' IS TRUE"), Value::Bool(true));
}

#[test]
fn wall_2_a_stored_check_carries_the_test_and_enforces_it() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE tk (b BOOL, CHECK (b IS TRUE))")
        .expect("the CHECK is a persistable expression");
    db.execute("INSERT INTO tk VALUES (true)").expect("passes");
    // Both `false` and NULL fail the test, which is what separates `IS TRUE` from a
    // bare boolean predicate.
    assert_eq!(
        sqlstate(&mut db, "INSERT INTO tk VALUES (false)"),
        Some("23514")
    );
    assert_eq!(
        sqlstate(&mut db, "INSERT INTO tk VALUES (NULL)"),
        Some("23514")
    );
    // The `unknown` a stored expression may carry is converted at DDL time, once —
    // there is no later chance, and the dump invariant's catalog part in
    // `docs/sequences.md` requires an accepted catalog to dump to something stock
    // Postgres loads.
    let mut db2 = Database::with_btree();
    assert_eq!(
        sqlstate(&mut db2, "CREATE TABLE tx (b BOOL, CHECK ('x' IS TRUE))"),
        Some("22P02")
    );
    assert_eq!(
        sqlstate(&mut db2, "CREATE TABLE ty (i INT, CHECK (i IS TRUE))"),
        Some("42804")
    );
}

#[test]
fn wall_3_concurrently_builds_an_index_and_refuses_a_transaction_block() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ti (id INT, v TEXT)")
        .expect("seeds");
    db.execute("CREATE INDEX CONCURRENTLY ti_id_idx ON ti (id)")
        .expect("outside a block it builds an ordinary index");
    db.execute("CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS ti_v_idx ON ti (v)")
        .expect("with IF NOT EXISTS, in gram.y's keyword order");
    db.execute("DROP INDEX CONCURRENTLY ti_id_idx")
        .expect("and drops one");
    // Inside a block both are 18.6's `25001` — the one decision the keyword makes
    // here, and the only reason the plan carries it.
    db.execute("BEGIN").expect("opens a block");
    assert_eq!(
        sqlstate(&mut db, "CREATE INDEX CONCURRENTLY ti_id_idx2 ON ti (id)"),
        Some("25001")
    );
    db.execute("ROLLBACK").expect("ends the block");
    db.execute("BEGIN").expect("opens a block");
    assert_eq!(
        sqlstate(&mut db, "DROP INDEX CONCURRENTLY IF EXISTS nope"),
        Some("25001"),
        "even the IF EXISTS spelling naming nothing"
    );
    db.execute("ROLLBACK").expect("ends the block");
    // A quoted `"concurrently"` is an index *name*, as on 18.6 — the keyword is
    // contextual, matched only unquoted.
    db.execute(r#"CREATE INDEX "concurrently" ON ti (id)"#)
        .expect("a quoted keyword is a name");
    db.execute(r#"DROP INDEX "concurrently""#)
        .expect("drops it");
}

#[test]
fn wall_4_extension_statements_answer_by_name() {
    let mut db = Database::with_btree();
    db.execute(r#"CREATE EXTENSION IF NOT EXISTS "uuid-ossp""#)
        .expect("the idiom every migration opens with");
    db.execute("CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public VERSION '1.3' CASCADE")
        .expect("the whole option tail is parsed and dropped");
    db.execute("DROP EXTENSION IF EXISTS pgcrypto CASCADE")
        .expect("and dropped");
    // The one decision the model keeps: whether the **name** is one pglet provides.
    assert_eq!(
        sqlstate(&mut db, "CREATE EXTENSION nosuch_ext"),
        Some("0A000")
    );
    assert_eq!(
        sqlstate(&mut db, "CREATE EXTENSION IF NOT EXISTS nosuch_ext"),
        Some("0A000"),
        "`IF NOT EXISTS` does not excuse an unavailable extension on 18.6 either"
    );
    assert_eq!(sqlstate(&mut db, "DROP EXTENSION nosuch"), Some("42704"));
    assert_eq!(sqlstate(&mut db, "DROP EXTENSION IF EXISTS nosuch"), None);
    // The names are compared against the parser's fold: unquoted upper case matches,
    // a quoted upper-case spelling does not (both measured on 18.6).
    assert_eq!(sqlstate(&mut db, "CREATE EXTENSION PGCRYPTO"), None);
    assert_eq!(
        sqlstate(&mut db, r#"CREATE EXTENSION "UUID-OSSP""#),
        Some("0A000")
    );
    // The functions the two extensions stand for are builtins here, with or without
    // the statement — which is why there is no installation state to model.
    assert!(matches!(
        scalar(&mut db, "SELECT gen_random_uuid() IS NOT NULL"),
        Value::Bool(true)
    ));
    assert!(matches!(
        scalar(&mut db, "SELECT uuid_generate_v4() IS NOT NULL"),
        Value::Bool(true)
    ));
}
