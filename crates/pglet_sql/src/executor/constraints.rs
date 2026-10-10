//! Write-path constraint enforcement: enum membership, NOT NULL, CHECK, UNIQUE,
//! FOREIGN KEY, and the size of every index entry a row writes.

use std::collections::{BTreeMap, HashMap};

use crate::catalog::{
    CheckConstraint, Column, EnumType, ForeignKey, Index, IndexKey, Schema, Table,
};
use crate::planner::{BoolContext, ConflictTarget, Expr};
use crate::types::{TypeOid, Value, value_to_text, value_to_text_with_oid};
use crate::{Error, Result};
use pglet_core::{FailingRow, FkDirection, ForeignKeyViolationDetail, KeyDescription};

use super::coerce::{apply_typmod, coerce_value};
use super::expr::{boolean_context_operand, eval_expr, predicate_holds};
use super::helpers::table_enum_orders;
use super::index_row::{
    check_btree_item_size, check_entry_size_field, check_gin_entries, check_page_key_len,
    encoded_key_len, index_entry_size, key_part_enum, key_part_is_enum,
};
use super::types::{IndexMethod, index_method_for};
use crate::gin::gin_decompose;

/// Coerce a value to its column's storage type and enforce whichever type modifier
/// it declared — a `NUMERIC(p,s)` rounded to `scale` and precision-checked, a
/// `varchar(n)`/`char(n)` length, a temporal `(p)` sub-second precision — the single
/// store-time value finalizer every write path storing a non-NULL value funnels through, so
/// coercion and typmod can't drift between INSERT / UPDATE / upsert / COPY / a `DEFAULT` /
/// a foreign key's `ON UPDATE` action (`dml`'s `action_assigned_values`). A `DELETE`'s
/// `ON DELETE SET NULL` writes its NULL without it, a value this returns unchanged. The
/// paired transform to [`validate_column`]'s constraint check: this produces the value that
/// gets stored, `validate_column` then vets it.
///
/// # Errors
///
/// Propagates a coercion error (`Error::TypeCoercion` / `InvalidInputSyntax`), an
/// explicit-only cast used in assignment context (`Error::Type`, SQLSTATE `42804`),
/// a `NUMERIC` overflow (`Error::NumericOutOfRange`, SQLSTATE `22003`), a
/// `varchar(n)`/`char(n)` length over-run whose excess is non-blank
/// (`Error::StringDataRightTruncation`, SQLSTATE `22001`), or a temporal `(p)`
/// rounding that carries out of range or a `date` / `timestamp` outside the date domain
/// (`Error::TemporalOutOfRange`, SQLSTATE `22008`).
pub fn coerce_column_value(value: Value, col: &Column) -> Result<Value> {
    reject_explicit_only_assignment(&value, col)?;
    let mut value = coerce_value(value, col.type_oid)?;
    // Each modifier below goes through the one [`apply_typmod`], which is what makes an
    // **array** column enforce its declared modifier: the catalog's modifier is the
    // element's (`numeric(4,1)[]` is an array of `numeric(4,1)`, `timestamp(3)[]` an
    // array of `timestamp(3)`) and `apply_typmod` walks the elements. A value variant
    // the modifier doesn't apply to passes through, so no variant guard is needed here.
    if let Some(tm) = col.numeric_typmod {
        value = apply_typmod(
            value,
            col.type_oid,
            crate::planner::Typmod::Numeric {
                precision: tm.precision,
                scale: tm.scale,
            },
            false,
        )?;
    }
    // A `varchar(n)`/`char(n)` column enforces its length on store (`is_explicit =
    // false`): an over-length value truncates iff its excess is all blanks, else
    // fails loud (`22001`).
    if let Some(n) = col.varchar_length {
        value = apply_typmod(
            value,
            col.type_oid,
            crate::planner::Typmod::Length(n),
            false,
        )?;
    }
    // A `timestamp(p)` / `timestamptz(p)` / `time(p)` / `interval(p)` column rounds
    // its microseconds on store, at the element's width for an array column. Every
    // write path funnels here, which is what makes `INSERT`, `UPDATE`, `COPY FROM`,
    // `ON CONFLICT DO UPDATE`, a `DEFAULT` backfill and `RETURNING` agree with 18.6
    // at once — each measured divergent before this arm existed.
    if let Some(p) = col.temporal_precision {
        value = apply_typmod(
            value,
            col.type_oid,
            crate::planner::Typmod::Precision(p),
            false,
        )?;
    }
    // A finite `date` or `timestamp` outside the date domain is not stored, and a
    // `timestamp` is stored at UTC; an infinity is stored as it is. The producers already
    // refuse such a value; what reaches this line with one is a value no producer built —
    // one a store written without the domain holds (a store older than the `PGMO` magic,
    // which reopen refuses), copied by `INSERT … SELECT` or assigned back by an `UPDATE` —
    // and the refusal keeps that write from adding such a row, short of an array held under
    // a non-temporal element type, which `normalize_stored_value_domain` does not walk.
    // Only an assigned column reaches it: an `UPDATE` of another column rewrites the row
    // with this one's value as stored.
    crate::types::normalize_stored_value_domain(&mut value)?;
    Ok(value)
}

/// Refuse a conversion Postgres registers as **explicit-only** when it is reached in
/// **assignment** context — a value being stored into a column.
///
/// `pg_cast.castcontext` is the fact, and pglet's [`coerce_value`] is one table
/// serving both contexts: it is the cast implementation *and* the store-time
/// conversion, so a pair Postgres registers `'e'` (explicit) was silently performed on
/// every write. Measured on 18.6, `int4 ↔ bool` is `'e'` in **both** directions, so:
/// `SELECT 1::bool` answers `t` and `SELECT true::int` answers `1`, while
/// `INSERT INTO t (boolcol) VALUES (1)` and `INSERT INTO t (intcol) VALUES (true)`
/// are each `42804 column "…" is of type … but expression is of type …`. pglet
/// accepted all four, and the two assignment ones were the engine's **silent** shape:
/// `INSERT INTO t (b) VALUES (1)` over `t(b BOOL)` stored `true` with no error
/// anywhere, and `DEFAULT 1 + 1` on a `BOOL` column was accepted at DDL and
/// backfilled `true` (the literal spelling `DEFAULT 1` was already loud only because
/// an integer literal is an `Int8`, for which no arm exists — an accident of literal
/// typing standing in for a rule).
///
/// **The gate is here rather than in [`coerce_value`]**, and the split is 18.6's own:
/// deleting the `int4 → bool` arm from the shared table is green across the whole
/// corpus and the workspace (measured) and still wrong, because it takes
/// `SELECT 1::bool` — a statement 18.6 answers — with it. No fixture casts an integer
/// to a boolean, so the corpus cannot see that direction; this is the
/// green-because-the-corpus-lacks-the-shape trap, caught by probing the oracle rather
/// than by the gate. [`coerce_column_value`] already takes the same explicit-vs-store
/// split one layer down, for the `varchar(n)` typmod (it passes `is_explicit = false`),
/// and every one of its callers is an assignment (INSERT / UPDATE / upsert / COPY /
/// `resolve_default` / the DDL default cook) — which is what makes it the place where
/// a context-dependent refusal belongs.
///
/// **It is a measured pair list, not a model of `pg_cast` — and it does not need to
/// be one.** Only the two directions above are enumerated, because they are the two
/// this engine has been measured performing in assignment context. The general audit
/// this used to defer landed at the **plan-time** gate instead
/// (`planner::legality`'s `assignment_legality`), which models the whole
/// `can_coerce_type(… COERCION_ASSIGNMENT)` verdict off a measured accept matrix; this
/// list stays a subset of it, and delegates the verdict so the two cannot disagree.
///
/// **And it is reached only by the values that arrive *typed*.** This gate reads a
/// `Value`, so whether it sees the pair at all depends on what the planner's
/// literal-coercion pass has already done: a bare literal is coerced to the target
/// column's type before any `Value` reaches here, so `INSERT INTO tb (b) VALUES (1)`
/// over `tb(b BOOL)` used to store `true` and `INSERT INTO ti (a) VALUES (true)` /
/// `UPDATE ti SET a = true` over `ti(a INT)` used to store `1`, all three silently,
/// where 18.6 answers `42804`. That half is closed **above** this one, at the four
/// assignment lowering sites (`planner::coercion`'s `check_assignment` at three of them,
/// and `planner::legality`'s `assignment_legality` called directly for
/// `INSERT … SELECT`), not by widening this list — which cannot be widened to cover
/// it, because it reads a `Value` and a
/// legal unknown literal and an illegal typed `text` are the same `Value::Text`. The
/// `DEFAULT` routes are covered by a third gate: [`cook_column_default`](super::ddl),
/// which is 18.6's `cookDefault` position and runs **both** halves — the same
/// `check_assignment` on the expression's declared type (or, for the bare-constant
/// `ColumnDefault::Literal` arm the parser has already folded to a `Value`,
/// `assignment_legality` on that value's own type), and then the default's own value
/// against the column before the literal pass can retype it. So what this list still catches on its own are the write paths that reach a
/// column with an already-typed value and **no** plan-time assignment gate above them —
/// `COPY`'s decoded cells, `resolve_default`'s stored defaults, and the `ADD COLUMN`
/// back-fill probe. It runs on every write path regardless; on the four gated ones the
/// plan-time refusal simply arrives first.
fn reject_explicit_only_assignment(value: &Value, col: &Column) -> Result<()> {
    let source = match (value, col.type_oid) {
        (Value::Int2(_), TypeOid::Bool) => TypeOid::Int2,
        (Value::Int4(_), TypeOid::Bool) => TypeOid::Int4,
        (Value::Int8(_), TypeOid::Bool) => TypeOid::Int8,
        (Value::Bool(_), TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8) => TypeOid::Bool,
        _ => return Ok(()),
    };
    // The `int2`/`int8` rows are the wider half of the same 18.6 answer rather than a
    // guess: `pg_cast` registers only the `int4 ↔ bool` pair, so `(1::int8)::bool` is
    // `42846 cannot cast type bigint to boolean` there, while the *assignment*
    // spelling of all six is one `42804` with the source type named
    // (`INSERT INTO t (b) VALUES (1::int8)`, `… (j BIGINT) VALUES (true)`, and the
    // `DEFAULT` spelling of each — all measured). pglet's integer literals are `Int8`,
    // so the `int8` row is the one an ordinary `DEFAULT 1` on a `BOOL` column reaches
    // — which is why it now answers 18.6's `42804` rather than the `42846` its missing
    // arm used to produce.
    //
    // The verdict and its sentence come from the one assignment rulebook rather than
    // being spelled again here: two producers of one Postgres message is the drift seam
    // this engine keeps paying for, and this list is a *subset* of what that rulebook
    // refuses, so the two cannot disagree about these six pairs.
    crate::planner::assignment_legality(source, col.type_oid, &col.name)
}

