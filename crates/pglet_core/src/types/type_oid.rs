use serde::{Deserialize, Serialize};

/// Postgres type OIDs we support.
///
/// One-dimensional array types are first-class here: each scalar type has a
/// matching `*Array` variant carrying Postgres's canonical array OID. The
/// element type is recovered with `array_element` and the array OID for an
/// element type with `array_of`. The enum stays `#[repr(u32)]` + `Copy`, so the
/// element type is carried on the `Value::Array` rather than nested in the OID.
// Serialize/deserialize **as the u32 OID**, not the variant ordinal: `#[repr(u32)]`
// alone does not make serde use the `= <oid>` discriminant, so a plain derive would
// key the postcard encoding on variant *order*. Since a `Schema` (with each
// `Column.type_oid`) is postcard-persisted, that would let inserting a variant
// mid-enum silently misread a persisted catalog. Routing through the OID makes
// variant order irrelevant forever and matches the on-the-wire type OID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "u32", try_from = "u32")]
#[repr(u32)]
pub enum TypeOid {
    Bool = 16,
    Int2 = 21,
    Int4 = 23,
    Int8 = 20,
    Float4 = 700,
    Float8 = 701,
    Text = 25,
    Varchar = 1043,
    Bytea = 17,
    Timestamp = 1114,
    Timestamptz = 1184,
    Date = 1082,
    /// `TIME` (time without time zone) — microseconds since midnight, wired as a
    /// bare `i64`. `TIMETZ` (`time with time zone`) stays unmapped and fails loud.
    Time = 1083,
    /// `NUMERIC`/`DECIMAL` — arbitrary-precision exact decimal (see
    /// [`crate::types::PgNumeric`]).
    Numeric = 1700,
    /// `INTERVAL` — Postgres's 3-field `(months, days, micros)` span (see
    /// [`crate::types::PgInterval`]). A full column type: stored, projected,
    /// wire-encoded (16-byte binary + `postgres`-style text), and B-tree-indexable
    /// on the comparison estimate.
    Interval = 1186,
    Jsonb = 3802,
    Uuid = 2950,
    /// `name` — Postgres's fixed-width identifier type, the declared type of
    /// `current_user` / `current_schema` / `current_database()` and of every
    /// identifier column in the system catalogs.
    ///
    /// **Shape-free here**: a `name` value is a [`crate::types::Value::Text`] and the
    /// OID rides on the descriptor, exactly as `varchar` does — the binary encoding
    /// is the raw bytes with no NUL padding (measured on 18.6: `current_user` sends
    /// the role name's bytes and nothing after them), so no codec arm distinguishes it
    /// from `text`. The `64` this
    /// reports as its wire size is `pg_type.typlen`, not a value length.
    ///
    /// It is deliberately **not** a column type: `type_oid_from_name` does not map
    /// `name`, so `CREATE TABLE t (a name)` and `'a'::name` stay `42704`. 18.6
    /// truncates a `name` at 63 characters and pglet does not (`docs/compatibility.md` records
    /// that as self-consistent); nothing here can produce an over-long one, since
    /// every `name`-declared value pglet mints is a session identifier or a schema
    /// name.
    Name = 19,
    /// `oid` — Postgres's object-identifier type, the declared type of every
    /// `*oid`/`*relid`/`*typid` column in the system catalogs.
    ///
    /// **Shape-free here**, exactly like [`Self::Name`]: an `oid` value is a
    /// [`crate::types::Value::Int4`] and the OID rides on the descriptor, so no codec
    /// arm distinguishes it from `int4` (18.6's binary `oid` is the same 4 bytes; its
    /// text form is the same digits for every OID pglet mints, which are all positive
    /// and below `i32::MAX`). It is deliberately **not** a column type:
    /// `type_oid_from_name` does not map `oid`, so `CREATE TABLE t (a oid)` stays
    /// `42704`.
    Oid = 26,
    /// `"char"` — Postgres's single-byte internal character type, the declared type of
    /// `pg_constraint.contype` / `confdeltype` and `pg_class.relkind`.
    ///
    /// **Shape-free here**: a `"char"` value is a [`crate::types::Value::Text`] holding
    /// exactly one ASCII byte, so text and binary encodings are the raw byte either way
    /// (measured on 18.6: `contype` binary for a CHECK is `63`). Not a column type
    /// (`type_oid_from_name` does not map it).
    Char = 18,
    /// `void` — the declared result type of a function that returns nothing
    /// (`pg_advisory_xact_lock`), a pseudo-type in 18.6's `pg_type` (`typtype = 'p'`,
    /// `typcategory = 'P'`, `typlen = 4`).
    ///
    /// **Shape-free here**, like [`Self::Name`]: a `void` value is a
    /// [`crate::types::Value::Text`] holding the empty string and the OID rides on the
    /// descriptor. 18.6's `void_out` answers `''` and `void_send` sends zero bytes, so the
    /// text and binary encodings of the empty text are both of it exactly; a NULL is a
    /// NULL (a strict call over a NULL argument). It has no column form, no cast target
    /// and no operator (`void = void` is `42883` on 18.6) — nothing here reads a `void`
    /// operand, because the one producer answers only as a top-level projection item.
    Void = 2278,
    // Array types (canonical Postgres array OIDs). One-dimensional only.
    BoolArray = 1000,
    ByteaArray = 1001,
    Int2Array = 1005,
    Int4Array = 1007,
    TextArray = 1009,
    VarcharArray = 1015,
    Int8Array = 1016,
    Float4Array = 1021,
    Float8Array = 1022,
    TimestampArray = 1115,
    TimestamptzArray = 1185,
    DateArray = 1182,
    TimeArray = 1183,
    IntervalArray = 1187,
    NumericArray = 1231,
    UuidArray = 2951,
    JsonbArray = 3807,
    /// `name[]` — the declared type of `current_schemas(bool)`.
    NameArray = 1003,
}

