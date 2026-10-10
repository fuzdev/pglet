//! `ALTER COLUMN … ADD GENERATED … AS IDENTITY` reads the column's declaration and its
//! default, and a column declared `NOT NULL` holds no NULL.
//!
//! An identity column is `NOT NULL`. The clause reads no row, so 18.6 refuses it over a
//! column not already declared `NOT NULL` (`55000`), whatever the rows hold; accepting it
//! there would declare `NOT NULL` over a NULL, and an `ORDER BY` served from an index over
//! the column would then leave that row out. And an identity column's value is its
//! sequence's, so 18.6 refuses the clause over a column that has a default (`55000`);
//! writing the identity over one would drop it and, over a `SERIAL`, draw the column's ids
//! again. The differential fixtures (`ddl/add_identity_*`) hold both refusals and which
//! error wins beside each against 18.6. These hold what a fixture cannot — its harness
//! stops at the first statement that fails — which is what a refused statement leaves
//! behind, on each of the embed's stores and across a reopen; and the property the first
//! refusal keeps, swept over the statements that could break it.

#![allow(clippy::unwrap_used, clippy::panic)]

use crate::common::ScratchDir;
use pglet::Database;
use pglet_sql::types::Value;

/// `sql`'s outcome: its last command tag and rows, or `ERR <SQLSTATE>: <message>`.
fn answer(db: &mut Database, sql: &str) -> String {
    match db.execute(sql) {
        Ok(results) => results
            .last()
            .map_or_else(String::new, |r| format!("{} {:?}", r.command_tag, r.rows)),
        Err(e) => format!("ERR {}: {e}", e.sqlstate()),
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

/// The refusal over column `c` of table `ai`, as 18.6 words it.
const NOT_DECLARED: &str = "ERR 55000: column \"c\" of relation \"ai\" must be declared NOT NULL \
                            before identity can be added";

/// The catalog facts the statement could have moved: the sequences, and the column.
fn catalog_facts(db: &Database) -> String {
    let schema = db.schema();
    format!(
        "{:?} {:?}",
        schema.sequences,
        schema.get_table("ai").map(|t| &t.columns)
    )
}

#[test]
fn the_clause_is_refused_whatever_the_rows_hold() {
    for (store, mut db) in stores() {
        for (rows, label) in [
            ("(NULL, 'null'), (1, 'one')", "a row holds a NULL"),
            ("(1, 'one'), (2, 'two')", "no row holds a NULL"),
            ("", "no row"),
        ] {
            for generation in ["ALWAYS", "BY DEFAULT"] {
                run(&mut db, "CREATE TABLE ai (c INT, v TEXT)");
                if !rows.is_empty() {
                    run(&mut db, &format!("INSERT INTO ai VALUES {rows}"));
                }
                let before = catalog_facts(&db);
                for sql in [
                    format!("ALTER TABLE ai ALTER COLUMN c ADD GENERATED {generation} AS IDENTITY"),
                    format!(
                        "ALTER TABLE IF EXISTS ai ALTER COLUMN c ADD GENERATED {generation} AS \
                         IDENTITY (SEQUENCE NAME ai_named START WITH 40)"
                    ),
                ] {
                    assert_eq!(
                        answer(&mut db, &sql),
                        NOT_DECLARED,
                        "{store}, {label}: {sql}"
                    );
                    assert_eq!(
                        catalog_facts(&db),
                        before,
                        "{store}, {label}: the refused statement moved the catalog"
                    );
                }
                run(&mut db, "DROP TABLE ai");
            }
        }
    }
}

#[test]
fn a_refused_clause_leaves_nothing_behind() {
    for (store, mut db) in stores() {
        run(&mut db, "CREATE TABLE ai (c INT, v TEXT)");
        run(&mut db, "INSERT INTO ai VALUES (1, 'one')");
        assert_eq!(
            answer(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY (START WITH 40)"
            ),
            NOT_DECLARED,
            "{store}"
        );
        // No sequence: the name the clause would have minted is free, and a sequence
        // made under it now starts where a new one starts.
        assert_eq!(
            answer(&mut db, "SELECT count(*) FROM information_schema.sequences"),
            "SELECT 1 [[Int8(0)]]",
            "{store}"
        );
        run(&mut db, "CREATE SEQUENCE ai_c_seq");
        assert_eq!(
            answer(&mut db, "SELECT nextval('ai_c_seq')"),
            "SELECT 1 [[Int8(1)]]",
            "{store}"
        );
        run(&mut db, "DROP SEQUENCE ai_c_seq");
        // The column is what it was: nullable, no identity, no default.
        assert_eq!(
            answer(
                &mut db,
                "SELECT is_nullable, is_identity, column_default FROM information_schema.columns \
                 WHERE table_name = 'ai' AND column_name = 'c'"
            ),
            "SELECT 1 [[Text(\"YES\"), Text(\"NO\"), Null]]",
            "{store}"
        );
        assert_eq!(
            answer(&mut db, "INSERT INTO ai VALUES (NULL, 'null')"),
            "INSERT 0 1 []",
            "{store}: the column still takes a NULL"
        );
        // The way in reads the rows: refused while one holds a NULL, then accepted, and
        // the clause after it draws from its `START WITH`.
        assert!(
            answer(&mut db, "ALTER TABLE ai ALTER COLUMN c SET NOT NULL").starts_with("ERR 23502"),
            "{store}"
        );
        run(&mut db, "DELETE FROM ai WHERE c IS NULL");
        run(&mut db, "ALTER TABLE ai ALTER COLUMN c SET NOT NULL");
        run(
            &mut db,
            "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY (START WITH 40)",
        );
        run(&mut db, "INSERT INTO ai (v) VALUES ('minted')");
        assert_eq!(
            answer(&mut db, "SELECT c, v FROM ai ORDER BY v"),
            "SELECT 2 [[Int4(40), Text(\"minted\")], [Int4(1), Text(\"one\")]]",
            "{store}"
        );
    }
}

#[test]
fn a_refused_clause_inside_a_block_leaves_nothing_behind() {
    for (store, mut db) in stores() {
        run(&mut db, "CREATE TABLE ai (c INT, v TEXT)");
        let before = catalog_facts(&db);
        run(&mut db, "BEGIN");
        run(&mut db, "INSERT INTO ai VALUES (1, 'one')");
        assert_eq!(
            answer(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY"
            ),
            NOT_DECLARED,
            "{store}"
        );
        run(&mut db, "ROLLBACK");
        assert_eq!(catalog_facts(&db), before, "{store}");
        // A savepoint takes the refusal back and the block goes on to the accepted form.
        run(&mut db, "BEGIN");
        run(&mut db, "INSERT INTO ai VALUES (1, 'one')");
        run(&mut db, "SAVEPOINT s");
        assert_eq!(
            answer(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY"
            ),
            NOT_DECLARED,
            "{store}"
        );
        run(&mut db, "ROLLBACK TO SAVEPOINT s");
        run(&mut db, "ALTER TABLE ai ALTER COLUMN c SET NOT NULL");
        run(
            &mut db,
            "ALTER TABLE ai ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY (START WITH 7)",
        );
        run(&mut db, "INSERT INTO ai (v) VALUES ('minted')");
        run(&mut db, "COMMIT");
        assert_eq!(
            answer(&mut db, "SELECT c FROM ai ORDER BY c"),
            "SELECT 2 [[Int4(1)], [Int4(7)]]",
            "{store}"
        );
    }
}

#[test]
fn a_refused_clause_writes_nothing_a_reopen_finds() {
    // The autocommit path cannot take a catalog write back, so a write ahead of the
    // refusal would be durable: a sequence with no column.
    let dir = ScratchDir::new("identity_declaration");
    {
        let mut db = Database::open(&dir.path).unwrap();
        run(&mut db, "CREATE TABLE ai (c INT, v TEXT)");
        run(&mut db, "INSERT INTO ai VALUES (NULL, 'null'), (1, 'one')");
        assert_eq!(
            answer(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_named)"
            ),
            NOT_DECLARED
        );
        // The next statement that succeeds is what would carry a phantom to the store:
        // it saves the catalog as it stands.
        run(&mut db, "CREATE TABLE later (x INT)");
        run(&mut db, "INSERT INTO ai VALUES (2, 'two')");
    }
    let mut db = Database::open(&dir.path).unwrap();
    assert!(
        db.schema().sequences.is_empty(),
        "{:?}",
        db.schema().sequences
    );
    let column = &db.schema().get_table("ai").unwrap().columns[0];
    assert!(column.nullable && column.identity.is_none() && column.default.is_none());
    assert_eq!(
        answer(&mut db, "SELECT v FROM ai ORDER BY c"),
        "SELECT 3 [[Text(\"one\")], [Text(\"two\")], [Text(\"null\")]]"
    );
    // The name the clause declared is free.
    run(&mut db, "CREATE SEQUENCE ai_named");
}

#[test]
fn which_refusal_wins_beside_the_declaration() {
    // Each pair measured on 18.6, which creates the identity's sequence — where the type
    // and the sequence's name are judged — before it reads the column's declaration, and
    // reads the declaration before the column's default.
    for (store, mut db) in stores() {
        for sql in [
            "CREATE TABLE ai (c INT, v TEXT)",
            "CREATE TABLE ai_text (c TEXT, v TEXT)",
            "CREATE TABLE ai_default (c INT DEFAULT 5, v TEXT)",
            "CREATE TABLE ai_taken (x INT)",
            "CREATE INDEX ai_taken_ix ON ai_taken (x)",
            "CREATE SEQUENCE ai_taken_seq",
            "CREATE TYPE ai_mood AS ENUM ('x')",
        ] {
            run(&mut db, sql);
        }
        for (sql, want) in [
            (
                "ALTER TABLE ai_missing ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                "ERR 42P01: relation \"ai_missing\" does not exist",
            ),
            (
                "ALTER TABLE ai ALTER COLUMN missing ADD GENERATED ALWAYS AS IDENTITY",
                "ERR 42703: column \"missing\" of relation \"ai\" does not exist",
            ),
            (
                "ALTER TABLE ai_text ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                "ERR 22023: identity column type must be smallint, integer, or bigint",
            ),
            (
                "ALTER TABLE ai_text ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_taken)",
                "ERR 22023: identity column type must be smallint, integer, or bigint",
            ),
            (
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_taken)",
                "ERR 42P07: relation \"ai_taken\" already exists",
            ),
            (
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_taken_ix)",
                "ERR 42P07: relation \"ai_taken_ix\" already exists",
            ),
            (
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_taken_seq)",
                "ERR 42P07: relation \"ai_taken_seq\" already exists",
            ),
            (
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_mood)",
                "ERR 42710: type \"ai_mood\" already exists",
            ),
            (
                "ALTER TABLE ai_default ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                "ERR 55000: column \"c\" of relation \"ai_default\" must be declared NOT NULL \
                 before identity can be added",
            ),
            (
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                NOT_DECLARED,
            ),
        ] {
            assert_eq!(answer(&mut db, sql), want, "{store}: {sql}");
        }
        // The default is read last: behind the type and the sequence's name, behind the
        // declaration (`ai_default` above), and behind the already-identity refusal, which
        // is what answers for an identity column's own `nextval`.
        for sql in [
            "CREATE TABLE ad (c INT NOT NULL DEFAULT 5, v TEXT)",
            "CREATE TABLE ad_text (c TEXT NOT NULL DEFAULT 'x', v TEXT)",
            "CREATE TABLE ad_identity (c INT GENERATED BY DEFAULT AS IDENTITY, v TEXT)",
        ] {
            run(&mut db, sql);
        }
        for (sql, want) in [
            (
                "ALTER TABLE ad_text ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                "ERR 22023: identity column type must be smallint, integer, or bigint",
            ),
            (
                "ALTER TABLE ad ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_taken)",
                "ERR 42P07: relation \"ai_taken\" already exists",
            ),
            (
                "ALTER TABLE ad ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_mood)",
                "ERR 42710: type \"ai_mood\" already exists",
            ),
            (
                "ALTER TABLE ad ALTER COLUMN missing ADD GENERATED ALWAYS AS IDENTITY",
                "ERR 42703: column \"missing\" of relation \"ad\" does not exist",
            ),
            (
                "ALTER TABLE ad_identity ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                "ERR 55000: column \"c\" of relation \"ad_identity\" is already an identity column",
            ),
            (
                "ALTER TABLE IF EXISTS ad ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY",
                "ERR 55000: column \"c\" of relation \"ad\" already has a default value",
            ),
        ] {
            assert_eq!(answer(&mut db, sql), want, "{store}: {sql}");
        }
        // Under `IF EXISTS` a missing table is the skip, and no refusal at all.
        let skipped = db
            .execute(
                "ALTER TABLE IF EXISTS ai_missing ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
            )
            .unwrap()
            .remove(0);
        assert_eq!(
            skipped.notices,
            vec!["relation \"ai_missing\" does not exist, skipping".to_string()],
            "{store}"
        );
    }
}

