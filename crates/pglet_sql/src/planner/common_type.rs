//! Postgres's **two** type-resolution rulebooks, and the line between them.
//!
//! It is tempting to believe a SQL engine has one notion of "the common type of
//! these expressions". Postgres has two, they disagree, and picking the wrong one
//! is invisible until a binary client decodes the result:
//!
//! - [`select_common_type`] — `parse_coerce.c`'s `select_common_type`, the
//!   **category/preferred-type** algorithm. Governs `COALESCE`, `CASE`,
//!   `GREATEST`/`LEAST`, `ARRAY[…]`, and set-operation columns. Order-dependent:
//!   `COALESCE(varchar_col, text_col)` is `varchar` while the reverse is `text`.
//! - [`nullif_result_type`] — `NULLIF` resolves the `=` **operator** and then takes
//!   *argument one's* type as the operator resolution left it
//!   (`parse_expr.c`: `result->opresulttype = exprType(linitial(result->args))`).
//!   So `NULLIF(int4_col, int8_col)` is `int4`, where `COALESCE` of the same pair is
//!   `int8`.
//!
//! Both are transcribed from measured behavior against the live oracle, not from the
//! source alone: every pair in the numeric / string / datetime families was probed and
//! the rules here reproduce all of them. Two consequences worth stating, because both
//! were wrong in pglet before and neither is intuitive:
//!
//! - **`float4` beats `int` and `numeric`** under `select_common_type`
//!   (`COALESCE(int4, real)` is `real`, not `double precision`), because `int4 → float4`
//!   is an implicit cast and the reverse is not. The *arithmetic* promotion
//!   ([`super::expr::numeric_family_result`]) is a different question with a different
//!   answer — `int4 + real` really is `double precision` — and the two must not be
//!   conflated.
//! - **A tie keeps the first.** When each type is implicitly coercible to the other
//!   (`text` ↔ `varchar`), Postgres keeps the one it already had, which is why the
//!   order of `CASE`'s branches is observable — and why [`super::coercion`] must feed
//!   `CASE`'s `ELSE` in **first** (`parse_expr.c` prepends `defresult`, commented
//!   "default result is considered the most significant type").

use crate::types::{TypeOid, type_oid_display_name};
use crate::{Error, Result};

/// A type as the resolution rulebooks resolve it — 18.6's `pg_type` **row**, not only
/// its OID.
///
/// A `TypeOid` alone cannot answer either question these rulebooks ask of an `ENUM`. An
/// enum column's `type_oid` is `Text` and its identity rides beside it on
/// `ColumnDesc::enum_type`, so on the OID alone `mood` reads as `text` — which makes
/// `mood` and `text` one type, and `mood` and `m3` one type as well. 18.6 answers both
/// from `pg_type`: an enum is `typcategory` `E` where `text`/`varchar`/`name` are `S`,
/// so that pair has no common type at all (`42804`) and no operator (`42883`); two
/// enums share `E`, so `select_common_type` succeeds there and the *conversion* is what
/// fails (`42846`). Carrying the identity beside the OID is what lets one rulebook give
/// all three.
///
/// [`Self::enum_type`] is `Some` for an enum **and** for an array whose element type is
/// one, which is why [`resolved_category`] asks the OID before it asks this field: a
/// `mood[]` is `typcategory` `A` like every other array, and against a `text[]` it is a
/// same-category conversion failure (`42846 could not convert type text[] to mood[]`,
/// measured) rather than a category mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedType {
    /// The OID the wire, the codecs and every non-enum rule read. `Text` for an enum.
    pub oid: TypeOid,
    /// What this operand's enum identity is — or that it cannot be decided.
    pub enum_type: EnumIdentity,
}

/// Whether an operand is an enum, and which — with a **third** state for the operands
/// whose identity this engine cannot decide.
///
/// The third state is not fussiness. `x::mood` reaches the planner as a cast to `text`,
/// the enum name being dropped where the parser reads the type (`EDGES.md` row 104) — kept
/// only inside the membership test beneath the cast (`StringFunc::EnumIn`), which no
/// identity reader consults — so the gates cannot tell `t::mood` from `t::text`. Refusing
/// both against an enum would
/// break `greatest(m, t::mood)`, which 18.6 answers; accepting both leaves `m = t::text`
/// silent, which is what pglet did before this rule and what row 104 already records. Of
/// the two, only the first is a **new** wrong answer, so the pair falls back to the OID
/// rule and the under-refusal stays row 104's.
///
/// A bare string or `NULL` literal takes the same state for a different and exact
/// reason: it is 18.6's `unknown`, which *adopts* the position's type, enum included —
/// `m = 'ok'` and `COALESCE(m, 'sad')` resolve to `mood` there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EnumIdentity {
    /// Not an enum: the operand's type is exactly its OID.
    #[default]
    NotAnEnum,
    /// This enum, by the lowercased catalog key into `Schema.enums` — the same fact
    /// `ColumnDesc::enum_type` carries. When the OID is an array type this is its
    /// **element's** identity.
    Named(String),
    /// Possibly an enum, and undecidable here — see this type's docs.
    Undecided,
}

impl EnumIdentity {
    /// The enum's name where this names one.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Named(n) => Some(n),
            Self::NotAnEnum | Self::Undecided => None,
        }
    }

    /// Whether this is the undecidable state, which makes a pair fall back to the OID
    /// rule rather than refuse.
    #[must_use]
    pub const fn is_undecided(&self) -> bool {
        matches!(self, Self::Undecided)
    }
}

impl ResolvedType {
    /// A type carrying no enum identity — every operand but an enum's, and the shape
    /// every eval-time caller builds, where a `Value` carries no identity to read.
    #[must_use]
    pub const fn plain(oid: TypeOid) -> Self {
        Self {
            oid,
            enum_type: EnumIdentity::NotAnEnum,
        }
    }

    /// `oid` under an enum name, or none.
    #[must_use]
    pub fn new(oid: TypeOid, enum_type: Option<String>) -> Self {
        Self {
            oid,
            enum_type: enum_type.map_or(EnumIdentity::NotAnEnum, EnumIdentity::Named),
        }
    }

    /// `oid` whose identity cannot be decided — [`EnumIdentity::Undecided`].
    #[must_use]
    pub const fn undecided_enum(oid: TypeOid) -> Self {
        Self {
            oid,
            enum_type: EnumIdentity::Undecided,
        }
    }

    /// A projected column's type, read off the descriptor that already carries both
    /// halves — the one conversion for every gate whose namespace is a `ColumnDesc`
    /// list (the set-operation fold, a `VALUES` list's, a `USING` merge's equality).
    #[must_use]
    pub fn of_column(column: &crate::types::ColumnDesc) -> Self {
        Self::new(column.type_oid, column.enum_type.clone())
    }

