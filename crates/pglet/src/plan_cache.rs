//! The bound entry point's cache of parsed statements, keyed by their text
//! ([`crate::Database::execute_bound`]).
//!
//! An entry is what that entry point computes from a statement's text before it binds:
//! the statement's one plan and its parameters' inferred targets. Both are read off the
//! text, the catalog, and the planner settings the calling thread holds, and off nothing
//! else — no session setting, clock, counter or connection reaches the parser or the
//! inference — so a statement found here is the statement parsed again. Everything else
//! the entry point does for a statement runs on a hit as on a miss.
//!
//! The inputs past the text are [`Stamp`]'s fields, and the whole cache answers for
//! one stamp: a lookup under another empties it first. **A planner input added anywhere —
//! a catalog change that does not move [`Schema::generation`], a thread setting the parser
//! starts reading — is one more field here, or a cached plan answers for a catalog or a
//! setting it was not planned under.** The catalog changes that leave the generation where
//! it was — a sequence's start, counter or owner, the OID counter — are ones no parse or
//! inference reads. The host's mutable borrow of the catalog can change anything, and can
//! move the generation backwards by replacing the catalog, so it empties the cache
//! ([`crate::Database::schema_mut`]).
//!
//! Natively the parser's byte check refuses `54001` against a limit made of the thread's
//! stack (in the stamp: [`pglet_sql::depth::thread_stack_bounds`], which a declared stack
//! and another thread move), the count's reserve (in the stamp, through the count), the
//! budget in force (a database's own empties its cache,
//! [`crate::Database::set_stack_budget`]; but the budget in force is the outermost entry's
//! on the thread, so a statement a `bind` callback runs on another database parses under
//! the calling database's budget, which the inner cache's stamp does not hold) — and how
//! deep in its own stack the caller already is, which is not in the stamp. A hit runs no
//! parse, so a statement held from a call made higher in the caller's stack is not refused
//! when the same text is called from deeper, where its parse would be; its execution's own
//! checks still run.

use std::sync::Arc;

use pglet_sql::catalog::Schema;
use pglet_sql::planner::{ParamTargets, Plan};

/// How many statements one cache holds; the next one empties it. A chosen bound, not a
/// measured one.
const CAP: usize = 128;

/// The longest text held. A plan is several times its text, so a long text — a seeding
/// `INSERT … VALUES` of thousands of rows — is parsed at each run rather than pinned, and
/// does not empty the cache of the short statements held beside it
/// ([`HELD_TEXT_BUDGET`]).
const MAX_TEXT_BYTES: usize = 4 * 1024;

/// The most text the held statements may total; the statement that would pass it empties
/// the cache first. It bounds the text held, not the plans: a plan's size follows the
/// catalog as well as its text — a `*` over a join of wide tables, or a `RETURNING *`,
/// expands to every column — so a short text can hold a large plan, and what a cache
/// retains is bounded only by [`CAP`] such plans.
const HELD_TEXT_BUDGET: usize = 64 * 1024;

/// A statement as the bound entry point parsed it: its one plan, and its parameters'
/// inferred targets — the types `bind` decodes against and the enums a bound value is
/// converted to.
#[derive(Debug)]
pub struct Prepared {
    pub plan: Plan,
    pub targets: ParamTargets,
}

/// What a parse and an inference read besides the text: the catalog, through its
/// generation, and the calling thread's statement-depth limit and unknown-type policy —
/// and natively the thread's stack, which the parser's byte check refuses `54001` against.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    generation: u64,
    max_depth: usize,
    coerce_unknown_types: bool,
    #[cfg(not(target_arch = "wasm32"))]
    stack: (usize, usize, usize),
}

impl Stamp {
    fn of(schema: &Schema) -> Self {
        Self {
            generation: schema.generation,
            max_depth: pglet_sql::depth::max_depth(),
            coerce_unknown_types: pglet_sql::planner::coerce_unknown_types(),
            #[cfg(not(target_arch = "wasm32"))]
            stack: pglet_sql::depth::thread_stack_bounds(),
        }
    }
}

/// One database's parsed statements, all under one [`Stamp`].
///
/// A list scanned by text rather than a map: it holds at most [`CAP`] texts, a comparison
/// checks the length first, and a map's code costs the wasm build more.
#[derive(Debug, Default)]
pub struct PlanCache {
    stamp: Stamp,
    entries: Vec<(Box<str>, Arc<Prepared>)>,
    /// The held texts' total length, at most [`HELD_TEXT_BUDGET`].
    held_text: usize,
    /// Lookups answered from the cache.
    #[cfg(test)]
    pub hits: usize,
}

impl PlanCache {
    /// The statement `sql` parsed under `schema` and this thread's settings, if it is
    /// held. Under a stamp other than the held one, the cache is emptied first.
    pub fn get(&mut self, sql: &str, schema: &Schema) -> Option<Arc<Prepared>> {
        let stamp = Stamp::of(schema);
        if stamp != self.stamp {
            self.clear();
            self.stamp = stamp;
        }
        let found = self
            .entries
            .iter()
            .find(|(text, _)| **text == *sql)
            .map(|(_, prepared)| Arc::clone(prepared));
        #[cfg(test)]
        if found.is_some() {
            self.hits += 1;
        }
        found
    }

