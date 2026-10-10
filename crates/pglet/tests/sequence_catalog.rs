//! The sequence as a catalog **record** — its `start` and its `owner` — end to end
//! over the embed engine, plus the file-backed reopen that the `PGMG` store bump
//! exists for.
//!
//! The differential fixtures under `tests/fixtures/sequences/` assert the SQL-visible
//! behaviour byte-for-byte against a live Postgres. Two things they structurally
//! cannot reach live here instead:
//!
//! - **durability.** The fixture harness talks to one long-lived server, so nothing
//!   there closes and reopens a store. A persisted field the magic bump did not
//!   actually carry would pass every fixture and lose the fact on the next restart.
//! - **the autocommit bracket.** Every fixture runs inside `BEGIN … ROLLBACK`, which
//!   restores the catalog snapshot — exactly the thing that makes a mutate-then-fail
//!   statement invisible. See `a_failed_set_default_leaves_no_phantom_sequence` in
//!   `pglet`'s own `lib.rs` tests for that half.

// A test harness: `.unwrap()`/`.expect()` surface a setup failure loudly here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::catalog::SequenceOwner;
use pglet_sql::types::Value;

/// Read a single scalar cell from a one-statement query.
fn scalar(db: &mut Database, sql: &str) -> Value {
    let mut results = db.execute(sql).expect("query");
    let result = results.remove(0);
    result.rows[0][0].clone()
}

/// `start` and `owner` survive a close/reopen of a file-backed store.
///
/// This is the only guard on the `PGMG` catalog shape actually carrying the two new
/// fields: an in-memory `Database` never serializes the catalog at all, so a
/// `Sequence` whose extra fields were silently dropped on the way to `SYS_CATALOG`
/// would pass every other test in the tree.
#[test]
fn sequence_start_and_owner_survive_reopen() {
    let dir = ScratchDir::new("sequence_catalog_reopen");

    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE owner_tbl (a INT)").unwrap();
        db.execute("CREATE SEQUENCE s START WITH 7").unwrap();
        db.execute("ALTER SEQUENCE s OWNED BY owner_tbl.a").unwrap();
        // Advance it, so `last_value` and `start` are visibly different numbers and a
        // reopen that reconstructed one from the other would be caught.
        assert_eq!(scalar(&mut db, "SELECT nextval('s')"), Value::Int8(7));
        assert_eq!(scalar(&mut db, "SELECT nextval('s')"), Value::Int8(8));
    }

    let mut db = Database::open(&dir.path).expect("reopen");
    let sequence = db
        .schema()
        .sequences
        .get("s")
        .expect("sequence after reopen");
    assert_eq!(
        sequence.start, 7,
        "START WITH survives the store round-trip"
    );
    assert_eq!(sequence.last_value, 8, "the counter survives too");
    assert_eq!(
        sequence.owner,
        Some(SequenceOwner {
            table: "owner_tbl".to_string(),
            column: "a".to_string(),
        }),
        "OWNED BY survives the store round-trip"
    );

    // ...and the restored `start` is *used*: a bare RESTART after the reopen returns
    // to 7, which is only computable from the persisted field.
    db.execute("ALTER SEQUENCE s RESTART").unwrap();
    assert_eq!(scalar(&mut db, "SELECT nextval('s')"), Value::Int8(7));
}

/// A `SERIAL` column's implicit sequence records its owner too — the same edge an
/// explicit `OWNED BY` writes — and it survives a reopen.
#[test]
fn implicit_serial_sequence_records_its_owner_across_reopen() {
    let dir = ScratchDir::new("sequence_catalog_serial_owner");

    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE t (id SERIAL, v INT)").unwrap();
    }

    let db = Database::open(&dir.path).expect("reopen");
    let sequence = db.schema().sequences.get("t_id_seq").expect("t_id_seq");
    assert_eq!(
        sequence.owner,
        Some(SequenceOwner {
            table: "t".to_string(),
            column: "id".to_string(),
        }),
    );
}

/// `setval` moves the counter and **nothing else**.
///
/// The record made this a live hazard: every site that used to `insert` a fresh
/// counter would now write a whole fresh record, silently resetting `start` (so a
/// later bare RESTART lands in the wrong place) and dropping `owner` (so the dump
/// stops emitting `OWNED BY`). Neither is visible to a query that only reads
/// `nextval`, which is why this asserts the catalog fields directly.
#[test]
fn setval_preserves_start_and_owner() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE own (a INT)").unwrap();
    db.execute("CREATE SEQUENCE s START WITH 5").unwrap();
    db.execute("ALTER SEQUENCE s OWNED BY own.a").unwrap();

    db.execute("SELECT setval('s', 100)").unwrap();
    db.execute("SELECT setval('s', 200, false)").unwrap();

    let sequence = db.schema().sequences.get("s").expect("sequence");
    assert_eq!(sequence.last_value, 199, "setval moved the counter");
    assert_eq!(sequence.start, 5, "setval must not touch START WITH");
    assert_eq!(
        sequence.owner,
        Some(SequenceOwner {
            table: "own".to_string(),
            column: "a".to_string(),
        }),
        "setval must not drop the owner"
    );
}

