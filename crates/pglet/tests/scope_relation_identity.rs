//! Every scope that has a relation carries its name, and the correlated outer-scope chain
//! binds through that name — over the embed engine (parse → plan → execute) on the
//! **default** CoW-B+tree backend.
//!
//! Two resolvers switch behaviour on a scope with no relation identity —
//! `SimpleColumns::resolve`'s last tier drops the qualifier and matches the bare name,
//! and `EvalEnv::resolve_outer_slot` treats the frame as addressing several relations by
//! the synthesized `"relation.column"` names a user's quoted `"a.b"` column is
//! byte-identical to — so a scope that *has* a relation and reports `None` answers for
//! relations the statement never named. Two call sites did: `execute_update` passed the
//! statement's FROM *alias* (so an unaliased target passed nothing) and
//! `single_table_name` had no `Plan::FunctionScan` arm. The chain then had two rules of its own
//! wrong, both of which wrote: a qualifier no frame owned fell to a bare match, and an
//! ambiguity at the claiming level escalated outward. Those are the second block below.
//!
//! **These tests exist beside the differential fixtures rather than instead of them, and
//! the reason is the assertion the fixtures cannot carry.** A refusal fixture is
//! `@expect-error`, which compares the SQLSTATE and nothing else — it is green for a
//! refusal raised *after* the rows are written. What is asserted here is that the target
//! rows are untouched. The reject SQLSTATEs themselves are pinned differentially
//! (`tests/fixtures/update/correlated_outer_ref_unaliased_target_loud.sql` and its three
//! siblings), since only a live 18.6 can say they are Postgres's.
//!
//! **Two things about what that buys, so the next reader does not over-read a green
//! run**, and they apply to the *first* block only — the second block's members raised
//! nothing at all before, so there the `is_err()` pin and the row assertion redden
//! together. The row assertions in the three UPDATE tests are *not* what reddens under
//! the regression those tests were written for. Each sits behind an `is_err()` pin in the same
//! test, and an `assert!` panics — so a regression that lets the statement succeed stops
//! at the pin and the row assertion never runs. Reaching a row assertion at all means the
//! statement was refused, so what those assertions guard is a *different* failure: a
//! refusal placed after the writes. That is not today's placement, and it is a shape this
//! engine has shipped before — `docs/scope.md` records a `RETURNING` refusal that was raised
//! from inside `execute_update` after the writes — which is why the assertions are
//! kept rather than trimmed. What being wrong there costs is backend-shaped:
//! `EDGES.md` records that `MemoryStorage` has no autocommit statement
//! rollback, so a write applied before a refusal stands. But a green run of these four is
//! not evidence about that placement.
//!
//! And the **backend is not discriminating** for these four. Measured through a server on
//! each backend, accelerated profile, simple-query text leg, with the same statements and
//! seeds: `--storage memory` (`MemoryStorage`, what `Database::in_memory` selects) answers
//! as the default CoW-B+tree does on every statement these tests issue — the refusals
//! raised, the rows intact, the accept polarities answering. The reason, read off the
//! source rather than measured per statement, is that the UPDATE refusals are raised
//! inside `collect_updated_simple` / `scan_matching_rows`, which write nothing, so the
//! backends' differing rollback behaviour has nothing to roll back. `with_btree` is used
//! here because it is the default backend, not because a guard on the other would be
//! vacuous.

// A test harness: setup `.unwrap()`s and the `rows` helper `.expect()`s, so a setup
// failure surfaces loudly here rather than as a confusing assertion mismatch.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pglet::Database;
use pglet_sql::types::Value;

fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(results.len(), 1, "expected one result set for {sql:?}");
    results.remove(0).rows
}

/// Seed `u1(id, "a.b")` = `(1,'hit'), (2,'miss')` beside a one-row `u2(yv)` = `('hit')`.
///
/// `u1` deliberately carries **no bare `b`**, because these tests pin the *identity* rule
/// and not the whole class: while a qualifier no frame owned fell to a bare match, adding
/// an ordinary `b` beside the quoted column let the reference escape and destroy a row
/// anyway. That escape is closed and is the second block's subject — `chw`, seeded there,
/// carries no quoted column at all and reaches the same class through a plain typo.
fn seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute(r#"CREATE TABLE u1 (id INT, "a.b" TEXT)"#)
        .unwrap();
    db.execute("CREATE TABLE u2 (yv TEXT)").unwrap();
    db.execute("INSERT INTO u1 VALUES (1, 'hit'), (2, 'miss')")
        .unwrap();
    db.execute("INSERT INTO u2 VALUES ('hit')").unwrap();
    db
}

fn u1_ids(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT id FROM u1 ORDER BY id")
}

/// **The discriminating assertion — keep it first.** An unaliased UPDATE's scope names
/// the target, so a correlated `a.b` names a relation the statement does not have and the
/// statement is refused **with both rows intact**. It used to update one of them.
#[test]
fn an_unaliased_update_refuses_a_foreign_qualifier_without_writing() {
    let mut db = seeded();
    let err = db.execute("UPDATE u1 SET id = 99 WHERE EXISTS (SELECT 1 FROM u2 WHERE u2.yv = a.b)");
    assert!(
        err.is_err(),
        "a qualifier the statement does not name must be loud"
    );
    assert_eq!(
        u1_ids(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede every write — this statement used to store 99"
    );
}

/// The same statement **aliased** was refused before the fix and must stay refused: it is
/// the control that identifies the missing identity rather than the statement kind, and
/// it is also what rejects the obvious wrong fix (pass the table name always), since an
/// alias *hides* the table name.
///
/// **Read the second assertion for exactly what it is.** It pins the `eref` rule in the
/// `SimpleColumns` spelling, where a qualifier the scope does not name is tier 3's
/// `42P01`. The *correlated* spelling of the same rule used to be unenforced, and it was a
/// write — `UPDATE hw x SET id = 99 WHERE EXISTS (SELECT 1 FROM u2b WHERE u2b.yv = hw.v)`
/// wrote `99` over the `hit` row where 18.6 is `42P01 "hw"`, hinting the alias `x`. It is
/// refused now, and its row assertion is
/// `a_correlated_reference_past_an_alias_writes_nothing` below; this test therefore says
/// the rule holds on one path, and that one says it holds on the other.
#[test]
fn an_alias_hides_the_table_name_and_still_refuses_a_foreign_qualifier() {
    let mut db = seeded();
    assert!(
        db.execute("UPDATE u1 x SET id = 77 WHERE EXISTS (SELECT 1 FROM u2 WHERE u2.yv = a.b)")
            .is_err(),
        "the aliased spelling was already loud and must remain so"
    );
    assert!(
        db.execute("UPDATE u1 x SET id = 77 WHERE u1.id = 1")
            .is_err(),
        "an alias hides the table name, so `u1` names nothing here"
    );
    assert_eq!(
        u1_ids(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]]
    );
}

/// The other resolver, same missing identity, same statement kind: a qualifier naming no
/// relation used to reach `SimpleColumns`'s loose last tier and read the target's own
/// column — in the `WHERE`, which decided *which* row was written, and in the `SET`
/// right-hand side, which decided *what* was written.
#[test]
fn an_unaliased_update_refuses_a_bogus_qualifier_in_where_and_in_the_set_rhs() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ex1 (n INT, v INT)").unwrap();
    db.execute("INSERT INTO ex1 VALUES (1, 1)").unwrap();

    assert!(
        db.execute("UPDATE ex1 SET v = 42 WHERE bogusrel.n = 1")
            .is_err()
    );
    assert!(db.execute("UPDATE ex1 SET v = bogusrel.n + 100").is_err());
    assert_eq!(
        rows(&mut db, "SELECT v FROM ex1"),
        vec![vec![Value::Int4(1)]],
        "neither refusal may leave a write behind — these stored 42 and 101"
    );

    // The accept polarity of the same tier: the target's own name still qualifies.
    assert_eq!(
        rows(&mut db, "SELECT v FROM ex1 WHERE ex1.n = 1"),
        vec![vec![Value::Int4(1)]]
    );
    db.execute("UPDATE ex1 SET v = ex1.n + 100 WHERE ex1.n = 1")
        .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT v FROM ex1"),
        vec![vec![Value::Int4(101)]]
    );
}

/// A FROM-clause `unnest(…)` names a relation (Postgres's `chooseScalarFunctionAlias`:
/// the explicit alias, else `unnest`), so its scope carries that name. A read rather than
/// a write, which is why it is the member that puts the UPDATE above in a class.
#[test]
fn an_unnest_from_item_carries_its_alias_as_the_scope_identity() {
    let mut db = seeded();
    assert!(
        db.execute(
            r#"SELECT (SELECT count(*) FROM u2 WHERE u2.yv = a.b) AS n
                 FROM unnest(ARRAY['hit','miss']) AS v("a.b")"#
        )
        .is_err(),
        "a correlated qualifier the unnest item is not named must be loud"
    );
    assert!(
        db.execute("SELECT zzz.b FROM unnest(ARRAY['x','y']) AS v(b)")
            .is_err(),
        "and so must one at the item's own level"
    );

    // Both accept polarities, which together reject an arm answering the literal
    // `"unnest"` instead of the alias: that value passes every reject above and fails
    // the first of these.
    assert_eq!(
        rows(
            &mut db,
            "SELECT v.b AS b FROM unnest(ARRAY['x','y']) AS v(b) ORDER BY b"
        ),
        vec![vec![Value::Text("x".into())], vec![Value::Text("y".into())]]
    );
    assert_eq!(
        rows(
            &mut db,
            "SELECT (SELECT count(*) FROM u2 WHERE u2.yv = v.b) AS n
               FROM unnest(ARRAY['hit','miss']) AS v(b)"
        ),
        vec![vec![Value::Int8(1)], vec![Value::Int8(0)]]
    );
    // The default name when no alias is written.
    assert_eq!(
        rows(&mut db, "SELECT unnest.unnest AS u FROM unnest(ARRAY['x'])"),
        vec![vec![Value::Text("x".into())]]
    );
}

