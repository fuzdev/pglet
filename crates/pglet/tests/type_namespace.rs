//! The type namespace pglet models — each enum type and each table's row type — and the
//! halves of it no differential fixture can reach: the hint 18.6 sends on the relation
//! route (an omitted field scores as agreement in the harness), and the catalog a
//! refused statement leaves on the autocommit path (every fixture runs inside
//! `BEGIN … ROLLBACK`, which restores it).

// A test harness: `.unwrap()`/`panic!` surface a setup failure or an unexpected cell
// shape loudly right here.
#![allow(clippy::unwrap_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::Value;

const RELATION_TYPE_HINT: &str = "A relation has an associated type of the same name, so you \
                                  must use a name that doesn't conflict with any existing type.";

/// A relation created over an enum's name is `42710` **with** the relation-route hint; an
/// enum created over a table's name is `42710` **without** one — 18.6's two routines.
#[test]
fn the_two_directions_carry_their_own_hint() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE e AS ENUM ('x')").unwrap();
    for sql in [
        "CREATE TABLE e (a INT)",
        "CREATE SEQUENCE e",
        "CREATE TABLE IF NOT EXISTS e (a INT)",
    ] {
        let err = db.execute(sql).unwrap_err();
        assert_eq!(err.sqlstate(), "42710", "{sql}");
        assert_eq!(err.to_string(), "type \"e\" already exists", "{sql}");
        assert_eq!(err.hint().as_deref(), Some(RELATION_TYPE_HINT), "{sql}");
    }

    db.execute("CREATE TABLE t (a INT)").unwrap();
    let err = db.execute("CREATE TYPE t AS ENUM ('x')").unwrap_err();
    assert_eq!(err.sqlstate(), "42710");
    assert_eq!(err.hint(), None);
}

/// A `CREATE TABLE` refused at its row type has already claimed its `SERIAL`'s sequence
/// and its `UNIQUE` index's name; neither may outlive the refusal on the autocommit path.
#[test]
fn a_table_refused_over_an_enum_leaves_no_phantom() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE u AS ENUM ('x')").unwrap();
    let err = db
        .execute("CREATE TABLE u (a SERIAL, b INT UNIQUE)")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "42710");
    assert!(db.schema().get_table("u").is_none());
    assert!(!db.schema().sequences.contains_key("u_a_seq"));
    assert!(!db.schema().indexes.contains_key("u_b_key"));
    // The names are free.
    db.execute("CREATE SEQUENCE u_a_seq").unwrap();
    db.execute("CREATE TABLE ub (b INT CONSTRAINT u_b_key UNIQUE)")
        .unwrap();
}

/// An `ADD COLUMN … SERIAL` refused at its sequence's type collision leaves the table
/// as it was: no column, no sequence, every row readable.
#[test]
fn an_added_serial_refused_over_an_enum_leaves_the_table_whole() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE t_b_seq AS ENUM ('x')").unwrap();
    db.execute("CREATE TABLE t (a INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1), (2)").unwrap();
    let err = db.execute("ALTER TABLE t ADD COLUMN b SERIAL").unwrap_err();
    assert_eq!(err.sqlstate(), "42710");
    assert_eq!(err.hint().as_deref(), Some(RELATION_TYPE_HINT));
    assert!(!db.schema().sequences.contains_key("t_b_seq"));
    let table = db.schema().get_table("t").unwrap();
    assert_eq!(table.columns.len(), 1);
    let rows = db.execute("SELECT * FROM t").unwrap().remove(0).rows;
    assert_eq!(rows.len(), 2);
}

/// `DROP TYPE` of a table's row type names the table in its hint, as 18.6's does.
#[test]
fn drop_type_of_a_table_hints_the_table() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE z (a INT)").unwrap();
    for sql in ["DROP TYPE z", "DROP TYPE IF EXISTS z CASCADE"] {
        let err = db.execute(sql).unwrap_err();
        assert_eq!(err.sqlstate(), "2BP01", "{sql}");
        assert_eq!(
            err.hint().as_deref(),
            Some("You can drop table z instead."),
            "{sql}"
        );
    }
    assert!(db.schema().get_table("z").is_some());
}

/// An enum is stored under its ASCII-folded key, so a quoted `"Ab"` over a table `ab` is
/// refused — an over-rejection, 18.6 holding `"Ab"` and `ab` apart — because accepting it
/// would put an enum and a table under the one name `ab` in the catalog, which the dump
/// emits as a `CREATE TYPE ab` and a `CREATE TABLE ab` no engine restores. The spelling
/// 18.6 itself refuses, a quoted name over a table of that exact name, is the fixture
/// corpus's.
#[test]
fn a_quoted_enum_name_is_refused_over_the_table_its_folded_key_names() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ab (a INT)").unwrap();
    let err = db.execute("CREATE TYPE \"Ab\" AS ENUM ('x')").unwrap_err();
    assert_eq!(err.sqlstate(), "42710");
    assert!(!db.schema().enums.contains_key("ab"));
}

