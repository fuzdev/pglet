//! Identifier case folding — the guards the differential fixture harness cannot carry.
//!
//! Most of this rule is pinned by `tests/fixtures/identifier/*` against a live
//! Postgres. Three classes are not reachable there, and each is here for a different
//! structural reason:
//!
//! - **A refusal has nowhere to observe what it did not destroy.** A fixture's `@test`
//!   is the statement under test, so `DROP SCHEMA "PUBLIC" CASCADE` can pin its
//!   `3F000` and nothing else. That the *rows survive* it — the whole point of the
//!   defect — needs a second statement after the failing one.
//! - **A pglet SQLSTATE that does not match Postgres's.** `@expect-error` requires the
//!   two to agree whenever Postgres also errors, so a quoted spelling pglet refuses
//!   with a *different* code than 18.6 cannot be a fixture at all. Each such case says
//!   below what both engines answer.
//! - **`COPY … FROM STDIN`**, which the fixture grammar does not speak.
//!
//! Everything here runs over `Database::with_btree` — the in-memory CoW B+tree the
//! server uses — rather than `Database::in_memory`'s `MemoryStorage`, which hides
//! rollback and storage-drop behaviour.

// A test harness: `.unwrap()`/`.expect()`/`panic!` surface a setup failure — or a
// statement that unexpectedly succeeded — loudly right here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::Value;

/// Read a single scalar cell from a one-statement query.
fn scalar(db: &mut Database, sql: &str) -> Value {
    let mut results = db.execute(sql).expect("query");
    results.remove(0).rows[0][0].clone()
}

/// The SQLSTATE a statement fails with, or a panic naming what it returned instead.
fn sqlstate(db: &mut Database, sql: &str) -> String {
    match db.execute(sql) {
        Ok(_) => panic!("{sql} unexpectedly succeeded"),
        Err(e) => e.sqlstate().to_string(),
    }
}

/// `DROP SCHEMA "PUBLIC" CASCADE` refuses **and leaves every row where it was.**
///
/// The refusal alone is a fixture (`identifier/drop_schema_quoted_public_loud.sql`);
/// this is the half it cannot express. `execute_drop_schema` used to compare its
/// argument case-insensitively, so the quoted spelling matched the implicit schema and
/// cleared the entire catalog *and its storage* — the most destructive reachable
/// consequence of comparing an identifier the wrong way. Asserting only the error would
/// still pass an engine that raised after wiping the tables.
#[test]
fn quoted_public_drop_schema_refuses_and_keeps_the_rows() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE keepme (a INT)").unwrap();
    db.execute("INSERT INTO keepme VALUES (1), (2), (3)")
        .unwrap();

    assert_eq!(
        sqlstate(&mut db, r#"DROP SCHEMA "PUBLIC" CASCADE"#),
        "3F000"
    );

    assert_eq!(
        scalar(&mut db, "SELECT count(*) FROM keepme"),
        Value::Int8(3)
    );
    // …and the table itself is still in the catalog, not merely emptied.
    assert!(db.schema().get_table("keepme").is_some());

    // The accept side, in the same session: the unquoted upper-case spelling still
    // means the implicit schema, so a repair that made the test exact without folding
    // the identifier fails here.
    db.execute("DROP SCHEMA PUBLIC CASCADE").unwrap();
    assert!(db.schema().get_table("keepme").is_none());
}

