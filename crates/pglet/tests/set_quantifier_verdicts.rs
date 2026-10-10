//! The function-call **set quantifier** (`DISTINCT` / `ALL`) — the three things the
//! differential fixtures structurally cannot reach, over the embed engine on the
//! **default** CoW-B+tree backend.
//!
//! The first is **state after a refusal**. A fixture runs inside `BEGIN … ROLLBACK`
//! and propagates the first error from every statement it runs, so the statement that is
//! refused and the statement that observes the refusal cannot sit on opposite sides of
//! the compared boundary — and an `@expect-error` fixture is green for a refusal raised
//! *after* the writes. The refusal's SQLSTATE is pinned differentially
//! (`tests/fixtures/sequences/set_quantifier_distinct_nextval_loud.sql` and its
//! siblings, which is what makes it 18.6's and not merely pglet's); what is asserted
//! here is that the counter and the rows are untouched.
//!
//! **At today's placement the *quantified* counter and row assertions cannot fail**, and
//! a green run of those should be read
//! accordingly: the verdict is raised while the statement is parsed, so nothing executes
//! and there is nothing to roll back. What they guard is a *future* placement after the
//! writes — a shape this engine has shipped before (`docs/scope.md` records a `RETURNING`
//! refusal raised from inside `execute_update` after them). There the **sequence**
//! members are the load-bearing ones: a `nextval`/`setval` advance is non-transactional,
//! so the row members would be undone by this backend's autocommit statement rollback
//! while a counter walked forward stays walked forward, past the failed statement and
//! every rollback around it.
//! The **unquantified** sequence member
//! (`an_unquantified_sequence_name_cast_does_not_move_the_counter`) is the exception and
//! the reason not to read that sentence as covering the file: it guards a check of the
//! sequence arm's own, it is red on that check's revert, and it caught the check being
//! deleted as redundant of a transform that no longer runs there.
//!
//! The second is the **message text**, which `@expect-error` never compares: 18.6
//! spells five modifier refusals into one sentence shape from one `ParseFuncOrColumn`
//! block, and pglet models two of them (`<name>(*)` and `DISTINCT`) through one error
//! variant. Nothing but a message assertion separates the two spellings, or catches the
//! variant being handed the wrong modifier.
//!
//! The third is a **refusal both engines make at different SQLSTATEs**, where an
//! `@expect-error` fixture compares the two codes and is red: the quoted `"trim"` member
//! of `planner::grammar_only_name`. What that test asserts is pglet's own posture, and
//! the divergence is stated in its own doc, which is the only place it can be.

// A test harness: setup `.unwrap()`s and the helpers `.expect()`, so a setup failure
// surfaces loudly here rather than as a confusing assertion mismatch.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::Value;

fn rows(db: &mut Database, sql: &str) -> Vec<Vec<Value>> {
    let mut results = db.execute(sql).expect("execute should succeed");
    assert_eq!(results.len(), 1, "expected one result set for {sql:?}");
    results.remove(0).rows
}

/// The `(SQLSTATE, message)` `sql` fails with — panicking when it succeeds, so a
/// regression that re-accepts the statement stops here rather than at a value.
fn err(db: &mut Database, sql: &str) -> (&'static str, String) {
    let e = db
        .execute(sql)
        .err()
        .unwrap_or_else(|| panic!("{sql:?} should be refused"));
    (e.sqlstate(), e.to_string())
}

/// A fresh database holding one sequence `sq` (never advanced) and one row in
/// `t (id INT, c TEXT)`.
fn seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE SEQUENCE sq").unwrap();
    db.execute("CREATE TABLE t (id INT, c TEXT)").unwrap();
    db.execute("INSERT INTO t VALUES (1, 'x')").unwrap();
    db
}

/// A refused `nextval(DISTINCT …)` leaves the counter where it was, so the next
/// ordinary call answers `1`.
///
/// The member that made this class worse than a wrong value: pglet dropped the modifier,
/// advanced the sequence and answered `1`, after which the next `nextval` answered `2`
/// where 18.6 — which refuses the statement in parse analysis — answers `1`.
#[test]
fn a_refused_nextval_does_not_advance_the_counter() {
    let mut db = seeded();
    let (state, message) = err(&mut db, "SELECT nextval(DISTINCT 'sq')");
    assert_eq!(state, "42809");
    assert_eq!(
        message,
        "DISTINCT specified, but nextval is not an aggregate function"
    );
    assert_eq!(
        rows(&mut db, "SELECT nextval('sq')"),
        vec![vec![Value::Int8(1)]]
    );
}

/// The `setval` spelling of the same property. It moves the counter to an arbitrary
/// value rather than stepping it, so a dropped modifier left a larger divergence than
/// one id — pglet answered `50` and the next `nextval` answered `51`.
#[test]
fn a_refused_setval_does_not_move_the_counter() {
    let mut db = seeded();
    let (state, message) = err(&mut db, "SELECT setval(DISTINCT 'sq', 50)");
    assert_eq!(state, "42809");
    assert_eq!(
        message,
        "DISTINCT specified, but setval is not an aggregate function"
    );
    assert_eq!(
        rows(&mut db, "SELECT nextval('sq')"),
        vec![vec![Value::Int8(1)]]
    );
}

