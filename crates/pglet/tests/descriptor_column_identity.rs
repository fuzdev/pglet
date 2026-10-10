//! A projected column's descriptor is the descriptor of the column the projection
//! **reads** — its type OID, modifier, provenance and enum identity — on the planner's
//! `Describe` list as on the executor's simple-query one.
//!
//! **Why this sits beside the differential fixtures rather than inside them.** The
//! fixtures compare each column's declared type OID on both legs, which is enough to see a
//! `text` column declared `int4`; they compare no modifier and no provenance, so a
//! `varchar(8)` column declared `varchar(5)` — or declared with another relation's table
//! OID and attnum — is invisible to them. Each test here compares the two producers
//! **field for field** against one expected descriptor, so a planner that agrees with a
//! wrong executor is red too. The expected type OID and modifier are the values 18.6 sends
//! (measured on a raw `RowDescription`). The expected provenance is not 18.6's numbers,
//! which are that server's own OIDs: it is the `(table_oid, attnum)` pglet's planner
//! declares for a plain `SELECT c FROM t` of the column 18.6 names, or `(0, 0)` where 18.6
//! sends none. The expected enum is the referenced column's.
//!
//! The two shapes are two FROM relations carrying the same column name, which is what a
//! lookup by bare name cannot tell apart: a window query's projected column
//! (`Plan::Window`'s descriptors), and a qualified reference to a FROM-clause `unnest`
//! (a relation the planner's scope has to register to be looked up by its qualifier).

// A test harness: setup `.unwrap()`s, so a setup failure surfaces loudly here rather
// than as a confusing assertion mismatch.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pglet::Database;
use pglet_sql::executor::ColumnDesc;
use pglet_sql::types::TypeOid;

/// Every descriptor field, from **each** producer, for one statement: the plan-side list
/// `Describe` sends and the executed one the simple-query `RowDescription` sends.
fn both_producers(db: &mut Database, sql: &str) -> (Vec<ColumnDesc>, Vec<ColumnDesc>) {
    let planned = {
        let schema = db.schema();
        let plans = pglet_sql::parser::parse(sql, schema).expect("should plan");
        assert_eq!(plans.len(), 1, "expected one statement for {sql:?}");
        plans[0].result_shape(schema).into_columns()
    };
    let executed = db
        .execute(sql)
        .expect("should execute")
        .remove(0)
        .shape
        .into_columns();
    (planned, executed)
}

/// The catalog's `(table_oid, attnum)` for `table.column` — read off a plain projection of
/// that one column, whose descriptor is the catalog's own.
fn provenance(db: &mut Database, table: &str, column: &str) -> (i32, i16) {
    let (planned, _) = both_producers(db, &format!("SELECT {column} FROM {table}"));
    (planned[0].table_oid, planned[0].column_id)
}

/// `(type_oid, type_modifier, (table_oid, attnum), enum_type)` — the whole identity.
type Identity = (TypeOid, i32, (i32, i16), Option<String>);

fn identity(c: &ColumnDesc) -> Identity {
    (
        c.type_oid,
        c.type_modifier,
        (c.table_oid, c.column_id),
        c.enum_type.clone(),
    )
}

/// Two relations sharing every column name, at **different** attnums, with a different
/// type (`k`), a different modifier over the same type (`v`, `n`) and an enum on one side
/// only (`m`) — so a descriptor borrowed from the wrong relation differs from the right
/// one in each field separately.
///
/// `x` is a real table that a derived table's alias shadows, and `t` is aliased onto
/// `t3`, so a lookup that asks the **catalog** for a qualifier — rather than the FROM
/// clause's own names — lands on a table the statement does not read.
fn seeded() -> Database {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')",
        "CREATE TABLE t (v VARCHAR(5), n NUMERIC(6,2), k INT, m mood)",
        "CREATE TABLE t3 (m TEXT, k TEXT, n NUMERIC(10,5), v VARCHAR(8))",
        "CREATE TABLE x (v VARCHAR(3))",
        "INSERT INTO t VALUES ('ab', 1.25, 1, 'happy')",
        "INSERT INTO t3 VALUES ('sad', 'x', 2.5, 'abcdefgh')",
        "INSERT INTO x VALUES ('xyz')",
    ] {
        db.execute(sql).unwrap();
    }
    db
}

