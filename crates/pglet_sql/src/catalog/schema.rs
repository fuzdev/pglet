use crate::RelationKind;
use crate::planner::Expr;
use crate::types::{TypeOid, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Database schema - tables and their definitions.
///
/// The collection fields are `BTreeMap` (not `HashMap`) so that
/// [`serialize_schema`] is **deterministic**: postcard serializes a map in
/// iteration order, and a `HashMap`'s order is process-/instance-random (seeded
/// per `RandomState`), so re-serializing a recovered schema produced different
/// bytes than the persisted ones — tripping the storage layer's byte-equality
/// catalog-change gate into a spurious `SYS_CATALOG` rewrite (an extra fsync) on
/// the first commit after reopening a multi-table DB. Sorted-key order makes the
/// encoding canonical, and incidentally makes `information_schema`/`pg_catalog`
/// row order and index selection deterministic (name-sorted) rather than random.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub tables: BTreeMap<String, Table>,
    pub indexes: BTreeMap<String, Index>,
    /// Sequences by name. Backs `SERIAL`/`BIGSERIAL` columns, identity columns, and
    /// `DEFAULT nextval(...)`; see [`Sequence`].
    #[serde(default)]
    pub sequences: BTreeMap<String, Sequence>,
    /// User `ENUM` types by (ASCII-downcased) name. An enum-typed column stores its
    /// value as the text label and references its type here for member validation;
    /// see `Column.enum_type` and [`EnumType`].
    #[serde(default)]
    pub enums: BTreeMap<String, EnumType>,
    /// Stamp moved forward by every execution of a plan that
    /// [`crate::planner::Plan::mutates_schema`] and by every restore of a snapshot taken
    /// at another stamp ([`Self::restore_from`]): a reader that holds what it derived
    /// from the catalog under one value — a cached plan, a measured depth — reads another
    /// value as a catalog that may have changed since. The same value is not proof of the same
    /// catalog: a holder of `&mut Schema` can change it without moving the stamp, or
    /// replace it and move the stamp backwards, so a host door that hands one out drops
    /// what its own readers hold (`pglet::Database::schema_mut` drops the facade's plan
    /// cache and measured depth); and a clone's stamp moves on its own from there. Purely
    /// runtime state (`#[serde(skip)]` — not persisted, resets to 0 on reopen). Not part
    /// of the on-disk format.
    #[serde(skip)]
    pub generation: u64,
    /// Monotonic OID high-water mark — the next object OID to hand out. Purely
    /// runtime state (`#[serde(skip)]`, seeded lazily from the persisted max): it
    /// only ever advances within a session, so a `DROP` that lowers the live OID
    /// max — or a `ROLLBACK` that restores an older catalog — cannot make a mint
    /// site **reuse** a dropped object's OID. Reuse of one of those is a real
    /// corruption: two logical indexes sharing one storage `index_id` means dropping
    /// one orphans the other's catalog def (→ "index not found"). A **failed**
    /// statement's number is handed back rather than burned, which is a different
    /// question with a different answer — `executor::helpers::next_object_oid`
    /// § A dropped number and a failed one. Not part of the on-disk format: a reopen
    /// seeds it past the catalog's greatest OID and past every index number the tree
    /// holds an entry under (`pglet`'s `Database::reattach_persisted_indexes`), which a
    /// dropped or rolled-back index's entries can outlive in a saved tree.
    #[serde(skip)]
    pub next_oid: i32,
}

/// A user `ENUM` type: its catalog OID plus its ordered member labels.
///
/// The **OID is a persisted catalog fact**, minted from the same
/// `Schema::next_oid` counter tables and indexes draw from, because `pg_type` and
/// `pg_enum` are joined on it (`pg_enum.enumtypid = pg_type.oid` — the spine's enum
/// snapshot) and a value that changed between two reads of the same database would
/// make that join name a different type each time. Sharing the one counter is what
/// keeps an enum's OID from colliding with a table's or an index's; see
/// `executor::helpers::next_object_oid`, the shared seed every mint site reads, which
/// must therefore read this map too.
///
/// Labels are in **declared order** — `pg_enum.enumsortorder` is the position, and
/// the ordering is what a consumer diffs, so it is a `Vec` and never a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumType {
    /// The type's catalog OID (`pg_type.oid` / `pg_enum.enumtypid`).
    pub oid: i32,
    /// The member labels, in declared order.
    pub labels: Vec<String>,
}

/// A sequence's catalog state.
///
/// pglet models a sequence as its **last-issued value** plus the two facts a bare
/// counter cannot answer. `INCREMENT`, `MINVALUE`, `MAXVALUE`, `CACHE` and `CYCLE`
/// stay parsed-and-ignored — `nextval` is unconditionally `last_value + 1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequence {
    /// The last value issued; the next `nextval` returns `last_value + 1`.
    ///
    /// This is Postgres's `(last_value, is_called = true)` pair. A Postgres
    /// sequence sitting at `is_called = false` — the state `setval(s, n, false)`
    /// and a never-advanced sequence are in — is modeled as `last_value = n - 1`,
    /// which agrees on the resume point and drops only the `currval`-is-defined
    /// bit (which is per-session state anyway; see `SequenceSession`).
    pub last_value: i64,
    /// The `START WITH` value.
    ///
    /// Not derivable from `last_value`, and that is the whole reason a sequence is
    /// a record: a bare `ALTER SEQUENCE … RESTART` returns the counter here, so an
    /// engine that kept only the counter could not implement it.
    pub start: i64,
    /// The column this sequence is `OWNED BY`, if any — Postgres's
    /// `sequence → (table, attnum)` dependency edge.
    ///
    /// Recorded by an explicit `OWNED BY` and by every implicit `SERIAL` / identity
    /// sequence, emitted by the dump, and **read by every consumer of ownership**:
    ///
    /// - `executor::dependencies` takes an owned sequence with a `DROP TABLE` /
    ///   `ALTER TABLE … DROP COLUMN` (the AUTO edge), and refuses the drop when a
    ///   column outside the deletion set draws from it;
    /// - `execute_truncate` builds `RESTART IDENTITY`'s restart set from this edge
    ///   (Postgres's `getOwnedSequences`), not from the columns' `NextVal` defaults.
    ///
    /// A column's `NextVal` default is the **other** edge and answers a different
    /// question — a sequence a column merely references is nobody's dependent. Reading
    /// it for ownership got both directions wrong at once: it restarted a referenced
    /// sequence Postgres leaves alone (so `ALTER SEQUENCE … OWNED BY NONE` followed by
    /// `TRUNCATE … RESTART IDENTITY` restarted a detached counter) and left an owned
    /// one whose column has no default untouched.
    pub owner: Option<SequenceOwner>,
}

impl Sequence {
    /// An unowned sequence at its `START WITH` — the counter seeded one below
    /// `start`, so the first `nextval` returns `start`.
    #[must_use]
    pub const fn new(start: i64) -> Self {
        Self {
            last_value: start.saturating_sub(1),
            start,
            owner: None,
        }
    }

    /// [`Self::new`] for the implicit sequence behind a `SERIAL` / identity column,
    /// owned by the column it feeds.
    #[must_use]
    pub fn owned_by(start: i64, table: &str, column: &str) -> Self {
        Self {
            owner: Some(SequenceOwner {
                table: table.to_string(),
                column: column.to_string(),
            }),
            ..Self::new(start)
        }
    }
}

/// The column an `OWNED BY` sequence belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceOwner {
    pub table: String,
    pub column: String,
}

/// A column's `DEFAULT`, resolved when a value is omitted at INSERT.
///
/// Kept catalog-local (not a planner `Expr`) so it serializes into the persisted
/// schema and doesn't leak planner types into the catalog layer.
///
/// **There is no `Literal(Value::Null)` arm's worth of meaning here**: 18.6 stores no
/// `pg_attrdef` row at all when a cooked default is a NULL constant
/// (`AddRelationNewConstraints` skips it), so `DEFAULT NULL` *is* "no default" there —
/// and `ALTER COLUMN … SET DEFAULT NULL` removes an existing row rather than replacing
/// it (measured: `pg_attrdef` 1 → 0). `Parser::parse_column_default` normalizes both
/// spellings to `None` for that reason, so the shape is unreachable from SQL. It was
/// reachable from a store written *before* that normalization, and the two renderers
/// (`information_schema`'s `column_default` and the dump) wore a belt reading it as
/// no-default; both are retired, because the store magic moved to `PGMI` and a
/// `PGMH`-or-older store now fails loud on reopen rather than being decoded.
///
/// **The five `now`-family spellings are five variants, not one.** They differ in what
/// 18.6 *evaluates* — only `clock_timestamp()` re-reads the clock per row, which is
/// what an `ALTER TABLE … ADD COLUMN` backfill over N rows exposes (measured: 3–4
/// distinct over 5 rows against one for the other four) — and in what
/// `pg_get_expr`/`pg_dump` render, which preserves each spelling verbatim. Collapsing
/// them cost both: one backfilled value where 18.6 writes many, and `now()` reported
/// for a column defined `DEFAULT CURRENT_TIMESTAMP`. Of the other four,
/// `statement_timestamp()` reads the statement's start and the rest the transaction's
/// (`crate::builtins`); those three evaluate identically and are kept distinct because
/// the *rendering* keys on the spelling.
///
/// The variant order is a persistence contract — postcard encodes the discriminant by
/// position and there is no `ColumnDefault` migration, so a new arm goes at the **end**
/// (pinned by `catalog::schema::tests`'s `column_default_postcard_variant_tags_are_pinned`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColumnDefault {
    /// A constant default (`DEFAULT 'private'`, `DEFAULT 0`, `DEFAULT true`).
    Literal(Value),
    /// `now()`.
    Now,
    /// `CURRENT_DATE` → the current UTC date.
    CurrentDate,
    /// `gen_random_uuid()` / `uuid_generate_v4()`.
    RandomUuid,
    /// `nextval(seq)` — the sequence name (auto-named `<table>_<col>_seq` for SERIAL).
    NextVal(String),
    /// `CURRENT_TIME` → the current UTC time-of-day (a plain `time`). Appended last
    /// so existing serialized schemas keep their `ColumnDefault` tags.
    CurrentTime,
    /// `transaction_timestamp()` — 18.6's alias for `now()`, rendered under its own
    /// name. Appended for postcard-discriminant stability.
    TransactionTimestamp,
    /// `statement_timestamp()` — stable within a statement on 18.6, so an `ADD COLUMN`
    /// backfill takes one value. Appended for postcard-discriminant stability.
    StatementTimestamp,
    /// `clock_timestamp()` — **volatile**: 18.6 re-reads the clock per row, so an
    /// `ADD COLUMN` backfill writes a different instant into each existing row. The
    /// one member of this family `executor::ddl::is_per_row_volatile` answers `true`
    /// for. Appended for postcard-discriminant stability.
    ClockTimestamp,
    /// `CURRENT_TIMESTAMP` — the SQL value function, kept distinct from `Now` because
    /// `pg_get_expr` renders it as the bare keyword rather than `now()`. Appended for
    /// postcard-discriminant stability.
    CurrentTimestamp,
    /// Any other expression, stored **unevaluated** and evaluated per row — the
    /// catch-all the ten arms above are the recognized special cases of. Appended
    /// last for postcard-discriminant stability.
    ///
    /// **No constant folding**, which is 18.6's own model: `pg_attrdef` holds the
    /// post-parse-analysis tree, so `DEFAULT 1+1` stores an operator expression
    /// (rendered `(1 + 1)` by `pg_get_expr`, not `2`) and `DEFAULT nextval('s') + 1`
    /// draws a fresh sequence value on **every** insert (measured: two inserts give
    /// `2, 3`). Storing the parsed [`Expr`] and evaluating it per row is exactly what
    /// [`CheckConstraint::expr`] already does, and `executor::dml::resolve_default`
    /// is the one evaluator.
    ///
    /// **The ten closed arms are kept rather than folded into this one**, and that is
    /// a decision about the *sequence* machinery rather than about expressiveness:
    /// `NextVal(_)` matches spread across the engine drive sequence naming,
    /// ownership, the `DROP` AUTO edge and the identity model, and re-deriving
    /// "is this default a bare
    /// `nextval`" from an `Expr` at each of them is the duplicated-rules seam this
    /// engine keeps paying for. The parser normalizes in the other direction instead
    /// — an expression that *is* one of the ten recognized forms lands on its own arm
    /// (`parser::ddl`'s `column_default_from_expr`), so `DEFAULT (nextval('s'))` and
    /// `DEFAULT nextval('s')` store the identical `NextVal`, which is what 18.6 does
    /// with them too (measured: both render `nextval('s'::regclass)` and both record
    /// the same dependency edge).
    ///
    /// The two readers that must therefore look *through* this arm are the ones
    /// asking a question about the whole tree rather than about its shape:
    /// `executor::dependencies`' sequence edge (18.6 records the NORMAL edge through
    /// an expression — `DROP SEQUENCE sq1` over `nvq(id INT DEFAULT nextval('sq1') +
    /// 1)` is `2BP01`, measured) and `executor::ddl::is_per_row_volatile`.
    Expr(Expr),
}