/// Validate that `value` is a member of `col`'s `ENUM` type, if it has one.
///
/// A NULL value passes (nullability is enforced separately, when constraints
/// land); a non-enum column always passes. Enum values are text labels, so any
/// non-text value is rejected.
///
/// # Errors
///
/// Returns `Error::InvalidEnumValue` if `value` is not one of the enum's members,
/// or `Error::Execution` if the column names an enum absent from the catalog.
pub(super) fn validate_enum_member(
    col: &Column,
    value: &Value,
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    let Some(enum_name) = col.enum_type.as_deref() else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let enum_type = enums.get(enum_name).ok_or_else(|| {
        Error::Execution(format!("enum type \"{enum_name}\" not found in catalog"))
    })?;
    let label = match value {
        Value::Text(s) => s.clone(),
        // Non-text values are never enum members; render for the error message.
        other => format!("{other:?}"),
    };
    if enum_type.labels.contains(&label) {
        Ok(())
    } else {
        Err(Error::InvalidEnumValue {
            enum_type: enum_name.to_string(),
            value: label,
        })
    }
}

/// Render a fully-assembled row for a violation's `Failing row contains (…)` DETAIL.
///
/// Column-type-aware (`value_to_text_with_oid`), because only the declared type says
/// whether a `Value::Timestamp` renders with its `+00` offset — the same reason
/// `enforce_checks` takes `col_types`. Values go in **unclipped**; the 64-byte
/// per-field clip is `FailingRow::render`'s, so the rule has one home.
pub(super) fn render_failing_row(col_types: &[TypeOid], row: &[Value]) -> FailingRow {
    FailingRow::new(
        row.iter()
            .enumerate()
            .map(|(i, v)| {
                col_types
                    .get(i)
                    .map_or_else(|| value_to_text(v), |&oid| value_to_text_with_oid(v, oid))
            })
            .collect(),
    )
}

/// Enforce every `NOT NULL` on `table` against a **fully assembled** row.
///
/// Row-scoped rather than per-column, and that is not a refactor of convenience: 18.6
/// checks not-null in `ExecConstraints`, *after* the tuple is formed, and reports the
/// whole tuple in `Failing row contains (…)`. Checking mid-assembly could only ever
/// show a half-built row, which is a field pglet cannot fill truthfully — so the
/// check moved to where the truth is. The order this restores is Postgres's too: a
/// later column's input-function failure (`22P02`, `22001`, a non-member enum label)
/// now outranks an earlier column's null, because 18.6 evaluates the whole target
/// list before it constrains anything.
///
/// `columns` is the table's column list in row order; `col_types` its declared types.
///
/// # Errors
///
/// `Error::NotNullViolation` for the **first** `NOT NULL` column holding a null, in
/// column order — 18.6's own order (`for attnum = 1 .. natts`).
pub fn enforce_not_null_row(
    table: &str,
    columns: &[Column],
    col_types: &[TypeOid],
    row: &[Value],
) -> Result<()> {
    for (i, col) in columns.iter().enumerate() {
        if !col.nullable && row.get(i).is_none_or(Value::is_null) {
            return Err(Error::NotNullViolation {
                table: table.to_string(),
                column: col.name.clone(),
                row: Some(Box::new(render_failing_row(col_types, row))),
            });
        }
    }
    Ok(())
}

/// Per-column write-path validation: enum membership.
///
/// The single entry point every write path (INSERT, UPDATE, ON CONFLICT DO
/// UPDATE, streaming bulk INSERT) calls per assigned/assembled column, so the
/// per-column constraint set can't drift between paths. The **row**-scoped checks —
/// `NOT NULL` (`enforce_not_null_row`) and `CHECK` (`enforce_checks`) — run once per
/// fully-assembled row; `enforce_row_constraints` chains all of them for the paths
/// that already hold a whole row.
///
/// `table` is unused by the enum check itself and kept in the signature because every
/// caller has it and a future per-column rule would want it. (It is `_table` for now
/// rather than dropped, so the four call sites keep their shape.)
///
/// # Errors
///
/// Propagates `Error::InvalidEnumValue` from `validate_enum_member`.
pub fn validate_column(
    _table: &str,
    col: &Column,
    value: &Value,
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    validate_enum_member(col, value, enums)
}

/// Gather a table's stored `CHECK` predicates into evaluable `(name, Expr)` pairs.
///
/// The predicates are already parsed (the catalog stores the `Expr` IR), so this
/// just clones each `(name, expr)` — no re-parse. Call once per statement and reuse
/// for every row. The `name` is the one the executor minted at DDL time; there is no
/// second opinion about it anywhere, which is the whole point of `CheckConstraint`
/// carrying a total `String`.
pub fn prepare_checks(table: &Table) -> Vec<(String, Expr)> {
    table.checks.iter().map(prepare_check).collect()
}

/// One `CHECK` constraint's stored `(name, Expr)` pair (a clone of the catalog IR).
pub fn prepare_check(check: &CheckConstraint) -> (String, Expr) {
    (check.name.clone(), check.expr.clone())
}

/// The single distinct base column an expression references, or `None` for zero or
/// more than one — the choice between Postgres's `<table>_<col>_check` and
/// `<table>_check` auto-names (`AddRelationNewConstraints`'s `pull_var_clause`
/// approximation of "was this written on a column or on the table"). Columns compare
/// by (unqualified) name.
#[must_use]
pub fn single_referenced_column(expr: &Expr) -> Option<String> {
    let mut cols: Vec<String> = Vec::new();
    collect_check_columns(expr, &mut cols);
    cols.sort();
    cols.dedup();
    match cols.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// Walk `expr`, pushing every referenced base column name (skipping the `*` wildcard).
fn collect_check_columns(expr: &Expr, out: &mut Vec<String>) {
    if let Expr::Column { name, .. } = expr
        && name != "*"
    {
        out.push(name.clone());
    }
    expr.for_each_child(|child| collect_check_columns(child, out));
}

/// Evaluate every prepared `CHECK` predicate against a fully-assembled row.
///
/// Uses SQL 3-valued logic: a predicate **violates** only when it resolves to
/// **false**; both true and NULL (unknown) pass. The resolution is
/// [`boolean_context_operand`](super::expr::boolean_context_operand)'s, not a bare
/// `Value::Bool` match, so a non-boolean predicate is `42804` rather than a row
/// admitted — the polarity here is what made a non-boolean `CHECK` pass *every*
/// row, the opposite direction from the filters, and the same resolver answers both.
/// `checks` comes from `prepare_checks` (prepared once per statement); `col_names`
/// is the table's column order matching `row`, and `col_types` the matching
/// declared types.
///
/// `col_types` is load-bearing, not decoration: a predicate that stringifies a
/// column — `CHECK (tz_col::text LIKE '%+00')` — must see the **same** string the
/// query does, and only the declared type says whether a `Value::Timestamp`
/// renders with its `+00` offset. Evaluating the constraint untyped rejected rows
/// Postgres accepts.
///
/// `col_enums` is load-bearing for the same reason one type further, and it decides
/// which rows are **written**: `CHECK (m < 'ok')` over an enum column keeps the members
/// declared before `ok` and refuses the rest, and evaluating it without the member
/// order kept and refused exactly the opposite set for any enum whose declared order is
/// not its lexical one.
///
/// # Errors
///
/// Returns `Error::CheckViolation` (naming the constraint minted at DDL time) for the
/// first predicate that evaluates to false, or an evaluation error.
pub fn enforce_checks(
    table: &str,
    checks: &[(String, Expr)],
    col_names: &[&str],
    col_types: &[TypeOid],
    col_enums: &[Option<&[String]>],
    row: &[Value],
) -> Result<()> {
    for (name, expr) in checks {
        let verdict = eval_expr(expr, col_names, col_types, col_enums, row)?;
        if boolean_context_operand(&verdict, BoolContext::Check)? == Some(false) {
            return Err(Error::CheckViolation {
                table: table.to_string(),
                constraint: name.clone(),
                row: Some(Box::new(render_failing_row(col_types, row))),
            });
        }
    }
    Ok(())
}

/// A key part of an enforced unique index, prepared once per statement.
#[derive(Debug)]
enum PreparedKeyPart {
    /// A plain bare-column key part, resolved to its row position.
    Column(usize),
    /// An expression key part (`LOWER(email)`, `COALESCE(...)`), the cloned IR.
    Expr(Expr),
}

/// An index with its predicate and key parts prepared once, ready to probe and to
/// size per row without re-deriving them — every index a write enters an entry into,
/// the enforced unique ones probed for a duplicate as well.
#[derive(Debug)]
pub struct PreparedIndex {
    /// The index name (for the violation error).
    name: String,
    /// Whether the index is unique **and** enforces it at write time — the indexes a
    /// write probes for a duplicate and an `ON CONFLICT` arbitrates against.
    unique: bool,
    /// The access method: a B-tree entry is one tuple of the key parts, a GIN one a
    /// tuple per decomposed element.
    method: IndexMethod,
    /// Each key part's type is an enum — the one fact about its size a
    /// `Value::Text` does not carry (`index_row::index_entry_size`).
    part_is_enum: Vec<bool>,
    /// Each key part's enum member order, where the part is an enum the catalog holds —
    /// what a build's key order ranks it by (`key_order`), as the index 18.6 builds
    /// orders an enum by its members' positions and not its labels' bytes.
    part_enum_orders: Vec<Option<Vec<String>>>,
    /// The relation the index is on — 18.6's `t` on a `23505`.
    table: String,
    /// The key columns' spelled names, in key order — the left half of 18.6's
    /// `Key (a, b)=(1, x)` DETAIL. `None` when any key part is an **expression**:
    /// neither its name nor the type its value renders under is derivable here, and a
    /// guessed one would be an invented field, so the `D` is omitted. `s`/`t`/`n` still
    /// go out — the relation is truthful whatever the key looks like, which is why
    /// `key_description` returns the payload with an `Option` key rather than an
    /// `Option` payload.
    key_names: Option<Vec<String>>,
    /// The declared type of each key part, positionally parallel to `key_names` —
    /// so a `timestamptz` key renders with its `+00` offset in the DETAIL.
    key_types: Vec<TypeOid>,
    /// The storage `index_id` (the OID as `u32`), or `None` for a prepared index
    /// whose catalog `Index` has **not been given an OID yet**.
    ///
    /// **The `None` is what makes the build gate's placeholder unreachable from
    /// `index_lookup`.** `executor::ddl`'s `execute_create_index` assembles its `Index`
    /// with `oid: 0` *before* it reads the OID at all — deliberately, so
    /// [`validate_index_build_unique`] can read the write path's own key preparation
    /// with nothing written — and every real OID starts at 16384
    /// (`executor::helpers::next_object_oid`), so `0` is a value no index has.
    /// Carried as a `u32` it was a live footgun: any future reader that probed storage
    /// with it would look up index `0` and silently see no rows, which on a uniqueness
    /// probe reads as "no conflict". As an `Option` the one probing reader
    /// ([`occupancy_of`], which both [`enforce_index_entries`] and [`probe_conflict`] go
    /// through) has to name the case, and fails loud rather than
    /// probing a placeholder.
    index_id: Option<u32>,
    /// The partial-index `WHERE` predicate (`None` for a full index).
    predicate: Option<Expr>,
    /// Each key part (a column position or a key expression), in key order.
    key_parts: Vec<PreparedKeyPart>,
    /// `NULLS NOT DISTINCT`: NULL key parts are still indexed/probed when `true`.
    nulls_not_distinct: bool,
    /// The table's column names in row order — the `col_names` the predicate and
    /// expression key parts (`eval_expr`) bind against.
    column_names: Vec<String>,
    /// The table's declared column types, positionally parallel to `column_names`
    /// — the `col_types` the same evaluation binds against, so a stored expression
    /// that stringifies a `timestamptz` keeps its `+00` offset here exactly as it
    /// does in a query (see `ColumnResolver::column_type`).
    column_types: Vec<TypeOid>,
    /// The parallel enum member orders — what makes a partial predicate's `m > 'sad'`
    /// decide membership by the members' positions here, as `IndexKeying`'s own
    /// maintenance walk decides it on the storage side (`IndexSpec::column_enum_orders`).
    /// Two rules that disagree here is a silent wrong **write**: a duplicate 18.6
    /// refuses lands, because neither row was placed in the index.
    column_enum_orders: Vec<Option<Vec<String>>>,
}

/// Gather **every** index on `table` once, up front, in the order 18.6 enters a row's
/// entries into them — ascending OID, `RelationGetIndexList`'s order — so the size
/// refusals and the duplicate probes of [`enforce_index_entries`] interleave across
/// indexes as 18.6's do.
///
/// Call this **once per statement** (before the row loop) and reuse the result for
/// every row. The partial predicate and expression key parts come straight from the
/// catalog IR (`index.predicate` / `index.key`) — cloned, not re-parsed. Plain key
/// columns are resolved to row positions here; an index whose table or a plain key
/// column can't be resolved is skipped (it can't match).
pub fn prepare_indexes(schema: &Schema, table: &str) -> Vec<PreparedIndex> {
    let Some(table_def) = schema.get_table(table) else {
        return Vec::new();
    };
    let mut indexes: Vec<&Index> = schema
        .indexes
        .values()
        .filter(|index| index.table_name == table)
        .collect();
    indexes.sort_by_key(|index| index.oid);
    indexes
        .into_iter()
        .filter_map(|index| prepare_one_index(index, table_def, &schema.enums))
        .collect()
}

/// Prepare a single `index` into the per-row probe form, or `None` if a plain key
/// column can't be resolved against `table_def` (a stale/corrupt def — the index can't
/// match any row, so it is skipped).
///
/// Gathers each key part once from the stored IR (`index.key`): expression parts
/// are cloned, plain columns resolved to row positions. The partial predicate,
/// NULL-distinctness, and the table's column names (the `eval_expr` binding scope)
/// come straight from the catalog. The single point `prepare_indexes` (per statement),
/// the `ON CONFLICT` arbiter and the index-build gates build their prepared indexes
/// through, so none re-derives the key parts.
fn prepare_one_index(
    index: &Index,
    table_def: &Table,
    enums: &BTreeMap<String, EnumType>,
) -> Option<PreparedIndex> {
    let key_parts = match &index.key {
        IndexKey::Exprs(exprs) => exprs.iter().cloned().map(PreparedKeyPart::Expr).collect(),
        IndexKey::Columns(cols) => cols
            .iter()
            .map(|col| {
                table_def
                    .columns
                    .iter()
                    .position(|c| &c.name == col)
                    .map(PreparedKeyPart::Column)
            })
            // A plain index referencing a missing column can't match any row.
            .collect::<Option<Vec<_>>>()?,
    };
    let key_names = match &index.key {
        IndexKey::Columns(cols) => Some(cols.clone()),
        IndexKey::Exprs(_) => None,
    };
    let key_types = key_parts
        .iter()
        .map(|part| match part {
            PreparedKeyPart::Column(pos) => table_def
                .columns
                .get(*pos)
                .map_or(TypeOid::Text, |c| c.type_oid),
            PreparedKeyPart::Expr(_) => TypeOid::Text,
        })
        .collect();
    let enum_column = |name: &str| {
        table_def
            .columns
            .iter()
            .any(|c| c.name == name && c.enum_type.is_some())
    };
    let part_is_enum = key_parts
        .iter()
        .map(|part| match part {
            PreparedKeyPart::Column(pos) => table_def
                .columns
                .get(*pos)
                .is_some_and(|c| c.enum_type.is_some()),
            PreparedKeyPart::Expr(expr) => key_part_is_enum(expr, &enum_column),
        })
        .collect();
    let column_enum = |name: &str| {
        table_def
            .columns
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.enum_type.clone())
    };
    let part_enum_orders = key_parts
        .iter()
        .map(|part| {
            let enum_name = match part {
                PreparedKeyPart::Column(pos) => table_def
                    .columns
                    .get(*pos)
                    .and_then(|c| c.enum_type.clone()),
                PreparedKeyPart::Expr(expr) => key_part_enum(expr, &column_enum),
            };
            super::helpers::enum_order_of(enums, enum_name.as_deref()).map(<[String]>::to_vec)
        })
        .collect();
    Some(PreparedIndex {
        name: index.name.clone(),
        unique: index.is_unique() && index.is_enforced(),
        method: index_method_for(index, table_def),
        part_is_enum,
        part_enum_orders,
        table: table_def.name.clone(),
        key_names,
        key_types,
        index_id: (index.oid != 0).then(|| index.index_id()),
        predicate: index.predicate.clone(),
        key_parts,
        nulls_not_distinct: index.nulls_not_distinct,
        column_names: table_def.columns.iter().map(|c| c.name.clone()).collect(),
        column_types: table_def.columns.iter().map(|c| c.type_oid).collect(),
        column_enum_orders: table_def
            .columns
            .iter()
            .map(|c| {
                super::helpers::enum_order_of(enums, c.enum_type.as_deref()).map(<[String]>::to_vec)
            })
            .collect(),
    })
}

