//! Index selection for the planner.
//!
//! Given a WHERE predicate and the schema, decide which single-column index (if
//! any) can serve the query and compute its bounds/keys. These are the pure
//! "predicate + schema -> chosen index / bounds / keys" helpers; the orchestration
//! that wires the chosen probe into a `Plan` lives in `query.rs`.

use crate::catalog::Schema;
use crate::executor::{IndexMethod, index_method_for};

use super::expr::{BinaryOp, Expr};
use super::plan::IndexBound;

/// Find an `indexed_col = <row-independent value>` equality in `predicate` that a
/// single-column index covers, returning `(index_oid, key_column, key_value,
/// column_first, residual)`.
///
/// `column_first` records whether the equality was written `col = v` (rather than the
/// flipped `v = col`) — diagnostics only, so an operand-type rejection on the probe
/// reports the operands in the order the user wrote them.
///
/// `predicate` is flattened over top-level `AND`s; the first conjunct that is an
/// eligible equality is lifted out and the remaining conjuncts (if any) are
/// re-AND'd into `residual` (in original order). `None` if no conjunct qualifies.
/// `alias` is the FROM alias so a qualified `c.col = …` over `FROM t c` resolves.
pub fn find_index_equality(
    predicate: &Expr,
    table: &str,
    alias: Option<&str>,
    schema: &Schema,
) -> Option<(i32, String, Expr, bool, Option<Expr>)> {
    let conjuncts = flatten_and(predicate);
    for (i, conj) in conjuncts.iter().enumerate() {
        if let Some((col, value, column_first)) = eq_column_and_value(conj, table, alias)
            && let Some(index) = single_column_index_for(schema, table, &col)
        {
            // Re-AND the other conjuncts as the residual filter.
            let residual = and_of(
                conjuncts
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, e)| (*e).clone()),
            );
            return Some((index.oid, col, value.clone(), column_first, residual));
        }
    }
    None
}

/// The detected single-column range probe: `(index_oid, key_column, lower, upper,
/// residual)` — the index OID, the bounded column, its optional lower/upper bounds,
/// and the residual conjuncts to re-apply in a wrapping `Filter`.
type RangeProbe = (
    i32,
    String,
    Option<IndexBound>,
    Option<IndexBound>,
    Option<Expr>,
);

/// Find range comparisons in `predicate` over a single indexed column and merge
/// them into `(index_oid, key_column, lower, upper, residual)`.
///
/// `predicate` is flattened over top-level `AND`s. The detector picks the first
/// indexed column carrying a range conjunct (`col {>,>=,<,<=} <row-independent
/// value>`, either operand order — `v < col` flips to `col > v`), then merges
/// **every** range conjunct on that same column into one bound set: the tightest
/// `>`/`>=` becomes `lower`, the tightest `<`/`<=` becomes `upper`. Conjuncts on
/// other columns, non-range conjuncts, and any range conjunct on the chosen
/// column that couldn't be merged (a redundant looser bound) are re-AND'd into
/// `residual` — so the node returns a superset and the wrapping `Filter` refines.
/// `None` if no conjunct is an eligible range on a single-column full index.
pub fn find_index_range(
    predicate: &Expr,
    table: &str,
    alias: Option<&str>,
    schema: &Schema,
) -> Option<RangeProbe> {
    let conjuncts = flatten_and(predicate);

    // The first indexed column that has at least one range conjunct.
    let (key_column, index) = conjuncts.iter().find_map(|conj| {
        let (col, _, _) = range_column_and_value(conj, table, alias)?;
        let index = single_column_index_for(schema, table, &col)?;
        Some((col, index))
    })?;
    let index_oid = index.oid;

    // Merge every range conjunct on the chosen column; everything else is residual.
    let mut lower: Option<IndexBound> = None;
    let mut upper: Option<IndexBound> = None;
    let mut residual_exprs: Vec<Expr> = Vec::new();
    for conj in &conjuncts {
        match range_column_and_value(conj, table, alias) {
            // A range on the chosen column: fold into lower/upper, keeping the
            // tighter bound. A bound that loses the tighten (a redundant looser one)
            // falls through to the residual filter — still correct, just less
            // selective.
            Some((col, is_lower, bound)) if col == key_column => {
                let slot = if is_lower { &mut lower } else { &mut upper };
                if tighten(slot, &bound, is_lower) {
                    // bound now owns the slot; nothing to residualize.
                } else {
                    residual_exprs.push((*conj).clone());
                }
            }
            // A range on a different column, or a non-range conjunct: residual.
            _ => residual_exprs.push((*conj).clone()),
        }
    }

    let residual = and_of(residual_exprs.into_iter());
    Some((index_oid, key_column, lower, upper, residual))
}

