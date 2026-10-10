//! `FLOAT(n)`'s modifier **selects** the type; it is not one the type carries.
//!
//! The differential fixtures (`tests/fixtures/**/float_precision_*`) hold the values,
//! the refusals and the catalog reads, comparing pglet against 18.6 through the wire.
//! Three things sit outside them:
//!
//! - **The window's edges as a list.** A fixture asserts one statement; the
//!   off-by-ones this rule invites — 24 selecting `float8`, 53 refusing, 54 selecting —
//!   are a *range*, and each of them is a silently wrong stored value rather than an
//!   error. Cheaper to keep the four edges in one place, and to run the refusal across
//!   the nine positions in one loop.
//! - **The dump.** The selection has to survive an export: a `FLOAT(1)` column must
//!   dump as `real` and reload as a `real` column, or a database holding one exports to
//!   something that reloads a different table.
//! - **The persisted catalog.** The selected type must survive a **double-meta** store
//!   round-trip, not only a live `Schema`.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface a setup failure loudly right
// here. `clippy::panic` has no test-allowance in this clippy (only unwrap/expect do).
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::common;

use common::open_database;
use pglet::Database;
use pglet_testing::block_io::MemBlockIo;

/// A literal `float8` holds whole and `float4` rounds to, so it separates the two
/// selections by **value** and not only by declared type.
const WIDE: &str = "0.1234567890123456";
/// What `WIDE` becomes once it has been through a `real`.
const NARROWED: &str = "0.12345679";

/// The SQLSTATE `sql` fails with, or `None` when it succeeds.
fn state(db: &mut Database, sql: &str) -> Option<&'static str> {
    db.execute(sql).err().map(|e| e.sqlstate())
}

/// The single text value `sql` answers.
fn text(db: &mut Database, sql: &str) -> String {
    let rows = db.execute(sql).expect("the query answers").remove(0).rows;
    let v = rows
        .first()
        .and_then(|r| r.first())
        .unwrap_or_else(|| panic!("expected one value from `{sql}`"));
    pglet_sql::types::value_to_text(v).unwrap_or_else(|| "NULL".to_string())
}

/// Every `n` in 1–24 selects `float4` and every `n` in 25–53 selects `float8` — the
/// edges being where an off-by-one lives — read through the three expression positions
/// (`::`, `CAST`, the typed constant) plus a `[]` suffix. The **column** positions are
/// the fixtures' (`types/float_precision_column_writes.sql`) and the reload test below's.
#[test]
fn the_window_edges_select_the_same_type_at_every_position() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE probe (id int)").expect("seed");
    db.execute("INSERT INTO probe VALUES (1)").expect("seed");
    for n in [1, 2, 23, 24] {
        assert_eq!(
            text(&mut db, &format!("SELECT {WIDE}::float({n})")),
            NARROWED
        );
        assert_eq!(
            text(&mut db, &format!("SELECT CAST({WIDE} AS float({n}))")),
            NARROWED
        );
        assert_eq!(
            text(&mut db, &format!("SELECT FLOAT({n}) '{WIDE}'")),
            NARROWED
        );
        assert_eq!(
            text(&mut db, &format!("SELECT '{{{WIDE}}}'::float({n})[]")),
            format!("{{{NARROWED}}}")
        );
    }
    for n in [25, 26, 52, 53] {
        assert_eq!(text(&mut db, &format!("SELECT {WIDE}::float({n})")), WIDE);
        assert_eq!(
            text(&mut db, &format!("SELECT CAST({WIDE} AS float({n}))")),
            WIDE
        );
        assert_eq!(text(&mut db, &format!("SELECT FLOAT({n}) '{WIDE}'")), WIDE);
    }
    // The `/*EMPTY*/` production: a bare `FLOAT` is `float8`, as `FLOAT(53)` is.
    assert_eq!(text(&mut db, &format!("SELECT {WIDE}::float")), WIDE);
    assert_eq!(text(&mut db, &format!("SELECT FLOAT '{WIDE}'")), WIDE);
}

