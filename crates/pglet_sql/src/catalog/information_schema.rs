//! Virtual `information_schema` view generation.
//!
//! Like `pg_catalog` (see `pg_catalog.rs`), these rows don't exist in storage —
//! they're synthesized on the fly from the live `Schema`. The planner routes a
//! reference to `information_schema.columns` to a `Plan::VirtualScan`, and the
//! executor calls the method here.
//!
//! Column schemas are defined in `virtual_tables.rs`.
//!
//! Implemented views: `columns`, `table_constraints` (CHECK + FOREIGN KEY +
//! PRIMARY KEY/UNIQUE), the FK-introspection view `referential_constraints`, and
//! `key_column_usage` / `constraint_column_usage` (which carry both the FK and the
//! PK/UNIQUE rows). Add more views as consumers need them — each is another method
//! building rows from the catalog.
//!
//! PK/UNIQUE rows come from the catalog `Index`'s constraint marker
//! ([`ConstraintKind`]): an index created for a declared `PRIMARY KEY` / `UNIQUE`
//! *is* the constraint, while a standalone `CREATE UNIQUE INDEX` is invisible here —
//! exactly Postgres's split. The `constraint_name` reported is the index's own name,
//! which is the constraint's: the user's `CONSTRAINT <n>` where one was declared, else
//! Postgres's auto-name. Synthesizing `<table>_pkey` here instead would hand an
//! introspecting client a plausible name that is not the constraint's, which it has no
//! way to detect.

use super::constraints_view::{ConstraintEntryKind, check_referenced_columns, constraint_entries};
use super::schema::{
    Column, ColumnDefault, ConstraintKind, IdentityGeneration, Index, ReferentialAction,
};
use super::{Schema, Table};
use crate::Result;
use crate::deparse::render_catalog_expr;
use crate::planner::PlannerScope;
use crate::types::{TypeOid, Value};

/// Virtual `information_schema` views over the live schema.
///
/// These don't exist as real tables — results are generated on the fly from the
/// actual schema, mirroring `PgCatalog`.
#[derive(Debug)]
pub struct InformationSchema<'a> {
    schema: &'a Schema,
}

impl<'a> InformationSchema<'a> {
    /// Creates a new `InformationSchema` view over the given schema.
    pub const fn new(schema: &'a Schema) -> Self {
        Self { schema }
    }

    /// Query `information_schema.tables` — one row per user table.
    ///
    /// The column list is defined by `CatalogTable::InformationSchemaTables`.
    /// pglet's tables are all ordinary base tables, so `table_type` is always
    /// `BASE TABLE` (no views/foreign/temp). `table_catalog` is a fixed value —
    /// pglet has no database-name concept, and consumers filter on
    /// `table_schema` / `table_name` (the spine's `auth_tables_exist` probe).
    pub fn tables(&self) -> Vec<Vec<Value>> {
        self.schema
            .tables
            .values()
            .map(|table| {
                vec![
                    Value::Text("pglet".into()),       // table_catalog
                    Value::Text(table.schema.clone()), // table_schema
                    Value::Text(table.name.clone()),   // table_name
                    Value::Text("BASE TABLE".into()),  // table_type
                ]
            })
            .collect()
    }