/// Replace `slot` with `bound` if `bound` is tighter (or `slot` is empty),
/// returning whether `bound` was taken. For a lower bound, tighter means a larger
/// value (or equal value with the same/narrower inclusivity); for an upper bound,
/// a smaller value. When the values are incomparable (different types — shouldn't
/// happen for a single typed column, but be safe) the incumbent is kept.
fn tighten(slot: &mut Option<IndexBound>, bound: &IndexBound, is_lower: bool) -> bool {
    let take = slot.as_ref().is_none_or(|existing| {
        match compare_bound_values(&bound.value, &existing.value) {
            Some(std::cmp::Ordering::Greater) => is_lower,
            Some(std::cmp::Ordering::Less) => !is_lower,
            // Equal value: prefer the exclusive bound (narrower window).
            Some(std::cmp::Ordering::Equal) => !bound.inclusive && existing.inclusive,
            None => false,
        }
    });
    if take {
        *slot = Some(bound.clone());
    }
    take
}

/// Compare two row-independent bound expressions by their literal value, for
/// tighten-merging. Only `Literal` integer/float/string values are ordered here
/// (the common BETWEEN/`AND` shape); a parameter, bound, or mixed-type pair is
/// incomparable (`None`) and the merge keeps the incumbent, residualizing the
/// rest — correctness is preserved either way.
fn compare_bound_values(a: &Expr, b: &Expr) -> Option<std::cmp::Ordering> {
    use super::expr::Literal;
    match (a, b) {
        (Expr::Literal(Literal::Int(x)), Expr::Literal(Literal::Int(y))) => Some(x.cmp(y)),
        (Expr::Literal(Literal::Float(x)), Expr::Literal(Literal::Float(y))) => x.partial_cmp(y),
        (Expr::Literal(Literal::String(x)), Expr::Literal(Literal::String(y))) => Some(x.cmp(y)),
        _ => None,
    }
}

/// If `expr` is a range comparison `col {>,>=,<,<=} <row-independent value>`
/// (either operand order) on this table's column, return `(column, is_lower,
/// (value, inclusive))`. `is_lower` is true when the value bounds the column from
/// below (`col > v`, `col >= v`, or the flipped `v < col`, `v <= col`).
///
/// The column may be unqualified or qualified by the table's name/alias; the value
/// must be row-independent (literal / `$N` / bound). Anything else → `None`.
fn range_column_and_value(
    expr: &Expr,
    table: &str,
    alias: Option<&str>,
) -> Option<(String, bool, IndexBound)> {
    let Expr::BinaryOp { left, op, right } = expr else {
        return None;
    };
    if !matches!(
        op,
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    ) {
        return None;
    }
    let inclusive = matches!(op, BinaryOp::Le | BinaryOp::Ge);
    // `col OP value` — bound on `col`.
    if let Some(col) = column_name_for(left, table, alias)
        && is_row_independent(right)
    {
        // col > / >= value → lower bound; col < / <= value → upper bound.
        let is_lower = matches!(op, BinaryOp::Gt | BinaryOp::Ge);
        return Some((
            col,
            is_lower,
            IndexBound {
                value: right.clone(),
                inclusive,
                column_first: true,
            },
        ));
    }
    // `value OP col` — flip to `col OP' value`.
    if let Some(col) = column_name_for(right, table, alias)
        && is_row_independent(left)
    {
        // value < col ≡ col > value (lower); value > col ≡ col < value (upper).
        let is_lower = matches!(op, BinaryOp::Lt | BinaryOp::Le);
        return Some((
            col,
            is_lower,
            IndexBound {
                value: left.clone(),
                inclusive,
                column_first: false,
            },
        ));
    }
    None
}

