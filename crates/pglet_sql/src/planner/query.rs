//! Query (SELECT) lowering.
//!
//! The native parser (`parser::select`) assembles the FROM/JOIN plan and the
//! per-clause IR into a [`SelectParts`] scaffold and calls [`lower_select`], which
//! is the single SELECT lowering path: WHERE type-resolution + index selection,
//! aggregate/window/projection classification, ORDER BY index pushdown, and
//! LIMIT/OFFSET.

use std::collections::{HashMap, HashSet};

use crate::catalog::{IndexKey, Schema, Table};
use crate::types::{ColumnDesc, TypeOid};
use crate::{CatalogError, Error, Result};

use super::coercion::{
    AssignmentSource, ColumnScope, OutputEntry, certain_entry_type, check_boolean_clause,
    resolve_entries_gated, resolve_expr_types_gated, validate_const_casts,
};
use super::common_type::{ResolvedType, select_common_type};
use super::correlation::{
    key_scope, resolve_correlated_subplans, scope_for_key, target_list_entries,
};
use super::dml::{
    ParameterSource, target_list_entry_mut, target_list_entry_with_input, target_list_source_types,
};
use super::expr::{BoolContext, Expr, Literal, OrderKey};
use super::function_scan::FromFunction;
#[cfg(feature = "index_acceleration")]
use super::index_selection::{
    dml_residual_is_certain, find_index_containment, find_index_equality, find_index_range,
    single_column_index_for,
};
use super::legality::comparison_legality;
use super::plan::{
    CertainTypes, EnclosingColumns, LeafScope, LimitCount, Plan, PlannerScope, ResolvedTargets,
    SetOpKind, SetOpOuterTypes, SideReferenceTypes, TypeScope, plan_relations, scan_effective_name,
};
use super::plan::{UsingMatch, using_column_match};

/// The per-clause planner IR of a `SELECT` *with* a FROM clause, before the shared
/// lowering. The native parser (`parser::select`) builds this from its parse, so
/// index selection, aggregate/window classification, ORDER BY pushdown, and
/// LIMIT/OFFSET have one implementation ([`lower_select`]). The no-FROM case
/// (`SELECT 1`) is the parser's own (it lowers to `Values`/`ProjectUnnest`, never
/// reaching here).
pub struct SelectParts {
    /// The FROM plan with any JOINs already assembled.
    pub from: Plan,
    /// The FROM tables as `(name, alias)`, in order — for column-type resolution,
    /// the single-table index-probe gate, and the ORDER BY pushdown's primary table.
    pub tables: Vec<(String, Option<String>)>,
    /// The projection expressions (a `*` wildcard is `Expr::Column { name: "*" }`).
    pub projection: Vec<Expr>,
    /// `SELECT DISTINCT` — dedup the projected rows.
    pub distinct: bool,
    /// `SELECT DISTINCT ON (e1[, e2…])` — the ON-key expressions (empty for none).
    /// Mutually exclusive with `distinct` (the grammar allows only one).
    pub distinct_on: Vec<Expr>,
    /// The WHERE predicate, converted but not yet type-resolved.
    pub selection: Option<Expr>,
    /// The GROUP BY expressions (empty for none).
    pub group_by: Vec<Expr>,
    /// The HAVING predicate.
    pub having: Option<Expr>,
    /// The ORDER BY sort keys (empty for none).
    pub order_by: Vec<OrderKey>,
    /// `Some((limit, offset))` iff a `LIMIT`/`OFFSET` clause was present; either
    /// bound may still be `None` (`LIMIT ALL`, or an `OFFSET`-only clause).
    pub limit: Option<(Option<LimitCount>, Option<LimitCount>)>,
    /// Whether the scope built for this level is its references' **whole** namespace —
    /// true exactly when no *expression* subquery encloses it (`Parser::
    /// scope_is_complete`). A level inside one sees the enclosing query's relations,
    /// which are not in this scope, so its binding verdicts have to wait for
    /// [`resolve_correlated_subplans`]'s chained re-run.
    pub scope_is_complete: bool,
}

/// Lower a parsed `SELECT` (with a FROM clause) into a [`Plan`].
///
/// Shared by both parser front-ends. Applies, in order: WHERE type-resolution +
/// index selection (`plan_filter_or_index_scan`),
/// aggregate/window/projection classification, ORDER BY index pushdown (`Sort`
/// elision), and LIMIT/OFFSET (with the elided-scan early-stop).
///
/// Finally, every **nested** query level the plan embeds — the sub-plans of an
/// `Expr::Subquery` / `Exists` / `InSubquery` — is re-resolved against a scope chained
/// with this level's relations ([`resolve_correlated_subplans`]). A correlated
/// subquery is lowered before this level's scope exists, so a literal compared against
/// an enclosing column can only get its coercing cast here; see `correlation.rs`.
///
/// # Errors
///
/// The coercion/gate errors each clause raises — a column reference no level binds
/// among them, in a closed scope — and those the correlated re-resolution raises.
pub fn lower_select(parts: SelectParts, schema: &Schema) -> Result<Plan> {
    // The chain this pass walks is complete only when `parts` is a statement's own top
    // level. An inner `SELECT` is lowered — and reaches here — *before* the query
    // containing it exists, so its enclosing relations are simply not available yet; the
    // outer level's own run is what re-walks it with the whole chain, and only that run
    // may close the nested gates.
    let chain_is_complete = parts.scope_is_complete;
    let mut plan = lower_select_level(parts, schema)?;
    resolve_correlated_subplans(&mut plan, schema, chain_is_complete)?;
    Ok(plan)
}

/// [`lower_select`] without the correlated-subquery pass — this query level alone.
fn lower_select_level(parts: SelectParts, schema: &Schema) -> Result<Plan> {
    let SelectParts {
        mut from,
        tables,
        projection,
        distinct,
        distinct_on,
        selection,
        group_by,
        having,
        order_by,
        limit,
        scope_is_complete,
    } = parts;

    // Columns of any non-base FROM relation (a CTE reference or a derived table),
    // keyed by its effective FROM name. Used to expand `alias.*` and type-resolve a
    // reference against them — and, since a key is proof of the FROM item's *kind*, to
    // decide which names the catalog must not be asked about at all.
    let nonbase = collect_nonbase_columns(&from, schema);
    // Columns of every FROM-clause function item, keyed the same way.
    let function_cols = function_scan_columns(&from, schema);

    // The relations in scope, each under the name a qualified reference must spell.
    // Drives both the coercion pass and the static comparison gate — one
    // qualifier-aware mechanism, so the cast a comparison gets and the types the gate
    // judges it by can't come from different relations (see `ColumnScope`).
    let mut scope = build_column_scope(
        &tables,
        &nonbase,
        &function_cols,
        &catalog_view_names(&from),
        schema,
    );
    // At expression-subquery depth 0 this scope is the whole namespace a reference here
    // may name, so binding failures are errors rather than unknowns. Inside one it is
    // provisional — the enclosing query's relations are not here yet — and
    // `resolve_correlated_subplans` closes the chained scope on its re-run instead.
    scope.push_merged_names(using_column_names(&from));
    if scope_is_complete {
        scope = scope.closed();
    }

    // Type-resolve the remaining clauses against that same scope. Postgres resolves an
    // unknown-typed literal inside operator resolution (`parse_oper.c`), reached from
    // the one `transformExpr` every clause goes through, so no clause can forget;
    // pglet's parser and planner are fused, so the equivalent is to run the pass at
    // each clause. `JOIN … ON`, `HAVING`, `GROUP BY`, `ORDER BY`, and the `DISTINCT ON`
    // keys used to reach the eval-time gate with their literals still untyped and
    // **over-reject** comparisons Postgres accepts (`… ON a.id = b.id AND a.id = '1'`).
    //
    // **The clause pairs move together or working queries regress.** `GROUP BY` is
    // compared to `HAVING` and to the projection by structural `==`
    // (`validate_grouped_expr`), and the `DISTINCT ON` keys are compared to the
    // `ORDER BY` keys by structural `==` (`merge_distinct_on_order_by`). Coercing one
    // side of either pair and not the other makes two spellings of the same expression
    // structurally unequal — a spurious `42803` on the grouping rule, or a spurious
    // `42P10` (or, worse, a silently wrong dedup group) on `DISTINCT ON`. Every member
    // of both pairs is therefore resolved here, in one place, against one `scope`.
    //
    // The single `scope` matters for the same reason: two separately-built scopes can
    // resolve a name two relations share to different types, reproducing the `==`
    // failure with both sides nominally coerced.
    //
    // Ordering: this runs before `validate_grouped_columns`, before the `DISTINCT ON`
    // canonicalization/merge, and before `try_order_by_index_pushdown` — whose bare-
    // `Expr::Column` gate is untouched either way, since the pass never wraps a bare
    // column (`resolve_expr_types` returns a `Column` through `map_children`, and
    // `coercion_never_wraps_a_bare_column` pins it).
    // `GROUP BY` is the one member of the set resolved **later** (just below, once the
    // projection exists) rather than here, because Postgres's
    // `findTargetlistEntrySQL92` lets a GROUP BY key name a *target-list entry* — by
    // ordinal (`GROUP BY 1`) or by output alias (`GROUP BY lbl`) — which cannot be
    // resolved before wildcard expansion and the projection's own type resolution have
    // run. It is resolved against this same `scope`, so the pairing this comment
    // describes is preserved; see `resolve_group_by_keys`.
    from = resolve_join_on_types(from, &scope, schema)?;
    // `ORDER BY` and `DISTINCT ON` are resolved further down, once the projection
    // exists — see the block above the `resolve_order_keys` call.

    // Expand `*` / `alias.*` wildcards into explicit column references against
    // the FROM tables. A sole bare `*` over a single relation is left for the
    // executor's fast path; over a **join** it is expanded here into the merged /
    // qualified star exprs, so the emitted shape is Postgres's (bare names, USING
    // columns coalesced once) rather than the join's internal dotted columns.
    let projection = expand_wildcards(projection, &from, &tables, schema, &nonbase)?;

    // The target list a `GROUP BY` **ordinal** counts against, when the projection is
    // the sole bare `*` that `expand_wildcards` deliberately leaves unexpanded for the
    // executor's fast path. Postgres resolves `GROUP BY 1` against the *expanded*
    // select list (`SELECT * FROM t GROUP BY 1` over a two-column `t` groups by the
    // first column and then rejects the second as ungrouped), so the ordinal needs the
    // expansion even though the projection keeps its `*`.
    let from_star_targets = if projection.len() == 1 && projection[0].is_bare_star() {
        Some(star_exprs(&from, schema)?)
    } else {
        None
    };
    let from_star_targets = from_star_targets.as_deref();

    // Type-resolve the projection just like the WHERE predicate: a string literal
    // compared against (or unified with) a typed column gets its coercing Cast, so
    // `SELECT COALESCE(int_col, '7')` / `SELECT id IN ('5')` match Postgres instead
    // of carrying an uncoerced text literal under an int-declared output column.
    //
    // The same pass statically gates the projection's comparisons against the declared
    // column types — Postgres rejects `SELECT text_col = 5` at parse time, so it must
    // not depend on whether a row exists to observe. The **`WHERE` under a join** is
    // gated for the same reason: with no matching row pair the WHERE is never
    // evaluated, so the eval-time gate can't be the only one (`FROM ka JOIN kb ON
    // ka.aid = kb.bk WHERE ka.aid = true` matched nothing and returned zero rows where
    // Postgres raises `42883`).
    //
    // The clause positions this pass does **not** reach are the ones with no
    // `ColumnScope` to reach them with:
    //
    // - a set-op's trailing `ORDER BY` (`plan_set_op_order_limit`, which has only
    //   output columns);
    // - an `INSERT`'s `ON CONFLICT` — its scope carries the `EXCLUDED` pseudo-relation,
    //   which the correlation chaining does not model, so `lower_insert` passes the
    //   clause through unresolved;
    // - a **sealed** level — one whose FROM introduces a relation whose columns cannot
    //   be enumerated (an `unnest`, a scan of a table absent from the schema). Name
    //   resolution stops at such a level rather than walk outward and risk binding a
    //   name to the wrong relation, so a reference there stays untyped and falls to the
    //   eval-time gate.
    //
    // A correlated `Expr::Subquery` / `Exists` / `InSubquery` *is* reached:
    // `resolve_correlated_subplans` re-runs this pass over each embedded sub-plan
    // against a scope chained with the enclosing levels. See `EDGES.md`, *Operand types — gated*.
    //
    // Each entry defines a column of the relation this level is to a query that reads
    // it — as a derived table, a `WITH` query, a set operation's side — so the pass also
    // answers the type each is certainly of, read as written (`certain_entry_type`), and
    // the projecting node below carries that record (`CertainTypes`).
    let (projection, certain) = resolve_entries_gated(projection, &scope, schema)?;

    // The primary (first) table drives ORDER BY pushdown.
    let table = tables.first().map_or("", |(t, _)| t.as_str());

    // WHERE → type-resolved filter, with a single-base-table index probe. The static
    // comparison gate runs **before** index selection, so the accelerated build (which
    // lifts the predicate into a probe and never evaluates it as an expression) and the
    // stripped build (which keeps a `Filter`) raise the identical error, in the operand
    // order the user wrote.
    let single_table = tables.len() == 1;
    if let Some(predicate) = selection {
        let predicate = resolve_expr_types_gated(predicate, &scope, schema)?;
        // The clause-root boolean gate, **above** index selection for the reason the
        // paragraph above gives: both build profiles must reach it identically, and the
        // accelerated one lifts the predicate into a probe it never evaluates.
        check_boolean_clause(&predicate, BoolContext::Where, &scope, schema)?;
        from = plan_filter_or_index_scan(from, predicate, single_table, schema);
    }

    // **`HAVING` resolves last**, below the projection and below the `WHERE`, because
    // that is where `analyze.c`'s `transformStmt` puts it: FROM, then the target list,
    // then `transformWhereClause` for the `WHERE`, then the same call for the `HAVING`.
    // Measured on 18.6 and previously divergent here in both codes:
    // `SELECT nosuchcol FROM t HAVING max(nosuch2) > 0` names `nosuchcol` there and
    // named `nosuch2` here, and `SELECT nosuchcol FROM t GROUP BY k HAVING b LIKE 'x'`
    // is `42703` there and was `42883` here. Only the *resolution* moved; nothing
    // between the old position and this one reads `having` except its own emptiness.
    let having = having
        .map(|h| resolve_expr_types_gated(h, &scope, schema))
        .transpose()?;
    if let Some(h) = &having {
        check_boolean_clause(h, BoolContext::Having, &scope, schema)?;
    }

    // An aggregated query cannot carry the unexpanded sole bare `*` through to the
    // executor — the `Aggregate` path has no star fast path, so `SELECT * FROM t GROUP
    // BY x, y` rejected the projection as `42703 column "*" does not exist` where
    // Postgres returns the rows. Expand it here, for the aggregated case only, so the
    // `Project` fast path keeps its `*`. The expansion needs no type-resolution pass:
    // it yields bare `Expr::Column` refs, which that pass returns unchanged. Keyed on
    // the *unresolved* `group_by` — only its emptiness is read, which key resolution
    // preserves.
    let (projection, certain) = match from_star_targets {
        Some(star) if !group_by.is_empty() || having.is_some() => {
            let mut star = star.to_vec();
            let certain = star
                .iter_mut()
                .map(|e| certain_entry_type(e, &scope, schema))
                .collect();
            (star, certain)
        }
        _ => (projection, certain),
    };

    // `ORDER BY` and `DISTINCT ON` keys — resolved **here**, after the projection
    // exists, because Postgres lets a key that *is* a bare single-field name in either
    // clause denote a target-list entry (`findTargetlistEntrySQL92`) rather than a FROM
    // column: `SELECT v AS lbl FROM t ORDER BY lbl`, and `SELECT * FROM a JOIN b USING
    // (num) ORDER BY num`, whose `num` is the merged output column and not either
    // side's. The claimant set that rule names is the target list, so a key resolved
    // before it exists cannot be judged at all — which is what these two calls used to
    // do, from above the wildcard expansion, against a scope whose binding gate simply
    // declined to judge *every* bare name. A key binding to nothing then survived plan
    // time, and a statement matching no row never reaches the eval-time resolver
    // either, so nothing judged it: `WITH c AS (SELECT a FROM ru ORDER BY bogus)
    // DELETE FROM rt` removed every row of `rt` where 18.6 is `42703`, and the CTE
    // being unreferenced made that row-count-independent.
    //
    // **The position is `transformSelectStmt`'s own** — FROM, target list, `WHERE`,
    // `HAVING`, **sort**, group, distinct — so a projection error and a `WHERE` one
    // both outrank a key's, and a key's outranks `GROUP BY`'s.
    // One clause on that list is deliberately *not* in its 18.6 position: window
    // definitions transform *after* the sort clause there, while pglet resolves a
    // window's own `ORDER BY` inside the projection — measured, `SELECT row_number()
    // OVER (ORDER BY bogusw) AS w FROM pt ORDER BY bogusord` is `42703` on both engines
    // naming `bogusord` there and `bogusw` here; codes agree, the reported name does not.
    // Measured on 18.6, each pair diverging before the move: `SELECT bogusproj
    // FROM ep ORDER BY bogusrel.a` is `42703`, `SELECT a FROM ep WHERE bogusw = 1
    // ORDER BY bogusrel.a` is `42703`, and `SELECT s = 5 FROM ep ORDER BY bogusrel.a`
    // is `42883` — all three `42P01` here when the keys were resolved first — while
    // `SELECT count(*) FROM ep GROUP BY bogusg ORDER BY bogusrel.a` is `42P01` on both
    // and must stay so. `ORDER BY` runs before `DISTINCT ON` for the same reason:
    // `SELECT DISTINCT ON (bogusdon) a FROM ep ORDER BY bogusord` names `bogusord`
    // there.
    //
    // The binding of each key **interleaves with its positional range check**, in
    // source order, because Postgres resolves one key at a time: `ORDER BY 9, bogus` is
    // `42P10` and `ORDER BY bogus, 9` is `42703`, and no arrangement of one whole-list
    // binding pass against one whole-list range pass yields both (the same shape
    // `plan_no_from_distinct_on` needs). `DISTINCT ON (9, bogusdon)` / `DISTINCT ON
    // (bogusdon, 9)` are the same pair one clause over.
    //
    // **The exemption is the key's own shape, not a property of every name inside
    // it**, which is `key_may_name_an_output_column`'s whole subject: SQL92's branch
    // is gated on `list_length(fields) == 1` *for the key*, so a name nested one level
    // down (`ORDER BY lbl+0`) is an ordinary column reference resolved against the
    // range table, and gating the whole clause on "is any name bare" exempted it too.
    // That let `SELECT q*10 AS z FROM tt ORDER BY z+0` answer rows where 18.6 is
    // `42703`, and `SELECT count(*) AS q FROM tt GROUP BY a ORDER BY q+0` answer rows
    // where it is `42803` — the nested name binding to the output in both.
    //
    // Both clauses are still resolved against the one `scope` the block above
    // describes, so the structural `==` pairings it protects are intact; the key scope
    // only *adds* the target-list claimants a bare key may name.
    let target_list = from_star_targets.unwrap_or(&projection);
    // Built only when there is a key to judge: the claimant set clones this level's whole
    // scope, and most statements carry neither clause.
    let keyscope = if order_by.is_empty() && distinct_on.is_empty() {
        ColumnScope::default()
    } else {
        let mut keyscope = scope.clone();
        keyscope.push_output_entries(target_list_entries(target_list, &scope));
        keyscope
    };
    let n_out = target_list.len();
    let order_by = resolve_order_keys(order_by, &scope, &keyscope, n_out, schema)?;
    let distinct_on = resolve_clause_exprs(distinct_on, &scope, &keyscope, n_out, schema)?;
    // The target-list entries the sort keys **name**, read here because this is the
    // last point they are still written as the user wrote them: the `DISTINCT ON`
    // branch below canonicalizes an ordinal to a by-name reference
    // (`resolve_output_position`) and `build_sort`'s sort-below rewrite replaces a
    // naming key with the entry's own expression, either of which loses the position.
    // See [`ResolvedTargets`].
    let out_names: Vec<String> = target_list.iter().map(Expr::output_name).collect();
    let sort_targets = ResolvedTargets::from_claims(
        order_by
            .iter()
            .map(|k| &k.expr)
            .chain(distinct_on.iter())
            .map(|e| {
                sql92_target_index(e, &out_names).or_else(|| sql99_target_index(e, target_list))
            }),
    );

    // GROUP BY keys — resolved here, after the projection exists (and is
    // wildcard-expanded and column-validated), because a key may name a target-list
    // entry by ordinal or output alias (see `resolve_group_by_keys`).
    // Substituting the **already-resolved** projection expression is what keeps the
    // clause pairing above intact: a substituted key is structurally identical to the
    // projection entry it came from by construction, so `validate_grouped_expr`'s
    // whole-expression match cannot manufacture a spurious `42803`.
    let (group_by, group_targets) = resolve_group_by_keys(group_by, &projection, &scope, schema)?;

    // GROUP BY / aggregates → Aggregate; else window functions → Window; else Project.
    let has_aggregates = projection.iter().any(contains_aggregate);
    let has_windows = projection.iter().any(contains_window);
    let certain = CertainTypes::new(certain);
    let mut plan = if !group_by.is_empty() || has_aggregates || having.is_some() {
        validate_grouped_columns(&projection, &group_by, having.as_ref(), &tables, schema)?;
        Plan::Aggregate {
            input: Box::new(from),
            group_by,
            exprs: projection,
            resolved_targets: group_targets,
            having,
            certain,
            enclosing: EnclosingColumns::default(),
        }
    } else if has_windows {
        Plan::Window {
            input: Box::new(from),
            window_exprs: projection,
            certain,
            enclosing: EnclosingColumns::default(),
        }
    } else {
        Plan::Project {
            input: Box::new(from),
            exprs: projection,
            certain,
            enclosing: EnclosingColumns::default(),
        }
    };

    // SELECT DISTINCT ON (…) — order-DEPENDENT, so it owns both the sort and the
    // dedup (unlike plain DISTINCT). Merge the ON key into the ORDER BY to get one
    // physical sort key (with equal-ON rows adjacent), build that single sort — which
    // is also the visible output order — then keep the first row of each ON group
    // above it. Mutually exclusive with `distinct`, and it consumes ORDER BY, so this
    // returns before the DISTINCT / ORDER BY paths below.
    if !distinct_on.is_empty() {
        let out_cols = plan.output_columns(schema);
        // The projection expressions (for the base-column→alias map used to canonicalize
        // an ON/ORDER BY key to its output-column identity). Cloned because a key the
        // projection cannot answer is appended to this very list as a junk column, and
        // the read has to be finished before the write.
        let projection_exprs: Vec<Expr> = match &plan {
            Plan::Project { exprs, .. } | Plan::Aggregate { exprs, .. } => exprs.clone(),
            Plan::Window { window_exprs, .. } => window_exprs.clone(),
            _ => Vec::new(),
        };
        // Canonicalize each ON expr and each ORDER BY key to its output-column identity
        // before comparing: a positional `DISTINCT ON (1)` / `ORDER BY 1`, and a column
        // reference or output alias (`emp.dept`, `dept`, an aliased `dept AS d`), all
        // collapse to the same bare column so ON and ORDER BY match the way Postgres
        // matches them (by target-entry identity, not spelling). Non-column exprs keep
        // structural equality.
        let out_names: HashSet<String> = out_cols.iter().map(|c| c.name.clone()).collect();
        let canon = |e: Expr, clause: &'static str| -> Result<Expr> {
            Ok(canonicalize_distinct_key(
                resolve_output_position(e, &out_cols, clause)?,
                &projection_exprs,
                &out_names,
                &scope,
            ))
        };
        let on_exprs = distinct_on
            .into_iter()
            .map(|e| canon(e, "DISTINCT ON"))
            .collect::<Result<Vec<_>>>()?;
        let resolved_order_by = order_by
            .into_iter()
            .map(
                |OrderKey {
                     expr,
                     asc,
                     nulls_first,
                 }| {
                    Ok(OrderKey {
                        expr: canon(expr, "ORDER BY")?,
                        asc,
                        nulls_first,
                    })
                },
            )
            .collect::<Result<Vec<_>>>()?;
        // `Plan::DistinctOn` sits **above** the projection, so every column an ON key
        // reads must be readable from the *projected* row. A reference the projection
        // carries under another name is rewritten to that name and stays exact; one it
        // does not carry at all cannot be expressed up here, so the whole key rides the
        // projecting node as a junk column instead. What it must never do is bind by
        // bare name to a same-named output column, which dedups by the wrong column
        // entirely — the measured `db.x`-answered-by-`da.x` case below.
        //
        // **The rewrite has to reach inside the key, and this used to test only a
        // whole qualified one.** A compound key (`DISTINCT ON (a+0)`, `abs(a)`,
        // `coalesce(a,0)`) is not an `Expr::Column`, so it passed the old test and
        // then read `a` off the projected row. Worse, once the sort below correctly
        // orders by the *base* column while the dedup groups by a *different*
        // projected one, equal groups are no longer adjacent and the dedup emits
        // non-adjacent duplicates: over `zz(a,b) = (1,5),(1,7),(2,5)`,
        // `SELECT DISTINCT ON (a+0) b AS a FROM zz ORDER BY a+0, b` answered `5 7 5`,
        // which is not a legal `DISTINCT ON` result of anything (18.6: `5 5`).
        //
        // The rewrite runs **after** the merge, never before it.
        // `merge_distinct_on_order_by` is Postgres's `transformDistinctOnClause` scan
        // and compares ON keys to ORDER BY keys by structural equality, so re-spelling
        // one side first makes a matching pair look mismatched — the `42P10` fires on
        // a legal statement, or the merge appends a second sort key and the dedup's
        // groups stop being adjacent again.
        // Resolve each ON key against the projection **before** comparing the list to
        // the ORDER BY keys, which is `transformDistinctOnClause`'s own order: it runs
        // `findTargetlistEntrySQL92` over every expression and only then looks for the
        // match. Measured on 18.6, `SELECT DISTINCT ON (bogus) v FROM g ORDER BY v` is
        // `42703` and was `42P10` here.
        let projected: Vec<Option<Expr>> = on_exprs
            .iter()
            .map(|e| project_distinct_on_key(e.clone(), &projection_exprs, &out_names, &scope))
            .collect();
        // A key the projection cannot answer becomes a junk column below — evaluated
        // only when a row is, so a key binding to **nothing** would be judged by nobody
        // on a statement matching none. `project_distinct_on_key`'s `None` used to be an
        // unconditional refusal and doubled as this check; minting instead made
        // `SELECT DISTINCT ON (bogus) v FROM g WHERE 1=0 ORDER BY bogus` answer
        // `(0 rows)` where 18.6 is `42703` — and the same statement *with* rows is
        // `42703` here either way, which is what identifies the hole as the no-row one.
        for (key, projected) in on_exprs.iter().zip(&projected) {
            if projected.is_none() {
                check_key_bindings(key, &scope)?;
            }
        }
        let merged = merge_distinct_on_order_by(&on_exprs, &resolved_order_by)?;
        // A key the projection cannot answer becomes a junk column on the projecting
        // node, exactly as an unanswerable `ORDER BY` key does — Postgres's
        // `transformDistinctOnClause` reaches `addTargetToSortList` for both. The three
        // arms below are the nodes `plan_select` builds at the top; a shape with no
        // expression list of its own keeps the junk-free refusal rather than having the
        // columns dropped silently, and so does a projection carrying an unexpanded
        // bare `*`, whose fast path in the executor reads the input's columns and would
        // drop an appended junk expression on the floor.
        //
        // **That second exclusion used to be justified by "which cannot need one anyway,
        // since it carries every FROM column", and that was false**: the reason a key
        // reaches here is `output_key_identity`'s name match against *projection
        // entries*, and a bare `*` is an `Expr::Column { name: "*" }` that matched none
        // of them. So `SELECT DISTINCT ON (zz.a) * FROM zz ORDER BY zz.a, b DESC` and
        // `SELECT DISTINCT ON (a+0) * …` were `0A000` here against three rows on 18.6.
        // The star now denotes every column the scope carries
        // (`output_key_identity`'s star arm), so both resolve with no junk column at
        // all, and the binding gate above turns a key naming nothing into `42703`.
        //
        // What is left reaching this arm is narrower and is a **residual, not nothing**.
        // State it as the rule rather than as a list: the star arm answers exactly what
        // this level's `ColumnScope` **enumerates**, and `build_column_scope` registers
        // some kinds of relation non-exhaustively — so a key naming a column such a
        // relation carries but the scope does not list is `Uncertain` at the gate and
        // lands here as `0A000`. The members measured are loud and pre-existing:
        //
        // - a **catalog view** column pglet does not model — `SELECT DISTINCT ON
        //   (c.udt_name) * FROM information_schema.columns c ORDER BY c.udt_name`, rows
        //   on 18.6;
        // - (a FROM-clause function item is not one: it registers its columns
        //   exhaustively, and `SELECT DISTINCT ON (u.u) * FROM unnest(ARRAY[1,2,2]) AS u
        //   ORDER BY u.u` answers 18.6's two rows, measured);
        // - a **relation the catalog lacks**, registered the same empty way, so pglet's
        //   own `42P01` for the missing table never gets to run — `SELECT DISTINCT ON
        //   (nosuch) * FROM no_such_tbl ORDER BY nosuch` is `0A000` against 18.6's
        //   `42P01`, where the non-star spelling is `42P01` here too.
        //
        // Recorded in `EDGES.md`.
        let can_carry_junk = matches!(
            plan,
            Plan::Project { .. } | Plan::Aggregate { .. } | Plan::Window { .. }
        ) && !projection_exprs.iter().any(Expr::is_bare_star);
        let mut on_junk: Vec<Expr> = Vec::new();
        let mut taken = out_names.clone();
        taken.extend(clause_key_names(
            on_exprs
                .iter()
                .cloned()
                .chain(merged.iter().map(|k| k.expr.clone())),
        ));
        let on_exprs = on_exprs
            .into_iter()
            .zip(projected)
            .map(|(e, projected)| match projected {
                Some(projected) => Ok(projected),
                None if can_carry_junk => Ok(mint_junk_column(e, &mut on_junk, &mut taken)),
                None => Err(distinct_on_key_dropped(&e)),
            })
            .collect::<Result<Vec<_>>>()?;
        let real_width = out_cols.len();
        let carried_on_junk = !on_junk.is_empty();
        push_junk_columns(&mut plan, on_junk);
        // The single sort (placed above/below the projection by `build_sort` so a key
        // on a dropped base column is still honored); DistinctOn keeps first-per-group.
        // The dedup reads its ON key above the Sort, so the junk trim is placed above
        // the dedup rather than under it. Both minters append, so the query's real
        // output is the leading `real_width` columns whichever of them ran.
        let sorted = build_sort(plan, merged, &scope, &sort_targets)?;
        let junk_keep = (carried_on_junk || sorted.junk_keep.is_some()).then_some(real_width);
        plan = SortedPlan {
            junk_keep,
            ..sorted
        }
        .map(|sorted| Plan::DistinctOn {
            on_exprs,
            input: Box::new(sorted),
        })
        .trimmed();
        if let Some((limit_val, offset_val)) = limit {
            plan = Plan::Limit {
                input: Box::new(plan),
                limit: limit_val,
                offset: offset_val,
            };
        }
        return Ok(plan);
    }

    // SELECT DISTINCT — dedup the projected rows before ORDER BY/LIMIT.
    //
    // Postgres requires every ORDER BY expression of a `SELECT DISTINCT` to appear in
    // the select list (`transformDistinctClause`, `42P10`): the dedup collapses rows,
    // so a key outside the output has no well-defined value to sort the survivors by.
    // The check is by target-entry identity, not spelling — a qualified `c.x` over a
    // projected `x`, an output alias, and the select-list expression itself all pass —
    // and rewriting each key to that identity is what keeps the Sort above the
    // `Distinct` resolvable against its bare output names.
    let order_by = if distinct {
        let out_cols = plan.output_columns(schema);
        let projection_exprs: &[Expr] = match &plan {
            Plan::Project { exprs, .. } | Plan::Aggregate { exprs, .. } => exprs,
            Plan::Window { window_exprs, .. } => window_exprs,
            _ => &[],
        };
        let out_names: HashSet<String> = out_cols.iter().map(|c| c.name.clone()).collect();
        let keys = order_by
            .into_iter()
            .map(|key| {
                if positional_key_index(&key.expr, out_cols.len(), "ORDER BY")?.is_some() {
                    return Ok(key);
                }
                let expr =
                    output_key_identity(&key.expr, projection_exprs, &out_names, &scope, true)
                        .ok_or(Error::DistinctOrderByNotInSelectList)?;
                Ok(OrderKey { expr, ..key })
            })
            .collect::<Result<Vec<_>>>()?;
        plan = Plan::Distinct {
            input: Box::new(plan),
        };
        keys
    } else {
        order_by
    };

    // ORDER BY — satisfy from a single-column index when possible (eliding the Sort).
    let mut sort_elided = false;
    if !order_by.is_empty() {
        // A positional `ORDER BY n` names the Nth **output** column, so validate its
        // range against the output column count here — independent of whether the
        // Sort lands above or below the projection (`build_sort`). Out of range fails
        // loud `42P10` (Postgres), and it lets `build_sort`'s sort-below rewrite index
        // the projection safely without re-checking.
        let out_count = plan.output_columns(schema).len();
        for key in &order_by {
            positional_key_index(&key.expr, out_count, "ORDER BY")?;
        }
        // The pushdown rewrites the scan to walk a **base column**'s index, so it may
        // only fire on a key that binds that base column. Its own gate 2a tests the
        // key's *shape* (a bare `Expr::Column`) and never its binding, which is the
        // same output-name question `build_sort` answers below — and answering it in
        // two places let the pushdown answer first and differently. Over
        // `zz(a NOT NULL indexed, b)`, `SELECT b AS a FROM zz ORDER BY a` walked the
        // index on base `a` where 18.6 sorts by the output column (the projected `b`),
        // and the same statement on an unindexed table was correct — so it was also a
        // build-profile split, the stripped planner never pushing down at all.
        let pushdown_ok = order_by
            .first()
            .is_some_and(|k| order_key_binds_a_base_column(&k.expr, &plan, schema));
        let (rewritten, elided) = if pushdown_ok {
            try_order_by_index_pushdown(plan, &order_by, table, schema)
        } else {
            (plan, false)
        };
        if elided {
            plan = rewritten;
            sort_elided = true;
        } else {
            plan = build_sort(rewritten, order_by, &scope, &sort_targets)?.trimmed();
        }
    }

    // LIMIT/OFFSET — push the count into an elided ordered scan when there's no OFFSET.
    if let Some((limit_val, offset_val)) = limit {
        if sort_elided
            && offset_val.is_none()
            && let Some(count) = limit_val
        {
            set_index_scan_limit(&mut plan, count);
        }
        plan = Plan::Limit {
            input: Box::new(plan),
            limit: limit_val,
            offset: offset_val,
        };
    }

    Ok(plan)
}