impl PreparedIndex {
    /// The storage `index_id` to probe, refusing the OID-less form.
    ///
    /// The only producer of an OID-less [`PreparedIndex`] is
    /// [`validate_index_build_unique`], which scans the heap and probes nothing — so
    /// this is unreachable today and is here to *keep* it unreachable: a probe against
    /// the placeholder would answer "no rows", which a uniqueness check reads as "no
    /// conflict" and writes on.
    ///
    /// # Errors
    ///
    /// [`Error::Execution`] naming the index, if a future caller probes storage with an
    /// index the catalog has not given an OID.
    fn probe_index_id(&self) -> Result<u32> {
        self.index_id.ok_or_else(|| {
            Error::Execution(format!(
                "index \"{}\" has no OID yet and cannot be probed",
                self.name
            ))
        })
    }

    /// The `23505` structured payload for a conflict on this index — the relation
    /// always, and the colliding `Key (…)=(…)` tuple only where it can be described
    /// truthfully (see [`PreparedIndex::key_names`]).
    ///
    /// The two are separated on purpose: an **expression** key has no spelled column
    /// name and no declared type to render its value under, so its `D` is dropped — but
    /// the relation is known either way and 18.6 sends `s`/`t` there, so returning
    /// `None` for the whole payload would have dropped a truthful field with an
    /// untruthful one.
    fn key_description(&self, key_values: &[Value]) -> pglet_core::UniqueViolationDetail {
        let key = self.key_names.as_ref().map(|names| {
            KeyDescription::new(
                names.clone(),
                key_values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        self.key_types
                            .get(i)
                            .map_or_else(|| value_to_text(v), |&oid| value_to_text_with_oid(v, oid))
                    })
                    .collect(),
            )
        });
        pglet_core::UniqueViolationDetail {
            table: self.table.clone(),
            key,
        }
    }
}

/// Render a foreign key's child-side values for a `Key (…)=(…)` DETAIL, using each
/// referencing column's declared type so a `timestamptz` keeps its `+00` offset.
///
/// Shared with `executor::ddl`'s `ALTER TABLE … ADD FOREIGN KEY` validation scan, which
/// rendered the same tuple through the type-blind `value_to_text` and dropped the `+00`
/// — one field, both engines sending it, different values, which is exactly the class
/// the differential field comparison scores red. A fourth copy is how that happened, so
/// there is one renderer.
pub(super) fn render_key_values(
    table_def: &Table,
    positions: &[usize],
    values: &[Value],
) -> Vec<Option<String>> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            positions
                .get(i)
                .and_then(|&pos| table_def.columns.get(pos))
                .map_or_else(
                    || value_to_text(v),
                    |c| value_to_text_with_oid(v, c.type_oid),
                )
        })
        .collect()
}

/// Enter a row into a set of pre-prepared indexes ([`prepare_indexes`]) the way 18.6
/// does, refusing it where 18.6 would — the hoisted, per-row half of every write.
///
/// Index by index, in the set's order (18.6's OID order): the row's entry is sized
/// (`index_row`), `index_form_tuple`'s refusal is asked, an enforced unique index is
/// probed for a duplicate (excluding `row_id_being_written`, the row's own id on an
/// UPDATE, and the conflicting row's own id on a `DO UPDATE`), and then the method's own
/// item limit is asked. That
/// is 18.6's order within one index (`btinsert` forms the tuple, `_bt_doinsert` checks
/// uniqueness before `_bt_findinsertloc` checks the size) and across indexes, so a
/// table's size refusal and its `23505` come out in whichever order its indexes were
/// created (measured both ways). A row a partial index's predicate excludes enters no
/// entry there and meets none of its refusals; a NULL key part is **sized** under any
/// NULL rule — 18.6's B-tree holds NULL entries — and exempts the row only from the
/// duplicate probe, under the default `NULLS DISTINCT`.
///
/// NULL key columns, for the duplicate probe:
/// - default `NULLS DISTINCT` (`nulls_not_distinct = false`): a NULL in **any**
///   key column skips the probe — SQL unique allows multiple NULLs by default (a
///   unique index with a NULL in any column never conflicts; `index_lookup`
///   returns nothing for such a tuple anyway).
/// - `NULLS NOT DISTINCT` (`nulls_not_distinct = true`): NULL key columns are
///   probed like any value, so two NULL keys collide.
///
/// **Partial** index (`Index.predicate`): the predicate is evaluated against the
/// written row; a row that fails it is not a uniqueness candidate and is skipped
/// (no conflict). The index only contains satisfying rows, so a hit on a
/// satisfying row is a real conflict.
///
/// **Expression** index (`IndexKey::Exprs`): the key tuple is each key expression
/// evaluated against the written row via `eval_expr` (a bare-column part is an
/// `Expr::Column`, so a mixed bare/expression key folds through one path) — the
/// **same** computation the storage layer used to build the stored entry, so the
/// probe agrees with what was indexed. A plain index (`IndexKey::Columns`) keys on
/// the column values directly. An index that is not an enforced unique one is sized and
/// never probed.
///
/// Read-your-own-writes: the transaction-aware storage adapter routes
/// `index_lookup` through the pending buffer, so an earlier row in the **same**
/// multi-row statement is already visible — intra-statement duplicates are caught.
/// The cross-connection half is a second question asked beside it
/// ([`Storage::unique_key_claimed_elsewhere`](super::Storage::unique_key_claimed_elsewhere)):
/// a key another live connection's uncommitted write claims conflicts too. No host lets
/// a second writer stage beside the block holding the writer — the native server makes
/// it wait for that block, the embedded `Database` refuses it `55P03` — so under them it
/// finds nothing; it stays as the storage layer's own guard, and a backend counts a hit
/// (`cross_connection_hits`).
///
/// # Errors
///
/// [`Error::IndexRowRequiresTooMuch`], [`Error::BtreeIndexRowTooLarge`] or
/// [`Error::GinIndexRowTooLarge`] for an oversized entry; `Error::UniqueViolation` on a
/// duplicate — a committed or own-pending row, or a key another live connection's
/// uncommitted write claims; a storage error from either probe; an evaluation error
/// from a key expression or predicate.
pub fn enforce_index_entries<S: super::Storage>(
    storage: &S,
    indexes: &[PreparedIndex],
    row: &[Value],
    row_id_being_written: Option<super::types::RowId>,
) -> Result<()> {
    enter_row(
        storage,
        indexes,
        row,
        &Occupants::Write(row_id_being_written),
    )
}

