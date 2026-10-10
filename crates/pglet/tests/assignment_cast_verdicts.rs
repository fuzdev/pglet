//! The assignment-cast gate's two skips — the part of that surface no differential
//! fixture can reach.
//!
//! `@expect-error` compares **SQLSTATE**, and both rules here answer a code 18.6 does
//! not: each is a statement 18.6 refuses `42P01` in parse analysis while pglet resolves
//! the relation at execution, so a faithful fixture would be red whichever way the code
//! went. What is worth guarding is not the code but the **shape** of the answer —
//! whether the statement is refused at all, and by which gate — and that is what these
//! assert.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface a setup failure or an
// unexpected verdict shape loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pglet::Database;

/// The SQLSTATE `sql` fails with, or `None` when it succeeds.
fn state(db: &mut Database, sql: &str) -> Option<&'static str> {
    db.execute(sql).err().map(|e| e.sqlstate())
}

/// A fresh database holding `gi (id GENERATED ALWAYS AS IDENTITY, v INT)` and no rows,
/// so an `UPDATE` over it matches nothing.
fn identity_db() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE gi (id INT GENERATED ALWAYS AS IDENTITY, v INT)")
        .expect("seeds");
    db
}

/// The identity guard keeps its `428C9` over an unresolvable relation in the `WHERE` or
/// the `RETURNING`, and yields to one in the `FROM` — the split `lower_update`'s
/// `from_relations_resolvable` draws, which is **not** "every clause 18.6 resolves
/// first".
///
/// 18.6 answers `42P01` to all three (`transformUpdateStmt` resolves FROM, `WHERE` and
/// `RETURNING` before the target list, and the identity refusal is a rewriter check
/// behind all of it). pglet raises that `42P01` at **execution**, and only the `FROM` is
/// a plan node the executor runs whatever the row count: a `WHERE` predicate and a
/// `RETURNING` item are evaluated per target row, so over the empty table here neither
/// is ever reached. Declining for those two would leave a statement 18.6 refuses
/// reporting `UPDATE 0`, so the guard keeps its wrong-but-loud code.
///
/// The residual that buys, measured on both engines and stated so nothing reads it as
/// fidelity: over a **matching** row the wide skip answered 18.6's own `42P01` for the
/// `WHERE` and `RETURNING` spellings, and this answers `428C9`.
#[test]
fn the_identity_guard_is_loud_over_a_missing_per_row_clause_relation() {
    let mut db = identity_db();
    assert_eq!(
        state(
            &mut db,
            "UPDATE gi SET id = 5 WHERE 1 = 0 RETURNING (SELECT 1 FROM nosuchrel)",
        ),
        Some("428C9"),
        "a RETURNING skip would report success on a statement 18.6 refuses",
    );
    assert_eq!(
        state(
            &mut db,
            "UPDATE gi SET id = 5 WHERE EXISTS (SELECT 1 FROM nosuchrel)",
        ),
        Some("428C9"),
        "and so would a WHERE skip — the predicate is per row, and there is no row",
    );
    assert_eq!(
        state(&mut db, "UPDATE gi SET id = 5 FROM nosuchrel"),
        Some("42P01"),
        "the FROM half of the skip stands: that scan runs, so pglet's 42P01 is 18.6's answer",
    );
}

/// A right-hand side whose own subquery names a relation the catalog lacks leaves the
/// assignment gate off (`assignment_source_type` answers `Opaque`), so the statement
/// keeps pglet's own execution-time verdict instead of a plan-time `42804` about the
/// wrong thing.
///
/// 18.6 answers `42P01` to all three statements below and pglet answers none of them,
/// which is why no fixture can hold this rule in either direction — but the rule is
/// still a choice between two pglet answers, and deleting the `Opaque` arm changes
/// `0A000` to `42804` here. That is what this asserts.
///
/// The zero-row spelling is the one that shows what the rule cannot fix: pglet reports
/// success, because a scalar subquery in a `SET` right-hand side is only evaluated
/// against a row. Recorded in `EDGES.md`.
#[test]
fn a_missing_relation_in_a_set_subquery_keeps_the_execution_verdict() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE sqt (id INT, b BOOL)")
        .expect("seeds");
    db.execute("INSERT INTO sqt VALUES (1, false)")
        .expect("seeds");
    assert_eq!(
        state(&mut db, "UPDATE sqt SET b = (SELECT n FROM sq_nosuch)"),
        Some("0A000"),
        "gating an Opaque source answers 42804 about types nothing in the statement has",
    );
    assert_eq!(
        state(
            &mut db,
            "INSERT INTO sqt (b) VALUES ((SELECT n FROM sq_nosuch))",
        ),
        Some("0A000"),
        "the INSERT … VALUES spelling of the same rule",
    );
    assert_eq!(
        state(
            &mut db,
            "UPDATE sqt SET b = (SELECT n FROM sq_nosuch) WHERE 1 = 0",
        ),
        None,
        "and with no row to evaluate against, the statement reports success — 18.6 is 42P01",
    );
}
