//! [`expr_enum_with`]'s node rule, arm by arm.
//!
//! The differential fixtures pin the arms a query can *reach* (`COALESCE`, `CASE`,
//! `greatest`/`least`, `nullif`, `min`/`max`, and — since it stopped being a boundary —
//! `Subquery`); these pin the ones whose whole content is a **`None`**, which a fixture
//! cannot express, because a fixture can only compare two answers and 18.6 is right
//! where pglet declines. Over-widening the walk (giving `Cast` an arm, say) would
//! silently change what `EDGES.md` row 104 records, and nothing else here would notice.
//!
//! The `Subquery` arm is here for the opposite reason and is the cautionary one: it
//! *was* one of those `None`s, and the note calling it unreachable had measured exactly
//! one position (`ORDER BY (SELECT …)`, `0A000` here for a reason that has nothing to do
//! with enums). Its two tests pin that the arm **asks**, and a deliberately blind leaf
//! separates that from an arm answering on its own.

use super::super::enum_order::{EnumLeaves, expr_enum_with};
use crate::planner::{BinaryOp, Expr, Literal, Plan, VariadicFunc};
use crate::types::TypeOid;

/// The column leaf: column `m` is `mood`, column `n` is `weather`, anything else is
/// not an enum. Instantiated at the enum's **name**, the descriptor site's `T`.
fn leaf(_table: Option<&str>, name: &str) -> Option<String> {
    match name {
        "m" => Some("mood".to_string()),
        "n" => Some("weather".to_string()),
        _ => None,
    }
}

/// The subquery leaf, standing in for `subquery_enum_name`: this module holds no
/// catalog, and what the arm has to be pinned on is that it **asks** — the arm was a
/// bare `None` for a whole slice, which is why every test below that reaches it
/// would have passed against a plan of any shape.
#[allow(
    clippy::unnecessary_wraps,
    reason = "the shape is `EnumLeaves::subquery`'s, and `walk_blind` is the same \
                  signature answering `None`"
)]
fn subquery_leaf(_plan: &Plan) -> Option<String> {
    Some("mood".to_string())
}

/// The subquery leaf a scope with no catalog answers with — the state
/// `ProjectionScope::expr_enum_name` is in when it carries no schema.
fn blind_subquery_leaf(_plan: &Plan) -> Option<String> {
    None
}

fn walk(e: &Expr) -> Option<String> {
    expr_enum_with(
        e,
        &EnumLeaves {
            column: &leaf,
            subquery: &subquery_leaf,
        },
    )
}

/// The same walk with the subquery leaf blind, so a test can separate "the arm asked
/// and the leaf answered" from "the arm answered `None` on its own".
fn walk_blind(e: &Expr) -> Option<String> {
    expr_enum_with(
        e,
        &EnumLeaves {
            column: &leaf,
            subquery: &blind_subquery_leaf,
        },
    )
}

fn subquery() -> Expr {
    Expr::Subquery {
        plan: Box::new(Plan::Empty),
    }
}

fn col(name: &str) -> Expr {
    Expr::Column {
        table: None,
        name: name.to_string(),
    }
}

fn lit(s: &str) -> Expr {
    Expr::Literal(Literal::String(s.to_string()))
}

#[test]
fn a_column_is_the_leaf_and_a_wrapper_passes_it_through() {
    assert_eq!(walk(&col("m")).as_deref(), Some("mood"));
    assert_eq!(walk(&col("k")), None);
    assert_eq!(
        walk(&Expr::Alias {
            expr: Box::new(col("m")),
            alias: "x".to_string(),
        })
        .as_deref(),
        Some("mood")
    );
    assert_eq!(
        walk(&Expr::UnaryPlus(Box::new(col("m")))).as_deref(),
        Some("mood")
    );
}

#[test]
fn a_type_unifying_node_takes_its_operands_enum() {
    assert_eq!(
        walk(&Expr::Coalesce(vec![col("m"), lit("sad")])).as_deref(),
        Some("mood")
    );
    assert_eq!(
        walk(&Expr::VariadicFn {
            func: VariadicFunc::Greatest,
            args: vec![lit("sad"), col("m")],
        })
        .as_deref(),
        Some("mood")
    );
    // `nullif`'s enum comes from *either* argument, because 18.6 coerces the pair
    // through `make_op` before reading the first one's type.
    assert_eq!(
        walk(&Expr::VariadicFn {
            func: VariadicFunc::NullIf,
            args: vec![lit("ok"), col("m")],
        })
        .as_deref(),
        Some("mood")
    );
}

#[test]
fn case_reads_its_results_and_not_its_conditions() {
    // The operand and the `WHEN` feed the comparison; only `THEN`/`ELSE` are the
    // result type. A `mood` operand under a non-enum result must not leak.
    assert_eq!(
        walk(&Expr::Case {
            operand: Some(Box::new(col("m"))),
            when_clauses: vec![(lit("sad"), col("k"))],
            else_result: Some(Box::new(col("k"))),
        }),
        None
    );
    assert_eq!(
        walk(&Expr::Case {
            operand: None,
            when_clauses: vec![(col("k"), col("m"))],
            else_result: None,
        })
        .as_deref(),
        Some("mood")
    );
}

#[test]
fn two_different_enums_unify_to_nothing() {
    assert_eq!(walk(&Expr::Coalesce(vec![col("m"), col("n")])), None);
    assert_eq!(
        walk(&Expr::Case {
            operand: None,
            when_clauses: vec![(col("k"), col("m"))],
            else_result: Some(Box::new(col("n"))),
        }),
        None
    );
}