// ---------------------------------------------------------------------------
// The correlated outer-scope chain: a frame that owns the qualifier stops the walk,
// and an ambiguity is final at the level that claims it.
//
// These are the members `@expect-error` cannot hold *at all* — they raised nothing
// before, so a fixture comparing SQLSTATEs had no pglet error to compare and a value
// comparison was red because 18.6 refuses. They raise now, so each also has a fixture
// pinning that the SQLSTATE is Postgres's; what stays here is the assertion no fixture
// carries either way, that **the rows are untouched**. `@expect-error` is green for a
// refusal raised after the writes, and this engine has shipped that placement before.
//
// Backend: `with_btree`, the default. It is not discriminating for the refusals — none
// of them reaches a write path — but it is discriminating for what a *misplaced* refusal
// would cost, since `MemoryStorage` has no autocommit statement rollback and would leave
// a pre-refusal write standing (`EDGES.md`).
// ---------------------------------------------------------------------------

/// Seed `chw(id, v)` = `(1,'hit'), (2,'miss')` beside a one-row `cu(yv)` = `('hit')`.
///
/// No quoted column anywhere: this pair is what shows the class needs no exotic schema —
/// a misspelled qualifier reaches it over two ordinary tables.
fn correlated_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE chw (id INT, v TEXT)").unwrap();
    db.execute("CREATE TABLE cu (yv TEXT)").unwrap();
    db.execute("INSERT INTO chw VALUES (1, 'hit'), (2, 'miss')")
        .unwrap();
    db.execute("INSERT INTO cu VALUES ('hit')").unwrap();
    db
}

fn chw_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT id, v FROM chw ORDER BY id")
}

fn chw_intact() -> Vec<Vec<Value>> {
    vec![
        vec![Value::Int4(1), Value::Text("hit".into())],
        vec![Value::Int4(2), Value::Text("miss".into())],
    ]
}

/// **The headline write of the class — keep it first.** A qualifier no frame owns used to
/// fall into a bare loop that never read it, so `zzz.v` resolved to the target's own `v`
/// and this was `DELETE 1`. 18.6: `42P01 "zzz"`, both rows surviving.
#[test]
fn a_correlated_bogus_qualifier_deletes_nothing() {
    let mut db = correlated_seed();
    assert!(
        db.execute("DELETE FROM chw WHERE EXISTS (SELECT 1 FROM cu WHERE cu.yv = zzz.v)")
            .is_err(),
        "a qualifier no frame owns must be loud"
    );
    assert_eq!(
        chw_rows(&mut db),
        chw_intact(),
        "the refusal must precede every write — this statement used to remove row 1"
    );

    // The accept polarity, which is what says the refusal is about the qualifier and not
    // about correlated `EXISTS`: the same statement naming the target resolves and writes.
    db.execute("DELETE FROM chw WHERE EXISTS (SELECT 1 FROM cu WHERE cu.yv = chw.v)")
        .unwrap();
    assert_eq!(
        chw_rows(&mut db),
        vec![vec![Value::Int4(2), Value::Text("miss".into())]]
    );
}

/// The `eref` rule on the correlated path: an alias hides the table name, so once `chw`
/// is aliased the qualifier `chw` names nothing. This wrote `99` over the `hit` row.
#[test]
fn a_correlated_reference_past_an_alias_writes_nothing() {
    let mut db = correlated_seed();
    assert!(
        db.execute("UPDATE chw x SET id = 99 WHERE EXISTS (SELECT 1 FROM cu WHERE cu.yv = chw.v)")
            .is_err(),
        "an alias hides the table name on the correlated path too"
    );
    assert_eq!(chw_rows(&mut db), chw_intact());

    // The alias itself still resolves, which rejects the wrong fix of refusing every
    // qualifier an aliased statement carries.
    db.execute("UPDATE chw x SET id = 99 WHERE EXISTS (SELECT 1 FROM cu WHERE cu.yv = x.v)")
        .unwrap();
    assert_eq!(
        chw_rows(&mut db),
        vec![
            vec![Value::Int4(2), Value::Text("miss".into())],
            vec![Value::Int4(99), Value::Text("hit".into())],
        ]
    );
}

/// An `INSERT … SELECT` carrying the same reference: the refusal has to land while the
/// source query runs, so nothing is inserted. This inserted a row.
#[test]
fn a_correlated_bogus_qualifier_inserts_nothing() {
    let mut db = correlated_seed();
    db.execute("CREATE TABLE cdst (id INT, v TEXT)").unwrap();
    assert!(
        db.execute(
            "INSERT INTO cdst SELECT id, v FROM chw WHERE EXISTS (SELECT 1 FROM cu WHERE cu.yv = zzz.v)"
        )
        .is_err()
    );
    assert_eq!(
        rows(&mut db, "SELECT id FROM cdst"),
        Vec::<Vec<Value>>::new(),
        "the refusal must precede the insert — this stored row 1"
    );
}

/// **The stop-at-owner write.** The owner is `sow_a` and lacks `v`; the intervening frame
/// `b` has one, so the reference used to rebind to it and this removed *every* row. 18.6
/// answers `42703 column sow_a.v does not exist`, hinting `b.v`.
///
/// Two levels of `EXISTS` are load-bearing: with one outer frame the reference finds
/// nothing and the statement was already refused, so a single-level test is green either
/// way.
#[test]
fn an_owning_frame_stops_the_walk_before_a_write() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE sow_a (id INT)").unwrap();
    db.execute("CREATE TABLE sow_b (v INT)").unwrap();
    db.execute("CREATE TABLE sow_k (k INT)").unwrap();
    db.execute("INSERT INTO sow_a VALUES (1), (2)").unwrap();
    db.execute("INSERT INTO sow_b VALUES (7)").unwrap();
    db.execute("INSERT INTO sow_k VALUES (7)").unwrap();

    let survivors = vec![vec![Value::Int4(1)], vec![Value::Int4(2)]];
    assert!(
        db.execute(
            "DELETE FROM sow_a WHERE EXISTS (SELECT 1 FROM sow_b b
               WHERE EXISTS (SELECT 1 FROM sow_k WHERE sow_k.k = sow_a.v))"
        )
        .is_err()
    );
    assert_eq!(
        rows(&mut db, "SELECT id FROM sow_a ORDER BY id"),
        survivors,
        "this removed both rows, binding `sow_a.v` to the inner frame's `v`"
    );
    assert!(
        db.execute(
            "UPDATE sow_a SET id = 99 WHERE EXISTS (SELECT 1 FROM sow_b b
               WHERE EXISTS (SELECT 1 FROM sow_k WHERE sow_k.k = sow_a.v))"
        )
        .is_err()
    );
    assert_eq!(
        rows(&mut db, "SELECT id FROM sow_a ORDER BY id"),
        survivors,
        "and this wrote 99 over both"
    );

    // The accept polarity of the same chain: `b.v` is the reference the statement means,
    // and it still resolves through two levels.
    db.execute(
        "DELETE FROM sow_a WHERE EXISTS (SELECT 1 FROM sow_b b
           WHERE EXISTS (SELECT 1 FROM sow_k WHERE sow_k.k = b.v))",
    )
    .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT id FROM sow_a"),
        Vec::<Vec<Value>>::new()
    );
}

/// The ambiguity-escalation write: `v` is on both sides of the inner join, so 18.6 is
/// `42702` and never looks outward. pglet read the inner resolver's ambiguity as "not
/// found here" and bound the outer row's `v`, deleting it.
#[test]
fn an_inner_level_ambiguity_deletes_nothing() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE amb_o (id INT, v INT)").unwrap();
    db.execute("CREATE TABLE amb_c (id INT, v INT)").unwrap();
    db.execute("CREATE TABLE amb_d (id INT, v INT)").unwrap();
    db.execute("INSERT INTO amb_o VALUES (1, 5)").unwrap();
    db.execute("INSERT INTO amb_c VALUES (1, 1)").unwrap();
    db.execute("INSERT INTO amb_d VALUES (1, 1)").unwrap();

    assert!(
        db.execute(
            "DELETE FROM amb_o WHERE EXISTS
               (SELECT 1 FROM amb_c JOIN amb_d ON amb_c.id = amb_d.id WHERE v = 5)"
        )
        .is_err(),
        "an ambiguity at the claiming level is final"
    );
    assert_eq!(
        rows(&mut db, "SELECT id FROM amb_o"),
        vec![vec![Value::Int4(1)]],
        "this removed the row, having bound the ambiguous `v` to the outer frame"
    );

    // The control that identifies the ambiguity rather than the correlation: with one
    // inner relation the bare `v` is unambiguous, resolves inward, and matches nothing.
    db.execute("DELETE FROM amb_o WHERE EXISTS (SELECT 1 FROM amb_c WHERE v = 5)")
        .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT id FROM amb_o"),
        vec![vec![Value::Int4(1)]]
    );
}