/// The **unquantified** spelling of the same two writes, which is a different check and
/// the one that has actually failed here.
///
/// `sequence_call` reduces its name argument to a `String` and discards the expression,
/// so a constant cast on it is raised by that arm or not at all — and this arm's check
/// was deleted as redundant of an *ungated* argument transform, then uncovered when the
/// transform was gated on a written quantifier (which it must be: run for an unquantified
/// call it pre-empts every executor-time DDL gate, `planner::transform_call_args`). With
/// the gate in and the check out, both statements answered a value and **moved the
/// counter**, where 18.6 evaluates `'sq'::int` in its argument transform and raises
/// `22P02`. Unlike the quantified members above, this one is not vacuous at today's
/// placement: it is red on the revert.
#[test]
fn an_unquantified_sequence_name_cast_does_not_move_the_counter() {
    let mut db = seeded();
    assert_eq!(err(&mut db, "SELECT nextval('sq'::int)").0, "22P02");
    assert_eq!(err(&mut db, "SELECT setval('sq'::int, 50)").0, "22P02");
    assert_eq!(
        rows(&mut db, "SELECT nextval('sq')"),
        vec![vec![Value::Int8(1)]]
    );
}

/// The three DML shapes: a refused `INSERT`, `UPDATE` and `DELETE` leave the row set as
/// it was. Each answered here before — the `INSERT` wrote a row, the `UPDATE` stored
/// `A`, the `DELETE` removed the only row — where 18.6 refuses the statement whole.
#[test]
fn a_refused_dml_statement_leaves_the_rows_alone() {
    let mut db = seeded();
    assert_eq!(
        err(&mut db, "INSERT INTO t VALUES (2, upper(DISTINCT 'a'))").0,
        "42809"
    );
    assert_eq!(
        err(
            &mut db,
            "INSERT INTO t VALUES (2, 'a') RETURNING upper(DISTINCT c)"
        )
        .0,
        "42809"
    );
    assert_eq!(
        err(&mut db, "UPDATE t SET c = upper(DISTINCT 'a') WHERE id = 1").0,
        "42809"
    );
    assert_eq!(
        err(&mut db, "DELETE FROM t WHERE upper(DISTINCT c) = 'X'").0,
        "42809"
    );
    assert_eq!(
        rows(&mut db, "SELECT id, c FROM t"),
        vec![vec![Value::Int4(1), Value::Text("x".to_string())]]
    );
}

/// The `DO $$ … $$` route reaches the same gate — its body is parsed by the same
/// expression parser — and the refused statement writes nothing.
#[test]
fn a_refused_statement_inside_a_do_block_writes_nothing() {
    let mut db = seeded();
    assert_eq!(
        err(
            &mut db,
            "DO $$ BEGIN INSERT INTO t VALUES (2, upper(DISTINCT 'a')); END $$"
        )
        .0,
        "42809"
    );
    assert_eq!(
        rows(&mut db, "SELECT count(*) FROM t"),
        vec![vec![Value::Int8(1)]]
    );
}

/// The two modifiers this engine models share one error variant and one SQLSTATE, so
/// only the message separates them — and only a message assertion catches the variant
/// being handed the wrong modifier, or the name twice where Postgres prints it once.
/// Both sentences are `ParseFuncOrColumn`'s, verbatim.
#[test]
fn the_two_modifier_refusals_keep_their_own_sentences() {
    let mut db = seeded();
    assert_eq!(
        err(&mut db, "SELECT upper(DISTINCT 'a')"),
        (
            "42809",
            "DISTINCT specified, but upper is not an aggregate function".to_string()
        )
    );
    assert_eq!(
        err(&mut db, "SELECT now(*)"),
        (
            "42809",
            "now(*) specified, but now is not an aggregate function".to_string()
        )
    );
}

/// The accept polarity, in one place where it is a *write*: `ALL` is the no-op
/// quantifier, so the sequence advances exactly as the unquantified call does. pglet
/// refused every `ALL` spelling `42601` before, which is the over-rejection half of the
/// same defect — and the half a gate that refuses both keywords alike would leave open.
#[test]
fn an_all_quantifier_is_a_no_op_on_a_sequence_function() {
    let mut db = seeded();
    assert_eq!(
        rows(&mut db, "SELECT nextval(ALL 'sq')"),
        vec![vec![Value::Int8(1)]]
    );
    assert_eq!(
        rows(&mut db, "SELECT nextval('sq')"),
        vec![vec![Value::Int8(2)]]
    );
}

