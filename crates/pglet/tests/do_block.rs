//! Minimal `DO $$ … $$` anonymous-block support over the embed engine — the
//! idempotent-`CREATE TYPE` idiom the fuz_app cell migration uses, the fail-loud
//! boundaries (an unmatched / unsupported condition), and what a failed `DO` leaves
//! behind: nothing, on every backend and after a reopen.
//!
//! The differential fixtures (`tests/fixtures/ddl/do_*.sql`) assert the same shapes'
//! answers against a live Postgres; this is the in-process counterpart, and the only
//! reader of the state a failed `DO` leaves, since the harness stops at a fixture's
//! first error.

use pglet::Database;
use pglet_sql::types::Value;
use pglet_testing::block_io::MemBlockIo;
use pglet_testing::db::open_database;

const IDEMPOTENT_CREATE_TYPE: &str = "DO $$ BEGIN
    CREATE TYPE cell_visibility AS ENUM ('private', 'public');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$";

/// The block creates the type on first run and swallows the `duplicate_object`
/// error on replay, so the migration is idempotent; the type is then usable.
#[test]
fn idempotent_create_type_replays() {
    let mut db = Database::in_memory();
    db.execute(IDEMPOTENT_CREATE_TYPE).unwrap();
    // Replays clean (the duplicate is caught by the EXCEPTION handler).
    db.execute(IDEMPOTENT_CREATE_TYPE).unwrap();
    db.execute(IDEMPOTENT_CREATE_TYPE).unwrap();

    // The enum is a real type now: usable as a column type with member validation.
    db.execute("CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private')")
        .unwrap();
    db.execute("INSERT INTO cell (id) VALUES (1)").unwrap();
    db.execute("INSERT INTO cell (id, visibility) VALUES (2, 'public')")
        .unwrap();

    let mut res = db
        .execute("SELECT id, visibility::text AS v FROM cell ORDER BY id")
        .unwrap();
    let rows = res.remove(0).rows;
    assert_eq!(
        rows,
        vec![
            vec![Value::Int4(1), Value::Text("private".into())],
            vec![Value::Int4(2), Value::Text("public".into())],
        ]
    );

    // A non-member value is still rejected (the type is genuinely enforced).
    assert!(
        db.execute("INSERT INTO cell (id, visibility) VALUES (3, 'secret')")
            .is_err()
    );
}

/// A body error whose SQLSTATE doesn't match the handler condition propagates
/// (the handler doesn't swallow everything).
#[test]
fn unmatched_condition_propagates() {
    let mut db = Database::in_memory();
    let err = db
        .execute(
            "DO $$ BEGIN
                INSERT INTO no_such_table VALUES (1);
            EXCEPTION WHEN duplicate_object THEN NULL;
            END $$",
        )
        .unwrap_err();
    // Undefined table (42P01) is not duplicate_object (42710), so it surfaces.
    assert!(format!("{err}").contains("no_such_table"), "{err}");
}

/// `WHEN OTHERS` is the catch-all — it swallows any body error.
#[test]
fn others_catches_any_error() {
    let mut db = Database::in_memory();
    // The undefined-table error is caught by OTHERS, so the block succeeds.
    db.execute(
        "DO $$ BEGIN
            INSERT INTO no_such_table VALUES (1);
        EXCEPTION WHEN OTHERS THEN NULL;
        END $$",
    )
    .unwrap();
}

/// A multi-statement body runs each statement in order, and a caught error
/// discards **every** one of them — the `EXCEPTION` clause is a subtransaction.
///
/// The opposite — the table the first statement created surviving — is what neither
/// engine does. Both halves are one measurement on
/// 18.6: the body reaches its third statement, and the catch leaves neither the
/// table nor the type. The ordering half is what the *second* `CREATE TYPE` proves
/// — it can only raise `duplicate_object` if the first one ran.
#[test]
fn a_multi_statement_body_runs_in_order_and_a_catch_discards_all_of_it() {
    let mut db = Database::in_memory();
    db.execute(
        "DO $$ BEGIN
            CREATE TABLE made (id INT);
            CREATE TYPE made_enum AS ENUM ('x');
            CREATE TYPE made_enum AS ENUM ('x');
        EXCEPTION WHEN duplicate_object THEN NULL;
        END $$",
    )
    .unwrap();
    // The duplicate-type error was caught (the block returned `Ok`), which is only
    // possible if the first `CREATE TYPE` ran — and the catch then unwound all three.
    assert_eq!(
        db.execute("INSERT INTO made VALUES (1)")
            .unwrap_err()
            .sqlstate(),
        "42P01",
    );
    // The type the body created is gone too, so its name is free — a *surviving*
    // one would answer `42710` here. This is the assertion the catalog half rests
    // on: the table above could also be missing because the `CREATE TABLE` never
    // ran, while the type's name is only free if the second `CREATE TYPE` **did**
    // run (it raised the caught error) and the first was then unwound.
    db.execute("CREATE TYPE made_enum AS ENUM ('y')").unwrap();
}

