//! The boolean-context rule's parts no differential fixture can reach.
//!
//! The **predicate identity** — that the gate asks `assignment_compatible(x, boolean)`
//! and nothing narrower — is a claim about which rulebook answers, not about any
//! statement's result, so no SQL text exhibits it. The **construct enumeration** is
//! likewise: a fixture can pin one construct's sentence at a time, and what matters is
//! that every member has one, that no two share a spelling by accident, and that the
//! right member is attached where the gate fires — which no fixture can see, the
//! SQLSTATE being the same whichever construct the message names. The **child-context
//! map** is what makes the interleaved gate reach the right operands; a fixture sees
//! only the first failure, so a context attached to the wrong child is visible here and
//! not there. Beside those sit the shapes no fixture can **set up**: a parameter's
//! inferred type, a catalog an older build wrote, and a verdict two spellings of one
//! statement separate only by the operand their message names.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface an unexpected verdict shape
// loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::HashSet;

use pglet::Database;
use pglet_sql::planner::{
    BinaryOp, BoolContext, BoolTest, Expr, Literal, boolean_context_legality,
};
use pglet_sql::types::TypeOid;

/// The `TypeOid`s this file sweeps, **derived rather than transcribed**:
/// `TypeOid::from_oid` is the declaration side of the enum's OID map, so a type added
/// there joins the sweep with no edit here. Two premises hold that up and the compiler
/// checks neither — that arm list being whole, and every OID the enum carries falling
/// inside the swept range (the largest today is `jsonb[]`'s 3807).
///
/// The transcribed list this replaced called itself every type the engine models and
/// was short of the enum by a scalar and most of the array family. A count is not the
/// repair — deriving the list is, since the count is what goes stale.
fn every_type_oid() -> Vec<TypeOid> {
    let swept: Vec<TypeOid> = (0..=i32::from(u16::MAX))
        .filter_map(TypeOid::from_oid)
        .collect();
    // Non-emptiness is not implied by the sweep, and the caller's
    // boolean-versus-everything-else branch would pass vacuously over an empty list.
    assert!(swept.contains(&TypeOid::Bool), "the sweep found no types");
    swept
}

/// **`boolean` and nothing else resolves.** `pg_cast` holds exactly two rows into
/// `boolean` — `integer` and `jsonb` — and both are explicit-context, so an assignment
/// coercion to `boolean` accepts only the type itself. The gate asks the shared
/// `assignment_compatible` rather than carrying its own list, and this is what says the
/// shared answer is the right one at this target: sweep every modelled type.
#[test]
fn only_boolean_resolves_at_a_boolean_context() {
    for oid in every_type_oid() {
        let verdict = boolean_context_legality(Some(oid), BoolContext::Where);
        if oid == TypeOid::Bool {
            verdict.expect("boolean resolves");
        } else {
            assert_eq!(
                verdict.expect_err("every other type is refused").sqlstate(),
                "42804",
                "{oid:?} must not resolve to boolean"
            );
        }
    }
}

/// An operand the plan-time reader could not type **defers** rather than being refused —
/// an unbound `$N`, a bare string literal, a scope that could not resolve a name. The
/// eval-time resolver sees the value and answers there.
#[test]
fn an_untyped_operand_defers_to_eval_time() {
    boolean_context_legality(None, BoolContext::Where).expect("an unknown defers");
    boolean_context_legality(None, BoolContext::Check).expect("an unknown defers");
}