/// The `ON CONFLICT DO UPDATE` pair, where a mis-bound qualifier does not merely fail to
/// refuse — it **stores the excluded value**, which is the branch a correct engine
/// refuses to compile. Both spellings reached the `excluded` frame through the bare loop.
#[test]
fn on_conflict_do_update_refuses_a_bogus_qualifier_without_storing_excluded() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE occ (id INT PRIMARY KEY, v TEXT)")
        .unwrap();
    db.execute("INSERT INTO occ VALUES (1, 'orig')").unwrap();
    let orig = vec![vec![Value::Int4(1), Value::Text("orig".into())]];

    assert!(
        db.execute("INSERT INTO occ VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = zzz.v")
            .is_err()
    );
    assert_eq!(rows(&mut db, "SELECT id, v FROM occ"), orig, "stored 'new'");

    // A quoted `"EXCLUDED"` names no relation — the pseudo-relation is `excluded` — so
    // the case-fold rule's reject polarity lands in the same place.
    assert!(
        db.execute(
            r#"INSERT INTO occ VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = "EXCLUDED".v"#
        )
        .is_err()
    );
    assert_eq!(rows(&mut db, "SELECT id, v FROM occ"), orig, "stored 'new'");

    // The `WHERE` spelling decides *whether* the row is written, and the polarity is the
    // point: compared against the excluded value the mis-binding made the predicate true.
    assert!(
        db.execute(
            "INSERT INTO occ VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = 'x'
               WHERE zzz.v = 'new'"
        )
        .is_err()
    );
    assert_eq!(rows(&mut db, "SELECT id, v FROM occ"), orig, "stored 'x'");

    // Both accept polarities, which reject a fix that simply stops offering the frame.
    db.execute("INSERT INTO occ VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = excluded.v")
        .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT id, v FROM occ"),
        vec![vec![Value::Int4(1), Value::Text("new".into())]]
    );
    db.execute(
        "INSERT INTO occ VALUES (1, 'z') ON CONFLICT (id) DO UPDATE SET v = occ.v || '-kept'
           WHERE excluded.v = 'z'",
    )
    .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT id, v FROM occ"),
        vec![vec![Value::Int4(1), Value::Text("new-kept".into())]]
    );
}

// ---------------------------------------------------------------------------
// The `ON CONFLICT DO UPDATE` action's namespace, closed at plan time.
//
// The action is complete by construction — the target plus `excluded`, no enclosing
// level — so its name errors need not wait for a row. Until they did not, an INSERT that
// hit **no conflict** never evaluated the action and wrote its row: the same statement
// erroring or writing depending on whether the row already existed. `@expect-error`
// fixtures pin the SQLSTATEs; what is asserted here is the row count, which is the half
// the fixtures cannot see.
// ---------------------------------------------------------------------------

/// Every name error in a `DO UPDATE` action refuses the **insert** too, on a statement
/// that hits no conflict — the polarity where the action is never evaluated.
///
/// The control is the last block: with a *conflicting* key the same statements were
/// already refused, so a guard written only on that polarity says nothing about this one.
#[test]
fn a_do_update_name_error_refuses_a_non_conflicting_insert() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ocn (n INT PRIMARY KEY, v TEXT)")
        .unwrap();
    db.execute("INSERT INTO ocn VALUES (1, 'a')").unwrap();
    let one_row = vec![vec![Value::Int4(1), Value::Text("a".into())]];

    // Each of the five names the action can get wrong, all on a **new** key.
    for action in [
        "SET nosuchcol = 'z'",
        "SET v = nosuchcol",
        "SET v = 'z' WHERE nosuchcol > 0",
        "SET v = excluded.nosuchcol",
        "SET v = zzz.v",
    ] {
        let sql = format!("INSERT INTO ocn VALUES (2, 'b') ON CONFLICT (n) DO UPDATE {action}");
        assert!(db.execute(&sql).is_err(), "{action} must be loud");
        assert_eq!(
            rows(&mut db, "SELECT n, v FROM ocn ORDER BY n"),
            one_row,
            "{action} must refuse the insert, not just the update — it stored row 2"
        );
    }

    // The accept polarity, on the same new key: an action that names only what is in
    // scope inserts normally.
    db.execute(
        "INSERT INTO ocn VALUES (2, 'b') ON CONFLICT (n) DO UPDATE SET v = excluded.v
           WHERE ocn.v <> excluded.v",
    )
    .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT n, v FROM ocn ORDER BY n"),
        vec![
            vec![Value::Int4(1), Value::Text("a".into())],
            vec![Value::Int4(2), Value::Text("b".into())],
        ]
    );
    // …and the same action on a conflicting key updates.
    db.execute(
        "INSERT INTO ocn VALUES (2, 'c') ON CONFLICT (n) DO UPDATE SET v = excluded.v
           WHERE ocn.v <> excluded.v",
    )
    .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT v FROM ocn WHERE n = 2"),
        vec![vec![Value::Text("c".into())]]
    );
}

/// A target relation the catalog does not hold keeps its **own** `42P01`, which is what
/// stops the closed scope from reporting `42703` for columns of a table that is not there.
///
/// `column_type_map_for` hands back an empty map for such a relation, so a scope closed
/// over it would call every name unbound. The condition that prevents it is one
/// `table_def.is_some()`, and this is what reddens if it goes.
///
/// **The three spellings are chosen, not illustrative.** A *bogus qualifier* (`zzz.v`) is
/// `42P01` under a closed scope too — `zzz` names no relation either way — so it is green
/// with or without the condition and belongs in no guard for it. What discriminates is a
/// name the closed scope would look up in an empty column map: a bare one, and the two
/// qualified by relations the scope does register (the target and `excluded`).
#[test]
fn a_missing_on_conflict_target_reports_the_table_and_not_its_columns() {
    let mut db = Database::with_btree();
    for action in ["SET v = anycol", "SET v = ocnope.v", "SET v = excluded.v"] {
        let sql = format!("INSERT INTO ocnope VALUES (1, 'a') ON CONFLICT (n) DO UPDATE {action}");
        let message = db
            .execute(&sql)
            .expect_err("a missing target must be loud")
            .to_string();
        assert!(
            message.contains("ocnope"),
            "{action}: the error must name the relation, got: {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// A sub-plan's `ORDER BY` / `DISTINCT ON` key is the third reader whose refusal the
// executor cannot stand in for, and it belongs here for the same reason as the block
// above: the plan-time gate is the only judge a statement matching **no row** ever
// reaches, so its absence is a write.
//
// `resolve_correlated_subplans` re-walks a nested level's `Plan::Sort` /
// `Plan::DistinctOn` keys against a scope that used to decline to judge every bare name
// — the exemption Postgres reserves for a key naming an *output* column
// (`findTargetlistEntrySQL92`). A key naming neither an output column nor a FROM one
// then survived plan time, and the executor never evaluated it, so the statement ran.
//
// Backend: `with_btree`, the default. The refusals below are raised at plan time, ahead
// of any write path, so the backends do not differ — what a *misplaced* one would cost
// does, since `MemoryStorage` has no autocommit statement rollback.
// ---------------------------------------------------------------------------

/// Seed `skt(a, v)` = `(1,1), (2,2)` beside an **empty** `sku(a)`.
///
/// The emptiness is the whole point: `a NOT IN (SELECT a FROM sku …)` is true for every
/// row of `skt`, so a statement that survives the gate matches everything — and the
/// subquery's own `ORDER BY` key is never evaluated, since there is no inner row.
fn sort_key_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE skt (a INT, v INT)").unwrap();
    db.execute("CREATE TABLE sku (a INT)").unwrap();
    db.execute("INSERT INTO skt VALUES (1, 1), (2, 2)").unwrap();
    db
}

/// The untouched target rows — shared by both sort-key seeds below, which deliberately
/// carry the same two rows so the row assertions read alike.
fn seed_rows_intact() -> Vec<Vec<Value>> {
    vec![
        vec![Value::Int4(1), Value::Int4(1)],
        vec![Value::Int4(2), Value::Int4(2)],
    ]
}

fn skt_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT a, v FROM skt ORDER BY a")
}

/// **The headline write — keep it first.** A bare sort key inside the subquery of a
/// `NOT IN` binds to nothing, and this was `DELETE 2` with no error. 18.6: `42703
/// "bogus"`, both rows surviving.
#[test]
fn a_subquery_sort_key_that_binds_to_nothing_deletes_nothing() {
    let mut db = sort_key_seed();
    assert!(
        db.execute("DELETE FROM skt WHERE a NOT IN (SELECT a FROM sku ORDER BY bogus)")
            .is_err(),
        "a sort key binding to nothing must be refused"
    );
    assert_eq!(
        skt_rows(&mut db),
        seed_rows_intact(),
        "the refusal must precede the delete"
    );
}

/// The UPDATE polarity of the same key, which wrote `99` over every row.
#[test]
fn a_subquery_sort_key_that_binds_to_nothing_updates_nothing() {
    let mut db = sort_key_seed();
    assert!(
        db.execute("UPDATE skt SET v = 99 WHERE a NOT IN (SELECT a FROM sku ORDER BY bogus)")
            .is_err(),
        "a sort key binding to nothing must be refused"
    );
    assert_eq!(
        skt_rows(&mut db),
        seed_rows_intact(),
        "the refusal must precede the update"
    );
}

/// The **accept** polarity, which is what a fix narrowed past the rule would break: the
/// exemption exists because a nested level's key may name its own level's output — an
/// alias, or a `pglet_junk_N` column the planner minted for a key the select list drops.
/// Both spellings run here, and both were `42703` before the exemption existed at all.
#[test]
fn a_subquery_sort_key_may_still_name_its_own_output_column() {
    let mut db = sort_key_seed();
    db.execute("INSERT INTO sku VALUES (7), (8)").unwrap();
    assert_eq!(
        rows(
            &mut db,
            "SELECT (SELECT count(*) AS c FROM sku GROUP BY a ORDER BY c LIMIT 1) AS by_alias,
                    (SELECT count(*) FROM sku GROUP BY a ORDER BY a DESC LIMIT 1) AS by_junk"
        ),
        vec![vec![Value::Int8(1), Value::Int8(1)]]
    );
    // A `DISTINCT ON` key the projection drops takes the same junk column, and the
    // dedup reads it above the sort.
    assert_eq!(
        rows(
            &mut db,
            "SELECT (SELECT DISTINCT ON (a) a * 10 FROM sku ORDER BY a DESC LIMIT 1) AS d"
        ),
        vec![vec![Value::Int4(80)]]
    );
}

/// Seed `ambt(a, v)` = `(1,1), (2,2)` beside `ambj(n, x)` and `ambk(n, y)`, whose bare
/// `n` each side carries.
///
/// The join is `ON false`, so the subquery answers nothing and `a NOT IN (…)` is true for
/// every row of `ambt` — and the key is never evaluated, since there is no inner row.
fn ambiguous_key_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ambt (a INT, v INT)").unwrap();
    db.execute("CREATE TABLE ambj (n INT, x INT)").unwrap();
    db.execute("CREATE TABLE ambk (n INT, y INT)").unwrap();
    db.execute("INSERT INTO ambt VALUES (1, 1), (2, 2)")
        .unwrap();
    db.execute("INSERT INTO ambj VALUES (2, 20), (1, 10)")
        .unwrap();
    db.execute("INSERT INTO ambk VALUES (1, 100), (2, 200)")
        .unwrap();
    db
}

