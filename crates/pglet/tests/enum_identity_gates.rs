//! What the enum-identity gates decide that no differential fixture can observe.
//!
//! The gates themselves are pinned by `tests/fixtures/enum/*_loud.sql` and their
//! accept-side controls. Three things sit outside that corpus by construction:
//!
//! - **The rows a refused write leaves behind.** The harness propagates the first error
//!   from the statements it runs, so a fixture whose `@test` is the refused `DELETE`
//!   never reaches a `SELECT` after it, and one whose `@setup` holds the `DELETE` is
//!   scored on that failure instead. The divergence these gates close *wrote* — the
//!   predicate took a row 18.6 keeps by refusing — so the survivors are the half worth
//!   asserting, and they can only be asserted here.
//! - **A refusal over an empty table**, which says the gate is plan-time rather than a
//!   property of the data a fixture happens to seed.
//! - **The catalog a refused DDL did not write**, which is what separates "the statement
//!   errored" from "the constraint is not there".

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface an unexpected verdict shape
// loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::Value;

/// A database holding `mood ('sad','ok','happy')`, a second enum `m3` whose member order
/// disagrees with it, and `t(id, m mood, y m3, s TEXT)` over three rows.
fn seeded() -> Database {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')",
        "CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok')",
        "CREATE TABLE t (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL)",
        "INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), \
         (3, 'happy', 'sad', 'ok')",
    ] {
        db.execute(sql).expect("seeds");
    }
    db
}

/// The number of rows `t` holds.
fn row_count(db: &mut Database) -> i64 {
    let result = db.execute("SELECT count(*) FROM t").expect("count");
    match result
        .last()
        .and_then(|r| r.rows.first())
        .and_then(|r| r.first())
    {
        Some(Value::Int8(n)) => *n,
        other => panic!("expected a count, got {other:?}"),
    }
}

/// The labels `t.m` holds, in `id` order.
fn labels(db: &mut Database) -> Vec<String> {
    let result = db
        .execute("SELECT m::text FROM t ORDER BY id")
        .expect("labels");
    result
        .last()
        .expect("one statement, one result")
        .rows
        .iter()
        .map(|r| match r.first() {
            Some(Value::Text(s)) => s.clone(),
            other => panic!("expected a label, got {other:?}"),
        })
        .collect()
}

/// The predicate `m > s` took the `happy` row where 18.6 refuses the statement and keeps
/// all three. The gate runs at plan time, so nothing is written before it answers —
/// which is the claim the row count makes and the fixture's SQLSTATE does not.
#[test]
fn a_refused_enum_predicate_writes_nothing() {
    let mut db = seeded();
    for sql in [
        "DELETE FROM t WHERE m > s",
        "UPDATE t SET s = 'hit' WHERE m > s",
        "DELETE FROM t WHERE m > y",
        "UPDATE t SET m = greatest(m, s)",
        "DELETE FROM t WHERE COALESCE(m, s) > 'ok'",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert!(
            matches!(err.sqlstate(), "42883" | "42804" | "42846"),
            "{sql}: {err} ({})",
            err.sqlstate()
        );
        assert_eq!(row_count(&mut db), 3, "{sql} left rows removed");
    }
    // …and the stored labels are untouched, which the count alone would not say for the
    // two `UPDATE`s.
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);
}

/// The gate is **plan-time**, so it answers over a table with no rows at all. An
/// eval-time gate reading the operands off `Value`s cannot: there is no row to read, and
/// the statement would succeed with `(0 rows)`.
#[test]
fn the_refusal_holds_over_an_empty_table() {
    let mut db = seeded();
    db.execute("DELETE FROM t WHERE id > 0").expect("empty it");
    assert_eq!(row_count(&mut db), 0);
    for sql in [
        "SELECT id FROM t WHERE m > s",
        "SELECT greatest(m, s) FROM t",
        "SELECT id FROM t WHERE m > y",
        "SELECT COALESCE(m, y) FROM t",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert!(
            matches!(err.sqlstate(), "42883" | "42804" | "42846"),
            "{sql}: {err}"
        );
    }
}

/// A refused `CHECK` or partial index leaves **no catalog entry**. The fixture asserts
/// the SQLSTATE; this asserts the consequence the SQLSTATE stands in for, which is what
/// the divergence actually cost — a persisted constraint whose dump stock 18.6 refuses
/// to restore while `psql` exits `0`.
#[test]
fn a_refused_stored_expression_records_nothing() {
    let mut db = seeded();
    for sql in [
        "CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (m <> s))",
        "CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (greatest(m, s) > 'ok'))",
        "CREATE TABLE ck (m mood NOT NULL, y m3 NOT NULL, CHECK (m > y))",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert!(
            matches!(err.sqlstate(), "42883" | "42804" | "42846"),
            "{sql}: {err}"
        );
        // The refused `CREATE TABLE` left no relation behind at all.
        assert!(db.execute("SELECT * FROM ck").is_err(), "{sql} left `ck`");
    }
    let err = db
        .execute("CREATE UNIQUE INDEX tix ON t (id) WHERE greatest(m, s) > 'ok'")
        .expect_err("the partial index is refused");
    assert_eq!(err.sqlstate(), "42804", "{err}");
    let indexes = db
        .execute("SELECT indexname FROM pg_indexes WHERE indexname = 'tix'")
        .expect("pg_indexes reads");
    assert!(
        indexes.last().is_some_and(|r| r.rows.is_empty()),
        "the refused index was recorded"
    );
}

/// The **accept** side, over the same database: an unresolved literal beside an enum is
/// 18.6's `unknown` and resolves to the enum, and a written cast leaves an identity this
/// engine cannot decide rather than one it can contradict. Both must still write.
#[test]
fn the_gates_still_admit_what_18_6_admits() {
    let mut db = seeded();
    db.execute("DELETE FROM t WHERE m > 'ok'")
        .expect("an unresolved literal resolves to the enum");
    assert_eq!(labels(&mut db), vec!["sad", "ok"]);
    db.execute("UPDATE t SET m = greatest(m, s::mood)")
        .expect("a cast leaves an undecidable identity, not a contradicting one");
    assert_eq!(labels(&mut db), vec!["ok", "ok"]);
    db.execute("DELETE FROM t WHERE m > n")
        .expect_err("no column `n` — the control that says the statements above ran");
}