/// **Each construct names itself, and no two share a spelling.** 18.6's message is
/// `argument of <CTX> must be type boolean`, and the construct half is the only part a
/// caller could get wrong silently — a `HAVING` reported as a `WHERE` is a correct
/// SQLSTATE with a misleading sentence, which every fixture in the corpus scores green.
///
/// This is the **spelling** half alone: it proves each member names itself and that no
/// two collide, never that the right member is attached where a gate fires. The
/// attachment is `the_reported_construct_is_the_clause_as_written`'s.
#[test]
fn every_construct_has_its_own_spelling() {
    let contexts = [
        BoolContext::Where,
        BoolContext::Having,
        BoolContext::JoinOn,
        BoolContext::CaseWhen,
        BoolContext::And,
        BoolContext::Or,
        BoolContext::Not,
        BoolContext::Check,
        BoolContext::Test(BoolTest::True),
        BoolContext::Test(BoolTest::NotTrue),
        BoolContext::Test(BoolTest::False),
        BoolContext::Test(BoolTest::NotFalse),
        BoolContext::Test(BoolTest::Unknown),
        BoolContext::Test(BoolTest::NotUnknown),
    ];
    let spellings: HashSet<&str> = contexts.iter().map(|c| c.sql()).collect();
    assert_eq!(
        spellings.len(),
        contexts.len(),
        "two constructs share a spelling"
    );
    for ctx in contexts {
        let message = boolean_context_legality(Some(TypeOid::Int4), ctx)
            .expect_err("integer is refused")
            .to_string();
        assert!(
            message.contains(&format!("argument of {} must be type boolean", ctx.sql())),
            "{ctx:?} does not name itself: {message}"
        );
    }
    // The `IS …` half delegates to `BoolTest`, so the two enumerations cannot drift into
    // two different spellings of one test.
    assert_eq!(
        BoolContext::Test(BoolTest::NotUnknown).sql(),
        BoolTest::NotUnknown.sql()
    );
}

/// Which children carry a boolean context, read off the **one** child walk every other
/// walker uses. A context on the wrong child is a gate aimed at the wrong operand, and
/// the first-failure reporting of every walker above it hides that: a fixture sees one
/// verdict, never the map.
#[test]
fn the_child_context_map_is_what_the_constructs_say() {
    let int = || Expr::Literal(Literal::Int(1));
    let contexts_of = |e: &Expr| {
        let mut out = Vec::new();
        e.for_each_child_ctx(|_, ctx| out.push(ctx));
        out
    };

    // `AND`/`OR` gate **both** operands; no other operator gates either.
    for (op, ctx) in [
        (BinaryOp::And, BoolContext::And),
        (BinaryOp::Or, BoolContext::Or),
    ] {
        let e = Expr::BinaryOp {
            left: Box::new(int()),
            op,
            right: Box::new(int()),
        };
        assert_eq!(contexts_of(&e), vec![Some(ctx), Some(ctx)]);
    }
    let eq = Expr::BinaryOp {
        left: Box::new(int()),
        op: BinaryOp::Eq,
        right: Box::new(int()),
    };
    assert_eq!(contexts_of(&eq), vec![None, None]);

    // `NOT` gates its one argument.
    assert_eq!(
        contexts_of(&Expr::Not(Box::new(int()))),
        vec![Some(BoolContext::Not)]
    );
    // …where the other unary nodes gate nothing, `IS [NOT] TRUE` included: that one is
    // an ordinary post-order node check, because `transformBooleanTest` transforms the
    // whole argument before coercing it.
    assert_eq!(contexts_of(&Expr::IsNull(Box::new(int()))), vec![None]);
    assert_eq!(
        contexts_of(&Expr::BooleanTest {
            expr: Box::new(int()),
            test: BoolTest::True,
        }),
        vec![None]
    );

    // A **searched** `CASE` gates each condition and no result or `ELSE`…
    let searched = Expr::Case {
        operand: None,
        when_clauses: vec![(int(), int()), (int(), int())],
        else_result: Some(Box::new(int())),
    };
    assert_eq!(
        contexts_of(&searched),
        vec![
            Some(BoolContext::CaseWhen),
            None,
            Some(BoolContext::CaseWhen),
            None,
            None
        ]
    );
    // …and a **simple** one gates nothing at all: 18.6 expands `CASE x WHEN v` to
    // `x = v` and coerces the comparison's result, so the label is the `=` rule's.
    let simple = Expr::Case {
        operand: Some(Box::new(int())),
        when_clauses: vec![(int(), int())],
        else_result: None,
    };
    assert_eq!(contexts_of(&simple), vec![None, None, None]);
}

