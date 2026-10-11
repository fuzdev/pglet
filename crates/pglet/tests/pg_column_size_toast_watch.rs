//! `pg_column_size` answers a stored varlena only while the store has watched every row
//! write to its table stay within 18.6's TOAST threshold (`Storage::rows_untoasted`).
//!
//! Each path that writes a row — `INSERT`, `UPDATE`, `ON CONFLICT DO UPDATE`, `COPY`, a
//! foreign key's action, a volatile `ADD COLUMN` default — ends that for its table
//! when the row it writes is past the threshold, and the column DDL that leaves 18.6's
//! rows unlike pglet's (`DROP COLUMN`, a non-NULL `ADD COLUMN` default over existing
//! rows) ends it whatever the size. A fork carries what its source knew, a table dropped
//! and created again under its name starts over, and a fixed-length column answers
//! regardless. The differential fixtures (`functions/pg_column_size_*`) hold the sizes
//! themselves against 18.6; this holds the watch, which a fixture cannot reach on every
//! path.
#![allow(clippy::panic, clippy::expect_used)]

use pglet::Database;
use pglet_sql::types::Value;

/// One way of writing a row.
type Write<'a> = Box<dyn Fn(&mut Database) + 'a>;

/// A `bytea` literal of 3 000 bytes: a row holding it is past the 2 032-byte threshold.
fn big() -> String {
    format!("'\\x{}'", "78".repeat(3000))
}

fn run(db: &mut Database, sql: &str) {
    db.execute(sql)
        .unwrap_or_else(|e| panic!("`{}`: {e}", &sql[..sql.len().min(80)]));
}

/// `pg_column_size(s)` of `t`'s row `id = 1`, or the SQLSTATE it is refused with.
fn size(db: &mut Database, column: &str) -> Result<Value, &'static str> {
    db.execute(&format!(
        "SELECT pg_column_size({column}) FROM t WHERE id = 1"
    ))
    .map(|mut results| results.remove(0).rows.remove(0).remove(0))
    .map_err(|e| e.sqlstate())
}

/// A fresh table `t (id int, s text, b bytea)` holding row 1 with a small `s`, answered.
fn fresh() -> Database {
    let mut db = Database::with_btree();
    run(
        &mut db,
        "CREATE TABLE t (id int PRIMARY KEY, s text, b bytea)",
    );
    run(&mut db, "INSERT INTO t VALUES (1, 'hello', NULL)");
    assert_eq!(
        size(&mut db, "s"),
        Ok(Value::Int4(6)),
        "a fresh table answers"
    );
    db
}

#[test]
fn each_row_write_past_the_threshold_ends_the_answer() {
    let big = big();
    let paths: [(&str, Write); 5] = [
        (
            "INSERT",
            Box::new(|db| run(db, &format!("INSERT INTO t VALUES (2, 'x', {big})"))),
        ),
        (
            "UPDATE",
            Box::new(|db| run(db, &format!("UPDATE t SET b = {big} WHERE id = 1"))),
        ),
        (
            "ON CONFLICT DO UPDATE",
            Box::new(|db| {
                run(
                    db,
                    &format!(
                        "INSERT INTO t VALUES (1, 'x', NULL) ON CONFLICT (id) DO UPDATE SET b = {big}"
                    ),
                );
            }),
        ),
        (
            "COPY",
            Box::new(|db| {
                let line = format!("2\tx\t\\\\x{}\n", "78".repeat(3000));
                db.copy_in("COPY t FROM STDIN", line.as_bytes())
                    .expect("copy");
            }),
        ),
        (
            "ADD COLUMN with a volatile default",
            Box::new(|db| {
                // a 1 990-byte `b` leaves row 1 at 2 030 bytes, within the threshold —
                // 18.6's own `t_len` for it, measured — so it still answers
                run(
                    db,
                    &format!("UPDATE t SET b = '\\x{}' WHERE id = 1", "78".repeat(1990)),
                );
                assert_eq!(
                    size(db, "s"),
                    Ok(Value::Int4(6)),
                    "2 030 bytes is within it"
                );
                // the volatile default rewrites every row with its own drawn `float8`,
                // and that rewrite is what takes row 1 to 2 040
                run(db, "ALTER TABLE t ADD COLUMN r float8 DEFAULT random()");
            }),
        ),
    ];
    for (path, write) in paths {
        let mut db = fresh();
        write(&mut db);
        assert_eq!(
            size(&mut db, "s"),
            Err("0A000"),
            "{path} past the threshold"
        );
        assert_eq!(
            size(&mut db, "id"),
            Ok(Value::Int4(4)),
            "{path}: a fixed-length column answers"
        );
    }
}

#[test]
fn a_write_within_the_threshold_keeps_the_answer() {
    let mut db = fresh();
    run(&mut db, "INSERT INTO t VALUES (2, 'x', '\\xdeadbeef')");
    run(&mut db, "UPDATE t SET b = '\\x00' WHERE id = 1");
    db.copy_in("COPY t FROM STDIN", b"3\ty\t\\\\x00\n")
        .expect("copy");
    assert_eq!(size(&mut db, "s"), Ok(Value::Int4(6)));
    assert_eq!(size(&mut db, "b"), Ok(Value::Int4(2)));
}

