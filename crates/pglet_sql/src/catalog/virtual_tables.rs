//! Virtual catalog table definitions for `pg_catalog` compatibility.
//!
//! These tables don't exist in storage - they're generated on the fly
//! from schema metadata. The planner recognizes these table names and
//! creates `Plan::VirtualScan` nodes, which the executor handles by
//! calling the appropriate `PgCatalog` method.

use crate::types::{ColumnDesc, TypeOid};
use serde::Serialize;

/// Virtual catalog table identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CatalogTable {
    PgClass,
    PgNamespace,
    PgAttribute,
    PgType,
    /// `pg_constraint` — one row per NOT NULL / CHECK / FOREIGN KEY / PRIMARY KEY /
    /// UNIQUE constraint, from the one enumeration `pg_get_constraintdef` resolves an
    /// OID against (`constraints_view`).
    PgConstraint,
    /// `pg_index` — one row per index, keyed by the catalog `Index.oid` that
    /// `pg_class`'s `relkind = 'i'` rows and `pg_constraint.conindid` also carry.
    PgIndex,
    /// `pg_enum` — one row per member label of every user `ENUM` type, carrying the
    /// `enumtypid` that joins to `pg_type.oid` (the fuz spine's enum snapshot).
    PgEnum,
    /// `pg_tables` — one row per user table (synthesized from `Schema.tables`).
    PgTables,
    /// `pg_indexes` — one row per index (synthesized from `Schema.indexes`), a
    /// `PRIMARY KEY` / `UNIQUE` constraint's own index included, under the
    /// constraint's name.
    PgIndexes,
    /// `information_schema.tables` — one row per user table (synthesized from
    /// `Schema.tables`; `table_type` is always `BASE TABLE`).
    InformationSchemaTables,
    /// `information_schema.columns` — one row per table column (synthesized).
    InformationSchemaColumns,
    /// `information_schema.table_constraints` — one row per NOT NULL / CHECK /
    /// FOREIGN KEY / PRIMARY KEY / UNIQUE constraint (synthesized; PK/UNIQUE from the
    /// catalog `Index`'s constraint marker). This view is where 18.6 **relabels** a
    /// `pg_constraint` `contype = 'n'` row as `CHECK`; `pg_constraint` itself reports it
    /// as its own kind.
    InformationSchemaTableConstraints,
    /// `information_schema.key_column_usage` — one row per key column of every
    /// PRIMARY KEY / UNIQUE / FOREIGN KEY constraint.
    InformationSchemaKeyColumnUsage,
    /// `information_schema.referential_constraints` — one row per FOREIGN KEY,
    /// carrying its `delete_rule` (and `update_rule` / `match_option`).
    InformationSchemaReferentialConstraints,
    /// `information_schema.constraint_column_usage` — the columns a constraint uses:
    /// a FOREIGN KEY's referenced (parent) columns, a PRIMARY KEY / UNIQUE's own key
    /// columns.
    InformationSchemaConstraintColumnUsage,
    /// `information_schema.sequences` — one row per sequence that is **not** an
    /// identity column's. 18.6's view excludes those via an INTERNAL `pg_depend` edge;
    /// a `SERIAL`'s sequence has an AUTO edge and *is* listed (measured).
    InformationSchemaSequences,
    /// `information_schema.check_constraints` — one row per CHECK, plus one per
    /// `NOT NULL` rendered as 18.6 renders it (`cid IS NOT NULL`).
    InformationSchemaCheckConstraints,
}

impl CatalogTable {
    /// Every variant, for the exhaustiveness guards that must walk them all (the
    /// `qualified_name` round trip, and the two-producer check that every declared
    /// column is emitted). A new variant added without a row producer fails there
    /// rather than shipping a relation nobody fills.
    pub const ALL: [Self; 17] = [
        Self::PgClass,
        Self::PgNamespace,
        Self::PgAttribute,
        Self::PgType,
        Self::PgConstraint,
        Self::PgIndex,
        Self::PgEnum,
        Self::PgTables,
        Self::PgIndexes,
        Self::InformationSchemaTables,
        Self::InformationSchemaColumns,
        Self::InformationSchemaTableConstraints,
        Self::InformationSchemaKeyColumnUsage,
        Self::InformationSchemaReferentialConstraints,
        Self::InformationSchemaConstraintColumnUsage,
        Self::InformationSchemaSequences,
        Self::InformationSchemaCheckConstraints,
    ];