fn ambt_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT a, v FROM ambt ORDER BY a")
}

/// **The second headline write.** The output-name match a subquery's bare sort key takes
/// counted nothing — it answered on the *first* entry carrying the name — so a name both
/// sides of a join carry resolved instead of raising, and this was `DELETE 2`. 18.6:
/// `42702 column reference "n" is ambiguous`, both rows surviving.
///
/// This is the same `42702` rule `binding_bare` already spelled for the range table, one
/// tier up in the same walk, and it was introduced by the fix for
/// `a_subquery_sort_key_that_binds_to_nothing_deletes_nothing` above — which is why both
/// live here rather than one standing in for the other.
#[test]
fn an_ambiguous_subquery_sort_key_deletes_nothing() {
    let mut db = ambiguous_key_seed();
    assert!(
        db.execute(
            "DELETE FROM ambt WHERE a NOT IN (SELECT ambj.x FROM ambj JOIN ambk ON false ORDER BY n)"
        )
        .is_err(),
        "a sort key two of the sorted input's columns claim must be refused"
    );
    assert_eq!(
        ambt_rows(&mut db),
        seed_rows_intact(),
        "the refusal must precede the delete"
    );
}

/// The UPDATE polarity of the same key.
#[test]
fn an_ambiguous_subquery_sort_key_updates_nothing() {
    let mut db = ambiguous_key_seed();
    assert!(
        db.execute(
            "UPDATE ambt SET v = 99 WHERE a NOT IN (SELECT ambj.x FROM ambj JOIN ambk ON false ORDER BY n)"
        )
        .is_err(),
        "a sort key two of the sorted input's columns claim must be refused"
    );
    assert_eq!(
        ambt_rows(&mut db),
        seed_rows_intact(),
        "the refusal must precede the update"
    );
}

/// The **accept** polarity that separates counting from refusing outright: one claimant
/// still binds. `ambj` alone carries `x`, so `ORDER BY x` inside the same subquery shape
/// resolves on both engines — a count that treated any output-name match as ambiguous
/// would break it.
#[test]
fn a_single_claimant_subquery_sort_key_still_resolves() {
    let mut db = ambiguous_key_seed();
    assert_eq!(
        rows(
            &mut db,
            "SELECT a FROM ambt WHERE a NOT IN (SELECT ambj.x FROM ambj JOIN ambk ON false ORDER BY x) ORDER BY a"
        ),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]]
    );
}

/// The **accept** polarity the claimant *count* got wrong, and the one
/// `a_single_claimant_subquery_sort_key_still_resolves` structurally cannot reach: that
/// test separates *one* matching entry from two, where this separates two **equal**
/// entries from two different ones. `findTargetlistEntrySQL92` raises `42702` only when
/// two matching entries fail `equal()`, so one column written twice resolves — its own
/// comment: "we allow SELECT a, a FROM table ORDER BY a but not SELECT a AS b, b FROM
/// table ORDER BY b".
///
/// Each spelling below reaches a different normalization and each was `42702` here on
/// its own, against 18.6's rows: a wildcard emits a **qualified** reference beside the
/// written bare one, an `AS` label rides the target entry rather than the expression, a
/// `GROUP BY` target list is a `Plan::Aggregate` and not a `Plan::Project`, and a
/// `DISTINCT ON` key reads above the sort so its projecting node sits one wrapper down.
/// The fixture polarities are `select/nested_sort_key_duplicate_output_entries_*.sql`;
/// this is the same rule at the level the write-survival tests above live on.
#[test]
fn duplicate_output_entries_of_one_expression_are_a_single_claimant() {
    let mut db = ambiguous_key_seed();
    // Every subquery is `WHERE false`, so `EXISTS` is false and `NOT EXISTS` true —
    // and the key never reaches the eval-time resolver, whose own answer over a row
    // carrying two same-named columns is a separate, row-bearing divergence.
    for subquery in [
        "SELECT *, n FROM ambj WHERE false ORDER BY n",
        "SELECT n AS n, n FROM ambj WHERE false ORDER BY n",
        "SELECT ambj.n, n FROM ambj WHERE false ORDER BY n",
        "SELECT n, n FROM ambj WHERE false GROUP BY n ORDER BY n",
        "SELECT DISTINCT ON (n) ambj.n, n FROM ambj WHERE false",
    ] {
        assert_eq!(
            rows(
                &mut db,
                &format!("SELECT a FROM ambt WHERE NOT EXISTS ({subquery}) ORDER BY a")
            ),
            vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
            "one column written twice is one claimant: {subquery}"
        );
    }
}

/// **The value-equality write.** Two target-list entries under one name are one claimant
/// only when they are `equal()`, and `equal()` is **structural** — on a `Const` it is
/// `datumIsEqual`, a `memcmp` of the datum. Reading it as pglet's own `PartialEq` on
/// `Expr` looks right and is not: two of the value types beneath `Expr` carry
/// hand-written *value*-semantic impls (`PgNumeric`, where `1.5 == 1.50`; `PgInterval`,
/// which compares Postgres's estimate, so `INTERVAL '30 days' == INTERVAL '1 mon'`), both
/// correct for SQL comparison and both wrong here. Under them these pairs folded into one
/// claimant, the `42702` was lost, and this was `DELETE 2`. 18.6 raises `42702` on each
/// and leaves both rows.
///
/// The interval pair is the sharper member: those two expressions compute **different
/// dates** (`2026-01-31 + 1 mon` is February, `+ 30 days` is March), so nothing about
/// them is "the same column written twice" — only the estimate comparison folds them.
///
/// The SQLSTATEs are pinned differentially
/// (`select/nested_sort_key_numeric_display_scale_is_not_equal.sql` and
/// `select/nested_sort_key_interval_estimate_is_not_equal.sql`); what is asserted here is
/// that the target rows survive, which an `@expect-error` fixture is green without.
#[test]
fn value_folded_output_entries_are_two_claimants_and_delete_nothing() {
    for subquery in [
        "SELECT 1.5 AS q, 1.50 AS q FROM ambj WHERE false ORDER BY q",
        "SELECT ambd.d + INTERVAL '1 mon' AS q, ambd.d + INTERVAL '30 days' AS q \
         FROM ambd WHERE false ORDER BY q",
    ] {
        let mut db = ambiguous_key_seed();
        db.execute("CREATE TABLE ambd (d DATE)").unwrap();
        db.execute("INSERT INTO ambd VALUES ('2026-01-31')")
            .unwrap();
        assert!(
            db.execute(&format!("DELETE FROM ambt WHERE NOT EXISTS ({subquery})"))
                .is_err(),
            "two entries a value comparison folds are still two claimants: {subquery}"
        );
        assert_eq!(
            ambt_rows(&mut db),
            seed_rows_intact(),
            "the refusal must precede the delete: {subquery}"
        );
    }
}

/// The **accept** polarity the test above must not be allowed to widen away: the *same*
/// literal written twice is one entry on both engines, so the structural identity has to
/// be an equality and not a rejection of every literal pair. Measured on 18.6:
/// `SELECT 1.5 AS q, 1.5 AS q … ORDER BY q` resolves, where `1.5 / 1.50` is `42702`.
#[test]
fn identical_literal_output_entries_are_a_single_claimant() {
    let mut db = ambiguous_key_seed();
    assert_eq!(
        rows(
            &mut db,
            "SELECT a FROM ambt WHERE NOT EXISTS \
             (SELECT 1.5 AS q, 1.5 AS q FROM ambj WHERE false ORDER BY q) ORDER BY a"
        ),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]]
    );
}