/// Flatten a left-deep `AND` tree into its conjunct list (a non-`AND` expr is a
/// single-element list). `OR` is opaque — never split.
pub fn flatten_and(expr: &Expr) -> Vec<&Expr> {
    flatten_binop(expr, BinaryOp::And)
}

/// Flatten a left-deep `OR` tree into its disjunct list (a non-`OR` expr is a
/// single-element list). `AND` is opaque — never split.
fn flatten_or(expr: &Expr) -> Vec<&Expr> {
    flatten_binop(expr, BinaryOp::Or)
}

/// Flatten a left-deep tree of one associative operator (`And` or `Or`) into its
/// operand list; any expr that isn't that operator at the top is a single element.
fn flatten_binop(expr: &Expr, op: BinaryOp) -> Vec<&Expr> {
    let mut out = Vec::new();
    collect_binop(expr, op, &mut out);
    out
}

/// Append `expr`'s top-level `op` operands to `out` (recursing through `op` nodes —
/// the plain ones and a long list's [`BinaryOp::AndList`] / [`BinaryOp::OrList`]
/// nodes — only); any other expression appends itself.
fn collect_binop<'a>(expr: &'a Expr, op: BinaryOp, out: &mut Vec<&'a Expr>) {
    if let Expr::BinaryOp {
        left,
        op: node_op,
        right,
    } = expr
        && node_op.connective() == op
    {
        collect_binop(left, op, out);
        collect_binop(right, op, out);
        return;
    }
    out.push(expr);
}

/// Re-combine conjuncts into one `AND` list ([`Expr::bool_list`]), or `None` if empty.
fn and_of(iter: impl Iterator<Item = Expr>) -> Option<Expr> {
    let operands: Vec<Expr> = iter.collect();
    (!operands.is_empty()).then(|| Expr::bool_list(BinaryOp::And, operands))
}

/// If `expr` is `col = <row-independent value>` (either operand order), return
/// the (bare column name, value, column_first) — where the column is unqualified or
/// qualified by `table`/`alias`, and the value is row-independent (a literal, a `$N`
/// parameter, or an already-bound value; crucially NOT another column). Else
/// `None`. `column_first` is true for the `col = v` spelling, false for `v = col`.
fn eq_column_and_value<'a>(
    expr: &'a Expr,
    table: &str,
    alias: Option<&str>,
) -> Option<(String, &'a Expr, bool)> {
    let Expr::BinaryOp {
        left,
        op: BinaryOp::Eq,
        right,
    } = expr
    else {
        return None;
    };
    if let Some(col) = column_name_for(left, table, alias)
        && is_row_independent(right)
    {
        return Some((col, right, true));
    }
    if let Some(col) = column_name_for(right, table, alias)
        && is_row_independent(left)
    {
        return Some((col, left, false));
    }
    None
}

/// The bare column name if `expr` is a column reference belonging to this table —
/// unqualified, or qualified by the table's real name or FROM alias. `None` for a
/// wildcard, a foreign-qualified column, or any non-column expression.
fn column_name_for(expr: &Expr, table: &str, alias: Option<&str>) -> Option<String> {
    let Expr::Column { table: qual, name } = expr else {
        return None;
    };
    if name == "*" {
        return None;
    }
    match qual.as_deref() {
        None => Some(name.clone()),
        Some(q) if q == table || alias == Some(q) => Some(name.clone()),
        Some(_) => None,
    }
}

