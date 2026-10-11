//! The one enumeration of a catalog's constraints, and the one renderer of a
//! constraint's SQL definition.
//!
//! Two surfaces read this module and **must not** disagree, which is why they read one
//! function rather than two walks: `pg_catalog.pg_constraint`'s rows (see
//! [`PgCatalog::pg_constraint`](super::PgCatalog::pg_constraint)) and the
//! `pg_get_constraintdef(oid)` function the fuz spine's schema snapshot calls on every
//! row it gets back. `pg_get_constraintdef` resolves its argument by *indexing into this
//! same enumeration*, so a row's `oid` and the definition that OID renders are the same
//! fact by construction.
//!
//! [`constraint_def`] is also the dump's constraint renderer (`pglet::dump` emits
//! `ALTER TABLE t ADD CONSTRAINT n <def>;`), so the text a `pg_dump`-style export
//! carries and the text an introspecting client reads are one string. They were two,
//! and they disagreed: the dump spelled `REFERENCES parent (id)` where 18.6's
//! `pg_get_constraintdef` spells `parent(id)`, and ordered `ON DELETE` before
//! `ON UPDATE` where 18.6 orders `ON UPDATE` first. Both spellings reload into stock
//! Postgres, so nothing could see the split — and the fuz spine compares these strings
//! *across backends*, which is exactly where a cosmetic split reads as schema drift.
//!
//! # The OID is synthesized, and that is this module's one limit
//!
//! pglet does not store a constraint OID. Three of the five kinds are catalog records
//! (`CheckConstraint`, `ForeignKey`, and a constraint-marked `Index`) and could carry
//! one; the fourth — `NOT NULL`, which 18.6 catalogs as `contype = 'n'` — is not a
//! record here at all but a `Column.nullable` bit, so no persisted field could name it.
//! Rather than mint OIDs for three kinds and derive them for the fourth (two
//! derivations, hence a seam), every entry's OID is derived here: [`SYNTHETIC_OID_BASE`]
//! plus its position in this enumeration.
//!
//! What that costs, stated plainly: a constraint's OID **shifts when a constraint is
//! added or dropped anywhere in the catalog**, where 18.6's is stable for the object's
//! lifetime. A consumer that reads `oid` and uses it inside the same statement — which
//! is what `pg_get_constraintdef(c.oid)` is — cannot see the difference; one that
//! caches an OID across DDL can. Recorded in `EDGES.md`.

use super::schema::{
    CheckConstraint, Column, ConstraintKind, ForeignKey, Index, ReferentialAction, Schema, Table,
    make_object_name,
};
use crate::deparse::{RenderMode, render_expr_mode};
use crate::lexer::quote_ident;
use crate::planner::{Expr, PlannerScope};
use crate::{Error, Result};

/// The first OID handed to a synthesized `pg_constraint` row.
///
/// Deliberately far above the object-OID space `executor::helpers::next_object_oid`
/// hands out from (which starts at 16384 and advances by one per table, index, and enum
/// type), so a synthesized constraint OID can never collide with a real relation's. The
/// two spaces meeting would be a silent wrong — a `JOIN pg_class ON oid = conrelid`
/// finding the wrong object — so the distance is asserted rather than assumed
/// (`next_object_oid` debug-asserts it stays below this, and it is the seed every mint
/// site reads — `generate_object_oid` included).
pub const SYNTHETIC_OID_BASE: i32 = 0x4000_0000;

/// The first OID handed to a synthesized `pg_enum` row.
///
/// A **second** band, half a billion above the constraint one, because the two row sets
/// are different objects and 18.6 gives them distinct OIDs: numbering both from
/// [`SYNTHETIC_OID_BASE`] made the first enum label and the first constraint share
/// `0x40000000`, which is precisely the collision this band scheme exists to prevent —
/// the module header said so while the code did it. Still far below `i32::MAX`, so
/// neither band can reach the other or overflow.
pub const SYNTHETIC_ENUM_OID_BASE: i32 = 0x6000_0000;