/// One table's rows as a `DELETE`'s foreign-key actions have left them so far, beside the
/// storage they have not been written to yet: the image each action has planned for a row
/// it rewrites (the latest, where two actions reach one row) and the rows the statement
/// removes.
///
/// 18.6 runs each action as a statement of its own over the child, so a row two actions
/// reach is rewritten twice, the second rewrite reading the first's image, and each
/// rewrite is checked and entered into the child's indexes before the next row's. The
/// `DELETE` plan runs before any write and reads storage, which holds neither the earlier
/// rewrite nor what it did to the indexes, so it keeps this beside it: an action builds a
/// row's new image from
/// [`ActionRows::image`], and an index probe reads a planned row by its image rather than
/// by the entry storage still holds for it ([`enforce_cascaded_index_entries`]).
#[derive(Debug, Default)]
pub struct ActionRows {
    /// The latest planned image of each row an action rewrites, not yet written.
    images: BTreeMap<super::types::RowId, Vec<Value>>,
    /// The rows the statement removes — a `DELETE`'s rows and the rows it cascades to.
    removed: std::collections::HashSet<super::types::RowId>,
}

impl ActionRows {
    /// The row's latest planned image, or `None` when no action has rewritten it.
    pub fn image(&self, id: super::types::RowId) -> Option<&[Value]> {
        self.images.get(&id).map(Vec::as_slice)
    }

    /// Record `image` as the row's latest planned image.
    pub fn record(&mut self, id: super::types::RowId, image: Vec<Value>) {
        self.images.insert(id, image);
    }

    /// Record that the statement removes the row.
    pub fn remove(&mut self, id: super::types::RowId) {
        self.removed.insert(id);
    }

    /// Whether the statement removes the row.
    pub fn is_removed(&self, id: super::types::RowId) -> bool {
        self.removed.contains(&id)
    }

    /// Whether the entries storage holds for the row are no longer its own: it has been
    /// rewritten or removed, so an index probe that finds it has found a dead version.
    fn supersedes(&self, id: super::types::RowId) -> bool {
        self.images.contains_key(&id) || self.removed.contains(&id)
    }

    /// The planned images of every rewritten row but `id` that the statement keeps.
    fn siblings(&self, id: super::types::RowId) -> impl Iterator<Item = &[Value]> {
        self.images
            .iter()
            .filter(move |(other, _)| **other != id && !self.removed.contains(other))
            .map(|(_, image)| image.as_slice())
    }
}

/// [`enforce_index_entries`] for a row a foreign-key action rewrites, beside what the
/// statement's actions have already done to the row's table (`rows`).
///
/// 18.6 writes a rewritten row's entries into each index in turn, every earlier rewrite
/// already written: an earlier rewritten row holds its new key there, and its old entry
/// is a dead version no probe counts. So under an enforced unique index, a planned
/// image holding the row's key is that index's duplicate, in its place in the per-index
/// order — before a later index's size refusal, after an earlier one's — and a storage
/// entry of a row `rows` has rewritten or removed is not. A storage probe cannot see a
/// planned image (nothing is written until the plan has run), so each is compared here,
/// under the index's own rule for NULL parts. That `23505` names the index and carries
/// no `Key` detail.
///
/// # Errors
///
/// As [`enforce_index_entries`], and `Error::UniqueViolation` for a planned image's key.
pub fn enforce_cascaded_index_entries<S: super::Storage>(
    storage: &S,
    indexes: &[PreparedIndex],
    row: &[Value],
    row_id_being_written: super::types::RowId,
    rows: &ActionRows,
) -> Result<()> {
    enter_row(
        storage,
        indexes,
        row,
        &Occupants::Action {
            id: row_id_being_written,
            rows,
        },
    )
}

/// Whose stored entry a duplicate probe discounts, and which unwritten images it
/// compares.
enum Occupants<'a> {
    /// A plain write: every holder of the key but the row being written conflicts.
    Write(Option<super::types::RowId>),
    /// A row a foreign-key action rewrites: its own and every superseded entry are
    /// discounted, and the planned images are compared ([`enforce_cascaded_index_entries`]).
    Action {
        id: super::types::RowId,
        rows: &'a ActionRows,
    },
}

impl Occupants<'_> {
    /// Whether a stored entry of `occupant` is no conflict for this row.
    fn discounts(&self, occupant: super::types::RowId) -> bool {
        match self {
            Self::Write(own) => *own == Some(occupant),
            Self::Action { id, rows } => *id == occupant || rows.supersedes(occupant),
        }
    }
}

/// The one per-row loop behind [`enforce_index_entries`] and
/// [`enforce_cascaded_index_entries`].
fn enter_row<S: super::Storage>(
    storage: &S,
    indexes: &[PreparedIndex],
    row: &[Value],
    occupants: &Occupants<'_>,
) -> Result<()> {
    for index in indexes {
        let Some(values) = index_entry_values(index, row)? else {
            continue;
        };
        match index.method {
            IndexMethod::Gin => {
                if let Some(key) = values.first() {
                    check_gin_entries(key, &gin_decompose(key), &index.name, &index.table)?;
                }
            }
            IndexMethod::Btree => {
                let size = index_entry_size(values.iter().zip(index.part_is_enum.iter().copied()));
                check_entry_size_field(size)?;
                if index.unique
                    && let Some(key_values) = unique_candidate(index, values)
                {
                    if let Occupants::Action { id, rows } = occupants {
                        for sibling in rows.siblings(*id) {
                            if prepared_unique_index_key(index, sibling)?.is_some_and(|other| {
                                other.len() == key_values.len()
                                    && other.iter().zip(&key_values).all(|(a, b)| keys_eq(a, b))
                            }) {
                                return Err(Error::UniqueViolation {
                                    constraint: index.name.clone(),
                                    key: None,
                                });
                            }
                        }
                    }
                    reject_occupancy(index, &occupancy_of(storage, index, key_values)?, |id| {
                        occupants.discounts(id)
                    })?;
                }
                check_btree_item_size(size, &index.name, &index.table)?;
            }
        }
    }
    Ok(())
}

/// The `23505` for an occupancy that conflicts: a foreign claim, or a readable row the
/// probe does not discount.
fn reject_occupancy(
    index: &PreparedIndex,
    occupancy: &KeyOccupancy,
    discounted: impl Fn(super::types::RowId) -> bool,
) -> Result<()> {
    let conflict = occupancy.claimed_elsewhere || occupancy.rows.iter().any(|&id| !discounted(id));
    if conflict {
        return Err(Error::UniqueViolation {
            constraint: index.name.clone(),
            key: Some(Box::new(index.key_description(&occupancy.key_values))),
        });
    }
    Ok(())
}

/// What holds a prepared unique index's key tuple for one row.
struct KeyOccupancy {
    /// The key tuple itself, kept so the caller renders the violation DETAIL from the
    /// values the probe actually used rather than re-deriving them.
    key_values: Vec<Value>,
    /// The row ids this connection can read that hold the tuple — committed rows plus
    /// its own staged writes.
    rows: Vec<super::types::RowId>,
    /// Whether another live connection's **uncommitted** write claims the tuple. There
    /// is no readable row behind this and there may never be one: the claiming write
    /// can still roll back.
    claimed_elsewhere: bool,
}

/// The one occupancy derivation both uniqueness probes read, or `None` when `row` is
/// not a uniqueness candidate for `index` (a partial predicate it fails, a NULL key
/// part under `NULLS DISTINCT`, a missing key part).
///
/// Both halves are asked here, at one place, because they are one question. The write
/// path ([`enforce_index_entries`]) and the `ON CONFLICT` arbiter ([`probe_conflict`])
/// reaching two different derivations is how the arbiter came to see a conflict
/// enforcement would not raise, and the reverse; this is the seam that keeps them from
/// disagreeing about what occupies a key.
///
/// # Errors
///
/// A storage error from either probe; an evaluation error from a key expression or the
/// partial predicate.
fn probe_key_occupancy<S: super::Storage>(
    storage: &S,
    index: &PreparedIndex,
    row: &[Value],
) -> Result<Option<KeyOccupancy>> {
    let Some(key_values) = prepared_unique_index_key(index, row)? else {
        return Ok(None);
    };
    occupancy_of(storage, index, key_values).map(Some)
}

/// What holds `key_values` in `index`, asked of both halves — this connection's
/// readable rows and every other live connection's claims.
fn occupancy_of<S: super::Storage>(
    storage: &S,
    index: &PreparedIndex,
    key_values: Vec<Value>,
) -> Result<KeyOccupancy> {
    let index_id = index.probe_index_id()?;
    let rows = storage.index_lookup(index_id, &key_values)?;
    let claimed_elsewhere = storage.unique_key_claimed_elsewhere(index_id, &key_values)?;
    Ok(KeyOccupancy {
        key_values,
        rows,
        claimed_elsewhere,
    })
}

/// The key tuple `row` would occupy in a **pre-prepared** unique index, or `None` if
/// the row is not a uniqueness candidate (a partial predicate the row fails, a NULL
/// key part under `NULLS DISTINCT`, or a missing key part). The per-row probe, over
/// the once-prepared (`prepare_one_index`) forms — the arbiter's storage probe
/// ([`probe_key_occupancy`]) and the in-memory cascade sibling check
/// ([`enforce_cascaded_index_entries`]) key through it.
///
/// For an EXPRESSION key part each value is `eval_expr`'d against the row — the same
/// computation the storage layer used to build the stored entry, so the probe agrees
/// with what was indexed; a plain part is a bare column read.
///
/// # Errors
///
/// An evaluation error from a key expression or the partial predicate.
fn prepared_unique_index_key(index: &PreparedIndex, row: &[Value]) -> Result<Option<Vec<Value>>> {
    Ok(index_entry_values(index, row)?.and_then(|values| unique_candidate(index, values)))
}

/// The key values `row` enters into `index` — NULL parts included — or `None` when the
/// index's partial predicate excludes the row, which then enters no entry at all.
///
/// The partial predicate is resolved by the same rule the storage side's
/// `IndexKeying::row_in_index` uses — the two decide the *same* predicate over the
/// *same* row in two crates, and a row enforced-unique but unindexed (or the reverse)
/// is what moving one alone costs. For an EXPRESSION key part each value is
/// `eval_expr`'d against the row — the same computation the storage layer used to build
/// the stored entry; a plain part is a bare column read.
///
/// # Errors
///
/// An evaluation error from a key expression or the partial predicate.
fn index_entry_values(index: &PreparedIndex, row: &[Value]) -> Result<Option<Vec<Value>>> {
    let col_names: Vec<&str> = index.column_names.iter().map(String::as_str).collect();
    let col_enums: Vec<Option<&[String]>> = index
        .column_enum_orders
        .iter()
        .map(|o| o.as_deref())
        .collect();
    if let Some(pred) = &index.predicate
        && !predicate_holds(
            &eval_expr(pred, &col_names, &index.column_types, &col_enums, row)?,
            BoolContext::Where,
        )?
    {
        return Ok(None);
    }
    let mut key_values: Vec<Value> = Vec::with_capacity(index.key_parts.len());
    for part in &index.key_parts {
        key_values.push(match part {
            PreparedKeyPart::Expr(expr) => {
                eval_expr(expr, &col_names, &index.column_types, &col_enums, row)?
            }
            PreparedKeyPart::Column(pos) => row.get(*pos).cloned().unwrap_or(Value::Null),
        });
    }
    Ok(Some(key_values))
}