/// Resolve the `GROUP BY` key list, implementing Postgres's
/// `findTargetlistEntrySQL92` (`parse_clause.c`) for the two spellings that name a
/// *target-list entry* rather than a FROM column, then type-resolving the rest against
/// `scope` exactly as [`resolve_clause_exprs`] does for the other clauses.
///
/// `projection` is the wildcard-expanded, already-type-resolved select list — the
/// target list both the ordinal and the alias rules count against.
///
/// Three key shapes, in Postgres's own order:
///
/// - A **bare integer constant** is a 1-based ordinal into the target list, resolving
///   to that entry's *expression* (alias peeled) — so `SELECT x+1 AS e … GROUP BY 1`
///   groups by `x+1`, not by the output name `e`. Substituting the expression (rather
///   than a by-name `Expr::Column`, which is what the `ORDER BY` / `DISTINCT ON`
///   [`resolve_output_position`] builds) is what makes `validate_grouped_expr`'s
///   whole-expression match succeed, and mirrors PG resolving the ordinal to the
///   `TargetEntry` itself. Out of range → `42P10`.
/// - Any **other bare constant** (`'str'`, `1.5`, `true`, `NULL`) → `42601`
///   `non-integer constant in GROUP BY`. These previously grouped every row into one
///   silent group.
/// - A **bare single identifier** takes the branch that makes GROUP BY differ from
///   `ORDER BY` / `DISTINCT ON`: a FROM-clause column **wins over** a same-named output
///   alias. PG's comment is explicit — *"In GROUP BY, we must prefer a match against a
///   FROM-clause column to one against the targetlist"* — and `ORDER BY` /
///   `DISTINCT ON` have no such precheck, so their precedence is the **inverse**
///   (`SELECT x AS y … GROUP BY y` groups by `t.y`, while `… ORDER BY y` orders by
///   `x`). Only when no FROM column claims the name does the alias search run, and a
///   name carried by two target entries with **different** expressions is `42702` —
///   see [`target_entry_by_alias`].
///
/// Anything else (`GROUP BY x+1`, `GROUP BY lower(a)`) is not a bare constant or
/// identifier and never reaches the alias/ordinal rules — PG routes it to
/// `findTargetlistEntrySQL99`, which resolves it as an ordinary expression. The
/// routing is shared; the resulting *error* is not. `GROUP BY lbl+0` over an alias
/// `lbl` is `42703 column "lbl" does not exist` on Postgres (SQL99 resolution reaches
/// only FROM columns, and `lbl` is not one) but `42803` on pglet, whose SQL99 path
/// leaves the unresolved name to the grouping validator rather than a column-existence
/// check. A **divergence**, loud on both sides.
///
/// A matched target entry containing an **aggregate** is rejected (`42803`,
/// `aggregate functions are not allowed in GROUP BY`) and one containing a **window
/// function** is rejected `42P20` (`window functions are not allowed in GROUP BY`),
/// mirroring `checkTargetlistEntrySQL92`, which tests the two separately and in that
/// order. Both apply to the ordinal and the alias alike, so `SELECT count(*) AS c …
/// GROUP BY c`, `SELECT x, count(*) … GROUP BY 2`, and `SELECT row_number() OVER () AS
/// w … GROUP BY w` all fail.
///
/// # Errors
///
/// `42P10` (ordinal out of range), `42601` (non-integer constant), `42702` (an
/// ambiguous output alias), `42803` (the matched entry is an aggregate), `42P20` (the
/// matched entry is a window function), plus whatever [`resolve_clause_exprs`]
/// propagates for an ordinary key.
fn resolve_group_by_keys(
    group_by: Vec<Expr>,
    projection: &[Expr],
    scope: &ColumnScope,
    schema: &Schema,
) -> Result<(Vec<Expr>, ResolvedTargets)> {
    if group_by.is_empty() {
        return Ok((group_by, ResolvedTargets::default()));
    }
    // The **position** each key named is kept beside the substituted entry, because the
    // substitution loses it: two entries can be equal, and only the position says which
    // one `GROUP BY 1` claimed. See [`ResolvedTargets`].
    let mut claims = Vec::with_capacity(group_by.len());
    let keys = group_by
        .into_iter()
        .map(|key| {
            let (target, index) = group_by_target_entry(&key, projection, scope)?;
            let Some(target) = target else {
                // An ordinary key: `findTargetlistEntrySQL99` still claims the first entry
                // it equals — see [`sql99_target_index`].
                let key = resolve_expr_types_gated(key, scope, schema)?;
                claims.push(sql99_target_index(&key, projection));
                return Ok(key);
            };
            claims.push(index);
            Ok(target)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((keys, ResolvedTargets::from_claims(claims)))
}

/// The target-list entry a `GROUP BY` key names **and its position**, or `(None, None)`
/// when the key is an ordinary expression to be resolved against the FROM columns. See
/// [`resolve_group_by_keys`].
///
/// The two are returned together because the entry alone cannot answer *which* entry it
/// was — `SELECT 't', 't' … GROUP BY 1` substitutes an expression both entries carry
/// (see [`ResolvedTargets`]).
///
/// # Errors
///
/// `42P10` / `42601` / `42702` / `42803` / `42P20` as described on
/// [`resolve_group_by_keys`].
fn group_by_target_entry(
    key: &Expr,
    targets: &[Expr],
    scope: &ColumnScope,
) -> Result<(Option<Expr>, Option<usize>)> {
    let index = match key {
        // A bare constant: an integer is an ordinal, anything else is `42601`. Shared
        // with the `ORDER BY` / `DISTINCT ON` rule, which is identical for both — only
        // the clause label in the message differs. Note `+1` is **not** an
        // `Expr::Literal` (see `Expr::UnaryPlus`), so it never reaches here — matching
        // Postgres, where only unary minus folds into a constant.
        Expr::Literal(_) => positional_key_index(key, targets.len(), "GROUP BY")?,
        // A bare single identifier — the FROM-column-first branch. A qualified
        // reference (`GROUP BY t.lbl`) never matches an alias: PG's precheck is gated
        // on `list_length(fields) == 1`, so a qualified name that no relation carries
        // is `42703` rather than an alias hit.
        Expr::Column { table: None, name } => {
            if scope.contains_column_ref(None, name) {
                // A FROM column claims the name — the alias search is skipped
                // entirely, and an ambiguous name resolves (or raises `42702`) as an
                // ordinary column reference would.
                return Ok((None, None));
            }
            target_entry_by_alias(targets, name, scope)?
        }
        _ => None,
    };
    let Some(index) = index else {
        return Ok((None, None));
    };
    let target = strip_alias(targets[index].clone());
    // Postgres's `checkTargetlistEntrySQL92` tests aggregates first, then window
    // functions, with **different** SQLSTATEs. Keep both the split and the order: a
    // matched entry that is an aggregate is `42803`, one that is a window function is
    // `42P20`.
    if contains_aggregate(&target) {
        return Err(Error::AggregateNotAllowed {
            context: "GROUP BY",
        });
    }
    if contains_window(&target) {
        return Err(Error::WindowingError {
            context: "GROUP BY",
        });
    }
    Ok((Some(target), Some(index)))
}

/// The index of the target-list entry whose **output name** is `name`, implementing
/// the ambiguity half of Postgres's `findTargetlistEntrySQL92` alias loop.
///
/// Postgres does not stop at the first match: it scans the whole target list and, on
/// a second entry carrying the same output name, raises `42702` *unless the two
/// expressions are equal* — `equal(target_result->expr, tle->expr)`. So
/// `SELECT x AS a, x AS a … GROUP BY a` is accepted (both entries denote `x`) while
/// `SELECT x AS a, count(*) AS a … GROUP BY a` is `GROUP BY "a" is ambiguous`.
/// Comparing *expressions* rather than counting matches is the whole point; a
/// first-match `position()` silently grouped by the first entry and returned rows
/// Postgres refuses.
///
/// Each candidate is compared with its alias peeled, since the alias is the name being
/// matched on, not part of the expression's identity (Postgres's `TargetEntry` keeps
/// `resname` beside `expr` for the same reason). Later matches are compared against
/// the **first** match, mirroring PG's retained `target_result`.
///
/// Scoped to `GROUP BY`. `ORDER BY` and `DISTINCT ON` run the identical loop in
/// Postgres, but pglet resolves those through [`resolve_output_position`] /
/// [`canonicalize_distinct_key`], whose behavior on a duplicated alias is a separate
/// question from this one and is not changed here.
///
/// A **no-op cast is stripped** before the comparison. Postgres compares parse nodes,
/// and its `coerce_type` has already elided any cast whose target equals the operand's
/// existing type — so `SELECT x AS a, x::integer AS a` (over an `integer` x) presents
/// two identical `Var`s and is accepted. pglet compares post-coercion planner `Expr`s,
/// where the redundant `Expr::Cast` survives; without the strip it read as a genuine
/// difference and raised `42702` on a query Postgres answers. The strip is
/// type-directed and can only remove a cast that changes nothing, so a *widening* cast
/// (`x::bigint`) still differs and still raises `42702`. See [`strip_noop_casts`].
///
/// # Errors
///
/// `42702` when two entries share `name` but denote different expressions.
fn target_entry_by_alias(
    targets: &[Expr],
    name: &str,
    scope: &ColumnScope,
) -> Result<Option<usize>> {
    let mut found: Option<(usize, Expr)> = None;
    for (i, target) in targets.iter().enumerate() {
        if target.output_name() != name {
            continue;
        }
        let denoted = strip_noop_casts(peel_alias(target).clone(), scope)?;
        match &found {
            None => found = Some((i, denoted)),
            Some((_, first)) => {
                if *first != denoted {
                    return Err(Error::AmbiguousClauseKey {
                        clause: "GROUP BY",
                        name: name.to_string(),
                    });
                }
            }
        }
    }
    Ok(found.map(|(i, _)| i))
}

/// Remove every cast whose target type is the operand's type already — the planner-`Expr`
/// counterpart of the elision Postgres's `coerce_type` performs at parse time.
///
/// Applied at **every depth** (via `map_children`), because Postgres elides at every
/// depth: `x::integer + 1` and `x + 1` are the same parse tree there.
///
/// Type resolution is deliberately narrow — a column looked up in `scope`, or a cast's
/// own declared target. Anything else yields `None` and the cast is **kept**, so an
/// unknown type can only ever preserve the current (stricter) verdict. `scope`'s
/// resolver declines an ambiguous bare name for the same reason.
///
/// [`Expr::CastTypmod`] is never stripped: a type modifier is not a no-op, and Postgres
/// keeps that node too (`x::varchar(7)` is a real coercion, not a relabel).
fn strip_noop_casts(expr: Expr, scope: &ColumnScope) -> Result<Expr> {
    crate::depth::check_stack()?;
    let expr = match expr {
        Expr::Cast {
            expr: inner,
            target_type,
        } if scope_expr_type(&inner, scope) == Some(target_type) => *inner,
        other => other,
    };
    expr.try_map_children(|child| strip_noop_casts(child, scope))
}

/// The type of an expression for [`strip_noop_casts`]'s no-op test, or `None` when it
/// cannot be established with certainty from `scope` alone.
fn scope_expr_type(expr: &Expr, scope: &ColumnScope) -> Option<TypeOid> {
    match expr {
        Expr::Column { table, name } => scope.certain_column_type(table.as_deref(), name),
        Expr::Cast { target_type, .. } => Some(*target_type),
        _ => None,
    }
}

/// The expression an output-list entry denotes, with any `AS` alias peeled — the
/// borrowing counterpart of [`strip_alias`].
fn peel_alias(expr: &Expr) -> &Expr {
    match expr {
        Expr::Alias { expr: inner, .. } => inner,
        other => other,
    }
}

/// Type-resolve a clause's expressions against `scope` — the `DISTINCT ON` key list.
///
/// # Errors
///
/// Propagates [`resolve_expr_types_gated`]: an unparseable constant cast, or a
/// comparison with no operator for its operand types (`42883`).
fn resolve_clause_exprs(
    exprs: Vec<Expr>,
    scope: &ColumnScope,
    keyscope: &ColumnScope,
    n_out: usize,
    schema: &Schema,
) -> Result<Vec<Expr>> {
    exprs
        .into_iter()
        .map(|e| resolve_sort_key(e, scope, keyscope, n_out, "DISTINCT ON", schema))
        .collect()
}

/// Type-resolve one `ORDER BY` / `DISTINCT ON` key against whichever of the two
/// scopes Postgres's SQL92 branch applies to it.
///
/// The shape of the test is the whole content: `findTargetlistEntrySQL92` is entered
/// only for a key that **is** a one-field `ColumnRef`, so the output-name exemption
/// belongs to the key, never to the names inside it. A key of any other shape —
/// `lbl+0`, `-lbl`, `lower(lbl)` — is an ordinary expression whose every leaf
/// `transformExpr` resolves against the range table, so it gates like any other
/// clause.
///
/// **This is not the only site that decides it, and saying it were would be false.**
/// The same shape question is also spelled by [`whole_key_output_name`] (the same rule
/// one phase later, where the output names are finally known), by
/// [`output_key_identity`]'s `whole_key` parameter, by `group_by_target_entry`'s
/// whole-key precheck, and by [`order_key_binds_a_base_column`] — which exists because
/// `try_order_by_index_pushdown` answered the question by not asking it, and so
/// silently outranked every one of the others whenever the key column had an index.
/// They are not linked in code, and no count of them is kept here because a count is
/// the form of this that goes stale without reddening anything: a change to the rule
/// is a change to all of them or it is a drift.
///
/// `keyscope` is `scope` plus this level's target-list entries as an explicit claimant
/// set (`super::correlation::target_list_entries`), so the SQL92 branch answers from the
/// output names and falls through to the range table — never a blanket decline.
///
/// The positional range check runs **first, per key**, because Postgres resolves one key
/// at a time and the two verdicts interleave in source order: `ORDER BY 9, bogus` is
/// `42P10` and `ORDER BY bogus, 9` is `42703` on 18.6.
///
/// # Errors
///
/// `42P10` (an ordinal outside the target list), `42601` (a non-integer constant key),
/// plus whatever [`resolve_expr_types_gated`] propagates — including the `42703` /
/// `42P01` / `42702` of a key binding to nothing, to no relation, or to two claimants.
fn resolve_sort_key(
    expr: Expr,
    scope: &ColumnScope,
    keyscope: &ColumnScope,
    n_out: usize,
    clause: &'static str,
    schema: &Schema,
) -> Result<Expr> {
    positional_key_index(&expr, n_out, clause)?;
    let scope = scope_for_key(&expr, scope, keyscope);
    resolve_expr_types_gated(expr, scope, schema)
}

/// Whether a sort/group key is the shape Postgres's SQL92 rule reaches — a key that
/// is *entirely* an unqualified name, which may therefore denote a target-list entry
/// rather than a FROM column.
///
/// `findTargetlistEntrySQL92` gates on
/// `IsA(node, ColumnRef) && list_length(fields) == 1`, so both a **qualifier**
/// (`t.lbl`) and any **nesting** (`lbl+0`) take the key out of the rule. Callers use
/// it to decide whether a key's names may go unjudged by the FROM-relation scope.
///
/// [`whole_key_output_name`] asks the same question one phase later, once the output
/// names exist, and additionally requires the key to *match* one. It does **not** call
/// this — the two run at different phases with different information, so they are two
/// spellings that must be changed together rather than one shared predicate.
///
/// `super::correlation::key_scope` *does* call this, and must: it re-asks the same
/// question at the same phase for a **nested** level, and a second copy there answered
/// it wrong in the direction a copy always does — over `tt(q, a, b)`,
/// `SELECT (SELECT q*10 AS z FROM tt ORDER BY z+0 LIMIT 1)` answered `10` where 18.6 is
/// `42703`, while the identical key at top level was already refused here.
pub(super) const fn key_may_name_an_output_column(expr: &Expr) -> bool {
    matches!(expr, Expr::Column { table: None, .. })
}

/// Type-resolve each `ORDER BY` key's expression, carrying its direction and NULL
/// placement through untouched.
///
/// A key that is a bare constant — the positional `ORDER BY 2`, or the `ORDER BY
/// 'lit'` Postgres rejects `42601` — has no comparison to resolve, so it passes
/// through unchanged and [`positional_key_index`] still sees the literal it needs to
/// apply the SQL92 ordinal rule. (Postgres reaches that rule *before* type
/// resolution, so a bare string constant must never be resolved to `text` and
/// accepted here.)
///
/// # Errors
///
/// Propagates [`resolve_expr_types_gated`].
fn resolve_order_keys(
    order_by: Vec<OrderKey>,
    scope: &ColumnScope,
    keyscope: &ColumnScope,
    n_out: usize,
    schema: &Schema,
) -> Result<Vec<OrderKey>> {
    order_by
        .into_iter()
        .map(
            |OrderKey {
                 expr,
                 asc,
                 nulls_first,
             }| {
                Ok(OrderKey {
                    expr: resolve_sort_key(expr, scope, keyscope, n_out, "ORDER BY", schema)?,
                    asc,
                    nulls_first,
                })
            },
        )
        .collect()
}

/// Type-resolve every `JOIN … ON` condition in an already-assembled FROM tree.
///
/// Recurses through [`Plan::Join`] arms **only**. Every other FROM leaf either has no
/// ON condition (`Scan`, `CteScan`, `VirtualScan`) or owns a self-contained sub-plan
/// that was lowered against its own scope (`SubqueryScan`) — descending into the
/// latter would re-resolve its expressions against the *outer* relations, which is
/// exactly the confusion [`ColumnScope`] exists to prevent.
///
/// The `scope` is the whole FROM clause's, so a nested join's ON sees every relation
/// rather than only its own chain — slightly more permissive than Postgres. That
/// costs nothing: coercion only *inserts a cast*, and a nested ON that names a
/// relation Postgres would not have in scope is rejected by column resolution
/// downstream regardless.
///
/// The recursion is **depth-first, arms before this node's own `ON`**, because
/// `transformFromClauseItem` transforms `j->larg` and `j->rarg` before it reaches the
/// join's quals: over `q1 JOIN q2 ON q1.nosuch JOIN q3 ON q2.k` the verdict 18.6
/// reports is the inner `ON`'s `42703`, not the outer `ON`'s `42804`. Resolving this
/// node's `on` first reported the enclosing join's verdict for an enclosed join's
/// error, which the `nested_join_on_*` fixtures under `tests/fixtures/bool_context/`
/// hold wherever the two verdicts carry different SQLSTATEs. Where they do not — both
/// `ON`s failing the gate, so only the operand named in the message separates them —
/// the fixture beside them holds the agreed SQLSTATE and the discriminating assertion is
/// `boolean_context_verdicts.rs`'s.
///
/// # Errors
///
/// Propagates [`resolve_expr_types_gated`].
fn resolve_join_on_types(mut plan: Plan, scope: &ColumnScope, schema: &Schema) -> Result<Plan> {
    resolve_join_on_types_in_place(&mut plan, scope, schema)?;
    Ok(plan)
}

/// [`resolve_join_on_types`] in place: a `JOIN` chain is walked from this frame, and
/// each `ON` resolved in one of its own, so a chain costs this frame a level
/// (`crate::depth`).
fn resolve_join_on_types_in_place(
    plan: &mut Plan,
    scope: &ColumnScope,
    schema: &Schema,
) -> Result<()> {
    crate::depth::check_stack()?;
    let Plan::Join {
        left, right, on, ..
    } = plan
    else {
        return Ok(());
    };
    resolve_join_on_types_in_place(left, scope, schema)?;
    resolve_join_on_types_in_place(right, scope, schema)?;
    resolve_join_on_finish(on, scope, schema)
}

/// One join's `ON`, resolved and gated.
#[inline(never)]
fn resolve_join_on_finish(on: &mut Expr, scope: &ColumnScope, schema: &Schema) -> Result<()> {
    let taken = std::mem::replace(on, Expr::Default);
    let resolved = resolve_expr_types_gated(taken, scope, schema)?;
    // `JOIN/ON` is the one clause-root boolean gate that **outranks the target list**:
    // `transformFromClause` runs before `transformTargetList`, so
    // `SELECT nosuchcol FROM a JOIN c ON a.k` is 18.6's `42804 argument of JOIN/ON`
    // and the `ON a.b` spelling its `42703` — measured, both engines agreeing on the
    // second. That is why this walk already sits above the projection, and the gate
    // rides it rather than earning a position of its own.
    //
    // A `CROSS JOIN` / comma join carries a synthesized `Expr::Literal(Bool(true))`
    // (`parser::select`), which is `boolean` and passes — a root the parse-site
    // predicate cannot see, judged here by its type like every other.
    check_boolean_clause(&resolved, BoolContext::JoinOn, scope, schema)?;
    *on = resolved;
    Ok(())
}

/// Build a [`Plan::SetOp`] combining two already-lowered sides, enforcing
/// Postgres's column-count and type-compatibility rules and recording the unified
/// per-position type the executor casts both sides to.
///
/// The sides must have the same number of output columns (else a loud `42601`,
/// `each UNION query must have the same number of columns`), and each column pair
/// must have a common type per [`select_common_type`] — the same rule
/// `CASE`/`COALESCE`/`GREATEST` use (else a loud `42804`, the datatype mismatch).
///
/// # An `unknown` entry yields to the other side
///
/// This is 18.6's `constructSetOpTargetlist`. A side's output column is `unknown` when it
/// is a bare string constant, a bare `NULL` or a bare `$N` the client did not declare,
/// seen through an `AS alias` and through the wrappers that resolve nothing — decided
/// by [`target_list_source_types`], the walk `INSERT … SELECT` asks the same question
/// with, because a sort, grouping, `DISTINCT` or window key naming the entry resolves it
/// to `text` first on both routes (`SELECT 1 UNION (SELECT '2' ORDER BY 1)` is `42804`,
/// `SELECT 1 UNION (SELECT '2' FROM t ORDER BY a)` is `integer`, measured). A key names
/// an entry by position or output name, or by being the same expression
/// ([`sql99_target_index`]) — how a window key and a `$N` sort key do. An `unknown`
/// input contributes no type to [`select_common_type`], so it takes the other side's,
/// and a column that is `unknown` on both sides is `text`. A nested set operation is
/// never `unknown` — its own column was resolved when it was built, which is why
/// `SELECT '1' UNION SELECT '2' UNION SELECT 3` is `42804` on both engines while
/// `SELECT 3 UNION SELECT '1' UNION SELECT '2'` is `integer`. It can still be **pending**,
/// though, and then it contributes no type either: a side whose own column waits on an
/// enclosing level leaves this column waiting too ([`set_op_pending_beneath`]), so
/// `(SELECT i EXCEPT SELECT '8') INTERSECT SELECT '07'` over an outer `integer` `i`
/// converts `'07'` to `7` rather than unifying against the side's provisional type.
///
/// Each `unknown` entry is then **rewritten** into a `Cast` to the column's type, left
/// side before right, column by column, and a constant one is converted here —
/// `constructSetOpTargetlist` replaces the `Const` / `Param` with the coerced
/// expression during parse analysis. That is what makes the conversion's error a
/// parse-analysis one: `SELECT 1 UNION SELECT 'x' FROM t WHERE false` and
/// `SELECT 1/0 UNION SELECT 'x'` are both `22P02` on 18.6, and `SELECT 1, 'x' UNION
/// SELECT 'y', 2` names `"y"` — the first column's conversion ahead of the second
/// column's. The rewrite is also what types an undeclared `$N` for inference and for
/// both declared-type walkers. The entry keeps its output name through an `Alias`.
///
/// # A side naming an enclosing query level
///
/// A side's entry that references a column of an **enclosing** level carries that
/// column's type into the unification, as 18.6's `Var` does: `SELECT i EXCEPT SELECT
/// '7'` inside a correlated subquery over an `integer` `i` is an `integer` column there.
/// Built inside an expression subquery — or inside a FROM item such a subquery encloses —
/// the node cannot see those levels yet, so such a column is left pending
/// ([`SetOpTyping::Deferring`]) and unified by the correlated re-walk. Its errors are
/// therefore raised there, after the enclosing level's own clauses have been lowered,
/// where 18.6 raises them while analyzing the subquery.
///
/// A pending column's provisional type is read by whatever asks the node for its columns
/// before that, so the re-walk's answer is not the end of it: the reference types it
/// unified with are recorded ([`SetOpOuterTypes`]) and the statement is parsed a second
/// time, reaching this function with [`SetOpTyping::Resolved`] — complete, nothing
/// pending, no provisional type for any reader of the returned plan to take.
/// `docs/scope.md` § A pending set-op column's type is never read.
///
/// The unified types ride on the node ([`Plan::SetOp::column_types`]); the executor
/// casts each side's values to them **positionally**, so a side with repeated /
/// unnamed column names can't collapse under by-name resolution.
///
/// # Errors
///
/// `42601` on a column-count mismatch; `42804` / `42846` when a column pair has no
/// common type; the conversion's own error (`22P02`, `22003`, `22007`, …) for a constant
/// the column's type refuses — each for a column that is not left pending.
pub fn build_set_op(
    op: SetOpKind,
    all: bool,
    mut left: Plan,
    mut right: Plan,
    schema: &Schema,
    typing: SetOpTyping<'_>,
) -> Result<Plan> {
    let width = left.output_columns(schema).len();
    if width != right.output_columns(schema).len() {
        return Err(Error::syntax(format!(
            "each {} query must have the same number of columns",
            op.keyword()
        )));
    }
    let typing = match typing {
        SetOpTyping::Complete => OuterTyping::Complete { references: None },
        SetOpTyping::Deferring => OuterTyping::Deferring,
        SetOpTyping::Resolved(references) => OuterTyping::Complete {
            references: Some(references),
        },
    };
    let mut column_types = vec![TypeOid::Text; width];
    let columns: Vec<usize> = (0..width).collect();
    let unified = unify_set_op_columns(
        SetOpColumns {
            op,
            left: &mut left,
            right: &mut right,
            column_types: &mut column_types,
            columns: &columns,
        },
        schema,
        &typing,
    )?;
    let outer_types =
        (!unified.pending.is_empty()).then(|| SetOpOuterTypes::new(width, &unified.pending));
    Ok(Plan::SetOp {
        op,
        all,
        left: Box::new(left),
        right: Box::new(right),
        column_types,
        outer_pending: unified.pending,
        outer_types,
    })
}

/// How [`build_set_op`] types a column whose entry names something its own level cannot
/// resolve.
#[derive(Debug, Clone, Copy)]
pub enum SetOpTyping<'a> {
    /// No query level encloses the node, so such a name is unresolvable and takes the
    /// side's own output type.
    Complete,
    /// Built where an enclosing level may own the name: the column is left pending
    /// ([`OuterTyping::Deferring`]).
    Deferring,
    /// The statement's second parse: per column, each side's enclosing-level reference
    /// the first parse's re-walk recorded ([`SetOpOuterTypes`]), so the node is built
    /// complete with nothing pending.
    Resolved(&'a [SideReferenceTypes]),
}

/// How [`unify_set_op_columns`] types a side's entry that names a column its own level
/// cannot resolve — a reference to an **enclosing** query level.
///
/// 18.6 types such a `Var` during parse analysis, because the enclosing levels'
/// namespaces are already on the `ParseState` chain when the subquery is analyzed. A
/// set operation here is built as its sides are parsed, which for an expression
/// subquery in a target list is **before** the enclosing level's `FROM` has been read,
/// so the reference has no type at that point: `SELECT (SELECT i UNION ALL SELECT '7'
/// LIMIT 1) FROM t` cannot be typed by anything its builder holds. The column is left
/// **pending** instead, and the correlated re-walk (`super::correlation`), which holds the
/// level-chained scope, types and unifies it.
pub(super) enum OuterTyping<'a> {
    /// Every column is unified now: the level's namespace is whole, so an unresolvable
    /// name takes the side's own output type — or, on a statement's second parse, a side
    /// whose entry named an enclosing level takes the type `references` recorded for it
    /// ([`SetOpTyping::Resolved`]).
    Complete {
        references: Option<&'a [SideReferenceTypes]>,
    },
    /// Built inside an expression subquery, or inside a FROM item one encloses: a column
    /// one of whose entries names something its own level cannot resolve is left
    /// pending, and so is every column after it.
    Deferring,
    /// The re-walk: each side's own level scope and that scope chained onto the
    /// enclosing levels, and whether the chain is the statement's whole one.
    Chained {
        left: SideScopes<'a>,
        right: SideScopes<'a>,
        complete: bool,
    },
}

/// One set-op side's scopes for [`OuterTyping::Chained`].
#[derive(Clone, Copy)]
pub(super) struct SideScopes<'a> {
    /// The side's own level, alone.
    pub(super) level: &'a ColumnScope,
    /// The side's level chained onto every enclosing level the re-walk holds.
    pub(super) chained: &'a ColumnScope,
}