    /// Query `information_schema.columns` — one row per column of every table.
    ///
    /// The column list is defined by `CatalogTable::InformationSchemaColumns`.
    /// `column_default` renders the catalog's stored `ColumnDefault` to Postgres's
    /// spelling (`gen_random_uuid()`, `now()`, `clock_timestamp()`,
    /// `CURRENT_TIMESTAMP`, `false`, `'success'::text`, `nextval('…'::regclass)`), and
    /// SQL NULL for a column with no default; see `render_column_default`.
    ///
    /// An **identity** column's `column_default` is SQL NULL, as 18.6's is — the
    /// sequence draw is recorded by `is_identity` / `identity_generation` there, not by
    /// a default. Reporting the `nextval(…)` pglet stores told a schema-diff consumer
    /// that a `GENERATED ALWAYS AS IDENTITY` column was a `SERIAL`, which is a different
    /// column (a SERIAL accepts an explicit value; an identity `ALWAYS` is `428C9`).
    ///
    /// An **enum** column reports 18.6's `USER-DEFINED` as its `data_type` and the enum
    /// type's own name as `udt_name` — the pair the fuz spine's snapshot compares. It
    /// used to report its storage type `text` for both, which read as a plain text
    /// column.
    pub fn columns(&self) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        for table in self.schema.tables.values() {
            for col in &table.columns {
                rows.push(vec![
                    // table_catalog — pglet has no database-name concept; a fixed
                    // value (the migration probe never filters on it).
                    Value::Text("pglet".into()),
                    Value::Text(table.schema.clone()), // table_schema
                    Value::Text(table.name.clone()),   // table_name
                    Value::Text(col.name.clone()),     // column_name
                    Value::Int4(i32::from(col.position)), // ordinal_position
                    render_column_default(col, self.schema), // column_default
                    Value::Text(if col.nullable { "YES" } else { "NO" }.into()), // is_nullable
                    Value::Text(column_data_type(col).into()), // data_type
                    character_maximum_length(col),     // character_maximum_length
                    numeric_precision(col),            // numeric_precision
                    numeric_precision_radix(col),      // numeric_precision_radix
                    numeric_scale(col),                // numeric_scale
                    datetime_precision(col),           // datetime_precision
                    Value::Text(udt_name(col)),        // udt_name
                    Value::Text(if col.identity.is_some() { "YES" } else { "NO" }.into()), // is_identity
                    match col.identity {
                        Some(IdentityGeneration::Always) => Value::Text("ALWAYS".into()),
                        Some(IdentityGeneration::ByDefault) => Value::Text("BY DEFAULT".into()),
                        None => Value::Null,
                    }, // identity_generation
                    // pglet has no `GENERATED … AS (…) STORED` columns, so 18.6's
                    // `NEVER` is the only value a pglet-shaped schema can produce.
                    Value::Text("NEVER".into()), // is_generated
                ]);
            }
        }
        rows
    }

    /// Query `information_schema.key_column_usage` — one row per key column of every
    /// PRIMARY KEY / UNIQUE / FOREIGN KEY constraint, in key order.
    ///
    /// For a **PK/UNIQUE** constraint the row names the constrained column and
    /// `position_in_unique_constraint` is NULL, matching Postgres (that column exists
    /// only to pair an FK's columns with the referenced key's).
    ///
    /// For a **FOREIGN KEY**, `ordinal_position` is the column's 1-based position in
    /// the FK's local column list and `position_in_unique_constraint` is the 1-based
    /// position of the *paired* parent column within the referenced unique key's own
    /// key order — so a reordered `REFERENCES parent(y, x)` against `PRIMARY KEY (x,
    /// y)` reports the permuted positions (Postgres semantics).
    pub fn key_column_usage(&self) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        for (index, _, table) in self.constraint_indexes() {
            // `bare_columns` always answers: a constraint's index is plain-column by
            // construction (`create_unique_index`).
            for (i, column) in index.key.bare_columns().unwrap_or(&[]).iter().enumerate() {
                rows.push(vec![
                    Value::Text("pglet".into()),       // constraint_catalog
                    Value::Text(table.schema.clone()), // constraint_schema
                    Value::Text(index.name.clone()),   // constraint_name
                    Value::Text("pglet".into()),       // table_catalog
                    Value::Text(table.schema.clone()), // table_schema
                    Value::Text(table.name.clone()),   // table_name
                    Value::Text(column.clone()),       // column_name
                    Value::Int4((i + 1) as i32),       // ordinal_position
                    Value::Null,                       // position_in_unique_constraint
                ]);
            }
        }
        for table in self.schema.tables.values() {
            for fk in &table.foreign_keys {
                let key_cols = self
                    .schema
                    .covering_unique_index(&fk.parent_table, &fk.parent_columns)
                    .and_then(|idx| idx.key.bare_columns());
                for (i, column) in fk.columns.iter().enumerate() {
                    // The paired parent column's 1-based slot in the referenced
                    // unique key; falls back to the ordinal if the key isn't found.
                    let pos_in_unique = fk
                        .parent_columns
                        .get(i)
                        .zip(key_cols)
                        .and_then(|(pc, cols)| cols.iter().position(|c| c == pc))
                        .map_or(i + 1, |p| p + 1);
                    rows.push(vec![
                        Value::Text("pglet".into()),       // constraint_catalog
                        Value::Text(table.schema.clone()), // constraint_schema
                        Value::Text(fk.name.clone()),      // constraint_name
                        Value::Text("pglet".into()),       // table_catalog
                        Value::Text(table.schema.clone()), // table_schema
                        Value::Text(table.name.clone()),   // table_name
                        Value::Text(column.clone()),       // column_name
                        Value::Int4((i + 1) as i32),       // ordinal_position
                        Value::Int4(pos_in_unique as i32), // position_in_unique_constraint
                    ]);
                }
            }
        }
        rows
    }

    /// Query `information_schema.referential_constraints` — one row per FOREIGN
    /// KEY, carrying its `delete_rule` and `update_rule`.
    ///
    /// `match_option` is always `NONE` (pglet has no MATCH options); `delete_rule` /
    /// `update_rule` come from the FK's `ON DELETE` / `ON UPDATE` actions.
    /// `unique_constraint_name` is resolved best-effort to the parent table's
    /// unique/PK index on the referenced column, else NULL.
    pub fn referential_constraints(&self) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        for table in self.schema.tables.values() {
            for fk in &table.foreign_keys {
                let unique_constraint = self
                    .schema
                    .covering_unique_index(&fk.parent_table, &fk.parent_columns)
                    .map_or(Value::Null, |idx| Value::Text(idx.name.clone()));
                rows.push(vec![
                    Value::Text("pglet".into()),                        // constraint_catalog
                    Value::Text(table.schema.clone()),                  // constraint_schema
                    Value::Text(fk.name.clone()),                       // constraint_name
                    Value::Text("pglet".into()),                        // unique_constraint_catalog
                    Value::Text("public".into()),                       // unique_constraint_schema
                    unique_constraint,                                  // unique_constraint_name
                    Value::Text("NONE".into()),                         // match_option
                    Value::Text(referential_rule(fk.on_update).into()), // update_rule
                    Value::Text(referential_rule(fk.on_delete).into()), // delete_rule
                ]);
            }
        }
        rows
    }

    /// Query `information_schema.constraint_column_usage` — the columns a constraint
    /// *uses*: the referenced (parent) columns of a FOREIGN KEY, and the key columns
    /// of a PRIMARY KEY / UNIQUE.
    ///
    /// For an FK the `table_name` / `column_name` are the parent side it references;
    /// for a PK/UNIQUE they are the constrained table's own key columns; a CHECK and a
    /// NOT NULL each list the columns their predicate references.
    pub fn constraint_column_usage(&self) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        // The `NOT NULL` and `CHECK` rows 18.6 emits here — the former its
        // `contype = 'n'` constraints (relabelled CHECK by `table_constraints`), the
        // latter one row per column the predicate references. Both were missing, which
        // is a *silent* gap in a view whose whole content is "which columns does this
        // constraint use": an introspecting client sees a CHECK with no columns.
        for entry in constraint_entries(self.schema) {
            let columns: Vec<&Column> = match entry.kind {
                ConstraintEntryKind::NotNull(col) => vec![col],
                ConstraintEntryKind::Check(check) => check_referenced_columns(entry.table, check),
                _ => continue,
            };
            for col in columns {
                rows.push(vec![
                    Value::Text("pglet".into()),             // table_catalog
                    Value::Text(entry.table.schema.clone()), // table_schema
                    Value::Text(entry.table.name.clone()),   // table_name
                    Value::Text(col.name.clone()),           // column_name
                    Value::Text("pglet".into()),             // constraint_catalog
                    Value::Text(entry.table.schema.clone()), // constraint_schema
                    Value::Text(entry.name()),               // constraint_name
                ]);
            }
        }
        for (index, _, table) in self.constraint_indexes() {
            for column in index.key.bare_columns().unwrap_or(&[]) {
                rows.push(vec![
                    Value::Text("pglet".into()),       // table_catalog
                    Value::Text(table.schema.clone()), // table_schema
                    Value::Text(table.name.clone()),   // table_name
                    Value::Text(column.clone()),       // column_name
                    Value::Text("pglet".into()),       // constraint_catalog
                    Value::Text(table.schema.clone()), // constraint_schema
                    Value::Text(index.name.clone()),   // constraint_name
                ]);
            }
        }
        for table in self.schema.tables.values() {
            for fk in &table.foreign_keys {
                for parent_column in &fk.parent_columns {
                    rows.push(vec![
                        Value::Text("pglet".into()),          // table_catalog
                        Value::Text("public".into()),         // table_schema (parent)
                        Value::Text(fk.parent_table.clone()), // table_name (parent)
                        Value::Text(parent_column.clone()),   // column_name (parent)
                        Value::Text("pglet".into()),          // constraint_catalog
                        Value::Text(table.schema.clone()),    // constraint_schema
                        Value::Text(fk.name.clone()),         // constraint_name
                    ]);
                }
            }
        }
        rows
    }

    /// Query `information_schema.table_constraints` — one row per CHECK, FOREIGN KEY,
    /// PRIMARY KEY and UNIQUE constraint across all tables.
    ///
    /// CHECK comes from `Table.checks`, FOREIGN KEY from `Table.foreign_keys`, and
    /// PRIMARY KEY / UNIQUE from the constraint-marked indexes (see
    /// [`Self::constraint_indexes`]). pglet emits no `NOT NULL` rows, which Postgres 18
    /// materializes as `<table>_<col>_not_null` CHECK constraints.
    ///
    /// **Every** CHECK gets a row, reading the name the executor minted at DDL time —
    /// the same stored fact the violation message and the dump read. Skipping the
    /// unnamed ones (as this used to, back when a name could be absent) hid a
    /// constraint Postgres reports, and did so *silently*: an introspecting client sees
    /// a table with fewer constraints than it has.
    pub fn table_constraints(&self) -> Vec<Vec<Value>> {
        constraint_entries(self.schema)
            .iter()
            .map(|entry| {
                let constraint_type = match entry.kind {
                    // 18.6 relabels a `pg_constraint` `contype = 'n'` row as CHECK here
                    // — this view is where the relabel happens, and the only place it
                    // does.
                    ConstraintEntryKind::NotNull(_) | ConstraintEntryKind::Check(_) => "CHECK",
                    ConstraintEntryKind::ForeignKey(_) => "FOREIGN KEY",
                    ConstraintEntryKind::Index(_, kind) => kind.constraint_type(),
                };
                constraint_row(entry.table, &entry.name(), constraint_type)
            })
            .collect()
    }

    /// Query `information_schema.check_constraints` — one row per CHECK, plus one per
    /// `NOT NULL` column.
    ///
    /// The NOT NULL rows carry 18.6's rendering `<column> IS NOT NULL` (measured: `cid
    /// IS NOT NULL`, unparenthesized, where a real CHECK's clause is the parenthesized
    /// deparse). 18.6 also lists the constraints of its own `pg_catalog` /
    /// `information_schema` relations here; pglet has none of those to report, so a
    /// differential query must filter `constraint_schema = 'public'`.
    ///
    /// **A CHECK that will not deparse fails the read rather than vanishing from it.**
    /// The `filter_map`-over-`.ok()?` this replaced is the same silent-omission shape
    /// [`Self::table_constraints`] two functions above documents as a bug it fixed —
    /// an introspecting client sees a table with fewer constraints than it has, and
    /// nothing tells it so. Unreachable today, and by a guard that is not in this file:
    /// `executor::ddl`'s render probe refuses a CHECK the deparser cannot spell at
    /// `CREATE TABLE` / `ALTER TABLE` time, so a stored `Check.expr` that will not render
    /// would have had to get past that probe — a probe that runs in the **portable**
    /// mode, which declines by `Expr` variant and so answers for the catalog mode too,
    /// the two sharing one arm list. That
    /// makes this a shape rule rather than a live defect — which is why the probe being
    /// the only thing holding it up must not be re-derived from this call's `Ok`-ness.
    ///
    /// # Errors
    ///
    /// Whatever [`render_catalog_expr`] refuses a stored CHECK expression with.
    pub fn check_constraints(&self) -> Result<Vec<Vec<Value>>> {
        let mut rows = Vec::new();
        for entry in &constraint_entries(self.schema) {
            let clause = match entry.kind {
                ConstraintEntryKind::NotNull(col) => format!("{} IS NOT NULL", col.name),
                ConstraintEntryKind::Check(check) => {
                    let scope = PlannerScope::catalog(self.schema, Some(entry.table.name.as_str()));
                    render_catalog_expr(&check.expr, &scope)?
                }
                // A FOREIGN KEY / PRIMARY KEY / UNIQUE entry has no `check_clause` and is
                // not a row of this view — a deliberate skip, not a failure.
                _ => continue,
            };
            rows.push(vec![
                Value::Text("pglet".into()),             // constraint_catalog
                Value::Text(entry.table.schema.clone()), // constraint_schema
                Value::Text(entry.name()),               // constraint_name
                Value::Text(clause),                     // check_clause
            ]);
        }
        Ok(rows)
    }

    /// Query `information_schema.sequences` — one row per sequence that is **not** an
    /// identity column's.
    ///
    /// That exclusion is 18.6's, and it is not obvious: its view skips a sequence
    /// carrying an INTERNAL `pg_depend` edge, which is exactly an identity column's,
    /// while a `SERIAL`'s edge is AUTO and *is* listed (measured — `parent_id_seq` and
    /// `child_cid_seq` absent, `ser_a_seq` present). pglet has no `pg_depend`, so the
    /// same split reads off the owner column's `identity` flag.
    ///
    /// `data_type` follows the owning column's type for a `SERIAL`, and is `bigint` for
    /// a bare `CREATE SEQUENCE` — 18.6's default sequence type. `minimum_value` /
    /// `maximum_value` / `numeric_precision` follow from that type; `increment` is 1 and
    /// `cycle_option` `NO`, because pglet parses `INCREMENT` / `CYCLE` and ignores them
    /// (`Sequence`'s doc).
    pub fn sequences(&self) -> Vec<Vec<Value>> {
        self.schema
            .sequences
            .iter()
            .filter(|(_, seq)| !self.owned_by_identity(seq))
            .map(|(name, seq)| {
                // A sequence pglet creates always ascends by 1, so `minimum_value` is
                // 18.6's ascending default of 1 and `maximum_value` the type's ceiling;
                // the type's floor belongs to a descending sequence, which pglet has no
                // syntax to create.
                let (data_type, precision, max) = match self.sequence_type(seq) {
                    TypeOid::Int2 => ("smallint", 16, "32767"),
                    TypeOid::Int4 => ("integer", 32, "2147483647"),
                    _ => ("bigint", 64, "9223372036854775807"),
                };
                vec![
                    Value::Text("pglet".into()),        // sequence_catalog
                    Value::Text("public".into()),       // sequence_schema
                    Value::Text(name.clone()),          // sequence_name
                    Value::Text(data_type.into()),      // data_type
                    Value::Int4(precision),             // numeric_precision
                    Value::Int4(2),                     // numeric_precision_radix
                    Value::Int4(0),                     // numeric_scale
                    Value::Text(seq.start.to_string()), // start_value
                    Value::Text("1".into()),            // minimum_value
                    Value::Text(max.into()),            // maximum_value
                    Value::Text("1".into()),            // increment
                    Value::Text("NO".into()),           // cycle_option
                ]
            })
            .collect()
    }

    /// Whether `seq` is an identity column's sequence — 18.6's INTERNAL-dependency test,
    /// read off the owner column's `identity` flag.
    fn owned_by_identity(&self, seq: &super::Sequence) -> bool {
        seq.owner.as_ref().is_some_and(|o| {
            self.schema
                .get_table(&o.table)
                .and_then(|t| t.columns.iter().find(|c| c.name == o.column))
                .is_some_and(|c| c.identity.is_some())
        })
    }

    /// The declared type of `seq` — its owning column's for a `SERIAL`, else `int8`
    /// (18.6's `CREATE SEQUENCE` default).
    fn sequence_type(&self, seq: &super::Sequence) -> TypeOid {
        seq.owner
            .as_ref()
            .and_then(|o| {
                self.schema
                    .get_table(&o.table)
                    .and_then(|t| t.columns.iter().find(|c| c.name == o.column))
            })
            .map_or(TypeOid::Int8, |c| c.type_oid)
    }

    /// Every index that backs a declared `PRIMARY KEY` / `UNIQUE` constraint, paired
    /// with its table — name-sorted (the catalog's `BTreeMap` order), so the three
    /// views that consume it agree and stay deterministic.
    ///
    /// A standalone `CREATE UNIQUE INDEX` has no marker and is skipped: it is an
    /// index, not a constraint, and Postgres likewise gives it no `pg_constraint` row.
    fn constraint_indexes(&self) -> impl Iterator<Item = (&'a Index, ConstraintKind, &'a Table)> {
        self.schema.indexes.values().filter_map(|index| {
            let kind = index.constraint_kind()?;
            let table = self.schema.get_table(&index.table_name)?;
            Some((index, kind, table))
        })
    }
}

