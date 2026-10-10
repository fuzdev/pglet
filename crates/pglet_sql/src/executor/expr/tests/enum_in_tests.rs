//! Tests for the enum-conversion arms of `IN` evaluation.

use super::super::eval::eval_expr;
use super::super::string_fn::eval_string_fn;
use crate::catalog::{EnumType, Schema};
use crate::parser::parse_expr;
use crate::planner::{Expr, Literal, StringFunc};
use crate::types::{TypeOid, Value};

/// A catalog holding `mood AS ENUM ('sad', 'ok')`.
fn schema() -> Schema {
    let mut schema = Schema::new();
    schema.enums.insert(
        "mood".to_string(),
        EnumType {
            oid: 90_000,
            labels: vec!["sad".to_string(), "ok".to_string()],
        },
    );
    schema
}

/// `sql` parsed against [`schema`] and evaluated over one `text` column `x`.
fn eval_over(sql: &str, x: Value) -> crate::Result<Value> {
    let expr = parse_expr(sql, &schema()).expect("a cast to an enum parses");
    eval_expr(&expr, &["x"], &[TypeOid::Text], &[], &[x])
}

/// A member literal keeps the plain `text` cast — the node a member literal always
/// was, so nothing reading it changes — and a literal the enum lacks, which differs
/// only in case, builds the conversion without refusing at parse time: the refusal is
/// the constant-cast pass's, at 18.6's statement position.
#[test]
fn only_an_undecided_operand_builds_the_conversion() {
    let member = parse_expr("'ok'::mood", &schema()).expect("parses");
    assert_eq!(
        member,
        Expr::Cast {
            expr: Box::new(Expr::Literal(Literal::String("ok".into()))),
            target_type: TypeOid::Text,
        }
    );
    for sql in [
        "'OK'::mood",
        "CAST('q' AS mood)",
        "mood 'q'",
        "x::mood",
        "'ok'::text::mood",
    ] {
        let Expr::Cast { expr, target_type } = parse_expr(sql, &schema()).expect(sql) else {
            panic!("{sql}: not a cast");
        };
        assert_eq!(target_type, TypeOid::Text, "{sql}");
        assert!(
            crate::planner::enum_in_parts(&expr).is_some(),
            "{sql}: {expr:?}"
        );
    }
    let null = parse_expr("NULL::mood", &schema()).expect("parses");
    assert!(matches!(null, Expr::Cast { ref expr, .. } if **expr == Expr::Literal(Literal::Null)));
    assert_eq!(
        crate::planner::validate_const_casts(&parse_expr("'OK'::mood", &schema()).expect("parses"))
            .expect_err("a non-member literal")
            .sqlstate(),
        "22P02"
    );
}

/// The conversion on a value: a member passes as its label, a non-member is `22P02`
/// naming the enum and the label, NULL is NULL, and a non-string value is 18.6's
/// `42846` — every one decided on the value, so a row that never arrives decides
/// nothing.
#[test]
fn the_conversion_decides_on_the_value() {
    let text = |s: &str| Value::Text(s.to_string());
    assert_eq!(
        eval_over("x::mood", text("ok")).expect("a member"),
        text("ok")
    );
    let refused = eval_over("x::mood", text("Ok")).expect_err("case is not folded");
    assert_eq!(refused.sqlstate(), "22P02");
    assert_eq!(
        refused.to_string(),
        "invalid input value for enum mood: \"Ok\""
    );
    assert_eq!(
        eval_over("x::mood", Value::Null).expect("NULL"),
        Value::Null
    );
    let e = eval_over("5::mood", Value::Null).expect_err("an integer source");
    assert_eq!(e.sqlstate(), "42846", "{e}");
    assert!(e.to_string().ends_with(" to mood"), "{e}");
    // The literal a gate missed is still refused where it is evaluated.
    assert_eq!(
        eval_over("'q'::mood", Value::Null)
            .expect_err("backstop")
            .sqlstate(),
        "22P02"
    );
}

/// The function-table arm over evaluated arguments, which the node's own arm in
/// `eval_expr_full` answers ahead of for every argument list the parser builds — so
/// it is reached only here, and pinned to the same rule.
#[test]
fn the_function_table_arm_answers_the_same_rule() {
    let text = |s: &str| Value::Text(s.to_string());
    let args = |v: Value| vec![v, text("mood"), text("sad"), text("ok")];
    assert_eq!(
        eval_string_fn(StringFunc::EnumIn, &args(text("sad")), None).expect("member"),
        text("sad")
    );
    assert_eq!(
        eval_string_fn(StringFunc::EnumIn, &args(text("q")), None)
            .expect_err("non-member")
            .sqlstate(),
        "22P02"
    );
    assert_eq!(
        eval_string_fn(StringFunc::EnumIn, &args(Value::Int4(1)), None)
            .expect_err("a non-string value")
            .sqlstate(),
        "42846"
    );
}