/// Whether the `residual` an equality or range probe leaves of a simple `UPDATE` /
/// `DELETE`'s `WHERE` may be evaluated over the probe's rows alone without moving the
/// statement's answer — the admission rule of `planner::query`'s `dml_target_probe`.
///
/// The scan a probe replaces evaluates the residual over every row of the table; the
/// probe path, over its candidates only. So a conjunct whose evaluation can **fail** is
/// the hazard. Where it fails whatever the row — a refusal pglet raises only at
/// evaluation, such as a `DEFAULT`, a missing relation inside a subquery, an aggregate,
/// or a constant 18.6 folds while planning (`1/0`) — the scan raises it at the first row
/// it reaches and the probe at none (`docs/indexes.md` § The starved-leaf exception), so a
/// write 18.6 refuses would report success. And where it fails for some rows only
/// (`10 / x > 0`), a probe that leaves those rows out answers where the scan raises —
/// 18.6's own answer there, but a build-profile divergence, which the scan path does not
/// have. The rule is a whitelist of conjuncts whose evaluation cannot fail once planning
/// has accepted the statement:
///
/// - a bare column of the target — a `boolean` one, the clause-root gate having proved
///   the type;
/// - a column's `IS NULL` / `IS NOT NULL`;
/// - a column compared (`=`, `<>`, `<`, `<=`, `>`, `>=`, `IS [NOT] DISTINCT FROM`) with a
///   key [`is_row_independent`] admits. A literal key's operator, and an enum label
///   literal's membership, were proved at plan time (an enum-typed parameter's
///   membership is proved at bind); a parameter's operator is asked at execution, of its
///   bound value, by `executor::query`'s `probe_dml_targets`, which declines where it
///   fails.
///
/// Every other conjunct declines the probe.
pub fn dml_residual_is_certain(
    residual: &Expr,
    table: &str,
    alias: Option<&str>,
    schema: &Schema,
) -> bool {
    let Some(table_def) = schema.get_table(table) else {
        return false;
    };
    let target_column = |e: &Expr| {
        column_name_for(e, table, alias)
            .is_some_and(|name| table_def.columns.iter().any(|c| c.name == name))
    };
    flatten_and(residual)
        .into_iter()
        .all(|conjunct| match conjunct {
            Expr::Column { .. } => target_column(conjunct),
            Expr::IsNull(operand) | Expr::IsNotNull(operand) => target_column(operand),
            Expr::BinaryOp { left, op, right }
                if super::legality::comparison_symbol(*op).is_some() =>
            {
                (target_column(left) && is_row_independent(right))
                    || (is_row_independent(left) && target_column(right))
            }
            _ => false,
        })
}

/// Whether `expr` evaluates independently of any row — a literal, a `$N`
/// parameter, a `Cast` directly over a `$N`, or an already-bound value. A column
/// reference (or anything containing one) is row-dependent and not a valid index
/// probe key.
///
/// The `Cast` over a `$N` is the shape a parameter whose type the client declared at
/// `Parse` takes (`parser::parse_single_with_param_types`), as well as a written
/// `$N::<type>`. Declining it cost a type-declaring client its point and range index
/// reads: a
/// point read over a 100 000-row primary key measured about 0.05 ms with the
/// parameter undeclared and about 19 ms declared `int4` once the declared type became
/// a `Cast`. The probe value is the cast's result, and `executor::query::scan`'s
/// `probe_key_value` gates and exact-or-declines it against the key column as it does
/// a bare parameter's.
fn is_row_independent(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Parameter { .. } | Expr::Bound(_) => true,
        Expr::Cast { expr: inner, .. } => matches!(**inner, Expr::Parameter { .. }),
        _ => false,
    }
}