/// The refusal over column `c` of table `ai` for a default it has, as 18.6 words it.
const HAS_DEFAULT: &str = "ERR 55000: column \"c\" of relation \"ai\" already has a default value";

#[test]
fn the_clause_is_refused_over_each_kind_of_default() {
    for (store, mut db) in stores() {
        run(&mut db, "CREATE SEQUENCE ai_free START WITH 100");
        for (setup, kind) in [
            (
                &["CREATE TABLE ai (c INT NOT NULL DEFAULT 5, v TEXT)"][..],
                "a constant",
            ),
            (
                &["CREATE TABLE ai (c INT NOT NULL DEFAULT 1 + 1, v TEXT)"],
                "an expression",
            ),
            (
                &["CREATE TABLE ai (c BIGINT NOT NULL DEFAULT nextval('ai_free'), v TEXT)"],
                "a nextval over a free sequence",
            ),
            (&["CREATE TABLE ai (c SERIAL, v TEXT)"], "a serial"),
            (
                &["CREATE TABLE ai (c SMALLSERIAL PRIMARY KEY, v TEXT)"],
                "a serial key",
            ),
            (
                &[
                    "CREATE TABLE ai (v TEXT)",
                    "ALTER TABLE ai ADD COLUMN c SERIAL",
                ],
                "an added serial",
            ),
            (
                &[
                    "CREATE TABLE ai (c INT NOT NULL, v TEXT)",
                    "ALTER TABLE ai ALTER COLUMN c SET DEFAULT 3",
                ],
                "a default set later",
            ),
            (
                &[
                    "CREATE TABLE ai (v TEXT)",
                    "ALTER TABLE ai ADD COLUMN c INT NOT NULL DEFAULT 5",
                ],
                "an added column's default",
            ),
        ] {
            for sql in setup {
                run(&mut db, sql);
            }
            run(&mut db, "INSERT INTO ai (v) VALUES ('a'), ('b')");
            let before = catalog_facts(&db);
            let rows = answer(&mut db, "SELECT c, v FROM ai ORDER BY v");
            for sql in [
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY \
                 (SEQUENCE NAME ai_named START WITH 40)",
            ] {
                assert_eq!(answer(&mut db, sql), HAS_DEFAULT, "{store}, {kind}: {sql}");
                assert_eq!(
                    catalog_facts(&db),
                    before,
                    "{store}, {kind}: the refused statement moved the catalog"
                );
            }
            // The default still answers: the next row draws what it drew before.
            run(&mut db, "INSERT INTO ai (v) VALUES ('c')");
            assert_eq!(
                answer(&mut db, "SELECT c, v FROM ai WHERE v < 'c' ORDER BY v"),
                rows,
                "{store}, {kind}"
            );
            assert_eq!(
                answer(&mut db, "SELECT count(*) FROM ai WHERE c IS NULL"),
                "SELECT 1 [[Int8(0)]]",
                "{store}, {kind}"
            );
            run(&mut db, "DROP TABLE ai");
        }
    }
}