/// An unsupported / unknown EXCEPTION condition name fails loud at parse time
/// (better a clear error than a handler that silently catches nothing).
#[test]
fn unknown_condition_fails_loud() {
    let mut db = Database::in_memory();
    let err = db
        .execute(
            "DO $$ BEGIN
                CREATE TYPE t AS ENUM ('a');
            EXCEPTION WHEN no_such_condition THEN NULL;
            END $$",
        )
        .unwrap_err();
    assert!(
        format!("{err}").contains("no_such_condition"),
        "expected unsupported-condition error, got: {err}"
    );
}

/// A `DO` body's `NOWAIT` / `SKIP LOCKED` is refused, and refused **before the block
/// runs** — nothing in a multi-statement body reaches storage.
///
/// The body is parsed by a sub-parser whose deferred refusal the outer parser adopts
/// (`Parser::adopt_deferred_refusals`), so the whole statement fails at parse time and
/// the earlier `INSERT` never executes. The SQLSTATE half is pinned in
/// `pglet_sql`'s `parser::select::tests`; what only a `Database` can assert is this
/// half — a move of the adoption to execute time would leave `77` behind and redden
/// here alone.
#[test]
fn a_do_bodys_wait_policy_is_refused_before_the_block_runs() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE dm (a INT)").unwrap();
    db.execute("CREATE TABLE ds (a INT)").unwrap();
    db.execute("INSERT INTO ds VALUES (1), (2)").unwrap();

    let err = db.execute(
        "DO $$ BEGIN
            INSERT INTO dm VALUES (77);
            INSERT INTO dm SELECT a FROM ds FOR UPDATE NOWAIT;
        END $$",
    );
    assert!(err.is_err(), "a DO body's wait policy must be loud");

    let mut results = db.execute("SELECT a FROM dm").unwrap();
    assert!(
        results.remove(0).rows.is_empty(),
        "the refusal precedes the block: the first INSERT must not have run"
    );

    // The control: the same body without a wait policy runs both statements.
    db.execute(
        "DO $$ BEGIN
            INSERT INTO dm VALUES (77);
            INSERT INTO dm SELECT a FROM ds FOR UPDATE;
        END $$",
    )
    .unwrap();
    let mut results = db.execute("SELECT a FROM dm ORDER BY a").unwrap();
    assert_eq!(
        results.remove(0).rows,
        vec![
            vec![Value::Int4(1)],
            vec![Value::Int4(2)],
            vec![Value::Int4(77)]
        ]
    );
}

/// `sql`'s last result as text — its rows joined, or `ERR <SQLSTATE>`.
fn answer(db: &mut Database, sql: &str) -> String {
    match db.execute(sql) {
        Ok(results) => results.last().map_or_else(String::new, |r| {
            r.rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect::<Vec<_>>()
                .join(",")
        }),
        Err(e) => format!("ERR {}", e.sqlstate()),
    }
}

/// The three backends a statement runs on: the file store (over an in-memory device, so
/// it reopens), the in-memory B+tree, and memory storage.
fn backends() -> Vec<(&'static str, Database, Option<MemBlockIo>)> {
    let io = MemBlockIo::new();
    vec![
        ("file store", open_database(Box::new(io.clone())), Some(io)),
        ("in-memory B+tree", Database::with_btree(), None),
        ("memory storage", Database::in_memory(), None),
    ]
}

/// What a failed `DO` must leave standing: `keep` with its two rows, two columns and
/// one index; `gone` with its two rows; `ks` one draw in.
const FAILED_DO_SETUP: [&str; 6] = [
    "CREATE TABLE keep (id INT PRIMARY KEY, v TEXT)",
    "INSERT INTO keep VALUES (1, 'a'), (2, 'b')",
    "CREATE TABLE gone (i INT)",
    "INSERT INTO gone VALUES (7), (8)",
    "CREATE TABLE other (i INT)",
    "CREATE SEQUENCE ks",
];