/// **A `$N` at a boolean context infers `boolean`, and an explicit cast outranks it.**
/// The priority is invisible to the differential corpus twice over: the harness never
/// reads `ParameterDescription`, and the statement that would discriminate — one whose
/// cast types the parameter to something the context refuses — is refused at `Parse`
/// before any parameter type reaches a client. So the inferred OIDs are read here, off
/// the same path the wasm host and the native server's `Describe` reply take.
#[test]
fn a_parameter_at_a_boolean_context_infers_boolean_below_a_cast() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE bp (a INT)").expect("seeds");

    let inferred = |db: &mut Database, sql: &str| -> Vec<Option<TypeOid>> {
        let mut seen = Vec::new();
        // The bind callback refuses, so nothing executes; the OIDs are the product.
        let _ = db.execute_bound(sql, 1, |oids| {
            seen = oids.to_vec();
            Err(())
        });
        seen
    };

    // A clause root, and the same `$N` under a connective — the node positions read the
    // context off the one child walk the gate reads.
    for sql in [
        "SELECT count(*) FROM bp WHERE $1",
        "SELECT count(*) FROM bp WHERE $1 AND a = 1",
        "SELECT count(*) FROM bp WHERE NOT $1",
        "SELECT count(*) FROM bp WHERE a = 1 OR $1",
        "SELECT count(*) FROM bp GROUP BY a HAVING $1",
        "DELETE FROM bp WHERE $1",
        "UPDATE bp SET a = 1 WHERE $1",
    ] {
        assert_eq!(inferred(&mut db, sql), vec![Some(TypeOid::Bool)], "{sql}");
    }

    // An explicit cast records at the higher priority, so it keeps its own type — and
    // the gate then refuses the statement, which is why this is unobservable from a
    // fixture.
    let refused = db.execute_bound("SELECT count(*) FROM bp WHERE $1::int", 1, |_| {
        Err::<Vec<Option<pglet_sql::types::Value>>, ()>(())
    });
    match refused {
        Err(pglet::BoundExecError::Engine(e)) => {
            assert_eq!(e.sqlstate(), "42804");
            assert!(
                e.to_string().contains("must be type boolean"),
                "the cast's type reaches the gate: {e}"
            );
        }
        other => panic!("expected the gate's refusal, got {other:?}"),
    }

    // A position that is **not** a boolean context still types from its own rule, so the
    // inference did not simply start answering `boolean` everywhere.
    assert_eq!(
        inferred(&mut db, "SELECT count(*) FROM bp WHERE a = $1"),
        vec![Some(TypeOid::Int4)]
    );
}

/// The **stored-predicate** gate is on new DDL only, which decides what an older
/// database holding a non-boolean `CHECK` now does: the predicate is decoded straight
/// from the store and re-passes no DDL check, so the eval-time resolver is the only
/// thing standing between it and a write. It answers `42804` — loud, where the old
/// polarity admitted every row — and that is the shape to know about before a store
/// bump is considered.
///
/// Reached here rather than by fixture because no fixture can write a catalog an
/// earlier build produced: the predicate is planted through the same `Expr` the store
/// would decode.
#[test]
fn a_predicate_an_older_build_persisted_fails_loud_at_the_write() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE legacy (a INT)").expect("seeds");
    // The gate refuses this statement, which is the point of the gate…
    let refused = db
        .execute("CREATE TABLE legacy2 (a INT CHECK (a))")
        .expect_err("the DDL gate refuses it");
    assert_eq!(refused.sqlstate(), "42804");
    // …so the only way such a constraint exists is a catalog written before it, which is
    // what this plants directly.
    {
        let table = db.schema_mut().tables.get_mut("legacy").expect("the table");
        table.checks.push(pglet_sql::catalog::CheckConstraint {
            name: "legacy_check".to_string(),
            expr: Expr::Column {
                table: None,
                name: "a".to_string(),
            },
        });
    }
    // The refusal is the **predicate's** and not the insert path's, which is what the
    // `UPDATE` half below shows: one finalizer, `enforce_checks`, stands under both.
    // Its wider caller list is `docs/coercion.md`'s to state; this test reaches these two.
    let err = db
        .execute("INSERT INTO legacy VALUES (9)")
        .expect_err("the insert is refused");
    assert_eq!(err.sqlstate(), "42804");
    assert!(
        err.to_string().contains("argument of CHECK"),
        "the write must name the CHECK context: {err}"
    );
    // Seed a row past the guard so the `UPDATE` has something to reach, by adding the
    // constraint after the write rather than before it.
    let mut db2 = Database::with_btree();
    db2.execute("CREATE TABLE legacy_u (a INT)").expect("seeds");
    db2.execute("INSERT INTO legacy_u VALUES (1)")
        .expect("the row predates the constraint");
    db2.schema_mut()
        .tables
        .get_mut("legacy_u")
        .expect("the table")
        .checks
        .push(pglet_sql::catalog::CheckConstraint {
            name: "legacy_u_check".to_string(),
            expr: Expr::Column {
                table: None,
                name: "a".to_string(),
            },
        });
    assert_eq!(
        db2.execute("UPDATE legacy_u SET a = 2")
            .expect_err("the update is refused")
            .sqlstate(),
        "42804"
    );
}

