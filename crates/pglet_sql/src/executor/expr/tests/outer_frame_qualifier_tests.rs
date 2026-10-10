//! Tests for the outer-frame slot resolution of qualified and bare column names.

use super::super::columns::ColumnRef;
use super::super::env::{EvalEnv, OuterFrame, OuterSlot};
use crate::types::{TypeOid, Value};

/// `(frame identity, frame column names)` → what the chain says about a qualified
/// correlated reference `t.name`, flattened to a comparable word.
///
/// `"found(i)"` is a resolution, `"claimed"` a frame owning `t` with no such column
/// (the caller's `42703 "t.name"`), `"ambiguous"` two exact claimants at one level
/// (`42702`), `"unclaimed"` no owner anywhere (the inner resolver's own verdict,
/// `42P01` where it has an identity).
fn slot(table: Option<&str>, names: &[&str], t: &str, name: &str) -> String {
    let row: Vec<Value> = (0..names.len()).map(|i| Value::Int4(i as i32)).collect();
    let types = vec![TypeOid::Int4; names.len()];
    let frame = OuterFrame::new(names, &types, &[], table, &row, None);
    let env = EvalEnv::EMPTY.push(&frame);
    match env.resolve_outer_slot(Some(t), name) {
        OuterSlot::Found(_, i) => format!("found({i})"),
        OuterSlot::Claimed => "claimed".to_string(),
        OuterSlot::Ambiguous => "ambiguous".to_string(),
        OuterSlot::Unclaimed => "unclaimed".to_string(),
    }
}

/// [`slot`] over a **pair-addressed** frame: `refs` is `(relation, column,
/// bare-visible)` per column, so the frame resolves the way a join's namespace does.
fn pair_slot(refs: &[(Option<&str>, &str, bool)], t: Option<&str>, name: &str) -> String {
    let names: Vec<&str> = refs.iter().map(|(_, n, _)| *n).collect();
    let row: Vec<Value> = (0..refs.len()).map(|i| Value::Int4(i as i32)).collect();
    let types = vec![TypeOid::Int4; refs.len()];
    let column_refs: Vec<ColumnRef> = refs
        .iter()
        .map(|(rel, n, bare)| ColumnRef {
            relation: rel.map(str::to_string),
            name: (*n).to_string(),
            bare_visible: *bare,
        })
        .collect();
    let frame = OuterFrame::over_relations(&names, &types, &[], &column_refs, &row, None);
    let env = EvalEnv::EMPTY.push(&frame);
    match env.resolve_outer_slot(t, name) {
        OuterSlot::Found(_, i) => format!("found({i})"),
        OuterSlot::Claimed => "claimed".to_string(),
        OuterSlot::Ambiguous => "ambiguous".to_string(),
        OuterSlot::Unclaimed => "unclaimed".to_string(),
    }
}

/// A join scope's pairs: one relation carries `w`, the other does not — the shape
/// whose *bare* correlated reference 18.6 resolves and the synthesized-name frame
/// could not find, because no column of it is spelled `w`.
///
/// The two-claimant arm is here rather than in a fixture because the plan-time
/// binding gate answers that spelling before the executor is reached
/// (`select/correlated_outer_ref_join_bare_ambiguous.sql` says so), so this is the
/// only reader of the frame's own count.
#[test]
fn a_pair_frame_resolves_a_bare_name_one_relation_claims() {
    let refs = [
        (Some("a"), "id", true),
        (Some("a"), "w", true),
        (Some("b"), "id", true),
        (Some("b"), "y", true),
    ];
    assert_eq!(pair_slot(&refs, None, "w"), "found(1)");
    // Two relations carrying it is `42702` at this level, never a first match.
    let both = [(Some("a"), "w", true), (Some("b"), "w", true)];
    assert_eq!(pair_slot(&both, None, "w"), "ambiguous");
    // A qualified reference is the exact pair; the qualifier's own relation owning
    // no such column stops the walk (`42703`), and an unnamed relation does not.
    assert_eq!(pair_slot(&refs, Some("b"), "y"), "found(3)");
    assert_eq!(pair_slot(&refs, Some("b"), "w"), "claimed");
    assert_eq!(pair_slot(&refs, Some("zzz"), "w"), "unclaimed");
}

/// A `USING` merge belongs to the join, so no qualifier names it and it is the only
/// bare claimant — the sides' own copies stay qualifiable and hidden from the count.
/// Dropping either half is a spurious `42702` on ordinary SQL.
#[test]
fn a_pair_frame_counts_only_the_bare_visible_using_merge() {
    let refs = [
        (None, "c", true),
        (Some("a"), "c", false),
        (Some("a"), "x", true),
        (Some("b"), "c", false),
        (Some("b"), "y", true),
    ];
    assert_eq!(pair_slot(&refs, None, "c"), "found(0)");
    assert_eq!(pair_slot(&refs, Some("a"), "c"), "found(1)");
    // The merge itself is nobody's column: a qualifier cannot reach it.
    assert_eq!(pair_slot(&refs, Some("joined"), "c"), "unclaimed");
}