/// A failed `DO` statement leaves the database exactly as it found it — catalog, rows,
/// storage directory and index registry, live and on a reopen — whichever statement of
/// the body fails and whatever wraps it. On 18.6 a `DO` is one statement and a failed
/// statement rolls back whole, so each script below answers its body's own SQLSTATE and
/// leaves nothing: measured there as the fixtures `ddl/do_failed_*` record the errors of.
///
/// The autocommit host rolls back the statement's staged storage writes and nothing
/// else, so what this pins is mostly the catalog: without the block's own undo every
/// object an earlier body statement created stood in the session (and in the catalog
/// the next successful write persisted), every table it dropped stayed dropped, and a
/// created table's storage was discarded under it — `INSERT` into it then answered
/// `42P01` while `CREATE TABLE` of its name answered `42P07`.
///
/// Each read after the failure is chosen so a leak changes its answer: the column list
/// and the index read `keep`'s shape, the `CREATE`s of every name the body made answer
/// `42P07` / `42710` over a survivor, and `gone`'s rows are its `DROP`'s undo. The
/// `other` write is a **successful** autocommit write after the failure, which is what
/// persisted a leak before a reopen could see it.
#[test]
fn a_failed_do_leaves_the_database_as_it_found_it() {
    const SCRIPTS: [(&str, &str, &str); 7] = [
        (
            "DDL of several kinds and rows, failing at the last statement",
            "DO $$ BEGIN \
               CREATE SEQUENCE fs; \
               CREATE TABLE ft (id SERIAL PRIMARY KEY, u INT UNIQUE, c INT CHECK (c > 0)); \
               INSERT INTO ft (u, c) VALUES (1, 1); \
               CREATE TYPE fe AS ENUM ('a'); \
               CREATE INDEX keep_v ON keep (v); \
               ALTER TABLE keep ADD COLUMN w INT DEFAULT 5; \
               INSERT INTO keep (id, v) VALUES (3, 'c'); \
               DROP TABLE gone; \
               INSERT INTO nosuch VALUES (1); \
             END $$",
            "42P01",
        ),
        (
            "failing at the first statement",
            "DO $$ BEGIN \
               INSERT INTO nosuch VALUES (1); \
               CREATE TABLE ft (a INT); \
             END $$",
            "42P01",
        ),
        (
            "a constraint error after DDL and DML",
            "DO $$ BEGIN \
               CREATE TABLE ft (a INT); \
               DROP TABLE gone; \
               INSERT INTO keep VALUES (3, 'c'); \
               INSERT INTO keep VALUES (1, 'dup'); \
             END $$",
            "23505",
        ),
        (
            "an EXCEPTION clause whose condition does not match",
            "DO $$ BEGIN \
               CREATE TABLE ft (a INT); \
               CREATE TYPE fe AS ENUM ('a'); \
               DROP TABLE gone; \
               INSERT INTO nosuch VALUES (1); \
             EXCEPTION WHEN unique_violation THEN NULL; END $$",
            "42P01",
        ),
        (
            "a handler that fails after writing",
            "DO $$ BEGIN \
               CREATE TABLE ft (a INT); \
               INSERT INTO nosuch VALUES (1); \
             EXCEPTION WHEN others THEN \
               CREATE TABLE fh (a INT); \
               CREATE SEQUENCE fs; \
               DROP TABLE gone; \
               INSERT INTO keep VALUES (3, 'c'); \
               INSERT INTO nosuch VALUES (2); \
             END $$",
            "42P01",
        ),
        (
            "a nested DO that fails",
            "DO $$ BEGIN \
               CREATE TABLE ft (a INT); \
               DO $x$ BEGIN CREATE TYPE fe AS ENUM ('a'); DROP TABLE gone; \
                 INSERT INTO nosuch VALUES (1); END $x$; \
             END $$",
            "42P01",
        ),
        (
            "a nested DO whose handler caught, then the outer body fails",
            "DO $$ BEGIN \
               DO $x$ BEGIN CREATE TABLE fx (a INT); INSERT INTO nosuch VALUES (1); \
                 EXCEPTION WHEN others THEN CREATE TABLE fh (a INT); END $x$; \
               DROP TABLE gone; \
               INSERT INTO nosuch VALUES (1); \
             END $$",
            "42P01",
        ),
    ];
    for (script, sql, sqlstate) in SCRIPTS {
        for (backend, mut db, io) in backends() {
            let cell = format!("{script}, {backend}");
            for setup in FAILED_DO_SETUP {
                db.execute(setup).unwrap();
            }
            assert_eq!(answer(&mut db, sql), format!("ERR {sqlstate}"), "{cell}");
            let reads = |db: &mut Database| {
                [
                    answer(db, "SELECT id, v FROM keep ORDER BY id"),
                    answer(
                        db,
                        "SELECT column_name::text FROM information_schema.columns \
                         WHERE table_name = 'keep' ORDER BY ordinal_position",
                    ),
                    answer(
                        db,
                        "SELECT indexname::text FROM pg_indexes WHERE tablename = 'keep' \
                         ORDER BY 1",
                    ),
                    answer(db, "SELECT i FROM gone ORDER BY i"),
                    answer(
                        db,
                        "SELECT count(*) FROM pg_class \
                         WHERE relname IN ('fs', 'ft', 'ft_id_seq', 'fh', 'fx', 'keep_v')",
                    ),
                    answer(db, "SELECT count(*) FROM pg_type WHERE typname = 'fe'"),
                ]
            };
            let whole = [
                "Int4(1)|Text(\"a\"),Int4(2)|Text(\"b\")".to_string(),
                "Text(\"id\"),Text(\"v\")".to_string(),
                "Text(\"keep_pkey\")".to_string(),
                "Int4(7),Int4(8)".to_string(),
                "Int8(0)".to_string(),
                "Int8(0)".to_string(),
            ];
            assert_eq!(reads(&mut db), whole, "{cell}: live");
            assert_eq!(
                answer(&mut db, "INSERT INTO other VALUES (1)"),
                "",
                "{cell}"
            );
            let mut db = match io {
                Some(io) => {
                    drop(db);
                    let mut reopened = open_database(Box::new(io.reopen()));
                    assert_eq!(reads(&mut reopened), whole, "{cell}: after a reopen");
                    reopened
                }
                None => db,
            };
            // Every name the body made is free, and what it would have made works: a
            // storage directory or index registry still holding one refuses here.
            for sql in [
                "CREATE SEQUENCE fs",
                "CREATE TABLE ft (a INT)",
                "INSERT INTO ft VALUES (1)",
                "CREATE TABLE fh (a INT)",
                "CREATE TABLE fx (a INT)",
                "CREATE TYPE fe AS ENUM ('z')",
                "CREATE INDEX keep_v ON keep (v)",
                "ALTER TABLE keep ADD COLUMN w INT DEFAULT 5",
                "INSERT INTO keep (id, v) VALUES (3, 'c')",
            ] {
                assert_eq!(answer(&mut db, sql), "", "{cell}: {sql}");
            }
            assert_eq!(
                answer(&mut db, "SELECT id, w FROM keep WHERE v = 'c'"),
                "Int4(3)|Int4(5)",
                "{cell}"
            );
        }
    }
}