/// The error a `TypeOid` deserialization yields for an OID it doesn't recognize.
///
/// Surfaced through serde's `try_from` bridge, so a corrupt/forward-version catalog
/// fails loud rather than silently mapping to a wrong type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownTypeOid(pub u32);

impl std::fmt::Display for UnknownTypeOid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown type OID: {}", self.0)
    }
}

impl std::error::Error for UnknownTypeOid {}

impl From<TypeOid> for u32 {
    fn from(oid: TypeOid) -> Self {
        // `#[repr(u32)]` with explicit `= <oid>` discriminants, so this is the OID.
        oid as Self
    }
}

impl TryFrom<u32> for TypeOid {
    type Error = UnknownTypeOid;

    fn try_from(oid: u32) -> Result<Self, Self::Error> {
        // OIDs are small (≤ 3807), so the `as i32` never wraps.
        Self::from_oid(oid as i32).ok_or(UnknownTypeOid(oid))
    }
}

impl TypeOid {
    /// Return the wire protocol size for this type.
    ///
    /// Returns:
    /// - Positive number for fixed-size types (e.g., 4 for INT4, 8 for INT8)
    /// - -1 for variable-length types (TEXT, BYTEA, etc.)
    ///
    /// This is used in `RowDescription` messages to tell clients how many bytes
    /// to expect when reading binary format data.
    #[must_use]
    // arms kept one-per-variant in enum order for documentation; several share a body intentionally
    #[allow(clippy::match_same_arms)]
    pub const fn wire_size(self) -> i16 {
        match self {
            Self::Bool => 1,
            Self::Int2 => 2,
            Self::Int4 => 4,
            Self::Int8 => 8,
            Self::Float4 => 4,
            Self::Float8 => 8,
            Self::Date => 4,
            Self::Time => 8,
            Self::Timestamp | Self::Timestamptz => 8,
            // `interval` is a fixed 16-byte binary value (time i64 + day i32 + month i32).
            Self::Interval => 16,
            Self::Uuid => 16,
            // `pg_type.typlen` for `name` is 64 (NAMEDATALEN); 18.6 reports it in the
            // `RowDescription` even though the value itself is variable-length bytes.
            Self::Name => 64,
            Self::Oid => 4,
            Self::Char => 1,
            // `pg_type.typlen` for `void` is 4, which 18.6 reports in the `RowDescription`
            // although the value it sends is zero bytes.
            Self::Void => 4,
            // Variable-length: text-family, bytea, jsonb, and all arrays.
            // NUMERIC is variable-length on the wire (base-10000 digit vector).
            Self::Numeric
            | Self::Text
            | Self::Varchar
            | Self::Bytea
            | Self::Jsonb
            | Self::BoolArray
            | Self::ByteaArray
            | Self::Int2Array
            | Self::Int4Array
            | Self::TextArray
            | Self::VarcharArray
            | Self::Int8Array
            | Self::Float4Array
            | Self::Float8Array
            | Self::TimestampArray
            | Self::TimestamptzArray
            | Self::DateArray
            | Self::TimeArray
            | Self::IntervalArray
            | Self::NumericArray
            | Self::UuidArray
            | Self::JsonbArray
            | Self::NameArray => -1,
        }
    }