    /// The name 18.6 prints for this type in an error message: the enum's own name, an
    /// array's element name with `[]` after it, else the OID's display name.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self.enum_type.name() {
            Some(name) if self.oid.array_element().is_some() => format!("{name}[]"),
            Some(name) => name.to_string(),
            None => type_oid_display_name(self.oid).to_string(),
        }
    }
}

/// Postgres's `pg_type.typcategory`, for the types pglet models.
///
/// Only the distinction matters, never the letter: two types in different categories
/// have no common type at all (`42804`), while two in the same category may still fail
/// to convert (`42846`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCategory {
    /// `B` — boolean.
    Boolean,
    /// `N` — the numeric family.
    Numeric,
    /// `S` — the string family.
    String,
    /// `D` — date/time. **Includes `time`**, which is why `COALESCE(date, time)` is a
    /// same-category *conversion* failure (`42846`) and not a category mismatch.
    DateTime,
    /// `T` — timespan (`interval` only). Deliberately **not** the same category as
    /// `time`, despite the two comparing to each other.
    Timespan,
    /// `U` — user-defined (`uuid`, `jsonb`, `bytea`). Mutually uncombinable, but as a
    /// `42846` rather than a `42804`.
    UserDefined,
    /// `A` — array types. **Includes an array of an enum**, which is why an enum
    /// identity does not by itself put a type in [`Self::Enum`] — see [`ResolvedType`].
    Array,
    /// `E` — an enum type. Reached only through [`resolved_category`]: no `TypeOid`
    /// names an enum, so [`type_category`] cannot answer it.
    Enum,
}

/// The `typcategory` of `oid`.
///
/// The `_` arm answers `Array`, which is right for every array OID and is where the
/// declaration types whose `pg_cast` rows are not modelled here land: `oid` is
/// `typcategory` `N` on 18.6 and `"char"` is `Z`, and their fold divergence is
/// `EDGES.md` row 87. `name` is **not** one of them — it is `S` here, as on 18.6, because
/// its rows are modelled ([`implicitly_coercible`]).
#[must_use]
pub const fn type_category(oid: TypeOid) -> TypeCategory {
    match oid {
        TypeOid::Bool => TypeCategory::Boolean,
        TypeOid::Int2
        | TypeOid::Int4
        | TypeOid::Int8
        | TypeOid::Float4
        | TypeOid::Float8
        | TypeOid::Numeric => TypeCategory::Numeric,
        TypeOid::Text | TypeOid::Varchar | TypeOid::Name => TypeCategory::String,
        TypeOid::Date | TypeOid::Timestamp | TypeOid::Timestamptz | TypeOid::Time => {
            TypeCategory::DateTime
        }
        TypeOid::Interval => TypeCategory::Timespan,
        TypeOid::Bytea | TypeOid::Jsonb | TypeOid::Uuid => TypeCategory::UserDefined,
        _ => TypeCategory::Array,
    }
}

/// The `typcategory` of a fully-resolved type — [`type_category`] with the one answer
/// an OID cannot give.
///
/// A scalar enum is `E`; an **array** of one is `A`, its element identity being what
/// decides a conversion inside that category rather than the category itself.
#[must_use]
pub const fn resolved_category(t: &ResolvedType) -> TypeCategory {
    if matches!(t.enum_type, EnumIdentity::Named(_)) && t.oid.array_element().is_none() {
        return TypeCategory::Enum;
    }
    type_category(t.oid)
}

/// Whether `oid` is its category's **preferred** type (`pg_type.typispreferred`).
///
/// Load-bearing in exactly one place: a preferred `ptype` is never displaced, which is
/// why `COALESCE(float8_col, int4_col)` stays `float8` and `COALESCE(text_col,
/// varchar_col)` stays `text`.
#[must_use]
pub const fn is_preferred_type(oid: TypeOid) -> bool {
    matches!(
        oid,
        TypeOid::Bool | TypeOid::Float8 | TypeOid::Text | TypeOid::Timestamptz | TypeOid::Interval
    )
}

/// Whether Postgres registers an **implicit** cast `from → to`
/// (`pg_cast.castcontext = 'i'`), enumerated from the live catalog.
///
/// Assignment-context casts are deliberately absent: `float8 → float4`,
/// `float4 → numeric`, and every float/numeric → int narrowing are `'a'`, and treating
/// one as implicit here would silently narrow a value in a `COALESCE`.
#[must_use]
pub fn implicitly_coercible(from: TypeOid, to: TypeOid) -> bool {
    if from as u32 == to as u32 {
        return true;
    }
    // **Arrays coerce element-wise.** Postgres resolves array-to-array through
    // `COERCION_PATH_ARRAYCOERCE`, which succeeds in whatever context the *element*
    // coercion succeeds in — so `int4[] → int8[]` is implicit exactly because
    // `int4 → int8` is. Without this arm two distinct array types sat in category `A`
    // with no conversion either way, and `COALESCE(int_array, bigint_array)` — which
    // Postgres accepts — became a `42846`. That over-rejection
    // escalated past a query error: a store already carrying a
    // `CHECK (COALESCE(a, b) IS NOT NULL)` over two array columns still read and wrote
    // fine, but its `dump` no longer restored into pglet, while the byte-identical dump
    // restored into real Postgres. Measured element-wise on every pairing: `int4[]` vs
    // `numeric[]` is `numeric[]`, `int4[]` vs `real[]` is `real[]`, `varchar[]` before
    // `text[]` stays `varchar[]` and the reverse is `text[]` — the scalar rules exactly,
    // one dimension up.
    if let (Some(from_element), Some(to_element)) = (from.array_element(), to.array_element()) {
        return implicitly_coercible(from_element, to_element);
    }
    matches!(
        (from, to),
        (
            TypeOid::Int2,
            TypeOid::Int4 | TypeOid::Int8 | TypeOid::Float4 | TypeOid::Float8 | TypeOid::Numeric
        ) | (
            TypeOid::Int4,
            TypeOid::Int8 | TypeOid::Float4 | TypeOid::Float8 | TypeOid::Numeric
        ) | (
            TypeOid::Int8,
            TypeOid::Float4 | TypeOid::Float8 | TypeOid::Numeric
        ) | (TypeOid::Float4, TypeOid::Float8)
            | (TypeOid::Numeric, TypeOid::Float4 | TypeOid::Float8)
            // The string family's rows, read off `pg_cast` on 18.6. `name` has three
            // implicit ones — `name → text`, `text → name`, `varchar → name` — while
            // `name → varchar` and `name → bpchar` are **assignment** and so absent, and
            // `bpchar` has no `TypeOid` to name. That asymmetry is what makes
            // `COALESCE(name, varchar)` resolve `name` (19) rather than `varchar`: the
            // fold displaces `ptype` only when the reverse cast is *not* implicit, and
            // here it is not. Clippy wants the sources nested, so `name → text` rides on
            // the `varchar → text` arm.
            | (TypeOid::Varchar | TypeOid::Name, TypeOid::Text)
            | (TypeOid::Text, TypeOid::Varchar)
            | (TypeOid::Text | TypeOid::Varchar, TypeOid::Name)
            | (TypeOid::Date, TypeOid::Timestamp | TypeOid::Timestamptz)
            | (TypeOid::Timestamp, TypeOid::Timestamptz)
            // `time → interval` is a real implicit cast, but `select_common_type` never
            // reaches it: `time` is category `D` and `interval` is `T`, so the category
            // check rejects the pair first (`COALESCE(time, interval)` is `42804`, as in
            // Postgres). The pair is resolved by the *operator* rulebook instead —
            // `comparison_operand_type` — which is what `NULLIF(time_col, interval_col)`
            // uses to answer `interval`.
            | (TypeOid::Time, TypeOid::Interval)
    )
}