/// The `referential_constraints.delete_rule` / `update_rule` text for a
/// referential action, matching Postgres's spelling (`CASCADE` / `SET NULL` /
/// `NO ACTION` / `RESTRICT`).
const fn referential_rule(action: ReferentialAction) -> &'static str {
    match action {
        ReferentialAction::NoAction => "NO ACTION",
        ReferentialAction::Restrict => "RESTRICT",
        ReferentialAction::Cascade => "CASCADE",
        ReferentialAction::SetNull => "SET NULL",
    }
}

/// Render a column's `DEFAULT` to its `information_schema.columns.column_default`
/// text, matching Postgres's spelling, or `Value::Null` when the column has no
/// default. `now()` / `gen_random_uuid()` / `nextval('seq'::regclass)` render as
/// the function call; a string literal carries the **column's** type name as its cast
/// suffix (`::text`, `::character varying`, or the enum's own name); a numeric/boolean
/// literal renders bare unless `get_const_expr` would quote it (so `DEFAULT 1` is
/// exactly `1` and `DEFAULT -1` is `'-1'::integer`) — see [`render_default_literal`].
/// A general expression goes through `crate::deparse`'s catalog mode, whose rules are
/// `docs/deparse.md`.
///
/// **Each `now`-family spelling reports itself** — `now()`,
/// `transaction_timestamp()`, `statement_timestamp()`, `clock_timestamp()` and the
/// bare keyword `CURRENT_TIMESTAMP` — because 18.6's `pg_get_expr` preserves the
/// written form (measured: five columns, five different strings). Reporting `now()`
/// for all five told an introspecting client that a column declared
/// `DEFAULT CURRENT_TIMESTAMP` carried a different default than it does, and hid
/// which of them is volatile.
///
/// **Known divergence in the `NextVal` arm.** The sequence name is interpolated raw,
/// where Postgres's `pg_get_expr` renders the *regclass literal* — an identifier quoted
/// when it needs to be, inside the string. So a sequence called `Odd_id_seq` reports
/// `nextval('Odd_id_seq'::regclass)` here and `nextval('"Odd_id_seq"'::regclass)` on
/// 18.6 (measured), and a name containing a `'` renders as text that is not valid SQL
/// at all. This is introspection output, not a reload path — nothing consumes it as SQL
/// — so it is recorded rather than repaired alongside the emitter's own two literal
/// name positions, which do quote. Note it is **this arm's** alone: the `Expr` arm's
/// inner `nextval` does carry the `::regclass` suffix, `crate::deparse`'s catalog mode
/// putting it there.
///
/// The `Expr` arm's own residual divergences are the deparser's, listed in
/// `docs/deparse.md` § What is recorded, not reproduced — a nested implicit coercion,
/// `CASE`'s indentation, an output function's normalisation, and the parser's `+5`
/// fold — not a set this function owns.
fn render_column_default(col: &Column, schema: &Schema) -> Value {
    // An identity column's sequence draw is not a `column_default` on 18.6 (measured:
    // NULL, with `is_identity = 'YES'`), even though the engine resolves it the same way
    // a SERIAL's is. Reported here, not filtered at the call site, so the `nextval`
    // spelling has exactly one gate.
    if col.identity.is_some() {
        return Value::Null;
    }
    let Some(default) = &col.default else {
        return Value::Null;
    };
    let text = match default {
        ColumnDefault::Now => "now()".to_string(),
        // Each `now`-family spelling reports itself. 18.6 renders whatever the column
        // was defined with (measured: five columns, five different `column_default`
        // strings), so collapsing them told an introspecting client that a column
        // declared `DEFAULT CURRENT_TIMESTAMP` carries `now()`.
        ColumnDefault::TransactionTimestamp => "transaction_timestamp()".to_string(),
        ColumnDefault::StatementTimestamp => "statement_timestamp()".to_string(),
        ColumnDefault::ClockTimestamp => "clock_timestamp()".to_string(),
        ColumnDefault::CurrentTimestamp => "CURRENT_TIMESTAMP".to_string(),
        ColumnDefault::CurrentDate => "CURRENT_DATE".to_string(),
        ColumnDefault::CurrentTime => "CURRENT_TIME".to_string(),
        ColumnDefault::RandomUuid => "gen_random_uuid()".to_string(),
        ColumnDefault::NextVal(seq) => format!("nextval('{seq}'::regclass)"),
        ColumnDefault::Literal(value) => render_default_literal(value, col),
        // A general expression goes through the **one** `Expr` deparser
        // (`crate::deparse`), the same one `pglet::dump` emits with — two renderings
        // of one persisted shape is the drift seam this engine keeps paying for, and
        // this one would be quiet: nothing compares a dump's spelling with an
        // introspecting client's.
        //
        // An unrenderable tree reports **SQL NULL**, and the choice is between two
        // wrongs rather than between right and wrong: this view has no error channel
        // (a `column_default` cell is a `Value`), so the alternatives are "no default"
        // and a fabricated string.
        //
        // **What makes it unreached is the render probe in `executor::ddl`'s
        // `cook_column_default`, not the node walk beside it.**
        // `check_default_expr_nodes` refuses the constructs *18.6* forbids in a
        // `DEFAULT`, a different set from the ones this deparser cannot spell, so the
        // cook runs `crate::deparse::render_expr` itself as its own over-rejection —
        // last, because 18.6 raises nothing here. That probe used to be skipped
        // whenever the expression's *evaluation* failed with a deferred code, and one
        // statement then reached this arm: over `DEFAULT now() + interval '1 day' *
        // (1/0)`, accepted at DDL, this cell read SQL NULL for a column that has a
        // default. Loud on the dump side of the same tree, silent here — which is why
        // the guard has to be at the cook and not at either reader.
        ColumnDefault::Expr(expr) => {
            // A `DEFAULT` names no column (both engines refuse a column reference in
            // one), so the scope is the empty catalog scope: it exists to make the
            // rendering a *catalog* one, not to resolve a `Column` leaf.
            let scope = PlannerScope::none(schema);
            match render_catalog_expr(expr, &scope) {
                Ok(sql) => sql,
                Err(_) => return Value::Null,
            }
        }
    };
    Value::Text(text)
}