    /// The array `TypeOid` whose elements are `self` (e.g. `Text` → `TextArray`).
    ///
    /// Returns `None` when `self` has no supported array type (including when
    /// `self` is already an array — pglet has no nested arrays).
    #[must_use]
    pub const fn array_of(self) -> Option<Self> {
        Some(match self {
            Self::Bool => Self::BoolArray,
            Self::Bytea => Self::ByteaArray,
            Self::Int2 => Self::Int2Array,
            Self::Int4 => Self::Int4Array,
            Self::Int8 => Self::Int8Array,
            Self::Float4 => Self::Float4Array,
            Self::Float8 => Self::Float8Array,
            Self::Text => Self::TextArray,
            Self::Varchar => Self::VarcharArray,
            Self::Timestamp => Self::TimestampArray,
            Self::Timestamptz => Self::TimestamptzArray,
            Self::Date => Self::DateArray,
            Self::Time => Self::TimeArray,
            Self::Interval => Self::IntervalArray,
            Self::Numeric => Self::NumericArray,
            Self::Uuid => Self::UuidArray,
            Self::Jsonb => Self::JsonbArray,
            Self::Name => Self::NameArray,
            _ => return None,
        })
    }

    /// The element `TypeOid` of an array type (e.g. `TextArray` → `Text`).
    ///
    /// Returns `None` for non-array types.
    #[must_use]
    pub const fn array_element(self) -> Option<Self> {
        Some(match self {
            Self::BoolArray => Self::Bool,
            Self::ByteaArray => Self::Bytea,
            Self::Int2Array => Self::Int2,
            Self::Int4Array => Self::Int4,
            Self::Int8Array => Self::Int8,
            Self::Float4Array => Self::Float4,
            Self::Float8Array => Self::Float8,
            Self::TextArray => Self::Text,
            Self::VarcharArray => Self::Varchar,
            Self::TimestampArray => Self::Timestamp,
            Self::TimestamptzArray => Self::Timestamptz,
            Self::DateArray => Self::Date,
            Self::TimeArray => Self::Time,
            Self::IntervalArray => Self::Interval,
            Self::NumericArray => Self::Numeric,
            Self::UuidArray => Self::Uuid,
            Self::JsonbArray => Self::Jsonb,
            Self::NameArray => Self::Name,
            _ => return None,
        })
    }

    /// True if this is a one-dimensional array type.
    #[must_use]
    pub const fn is_array(self) -> bool {
        self.array_element().is_some()
    }

    /// True if this is a **container** type — a one-dimensional array or JSONB.
    ///
    /// These are exactly the types a GIN inverted index decomposes (and the types
    /// the `@>` containment operator applies to). A B-tree keys them too
    /// (`Value::encode_index_key`); what this predicate decides is which *method* a
    /// non-unique index over a single bare column of one is. Both the DDL
    /// `USING`-method validation and the planner's index-method derivation route
    /// through this one predicate.
    #[must_use]
    pub const fn is_container(self) -> bool {
        matches!(self, Self::Jsonb) || self.is_array()
    }