/// `values` as a uniqueness candidate for `index`: under the default `NULLS DISTINCT` a
/// NULL in **any** key part means the row never conflicts; under `NULLS NOT DISTINCT`
/// NULL keys are probed, so two NULLs collide.
fn unique_candidate(index: &PreparedIndex, values: Vec<Value>) -> Option<Vec<Value>> {
    (index.nulls_not_distinct || !values.iter().any(Value::is_null)).then_some(values)
}

/// Validate that `table_def`'s existing rows hold no two entries `index`'s own equality
/// calls equal — the **index build** 18.6 performs inside `index_create`, before the
/// index exists.
///
/// `CREATE UNIQUE INDEX` had no such gate: it registered the index and back-filled it,
/// so an index over already-duplicate rows was created and simply never held, after
/// which the two engines disagreed on row *contents* — `INSERT` of a third duplicate
/// was `INSERT 0 1` on 18.6 (which never created the index) and `23505` here. The
/// `ALTER TABLE … ADD CONSTRAINT UNIQUE` / `ADD PRIMARY KEY` routes already validate
/// through `executor::ddl`'s `validate_existing_unique` bare-column scan; this is the same gate for
/// the route that carries a predicate, expression key parts and `NULLS NOT DISTINCT`,
/// and it reuses the **write path's own** preparation ([`prepare_one_index`] +
/// [`prepared_unique_index_key`]) rather than re-deriving the key rules, so what the
/// build refuses and what a later write refuses cannot drift.
///
/// **The caller filters on `Index::is_enforced`**, which is the set
/// [`enforce_index_entries`] probes per row — so the build refuses exactly what
/// a later write would refuse, and a `unique && !enforced` index (the accelerate-only
/// mode) is neither. On the `CREATE [UNIQUE] INDEX` route the two are the same set,
/// `enforced` being `unique`; the distinction is what keeps that true if the route ever
/// changes. A row the partial predicate excludes, or one carrying a NULL key part under
/// the default `NULLS DISTINCT`, is not a uniqueness candidate and is skipped, exactly
/// as at write time.
///
/// The `23505` is [`Error::DuplicateIndexKey`] — 18.6's *"could not create unique index
/// … Key (b)=(7) is duplicated."*, the build route's own message and DETAIL, not the
/// per-row *"already exists"* one.
///
/// Equality is [`index_key_identity`]'s, which is the **index's own**; a structural
/// (postcard) comparison is not the same relation and calling `1.5` and `1.50` distinct
/// is where it parts.
///
/// # Errors
///
/// [`Error::DuplicateIndexKey`] on the first duplicate key tuple; a storage error from
/// the scan; an evaluation error from a key expression or the partial predicate.
pub fn validate_index_build_unique<S: super::Storage>(
    storage: &S,
    table_def: &Table,
    index: &Index,
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    // `None` only for a plain key naming a column the table does not have, which the
    // caller has already refused (`validate_columns_exist`).
    let Some(prepared) = prepare_one_index(index, table_def, enums) else {
        return Ok(());
    };
    // The map keeps the **first** row's key values under each identity, because that is
    // the tuple 18.6 names: over a `numeric` column holding `1.50` then `1.5` its DETAIL
    // reads `Key (n)=(1.50)`, and with the rows the other way round `Key (n)=(1.5)` —
    // scan order, not the row that triggered. Reporting the colliding row instead named
    // the other one of an equal pair whenever the two spellings differ.
    let mut seen: HashMap<Vec<u8>, Vec<Value>> = HashMap::new();
    for (_, row) in storage.scan(&table_def.name)? {
        let Some(key_values) = prepared_unique_index_key(&prepared, &row)? else {
            continue;
        };
        match seen.entry(index_key_identity(&key_values)?) {
            std::collections::hash_map::Entry::Occupied(first) => {
                return Err(Error::DuplicateIndexKey {
                    index: prepared.name.clone(),
                    detail: Some(Box::new(prepared.key_description(first.get()))),
                });
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(key_values);
            }
        }
    }
    Ok(())
}

/// The size refusals an index build over existing rows meets, split at the point 18.6's
/// build raises its duplicate refusal between them.
///
/// 18.6 builds a B-tree by spooling every heap tuple — `index_form_tuple`'s refusal,
/// raised in heap order — then sorting the spool, where a unique index's duplicate is
/// raised (`could not create unique index`), then loading the sorted entries, where
/// `_bt_check_third_page` refuses the first oversized one **in key order**. So a table
/// holding both an entry past 8 191 bytes and a duplicate answers the first, one holding
/// an oversized entry and a duplicate answers the duplicate, and two oversized entries
/// name the one whose key sorts first (each measured). A GIN build refuses inside
/// `GinFormTuple` as it inserts its accumulated entries, sorted, so both of its refusals
/// fall after the (absent) duplicate check. pglet reads the rows in row-id order, which
/// agrees with the heap's over rows only ever inserted; once an `UPDATE` has moved a row
/// to the heap's end, the size-field refusal can name another row (`EDGES.md` row 194).
#[derive(Debug, Default)]
pub struct BuildSizeRefusals {
    /// `index_form_tuple`'s refusal, which a B-tree build raises before its duplicate
    /// check.
    pub before_duplicates: Option<Error>,
    /// The method's item-limit refusal, raised after it.
    pub after_duplicates: Option<Error>,
    /// pglet's own refusal of a key too long for its page
    /// (`index_row::check_page_key_len`), which stands in for no 18.6 error and so is
    /// raised after every refusal of the statement that does — an `ADD PRIMARY KEY`'s
    /// `23502` included, which 18.6 raises after the whole build.
    pub page_bound: Option<Error>,
}

impl BuildSizeRefusals {
    /// Raise the refusal that precedes the duplicate check, if any.
    ///
    /// # Errors
    ///
    /// That refusal.
    pub fn raise_before_duplicates(&mut self) -> Result<()> {
        self.before_duplicates.take().map_or(Ok(()), Err)
    }

    /// Raise the refusal that follows the duplicate check, if any.
    ///
    /// # Errors
    ///
    /// That refusal.
    pub fn raise_after_duplicates(&mut self) -> Result<()> {
        self.after_duplicates.take().map_or(Ok(()), Err)
    }

    /// Raise pglet's own page-bound refusal, if any — the caller's last fallible step
    /// that reads the build.
    ///
    /// # Errors
    ///
    /// That refusal.
    pub fn raise_page_bound(&mut self) -> Result<()> {
        self.page_bound.take().map_or(Ok(()), Err)
    }
}

/// Size every entry an index build over `table_def`'s existing rows would write, and
/// return the refusals 18.6's build raises over them ([`BuildSizeRefusals`]), without
/// raising either — the caller interleaves them with its duplicate check.
///
/// The entries are `index`'s own: the write path's preparation ([`prepare_one_index`] +
/// [`index_entry_values`]) keys each row, partial predicate and expression parts
/// included, so what a build refuses and what a later write refuses cannot drift. The
/// scan runs before any catalog or storage write, which is what keeps a refused build
/// from leaving a catalog fact behind (`docs/ddl.md`'s failed-DDL rule).
///
/// # Errors
///
/// A storage error from the scan; an evaluation error from a key expression or the
/// partial predicate.
pub fn scan_index_build_sizes<S: super::Storage>(
    storage: &S,
    table_def: &Table,
    index: &Index,
    enums: &BTreeMap<String, EnumType>,
) -> Result<BuildSizeRefusals> {
    let mut refusals = BuildSizeRefusals::default();
    let Some(prepared) = prepare_one_index(index, table_def, enums) else {
        return Ok(refusals);
    };
    // The oversized entry to name: a B-tree's key tuple, or a GIN's first element.
    let mut first_over_limit: Option<(Vec<Value>, usize)> = None;
    // pglet's own page bound, first in scan order.
    let mut page_bound: Option<Error> = None;
    let mut note_key = |key: &[Value], nulls_not_distinct: bool| {
        if page_bound.is_none()
            && let Err(e) =
                check_page_key_len(encoded_key_len(key, nulls_not_distinct), &prepared.name)
        {
            page_bound = Some(e);
        }
    };
    let mut consider = |key: Vec<Value>, size: usize| {
        let sorts_first = first_over_limit.as_ref().is_none_or(|(best, _)| {
            key_order(&key, best, &prepared.part_enum_orders) == std::cmp::Ordering::Less
        });
        if sorts_first {
            first_over_limit = Some((key, size));
        }
    };
    for (_, row) in storage.scan(&table_def.name)? {
        let Some(values) = index_entry_values(&prepared, &row)? else {
            continue;
        };
        match prepared.method {
            IndexMethod::Btree => {
                let size =
                    index_entry_size(values.iter().zip(prepared.part_is_enum.iter().copied()));
                if refusals.before_duplicates.is_none()
                    && let Err(e) = check_entry_size_field(size)
                {
                    refusals.before_duplicates = Some(e);
                }
                note_key(&values, prepared.nulls_not_distinct);
                if let Some(size) =
                    size.filter(|&size| size > super::index_row::BTREE_MAX_ITEM_SIZE)
                {
                    consider(values, size);
                }
            }
            IndexMethod::Gin => {
                if let Some(key) = values.first()
                    && matches!(key, Value::Array(_))
                {
                    for element in gin_decompose(key) {
                        note_key(std::slice::from_ref(&element), false);
                        if let Some(size) = index_entry_size([(&element, false)])
                            .filter(|&size| size > super::index_row::GIN_MAX_ITEM_SIZE)
                        {
                            consider(vec![element], size);
                        }
                    }
                }
            }
        }
    }
    refusals.page_bound = page_bound;
    if let Some((_, size)) = first_over_limit {
        refusals.after_duplicates = Some(match prepared.method {
            IndexMethod::Btree => {
                check_btree_item_size(Some(size), &prepared.name, &prepared.table)
                    .err()
                    .unwrap_or(Error::IndexRowRequiresTooMuch { size })
            }
            IndexMethod::Gin => match check_entry_size_field(Some(size)) {
                Err(e) => e,
                Ok(()) => {
                    Error::GinIndexRowTooLarge(Box::new(pglet_core::error_fields::IndexRowSize {
                        size,
                        index: prepared.name,
                        table: prepared.table,
                    }))
                }
            },
        });
    }
    Ok(refusals)
}