    /// Hold `prepared` as `sql`'s, parsed under the stamp the [`Self::get`] that missed it
    /// read — unless running it changes what a parse reads ([`Plan::mutates_schema`]), when
    /// the next lookup would empty the cache anyway, its text is longer than
    /// [`MAX_TEXT_BYTES`], or it is deeper than any thread may have to drop it
    /// ([`droppable_anywhere`]). A cache holding [`CAP`] statements, or one this text would
    /// take past [`HELD_TEXT_BUDGET`], is emptied first.
    pub fn insert(&mut self, sql: &str, prepared: Prepared) -> Arc<Prepared> {
        let prepared = Arc::new(prepared);
        if sql.len() <= MAX_TEXT_BYTES
            && !prepared.plan.mutates_schema()
            && droppable_anywhere(&prepared.plan)
        {
            if self.entries.len() >= CAP || self.held_text + sql.len() > HELD_TEXT_BUDGET {
                self.clear();
            }
            self.held_text += sql.len();
            self.entries.push((sql.into(), Arc::clone(&prepared)));
        }
        prepared
    }

    /// Hold nothing.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.held_text = 0;
    }

    /// How many statements are held.
    #[cfg(test)]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// How much text is held, counted from the entries rather than the running total.
    #[cfg(test)]
    pub fn text_held(&self) -> usize {
        self.entries.iter().map(|(text, _)| text.len()).sum()
    }
}

/// Whether `plan` is no deeper than [`pglet_sql::depth::FALLBACK_MAX_DEPTH`], the limit
/// sized for the smallest stack a thread has by default.
///
/// An uncached plan is dropped where it was parsed, under the limit that admitted it; a
/// held one is dropped wherever the cache is emptied, and a host may move the database to
/// a thread with less stack than the one that parsed it. A walk only where this thread's
/// limit is above that one: at or under it, the parse's own check has bounded the plan,
/// to within the one level a locking read's wrapper adds after the check.
#[cfg(not(target_arch = "wasm32"))]
fn droppable_anywhere(plan: &Plan) -> bool {
    use pglet_sql::depth::{FALLBACK_MAX_DEPTH, max_depth, plan_depth};
    max_depth() <= FALLBACK_MAX_DEPTH || plan_depth(plan) <= FALLBACK_MAX_DEPTH
}

/// One thread, one stack: a plan the module parsed is droppable wherever it runs.
#[cfg(target_arch = "wasm32")]
const fn droppable_anywhere(_plan: &Plan) -> bool {
    true
}

#[cfg(test)]
mod tests {
    //! The guards of [`super::PlanCache`] through [`crate::Database::execute_bound`], the
    //! one entry point that consults it. The differential fixtures cannot see it: they
    //! run the native server, which never calls that entry point.

    use crate::{BoundExecError, Database, QueryResult, Schema};
    use pglet_sql::types::{TypeOid, Value};

    /// `sql` on `conn` with `params` bound as the inferred types read them, as its result
    /// or `ERR <SQLSTATE> <message>`.
    fn run(
        db: &mut Database,
        sql: &str,
        conn: i32,
        params: &[Value],
    ) -> Result<QueryResult, String> {
        let params = params.to_vec();
        match db.execute_bound(sql, conn, move |_| {
            Ok::<_, ()>(params.into_iter().map(Some).collect())
        }) {
            Ok(result) => Ok(result),
            Err(BoundExecError::Engine(e)) => Err(format!("ERR {} {e}", e.sqlstate())),
            Err(BoundExecError::Bind(())) => Err("bind".to_owned()),
        }
    }