/// A `DEFAULT nextval('x')` whose `x` is the name a constraint index of the **same
/// statement** would take: the index disambiguates, as it does in Postgres when that
/// sequence already exists.
///
/// No fixture can reach this. Postgres resolves `'x'::regclass` when it cooks the
/// default, so a name nothing holds is `42P01` there and the statement never runs;
/// pglet models no regclass and auto-creates the sequence (a filed divergence), so the
/// two engines take different paths through the *same* statement and the harness has
/// nothing to compare. What the harness *can* see is the control — pre-create the
/// sequence and both engines answer `t5_pkey1` — and this asserts pglet reaches that
/// same state when the statement itself is what registers the name.
///
/// The failure it guards is not the error: without the claim the sequence is invisible
/// to the namer, the index is minted straight on top of it, and the collision surfaces
/// from `create_unique_index`'s backstop — **after** `schema.create_table`. The
/// autocommit path rolls back storage and keeps the mutated catalog, so the table name
/// is `42P07` forever while every read of it is `42P01`. Hence the reopen: the phantom
/// this guards is durable, and a same-process assertion would not say so.
#[test]
fn a_default_nextval_naming_a_constraint_index_disambiguates() {
    let dir = ScratchDir::new("sequence_catalog_default_nextval_index");

    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE t1 (a INT DEFAULT nextval('t1_pkey'), b INT PRIMARY KEY)")
            .expect("the index yields to the sequence this statement registers");
        assert!(
            db.schema().get_index("t1_pkey1").is_some(),
            "the constraint index disambiguated around the sequence"
        );
        assert!(db.schema().sequences.contains_key("t1_pkey"));
        // The behavioural half: the table is real on both sides of the catalog.
        db.execute("INSERT INTO t1 (b) VALUES (1)").unwrap();
        assert_eq!(scalar(&mut db, "SELECT a FROM t1"), Value::Int4(1));

        // A **declared** constraint name is the user's, so it collides instead of
        // disambiguating — the same `42P07` Postgres gives when the sequence already
        // exists. That half must land read-only, before the table is created.
        let err = db
            .execute("CREATE TABLE t6 (a INT DEFAULT nextval('t6_ck'), b INT, CONSTRAINT t6_ck PRIMARY KEY (b))")
            .expect_err("a declared constraint name equal to the default's sequence collides");
        assert_eq!(err.sqlstate(), "42P07");
        assert!(db.schema().get_table("t6").is_none(), "phantom table");
        assert!(
            !db.schema().sequences.contains_key("t6_ck"),
            "phantom sequence"
        );
        // ...and both names are still free.
        db.execute("CREATE TABLE t6 (q INT)").unwrap();
        db.execute("CREATE SEQUENCE t6_ck").unwrap();
    }

    let mut db = Database::open(&dir.path).expect("reopen");
    assert_eq!(scalar(&mut db, "SELECT a FROM t1"), Value::Int4(1));
    assert!(db.schema().get_index("t1_pkey1").is_some());
}

