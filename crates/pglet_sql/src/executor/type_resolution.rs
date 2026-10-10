//! DDL type resolution: mapping SQL type names (and user `ENUM`s) to storage
//! `TypeOid`s.
//!
//! **A canonical type name decomposes once — base, modifier, array suffix — and each
//! of the three is then decided by the *type*, never by the spelling.**
//! [`decompose_type_name`] is that one split, and every reader below takes its parts
//! rather than re-cutting the string: whether a `(…)` is legal is
//! `type_oid_carries_modifier`'s answer about the resolved type, and a `[]` makes the
//! modifier the **element's** rather than a reason to lose either the modifier or the
//! array. `docs/coercion.md` § A canonical type name decomposes once states the rule
//! in full.
//!
//! **A modifier the type refuses is a refusal, never a modifier dropped**, which makes
//! the `(…)` a *value the type validates* and not a hint it may ignore. Two **gates**
//! live here, in `typenameTypeMod`'s own order: [`reject_type_modifier`]'s `42601` for a
//! type that carries none, then the carrier's own reader —
//! [`parse_length_typmod`]'s `22023`s for the length's two bounds and
//! [`parse_numeric_typmod`]'s for the precision, the scale and the arity, each preceded
//! by the `int4` coercion of every component ([`typmod_component_to_int4`]), whose own
//! `22003`/`22P02` are counted with neither reader anywhere in this file.
//! [`parse_precision_typmod`] is the one that refuses nothing, because a temporal `(p)`
//! above `6` is **clamped**.
//!
//! **One thing here is decided by the *position* as well**: the fixed-length default a
//! parenless `char` takes. That is the reason `gram.y`'s `ConstCharacter` comment gives
//! for clearing what `Typename` installs — `ConstTypename` differs from `Typename` in
//! much more than this, and this is the part pglet carries — and [`TypeNamePosition`]
//! is where it lives, with [`resolve_column_type`] and [`resolve_constant_type`] the
//! two entries onto the one [`resolve_type_name`], differing in that default and in
//! nothing else. **Parenless is the predicate**, and now trivially so: a spelled `(n)`
//! either resolves to a length or takes the statement down, so there is no third state
//! for the default to be collected into.
//! `docs/parser.md` § A typed constant is a cast.

use std::num::IntErrorKind;

use crate::Result;
use crate::catalog::{NumericTypmod, Schema};
use crate::planner::Typmod;
use crate::types::{
    MAX_CHARACTER_LENGTH, MAX_NUMERIC_PRECISION, MAX_NUMERIC_SCALE, TypeOid,
    type_oid_carries_modifier,
};
use pglet_core::Error;

/// The three parts of a canonical type name, cut apart exactly once.
///
/// Produced by [`decompose_type_name`]; the invariant this module exists to hold is
/// that nothing else splits the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TypeNameParts<'a> {
    /// The bare type name — no modifier, no `[]` (`varchar`, `int4`,
    /// `character varying`, an enum name).
    base: &'a str,
    /// The text between the parens (`3`, `4,1`, `5,-2`), if the spelling carried a
    /// `(…)` at all. `Some("")` is possible only from a malformed name; the
    /// distinction that matters to the gate is `Some` vs `None`.
    modifier: Option<&'a str>,
    /// Whether the spelling carried a `[]` suffix.
    is_array: bool,
}

/// Split a canonical type name into base, modifier and array suffix.
///
/// **The `[]` comes off first.** `parser::parse_type_name` appends the suffix to a
/// scalar name that may already carry its modifier, so the canonical spelling is
/// `varchar(3)[]` — cutting on `(` first left the `[]` inside the discarded tail, and
/// the name then resolved to the *scalar* element type with the cap lost as well
/// (`EDGES.md` row 120, closed).
///
/// A `(` with no closing `)` still reports `Some`: "a modifier was spelled" is the
/// question the refusal asks, and a malformed name must not slip past it by failing to
/// parse. The parser cannot produce one — every arm that opens a paren also expects
/// its `)` — so this is a floor under a caller passing a hand-built string.
fn decompose_type_name(type_name: &str) -> TypeNameParts<'_> {
    let mut head = type_name.trim();
    // `parse_type_name` collapses `[][]` to one `[]`, so a repeat can only come from a
    // hand-built name; strip them all rather than leaving `int4[]` as a base no table
    // holds.
    let mut is_array = false;
    while let Some(rest) = head.strip_suffix("[]") {
        head = rest.trim_end();
        is_array = true;
    }
    match head.split_once('(') {
        Some((base, rest)) => TypeNameParts {
            base: base.trim(),
            modifier: Some(rest.strip_suffix(')').unwrap_or(rest).trim()),
            is_array,
        },
        None => TypeNameParts {
            base: head,
            modifier: None,
            is_array,
        },
    }
}

/// Convert SQL type name to `TypeOid`.
///
/// A type name outside `type_oid_from_name`'s table **fails loud** with
/// `Error::UnsupportedType` — the engine never silently
/// coerces to `TEXT`. Set `planner::set_coerce_unknown_types(true)` to restore
/// the legacy lenient coercion. Shares the fail-loud-vs-coerce decision point
/// (`planner::unsupported_type_oid`) with the `CAST`/`::` path so the two can't
/// drift.
///
/// The modifier is **not** this function's business — it maps a name to a storage
/// type, and a modifier's legality is [`resolve_column_type`]'s gate. Callers that
/// need the refusal must go through that one, which is why this stays crate-visible
/// plus a test re-export rather than a second entry point onto the DDL path.
///
/// # Errors
///
/// Returns `Error::UnsupportedType` for an unsupported or unrecognized type when
/// `coerce_unknown_types` is `false` (the default).
pub fn type_name_to_oid(type_name: &str) -> Result<TypeOid> {
    // `type_name` is already the canonical lower-case spelling: the one type-name
    // reader (`parser::parse_type_name`, over `parse_scalar_type_name`) accepts a bare identifier and downcases it,
    // and every other caller passes a canonical name. No fold here, so the table is
    // the only thing deciding what resolves.
    type_oid_from_parts(decompose_type_name(type_name))
}

/// The storage `TypeOid` for an already-decomposed name: the base through the shared
/// str→TypeOid table, lifted to its array OID when the spelling carried a `[]`.
///
/// An unrecognized base routes through `unsupported_type_oid`, which honors the coerce
/// toggle (fail loud by default, coerce to TEXT when set).
fn type_oid_from_parts(parts: TypeNameParts<'_>) -> Result<TypeOid> {
    let base_oid = crate::types::type_oid_from_name(parts.base)
        .map_or_else(|| crate::planner::unsupported_type_oid(parts.base), Ok)?;
    if parts.is_array {
        Ok(base_oid.array_of().unwrap_or(TypeOid::TextArray))
    } else {
        Ok(base_oid)
    }
}

