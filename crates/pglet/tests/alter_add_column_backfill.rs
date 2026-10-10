//! `ALTER TABLE … ADD COLUMN` end-to-end over the embed engine: DEFAULT backfill of
//! existing rows, inline `UNIQUE`/`CHECK` enforcement, the NOT-NULL-without-default
//! rejection, and the `RENAME`-loud / inert-clause-no-op split.
//!
//! These are the in-process counterparts to the differential SQL fixtures under
//! `tests/fixtures/ddl/` (`alter_addcol_*` / `alter_rename_loud` / `alter_inert_noop`);
//! each behavior is validated byte-for-byte against a live Postgres there and
//! exercised without a server here.

// A test harness: `.unwrap()`/`panic!` surface a setup failure or an unexpected cell
// shape loudly right here.
#![allow(clippy::unwrap_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::Value;

/// Run a query and return its single scalar `i64` (a `count(*)` / integer cell).
fn scalar_i64(db: &mut Database, sql: &str) -> i64 {
    let mut results = db.execute(sql).unwrap();
    let result = results.remove(0);
    match result.rows[0][0] {
        Value::Int8(n) => n,
        Value::Int4(n) => i64::from(n),
        ref other => panic!("expected an integer scalar, got {other:?}"),
    }
}

/// A constant `DEFAULT` backfills every existing row (Postgres fills them, not NULL).
#[test]
fn add_column_const_default_backfills_existing_rows() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

    db.execute("ALTER TABLE t ADD COLUMN c INT DEFAULT 5")
        .unwrap();

    // All three pre-existing rows carry the default, none NULL.
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t WHERE c = 5"), 3);
    assert_eq!(
        scalar_i64(&mut db, "SELECT count(*) FROM t WHERE c IS NULL"),
        0
    );
    // A subsequent insert with no value for `c` still resolves the default.
    db.execute("INSERT INTO t (id) VALUES (4)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t WHERE c = 5"), 4);
}

/// `DEFAULT … NOT NULL` backfills, and the column is non-null after (valid).
#[test]
fn add_column_not_null_with_default_backfills() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();

    db.execute("ALTER TABLE t ADD COLUMN d INT DEFAULT 9 NOT NULL")
        .unwrap();

    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t WHERE d = 9"), 2);
    assert_eq!(
        scalar_i64(&mut db, "SELECT count(*) FROM t WHERE d IS NULL"),
        0
    );
}

/// A volatile `gen_random_uuid()` default is evaluated **per row** — every existing
/// row gets a distinct UUID.
#[test]
fn add_column_uuid_default_is_distinct_per_row() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

    db.execute("ALTER TABLE t ADD COLUMN u UUID DEFAULT gen_random_uuid()")
        .unwrap();

    // Three rows, three distinct non-NULL uuids.
    assert_eq!(
        scalar_i64(&mut db, "SELECT count(DISTINCT u) FROM t"),
        3,
        "each backfilled row gets a fresh uuid"
    );
    assert_eq!(
        scalar_i64(&mut db, "SELECT count(*) FROM t WHERE u IS NULL"),
        0
    );
}

/// A `SERIAL` (nextval) default backfills sequential values, and the sequence
/// continues from there for later inserts.
#[test]
fn add_column_serial_default_backfills_sequentially() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

    db.execute("ALTER TABLE t ADD COLUMN s SERIAL").unwrap();

    // Backfilled 1,2,3 across the three rows (distinct, min 1, max 3).
    assert_eq!(scalar_i64(&mut db, "SELECT count(DISTINCT s) FROM t"), 3);
    assert_eq!(scalar_i64(&mut db, "SELECT min(s) FROM t"), 1);
    assert_eq!(scalar_i64(&mut db, "SELECT max(s) FROM t"), 3);
    // The next insert continues the sequence at 4.
    db.execute("INSERT INTO t (id) VALUES (4)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT s FROM t WHERE id = 4"), 4);
}

/// `ADD COLUMN … NOT NULL` with **no** default on a non-empty table is `23502` —
/// existing rows can't be filled (matching Postgres), not a silent store of NULLs.
#[test]
fn add_column_not_null_no_default_on_nonempty_is_23502() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();

    let err = db
        .execute("ALTER TABLE t ADD COLUMN e INT NOT NULL")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "23502", "{err}");

    // The failed ADD left the catalog untouched — the column is not present.
    assert_eq!(
        scalar_i64(&mut db, "SELECT count(*) FROM t"),
        1,
        "the table is unchanged"
    );
    // Re-adding it nullable (or with a default) still works — no phantom column.
    db.execute("ALTER TABLE t ADD COLUMN e INT").unwrap();
}