/// Whether an identity column (`GENERATED … AS IDENTITY`) is `ALWAYS` or `BY
/// DEFAULT`.
///
/// An identity column is a SERIAL over an implicit `<table>_<col>_seq` sequence —
/// its [`ColumnDefault`] is `NextVal(seq)` and it is implicitly `NOT NULL` — with
/// one extra rule enforced by this flag: an `ALWAYS` column rejects an explicit
/// (non-`DEFAULT`) INSERT/UPDATE value with `428C9` unless the INSERT carries
/// `OVERRIDING SYSTEM VALUE`, whereas a `BY DEFAULT` column accepts an explicit
/// value exactly like a plain SERIAL. Modeling only this two-way distinction (not a
/// new `ColumnDefault` variant) keeps the full SERIAL machinery — sequence
/// registration, `DEFAULT` resolution, dump — reused unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityGeneration {
    /// `GENERATED ALWAYS AS IDENTITY` — an explicit non-`DEFAULT` value is rejected
    /// (`428C9`) unless the INSERT overrides with `OVERRIDING SYSTEM VALUE`.
    Always,
    /// `GENERATED BY DEFAULT AS IDENTITY` — an explicit value is accepted and used,
    /// exactly like SERIAL; omitting it (or `DEFAULT`) draws the next sequence value.
    ByDefault,
}

/// A table-scoped `CHECK` constraint.
///
/// The predicate is stored as the **parsed planner `Expr`** (converted once at
/// `CREATE TABLE` / `ALTER TABLE ADD CONSTRAINT`), serialized into the persisted
/// schema directly — no source-SQL string and no re-parse at write time. It is
/// evaluated against each written row via `eval_expr`; only `Value::Bool(false)`
/// violates (a `Bool(true)` or `Null` result passes, per SQL 3-valued logic). A
/// column-level `CHECK` normalizes to a table-scoped check here.
///
/// The name is **total**, and that is the point: it is minted once at DDL time by
/// `executor::ddl`'s `ConstraintNames` — the declared `CONSTRAINT <n>` or Postgres's
/// disambiguated auto-name — and every reader (the violation message,
/// `information_schema.table_constraints`, the dump emitter) reads that one fact. An
/// `Option` here re-opens the defect the mint rule closed: a `None` has to be
/// reconstructed per reader, the readers disagree, and the reconstruction is not even
/// self-consistent with a fresh mint (it can only search the table's own preceding
/// checks, where `ChooseConstraintName` searches the whole schema across all four
/// kinds). The parse-side counterpart that *can* say "no name declared" is
/// `planner::CheckConstraintDef`.
///
/// Not `Eq` (the predicate `Expr` may carry an `f64` literal); `PartialEq` is for
/// the `Plan`-equivalence oracle and round-trip tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckConstraint {
    /// The constraint's name — declared or minted, never absent.
    pub name: String,
    /// The predicate, parsed into the planner IR (e.g. `level IN ('viewer', 'editor')`).
    pub expr: Expr,
}

/// A foreign key's referential action.
///
/// Taken on referencing (child) rows when the referenced (parent) row is deleted
/// (`ON DELETE`) or its referenced key is updated (`ON UPDATE`) — both directions
/// share the same action set.
///
/// `NoAction` and `Restrict` both reject immediately when a parent delete /
/// referenced-key update would orphan a child (pglet has no deferral), with Postgres's
/// distinct SQLSTATEs: `NoAction` raises `Error::ForeignKeyViolation` (`23503`),
/// `Restrict` raises `Error::RestrictViolation` (`23001`). They also differ
/// **semantically**, not only by deferral timing: on an `ON UPDATE`, `NoAction` accepts
/// a *substitute* — a same-statement row that moves onto the vacated key so no child is
/// left dangling — whereas `Restrict` **never** substitutes and blocks whenever a child
/// referenced the old value (Postgres gates the substitute test on NO ACTION, so this
/// distinction is real even under immediate checking).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferentialAction {
    /// `NO ACTION` (the default) — reject if a child would be orphaned (a same-statement
    /// substitute row that refills the vacated key is allowed).
    NoAction,
    /// `RESTRICT` — reject if a child references the affected parent row/key, with no
    /// substitute exception.
    Restrict,
    /// `CASCADE` — `ON DELETE` deletes the referencing child rows; `ON UPDATE`
    /// repoints their FK columns to the new parent-key value.
    Cascade,
    /// `SET NULL` — null the child's FK column(s).
    SetNull,
}

/// A `FOREIGN KEY` constraint (single- or multi-column).
///
/// Catalog-local (like `CheckConstraint`/`ColumnDefault`) so it serializes into the
/// persisted schema and re-enforces on reopen. The inline column-level `REFERENCES
/// parent(col)` and the table-level `FOREIGN KEY (col…) REFERENCES parent(col…)`
/// forms both normalize here; a composite key carries all its columns.
///
/// `columns[i]` pairs positionally with `parent_columns[i]` (the local↔parent
/// mapping is positional even when the referenced list is reordered relative to the
/// parent's unique-constraint key order — see `Schema::covering_unique_index`).
/// `columns.len() == parent_columns.len()`, and the referenced columns are covered
/// by a PK/UNIQUE key on the parent (matched as a set, order-independent).
///
/// Child-side existence (INSERT/UPDATE) is enforced by `enforce_foreign_keys`; the
/// parent-side `on_delete` action is applied by `execute_delete`, and the
/// `on_update` action by `execute_update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    /// The constraint name if declared (`CONSTRAINT fk_foo …`), else a
    /// Postgres-style auto-name (`<table>_<col1>_<col2>_…_fkey`) synthesized at capture.
    pub name: String,
    /// The referencing (child) columns on this table, in declared order.
    pub columns: Vec<String>,
    /// The referenced (parent) table.
    pub parent_table: String,
    /// The referenced (parent) columns, paired positionally with `columns` (covered
    /// by a PK/UNIQUE key on the parent, matched as a set).
    pub parent_columns: Vec<String>,
    /// What a parent-row delete does to referencing child rows.
    pub on_delete: ReferentialAction,
    /// What an update to a referenced (parent-key) column does to referencing child
    /// rows.
    pub on_update: ReferentialAction,
}

/// The column names a relation's heap already owns, so no user column may take one.
///
/// 18.6's `heap.c` `SysAtt[]`, in its own order — `ctid`, `xmin`, `cmin`, `xmax`,
/// `cmax`, `tableoid`. `oid` has not been one since it stopped being a per-row system
/// column, and is an ordinary column name on both engines (measured at `CREATE TABLE`
/// and at `ALTER TABLE … ADD COLUMN`).
///
/// The names are **folded** already wherever this list is consulted — the parser
/// downcases an unquoted identifier where it reads it — so an unquoted `CTID` is
/// refused and a quoted `"CTID"` is an ordinary column, as on 18.6.
const SYSTEM_COLUMN_NAMES: [&str; 6] = ["ctid", "xmin", "cmin", "xmax", "cmax", "tableoid"];

/// Whether `name` is one of the [`SYSTEM_COLUMN_NAMES`] a relation's heap owns.
///
/// The one predicate behind [`CatalogError::SystemColumnName`](pglet_core::CatalogError::SystemColumnName),
/// read where a statement adds a name to a relation's column list — `CREATE TABLE` and
/// `ALTER TABLE … ADD COLUMN`, which is what `Plan`'s variants come to. `name` is
/// compared as given: the fold happens once, in the lexer.
#[must_use]
pub fn is_system_column_name(name: &str) -> bool {
    SYSTEM_COLUMN_NAMES.contains(&name)
}

/// Table definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub schema: String,
    pub columns: Vec<Column>,
    /// Object ID (matches Postgres wire protocol as i32).
    pub oid: i32,
    /// Table-scoped `CHECK` constraints (column-level checks normalize here).
    /// Enforced on every written row; see `CheckConstraint`.
    #[serde(default)]
    pub checks: Vec<CheckConstraint>,
    /// `FOREIGN KEY` constraints declared on this table (the child side),
    /// single- or multi-column. Child-side existence is enforced on every written
    /// row; the parent side's `ON DELETE` action is applied when a referenced row is
    /// deleted. See `ForeignKey`.
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKey>,
}

/// Column definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub type_oid: TypeOid,
    pub nullable: bool,
    pub position: i16,
    /// Value to use when the column is omitted from an INSERT (`None` = NULL).
    #[serde(default)]
    pub default: Option<ColumnDefault>,
    /// If set, this column has a user `ENUM` type (the lowercased name, keyed into
    /// `Schema.enums`); inserted/updated values must be one of its members. The
    /// value itself stores as `TypeOid::Text` (enum values round-trip as labels).
    #[serde(default)]
    pub enum_type: Option<String>,
    /// A `NUMERIC(precision, scale)` type modifier, when the column declared one *and*
    /// [`crate::executor::resolve_column_type`] read it. Written
    /// values are rounded to `scale` and precision-checked on store (see
    /// `coerce_column_value`); absent for unconstrained `NUMERIC` and for every
    /// non-numeric type.
    ///
    /// **It is the *element's* modifier**, which for a scalar column is the column's
    /// own: a `numeric(4,1)[]` column carries `(4,1)` here and `coerce_column_value`
    /// applies it element-wise, exactly as 18.6 stores the element's typmod in the array
    /// column's `atttypmod`. So a reader asking about the **column** — the
    /// `information_schema` widths — must gate on the scalar `TypeOid` rather than on
    /// this field being set.
    #[serde(default)]
    pub numeric_typmod: Option<NumericTypmod>,
    /// A `varchar(n)` / `char(n)` character-length modifier, when the column declared one
    /// *and* [`crate::executor::resolve_column_type`] read it. On
    /// store, an over-length value truncates iff its excess is all blanks, else raises
    /// `22001` (see `coerce_column_value`); absent for unconstrained `text`/`varchar`
    /// and for every non-text type. The **element's**, on the same terms as
    /// [`Self::numeric_typmod`] — a bare `char[]` column carries `1`, as 18.6's
    /// `atttypmod = 5` says it should.
    /// Kept separate from [`Self::numeric_typmod`] so neither perturbs the other's
    /// persisted wire shape.
    #[serde(default)]
    pub varchar_length: Option<u32>,
    /// The identity generation (`GENERATED ALWAYS | BY DEFAULT AS IDENTITY`), or
    /// `None` for a non-identity column. An identity column is a SERIAL (its
    /// [`default`](Self::default) is `NextVal`, it is implicitly `NOT NULL`); this
    /// flag adds only the `428C9` explicit-value rule (see [`IdentityGeneration`]).
    /// Appended last with `#[serde(default)]` so it never perturbs the earlier
    /// fields' persisted wire shape.
    #[serde(default)]
    pub identity: Option<IdentityGeneration>,
    /// The sub-second precision of a `timestamp(p)` / `timestamptz(p)` / `time(p)` /
    /// `interval(p)` column, when it declared one *and*
    /// [`crate::executor::resolve_column_type`] read it — already clamped to
    /// [`MAX_TEMPORAL_PRECISION`](pglet_core::types::MAX_TEMPORAL_PRECISION). On store
    /// the value's microseconds round to `p` fractional digits (see
    /// `coerce_column_value`); absent for a bare `timestamp` and for every
    /// non-temporal type.
    ///
    /// The **element's**, on the same terms as [`Self::numeric_typmod`] — a
    /// `timestamp(3)[]` column carries `3`, as 18.6's `atttypmod = 3` says it should —
    /// so [`information_schema`](crate::catalog::information_schema)'s
    /// `datetime_precision`, whose question is about the *column*, gates on the scalar
    /// `TypeOid` and answers NULL for an array.
    ///
    /// Appended last with `#[serde(default)]`, the pattern [`Self::identity`] and
    /// [`Self::varchar_length`] already state — which keeps a *new* store's bytes
    /// well-ordered and buys **nothing** for an old one. postcard is not
    /// self-describing: it reads exactly the fields the struct declares, in order, with
    /// no per-struct length, so a store written before this field takes one field too
    /// many out of every `Column` and reads it from the next one in the `Vec`. That is
    /// why the store magic bumped with it (`PGMM` → `PGMN`), on the same terms
    /// [`Self::varchar_length`] and [`Self::identity`] each bumped it — measured on a
    /// real pre-bump store, whose reopen died with
    /// `failed to deserialize schema: Hit the end of buffer, expected more data`.
    #[serde(default)]
    pub temporal_precision: Option<u8>,
}

