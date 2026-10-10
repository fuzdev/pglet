use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};

use super::text::{quote_array_element, value_to_text, value_to_text_with_oid};
use super::{PgDate, PgInterval, PgNumeric, PgTimestamp, TypeOid};

/// Column description in query results.
///
/// Beyond the name and type, this carries the wire-`RowDescription` metadata a
/// driver reads: the Postgres-encoded **type modifier** (`atttypmod` — a
/// `numeric(p,s)`'s precision/scale, a `varchar(n)`'s length, or `-1` for none;
/// see [`Self::type_modifier`]) and, for a plain base-table column, its source
/// **table OID** + 1-based **attnum** (both `0` for a computed/expression column).
/// A freshly-built descriptor defaults them to "no metadata" (`-1`/`0`/`0`); the
/// base-table converter (`executor::table_column_descs`) fills them from the
/// catalog, and projection propagates them for a passthrough column.
///
/// It also carries the one piece of declared-type information the `TypeOid` cannot
/// hold: an enum column's **type name** ([`Self::enum_type`]), which is what says
/// the column's order is its members' positions rather than its labels' bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnDesc {
    pub name: String,
    pub type_oid: TypeOid,
    /// Postgres's `atttypmod` wire encoding: `((precision << 16) | (scale &
    /// 0x7ff)) + 4` for `numeric(p,s)`, `length + 4` for `varchar(n)`/`char(n)`,
    /// and `-1` when the type carries no modifier. Read by a driver's
    /// `getPrecision`/`getScale` and any width-deriving client.
    pub type_modifier: i32,
    /// The OID of the table a plain base-table column originates from (`0` for a
    /// computed/expression column, a join-merged column, or a virtual table).
    pub table_oid: i32,
    /// The 1-based attribute number (`attnum`) of the source column within
    /// [`Self::table_oid`] (`0` when `table_oid` is `0`).
    pub column_id: i16,
    /// The user `ENUM` type this column was declared with (the lowercased catalog
    /// key into `Schema.enums`), or `None` for every other column.
    ///
    /// An enum value is a `Value::Text` and its `type_oid` is `Text`, so nothing in
    /// either says the column's order is its members' **positions** rather than its
    /// labels' bytes. This is what says it, and it rides here for the same reason
    /// [`Self::type_modifier`] does: a projecting node propagates the whole
    /// descriptor for a passthrough column, so a derived table, a CTE and a join
    /// carry it without a second derivation.
    #[serde(default)]
    pub enum_type: Option<String>,
    /// Whether [`Self::type_oid`] is **certain**: the planner vouches that its declared
    /// type for the column is the type the column's values have and the type PostgreSQL's
    /// parse analysis gives it, rather than a walk's fallback or a guess.
    ///
    /// A planner fact, written where the planner derives a relation's columns
    /// (`Plan::output_columns`): a base table's column is certain, a column an expression
    /// defines is certain where the coercion pass typed that expression as written —
    /// reading a scalar subquery in it by its sub-plan's own flag, never by a cast the
    /// pass itself wrote — and a node that passes a column through passes this with it.
    /// The coercion pass reads it off a derived relation's columns and off a scalar
    /// subquery's sub-plan, and the planner's own derivation reads it off the relations
    /// a node combines. It is not wire metadata, and a descriptor the executor builds
    /// leaves it `false`.
    #[serde(default)]
    pub type_certain: bool,
}

impl ColumnDesc {
    /// A result column with no wire metadata — type modifier `-1`, no source
    /// table/attnum, no enum type. The common case for a computed or freshly-typed
    /// column.
    pub fn new(name: impl Into<String>, type_oid: TypeOid) -> Self {
        Self {
            name: name.into(),
            type_oid,
            type_modifier: -1,
            table_oid: 0,
            column_id: 0,
            enum_type: None,
            type_certain: false,
        }
    }
}

