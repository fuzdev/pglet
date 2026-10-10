use pglet::{BoundExecError, Database};
use pglet_sql::executor::QueryResult as ExecQueryResult;
use wasm_bindgen::prelude::*;

use crate::call::ModuleCall;
use crate::error::pglet_error_to_js;
use crate::param::bind_js_params;
use crate::result::{QueryResult, RowShape};

/// The single logical connection every wasm binding drives (the host is
/// single-threaded, so one connection id suffices for transaction routing).
const CONN_ID: i32 = 0;

/// A statement's failure on its way out to JS.
///
/// The exec core returns this rather than a `JsValue` so that a statement with an
/// **empty `params`** reaches no JS call: `wasm_bindgen`'s imports abort on a
/// non-wasm target, so a core that built its error eagerly could not be driven by
/// `cargo test` — and the transaction routing is exactly what wants driving there.
/// The bound-parameter path is not free of JS calls and cannot be: `bind_js_params`
/// asks each argument what it is. The `#[wasm_bindgen]` methods convert at the
/// boundary via [`run_error_to_js`], which is where the JS shapes are decided.
pub enum RunError {
    /// A parse / plan / execute error from the engine, carrying its SQLSTATE. The
    /// transaction lifecycle rides this too: the facade's session raises an ordinary
    /// engine error for a `COMMIT` that cannot persist or a `ROLLBACK` whose save the
    /// store refuses, so there is no second error domain for it.
    Engine(pglet::Error),
    /// The parameter-binding callback's own JS error, passed through unchanged.
    Bind(JsValue),
}

impl From<BoundExecError<JsValue>> for RunError {
    fn from(err: BoundExecError<JsValue>) -> Self {
        match err {
            BoundExecError::Engine(e) => Self::Engine(e),
            BoundExecError::Bind(js) => Self::Bind(js),
        }
    }
}

/// Render a [`RunError`] as the single JS error the binding throws.
///
/// An engine error becomes a JS `Error` carrying its SQLSTATE; a binding (JS-decode)
/// error is already a `JsValue` and passes straight through, so each domain keeps its
/// original JS shape.
pub fn run_error_to_js(err: RunError) -> JsValue {
    match err {
        RunError::Engine(e) => pglet_error_to_js(&e),
        RunError::Bind(js) => js,
    }
}

// This module holds no transaction lifecycle. One `pglet::TransactionSession` inside
// the facade drives this leg and the native server's alike — the aborted-block gate
// (`25P02`, and a `COMMIT` of an aborted block converted to a `ROLLBACK`), the tag
// dispatch and the catalog snapshot/restore — so `exec_bound` gets all of it from the
// facade, and the rules are on `TransactionSession`'s methods. A copy here would be a
// second one, free to disagree with the server's about **committed data**.

/// Run one statement through the facade with bound parameters.
///
/// The facade infers each `$N`, the `bind_js_params` callback decodes the JS
/// arguments against the inferred types (an empty `params` binds nothing), and the
/// aborted-block gate, the transaction lifecycle and the autocommit-persist decision
/// all live in the facade. The in-memory binding here and the persistent one
/// (`crate::persistent`, built only with the `opfs` feature) both run their statements through here, so the
/// two share one state machine.
pub fn exec_bound(
    db: &mut Database,
    sql: &str,
    params: &[JsValue],
) -> Result<ExecQueryResult, RunError> {
    db.execute_bound(sql, CONN_ID, |oids| bind_js_params(params, oids))
        .map_err(RunError::from)
}

/// The database a [`PgletBtree`] holds: an in-memory B+tree that [`CONN_ID`] alone
/// drives, whose commits encode no catalog (`Database::with_btree_for_one_connection`).
fn in_memory_database() -> Database {
    Database::with_btree_for_one_connection(CONN_ID)
}

