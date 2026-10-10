//! Virtual `pg_catalog` table data generation.
//!
//! This module generates row data for `pg_catalog` virtual tables (`pg_class`,
//! `pg_attribute`, `pg_type`, `pg_namespace`). The planner creates `Plan::VirtualScan`
//! nodes for these tables, and the executor calls methods here to get the rows.
//!
//! Column schemas are defined in `virtual_tables.rs`.
//!
//! TODO: Additional `pg_catalog` tables not yet implemented:
//! - `pg_attrdef`, `pg_am`, `pg_opclass`
//! - `pg_roles`, `pg_policies`, `pg_sequences`, `pg_stat_user_indexes`, `pg_user`

use super::constraints_view::{
    ConstraintEntryKind, SYNTHETIC_ENUM_OID_BASE, check_referenced_columns, constraint_entries,
    referential_action_code,
};
use super::{ConstraintKind, IdentityGeneration, Index, IndexKey, Schema, Table};
use crate::deparse::render_catalog_expr;
use crate::executor::{IndexMethod, index_method_for};
use crate::lexer::quote_ident;
use crate::planner::PlannerScope;
use crate::types::{TypeOid, Value, type_oid_pg_name};

/// Virtual `pg_catalog` tables for Postgres compatibility.
///
/// These don't exist as real tables - we generate results on the fly
/// based on the actual schema.
#[derive(Debug)]
pub struct PgCatalog<'a> {
    schema: &'a Schema,
}

impl<'a> PgCatalog<'a> {
    /// Creates a new `PgCatalog` view over the given schema.
    pub const fn new(schema: &'a Schema) -> Self {
        Self { schema }
    }

    /// Query `pg_namespace` (schemas).
    ///
    /// Two rows, with 18.6's own OIDs: `pg_catalog` (11) and `public` (2200). The
    /// system schema is here because `pg_type` needs somewhere truthful to put a
    /// built-in type: reporting `int4` as living in `public` made
    /// `pg_type ⨝ pg_namespace WHERE nspname = 'public'` — the shape the fuz spine's
    /// enum snapshot takes — answer every built-in type where 18.6 answers only the
    /// user enums. 18.6 also carries `pg_toast` and `information_schema`, whose OID is
    /// per-cluster; neither is modelled (`docs/compatibility.md`'s pg_catalog row).
    pub fn pg_namespace(&self) -> Vec<Vec<Value>> {
        vec![
            vec![
                Value::Int4(PG_CATALOG_NAMESPACE), // oid
                Value::Text("pg_catalog".into()),  // nspname
                Value::Int4(10),                   // nspowner
            ],
            vec![
                Value::Int4(PUBLIC_NAMESPACE), // oid
                Value::Text("public".into()),  // nspname
                Value::Int4(10),               // nspowner
            ],
        ]
    }