/// Whether a statement returns a **result set** at all, and — when it does — the
/// columns of that result set.
///
/// The distinction a bare `Vec<ColumnDesc>` cannot make: *a result set of zero
/// columns is not the absence of a result set.* Postgres holds the same two facts
/// separately (`resultDesc`/`tupDesc` is a null pointer for a statement that returns
/// nothing, and a valid zero-attribute tuple descriptor for `SELECT *` over a
/// relation whose last column was dropped), and every wire reply that has to tell
/// them apart reads this rather than a column count:
///
/// - the simple-`Query` reply sends a `RowDescription` — `fields=0` and all — plus
///   the `DataRow`s for a result set, and neither for a statement without one;
/// - `Describe` answers `NoData` for [`Self::NoResultSet`] and a `RowDescription`
///   otherwise;
/// - the aborted-block `Describe` gate refuses exactly the statements that return a
///   result set, which is Postgres's `psrc->resultDesc != NULL` and **not** "declares
///   at least one column" (18.6 refuses a zero-column `SELECT *` there with `25P02`
///   and describes a `RETURNING`-less `INSERT`, measured).
///
/// Reading the column count in place of this is what `EDGES.md` row 59 recorded: a
/// zero-column `SELECT` lost its rows on the simple leg and was described `NoData` on
/// the extended one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResultShape {
    /// The statement returns no result set: DDL, transaction control, `SET`, `COPY`,
    /// and DML with no `RETURNING` clause.
    ///
    /// **It carries a second meaning on the `Describe` path, and the overload is
    /// load-bearing:** the server's `describe_result_shape` also answers this when there
    /// is no plan to ask — a re-plan that failed after a DDL, a text that yields no
    /// statement at all, and the statements serviced outside the planner
    /// (`runs_outside_the_planner`'s `SET`, `RESET`, `DEALLOCATE` and `DISCARD`; its `SHOW`,
    /// which returns a result set, is answered ahead of all three by
    /// `catalog::show_result_shape`). Three
    /// **situations, not three branches**: the first and last are one
    /// `parse_single`-returns-`Err` branch, which is why row 29 records them together —
    /// see `EDGES.md` row 29 and `docs/declared_types.md`.
    NoResultSet,
    /// The statement returns a result set of these columns. The list **may be empty**
    /// — that is a result set of zero columns, not the absence of one.
    Columns(Vec<ColumnDesc>),
}

impl ResultShape {
    /// Whether the statement returns a result set — Postgres's `resultDesc != NULL`.
    ///
    /// True for a zero-column result set; the column count says nothing here.
    #[must_use]
    pub const fn is_result_set(&self) -> bool {
        matches!(self, Self::Columns(_))
    }

    /// The result set's columns, or an empty slice when there is no result set.
    ///
    /// For the readers that only need the list — expression typing, value encoding,
    /// a row's field names. A reader deciding *whether to describe a result set* must
    /// use [`Self::is_result_set`] instead: this slice is empty for both cases.
    #[must_use]
    pub fn columns(&self) -> &[ColumnDesc] {
        match self {
            Self::NoResultSet => &[],
            Self::Columns(columns) => columns,
        }
    }

    /// The result set's columns by value, empty when there is no result set. The
    /// lossy direction of [`Self::columns`], for callers that want an owned list.
    #[must_use]
    pub fn into_columns(self) -> Vec<ColumnDesc> {
        match self {
            Self::NoResultSet => Vec::new(),
            Self::Columns(columns) => columns,
        }
    }

    /// Rewrite the columns in place, keeping the result-set-ness. A no-op for
    /// [`Self::NoResultSet`], which has no list to rewrite.
    #[must_use]
    pub fn map_columns(self, f: impl FnOnce(Vec<ColumnDesc>) -> Vec<ColumnDesc>) -> Self {
        match self {
            Self::NoResultSet => Self::NoResultSet,
            Self::Columns(columns) => Self::Columns(f(columns)),
        }
    }
}