/// The name 18.6 puts in `type modifier is not allowed for type "%s"` —
/// `TypeNameToString`, which prints the name **as written** plus a single `[]` for an
/// array of any dimension count (measured: `int4(3)[][]` names `"int4[]"`).
///
/// The one spelling that is not as-written is the `SERIAL` family, because the grammar
/// has already rewritten it to its integer type by the time parse analysis reads the
/// modifier — measured on 18.6, `serial(3)`/`serial4(3)` name `integer`,
/// `bigserial(3)`/`serial8(3)` name `bigint`, `smallserial(3)`/`serial2(3)` name
/// `smallint`. The rewrite is read off the resolved `TypeOid` rather than a second
/// list of serial spellings.
fn modifier_refusal_name(parts: TypeNameParts<'_>, type_oid: TypeOid) -> String {
    let base = if crate::planner::is_serial_type(parts.base) {
        let element = type_oid.array_element().unwrap_or(type_oid);
        crate::types::type_oid_to_sql_name(element).unwrap_or(parts.base)
    } else {
        parts.base
    };
    if parts.is_array {
        format!("{base}[]")
    } else {
        base.to_string()
    }
}

/// Refuse a `(…)` modifier on a type that carries none — 18.6's `42601`, raised where
/// `typenameTypeMod` raises it (`parse_type.c`): after the name resolves, so an
/// unrecognized name still answers `42704` first.
///
/// `carries` is the resolved type's own fact, never the spelling's: an **enum** passes
/// `false` because `typtype = 'e'` implies `typmodin = 0`, and an array passes its
/// element's, which is what `_varchar.typmodin = varchartypmodin` means.
///
/// This is `typenameTypeMod`'s **own** gate, ahead of its delegation, and it stays
/// first: 18.6 tests `typmodin == InvalidOid` and only then calls the typmodin, so a
/// type that carries no modifier answers this `42601` rather than a carrier's `22023`
/// (`parse_length_typmod` / `parse_numeric_typmod`).
fn reject_type_modifier(parts: TypeNameParts<'_>, type_oid: TypeOid, carries: bool) -> Result<()> {
    if parts.modifier.is_some() && !carries {
        return Err(Error::syntax_sentence(format!(
            "type modifier is not allowed for type \"{}\"",
            modifier_refusal_name(parts, type_oid)
        )));
    }
    Ok(())
}

/// Whether a [`TypeOid::Varchar`] spelling came from the **`bpchar`** half of 18.6's
/// `character:` production rather than the `varchar` half.
///
/// **One predicate, two readers, because they are two facts about the same half of one
/// grammar arm**: the fixed-length default a *parenless* spelling takes (a `char` column
/// is `char(1)`, a `varchar` unlimited) and the type name a length refusal carries
/// ([`character_typmod_name`]). Two lists would have to agree; one predicate cannot
/// disagree with itself.
///
/// `parser::parse_type_name` canonicalizes `character` to `char` and
/// `character varying` to `varchar`, so the long spellings reach this only from a
/// hand-built name — answered all the same, the way [`decompose_type_name`] is a floor
/// under one.
fn is_bpchar_base(base: &str) -> bool {
    matches!(base, "char" | "character")
}

/// The type name 18.6's length refusal carries — `anychar_typmodin`'s `typename`
/// argument, which is `"char"` from `bpchartypmodin` and `"varchar"` from
/// `varchartypmodin` (`varchar.c`).
///
/// It is the **internal** name and not the spelling: measured on 18.6,
/// `'ab'::character(0)` names `char`, `'ab'::character varying(0)` names `varchar`, and
/// `'{ab}'::char(0)[]` names the element's `char` — which falls out of reading
/// [`TypeNameParts::base`], the array suffix having already come off.
fn character_typmod_name(base: &str) -> &'static str {
    if is_bpchar_base(base) {
        "char"
    } else {
        "varchar"
    }
}

/// The resolved storage type of a declared column: its `TypeOid`, an optional
/// user-`ENUM` name, and whichever of the three modifiers the type carries.
///
/// **All three modifier fields describe the column's *element* type**, which for a
/// scalar column is the column type itself. That is Postgres's own shape — a `varchar(3)[]`
/// column's `atttypmod` is `7`, the `varchar(3)` encoding, not an array-flavoured one
/// (measured on 18.6) — so the fields carry the element's cap for an array column and
/// each reader decides for itself whether the question it asks is about the element or
/// about the column.
#[derive(Debug)]
pub struct ResolvedColumnType {
    pub type_oid: TypeOid,
    /// The lowercased user-`ENUM` type name (keys into `Schema.enums`), if any.
    pub enum_type: Option<String>,
    /// The `NUMERIC(precision, scale)` modifier, if the type spelled one — of the
    /// element for a `numeric(p,s)[]`.
    pub numeric_typmod: Option<NumericTypmod>,
    /// The `varchar(n)` / `char(n)` character-length modifier, if the type spelled
    /// one (both `varchar` and `char` map to `TypeOid::Varchar`, so the length is
    /// enforced identically for both) — of the element for a `varchar(n)[]`.
    pub varchar_length: Option<u32>,
    /// The sub-second precision of a `timestamp(p)` / `timestamptz(p)` / `time(p)` /
    /// `interval(p)`, already **clamped** to
    /// [`MAX_TEMPORAL_PRECISION`](pglet_core::types::MAX_TEMPORAL_PRECISION) — of the
    /// element for a `timestamp(p)[]`, whose `atttypmod` 18.6 records at the element's
    /// width too (measured: `3` for a `timestamp(3)[]`).
    ///
    /// `Some(6)` is a *captured* modifier whose rounding is a no-op, not the same
    /// thing as the `None` a bare `timestamp` carries: 18.6 records
    /// `atttypmod = 6` for `timestamp(9)` (with a warning) and `-1` for `timestamp`,
    /// and `format_type` prints the `(6)`.
    pub temporal_precision: Option<u8>,
}

impl ResolvedColumnType {
    /// The type modifier as a unified [`Typmod`], if any. The three arms are
    /// mutually exclusive — a type is numeric xor length-bearing xor
    /// sub-second-precision-bearing — so the order is a spelling of that fact, not a
    /// precedence anything can exercise. The single conversion the cast path uses to
    /// decide between
    /// [`Expr::Cast`](crate::planner::Expr::Cast) and
    /// [`Expr::CastTypmod`](crate::planner::Expr::CastTypmod).
    ///
    /// It is the **element's** modifier for an array target, and
    /// [`crate::executor::coerce::apply_typmod`] applies it element-wise there, as
    /// 18.6's `ArrayCoerceExpr` does.
    #[must_use]
    pub fn typmod(&self) -> Option<Typmod> {
        if let Some(nt) = self.numeric_typmod {
            return Some(Typmod::Numeric {
                precision: nt.precision,
                scale: nt.scale,
            });
        }
        if let Some(n) = self.varchar_length {
            return Some(Typmod::Length(n));
        }
        self.temporal_precision.map(Typmod::Precision)
    }
}

