//! Correlated-subquery resolution: re-resolving a nested query level's expressions
//! against the **enclosing** query's relations.
//!
//! # Why a second pass exists at all
//!
//! pglet's parser and planner are fused: `EXISTS (SELECT …)` is parsed *and fully
//! lowered* — its coercion pass run, its errors already propagated — from inside the
//! enclosing query's clause parsing, long before the enclosing query has a
//! [`ColumnScope`] of its own (`lower_select` builds it after the whole statement is
//! parsed). At the moment the inner scope is consumed, the outer scope does not exist.
//! Threading a parent pointer through the existing call order is therefore not a
//! matter of plumbing — there is nothing to point at.
//!
//! So the enclosing level fixes it up afterwards. Once `lower_select` has built its
//! scope, [`resolve_correlated_subplans`] walks the assembled plan, finds every
//! sub-plan embedded in an expression (`Expr::Subquery` / `Expr::Exists` /
//! `Expr::InSubquery` — leaves to every other walker), and **re-runs the coercion and
//! gate passes** over that sub-plan's expressions against a scope chained with the
//! enclosing levels ([`ColumnScope::nested_in`]). Descent recurses, so a two-level
//! nest is resolved with a three-level chain.
//!
//! Without this, a bare literal compared against an enclosing column
//! (`… WHERE EXISTS (SELECT 1 FROM b WHERE a.uid = '<uuid>')`) never got its coercing
//! cast, and the *eval-time* operand gate then saw `uuid = text` and raised `42883` on
//! a query Postgres runs.
//!
//! # Why re-running is safe rather than merely convenient
//!
//! The pass is **idempotent under a widening scope**. Every coercion rule is keyed on
//! an operand it can type: a comparison casts a string literal only beside a *typed*
//! column, and the `CASE`/`COALESCE`/`greatest` unifications **decline outright** when
//! any branch is a column the scope cannot type (`is_untyped_column`). So the first
//! run leaves a correlated reference's neighbourhood untouched rather than coercing it
//! against a wrong sibling, and the second run — with the reference now typed —
//! reaches the same decision Postgres does. Re-running with the *identical* scope
//! changes nothing, which is what lets the top level skip the rewrite entirely.
//!
//! # No expression is deferred into silence
//!
//! The rewrite is not a marker that something might revisit; it is
//! `resolve_expr_types_gated` run again, in full, including
//! `validate_const_casts` and the static comparison gate. A reference that resolves at
//! **no** level is left exactly as the first run left it — uncoerced — and the
//! eval-time operand gate raises on it as it always has. Widening the scope can only
//! turn a loud-at-eval error into a loud-at-plan one, or into the row Postgres
//! returns; it can never turn either into silence. The one direction that *could*
//! introduce silence — resolving a name against an enclosing relation when it really
//! belonged to an un-enumerable relation at the inner level — is closed by
//! registering such a relation **non-exhaustively**: a level holding a relation this
//! module cannot fully enumerate stops the walk instead of letting the name escape
//! outward.
//!
//! @module

use std::collections::HashMap;

use crate::Result;
use crate::catalog::Schema;

use super::coercion::{
    ColumnScope, OutputEntry, build_column_type_map, check_boolean_clause, resolve_expr_types_gated,
};
use super::dml::{
    insert_returning_relations, lowers_clauses, on_conflict_action_scope, returning_scope,
};
use super::expr::{BoolContext, Expr};
use super::plan::{
    CtePlan, EnclosingColumns, InsertRows, OnConflictAction, Plan, scan_effective_name,
};
use super::query::{
    OuterTyping, SetOpColumns, SideScopes, collect_referenced_columns,
    key_may_name_an_output_column, unify_set_op_columns,
};
use crate::types::ColumnDesc;

/// Re-resolve every **nested** query level inside `plan` against the enclosing levels.
///
/// Called by `lower_select`, `lower_update`, and `lower_delete` once each has built
/// its own scope. `plan`'s own expressions are already resolved against exactly this
/// level's scope, so they are left alone; only the sub-plans embedded in them (and the
/// levels below those) are re-run.
///
/// `lower_insert` calls it too, and an `INSERT` is the one entry point whose clauses do
/// **not** share a namespace: `RETURNING` names the target, the `ON CONFLICT` action
/// names the target and `EXCLUDED`, and neither may reach an enclosing level. So
/// [`walk_level`]'s `Plan::Insert` arm builds a scope per clause rather than reading the
/// one `here` every other arm shares. Its `VALUES` cells and its `INSERT … SELECT` source
/// are deliberately not visited: a `VALUES` cell has no namespace in Postgres, and the
/// source is lowered as a statement in its own right and re-walked by its own
/// `lower_select`.
///
/// # Errors
///
/// A constant string cast the widened scope newly created and that cannot be coerced
/// (`22P02` / `22007` / …), or a comparison the widened scope newly typed and for
/// which Postgres has no operator (`42883`) — the same errors
/// [`resolve_expr_types_gated`] raises at any other clause position.
pub fn resolve_correlated_subplans(
    plan: &mut Plan,
    schema: &Schema,
    chain_is_complete: bool,
) -> Result<()> {
    // Nearly every statement has no sub-plan at all; the read-only pre-check keeps the
    // rewriting walk (which rebuilds expression trees) off that path entirely.
    if !contains_subplan(plan) {
        return Ok(());
    }
    let here = level_scope(plan, schema);
    walk_level(
        plan,
        &here,
        &ColumnScope::default(),
        schema,
        false,
        chain_is_complete,
    )
}

/// Judge the nested level `plan` against `enclosing`, the whole namespace around it, at
/// the position a statement's own level reaches it — so its verdict is raised in that
/// level's walk order rather than after every one of its clauses.
///
/// A copy is re-run, because the walk reaching it reads the plan and does not own it;
/// the statement's own [`resolve_correlated_subplans`] still rewrites the sub-plan in
/// place, and raises nothing this did not.
///
/// # Errors
///
/// What [`resolve_level`] raises over a complete chain.
pub(super) fn judge_nested_level(
    plan: &Plan,
    enclosing: &ColumnScope,
    schema: &Schema,
) -> Result<()> {
    let mut level = plan.clone();
    resolve_level(&mut level, enclosing, schema, true)
}

/// Re-resolve one nested level and everything below it.
///
/// `outer` is the chain of levels enclosing `plan` (already innermost-first). This
/// level's own relations are stamped level `0` in front of them, and every expression
/// this level owns is re-run against the result — unless `outer` names no relation at
/// all, in which case the chain adds nothing and the rewrite would be a no-op.
fn resolve_level(
    plan: &mut Plan,
    outer: &ColumnScope,
    schema: &Schema,
    chain_is_complete: bool,
) -> Result<()> {
    crate::depth::check_stack()?;
    // The chain is complete here — this level's own relations plus every enclosing
    // level's — which is what a nested level's scope was not at first lowering, so this
    // is where its binding gate can close. Before it did, the verdict was
    // row-count-dependent: `DELETE FROM t WHERE EXISTS (SELECT 1 FROM u WHERE u.k =
    // zzz.v)` over an **empty** `u` evaluated nothing and reported `DELETE 0` where 18.6
    // raises `42P01`, and an aggregate subquery's `HAVING bogusrel.c` deleted a row.
    let here = level_scope(plan, schema).nested_in(outer);
    // **Only a complete chain may close.** This pass runs at *every* level's own
    // lowering — `lower_select` calls it, and an inner `SELECT` is lowered before the
    // query containing it exists — so `outer` is the whole chain only when the plan this
    // walk started from is the statement's own top level. Closing regardless made a
    // two-level correlated reference `42P01`: at the middle query's own lowering the
    // outermost relation is simply not there yet.
    let here = if chain_is_complete {
        here.closed()
    } else {
        here
    };
    // **Always rewrite a nested level.** `rewrite` used to be `!outer.is_empty()`,
    // skipping the pass when the enclosing level contributed no relations — which is
    // precisely a level whose own scope is the whole chain and whose bogus qualifiers
    // therefore have nowhere left to hide. The pass is what runs the gate, so skipping
    // it skipped the gate.
    walk_level(plan, &here, outer, schema, true, chain_is_complete)
}