/// A quoted function name is a different function, and pglet refuses it.
///
/// 18.6: `42883 function NOW() does not exist` / `function COUNT() does not exist`.
/// pglet: `0A000` — its `build_call` catch-all for an unresolved name — so the codes
/// disagree and no `@expect-error` fixture can hold this. What matters for the fold is
/// that a quoted spelling no longer resolves to the builtin, which used to make
/// `SELECT "NOW"()` answer a timestamp and `SELECT "COUNT"(*)` answer `0`.
#[test]
fn a_quoted_function_name_does_not_resolve_to_the_builtin() {
    let mut db = Database::with_btree();
    for sql in [
        r#"SELECT "NOW"()"#,
        r#"SELECT "COUNT"(*)"#,
        r#"SELECT "LOWER"('A')"#,
        // The window-function dispatch is a separate `match` from `build_call`'s and
        // needs its own probe (18.6: `42883 function ROW_NUMBER() does not exist`).
        r#"SELECT "ROW_NUMBER"() OVER () FROM (SELECT 1) s"#,
    ] {
        assert!(db.execute(sql).is_err(), "{sql} must not resolve");
    }
    // The accept side: an unquoted spelling in any case still resolves, because the
    // parser downcased it before dispatch.
    assert_eq!(
        scalar(&mut db, "SELECT COUNT(*) FROM (SELECT 1) s"),
        Value::Int8(1)
    );
    assert_eq!(
        scalar(&mut db, "SELECT LoWeR('A')"),
        Value::Text("a".into())
    );
    db.execute("SELECT Now()").unwrap();
    db.execute("SELECT Row_Number() OVER () FROM (SELECT 1) s")
        .unwrap();
}