/// A column a user quoted a dot into is an ordinary column name here — it neither
/// manufactures a relation nor claims the bare name its suffix spells. Both
/// directions were silent wrongs under the string addressing.
#[test]
fn a_pair_frame_does_not_read_a_dotted_column_name_as_a_pair() {
    let refs = [(Some("s"), "zz.v", true), (Some("s"), "id", true)];
    assert_eq!(pair_slot(&refs, Some("zz"), "v"), "unclaimed");
    assert_eq!(pair_slot(&refs, Some("s"), "zz.v"), "found(0)");
    assert_eq!(pair_slot(&refs, None, "v"), "unclaimed");
    assert_eq!(pair_slot(&refs, None, "zz.v"), "found(0)");
}

/// The bare-name spelling of [`slot`].
fn bare_slot(table: Option<&str>, names: &[&str], name: &str) -> String {
    let row: Vec<Value> = (0..names.len()).map(|i| Value::Int4(i as i32)).collect();
    let types = vec![TypeOid::Int4; names.len()];
    let frame = OuterFrame::new(names, &types, &[], table, &row, None);
    let env = EvalEnv::EMPTY.push(&frame);
    match env.resolve_outer_slot(None, name) {
        OuterSlot::Found(_, i) => format!("found({i})"),
        OuterSlot::Claimed => "claimed".to_string(),
        OuterSlot::Ambiguous => "ambiguous".to_string(),
        OuterSlot::Unclaimed => "unclaimed".to_string(),
    }
}

/// **The discriminating assertion — keep it first**, so a fixer sees this
/// message rather than a later pin's.
///
/// A frame that knows its relation must not answer for a different one. Its
/// names come straight from that relation, so a dotted entry there is a user's
/// quoted `"a.b"` column and *not* the multi-relation addressing — and the two
/// are byte-identical. Live: over `q1(id, "a.b")` and `q2(yv)`,
/// `DELETE FROM q1 WHERE EXISTS (SELECT 1 FROM q2 WHERE q2.yv = a.b)` deleted a
/// row here where 18.6 raises `42P01` and both rows survive.
///
/// `unclaimed` and not `claimed`: `q1` is not the relation `a`, so the reference
/// leaves the chain entirely and the inner resolver reports the missing FROM entry.
#[test]
fn a_frame_that_knows_its_relation_does_not_answer_a_foreign_qualifier() {
    assert_eq!(slot(Some("q1"), &["id", "a.b"], "a", "b"), "unclaimed");
    // …nor when the dotted entry is the only column, so nothing else could have
    // answered instead.
    assert_eq!(slot(Some("q1"), &["a.b"], "a", "b"), "unclaimed");
}

/// The relation between the two shapes, which is what separates the correct fix
/// from every wrong one: **the same reference against the same name list
/// resolves iff the frame has no relation identity.** A fix keyed to a statement
/// kind, or one spelled `f.table != Some(t)`, breaks this pair.
#[test]
fn the_dotted_tier_belongs_to_the_frame_without_an_identity() {
    let names = ["ta.id", "ta.v", "tb.w"];
    assert_eq!(
        slot(None, &names, "ta", "v"),
        "found(1)",
        "join frame addresses"
    );
    assert_eq!(
        slot(Some("s"), &names, "ta", "v"),
        "unclaimed",
        "derived table does not"
    );
}

/// The accept polarity: a frame *is* the qualifier, so its bare column answers
/// (`FROM ta a2 … WHERE tc.k = a2.v`) — and a column that relation lacks is
/// **`claimed`**, the owner stopping the walk, not a licence to look elsewhere.
///
/// That second assertion is the one the old two-state slot could not spell: it
/// answered `None` there, indistinguishable from "no frame owns this", and the
/// caller then rebound the reference to whatever bare column the chain happened to
/// carry. Its SQL is `select/correlated_outer_ref_owner_lacks_column.sql`.
#[test]
fn a_frame_that_is_the_qualifier_answers_its_bare_column() {
    assert_eq!(slot(Some("a2"), &["id", "v"], "a2", "v"), "found(1)");
    assert_eq!(slot(Some("a2"), &["id", "v"], "a2", "nosuch"), "claimed");
}