/// The same three things, one gate further out: the operand readers now reach a
/// **computed** text side, where they used to hand the rulebook nothing and the write
/// went through. `lower(s)`, a concatenation and a scalar subquery each stand where a
/// bare `text` column stands above.
#[test]
fn a_refused_computed_operand_predicate_writes_nothing() {
    let mut db = seeded();
    for sql in [
        "DELETE FROM t WHERE m > lower(s)",
        "DELETE FROM t WHERE m > (s || '')",
        "DELETE FROM t WHERE m > (SELECT s FROM t WHERE id = 1)",
        "UPDATE t SET m = greatest(m, lower(s))",
        "UPDATE t SET m = COALESCE(m, lower(s))",
        "DELETE FROM t WHERE m IN (lower(s))",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert!(
            matches!(err.sqlstate(), "42883" | "42804" | "42846"),
            "{sql}: {err} ({})",
            err.sqlstate()
        );
        assert_eq!(row_count(&mut db), 3, "{sql} left rows removed");
    }
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);
}

/// A non-member label is refused where 18.6's `enum_in` refuses it, and the write it used
/// to make is the assertion. The last statement is the one no enum column guards: the
/// value lands in a **text** column, so the write path's own membership check never sees
/// it.
#[test]
fn a_non_member_literal_writes_nothing() {
    let mut db = seeded();
    db.execute("CREATE TABLE tx (v TEXT)").expect("text sink");
    for sql in [
        "DELETE FROM t WHERE m IN ('ok', 'zzz')",
        "DELETE FROM t WHERE m > 'zzz'",
        "UPDATE t SET s = 'hit' WHERE m = 'zzz'",
        "DELETE FROM t WHERE COALESCE(m, 'zzz') > 'sad'",
        "INSERT INTO tx SELECT greatest(m, 'zzz') FROM t",
        "INSERT INTO tx SELECT m FROM t UNION ALL SELECT 'zzz'",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "22P02", "{sql}: {err}");
        assert_eq!(row_count(&mut db), 3, "{sql} left rows removed");
    }
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);
    let sink = db.execute("SELECT count(*) FROM tx").expect("tx reads");
    assert_eq!(
        sink.last().and_then(|r| r.rows.first()).map(Vec::as_slice),
        Some([Value::Int8(0)].as_slice()),
        "a refused projection landed rows in the text column"
    );
}

/// The membership refusal over an **empty** table, which says it is 18.6's parse-time
/// coercion rather than a property of the rows: a gate that read the label off a `Value`
/// would answer `(0 rows)` here.
#[test]
fn the_membership_refusal_holds_over_an_empty_table() {
    let mut db = seeded();
    db.execute("DELETE FROM t WHERE id > 0").expect("empty it");
    assert_eq!(row_count(&mut db), 0);
    for sql in [
        "SELECT id FROM t WHERE m > 'zzz'",
        "SELECT greatest(m, 'zzz') FROM t",
        "SELECT m FROM t UNION ALL SELECT 'zzz'",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "22P02", "{sql}: {err}");
    }
}

/// A non-member label in a persisted expression leaves **no catalog entry** — the half
/// that broke the dump invariant, a recorded `CHECK` or partial index restoring into
/// stock 18.6 with neither while `psql` exits `0`.
#[test]
fn a_refused_non_member_stored_expression_records_nothing() {
    let mut db = seeded();
    for sql in [
        "CREATE TABLE ck (m mood NOT NULL, CHECK (m > 'zzz'))",
        "CREATE TABLE ck (m mood NOT NULL, CHECK (greatest(m, 'zzz') > 'sad'))",
        "CREATE TABLE ck (m mood NOT NULL, CHECK (m IN ('ok', 'zzz')))",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "22P02", "{sql}: {err}");
        assert!(db.execute("SELECT * FROM ck").is_err(), "{sql} left `ck`");
    }
    let err = db
        .execute("CREATE INDEX tix ON t (id) WHERE m > 'zzz'")
        .expect_err("the partial index is refused");
    assert_eq!(err.sqlstate(), "22P02", "{err}");
    let indexes = db
        .execute("SELECT indexname FROM pg_indexes WHERE indexname = 'tix'")
        .expect("pg_indexes reads");
    assert!(
        indexes.last().is_some_and(|r| r.rows.is_empty()),
        "the refused index was recorded"
    );
}

/// The accept side of **both** halves of the gate, over the same database.
///
/// A **member** label must still write through every door the non-member one is refused
/// at — that pair is what makes the membership guard discriminate, rather than pinning a
/// rule that simply refuses every literal. And a computed **comparison** pair with no enum
/// in it must still answer: the widened read is spent there only where the pair names
/// one, so that pair keeps the conservatism `EDGES.md` row 65 records, and a fixture
/// cannot say so, since agreeing-and-silent is what it looked like before. A *branch set*
/// is widened where the set names an enum, which does move a non-enum pair inside it —
/// toward 18.6, row 114's bullet — so no branch set is asserted here.
#[test]
fn the_widened_gates_still_admit_what_18_6_admits() {
    let mut db = seeded();
    db.execute("CREATE TABLE tx (v TEXT)").expect("text sink");
    db.execute("SELECT lower(s) > upper(s), length(s) > 1 FROM t")
        .expect("a computed pair with no enum keeps the conservative reader");
    db.execute("SELECT max(m) > min(m) FROM t")
        .expect("two computed operands of the same enum");
    db.execute("INSERT INTO tx SELECT greatest(m, 'ok') FROM t")
        .expect("a member label converts");
    db.execute("INSERT INTO tx SELECT m FROM t UNION ALL SELECT 'happy'")
        .expect("a member label converts at the set-operation door too");
    db.execute("CREATE TABLE ck (m mood NOT NULL, CHECK (greatest(m, 'ok') >= 'sad'))")
        .expect("a member label converts in a persisted expression");
    db.execute("CREATE INDEX tix ON t (id) WHERE m > 'ok'")
        .expect("and in a partial index predicate");
    db.execute("DELETE FROM t WHERE m > 'ok'")
        .expect("an unresolved member literal resolves to the enum");
    assert_eq!(labels(&mut db), vec!["sad", "ok"]);
}