/// A control for the one above, through the same planted route: a predicate that **is**
/// boolean still enforces normally, so the refusal above is the type's and not the
/// route's.
#[test]
fn a_boolean_predicate_planted_the_same_way_still_enforces() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE legacy3 (a INT)").expect("seeds");
    {
        let table = db
            .schema_mut()
            .tables
            .get_mut("legacy3")
            .expect("the table");
        table.checks.push(pglet_sql::catalog::CheckConstraint {
            name: "legacy3_check".to_string(),
            expr: Expr::BinaryOp {
                left: Box::new(Expr::Column {
                    table: None,
                    name: "a".to_string(),
                }),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Literal::Int(0))),
            },
        });
    }
    db.execute("INSERT INTO legacy3 VALUES (9)")
        .expect("a satisfying row is written");
    assert_eq!(
        db.execute("INSERT INTO legacy3 VALUES (-1)")
            .expect_err("a violating row is refused")
            .sqlstate(),
        "23514"
    );
}

/// **A nested join reports the enclosed join's verdict, not the enclosing join's.**
/// `transformFromClauseItem` transforms `j->larg` and `j->rarg` before the join's own
/// quals, so `resolve_join_on_types` recurses into both arms before resolving and
/// gating its own `ON`.
///
/// The measured shapes whose two verdicts carry different SQLSTATEs are pinned as
/// fixtures (`tests/fixtures/bool_context/nested_join_on_*`). The one that does not is
/// here because no fixture can discriminate it: both `ON`s fail the gate, so the SQLSTATE is `42804` whichever
/// is reached first and only the **operand named in the message** separates them —
/// `integer` for the inner `ON`, `text` for the outer. `@expect-message` cannot assert
/// it either, pglet's `42804` carrying a `type error: ` prefix 18.6 does not send.
#[test]
fn nested_join_on_reports_the_inner_joins_operand() {
    let mut db = Database::with_btree();
    for t in ["nj1", "nj2", "nj3"] {
        db.execute(&format!("CREATE TABLE {t} (k INT, t TEXT)"))
            .expect("seeds");
    }
    let err = db
        .execute("SELECT 1 FROM nj1 JOIN nj2 ON nj1.k JOIN nj3 ON nj2.t")
        .expect_err("both ON clauses fail the gate");
    assert_eq!(err.sqlstate(), "42804");
    let message = err.to_string();
    assert!(
        message.contains("not type integer"),
        "the inner JOIN/ON's operand answers: {message}"
    );
    assert!(
        !message.contains("not type text"),
        "the outer JOIN/ON's operand must not answer: {message}"
    );

    // The control: with the two operands swapped the named type swaps too, so the
    // assertion above reads the arm order and not a fixed answer.
    let swapped = db
        .execute("SELECT 1 FROM nj1 JOIN nj2 ON nj1.t JOIN nj3 ON nj2.k")
        .expect_err("both ON clauses fail the gate");
    assert!(
        swapped.to_string().contains("not type text"),
        "the inner JOIN/ON's operand answers: {swapped}"
    );
}