/// **A window query's projected column is the one its qualifier names.**
///
/// Measured on 18.6 over these tables: `SELECT b.k, b.v, b.n, b.m, row_number() OVER ()
/// FROM t a, t3 b` sends `text`, `varchar(8)` (`12`), `numeric(10,5)` (`655369`) and
/// `text`, each with `t3`'s oid and its own attnum, on both legs. The planner's window
/// arm used to take the **first** input column carrying each name — `t`'s — and declared
/// `int4`, `9`, `393222` and `mood` with `t`'s provenance on `Describe` alone.
///
/// The assertions are ordered so the first failure names the field: the type OID
/// (`b.k`, which a binary client would decode as the wrong type) comes first.
#[test]
fn a_window_querys_qualified_column_is_the_one_its_qualifier_names() {
    let mut db = seeded();
    let t3 = |db: &mut Database, c: &str| provenance(db, "t3", c);
    let expected: Vec<Identity> = vec![
        (TypeOid::Text, -1, t3(&mut db, "k"), None),
        (TypeOid::Varchar, 12, t3(&mut db, "v"), None),
        (TypeOid::Numeric, 655_369, t3(&mut db, "n"), None),
        (TypeOid::Text, -1, t3(&mut db, "m"), None),
    ];
    let (planned, executed) = both_producers(
        &mut db,
        "SELECT b.k, b.v, b.n, b.m, row_number() OVER () FROM t a, t3 b",
    );
    let planned: Vec<Identity> = planned.iter().take(4).map(identity).collect();
    let executed: Vec<Identity> = executed.iter().take(4).map(identity).collect();
    assert_eq!(planned[0].0, TypeOid::Text, "b.k is t3's text column");
    assert_eq!(planned, expected, "Describe");
    assert_eq!(executed, expected, "the simple-query leg");

    // The enum half the other way round: `b` is now `t`, whose `m` is the enum.
    let (planned, executed) =
        both_producers(&mut db, "SELECT b.m, row_number() OVER () FROM t3 a, t b");
    assert_eq!(planned[0].enum_type.as_deref(), Some("mood"), "Describe");
    assert_eq!(identity(&planned[0]), identity(&executed[0]));
}

/// **The qualifier is resolved against the FROM clause's own names, not the catalog.**
/// `t` here is an alias of `t3`, and `x` a derived table over `t` shadowing the real
/// table `x` — so a lookup that asks the catalog for the qualifier reads `t.v` (`9`) and
/// `x.v` (`7`) where the statement reads `t3.v` (`12`) and `t.v` (`9`, carried through
/// the derived table with `t`'s provenance, as 18.6 carries it).
#[test]
fn a_window_querys_qualifier_is_the_from_clauses_name() {
    let mut db = seeded();
    let t3_v = provenance(&mut db, "t3", "v");
    let t_v = provenance(&mut db, "t", "v");
    let (planned, executed) = both_producers(
        &mut db,
        "SELECT t.v, x.v, row_number() OVER () FROM t3 t, (SELECT v FROM t) x",
    );
    let expected: Vec<Identity> = vec![
        (TypeOid::Varchar, 12, t3_v, None),
        (TypeOid::Varchar, 9, t_v, None),
    ];
    let planned: Vec<Identity> = planned.iter().take(2).map(identity).collect();
    let executed: Vec<Identity> = executed.iter().take(2).map(identity).collect();
    assert_eq!(planned, expected, "Describe");
    assert_eq!(executed, expected, "the simple-query leg");
}

/// **An aliased column beside a window call passes its descriptor through, and a derived
/// table over the window query re-exports the right one.** 18.6 sends `varchar(8)` with
/// `t3`'s provenance for `b.v AS w`, and the same for the derived table's `x.v`; the
/// planner's window arm used to drop the aliased column's modifier and provenance, and to
/// hand the derived table `t`'s.
#[test]
fn a_window_querys_aliased_and_re_exported_columns_keep_their_identity() {
    let mut db = seeded();
    let t3_v = provenance(&mut db, "t3", "v");
    for sql in [
        "SELECT b.v AS w, row_number() OVER () FROM t a, t3 b",
        "SELECT x.v FROM (SELECT b.v, row_number() OVER () AS rn FROM t a, t3 b) x",
        "WITH c AS (SELECT b.v, row_number() OVER () AS rn FROM t a, t3 b) SELECT v FROM c",
    ] {
        let (planned, executed) = both_producers(&mut db, sql);
        let expected: Identity = (TypeOid::Varchar, 12, t3_v, None);
        assert_eq!(identity(&planned[0]), expected, "Describe: {sql}");
        assert_eq!(identity(&executed[0]), expected, "simple leg: {sql}");
    }
}

/// **A FROM-clause `unnest`'s qualified column is the `unnest`'s.** 18.6 types
/// `u.k` over `unnest(ARRAY['a']) u(k)` as `text` with no provenance. The planner's scope
/// registered no relation for the `unnest`, so the qualifier fell through to the flat
/// column list and took `t.k` — `int4`, with `t`'s provenance — on `Describe`, both for a
/// plain projection and through the `Aggregate` and `Window` arms, which read the same
/// scope.
#[test]
fn an_unnests_qualified_column_is_the_unnests() {
    let mut db = seeded();
    for sql in [
        "SELECT u.k FROM t, unnest(ARRAY['a']) AS u(k)",
        "SELECT u.k, count(*) FROM t, unnest(ARRAY['a']) AS u(k) GROUP BY u.k",
        "SELECT u.k, row_number() OVER () FROM t, unnest(ARRAY['a']) AS u(k)",
    ] {
        let (planned, executed) = both_producers(&mut db, sql);
        let expected: Identity = (TypeOid::Text, -1, (0, 0), None);
        assert_eq!(identity(&planned[0]), expected, "Describe: {sql}");
        assert_eq!(identity(&executed[0]), expected, "simple leg: {sql}");
    }
    // The array's element type is the `unnest`'s, and it is not the colliding column's.
    let (planned, executed) = both_producers(
        &mut db,
        "SELECT u.v FROM t, unnest(ARRAY['a'], ARRAY[2.5]) AS u(k, v)",
    );
    assert_eq!(identity(&planned[0]), (TypeOid::Numeric, -1, (0, 0), None));
    assert_eq!(identity(&executed[0]), identity(&planned[0]));
}