/// Seed `nlt(id, v)` = `(10,'A'), (20,'B'), (30,'C')` beside a one-row `nls(id)` = `(10)`.
///
/// The subquery here **matches** (unlike the two seeds above, whose inner relation is
/// empty or joined `ON false`), so the statements below are ordinary filtering DML that
/// removes or rewrites exactly one row when it runs at all — which is what makes the
/// silent write row-count-independent rather than a shape needing an empty relation.
fn negative_ordinal_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE nlt (id INT, v TEXT)").unwrap();
    db.execute("CREATE TABLE nls (id INT)").unwrap();
    db.execute("INSERT INTO nlt VALUES (10, 'A'), (20, 'B'), (30, 'C')")
        .unwrap();
    db.execute("INSERT INTO nls VALUES (10)").unwrap();
    db
}

/// The untouched target rows of [`negative_ordinal_seed`].
fn nlt_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT id, v FROM nlt ORDER BY id")
}

/// **The headline write.** A subquery's `ORDER BY -1` is a **negative position** on 18.6
/// (`42P10 ORDER BY position -1 is not in select list`, `findTargetlistEntrySQL92`), and
/// was an ordinary constant sort key here — because `parser::expr` built an
/// `Expr::Negate` for every unary minus where `doNegate` folds one over a numeric
/// constant into the constant, so the key was never an `Expr::Literal` and never reached
/// the ordinal's bounds check. The statement then ran and **destroyed a row**.
///
/// The write is row-count-independent: `nls` holds the matching `10`, so this is an
/// ordinary filtering `DELETE` and not one of the empty-relation shapes the sort-key
/// tests above need. It is also independent of the bare-name rule those tests pin — the
/// key here binds to nothing *because it is a constant*, a second and separate cause
/// reaching the same position.
#[test]
fn a_negative_ordinal_sort_key_deletes_nothing() {
    let mut db = negative_ordinal_seed();
    assert!(
        db.execute("DELETE FROM nlt WHERE id IN (SELECT id FROM nls ORDER BY -1)")
            .is_err(),
        "a negative ORDER BY position must be refused"
    );
    assert_eq!(
        nlt_rows(&mut db),
        vec![
            vec![Value::Int4(10), Value::Text("A".into())],
            vec![Value::Int4(20), Value::Text("B".into())],
            vec![Value::Int4(30), Value::Text("C".into())],
        ],
        "the refusal must precede the delete"
    );
}

/// The UPDATE polarity of the same key, which rewrote the matching row's `v`.
#[test]
fn a_negative_ordinal_sort_key_updates_nothing() {
    let mut db = negative_ordinal_seed();
    assert!(
        db.execute("UPDATE nlt SET v = 'X' WHERE id IN (SELECT id FROM nls ORDER BY -1)")
            .is_err(),
        "a negative ORDER BY position must be refused"
    );
    assert_eq!(
        nlt_rows(&mut db),
        vec![
            vec![Value::Int4(10), Value::Text("A".into())],
            vec![Value::Int4(20), Value::Text("B".into())],
            vec![Value::Int4(30), Value::Text("C".into())],
        ],
        "the refusal must precede the update"
    );
}

/// The **accept** polarity, which a fold written one step too wide would break: `-1::int`
/// is a cast node and not a constant on either engine (`TYPECAST` outranks `UMINUS`), so
/// it is an ordinary constant sort key and the statement runs. Measured on 18.6, where
/// `SELECT id FROM nls ORDER BY -99::int` answers its row.
#[test]
fn a_cast_negative_sort_key_is_not_an_ordinal_and_still_deletes() {
    let mut db = negative_ordinal_seed();
    db.execute("DELETE FROM nlt WHERE id IN (SELECT id FROM nls ORDER BY -99::int)")
        .expect("a cast key is not a position");
    assert_eq!(
        nlt_rows(&mut db),
        vec![
            vec![Value::Int4(20), Value::Text("B".into())],
            vec![Value::Int4(30), Value::Text("C".into())],
        ],
        "the matching row is removed"
    );
}

/// Seed `dnf(a)` = `(1), (2)`, the target of the no-FROM `DISTINCT ON` writes below.
///
/// The CTE those statements carry is **unreferenced**, so its body never executes and the
/// eval-time resolver never sees the key at any row count. The empty
/// `unnest(ARRAY[]::int[])` is therefore not what makes these write — the same statements
/// over `unnest(ARRAY[1,2])` and over a plain `1 AS q` removed both rows too — it is kept
/// so these differ from the read fixtures in one thing only.
fn no_from_distinct_on_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE dnf (a INT)").unwrap();
    db.execute("INSERT INTO dnf VALUES (1), (2)").unwrap();
    db
}

/// The untouched target rows of [`no_from_distinct_on_seed`].
fn dnf_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT a FROM dnf ORDER BY a")
}

/// **The no-FROM `DISTINCT ON` write.** `plan_no_from_distinct_on` resolved its ON keys
/// against a fresh `ColumnScope::default()` — empty *and open* — and never gated them, so
/// a key binding to nothing survived plan time, leaving the eval-time resolver as its only
/// judge — and an unreferenced CTE's body never runs, so nothing judged it at all. The
/// `DELETE` removed both rows where 18.6 raises `42703` and leaves them.
///
/// The SQLSTATE is pinned differentially
/// (`tests/fixtures/select/distinct_on_no_from_bogus_key_in_a_cte_delete_loud.sql`); what
/// this adds is that the refusal precedes the delete, which an `@expect-error` fixture
/// cannot distinguish from a refusal raised after it.
#[test]
fn a_no_from_distinct_on_key_binding_to_nothing_does_not_delete() {
    let mut db = no_from_distinct_on_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT DISTINCT ON (bogusdon) unnest(ARRAY[]::int[]) AS q) \
             DELETE FROM dnf"
        )
        .is_err(),
        "a no-FROM DISTINCT ON key binding to nothing must be refused"
    );
    assert_eq!(
        dnf_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The `UPDATE` polarity of the same key, which overwrote every row with `99`.
#[test]
fn a_no_from_distinct_on_key_binding_to_nothing_does_not_update() {
    let mut db = no_from_distinct_on_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT DISTINCT ON (bogusdon) unnest(ARRAY[]::int[]) AS q) \
             UPDATE dnf SET a = 99"
        )
        .is_err(),
        "a no-FROM DISTINCT ON key binding to nothing must be refused"
    );
    assert_eq!(
        dnf_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the update"
    );
}

/// The **qualified** spelling writes through the same site and is refused by a different
/// arm of the gate — a qualifier never reaches `findTargetlistEntrySQL92`, so it is judged
/// against the FROM relations, of which a no-FROM select has none (`42P01` on 18.6).
#[test]
fn a_no_from_distinct_on_qualifier_binding_to_nothing_does_not_delete() {
    let mut db = no_from_distinct_on_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT DISTINCT ON (bogusrel.z) unnest(ARRAY[]::int[]) AS q) \
             DELETE FROM dnf"
        )
        .is_err(),
        "a no-FROM DISTINCT ON qualifier no relation answers must be refused"
    );
    assert_eq!(
        dnf_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **accept** polarity, which a gate closed over the FROM relations alone would break:
/// a no-FROM select's ON key may name one of its own target-list entries, so this ordinary
/// statement still runs and still removes the matched row.
#[test]
fn a_no_from_distinct_on_key_naming_an_output_column_still_deletes() {
    let mut db = no_from_distinct_on_seed();
    db.execute("DELETE FROM dnf WHERE a IN (SELECT DISTINCT ON (q) unnest(ARRAY[1]) AS q)")
        .expect("a key naming an output column binds");
    assert_eq!(
        dnf_rows(&mut db),
        vec![vec![Value::Int4(2)]],
        "the matching row is removed"
    );
}

/// Seed `sopw(a)` = `(1), (2)` beside two empty set-op arms, the target of the set-op
/// trailing-`ORDER BY` writes below.
///
/// The arms are left empty because that is what made the **bare** spelling silent — the
/// eval-time resolver is reached only where a row reaches the Sort. It is not what makes
/// these write: the CTE those statements carry is unreferenced, so its body never
/// executes, and the qualified spelling removed both rows with a row seeded into each arm
/// as well.
fn set_op_sort_key_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE sopw (a INT)").unwrap();
    db.execute("INSERT INTO sopw VALUES (1), (2)").unwrap();
    db.execute("CREATE TABLE sopwa (a INT)").unwrap();
    db.execute("CREATE TABLE sopwb (a INT)").unwrap();
    db
}

/// The untouched target rows of [`set_op_sort_key_seed`].
fn sopw_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT a FROM sopw ORDER BY a")
}

/// **The set-op trailing-`ORDER BY` write.** `plan_set_op_order_limit` resolved those keys
/// with no `ColumnScope` at all, so a key binding to nothing survived plan time and the
/// eval-time resolver was its only judge — which an unreferenced CTE's body never reaches.
/// The `DELETE` removed both rows where 18.6 raises `42703` and leaves them.
///
/// The SQLSTATE is pinned differentially
/// (`tests/fixtures/select/set_op_order_by_key_in_a_cte_delete_loud.sql`); what this adds
/// is that the refusal precedes the delete, which an `@expect-error` fixture cannot
/// distinguish from a refusal raised after it.
#[test]
fn a_set_op_sort_key_binding_to_nothing_does_not_delete() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY bogus) \
             DELETE FROM sopw"
        )
        .is_err(),
        "a set-op sort key binding to nothing must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **qualified** spelling, refused by a different arm of the gate — a qualifier never