/// The scope a `Plan::Sort` / `Plan::DistinctOn` key re-resolves against: this level's
/// FROM relations, plus the **target-list entries** the sorted input carries.
///
/// **Three callers, and only the first re-resolves anything.** [`walk_level`] uses it for
/// a nested level's re-walk; `super::query::plan_no_from_distinct_on` and
/// `super::query::plan_no_from_order_limit` use it at a no-FROM select's *own* lowering —
/// for the `DISTINCT ON` keys and the trailing `ORDER BY` keys respectively — where the
/// FROM half is empty and the target list is therefore the whole namespace. Widening what
/// this offers is a change to all three, deliberately — the claimant set a bare key may
/// name is one rule, and a clause whose readers each hold their own copy of a rule is
/// how that rule drifts.
///
/// The second half is Postgres's `findTargetlistEntrySQL92` branch, spelled as a
/// claimant set rather than as a blanket exemption. `input` is the node the key reads
/// its row from, so its `output_columns` are exactly the names the key may answer from —
/// the select list, and the `pglet_junk_N` columns the planner minted for the keys the
/// select list could not answer. A name in neither that list nor the FROM relations
/// binds to nothing and is `42703`.
///
/// Each entry carries its **expression** beside its name ([`OutputEntry`]), because the
/// C loop's ambiguity test is `equal()` over two matching entries and not their count.
/// A junk column cannot add a claimant to a user's name for a reason worth checking
/// rather than assuming — Postgres skips `resjunk` entries here, while pglet keeps them
/// and relies on `super::query`'s minter reserving `pglet_junk_N` against the clause's
/// own output names, so no junk entry ever shares a name with a real one. The mirror
/// residual a previous reading of this claimed — that `ORDER BY pglet_junk_0` resolves
/// here and is `42703` there — does **not** reproduce: re-probed at both positions
/// (nested inside an `EXISTS`, and at top level) over all four minters (the aggregate,
/// window, `DISTINCT ON` and plain-sort ones), every nested one of those eight
/// **refuses** on both engines, and the top-level one succeeds only over an **empty** relation, where the
/// literal `totally_bogus` in the same position succeeds identically. One nested spelling
/// refuses with a *different* SQLSTATE, and it is not the junk name's doing:
/// `EXISTS (SELECT DISTINCT ON (c) c FROM jk ORDER BY pglet_junk_0)` is `42703` on 18.6
/// and `42P10 SELECT DISTINCT ON expressions must match initial ORDER BY expressions`
/// here — the same `42P10` at top level. What identifies it is that an ordinary bogus
/// name in that position answers identically (`… ORDER BY totally_bogus` is `42P10` here
/// too, at either row count), so what answers is the `DISTINCT ON`/`ORDER BY` prefix
/// check running ahead of name resolution, and nothing about `pglet_junk_N` is involved.
/// So nothing is left there: a level's own lowering carries the same claimant set now
/// ([`target_list_entries`]), and the junk name is not a member of anything of its own.
///
/// **Which keys may ride it is the key's own shape, and the caller decides that** —
/// [`key_may_name_an_output_column`], the same predicate `resolve_sort_key` uses at the
/// first lowering, never a second copy of it. `findTargetlistEntrySQL92` is entered only
/// for a key that *is* a one-field `ColumnRef`, so a qualifier (`t.lbl`) and any nesting
/// (`lbl+0`, `-lbl`, `lower(lbl)`) alike take the key out of the rule and leave every
/// name inside it an ordinary column reference gated against the FROM relations. Pushing
/// these names for the whole key tree instead was an over-acceptance measured on 18.6:
/// over `tt(q, a, b)`, `SELECT (SELECT q*10 AS z FROM tt ORDER BY z+0 LIMIT 1)` answered
/// `10` and `SELECT (SELECT count(*) AS c FROM tt GROUP BY a ORDER BY c+0 LIMIT 1)`
/// answered `1`, both `42703` there — while the *top-level* spelling of the first,
/// judged by `resolve_sort_key`, was already `42703` here. That pair is the
/// discriminator: the same key shape, judged correctly at one phase and wrongly at the
/// other.
pub(super) fn key_scope(here: &ColumnScope, input: &Plan, schema: &Schema) -> ColumnScope {
    let mut scope = here.clone();
    scope.push_output_entries(output_entries(here, input, schema));
    scope
}

/// The sorted input's output columns as [`OutputEntry`]s — name plus, where the input
/// **is** a target list, the entry's expression in the shape `equal()` compares.
///
/// The name half is `input.output_columns`, unchanged. The expression half comes from
/// [`target_list_exprs`] and is offered only when it lines up one-for-one with those
/// columns: a sole bare `*` is one projection expression standing for N output columns,
/// so zipping them would pair a name with the wrong entry. Where they do not line up
/// every identity is `None`, which is the pre-existing count-by-name behaviour.
fn output_entries(here: &ColumnScope, input: &Plan, schema: &Schema) -> Vec<OutputEntry> {
    let columns = input.output_columns(schema);
    let exprs = target_list_exprs(input)
        .filter(|exprs| exprs.len() == columns.len() && !exprs.iter().any(Expr::is_bare_star));
    columns
        .into_iter()
        .enumerate()
        .map(|(i, column)| OutputEntry {
            name: column.name,
            expr: exprs.map(|exprs| entry_identity(&exprs[i], here)),
        })
        .collect()
}

/// A target list's entries, for the caller that **has** the list rather than the plan
/// node carrying it — `super::query::lower_select_level`, which resolves its `ORDER BY` /
/// `DISTINCT ON` keys against its own select list once the wildcards are expanded.
///
/// The sibling of [`output_entries`] and deliberately not a second rule: both call the
/// same [`entry_identity`], so the `AS`-label strip and the bare-column binding decide
/// ambiguity identically at a level's own lowering and at the correlated re-walk. What
/// differs is only where the pair comes from — here the entry *is* the expression, so
/// every identity is `Some` and the name is [`Expr::output_name`] (which is what
/// `Plan::output_columns` names a projected entry too), where `output_entries` has to
/// line a plan's columns up against a target list that may not exist.
///
/// `targets` must be the **expanded** select list: a sole bare `*` is one expression
/// standing for N output columns, so the caller passes the star expansion instead.
pub(super) fn target_list_entries(targets: &[Expr], here: &ColumnScope) -> Vec<OutputEntry> {
    targets
        .iter()
        .map(|expr| OutputEntry {
            name: expr.output_name(),
            expr: Some(entry_identity(expr, here)),
        })
        .collect()
}