/// `ADD COLUMN … NOT NULL` with no default on an **empty** table succeeds (no rows
/// to violate), matching Postgres.
#[test]
fn add_column_not_null_no_default_on_empty_is_ok() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("ALTER TABLE t ADD COLUMN e INT NOT NULL")
        .unwrap();
    // The new NOT NULL column now rejects a NULL insert.
    let err = db.execute("INSERT INTO t (id) VALUES (1)").unwrap_err();
    assert_eq!(err.sqlstate(), "23502", "{err}");
}

/// An inline `UNIQUE` on the added column is **enforced**: a later duplicate
/// insert is `23505`, not silently accepted.
#[test]
fn add_column_inline_unique_is_enforced() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();

    db.execute("ALTER TABLE t ADD COLUMN email TEXT UNIQUE")
        .unwrap();

    db.execute("INSERT INTO t VALUES (3, 'x')").unwrap();
    let dup = db.execute("INSERT INTO t VALUES (4, 'x')").unwrap_err();
    assert_eq!(dup.sqlstate(), "23505", "{dup}");
}

/// An inline `CHECK` on the added column is **enforced**: a violating insert
/// is `23514`.
#[test]
fn add_column_inline_check_is_enforced() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();

    db.execute("ALTER TABLE t ADD COLUMN val INT CHECK (val > 0)")
        .unwrap();

    db.execute("INSERT INTO t VALUES (2, 5)").unwrap();
    let bad = db.execute("INSERT INTO t VALUES (3, -5)").unwrap_err();
    assert_eq!(bad.sqlstate(), "23514", "{bad}");
}

/// Enforcement meeting the backfill: a `DEFAULT` whose backfilled value violates the
/// new column's own inline `CHECK` aborts loud (`23514`) — the case the old
/// NULL-punt sidestepped.
#[test]
fn add_column_check_violated_by_default_is_23514() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();

    let err = db
        .execute("ALTER TABLE t ADD COLUMN val INT CHECK (val > 10) DEFAULT 5")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "23514", "{err}");
}

/// A constant `DEFAULT` under an inline `UNIQUE` collides across >1 existing row
/// (`23505`), matching Postgres's "could not create unique index".
#[test]
fn add_column_unique_collides_on_constant_default() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();

    let err = db
        .execute("ALTER TABLE t ADD COLUMN email TEXT UNIQUE DEFAULT 'x'")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "23505", "{err}");

    // Over a single row the same shape succeeds (no collision).
    let mut db1 = Database::with_btree();
    db1.execute("CREATE TABLE t1 (id INT)").unwrap();
    db1.execute("INSERT INTO t1 VALUES (1)").unwrap();
    db1.execute("ALTER TABLE t1 ADD COLUMN email TEXT UNIQUE DEFAULT 'x'")
        .unwrap();
    assert_eq!(scalar_i64(&mut db1, "SELECT count(*) FROM t1"), 1);
}

/// `ALTER TABLE … RENAME` fails loud (`0A000`) instead of the old silent no-op that
/// reported success while renaming nothing.
#[test]
fn alter_table_rename_is_loud() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (a INT, b INT)").unwrap();

    for sql in [
        "ALTER TABLE t RENAME TO t2",
        "ALTER TABLE t RENAME COLUMN a TO c",
        "ALTER TABLE t RENAME CONSTRAINT x TO y",
        "ALTER TABLE ONLY t RENAME TO t2",
    ] {
        let err = db.execute(sql).unwrap_err();
        assert_eq!(err.sqlstate(), "0A000", "{sql}: {err}");
    }

    // The table was never renamed — it is still reachable under its original name.
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t"), 0);
    // A genuinely-inert pg_dump-preamble ALTER clause still succeeds as a no-op.
    db.execute("ALTER TABLE t SET (fillfactor = 70)").unwrap();
    db.execute("ALTER TABLE t OWNER TO some_role").unwrap();
    // ...and the table is still there afterward.
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t"), 0);
}