/// [`is_preferred_type`] over a fully-resolved type. No enum is preferred
/// (`pg_type.typispreferred` is false for every one), and asking the OID alone would
/// read `mood` as `text`, which is.
#[must_use]
const fn resolved_is_preferred(t: &ResolvedType) -> bool {
    matches!(t.enum_type, EnumIdentity::NotAnEnum) && is_preferred_type(t.oid)
}

/// [`implicitly_coercible`] over fully-resolved types.
///
/// `pg_cast` holds **no** row for an enum in either direction — `mood → text` and
/// `text → mood` are I/O conversions an explicit cast performs and nothing implicit
/// reaches — so an enum coerces implicitly only to *itself*, and an array of one only
/// to an array of the same one. Every other pair is [`implicitly_coercible`]'s.
#[must_use]
pub fn resolved_coercible(from: &ResolvedType, to: &ResolvedType) -> bool {
    // An undecidable identity on either side is no information, so the pair is judged on
    // the OIDs exactly as it was before enums reached this rulebook.
    if from.enum_type.is_undecided() || to.enum_type.is_undecided() {
        return implicitly_coercible(from.oid, to.oid);
    }
    if from.enum_type.name().is_some() || to.enum_type.name().is_some() {
        return from == to;
    }
    implicitly_coercible(from.oid, to.oid)
}

/// One step of `select_common_type`: fold `next` into the running `ptype`.
///
/// `None` means the two have **no** common type (different categories). Transcribed
/// from `parse_coerce.c`:
///
/// ```text
/// if (nexttype != ptype) {
///     if (category differs) error;
///     else if (!pispreferred && implicit(ptype→next) && !implicit(next→ptype))
///         ptype = nexttype;
/// }
/// ```
///
/// The asymmetry is the whole point — see the module docs. Folding must go
/// **left to right** over the expressions in Postgres's own order, or the tie rule
/// picks the wrong winner.
///
/// Two **different enums** take the last line: they share category `E`, neither is
/// preferred and neither converts to the other, so this answers `None` and
/// [`no_common_type`] reads the shared category and gives 18.6's `42846`
/// (*could not convert type m3 to mood*) rather than the `42804` the enum/`text` pair
/// takes. That split is the whole of `EDGES.md` row 115's code, and modelling the
/// **category** is what produces it — a refusal keyed on type identity would give both
/// pairs one code.
#[must_use]
pub fn common_type_step(ptype: &ResolvedType, next: &ResolvedType) -> Option<ResolvedType> {
    if ptype == next {
        return Some(ptype.clone());
    }
    // An undecidable identity neither supplies an enum nor contradicts one: the fold
    // runs on the OIDs and the result keeps whichever side did carry an identity, so a
    // `COALESCE(m, t::mood)` resolves as it did before this rulebook learned about enums.
    if ptype.enum_type.is_undecided() || next.enum_type.is_undecided() {
        let oid = common_type_step_oid(ptype.oid, next.oid)?;
        let carrier = if oid as u32 == ptype.oid as u32 {
            ptype
        } else {
            next
        };
        return Some(ResolvedType {
            oid,
            enum_type: carrier.enum_type.clone(),
        });
    }
    if resolved_category(ptype) != resolved_category(next) {
        return None;
    }
    if !resolved_is_preferred(ptype)
        && resolved_coercible(ptype, next)
        && !resolved_coercible(next, ptype)
    {
        return Some(next.clone());
    }
    // Keeping `ptype` is only legal if `next` can actually convert to it. Sharing a
    // category does **not** guarantee that: `time` and `timestamp` are both category
    // `D` and neither is implicitly coercible to the other. Returning `ptype` anyway was
    // silent-wrong, because `coerce_value(timestamp → time)` is a supported *truncating*
    // cast — so the bogus target quietly succeeded and yielded `10:00:00` from a
    // timestamp instead of the error Postgres raises.
    //
    // **This is now belt to `select_common_type`'s braces, not the load-bearing check.**
    // Every caller that once folded this raw — set-op columns, multi-row `VALUES`, the
    // eval-time `greatest`/`least`/`nullif` fold, and the recursive-CTE type check — goes
    // through `select_common_type`, whose closing loop re-verifies every input against
    // the final target and would catch the same pair. Two steps still differ in *which*
    // pair a ≥3-input fold names, and the guard is a line, so it stays; but a test that
    // reddens when it is removed is testing the wrapper, not this.
    resolved_coercible(next, ptype).then(|| ptype.clone())
}

/// [`common_type_step`] over bare OIDs — the rule as it stood before an enum identity
/// reached it, and what a pair with an [`EnumIdentity::Undecided`] side falls back to.
#[must_use]
fn common_type_step_oid(ptype: TypeOid, next: TypeOid) -> Option<TypeOid> {
    if ptype as u32 == next as u32 {
        return Some(ptype);
    }
    if type_category(ptype) != type_category(next) {
        return None;
    }
    if !is_preferred_type(ptype)
        && implicitly_coercible(ptype, next)
        && !implicitly_coercible(next, ptype)
    {
        return Some(next);
    }
    implicitly_coercible(next, ptype).then_some(ptype)
}