#[test]
fn a_serial_column_becomes_an_identity_column_once_its_default_is_dropped() {
    // The rows keep their ids and the next one continues past them — where the clause
    // accepted over the default would start the ids again at its own `START WITH`.
    for (store, mut db) in stores() {
        for (index, setup) in [
            &["CREATE TABLE ai (c SERIAL PRIMARY KEY, v TEXT)"][..],
            &[
                "CREATE TABLE ai (v TEXT)",
                "ALTER TABLE ai ADD COLUMN c SERIAL",
            ],
            &[
                "CREATE TABLE ai (v TEXT)",
                "ALTER TABLE ai ADD COLUMN c BIGSERIAL PRIMARY KEY",
            ],
        ]
        .iter()
        .enumerate()
        {
            for sql in *setup {
                run(&mut db, sql);
            }
            run(&mut db, "INSERT INTO ai (v) VALUES ('a'), ('b')");
            assert_eq!(
                answer(
                    &mut db,
                    "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY"
                ),
                HAS_DEFAULT,
                "{store}, {index}"
            );
            run(&mut db, "ALTER TABLE ai ALTER COLUMN c DROP DEFAULT");
            run(&mut db, "DROP SEQUENCE ai_c_seq");
            run(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY (START WITH 3)",
            );
            run(&mut db, "INSERT INTO ai (v) VALUES ('c')");
            let ids: Vec<String> = db
                .execute("SELECT c FROM ai ORDER BY v")
                .unwrap()
                .remove(0)
                .rows
                .iter()
                .map(|row| match &row[0] {
                    Value::Int4(n) => n.to_string(),
                    Value::Int8(n) => n.to_string(),
                    other => panic!("{store}, {index}: {other:?}"),
                })
                .collect();
            assert_eq!(ids, ["1", "2", "3"], "{store}, {index}");
            assert_eq!(
                answer(
                    &mut db,
                    "SELECT is_nullable, is_identity FROM information_schema.columns \
                     WHERE table_name = 'ai' AND column_name = 'c'"
                ),
                "SELECT 1 [[Text(\"NO\"), Text(\"YES\")]]",
                "{store}, {index}"
            );
            run(&mut db, "DROP TABLE ai");
        }
    }
}