/// The first single-column **full B-tree** index over `(table, column)`, if any.
///
/// The four gates are independent and each excludes a different failure:
///
/// - **Method** — the index must actually *be* a B-tree
///   (`executor::index_method_for`, the same derivation that builds the storage
///   `IndexSpec`, so selection cannot disagree with what storage maintains). A GIN
///   index stores one **decomposed** entry per element / object key, not one per row,
///   and answers containment only; it can serve neither an equality point-read, a
///   range probe, nor an ordered scan. Offering one to any of the three is a silent
///   wrong answer, not a slow one: `SELECT id FROM t ORDER BY jsonb_col` over a
///   `USING gin` index returned each row **once per decomposed key**, in key order
///   rather than row order (and `LIMIT 1` then answered a single, entirely
///   ordinary-looking, wrong row), while `WHERE jsonb_col = $1` selected the GIN
///   index and probed its decomposed entries with a whole-value key, where Postgres
///   answers rows. All of it from DDL both engines accept, which is why
///   `gin_index_for` is the *only* door a GIN index may enter by. Declining here is a
///   **fall-through**, not a stop: `find_index_equality` / `find_index_range` keep
///   walking the conjuncts, so `WHERE j = $1 AND a = 1` over a GIN `j` and a B-tree
///   `a` probes `a` (measured byte-identical to Postgres for the equality, range, and
///   `TEXT[]` spellings).
/// - **Partial** (`predicate.is_some()`) — such an index only contains rows that
///   satisfy its `WHERE` predicate, so an `IndexScan` point-read over it would
///   silently drop the rows that don't (the predicate is never re-applied as a
///   residual filter). It can't serve an unconditional equality probe.
/// - **Single bare column** — composite and expression indexes are excluded by
///   `single_bare_column` (it only answers for a single-column `IndexKey::Columns`).
/// - **Not an enum column** — an enum orders by its members' **positions**
///   (`executor::expr`'s `values_compare_ordered`), and the B-tree key encoder has no
///   enum arm at all: it encodes the label's bytes, as it does any `Value::Text`. So an
///   index over an enum column is ordered by something the engine no longer orders by,
///   and every read it serves is a different answer from the row filter beside it — an
///   *index-presence* divergence on one binary, and a **build-profile** divergence
///   besides, since the stripped planner declines every index and row-filters. Measured
///   over `mood AS ENUM ('sad','ok','happy')`: `ORDER BY m` answered `happy, ok, sad`
///   from the index against the unindexed table's `sad, ok, happy`, and `WHERE m >
///   'sad'` answered all three rows against the correct two. Declining is a
///   **performance** cost only: the row filter is exact, and label-byte *equality* is
///   still enum equality (18.6's `enum_eq` is bare OID identity), so nothing an index
///   could have answered is lost. Closing it the other way — encoding the member
///   position in the key — is a persisted-shape change (`docs/enums.md`).
///
/// Note what the method gate deliberately does **not** exclude: a *unique* index over
/// a container column, which `index_method_for` reports as a B-tree because that is
/// what storage builds for it (GIN cannot enforce uniqueness). It is keyed in the
/// comparator's order (`docs/indexes.md` § A container key), so the equality probe, the
/// range and the ordered scan this selector offers it to answer as the row filter does:
/// `CREATE TABLE fx (id INT, j JSONB NOT NULL UNIQUE); SELECT * FROM fx WHERE j = $1`,
/// its range spelling `j > $1`, the same over a `TEXT[]` column and `PRIMARY KEY` in
/// place of `UNIQUE` agree on both build profiles and with 18.6
/// (`tests/fixtures/container_key/`).
pub fn single_column_index_for<'a>(
    schema: &'a Schema,
    table: &str,
    column: &str,
) -> Option<&'a crate::catalog::Index> {
    let table_def = schema.get_table(table)?;
    // The enum gate is on the *column*, not on any one index, so it is asked once here
    // rather than inside the per-index filter.
    if table_def
        .columns
        .iter()
        .any(|c| c.name == column && c.enum_type.is_some())
    {
        return None;
    }
    schema.indexes.values().find(|ix| {
        ix.table_name == table
            && ix.key.single_bare_column() == Some(column)
            && ix.predicate.is_none()
            && index_method_for(ix, table_def) == IndexMethod::Btree
    })
}