/// Which kind of constraint an entry is, with the catalog record behind it.
///
/// The `contype` letters are 18.6's (`pg_constraint.h`): `n` NOT NULL, `c` CHECK, `f`
/// FOREIGN KEY, `p` PRIMARY KEY, `u` UNIQUE. `n` is a **distinct kind** there and not a
/// CHECK — only `information_schema.table_constraints` / `check_constraints` relabel a
/// NOT NULL as one.
#[derive(Debug, Clone, Copy)]
pub enum ConstraintEntryKind<'a> {
    /// `contype = 'n'` — one per `NOT NULL` column.
    NotNull(&'a Column),
    /// `contype = 'c'`.
    Check(&'a CheckConstraint),
    /// `contype = 'f'`.
    ForeignKey(&'a ForeignKey),
    /// `contype = 'p'` / `'u'` — a constraint-marked index (the constraint's name *is*
    /// the index's name; see [`ConstraintKind`]).
    Index(&'a Index, ConstraintKind),
}

impl ConstraintEntryKind<'_> {
    /// This kind's `pg_constraint.contype` letter.
    pub const fn contype(self) -> &'static str {
        match self {
            Self::NotNull(_) => "n",
            Self::Check(_) => "c",
            Self::ForeignKey(_) => "f",
            Self::Index(_, ConstraintKind::PrimaryKey) => "p",
            Self::Index(_, ConstraintKind::Unique) => "u",
        }
    }

    /// `pg_constraint.connoinherit` — `true` for PRIMARY KEY / UNIQUE / FOREIGN KEY,
    /// `false` for CHECK and NOT NULL (measured on 18.6 over a pglet-shaped schema; a
    /// pglet catalog has no inheritance, so the value is a function of the kind alone).
    pub const fn noinherit(self) -> bool {
        matches!(self, Self::ForeignKey(_) | Self::Index(_, _))
    }
}

/// One constraint of the catalog: its synthesized OID, the table it is on, and the
/// record behind it.
///
/// The **name** is a method rather than a field, because one of the five kinds has no
/// stored name to put in one: a `NOT NULL` is a `Column.nullable` bit here, so its
/// `conname` is minted from `(table, column)`. A field would have to hold a placeholder
/// for that kind, and a placeholder is what a reader eventually reports.
#[derive(Debug, Clone, Copy)]
pub struct ConstraintEntry<'a> {
    /// The synthesized `pg_constraint.oid` — see this module's header.
    pub oid: i32,
    /// The table the constraint is declared on (`conrelid`).
    pub table: &'a Table,
    pub kind: ConstraintEntryKind<'a>,
}

impl ConstraintEntry<'_> {
    /// The `pg_constraint.conname` — the catalog's own name, or 18.6's
    /// `<table>_<column>_not_null` for the NOT NULL rows pglet stores no record for, cut
    /// to 63 bytes as 18.6 cuts it.
    ///
    /// That one name is rendered here and minted nowhere, so it has no digit: where 18.6
    /// steps a `NOT NULL`'s name around one already held, this answers the same name
    /// twice (`EDGES.md` row 278's residual).
    #[must_use]
    pub fn name(&self) -> String {
        match self.kind {
            ConstraintEntryKind::NotNull(col) => {
                make_object_name(&self.table.name, &col.name, "not_null")
            }
            ConstraintEntryKind::Check(check) => check.name.clone(),
            ConstraintEntryKind::ForeignKey(fk) => fk.name.clone(),
            ConstraintEntryKind::Index(index, _) => index.name.clone(),
        }
    }
}