#[test]
fn a_clause_refused_for_a_default_writes_nothing_a_reopen_finds() {
    let dir = ScratchDir::new("identity_default");
    {
        let mut db = Database::open(&dir.path).unwrap();
        run(&mut db, "CREATE TABLE ai (c SERIAL, v TEXT)");
        run(&mut db, "INSERT INTO ai (v) VALUES ('a'), ('b')");
        assert_eq!(
            answer(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ai_named)"
            ),
            HAS_DEFAULT
        );
        // The next statement that succeeds saves the catalog as it stands.
        run(&mut db, "INSERT INTO ai (v) VALUES ('c')");
    }
    let mut db = Database::open(&dir.path).unwrap();
    let sequences: Vec<&String> = db.schema().sequences.keys().collect();
    assert_eq!(
        sequences,
        ["ai_c_seq"],
        "the serial's sequence, and no other"
    );
    let column = &db.schema().get_table("ai").unwrap().columns[0];
    assert!(!column.nullable && column.identity.is_none() && column.default.is_some());
    run(&mut db, "INSERT INTO ai (v) VALUES ('d')");
    assert_eq!(
        answer(&mut db, "SELECT c FROM ai ORDER BY c"),
        "SELECT 4 [[Int4(1)], [Int4(2)], [Int4(3)], [Int4(4)]]"
    );
    // The name the clause declared is free.
    run(&mut db, "CREATE SEQUENCE ai_named");
}

