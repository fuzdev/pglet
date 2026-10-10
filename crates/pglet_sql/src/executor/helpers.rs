//! Projection and descriptor helpers for query execution: filter matching,
//! column-descriptor building, `RETURNING` projection, executor-local expression
//! type inference, and OID generation for new catalog objects.

use std::collections::BTreeMap;

use crate::Result;
use crate::catalog::{Column, EnumType, Schema, Table};
use crate::planner::{
    BinaryOp, EnclosingColumn, Expr, LeafScope, Plan, aggregate_type_of, arith_result_type,
    find_enclosing, numeric_family_result, shared_leaf_cell_type, shared_multi_operand_type,
};
use crate::types::{ResultShape, TypeOid, Value};
use pglet_core::RelationKind;

use super::Storage;
use super::expr::{
    ColumnRef, ColumnResolver, EvalEnv, JoinColumnMap, ScopeIdentity, SimpleColumns,
};
use super::query::eval_expr_subquery_with;
use super::types::{ColumnDesc, QueryResult};

/// Borrow a table from the schema, or fail loud with `42P01` (table not found) — the
/// single read-only table lookup the executor's scan / DDL paths share.
///
/// A name **another relation kind** holds is `42809` instead, because it is not
/// missing: Postgres answers `42809` to every scan, DML, `COPY`, `TRUNCATE`,
/// `DROP TABLE`, `ALTER TABLE`, `CREATE INDEX … ON` and `REFERENCES` naming a
/// sequence *or an index*, and pglet reporting `42P01` made one name give two
/// contradictory answers about its own existence (`SELECT … FROM x` said it did not
/// exist; `CREATE TABLE x` said it already did).
///
/// The kind comes from [`Schema::relation_kind`], the one enumeration of the relation
/// namespace, rather than from a probe of a single map — reading only `sequences`
/// here is what left every index-side shape answering `42P01`, and left the two
/// `IF EXISTS` spellings *silently succeeding*. See
/// [`Error::WrongObjectType`](crate::Error::WrongObjectType), whose doc names the one
/// direction (reading a sequence) that is a pglet divergence rather than a refusal
/// Postgres shares.
pub fn get_table_or_err<'a>(schema: &'a Schema, table: &str) -> Result<&'a Table> {
    if let Some(t) = schema.get_table(table) {
        return Ok(t);
    }
    if let Some(actual) = schema.relation_kind(table) {
        // 18.6 has **two** wordings for a wrong-kind relation and they come from
        // different functions: `validate_relation_kind` (`table.c`) answers a scan,
        // DML, `COPY`, `CREATE INDEX … ON` or `REFERENCES` with `cannot open relation
        // "x"` plus `This operation is not supported for indexes.` in the DETAIL,
        // while `DropErrorMsgWrongType` answers a `DROP` with `"x" is not a table`
        // plus a `Use DROP INDEX …` HINT. This is the first route
        // (`Error::WrongRelationKind`); `resolve_relation_for_drop` is the second
        // (`Error::WrongObjectType`). Both measured on 18.6.
        return Err(crate::Error::WrongRelationKind {
            relation: table.to_string(),
            kind: actual,
        });
    }
    Err(crate::CatalogError::TableNotFound {
        table: table.to_string(),
    }
    .into())
}

/// The verdict of a `DROP`'s **phase-1** name resolution — see
/// [`resolve_relation_for_drop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropResolution {
    /// The name holds a relation of the expected kind: it is a target of this drop.
    Found,
    /// The name holds no relation at all and `IF EXISTS` was written: Postgres emits
    /// one `NOTICE` per **written** name and carries on with the rest of the list.
    Skipped,
}

/// Resolve one `DROP {TABLE | INDEX | SEQUENCE}` name against the relation namespace,
/// with the kind check and the `IF EXISTS` skip in Postgres's order.
///
/// This is `RemoveRelations`' per-name half, and both halves of it are load-bearing:
///
/// - **The kind is decided on a name that resolves**, so a relation of the wrong kind
///   is `42809` *whether or not* `IF EXISTS` was written — only a name no relation
///   holds at all reaches `DropErrorMsgNonExistent`'s skip (measured on 18.6:
///   `DROP TABLE IF EXISTS <a sequence>`, `DROP INDEX IF EXISTS <a table>` and
///   `DROP SEQUENCE IF EXISTS <a table>` are all `42809`). Read the other way round
///   the class goes *silent*: the statement reports success and the next one to name
///   that relation insists it already exists.
/// - **The lookup reads the whole namespace** ([`Schema::relation_kind`]), not one
///   map, because the kind is a property of the (found, expected) pair — a check
///   written against `schema.sequences` alone is right about a third of the class.
///
/// It exists as one function because the three multi-name drops resolve their names
/// in a loop, and a gate copied into three loops is three gates. The "not found"
/// spelling is the only per-kind part: `42P01` for a table or a sequence, `42704` for
/// an index (measured on 18.6).
///
/// # Errors
///
/// [`Error::WrongObjectType`](crate::Error::WrongObjectType) (`42809`) for a relation
/// of another kind, or the kind's own "does not exist" error when `if_exists` is
/// false.
pub fn resolve_relation_for_drop(
    schema: &Schema,
    name: &str,
    expected: RelationKind,
    if_exists: bool,
) -> Result<DropResolution> {
    match schema.relation_kind(name) {
        Some(actual) if actual == expected => Ok(DropResolution::Found),
        Some(actual) => Err(crate::Error::WrongObjectType {
            relation: name.to_string(),
            actual,
            expected,
            route: crate::WrongTypeRoute::Drop,
        }),
        None if if_exists => Ok(DropResolution::Skipped),
        None => Err(match expected {
            RelationKind::Table => crate::CatalogError::TableNotFound {
                table: name.to_string(),
            }
            .into(),
            RelationKind::Index => crate::CatalogError::IndexNotFound {
                index: name.to_string(),
            }
            .into(),
            RelationKind::Sequence => crate::Error::SequenceNotFound(name.to_string()),
        }),
    }
}

/// The `NOTICE` text a skipped `DROP … IF EXISTS` name raises, in Postgres's wording
/// (`DropErrorMsgNonExistent`), and **one per written name** — a list naming the same
/// missing relation twice notices twice, because the skip is per list element.
#[must_use]
pub fn drop_skip_notice(kind: RelationKind, name: &str) -> String {
    let noun = match kind {
        RelationKind::Table => "table",
        RelationKind::Index => "index",
        RelationKind::Sequence => "sequence",
    };
    format!("{noun} \"{name}\" does not exist, skipping")
}

/// Build column descriptors from a table definition.
///
/// Each descriptor carries its wire metadata straight from the catalog: the
/// column's `atttypmod` (numeric/varchar modifier) and its base-table provenance
/// (the table OID + 1-based attnum), so a `SELECT *` / scan reports them in
/// `RowDescription`. Projection propagates them for a passthrough column.
pub fn table_column_descs(table: &Table) -> Vec<ColumnDesc> {
    table
        .columns
        .iter()
        .map(|c| ColumnDesc {
            name: c.name.clone(),
            type_oid: c.type_oid,
            type_modifier: c.pg_type_modifier(),
            table_oid: table.oid,
            column_id: c.position,
            enum_type: c.enum_type.clone(),
            // A planner fact (`ColumnDesc::type_certain`): no reader of a descriptor the
            // executor builds asks it.
            type_certain: false,
        })
        .collect()
}

/// The **ordered member labels** of the user `ENUM` named `enum_type`, or `None` when
/// the column declares none (or the catalog has no such type).
///
/// The one place an enum's *name* becomes an enum's *order*, borrowed straight out of
/// `Schema.enums`. A site that can hold the borrow does; the three that cannot copy it
/// (`JoinColumnMap`, `PreparedIndex`, `IndexSpec::column_enum_orders`), and only
/// the last outlives the statement that took it — which is why `Schema.enums` moving
/// has to reach it (`docs/enums.md` § A copy that outlives its statement). See
/// [`ColumnResolver::column_enum_order`](super::expr::ColumnResolver::column_enum_order).
#[must_use]
pub fn enum_order_of<'s>(
    enums: &'s BTreeMap<String, EnumType>,
    enum_type: Option<&str>,
) -> Option<&'s [String]> {
    enums.get(enum_type?).map(|e| -> &'s [String] { &e.labels })
}

/// Each descriptor's enum member order, positionally parallel to `columns` — the slice
/// an executor node attaches with `SimpleColumns::with_enum_orders`.
///
/// Derived beside the node's `col_types` and from the **same** descriptors, so the
/// order lands on exactly the column the value and the declared type came from.
#[must_use]
pub fn column_enum_orders<'s>(
    columns: &[ColumnDesc],
    enums: &'s BTreeMap<String, EnumType>,
) -> Vec<Option<&'s [String]>> {
    enum_orders(columns.iter().map(|c| c.enum_type.as_deref()), enums)
}