/// Which of 18.6's two type-name productions a spelling was read at.
///
/// `gram.y` keeps `ConstTypename` separate from `Typename` for many reasons —
/// `ConstTypename` admits neither `GenericType` nor either `ConstInterval` arm, and
/// `Typename`'s array bounds and `SETOF` are outside it — and **the one this flag
/// carries** is the one the comment above `ConstTypename` states first: "to allow
/// defaulting fixed-length types such as CHAR() and BIT() to an unspecified length …
/// this makes no sense for constructs like CHAR 'hi' and BIT '0101'". So the parenless
/// `bpchar` arm installs `bpchar(1)` under `Typename` and `ConstCharacter` clears it
/// again.
///
/// **Which arm clears is the whole rule**: `ConstCharacter` clears
/// `CharacterWithoutLength` and passes `CharacterWithLength` through, so the two
/// positions disagree at exactly the **parenless** spelling, and a spelled `(n)` is the
/// same at both — one this file **refuses** included, the refusal being the same verdict
/// at each. [`resolve_type_name`] states the consequence at the code.
///
/// **What of it is inert here.** 18.6's `character:` production reaches `bpchar` under
/// five spellings (`character`, `char`, `national character`, `national char`,
/// `nchar`), and pglet models the first two: `nchar` is `42704` at both positions, and
/// `national char[acter]` is `42704` at a declared one and `42601` at a constant one
/// (the multiword lead is outside the speculation set — `docs/parser.md`).
/// `ConstBit` does the same clearing to `bit(1)` and is inert the same way — `bit '101'`
/// is `42704` here against 18.6's `101` — the way `timetz`'s share of the
/// temporal-precision rule is inert. Each measured on both engines.
///
/// That default is the whole of what this flag decides: `position` is read at **one**
/// place in [`resolve_type_name`], inside the arm that handles a spelling carrying no
/// modifier at all, so the name's resolution, the enum lookup, the `42601` modifier
/// gate and a **spelled** modifier are the same at both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeNamePosition {
    /// `Typename` — a column definition, `ADD COLUMN`, and a `::` / `CAST(… AS …)`
    /// target. A parenless `char` / `character` is `char(1)` here.
    Declared,
    /// `ConstTypename` — the type name of a typed constant (`char 'ab'`). A parenless
    /// `char` / `character` carries **no** length here.
    Constant,
}

/// Resolve a column's declared type name to its storage `TypeOid` and, for a user
/// `ENUM`, the enum type's (lowercased) name, plus any `NUMERIC(p,s)` modifier.
///
/// Enum values round-trip as their text label, so an enum column is stored as
/// `TypeOid::Text`; the returned name keys into `Schema.enums` for member
/// validation at write time (see `validate_enum_member`). Non-enum names fall back
/// to `type_name_to_oid`, which fails loud on an unsupported/unrecognized type. A
/// `numeric(p[,s])` spelling additionally captures its modifier so the scale is
/// enforced on store.
///
/// **The `(…)` modifier gates are [`resolve_type_name`]'s, so they run at every position
/// that resolves a type name — this one and [`resolve_constant_type`] alike, the
/// [`TypeNamePosition`] flag deciding no part of them** — and they take
/// `typenameTypeMod`'s order: `42601 type modifier is not allowed for type "…"` when the
/// type carries none, and then the carrier's own reader's `22023` when it carries one and
/// refuses the value (`length for type char must be at least 1`,
/// `NUMERIC precision 0 must be between 1 and 1000`, …). That pair is pglet's and not a
/// total over 18.6's own half, which raises further arms of its own that pglet does not: a
/// `+`-signed or computed component, refused a layer earlier here under pglet's own
/// `42601` sentence, and a modifier on a **shell** type, which pglet has nothing to
/// reach — it models no shell type, and the statement that would mint one is itself
/// `42601` here (`docs/coercion.md` § A modifier the type refuses). Both of pglet's are
/// raised where `typenameTypeMod` raises them: after the name resolves (so an unrecognized name still
/// answers `42704`) and before anything the statement does with the type. Their position
/// in a statement's error order is therefore the position `42704` already had: they
/// outrank `42P01` at the cast position, where 18.6 resolves the relation first, and they
/// take the left-to-right column walk on `CREATE TABLE`, where 18.6 does the same —
/// while on `ADD COLUMN` they run **after** the relation lookup and the column-existence
/// check, which is where 18.6 puts them there (see `execute_alter_table_add_column`).
///
/// That shared position is why the `22023` half needed no reordering and inherits the
/// `42601`'s measured order rather than earning one — including the `42P01` residual at
/// the cast, where
/// `SELECT 'ab'::char(0) FROM nosuchtbl` is the `22023` here against 18.6's `42P01`,
/// exactly as `::nosuchtype` and `::int4(3)` already are.
///
/// **"Every position that resolves one" is the whole scope, and two positions do not.**
/// `CREATE`/`ALTER SEQUENCE … AS <type>` (`parser::ddl`) and a column identity clause's
/// `( AS <type> )` each call `parse_type_name` and **discard** the result, so nothing
/// here runs for them: `CREATE SEQUENCE sq1 AS int4(3)`, `… AS varchar(3)` and
/// `… AS nosuchtype(3)` are each an accepted `CREATE SEQUENCE` against 18.6's `42601` /
/// `22023 sequence type must be smallint, integer, or bigint` / `42704`, and each of
/// five identity spellings measured (`int4`, `bigint`, `int4(3)`, `varchar(3)`,
/// `nosuchtype`) is accepted here against 18.6's blanket
/// `42601 conflicting or redundant options` — all measured on both engines. Both
/// predate this gate (the `AS` clause has always been dropped) and both are the
/// dropped-clause divergence rather than this one's.
///
/// **Discarding the result is not the same as accepting every spelling**, and one
/// refusal reaches those two positions anyway: a `FLOAT(n)` outside 1–53 is refused by
/// the *parser*, where 18.6's `opt_float` refuses it, so `CREATE SEQUENCE s AS FLOAT(0)`
/// and `… IDENTITY (AS FLOAT(0))` are `22023` on both engines while their `FLOAT(1)`
/// spellings stay in the dropped-clause divergence above (`EDGES.md` row 116). The
/// `22023` a *carrier* raises is **not** such a refusal — it is this resolver's, and
/// `CREATE SEQUENCE s AS varchar(0)` and `… IDENTITY (AS char(0))` are accepted here
/// against 18.6's `22023` / `42601` for that reason.
///
/// # Errors
///
/// Returns `Error::UnsupportedType` (from `type_name_to_oid`) for an unsupported
/// or unrecognized non-enum type when `coerce_unknown_types` is `false` (the
/// default), `Error::Syntax` (`42601`) for a modifier on a type that carries none, or
/// one of the carrier refusals a modifier the type *does* carry can earn —
/// [`parse_length_typmod`]'s and [`parse_numeric_typmod`]'s.
pub fn resolve_column_type(type_name: &str, schema: &Schema) -> Result<ResolvedColumnType> {
    resolve_type_name(type_name, schema, TypeNamePosition::Declared)
}

/// Resolve the type name of a **typed constant** (`gram.y`'s `ConstTypename Sconst`).
///
/// The same resolution [`resolve_column_type`] performs — same catalog, same enum
/// lookup, same two modifier gates — with the one difference `ConstCharacter`'s own
/// comment gives as its reason: a parenless `char` / `character` carries **no** length,
/// so `char 'ab'` is `ab` rather than the `a` an `'ab'::char` would hold (a `char`
/// *column* holds neither — it is `22001` on both engines).
///
/// A **spelled** modifier is not narrowed: `char(1) 'ab'` still truncates, and a length
/// this file **refuses** (`char(0) 'ab'`, `char(20000000) 'ab'`) answers here exactly what
/// it answers at the declared position — the `22023`, on both engines — see
/// [`resolve_type_name`]'s comment for why that is the same rule and not an exception to
/// it.
///
/// # Errors
///
/// [`resolve_column_type`]'s, unchanged.
pub fn resolve_constant_type(type_name: &str, schema: &Schema) -> Result<ResolvedColumnType> {
    resolve_type_name(type_name, schema, TypeNamePosition::Constant)
}