/// **Which construct a gate names is a second fact from the SQLSTATE it raises.**
/// `every_construct_has_its_own_spelling` proves each `BoolContext` names *itself*; it
/// says nothing about the right one being attached where the gate fires, and a `HAVING`
/// reported as a `WHERE` is a correct SQLSTATE under a misleading sentence that no
/// fixture in this family can catch, `@expect-message` being unusable while
/// `Error::Type`'s `type error: ` prefix stands. This drives statements and reads the
/// construct back out of the message; every expectation in the loop below is 18.6's own
/// construct, measured.
///
/// The last two cases carry the divergence and its control (`EDGES.md` row 126). At a
/// **nested** level the clause-root gate defers on a correlated reference, and by the
/// time the correlated re-walk gates the node, index selection has split the `AND` and
/// left the mis-typed conjunct alone in the `Plan::Filter` — so an accelerated build
/// reports `WHERE` where a stripped one and 18.6 report `AND`. The expectation is
/// per-profile rather than absolute, so this reddens if either profile moves.
#[test]
fn the_reported_construct_is_the_clause_as_written() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE rc1 (a INT)").expect("seeds");
    db.execute("CREATE TABLE rc2 (k INT)").expect("seeds");

    let construct = |db: &mut Database, sql: &str| -> String {
        let message = db
            .execute(sql)
            .expect_err("the gate refuses it")
            .to_string();
        let tail = message
            .split_once("argument of ")
            .unwrap_or_else(|| panic!("not a boolean-context message: {message}"))
            .1;
        tail.split_once(" must be type boolean")
            .unwrap_or_else(|| panic!("not a boolean-context message: {message}"))
            .0
            .to_string()
    };

    for (sql, expected) in [
        ("SELECT 1 FROM rc2 WHERE rc2.k", "WHERE"),
        ("SELECT 1 FROM rc2 WHERE rc2.k = 1 AND rc2.k", "AND"),
        ("SELECT 1 FROM rc2 WHERE rc2.k = 1 OR rc2.k", "OR"),
        ("SELECT 1 FROM rc2 WHERE NOT rc2.k", "NOT"),
        ("SELECT 1 FROM rc2 GROUP BY k HAVING max(k)", "HAVING"),
        (
            "SELECT 1 FROM rc1 WHERE EXISTS (SELECT 1 FROM rc2 WHERE rc2.k = 1 AND rc1.a)",
            "AND",
        ),
    ] {
        assert_eq!(construct(&mut db, sql), expected, "{sql}");
    }

    // The same correlated statement, with an index the planner selects between it and
    // the previous assertion and nothing else changed.
    db.execute("CREATE INDEX rc2_ix ON rc2 (k)").expect("seeds");
    assert_eq!(
        construct(
            &mut db,
            "SELECT 1 FROM rc1 WHERE EXISTS (SELECT 1 FROM rc2 WHERE rc2.k = 1 AND rc1.a)"
        ),
        if cfg!(feature = "index_acceleration") {
            "WHERE"
        } else {
            "AND"
        },
        "EDGES.md row 126: the construct a nested level reports follows the plan"
    );
    // The control that keeps the lost context the **root**'s: put a `NOT` between the
    // conjunct and the leaf and the residual is the `NOT` node, whose child is gated at
    // `NOT` whichever profile ran — so this one agrees with 18.6 under the same index.
    assert_eq!(
        construct(
            &mut db,
            "SELECT 1 FROM rc1 WHERE EXISTS (SELECT 1 FROM rc2 WHERE rc2.k = 1 AND NOT rc1.a)"
        ),
        "NOT"
    );
}

/// The construct half of a boolean-context refusal — `<CTX>` out of
/// `argument of <CTX> must be type boolean`. Panics on any other verdict shape, so a
/// statement that stopped reaching its gate is loud rather than silently unasserted.
fn refused_construct(db: &mut Database, sql: &str) -> String {
    construct_in(
        &db.execute(sql)
            .expect_err("the gate refuses it")
            .to_string(),
    )
}