/// A sequence a failed `DO` advanced keeps the advance, and one whose cell it replaced
/// gets back the cell its in-place advances left — the repo's sequence rule, which
/// `Schema::restore_from` carries. Measured on 18.6 over `ks` drawn to `3`: a body
/// that draws once and then `ALTER SEQUENCE … RESTART WITH 100` before failing leaves
/// the next draw `5` (the in-body draw `4` kept, the restart undone); a body that draws
/// and then `DROP SEQUENCE`s it leaves the sequence standing and the next draw `7`; a
/// caught body that draws and restarts, whose handler draws and restarts again and then
/// fails, leaves the next draw `10` — the handler's own draw kept, so the cell its undo
/// restores is the one the handler replaced, not the body's; a `TRUNCATE … RESTART
/// IDENTITY` over a serial table leaves both rows and the next id `3`.
#[test]
fn a_failed_do_keeps_a_sequence_advance_and_restores_a_replaced_cell() {
    for (backend, mut db, _) in backends() {
        for sql in [
            "CREATE SEQUENCE ks",
            "CREATE TABLE d (n BIGINT)",
            "CREATE TABLE rt (id SERIAL PRIMARY KEY, v INT)",
            "INSERT INTO rt (v) VALUES (1), (2)",
        ] {
            db.execute(sql).unwrap();
        }
        assert_eq!(
            answer(
                &mut db,
                "SELECT nextval('ks'), nextval('ks'), nextval('ks')"
            ),
            "Int8(1)|Int8(2)|Int8(3)"
        );
        for (sql, next) in [
            (
                "DO $$ BEGIN INSERT INTO d VALUES (nextval('ks')); \
                 ALTER SEQUENCE ks RESTART WITH 100; INSERT INTO nosuch VALUES (1); END $$",
                "Int8(5)",
            ),
            (
                "DO $$ BEGIN INSERT INTO d VALUES (nextval('ks')); DROP SEQUENCE ks; \
                 INSERT INTO nosuch VALUES (1); END $$",
                "Int8(7)",
            ),
            (
                "DO $$ BEGIN INSERT INTO d VALUES (nextval('ks')); \
                 ALTER SEQUENCE ks RESTART WITH 100; INSERT INTO nosuch VALUES (1); \
                 EXCEPTION WHEN others THEN INSERT INTO d VALUES (nextval('ks')); \
                 ALTER SEQUENCE ks RESTART WITH 200; INSERT INTO nosuch VALUES (2); END $$",
                "Int8(10)",
            ),
        ] {
            assert_eq!(answer(&mut db, sql), "ERR 42P01", "{backend}: {sql}");
            assert_eq!(
                answer(&mut db, "SELECT nextval('ks')"),
                next,
                "{backend}: {sql}"
            );
        }
        assert_eq!(
            answer(&mut db, "SELECT count(*) FROM d"),
            "Int8(0)",
            "{backend}"
        );
        assert_eq!(
            answer(
                &mut db,
                "DO $$ BEGIN TRUNCATE rt RESTART IDENTITY; INSERT INTO rt (v) VALUES (9); \
                 INSERT INTO nosuch VALUES (1); END $$"
            ),
            "ERR 42P01",
            "{backend}"
        );
        assert_eq!(
            answer(&mut db, "SELECT id, v FROM rt ORDER BY id"),
            "Int4(1)|Int4(1),Int4(2)|Int4(2)",
            "{backend}"
        );
        assert_eq!(
            answer(&mut db, "INSERT INTO rt (v) VALUES (3) RETURNING id"),
            "Int4(3)",
            "{backend}"
        );
    }
}