/// Postgres's `select_common_type` over `types`, in source order.
///
/// `types` carries `None` for an **unknown**-typed expression (a bare string literal,
/// a bare `NULL`) — those adopt whatever the known types resolve to and never drive the
/// choice. `Ok(None)` means *every* input was unknown, which Postgres resolves to
/// `text`; the caller decides, because a `CASE` with only unknown branches and an
/// `ARRAY[]` of them differ in what they do next.
///
/// `context` names the construct for the error message, exactly as Postgres does
/// (`COALESCE`, `CASE`, `ARRAY`, …).
///
/// # Errors
///
/// - `42804` (`Error::Type`) when two inputs are in **different categories** —
///   Postgres's "types %s and %s cannot be matched".
/// - `42846` (`Error::TypeCoercion`) when they share a category but some input cannot
///   be implicitly coerced to the resolved type — Postgres's "could not convert type
///   %s to %s". `COALESCE(date_col, time_col)` is the reachable case: both are category
///   `D`, and neither converts to the other.
pub fn select_common_type(
    types: &[Option<ResolvedType>],
    context: &str,
) -> Result<Option<ResolvedType>> {
    let mut ptype: Option<ResolvedType> = None;
    for known in types.iter().flatten() {
        ptype = Some(match ptype {
            None => known.clone(),
            // `common_type_step` answers `None` for **both** failure modes; which one it
            // was is decided here, because only this level knows the construct name the
            // message needs. Different categories are Postgres's "types %s and %s cannot
            // be matched" (`42804`); same category with no conversion is its
            // "could not convert type %s to %s" (`42846`).
            Some(p) => {
                common_type_step(&p, known).ok_or_else(|| no_common_type(context, &p, known))?
            }
        });
    }
    let Some(target) = ptype else {
        return Ok(None);
    };
    // Belt and braces: the fold guarantees each *step* converts, and this confirms it
    // for every input against the final target.
    for known in types.iter().flatten() {
        if !resolved_coercible(known, &target) {
            return Err(no_common_type(context, &target, known));
        }
    }
    Ok(Some(target))
}

/// Postgres's `select_common_typmod` over `sides`, given the common type
/// [`select_common_type`] already chose.
///
/// The `atttypmod` a set operation's resolved column carries, and it is **not** a fold
/// in the sense the type one is: it is the first side's modifier, kept only while every
/// side agrees. A side whose type is not `common` — 18.6 reaches this after coercing the
/// sides, so such a side is one no coercion changed — collapses it, and so does the
/// first disagreement. Everything else is `-1`.
///
/// That asymmetry matters to a caller that **compares** the answer back against one
/// side: `varchar(5)` beside `varchar(8)` folds to `-1` and so does `varchar` beside
/// `varchar(8)`, but only the first is a *change* from its seed's modifier. 18.6's
/// recursive-CTE check is exactly such a caller and raises `42804` on the first alone;
/// pglet's is not — it declares the answer — for the reason
/// `docs/declared_types.md` § A recursive CTE's declared modifier gives.
///
/// **The `oid != common` arm is reachable, and it is what collapses the modifier there.**
/// A side whose type is not the common one carries a modifier in *its own* type's
/// encoding, so agreeing numbers across two types are not an agreement: a
/// `TIMESTAMPTZ(3)` recursive-CTE seed beside a `TIMESTAMP(3)` column in the recursive
/// term is `(timestamptz, 3)` and `(timestamp, 3)` over a common `timestamptz`, and
/// without the arm the loop would answer `3` where this answers `-1`. 18.6 refuses that
/// statement (`42804 … has type timestamp(3) with time zone in non-recursive term but
/// type timestamp with time zone overall`), its coerced side reading `-1`; pglet accepts
/// it and declares this answer, measured on both legs of the native server. Pinned by this module's
/// `select_common_typmod_collapses_a_side_of_another_type` and, through the engine, by
/// `cte_column_list`'s `a_column_typed_recursive_term_folds_what_pglet_reads_exactly`.
#[must_use]
pub fn select_common_typmod(sides: &[(TypeOid, i32)], common: TypeOid) -> i32 {
    let mut result = -1;
    for (i, (oid, typmod)) in sides.iter().enumerate() {
        if *oid != common {
            return -1;
        }
        if i == 0 {
            result = *typmod;
        } else if result != *typmod {
            return -1;
        }
    }
    result
}

/// The error for a pair with no common type, split by Postgres's own two codes.
fn no_common_type(context: &str, ptype: &ResolvedType, next: &ResolvedType) -> Error {
    if resolved_category(ptype) == resolved_category(next) {
        Error::CannotConvertType {
            context: context.to_string(),
            from: next.display_name(),
            to: ptype.display_name(),
        }
    } else {
        Error::Type(format!(
            "{context} types {} and {} cannot be matched",
            ptype.display_name(),
            next.display_name(),
        ))
    }
}

/// [`select_common_type`] over a list of **known** types, discarding the diagnosis.
///
/// The convenience form for callers that only need the target and report their own
/// error (`greatest`/`least` argument unification, the result-type walkers). `None`
/// for an empty list or a set with no common type.
#[must_use]
pub fn variadic_common_type(types: &[ResolvedType]) -> Option<ResolvedType> {
    let owned: Vec<Option<ResolvedType>> = types.iter().cloned().map(Some).collect();
    select_common_type(&owned, "GREATEST").ok().flatten()
}

/// [`variadic_common_type`] over plain OIDs, for the callers that hold no identity to
/// pass — the declared-type walkers, whose answer is the wire OID and for which an
/// enum's is `text` either way.
#[must_use]
pub fn variadic_common_type_oid(types: &[TypeOid]) -> Option<TypeOid> {
    let owned: Vec<ResolvedType> = types.iter().copied().map(ResolvedType::plain).collect();
    variadic_common_type(&owned).map(|t| t.oid)
}