    /// Query `pg_class` (relations).
    ///
    /// One row per ordinary table (`relkind = 'r'`) and one per index (`'i'`) — an
    /// index's OID is the catalog `Index.oid`, a real persisted fact, which is what
    /// makes `JOIN pg_class ic ON ic.oid = i.indexrelid` and a `pg_constraint`'s
    /// `conindid` resolvable.
    ///
    /// **A sequence gets no row**, where 18.6 emits `relkind = 'S'`. pglet's `Sequence`
    /// record carries no OID and none can be derived without either colliding with a
    /// relation's or changing under DDL, so the row is omitted rather than invented —
    /// the same call `information_schema.sequences` does *not* have to make, because
    /// that view exposes no OID. Recorded in `EDGES.md`.
    pub fn pg_class(&self) -> Vec<Vec<Value>> {
        let mut rows: Vec<Vec<Value>> = self
            .schema
            .tables
            .values()
            .map(|t| {
                let has_indexes = self.schema.indexes.values().any(|i| i.table_name == t.name);
                vec![
                    Value::Int4(t.oid),                                              // oid
                    Value::Text(t.name.clone()),                                     // relname
                    Value::Int4(PUBLIC_NAMESPACE),                                   // relnamespace
                    Value::Text("r".into()),                                         // relkind
                    Value::Bool(has_indexes),                                        // relhasindex
                    Value::Int2(i16::try_from(t.columns.len()).unwrap_or(i16::MAX)), // relnatts
                    Value::Int2(i16::try_from(t.checks.len()).unwrap_or(i16::MAX)),  // relchecks
                    Value::Text("p".into()), // relpersistence
                    Value::Bool(false),      // relrowsecurity
                ]
            })
            .collect();
        rows.extend(self.schema.indexes.values().map(|idx| {
            vec![
                Value::Int4(idx.oid),                                          // oid
                Value::Text(idx.name.clone()),                                 // relname
                Value::Int4(PUBLIC_NAMESPACE),                                 // relnamespace
                Value::Text("i".into()),                                       // relkind
                Value::Bool(false),                                            // relhasindex
                Value::Int2(i16::try_from(idx.key.len()).unwrap_or(i16::MAX)), // relnatts
                Value::Int2(0),                                                // relchecks
                Value::Text("p".into()),                                       // relpersistence
                Value::Bool(false),                                            // relrowsecurity
            ]
        }));
        rows
    }