/// `pg_column_size(<column>)` of `table`'s only row, or the SQLSTATE it is refused with.
fn size_in(db: &mut Database, table: &str, column: &str) -> Result<Value, &'static str> {
    db.execute(&format!("SELECT pg_column_size({column}) FROM {table}"))
        .map(|mut results| results.remove(0).rows.remove(0).remove(0))
        .map_err(|e| e.sqlstate())
}

/// A foreign key's action writes the child row, and a write that takes it past the
/// threshold ends the answer — each child here sits at exactly 2 032 bytes until its
/// action runs, and 18.6 then compresses its `b` (measured: `pg_column_compression` is
/// `pglz` after each).
#[test]
fn a_foreign_key_action_is_a_row_write() {
    // `ON UPDATE CASCADE` lengthens the child's key from 1 character to 10
    let mut db = Database::with_btree();
    run(&mut db, "CREATE TABLE fp (k text PRIMARY KEY)");
    run(
        &mut db,
        "CREATE TABLE fc (id int, s text, b bytea, pk text REFERENCES fp ON UPDATE CASCADE)",
    );
    run(&mut db, "INSERT INTO fp VALUES ('a')");
    run(
        &mut db,
        &format!(
            "INSERT INTO fc VALUES (1, 'hello', '\\x{}', 'a')",
            "78".repeat(1990)
        ),
    );
    assert_eq!(
        size_in(&mut db, "fc", "s"),
        Ok(Value::Int4(6)),
        "2 032 bytes is within it"
    );
    run(&mut db, "UPDATE fp SET k = 'abcdefghij'");
    assert_eq!(
        size_in(&mut db, "fc", "s"),
        Err("0A000"),
        "ON UPDATE CASCADE"
    );

    // `ON DELETE SET NULL` gives a ten-column row with no NULL a null bitmap, which takes
    // its header from 24 bytes to 32
    let mut db = Database::with_btree();
    run(&mut db, "CREATE TABLE sp (id int PRIMARY KEY)");
    run(
        &mut db,
        "CREATE TABLE sc (c1 int, c2 int, c3 int, c4 int, c5 int, c6 int, c7 int, c8 int, \
         b bytea, pid int REFERENCES sp ON DELETE SET NULL)",
    );
    run(&mut db, "INSERT INTO sp VALUES (1)");
    run(
        &mut db,
        &format!(
            "INSERT INTO sc VALUES (1, 2, 3, 4, 5, 6, 7, 8, '\\x{}', 1)",
            "78".repeat(1966)
        ),
    );
    assert_eq!(size_in(&mut db, "sc", "c1"), Ok(Value::Int4(4)));
    assert!(
        size_in(&mut db, "sc", "b").is_ok(),
        "2 032 bytes is within it"
    );
    run(&mut db, "DELETE FROM sp WHERE id = 1");
    assert_eq!(
        size_in(&mut db, "sc", "b"),
        Err("0A000"),
        "ON DELETE SET NULL"
    );
}

#[test]
fn column_ddl_that_leaves_rows_unlike_pglets_ends_the_answer() {
    // a dropped column's bytes stay in 18.6's rows
    let mut db = fresh();
    run(&mut db, "ALTER TABLE t DROP COLUMN b");
    assert_eq!(size(&mut db, "s"), Err("0A000"), "DROP COLUMN");

    // a non-NULL default over existing rows is read from `attmissingval`
    let mut db = fresh();
    run(&mut db, "ALTER TABLE t ADD COLUMN c text DEFAULT 'abc'");
    assert_eq!(size(&mut db, "c"), Err("0A000"), "ADD COLUMN DEFAULT");

    // a NULL default changes no row's bytes
    let mut db = fresh();
    run(&mut db, "ALTER TABLE t ADD COLUMN c text");
    assert_eq!(
        size(&mut db, "s"),
        Ok(Value::Int4(6)),
        "ADD COLUMN without one"
    );

    // the same inside a block, which the store routes through its transaction's own
    // column DDL
    for (ddl, column) in [
        ("ALTER TABLE t DROP COLUMN b", "s"),
        ("ALTER TABLE t ADD COLUMN c text DEFAULT 'abc'", "c"),
    ] {
        let mut db = fresh();
        run(&mut db, "BEGIN");
        run(&mut db, ddl);
        assert_eq!(size(&mut db, column), Err("0A000"), "{ddl} in a block");
        run(&mut db, "ROLLBACK");
        // a rollback restores the rows, and what they were is no longer known
        assert_eq!(size(&mut db, "s"), Err("0A000"), "{ddl}, rolled back");
    }
}

#[test]
fn a_fork_carries_the_watch_and_a_recreated_table_starts_over() {
    let mut base = fresh();
    let mut fork = base.fork().expect("fork");
    assert_eq!(
        size(&mut fork, "s"),
        Ok(Value::Int4(6)),
        "the fork knows it"
    );
    run(
        &mut fork,
        &format!("INSERT INTO t VALUES (2, 'x', {})", big()),
    );
    assert_eq!(
        size(&mut fork, "s"),
        Err("0A000"),
        "the fork's write ends its own"
    );
    assert_eq!(
        size(&mut base, "s"),
        Ok(Value::Int4(6)),
        "and not its source's"
    );

    run(&mut fork, "DROP TABLE t");
    run(
        &mut fork,
        "CREATE TABLE t (id int PRIMARY KEY, s text, b bytea)",
    );
    run(&mut fork, "INSERT INTO t VALUES (1, 'hello', NULL)");
    assert_eq!(
        size(&mut fork, "s"),
        Ok(Value::Int4(6)),
        "a new table is watched anew"
    );
}