    /// Get the canonical table name (e.g., `pg_class`, `pg_type`).
    pub const fn name(&self) -> &'static str {
        match self {
            Self::PgClass => "pg_class",
            Self::PgNamespace => "pg_namespace",
            Self::PgAttribute => "pg_attribute",
            Self::PgType => "pg_type",
            Self::PgConstraint => "pg_constraint",
            Self::PgIndex => "pg_index",
            Self::PgEnum => "pg_enum",
            Self::PgTables => "pg_tables",
            Self::PgIndexes => "pg_indexes",
            Self::InformationSchemaTables => "tables",
            Self::InformationSchemaColumns => "columns",
            Self::InformationSchemaTableConstraints => "table_constraints",
            Self::InformationSchemaKeyColumnUsage => "key_column_usage",
            Self::InformationSchemaReferentialConstraints => "referential_constraints",
            Self::InformationSchemaConstraintColumnUsage => "constraint_column_usage",
            Self::InformationSchemaSequences => "sequences",
            Self::InformationSchemaCheckConstraints => "check_constraints",
        }
    }

    /// The canonical **resolvable** name — what [`Self::from_name`] round-trips on
    /// (`from_name(t.qualified_name()) == Some(t)`). Same as [`Self::name`] for
    /// `pg_catalog` tables (matched bare), but schema-qualified for
    /// `information_schema` views (matched only on the qualifier). Used as the
    /// FROM-scope relation name for parameter typing, where a bare `name()` like
    /// `tables` couldn't be re-resolved.
    pub const fn qualified_name(&self) -> &'static str {
        match self {
            // information_schema views resolve only by the schema-qualified name.
            Self::InformationSchemaTables => "information_schema.tables",
            Self::InformationSchemaColumns => "information_schema.columns",
            Self::InformationSchemaTableConstraints => "information_schema.table_constraints",
            Self::InformationSchemaKeyColumnUsage => "information_schema.key_column_usage",
            Self::InformationSchemaReferentialConstraints => {
                "information_schema.referential_constraints"
            }
            Self::InformationSchemaConstraintColumnUsage => {
                "information_schema.constraint_column_usage"
            }
            Self::InformationSchemaSequences => "information_schema.sequences",
            Self::InformationSchemaCheckConstraints => "information_schema.check_constraints",
            // pg_catalog tables resolve bare, so the qualified name is the bare name.
            Self::PgClass
            | Self::PgNamespace
            | Self::PgAttribute
            | Self::PgType
            | Self::PgConstraint
            | Self::PgIndex
            | Self::PgEnum
            | Self::PgTables
            | Self::PgIndexes => self.name(),
        }
    }

    /// Try to parse a table name as a virtual catalog table.
    ///
    /// `pg_catalog` tables match bare (`pg_class`) or schema-qualified
    /// (`pg_catalog.pg_class`); `information_schema` views require the qualifier
    /// (`information_schema.columns`) so a bare user table named `columns` isn't
    /// shadowed.
    ///
    /// The match is **exact**: `name` arrives from the parser's name fold, so
    /// `PG_CLASS` is already `pg_class` and a quoted `"PG_CLASS"` correctly resolves
    /// to nothing (`42P01` on 18.6).
    pub fn from_name(name: &str) -> Option<Self> {
        // `information_schema` views are matched only on the schema-qualified name
        // (so a bare user table of the same name isn't shadowed).
        match name {
            "information_schema.tables" => return Some(Self::InformationSchemaTables),
            "information_schema.columns" => return Some(Self::InformationSchemaColumns),
            "information_schema.table_constraints" => {
                return Some(Self::InformationSchemaTableConstraints);
            }
            "information_schema.key_column_usage" => {
                return Some(Self::InformationSchemaKeyColumnUsage);
            }
            "information_schema.referential_constraints" => {
                return Some(Self::InformationSchemaReferentialConstraints);
            }
            "information_schema.constraint_column_usage" => {
                return Some(Self::InformationSchemaConstraintColumnUsage);
            }
            "information_schema.sequences" => return Some(Self::InformationSchemaSequences),
            "information_schema.check_constraints" => {
                return Some(Self::InformationSchemaCheckConstraints);
            }
            _ => {}
        }
        match name.strip_prefix("pg_catalog.").unwrap_or(name) {
            "pg_class" => Some(Self::PgClass),
            "pg_namespace" => Some(Self::PgNamespace),
            "pg_attribute" => Some(Self::PgAttribute),
            "pg_type" => Some(Self::PgType),
            "pg_constraint" => Some(Self::PgConstraint),
            "pg_index" => Some(Self::PgIndex),
            "pg_enum" => Some(Self::PgEnum),
            "pg_tables" => Some(Self::PgTables),
            "pg_indexes" => Some(Self::PgIndexes),
            _ => None,
        }
    }

    /// Whether [`Self::columns`] is the relation's **whole** column list — every
    /// column 18.6 declares for it.
    ///
    /// This is the fact the plan-time binding gate needs, and it is why a catalog view
    /// is not simply registered exhaustively: for a relation pglet models only part of,
    /// an exhaustive scope would turn `SELECT <a column only 18.6's view has> FROM
    /// information_schema.columns WHERE table_name = '<absent>'` — `(0 rows)` on both
    /// engines — into a false `42703`. For a **complete** relation the opposite holds, and it was the standing
    /// defect this fact closes: `pg_constraint` and `pg_index` returned zero rows
    /// always, so `SELECT zzz FROM pg_constraint` answered `(0 rows)` with `zzz`
    /// described as `text` where 18.6 raises `42703`, and *any* invented name did.
    ///
    /// A `true` here is a claim about **both producers**: the column list below, and
    /// the row builder in `PgCatalog` / `InformationSchema` that must emit a value for
    /// every one of them. The unit test
    /// `every_catalog_relation_emits_exactly_its_declared_columns` (in
    /// `executor::query::scan`) is what holds the pair together — a column the gate binds that the row producer does not emit is the
    /// `ResultShape` two-producer class.
    ///
    /// A relation that is complete here answers `42703` for a column 18.6 *does*
    /// declare but pglet does not model — a deliberate over-refusal, taken because the
    /// alternative is a fabricated zero-row `text` column, which is silent. The
    /// per-relation omissions are listed in `docs/compatibility.md`.
    pub const fn columns_complete(&self) -> bool {
        match self {
            // Every column 18.6 declares, modelled — plus `pg_constraint` (22 of 28)
            // and `pg_index` (15 of 21), which are modelled in part and loud about the
            // part: a name outside the list is `42703` here where 18.6 would answer the
            // column. That over-refusal is the deliberate half of this fact; see the
            // doc above.
            Self::PgTables
            | Self::PgIndexes
            | Self::PgEnum
            | Self::PgConstraint
            | Self::PgIndex
            | Self::InformationSchemaKeyColumnUsage
            | Self::InformationSchemaReferentialConstraints
            | Self::InformationSchemaConstraintColumnUsage
            | Self::InformationSchemaSequences
            | Self::InformationSchemaCheckConstraints => true,
            // Strict subsets of 18.6's list, registered non-exhaustively so a name
            // this engine does not model still binds (and answers zero rows).
            Self::PgClass
            | Self::PgNamespace
            | Self::PgAttribute
            | Self::PgType
            | Self::InformationSchemaTables
            | Self::InformationSchemaColumns
            | Self::InformationSchemaTableConstraints => false,
        }
    }

    /// Returns the column schema for this virtual table.
    pub fn columns(&self) -> Vec<ColumnDesc> {
        self.column_specs()
            .iter()
            .map(|(name, type_oid)| ColumnDesc::new(*name, *type_oid))
            .collect()
    }

    /// This relation's declared columns as a **static** `(name, TypeOid)` table.
    ///
    /// The descriptors are built from it rather than written as a list of constructor
    /// calls, because a `Vec` of `ColumnDesc::new(…)` calls costs code per column and
    /// this engine declares ~180 catalog columns — measured, the static form is the
    /// difference between clearing the wasm size ceiling and breaching it. It is also
    /// the list [`Self::columns_complete`] makes a claim about, so keeping it one
    /// datum keeps the claim readable beside it.
    const fn column_specs(self) -> &'static [(&'static str, TypeOid)] {
        match self {
            Self::PgClass => &[
                ("oid", TypeOid::Oid),
                ("relname", TypeOid::Name),
                ("relnamespace", TypeOid::Oid),
                ("relkind", TypeOid::Char),
                ("relhasindex", TypeOid::Bool),
                ("relnatts", TypeOid::Int2),
                ("relchecks", TypeOid::Int2),
                ("relpersistence", TypeOid::Char),
                ("relrowsecurity", TypeOid::Bool),
            ],
            Self::PgNamespace => &[
                ("oid", TypeOid::Oid),
                ("nspname", TypeOid::Name),
                ("nspowner", TypeOid::Oid),
            ],
            Self::PgAttribute => &[
                ("attrelid", TypeOid::Oid),
                ("attname", TypeOid::Name),
                ("atttypid", TypeOid::Oid),
                ("attlen", TypeOid::Int2),
                ("attnum", TypeOid::Int2),
                ("atttypmod", TypeOid::Int4),
                ("attndims", TypeOid::Int2),
                ("attnotnull", TypeOid::Bool),
                ("atthasdef", TypeOid::Bool),
                ("attidentity", TypeOid::Char),
                ("attgenerated", TypeOid::Char),
                ("attisdropped", TypeOid::Bool),
            ],
            Self::PgType => &[
                ("oid", TypeOid::Oid),
                ("typname", TypeOid::Name),
                ("typnamespace", TypeOid::Oid),
                ("typlen", TypeOid::Int2),
                ("typbyval", TypeOid::Bool),
                ("typtype", TypeOid::Char),
                ("typcategory", TypeOid::Char),
                ("typisdefined", TypeOid::Bool),
                ("typelem", TypeOid::Oid),
                ("typarray", TypeOid::Oid),
            ],
            Self::InformationSchemaColumns => &[
                ("table_catalog", TypeOid::Text),
                ("table_schema", TypeOid::Text),
                ("table_name", TypeOid::Text),
                ("column_name", TypeOid::Text),
                ("ordinal_position", TypeOid::Int4),
                ("column_default", TypeOid::Text),
                ("is_nullable", TypeOid::Text),
                ("data_type", TypeOid::Text),
                ("character_maximum_length", TypeOid::Int4),
                ("numeric_precision", TypeOid::Int4),
                ("numeric_precision_radix", TypeOid::Int4),
                ("numeric_scale", TypeOid::Int4),
                ("datetime_precision", TypeOid::Int4),
                ("udt_name", TypeOid::Text),
                ("is_identity", TypeOid::Text),
                ("identity_generation", TypeOid::Text),
                ("is_generated", TypeOid::Text),
            ],
            Self::InformationSchemaTableConstraints => &[
                ("constraint_catalog", TypeOid::Text),
                ("constraint_schema", TypeOid::Text),
                ("constraint_name", TypeOid::Text),
                ("table_catalog", TypeOid::Text),
                ("table_schema", TypeOid::Text),
                ("table_name", TypeOid::Text),
                ("constraint_type", TypeOid::Text),
                ("is_deferrable", TypeOid::Text),
                ("initially_deferred", TypeOid::Text),
            ],
            Self::InformationSchemaTables => &[
                ("table_catalog", TypeOid::Text),
                ("table_schema", TypeOid::Text),
                ("table_name", TypeOid::Text),
                ("table_type", TypeOid::Text),
            ],
            Self::PgTables => &[
                ("schemaname", TypeOid::Name),
                ("tablename", TypeOid::Name),
                ("tableowner", TypeOid::Name),
                ("tablespace", TypeOid::Name),
                ("hasindexes", TypeOid::Bool),
                ("hasrules", TypeOid::Bool),
                ("hastriggers", TypeOid::Bool),
                ("rowsecurity", TypeOid::Bool),
            ],
            Self::PgIndexes => &[
                ("schemaname", TypeOid::Name),
                ("tablename", TypeOid::Name),
                ("indexname", TypeOid::Name),
                ("tablespace", TypeOid::Name),
                ("indexdef", TypeOid::Text),
            ],
            Self::InformationSchemaKeyColumnUsage => &[
                ("constraint_catalog", TypeOid::Text),
                ("constraint_schema", TypeOid::Text),
                ("constraint_name", TypeOid::Text),
                ("table_catalog", TypeOid::Text),
                ("table_schema", TypeOid::Text),
                ("table_name", TypeOid::Text),
                ("column_name", TypeOid::Text),
                ("ordinal_position", TypeOid::Int4),
                ("position_in_unique_constraint", TypeOid::Int4),
            ],
            Self::InformationSchemaReferentialConstraints => &[
                ("constraint_catalog", TypeOid::Text),
                ("constraint_schema", TypeOid::Text),
                ("constraint_name", TypeOid::Text),
                ("unique_constraint_catalog", TypeOid::Text),
                ("unique_constraint_schema", TypeOid::Text),
                ("unique_constraint_name", TypeOid::Text),
                ("match_option", TypeOid::Text),
                ("update_rule", TypeOid::Text),
                ("delete_rule", TypeOid::Text),
            ],
            Self::InformationSchemaConstraintColumnUsage => &[
                ("table_catalog", TypeOid::Text),
                ("table_schema", TypeOid::Text),
                ("table_name", TypeOid::Text),
                ("column_name", TypeOid::Text),
                ("constraint_catalog", TypeOid::Text),
                ("constraint_schema", TypeOid::Text),
                ("constraint_name", TypeOid::Text),
            ],
            Self::PgConstraint => &[
                ("oid", TypeOid::Oid),
                ("conname", TypeOid::Name),
                ("connamespace", TypeOid::Oid),
                ("contype", TypeOid::Char),
                ("condeferrable", TypeOid::Bool),
                ("condeferred", TypeOid::Bool),
                ("conenforced", TypeOid::Bool),
                ("convalidated", TypeOid::Bool),
                ("conrelid", TypeOid::Oid),
                ("contypid", TypeOid::Oid),
                ("conindid", TypeOid::Oid),
                ("conparentid", TypeOid::Oid),
                ("confrelid", TypeOid::Oid),
                ("confupdtype", TypeOid::Char),
                ("confdeltype", TypeOid::Char),
                ("confmatchtype", TypeOid::Char),
                ("conislocal", TypeOid::Bool),
                ("coninhcount", TypeOid::Int2),
                ("connoinherit", TypeOid::Bool),
                ("conperiod", TypeOid::Bool),
                ("conkey", TypeOid::Int2Array),
                ("confkey", TypeOid::Int2Array),
            ],
            Self::PgIndex => &[
                ("indexrelid", TypeOid::Oid),
                ("indrelid", TypeOid::Oid),
                ("indnatts", TypeOid::Int2),
                ("indnkeyatts", TypeOid::Int2),
                ("indisunique", TypeOid::Bool),
                ("indnullsnotdistinct", TypeOid::Bool),
                ("indisprimary", TypeOid::Bool),
                ("indisexclusion", TypeOid::Bool),
                ("indimmediate", TypeOid::Bool),
                ("indisclustered", TypeOid::Bool),
                ("indisvalid", TypeOid::Bool),
                ("indcheckxmin", TypeOid::Bool),
                ("indisready", TypeOid::Bool),
                ("indislive", TypeOid::Bool),
                ("indisreplident", TypeOid::Bool),
            ],
            Self::PgEnum => &[
                ("oid", TypeOid::Oid),
                ("enumtypid", TypeOid::Oid),
                ("enumsortorder", TypeOid::Float4),
                ("enumlabel", TypeOid::Name),
            ],
            Self::InformationSchemaSequences => &[
                ("sequence_catalog", TypeOid::Text),
                ("sequence_schema", TypeOid::Text),
                ("sequence_name", TypeOid::Text),
                ("data_type", TypeOid::Text),
                ("numeric_precision", TypeOid::Int4),
                ("numeric_precision_radix", TypeOid::Int4),
                ("numeric_scale", TypeOid::Int4),
                ("start_value", TypeOid::Text),
                ("minimum_value", TypeOid::Text),
                ("maximum_value", TypeOid::Text),
                ("increment", TypeOid::Text),
                ("cycle_option", TypeOid::Text),
            ],
            Self::InformationSchemaCheckConstraints => &[
                ("constraint_catalog", TypeOid::Text),
                ("constraint_schema", TypeOid::Text),
                ("constraint_name", TypeOid::Text),
                ("check_clause", TypeOid::Text),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_name() {
        assert_eq!(
            CatalogTable::from_name("pg_class"),
            Some(CatalogTable::PgClass)
        );
        assert_eq!(
            CatalogTable::from_name("pg_catalog.pg_class"),
            Some(CatalogTable::PgClass)
        );
        // The comparison is exact: an uppercase spelling reaches this function only
        // as the parser's already-downcased `pg_class`, and a *quoted* `"PG_CLASS"`
        // must not resolve (`42P01` on 18.6).
        assert_eq!(CatalogTable::from_name("PG_CLASS"), None);
        assert_eq!(CatalogTable::from_name("users"), None);
        assert_eq!(CatalogTable::from_name("pg_something_else"), None);
        // information_schema views require the schema qualifier.
        assert_eq!(
            CatalogTable::from_name("information_schema.columns"),
            Some(CatalogTable::InformationSchemaColumns)
        );
        assert_eq!(CatalogTable::from_name("INFORMATION_SCHEMA.COLUMNS"), None);
        // A bare `columns` is a user table, not the view — never shadowed.
        assert_eq!(CatalogTable::from_name("columns"), None);
        // `pg_tables` / `pg_indexes` are pg_catalog views (bare or qualified).
        assert_eq!(
            CatalogTable::from_name("pg_tables"),
            Some(CatalogTable::PgTables)
        );
        assert_eq!(
            CatalogTable::from_name("pg_catalog.pg_indexes"),
            Some(CatalogTable::PgIndexes)
        );
        // The FK-introspection views require the `information_schema` qualifier.
        assert_eq!(
            CatalogTable::from_name("information_schema.key_column_usage"),
            Some(CatalogTable::InformationSchemaKeyColumnUsage)
        );
        assert_eq!(
            CatalogTable::from_name("information_schema.referential_constraints"),
            Some(CatalogTable::InformationSchemaReferentialConstraints)
        );
        assert_eq!(
            CatalogTable::from_name("information_schema.constraint_column_usage"),
            Some(CatalogTable::InformationSchemaConstraintColumnUsage)
        );
        // A bare `key_column_usage` is a user table, not the view.
        assert_eq!(CatalogTable::from_name("key_column_usage"), None);
    }

    #[test]
    fn test_columns() {
        let cols = CatalogTable::PgClass.columns();
        assert_eq!(cols.len(), 9);
        assert_eq!(cols[0].name, "oid");
        assert_eq!(cols[1].name, "relname");
    }

    #[test]
    fn qualified_name_round_trips_through_from_name() {
        // `qualified_name` is the resolvable name used as the FROM-scope relation for
        // parameter typing; `from_name` must round-trip it for every variant (a new
        // variant added with a non-resolvable name fails loud here).
        for t in CatalogTable::ALL {
            assert_eq!(
                CatalogTable::from_name(t.qualified_name()),
                Some(t),
                "from_name must round-trip qualified_name for {t:?}"
            );
        }
    }
}