/// Runtime value representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int2(i16),
    Int4(i32),
    Int8(i64),
    Float4(f32),
    Float8(f64),
    Text(String),
    Bytea(Vec<u8>),
    /// A `timestamp` / `timestamptz` instant, held as UTC (pglet has no distinct
    /// timestamptz value — only the column's `TypeOid` carries the tz/no-tz bit).
    ///
    /// **Microsecond-exact by invariant.** Postgres's `Timestamp` is an `i64` of
    /// microseconds, so sub-microsecond precision is not merely unrendered here —
    /// it is unrepresentable *there*, and every one of pglet's three encoders
    /// (text, binary, index key) divides it away. A `Value::Timestamp` carrying
    /// nanoseconds therefore compares by a precision nothing can display: three
    /// such values print one string and `SELECT DISTINCT` answers three, and a
    /// `UNIQUE` timestamp column raises `23505` for two values `WHERE a = b` calls
    /// different. The invariant is held where sub-microsecond precision can *enter*:
    /// `text::parse_frac_micros` for text input, [`truncate_timestamp_to_micros`] for
    /// a clock read, and `normalize_value_domain` / `normalize_stored_value_domain` for a
    /// value that arrives already built — one a caller binds through
    /// `Database::execute_bound`, and one a write stores. After those, arithmetic
    /// preserves it (an interval is itself microsecond-based) and the binary decode is
    /// microsecond-native.
    ///
    /// **An already-built value is truncated toward the earlier instant**, the clock
    /// read's rule and not text input's rounding: 18.6's binary format has no field for
    /// the nanoseconds, so it has no rule of its own at this door; the value a caller most
    /// plausibly binds with nanoseconds is a clock read (`OffsetDateTime::now_utc()`),
    /// which truncation makes the instant `now()` would answer; and the text renderer's
    /// microsecond field already reads an instant that way, so what a value displays does
    /// not move. The persisted form is the binary format's microsecond count
    /// ([`PgTimestamp::to_pg_micros`]), so a value holding nanoseconds would answer
    /// differently after a store is reopened than before — which is what the door's
    /// truncation prevents.
    ///
    /// **It is a convention across those call sites, not a property of the type.** A
    /// value handed to a `Storage` directly reaches none of them, and
    /// [`PgTimestamp::to_pg_micros`] divides nanoseconds away toward the Unix epoch
    /// rather than toward the earlier instant. Making the invariant structural means a
    /// newtype with a private field, which reaches every construction and pattern in the
    /// workspace; recorded rather than taken.
    ///
    /// `infinity` and `-infinity` are the [`PgTimestamp`] variants beside the instant.
    Timestamp(PgTimestamp),
    Jsonb(JsonbValue),
    Uuid([u8; 16]),
    /// A one-dimensional array (e.g. `text[]`, `uuid[]`). Carries its element
    /// type so empty arrays and `unnest` results stay typed; see `ArrayValue`.
    Array(ArrayValue),
    /// An `INTERVAL` — Postgres's 3-field `(months, days, micros)` span (see
    /// [`PgInterval`]). A full column type: storable, projectable, wire-encoded
    /// (16-byte binary + `postgres`-style text), and B-tree-indexable on the
    /// comparison estimate (`30 days` collides `1 mon`). Produced by `INTERVAL '…'`
    /// literals, `timestamp - timestamp`, and `time - time`; consumed by comparison
    /// and calendar arithmetic (`timestamp ± interval`, `interval ± interval`,
    /// `interval ×/÷ number`).
    Interval(PgInterval),
    /// A `DATE` — a calendar date with no time-of-day, distinct from `Timestamp`.
    /// Rendered as `YYYY-MM-DD`, wired as an `i32` day count from the Postgres
    /// epoch (2000-01-01), and B-tree-indexable (chronological order). `infinity` and
    /// `-infinity` are the [`PgDate`] variants beside the calendar day.
    Date(PgDate),
    /// A `NUMERIC`/`DECIMAL` — an in-house arbitrary-precision exact decimal (see
    /// [`PgNumeric`]). Appended as the last variant so existing postcard-encoded
    /// `Value`s keep their tags.
    Numeric(PgNumeric),
    /// A `TIME` (time without time zone) — microseconds since midnight in the
    /// inclusive range `[0, 86_400_000_000]` (a raw `i64`, not `time::Time`, so the
    /// `24:00:00` endpoint is representable). Rendered as `HH:MM:SS[.ffffff]`, wired
    /// as a bare big-endian `i64`, and B-tree-indexable (chronological order).
    /// Appended after `Numeric` so existing postcard-encoded `Value`s keep their tags.
    Time(i64),
}