#[test]
fn no_default_is_what_18_6_calls_none() {
    // `DEFAULT NULL`, a default removed, and one set to NULL are each no default, and the
    // clause is accepted over them.
    for (store, mut db) in stores() {
        for (index, setup) in [
            &["CREATE TABLE ai (c INT NOT NULL DEFAULT NULL, v TEXT)"][..],
            &["CREATE TABLE ai (c INT NOT NULL DEFAULT (NULL), v TEXT)"],
            &[
                "CREATE TABLE ai (c INT NOT NULL DEFAULT 5, v TEXT)",
                "ALTER TABLE ai ALTER COLUMN c DROP DEFAULT",
            ],
            &[
                "CREATE TABLE ai (c INT NOT NULL DEFAULT 5, v TEXT)",
                "ALTER TABLE ai ALTER COLUMN c SET DEFAULT NULL",
            ],
            &[
                "CREATE TABLE ai (c SERIAL, v TEXT)",
                "DROP SEQUENCE ai_c_seq CASCADE",
            ],
        ]
        .iter()
        .enumerate()
        {
            for sql in *setup {
                run(&mut db, sql);
            }
            run(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY (START WITH 9)",
            );
            run(&mut db, "INSERT INTO ai (v) VALUES ('minted')");
            assert_eq!(
                answer(&mut db, "SELECT c FROM ai"),
                "SELECT 1 [[Int4(9)]]",
                "{store}, {index}"
            );
            run(&mut db, "DROP TABLE ai");
        }
    }
}