/// The one resolution both positions run; `position` decides only the fixed-length
/// default (see [`TypeNamePosition`]).
fn resolve_type_name(
    type_name: &str,
    schema: &Schema,
    position: TypeNamePosition,
) -> Result<ResolvedColumnType> {
    // ASCII, matching `lexer::Token::name` and the width `execute_create_type` writes
    // the catalog key at. All four callers today reach here through
    // `parser::type_name`'s `expect_type_word` — `CREATE TABLE`, `ADD COLUMN` and
    // `::`/`CAST` through `parse_type_name`, the typed constant through
    // `parse_scalar_type_name`, which `parse_type_name` wraps in its `[]` loop —
    // and that reader already applied the fold, so this is a no-op on those inputs
    // and exists so the site cannot fold *wider* than the parser: a Unicode
    // `to_lowercase()` turns the parser's `cafÉ` into `café` and binds the column to a
    // different enum than the one it named (see `executor::ddl::execute_create_type`
    // for the measured pair).
    let key = type_name.to_ascii_lowercase();
    let parts = decompose_type_name(&key);
    // The enum lookup is keyed on the **base**, so `moodg(3)` finds the type its
    // modifier rides on rather than missing the catalog and answering `42704` about a
    // name nobody spelled. An enum's `typmodin` is 0 (`typtype = 'e'`), so the modifier
    // is refused — at the array spelling too, ahead of pglet's own array-of-enum gap,
    // which is what 18.6 does (`_moodg` resolves, then the modifier is rejected).
    //
    // **A builtin's name is the builtin's, whatever enum shares it.** 18.6 reads an
    // unqualified type name through the search path, `pg_catalog` first, and a keyword
    // spelling (`integer`, `boolean`, `double precision`) names `pg_catalog`'s type
    // outright — so beside `CREATE TYPE int4 AS ENUM ('k')`, `'5'::int4` is `5` and a
    // `c int4` column is an integer there, the enum reachable only as `public.int4`,
    // which pglet does not parse. So the enum is asked only after every name
    // `type_oid_from_name` resolves. The `SERIAL` spellings are excepted, though
    // `type_oid_from_name` resolves them:
    // 18.6 has no such type, only `transformColumnDefinition`'s rewrite of a column's,
    // so an enum named `serial` is the cast target there as here (`EDGES.md` row 118
    // for the column half and for a builtin pglet has no type for).
    let builtin = crate::types::type_oid_from_name(parts.base).is_some()
        && !crate::planner::is_serial_type(parts.base);
    if !builtin && schema.enums.contains_key(parts.base) {
        reject_type_modifier(parts, TypeOid::Text, false)?;
        if !parts.is_array {
            return Ok(ResolvedColumnType {
                type_oid: TypeOid::Text,
                enum_type: Some(parts.base.to_string()),
                numeric_typmod: None,
                varchar_length: None,
                temporal_precision: None,
            });
        }
        // `moodg[]` has no pglet type; fall through to the `42704` it already answered.
    }
    let type_oid = type_oid_from_parts(parts)?;
    reject_type_modifier(parts, type_oid, type_oid_carries_modifier(type_oid))?;
    // The modifier belongs to the **element**: `numeric(4,1)[]` is an array of
    // `numeric(4,1)`, and 18.6 stores exactly that in the array column's `atttypmod`.
    let element_oid = type_oid.array_element().unwrap_or(type_oid);
    let numeric_typmod = match parts.modifier {
        Some(inner) if element_oid == TypeOid::Numeric => Some(parse_numeric_typmod(inner)?),
        _ => None,
    };
    // `varchar(n)` / `char(n)` — both resolve to `TypeOid::Varchar` — carry a
    // character-length cap. A parenless **bpchar** (`char` / `character`, both
    // canonicalized to the string `char`) defaults to length **1** at a **declared**
    // position (Postgres: a `char` column is `char(1)`) and to **no** length at a
    // constant one, which is what `ConstTypename` splits off (`TypeNamePosition`); a
    // parenless `varchar` / `character varying` stays unlimited (`None`) at both, so
    // the position is invisible to that family. The canonical name's **base**
    // distinguishes the two families even though both share `TypeOid::Varchar`, and it
    // does so at the array spelling too: `char[]` is an array of `char(1)` on 18.6 — a
    // declared-position spelling, the typed-constant grammar being scalar-only.
    //
    // **Whether a length was spelled is the question, and a spelled one the type
    // refuses is a refusal.** `gram.y` installs the default on
    // `CharacterWithoutLength` and nowhere else, and `ConstCharacter` clears that one
    // arm; `CharacterWithLength` hands its `Iconst` straight to `typenameTypeMod`,
    // which hands it to `anychar_typmodin`. So the two answers a spelled `(n)` can
    // have are a length and an **error** — never "no cap", which is what reading
    // `parse_length_typmod`'s old `None` as the parenless case produced: `'ab'::char(0)`
    // was a silent `char(1)` that truncated to `a` under one keying and an
    // unconstrained column that stored `'abcdef'` under the next, where 18.6 has no
    // table to hold the row (`EDGES.md` row 124).
    let varchar_length = if element_oid == TypeOid::Varchar {
        match parts.modifier {
            // Spelled: `CharacterWithLength`, validated by the type at both positions.
            Some(inner) => Some(parse_length_typmod(
                character_typmod_name(parts.base),
                inner,
            )?),
            // Parenless: `CharacterWithoutLength`, the one arm `ConstCharacter`
            // clears — so this is the only place `position` is read.
            None => {
                (is_bpchar_base(parts.base) && position == TypeNamePosition::Declared).then_some(1)
            }
        }
    } else {
        None
    };
    // The four types that carry a sub-second precision. `timetz` would be a fifth on
    // 18.6 and has no `TypeOid` here (`time with time zone` is `42704` before this
    // runs), so its share of the rule is inert.
    let temporal_precision = matches!(
        element_oid,
        TypeOid::Timestamp | TypeOid::Timestamptz | TypeOid::Time | TypeOid::Interval
    )
    .then(|| parts.modifier.and_then(parse_precision_typmod))
    .flatten();
    Ok(ResolvedColumnType {
        type_oid,
        enum_type: None,
        numeric_typmod,
        varchar_length,
        temporal_precision,
    })
}