    /// Convert a Postgres type OID (i32) to `TypeOid` enum.
    ///
    /// Returns None for unrecognized OIDs. This is used when decoding
    /// protocol messages that contain type OIDs.
    ///
    /// Per `PostgreSQL` wire protocol, OID 0 means "unspecified" - the client
    /// wants the server to infer the type.
    #[must_use]
    // the 0 and `_` arms share a `None` body but document distinct meanings (unspecified vs unknown OID)
    #[allow(clippy::match_same_arms)]
    pub const fn from_oid(oid: i32) -> Option<Self> {
        // Note: OID values from PostgreSQL pg_type catalog
        match oid as u32 {
            16 => Some(Self::Bool),          // BOOL
            21 => Some(Self::Int2),          // INT2
            23 => Some(Self::Int4),          // INT4
            20 => Some(Self::Int8),          // INT8
            700 => Some(Self::Float4),       // FLOAT4
            701 => Some(Self::Float8),       // FLOAT8
            25 => Some(Self::Text),          // TEXT
            1043 => Some(Self::Varchar),     // VARCHAR
            17 => Some(Self::Bytea),         // BYTEA
            1114 => Some(Self::Timestamp),   // TIMESTAMP
            1184 => Some(Self::Timestamptz), // TIMESTAMPTZ
            1082 => Some(Self::Date),        // DATE
            1083 => Some(Self::Time),        // TIME
            1186 => Some(Self::Interval),    // INTERVAL
            1700 => Some(Self::Numeric),     // NUMERIC
            3802 => Some(Self::Jsonb),       // JSONB
            2950 => Some(Self::Uuid),        // UUID
            19 => Some(Self::Name),          // NAME
            26 => Some(Self::Oid),           // OID
            18 => Some(Self::Char),          // "char"
            2278 => Some(Self::Void),        // VOID
            // Array types (canonical Postgres array OIDs)
            1000 => Some(Self::BoolArray),
            1001 => Some(Self::ByteaArray),
            1005 => Some(Self::Int2Array),
            1007 => Some(Self::Int4Array),
            1009 => Some(Self::TextArray),
            1015 => Some(Self::VarcharArray),
            1016 => Some(Self::Int8Array),
            1021 => Some(Self::Float4Array),
            1022 => Some(Self::Float8Array),
            1115 => Some(Self::TimestampArray),
            1185 => Some(Self::TimestamptzArray),
            1182 => Some(Self::DateArray),
            1183 => Some(Self::TimeArray),
            1187 => Some(Self::IntervalArray),
            1231 => Some(Self::NumericArray),
            2951 => Some(Self::UuidArray),
            3807 => Some(Self::JsonbArray),
            1003 => Some(Self::NameArray),
            0 => None, // UNSPECIFIED_TYPE_OID - client wants server to infer
            _ => None, // Unknown type OID
        }
    }
}

/// The largest `varchar(n)` / `char(n)` character length — Postgres's `MaxAttrSize`, the
/// bound `anychar_typmodin` refuses above.
///
/// `htup_details.h`'s `10 * 1024 * 1024`, read from the 20devel source clone; the value is
/// confirmed on the 18.6 oracle by the refusal's own text (`cannot exceed 10485760`), and
/// the constant's name and header are the clone's.
///
/// The home is here because the bounds a modifier is **refused** outside are the same
/// fact as [`type_oid_carries_modifier`]'s membership: a type that carries a `(…)` is the
/// type that says which values it admits. (A bound that **clamps** rather than refuses
/// answers to the rounding that reads it instead, which is why
/// [`MAX_TEMPORAL_PRECISION`](super::MAX_TEMPORAL_PRECISION) lives beside that rounding in
/// `types::precision` and not here.) Both the refusal's message
/// ([`Error::CharacterLengthTooLarge`](crate::Error::CharacterLengthTooLarge)) and the
/// comparison (`executor::type_resolution`'s length reader) read this one constant, so
/// the sentence cannot name a number the check does not use.
pub const MAX_CHARACTER_LENGTH: u32 = 10_485_760;

/// The largest `numeric(p[,s])` precision — Postgres's `NUMERIC_MAX_PRECISION`.
///
/// `numeric.h`'s constant (read from the 20devel source clone), the bound
/// `numerictypmodin`'s `make_numeric_typmod_safe` refuses above; the 18.6 oracle confirms
/// the value through its own `must be between 1 and 1000`. The window's other end is `1`,
/// spelled in the message beside this. See [`MAX_CHARACTER_LENGTH`] for why the bounds live here.
pub const MAX_NUMERIC_PRECISION: i32 = 1000;