/// A **bound** `$N` at an enum position is converted at bind, and the rows a refused
/// write leaves are what says the refusal came first.
///
/// The two comparison positions whose right-hand operand is **not a sibling `Expr`** —
/// an `IN (SELECT …)`'s sub-plan column and an `op ANY`/`op ALL`'s array element. Both
/// write, and both are outside the fixture corpus for the reasons above: the survivors
/// of the refused statement, and the refusal over a table with no rows.
#[test]
fn a_refused_one_sided_predicate_writes_nothing() {
    let mut db = seeded();
    for sql in [
        "DELETE FROM t WHERE m IN (SELECT s FROM t)",
        "DELETE FROM t WHERE m NOT IN (SELECT s FROM t)",
        "DELETE FROM t WHERE m IN (SELECT y FROM t)",
        "DELETE FROM t WHERE m IN (SELECT * FROM (VALUES ('sad')) v)",
        "UPDATE t SET s = 'hit' WHERE m IN (SELECT s FROM t)",
        "DELETE FROM t WHERE m = ANY(ARRAY[s])",
        "DELETE FROM t WHERE m = ALL(ARRAY[s])",
        "DELETE FROM t WHERE m > ANY(ARRAY[y])",
        // An all-`unknown` array is `text[]` on 18.6, member labels and all.
        "DELETE FROM t WHERE m = ANY(ARRAY['ok', 'sad'])",
        "UPDATE t SET s = 'hit' WHERE m = ANY(ARRAY[s])",
        // A **mixed** array is decided by its typed element, the `unknown` beside it
        // contributing nothing to `transformArrayExpr`'s fold — so one added label does
        // not re-open the door. Every comparison spelling `comparison_symbol` carries,
        // in both quantifiers, and both writes.
        "DELETE FROM t WHERE m = ANY(ARRAY[s, 'ok'])",
        "DELETE FROM t WHERE m = ANY(ARRAY['ok', s])",
        "DELETE FROM t WHERE m = ANY(ARRAY[s, NULL])",
        "DELETE FROM t WHERE m <> ANY(ARRAY[s, 'ok'])",
        "DELETE FROM t WHERE m > ANY(ARRAY[s, 'ok'])",
        "DELETE FROM t WHERE m >= ANY(ARRAY[s, 'ok'])",
        "DELETE FROM t WHERE m < ANY(ARRAY[s, 'ok'])",
        "DELETE FROM t WHERE m <= ANY(ARRAY[s, 'ok'])",
        "DELETE FROM t WHERE m = ALL(ARRAY[s, 'ok'])",
        "DELETE FROM t WHERE m = ANY(ARRAY[y, 'ok'])",
        "UPDATE t SET s = 'hit' WHERE m = ANY(ARRAY[s, 'ok'])",
        // …and the `ARRAY[…]` **pair** spelling of the same element rule.
        "DELETE FROM t WHERE ARRAY[m] < ARRAY['ok']",
        // A **computed** sub-plan column the walk decides is no enum keeps refusing:
        // the widening is spent on the identity, not on the door.
        "DELETE FROM t WHERE m IN (SELECT lower(s) FROM t)",
        "DELETE FROM t WHERE m IN (SELECT min(y) FROM t)",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "42883", "{sql}: {err}");
        assert_eq!(row_count(&mut db), 3, "{sql} left rows removed");
    }
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);
    let untouched = db
        .execute("SELECT s FROM t WHERE s = 'hit'")
        .expect("reads");
    assert!(
        untouched.last().is_some_and(|r| r.rows.is_empty()),
        "a refused UPDATE wrote"
    );
}

/// The accept polarity of the same two positions **writes**, and that is the half a
/// refusal-only guard cannot see: an over-refusal here costs the rows 18.6 removes.
///
/// A sub-plan's projected *expression* used to carry no `ColumnDesc::enum_type`
/// (`EDGES.md` row 107), and reading that `None` as *not an enum* refused the two
/// `IN (SELECT …)` statements below; the array one already answered, and is here because
/// the element rule that closed the mixed array had to leave it answering.
#[test]
fn an_accepted_one_sided_predicate_writes_what_18_6_writes() {
    let mut db = seeded();
    db.execute("DELETE FROM t WHERE m IN (SELECT 'ok'::mood)")
        .expect("a cast sub-plan column resolves the operator");
    assert_eq!(labels(&mut db), vec!["sad", "happy"]);
    db.execute("DELETE FROM t WHERE m IN (SELECT min(m) FROM t)")
        .expect("an aggregate sub-plan column carries its argument's identity");
    assert_eq!(labels(&mut db), vec!["happy"]);
    db.execute("DELETE FROM t WHERE m = ANY(ARRAY[m, 'ok'])")
        .expect("a mixed array whose typed element is the enum resolves it");
    assert_eq!(row_count(&mut db), 0);
}

/// The same two positions over a table with **no rows**, which is what separates a
/// plan-time gate from an eval-time one: there is no `Value` to read the operand kinds
/// off, and the statement would answer `(0 rows)`.
#[test]
fn the_one_sided_refusal_holds_over_an_empty_table() {
    let mut db = seeded();
    db.execute("DELETE FROM t WHERE id > 0").expect("empty it");
    assert_eq!(row_count(&mut db), 0);
    for sql in [
        "SELECT id FROM t WHERE m IN (SELECT s FROM t)",
        "SELECT id FROM t WHERE m IN (SELECT s FROM t WHERE false)",
        "SELECT id FROM t WHERE m IN (SELECT y FROM t)",
        "SELECT id FROM t WHERE m = ANY(ARRAY[s])",
        "SELECT id FROM t WHERE m > ANY(ARRAY[y])",
        "SELECT id FROM t WHERE m = ANY(ARRAY['ok'])",
        "SELECT id FROM t WHERE m = ANY(ARRAY[s, 'ok'])",
        "SELECT id FROM t WHERE m = ANY(ARRAY[s, NULL])",
        "SELECT id FROM t WHERE m IN (SELECT lower(s) FROM t)",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "42883", "{sql}: {err}");
    }
    // …and the conversion half, which is the *other* verdict this door can reach.
    let err = db
        .execute("SELECT id FROM t WHERE 'zzz' IN (SELECT m FROM t)")
        .expect_err("a non-member left-hand literal");
    assert_eq!(err.sqlstate(), "22P02", "{err}");
}