/// Parse the `(p)` sub-second precision out of a `timestamp(p)` / `timestamptz(p)` /
/// `time(p)` / `interval(p)` spelling — the text between the parens, from
/// [`decompose_type_name`] — **clamping** it to
/// [`MAX_TEMPORAL_PRECISION`](pglet_core::types::MAX_TEMPORAL_PRECISION).
///
/// The clamp is where 18.6 puts it: `anytimestamp_typmodin` / `intervaltypmodin`
/// warn (`precision reduced to maximum allowed, 6`) and then record `6`, so
/// `timestamp(9)`'s column reports `atttypmod = 6` and `datetime_precision = 6`
/// rather than being refused or left unmodified. pglet has no NOTICE channel here,
/// so the warning is the one half not reproduced (`docs/temporal.md`).
///
/// A modifier that is not a non-negative `int32` cannot reach this from the parser —
/// `parser::type_name`'s `opt_precision` refuses it as `42601`, which is 18.6's own
/// answer — so the `None` here is the floor under a hand-built name.
///
/// **That floor is this reader's alone, and the asymmetry is the clamp's.** Its two
/// neighbours refuse ([`parse_length_typmod`], [`parse_numeric_typmod`]) and so return a
/// `Result`; here every `p` an `Iconst` can spell is *legal* — above `6` it is clamped
/// rather than refused — so there is no refusal to raise.
///
/// **The caller does fold this `None` into the parenless case**, which is exactly the
/// conflation `EDGES.md` row 124 is about: `.then(|| parts.modifier.and_then(…))`
/// cannot tell *no modifier was spelled* from *a spelled one this reader declined*. It
/// is a **shadowed** seam rather than an absence, and two facts hold it shut — every
/// path from SQL text arrives through `parser::type_name`'s `opt_precision`, which has
/// already proved the text a non-negative `int32` and so leaves this reader's two
/// `None` arms (a non-`i32` parse, `p < 0`) unreachable; and the one value that *would*
/// otherwise need a verdict, `p` above `6`, is clamped instead. Widen the temporal
/// window past that proof — a range spec, a second `passthrough_type`-style route —
/// and this arm reopens row 124 at the one type family the close exempted.
/// `docs/temporal.md` § Sub-second precision.
fn parse_precision_typmod(inner: &str) -> Option<u8> {
    let max = i32::from(pglet_core::types::MAX_TEMPORAL_PRECISION);
    let p = inner.trim().parse::<i32>().ok()?;
    if p < 0 {
        return None;
    }
    // Clamp **before** narrowing: `timestamp(2147483647)` is `6` on 18.6, and
    // narrowing first would drop it as unrepresentable — the "discarded" failure this
    // rule exists to close, reappearing above 255.
    u8::try_from(p.min(max)).ok()
}

/// Coerce one `(…)` component to `int4`, as 18.6's `ArrayGetIntegerTypmods`
/// (`arrayutils.c`) does on every component before any type's modifier reader sees one.
///
/// **It is part of the modifier's validation and not a parse detail**, which is why its
/// two refusals are positioned rather than merged into the range checks: a component
/// outside `int32` is `22003` and one that is not an integer at all is
/// `22P02 invalid input syntax for type integer: "…"`, and both outrank the range and
/// arity verdicts below — `numeric(0,1.5)` and `numeric(5,2,1.5)` each answer the
/// `22P02` on 18.6, measured, where a later coercion would have named the precision and
/// the arity.
///
/// Only the `numeric` grammar (`opt_type_modifiers: '(' expr_list ')'`) reaches either
/// from **pglet's** SQL text. Every other *carrier*'s window is one `Iconst` that
/// `parser::type_name`'s
/// `opt_precision` has already proved an `int32`, so for those this is the floor under a
/// hand-built name. On 18.6 that `expr_list` is not `numeric`'s alone and any carrier can
/// reach the coercion — `docs/coercion.md` § Two grammar layers, `EDGES.md` row 124's
/// residual.
///
/// The `22003`'s **sentence** is pglet's own (`integer out of range`) where 18.6's is
/// `value "2147483648" is out of range for type integer`: the SQLSTATE conforms and the
/// message does not. That wording is the integer-input path's and not this position's —
/// off this position the same path diverges in the **code** too, `'2147483648'::int` being
/// `22P02` here against `22003` there, with `'99999'::int2` and
/// `'9223372036854775808'::int8` the same, all measured — `EDGES.md` row 324.
fn typmod_component_to_int4(text: &str) -> Result<i32> {
    let trimmed = text.trim();
    trimmed.parse::<i32>().map_err(|e| match e.kind() {
        IntErrorKind::PosOverflow | IntErrorKind::NegOverflow => {
            Error::NumericOutOfRange("integer")
        }
        _ => Error::InvalidInputSyntax {
            type_name: "integer",
            value: trimmed.to_string(),
        },
    })
}

/// Parse the `(n)` character length out of a `varchar(n)` / `char(n)` modifier — the
/// text between the parens, from [`decompose_type_name`] — **refusing** a length the
/// type does not admit rather than dropping it.
///
/// `type_name` is [`character_typmod_name`]'s answer: the internal name the refusal
/// carries, which is not always the spelling.
///
/// **There is no "no cap" answer here.** A spelled modifier resolves to a length or to
/// an error, and the `Option` that means *no modifier was spelled* is the caller's
/// [`TypeNameParts::modifier`] — a different `Option`, one hop earlier. Conflating the
/// two is `EDGES.md` row 124: the decline read as "no cap" made `'ab'::char(0)` answer
/// `ab` and `CREATE TABLE q (c CHAR(0))` build a column that stored `'abcdef'`, where
/// 18.6 raises and has no table to hold the row.
///
/// # Errors
///
/// `Error::CharacterLengthTooSmall` below `1` and `Error::CharacterLengthTooLarge` above
/// [`MAX_CHARACTER_LENGTH`] — `anychar_typmodin`'s two `22023`s, in its order — or
/// [`typmod_component_to_int4`]'s, ahead of both.
fn parse_length_typmod(type_name: &'static str, inner: &str) -> Result<u32> {
    let n = typmod_component_to_int4(inner)?;
    match u32::try_from(n) {
        // `0` and every negative `n` — 18.6's `if (*tl < 1)`. The grammar admits only
        // the `0` (a leading `-` is `42601` in `opt_precision`); a negative one is the
        // floor under a hand-built name.
        Ok(0) | Err(_) => Err(Error::CharacterLengthTooSmall { type_name }),
        Ok(n) if n > MAX_CHARACTER_LENGTH => Err(Error::CharacterLengthTooLarge { type_name }),
        Ok(n) => Ok(n),
    }
}

/// Parse the `(precision[, scale])` modifier out of a `numeric(...)`/`decimal(...)`
/// spelling — the text between the parens, from [`decompose_type_name`].
/// `numeric(p)` defaults the scale to `0`, matching Postgres.
///
/// **Refused in 18.6's own order**, which is one function's — `numerictypmodin`, whose
/// name the arity refusal's `LOCATION` carries on the oracle, calling
/// `ArrayGetIntegerTypmods` first and then the range checks of the
/// `make_numeric_typmod_safe` both its arity branches share — and measured rather
/// than inferred from the source: every component is coerced to `int4`, the count then
/// *selects* the branch (one or two components reach the range checks, any other arity is
/// the `22023`), and inside the two-component branch the precision is checked before the
/// scale. So `numeric(5,2,1.5)` is the coercion's `22P02` and not the arity's `22023`,
/// and `numeric(0,1001)` names the precision and not the scale. The coercion's two
/// refusals are [`typmod_component_to_int4`]'s and stay **outside** this reader's own
/// list throughout this file, as the `# Errors` split below has them.
///
/// **Like [`parse_length_typmod`] it has no "no modifier" answer**, and the component
/// it used to drop was not only out of range but *surplus*: a third one was ignored
/// outright, so `1.5::numeric(5,2,3)` applied the scale `2` from a spelling 18.6 refuses
/// as a whole.
///
/// # Errors
///
/// `Error::InvalidNumericTypeModifier` for an arity other than one or two,
/// `Error::NumericPrecisionOutOfRange` for a precision outside `1` to
/// [`MAX_NUMERIC_PRECISION`], `Error::NumericScaleOutOfRange` for a scale outside
/// ±[`MAX_NUMERIC_SCALE`] — 18.6's three `22023`s — or
/// [`typmod_component_to_int4`]'s, ahead of all three.
fn parse_numeric_typmod(inner: &str) -> Result<NumericTypmod> {
    let components = inner
        .split(',')
        .map(typmod_component_to_int4)
        .collect::<Result<Vec<_>>>()?;
    let (precision, scale) = match components[..] {
        [precision] => (precision, 0),
        [precision, scale] => (precision, scale),
        _ => return Err(Error::InvalidNumericTypeModifier),
    };
    // The window and the narrowing are one step, so the catalog's field widths cannot
    // disagree with the bounds the refusal names: a precision in `1 ..= 1000` fits
    // `u16` and a scale in `-1000 ..= 1000` fits `i16`, and a bound that outgrew either
    // would refuse rather than wrap.
    let precision = u16::try_from(precision)
        .ok()
        .filter(|&p| (1..=MAX_NUMERIC_PRECISION).contains(&i32::from(p)))
        .ok_or(Error::NumericPrecisionOutOfRange { precision })?;
    let scale = i16::try_from(scale)
        .ok()
        .filter(|&s| (-MAX_NUMERIC_SCALE..=MAX_NUMERIC_SCALE).contains(&i32::from(s)))
        .ok_or(Error::NumericScaleOutOfRange { scale })?;
    Ok(NumericTypmod { precision, scale })
}