/// Outside the window the answer is `22023` — at each position, and at each end.
///
/// The positions that *resolve* no type name are the ones the placement buys:
/// `CREATE SEQUENCE … AS` and an identity column's `(AS …)` read a name and throw it
/// away, so they agree only because 18.6 refuses in the grammar and so does pglet.
#[test]
fn a_precision_outside_the_window_is_22023_at_every_position() {
    let mut db = Database::with_btree();
    for n in ["0", "54", "55", "1000", "2147483647"] {
        for sql in [
            format!("SELECT 1::float({n})"),
            format!("SELECT CAST(1 AS float({n}))"),
            format!("SELECT FLOAT({n}) '0.5'"),
            format!("SELECT float({n})"),
            format!("CREATE TABLE zq (a float({n}))"),
            format!("CREATE TABLE zq (a float({n})[])"),
            format!("ALTER TABLE zt ADD COLUMN a float({n})"),
            format!("CREATE SEQUENCE zs AS FLOAT({n})"),
            format!("CREATE TABLE zi (id int GENERATED ALWAYS AS IDENTITY (AS FLOAT({n})), b int)"),
        ] {
            assert_eq!(state(&mut db, &sql), Some("22023"), "{sql}");
        }
    }
    // The refusal is the *precision*'s and not the position's: in range, each of those
    // positions gets past it. `ALTER TABLE` needs its table, so seed one here rather
    // than above, where a refused `CREATE TABLE` must find nothing.
    db.execute("CREATE TABLE zt (id int)").expect("seed");
    for sql in [
        "SELECT 1::float(24)",
        "SELECT CAST(1 AS float(24))",
        "SELECT FLOAT(24) '0.5'",
        "CREATE TABLE zq (a float(24))",
        "ALTER TABLE zt ADD COLUMN a float(24)",
    ] {
        assert_eq!(state(&mut db, sql), None, "{sql}");
    }
}

/// A `FLOAT(n)` column dumps as the **selected** type and reloads as that column.
///
/// The reload is the load-bearing half: `real` is valid SQL whatever the column is, so
/// a text assertion alone would not say that the reloaded column narrows its writes.
#[test]
fn a_float_precision_column_dumps_as_the_selected_type_and_reloads() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE d (a FLOAT(1), b FLOAT(25), c FLOAT, e FLOAT(1)[])",
        &format!("INSERT INTO d VALUES ({WIDE}, {WIDE}, {WIDE}, '{{{WIDE}}}')"),
    ] {
        db.execute(sql).expect("seed");
    }
    let dump = db.dump_sql().expect("dump");
    for spelling in [
        "a real",
        "b double precision",
        "c double precision",
        "e real[]",
    ] {
        assert!(
            dump.contains(spelling),
            "dump must spell `{spelling}`:\n{dump}"
        );
    }
    assert!(
        !dump.contains("float("),
        "no `float(n)` spelling survives the selection:\n{dump}"
    );

    // Reload the DDL into a fresh engine and re-derive the shape from what it stores.
    // The `COPY … FROM stdin` blocks carry the rows over a wire this test has no client
    // for and are dropped; the column shapes are what is under test.
    let mut in_copy = false;
    let ddl: String = dump
        .lines()
        .filter(|line| {
            if in_copy {
                in_copy = *line != "\\.";
                return false;
            }
            in_copy = line.starts_with("COPY ") && line.ends_with("FROM stdin;");
            !in_copy
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut reloaded = Database::with_btree();
    for stmt in ddl.split(";\n") {
        let stmt = stmt.trim().trim_end_matches(';');
        if stmt.is_empty() {
            continue;
        }
        reloaded
            .execute(stmt)
            .unwrap_or_else(|e| panic!("reload `{stmt}`: {e}"));
    }
    reloaded
        .execute(&format!(
            "INSERT INTO d VALUES ({WIDE}, {WIDE}, {WIDE}, '{{{WIDE}}}')"
        ))
        .expect("the reloaded columns take the row");
    assert_eq!(text(&mut reloaded, "SELECT a FROM d"), NARROWED);
    assert_eq!(text(&mut reloaded, "SELECT b FROM d"), WIDE);
    assert_eq!(text(&mut reloaded, "SELECT c FROM d"), WIDE);
    assert_eq!(
        text(&mut reloaded, "SELECT e FROM d"),
        format!("{{{NARROWED}}}")
    );
}

/// The selected type survives a **double-meta store** round-trip: it is the column's
/// own `TypeOid`, and a column whose width lived only in the live `Schema` would store
/// a different value after a reopen.
#[test]
fn a_float_precision_columns_selected_type_survives_a_reopen() {
    let io = MemBlockIo::new();
    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE p (a FLOAT(1), b FLOAT(25))")
            .expect("create");
        db.commit(0).expect("commit");
    }
    let mut db = open_database(Box::new(io));
    db.execute(&format!("INSERT INTO p VALUES ({WIDE}, {WIDE})"))
        .expect("insert after reopen");
    assert_eq!(text(&mut db, "SELECT a FROM p"), NARROWED);
    assert_eq!(text(&mut db, "SELECT b FROM p"), WIDE);
}