    /// Query `pg_attribute` (columns).
    ///
    /// `attlen` is `pg_type.typlen` for the column's type; `atttypmod` is the same
    /// `Column::pg_type_modifier` the `RowDescription` carries (so a `varchar(8)` is 12
    /// and a `numeric(10,2)` is 655366, both measured against 18.6). `attidentity` is
    /// `a` / `d` / the empty `"char"` (18.6 stores `\0` there, which renders as an empty
    /// string — *not* the space a non-FK `confdeltype` carries; the two "none" codes are
    /// different bytes, measured). `attgenerated` is always empty and `attisdropped`
    /// always false: pglet has no generated columns, and a dropped column leaves no
    /// catalog entry here.
    pub fn pg_attribute(&self) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        for table in self.schema.tables.values() {
            for col in &table.columns {
                let identity = match col.identity {
                    Some(IdentityGeneration::Always) => "a",
                    Some(IdentityGeneration::ByDefault) => "d",
                    None => "",
                };
                // An **enum** column reports its enum type's OID and 18.6's `typlen` for
                // an enum (4), not the `text` pglet stores it as — so
                // `pg_attribute ⨝ pg_type` lands on the `pg_type` row [`Self::pg_type`]
                // emits for that enum. Reporting `text`/`-1` here made the join resolve
                // to `text` and told an introspecting client the column was one.
                let (atttypid, attlen) = col
                    .enum_type
                    .as_ref()
                    .and_then(|name| self.schema.enums.get(name))
                    .map_or_else(
                        || (col.type_oid as i32, col.type_oid.wire_size()),
                        |ty| (ty.oid, 4),
                    );
                rows.push(vec![
                    Value::Int4(table.oid),                                       // attrelid
                    Value::Text(col.name.clone()),                                // attname
                    Value::Int4(atttypid),                                        // atttypid
                    Value::Int2(attlen),                                          // attlen
                    Value::Int2(col.position),                                    // attnum
                    Value::Int4(col.pg_type_modifier()),                          // atttypmod
                    Value::Int2(i16::from(col.type_oid.is_array())),              // attndims
                    Value::Bool(!col.nullable),                                   // attnotnull
                    Value::Bool(col.default.is_some() && col.identity.is_none()), // atthasdef
                    Value::Text(identity.into()),                                 // attidentity
                    Value::Text(String::new()),                                   // attgenerated
                    Value::Bool(false),                                           // attisdropped
                ]);
            }
        }
        rows
    }

    /// Query `pg_tables` — one row per user table.
    ///
    /// `tableowner` is a fixed `pglet` (pglet has no roles); `tablespace` is NULL
    /// (the default tablespace); `hasindexes` reflects whether any index targets
    /// the table; `hasrules` / `rowsecurity` are always `false` (pglet has neither
    /// feature).
    ///
    /// **`hastriggers` is computed from the FK set, on both sides.** 18.6 installs
    /// system triggers on the referencing *and* the referenced table, so both read `t`
    /// there while a table with no FK either way reads `f` (measured). pglet has no user
    /// triggers, so an FK is the only thing that can make this true and the derivation is
    /// exact rather than approximate. It used to be a flat `false` — tolerable while this
    /// relation was registered non-exhaustively, and not once `columns_complete` made it
    /// a promise that every column it declares is answered.
    pub fn pg_tables(&self) -> Vec<Vec<Value>> {
        self.schema
            .tables
            .values()
            .map(|t| {
                let has_indexes = self.schema.indexes.values().any(|i| i.table_name == t.name);
                // Either side of any foreign key — the child that declares one, or the
                // parent it references.
                let has_triggers = !t.foreign_keys.is_empty()
                    || self
                        .schema
                        .tables
                        .values()
                        .any(|o| o.foreign_keys.iter().any(|fk| fk.parent_table == t.name));
                vec![
                    Value::Text(t.schema.clone()), // schemaname
                    Value::Text(t.name.clone()),   // tablename
                    Value::Text("pglet".into()),   // tableowner
                    Value::Null,                   // tablespace
                    Value::Bool(has_indexes),      // hasindexes
                    Value::Bool(false),            // hasrules
                    Value::Bool(has_triggers),     // hastriggers
                    Value::Bool(false),            // rowsecurity
                ]
            })
            .collect()
    }

    /// Query `pg_indexes` — one row per index, including a `PRIMARY KEY` /
    /// `UNIQUE` constraint's own index, which is listed under the constraint's name
    /// (the declared `CONSTRAINT <n>`, else `<table>_pkey` / `<table>_<col>_key`).
    ///
    /// `schemaname` is `public` (pglet's only schema); `tablespace` is NULL.
    /// `indexdef` re-emits the index the way 18.6's `pg_get_indexdef` does — including
    /// an **expression** key (deparsed from the stored `Expr`) and a **partial** index's
    /// `WHERE (…)` suffix. Both used to be lost: an expression key rendered
    /// `USING btree (...)` and a predicate was dropped entirely, so
    /// `CREATE INDEX i ON t (qty) WHERE qty > 10` read back as a plain index on `qty` —
    /// a different, existing-looking index. That is what the fuz spine's schema
    /// snapshot stores verbatim as its cross-backend diff value, so the loss was
    /// silent drift rather than a visible gap.
    pub fn pg_indexes(&self) -> Vec<Vec<Value>> {
        self.schema
            .indexes
            .values()
            .map(|idx| {
                vec![
                    Value::Text("public".into()),        // schemaname
                    Value::Text(idx.table_name.clone()), // tablename
                    Value::Text(idx.name.clone()),       // indexname
                    Value::Null,                         // tablespace
                    Value::Text(self.index_def(idx)),    // indexdef
                ]
            })
            .collect()
    }

    /// Synthesize `pg_indexes.indexdef` for `index`, in 18.6's `pg_get_indexdef`
    /// spelling.
    ///
    /// An expression key part is deparsed through the **one** `Expr` deparser
    /// (`crate::deparse`), the same one the dump and `pg_get_constraintdef` use, so
    /// where that deparser diverges from 18.6's this diverges with it
    /// (`docs/deparse.md` § What is recorded, not reproduced) rather than in a way of
    /// its own. A part the deparser
    /// cannot render falls back to `(...)` — this view has no error channel, and a
    /// stored key that reaches here has already passed the DDL-time render probe, so
    /// the fallback stands for a shape no accepted `CREATE INDEX` produces.
    ///
    /// The key list and the predicate are both **catalog** renderings
    /// (`crate::deparse`'s `RenderMode::Catalog`, over this index's own relation), so an
    /// *unknown* literal in the predicate resolves against the column it is compared to
    /// — `WHERE moderation = 'pending'` reads back `WHERE (moderation = 'pending'::text)`
    /// as 18.6 spells it. `docs/deparse.md`.
    ///
    /// 18.6 wraps a key part in parens of its own unless the node is "simple"
    /// (`ruleutils.c`'s `isSimpleNode`): measured, `lower(label)` renders
    /// `USING btree (lower(label))` while `qty + 1` renders
    /// `USING btree (((qty + 1)))` — the key-list parens, 18.6's wrap, and the
    /// expression's own. [`render_index_key_expr`] reproduces that by asking the
    /// **deparser's own output** whether it is already parenthesized, rather than
    /// re-listing which variants get parens: a second list of that rule beside
    /// `deparse`'s would be one more pair that can drift, and this one would drift
    /// silently (both renderings are valid SQL).
    fn index_def(&self, index: &Index) -> String {
        let unique = if index.is_unique() { "UNIQUE " } else { "" };
        let method = self.index_method_name(index);
        let key = match &index.key {
            // Through `quote_ident`: `position` is a keyword on both engines, so 18.6
            // prints `(parent_id, "position")` here — and pglet's own
            // `pg_get_constraintdef` already printed the quoted form for the same
            // index's PRIMARY KEY, so this list was one name position that skipped the
            // shared predicate. **Not every name position in this rendering**: the
            // `format!` below interpolates the index's own name and its table's raw,
            // where 18.6 renders both through `quote_identifier` — `EDGES.md` row 122,
            // measured, recorded rather than fixed. And the predicate's keyword half is
            // not 18.6's in either direction (row 85 (f)), which is why a column named
            // `action` reads `"action"` here against 18.6's bare `action`.
            IndexKey::Columns(cols) => cols
                .iter()
                .map(|c| quote_ident(c))
                .collect::<Vec<_>>()
                .join(", "),
            IndexKey::Exprs(exprs) => {
                let scope = PlannerScope::catalog(self.schema, Some(index.table_name.as_str()));
                exprs
                    .iter()
                    .map(|e| render_index_key_expr(e, &scope))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        let mut out = format!(
            "CREATE {unique}INDEX {} ON public.{} USING {method} ({key})",
            index.name, index.table_name
        );
        if let Some(pred) = &index.predicate {
            // A catalog rendering, over the index's own relation: an *unknown* literal
            // in the predicate resolves against the column it is compared to, which is
            // what makes `WHERE moderation = 'pending'` read back
            // `WHERE (moderation = 'pending'::text)` as it does on 18.6.
            let scope = PlannerScope::catalog(self.schema, Some(index.table_name.as_str()));
            if let Ok(sql) = render_catalog_expr(pred, &scope) {
                out.push_str(" WHERE ");
                out.push_str(&sql);
            }
        }
        out
    }

    /// The access-method name (`gin` / `btree`) for `index`, derived from its key
    /// shape and uniqueness as well as its key column's type. Delegates to the
    /// executor's [`index_method_for`] (via [`IndexMethod::name`]) so the rendered
    /// `indexdef` can't drift from the method storage actually builds — which is what
    /// makes a *unique* index over a jsonb or array column render `USING btree` here,
    /// GIN being unable to enforce uniqueness. An unknown table falls back to B-tree
    /// (the default).
    fn index_method_name(&self, index: &Index) -> &'static str {
        self.schema
            .get_table(&index.table_name)
            .map_or(IndexMethod::Btree, |t| index_method_for(index, t))
            .name()
    }

    /// Query `pg_type` (data types).
    ///
    /// One row per scalar type pglet models, one per **array** type (derived from the
    /// scalar's own `array_of`, so the two can never disagree about `typelem` /
    /// `typarray`), and one per user `ENUM`. The list used to be fourteen hand-written
    /// scalars — `numeric`, `time`, `interval` and `name` were absent though `TypeOid`
    /// has them, and so was every array type — which made
    /// `pg_attribute ⨝ pg_type` **drop the row** for a `numeric` column: a join that
    /// silently loses a column is worse than one that reports it wrong.
    ///
    /// The derivations are 18.6's, each measured over its whole `pg_type`:
    /// `typlen` is [`TypeOid::wire_size`] for every one of them; `typbyval` is
    /// "fixed-width and no wider than a machine word" (so `name`(64), `uuid`(16) and
    /// `interval`(16) are false while every 1–8-byte type is true); `typtype` is `b` for
    /// every base type and `e` for an enum. `typcategory` is the one fact with no
    /// derivation and is carried per type.
    ///
    /// A **built-in** type reports `typnamespace = pg_catalog`, a user enum `public` —
    /// which is the whole reason [`Self::pg_namespace`] carries two rows.
    pub fn pg_type(&self) -> Vec<Vec<Value>> {
        // (scalar type, `typcategory`). The array type is derived from `array_of` below,
        // except for the three whose array type pglet has no `TypeOid` for: 18.6's
        // `char`, `oid` and `name` do have one (`_char` 1002, `_oid` 1028, `_name` 1003)
        // and `name`'s `typelem` is `char` (18) — all four are facts about Postgres's
        // type system, so reporting a `0` for them would be *inventing* "there is no
        // array type" where the truth is known. The array row itself stays absent, which
        // is `pg_type`'s ordinary partial-relation gap.
        const OVERRIDES: [(TypeOid, i32, i32); 3] = [
            (TypeOid::Char, 0, 1002),
            (TypeOid::Name, 18, 1003),
            (TypeOid::Oid, 0, 1028),
        ];
        const SCALARS: [(TypeOid, &str); 20] = [
            (TypeOid::Bool, "B"),
            (TypeOid::Bytea, "U"),
            (TypeOid::Char, "Z"),
            (TypeOid::Name, "S"),
            (TypeOid::Int8, "N"),
            (TypeOid::Int2, "N"),
            (TypeOid::Int4, "N"),
            (TypeOid::Text, "S"),
            (TypeOid::Oid, "N"),
            (TypeOid::Float4, "N"),
            (TypeOid::Float8, "N"),
            (TypeOid::Varchar, "S"),
            (TypeOid::Date, "D"),
            (TypeOid::Time, "D"),
            (TypeOid::Timestamp, "D"),
            (TypeOid::Timestamptz, "D"),
            (TypeOid::Interval, "T"),
            (TypeOid::Numeric, "N"),
            (TypeOid::Uuid, "U"),
            (TypeOid::Jsonb, "U"),
        ];
        let mut rows: Vec<Vec<Value>> = Vec::new();
        for (oid, category) in SCALARS {
            let (typelem, typarray) = OVERRIDES
                .iter()
                .find(|(t, _, _)| *t as u32 == oid as u32)
                .map_or_else(
                    || (0, oid.array_of().map_or(0, |a| a as i32)),
                    |(_, elem, array)| (*elem, *array),
                );
            rows.push(type_row(
                oid as i32,
                type_oid_pg_name(oid),
                PG_CATALOG_NAMESPACE,
                oid.wire_size(),
                category,
                "b",
                typelem,
                typarray,
            ));
            if let Some(array) = oid.array_of() {
                rows.push(type_row(
                    array as i32,
                    type_oid_pg_name(array),
                    PG_CATALOG_NAMESPACE,
                    array.wire_size(),
                    "A",
                    "b",
                    oid as i32,
                    0,
                ));
            }
        }
        // User `ENUM`s, in the `public` namespace where 18.6 puts them. `typlen` 4 and
        // `typbyval` true are 18.6's representation (an enum value is an OID there),
        // reported as measured even though pglet stores the label as text — the pair
        // `pg_attribute` reports for an enum column, so the two agree.
        rows.extend(
            self.schema
                .enums
                .iter()
                .map(|(name, ty)| type_row(ty.oid, name, PUBLIC_NAMESPACE, 4, "E", "e", 0, 0)),
        );
        rows
    }

    /// Query `pg_enum` — one row per member label of every user `ENUM` type.
    ///
    /// `enumtypid` is the type's catalog OID, the same fact [`Self::pg_type`] reports as
    /// `oid`, so the fuz spine's `pg_type ⨝ pg_enum` join resolves. `enumsortorder` is
    /// 18.6's `float4` 1-based position and `enumlabel` the label; the row's own `oid`
    /// is synthesized in a band of its **own** ([`SYNTHETIC_ENUM_OID_BASE`], distinct
    /// from the constraint band so no label shares an OID with a constraint, as 18.6
    /// guarantees) and is what `EDGES.md` records alongside `pg_constraint.oid`.
    pub fn pg_enum(&self) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        let mut next = SYNTHETIC_ENUM_OID_BASE;
        for ty in self.schema.enums.values() {
            for (i, label) in ty.labels.iter().enumerate() {
                rows.push(vec![
                    Value::Int4(next),             // oid
                    Value::Int4(ty.oid),           // enumtypid
                    Value::Float4((i + 1) as f32), // enumsortorder
                    Value::Text(label.clone()),    // enumlabel
                ]);
                next = next.saturating_add(1);
            }
        }
        rows
    }

    /// Query `pg_constraint` — one row per NOT NULL / CHECK / FOREIGN KEY / PRIMARY KEY
    /// / UNIQUE constraint.
    ///
    /// The rows come from the one catalog-wide enumeration
    /// ([`constraint_entries`](super::constraint_entries)) that `pg_get_constraintdef`
    /// resolves an OID against, so a row's `oid` and the definition that OID renders
    /// cannot disagree.
    ///
    /// **`NOT NULL` is `contype = 'n'`, a kind of its own** — not a CHECK.
    /// `information_schema.table_constraints` / `check_constraints` are the views that
    /// relabel it, and the fuz spine's snapshot filters `contype <> 'n'` on both legs
    /// precisely because engines disagree about whether to catalog it at all.
    ///
    /// The constant columns are 18.6's values for a pglet-shaped schema, each measured:
    /// `condeferrable` / `condeferred` / `conperiod` false (no deferral, no temporal
    /// keys), `conenforced` / `convalidated` / `conislocal` true, `coninhcount` 0 and
    /// `contypid` / `conparentid` 0 (no inheritance, no partitions, no domain
    /// constraints). `confupdtype` / `confdeltype` / `confmatchtype` are a **space** for
    /// a non-FK row — 18.6's own "no value" byte there, and deliberately not the empty
    /// string `attidentity` uses.
    pub fn pg_constraint(&self) -> Vec<Vec<Value>> {
        constraint_entries(self.schema)
            .iter()
            .map(|entry| {
                let (conindid, confrelid, conkey, confkey, upd, del, matchtype) = match entry.kind {
                    ConstraintEntryKind::NotNull(col) => (
                        0,
                        0,
                        int2_array(&[col.position]),
                        Value::Null,
                        " ",
                        " ",
                        " ",
                    ),
                    ConstraintEntryKind::Check(check) => (
                        0,
                        0,
                        int2_array(
                            &check_referenced_columns(entry.table, check)
                                .iter()
                                .map(|c| c.position)
                                .collect::<Vec<_>>(),
                        ),
                        Value::Null,
                        " ",
                        " ",
                        " ",
                    ),
                    ConstraintEntryKind::Index(index, _) => (
                        index.oid,
                        0,
                        int2_array(&Self::key_positions(entry.table, index.key.bare_columns())),
                        Value::Null,
                        " ",
                        " ",
                        " ",
                    ),
                    ConstraintEntryKind::ForeignKey(fk) => {
                        let parent = self.schema.get_table(&fk.parent_table);
                        let referenced = self
                            .schema
                            .covering_unique_index(&fk.parent_table, &fk.parent_columns);
                        (
                            referenced.map_or(0, |i| i.oid),
                            parent.map_or(0, |t| t.oid),
                            int2_array(&Self::key_positions(entry.table, Some(&fk.columns))),
                            parent.map_or(Value::Null, |t| {
                                int2_array(&Self::key_positions(t, Some(&fk.parent_columns)))
                            }),
                            referential_action_code(fk.on_update),
                            referential_action_code(fk.on_delete),
                            // pglet has no `MATCH FULL` / `MATCH PARTIAL`, so every FK
                            // is 18.6's `s` (MATCH SIMPLE).
                            "s",
                        )
                    }
                };
                vec![
                    Value::Int4(entry.oid),                   // oid
                    Value::Text(entry.name()),                // conname
                    Value::Int4(PUBLIC_NAMESPACE),            // connamespace
                    Value::Text(entry.kind.contype().into()), // contype
                    Value::Bool(false),                       // condeferrable
                    Value::Bool(false),                       // condeferred
                    Value::Bool(true),                        // conenforced
                    Value::Bool(true),                        // convalidated
                    Value::Int4(entry.table.oid),             // conrelid
                    Value::Int4(0),                           // contypid
                    Value::Int4(conindid),                    // conindid
                    Value::Int4(0),                           // conparentid
                    Value::Int4(confrelid),                   // confrelid
                    Value::Text(upd.into()),                  // confupdtype
                    Value::Text(del.into()),                  // confdeltype
                    Value::Text(matchtype.into()),            // confmatchtype
                    Value::Bool(true),                        // conislocal
                    Value::Int2(0),                           // coninhcount
                    Value::Bool(entry.kind.noinherit()),      // connoinherit
                    Value::Bool(false),                       // conperiod
                    conkey,                                   // conkey
                    confkey,                                  // confkey
                ]
            })
            .collect()
    }

    /// Query `pg_index` — one row per index.
    ///
    /// `indexrelid` is the catalog `Index.oid` and `indrelid` its table's, both real
    /// persisted facts. `indnatts` / `indnkeyatts` are the key-part count (pglet has no
    /// `INCLUDE` columns, so the two are always equal). The constants are 18.6's:
    /// `indimmediate` / `indisvalid` / `indisready` / `indislive` true (every pglet
    /// index is built synchronously and valid), `indisexclusion` / `indisclustered` /
    /// `indcheckxmin` / `indisreplident` false.
    ///
    /// **`indkey`, `indcollation`, `indclass`, `indoption`, `indexprs` and `indpred` are
    /// not modelled** and are therefore `42703` here (this relation binds exhaustively).
    /// `indkey` is 18.6's `int2vector` — space-separated in text, lower bound 0 in
    /// binary — which is a type pglet has no encoder for, and declaring it `int2[]`
    /// would be wrong on both counts; the other four need an opclass / collation model
    /// or a `pg_node_tree` renderer. What an expression or partial index *is* still
    /// reads faithfully off `pg_indexes.indexdef`.
    pub fn pg_index(&self) -> Vec<Vec<Value>> {
        self.schema
            .indexes
            .values()
            .map(|idx| {
                let natts = i16::try_from(idx.key.len()).unwrap_or(i16::MAX);
                vec![
                    Value::Int4(idx.oid), // indexrelid
                    Value::Int4(self.schema.get_table(&idx.table_name).map_or(0, |t| t.oid)), // indrelid
                    Value::Int2(natts),                  // indnatts
                    Value::Int2(natts),                  // indnkeyatts
                    Value::Bool(idx.is_unique()),        // indisunique
                    Value::Bool(idx.nulls_not_distinct), // indnullsnotdistinct
                    Value::Bool(matches!(
                        idx.constraint_kind(),
                        Some(ConstraintKind::PrimaryKey)
                    )), // indisprimary
                    Value::Bool(false),                  // indisexclusion
                    Value::Bool(true),                   // indimmediate
                    Value::Bool(false),                  // indisclustered
                    Value::Bool(true),                   // indisvalid
                    Value::Bool(false),                  // indcheckxmin
                    Value::Bool(true),                   // indisready
                    Value::Bool(true),                   // indislive
                    Value::Bool(false),                  // indisreplident
                ]
            })
            .collect()
    }

    /// The 1-based column positions of `columns` in `table`, in the given order — a
    /// `conkey` / `confkey` vector. A name the table lacks is skipped (unreachable: a
    /// stored constraint's columns were validated at DDL time).
    fn key_positions(table: &Table, columns: Option<&[String]>) -> Vec<i16> {
        columns
            .unwrap_or(&[])
            .iter()
            .filter_map(|c| {
                table
                    .columns
                    .iter()
                    .find(|col| col.name == *c)
                    .map(|col| col.position)
            })
            .collect()
    }
}