/// Render a literal `DEFAULT` value in 18.6's `column_default` spelling — which is
/// `get_const_expr(showtype = 0)` over the `Const` **18.6's parse analysis would have
/// built**, not over the value pglet coerced and stored.
///
/// Every arm delegates to `crate::deparse`'s one implementation of the rule
/// (`render_int_const`, `render_numeric_const`, `render_catalog_string_const`); what
/// stays here is the `Value` → position mapping, which is this surface's alone.
///
/// The two `Const`s differ in exactly the places this function has to reconstruct, each
/// measured on 18.6:
///
/// - **The number's own type is `int4` or `numeric`, whatever the column is.**
///   `DEFAULT -1` on an `int2`, an `int4` and an `int8` column all read
///   `'-1'::integer` there, and `DEFAULT -1.5` on a `float4` and a `float8` column both
///   read `'-1.5'::numeric`: the literal is typed before the assignment cast to the
///   column type, and that cast is the implicit one `pg_get_expr` hides. pglet stored
///   the *coerced* value, so the width has to be put back from the value's magnitude
///   rather than read off its `Value` variant.
/// - **A negative number, and any integer outside `int4`, is quoted and labelled** —
///   `'-1'::integer`, `'2147483648'::bigint` — so the text re-parses as a constant
///   rather than as an operator applied to one (`ruleutils.c` says so for the negative
///   case; the wide case is the `default:` arm, which quotes whatever the sign).
/// - **A string literal is folded and labelled by the column's own type**, the literal
///   having been `unknown` until `coerce_to_target_type` typed it: `::text` for a
///   `text` column, `::character varying` for a `varchar(n)` one, the enum type's name
///   for an enum column (the spine's `DEFAULT 'private'` on a `cell_visibility` column
///   reads `'private'::cell_visibility`) — and, where `get_const_expr` folds through
///   the type's output function, no label at all (`bool DEFAULT 't'` is `true`,
///   `int DEFAULT '007'` is `7`). That fold lives in **one** place,
///   `crate::deparse::render_catalog_string_const`, which this arm calls: the copy
///   that used to stand here labelled without folding, so all three of `bool`, `int4`
///   and `numeric` diverged at this position while the deparser's own copy agreed.
fn render_default_literal(value: &Value, col: &Column) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Int2(n) => crate::deparse::render_int_const(i64::from(*n)),
        Value::Int4(n) => crate::deparse::render_int_const(i64::from(*n)),
        Value::Int8(n) => crate::deparse::render_int_const(*n),
        Value::Float4(n) => crate::deparse::render_numeric_const(&n.to_string()),
        Value::Float8(n) => crate::deparse::render_numeric_const(&n.to_string()),
        Value::Numeric(n) => crate::deparse::render_numeric_const(&n.to_display_string()),
        // The **one** string-constant rule, reached from this position too. It labels
        // with the column's own type — `coerce_to_target_type` is what typed the
        // unknown — and folds through that type's output function where
        // `get_const_expr` does, which is what makes `bool DEFAULT 't'` read `true` and
        // `int DEFAULT '007'` read `7`.
        Value::Text(s) => crate::deparse::render_catalog_string_const(
            s,
            Some(col.type_oid),
            col.enum_type.as_deref(),
        ),
        // Not reached: the parser reduces a `DEFAULT` whose cooked expression is a
        // NULL constant to *no default* (18.6 records none either), and the store
        // magic moved to `PGMI`, so no legacy store carries the shape. Kept so the
        // match stays total over the shapes a `ColumnDefault::Literal` can hold.
        Value::Null => "NULL".to_string(),
        other => format!("'{}'", format!("{other:?}").replace('\'', "''")),
    }
}