/// The expressions of `plan`'s target list, or `None` when `plan` emits a **range
/// table** row rather than a target list.
///
/// Three variants are matched: a `Project`, an `Aggregate` (a `GROUP BY` query's target
/// list — `SELECT c, c FROM ja2 WHERE false GROUP BY c ORDER BY c` reaches this arm and
/// no other), and a `ProjectUnnest`. Everything else answers `None` — the `Plan::Join` or
/// `Plan::Scan` a sort placed *below* the projection reads, whose same-named columns are
/// distinct columns Postgres would compare as distinct `Var`s.
///
/// `Plan::Values` is the one unmatched variant that arguably *is* a target list, and
/// **leaving it out is a measured false rejection** — not the free choice a previous
/// reading of this recorded. Neither half of that reading held. Its output names are not
/// always `column1`, `column2`, …: the FROM-clause spelling takes a user alias list, so
/// `FROM (VALUES (1,2)) v(a, a)` carries `a` twice (`42702` on both engines, measured).
/// And it is not out of reach either: a no-FROM `SELECT` lowers straight to
/// `Plan::Values` (`parser::select`'s `plan_select_no_from`), and `plan_no_from_distinct_on`
/// wraps that in `Plan::Sort` under a `Plan::DistinctOn` — so
/// `SELECT DISTINCT ON (q) 1 AS q, 1 AS q` reaches this walk with `Values` as the peeled
/// node, every identity is `None`, and the two entries count as two claimants: `42702`
/// here against 18.6's row (measured). Its **top-level** spelling reaches the same
/// derivation now, through `plan_no_from_distinct_on`'s own binding gate rather than this
/// walk, and answers the identical `42702` the eval-time resolver used to raise — so the
/// false rejection is one member at both positions rather than two. **A `Plan::Values` arm
/// would not retire the shape, though, because a second cause carries it**: over a
/// projection that lowers to `Plan::ProjectUnnest` this walk *does* answer, both entries
/// carry the identity `1`, and the plan-time gate correctly accepts — yet
/// `SELECT DISTINCT ON (q) unnest(ARRAY[1,2]) AS z, 1 AS q, 1 AS q` is still `42702` here
/// against 18.6's row, now from the **eval-time** resolver, whose claimant count is
/// by name. The empty-array spelling of that statement is `(0 rows)` on both engines
/// (measured), which is what puts the refusal at eval rather than at the gate.
/// The *`ORDER BY`* spelling of the same shape used to be out of it, and the reason was a
/// second gap rather than a defence: a no-FROM `SELECT` consumed and **discarded** the
/// trailing `ORDER BY` and `LIMIT`/`OFFSET` it parsed, so no key survived for any gate to
/// judge. That tail routes through `super::query::plan_no_from_order_limit` now, which
/// binds its keys against this same claimant set — so `SELECT 1 AS q, 1 AS q ORDER BY q`
/// is `42702` here against 18.6's row, joining the `DISTINCT ON` spelling as a **second
/// member** of this gap rather than an exemption from it. The control that identifies the
/// gap rather than the gate is `SELECT 1 AS q, 2 AS q ORDER BY q`, which is `42702` on
/// **both** engines — two entries 18.6 also calls ambiguous. A `Values` arm would
/// have to answer only for the no-FROM row, since a FROM-clause `(VALUES …)` sits under a
/// `Plan::SubqueryScan` this walk does not peel and whose columns *are* range-table
/// columns.
///
/// The column-preserving wrappers are peeled first, and that is load-bearing rather than
/// defensive: a `DISTINCT ON` reads its keys **above** the sort, so its input is
/// `Sort { Project { … } }` and stopping at the `Sort` reported every one of its target
/// list's entries as identity-less — `SELECT DISTINCT ON (c) c, c FROM ja2 WHERE false`
/// was `42702` against 18.6's `(0 rows)`. The set peeled is exactly
/// `Plan::output_columns_scoped`'s pass-through arm, so the peeled node's columns are
/// the ones the caller lined the entries up against. `Plan::JunkFilter` is **not** in
/// it — it is the one wrapper that drops columns, so peeling it would offer entries the
/// sorted row does not carry.
fn target_list_exprs(plan: &Plan) -> Option<&[Expr]> {
    match plan {
        Plan::Filter { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input, .. }
        | Plan::DistinctOn { input, .. }
        | Plan::Limit { input, .. } => target_list_exprs(input),
        Plan::Project { exprs, .. }
        | Plan::Aggregate { exprs, .. }
        | Plan::ProjectUnnest { exprs } => Some(exprs.as_slice()),
        _ => None,
    }
}

/// One target-list entry's identity, in the shape Postgres's `equal()` compares.
///
/// Two normalizations, each measured as a false rejection without it (both `(0 rows)` on
/// 18.6, both `42702` here before this):
///
/// - the `AS` label is **stripped**, because `resname` lives on the `TargetEntry` and not
///   on the expression — `SELECT c AS c, c FROM ja2 WHERE false ORDER BY c`;
/// - a **bare** column reference is bound to the relation carrying it, because a
///   transformed `Var` records the range-table entry and not the spelling —
///   `SELECT ja2.c, c FROM ja2 WHERE false ORDER BY c`, the shape a `SELECT *, c`
///   produces since wildcard expansion emits qualified references.
fn entry_identity(expr: &Expr, here: &ColumnScope) -> Expr {
    let expr = match expr {
        Expr::Alias { expr, .. } => expr.as_ref(),
        other => other,
    };
    bind_bare_columns(expr.clone(), here)
}

/// `expr` with every bare column reference qualified by the level-0 relation that
/// carries it, leaving one no single relation claims alone — see
/// [`ColumnScope::sole_relation_carrying`].
fn bind_bare_columns(mut expr: Expr, here: &ColumnScope) -> Expr {
    bind_bare_columns_in_place(&mut expr, here);
    expr
}

/// [`bind_bare_columns`] in place: the chain-forming variants are walked from this frame
/// rather than through a visitor's, so a chain costs one frame a level
/// (`crate::depth`).
fn bind_bare_columns_in_place(expr: &mut Expr, here: &ColumnScope) {
    match expr {
        Expr::Column { table, name } => {
            if table.is_none() {
                *table = here.sole_relation_carrying(name).map(ToString::to_string);
            }
        }
        Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } => {
            bind_bare_columns_in_place(left, here);
            bind_bare_columns_in_place(right, here);
        }
        Expr::Negate(child)
        | Expr::UnaryPlus(child)
        | Expr::Not(child)
        | Expr::IsNull(child)
        | Expr::IsNotNull(child)
        | Expr::Cast { expr: child, .. }
        | Expr::CastTypmod { expr: child, .. }
        | Expr::InSubquery { expr: child, .. }
        | Expr::BooleanTest { expr: child, .. } => bind_bare_columns_in_place(child, here),
        Expr::InList {
            expr: child, list, ..
        } => {
            bind_bare_columns_in_place(child, here);
            bind_bare_columns_in_list(list, here);
        }
        other => other.for_each_child_mut(&mut |child| bind_bare_columns_in_place(child, here)),
    }
}

/// [`bind_bare_columns_in_place`] over an `IN` list's members.
#[inline(never)]
fn bind_bare_columns_in_list(list: &mut [Expr], here: &ColumnScope) {
    for item in list {
        bind_bare_columns_in_place(item, here);
    }
}

/// The scope one key of a `Plan::Sort` / `Plan::DistinctOn` resolves against — `keyscope`
/// when the key's shape reaches `findTargetlistEntrySQL92`, `here` otherwise.
///
/// The split between "this key may name an output column" and "this key's names are
/// ordinary FROM references" is one rule, and a second copy of it anywhere would be the
/// seam this module's own doc names — so every reader of that split calls this. They are
/// [`walk_level`]'s `Plan::Sort` and `Plan::DistinctOn` arms (a nested level's re-walk),
/// `super::query::resolve_sort_key` (a FROM-bearing level's own lowering, for both
/// clauses), `super::query::check_set_op_key_leaves` (a set-op's trailing `ORDER BY`,
/// applied per leaf one level down), and `super::query::plan_no_from_distinct_on` /
/// `super::query::plan_no_from_order_limit` (a no-FROM select's own lowering, where the
/// FROM half is empty). Read that as the readers there are, not as a count: a new clause
/// whose bare keys reach `findTargetlistEntrySQL92` belongs on this list rather than
/// beside it.
pub(super) const fn scope_for_key<'a>(
    key: &Expr,
    here: &'a ColumnScope,
    keyscope: &'a ColumnScope,
) -> &'a ColumnScope {
    if key_may_name_an_output_column(key) {
        keyscope
    } else {
        here
    }
}