/// The `ADD COLUMN` twin, and the one whose failure is data corruption rather than a
/// burnt name — so it asserts the rows, not just the catalog.
///
/// Without the claim, `ALTER TABLE e1 ADD COLUMN y INT DEFAULT nextval('e1_y_key')
/// UNIQUE` raised `42P07` from the same post-mutation backstop, on a table that already
/// had rows: the column was in the catalog, the widened rows were rolled back with
/// storage, and `SELECT *` then sent a two-column `RowDescription` with one-field
/// `DataRow`s — `unexpected field count in "D" message`, permanently, for every client.
///
/// **Backend matters here.** The rollback is what strands the rows, so the corruption
/// appears on the CoW B+tree (in-memory or file-backed) and *not* on the plain
/// `MemoryStorage` double, which does not roll back and leaves the backfill applied.
/// This runs file-backed, the shape the server's `--data-dir` uses.
#[test]
fn an_add_column_default_nextval_naming_its_index_keeps_every_row() {
    let dir = ScratchDir::new("sequence_catalog_add_column_default_nextval");

    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE e1 (x INT)").unwrap();
        db.execute("INSERT INTO e1 VALUES (1), (2)").unwrap();
        db.execute("ALTER TABLE e1 ADD COLUMN y INT DEFAULT nextval('e1_y_key') UNIQUE")
            .expect("the index yields to the sequence this statement registers");
        assert!(db.schema().get_index("e1_y_key1").is_some());
        assert!(db.schema().sequences.contains_key("e1_y_key"));
    }

    let mut db = Database::open(&dir.path).expect("reopen");
    let result = db.execute("SELECT * FROM e1 ORDER BY x").unwrap().remove(0);
    assert_eq!(result.columns().len(), 2);
    assert_eq!(
        result.rows,
        vec![
            vec![Value::Int4(1), Value::Int4(1)],
            vec![Value::Int4(2), Value::Int4(2)],
        ],
        "every row must carry a value for the added column"
    );

    // The other way to make the statement above succeed is to register the sequence
    // *before* the naming block, so the namer sees it in the catalog rather than in
    // flight. That would pass every assertion so far and re-open the class this whole
    // family exists for, so pin it from the other side: an `ADD COLUMN` that fails on a
    // declared name must leave no sequence behind.
    let err = db
        .execute(
            "ALTER TABLE e1 ADD COLUMN z INT DEFAULT nextval('ghost') CONSTRAINT e1_y_key1 UNIQUE",
        )
        .expect_err("the declared index name is taken");
    assert_eq!(err.sqlstate(), "42P07");
    assert!(
        !db.schema().sequences.contains_key("ghost"),
        "the failed ADD COLUMN left a phantom sequence"
    );
}

/// `TRUNCATE … RESTART IDENTITY` likewise moves only the counter — and moves it to
/// `start`, not to zero.
///
/// The `START WITH` is deliberately **not** 1: at the default the two answers
/// coincide (`start - 1 == 0`), which is exactly why the fixture that used a plain
/// `SERIAL` could never see the reset target being wrong.
#[test]
fn truncate_restart_identity_preserves_start_and_owner() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT GENERATED ALWAYS AS IDENTITY (START WITH 40), v INT)")
        .unwrap();
    db.execute("INSERT INTO t (v) VALUES (1), (2)").unwrap();
    db.execute("TRUNCATE t RESTART IDENTITY").unwrap();

    let sequence = db.schema().sequences.get("t_id_seq").expect("sequence");
    assert_eq!(
        sequence.last_value, 39,
        "the counter returns to `start - 1`, so the next value is the START WITH"
    );
    assert_eq!(sequence.start, 40);
    assert_eq!(
        sequence.owner,
        Some(SequenceOwner {
            table: "t".to_string(),
            column: "id".to_string(),
        }),
    );
}

/// The `setval` overload gate reads an argument's **static** type, and a bare literal
/// has none — Postgres leaves it `unknown` and coerces it to the parameter afterwards,
/// so `setval(s, '5')` is an ordinary call there (it answers `5`).
///
/// pglet's IR has no `unknown`: a bare string literal types as `text`, which the gate
/// would reject as `42883` if it read `infer_type` alone. It does not — the value path
/// speaks instead, and still answers `42804` on the coercion, which is the pre-existing
/// literal-coercion gap and **not** this gate's to widen. The fixtures cannot guard this:
/// under `@expect-error` any pglet error passes when Postgres accepts the query, so
/// `42883` and `42804` are indistinguishable there.
#[test]
fn the_setval_overload_gate_leaves_an_unknown_literal_alone() {
    let mut db = Database::with_btree();
    db.execute("CREATE SEQUENCE unk").unwrap();

    let err = db
        .execute("SELECT setval('unk', '5')")
        .expect_err("pglet does not coerce the unknown literal");
    assert_eq!(
        err.sqlstate(),
        "42804",
        "the gate must decline an unknown literal and let the value path answer"
    );

    // A typed argument the overloads do take is unaffected either way.
    assert_eq!(scalar(&mut db, "SELECT setval('unk', 5)"), Value::Int8(5));
}