/// Every constraint in `schema`, in one canonical order, each with its synthesized OID.
///
/// The order is table name (the catalog's `BTreeMap` order), then, within a table,
/// NOT NULL by column position → CHECK in declared order → FOREIGN KEY in declared
/// order → PRIMARY KEY / UNIQUE by index name. It is a pure function of the catalog, so
/// two calls against one `Schema` agree — which is what makes indexing back into it by
/// OID sound.
#[must_use]
pub fn constraint_entries(schema: &Schema) -> Vec<ConstraintEntry<'_>> {
    let mut kinds: Vec<(&Table, ConstraintEntryKind<'_>)> = Vec::new();
    for table in schema.tables.values() {
        for column in &table.columns {
            if !column.nullable {
                kinds.push((table, ConstraintEntryKind::NotNull(column)));
            }
        }
        for check in &table.checks {
            kinds.push((table, ConstraintEntryKind::Check(check)));
        }
        for fk in &table.foreign_keys {
            kinds.push((table, ConstraintEntryKind::ForeignKey(fk)));
        }
        for index in schema.indexes.values() {
            if index.table_name != table.name {
                continue;
            }
            if let Some(kind) = index.constraint_kind() {
                kinds.push((table, ConstraintEntryKind::Index(index, kind)));
            }
        }
    }
    kinds
        .into_iter()
        .enumerate()
        .map(|(i, (table, kind))| ConstraintEntry {
            oid: SYNTHETIC_OID_BASE.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)),
            table,
            kind,
        })
        .collect()
}

/// Render a constraint's SQL definition exactly as 18.6's `pg_get_constraintdef` does.
///
/// The five shapes, each measured on 18.6 rather than inferred:
/// `NOT NULL id`, `CHECK ((qty > 0))`, `PRIMARY KEY (id)`, `UNIQUE (code, region)`,
/// `FOREIGN KEY (pid) REFERENCES parent(id) ON DELETE CASCADE`. Note the FK's
/// `parent(id)` — no space before the paren — and that `MATCH` precedes `ON UPDATE`,
/// which precedes `ON DELETE`, with the `NO ACTION` / `MATCH SIMPLE` defaults omitted.
///
/// The CHECK arm's inner text is the **one** `Expr` deparser (`crate::deparse`), whose
/// parenthesization already matches 18.6's (`render_binary_op` wraps, so `qty > 0`
/// renders `(qty > 0)` and this wraps it once more). `mode` is what makes this function
/// serve both callers: `pg_get_constraintdef` passes `RenderMode::Catalog` over the
/// constraint's own relation, so an *unknown* literal resolves and an `IN` list renders
/// as the node 18.6 stores; `pglet::dump` passes `RenderMode::Portable`, whose
/// rendering has to re-parse to the identical `Expr`. Where the catalog rendering still
/// diverges from 18.6's — a nested implicit coercion, `CASE`'s indentation — this
/// diverges with it; the set is `docs/deparse.md` § What is recorded, not reproduced,
/// not a new one.
///
/// A constraint-marked index is never `NULLS NOT DISTINCT` here (the parser accepts that
/// modifier only on `CREATE INDEX`, which mints no constraint marker), so the `UNIQUE`
/// arm has no such spelling to emit.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] when a CHECK predicate is outside the deparser's
/// renderable subset — the same error the dump's render probe raises at DDL time, so a
/// stored check that reaches here can always be rendered.
pub fn constraint_def(kind: ConstraintEntryKind<'_>, mode: RenderMode<'_>) -> Result<String> {
    Ok(match kind {
        ConstraintEntryKind::NotNull(col) => format!("NOT NULL {}", quote_ident(&col.name)),
        ConstraintEntryKind::Check(check) => {
            format!("CHECK ({})", render_expr_mode(&check.expr, mode)?)
        }
        ConstraintEntryKind::Index(index, kind) => {
            let keyword = match kind {
                ConstraintKind::PrimaryKey => "PRIMARY KEY",
                ConstraintKind::Unique => "UNIQUE",
            };
            format!("{keyword} ({})", join_idents(index.key.bare_columns()))
        }
        ConstraintEntryKind::ForeignKey(fk) => render_foreign_key(fk),
    })
}

