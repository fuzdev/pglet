//! The forcing function for **an aggregate under every node the scalar walk evaluates**.
//!
//! A grouping query evaluates its select list, `ORDER BY` junk columns and `HAVING`
//! through the ordinary scalar walk, which reads each aggregate's value through the
//! lookup the grouping executor lends it (`SubqueryExec::aggregate`). So an aggregate
//! under a node evaluates **only where that node's arm hands the lookup down** to the
//! child holding it. An arm that evaluated a child through a context with no lookup
//! (`SubqueryExec::NONE`) would compile clean and be `0A000` at runtime, on queries 18.6
//! answers.
//!
//! The guard is the exhaustive `match` in [`aggregate_arm`]: **adding a variant to
//! `Expr` fails this file to compile** until someone decides it, and a variant that can
//! sit above an aggregate must supply a SQL spelling that puts one under it — which
//! [`every_compound_arm_evaluates_an_aggregate`] then runs in both the projection and
//! `HAVING`. A claim with nothing behind it would be a comment; this makes it a test.

// A test harness: `.expect()` surfaces a setup failure loudly right here.
#![allow(clippy::expect_used)]

use pglet::Database;
use pglet_sql::planner::Expr;

/// What a grouping query does with an aggregate under a variant.
enum AggregateArm {
    /// The scalar walk evaluates the variant and hands the aggregate lookup to its
    /// operands, so an aggregate below it evaluates. `projection` is a SQL projection
    /// that builds exactly that shape; `having` is the same shape as a `HAVING`
    /// predicate, or `None` for a variant the grammar admits in no such position (an
    /// output alias is the only one).
    Evaluates {
        projection: &'static str,
        having: Option<&'static str>,
    },
    /// No aggregate of this query is evaluated below the variant: it is a leaf, the
    /// aggregate itself, a node another executor evaluates, or a sub-plan.
    NoAggregateBelow,
}

/// One decision per `Expr` variant. Exhaustive **without `..`** on purpose — see the
/// module doc.
//
// `match_same_arms`: the `NoAggregateBelow` groups are kept apart by *reason* (a leaf, a
// sub-plan, the aggregate itself, another executor's node), which is the whole content of
// the decision this file exists to force.
#[allow(clippy::match_same_arms, clippy::missing_const_for_fn)]
fn aggregate_arm(expr: &Expr) -> AggregateArm {
    use AggregateArm::{Evaluates, NoAggregateBelow};
    /// The common case: the same spelling works in both positions.
    const fn both(spelling: &'static str) -> AggregateArm {
        Evaluates {
            projection: spelling,
            having: Some(spelling),
        }
    }
    match expr {
        // Leaves.
        Expr::Literal(_)
        | Expr::Column { .. }
        | Expr::Default
        | Expr::Parameter { .. }
        | Expr::Bound(_)
        | Expr::Builtin(_)
        | Expr::SequenceFn { .. } => NoAggregateBelow,
        // Sub-plans: an aggregate inside one is the inner query's, computed by the inner
        // query's own grouping. `IN (SELECT …)`'s left operand is an aggregate of this
        // query's, but a grouping query lends its expressions no sub-plan callback
        // (`EDGES.md` row 89), so that shape is refused before the operand matters.
        Expr::Subquery { .. } | Expr::Exists { .. } | Expr::InSubquery { .. } => NoAggregateBelow,
        // The aggregate itself, a window function (the window executor's), and a
        // set-returning `unnest`, which the scalar walk refuses wherever it stands.
        Expr::Aggregate { .. } | Expr::Window { .. } | Expr::Unnest(_) => NoAggregateBelow,
        // An output alias has no `HAVING` spelling — `HAVING (x AS a)` is not grammar on
        // either engine — so this arm is the reason `having` is optional.
        Expr::Alias { .. } => Evaluates {
            projection: "sum(i) AS s",
            having: None,
        },
        Expr::BinaryOp { .. } => both("sum(i) + 1"),
        Expr::UnaryPlus(_) => both("+ sum(i)"),
        Expr::Negate(_) => both("- sum(i)"),
        Expr::Not(_) => both("NOT (count(*) > 1)"),
        Expr::IsNull(_) => both("sum(i) IS NULL"),
        Expr::IsNotNull(_) => both("sum(i) IS NOT NULL"),
        Expr::Cast { .. } => both("sum(i)::text"),
        Expr::CastTypmod { .. } => both("sum(i)::numeric(10,2)"),
        Expr::Coalesce(_) => both("COALESCE(sum(i), 0)"),
        Expr::BooleanTest { .. } => both("(count(*) > 1) IS TRUE"),
        // The operand and an item both, so the list's own walk is covered as well as
        // the operand's.
        Expr::InList { .. } => both("max(i) IN (1, count(*))"),
        Expr::Array(_) => both("ARRAY[max(i), min(i)]"),
        Expr::JsonbBuildObject(_) => both("jsonb_build_object('n', count(*))"),
        Expr::AnyAll { .. } => both("max(i) = ANY (ARRAY[min(i), 30])"),
        Expr::Lower(_) => both("lower(max(i)::text)"),
        Expr::Case { .. } => both("CASE WHEN count(*) > 1 THEN max(i) ELSE min(i) END"),
        Expr::MathFn { .. } => both("abs(sum(i))"),
        Expr::StringFn { .. } => both("upper(max(i)::text)"),
        Expr::VariadicFn { .. } => both("greatest(max(i), min(i))"),
        Expr::Extract { .. } => both("extract(year FROM max(d))"),
        // A computed argument is refused when it is not NULL (its bytes are not
        // modelled), so the shape makes it NULL: the aggregates under it still evaluate.
        Expr::ColumnSize { .. } => Evaluates {
            projection: "pg_column_size(nullif(max(i), max(i)))",
            having: Some("pg_column_size(nullif(max(i), max(i))) IS NULL"),
        },
    }
}