/// A refused `op ANY`/`op ALL` inside a **persisted** expression leaves no catalog entry.
/// That is the half of this door the DDL-time walk owns, and what it cost was a recorded
/// `CHECK` whose dump stock 18.6 refuses to load while `psql` exits `0`. The
/// `IN (SELECT …)` half has no counterpart here: both engines refuse a subquery in every
/// persisted-expression context, so no stored expression can carry one.
#[test]
fn a_refused_array_quantifier_stored_expression_records_nothing() {
    let mut db = seeded();
    for sql in [
        "CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (m = ANY(ARRAY[s])))",
        "CREATE TABLE ck (m mood NOT NULL, CHECK (m = ANY(ARRAY['ok', 'sad'])))",
        "CREATE TABLE ck (m mood NOT NULL, y m3 NOT NULL, CHECK (m > ANY(ARRAY[y])))",
        "CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (m = ANY(ARRAY[s, 'ok'])))",
        "CREATE TABLE ck (m mood NOT NULL, y m3 NOT NULL, CHECK (m = ANY(ARRAY[y, 'ok'])))",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "42883", "{sql}: {err}");
        assert!(db.execute("SELECT * FROM ck").is_err(), "{sql} left `ck`");
    }
    let err = db
        .execute("CREATE UNIQUE INDEX tix ON t (id) WHERE m = ANY(ARRAY[s])")
        .expect_err("the partial index is refused");
    assert_eq!(err.sqlstate(), "42883", "{err}");
    let indexes = db
        .execute("SELECT indexname FROM pg_indexes WHERE indexname = 'tix'")
        .expect("pg_indexes reads");
    assert!(
        indexes.last().is_some_and(|r| r.rows.is_empty()),
        "the refused index was recorded"
    );
    // The accept polarity of the element rule, at both persisted positions: an element a
    // cast leaves undecidable is not an `unknown`, and 18.6 types that array `mood[]`.
    db.execute("CREATE TABLE ok1 (m mood NOT NULL, CHECK (m = ANY(ARRAY['ok'::mood])))")
        .expect("a cast element is not folded to text");
    db.execute("CREATE INDEX tix2 ON t (id) WHERE m = ANY(ARRAY[m])")
        .expect("an enum column's array keeps its element identity");
    db.execute("CREATE TABLE ok2 (m mood NOT NULL, CHECK (m = ANY(ARRAY[m, 'ok'])))")
        .expect("a mixed array whose typed element is the enum records");
}

/// `Database::execute_bound` is the in-process host — the wasm bindings' `queryParams`
/// runs through it — and the fixture corpus reaches it only through the native server's
/// extended protocol, so these three facts sit outside it: the survivors of a refused
/// `DELETE`, the rows a refused `INSERT … SELECT` did not land, and a member label
/// binding through the same statements.
#[test]
fn a_bound_parameter_converts_to_the_enum_and_a_refused_write_leaves_the_rows() {
    let mut db = seeded();
    db.execute("CREATE TABLE tx (v TEXT)").expect("text sink");

    // A non-member through each position the target walk reaches, over both writes and
    // reads. Every one is `22P02` before anything executes.
    for sql in [
        "DELETE FROM t WHERE m < $1",
        "INSERT INTO tx SELECT greatest(m, $1) FROM t",
        "UPDATE t SET s = 'x' WHERE m > $1",
        "SELECT m > $1 FROM t",
        "SELECT m IN ($1) FROM t",
        "SELECT nullif(m, $1) FROM t",
        "SELECT CASE m WHEN $1 THEN 1 ELSE 0 END FROM t",
        "SELECT COALESCE(m, $1) FROM t",
        "SELECT ARRAY[m, $1] FROM t",
        // The array side carries the target: `make_scalar_array_op` resolves the
        // operator against the **element**, so a bare `$N` beside it is that element's
        // `unknown`. (A `$N` *inside* the array is not — an all-`unknown` element set is
        // `text`, and `m = ANY(ARRAY[$1])` is 18.6's `42883`.)
        "SELECT $1 = ANY(ARRAY[m]) FROM t",
        "DELETE FROM t WHERE $1 = ANY(ARRAY[m])",
        // No row reaches the predicate, and the refusal is unmoved: the conversion is
        // the bound value's, not a row's.
        "SELECT m > $1 FROM t WHERE false",
    ] {
        match bind_one(&mut db, sql, "zzz") {
            Err(pglet::BoundExecError::Engine(e)) => {
                assert_eq!(e.sqlstate(), "22P02", "{sql}: {e}");
                assert_eq!(
                    e.to_string(),
                    "invalid input value for enum mood: \"zzz\"",
                    "{sql}"
                );
            }
            other => panic!("{sql}: expected the conversion's refusal, got {other:?}"),
        }
        db.execute("ROLLBACK").ok();
    }
    assert_eq!(row_count(&mut db), 3, "a refused write left the rows");
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);
    assert!(
        db.execute("SELECT v FROM tx")
            .expect("the sink reads")
            .last()
            .is_some_and(|r| r.rows.is_empty()),
        "a refused `INSERT … SELECT` landed rows"
    );

    // The accept polarity, and the discriminator: a **member** binds, and it orders by
    // the enum's member positions rather than its label bytes — `happy` is the only row
    // greater than `ok`, where the bytes would put `sad` there too.
    bind_one(&mut db, "DELETE FROM t WHERE m > $1", "ok").expect("a member label binds");
    assert_eq!(labels(&mut db), vec!["sad", "ok"]);
    bind_one(
        &mut db,
        "INSERT INTO tx SELECT greatest(m, $1) FROM t",
        "ok",
    )
    .expect("a member label converts at the fold too");
}

/// Bind `$1` to one text value and run `sql` — the one-parameter shape every case in
/// this file's bound-parameter test takes.
fn bind_one(
    db: &mut Database,
    sql: &str,
    value: &str,
) -> Result<pglet_sql::executor::QueryResult, pglet::BoundExecError<()>> {
    db.execute_bound(sql, 1, |_| Ok(vec![Some(Value::Text(value.to_string()))]))
}