/// The identity tier's own discriminator, and the shape a single frame cannot
/// express: **two** frames both carrying a bare `v`, the qualifier naming the
/// *outer* one. Only a tier that tests the frame's relation identity answers the
/// outer value; a qualifier-blind fallback takes the innermost frame's, so this
/// pair separates the two where the accept test above cannot.
/// Its SQL is `select/correlated_outer_ref_shadowed_across_levels.sql`.
#[test]
fn the_identity_tier_binds_to_the_named_frame_not_the_innermost() {
    let outer_row = [Value::Int4(1), Value::Int4(20)];
    let types = [TypeOid::Int4, TypeOid::Int4];
    let outer = OuterFrame::new(&["id", "v"], &types, &[], Some("a4"), &outer_row, None);
    let inner_row = [Value::Int4(1), Value::Int4(30)];
    let inner = OuterFrame::new(
        &["id", "v"],
        &types,
        &[],
        Some("b4"),
        &inner_row,
        Some(&outer),
    );
    let env = EvalEnv::EMPTY.push(&inner);
    let outer_v = match env.resolve_outer_slot(Some("a4"), "v") {
        OuterSlot::Found(f, i) => f.value_at(i),
        other => panic!("a4.v must resolve, got {other:?}"),
    };
    assert_eq!(
        outer_v,
        Value::Int4(20),
        "a4.v is the outer frame's column, not the innermost frame's"
    );
    // …and the chain is walked at all: the inner frame answers its own.
    let inner_v = match env.resolve_outer_slot(Some("b4"), "v") {
        OuterSlot::Found(f, i) => f.value_at(i),
        other => panic!("b4.v must resolve, got {other:?}"),
    };
    assert_eq!(inner_v, Value::Int4(30));
}

/// **The stop-at-owner rule across levels**, which is the half no single-frame
/// assertion reaches: the owner is `a4`, the column lives on the *inner* `b4`, and
/// the reference must stop at `a4` rather than take it. This one destroyed rows —
/// `DELETE FROM a WHERE EXISTS (SELECT 1 FROM b WHERE EXISTS (… k.k = a.v))`
/// removed every row of `a` where 18.6 answers `42703 column a.v does not exist`.
#[test]
fn an_owning_frame_stops_the_walk_even_when_an_inner_frame_has_the_name() {
    let outer_row = [Value::Int4(1)];
    let types = [TypeOid::Int4];
    let outer = OuterFrame::new(&["id"], &types, &[], Some("a4"), &outer_row, None);
    let inner_row = [Value::Int4(7)];
    let inner = OuterFrame::new(&["v"], &types, &[], Some("b4"), &inner_row, Some(&outer));
    let env = EvalEnv::EMPTY.push(&inner);
    assert!(matches!(
        env.resolve_outer_slot(Some("a4"), "v"),
        OuterSlot::Claimed
    ));
    // The bare spelling of the same chain is *not* ambiguity — different levels
    // shadow, so the innermost carrier answers (18.6 agrees).
    assert!(matches!(
        env.resolve_outer_slot(None, "v"),
        OuterSlot::Found(_, 0)
    ));
}

/// Claimant counting is **exact**, at one level, in both spellings.
///
/// The accept polarity is load-bearing and is what rejects a suffix match here: a
/// `USING`-merged `id` is one exact claimant sitting ahead of the raw `a.id` /
/// `b.id`, so `FROM a JOIN b USING (id) … WHERE k.k = id` keeps resolving. Count
/// suffixes too and that becomes three claimants and a spurious `42702`.
#[test]
fn two_exact_claimants_at_one_level_are_ambiguous_and_suffixes_are_not_claimants() {
    // A derived table projecting two columns under one alias (`SELECT a AS x,
    // b AS x`), in the qualified and the bare spelling — 18.6 answers `42702` to
    // both.
    assert_eq!(slot(Some("d"), &["x", "x"], "d", "x"), "ambiguous");
    assert_eq!(bare_slot(Some("d"), &["x", "x"], "x"), "ambiguous");
    // A `USING`-merged join frame: one bare `id` ahead of the per-side columns.
    assert_eq!(
        bare_slot(None, &["id", "a.id", "a.x", "b.id", "b.y"], "id"),
        "found(0)"
    );
    // An unmerged join frame addressed by *names* carries no bare `w` at all — no
    // claimant, not a suffix one. 18.6 resolves this, and what closed the
    // over-rejection is the frame's pairs rather than a looser match here: the same
    // scope spelled as pairs answers, per
    // `a_pair_frame_resolves_a_bare_name_one_relation_claims`. This arm is what a
    // scope whose pairs could not be derived still gets.
    assert_eq!(bare_slot(None, &["a.w", "b.y"], "w"), "unclaimed");
}