/// The parts of a [`Plan::SetOp`] [`unify_set_op_columns`] reads and rewrites.
pub(super) struct SetOpColumns<'a> {
    pub(super) op: SetOpKind,
    pub(super) left: &'a mut Plan,
    pub(super) right: &'a mut Plan,
    pub(super) column_types: &'a mut Vec<TypeOid>,
    /// The column positions to unify, ascending.
    pub(super) columns: &'a [usize],
}

/// What an entry's column references resolve to, for [`unify_set_op_columns`].
enum OuterReference {
    /// No reference was found to need an enclosing level: each resolves at the side's own
    /// level, or the entry has none, or the column is `unknown` or not reached by
    /// `target_list_entry_with_input`, or the typing is [`OuterTyping::Complete`].
    Local,
    /// A reference resolves only at an enclosing level; the entry's type through the
    /// chained scope, when the type walk can name one, with the enum it names.
    Outer(Option<ResolvedType>),
    /// A reference no scope held resolves.
    Unresolved,
}

/// What [`unify_set_op_columns`] leaves behind.
pub(super) struct UnifiedSetOpColumns {
    /// The positions still **pending** an enclosing level ([`OuterTyping`]).
    pub(super) pending: Vec<usize>,
    /// Each column unified by this call, with each side's enclosing-level reference it was
    /// unified with — what the re-walk records in [`SetOpOuterTypes`].
    pub(super) unified: Vec<(usize, SideReferenceTypes)>,
}

/// Unify the set-op columns `parts.columns` names — [`build_set_op`]'s § An `unknown`
/// entry yields to the other side.
///
/// A pending column is given a provisional type (the types of its sides that are neither
/// `unknown` nor pending, else `text`), raises nothing and rewrites no entry: whether its
/// `unknown` entry converts, and to what, waits for the reference's real type.
/// `SELECT i EXCEPT SELECT '7'` over an outer `integer` `i` is an `integer` column whose
/// `'7'` converts to `7` on 18.6, measured, and converting it to `text` first compared
/// `7` with `'7'` and kept a row 18.6 removes. The provisional type is read by whatever
/// asks the node for its columns before the re-walk, which is why the parser does not
/// return a plan holding one ([`SetOpOuterTypes`]).
///
/// A side that is itself a set operation with the column pending leaves the column
/// pending too, rather than unifying against the side's provisional type:
/// `(SELECT i EXCEPT SELECT '8') INTERSECT SELECT '07'` over an outer `integer` `i` is an
/// `integer` column on 18.6, whose `'07'` converts to `7`.
///
/// # Errors
///
/// As [`build_set_op`], for each column unified.
pub(super) fn unify_set_op_columns(
    parts: SetOpColumns<'_>,
    schema: &Schema,
    typing: &OuterTyping<'_>,
) -> Result<UnifiedSetOpColumns> {
    let SetOpColumns {
        op,
        left,
        right,
        column_types,
        columns,
    } = parts;
    let left_cols = left.output_columns(schema);
    let right_cols = right.output_columns(schema);
    let left_unknown = unknown_output_columns(left, schema, left_cols.len());
    let right_unknown = unknown_output_columns(right, schema, right_cols.len());
    let (left_scopes, right_scopes, complete, recorded) = match typing {
        OuterTyping::Complete { references } => (None, None, true, *references),
        OuterTyping::Deferring => (None, None, false, None),
        OuterTyping::Chained {
            left,
            right,
            complete,
        } => (Some(*left), Some(*right), *complete, None),
    };
    let finalizing = matches!(typing, OuterTyping::Complete { .. });
    let reference = |side: &Plan,
                     col: &ColumnDesc,
                     unknown: bool,
                     i: usize,
                     s: usize,
                     scopes: Option<SideScopes<'_>>| {
        if unknown {
            OuterReference::Local
        } else if finalizing {
            recorded.and_then(|r| r.get(i)).and_then(|r| r[s]).map_or(
                OuterReference::Local,
                |(oid, enum_oid)| {
                    let enum_name = enum_oid.and_then(|e| {
                        schema
                            .enums
                            .iter()
                            .find(|(_, t)| t.oid == e)
                            .map(|(n, _)| n.clone())
                    });
                    OuterReference::Outer(Some(ResolvedType::new(oid, enum_name)))
                },
            )
        } else {
            match set_op_pending_beneath(side, i) {
                Some(pending) if pending.contains(&i) => OuterReference::Unresolved,
                // The re-walk has unified a nested set operation's column by now, and its
                // identity is recorded for the second parse, where the side's descriptor
                // cannot yet say it: an arm's enclosing-level reference is described
                // through a record only the re-walk writes ([`EnclosingColumns`]).
                Some(_) if scopes.is_some() => {
                    OuterReference::Outer(Some(ResolvedType::of_column(col)))
                }
                _ => outer_reference(side, i, schema, scopes),
            }
        }
    };

    // The unified type per column position, and the coercion of each side's `unknown`
    // entry to it, interleaved per column as `constructSetOpTargetlist` interleaves them.
    let mut pending = Vec::new();
    let mut unified_columns = Vec::new();
    for &i in columns {
        let (Some(l), Some(r)) = (left_cols.get(i), right_cols.get(i)) else {
            continue;
        };
        let l_unknown = left_unknown[i];
        let r_unknown = right_unknown[i];
        let l_ref = reference(left, l, l_unknown, i, 0, left_scopes);
        let r_ref = reference(right, r, r_unknown, i, 1, right_scopes);
        let unresolved = |r: &OuterReference| matches!(r, OuterReference::Unresolved);
        // Routed through `select_common_type` so the two no-common-type codes stay
        // split the way Postgres splits them: `42804` across categories, `42846`
        // within one (`time` vs `timestamp`).
        // Each side is read as a [`ResolvedType`] — the OID **and** the enum identity,
        // which is the only thing that tells a `mood` column from a `text` one at this
        // fold (`EDGES.md` row 114) or from another enum (row 115). A correlated outer
        // reference carries the enum its entry names through the chained scope, and the
        // record the statement's second parse reads carries the same identity.
        let known = |unknown: bool, reference: &OuterReference, col: &ColumnDesc| {
            if unknown {
                return None;
            }
            match reference {
                OuterReference::Outer(Some(t)) => Some(t.clone()),
                _ => Some(ResolvedType::of_column(col)),
            }
        };
        // A column after a pending one waits too, because the columns are unified left to
        // right and the first column's error is the one reported: `SELECT i, 'x' EXCEPT
        // SELECT 'y', 2` over an outer `integer` `i` names `"y"` on 18.6, measured.
        if !complete && (!pending.is_empty() || unresolved(&l_ref) || unresolved(&r_ref)) {
            let settled = |unknown: bool, reference: &OuterReference, col: &ColumnDesc| {
                if unresolved(reference) {
                    None
                } else {
                    known(unknown, reference, col)
                }
            };
            column_types[i] = select_common_type(
                &[settled(l_unknown, &l_ref, l), settled(r_unknown, &r_ref, r)],
                op.keyword(),
            )
            .ok()
            .flatten()
            .map_or(TypeOid::Text, |t| t.oid);
            pending.push(i);
            continue;
        }
        let resolved = select_common_type(
            &[known(l_unknown, &l_ref, l), known(r_unknown, &r_ref, r)],
            op.keyword(),
        )?;
        let unified = resolved.as_ref().map_or(TypeOid::Text, |t| t.oid);
        // The enum the fold landed on, for the `unknown` side's conversion to it. The
        // OID alone cannot carry it — a `mood` column's is `Text` — and this is the one
        // position in this fold where 18.6 runs the target's **input function**.
        let unified_enum = resolved.as_ref().and_then(|t| t.enum_type.name());
        if l_unknown {
            coerce_unknown_entry(left, i, unified, unified_enum, schema, &l.name)?;
        }
        if r_unknown {
            coerce_unknown_entry(right, i, unified, unified_enum, schema, &r.name)?;
        }
        column_types[i] = unified;
        let outer_type = |reference: &OuterReference| match reference {
            OuterReference::Outer(Some(t)) => Some((
                t.oid,
                t.enum_type
                    .name()
                    .and_then(|n| schema.enums.get(n))
                    .map(|e| e.oid),
            )),
            _ => None,
        };
        unified_columns.push((i, [outer_type(&l_ref), outer_type(&r_ref)]));
    }
    Ok(UnifiedSetOpColumns {
        pending,
        unified: unified_columns,
    })
}

/// The **pending** columns of the set operation a set-op side's output column `index`
/// passes through, or `None` where it passes through none — see
/// [`unify_set_op_columns`].
fn set_op_pending_beneath(side: &Plan, index: usize) -> Option<&[usize]> {
    match side {
        Plan::SetOp { outer_pending, .. } => Some(outer_pending),
        Plan::Filter { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Distinct { input }
        | Plan::DistinctOn { input, .. } => set_op_pending_beneath(input, index),
        Plan::JunkFilter { input, keep } if index < *keep => set_op_pending_beneath(input, index),
        _ => None,
    }
}

/// Whether output column `index` of a set-op side names an enclosing query level — see
/// [`OuterReference`]. `scopes` is `None` before the re-walk, where a name the side's
/// own input cannot type is [`OuterReference::Unresolved`].
///
/// The side's own input is asked first, through the same [`TypeScope`] its output
/// columns are typed with, then — for a name that scope misses — the side's level scope
/// alone, so a name the input walk cannot type but this level owns keeps the side's own
/// output type rather than being re-typed through the chain. Only a name neither level
/// reading resolves is looked up in the enclosing levels.
fn outer_reference(
    side: &Plan,
    index: usize,
    schema: &Schema,
    scopes: Option<SideScopes<'_>>,
) -> OuterReference {
    let Some((entry, input)) = target_list_entry_with_input(side, index) else {
        return OuterReference::Local;
    };
    let mut refs = Vec::new();
    collect_referenced_columns(entry, &mut refs);
    if refs.is_empty() {
        return OuterReference::Local;
    }
    let scoped = input.map(|p| p.output_columns_scoped(schema));
    let local = scoped
        .as_ref()
        .map_or(TypeScope::NONE, |(shape, relations)| TypeScope::Input {
            relations,
            columns: shape.columns(),
            enclosing: None,
        });
    let mut outer = false;
    for (table, name) in &refs {
        if local
            .own_column_type(schema, table.as_deref(), name)
            .is_some()
        {
            continue;
        }
        let Some(scopes) = scopes else {
            return OuterReference::Unresolved;
        };
        if scopes.level.column_type(table.as_deref(), name).is_some() {
            continue;
        }
        if scopes.chained.column_type(table.as_deref(), name).is_none() {
            return OuterReference::Unresolved;
        }
        outer = true;
    }
    match scopes {
        Some(scopes) if outer => {
            let leaf = OuterLeafScope {
                local: PlannerScope::new(schema, local),
                chained: scopes.chained,
            };
            OuterReference::Outer(entry.infer_type(&leaf).map(|oid| {
                ResolvedType::new(
                    oid,
                    crate::executor::expr_enum_with(
                        entry,
                        &crate::executor::EnumLeaves {
                            column: &|table, name| leaf.column_enum(table, name),
                            subquery: &|_| None,
                        },
                    ),
                )
            }))
        }
        _ => OuterReference::Local,
    }
}

/// A set-op side's own [`PlannerScope`] with the level-chained [`ColumnScope`] behind
/// it, for typing an entry that names an enclosing level ([`outer_reference`]).
struct OuterLeafScope<'a> {
    local: PlannerScope<'a>,
    chained: &'a ColumnScope,
}

impl LeafScope for OuterLeafScope<'_> {
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        self.local
            .own_column_type(table, name)
            .or_else(|| self.chained.column_type(table, name))
    }

    fn subquery_type(&self, plan: &Plan) -> Option<TypeOid> {
        self.local.subquery_type(plan)
    }
}

impl OuterLeafScope<'_> {
    /// The enum identity beside [`LeafScope::column_type`]'s walk, read from the level the
    /// type was.
    fn column_enum(&self, table: Option<&str>, name: &str) -> Option<String> {
        if self.local.own_column_type(table, name).is_some() {
            return self.local.column_enum(table, name);
        }
        self.chained.column_enum(table, name)
    }
}

/// The per-column type of a multi-row `VALUES` list, with every cell coerced to it —
/// 18.6's `transformValuesClause`, which asks [`select_common_type`] over each column and
/// then coerces each of the column's cells, column by column, before the next column's
/// common type is chosen.
///
/// An `unknown` cell — a bare string constant, a bare `NULL` or a bare `$N` the client
/// did not declare, seen through an `AS alias` — contributes no type and takes the
/// column's, so `(VALUES (1), ('2'))` is `integer` and an all-`unknown` column is `text`.
/// Each such cell is rewritten into a `Cast` and a constant one converted here, in row
/// order, as `coerce_to_common_type` converts an `unknown` `Const` during parse analysis:
/// `(VALUES (1), ('x')) v WHERE false` and `(VALUES (1/0), ('x'))` are `22P02` on 18.6, and
/// `(VALUES (1, 'a'), ('2', 3))` names `"a"`. A typed cell whose type differs from the
/// column's is wrapped in an eval-time `Cast`, and a cell no walker can type contributes
/// nothing and is cast likewise.
///
/// `written` holds the type the coercion pass read each cell at as written, by row and
/// column (`certain_entry_type`), and the answer is each column's record of it: a column
/// is certain where every cell of it is `unknown` or was read as written at the type the
/// fold reads it at here. Its type is then 18.6's common type of types that are 18.6's,
/// and every cell holds a value of it — one already of the column's type, or a cast to
/// it. A cell typed only by a walker's guess leaves its column's type a guess.
///
/// # Errors
///
/// `42804` / `42846` when a column has no common type; the conversion's own error for a
/// constant the column's type refuses.
pub fn unify_values_columns(
    rows: &mut [Vec<Expr>],
    written: &[Vec<Option<TypeOid>>],
    width: usize,
    schema: &Schema,
) -> Result<CertainTypes> {
    let scope = PlannerScope::none(schema);
    let mut certain = Vec::with_capacity(width);
    for i in 0..width {
        // A cell's enum identity comes from the same exhaustive walk the gates use
        // (`PlannerScope::expr_enum`) — a `VALUES` row holding a scalar subquery over an
        // enum column is `42804` beside a `text` sibling on 18.6, and reading the OID
        // alone made it a `text` column (`EDGES.md` row 114).
        let column: Vec<(bool, Option<ResolvedType>)> = rows
            .iter()
            .map(|row| {
                let cell = &row[i];
                let unknown = ParameterSource::Unknown.source_type(cell, schema, TypeScope::NONE)
                    == AssignmentSource::Unknown;
                (
                    unknown,
                    if unknown {
                        None
                    } else {
                        scope.resolved_type(cell)
                    },
                )
            })
            .collect();
        let types: Vec<Option<ResolvedType>> = column.iter().map(|(_, t)| t.clone()).collect();
        let unified = select_common_type(&types, "VALUES")?.map_or(TypeOid::Text, |t| t.oid);
        let every_cell_certain =
            column
                .iter()
                .zip(written)
                .all(|((unknown, cell_type), written_row)| {
                    *unknown
                        || cell_type
                            .as_ref()
                            .is_some_and(|t| written_row.get(i).copied().flatten() == Some(t.oid))
                });
        certain.push((every_cell_certain && written.len() == rows.len()).then_some(unified));
        for (row, (unknown, cell_type)) in rows.iter_mut().zip(&column) {
            let unknown = *unknown;
            if !unknown && cell_type.as_ref().map(|t| t.oid) == Some(unified) {
                continue;
            }
            let cell = std::mem::replace(&mut row[i], Expr::Literal(Literal::Null));
            let cast = Expr::Cast {
                expr: Box::new(cell),
                target_type: unified,
            };
            let converted = if unknown {
                validate_const_casts(&cast)
            } else {
                Ok(())
            };
            row[i] = cast;
            converted?;
        }
    }
    Ok(CertainTypes::new(certain))
}

/// A per-output-column mask of which columns of a set-op side are `unknown` — see
/// [`build_set_op`] § An `unknown` entry yields to the other side. The mask aligns with
/// the side's [`Plan::output_columns`] (`count` entries); a walk answering a different
/// width (a projection whose `*` expanded) is all-`false`, every column then taking its
/// inferred type.
pub fn unknown_output_columns(plan: &Plan, schema: &Schema, count: usize) -> Vec<bool> {
    let sources = target_list_source_types(plan, schema, ParameterSource::Unknown);
    if sources.len() != count {
        return vec![false; count];
    }
    sources
        .into_iter()
        .map(|source| source == AssignmentSource::Unknown)
        .collect()
}

/// The conversion of `plan`'s `unknown` output entry `index` to the enum `enum_name` —
/// the membership test [`coerce_unknown_entry`] runs at the set-operation door, for a
/// fold that keeps the plan as it is (a recursive CTE's term, whose rows the executor
/// casts per iteration).
///
/// # Errors
///
/// [`Error::InvalidEnumValue`] (`22P02`) for a literal the enum does not hold.
pub fn unknown_entry_enum_legality(
    plan: &Plan,
    index: usize,
    enum_name: &str,
    schema: &Schema,
) -> Result<()> {
    let mut probe = crate::depth::clone_plan(plan)?;
    target_list_entry_mut(&mut probe, index).map_or(Ok(()), |(entry, _)| {
        crate::planner::enum_literal_legality(enum_name, entry, schema)
    })
}

/// Rewrite a set-op side's `unknown` output entry `index` into a `Cast` to `target`,
/// named `name`, and convert a constant one now (`validate_const_casts`), so a constant
/// the type refuses fails at plan time. The side's record of the entry's type moves with
/// it ([`CertainTypes`]).
fn coerce_unknown_entry(
    plan: &mut Plan,
    index: usize,
    target: TypeOid,
    target_enum: Option<&str>,
    schema: &Schema,
    name: &str,
) -> Result<()> {
    let Some((entry, certain)) = target_list_entry_mut(plan, index) else {
        return Err(Error::Execution(format!(
            "set operation column {} has no target-list entry to coerce",
            index + 1
        )));
    };
    let taken = std::mem::replace(entry, Expr::Literal(Literal::Null));
    let (inner, alias) = match taken {
        Expr::Alias { expr, alias } => (*expr, alias),
        other => (other, name.to_string()),
    };
    // An enum target's conversion is its **input function**, and `Expr::Cast` cannot
    // name an enum — the cast below is to `text`, 18.6's *other* type, so it proves
    // nothing about membership. Running `enum_in` here is what the cast stands in for
    // everywhere else, and skipping it wrote: `INSERT INTO t SELECT m FROM w UNION ALL
    // SELECT 'zzz'` landed three rows in a `text` column where 18.6 lands none, and
    // `DELETE … WHERE v IN (SELECT m FROM w UNION ALL SELECT 'zzz')` removed both rows
    // 18.6 keeps by refusing. The wrap itself is unchanged: the value is already a
    // `Value::Text`, so the coercion is the membership test and nothing else.
    if let Some(enum_name) = target_enum {
        crate::planner::enum_literal_legality(enum_name, &inner, schema)?;
    }
    let cast = Expr::Cast {
        expr: Box::new(inner),
        target_type: target,
    };
    let result = validate_const_casts(&cast);
    *entry = Expr::Alias {
        expr: Box::new(cast),
        alias,
    };
    // The entry is of the column's type now, as 18.6 makes an `unknown` one, so the
    // column is certain where the other side's is ([`Plan::SetOp`]'s columns).
    certain.set(index, Some(target));
    result
}