/// A failed `DO` inside a block restores the counter cells its own statements replaced
/// before its error returns — a failed `DO` reports none to the block, so its own undo is
/// the only one that reads them — and the block's rollback finds each sequence on its own
/// cell.
/// Measured on 18.6 over `bs` drawn to `3`: a block that draws `4`, runs a failing body's
/// `RESTART WITH 100` and rolls back leaves the next draw `5`; a block that draws `6`, runs
/// a failing body's `DROP SEQUENCE` + `CREATE SEQUENCE … START WITH 50` and rolls back
/// leaves it `7`. Without the `DO`'s undo the two answered `100` and `50`.
#[test]
fn a_failed_do_inside_a_block_restores_the_cell_it_replaced() {
    for (backend, mut db, _) in backends() {
        db.execute("CREATE SEQUENCE bs").unwrap();
        assert_eq!(
            answer(
                &mut db,
                "SELECT nextval('bs'), nextval('bs'), nextval('bs')"
            ),
            "Int8(1)|Int8(2)|Int8(3)"
        );
        for (body, drawn, next) in [
            ("ALTER SEQUENCE bs RESTART WITH 100", "Int8(4)", "Int8(5)"),
            (
                "DROP SEQUENCE bs; CREATE SEQUENCE bs START WITH 50",
                "Int8(6)",
                "Int8(7)",
            ),
        ] {
            assert_eq!(answer(&mut db, "BEGIN"), "", "{backend}");
            assert_eq!(answer(&mut db, "SELECT nextval('bs')"), drawn, "{backend}");
            assert_eq!(
                answer(
                    &mut db,
                    &format!("DO $$ BEGIN {body}; INSERT INTO nosuch VALUES (1); END $$")
                ),
                "ERR 42P01",
                "{backend}: {body}"
            );
            assert_eq!(answer(&mut db, "ROLLBACK"), "", "{backend}");
            assert_eq!(
                answer(&mut db, "SELECT nextval('bs')"),
                next,
                "{backend}: {body}"
            );
        }
    }
}

/// A failed `DO` inside an open block aborts the block (`25P02` for what follows, as on
/// 18.6) and takes nothing of the block's own earlier work with it: a `ROLLBACK TO` a
/// savepoint taken before it lets the block go on and commit, with the `DO`'s objects
/// gone and the block's own write kept. Both routes were already right through the
/// block's frames; this is the control that the `DO`'s own undo reaches no further back
/// than its own statement.
#[test]
fn a_failed_do_inside_a_block_undoes_only_itself() {
    for (backend, mut db, io) in backends() {
        for setup in FAILED_DO_SETUP {
            db.execute(setup).unwrap();
        }
        let failing = "DO $$ BEGIN CREATE TABLE ft (a INT); DROP TABLE gone; \
                       INSERT INTO keep VALUES (4, 'd'); INSERT INTO nosuch VALUES (1); END $$";
        for sql in ["BEGIN", "INSERT INTO keep VALUES (3, 'c')"] {
            assert_eq!(answer(&mut db, sql), "", "{backend}: {sql}");
        }
        assert_eq!(answer(&mut db, failing), "ERR 42P01", "{backend}");
        assert_eq!(answer(&mut db, "SELECT 1"), "ERR 25P02", "{backend}");
        assert_eq!(answer(&mut db, "ROLLBACK"), "", "{backend}");

        for sql in ["BEGIN", "INSERT INTO keep VALUES (3, 'c')", "SAVEPOINT sp"] {
            assert_eq!(answer(&mut db, sql), "", "{backend}: {sql}");
        }
        assert_eq!(answer(&mut db, failing), "ERR 42P01", "{backend}");
        for sql in ["ROLLBACK TO sp", "CREATE TABLE ft (a INT)", "COMMIT"] {
            assert_eq!(answer(&mut db, sql), "", "{backend}: {sql}");
        }
        let reads = |db: &mut Database| {
            [
                answer(db, "SELECT id FROM keep ORDER BY id"),
                answer(db, "SELECT i FROM gone ORDER BY i"),
                answer(db, "SELECT count(*) FROM ft"),
            ]
        };
        let expected = [
            "Int4(1),Int4(2),Int4(3)".to_string(),
            "Int4(7),Int4(8)".to_string(),
            "Int8(0)".to_string(),
        ];
        assert_eq!(reads(&mut db), expected, "{backend}: live");
        if let Some(io) = io {
            drop(db);
            let mut reopened = open_database(Box::new(io.reopen()));
            assert_eq!(reads(&mut reopened), expected, "{backend}: after a reopen");
        }
    }
}