#[cfg(test)]
mod tests {
    use super::{TypeNameParts, decompose_type_name, resolve_column_type, resolve_constant_type};
    use crate::catalog::{EnumType, NumericTypmod, Schema};
    use crate::planner::Typmod;
    use crate::types::TypeOid;

    #[test]
    fn bare_char_defaults_to_length_one_at_a_declared_position() {
        let schema = Schema::new();
        let len = |t: &str| resolve_column_type(t, &schema).unwrap().varchar_length;
        // A parenless bpchar (`char` / `character`, both canonicalized to "char") is
        // `char(1)`; an explicit `char(n)` keeps its length.
        assert_eq!(len("char"), Some(1));
        assert_eq!(len("char(3)"), Some(3));
        // A parenless `varchar` stays unlimited; `varchar(n)` keeps its length.
        assert_eq!(len("varchar"), None);
        assert_eq!(len("varchar(5)"), Some(5));
        // Both survive a `[]`, at the element's width: 18.6's bare `char[]` column
        // reports `atttypmod = 5` (`char(1)`), and its `varchar[]` reports `-1`.
        assert_eq!(len("char[]"), Some(1));
        assert_eq!(len("char(3)[]"), Some(3));
        assert_eq!(len("varchar[]"), None);
        assert_eq!(len("varchar(5)[]"), Some(5));
    }

    /// What `gram.y`'s `ConstCharacter` narrows: a **parenless** `char` carries no
    /// length at a typed-constant position, where a declared one is `char(1)`.
    ///
    /// **Parenless is the predicate, and the refused lengths are why it has to be.**
    /// `ConstCharacter` clears the `CharacterWithoutLength` arm and passes
    /// `CharacterWithLength` through, so a `(n)` this file's `parse_length_typmod`
    /// **refuses** — below 1, or above 10 485 760 — is a spelled modifier at both
    /// positions and takes the default at neither. Two keyings of this have been wrong
    /// in opposite directions: reading the refusal as "no modifier was spelled" split
    /// the positions at a spelling that is not parenless at all (`char(0)` resolved to
    /// `Some(1)` declared against the constant's `None`), and reading it as "no cap"
    /// resolved both to an unconstrained `varchar` where 18.6 raises.
    ///
    /// The two sides of each pair route through the same `resolve_type_name`, so this
    /// is blind to anything that moves them together (the `(n)` parse, the array
    /// widths) — the absolute assertions are what separate them, and
    /// `a_refused_length_is_not_the_parenless_spelling` in
    /// `crates/pglet/tests/typmod_decomposition.rs` is the end-to-end half, over values
    /// and a write rather than over this resolver.
    #[test]
    fn bare_char_carries_no_length_at_a_constant_position() {
        let schema = Schema::new();
        let declared = |t: &str| resolve_column_type(t, &schema).unwrap().varchar_length;
        let constant = |t: &str| resolve_constant_type(t, &schema).unwrap().varchar_length;
        // The split, both spellings of the parenless bpchar (canonicalized to "char").
        assert_eq!(declared("char"), Some(1));
        assert_eq!(constant("char"), None);
        // A **spelled** modifier is the constant's too — `ConstCharacter` clears only
        // the `CharacterWithoutLength` arm's default.
        assert_eq!(constant("char(3)"), Some(3));
        assert_eq!(constant("char(1)"), Some(1));
        // …and a spelled one this file **refuses** is nobody's default: the same
        // refusal at both positions, message included, rather than a `Some(1)` on one
        // side or an unconstrained type on both. Both ends of the window — below 1 and
        // above the cap — with the `varchar` spelling and the array one beside them,
        // and `char(-1)`, which only a hand-built name reaches (`opt_precision` takes
        // the `-` as `42601` first).
        let refused = |t: &str| {
            let declared_err = resolve_column_type(t, &schema).unwrap_err();
            let constant_err = resolve_constant_type(t, &schema).unwrap_err();
            assert_eq!(
                declared_err.to_string(),
                constant_err.to_string(),
                "the two positions refuse `{t}` alike"
            );
            (declared_err.sqlstate(), declared_err.to_string())
        };
        for (name, message) in [
            ("char(0)", "length for type char must be at least 1"),
            ("char(-1)", "length for type char must be at least 1"),
            ("varchar(0)", "length for type varchar must be at least 1"),
            ("char(0)[]", "length for type char must be at least 1"),
            ("varchar(0)[]", "length for type varchar must be at least 1"),
            (
                "char(10485761)",
                "length for type char cannot exceed 10485760",
            ),
            (
                "char(20000000)",
                "length for type char cannot exceed 10485760",
            ),
            (
                "varchar(10485761)",
                "length for type varchar cannot exceed 10485760",
            ),
        ] {
            assert_eq!(refused(name), ("22023", message.to_string()), "`{name}`");
        }
        // The window's inclusive ends are accepted, which is what keeps the refusal
        // from being an off-by-one: `1` and `10485760` resolve, at both positions.
        assert_eq!(declared("char(1)"), Some(1));
        assert_eq!(declared("varchar(10485760)"), Some(10_485_760));
        assert_eq!(constant("varchar(10485760)"), Some(10_485_760));
        assert_eq!(declared("char[]"), Some(1));
        // The `varchar` branch of the same grammar arm spells no default at either
        // position, so it cannot see the split.
        assert_eq!(declared("varchar"), None);
        assert_eq!(constant("varchar"), None);
        assert_eq!(constant("varchar(5)"), Some(5));
        // The modifiers a type does carry are the position's alike — `position` is read
        // at one place in `resolve_type_name`, and it is not this one.
        assert_eq!(
            resolve_constant_type("numeric(5,2)", &schema)
                .unwrap()
                .typmod(),
            Some(Typmod::Numeric {
                precision: 5,
                scale: 2
            })
        );
        assert_eq!(
            resolve_constant_type("timestamp(3)", &schema)
                .unwrap()
                .temporal_precision,
            Some(3)
        );
        // And so is the `42601` modifier gate, and the `42704` a name outside the
        // table answers — the position decides the parenless default and nothing else.
        assert_eq!(
            resolve_constant_type("int4(3)", &schema)
                .unwrap_err()
                .sqlstate(),
            "42601"
        );
        assert_eq!(
            resolve_constant_type("nosuchtype", &schema)
                .unwrap_err()
                .sqlstate(),
            "42704"
        );
    }

