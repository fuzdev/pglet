//! Tests for enum member ordering in comparisons.

use super::super::compare::{values_compare, values_compare_ordered, values_equal};
use super::super::enum_order::operand_can_be_enum;
use crate::types::Value;
use std::cmp::Ordering;

/// `mood`'s declared order is the exact reverse of its lexical one, so every
/// assertion below separates the two.
fn mood() -> Vec<String> {
    ["sad", "ok", "happy"].map(String::from).to_vec()
}

fn txt(s: &str) -> Value {
    Value::Text(s.to_string())
}

#[test]
fn members_order_by_declared_position() {
    let m = mood();
    let o = Some(m.as_slice());
    assert_eq!(
        values_compare_ordered(&txt("sad"), &txt("ok"), o),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare_ordered(&txt("ok"), &txt("happy"), o),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare_ordered(&txt("happy"), &txt("sad"), o),
        Some(Ordering::Greater)
    );
    // ...and the same pair without an order is the label bytes, unchanged.
    assert_eq!(
        values_compare(&txt("sad"), &txt("ok")),
        Some(Ordering::Greater)
    );
}

/// The invariant the *equality*-asking callers depend on (window peers, aggregate
/// `DISTINCT`, `ON CONFLICT`'s scan fallback): they ask `values_compare` and read
/// `== Some(Equal)`, so a position-aware comparator that ever answered `Equal` for
/// two different labels — or `Less`/`Greater` for one label against itself — would
/// silently merge or split their groups. 18.6's `enum_eq` is bare OID identity and
/// one member is one label is one OID, which is why label-byte equality is the
/// right answer and this pins that the fix did not move it.
#[test]
fn equal_iff_the_labels_are_byte_equal() {
    let m = mood();
    let o = Some(m.as_slice());
    let labels = ["sad", "ok", "happy", "bogus", ""];
    for a in labels {
        for b in labels {
            let eq = values_compare_ordered(&txt(a), &txt(b), o) == Some(Ordering::Equal);
            assert_eq!(eq, a == b, "Equal must mean byte-equal, for {a:?} vs {b:?}");
            // ...and it agrees with the equality operator's own answer.
            assert_eq!(values_equal(&txt(a), &txt(b)), Some(a == b));
        }
    }
}

/// A label the type does not hold still compares, and the result is a **total
/// order** — `sort_by`'s requirement, which "members by position, the rest by bytes,
/// interleaved" does not satisfy. Every member sorts below every non-member;
/// non-members sort among themselves by their bytes.
#[test]
fn a_non_member_label_sorts_after_every_member() {
    let m = mood();
    let o = Some(m.as_slice());
    for member in ["sad", "ok", "happy"] {
        assert_eq!(
            values_compare_ordered(&txt(member), &txt("zzz"), o),
            Some(Ordering::Less),
            "{member} must sort below a non-member"
        );
    }
    assert_eq!(
        values_compare_ordered(&txt("aaa"), &txt("zzz"), o),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare_ordered(&txt("zzz"), &txt("aaa"), o),
        Some(Ordering::Greater)
    );
}

/// Transitivity and antisymmetry over a mixed member/non-member set, checked by
/// exhaustion — a comparator that is not a total order makes `sort_by`'s output
/// depend on the input permutation, which no differential fixture can see.
#[test]
fn the_comparator_is_a_total_order() {
    let m = mood();
    let o = Some(m.as_slice());
    let labels = ["sad", "ok", "happy", "aaa", "zzz", "ok2"];
    let cmp = |a: &str, b: &str| values_compare_ordered(&txt(a), &txt(b), o).expect("non-null");
    for a in labels {
        assert_eq!(cmp(a, a), Ordering::Equal);
        for b in labels {
            assert_eq!(cmp(a, b), cmp(b, a).reverse(), "antisymmetry: {a} vs {b}");
            for c in labels {
                if cmp(a, b) != Ordering::Greater && cmp(b, c) != Ordering::Greater {
                    assert_ne!(cmp(a, c), Ordering::Greater, "transitivity: {a} {b} {c}");
                }
            }
        }
    }
}