#[test]
fn a_cast_is_the_boundary_and_stays_one() {
    assert_eq!(
        walk(&Expr::Cast {
            expr: Box::new(col("m")),
            target_type: TypeOid::Text,
        }),
        None
    );
    assert_eq!(
        walk(&Expr::CastTypmod {
            expr: Box::new(col("m")),
            type_oid: TypeOid::Varchar,
            typmod: crate::planner::Typmod::Length(8),
        }),
        None
    );
    // A cast *to* an enum converts (`StringFunc::EnumIn`) and still erases: the
    // conversion node is a string function, whose result is not its operand's type,
    // under the `text` cast it is planned as — so `m::mood` and `s::mood` answer what
    // they answered before the conversion existed (`EDGES.md` row 104).
    assert_eq!(
        walk(&crate::planner::build_enum_cast(
            col("m"),
            "mood",
            &["sad".to_string(), "ok".to_string()]
        )),
        None
    );
}

/// A scalar subquery is **not** a boundary: the arm asks the caller's subquery leaf.
/// It was a bare `None`, recorded as measured-unreachable from the one position that
/// is `0A000` for its own reason, while `SELECT`, `WHERE` and a `DELETE` predicate
/// all reached it silently.
///
/// The `walk_blind` half is the discriminator: without it an arm hard-coded to
/// `Some("mood")` would pass both assertions beside it.
#[test]
fn a_subquery_asks_the_leaf_rather_than_declining() {
    assert_eq!(walk(&subquery()).as_deref(), Some("mood"));
    assert_eq!(walk_blind(&subquery()), None);
}

/// And it unifies like any other operand: agreeing with a column keeps the enum,
/// disagreeing with one cancels, and a leaf that cannot answer is no information.
#[test]
fn a_subquery_unifies_with_its_siblings() {
    assert_eq!(
        walk(&Expr::Coalesce(vec![subquery(), col("m")])).as_deref(),
        Some("mood")
    );
    assert_eq!(walk(&Expr::Coalesce(vec![subquery(), col("n")])), None);
    assert_eq!(
        walk_blind(&Expr::Coalesce(vec![subquery(), col("m")])).as_deref(),
        Some("mood")
    );
}

#[test]
fn a_node_whose_result_is_not_its_operands_type_carries_no_enum() {
    // Boolean-valued, even over an enum operand — the operand's order still decides
    // the *comparison*, which is `operands_enum_order`'s job.
    assert_eq!(
        walk(&Expr::BinaryOp {
            left: Box::new(col("m")),
            op: BinaryOp::Gt,
            right: Box::new(lit("sad")),
        }),
        None
    );
    assert_eq!(walk(&Expr::IsNull(Box::new(col("m")))), None);
    // Text-valued.
    assert_eq!(walk(&Expr::Lower(Box::new(col("m")))), None);
    assert_eq!(
        walk(&Expr::VariadicFn {
            func: VariadicFunc::Concat,
            args: vec![col("m"), lit("!")],
        }),
        None
    );
    // Carries no declared enum at all.
    assert_eq!(walk(&lit("happy")), None);
    assert_eq!(walk(&Expr::Parameter { index: 1 }), None);
}

/// An `ARRAY[…]` answers at its **element's** width, and it unifies every element —
/// not the first.
///
/// The mixed pair is the discriminating one: a first-element rule reads `None` from
/// the cast and drops the enum the second element carries, which the single-element
/// spelling beside it cannot separate. The empty array is the arm's other end and
/// is deliberately `None` rather than a guess — nothing is compared element-wise
/// there for an order to be read at.
#[test]
fn an_array_takes_its_elements_unified_enum() {
    assert_eq!(walk(&Expr::Array(vec![col("m")])).as_deref(), Some("mood"));
    assert_eq!(
        walk(&Expr::Array(vec![
            Expr::Cast {
                expr: Box::new(lit("ok")),
                target_type: TypeOid::Text,
            },
            col("m"),
        ]))
        .as_deref(),
        Some("mood"),
        "the cast carries no identity, so the column's must survive the unification"
    );
    assert_eq!(walk(&Expr::Array(vec![col("m"), col("n")])), None);
    assert_eq!(walk(&Expr::Array(vec![lit("happy")])), None);
    assert_eq!(walk(&Expr::Array(vec![])), None);
}

/// `unnest`'s result type **is** its operand's element type, so it passes the array
/// node's answer through — the inverse of the arm above, and the pair is what keeps
/// `ARRAY[…]` and `unnest(…)` from disagreeing about the same list.
#[test]
fn unnest_passes_its_operands_element_enum_through() {
    assert_eq!(
        walk(&Expr::Unnest(Box::new(Expr::Array(vec![col("m")])))).as_deref(),
        Some("mood")
    );
    assert_eq!(
        walk(&Expr::Unnest(Box::new(Expr::Array(vec![lit("happy")])))),
        None
    );
    // It asks rather than answering: a blind subquery leaf under it reads `None`
    // where the answering one reads the enum.
    assert_eq!(
        walk(&Expr::Unnest(Box::new(Expr::Array(vec![subquery()])))).as_deref(),
        Some("mood")
    );
    assert_eq!(
        walk_blind(&Expr::Unnest(Box::new(Expr::Array(vec![subquery()])))),
        None
    );
}