/// The magnitude of the `numeric(p,s)` **scale** window.
///
/// Postgres's `NUMERIC_MAX_SCALE` and, negated, its `NUMERIC_MIN_SCALE` — `1000` and
/// `(-1000)` in `numeric.h`, read from the 20devel source clone; the 18.6 oracle confirms
/// the window through its own `must be between -1000 and 1000`. A scale may be negative
/// (`numeric(5,-2)` rounds to hundreds), so the window is symmetric where the precision's
/// is not. Deliberately **not** `numeric`'s own private `MAX_DISPLAY_SCALE`: the header
/// keeps `NUMERIC_MAX_DISPLAY_SCALE` as a separate constant of the same value, and the two
/// bound different things.
pub const MAX_NUMERIC_SCALE: i32 = 1000;

/// Whether a type accepts a `(…)` **type modifier** — Postgres's
/// `pg_type.typmodin <> 0`, read as a property of the *type* rather than of the
/// spelling that named it.
///
/// What a carrier then *admits* is the type's too. The three bounds a modifier is
/// **refused** outside are the constants directly above —
/// [`MAX_CHARACTER_LENGTH`], [`MAX_NUMERIC_PRECISION`], [`MAX_NUMERIC_SCALE`] — while the
/// four temporal types' bound **clamps** instead of refusing and therefore lives with the
/// rounding that applies it, [`MAX_TEMPORAL_PRECISION`](super::MAX_TEMPORAL_PRECISION) in
/// `types::precision`. Four bounds, two homes, split by what the bound *does*.
///
/// The fact is 18.6's, measured on it rather than reasoned from the spellings:
/// `bpchar`, `varchar`, `numeric`, `time`, `timetz`, `timestamp`, `timestamptz` and
/// `interval` carry one; `bool`, `bytea`, `"char"`, `date`, `float4`, `float8`,
/// `int2`, `int4`, `int8`, `json`, `jsonb`, `money`, `name`, `oid`, `regclass`,
/// `text`, `uuid`, `xml` and **every enum** (`typtype = 'e'`) carry none. pglet folds
/// `char`/`character` onto [`TypeOid::Varchar`], and both spellings carry, so the
/// single arm is right for the pair.
///
/// **An array type answers at its element's width**, which is 18.6's own shape rather
/// than a convenience: `LookupTypeName` resolves `varchar(3)[]` to `_varchar` *before*
/// `typenameTypeMod` reads the modifier, and `_varchar.typmodin` is
/// `varchartypmodin` — measured, as `_int4.typmodin` is 0. So the array arms below
/// pair with their elements' and a divergence between the two would be a bug in this
/// table, not a difference this engine models.
///
/// Exhaustive (no wildcard) so a new `TypeOid` forces the decision rather than
/// silently joining the "carries none" half, which is the half that **refuses**.
#[must_use]
pub const fn type_oid_carries_modifier(oid: TypeOid) -> bool {
    match oid {
        TypeOid::Varchar
        | TypeOid::VarcharArray
        | TypeOid::Numeric
        | TypeOid::NumericArray
        | TypeOid::Interval
        | TypeOid::IntervalArray
        | TypeOid::Timestamp
        | TypeOid::TimestampArray
        | TypeOid::Timestamptz
        | TypeOid::TimestamptzArray
        | TypeOid::Time
        | TypeOid::TimeArray => true,
        TypeOid::Bool
        | TypeOid::BoolArray
        | TypeOid::Bytea
        | TypeOid::ByteaArray
        | TypeOid::Int2
        | TypeOid::Int2Array
        | TypeOid::Int4
        | TypeOid::Int4Array
        | TypeOid::Int8
        | TypeOid::Int8Array
        | TypeOid::Float4
        | TypeOid::Float4Array
        | TypeOid::Float8
        | TypeOid::Float8Array
        | TypeOid::Text
        | TypeOid::TextArray
        | TypeOid::Date
        | TypeOid::DateArray
        | TypeOid::Jsonb
        | TypeOid::JsonbArray
        | TypeOid::Uuid
        | TypeOid::UuidArray
        | TypeOid::Name
        | TypeOid::NameArray
        | TypeOid::Oid
        | TypeOid::Char
        | TypeOid::Void => false,
    }
}