/// The same exhaustion over a member list that carries a **duplicate label** — the
/// state `CREATE TYPE … AS ENUM ('b', 'a', 'b')` used to reach and no longer does.
///
/// Read this as a *legacy-state* pin and not as a regression guard for that gate.
/// [`enum_rank`] takes `position()` — the first match — so the second `b` is never
/// ranked and distinct labels still take distinct ranks, and the arm this exercises
/// is untouched by the gate. That is the point, and it was measured from SQL on the
/// pre-gate engine rather than reasoned: over a column of
/// `CREATE TYPE e AS ENUM ('b', 'a', 'b')`, `ORDER BY`, `min`, `max`,
/// `greatest` and `least` all answered by the member positions and agreed with each
/// other. What the duplicate cost was the catalog (`pg_enum` answered two rows for
/// one label) and the dump (that statement is what `pglet::dump` emitted and stock
/// 18.6 refuses), never the order. The gate closes the *producers*, not the state
/// — a store written by an earlier build still holds such a type, reopens, and is
/// read through this arm.
#[test]
fn a_duplicated_label_keeps_the_comparator_a_total_order() {
    let dup = ["b", "a", "b"].map(String::from).to_vec();
    let o = Some(dup.as_slice());
    let labels = ["a", "b", "aaa", "zzz"];
    let cmp = |a: &str, b: &str| values_compare_ordered(&txt(a), &txt(b), o).expect("non-null");
    // The declared positions, with the repeat's first occurrence deciding `b`.
    assert_eq!(cmp("b", "a"), Ordering::Less);
    for a in labels {
        assert_eq!(cmp(a, a), Ordering::Equal);
        for b in labels {
            assert_eq!(cmp(a, b), cmp(b, a).reverse(), "antisymmetry: {a} vs {b}");
            // `Equal` still means byte-equal: the repeat must not make two distinct
            // labels tie, which is the one way a `position()` rank could fail.
            assert_eq!(
                cmp(a, b) == Ordering::Equal,
                a == b,
                "Equal iff byte-equal: {a} vs {b}"
            );
            for c in labels {
                if cmp(a, b) != Ordering::Greater && cmp(b, c) != Ordering::Greater {
                    assert_ne!(cmp(a, c), Ordering::Greater, "transitivity: {a} {b} {c}");
                }
            }
        }
    }
}

/// The order is the **type's**, not the labels': two enums over the same labels in
/// opposite orders answer opposite verdicts for the same value pair.
#[test]
fn two_enums_over_the_same_labels_disagree() {
    let ea = ["x", "y"].map(String::from).to_vec();
    let eb = ["y", "x"].map(String::from).to_vec();
    assert_eq!(
        values_compare_ordered(&txt("x"), &txt("y"), Some(&ea)),
        Some(Ordering::Less)
    );
    assert_eq!(
        values_compare_ordered(&txt("x"), &txt("y"), Some(&eb)),
        Some(Ordering::Greater)
    );
}

/// A NULL operand still propagates, and the enum arm must not claim a pair that is
/// not two `Text` values.
#[test]
fn null_and_non_text_operands_are_unclaimed() {
    let m = mood();
    let o = Some(m.as_slice());
    assert_eq!(values_compare_ordered(&Value::Null, &txt("ok"), o), None);
    assert_eq!(values_compare_ordered(&txt("ok"), &Value::Null, o), None);
    // An int against a text falls through to the mixed-pair fallback, exactly as it
    // does with no order attached.
    assert_eq!(
        values_compare_ordered(&Value::Int4(1), &txt("ok"), o),
        values_compare(&Value::Int4(1), &txt("ok"))
    );
}