/// Build one `information_schema.table_constraints` row. `is_deferrable` /
/// `initially_deferred` are always `NO` (pglet constraints are immediate).
fn constraint_row(table: &Table, name: &str, constraint_type: &str) -> Vec<Value> {
    vec![
        Value::Text("pglet".into()),              // constraint_catalog
        Value::Text(table.schema.clone()),        // constraint_schema
        Value::Text(name.to_string()),            // constraint_name
        Value::Text("pglet".into()),              // table_catalog
        Value::Text(table.schema.clone()),        // table_schema
        Value::Text(table.name.clone()),          // table_name
        Value::Text(constraint_type.to_string()), // constraint_type
        Value::Text("NO".into()),                 // is_deferrable
        Value::Text("NO".into()),                 // initially_deferred
    ]
}

/// The `information_schema.columns.data_type` for a column: 18.6's `USER-DEFINED` for an
/// enum column, else the canonical name of its `TypeOid`.
const fn column_data_type(col: &Column) -> &'static str {
    if col.enum_type.is_some() {
        return "USER-DEFINED";
    }
    information_schema_data_type(col.type_oid)
}

/// The `information_schema.columns.udt_name` for a column — `pg_type.typname` for its
/// type (`int4`, `varchar`, `_text`), or the **enum type's own name** for an enum column,
/// which is what makes an enum distinguishable from the `text` it is stored as.
fn udt_name(col: &Column) -> String {
    if let Some(name) = &col.enum_type {
        return name.clone();
    }
    crate::types::type_oid_pg_name(col.type_oid).to_string()
}