/// Every `Expr` variant, as a value, so [`aggregate_arm`] can be asked of each. The
/// shapes are irrelevant — only the discriminant is read.
fn every_variant() -> Vec<Expr> {
    let boxed = |e: Expr| Box::new(e);
    let lit = || Expr::Literal(pglet_sql::planner::Literal::Int(1));
    vec![
        lit(),
        Expr::Column {
            table: None,
            name: "c".into(),
        },
        Expr::BinaryOp {
            left: boxed(lit()),
            op: pglet_sql::planner::BinaryOp::Add,
            right: boxed(lit()),
        },
        Expr::Negate(boxed(lit())),
        Expr::UnaryPlus(boxed(lit())),
        Expr::Not(boxed(lit())),
        Expr::IsNull(boxed(lit())),
        Expr::IsNotNull(boxed(lit())),
        Expr::Default,
        Expr::Cast {
            expr: boxed(lit()),
            target_type: pglet_sql::types::TypeOid::Text,
        },
        Expr::CastTypmod {
            expr: boxed(lit()),
            type_oid: pglet_sql::types::TypeOid::Numeric,
            typmod: pglet_sql::planner::Typmod::Numeric {
                precision: 10,
                scale: 2,
            },
        },
        Expr::Alias {
            expr: boxed(lit()),
            alias: "a".into(),
        },
        Expr::Aggregate {
            func: pglet_sql::planner::AggregateFunc::Count,
            arg: boxed(lit()),
            distinct: false,
            certainty: pglet_sql::planner::ArgCertainty::NotCertainInteger,
        },
        Expr::Parameter { index: 1 },
        Expr::Bound(pglet_sql::types::Value::Null),
        Expr::Builtin(pglet_sql::planner::BuiltinFn::Now),
        Expr::Array(vec![lit()]),
        Expr::JsonbBuildObject(vec![]),
        Expr::AnyAll {
            left: boxed(lit()),
            op: pglet_sql::planner::BinaryOp::Eq,
            right: boxed(lit()),
            all: false,
            sub_select: false,
        },
        Expr::Unnest(boxed(lit())),
        Expr::Lower(boxed(lit())),
        Expr::Coalesce(vec![lit()]),
        Expr::Case {
            operand: None,
            when_clauses: vec![],
            else_result: None,
        },
        Expr::MathFn {
            func: pglet_sql::planner::MathFunc::Abs,
            args: vec![lit()],
        },
        Expr::StringFn {
            func: pglet_sql::planner::StringFunc::Upper,
            args: vec![lit()],
        },
        Expr::VariadicFn {
            func: pglet_sql::planner::VariadicFunc::Concat,
            args: vec![lit()],
        },
        Expr::Extract {
            source: boxed(lit()),
            field: boxed(lit()),
            retnumeric: true,
        },
        Expr::SequenceFn {
            func: pglet_sql::planner::SequenceFunc::Currval,
            seq: "s".into(),
            args: vec![],
        },
        Expr::InList {
            expr: boxed(lit()),
            list: vec![lit()],
            negated: false,
        },
        Expr::Window {
            func: pglet_sql::planner::WindowFunc::RowNumber,
            partition_by: vec![],
            order_by: vec![],
        },
        Expr::BooleanTest {
            expr: boxed(lit()),
            test: pglet_sql::planner::BoolTest::True,
        },
        Expr::ColumnSize {
            arg: boxed(lit()),
            origin: pglet_sql::planner::ColumnSizeOrigin::Unclassified,
        },
    ]
}

/// Every arm that claims the walk evaluates an aggregate below it must actually do so —
/// in **all three** positions a grouping query evaluates: the projection, an
/// `ORDER BY` key (a junk column of the same node), and `HAVING`.
#[test]
fn every_compound_arm_evaluates_an_aggregate() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, i INT, d DATE)")
        .expect("seeds");
    db.execute(
        "INSERT INTO t VALUES (1, 10, '2024-01-15'), (1, 20, '2024-02-01'), \
         (2, 30, '2025-03-01')",
    )
    .expect("seeds");
    for expr in every_variant() {
        let AggregateArm::Evaluates { projection, having } = aggregate_arm(&expr) else {
            continue;
        };
        let projected = format!("SELECT {projection} FROM t");
        assert!(
            db.execute(&projected).is_ok(),
            "projection `{projected}` should evaluate its aggregate"
        );
        // `HAVING` is a second entry to the grouping, and an `ORDER BY` key a third (a
        // junk column, evaluated with the select list); a variant reachable from one and
        // not another would pass a projection-only check.
        if let Some(having) = having {
            let clause = format!("SELECT id FROM t GROUP BY id HAVING ({having}) IS NOT NULL");
            assert!(
                db.execute(&clause).is_ok(),
                "`{clause}` should evaluate its aggregate"
            );
            let ordered = format!("SELECT id FROM t GROUP BY id ORDER BY ({having}) IS NULL, id");
            assert!(
                db.execute(&ordered).is_ok(),
                "`{ordered}` should evaluate its aggregate"
            );
        }
    }
}

