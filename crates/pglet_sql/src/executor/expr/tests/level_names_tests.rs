//! A scope over a node's output names, told its level's FROM names
//! ([`SimpleColumns::within_level`]), against an enclosing frame that carries the same
//! relation name — the shape `SELECT y, (SELECT max(o.n) FROM ev o) FROM t o` reaches at
//! the aggregate's argument. Its SQL is the `select/inner_alias_shadows_outer_*` family.

use super::super::columns::{ColumnRef, ColumnResolver, LevelScope, SimpleColumns};
use super::super::env::{EvalEnv, OuterFrame, SubqueryExec};
use super::super::eval::{column_declared_type, eval_expr_full};
use crate::Result;
use crate::planner::Expr;
use crate::types::{TypeOid, Value};

fn qualified(t: &str, n: &str) -> Expr {
    Expr::Column {
        table: Some(t.to_string()),
        name: n.to_string(),
    }
}

/// Evaluate `expr` over the inner row `[7]` named `n`, told `level`, under an enclosing
/// frame `o (y, n)` holding `(1, 10)` and, above it, `p (n)` holding `20`.
fn level<'a>(names: &[&'a str]) -> LevelScope<'a> {
    LevelScope {
        names: names.to_vec(),
        pairs: None,
    }
}

fn eval_under_frames(expr: &Expr, level_names: &[&str]) -> Result<Value> {
    let types = [TypeOid::Int4, TypeOid::Int4];
    let p_row = [Value::Int4(20)];
    let p = OuterFrame::new(&["n"], &types[..1], &[], Some("p"), &p_row, None);
    let o_row = [Value::Int4(1), Value::Int4(10)];
    let o = OuterFrame::new(&["y", "n"], &types, &[], Some("o"), &o_row, Some(&p));
    let env = EvalEnv::EMPTY.push(&o);
    let names = ["n"];
    let level = level(level_names);
    let resolver = SimpleColumns::new(&names)
        .with_types(&types[..1])
        .within_level(&level);
    eval_expr_full(expr, &resolver, &[Value::Int4(7)], SubqueryExec::NONE, env)
}

#[test]
fn a_qualifier_the_level_owns_binds_the_level_before_an_enclosing_frame() {
    assert_eq!(
        eval_under_frames(&qualified("o", "n"), &["o"]).unwrap(),
        Value::Int4(7)
    );
    // The level's names are a set: the owner need not be the only item.
    assert_eq!(
        eval_under_frames(&qualified("o", "n"), &["x", "o"]).unwrap(),
        Value::Int4(7)
    );
}

/// The column the level's item lacks is that item's `42703`, never the enclosing
/// frame's value — which `o.y` would otherwise be (`1`).
#[test]
fn a_column_the_owning_item_lacks_is_its_own_42703() {
    let err = eval_under_frames(&qualified("o", "y"), &["o"]).unwrap_err();
    assert_eq!(err.sqlstate(), "42703");
    assert!(err.to_string().contains("o.y"), "{err}");
}

/// The other polarity: a qualifier the level does not own is a correlated reference,
/// and reads the enclosing frame that owns it — the nearest (`o`) and one further
/// out (`p`).
#[test]
fn a_qualifier_only_an_enclosing_level_owns_reads_that_frame() {
    assert_eq!(
        eval_under_frames(&qualified("o", "n"), &["ev"]).unwrap(),
        Value::Int4(10)
    );
    assert_eq!(
        eval_under_frames(&qualified("p", "n"), &["o"]).unwrap(),
        Value::Int4(20)
    );
    assert_eq!(
        eval_under_frames(&qualified("o", "y"), &["ev"]).unwrap(),
        Value::Int4(1)
    );
}

/// A qualifier the level owns binds a column of its own relation, decided by the
/// level's structure: over a join's rows by the `(relation, column)` pair — never the
/// sibling's `"w.k"` a suffix match finds — and otherwise by the bare name exactly,
/// whatever dots a user quoted into a column's name.
#[test]
fn an_owned_qualifier_binds_by_structure_never_by_name_text() {
    // A join's rows: the pairs decide.
    let names = ["o.n", "w.k"];
    let join = LevelScope {
        names: vec!["o", "w"],
        pairs: Some(vec![ColumnRef::of("o", "n"), ColumnRef::of("w", "k")]),
    };
    let resolver = SimpleColumns::new(&names).within_level(&join);
    assert_eq!(
        resolver.resolve(Some("o"), "k").unwrap_err().sqlstate(),
        "42703"
    );
    assert_eq!(resolver.resolve(Some("o"), "n").unwrap(), 0);
    assert_eq!(resolver.resolve(Some("w"), "k").unwrap(), 1);
    // One relation whose user columns are `"o.z"` and `n`: the bare names decide, and
    // a dot in a column's name is not a relation.
    let user = ["o.z", "n"];
    let single = level(&["o"]);
    let resolver = SimpleColumns::new(&user).within_level(&single);
    assert_eq!(resolver.resolve(Some("o"), "n").unwrap(), 1);
    assert_eq!(resolver.resolve(Some("o"), "o.z").unwrap(), 0);
    assert_eq!(
        resolver.resolve(Some("o"), "z").unwrap_err().sqlstate(),
        "42703"
    );
    // A user column named `"o.n"` beside `n` is not `o.n`.
    let forged = ["o.n", "n"];
    let resolver = SimpleColumns::new(&forged).within_level(&single);
    assert_eq!(resolver.resolve(Some("o"), "n").unwrap(), 1);
    // A projection's output: the bare name, exactly — never a suffix.
    let outputs = ["x.k"];
    let resolver = SimpleColumns::new(&outputs).within_level(&single);
    assert_eq!(
        resolver.resolve(Some("o"), "k").unwrap_err().sqlstate(),
        "42703"
    );
}

/// The declared type takes the same route as the value (`column_declared_type`
/// branches on the same `scope_has_table`): the inner column's `int4`, not the
/// enclosing frame's, whose `n` is declared `text` here.
#[test]
fn the_declared_type_follows_the_value_to_the_level() {
    let types = [TypeOid::Int4, TypeOid::Text];
    let o_row = [Value::Int4(1), Value::Text("outer".into())];
    let o = OuterFrame::new(&["y", "n"], &types, &[], Some("o"), &o_row, None);
    let env = EvalEnv::EMPTY.push(&o);
    let names = ["n"];
    let inner_types = [TypeOid::Int4];
    let level = level(&["o"]);
    let resolver = SimpleColumns::new(&names)
        .with_types(&inner_types)
        .within_level(&level);
    let declared = column_declared_type(&resolver, env, Some("o"), "n");
    assert_eq!(declared, Some(TypeOid::Int4));
}