#[test]
fn the_clause_is_accepted_over_each_way_of_declaring_not_null() {
    for (store, mut db) in stores() {
        for (index, setup) in [
            &["CREATE TABLE ai (c INT NOT NULL, v TEXT)"][..],
            &["CREATE TABLE ai (c BIGINT PRIMARY KEY, v TEXT)"],
            &["CREATE TABLE ai (c INT, v TEXT, PRIMARY KEY (c))"],
            &[
                "CREATE TABLE ai (c INT, v TEXT)",
                "ALTER TABLE ai ALTER COLUMN c SET NOT NULL",
            ],
            &[
                "CREATE TABLE ai (c SMALLINT, v TEXT)",
                "ALTER TABLE ai ADD PRIMARY KEY (c)",
            ],
            &[
                "CREATE TABLE ai (v TEXT)",
                "ALTER TABLE ai ADD COLUMN c INT NOT NULL",
            ],
        ]
        .iter()
        .enumerate()
        {
            for sql in *setup {
                run(&mut db, sql);
            }
            run(
                &mut db,
                "ALTER TABLE ai ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY (START WITH 3)",
            );
            run(&mut db, "INSERT INTO ai (v) VALUES ('minted')");
            let rows = db.execute("SELECT c FROM ai").unwrap().remove(0).rows;
            let minted = match &rows[0][0] {
                Value::Int2(n) => i64::from(*n),
                Value::Int4(n) => i64::from(*n),
                Value::Int8(n) => *n,
                other => panic!("{store}, declaration {index}: {other:?}"),
            };
            assert_eq!(minted, 3, "{store}, declaration {index}");
            let column = db
                .schema()
                .get_table("ai")
                .unwrap()
                .columns
                .iter()
                .find(|c| c.name == "c")
                .unwrap();
            assert!(
                !column.nullable && column.identity.is_some(),
                "{store}, {index}"
            );
            run(&mut db, "DROP TABLE ai");
        }
    }
}

/// Every column the catalog declares `NOT NULL` holds no NULL, and every `ORDER BY` over
/// a column with a one-column index answers what sorting the table's rows answers —
/// which is what an index-ordered read does only where no key is NULL.
fn assert_declarations_hold(db: &mut Database, label: &str) {
    let declared: Vec<(String, String)> = db
        .schema()
        .tables
        .values()
        .flat_map(|t| {
            t.columns
                .iter()
                .filter(|c| !c.nullable)
                .map(|c| (t.name.clone(), c.name.clone()))
        })
        .collect();
    for (table, column) in declared {
        assert_eq!(
            answer(
                db,
                &format!("SELECT count(*) FROM {table} WHERE {column} IS NULL")
            ),
            "SELECT 1 [[Int8(0)]]",
            "{label}: `{table}.{column}` is declared NOT NULL and holds a NULL"
        );
    }
    let indexed: Vec<(String, String)> = db
        .schema()
        .indexes
        .values()
        .filter_map(|ix| {
            ix.key
                .single_bare_column()
                .map(|column| (ix.table_name.clone(), column.to_string()))
        })
        .collect();
    for (table, column) in indexed {
        // The table's rows, sorted here as 18.6 sorts them: ascending, NULLs last.
        let mut keys: Vec<Option<i64>> = db
            .execute(&format!("SELECT {column} FROM {table}"))
            .unwrap_or_else(|e| panic!("{label}: {e}"))
            .remove(0)
            .rows
            .iter()
            .map(|row| match &row[0] {
                Value::Null => None,
                Value::Int4(n) => Some(i64::from(*n)),
                other => panic!("{label}: an unexpected key {other:?}"),
            })
            .collect();
        keys.sort_by_key(|k| (k.is_none(), *k));
        let render = |keys: &[Option<i64>]| -> String {
            let rows: Vec<String> = keys
                .iter()
                .map(|k| k.map_or_else(|| "[Null]".to_string(), |n| format!("[Int4({n})]")))
                .collect();
            format!("SELECT {} [{}]", keys.len(), rows.join(", "))
        };
        assert_eq!(
            answer(
                db,
                &format!("SELECT {column} FROM {table} ORDER BY {column}")
            ),
            render(&keys),
            "{label}: `ORDER BY {table}.{column}`"
        );
        keys.reverse();
        assert_eq!(
            answer(
                db,
                &format!("SELECT {column} FROM {table} ORDER BY {column} DESC")
            ),
            render(&keys),
            "{label}: `ORDER BY {table}.{column} DESC`"
        );
    }
}