/// A refused `ALTER SEQUENCE … OWNED BY` on an identity sequence leaves the whole
/// record untouched — the counter, the `start`, and the owner edge.
///
/// The identity gate is the statement's **last** check (the owner is resolved first,
/// as `process_owned_by` does), so it is the one most easily written after a field has
/// already moved. `START WITH` / `RESTART` are applied in the same call, and on the
/// autocommit path a mutate-then-fail statement is durable: the server snapshots the
/// catalog only inside an explicit `BEGIN`, so a `RESTART` that landed before the
/// refusal would silently stand. Every fixture runs inside `BEGIN … ROLLBACK`, so none
/// of them can see it.
#[test]
fn a_refused_identity_owned_by_leaves_the_sequence_record_untouched() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT GENERATED ALWAYS AS IDENTITY (START WITH 7), v INT)")
        .unwrap();
    assert_eq!(
        scalar(&mut db, "SELECT nextval('t_id_seq')"),
        Value::Int8(7)
    );

    let err = db
        .execute("ALTER SEQUENCE t_id_seq START WITH 200 RESTART WITH 50 OWNED BY NONE")
        .expect_err("an identity sequence refuses every OWNED BY");
    assert_eq!(err.sqlstate(), "0A000");

    let sequence = db.schema().sequences.get("t_id_seq").expect("sequence");
    assert_eq!(sequence.last_value, 7, "the RESTART must not have landed");
    assert_eq!(sequence.start, 7, "nor the START WITH");
    assert_eq!(
        sequence.owner,
        Some(SequenceOwner {
            table: "t".to_string(),
            column: "id".to_string(),
        }),
        "nor the detach"
    );
    assert_eq!(
        scalar(&mut db, "SELECT nextval('t_id_seq')"),
        Value::Int8(8)
    );
}

/// A refused `SET DEFAULT` on an identity column registers no sequence.
///
/// `execute_alter_table_set_default` *creates* the sequence a `nextval('x')` default
/// names, so a gate placed after that registration would leave a durable phantom on
/// the autocommit path — one `nextval` then serves, and the dump emits a
/// `CREATE SEQUENCE` for a sequence no statement asked for. The identity refusal is
/// therefore ahead of it, which is also where 18.6 puts it (`SET DEFAULT
/// nextval('<no such sequence>')` on an identity column is `42601`, not the name's
/// own error).
#[test]
fn a_refused_identity_set_default_registers_no_sequence() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT GENERATED ALWAYS AS IDENTITY, v INT)")
        .unwrap();

    let err = db
        .execute("ALTER TABLE t ALTER COLUMN id SET DEFAULT nextval('ghost')")
        .expect_err("an identity column's default is not the user's to move");
    assert_eq!(err.sqlstate(), "42601");
    assert!(
        !db.schema().sequences.contains_key("ghost"),
        "the refused statement must register nothing"
    );

    // The identity link is intact: the column still draws from its own sequence, and
    // that sequence is still the one the INTERNAL edge protects.
    db.execute("INSERT INTO t (v) VALUES (1)").unwrap();
    assert_eq!(scalar(&mut db, "SELECT id FROM t"), Value::Int4(1));
    let err = db
        .execute("DROP SEQUENCE t_id_seq")
        .expect_err("the sequence is still the identity's internal implementation");
    assert_eq!(err.sqlstate(), "2BP01");
}

/// A durable store writes its catalog at every commit that changes it — a `CREATE
/// SEQUENCE`, an autocommit `nextval`, an `INSERT` drawing a `SERIAL` id — and a reopen of
/// the image after each reads it back. The control beside `pglet`'s
/// `a_store_one_connection_drives_encodes_no_catalog`, where a store nothing reopens and
/// no other connection shares writes none: a durable store has a reader on every commit.
///
/// Red with `StorageBackend::persists_catalog` answering `false` for a durable B+tree.
#[test]
fn a_durable_store_writes_its_catalog_at_each_change_and_reopens_with_it() {
    use pglet_testing::block_io::MemBlockIo;
    use pglet_testing::db::open_database;
    let io = MemBlockIo::new();
    let mut db = open_database(Box::new(io.clone()));
    let durable = |db: &Database| db.storage().persisted_catalog().unwrap().unwrap();
    let reopened = || open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
    let mut held = durable(&db);
    for sql in [
        "CREATE SEQUENCE s",
        "SELECT nextval('s')",
        "CREATE TABLE t (id SERIAL, v INT)",
        "INSERT INTO t (v) VALUES (1)",
    ] {
        db.execute(sql).unwrap();
        let now = durable(&db);
        assert!(
            !std::sync::Arc::ptr_eq(&held, &now),
            "{sql}: a catalog write"
        );
        held = now;
    }
    let mut image = reopened();
    assert_eq!(scalar(&mut image, "SELECT nextval('s')"), Value::Int8(2));
    assert_eq!(
        scalar(&mut image, "INSERT INTO t (v) VALUES (2) RETURNING id"),
        Value::Int4(2)
    );
}