/// Truncate an instant to microsecond resolution — Postgres's clock read.
///
/// The **`now()`** half of the [`Value::Timestamp`] microsecond invariant, and
/// deliberately not the text readers' rounding: `GetCurrentTimestamp` reads
/// `gettimeofday`, whose `tv_usec` never saw the nanoseconds a `clock_gettime`
/// read carries, so discarding them reproduces the value Postgres would have
/// observed. Rounding would also let `now()` answer an instant fractionally in
/// the future. Two rules because Postgres has two, not one rule applied twice.
#[must_use]
pub fn truncate_timestamp_to_micros(dt: OffsetDateTime) -> OffsetDateTime {
    let nanos = dt.unix_timestamp_nanos();
    OffsetDateTime::from_unix_timestamp_nanos(nanos.div_euclid(1_000) * 1_000).unwrap_or(dt)
}

/// A one-dimensional SQL array value.
///
/// pglet supports one-dimensional arrays of a scalar element type. The element
/// type is carried explicitly (not derived from the elements) so empty arrays
/// keep their type; the array's own `TypeOid` is `element_type.array_of()`.
/// Elements are stored as their scalar `Value`s, with `Value::Null` for a NULL
/// element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrayValue {
    /// The element `TypeOid` (e.g. `Text` for a `text[]`).
    ///
    /// **Not enforced here.** The field is plain and `new` is a `const fn` that takes
    /// whatever it is handed, so "one-dimensional" is an invariant the *constructing*
    /// code owns, not one this type upholds. The engine's array constructor rejects an
    /// array-typed element loud (`0A000`, both at plan time and at eval); a nested
    /// value reaching here would make [`Value::type_oid`] report `None` — NULL-like on
    /// the wire — which is why the rejection is up front rather than defensive.
    pub element_type: TypeOid,
    /// Element values in order; `Value::Null` marks a NULL element.
    pub elements: Vec<Value>,
}

impl ArrayValue {
    /// Construct an array value from an element type and its elements.
    #[must_use]
    pub const fn new(element_type: TypeOid, elements: Vec<Value>) -> Self {
        Self {
            element_type,
            elements,
        }
    }

    /// Render as a Postgres array text literal (`{a,b,NULL}`).
    ///
    /// The canonical array text format, shared by the `array::text` cast and the
    /// text wire format. Elements are quoted when ambiguous (empty, the word
    /// `NULL`, or containing braces/commas/quotes/backslashes/whitespace); a NULL
    /// element renders as unquoted `NULL`.
    #[must_use]
    pub fn to_text_format(&self) -> String {
        self.render_text_format(value_to_text)
    }

    /// Render as [`Self::to_text_format`] does, each element rendered as the type
    /// `element` — the declared element type where a caller holds one. The value's own
    /// `element_type` does not say `timestamptz` reliably: `ARRAY[timestamptz '…']`
    /// builds its value from `Value::Timestamp` elements, which carry no tz bit, and a
    /// `timestamptz` element's text needs its `+00` (`value_to_text_with_oid`).
    #[must_use]
    pub fn to_text_format_as(&self, element: TypeOid) -> String {
        self.render_text_format(|v| value_to_text_with_oid(v, element))
    }

    fn render_text_format(&self, render: impl Fn(&Value) -> Option<String>) -> String {
        let mut out = String::from("{");
        for (i, elem) in self.elements.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            match render(elem) {
                None => out.push_str("NULL"),
                Some(s) => out.push_str(&quote_array_element(&s)),
            }
        }
        out.push('}');
        out
    }
}