/// Map a lowercased base type name (no `(...)` modifier, no `[]` suffix) to its
/// `TypeOid`, or `None` if unrecognized.
///
/// The single str→TypeOid table shared by the DDL path and the `::cast` path; each
/// caller applies its own fail-loud/coerce policy.
#[must_use]
pub fn type_oid_from_name(base: &str) -> Option<TypeOid> {
    Some(match base {
        "bool" | "boolean" => TypeOid::Bool,
        "int2" | "smallint" | "serial2" | "smallserial" => TypeOid::Int2,
        "int" | "int4" | "integer" | "serial" | "serial4" => TypeOid::Int4,
        "int8" | "bigint" | "bigserial" | "serial8" => TypeOid::Int8,
        "float4" | "real" => TypeOid::Float4,
        "float8" | "double precision" | "float" => TypeOid::Float8,
        "text" => TypeOid::Text,
        "varchar" | "character varying" | "char" | "character" => TypeOid::Varchar,
        "bytea" => TypeOid::Bytea,
        "timestamp" | "timestamp without time zone" => TypeOid::Timestamp,
        "timestamptz" | "timestamp with time zone" => TypeOid::Timestamptz,
        "date" => TypeOid::Date,
        "time" => TypeOid::Time,
        "interval" => TypeOid::Interval,
        "numeric" | "decimal" => TypeOid::Numeric,
        // `json` is deliberately **not** mapped here, and is not an alias for `jsonb`:
        // 18.6's `json` keeps its input text verbatim — whitespace, key order and
        // duplicate keys, none of which a `jsonb` value can hold — so resolving the
        // spelling to `Jsonb` silently rewrote the value. It answers `42704` as an
        // unrecognized type name does; the parser keeps its own `json` arm for a reason of
        // its own (`parser::type_name`).
        "jsonb" => TypeOid::Jsonb,
        "uuid" => TypeOid::Uuid,
        _ => return None,
    })
}

/// The round-trippable Postgres DDL spelling for a `TypeOid` (the inverse of
/// `type_oid_from_name`), or `None` where the type has no column form.
///
/// Every **column** type has a spelling — `interval` included, since `INTERVAL` is a
/// full column type. The `None`s are the array OIDs plus `name` / `oid` / `"char"` / `void`,
/// which are declaration-only; the arms below spell each out.
///
/// Used by the dump emitter to spell a column's type in a `CREATE TABLE` that
/// re-parses on both stock Postgres and pglet: every name here is one
/// `type_oid_from_name` accepts back (`integer` → `Int4` → `integer`). It differs
/// deliberately from `information_schema`'s `data_type` labels (which render a
/// `Varchar` as `character varying`, a `Timestamptz` as `timestamp with time
/// zone`, and any array as the bare word `ARRAY`): those describe the catalog,
/// these reconstruct DDL. An array type returns `None` — the caller spells it as
/// `<element>[]` from `array_element`.
#[must_use]
pub const fn type_oid_to_sql_name(oid: TypeOid) -> Option<&'static str> {
    Some(match oid {
        TypeOid::Bool => "boolean",
        TypeOid::Int2 => "smallint",
        TypeOid::Int4 => "integer",
        TypeOid::Int8 => "bigint",
        TypeOid::Float4 => "real",
        TypeOid::Float8 => "double precision",
        TypeOid::Text => "text",
        TypeOid::Varchar => "varchar",
        TypeOid::Bytea => "bytea",
        TypeOid::Timestamp => "timestamp",
        TypeOid::Timestamptz => "timestamptz",
        TypeOid::Date => "date",
        TypeOid::Time => "time",
        TypeOid::Interval => "interval",
        TypeOid::Numeric => "numeric",
        TypeOid::Uuid => "uuid",
        TypeOid::Jsonb => "jsonb",
        // No scalar column form: an array spells as `<element>[]` (caller's job
        // via `array_element`).
        TypeOid::IntervalArray
        | TypeOid::BoolArray
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
        | TypeOid::NumericArray
        | TypeOid::UuidArray
        | TypeOid::JsonbArray
        // `name` / `name[]` are declared output types with **no** pglet column form
        // (`type_oid_from_name` maps neither), so a dump must fail loud rather than
        // emit a type that will not re-parse. They are what makes
        // `deparse::unsupported_type`'s guard reachable.
        | TypeOid::Name
        | TypeOid::Oid
        | TypeOid::Char
        | TypeOid::Void
        | TypeOid::NameArray => return None,
    })
}