/// [`column_enum_orders`] over a table's **catalog** columns — the write path's form,
/// where the row is the table's own shape rather than a node's output.
#[must_use]
pub fn table_enum_orders<'s>(
    columns: &[Column],
    enums: &'s BTreeMap<String, EnumType>,
) -> Vec<Option<&'s [String]>> {
    enum_orders(columns.iter().map(|c| c.enum_type.as_deref()), enums)
}

/// The enum type an output column **unified from two others** declares —
/// [`unify_enum_pair`](super::expr::unify_enum_pair) over the two descriptors' own
/// `enum_type`, which is where the rule lives.
///
/// Such a column is typed by a unified type and is built with no passthrough wire
/// metadata — where 18.6 keeps the sides' agreeing modifier, `EDGES.md` row 155 — but
/// the enum is not wire metadata, it is what says the column's `ORDER BY` orders by
/// member position, and 18.6 types both spellings that reach here as the enum:
/// `SELECT m FROM t UNION ALL SELECT m FROM t`, and the `COALESCE(l.m, r.m)` a
/// `JOIN … USING (m)` merges.
///
/// **The side that carries no enum is the one that reaches this in practice**, and
/// reading it as a veto was wrong: `SELECT m FROM t UNION ALL SELECT 'ok'` is a `mood`
/// column on 18.6 — an `unknown` literal takes the other branch's type — where a
/// same-enum-only rule left it on the label bytes, ordering `happy, ok, ok, sad` against
/// 18.6's `sad, ok, ok, happy` and answering a **different row** for
/// `WHERE x > 'ok'`. A `NULL` branch, the literal branch on the left, `UNION` and
/// `EXCEPT` are the same shape. Two *different* enums stay `None` — a pair 18.6 refuses
/// to unify at all, left on the label-byte order rather than given an invented verdict.
///
/// **The pairs this is asked about are the pairs 18.6 itself unifies.** An enum against
/// a plain `text` **column** is indistinguishable from the literal here — both sides read
/// `None` — and that used to hand the merged column the enum's order where 18.6 refuses
/// the pair outright. It no longer arrives: `executor::query::join`'s `resolve_using` and
/// `planner::query`'s merge both run `comparison_legality` over
/// `ResolvedType::of_column` first, and the set-operation fold goes through
/// `select_common_type` over `ResolvedType`s, so an enum beside a `text`/`varchar`/`name`
/// column is `42883`/`42804` before this is reached, and two *different* enums are
/// `42883`/`42846` (`EDGES.md` rows 114, 115 and 109, closed). What still reaches the
/// `(Some, None)` asymmetry is the operand kind 18.6 also unifies — an `unknown` literal —
/// and the one this engine cannot tell from it, a `::text`-cast side (row 104).
///
/// **Two shapes, four sites**, because each shape's descriptors are built twice — once
/// by the executor's node and once by `Plan::output_columns_scoped` — and neither reads
/// the other: `executor::query::mod`'s `SetOp` arm, `executor::query::join`'s
/// `resolve_using`, and the two matching arms in `planner::plan`. Re-derive that list by
/// grep rather than from this sentence; the `USING` pair is what the set-op pair's own
/// "no single-column wire metadata" comment was copied into, and it kept the label-byte
/// order for a merged enum column a whole slice longer.
#[must_use]
pub fn unified_enum_type(left: &ColumnDesc, right: &ColumnDesc) -> Option<String> {
    super::expr::unify_enum_pair(left.enum_type.clone(), right.enum_type.clone())
}

/// The shared body of the two above: one `enum_order_of` per column, in order.
fn enum_orders<'s, 'n>(
    enum_types: impl Iterator<Item = Option<&'n str>>,
    enums: &'s BTreeMap<String, EnumType>,
) -> Vec<Option<&'s [String]>> {
    enum_types.map(|n| enum_order_of(enums, n)).collect()
}

/// One relation of a `RETURNING` clause's namespace: the name a qualifier must spell
/// (the FROM alias when one was written, else the relation's own name) and that
/// relation's columns, in declaration order.
///
/// The executor's counterpart of the planner's `returning_relations`, and the shape it
/// is in is the point: a `(relation, column)` **pair** rather than a `relation.column`
/// string. See [`project_returning`] § The namespace is a pair, never a string.
pub type ReturningRelation<'a> = (&'a str, &'a [ColumnDesc]);