/// Minimal JSONB representation.
///
/// A number is stored as the exact [`PgNumeric`] (not `f64`), matching Postgres —
/// `jsonb_in` runs the JSON number token through `numeric_in` and `jsonb_out`
/// renders it with `numeric_out`, so a 30-digit integer or a high-scale decimal
/// round-trips without loss (an `f64` would silently round `123…890` above 2^53).
/// The variant order is unchanged (only `Number`'s payload type changed), so other
/// variants keep their postcard tags within a store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JsonbValue {
    Null,
    Bool(bool),
    Number(PgNumeric),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl JsonbValue {
    /// Rewrite into Postgres's **canonical** jsonb form: recursively, every
    /// object's keys are deduplicated (**last** occurrence's value wins) and then
    /// **sorted by key length first, then bytewise**.
    ///
    /// That ordering is not lexicographic — it is Postgres's
    /// `lengthCompareJsonbString` (`jsonb_util.c`), which compares lengths before
    /// contents, so `{"b":1,"aa":2,"c":3}` stores (and renders) as
    /// `{"b": 1, "c": 3, "aa": 2}`. Array element order is *data* and is preserved.
    ///
    /// Canonicalizing at every construction site is what makes key order
    /// **semantically insignificant**, as it is in Postgres: `jsonb_in` runs
    /// `uniqueifyJsonbObject` on input, so by the time any comparison happens both
    /// operands are already sorted. pglet's structural equality and its
    /// `jsonb_compare` B-tree order both walk objects in stored order, so without
    /// this `'{"a":1,"b":2}' = '{"b":2,"a":1}'` was **false** — a silently wrong
    /// row identity that reached `=`, `DISTINCT`, `GROUP BY`, and join dedup.
    ///
    /// Sort-then-dedup is `uniqueifyJsonbObject`'s own algorithm, and the reason it
    /// is spelled that way rather than as a dedup pass followed by a sort: the
    /// comparator breaks an equal-key tie by **descending** insertion order
    /// (`lengthCompareJsonbPair`), so the last occurrence of a duplicate leads its
    /// run and "keep the first of each run" *is* last-wins. Order-then-scan is also
    /// what keeps this `O(n log n)`; a linear-scan dedup is quadratic in the key
    /// count, which a wide document (thousands of keys in one object) feels.
    #[must_use]
    pub fn canonical(self) -> Self {
        match self {
            // Element order is significant; only recurse.
            Self::Array(items) => Self::Array(items.into_iter().map(Self::canonical).collect()),
            Self::Object(pairs) => {
                let mut sorted: Vec<(usize, String, Self)> = pairs
                    .into_iter()
                    .enumerate()
                    .map(|(i, (k, v))| (i, k, v.canonical()))
                    .collect();
                sorted.sort_by(|(ia, a, _), (ib, b, _)| {
                    jsonb_key_order(a, b).then_with(|| ib.cmp(ia))
                });
                let mut out: Vec<(String, Self)> = Vec::with_capacity(sorted.len());
                for (_, key, value) in sorted {
                    // First of each equal-key run wins; the tie-break above put the
                    // latest occurrence there.
                    if out.last().is_some_and(|(k, _)| *k == key) {
                        continue;
                    }
                    out.push((key, value));
                }
                Self::Object(out)
            }
            scalar => scalar,
        }
    }

    /// Whether this is a JSON scalar (non-container).
    ///
    /// The distinction is load-bearing for containment: Postgres wraps a top-level
    /// scalar in a one-element "raw scalar" pseudo-array, and several `@>` rules
    /// turn on whether an operand is one.
    #[must_use]
    pub const fn is_scalar(&self) -> bool {
        matches!(
            self,
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_)
        )
    }

    /// Serialize to JSON string format for wire protocol and display.
    pub fn to_json_string(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            // Exact Postgres `numeric_out` rendering (never scientific notation) —
            // `1e2` renders `100`, `1.0`/`1.00` keep their trailing zeros.
            Self::Number(n) => n.to_display_string(),
            Self::String(s) => format!("\"{}\"", escape_json_string(s)),
            Self::Array(arr) => {
                let items: Vec<String> = arr.iter().map(Self::to_json_string).collect();
                format!("[{}]", items.join(", "))
            }
            Self::Object(pairs) => {
                let items: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("\"{}\": {}", escape_json_string(k), v.to_json_string()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
        }
    }
}

/// Postgres's jsonb object-key order: **length first, then bytewise**.
///
/// `lengthCompareJsonbString` (`jsonb_util.c`) compares lengths before contents,
/// so this is deliberately *not* `str::cmp` — `"c"` sorts before `"aa"`. Rust's
/// `str` ordering is bytewise over the UTF-8 encoding, the same comparison
/// `memcmp` performs there, so the tie-break is byte-identical to Postgres's.
fn jsonb_key_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Escape a string for JSON text output, matching Postgres's `escape_json`:
/// `"` and `\` are backslash-escaped, the C0 control characters use their short
/// escapes (`\b \f \n \r \t`) or a `\u00XX` sequence, and every other character
/// (including `/` and non-ASCII) is emitted verbatim. The inverse is the
/// `unescape_json_string` decoder in `json.rs`; without this a control char in a
/// jsonb string rendered as a raw byte — invalid JSON on the wire (a binary
/// client's `JSON.parse` rejects it) and a `::text` divergence from Postgres.
///
/// **A key and a string value go through this one predicate.** 18.6 hands both to
/// `jsonb_put_escaped_value`, so a key holding a `"`, a `\` or a control character is
/// written with the same spelling a value holding it takes; rendering a key raw made
/// one key's text another key's spelling, and made a document holding a quote in a key
/// invalid JSON that neither engine could read back.
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