/// reaches `findTargetlistEntrySQL92`, and a set-op's trailing keys reach no range table
/// at all, so any qualifier is `42P01` on 18.6. This one was silent at **every** row
/// count, which is why no seeded arm would have caught it.
#[test]
fn a_set_op_sort_key_with_a_bogus_qualifier_does_not_update() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY bogusrel.a) \
             UPDATE sopw SET a = 99"
        )
        .is_err(),
        "a set-op sort key with a qualifier no relation answers must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the update"
    );
}

/// The **subquery-keyed** write, which is the shape that reaches the gate without a `WITH`
/// at all: the set op is the `NOT IN` source, so `sopw`'s rows are matched against an empty
/// result and every one of them was removed.
#[test]
fn a_set_op_sort_key_in_a_not_in_source_does_not_delete() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "DELETE FROM sopw WHERE a NOT IN \
             (SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY bogusrel.a)"
        )
        .is_err(),
        "a set-op sort key with a bogus qualifier must be refused inside a NOT IN source"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **paren-leading** chain writes through the same site through a different parser
/// arm, which is why the gate keys on the tree being a set op rather than on the caller.
#[test]
fn a_paren_leading_set_op_sort_key_does_not_delete() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "WITH c AS ((SELECT a FROM sopwa) UNION SELECT a FROM sopwb ORDER BY bogus) \
             DELETE FROM sopw"
        )
        .is_err(),
        "a paren-leading set-op chain's sort key binding to nothing must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **accept** polarity, which a gate over anything but the set-op's own output names
/// would break: `a` is the output column name the left arm gives, so this ordinary
/// statement still runs and still removes the matched row.
#[test]
fn a_set_op_sort_key_naming_an_output_column_still_deletes() {
    let mut db = set_op_sort_key_seed();
    db.execute("INSERT INTO sopwa VALUES (1)").unwrap();
    db.execute(
        "DELETE FROM sopw WHERE a IN \
         (SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY a)",
    )
    .expect("a key naming an output column binds");
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(2)]],
        "the matching row is removed"
    );
}

/// The **paren-leading plain select** accept polarity — the same parser arm as
/// `a_paren_leading_set_op_sort_key_does_not_delete`, with no set op, whose sort clause
/// 18.6 folds into the inner query so its key may name an inner FROM column the projection
/// drops. Gating per caller rather than per tree would refuse this and remove nothing.
#[test]
fn a_paren_leading_plain_select_sort_key_may_name_an_inner_column() {
    let mut db = set_op_sort_key_seed();
    db.execute("CREATE TABLE sopwt (a INT, b INT)").unwrap();
    db.execute("INSERT INTO sopwt VALUES (1, 9), (2, 8)")
        .unwrap();
    db.execute("WITH c AS ((SELECT a FROM sopwt) ORDER BY b) DELETE FROM sopw WHERE a = 1")
        .expect("a paren-leading plain select's key may name an inner FROM column");
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(2)]],
        "the matching row is removed"
    );
}

/// Seed `sopl(a)` = `(1), (2)` beside an empty `sople(k)`, the target of the no-FROM
/// set-op **leaf** writes below.
///
/// These are row-count independent: the CTE is unreferenced, so its body never executes,
/// and the class needs no key at all — a plain correlated reference in the leaf rides it.
fn set_op_no_from_leaf_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE sopl (a INT)").unwrap();
    db.execute("INSERT INTO sopl VALUES (1), (2)").unwrap();
    db.execute("CREATE TABLE sople (k INT)").unwrap();
    db
}

/// The untouched target rows of [`set_op_no_from_leaf_seed`].
fn sopl_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT a FROM sopl ORDER BY a")
}

/// **The no-FROM set-op leaf write.** `finalize_leaf` lowered a `SelectLeaf::NoFrom`
/// operand as `Ok(*plan)` where its `parse_select_inner` twin runs the correlated pass, so
/// a level inside such a leaf kept its open scope and nothing ever closed it. The `DELETE`
/// removed both rows where 18.6 raises `42703` and leaves them.
///
/// The SQLSTATE is pinned differentially
/// (`tests/fixtures/select/set_op_no_from_leaf_key_in_a_cte_delete_loud.sql`).
#[test]
fn a_set_op_no_from_leaf_bogus_key_does_not_delete() {
    let mut db = set_op_no_from_leaf_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT 1 AS z UNION ALL SELECT (SELECT DISTINCT ON (bogusdon) 1 AS q)) \
             DELETE FROM sopl"
        )
        .is_err(),
        "a bogus key inside a no-FROM set-op leaf must be refused"
    );
    assert_eq!(
        sopl_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The same class **with no key at all**: an ordinary correlated reference inside the leaf
/// is judged by nothing either, and the `UPDATE` polarity overwrote every row with `99`
/// where 18.6 raises `42P01`.
#[test]
fn a_set_op_no_from_leaf_correlated_ref_does_not_update() {
    let mut db = set_op_no_from_leaf_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT 1 AS z UNION ALL SELECT (SELECT 1 FROM sople WHERE zzz.k = 1)) \
             UPDATE sopl SET a = 99"
        )
        .is_err(),
        "a correlated reference no relation answers inside a no-FROM set-op leaf must be refused"
    );
    assert_eq!(
        sopl_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the update"
    );
}

/// The **left** operand reaches `finalize_leaf` through the other call site, so the repair
/// has to be in the shared function rather than at one caller.
#[test]
fn a_set_op_no_from_left_leaf_correlated_ref_does_not_delete() {
    let mut db = set_op_no_from_leaf_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT (SELECT 1 FROM sople WHERE zzz.k = 1) UNION ALL SELECT 1 AS z) \
             DELETE FROM sopl"
        )
        .is_err(),
        "a correlated reference inside the left no-FROM leaf must be refused"
    );
    assert_eq!(
        sopl_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **accept** polarity, and the reason the leaf's pass is handed `scope_is_complete()`
/// rather than closing unconditionally: inside an `IN (…)` the leaf may legitimately name
/// the enclosing query's relation, and both engines answer.
#[test]
fn a_set_op_no_from_leaf_may_name_an_enclosing_relation() {
    let mut db = set_op_no_from_leaf_seed();
    db.execute("INSERT INTO sople VALUES (1)").unwrap();
    db.execute(
        "DELETE FROM sopl WHERE a IN \
         (SELECT 1 AS z UNION ALL SELECT (SELECT k FROM sople WHERE sople.k = sopl.a))",
    )
    .expect("a correlated reference inside a no-FROM set-op leaf binds to the enclosing level");
    assert_eq!(
        sopl_rows(&mut db),
        vec![vec![Value::Int4(2)]],
        "the matching row is removed"
    );
}

/// **The `Plan::Sort` peel.** `is_set_op_tree` peeled a parenthesized operand's own
/// `Plan::Limit` and not its own `Plan::Sort`, so an operand carrying its *own* `ORDER BY`
/// put the gate's whole subject out of reach and the trailing key went unjudged again.
/// The `DELETE` removed both rows.
///
/// 18.6 refuses the spelling with `42601 multiple ORDER BY clauses not allowed`, and so
/// does pglet now, in the parse and ahead of the peel — so the tests of this spelling hold that the
/// statement is refused with nothing written, whichever gate refuses it.
#[test]
fn a_paren_leading_set_op_carrying_its_own_sort_key_does_not_delete() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "WITH c AS ((SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY a) \
             ORDER BY bogus) DELETE FROM sopw"
        )
        .is_err(),
        "a trailing key over an operand carrying its own ORDER BY must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **qualified** polarity of the peel, which reaches the gate's other arm (`42P01`).
#[test]
fn a_paren_leading_set_op_carrying_its_own_sort_qualified_key_does_not_update() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "WITH c AS ((SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY a) \
             ORDER BY bogusrel.a) UPDATE sopw SET a = 99"
        )
        .is_err(),
        "a qualified trailing key over an operand carrying its own ORDER BY must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the update"
    );
}

/// The operand carrying **both** a bound and a sort, so the tree is `Limit(Sort(SetOp))`
/// and the walk has to peel through both arms rather than stop at the first.
#[test]
fn a_paren_leading_set_op_carrying_a_bound_and_its_own_sort_does_not_delete() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "WITH c AS ((SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY a LIMIT 1) \
             ORDER BY bogus) DELETE FROM sopw"
        )
        .is_err(),
        "a trailing key over a bounded, self-sorted operand must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The peel's **accept** polarity, and the reason it is safe: nothing under a `Plan::Limit`
/// over a parenthesized *plain* select is a `Plan::SetOp`, so the walk stops on its own and
/// the key stays ungated. 18.6 refuses this spelling `42703` — its grammar merges the
/// trailing `ORDER BY` into the parenthesized select, whose namespace lacks the name — so it
/// is a pglet leniency the peel neither causes nor closes, and not fixturable in either
/// direction, which is why the guard is here. The operand carries a `LIMIT` rather than an
/// `ORDER BY` of its own, which would be `42601` before any walk.
///
/// `bogusplain` names nothing, so a peel that ran past the plain select would refuse it.
#[test]
fn a_sort_over_a_bounded_plain_select_stays_ungated() {
    let mut db = set_op_sort_key_seed();
    db.execute("CREATE TABLE sopwt (a INT, b INT)").unwrap();
    db.execute("INSERT INTO sopwt VALUES (1, 9), (2, 8)")
        .unwrap();
    db.execute(
        "WITH c AS ((SELECT a FROM sopwt LIMIT 5) ORDER BY bogusplain) \
         DELETE FROM sopw WHERE a = 1",
    )
    .expect("a sort over a bounded plain select is not a set-op tree");
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(2)]],
        "the matching row is removed"
    );
}