/// Find a top-level containment conjunct a GIN index can accelerate, returning
/// `(index_oid, key_column, probes, guards)`.
///
/// `predicate` is flattened over top-level `AND`s; the first conjunct that is an
/// eligible containment (or an `OR` of them) is lifted. Unlike equality/range, the
/// conjunct stays in the caller's recheck `Filter` (GIN is lossy), so only the
/// probe/guard exprs are returned. Two shapes qualify:
///
/// - **Simple** — a `col @> <scan-independent expr>` conjunct → `probes = [expr]`,
///   `guards = []`.
/// - **OR-union** — a conjunct that is an `OR` whose every disjunct is either a
///   `col @> <scan-independent expr>` containment on the **same** GIN column (→ a
///   `probes` entry, OR-unioned) or a scan-independent boolean guard (→ a `guards`
///   entry). This is the spine's multi-key audit (`metadata @> a OR metadata @> b`)
///   and its param-null guard (`$p IS NULL OR refs @> ARRAY[$p]`).
///
/// Eligibility for a containment (each disjunct, or the simple conjunct):
/// - the left side is this scan's `col` (qualified by table/alias or bare),
/// - the right side is **independent of this scan's rows** — a literal, `$N`, or a
///   correlated outer-scope column, but never one of this scan's own columns (so the
///   probe is evaluable once, before scanning),
/// - `col` has a single-column GIN index (`gin_index_for`),
/// - and the query **implies** that index's partial predicate
///   (`query_implies_index_predicate`), checked against the top-level `AND` conjuncts.
pub fn find_index_containment(
    predicate: &Expr,
    table: &str,
    alias: Option<&str>,
    schema: &Schema,
) -> Option<(i32, String, Vec<Expr>, Vec<Expr>)> {
    let conjuncts = flatten_and(predicate);
    for conj in &conjuncts {
        // Simple case: a single containment conjunct on a GIN column.
        if let Some((index, col, probe)) = gin_containment_probe(conj, table, alias, schema) {
            if query_implies_index_predicate(index, &col, &conjuncts) {
                return Some((index.oid, col, vec![probe], Vec::new()));
            }
            // A containment that fails the partial-index gate — try the next conjunct.
            continue;
        }
        // OR-union case: an `OR` of same-column probes (+ scan-independent guards).
        if matches!(conj, Expr::BinaryOp { op, .. } if op.connective() == BinaryOp::Or)
            && let Some(found) =
                or_containment_union(&flatten_or(conj), table, alias, schema, &conjuncts)
        {
            return Some(found);
        }
    }
    None
}

/// If `expr` is `col @> <scan-independent>` on a single-column GIN index, return
/// `(gin_index, col, probe)` — the shared eligibility check for both the simple
/// top-level conjunct and each disjunct of an OR-union.
///
/// The probe (the `@>` RHS) must be independent of this scan's rows (a literal,
/// `$N`, or a correlated outer-scope column — never one of this scan's own columns),
/// so it is evaluable once before scanning. The caller still gates on the index's
/// (partial) predicate via `query_implies_index_predicate`.
fn gin_containment_probe<'a>(
    expr: &Expr,
    table: &str,
    alias: Option<&str>,
    schema: &'a Schema,
) -> Option<(&'a crate::catalog::Index, String, Expr)> {
    let Expr::BinaryOp {
        left,
        op: BinaryOp::Containment,
        right,
    } = expr
    else {
        return None;
    };
    let col = column_name_for(left, table, alias)?;
    if !expr_is_scan_independent(right, table, alias) {
        return None;
    }
    let index = gin_index_for(schema, table, &col)?;
    Some((index, col, (**right).clone()))
}