impl Value {
    /// A finite `date`.
    #[must_use]
    pub const fn date(d: Date) -> Self {
        Self::Date(super::Infinite::Finite(d))
    }

    /// A finite `timestamp` / `timestamptz`.
    #[must_use]
    pub const fn timestamp(ts: OffsetDateTime) -> Self {
        Self::Timestamp(super::Infinite::Finite(ts))
    }

    /// Returns the Postgres type OID for this value, or `None` for null (and for
    /// an array whose element type has no supported array OID).
    pub const fn type_oid(&self) -> Option<TypeOid> {
        match self {
            Self::Null => None,
            Self::Bool(_) => Some(TypeOid::Bool),
            Self::Int2(_) => Some(TypeOid::Int2),
            Self::Int4(_) => Some(TypeOid::Int4),
            Self::Int8(_) => Some(TypeOid::Int8),
            Self::Float4(_) => Some(TypeOid::Float4),
            Self::Float8(_) => Some(TypeOid::Float8),
            Self::Text(_) => Some(TypeOid::Text),
            Self::Bytea(_) => Some(TypeOid::Bytea),
            Self::Timestamp(_) => Some(TypeOid::Timestamp),
            Self::Jsonb(_) => Some(TypeOid::Jsonb),
            Self::Uuid(_) => Some(TypeOid::Uuid),
            Self::Interval(_) => Some(TypeOid::Interval),
            Self::Date(_) => Some(TypeOid::Date),
            Self::Numeric(_) => Some(TypeOid::Numeric),
            Self::Time(_) => Some(TypeOid::Time),
            // An element type with no array OID reports `None` here — i.e. NULL-like
            // on the wire. Two ways to get one: a future enum array (enum values are
            // `Text` + a catalog name), and a **nested** array, whose element type is
            // itself an array (`array_of` has no second dimension to return). The
            // engine rejects the nested case loud before it can be constructed
            // (`0A000`, see `build_array_value`); give enum arrays a real array OID —
            // or their own loud failure — when they land.
            Self::Array(arr) => arr.element_type.array_of(),
        }
    }