/// The accept polarity that actually **discriminates** the stop condition: a plain select
/// with no set op anywhere cannot tell a walk that stops from one that keeps going, since
/// there is no `Plan::SetOp` to reach either way. Here there is one — under a derived table
/// — and the walk must still stop, the sort clause belonging to the plain select that reads
/// it, which 18.6 folds it into (and refuses `42703`, the leniency above).
///
/// Be exact about which over-peel it catches, because that is measurable and a general
/// claim is not: a `Plan::SubqueryScan` arm added to `is_set_op_tree` alone does not
/// redden it, and neither does a `Plan::Project` arm alone; the two together do — the
/// operand's `LIMIT` stands over a `Project`. The spelling that reddened on the
/// `SubqueryScan` arm alone, an `ORDER BY` inside the parentheses under the trailing one,
/// is `42601` in the parse now, before any walk.
///
/// The `WITH` is unreferenced so its body never executes, which is what keeps the
/// eval-time resolver out of the answer: the same statement spelled as a top-level read is
/// `42703` from the resolver whichever way the walk goes, and would prove nothing.
#[test]
fn a_sort_over_a_bounded_set_op_derived_table_stays_ungated() {
    let mut db = set_op_sort_key_seed();
    db.execute("INSERT INTO sopwa VALUES (1)").unwrap();
    db.execute("INSERT INTO sopwb VALUES (2)").unwrap();
    db.execute(
        "WITH c AS ((SELECT a FROM (SELECT a FROM sopwa UNION SELECT a FROM sopwb) s \
         LIMIT 5) ORDER BY bogusd) DELETE FROM sopw WHERE a = 1",
    )
    .expect("a set op under a derived table is not this clause's tree");
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(2)]],
        "the matching row is removed"
    );
}

/// **The computed-key leaf gate.** A key that was not itself an [`Expr::Column`] escaped
/// the set-op binding gate entirely, so a bogus name *inside* one was judged by nothing at
/// plan time and the `DELETE` removed both rows. 18.6 resolves a set-op sort key's names
/// before it raises its own `0A000` for the expression, so this is `42703` there.
///
/// The SQLSTATE is pinned differentially
/// (`tests/fixtures/select/set_op_order_by_computed_key_in_a_cte_delete_loud.sql`); what
/// this adds is that the refusal precedes the delete.
#[test]
fn a_set_op_computed_sort_key_leaf_binding_to_nothing_does_not_delete() {
    let mut db = set_op_sort_key_seed();
    assert!(
        db.execute(
            "WITH c AS (SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY bogus + 1) \
             DELETE FROM sopw"
        )
        .is_err(),
        "a computed key whose leaf binds to nothing must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **qualified** leaf polarity of the same gate — `42P01`, since a set-op's trailing
/// keys reach no range table at all. Row-count independent on both sides: the arms are
/// seeded here and the statement still wrote.
#[test]
fn a_set_op_computed_sort_key_with_a_bogus_qualifier_does_not_update() {
    let mut db = set_op_sort_key_seed();
    db.execute("INSERT INTO sopwa VALUES (1)").unwrap();
    db.execute("INSERT INTO sopwb VALUES (2)").unwrap();
    assert!(
        db.execute(
            "WITH c AS (SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY bogusrel.a + 1) \
             UPDATE sopw SET a = 99"
        )
        .is_err(),
        "a computed key carrying a qualifier no relation answers must be refused"
    );
    assert_eq!(
        sopw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the update"
    );
}

/// The leaf gate's **accept** polarity — the documented leniency it must not eat. A
/// computed key whose leaves all bind still *sorts* here, where 18.6 refuses every one of
/// these `0A000 invalid UNION/INTERSECT/EXCEPT ORDER BY clause`. That divergence is what
/// makes them unfixturable, so the guard is here, and it asserts the **order** rather than
/// merely that the statement runs: a gate that dropped the sort would still not error.
#[test]
fn a_set_op_computed_sort_key_over_bound_leaves_still_sorts() {
    let mut db = set_op_sort_key_seed();
    db.execute("INSERT INTO sopwa VALUES (3), (1)").unwrap();
    db.execute("INSERT INTO sopwb VALUES (2)").unwrap();
    let descending = vec![
        vec![Value::Int4(3)],
        vec![Value::Int4(2)],
        vec![Value::Int4(1)],
    ];
    for key in ["-a", "a + 1", "abs(a)", "COALESCE(a, 0)"] {
        let sql =
            format!("SELECT a FROM sopwa UNION SELECT a FROM sopwb ORDER BY {key} DESC NULLS LAST");
        let mut expected = descending.clone();
        // `-a` inverts the comparison, so `DESC` over it is ascending in `a`.
        if key == "-a" {
            expected.reverse();
        }
        assert_eq!(
            rows(&mut db, &sql),
            expected,
            "`ORDER BY {key}` still sorts"
        );
    }
}

/// Seed a source and a target for the **first-lowering** sort-key writes below.
///
/// The CTE those statements carry is unreferenced, so its body never executes and the
/// eval-time resolver — the only judge a top-level `ORDER BY` key had before its keys were
/// resolved against the target list — never ran. The source is **seeded** deliberately:
/// this polarity is row-count-independent, where the read polarity needs an empty relation
/// to reproduce (`tests/fixtures/select/order_by_key_binds_to_nothing_loud.sql`).
fn sort_key_write_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE skw_src (a INT)").unwrap();
    db.execute("CREATE TABLE skw (a INT)").unwrap();
    db.execute("INSERT INTO skw_src VALUES (7)").unwrap();
    db.execute("INSERT INTO skw VALUES (1), (2)").unwrap();
    db
}

/// The untouched target rows of [`sort_key_write_seed`].
fn skw_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT a FROM skw ORDER BY a")
}