    /// The `[]` must come off **before** the `(` split, or the suffix rides away with
    /// the modifier and the name resolves to the scalar element.
    #[test]
    fn a_canonical_name_decomposes_into_base_modifier_and_suffix() {
        let parts = |s| decompose_type_name(s);
        assert_eq!(
            parts("varchar(3)[]"),
            TypeNameParts {
                base: "varchar",
                modifier: Some("3"),
                is_array: true
            }
        );
        assert_eq!(
            parts("numeric(5,-2)"),
            TypeNameParts {
                base: "numeric",
                modifier: Some("5,-2"),
                is_array: false
            }
        );
        assert_eq!(
            parts("character varying"),
            TypeNameParts {
                base: "character varying",
                modifier: None,
                is_array: false
            }
        );
        assert_eq!(
            parts("int4[]"),
            TypeNameParts {
                base: "int4",
                modifier: None,
                is_array: true
            }
        );
        // A repeated suffix collapses, as `parse_type_name` collapses it.
        assert_eq!(
            parts("int4(3)[][]"),
            TypeNameParts {
                base: "int4",
                modifier: Some("3"),
                is_array: true
            }
        );
        // An unbalanced paren still reports a modifier: the gate asks whether one was
        // spelled, and a name that failed to parse must not slip past it.
        assert_eq!(
            parts("int4(3"),
            TypeNameParts {
                base: "int4",
                modifier: Some("3"),
                is_array: false
            }
        );
    }

    #[test]
    fn a_modifier_on_a_type_that_carries_none_is_refused() {
        let mut schema = Schema::new();
        schema.enums.insert(
            "moodx".to_string(),
            EnumType {
                oid: 90000,
                labels: vec!["ok".to_string()],
            },
        );
        let sqlstate = |t: &str| {
            resolve_column_type(t, &schema)
                .map(|r| r.type_oid)
                .map_err(|e| e.sqlstate().to_string())
        };
        // The refusal, at the scalar and array spellings and over an enum.
        for name in [
            "int4(3)",
            "bool(3)",
            "date(3)",
            "float8(3)",
            "serial(3)",
            "int4(5,2)",
            "int4(3)[]",
            "bool(3)[]",
            "moodx(3)",
            "moodx(3)[]",
        ] {
            assert_eq!(sqlstate(name), Err("42601".to_string()), "{name}");
        }
        // The polarity control: a modifier the type does carry, and an array with no
        // modifier at all, both resolve.
        assert_eq!(sqlstate("varchar(3)"), Ok(TypeOid::Varchar));
        assert_eq!(sqlstate("numeric(4,1)"), Ok(TypeOid::Numeric));
        assert_eq!(sqlstate("interval(3)"), Ok(TypeOid::Interval));
        assert_eq!(sqlstate("varchar(3)[]"), Ok(TypeOid::VarcharArray));
        assert_eq!(sqlstate("numeric(4,1)[]"), Ok(TypeOid::NumericArray));
        assert_eq!(sqlstate("int4[]"), Ok(TypeOid::Int4Array));
        assert_eq!(sqlstate("moodx"), Ok(TypeOid::Text));
        // An unrecognized name still answers `42704` — the name resolves before the
        // modifier is judged, which is 18.6's order.
        assert_eq!(sqlstate("nosuchtype(3)"), Err("42704".to_string()));
    }

    /// **A modifier the type does carry is kept, clamped, and is the *element's*.**
    /// Four types carry a sub-second precision; the clamp is `anytimestamp_typmodin`'s
    /// (18.6 warns and records `6`); and a `[]` captures at the element's width, which
    /// is what 18.6's `atttypmod` holds for the array column (measured: `3` for a
    /// `timestamp(3)[]`).
    #[test]
    fn a_temporal_precision_is_captured_at_the_elements_width_and_clamped() {
        let schema = Schema::new();
        let p = |t: &str| resolve_column_type(t, &schema).unwrap().temporal_precision;
        assert_eq!(p("timestamp(3)"), Some(3));
        assert_eq!(p("timestamptz(3)"), Some(3));
        assert_eq!(p("time(3)"), Some(3));
        assert_eq!(p("interval(3)"), Some(3));
        assert_eq!(p("timestamp(0)"), Some(0));
        // `6` is a *captured* modifier whose rounding is a no-op, not the `None` a bare
        // spelling carries — 18.6 records `atttypmod = 6` against `-1`.
        assert_eq!(p("timestamp(6)"), Some(6));
        assert_eq!(p("timestamp"), None);
        assert_eq!(p("interval"), None);
        // Clamped rather than refused or dropped, at both widths of the number: `u8`'s
        // range and beyond it.
        assert_eq!(p("timestamp(9)"), Some(6));
        assert_eq!(p("timestamp(300)"), Some(6));
        assert_eq!(p("timestamp(2147483647)"), Some(6));
        assert_eq!(p("interval(9)"), Some(6));
        // The element's, under a `[]`.
        assert_eq!(p("timestamp(3)[]"), Some(3));
        assert_eq!(p("interval(3)[]"), Some(3));
        assert_eq!(p("timestamp[]"), None);
        // A type that carries no sub-second precision captures none, whatever it
        // carries otherwise.
        assert_eq!(p("date"), None);
        assert_eq!(p("varchar(3)"), None);
        assert_eq!(p("numeric(4,1)"), None);
        // ...and the unified `Typmod` follows, which is what the cast path reads.
        let typmod = |t: &str| resolve_column_type(t, &schema).unwrap().typmod();
        assert_eq!(typmod("timestamp(3)"), Some(Typmod::Precision(3)));
        assert_eq!(typmod("interval(9)"), Some(Typmod::Precision(6)));
        assert_eq!(typmod("timestamp"), None);
        assert_eq!(typmod("varchar(3)"), Some(Typmod::Length(3)));
    }

    /// 18.6 names the type **as written**, plus one `[]` for an array — except the
    /// `SERIAL` family, which the grammar has already rewritten by the time parse
    /// analysis reads the modifier. Each string measured on 18.6.
    #[test]
    fn the_refusal_names_the_type_18_6_names() {
        let schema = Schema::new();
        let msg = |t: &str| resolve_column_type(t, &schema).unwrap_err().to_string();
        assert_eq!(
            msg("int4(3)"),
            "type modifier is not allowed for type \"int4\""
        );
        assert_eq!(
            msg("int4(3)[]"),
            "type modifier is not allowed for type \"int4[]\""
        );
        assert_eq!(
            msg("serial(3)"),
            "type modifier is not allowed for type \"integer\""
        );
        assert_eq!(
            msg("serial4(3)"),
            "type modifier is not allowed for type \"integer\""
        );
        assert_eq!(
            msg("bigserial(3)"),
            "type modifier is not allowed for type \"bigint\""
        );
        assert_eq!(
            msg("serial8(3)"),
            "type modifier is not allowed for type \"bigint\""
        );
        assert_eq!(
            msg("smallserial(3)"),
            "type modifier is not allowed for type \"smallint\""
        );
        assert_eq!(
            msg("serial2(3)"),
            "type modifier is not allowed for type \"smallint\""
        );
    }

