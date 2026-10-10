//! A `SERIAL` column added by `ALTER TABLE … ADD COLUMN` is declared `NOT NULL`, as a
//! `CREATE TABLE`'s is and as 18.6's is.
//!
//! The differential fixtures (`ddl/add_column_serial_*`) hold the declaration and its
//! refusals against 18.6. These hold it on each of the embed's stores, over a table with
//! and without rows, across a block's rollback, a failed `ADD COLUMN` and a dump, which a
//! fixture's one compared statement does not reach.

#![allow(clippy::unwrap_used, clippy::panic)]

use pglet::Database;

/// `sql`'s outcome: its last command tag and rows, or `ERR <SQLSTATE>`.
fn answer(db: &mut Database, sql: &str) -> String {
    match db.execute(sql) {
        Ok(results) => results
            .last()
            .map_or_else(String::new, |r| format!("{} {:?}", r.command_tag, r.rows)),
        Err(e) => format!("ERR {}", e.sqlstate()),
    }
}

fn run(db: &mut Database, sql: &str) {
    db.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
}

fn stores() -> Vec<(&'static str, Database)> {
    vec![
        ("memory", Database::in_memory()),
        ("btree", Database::with_btree()),
        (
            "btree, one connection",
            Database::with_btree_for_one_connection(0),
        ),
    ]
}

/// Whether the catalog declares `table.column` nullable.
fn nullable(db: &Database, table: &str, column: &str) -> bool {
    db.schema()
        .get_table(table)
        .unwrap()
        .columns
        .iter()
        .find(|c| c.name == column)
        .unwrap_or_else(|| panic!("no column {table}.{column}"))
        .nullable
}

#[test]
fn each_serial_spelling_is_declared_not_null_by_add_column() {
    for (store, mut db) in stores() {
        for rows in [false, true] {
            run(&mut db, "CREATE TABLE t (v TEXT)");
            if rows {
                run(&mut db, "INSERT INTO t VALUES ('a'), ('b')");
            }
            for (column, spelling) in [
                ("a", "SERIAL"),
                ("b", "BIGSERIAL"),
                ("c", "SMALLSERIAL"),
                ("d", "SERIAL4"),
                ("e", "SERIAL8"),
                ("f", "SERIAL2"),
            ] {
                let label = format!("{store}, rows: {rows}, {spelling}");
                run(
                    &mut db,
                    &format!("ALTER TABLE t ADD COLUMN {column} {spelling}"),
                );
                assert!(!nullable(&db, "t", column), "{label}");
                assert_eq!(
                    answer(
                        &mut db,
                        &format!(
                            "SELECT is_nullable FROM information_schema.columns \
                             WHERE table_name = 't' AND column_name = '{column}'"
                        )
                    ),
                    "SELECT 1 [[Text(\"NO\")]]",
                    "{label}"
                );
                // The rows the table held each drew a value; none is NULL.
                assert_eq!(
                    answer(
                        &mut db,
                        &format!("SELECT count(*) FROM t WHERE {column} IS NULL")
                    ),
                    "SELECT 1 [[Int8(0)]]",
                    "{label}"
                );
                assert_eq!(
                    answer(
                        &mut db,
                        &format!("INSERT INTO t ({column}, v) VALUES (NULL, 'n')")
                    ),
                    "ERR 23502",
                    "{label}"
                );
                assert_eq!(
                    answer(&mut db, &format!("UPDATE t SET {column} = NULL")),
                    if rows { "ERR 23502" } else { "UPDATE 0 []" },
                    "{label}"
                );
            }
            // `IF NOT EXISTS`: declared where the column is added, untouched where it is not.
            run(&mut db, "ALTER TABLE t ADD COLUMN IF NOT EXISTS g SERIAL");
            assert!(!nullable(&db, "t", "g"), "{store}");
            run(&mut db, "ALTER TABLE t ADD COLUMN IF NOT EXISTS v SERIAL");
            assert!(
                nullable(&db, "t", "v"),
                "{store}: an existing column is not redeclared"
            );
            run(&mut db, "INSERT INTO t (v) VALUES ('next')");
            assert_eq!(
                answer(
                    &mut db,
                    "SELECT count(*) FROM t WHERE a IS NULL OR b IS NULL OR c IS NULL \
                     OR d IS NULL OR e IS NULL OR f IS NULL OR g IS NULL"
                ),
                "SELECT 1 [[Int8(0)]]",
                "{store}: every row drew every serial column"
            );
            run(&mut db, "DROP TABLE t");
        }
    }
}