/// In-memory copy-on-write B+tree database for JS — the shipped in-memory database.
///
/// The same CoW B+tree as the native default and the browser's persistent store — MVCC pending-writes
/// buffering, and the same storage the native server runs — but holds everything in
/// memory (no OPFS, no persistence), exactly what a Node test harness needs. One
/// connection drives it, so its commits encode no catalog (`in_memory_database`). It
/// also supports bound `$N` parameters via [`PgletBtree::query_params`].
///
/// There is no wasm binding over plain `MemoryStorage`: the build leaves `pglet`'s
/// `memory_storage` feature off, so `StorageBackend` has no `Memory` arm here (see
/// the compile-time check in `lib.rs`).
#[wasm_bindgen]
#[derive(Debug)]
pub struct PgletBtree {
    db: Database,
}

#[wasm_bindgen]
impl PgletBtree {
    /// Creates a new in-memory B+tree database.
    ///
    /// # Errors
    ///
    /// `PgletModulePoisoned` once a host exception has unwound a call into this module,
    /// and `PgletReentrantCall` for a call made while another is running (`call`
    /// module).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<Self, JsValue> {
        let _call = ModuleCall::enter()?;
        Ok(Self {
            db: in_memory_database(),
        })
    }

    /// Executes a SQL statement, discarding any result rows.
    #[wasm_bindgen]
    pub fn exec(&mut self, sql: &str) -> Result<(), JsValue> {
        self.run(sql, &[], RowShape::Objects)?;
        Ok(())
    }

    /// Executes a SQL statement and returns the result rows — objects of JS values; with
    /// `positional`, arrays of them; with `text`, arrays of each cell's text form
    /// (`QueryResult::from_pglet_result`).
    #[wasm_bindgen]
    pub fn query(
        &mut self,
        sql: &str,
        text: Option<bool>,
        positional: Option<bool>,
    ) -> Result<QueryResult, JsValue> {
        let shape = RowShape::from_flags(text.unwrap_or(false), positional.unwrap_or(false));
        self.run(sql, &[], shape)
    }

    /// Executes a SQL statement with bound positional `$N` parameters.
    ///
    /// `params` is a JS array; each element is decoded to the parameter type
    /// inferred from query context. JS `null`/`undefined` binds SQL `NULL`.
    // wasm-bindgen marshals a JS array as an owned `Vec<JsValue>`; we only borrow it.
    #[allow(clippy::needless_pass_by_value)]
    #[wasm_bindgen(js_name = queryParams)]
    pub fn query_params(
        &mut self,
        sql: &str,
        params: Vec<JsValue>,
        text: Option<bool>,
        positional: Option<bool>,
    ) -> Result<QueryResult, JsValue> {
        let shape = RowShape::from_flags(text.unwrap_or(false), positional.unwrap_or(false));
        self.run(sql, &params, shape)
    }

    /// How many transaction levels are open: `0` idle, `1` inside a
    /// `BEGIN … COMMIT`, and one more per live savepoint.
    ///
    /// Exposed because a host cannot derive it from command tags: `ROLLBACK TO
    /// SAVEPOINT` reports tag `ROLLBACK`, exactly as a whole-block rollback does, so
    /// a tag-mirroring adapter reads the first as the second and lets the next
    /// `BEGIN` open a second block over the still-open first — committing or
    /// discarding rows that belong to the outer one. The npm adapter reads this
    /// instead of mirroring, and it is also how it decides whether its
    /// `transaction()` should issue a `BEGIN` or a `SAVEPOINT`.
    ///
    /// # Errors
    ///
    /// `PgletModulePoisoned` once a host exception has unwound a call into this module,
    /// and `PgletReentrantCall` for a call made while another is running.
    #[wasm_bindgen(js_name = transactionDepth)]
    pub fn transaction_depth(&self) -> Result<usize, JsValue> {
        let _call = ModuleCall::enter()?;
        Ok(self.db.transaction_depth())
    }

    /// Fork this database into a fresh, isolated copy of its current state.
    ///
    /// The per-test-fixture primitive: seed a base instance once (schema + seed
    /// data), then `fork()` a cheap independent database per test instead of
    /// building a new `PgletBtree` and re-running the setup SQL each time. The
    /// fork shares the base's row data copy-on-write, so it is far cheaper than
    /// replaying the schema + seed. Writes on either side are invisible to the
    /// other. The fork starts with no open transaction and reflects **committed**
    /// state — which is why forking while a block is open on this instance is
    /// refused (`25001`) rather than served: the branch would carry the block's
    /// catalog writes over a heap that lacks its rows.
    ///
    /// # Errors
    ///
    /// Returns a JS error if a transaction block is open here (`25001`), or if the
    /// fork can't be created.
    #[wasm_bindgen]
    pub fn fork(&self) -> Result<Self, JsValue> {
        let _call = ModuleCall::enter()?;
        let db = self.db.fork().map_err(|e| pglet_error_to_js(&e))?;
        Ok(Self { db })
    }
}

