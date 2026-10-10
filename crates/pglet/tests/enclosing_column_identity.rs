//! A sub-plan's target-list reference to an **enclosing** level's column is described with
//! the identity of the column it binds to — on the sub-plan's own output descriptor, which
//! every reader of the sub-plan reads, and at the readers the differential fixtures cannot
//! reach: a `$N`'s inferred type and the assignment gate of an `UPDATE`, which runs before
//! the statement's own correlated re-run.
//!
//! The fixtures (`*/correlated_target_list_enclosing_identity_*.sql`) hold the answers; these
//! hold the record itself and the reader that reads it earliest.

// A test harness: setup `.unwrap()`s, so a setup failure surfaces loudly here rather
// than as a confusing assertion mismatch.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pglet::Database;
use pglet_sql::planner::{Expr, Plan};
use pglet_sql::types::TypeOid;

fn database() -> Database {
    let mut db = Database::in_memory();
    db.execute(
        "CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
         CREATE TABLE te (m mood, k INT);
         CREATE TABLE tt (m TEXT, k TEXT);
         CREATE TABLE p (a INT, b INT);
         CREATE TABLE q (c INT);
         CREATE TABLE q3 (c INT, b TEXT);
         INSERT INTO te VALUES ('happy', 1), ('sad', 2);
         INSERT INTO p VALUES (1, 10), (2, 20);
         INSERT INTO q VALUES (5);",
    )
    .unwrap();
    db
}

/// The sub-plan of the first scalar subquery in `plan`'s first target-list entry.
fn first_sub_plan(plan: &Plan) -> &Plan {
    let entry = &plan.projection_exprs()[0];
    let entry = match entry {
        Expr::Alias { expr, .. } => expr.as_ref(),
        other => other,
    };
    let Expr::Subquery { plan } = entry else {
        unreachable!("expected a scalar subquery, found {entry:?}");
    };
    plan
}

/// `(type_oid, enum_type)` of the sub-plan's single output column.
fn sub_plan_column(db: &Database, sql: &str) -> (TypeOid, Option<String>) {
    let schema = db.schema();
    let plans = pglet_sql::parser::parse(sql, schema).expect("should plan");
    let columns = first_sub_plan(&plans[0]).output_columns(schema);
    assert_eq!(columns.len(), 1, "{sql}");
    (columns[0].type_oid, columns[0].enum_type.clone())
}

#[test]
fn a_sub_plans_column_is_the_enclosing_columns_identity() {
    let db = database();
    // The inner level carries a same-named `text` column; the reference binds the enclosing
    // `mood` column, and the sub-plan's descriptor says so.
    assert_eq!(
        sub_plan_column(&db, "SELECT (SELECT o.m FROM tt b) FROM te o"),
        (TypeOid::Text, Some("mood".to_string()))
    );
    // A same-named `text` column, a bare name the inner level lacks, and a computed entry.
    for sql in [
        "SELECT (SELECT p.b FROM q3) FROM p",
        "SELECT (SELECT b FROM q) FROM p",
        "SELECT (SELECT p.b + 1 FROM q3) FROM p",
    ] {
        assert_eq!(sub_plan_column(&db, sql), (TypeOid::Int4, None), "{sql}");
    }
    // A no-FROM level binds nothing, so its reference is the enclosing column too.
    assert_eq!(
        sub_plan_column(&db, "SELECT (SELECT o.m) FROM te o"),
        (TypeOid::Text, Some("mood".to_string()))
    );
    // Control: a bare name the inner level carries binds there.
    assert_eq!(
        sub_plan_column(&db, "SELECT (SELECT b FROM q3) FROM p"),
        (TypeOid::Text, None)
    );
}

#[test]
fn a_parameter_beside_the_sub_plan_takes_the_enclosing_columns_type() {
    let db = database();
    let schema = db.schema();
    for sql in [
        "SELECT (SELECT p.b FROM q3) = $1 FROM p",
        "SELECT (SELECT p.b FROM q) = $1 FROM p",
    ] {
        let plans = pglet_sql::parser::parse(sql, schema).expect("should plan");
        let types = pglet_sql::planner::infer_param_types(&plans[0], schema, &[]).unwrap();
        assert_eq!(types, [Some(TypeOid::Int4)], "{sql}");
    }
}

/// The `UPDATE`'s assignment gate runs at the statement's own lowering, ahead of its
/// correlated re-run, and reads the sub-plan's descriptor there — the record the
/// judgement of the nested level wrote on its copy, which shares it. Unshared, the gate
/// read `text` and refused `42804`; what is left is `EDGES.md` row 89's missing evaluation
/// context, `0A000`, where 18.6 answers `10`, `20`.
#[test]
fn the_assignment_gate_reads_the_record_ahead_of_the_re_run() {
    let mut db = database();
    let err = db
        .execute("UPDATE p SET a = (SELECT p.b FROM q) RETURNING a")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "0A000", "{err}");
}