    fn ok(db: &mut Database, sql: &str) -> QueryResult {
        run(db, sql, 0, &[]).unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    fn err(db: &mut Database, sql: &str, conn: i32) -> String {
        match run(db, sql, conn, &[]) {
            Ok(result) => panic!("{sql}: answered {}", result.command_tag),
            Err(e) => e,
        }
    }

    fn hits(db: &Database) -> usize {
        db.plan_cache.hits
    }

    /// The types `SELECT a FROM t WHERE a = $1` on `conn` hands its binding — the
    /// statement then run with the parameter `NULL`.
    fn param_types(db: &mut Database, conn: i32) -> Vec<Option<TypeOid>> {
        let mut seen = Vec::new();
        db.execute_bound("SELECT a FROM t WHERE a = $1", conn, |oids| {
            seen = oids.to_vec();
            Ok::<_, ()>(vec![None; oids.len()])
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
        seen
    }

    /// The same text twice is one parse; a statement that changes what a parse reads is
    /// never held, and the next lookup after it misses.
    ///
    /// Red with no cache (no hit), with the generation out of the stamp (the third run
    /// hits), and with a DDL plan held (the count).
    #[test]
    fn a_repeated_text_is_parsed_once_and_a_catalog_change_misses() {
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        assert_eq!(db.plan_cache.len(), 0, "a DDL plan is not held");
        ok(&mut db, "INSERT INTO t VALUES (1)");
        let select = "SELECT a FROM t";
        assert_eq!(ok(&mut db, select).rows, vec![vec![Value::Int4(1)]]);
        assert_eq!(hits(&db), 0);
        assert_eq!(ok(&mut db, select).rows, vec![vec![Value::Int4(1)]]);
        assert_eq!(hits(&db), 1, "the second run is a hit");
        let held = db.plan_cache.len();
        ok(&mut db, "CREATE INDEX t_a ON t (a)");
        assert_eq!(db.plan_cache.len(), held, "a DDL plan is not held");
        assert_eq!(ok(&mut db, select).rows, vec![vec![Value::Int4(1)]]);
        assert_eq!(hits(&db), 1, "a DDL between: a miss");
        assert_eq!(db.plan_cache.len(), 1, "the change emptied the cache");
    }

    /// A held plan of a FROM-clause `generate_series` keeps its arguments as expressions:
    /// the same text bound to other values answers each, the bound refusal included, and
    /// a value under the bound after one over it answers again.
    ///
    /// Red with the series, its length or its bound folded into the plan.
    #[test]
    fn a_held_series_answers_each_bound_value() {
        let mut db = Database::with_btree();
        let sql = "SELECT count(*), max(g) FROM generate_series(1, $1) g";
        let answer = |db: &mut Database, n: i32| run(db, sql, 0, &[Value::Int4(n)]).map(|r| r.rows);
        assert_eq!(
            answer(&mut db, 3),
            Ok(vec![vec![Value::Int8(3), Value::Int4(3)]])
        );
        assert_eq!(
            answer(&mut db, 5),
            Ok(vec![vec![Value::Int8(5), Value::Int4(5)]])
        );
        assert_eq!(hits(&db), 1, "the second run is a hit");
        assert!(
            matches!(answer(&mut db, 2_000_000), Err(e) if e.starts_with("ERR 54000")),
            "over the bound"
        );
        assert_eq!(
            answer(&mut db, 0),
            Ok(vec![vec![Value::Int8(0), Value::Null]])
        );
        assert_eq!(hits(&db), 3, "every later run is a hit");
    }

    /// A catalog change between two runs of one text is seen by the second: a table
    /// re-created with another column type, a block's rollback of that, a savepoint's,
    /// another connection's view of a block's change and of its rollback. The second run
    /// shows it by the type its parameter is inferred as: a plan held across an added
    /// column, a dropped index or a rolled-back one, with the generation out of the stamp,
    /// answered `SELECT *` and an indexed read as the catalog does (measured), so neither
    /// shows it. A failed `DO`'s undo and a failed DDL statement are controls: no
    /// statement of this entry point is parsed against the catalog they leave behind.
    ///
    /// Red with the generation out of the stamp.
    #[test]
    fn a_catalog_change_or_its_rollback_is_seen_by_the_same_text() {
        use pglet_sql::types::TypeOid::{Int4, Int8, Text};
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        assert_eq!(param_types(&mut db, 0), [Some(Int4)]);
        ok(&mut db, "DROP TABLE t");
        ok(&mut db, "CREATE TABLE t (a TEXT)");
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);

        // A block's rollback, read by the block's connection and another.
        ok(&mut db, "BEGIN");
        ok(&mut db, "DROP TABLE t");
        ok(&mut db, "CREATE TABLE t (a INT8)");
        assert_eq!(param_types(&mut db, 0), [Some(Int8)]);
        assert_eq!(param_types(&mut db, 1), [Some(Int8)]);
        ok(&mut db, "ROLLBACK");
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);
        assert_eq!(param_types(&mut db, 1), [Some(Text)]);

        // A savepoint's.
        ok(&mut db, "BEGIN");
        ok(&mut db, "SAVEPOINT s");
        ok(&mut db, "DROP TABLE t");
        ok(&mut db, "CREATE TABLE t (a INT8)");
        assert_eq!(param_types(&mut db, 0), [Some(Int8)]);
        ok(&mut db, "ROLLBACK TO s");
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);
        ok(&mut db, "COMMIT");
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);