/// The type a comparison operator resolves **both** operands to, for the pairs
/// Postgres registers an implicitly-coerced `=` for.
///
/// This is *not* [`select_common_type`] — it is the operator-resolution answer, and it
/// differs: `int4` vs `float4` compares as `float8` here (there is no `int4 = float4`
/// operator, and `float8 = float8` wins the resolution) while `select_common_type` says
/// `float4`. Used only as [`nullif_result_type`]'s fallback.
#[must_use]
pub fn comparison_operand_type(a: TypeOid, b: TypeOid) -> Option<TypeOid> {
    // The string family resolves to `text` in every pairing — `varchar` has no
    // comparison operator of its own — so this precedes the same-type shortcut, which
    // would otherwise answer `varchar` for two `varchar`s.
    if matches!(a, TypeOid::Text | TypeOid::Varchar)
        && matches!(b, TypeOid::Text | TypeOid::Varchar)
    {
        return Some(TypeOid::Text);
    }
    if a as u32 == b as u32 {
        return Some(a);
    }
    let int_rank = |t| match t {
        TypeOid::Int2 => Some(0u8),
        TypeOid::Int4 => Some(1),
        TypeOid::Int8 => Some(2),
        _ => None,
    };
    if let (Some(ra), Some(rb)) = (int_rank(a), int_rank(b)) {
        return Some(if ra >= rb { a } else { b });
    }
    let temporal_rank = |t| match t {
        TypeOid::Date => Some(0u8),
        TypeOid::Timestamp => Some(1),
        TypeOid::Timestamptz => Some(2),
        _ => None,
    };
    if let (Some(ra), Some(rb)) = (temporal_rank(a), temporal_rank(b)) {
        return Some(if ra >= rb { a } else { b });
    }
    // Postgres implicitly casts `time → interval`, so the pair compares as intervals.
    if matches!(
        (a, b),
        (TypeOid::Time, TypeOid::Interval) | (TypeOid::Interval, TypeOid::Time)
    ) {
        return Some(TypeOid::Interval);
    }
    // Arithmetic's promotion answers for any pair holding a float or a `numeric`, a
    // `text` or a `boolean` partner included; a comparison has an operator only where
    // both operands are in the numeric category. Answering for the other pairs cast
    // `NULLIF`'s first argument to the partner's type ahead of every gate:
    // `nullif(text_col, float8_col)` answered the `text` read as a number where 18.6
    // refuses `42883 operator does not exist: text = double precision`.
    let numeric = |t| {
        matches!(
            t,
            TypeOid::Int2
                | TypeOid::Int4
                | TypeOid::Int8
                | TypeOid::Numeric
                | TypeOid::Float4
                | TypeOid::Float8
        )
    };
    if numeric(a) && numeric(b) {
        super::expr::numeric_family_result(a, b)
    } else {
        None
    }
}

/// Whether Postgres registers an `=` operator whose **left** argument type is exactly
/// `left`, and which accepts `right` after implicit coercion.
///
/// When it does, `NULLIF`'s first argument is passed through untouched and keeps its
/// own type; when it doesn't, argument one is coerced and the result widens with it.
/// The registered cross-type `=` families are the integers (every pairing), the floats
/// (`float4`/`float8` both ways), and the date/timestamp/timestamptz set; everything
/// else has only a same-type operator, reached by coercing the operands to it.
///
/// `varchar` is the subtle one: it has **no** `=` operator of its own (comparison goes
/// through `text`, to which it is binary-coercible), so `NULLIF(varchar_col, text_col)`
/// is `text` — argument one does *not* survive.
#[must_use]
const fn comparison_keeps_left(left: TypeOid, right: TypeOid) -> bool {
    // `varchar` is checked **before** the same-type shortcut, because it has no `=`
    // operator of its own at all: comparison goes through `text`, to which it is
    // binary-coercible. So argument one is coerced even against another `varchar` —
    // `NULLIF(varchar_col, varchar_col)` is `text`, not `character varying`. Ordering
    // this after the shortcut contradicted this function's own documentation.
    if matches!(left, TypeOid::Varchar) {
        return false;
    }
    if left as u32 == right as u32 {
        return true;
    }
    match left {
        // int2/int4/int8 = int2/int4/int8 are all registered.
        TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8 => {
            matches!(right, TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8)
        }
        // float4=float4, float4=float8, float8=float4, float8=float8 are registered,
        // and every other numeric implicitly coerces into one of the float types.
        TypeOid::Float4 | TypeOid::Float8 => {
            matches!(type_category(right), TypeCategory::Numeric)
        }
        // Only numeric=numeric, reachable from the integers (int → numeric is
        // implicit) but not from the floats (float → numeric is assignment-only).
        TypeOid::Numeric => matches!(right, TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8),
        TypeOid::Text => matches!(right, TypeOid::Varchar),
        TypeOid::Date | TypeOid::Timestamp | TypeOid::Timestamptz => matches!(
            right,
            TypeOid::Date | TypeOid::Timestamp | TypeOid::Timestamptz
        ),
        // interval=interval, reachable from `time` (implicit); the reverse is not,
        // so `NULLIF(time_col, interval_col)` is `interval`.
        TypeOid::Interval => matches!(right, TypeOid::Time),
        _ => false,
    }
}

/// The declared type of `NULLIF(a, b)`.
///
/// Postgres builds the `=` `OpExpr`, then overwrites its result type with *argument
/// one's* type **as the operator resolution left it**. So the answer is `a` whenever a
/// registered `=` takes `a` on the left ([`comparison_keeps_left`]), and otherwise the
/// type both operands were coerced to ([`comparison_operand_type`]).
///
/// `None` only when **both** arguments are unknown (`NULLIF('a', 'b')`, which stays
/// text) — the caller then leaves them as they are.
///
/// Worked examples, all measured: `NULLIF(int4, int8)` is `int4` (an `int4 = int8`
/// operator exists), `NULLIF(int4, float4)` is `float8` (none exists, so both go to
/// `float8`), and `NULLIF(float4, int4)` is `float4` (`float4 = float4` takes the
/// coerced right operand). Note the first differs from `COALESCE(int4, int8)`, which is
/// `int8`.
#[must_use]
pub fn nullif_result_type(a: Option<TypeOid>, b: Option<TypeOid>) -> Option<TypeOid> {
    // An **unknown** first argument adopts the second's type — Postgres resolves
    // `unknown = int4` to `int4 = int4` and coerces it, so `NULLIF('a', 1)` is
    // `integer` and fails `22P02` on the coercion rather than reporting no common type.
    let Some(a) = a else {
        return b;
    };
    // An unknown *second* argument adopts the first's type, leaving it untouched.
    let Some(b) = b else {
        return Some(a);
    };
    if comparison_keeps_left(a, b) {
        Some(a)
    } else {
        comparison_operand_type(a, b)
    }
}