/// Walk the nodes belonging to **one** query level, re-resolving each expression
/// against `here` (when `rewrite`) and handing every nested level the chained scope.
///
/// The match is exhaustive: a new `Plan` variant is a compile error here rather than a
/// silently unvisited node.
fn walk_level(
    plan: &mut Plan,
    here: &ColumnScope,
    outer: &ColumnScope,
    schema: &Schema,
    rewrite: bool,
    chain_is_complete: bool,
) -> Result<()> {
    crate::depth::check_stack()?;
    // The arms a nested level and a plan chain recurse through most are walked from
    // this frame; every other one in [`walk_level_rest`]'s (`crate::depth`). Each
    // keeps the order the arm states there.
    match plan {
        Plan::Filter { input, predicate } => {
            visit_level_clause(
                predicate,
                BoolContext::Where,
                here,
                schema,
                rewrite,
                chain_is_complete,
            )?;
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        Plan::Project {
            input,
            exprs,
            enclosing,
            ..
        } => {
            for e in exprs.iter_mut() {
                visit_expr(e, here, schema, rewrite, chain_is_complete)?;
            }
            record_enclosing(exprs.iter(), here, enclosing, rewrite);
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        Plan::Distinct { input } | Plan::Limit { input, .. } | Plan::JunkFilter { input, .. } => {
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        Plan::Join {
            left, right, on, ..
        } => {
            visit_level_clause(
                on,
                BoolContext::JoinOn,
                here,
                schema,
                rewrite,
                chain_is_complete,
            )?;
            walk_level(left, here, outer, schema, rewrite, chain_is_complete)?;
            walk_level(right, here, outer, schema, rewrite, chain_is_complete)
        }
        _ => walk_level_rest(plan, here, outer, schema, rewrite, chain_is_complete),
    }
}

/// A **clause root** of a nested level: its expression visited, then the boolean
/// context the clause puts it in gated — only when the pass rewrites, which is the same
/// condition the resolve rides (see [`walk_level_rest`]'s `visit_bool`).
fn visit_level_clause(
    expr: &mut Expr,
    ctx: BoolContext,
    here: &ColumnScope,
    schema: &Schema,
    rewrite: bool,
    chain_is_complete: bool,
) -> Result<()> {
    visit_expr(expr, here, schema, rewrite, chain_is_complete)?;
    if rewrite {
        check_boolean_clause(expr, ctx, here, schema)?;
    }
    Ok(())
}

/// [`walk_level`]'s arms other than the ones it walks itself.
#[inline(never)]
fn walk_level_rest(
    plan: &mut Plan,
    here: &ColumnScope,
    outer: &ColumnScope,
    schema: &Schema,
    rewrite: bool,
    chain_is_complete: bool,
) -> Result<()> {
    let visit =
        |e: &mut Expr| -> Result<()> { visit_expr(e, here, schema, rewrite, chain_is_complete) };
    // A **clause root** carries a boolean context the clause-agnostic `visit` cannot
    // see, so the three arms that own one say which they are. Without this every
    // nested level's `WHERE` / `HAVING` / `ON` is ungated against the chained scope —
    // the half that silently *writes*, since a correlated reference types to `None` at
    // the level's own lowering and only this re-walk can see it.
    let visit_bool = |e: &mut Expr, ctx: BoolContext| -> Result<()> {
        visit_expr(e, here, schema, rewrite, chain_is_complete)?;
        // Gated only when the pass rewrites, which is the same condition the resolve
        // rides: the statement's own top level was already gated at its lowering, and
        // `resolve_correlated_subplans` enters with `rewrite = false` for exactly that
        // reason.
        if rewrite {
            check_boolean_clause(e, ctx, here, schema)?;
        }
        Ok(())
    };
    match plan {
        Plan::LockRows { input } => {
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        // Row-shaping nodes: their expressions belong to this level, and their input
        // stays at this level too.
        // Walked by `walk_level` itself: never here.
        Plan::Filter { .. }
        | Plan::Project { .. }
        | Plan::Distinct { .. }
        | Plan::Limit { .. }
        | Plan::JunkFilter { .. }
        | Plan::Join { .. } => walk_level(plan, here, outer, schema, rewrite, chain_is_complete),
        // `ORDER BY` and `DISTINCT ON` keys are the two clauses whose **bare** names may
        // name an output column rather than a FROM one (`findTargetlistEntrySQL92`), so
        // re-gating them against the FROM scope alone is a false rejection, and this walk
        // is where it was reachable: `SELECT (SELECT count(*) AS c FROM g GROUP BY k
        // ORDER BY c LIMIT 1)` was `42703` where 18.6 answers, since only a *nested*
        // level re-resolves its own expressions. Every junk column (`Plan::JunkFilter`)
        // is such a bare name too, so this is also what lets a key the output cannot
        // answer survive inside a subquery.
        //
        // The scope those names ride is `key_scope`: this level's FROM relations plus
        // the input's output columns as an **explicit claimant set**. The blanket
        // decline that stood here — and, until the keys moved below the projection, at a
        // level's own lowering too — judged
        // *no* bare name, so one binding to nothing survived plan time — and a statement
        // matching no row never reaches the eval-time resolver either, so it was never
        // judged at all: `DELETE FROM dt WHERE a NOT IN (SELECT a FROM du ORDER BY
        // bogus)` over an empty `du` removed **every** row where 18.6 is `42703`.
        Plan::Sort {
            input, order_by, ..
        } => {
            let keyscope = key_scope(here, input, schema);
            for key in order_by {
                let scope = scope_for_key(&key.expr, here, &keyscope);
                visit_expr(&mut key.expr, scope, schema, rewrite, chain_is_complete)?;
            }
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        Plan::DistinctOn { on_exprs, input } => {
            let keyscope = key_scope(here, input, schema);
            for e in on_exprs {
                let scope = scope_for_key(e, here, &keyscope);
                visit_expr(e, scope, schema, rewrite, chain_is_complete)?;
            }
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        Plan::Aggregate {
            input,
            group_by,
            exprs,
            having,
            enclosing,
            ..
        } => {
            for e in group_by.iter_mut().chain(exprs.iter_mut()) {
                visit(e)?;
            }
            if let Some(h) = having {
                visit_bool(h, BoolContext::Having)?;
            }
            record_enclosing(exprs.iter(), here, enclosing, rewrite);
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        Plan::Window {
            input,
            window_exprs,
            enclosing,
            ..
        } => {
            for e in window_exprs.iter_mut() {
                visit(e)?;
            }
            record_enclosing(window_exprs.iter(), here, enclosing, rewrite);
            walk_level(input, here, outer, schema, rewrite, chain_is_complete)
        }
        Plan::Values {
            rows, enclosing, ..
        } => {
            for row in rows.iter_mut() {
                for e in row {
                    visit(e)?;
                }
            }
            record_enclosing(rows.iter().flatten(), here, enclosing, rewrite);
            Ok(())
        }
        Plan::ProjectUnnest { exprs } => {
            for e in exprs {
                visit(e)?;
            }
            Ok(())
        }
        Plan::FunctionScan { args, .. } => {
            for e in args {
                visit(e)?;
            }
            Ok(())
        }
        // The accelerated read paths, visited **defensively — not load-bearing.**
        // Index selection has already destructured the comparison by the time these
        // nodes exist, so a visit here sees a lone probe value and never the column it
        // was compared against: it reaches neither the coercion (which casts a literal
        // only *beside* a typed column) nor the gate (which needs both operand types).
        // Making these three arms inert reddens nothing today.
        //
        // Two-profile parity holds for a different reason, and would hold without them.
        // An index probe's column is always a level-0 relation of the level being
        // resolved, so chaining the enclosing levels onto the scope cannot change the
        // type that column resolves to — the widened scope reaches the same verdict as
        // the narrow one. And `is_row_independent` (`index_selection.rs`) admits only
        // `Expr::Literal` / `Parameter` / `Bound` as a probe value, so a sub-plan
        // carrying an outer reference can never occupy one of these slots in the first
        // place. The arms stay because they cost nothing and keep the walk total over
        // the plan IR as these shapes grow.
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { key_value, .. } => visit(key_value),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexRangeScan { lower, upper, .. } => {
            for bound in lower.iter_mut().chain(upper.iter_mut()) {
                visit(&mut bound.value)?;
            }
            Ok(())
        }
        #[cfg(feature = "index_acceleration")]
        Plan::GinScan { probes, guards, .. } => {
            for e in probes.iter_mut().chain(guards.iter_mut()) {
                visit(e)?;
            }
            Ok(())
        }
        // A **FROM-clause** nested level chains from `outer`, not `here`: a derived
        // table, an inlined read-only CTE, a set-op arm, and a `WITH` CTE body all see
        // the *enclosing* query levels but **not** this level's own FROM relations.
        // That is Postgres's rule — without `LATERAL`, a FROM subquery referencing a
        // sibling relation is an error, not a binding — and passing `here` would hand
        // it a sibling's column type to cast against.
        Plan::SubqueryScan { subquery, .. } => {
            resolve_level(subquery, outer, schema, chain_is_complete)
        }
        // Each set-op arm is its own level, chained past the set-op node itself: the
        // node introduces no relation name, so it is not a namespace level at all.
        //
        // A column whose type waited on an enclosing level (`outer_pending`) is unified
        // here, once both arms are re-walked, against each arm's scope chained onto
        // `outer` — the namespace an arm's references resolve in — and the reference
        // types it was unified with are recorded for the statement's second parse
        // (`SetOpOuterTypes`).
        Plan::SetOp {
            op,
            left,
            right,
            column_types,
            outer_pending,
            outer_types,
            ..
        } => {
            resolve_level(left, outer, schema, chain_is_complete)?;
            resolve_level(right, outer, schema, chain_is_complete)?;
            if outer_pending.is_empty() {
                return Ok(());
            }
            let left_level = level_scope(left, schema);
            let left_chained = left_level.clone().nested_in(outer);
            let right_level = level_scope(right, schema);
            let right_chained = right_level.clone().nested_in(outer);
            let columns = std::mem::take(outer_pending);
            let unified = unify_set_op_columns(
                SetOpColumns {
                    op: *op,
                    left,
                    right,
                    column_types,
                    columns: &columns,
                },
                schema,
                &OuterTyping::Chained {
                    left: SideScopes {
                        level: &left_level,
                        chained: &left_chained,
                    },
                    right: SideScopes {
                        level: &right_level,
                        chained: &right_chained,
                    },
                    complete: chain_is_complete,
                },
            )?;
            if let Some(record) = outer_types {
                record.resolve(&unified.unified);
            }
            *outer_pending = unified.pending;
            Ok(())
        }
        // A `WITH`'s body *is* this statement's query level; each materialized CTE's
        // sub-plan (a data-modifying CTE's DML, a recursive CTE's two terms) is a
        // separate FROM-clause level.
        Plan::With { ctes, body } => {
            for plan in ctes.iter_mut().flat_map(CtePlan::plans_mut) {
                resolve_level(plan, outer, schema, chain_is_complete)?;
            }
            walk_level(body, here, outer, schema, rewrite, chain_is_complete)
        }
        // `UPDATE` / `DELETE` are query levels too: their target relation (plus an
        // `UPDATE … FROM` source) is exactly what a correlated `WHERE EXISTS (…)`
        // subquery must be able to see. Entered from `lower_update` / `lower_delete`.
        Plan::Update {
            from,
            assignments,
            filter,
            returning,
            ..
        } => {
            for (_, e) in assignments.iter_mut() {
                visit(e)?;
            }
            for e in filter.iter_mut().chain(returning.iter_mut()) {
                visit(e)?;
            }
            from.as_mut().map_or(Ok(()), |source| {
                walk_level(source, here, outer, schema, rewrite, chain_is_complete)
            })
        }
        Plan::Delete {
            filter, returning, ..
        } => {
            for e in filter.iter_mut().chain(returning.iter_mut()) {
                visit(e)?;
            }
            Ok(())
        }
        // An `INSERT` is two clauses with **two different namespaces**, so it is the one
        // arm that does not read `here`. `RETURNING` names the target relation
        // ([`insert_returning_relations`]); the `ON CONFLICT DO UPDATE` action names the
        // target and `EXCLUDED` ([`on_conflict_action_scope`]); and neither may reach an
        // enclosing level, so neither chains onto `outer` — a CTE's `INSERT` cannot see
        // the query containing it, and a top-level one has nothing to see. Both scopes
        // are the **same objects** the clauses' own lowering gated against, taken from
        // `super::dml` rather than rebuilt here.
        //
        // The clauses **not** visited are `VALUES` cells, the `INSERT … SELECT` source and
        // the `ON CONFLICT` **arbiter target**, and each for its own reason. The
        // `INSERT … SELECT` source is lowered as a statement in its own right and
        // re-walked by its own `lower_select` (`INSERT INTO it SELECT (SELECT b FROM et
        // ORDER BY zzz)` is `42703` on both engines, with and without this arm). A
        // `VALUES` cell is **loud for an unrelated structural reason**, and the
        // distinction matters: pglet cannot evaluate *any* subquery in a `VALUES` cell —
        // scalar, `EXISTS` and `IN` alike are `0A000 … requires execution context` — so
        // `INSERT INTO t9 VALUES ((SELECT b FROM e9 ORDER BY zzz))` is `0A000` here
        // against 18.6's `42703`, and its nested level is not *judged* so much as never
        // reached. (The derived-table spelling `VALUES ((SELECT q FROM (SELECT b AS q
        // FROM et ORDER BY zzz) x))` *is* `42703` on both, but only because
        // `Parser::in_from_item` resets `expr_subquery_depth` and the level's own scope
        // was therefore complete at its lowering — a property of the reset, not of this
        // clause.) So the day that `0A000` closes, the `VALUES` route becomes a
        // silent-wrong route with no guard, and this arm is where it would be closed.
        // The arbiter is never resolved or gated at all —
        // `lower_on_conflict` leaves it alone deliberately, since gating it would change
        // how an arbiter matches an expression index, and this arm keeps that posture.
        //
        // Until this arm existed those clauses' nested levels were judged by nobody: an
        // expression subquery encloses them, so each deferred its plan-time binding gate
        // at its own lowering, and `resolve_correlated_subplans` never ran over an
        // `INSERT` to close it. Over an empty inner relation the eval-time resolver never
        // fires either, so `INSERT INTO it VALUES (1) RETURNING (SELECT b FROM et ORDER
        // BY zzz)` wrote its row and the no-conflict `ON CONFLICT (id) DO UPDATE SET v =
        // (SELECT b FROM et ORDER BY zzz)` inserted, both `42703` on 18.6.
        //
        // **`rewrite` is ignored here**, unlike every other arm, and the honest reason is
        // narrower than "only the descent is wanted". Passing the caller's `rewrite`
        // through is *indistinguishable* from hard-coding `false`: the only live entry to
        // this arm is `resolve_correlated_subplans`'s own `walk_level`, which passes
        // `false`, and an `assert!(!rewrite)` here never fires over the whole fixture
        // corpus or `cargo test --workspace`. Hard-coding `true` **is** distinguishable,
        // and on the statement that separates them it matches 18.6 where this does not:
        // over a `td(id PK, v INT, tx TEXT)`, the non-conflicting `… ON CONFLICT (id) DO
        // UPDATE SET v = 'abc', tx = (SELECT max(b)::text FROM ed)` is `22P02` there and
        // writes the row here. That is not a reason to flip it, because the divergence is
        // a **broader pre-existing class** — the same `SET v = 'abc'` with no subquery
        // anywhere writes silently too — so `true` would close only the sub-case that
        // happens to carry a sibling subquery, an arbitrary half-close. The class is tracked separately;
        // this arm stays on `false` so it closes none of it rather than
        // half of it.
        //
        // A target the catalog does not hold is skipped for [`lowers_clauses`]'s reason:
        // its columns are not derivable, and a closed scope over an empty relation would
        // answer `42703` in front of the executor's `42P01` for the table itself. **That
        // branch is unguarded**, and honestly so: `lower_insert` tests the same predicate
        // before calling, and the only other route to this arm — a data-modifying CTE's
        // `INSERT` through `Plan::With`'s `resolve_level` — is not live, because
        // `wrap_with_ctes` builds the `Plan::With` *after* the body's own
        // `resolve_correlated_subplans` has run and nothing walks the assembled node.
        // Measured: with this branch mutated to `panic!`, the whole fixture corpus and
        // `cargo test --workspace` stay green. It stays because that route is written for
        // and would make the arm silently wrong the day it goes live.
        Plan::Insert {
            table,
            returning,
            on_conflict,
            ..
        } => {
            if !lowers_clauses(schema, table) {
                return Ok(());
            }
            // `ON CONFLICT` before `RETURNING`, which is `analyze.c`'s order for an
            // INSERT (row source, then `ON CONFLICT`, then `RETURNING`) and so decides
            // which clause reports when both are wrong. Measured on 18.6: over an empty
            // `eh`, `INSERT INTO th VALUES (3,3) ON CONFLICT (id) DO UPDATE SET v =
            // (SELECT b FROM eh ORDER BY aaa) RETURNING (SELECT b FROM eh ORDER BY bbb)`
            // is `42703 "aaa"`. Visiting `returning` first answers `"bbb"` — loud either
            // way, so no fixture that pins one clause alone can see it, which is why the
            // two-key fixture below exists.
            if let Some(OnConflictAction::DoUpdate {
                assignments,
                where_predicate,
            }) = on_conflict.as_mut().map(|c| &mut c.action)
            {
                let action_scope = on_conflict_action_scope(table, schema);
                for (_, e) in assignments.iter_mut() {
                    visit_expr(e, &action_scope, schema, false, chain_is_complete)?;
                }
                if let Some(predicate) = where_predicate {
                    visit_expr(predicate, &action_scope, schema, false, chain_is_complete)?;
                }
            }
            let clause_scope = returning_scope(&insert_returning_relations(table, schema));
            for e in returning.iter_mut() {
                visit_expr(e, &clause_scope, schema, false, chain_is_complete)?;
            }
            Ok(())
        }
        // Relation leaves and every non-query statement: no expression at this level
        // to re-resolve. A `DoBlock`'s statements are lowered as statements in their own
        // right rather than as nested *expression* levels, so they are not descended.
        Plan::Scan { .. }
        | Plan::VirtualScan { .. }
        | Plan::CteScan { .. }
        | Plan::Empty
        | Plan::Copy { .. }
        | Plan::CreateTable { .. }
        | Plan::DropTable { .. }
        | Plan::Truncate { .. }
        | Plan::CreateIndex { .. }
        | Plan::DropIndex { .. }
        | Plan::CreateType { .. }
        | Plan::DropType { .. }
        | Plan::DropSchema { .. }
        | Plan::CreateSchema { .. }
        | Plan::AlterTableAddColumn { .. }
        | Plan::AlterTableDropColumn { .. }
        | Plan::AlterTableAddCheck { .. }
        | Plan::AlterTableAddConstraint { .. }
        | Plan::AlterTableSetDefault { .. }
        | Plan::AlterTableSetNotNull { .. }
        | Plan::AlterTableAddIdentity { .. }
        | Plan::CreateSequence { .. }
        | Plan::AlterSequence { .. }
        | Plan::DropSequence { .. }
        | Plan::Noop { .. }
        | Plan::AlterTableUnperformed { .. }
        | Plan::Savepoint { .. }
        | Plan::RollbackToSavepoint { .. }
        | Plan::ReleaseSavepoint { .. }
        | Plan::BeginTransaction { .. }
        | Plan::CommitTransaction
        | Plan::RollbackTransaction
        | Plan::DoBlock { .. } => Ok(()),
    }
}

/// Record, on a projecting node of a nested level, the identity each of its target
/// list's references an enclosing level binds ([`EnclosingColumns`]) — under `here`,
/// the level chained onto the levels around it, which no scope had before this re-run.
///
/// Only a nested level's walk records (`rewrite`): the statement's own level binds every
/// reference it makes. A reference no level of `here` binds is left out, so a re-run
/// over an incomplete chain leaves it undecided rather than recording a guess.
fn record_enclosing<'e>(
    exprs: impl IntoIterator<Item = &'e Expr>,
    here: &ColumnScope,
    record: &EnclosingColumns,
    rewrite: bool,
) {
    if !rewrite {
        return;
    }
    let mut refs = Vec::new();
    for e in exprs {
        collect_referenced_columns(e, &mut refs);
    }
    let mut entries: Vec<(Option<String>, ColumnDesc)> = Vec::new();
    for (table, name) in refs {
        if entries.iter().any(|(q, c)| *q == table && c.name == name) {
            continue;
        }
        let Some((oid, enum_type)) = here
            .enclosing_binding(table.as_deref(), &name)
            .map(|bound| (bound.oid, bound.enum_type.name().map(ToString::to_string)))
        else {
            continue;
        };
        entries.push((
            table,
            ColumnDesc {
                enum_type,
                ..ColumnDesc::new(name, oid)
            },
        ));
    }
    record.record(entries);
}

/// Re-resolve `expr` against `here` (when `rewrite`), then hand every sub-plan it
/// embeds the chained scope.
fn visit_expr(
    expr: &mut Expr,
    here: &ColumnScope,
    schema: &Schema,
    rewrite: bool,
    chain_is_complete: bool,
) -> Result<()> {
    if rewrite {
        let taken = std::mem::replace(expr, Expr::Default);
        *expr = resolve_expr_types_gated(taken, here, schema)?;
    }
    descend_subplans(expr, here, schema, chain_is_complete)
}

/// Recurse through `expr` to every embedded sub-plan, resolving each as a nested
/// query level under `here`.
///
/// `Expr::Subquery` / `Expr::Exists` / `Expr::InSubquery` are leaves to
/// `Expr::for_each_child`, so the three are matched here explicitly; everything else
/// delegates its children to the shared walker.
/// [`descend_subplans`] over an `IN` list's members, in order.
#[inline(never)]
fn descend_subplans_in_list(
    list: &mut [Expr],
    here: &ColumnScope,
    schema: &Schema,
    chain_is_complete: bool,
) -> Result<()> {
    list.iter_mut()
        .try_for_each(|item| descend_subplans(item, here, schema, chain_is_complete))
}

fn descend_subplans(
    expr: &mut Expr,
    here: &ColumnScope,
    schema: &Schema,
    chain_is_complete: bool,
) -> Result<()> {
    crate::depth::check_stack()?;
    // The owned `map_children` rebuild below allocates, so skip it when there is
    // demonstrably nothing below to descend into.
    if !expr_contains_subplan(expr) {
        return Ok(());
    }
    match expr {
        Expr::Subquery { plan } | Expr::Exists { plan, .. } => {
            resolve_level(plan, here, schema, chain_is_complete)
        }
        Expr::InSubquery {
            expr: subject,
            plan,
            ..
        } => {
            descend_subplans(subject, here, schema, chain_is_complete)?;
            resolve_level(plan, here, schema, chain_is_complete)
        }
        // The chain-forming variants are descended from this frame (`crate::depth`).
        Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } => {
            descend_subplans(left, here, schema, chain_is_complete)?;
            descend_subplans(right, here, schema, chain_is_complete)
        }
        Expr::InList {
            expr: child, list, ..
        } => {
            descend_subplans(child, here, schema, chain_is_complete)?;
            descend_subplans_in_list(list, here, schema, chain_is_complete)
        }
        Expr::Negate(child)
        | Expr::UnaryPlus(child)
        | Expr::Not(child)
        | Expr::IsNull(child)
        | Expr::IsNotNull(child)
        | Expr::Cast { expr: child, .. }
        | Expr::CastTypmod { expr: child, .. }
        | Expr::BooleanTest { expr: child, .. } => {
            descend_subplans(child, here, schema, chain_is_complete)
        }
        other => {
            // Every child in place, in `map_children`'s order; the first error is
            // captured and returned, and no later child is descended.
            let mut error = None;
            other.for_each_child_mut(&mut |child| {
                if error.is_none()
                    && let Err(e) = descend_subplans(child, here, schema, chain_is_complete)
                {
                    error = Some(e);
                }
            });
            error.map_or(Ok(()), Err)
        }
    }
}

/// Whether `plan` embeds any sub-plan in an expression — the read-only pre-check that
/// keeps the rewriting walk (and the per-level scope construction, which allocates a
/// column map per relation) off the overwhelming majority of statements, which have no
/// subquery at all.
///
/// Exhaustive over `Plan` for the same reason [`walk_level`] is: a new query-shaped
/// variant must be a compile error, not a silently-skipped one. Unlike `walk_level`
/// this only has to find *whether* a sub-plan exists anywhere below, so nested levels
/// and same-level children are treated alike.
fn contains_subplan(plan: &Plan) -> bool {
    let any = |exprs: &[Expr]| exprs.iter().any(expr_contains_subplan);
    match plan {
        Plan::LockRows { input } => contains_subplan(input),
        Plan::Filter { input, predicate } => {
            expr_contains_subplan(predicate) || contains_subplan(input)
        }
        Plan::Project { input, exprs, .. } => any(exprs) || contains_subplan(input),
        Plan::Sort {
            input, order_by, ..
        } => order_by.iter().any(|k| expr_contains_subplan(&k.expr)) || contains_subplan(input),
        Plan::DistinctOn { on_exprs, input } => any(on_exprs) || contains_subplan(input),
        Plan::Aggregate {
            input,
            group_by,
            exprs,
            having,
            ..
        } => {
            any(group_by)
                || any(exprs)
                || having.as_ref().is_some_and(expr_contains_subplan)
                || contains_subplan(input)
        }
        Plan::Window {
            input,
            window_exprs,
            ..
        } => any(window_exprs) || contains_subplan(input),
        Plan::Join {
            left, right, on, ..
        } => expr_contains_subplan(on) || contains_subplan(left) || contains_subplan(right),
        Plan::Distinct { input } | Plan::Limit { input, .. } | Plan::JunkFilter { input, .. } => {
            contains_subplan(input)
        }
        Plan::Values { rows, .. } => rows.iter().any(|row| any(row)),
        Plan::ProjectUnnest { exprs } => any(exprs),
        Plan::FunctionScan { args, .. } => any(args),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { key_value, .. } => expr_contains_subplan(key_value),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexRangeScan { lower, upper, .. } => lower
            .iter()
            .chain(upper.iter())
            .any(|b| expr_contains_subplan(&b.value)),
        #[cfg(feature = "index_acceleration")]
        Plan::GinScan { probes, guards, .. } => any(probes) || any(guards),
        Plan::SubqueryScan { subquery, .. } => contains_subplan(subquery),
        Plan::SetOp { left, right, .. } => contains_subplan(left) || contains_subplan(right),
        Plan::With { ctes, body } => {
            ctes.iter().flat_map(CtePlan::plans).any(contains_subplan) || contains_subplan(body)
        }
        Plan::Update {
            from,
            assignments,
            filter,
            returning,
            ..
        } => {
            assignments.iter().any(|(_, e)| expr_contains_subplan(e))
                || filter.as_ref().is_some_and(expr_contains_subplan)
                || any(returning)
                || from.as_ref().is_some_and(|f| contains_subplan(f))
        }
        Plan::Delete {
            filter, returning, ..
        } => filter.as_ref().is_some_and(expr_contains_subplan) || any(returning),
        // Deliberately **wider** than `walk_level`'s `Plan::Insert` arm, which visits
        // only `RETURNING` and the `ON CONFLICT` action. This is a pre-check whose only
        // job is to keep the walk off statements that have nothing below; over-reporting
        // costs one no-op walk, while under-reporting would skip the walk entirely and
        // is exactly how the `INSERT` clauses' nested levels went unjudged.
        Plan::Insert {
            rows,
            returning,
            on_conflict,
            ..
        } => {
            let rows_carry = match rows {
                InsertRows::Values { rows, .. } => rows.iter().any(|row| any(row)),
                InsertRows::Select { source, .. } => contains_subplan(source),
            };
            rows_carry
                || any(returning)
                || on_conflict
                    .as_ref()
                    .is_some_and(|c| on_conflict_contains_subplan(&c.action))
        }
        // Relation leaves and every non-query statement — see `walk_level`'s
        // matching arm for why none of them is descended.
        Plan::Scan { .. }
        | Plan::VirtualScan { .. }
        | Plan::CteScan { .. }
        | Plan::Empty
        | Plan::Copy { .. }
        | Plan::CreateTable { .. }
        | Plan::DropTable { .. }
        | Plan::Truncate { .. }
        | Plan::CreateIndex { .. }
        | Plan::DropIndex { .. }
        | Plan::CreateType { .. }
        | Plan::DropType { .. }
        | Plan::DropSchema { .. }
        | Plan::CreateSchema { .. }
        | Plan::AlterTableAddColumn { .. }
        | Plan::AlterTableDropColumn { .. }
        | Plan::AlterTableAddCheck { .. }
        | Plan::AlterTableAddConstraint { .. }
        | Plan::AlterTableSetDefault { .. }
        | Plan::AlterTableSetNotNull { .. }
        | Plan::AlterTableAddIdentity { .. }
        | Plan::CreateSequence { .. }
        | Plan::AlterSequence { .. }
        | Plan::DropSequence { .. }
        | Plan::Noop { .. }
        | Plan::AlterTableUnperformed { .. }
        | Plan::Savepoint { .. }
        | Plan::RollbackToSavepoint { .. }
        | Plan::ReleaseSavepoint { .. }
        | Plan::BeginTransaction { .. }
        | Plan::CommitTransaction
        | Plan::RollbackTransaction
        | Plan::DoBlock { .. } => false,
    }
}

/// Whether an `ON CONFLICT` action embeds a sub-plan — the `DO UPDATE` assignments'
/// right-hand sides and the action's `WHERE`; a `DO NOTHING` carries neither.
fn on_conflict_contains_subplan(action: &OnConflictAction) -> bool {
    match action {
        OnConflictAction::DoNothing => false,
        OnConflictAction::DoUpdate {
            assignments,
            where_predicate,
        } => {
            assignments.iter().any(|(_, e)| expr_contains_subplan(e))
                || where_predicate.as_ref().is_some_and(expr_contains_subplan)
        }
    }
}

/// Whether `expr` embeds a sub-plan anywhere within it.
fn expr_contains_subplan(expr: &Expr) -> bool {
    if matches!(
        expr,
        Expr::Subquery { .. } | Expr::Exists { .. } | Expr::InSubquery { .. }
    ) {
        return true;
    }
    let mut found = false;
    expr.for_each_child(|child| {
        found = found || expr_contains_subplan(child);
    });
    found
}

/// The relations addressable by name from **one** query level of `plan`.
///
/// Walks only the same-level structure — through the row-shaping nodes and joins,
/// stopping at each relation leaf. A nested level (a derived table's inner plan, a
/// set-op arm) contributes its *output* columns as one relation, never its internals.
///
/// Any node that introduces a relation this walker cannot fully enumerate registers it
/// **non-exhaustively** — under its name where there is one, opaque where there is not —
/// so a name it might carry stops here instead of
/// escaping to an enclosing level and binding to a same-named relation there. That is
/// the whole guard against the chaining turning an honest over-rejection into a
/// silently wrong cast, so the conservative arms below are load-bearing, not filler.
fn level_scope(plan: &Plan, schema: &Schema) -> ColumnScope {
    let mut scope = ColumnScope::default();
    push_level_relations(&mut scope, plan, schema);
    scope
}

/// [`level_scope`]'s recursive body.
fn push_level_relations(scope: &mut ColumnScope, plan: &Plan, schema: &Schema) {
    match plan {
        Plan::LockRows { input } => push_level_relations(scope, input, schema),
        Plan::Scan { table, alias } => {
            if let Some(t) = schema.get_table(table) {
                scope.push_base_relation(
                    scan_effective_name(table, alias),
                    build_column_type_map(&t.columns),
                );
            } else {
                // A scan of a table the schema has no entry for: the *name* is known —
                // it is the FROM item's — and only the columns are unknowable, so the
                // relation registers non-exhaustively. Registering nothing would let a
                // closed scope answer `42P01` for a qualifier the statement plainly
                // writes, where the execute-time `42P01` for the table is the error
                // 18.6 gives.
                scope.push_partial_relation(scan_effective_name(table, alias), HashMap::new());
            }
        }
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { table, alias, .. }
        | Plan::IndexRangeScan { table, alias, .. }
        | Plan::GinScan { table, alias, .. } => {
            if let Some(t) = schema.get_table(table) {
                scope.push_base_relation(
                    scan_effective_name(table, alias),
                    build_column_type_map(&t.columns),
                );
            } else {
                scope.push_partial_relation(scan_effective_name(table, alias), HashMap::new());
            }
        }
        Plan::VirtualScan {
            catalog_table,
            alias,
        } => {
            let name = alias
                .clone()
                .unwrap_or_else(|| catalog_table.name().to_string());
            // **Non-exhaustive for every catalog view, including the ten that are
            // complete.** pglet emits a strict subset of 18.6's columns for nine of the
            // 17 relations it models (`information_schema.columns` 17 of 44, `pg_class` 9
            // of 34, `pg_attribute` 12 of 25, …), and reading the seven
            // `columns_complete` calls incomplete as complete would turn `SELECT <a column
            // only 18.6's view has> FROM information_schema.columns WHERE …` — which both
            // engines answer — into a false `42703`. This builder does not consult
            // `CatalogTable::columns_complete` and registers every view partially, which
            // is a **weaker** verdict than `build_column_scope`'s and therefore safe: it
            // can only decline to refuse. Closing the ten here is a separate change with
            // its own polarities to measure, not a follow-on from that one.
            scope.push_partial_relation(
                name,
                crate::planner::build_desc_type_map(&catalog_table.columns()),
            );
        }
        Plan::CteScan { alias, columns, .. } => {
            scope.push_derived_relation(alias.clone(), columns);
        }
        Plan::SubqueryScan {
            subquery,
            alias,
            column_aliases,
            cte_rank: _,
        } => {
            scope.push_derived_relation(
                alias.clone(),
                &super::alias_relation_columns(subquery.output_columns(schema), column_aliases),
            );
        }
        // Row-shaping nodes pass their input's relations through unchanged.
        Plan::Filter { input, .. }
        | Plan::Project { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input }
        | Plan::DistinctOn { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Aggregate { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Window { input, .. } => push_level_relations(scope, input, schema),
        Plan::Join {
            left, right, using, ..
        } => {
            // A `USING` column is *one* column however many sides declare it, so the
            // binding gate must not count it once per input relation.
            scope.push_merged_names(using.iter().cloned());
            push_level_relations(scope, left, schema);
            push_level_relations(scope, right, schema);
        }
        // A `WITH` / `WITH RECURSIVE` body carries this level's relations.
        Plan::With { body, .. } => {
            push_level_relations(scope, body, schema);
        }
        // A FROM-clause function is a named relation like any other — Postgres's
        // `chooseScalarFunctionAlias` gives it the explicit alias, else the function's
        // name — and `plan_relations` registers it that way.
        Plan::FunctionScan { alias, .. } => {
            scope.push_derived_relation(alias.clone(), &plan.output_columns(schema));
        }
        // A set-op node's output columns are not addressable by relation name at all,
        // so it is opaque: a bare name might belong here.
        Plan::SetOp { .. } => scope.push_opaque_relation(),
        // No FROM at all: `Values` and `ProjectUnnest` name no relation, and neither
        // does a bare statement node. Nothing to add, nothing to hide.
        Plan::Values { .. } | Plan::ProjectUnnest { .. } | Plan::Empty => {}
        // The `UPDATE`/`DELETE` target relation, under the name a qualified reference
        // must spell (its alias if it has one) — mirroring the scope `lower_update` /
        // `lower_delete` build — plus every relation an `UPDATE … FROM` introduces.
        Plan::Update {
            table, alias, from, ..
        } => {
            push_target_relation(scope, table, alias.as_deref(), schema);
            if let Some(source) = from {
                push_level_relations(scope, source, schema);
            }
        }
        Plan::Delete { table, .. } => push_target_relation(scope, table, None, schema),
        // Every other node is a statement, not a query level a column reference can
        // resolve against. Reaching one means this walker's model of the plan shape is
        // wrong, so register an opaque relation rather than silently claiming the level
        // is fully enumerated.
        //
        // `Plan::Insert` is here **deliberately**, though [`walk_level`] now has an arm
        // for it: that arm builds a scope per clause (`RETURNING` and the `ON CONFLICT`
        // action have different namespaces) and never reads the `here` this function
        // feeds, so nothing consumes an `INSERT`'s level scope. Measured rather than
        // assumed — giving it a `push_target_relation` arm left every answer unchanged:
        // the four refusals and two accepts in
        // `crates/pglet/tests/insert_clause_subplan_binding.rs`, the data-modifying-CTE
        // spellings of the first two, `cargo test -p pglet_sql`, and the whole fixture
        // corpus.
        Plan::Insert { .. }
        | Plan::Copy { .. }
        | Plan::CreateTable { .. }
        | Plan::DropTable { .. }
        | Plan::Truncate { .. }
        | Plan::CreateIndex { .. }
        | Plan::DropIndex { .. }
        | Plan::CreateType { .. }
        | Plan::DropType { .. }
        | Plan::DropSchema { .. }
        | Plan::CreateSchema { .. }
        | Plan::AlterTableAddColumn { .. }
        | Plan::AlterTableDropColumn { .. }
        | Plan::AlterTableAddCheck { .. }
        | Plan::AlterTableAddConstraint { .. }
        | Plan::AlterTableSetDefault { .. }
        | Plan::AlterTableSetNotNull { .. }
        | Plan::AlterTableAddIdentity { .. }
        | Plan::CreateSequence { .. }
        | Plan::AlterSequence { .. }
        | Plan::DropSequence { .. }
        | Plan::Noop { .. }
        | Plan::AlterTableUnperformed { .. }
        | Plan::Savepoint { .. }
        | Plan::RollbackToSavepoint { .. }
        | Plan::ReleaseSavepoint { .. }
        | Plan::BeginTransaction { .. }
        | Plan::CommitTransaction
        | Plan::RollbackTransaction
        | Plan::DoBlock { .. } => scope.push_opaque_relation(),
    }
}

/// Register a DML target relation under its effective name, sealing when the schema
/// has no entry for it (the columns are then unknowable, so a name must not escape to
/// an enclosing level).
fn push_target_relation(
    scope: &mut ColumnScope,
    table: &str,
    alias: Option<&str>,
    schema: &Schema,
) {
    if let Some(t) = schema.get_table(table) {
        scope.push_base_relation(
            alias.unwrap_or(table).to_string(),
            build_column_type_map(&t.columns),
        );
    } else {
        // The target's effective name is known even when the schema has no entry for
        // it; only its columns are not. See the `Plan::Scan` arm.
        scope.push_partial_relation(alias.unwrap_or(table).to_string(), HashMap::new());
    }
}