/// The **derived-relation** door — a derived table, a CTE, a `JOIN`, an `UPDATE … FROM`
/// source, and the scalar-subquery operand beside them — binds its columns through a
/// namespace built from `ColumnDesc`s, and a computed projection's descriptor carried no
/// enum (`EDGES.md` row 107). Reading that `None` as *not an enum* refused ordinary SQL
/// 18.6 answers, at every one of those spellings.
///
/// Both polarities are asserted **over a table with no rows**, and that is what puts this
/// outside the fixture corpus. The refusals are the discriminating half: a gate reading
/// its operands off `Value`s has nothing to read here, so an eval-time one would let all
/// four through with `(0 rows)`. The accepts beside them say the repair did not buy its
/// answer from a row either.
#[test]
fn the_derived_relation_door_decides_at_plan_time() {
    let mut db = seeded();
    db.execute("DELETE FROM t").expect("empty the table");

    // The accept polarity over **no rows at all**: the identity is derived from the
    // projection, which an empty table does not change.
    for sql in [
        "SELECT t.id FROM t, (SELECT min(m) AS z FROM t t2) v WHERE t.m > v.z",
        "WITH c AS (SELECT min(m) AS z FROM t t2) SELECT t.id FROM t, c WHERE t.m = c.z",
        "SELECT t.id FROM t JOIN (SELECT min(m) AS z FROM t t2) v ON t.m = v.z",
        "SELECT t.id FROM t WHERE t.m > (SELECT min(m) FROM t t2)",
        "UPDATE t SET s = 'x' FROM (SELECT min(m) AS z FROM t t2) v WHERE t.m = v.z",
    ] {
        db.execute(sql).expect(sql);
    }

    // The refuse polarity over the same empty table, at both halves the derivation has
    // to keep deciding: a passthrough `text` column, whose descriptor could always
    // decide it, and a computed `text` one, whose descriptor never could.
    for sql in [
        "SELECT t.id FROM t, (SELECT s AS z FROM t t2) v WHERE t.m = v.z",
        "SELECT t.id FROM t, (SELECT lower(s) AS z FROM t t2) v WHERE t.m = v.z",
        "SELECT t.id FROM t, (SELECT min(y) AS z FROM t t2) v WHERE t.m = v.z",
        "SELECT t.id FROM t WHERE t.m > (SELECT min(s) FROM t t2)",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "42883", "{sql}: {err}");
    }
}

/// What the still-refused writes at that door leave behind, and what the accepted ones
/// write — the halves a fixture cannot show, its `@test` stopping at the first error and
/// its `@setup` being scored as the result.
#[test]
fn the_derived_relation_door_writes_what_18_6_writes() {
    let mut db = seeded();
    // `DELETE … USING` is a parser gap here, so the `FROM`-source write spelling is the
    // `UPDATE` one — a `42601` would make every assertion below it vacuous.
    for sql in [
        "UPDATE t SET s = 'hit' FROM (SELECT s AS z FROM t t2) v WHERE t.m = v.z",
        "UPDATE t SET s = 'hit' FROM (SELECT lower(s) AS z FROM t t2) v WHERE t.m = v.z",
        "DELETE FROM t WHERE m = (SELECT min(s) FROM t t2)",
        "DELETE FROM t WHERE m = (SELECT s::text FROM t t2 WHERE id = 1)",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "42883", "{sql}: {err}");
        assert_eq!(row_count(&mut db), 3, "{sql} left rows removed");
    }
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);

    // The accepted write orders by the enum's **member positions**: the subquery is `ok`
    // (the greater of `sad` and `ok` by position), and `happy` is the only row above it.
    // By the label bytes the subquery would be `sad`, and the predicate would take two.
    db.execute("DELETE FROM t WHERE m > (SELECT max(m) FROM t t2 WHERE id < 3)")
        .expect("a computed scalar subquery operand is an accepted predicate");
    assert_eq!(labels(&mut db), vec!["sad", "ok"]);
}

/// **A recorded divergence, locked at the polarity that writes.** `EDGES.md` row 114's
/// sub-plan-column residual: a cast leaves the projection's identity undecidable, and the
/// `IN (SELECT …)` door falls such a pair back to the OID rule and **accepts**, so the
/// predicate removes rows 18.6 keeps by refusing `42883 operator does not exist: mood =
/// text`. No fixture can hold it in either polarity: an `@expect-error` one needs pglet
/// to raise, which it does not, and a value one needs the two engines to agree, which
/// they do not — so the rows the predicate removed are only visible here.
///
/// The two neighbouring doors take the opposite side of the same trade and are asserted
/// beside it: the scalar-subquery operand and the derived table both refuse. Closing the
/// residual means this test changes, which is the point of writing it — the alternative
/// is prose no run can contradict. What it cannot be closed *by* is a rule keyed on the
/// cast's source: `x::mood` and `x::text` are the same `Expr` at every source shape
/// measured, literal and typed alike (row 104), so refusing the typed ones breaks
/// `m IN (SELECT s::mood FROM t)`, which 18.6 answers.
#[test]
fn the_in_subquery_cast_door_still_writes() {
    let mut db = seeded();
    db.execute("DELETE FROM t WHERE m IN (SELECT s::text FROM t t2)")
        .expect("the door accepts where 18.6 raises 42883");
    assert_eq!(
        labels(&mut db),
        vec!["sad", "happy"],
        "the `ok` row 18.6 keeps by refusing"
    );

    // The same cast at the two doors that refuse it.
    let mut db = seeded();
    for sql in [
        "DELETE FROM t WHERE m = (SELECT s::text FROM t t2 WHERE id = 1)",
        "UPDATE t SET s = 'hit' FROM (SELECT s::text AS z FROM t t2) v WHERE t.m = v.z",
    ] {
        let err = db.execute(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "42883", "{sql}: {err}");
    }
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);

    // And the accept the door buys with it, which any narrowing must keep: 18.6 resolves
    // `'ok'::mood` to the enum and answers.
    let mut db = seeded();
    db.execute("DELETE FROM t WHERE m IN (SELECT 'ok'::mood)")
        .expect("a cast to the enum itself is what the door's middle state buys");
    assert_eq!(labels(&mut db), vec!["sad", "happy"]);
}