/// `ADD COLUMN … NOT NULL DEFAULT NULL` over a non-empty table is `23502` — the
/// integrity violation this test locks.
///
/// 18.6: `ERROR: 23502 column "c" of relation "t" contains null values`, raised in
/// `ATRewriteTable`. pglet used to accept it and write a NULL into every existing row
/// of a `NOT NULL` column, because the gate tested whether a DEFAULT was *present*
/// rather than what it *resolved to*, and `DEFAULT NULL` is present.
///
/// **The row survival is why this is a unit test and not only a fixture.** An
/// `@expect-error` fixture compares SQLSTATEs and scores green for a refusal raised
/// *after* the writes; and the harness wraps every fixture in `BEGIN … ROLLBACK`, so
/// it cannot see that the failed statement left the catalog and the rows alone. Both
/// are asserted here.
#[test]
fn add_column_not_null_default_null_on_nonempty_is_23502() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();

    let err = db
        .execute("ALTER TABLE t ADD COLUMN c INT NOT NULL DEFAULT NULL")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "23502", "{err}");

    // The rows are intact and the column never landed — no phantom, and no widened
    // rows left behind by a refusal raised after the write.
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t"), 2);
    assert_eq!(scalar_i64(&mut db, "SELECT sum(id) FROM t"), 3);
    let gone = db.execute("SELECT c FROM t").unwrap_err();
    assert_eq!(gone.sqlstate(), "42703", "{gone}");
    // The name is free — re-adding it nullable works.
    db.execute("ALTER TABLE t ADD COLUMN c INT").unwrap();
    assert_eq!(
        scalar_i64(&mut db, "SELECT count(*) FROM t WHERE c IS NULL"),
        2
    );
}

/// The same statement over an **empty** table succeeds, on both engines (measured:
/// 18.6 accepts it, records no `pg_attrdef` row and leaves `attnotnull` set). The
/// control against over-rejecting: a values-based gate that fired on the *default*
/// rather than on the backfilled rows would break this.
#[test]
fn add_column_not_null_default_null_on_empty_is_ok() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("ALTER TABLE t ADD COLUMN c INT NOT NULL DEFAULT NULL")
        .unwrap();
    // `DEFAULT NULL` is no default, so an omitted insert hits the NOT NULL.
    let err = db.execute("INSERT INTO t (id) VALUES (1)").unwrap_err();
    assert_eq!(err.sqlstate(), "23502", "{err}");
    db.execute("INSERT INTO t (id, c) VALUES (1, 7)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT c FROM t"), 7);
}

/// `CREATE TABLE t (c INT NOT NULL DEFAULT NULL)` stays **accepted** — 18.6 accepts it
/// too (there are no rows to backfill) and raises `23502` at the first omitting
/// insert. The `CREATE TABLE`-side control for the gate above.
#[test]
fn create_table_not_null_default_null_is_accepted() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, c INT NOT NULL DEFAULT NULL)")
        .unwrap();
    let err = db.execute("INSERT INTO t (id) VALUES (1)").unwrap_err();
    assert_eq!(err.sqlstate(), "23502", "{err}");
    db.execute("INSERT INTO t VALUES (1, 5)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT c FROM t"), 5);
}

/// A NOT NULL backfill violation outranks an inline `CHECK` violation on the same
/// statement, in **either** textual order — 18.6's `ATRewriteTable` tests the not-null
/// attributes (`tablecmds.c:6465`) before the constraint list (`:6502`), measured.
///
/// **The CHECK has to be `c IS NOT NULL`, and that is the whole discriminating point.**
/// The obvious spelling `CHECK (c > 100)` proves nothing about ordering: a NULL makes
/// that predicate NULL, which is not a CHECK violation on either engine, so the
/// `23502` wins wherever the gate sits. `IS NOT NULL` is the one predicate a NULL
/// backfill actually *violates*, so only it puts the two errors in genuine
/// competition — measured as a mutation, moving pglet's gate after the constraint
/// validation leaves the `c > 100` spelling green and turns this one red.
///
/// The nullable spelling is the control that identifies the ordering rather than the
/// gate: drop the `NOT NULL` and the same statement is `23514` on both engines.
#[test]
fn a_not_null_backfill_violation_outranks_an_inline_check() {
    for spelling in [
        "ALTER TABLE t ADD COLUMN c INT NOT NULL DEFAULT NULL CHECK (c IS NOT NULL)",
        "ALTER TABLE t ADD COLUMN c INT CHECK (c IS NOT NULL) NOT NULL DEFAULT NULL",
        // The non-discriminating spellings, kept because they are the shapes a reader
        // reaches for: both engines answer `23502`, but for the reason above they say
        // nothing about where the gate runs.
        "ALTER TABLE t ADD COLUMN c INT NOT NULL DEFAULT NULL CHECK (c > 100)",
        "ALTER TABLE t ADD COLUMN c INT CHECK (c > 100) NOT NULL DEFAULT NULL",
    ] {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1), (2)").unwrap();
        let err = db.execute(spelling).unwrap_err();
        assert_eq!(err.sqlstate(), "23502", "{spelling}: {err}");
    }
    // Control 1 — the same CHECK without the NOT NULL is `23514` (measured on 18.6).
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();
    let err = db
        .execute("ALTER TABLE t ADD COLUMN c INT CHECK (c IS NOT NULL)")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "23514", "{err}");
    // Control 2 — with the NOT NULL satisfied by a real default, the CHECK answers.
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();
    let err = db
        .execute("ALTER TABLE t ADD COLUMN c INT NOT NULL DEFAULT 5 CHECK (c > 100)")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "23514", "{err}");
}