    /// The length refusal names the **internal** type and not the spelling — 18.6 hands
    /// `anychar_typmodin` the `"char"` / `"varchar"` its two wrappers pass, never the
    /// `character` / `character varying` a statement wrote — and an array names its
    /// *element*.
    ///
    /// The long spellings reach the resolver only from a hand-built name
    /// (`parse_type_name` canonicalizes both), so they are here as the floor under one:
    /// `is_bpchar_base` answering them is what keeps the message and the parenless
    /// default from disagreeing about which half of the `character:` production a name
    /// came from. Each string measured on 18.6 at the spelling pglet emits.
    #[test]
    fn the_length_refusal_names_the_internal_type_and_an_array_names_its_element() {
        let schema = Schema::new();
        let msg = |t: &str| resolve_column_type(t, &schema).unwrap_err().to_string();
        for name in ["char(0)", "character(0)", "char(0)[]", "character(0)[]"] {
            assert_eq!(
                msg(name),
                "length for type char must be at least 1",
                "`{name}`"
            );
        }
        for name in [
            "varchar(0)",
            "character varying(0)",
            "varchar(0)[]",
            "character varying(0)[]",
        ] {
            assert_eq!(
                msg(name),
                "length for type varchar must be at least 1",
                "`{name}`"
            );
        }
        // The same predicate decides the parenless default, so a hand-built
        // `character` is `char(1)` and a hand-built `character varying` unlimited.
        let len = |t: &str| resolve_column_type(t, &schema).unwrap().varchar_length;
        assert_eq!(len("character"), Some(1));
        assert_eq!(len("character varying"), None);
    }

    /// `numeric`'s refusals, **in `numerictypmodin`'s order** — the function 18.6 names
    /// in the arity `22023`'s `LOCATION`, the range checks' being the
    /// `make_numeric_typmod_safe` it calls: every component is coerced to `int4` first,
    /// the arity then selects the branch, and the range check tests the precision before
    /// the scale (the one-component branch has no scale to reach).
    ///
    /// The order is the point, and each pair below separates one step from the next —
    /// `numeric(5,2,1.5)` is the coercion's and not the arity's, `numeric(0,1.5)` the
    /// coercion's and not the precision's, `numeric(0,1001)` the precision's and not the
    /// scale's. All measured on 18.6; a whole-pass reordering would keep every single
    /// refusal right and answer the wrong one at each of these three.
    #[test]
    fn a_numeric_modifier_is_refused_in_18_6s_order() {
        let schema = Schema::new();
        let verdict = |t: &str| {
            let e = resolve_column_type(t, &schema).unwrap_err();
            (e.sqlstate(), e.to_string())
        };
        let precision = |p: &str| {
            (
                "22023",
                format!("NUMERIC precision {p} must be between 1 and 1000"),
            )
        };
        let scale = |s: &str| {
            (
                "22023",
                format!("NUMERIC scale {s} must be between -1000 and 1000"),
            )
        };
        let arity = || ("22023", "invalid NUMERIC type modifier".to_string());
        let not_an_integer = |v: &str| {
            (
                "22P02",
                format!("invalid input syntax for type integer: \"{v}\""),
            )
        };
        let out_of_int4 = || ("22003", "integer out of range".to_string());
        for (name, want) in [
            // The precision's window, both ends, and the negative the scale grammar's
            // `-` lets through.
            ("numeric(0)", precision("0")),
            ("numeric(-1)", precision("-1")),
            ("numeric(1001)", precision("1001")),
            ("decimal(0)", precision("0")),
            ("numeric(0)[]", precision("0")),
            // The scale's window, both ends.
            ("numeric(5,1001)", scale("1001")),
            ("numeric(5,-1001)", scale("-1001")),
            // The arity, which is 18.6's only reachable `invalid … type modifier`.
            ("numeric(5,2,3)", arity()),
            ("numeric(5,2,3,4)", arity()),
            // The `int4` coercion, ahead of all three.
            ("numeric(2147483648)", out_of_int4()),
            ("numeric(5,2147483648)", out_of_int4()),
            ("numeric(1.5)", not_an_integer("1.5")),
            ("numeric(5,1.5)", not_an_integer("1.5")),
            // …and the three orderings that separate the steps.
            ("numeric(5,2,1.5)", not_an_integer("1.5")),
            ("numeric(0,1.5)", not_an_integer("1.5")),
            ("numeric(0,2147483648)", out_of_int4()),
            ("numeric(0,1001)", precision("0")),
        ] {
            assert_eq!(verdict(name), want, "`{name}`");
        }
        // The accepted ends: both windows are inclusive, and a negative scale is
        // ordinary (`numeric(5,-2)` rounds to hundreds).
        let typmod = |t: &str| resolve_column_type(t, &schema).unwrap().numeric_typmod;
        assert_eq!(
            typmod("numeric(1000)"),
            Some(NumericTypmod {
                precision: 1000,
                scale: 0
            })
        );
        assert_eq!(
            typmod("numeric(5,1000)"),
            Some(NumericTypmod {
                precision: 5,
                scale: 1000
            })
        );
        assert_eq!(
            typmod("numeric(5,-1000)"),
            Some(NumericTypmod {
                precision: 5,
                scale: -1000
            })
        );
        assert_eq!(
            typmod("numeric(1)"),
            Some(NumericTypmod {
                precision: 1,
                scale: 0
            })
        );
        assert_eq!(typmod("numeric"), None);
    }

    /// A modifier spelled on a type the name resolves to **something else** is not this
    /// rule's to judge, and the three ways that happens are each unchanged.
    ///
    /// The polarity list the two new refusals invite an over-reach onto: a temporal
    /// `(p)` is **clamped** rather than refused at any `int32`; a type that carries no
    /// modifier answers the earlier `42601`; and an unrecognized name answers `42704`
    /// before either reader runs. Each measured on 18.6.
    #[test]
    fn the_carrier_refusals_do_not_reach_a_non_carrier() {
        let schema = Schema::new();
        let state = |t: &str| {
            resolve_column_type(t, &schema)
                .map(|r| r.typmod())
                .map_err(|e| e.sqlstate())
        };
        // Clamped, not refused — the arm with no `Result`.
        assert_eq!(
            state("timestamp(2147483647)"),
            Ok(Some(Typmod::Precision(6)))
        );
        assert_eq!(state("interval(9)"), Ok(Some(Typmod::Precision(6))));
        assert_eq!(state("time(0)"), Ok(Some(Typmod::Precision(0))));
        // The carries-none refusal still wins, and it is `42601` and not a `22023`
        // about a window `int4` has no business in.
        assert_eq!(state("int4(0)"), Err("42601"));
        assert_eq!(state("bool(2147483648)"), Err("42601"));
        assert_eq!(state("date(0)"), Err("42601"));
        // And the name still resolves first.
        assert_eq!(state("nosuchtype(0)"), Err("42704"));
    }
}