/// Postgres's `pg_type.typname` — the catalog's own internal spelling.
///
/// `int4`, `varchar`, `timestamptz`, `_text`: what
/// `information_schema.columns.udt_name` reports and what `pg_catalog.pg_type.typname`
/// carries.
///
/// A fourth naming scheme beside [`type_oid_display_name`] (the user-facing
/// `integer` / `character varying`), `type_oid_to_sql_name` (round-trippable DDL, and
/// `None` for an array), and the wasm adapter's upper-case short form. They are separate
/// because their audiences are: this one is a **catalog value** a consumer diffs across
/// backends, so it must be 18.6's byte-for-byte and may not borrow another's spelling.
#[must_use]
pub const fn type_oid_pg_name(oid: TypeOid) -> &'static str {
    match oid {
        TypeOid::Bool => "bool",
        TypeOid::Int2 => "int2",
        TypeOid::Int4 => "int4",
        TypeOid::Int8 => "int8",
        TypeOid::Float4 => "float4",
        TypeOid::Float8 => "float8",
        TypeOid::Text => "text",
        TypeOid::Varchar => "varchar",
        TypeOid::Bytea => "bytea",
        TypeOid::Timestamp => "timestamp",
        TypeOid::Timestamptz => "timestamptz",
        TypeOid::Date => "date",
        TypeOid::Time => "time",
        TypeOid::Interval => "interval",
        TypeOid::Numeric => "numeric",
        TypeOid::Uuid => "uuid",
        TypeOid::Jsonb => "jsonb",
        TypeOid::Name => "name",
        TypeOid::Oid => "oid",
        TypeOid::Char => "char",
        TypeOid::Void => "void",
        TypeOid::BoolArray => "_bool",
        TypeOid::ByteaArray => "_bytea",
        TypeOid::Int2Array => "_int2",
        TypeOid::Int4Array => "_int4",
        TypeOid::Int8Array => "_int8",
        TypeOid::Float4Array => "_float4",
        TypeOid::Float8Array => "_float8",
        TypeOid::TextArray => "_text",
        TypeOid::VarcharArray => "_varchar",
        TypeOid::TimestampArray => "_timestamp",
        TypeOid::TimestamptzArray => "_timestamptz",
        TypeOid::DateArray => "_date",
        TypeOid::TimeArray => "_time",
        TypeOid::IntervalArray => "_interval",
        TypeOid::NumericArray => "_numeric",
        TypeOid::UuidArray => "_uuid",
        TypeOid::JsonbArray => "_jsonb",
        TypeOid::NameArray => "_name",
    }
}

/// Postgres's **display** spelling for a `TypeOid` — what `format_type` puts in a
/// user-facing message (`integer`, `boolean`, `character varying`, `timestamp with
/// time zone`, `integer[]`).
///
/// This is the spelling an error message wants, and it is deliberately *not*
/// `type_oid_to_sql_name` (which reconstructs round-trippable DDL: `varchar`,
/// `timestamptz`, `None` for arrays) nor `pg_type.typname`'s own short form
/// (`int4`, `_text`). All three naming schemes are real Postgres surfaces with
/// different audiences; only this one belongs in `operator does not exist: …` / `function min(…) does not exist`.
#[must_use]
pub const fn type_oid_display_name(oid: TypeOid) -> &'static str {
    match oid {
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
        TypeOid::Interval => "interval",
        TypeOid::Numeric => "numeric",
        TypeOid::Uuid => "uuid",
        TypeOid::Jsonb => "jsonb",
        TypeOid::BoolArray => "boolean[]",
        TypeOid::ByteaArray => "bytea[]",
        TypeOid::Int2Array => "smallint[]",
        TypeOid::Int4Array => "integer[]",
        TypeOid::Int8Array => "bigint[]",
        TypeOid::Float4Array => "real[]",
        TypeOid::Float8Array => "double precision[]",
        TypeOid::TextArray => "text[]",
        TypeOid::VarcharArray => "character varying[]",
        TypeOid::TimestampArray => "timestamp without time zone[]",
        TypeOid::TimestamptzArray => "timestamp with time zone[]",
        TypeOid::DateArray => "date[]",
        TypeOid::TimeArray => "time without time zone[]",
        TypeOid::IntervalArray => "interval[]",
        TypeOid::NumericArray => "numeric[]",
        TypeOid::UuidArray => "uuid[]",
        TypeOid::JsonbArray => "jsonb[]",
        TypeOid::Name => "name",
        TypeOid::Oid => "oid",
        TypeOid::Char => "\"char\"",
        TypeOid::Void => "void",
        TypeOid::NameArray => "name[]",
    }
}