/// **The identity derivation on `Expr::to_column_desc` must not re-enter
/// `Plan::output_columns`, and this is the guard that says so.** `Expr::infer_type`
/// already reaches a scalar subquery by descending into `output_columns`, which builds
/// descriptors back through `to_column_desc`; a walk whose subquery leaf descends the
/// same way makes that two descents per level instead of one, and the cost doubles per
/// level. Measured on a release server over the shape below, the declining leaf this
/// ships is flat — 32 / 32 / 31 / 33 ms at depth 12 / 16 / 18 / 20 — where a descending
/// one is 50 / 316 / 1165 / 4507 ms at the same four and ~17 s at depth 22, against
/// 18.4's 39 ms. The planner's own `describe_over_a_*_is_not_exponential` guards cannot
/// see this: they run over a schema with no enum in it, and the derivation is skipped
/// there.
///
/// A deadline rather than an elapsed-time assertion, because the failure mode is a hang
/// rather than a slowdown — and the deadline has to sit in the **gap** between the two
/// builds rather than at one end of it. Those release figures are not the ones that
/// decide that: this runs under `cargo test`, so the profile it is judged in is the
/// unoptimized one, where the shipped build is `0.01 s` and the descending leaf takes
/// `42.7 s` at depth 22 (measured, by mutating the leaf). `5 s` is ~500× above the pass
/// side and ~8× below the fail side. The `30 s` this carried was 1.4× below the fail
/// side, which is a race on the half that matters: a box 1.4× quicker brings a
/// **regressed** build in under the bound and the guard goes green on the defect it
/// names.
///
/// The shape has to be a **projecting** nest (`… FROM t` at every level), since a
/// no-FROM one is a `Plan::Values` that never calls `to_column_desc` at all and would
/// make the guard vacuous; and `seeded()` creates two enums, so the
/// catalog-holds-no-enum skip does not apply either.
#[test]
fn the_identity_derivation_does_not_re_enter_output_columns() {
    let (tx, rx) = std::sync::mpsc::channel();
    // The nest is ~90 tree levels, past the statement-depth limit a 2 MiB debug thread
    // is given (`pglet_sql::depth`), so it runs on a stack sized for a higher one.
    let spawned = std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(move || {
            pglet_sql::depth::set_max_depth(400);
            let mut db = seeded();
            let mut sql = "SELECT m FROM t WHERE id = 2".to_string();
            for _ in 0..22 {
                sql = format!("SELECT ({sql}) FROM t WHERE id = 1");
            }
            let _ = tx.send(db.execute(&format!("SELECT ({sql}) > 'sad'")).is_ok());
        });
    spawned.expect("spawn");
    let done = rx.recv_timeout(std::time::Duration::from_secs(5));
    assert_eq!(
        done.ok(),
        Some(true),
        "a depth-22 projecting scalar-subquery nest did not plan within 5s — the enum \
         derivation on `Expr::to_column_desc` is descending into sub-plans again"
    );
}

/// Bind `$1` and expect 18.6's `enum_in` refusal of `value` for `mood`.
fn assert_refused_at_bind(db: &mut Database, sql: &str, value: &str) {
    match bind_one(db, sql, value) {
        Err(pglet::BoundExecError::Engine(e)) => {
            assert_eq!(e.sqlstate(), "22P02", "{sql}: {e}");
            assert_eq!(
                e.to_string(),
                format!("invalid input value for enum mood: \"{value}\""),
                "{sql}"
            );
        }
        other => panic!("{sql}: expected the conversion's refusal, got {other:?}"),
    }
    db.execute("ROLLBACK").ok();
}

/// A **cast** to the enum is a conversion too (`EDGES.md` row 241), and each operand kind
/// converts where 18.6 runs `enum_in`: an undeclared `$1::mood` at `Bind`, so over no
/// row; a `text` operand (`$1::text::mood`) on the value, so only when a row reaches it;
/// a string literal in parse analysis, so a `DELETE` filtered on `'zzz'::mood` removes
/// nothing. The survivors and the untouched catalog are what no fixture can read.
#[test]
fn a_cast_to_the_enum_converts_where_18_6_converts() {
    let mut db = seeded();
    db.execute("CREATE TABLE tx (v TEXT)").expect("text sink");
    db.execute("CREATE SEQUENCE zseq").expect("a sequence");
    db.execute("CREATE TABLE empty (id INT)")
        .expect("an empty table");

    for sql in [
        "SELECT $1::mood::text FROM t WHERE false",
        "DELETE FROM t WHERE m = $1::mood",
        "UPDATE t SET s = 'x' WHERE m = $1::mood",
        "INSERT INTO tx SELECT $1::mood FROM t",
    ] {
        assert_refused_at_bind(&mut db, sql, "zzz");
    }
    // Labels are compared as written: a label differing only in case is not a member.
    assert_refused_at_bind(&mut db, "SELECT $1::mood::text FROM t WHERE false", "OK");

    // A `text` operand converts on the value: over no row nothing is converted, over a
    // row the non-member is refused.
    bind_one(
        &mut db,
        "SELECT $1::text::mood::text FROM t WHERE false",
        "zzz",
    )
    .expect("no row reaches the conversion");
    assert_refused_at_bind(&mut db, "SELECT $1::text::mood::text FROM t", "zzz");

    // A literal: refused before anything executes, whatever the predicate selects; and
    // the `ADD COLUMN` default, bare or cast, before its back-fill stores anything.
    for sql in [
        "DELETE FROM t WHERE 'zzz'::mood > 'sad'::mood",
        "INSERT INTO tx SELECT 'zzz'::mood",
        // A column's value converts per row: `s || 'x'` is `okx`, which `mood` lacks.
        "INSERT INTO tx SELECT (s || 'x')::mood FROM t",
        "ALTER TABLE t ADD COLUMN z mood DEFAULT 'zzz'",
        "ALTER TABLE t ADD COLUMN z mood DEFAULT 'zzz'::mood",
    ] {
        let e = db.execute(sql).expect_err(sql);
        assert_eq!(e.sqlstate(), "22P02", "{sql}: {e}");
    }
    // A default the assignment gate reads as plain `text` or `int`, whose back-fill stored
    // the label: 18.6 refuses each `42804` (the assignment has no cast), and pglet refuses
    // it as the enum's `22P02` before the back-fill — `EDGES.md` row 47's recorded code
    // divergence. What this change owns is the refusal and the untouched rows, so either
    // code passes here; the code is the record's, not this test's.
    for sql in [
        "ALTER TABLE t ADD COLUMN z mood DEFAULT 'zzz' || ''",
        "ALTER TABLE t ADD COLUMN z mood DEFAULT 5",
        // …and the defaults the cook does not evaluate — an engine-valued arm, a
        // sequence draw — whose value only the back-fill sees.
        "ALTER TABLE t ADD COLUMN z mood DEFAULT now()",
        "ALTER TABLE t ADD COLUMN z mood DEFAULT gen_random_uuid()",
        "ALTER TABLE t ADD COLUMN z mood DEFAULT 'q' || nextval('zseq')::text",
    ] {
        let e = db.execute(sql).expect_err(sql);
        assert!(
            matches!(e.sqlstate(), "42804" | "22P02"),
            "{sql}: refused, but not as the assignment or the conversion: {e}"
        );
    }
    assert_eq!(row_count(&mut db), 3, "a refused write left the rows");
    assert_eq!(labels(&mut db), vec!["sad", "ok", "happy"]);
    assert!(
        db.execute("SELECT v FROM tx")
            .expect("the sink reads")
            .last()
            .is_some_and(|r| r.rows.is_empty()),
        "a refused write landed rows"
    );
    assert!(
        !db.schema().tables["t"]
            .columns
            .iter()
            .any(|c| c.name == "z"),
        "a refused `ADD COLUMN` back-filled a label the enum does not hold"
    );
    // Over no row there is no back-fill to test, and the default is still refused where
    // the cook evaluates it — the column is not left declaring a default no insert can
    // take.
    let e = db
        .execute("ALTER TABLE empty ADD COLUMN z mood DEFAULT 'zzz' || ''")
        .expect_err("the evaluated default is not a member");
    assert!(matches!(e.sqlstate(), "42804" | "22P02"), "{e}");

    // The accept polarity: a member converts at every one of the same doors.
    bind_one(&mut db, "DELETE FROM t WHERE m = $1::mood", "happy").expect("a member binds");
    assert_eq!(labels(&mut db), vec!["sad", "ok"]);
    db.execute("ALTER TABLE t ADD COLUMN z mood DEFAULT 'ok'")
        .expect("a member default back-fills");
}