/// `VARHDRSZ` — the four bytes a `varlena` header costs, which `varchartypmodin` and
/// `numerictypmodin` both add to the modifier they encode.
const VARHDRSZ: i32 = 4;

/// 18.6's `INTERVAL_FULL_RANGE` — the range mask a bare `interval(p)` carries.
const INTERVAL_FULL_RANGE: i32 = 0x7FFF;

impl Column {
    /// The Postgres `atttypmod` wire encoding for this column, fed into the
    /// `RowDescription` `FieldDescription.type_modifier` so a driver's
    /// `getPrecision`/`getScale` and any width-deriving client read the declared
    /// modifier: `((precision << 16) | (scale & 0x7ff)) + 4` for a `numeric(p,s)`
    /// (matching `numerictypmodin`, sign-safe for a negative scale), `length + 4`
    /// for a `varchar(n)`/`char(n)` (`VARHDRSZ = 4`), and `-1` for a type with no
    /// modifier.
    ///
    /// A **sub-second precision** encodes as the bare `p` for `timestamp(p)` /
    /// `timestamptz(p)` / `time(p)` (`anytimestamp_typmodin` stores the precision
    /// alone, with no `VARHDRSZ`) and as `(INTERVAL_FULL_RANGE << 16) | p` for
    /// `interval(p)`, whose typmod packs a *range* mask beside the precision —
    /// measured on 18.6: `interval(3)` is `2147418115` and `interval(4)` is
    /// `2147418116`, `0x7FFF0000` plus the precision.
    ///
    /// An **array** column reports its element's, unadorned — a `varchar(3)[]` column is
    /// `7`, the same value the scalar `varchar(3)` beside it reports and the same 18.6
    /// sends (measured), and a `timestamp(3)[]` is `3` on the same terms. That falls out
    /// of the fields holding the element's modifier; nothing here distinguishes the two
    /// widths, and nothing should — but the *interval* arm has to ask the **element's**
    /// OID which encoding to use, since a `interval(3)[]` column's `TypeOid` is
    /// `IntervalArray` and 18.6 still reports `2147418115` (measured).
    #[must_use]
    pub fn pg_type_modifier(&self) -> i32 {
        if let Some(t) = self.numeric_typmod {
            return ((i32::from(t.precision) << 16) | (i32::from(t.scale) & 0x7ff)) + VARHDRSZ;
        }
        if let Some(len) = self.varchar_length {
            return i32::try_from(len).map_or(-1, |n| n.saturating_add(VARHDRSZ));
        }
        if let Some(p) = self.temporal_precision {
            let element = self.type_oid.array_element().unwrap_or(self.type_oid);
            return if element == TypeOid::Interval {
                (INTERVAL_FULL_RANGE << 16) | i32::from(p)
            } else {
                i32::from(p)
            };
        }
        -1
    }
}

/// The `(…)` a type modifier spells, decoded from [`Column::pg_type_modifier`]'s
/// `atttypmod` encoding.
///
/// `(5)` for a `varchar(5)`'s `9`, `(6,2)` for a `numeric(6,2)`'s `393222`, `(3)` for a
/// `timestamp(3)`'s `3`, `(3)` for an `interval(3)`'s `2147418115`. `None` for
/// `typmod < 0` and for a type that carries no modifier at all.
///
/// **It is the inverse of the encoder above and lives beside it** so the two cannot
/// drift: an arm added there without one here would print a bare type name where 18.6
/// prints a width, in the one sentence that reads it — the recursive-CTE `42804`'s
/// *type*-mismatch message, whose two halves 18.6 spells with the modifier. The arm
/// *list* is pinned against the membership predicate
/// [`crate::types::type_oid_carries_modifier`] by
/// `every_modifier_carrying_type_has_a_spelling`, since
/// that predicate — not this match — is what says a modifier can be there.
///
/// An **array** is answered at its **element's** width, exactly as the encoder writes it:
/// a `varchar(3)[]` column's typmod is the scalar `varchar(3)`'s `7`, so the same `(3)`
/// comes back and the caller decides where the `[]` goes.
///
/// A scale is sign-extended from eleven bits the way `numeric_typmod_scale` does
/// (`(x ^ 1024) - 1024`), since `numerictypmodin` admits a negative one.
#[must_use]
pub fn pg_type_modifier_spelling(oid: TypeOid, typmod: i32) -> Option<String> {
    if typmod < 0 {
        return None;
    }
    let element = oid.array_element().unwrap_or(oid);
    match element {
        TypeOid::Varchar => Some(format!("({})", typmod - VARHDRSZ)),
        TypeOid::Numeric => {
            let precision = ((typmod - VARHDRSZ) >> 16) & 0xffff;
            let scale = (((typmod - VARHDRSZ) & 0x7ff) ^ 1024) - 1024;
            Some(format!("({precision},{scale})"))
        }
        TypeOid::Timestamp | TypeOid::Timestamptz | TypeOid::Time => Some(format!("({typmod})")),
        TypeOid::Interval => Some(format!("({})", typmod & 0xffff)),
        _ => None,
    }
}

/// A `NUMERIC(precision, scale)` type modifier. `precision` is the total
/// significant-digit budget and `scale` the fractional digits; `NUMERIC(p)` (no
/// scale) is `scale = 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumericTypmod {
    pub precision: u16,
    pub scale: i16,
}

/// The kind of declared table constraint that owns an auto-created unique index.
///
/// Postgres backs a `PRIMARY KEY` / `UNIQUE` *constraint* with an index and records
/// an INTERNAL dependency from the index to the constraint, which is what makes
/// `DROP INDEX` on it `2BP01` while a standalone `CREATE UNIQUE INDEX` drops freely.
/// pglet carries the same distinction here.
///
/// The constraint's **name is the index's own name**, which is why this marker
/// carries no name of its own. That holds because pglet *mints* the index under the
/// constraint's name: the `CONSTRAINT <name>` the user declared when there is one,
/// else Postgres's `<table>_pkey` / `<table>_<col…>_key` auto-name (cut by
/// [`make_object_name`]). Postgres does the same — a
/// constraint's index takes the constraint's name, user-given or generated — so the
/// two share one namespace and one string.
///
/// The load-bearing half is the **mint** side ([`UniqueConstraint::name`](crate::planner::UniqueConstraint)
/// threaded into `create_unique_index`), not this type: read the marker to know
/// *whether* an index is a constraint, read [`Index::name`] to know what it is
/// called. Nothing may re-derive the name from the table's name — a declared
/// constraint breaks that convention (use [`Schema::primary_key_index`] instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintKind {
    /// A `PRIMARY KEY` constraint.
    PrimaryKey,
    /// A `UNIQUE` constraint.
    Unique,
}

impl ConstraintKind {
    /// The `information_schema.table_constraints.constraint_type` text for this
    /// kind, matching Postgres's spelling.
    #[must_use]
    pub const fn constraint_type(self) -> &'static str {
        match self {
            Self::PrimaryKey => "PRIMARY KEY",
            Self::Unique => "UNIQUE",
        }
    }
}

/// Whether an index is unique and — when it is — whether it enforces its uniqueness
/// at write time and which declared constraint (if any) owns it.
///
/// Both facts are meaningful only for a unique index — a non-unique index has nothing
/// to enforce, and a `PRIMARY KEY` / `UNIQUE` constraint's index is unique by
/// construction — so both ride inside the `Unique` variant: the nonsense states
/// ("non-unique but enforcing", "non-unique index owned by a PRIMARY KEY") are simply
/// unrepresentable, rather than flags guarded by a convention. `Unique { enforced:
/// false, .. }` is the (currently unused) accelerate-only-unique case the design
/// leaves room for.
///
/// On disk it (de)serializes through the flat [`UniquenessWire`] shape, which
/// normalizes a nonsense `!unique` combination back to `NonUnique` on read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "UniquenessWire", into = "UniquenessWire")]
pub enum Uniqueness {
    /// A plain (accelerate-only) index — no uniqueness constraint.
    NonUnique,
    /// A unique index (PK, `UNIQUE`, or `CREATE UNIQUE INDEX`). `enforced` records
    /// whether it rejects duplicate keys at write time (see `enforce_index_entries`);
    /// `constraint` records the declared `PRIMARY KEY` / `UNIQUE` constraint that
    /// owns it, or `None` for a standalone `CREATE UNIQUE INDEX`.
    Unique {
        enforced: bool,
        constraint: Option<ConstraintKind>,
    },
}

impl Uniqueness {
    /// Build a **standalone** index's uniqueness from the `(unique, enforced)` pair —
    /// the `CREATE [UNIQUE] INDEX` path, which never has an owning constraint.
    /// Normalizes the nonsense `!unique && enforced` away (a non-unique index is
    /// never enforcing).
    #[must_use]
    pub const fn new(unique: bool, enforced: bool) -> Self {
        if unique {
            Self::Unique {
                enforced,
                constraint: None,
            }
        } else {
            Self::NonUnique
        }
    }

    /// The uniqueness of the enforced index auto-created for a declared `PRIMARY KEY`
    /// / `UNIQUE` constraint.
    #[must_use]
    pub const fn for_constraint(kind: ConstraintKind) -> Self {
        Self::Unique {
            enforced: true,
            constraint: Some(kind),
        }
    }

    /// Whether this is a unique index.
    #[must_use]
    pub const fn is_unique(self) -> bool {
        matches!(self, Self::Unique { .. })
    }

    /// Whether this is a unique index that enforces uniqueness at write time.
    #[must_use]
    pub const fn is_enforced(self) -> bool {
        matches!(self, Self::Unique { enforced: true, .. })
    }

    /// The declared constraint that owns this index, or `None` for a standalone
    /// index (unique or not).
    #[must_use]
    pub const fn constraint(self) -> Option<ConstraintKind> {
        match self {
            Self::Unique { constraint, .. } => constraint,
            Self::NonUnique => None,
        }
    }
}

/// The flat on-disk shape of [`Uniqueness`].
///
/// Keeping the wire flat (rather than serializing the enum's variant tag) means the
/// *fields* are the format, so a nonsense combination read back from disk normalizes
/// through [`From`] instead of being trusted.
#[derive(Serialize, Deserialize)]
struct UniquenessWire {
    unique: bool,
    enforced: bool,
    constraint: Option<ConstraintKind>,
}

impl From<UniquenessWire> for Uniqueness {
    fn from(w: UniquenessWire) -> Self {
        if w.unique {
            Self::Unique {
                enforced: w.enforced,
                constraint: w.constraint,
            }
        } else {
            Self::NonUnique
        }
    }
}

impl From<Uniqueness> for UniquenessWire {
    fn from(u: Uniqueness) -> Self {
        match u {
            Uniqueness::NonUnique => Self {
                unique: false,
                enforced: false,
                constraint: None,
            },
            Uniqueness::Unique {
                enforced,
                constraint,
            } => Self {
                unique: true,
                enforced,
                constraint,
            },
        }
    }
}

/// An index's key parts: either bare columns (by name) or parsed expressions.
///
/// A **plain** index (`Columns`) keys on bare column names, resolved to row
/// positions and probed by name — a single-column plain index is what the
/// point/range/GIN probe planners match against, and a plain index of **any** arity
/// is what an FK existence check matches (`Schema::covering_unique_index`). An
/// **expression** index (`Exprs`) keys on arbitrary scalar expressions
/// (`LOWER(email)`, `COALESCE(scope_kind, 'GLOBAL')`), stored as the parsed
/// planner `Expr` and evaluated per row via `eval_expr`; a bare-column reference
/// inside such a key is a plain `Expr::Column`, so a mixed bare/expression key
/// (`(actor_id, role, COALESCE(scope_kind, 'GLOBAL'))`) folds through one eval path.
///
/// Modeling the two as a sum type (replacing the former `columns: Vec<String>` +
/// `key_is_expression: bool` pair) makes the distinction explicit at the type
/// level: an expression index can never be mistaken for a single-column plain
/// index by the probe matchers (`single_bare_column` only answers for the
/// `Columns` variant), and the key `Expr`s are stored directly — no source-SQL
/// string to re-parse at write time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IndexKey {
    /// Plain bare-column key parts in key order (one for single-column, N for a
    /// composite key); resolved to row positions for keying.
    Columns(Vec<String>),
    /// Expression key parts in key order, parsed once at `CREATE INDEX` and
    /// evaluated per row. A bare-column reference is an `Expr::Column`.
    Exprs(Vec<Expr>),
}