/// An aggregate reached by an evaluation **no grouping lends values to** is refused, not
/// answered: the lookup is absent there, and the walk never invents a value for the node.
/// A `RETURNING` aggregate is such a position (`EDGES.md` row 4, where 18.6 refuses at
/// parse analysis instead).
#[test]
fn an_aggregate_with_no_grouping_behind_it_is_loud() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT, i INT)").expect("seeds");
    db.execute("INSERT INTO t VALUES (1, 10)").expect("seeds");
    let err = db
        .execute("DELETE FROM t WHERE id = 1 RETURNING abs(count(*))")
        .expect_err("an aggregate no grouping computes is refused");
    assert_eq!(err.sqlstate(), "0A000");
}

/// An operator pglet implements nothing for is refused `42883` with its operands' types
/// named, and the message is built while the statement is parsed — before the coercion
/// pass has recorded any aggregate's argument certainty. So an aggregate operand is
/// typed there by a reader that asks for no such record: `count(*)` is `bigint`, and an
/// aggregate over a column the parser cannot type is `unknown`, as the column is.
#[test]
fn an_unknown_operator_over_an_aggregate_is_refused_with_its_type() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE o (a INT, b INT, t TEXT)")
        .expect("seeds");
    db.execute("CREATE TABLE sink (x INT)").expect("seeds");
    db.execute("INSERT INTO o VALUES (1, 10, 'x')")
        .expect("seeds");
    for (sql, message) in [
        (
            "SELECT count(*) & 1 FROM o",
            "operator does not exist: bigint & integer",
        ),
        (
            "SELECT max(a) & 1 FROM o",
            "operator does not exist: unknown & integer",
        ),
        (
            "SELECT |/ count(b) FROM o",
            "operator does not exist: |/ bigint",
        ),
        (
            "SELECT @ avg(1) FROM o",
            "operator does not exist: @ numeric",
        ),
        (
            "SELECT |/ coalesce(max(a), 0) FROM o",
            "operator does not exist: |/ integer",
        ),
        (
            "SELECT a FROM o GROUP BY a HAVING count(*) # 1 > 0",
            "operator does not exist: bigint # integer",
        ),
        (
            "SELECT (SELECT count(*) <-> 1 FROM o)",
            "operator does not exist: bigint <-> integer",
        ),
        (
            "INSERT INTO sink SELECT count(*) & 1 FROM o",
            "operator does not exist: bigint & integer",
        ),
    ] {
        let err = db.execute(sql).expect_err("an unknown operator is refused");
        assert_eq!(err.sqlstate(), "42883", "{sql}");
        assert_eq!(err.to_string(), message, "{sql}");
    }
    // An `ORDER BY` key is refused under another code before the operator's refusal is
    // raised; what this holds is that the message was built without a panic.
    assert!(
        db.execute("SELECT a FROM o GROUP BY a ORDER BY max(a) & 1")
            .is_err()
    );
}

/// An aggregate's operand type is refused for the statement, ahead of every group and
/// in the order 18.6 analyses the clauses in: the select list's refusal is raised before
/// `HAVING`'s, and either before a row is aggregated. The fixtures compare the SQLSTATE,
/// which both refusals share, so the order is held here by the message.
#[test]
fn an_operand_type_refusal_is_raised_in_clause_order_ahead_of_the_groups() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE o (a INT, b INT, t TEXT, bo BOOL)")
        .expect("seeds");
    db.execute("INSERT INTO o VALUES (1, 0, 'x', true)")
        .expect("seeds");
    for (sql, message) in [
        (
            "SELECT max(bo) FROM o HAVING sum(t) > 0",
            "function max(boolean) does not exist",
        ),
        (
            "SELECT true OR max(bo) FROM o HAVING false AND sum(t) > 0",
            "function max(boolean) does not exist",
        ),
        (
            "SELECT count(*) FROM o HAVING false AND sum(t) > 0",
            "function sum(text) does not exist",
        ),
        // Ahead of the division a kept aggregate would fail on.
        (
            "SELECT sum(1/b), true OR sum(t) > 0 FROM o",
            "function sum(text) does not exist",
        ),
        (
            "SELECT sum(1/b) FROM o HAVING true OR min(bo)",
            "function min(boolean) does not exist",
        ),
    ] {
        let err = db.execute(sql).expect_err("the operand type is refused");
        assert_eq!(err.sqlstate(), "42883", "{sql}");
        assert_eq!(err.to_string(), message, "{sql}");
    }
}