/// Wrap a set-op tree with its trailing `ORDER BY` / `LIMIT`, which bind to the
/// **combined** result: `Limit(Sort(SetOp(…)))`.
///
/// The `ORDER BY` resolves against the set-op's *output* columns (there is no FROM
/// scope): a positional `ORDER BY <int>` maps to the Nth output column (1-based;
/// out of range fails loud), and a name resolves against the output column names
/// (from the left side).
///
/// **A set-op's trailing keys are bound here, because that namespace is complete by
/// construction.** 18.6 restricts them to the set-op's own output names — a
/// correlated reference and any *computed* expression over resolvable names are
/// `0A000 invalid UNION/INTERSECT/EXCEPT ORDER BY clause` there (a computed key over a
/// name that binds to nothing is `42703`, resolved first) — so there is no
/// enclosing level to wait for and no re-walk to defer to (`correlation.rs`'s
/// `Plan::SetOp` arm registers an opaque relation, which leaves the level
/// name-unknown, and that registration stays: it protects names in the set-op's
/// *arms*). Ungated, a key binding to nothing survived plan time and a write through
/// it destroyed rows — `DELETE FROM rt WHERE a NOT IN (SELECT a FROM ru UNION SELECT
/// a FROM rw ORDER BY bogusrel.a)` removed every row of `rt` at either row count
/// against 18.6's `42P01`, and the bare spelling did the same over empty arms
/// against `42703`.
///
/// **Every [`Expr::Column`] leaf is judged, not only a whole key that is one.** 18.6
/// resolves the names in a computed key *before* raising its `0A000`, and against the
/// same namespace: `transformSetOperationStmt` clears `p_namespace`, builds a join RTE
/// over the synthesized output columns, and adds it with
/// `addNSItemToQuery(pstate, jnsitem, false, false, true)` — the **var** namespace only.
/// That one call decides both arms of the routing below. A bare name sees the output
/// names and nothing else: `SELECT x AS lbl … UNION … ORDER BY lbl+0` is `0A000` (the
/// alias resolved), while `… ORDER BY x+0` over that same query is `42703` — an arm's own
/// column name is not an output name, and 18.6's DETAIL says exactly that. A qualifier
/// sees no relation at all, the RTE having been kept out of the *rel* namespace, so
/// `bogusrel.a+1` is `42P01`. And `bogus+1` / `bogus::int` / `a + bogus` are `42703`.
///
/// Ungated, every one of those **wrote**: the `WITH c AS (…) DELETE` polarity of each
/// removed every row, row-count independent since an unreferenced CTE's body never runs.
/// The *read* polarity is narrower and worth stating exactly, because it is what a
/// fixture can pin — a **bare** leaf over seeded arms was already `42703` from the
/// eval-time resolver, so only the empty-arm spelling answered, while the **qualified**
/// leaf answered at either row count.
///
/// What stays lenient is the `0A000` itself: a computed key whose leaves all bind
/// still **sorts** here (`-a`, `a+1`, `abs(a)`, `COALESCE(a,0)` — each `0A000` on 18.6).
/// An aggregate or scalar-subquery key is **not** a member of that leniency: `count(*)`
/// and `(SELECT 1)` are `0A000` on both engines — pglet from its own eval-context
/// refusals, 18.6 from the set-op rule — an agreement by different causes, pinned by
/// `select/set_op_order_by_aggregate_key_loud.sql` and
/// `select/set_op_order_by_scalar_subquery_key_loud.sql`, which can exist precisely
/// because the SQLSTATEs match. One divergence class follows from
/// judging the leaves while not raising that error, in two spellings, both loud on both
/// engines with the wrong code here: a **correlated** leaf is `42P01` qualified
/// (`… IN (SELECT x FROM sa UNION … ORDER BY x + tgt.a)`) and `42703` bare
/// (`… ORDER BY x + a`), against 18.6's `0A000` for either — 18.6 resolves it off the
/// *parent* parse state, which the cleared `p_namespace` does not reach, and then finds
/// a resjunk entry it will not accept. It is the class the whole-key correlated form
/// already carries. Recorded in `EDGES.md`.
///
/// The gate is keyed on the tree being a set-op rather than on the caller, because
/// `parser::select`'s paren-leading arm serves both: a parenthesized *plain* select's
/// sort clause is folded into the inner query by 18.6, so its key may name an inner
/// FROM column — `WITH c AS ((SELECT a FROM pt) ORDER BY b) DELETE FROM n8` answers on
/// both engines and gating that spelling would refuse it. (The **bare** top-level
/// `(SELECT a FROM pt) ORDER BY b` is a separate, pre-existing `42703` here against
/// 18.6's row — a member of `EDGES.md` row 167, the parenthesized query planned before
/// the clauses written after it; this gate neither caused nor closes it.)
///
/// # Errors
///
/// Fails loud on a positional `ORDER BY` index outside the output column range
/// (`42P10`), and — over a set-op tree — on any [`Expr::Column`] leaf of a key naming an
/// unknown qualifier (`42P01`), an unknown output column (`42703`), or two same-named
/// output entries (`42702`).
pub fn plan_set_op_order_limit(
    tree: Plan,
    order_by: Vec<OrderKey>,
    limit: Option<(Option<LimitCount>, Option<LimitCount>)>,
    schema: &Schema,
) -> Result<Plan> {
    let mut plan = tree;
    if !order_by.is_empty() {
        let out_cols = plan.output_columns(schema);
        // The two scopes a set-op key resolves against: an empty **closed** one (a
        // qualifier is `42P01`, since a set-op's trailing keys reach no range table),
        // and that one carrying the output names as claimants for a bare name. Every
        // identity is `None`, and that **matches** `equal()` rather than diverging from
        // it: `transformSetOperationStmt` does not reuse the arms' expressions, it
        // *synthesizes* the outer target list as one `makeVar(leftmostRTI, resno)` per
        // output position, so `SELECT a AS x, a AS x … UNION … ORDER BY x` compares two
        // Vars differing in `varattno`, `equal()` is false, and 18.6 answers `42702`
        // (measured) — which is what an identity-less entry, counting as its own
        // claimant, already reports.
        let scopes = is_set_op_tree(&plan).then(|| {
            let base = ColumnScope::default().closed();
            let mut keys = base.clone();
            keys.push_output_entries(out_cols.iter().map(|c| OutputEntry {
                name: c.name.clone(),
                expr: None,
            }));
            (base, keys)
        });
        let resolved = order_by
            .into_iter()
            .map(|key| {
                // Validate a positional ordinal at plan time (`42P10` on
                // out-of-range) but keep the literal, so the Sort resolves it by
                // *index* rather than by a set-op's ambiguous `?column?` name (see
                // `eval_order_key`). A named / expression key passes through unchanged.
                // The direction + NULL placement ride along untouched.
                //
                // The binding check sits in this same per-key closure so the two
                // interleave in source order, as 18.6 resolves them: `ORDER BY bogus,
                // 99` is `42703` there and `ORDER BY 99, bogus` is `42P10`.
                positional_key_index(&key.expr, out_cols.len(), "ORDER BY")?;
                if let Some((base, keys)) = scopes.as_ref() {
                    check_set_op_key_leaves(&key.expr, base, keys)?;
                }
                Ok(key)
            })
            .collect::<Result<Vec<_>>>()?;
        plan = Plan::Sort {
            input: Box::new(plan),
            order_by: resolved,
            // A set operation resolves every `unknown` in both its sides before this
            // clause is reached (`build_set_op`'s per-position unification), so there
            // is no entry left for a key to claim.
            resolved_targets: ResolvedTargets::default(),
        };
    }
    if let Some((limit_val, offset_val)) = limit {
        plan = Plan::Limit {
            input: Box::new(plan),
            limit: limit_val,
            offset: offset_val,
        };
    }
    Ok(plan)
}

/// Judge every [`Expr::Column`] leaf of a set-op's trailing `ORDER BY` key, in source
/// order, against the two scopes [`plan_set_op_order_limit`] builds — a bare name
/// against `keys` (the output-name namespace), a qualified one against the empty closed
/// `base`, which is [`super::correlation::scope_for_key`]'s own split applied one level
/// down.
///
/// A key that *is* a column is the one-leaf case of this and not a separate rule; the
/// recursion is what closes the computed spellings (`bogus+1`, `bogus::int`, `a +
/// bogus`, `bogusrel.a+1`), each of which 18.6 resolves before its own `0A000`. A
/// self-contained sub-plan is a leaf to [`Expr::for_each_child`] and so is one here: a
/// `(SELECT …)` key resolves its own columns internally.
///
/// # Errors
///
/// The first leaf's [`ColumnScope::check_binding`] verdict — `42P01` / `42703` /
/// `42702` — which is Postgres's positionally-first one, since children are visited in
/// source order.
fn check_set_op_key_leaves(expr: &Expr, base: &ColumnScope, keys: &ColumnScope) -> Result<()> {
    crate::depth::check_stack()?;
    if let Expr::Column { table, name } = expr {
        return scope_for_key(expr, base, keys).check_binding(table.as_deref(), name);
    }
    expr.try_for_each_child(|child| check_set_op_key_leaves(child, base, keys))
}

/// Whether a trailing `ORDER BY`'s input is a set operation, so its keys bind to the
/// set-op's output names alone (see [`plan_set_op_order_limit`]).
///
/// A parenthesized operand's own `LIMIT` **and** its own `ORDER BY` are peeled: on 18.6
/// `(SELECT a FROM ru UNION SELECT a FROM rw LIMIT 1) ORDER BY bogus` is `42703`, the
/// same as without the bound. An operand carrying its own `ORDER BY` under a trailing one
/// is `42601 multiple ORDER BY clauses not allowed` in the parse
/// (`parser::select::QueryClauses::merge`) before this walk runs.
///
/// The peel is not what keeps a parenthesized **plain** select ungated — nothing below a
/// `Plan::Limit` over a plain select is a `Plan::SetOp`, so the walk stops on its own.
/// `WITH c AS ((SELECT a FROM t LIMIT 5) ORDER BY bogus) DELETE FROM tgt` is 18.6's
/// `42703` — its grammar merges the trailing `ORDER BY` into the plain select — and
/// pglet answers it and writes, a pre-existing leniency this gate neither causes nor
/// closes.
fn is_set_op_tree(plan: &Plan) -> bool {
    match plan {
        Plan::SetOp { .. } => true,
        Plan::Limit { input, .. } | Plan::Sort { input, .. } => is_set_op_tree(input),
        _ => false,
    }
}

/// Resolve a positional `ORDER BY` / `DISTINCT ON` key that is a bare integer
/// literal to the Nth output column (1-based), so `ORDER BY 1` orders by the first
/// result column. Any other key (a name or expression) is returned unchanged.
///
/// `clause` is the label (`"ORDER BY"` / `"DISTINCT ON"`) used in the out-of-range
/// error, matching Postgres's `<clause> position <n> is not in select list`.
///
/// # Errors
///
/// A positional index below 1 or past the output column count fails loud (`42P10`).
fn resolve_output_position(
    key: Expr,
    out_cols: &[ColumnDesc],
    clause: &'static str,
) -> Result<Expr> {
    positional_key_index(&key, out_cols.len(), clause)?.map_or(Ok(key), |idx| {
        Ok(Expr::Column {
            table: None,
            name: out_cols[idx].name.clone(),
        })
    })
}

/// The 0-based output-column index a positional `ORDER BY` / `DISTINCT ON` key (a
/// bare integer literal) refers to, or `None` for a non-positional key.
///
/// The plan-time range check shared by [`resolve_output_position`] (which lowers the
/// ordinal to a by-name reference for `DISTINCT ON` canonicalization) and the set-op
/// `ORDER BY` path (which keeps the literal so the Sort resolves it by *index* — a
/// set-op's unnamed `?column?` columns collide by name).
///
/// # Errors
///
/// A `1`-based ordinal outside `1..=n_out` fails loud (`42P10`), matching Postgres's
/// `<clause> position <n> is not in select list`. A bare **non-integer constant** key
/// (`ORDER BY 1.5`, `'x'`, `true`, `NULL`, `DISTINCT ON (1.5)`) fails loud (`42601`,
/// `non-integer constant in <clause>`) — only an integer literal is a position, and no
/// other constant is a meaningful key. A non-constant expression (`ORDER BY 1+1`) is
/// not an `Expr::Literal`, so it passes through as an ordinary sort/distinct key.
fn positional_key_index(key: &Expr, n_out: usize, clause: &'static str) -> Result<Option<usize>> {
    let n = match key {
        Expr::Literal(Literal::Int(n)) => *n,
        // Any other bare constant literal is rejected as a non-integer constant key
        // (matching Postgres), distinct from an out-of-range integer ordinal.
        Expr::Literal(
            Literal::Null
            | Literal::Bool(_)
            | Literal::Float(_)
            | Literal::String(_)
            | Literal::Numeric(_),
        ) => return Err(Error::NonIntegerConstant { clause }),
        _ => return Ok(None),
    };
    usize::try_from(n)
        .ok()
        .filter(|&i| i >= 1 && i <= n_out)
        .map_or_else(
            || {
                Err(Error::InvalidColumnPosition {
                    clause,
                    position: n,
                })
            },
            |i| Ok(Some(i - 1)),
        )
}

/// Merge a `DISTINCT ON` key into the `ORDER BY` to produce the single physical
/// sort key a `DISTINCT ON` query needs — one whose leading entries are the ON
/// exprs (so equal-ON rows are adjacent for the first-per-group dedup) and whose
/// order is also the query's visible output order.
///
/// This mirrors Postgres's `transformDistinctOnClause` (parse_clause.c), which
/// "absorbs the sorting semantics of ORDER BY into the distinctClause": the ON
/// exprs must appear among the *initial* ORDER BY items — before any non-ON
/// ("foreign") sort key — and any ON expr the ORDER BY omits is appended `ASC`.
/// It is a **stateful** scan, not a static prefix check:
///
/// - Walk the ORDER BY left→right, keeping every item (both ON keys and the
///   foreign tiebreakers, e.g. `salary DESC`, that decide which row wins per
///   group). Once a foreign item is seen, a *later* ON item is a mismatch → fail
///   loud `42P10`.
/// - Append each ON expr not already present, `ASC` — unless a foreign item was
///   seen anywhere (then an unmatched ON expr is likewise a `42P10` mismatch).
///
/// With no ORDER BY the merged key is just the ON exprs `ASC` (Postgres allows
/// this; which row of each group survives is then unspecified, not an error).
/// ON exprs are compared by `Expr`'s structural equality, so callers must resolve
/// positional/alias forms to their output column first.
///
/// # Errors
///
/// `42P10` when a `DISTINCT ON` expr does not match the initial `ORDER BY`.
fn merge_distinct_on_order_by(on_exprs: &[Expr], order_by: &[OrderKey]) -> Result<Vec<OrderKey>> {
    let mut merged: Vec<OrderKey> = Vec::with_capacity(order_by.len() + on_exprs.len());
    // Whether a non-`DISTINCT ON` ("foreign") ORDER BY item has been passed.
    let mut skipped_foreign = false;

    // Keep every ORDER BY item (with its direction + NULL placement), but reject an
    // ON expr that follows a foreign one.
    for key in order_by {
        if on_exprs.contains(&key.expr) {
            if skipped_foreign {
                return Err(Error::DistinctOnOrderByMismatch);
            }
        } else {
            skipped_foreign = true;
        }
        merged.push(key.clone());
    }

    // Append any ON expr the ORDER BY didn't cover, ASC (default NULL placement) —
    // but only while no foreign item has been seen (an unmatched ON expr past one is
    // the same mismatch).
    for expr in on_exprs {
        if merged.iter().any(|k| &k.expr == expr) {
            continue;
        }
        if skipped_foreign {
            return Err(Error::DistinctOnOrderByMismatch);
        }
        merged.push(OrderKey::new(expr.clone(), true));
    }

    Ok(merged)
}

/// Canonicalize a `DISTINCT ON` / `ORDER BY` key to the identity used to match a
/// `DISTINCT ON` expr against an `ORDER BY` key. Postgres matches by target-entry
/// identity (`sortgroupref`), not spelling, so a qualified `emp.dept`, a bare
/// `dept`, and an output alias all denote the same output column. A column
/// reference (or output alias) that resolves to an output column becomes a bare
/// `Column { name: <output name> }`; a non-column expression — or a column not in
/// the output (a foreign `ORDER BY` key on a dropped base column) — is returned
/// unchanged, so the sort-below-projection path still sees its original (qualified)
/// form and non-column exprs keep structural equality.
fn canonicalize_distinct_key(
    expr: Expr,
    projection: &[Expr],
    out_names: &HashSet<String>,
    scope: &ColumnScope,
) -> Expr {
    output_key_identity(&expr, projection, out_names, scope, true).unwrap_or(expr)
}

/// Rewrite a canonicalized `DISTINCT ON` key so every column it reads is a column of
/// the **projected** row, which is what [`Plan::DistinctOn`] evaluates it against.
///
/// A key already tied to an output column is returned untouched. Otherwise each column
/// reference inside it is re-tied through [`output_key_identity`]'s
/// projection-denotes-this-base-column branch — the value-preserving one, so the
/// rewritten key reads the same column under the name the projection gives it. A
/// reference the projection carries under no name cannot be read up here at all, and
/// is refused rather than left to bind by bare name to whatever output column happens
/// to share its spelling.
///
/// [`None`] when the projection drops a reference the key needs, so the **whole** key
/// has to ride the projecting node as a junk column instead — the same whole-key
/// verdict [`resolve_aggregate_order_keys`] makes, and for the same reason: a key
/// evaluated below the projection cannot have any part of it rewritten to an output
/// alias.
fn project_distinct_on_key(
    expr: Expr,
    projection: &[Expr],
    out_names: &HashSet<String>,
    scope: &ColumnScope,
) -> Option<Expr> {
    // The key itself, already tied to an output column by canonicalization.
    if let Expr::Column { table: None, name } = &expr
        && out_names.contains(name)
    {
        return Some(expr);
    }
    rewrite_key_columns_to_output(expr, projection, out_names, scope)
}

/// The refusal for a `DISTINCT ON` key the projecting node cannot carry as a junk
/// column — a plan shape with no expression list of its own.
///
/// Postgres accepts every such key; this is what is left of pglet's over-rejection
/// once the junk column closed the reachable half of it.
fn distinct_on_key_dropped(expr: &Expr) -> Error {
    let column = match expr {
        Expr::Column {
            table: Some(t),
            name,
        } => format!("{t}.{name}"),
        Expr::Column { table: None, name } => name.clone(),
        other => other.output_name(),
    };
    Error::NotImplemented(format!(
        "DISTINCT ON key \"{column}\" must be an output column of the select list"
    ))
}

/// Append junk columns to whichever projecting node a `SELECT` put at the top.
///
/// The arms are exactly the nodes `plan_select` builds there, and the caller has
/// already established that `plan` is one of them (`can_carry_junk`) — minting a junk
/// column it could not place would drop the column and dedup by the wrong one.
fn push_junk_columns(plan: &mut Plan, junk: Vec<Expr>) {
    match plan {
        Plan::Project { exprs, .. } | Plan::Aggregate { exprs, .. } => exprs.extend(junk),
        Plan::Window { window_exprs, .. } => window_exprs.extend(junk),
        _ => debug_assert!(junk.is_empty(), "junk column on a node with no projection"),
    }
}

/// [`project_distinct_on_key`]'s walk: re-tie **every** column reference, at every
/// depth, through [`output_key_identity`]'s projection-denotes-this-base-column
/// branch.
///
/// Separate from its caller so the "already an output column" shortcut applies to the
/// key alone. Folded into one recursive function, that shortcut reaches the nested
/// references too — and a nested bare name matching an output column is exactly the
/// rebinding this walk exists to prevent, so the walk silently did nothing for the
/// keys it was written for (`DISTINCT ON (a+0)` over `SELECT b AS a`).
///
/// [`None`] as soon as any reference at any depth is one the projection drops, which
/// makes the whole key junk.
fn rewrite_key_columns_to_output(
    expr: Expr,
    projection: &[Expr],
    out_names: &HashSet<String>,
    scope: &ColumnScope,
) -> Option<Expr> {
    let mut expr = expr;
    rewrite_key_columns_in_place(&mut expr, projection, out_names, scope).then_some(expr)
}

/// [`rewrite_key_columns_in_place`] over an `IN` list's members, stopping at the first
/// that denotes no output column.
#[inline(never)]
fn rewrite_key_columns_in_list(
    list: &mut [Expr],
    projection: &[Expr],
    out_names: &HashSet<String>,
    scope: &ColumnScope,
) -> bool {
    list.iter_mut()
        .all(|item| rewrite_key_columns_in_place(item, projection, out_names, scope))
}

/// [`rewrite_key_columns_to_output`] in place, `false` when a column denotes no output
/// column. A chain is walked from this frame, one a level (`crate::depth`), and once
/// one column fails nothing more is rewritten — the key is dropped whole.
fn rewrite_key_columns_in_place(
    expr: &mut Expr,
    projection: &[Expr],
    out_names: &HashSet<String>,
    scope: &ColumnScope,
) -> bool {
    let walk = |child: &mut Expr| rewrite_key_columns_in_place(child, projection, out_names, scope);
    match expr {
        Expr::Column { .. } => {
            let Some(output) = output_key_identity(expr, projection, out_names, scope, false)
            else {
                return false;
            };
            *expr = output;
            true
        }
        Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } => {
            walk(left) && walk(right)
        }
        Expr::Negate(child)
        | Expr::UnaryPlus(child)
        | Expr::Not(child)
        | Expr::IsNull(child)
        | Expr::IsNotNull(child)
        | Expr::Cast { expr: child, .. }
        | Expr::CastTypmod { expr: child, .. }
        | Expr::InSubquery { expr: child, .. }
        | Expr::BooleanTest { expr: child, .. } => walk(child),
        Expr::InList {
            expr: child, list, ..
        } => walk(child) && rewrite_key_columns_in_list(list, projection, out_names, scope),
        other => {
            let mut kept = true;
            other.for_each_child_mut(&mut |child: &mut Expr| {
                kept = kept && walk(child);
            });
            kept
        }
    }
}

/// The output column an `ORDER BY` / `DISTINCT ON` key denotes, as a bare
/// `Column { name: <output name> }`, or `None` when the key is not an output
/// expression at all (a foreign key on a dropped base column).
///
/// Postgres matches these keys by target-entry identity (`sortgroupref`), not
/// spelling, so a qualified `emp.dept`, a bare `dept`, an output alias, and the
/// select-list expression itself all denote the same output column. The three
/// branches below are that identity test, and each is **qualifier-aware**: a
/// qualified `pb.x` must denote relation `pb`'s column, and must not be answered by a
/// sibling `pa.x` that merely shares the bare name (`findTargetlistEntrySQL92` takes
/// the output-name branch only for a single-field name).
///
/// `whole_key` says whether `expr` is the key itself rather than a node inside it, and
/// gates branch (a) alone — see [`key_may_name_an_output_column`], the same rule
/// applied one pass earlier where the output names are not known yet.
fn output_key_identity(
    expr: &Expr,
    projection: &[Expr],
    out_names: &HashSet<String>,
    scope: &ColumnScope,
    whole_key: bool,
) -> Option<Expr> {
    let bare = |name: &str| Expr::Column {
        table: None,
        name: name.to_string(),
    };
    if let Expr::Column { table, name } = expr {
        // (a) An **unqualified** reference naming an output column — a bare projected
        // column or an output alias. Qualified references are excluded here by
        // Postgres's single-field rule, and so is a reference **nested** inside a
        // larger key, for the same clause of the same rule: `findTargetlistEntrySQL92`
        // tests the key, so `ORDER BY lbl+0` resolves `lbl` against the range table.
        // Branch (b) is what resolves both of those, and it is a stricter test —
        // the projection must *denote that very base column*, so the rewrite it
        // performs is value-preserving where this one is a rebinding. Measured over
        // `tt(q, a)`: `SELECT count(*) AS q FROM tt GROUP BY a ORDER BY q+0` is
        // `42803` on 18.6 (the nested `q` is `tt.q`, ungrouped) and answered rows here.
        if whole_key && table.is_none() && out_names.contains(name) {
            return Some(bare(name));
        }
        // (b) A projection entry that denotes this very base column — under an alias
        // (`pa.x AS d`) or spelled differently (`ORDER BY c.x` over `SELECT x`).
        for p in projection {
            let (inner, out_name) = match p {
                Expr::Alias { expr: inner, alias } => (inner.as_ref(), alias.clone()),
                other => (other, other.output_name()),
            };
            // An unexpanded bare `*` projects **every** column of the FROM relation
            // under its own name, so it denotes this base column exactly when the scope
            // carries it — and the output name is the column's own, not the entry's
            // (`*`). `expand_wildcards` leaves a bare `*` unexpanded only when it is the
            // projection's **sole** item and the FROM is not a `Plan::Join`, so there is
            // no side to pick — a comma FROM *is* a
            // join and takes the expansion (measured: `SELECT DISTINCT ON (ot.v) * FROM
            // ot, ob ORDER BY ot.v` answers identically on both engines).
            // Without this arm the star matched no projection entry and an ordinary
            // `SELECT DISTINCT ON (zz.a) * FROM zz ORDER BY zz.a, b DESC` was `0A000`.
            if inner.is_bare_star() {
                if scope.contains_column_ref(table.as_deref(), name) {
                    return Some(bare(name));
                }
                continue;
            }
            if projection_denotes_column(inner, table.as_deref(), name, scope) {
                return Some(bare(&out_name));
            }
        }
        return None;
    }
    // (c) A non-column key matching a select-list expression whole (`SELECT x+1 AS z
    // … ORDER BY x+1`) denotes that output column.
    projection.iter().find_map(|p| {
        let (inner, out_name) = match p {
            Expr::Alias { expr: inner, alias } => (inner.as_ref(), alias.clone()),
            other => (other, other.output_name()),
        };
        (inner == expr).then(|| bare(&out_name))
    })
}

/// Whether the projection entry `projected` (already stripped of any `AS` alias)
/// denotes exactly the base column `[qualifier.]name`.
///
/// A **bare** projection entry denotes a qualified key only when the key's relation is
/// the sole one carrying that column — otherwise the unqualified projection could bind
/// to the other side of the join, and claiming the match would repeat the bare-name
/// conflation this whole path exists to avoid.
fn projection_denotes_column(
    projected: &Expr,
    qualifier: Option<&str>,
    name: &str,
    scope: &ColumnScope,
) -> bool {
    let Expr::Column {
        table: ptable,
        name: pname,
    } = projected
    else {
        return false;
    };
    if pname != name {
        return false;
    }
    match (qualifier, ptable.as_deref()) {
        // Both qualified — same relation or nothing.
        (Some(q), Some(pq)) => q == pq,
        // Key qualified, projection bare: the projection binds to whichever relation
        // carries `name`, so this is the key's relation only if it is the sole one.
        (Some(q), None) => scope.sole_relation_with(name) == Some(q),
        // Key bare: it binds the same way the projection does.
        (None, _) => true,
    }
}