/// A quoted `"trim"` is refused rather than answered — the member of
/// `planner::grammar_only_name` no differential fixture can hold **at all**. Every
/// member's *quoted* spelling is a code disagreement (18.6 resolves a quoted name
/// against `pg_proc`, which holds none of the five, so it answers `42883` where this
/// engine answers `42601`), and the other four are fixtured through their **bare**
/// spelling, where both engines say `42601`. `trim`'s bare spelling never reaches the
/// gate — `parser::parse_trim` owns it — so the quoted one is all there is, and an
/// `@expect-error` fixture over it would compare `42601` against 18.6's
/// `42883 function trim(unknown) does not exist` and be red. What is asserted here is
/// therefore pglet's own posture, not 18.6's: loud, and the same `42601` the four bare
/// grammar names carry.
///
/// The member exists because teaching the parser `ALL` opened it. A quoted `"trim"`
/// leaves `parse_trim`'s keyword route and lands on the ordinary dispatch's
/// `"btrim" | "trim"` arm, where `ALL` is dropped as the no-op it is — so
/// `SELECT "trim"(ALL ' x ')` **answered `x`** against 18.6's refusal, a silent
/// over-acceptance where the unquantified spelling's silence was already recorded.
///
/// The two controls are what keep the rule `pg_proc` membership rather than a guess at
/// 18.6's grammar list: `"btrim"` is the same arm under a name `pg_proc` does hold, and
/// the bare `trim` never reaches the dispatch at all. Their differential half —
/// `"substring"`, the neighbour on 18.6's grammar list that `pg_proc` also holds — is
/// two fixtures (`functions/set_quantifier_*_quoted_substring*.sql`), since there both
/// engines agree.
#[test]
fn a_quoted_grammar_name_pg_proc_lacks_is_refused_rather_than_answered() {
    let mut db = seeded();
    assert_eq!(
        err(&mut db, "SELECT \"trim\"(ALL ' x ')"),
        (
            "42601",
            "syntax error: ALL is not allowed in the trim expression".to_string()
        )
    );
    assert_eq!(
        err(&mut db, "SELECT \"trim\"(DISTINCT ' x ')").0,
        "42601",
        "the DISTINCT spelling takes the same gate, ahead of the 42809 it used to answer"
    );
    assert_eq!(
        rows(&mut db, "SELECT \"btrim\"(ALL ' x ')"),
        vec![vec![Value::Text("x".to_string())]],
        "the same dispatch arm under a name pg_proc holds must keep answering"
    );
    assert_eq!(
        rows(&mut db, "SELECT trim(' x ')"),
        vec![vec![Value::Text("x".to_string())]],
        "the bare spelling is parse_trim's and never reaches the gate"
    );
}

/// The aggregate control, over the flag rather than over an answer: `DISTINCT` sets the
/// aggregate's own dedup and `ALL` — the explicit spelling of the default — must not.
/// Over `1, 1, 2` those are `2` and `3`.
#[test]
fn the_aggregates_keep_taking_a_quantifier() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE agg (a INT)").unwrap();
    db.execute("INSERT INTO agg VALUES (1), (1), (2)").unwrap();
    assert_eq!(
        rows(&mut db, "SELECT count(DISTINCT a) FROM agg"),
        vec![vec![Value::Int8(2)]]
    );
    assert_eq!(
        rows(&mut db, "SELECT count(ALL a) FROM agg"),
        vec![vec![Value::Int8(3)]]
    );
    assert_eq!(
        rows(&mut db, "SELECT count(a) FROM agg"),
        vec![vec![Value::Int8(3)]]
    );
}

/// The **third** route that reads a quantifier — `parser::ddl`'s `parse_nextval_arg`,
/// the DDL `DEFAULT` classifier's `nextval` arm — carries the same sentence as the other
/// two, which is the half no `@expect-error` fixture compares. The route was absent from
/// the enumeration once, and while it was, `DEFAULT nextval(ALL 's')` was `42601` here
/// against a plain accept on 18.6.
///
/// What follows is this file's first thing at a DDL site: the refused statement left
/// **nothing** behind — its table name is free, and the sequence it named is still
/// unadvanced, which the accepted `ALL` spelling then reads as `1`. Both hold for a
/// structural reason worth stating rather than for a rollback: the refusal is raised while
/// the statement is *parsed*, so no catalog write and no `nextval` had happened.
/// `docs/ddl.md`'s phantom rule is about a check that runs after a catalog write, and this
/// one cannot — which is why the assertions are cheap to keep and would only start
/// failing if the verdict moved.
#[test]
fn the_column_default_classifier_carries_the_same_verdict() {
    let mut db = seeded();
    assert_eq!(
        err(
            &mut db,
            "CREATE TABLE dq (a BIGINT DEFAULT nextval(DISTINCT 'sq'))"
        ),
        (
            "42809",
            "DISTINCT specified, but nextval is not an aggregate function".to_string()
        )
    );
    db.execute("CREATE TABLE dq (a BIGINT DEFAULT nextval(ALL 'sq'))")
        .expect("the refused name is free and the ALL spelling is accepted");
    db.execute("INSERT INTO dq DEFAULT VALUES").unwrap();
    assert_eq!(
        rows(&mut db, "SELECT a FROM dq"),
        vec![vec![Value::Int8(1)]]
    );
}