impl PgletBtree {
    /// Shared exec core: the facade gates the aborted block, executes the statement
    /// with the bound parameters, and applies the transaction lifecycle its carried
    /// `TransactionCommand` calls for (catalog snapshot/restore for in-transaction DDL
    /// included).
    fn run(
        &mut self,
        sql: &str,
        params: &[JsValue],
        shape: RowShape,
    ) -> Result<QueryResult, JsValue> {
        let _call = ModuleCall::enter()?;
        let result = exec_bound(&mut self.db, sql, params).map_err(run_error_to_js)?;
        QueryResult::from_pglet_result(result, shape)
    }
}

#[cfg(test)]
mod transaction_routing_tests {
    //! Which statements open and end a transaction on the wasm leg.
    //!
    //! These drive [`exec_bound`] itself — the function every binding in this crate
    //! calls — on the **host** target under `cargo test`. That is possible only
    //! because it makes no JS call with an empty `params`: `wasm_bindgen`'s imports
    //! abort off wasm, so a version that built its `JsValue` result or error inside
    //! would be untestable here (measured — it aborts in `js_sys`). What these cannot
    //! reach is the JS marshaling on either side of it, `RunError`'s rendering, and
    //! the wasm target itself; `tests-web/integration/pglet_wasm_test.ts` drives the
    //! built artifact from JS for that.
    //!
    //! The rules they check are the facade's `pglet::TransactionSession`, which the
    //! native server drives too; these assert that this leg *reaches* them, which is
    //! the property that was false — the two legs used to hold two copies and
    //! disagreed about committed data.

    use pglet::Database;
    use pglet_sql::types::Value;

    use super::{CONN_ID, RunError, exec_bound, in_memory_database};

    /// One statement through the real exec core: its command tag, or its SQLSTATE.
    fn run(db: &mut Database, sql: &str) -> Result<String, String> {
        match exec_bound(db, sql, &[]) {
            Ok(result) => Ok(result.command_tag),
            Err(RunError::Engine(e)) => Err(e.sqlstate().to_string()),
            // Unreachable with no parameters to decode, and building a `JsValue`
            // message here would abort off wasm anyway.
            Err(RunError::Bind(_)) => Err("bind".to_owned()),
        }
    }