    /// Returns true if this value is null.
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Control chars in a jsonb string must render as valid JSON escapes (PG's
    // `escape_json`), not raw bytes — otherwise the wire payload is invalid JSON.
    #[test]
    fn jsonb_string_escapes_control_chars() {
        let v = JsonbValue::Object(vec![(
            "s".to_string(),
            JsonbValue::String("a\tb\nc\r\"d\\e\u{01}f".to_string()),
        )]);
        assert_eq!(v.to_json_string(), r#"{"s": "a\tb\nc\r\"d\\e\u0001f"}"#);
    }

    // A key goes through the same predicate as a string value -- 18.6 hands both to
    // `jsonb_put_escaped_value`. The expectations are 18.6's own text, measured:
    // `'{"a\"b": 1}'::jsonb::text` is `{"a\"b": 1}`, `'{"a\\b": 1}'` is `{"a\\b": 1}`,
    // `'{"a\tb": 1}'` is `{"a\tb": 1}` and `'{"a\u0001b": 1}'` is `{"a\u0001b": 1}`.
    // Rendered raw instead, the first is not valid JSON and the second is another key.
    #[test]
    fn jsonb_object_key_escapes_where_a_string_value_escapes() {
        let key = |k: &str| JsonbValue::Object(vec![(k.to_string(), JsonbValue::Null)]);
        assert_eq!(key("a\"b").to_json_string(), r#"{"a\"b": null}"#);
        assert_eq!(key("a\\b").to_json_string(), r#"{"a\\b": null}"#);
        assert_eq!(key("a\tb").to_json_string(), r#"{"a\tb": null}"#);
        assert_eq!(key("a\u{01}b").to_json_string(), r#"{"a\u0001b": null}"#);
        // The accept side: `/` and non-ASCII are verbatim in a key too.
        assert_eq!(key("a/éb").to_json_string(), r#"{"a/éb": null}"#);
    }

    #[test]
    fn jsonb_string_passes_non_control_chars_verbatim() {
        // `/` and non-ASCII are emitted raw (PG does not escape them).
        let v = JsonbValue::String("a/b éz 😀".to_string());
        assert_eq!(v.to_json_string(), r#""a/b éz 😀""#);
    }

    fn obj(pairs: &[(&str, i64)]) -> JsonbValue {
        JsonbValue::Object(
            pairs
                .iter()
                .map(|(k, v)| {
                    (
                        (*k).to_string(),
                        JsonbValue::Number(PgNumeric::parse(&v.to_string()).expect("numeric")),
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn canonical_sorts_keys_by_length_then_bytes() {
        // Postgres's `lengthCompareJsonbString`, not lexicographic: every
        // single-char key precedes every two-char key, whatever the bytes.
        assert_eq!(
            obj(&[("aa", 2), ("c", 3), ("b", 1)]).canonical(),
            obj(&[("b", 1), ("c", 3), ("aa", 2)])
        );
        // A lexicographic sort would put "aa" first and this assertion is what
        // separates the two rules.
        assert_ne!(
            obj(&[("aa", 2), ("b", 1)]).canonical(),
            obj(&[("aa", 2), ("b", 1)])
        );
    }

    #[test]
    fn canonical_dedups_last_wins_and_recurses() {
        // Last occurrence's value survives (`uniqueifyJsonbObject` sorts equal keys
        // by descending insertion order and keeps the first of each run).
        assert_eq!(obj(&[("a", 1), ("a", 2)]).canonical(), obj(&[("a", 2)]));
        // Three occurrences, interleaved with other keys, pins that the sort's
        // descending-index tie-break really selects the *last* rather than merely
        // "not the first" — the two are indistinguishable at two occurrences, and a
        // stable ascending tie-break would pass that case while failing this one.
        assert_eq!(
            obj(&[("k", 1), ("b", 8), ("k", 2), ("a", 7), ("k", 3)]).canonical(),
            obj(&[("a", 7), ("b", 8), ("k", 3)])
        );

        // Nested objects are canonicalized; array *element* order is data and is
        // preserved, though each element is canonicalized in place.
        let nested = JsonbValue::Array(vec![
            obj(&[("bb", 1), ("a", 2)]),
            obj(&[("z", 9), ("y", 8)]),
        ]);
        assert_eq!(
            nested.canonical(),
            JsonbValue::Array(vec![
                obj(&[("a", 2), ("bb", 1)]),
                obj(&[("y", 8), ("z", 9)]),
            ])
        );
    }

    #[test]
    fn canonical_makes_key_order_insignificant_for_equality() {
        // The property the whole change exists for: two spellings of one value are
        // the same value. `JsonbValue`'s derived `PartialEq` is positional, so this
        // holds only because both sides are canonicalized on construction.
        assert_eq!(
            obj(&[("a", 1), ("b", 2)]).canonical(),
            obj(&[("b", 2), ("a", 1)]).canonical()
        );
    }
}

#[cfg(test)]
mod microsecond_boundary_tests {
    use super::truncate_timestamp_to_micros;
    use time::OffsetDateTime;

    fn at_nanos(nanos: i128) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp_nanos(nanos).expect("in range")
    }

    /// Truncation floors, on both signs — `gettimeofday`'s `tv_usec` is `0..999_999`
    /// beside a negative `tv_sec`, so a pre-epoch clock read floors there too. A
    /// toward-zero truncation would answer `-1_000` for the last case and put the
    /// instant one microsecond in the future.
    #[test]
    fn truncation_floors_on_both_signs() {
        for (nanos, expected) in [(1_999, 1_000), (999, 0), (-1, -1_000), (-1_001, -2_000)] {
            assert_eq!(
                truncate_timestamp_to_micros(at_nanos(nanos)).unix_timestamp_nanos(),
                expected,
                "{nanos} ns"
            );
        }
    }
}