/// `DROP TYPE "Rd"` beside a table `"Rd"` and an enum `rd` names the table's row type:
/// `2BP01`, the message and the hint quoting the mixed-case name as 18.6's do, and the
/// enum the folded key finds survives. The unquoted spelling names the enum and drops it.
#[test]
fn drop_type_finds_a_quoted_row_type_before_the_folded_enum() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE rd AS ENUM ('a')").unwrap();
    db.execute("CREATE TABLE \"Rd\" (a INT)").unwrap();
    for sql in ["DROP TYPE \"Rd\"", "DROP TYPE IF EXISTS \"Rd\" CASCADE"] {
        let err = db.execute(sql).unwrap_err();
        assert_eq!(err.sqlstate(), "2BP01", "{sql}");
        assert_eq!(
            err.to_string(),
            "cannot drop type \"Rd\" because table \"Rd\" requires it",
            "{sql}"
        );
        assert_eq!(
            err.hint().as_deref(),
            Some("You can drop table \"Rd\" instead."),
            "{sql}"
        );
        assert!(db.schema().enums.contains_key("rd"), "{sql}");
    }
    db.execute("DROP TYPE rd").unwrap();
    assert!(!db.schema().enums.contains_key("rd"));
    assert!(db.schema().get_table("Rd").is_some());
}

/// Run a dump's statements into a fresh database, skipping the `COPY` blocks (their
/// rows need the copy-in path); every other block must be accepted.
fn restore_schema(dump: &str) -> Database {
    let mut restored = Database::with_btree();
    for block in dump.split("\n\n") {
        let sql = block.trim();
        if sql.is_empty() || sql.starts_with("COPY") {
            continue;
        }
        restored
            .execute(sql)
            .unwrap_or_else(|e| panic!("restore failed on `{sql}`: {e}\n{dump}"));
    }
    restored
}

/// The integer a single-row, single-column `sql` returns.
fn draw(db: &mut Database, sql: &str) -> i64 {
    let result = db.execute(sql).unwrap().remove(0);
    match result.rows[0][0] {
        Value::Int8(n) => n,
        Value::Int4(n) => i64::from(n),
        ref other => panic!("expected an integer, got {other:?}"),
    }
}

/// A sequence and an enum of one name are built by both engines — the sequence first,
/// since `CREATE TYPE` over a sequence's name is accepted and `CREATE SEQUENCE` over an
/// enum's is `42710` — so the dump creates each sequence ahead of the types, and the
/// script reloads: a plain sequence a column draws from, and a `SERIAL`'s sequence owned
/// by a column of a table whose other column has the enum type of the sequence's name.
/// The `psql` legs into 18.6 and into a server are `test_dump`'s.
#[test]
fn a_sequence_and_an_enum_of_one_name_dump_to_a_script_that_reloads() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE SEQUENCE sq9",
        "SELECT nextval('sq9'), nextval('sq9')",
        "CREATE TYPE sq9 AS ENUM ('a', 'b')",
        "CREATE TABLE uses9 (id INT DEFAULT nextval('sq9'), m sq9)",
        "INSERT INTO uses9 (m) VALUES ('b'), ('a')",
        "CREATE TABLE sr (id SERIAL PRIMARY KEY, v INT)",
        "CREATE TYPE sr_id_seq AS ENUM ('x', 'y')",
        "ALTER TABLE sr ADD COLUMN m sr_id_seq",
        "INSERT INTO sr (v, m) VALUES (10, 'y'), (20, 'x')",
    ] {
        db.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let dump = db.dump_sql().unwrap();
    for name in ["sq9", "sr_id_seq"] {
        let sequence = dump
            .find(&format!("CREATE SEQUENCE {name};"))
            .unwrap_or_else(|| panic!("no CREATE SEQUENCE {name}:\n{dump}"));
        let enum_type = dump
            .find(&format!("CREATE TYPE {name} AS ENUM"))
            .unwrap_or_else(|| panic!("no CREATE TYPE {name}:\n{dump}"));
        assert!(
            sequence < enum_type,
            "{name}: the sequence must come first\n{dump}"
        );
    }
    let mut restored = restore_schema(&dump);
    assert_eq!(
        draw(
            &mut restored,
            "INSERT INTO uses9 (m) VALUES ('a') RETURNING id"
        ),
        5
    );
    assert_eq!(
        draw(
            &mut restored,
            "INSERT INTO sr (v, m) VALUES (30, 'x') RETURNING id"
        ),
        3
    );
}

/// An identity column's sequence is created by its `ADD GENERATED`, after its table, and
/// the types precede every table — so an enum of the sequence's name, which both engines
/// build over it, refuses that statement on a reload. The export refuses the database
/// loudly rather than emit the script, which a `psql` without `ON_ERROR_STOP` would run
/// to exit `0` with the identity lost.
#[test]
fn an_identity_sequence_named_like_an_enum_refuses_the_dump() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE f1i (id INT GENERATED ALWAYS AS IDENTITY, v INT)",
        "CREATE TYPE f1i_id_seq AS ENUM ('a', 'b')",
        "INSERT INTO f1i (v) VALUES (1)",
    ] {
        db.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let err = db.dump_sql().unwrap_err();
    assert_eq!(err.sqlstate(), "0A000");
    assert!(err.to_string().contains("f1i_id_seq"), "{err}");

    // A plain sequence of the enum's name is not refused: only the identity route.
    let mut plain = Database::with_btree();
    for sql in [
        "CREATE TABLE f1p (id INT GENERATED ALWAYS AS IDENTITY, v INT)",
        "CREATE SEQUENCE f1p_other",
        "CREATE TYPE f1p_other AS ENUM ('a')",
    ] {
        plain.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    restore_schema(&plain.dump_sql().unwrap());
}