/// The `RETURNING` namespace as a relation-keyed resolver: a qualified reference is an
/// exact `(relation, column)` match (a qualifier no relation answers to is `42P01`, a
/// name the named relation lacks is `42703`), and a bare one counts the relations
/// carrying the name — more than one is `42702`, none is `42703`. Those are the verdicts
/// `ColumnScope::check_binding` reaches at plan time; the two are separate derivations of
/// the same relation list and are pinned against each other by fixture.
///
/// `JoinColumnMap` is the executor's existing object for exactly this, and the one
/// `collect_updated_from` already builds for the `WHERE` / `SET` clauses of the very
/// same statement, so the two clauses of an `UPDATE … FROM` now read one kind of
/// namespace rather than two.
fn returning_column_map(
    relations: &[ReturningRelation<'_>],
    enums: &BTreeMap<String, EnumType>,
) -> JoinColumnMap {
    let mut map = JoinColumnMap::new();
    let mut base = 0;
    for (relation, columns) in relations {
        for (i, c) in columns.iter().enumerate() {
            map.add_column(
                relation,
                &c.name,
                base + i,
                c.type_oid,
                enum_order_of(enums, c.enum_type.as_deref()).map(<[String]>::to_vec),
            );
        }
        base += columns.len();
    }
    map
}

/// The projected row's descriptors, positionally parallel to the row: every relation's
/// columns concatenated in FROM order.
///
/// A single-relation namespace keeps its columns' **bare** names; a multi-relation one
/// renames each to `relation.column`. That renaming is **not** how this clause resolves
/// anything (see [`project_returning`]); it is the addressing the correlated
/// [`OuterFrame`](super::expr::OuterFrame) uses, and a name a user quoted into existence
/// with a dot in it is byte-identical to one of these — the collision recorded in
/// `EDGES.md`. The two halves of this function are what makes
/// `EvalEnv::resolve_outer_slot`'s `table.is_none()` gate sound *for this clause*: the
/// *synthesized* names are emitted only by the branch whose frame also has no `table`,
/// so a dotted name in a frame that does carry one is a user's spelling rather than this
/// addressing, and the gate refuses it there. A frame with no `table` still cannot tell
/// the two apart, which is the half of the collision that remains.
///
/// **That argument is per-producer and does not generalize to the gate**, which is where
/// it was previously written one step too wide. It says a frame *with* a `table` never
/// carries this addressing; it says nothing about a frame that lacks one for a reason
/// other than being multi-relation, and two call sites used to hand `None` to
/// single-relation scopes that had perfectly good names — an unaliased `UPDATE` and a
/// FROM-clause `unnest(…)` — where the gate then had nothing to fire on and the dotted
/// tier answered a user's `"a.b"` column, on statements 18.6 refuses `42P01` and one of
/// them a **write**. Both are closed at their call sites rather than here; the soundness
/// of the gate is a property of what every producer of a frame passes, so a new one is a
/// new obligation.
/// `execute_join` emits the same *spelling* for its
/// output descriptors, but its own `(relation, column)` map is built by re-parsing each
/// input column's name rather than from this list, so the two carry different symptoms
/// and are not one mechanism.
fn returning_input_columns(relations: &[ReturningRelation<'_>]) -> Vec<ColumnDesc> {
    if let [(_, columns)] = relations {
        return columns.to_vec();
    }
    relations
        .iter()
        .flat_map(|(relation, columns)| {
            columns.iter().map(move |c| ColumnDesc {
                name: format!("{relation}.{}", c.name),
                ..c.clone()
            })
        })
        .collect()
}

/// Project rows according to RETURNING expressions.
///
/// `relations` **is** the clause's namespace: for a statement with no `FROM`, the target
/// table under its effective name (its alias when one was written), so a qualified
/// `o.id` resolves alias-awarely and a bogus qualifier is `42P01`; for an
/// `UPDATE … FROM`, the target followed by the source, in FROM order, so a qualified
/// `s.k` picks the source and a bare name both carry is `42702`.
///
/// # The namespace is a pair, never a string
///
/// This used to be a flat `&[ColumnDesc]` whose multi-relation names were synthesized
/// `relation.column` strings, resolved by `SimpleColumns`'s exact-then-suffix tiers. A
/// user column quoted into existence with a dot in its name is **byte-identical** to
/// such a synthesized name, and the exact tier runs first and wins outright, so the
/// clause answered a different relation's column with no error on either engine —
/// measured against 18.6 both ways round, with the target `wt(id, v)` and the source
/// `sd s(id, "wt.v")`: `RETURNING "wt.v"` answered the *target's* `v` where 18.6
/// answers the source's column, and with the target `qt(id, "s.k")` and the source
/// `ws s(id, v, k)`, `RETURNING "s.k"` answered the *source's* `k` where 18.6 answers
/// the target's column. Both spellings' qualified controls (`s."wt.v"` / `qt."s.k"`)
/// were correct on both engines throughout, which is what identifies the resolution
/// rather than the column name. The executor also declared the wrong column's *type*
/// (`int4` on the simple-query leg where 18.6 and pglet's own `Describe` both say
/// `text`), so a binary client decoded four int4 bytes as text.
///
/// Resolution therefore keys on the `(relation, column)` pair
/// ([`returning_column_map`]), which no string can forge. The synthesized *names* survive
/// on the descriptors — `col_names` below, and a [`ProjectionScope`] that consults its
/// `names` only when it has no resolver — but nothing resolves through them any more: the
/// correlated outer frame is handed this clause's own pairs
/// ([`ColumnRef`](super::expr::ColumnRef)), so a **bare** dotted reference inside a
/// `RETURNING` subquery — the one path no qualifier gate can reach — answers the relation
/// whose column carries that name rather than the first descriptor spelled it.
/// A join wears the same *spelling* on its output descriptors by a second route,
/// `register_input_columns`, which takes its pairs from the plan for the same reason.
///
/// Each `RETURNING` item is evaluated with **full subquery support**
/// (`eval_expr_subquery_with`), so a scalar correlated subquery — the spine's
/// `RETURNING *, (SELECT COUNT(*) … WHERE cell_id = cell.id) AS grant_count` — runs
/// against the just-written row.
// The execution-context triple (schema/storage/env) is threaded individually, as
// across the DML executor — bundling it would diverge from `execute_insert`/`_update`.
pub fn project_returning<S: Storage>(
    rows: &[Vec<Value>],
    relations: &[ReturningRelation<'_>],
    returning: &[Expr],
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
) -> Result<(Vec<Vec<Value>>, Vec<ColumnDesc>)> {
    let resolver = returning_column_map(relations, &schema.enums);
    let input_columns = returning_input_columns(relations);
    // The name list and single-table identity the correlated **outer frame** addresses
    // the projected row by; resolution of this clause's own items goes through
    // `resolver` instead.
    let col_names: Vec<&str> = input_columns.iter().map(|c| c.name.as_str()).collect();
    let table = match relations {
        [(relation, _)] => Some(*relation),
        _ => None,
    };
    // The correlated outer frame reads the same namespace this clause resolves through,
    // by the same `(relation, column)` pairs — so a bare dotted reference inside a
    // `RETURNING` subquery no longer answers off the synthesized `relation.column`
    // strings a user's quoted column is byte-identical to.
    let refs: Vec<ColumnRef> = relations
        .iter()
        .flat_map(|(relation, columns)| {
            columns
                .iter()
                .map(move |c| ColumnRef::of(relation, &c.name))
        })
        .collect();
    let identity = table.map_or(ScopeIdentity::Relations(&refs), ScopeIdentity::Single);

    // A sole **bare** `RETURNING *` is the whole projected row, already materialized —
    // and only for a single-relation namespace, whose `input_columns` carry the bare
    // names a client expects. (A joined namespace's are dotted, so answering from here
    // would emit `s.k` as a column name; it falls through to the projection instead,
    // which raises `42703` on the `*`.)
    //
    // **No client route reaches this arm at all today**: every route to a `RETURNING`
    // plan lowers it, and `lower_returning` expands every wildcard it sees. Reaching an
    // *unlowered* one would need a DDL between `Parse` and `Execute` — which bumps
    // `Schema::generation`, and both `describe_result_shape` and `execute_query_core` re-plan
    // on a generation mismatch. So the guard here is a second wall behind a closed door,
    // not a live check. (What is **not** the reason: that this is unreachable from a
    // unit test. `pglet_sql` already carries `pglet_testing`'s `oracle` feature as a
    // dev-dependency, so `MemStorage` is in reach and this function is `pub` — a test
    // could construct the call directly.)
    if relations.len() == 1 && returning.len() == 1 && returning[0].is_bare_star() {
        return Ok((rows.to_vec(), input_columns.clone()));
    }

    // Build output column descriptors
    let output_columns = expr_column_descs(
        returning,
        // The same resolver `eval_expr_subquery_with` below binds the row's values
        // through — the agreement `ProjectionScope` exists to guarantee.
        ProjectionScope::new(&input_columns, &col_names, table)
            .with_schema(schema)
            .with_resolver(&resolver),
    );
    // Declared types of the just-written row, so a `||` / `::text` over a
    // `timestamptz` column in RETURNING keeps its `+00` offset.
    let col_types: Vec<TypeOid> = input_columns.iter().map(|c| c.type_oid).collect();
    // ...and their enum member orders, so a `RETURNING m > 'sad'` orders by member
    // position (see `ColumnResolver::column_enum_order`).
    let col_enums = column_enum_orders(&input_columns, &schema.enums);

    // Project each row, the just-written row pushed as the correlated outer scope
    // so a subquery item (`(SELECT … WHERE cell_id = cell.id)`) resolves against it.
    let projected_rows: Vec<Vec<Value>> = rows
        .iter()
        .map(|row| {
            returning
                .iter()
                .map(|expr| {
                    eval_expr_subquery_with(
                        expr, &resolver, &col_names, &col_types, &col_enums, identity, row, schema,
                        storage, env,
                    )
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((projected_rows, output_columns))
}

/// Assemble the `RETURNING` / command-tag tail shared by INSERT / UPDATE / DELETE.
///
/// An empty `returning` short-circuits to a row-less [`QueryResult`] carrying only
/// `command_tag` (the count-bearing tag — `INSERT 0 n` / `UPDATE n` / `DELETE n` —
/// the sole thing that differs between the three callers); otherwise the just-written
/// `rows` are projected through [`project_returning`] against `relations`.
/// `rows` is the relevant per-statement row set (inserted / updated / deleted), each
/// row positionally parallel to `relations`' concatenated columns.
pub fn finish_returning<S: Storage>(
    rows: &[Vec<Value>],
    relations: &[ReturningRelation<'_>],
    returning: &[Expr],
    schema: &Schema,
    storage: &S,
    env: EvalEnv<'_>,
    command_tag: String,
) -> Result<QueryResult> {
    if returning.is_empty() {
        return Ok(QueryResult {
            shape: ResultShape::NoResultSet,
            rows: Vec::new(),
            command_tag,
            notices: Vec::new(),
            tx_command: None,
            swapped_sequences: Vec::new(),
        });
    }
    let (projected_rows, columns) =
        project_returning(rows, relations, returning, schema, storage, env)?;
    Ok(QueryResult {
        shape: ResultShape::Columns(columns),
        rows: projected_rows,
        command_tag,
        notices: Vec::new(),
        tx_command: None,
        swapped_sequences: Vec::new(),
    })
}

/// Get column name and type from an expression.
///
/// The **name** is delegated to `Expr::output_name` (the planner's name walk, which
/// is schema-free) so the two can't drift. The **type** uses an executor-local walk
/// (`expr_type_oid`), which resolves a `Column` against the already-materialized
/// `input_columns` and is **total** — an operand it cannot type reaches the next rule
/// as `TypeOid::Text` rather than as `None`. Both scopes implement one [`LeafScope`]
/// (this one included), and the cells where the totality cannot show are one
/// shared rule ([`shared_leaf_cell_type`](crate::planner::shared_leaf_cell_type)) —
/// but be exact about how far that reaches: it is the `Subquery` leaf both walks read
/// through the trait. This walk's `Column` arm still calls
/// [`ProjectionScope::resolve`] directly, where `Expr::infer_type`'s calls
/// `LeafScope::column_type`; the two bottom out in the same
/// [`ProjectionScope::column_desc`] and so answer alike, but that leaf is an
/// identity the pin corpus holds, not one the trait enforces. What separates the two
/// walks is that totality, the arms whose rule turns on it, and the four rules this
/// walk carries with no counterpart on the other. Each of those four leaves its arm's
/// **shared** rule in charge — three fill in for the `None` that rule declines with
/// (`array_of`, `array_element`, `arith_result_type`) and one shapes the operand fed to
/// it (`abs`'s bare-literal carve-out) — where the one rule now deleted *pre-empted* a
/// shared rule outright: a `float8` guess returned from `Expr::Aggregate`, an arm both
/// walks have, before `aggregate_result_type` ran at all.
pub fn expr_column_info(expr: &Expr, scope: ProjectionScope<'_>) -> (String, TypeOid) {
    (expr.output_name(), expr_type_oid(expr, scope))
}

/// Map each projection expression to its output [`ColumnDesc`] — the descriptor
/// list every projecting node (`Project`, `Aggregate`, `Window`, `VALUES`,
/// `ProjectUnnest`, `RETURNING`) builds from its output expressions.
///
/// A **passthrough column reference** (bare or aliased) carries the source
/// column's wire metadata — type, `atttypmod`, and table/attnum provenance —
/// through the projection (so `SELECT amt FROM t` reports amt's `numeric(p,s)`
/// modifier and base-table provenance just as `SELECT *` does); every other
/// expression yields a fresh descriptor with no modifier (`-1`) and no provenance.
pub fn expr_column_descs(exprs: &[Expr], scope: ProjectionScope<'_>) -> Vec<ColumnDesc> {
    exprs.iter().map(|e| expr_output_column(e, scope)).collect()
}

/// The output [`ColumnDesc`] for one projection expression (see
/// [`expr_column_descs`]) — propagating wire metadata for a passthrough column.
fn expr_output_column(expr: &Expr, scope: ProjectionScope<'_>) -> ColumnDesc {
    // A bare or aliased column reference is a passthrough: keep the source column's
    // type + typmod + provenance, only renaming to the projected output name.
    let (out_name, inner) = match expr {
        Expr::Alias { expr: inner, alias } => (alias.clone(), inner.as_ref()),
        other => (other.output_name(), other),
    };
    if matches!(inner, Expr::Column { .. })
        && let Some(src) = scope.resolve(inner)
    {
        return ColumnDesc {
            name: out_name,
            ..src.clone()
        };
    }
    // Not a resolvable base column: a fresh descriptor with no wire metadata — but
    // the **enum** is not wire metadata. It is what says this column's `ORDER BY`
    // orders by member position, and 18.6 types `COALESCE(m, 'sad')`, `min(m)` and
    // `CASE … THEN m END` as the enum, so an enclosing `ORDER BY q.mm` over a derived
    // table must see it (`docs/enums.md`).
    let (name, type_oid) = expr_column_info(expr, scope);
    ColumnDesc {
        enum_type: scope.expr_enum_name(expr),
        ..ColumnDesc::new(name, type_oid)
    }
}

/// The scope a projection's output descriptors resolve their column references
/// against: the input [`ColumnDesc`]s, their names, and the **effective
/// single-table name** the node's evaluator uses (its FROM alias if present, else
/// the table; `None` for a join or a no-FROM scope).
///
/// It exists so the *declared type* of a projected column and the *value* under it
/// are looked up by the same rule. They previously were not: this side matched a
/// qualified `q.c` only against an input column literally **named** `"q.c"`, which
/// a join's merged columns are and a base-table scan's are not — so
/// `SELECT t.x FROM t` found nothing, fell back to `TypeOid::Text`, and declared a
/// text column over an int4 value. Joins escaped only because
/// `register_input_columns` happens to produce the dotted spelling. Every text-protocol
/// client (psql, node-postgres, the wasm adapter, anything reading `RowDescription`)
/// types its output by that declaration, so a wrong one is a wrong value.
///
/// The rule is not re-derived here. Resolution delegates to [`SimpleColumns`] —
/// the executor's one column resolver, the very object `eval_expr_*` binds values
/// through — so the descriptor cannot name a different column than the value came
/// from. (The planner's [`ColumnScope`](crate::planner::ColumnScope) and
/// `RelationColumns` are the same idea one layer up, but they are built from
/// the catalog `Schema`, which the executor no longer has here; it holds
/// materialized descriptors instead. Sharing the *resolver that must agree* is the
/// point, and that is `SimpleColumns`.)
#[derive(Clone, Copy)]
pub struct ProjectionScope<'a> {
    columns: &'a [ColumnDesc],
    names: &'a [&'a str],
    table: Option<&'a str>,
    /// The catalog, when the node has one — it is what lets a **scalar subquery**
    /// item be typed by the planner's own schema-aware walk instead of the
    /// executor's blind one. See this scope's [`LeafScope::subquery_type`].
    schema: Option<&'a Schema>,
    /// The resolver this scope's column references bind through, when the node's
    /// evaluator binds through one the flat `(names, table)` form cannot reproduce — a
    /// relation-keyed namespace (`project_returning`) or a level-aware one (the
    /// aggregate and window nodes). `None` gets the `(names, table)` [`SimpleColumns`].
    resolver: Option<&'a dyn ColumnResolver>,
    /// The projecting node's record of the references an enclosing level binds
    /// (`planner::EnclosingColumns`), when the node is a nested level's.
    enclosing: &'a [EnclosingColumn],
}

// `dyn ColumnResolver` is not `Debug`; the rest of the scope is, and the resolver's
// identity is not what a `Debug` reader wants anyway.
impl std::fmt::Debug for ProjectionScope<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionScope")
            .field("columns", &self.columns)
            .field("names", &self.names)
            .field("table", &self.table)
            .field("schema", &self.schema.map(|_| "<schema>"))
            .field("resolver", &self.resolver.map(|_| "<resolver>"))
            .finish()
    }
}

impl<'a> ProjectionScope<'a> {
    /// A scope over `columns`, whose `names` and `table` **must** be the ones the
    /// same node passes to its evaluator — that agreement is the whole contract.
    #[must_use]
    pub const fn new(
        columns: &'a [ColumnDesc],
        names: &'a [&'a str],
        table: Option<&'a str>,
    ) -> Self {
        Self {
            columns,
            names,
            table,
            schema: None,
            resolver: None,
            enclosing: &[],
        }
    }

    /// Read the references this node's input does not bind off `enclosing`, the node's
    /// record of the enclosing-level columns they bind to.
    #[must_use]
    pub const fn with_enclosing(mut self, enclosing: &'a [EnclosingColumn]) -> Self {
        self.enclosing = enclosing;
        self
    }

    /// Attach the catalog, so a scalar-subquery projection item can be typed
    /// against it (see [`Self::subquery_type`]). Every node that holds a `Schema`
    /// threads it; the schema-less probes don't and keep the blind fallback.
    #[must_use]
    pub const fn with_schema(mut self, schema: &'a Schema) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Bind this scope's column references through `resolver` instead of the
    /// `(names, table)` [`SimpleColumns`].
    ///
    /// The contract is unchanged and is the reason this exists: the descriptor scope
    /// must resolve by the **same** rule the node's evaluator does, and a node whose
    /// namespace is relation-keyed (`RETURNING`, see [`project_returning`]) cannot
    /// express that rule as a flat name list — the flat form is what let a quoted
    /// dotted user column and a synthesized `relation.column` name collide.
    #[must_use]
    pub const fn with_resolver(mut self, resolver: &'a dyn ColumnResolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// The empty scope — a projection with no input row (`VALUES`, a no-FROM
    /// `SELECT`, a scalar subquery's type probe).
    #[must_use]
    pub const fn empty() -> Self {
        Self::new(&[], &[], None)
    }

    /// The index of `expr`'s column in the input row, by this scope's resolver.
    fn resolve_index(&self, table: Option<&str>, name: &str) -> Option<usize> {
        self.resolver.map_or_else(
            || {
                SimpleColumns::with_table(self.names, self.table)
                    .resolve(table, name)
                    .ok()
            },
            |r| r.resolve(table, name).ok(),
        )
    }

    /// The input descriptor `[table.]name` refers to, or `None` when this scope can't
    /// resolve it (an unknown qualifier, an ambiguous bare name, a correlated outer
    /// reference). `None` is always safe: the caller falls back to inference, and the
    /// evaluator raises the real error if there is one.
    ///
    /// The whole descriptor, not just its type: a passthrough projection carries the
    /// source column's `atttypmod` and provenance too (see [`expr_output_column`]), and
    /// that is what makes this — rather than [`LeafScope::column_type`], which is this
    /// same lookup narrowed to the type — the executor's own reader.
    ///
    /// A reference the input does not bind is an enclosing level's, read off the node's
    /// record of the column it binds to ([`Self::with_enclosing`]), as the planner's
    /// derivation of the same node reads it.
    fn column_desc(&self, table: Option<&str>, name: &str) -> Option<&'a ColumnDesc> {
        self.resolve_index(table, name).map_or_else(
            || find_enclosing(self.enclosing, table, name),
            |index| self.columns.get(index),
        )
    }

    /// The input descriptor an `Expr::Column` refers to, or `None` for any other
    /// expression (see [`Self::column_desc`]).
    fn resolve(&self, expr: &Expr) -> Option<&'a ColumnDesc> {
        let Expr::Column { table, name } = expr else {
            return None;
        };
        self.column_desc(table.as_deref(), name)
    }

    /// The enum member order `expr` is ordered by in this scope, through the **one**
    /// enum walk (`executor::expr`'s `expr_enum_with`) over this scope's own descriptor
    /// lookup — so an aggregate's `MIN`/`MAX` and the value it reads resolve the same
    /// column.
    ///
    /// `None` without an attached catalog: the order is `Schema.enums`'s slice, and a
    /// scope with no schema has nothing to borrow it from.
    #[must_use]
    pub fn expr_enum_order(&self, expr: &Expr) -> Option<&'a [String]> {
        let schema = self.schema?;
        super::expr::expr_enum_order_with(
            expr,
            &super::expr::EnumLeaves {
                column: &|table, name| {
                    enum_order_of(
                        &schema.enums,
                        self.column_desc(table, name)?.enum_type.as_deref(),
                    )
                },
                subquery: &|plan| super::expr::subquery_enum_order(plan, schema),
            },
        )
    }

    /// The **name** of the enum `expr`'s type carries in this scope — the same
    /// `executor::expr::expr_enum_with` walk as [`Self::expr_enum_order`], instantiated
    /// at the enum's name instead of its member list.
    ///
    /// Its **column** leaf needs no catalog — the name is already on the input
    /// descriptors, so this answers for a schema-less scope where the order cannot; its
    /// *subquery* leaf does, and answers `None` without one. That the two are one walk
    /// is the point — a projected `min(m)`'s descriptor and the order an outer
    /// `ORDER BY` reads off it are the same fact, and were previously two lists (this
    /// one lacked the aggregate arm, so `SELECT g FROM (SELECT id AS g, min(m) AS mm …)
    /// q ORDER BY q.mm` ordered by label bytes).
    #[must_use]
    fn expr_enum_name(&self, expr: &Expr) -> Option<String> {
        super::expr::expr_enum_with(
            expr,
            &super::expr::EnumLeaves {
                column: &|table, name| self.column_desc(table, name)?.enum_type.clone(),
                subquery: &|plan| super::expr::subquery_enum_name(plan, self.schema?),
            },
        )
    }
}

/// The executor scope's answers to the declared-type walk's two outward-facing leaves.
///
/// The other implementation is the planner's `PlannerScope`, and the *difference*
/// between the two is why the two walks are separate code: this one resolves a column
/// against a projecting node's already-materialized descriptors and has no catalog at
/// all, where the planner's resolves against the catalog `Schema`. The trait puts that
/// difference behind one object at the leaf both walks actually read through it —
/// `subquery_type` — and that is what lets a rule be written once there (see
/// [`shared_leaf_cell_type`], the cells both walks answer identically — three
/// `Expr` variants plus the `StringFn` members whose result follows an argument). At the
/// `Column` leaf the two
/// walks still reach their scope through separate code, as the paragraph below states.
///
/// Both methods keep the executor walk's existing readers — `column_type` is
/// [`ProjectionScope::column_desc`] narrowed to the type, and `subquery_type` is the
/// path `expr_type_oid`'s `Subquery` arm has always taken — so putting them behind the
/// trait moves no answer.
///
/// Only `subquery_type` is *reached* through the trait by that walk, though:
/// `expr_type_oid`'s `Subquery` arm goes through `shared_leaf_cell_type`, while its
/// `Column` arm still calls [`ProjectionScope::resolve`]. **This impl's** `column_type`
/// has one caller in the tree, the `walker_pins` scope test that asserts the two agree
/// — the bare name is shared with `ColumnResolver::column_type`, a different trait with
/// three live callers in `executor/expr/eval.rs`.
impl LeafScope for ProjectionScope<'_> {
    fn column_type(&self, table: Option<&str>, name: &str) -> Option<TypeOid> {
        self.column_desc(table, name).map(|c| c.type_oid)
    }

    /// The output type of a scalar subquery item, via the **planner's**
    /// `Plan::output_columns` when this scope carries a catalog.
    ///
    /// The executor's own [`infer_plan_output_type`] is schema-blind: it recurses to
    /// the innermost projection and types its first expression against *no* input
    /// columns, so `(SELECT id FROM t)` — a bare column it cannot resolve — fell back
    /// to `TypeOid::Text`. The planner already answers this correctly (it is what the
    /// extended protocol's `Describe` reports, which is why the two protocols
    /// disagreed on exactly these queries), so this defers to it rather than teaching
    /// a second walk the same thing.
    ///
    /// The `None` this answers when *neither* route resolves is the walk's own unknown,
    /// and `expr_type_oid` is where it becomes `TypeOid::Text`. That substitution belongs
    /// to the total walk and not to this scope: the shared rule reads both scopes through
    /// the same trait, so a `Text` here would reach the planner's walk too.
    fn subquery_type(&self, plan: &Plan) -> Option<TypeOid> {
        self.schema
            .map(|schema| plan.output_columns(schema))
            .and_then(|cols| cols.first().map(|c| c.type_oid))
            .or_else(|| infer_plan_output_type(plan))
    }
}

/// An expression's type when it is **certainly** knowable from `input_columns`,
/// `None` when it is a guess.
///
/// The counterpart to [`expr_type_oid`], which always answers — falling back to
/// `TypeOid::Text` for anything it can't resolve. That fallback is fine for
/// *describing* a column (a wrong guess only mislabels), but fatal for *gating*:
/// rejecting on a guessed `Text` would reject legitimate queries. So this returns
/// `None` instead of guessing, and a gate that gets `None` falls back to whatever
/// evidence it has (e.g. the observed values).
///
/// Deliberately shallow — a column, a literal, a cast, a bound parameter, an
/// aggregate over any of those, and `Alias` looking through. Anything else (an
/// operator result, a subquery) answers `None`.
#[must_use]
pub fn certain_expr_type_oid(expr: &Expr, scope: ProjectionScope<'_>) -> Option<TypeOid> {
    match expr {
        Expr::Column { .. } => scope.resolve(expr).map(|c| c.type_oid),
        Expr::Literal(lit) => lit.infer_type(),
        Expr::Cast { target_type, .. } => Some(*target_type),
        Expr::CastTypmod { type_oid, .. } => Some(*type_oid),
        Expr::Bound(v) => v.type_oid(),
        Expr::Alias { expr, .. } => certain_expr_type_oid(expr, scope),
        // An aggregate's result type follows its argument's, so it is certain
        // exactly when the argument is (`max(tz_col)` is `timestamptz`, which is
        // what a `||` or `::text` around it needs to keep the `+00` offset).
        Expr::Aggregate {
            func,
            arg,
            certainty,
            ..
        } => aggregate_type_of(*func, *certainty, certain_expr_type_oid(arg, scope)),
        _ => None,
    }
}

/// Executor-local type inference for an expression, resolving its leaves against the
/// scope — a `Column` against the materialized `input_columns` through the inherent
/// [`ProjectionScope::resolve`], a `Subquery` through the scope's [`LeafScope`] impl
/// (the planner, or [`infer_plan_output_type`]). Only the second of those two leaves
/// goes through the trait on this walk.
///
/// The type-only half of `expr_column_info`. Mirrors the planner's `Expr::infer_type`
/// for Postgres-correctness — arithmetic follows the operand types (not a blanket
/// `Float8`) and integer literals widen by value — but resolves columns/subqueries
/// against execution-time inputs rather than the catalog `Schema`, and is **total**
/// where that walk answers `Option`. The cells where that difference cannot show
/// are one shared arm below; the rest stay this walk's own, and
/// [`shared_leaf_cell_type`](crate::planner::shared_leaf_cell_type)'s doc says which of
/// those are later mechanical work and which are a decision against the oracle.
// `match_same_arms`: the `BinaryOp` arms are grouped by category for documentation
// (comparison/logical and array-containment both yield Bool) but kept separate, mirroring
// the planner's `Expr::infer_type`.
// `option_if_let_else`: the `find` predicate's else branch allocates via `format!` and the `Some`
// arm needs a `ref` binding, so the if-let reads clearer than a `map_or`.
#[allow(clippy::match_same_arms, clippy::option_if_let_else)]
fn expr_type_oid(expr: &Expr, scope: ProjectionScope<'_>) -> TypeOid {
    match expr {
        Expr::Column { .. } => scope
            .resolve(expr)
            .map_or(TypeOid::Text, |col| col.type_oid),
        // Integer literals widen by value (Int4 if it fits i32, else Int8); a NULL
        // literal has no inferable type, so fall back to TEXT (matching Postgres'
        // `unknown` → text default and the prior behavior).
        Expr::Literal(lit) => lit.infer_type().unwrap_or(TypeOid::Text),
        // Depends only on the builtin, so both walkers read the one mapping
        // (`planner::builtin_fn_result_type`) rather than a copy of it each.
        Expr::Builtin(func) => crate::planner::builtin_fn_result_type(*func),
        // `mod(…)` / `power(…)` / `pow(…)` are typed as the operator each is.
        Expr::BinaryOp { left, op, right } => match op.operator_form() {
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::AndList
            | BinaryOp::OrList
            | BinaryOp::AndListInner
            | BinaryOp::OrListInner
            | BinaryOp::Like
            | BinaryOp::NotLike
            | BinaryOp::Ilike
            | BinaryOp::NotIlike
            | BinaryOp::StartsWith
            | BinaryOp::RegexMatch
            | BinaryOp::NotRegexMatch
            | BinaryOp::IsDistinctFrom
            | BinaryOp::IsNotDistinctFrom => TypeOid::Bool,
            // Arithmetic result follows the operand types, via the one shared chain
            // (`arith_result_type`: temporal, then the float/NUMERIC promotion, then
            // the integer width). Only the *unknown*-operand fallback is local: this
            // walker is total, so an un-inferable operand arrives as `Text` rather
            // than as `None`, and a `Text` left operand means "take the right".
            op @ (BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::ModFn) => {
                let left_type = expr_type_oid(left, scope);
                let right_type = expr_type_oid(right, scope);
                arith_result_type(op, left_type, right_type).unwrap_or({
                    if left_type == TypeOid::Text {
                        right_type
                    } else {
                        left_type
                    }
                })
            }
            // `^`: numeric only when both sides resolve to numeric; every other mix
            // (incl. `int ^ int` and `real ^ real`) is float8 — Postgres registers only
            // `^(float8,float8)` and `^(numeric,numeric)`.
            BinaryOp::Pow | BinaryOp::PowerFn | BinaryOp::PowFn => {
                let left_type = expr_type_oid(left, scope);
                let right_type = expr_type_oid(right, scope);
                crate::planner::pow_result_type(numeric_family_result(left_type, right_type))
            }
            BinaryOp::JsonGet | BinaryOp::JsonPathGet => TypeOid::Jsonb,
            BinaryOp::JsonGetText
            | BinaryOp::JsonPathGetText
            | BinaryOp::ToChar
            | BinaryOp::Concat => TypeOid::Text,
            BinaryOp::JsonExists | BinaryOp::JsonExistsAny | BinaryOp::JsonExistsAll => {
                TypeOid::Bool
            }
            BinaryOp::Containment | BinaryOp::ContainedBy => TypeOid::Bool,
            BinaryOp::AtTimeZone => TypeOid::Timestamp,
        },
        // Negation preserves the type of the inner expression
        Expr::Negate(inner) | Expr::UnaryPlus(inner) => expr_type_oid(inner, scope),
        Expr::Not(_) => TypeOid::Bool,
        Expr::IsNull(_) | Expr::IsNotNull(_) => TypeOid::Bool,
        // The twin of the planner walk's `BooleanTest` arm: `boolean` for every
        // operand, NULL included.
        Expr::BooleanTest { .. } => TypeOid::Bool,
        // The cells the planner's twin walk answers identically, from the one
        // rule both read: a bare `DEFAULT` and an unbound `$N` (no type), a scalar
        // subquery (this scope's business — see its `LeafScope` impl), and `date_trunc` /
        // `substring` / `substr` (an argument's type).
        //
        // **This walk's totality is restored here**, once for all of them, rather than
        // inside the shared rule: `TypeOid::Text` for "no type" is *this* walk's
        // convention, and the planner's must go on answering `None`. Pushing the
        // substitution one level down would move the planner walk's `DEFAULT` and `$N`
        // answers from `None` to `Some(text)` — answers read by `$N` inference and the
        // DDL static gate as well as by `Describe`. The rules themselves are merged; the
        // two walks' unknown-ness is not, and this line is the boundary between those
        // two facts.
        //
        // `operand_type` is this walk itself, wrapped in `Some` because it always
        // answers — which is exactly the difference `date_trunc_result_type` and
        // `substring_result_type` absorb.
        Expr::Default
        | Expr::Parameter { .. }
        | Expr::Subquery { .. }
        | Expr::StringFn {
            func:
                crate::planner::StringFunc::DateTrunc
                | crate::planner::StringFunc::Substring
                | crate::planner::StringFunc::Substr,
            ..
        } => shared_leaf_cell_type(expr, &scope, &|e| Some(expr_type_oid(e, scope)))
            .unwrap_or(TypeOid::Text),
        // The cells that pick one operand's type out of several, from the one rule
        // both walks read (see `shared_multi_operand_type`). The hook is
        // [`multi_operand_type`], this walk's own recursion with the typeless leaves
        // declining; the totality boundary is restored here, at the same level as the
        // leaf cells' above.
        Expr::Array(_) | Expr::VariadicFn { .. } | Expr::Coalesce(_) | Expr::Case { .. } => {
            shared_multi_operand_type(expr, &|e| multi_operand_type(e, scope))
                .unwrap_or(TypeOid::Text)
        }
        Expr::Cast { target_type, .. } => *target_type,
        // A typmod'd cast's type is its target (the modifier changes the value only).
        Expr::CastTypmod { type_oid, .. } => *type_oid,
        Expr::Alias { expr, .. } => expr_type_oid(expr, scope),
        Expr::Aggregate {
            func,
            arg,
            certainty,
            ..
        } => {
            let arg_type = expr_type_oid(arg, scope);
            aggregate_type_of(*func, *certainty, Some(arg_type)).unwrap_or(arg_type)
        }
        // IN expressions return boolean
        Expr::InSubquery { .. } | Expr::InList { .. } => TypeOid::Bool,
        // `[NOT] EXISTS(...)` yields a boolean.
        Expr::Exists { .. } => TypeOid::Bool,
        // Window functions return BIGINT (for ROW_NUMBER, RANK, DENSE_RANK)
        Expr::Window { .. } => TypeOid::Int8,
        // A bound parameter carries its value's type.
        Expr::Bound(v) => v.type_oid().unwrap_or(TypeOid::Text),
        // `jsonb_build_object(...)` yields jsonb.
        Expr::JsonbBuildObject(_) => TypeOid::Jsonb,
        // ANY/ALL comparisons yield boolean.
        Expr::AnyAll { .. } => TypeOid::Bool,
        // `unnest(arr)` projects the array's element type. The `unwrap_or` fallback —
        // where the planner walk answers `None` instead, `EDGES.md` row 55's cell-F
        // split — is reached only where the plan-time `unnest_legality` gate declined to
        // type the argument, so today that is an argument `arg_known_type`'s `_` arm
        // swallows (`unnest(i + 0)`, and equally `unnest(CASE …)` or the `Expr::Alias`
        // `unnest(set_config(…))`) and no longer a column or a literal, which are `42883`
        // before either walk runs.
        Expr::Unnest(inner) => {
            let type_oid = expr_type_oid(inner, scope);
            type_oid.array_element().unwrap_or(type_oid)
        }
        // `LOWER(text)` is text.
        Expr::Lower(_) => TypeOid::Text,
        Expr::MathFn { func, args } => {
            // A **bare `NULL` literal** argument is Postgres's `unknown`, which overload
            // resolution answers with the numeric category's preferred type, `float8` —
            // so it has to reach `math_fn_result_type` as `None` and not as this
            // walker's total `Text` fallback. Only `abs` can tell the difference (it is
            // the one member whose rule passes the argument type through), and it did:
            // `SELECT abs(NULL)` declared `text` here against `float8` on 18.6.
            //
            // Narrow on purpose, and narrower than "untyped constant" would suggest:
            // routing through `Literal::infer_type` — the same reader the planner walk
            // uses — makes `Literal::Null` the *only* literal that answers `None` here,
            // since a bare string literal types as `text` rather than staying Postgres's
            // `unknown` (`abs('5')` is `0A000` here against 18.6's `701` answering `5`;
            // the general literal-typing gap, recorded in `docs/compatibility.md`). Every non-literal
            // shape keeps `expr_type_oid`, because the shallow `certain_expr_type_oid`
            // would answer `None` for an ordinary computed argument (`abs(v + 1)` over an
            // int column is `int4` on 18.6, and would become `float8` here) — which is
            // also why an argument that is *transitively* untyped still splits the two
            // walkers: `abs(-NULL)`, `abs(CASE WHEN true THEN NULL END)` and
            // `abs(COALESCE(NULL, NULL))` each declare `25` here against `Describe`'s
            // `701`, where 18.6 refuses all three (`42725` / `42883` / `42883`). What
            // that leaves open is the general un-inferable-operand policy — see
            // `math_fn_result_type`.
            let arg_type = args.first().and_then(|a| match a {
                Expr::Literal(lit) => lit.infer_type(),
                other => Some(expr_type_oid(other, scope)),
            });
            crate::planner::math_fn_result_type(*func, arg_type, args.len() >= 2)
        }
        // Strict-NULL string function: result type depends only on the function. The
        // members whose result follows an argument — `date_trunc`, `substring`, `substr`
        // — are answered by the shared arm above.
        Expr::StringFn { func, .. } => crate::planner::string_fn_result_type(*func),
        // `EXTRACT`/`extract` yield numeric; `date_part` yields float8.
        Expr::Extract { retnumeric, .. } => {
            if *retnumeric {
                TypeOid::Numeric
            } else {
                TypeOid::Float8
            }
        }
        // Every sequence function returns bigint.
        Expr::SequenceFn { .. } => TypeOid::Int8,
    }
}

/// This walk's operand reader for [`shared_multi_operand_type`]: its own recursion,
/// except that a **typeless leaf** declines instead of answering `TypeOid::Text`.
///
/// The rule those cells apply is "the first operand that types", and this walk's
/// totality made every operand type — so it always took the *first*, where the planner
/// walk and 18.6 take the first **inferable** one. Handing the shared rule a real `None`
/// for the leaves that genuinely have no type is what makes one rule answer as both
/// walks should: measured on 18.6, `COALESCE(NULL, length('ab'))`,
/// `CASE WHEN true THEN NULL ELSE length('ab') END`, `greatest`/`least`/`nullif` of the
/// same pair and `ARRAY[NULL, length('ab')]` are `int4`/`int4[]` there, and were `text`/
/// `text[]` on the simple-query `RowDescription` here against a correct `Describe`.
/// Over a `timestamptz` operand it was a **value** and not only an OID: the text
/// encoder is column-type-aware, so `COALESCE(NULL, date_trunc('day', ts))` sent
/// `2024-01-15 00:00:00` where 18.6 sends `2024-01-15 00:00:00+00`.
///
/// **"The first inferable operand" is 18.6's answer only where one branch type is
/// known *to either walk***, which is every member this closed and is not the rule 18.6
/// applies: it resolves the *common type* of all the known branches. With **two**
/// computed branches
/// of different types neither walk folds, and both are wrong together — measured,
/// `COALESCE(NULL, length('ab'), n + 1)` is `int4` here against 18.6's `numeric`, and
/// `COALESCE(i + 0, b + 0)` over a row whose `i` is NULL declares `int4` while returning
/// an `int8` value. That band predates this hook and is untouched by it; the
/// `VariadicFn` cell is exempt because `variadic_fn_result_type` folds. `EDGES.md` row
/// 54 records it.
///
/// The qualifier binds in the **other** direction too, because 18.6's notion of *known*
/// is not this one's: a bare string literal is `unknown` there and `text` to both walks
/// here, so `COALESCE('7', length('ab'))` is `text` on both legs against 18.6's `int4`
/// and `COALESCE('abc', length('ab'))` answers `abc` where 18.6 is `22P02`. This hook
/// routes a literal through `Literal::infer_type` precisely so a string keeps typing as
/// `text`, which leaves that band exactly where it was (row 54 records it beside the
/// fold). And the same `Ok(None)` that lets a branch set reach these walks raw also
/// skips the `42804` **gate** over it, which is `EDGES.md` row 65 — a wrong answer
/// rather than a wrong label, and closed by the same widening.
///
/// **Narrow on purpose**, the same shape and the same reason as the `abs` carve-out in
/// `expr_type_oid`'s `MathFn` arm: the leaves are the four whose *own* answer is "no
/// type" — a bare `NULL` literal, a bound NULL, an unbound `$N`, a bare `DEFAULT` — and
/// nothing else, because a genuinely `text` operand must keep answering `text`. Routing
/// a literal through `Literal::infer_type` (the reader the planner walk uses) makes
/// `Literal::Null` the only literal that declines; a bare string literal still types as
/// `text`. An operand that is only *transitively* untyped keeps the total answer, so
/// `COALESCE(-NULL, length('ab'))` still splits the two walks — the residual is the
/// `abs(-NULL)` family's, recorded in `EDGES.md`, and 18.6 refuses those spellings
/// outright rather than answering the other branch.
fn multi_operand_type(expr: &Expr, scope: ProjectionScope<'_>) -> Option<TypeOid> {
    match expr {
        Expr::Literal(lit) => lit.infer_type(),
        Expr::Bound(v) => v.type_oid(),
        Expr::Parameter { .. } | Expr::Default => None,
        other => Some(expr_type_oid(other, scope)),
    }
}

/// Infer the output type of a scalar subquery plan.
///
/// Returns the `TypeOid` of the first output column, or None if it can't be determined.
fn infer_plan_output_type(plan: &Plan) -> Option<TypeOid> {
    match plan {
        // For Aggregate, look at the expression type
        Plan::Aggregate { exprs, .. } => {
            if let Some(first_expr) = exprs.first() {
                let (_, type_oid) = expr_column_info(first_expr, ProjectionScope::empty());
                return Some(type_oid);
            }
            None
        }
        // For Project, look at the first projected expression
        Plan::Project { exprs, .. } => {
            if let Some(first_expr) = exprs.first() {
                // Recursively get the type
                let (_, type_oid) = expr_column_info(first_expr, ProjectionScope::empty());
                return Some(type_oid);
            }
            None
        }
        // For Filter, Sort, Limit - recurse into input
        Plan::Filter { input, .. } | Plan::Sort { input, .. } | Plan::Limit { input, .. } => {
            infer_plan_output_type(input)
        }
        // For other plan types, we can't easily determine the type
        _ => None,
    }
}

/// Generate a unique OID for a new catalog object, advancing the counter.
///
/// `execute_create_table` is the one production caller (a `#[cfg(test)]` monotonicity
/// guard in `executor` is the other); every other mint site reads [`next_object_oid`]
/// instead and writes the counter itself, once its own last fallible step has returned.
///
/// OIDs are unique across both tables and indexes. Allocation is **monotonic**
/// via `Schema::next_oid` — a **dropped** object's OID is never handed out again
/// within a session, even after a `DROP` lowers the live max or a `ROLLBACK`
/// restores an older catalog. Reuse of one of those would let two logical indexes
/// share one storage `index_id`, so dropping one orphans the other (→ "index not
/// found"). `next_oid` is seeded lazily from the current catalog max (so a freshly
/// opened / deserialized schema picks up where the persisted OIDs left off), then
/// only ever advances.
///
/// A **failed** statement's number is a different question, and a different answer:
/// see [`next_object_oid`] § A dropped number and a failed one.
pub fn generate_object_oid(schema: &mut Schema) -> i32 {
    let oid = next_object_oid(schema);
    schema.next_oid = oid + 1;
    oid
}

/// The OID [`generate_object_oid`] would mint, **without** advancing the counter — for a
/// statement whose last fallible step runs after it needs the number.
///
/// Three callers, each for the same shape of reason:
///
/// - `execute_create_type` has to put the new `EnumType` into the map it stages before
///   the rebuild can read that map's member lists, and the rebuild can raise. Minting
///   there wrote `next_oid` ahead of a statement that then failed — measured, `16387`
///   before a refused `CREATE TYPE` and `16389` after.
/// - `execute_create_index` and `create_unique_index` need the number to build the
///   `IndexSpec` the storage build takes, and that build is the statement's last
///   fallible step (`docs/ddl.md` § A failed index build).
///
/// Each caller writes `schema.next_oid = oid + 1` once the fallible work is behind it.
/// That write is not what keeps two objects in one statement apart — the seed below
/// re-reads the live catalog max, so an object already registered is already counted —
/// it is what keeps a **dropped** object's number from being handed out twice, which
/// [`generate_object_oid`] explains the cost of.
///
/// # A dropped number and a failed one
///
/// The two are opposite answers.
///
/// - A **dropped** object's number is never handed out again *within a session* (the
///   counter is runtime state, reseeded on reopen from the catalog max and from the
///   greatest index number the tree holds an entry under — `Schema`), and an index's
///   number is not handed out again after a reopen while its entries sit in the tree.
///   The counter only advances, so a `DROP` that lowers the live catalog max cannot make
///   the seed step backward: measured, a `CREATE INDEX` at `16385`, a `DROP INDEX`, and
///   the next `CREATE INDEX` takes `16386`.
/// - A **failed** statement's number *is* handed back — that is the point of reading
///   it without advancing. Measured, a `CREATE INDEX` refused at its storage build
///   leaves `next_oid` at `16385` and the next `CREATE INDEX` on that table takes
///   `16385`.
///
/// What makes the second safe is that the two leave different things behind. A
/// dropped index's storage band held entries; a failed build's does not.
/// `Storage::create_index` is all-or-nothing, so a refused build registers no
/// definition and no entries — and a back-fill whose page writes fail part-way writes
/// pages the published root never reaches, so the band a re-used id lands on is empty.
/// Measured in `pglet`'s `a_failed_back_fill_write_leaves_no_readable_entry_in_the_band`,
/// which probes the re-used band directly.
#[must_use]
pub fn next_object_oid(schema: &Schema) -> i32 {
    // Start from 16384 (first user OID in Postgres).
    let base_oid = 16384;
    let max_table_oid = schema
        .tables
        .values()
        .map(|t| t.oid)
        .max()
        .unwrap_or(base_oid - 1);
    let max_index_oid = schema
        .indexes
        .values()
        .map(|idx| idx.oid)
        .max()
        .unwrap_or(base_oid - 1);
    // A user `ENUM` type's OID comes from this same counter (`pg_type.oid` /
    // `pg_enum.enumtypid`), so the seed has to read that map too: leaving it out let a
    // table created after a reopen take an OID an enum already holds, and `pg_class`
    // and `pg_type` would then answer for the same OID with different objects.
    let max_enum_oid = schema
        .enums
        .values()
        .map(|e| e.oid)
        .max()
        .unwrap_or(base_oid - 1);
    // Seed from the persisted max on first use; never step backward thereafter.
    let oid = schema
        .next_oid
        .max(max_table_oid.max(max_index_oid).max(max_enum_oid) + 1);
    // The object-OID space and the synthesized-`pg_constraint` band must stay disjoint:
    // if they met, `JOIN pg_class ON oid = conrelid` would resolve a constraint's OID to
    // a relation — a silent wrong. The distance is over a billion allocations, each
    // needing its own DDL statement and its own catalog entry, so this asserts the shape
    // of the two spaces rather than guarding a bound anything approaches. It is a
    // `debug_assert` because `constraints_view`'s header claims it, and a claim no code
    // makes is the class this repo keeps paying for.
    debug_assert!(
        oid < crate::catalog::SYNTHETIC_OID_BASE,
        "object OIDs have reached the synthesized-constraint band"
    );
    oid
}

#[cfg(test)]
mod returning_namespace_tests {
    use std::collections::BTreeMap;

    use super::{ReturningRelation, returning_column_map, returning_input_columns};
    use crate::Error;
    use crate::executor::expr::ColumnResolver;
    use crate::executor::types::ColumnDesc;
    use crate::types::TypeOid;

    fn desc(name: &str, type_oid: TypeOid) -> ColumnDesc {
        ColumnDesc::new(name.to_string(), type_oid)
    }

    #[test]
    fn a_quoted_dotted_user_column_cannot_forge_a_synthesized_name() {
        // The target `wt` carries `v`, so the flat form synthesizes `wt.v`; the source
        // `s` carries a column a user quoted into existence as `"wt.v"`. Under the flat
        // form the bare reference matched the *target's* column exactly, which is the
        // silent wrong. Resolution is over `(relation, column)` pairs now, and the pair
        // `(s, "wt.v")` is the only claimant of that name.
        let target = [desc("id", TypeOid::Int4), desc("v", TypeOid::Text)];
        let source = [desc("id", TypeOid::Int4), desc("wt.v", TypeOid::Text)];
        let relations: [ReturningRelation<'_>; 2] = [("wt", &target), ("s", &source)];
        let map = returning_column_map(&relations, &BTreeMap::new());

        assert_eq!(map.resolve(None, "wt.v").unwrap(), 3, "the source's column");
        assert_eq!(map.resolve(Some("s"), "wt.v").unwrap(), 3);
        assert_eq!(map.resolve(Some("wt"), "v").unwrap(), 1);

        // The **descriptor** list still carries the synthesized names, because the
        // correlated outer frame addresses the row that way — so this asserts the two
        // are genuinely different objects rather than the same one renamed.
        let columns = returning_input_columns(&relations);
        assert_eq!(columns[1].name, "wt.v");
        assert_eq!(columns[3].name, "s.wt.v");
    }

    #[test]
    fn a_bare_name_two_relations_carry_is_42702_and_one_relation_carries_resolves() {
        // Both polarities of the claimant count, which is the verdict
        // `ColumnScope::check_binding` reaches at plan time over the same relations.
        let target = [desc("id", TypeOid::Int4), desc("v", TypeOid::Text)];
        let source = [desc("v", TypeOid::Text), desc("k", TypeOid::Int4)];
        let relations: [ReturningRelation<'_>; 2] = [("t", &target), ("s", &source)];
        let map = returning_column_map(&relations, &BTreeMap::new());

        let err = map.resolve(None, "v").unwrap_err();
        assert!(matches!(err, Error::AmbiguousColumn { .. }));
        assert_eq!(err.sqlstate(), "42702");
        assert_eq!(map.resolve(None, "k").unwrap(), 3);
        assert_eq!(map.resolve(None, "id").unwrap(), 0);
    }

    #[test]
    fn a_bare_name_is_never_a_suffix_of_a_column_name() {
        // The near-miss repair — dropping the exact tier and matching `.{name}` — is
        // wrong here and this is where it shows: only the source carries a column
        // *named* `c`, while the target's `"b.c"` merely ends with it. 18.6 answers the
        // source's column; a suffix rule counts two claimants and says `42702`.
        let target = [desc("id", TypeOid::Int4), desc("b.c", TypeOid::Text)];
        let source = [desc("c", TypeOid::Text)];
        let relations: [ReturningRelation<'_>; 2] = [("t", &target), ("s", &source)];
        let map = returning_column_map(&relations, &BTreeMap::new());

        assert_eq!(map.resolve(None, "c").unwrap(), 2, "the source's own `c`");
        assert_eq!(map.resolve(None, "b.c").unwrap(), 1, "the target's column");
    }

    #[test]
    fn a_dotted_alias_and_a_dotted_column_stay_distinct() {
        // Both qualified spellings flatten to the same string `t.x.y`, so a fix that
        // keeps the flat namespace and only qualifies its references still collides.
        let target = [desc("x.y", TypeOid::Text)];
        let source = [desc("y", TypeOid::Text)];
        let relations: [ReturningRelation<'_>; 2] = [("t", &target), ("t.x", &source)];
        let map = returning_column_map(&relations, &BTreeMap::new());

        assert_eq!(map.resolve(Some("t"), "x.y").unwrap(), 0);
        assert_eq!(map.resolve(Some("t.x"), "y").unwrap(), 1);
    }

    #[test]
    fn a_single_relation_keeps_bare_descriptor_names_and_alias_aware_resolution() {
        // The no-`FROM` namespace: an unknown qualifier is `42P01` (not a loose match),
        // the target's own effective name resolves, and the descriptors keep the bare
        // names a client expects from `RETURNING *`.
        //
        // The `k`-is-not-a-suffix-of-`"s.k"` line below is a **second wall**, not a live
        // divergence closed here: `SimpleColumns` did resolve that bare name by suffix,
        // but no SQL reaches it — `lower_returning`'s closed scope counts zero claimants
        // and raises `42703` first, measured on a build with this resolver reverted. This
        // assertion is what covers the resolver's own half; the end-to-end statement is
        // pinned by `update/returning_bare_name_is_not_a_dotted_suffix.sql`, whose header
        // says which walls it can and cannot separate.
        let target = [desc("id", TypeOid::Int4), desc("s.k", TypeOid::Text)];
        let relations: [ReturningRelation<'_>; 1] = [("o", &target)];
        let map = returning_column_map(&relations, &BTreeMap::new());

        assert_eq!(map.resolve(Some("o"), "s.k").unwrap(), 1);
        assert_eq!(map.resolve(None, "s.k").unwrap(), 1);
        assert_eq!(
            map.resolve(Some("bogus"), "id").unwrap_err().sqlstate(),
            "42P01"
        );
        // `k` is not a column; the suffix of `"s.k"` must not answer for it.
        assert_eq!(map.resolve(None, "k").unwrap_err().sqlstate(), "42703");

        assert_eq!(
            returning_input_columns(&relations)
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
            vec!["id".to_string(), "s.k".to_string()]
        );
    }
}

/// [`unified_enum_type`], the rule a set operation's and a `USING` merge's output column
/// share.
///
/// **The arms a fixture reaches and the arms only this can.** The `(Some, None)` pair is
/// the one real SQL produces — an `unknown`-typed literal or a `NULL` beside an enum
/// branch — and `tests/fixtures/enum/set_op_*` holds it against 18.6 in both operand
/// orders. The `(Some(a), Some(b))` pair is now held by fixtures too — pglet raises 18.6's
/// own `42846 UNION could not convert type weather to mood` at a gate **above** this rule
/// (`enum/two_enums_setop_loud.sql` and its four siblings), and not the
/// `types … cannot be matched` spelling, which is the **text** pair's. What only this can
/// hold is the arm's own verdict *below* that gate, which no statement reaches any more,
/// and it is also the one a "return the left side" simplification passes.
///
/// What these do **not** cover is the fold *over* the arm: a third branch beside a
/// cancelled pair would read this function's `None` as "no enum" and revive it, which no
/// statement reaches now that the pair is refused above the fold (`EDGES.md` row 109,
/// closed).
#[cfg(test)]
mod unified_enum_type_tests {
    use super::unified_enum_type;
    use crate::executor::ColumnDesc;
    use crate::types::TypeOid;

    fn desc(name: &str, enum_type: Option<&str>) -> ColumnDesc {
        ColumnDesc {
            enum_type: enum_type.map(str::to_string),
            ..ColumnDesc::new(name.to_string(), TypeOid::Text)
        }
    }

    /// A side carrying no enum is **no information**, not a veto — 18.6's
    /// `select_common_type`, where an `unknown` literal takes the other branch's type.
    /// Asserted in **both** orders: measured, a left-only read fails the first and a
    /// right-only read fails the second, so the pair is what separates them — and each
    /// of those two shortcuts also reddens the matching `enum/set_op_*` fixture.
    #[test]
    fn an_untyped_side_takes_the_other_sides_enum() {
        assert_eq!(
            unified_enum_type(&desc("m", Some("mood")), &desc("m", None)).as_deref(),
            Some("mood")
        );
        assert_eq!(
            unified_enum_type(&desc("m", None), &desc("m", Some("mood"))).as_deref(),
            Some("mood")
        );
    }

    /// The agreeing and the empty pairs — the two ends the asymmetric arm sits between.
    #[test]
    fn two_of_the_same_unify_and_two_of_nothing_stay_nothing() {
        assert_eq!(
            unified_enum_type(&desc("m", Some("mood")), &desc("m", Some("mood"))).as_deref(),
            Some("mood")
        );
        assert_eq!(unified_enum_type(&desc("m", None), &desc("m", None)), None);
    }

    /// Two *different* enums decline, in both orders. This is the arm no differential
    /// fixture can reach — 18.6 refuses the pair outright — so it lives here or nowhere.
    #[test]
    fn two_different_enums_decline_rather_than_picking_a_side() {
        assert_eq!(
            unified_enum_type(&desc("m", Some("mood")), &desc("m", Some("weather"))),
            None
        );
        assert_eq!(
            unified_enum_type(&desc("m", Some("weather")), &desc("m", Some("mood"))),
            None
        );
    }
}