/// Try to route an `OR` conjunct (its flattened `disjuncts`) to a GIN union,
/// returning `(index_oid, key_column, probes, guards)`.
///
/// Qualifies when every disjunct is either a `col @> <scan-independent>` containment
/// on a single shared GIN column (→ a probe) or a scan-independent boolean guard
/// (→ a guard, e.g. `$p IS NULL`), at least one probe exists, and the query implies
/// the (partial) index's predicate. `None` (fall back to a scan) if any disjunct is
/// scan-dependent and not such a probe, or the probes span more than one column.
///
/// `query_conjuncts` is the enclosing WHERE's top-level `AND` conjuncts, used for the
/// partial-index implication check (the `OR` itself is one of them).
fn or_containment_union(
    disjuncts: &[&Expr],
    table: &str,
    alias: Option<&str>,
    schema: &Schema,
    query_conjuncts: &[&Expr],
) -> Option<(i32, String, Vec<Expr>, Vec<Expr>)> {
    let mut gin_col: Option<String> = None;
    let mut probes: Vec<Expr> = Vec::new();
    let mut guards: Vec<Expr> = Vec::new();
    for disj in disjuncts {
        // A same-column `col @> <scan-independent>` containment on a GIN column → a probe.
        if let Some((_, col, probe)) = gin_containment_probe(disj, table, alias, schema) {
            // Every probe must share one GIN column (one posting-list keyspace).
            if let Some(existing) = &gin_col {
                if existing != &col {
                    return None;
                }
            } else {
                gin_col = Some(col);
            }
            probes.push(probe);
            continue;
        }
        // Else it must be a scan-independent guard (evaluable once, no scan column).
        if expr_is_scan_independent(disj, table, alias) {
            guards.push((**disj).clone());
            continue;
        }
        // A scan-dependent, non-probe disjunct can't be GIN-accelerated.
        return None;
    }
    let col = gin_col?;
    let index = gin_index_for(schema, table, &col)?;
    if !query_implies_index_predicate(index, &col, query_conjuncts) {
        return None;
    }
    Some((index.oid, col, probes, guards))
}

/// The single-column **GIN** index over `(table, column)`, if any.
///
/// The exact complement of [`single_column_index_for`]'s method gate, and asked of the
/// same `executor::index_method_for` rather than re-derived here, so neither can drift
/// from the other or from the `IndexSpec` storage builds. The complement is of that
/// **gate**, not of the selector: [`single_column_index_for`] additionally excludes a
/// partial B-tree index on `predicate.is_none()`, while this one offers a partial GIN
/// index and leaves its predicate to `query_implies_index_predicate` on the selection
/// path. (A partial B-tree serves no unconditional probe; a partial GIN is safe only
/// because that implication gate runs — the containment recheck below is a separate
/// job, and cannot recover a row a partial index never held.)
///
/// Partial GIN indexes are **included** (unlike `single_column_index_for`): the
/// containment recheck `Filter` re-applies the full predicate, and
/// `find_index_containment` separately gates that the query implies the partial
/// predicate, so a partial GIN never drops a wanted row.
fn gin_index_for<'a>(
    schema: &'a Schema,
    table: &str,
    column: &str,
) -> Option<&'a crate::catalog::Index> {
    let table_def = schema.get_table(table)?;
    schema.indexes.values().find(|ix| {
        ix.table_name == table
            && ix.key.single_bare_column() == Some(column)
            && index_method_for(ix, table_def) == IndexMethod::Gin
    })
}

/// Whether the query's conjuncts **imply** a (partial) GIN index's predicate, so the
/// index is safe to use (it can't be missing a row the query wants).
///
/// A full index (no predicate) always qualifies. For a partial index, every
/// predicate conjunct must be an `IS [NOT] NULL` check on a column that the query
/// also asserts — with one extra rule: `<gin_col> IS NOT NULL` is **auto-implied** by
/// the `@>` probe on `gin_col` (a NULL array/JSONB contains nothing). Any other
/// predicate shape (or an unimplied check) fails the gate, conservatively falling
/// back to a scan. This is scoped to the spine's partial GIN predicates
/// (`refs IS NOT NULL AND deleted_at IS NULL`, `deleted_at IS NULL`).
fn query_implies_index_predicate(
    index: &crate::catalog::Index,
    gin_col: &str,
    query_conjuncts: &[&Expr],
) -> bool {
    let Some(pred_expr) = &index.predicate else {
        return true;
    };
    flatten_and(pred_expr).iter().all(|pc| {
        let Some((is_not_null, col)) = null_check_column(pc) else {
            return false;
        };
        // `gin_col IS NOT NULL` is auto-implied by the `@>` probe on gin_col.
        if is_not_null && col == gin_col {
            return true;
        }
        // Else the query must carry the same IS [NOT] NULL check (any qualifier).
        query_conjuncts
            .iter()
            .any(|qc| null_check_column(qc) == Some((is_not_null, col)))
    })
}