/// One `pg_type` row. `typbyval` is derived from `typlen` — 18.6 passes a fixed-width
/// type of at most a machine word by value and everything else by reference (measured
/// over its whole `pg_type`: `name`(64), `uuid`(16) and `interval`(16) are the
/// fixed-width types that are not) — so the two facts cannot be given inconsistently.
// One row of a ten-column catalog relation; the arguments are its columns.
#[allow(clippy::too_many_arguments)]
fn type_row(
    oid: i32,
    name: &str,
    namespace: i32,
    typlen: i16,
    typcategory: &str,
    typtype: &str,
    typelem: i32,
    typarray: i32,
) -> Vec<Value> {
    vec![
        Value::Int4(oid),                       // oid
        Value::Text(name.to_string()),          // typname
        Value::Int4(namespace),                 // typnamespace
        Value::Int2(typlen),                    // typlen
        Value::Bool(typlen > 0 && typlen <= 8), // typbyval
        Value::Text(typtype.to_string()),       // typtype
        Value::Text(typcategory.to_string()),   // typcategory
        Value::Bool(true),                      // typisdefined
        Value::Int4(typelem),                   // typelem
        Value::Int4(typarray),                  // typarray
    ]
}

/// 18.6's OID for the `pg_catalog` schema.
const PG_CATALOG_NAMESPACE: i32 = 11;

/// 18.6's OID for the `public` schema.
const PUBLIC_NAMESPACE: i32 = 2200;

/// An `int2[]` value from a position vector (`conkey` / `confkey`).
fn int2_array(positions: &[i16]) -> Value {
    Value::Array(crate::types::ArrayValue::new(
        TypeOid::Int2,
        positions.iter().map(|p| Value::Int2(*p)).collect(),
    ))
}

/// One `pg_indexes.indexdef` key part in 18.6's parenthesization.
///
/// A part the deparser already wrapped (every operator node) takes 18.6's extra wrap;
/// one it left bare (a column reference, a function call) does not. Reading the wrap off
/// the rendered text is deliberate — see [`PgCatalog::index_def`]. A part the deparser
/// cannot render falls back to `(...)`, standing for a shape no accepted `CREATE INDEX`
/// produces (the DDL-time render probe refuses it first).
fn render_index_key_expr(expr: &crate::planner::Expr, scope: &PlannerScope<'_>) -> String {
    match render_catalog_expr(expr, scope) {
        Ok(sql) if sql.starts_with('(') => format!("({sql})"),
        Ok(sql) => sql,
        Err(_) => "(...)".to_string(),
    }
}