/// An undeclared `$1::mood` converts at `Bind` only where it is the occurrence that fixes
/// `$1`'s type — 18.6 fixes an unspecified parameter at its first coercion in parse-analysis
/// order, and a later cast converts a value of that type, on the value. So the guarded
/// idioms of hand-written SQL bind a non-member and never convert it, while the cast that
/// comes first still refuses it over no row.
///
/// The discriminating pair comes first, and it is a relation between the two spellings:
/// the same two occurrences, swapped, flip the verdict. A rule that records the cast
/// whatever else types `$1` refuses both; one that records it only when every occurrence
/// is the cast accepts both.
///
/// Every refusal is over the **empty** `e0`: `AND`/`OR` evaluate every operand here
/// (`EDGES.md` row 172), so over a row a conversion on the value raises the same `22P02`
/// and the assertion could not tell `Bind` from evaluation.
#[test]
fn a_cast_is_the_enum_target_only_where_it_fixes_the_parameter() {
    let mut db = seeded();
    db.execute("CREATE TABLE u (id INT PRIMARY KEY, s TEXT, m mood)")
        .expect("an upsert target");
    db.execute("INSERT INTO u VALUES (1, 'a', 'ok')")
        .expect("one row");
    db.execute("CREATE TABLE e0 (id INT, m mood)")
        .expect("an empty table");

    assert_refused_at_bind(&mut db, "SELECT $1::mood::text, $1::text FROM e0", "zzz");
    bind_one(&mut db, "SELECT $1::text, $1::mood::text FROM e0", "zzz")
        .expect("a text use first: the cast converts a text value no row reaches");

    // Where the cast is first in **parse-analysis** order, which is not this walk's
    // visiting order: the select list before `ORDER BY`, `WHERE` before `RETURNING`, and
    // a comparison's bare operand timed when the comparison resolves — after the cast
    // inside its other operand.
    for sql in [
        "SELECT $1::mood::text FROM e0 ORDER BY $1::text",
        "DELETE FROM e0 WHERE m = $1::mood RETURNING $1::text",
        "SELECT count(*) FROM e0 WHERE $1 = $1::mood",
        "SELECT count(*) FROM e0 WHERE m = $1::mood OR $1 IS NULL",
    ] {
        assert_refused_at_bind(&mut db, sql, "zzz");
    }
    // A `WITH` query is analysed ahead of the statement's own clauses, in the list's
    // analysis order — declaration order, or `TopologicalSort`'s under `RECURSIVE` —
    // wherever it is read and however it runs (inlined or materialized). So a cast in a
    // CTE is first: ahead of a select list that reads the CTE only from a scalar
    // subquery, ahead of a later CTE the statement reads first, and, under `RECURSIVE`,
    // ahead of an earlier-declared CTE that reads it. The accepts, the text use first,
    // are `params/enum_cast_param_order_cte_*`.
    for sql in [
        "WITH a AS (SELECT $1::mood::text AS v FROM e0) SELECT $1::text, (SELECT v FROM a) FROM e0",
        "WITH b AS (SELECT $1::mood::text AS w FROM e0), a AS (SELECT $1::text AS v) \
         SELECT * FROM a, b",
        "WITH d AS (INSERT INTO u (id, s) SELECT 9, $1::mood::text FROM e0 RETURNING s), \
         a AS (SELECT $1::text AS v) SELECT * FROM a, d",
        "WITH RECURSIVE a AS (SELECT $1::text AS v FROM e0 WHERE EXISTS (SELECT 1 FROM b)), \
         b AS (SELECT $1::mood::text AS w FROM e0) SELECT * FROM a",
    ] {
        assert_refused_at_bind(&mut db, sql, "zzz");
    }
    // A bare select-list `$1` and a `$1` under `IS NULL` fix no type, so the cast after them
    // is still the first to. 18.6 refuses both statements `42P08`. The null test's is the
    // parameter left undetermined, which is refused here under that code, before the
    // value is read; the select-list entry's is the parameter's deductions colliding — a
    // check pglet does not make, so what is asserted there is the refusal, not its code
    // (`EDGES.md` row 241's residuals).
    let sql = "SELECT count(*) FROM e0 WHERE $1 IS NULL OR m = $1::mood";
    match bind_one(&mut db, sql, "zzz") {
        Err(pglet::BoundExecError::Engine(e)) => assert_eq!(e.sqlstate(), "42P08", "{sql}"),
        other => panic!("{sql}: {other:?}"),
    }
    db.execute("ROLLBACK").ok();
    let sql = "SELECT $1, $1::mood::text FROM e0";
    assert!(bind_one(&mut db, sql, "zzz").is_err(), "{sql}: accepted");
    db.execute("ROLLBACK").ok();
    // …and where a text use comes first, in the same terms: `WHERE`, `RETURNING` and an
    // `UPDATE … FROM` source before the `SET` list, `HAVING` before `GROUP BY`, the
    // `VALUES` row before the `ON CONFLICT` action, an earlier `CASE` condition, a
    // set operation's left branch.
    for (sql, value) in [
        (
            "SELECT CASE WHEN $1 = 'x' THEN $1::mood END::text FROM t",
            "zzz",
        ),
        (
            "UPDATE u SET m = CASE WHEN $1 = '' THEN NULL ELSE $1::mood END WHERE id = 1",
            "",
        ),
        ("UPDATE u SET m = $1::mood WHERE $1 <> 'zzz'", "zzz"),
        (
            "UPDATE u SET m = $1::mood WHERE false RETURNING $1::text",
            "zzz",
        ),
        (
            "UPDATE u SET m = $1::mood FROM (SELECT $1::text AS x) s WHERE false",
            "zzz",
        ),
        (
            "SELECT count(*) FROM t WHERE false GROUP BY $1::mood HAVING $1::text = 'x'",
            "zzz",
        ),
        (
            "INSERT INTO u (id, s) VALUES (5, $1) ON CONFLICT (id) DO UPDATE SET m = $1::mood",
            "zzz",
        ),
        (
            "SELECT $1::text FROM t WHERE false UNION ALL SELECT $1::mood::text FROM t WHERE false",
            "zzz",
        ),
    ] {
        bind_one(&mut db, sql, value).unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    }
    let result = db
        .execute("SELECT id, m::text, s FROM u ORDER BY id")
        .expect("reads");
    assert_eq!(
        result.last().expect("one result").rows,
        vec![
            vec![Value::Int4(1), Value::Null, Value::Text("a".into())],
            vec![Value::Int4(5), Value::Null, Value::Text("zzz".into())],
        ],
        "the empty-means-NULL update stored NULL, the guarded update touched nothing, \
         and the upsert inserted its row without reaching the action"
    );
    // A text use first leaves the conversion on the value: a row that reaches it refuses.
    let e = bind_one(&mut db, "SELECT $1::text, $1::mood::text FROM t", "zzz")
        .expect_err("a row reaches the conversion");
    assert!(
        matches!(&e, pglet::BoundExecError::Engine(e) if e.sqlstate() == "22P02"),
        "{e:?}"
    );
}