impl IndexKey {
    /// Whether this is an expression-keyed index (vs. plain bare columns).
    #[must_use]
    pub const fn is_expression(&self) -> bool {
        matches!(self, Self::Exprs(_))
    }

    /// The number of key parts.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::Columns(c) => c.len(),
            Self::Exprs(e) => e.len(),
        }
    }

    /// Whether the key has no parts (never true for a well-formed index).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bare column names of a plain index, or `None` for an expression index.
    #[must_use]
    pub fn bare_columns(&self) -> Option<&[String]> {
        match self {
            Self::Columns(c) => Some(c),
            Self::Exprs(_) => None,
        }
    }

    /// The single bare column of a **single-column plain** index — what the
    /// point/range/GIN probe matchers key on — or `None` for a composite plain index
    /// or any expression index. Not the FK existence check's matcher: that one takes a
    /// key of any arity, through [`Self::bare_columns`].
    #[must_use]
    pub fn single_bare_column(&self) -> Option<&str> {
        match self {
            Self::Columns(c) if c.len() == 1 => Some(c[0].as_str()),
            _ => None,
        }
    }

    /// The parsed key-part expressions of an expression index, or `None` for a
    /// plain index.
    #[must_use]
    pub fn exprs(&self) -> Option<&[Expr]> {
        match self {
            Self::Exprs(e) => Some(e),
            Self::Columns(_) => None,
        }
    }
}

/// Index definition.
///
/// Single-column indexes accelerate equality point reads: the planner lowers a
/// `WHERE indexed_col = <value>` to a `Plan::IndexScan` probe (see
/// `planner/query.rs`), and storage maintains the entries on every write.
///
/// A `unique` index that is also `enforced` rejects duplicate keys at write time
/// (see `enforce_index_entries`); `unique && !enforced` keeps today's accelerate-only
/// behavior. See the `enforced` field for which unique indexes are enforceable.
///
/// `key` holds the index's key parts as the parsed [`IndexKey`] IR — bare column
/// names (`IndexKey::Columns`, the fast position-keyed path) or parsed key
/// expressions (`IndexKey::Exprs`, evaluated per row via `eval_expr`). Because the
/// two are distinct variants, the point-read planner's single-column lookup
/// (`single_column_index_for`, which calls `key.single_bare_column()`) and the FK
/// existence probe (`Schema::covering_unique_index`, which calls
/// `key.bare_columns()` and so takes a key of any arity) never mistake an expression
/// index for a column index. The key `Expr`s are stored directly, so there is no
/// source SQL to re-parse at write time.
///
/// Range scans (`<`, `>`, `BETWEEN`) over the order-preserving key encoding and
/// single-key `ORDER BY [ASC|DESC] [LIMIT]` served from a NOT-NULL single-column
/// index (`Sort` elision) are both wired through `Plan::IndexRangeScan`.
///
/// **The access method is not a field here — it is derived**, by
/// `executor::index_method_for`, from *both* the key shape and uniqueness: a
/// **non-unique** single bare column of an array or JSONB type is **GIN**
/// (`CREATE INDEX … USING gin(col)`, decomposed into per-element entries, probed for a
/// `col @> <const>` conjunct via `Plan::GinScan` + recheck); everything else — scalar,
/// composite, expression, or *unique* — is B-tree. One derivation feeds the storage
/// `IndexSpec`, `pg_indexes.indexdef`, and the planner, so none of the three can
/// disagree about what an index is.
///
/// That last consumer is load-bearing rather than incidental: a decomposed key holds one
/// entry per element, not per row, so **only** containment may be served from a GIN index.
/// The planner's two selectors split by method — `single_column_index_for` (equality,
/// range, `ORDER BY` `Sort` elision) takes B-tree only, `gin_index_for` takes GIN only —
/// and both ask `index_method_for` rather than re-deriving it. The split is of the
/// *method gate* only: `single_column_index_for` additionally excludes a partial B-tree
/// index (`predicate.is_none()`), while `gin_index_for` offers a partial GIN index and
/// leaves its predicate to the implication check that follows it.
///
/// TODO(index-optimization): broaden index usage:
/// - composite (multi-column) index probes for `IndexScan` acceleration
///   (composite PK/UNIQUE *enforcement* already works via `enforce_index_entries`)
/// - multi-key / `NULLS FIRST|LAST` / expression-column `ORDER BY` served from an
///   index (single-key NOT-NULL is covered; the rest keep an explicit `Sort`)
//
// The plain/expression distinction lives in the `IndexKey` variant (not a separate
// `key_is_expression` flag); `nulls_not_distinct` is a catalog fact orthogonal to
// uniqueness (a unique index may be plain or expression-keyed, full or partial,
// NULLS [NOT] DISTINCT). Uniqueness and its enforcement are *not* independent
// (enforcement only means anything for a unique index), so they fold into the
// `Uniqueness` enum rather than a bool pair that could express the nonsense
// "non-unique but enforcing". Not `Eq` (a key/predicate `Expr` may carry an `f64`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    pub table_name: String,
    /// The key parts: bare columns (position-keyed) or parsed expressions. See
    /// [`IndexKey`].
    pub key: IndexKey,
    /// Whether the index is unique and, if so, whether it enforces uniqueness at
    /// write time and which declared constraint owns it.
    ///
    /// A unique enforced index rejects duplicate keys at write time (PK,
    /// inline/table-level `UNIQUE`, and a `CREATE UNIQUE INDEX (key…)` —
    /// single-column or composite, plain-column or **expression** (`LOWER(email)`),
    /// **including** partial (`… WHERE deleted_at IS NULL`) and `NULLS NOT DISTINCT`
    /// indexes). The key tuple is recomputed the same way at maintenance and probe
    /// time (positions for a plain key, `eval_expr` for an expression key — see
    /// [`IndexKey`]), so the write-time probe always agrees with the stored entry.
    /// Reads via `is_unique()` / `is_enforced()` / `constraint_kind()`.
    pub uniqueness: Uniqueness,
    /// The `WHERE` predicate of a **partial** unique index, parsed into the planner
    /// IR (e.g. `deleted_at IS NULL`), or `None` for a full index. Serialized into
    /// the persisted schema directly — no source SQL, no re-parse — and evaluated
    /// per row to decide whether the row participates in the index (both for
    /// storage-side maintenance and write-time `enforce_index_entries`). A row that fails
    /// the predicate is not a uniqueness candidate, so soft-deleted duplicates are
    /// allowed.
    pub predicate: Option<Expr>,
    /// `NULLS NOT DISTINCT`: when `true`, NULL key columns ARE indexed and two NULL
    /// keys conflict (Postgres `NULLS NOT DISTINCT`). When `false` (the default,
    /// `NULLS DISTINCT`), a NULL in any key column means the row is never a
    /// uniqueness candidate and is not indexed.
    pub nulls_not_distinct: bool,
    /// Object ID (matches Postgres wire protocol as i32).
    pub oid: i32,
}

impl Index {
    /// Whether this is a unique index (PK, `UNIQUE`, or `CREATE UNIQUE INDEX`).
    #[must_use]
    pub const fn is_unique(&self) -> bool {
        self.uniqueness.is_unique()
    }

    /// Whether this unique index enforces uniqueness at write time (always false
    /// for a non-unique index).
    #[must_use]
    pub const fn is_enforced(&self) -> bool {
        self.uniqueness.is_enforced()
    }

    /// The declared `PRIMARY KEY` / `UNIQUE` constraint this index backs, or `None`
    /// for a standalone `CREATE [UNIQUE] INDEX`.
    ///
    /// The distinction is load-bearing three times over: `DROP INDEX` refuses a
    /// constraint's index (`2BP01`), `information_schema` reports the constraint, and
    /// the dump re-emits it as `ADD CONSTRAINT` rather than `CREATE INDEX`. The
    /// constraint's name is [`Self::name`] — see [`ConstraintKind`].
    #[must_use]
    pub const fn constraint_kind(&self) -> Option<ConstraintKind> {
        self.uniqueness.constraint()
    }

    /// The storage `index_id` for this index — its catalog [`oid`](Self::oid) as a
    /// `u32`.
    ///
    /// Index OIDs are always positive, so the `i32 -> u32` cast is lossless. The
    /// single place that conversion lives, so the "OIDs are positive" invariant
    /// isn't restated at every storage probe / drop / re-register site.
    #[must_use]
    pub const fn index_id(&self) -> u32 {
        self.oid as u32
    }
}

/// The longest name 18.6 stores, in bytes (`NAMEDATALEN - 1`).
const MAX_NAME_BYTES: usize = 63;