/// The gate the ordered-comparison sites ask before walking for a member order is
/// exactly the arm that reads one: for a pair `operand_can_be_enum` rejects,
/// [`values_compare_ordered`] answers the same with an order and without, and for the
/// pair it accepts it does not.
///
/// This pins the **rule** over the pairs it lists, not over the whole `Value`
/// cross-product, and not its wiring: that each call site asks it is unobservable —
/// skipping the walk changes no answer, which is the point — so a caller that dropped
/// the gate would cost time and redden nothing here.
#[test]
fn the_walk_is_gated_on_exactly_the_pairs_whose_verdict_the_order_moves() {
    let m = mood();
    let o = Some(m.as_slice());
    for (l, r) in [
        (Value::Int4(1), Value::Int4(2)),
        (txt("happy"), Value::Int4(2)),
        (Value::Int4(1), txt("ok")),
        (Value::Null, txt("ok")),
        (txt("happy"), Value::Null),
    ] {
        assert!(
            !(operand_can_be_enum(&l) && operand_can_be_enum(&r)),
            "the gate must reject {l:?} vs {r:?}"
        );
        assert_eq!(
            values_compare_ordered(&l, &r, o),
            values_compare_ordered(&l, &r, None),
            "a pair the gate rejects must not read the member order: {l:?} vs {r:?}"
        );
    }
    let (l, r) = (txt("happy"), txt("ok"));
    assert!(operand_can_be_enum(&l) && operand_can_be_enum(&r));
    assert_ne!(
        values_compare_ordered(&l, &r, o),
        values_compare_ordered(&l, &r, None),
        "the accepted pair is the one whose verdict the order moves"
    );
}

/// Two arrays compare **element-wise against the same member list**, which is what
/// makes `expr_enum_with`'s `Array` arm readable at all: the arm answers at the
/// element's width and this is the reading that consumes it.
///
/// The `values_compare_ordered(_, _, None)` half beside each is the discriminator —
/// without it an element loop that dropped the order would pass every assertion, the
/// label bytes agreeing with the member positions on some pairs by luck.
#[test]
fn arrays_compare_element_wise_against_the_same_member_order() {
    let m = mood();
    let o = Some(m.as_slice());
    let arr = |labels: &[&str]| {
        Value::Array(crate::types::ArrayValue {
            element_type: crate::types::TypeOid::Text,
            elements: labels.iter().map(|l| txt(l)).collect(),
        })
    };
    let (l, r) = (arr(&["happy"]), arr(&["ok"]));
    assert_eq!(values_compare_ordered(&l, &r, o), Some(Ordering::Greater));
    assert_eq!(
        values_compare_ordered(&l, &r, None),
        Some(Ordering::Less),
        "the label bytes give the opposite verdict, which is what separates the two"
    );
    // The order is read at **every** element, not only the first: element 0 is equal
    // here, so element 1 decides.
    let (l2, r2) = (arr(&["ok", "happy"]), arr(&["ok", "sad"]));
    assert_eq!(values_compare_ordered(&l2, &r2, o), Some(Ordering::Greater));
    assert_eq!(values_compare_ordered(&l2, &r2, None), Some(Ordering::Less));
}

/// The gate admits an array operand exactly when the element loop could reach the
/// enum arm — one level down, and by the same `Value::Text` test.
///
/// The rejected shapes are the ones the loop decides without reading an order at
/// all: an all-NULL array (the NULL rule fires first) and a length difference with
/// no `Text` on one side. Each is paired with the with-order/without-order
/// equivalence the gate's contract requires — *a pair the gate rejects must not read
/// the member order.*
#[test]
fn the_gate_admits_an_array_whose_elements_could_be_enum_labels() {
    let m = mood();
    let o = Some(m.as_slice());
    let arr = |vals: Vec<Value>| {
        Value::Array(crate::types::ArrayValue {
            element_type: crate::types::TypeOid::Text,
            elements: vals,
        })
    };
    assert!(operand_can_be_enum(&arr(vec![txt("happy")])));
    assert!(
        operand_can_be_enum(&arr(vec![Value::Null, txt("happy")])),
        "one readable element is enough — the loop reaches the arm at that position"
    );
    for v in [
        arr(vec![]),
        arr(vec![Value::Null]),
        arr(vec![Value::Int4(1)]),
    ] {
        assert!(!operand_can_be_enum(&v), "the gate must reject {v:?}");
        let other = arr(vec![txt("ok")]);
        assert_eq!(
            values_compare_ordered(&v, &other, o),
            values_compare_ordered(&v, &other, None),
            "a pair the gate rejects must not read the member order: {v:?}"
        );
    }
}