/// A `clock_timestamp()` default is resolved **per backfilled row**, where the other
/// four `now`-family spellings take one value for the whole statement.
///
/// 18.6's rule is the function's volatility: `clock_timestamp()` is VOLATILE and
/// forces a rewrite that calls it per row (measured on 18.6 on one near-idle host: 3–4
/// distinct instants over 5 rows, 6–7 over 20), while `now()` / `transaction_timestamp()` are
/// transaction-stable and `statement_timestamp()` / `CURRENT_TIMESTAMP`
/// statement-stable, so one `ALTER` backfills a single value for each (measured: 1).
///
/// **The distinctness assertion is `> 1`, not `= 40`, and that is not a hedge.**
/// Consecutive clock reads can share a microsecond, so the count is the build's and the
/// host's: over 40 rows on one near-idle host, 18.6 gave 11 to 13, a release pglet server 8
/// to 11 and a debug one 40, so `= 40` is red there on 18.6 and on a release build. The
/// `> 1` form is what both engines satisfy; the deterministic pin on the classifier itself is
/// `executor::ddl::backfill_volatility_tests`, which cannot flake at all.
#[test]
fn add_column_clock_timestamp_default_is_resolved_per_row() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    for i in 1..=40 {
        db.execute(&format!("INSERT INTO t VALUES ({i})")).unwrap();
    }
    db.execute("ALTER TABLE t ADD COLUMN c TIMESTAMPTZ DEFAULT clock_timestamp()")
        .unwrap();
    assert!(
        scalar_i64(&mut db, "SELECT count(DISTINCT c) FROM t") > 1,
        "clock_timestamp() must be re-read per backfilled row"
    );

    // The other four are one value across the whole backfill — the control that
    // identifies the volatility classification rather than the clock.
    for spelling in [
        "now()",
        "transaction_timestamp()",
        "statement_timestamp()",
        "CURRENT_TIMESTAMP",
    ] {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT)").unwrap();
        for i in 1..=40 {
            db.execute(&format!("INSERT INTO t VALUES ({i})")).unwrap();
        }
        db.execute(&format!(
            "ALTER TABLE t ADD COLUMN c TIMESTAMPTZ DEFAULT {spelling}"
        ))
        .unwrap();
        assert_eq!(
            scalar_i64(&mut db, "SELECT count(DISTINCT c) FROM t"),
            1,
            "{spelling} is stable within one statement"
        );
    }
}

/// `ADD COLUMN … SERIAL NOT NULL` over a non-empty table still **succeeds** — the
/// regression the NOT NULL gate has to keep clearing.
///
/// The gate judges the resolved backfill values, which for a `SERIAL` are the
/// sequence's, so it does not fire. Reading the *planner*'s `ColumnDef.default`
/// instead would: an implicit-sequence column carries `default: None` on the parse
/// side (the sequence name is the executor's to mint), so a gate keyed there refuses
/// ordinary DDL. The old presence-keyed gate carried that reason in a comment and
/// nothing tested it.
#[test]
fn add_column_serial_not_null_on_a_nonempty_table_is_accepted() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();
    db.execute("ALTER TABLE t ADD COLUMN s SERIAL NOT NULL")
        .unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT count(DISTINCT s) FROM t"), 2);
    assert_eq!(
        scalar_i64(&mut db, "SELECT count(*) FROM t WHERE s IS NULL"),
        0
    );
}