#[test]
fn no_statement_leaves_a_null_under_a_not_null_declaration() {
    // `c` holds a NULL under a `NULLS NOT DISTINCT` index, the index that keeps an entry
    // for it; `d` holds one under a plain index; `k` is declared `NOT NULL`; `sr` takes a
    // serial column over the rows it holds. Each statement below would declare `NOT NULL`
    // over a NULL, or write a NULL under the declaration, were it accepted as written.
    // Whatever each answers, the declarations hold after it and no ordered read loses or
    // misplaces a row.
    for (store, mut db) in stores() {
        for sql in [
            "CREATE TABLE r (c INT, d INT, k INT NOT NULL, v TEXT)",
            "CREATE INDEX r_c ON r (c) NULLS NOT DISTINCT",
            "CREATE INDEX r_d ON r (d)",
            "CREATE UNIQUE INDEX r_k ON r (k)",
            "INSERT INTO r VALUES (NULL, NULL, 1, 'null'), (1, 10, 2, 'one'), (2, 20, 3, 'two')",
            "CREATE TABLE parent (id INT PRIMARY KEY)",
            "INSERT INTO parent VALUES (1), (2)",
            "CREATE TABLE child (p INT NOT NULL REFERENCES parent (id) ON DELETE SET NULL)",
            "CREATE INDEX child_p ON child (p) NULLS NOT DISTINCT",
            "INSERT INTO child VALUES (1), (2)",
            "CREATE TABLE sr (v TEXT)",
            "INSERT INTO sr VALUES ('a'), ('b'), ('c')",
        ] {
            run(&mut db, sql);
        }
        assert_declarations_hold(&mut db, &format!("{store}: the seed"));
        for sql in [
            "ALTER TABLE r ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
            "ALTER TABLE r ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY",
            "ALTER TABLE r ALTER COLUMN d ADD GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME r_d_named)",
            "ALTER TABLE IF EXISTS r ALTER COLUMN d ADD GENERATED BY DEFAULT AS IDENTITY",
            "ALTER TABLE r ALTER COLUMN c SET NOT NULL",
            "ALTER TABLE r ALTER COLUMN d SET NOT NULL",
            "ALTER TABLE r ADD PRIMARY KEY (c)",
            "ALTER TABLE r ADD CONSTRAINT r_pk PRIMARY KEY (d)",
            "ALTER TABLE r ADD PRIMARY KEY (k, c)",
            "ALTER TABLE r ADD COLUMN e INT NOT NULL",
            "ALTER TABLE r ADD COLUMN e INT NOT NULL DEFAULT NULL",
            "ALTER TABLE r ADD COLUMN e INT PRIMARY KEY",
            "INSERT INTO r VALUES (3, 30, NULL, 'x')",
            "INSERT INTO r (c, d, v) VALUES (3, 30, 'x')",
            "INSERT INTO r SELECT 3, 30, NULL, 'x'",
            "UPDATE r SET k = NULL",
            "UPDATE r SET k = NULL WHERE c = 1",
            "UPDATE r SET k = DEFAULT",
            "UPDATE r SET k = NULLIF(k, 2)",
            "INSERT INTO r VALUES (5, 50, 2, 'x') ON CONFLICT (k) DO UPDATE SET k = NULL",
            "DELETE FROM parent WHERE id = 1",
            "UPDATE child SET p = NULL",
            // A serial column added over rows: declared `NOT NULL` once each has drawn.
            "ALTER TABLE sr ADD COLUMN s SERIAL CHECK (s > 1)",
            "ALTER TABLE sr ADD COLUMN s SERIAL",
            "ALTER TABLE sr ADD COLUMN b BIGSERIAL",
            "ALTER TABLE sr ADD COLUMN IF NOT EXISTS s SMALLSERIAL",
            "CREATE INDEX sr_s ON sr (s) NULLS NOT DISTINCT",
            "INSERT INTO sr (s, v) VALUES (NULL, 'n')",
            "INSERT INTO sr (b, v) VALUES (NULL, 'n')",
            "UPDATE sr SET s = NULL WHERE v = 'a'",
            "UPDATE sr SET b = NULL",
            "INSERT INTO sr (v) VALUES ('d')",
            "ALTER TABLE sr ALTER COLUMN s ADD GENERATED ALWAYS AS IDENTITY",
            "ALTER TABLE sr ALTER COLUMN s DROP DEFAULT",
            "INSERT INTO sr (v) VALUES ('no default')",
            "UPDATE sr SET s = DEFAULT WHERE v = 'a'",
            "ALTER TABLE sr ALTER COLUMN s ADD GENERATED BY DEFAULT AS IDENTITY (START WITH 50)",
            "INSERT INTO sr (v) VALUES ('minted')",
            "INSERT INTO sr (s, v) VALUES (NULL, 'n')",
            "UPDATE sr SET s = DEFAULT WHERE v = 'b'",
        ] {
            let got = answer(&mut db, sql);
            assert_declarations_hold(&mut db, &format!("{store}: after `{sql}` ({got})"));
        }
        // Inside a block, where the rows that fail a declaration are the block's own and
        // a rollback takes a declaration back with the rows it was read over.
        for script in [
            &[
                "BEGIN",
                "DELETE FROM r WHERE c IS NULL",
                "ALTER TABLE r ALTER COLUMN c SET NOT NULL",
                "ALTER TABLE r ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY",
                "ROLLBACK",
            ][..],
            &[
                "BEGIN",
                "SAVEPOINT s",
                "DELETE FROM r WHERE d IS NULL",
                "ALTER TABLE r ALTER COLUMN d SET NOT NULL",
                "ROLLBACK TO SAVEPOINT s",
                "COMMIT",
            ],
            &[
                "BEGIN",
                "ALTER TABLE r ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY",
                "ROLLBACK",
            ],
        ] {
            for sql in script {
                let got = answer(&mut db, sql);
                if *sql == "ROLLBACK" || *sql == "COMMIT" {
                    assert_declarations_hold(
                        &mut db,
                        &format!("{store}: block ending `{sql}` ({got})"),
                    );
                }
            }
        }
        // The accepted way in, and the ordered read it makes index-served.
        for sql in [
            "DELETE FROM r WHERE c IS NULL",
            "ALTER TABLE r ALTER COLUMN c SET NOT NULL",
            "ALTER TABLE r ALTER COLUMN c ADD GENERATED BY DEFAULT AS IDENTITY (START WITH 100)",
            "INSERT INTO r (d, k, v) VALUES (30, 4, 'minted')",
        ] {
            run(&mut db, sql);
            assert_declarations_hold(&mut db, &format!("{store}: after `{sql}`"));
        }
        assert_eq!(
            answer(&mut db, "SELECT c FROM r ORDER BY c"),
            "SELECT 3 [[Int4(1)], [Int4(2)], [Int4(100)]]",
            "{store}"
        );
        // The serial routes ran, and ended where 18.6 ends them: a refused row draws its
        // other columns' defaults, so `d` takes the id after the one a refused row drew.
        assert_eq!(
            answer(&mut db, "SELECT s, v FROM sr ORDER BY s"),
            "SELECT 5 [[Int4(1), Text(\"a\")], [Int4(3), Text(\"c\")], [Int4(5), Text(\"d\")], \
             [Int4(50), Text(\"minted\")], [Int4(51), Text(\"b\")]]",
            "{store}"
        );
    }
}