/// **The first-lowering sort-key write.** `lower_select_level` resolved its `ORDER BY` and
/// `DISTINCT ON` keys before the projection existed, so the binding gate had no claimant
/// set to judge a bare name against and declined to judge any — and an unreferenced CTE's
/// body never runs, so nothing judged it at all. The `DELETE` removed both rows where 18.6
/// raises `42703` and leaves them.
///
/// The SQLSTATE is pinned differentially
/// (`tests/fixtures/select/order_by_key_binds_to_nothing_in_a_cte_delete_loud.sql`); what
/// this adds is that the refusal precedes the delete, which an `@expect-error` fixture
/// cannot distinguish from a refusal raised after it.
#[test]
fn a_sort_key_binding_to_nothing_does_not_delete() {
    let mut db = sort_key_write_seed();
    assert!(
        db.execute("WITH c AS (SELECT a FROM skw_src ORDER BY bogus) DELETE FROM skw")
            .is_err(),
        "a top-level sort key binding to nothing must be refused"
    );
    assert_eq!(
        skw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The `UPDATE` polarity of the same key, which overwrote every row with `99`.
#[test]
fn a_sort_key_binding_to_nothing_does_not_update() {
    let mut db = sort_key_write_seed();
    assert!(
        db.execute("WITH c AS (SELECT a FROM skw_src ORDER BY bogus) UPDATE skw SET a = 99")
            .is_err(),
        "a top-level sort key binding to nothing must be refused"
    );
    assert_eq!(
        skw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the update"
    );
}

/// The same write through the three **other** plan nodes the one blanket reached — an
/// aggregate's and a window's junk column, and a `SELECT DISTINCT` — plus a `DISTINCT ON`
/// key, each inside the same unreferenced CTE. One node was fixed per member would have
/// left the rest writing, so the guard enumerates them rather than trusting the shape.
///
/// The `SELECT DISTINCT` member is the one that was already loud (`42P10` for 18.6's
/// `42703`, so no row was ever lost through it); it is here for the enumeration, and its
/// code is pinned by
/// `tests/fixtures/select/order_by_key_binds_to_nothing_under_distinct_loud.sql`.
#[test]
fn a_sort_key_binding_to_nothing_does_not_delete_through_any_projecting_node() {
    for source in [
        "SELECT count(*) FROM skw_src GROUP BY a ORDER BY bogus",
        "SELECT row_number() OVER () AS w FROM skw_src ORDER BY bogus",
        "SELECT DISTINCT a FROM skw_src ORDER BY bogus",
        "SELECT DISTINCT ON (bogus) a FROM skw_src",
    ] {
        let mut db = sort_key_write_seed();
        assert!(
            db.execute(&format!("WITH c AS ({source}) DELETE FROM skw"))
                .is_err(),
            "`{source}` must be refused"
        );
        assert_eq!(
            skw_rows(&mut db),
            vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
            "the refusal must precede the delete for `{source}`"
        );
    }
}

/// The **ambiguous** polarity's write: a bare key two of a join's columns claim, where
/// taking the first claimant sorted and let the `DELETE` through. `ON false` keeps every
/// row away from the eval-time resolver, which is what makes the plan-time count the only
/// judge — its read polarity is
/// `tests/fixtures/select/order_by_ambiguous_join_column_over_no_row_loud.sql`.
#[test]
fn an_ambiguous_sort_key_does_not_delete() {
    let mut db = sort_key_write_seed();
    db.execute("CREATE TABLE skj1 (n INT, x INT)").unwrap();
    db.execute("CREATE TABLE skj2 (n INT, y INT)").unwrap();
    db.execute("INSERT INTO skj1 VALUES (1, 5)").unwrap();
    db.execute("INSERT INTO skj2 VALUES (1, 6)").unwrap();
    assert!(
        db.execute(
            "WITH c AS (SELECT skj1.x FROM skj1 JOIN skj2 ON false ORDER BY n) DELETE FROM skw"
        )
        .is_err(),
        "a sort key two join columns claim must be refused"
    );
    assert_eq!(
        skw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **accept** polarity, guarding the same gate from the other side: the two spellings
/// `findTargetlistEntrySQL92` resolves — a key naming an output alias no FROM relation
/// carries, and a key naming a base column the projection **drops** — must still answer,
/// and in the right order. A claimant set that degenerated into a count by name, or a gate
/// that stopped falling through to the range table, breaks one each.
#[test]
fn a_sort_key_may_name_an_output_alias_or_a_dropped_base_column() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ska (a INT, b INT)").unwrap();
    db.execute("INSERT INTO ska VALUES (1, 30), (2, 10), (3, 20)")
        .unwrap();
    assert_eq!(
        rows(&mut db, "SELECT b * 1 AS lbl FROM ska ORDER BY lbl"),
        vec![
            vec![Value::Int4(10)],
            vec![Value::Int4(20)],
            vec![Value::Int4(30)]
        ],
        "a bare key naming an output alias binds to the target-list entry"
    );
    assert_eq!(
        rows(&mut db, "SELECT b FROM ska ORDER BY a DESC"),
        vec![
            vec![Value::Int4(20)],
            vec![Value::Int4(10)],
            vec![Value::Int4(30)]
        ],
        "a bare key no target-list entry carries falls through to the range table"
    );
}

/// Seed `nfw(a)` = `(1), (2)` for the **no-FROM tail**'s write polarities.
///
/// A no-FROM select carries no relation of its own, so the CTE wrapping it is
/// unreferenced and its body never executes — which is what makes these row-count
/// independent, exactly as the no-FROM `DISTINCT ON` writes above are. There is no array
/// length and no seed that saves them.
fn no_from_tail_seed() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE nfw (a INT)").unwrap();
    db.execute("INSERT INTO nfw VALUES (1), (2)").unwrap();
    db
}

/// The untouched target rows of [`no_from_tail_seed`].
fn nfw_rows(db: &mut Database) -> Vec<Vec<Value>> {
    rows(db, "SELECT a FROM nfw ORDER BY a")
}

/// **The no-FROM sort key's write.** `parser::select`'s `SelectLeaf::NoFrom` arm parsed the
/// trailing `ORDER BY` and threw the keys away, so no key survived for any gate to judge
/// and the `DELETE` removed both rows where 18.6 raises `42703` and leaves them.
///
/// Routing the tail alone would not close this. A `Plan::Sort` over a no-FROM projection
/// judges its key only where a row reaches the eval-time resolver, and an unreferenced
/// CTE's body never runs at all — so the gate `plan_no_from_order_limit` adds at plan time
/// is what this reddens on. The SQLSTATE is pinned differentially
/// (`tests/fixtures/select/no_from_order_by_bogus_key_in_a_cte_delete_loud.sql`); what
/// this adds is that the refusal precedes the delete.
#[test]
fn a_no_from_sort_key_binding_to_nothing_does_not_delete() {
    let mut db = no_from_tail_seed();
    assert!(
        db.execute("WITH c AS (SELECT 1 AS q ORDER BY zzz) DELETE FROM nfw")
            .is_err(),
        "a no-FROM sort key binding to nothing must be refused"
    );
    assert_eq!(
        nfw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The `UPDATE` polarity of the same key, which overwrote every row with `99`.
#[test]
fn a_no_from_sort_key_binding_to_nothing_does_not_update() {
    let mut db = no_from_tail_seed();
    assert!(
        db.execute("WITH c AS (SELECT 1 AS q ORDER BY zzz) UPDATE nfw SET a = 99")
            .is_err(),
        "a no-FROM sort key binding to nothing must be refused"
    );
    assert_eq!(
        nfw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the update"
    );
}

/// The **qualified** spelling writes through the same site and is refused by a different
/// arm of the gate: a qualifier never reaches `findTargetlistEntrySQL92`, so it is judged
/// against the FROM relations, of which a no-FROM select has none (`42P01` on 18.6, where
/// the bare spelling is `42703`).
#[test]
fn a_no_from_sort_key_qualifier_binding_to_nothing_does_not_delete() {
    let mut db = no_from_tail_seed();
    assert!(
        db.execute("WITH c AS (SELECT 1 AS q ORDER BY bogusrel.a) DELETE FROM nfw")
            .is_err(),
        "a no-FROM sort key qualifier binding to nothing must be refused"
    );
    assert_eq!(
        nfw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "the refusal must precede the delete"
    );
}

/// The **positional** spelling is a third arm — the range check rather than the binding —
/// and it wrote the same way, an ordinal past the select list having never been checked at
/// all. `-1` reaches it through `parser::expr`'s constant fold, so this also guards that
/// the fold's product is judged here rather than only where a `FROM` clause exists.
#[test]
fn a_no_from_sort_key_out_of_range_position_does_not_delete() {
    let mut db = no_from_tail_seed();
    for sql in [
        "WITH c AS (SELECT 1 AS q ORDER BY 5) DELETE FROM nfw",
        "WITH c AS (SELECT 1 AS q ORDER BY -1) DELETE FROM nfw",
    ] {
        assert!(
            db.execute(sql).is_err(),
            "an out-of-range no-FROM ordinal must be refused: {sql}"
        );
        assert_eq!(
            nfw_rows(&mut db),
            vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
            "the refusal must precede the delete: {sql}"
        );
    }
}

/// The **bound**'s write polarity, and the one member of this family that is silent on both
/// engines: nothing raises here, so an `is_err()` pin cannot hold it and the row set is the
/// whole assertion. With the `LIMIT 0` discarded the subquery answered its row and the
/// `DELETE` removed one 18.6 keeps.
///
/// Its differential twin (`tests/fixtures/select/no_from_limit_zero_in_a_delete.sql`) pins
/// the same rows against a live 18.6; this one pins them on the embed engine, where the
/// `@setup`/`@test` split does not exist and the statement's own tag is visible.
#[test]
fn a_no_from_bound_does_not_widen_a_delete() {
    let mut db = no_from_tail_seed();
    db.execute("DELETE FROM nfw WHERE a IN (SELECT 1 AS q LIMIT 0)")
        .unwrap();
    assert_eq!(
        nfw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "a no-FROM LIMIT 0 subquery matches nothing, so the delete removes nothing"
    );
    db.execute("DELETE FROM nfw WHERE a IN (SELECT 1 AS q OFFSET 1)")
        .unwrap();
    assert_eq!(
        nfw_rows(&mut db),
        vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
        "a no-FROM OFFSET 1 subquery matches nothing either"
    );
}

/// The discarded `DISTINCT`'s write polarity, silent on both engines like the bound above:
/// `INSERT INTO … SELECT DISTINCT unnest(ARRAY[1,1,2])` wrote **three** rows against 18.6's
/// two. The row *count* is the assertion, which is what keeps this independent of the
/// unspecified dedup output order.
#[test]
fn a_no_from_distinct_does_not_write_an_extra_row() {
    let mut db = no_from_tail_seed();
    db.execute("INSERT INTO nfw SELECT DISTINCT unnest(ARRAY[1, 1, 2])")
        .unwrap();
    assert_eq!(
        nfw_rows(&mut db),
        vec![
            vec![Value::Int4(1)],
            vec![Value::Int4(1)],
            vec![Value::Int4(2)],
            vec![Value::Int4(2)],
        ],
        "the dedup happens before the insert, so two rows are added and not three"
    );
}

/// The **accept** polarity of the no-FROM tail, guarding the routing from the other side:
/// a sort key that binds must still sort, a bound must still window, and a no-FROM select
/// inside an expression subquery must still be able to name the enclosing relation — the
/// last being what an unconditional close would refuse.
#[test]
fn a_no_from_tail_still_answers_what_both_engines_answer() {
    let mut db = no_from_tail_seed();
    assert_eq!(
        rows(
            &mut db,
            "SELECT unnest(ARRAY[3, 1, 4, 2]) AS q ORDER BY q DESC LIMIT 2 OFFSET 1"
        ),
        vec![vec![Value::Int4(3)], vec![Value::Int4(2)]],
        // The array is chosen so the two nestings disagree: a `Limit` under the `Sort`
        // windows the unsorted rows to `1, 4` and answers `4, 1`. Over a sorted-prefix
        // array such as `[3, 1, 2]` both nestings answer the same thing, which is how the
        // first draft of this assertion passed under that mutation.
        "the Sort sits under the Limit, so the window is taken from the sorted rows"
    );
    assert_eq!(
        rows(
            &mut db,
            "SELECT (SELECT 1 AS q ORDER BY nfw.a) AS r FROM nfw"
        ),
        vec![vec![Value::Int4(1)], vec![Value::Int4(1)]],
        "a no-FROM sort key may name an enclosing relation, so the gate must stay open there"
    );
}