/// The accept side: a `DO` that succeeds keeps everything its body did — and a caught
/// one keeps its handler's writes — live and on a reopen. And every `DO` takes its
/// savepoint level back off the backend's stack on success too: a level left behind
/// makes the next block's `SAVEPOINT` fail the session's lockstep check, or its
/// `ROLLBACK TO` restore the wrong level, which the block after the `DO`s reads.
#[test]
fn a_successful_do_keeps_its_body_and_a_caught_one_its_handler() {
    for (backend, mut db, io) in backends() {
        for setup in FAILED_DO_SETUP {
            db.execute(setup).unwrap();
        }
        for sql in [
            "DO $$ BEGIN CREATE TABLE ft (a INT); INSERT INTO ft VALUES (1); DROP TABLE gone; END $$",
            "DO $$ BEGIN CREATE TABLE fx (a INT); INSERT INTO nosuch VALUES (1); \
             EXCEPTION WHEN others THEN CREATE TABLE fh (a INT); INSERT INTO fh VALUES (2); END $$",
        ] {
            assert_eq!(answer(&mut db, sql), "", "{backend}: {sql}");
        }
        let reads = |db: &mut Database| {
            [
                answer(db, "SELECT a FROM ft"),
                answer(db, "SELECT a FROM fh"),
                answer(db, "SELECT count(*) FROM gone"),
                answer(db, "SELECT count(*) FROM fx"),
            ]
        };
        let expected = [
            "Int4(1)".to_string(),
            "Int4(2)".to_string(),
            "ERR 42P01".to_string(),
            "ERR 42P01".to_string(),
        ];
        assert_eq!(reads(&mut db), expected, "{backend}: live");
        let mut db = match io {
            Some(io) => {
                drop(db);
                let mut reopened = open_database(Box::new(io.reopen()));
                assert_eq!(reads(&mut reopened), expected, "{backend}: after a reopen");
                reopened
            }
            None => db,
        };
        for sql in [
            "BEGIN",
            "DO $$ BEGIN INSERT INTO ft VALUES (3); END $$",
            "SAVEPOINT sp",
            "INSERT INTO ft VALUES (4)",
            "ROLLBACK TO sp",
            "COMMIT",
        ] {
            assert_eq!(answer(&mut db, sql), "", "{backend}: {sql}");
        }
        assert_eq!(
            answer(&mut db, "SELECT a FROM ft ORDER BY a"),
            "Int4(1),Int4(3)",
            "{backend}"
        );
    }
}

/// Run `steps` in order and return the answer of `probe`. A step ending ` -- 42P01` must
/// answer that error; a `SELECT` step must succeed; any other must succeed with no rows.
fn run_then(db: &mut Database, label: &str, steps: &[&str], probe: &str) -> String {
    for step in steps {
        let (sql, expected) = step
            .strip_suffix(" -- 42P01")
            .map_or((*step, ""), |sql| (sql, "ERR 42P01"));
        let got = answer(db, sql);
        // A `SELECT nextval(…)` step answers its draw; only its success is asserted.
        if expected.is_empty() && sql.starts_with("SELECT") {
            assert!(!got.starts_with("ERR"), "{label}: {sql}: {got}");
        } else {
            assert_eq!(got, expected, "{label}: {sql}");
        }
    }
    answer(db, probe)
}