/// A persisted expression converting a non-constant operand to an enum names the type
/// (`StringFunc::EnumIn`), so `DROP TYPE` refuses it `2BP01` with 18.6's detail — at a
/// `CHECK` and at a `DEFAULT` — and `CASCADE` drops the dependent: the constraint goes,
/// the default is cleared, and the table, its columns and its rows stay. What no fixture
/// reads: the catalog after the refusal, and a block's rollback putting the dependents
/// back.
#[test]
fn drop_type_refuses_an_expression_that_converts_to_it() {
    let mut db = Database::with_btree();
    for (setup, dependent) in [
        (
            "CREATE TABLE d1 (s TEXT CHECK (s::lone IS NOT NULL))",
            "constraint d1_s_check on table d1",
        ),
        (
            "CREATE TABLE d1 (s TEXT DEFAULT ('a'::text)::lone)",
            "default value for column s of table d1",
        ),
    ] {
        db.execute("CREATE TYPE lone AS ENUM ('a')")
            .expect("the type");
        db.execute(setup).expect(setup);
        db.execute("INSERT INTO d1 VALUES ('a')").expect("a row");
        let e = db.execute("DROP TYPE lone").expect_err(setup);
        assert_eq!(e.sqlstate(), "2BP01", "{setup}: {e}");
        assert_eq!(
            e.to_string(),
            "cannot drop type lone because other objects depend on it"
        );
        assert_eq!(
            e.detail().as_deref(),
            Some(format!("{dependent} depends on type lone").as_str())
        );
        assert!(
            db.schema().enums.contains_key("lone"),
            "{setup}: the type survives"
        );
        // A rolled-back cascade puts the dependent and the type back.
        db.execute("BEGIN").expect("a block");
        db.execute("DROP TYPE lone CASCADE").expect("the cascade");
        db.execute("ROLLBACK").expect("rolled back");
        assert!(
            db.execute("DROP TYPE lone").is_err(),
            "{setup}: the rollback restored the dependent"
        );
        db.execute("DROP TYPE lone CASCADE")
            .expect("CASCADE drops the dependent and the type");
        let table = &db.schema().tables["d1"];
        assert!(table.checks.is_empty(), "{setup}: the constraint is gone");
        assert!(
            table.columns.iter().all(|c| c.default.is_none()),
            "{setup}: the default is cleared"
        );
        assert!(!db.schema().enums.contains_key("lone"));
        db.execute("INSERT INTO d1 VALUES ('zzz')")
            .expect("nothing converts to the dropped type any more");
        db.execute("DROP TABLE d1").expect("clean up");
    }
    // Beside a column declaring the type, 18.6's detail lists the column too; this engine
    // does not refuse over a column (`EDGES.md` row 261), so it omits the detail rather
    // than send a partial list.
    db.execute("CREATE TYPE lone AS ENUM ('a')")
        .expect("the type");
    db.execute("CREATE TABLE d3 (m lone, s TEXT CHECK (s::lone IS NOT NULL))")
        .expect("a declaring column and a converting check");
    let e = db
        .execute("DROP TYPE lone")
        .expect_err("the check depends on it");
    assert_eq!(e.sqlstate(), "2BP01", "{e}");
    assert_eq!(e.detail(), None, "a partial dependent list");
    db.execute("DROP TABLE d3").expect("clean up");
    // An index cannot hold such a conversion at all: 18.6 refuses it `42P17`, `enum_in`
    // not being immutable, and so does this — before anything is written.
    db.execute("CREATE TABLE d1 (s TEXT)").expect("a table");
    for sql in [
        "CREATE INDEX d1_ix ON d1 (s) WHERE s::lone IS NOT NULL",
        "CREATE INDEX d1_kx ON d1 ((s::lone))",
    ] {
        let e = db.execute(sql).expect_err(sql);
        assert_eq!(e.sqlstate(), "42P17", "{sql}: {e}");
    }
    assert!(
        db.schema().indexes.is_empty(),
        "a refused index was recorded"
    );
    db.execute("DROP TABLE d1").expect("clean up");
    // A member **literal** cast is a plain `text` cast here and names no type, so the
    // drop proceeds; 18.6 records the dependency and refuses it `2BP01`. That is a
    // recorded divergence (`EDGES.md` row 241's residuals), asserted as the record states
    // it — not the requirement.
    db.execute("CREATE TABLE d2 (s TEXT CHECK (s <> 'a'::lone::text))")
        .expect("a member literal");
    db.execute("DROP TYPE lone")
        .expect("recorded divergence: a member literal records no dependency here");
}