/// The longest prefix of `name` that is at most `max` bytes and ends on a character
/// boundary — 18.6's `pg_mbcliplen`.
fn clip_name(name: &str, max: usize) -> &str {
    let mut end = max.min(name.len());
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// `<name1>_<name2>_<label>` cut to [`MAX_NAME_BYTES`] — 18.6's `makeObjectName`
/// (`indexcmds.c`).
///
/// The one cut behind each name the engine chooses: a constraint's (and so a `PRIMARY
/// KEY` / `UNIQUE` index's), an unnamed index's, an implicit sequence's, and the
/// `<table>_<column>_not_null` a `NOT NULL` is rendered under.
///
/// An **empty** `name2` is 18.6's absent one — `<name1>_<label>`, with no second
/// underscore — which is how a primary key (`<table>_pkey`) and a `CHECK` over no
/// single column (`<table>_check`) are named; no caller has a second name that is
/// present and empty, a zero-length identifier being refused where it is read.
///
/// The label is never cut. The two names share what the label and the underscores
/// leave, the **longer** one giving up a byte at a time until they fit, and each is
/// then clipped back to a character boundary — so the result can be shorter than the
/// limit, never longer.
#[must_use]
pub fn make_object_name(name1: &str, name2: &str, label: &str) -> String {
    let overhead = label.len() + 1 + usize::from(!name2.is_empty());
    let available = MAX_NAME_BYTES.saturating_sub(overhead);
    let (mut len1, mut len2) = (name1.len(), name2.len());
    while len1 + len2 > available {
        if len1 > len2 {
            len1 -= 1;
        } else {
            len2 -= 1;
        }
    }
    let mut name = String::with_capacity(MAX_NAME_BYTES);
    name.push_str(clip_name(name1, len1));
    if !name2.is_empty() {
        name.push('_');
        name.push_str(clip_name(name2, len2));
    }
    name.push('_');
    name.push_str(label);
    name
}

/// Postcard-serialize the catalog `schema` for persistence in the storage meta slot.
///
/// The single definition of the schema → bytes step, shared by every persist site
/// (the embed facade's auto-commit / `COMMIT`, the server's auto-commit and
/// `COMMIT`, and the wasm `save_state`). It is **load-bearing for durability**: a
/// caller must NOT persist on `Err` — writing a truncated/empty blob would flip the
/// durable meta slot to an empty catalog and lose every table on reopen. Each caller
/// maps the returned [`Error`](crate::Error) into its own error type and surfaces it
/// loud instead.
///
/// # Errors
///
/// Returns [`Error::Execution`](crate::Error::Execution) if postcard serialization
/// fails (only on a pathological `Schema`).
pub fn serialize_schema(schema: &Schema) -> crate::Result<Vec<u8>> {
    postcard::to_allocvec(schema).map_err(|e| {
        crate::depth::encoding_error(crate::Error::Execution(format!(
            "schema serialize failed: {e}"
        )))
    })
}

/// Decode a [`Schema`] from the bytes [`serialize_schema`] produced — the one reader
/// a store's open goes through.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`](crate::Error::StackDepthExceeded) (`54001`) for a
/// stored expression deeper than this thread's bounds admit
/// ([`crate::depth::EncodingLevel`]) — a store written by a host with larger ones; else
/// [`Error::Execution`](crate::Error::Execution) carrying `postcard`'s message.
pub fn deserialize_schema(bytes: &[u8]) -> crate::Result<Schema> {
    let schema: Schema = postcard::from_bytes(bytes)
        .map_err(|e| crate::depth::encoding_error(crate::Error::Execution(e.to_string())))?;
    // The second door a catalog's expressions come in by, beside the DDL statement that
    // stores one (`crate::depth::STORED_DEPTH_SEEN`): a store read back holds whatever
    // the engine that wrote it stored.
    crate::depth::note_stored_depth(crate::depth::catalog_depth(&schema));
    Ok(schema)
}

impl Schema {
    /// Creates an empty schema.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a table to the schema.
    pub fn create_table(&mut self, table: Table) {
        self.tables.insert(table.name.clone(), table);
    }

    /// Gets a table by name.
    pub fn get_table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }

    /// Gets a mutable reference to a table by name.
    pub fn get_table_mut(&mut self, name: &str) -> Option<&mut Table> {
        self.tables.get_mut(name)
    }

    /// Removes a table by name. Returns true if the table existed.
    pub fn drop_table(&mut self, name: &str) -> bool {
        self.tables.remove(name).is_some()
    }

    /// Adds an index to the schema.
    pub fn create_index(&mut self, index: Index) {
        self.indexes.insert(index.name.clone(), index);
    }

    /// Gets an index by name.
    pub fn get_index(&self, name: &str) -> Option<&Index> {
        self.indexes.get(name)
    }

    /// Removes an index by name. Returns true if the index existed.
    pub fn drop_index(&mut self, name: &str) -> bool {
        self.indexes.remove(name).is_some()
    }

    /// Find a unique index on `table` whose plain key columns form exactly the SET
    /// `columns` (order-independent) — the covering unique constraint a `FOREIGN
    /// KEY` references (Postgres matches the referenced columns to a PK/UNIQUE key
    /// as an unordered set). Returns the first such index, or `None` if the parent
    /// has no covering unique key (the CREATE-time `42830` case).
    ///
    /// The FK's own local↔parent column pairing stays positional; only the *match*
    /// against the parent's unique key is set-based (so `REFERENCES parent(y, x)`
    /// against `PRIMARY KEY (x, y)` is covered). An expression-keyed unique index is
    /// never a match (it has no bare columns to set-compare).
    #[must_use]
    pub fn covering_unique_index(&self, table: &str, columns: &[String]) -> Option<&Index> {
        self.indexes.values().find(|idx| {
            idx.table_name == table
                && idx.is_unique()
                && idx.key.bare_columns().is_some_and(|key_cols| {
                    key_cols.len() == columns.len() && columns.iter().all(|c| key_cols.contains(c))
                })
        })
    }

    /// The index behind `table`'s declared `PRIMARY KEY` constraint, or `None` if the
    /// table has none.
    ///
    /// Asks the catalog's constraint marker rather than looking up the auto-name
    /// `<table>_pkey`: a `CONSTRAINT my_pk PRIMARY KEY (…)` names its index `my_pk`, so
    /// a name lookup would answer "no primary key" — and a plain `CREATE UNIQUE INDEX
    /// t_pkey` would answer with an index that is not one. The marker is the fact; the
    /// name is a default.
    ///
    /// The `find` is unambiguous only because a table may have **at most one** primary
    /// key (`42P16`, enforced at every mint site). Without that gate this would answer
    /// whichever PK sorts first by index name, quietly making FK default-key resolution
    /// and `GROUP BY` functional dependency depend on constraint naming.
    #[must_use]
    pub fn primary_key_index(&self, table: &str) -> Option<&Index> {
        self.indexes.values().find(|idx| {
            idx.table_name == table && idx.constraint_kind() == Some(ConstraintKind::PrimaryKey)
        })
    }

    /// Whether `name` is taken in the **relation namespace** — Postgres's `pg_class`,
    /// where tables, indexes and **sequences** share one name space per schema, so a
    /// `CONSTRAINT <n> PRIMARY KEY` collides with a table named `<n>` exactly as it
    /// does with an index or a sequence (`42P07`, `relation "<n>" already exists`).
    ///
    /// All three memberships are measured on 18.6 in both polarities:
    /// `CREATE SEQUENCE ns1; CREATE TABLE ns1 (…)` and its reverse are `42P07`, as is
    /// a sequence colliding with an index name; and `CREATE TABLE IF NOT EXISTS <a
    /// sequence>` / `CREATE INDEX IF NOT EXISTS <a sequence>` skip rather than error.
    ///
    /// pglet models one schema, so this is the whole namespace. Enum types are *not*
    /// in it — Postgres puts a type in `pg_type`, not `pg_class` — and the other
    /// namespace is [`Self::type_name_taken`].
    #[must_use]
    pub fn relation_name_taken(&self, name: &str) -> bool {
        self.relation_kind(name).is_some()
    }

    /// Whether `name` is taken in the **type namespace** pglet models — Postgres's
    /// `pg_type`, which holds each enum type and each table's **row type**, a type of
    /// the table's own name that `heap_create_with_catalog` creates with the table.
    ///
    /// The kinds are 18.6's, measured with `reltype <> 0` over `pg_class`: a table (and
    /// a view, which pglet does not model) carries a row type, while an index and a
    /// sequence carry none — so `CREATE TYPE` over an index's or a sequence's name is
    /// accepted there, and over a table's is `42710`. The other direction is not the
    /// mirror image: `heap_create_with_catalog` tests this namespace for **every**
    /// relation it creates, a sequence included, so a table or a sequence over an
    /// enum's name is `42710` too, while `CREATE INDEX` (`index_create`) never asks.
    /// Each creating statement therefore asks the question its own 18.6 routine asks
    /// rather than one shared "is this name free?".
    #[must_use]
    pub fn type_name_taken(&self, name: &str) -> bool {
        self.enums.contains_key(name) || self.tables.contains_key(name)
    }

    /// Which **kind** of relation `name` denotes, or `None` if the relation
    /// namespace does not hold it.
    ///
    /// The other half of [`Self::relation_name_taken`], and deliberately the same
    /// walk: every "is this name free?" gate and every "you named the wrong kind of
    /// thing" (`42809`) message read one enumeration of the namespace, so a fourth
    /// relation kind cannot be added to one and forgotten in the other. Every caller
    /// that needs the kind — `ALTER SEQUENCE`'s target, an `OWNED BY` owner,
    /// `nextval`/`currval`/`setval`, and the DDL-time `DEFAULT nextval('x')`
    /// registration — goes through here rather than probing the three maps itself.
    #[must_use]
    pub fn relation_kind(&self, name: &str) -> Option<RelationKind> {
        if self.tables.contains_key(name) {
            Some(RelationKind::Table)
        } else if self.indexes.contains_key(name) {
            Some(RelationKind::Index)
        } else if self.sequences.contains_key(name) {
            Some(RelationKind::Sequence)
        } else {
            None
        }
    }

    /// Whether `table` already carries a constraint named `name`, of **any** of the
    /// four kinds pglet models.
    ///
    /// Postgres keeps one unique index on `pg_constraint (conrelid, conname)` spanning
    /// CHECK, FOREIGN KEY, PRIMARY KEY and UNIQUE, so the kinds do not get separate
    /// name pools (`42710`). The first two are stored on the table by name; the last
    /// two are stored as their *index*, whose name is the constraint's name — so all
    /// four are read here rather than from two half-namespaces that cannot see each
    /// other.
    #[must_use]
    pub fn constraint_name_taken(&self, table: &str, name: &str) -> bool {
        self.constraint_marked_index(table, name).is_some()
            || self.get_table(table).is_some_and(|t| {
                t.checks
                    .iter()
                    .map(|c| c.name.as_str())
                    .chain(t.foreign_keys.iter().map(|fk| fk.name.as_str()))
                    .any(|existing| existing == name)
            })
    }

    /// The index named `name` when it backs a declared `PRIMARY KEY` / `UNIQUE` on
    /// `table` — i.e. when the name denotes a *constraint* and not merely an index.
    fn constraint_marked_index(&self, table: &str, name: &str) -> Option<&Index> {
        self.get_index(name)
            .filter(|idx| idx.table_name == table && idx.constraint_kind().is_some())
    }

    /// Whether **any** constraint anywhere in the schema is named `name` — CHECK and
    /// FOREIGN KEY off the tables, PRIMARY KEY and UNIQUE off their constraint-marked
    /// indexes.
    ///
    /// Deliberately **schema-wide**, and the exact scope Postgres gives an
    /// *auto-generated* name: `ChooseConstraintName` scans `pg_constraint (conname,
    /// connamespace)` and `ChooseRelationName(isconstraint = true)` finishes with
    /// `ConstraintNameExists` over the same index. Both cover all four kinds, so a
    /// `UNIQUE` named `t_a_check` on **another** table pushes this table's next
    /// auto-named CHECK to `t_a_check1` (measured on 18.6).
    ///
    /// Only used to *skip* a candidate auto-name. The rejection gate for a **declared**
    /// name is [`Self::constraint_name_taken`], which stays table-scoped as
    /// `pg_constraint`'s unique index is (`ConstraintNameIsUsed`) — "deliberately not
    /// the same test as `ChooseConstraintName` uses". Plain (non-constraint) indexes are
    /// **not** here: an index named `r_a_fkey` does not push an FK's auto-name, since
    /// `ChooseConstraintName` never reads `pg_class`.
    #[must_use]
    pub fn constraint_name_exists(&self, name: &str) -> bool {
        self.tables.values().any(|t| {
            t.checks
                .iter()
                .map(|c| c.name.as_str())
                .chain(t.foreign_keys.iter().map(|fk| fk.name.as_str()))
                .any(|existing| existing == name)
        }) || self
            .indexes
            .get(name)
            .is_some_and(|idx| idx.constraint_kind().is_some())
    }

    /// Every foreign key that **references** `parent_table` — the parent side of the
    /// relationship — as `(child table, position in its `foreign_keys`, the key)`.
    ///
    /// The single reverse-FK walk in the engine. Four callers need it and each used to
    /// re-roll the same `tables.values().flat_map(|t| t.foreign_keys …)` filter:
    /// `TRUNCATE`'s cascade closure, `DELETE`'s `ON DELETE` worklist, `UPDATE`'s `ON
    /// UPDATE` worklist, and the DROP-time dependency resolver. A self-referencing FK
    /// is included (its declaring table *is* `parent_table`) — the callers that must
    /// treat it differently check that themselves.
    ///
    /// The position rides along because a constraint's *name* is not its identity: the
    /// DROP-time resolver removes by position, since two same-named FKs on one table
    /// would otherwise be indistinguishable (see `executor::dependencies::FkRef`).
    pub fn referencing_foreign_keys<'a>(
        &'a self,
        parent_table: &'a str,
    ) -> impl Iterator<Item = (&'a Table, usize, &'a ForeignKey)> + 'a {
        self.tables.values().flat_map(move |child| {
            child
                .foreign_keys
                .iter()
                .enumerate()
                .filter(move |(_, fk)| fk.parent_table == parent_table)
                .map(move |(i, fk)| (child, i, fk))
        })
    }

    /// Put `snapshot`'s catalog **content** back, keeping the stamps monotonic and
    /// the sequence counters where a rollback must leave them.
    ///
    /// Three separate rules, and the third is the one that is easy to miss:
    ///
    /// - `generation` stays where it is when it equals the snapshot's: nothing moved it
    ///   since the snapshot was taken, so no change a parse or a parameter inference reads
    ///   was made since, and the restored catalog is, to those readers, the one the live
    ///   stamp already names — a rollback of a block that ran only DML keeps every plan
    ///   cached against it. Otherwise the restore takes back a change that moved it, and
    ///   the stamp advances past the live value rather than returning to the snapshot's
    ///   lower one, so `restore_from` never hands a prepared-plan cache keyed `plan_gen ==
    ///   generation` a stamp it already gave another catalog. Equal stamps prove nothing
    ///   where a holder of `&mut Self` changed the catalog since the snapshot without
    ///   moving the stamp, or replaced it: a caller that lent the catalog out so passes
    ///   a snapshot whose stamp differs from the live one (`pglet::TransactionSession`'s
    ///   `lend_catalog`).
    /// - `next_oid` keeps the live high-water for the same reason: a restored one
    ///   would let a re-minted index reuse a freed storage `index_id`.
    /// - **A sequence counter is not rolled back — unless the statement replaced the
    ///   counter cell.** `nextval` and `setval` write the current cell in place and
    ///   are non-transactional (measured on 18.6 for a whole-block `ROLLBACK`, a
    ///   `ROLLBACK TO SAVEPOINT`, a caught `DO … EXCEPTION` and a failed `DO` alike), so those
    ///   sequences keep the **live** value; restoring it is a silent wrong of the
    ///   worst kind, since the counter walks back over ids already handed out and
    ///   the next insert collides on a primary key. `ALTER SEQUENCE … RESTART`,
    ///   `TRUNCATE … RESTART IDENTITY`, `DROP SEQUENCE`, and a `DROP TABLE`, `ALTER
    ///   TABLE … DROP COLUMN` or `DROP SCHEMA public` that drops a sequence are the
    ///   other kind — they give the sequence a new cell, and the rollback puts the old
    ///   one back **as the in-place advances left it**, which is neither the live value
    ///   nor the snapshot's. That third value arrives in `sequence_swaps` as
    ///   `(name, the last_value the outgoing cell held)`, collected before each
    ///   statement ran by [`Plan::swapped_sequences`](crate::planner::Plan::swapped_sequences);
    ///   the **first** entry for a name wins, since later ones replaced a cell this
    ///   region had already created. `start` and `owner` are ordinary catalog facts
    ///   and come back from the snapshot in every case.
    ///
    ///   A `DO` statement's replacements reach the list too, though that function lists
    ///   nothing for a `DO`: the `DO` block asks each of its statements as it runs them,
    ///   one that completes reports what it replaced in its result, and the host and any
    ///   enclosing `DO` add that to their own lists; one that **fails** puts its replaced
    ///   cells back itself before its error returns. A cell replaced by a route the list
    ///   does not name would have no outgoing value here, and the restored sequence would
    ///   keep the counter of whatever holds its name live, or its snapshot counter where
    ///   nothing does.
    ///
    /// One implementation for three callers — the whole-block rollback, the
    /// savepoint rollback (both `pglet::TransactionSession`) and the `DO` block's undo
    /// (a caught body's subtransaction, and a failed `DO` statement) — because the three rules above are one
    /// contract and a second copy of them would drift.
    pub fn restore_from(&mut self, snapshot: Self, sequence_swaps: &[(String, i64)]) {
        let live_gen = self.generation;
        let moved = live_gen != snapshot.generation;
        let live_oid = self.next_oid;
        let counters: Vec<(String, i64)> = snapshot
            .sequences
            .keys()
            .filter_map(|name| {
                let carried = sequence_swaps
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| *v)
                    .or_else(|| Some(self.sequences.get(name)?.last_value))?;
                Some((name.clone(), carried))
            })
            .collect();
        *self = snapshot;
        self.generation = if moved {
            live_gen.wrapping_add(1)
        } else {
            live_gen
        };
        self.next_oid = live_oid;
        for (name, last_value) in counters {
            if let Some(seq) = self.sequences.get_mut(&name) {
                seq.last_value = last_value;
            }
        }
    }

    /// What a `TRUNCATE` of `tables` touches: `(closure, restart_sequences)`.
    ///
    /// `closure` is the truncation set — the named tables plus, under `cascade`, every
    /// table that transitively FK-references one of them (via
    /// [`Schema::referencing_foreign_keys`], the single reverse-FK walk). A
    /// self-referencing FK is already in the set and so never blocks.
    ///
    /// `restart_sequences` is what `RESTART IDENTITY` rewrites: the closure's **owned**
    /// sequences (Postgres's `getOwnedSequences`, the `sequence → (table, attnum)`
    /// edge), empty when `restart_identity` is false. The set is *not* the columns'
    /// `NextVal` defaults, and the two differ in both directions (measured on 18.6). A
    /// sequence a column merely **references** without owning is left alone:
    /// `CREATE SEQUENCE s START WITH 100; CREATE TABLE t (x INT DEFAULT nextval('s'))`
    /// keeps counting across the truncate. And an **owned** sequence whose column
    /// carries no default — or carries a *different* one — still restarts. Reading the
    /// defaults instead therefore over-resets one sequence and under-resets another in
    /// the same statement.
    ///
    /// **One derivation, several readers, and `Plan::mutates_catalog` is why this is a
    /// catalog method rather than executor-local.** `execute_truncate` writes exactly the
    /// sequences returned here, `Plan::swapped_sequences` reads the same list for the
    /// cells a `RESTART` replaces, and `Plan::mutates_catalog` asks whether it is empty to
    /// decide whether the statement writes the catalog at all. A second derivation would
    /// let a reader and the write disagree, in both directions: a *syntactic* `RESTART
    /// IDENTITY` reading records a restart over a table owning no sequence, and a
    /// `CASCADE`-pulled child's sequence is in the write set, so a reader of only the
    /// *named* tables would miss that restart and let it outlive its own `ROLLBACK`.
    ///
    /// # Errors
    ///
    /// `0A000` when a table outside the closure FK-references one inside it and
    /// `cascade` was not given (Postgres's "cannot truncate a table referenced in a
    /// foreign key constraint"). A caller asking only whether the catalog is written
    /// reads that as *no* — the statement fails before writing anything.
    pub fn truncation_targets(
        &self,
        tables: &[String],
        cascade: bool,
        restart_identity: bool,
    ) -> crate::Result<(Vec<String>, Vec<String>)> {
        let mut closure: Vec<String> = tables.to_vec();
        let mut i = 0;
        while i < closure.len() {
            let parent = closure[i].clone();
            i += 1;
            let children: Vec<String> = self
                .referencing_foreign_keys(&parent)
                .map(|(child, _, _)| child.name.clone())
                .collect();
            for child in children {
                if closure.contains(&child) {
                    continue;
                }
                if !cascade {
                    // Postgres: "cannot truncate a table referenced in a foreign key
                    // constraint" (0A000) unless CASCADE is given. Erroring here keeps
                    // TRUNCATE from silently orphaning the child's referencing rows.
                    return Err(crate::Error::NotImplemented(format!(
                        "cannot truncate \"{parent}\" because \"{child}\" references it via a \
                         foreign key; use TRUNCATE ... CASCADE"
                    )));
                }
                closure.push(child);
            }
        }

        let restart_sequences: Vec<String> = if restart_identity {
            self.sequences
                .iter()
                .filter(|(_, s)| s.owner.as_ref().is_some_and(|o| closure.contains(&o.table)))
                .map(|(name, _)| name.clone())
                .collect()
        } else {
            Vec::new()
        };
        Ok((closure, restart_sequences))
    }

    /// The oids of every index on `table_name`, without removing them — the read-only
    /// half of [`Self::drop_indexes_for_table`], for a caller that must shed storage
    /// **before** it edits the catalog.
    ///
    /// `DROP TABLE a, b` needs the split: storage rolls back on the autocommit path and
    /// the catalog does not, so a fallible `storage.drop_index` sequenced after a catalog
    /// removal leaves an earlier target's indexes durably gone from `schema` while their
    /// storage entries come back — a key silently un-enforced on a table that still
    /// exists. Reading the oids here and removing the catalog rows in a second pass keeps
    /// every fallible call ahead of every mutation. The two must select identically, so
    /// each is the same filter over `table_name`; a caller that removes without shedding
    /// storage leaks the registered def instead.
    pub fn index_oids_for_table(&self, table_name: &str) -> Vec<u32> {
        self.indexes
            .values()
            .filter(|index| index.table_name == table_name)
            .map(Index::index_id)
            .collect()
    }

    /// Removes all indexes associated with a table from the catalog, returning the
    /// removed indexes' oids so the caller can shed them from storage in the same
    /// pass (the storage shed can't drift from the catalog removal). Called when
    /// dropping a table to cascade to dependent indexes.
    ///
    /// A caller sequencing a **fallible** storage shed after this one loses the ordering
    /// [`Self::index_oids_for_table`] exists to restore; `DROP TABLE`'s multi-name path
    /// uses that pair instead.
    pub fn drop_indexes_for_table(&mut self, table_name: &str) -> Vec<u32> {
        let oids: Vec<u32> = self
            .indexes
            .values()
            .filter(|index| index.table_name == table_name)
            .map(Index::index_id)
            .collect();
        self.indexes
            .retain(|_, index| index.table_name != table_name);
        oids
    }
}