    /// A seeded one-row table over the in-memory CoW B+tree — the database every
    /// in-memory binding holds.
    fn seeded() -> Database {
        let mut db = in_memory_database();
        db.execute("CREATE TABLE t (a INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        db
    }

    fn count(db: &mut Database) -> i64 {
        match db.execute("SELECT count(*)::int8 FROM t").unwrap()[0].rows[0][0] {
            Value::Int8(n) => n,
            ref other => panic!("expected a count, got {other:?}"),
        }
    }

    /// The binding's database is one its one connection drives, so its commits write no
    /// catalog: a commit that changes the catalog and no row — a sequence draw, alone or
    /// in a block — leaves the store's commit stamp where it was, on a fork of it too.
    ///
    /// Red with the binding back on `Database::with_btree`, whose catalog write moves the
    /// stamp.
    #[test]
    fn the_bindings_database_writes_no_catalog() {
        let draws = [
            "SELECT nextval('t_id_seq')",
            "BEGIN",
            "SELECT nextval('t_id_seq')",
            "COMMIT",
        ];
        let mut db = in_memory_database();
        run(&mut db, "CREATE TABLE t (id SERIAL, v INT)").unwrap();
        let at = db.storage().commit_stamp();
        for sql in draws {
            run(&mut db, sql).unwrap();
        }
        assert_eq!(db.storage().commit_stamp(), at);
        let mut fork = db.fork().unwrap();
        let at = fork.storage().commit_stamp();
        for sql in draws {
            run(&mut fork, sql).unwrap();
        }
        assert_eq!(fork.storage().commit_stamp(), at);
    }

    /// Savepoints work on this leg and the aborted block recovers through one, both
    /// byte-for-byte with 18.6.
    ///
    /// The failure it guards is worth keeping: with
    /// no savepoints in the engine, routing on a bare `starts_with("ROLLBACK")`
    /// answered `ROLLBACK TO SAVEPOINT sp` with tag `ROLLBACK` **and threw away every
    /// write since `BEGIN`** — silently, since a tag is all a caller sees. The
    /// signal is a `TransactionCommand` now rather than the tag, which is what lets
    /// the same tag mean two different things; the shape that met the old bug — a
    /// savepoint-per-test harness — is the one asserted here.
    ///
    /// The tail is the other half: once a statement raises, the block aborts, and
    /// `ROLLBACK TO` is the one statement it still admits (18.6's
    /// `IsTransactionExitStmt`). A successful one clears the abort and the block
    /// carries on, so the `COMMIT` keeps what the surviving level wrote.
    #[test]
    fn a_savepoint_rolls_back_its_own_level_and_recovers_an_aborted_block() {
        let mut db = seeded();
        run(&mut db, "BEGIN").unwrap();
        run(&mut db, "INSERT INTO t VALUES (2)").unwrap();
        assert_eq!(run(&mut db, "SAVEPOINT sp").unwrap(), "SAVEPOINT");
        run(&mut db, "INSERT INTO t VALUES (3)").unwrap();
        assert_eq!(count(&mut db), 3);

        // `ROLLBACK TO` reports tag `ROLLBACK` — the same tag the whole-block
        // rollback reports, which is exactly why the routing cannot read the tag —
        // and discards only what the level wrote.
        assert_eq!(
            run(&mut db, "ROLLBACK TO SAVEPOINT sp").unwrap(),
            "ROLLBACK"
        );
        assert_eq!(count(&mut db), 2, "only the savepoint's own write is gone");
        assert!(
            db.storage().has_transaction(CONN_ID),
            "the block itself must survive a ROLLBACK TO"
        );

        // A failing statement aborts the block: everything but an exit statement is
        // `25P02`…
        assert_eq!(run(&mut db, "SELECT 1/0").unwrap_err(), "22012");
        assert_eq!(run(&mut db, "SELECT count(*) FROM t").unwrap_err(), "25P02");
        assert_eq!(run(&mut db, "SAVEPOINT sp2").unwrap_err(), "25P02");
        // …except `ROLLBACK TO`, which is admitted and clears the abort.
        assert_eq!(run(&mut db, "ROLLBACK TO sp").unwrap(), "ROLLBACK");
        run(&mut db, "INSERT INTO t VALUES (4)").unwrap();

        assert_eq!(run(&mut db, "COMMIT").unwrap(), "COMMIT");
        assert!(!db.storage().has_transaction(CONN_ID));
        assert_eq!(count(&mut db), 3, "the recovered block commits its writes");
    }

    /// A `ROLLBACK TO` naming no live savepoint is `3B001` **and aborts the block**,
    /// so the `COMMIT` after it keeps nothing.
    ///
    /// The two halves are one measurement on 18.6 and neither follows from the
    /// other: the error is the session's, while the abort is the ordinary rule that
    /// any raising statement aborts its block — which a lifecycle applied *after*
    /// the executor could easily have skipped, since nothing the executor ran
    /// failed.
    #[test]
    fn a_rollback_to_a_missing_savepoint_is_3b001_and_aborts_the_block() {
        let mut db = seeded();
        run(&mut db, "BEGIN").unwrap();
        run(&mut db, "INSERT INTO t VALUES (2)").unwrap();
        assert_eq!(run(&mut db, "ROLLBACK TO nope").unwrap_err(), "3B001");
        assert_eq!(run(&mut db, "SELECT count(*) FROM t").unwrap_err(), "25P02");
        assert_eq!(run(&mut db, "COMMIT").unwrap(), "ROLLBACK");
        assert_eq!(count(&mut db), 1, "the aborted block must commit nothing");
    }

    /// The headline script, on the binding that used to get it wrong.
    ///
    /// `CREATE TABLE t (id INT PRIMARY KEY); BEGIN; INSERT 1; INSERT 2; INSERT 1;
    /// SELECT 1; INSERT 3; COMMIT; SELECT id FROM t` — measured on 18.6 and on the
    /// native server as `23505`, then `25P02` for everything after it, then command
    /// tag `ROLLBACK` and **no rows**. This binding answered the `23505`, ran the
    /// `SELECT` and the third `INSERT` normally, answered tag `COMMIT`, and left
    /// `1, 2, 3` committed — a divergence in committed data with no error or notice
    /// anywhere saying so.
    ///
    /// The differential fixture harness is structurally blind to this: every fixture
    /// runs inside `BEGIN … ROLLBACK`, which restores the snapshot, so the corpus only
    /// ever measures the `ROLLBACK` column. That is a blindness of the instrument, not
    /// a claim that the engine was right — hence an embed-shaped guard here and its
    /// twin in `crates/pglet`.
    #[test]
    fn a_failed_statement_aborts_the_block_and_the_commit_keeps_nothing() {
        let mut db = in_memory_database();
        db.execute("CREATE TABLE k (id INT PRIMARY KEY)").unwrap();

        run(&mut db, "BEGIN").unwrap();
        run(&mut db, "INSERT INTO k VALUES (1)").unwrap();
        run(&mut db, "INSERT INTO k VALUES (2)").unwrap();
        assert_eq!(
            run(&mut db, "INSERT INTO k VALUES (1)").unwrap_err(),
            "23505"
        );

        assert_eq!(run(&mut db, "SELECT 1").unwrap_err(), "25P02");
        assert_eq!(
            run(&mut db, "INSERT INTO k VALUES (3)").unwrap_err(),
            "25P02"
        );
        assert_eq!(run(&mut db, "COMMIT").unwrap(), "ROLLBACK");

        let rows = &db.execute("SELECT id FROM k").unwrap()[0].rows;
        assert!(rows.is_empty(), "the COMMIT must publish nothing: {rows:?}");
    }

    /// A leading comment no longer wedges the session, on this leg either.
    ///
    /// The exit classifier is the facade's and reads the lexer, so a comment ahead of
    /// the `COMMIT` is stripped before anything classifies. Under the old text match
    /// this answered `25P02` **and so did every statement after it**, `BEGIN`
    /// included, until a bare exit arrived; 18.6 ends the block.
    #[test]
    fn a_comment_prefixed_exit_ends_an_aborted_block() {
        let mut db = seeded();
        run(&mut db, "BEGIN").unwrap();
        run(&mut db, "INSERT INTO t VALUES (2)").unwrap();
        run(&mut db, "SELECT * FROM no_such_table").unwrap_err();

        assert_eq!(run(&mut db, "/* trace */ COMMIT").unwrap(), "ROLLBACK");
        assert!(!db.storage().has_transaction(CONN_ID));
        assert_eq!(count(&mut db), 1);

        // The other spelling, and the `ROLLBACK` half.
        run(&mut db, "BEGIN").unwrap();
        run(&mut db, "SELECT * FROM no_such_table").unwrap_err();
        assert_eq!(run(&mut db, "-- trace\nROLLBACK").unwrap(), "ROLLBACK");
        assert!(!db.storage().has_transaction(CONN_ID));
        // The session is usable again rather than wedged: a fresh block opens.
        assert_eq!(run(&mut db, "BEGIN").unwrap(), "BEGIN");
        assert_eq!(run(&mut db, "ROLLBACK").unwrap(), "ROLLBACK");
    }

    /// A second `BEGIN` inside an open transaction changes nothing.
    ///
    /// The binding's catalog snapshot and the storage transaction must have the same
    /// lifetime, and a `BEGIN` arm that re-snapshots unconditionally breaks that: the
    /// storage `begin_transaction` is silently idempotent, so it keeps the **first**
    /// snapshot while the binding replaces the catalog one with the post-DDL state, and
    /// the `ROLLBACK` then restores storage to before the table and the catalog to
    /// after it. That is the durable-phantom state — `SELECT` is `42P01`, the name is
    /// `42P07`, and only a `DROP TABLE` no client would issue recovers it — reached
    /// from plain JS.
    ///
    /// 18.6 answers the nested `BEGIN` with `WARNING: there is already a transaction
    /// in progress` and a no-op, tag `BEGIN`; `pglet_server`'s
    /// `Connection::begin_transaction` returns early the same way and emits no warning
    /// (both measured), and neither does this. So the assertion is the tag and the
    /// state, and the discriminating one is the last: with the condition removed, the
    /// undone write and the closed transaction both still check out and only the
    /// **name is free again** fails, with `42P07`.
    #[test]
    fn a_nested_begin_keeps_the_first_catalog_snapshot() {
        let mut db = seeded();
        run(&mut db, "BEGIN").unwrap();
        run(&mut db, "CREATE TABLE a (x INT)").unwrap();
        run(&mut db, "INSERT INTO t VALUES (2)").unwrap();

        assert_eq!(run(&mut db, "BEGIN").unwrap(), "BEGIN");
        assert!(
            db.storage().has_transaction(CONN_ID),
            "the nested BEGIN must leave the transaction open"
        );

        assert_eq!(run(&mut db, "ROLLBACK").unwrap(), "ROLLBACK");
        assert!(!db.storage().has_transaction(CONN_ID));
        assert_eq!(count(&mut db), 1, "the write must be undone");
        run(&mut db, "CREATE TABLE a (x INT)")
            .expect("the rolled-back name must be free again, not a 42P07 phantom");
    }

    /// A statement the parser refuses neither opens nor ends a transaction.
    ///
    /// Each of these was a success with a synthesized tag under the text match. The
    /// first three are an identifier that merely starts with the keyword, and are
    /// `42601` on 18.6 and on the native server. The last two carry a modifier 18.6
    /// implements and pglet does not, so pglet refuses the whole statement where the
    /// text match half-honored it by dropping the modifier — and 18.6 accepts
    /// `START TRANSACTION READ ONLY` while `COMMIT AND CHAIN` is `25P01` outside a
    /// transaction block and commits-and-chains inside one.
    ///
    /// The `READ ONLY` member replaced
    /// `START TRANSACTION ISOLATION LEVEL SERIALIZABLE`, which the parser now
    /// accepts: this guard's seed was the parser refusing a modifier, so closing one
    /// refusal empties the case unless another refused modifier takes its place.
    #[test]
    fn a_statement_the_parser_refuses_does_not_route() {
        for sql in [
            "BEGINNING",
            "COMMITTED",
            "ROLLBACKX",
            "COMMIT AND CHAIN",
            "START TRANSACTION READ ONLY",
        ] {
            let mut db = seeded();
            // The identifier trio's `42601` is asserted because both engines answer
            // it; the two modifiers get only the refusal, since which code pglet
            // reports for a statement 18.6 accepts is the parser's answer and not
            // this routing's.
            let code = run(&mut db, sql).unwrap_err();
            if !sql.contains(' ') {
                assert_eq!(code, "42601", "{sql:?}");
            }
            assert!(
                !db.storage().has_transaction(CONN_ID),
                "must not open a transaction: {sql:?}"
            );
        }
        // The other polarity of the same rule, in an open transaction: an unparseable
        // statement must not end it.
        let mut db = seeded();
        run(&mut db, "BEGIN").unwrap();
        for sql in ["COMMITTED", "ROLLBACKX", "COMMIT AND CHAIN"] {
            run(&mut db, sql).unwrap_err();
            assert!(
                db.storage().has_transaction(CONN_ID),
                "must not end the transaction: {sql:?}"
            );
        }
    }

    /// A non-ASCII spelling of a keyword is refused, and by the parser's own fold.
    ///
    /// U+0131 LATIN SMALL LETTER DOTLESS I uppercases to ASCII `I` and U+017F LATIN
    /// SMALL LETTER LONG S to `S`, so a Unicode-folding text match routed `COMMıT`
    /// and `ſTART TRANSACTION` into the lifecycle where 18.6 and the native server
    /// answer `42601`. Nothing here folds now — the parser decides — so this asserts
    /// the outcome rather than a fold width.
    #[test]
    fn a_non_ascii_keyword_spelling_does_not_route() {
        let mut db = seeded();
        for sql in [
            "COMMıT",
            "ſTART TRANSACTION",
            "BEGıN",
            "ﬅART TRANSACTION",
            "ﬆART TRANSACTION",
        ] {
            run(&mut db, sql).unwrap_err();
            assert!(
                !db.storage().has_transaction(CONN_ID),
                "must not open a transaction: {sql:?}"
            );
        }
    }

    /// The spellings below all route — including the two a text match missed
    /// (`/* c */ BEGIN` and the newline-split `START TRANSACTION`), which returned
    /// tag `BEGIN` having opened nothing, so the write that followed autocommitted
    /// and the `ROLLBACK` after it reverted nothing. The rest are the controls: a
    /// text match routed those, and they must keep routing.
    #[test]
    fn every_accepted_begin_spelling_opens_a_real_transaction() {
        for begin in [
            "BEGIN",
            "begin",
            "  BEGIN  ",
            "BEGIN;",
            "BEGIN WORK",
            "BEGIN TRANSACTION",
            "START TRANSACTION",
            "START\nTRANSACTION",
            "/* c */ BEGIN",
            "BEGIN /* x */",
            "BEGIN -- trailing\n",
        ] {
            let mut db = seeded();
            assert_eq!(run(&mut db, begin).unwrap(), "BEGIN", "{begin:?}");
            assert!(db.storage().has_transaction(CONN_ID), "{begin:?}");
            run(&mut db, "INSERT INTO t VALUES (2)").unwrap();
            assert_eq!(run(&mut db, "ROLLBACK").unwrap(), "ROLLBACK");
            assert_eq!(count(&mut db), 1, "the write must be undone: {begin:?}");
        }
    }

    /// The end spellings, and `COMMIT` keeps what the transaction wrote.
    #[test]
    fn the_accepted_end_spellings_route() {
        for commit in ["COMMIT", "commit;", "COMMIT WORK", "COMMIT TRANSACTION"] {
            let mut db = seeded();
            run(&mut db, "BEGIN").unwrap();
            run(&mut db, "INSERT INTO t VALUES (2)").unwrap();
            assert_eq!(run(&mut db, commit).unwrap(), "COMMIT");
            assert!(!db.storage().has_transaction(CONN_ID), "{commit:?}");
            assert_eq!(count(&mut db), 2, "{commit:?}");
        }
        for rollback in [
            "ROLLBACK",
            "rollback;",
            "ROLLBACK WORK",
            "ROLLBACK TRANSACTION",
        ] {
            let mut db = seeded();
            run(&mut db, "BEGIN").unwrap();
            run(&mut db, "INSERT INTO t VALUES (2)").unwrap();
            assert_eq!(run(&mut db, rollback).unwrap(), "ROLLBACK");
            assert!(!db.storage().has_transaction(CONN_ID), "{rollback:?}");
            assert_eq!(count(&mut db), 1, "{rollback:?}");
        }
    }

    /// The schema snapshot the `BEGIN` arm takes is what undoes an in-transaction
    /// DDL — the backend rollback restores storage and leaves the catalog alone.
    ///
    /// The discriminating assertion is that the **name is free again**: a read of the
    /// rolled-back table fails either way (storage rolled back under a catalog entry
    /// that survived is exactly the phantom this guards), so asserting only that
    /// would pass with the restore deleted — measured.
    #[test]
    fn a_rolled_back_ddl_leaves_no_table_behind() {
        let mut db = seeded();
        run(&mut db, "BEGIN").unwrap();
        run(&mut db, "CREATE TABLE gone (a INT)").unwrap();
        run(&mut db, "ROLLBACK").unwrap();
        assert!(db.execute("SELECT * FROM gone").is_err());
        run(&mut db, "CREATE TABLE gone (a INT)")
            .expect("the rolled-back name must be free again, not a 42P07 phantom");
    }
}