/// [`refused_construct`] for a statement whose gate is the **eval-time** one: the `$1`
/// is deferred at plan time and the bind hands it an `integer` anyway, which is how
/// these cases put a non-boolean *value* at a truth site.
///
/// `inferred` is the OID the plan-time walk recorded for that `$1`, asserted here so a
/// case cannot start reaching its site for the wrong reason.
fn bound_refused_construct(db: &mut Database, sql: &str, inferred: Option<TypeOid>) -> String {
    let refused = db.execute_bound(sql, 1, |oids| {
        assert_eq!(oids, [inferred], "the inferred `$1` moved: {sql}");
        Ok::<_, ()>(vec![Some(pglet_sql::types::Value::Int4(1))])
    });
    match refused {
        Err(pglet::BoundExecError::Engine(e)) => construct_in(&e.to_string()),
        other => panic!("expected an eval-time refusal for {sql}, got {other:?}"),
    }
}

fn construct_in(message: &str) -> String {
    let tail = message
        .split_once("argument of ")
        .unwrap_or_else(|| panic!("not a boolean-context message: {message}"))
        .1;
    tail.split_once(" must be type boolean")
        .unwrap_or_else(|| panic!("not a boolean-context message: {message}"))
        .0
        .to_string()
}

/// **Each gated position names the construct 18.6 names there** — the per-position
/// half of what `every_construct_has_its_own_spelling` leaves open.
///
/// That test proves each `BoolContext` names *itself* and that no two collide; it is
/// blind to a *site* attaching the wrong member, which is a correct SQLSTATE under a
/// misleading sentence that every fixture in the corpus scores green. So is
/// `the_child_context_map_is_what_the_constructs_say`, which reads the map and drives
/// no statement. The gap that shape leaves is not hypothetical, and nothing outside
/// this file closes it: the fixture harness compares a message only under
/// `@expect-message`, no fixture in this family carries it (`Error::Type`'s
/// `type error: ` prefix makes it unusable), and the construct sentence is asserted in
/// no other test — so `JoinOn`'s spelling could be changed to `HAVING` with the corpus
/// green.
///
/// Every expectation below was **measured on 18.6** for the statement beside it. The
/// eval-time half is the exception and says so: 18.6 types a bare `$1` at a boolean
/// position and refuses the `integer` bind, so it never reaches a truth site at all;
/// what those cases assert is that the *same syntactic position* names the *same*
/// construct on both of pglet's two gates, which is the property the plan-time and
/// eval-time halves being one rule means.
///
/// **Each case is measured discriminating, not argued so**, by the one mutation its
/// site admits: where the site *constructs* a context, swap it for another member and
/// the case driving that site fails; where the site *forwards* one there is nothing to
/// swap, and the mutation is a **deletion** instead — the `AND` line below, whose
/// paragraph carries that measurement. The two are different mutations of the same
/// claim and not one word.
///
/// The sites those mutations were run at come from a list re-derived by the **caller**
/// predicate — every call that *enters* the gate (`check_boolean_clause`,
/// `check_boolean_operand`, `boolean_context_operand` and its `predicate_holds` /
/// `eval_not` wrappers), plus the statement positions of `validate_stored_predicate`.
/// **Re-derive that list, and re-derive what reaches each site on it**: the two are
/// separate questions and each has gone short. A grep for a `BoolContext`
/// **construction** cannot see a site that forwards one, which is what left
/// `executor::ddl`'s per-child gate off the list; and a list that names `eval_not` says
/// nothing about how many entries reach it — a `WHERE` and a grouping query's `HAVING`
/// are two, and a fixture named for the second
/// (`aggregate_having_not_non_boolean_loud.sql`) does not drive it: its `HAVING NOT 1` is
/// answered by the plan-time gate, reaching no evaluator at all.
///
/// **The predicate is the caller and not the datum**, for a measured reason: a site
/// can *forward* a context instead of constructing one, and `executor::ddl`'s
/// per-child gate inside `check_stored_expr_nodes` does, taking whatever
/// `try_for_each_child_ctx` hands it. The `AND` line below is what covers it, and the
/// measurement is re-runnable: drop that line and delete the call, and this file — and
/// `cargo test --workspace` entire — is green, because what the deletion costs is not a
/// different code but an *accept*
/// (`CREATE TABLE rcc (a INT, b BOOL, CHECK (a AND b))` reports `CREATE TABLE`).
///
/// **Something this deliberately does not reach**, named rather than left to look
/// covered: the partial-index predicate evaluated during **index maintenance**, whose
/// two copies are `executor::constraints`' `row_in_index` twin and `pglet_storage`'s
/// `index_keying`. A predicate reaches either only after passing the DDL gate, so
/// driving a non-boolean one needs a planted catalog *and* a planted `IndexSpec` in the
/// storage backend, which is a different test's subject.
#[test]
fn every_gated_position_names_its_own_construct() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE rcx (a INT)").expect("seeds");
    db.execute("CREATE TABLE rcy (k INT)").expect("seeds");
    db.execute("CREATE TABLE rcu (k INT PRIMARY KEY, v INT)")
        .expect("seeds");
    // A relation an eval-time case drives holds a row: an eval-time truth site over an
    // empty input is never reached at all, which is the data dependence the plan-time
    // gate exists to close. Measured — with these removed the eval-time half reaches no
    // site and `bound_refused_construct` panics on a statement that did not refuse, so
    // the seed is what lets that half assert rather than what keeps it honest.
    db.execute("INSERT INTO rcx VALUES (1)").expect("seeds");
    db.execute("INSERT INTO rcy VALUES (1)").expect("seeds");
    db.execute("INSERT INTO rcu VALUES (1, 1)").expect("seeds");

    // Plan time: a column of the wrong type at the position, so the gate reads a type.
    for (sql, expected) in [
        ("SELECT 1 FROM rcy WHERE rcy.k", "WHERE"),
        ("SELECT 1 FROM rcy GROUP BY k HAVING max(k)", "HAVING"),
        ("SELECT 1 FROM rcx a JOIN rcy c ON c.k", "JOIN/ON"),
        ("SELECT CASE WHEN rcy.k THEN 1 END FROM rcy", "CASE/WHEN"),
        ("SELECT 1 FROM rcy WHERE rcy.k AND k = 1", "AND"),
        ("SELECT 1 FROM rcy WHERE k = 1 AND rcy.k", "AND"),
        ("SELECT 1 FROM rcy WHERE rcy.k OR k = 1", "OR"),
        ("SELECT 1 FROM rcy WHERE k = 1 OR rcy.k", "OR"),
        ("SELECT 1 FROM rcy WHERE NOT rcy.k", "NOT"),
        ("SELECT 1 FROM rcy WHERE rcy.k IS TRUE", "IS TRUE"),
        ("SELECT 1 FROM rcy WHERE rcy.k IS NOT FALSE", "IS NOT FALSE"),
        ("UPDATE rcy SET k = 1 WHERE rcy.k", "WHERE"),
        ("UPDATE rcy SET k = 1 FROM rcx WHERE rcx.a", "WHERE"),
        ("DELETE FROM rcy WHERE rcy.k", "WHERE"),
        (
            "INSERT INTO rcu VALUES (1, 1) ON CONFLICT (k) DO UPDATE SET v = 1 WHERE rcu.k",
            "WHERE",
        ),
        // DDL time — the stored-predicate clause roots (`Check` and `Where`), plus the
        // per-child gate inside the node walk that runs ahead of them, which forwards
        // its context rather than constructing one. The eval-time `Check` site is a
        // different one, the write path's, and what drives that is
        // `a_predicate_an_older_build_persisted_fails_loud_at_the_write`.
        ("CREATE TABLE rcc (a INT, CHECK (a))", "CHECK"),
        ("CREATE TABLE rcc (a INT, CHECK (a IS TRUE))", "IS TRUE"),
        ("CREATE TABLE rcc (a INT, b BOOL, CHECK (a AND b))", "AND"),
        ("CREATE INDEX rcy_ix ON rcy (k) WHERE rcy.k", "WHERE"),
        // The correlated re-walk reads the `WHERE`, `HAVING` and `JOIN … ON` roots
        // again, carrying its own context per clause rather than inheriting the
        // enclosing level's.
        (
            "SELECT 1 FROM rcx WHERE EXISTS (SELECT 1 FROM rcy WHERE rcx.a)",
            "WHERE",
        ),
        (
            "SELECT 1 FROM rcx WHERE EXISTS (SELECT 1 FROM rcy GROUP BY k HAVING rcx.a)",
            "HAVING",
        ),
        (
            "SELECT 1 FROM rcx WHERE EXISTS (SELECT 1 FROM rcy x JOIN rcy y ON rcx.a)",
            "JOIN/ON",
        ),
    ] {
        assert_eq!(refused_construct(&mut db, sql), expected, "{sql}");
    }

    // Eval time: the same positions reached with a value instead of a type.
    for (sql, expected) in [
        ("SELECT 1 FROM rcy WHERE $1", "WHERE"),
        ("SELECT max(k) FROM rcy GROUP BY k HAVING $1", "HAVING"),
        ("SELECT 1 FROM rcx a JOIN rcy c ON $1", "JOIN/ON"),
        ("SELECT CASE WHEN $1 THEN 1 END FROM rcy", "CASE/WHEN"),
        // Both operands, because the two sides are two call sites — each mutated on its
        // own to prove the pair is watched rather than one standing in for both. The
        // left operand of each right-side case leaves the connective undecided over
        // `rcy`'s row, or the right is never evaluated (`connective_decided_by`).
        ("SELECT 1 FROM rcy WHERE $1 AND k = 1", "AND"),
        ("SELECT 1 FROM rcy WHERE k = 1 AND $1", "AND"),
        ("SELECT 1 FROM rcy WHERE $1 OR k = 1", "OR"),
        ("SELECT 1 FROM rcy WHERE k <> 1 OR $1", "OR"),
        ("SELECT 1 FROM rcy WHERE NOT $1", "NOT"),
        // A grouping query's `HAVING` reaches `NOT` through the scalar walk's arm too,
        // the grouping lending that walk its aggregates' values; this line watches the
        // `HAVING` entry to it.
        ("SELECT max(k) FROM rcy GROUP BY k HAVING NOT $1", "NOT"),
        ("UPDATE rcy SET k = 1 WHERE $1", "WHERE"),
        ("UPDATE rcy SET k = 1 FROM rcx WHERE $1", "WHERE"),
        ("DELETE FROM rcy WHERE $1", "WHERE"),
        (
            "INSERT INTO rcu VALUES (1, 1) ON CONFLICT (k) DO UPDATE SET v = 1 WHERE $1",
            "WHERE",
        ),
    ] {
        assert_eq!(
            bound_refused_construct(&mut db, sql, Some(TypeOid::Bool)),
            expected,
            "{sql}"
        );
    }

    // The `IS [NOT] TRUE` operand is a boolean position the *inference* does not
    // reach: its gate is an ordinary post-order check rather than a child context, so
    // `for_each_child_ctx` hands it none and the `$1` stays OID 0
    // (`docs/param_binding.md` § Deferred / known gaps). The site still names itself,
    // which is what this asserts; the `None` beside it is the residual, and this
    // reddens the day it closes.
    for sql in [
        "SELECT 1 FROM rcy WHERE $1 IS TRUE",
        // The **aggregate** walk is a second expression evaluator with its own
        // `BooleanTest` arm, so the same spelling reached through a `HAVING` is a
        // second site; mutating either alone leaves the other green.
        "SELECT max(k) FROM rcy GROUP BY k HAVING $1 IS TRUE",
    ] {
        assert_eq!(
            bound_refused_construct(&mut db, sql, None),
            "IS TRUE",
            "{sql}"
        );
    }

    // The **GIN guard** disjunct: on the accelerated profile a top-level `OR` beside a
    // containment probe is lifted out of the `Filter` and judged on its own, under the
    // `OR` it sits in. A guard that fails there hands the recheck `Filter` every row,
    // so the refusal is the `Filter`'s ordinary `OR` arm's, raised for the row whose
    // probe disjunct is `false` — the one row here. On the stripped profile there is
    // no lift and that arm answers directly, the same construct either way.
    db.execute("CREATE TABLE rcg (j JSONB)").expect("seeds");
    db.execute("CREATE INDEX rcg_gin ON rcg USING gin (j)")
        .expect("seeds");
    db.execute("INSERT INTO rcg VALUES ('{\"a\": 2}')")
        .expect("seeds");
    assert_eq!(
        bound_refused_construct(
            &mut db,
            "SELECT 1 FROM rcg WHERE j @> '{\"a\": 1}' OR $1",
            Some(TypeOid::Bool),
        ),
        "OR"
    );
}