#[cfg(test)]
mod object_name_tests {
    use super::{clip_name, make_object_name};

    /// A cut lands on a character boundary, giving the split character up whole.
    #[test]
    fn a_cut_never_splits_a_character() {
        let name = format!("n{}", "\u{fc}".repeat(3));
        assert_eq!(clip_name(&name, 4), format!("n{}", "\u{fc}"));
        assert_eq!(clip_name(&name, 5), format!("n{}", "\u{fc}".repeat(2)));
        assert_eq!(clip_name(&name, 0), "");
        assert_eq!(clip_name(&name, 100), name);
    }

    /// An empty second name is 18.6's absent one: one underscore, and the whole budget
    /// behind the label is the first name's.
    #[test]
    fn an_absent_second_name_takes_no_underscore() {
        assert_eq!(make_object_name("t", "", "pkey"), "t_pkey");
        let table = "t".repeat(63);
        assert_eq!(
            make_object_name(&table, "", "pkey"),
            format!("{}_pkey", "t".repeat(58))
        );
        assert_eq!(
            make_object_name(&table, "", "check1"),
            format!("{}_check1", "t".repeat(56))
        );
    }
}

#[cfg(test)]
mod uniqueness_serde {
    use super::{ConstraintKind, Uniqueness};

    #[test]
    fn postcard_layout_is_the_flat_unique_enforced_constraint_triple() {
        // The catalog persists through postcard, a positional format: `Uniqueness`
        // encodes through `UniquenessWire` as `unique: bool, enforced: bool,
        // constraint: Option<ConstraintKind>`, in that order. Pinned so a field
        // reorder is a loud failure rather than a silent misdecode.
        assert_eq!(
            postcard::to_allocvec(&Uniqueness::NonUnique).unwrap(),
            vec![0, 0, 0]
        );
        assert_eq!(
            postcard::to_allocvec(&Uniqueness::Unique {
                enforced: false,
                constraint: None
            })
            .unwrap(),
            vec![1, 0, 0]
        );
        assert_eq!(
            postcard::to_allocvec(&Uniqueness::for_constraint(ConstraintKind::PrimaryKey)).unwrap(),
            vec![1, 1, 1, 0]
        );
        assert_eq!(
            postcard::to_allocvec(&Uniqueness::for_constraint(ConstraintKind::Unique)).unwrap(),
            vec![1, 1, 1, 1]
        );
    }

    #[test]
    fn round_trips_and_normalizes_nonsense_bytes() {
        for u in [
            Uniqueness::NonUnique,
            Uniqueness::Unique {
                enforced: false,
                constraint: None,
            },
            Uniqueness::new(true, true),
            Uniqueness::for_constraint(ConstraintKind::PrimaryKey),
            Uniqueness::for_constraint(ConstraintKind::Unique),
        ] {
            let bytes = postcard::to_allocvec(&u).unwrap();
            assert_eq!(postcard::from_bytes::<Uniqueness>(&bytes).unwrap(), u);
        }
        // `!unique` with an enforced flag and/or an owning constraint — the
        // now-unrepresentable nonsense — reads back normalized to `NonUnique`.
        assert_eq!(
            postcard::from_bytes::<Uniqueness>(&[0, 1, 0]).unwrap(),
            Uniqueness::NonUnique
        );
        assert_eq!(
            postcard::from_bytes::<Uniqueness>(&[0, 0, 1, 0]).unwrap(),
            Uniqueness::NonUnique
        );
    }