/// Assemble `SELECT DISTINCT ON (…)` for a **no-FROM** select (`SELECT DISTINCT ON
/// (…) <exprs>`). A no-FROM *leaf* carries no `ORDER BY` — the trailing one binds to the
/// whole select and is applied above this node by [`plan_no_from_order_limit`] — so the ON
/// key is the whole effective sort key here, and a positional `DISTINCT ON (5)` past the
/// select-list length fails loud (matching Postgres) rather than being silently dropped.
/// **The two clauses are therefore not merged**, where `lower_select_level` merges them
/// (`merge_distinct_on_order_by`), and that leaves one divergence measured and open:
/// Postgres's rule that the ON expressions match the leftmost `ORDER BY` is not enforced
/// for this shape, so `SELECT DISTINCT ON (q) unnest(ARRAY[2,1,2]) AS q, 5 AS r ORDER BY
/// r, q` answers here against 18.6's `42P10`. It answered before the tail was routed too
/// — the clause was simply dropped — so the routing neither opened nor closed it; what
/// the routing *did* close is the ordering, `… DISTINCT ON (q) … ORDER BY q DESC` having
/// answered `1 2` here against 18.6's `2 1`. The dedup is a
/// no-op over a plain one-row projection and is **not** one over a projection `unnest`,
/// which row-expands (`SELECT DISTINCT ON (q) unnest(ARRAY[2,1]) AS q` answers two rows on
/// both engines) — which is also what makes the binding hole below reachable.
///
/// **A top-level sequence projection is excluded from the wrapping**, for
/// [`plan_no_from_order_limit`]'s plan-*shape* reason: the executor dispatches
/// `SELECT nextval/currval/setval(…)` on [`Plan::is_sequence_projection`], which matches a
/// bare `Plan::Values`, so wrapping one routes it to the read-only eval path where a
/// `SequenceFn` fails loud — `SELECT DISTINCT ON (1) nextval('s')` was `0A000` here
/// against 18.6's row. The gates above still run before the decline, so nothing loud
/// becomes silent (`DISTINCT ON (5)` over a sequence projection is `42P10` and
/// `DISTINCT ON (bogusdon)` is `42703`, both matching 18.6). Unlike that function's
/// decline, this one **changes no answer**: such a projection is a single row, a
/// first-per-group dedup under a one-key sort returns that row unchanged, and the same
/// soundness is what lets `parser::select` elide a plain `DISTINCT` over it. The tail's
/// `LIMIT`/`OFFSET` does change an answer over one row, which is why that decline is a
/// recorded divergence and this one is not.
///
/// **Every ON key's column references are bound here, at plan time.** A no-FROM select
/// has no FROM relations at all, so at a level no *expression* subquery encloses the only
/// names an ON key may resolve to are its own target-list entries — which makes that
/// level's namespace complete by construction and lets the gate close, the argument an
/// `ON CONFLICT` action's scope makes unconditionally and this one makes per level.
/// Judging the key at eval time instead is judging it only when a row reaches the dedup,
/// and there are two ways none does. A projection `unnest` over an **empty** array
/// produces no row, which is the *read* polarity: `SELECT DISTINCT ON (bogusdon)
/// unnest(ARRAY[]::int[]) AS q` answered `(0 rows)` where 18.6 raises `42703`. An
/// **unreferenced CTE**'s body is never executed at all, which is the write polarity and
/// is row-count **independent**: `WITH c AS (SELECT DISTINCT ON (bogusdon)
/// unnest(ARRAY[]::int[]) AS q) DELETE FROM n8` **removed every row** where 18.6 raises
/// `42703` and leaves them, its `UPDATE` twin overwrote them, and the same `WITH` over
/// `unnest(ARRAY[1,2])` or a plain `1 AS q` wrote identically (measured). Reading the
/// class off the read polarity alone gets that backwards.
///
/// **What was already loud is what an *expression* subquery encloses**, and the split is
/// `scope_is_complete`'s: measured against the engine as it stood before this gate, the
/// `EXISTS (…)`, `IN (SELECT …)` and scalar `(SELECT …)` spellings of that key each
/// refused, because the enclosing level's `resolve_correlated_subplans` re-walks them once
/// its chain is whole. The positions where that depth is **zero** were all silent, and
/// only two of them get there by *resetting* it: a **CTE** (referenced or not) and a
/// **derived table**, both of which parse through `Parser::in_from_item`. The rest never
/// raised it at all — a **set-op arm** (which answered the other arm's row), an
/// **`INSERT … SELECT`** source (a silent `INSERT 0 0`), the **paren-leading** spelling,
/// and the top level itself. Those are the members measured, not a closed enumeration of
/// the positions a no-FROM select can sit in. The **qualified**
/// key rode the hole the same way (`DISTINCT ON (bogusrel.z)`, 18.6's `42P01`), and where
/// it *was* loud it carried the wrong code: over a non-empty `unnest` the eval-time
/// resolver answers `42703 column "bogusrel.z" does not exist`, a name error for a
/// relation error.
///
/// Which scope a key is judged against is `correlation::scope_for_key`'s rule and not a
/// second copy of it: a **bare single-field** key reaches `findTargetlistEntrySQL92` and
/// so may name a target-list entry, while a qualified or nested one
/// (`DISTINCT ON (bogusrel.z)`, `DISTINCT ON (q+0)`) resolves against the FROM relations —
/// here, none — and is refused once the scope closes. The keys are judged **in source
/// order, interleaved with the positional range check**, because Postgres resolves them
/// one at a time in `transformDistinctOnClause`: measured on 18.6,
/// `DISTINCT ON (bogusdon, 5)` is `42703` and `DISTINCT ON (5, bogusdon)` is `42P10`, and
/// no arrangement of one whole-list binding pass against one whole-list range pass yields
/// both. The same interleave puts a first key's `42703` ahead of a later
/// `DISTINCT ON (1.5)`'s `42601`, and the whole per-key gate ahead of a **runtime** error
/// in the projection (`SELECT DISTINCT ON (bogusdon) 1/0 AS q` is `42703` on both, and was
/// `22012` here). A projection error 18.6 raises in *parse analysis* still wins, on both
/// engines, because the projection is lowered before this function is called
/// (`SELECT DISTINCT ON (bogusdon) 1 = 'x' AS q` is `22P02` on both).
///
/// `scope_is_complete` is `Parser::scope_is_complete` — false inside an *expression*
/// subquery, where this level may still name the enclosing query's relations and the gate
/// must decline rather than invent a refusal. `SELECT (SELECT DISTINCT ON (ct.a) 1 AS q)
/// FROM ct` answers a row on both engines; that key is re-judged by the enclosing level's
/// `resolve_correlated_subplans` against the chained scope.
///
/// # Errors
///
/// A positional `DISTINCT ON` ordinal outside the select-list range fails loud (`42P10`),
/// a bare key no target-list entry carries is `42703`, and a qualifier no relation answers
/// is `42P01`. Two entries under one name are `42702` — and over a projection that lowers
/// to `Plan::Values` that reads *two entries* rather than *two different expressions*,
/// because `correlation::target_list_exprs` has no `Plan::Values` arm and every identity is
/// then `None`: `SELECT DISTINCT ON (q) 1 AS q, 1 AS q` is `42702` here against 18.6's row.
/// That false rejection is unchanged by this gate — it was the eval-time resolver's before
/// and is the same error under the same message now — and it is
/// `correlation::target_list_exprs`'s own recorded gap, measured there on this very
/// statement.
pub fn plan_no_from_distinct_on(
    plan: Plan,
    distinct_on: Vec<Expr>,
    schema: &Schema,
    scope_is_complete: bool,
) -> Result<Plan> {
    let out_cols = plan.output_columns(schema);
    let out_names: HashSet<String> = out_cols.iter().map(|c| c.name.clone()).collect();
    // No FROM relations, so the base scope is empty — and closed, whenever this level's
    // chain is whole, which is what makes an unbindable key an error rather than an
    // unknown.
    let base = if scope_is_complete {
        ColumnScope::default().closed()
    } else {
        ColumnScope::default()
    };
    let keyscope = key_scope(&base, &plan, schema);
    // Read before anything wraps the plan — the predicate matches a bare `Plan::Values`.
    let sequence_projection = plan.is_sequence_projection();
    // Read before `resolve_output_position` canonicalizes an ordinal into a by-name
    // reference — see [`ResolvedTargets`].
    let out_entry_names: Vec<String> = out_cols.iter().map(|c| c.name.clone()).collect();
    let resolved_targets = ResolvedTargets::from_claims(distinct_on.iter().map(|e| {
        sql92_target_index(e, &out_entry_names)
            .or_else(|| sql99_target_index(e, single_row_targets(&plan)))
    }));
    let on_exprs = distinct_on
        .into_iter()
        .map(|e| {
            let key = resolve_output_position(e, &out_cols, "DISTINCT ON")?;
            check_key_bindings(&key, scope_for_key(&key, &base, &keyscope))?;
            Ok(canonicalize_distinct_key(
                key,
                &[],
                &out_names,
                &ColumnScope::default(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    if sequence_projection {
        return Ok(plan);
    }
    // No ORDER BY reaches a no-FROM leaf, so sort by the ON key itself (ASC) — enough
    // to make equal-ON rows adjacent for the first-per-group dedup.
    let order_by: Vec<OrderKey> = on_exprs
        .iter()
        .map(|e| OrderKey::new(e.clone(), true))
        .collect();
    let sorted = Plan::Sort {
        input: Box::new(plan),
        order_by,
        resolved_targets,
    };
    Ok(Plan::DistinctOn {
        on_exprs,
        input: Box::new(sorted),
    })
}

/// Apply a **no-FROM** select's trailing `ORDER BY` / `LIMIT` / `OFFSET`
/// (`SELECT 1 AS q ORDER BY q LIMIT 1`), the tail `parser::select`'s `SelectLeaf::NoFrom`
/// arm used to parse and throw away.
///
/// The discard was three divergences, not one, and the two silent ones are the reason
/// this function exists. Measured on both engines: `SELECT 1 AS q LIMIT 0` and
/// `SELECT 1 AS q OFFSET 1` each answered a row here against `(0 rows)` on 18.6, **with
/// no error on either engine**, so the same select inside a write moved rows —
/// `DELETE FROM w1 WHERE a IN (SELECT 1 AS q LIMIT 0)` removed one 18.6 keeps,
/// `UPDATE w2 SET a=99 WHERE a IN (SELECT 1 AS q OFFSET 1)` overwrote one, and
/// `INSERT INTO w3 SELECT 7 AS q LIMIT 0` invented one. The `ORDER BY` half was at least
/// loud there (`ORDER BY 5` is `42P10`, `ORDER BY zzz` is `42703`, `ORDER BY bogusrel.a`
/// is `42P01`) and answered a row here, and it wrote too — measured through a `DELETE`
/// (`… WHERE a NOT IN (SELECT 1 AS q ORDER BY zzz)`, `DELETE 2` against three surviving
/// rows there) and through the unreferenced-CTE `DELETE`/`UPDATE` pair, which is the
/// row-count-independent polarity.
///
/// **Routing the tail does not by itself judge the keys**, which is where the record this
/// closes was expecting more than it measured: a `Plan::Sort` over a `Plan::Values` judges
/// its key only when a row reaches the eval-time resolver, so the empty-projection read
/// (`SELECT unnest(ARRAY[]::int[]) AS q ORDER BY zzz`) and the unreferenced-CTE write
/// (`WITH c AS (SELECT 1 AS q ORDER BY zzz) DELETE FROM n8`, whose body never runs) would
/// both still be silent. So the keys are bound **here**, at plan time, by the same
/// machinery [`plan_no_from_distinct_on`] uses for a no-FROM `DISTINCT ON` key: a bare
/// single-field key against the target-list claimants
/// ([`super::correlation::key_scope`]), every other spelling against the FROM scope,
/// which for a no-FROM select is empty — [`super::correlation::scope_for_key`]'s split,
/// not a second copy of it.
///
/// The binding and the positional range check **interleave per key**, in source order, as
/// 18.6 resolves them: measured there, `ORDER BY zzz, 9` is `42703` and `ORDER BY 9, zzz`
/// is `42P10`, and no arrangement of one whole-list binding pass against one whole-list
/// range pass yields both.
///
/// `scope_is_complete` is `Parser::scope_is_complete` and is load-bearing in the same way
/// it is for the `DISTINCT ON` gate: a no-FROM select inside an *expression* subquery may
/// name the enclosing query's relations, and 18.6 answers it —
/// `SELECT (SELECT 1 AS q ORDER BY ct.a) AS r FROM ct` is a row on both engines. There the
/// scope stays open, `check_binding` answers `Uncertain`, and the key is re-judged by the
/// enclosing level's `resolve_correlated_subplans` through `walk_level`'s `Plan::Sort` arm.
///
/// **A top-level sequence projection keeps the discard**, and that exclusion is a plan
/// *shape* fact rather than a leniency: the executor dispatches
/// `SELECT nextval/currval/setval(…)` on [`Plan::is_sequence_projection`] — the single
/// definition of that predicate, which matches a bare `Plan::Values` — so wrapping one in
/// a `Sort`/`Limit` routes it to the read-only eval path, where a `SequenceFn` fails loud
/// (`0A000`) and an accepted statement becomes an error. The gates above still run, so
/// nothing loud becomes silent; only the bound and the ordering are not applied.
/// Measured, and left open in `EDGES.md`:
/// `SELECT nextval('s') LIMIT 0` answers a row here against `(0 rows)` there — and 18.6
/// does not advance the counter for it, where each pglet spelling measured (`LIMIT 0`,
/// `OFFSET 1`, `ORDER BY 1`, `DISTINCT`) does — while `SELECT nextval('s') OFFSET 1` is
/// `(0 rows)` there **with** the counter advanced. Two bounds on that divergence, both
/// measured and both in the record: it is not confined to the outermost select (a
/// **referenced CTE body** carries it, and so does a projection merely *carrying* a
/// sequence function beside other expressions — the predicate matches on `any`), and the
/// **write** spellings measured are each loud here, so the wrong row count stays inside a
/// read.
///
/// # Errors
///
/// A positional ordinal outside the select-list range is `42P10` and a bare non-integer
/// constant key `42601` ([`positional_key_index`]); a bare key no target-list entry
/// carries is `42703`, a qualifier no relation answers `42P01`, and two entries under one
/// name `42702` — the last a **widening** of `correlation::target_list_exprs`' recorded
/// `Plan::Values` gap to the `ORDER BY` spelling: `SELECT 1 AS q, 1 AS q ORDER BY q` is
/// `42702` here against 18.6's row, as its `DISTINCT ON` twin already was.
pub fn plan_no_from_order_limit(
    plan: Plan,
    order_by: Vec<OrderKey>,
    limit: Option<(Option<LimitCount>, Option<LimitCount>)>,
    schema: &Schema,
    scope_is_complete: bool,
) -> Result<Plan> {
    let mut plan = plan;
    // Read before anything wraps the plan — the predicate matches a bare `Plan::Values`.
    let sequence_projection = plan.is_sequence_projection();
    if !order_by.is_empty() {
        let out_cols = plan.output_columns(schema);
        let out_entry_names: Vec<String> = out_cols.iter().map(|c| c.name.clone()).collect();
        let resolved_targets = ResolvedTargets::from_claims(order_by.iter().map(|k| {
            sql92_target_index(&k.expr, &out_entry_names)
                .or_else(|| sql99_target_index(&k.expr, single_row_targets(&plan)))
        }));
        let base = if scope_is_complete {
            ColumnScope::default().closed()
        } else {
            ColumnScope::default()
        };
        let keyscope = key_scope(&base, &plan, schema);
        let resolved = order_by
            .into_iter()
            .map(|key| {
                // The ordinal keeps its literal form (`eval_order_key` resolves it by
                // index), so only its range is checked here.
                positional_key_index(&key.expr, out_cols.len(), "ORDER BY")?;
                check_key_bindings(&key.expr, scope_for_key(&key.expr, &base, &keyscope))?;
                Ok(key)
            })
            .collect::<Result<Vec<_>>>()?;
        if !sequence_projection {
            plan = Plan::Sort {
                input: Box::new(plan),
                order_by: resolved,
                resolved_targets,
            };
        }
    }
    if let Some((limit_val, offset_val)) = limit
        && !sequence_projection
    {
        plan = Plan::Limit {
            input: Box::new(plan),
            limit: limit_val,
            offset: offset_val,
        };
    }
    Ok(plan)
}

/// Place the `Plan::Sort` for a non-index-elided ORDER BY either above or below the
/// projection, so a key on a column the projection drops is still honored.
///
/// A `Plan::Sort` evaluates its keys against its **input's output** columns. With
/// the Sort above a plain projection (the natural shape), a key on a non-projected
/// base column (`SELECT id … ORDER BY created_at` — the spine's eviction subquery)
/// resolves to nothing and the sort silently no-ops. The fix is to sort the
/// **pre-projection** rows (which still carry every column) and project afterward —
/// no column is lost, and projecting is row-order-preserving.
///
/// **The only constraint on sort-below is that every key be *readable* from the base
/// row**, and there are two ways to read one. A key whose column references are base
/// columns reads them directly. A key the *output* answers — a positional ordinal, or
/// a whole-key simple name matching an output column — reads instead through the
/// projection expression that produces that column, which is itself defined over base
/// columns; [`rewrite_output_key_to_projection`] carries it down. So the two kinds mix
/// freely: `SELECT x … ORDER BY x, a` sorts below by the projection's `x` and the
/// dropped base `a` together, which is the shape (an ordinary tie-break on a dropped
/// column, and the `DISTINCT ON … ORDER BY <key>, <other>.<ts>` latest-row-per-group
/// idiom) that a "no key may read the output" rule wrongly rejected.
///
/// Sort-below is *taken* only when it buys something: some key must reference a base
/// column the output does not carry, or sort-above already sees everything and stays
/// the natural shape — where the projected output (a bare `*` passthrough, computed
/// aliases) is visible.
///
/// **Only a plain `Plan::Project` is reordered.** For Distinct that is correct by
/// construction: `SELECT DISTINCT id … ORDER BY txt` is an error in Postgres too
/// (`for SELECT DISTINCT, ORDER BY expressions must appear in select list`), so a
/// legal DISTINCT query's keys are always output expressions.
///
/// **An Aggregate keeps the sort above it and pays for a dropped key with a junk
/// column instead.** Its rows have already collapsed, so there is no base row left to
/// sort below — Postgres's own answer is a `resjunk` target-list entry, and
/// [`resolve_aggregate_order_keys`] builds one for every key the Aggregate's output
/// cannot answer: a grouped-but-unprojected column (`SELECT count(*) FROM t GROUP BY
/// txt ORDER BY txt`) and an aggregate the select list does not carry
/// (`ORDER BY count(*)`, `ORDER BY min(v)`). The junk columns ride the Aggregate's
/// own expression list, so the returned [`SortedPlan`] carries the width its caller
/// must trim back to. An **ungrouped** column stays Postgres's own `42803`.
///
/// **A Window takes the same treatment for the same reason**, through
/// [`resolve_window_order_keys`] — its functions are computed over the input order, so
/// the Sort cannot go below it either. It has no grouping rule to enforce, which is the
/// whole difference between the two branches.
///
/// # Errors
///
/// An ORDER BY key over an aggregated query that references an ungrouped base column
/// fails loud (`42803`); see [`resolve_aggregate_order_keys`].
fn build_sort(
    plan: Plan,
    order_by: Vec<OrderKey>,
    scope: &ColumnScope,
    resolved_targets: &ResolvedTargets,
) -> Result<SortedPlan> {
    // An Aggregate always keeps the Sort above it (rows have already collapsed), so
    // its keys are resolved against the Aggregate's own output up front — and the
    // ones that output cannot answer become junk columns on the Aggregate itself.
    let (plan, order_by, junk_keep) = if let Plan::Aggregate {
        input,
        group_by,
        exprs,
        // The Aggregate's own `GROUP BY` claims, carried through untouched — a different
        // clause's claims from the `resolved_targets` parameter, which is this level's
        // sort keys'.
        resolved_targets: group_targets,
        having,
        certain,
        enclosing,
    } = plan
    {
        let (order_by, junk) = resolve_aggregate_order_keys(order_by, &group_by, &exprs, scope)?;
        let keep = (!junk.is_empty()).then_some(exprs.len());
        let mut exprs = exprs;
        exprs.extend(junk);
        (
            Plan::Aggregate {
                input,
                group_by,
                exprs,
                resolved_targets: group_targets,
                having,
                certain,
                enclosing,
            },
            order_by,
            keep,
        )
    } else if let Plan::Window {
        input,
        window_exprs,
        certain,
        enclosing,
    } = plan
    {
        // A Window is the other node the Sort cannot go below: its functions are
        // computed over the *input* order, so reordering the input changes their
        // values. Its keys take junk columns for the same reason the Aggregate's do.
        let (order_by, junk) = resolve_window_order_keys(order_by, &window_exprs);
        let keep = (!junk.is_empty()).then_some(window_exprs.len());
        let mut window_exprs = window_exprs;
        window_exprs.extend(junk);
        (
            Plan::Window {
                input,
                window_exprs,
                certain,
                enclosing,
            },
            order_by,
            keep,
        )
    } else {
        (plan, order_by, None)
    };

    let sort_above = |inner: Plan| {
        Ok(SortedPlan {
            plan: Plan::Sort {
                input: Box::new(inner),
                order_by: order_by.clone(),
                resolved_targets: resolved_targets.clone(),
            },
            junk_keep,
        })
    };

    let Plan::Project {
        input,
        exprs,
        certain,
        enclosing,
    } = plan
    else {
        return sort_above(plan);
    };
    // A bare `*` projects every column — the post-projection Sort sees them all.
    if exprs.iter().any(Expr::is_bare_star) {
        return sort_above(Plan::Project {
            input,
            exprs,
            certain,
            enclosing,
        });
    }

    let output_names: HashSet<String> = exprs.iter().map(Expr::output_name).collect();
    // Classify each key by *where the Sort must read it from*. Two kinds:
    //
    // - A key the **output** answers — a whole-key simple name matching an output
    //   column (`whole_key_output_name`), or a positional ordinal. Below the
    //   projection it is read by evaluating the projection expression that produces
    //   that column, which is defined over base columns; the rewrite below does that.
    // - Everything else, whose column references must each be readable from the base
    //   row (qualifier-aware, so `pb.x` asks relation `pb` and is not answered by a
    //   sibling `pa.x`).
    //
    // Sort-below is legal when every reference is readable that way, and *wanted*
    // when some reference is a base column the output does not carry (`SELECT x …
    // ORDER BY x, a`) — otherwise the natural sort-above shape already sees
    // everything. Mixing the two kinds is fine: it is only *readability* from the
    // base row that constrains the position, and the output-answered keys carry
    // their projection expression down with them.
    let mut needs_a_base_column = false;
    let mut every_key_is_readable = true;
    for key in &order_by {
        if let Some(name) = whole_key_output_name(&key.expr, &output_names) {
            // Two projection entries sharing an output name give the rewrite no one
            // expression to carry down — decline and let the sort-above resolver
            // render its own verdict on the ambiguity.
            if exprs.iter().filter(|e| e.output_name() == name).count() != 1 {
                every_key_is_readable = false;
            }
            continue;
        }
        // Not a whole bare name, so SQL92 does not reach it and **every** reference
        // inside it — qualified or bare — is an ordinary column reference resolved
        // against the FROM relations. Only `Plan::Project` reaches this branch; the
        // other two nodes that can top a query take the same rule one layer up, by
        // junk column rather than by sort placement (`resolve_aggregate_order_keys`,
        // `resolve_window_order_keys`). Reading a nested bare name off the output
        // instead is a different value whenever the projection renames or computes
        // it: measured over `tt(q, a)`, `SELECT a AS q FROM tt ORDER BY q+0` is
        // `200 300 100` on 18.6 (the base `q`) and was `100 200 300` here (the
        // output `q`), with no error on either engine.
        let mut referenced = Vec::new();
        collect_referenced_columns(&key.expr, &mut referenced);
        for (qualifier, name) in &referenced {
            let (qualifier, name) = (qualifier.as_deref(), name.as_str());
            if scope.contains_column_ref(qualifier, name) {
                needs_a_base_column = true;
            } else {
                every_key_is_readable = false;
            }
        }
    }
    let sort_below = needs_a_base_column && every_key_is_readable;

    if sort_below {
        // Sort BELOW the projection: the Sort sees the *base* (pre-projection) row,
        // so a key the sort-above layout would have answered from the output — a
        // positional `ORDER BY n`, or a bare name matching an output column — must be
        // rewritten to the projection expression producing that column, which is
        // defined over base columns. Every other key already reads base columns.
        let order_by = order_by
            .into_iter()
            .map(
                |OrderKey {
                     expr,
                     asc,
                     nulls_first,
                 }| OrderKey {
                    expr: rewrite_output_key_to_projection(expr, &exprs, &output_names),
                    asc,
                    nulls_first,
                },
            )
            .collect();
        Ok(SortedPlan {
            plan: Plan::Project {
                input: Box::new(Plan::Sort {
                    input,
                    order_by,
                    resolved_targets: resolved_targets.clone(),
                }),
                exprs,
                certain,
                enclosing,
            },
            junk_keep,
        })
    } else {
        sort_above(Plan::Project {
            input,
            exprs,
            certain,
            enclosing,
        })
    }
}

/// A sort built by [`build_sort`], plus the junk trim its keys made necessary.
///
/// The two travel together because the trim is not [`build_sort`]'s to place: a
/// `DISTINCT ON` reads its own key **above** the `Sort`, so the [`Plan::JunkFilter`]
/// has to go above the dedup too, and only the caller knows what else it is about to
/// stack on.
struct SortedPlan {
    plan: Plan,
    /// `Some(n)` when a key needed a junk column: the query's real output is the
    /// leading `n` columns, and everything after them is junk to be trimmed.
    junk_keep: Option<usize>,
}

impl SortedPlan {
    /// Stack a node that still reads the junk columns on top, keeping the trim owed.
    fn map(self, f: impl FnOnce(Plan) -> Plan) -> Self {
        Self {
            plan: f(self.plan),
            junk_keep: self.junk_keep,
        }
    }

    /// The plan with its junk columns trimmed — every consumer of them having been
    /// stacked on first.
    fn trimmed(self) -> Plan {
        match self.junk_keep {
            Some(keep) => Plan::JunkFilter {
                input: Box::new(self.plan),
                keep,
            },
            None => self.plan,
        }
    }
}

/// Resolve the ORDER BY keys of an **aggregated** query against the Aggregate's
/// output, enforcing Postgres's grouping rule along the way, and build a **junk
/// column** for every key that output cannot answer.
///
/// The Sort sits **above** the Aggregate, whose output carries only the
/// grouped/aggregated expressions under their bare output names — so a key the
/// output *can* answer has to be found there, and the executor's last-resort
/// bare/suffix match would otherwise answer a qualified `pb.x` with the projected
/// `pa.x` and sort by the wrong column entirely. Each column reference is therefore
/// resolved to the output column it *denotes* ([`output_key_identity`], which
/// compares by target-entry identity and is qualifier-aware) and rewritten to that
/// column's bare name, so the binding is exact rather than accidental.
///
/// A key the output cannot answer is Postgres's `resjunk` target-list entry
/// (`transformSortClause` → `addTargetToSortList`): the whole key rides the Aggregate
/// as an extra output column, evaluated there against the base rows like any other
/// select-list expression, and the Sort above reads it by that column's name. Two
/// kinds reach it, and both are ordinary SQL:
///
/// - **A grouped column the select list drops** — `SELECT count(*) FROM t GROUP BY k
///   ORDER BY k`, and every expression over one (`ORDER BY k*2`, `ORDER BY k DESC`).
/// - **An aggregate the select list drops** — `ORDER BY count(*)`, `ORDER BY min(v)`,
///   `ORDER BY sum(v)/count(*)`. An aggregate the select list *does* carry is not junk:
///   Postgres reuses that target entry, and branch (c) of [`output_key_identity`] is
///   that reuse.
///
/// The junk decision is **whole-key**, and that is not a simplification: a key with
/// one junk reference is evaluated inside the Aggregate, where an output alias means
/// nothing, so no part of it may have been rewritten to one. The resolution is
/// therefore attempted first and *discarded entirely* when any part of it comes back
/// [`None`] — the original key is what becomes junk.
///
/// An **ungrouped** column stays loud: once a query groups, every column outside an
/// aggregate must be a grouping column (Postgres's own `42803`,
/// `substitute_grouped_columns_mutator` in `parse_agg.c`). That check runs over the
/// junk key too, which is Postgres putting `parseCheckAggregates` over the whole
/// target list, junk entries included.
///
/// A key matching a `GROUP BY` key whole (`GROUP BY a+b ORDER BY a+b`) is valid
/// however it is built and passes through structurally intact. Aggregate arguments
/// are skipped: `ORDER BY count(pb.x)` aggregates `pb.x` away, so neither the
/// grouping rule nor the output-column requirement reaches it.
///
/// # Errors
///
/// [`Error::GroupingError`] (`42803`) for a key on an ungrouped base column.
fn resolve_aggregate_order_keys(
    order_by: Vec<OrderKey>,
    group_by: &[Expr],
    exprs: &[Expr],
    scope: &ColumnScope,
) -> Result<(Vec<OrderKey>, Vec<Expr>)> {
    let output_names: HashSet<String> = exprs.iter().map(Expr::output_name).collect();
    let mut junk = Vec::new();
    let mut taken = output_names.clone();
    taken.extend(clause_key_names(order_by.iter().map(|k| k.expr.clone())));
    let keys = order_by
        .into_iter()
        .map(|key| {
            let resolved = resolve_aggregate_order_expr(
                key.expr.clone(),
                group_by,
                exprs,
                &output_names,
                scope,
                true,
            )?;
            let expr = if let Some(expr) = resolved {
                expr
            } else {
                mint_junk_column(key.expr, &mut junk, &mut taken)
            };
            Ok(OrderKey { expr, ..key })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((keys, junk))
}

/// Resolve the ORDER BY keys of a **windowed** query against the Window's output,
/// building a junk column for every key that output cannot answer.
///
/// The Sort sits above the Window and cannot go below it: a window function is
/// computed over the *input* order, so reordering the input changes the values being
/// sorted. That is why the `Plan::Project` sort-below rewrite does not generalize here
/// and Postgres's `resjunk` entry does — the key rides the Window's own target list.
///
/// The rule is [`findTargetlistEntrySQL92`'s](https://github.com/postgres/postgres/blob/master/src/backend/parser/parse_clause.c)
/// one-field gate and nothing else, because a Window has no grouping rule to enforce:
/// a **positional** ordinal and a **whole bare name matching an output column** are
/// answered by the output, and every other key — a qualified reference, a nested name,
/// an expression, a window function — resolves against the FROM relations and becomes
/// junk. All three of the non-output kinds were wrong before, and two silently:
/// measured over `zz(a, b) = (1,5),(1,7),(2,5)`, `SELECT b AS a, row_number() OVER ()
/// FROM zz ORDER BY a+0` and `… ORDER BY zz.a` both answered the *output* `a` (which is
/// `b`) where 18.6 answers the base `zz.a`, and `SELECT b, row_number() OVER () FROM zz
/// ORDER BY a` was `42703` where 18.6 answers rows.
///
/// The positional test is spelled inline rather than through
/// [`positional_key_index`]: the *range* check has already run over every key against
/// this same output count (`plan_select`'s ORDER BY path, and `resolve_output_position`
/// on the `DISTINCT ON` path), so all that is left to ask here is which keys the output
/// answers. A non-integer bare constant (`ORDER BY 'lit'`) is `42601` from that earlier
/// check and never reaches this walk.
///
/// **Divergence, unchanged and recorded:** an *aggregate* in the key
/// (`SELECT b, row_number() OVER () FROM zz ORDER BY count(*)`) makes the statement an
/// aggregated query on 18.6, which then reports `42803` for the ungrouped `b`; here the
/// junk column carries the aggregate into the Window, which refuses it (`0A000`). Loud
/// either way, different code.
fn resolve_window_order_keys(
    order_by: Vec<OrderKey>,
    window_exprs: &[Expr],
) -> (Vec<OrderKey>, Vec<Expr>) {
    let output_names: HashSet<String> = window_exprs.iter().map(Expr::output_name).collect();
    let mut junk = Vec::new();
    let mut taken = output_names.clone();
    taken.extend(clause_key_names(order_by.iter().map(|k| k.expr.clone())));
    let keys = order_by
        .into_iter()
        .map(|key| {
            let answered_by_output = matches!(&key.expr, Expr::Literal(Literal::Int(_)))
                || whole_key_output_name(&key.expr, &output_names).is_some();
            if answered_by_output {
                return key;
            }
            let expr = mint_junk_column(key.expr, &mut junk, &mut taken);
            OrderKey { expr, ..key }
        })
        .collect();
    (keys, junk)
}

/// Error when any column reference inside a `DISTINCT ON` key binds to nothing.
///
/// **Every caller decides for itself which keys to hand here and against which scope**,
/// because a key that names an *output* column may name something no FROM relation
/// carries — an alias (`SELECT v AS lbl … DISTINCT ON (lbl)`) is exactly that, and gating
/// it against the FROM scope alone would refuse ordinary SQL. There are three callers and
/// they answer that differently, for the same reason:
///
/// - [`lower_select_level`]'s `DISTINCT ON` block calls it only for a key the projection
///   cannot answer, handing it that query's FROM scope — a key the projection *does*
///   answer needs no check at all;
/// - [`plan_no_from_distinct_on`] and [`plan_no_from_order_limit`] call it for **every**
///   key — the `DISTINCT ON` keys and the trailing `ORDER BY` keys of a no-FROM select
///   respectively — handing a bare single-field one a scope carrying the target list as an
///   explicit claimant set (`correlation::key_scope`) and every other spelling the FROM
///   scope, which for a no-FROM select is empty. That is
///   `findTargetlistEntrySQL92`'s own split, and there is no projection-answers-it
///   shortcut there because nothing else ever judges those keys.
///
/// A no-op in an **open** scope (one inside an expression subquery, whose FROM namespace
/// is not yet whole): there the key is re-gated by `resolve_correlated_subplans` once the
/// enclosing level exists — for the first caller through the junk column's own expression
/// on the projecting node, for the second through `walk_level`'s `Plan::DistinctOn` arm,
/// and for the third through its `Plan::Sort` arm. A level no such re-walk reaches has no
/// second reader, and the one such position left is the *top* level, whose scope was never
/// open. A **no-FROM set-op leaf** used to be a
/// second — `parser::select`'s `finalize_leaf` ran no `resolve_correlated_subplans` where
/// its `parse_select_inner` twin does, so a level that leaf enclosed kept its open scope
/// and nothing re-read it; `finalize_leaf` makes the call now, and the enumeration is a
/// claim about the callers of `resolve_correlated_subplans` rather than about this
/// function.
///
/// # Errors
///
/// `42P01` for a qualifier no relation answers, `42703` for a name none carries, `42702`
/// for a bare name two claimants of one level carry.
fn check_key_bindings(key: &Expr, scope: &ColumnScope) -> Result<()> {
    let mut referenced = Vec::new();
    collect_referenced_columns(key, &mut referenced);
    for (qualifier, name) in &referenced {
        scope.check_binding(qualifier.as_deref(), name)?;
    }
    Ok(())
}

/// Every column name the keys of one clause mention, at any depth and under any
/// qualifier, so a junk column cannot be minted onto a name one of them spells.
///
/// The qualifier is dropped deliberately: the junk column's own name is bare, and the
/// executor's resolver has a loose tier that answers a qualified reference from a bare
/// column, so a qualified `t.n` has to reserve `n` too.
fn clause_key_names(keys: impl IntoIterator<Item = Expr>) -> HashSet<String> {
    let mut names = HashSet::new();
    for key in keys {
        let mut referenced = Vec::new();
        collect_referenced_columns(&key, &mut referenced);
        names.extend(referenced.into_iter().map(|(_, name)| name));
    }
    names
}

/// Move `key` onto the node below as a junk column, and return the reference that
/// reads it back.
///
/// Shared by the three minters — the Aggregate branch, the Window branch and the
/// `DISTINCT ON` key rewrite — so they cannot disagree about how a junk column is
/// named, aliased, or referenced. Seeding `taken` is the caller's, because what a name
/// must avoid is the caller's clause.
fn mint_junk_column(key: Expr, junk: &mut Vec<Expr>, taken: &mut HashSet<String>) -> Expr {
    let name = junk_column_name(taken);
    junk.push(Expr::Alias {
        expr: Box::new(key),
        alias: name.clone(),
    });
    Expr::Column { table: None, name }
}

/// A name for one junk column, distinct from every name already in `taken` — the
/// node's own output names, every name the clause's own keys mention, and the junk
/// names minted before it — and recorded there.
///
/// Distinctness is what the Sort's by-name read of the junk column rests on, and it is
/// **checked against the actual name list** rather than assumed from an exotic
/// spelling: no name is unreachable, because a user can quote any identifier into
/// existence. Two collisions matter and they are different. A real **output column**
/// wearing the generated name would answer in the junk column's place, since the
/// resolver's exact tier takes the first match. And a **key** naming it would answer
/// off the junk column instead of failing: a key the pre-junk output cannot resolve is
/// left for the executor to judge, and the executor sees the row *after* the junk
/// column is on it — measured, `SELECT count(*) FROM g GROUP BY k ORDER BY k,
/// pglet_junk_0` answered rows where 18.6 is `42703`. Seeding both is what
/// [`clause_key_names`] is for.
fn junk_column_name(taken: &mut HashSet<String>) -> String {
    let mut n = 0;
    loop {
        let candidate = format!("pglet_junk_{n}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Resolve one ORDER BY expression of an aggregated query against the Aggregate's
/// output — see [`resolve_aggregate_order_keys`].
///
/// [`None`] means the output cannot answer it and the **whole key** must become a junk
/// column; it propagates up through the walk for exactly that reason.
///
/// # Errors
///
/// [`Error::GroupingError`] (`42803`) for a reference to an ungrouped base column.
fn resolve_aggregate_order_expr(
    expr: Expr,
    group_by: &[Expr],
    exprs: &[Expr],
    output_names: &HashSet<String>,
    scope: &ColumnScope,
    whole_key: bool,
) -> Result<Option<Expr>> {
    crate::depth::check_stack()?;
    // An aggregate consumes the columns inside it — do not descend. The select list
    // may carry this very aggregate, in which case Postgres reuses that target entry
    // (branch (c)); otherwise it is a junk column.
    if matches!(expr, Expr::Aggregate { .. }) {
        return Ok(output_key_identity(
            &expr,
            exprs,
            output_names,
            scope,
            whole_key,
        ));
    }
    if let Expr::Column { table, name } = &expr {
        if name == "*" {
            return Ok(Some(expr));
        }
        // The output column this reference denotes, by target-entry identity — the
        // exact binding, in place of the resolver's bare/suffix fallback.
        if let Some(out) = output_key_identity(&expr, exprs, output_names, scope, whole_key) {
            return Ok(Some(out));
        }
        // Not a base column of any FROM relation — an output alias or an unknown
        // name; leave the verdict to the resolver, as the projection's own grouping
        // check does.
        if !scope.contains_column_ref(table.as_deref(), name) {
            return Ok(Some(expr));
        }
        let grouped = group_by.iter().any(|g| {
            matches!(g, Expr::Column { table: gt, name: gn }
                if gn == name && (table.is_none() || gt.is_none() || gt == table))
        });
        if grouped {
            return Ok(None);
        }
        return Err(Error::GroupingError {
            column: table
                .as_ref()
                .map_or_else(|| name.clone(), |t| format!("{t}.{name}")),
        });
    }
    // A non-column key matching a GROUP BY key whole is valid however it is built,
    // and must keep its structure to go on comparing equal to that key.
    if group_by.contains(&expr) {
        return Ok(Some(expr));
    }
    let mut failure = None;
    let mut needs_junk = false;
    let mapped = expr.map_children(|child| {
        // `false`: a node inside the key is not the key, so SQL92's output-name
        // branch does not reach it (`key_may_name_an_output_column`).
        match resolve_aggregate_order_expr(
            child.clone(),
            group_by,
            exprs,
            output_names,
            scope,
            false,
        ) {
            Ok(Some(resolved)) => resolved,
            Ok(None) => {
                needs_junk = true;
                child
            }
            Err(e) => {
                failure.get_or_insert(e);
                child
            }
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    Ok((!needs_junk).then_some(mapped))
}

/// In a sort-**below**-projection layout, rewrite a key the sort-**above** layout
/// would have answered from the projection's *output* into the projection expression
/// that produces that output column, stripped of its `AS` alias so it evaluates
/// against the base (pre-projection) row the Sort now sees.
///
/// Two such keys exist. A **positional** `ORDER BY n` (a bare integer literal) can no
/// longer be resolved by output index down here; the range was validated at lowering
/// time and the sort-below branch never sees a bare `*` (it early-returns
/// sort-above), so an in-range `n` always indexes `exprs`. A **whole-key simple name**
/// matching an output column ([`whole_key_output_name`]) would otherwise bind to a
/// same-named *base* column, which is a different value whenever the projection
/// computes or renames it (`SELECT b AS a … ORDER BY a, c` must sort by `b`).
///
/// Any other key — a qualified reference, a compound expression, a bare name with no
/// output column — already reads base columns and is returned unchanged.
///
/// **A carried-down entry that is itself a bare integer literal has to be shielded**,
/// because a bare integer literal *is* the plan's spelling of a positional reference
/// (`eval_order_key` resolves one by index against the row it sees) — and the row the
/// Sort sees down here is the **base** row. Carrying `SELECT 't', 5 … ORDER BY 2` down
/// raw made the Sort read base column 5: `42P10` over a narrow source, and a silently
/// different row order over a wide one (`SELECT 2 AS z, b FROM s3 ORDER BY 1, a`
/// answered `10, 20, 30` here against 18.6's `10, 30, 20`, with no error on either
/// engine). An `Expr::Alias` is the shield because the evaluator forwards through one
/// and the executor's `eval_order_key` does not treat it as positional; every other
/// entry shape already evaluates as itself.
fn rewrite_output_key_to_projection(
    key: Expr,
    exprs: &[Expr],
    output_names: &HashSet<String>,
) -> Expr {
    if let Expr::Literal(Literal::Int(n)) = key {
        return usize::try_from(n)
            .ok()
            .filter(|&i| i >= 1 && i <= exprs.len())
            .map_or(key, |i| carried_key(&exprs[i - 1]));
    }
    let Some(name) = whole_key_output_name(&key, output_names).map(ToString::to_string) else {
        return key;
    };
    exprs
        .iter()
        .find(|e| e.output_name() == name)
        .map_or(key, carried_key)
}

/// One projection entry as a sort key evaluated against the **base** row: its `AS`
/// alias stripped, and a bare integer literal re-wrapped so it is not read as a
/// positional reference. See [`rewrite_output_key_to_projection`].
fn carried_key(entry: &Expr) -> Expr {
    let name = entry.output_name();
    match strip_alias(entry.clone()) {
        literal @ Expr::Literal(Literal::Int(_)) => Expr::Alias {
            expr: Box::new(literal),
            alias: name,
        },
        other => other,
    }
}

/// Whether an `ORDER BY` key binds a **base column** — the precondition the index
/// pushdown's scan rewrite assumes and does not itself test.
///
/// A key that is not a bare name binds a base column by definition (SQL92's
/// output-name branch needs [`key_may_name_an_output_column`]'s shape). A bare name
/// binds one unless it matches an output column, and even then it still does when the
/// projection entry carrying that name *denotes that very base column* — the ordinary
/// `SELECT a FROM t ORDER BY a`, where output and base are the same value and the
/// index walk is order-correct.
///
/// Deliberately narrow: a **qualified** key is left alone. It always resolves against
/// its named relation, so the pushdown's existing behaviour is already the binding
/// Postgres gives it (measured agreeing on both engines), and widening this to
/// re-check the qualifier against the scan's table is a separate question from the
/// output-name one this exists to answer.
fn order_key_binds_a_base_column(key: &Expr, plan: &Plan, schema: &Schema) -> bool {
    let Expr::Column { table: None, name } = key else {
        return true;
    };
    let out_cols = plan.output_columns(schema);
    if !out_cols.iter().any(|c| &c.name == name) {
        return true;
    }
    // A sole bare `*` is left unexpanded for the executor's fast path
    // (`expand_wildcards`), so the projection list is the star itself and carries no
    // entry to match — while the output it stands for is precisely the base columns,
    // each under its own name. Reading the list literally here declined the pushdown
    // for `SELECT * FROM t ORDER BY <indexed col>`, which is most of what it exists
    // for. **No fixture can see that**: declining only keeps the `Sort`, so every row
    // is still correct and only the plan differs — it is the plan-shape unit tests in
    // `planner::tests` that hold this arm.
    if plan.projection_exprs().iter().any(Expr::is_bare_star) {
        return true;
    }
    plan.projection_exprs().iter().any(|p| {
        let (inner, out_name) = match p {
            Expr::Alias { expr: inner, alias } => (inner.as_ref(), alias.clone()),
            other => (other, other.output_name()),
        };
        out_name == *name && matches!(inner, Expr::Column { name: pname, .. } if pname == name)
    })
}

/// The target-list **position** a key names under Postgres's `findTargetlistEntrySQL92`
/// — a positional ordinal, or a bare single-field name matching an output column — or
/// `None` for a key that names none.
///
/// The two spellings are the only ones that reach a target-list entry: a qualified
/// reference and a computed key are ordinary expressions resolved against the range
/// table, which is why `SELECT 't' FROM t ORDER BY n` is accepted on both engines while
/// `ORDER BY 1` is `42804`.
///
/// **A name matching more than one output column names none of them here**, which is
/// the accept-preferring answer rather than a faithful one — 18.6 raises `42702` for a
/// duplicated output alias. **Nothing observable rides on it today, in either
/// direction**, and both halves are measured against 18.6 rather than argued. The
/// acceptance the leniency would buy is pre-empted: `INSERT INTO t (b, i) SELECT
/// DISTINCT ON (2) 't', 5 FROM s` is accepted there and `42702 column reference
/// "?column?" is ambiguous` here, from the duplicate-output-entry gate upstream of this
/// one (the ordinal is canonicalized to a by-name reference, and the two entries are
/// both `?column?`). The `42702` it declines to raise is raised anyway: `INSERT INTO t
/// (b, i) SELECT 't' AS z, 5 AS z FROM s ORDER BY z` is `42702` on **both** engines,
/// and so is the plain `SELECT` spelling. It is written this way because the
/// alternative — marking the first match — is the one polarity that can write a row
/// 18.6 refuses.
///
/// **The "raised anyway" is itself only half-faithful, and the claim above rests on
/// it.** 18.6's `findTargetlistEntrySQL92` exempts duplicate matches whose expressions
/// are `equal()`: `SELECT 't' AS z, 't' AS z FROM s ORDER BY z` answers rows there and
/// is `42702` here, while the `GROUP BY z` spelling of the same statement is correct
/// here — the GROUP BY claimant path carries the exemption and this one does not. A
/// loud pre-existing over-rejection, recorded because closing it would make this
/// rule's duplicate-name leniency observable and force a revisit.
///
/// `out_names` is the target list's output names in order, so a caller may pass them
/// before the plan node that would carry them exists.
fn sql92_target_index(key: &Expr, out_names: &[String]) -> Option<usize> {
    match key {
        Expr::Literal(Literal::Int(n)) => usize::try_from(*n)
            .ok()
            .filter(|&i| i >= 1 && i <= out_names.len())
            .map(|i| i - 1),
        Expr::Column { table: None, name } => {
            let mut matches = out_names.iter().enumerate().filter(|(_, c)| *c == name);
            let first = matches.next()?;
            matches.next().is_none().then_some(first.0)
        }
        _ => None,
    }
}

/// The position of the first target-list entry `key` is structurally equal to, seen
/// through each entry's `AS alias` — 18.6's `findTargetlistEntrySQL99`, which compares
/// the transformed key with each entry's expression by `equal()` and returns the **first**
/// match ("it doesn't really matter whether there is more than one match").
///
/// A key reaches that rule wherever the SQL92 one does not take it: every window
/// `PARTITION BY` / `ORDER BY` key (`transformWindowDefinitions` forces SQL99), and an
/// `ORDER BY` / `GROUP BY` / `DISTINCT ON` key the SQL92 branch does not resolve. What makes the match observable is `addTargetToSortList` /
/// `addTargetToGroupList` resolving the matched entry's `unknown` to `text`: a window key
/// can be a bare string constant or `NULL` equal to such an entry
/// (`SELECT 1 UNION SELECT '2', row_number() OVER (PARTITION BY '2') FROM t` is `42804`
/// on 18.6, measured), while in the other three clauses a constant is an ordinal or
/// `42601 non-integer constant` before it gets here — `NULL` included, measured — so the
/// member there is a bare undeclared `$N` (`SELECT 1 UNION (SELECT $1
/// FROM t ORDER BY $1)` is `42804`, measured). Equality over a typed entry is harmless:
/// only an `unknown` entry changes type when claimed, and an expression equal to one is
/// the same bare shape.
pub(super) fn sql99_target_index(key: &Expr, targets: &[Expr]) -> Option<usize> {
    targets.iter().position(|t| match t {
        Expr::Alias { expr, .. } => expr.as_ref() == key,
        other => other == key,
    })
}

/// A no-FROM `SELECT`'s target list — the one row its `Plan::Values` carries — or no
/// entries for any other plan, for [`sql99_target_index`].
fn single_row_targets(plan: &Plan) -> &[Expr] {
    match plan {
        Plan::Values { rows, .. } if rows.len() == 1 => &rows[0],
        _ => &[],
    }
}

/// The output column a **whole** ORDER BY key denotes under Postgres's SQL92 rule, or
/// `None` when the key resolves against the input instead.
///
/// The rule reaches only a key that is *entirely* a simple (unqualified) name — a
/// qualified name skips it (`findTargetlistEntrySQL92` gates on
/// `list_length(fields) == 1`), and so does a name nested inside a larger expression,
/// which `transformExpr` resolves against the range table like any other column
/// reference. Verified against the oracle: with `SELECT x*10 AS a FROM t`,
/// `ORDER BY a` sorts by `x*10` while `ORDER BY a+0` sorts by the base column `a`.
fn whole_key_output_name<'a>(expr: &'a Expr, output_names: &HashSet<String>) -> Option<&'a str> {
    match expr {
        Expr::Column { table: None, name } if output_names.contains(name) => Some(name),
        _ => None,
    }
}

/// Collect every column an expression references as a `(qualifier, name)` pair,
/// skipping the `*` wildcard and self-contained subqueries (whose columns resolve
/// internally). Used to decide which columns an ORDER BY key needs available at sort
/// time.
///
/// **The qualifier is part of the identity.** Dropping it collapses `pb.x` to `x`,
/// where a sibling `pa.x` in the projection then answers for it — the sort silently
/// reads the wrong relation's column. Every consumer here compares the pair.
pub(super) fn collect_referenced_columns(expr: &Expr, out: &mut Vec<(Option<String>, String)>) {
    // A bare column is the only contributor; everything else just recurses over its
    // children (self-contained subqueries and column-free leaves yield none).
    if let Expr::Column { table, name } = expr
        && name != "*"
    {
        out.push((table.clone(), name.clone()));
    }
    expr.for_each_child(|child| collect_referenced_columns(child, out));
}

/// Expand `*` / `alias.*` wildcards in a projection into explicit per-column
/// references against the FROM tables.
///
/// A lone bare `*` over a **single relation** is left untouched (the `Plan::Project`
/// executor returns the input columns directly — the fast path, whose columns are
/// already bare). Over a **join** it is expanded via [`star_exprs`] so the emitted
/// shape matches Postgres — bare column names, and each `JOIN … USING` column merged
/// to a single `COALESCE` — rather than the join executor's internal dotted columns
/// (the pre-existing bare-`*`-over-join bug). Every other wildcard is expanded too: a
/// qualified `alias.*` to that one relation's columns (so it picks only its side of a
/// JOIN, e.g. the spine's `SELECT al.*, …`), and a bare `*` mixed with other items to
/// the same merged/qualified star exprs (over a join) or every FROM table's columns
/// in order (single relation).
fn expand_wildcards(
    projection: Vec<Expr>,
    from: &Plan,
    tables: &[(String, Option<String>)],
    schema: &Schema,
    nonbase: &HashMap<String, Vec<ColumnDesc>>,
) -> Result<Vec<Expr>> {
    let has_join = matches!(from, Plan::Join { .. });
    if projection.len() == 1 && projection[0].is_bare_star() {
        return if has_join {
            star_exprs(from, schema)
        } else {
            Ok(projection)
        };
    }
    let mut out = Vec::with_capacity(projection.len());
    for expr in projection {
        match &expr {
            Expr::Column { table, name } if name == "*" => match table {
                Some(qualifier) => {
                    out.extend(expand_qualified_wildcard(
                        qualifier, tables, schema, nonbase,
                    )?);
                }
                None if has_join => out.extend(star_exprs(from, schema)?),
                None => {
                    for (tname, talias) in tables {
                        let qualifier = talias.as_deref().unwrap_or(tname);
                        out.extend(expand_qualified_wildcard(
                            qualifier, tables, schema, nonbase,
                        )?);
                    }
                }
            },
            _ => out.push(expr),
        }
    }
    Ok(out)
}

/// Expand a bare `*` over a FROM **join tree** into the explicit projection exprs
/// Postgres emits — resolving the addressable-vs-emitted split the flat column model
/// otherwise conflates.
///
/// The join executor's row carries every raw per-side column (dotted, so
/// `left.c` / `right.c` stay qualifiable) plus, for a USING join, a prepended merged
/// column; a bare `*` must instead show Postgres's shape. So rather than forward the
/// executor's columns, `*` becomes explicit exprs that resolve against them:
///
/// - An ordinary / CROSS / comma join → each side's columns as qualified
///   `Column { table: Some(effective), name }` refs, left then right (dups kept) —
///   whose bare `output_name`s give the correct wire names (the bare-`*` bug fix).
/// - A `JOIN … USING (c…)` → one `COALESCE(left.c, right.c) AS c` per USING column
///   first (USING order), then each side's non-USING columns — matching Postgres's
///   merged column ordering. Chained USING composes: a nested side's `c` is already a
///   `COALESCE`, so the outer merge nests it (`COALESCE(COALESCE(a, b), c)`).
///
/// # Errors
///
/// Propagates a would-be-unreachable missing USING column (the plan-time
/// `resolve_join_constraint` validated presence) as `42703`, staying correct-or-loud.
fn star_exprs(plan: &Plan, schema: &Schema) -> Result<Vec<Expr>> {
    let mut names = Vec::new();
    collect_leaf_names(plan, &mut names);
    star_exprs_in(plan, schema, &names)
}

/// The effective names of the FROM items under `plan`'s joins — one level's items. A
/// walk over joins only, as [`collect_nonbase_into`]'s is.
fn collect_leaf_names(plan: &Plan, out: &mut Vec<String>) {
    if let Plan::Join { left, right, .. } = plan {
        collect_leaf_names(left, out);
        collect_leaf_names(right, out);
    } else {
        out.extend(leaf_name(plan));
    }
}

/// [`star_exprs`]'s recursive body; `names` holds every FROM item's effective name at
/// the level, for [`leaf_qualifier`].
fn star_exprs_in(plan: &Plan, schema: &Schema, names: &[String]) -> Result<Vec<Expr>> {
    crate::depth::check_stack()?;
    let Plan::Join {
        left, right, using, ..
    } = plan
    else {
        // A leaf relation: qualified refs to each of its output columns.
        let qualifier = leaf_qualifier(plan, names);
        return Ok(plan
            .output_columns(schema)
            .into_iter()
            .map(|c| Expr::Column {
                table: qualifier.clone(),
                name: c.name,
            })
            .collect());
    };

    let left_exprs = star_exprs_in(left, schema, names)?;
    let right_exprs = star_exprs_in(right, schema, names)?;
    if using.is_empty() {
        let mut out = left_exprs;
        out.extend(right_exprs);
        return Ok(out);
    }

    // The value expression for USING column `c` on one side (its projection expr with
    // the alias peeled, so an outer merge can nest a nested side's `COALESCE`).
    let side_value = |exprs: &[Expr], c: &str| -> Option<Expr> {
        exprs
            .iter()
            .find(|e| e.output_name() == c)
            .cloned()
            .map(strip_alias)
    };

    // `USING (k)` is `left.k = right.k`, so an operand pair with no `=` operator is an
    // operator error — and Postgres reports *that*, not the merged output column's
    // common type: `text` `k` against `int` `k` is `42883`, never the `42804` the
    // `COALESCE` below would otherwise raise first. Checking here keeps the two errors
    // in Postgres's order (the join qual is resolved before the merged column).
    let left_cols = left.output_columns(schema);
    let right_cols = right.output_columns(schema);
    for c in using {
        let side_type = |cols: &[ColumnDesc]| match using_column_match(cols, c) {
            UsingMatch::Found(i) => Some(ResolvedType::of_column(&cols[i])),
            _ => None,
        };
        comparison_legality(side_type(&left_cols), side_type(&right_cols), "=")?;
    }

    let mut out = Vec::with_capacity(left_exprs.len() + right_exprs.len());
    for c in using {
        let missing = || Error::from(CatalogError::ColumnNotFoundInExpr { column: c.clone() });
        let lval = side_value(&left_exprs, c).ok_or_else(missing)?;
        let rval = side_value(&right_exprs, c).ok_or_else(missing)?;
        out.push(Expr::Alias {
            expr: Box::new(Expr::Coalesce(vec![lval, rval])),
            alias: c.clone(),
        });
    }
    // The non-USING columns of each side, left then right (USING order already emitted).
    for e in left_exprs.into_iter().chain(right_exprs) {
        if !using.iter().any(|c| c == &e.output_name()) {
            out.push(e);
        }
    }
    Ok(out)
}

/// The effective FROM-clause qualifier of a leaf relation (for star expansion), or
/// `None` for a source with no stable name (its columns then resolve unqualified).
///
/// A FROM function item is qualified only where no other item in `names` — the level's
/// effective names — shares its name. Two items sharing a name are 18.6's `42712`, a gate
/// pglet does not have (`EDGES.md` row 322), and both qualified by the shared name, the
/// expansion's references bind the first item for both and answer its values twice;
/// unqualified, they are the `42702` they were before the arm. The other kinds keep
/// their qualifier whatever their neighbours are called.
fn leaf_qualifier(plan: &Plan, names: &[String]) -> Option<String> {
    let name = leaf_name(plan)?;
    let shared = names.iter().filter(|n| **n == name).count() > 1;
    (!(shared && matches!(plan, Plan::FunctionScan { .. }))).then_some(name)
}

/// A leaf relation's effective FROM name, or `None` for a source with no stable name.
fn leaf_name(plan: &Plan) -> Option<String> {
    match plan {
        Plan::Scan { table, alias } => Some(scan_effective_name(table, alias)),
        Plan::SubqueryScan { alias, .. }
        | Plan::CteScan { alias, .. }
        | Plan::FunctionScan { alias, .. } => Some(alias.clone()),
        Plan::VirtualScan {
            catalog_table,
            alias,
        } => Some(
            alias
                .clone()
                .unwrap_or_else(|| catalog_table.name().to_string()),
        ),
        _ => None,
    }
}

/// Peel an outer `Expr::Alias`, yielding the underlying value expression (used to
/// nest a merged USING column into an outer `COALESCE`).
fn strip_alias(e: Expr) -> Expr {
    match e {
        Expr::Alias { expr, .. } => *expr,
        other => other,
    }
}

/// Expand a single `qualifier.*` into one `qualifier.col` reference per column of
/// the relation the qualifier names — a non-base CTE/derived relation
/// (`nonbase`, keyed by effective FROM name) or, failing that, a base table
/// (by FROM alias, else table name).
fn expand_qualified_wildcard(
    qualifier: &str,
    tables: &[(String, Option<String>)],
    schema: &Schema,
    nonbase: &HashMap<String, Vec<ColumnDesc>>,
) -> Result<Vec<Expr>> {
    // A CTE reference / derived table — expand to its captured output columns.
    if let Some(cols) = nonbase.get(qualifier) {
        return Ok(cols
            .iter()
            .map(|c| Expr::Column {
                table: Some(qualifier.to_string()),
                name: c.name.clone(),
            })
            .collect());
    }
    let table_name = tables
        .iter()
        .find(|(name, alias)| {
            alias.as_deref() == Some(qualifier) || (alias.is_none() && name == qualifier)
        })
        .map(|(name, _)| name.as_str())
        .ok_or_else(|| {
            Error::NotImplemented(format!(
                "unknown table qualifier `{qualifier}` in `{qualifier}.*`"
            ))
        })?;
    let table = schema.get_table(table_name).ok_or_else(|| {
        Error::NotImplemented(format!("`{qualifier}.*` over a non-base relation"))
    })?;
    Ok(table
        .columns
        .iter()
        .map(|col| Expr::Column {
            table: Some(qualifier.to_string()),
            name: col.name.clone(),
        })
        .collect())
}

/// The [`ColumnScope`] the static comparison gate resolves references against — one
/// entry per FROM relation, under the name a qualified reference must spell (its
/// alias if it has one, else the table name).
///
/// A base table's columns come from the catalog, a CTE / derived table / virtual
/// catalog view's from `nonbase` (keyed by the same effective name), and a FROM-clause
/// function's from `function_cols` (likewise). A relation in none of the three
/// contributes its name and no columns, which is sound: see [`ColumnScope`].
///
/// **The three are consulted by FROM-item kind, and the catalog is asked after the two
/// kind maps rather than first** — third of the four arms, the fourth being the fallback
/// for a relation none of the three enumerates. A FROM
/// item's effective name *replaces* the namespace entry for that name at its level
/// (`addRangeTableEntry`), so a base table sharing the spelling is not in scope under it
/// at all. `nonbase` and `function_cols` are keyed by effective name and hold exactly this
/// level's FROM items of those kinds, so a hit in either **proves** this item is one of
/// them — where a `schema.get_table` hit proves only that some table is spelled the same.
/// Asking the catalog first therefore registered a derived table's or CTE's name with the
/// *table's* columns, exhaustively: `SELECT z FROM (SELECT 1 AS z) AS t` and `WITH t AS
/// (SELECT 1 AS z) SELECT z FROM t` were `42703` here against 18.6's row, as were the
/// qualified and `t.*` spellings and the `ORDER BY` / `GROUP BY` / `WHERE` positions.
/// Where the base table *carries* the name at another type the reference bound instead,
/// and the wrong relation's type reached `ColumnScope::coercion_type`: over a `shadow(z
/// TEXT)` shadowed by `(SELECT 5 AS z)`, `WHERE z = '5'` was `42883 operator does not
/// exist: integer = text`. The **silent** polarity needed an empty derived table —
/// `SELECT a FROM (SELECT a AS z FROM t WHERE false) AS t` answered `(0 rows)` against
/// 18.6's `42703`, because the base table's `a` was *bound* and only a row would have
/// refused it. What made the class a wrong **refusal** rather than a quiet
/// mis-resolution is the plan-time `unnest(<non-array>)` gate, which reads its argument's
/// type from this scope. Members, residuals and the duplicate-alias cost:
/// `docs/scope.md` § A FROM name is resolved by the item's kind.
///
/// The unnest entry registers **exhaustively**, like a base table and unlike a catalog
/// view. That is Postgres's own reading — a function RTE has exactly its declared
/// columns — and the map is not an approximation of them: `collect_function_scan_into`
/// reads the node's own `output_columns`, the same derivation the planner-declared type
/// walk uses, so the relation *is* its map. The arm arrived in two steps and each was a
/// separate closure, which is worth keeping apart because only the second is about
/// binding at all.
///
/// **The columns.** They were an empty map, and filling them is what the coercion pass
/// was reached for: the branch-unification pass can only *type* such a column with them,
/// and it **defers on any branch it cannot type** (`unify_common_type_branches`), so an
/// empty map silently disabled unification for the whole expression —
/// `SELECT COALESCE(NULL, v) FROM unnest(ARRAY[1,2]) AS u(v)` left its `NULL` branch
/// untyped and the executor's total walk declared `text` over two int4 values, against
/// 18.6's — and pglet's own `Describe`'s — `int4`
/// (`select/unnest_coalesce_declared_type.sql`). The same map lets the **static
/// comparison gate** type the column: `WHERE v = '2'` over `unnest(ARRAY[1,2]) AS u(v)`
/// was `42883 operator does not exist: integer = text`, bare and qualified spellings
/// alike and `>` as well as `=`, where both engines answer
/// (`select/unnest_column_compared_with_literal.sql`).
///
/// **The exhaustive bit.** Filling the map already moved one `Binding` arm — the
/// **claimant count**, since `binding_bare` counts every relation whose `columns` map
/// holds the name, exhaustive or not, and tests `claimants >= 2` *ahead* of the
/// uncertainty check, so `SELECT id FROM amb_un, unnest(ARRAY[]::int[]) AS u(id)` over
/// `amb_un(id, x)` went from `(0 rows)` to 18.6's `42702` at **Parse**
/// (`select/unnest_ambiguous_bare_name.sql`). The bit moves the other two, both in the
/// *relation-lacks-the-name* direction the columns could not reach:
/// `binding_qualified` reads [`ColumnScope`]'s `exhaustive` field directly and answers
/// `NoColumn` where it answered `Uncertain`, and `binding_bare` walks past a level whose
/// relations are all exhaustive instead of stopping at it, so a name nothing carries
/// reaches `NoColumn` at the end of the chain. Measured, each `(0 rows)` here before and
/// `42703` on both engines after — at **Parse** on both, raw v3 `Parse` + `Sync`:
/// `SELECT bogus FROM unnest(ARRAY[]::int[]) AS u(x)` and its `u.bogus` spelling, the
/// `GROUP BY bogus` / `ORDER BY bogus` / `HAVING bogus > 1` clause positions, and
/// `SELECT lbl AS q FROM unnest(ARRAY[3,1,2]) AS u(lbl) ORDER BY q + 0` (a *nested* key
/// naming an output column, which answered rows). Pinned by
/// `select/unnest_bogus_*_loud.sql` and `select/unnest_nested_order_by_name_loud.sql`,
/// each red on the revert of this one call.
///
/// A third reader moves, and it is **not** `check_binding`'s:
/// `ColumnScope::sole_relation_carrying` declines outright on an uncertain level, so no
/// `ORDER BY` key over an unnest level could be bound to its relation when
/// `output_name_binding` decides whether two same-named target entries are one expression
/// or two. That move has an observable of its own and it is red on the revert:
/// `select/unnest_duplicate_output_entries_bind_to_the_unnest.sql`, a false `42702` before.
///
/// **Two further readers of the same field cannot change a verdict here, and the reason
/// is this scope's shape rather than their rules.** `bare_candidates` and `qualified`
/// consult `level_columns_uncertain` only to decide whether to keep walking **outward**,
/// and a scope this function builds has nowhere outward to walk: `push_scope_relation`
/// stamps every relation level `0`, and the only production `nested_in` — the one call in
/// `planner::correlation`'s `resolve_level` — chains a scope `push_level_relations` built,
/// whose `FunctionScan` arm was exhaustive all along. So `levels()` is `0..=0` in every scope
/// the flipped line touches, and both readers return identically either way. The
/// "the coercion pass could not walk past an unnest level" sentence this paragraph used
/// to carry was a mechanism story with no reachable member; the three genuine movers are
/// `binding_qualified`, `binding_bare` and `sole_relation_carrying`.
///
/// The accept polarity of all of it is
/// `select/unnest_correlated_outer_bare_name_answers.sql`,
/// `select/unnest_real_column_over_an_empty_array_answers.sql`,
/// `select/unnest_sibling_relations_bare_name_binds.sql` and
/// `select/unnest_output_alias_sort_key_answers.sql` — those four green on either side of
/// the flip, since they guard against a widening that went too far.
///
/// **Every array in that reject list is empty, and that is the class's shape rather than
/// a fixture convention.** With elements the eval-time resolver reaches the reference and
/// raises the same `42703`, so the non-empty spellings were already green on both engines
/// — what the flip moves there is only the *phase*, from `Execute` to `Parse`. The one
/// exception is the nested `ORDER BY` key, whose divergence was silent (rows against a
/// refusal) and therefore needs rows to be visible at all.
///
/// What the flip does **not** touch, re-measured on both engines: `SELECT u.*` over an
/// unnest is still `0A000` where no base table is spelled `u` — and where one is whose
/// column names are a **subset** of the alias list's, still that table's columns,
/// silently, since [`expand_qualified_wildcard`] reads `nonbase` (which has no arm for an
/// `unnest` item) and then the catalog, not this scope. Only that subset polarity is untouched:
/// where the base table carries a name the alias list lacks, the flip **moved** it, and
/// moved a silent member to a loud one — beside a base `u(x, y)`, `SELECT u.* FROM
/// unnest(ARRAY[]::int[]) AS u(q)` bound and answered `x | y` with no rows before and is
/// `42703 column "u.x"` now, matching what the non-empty spelling already was, so the
/// class is row-count-independent at this revision (`EDGES.md` row 8's three-build table,
/// line 5). Unmoved beside it: `AS u(a,b)` over
/// a one-column unnest is still accepted here against 18.6's `42P10`, a bare `unnest($1)`
/// is still pglet's `0A000` against 18.6's `42725`, and two same-named unnest aliases are
/// still not 18.6's `42712`. That last one moved for *some* of its spellings — two that
/// answered rows now answer a wrong code — while others still answer rows, and the
/// kind-ordering repair above moved it a second time, trading one silent member for
/// another over a base table colliding with a derived one; the members of both moves are
/// enumerated in `EDGES.md`. The gaps this scope leaves open — the qualified reference,
/// `alias.*`, and the bare `*` that is not the projection's sole item (the same reader as
/// `alias.*`, reached from [`expand_wildcards`]'s second call site) — live in
/// [`collect_nonbase_into`] and `leaf_qualifier` rather than here.
fn build_column_scope(
    tables: &[(String, Option<String>)],
    nonbase: &HashMap<String, Vec<ColumnDesc>>,
    function_cols: &HashMap<String, Vec<ColumnDesc>>,
    catalog_views: &HashSet<String>,
    schema: &Schema,
) -> ColumnScope {
    let mut scope = ColumnScope::default();
    for (table_name, alias) in tables {
        let effective = alias.clone().unwrap_or_else(|| table_name.clone());
        // **Kind before name.** `function_cols` and `nonbase` are keyed by effective FROM
        // name and hold exactly the FROM items of the two non-base kinds, so a hit is
        // proof that *this* FROM item is one of them — while `schema.get_table` proves
        // only that some table shares the spelling. Asking the catalog first therefore
        // resolved a derived table or CTE aliased to a base table's name to the *table*,
        // exhaustively; see this function's doc comment for the measured members.
        if let Some(cols) = function_cols.get(&effective) {
            // A FROM-clause function is columns-**exhaustive**: Postgres's function RTE
            // has exactly its declared columns, and `collect_function_scan_into` reads them off
            // the node's own `output_columns`, so this map *is* the relation. The same
            // reading is what `push_level_relations` has always used for the correlated
            // sub-plan scope; the two arms agree now.
            scope.push_derived_relation(effective, cols);
        } else if let Some(cols) = nonbase.get(&effective) {
            let columns = crate::planner::build_desc_type_map(cols);
            // A catalog view pglet models only **in part** is registered
            // non-exhaustively: a gate reading its subset as complete turns
            // `SELECT udt_name FROM information_schema.columns WHERE table_name =
            // '<absent>'` — `(0 rows)` on both engines — into a false `42703`. A view
            // whose declared list is 18.6's whole list is not in `catalog_views` at all
            // and falls to `push_relation` below, exhaustively. A CTE reference and a
            // derived table carry every column they have.
            if catalog_views.contains(&effective) {
                scope.push_partial_relation(effective, columns);
            } else {
                scope.push_derived_relation(effective, cols);
            }
        } else if let Some(t) = schema.get_table(table_name) {
            // A base table, looked up by its **table name** rather than its effective
            // name — `FROM users u` is the catalog's `users` under the scope name `u`.
            scope.push_stored_relation(effective, t);
        } else {
            // A relation none of the three maps enumerates — a table the schema has no
            // entry for. The FROM item's **name** is known either way; only its columns
            // are not, so it registers non-exhaustively. Registering nothing would let a
            // closed scope answer `42P01` for a qualifier the statement plainly writes.
            scope.push_partial_relation(effective, HashMap::new());
        }
    }
    scope
}

/// The columns of every FROM-clause function item (`unnest(…)`, `generate_series(…)`) in
/// this plan, keyed by its effective FROM name (`chooseScalarFunctionAlias`: the explicit
/// alias, else the function's name).
///
/// A sibling of [`collect_nonbase_columns`], of which an `unnest` item is deliberately
/// **not** an arm (a `generate_series` item is, under the same name and columns). The
/// two maps feed different readers: this one reaches [`build_column_scope`] alone, while
/// `nonbase` additionally drives wildcard expansion — `alias.*` and, at
/// [`expand_wildcards`]'s second call site, a bare `*` that is not the projection's sole
/// item over a non-join `FROM` — and the column-type map. Widening
/// `collect_nonbase_into` instead would move `SELECT u.* FROM unnest(…) AS u` — a
/// recorded `0A000` divergence in its own right (`EDGES.md`) —
/// as a side effect of a declared-type fix, which is the kind of quiet neighbour change
/// this engine's fixtures cannot see coming.
///
/// **That omission has a second cost, and it is silent**: because `nonbase` has no
/// `unnest` arm, [`expand_qualified_wildcard`] falls past it to `schema.get_table`, so a
/// base table sharing the unnest's alias answers for it — `SELECT uu.* FROM
/// unnest(ARRAY[1,2], ARRAY[3,4]) AS uu(q, r)` beside a `uu(q INT)` expands one column
/// where 18.6 expands two, and `SELECT *, 1 FROM …` reads identically, that reader's
/// second call site taking any bare `*` the projection does not consist of alone. So the
/// arm is a *deliberate* omission and not a free one; the
/// members, both polarities and the controls are `EDGES.md`'s row 8.
///
/// The column *types* come from the node's own `output_columns`, so this map and the
/// planner-declared type walk cannot disagree about them.
fn function_scan_columns(plan: &Plan, schema: &Schema) -> HashMap<String, Vec<ColumnDesc>> {
    let mut map = HashMap::new();
    collect_function_scan_into(plan, schema, &mut map);
    map
}

/// [`function_scan_columns`]'s recursive body — the FROM-shaped nodes a leaf can hide under.
fn collect_function_scan_into(
    plan: &Plan,
    schema: &Schema,
    map: &mut HashMap<String, Vec<ColumnDesc>>,
) {
    match plan {
        Plan::Join { left, right, .. } => {
            collect_function_scan_into(left, schema, map);
            collect_function_scan_into(right, schema, map);
        }
        Plan::FunctionScan { alias, .. } => {
            map.insert(alias.clone(), plan.output_columns(schema));
        }
        _ => {}
    }
}

/// The effective FROM names of the catalog views this plan scans **that pglet models
/// only in part**.
///
/// They sit in `nonbase` beside CTE references and derived tables, and only these three
/// kinds reach that map — but the three are not alike to the binding gate: a CTE
/// reference and a derived table carry every column they have, while pglet models a
/// strict subset of 18.6's columns for several of its catalog relations, so *those*
/// have to register non-exhaustively or an answerable query becomes a false `42703`.
/// The rest — the relations whose declared list is 18.6's whole list — are deliberately
/// **absent** from this set, so they register exhaustively and a name outside the list
/// is refused at plan time. `CatalogTable::columns_complete` is the one fact that
/// decides which side a relation is on.
fn catalog_view_names(plan: &Plan) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_catalog_view_names(plan, &mut names);
    names
}

/// [`catalog_view_names`]'s recursive body — the same leaf kinds
/// [`collect_nonbase_into`] walks.
fn collect_catalog_view_names(plan: &Plan, names: &mut HashSet<String>) {
    match plan {
        Plan::Join { left, right, .. } => {
            collect_catalog_view_names(left, names);
            collect_catalog_view_names(right, names);
        }
        // Only the relations pglet models **in part**. A relation whose declared
        // column list is 18.6's whole list registers exhaustively instead, so a
        // name outside it is `42703` at plan time rather than a fabricated zero-row
        // `text` column — see `CatalogTable::columns_complete`.
        Plan::VirtualScan {
            catalog_table,
            alias,
        } if !catalog_table.columns_complete() => {
            names.insert(
                alias
                    .clone()
                    .unwrap_or_else(|| catalog_table.name().to_string()),
            );
        }
        _ => {}
    }
}

/// Every column a `JOIN … USING (…)` in this FROM plan merges, in walk order.
///
/// A merged column is *one* column carrying the name however many sides declare it, so
/// the binding gate must not count it once per side — `SELECT num FROM a JOIN b USING
/// (num)` is ordinary SQL and the scope holds `a.num` and `b.num`. The walk descends
/// both sides so a chain of `USING` joins contributes each name once per join, which is
/// harmless: the gate asks only whether a name is merged, never how often.
pub(super) fn using_column_names(plan: &Plan) -> Vec<String> {
    let mut names = Vec::new();
    collect_using_names(plan, &mut names);
    names
}

/// [`using_column_names`]'s recursive body.
fn collect_using_names(plan: &Plan, names: &mut Vec<String>) {
    if let Plan::Join {
        left, right, using, ..
    } = plan
    {
        names.extend(using.iter().cloned());
        collect_using_names(left, names);
        collect_using_names(right, names);
    }
}

/// Register every relation an already-assembled FROM plan introduces into `scope`,
/// under the name a qualified reference must spell.
///
/// The plan-walking counterpart to [`build_column_scope`], for the lowering paths that
/// hold a FROM sub-plan but no parsed `(name, alias)` list — `UPDATE … SET … FROM src
/// s`, whose source relations must be in scope or `s.x` resolves against the *target*
/// table's same-named column and gets a cast to the wrong type.
///
/// The walk itself is [`plan_relations`], which is also what `lower_returning` expands a
/// wildcard over and what the planner-declared type walk reads — so the ordered list and
/// this unordered scope cannot disagree about which relations the `FROM` introduces or
/// what they are called. This adapter only drops the column *order*, which a
/// [`ColumnScope`] has no use for.
pub(super) fn push_plan_relations(scope: &mut ColumnScope, plan: &Plan, schema: &Schema) {
    for (name, columns) in plan_relations(plan, schema).iter() {
        scope.push_derived_relation(name.to_string(), columns);
    }
}

/// Collect the output columns of every non-base FROM relation — a CTE reference
/// (`Plan::CteScan`), a derived table / inlined read-only CTE (`Plan::SubqueryScan`) or a
/// virtual catalog view (`Plan::VirtualScan`) — keyed by its effective FROM name.
/// Descends the assembled FROM join tree; the recursion is [`collect_nonbase_into`].
///
/// A key here is **proof of the FROM item's kind**, which is what
/// [`build_column_scope`]'s ordering rests on. It is *not* "the relations
/// `schema.get_table` can't resolve" — that reading was this comment's, and it is false in
/// the direction that matters: a derived table or CTE aliased to a base table's name
/// resolves there perfectly well, to the wrong relation.
fn collect_nonbase_columns(from: &Plan, schema: &Schema) -> HashMap<String, Vec<ColumnDesc>> {
    let mut map = HashMap::new();
    let mut names = Vec::new();
    collect_leaf_names(from, &mut names);
    collect_nonbase_into(from, schema, &names, &mut map);
    map
}

/// Register every non-base FROM relation's columns under its effective FROM name.
///
/// **Several functions walk these same leaf kinds and they do not agree about which
/// ones they cover.** [`push_plan_relations`], `push_level_relations`, `leaf_qualifier`
/// and the executor's `single_table_name` and `get_table_name_from_plan` each have an arm
/// for every FROM function item (`leaf_qualifier`'s qualifying only one whose name is
/// unique at its level), as does [`function_scan_columns`]'s
/// [`collect_function_scan_into`]; this one has a `generate_series` arm for an item so
/// named and no `unnest`
/// one, so a FROM-clause `unnest(…)` names no relation here. A qualified reference to its column
/// still reads the `unnest`'s value: where another `FROM` relation carries the same name,
/// `u.k` was measured to answer as 18.6 does in a projection, a `WHERE`, an `ORDER BY`, a
/// `GROUP BY` and beside a window call. `alias.*` over it — plus
/// the bare `*` that shares its reader, any `*` that is not the projection's sole item
/// over a non-join `FROM` — is `0A000` only where **no base table carries the alias's
/// spelling**; where one does, [`expand_qualified_wildcard`] falls past this map to the
/// catalog and silently expands *that table's* column names instead. Measured pairs and
/// the reason a plain `SELECT` diverges silently where
/// `UPDATE … FROM` diverges loudly: `EDGES.md`. Recorded, not
/// closed.
///
/// **The two arms that do cover it used to disagree about *exhaustiveness*, and that
/// split is now closed.** Both [`build_column_scope`] and `push_level_relations` register
/// a FROM function item with `push_relation` — columns-exhaustive — which is Postgres's reading of
/// a function RTE and, here, literally the node's `output_columns`. While they disagreed,
/// the consequence read backwards from the arms: a bogus name was judged correctly one
/// level **down** and not at the query's own level. Over an empty unnest,
/// `SELECT nosuch FROM unnest(ARRAY[]::int[]) AS u(q)` answered `(0 rows)` here against
/// 18.6's `42703` (the `u.nosuch` spelling likewise), while
/// `… WHERE EXISTS (SELECT 1 FROM unnest(ARRAY[]::int[]) AS u(q) WHERE nosuch = 1)` was
/// `42703` on both. Non-empty spellings agreed on both engines throughout, from the
/// eval-time resolver — which is what kept the own-level half invisible on real data, and
/// why the fixtures that pin it carry `ARRAY[]::int[]`.
///
/// **The flip was sized before it was taken, and the sizing said nothing could see it** —
/// the whole gate corpus stayed green (1756/0/8 fixtures, 1431/0 `cargo test`, both
/// declared-type legs unmoved with 0 divergent) with the arm already exhaustive. So the
/// fixtures were written first and the accept polarity measured on both engines before
/// the one-line change; the reject set is red on its revert and the accept set is not.
/// A green run over an unsized change here is not evidence in either direction.
///
/// A third builder covers the same relation kind and never had the split:
/// [`push_plan_relations`] (over [`plan_relations`]) has only `push_relation`, which is
/// why `UPDATE … FROM unnest(…) … WHERE bogus = 1` was already `42703` on both engines —
/// `update/unnest_from_source_bogus_bare_name_loud.sql`, green on either side of the
/// flip. Three builders, one relation kind, and the answer differed by route.
fn collect_nonbase_into(
    plan: &Plan,
    schema: &Schema,
    names: &[String],
    map: &mut HashMap<String, Vec<ColumnDesc>>,
) {
    match plan {
        Plan::Join { left, right, .. } => {
            collect_nonbase_into(left, schema, names, map);
            collect_nonbase_into(right, schema, names, map);
        }
        Plan::CteScan { alias, columns, .. } => {
            map.insert(alias.clone(), columns.clone());
        }
        Plan::SubqueryScan {
            subquery,
            alias,
            column_aliases,
            cte_rank: _,
        } => {
            map.insert(
                alias.clone(),
                super::alias_relation_columns(subquery.output_columns(schema), column_aliases),
            );
        }
        // A virtual catalog table (`information_schema.columns`, the FK views, …):
        // the catalog has no `Table` for it, so register its synthesized columns
        // here keyed by its effective FROM name. This makes its columns visible to
        // the column-type map (so `build_sort` treats a non-projected virtual
        // column like `ordinal_position` as a base column and sorts the
        // pre-projection rows) and to `alias.*` expansion.
        Plan::VirtualScan {
            catalog_table,
            alias,
        } => {
            let name = alias
                .clone()
                .unwrap_or_else(|| catalog_table.name().to_string());
            map.insert(name, catalog_table.columns());
        }
        // A `generate_series` item carries every column it has, so it expands as a
        // derived table does — `g.*`, and a bare `*` beside another item, beside a base
        // table spelled `g` too. `unnest` stays out (see [`function_scan_columns`]). An
        // item whose name another item at its level shares stays out as well: the map
        // keeps one entry per name, and `a.*` over two series named `a` expanded the one
        // entry, which the first item then answered twice (`EDGES.md` row 322).
        Plan::FunctionScan {
            func: FromFunction::GenerateSeries(_),
            alias,
            ..
        } if names.iter().filter(|n| *n == alias).count() == 1 => {
            map.insert(alias.clone(), plan.output_columns(schema));
        }
        _ => {}
    }
}

/// Build the WHERE-clause node, using an index probe when one applies.
///
/// `input` is the FROM plan; for index eligibility it must be a single
/// base-table `Plan::Scan` (`single_table` gates the no-join case). Two probe
/// shapes are tried, in order:
///
/// 1. **Equality** — if the `predicate` is (or AND-contains) an
///    `indexed_col = <row-independent value>` equality on a single-column index,
///    emit a `Plan::IndexScan` for it (most selective, so preferred).
/// 2. **Range** — else if it AND-contains range comparisons
///    (`indexed_col {>,>=,<,<=} <row-independent value>`, or a desugared
///    `BETWEEN`) on a single-column index, merge every range conjunct on that
///    same column into one `Plan::IndexRangeScan` with `lower`/`upper` bounds.
///
/// Either way any remaining conjuncts (other columns, non-range, `OR`) wrap the
/// probe in a `Filter`. With no eligible conjunct, falls back to plain
/// `Filter(Scan)`.
fn plan_filter_or_index_scan(
    input: Plan,
    predicate: Expr,
    single_table: bool,
    schema: &Schema,
) -> Plan {
    // Index probes only apply over a single base table with a registered index.
    // With acceleration off (the stripped "testing" profile) the planner emits a
    // plain `Filter(Scan)` for every predicate — intended same rows, full scan.
    #[cfg(not(feature = "index_acceleration"))]
    let _ = (single_table, schema);
    #[cfg(feature = "index_acceleration")]
    if single_table && let Plan::Scan { table, alias } = &input {
        // 1. Equality probe — the most selective, so tried first.
        if let Some((index_oid, key_column, key_value, column_first, residual)) =
            find_index_equality(&predicate, table, alias.as_deref(), schema)
        {
            let scan = Plan::IndexScan {
                table: table.clone(),
                alias: alias.clone(),
                index_oid,
                key_column,
                key_value: Box::new(key_value),
                column_first,
            };
            return wrap_residual(scan, residual);
        }
        // 2. Range probe — bounded scan over a single-column index.
        if let Some((index_oid, key_column, lower, upper, residual)) =
            find_index_range(&predicate, table, alias.as_deref(), schema)
        {
            let scan = Plan::IndexRangeScan {
                table: table.clone(),
                alias: alias.clone(),
                index_oid,
                key_column,
                lower,
                upper,
                // Bare WHERE: order/limit don't matter (a later task uses them
                // for ORDER BY pushdown / Sort elision).
                ascending: true,
                limit: None,
            };
            return wrap_residual(scan, residual);
        }
        // 3. GIN containment probe — `col @> <const>` on a GIN index, either a
        //    single top-level conjunct or an OR-union of same-column probes
        //    (with optional scan-independent guards).
        if let Some((index_oid, key_column, probes, guards)) =
            find_index_containment(&predicate, table, alias.as_deref(), schema)
        {
            let scan = Plan::GinScan {
                table: table.clone(),
                alias: alias.clone(),
                index_oid,
                key_column,
                probes,
                guards,
            };
            // GIN is lossy → ALWAYS recheck the FULL original predicate (the `@>`
            // for lossiness + every other conjunct, including the partial-index
            // predicate the gate required the query to imply). The GinScan only
            // narrows to a candidate superset.
            return Plan::Filter {
                input: Box::new(scan),
                predicate,
            };
        }
    }

    Plan::Filter {
        input: Box::new(input),
        predicate,
    }
}

/// The index probe a simple `UPDATE` / `DELETE` collects its target rows through — the
/// node [`plan_filter_or_index_scan`] builds for the same `WHERE` over the target, so the
/// write takes the read path's own index decision rather than a second copy of it — or
/// `None`, for a full scan.
///
/// Only an **equality** probe is taken, and only where the residual conjuncts it leaves
/// pass [`dml_residual_is_certain`]: those are evaluated over the probe's rows alone, and
/// a conjunct that can fail would make the probe skip the failure the scan raises. Two
/// probe kinds are declined:
///
/// - **A range probe**, for cost: a B+tree host runs every write inside a storage
///   transaction, and the in-transaction range lookup (`index_range_lookup_in_tx`) reads
///   the whole table, encodes every key and sorts — no cheaper than the scan it would
///   replace, and slower once the probed rows are fetched again one by one. The
///   in-transaction equality lookup reads the committed index and the transaction's own
///   writes, so it is the one that saves the scan.
/// - **A GIN containment probe**: its probe and guard expressions are general
///   scan-independent expressions evaluated once ahead of every row, and its recheck is
///   the whole predicate, so there is no residual to judge apart from the containment.
///
/// The executor still declines a probe chosen here when a key cannot be used as it stands
/// (`executor::query`'s `probe_dml_targets`), and tests every candidate against the whole
/// `filter`.
#[cfg(feature = "index_acceleration")]
pub(super) fn dml_target_probe(
    table: &str,
    alias: Option<&str>,
    filter: &Expr,
    schema: &Schema,
) -> Option<Box<Plan>> {
    let scan = Plan::Scan {
        table: table.to_string(),
        alias: alias.map(str::to_string),
    };
    let (probe, residual) = match plan_filter_or_index_scan(scan, filter.clone(), true, schema) {
        Plan::Filter { input, predicate } => (*input, Some(predicate)),
        bare => (bare, None),
    };
    (matches!(probe, Plan::IndexScan { .. })
        && residual
            .as_ref()
            .is_none_or(|rest| dml_residual_is_certain(rest, table, alias, schema)))
    .then(|| Box::new(probe))
}

/// The stripped build's [`dml_target_probe`]: no index acceleration, so every simple
/// `UPDATE` / `DELETE` collects its targets by a full scan.
#[cfg(not(feature = "index_acceleration"))]
pub(super) const fn dml_target_probe(
    _table: &str,
    _alias: Option<&str>,
    _filter: &Expr,
    _schema: &Schema,
) -> Option<Box<Plan>> {
    None
}

/// Wrap a probe plan in a `Filter` of its residual conjuncts, or return it bare
/// when there is no residual.
#[cfg(feature = "index_acceleration")]
fn wrap_residual(scan: Plan, residual: Option<Expr>) -> Plan {
    match residual {
        Some(rest) => Plan::Filter {
            input: Box::new(scan),
            predicate: rest,
        },
        None => scan,
    }
}

/// Try to satisfy an `ORDER BY` from a single-column index, eliding the `Sort`.
///
/// Returns `(plan, elided)`: when `elided` is `true` the returned `plan` has its
/// base-table scan rewritten to an ordered `IndexRangeScan` (`ascending` set from
/// the sort direction) and carries no `Sort`; when `false` the `plan` is returned
/// unchanged for the caller to wrap in `Plan::Sort`. (A by-value `(Plan, bool)`
/// rather than `Result<Plan, Plan>` — the latter trips `result_large_err` since
/// `Plan` is a large enum.)
///
/// All gates must hold (else keep the `Sort`):
///
/// 1. **One base table** — the FROM is a bare `Plan::Scan` or an already-built
///    `IndexRangeScan` (a wrapping `Project` is reached through; an
///    `Aggregate`/`Window`/`Filter`/`Join` underneath bails). The index order
///    does not survive aggregation/windowing, and a residual `Filter` (from a
///    range WHERE on a *different* column) means the scan node isn't the order
///    column's index.
/// 2. **Exactly one ORDER BY key**, a bare `Expr::Column` whose column has a
///    single-column **full** index (`single_column_index_for`). A single-column
///    index covers only a single-key sort.
/// 3. **The column is NOT NULL** (`Column.nullable == false`). An index range read
///    answers no row whose key is NULL (`Storage::index_range_lookup`): a plain B-tree
///    index holds no entry for one (`encode_row_key` drops a row whose key part is NULL
///    under default `NULLS DISTINCT`), and a `NULLS NOT DISTINCT` index holds it under a
///    key no range read returns. So an ordered index scan would *omit* every NULL row —
///    eliding on a nullable column would drop rows, not just misorder them, whatever
///    `NULLS FIRST/LAST` was requested. No statement this build accepts is known to
///    leave a NULL under a NOT NULL declaration — each one that sets the declaration
///    over existing rows reads them, and the identity clause, which reads none, is
///    refused over a column not already declared — so any placement clause is vacuous
///    and elision stays order-correct; a nullable column keeps the `Sort`, which honors
///    the placement. (A store an earlier build wrote can hold a NULL under the
///    declaration: `EDGES.md` row 38.)
/// 4. If the scan is already an `IndexRangeScan` (a range WHERE), it must be on
///    the SAME column as the order key — can't drive two indexes; differing
///    columns bail.
///
/// With acceleration off (the stripped "testing" profile) there is no ordered
/// index scan to elide into, so this always declines — the caller keeps the `Sort`.
#[cfg(not(feature = "index_acceleration"))]
const fn try_order_by_index_pushdown(
    plan: Plan,
    _order_by: &[OrderKey],
    _table: &str,
    _schema: &Schema,
) -> (Plan, bool) {
    (plan, false)
}

#[cfg(feature = "index_acceleration")]
fn try_order_by_index_pushdown(
    plan: Plan,
    order_by: &[OrderKey],
    table: &str,
    schema: &Schema,
) -> (Plan, bool) {
    // Gate 2a: exactly one ORDER BY key, and it is a bare column.
    let [key] = order_by else {
        return (plan, false);
    };
    let Expr::Column {
        name: order_col, ..
    } = &key.expr
    else {
        return (plan, false);
    };
    if order_col == "*" {
        return (plan, false);
    }

    // Gate 2b: the order column has a single-column full index.
    let Some(index) = single_column_index_for(schema, table, order_col) else {
        return (plan, false);
    };

    // Gate 3: the column is NOT NULL. A **nullable** column can never elide,
    // regardless of the requested/defaulted NULL placement: an index range read
    // answers no NULL-keyed row — a plain B-tree index holds no entry for one, a
    // `NULLS NOT DISTINCT` index holds it under a key no range read returns — so an
    // ordered index scan would *omit* every NULL row, not merely misplace it. No
    // statement this build accepts is known to leave a NULL under a NOT NULL
    // declaration (a store an earlier build wrote can hold one: `EDGES.md` row 38), so
    // any `NULLS FIRST/LAST` clause is vacuous and elision is order-correct. A nullable
    // column keeps the `Sort`, which honors the placement (see `compare_order_values`).
    let column_is_not_null = schema
        .get_table(table)
        .and_then(|t| t.columns.iter().find(|c| &c.name == order_col))
        .is_some_and(|c| !c.nullable);
    if !column_is_not_null {
        return (plan, false);
    }

    rewrite_scan_for_order(plan, order_col, index.oid, key.asc)
}

/// Rewrite the base-table scan under `plan` to an ordered `IndexRangeScan` on
/// `order_col`, preserving a wrapping `Project`. Returns `(rewritten, true)`, or
/// `(plan, false)` (the plan moved back out untouched) if the shape isn't a
/// pushdown candidate — an `Aggregate`/`Window`/`Filter`/`Join`, or an
/// `IndexRangeScan` on a different column. The bail path is lossless: no node is
/// consumed when the rewrite declines.
#[cfg(feature = "index_acceleration")]
fn rewrite_scan_for_order(
    plan: Plan,
    order_col: &str,
    index_oid: i32,
    ascending: bool,
) -> (Plan, bool) {
    match plan {
        // A wrapping projection: rewrite the input, re-wrap the Project (on the
        // bail path the input is moved back, so the Project is rebuilt intact).
        Plan::Project {
            input,
            exprs,
            certain,
            enclosing,
        } => {
            let (inner, elided) = rewrite_scan_for_order(*input, order_col, index_oid, ascending);
            (
                Plan::Project {
                    input: Box::new(inner),
                    exprs,
                    certain,
                    enclosing,
                },
                elided,
            )
        }
        // A plain table scan → an unbounded ordered IndexRangeScan.
        Plan::Scan { table, alias } => (
            Plan::IndexRangeScan {
                table,
                alias,
                index_oid,
                key_column: order_col.to_string(),
                lower: None,
                upper: None,
                ascending,
                limit: None,
            },
            true,
        ),
        // An existing IndexRangeScan (from a range WHERE): reuse it, just set the
        // scan direction — but only when it is on the SAME column as the order
        // key. A different column can't be served by this index, so the node is
        // rebuilt with its existing direction and the caller keeps the `Sort`.
        Plan::IndexRangeScan {
            table,
            alias,
            index_oid: existing_oid,
            key_column,
            lower,
            upper,
            ascending: existing_ascending,
            limit,
        } => {
            let same = key_column == order_col;
            (
                Plan::IndexRangeScan {
                    table,
                    alias,
                    index_oid: existing_oid,
                    key_column,
                    lower,
                    upper,
                    ascending: if same { ascending } else { existing_ascending },
                    limit,
                },
                same,
            )
        }
        // Anything else (Aggregate, Window, Filter residual, Join, …) → not a
        // candidate; move the plan back unchanged.
        other => (other, false),
    }
}

/// Set the `limit` on the elided ordered scan (an early-stop), reaching through a
/// wrapping `Project`. A no-op for any other shape.
///
/// With acceleration off there is no `IndexRangeScan` to early-stop (and no `Sort`
/// is ever elided), so this is a no-op — the outer `Plan::Limit` is the only bound.
#[cfg(not(feature = "index_acceleration"))]
const fn set_index_scan_limit(_plan: &mut Plan, _count: LimitCount) {}

#[cfg(feature = "index_acceleration")]
fn set_index_scan_limit(plan: &mut Plan, count: LimitCount) {
    match plan {
        Plan::Project { input, .. } => set_index_scan_limit(input, count),
        Plan::IndexRangeScan { limit, .. } => *limit = Some(count),
        _ => {}
    }
}

/// projection-level unnest, distinct from the FROM-clause `Plan::FunctionScan`). Used to
/// decide whether a no-FROM projection must row-expand.
pub fn contains_unnest(expr: &Expr) -> bool {
    match expr {
        Expr::Unnest(_) => return true,
        // Deliberately **narrower** than the structural walkers: these are treated as
        // leaves (no descent), so a nested `unnest` inside them stays undetected. A
        // single such `unnest` fails loud identically either way (`eval_expr` rejects a
        // scalar-position `unnest`); broadening would only change the loud-error path on
        // the contrived multi-nested-`unnest` edge, so the narrow set is preserved.
        Expr::Aggregate { .. }
        | Expr::InSubquery { .. }
        | Expr::Lower(_)
        | Expr::Coalesce(_)
        | Expr::Case { .. }
        | Expr::Window { .. } => return false,
        _ => {}
    }
    let mut found = false;
    expr.for_each_child(|child| found = found || contains_unnest(child));
    found
}

/// Shared context for the [`validate_grouped_columns`] walk.
struct GroupCtx<'a> {
    /// The GROUP BY expressions (for whole-expression matching, e.g. `GROUP BY
    /// lower(x)` legitimizing a projected `lower(x)`).
    group_by: &'a [Expr],
    /// Bare-column names that are top-level GROUP BY keys.
    grouped_cols: &'a HashSet<&'a str>,
    /// `true` when the table's full primary key is grouped, so every column is
    /// functionally dependent on the GROUP BY and thus allowed.
    fd_all_columns: bool,
    /// The single base table being grouped (for column-membership checks).
    table: &'a Table,
    /// The name a qualified reference to [`Self::table`] must spell — its FROM alias
    /// if it has one, else the table name. A reference qualified with anything else
    /// belongs to another relation (an enclosing query level), not this one.
    effective: &'a str,
}

impl GroupCtx<'_> {
    /// Whether a column reference qualified `qualifier` addresses the relation being
    /// grouped. See [`is_own_relation`].
    fn is_own_relation(&self, qualifier: Option<&str>) -> bool {
        is_own_relation(self.effective, qualifier)
    }
}

/// Whether a column reference qualified `qualifier` addresses the relation whose
/// effective name (its FROM alias if it has one, else the table name) is `effective`.
///
/// An unqualified reference does — it can only bind here, since the grouping walk runs
/// only for a single-table query. A reference qualified with another name belongs to
/// another relation (an enclosing query level) and the grouping rule does not reach it.
///
/// Free-standing because the two callers straddle [`GroupCtx`]'s construction: the
/// `grouped_cols` set is built *before* the context exists, and the projection walk
/// applies the same rule *through* it.
fn is_own_relation(effective: &str, qualifier: Option<&str>) -> bool {
    qualifier.is_none_or(|q| q == effective)
}

/// Enforce Postgres's `42803` grouping rule at plan time: in an aggregated query
/// (`GROUP BY`, any aggregate, or `HAVING`), every projected / `HAVING` column
/// must be grouped, sit inside an aggregate, or be **functionally dependent** on
/// the GROUP BY — the table's full primary key being grouped implies every column
/// (PG allows `SELECT pk, other … GROUP BY pk`). Without this, a non-grouped
/// column silently returned an arbitrary group row.
///
/// Scoped to a single base table: in a JOIN a bare column's owning table is
/// ambiguous without full multi-table resolution, so a JOIN is left unvalidated
/// (lenient — no false rejections; the engine never enforced this before and the
/// spine issues no GROUP BY). An expression-keyed primary key yields no FD
/// modeling (the `_pkey` index isn't a plain-column key).
fn validate_grouped_columns(
    projection: &[Expr],
    group_by: &[Expr],
    having: Option<&Expr>,
    tables: &[(String, Option<String>)],
    schema: &Schema,
) -> Result<()> {
    let [(table_name, alias)] = tables else {
        return Ok(());
    };
    let Some(table) = schema.get_table(table_name) else {
        return Ok(());
    };
    let effective = alias.as_deref().unwrap_or(table_name.as_str());
    // Qualifier-aware: only a key that addresses *this* relation grounds a projected
    // column. A key qualified with another name (`GROUP BY o.bar` inside a correlated
    // subquery) names an enclosing level's column and must not legitimize a same-named
    // column of this one. Same rule as `GroupCtx::is_own_relation`, which the
    // projection walk applies — shared here through the free function both call, so
    // the two cannot drift while `GroupCtx` is not yet built.
    let grouped_cols: HashSet<&str> = group_by
        .iter()
        .filter_map(|e| match e {
            Expr::Column { table: q, name } if is_own_relation(effective, q.as_deref()) => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect();
    let fd_all_columns = primary_key_columns(schema, table_name)
        .is_some_and(|pk| !pk.is_empty() && pk.iter().all(|c| grouped_cols.contains(c.as_str())));
    let ctx = GroupCtx {
        group_by,
        grouped_cols: &grouped_cols,
        fd_all_columns,
        table,
        effective,
    };
    for expr in projection {
        validate_grouped_expr(expr, &ctx)?;
    }
    if let Some(h) = having {
        validate_grouped_expr(h, &ctx)?;
    }
    Ok(())
}

/// Recursively check one expression against the grouping rule (see
/// [`validate_grouped_columns`]).
fn validate_grouped_expr(expr: &Expr, ctx: &GroupCtx) -> Result<()> {
    crate::depth::check_stack()?;
    // A projected expression that exactly matches a GROUP BY key is valid whole
    // (`GROUP BY a + b`, `SELECT a + b`) — checked before descending.
    if ctx.group_by.iter().any(|g| g == expr) {
        return Ok(());
    }
    match expr {
        // An aggregate consumes any columns inside it — do not descend.
        Expr::Aggregate { .. } => return Ok(()),
        Expr::Column { table, name } => {
            let ok = name == "*"
                || ctx.fd_all_columns
                // A reference qualified with another relation's name is an outer
                // reference from an enclosing query level, not a column of the
                // relation being grouped — the grouping rule does not reach it.
                // (`SELECT o.id, (SELECT o.id FROM i WHERE … GROUP BY i.k) FROM o`
                // is valid in Postgres even when `i` happens to have an `id` too.)
                || !ctx.is_own_relation(table.as_deref())
                || ctx.grouped_cols.contains(name.as_str())
                // A name that isn't a base column of this table (an output alias,
                // a join column) is left to the column-existence validator rather
                // than rejected here.
                || !ctx.table.columns.iter().any(|c| &c.name == name);
            return if ok {
                Ok(())
            } else {
                Err(Error::GroupingError {
                    column: name.clone(),
                })
            };
        }
        // Every other variant (column-free leaves, self-contained sub-plans, and the
        // compounds) is valid iff each of its children is — delegate to the shared
        // child set.
        _ => {}
    }
    expr.try_for_each_child(|child| validate_grouped_expr(child, ctx))
}

/// The plain-column primary key of a table, or `None` when absent or
/// expression-keyed (no functional-dependency modeling).
///
/// Found by the catalog's constraint marker ([`Schema::primary_key_index`]), not by
/// the `<table>_pkey` name — a `CONSTRAINT my_pk PRIMARY KEY (…)` names its index
/// `my_pk`, and a plain `CREATE UNIQUE INDEX t_pkey` is not a primary key at all.
fn primary_key_columns(schema: &Schema, table: &str) -> Option<Vec<String>> {
    match &schema.primary_key_index(table)?.key {
        IndexKey::Columns(cols) => Some(cols.clone()),
        IndexKey::Exprs(_) => None,
    }
}

/// Check if an expression contains any aggregate function calls.
pub fn contains_aggregate(expr: &Expr) -> bool {
    match expr {
        // Found one — its own arguments are immaterial.
        Expr::Aggregate { .. } => return true,
        // A window function is not an aggregate, and (historically) its operands are a
        // separate scope — treat it as a leaf, never descended into.
        Expr::Window { .. } => return false,
        _ => {}
    }
    // A subquery/EXISTS is a self-contained scalar from the outer query's view (a leaf
    // in the child set); every other compound contains an aggregate iff some child does.
    let mut found = false;
    expr.for_each_child(|child| found = found || contains_aggregate(child));
    found
}

/// Check if an expression contains any window function calls.
pub fn contains_window(expr: &Expr) -> bool {
    if let Expr::Window { .. } = expr {
        return true;
    }
    // Descends into an aggregate's argument (`for_each_child` yields it) and every
    // other compound's children; a subquery/EXISTS is a self-contained leaf. So the
    // expression contains a window iff some child does.
    let mut found = false;
    expr.for_each_child(|child| found = found || contains_window(child));
    found
}

#[cfg(test)]
mod group_by_tests {
    use crate::catalog::{Column, ConstraintKind, Index, IndexKey, Schema, Table, Uniqueness};
    use crate::parser::parse;
    use crate::types::TypeOid;

    /// A schema with `grp(pk INT PRIMARY KEY, name TEXT, other TEXT)`.
    fn schema_with_pk() -> Schema {
        let col = |name: &str, type_oid, position| Column {
            name: name.to_string(),
            type_oid,
            nullable: name != "pk",
            position,
            default: None,
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            temporal_precision: None,
            identity: None,
        };
        let mut schema = Schema::new();
        schema.create_table(Table {
            name: "grp".to_string(),
            schema: "public".to_string(),
            columns: vec![
                col("pk", TypeOid::Int4, 1),
                col("name", TypeOid::Text, 2),
                col("other", TypeOid::Text, 3),
            ],
            oid: 1,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
        });
        schema.create_index(Index {
            name: "grp_pkey".to_string(),
            table_name: "grp".to_string(),
            key: IndexKey::Columns(vec!["pk".to_string()]),
            uniqueness: Uniqueness::for_constraint(ConstraintKind::PrimaryKey),
            predicate: None,
            nulls_not_distinct: false,
            oid: 2,
        });
        schema
    }

    fn sqlstate_of(sql: &str) -> Option<&'static str> {
        parse(sql, &schema_with_pk()).err().map(|e| e.sqlstate())
    }

    #[test]
    fn ungrouped_column_fails_loud() {
        // `other` is neither grouped nor aggregated → 42803.
        assert_eq!(
            sqlstate_of("SELECT other, COUNT(*) FROM grp GROUP BY name"),
            Some("42803"),
        );
        // Pure-aggregate query (no GROUP BY) with a stray bare column also errors.
        assert_eq!(
            sqlstate_of("SELECT other, COUNT(*) FROM grp"),
            Some("42803"),
        );
    }

    #[test]
    fn group_by_alias_ambiguity_compares_expressions_not_match_counts() {
        // Two entries aliased `dup` denoting **different** expressions → `42702`.
        assert_eq!(
            sqlstate_of("SELECT name AS dup, COUNT(*) AS dup FROM grp GROUP BY dup"),
            Some("42702"),
        );
        // The same duplication with **equal** expressions is accepted by Postgres
        // (`equal(target_result->expr, tle->expr)`), so a count-based check is wrong.
        assert!(
            parse(
                "SELECT name AS dup, name AS dup FROM grp GROUP BY dup",
                &schema_with_pk(),
            )
            .is_ok()
        );
        // The alias is peeled before comparing: `name` and `name AS dup` are the same
        // expression wearing different labels.
        assert!(
            parse(
                "SELECT name, name AS name FROM grp GROUP BY name",
                &schema_with_pk()
            )
            .is_ok()
        );
    }

    #[test]
    fn group_by_from_column_precheck_precedes_the_alias_ambiguity_check() {
        // `name` is a FROM column, so the targetlist alias search never runs and the
        // two entries aliased `name` are never compared — `42803` (ungrouped `other`),
        // not `42702`.
        assert_eq!(
            sqlstate_of("SELECT other AS name, pk AS name FROM grp GROUP BY name"),
            Some("42803"),
        );
    }

    #[test]
    fn group_by_window_entry_is_a_windowing_error_not_a_grouping_error() {
        // Postgres's `checkTargetlistEntrySQL92` splits the two: aggregates are
        // `42803`, window functions `42P20`. Both the alias and ordinal spellings.
        assert_eq!(
            sqlstate_of("SELECT ROW_NUMBER() OVER () AS w FROM grp GROUP BY w"),
            Some("42P20"),
        );
        assert_eq!(
            sqlstate_of("SELECT ROW_NUMBER() OVER () AS w FROM grp GROUP BY 1"),
            Some("42P20"),
        );
        assert_eq!(
            sqlstate_of("SELECT COUNT(*) AS c FROM grp GROUP BY c"),
            Some("42803"),
        );
    }

    #[test]
    fn unary_plus_is_not_a_clause_ordinal() {
        // `+1` keeps an `Expr::UnaryPlus` wrapper, so it is not a bare literal and
        // never reads as a position — matching Postgres, where only unary minus folds
        // into a constant. The key is an ordinary constant, leaving `name` ungrouped.
        assert_eq!(
            sqlstate_of("SELECT name FROM grp GROUP BY +1"),
            Some("42803")
        );
        // And the plain literal still *is* an ordinal.
        assert!(parse("SELECT name FROM grp GROUP BY 1", &schema_with_pk()).is_ok());
    }

    #[test]
    fn grouped_and_aggregated_columns_pass() {
        // `name` grouped, `COUNT(*)` aggregated, `other` inside an aggregate.
        assert!(
            parse(
                "SELECT name, COUNT(*), MAX(other) FROM grp GROUP BY name",
                &schema_with_pk(),
            )
            .is_ok()
        );
    }

    #[test]
    fn functional_dependency_on_pk_allows_any_column() {
        // The full PK is grouped, so every column is functionally dependent.
        assert!(
            parse(
                "SELECT pk, name, other, COUNT(*) FROM grp GROUP BY pk",
                &schema_with_pk(),
            )
            .is_ok()
        );
        // Grouping by a non-PK column does NOT grant the dependency.
        assert_eq!(
            sqlstate_of("SELECT pk FROM grp GROUP BY name"),
            Some("42803"),
        );
    }

    #[test]
    fn grouped_whole_expression_passes() {
        // A projected expression that matches a GROUP BY key is valid as a whole.
        assert!(
            parse(
                "SELECT LOWER(name), COUNT(*) FROM grp GROUP BY LOWER(name)",
                &schema_with_pk(),
            )
            .is_ok()
        );
    }
}

/// Behavior of the `for_each_child`-based structural walkers, including the
/// divergences each keeps explicit (the `Window`-leaf and narrow-`unnest` boundaries).
#[cfg(test)]
mod walker_tests {
    use super::{collect_referenced_columns, contains_aggregate, contains_unnest, contains_window};
    use crate::planner::expr::{AggregateFunc, BinaryOp, Expr, Literal, WindowFunc};
    use crate::types::TypeOid;

    fn col(name: &str) -> Expr {
        Expr::Column {
            table: None,
            name: name.to_string(),
        }
    }

    fn count(arg: Expr) -> Expr {
        Expr::Aggregate {
            func: AggregateFunc::Count,
            arg: Box::new(arg),
            distinct: false,
            certainty: crate::planner::ArgCertainty::NotCertainInteger,
        }
    }

    fn row_number(partition_by: Vec<Expr>) -> Expr {
        Expr::Window {
            func: WindowFunc::RowNumber,
            partition_by,
            order_by: vec![],
        }
    }

    #[test]
    fn collect_referenced_columns_walks_every_child() {
        // COALESCE(a, CASE WHEN b THEN c ELSE d END) — every base column is collected.
        let case = Expr::Case {
            operand: None,
            when_clauses: vec![(col("b"), col("c"))],
            else_result: Some(Box::new(col("d"))),
        };
        let mut out = Vec::new();
        collect_referenced_columns(&Expr::Coalesce(vec![col("a"), case]), &mut out);
        assert_eq!(
            out,
            ["a", "b", "c", "d"].map(|n| (None, n.to_string())).to_vec()
        );
        // The `*` wildcard contributes nothing.
        let mut wild = Vec::new();
        collect_referenced_columns(&col("*"), &mut wild);
        assert!(wild.is_empty());
    }

    #[test]
    fn collect_referenced_columns_keeps_the_qualifier() {
        // `pb.x` and a bare `x` are distinct references — collapsing them to the bare
        // name is what let a sort key on one relation bind to the other's column.
        let qualified = Expr::Column {
            table: Some("pb".to_string()),
            name: "x".to_string(),
        };
        let mut out = Vec::new();
        collect_referenced_columns(&qualified, &mut out);
        collect_referenced_columns(&col("x"), &mut out);
        assert_eq!(
            out,
            vec![
                (Some("pb".to_string()), "x".to_string()),
                (None, "x".to_string()),
            ]
        );
    }

    #[test]
    fn contains_aggregate_treats_window_as_a_leaf() {
        // An aggregate nested in a CASE is found.
        let case = Expr::Case {
            operand: None,
            when_clauses: vec![(Expr::Literal(Literal::Bool(true)), count(col("a")))],
            else_result: None,
        };
        assert!(contains_aggregate(&case));
        // ...but an aggregate inside a window's PARTITION BY is NOT — a window is a leaf
        // for `contains_aggregate` (its operands are a separate scope).
        let win = row_number(vec![count(col("a"))]);
        assert!(!contains_aggregate(&win));
        // ...while `contains_window` sees the window itself.
        assert!(contains_window(&win));
    }

    #[test]
    fn contains_window_descends_into_aggregate_arg() {
        // A window inside an aggregate's argument — `contains_window` descends into it,
        // and `contains_aggregate` is true at the top aggregate.
        let agg = count(row_number(vec![]));
        assert!(contains_window(&agg));
        assert!(contains_aggregate(&agg));
    }

    #[test]
    fn contains_unnest_keeps_its_narrow_set() {
        let unnest = Expr::Unnest(Box::new(col("a")));
        // Detected through the structural wrappers it descends into.
        assert!(contains_unnest(&Expr::Cast {
            expr: Box::new(unnest.clone()),
            target_type: TypeOid::Text,
        }));
        assert!(contains_unnest(&Expr::Array(vec![unnest.clone()])));
        assert!(contains_unnest(&Expr::BinaryOp {
            left: Box::new(unnest.clone()),
            op: BinaryOp::Eq,
            right: Box::new(col("b")),
        }));
        // ...but NOT through the deliberately-excluded variants (narrower than the other
        // walkers): COALESCE, CASE, LOWER, an aggregate, a window.
        assert!(!contains_unnest(&Expr::Coalesce(vec![unnest.clone()])));
        assert!(!contains_unnest(&Expr::Case {
            operand: None,
            when_clauses: vec![(col("b"), unnest.clone())],
            else_result: None,
        }));
        assert!(!contains_unnest(&Expr::Lower(Box::new(unnest.clone()))));
        assert!(!contains_unnest(&count(unnest.clone())));
        assert!(!contains_unnest(&row_number(vec![unnest])));
    }
}