/// `information_schema.columns.character_maximum_length` — the declared `n` of a
/// `varchar(n)` / `char(n)` column, and SQL NULL for everything else.
///
/// **An array column reports NULL even though it has a cap**, which is 18.6's answer
/// and not an omission: `Column::varchar_length` holds the *element's* modifier, so a
/// `varchar(3)[]` column carries `3` while `information_schema` describes the
/// **column's** type, whose character length no `_varchar` has (measured on 18.6:
/// NULL, beside the `atttypmod` of `7` the same column's `pg_attribute` row reports).
/// The same split is why `numeric_precision` / `numeric_scale` below key on the scalar
/// `TypeOid::Numeric` rather than on the field being set.
fn character_maximum_length(col: &Column) -> Value {
    if col.type_oid != TypeOid::Varchar {
        return Value::Null;
    }
    col.varchar_length.map_or(Value::Null, |n| {
        Value::Int4(i32::try_from(n).unwrap_or(i32::MAX))
    })
}

/// `information_schema.columns.numeric_precision` — the significant-digit budget: the
/// bit width for an integer, the mantissa bits for a float, the declared `p` for a
/// `numeric(p,s)`, and SQL NULL for an unconstrained `numeric` and every non-numeric
/// type (each measured on 18.6).
fn numeric_precision(col: &Column) -> Value {
    match col.type_oid {
        TypeOid::Int2 => Value::Int4(16),
        TypeOid::Int4 => Value::Int4(32),
        TypeOid::Int8 => Value::Int4(64),
        TypeOid::Float4 => Value::Int4(24),
        TypeOid::Float8 => Value::Int4(53),
        TypeOid::Numeric => col
            .numeric_typmod
            .map_or(Value::Null, |t| Value::Int4(i32::from(t.precision))),
        _ => Value::Null,
    }
}