/// A B-tree's key order over two key tuples, for naming the oversized entry a build
/// loads first: part by part, NULLs last (the default `ASC NULLS LAST`), each part by
/// the engine's value comparison — an enum part by its members' positions
/// (`enum_orders`, one per part), as 18.6's index orders it.
///
/// A key part's written `DESC` or `NULLS FIRST` is not in the catalog to read (the
/// parser consumes and drops it, `EDGES.md` row 92), so such a build names the entry
/// that sorts first ascending; and a text part compares by bytes, which is the `C`
/// collation's order (row 43).
fn key_order(a: &[Value], b: &[Value], enum_orders: &[Option<Vec<String>>]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let ord = match (x.is_null(), y.is_null()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => super::expr::values_compare_ordered(
                x,
                y,
                enum_orders.get(i).and_then(Option::as_deref),
            )
            .unwrap_or(Ordering::Equal),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// A key tuple's identity **under the index's own equality**, for the two validation
/// scans that have to decide "would this index already hold two equal entries?".
///
/// The relation an index enforces is its *key encoding*'s, not the `Value`'s structural
/// one, and the two are different relations: `Value::encode_index_key` is
/// order-preserving and therefore normalises what ordering cannot distinguish, so a
/// `numeric` `1.5` and `1.50` encode alike while `postcard` keeps them apart. Both scans
/// used to compare postcard bytes, and both let that pair through — `CREATE UNIQUE INDEX`
/// and `ALTER TABLE … ADD CONSTRAINT UNIQUE` / `ADD PRIMARY KEY` alike built an index
/// 18.6 refuses (`Key (n)=(1.5) is duplicated.`), after which the *write* path refused
/// the very row that was already there. One equality, one function, all three routes.
///
/// The framing mirrors `pglet_storage`'s `encode_probe_key`: a per-segment presence tag,
/// so a NULL cannot collide with a present value whose encoding is empty (`Null`,
/// `Text("")` and `Bytea([])` all encode to no bytes), plus a length prefix so two
/// segments cannot run together. It does **not** call that function — `pglet_storage`
/// depends on `pglet_sql` and not the reverse — and it does not need its
/// order-preserving *composite* fold either, equality over the tagged segment list being
/// the same relation.
///
/// Every value is compared by its key, a container included: `jsonb '1'` beside `'1.0'`
/// and `'0.0'::float8` beside `'-0.0'::float8` are each the `23505` 18.6 raises.
pub(super) fn index_key_identity(key_values: &[Value]) -> Result<Vec<u8>> {
    // Segment tags, `encode_probe_key`'s two.
    const SEG_NULL: u8 = 0;
    const SEG_ENCODED: u8 = 1;
    let mut out: Vec<u8> = Vec::with_capacity(key_values.len() * 16);
    for value in key_values {
        if value.is_null() {
            out.push(SEG_NULL);
            continue;
        }
        let bytes = value.encode_index_key()?;
        out.push(SEG_ENCODED);
        out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

/// Resolve an `ON CONFLICT (target)` arbiter: the enforced unique index whose key
/// parts and partial predicate match `target`, prepared for per-row probing.
///
/// This is Postgres "index inference": the arbiter is the unique index whose key parts
/// are the **same set** as the target's (bare columns or key expressions like
/// `COALESCE(scope_kind, '')`) and whose partial predicate matches. Neither the written
/// order nor a repeat reaches that verdict, and a name compares by exact bytes — both
/// rules are [`conflict_target_matches`]'s, stated in full there. The first matching
/// enforced unique index wins. Resolve once per statement (the target is row-
/// independent), then probe each row through [`probe_conflict`].
///
/// # Errors
///
/// `Error::NoConflictTarget` (SQLSTATE `42P10`) if no enforced unique index matches
/// — Postgres's "no unique or exclusion constraint matching the ON CONFLICT
/// specification".
pub fn match_conflict_index(
    schema: &Schema,
    table: &str,
    target: &ConflictTarget,
) -> Result<PreparedIndex> {
    let Some(table_def) = schema.get_table(table) else {
        return Err(Error::NoConflictTarget);
    };
    schema
        .indexes
        .values()
        .filter(|index| index.table_name == table && index.is_unique() && index.is_enforced())
        .filter(|index| conflict_target_matches(index, target))
        .find_map(|index| prepare_one_index(index, table_def, &schema.enums))
        .ok_or(Error::NoConflictTarget)
}

/// Whether `index`'s key parts and partial predicate match the `ON CONFLICT` target.
///
/// The partial predicate must match exactly (a bare `ON CONFLICT (cols)` with no
/// `WHERE` does **not** match a partial index — Postgres requires the predicate be
/// restated); both predicates come from the same parser, so a structural `==` is
/// sound.
///
/// **The key parts are matched as sets, not as sequences**, and that is
/// `infer_arbiter_indexes`'s own rule (`optimizer/util/plancat.c`, whose header says an
/// exact match is required "although they can appear in any order"). It splits both
/// sides the way the catalog stores an index — plain attributes into a `Bitmapset`
/// tested with `bms_equal`, key expressions into a list tested with `list_member` one
/// way and `list_difference` the other — so on each half order is irrelevant and a
/// repeat collapses. [`split_key_parts`] reproduces that split over pglet's key shapes
/// and [`same_set`] the two-way containment.
///
/// So each of these matches on 18.6 and now here, where each was `42P10`:
/// `ON CONFLICT (a, a)` and `("a", a)` against a `PRIMARY KEY (a)`; `(b, a)` — reordered,
/// with no repeat at all — and `(a, a, b)` against a `UNIQUE (a, b)`; `(y, lower(x))` and
/// `(lower(x), lower(x), y)` against a `UNIQUE INDEX ((lower(x)), y)`; and `(a)` against
/// a `UNIQUE INDEX (a, a)`, a two-segment index over one column both engines accept.
/// **Set equality is still equality**, which is what the reordered member says and what
/// keeps a subset out: `(a, a)` against a `UNIQUE (a, b)` is `42P10` on both, as is
/// `(a, b)` against a `PRIMARY KEY (a)`.
///
/// A target part that is neither a bare column nor an expression the index holds falls
/// on the expression side and matches nothing a plain index offers, which is where a
/// **qualified** reference lands: `ON CONFLICT (u.a)` is `42P10` here and a grammar
/// `42601` on 18.6, a divergence this split leaves exactly where it found it.
///
/// There is no casing for either comparison to reconcile — both sides are produced
/// by the same parser and every name in them went through its fold, so an unquoted
/// spelling is already lowercase on both. The case-insensitive form this used to
/// carry made `ON CONFLICT (userid)` match an index over `"UserId"`, which is
/// `42703` on 18.6, and it reached a **write** (the `DO UPDATE` then ran).
fn conflict_target_matches(index: &Index, target: &ConflictTarget) -> bool {
    if index.predicate != target.predicate {
        return false;
    }
    let (index_columns, index_exprs) = match &index.key {
        IndexKey::Columns(cols) => (cols.iter().map(String::as_str).collect(), Vec::new()),
        IndexKey::Exprs(exprs) => split_key_parts(exprs),
    };
    let (target_columns, target_exprs) = split_key_parts(&target.keys);
    same_set(&index_columns, &target_columns) && same_set(&index_exprs, &target_exprs)
}

/// Split a list of key parts into the bare-column names and the expression parts.
///
/// The split 18.6's catalog makes for it: a plain column is an `indkey` attribute number
/// and an expression is an `indexprs` entry, and `infer_arbiter_indexes` matches the two
/// halves by different means. A **qualified** column reference is not a plain attribute
/// (18.6's inference elements are `Var`s, produced only from an unqualified
/// `index_elem`), so it stays on the expression side here.
fn split_key_parts(parts: &[Expr]) -> (Vec<&str>, Vec<&Expr>) {
    let mut columns = Vec::new();
    let mut exprs = Vec::new();
    for part in parts {
        match part {
            Expr::Column { table: None, name } => columns.push(name.as_str()),
            other => exprs.push(other),
        }
    }
    (columns, exprs)
}

/// Whether two key-part lists hold the same members, ignoring order and repeats.
///
/// Two-way containment, which is what `bms_equal` gives the attribute half and what
/// `list_member` plus `list_difference` give the expression half. The lists are one half
/// of an index's key parts or of an inference list's, so they are short and the quadratic
/// walk is the cheap form.
fn same_set<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    a.iter().all(|x| b.contains(x)) && b.iter().all(|x| a.contains(x))
}

/// What the `ON CONFLICT` arbiter found holding `row`'s key tuple.
///
/// Three answers rather than an `Option<RowId>`, because the third one is neither a
/// row nor the absence of a conflict: another connection's staged write can claim the
/// key with no row this connection can read, and an `Option` collapses that onto
/// "free", where `DO NOTHING` would insert a duplicate. Under pglet's hosts the third
/// answer is not reached — a second writer waits for the block holding the writer on
/// the native server and is refused `55P03` by the embedded `Database`, so nothing
/// another connection staged is pending when the arbiter probes — and it stays as the
/// storage guard's answer (`Storage::unique_key_claimed_elsewhere`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbiterConflict {
    /// Nothing holds the key tuple, or `row` is not a uniqueness candidate for this
    /// index at all: the row takes the ordinary insert path.
    Free,
    /// An existing row this connection can read holds it.
    Row(super::types::RowId),
    /// Another live connection's uncommitted write claims it, so there is no row here
    /// to skip past or to update. Not reached under a host that lets one block stage
    /// writes at a time.
    ClaimedElsewhere,
}

/// Probe the arbiter index for what holds `row`'s key tuple.
///
/// `index` is the [`match_conflict_index`] result. The occupancy is computed by the
/// **same** [`probe_key_occupancy`] the write-path probe
/// ([`enforce_index_entries`]) computes, so the conflict the arbiter sees is the one
/// enforcement would raise. Read-your-own-writes: the storage adapter routes
/// `index_lookup` through the pending buffer, so an earlier row in the same multi-row
/// INSERT is already visible.
///
/// A readable row outranks a foreign claim when both hold the key: it is the row the
/// clause's action actually operates on, and the claim adds nothing an action could
/// use.
///
/// # Errors
///
/// A storage error from either probe; an evaluation error from a key expression or
/// the partial predicate.
pub fn probe_conflict<S: super::Storage>(
    storage: &S,
    index: &PreparedIndex,
    row: &[Value],
) -> Result<ArbiterConflict> {
    let Some(occupancy) = probe_key_occupancy(storage, index, row)? else {
        return Ok(ArbiterConflict::Free);
    };
    if let Some(&id) = occupancy.rows.first() {
        return Ok(ArbiterConflict::Row(id));
    }
    if occupancy.claimed_elsewhere {
        return Ok(ArbiterConflict::ClaimedElsewhere);
    }
    Ok(ArbiterConflict::Free)
}

/// What a **target-less** `ON CONFLICT DO NOTHING` faces across the *whole* prepared
/// unique-index set — the set it arbitrates against, naming no single index.
///
/// Three answers, and the ordering between them is the rule: a readable row **anywhere**
/// outranks a claim **everywhere**. 18.6 pre-checks every unique index and does nothing
/// as soon as one reports a conflict with a live tuple (18.6's own
/// `ExecCheckIndexConstraints` over an empty arbiter list), and a claim it waits on can
/// only confirm that verdict or vanish — so where any readable row already makes the
/// insert a no-op, both holder outcomes skip. Measured over
/// `t(a INT UNIQUE, b INT UNIQUE)` holding `(1,100)` with a second connection's block
/// staging `(2,200)`: `INSERT (1,200)` and `INSERT (2,100)` are both `INSERT 0 0` on 18.6
/// whether that block commits or rolls back, while `(2,300)` — a claim and nothing else —
/// is `INSERT 0 0` on commit and `INSERT 0 1` on rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetLessConflict {
    /// Nothing occupies any of the row's enforced unique keys.
    Free,
    /// A row this connection can read occupies at least one of them, so the insert is
    /// already a no-op under every holder outcome and the clause skips.
    Row,
    /// No readable row occupies any of them, and at least one carries another live
    /// connection's uncommitted claim — the claim is the *sole* reason the row cannot be
    /// written, and the only shape a skip would decide on the holder's behalf. Not
    /// reached under a host that lets one block stage writes at a time.
    ClaimedElsewhere,
}

/// Classify a target-less `ON CONFLICT DO NOTHING`'s conflict across every enforced
/// unique index of the row's table.
///
/// The question the clause must ask before it skips, and it is a property of the whole
/// set rather than of any one index: [`enforce_index_entries`] stops at the *first*
/// index with an occupancy, so the index it names says nothing about what the other
/// indexes hold. Deciding at that index — or raising as soon as some index carries a
/// claim — makes the answer depend on probe order, which no 18.6 behaviour does.
///
/// A skip is licensed by [`TargetLessConflict::Row`] alone. Over
/// [`TargetLessConflict::ClaimedElsewhere`] the statement is refused: a skip is 18.6's
/// answer only if the claiming write goes on to commit, and when it rolls back 18.6
/// writes the row while a skip loses it with no error at all — the one class the
/// correct-or-loud axiom exists to prevent, and which of the two happens is unknowable
/// at the statement.
///
/// Each index is asked through [`probe_conflict`] rather than a rule of its own, so the
/// per-index precedence is the arbiter's too. Reached only once a conflict is already
/// known, so the walk is off the uncontended path.
///
/// # Errors
///
/// A storage error from either probe; an evaluation error from a key expression or a
/// partial predicate.
pub fn probe_target_less_conflict<S: super::Storage>(
    storage: &S,
    indexes: &[PreparedIndex],
    row: &[Value],
) -> Result<TargetLessConflict> {
    let mut claimed = false;
    for index in indexes.iter().filter(|index| index.unique) {
        match probe_conflict(storage, index, row)? {
            // The top of the order: no later index can change this verdict, so the walk
            // stops — and stopping here is not the order-dependence above, because every
            // other answer is dominated by this one.
            ArbiterConflict::Row(_) => return Ok(TargetLessConflict::Row),
            ArbiterConflict::ClaimedElsewhere => claimed = true,
            ArbiterConflict::Free => {}
        }
    }
    Ok(if claimed {
        TargetLessConflict::ClaimedElsewhere
    } else {
        TargetLessConflict::Free
    })
}

/// Index-key equality for the in-memory sibling check: like SQL `=` but a NULL key
/// part equals a NULL key part (the `NULLS NOT DISTINCT` collision the caller is
/// looking for — `prepared_unique_index_key` only yields NULL parts for such
/// indexes).
fn keys_eq(a: &Value, b: &Value) -> bool {
    match (a.is_null(), b.is_null()) {
        (true, true) => true,
        (false, false) => super::expr::values_equal(a, b) == Some(true),
        _ => false,
    }
}

/// Verify every `FOREIGN KEY` on `table` against a row about to be written
/// (INSERT / UPDATE / ON CONFLICT DO UPDATE / streaming INSERT — the child side).
///
/// Builds each FK's child value **tuple** (`columns[i]` in order) and, under **MATCH
/// SIMPLE** (Postgres's default), skips the whole constraint if **any** tuple element
/// is NULL — the row is "not yet pointing anywhere". Otherwise the referenced parent
/// tuple must exist, checked by [`parent_tuple_exists`]: probe the parent's covering
/// unique index (permuting the child values into the index's key order), else a
/// full-scan fallback.
///
/// Read-your-own-writes: the transaction-aware storage adapter routes
/// `index_lookup`/`scan` through the pending buffer, so a parent inserted earlier in
/// the **same** statement/transaction is already visible.
///
/// # Errors
///
/// `Error::ForeignKeyViolation` (SQLSTATE `23503`) naming the constraint and child
/// table when the parent is absent; a storage error from the probe/scan.
pub fn enforce_foreign_keys<S: super::Storage>(
    storage: &S,
    schema: &Schema,
    table: &str,
    row: &[Value],
) -> Result<()> {
    let Some(table_def) = schema.get_table(table) else {
        return Ok(());
    };
    for fk in &table_def.foreign_keys {
        // Resolve the FK's child columns to row positions.
        let Some(positions) = fk_child_positions(table_def, fk) else {
            continue;
        };
        enforce_foreign_key(storage, schema, table_def, fk, &positions, row)?;
    }
    Ok(())
}

/// The row positions of `fk`'s referencing columns in `table_def`, or `None` when one
/// names no column.
pub(super) fn fk_child_positions(table_def: &Table, fk: &ForeignKey) -> Option<Vec<usize>> {
    fk.columns
        .iter()
        .map(|col| table_def.columns.iter().position(|c| &c.name == col))
        .collect()
}

/// [`enforce_foreign_keys`] for one of `table_def`'s foreign keys, its referencing
/// columns at `positions`: skipped under MATCH SIMPLE when a key part is NULL, else the
/// referenced tuple must exist.
///
/// # Errors
///
/// `Error::ForeignKeyViolation` (SQLSTATE `23503`) when the parent is absent; a storage
/// error from the probe/scan.
pub(super) fn enforce_foreign_key<S: super::Storage>(
    storage: &S,
    schema: &Schema,
    table_def: &Table,
    fk: &ForeignKey,
    positions: &[usize],
    row: &[Value],
) -> Result<()> {
    // MATCH SIMPLE: a NULL in ANY FK column skips the whole constraint.
    let Some(pair_values) = fk_child_tuple(row, positions) else {
        return Ok(());
    };
    if parent_tuple_exists(
        storage,
        schema,
        &fk.parent_table,
        &fk.parent_columns,
        &pair_values,
    )? {
        return Ok(());
    }
    Err(Error::ForeignKeyViolation(Box::new(
        ForeignKeyViolationDetail {
            child_table: table_def.name.clone(),
            parent_table: fk.parent_table.clone(),
            constraint: fk.name.clone(),
            direction: FkDirection::Insert,
            key: KeyDescription::new(
                fk.columns.clone(),
                render_key_values(table_def, positions, &pair_values),
            ),
        },
    )))
}

/// Build a child FK column value tuple from `row` at `positions` (paired positionally
/// with the FK's `parent_columns`), or `None` under **MATCH SIMPLE** when any element
/// is NULL or missing — in which case the constraint is skipped for this row (the row
/// is "not yet pointing anywhere"). Shared by the write-path probe and the
/// ALTER-ADD-CONSTRAINT existing-row validation so the null-skip rule can't drift.
pub(super) fn fk_child_tuple(row: &[Value], positions: &[usize]) -> Option<Vec<Value>> {
    let mut out = Vec::with_capacity(positions.len());
    for &pos in positions {
        match row.get(pos) {
            Some(v) if v.is_null() => return None,
            Some(v) => out.push(v.clone()),
            None => return None,
        }
    }
    Some(out)
}

/// The per-row write-path enforcement — NOT NULL, row-local CHECK, then every index
/// entry the row writes ([`enforce_index_entries`]: its size refusals and its
/// unique-index conflicts, excluding `exclude_id`, the row's own id on an UPDATE / DO
/// UPDATE), then foreign-key existence — run in that fixed order against a row about
/// to be written.
///
/// One definition for the two write sites that check a row's own foreign keys as they
/// write it (the upsert `DO UPDATE` branch and `execute_copy_from`), over
/// [`enforce_row_checks`], which a plain `UPDATE` calls alone, checking its rows' keys
/// once the statement's actions have run. Per-column `validate_column`
/// (enum membership / NOT NULL) is deliberately **not** here — it varies per site
/// (INSERT validates inside its build loop; COPY / DO UPDATE validate just before
/// calling this), and INSERT's normal path stays out of this helper because its
/// CHECK runs before the ON CONFLICT probe (separated from its unique/FK pass).
#[allow(clippy::too_many_arguments)]
pub fn enforce_row_constraints<S: super::Storage>(
    storage: &S,
    schema: &Schema,
    table: &str,
    columns: &[Column],
    checks: &[(String, Expr)],
    col_names: &[&str],
    col_types: &[TypeOid],
    indexes: &[PreparedIndex],
    row: &[Value],
    exclude_id: Option<super::types::RowId>,
) -> Result<()> {
    enforce_row_checks(
        storage, schema, table, columns, checks, col_names, col_types, indexes, row, exclude_id,
    )?;
    enforce_foreign_keys(storage, schema, table, row)?;
    Ok(())
}

/// [`enforce_row_constraints`] short of the foreign keys: NOT NULL, row-local CHECK, then
/// every index entry the row writes, in that order.
///
/// # Errors
///
/// `Error::NotNullViolation`, `Error::CheckViolation`, `Error::UniqueViolation` or an
/// index-size refusal; an evaluation or storage error from a check or a probe.
#[allow(clippy::too_many_arguments)]
pub fn enforce_row_checks<S: super::Storage>(
    storage: &S,
    schema: &Schema,
    table: &str,
    columns: &[Column],
    checks: &[(String, Expr)],
    col_names: &[&str],
    col_types: &[TypeOid],
    indexes: &[PreparedIndex],
    row: &[Value],
    exclude_id: Option<super::types::RowId>,
) -> Result<()> {
    enforce_not_null_row(table, columns, col_types, row)?;
    let col_enums = table_enum_orders(columns, &schema.enums);
    enforce_checks(table, checks, col_names, col_types, &col_enums, row)?;
    enforce_index_entries(storage, indexes, row, exclude_id)
}

/// Whether a row exists in `parent_table` whose referenced columns
/// (`parent_columns`) equal `pair_values` — `pair_values[i]` pairs with
/// `parent_columns[i]` (the FK's positional local↔parent mapping already applied by
/// the caller). Single- or multi-column.
///
/// Prefers an index probe on the parent's **covering unique index** — the PK/UNIQUE
/// key whose column SET equals `parent_columns`, matched order-independently (see
/// `Schema::covering_unique_index`). Because the referenced list may be reordered
/// relative to the index's key order (`REFERENCES parent(y, x)` against `PRIMARY KEY
/// (x, y)`), the probe tuple is **permuted into the index's own key order**: for each
/// index key column, pick the `pair_values` element whose `parent_columns` entry
/// names it. Falls back to a full tuple scan when no covering index is found (index
/// dropped, or a defensive path). A NULL element never matches (callers apply MATCH
/// SIMPLE before reaching here).
///
/// # Errors
///
/// A storage error from `index_lookup` or `scan`.
pub(super) fn parent_tuple_exists<S: super::Storage>(
    storage: &S,
    schema: &Schema,
    parent_table: &str,
    parent_columns: &[String],
    pair_values: &[Value],
) -> Result<bool> {
    let Some(parent_def) = schema.get_table(parent_table) else {
        return Ok(false);
    };

    // Fast path: the covering unique index (set-match). Build the probe tuple in the
    // index's OWN key order, permuting via the FK's parent-column pairing, and coerce
    // each child value to the PARENT column's type so a cross-type reference (a child
    // `int4` → parent `int8`, or `date` → `timestamp`) probes with the parent's key
    // encoding. The child value is stored in its OWN type, whose index key never
    // matches the parent's differently-typed key, so an un-coerced probe wrongly
    // reported a valid parent absent (`23503`). A same-type FK (the common case, incl.
    // the spine's uuid keys) coerces to a no-op, so its behavior is unchanged.
    if let Some(idx) = schema.covering_unique_index(parent_table, parent_columns)
        && let Some(key_cols) = idx.key.bare_columns()
    {
        // `None` where the child value does not coerce to the parent's type, or the
        // coercion moved it (`coerced_probe_tuple`) — fall through to the scan, which
        // decides through `fk_key_matches`.
        if let Some(tuple) = coerced_probe_tuple(parent_def, parent_columns, key_cols, pair_values)
        {
            let hits = storage.index_lookup(idx.index_id(), &tuple)?;
            return Ok(!hits.is_empty());
        }
    }

    // Fallback: scan the parent, comparing the full tuple positionally.
    let Some(positions) = parent_columns
        .iter()
        .map(|pc| parent_def.columns.iter().position(|c| &c.name == pc))
        .collect::<Option<Vec<usize>>>()
    else {
        return Ok(false);
    };
    for (_, prow) in storage.scan(parent_table)? {
        // Equality must be type-correct for the FK key (commonly a UUID): `fk_key_matches`
        // reads `values_equal`, not `values_compare`, whose ordering fallback
        // returns `Equal` for any two non-numeric/non-text values and would report
        // a parent match against any row.
        if positions
            .iter()
            .zip(pair_values)
            .all(|(&pos, want)| prow.get(pos).is_some_and(|v| fk_key_matches(v, want)))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether 18.6 compares a `child` key value against a parent key of `parent_type` by
/// casting the child to that type first, rather than through a cross-type `=`.
///
/// A foreign key's equality is the parent opfamily's operator over the pair where it
/// has one, and otherwise the parent type's own `=` with the child implicitly cast.
/// The float opfamily holds `float4`/`float8` only, so an integer or `numeric` child
/// of a float parent is cast — and to a `float4` parent that cast rounds: `1.1` and
/// `16777217` reference the `real` keys `1.1` and `16777216`. Every other pair either
/// has its operator or converts without moving the value.
const fn child_is_cast_to(child: &Value, parent_type: TypeOid) -> bool {
    matches!(parent_type, TypeOid::Float4 | TypeOid::Float8)
        && !matches!(child, Value::Float4(_) | Value::Float8(_))
}

/// Whether a `child` key value references the `parent` key value, as 18.6's foreign-key
/// equality decides it: [`values_equal`](super::expr::values_equal) over the pair, the
/// child first cast to the parent's type where [`child_is_cast_to`] says 18.6 casts it.
/// A NULL on either side references nothing.
pub(super) fn fk_key_matches(parent: &Value, child: &Value) -> bool {
    use super::expr::values_equal;
    match parent.type_oid() {
        Some(parent_type) if child_is_cast_to(child, parent_type) => {
            coerce_value(child.clone(), parent_type)
                .is_ok_and(|cast| values_equal(parent, &cast) == Some(true))
        }
        _ => values_equal(parent, child) == Some(true),
    }
}

/// Build the covering-index FK fast-path probe tuple in the index's key order,
/// coercing each child value to the PARENT column's type.
///
/// The FK's set-match (`Schema::covering_unique_index`) guarantees every index key
/// column names a `parent_columns` entry, so the permutation position is always
/// found; the coercion re-encodes a cross-type child value (a child `int4` → parent
/// `int8`, `date` → `timestamp`) into the parent's own key type, which is what the
/// stored index key was built from. A same-type FK coerces to a no-op ([`coerce_value`]
/// short-circuits when the value already has the target type), so same-type FKs probe
/// exactly as before.
///
/// Returns `None` — signalling the caller to fall back to the full scan — if a key
/// column is missing from the parent def, its `pair_values` slot is absent, a child
/// value can't be coerced to the parent column's type (out of its range, or a genuine
/// type incompatibility a validly-created FK never has), or the coercion **moved** the
/// value where 18.6 compares the pair un-narrowed: a `float8` child `1.1` is not the
/// `float4` parent `1.1`, and a `timestamp` child at noon is not its day's `date`
/// parent. The scan then answers through [`fk_key_matches`], so no valid reference is
/// dropped and no rounded one is invented.
fn coerced_probe_tuple(
    parent_def: &Table,
    parent_columns: &[String],
    key_cols: &[String],
    pair_values: &[Value],
) -> Option<Vec<Value>> {
    let mut tuple = Vec::with_capacity(key_cols.len());
    for key_col in key_cols {
        let i = parent_columns.iter().position(|p| p == key_col)?;
        let target = parent_def
            .columns
            .iter()
            .find(|c| &c.name == key_col)?
            .type_oid;
        let child = pair_values.get(i)?;
        let coerced = coerce_value(child.clone(), target).ok()?;
        // Exact-or-decline, with one exception. Where 18.6 has a cross-type `=` for
        // the pair (`float8 = float4`, `timestamp = date`) it compares both sides
        // un-narrowed, so a coercion that moved the value would find a parent 18.6
        // does not match — decline, and the scan's `fk_key_matches` decides. The
        // exception is the pair with no such operator, which 18.6 compares by casting
        // the child to the parent's type: a non-float child of a float parent.
        if !child_is_cast_to(child, target)
            && super::expr::values_equal(&coerced, child) != Some(true)
        {
            return None;
        }
        tuple.push(coerced);
    }
    Some(tuple)
}

#[cfg(test)]
mod tests {
    use super::{coerced_probe_tuple, single_referenced_column};
    use crate::catalog::{Column, Table};
    use crate::planner::{BinaryOp, Expr, Literal};
    use crate::types::{TypeOid, Value};

    fn col(name: &str, type_oid: TypeOid) -> Column {
        Column {
            name: name.to_string(),
            type_oid,
            nullable: true,
            position: 0,
            default: None,
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            temporal_precision: None,
            identity: None,
        }
    }

    fn one_col_table(name: &str, col_name: &str, type_oid: TypeOid) -> Table {
        Table {
            name: name.to_string(),
            schema: "public".to_string(),
            columns: vec![col(col_name, type_oid)],
            oid: 0,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
        }
    }

    fn probe(parent_type: TypeOid, child: Value) -> Option<Vec<Value>> {
        let parent = one_col_table("p", "id", parent_type);
        let cols = vec!["id".to_string()];
        coerced_probe_tuple(&parent, &cols, &cols, &[child])
    }

    #[test]
    fn cross_type_fk_probe_coerces_child_to_parent_type() {
        // A child int4/int2 → parent int8/int4 FK: the child value is re-encoded in the
        // parent's key type, so the covering-index probe matches the parent's stored key
        // (previously an un-coerced probe reported the valid parent absent → wrong
        // 23503).
        assert_eq!(
            probe(TypeOid::Int8, Value::Int4(5)),
            Some(vec![Value::Int8(5)])
        );
        assert_eq!(
            probe(TypeOid::Int4, Value::Int2(7)),
            Some(vec![Value::Int4(7)])
        );
        // date → timestamp coerces to midnight UTC (matches PG's date2timestamp).
        let d = crate::types::parse_date("2024-01-01").unwrap();
        assert_eq!(
            probe(TypeOid::Timestamp, Value::Date(d)),
            Some(vec![Value::Timestamp(d.map(|d| d.midnight().assume_utc()))]),
        );
    }

    /// The probe is exact-or-decline: a coercion that moved the child's value hands
    /// the verdict to the scan, except for the non-float child of a float parent,
    /// which 18.6 itself casts.
    #[test]
    fn narrowing_fk_probe_declines_unless_the_cast_is_the_comparison() {
        // `float8 = float4` compares both as `float8`: 1.1 is not the `real` 1.1.
        assert_eq!(probe(TypeOid::Float4, Value::Float8(1.1)), None);
        assert_eq!(probe(TypeOid::Float4, Value::Float8(1e300)), None);
        // ...while a value the narrow width holds exactly probes, as its own type.
        assert_eq!(
            probe(TypeOid::Float4, Value::Float8(1.5)),
            Some(vec![Value::Float4(1.5)])
        );
        // `timestamp = date` compares as timestamps: noon is not its day.
        let noon = crate::types::parse_timestamp("2024-01-01 12:00:00").unwrap();
        assert_eq!(probe(TypeOid::Date, Value::Timestamp(noon)), None);
        // A `numeric` or integer child of a float parent is cast, and the cast rounds.
        let n = crate::types::PgNumeric::parse("1.1").unwrap();
        assert_eq!(
            probe(TypeOid::Float4, Value::Numeric(n)),
            Some(vec![Value::Float4(1.1)])
        );
        assert_eq!(
            probe(TypeOid::Float4, Value::Int8(16_777_217)),
            Some(vec![Value::Float4(16_777_216.0)])
        );
        // The pairs that convert without moving the value still probe.
        assert_eq!(
            probe(TypeOid::Varchar, Value::Text("a".into())).map(|t| t.len()),
            Some(1)
        );
    }

    /// The scan's equality is the probe's: a cast child matches the parent it rounds
    /// to, and a cross-width float pair is compared un-narrowed.
    #[test]
    fn fk_key_matches_casts_a_non_float_child_to_a_float_parent() {
        use super::fk_key_matches;
        let n = crate::types::PgNumeric::parse("1.1").unwrap();
        assert!(fk_key_matches(&Value::Float4(1.1), &Value::Numeric(n)));
        assert!(fk_key_matches(
            &Value::Float4(16_777_216.0),
            &Value::Int8(16_777_217)
        ));
        assert!(!fk_key_matches(&Value::Float4(1.1), &Value::Float8(1.1)));
        assert!(fk_key_matches(
            &Value::Float8(f64::NAN),
            &Value::Float4(f32::NAN)
        ));
        assert!(fk_key_matches(&Value::Float8(0.0), &Value::Float8(-0.0)));
        assert!(!fk_key_matches(&Value::Float4(1.5), &Value::Null));
        assert!(fk_key_matches(&Value::Int8(5), &Value::Int4(5)));
    }

    #[test]
    fn same_type_fk_probe_is_a_no_op() {
        // The common case (incl. the spine's uuid keys): a same-type value is returned
        // unchanged, so same-type FK enforcement is byte-identical to before.
        assert_eq!(
            probe(TypeOid::Int8, Value::Int8(5)),
            Some(vec![Value::Int8(5)])
        );
        let u = [0u8; 16];
        assert_eq!(
            probe(TypeOid::Uuid, Value::Uuid(u)),
            Some(vec![Value::Uuid(u)])
        );
    }

    #[test]
    fn incompatible_probe_falls_back_to_scan() {
        // A genuinely incompatible value (a valid FK never produces one) yields `None`,
        // signalling the caller to use the scan fallback rather than erroring.
        assert_eq!(probe(TypeOid::Uuid, Value::Bool(true)), None);
    }

    fn bin(op: BinaryOp, l: Expr, r: Expr) -> Expr {
        Expr::BinaryOp {
            left: Box::new(l),
            op,
            right: Box::new(r),
        }
    }

    fn column(name: &str) -> Expr {
        Expr::Column {
            table: None,
            name: name.to_string(),
        }
    }

    #[test]
    fn check_auto_name_single_column_uses_column() {
        // A single-column predicate (even referencing the column twice) names
        // `<table>_<col>_check`; a multi-column predicate names `<table>_check`.
        let single = bin(BinaryOp::Gt, column("val"), Expr::Literal(Literal::Int(0)));
        assert_eq!(single_referenced_column(&single), Some("val".to_string()));

        let repeated = bin(
            BinaryOp::And,
            bin(BinaryOp::Gt, column("val"), Expr::Literal(Literal::Int(0))),
            bin(
                BinaryOp::Lt,
                column("val"),
                Expr::Literal(Literal::Int(100)),
            ),
        );
        assert_eq!(single_referenced_column(&repeated), Some("val".to_string()));

        let multi = bin(BinaryOp::Lt, column("a"), column("b"));
        assert_eq!(single_referenced_column(&multi), None);
    }
}