/// The input type of the default B-tree operator class 18.6 indexes a key column of this
/// type under (`pg_opclass.opcintype`), and the operator family that class belongs to.
///
/// What a family decides is which **cross-type** equalities exist without a coercion:
/// `integer_ops` holds `=` for every pair of `int2`/`int4`/`int8`, `float_ops` for
/// `float4`/`float8`, `datetime_ops` for `date`/`timestamp`/`timestamptz`, and
/// `text_ops` for `text`/`name` — `varchar` has an opclass in `text_ops`
/// (`varchar_ops`, input type `text`) but **no operator of its own**, so it is no member
/// here. Every other type pglet stores is alone in its family.
///
/// `None` for the two **polymorphic** classes — `enum_ops` (input `anyenum`) and
/// `array_ops` (input `anyarray`) — whose answer [`foreign_key_types_compatible`] takes
/// directly, and for a type with no column form.
const fn btree_opclass(oid: TypeOid) -> Option<(BtreeFamily, TypeOid)> {
    Some(match oid {
        TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8 => (BtreeFamily::Integer, oid),
        TypeOid::Float4 | TypeOid::Float8 => (BtreeFamily::Float, oid),
        TypeOid::Date | TypeOid::Timestamp | TypeOid::Timestamptz => (BtreeFamily::Datetime, oid),
        TypeOid::Text | TypeOid::Name => (BtreeFamily::Text, oid),
        TypeOid::Varchar => (BtreeFamily::Text, TypeOid::Text),
        TypeOid::Bool
        | TypeOid::Bytea
        | TypeOid::Numeric
        | TypeOid::Time
        | TypeOid::Interval
        | TypeOid::Uuid
        | TypeOid::Jsonb => (BtreeFamily::Alone(oid), oid),
        _ => return None,
    })
}

/// A B-tree operator family, as far as [`btree_opclass`] needs to tell them apart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BtreeFamily {
    /// `integer_ops`.
    Integer,
    /// `float_ops`.
    Float,
    /// `datetime_ops`.
    Datetime,
    /// `text_ops`.
    Text,
    /// A family holding one type's operators and no cross-type ones.
    Alone(TypeOid),
}

impl BtreeFamily {
    /// Whether the family registers `=` for `(<its input type>, ty)` **and** for
    /// `(ty, ty)` — 18.6's `pfeqop` and `ffeqop` lookups, which each of the cross-type
    /// families above answers for every pair of its members.
    const fn has_equality_for(self, ty: TypeOid) -> bool {
        match self {
            Self::Integer => matches!(ty, TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8),
            Self::Float => matches!(ty, TypeOid::Float4 | TypeOid::Float8),
            Self::Datetime => matches!(
                ty,
                TypeOid::Date | TypeOid::Timestamp | TypeOid::Timestamptz
            ),
            Self::Text => matches!(ty, TypeOid::Text | TypeOid::Name),
            Self::Alone(only) => ty as u32 == only as u32,
        }
    }
}