        // A failed `DO`'s undo of the DDL it ran.
        let failed_do = "DO $$ BEGIN DROP TABLE t; CREATE TABLE t (a INT8); \
                         INSERT INTO nosuch VALUES (1); END $$";
        assert!(err(&mut db, failed_do, 0).starts_with("ERR 42P01"));
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);

        // A failed DDL statement changes nothing.
        assert!(err(&mut db, "ALTER TABLE t ADD COLUMN a INT", 0).starts_with("ERR 42701"));
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);
    }

    /// A rollback that changed nothing a parse reads keeps the statements held: a block
    /// that ran only DML — a draw on a sequence among it, whose counter the rollback
    /// leaves where the draw put it — and a savepoint's rollback over DML. The second run
    /// of each script is all hits and answers as the first did, the draw apart.
    ///
    /// Red with every restore moving the generation: the second run of each misses.
    #[test]
    fn a_rollback_that_changed_no_catalog_keeps_the_cache() {
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (id SERIAL, a INT)");
        let insert = "INSERT INTO t (a) VALUES (1) RETURNING id";
        let select = "SELECT a FROM t";
        let block = ["BEGIN", insert, select, "ROLLBACK"];
        let savepoint = [
            "BEGIN",
            "SAVEPOINT s",
            insert,
            "ROLLBACK TO s",
            select,
            "COMMIT",
        ];
        let generation = db.schema().generation;
        let mut id = 0;
        for script in [&block[..], &savepoint[..]] {
            for pass in 0..2 {
                let before = hits(&db);
                for sql in script {
                    let answer = ok(&mut db, sql).rows;
                    if *sql == insert {
                        id += 1;
                        assert_eq!(answer, vec![vec![Value::Int4(id)]], "the draw stands");
                    }
                }
                if pass == 1 {
                    assert_eq!(hits(&db), before + script.len(), "{script:?}: all hits");
                }
                assert_eq!(ok(&mut db, select).rows, Vec::<Vec<Value>>::new());
            }
        }
        assert_eq!(db.schema().generation, generation);
    }

    /// A rollback that took back a change a parse reads holds nothing parsed against it:
    /// the same text answers the catalog the rollback restored — a `DROP SEQUENCE` whose
    /// absence parameter inference reads, a `CREATE SEQUENCE`, and a block's and a
    /// savepoint's index swap. A statement that moved the generation and failed moves it
    /// at the rollback too.
    ///
    /// Red with every restore keeping the generation: the held plans answer the
    /// rolled-back catalog — the executor's `42809` over the sequence where inference
    /// refuses `42P18`, and a probe of the index the rollback dropped (`22000`). The
    /// failed statement's leg is red with a restore returning the snapshot's stamp.
    #[test]
    fn a_rollback_of_a_catalog_change_holds_nothing_parsed_against_it() {
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        ok(&mut db, "CREATE INDEX t_a ON t (a)");
        ok(&mut db, "INSERT INTO t VALUES (1)");

        // A sequence's absence, read by inference, and its presence.
        let sql = "SELECT 1 FROM s WHERE $1 IS NULL";
        ok(&mut db, "CREATE SEQUENCE s");
        let fresh = err(&mut db, sql, 0);
        assert!(fresh.starts_with("ERR 42P18"), "{fresh}");
        ok(&mut db, "BEGIN");
        ok(&mut db, "INSERT INTO t VALUES (2)");
        ok(&mut db, "DROP SEQUENCE s");
        assert!(err(&mut db, sql, 0).starts_with("ERR 42P01"));
        ok(&mut db, "ROLLBACK");
        assert_eq!(err(&mut db, sql, 0), fresh);
        let created = "SELECT 1 FROM s2 WHERE $1 IS NULL";
        ok(&mut db, "BEGIN");
        ok(&mut db, "INSERT INTO t VALUES (2)");
        ok(&mut db, "CREATE SEQUENCE s2");
        assert!(err(&mut db, created, 0).starts_with("ERR 42P18"));
        ok(&mut db, "ROLLBACK");
        assert!(err(&mut db, created, 0).starts_with("ERR 42P01"));

        // A block's index swap, then a savepoint's over a block that wrote before it.
        let probe = "SELECT a FROM t WHERE a = 1";
        let one = vec![vec![Value::Int4(1)]];
        ok(&mut db, "BEGIN");
        ok(&mut db, "DROP INDEX t_a");
        ok(&mut db, "CREATE INDEX t_a2 ON t (a)");
        assert_eq!(ok(&mut db, probe).rows, one);
        ok(&mut db, "ROLLBACK");
        let before = hits(&db);
        assert_eq!(ok(&mut db, probe).rows, one);
        assert_eq!(hits(&db), before, "a miss");
        ok(&mut db, "BEGIN");
        ok(&mut db, "INSERT INTO t VALUES (2)");
        ok(&mut db, "SAVEPOINT s");
        ok(&mut db, "DROP INDEX t_a");
        ok(&mut db, "CREATE INDEX t_a3 ON t (a)");
        assert_eq!(ok(&mut db, probe).rows, one);
        ok(&mut db, "ROLLBACK TO s");
        let before = hits(&db);
        assert_eq!(ok(&mut db, probe).rows, one);
        assert_eq!(hits(&db), before, "a miss");
        ok(&mut db, "ROLLBACK");

        // A statement that moved the generation and then failed.
        let generation = db.schema().generation;
        ok(&mut db, "BEGIN");
        ok(&mut db, "INSERT INTO t VALUES (3)");
        assert!(err(&mut db, "ALTER TABLE t ADD COLUMN a INT", 0).starts_with("ERR 42701"));
        ok(&mut db, "ROLLBACK");
        assert_ne!(
            db.schema().generation,
            generation,
            "the failed statement moved it"
        );
        assert_eq!(ok(&mut db, probe).rows, one);
    }

    /// The host's mutable borrow inside a block that wrote: the edit moves no
    /// generation, and a replacement sets it to whatever the new catalog holds — here
    /// the frame's own — so the rollback moves it however the stamps compare, and the
    /// same text reads the catalog the rollback restored.
    ///
    /// Red with the session not told of the borrow: each leg's rollback keeps the stamp
    /// the host's catalog was parsed under, and infers its `text`.
    #[test]
    fn a_rollback_after_the_hosts_catalog_edit_misses() {
        use pglet_sql::types::TypeOid::{Int4, Text};
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        ok(&mut db, "BEGIN");
        ok(&mut db, "INSERT INTO t VALUES (1)");
        db.schema_mut().tables.get_mut("t").unwrap().columns[0].type_oid = Text;
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);
        ok(&mut db, "ROLLBACK");
        assert_eq!(param_types(&mut db, 0), [Some(Int4)]);

        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        let generation = db.schema().generation;
        ok(&mut db, "BEGIN");
        ok(&mut db, "INSERT INTO t VALUES (1)");
        *db.schema_mut() = Schema::new();
        ok(&mut db, "CREATE TABLE t (a TEXT)");
        assert_eq!(db.schema().generation, generation, "the frame's stamp");
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);
        ok(&mut db, "ROLLBACK");
        assert_eq!(param_types(&mut db, 0), [Some(Int4)]);
    }

    /// A failed `DO`'s undo moves the generation only where its body did: the `DO`
    /// moves it once itself, as any statement that may change the catalog does, so the
    /// statements held before it are dropped either way — and a body that ran only DML
    /// adds nothing to that move, where one that ran DDL moves it past the body's.
    ///
    /// Red with every restore moving the generation (the DML body's count).
    #[test]
    fn a_failed_do_moves_the_generation_only_where_its_body_did() {
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        let generation = db.schema().generation;
        let dml = "DO $$ BEGIN INSERT INTO t VALUES (1); INSERT INTO nosuch VALUES (1); END $$";
        assert!(err(&mut db, dml, 0).starts_with("ERR 42P01"));
        assert_eq!(db.schema().generation, generation + 1, "the DO's own move");
        let generation = db.schema().generation;
        let ddl = "DO $$ BEGIN CREATE TABLE u (x INT); INSERT INTO nosuch VALUES (1); END $$";
        assert!(err(&mut db, ddl, 0).starts_with("ERR 42P01"));
        assert_eq!(
            db.schema().generation,
            generation + 3,
            "the DO's, the CREATE's, and the undo's"
        );
        assert!(err(&mut db, "SELECT x FROM u", 0).starts_with("ERR 42P01"));
        assert_eq!(
            ok(&mut db, "SELECT a FROM t").rows,
            Vec::<Vec<Value>>::new()
        );
    }

    /// The host's mutable borrow of the catalog changes it without moving the generation,
    /// or replaces it and moves the generation back — the persistent wasm binding's
    /// `clear()` — and the same text reads the catalog as the host left it.
    ///
    /// Red with `schema_mut` leaving the cache alone: each leg infers the old type.
    #[test]
    fn the_hosts_catalog_edit_empties_the_cache() {
        use pglet_sql::types::TypeOid::{Int4, Text};
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        assert_eq!(param_types(&mut db, 0), [Some(Int4)]);
        let generation = db.schema().generation;
        db.schema_mut().tables.get_mut("t").unwrap().columns[0].type_oid = Text;
        assert_eq!(
            db.schema().generation,
            generation,
            "the edit moves no generation"
        );
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);

        // The catalog replaced: the generation goes back to 0, and the next DDL takes it
        // to the number the held plan was stamped at.
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        let generation = db.schema().generation;
        assert_eq!(param_types(&mut db, 0), [Some(Int4)]);
        *db.schema_mut() = Schema::new();
        ok(&mut db, "CREATE TABLE t (a TEXT)");
        assert_eq!(
            db.schema().generation,
            generation,
            "the same stamp, another catalog"
        );
        assert_eq!(param_types(&mut db, 0), [Some(Text)]);
    }

    /// A new sequence's name is read by parameter inference, which refuses an untyped
    /// parameter over a relation that exists and stands down over one that does not: the
    /// same text answers the executor's `42P01` before `CREATE SEQUENCE` and the
    /// inference's `42P18` after it, as a database that never ran it answers.
    ///
    /// Red with `CreateSequence` out of `Plan::mutates_schema`: the second run hits the
    /// plan inferred without the sequence and answers `42809`.
    #[test]
    fn a_new_sequence_is_seen_by_the_same_texts_inference() {
        let sql = "SELECT 1 FROM s WHERE $1 IS NULL";
        let mut db = Database::with_btree();
        assert!(err(&mut db, sql, 0).starts_with("ERR 42P01"));
        ok(&mut db, "CREATE SEQUENCE s");
        let mut fresh = Database::with_btree();
        ok(&mut fresh, "CREATE SEQUENCE s");
        let answer = err(&mut fresh, sql, 0);
        assert!(answer.starts_with("ERR 42P18"), "{answer}");
        assert_eq!(err(&mut db, sql, 0), answer);
    }

    /// The thread's planner settings are part of what a parse reads: the unknown-type
    /// policy, and the statement-depth limit.
    ///
    /// Red with either out of the stamp: the coerced plan answers `'x'` where the policy
    /// now refuses `42704`, and the plan parsed under the higher limit runs where the
    /// lower one refuses `54001`.
    #[test]
    fn a_planner_setting_of_the_thread_is_part_of_the_stamp() {
        let mut db = Database::with_btree();
        let cast = "SELECT 'x'::frobnicate AS v";
        db.set_coerce_unknown_types(true);
        let coerced = ok(&mut db, cast).rows;
        db.set_coerce_unknown_types(false);
        let refused = run(&mut db, cast, 0, &[]);
        assert_eq!(coerced, vec![vec![Value::Text("x".to_owned())]]);
        assert!(
            refused.as_ref().is_err_and(|e| e.starts_with("ERR 42704")),
            "{refused:?}"
        );

        let chain = format!("SELECT 1{}", "+1".repeat(60));
        let previous = crate::set_thread_max_depth(64);
        let deep = run(&mut db, &chain, 0, &[]).map(|r| r.rows);
        crate::set_thread_max_depth(32);
        let shallow = run(&mut db, &chain, 0, &[]).map(|r| r.rows);
        crate::set_thread_max_depth(previous);
        assert_eq!(deep, Ok(vec![vec![Value::Int4(61)]]));
        assert!(
            shallow.as_ref().is_err_and(|e| e.starts_with("ERR 54001")),
            "{shallow:?}"
        );
    }

    /// A hit runs everything a miss runs but the parse — so the statement's session is
    /// installed for it, and a session function answers the connection's own setting,
    /// not the one another connection set last.
    ///
    /// Red with the lookup ahead of the ambient session's install.
    #[test]
    fn a_hit_answers_its_own_connections_setting() {
        let setting = "SELECT current_setting('application_name') AS v";
        let text = |s: &str| vec![vec![Value::Text(s.to_owned())]];
        let mut db = Database::with_btree();
        assert_eq!(ok(&mut db, setting).rows, text(""));
        run(&mut db, "SET application_name = 'other'", 1, &[]).unwrap();
        assert_eq!(run(&mut db, setting, 1, &[]).unwrap().rows, text("other"));
        assert_eq!(ok(&mut db, setting).rows, text(""), "conn 0's own");
        assert_eq!(run(&mut db, setting, 1, &[]).unwrap().rows, text("other"));
        assert_eq!(hits(&db), 3, "each run after the first was a hit");
    }

    /// The clock family answers the block a held statement runs in, not the last block a
    /// statement was parsed in — and an autocommit statement after a block its own start.
    ///
    /// Red with the lookup ahead of the ambient session's install, which is where the
    /// block's start is installed for the statement.
    #[test]
    fn a_hit_answers_its_own_blocks_clock() {
        let mut db = Database::with_btree();
        let now = "SELECT now() AS v";
        ok(&mut db, "BEGIN");
        let first = ok(&mut db, now).rows;
        ok(&mut db, "COMMIT");
        std::thread::sleep(std::time::Duration::from_millis(2));
        ok(&mut db, "BEGIN");
        let hits_before = hits(&db);
        let second = ok(&mut db, now).rows;
        let again = ok(&mut db, now).rows;
        assert_eq!(hits(&db), hits_before + 2, "both reads were hits");
        ok(&mut db, "COMMIT");
        assert_ne!(first, second, "a later block's start");
        assert_eq!(second, again, "one block, one start");
        let agree = "SELECT now() = statement_timestamp() AS v";
        ok(&mut db, "BEGIN");
        ok(&mut db, agree);
        ok(&mut db, "COMMIT");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let hits_before = hits(&db);
        assert_eq!(ok(&mut db, agree).rows, vec![vec![Value::Bool(true)]]);
        assert_eq!(hits(&db), hits_before + 1, "a hit");
    }

    /// A held statement in an aborted block is refused by the gate as any statement is,
    /// its `COMMIT` is the block's rollback, and a binding failure on a hit fails the
    /// block.
    ///
    /// Red with the lookup ahead of the gate.
    #[test]
    fn a_hit_in_an_aborted_block_is_refused() {
        let mut db = Database::with_btree();
        ok(&mut db, "CREATE TABLE t (a INT)");
        let select = "SELECT a FROM t";
        ok(&mut db, select);
        ok(&mut db, "BEGIN");
        ok(&mut db, "COMMIT");
        ok(&mut db, "BEGIN");
        assert!(err(&mut db, "SELECT 1/0", 0).starts_with("ERR 22012"));
        assert!(err(&mut db, select, 0).starts_with("ERR 25P02"));
        assert_eq!(ok(&mut db, "COMMIT").command_tag, "ROLLBACK");
        assert_eq!(db.transaction_status(), crate::TransactionStatus::Idle);

        // A binding failure on a hit fails the open block.
        let bound = "SELECT a FROM t WHERE a = $1";
        run(&mut db, bound, 0, &[Value::Int4(1)]).unwrap();
        ok(&mut db, "BEGIN");
        let failed = db.execute_bound(bound, 0, |_| Err::<Vec<Option<Value>>, _>(()));
        assert!(matches!(failed, Err(BoundExecError::Bind(()))));
        assert!(err(&mut db, select, 0).starts_with("ERR 25P02"));
        ok(&mut db, "ROLLBACK");
    }

    /// A fork parses its own statements: it starts with nothing held, and a catalog
    /// change on either side — which can take both to one generation — leaves the other's
    /// answers alone.
    ///
    /// Red with the fork handed its parent's statements (the count) and with one cache
    /// shared by the lineage (the base infers the fork's type).
    #[test]
    fn a_fork_holds_its_own_statements() {
        use pglet_sql::types::TypeOid::{Int4, Text};
        let mut base = Database::with_btree();
        ok(&mut base, "CREATE TABLE t (a INT)");
        assert_eq!(param_types(&mut base, 0), [Some(Int4)]);
        let mut fork = base.fork().unwrap();
        assert_eq!(fork.plan_cache.len(), 0, "a fork starts with nothing held");
        ok(&mut fork, "DROP TABLE t");
        ok(&mut fork, "CREATE TABLE t (a TEXT)");
        assert_eq!(param_types(&mut fork, 0), [Some(Text)]);
        ok(&mut base, "CREATE INDEX t_a ON t (a)");
        ok(&mut base, "DROP INDEX t_a");
        assert_eq!(base.schema().generation, fork.schema().generation);
        assert_eq!(param_types(&mut base, 0), [Some(Int4)]);
        assert_eq!(param_types(&mut fork, 0), [Some(Text)]);
    }

    /// A full cache is emptied and goes on holding: a statement run after more distinct
    /// texts than the bound is still held, and the cache never holds more than it.
    ///
    /// Red with a full cache refusing new entries (the last text never hits) or never
    /// emptied (the count).
    #[test]
    fn a_full_cache_goes_on_holding() {
        let mut db = Database::with_btree();
        for n in 0..=super::CAP * 2 {
            let sql = format!("SELECT {n} AS v");
            assert_eq!(
                ok(&mut db, &sql).rows,
                vec![vec![Value::Int4(n.try_into().unwrap())]]
            );
            assert!(db.plan_cache.len() <= super::CAP);
        }
        let before = hits(&db);
        ok(&mut db, "SELECT 'last' AS v");
        ok(&mut db, "SELECT 'last' AS v");
        assert_eq!(hits(&db), before + 1);
    }

    /// A stack budget is something a parse reads (natively): a text held under the
    /// thread's whole stack answers under a narrower budget what a database that never ran
    /// it answers — refused `54001` where its parse does not fit, though its plan, a bare
    /// literal under parentheses, runs in any of these budgets.
    ///
    /// Red with `set_stack_budget` leaving the cache alone: the held plan answers.
    #[test]
    fn a_narrower_stack_budget_empties_the_cache() {
        let mut refusals = 0;
        for n in [20usize, 80] {
            let nested = format!("SELECT {}1{} AS v", "(".repeat(n), ")".repeat(n));
            for kib in [64usize, 128, 256, 512, 1024] {
                let mut db = Database::with_btree();
                assert!(
                    run(&mut db, &nested, 0, &[]).is_ok(),
                    "{n}: the thread's stack"
                );
                db.set_stack_budget(Some(kib * 1024));
                let held = run(&mut db, &nested, 0, &[]).map(|r| r.rows);
                let mut fresh = Database::with_btree();
                fresh.set_stack_budget(Some(kib * 1024));
                let parsed = run(&mut fresh, &nested, 0, &[]).map(|r| r.rows);
                assert_eq!(held, parsed, "{n} under {kib} KiB");
                refusals += usize::from(parsed.is_err_and(|e| e.starts_with("ERR 54001")));
            }
        }
        assert!(refusals > 0, "some budget refuses the parse");
    }

    /// Run `f` on a new thread of `stack` bytes, declared to the byte check.
    fn on_declared_thread<T: Send + 'static>(
        stack: usize,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || {
                crate::set_thread_stack(stack);
                f()
            })
            .unwrap()
            .join()
            .unwrap()
    }

    /// A plan deeper than a default thread's limit is run but not held: a host that
    /// raised its thread's limit can move the database to a default thread, where
    /// emptying the cache would drop the plan. Measured without this: a 120 000-term
    /// chain held on a declared 1 GiB thread overflowed a 2 MiB one at its next statement.
    ///
    /// Red with every plan held (the count).
    #[test]
    fn a_plan_deeper_than_a_default_thread_admits_is_not_held() {
        use pglet_sql::depth::FALLBACK_MAX_DEPTH;
        let mut db = on_declared_thread(256 << 20, || {
            crate::set_thread_max_depth(FALLBACK_MAX_DEPTH * 4);
            let mut db = Database::with_btree();
            let deep = format!("SELECT 1{} AS v", "+1".repeat(FALLBACK_MAX_DEPTH * 2));
            let answer = run(&mut db, &deep, 0, &[]).map(|r| r.rows);
            let terms = i32::try_from(FALLBACK_MAX_DEPTH * 2 + 1).unwrap();
            assert_eq!(answer, Ok(vec![vec![Value::Int4(terms)]]));
            assert_eq!(db.plan_cache.len(), 0, "run, not held");
            ok(&mut db, "SELECT 1 AS v");
            assert_eq!(db.plan_cache.len(), 1, "a shallow plan is held");
            db
        });
        let answer = on_declared_thread(2 << 20, move || ok(&mut db, "SELECT 2 AS v").rows);
        assert_eq!(answer, vec![vec![Value::Int4(2)]]);
    }

    /// A long text is run and not held, and the held texts never total more than the
    /// budget, however many distinct texts run — while a short statement run after them
    /// is still held.
    ///
    /// Red with the length cut gone (the long text is held) or the budget gone (the total
    /// passes it).
    #[test]
    fn what_the_cache_holds_is_bounded_by_its_text() {
        let mut db = Database::with_btree();
        let rows = |n: usize| {
            (0..n)
                .map(|i| format!("({i})"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let long = format!("SELECT count(*) AS n FROM (VALUES {}) v", rows(2000));
        assert!(long.len() > super::MAX_TEXT_BYTES);
        assert_eq!(ok(&mut db, &long).rows, vec![vec![Value::Int8(2000)]]);
        assert_eq!(db.plan_cache.len(), 0, "a long text is not held");
        for n in 0..200 {
            // Distinct texts under the cut.
            let sql = format!("SELECT count(*) AS n FROM (VALUES {}) v -- {n}", rows(500));
            assert!(sql.len() <= super::MAX_TEXT_BYTES, "{}", sql.len());
            ok(&mut db, &sql);
            assert!(db.plan_cache.text_held() <= super::HELD_TEXT_BUDGET);
        }
        let before = hits(&db);
        ok(&mut db, "SELECT 1 AS v");
        ok(&mut db, "SELECT 1 AS v");
        assert_eq!(hits(&db), before + 1, "still holding");
    }

    /// `SELECT ((…1…)) AS v`, `n` levels: a plan of one literal whose parse recurses `n`
    /// deep.
    fn nested(n: usize) -> String {
        format!("SELECT {}1{} AS v", "(".repeat(n), ")".repeat(n))
    }

    /// The thread's stack is part of what a parse reads natively: a text held on a thread
    /// declared large answers, after the thread declares a smaller stack, what a database
    /// that never ran it answers — refused `54001` where its parse does not fit, though its
    /// plan runs.
    ///
    /// Red with the thread's stack out of the stamp.
    #[test]
    fn a_smaller_declared_stack_is_part_of_the_stamp() {
        let refusals = on_declared_thread(8 << 20, move || {
            let mut refusals = 0;
            for n in [10usize, 20, 80] {
                for kib in [850usize, 900, 1000, 1200, 1500] {
                    crate::set_thread_stack(8 << 20);
                    let mut db = Database::with_btree();
                    assert!(run(&mut db, &nested(n), 0, &[]).is_ok(), "{n}: 8 MiB");
                    crate::set_thread_stack(kib * 1024);
                    let held = run(&mut db, &nested(n), 0, &[]).map(|r| r.rows);
                    let parsed =
                        run(&mut Database::with_btree(), &nested(n), 0, &[]).map(|r| r.rows);
                    assert_eq!(held, parsed, "{n} on {kib} KiB declared");
                    refusals += usize::from(parsed.is_err());
                }
            }
            refusals
        });
        assert!(refusals > 0, "some declared stack refuses the parse");
    }

    /// The same, for the database moved to a smaller thread.
    ///
    /// Red with the thread's stack out of the stamp.
    #[test]
    fn a_smaller_thread_is_part_of_the_stamp() {
        let mut refusals = 0;
        for n in [10usize, 20, 80] {
            for kib in [900usize, 1000, 1500] {
                let text = nested(n);
                let mut db = on_declared_thread(8 << 20, {
                    let text = text.clone();
                    move || {
                        let mut db = Database::with_btree();
                        assert!(run(&mut db, &text, 0, &[]).is_ok(), "{n}: 8 MiB");
                        db
                    }
                });
                let (held, parsed) = on_declared_thread(kib * 1024, move || {
                    let held = run(&mut db, &text, 0, &[]).map(|r| r.rows);
                    let parsed = run(&mut Database::with_btree(), &text, 0, &[]).map(|r| r.rows);
                    (held, parsed)
                });
                assert_eq!(held, parsed, "{n} moved to {kib} KiB");
                refusals += usize::from(parsed.is_err());
            }
        }
        assert!(refusals > 0, "some smaller thread refuses the parse");
    }
}