/// Extract `(is_not_null, column_name)` from an `IS [NOT] NULL` check on a bare
/// column, ignoring any table qualifier. `None` for any other shape.
fn null_check_column(expr: &Expr) -> Option<(bool, &str)> {
    let (is_not_null, inner) = match expr {
        Expr::IsNull(inner) => (false, inner),
        Expr::IsNotNull(inner) => (true, inner),
        _ => return None,
    };
    match inner.as_ref() {
        Expr::Column { name, .. } if name != "*" => Some((is_not_null, name.as_str())),
        _ => None,
    }
}

/// Whether `expr` is independent of `table`/`alias`'s own rows — it references no
/// column of this scan and calls nothing volatile, so it can be evaluated **once**
/// (before scanning) against params and any outer scope.
///
/// A volatile call (`Expr::is_volatile_call` — `random()`, `gen_random_uuid()`,
/// `clock_timestamp()`, …) is dependent wherever it stands: 18.6 evaluates it once a row,
/// and a guard or probe holding one would answer once for the whole scan.
///
/// A `Column` qualified by this scan's table/alias, or **unqualified** (it might
/// bind here), is scan-dependent. A column qualified by a *different* alias is an
/// outer-scope reference and is independent. Literals/params/bound values are
/// independent; the structural variants recurse. Plan-bearing or aggregate variants
/// are treated as dependent (conservative — they never form a containment probe).
fn expr_is_scan_independent(expr: &Expr, table: &str, alias: Option<&str>) -> bool {
    // A volatile call answers afresh at each evaluation: evaluated once ahead of the
    // scan it would answer once where the `WHERE` answers once a row.
    if expr.is_volatile_call() {
        return false;
    }
    let indep = |e: &Expr| expr_is_scan_independent(e, table, alias);
    match expr {
        Expr::Literal(_)
        | Expr::Parameter { .. }
        | Expr::Bound(_)
        | Expr::Builtin(_)
        | Expr::Default => true,
        // Bare/this-scan-qualified column → dependent; a different qualifier (outer
        // scope) → independent. `column_name_for` returns `Some` exactly for the
        // dependent case.
        Expr::Column { .. } => column_name_for(expr, table, alias).is_none(),
        Expr::Negate(e)
        | Expr::UnaryPlus(e)
        | Expr::Not(e)
        | Expr::IsNull(e)
        | Expr::IsNotNull(e)
        | Expr::Cast { expr: e, .. }
        | Expr::CastTypmod { expr: e, .. }
        | Expr::Alias { expr: e, .. }
        | Expr::Unnest(e)
        | Expr::BooleanTest { expr: e, .. }
        | Expr::Lower(e) => indep(e),
        Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } => {
            indep(left) && indep(right)
        }
        Expr::Extract { source, field, .. } => indep(source) && indep(field),
        Expr::Array(items)
        | Expr::JsonbBuildObject(items)
        | Expr::Coalesce(items)
        | Expr::MathFn { args: items, .. }
        | Expr::StringFn { args: items, .. }
        | Expr::VariadicFn { args: items, .. } => items.iter().all(indep),
        Expr::InList { expr, list, .. } => indep(expr) && list.iter().all(indep),
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            operand.as_deref().is_none_or(indep)
                && when_clauses
                    .iter()
                    .all(|(condition, result)| indep(condition) && indep(result))
                && else_result.as_deref().is_none_or(indep)
        }
        // Plan-bearing, aggregate, window, or sequence exprs never form a
        // containment probe (a sequence function only ever appears at the statement
        // level, never in an indexable predicate).
        Expr::Subquery { .. }
        | Expr::InSubquery { .. }
        | Expr::Exists { .. }
        | Expr::Aggregate { .. }
        | Expr::Window { .. }
        | Expr::SequenceFn { .. } => false,
    }
}