    #[test]
    fn enum_type_postcard_field_layout_is_pinned() {
        use super::EnumType;
        // `EnumType` is the record the store magic moved for (`PGMI` → `PGMJ`), so it
        // gets the pin `Index` has and `Sequence` does not: postcard is positional, and
        // swapping `oid` and `labels` would decode a persisted enum silently — an OID
        // read as a label count. `Sequence` stays unpinned deliberately; no store bump
        // moved its shape, and adding a pin there would be a guard seeded by nothing.
        let ty = EnumType {
            oid: 16500,
            labels: vec!["sad".into(), "ok".into()],
        };
        // `oid` 16500 as a zigzag varint `[232, 129, 2]`, then the `Vec<String>`:
        // length `[2]`, `"sad"` `[3, 115, 97, 100]`, `"ok"` `[2, 111, 107]`.
        let bytes = postcard::to_allocvec(&ty).unwrap();
        assert_eq!(bytes, vec![232, 129, 2, 2, 3, 115, 97, 100, 2, 111, 107]);
        let back: EnumType = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, ty);
    }

    #[test]
    fn index_postcard_field_layout_is_pinned() {
        use super::{ConstraintKind, Index, IndexKey, Uniqueness};
        let idx = Index {
            name: "idx".into(),
            table_name: "t".into(),
            key: IndexKey::Columns(vec!["a".into()]),
            uniqueness: Uniqueness::for_constraint(ConstraintKind::PrimaryKey),
            predicate: None,
            nulls_not_distinct: false,
            oid: 1234,
        };
        // Golden on-disk layout (postcard, positional): pins the field ORDER, so a
        // future field reorder that would silently break persisted catalogs is a
        // loud test failure rather than a latent migration bug. `key` serializes as
        // the `IndexKey::Columns` variant tag `[0]` then the `Vec<String>` `[1, 1,
        // 97]` (one elem "a"); the `uniqueness` enum serializes through
        // `UniquenessWire` as the flat `[unique, enforced, constraint] = [1, 1,
        // Some(PrimaryKey) = 1, 0]` triple. Tail: `predicate` None `[0]`,
        // `nulls_not_distinct` `[0]`, `oid` 1234 as a zigzag varint `[164, 19]`.
        let bytes = postcard::to_allocvec(&idx).unwrap();
        assert_eq!(
            bytes,
            vec![
                3, 105, 100, 120, 1, 116, 0, 1, 1, 97, 1, 1, 1, 0, 0, 0, 164, 19
            ]
        );
        // ...and it round-trips with the uniqueness/oid intact.
        let back: Index = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(
            back.uniqueness,
            Uniqueness::for_constraint(ConstraintKind::PrimaryKey)
        );
        assert_eq!(back.constraint_kind(), Some(ConstraintKind::PrimaryKey));
        assert_eq!(back.oid, 1234);
    }

    #[test]
    fn typeoid_postcard_encodes_as_oid() {
        use crate::types::TypeOid;
        // A `Column.type_oid` persists via postcard, so `TypeOid` must key its
        // encoding on the u32 OID, NOT the variant ordinal — otherwise inserting a
        // variant mid-enum (e.g. `Time` between `Date` and `Numeric`) would shift
        // every later variant's tag and misread a persisted catalog. `Int4`'s OID
        // is 23 (< 128 → a one-byte LEB128 varint).
        assert_eq!(postcard::to_allocvec(&TypeOid::Int4).unwrap(), vec![23]);
        // A high OID is a two-byte varint: `Numeric` (1700) → [164, 13].
        assert_eq!(
            postcard::to_allocvec(&TypeOid::Numeric).unwrap(),
            vec![164, 13]
        );
        // `Name` is a one-byte OID (19); `"char"` (18) sits just below it.
        assert_eq!(postcard::to_allocvec(&TypeOid::Name).unwrap(), vec![19]);
        // Round-trips for a spread including the mid-enum insert (`Time` = 1083) and
        // its array OID — order-independence made concrete.
        for oid in [
            TypeOid::Bool,
            TypeOid::Int4,
            TypeOid::Time,
            TypeOid::Numeric,
            TypeOid::TimeArray,
            TypeOid::JsonbArray,
            // `Name` (19) and `NameArray` (1003) sit at the **end** of the variant
            // list but carry low OIDs, so they are the pair that would break first
            // if the encoding ever keyed on the ordinal again.
            TypeOid::Name,
            TypeOid::NameArray,
        ] {
            let bytes = postcard::to_allocvec(&oid).unwrap();
            assert_eq!(postcard::from_bytes::<TypeOid>(&bytes).unwrap(), oid);
        }
    }
}

#[cfg(test)]
mod serialize_determinism {
    use super::{EnumType, Schema, Sequence, serialize_schema};

    /// Two independently-built schemas with identical content must serialize to
    /// identical bytes, regardless of the order entries were inserted.
    ///
    /// `serialize_schema` feeds the storage layer's byte-equality catalog-change
    /// gate; if the encoding weren't canonical, a reopened schema (rebuilt into
    /// fresh maps) would re-serialize to different bytes and trip a spurious
    /// `SYS_CATALOG` rewrite on the first commit after open. The `BTreeMap`-backed
    /// catalog makes the encoding deterministic — a regression to `HashMap` (whose
    /// per-instance `RandomState` randomizes iteration order) would make these two
    /// independent instances disagree.
    #[test]
    fn serialize_schema_is_canonical_regardless_of_insertion_order() {
        let mut forward = Schema::new();
        let mut reverse = Schema::new();
        let names = ["zebra", "apple", "mango", "cherry", "delta", "kilo"];
        for (i, name) in names.iter().enumerate() {
            forward.enums.insert(
                (*name).to_string(),
                EnumType {
                    oid: 20000 + i as i32,
                    labels: vec![format!("m{i}")],
                },
            );
            forward
                .sequences
                .insert((*name).to_string(), Sequence::new(i as i64));
        }
        for (i, name) in names.iter().enumerate().rev() {
            reverse.enums.insert(
                (*name).to_string(),
                EnumType {
                    oid: 20000 + i as i32,
                    labels: vec![format!("m{i}")],
                },
            );
            reverse
                .sequences
                .insert((*name).to_string(), Sequence::new(i as i64));
        }
        assert_eq!(
            serialize_schema(&forward).unwrap(),
            serialize_schema(&reverse).unwrap(),
            "catalog serialization must be canonical (insertion-order independent)"
        );
    }

    /// A reopen round-trip (serialize → deserialize → serialize) must be
    /// byte-stable, the exact invariant the catalog-change gate relies on.
    #[test]
    fn serialize_schema_survives_reserialization_byte_stable() {
        let mut schema = Schema::new();
        for (i, name) in ["one", "two", "three", "four"].iter().enumerate() {
            schema
                .sequences
                .insert((*name).to_string(), Sequence::new(i as i64));
            schema.enums.insert(
                (*name).to_string(),
                EnumType {
                    oid: 30000 + i as i32,
                    labels: vec![format!("v{i}")],
                },
            );
        }
        let first = serialize_schema(&schema).unwrap();
        let reopened: Schema = postcard::from_bytes(&first).unwrap();
        let second = serialize_schema(&reopened).unwrap();
        assert_eq!(
            first, second,
            "serialize→deserialize→serialize must be byte-stable"
        );
    }
}

#[cfg(test)]
mod constraint_name_scopes {
    use super::{
        CheckConstraint, Column, ConstraintKind, ForeignKey, Index, IndexKey, ReferentialAction,
        Schema, Table, Uniqueness,
    };
    use crate::planner::{BinaryOp, Expr, Literal};
    use crate::types::TypeOid;

    fn table_with(name: &str, checks: Vec<CheckConstraint>, fks: Vec<ForeignKey>) -> Table {
        Table {
            name: name.to_string(),
            schema: "public".to_string(),
            columns: vec![Column {
                name: "a".to_string(),
                type_oid: TypeOid::Int4,
                nullable: true,
                position: 1,
                default: None,
                enum_type: None,
                numeric_typmod: None,
                varchar_length: None,
                temporal_precision: None,
                identity: None,
            }],
            oid: 1,
            checks,
            foreign_keys: fks,
        }
    }

    fn check(name: &str) -> CheckConstraint {
        CheckConstraint {
            name: name.to_string(),
            expr: Expr::BinaryOp {
                left: Box::new(Expr::Column {
                    table: None,
                    name: "a".to_string(),
                }),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Literal::Int(0))),
            },
        }
    }

    fn fk(name: &str, table: &str) -> ForeignKey {
        ForeignKey {
            name: name.to_string(),
            columns: vec!["a".to_string()],
            parent_table: table.to_string(),
            parent_columns: vec!["a".to_string()],
            on_delete: ReferentialAction::NoAction,
            on_update: ReferentialAction::NoAction,
        }
    }

    fn index(name: &str, table: &str, uniqueness: Uniqueness) -> Index {
        Index {
            name: name.to_string(),
            table_name: table.to_string(),
            key: IndexKey::Columns(vec!["a".to_string()]),
            uniqueness,
            predicate: None,
            nulls_not_distinct: false,
            oid: 100,
        }
    }

    /// `t1` carries `ck`/`fkey`; `t2` carries a constraint-marked UNIQUE index `uq`
    /// and a standalone (non-constraint) index `plain`.
    fn schema() -> Schema {
        let mut s = Schema::new();
        s.create_table(table_with("t1", vec![check("ck")], vec![fk("fkey", "t2")]));
        s.create_table(table_with("t2", Vec::new(), Vec::new()));
        s.create_index(index(
            "uq",
            "t2",
            Uniqueness::for_constraint(ConstraintKind::Unique),
        ));
        s.create_index(index("plain", "t2", Uniqueness::NonUnique));
        s
    }

    #[test]
    fn declared_name_gate_is_per_relation() {
        let s = schema();
        // The direction the corpus asserts everywhere: a name on *this* table is taken,
        // across all four kinds.
        assert!(s.constraint_name_taken("t1", "ck"));
        assert!(s.constraint_name_taken("t1", "fkey"));
        assert!(s.constraint_name_taken("t2", "uq"));

        // The direction nothing asserted, and the one that bricks valid DDL when it
        // goes wrong: the same name on a *different* relation is free
        // (`ConstraintNameIsUsed` keys on `(conrelid, conname)`). Widening this to the
        // schema-wide check leaves the whole fixture corpus green while rejecting
        // `CREATE TABLE a (x INT CONSTRAINT positive_amount CHECK (x > 0));
        //  CREATE TABLE b (x INT CONSTRAINT positive_amount CHECK (x > 0));`.
        assert!(!s.constraint_name_taken("t2", "ck"));
        assert!(!s.constraint_name_taken("t2", "fkey"));
        assert!(!s.constraint_name_taken("t1", "uq"));
    }

    #[test]
    fn auto_name_scope_is_schema_wide_but_constraints_only() {
        let s = schema();
        // `ChooseConstraintName` scans `pg_constraint (conname, connamespace)`: every
        // kind, every relation. So a name on another table pushes an auto-name aside
        // even though the declared gate above lets it through.
        assert!(s.constraint_name_exists("ck"));
        assert!(s.constraint_name_exists("fkey"));
        assert!(s.constraint_name_exists("uq"));

        // …and **only** `pg_constraint`: a plain `CREATE INDEX` is a relation, not a
        // constraint, so it does not push a CHECK's or FK's auto-name. That asymmetry
        // is why a PK/UNIQUE auto-name additionally clears `relation_name_taken` (its
        // index carries the name) and a CHECK/FK auto-name does not.
        assert!(!s.constraint_name_exists("plain"));
        assert!(s.relation_name_taken("plain"));
        assert!(!s.constraint_name_exists("nothing"));
    }
}

#[cfg(test)]
mod schema_enum_postcard_layout {
    use super::{
        ColumnDefault, ConstraintKind, IdentityGeneration, IndexKey, ReferentialAction, Uniqueness,
    };

    /// One enum's every-variant tag pin.
    fn pin_tags<T: serde::Serialize + core::fmt::Debug>(enum_name: &str, pins: &[(u8, T)]) {
        for (expected, value) in pins {
            let bytes = postcard::to_allocvec(value).expect("variant serializes");
            assert_eq!(
                bytes[0], *expected,
                "{enum_name}::{value:?} no longer encodes as postcard variant {expected} \
                 — see this test's doc comment; the persisted catalog decodes by position"
            );
        }
    }

