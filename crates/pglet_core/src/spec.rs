//! Storage-facing identifiers shared across the crate graph.

use serde::{Deserialize, Serialize};

use crate::types::TypeOid;

/// Unique identifier for a row within a table.
///
/// Row IDs are assigned sequentially when rows are inserted and never reused
/// within a table's lifetime, so a DELETE/UPDATE addresses a row by its stable ID
/// rather than its full value. The executor evaluates predicates and calls storage
/// with specific row IDs for delete/update operations.
pub type RowId = u64;

/// Which access method maintains and probes a secondary index's entries.
///
/// `Btree` (the default) keys each row to **one** order-preserving composite blob,
/// for equality and range probes. `Gin` is an inverted index: it decomposes an
/// array/JSONB value into **many** element/key entries (one posting entry per
/// element), so `@>` containment probes intersect posting lists instead of scanning.
///
/// Not persisted: it is derived when the storage `IndexSpec` is built
/// (`index_spec_from_catalog`) from the key **shape** and **uniqueness** as well as
/// the column type — a *non-unique* single bare array/JSONB column is `Gin`,
/// everything else (scalar, composite, expression, or unique) `Btree`. The column
/// type does not decide it on its own, and the difference is not academic: GIN
/// cannot enforce uniqueness, so a declared `UNIQUE` / `PRIMARY KEY` over a jsonb or
/// array column mints a **B-tree** index over it. Measured — `CREATE TABLE dj (id
/// INT, j JSONB UNIQUE)` renders `CREATE UNIQUE INDEX dj_j_key ON public.dj USING
/// btree (j)` byte-identically here and on 18.6, keyed by `Value::encode_index_key`.
/// `CREATE INDEX` gates the declared method against the derived one: a declared `gin`
/// needs the shape that derives `Gin`, and a `btree` — declared or defaulted — over
/// that shape is refused, there being no field to record it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IndexMethod {
    /// Order-preserving B+tree: one key blob per row (equality + range).
    #[default]
    Btree,
    /// Inverted index: one entry per decomposed array element / JSONB key, for
    /// `@>` containment.
    Gin,
}

impl IndexMethod {
    /// The Postgres access-method name (`btree` / `gin`) — the spelling
    /// `pg_indexes.indexdef`'s `USING <method>` clause and `CREATE INDEX` use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Btree => "btree",
            Self::Gin => "gin",
        }
    }
}

/// Everything storage needs to register/maintain a secondary index.
///
/// Bundled into one struct so the index params (the plain key plus the partial
/// predicate, the NULL-distinctness rule, and the column names the predicate
/// evaluator binds against) propagate through the executor and backend traits
/// without a positional-argument explosion across the many `Storage` impls.
///
/// `predicate_ir` is the partial index's `WHERE` predicate as the **serialized
/// planner `Expr`** (`None` for a full index); the storage wrapper deserializes it
/// once and caches the `Expr`. The bytes are opaque to this leaf crate (which can't
/// name `pglet_sql::planner::Expr`) — `pglet_sql`'s `serialize_expr` produces them
/// and the storage layer's `deserialize_expr` reads them back.
/// `nulls_not_distinct` selects the NULL-key rule (see `Index.nulls_not_distinct`).
/// `column_names` is the indexed table's column names in row order — the
/// `col_names` the pure predicate evaluator (`eval_expr`) binds against.
#[derive(Debug, Clone)]
pub struct IndexSpec {
    /// The catalog-assigned id the index entries live under.
    pub index_id: u32,
    /// The index's name — what a size refusal raised during maintenance names, as
    /// 18.6's does. Not persisted, as below.
    pub name: String,
    /// The indexed table's name.
    pub table: String,
    /// Indexed column positions in row order (one for single-column, N composite).
    ///
    /// Used only for a **plain** (bare-column) index — `key_part_ir` is `None`. An
    /// expression index leaves this empty and keys via `key_part_ir` instead.
    pub key_col_positions: Vec<usize>,
    /// For an **expression** index: each key part as a **serialized planner `Expr`**
    /// (a bare column is an `Expr::Column`, an expression its IR — e.g. the IR for
    /// `LOWER(email)` or `(actor_id, role, COALESCE(scope_kind, 'GLOBAL'))`). `None`
    /// for a plain index, which uses `key_col_positions`. The storage wrapper
    /// deserializes each part once into a cached `Expr` and evaluates it per row
    /// against `column_names` — the same computation at maintenance and probe time,
    /// so the stored key and the write-time probe always agree.
    pub key_part_ir: Option<Vec<Vec<u8>>>,
    /// Uniqueness intent (the SQL executor enforces it via `enforce_index_entries`).
    pub unique: bool,
    /// Partial index `WHERE` predicate as a serialized planner `Expr`, or `None` for
    /// a full index.
    pub predicate_ir: Option<Vec<u8>>,
    /// `NULLS NOT DISTINCT`: index NULL keys (two NULLs conflict) when `true`.
    pub nulls_not_distinct: bool,
    /// The indexed table's column names in row order (predicate- and key-evaluator
    /// binding — the `col_names` `eval_expr` resolves columns against).
    pub column_names: Vec<String>,
    /// The indexed table's declared column types, positionally parallel to
    /// `column_names` — the `col_types` the same evaluation binds against.
    ///
    /// Load-bearing for the one distinction a `Value` cannot carry: `timestamptz`
    /// and `timestamp` are both `Value::Timestamp`, and only the declared type says
    /// whether a stringified value keeps its `+00` offset. Without it a stored
    /// expression (`WHERE tz_col::text LIKE '%+00'`, a `(tz_col::text)` key) would
    /// index a different string than the query computes. Not persisted — the spec
    /// is rebuilt from the catalog on every open (`index_spec_from_catalog`).
    pub column_types: Vec<TypeOid>,
    /// The indexed table's enum member orders, positionally parallel to
    /// `column_names` (`None` for a non-enum column) — the `col_enums` the same
    /// evaluation binds against.
    ///
    /// Load-bearing for the same reason [`Self::column_types`] is, one type further:
    /// an enum value is a `Value::Text` and its declared type is `Text`, so neither
    /// says that a stored predicate's `m > 'sad'` orders by the members' **positions**.
    /// Without it a partial index's membership is decided by the label bytes while
    /// every comparison beside it is decided by the member order — which for a
    /// **unique** partial index is a silent wrong *write*: a duplicate 18.6 refuses is
    /// accepted, because neither row was placed in the index. Not persisted, as above.
    pub column_enum_orders: Vec<Option<Vec<String>>>,
    /// The access method (`Btree` or `Gin`). Selects single-blob keying vs.
    /// element decomposition at maintenance time, and equality/range vs.
    /// containment probing. Derived from the key shape and uniqueness as well as the
    /// key column's type (see `IndexMethod`).
    pub method: IndexMethod,
}