/// Whether 18.6 can implement a foreign key pairing a `referencing` column with a
/// `referenced` key column — `ATAddForeignKeyConstraint`'s per-pair test.
///
/// Where this answers `false`, 18.6 refuses the pair `42804 foreign key constraint "…"
/// cannot be implemented`.
///
/// The test is asked of the referenced key's **B-tree operator family**, not of the two
/// types: the pair is implementable when the family holds an equality between its input
/// type and the referencing type and one between the referencing type and itself, or —
/// failing that — when **both** column types coerce *implicitly* to the class's input
/// type, in which case the key's own equality is used. So `bigint` referencing `int` is
/// accepted through `integer_ops`'s cross-type `=`, `int` referencing `numeric` and
/// `varchar` referencing `text` through the implicit coercion, and `numeric` referencing
/// `int` or `text` referencing `int` is refused — no member and no implicit cast. The
/// coercion is one-way, so a pair can be accepted in one direction only: `numeric`
/// referencing `float8` is implementable and `float8` referencing `numeric` is not.
///
/// The polymorphic classes take the coercion half with a polymorphic target, which both
/// column types must resolve to **one** type: an enum key accepts only that enum, and an
/// array key only that same array type — `int4[]` referencing `int8[]` is refused even
/// though `int4[] → int8[]` is an implicit coercion, because `anyarray` cannot be bound to
/// both. An enum is read by its identity, never its storage OID (`text`), so `mood`
/// referencing `text`, `text` referencing `mood` and `mood` referencing `mood2` are each
/// refused.
///
/// 18.6's collation test beside this one is not modelled: pglet has no
/// nondeterministic collation to fail it.
#[must_use]
pub fn foreign_key_types_compatible(referencing: &ResolvedType, referenced: &ResolvedType) -> bool {
    let referenced_is_enum = matches!(referenced.enum_type, EnumIdentity::Named(_));
    if referenced_is_enum || referenced.oid.array_element().is_some() {
        return referencing == referenced;
    }
    let Some((family, input)) = btree_opclass(referenced.oid) else {
        return false;
    };
    let referencing_is_enum = matches!(referencing.enum_type, EnumIdentity::Named(_));
    if !referencing_is_enum && family.has_equality_for(referencing.oid) {
        return true;
    }
    let input = ResolvedType::plain(input);
    resolved_coercible(referenced, &input) && resolved_coercible(referencing, &input)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole pglet column-type matrix against 18.6's verdicts: every scalar column
    /// type, its array, and two enums, on both sides of a foreign key. 18.6 was measured
    /// over all of them (`CREATE TABLE p (k <referenced> PRIMARY KEY)`, `CREATE TABLE c
    /// (k <referencing> REFERENCES p)`), and it accepts each identical pair and exactly
    /// the cross-type pairs listed — every other pair is `42804`. `char(n)` is left out
    /// because pglet stores it as `varchar` (`EDGES.md` row 95).
    #[test]
    fn foreign_key_types_compatible_matches_the_measured_matrix() {
        let scalars = [
            TypeOid::Bool,
            TypeOid::Int2,
            TypeOid::Int4,
            TypeOid::Int8,
            TypeOid::Float4,
            TypeOid::Float8,
            TypeOid::Numeric,
            TypeOid::Text,
            TypeOid::Varchar,
            TypeOid::Bytea,
            TypeOid::Timestamp,
            TypeOid::Timestamptz,
            TypeOid::Date,
            TypeOid::Time,
            TypeOid::Interval,
            TypeOid::Jsonb,
            TypeOid::Uuid,
        ];
        let mut types: Vec<ResolvedType> =
            scalars.iter().map(|&t| ResolvedType::plain(t)).collect();
        for &t in &scalars {
            let array = t
                .array_of()
                .expect("every scalar column type has an array type");
            types.push(ResolvedType::plain(array));
        }
        types.push(ResolvedType::new(TypeOid::Text, Some("mood".to_string())));
        types.push(ResolvedType::new(TypeOid::Text, Some("mood2".to_string())));
        // (referenced, referencing), each accepted by 18.6.
        let cross: &[(TypeOid, TypeOid)] = &[
            (TypeOid::Int2, TypeOid::Int4),
            (TypeOid::Int2, TypeOid::Int8),
            (TypeOid::Int4, TypeOid::Int2),
            (TypeOid::Int4, TypeOid::Int8),
            (TypeOid::Int8, TypeOid::Int2),
            (TypeOid::Int8, TypeOid::Int4),
            (TypeOid::Float4, TypeOid::Int2),
            (TypeOid::Float4, TypeOid::Int4),
            (TypeOid::Float4, TypeOid::Int8),
            (TypeOid::Float4, TypeOid::Float8),
            (TypeOid::Float4, TypeOid::Numeric),
            (TypeOid::Float8, TypeOid::Int2),
            (TypeOid::Float8, TypeOid::Int4),
            (TypeOid::Float8, TypeOid::Int8),
            (TypeOid::Float8, TypeOid::Float4),
            (TypeOid::Float8, TypeOid::Numeric),
            (TypeOid::Numeric, TypeOid::Int2),
            (TypeOid::Numeric, TypeOid::Int4),
            (TypeOid::Numeric, TypeOid::Int8),
            (TypeOid::Text, TypeOid::Varchar),
            (TypeOid::Varchar, TypeOid::Text),
            (TypeOid::Date, TypeOid::Timestamp),
            (TypeOid::Date, TypeOid::Timestamptz),
            (TypeOid::Timestamp, TypeOid::Date),
            (TypeOid::Timestamp, TypeOid::Timestamptz),
            (TypeOid::Timestamptz, TypeOid::Date),
            (TypeOid::Timestamptz, TypeOid::Timestamp),
            (TypeOid::Interval, TypeOid::Time),
        ];
        let mut wrong = Vec::new();
        for referenced in &types {
            for referencing in &types {
                let expected = referenced == referencing
                    || (referenced.enum_type.name().is_none()
                        && referencing.enum_type.name().is_none()
                        && cross.iter().any(|&(pk, fk)| {
                            pk as u32 == referenced.oid as u32
                                && fk as u32 == referencing.oid as u32
                        }));
                if foreign_key_types_compatible(referencing, referenced) != expected {
                    wrong.push(format!(
                        "{} referencing {}: 18.6 {}",
                        referencing.display_name(),
                        referenced.display_name(),
                        if expected { "accepts" } else { "refuses" }
                    ));
                }
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Every row here was read off the live oracle (`pg_typeof(COALESCE(…))` over the
    /// full type matrix). They are the reason this module exists: the previous rule was
    /// a symmetric width fold, and four of these rows contradict it.
    #[test]
    fn select_common_type_matches_postgres() {
        let cases: &[(TypeOid, TypeOid, TypeOid)] = &[
            // The integer ladder — widen, order-independent.
            (TypeOid::Int2, TypeOid::Int4, TypeOid::Int4),
            (TypeOid::Int4, TypeOid::Int2, TypeOid::Int4),
            (TypeOid::Int4, TypeOid::Int8, TypeOid::Int8),
            (TypeOid::Int8, TypeOid::Int4, TypeOid::Int8),
            // float4 BEATS int and numeric here (int→float4 is implicit, and the
            // reverse is not) — the opposite of the arithmetic promotion.
            (TypeOid::Int4, TypeOid::Float4, TypeOid::Float4),
            (TypeOid::Int8, TypeOid::Float4, TypeOid::Float4),
            (TypeOid::Float4, TypeOid::Int4, TypeOid::Float4),
            (TypeOid::Numeric, TypeOid::Float4, TypeOid::Float4),
            (TypeOid::Float4, TypeOid::Numeric, TypeOid::Float4),
            // float8 is preferred and is never displaced.
            (TypeOid::Float8, TypeOid::Numeric, TypeOid::Float8),
            (TypeOid::Float8, TypeOid::Int4, TypeOid::Float8),
            (TypeOid::Float4, TypeOid::Float8, TypeOid::Float8),
            (TypeOid::Numeric, TypeOid::Int4, TypeOid::Numeric),
            (TypeOid::Int4, TypeOid::Numeric, TypeOid::Numeric),
            // A mutual-implicit tie keeps the FIRST — the observable `CASE` ordering.
            (TypeOid::Text, TypeOid::Varchar, TypeOid::Text),
            (TypeOid::Varchar, TypeOid::Text, TypeOid::Varchar),
            // `name` ties with `text` both ways, so the first wins there too; against
            // `varchar` it wins from **either** side, `varchar → name` being implicit
            // while `name → varchar` is assignment. `name[]` follows element-wise.
            (TypeOid::Name, TypeOid::Text, TypeOid::Name),
            (TypeOid::Text, TypeOid::Name, TypeOid::Text),
            (TypeOid::Name, TypeOid::Varchar, TypeOid::Name),
            (TypeOid::Varchar, TypeOid::Name, TypeOid::Name),
            (TypeOid::NameArray, TypeOid::TextArray, TypeOid::NameArray),
            (TypeOid::TextArray, TypeOid::NameArray, TypeOid::TextArray),
            // Temporal widening.
            (TypeOid::Date, TypeOid::Timestamp, TypeOid::Timestamp),
            (TypeOid::Timestamp, TypeOid::Date, TypeOid::Timestamp),
            (TypeOid::Date, TypeOid::Timestamptz, TypeOid::Timestamptz),
            (
                TypeOid::Timestamptz,
                TypeOid::Timestamp,
                TypeOid::Timestamptz,
            ),
        ];
        for &(a, b, want) in cases {
            let got = select_common_type(
                &[Some(ResolvedType::plain(a)), Some(ResolvedType::plain(b))],
                "COALESCE",
            )
            .unwrap_or_else(|e| panic!("COALESCE({a:?}, {b:?}) errored: {e}"));
            assert_eq!(
                got,
                Some(ResolvedType::plain(want)),
                "COALESCE({a:?}, {b:?})"
            );
        }
    }

    /// The two failure codes are different facts and Postgres reports them apart.
    #[test]
    fn no_common_type_distinguishes_category_from_conversion() {
        // Different categories — "types X and Y cannot be matched".
        let plain = |a: TypeOid, b: TypeOid| {
            select_common_type(
                &[Some(ResolvedType::plain(a)), Some(ResolvedType::plain(b))],
                "COALESCE",
            )
        };
        let e = plain(TypeOid::Int4, TypeOid::Text)
            .expect_err("int4 and text are different categories");
        assert_eq!(e.sqlstate(), "42804", "{e}");
        let e = plain(TypeOid::Bool, TypeOid::Int4)
            .expect_err("bool and int4 are different categories");
        assert_eq!(e.sqlstate(), "42804", "{e}");
        // Same category (`D`), but neither converts to the other.
        let e = plain(TypeOid::Date, TypeOid::Time)
            .expect_err("date and time are both category D but do not convert");
        assert_eq!(e.sqlstate(), "42846", "{e}");
        // Same category (`U`).
        let e =
            plain(TypeOid::Uuid, TypeOid::Bytea).expect_err("uuid and bytea are both category U");
        assert_eq!(e.sqlstate(), "42846", "{e}");
    }

    /// Unknown-typed inputs never drive the choice, and an all-unknown set resolves to
    /// nothing here (the caller applies Postgres's `text` default).
    #[test]
    fn unknown_inputs_adopt_the_known_type() {
        assert_eq!(
            select_common_type(
                &[None, Some(ResolvedType::plain(TypeOid::Int4)), None],
                "COALESCE"
            )
            .unwrap(),
            Some(ResolvedType::plain(TypeOid::Int4))
        );
        assert_eq!(select_common_type(&[None, None], "COALESCE").unwrap(), None);
        assert_eq!(select_common_type(&[], "COALESCE").unwrap(), None);
    }

    /// `NULLIF` is the other rulebook, and it disagrees with the one above on the very
    /// first row. Every case measured against the live oracle.
    #[test]
    fn nullif_takes_argument_ones_type_after_operator_resolution() {
        let cases: &[(TypeOid, TypeOid, TypeOid)] = &[
            // A registered cross-type `=` leaves argument one alone. Contrast
            // `COALESCE(int4, int8)`, which is `int8`.
            (TypeOid::Int4, TypeOid::Int8, TypeOid::Int4),
            (TypeOid::Int8, TypeOid::Int4, TypeOid::Int8),
            (TypeOid::Int2, TypeOid::Int8, TypeOid::Int2),
            (TypeOid::Float4, TypeOid::Float8, TypeOid::Float4),
            (TypeOid::Float8, TypeOid::Float4, TypeOid::Float8),
            (TypeOid::Date, TypeOid::Timestamptz, TypeOid::Date),
            (TypeOid::Timestamp, TypeOid::Timestamptz, TypeOid::Timestamp),
            // No `int = float` operator: both operands go to float8, and argument one
            // widens with them. This is the direction `COALESCE` disagrees on.
            (TypeOid::Int4, TypeOid::Float4, TypeOid::Float8),
            (TypeOid::Int2, TypeOid::Float4, TypeOid::Float8),
            (TypeOid::Int8, TypeOid::Float8, TypeOid::Float8),
            // ...but a float on the left survives any numeric on the right.
            (TypeOid::Float4, TypeOid::Int8, TypeOid::Float4),
            (TypeOid::Float4, TypeOid::Numeric, TypeOid::Float4),
            // numeric = numeric is reachable from the integers, not from the floats.
            (TypeOid::Numeric, TypeOid::Int4, TypeOid::Numeric),
            (TypeOid::Int4, TypeOid::Numeric, TypeOid::Numeric),
            (TypeOid::Numeric, TypeOid::Float4, TypeOid::Float8),
            // varchar has no `=` of its own, so argument one does NOT survive.
            (TypeOid::Text, TypeOid::Varchar, TypeOid::Text),
            (TypeOid::Varchar, TypeOid::Text, TypeOid::Text),
            // time → interval is implicit; the reverse is not.
            (TypeOid::Interval, TypeOid::Time, TypeOid::Interval),
            (TypeOid::Time, TypeOid::Interval, TypeOid::Interval),
        ];
        for &(a, b, want) in cases {
            assert_eq!(
                nullif_result_type(Some(a), Some(b)),
                Some(want),
                "NULLIF({a:?}, {b:?})"
            );
        }
        // An unknown argument on either side adopts the other's type: Postgres
        // resolves the `=` against the known side and coerces the literal, which is
        // why `NULLIF('a', 1)` is `22P02` (a failed int coercion) and not `42804`.
        assert_eq!(
            nullif_result_type(Some(TypeOid::Int4), None),
            Some(TypeOid::Int4)
        );
        assert_eq!(
            nullif_result_type(None, Some(TypeOid::Int4)),
            Some(TypeOid::Int4)
        );
        // Both unknown — nothing to resolve against.
        assert_eq!(nullif_result_type(None, None), None);
        // A pair with no `=` operator has no type either, in each order: a float or a
        // `numeric` beside an operand outside the numeric category. Arithmetic's
        // promotion answers for such a pair, and an answer here cast argument one to the
        // partner's type before any gate read the pair.
        for outside in [
            TypeOid::Text,
            TypeOid::Varchar,
            TypeOid::Bool,
            TypeOid::Date,
            TypeOid::Uuid,
            TypeOid::Int4Array,
        ] {
            for numeric in [TypeOid::Float8, TypeOid::Float4, TypeOid::Numeric] {
                assert_eq!(
                    nullif_result_type(Some(outside), Some(numeric)),
                    None,
                    "NULLIF({outside:?}, {numeric:?})"
                );
                assert_eq!(
                    comparison_operand_type(numeric, outside),
                    None,
                    "{numeric:?} = {outside:?}"
                );
            }
        }
    }

    /// 18.6's `select_common_typmod` loop, arm by arm. The first assertion is the one a
    /// loop that compared the modifiers alone would fail: two sides agreeing on the
    /// *number* `3` over two different types are not agreeing on a modifier, because
    /// each number is in its own type's encoding.
    #[test]
    fn select_common_typmod_collapses_a_side_of_another_type() {
        assert_eq!(
            select_common_typmod(
                &[(TypeOid::Timestamptz, 3), (TypeOid::Timestamp, 3)],
                TypeOid::Timestamptz,
            ),
            -1,
            "a side whose type is not the common one collapses the modifier"
        );
        assert_eq!(
            select_common_typmod(
                &[(TypeOid::Timestamptz, 3), (TypeOid::Timestamptz, 3)],
                TypeOid::Timestamptz,
            ),
            3,
            "the control: the same pair over one type keeps it"
        );
        // The comparison arms: a disagreement, in either order and against an
        // undeclared side, is `-1`.
        for sides in [
            [(TypeOid::Varchar, 9), (TypeOid::Varchar, 12)],
            [(TypeOid::Varchar, 12), (TypeOid::Varchar, 9)],
            [(TypeOid::Varchar, -1), (TypeOid::Varchar, 9)],
            [(TypeOid::Varchar, 9), (TypeOid::Varchar, -1)],
        ] {
            assert_eq!(
                select_common_typmod(&sides, TypeOid::Varchar),
                -1,
                "{sides:?}"
            );
        }
    }
}
