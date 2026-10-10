//! The writer on the embed facade: at most one logical connection's block stages
//! writes, and every rollback of a block that took it restores the block whole.
//!
//! A `Database` runs one statement at a time on one thread, so where the native server
//! makes a second writer wait this host refuses it `55P03`: another connection's write
//! while a block holds the writer, and another connection's `BEGIN` / `COMMIT` /
//! `ROLLBACK` while a block is open. Reads run — but for one whose plan fails because of a
//! change the writer's block made to the catalog, refused as the server would make it wait
//! — and so does a write beside a block that has only read — the block's own first write
//! then re-bases it on that write. The rollback half is the single-connection property the refusal licenses:
//! with nothing but the block changing the catalog and the store while it holds the
//! writer, its rollback restores unconditionally, and a block that never took the
//! writer restores nothing.

#![allow(clippy::unwrap_used, clippy::panic)]

use pglet::Database;
use pglet_sql::Error;
use pglet_testing::block_io::MemBlockIo;
use pglet_testing::db::open_database;

/// `sql` on `conn`: its last result's rows as text, or `ERR <SQLSTATE>`.
fn answer(db: &mut Database, sql: &str, conn: i32) -> String {
    match db.execute_as(sql, conn) {
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

/// A file-backed database (over an in-memory device) with `t` holding `1`.
fn seeded(io: &MemBlockIo) -> Database {
    let mut db = open_database(Box::new(io.clone()));
    db.execute("CREATE TABLE t (i INT)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();
    db
}

/// The refusal is keyed on the **writer**, not on the block: another connection's write
/// runs beside a block that has only read, and is refused `55P03` once the block has
/// written — without aborting the block. Its reads run throughout. The block's first
/// write re-bases it on the other connection's commit, so it commits over it rather than
/// under it: `1 2 4`, 18.6's final rows.
///
/// Red for a refusal keyed on block ownership (at `BEGIN`), which refuses connection 3's
/// first insert, and red the other way for no
/// refusal, where connection 3's second insert lands inside connection 4's window.
#[test]
fn another_connections_write_is_refused_only_while_a_block_holds_the_writer() {
    let io = MemBlockIo::new();
    let mut db = seeded(&io);
    assert_eq!(answer(&mut db, "BEGIN", 4), "");
    assert_eq!(answer(&mut db, "SELECT count(*) FROM t", 4), "Int8(1)");
    assert_eq!(
        answer(&mut db, "INSERT INTO t VALUES (2)", 3),
        "",
        "a block that has only read holds no writer"
    );
    assert_eq!(answer(&mut db, "INSERT INTO t VALUES (4)", 4), "");
    assert_eq!(
        answer(&mut db, "INSERT INTO t VALUES (5)", 3),
        "ERR 55P03",
        "the block holds the writer from its first write"
    );
    assert_eq!(
        answer(&mut db, "SELECT count(*) FROM t", 3),
        "Int8(2)",
        "a read always runs, and reads what is committed"
    );
    assert_eq!(
        db.transaction_status(),
        pglet_core::TransactionStatus::InTransaction,
        "the refusal aborted nothing of connection 4's block"
    );
    assert_eq!(answer(&mut db, "COMMIT", 4), "");
    assert_eq!(
        answer(&mut db, "SELECT i FROM t ORDER BY i", 0),
        "Int4(1),Int4(2),Int4(4)"
    );
}

/// Another connection's transaction control is refused `55P03` while any block is open —
/// by SQL and through the direct `Database::begin` / `commit` / `rollback(conn_id)` API
/// alike — and the open block is left exactly as it was: its depth, its rows, its
/// commit. Without the gate `Database::rollback(4)` ended connection 3's block and its
/// rows went with it, and connection 4's `BEGIN` opened nothing while its
/// `COMMIT` ended connection 3's block (row 210).
#[test]
fn another_connections_transaction_control_is_refused_while_a_block_is_open() {
    for block_wrote in [true, false] {
        let io = MemBlockIo::new();
        let mut db = seeded(&io);
        assert_eq!(answer(&mut db, "BEGIN", 3), "");
        if block_wrote {
            assert_eq!(answer(&mut db, "INSERT INTO t VALUES (2)", 3), "");
        } else {
            assert_eq!(answer(&mut db, "SELECT count(*) FROM t", 3), "Int8(1)");
        }
        for (label, refused) in [
            ("Database::rollback(4)", db.rollback(4)),
            ("Database::commit(4)", db.commit(4)),
            ("Database::begin(4)", db.begin(4)),
        ] {
            assert!(
                matches!(refused, Err(Error::WriterHeldElsewhere(_))),
                "{label}, block wrote {block_wrote}: {refused:?}"
            );
        }
        for sql in ["BEGIN", "COMMIT", "ROLLBACK"] {
            assert_eq!(
                answer(&mut db, sql, 4),
                "ERR 55P03",
                "{sql} on connection 4, block wrote {block_wrote}"
            );
        }
        assert_eq!(db.transaction_depth(), 1, "block wrote {block_wrote}");
        assert_eq!(
            answer(&mut db, "SELECT count(*) FROM t", 4),
            "Int8(1)",
            "connection 4 reads what is committed"
        );
        assert_eq!(answer(&mut db, "INSERT INTO t VALUES (3)", 3), "");
        assert_eq!(answer(&mut db, "COMMIT", 3), "");
        let committed = if block_wrote {
            "Int4(1),Int4(2),Int4(3)"
        } else {
            "Int4(1),Int4(3)"
        };
        assert_eq!(
            answer(&mut db, "SELECT i FROM t ORDER BY i", 0),
            committed,
            "block wrote {block_wrote}"
        );
    }
}

/// The three backends a rollback runs on: the file store (over an in-memory device, so
/// it reopens), the in-memory B+tree, and memory storage.
fn backends() -> Vec<(&'static str, Database, Option<MemBlockIo>)> {
    let io = MemBlockIo::new();
    vec![
        ("file store", open_database(Box::new(io.clone())), Some(io)),
        ("in-memory B+tree", Database::with_btree(), None),
        ("memory storage", Database::in_memory(), None),
    ]
}

/// Every rollback of a block that took the writer restores unconditionally (the repo's
/// `EDGES.md` row 181). A savepoint- or `DO`-undone table followed by dropping and
/// re-creating a committed table used to refuse the `ROLLBACK` `22000` — the storage's
/// and the catalog's rollback tiers counting different things — and leave the committed
/// table answering nothing, durably. Each script's `ROLLBACK` answers without an error
/// and the committed rows read back, live and on a reopen of the file store. Red with a
/// tier put back (a restore gated on a count of the block's DDL).
#[test]
fn a_rollback_restores_the_block_whole_after_an_undone_table() {
    const SCRIPTS: [(&str, &[&str]); 3] = [
        (
            "ROLLBACK TO undid a table, then the committed one re-created",
            &[
                "SAVEPOINT sp",
                "CREATE TABLE u9 (i INT)",
                "ROLLBACK TO sp",
                "DROP TABLE c",
                "CREATE TABLE c (i INT)",
                "INSERT INTO c VALUES (44)",
            ],
        ),
        (
            "the same, over a primary key",
            &[
                "SAVEPOINT sp",
                "CREATE TABLE u9 (i INT)",
                "ROLLBACK TO sp",
                "DROP TABLE k",
                "CREATE TABLE k (i INT PRIMARY KEY)",
                "INSERT INTO k VALUES (1)",
            ],
        ),
        (
            "a caught DO undid a table",
            &[
                "DO $$ BEGIN CREATE TABLE dq_y (i int); CREATE TABLE dq_dup (i int); \
                 EXCEPTION WHEN duplicate_table THEN NULL; END $$",
                "DROP TABLE c",
                "CREATE TABLE c (i INT)",
                "INSERT INTO c VALUES (44)",
            ],
        ),
    ];
    for (script, block) in SCRIPTS {
        for (backend, mut db, io) in backends() {
            let cell = format!("{script}, {backend}");
            for sql in [
                "CREATE TABLE c (i INT)",
                "INSERT INTO c VALUES (1), (2), (3)",
                "CREATE TABLE k (i INT PRIMARY KEY)",
                "INSERT INTO k VALUES (1), (2), (3)",
                "CREATE TABLE dq_dup (i int)",
            ] {
                db.execute(sql).unwrap();
            }
            db.execute("BEGIN").unwrap();
            for sql in block {
                db.execute(sql)
                    .unwrap_or_else(|e| panic!("{cell}: {sql}: {e}"));
            }
            db.execute("ROLLBACK")
                .unwrap_or_else(|e| panic!("{cell}: the ROLLBACK: {e}"));
            let reads = |db: &mut Database| {
                [
                    answer(db, "SELECT i FROM c ORDER BY i", 0),
                    answer(db, "SELECT i FROM k WHERE i = 2", 0),
                    answer(db, "INSERT INTO k VALUES (2)", 0),
                ]
            };
            let whole = [
                "Int4(1),Int4(2),Int4(3)".to_string(),
                "Int4(2)".to_string(),
                "ERR 23505".to_string(),
            ];
            assert_eq!(reads(&mut db), whole, "{cell}: live");
            drop(db);
            if let Some(io) = io {
                let mut reopened = open_database(Box::new(io.reopen()));
                assert_eq!(reads(&mut reopened), whole, "{cell}: after a reopen");
            }
        }
    }
}

/// A block that never took the writer restores nothing: its frames are older than what
/// another connection committed while it read, and a restore takes that back — measured,
/// with the restore unconditional, on memory storage, whose `BEGIN` clones the whole store: a read-only
/// block's `ROLLBACK` removed the other connection's row, table and sequence (the repo's
/// `EDGES.md` row 102, its read-only half). Asserted on every backend, for `ROLLBACK`,
/// an aborted block's `COMMIT`, and a healthy `COMMIT`. Red for a restore keyed on
/// anything but the writer.
#[test]
fn a_block_that_never_wrote_restores_nothing() {
    for end in ["ROLLBACK", "aborted COMMIT", "COMMIT"] {
        for (backend, mut db, io) in backends() {
            let cell = format!("{end}, {backend}");
            db.execute("CREATE TABLE ro_t (i INT)").unwrap();
            assert_eq!(answer(&mut db, "BEGIN", 4), "");
            assert_eq!(answer(&mut db, "SELECT count(*) FROM ro_t", 4), "Int8(0)");
            for sql in [
                "INSERT INTO ro_t VALUES (1)",
                "CREATE TABLE ro_u (i INT)",
                "CREATE SEQUENCE ro_s",
            ] {
                assert_eq!(answer(&mut db, sql, 3), "", "{cell}: {sql}");
            }
            if end == "aborted COMMIT" {
                assert!(answer(&mut db, "SELECT 1 / 0", 4).starts_with("ERR"));
            }
            let sql = if end == "ROLLBACK" {
                "ROLLBACK"
            } else {
                "COMMIT"
            };
            assert_eq!(answer(&mut db, sql, 4), "", "{cell}");
            let reads = |db: &mut Database| {
                [
                    answer(db, "SELECT count(*) FROM ro_t", 0),
                    answer(db, "SELECT count(*) FROM ro_u", 0),
                ]
            };
            let kept = ["Int8(1)".to_string(), "Int8(0)".to_string()];
            assert_eq!(reads(&mut db), kept, "{cell}: live");
            assert_eq!(
                answer(&mut db, "SELECT nextval('ro_s')", 0),
                "Int8(1)",
                "{cell}"
            );
            drop(db);
            if let Some(io) = io {
                let mut reopened = open_database(Box::new(io.reopen()));
                assert_eq!(reads(&mut reopened), kept, "{cell}: after a reopen");
            }
        }
    }
}

/// A block that took the writer rolls back to the state it took the writer on, not to its
/// `BEGIN`: what another connection committed between the two stands, on every backend —
/// the whole block's `ROLLBACK`, and a `ROLLBACK TO` a savepoint taken before the first
/// write. Red without the re-base at acquisition: memory storage's `BEGIN`-time store
/// clone restores over the other connection's row, and the B+tree's `BEGIN`-time name map
/// and catalog frames take back its table and sequence.
#[test]
fn a_rollback_keeps_what_committed_before_the_blocks_first_write() {
    for to_savepoint in [false, true] {
        for (backend, mut db, io) in backends() {
            let cell = format!("ROLLBACK TO {to_savepoint}, {backend}");
            db.execute("CREATE TABLE t (i INT)").unwrap();
            db.execute("INSERT INTO t VALUES (1)").unwrap();
            assert_eq!(answer(&mut db, "BEGIN", 4), "");
            if to_savepoint {
                assert_eq!(answer(&mut db, "SAVEPOINT sp", 4), "");
            }
            for sql in [
                "INSERT INTO t VALUES (2)",
                "CREATE TABLE u (i INT)",
                "CREATE SEQUENCE s",
            ] {
                assert_eq!(answer(&mut db, sql, 3), "", "{cell}: {sql}");
            }
            assert_eq!(answer(&mut db, "INSERT INTO t VALUES (4)", 4), "", "{cell}");
            if to_savepoint {
                assert_eq!(answer(&mut db, "ROLLBACK TO sp", 4), "", "{cell}");
                assert_eq!(
                    answer(&mut db, "SELECT i FROM t ORDER BY i", 4),
                    "Int4(1),Int4(2)",
                    "{cell}: inside the block after the ROLLBACK TO"
                );
            }
            assert_eq!(answer(&mut db, "ROLLBACK", 4), "", "{cell}");
            let reads = |db: &mut Database| {
                [
                    answer(db, "SELECT i FROM t ORDER BY i", 0),
                    answer(db, "SELECT count(*) FROM u", 0),
                ]
            };
            let kept = ["Int4(1),Int4(2)".to_string(), "Int8(0)".to_string()];
            assert_eq!(reads(&mut db), kept, "{cell}: live");
            assert_eq!(
                answer(&mut db, "SELECT nextval('s')", 0),
                "Int8(1)",
                "{cell}"
            );
            drop(db);
            if let Some(io) = io {
                let mut reopened = open_database(Box::new(io.reopen()));
                assert_eq!(reads(&mut reopened), kept, "{cell}: after a reopen");
            }
        }
    }
}

/// `Database::copy_in` is a door that stages writes, so the writer decides it like any
/// statement: refused `55P03` while another connection's block holds the writer, and, as
/// its own block's first write, the point that block is re-based — so the block's
/// rollback keeps what another connection committed before it. Red for a `copy_in` that
/// does not claim the writer: the ingest lands inside the other block's window, and on
/// memory storage the rollback restores the `BEGIN`-time store over the committed row.
#[test]
fn copy_in_is_a_write_the_writer_decides() {
    for (backend, mut db, _) in backends() {
        db.execute("CREATE TABLE t (i INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(answer(&mut db, "BEGIN", 3), "");
        assert_eq!(answer(&mut db, "INSERT INTO t VALUES (3)", 3), "");
        let refused = db.copy_in("COPY t FROM STDIN", b"9\n");
        assert!(
            matches!(refused, Err(Error::WriterHeldElsewhere(_))),
            "{backend}: {refused:?}"
        );
        assert_eq!(answer(&mut db, "ROLLBACK", 3), "", "{backend}");

        assert_eq!(answer(&mut db, "BEGIN", 0), "");
        assert_eq!(
            answer(&mut db, "INSERT INTO t VALUES (2)", 3),
            "",
            "{backend}"
        );
        assert_eq!(
            db.copy_in("COPY t FROM STDIN", b"4\n").ok(),
            Some(1),
            "{backend}"
        );
        assert_eq!(answer(&mut db, "ROLLBACK", 0), "", "{backend}");
        assert_eq!(
            answer(&mut db, "SELECT i FROM t ORDER BY i", 0),
            "Int4(1),Int4(2)",
            "{backend}"
        );
    }
}

/// The host's own doors to the catalog and the store — `Database::schema_mut` and
/// `Database::storage_mut` — are the open block's, so each takes the writer for it: another
/// connection's write is refused `55P03` after either, and the block's rollback keeps
/// what that connection committed before. Red for a door that does not take the writer.
#[test]
fn the_hosts_mutable_doors_take_the_writer_for_the_open_block() {
    for door in ["schema_mut", "storage_mut"] {
        for (backend, mut db, _) in backends() {
            let cell = format!("{door}, {backend}");
            db.execute("CREATE TABLE t (i INT)").unwrap();
            db.execute("INSERT INTO t VALUES (1)").unwrap();
            assert_eq!(answer(&mut db, "BEGIN", 0), "");
            assert_eq!(answer(&mut db, "INSERT INTO t VALUES (2)", 3), "", "{cell}");
            if door == "schema_mut" {
                let _ = db.schema_mut();
            } else {
                let _ = db.storage_mut();
            }
            assert_eq!(
                answer(&mut db, "INSERT INTO t VALUES (3)", 3),
                "ERR 55P03",
                "{cell}"
            );
            assert_eq!(answer(&mut db, "ROLLBACK", 0), "", "{cell}");
            assert_eq!(
                answer(&mut db, "SELECT i FROM t ORDER BY i", 0),
                "Int4(1),Int4(2)",
                "{cell}"
            );
        }
    }
}

/// On the embed facade, a block creates a table, another connection's writes into
/// it and beside it are refused `55P03` (the block holds the writer), and the block's
/// `ROLLBACK` answers without an error — so the database reopens, with the committed
/// rows and none of the block. Without the writer, the other connection's writes
/// landed, the `ROLLBACK` refused `22000`, and with a primary key the store never
/// reopened (`re-register index t2_pkey: relation "t2" does not exist`).
#[test]
fn a_block_beside_another_connection_rolls_back_and_reopens() {
    for pk in [true, false] {
        let cell = format!("primary key {pk}");
        let io = MemBlockIo::new();
        let mut db = open_database(Box::new(io.clone()));
        for sql in [
            "CREATE TABLE g (k INT)",
            "CREATE TABLE t (i INT)",
            "INSERT INTO t VALUES (1), (2), (3)",
        ] {
            db.execute(sql).unwrap();
        }
        let ty = if pk { "INT PRIMARY KEY" } else { "INT" };
        assert_eq!(answer(&mut db, "BEGIN", 0), "");
        assert_eq!(answer(&mut db, &format!("CREATE TABLE t2 (i {ty})"), 0), "");
        assert_eq!(
            answer(&mut db, "INSERT INTO t2 VALUES (42)", 7),
            "ERR 55P03",
            "{cell}"
        );
        assert_eq!(
            answer(&mut db, "CREATE INDEX g_k ON g (k)", 7),
            "ERR 55P03",
            "{cell}"
        );
        assert_eq!(answer(&mut db, "ROLLBACK", 0), "", "{cell}");
        assert_eq!(
            answer(&mut db, "SELECT i FROM t2", 0),
            "ERR 42P01",
            "{cell}"
        );
        assert_eq!(
            answer(&mut db, "CREATE INDEX g_k ON g (k)", 7),
            "",
            "{cell}"
        );
        assert_eq!(answer(&mut db, "INSERT INTO g VALUES (5)", 7), "", "{cell}");
        drop(db);
        let mut reopened = open_database(Box::new(io.reopen()));
        assert_eq!(
            answer(&mut reopened, "SELECT i FROM t ORDER BY i", 0),
            "Int4(1),Int4(2),Int4(3)",
            "{cell}"
        );
        assert_eq!(
            answer(&mut reopened, "SELECT k FROM g", 0),
            "Int4(5)",
            "{cell}"
        );
        assert_eq!(
            answer(&mut reopened, "SELECT i FROM t2", 0),
            "ERR 42P01",
            "{cell}"
        );
    }
}

/// `lock_timeout` on the embed facade: `SHOW` answers pglet's default `1s`, a `SET`
/// and a `SET LOCAL` move it — the latter to the end of the block — `RESET` restores the
/// host's default, and `Database::set_default_lock_timeout` moves that default. A
/// value its type refuses is `22023`. (This host never waits, so the value bounds
/// nothing here; what it answers is what a client reads.)
#[test]
fn lock_timeout_is_a_session_setting() {
    let mut db = Database::with_btree();
    let show = |db: &mut Database| answer(db, "SHOW lock_timeout", 0);
    assert_eq!(show(&mut db), r#"Text("1s")"#);
    assert_eq!(answer(&mut db, "SET lock_timeout = '1500ms'", 0), "");
    assert_eq!(show(&mut db), r#"Text("1500ms")"#);
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(answer(&mut db, "SET LOCAL lock_timeout TO '2min'", 0), "");
    assert_eq!(show(&mut db), r#"Text("2min")"#);
    assert_eq!(answer(&mut db, "COMMIT", 0), "");
    assert_eq!(
        show(&mut db),
        r#"Text("1500ms")"#,
        "SET LOCAL ends with its block"
    );
    assert_eq!(
        answer(&mut db, "SET LOCAL lock_timeout = 5", 0),
        "",
        "outside a block SET LOCAL is accepted and changes nothing"
    );
    assert_eq!(show(&mut db), r#"Text("1500ms")"#);
    assert_eq!(answer(&mut db, "SET lock_timeout = 0", 0), "");
    assert_eq!(show(&mut db), r#"Text("0")"#);
    assert_eq!(answer(&mut db, "RESET lock_timeout", 0), "");
    assert_eq!(show(&mut db), r#"Text("1s")"#);
    for bad in [
        "SET lock_timeout = 'soon'",
        "SET lock_timeout = '5 fortnights'",
        "SET lock_timeout = -1",
    ] {
        assert_eq!(answer(&mut db, bad, 0), "ERR 22023", "{bad}");
    }
    db.set_default_lock_timeout(250);
    assert_eq!(show(&mut db), r#"Text("250ms")"#);
    assert_eq!(answer(&mut db, "SET lock_timeout = '1h'", 0), "");
    assert_eq!(answer(&mut db, "RESET ALL", 0), "");
    assert_eq!(
        show(&mut db),
        r#"Text("250ms")"#,
        "RESET restores the host's default"
    );
}

/// A rollback's counter save writes nothing while the live catalog holds a change no save
/// wrote beside the counters: a host's change through `Database::schema_mut` outside any
/// block, then a block that draws from a sequence and rolls back. A save writes the store's
/// current tree beside the catalog, and a structure no save has written may have a footprint
/// there the durable catalog does not account for — so the counters wait for the next save
/// that writes the live catalog, and a reopen before it holds neither the host's change nor
/// the draw (`EDGES.md` row 214's shape). Red with the structure check in
/// `TransactionSession::counter_save` removed: the rollback's save writes the live catalog,
/// and the reopen holds the host's sequence.
#[test]
fn a_rollbacks_counter_save_never_writes_a_host_change_no_save_wrote() {
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    db.execute("CREATE SEQUENCE s").unwrap();
    assert_eq!(answer(&mut db, "SELECT nextval('s')", 0), "Int8(1)");
    db.schema_mut()
        .sequences
        .insert("host_seq".into(), pglet_sql::catalog::Sequence::new(1));
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(answer(&mut db, "SELECT nextval('s')", 0), "Int8(2)");
    assert_eq!(answer(&mut db, "ROLLBACK", 0), "");
    assert!(
        db.schema().sequences.contains_key("host_seq"),
        "the host's change stands live"
    );
    drop(db);
    let mut reopened = open_database(Box::new(io.reopen()));
    assert_eq!(
        answer(&mut reopened, "SELECT nextval('host_seq')", 0),
        "ERR 42P01",
        "no save wrote the host's change"
    );
    assert_eq!(
        answer(&mut reopened, "SELECT nextval('s')", 0),
        "Int8(2)",
        "and the rollback's counter save did not either: the draw is handed out again"
    );
}

/// A `REPEATABLE READ` or `SERIALIZABLE` block keeps the snapshot it began with, and its
/// first write is refused `40001` when the store moved past that snapshot — here another logical
/// connection's insert beside the block's reads — which aborts the block (`25P02` until
/// `ROLLBACK`) and leaves the other connection's commit. With nothing committed since, the
/// same block writes and commits (the accept side), and at `READ COMMITTED` it re-bases and
/// writes over the commit, as before. Every route that sets the level reaches it: `BEGIN …
/// ISOLATION LEVEL`, `SET TRANSACTION`, `SET SESSION CHARACTERISTICS`, and
/// `default_transaction_isolation`; `SHOW transaction_isolation` answers the block's level.
///
/// Red with the check removed from `TransactionSession::take_writer`: every refused cell
/// writes and commits (S13RR's lost update, 18.6's `40001`).
#[test]
fn a_repeatable_read_block_is_refused_40001_when_anything_committed_since_it_began() {
    type Route = (&'static str, &'static [&'static str], &'static str);
    let routes: [Route; 5] = [
        (
            "BEGIN … REPEATABLE READ",
            &["BEGIN ISOLATION LEVEL REPEATABLE READ"],
            "repeatable read",
        ),
        (
            "START … SERIALIZABLE",
            &["START TRANSACTION ISOLATION LEVEL SERIALIZABLE"],
            "serializable",
        ),
        (
            "SET TRANSACTION",
            &["BEGIN", "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"],
            "repeatable read",
        ),
        (
            "SET SESSION CHARACTERISTICS",
            &[
                "SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL SERIALIZABLE",
                "BEGIN",
            ],
            "serializable",
        ),
        (
            "default_transaction_isolation",
            &[
                "SET default_transaction_isolation = 'repeatable read'",
                "BEGIN",
            ],
            "repeatable read",
        ),
    ];
    for (route, begin, level) in routes {
        for concurrent in [true, false] {
            let cell = format!("{route}, a concurrent commit {concurrent}");
            let mut db = Database::with_btree();
            db.execute("CREATE TABLE v (id INT PRIMARY KEY, c INT)")
                .unwrap();
            db.execute("CREATE TABLE other (i INT)").unwrap();
            db.execute("INSERT INTO v VALUES (1, 0)").unwrap();
            for sql in begin {
                assert_eq!(answer(&mut db, sql, 4), "", "{cell}: {sql}");
            }
            assert_eq!(
                answer(&mut db, "SHOW transaction_isolation", 4),
                format!("Text({level:?})"),
                "{cell}"
            );
            assert_eq!(answer(&mut db, "SELECT c FROM v", 4), "Int4(0)", "{cell}");
            if concurrent {
                assert_eq!(
                    answer(&mut db, "INSERT INTO other VALUES (1)", 3),
                    "",
                    "{cell}"
                );
                assert_eq!(
                    answer(&mut db, "UPDATE v SET c = c + 50", 4),
                    "ERR 40001",
                    "{cell}"
                );
                assert_eq!(answer(&mut db, "SELECT 1", 4), "ERR 25P02", "{cell}");
                assert_eq!(answer(&mut db, "ROLLBACK", 4), "", "{cell}");
                assert_eq!(answer(&mut db, "SELECT c FROM v", 3), "Int4(0)", "{cell}");
                assert_eq!(
                    answer(&mut db, "SELECT count(*) FROM other", 3),
                    "Int8(1)",
                    "{cell}"
                );
            } else {
                assert_eq!(answer(&mut db, "UPDATE v SET c = c + 50", 4), "", "{cell}");
                assert_eq!(answer(&mut db, "COMMIT", 4), "", "{cell}");
                assert_eq!(answer(&mut db, "SELECT c FROM v", 3), "Int4(50)", "{cell}");
            }
        }
    }
    // READ COMMITTED is unchanged: the block's first write re-bases over the commit.
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE v (id INT PRIMARY KEY, c INT)")
        .unwrap();
    db.execute("INSERT INTO v VALUES (1, 0)").unwrap();
    assert_eq!(
        answer(&mut db, "BEGIN ISOLATION LEVEL READ COMMITTED", 4),
        ""
    );
    assert_eq!(answer(&mut db, "SELECT c FROM v", 4), "Int4(0)");
    assert_eq!(answer(&mut db, "UPDATE v SET c = 5", 3), "");
    assert_eq!(answer(&mut db, "UPDATE v SET c = c + 50", 4), "");
    assert_eq!(answer(&mut db, "COMMIT", 4), "");
    assert_eq!(answer(&mut db, "SELECT c FROM v", 3), "Int4(55)");
}

/// The settings pglet acts on are transactional, as 18.6's GUCs are: a `SET` inside a
/// block is taken back by the block's `ROLLBACK` (and by the `COMMIT` of an aborted
/// block, which rolls it back) and kept by its `COMMIT`; a `SET` or `SET LOCAL` after a
/// savepoint is taken back by a `ROLLBACK TO` it and kept by a `RELEASE`; and a `SET
/// TRANSACTION ISOLATION LEVEL` is refused `25001` once the block has read or inside a
/// savepoint. A list is `22023` (`SET lock_timeout takes only one argument`), and
/// `transaction_isolation` cannot be reset (`0A000`).
#[test]
fn the_settings_pglet_acts_on_are_transactional() {
    let mut db = Database::with_btree();
    let show = |db: &mut Database, name: &str| answer(db, &format!("SHOW {name}"), 0);
    let text = |s: &str| format!("Text({s:?})");
    for (end, kept) in [("ROLLBACK", "1s"), ("COMMIT", "7s")] {
        db.execute("RESET ALL").unwrap();
        assert_eq!(answer(&mut db, "BEGIN", 0), "");
        assert_eq!(answer(&mut db, "SET lock_timeout = '7s'", 0), "");
        assert_eq!(
            answer(
                &mut db,
                "SET default_transaction_isolation = serializable",
                0
            ),
            ""
        );
        assert_eq!(answer(&mut db, end, 0), "");
        assert_eq!(show(&mut db, "lock_timeout"), text(kept), "{end}");
        assert_eq!(
            show(&mut db, "default_transaction_isolation"),
            text(if end == "COMMIT" {
                "serializable"
            } else {
                "read committed"
            }),
            "{end}"
        );
    }
    db.execute("RESET ALL").unwrap();
    // An aborted block's COMMIT is a rollback.
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(answer(&mut db, "SET lock_timeout = '7s'", 0), "");
    assert_eq!(answer(&mut db, "SELECT 1 / 0", 0), "ERR 22012");
    assert_eq!(answer(&mut db, "COMMIT", 0), "");
    assert_eq!(show(&mut db, "lock_timeout"), text("1s"));
    // Savepoints.
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(answer(&mut db, "SET LOCAL lock_timeout = '8s'", 0), "");
    assert_eq!(answer(&mut db, "SAVEPOINT a", 0), "");
    assert_eq!(answer(&mut db, "SET LOCAL lock_timeout = '9s'", 0), "");
    assert_eq!(
        answer(
            &mut db,
            "SET default_transaction_isolation = 'serializable'",
            0
        ),
        ""
    );
    assert_eq!(answer(&mut db, "ROLLBACK TO a", 0), "");
    assert_eq!(show(&mut db, "lock_timeout"), text("8s"));
    assert_eq!(
        show(&mut db, "default_transaction_isolation"),
        text("read committed")
    );
    assert_eq!(answer(&mut db, "SET lock_timeout = '3s'", 0), "");
    assert_eq!(answer(&mut db, "RELEASE a", 0), "");
    assert_eq!(answer(&mut db, "COMMIT", 0), "");
    assert_eq!(
        show(&mut db, "lock_timeout"),
        text("3s"),
        "released, then committed"
    );
    // The isolation level is fixed once the block reads, and inside a savepoint.
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(
        answer(&mut db, "SET TRANSACTION ISOLATION LEVEL READ COMMITTED", 0),
        ""
    );
    assert_eq!(answer(&mut db, "SAVEPOINT s", 0), "");
    assert_eq!(
        answer(&mut db, "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE", 0),
        "ERR 25001"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 0), "");
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(answer(&mut db, "SELECT 1", 0), "Int4(1)");
    assert_eq!(
        answer(
            &mut db,
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            0
        ),
        "ERR 25001"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 0), "");
    // The GUC spelling of `SET TRANSACTION`, and a nested `BEGIN` naming a level — both set
    // the open block's, under the same rule.
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(
        answer(&mut db, "SET transaction_isolation = 'serializable'", 0),
        ""
    );
    assert_eq!(show(&mut db, "transaction_isolation"), text("serializable"));
    assert_eq!(answer(&mut db, "SELECT 1", 0), "Int4(1)");
    assert_eq!(
        answer(&mut db, "BEGIN ISOLATION LEVEL READ COMMITTED", 0),
        "ERR 25001"
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 0), "");
    assert_eq!(
        answer(
            &mut db,
            "SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            0
        ),
        ""
    );
    assert_eq!(
        show(&mut db, "default_transaction_isolation"),
        text("repeatable read")
    );
    assert_eq!(
        show(&mut db, "transaction_isolation"),
        text("repeatable read")
    );
    assert_eq!(answer(&mut db, "RESET ALL", 0), "");
    assert_eq!(
        show(&mut db, "transaction_isolation"),
        text("read committed")
    );
    for (sql, code) in [
        ("SET lock_timeout = 1, 2", "22023"),
        ("SET default_transaction_isolation = 'x'", "22023"),
        ("SET default_transaction_isolation = 'a', 'b'", "22023"),
        ("RESET transaction_isolation", "0A000"),
        (
            "SELECT set_config('transaction_isolation', 'serializable', true)",
            "0A000",
        ),
    ] {
        assert_eq!(answer(&mut db, sql, 0), format!("ERR {code}"), "{sql}");
    }
}

/// A plan another connection fails against a catalog the writer's block has changed is
/// refused `55P03`, as that block's writes are — the native server makes it wait for the
/// block and plan again, and this host cannot wait — rather than answered from the
/// block's uncommitted change: connection 3's `UPDATE` of a column connection 4's block
/// dropped. Before the block changes the catalog, and for a syntax error, the plan's own
/// error stands. After the block's `ROLLBACK` the same `UPDATE` runs. (A write to a table
/// the block never touched is refused as well: the refusal asks only whether the block holds
/// the writer — `EDGES.md` row 229.) Red with the plan's
/// failure answered as it stands: `42703`, although the column is back after the rollback.
#[test]
fn a_plan_failing_against_the_writers_uncommitted_catalog_is_refused_55p03() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE d (id INT, c INT)").unwrap();
    db.execute("CREATE TABLE other (i INT)").unwrap();
    db.execute("INSERT INTO d VALUES (1, 1)").unwrap();
    assert_eq!(answer(&mut db, "BEGIN", 4), "");
    assert_eq!(answer(&mut db, "INSERT INTO d VALUES (2, 2)", 4), "");
    assert_eq!(
        answer(&mut db, "INSERT INTO other VALUES (1)", 3),
        "ERR 55P03",
        "a table the block never touched is refused as its own is"
    );
    assert_eq!(
        answer(&mut db, "UPDATE d SET nosuch = 1", 3),
        "ERR 42703",
        "the block has changed no catalog"
    );
    assert_eq!(answer(&mut db, "ALTER TABLE d DROP COLUMN c", 4), "");
    assert_eq!(answer(&mut db, "UPDATE d SET c = 9", 3), "ERR 55P03");
    assert_eq!(answer(&mut db, "SELECT c FROM d", 3), "ERR 55P03");
    assert_eq!(answer(&mut db, "UPDATE d SET = 9", 3), "ERR 42601");
    // A `42601` the catalog decides — a set operation over a `SELECT *` of the table,
    // two columns against the committed catalog and one against the block's — is refused
    // as the block's too.
    let set_op = "SELECT * FROM d UNION ALL SELECT 7, 8 ORDER BY 1";
    assert_eq!(answer(&mut db, set_op, 3), "ERR 55P03");
    assert_eq!(answer(&mut db, "ROLLBACK", 4), "");
    assert_eq!(answer(&mut db, "UPDATE d SET c = 9", 3), "");
    assert_eq!(answer(&mut db, "SELECT c FROM d", 3), "Int4(9)");
    assert_eq!(
        answer(&mut db, set_op, 3),
        "Int4(1)|Int4(9),Int4(7)|Int4(8)"
    );
}

/// A `COPY`'s column list is resolved as a plan is: naming a column another connection's
/// block dropped, `copy_out` and `copy_in` are refused `55P03` — the native server makes
/// them wait for the block — rather than answered `42703` from the block's uncommitted
/// catalog. A column no catalog has is the statement's own `42703`, in either direction.
/// After the block's `ROLLBACK` both run. Red, for `copy_out`, with the resolution left
/// to execution: `42703` although the column is back after the rollback (`copy_in`'s
/// refusal would still come from the writer, which the block holds).
#[test]
fn a_copy_column_list_failing_against_the_writers_uncommitted_catalog_is_refused_55p03() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE cd (a INT, c INT)").unwrap();
    db.execute("INSERT INTO cd VALUES (1, 7)").unwrap();
    assert_eq!(answer(&mut db, "BEGIN", 4), "");
    assert_eq!(answer(&mut db, "ALTER TABLE cd DROP COLUMN c", 4), "");
    let code = |r: Result<u64, Error>| r.map_err(|e| e.sqlstate().to_string());
    assert_eq!(
        db.copy_out("COPY cd (c) TO STDOUT")
            .map_err(|e| e.sqlstate().to_string()),
        Err("55P03".to_string())
    );
    assert_eq!(
        code(db.copy_in("COPY cd (c) FROM STDIN", b"9\n")),
        Err("55P03".to_string())
    );
    assert_eq!(
        db.copy_out("COPY cd (nosuch) TO STDOUT")
            .map_err(|e| e.sqlstate().to_string()),
        Err("42703".to_string())
    );
    assert_eq!(
        code(db.copy_in("COPY cd (nosuch) FROM STDIN", b"9\n")),
        Err("42703".to_string())
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 4), "");
    assert_eq!(db.copy_out("COPY cd (c) TO STDOUT").unwrap(), b"7\n");
    assert_eq!(code(db.copy_in("COPY cd (c) FROM STDIN", b"9\n")), Ok(1));
    assert_eq!(
        answer(&mut db, "SELECT a, c FROM cd ORDER BY c", 0),
        "Int4(1)|Int4(7),Null|Int4(9)"
    );
}

/// A plan that fails for its own reason is the statement's answer on this host too,
/// whatever another connection's block has done to the catalog: behind a block that
/// created a table none of these statements names, a column no catalog has, a cast no
/// catalog decides and a function no catalog has answer what they answer with no block
/// open — not `55P03`. The failure the block caused is still refused
/// (`a_plan_failing_against_the_writers_uncommitted_catalog_is_refused_55p03`). Red with
/// every non-syntax plan failure behind a changed catalog taken as the block's.
#[test]
fn a_plans_own_failure_is_answered_behind_another_blocks_catalog_change() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE base (a INT)").unwrap();
    db.execute("INSERT INTO base VALUES (1)").unwrap();
    let statements = [
        "SELECT nosuch FROM base",
        "SELECT 'abc'::int",
        "SELECT no_such_fn(1)",
        "UPDATE base SET nosuch = 1",
    ];
    let own: Vec<String> = statements
        .iter()
        .map(|sql| answer(&mut db, sql, 3))
        .collect();
    assert!(own.iter().all(|a| a.starts_with("ERR ")), "{own:?}");
    assert_eq!(answer(&mut db, "BEGIN", 4), "");
    assert_eq!(answer(&mut db, "CREATE TABLE unrel (x INT)", 4), "");
    for (sql, want) in statements.iter().zip(&own) {
        assert_eq!(&answer(&mut db, sql, 3), want, "{sql}");
    }
    assert_eq!(answer(&mut db, "SELECT a FROM base", 3), "Int4(1)");
    assert_eq!(answer(&mut db, "ROLLBACK", 4), "");
}

/// A `REPEATABLE READ` block's snapshot is taken by its first statement, not by `BEGIN`
/// (18.6's rule), so a block whose first statement is its write takes in what another
/// connection committed between them rather than being refused: `50`, 18.6's answer.
/// Red with the check measured from `BEGIN`: `40001`.
#[test]
fn a_blocks_first_statement_write_takes_its_snapshot() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE v (id INT PRIMARY KEY, c INT)")
        .unwrap();
    db.execute("INSERT INTO v VALUES (1, 0)").unwrap();
    assert_eq!(
        answer(&mut db, "BEGIN ISOLATION LEVEL REPEATABLE READ", 4),
        ""
    );
    assert_eq!(answer(&mut db, "UPDATE v SET c = 5", 3), "");
    assert_eq!(answer(&mut db, "UPDATE v SET c = c * 10", 4), "");
    assert_eq!(answer(&mut db, "COMMIT", 4), "");
    assert_eq!(answer(&mut db, "SELECT c FROM v", 3), "Int4(50)");
}

/// Each logical connection's settings are its own, as each socket's are on the native
/// server: another connection's `SET TRANSACTION` outside a block of its own changes
/// nothing (18.6's warning, no error) — neither `25001` nor the open block's level — and
/// its session `SET`s are not taken back by the open block's `ROLLBACK`. Red with one
/// set of settings shared by every connection: the `SET TRANSACTION` is `25001` after the
/// block read, flips the block to `SERIALIZABLE` before it, and the `ROLLBACK` puts back
/// `1s` / `read committed`.
#[test]
fn each_connections_settings_are_its_own() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (i INT)").unwrap();
    let text = |s: &str| format!("Text({s:?})");
    // Another connection's `SET TRANSACTION` beside a block that read.
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(answer(&mut db, "SELECT count(*) FROM t", 0), "Int8(0)");
    assert_eq!(
        answer(&mut db, "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE", 5),
        ""
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 0), "");
    // … and beside one that has not: the block's level is its own.
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(
        answer(&mut db, "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE", 5),
        ""
    );
    assert_eq!(
        answer(&mut db, "SHOW transaction_isolation", 0),
        text("read committed")
    );
    assert_eq!(answer(&mut db, "INSERT INTO t VALUES (1)", 5), "");
    assert_eq!(answer(&mut db, "INSERT INTO t VALUES (2)", 0), "");
    assert_eq!(answer(&mut db, "COMMIT", 0), "");
    // Another connection's session settings outlive the block's rollback.
    assert_eq!(answer(&mut db, "BEGIN", 0), "");
    assert_eq!(answer(&mut db, "SET lock_timeout = '9s'", 5), "");
    assert_eq!(
        answer(
            &mut db,
            "SET default_transaction_isolation = 'serializable'",
            5
        ),
        ""
    );
    assert_eq!(answer(&mut db, "ROLLBACK", 0), "");
    assert_eq!(answer(&mut db, "SHOW lock_timeout", 5), text("9s"));
    assert_eq!(
        answer(&mut db, "SHOW default_transaction_isolation", 5),
        text("serializable")
    );
    assert_eq!(answer(&mut db, "SHOW lock_timeout", 0), text("1s"));
}

/// A host's `set_default_lock_timeout` is a host setting, not a `SET`: a connection's
/// block that began before it and set its own value is rolled back to the host's value,
/// not to the one the block began with, and a `fork` of the database starts from it.
/// Red with the block's levels left as they began (`1s` after the `ROLLBACK`), and with
/// a fork's settings built from the defaults (`1s` on the fork).
#[test]
fn a_hosts_default_lock_timeout_outlives_a_rollback_and_carries_to_a_fork() {
    let mut db = Database::with_btree();
    let text = |s: &str| format!("Text({s:?})");
    assert_eq!(answer(&mut db, "BEGIN", 6), "");
    assert_eq!(answer(&mut db, "SET lock_timeout = '9s'", 6), "");
    db.set_default_lock_timeout(5000);
    assert_eq!(answer(&mut db, "SHOW lock_timeout", 6), text("5s"));
    assert_eq!(answer(&mut db, "ROLLBACK", 6), "");
    assert_eq!(answer(&mut db, "SHOW lock_timeout", 6), text("5s"));
    assert_eq!(answer(&mut db, "SHOW lock_timeout", 7), text("5s"));
    let mut fork = db.fork().unwrap();
    assert_eq!(answer(&mut fork, "SHOW lock_timeout", 0), text("5s"));
    assert_eq!(answer(&mut fork, "SET lock_timeout = '2s'", 0), "");
    assert_eq!(answer(&mut fork, "RESET lock_timeout", 0), "");
    assert_eq!(answer(&mut fork, "SHOW lock_timeout", 0), text("5s"));
}

/// `pg_advisory_xact_lock` takes the writer as a write does, on every backend: a block's
/// call holds it, so another logical connection's call — and its write — is refused
/// `55P03` without aborting the block, and admitted once the block ends, at `COMMIT` or at
/// `ROLLBACK`. A connection's own call answers `void` (the empty text) inside its block
/// and in autocommit, where it holds nothing past its statement.
///
/// Red for a lock that takes no writer (connection 3's call goes through) and for one
/// that outlives its block (the call after the end is refused).
#[test]
fn an_advisory_xact_lock_is_the_writer() {
    for (backend, mut db, _) in backends() {
        db.execute("CREATE TABLE t (i INT)").unwrap();
        let lock = "SELECT pg_advisory_xact_lock(7)";
        assert_eq!(
            answer(&mut db, lock, 3),
            "Text(\"\")",
            "{backend}: autocommit"
        );
        assert_eq!(
            answer(&mut db, "INSERT INTO t VALUES (1)", 4),
            "",
            "{backend}: an autocommit call holds nothing past its statement"
        );
        for end in ["COMMIT", "ROLLBACK"] {
            let cell = format!("{backend}, {end}");
            assert_eq!(answer(&mut db, "BEGIN", 0), "", "{cell}");
            assert_eq!(answer(&mut db, lock, 0), "Text(\"\")", "{cell}");
            assert_eq!(answer(&mut db, lock, 0), "Text(\"\")", "{cell}: again");
            assert_eq!(answer(&mut db, lock, 3), "ERR 55P03", "{cell}");
            assert_eq!(
                answer(&mut db, "INSERT INTO t VALUES (2)", 3),
                "ERR 55P03",
                "{cell}: the lock is the writer"
            );
            assert_eq!(
                answer(&mut db, "SELECT count(*) FROM t", 0),
                "Int8(1)",
                "{cell}: the block is not aborted"
            );
            assert_eq!(answer(&mut db, end, 0), "", "{cell}");
            assert_eq!(answer(&mut db, lock, 3), "Text(\"\")", "{cell}: released");
        }
        assert_eq!(
            answer(&mut db, "SELECT pg_advisory_xact_lock(NULL)", 0),
            "Null",
            "{backend}: strict"
        );
    }
}