/// The display spelling for a [`Value::type_oid`](crate::types::Value::type_oid)
/// result: [`type_oid_display_name`], or Postgres's `unknown` for the typeless
/// NULL / untyped-literal case.
#[must_use]
pub fn opt_type_oid_display_name(oid: Option<TypeOid>) -> &'static str {
    oid.map_or("unknown", type_oid_display_name)
}

#[cfg(test)]
mod carries_modifier_tests {
    use super::{TypeOid, type_oid_carries_modifier, type_oid_from_name};

    /// The membership list, measured on 18.6 as
    /// `SELECT typname, typmodin <> 0 FROM pg_type`. Pinned through the spellings a
    /// user writes rather than through `TypeOid` directly, so a wrong arm shows up as
    /// the SQL that would take it — a few of those spellings (`character varying`,
    /// `double precision`, `float`, the `serial` family) are grammar productions with
    /// no `pg_type` row of their own, and the assertion is about the type each
    /// resolves to.
    #[test]
    fn membership_matches_pg_type_typmodin() {
        for name in [
            "varchar",
            "character varying",
            "char",
            "character",
            "numeric",
            "decimal",
            "interval",
            "timestamp",
            "timestamptz",
            "time",
        ] {
            let oid = type_oid_from_name(name).unwrap_or_else(|| panic!("{name} resolves"));
            assert!(
                type_oid_carries_modifier(oid),
                "{name} carries a modifier on 18.6"
            );
        }
        for name in [
            "bool",
            "boolean",
            "int2",
            "smallint",
            "int",
            "int4",
            "integer",
            "int8",
            "bigint",
            "float4",
            "real",
            "float8",
            "double precision",
            "float",
            "text",
            "bytea",
            "date",
            "jsonb",
            "uuid",
            "serial",
            "bigserial",
            "smallserial",
        ] {
            let oid = type_oid_from_name(name).unwrap_or_else(|| panic!("{name} resolves"));
            assert!(
                !type_oid_carries_modifier(oid),
                "{name} carries none on 18.6"
            );
        }
        // The declaration-only types, which have no `type_oid_from_name` entry.
        for oid in [
            TypeOid::Name,
            TypeOid::Oid,
            TypeOid::Char,
            TypeOid::Void,
            TypeOid::NameArray,
        ] {
            assert!(
                !type_oid_carries_modifier(oid),
                "{oid:?} carries none on 18.6"
            );
        }
    }

    /// An array type answers at its element's width, because 18.6's `_x.typmodin` is
    /// `x`'s (`_varchar` → `varchartypmodin`, `_int4` → none, measured). A pair that
    /// disagreed would let `varchar(3)[]` be refused or `int4(3)[]` be accepted.
    #[test]
    fn an_array_answers_at_its_elements_width() {
        for oid in [
            TypeOid::Bool,
            TypeOid::Bytea,
            TypeOid::Int2,
            TypeOid::Int4,
            TypeOid::Int8,
            TypeOid::Float4,
            TypeOid::Float8,
            TypeOid::Text,
            TypeOid::Varchar,
            TypeOid::Timestamp,
            TypeOid::Timestamptz,
            TypeOid::Date,
            TypeOid::Time,
            TypeOid::Interval,
            TypeOid::Numeric,
            TypeOid::Uuid,
            TypeOid::Jsonb,
            TypeOid::Name,
        ] {
            let arr = oid
                .array_of()
                .unwrap_or_else(|| panic!("{oid:?} has an array OID"));
            assert_eq!(
                type_oid_carries_modifier(arr),
                type_oid_carries_modifier(oid),
                "{arr:?} must answer as {oid:?} does",
            );
        }
    }
}