/// `information_schema.columns.numeric_precision_radix` — 2 for the binary types, 10 for
/// `numeric` (including an unconstrained one, whose precision is NULL while its radix is
/// not), NULL otherwise.
const fn numeric_precision_radix(col: &Column) -> Value {
    match col.type_oid {
        TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8 | TypeOid::Float4 | TypeOid::Float8 => {
            Value::Int4(2)
        }
        TypeOid::Numeric => Value::Int4(10),
        _ => Value::Null,
    }
}

/// `information_schema.columns.numeric_scale` — 0 for an integer, the declared `s` for a
/// `numeric(p,s)`, and NULL for a float, an unconstrained `numeric`, and every
/// non-numeric type.
fn numeric_scale(col: &Column) -> Value {
    match col.type_oid {
        TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8 => Value::Int4(0),
        TypeOid::Numeric => col
            .numeric_typmod
            .map_or(Value::Null, |t| Value::Int4(i32::from(t.scale))),
        _ => Value::Null,
    }
}

/// `information_schema.columns.datetime_precision` — the sub-second precision of a
/// temporal column, and SQL NULL for everything else.
///
/// **It is a fact about the *type*, not about whether a modifier was spelled.** A
/// bare `timestamp` reports `6` (its full precision) where its `atttypmod` is `-1`,
/// and a `date`, which carries no modifier at all, reports `0` — both measured on
/// 18.6, beside NULL at every non-temporal column.
///
/// **An array column reports NULL**, the same split
/// [`character_maximum_length`] takes: `Column::temporal_precision` holds the
/// *element's* modifier (a `timestamp(3)[]` column carries `3`, and its
/// `atttypmod` is `3`), while `information_schema` describes the **column's** type,
/// whose datetime precision no `_timestamp` has (measured: NULL for both
/// `timestamp(3)[]` and `interval(3)[]`). Gating on the scalar `TypeOid` is what
/// makes that fall out.
fn datetime_precision(col: &Column) -> Value {
    match col.type_oid {
        // `date` has no sub-second field at all, and 18.6 reports `0` rather than
        // NULL for it.
        TypeOid::Date => Value::Int4(0),
        TypeOid::Timestamp | TypeOid::Timestamptz | TypeOid::Time | TypeOid::Interval => {
            Value::Int4(
                col.temporal_precision
                    .unwrap_or(pglet_core::types::MAX_TEMPORAL_PRECISION)
                    .into(),
            )
        }
        _ => Value::Null,
    }
}