/// A `DO` nested in another is one of its statements, so the cells it replaced are among
/// the ones the outer `DO`'s undo puts back — whether the inner one completed (it hands
/// them up in its result) or failed and was caught (its own undo put them back already).
/// Measured on 18.6 over a sequence drawn to `3` (and a serial table holding ids `1`–`3`):
/// every shape below answers the next value `4`, the block `5` (its own draw kept). Before
/// the inner `DO`'s swaps reached the outer list they answered `100`, `1`, a `23505` on the
/// insert, `500`, `900`, `70`, `40`, `60` and `302` — the inner cell, or a counter the outer
/// undo never put back.
#[test]
fn a_do_undo_restores_the_cells_a_nested_do_replaced() {
    const INNER_RESTART_FAILED_OUTER: &str = "DO $o$ BEGIN DO $i$ BEGIN ALTER SEQUENCE na \
        RESTART WITH 100; END $i$; INSERT INTO nosuch VALUES (1); END $o$ -- 42P01";
    let cases: [(&str, &[&str], &str, &str); 9] = [
        (
            "inner RESTART, failed outer",
            &["CREATE SEQUENCE na", INNER_RESTART_FAILED_OUTER],
            "SELECT nextval('na')",
            "Int8(4)",
        ),
        (
            "inner DROP and CREATE SEQUENCE, failed outer",
            &[
                "CREATE SEQUENCE nb",
                "DO $o$ BEGIN DO $i$ BEGIN DROP SEQUENCE nb; CREATE SEQUENCE nb; END $i$; \
                 INSERT INTO nosuch VALUES (1); END $o$ -- 42P01",
            ],
            "SELECT nextval('nb')",
            "Int8(4)",
        ),
        (
            "inner TRUNCATE RESTART IDENTITY, failed outer",
            &[
                "CREATE TABLE nc (id SERIAL PRIMARY KEY, v INT)",
                "DO $o$ BEGIN DO $i$ BEGIN TRUNCATE nc RESTART IDENTITY; END $i$; \
                 INSERT INTO nosuch VALUES (1); END $o$ -- 42P01",
            ],
            "INSERT INTO nc (v) VALUES (4) RETURNING id",
            "Int4(4)",
        ),
        (
            "inner RESTART, caught outer",
            &[
                "CREATE SEQUENCE nd",
                "DO $o$ BEGIN DO $i$ BEGIN ALTER SEQUENCE nd RESTART WITH 500; END $i$; \
                 INSERT INTO nosuch VALUES (1); EXCEPTION WHEN others THEN NULL; END $o$",
            ],
            "SELECT nextval('nd')",
            "Int8(4)",
        ),
        (
            "inner RESTART, failed outer in a block rolled back",
            &[
                "CREATE SEQUENCE ne",
                "BEGIN",
                "SELECT nextval('ne')",
                "DO $o$ BEGIN DO $i$ BEGIN ALTER SEQUENCE ne RESTART WITH 900; END $i$; \
                 INSERT INTO nosuch VALUES (1); END $o$ -- 42P01",
                "ROLLBACK",
            ],
            "SELECT nextval('ne')",
            "Int8(5)",
        ),
        (
            "two levels down, failed outer",
            &[
                "CREATE SEQUENCE nf",
                "DO $o$ BEGIN DO $m$ BEGIN DO $i$ BEGIN ALTER SEQUENCE nf RESTART WITH 70; \
                 END $i$; END $m$; INSERT INTO nosuch VALUES (1); END $o$ -- 42P01",
            ],
            "SELECT nextval('nf')",
            "Int8(4)",
        ),
        (
            "an inner handler's RESTART, failed outer",
            &[
                "CREATE SEQUENCE ng",
                "DO $o$ BEGIN DO $i$ BEGIN INSERT INTO nosuch VALUES (1); EXCEPTION WHEN \
                 others THEN ALTER SEQUENCE ng RESTART WITH 40; END $i$; \
                 INSERT INTO nosuch VALUES (1); END $o$ -- 42P01",
            ],
            "SELECT nextval('ng')",
            "Int8(4)",
        ),
        (
            "an inner DO in the outer's failing handler",
            &[
                "CREATE SEQUENCE nh",
                "DO $o$ BEGIN INSERT INTO nosuch VALUES (1); EXCEPTION WHEN others THEN \
                 DO $i$ BEGIN ALTER SEQUENCE nh RESTART WITH 60; END $i$; \
                 INSERT INTO nosuch VALUES (2); END $o$ -- 42P01",
            ],
            "SELECT nextval('nh')",
            "Int8(4)",
        ),
        (
            "inner draw from its new cell, failed outer",
            &[
                "CREATE SEQUENCE ni",
                "CREATE TABLE nil (v BIGINT)",
                "DO $o$ BEGIN DO $i$ BEGIN ALTER SEQUENCE ni RESTART WITH 300; \
                 INSERT INTO nil VALUES (nextval('ni')); END $i$; \
                 INSERT INTO nil VALUES (nextval('ni')); INSERT INTO nosuch VALUES (1); \
                 END $o$ -- 42P01",
            ],
            "SELECT nextval('ni')",
            "Int8(4)",
        ),
    ];
    for (backend, mut db, _) in backends() {
        for (label, steps, probe, expected) in cases {
            // Draw the case's sequence, or fill its serial table, to `3` first.
            let (create, rest) = steps.split_first().unwrap();
            assert_eq!(answer(&mut db, create), "", "{backend}: {label}");
            let fill = if create.starts_with("CREATE TABLE nc") {
                "INSERT INTO nc (v) VALUES (1), (2), (3)".to_string()
            } else {
                let seq = create.trim_start_matches("CREATE SEQUENCE ");
                format!("SELECT nextval('{seq}'), nextval('{seq}'), nextval('{seq}')")
            };
            db.execute(&fill).unwrap();
            let label = format!("{backend}: {label}");
            assert_eq!(run_then(&mut db, &label, rest, probe), expected, "{label}");
        }
    }
}