    /// Pins the postcard variant discriminants of the persisted-`Schema` enums that no
    /// other pin covers: [`IdentityGeneration`], [`ReferentialAction`] and [`IndexKey`].
    ///
    /// Same class as `column_default_postcard_variant_tags_are_pinned` and
    /// `planner::expr`'s two, one level out: these ride the `Schema` postcard bytes
    /// through `Column.identity`, `ForeignKey.on_delete`/`.on_update` and `Index.key`,
    /// they encode positionally, and there is no migration. Exchanging
    /// `IdentityGeneration`'s two variants turns every stored `GENERATED ALWAYS` column
    /// into a `GENERATED BY DEFAULT` one — the `428C9` refusal of an explicit value
    /// silently stops firing — with the store magic unmoved and every gate green.
    ///
    /// **The set is derived, not inherited**: every `Serialize`-deriving enum the
    /// `Schema` type graph reaches, minus the ones already pinned. Already covered and
    /// deliberately absent from the pin list: `ColumnDefault` (the module below, whose
    /// *append* the guard beneath this one covers), `ConstraintKind` (positional; its
    /// tags sit inside `uniqueness_serde`'s byte triples, its *append* is caught by the
    /// guard beneath) and `Uniqueness` (no variant tags of its own — it encodes through
    /// the flat `UniquenessWire`, whose *field order* those same triples pin),
    /// `Index`'s *field* layout (`index_postcard_field_layout_is_pinned`) and
    /// [`crate::types::TypeOid`] (`typeoid_postcard_encodes_as_oid` — keyed on the OID,
    /// so it alone may be reordered). Not in the set at all: `CatalogTable`, which no
    /// persisted type holds, and `RelationKind`, which derives no `Serialize`. The
    /// `Expr` graph below `ColumnDefault::Expr` and `IndexKey::Exprs` is `planner::expr`'s
    /// `expr_sub_enum_postcard_variant_tags_are_pinned`.
    ///
    /// **If this test fails**, you reordered or removed a variant. Move it back; a new
    /// one goes at the end with a new row — never renumber to match, bump the magic.
    #[test]
    fn schema_enum_postcard_variant_tags_are_pinned() {
        pin_tags(
            "IdentityGeneration",
            &[
                (0, IdentityGeneration::Always),
                (1, IdentityGeneration::ByDefault),
            ],
        );
        pin_tags(
            "ReferentialAction",
            &[
                (0, ReferentialAction::NoAction),
                (1, ReferentialAction::Restrict),
                (2, ReferentialAction::Cascade),
                (3, ReferentialAction::SetNull),
            ],
        );
        pin_tags(
            "IndexKey",
            &[
                (0, IndexKey::Columns(Vec::new())),
                (1, IndexKey::Exprs(Vec::new())),
            ],
        );
    }

    /// The **length** half of the pin above, as a compile error rather than an
    /// assertion.
    ///
    /// A tag list cannot notice a variant *appended* without a row — an append is not
    /// itself a misdecode (postcard tolerates it; no store carries the new tag), but it
    /// leaves that variant unpinned, and the next mid-enum insertion then moves it
    /// silently. These matches are exhaustive, so the append is a compile error here and
    /// the row gets written while the window is still shut.
    #[test]
    fn every_pinned_schema_enum_variant_has_a_row() {
        // `ColumnDefault`'s tags are pinned by `column_default_postcard_layout`; its
        // append is caught here, the file's one append guard.
        fn _column_default(v: &ColumnDefault) {
            match v {
                ColumnDefault::Literal(_)
                | ColumnDefault::Now
                | ColumnDefault::CurrentDate
                | ColumnDefault::RandomUuid
                | ColumnDefault::NextVal(_)
                | ColumnDefault::CurrentTime
                | ColumnDefault::TransactionTimestamp
                | ColumnDefault::StatementTimestamp
                | ColumnDefault::ClockTimestamp
                | ColumnDefault::CurrentTimestamp
                | ColumnDefault::Expr(_) => (),
            }
        }
        // `Uniqueness` and `ConstraintKind` are the same shape as `ColumnDefault`: tags
        // pinned by `uniqueness_serde`'s byte triples, appends caught here.
        fn _uniqueness(v: Uniqueness) {
            match v {
                Uniqueness::NonUnique | Uniqueness::Unique { .. } => (),
            }
        }
        fn _constraint_kind(v: ConstraintKind) {
            match v {
                ConstraintKind::PrimaryKey | ConstraintKind::Unique => (),
            }
        }
        fn _identity(v: IdentityGeneration) {
            match v {
                IdentityGeneration::Always | IdentityGeneration::ByDefault => (),
            }
        }
        fn _referential(v: ReferentialAction) {
            match v {
                ReferentialAction::NoAction
                | ReferentialAction::Restrict
                | ReferentialAction::Cascade
                | ReferentialAction::SetNull => (),
            }
        }
        fn _index_key(v: &IndexKey) {
            match v {
                IndexKey::Columns(_) | IndexKey::Exprs(_) => (),
            }
        }
    }
}

#[cfg(test)]
mod column_default_postcard_layout {
    use super::ColumnDefault;
    use crate::types::Value;

    /// The postcard variant tag `default` encodes to — its leading byte.
    fn tag(default: &ColumnDefault) -> u8 {
        postcard::to_allocvec(default).expect("ColumnDefault serializes")[0]
    }

    /// Pins the postcard **variant discriminants** of [`ColumnDefault`].
    ///
    /// The twin of `planner::expr`'s `expr_postcard_variant_tags_are_pinned`, and it
    /// was missing: `ColumnDefault` is persisted into the catalog `Column` exactly as
    /// `Expr` is persisted into a CHECK, a serde-derived enum encodes its variant as a
    /// **positional** index, and there is no `ColumnDefault` migration — so the tags
    /// are on-disk format. Only the doc comment on the enum said so, and a doc comment
    /// is not a gate. A store written before a mid-enum insertion would decode a
    /// `now()` default as a `CURRENT_DATE` one, or a `nextval` name as a literal, with
    /// no error anywhere.
    ///
    /// **Every** variant is pinned, not a first/last/anchor sample. A sample pins the
    /// enum's *length* — the last variant's tag moves whenever anything is inserted
    /// ahead of it — but leaves the interior permutable, and a swap of two unsampled
    /// neighbours is the mutation with no other symptom: `CurrentDate` and `RandomUuid`
    /// exchanged would silently decode a persisted `DEFAULT CURRENT_DATE` as
    /// `gen_random_uuid()`, on a store the magic still admits.
    ///
    /// **If this test fails**, you inserted (or removed, or reordered) a variant
    /// somewhere other than the end. Move it to the end and append its row below. Do
    /// **not** just edit the numbers to match — bump the store magic in
    /// `pglet_storage`'s `double_meta` instead, so old stores fail loud on reopen
    /// rather than misdecoding.
    #[test]
    fn column_default_postcard_variant_tags_are_pinned() {
        // Every variant, in declaration order, with the tag it must keep forever.
        let pins: [(u8, ColumnDefault); 11] = [
            (0, ColumnDefault::Literal(Value::Int8(1))),
            (1, ColumnDefault::Now),
            (2, ColumnDefault::CurrentDate),
            (3, ColumnDefault::RandomUuid),
            (4, ColumnDefault::NextVal("s".to_string())),
            (5, ColumnDefault::CurrentTime),
            (6, ColumnDefault::TransactionTimestamp),
            (7, ColumnDefault::StatementTimestamp),
            (8, ColumnDefault::ClockTimestamp),
            (9, ColumnDefault::CurrentTimestamp),
            (
                10,
                ColumnDefault::Expr(crate::planner::Expr::Bound(Value::Int8(1))),
            ),
        ];
        for (expected, default) in &pins {
            assert_eq!(
                tag(default),
                *expected,
                "{default:?} no longer encodes as postcard variant {expected} — see \
                 this test's doc comment before changing this number; persisted \
                 column defaults decode by position"
            );
        }

        // No trailing length anchor: over an every-variant list it would restate the
        // loop's own last row. An appended variant is caught by
        // `schema_enum_postcard_layout`'s `every_pinned_schema_enum_variant_has_a_row`
        // exhaustive match instead.

        // And the tags round-trip, so the pin is over real encode/decode rather than a
        // leading-byte coincidence.
        for (_, default) in pins {
            let bytes = postcard::to_allocvec(&default).expect("serializes");
            assert_eq!(
                postcard::from_bytes::<ColumnDefault>(&bytes).expect("deserializes"),
                default
            );
        }
    }
}

#[cfg(test)]
mod type_modifier_spelling {
    use super::{Column, NumericTypmod, pg_type_modifier_spelling};
    use crate::types::{TypeOid, type_oid_carries_modifier};

    /// A `Column` carrying nothing but the fields `pg_type_modifier` reads.
    fn col(type_oid: TypeOid) -> Column {
        Column {
            name: "c".to_string(),
            type_oid,
            nullable: true,
            position: 1,
            default: None,
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            identity: None,
            temporal_precision: None,
        }
    }

    /// Encode then decode: every modifier a column can declare comes back spelled as
    /// 18.6's `format_type` spells it. Every non-`-1` typmod pglet puts on the wire
    /// originates in the encoder — the two base-table-column sites call it, and the
    /// recursive-CTE fold either passes one of those along or answers `-1` — so this
    /// pair is the whole round trip.
    #[test]
    fn a_columns_modifier_round_trips_through_the_spelling() {
        let mut varchar = col(TypeOid::Varchar);
        varchar.varchar_length = Some(5);
        assert_eq!(varchar.pg_type_modifier(), 9);
        assert_eq!(
            pg_type_modifier_spelling(TypeOid::Varchar, 9),
            Some("(5)".to_string())
        );

        let mut numeric = col(TypeOid::Numeric);
        numeric.numeric_typmod = Some(NumericTypmod {
            precision: 6,
            scale: 2,
        });
        assert_eq!(numeric.pg_type_modifier(), 393_222);
        assert_eq!(
            pg_type_modifier_spelling(TypeOid::Numeric, 393_222),
            Some("(6,2)".to_string())
        );

        // `numerictypmodout` always prints both components, so a `numeric(6)` reads
        // back as `(6,0)` — 18.6's own spelling, not a lost scale.
        numeric.numeric_typmod = Some(NumericTypmod {
            precision: 6,
            scale: 0,
        });
        assert_eq!(
            pg_type_modifier_spelling(TypeOid::Numeric, numeric.pg_type_modifier()),
            Some("(6,0)".to_string())
        );

        // A negative scale is what the eleven-bit sign extension is for.
        numeric.numeric_typmod = Some(NumericTypmod {
            precision: 6,
            scale: -3,
        });
        assert_eq!(
            pg_type_modifier_spelling(TypeOid::Numeric, numeric.pg_type_modifier()),
            Some("(6,-3)".to_string())
        );

        let mut ts = col(TypeOid::Timestamp);
        ts.temporal_precision = Some(3);
        assert_eq!(ts.pg_type_modifier(), 3);
        assert_eq!(
            pg_type_modifier_spelling(TypeOid::Timestamp, 3),
            Some("(3)".to_string())
        );

        // An `interval`'s typmod packs the range mask beside the precision, so the
        // decode has to mask it back off: `2147418115` is `interval(3)`, measured.
        let mut interval = col(TypeOid::Interval);
        interval.temporal_precision = Some(3);
        assert_eq!(interval.pg_type_modifier(), 2_147_418_115);
        assert_eq!(
            pg_type_modifier_spelling(TypeOid::Interval, 2_147_418_115),
            Some("(3)".to_string())
        );

        // An array answers at its element's width, on the same typmod the scalar
        // beside it carries.
        let mut varchar_array = col(TypeOid::VarcharArray);
        varchar_array.varchar_length = Some(3);
        assert_eq!(varchar_array.pg_type_modifier(), 7);
        assert_eq!(
            pg_type_modifier_spelling(TypeOid::VarcharArray, 7),
            Some("(3)".to_string())
        );
    }

    /// A negative typmod is the absence of a modifier, whatever the type.
    #[test]
    fn no_modifier_spells_nothing() {
        assert_eq!(pg_type_modifier_spelling(TypeOid::Varchar, -1), None);
        assert_eq!(pg_type_modifier_spelling(TypeOid::Numeric, -1), None);
        assert_eq!(pg_type_modifier_spelling(TypeOid::Int4, 9), None);
        assert_eq!(pg_type_modifier_spelling(TypeOid::Text, 9), None);
    }

    /// The membership predicate — not this match — is what says a modifier can be
    /// there, so every type it admits must have a spelling arm. A type joining
    /// `type_oid_carries_modifier`'s `true` half without one here reddens this, rather
    /// than silently printing a bare name where 18.6 prints a width.
    #[test]
    fn every_modifier_carrying_type_has_a_spelling() {
        let mut seen = 0;
        for raw in 0..5000_i32 {
            let Some(oid) = TypeOid::from_oid(raw) else {
                continue;
            };
            seen += 1;
            if type_oid_carries_modifier(oid) {
                assert!(
                    pg_type_modifier_spelling(oid, 5).is_some(),
                    "{oid:?} carries a modifier but has no spelling arm"
                );
            } else {
                assert_eq!(
                    pg_type_modifier_spelling(oid, 5),
                    None,
                    "{oid:?} carries no modifier but spells one"
                );
            }
        }
        assert!(seen >= 30, "the OID sweep found only {seen} types");
    }
}