/// `FOREIGN KEY (<cols>) REFERENCES <parent>(<pcols>)[ MATCH …][ ON UPDATE …][ ON
/// DELETE …]`, in 18.6's clause order with its defaults omitted.
fn render_foreign_key(fk: &ForeignKey) -> String {
    let mut out = format!(
        "FOREIGN KEY ({}) REFERENCES {}({})",
        join_idents(Some(fk.columns.as_slice())),
        quote_ident(&fk.parent_table),
        join_idents(Some(fk.parent_columns.as_slice())),
    );
    // pglet has no `MATCH FULL` / `MATCH PARTIAL`, so `MATCH SIMPLE` — 18.6's default,
    // which it omits — is the only spelling reachable and nothing is appended here.
    if let Some(action) = referential_action_sql(fk.on_update) {
        out.push_str(" ON UPDATE ");
        out.push_str(action);
    }
    if let Some(action) = referential_action_sql(fk.on_delete) {
        out.push_str(" ON DELETE ");
        out.push_str(action);
    }
    out
}

/// The `ON UPDATE` / `ON DELETE` spelling for an action, or `None` for `NO ACTION` —
/// the default, which 18.6's `pg_get_constraintdef` omits.
const fn referential_action_sql(action: ReferentialAction) -> Option<&'static str> {
    match action {
        ReferentialAction::NoAction => None,
        ReferentialAction::Restrict => Some("RESTRICT"),
        ReferentialAction::Cascade => Some("CASCADE"),
        ReferentialAction::SetNull => Some("SET NULL"),
    }
}

/// `pg_constraint.confupdtype` / `confdeltype` — 18.6's `"char"` code for a referential
/// action (`pg_constraint.h`'s `FKCONSTR_ACTION_*`).
pub const fn referential_action_code(action: ReferentialAction) -> &'static str {
    match action {
        ReferentialAction::NoAction => "a",
        ReferentialAction::Restrict => "r",
        ReferentialAction::Cascade => "c",
        ReferentialAction::SetNull => "n",
    }
}

/// A quoted, comma-joined identifier list (`code, region`); empty for `None`, which a
/// constraint-marked index's key never is (it is bare-column by construction).
fn join_idents(cols: Option<&[String]>) -> String {
    cols.unwrap_or(&[])
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The columns a CHECK predicate references, in **predicate walk order**, each at most
/// once.
///
/// The order is 18.6's and it is *not* the table's column order: `pull_var_clause`
/// yields the vars as the expression names them, so `CHECK (c > a)` over `(a, b, c)` is
/// `conkey = {3,1}` and `CHECK (b + a > c)` is `{2,1,3}` (both measured). Sorting by
/// position gave `{1,3}` and `{1,2,3}` — a silently wrong vector in a relation that
/// binds exhaustively, and one that the single-column `CHECK (qty > 0)` this rule was
/// first written against could not distinguish.
///
/// **One derivation, two readers**, and they answer different questions with it:
/// `pg_constraint.conkey` wants the positions in this order, and
/// `information_schema.constraint_column_usage` wants a row per column — where the order
/// is not a contract (a view's rows are a set; the fixtures sort). Written twice, the
/// two would drift over exactly the cases that are hard: a predicate naming one column
/// twice, naming them out of order, or naming a column the table does not have.
#[must_use]
pub fn check_referenced_columns<'a>(table: &'a Table, check: &CheckConstraint) -> Vec<&'a Column> {
    let mut names: Vec<String> = Vec::new();
    collect_expr_columns(&check.expr, &mut names);
    let mut cols: Vec<&Column> = Vec::new();
    for name in &names {
        // Deduplicate by **first occurrence**, keeping walk order — `sort` + `dedup`
        // would keep the same set and lose the order this function exists to preserve.
        if cols.iter().any(|c| c.name == *name) {
            continue;
        }
        if let Some(col) = table.columns.iter().find(|c| c.name == *name) {
            cols.push(col);
        }
    }
    cols
}

