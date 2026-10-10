use thiserror::Error;

/// Catalog-related errors (Postgres class 42 - Syntax Error or Access Rule Violation).
#[derive(Debug, Error)]
pub enum CatalogError {
    /// A statement naming a relation the catalog does not hold. 18.6's noun is
    /// **`relation`**, not `table` — `parserOpenTable` words it that way for every
    /// kind, and this is the most-raised message in the engine (measured on 18.6:
    /// `relation "no_such_table_xyz" does not exist`). The `DROP … IF EXISTS` *skip
    /// notice* is the one place 18.6 says `table` (`DropErrorMsgNonExistent` words
    /// that one per expected kind), and it does not route through here.
    #[error("relation \"{table}\" does not exist")]
    TableNotFound { table: String },

    #[error("missing FROM-clause entry for table \"{table}\"")]
    MissingFromClauseEntry { table: String },

    #[error("relation \"{table}\" already exists")]
    TableAlreadyExists { table: String },

    #[error("column \"{column}\" of relation \"{table}\" does not exist")]
    ColumnNotFound { table: String, column: String },

    #[error("column \"{column}\" of relation \"{table}\" already exists")]
    ColumnAlreadyExists { table: String, column: String },

    /// One (folded) column name claimed twice by a list that may claim it once. The
    /// sentence is shared by two families with different 18.6 producers and different
    /// orderings, so read the producer rather than the variant:
    ///
    /// - A `CREATE TABLE`'s **column definition** list, from `MergeAttributes`, which
    ///   walks the list once the parse-analysis passes are behind it — so it loses to
    ///   the column types and to the index constraints, and beats the relation's own
    ///   `42P07`.
    /// - A statement's **target column** list — an `INSERT`'s, from
    ///   `checkInsertTargets`, and a `COPY`'s in either direction, from
    ///   `CopyGetAttnums` — where the claim interleaves with the name resolution one
    ///   list item at a time. `planner::claim_target_column` is that claim, and
    ///   `docs/names.md` carries the ordering each of its callers owns.
    ///
    /// Distinct from [`Self::ColumnAlreadyExists`], which is the `ALTER TABLE … ADD
    /// COLUMN` collision against a column the catalog already holds: different
    /// sentence, different route, same `42701`.
    #[error("column \"{column}\" specified more than once")]
    DuplicateColumnName { column: String },

    /// A column name a relation cannot take because the heap already owns it.
    /// 18.6's six are `ctid`, `xmin`, `cmin`, `xmax`, `cmax` and `tableoid` —
    /// `heap.c`'s `SysAtt[]`, each measured on the oracle; `oid` is **not** among
    /// them and is an ordinary column name on both engines.
    ///
    /// Raised where a statement adds a name to a relation's column list —
    /// `CREATE TABLE` and `ALTER TABLE … ADD COLUMN` — at the
    /// position each engine puts it: `CheckAttributeNamesTypes` for `CREATE TABLE`
    /// (behind the duplicate-name check, ahead of the relation's `42P07`) and
    /// `check_for_column_name_collision` for `ALTER TABLE … ADD COLUMN` (ahead of
    /// the declared type, and **ahead of the `IF NOT EXISTS` skip**, which only
    /// fires for a collision with a *user* column).
    #[error("column name \"{column}\" conflicts with a system column name")]
    SystemColumnName { column: String },

    /// One `PRIMARY KEY` / `UNIQUE` key list naming the same column twice. The
    /// check is the **constraint's**, not its index's: `CREATE INDEX t_i ON t (a,
    /// a)` is accepted by both engines, as is a foreign key's *referencing* list.
    ///
    /// 18.6 raises it per key inside `transformIndexConstraint`, against the keys
    /// already accepted — which is why the position differs by route. On
    /// `CREATE TABLE` each key is resolved before its own duplicate test, so
    /// `PRIMARY KEY (a, nope, a)` is `42703`; on `ALTER TABLE … ADD CONSTRAINT`
    /// that resolution is skipped (`!cxt->isalter`) and deferred to `DefineIndex`,
    /// so the same spelling is `42701`. Both measured on 18.6.
    #[error("column \"{column}\" appears twice in {} constraint", if *primary { "primary key" } else { "unique" })]
    DuplicateKeyColumn {
        column: String,
        /// `true` for `PRIMARY KEY`, `false` for `UNIQUE` — the one fact the two
        /// sentences differ by.
        primary: bool,
    },

    #[error("column \"{column}\" does not exist")]
    ColumnNotFoundInExpr { column: String },

    #[error("index \"{index}\" does not exist")]
    IndexNotFound { index: String },

    #[error("relation \"{index}\" already exists")]
    IndexAlreadyExists { index: String },

    /// `CREATE SEQUENCE` naming a relation that already exists. A sequence is a
    /// relation in `pg_class`, so the collision is with the whole namespace (a
    /// table, an index, or another sequence) and Postgres reports it exactly as it
    /// reports a duplicate table: `42P07`, `relation "<n>" already exists`.
    #[error("relation \"{sequence}\" already exists")]
    SequenceAlreadyExists { sequence: String },

    #[error("constraint \"{constraint}\" for relation \"{table}\" already exists")]
    ConstraintAlreadyExists { constraint: String, table: String },

    /// A second `PRIMARY KEY` on one table. Postgres allows exactly one, and says so
    /// with its own SQLSTATE (`42P16`, `invalid_table_definition`) rather than a name
    /// collision — the two primary keys may be perfectly well named.
    ///
    /// Not a cosmetic check: `Schema::primary_key_index` finds *a* primary key by
    /// scanning the name-sorted index map, so a table with two would have its FK
    /// default-key resolution and its `GROUP BY` functional-dependency decided by
    /// index-name order.
    #[error("multiple primary keys for table \"{table}\" are not allowed")]
    MultiplePrimaryKeys { table: String },
}

impl CatalogError {
    /// Returns the SQLSTATE code for this error.
    // arms grouped one-per-variant to mirror Postgres's SQLSTATE catalog; TableAlreadyExists, IndexAlreadyExists and SequenceAlreadyExists intentionally share 42P07 (duplicate_table) but are kept separate for the per-variant mapping
    #[allow(clippy::match_same_arms)]
    pub const fn sqlstate(&self) -> &'static str {
        match self {
            Self::TableNotFound { .. } => "42P01",
            Self::MissingFromClauseEntry { .. } => "42P01", // undefined_table (errorMissingRTE)
            Self::TableAlreadyExists { .. } => "42P07",
            Self::ColumnNotFound { .. } | Self::ColumnNotFoundInExpr { .. } => "42703",
            Self::ColumnAlreadyExists { .. } => "42701",
            Self::DuplicateColumnName { .. } => "42701",
            Self::SystemColumnName { .. } => "42701",
            Self::DuplicateKeyColumn { .. } => "42701",
            Self::IndexNotFound { .. } => "42704", // undefined_object
            Self::IndexAlreadyExists { .. } => "42P07", // duplicate_table
            Self::SequenceAlreadyExists { .. } => "42P07", // duplicate_table
            Self::ConstraintAlreadyExists { .. } => "42710", // duplicate_object
            Self::MultiplePrimaryKeys { .. } => "42P16", // invalid_table_definition
        }
    }
}

/// The relation kind a wrong-kind error found where it wanted a different one.
///
/// Postgres puts this in the error's DETAIL line ("This operation is not supported
/// for tables."), and so does pglet — the kind is rendered into the `D` field by
/// `error_fields`, not into the message. A sum type rather than a `&'static str` so
/// the call sites cannot spell the same kind two ways.
///
/// Every kind is reachable from every side, because pglet's relation namespace is
/// Postgres's `pg_class`: a name resolves to exactly one of the three, and any
/// statement can name the wrong one (`DROP TABLE <an index>`, `DROP INDEX <a table>`,
/// `OWNED BY <a sequence>.<c>` — all `42809` on 18.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Table,
    Index,
    Sequence,
}

impl RelationKind {
    /// The kind as an article + noun (`a table`, `an index`), for a message that
    /// names the kind inline rather than as Postgres's DETAIL sentence.
    #[must_use]
    pub const fn article_noun(self) -> &'static str {
        match self {
            Self::Table => "a table",
            Self::Index => "an index",
            Self::Sequence => "a sequence",
        }
    }
}

impl std::fmt::Display for RelationKind {
    /// The **plural** noun, because the one sentence this kind is rendered into is
    /// Postgres's `This operation is not supported for indexes.` DETAIL line — now a
    /// real `D` field rather than message text (see `error_fields`). Spelled out per
    /// arm rather than as `noun + "s"`, since `index` does not pluralize that way.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Table => "tables",
            Self::Index => "indexes",
            Self::Sequence => "sequences",
        })
    }
}

/// Which statement raised a [`Error::WrongObjectType`] — see its `route` field.
///
/// Not a cosmetic distinction: 18.6 words the *message* identically from two
/// functions and only one of them offers a hint, so a single variant that always hinted
/// would be sending a field the oracle omits on half its call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrongTypeRoute {
    /// `DROP TABLE` / `DROP INDEX` / `DROP SEQUENCE` — `DropErrorMsgWrongType`, which
    /// attaches the `Use DROP <KIND> …` hint for the kind the name actually holds.
    Drop,
    /// `TRUNCATE` — `truncate_check_rel`, message only.
    Truncate,
}

/// The catalog object a `DROP` names, for a dependency error's message.
///
/// The *shape* is Postgres's `getObjectDescription` — `table <t>` / `column <c> of
/// table <t>` — under pglet's own quoting convention. (Postgres leaves those
/// identifiers unquoted: `cannot drop table dtr_par because …`; pglet quotes them, as
/// it does in every other message. Message text is not compared against the oracle;
/// the SQLSTATE is.)
///
/// A sum type rather than a pre-formatted `String` field, so the shapes of
/// [`Error::DependentForeignKey`] and [`Error::DependentColumnDefault`] cannot be
/// spelled inconsistently by their call sites (`DROP TABLE`,
/// `ALTER TABLE … DROP COLUMN`, `DROP SEQUENCE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropTargetName {
    /// A whole table (`DROP TABLE t`).
    Table { table: String },
    /// One column of a table (`ALTER TABLE t DROP COLUMN c`).
    Column { table: String, column: String },
    /// A sequence (`DROP SEQUENCE s`).
    Sequence { sequence: String },
    /// A user type (`DROP TYPE e`).
    Type { type_name: String },
}

impl DropTargetName {
    /// The spelling Postgres's `getObjectDescription` uses in a dependency message —
    /// identifiers **unquoted** (`table p_dep`, `column a of table t_dep2`).
    ///
    /// pglet quotes identifiers everywhere else; the `2BP01` family is the exception
    /// because its whole message, detail and hint are now compared byte-for-byte
    /// against 18.6's, which leaves them bare.
    #[must_use]
    pub fn unquoted(&self) -> String {
        match self {
            Self::Table { table } => format!("table {table}"),
            Self::Column { table, column } => format!("column {column} of table {table}"),
            Self::Sequence { sequence } => format!("sequence {sequence}"),
            Self::Type { type_name } => format!("type {type_name}"),
        }
    }
}

impl std::fmt::Display for DropTargetName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Table { table } => write!(f, "table \"{table}\""),
            Self::Column { table, column } => {
                write!(f, "column \"{column}\" of table \"{table}\"")
            }
            Self::Sequence { sequence } => write!(f, "sequence \"{sequence}\""),
            Self::Type { type_name } => write!(f, "type \"{type_name}\""),
        }
    }
}

/// A column `DEFAULT nextval(<sequence>)` standing in the way of a `DROP` — Postgres's
/// `pg_attrdef` → sequence NORMAL edge, and the payload of
/// [`Error::DependentColumnDefault`].
///
/// Boxed into the variant so the three names plus the target stay off every `Result`
/// frame in the engine (see [`MAX_ERROR_SIZE`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDefaultDependency {
    /// What the statement is trying to drop.
    pub target: DropTargetName,
    /// The table whose column carries the default.
    pub table: String,
    /// The column whose default names the sequence.
    pub column: String,
    /// The sequence the default draws from.
    pub sequence: String,
}

impl std::fmt::Display for ColumnDefaultDependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only the target is in the message; the other three names are the DETAIL
        // line's, built by `error_fields`.
        let Self { target, .. } = self;
        write!(
            f,
            "cannot drop {} because other objects depend on it",
            target.unquoted()
        )
    }
}

/// A stored expression converting to an enum, standing in the way of a `DROP TYPE`.
///
/// Postgres's NORMAL edge from the constraint, the default or the index to the type its
/// expression coerces to, and the payload of [`Error::DependentOnType`].
///
/// Boxed into the variant for the same reason as [`ColumnDefaultDependency`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDependency {
    /// The enum the statement is trying to drop.
    pub type_name: String,
    /// The dependent as `getObjectDescription` spells it, identifiers unquoted —
    /// `constraint c on table t`, `default value for column c of table t`, `index i` —
    /// when it is the **only** one. 18.6's `DETAIL` lists every dependent, one line
    /// each, in an order its OIDs decide, so with more than one — a column declaring
    /// the type counts, though pglet does not refuse the drop over one (`EDGES.md` row
    /// 261) — the field is omitted rather than filled with a partial or reordered list.
    pub dependent: Option<String>,
}

impl std::fmt::Display for TypeDependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot drop type {} because other objects depend on it",
            self.type_name
        )
    }
}

/// An identity column standing in the way of dropping its own sequence — Postgres's
/// INTERNAL edge, and the payload of [`Error::SequenceRequiredByIdentity`].
///
/// Boxed into the variant for the same reason as [`ColumnDefaultDependency`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentitySequenceDependency {
    /// The sequence the statement is trying to drop.
    pub sequence: String,
    /// The table holding the identity column.
    pub table: String,
    /// The identity column the sequence is the internal implementation of.
    pub column: String,
}

impl std::fmt::Display for IdentitySequenceDependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            sequence,
            table,
            column,
        } = self;
        write!(
            f,
            "cannot drop sequence {sequence} because column {column} of table {table} requires it"
        )
    }
}