/// A quoted SQL **value function** spelling is an ordinary column reference.
///
/// `CURRENT_DATE` and friends are keywords to Postgres, so `"CURRENT_DATE"` is an
/// identifier there and resolves as a column (`42703` with no FROM clause). pglet
/// raises its own column-not-found rather than silently evaluating the special form,
/// which is what `SqlValueFunction::from_name`'s fold used to do.
///
/// The two spellings below are **not** equal in strength, and which is which matters
/// to anyone changing `parse_identifier_primary`. `SqlValueFunction::from_name`
/// matches lower-case names exactly, so `"CURRENT_DATE"` misses it on the name alone
/// and would still fail with the quotedness bit deleted — it is the weak half, kept
/// because it is the spelling a reader reaches for. `"current_timestamp"` matches the
/// table by name, so **only** the `!quoted` guard stops it: that assertion is the
/// discriminator.
#[test]
fn a_quoted_value_function_spelling_is_not_the_value_function() {
    let mut db = Database::with_btree();
    assert!(db.execute(r#"SELECT "CURRENT_DATE""#).is_err());
    assert!(db.execute(r#"SELECT "current_timestamp""#).is_err());
    // Unquoted, in any case, is still the value function.
    db.execute("SELECT Current_Date").unwrap();
    db.execute("SELECT CURRENT_TIMESTAMP").unwrap();
}

/// A quoted access-method name is not the method.
///
/// 18.6: `42704 access method "GIN" does not exist`. pglet: the `0A000` it already
/// gives every unrecognized method (`USING hash`, `USING nosuchmethod` — both `42704`
/// there), so this is a spelling joining an existing set rather than a new answer, and
/// the codes disagree with Postgres either way.
#[test]
fn a_quoted_index_method_is_not_the_method() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE g1 (j JSONB)").unwrap();
    assert!(
        db.execute(r#"CREATE INDEX g1_idx ON g1 USING "GIN" (j)"#)
            .is_err()
    );
    // Unquoted, in any case, still selects GIN.
    db.execute("CREATE INDEX g1_ok ON g1 USING GiN (j)")
        .unwrap();
}

/// A quoted `ON CONFLICT` arbiter column is a different column, so the upsert refuses
/// **and writes nothing.**
///
/// 18.6: `42703 column "userid" does not exist` (it resolves the arbiter against the
/// target's scope during parse analysis). pglet: `42P10`, since it matches the arbiter
/// against its indexes and finds none — the codes disagree, so this cannot be a
/// fixture. The state assertion is the load-bearing half: `conflict_target_matches`
/// and `apply_do_update` both compared case-insensitively, so this statement used to
/// match the `"UserId"` index *and run the `DO UPDATE`*, writing to `"Val"`.
///
/// **Which of those two gates this test still reaches is only the arbiter's.** The
/// arbiter refuses first, so `apply_do_update` never runs — measured: restoring its
/// `eq_ignore_ascii_case` alone leaves this test, and the other six here, green. Its
/// independent guard is
/// `tests/fixtures/identifier/on_conflict_set_target_is_exact_loud.sql`, which spells
/// the arbiter exactly so the SET target is the only thing left to decide.
#[test]
fn a_mismatched_case_conflict_arbiter_refuses_and_writes_nothing() {
    let mut db = Database::with_btree();
    db.execute(r#"CREATE TABLE ci ("UserId" INT PRIMARY KEY, "Val" TEXT)"#)
        .unwrap();
    db.execute(r#"INSERT INTO ci ("UserId", "Val") VALUES (1, 'a')"#)
        .unwrap();

    assert!(
        db.execute(
            r#"INSERT INTO ci ("UserId", "Val") VALUES (1, 'b')
               ON CONFLICT (userid) DO UPDATE SET val = 'z'"#
        )
        .is_err()
    );
    assert_eq!(
        scalar(&mut db, r#"SELECT "Val" FROM ci"#),
        Value::Text("a".into())
    );

    // The accept side, quoted on both sides: the arbiter and the SET target match the
    // catalog exactly, so the upsert runs.
    db.execute(
        r#"INSERT INTO ci ("UserId", "Val") VALUES (1, 'b')
           ON CONFLICT ("UserId") DO UPDATE SET "Val" = 'z'"#,
    )
    .unwrap();
    assert_eq!(
        scalar(&mut db, r#"SELECT "Val" FROM ci"#),
        Value::Text("z".into())
    );
}

/// A quoted `COPY` format name is not the format.
///
/// 18.6: `22023 COPY format "TEXT" not recognized`; pglet raises its own `42601`-shaped
/// "not recognized", the code it already gives an unrecognized bare format. The
/// fixture grammar has no `COPY … FROM STDIN` form for the differential harness, so the
/// parse is asserted directly.
#[test]
fn a_quoted_copy_format_is_not_the_format() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE cpt (a INT)").unwrap();
    // **Both** polarities are asserted at the *parse*, not through `execute`: `COPY …
    // FROM STDIN` needs the wire protocol to carry its bytes, so `execute` refuses it
    // whatever the format name says — which makes an `is_err()` on the reject side
    // vacuous (measured: reverting this gate reddened nothing until the assertion
    // moved here). The format name is resolved during the parse, so that is where the
    // difference is observable.
    assert!(
        pglet_sql::parser::parse(r#"COPY cpt FROM STDIN WITH (FORMAT "TEXT")"#, db.schema())
            .is_err(),
        "a quoted format name must not resolve"
    );
    pglet_sql::parser::parse(r"COPY cpt FROM STDIN WITH (FORMAT Text)", db.schema())
        .expect("an unquoted format name resolves in any case");
    // The option *name* itself is a `ColLabel` on the same rule.
    assert!(
        pglet_sql::parser::parse(r#"COPY cpt FROM STDIN WITH ("FORMAT" text)"#, db.schema())
            .is_err(),
        "a quoted option name must not resolve"
    );
}

/// A `--data-dir` store written before the fold cannot be reopened.
///
/// The store-magic bump (`PGMG` → `PGMH`) is what stands between an upgrading user and
/// a silent one: a table a pre-fold build registered as `Users` is still reachable as
/// `"Users"`, but `CREATE TABLE IF NOT EXISTS Users` now mints a second, empty `users`
/// that shadows it, with nothing left in the store to detect it by. This asserts the
/// magic actually moved — a bump left out of `STALE_MAGICS`, or not applied at all,
/// makes the old file open cleanly.
#[test]
fn a_pre_fold_store_magic_is_rejected() {
    use pglet_storage::btree::DoubleMetaStore;
    use pglet_testing::block_io::MemBlockIo;

    // Two 64 KiB meta slots — the whole pre-data region, so the file is non-empty
    // enough to be read as a store rather than as a fresh one.
    let mut bytes = vec![0u8; 2 * 64 * 1024];
    bytes[0..4].copy_from_slice(b"PGMG");
    let err = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(bytes)))
        .expect_err("a PGMG store must not reopen");
    assert!(
        format!("{err:?}").contains("stale on-disk format"),
        "expected a stale-format error, got {err:?}"
    );
}