/// Push every base column name `expr` references (skipping the `*` wildcard).
fn collect_expr_columns(expr: &Expr, out: &mut Vec<String>) {
    if let Expr::Column { name, .. } = expr
        && name != "*"
    {
        out.push(name.clone());
    }
    expr.for_each_child(|child| collect_expr_columns(child, out));
}

/// Resolve a `pg_get_constraintdef(oid)` argument to its rendered definition.
///
/// Reads [`constraint_entries`] — the same enumeration `pg_constraint`'s rows come from
/// — so the OID a client just read back always resolves, and to the row it read it from.
///
/// # Errors
///
/// [`Error::Execution`] for an OID no constraint holds. 18.6 answers **NULL** there
/// rather than raising (measured), so the caller in `eval_string_fn` maps this to NULL;
/// the `Result` is what keeps that mapping a decision at the call site instead of a
/// silent one here, where a genuine render failure (a CHECK outside the deparser's
/// subset) would otherwise become the same NULL.
pub fn constraint_def_by_oid(schema: &Schema, oid: i32) -> Result<String> {
    let entries = constraint_entries(schema);
    let entry = entries
        .iter()
        .find(|e| e.oid == oid)
        .ok_or_else(|| Error::Execution(format!("cache lookup failed for constraint {oid}")))?;
    // `pg_get_constraintdef` is a catalog surface: it prints what 18.6's own
    // `pg_get_constraintdef` prints, over the constraint's own relation so an
    // *unknown* literal in the CHECK resolves against the column it is compared to.
    let scope = PlannerScope::catalog(schema, Some(entry.table.name.as_str()));
    constraint_def(entry.kind, RenderMode::Catalog(&scope))
}

#[cfg(test)]
mod tests {
    use super::{SYNTHETIC_ENUM_OID_BASE, SYNTHETIC_OID_BASE};

    /// The synthesized-constraint OID band must stay clear of the object-OID space.
    ///
    /// `executor::helpers::next_object_oid` hands out `16384, 16385, …`, one per
    /// table, index and enum type. If the two spaces ever met, a
    /// `JOIN pg_class ON oid = conrelid` would resolve a constraint's OID to a relation
    /// — a silent wrong, not a failure. The distance is over a billion allocations, each
    /// of which needs its own DDL statement and its own catalog entry, so this is a
    /// statement about the shape of the two spaces rather than a bound anything
    /// approaches.
    /// The two synthesized bands must not overlap **or touch**: a `pg_enum` row and a
    /// `pg_constraint` row are different objects and 18.6 gives them different OIDs, so
    /// numbering both from [`SYNTHETIC_OID_BASE`] made the first enum label and the first
    /// constraint share `0x40000000`.
    #[test]
    fn the_two_synthetic_bands_are_disjoint() {
        const { assert!(SYNTHETIC_ENUM_OID_BASE > SYNTHETIC_OID_BASE) };
        const {
            assert!(
                SYNTHETIC_ENUM_OID_BASE - SYNTHETIC_OID_BASE > 100_000_000,
                "the constraint band must not be able to run into the enum one"
            );
        };
        const {
            assert!(
                i32::MAX - SYNTHETIC_ENUM_OID_BASE > 100_000_000,
                "the enum band must leave room above it for its own rows"
            );
        };
    }

    #[test]
    fn the_synthetic_oid_band_is_far_above_the_object_oid_space() {
        const FIRST_USER_OID: i32 = 16384;
        const { assert!(SYNTHETIC_OID_BASE > FIRST_USER_OID) };
        const {
            assert!(
                SYNTHETIC_OID_BASE - FIRST_USER_OID > 1_000_000_000,
                "the band must leave room for more objects than any catalog can hold"
            );
        };
    }
}