/// The canonical `information_schema.columns.data_type` name for a `TypeOid`
/// (e.g. `int4` → `integer`, `timestamptz` → `timestamp with time zone`). A
/// one-dimensional array reports `ARRAY`; Postgres records the element type
/// separately (in `element_types`), which pglet does not model. Exhaustive (no
/// wildcard) so a future `TypeOid` forces a mapping rather than silently
/// mislabeling.
const fn information_schema_data_type(type_oid: TypeOid) -> &'static str {
    match type_oid {
        TypeOid::Bool => "boolean",
        TypeOid::Int2 => "smallint",
        TypeOid::Int4 => "integer",
        TypeOid::Int8 => "bigint",
        TypeOid::Float4 => "real",
        TypeOid::Float8 => "double precision",
        TypeOid::Text => "text",
        TypeOid::Varchar => "character varying",
        TypeOid::Bytea => "bytea",
        TypeOid::Timestamp => "timestamp without time zone",
        TypeOid::Timestamptz => "timestamp with time zone",
        TypeOid::Date => "date",
        TypeOid::Time => "time without time zone",
        TypeOid::Numeric => "numeric",
        TypeOid::Interval => "interval",
        TypeOid::Jsonb => "jsonb",
        TypeOid::Uuid => "uuid",
        TypeOid::BoolArray
        | TypeOid::ByteaArray
        | TypeOid::Int2Array
        | TypeOid::Int4Array
        | TypeOid::TextArray
        | TypeOid::VarcharArray
        | TypeOid::Int8Array
        | TypeOid::Float4Array
        | TypeOid::Float8Array
        | TypeOid::TimestampArray
        | TypeOid::TimestamptzArray
        | TypeOid::DateArray
        | TypeOid::TimeArray
        | TypeOid::IntervalArray
        | TypeOid::NumericArray
        | TypeOid::UuidArray
        | TypeOid::JsonbArray
        | TypeOid::NameArray => "ARRAY",
        // Unreachable through a user column: `name` / `oid` / `"char"` / `void` have no
        // DDL spelling (`type_oid_from_name` maps none of them), so no `CREATE TABLE` can
        // declare one. Spelled rather than folded into a wildcard so they stay honest
        // if a catalog column is ever declared with one here.
        TypeOid::Name => "name",
        TypeOid::Oid => "oid",
        TypeOid::Char => "\"char\"",
        TypeOid::Void => "void",
    }
}