/// A `DO` that completes inside a block hands the counter cells its statements replaced to
/// the block, so the block's rollback puts each sequence back on its own cell, as the
/// in-place advances left it — the host reads them off the `DO`'s result, since
/// `Plan::swapped_sequences` cannot list a `DO`'s ahead of it. Measured on 18.6 over a
/// sequence drawn to `3`, or a `(x INT, id SERIAL)` table holding ids `1`–`3`: each shape
/// answers `4`, or `5` where the block itself drew `4` before the `DO` dropped the cell.
/// Before the hand-off the block's rollback read nothing of the `DO` and answered the
/// replacing cell's counter or the `BEGIN` one: `100`, `100`, `1`, `1`, `1`, `1`, `4`,
/// `4`, `100` and `101`. On the file store each shape also reopens at its answer.
#[test]
fn a_block_rollback_restores_the_cells_a_completed_do_replaced() {
    let cases: [(&str, &str, &[&str], &str, &str); 11] = [
        (
            "body RESTART",
            "CREATE SEQUENCE r1",
            &[
                "BEGIN",
                "DO $$ BEGIN ALTER SEQUENCE r1 RESTART WITH 100; END $$",
                "ROLLBACK",
            ],
            "SELECT nextval('r1')",
            "Int8(4)",
        ),
        (
            "handler RESTART",
            "CREATE SEQUENCE r2",
            &[
                "BEGIN",
                "DO $$ BEGIN INSERT INTO nosuch VALUES (1); EXCEPTION WHEN others THEN \
                 ALTER SEQUENCE r2 RESTART WITH 100; END $$",
                "ROLLBACK",
            ],
            "SELECT nextval('r2')",
            "Int8(4)",
        ),
        (
            "body DROP and CREATE SEQUENCE",
            "CREATE SEQUENCE r3",
            &[
                "BEGIN",
                "DO $$ BEGIN DROP SEQUENCE r3; CREATE SEQUENCE r3; END $$",
                "ROLLBACK",
            ],
            "SELECT nextval('r3')",
            "Int8(4)",
        ),
        (
            "body TRUNCATE RESTART IDENTITY",
            "CREATE TABLE w4 (x INT, id SERIAL)",
            &[
                "BEGIN",
                "DO $$ BEGIN TRUNCATE w4 RESTART IDENTITY; END $$",
                "ROLLBACK",
            ],
            "INSERT INTO w4 (x) VALUES (4) RETURNING id",
            "Int4(4)",
        ),
        (
            "body DROP TABLE and re-create",
            "CREATE TABLE w5 (x INT, id SERIAL)",
            &[
                "BEGIN",
                "DO $$ BEGIN DROP TABLE w5; CREATE TABLE w5 (x INT, id SERIAL); END $$",
                "ROLLBACK",
            ],
            "INSERT INTO w5 (x) VALUES (4) RETURNING id",
            "Int4(4)",
        ),
        (
            "body DROP SCHEMA public and re-create",
            "CREATE SEQUENCE r6",
            &[
                "BEGIN",
                "DO $$ BEGIN DROP SCHEMA public CASCADE; CREATE SCHEMA public; \
                 CREATE SEQUENCE r6; END $$",
                "ROLLBACK",
            ],
            "SELECT nextval('r6')",
            "Int8(4)",
        ),
        (
            "DROP TABLE and re-create, ROLLBACK TO then COMMIT",
            "CREATE TABLE w7 (x INT, id SERIAL)",
            &[
                "BEGIN",
                "SAVEPOINT a",
                "DO $$ BEGIN DROP TABLE w7; CREATE TABLE w7 (x INT, id SERIAL); END $$",
                "ROLLBACK TO SAVEPOINT a",
                "COMMIT",
            ],
            "INSERT INTO w7 (x) VALUES (4) RETURNING id",
            "Int4(4)",
        ),
        (
            "a draw, then body DROP SEQUENCE",
            "CREATE SEQUENCE r8",
            &[
                "BEGIN",
                "SELECT nextval('r8')",
                "DO $$ BEGIN DROP SEQUENCE r8; END $$",
                "ROLLBACK",
            ],
            "SELECT nextval('r8')",
            "Int8(5)",
        ),
        (
            "an insert, then body DROP COLUMN",
            "CREATE TABLE w9 (x INT, id SERIAL)",
            &[
                "BEGIN",
                "SELECT nextval('w9_id_seq')",
                "DO $$ BEGIN ALTER TABLE w9 DROP COLUMN id; END $$",
                "ROLLBACK",
            ],
            "INSERT INTO w9 (x) VALUES (5) RETURNING id",
            "Int4(5)",
        ),
        (
            "a nested DO's RESTART",
            "CREATE SEQUENCE r10",
            &[
                "BEGIN",
                "DO $o$ BEGIN DO $i$ BEGIN ALTER SEQUENCE r10 RESTART WITH 100; END $i$; END $o$",
                "ROLLBACK",
            ],
            "SELECT nextval('r10')",
            "Int8(4)",
        ),
        (
            "a draw, a caught body's RESTART and the handler's",
            "CREATE SEQUENCE r12",
            &[
                "BEGIN",
                "SELECT nextval('r12')",
                "DO $$ BEGIN ALTER SEQUENCE r12 RESTART WITH 50; INSERT INTO nosuch VALUES (1); \
                 EXCEPTION WHEN others THEN ALTER SEQUENCE r12 RESTART WITH 100; END $$",
                "SELECT nextval('r12')",
                "ROLLBACK",
            ],
            "SELECT nextval('r12')",
            "Int8(5)",
        ),
    ];
    let fill = |db: &mut Database, create: &str| {
        let fill = create.strip_prefix("CREATE TABLE ").map_or_else(
            || {
                let seq = create.trim_start_matches("CREATE SEQUENCE ");
                format!("SELECT nextval('{seq}'), nextval('{seq}'), nextval('{seq}')")
            },
            |rest| {
                let table = rest.split_whitespace().next().unwrap();
                format!("INSERT INTO {table} (x) VALUES (1), (2), (3)")
            },
        );
        db.execute(create).unwrap();
        db.execute(&fill).unwrap();
    };
    for (backend, mut db, _) in backends() {
        for (label, create, steps, probe, expected) in cases {
            fill(&mut db, create);
            let label = format!("{backend}: {label}");
            assert_eq!(run_then(&mut db, &label, steps, probe), expected, "{label}");
        }
    }
    // The durable half, one fresh file store per shape, closed after the block ends: the
    // rollback's counter save and the restored catalog agree, so a reopen answers what
    // the live database answers.
    for (label, create, steps, probe, expected) in cases {
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        fill(&mut db, create);
        let label = format!("file store: {label}, reopened");
        assert_eq!(run_then(&mut db, &label, steps, "SELECT 1"), "Int4(1)");
        drop(db);
        let mut reopened = open_database(Box::new(io.reopen()));
        assert_eq!(answer(&mut reopened, probe), expected, "{label}");
    }
}