#[test]
fn a_rolled_back_add_column_takes_the_declaration_with_it() {
    for (store, mut db) in stores() {
        run(&mut db, "CREATE TABLE t (v TEXT)");
        run(&mut db, "INSERT INTO t VALUES ('a'), ('b')");
        run(&mut db, "BEGIN");
        run(&mut db, "ALTER TABLE t ADD COLUMN id SERIAL");
        assert_eq!(
            answer(&mut db, "INSERT INTO t (id, v) VALUES (NULL, 'n')"),
            "ERR 23502",
            "{store}: declared inside the block"
        );
        run(&mut db, "ROLLBACK");
        assert_eq!(
            db.schema().get_table("t").unwrap().columns.len(),
            1,
            "{store}"
        );
        assert!(db.schema().sequences.is_empty(), "{store}");
        run(&mut db, "BEGIN");
        run(&mut db, "ALTER TABLE t ADD COLUMN id SERIAL");
        run(&mut db, "COMMIT");
        assert!(!nullable(&db, "t", "id"), "{store}");
        assert_eq!(
            answer(&mut db, "SELECT id, v FROM t ORDER BY id"),
            "SELECT 2 [[Int4(1), Text(\"a\")], [Int4(2), Text(\"b\")]]",
            "{store}"
        );
    }
}

#[test]
fn a_refused_add_column_declares_nothing() {
    // The declaration is the added column's, and a refused statement adds none: no
    // column, and no sequence under the name it would have minted.
    for (store, mut db) in stores() {
        run(&mut db, "CREATE TABLE t (v TEXT)");
        run(&mut db, "INSERT INTO t VALUES ('a'), ('b')");
        for (sql, want) in [
            (
                "ALTER TABLE t ADD COLUMN id SERIAL CHECK (id > 1)",
                "ERR 23514",
            ),
            (
                "ALTER TABLE t ADD COLUMN id SERIAL REFERENCES nosuch (x)",
                "ERR 42P01",
            ),
        ] {
            assert_eq!(answer(&mut db, sql), want, "{store}: {sql}");
            assert_eq!(
                db.schema().get_table("t").unwrap().columns.len(),
                1,
                "{store}: {sql}"
            );
            assert!(db.schema().sequences.is_empty(), "{store}: {sql}");
        }
        run(&mut db, "ALTER TABLE t ADD COLUMN id SERIAL");
        assert_eq!(
            answer(&mut db, "SELECT id FROM t ORDER BY id"),
            "SELECT 2 [[Int4(1)], [Int4(2)]]",
            "{store}: the sequence starts where a new one starts"
        );
    }
}

#[test]
fn an_added_serial_column_dumps_declared_and_its_schema_reloads() {
    for (store, mut db) in stores() {
        for sql in [
            "CREATE TABLE t (v TEXT)",
            "INSERT INTO t VALUES ('a'), ('b')",
            "ALTER TABLE t ADD COLUMN id SERIAL",
            "ALTER TABLE t ADD COLUMN big BIGSERIAL",
            "INSERT INTO t (v) VALUES ('c')",
        ] {
            run(&mut db, sql);
        }
        let dump = db.dump_sql().unwrap();
        assert!(dump.contains("id integer NOT NULL"), "{store}:\n{dump}");
        assert!(dump.contains("big bigint NOT NULL"), "{store}:\n{dump}");
        // The statements ahead of the data — the sequences, the table, the ownership and
        // the defaults — replayed into a fresh database declare the columns again.
        let schema_part = dump.split("\nCOPY ").next().unwrap();
        let mut reloaded = Database::with_btree();
        for statement in schema_part.split(";\n") {
            if !statement.trim().is_empty() {
                run(&mut reloaded, statement.trim());
            }
        }
        assert!(
            !nullable(&reloaded, "t", "id") && !nullable(&reloaded, "t", "big"),
            "{store}"
        );
        assert_eq!(
            answer(
                &mut reloaded,
                "INSERT INTO t (v) VALUES ('first') RETURNING id, big"
            ),
            "INSERT 0 1 [[Int4(1), Int8(1)]]",
            "{store}: the reloaded defaults draw"
        );
        assert_eq!(
            answer(&mut reloaded, "INSERT INTO t (id, v) VALUES (NULL, 'n')"),
            "ERR 23502",
            "{store}"
        );
    }
}