/// Render the bytes of an encoding failure the way 18.6's `report_invalid_encoding`
/// does: `0x%02x`, space-separated.
///
/// The caller has already taken the slice — the bytes from the failure offset,
/// `min(pg_utf_mblen(lead), remaining)` of them. 18.6 also caps that count at 8, which
/// no UTF-8 input reaches: `pg_utf_mblen` answers 1, 2, 3 or 4 and never more (its 5-
/// and 6-byte arms are `#ifdef NOT_USED`), so the cap is not reproduced.
fn format_encoding_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("0x{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Top-level error type for pglet.
#[derive(Debug, Error)]
pub enum Error {
    /// A `42601` parse or lexical failure.
    ///
    /// `message` is the **whole** sentence, not a body a `Display` prefix is glued
    /// onto — the prefix form produced `syntax error: syntax error at or near "X"`
    /// wherever a producer already wrote Postgres's own wording. Build it with
    /// [`Error::syntax`] (which adds pglet's `syntax error: ` prefix to a bare body)
    /// or [`Error::syntax_at`], or with [`Error::syntax_sentence`] /
    /// [`Error::syntax_sentence_at`], which take the whole sentence and add no prefix.
    ///
    /// `position` is 18.6's `P`: a **1-based Unicode codepoint** index into the
    /// statement text. It rides here rather than in the message because 18.6's `M`
    /// never carries a position — a client reading `.position` off its driver got
    /// nothing while pglet spelled a 0-based *byte* offset into the sentence.
    #[error("{message}")]
    Syntax {
        message: String,
        position: Option<u32>,
    },

    #[error(transparent)]
    Catalog(#[from] CatalogError),

    #[error("type error: {0}")]
    Type(String),

    #[error("cannot cast type {from} to {to}")]
    TypeCoercion { from: String, to: String },

    /// A construct whose inputs share a `typcategory` but cannot all convert to the
    /// resolved common type — Postgres's `%s could not convert type %s to %s`
    /// (`COALESCE`, `CASE`, `ARRAY`, `UNION`, `VALUES`, `GREATEST`, `LEAST`).
    /// SQLSTATE `42846` like [`Self::TypeCoercion`], but a distinct variant because the
    /// message carries the *construct* as well as the pair; folding the construct name
    /// into `TypeCoercion`'s `from` produced the malformed "cannot cast type COALESCE
    /// could not convert date to time without time zone".
    #[error("{context} could not convert type {from} to {to}")]
    CannotConvertType {
        context: String,
        from: String,
        to: String,
    },

    #[error("invalid input value for enum {enum_type}: \"{value}\"")]
    InvalidEnumValue { enum_type: String, value: String },

    /// Malformed JSON text handed to the `::jsonb` cast or the binary JSONB
    /// decoder (`value_from_binary`). Postgres raises SQLSTATE `22P02`
    /// (invalid_text_representation): `invalid input syntax for type json`.
    /// Replaces the old `TypeCoercion` (`42846` cannot_coerce), which mis-coded
    /// a malformed-input error as a cast-incompatibility one.
    #[error("invalid input syntax for type json")]
    InvalidJson,

    /// A binary-format `jsonb` parameter whose leading **version byte** is not `1`.
    /// 18.6's `jsonb_recv` reads that byte unconditionally — `pq_getmsgint(buf, 1)`,
    /// which yields an unsigned `0`–`255` — and takes the JSON text to be everything
    /// after it, so a payload sent with no version byte is a payload whose first byte
    /// *is* the version.
    ///
    /// SQLSTATE `XX000` (`internal_error`) for a condition any client can reach by
    /// sending one byte: 18.6's raise site sets no errcode, and `errstart` defaults
    /// anything at `ERROR` or above to `ERRCODE_INTERNAL_ERROR`. `elog` is not what
    /// decides that — it expands to `ereport(elevel, errmsg_internal(...))`, so an
    /// `ereport(ERROR, …)` carrying no `errcode()` of its own lands on the same default.
    /// pglet reproduces the code 18.6 sends rather than choosing a tidier one — this is
    /// the engine's only `XX000` producer. 18.6 also
    /// sends a `W` (`unnamed portal parameter $1`) with it; pglet sends no `W` field
    /// anywhere, which is [`Self::InvalidJson`]'s standing omission too (`docs/errors.md`).
    #[error("unsupported jsonb version number {version}")]
    UnsupportedJsonbVersion { version: u8 },

    /// A binary value's reader asked for more bytes than its buffer holds, through one
    /// of 18.6's multi-byte message readers — `pq_getmsgbytes`, or `pq_copymsgbytes`,
    /// which `pq_getmsgint` and `pq_getmsgint64` read through. SQLSTATE `08P01`
    /// (`protocol_violation`). The width asked for does not pick this over
    /// [`Self::NoDataLeft`]: `jsonb_recv`'s one-byte `pq_getmsgint` says this one.
    #[error("insufficient data left in message")]
    InsufficientDataLeft,

    /// A binary value's reader asked for a byte its buffer does not hold through
    /// `pq_getmsgbyte`, the single-byte reader `boolrecv` calls. SQLSTATE `08P01`.
    #[error("no data left in message")]
    NoDataLeft,

    /// A binary-format Bind parameter whose type's reader returned without consuming
    /// the whole payload — `exec_bind_message`'s check after the `*recv` call.
    /// `param` is the parameter's 1-based number. SQLSTATE `22P03`
    /// (`invalid_binary_representation`).
    #[error("incorrect binary data format in bind parameter {param}")]
    IncorrectBinaryParamFormat { param: usize },

    /// A binary array element's length field below `-1` or past the bytes left in the
    /// array's buffer — `ReadArrayBinary`'s own check, which carries the message
    /// reader's sentence under SQLSTATE `22P03` rather than `08P01`. Neither the words
    /// nor the code alone identify it: [`Self::InsufficientDataLeft`] shares the words,
    /// and [`Self::ImproperArrayElementFormat`] the code.
    #[error("insufficient data left in message")]
    ArrayElementLengthInvalid,

    /// A binary array element whose type's reader returned without consuming the
    /// element's own length — `ReadArrayBinary`'s check after the element's `*recv`.
    /// `element` is 1-based and counts NULL elements. SQLSTATE `22P03`.
    #[error("improper binary format in array element {element}")]
    ImproperArrayElementFormat { element: usize },

    /// A binary array header whose element OID names an array type `TypeOid::from_oid`
    /// recognises, where the parameter's element type is a scalar — one member of 18.6's
    /// `array_recv` mismatch check, which refuses wherever both OIDs sit below its
    /// built-in floor (`FirstGenbkiObjectId`, `10000`). A header OID pglet does not
    /// recognise is not refused here, below that floor or above it. The names are
    /// `format_type`'s. SQLSTATE `42804` (`datatype_mismatch`).
    #[error(
        "binary data has array element type {found_oid} ({found}) instead of expected {expected_oid} ({expected})"
    )]
    BinaryArrayElementTypeMismatch {
        found_oid: u32,
        found: &'static str,
        expected_oid: u32,
        expected: &'static str,
    },

    /// A binary `numeric` whose sign word is none of `numeric_recv`'s accepted values.
    /// SQLSTATE `22P03`.
    #[error("invalid sign in external \"numeric\" value")]
    InvalidNumericSign,

    /// A binary `numeric` whose display scale sets a bit outside
    /// `NUMERIC_DSCALE_MASK` (`0x3FFF`). SQLSTATE `22P03`.
    #[error("invalid scale in external \"numeric\" value")]
    InvalidNumericScale,

    /// A binary `numeric` digit outside `0 ..= 9999`, read as a signed 16-bit value.
    /// SQLSTATE `22P03`.
    #[error("invalid digit in external \"numeric\" value")]
    InvalidNumericDigit,

    /// A text value whose *content* can't be parsed as the target scalar type —
    /// `'foo'::int`, `'x'::uuid`, a malformed timestamp/bytea literal. Postgres
    /// raises SQLSTATE `22P02` (invalid_text_representation): `invalid input syntax
    /// for type integer: "foo"`. Distinct from `TypeCoercion` (`42846`), which is a
    /// genuinely incompatible cast (`int → uuid`) — here the cast itself is valid
    /// and only this value fails to parse. `type_name` is the SQL target type.
    #[error("invalid input syntax for type {type_name}: \"{value}\"")]
    InvalidInputSyntax {
        type_name: &'static str,
        value: String,
    },

    /// A `date`/`time` literal whose text is **unparseable** — the wrong shape
    /// entirely (`date 'garbage'`, `time 'garbage'`). Postgres raises SQLSTATE
    /// `22007` (`invalid_datetime_format`): `invalid input syntax for type date:
    /// "garbage"`. Distinct from `InvalidInputSyntax` (`22P02`, used by the other
    /// scalar parsers) and from `TemporalOutOfRange` (`22008`, a well-formed but
    /// out-of-range field like `date '2026-13-01'`). `type_name` is the SQL type.
    #[error("invalid input syntax for type {type_name}: \"{value}\"")]
    InvalidDatetimeFormat {
        type_name: &'static str,
        value: String,
    },

    /// A malformed `COPY ... FROM` text stream: a data row with too few or too many
    /// columns, or a line that starts with the end-of-data marker (`\.`) and goes on.
    /// SQLSTATE `22P04` (`bad_copy_file_format`), under the sentences
    /// `copyfromparse.c` spells: `missing data for column "x"` /
    /// `extra data after last expected column` /
    /// `end-of-copy marker is not alone on its line`. Distinct from
    /// `InvalidInputSyntax` (`22P02`), which is a single field's value failing its column
    /// type's input function — the framing is well-formed but the value is not — and
    /// from `InvalidEncoding` (`22021`), which is COPY input or a de-escaped field the
    /// server encoding refuses.
    ///
    /// Sharing 18.6's sentences does not make the two engines refuse the same lines.
    /// pglet takes `\.` as the marker only at the start of a line, where 18.6's line
    /// reader also refuses one after other data on its line; and 18.6 refuses a `\.`
    /// LF line after an `ok` CRLF line, which pglet loads (`EDGES.md`).
    #[error("{0}")]
    CopyFormat(String),

    /// A `NOT NULL` column left null by a write. 18.6 sends `s`/`t`/`c` and a
    /// `Failing row contains (…)` DETAIL built from the **whole** tuple
    /// (`ExecBuildSlotValueDescription`), which is why `row` is `Option`: a path that
    /// raises this without a fully assembled row has no truthful detail to send and
    /// omits it rather than showing a half-built one.
    #[error(
        "null value in column \"{column}\" of relation \"{table}\" violates not-null constraint"
    )]
    NotNullViolation {
        table: String,
        column: String,
        /// The refused row, for the `D` field — boxed to keep [`enum@Error`] inside
        /// [`MAX_ERROR_SIZE`].
        row: Option<Box<crate::error_fields::FailingRow>>,
    },

    /// A row-scoped `CHECK` refused by a write (`ExecConstraints`). 18.6 sends
    /// `s`/`t`/`n` and the same `Failing row contains (…)` DETAIL as a not-null
    /// violation.
    ///
    /// Distinct from [`Self::CheckConstraintNotSatisfied`], which is the *other* route
    /// to `23514` and has both a different message and no detail at all.
    #[error("new row for relation \"{table}\" violates check constraint \"{constraint}\"")]
    CheckViolation {
        table: String,
        constraint: String,
        /// The refused row, for the `D` field.
        row: Option<Box<crate::error_fields::FailingRow>>,
    },

    /// An `ALTER TABLE` that would make an existing column `NOT NULL` — `SET NOT
    /// NULL`, or `ADD COLUMN … NOT NULL` whose backfill is null — while the table
    /// already holds a null there.
    ///
    /// 18.6's `ATRewriteTable` route, and a **different error** from
    /// [`Self::NotNullViolation`] despite sharing `23502`: `column "x" of relation "t"
    /// contains null values`, with `s`/`t`/`c` and no `D` (measured — there is no
    /// "new row" to show). pglet answered it with the write-path wording, which named
    /// a row the statement never wrote.
    #[error("column \"{column}\" of relation \"{table}\" contains null values")]
    NotNullColumnHasNulls { table: String, column: String },

    /// An `ALTER TABLE … ADD { PRIMARY KEY | UNIQUE }` / `ADD COLUMN … UNIQUE` whose
    /// index build found duplicate keys among the existing rows.
    ///
    /// 18.6 reports the **index build**, not a row write: `could not create unique
    /// index "x"` with `Key (…)=(…) is duplicated.` (note *duplicated*, not *already
    /// exists*) and `s`/`t`/`n` — `23505`, same as [`Self::UniqueViolation`], different
    /// error. Measured on all three spellings.
    #[error("could not create unique index \"{index}\"")]
    DuplicateIndexKey {
        index: String,
        /// The relation and duplicated key — boxed to keep [`enum@Error`] inside
        /// [`MAX_ERROR_SIZE`]; `None` when the key cannot be described truthfully.
        detail: Option<Box<crate::error_fields::UniqueViolationDetail>>,
    },

    /// An `ALTER TABLE … ADD CONSTRAINT CHECK` whose validation scan found an
    /// existing row the predicate refuses — 18.6's `ATRewriteTable` route.
    ///
    /// Same SQLSTATE as [`Self::CheckViolation`] and a different message, because it
    /// is a different error: no single row is "the new row", so 18.6 sends
    /// `[SVCMstnFLR]` — `check constraint "x" of relation "y" is violated by some row`
    /// and **no** `D` (measured). Answering it with the row-check wording named a row
    /// the statement never wrote.
    #[error("check constraint \"{constraint}\" of relation \"{table}\" is violated by some row")]
    CheckConstraintNotSatisfied { table: String, constraint: String },

    /// A collision with a unique index — ordinarily a write against an enforced one.
    /// 18.6 sends `s`/`t`/`n` and a `Key (…)=(…) already exists.` DETAIL; the
    /// constraint's **name** — all this variant used to carry — determines none of the
    /// other three, so `key` carries the relation and the colliding tuple.
    ///
    /// `CREATE TYPE`'s repeated enum label raises it with `key: None`: it stands in for
    /// a collision on 18.6's own `pg_enum_typid_label_index`, an index this engine does
    /// not hold, and sends the constraint name alone.
    #[error("duplicate key value violates unique constraint \"{constraint}\"")]
    UniqueViolation {
        constraint: String,
        /// The relation and colliding key, for `t` and the `D` field — boxed to keep
        /// [`enum@Error`] inside [`MAX_ERROR_SIZE`].
        key: Option<Box<crate::error_fields::UniqueViolationDetail>>,
    },

    /// An `INSERT … ON CONFLICT DO UPDATE` whose conflict probe finds a row the **same
    /// statement** already inserted or updated. SQLSTATE `21000` (`cardinality_violation`).
    ///
    /// 18.6 raises it in `ExecOnConflictUpdate` (the PostgreSQL source's
    /// `ExecOnConflictLockRow`), where locking the conflicting tuple answers
    /// `TM_Invisible` because the current command wrote it — so the test is the
    /// row's identity, not its key, and it runs before the action's `WHERE`. 18.6 sends
    /// the message and a hint and nothing else: no `s`/`t`/`n` (measured).
    #[error("ON CONFLICT DO UPDATE command cannot affect row a second time")]
    OnConflictRowAffectedTwice,

    /// A child INSERT/UPDATE references a missing parent, or a parent DELETE/UPDATE
    /// under **NO ACTION** would orphan a child. SQLSTATE `23503`
    /// (`foreign_key_violation`).
    ///
    /// The payload is destructured rather than a pre-formatted sentence because 18.6
    /// sends four facts the sentence cannot be split back into — and one of them
    /// inverts: `t` is the **child** table in both directions while the delete-side
    /// message names the parent. See
    /// [`ForeignKeyViolationDetail`](crate::error_fields::ForeignKeyViolationDetail).
    #[error("{}", .0.no_action_message())]
    ForeignKeyViolation(Box<crate::error_fields::ForeignKeyViolationDetail>),

    /// A parent DELETE/UPDATE under an explicit **RESTRICT** action is rejected by a
    /// referencing child. Postgres separates this from NO ACTION: it raises SQLSTATE
    /// `23001` (`restrict_violation`) with a `violates RESTRICT setting of foreign key
    /// constraint` message, whereas NO ACTION raises `23503`. Its DETAIL is the
    /// delete-side sentence **without** the word "still" (measured).
    ///
    /// Shares [`ForeignKeyViolationDetail`](crate::error_fields::ForeignKeyViolationDetail)
    /// with [`Self::ForeignKeyViolation`]; RESTRICT only ever fires on the delete
    /// side, so the payload's `direction` is always
    /// [`FkDirection::Delete`](crate::error_fields::FkDirection::Delete) here.
    #[error("{}", .0.restrict_message())]
    RestrictViolation(Box<crate::error_fields::ForeignKeyViolationDetail>),

    /// A `CREATE TYPE` of a type that already exists. `CREATE TYPE` has no
    /// `IF NOT EXISTS`, so Postgres raises SQLSTATE `42710` (`duplicate_object`) —
    /// the error the idempotent `DO $$ … EXCEPTION WHEN duplicate_object THEN NULL
    /// … $$` migration idiom catches. (Previously a generic `Execution`/`22000`,
    /// which the EXCEPTION matcher couldn't recognize.)
    #[error("type \"{0}\" already exists")]
    DuplicateType(String),

    /// A **relation** created over a name the type namespace already holds — a table or
    /// a sequence named like an enum. 18.6 raises it from `heap_create_with_catalog`,
    /// which tests `pg_type` for every relation it creates (a table's row type would
    /// collide, and a sequence, which gets no row type, is refused all the same), after
    /// the relation name's own `42P07`. The message is [`Self::DuplicateType`]'s; what
    /// separates the two is the hint, which 18.6 sends on this route only (measured;
    /// `DefineEnum`'s `42710` carries none).
    #[error("type \"{0}\" already exists")]
    RelationTypeAlreadyExists(String),

    /// `CREATE EXTENSION <name>` for an extension **pglet** does not provide.
    ///
    /// The SQLSTATE is the one 18.6 gives an extension it cannot find a control file
    /// for (`0A000`, `feature_not_supported`, and the code it gives even under
    /// `IF NOT EXISTS` — measured), but the *message* deliberately does not copy that
    /// server's wording: the set here is what this engine provides, and for an
    /// extension the reference server ships and pglet lacks (`citext`, `hstore`,
    /// `btree_gin`, …) "is not available" would be a false claim about the system.
    #[error("extension \"{0}\" is not provided by pglet")]
    ExtensionNotAvailable(String),

    /// `DROP EXTENSION <name>` (without `IF EXISTS`) for an extension pglet does not
    /// know. Postgres raises SQLSTATE `42704` (`undefined_object`).
    #[error("extension \"{0}\" does not exist")]
    UndefinedExtension(String),

    #[error("execution error: {0}")]
    Execution(String),

    /// A page write or `fsync` the device refused before the statement published
    /// anything, which leaves the database serving: the statement's rows are dropped with
    /// the batch that met it (a sequence advance stands, as a rolled-back one does). The
    /// SQLSTATE follows 18.6's `errcode_for_file_access` for `ENOSPC` (`53100`,
    /// `disk_full`) and `EIO` (`58030`, `io_error`) and is `58030` for any other OS error,
    /// where that function answers per errno (`42501` for `EROFS`, `XX000` by default —
    /// `EDGES.md` row 208); 18.6 raises it at `ERROR` for a data file it cannot extend
    /// (`mdextend`).
    #[error("could not write to the database file: {reason}")]
    WriteRefused {
        /// The OS error's text.
        reason: String,
        /// Whether it was `ENOSPC`.
        disk_full: bool,
    },

    /// A write or `fsync` the device refused where the statement's effect cannot be
    /// dropped — something it depends on was already visible without being durable (a
    /// published batch, a catalog change) — and before any meta slot naming the
    /// statement's writes was written, so none of it is durable. The statement that met
    /// it gets this, and the store then refuses every later one
    /// ([`Self::DatabaseMustBeReopened`]) until a reopen, which keeps what the slot on
    /// disk holds. The SQLSTATE is chosen as for [`Self::WriteRefused`] (`53100` for
    /// `ENOSPC`, else `58030`); 18.6 raises `errcode_for_file_access`'s at `PANIC` for a
    /// WAL write or flush it cannot complete, ending every connection.
    #[error(
        "could not make the statement durable ({reason}); the database refuses statements until it is reopened"
    )]
    DurableWriteFailed {
        /// The OS error's text.
        reason: String,
        /// Whether it was `ENOSPC`.
        disk_full: bool,
    },

    /// A write or `fsync` the device refused where the statement may be durable in part or
    /// whole: at or after the meta-slot write that commits it, which may have landed and
    /// which a reopen adopts if it did — or where it fails the store in a block's
    /// `COMMIT`, of a healthy block or of an aborted one (which 18.6 converts to a
    /// rollback), when the block wrote the catalog and a save has written a catalog since
    /// it took the writer that differs from the one it was re-based on, a counter advanced in
    /// place aside: every save but a rollback's counter save encodes the live catalog, and
    /// that one writes nothing while the live catalog differs from the durable one in
    /// anything but counters (`pglet`'s `TransactionSession::counter_save`), so a save after
    /// a catalog change of the block's made that change durable (`pglet`'s
    /// `TransactionSession::commit`). The answer says the outcome is unknown rather than
    /// that the statement failed: a client told it failed may retry a statement that
    /// persisted. The store fails as for
    /// [`Self::DurableWriteFailed`], with the same SQLSTATE, and 18.6 raises the same
    /// condition at `PANIC`, ending the connection.
    #[error(
        "whether the statement persists is unknown until the database is reopened ({reason}): a write it needed failed, and part or all of it may be durable; the database refuses statements until it is reopened"
    )]
    DurableWriteUnconfirmed {
        /// The OS error's text.
        reason: String,
        /// Whether it was `ENOSPC`.
        disk_full: bool,
    },

    /// A statement issued after a durable write failed ([`Self::DurableWriteFailed`] or
    /// [`Self::DurableWriteUnconfirmed`]),
    /// on any connection: the in-memory state may hold a commit the durable state does
    /// not, so nothing is read or written until the database is reopened. SQLSTATE
    /// `57P02` (`crash_shutdown`), what 18.6 tells every other connection when a
    /// `PANIC` restarts the server — it ends them; an embedding host has no connection
    /// to end, so each statement is refused instead.
    #[error("the database refuses statements since a durable write failed ({0}); reopen it")]
    DatabaseMustBeReopened(String),

    /// pglet's own refusal of a B-tree index over a key column of this type — today a
    /// non-unique one over a single bare `jsonb` or array column, at the DDL. `0A000`.
    #[error("data type {0} has no default operator class for a B-tree index")]
    UnindexableType(String),

    /// A declared/cast type the engine does not implement (NUMERIC, TIME, an
    /// INTERVAL column, or any unrecognized type). Resolution fails loud by default —
    /// the engine never silently coerces an unsupported type to TEXT. Opt back
    /// into the old coercion with `coerce_unknown_types` (see
    /// `planner::set_coerce_unknown_types`). SQLSTATE is `42704`
    /// (undefined_object), matching Postgres's `type "x" does not exist`.
    #[error("type \"{type_name}\" is not supported")]
    UnsupportedType { type_name: String },

    /// A `FLOAT(n)` whose `n` is below 18.6's window — `FLOAT(0)`, and nothing else,
    /// the grammar admitting no negative or fractional `n`.
    ///
    /// **This is the one modifier `parser::type_name` reads that *selects* a type
    /// rather than narrowing one**, so its range check belongs to the grammar and not
    /// to a type's `typmodin`: 18.6 raises it while reducing `gram.y`'s `opt_float`, before any
    /// name resolution, which is why the same refusal reaches positions that never
    /// resolve a type name (`CREATE SEQUENCE s AS FLOAT(0)`). SQLSTATE `22023`
    /// (`invalid_parameter_value`).
    ///
    /// `position` is 18.6's `P`: the **1-based codepoint** index of the number
    /// itself, which is where `parser_errposition(@2)` points. It is not optional —
    /// the only producer is a parser arm holding the number's token.
    #[error("precision for type float must be at least 1 bit")]
    FloatPrecisionTooSmall { position: u32 },

    /// A `FLOAT(n)` whose `n` is above 18.6's window (`FLOAT(54)` and up, to the
    /// `int32` ceiling the grammar's `Iconst` sets). The sibling of
    /// [`Self::FloatPrecisionTooSmall`]; that variant carries the shared rule.
    #[error("precision for type float must be less than 54 bits")]
    FloatPrecisionTooLarge { position: u32 },

    /// A `varchar(n)` / `char(n)` length below 18.6's window. `n < 1`, which
    /// `CharacterWithLength`'s `Iconst` narrows to `0` alone — a leading `-` is a
    /// different token and `parser::type_name`'s reader refuses it as `42601` first.
    ///
    /// **A modifier the type refuses is a refusal, never a modifier dropped.** 18.6
    /// raises it from the type's own modifier reader — `anychar_typmodin`
    /// (`varchar.c`), reached through `typenameTypeMod` *after* the name resolves — so
    /// pglet raises it there too, in `executor::type_resolution`'s `resolve_type_name`,
    /// which is the one resolution every position that resolves a declared type runs.
    /// SQLSTATE `22023` (`invalid_parameter_value`). `docs/coercion.md` § A canonical
    /// type name decomposes once.
    ///
    /// `type_name` is the **internal** name `anychar_typmodin` is handed rather than
    /// the spelling: `character(0)` names `char` and `character varying(0)` names
    /// `varchar`, and an array names its element (`char(0)[]` is `char`) — each
    /// measured on 18.6.
    ///
    /// **No `P` is sent**, where 18.6 sends one. The omission belongs to the
    /// **resolver** and not to any one caller: `resolve_type_name` takes a type name and
    /// no span, so none of its four callers can hand one down — and two of them are the
    /// *parser* (`parser::expr`'s cast target and typed constant), which holds a cursor
    /// and runs the reader that fills [`Self::FloatPrecisionTooSmall`]'s `P`
    /// (`parser::type_name`'s `float_type_name`). Threading one
    /// from those two is possible and unasked; [`Self::position`] fills `P` only where a
    /// variant carries one, and the differential harness scores an omitted field as
    /// agreement (`docs/errors.md`).
    #[error("length for type {type_name} must be at least 1")]
    CharacterLengthTooSmall { type_name: &'static str },

    /// A `varchar(n)` / `char(n)` length above Postgres's `MaxAttrSize`
    /// ([`MAX_CHARACTER_LENGTH`](crate::types::MAX_CHARACTER_LENGTH)). The sibling of
    /// [`Self::CharacterLengthTooSmall`], which carries the shared rule.
    #[error(
        "length for type {type_name} cannot exceed {}",
        crate::types::MAX_CHARACTER_LENGTH
    )]
    CharacterLengthTooLarge { type_name: &'static str },

    /// A `numeric(p[,s])` / `decimal(p[,s])` precision outside `1 ..= 1000`, including
    /// every negative one — the scale grammar admits a `-`, so this reaches values a
    /// character length cannot.
    ///
    /// 18.6's `numerictypmodin` (`numeric.c`), reached through `typenameTypeMod`, whose
    /// two arity branches share one range check, `make_numeric_typmod_safe` — the
    /// function this refusal's own `LOCATION` names on the oracle, the same line for
    /// `numeric(0)` and `numeric(0,1001)` (`numeric.c:1333`, measured; the scale's is
    /// `:1338`). pglet raises it from the same place
    /// [`Self::CharacterLengthTooSmall`] does, whose doc carries the shared rule and the
    /// omitted `P`. SQLSTATE `22023` (`invalid_parameter_value`).
    ///
    /// The precision is checked **before** the scale inside the **two-component**
    /// branch — `numeric(0,1001)` names the precision on both engines, measured. The
    /// one-component branch has no scale to reach at all.
    #[error(
        "NUMERIC precision {precision} must be between 1 and {}",
        crate::types::MAX_NUMERIC_PRECISION
    )]
    NumericPrecisionOutOfRange { precision: i32 },

    /// A `numeric(p,s)` scale outside `-1000 ..= 1000` — Postgres's `NUMERIC_MIN_SCALE` /
    /// `NUMERIC_MAX_SCALE` ([`MAX_NUMERIC_SCALE`](crate::types::MAX_NUMERIC_SCALE)
    /// negated and plain). The sibling of [`Self::NumericPrecisionOutOfRange`].
    ///
    /// A scale **inside** the window can still refuse the value rather than the
    /// spelling: `1.5::numeric(5,1000)` is `22003 numeric field overflow` on 18.6, a
    /// value error past this gate and not one of its members.
    #[error(
        "NUMERIC scale {scale} must be between {} and {}",
        -crate::types::MAX_NUMERIC_SCALE,
        crate::types::MAX_NUMERIC_SCALE
    )]
    NumericScaleOutOfRange { scale: i32 },

    /// A `numeric(…)` modifier with neither one nor two components —
    /// `numeric(5,2,3)`, `numeric(5,2,3,4)`. 18.6's `numerictypmodin` counts after
    /// coercing every component to `int4`, so a malformed component outranks the
    /// count (`numeric(5,2,1.5)` is `22P02` there, measured). SQLSTATE `22023`
    /// (`invalid_parameter_value`).
    ///
    /// `anychar_typmodin` has the same arm and pglet cannot reach it: a character
    /// length's grammar is one `Iconst`, so `char(5,2)` is `42601` on both engines
    /// before any type reads the modifier.
    #[error("invalid NUMERIC type modifier")]
    InvalidNumericTypeModifier,

    /// A `SHOW`/config reference to a runtime parameter pglet doesn't model.
    /// Postgres raises SQLSTATE `42704` (undefined_object): `unrecognized
    /// configuration parameter "x"`. Replaces the old `.contains()` substring
    /// dispatch that silently returned an empty result for an unknown name.
    #[error("unrecognized configuration parameter \"{0}\"")]
    UnrecognizedParameter(String),

    /// Division (or modulo) by zero. Postgres raises SQLSTATE `22012`
    /// (`division_by_zero`); pglet matches so the differential harness agrees,
    /// rather than the old `f64::NAN as i64 == 0` that silently returned 0.
    #[error("division by zero")]
    DivisionByZero,

    /// An integer arithmetic result overflowed its type's range. Postgres raises
    /// `22003` (`numeric_value_out_of_range`, e.g. "integer out of range");
    /// pglet matches rather than wrapping or saturating through `f64`. The field
    /// is the SQL type name (`smallint` / `integer` / `bigint`) for the message.
    #[error("{0} out of range")]
    NumericOutOfRange(&'static str),

    /// A `real` / `double precision` read from text whose decimal value overflows the
    /// type, or whose nonzero value underflows it to zero. Postgres raises SQLSTATE
    /// `22003` (`numeric_value_out_of_range`) from `float4in` / `float8in`:
    /// `"1e39" is out of range for type real`. `value` is the number as the reader
    /// consumed it — no surrounding whitespace and no trailing text, which is what 18.6
    /// prints — and a `numeric` cast to a float type is read from its `numeric_out`
    /// text, so the message there names every digit.
    #[error("\"{value}\" is out of range for type {type_name}")]
    FloatInputOutOfRange {
        value: String,
        type_name: &'static str,
    },

    /// A finite `numeric` whose base-10 weight is outside what the storage format's
    /// `int16` weight field can hold — more than 131072 integer digits, or a scale
    /// past the mirrored lower bound. Postgres raises this from `make_result`
    /// (`22003`, "value overflows numeric format"), which `numeric_in` runs over a
    /// literal and every arithmetic result runs over its product; it is a **format**
    /// bound rather than a type's range, which is why it does not name a type the way
    /// `NumericOutOfRange` does.
    #[error("value overflows numeric format")]
    NumericFormatOverflow,

    /// A text value longer than a `varchar(n)` / `char(n)` **column**'s declared
    /// length whose excess is not all blanks. Postgres raises SQLSTATE `22001`
    /// (`string_data_right_truncation`): `value too long for type character
    /// varying(n)`. Only the column-store path fails loud — an explicit
    /// `x::varchar(n)` cast truncates silently instead (see
    /// `executor::apply_typmod`). `max_length` is the declared character length.
    #[error("value too long for type character varying({max_length})")]
    StringDataRightTruncation { max_length: u32 },

    /// Text that is not valid in the server encoding, which Postgres reports with
    /// SQLSTATE `22021` (`character_not_in_repertoire`): a finished `E'…'` literal, COPY
    /// input or a de-escaped COPY field, a bound parameter (on the native server in
    /// either wire format, or bound from JS on the wasm host), or the dump's COPY
    /// payload. Each is scanned left to right (`pg_verify_mbstr`, through
    /// `crate::encoding`), so the failure sits at the first NUL or the first byte that
    /// starts no legal UTF-8 sequence.
    ///
    /// `bytes` is what 18.6's `report_invalid_encoding` prints: the bytes from that
    /// failure offset, `min(pg_utf_mblen(lead), remaining)` of them. This variant's
    /// `#[error]` attribute is the **one home** for the sentence — the lexer asks it for
    /// the message rather than writing a second copy.
    ///
    /// A NUL written as a *Unicode* escape in a string literal (`\u0000`, `U&'\0000'`) is
    /// caught earlier as an invalid Unicode code point and is a `42601` syntax error
    /// instead, matching Postgres; in a jsonb string it is `UnsupportedUnicodeEscape`.
    #[error(
        "invalid byte sequence for encoding \"UTF8\": {}",
        format_encoding_bytes(bytes)
    )]
    InvalidEncoding { bytes: Vec<u8> },

    /// A `\u` / `\U` escape inside an `E'…'` literal with too few hex digits — 18.6's
    /// `{xeunicodefail}` scanner rule, which raises `ERRCODE_INVALID_ESCAPE_SEQUENCE`
    /// (`22025`) rather than a syntax error.
    ///
    /// It belongs to the `E'…'` decoder alone. The **same** malformation inside a
    /// `U&'…'` goes through the grammar's own unescaper, which raises a plain `42601`
    /// carrying this same sentence: 18.6 disagrees with itself across its two decoders,
    /// and pglet reproduces both sides rather than picking one.
    #[error("invalid Unicode escape")]
    InvalidUnicodeEscape,

    /// A jsonb string's `\u0000` escape — 18.6's `JSON_UNICODE_CODE_POINT_ZERO`, which
    /// `json_errsave_error` raises as SQLSTATE `22P05` (`untranslatable_character`)
    /// because a `text` value cannot hold a NUL. The string-literal decoders refuse the
    /// same escape as an invalid escape value (`42601`) instead.
    #[error("unsupported Unicode escape sequence")]
    UnsupportedUnicodeEscape,

    /// A relation-name **string literal** — a `nextval`/`currval`/`setval` argument,
    /// the shape Postgres reads through `stringToQualifiedNameList` — that is not a
    /// well-formed (optionally qualified) identifier: an empty name, interior
    /// whitespace outside double quotes (`'s x'`), or an unterminated `"`.
    /// Postgres raises SQLSTATE `42602` (`invalid_name`) with exactly this message
    /// and no detail. Distinct from `Syntax` (`42601`), which covers a malformed
    /// *statement* rather than a malformed name inside a well-formed one.
    #[error("invalid name syntax")]
    InvalidName,

    /// A malformed `INTERVAL` literal (`interval_in`). Postgres raises SQLSTATE
    /// `22007` (`invalid_datetime_format`) for interval input — distinct from the
    /// `22P02` the other scalar parsers use. The field is the value that failed to
    /// parse.
    #[error("invalid input syntax for type interval: \"{0}\"")]
    InvalidIntervalSyntax(String),

    /// An `INTERVAL` literal whose field magnitude overflows the representable range
    /// at parse/decode time (`interval '999999999999 years'`). Postgres raises
    /// SQLSTATE `22015` (`interval_field_overflow`): `interval field value out of
    /// range: "…"`. Distinct from `InvalidIntervalSyntax` (`22007`, a malformed
    /// spelling) and from `TemporalOutOfRange` (`22008`, which covers date/time and
    /// interval *arithmetic* overflow). The field is the value that overflowed.
    #[error("interval field value out of range: \"{0}\"")]
    IntervalFieldOverflow(String),

    /// A temporal value outside its range — an arithmetic result, a decoded binary
    /// value, or a `date` / `timestamp` outside the date domain
    /// (`types::check_date_domain`). Postgres raises SQLSTATE `22008`
    /// (`datetime_field_overflow`): `date out of range` / `timestamp out of range` /
    /// `interval out of range`. Distinct from `NumericOutOfRange` (`22003`,
    /// integer/numeric overflow). The field is the SQL type name for the message.
    #[error("{0} out of range")]
    TemporalOutOfRange(&'static str),

    /// A `date` / `timestamp` / `timestamptz` text literal whose **year field** is `0` —
    /// 18.6's `ValidateDate` refusal, raised through `DateTimeParseError` as SQLSTATE
    /// `22008` (`datetime_field_overflow`): `date/time field value out of range: "…"`,
    /// quoting the input as written. Distinct from `TemporalOutOfRange`, which carries
    /// the bare `date out of range` / `timestamp out of range` a *value* leaving its
    /// range answers. The field is the literal.
    #[error("date/time field value out of range: \"{0}\"")]
    DatetimeFieldValueOutOfRange(String),

    /// A `timestamp` / `timestamptz` text literal whose numeric UTC offset 18.6's
    /// `DecodeTimezone` refuses — an hour outside `0 ..= 15`, a minute or second outside
    /// `0 ..= 59`, or a field `strtoint` overflows — raised as SQLSTATE `22009`
    /// (`invalid_time_zone_displacement_value`), quoting the input as written. The field
    /// is the literal.
    #[error("time zone displacement out of range: \"{0}\"")]
    TimeZoneDisplacementOutOfRange(String),

    /// `date - date` with an infinite operand — `date_mi`'s `22008`.
    #[error("cannot subtract infinite dates")]
    CannotSubtractInfiniteDates,

    /// A `substring(str FROM start FOR length)` (or `substring(str, start,
    /// length)`) with a negative `length`. Postgres raises SQLSTATE `22011`
    /// (`substring_error`): `negative substring length not allowed`.
    #[error("negative substring length not allowed")]
    NegativeSubstringLength,

    /// A `split_part(str, delim, 0)` — a zero field position. `split_part` is
    /// 1-based (a negative position counts from the end), so zero is invalid;
    /// Postgres raises SQLSTATE `22023` (`invalid_parameter_value`): `field
    /// position must not be zero`.
    #[error("field position must not be zero")]
    FieldPositionZero,

    /// A malformed `\x…` hex `bytea` literal — a non-hex digit (`'\xZZ'::bytea`) or
    /// an odd number of digits (`'\xDEA'::bytea`). Postgres's `byteain` hex path
    /// raises SQLSTATE `22023` (`invalid_parameter_value`): `invalid hexadecimal
    /// digit: "Z"` / `invalid hexadecimal data: odd number of digits`. Distinct
    /// from `InvalidInputSyntax` (`22P02`, the other scalar text parsers) — the hex
    /// decoder uses `ERRCODE_INVALID_PARAMETER_VALUE`, not
    /// `invalid_text_representation`. The field is the full Postgres message (it
    /// differs per case), so it renders byte-for-byte.
    #[error("{0}")]
    InvalidHexData(String),

    /// A request exceeding an implementation limit — a `lpad`/`rpad` target length
    /// so large the padded string can't be built. Postgres raises SQLSTATE `54000`
    /// (`program_limit_exceeded`): `requested length too large`. pglet caps the
    /// length rather than attempting a multi-gigabyte allocation.
    #[error("{0}")]
    ProgramLimitExceeded(String),

    /// An index entry over 18.6's B-tree item limit — `_bt_check_third_page`'s
    /// `54000`: a third of an 8 KB page, `BTMaxItemSize` = 2 704 bytes, measured
    /// after `index_form_tuple` has laid the key out (`executor::index_row`).
    ///
    /// 18.6 sends `s`/`t`/`n` (`errtableconstraint(heap, index)`) and its hint; its
    /// `D` names the heap tuple the entry references (`Index row references tuple
    /// (0,2) in relation "t".`), a `ctid` pglet does not have, so the `D` is omitted
    /// rather than invented. Boxed to keep [`enum@Error`] inside [`MAX_ERROR_SIZE`].
    #[error(
        "index row size {} exceeds btree version 4 maximum 2704 for index \"{}\"",
        .0.size,
        .0.index
    )]
    BtreeIndexRowTooLarge(Box<crate::error_fields::IndexRowSize>),

    /// A GIN entry over 18.6's GIN item limit — `GinFormTuple`'s `54000`,
    /// `GinMaxItemSize` = 2 712 bytes. 18.6 sends no detail, hint, or `s`/`t`/`n`
    /// (measured), and neither does this.
    #[error(
        "index row size {} exceeds maximum 2712 for index \"{}\"",
        .0.size,
        .0.index
    )]
    GinIndexRowTooLarge(Box<crate::error_fields::IndexRowSize>),

    /// An index entry past the size field of 18.6's index tuple header —
    /// `index_form_tuple`'s `54000`, raised for any access method before the method's
    /// own item limit is asked, with no fields beside the message (measured).
    #[error("index row requires {size} bytes, maximum size is 8191")]
    IndexRowRequiresTooMuch { size: usize },

    /// An index entry whose key, in pglet's own order-preserving encoding, is longer
    /// than an internal B+tree page can carry as a separator — pglet's limit, not
    /// 18.6's, so there is no 18.6 message to reproduce. `program_limit_exceeded`
    /// because that is what it is: an implementation limit a client can reach, the
    /// class 18.6 files its own index-size refusals under. It is reached only where
    /// the encoding is longer than 18.6's layout — a `bytea` dense in NUL bytes (each
    /// escaped to two), a long `numeric` (a byte a digit against 18.6's two bytes per
    /// four) — so a key 18.6 accepts can meet it (`EDGES.md` row 192).
    #[error(
        "index key for index \"{index}\" encodes to {len} bytes, over the {max} a pglet B+tree page can hold"
    )]
    IndexKeyTooLongForPage {
        index: String,
        len: usize,
        max: usize,
    },

    /// A statement whose expression or plan tree is deeper than this thread's walkers
    /// are allowed to recurse — SQLSTATE `54001` (`statement_too_complex`), 18.6's
    /// `check_stack_depth` wording. 18.6 adds a hint naming `max_stack_depth`; pglet
    /// has no such setting, so the hint is omitted rather than invented.
    #[error("stack depth limit exceeded")]
    StackDepthExceeded,

    /// An argument outside a power/root function's domain — zero raised to a
    /// negative power (`0 ^ -1`), a negative base raised to a non-integer exponent
    /// (`(-2) ^ 0.5`), or the square root of a negative number (`sqrt(-1)`).
    /// Postgres raises SQLSTATE `2201F` (`invalid_argument_for_power_function`);
    /// the `0` field is the Postgres-shaped sentence (they differ per case), so it
    /// renders byte-for-byte. Shared by the float8 and numeric `^`/`sqrt` paths.
    #[error("{0}")]
    InvalidArgumentForPowerFunction(String),

    /// A scalar-function call that resolves to no supported function signature — a
    /// wrong argument count or an argument of the wrong type (e.g. `length(5)`,
    /// `upper('a', 'b')`). Postgres raises SQLSTATE `42883` (`undefined_function`):
    /// `function foo(integer) does not exist`. pglet reports it at plan time for a
    /// bad arity and at eval time for a bad argument type (it dispatches on the
    /// function name, then checks the argument once its value is known). Distinct
    /// from `NotImplemented` (`0A000`), which is a construct pglet models no support
    /// for at all — here the function exists but this call doesn't match it.
    #[error("{0}")]
    UndefinedFunction(String),

    /// A call 18.6's function resolution cannot narrow to one signature — `mod('7', '3')`,
    /// whose two *unknown* arguments fit each of `mod`'s four overloads equally.
    /// SQLSTATE `42725` (`ambiguous_function`): `function mod(unknown, unknown) is not
    /// unique`.
    #[error("function {0} is not unique")]
    AmbiguousFunction(String),

    /// A call carrying an **aggregate-only modifier** over a name that resolves to an
    /// ordinary function. Postgres raises SQLSTATE `42809` (`wrong_object_type`) from
    /// one block of `ParseFuncOrColumn` (`parse_func.c`), which tests each aggregate-only
    /// modifier in turn under one guard — `fdresult` being `FUNCDETAIL_NORMAL`,
    /// `FUNCDETAIL_PROCEDURE` **or** `FUNCDETAIL_COERCION`, of which pglet reaches only
    /// the first: it models no procedures, and a type name used as a single-argument
    /// coercion (`func_get_detail`'s `FuncNameAsType` path) is not in its function table
    /// at all, so `SELECT int4(DISTINCT '1')` is 18.6's `42809` against pglet's
    /// `0A000 function not supported: int4` — the unmodeled-function class, which the
    /// unquantified `SELECT int4('1')` shows by answering the same `0A000` against a
    /// row. Five of the block's seven arms
    /// share one sentence — `%s(*)`, `DISTINCT`, `WITHIN GROUP`, `ORDER BY`,
    /// `FILTER` (the block's `OVER` and null-treatment arms word theirs differently).
    /// `modifier` is that spelling, so one variant reproduces those five: `now(*)` for
    /// the wildcard arm (the name appears twice, as it does in the C `errmsg`) and
    /// `DISTINCT` for the set quantifier. pglet models those two. The other three have no
    /// grammar here at all, so each is `42601` **here** — and that is a divergence rather
    /// than an agreement, which this doc claimed the other way round until it was
    /// measured: `upper('a' ORDER BY 1)` is 18.6's `42809 ORDER BY specified, …` and
    /// `upper('a') FILTER (WHERE true)` its `42809 FILTER specified, …`, while a
    /// `WITHIN GROUP` first adds the sort argument to the signature — so
    /// `upper('a') WITHIN GROUP (ORDER BY 1)` is `42883 function upper(unknown, integer)
    /// does not exist` and only a name that still resolves reaches the arm
    /// (`concat('a') WITHIN GROUP (ORDER BY 'b')` → `42809 WITHIN GROUP specified, …`).
    /// None of the three is pinnable by a fixture — `@expect-error` compares the two
    /// SQLSTATEs and `42601` is not 18.6's — so `EDGES.md`'s row-51 bullet and this doc
    /// are the record.
    ///
    /// **Both arms are reached only when the name resolves**, because `func_get_detail`
    /// runs first — so a call whose *argument list* matches no signature is `42883`
    /// ahead of this. For the wildcard that means a zero-argument signature, since a
    /// `*` contributes no argument: on 18.6 `now(*)` and `gen_random_uuid(*)` are this
    /// error while `upper(*)` / `sum(*)` are `42883 function upper() does not exist`.
    /// For `DISTINCT` it means an ordinary resolution, so `upper(DISTINCT 'a')` is this
    /// error and `upper(DISTINCT 'a', 'b')` is `42883` (measured on both engines).
    ///
    /// The wildcard arm therefore fires for exactly seven names, and the list is a
    /// **measured enumeration over the whole function table rather than a rule about
    /// builtins**: the six niladic impure builtins, plus `jsonb_build_object` (`SELECT
    /// jsonb_build_object()` answers `{}` on both engines). The two arms sit in
    /// different halves of `build_call`, so a name added later needs asking again.
    /// Six of the seven agree with 18.6; the seventh, `uuid_generate_v4`, does not —
    /// the oracle carries no `uuid-ossp` extension, so `uuid_generate_v4(*)` is
    /// `42883 function uuid_generate_v4() does not exist` there. That is a deliberate
    /// keep: pglet models the function as existing, and `42809` is what a Postgres
    /// *with* the extension answers. Every other modeled name keeps the generic
    /// `NotImplemented` wildcard refusal, a recorded loud divergence against 18.6's
    /// `42883`/`42601` (`EDGES.md`,
    /// *Wrong-SQLSTATE-class*).
    ///
    /// The `DISTINCT` arm fires for a modeled non-aggregate whose call **otherwise
    /// resolves** — so not for one whose argument count is already wrong (`42883` wins),
    /// not for one whose *arguments* fail to transform (18.6 transforms them first, so an
    /// unparseable constant cast is `22P02` ahead of this: `upper(DISTINCT 'a'::int)`,
    /// measured on both, and `planner::transform_call_args` is where that runs), and not
    /// for the names 18.6 spells in its own grammar (see
    /// `planner::grammar_only_name`), which refuse the quantifier as a syntax error
    /// there and here.
    ///
    /// It is *not* a relation-kind error despite sharing `42809` with
    /// [`Self::WrongObjectType`] and its two siblings — nothing here names a
    /// relation, so it carries the function's spelled name alone.
    #[error("{modifier} specified, but {name} is not an aggregate function")]
    NotAnAggregate {
        /// The modifier as Postgres spells it in the message — `"<name>(*)"` or
        /// `"DISTINCT"`.
        modifier: String,
        /// The function's spelled name.
        name: String,
    },

    /// A parameterless aggregate called with an empty argument list instead of `(*)`.
    /// Postgres raises SQLSTATE `42809` (`wrong_object_type`) from `ParseFuncOrColumn`
    /// once the empty list has resolved the zero-argument aggregate — so it is the name's
    /// answer only where such an aggregate exists: `count()` is this error on 18.6 while
    /// `sum()` is `42883 function sum() does not exist`.
    #[error("{name}(*) must be used to call a parameterless aggregate function")]
    ParameterlessAggregateWithoutStar {
        /// The aggregate's spelled name.
        name: String,
    },

    /// An ordered-set aggregate called without `WITHIN GROUP`. Postgres raises SQLSTATE
    /// `42809` (`wrong_object_type`) from `ParseFuncOrColumn` when an argument list
    /// resolves a name to one: `rank(a) OVER (ORDER BY a)` resolves the hypothetical-set
    /// `rank(VARIADIC "any")` aggregate rather than the zero-argument `rank()` window
    /// function, and is refused here rather than at the `OVER`.
    #[error("WITHIN GROUP is required for ordered-set aggregate {name}")]
    WithinGroupRequired {
        /// The aggregate's spelled name.
        name: String,
    },

    /// An `ON CONFLICT (target)` whose arbiter matches no unique/exclusion
    /// constraint on the table. Postgres raises SQLSTATE `42P10`
    /// (`invalid_column_reference`): `there is no unique or exclusion constraint
    /// matching the ON CONFLICT specification`.
    #[error("there is no unique or exclusion constraint matching the ON CONFLICT specification")]
    NoConflictTarget,

    /// A `FOREIGN KEY` whose referenced columns are covered by no PK/UNIQUE key on
    /// the parent table (or an omitted `REFERENCES parent` whose parent has no
    /// PRIMARY KEY). Postgres raises SQLSTATE `42830` (`invalid_foreign_key`):
    /// `there is no unique constraint matching given keys for referenced table "x"`.
    /// Checked at CREATE / ALTER ADD CONSTRAINT time, so a bad FK fails loud rather
    /// than being stored unenforceable.
    #[error("there is no unique constraint matching given keys for referenced table \"{table}\"")]
    NoUniqueForForeignKey { table: String },

    /// A `FOREIGN KEY` whose referencing and referenced column lists have different
    /// lengths (`FOREIGN KEY (a, b) REFERENCES parent (x)`). Postgres raises SQLSTATE
    /// `42830` (`invalid_foreign_key`): `number of referencing and referenced columns
    /// for foreign key disagree`.
    ///
    /// 18.6 raises it after the referenced key resolves. pglet raises a *written* list's
    /// mismatch in the parser once it has read the key's clause — whose own `MATCH` and
    /// `ON DELETE` / `ON UPDATE` refusals (`MATCH FULL`'s and `SET DEFAULT`'s `0A000`, a
    /// malformed action's `42601`) come first — so ahead of anything the parser would refuse
    /// after that clause and of every verdict past the parse, a missing
    /// referenced relation's `42P01`, either list's `42703`, the covering key and an `IF
    /// [NOT] EXISTS` skip among them (`EDGES.md` row 275); an omitted list's, resolved to
    /// the parent's primary key, is the executor's.
    #[error("number of referencing and referenced columns for foreign key disagree")]
    ForeignKeyColumnMismatch,

    /// A `FOREIGN KEY` whose **referenced** list names one parent column twice
    /// (`REFERENCES parent (x, x)`). 18.6 refuses it in `transformFkeyCheckAttrs`
    /// — the SQL standard forbids it, and the list-to-index match would be
    /// ambiguous about which opclass pairs with which key. Same `42830` as
    /// [`Self::NoUniqueForForeignKey`], raised ahead of it.
    ///
    /// The **referencing** list carries no such rule: `FOREIGN KEY (a, a)
    /// REFERENCES parent (x, y)` is accepted by both engines.
    #[error("foreign key referenced-columns list must not contain duplicates")]
    ForeignKeyDuplicateReferencedColumn,

    /// A column a `FOREIGN KEY` names — on either side, the referencing list or the
    /// referenced one — that its table does not have. 18.6's `transformColumnNameList`
    /// raises SQLSTATE `42703` (`undefined_column`), naming neither the table nor the
    /// side, after the referenced relation resolves and ahead of the referenced list's
    /// repeat, the covering key and the arity (`42830`). pglet keeps that order except
    /// for a *written* list's arity mismatch, which its parser refuses ahead of this
    /// ([`Self::ForeignKeyColumnMismatch`]).
    #[error("column \"{column}\" referenced in foreign key constraint does not exist")]
    ForeignKeyColumnNotFound { column: String },

    /// A `FOREIGN KEY` one of whose column pairs has no equality 18.6 can implement
    /// through the referenced key's B-tree operator family — `ATAddForeignKeyConstraint`'s
    /// SQLSTATE `42804` (`datatype_mismatch`), naming the first such pair in key order.
    /// See [`ForeignKeyTypeMismatch`](crate::error_fields::ForeignKeyTypeMismatch).
    #[error("foreign key constraint \"{}\" cannot be implemented", .0.constraint)]
    ForeignKeyCannotBeImplemented(Box<crate::error_fields::ForeignKeyTypeMismatch>),

    /// A `SELECT DISTINCT ON (…)` whose expressions don't match the leading
    /// `ORDER BY` expressions. Postgres raises SQLSTATE `42P10`
    /// (`invalid_column_reference`): every `DISTINCT ON` expression must appear
    /// among the initial `ORDER BY` items (before any non-`DISTINCT ON` sort key),
    /// so the first row of each group is well-defined.
    #[error("SELECT DISTINCT ON expressions must match initial ORDER BY expressions")]
    DistinctOnOrderByMismatch,

    /// A `SELECT DISTINCT` whose `ORDER BY` names an expression outside the select
    /// list (`SELECT DISTINCT pa.x … ORDER BY pb.x`). Postgres raises SQLSTATE
    /// `42P10` (`invalid_column_reference`) from `transformDistinctClause`: the dedup
    /// collapses rows, so a sort key the output doesn't carry has no well-defined
    /// value among the survivors. Matched by target-entry identity, not spelling — a
    /// qualified reference to a projected column, an output alias, and the select-list
    /// expression itself all satisfy it.
    #[error("for SELECT DISTINCT, ORDER BY expressions must appear in select list")]
    DistinctOrderByNotInSelectList,

    /// A positional `ORDER BY` / `DISTINCT ON` ordinal outside the select-list
    /// range (`SELECT DISTINCT ON (5) 1`, `… UNION … ORDER BY 5`). Postgres raises
    /// SQLSTATE `42P10` (`invalid_column_reference`): `<clause> position <n> is not
    /// in select list`, where `<clause>` is `ORDER BY` or `DISTINCT ON`.
    #[error("{clause} position {position} is not in select list")]
    InvalidColumnPosition { clause: &'static str, position: i64 },

    /// A bare **non-integer** constant used as an `ORDER BY` / `DISTINCT ON` key
    /// (`ORDER BY 1.5`, `ORDER BY 'x'`, `ORDER BY true`, `ORDER BY NULL`,
    /// `DISTINCT ON (1.5)`). Only an integer literal is a positional column
    /// reference; any other bare constant can be neither a position nor a
    /// meaningful sort expression, so Postgres raises SQLSTATE `42601`
    /// (`syntax_error`): `non-integer constant in <clause>`. Distinct from an
    /// out-of-range integer ordinal ([`Self::InvalidColumnPosition`], `42P10`); a
    /// non-constant expression like `ORDER BY 1+1` is *not* rejected. `<clause>` is
    /// `ORDER BY` or `DISTINCT ON`.
    #[error("non-integer constant in {clause}")]
    NonIntegerConstant { clause: &'static str },

    /// A derived-table column-alias list that names more columns than the relation
    /// provides (`SELECT * FROM (VALUES (1, 2)) AS t(a, b, c)`). Postgres raises
    /// SQLSTATE `42P10` (`invalid_column_reference`): `table "t" has N columns
    /// available but M columns specified`. Fewer aliases than columns is not an error
    /// (the trailing columns keep their default names).
    #[error(
        "table \"{table}\" has {available} columns available but {specified} columns specified"
    )]
    TableColumnCountMismatch {
        table: String,
        available: usize,
        specified: usize,
    },

    /// A `WITH` query's column-alias list that names more columns than the CTE body
    /// provides (`WITH c(w, x, y, z) AS (SELECT 1, 2, 3) SELECT 1`). Postgres raises
    /// SQLSTATE `42P10` (`invalid_column_reference`) from `analyzeCTE`, with a wording
    /// of its own: `WITH query "c" has N columns available but M columns specified` —
    /// where the FROM-alias route ([`Error::TableColumnCountMismatch`],
    /// `parse_relation.c`) says `table "c"`. The two are separate variants because the
    /// two messages are 18.6's, and a CTE that reached the FROM wording read as a
    /// derived table to anyone debugging it. Fewer aliases than the body's columns is
    /// not an error — the trailing columns keep the body's own names.
    #[error(
        "WITH query \"{name}\" has {available} columns available but {specified} columns specified"
    )]
    CteColumnCountMismatch {
        name: String,
        available: usize,
        specified: usize,
    },

    /// Two `WITH` queries in one clause declared under the same name
    /// (`WITH c AS (…), c AS (…) …`). Postgres raises SQLSTATE `42712`
    /// (`duplicate_alias`) from `transformWithClause`'s name-collision scan, which runs
    /// over the raw `WITH` list **before** any body is analysed — measured, `WITH c AS
    /// (SELECT 1 FROM nosuchtbl), c AS (SELECT 1) SELECT 1` is this and not `42P01`.
    ///
    /// The first `42712` this engine raises. The FROM-item spellings of the same code
    /// (a source aliased to the `UPDATE` target's own name, a duplicated FROM alias)
    /// are still ungated — `EDGES.md`.
    #[error("WITH query name \"{name}\" specified more than once")]
    DuplicateCteName { name: String },

    /// A negative `LIMIT` row count. Postgres raises SQLSTATE `2201W`
    /// (`invalid_row_count_in_limit_clause`); pglet matches rather than silently
    /// treating the clause as unbounded (returning every row).
    #[error("LIMIT must not be negative")]
    NegativeLimit,

    /// A negative `OFFSET` row count. Postgres raises SQLSTATE `2201X`
    /// (`invalid_row_count_in_offset_clause`); pglet matches rather than silently
    /// treating the clause as zero.
    #[error("OFFSET must not be negative")]
    NegativeOffset,

    /// A non-aggregated column in an aggregated query that is neither in the
    /// `GROUP BY` nor functionally dependent on it. Postgres raises SQLSTATE
    /// `42803` (`grouping_error`): `column "X" must appear in the GROUP BY clause
    /// or be used in an aggregate function`. Validated at plan time, so the engine
    /// fails loud instead of returning an arbitrary group row.
    #[error(
        "column \"{column}\" must appear in the GROUP BY clause or be used in an aggregate function"
    )]
    GroupingError { column: String },

    /// A `DROP SCHEMA name` (without `IF EXISTS`) naming a schema that doesn't
    /// exist. pglet models a single implicit `public` schema, so any other name
    /// is "not found". Postgres raises SQLSTATE `3F000` (`invalid_schema_name`):
    /// `schema "x" does not exist`.
    #[error("schema \"{0}\" does not exist")]
    SchemaNotFound(String),

    /// A `DROP SCHEMA public` (without `CASCADE`) while the schema still holds
    /// objects (a table, an enum type or a sequence). Postgres raises SQLSTATE
    /// `2BP01` (`dependent_objects_still_exist`): `cannot drop schema public
    /// because other objects depend on it`. The reset idiom uses `CASCADE`.
    #[error("cannot drop schema {0} because other objects depend on it")]
    DependentObjectsExist(String),

    /// A `DROP TABLE` / `ALTER TABLE … DROP COLUMN` (without `CASCADE`) blocked by a
    /// `FOREIGN KEY` constraint that is **not** removed along with the target — one
    /// declared on another table (or, for a referenced *column*, one on any table
    /// including this one) and referencing it. Postgres records that as a NORMAL
    /// `pg_depend` edge, which `RESTRICT` (the default) refuses to follow, raising
    /// SQLSTATE `2BP01` (`dependent_objects_still_exist`).
    ///
    /// The message, the dependent's DETAIL line and the CASCADE HINT are three
    /// separate wire fields, as they are on 18.6 (`reportDependentObjects`), and the
    /// identifiers in all three are **unquoted** — the one family where pglet drops
    /// its own quoting convention, because all three strings are compared against the
    /// oracle byte for byte.
    ///
    /// `target` is boxed to keep [`enum@Error`] inside its size budget — see
    /// [`MAX_ERROR_SIZE`].
    #[error("cannot drop {} because other objects depend on it", .target.unquoted())]
    DependentForeignKey {
        target: Box<DropTargetName>,
        constraint: String,
        child_table: String,
    },

    /// A `DROP TABLE` / `ALTER TABLE … DROP COLUMN` / `DROP SEQUENCE` (without
    /// `CASCADE`) blocked by a column whose `DEFAULT` is `nextval` of a sequence the
    /// statement would remove — Postgres's `pg_attrdef` → sequence NORMAL edge, the
    /// same kind as [`Self::DependentForeignKey`] and the same SQLSTATE `2BP01`.
    /// `CASCADE` sheds **that column's default**; the column and its rows survive.
    ///
    /// The edge reaches further than "the sequence's own column": a `SERIAL`'s
    /// sequence is `OWNED BY` the column it feeds, so dropping the *table that owns
    /// it* takes the sequence with it — and any **other** table's default drawing from
    /// that sequence is then a dependent of that drop. Leaving it unreported is what
    /// turns a leaked sequence into a second table whose ids silently restart.
    ///
    /// An **identity** column is not this edge (see [`Self::SequenceRequiredByIdentity`]):
    /// Postgres stores no `pg_attrdef` row for one, so it has nothing to shed. What
    /// earns that exemption is being the sequence's *own* column, not carrying the
    /// identity flag — see `executor::dependencies::identity_sequence_owner`.
    ///
    /// Postgres lists *every* dependent in one message; pglet names one, as it already
    /// does for several blocking foreign keys. The order Postgres uses is neither by
    /// kind nor by the dependent's own OID (both falsified on 18.6 — see
    /// `Dependents::ensure_droppable`), so there is no ordering rule to reproduce and
    /// the divergence is one of completeness.
    #[error("{0}")]
    DependentColumnDefault(Box<ColumnDefaultDependency>),

    /// A `DROP SEQUENCE` naming the sequence behind an identity column
    /// (`GENERATED … AS IDENTITY`). Postgres records an INTERNAL `pg_depend` edge from
    /// the sequence to the column, and `findDependentObjects` refuses it at the
    /// outermost recursion level with **no `DropBehavior` test at all** — so `CASCADE`
    /// does not help, unlike the `pg_attrdef` edge a `SERIAL` carries. SQLSTATE `2BP01`.
    ///
    /// The same shape as [`Self::IndexRequiredByConstraint`]: an object that is the
    /// internal implementation of another cannot be dropped on its own, only with the
    /// thing that owns it.
    #[error("{0}")]
    SequenceRequiredByIdentity(Box<IdentitySequenceDependency>),

    /// A `DROP INDEX` naming the index that backs a declared `PRIMARY KEY` / `UNIQUE`
    /// constraint. Postgres records an INTERNAL `pg_depend` edge from the index to its
    /// constraint, so the drop is refused with SQLSTATE `2BP01` regardless of
    /// `CASCADE` — the constraint must be dropped instead. Without this the index
    /// vanishes, the constraint silently stops enforcing, and the table's dump loses
    /// the key entirely.
    ///
    /// The constraint's name *is* the index's name (pglet mints a constraint's index
    /// under the constraint's name, as Postgres does), so `index` fills both slots of
    /// Postgres's message rather than being stored twice.
    ///
    /// **The tail is not Postgres's hint.** Postgres suggests `ALTER TABLE … DROP
    /// CONSTRAINT`; pglet does not implement that statement (it is parsed and refused
    /// `0A000`), and this refusal closed the one route that used to exist. Saying "drop
    /// the constraint instead" would send the reader at a statement that is refused, so
    /// the message names the only operation pglet actually offers.
    #[error("cannot drop index {index} because constraint {index} on table {table} requires it")]
    IndexRequiredByConstraint { index: String, table: String },

    /// A `DROP TYPE` naming a table's **row type** — the type of the table's own name
    /// that 18.6 creates with every table, and that the table requires by an internal
    /// edge, so `CASCADE` and `IF EXISTS` do not reach it (both measured). The table's
    /// name fills both slots of the message, as it does in Postgres's, and it is carried
    /// **as rendered** — quoted where an identifier needs it (`"Rd_Ab"`, measured on
    /// 18.6 in the message and the hint alike) by the caller, through `pglet_sql`'s
    /// `lexer::quote_ident`.
    #[error("cannot drop type {0} because table {0} requires it")]
    TypeRequiredByTable(String),

    /// A `DROP TYPE` of an enum a persisted expression converts to — a `CHECK`, a column
    /// `DEFAULT` or an index predicate or key holding `x::<enum>` over a non-constant
    /// operand, which is persisted naming the type (`planner::StringFunc::EnumIn`).
    /// SQLSTATE `2BP01`, with the dependent in the DETAIL and the `CASCADE` hint, as
    /// 18.6 sends them — the DETAIL only where there is one dependent
    /// ([`TypeDependency::dependent`]).
    #[error("{0}")]
    DependentOnType(Box<TypeDependency>),

    /// A `CREATE INDEX` key expression (`position` `"expression"`) or partial predicate
    /// (`"predicate"`) holding a function 18.6 does not mark `IMMUTABLE` — a conversion to
    /// an enum, `enum_in` being `STABLE`. SQLSTATE `42P17` (`invalid_object_definition`),
    /// 18.6's `CheckPredicate` / `ComputeIndexAttrs` message.
    #[error("functions in index {position} must be marked IMMUTABLE")]
    IndexFunctionNotImmutable { position: &'static str },

    /// An `EXTRACT`/`date_part`/`date_trunc` field name recognized by neither of
    /// Postgres's token tables (`DecodeUnits` then `DecodeSpecial`) — or, for
    /// `date_trunc`, a field only in the special tier (`dow`/`epoch`/…), which
    /// `date_trunc` never consults. Postgres raises SQLSTATE `22023`
    /// (`invalid_parameter_value`): `unit "x" not recognized for type ...`.
    #[error("unit \"{0}\" not recognized")]
    UnrecognizedTimeUnit(String),

    /// An `EXTRACT`/`date_part`/`date_trunc` field that *is* a recognized unit but
    /// isn't defined for the source type (`extract(hour from date)`,
    /// `date_trunc('week', interval)`, `extract(timezone from timestamp)`). Postgres
    /// raises SQLSTATE `0A000` (`feature_not_supported`): `unit "x" not supported for
    /// type ...`.
    #[error("unit \"{0}\" not supported for this type")]
    UnsupportedTimeUnit(String),

    /// `nextval`/`currval`/`setval` naming a sequence that isn't registered.
    /// Postgres resolves the sequence as a relation, so it raises SQLSTATE `42P01`
    /// (`undefined_table`): `relation "s" does not exist`. Distinct from
    /// `CatalogError::TableNotFound` only in the noun (Postgres says `relation` for a
    /// sequence) — the SQLSTATE is the same.
    #[error("relation \"{0}\" does not exist")]
    SequenceNotFound(String),

    /// A sequence-only statement (`ALTER SEQUENCE …`) naming a relation of the wrong
    /// kind. Postgres's `validate_relation_kind` raises SQLSTATE `42809`
    /// (`wrong_object_type`): `cannot open relation "x"`, with the kind in the DETAIL
    /// line ("This operation is not supported for tables." / "… for indexes.").
    ///
    /// It runs **before** the `IF EXISTS` skip, so `ALTER SEQUENCE IF EXISTS <a
    /// table>` is still this error and not a no-op (measured on 18.6). pglet keeps
    /// tables, indexes and sequences in three separate maps, so a statement that only
    /// looked in `sequences` would report `42P01` here — the wrong error, and the
    /// easiest thing in this area to get silently wrong.
    #[error("cannot open relation \"{relation}\"")]
    WrongRelationKind {
        relation: String,
        kind: RelationKind,
    },

    /// A statement naming a relation of the wrong **kind** — the whole class, both
    /// directions. Postgres raises SQLSTATE `42809` (`wrong_object_type`) for every
    /// shape, each with its own wording, and pglet now splits them the way 18.6's own
    /// source does rather than carrying one sentence for all:
    ///
    /// - **This variant** words `"x" is not a table` — against the **expected** kind —
    ///   plus, on the `Drop` route alone, a `Use DROP INDEX to remove an index.` HINT
    ///   keyed on the kind the name *actually* holds (`dropmsgstringarray`). Its
    ///   readers are the two the `route` field below distinguishes,
    ///   `resolve_relation_for_drop` and `execute_truncate`; 18.6 sends the message
    ///   from both and the HINT from the `DROP` half only (measured). Those two halves
    ///   are two **different** 18.6 functions that happen to spell one sentence, which
    ///   is why the HINT splits by route at all; the `route` field's own doc names each.
    /// - [`Self::WrongRelationKind`] is `validate_relation_kind` (`table.c`), reached
    ///   from `get_table_or_err` for a scan / DML / `COPY` / `CREATE INDEX … ON x` /
    ///   `REFERENCES x`: `cannot open relation "x"` plus the kind in the DETAIL.
    /// - [`Self::SequenceOwnerWrongKind`] is `process_owned_by`'s own wording.
    ///
    /// All three measured on 18.6. `ALTER TABLE`'s fourth wording (`ALTER action …
    /// cannot be performed on relation "x"`) is still answered with one of the above —
    /// a recorded message divergence, not a field one.
    ///
    /// The kind is decided per (found, expected) pair and **not** per statement, which
    /// is why one variant is right where two kind-specific ones were wrong: they read
    /// only `schema.sequences`, so a name an **index** held reported `42P01` — and
    /// `DROP TABLE IF EXISTS <an index>` / `DROP INDEX IF EXISTS <a table>` therefore
    /// *skipped and reported success*, which is the same silent contradiction the
    /// sequence side was fixed to remove (the next statement to name that relation
    /// insists it already exists).
    ///
    /// It outranks a **skip**, not just an error: the relkind is tested on a name that
    /// *resolves*, so only a name no relation holds at all reaches
    /// `DropErrorMsgNonExistent`. `DROP TABLE IF EXISTS`, `DROP INDEX IF EXISTS` and
    /// `CREATE INDEX IF NOT EXISTS <a taken name> ON <it>` all raise this over any
    /// wrong-kind target.
    ///
    /// **One direction is a pglet divergence rather than a refusal Postgres shares:**
    /// `SELECT last_value FROM <a sequence>` is a legal three-column read in Postgres,
    /// and pglet does not model a sequence as a readable relation. It is loud and
    /// recorded (`EDGES.md`) rather than silently `42P01`,
    /// which is the answer that made one name deny its own existence.
    #[error("\"{relation}\" is not {}", .expected.article_noun())]
    WrongObjectType {
        relation: String,
        /// The kind the relation namespace actually holds under this name.
        actual: RelationKind,
        /// The kind the statement required.
        expected: RelationKind,
        /// Which of the two statements that share this **message** raised it — the
        /// only thing it decides is the HINT, and it decides it because the two
        /// differ: `DropErrorMsgWrongType` attaches `Use DROP INDEX to remove an
        /// index.` and `truncate_check_rel` (`tablecmds.c:2404`) attaches nothing at
        /// all (measured on 18.6 for an index and a sequence, both statements).
        route: WrongTypeRoute,
    },

    /// An `OWNED BY <relation>.<column>` naming a relation that is not a table.
    /// Postgres's `process_owned_by` raises SQLSTATE `42809` (`wrong_object_type`)
    /// with its **own** wording — `sequence cannot be owned by relation "x"`, not
    /// `cannot open relation "x"` — and the same DETAIL line naming the kind
    /// (measured on 18.6 for an index and for a sequence).
    ///
    /// A separate variant from [`Self::WrongRelationKind`] precisely because the two
    /// wordings differ: the statement's *target* and its *owner* are two operands of
    /// one statement and each gets its own message, so reusing one error for both
    /// would hand an introspecting reader the wrong sentence. A name no relation
    /// holds at all stays `42P01` (`process_owned_by` opens the relation first).
    #[error("sequence cannot be owned by relation \"{relation}\"")]
    SequenceOwnerWrongKind {
        relation: String,
        kind: RelationKind,
    },

    /// An `ALTER SEQUENCE … OWNED BY` that would move or clear the ownership edge of a
    /// sequence that is an **identity** column's internal implementation. Postgres's
    /// `process_owned_by` raises SQLSTATE `0A000` (`feature_not_supported`): `cannot
    /// change ownership of identity sequence`, with `Sequence "s" is linked to table
    /// "t".` in the DETAIL line (measured on 18.6 for `OWNED BY NONE`, for another
    /// table's column, for a plain sibling column, and for the identity column
    /// *itself*).
    ///
    /// The refusal is what makes an identity column's `DEFAULT nextval` and its
    /// sequence's `owner` two halves of one fact. Without it the edge can be detached
    /// and the sequence then dropped through an ordinary route, leaving the column's
    /// default naming a sequence the catalog lacks — every write `42P01`, and a dump
    /// that emits the `DEFAULT nextval(…)` with no `CREATE SEQUENCE` to match.
    ///
    /// It sits **after** the owner is resolved, not before: `process_owned_by` opens
    /// the named relation, checks its kind and looks up the column first, so a missing
    /// table (`42P01`), a missing column (`42703`) and a wrong-kind owner (`42809`) all
    /// outrank it (all measured on 18.6).
    #[error("cannot change ownership of identity sequence")]
    IdentitySequenceOwnershipChange { sequence: String, table: String },

    /// `currval(seq)` before any `nextval(seq)` in this session. Postgres raises
    /// SQLSTATE `55000` (`object_not_in_prerequisite_state`): `currval of sequence
    /// "s" is not yet defined in this session`. Per-session state — the sequence
    /// exists (else `SequenceNotFound`), it just hasn't been advanced here yet.
    #[error("currval of sequence \"{0}\" is not yet defined in this session")]
    CurrvalNotDefined(String),

    /// A column reference — a bare projected column or a `JOIN … USING (c)` column —
    /// that matches more than one relation in scope. Postgres raises SQLSTATE `42702`
    /// (`ambiguous_column`): `column reference "c" is ambiguous`. For `USING`, the
    /// named column must exist on exactly one relation of each side.
    #[error("column reference \"{column}\" is ambiguous")]
    AmbiguousColumn { column: String },

    /// A `GROUP BY` / `ORDER BY` / `DISTINCT ON` key naming an **output alias** that
    /// two select-list entries carry with *different* expressions. Postgres raises
    /// SQLSTATE `42702` (`ambiguous_column`) with the clause as the subject —
    /// `GROUP BY "a" is ambiguous` — from `findTargetlistEntrySQL92`, which permits
    /// the duplicate only when the two entries denote equal expressions
    /// (`SELECT x AS a, x AS a … GROUP BY a` is fine). Distinct from
    /// [`Self::AmbiguousColumn`], whose subject is a *relation* column matching more
    /// than one FROM relation, and whose message differs.
    #[error("{clause} \"{name}\" is ambiguous")]
    AmbiguousClauseKey { clause: &'static str, name: String },

    /// An INSERT supplying an explicit (non-`DEFAULT`) value for a `GENERATED
    /// ALWAYS AS IDENTITY` column without `OVERRIDING SYSTEM VALUE`. Postgres raises
    /// SQLSTATE `428C9` (`generated_always`): `cannot insert a non-DEFAULT value into
    /// column "x"`. The remedy is `DEFAULT`, or `OVERRIDING SYSTEM VALUE` to force
    /// the literal.
    #[error("cannot insert a non-DEFAULT value into column \"{column}\"")]
    GeneratedAlwaysInsert { table: String, column: String },

    /// An UPDATE setting a `GENERATED ALWAYS AS IDENTITY` column to a non-`DEFAULT`
    /// value. Postgres raises SQLSTATE `428C9` (`generated_always`): `column "x" can
    /// only be updated to DEFAULT`. (UPDATE has no `OVERRIDING` clause, so unlike
    /// INSERT there is no override.) Also raised on the `ON CONFLICT DO UPDATE SET`
    /// path, which never routes through the plain-UPDATE lowering.
    #[error("column \"{column}\" can only be updated to DEFAULT")]
    GeneratedAlwaysUpdate { table: String, column: String },

    /// A `GENERATED … AS IDENTITY` column — inline or via `ALTER … ADD GENERATED` —
    /// whose type is not an integer. Postgres restricts identity columns to
    /// `smallint`/`integer`/`bigint` (an implicit sequence is integer-typed) and
    /// raises SQLSTATE `22023` (`invalid_parameter_value`).
    #[error("identity column type must be smallint, integer, or bigint")]
    IdentityColumnType,

    /// An `ALTER TABLE … ALTER COLUMN … ADD GENERATED …` naming a column that is
    /// already an identity column. Postgres raises SQLSTATE `55000`
    /// (`object_not_in_prerequisite_state`): `column "x" of relation "t" is already
    /// an identity column`.
    #[error("column \"{column}\" of relation \"{table}\" is already an identity column")]
    AlreadyIdentityColumn { table: String, column: String },

    /// An `ALTER TABLE … ALTER COLUMN … ADD GENERATED … AS IDENTITY` naming a column
    /// that is not declared `NOT NULL`. Postgres's `ATExecAddIdentity` raises SQLSTATE
    /// `55000` (`object_not_in_prerequisite_state`): `column "x" of relation "t" must
    /// be declared NOT NULL before identity can be added`.
    ///
    /// It reads the declaration and no row: the refusal is the same over an empty
    /// table, over rows that hold no NULL and over rows that hold one. An identity
    /// column is `NOT NULL`, and the clause may not declare what nothing has verified
    /// of the rows.
    #[error(
        "column \"{column}\" of relation \"{table}\" must be declared NOT NULL before identity can be added"
    )]
    IdentityColumnNotDeclaredNotNull { table: String, column: String },

    /// An `ALTER TABLE … ALTER COLUMN … ADD GENERATED … AS IDENTITY` naming a column
    /// that has a default — a constant's, an expression's or a `SERIAL`'s. Postgres's
    /// `ATExecAddIdentity` raises SQLSTATE `55000`
    /// (`object_not_in_prerequisite_state`): `column "x" of relation "t" already has a
    /// default value`.
    ///
    /// An identity column's value comes from its own sequence, and the clause does not
    /// choose between that and a default the column was given: the default is dropped
    /// first, by the statement that says so.
    #[error("column \"{column}\" of relation \"{table}\" already has a default value")]
    IdentityColumnHasDefault { table: String, column: String },

    /// An `ALTER TABLE … ALTER COLUMN … SET DEFAULT` / `DROP DEFAULT` naming an
    /// identity column. Postgres's `ATExecColumnDefault` raises SQLSTATE `42601`
    /// (`syntax_error`): `column "x" of relation "t" is an identity column`, adding the
    /// hint `Use ALTER TABLE ... ALTER COLUMN ... DROP IDENTITY instead.` on the
    /// `DROP DEFAULT` spelling (measured on 18.6).
    ///
    /// An identity column's default is not the user's to set: it is the engine's link
    /// to the column's own sequence, and pglet — unlike Postgres, which stores no
    /// `pg_attrdef` row for one at all — models it as an ordinary `NextVal` default.
    /// Letting a `SET DEFAULT` re-point it or a `DROP DEFAULT` clear it therefore
    /// severs that link, and the sequence the column no longer draws from stops being
    /// its internal implementation — after which an ordinary `DROP SEQUENCE` succeeds
    /// and the column's default dangles (see
    /// [`Self::IdentitySequenceOwnershipChange`], the same class through the other
    /// half of the fact).
    #[error("column \"{column}\" of relation \"{table}\" is an identity column")]
    IdentityColumnDefault { table: String, column: String },

    /// A `WITH RECURSIVE` whose body isn't a well-formed recursive query: a
    /// combinator other than `UNION`/`UNION ALL` between the terms, an aggregate /
    /// window function in the recursive term, or the self-reference appearing more
    /// than once (a set-op chain). Postgres raises SQLSTATE `42P19`
    /// (`invalid_recursion`): `recursive query "t" does not have the form
    /// non-recursive-term UNION [ALL] recursive-term`. The message is the
    /// Postgres-shaped sentence.
    #[error("{0}")]
    InvalidRecursion(String),

    /// A `WITH RECURSIVE` construct Postgres itself does not implement, refused with
    /// its own sentence — `mutual recursion between WITH items is not implemented`
    /// (`TopologicalSort`), and `ORDER BY` / `OFFSET` / `LIMIT` / `FOR UPDATE/SHARE in
    /// a recursive query is not implemented` (`checkWellFormedRecursion`). SQLSTATE
    /// `0A000` (`feature_not_supported`). Distinct from [`Self::NotImplemented`]: these
    /// are 18.6's refusals, not pglet's, so the message carries no `not implemented:`
    /// prefix of pglet's own.
    #[error("{0}")]
    RecursiveQueryNotImplemented(String),

    /// A window function used where Postgres forbids one — a CHECK constraint, an
    /// index predicate, or an index expression. Postgres raises SQLSTATE `42P20`
    /// (`windowing_error`): `window functions are not allowed in <context>`. pglet
    /// rejects these at DDL time (`context` is the Postgres-matching plural phrase,
    /// e.g. `check constraints`), matching Postgres and — load-bearing — keeping an
    /// `Expr::Window` out of every **persisted** `Expr` context (a window's
    /// `order_by` is a postcard-serialized field, so its shape must never reach the
    /// catalog's CHECK / partial-index predicates). A window reachable only *inside*
    /// a subquery is closed by [`Self::SubqueryNotAllowed`], which fires first.
    #[error("window functions are not allowed in {context}")]
    WindowingError { context: &'static str },

    /// A subquery used where Postgres forbids one — a CHECK constraint, an index
    /// predicate, or an index expression. Postgres raises SQLSTATE `0A000`
    /// (`feature_not_supported`): `cannot use subquery in <context>` (`context` the
    /// **singular** phrase, e.g. `check constraint`). pglet rejects these at DDL
    /// time — matching Postgres, and load-bearing for persistence: a subquery is the
    /// only remaining way an `Expr::Window` (or any plan-bearing `Expr`) could reach
    /// a **persisted** CHECK / index-predicate / index-expression tree, so rejecting
    /// every subquery there makes "no window/plan `Expr` is ever persisted" airtight.
    #[error("cannot use subquery in {context}")]
    SubqueryNotAllowed { context: &'static str },

    /// An aggregate used where Postgres forbids one — a CHECK constraint, an index
    /// predicate, or an index expression. Postgres raises SQLSTATE `42803`
    /// (`grouping_error`): `aggregate functions are not allowed in <context>`
    /// (`context` the plural phrase). pglet rejects these at DDL time to match
    /// Postgres's CHECK/index restrictions (an aggregate carries no `OrderKey`, so
    /// unlike the window/subquery cases this is conformance, not persistence-safety).
    #[error("aggregate functions are not allowed in {context}")]
    AggregateNotAllowed { context: &'static str },

    /// An aggregate call whose argument holds another aggregate call of the same query
    /// level — `avg(avg(i))`, `sum(b + max(b))`. Postgres raises SQLSTATE `42803`
    /// (`grouping_error`) from `check_agg_arguments`, as it transforms the outer call,
    /// after the call's own arguments are transformed.
    #[error("aggregate function calls cannot be nested")]
    NestedAggregate,

    /// A **set-returning** function used where Postgres forbids one. Postgres raises
    /// SQLSTATE `0A000` (`feature_not_supported`): `set-returning functions are not
    /// allowed in <context>` (`context` the plural phrase, e.g. `DEFAULT
    /// expressions`), from the `p_last_srf` check each restricted parse context runs.
    ///
    /// Distinct from [`Self::NotImplemented`] despite sharing `0A000`: this is
    /// Postgres's own refusal of a function it *has*, not pglet's refusal of one it
    /// lacks — the two are only distinguishable by message, so a walk that reports
    /// this must not be replaced by the generic unsupported-construct arm.
    #[error("set-returning functions are not allowed in {context}")]
    SetReturningNotAllowed { context: &'static str },

    /// A **column reference** used where Postgres forbids one — today only a column
    /// `DEFAULT` expression, which is evaluated with no row in scope. Postgres raises
    /// SQLSTATE `0A000` (`feature_not_supported`): `cannot use column reference in
    /// <context>` (`context` the **singular** phrase, e.g. `DEFAULT expression`),
    /// from `transformColumnRef`.
    ///
    /// **Unconditional — the column's existence is never consulted.** There is no
    /// `42703` in this context on either engine: `DEFAULT totally_bogus_name` and
    /// `DEFAULT <a real sibling column>` are the same error, because the refusal is
    /// raised before name resolution runs. That is the opposite of the CHECK /
    /// index-expression contexts, where a column reference is legal and only an
    /// *unknown* one raises ([`crate::CatalogError::ColumnNotFoundInExpr`]).
    #[error("cannot use column reference in {context}")]
    ColumnReferenceNotAllowed { context: &'static str },

    /// A `$N` placeholder in a context that binds no parameters — a CHECK
    /// constraint, an index predicate, or an index key expression. Postgres raises
    /// SQLSTATE `42P02` (`undefined_parameter`) from `transformParamRef`: `there is
    /// no parameter $1`. pglet rejects these at DDL time for the same reason it
    /// rejects `DEFAULT` there: a stored expression is re-evaluated on every write
    /// with no parameter list, so accepting one yields a table that can never be
    /// written to *and* a database that can never be dumped.
    #[error("there is no parameter ${index}")]
    UndefinedParameter { index: usize },

    /// A `$N` parse analysis leaves without a type — an occurrence in a position that
    /// coerces nothing (`$1 IS NULL`, `count($1)`, a `concat` argument) reached while
    /// nothing earlier in the statement had fixed the parameter's type. 18.6 has two
    /// raise sites with one sentence: `42P18` (`indeterminate_datatype`) where no
    /// occurrence fixes the type at all, and `42P08` (`ambiguous_parameter`) where a
    /// later one does (`fixed_later`), from `check_variable_parameters`.
    #[error("could not determine data type of parameter ${index}")]
    IndeterminateParameter { index: usize, fixed_later: bool },

    /// A statement issued inside an **aborted** transaction block — one whose
    /// earlier statement raised — that is not one of the statements ending the
    /// block. Postgres raises SQLSTATE `25P02` (`in_failed_sql_transaction`) with
    /// exactly this message, from every phase that gates the state.
    ///
    /// It is an `Error` variant rather than a hand-built wire error because both
    /// hosts raise it: the embed facade returns it from `execute`, and the server
    /// encodes it at four protocol phases. One variant is one message and one
    /// SQLSTATE for all five sites, which is what keeps them byte-identical.
    #[error("current transaction is aborted, commands ignored until end of transaction block")]
    AbortedTransaction,

    /// A write that waited for **the writer** — the one connection whose block may
    /// stage writes — past the session's `lock_timeout`. Postgres raises SQLSTATE
    /// `55P03` (`lock_not_available`) with exactly this message and no detail, from
    /// `postgres.c`'s `ProcessInterrupts` lock-timeout arm. Inside a block it aborts the
    /// block, as any statement error does.
    #[error("canceling statement due to lock timeout")]
    LockTimeout,

    /// A runtime parameter assigned a value its type refuses — Postgres's
    /// `parse_and_validate_value`, SQLSTATE `22023` (`invalid_parameter_value`):
    /// `invalid value for parameter "…": "…"` for a value that does not parse (with the
    /// units hint where a unit was the trouble), or `… is outside the valid range for
    /// parameter …` for one that parses past the bounds.
    #[error("{message}")]
    InvalidParameterValue {
        message: String,
        hint: Option<String>,
    },

    /// A write the embedded `Database` cannot let wait: another logical connection's
    /// block holds the writer, or has a block open when this connection asks for
    /// transaction control, and one thread cannot wait on itself. SQLSTATE `55P03`
    /// (`lock_not_available`), the code 18.6 gives a lock it declines to wait for
    /// (`NOWAIT`); the message is pglet's, naming the holder.
    #[error("{0}")]
    WriterHeldElsewhere(String),

    /// A `REPEATABLE READ` or `SERIALIZABLE` block's first write, refused because
    /// something committed since the block's snapshot. SQLSTATE `40001`
    /// (`serialization_failure`) with the message 18.6's executor gives a write that
    /// meets a row updated since its snapshot (`nodeModifyTable.c`), and no detail. The
    /// check is pglet's and coarser than 18.6's per-row one: it asks whether anything
    /// committed, not whether the rows the block writes were touched. Inside a block it
    /// aborts the block, as any statement error does.
    #[error("could not serialize access due to concurrent update")]
    SerializationFailure,

    /// A transaction's isolation level set after it can no longer change — 18.6's
    /// `check_transaction_isolation`, SQLSTATE `25001` (`active_sql_transaction`), with
    /// its message: `… must be called before any query` once the block has read, `…
    /// must not be called in a subtransaction` inside a savepoint.
    #[error("{0}")]
    TransactionIsolationFixed(&'static str),

    /// A `SAVEPOINT` / `ROLLBACK TO SAVEPOINT` / `RELEASE SAVEPOINT` issued with no
    /// transaction block open. Postgres raises SQLSTATE `25P01`
    /// (`no_active_sql_transaction`) from `CheckTransactionBlock`, naming the
    /// statement — and it names the *canonical* spelling, not the one written: a
    /// bare `ROLLBACK TO s` reports `ROLLBACK TO SAVEPOINT`, and a bare `RELEASE s`
    /// reports `RELEASE SAVEPOINT`. So `stmt` carries the canonical name rather
    /// than being reconstructed from the parsed text.
    #[error("{stmt} can only be used in transaction blocks")]
    NoActiveSqlTransaction { stmt: &'static str },

    /// A portal the session does not hold — an extended-protocol `Describe`/`Execute`
    /// naming one that was never bound, or was bound and then closed. Postgres raises
    /// SQLSTATE `34000` (`undefined_cursor`) from `postgres.c` with exactly this
    /// message. The wire's `Close('P')` on the same store is a silent no-op, the same
    /// asymmetry [`Error::UndefinedPreparedStatement`] carries for statements.
    #[error("portal \"{name}\" does not exist")]
    UndefinedPortal { name: String },

    /// A prepared statement the session does not hold — `DEALLOCATE <name>`, or an
    /// extended-protocol `Bind`/`Describe` naming one. Postgres raises SQLSTATE
    /// `26000` (`undefined_pstatement`) from `FetchPreparedStatement` with exactly
    /// this message and no detail.
    ///
    /// One variant for all three sites because the wire's own `Close` on the *same*
    /// store is deliberately silent (`DropPreparedStatement`'s `showError = false`):
    /// the difference between the two behaviours is a flag at the call site, so the
    /// message and code they share must not be typed out per site.
    #[error("prepared statement \"{name}\" does not exist")]
    UndefinedPreparedStatement { name: String },

    /// An operation refused because a transaction block is open on the session.
    /// Postgres raises SQLSTATE `25001` (`active_sql_transaction`) from
    /// `PreventInTransactionBlock`, naming the operation, and this variant keeps
    /// that message shape. `op` carries the caller's own name so one variant serves
    /// every such refusal — what a caller needs is "a block is open", not a
    /// per-operation arm.
    #[error("{op} cannot run inside a transaction block")]
    ActiveSqlTransaction { op: &'static str },

    /// A `ROLLBACK TO SAVEPOINT` / `RELEASE SAVEPOINT` naming a savepoint the block
    /// does not hold — never established, already released, or destroyed by a
    /// `ROLLBACK TO` / `RELEASE` of an outer one. Postgres raises SQLSTATE `3B001`
    /// (`invalid_savepoint_specification`) with exactly this message, **and the
    /// block goes on to abort** — that second half is not this variant's to do, it
    /// falls out of the ordinary rule that a statement raising inside a block aborts
    /// it.
    #[error("savepoint \"{0}\" does not exist")]
    SavepointNotFound(String),

    /// A `search_path` naming a schema a pglet database does not have, from either
    /// route that can set one — the `SET` statement or
    /// `set_config('search_path', …)`, which reach one decider
    /// (`catalog::check_search_path_elements`). Deliberately **stricter than
    /// Postgres**, which accepts any name here (a schema may be created later, and a
    /// missing one is skipped at resolution) — so there is no Postgres SQLSTATE to
    /// match and this takes `0A000` (`feature_not_supported`), the code pglet already
    /// uses for a construct it recognizes and does not implement.
    ///
    /// Accepting it is a silent write to the wrong place: a schema-per-test harness
    /// that sets `search_path` to its own schema would read and write the shared one
    /// while believing it was isolated.
    ///
    /// What is closed, exactly: a list *naming* another schema, written as a `SET`
    /// statement or as `set_config` with a **literal** setting name. Two things are
    /// not, and each has a ledger row rather than a sentence here — a list naming
    /// **no** schema (the empty one `pg_dump`'s preamble writes) is accepted and still
    /// resolves to `public` (row 66), and a `set_config` whose setting name is
    /// *computed* never reaches the decider at all, so it is refused outright instead
    /// (row 70).
    #[error("unsupported search_path \"{requested}\": pglet has only the \"public\" schema")]
    UnsupportedSearchPath { requested: String },

    /// `set_config(NULL, …)` — Postgres raises SQLSTATE `22004`
    /// (`null_value_not_allowed`) from `set_config_by_name` with exactly this
    /// message, before it looks the name up. pglet used to echo the *value*
    /// argument and report success.
    #[error("SET requires parameter name")]
    SetRequiresParameterName,

    #[error("not implemented: {0}")]
    NotImplemented(String),
}

impl Error {
    /// Returns the SQLSTATE code for this error.
    // arms grouped one-per-variant to mirror Postgres's SQLSTATE catalog;
    // UnindexableType and NotImplemented intentionally share 0A000
    // (feature_not_supported) but stay separate for the per-variant mapping
    #[allow(clippy::match_same_arms)]
    pub const fn sqlstate(&self) -> &'static str {
        match self {
            Self::Syntax { .. } => "42601",
            Self::Catalog(e) => e.sqlstate(),
            Self::Type(_) => "42804",
            Self::TypeCoercion { .. } => "42846", // cannot_coerce
            Self::CannotConvertType { .. } => "42846", // cannot_coerce
            Self::InvalidEnumValue { .. } => "22P02", // invalid_text_representation
            Self::InvalidJson => "22P02",         // invalid_text_representation
            Self::UnsupportedJsonbVersion { .. } => "XX000", // internal_error
            Self::InsufficientDataLeft | Self::NoDataLeft => "08P01", // protocol_violation
            Self::IncorrectBinaryParamFormat { .. }
            | Self::ArrayElementLengthInvalid
            | Self::ImproperArrayElementFormat { .. }
            | Self::InvalidNumericSign
            | Self::InvalidNumericScale
            | Self::InvalidNumericDigit => "22P03", // invalid_binary_representation
            Self::BinaryArrayElementTypeMismatch { .. } => "42804", // datatype_mismatch
            Self::InvalidInputSyntax { .. } => "22P02", // invalid_text_representation
            Self::InvalidDatetimeFormat { .. } => "22007", // invalid_datetime_format
            Self::CopyFormat(_) => "22P04",       // bad_copy_file_format
            Self::NotNullViolation { .. } => "23502", // not_null_violation
            Self::CheckViolation { .. } | Self::CheckConstraintNotSatisfied { .. } => "23514", // check_violation
            Self::NotNullColumnHasNulls { .. } => "23502", // not_null_violation
            Self::DuplicateIndexKey { .. } => "23505",     // unique_violation
            Self::UniqueViolation { .. } => "23505",       // unique_violation
            Self::OnConflictRowAffectedTwice => "21000",   // cardinality_violation
            Self::ForeignKeyViolation(_) => "23503",       // foreign_key_violation
            Self::RestrictViolation(_) => "23001",         // restrict_violation
            Self::DuplicateType(_) | Self::RelationTypeAlreadyExists(_) => "42710", // duplicate_object
            Self::ExtensionNotAvailable(_) => "0A000", // feature_not_supported
            Self::UndefinedExtension(_) => "42704",    // undefined_object
            Self::Execution(_) => "22000",
            Self::WriteRefused { disk_full, .. }
            | Self::DurableWriteFailed { disk_full, .. }
            | Self::DurableWriteUnconfirmed { disk_full, .. } => {
                if *disk_full { "53100" } else { "58030" } // disk_full / io_error
            }
            Self::DatabaseMustBeReopened(_) => "57P02", // crash_shutdown
            Self::FloatPrecisionTooSmall { .. } | Self::FloatPrecisionTooLarge { .. } => "22023", // invalid_parameter_value
            // The refusals a carrier's own modifier reader raises — `anychar_typmodin`
            // and `numerictypmodin`, both `ERRCODE_INVALID_PARAMETER_VALUE`.
            Self::CharacterLengthTooSmall { .. }
            | Self::CharacterLengthTooLarge { .. }
            | Self::NumericPrecisionOutOfRange { .. }
            | Self::NumericScaleOutOfRange { .. }
            | Self::InvalidNumericTypeModifier => "22023", // invalid_parameter_value
            Self::UnrecognizedParameter(_) => "42704", // undefined_object
            Self::DivisionByZero => "22012",           // division_by_zero
            Self::NumericOutOfRange(_)
            | Self::NumericFormatOverflow
            | Self::FloatInputOutOfRange { .. } => "22003", // numeric_value_out_of_range
            Self::StringDataRightTruncation { .. } => "22001", // string_data_right_truncation
            Self::InvalidEncoding { .. } => "22021",   // character_not_in_repertoire
            Self::InvalidUnicodeEscape => "22025",     // invalid_escape_sequence
            Self::UnsupportedUnicodeEscape => "22P05", // untranslatable_character
            Self::InvalidName => "42602",              // invalid_name
            Self::InvalidIntervalSyntax(_) => "22007", // invalid_datetime_format
            Self::IntervalFieldOverflow(_) => "22015", // interval_field_overflow
            Self::TemporalOutOfRange(_)
            | Self::DatetimeFieldValueOutOfRange(_)
            | Self::CannotSubtractInfiniteDates => "22008", // datetime_field_overflow
            Self::TimeZoneDisplacementOutOfRange(_) => "22009", // invalid_time_zone_displacement_value
            Self::NegativeSubstringLength => "22011",           // substring_error
            Self::FieldPositionZero => "22023",                 // invalid_parameter_value
            Self::InvalidHexData(_) => "22023",                 // invalid_parameter_value
            Self::ProgramLimitExceeded(_)
            | Self::BtreeIndexRowTooLarge(_)
            | Self::GinIndexRowTooLarge(_)
            | Self::IndexRowRequiresTooMuch { .. }
            | Self::IndexKeyTooLongForPage { .. } => "54000", // program_limit_exceeded
            Self::StackDepthExceeded => "54001",                // statement_too_complex
            Self::InvalidArgumentForPowerFunction(_) => "2201F", // invalid_argument_for_power_function
            Self::UndefinedFunction(_) => "42883",               // undefined_function
            Self::AmbiguousFunction(_) => "42725",               // ambiguous_function
            Self::UnindexableType(_) => "0A000",                 // feature_not_supported
            Self::UnsupportedType { .. } => "42704",             // undefined_object
            Self::NoConflictTarget => "42P10",                   // invalid_column_reference
            Self::NoUniqueForForeignKey { .. } => "42830",       // invalid_foreign_key
            Self::ForeignKeyColumnMismatch => "42830",           // invalid_foreign_key
            Self::ForeignKeyDuplicateReferencedColumn => "42830", // invalid_foreign_key
            Self::ForeignKeyColumnNotFound { .. } => "42703",    // undefined_column
            Self::ForeignKeyCannotBeImplemented(_) => "42804",   // datatype_mismatch
            Self::DistinctOnOrderByMismatch => "42P10",          // invalid_column_reference
            Self::DistinctOrderByNotInSelectList => "42P10",     // invalid_column_reference
            Self::InvalidColumnPosition { .. } => "42P10",       // invalid_column_reference
            Self::NonIntegerConstant { .. } => "42601",          // syntax_error
            Self::TableColumnCountMismatch { .. } => "42P10",    // invalid_column_reference
            Self::CteColumnCountMismatch { .. } => "42P10",      // invalid_column_reference
            Self::DuplicateCteName { .. } => "42712",            // duplicate_alias
            Self::NegativeLimit => "2201W", // invalid_row_count_in_limit_clause
            Self::NegativeOffset => "2201X", // invalid_row_count_in_offset_clause
            Self::GroupingError { .. } => "42803", // grouping_error
            Self::SchemaNotFound(_) => "3F000", // invalid_schema_name
            Self::DependentObjectsExist(_)
            | Self::DependentForeignKey { .. }
            | Self::DependentColumnDefault(_)
            | Self::SequenceRequiredByIdentity(_)
            | Self::IndexRequiredByConstraint { .. }
            | Self::TypeRequiredByTable(_)
            | Self::DependentOnType(_) => "2BP01", // dependent_objects_still_exist
            Self::IndexFunctionNotImmutable { .. } => "42P17", // invalid_object_definition
            Self::UnrecognizedTimeUnit(_) => "22023", // invalid_parameter_value
            Self::UnsupportedTimeUnit(_) => "0A000", // feature_not_supported
            Self::SequenceNotFound(_) => "42P01", // undefined_table
            Self::WrongRelationKind { .. }
            | Self::SequenceOwnerWrongKind { .. }
            | Self::NotAnAggregate { .. }
            | Self::ParameterlessAggregateWithoutStar { .. }
            | Self::WithinGroupRequired { .. }
            | Self::WrongObjectType { .. } => "42809", // wrong_object_type
            Self::IdentitySequenceOwnershipChange { .. } => "0A000", // feature_not_supported
            Self::IdentityColumnDefault { .. } => "42601", // syntax_error
            Self::CurrvalNotDefined(_) => "55000", // object_not_in_prerequisite_state
            Self::AmbiguousColumn { .. } | Self::AmbiguousClauseKey { .. } => "42702", // ambiguous_column
            Self::GeneratedAlwaysInsert { .. } | Self::GeneratedAlwaysUpdate { .. } => "428C9", // generated_always
            Self::IdentityColumnType => "22023", // invalid_parameter_value
            Self::AlreadyIdentityColumn { .. }
            | Self::IdentityColumnNotDeclaredNotNull { .. }
            | Self::IdentityColumnHasDefault { .. } => "55000", // object_not_in_prerequisite_state
            Self::InvalidRecursion(_) => "42P19", // invalid_recursion
            Self::RecursiveQueryNotImplemented(_) => "0A000", // feature_not_supported
            Self::WindowingError { .. } => "42P20", // windowing_error
            Self::SubqueryNotAllowed { .. } => "0A000", // feature_not_supported
            Self::AggregateNotAllowed { .. } | Self::NestedAggregate => "42803", // grouping_error
            // Both share `0A000` with `NotImplemented` — Postgres's own refusals of
            // constructs it supports elsewhere, told apart only by their messages.
            Self::SetReturningNotAllowed { .. } => "0A000", // feature_not_supported
            Self::ColumnReferenceNotAllowed { .. } => "0A000", // feature_not_supported
            Self::UndefinedParameter { .. } => "42P02",     // undefined_parameter
            Self::IndeterminateParameter {
                fixed_later: false, ..
            } => "42P18", // indeterminate_datatype
            Self::IndeterminateParameter {
                fixed_later: true, ..
            } => "42P08", // ambiguous_parameter
            Self::AbortedTransaction => "25P02",            // in_failed_sql_transaction
            Self::LockTimeout | Self::WriterHeldElsewhere(_) => "55P03", // lock_not_available
            Self::SerializationFailure => "40001",          // serialization_failure
            Self::TransactionIsolationFixed(_) => "25001",  // active_sql_transaction
            Self::InvalidParameterValue { .. } => "22023",  // invalid_parameter_value
            Self::UndefinedPreparedStatement { .. } => "26000", // undefined_pstatement
            Self::UndefinedPortal { .. } => "34000",        // undefined_cursor
            Self::ActiveSqlTransaction { .. } => "25001",   // active_sql_transaction
            Self::NoActiveSqlTransaction { .. } => "25P01", // no_active_sql_transaction
            Self::SavepointNotFound(_) => "3B001",          // invalid_savepoint_specification
            Self::UnsupportedSearchPath { .. } => "0A000",  // feature_not_supported
            Self::SetRequiresParameterName => "22004",      // null_value_not_allowed
            Self::NotImplemented(_) => "0A000",
        }
    }
}

impl Error {
    /// A `42601` with pglet's own wording — the body gets the `syntax error: ` prefix
    /// and no position.
    ///
    /// The prefix lives here rather than in the `Display` attribute so a producer that
    /// already writes Postgres's own sentence (`syntax error at or near "X"`) can use
    /// [`Self::syntax_sentence_at`] and not end up with it twice.
    #[must_use]
    pub fn syntax(body: impl std::fmt::Display) -> Self {
        Self::Syntax {
            message: format!("syntax error: {body}"),
            position: None,
        }
    }

    /// A `42601` with pglet's own wording and 18.6's `P` — see [`Self::syntax`].
    /// `position` is a **1-based codepoint** index (`codepoint_position`).
    #[must_use]
    pub fn syntax_at(body: impl std::fmt::Display, position: u32) -> Self {
        Self::Syntax {
            message: format!("syntax error: {body}"),
            position: Some(position),
        }
    }

    /// A `42601` whose `message` is already a complete sentence, with **no** position.
    ///
    /// For a producer that has Postgres's wording but cannot compute its `P`
    /// truthfully; its callers are the enumeration. `reject_type_modifier` sits inside
    /// a resolver taking a type *name* and no span, so there is no offset to compute
    /// from at all. `syntax error at end of input` on the simple-query path has one it
    /// cannot trust: 18.6's position there is one past the *raw* statement's last
    /// character, and `parser::split_statements` hands the session-command readers a
    /// **trimmed** slice, so the length they can see is right only when nothing was
    /// trimmed (measured on 18.6: `'DEALLOCATE '` reports `P=12`, the trimmed length
    /// would say 11). Right-by-accident is the invention this crate's field rule
    /// forbids, so the field is left out — see `docs/errors.md`. The readers take only
    /// the text and cannot tell that path from the extended one, where `Parse` hands
    /// them the raw statement and the length would be exact, so the omission covers
    /// both protocols: five extended-protocol shapes that could carry a correct `P`
    /// (`DEALLOCATE` → 11, `DISCARD` → 8, `RESET` → 6, …) omit it too. An `INSERT`'s
    /// row-length refusals have the first reason: 18.6 points them at a cell, and a lowered
    /// row carries no spans.
    #[must_use]
    pub fn syntax_sentence(sentence: impl Into<String>) -> Self {
        Self::Syntax {
            message: sentence.into(),
            position: None,
        }
    }

    /// A `42601` whose `message` is already a complete sentence — for the producers
    /// that write Postgres's exact `syntax error at or near "X"` shape.
    #[must_use]
    pub fn syntax_sentence_at(sentence: impl Into<String>, position: u32) -> Self {
        Self::Syntax {
            message: sentence.into(),
            position: Some(position),
        }
    }
}

/// 18.6's `P`: the 1-based **Unicode codepoint** index of `byte_offset` within
/// `source`.
///
/// Postgres counts characters, not bytes and not UTF-16 units — measured on 18.6,
/// `SELECT 'ααααα' AS a, FROM t` reports `P=22` where the byte offset is 27, and
/// three emoji advance it by three, not six. A byte offset past the end (the
/// end-of-input case) lands on `source.chars().count() + 1`, which is what 18.6
/// reports for `syntax error at end of input`.
#[must_use]
pub fn codepoint_position(source: &str, byte_offset: usize) -> u32 {
    let clamped = byte_offset.min(source.len());
    let prefix = source.get(..clamped).unwrap_or(source);
    u32::try_from(prefix.chars().count() + 1).unwrap_or(u32::MAX)
}

pub type Result<T> = std::result::Result<T, Error>;

/// The size budget for [`enum@Error`], in bytes.
///
/// Every `Result<T>` in the engine is `Result<T, Error>`, so `Error`'s size is a
/// floor on the stack frame of every fallible function — including the parser's
/// deeply recursive `parse_expr_bp`, which walks one frame per nesting level. Adding
/// a fat variant is therefore not a local change.
///
/// **Measured, not guessed.** The budget was set against
/// `parser::fuzz::deep_nesting_fails_loud_not_overflow`, which drives the parser toward
/// `parser::MAX_DEPTH` on a **debug** build inside libtest's default 2 MiB thread.
/// Measured without the byte check in the parser's nesting, at
/// `size_of::<Error>() == 72` that test consumed more than 1664 KiB and at most
/// 1792 KiB — roughly 85% of the thread — and probing a fattened variant against that
/// same thread gave 72 ✓, 80 ✓, 88 ✓, 96 ✗ (an overflow). Natively the parser's nesting
/// takes `pglet_sql::depth`'s byte check, so a wider `Error` costs depth answered rather
/// than aborting, and that test does not see the cliff: widened to 96 B, `Error`
/// leaves it green on a debug build (measured). On wasm, which has no byte check, the
/// cliff stands, and no native test measures it.
///
/// The budget is therefore set one 8-byte step **below** the last measured survivor,
/// so tripping this assert still leaves real headroom rather than reporting a failure
/// that has already happened. Today `Error` is 72, so there is one step of growth
/// room; **box a payload that doesn't fit rather than raising this number**, and if it
/// must be raised, re-measure the cliff on wasm and revisit the other half of the same
/// product — `pglet_sql`'s `parser::MAX_DEPTH`, which sets how many frames deep the
/// recursion is allowed to go. Neither constant is safe to read alone — and
/// `pglet_sql::depth`'s limits are sized from the same frames, measured by
/// `crates/pglet/tests/stack_depth.rs`.
pub const MAX_ERROR_SIZE: usize = 80;

#[cfg(test)]
mod size_budget {
    use super::{Error, MAX_ERROR_SIZE};

    #[test]
    fn error_fits_its_size_budget() {
        assert!(
            size_of::<Error>() <= MAX_ERROR_SIZE,
            "Error is {} bytes, over the {MAX_ERROR_SIZE}-byte budget — box the new \
             variant's payload rather than widening every Result frame in the engine. \
             The budget sits below a measured stack cliff, not at it: read \
             MAX_ERROR_SIZE's docs before raising it: natively the byte check keeps \
             parser::fuzz::deep_nesting_fails_loud_not_overflow green past the cliff, \
             so re-measure it on wasm if you do",
            size_of::<Error>(),
        );
    }
}
