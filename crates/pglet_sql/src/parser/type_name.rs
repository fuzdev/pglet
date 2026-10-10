//! Column / cast type-name normalization.
//!
//! Parses a type spelling into Postgres's **canonical** normalized string — `INT`
//! → `integer`, `INT[]` → `integer[]`, `TIMESTAMPTZ` → `timestamptz`, `SERIAL` →
//! `serial`, `VARCHAR(255)` → `varchar(255)`, … — so `ColumnDef.type_name` holds a
//! stable canonical spelling (the differential fixtures check it byte-for-byte vs
//! real Postgres). An unrecognized type passes through lowercased (a custom/enum
//! name, or an unsupported type that later fails loud at resolution).

use crate::lexer::{Keyword, TokenKind};
use crate::{Error, Result};

use super::Parser;

/// One `'(' Iconst ')'` type modifier as read — the window
/// [`Parser::opt_precision`] consumes.
///
/// Three facts about the same number, because three readers want different ones: a
/// canonical name carries the **text**, `opt_float`'s selector switches on
/// the **value**, and a refusal points `P` at the **position**.
struct IconstModifier {
    /// The number as [`crate::lexer::Token::text`] reads it — the source slice unless
    /// the lexer built a decoded payload, which it does for an underscored spelling
    /// (`1_0` → `10`) and a radix one (`0x3` → `3`). Not canonicalised beyond that: a
    /// leading-zero `timestamp(03)` carries `03` in the canonical name. What a client
    /// reads is rebuilt from the reparsed number instead —
    /// `executor::type_resolution`'s `decompose_type_name` takes the name apart, and
    /// `atttypmod`, `information_schema`, the dump and `deparse`'s `Catalog`
    /// renderings are each built from that number rather than from this text. Measured
    /// on both engines: `timestamp(03)` and `timestamp(0x3)` report `atttypmod` and
    /// `datetime_precision` `3`, dump as `timestamp(3)`, and render
    /// `timestamp(3) without time zone` inside a stored `DEFAULT`. And
    /// `modifier_refusal_name` prints the base alone.
    text: String,
    /// The same number parsed — an `Iconst` is an unsigned integer that fits `int32`,
    /// which is what the read already proved.
    value: i32,
    /// 18.6's `P` for the number itself: a 1-based codepoint index, where
    /// `parser_errposition(@2)` points.
    position: u32,
}

/// Append a canonical name's modifier — **after** the whole name, including a
/// multi-word one. See [`Parser::timestamp_or_time_name`] for why the position is a
/// correctness decision rather than a formatting one.
fn with_precision(name: &str, precision: Option<&str>) -> String {
    precision.map_or_else(|| name.to_string(), |p| format!("{name}({p})"))
}

impl Parser<'_> {
    /// Parse a type name (scalar + optional `[]` array suffix), normalized to
    /// Postgres's canonical spelling (see the module doc). The DDL / `CAST` entry point.
    pub(super) fn parse_type_name(&mut self) -> Result<String> {
        let element = self.parse_scalar_type_name()?;
        // One or more `[]` array suffixes collapse to a single level (pglet is 1-D,
        // and the canonical spelling emits one `[]`); an optional dimension is ignored.
        let mut is_array = false;
        while self.ts.at(&TokenKind::LBracket) {
            self.ts.bump()?;
            if matches!(self.ts.peek_kind(), TokenKind::Number) {
                self.ts.bump()?;
            }
            self.ts.expect(&TokenKind::RBracket)?;
            is_array = true;
        }
        if is_array {
            Ok(format!("{element}[]"))
        } else {
            Ok(element)
        }
    }

    /// Parse a scalar (non-array) type name, normalized to Postgres's canonical
    /// spelling.
    pub(super) fn parse_scalar_type_name(&mut self) -> Result<String> {
        let word = self.expect_type_word()?;
        let canonical = match word.as_str() {
            "double" => {
                self.expect_ident("precision")?;
                return Ok("double precision".to_string());
            }
            "character" => {
                let base = if self.eat_ident("varying")? {
                    "varchar"
                } else {
                    "char"
                };
                return self.with_iconst_modifier(base);
            }
            "varchar" => return self.with_iconst_modifier("varchar"),
            "char" => return self.with_iconst_modifier("char"),
            "timestamp" | "timestamptz" | "time" | "timetz" => {
                return self.timestamp_or_time_name(&word);
            }
            // `interval` reads its precision through the **same** grammar as the
            // three above rather than through [`Self::passthrough_type`], so one
            // predicate decides the modifier for all four types that carry one:
            // exactly one `int32` number. The pass-through took a comma list of
            // signed numbers, which accepted `interval(3,1)`, `interval(-1)` and
            // `interval(99999999999999)` — each a `42601` on 18.6
            // (`ConstInterval '(' Iconst ')'`, one `Iconst`), measured. A **range**
            // spec (`interval day to second`, `interval second(0)`) still has no
            // grammar here and stays `42601`; only the `(p)` window is read.
            "interval" => return self.with_iconst_modifier("interval"),
            "int" | "integer" => "integer",
            "smallint" => "smallint",
            "bigint" => "bigint",
            "real" => "real",
            "boolean" => "boolean",
            "text" => "text",
            "bytea" => "bytea",
            // `DEC` is SQL's abbreviation for `DECIMAL`; 18.6 accepts it wherever
            // `decimal` is accepted (`1::dec`, `dec(4,1) '1.55'`), and the resolver
            // keys on the spelled-out name.
            "dec" => return self.passthrough_type("decimal"),
            // `json` is not a pglet type — it fails loud at resolution — but it keeps an
            // arm rather than falling through to `passthrough_type`, and what the two
            // spellings decide is a **modifier**. `json` is a `COL_NAME_KEYWORD` on 18.6
            // (`kwlist.h`), so `json(3)` is `42601` there at the column, `ADD COLUMN`,
            // `::`, `CAST` and typed-constant spellings alike; reading the name and leaving
            // the `(` unread answers `42601` here at the two that resolve *after* parsing —
            // a `CREATE TABLE` / `ADD COLUMN` column — where the pass-through would consume
            // the modifier and answer `42704`. The `::`/`CAST` positions resolve inline, so
            // they answer `42704` either way; that one is a residual, not a choice. The
            // fifth of 18.6's five spellings never reaches this reader at all: `json` is
            // not an `is_multiword_type_lead` word, so `try_typed_constant` declines the
            // `json ( 3 ) '…'` window and `SELECT json(3) '{}'` falls to the function-call
            // path for pglet's own `0A000 function not supported: json` — the shape's
            // answer rather than `json`'s, since `point(3) '{}'` and `nosuchfn(3) '{}'`
            // read the same here. Each measured on both engines.
            "json" => "json",
            "jsonb" => "jsonb",
            "uuid" => "uuid",
            // `float`'s modifier **selects** the type instead of narrowing one, which
            // is a different rule from every other `(…)` here — see
            // [`Self::float_type_name`].
            "float" => return self.float_type_name(),
            // Pass-through (the canonical catch-all): `int2`/`int4`/`int8`,
            // `float4`/`float8`, `bool`, the `serial` family, `numeric`/`decimal`,
            // `date`, and custom/enum type names — all lowercased, precision verbatim.
            _ => return self.passthrough_type(&word),
        };
        Ok(canonical.to_string())
    }

    /// Parse `TIMESTAMP`/`TIME` with an optional precision and an optional
    /// `WITH`/`WITHOUT TIME ZONE`, normalized to the canonical spelling.
    ///
    /// **The precision is read where 18.6 reads it and written where pglet's own
    /// decomposition can find it** — those are two different places.
    /// `ConstDatetime` puts the `(p)` between the word and the time-zone clause
    /// (`TIMESTAMP '(' Iconst ')' opt_timezone`), and 18.6's `format_type` prints it
    /// there too; but `executor::type_resolution`'s `decompose_type_name` cuts on the
    /// **first** `(`, so a canonical `time(3) with time zone` would decompose to base
    /// `time` and build a `time(3)` column out of a type this engine does not support.
    /// The canonical spelling therefore carries the modifier **last** —
    /// `time with time zone(3)` — which keeps
    /// `42704 type "time with time zone" is not supported` the answer at `timetz(3)`
    /// and `time(3) with time zone` alike, as it already is at their parenless
    /// spellings (all four measured). 18.6's mid-name spelling is a *rendering*, and
    /// `deparse::cast_type_spelling` is where it is reproduced.
    fn timestamp_or_time_name(&mut self, word: &str) -> Result<String> {
        let precision = self.opt_precision()?.map(|m| m.text);
        let mut tz = matches!(word, "timestamptz" | "timetz");
        if self.ts.eat_kw(Keyword::With)? {
            self.expect_ident("time")?;
            self.expect_ident("zone")?;
            tz = true;
        } else if self.eat_ident("without")? {
            self.expect_ident("time")?;
            self.expect_ident("zone")?;
            tz = false;
        }
        let base = if word.starts_with("timestamp") {
            "timestamp"
        } else {
            "time"
        };
        let name = match (base, tz) {
            ("timestamp", true) => "timestamptz",
            ("timestamp", false) => "timestamp",
            // `time with time zone` is the unsupported one (it fails loud at
            // resolution, `42704 type "time with time zone" is not supported`), so this
            // spelling only needs to not leave dangling tokens. Bare `time` below is a
            // full column type.
            ("time", true) => "time with time zone",
            _ => "time",
        };
        Ok(with_precision(name, precision.as_deref()))
    }

    /// A type whose modifier is one `Iconst`, read through [`Self::opt_precision`] and
    /// appended verbatim to the canonical name: `char`/`varchar`'s **length** and
    /// `interval`'s sub-second **precision**.
    ///
    /// The two families share this reader because they share the grammar —
    /// `CharacterWithLength: character '(' Iconst ')'` and
    /// `SimpleTypename: ConstInterval '(' Iconst ')'` — not because they share a
    /// meaning. What the number then means is the resolver's, and the two answers are
    /// opposite: a length outside `1 ..= 10485760` is **refused** where a precision
    /// above `6` is **clamped** (`executor::type_resolution`).
    ///
    /// Reading the length through a *weaker* window than `Iconst` is what made
    /// `char(3.5)` and `char(2147483648)` resolve to an unconstrained `varchar`
    /// instead of the `42601` both engines owe them — the number has to be proved an
    /// `int32` here, or the resolver's own refusal fires under the wrong SQLSTATE.
    fn with_iconst_modifier(&mut self, base: &str) -> Result<String> {
        let modifier = self.opt_precision()?;
        Ok(with_precision(
            base,
            modifier.as_ref().map(|m| m.text.as_str()),
        ))
    }

    /// `FLOAT` / `FLOAT(n)` — the one modifier this reader takes that **selects** a
    /// type rather than narrowing one.
    ///
    /// 18.6 decides it in the grammar (`gram.y`'s `opt_float`) and not in a type's
    /// `typmodin`: `float4` for `n` in 1–24, `float8` for 25–53, and `22023` at both
    /// ends — all while reducing `FLOAT_P opt_float`, and therefore **before any name
    /// resolution**. Raising it here, at parse time, is what makes the positions that
    /// resolve no type name at all agree: `CREATE SEQUENCE s AS FLOAT(0)` and an
    /// identity column's `(AS FLOAT(0))` are this `22023` on both engines, where their
    /// `FLOAT(1)` spellings part for reasons of the position rather than the name
    /// (`EDGES.md` row 116).
    ///
    /// The canonical spelling is the **selected** type's, so this arm has exactly the
    /// outputs `opt_float`'s `SystemTypeName` calls name — `real` and
    /// `double precision`, each already emitted verbatim by its own arm above and
    /// pinned differentially there. A bare `FLOAT` takes `double precision` too, as
    /// the `/*EMPTY*/` production takes `float8`, rather than keeping a third spelling
    /// that means the same type. Neither output carries a `(…)`, so
    /// `executor::type_resolution`'s first-`(` cut — the trap the temporal precisions
    /// route around by carrying theirs last — has nothing to read here.
    ///
    /// The grammar is `opt_precision`'s: one `Iconst`. That is 18.6's spelling too
    /// (`opt_float: '(' Iconst ')'`), so `float(1.5)`, `float(1,2)` and
    /// `float(2147483648)` are `42601` on both engines with nothing further to decide.
    fn float_type_name(&mut self) -> Result<String> {
        let Some(modifier) = self.opt_precision()? else {
            return Ok("double precision".to_string());
        };
        let position = modifier.position;
        match modifier.value {
            // 18.6's `if ($2 < 1)`. Only `0` can reach it — a leading `-` is a
            // different token and `expect_number_text_at` refuses it first. Both
            // refusals are `opt_float`'s own, in `gram.y`.
            ..=0 => Err(self.before_analysis(Error::FloatPrecisionTooSmall { position })),
            1..=24 => Ok("real".to_string()),
            25..=53 => Ok("double precision".to_string()),
            _ => Err(self.before_analysis(Error::FloatPrecisionTooLarge { position })),
        }
    }

    /// Consume an optional `'(' Iconst ')'` precision, returning what was read.
    ///
    /// **One number, and it must be an `Iconst`.** Every window this reader serves is
    /// spelled with a single `Iconst` in `gram.y`, which is the reason they can share
    /// it; naming them rather than counting them, they are `ConstDatetime`'s
    /// `'(' Iconst ')'` (through [`Self::timestamp_or_time_name`]),
    /// `SimpleTypename`'s `ConstInterval '(' Iconst ')'` and
    /// `CharacterWithLength`'s `character '(' Iconst ')'` (both through
    /// [`Self::with_iconst_modifier`]), and `opt_float`'s `'(' Iconst ')'`
    /// (through [`Self::float_type_name`]).
    ///
    /// An `ICONST` token is an unsigned integer that fits `int32`, which is why
    /// `timestamp(3,1)`, `timestamp(3.5)`, `timestamp(-1)`, `timestamp(2147483648)` and
    /// the same four spellings of `char(n)` are each `42601 syntax error` on 18.6, all
    /// measured. Expecting `)` after one number covers the comma; the `i32` parse covers
    /// the other three.
    ///
    /// What the number then *means* is the caller's, and the callers disagree: a
    /// temporal type carries it as a modifier **clamped** to `6`, a character length is
    /// carried and **refused** outside `1 ..= 10485760`, and
    /// [`Self::float_type_name`] spends it selecting a type. Only the last is decided
    /// here — 18.6 decides the other two in the type's own modifier reader
    /// (`anytimestamp_typmodin` / `anychar_typmodin`), and so does pglet
    /// (`executor::resolve_column_type`).
    fn opt_precision(&mut self) -> Result<Option<IconstModifier>> {
        if !self.ts.eat(&TokenKind::LParen)? {
            return Ok(None);
        }
        let (text, position) = self.expect_number_text_at()?;
        let Ok(value) = text.parse::<i32>() else {
            // The number is already consumed, so the reported position is the token
            // *after* it — one past where 18.6 points. The message names the number to
            // make up for it, though not always the *source slice*: the lexer has
            // already decoded the payload, so `char(0xFFFFFFFF)` names `"4294967295"`
            // where 18.6's `syntax error at or near "0xFFFFFFFF"` quotes what was
            // written (measured). The SQLSTATE is `42601` either way, which is what a
            // fixture compares — none carries `@expect-message` here.
            return Err(self.ts.error(&format!(
                "type modifier must be an integer that fits int4, found \"{text}\""
            )));
        };
        self.ts.expect(&TokenKind::RParen)?;
        Ok(Some(IconstModifier {
            text,
            value,
            position,
        }))
    }

    /// A pass-through type: the lowercased base name plus a verbatim length/precision
    /// `(...)` if present (`int4`, `serial`, `numeric(10,2)`, `numeric(5,-2)`, a
    /// custom enum name). The modifier reader accepts a leading `-` so a negative
    /// `NUMERIC` scale (`numeric(5,-2)`, which rounds to hundreds) survives to the
    /// resolver; a stray sign on a non-numeric type is a resolution-time concern.
    fn passthrough_type(&mut self, word: &str) -> Result<String> {
        if self.ts.eat(&TokenKind::LParen)? {
            let nums = self.comma_list(Self::expect_signed_number_text)?;
            self.ts.expect(&TokenKind::RParen)?;
            Ok(format!("{word}({})", nums.join(",")))
        } else {
            Ok(word.to_string())
        }
    }

    /// Consume a type-name word — a **bare** identifier only — returning it
    /// downcased, the same fold every other unquoted name position applies.
    ///
    /// A `"…"`-quoted type name is a `42601` here where 18.6 reads it as an ordinary
    /// name (`SELECT 1::"int4"` is `1` there, `SELECT 1::"INT"` is `42704`). Closing
    /// that is **not** just a matter of admitting the token: the names below are a
    /// mix of real `pg_type` entries and spellings that exist only as grammar
    /// keywords, and Postgres resolves a quoted name against `pg_type` alone —
    /// `1::"numeric"` and `1::"bpchar"` resolve on 18.6 while `1::"integer"`,
    /// `1::"int"`, `1::"boolean"`, `1::"decimal"`, `1::"serial"`,
    /// `1::"double precision"` and `1::"character varying"` are each
    /// `42704 type "…" does not exist`. Admitting the token without splitting those
    /// two sets would trade one over-acceptance for a dozen, in the direction this
    /// fold exists to close. It needs a typname table `type_oid_from_name` does not
    /// have.
    ///
    /// `regclass` is one more entry in the first set and reaches this reader the same
    /// way: `SELECT 't'::"regclass"` is `42601` here against `t` there, measured over a
    /// table named `t`. The refusal happens before any type name is read, so
    /// [`Parser::parse_cast_target`]'s canonical-name branch never sees the spelling —
    /// `EDGES.md` row 118 is the record.
    fn expect_type_word(&mut self) -> Result<String> {
        let tok = self.ts.advance()?;
        if matches!(tok.kind, TokenKind::Identifier) {
            Ok(self.ts.name(&tok))
        } else {
            Err(self.ts.expected("a type name"))
        }
    }

    /// Consume a numeric-literal token, returning its source text.
    fn expect_number_text(&mut self) -> Result<String> {
        self.expect_number_text_at().map(|(text, _)| text)
    }

    /// Consume a numeric-literal token, returning its source text **and** 18.6's `P`
    /// for it — the position has to be taken from the token, since reading it moves
    /// the cursor past the thing a refusal must point at.
    fn expect_number_text_at(&mut self) -> Result<(String, u32)> {
        let tok = self.ts.advance()?;
        if matches!(tok.kind, TokenKind::Number) {
            Ok((self.ts.text(&tok).to_string(), self.ts.token_position(&tok)))
        } else {
            Err(self.ts.expected("a number"))
        }
    }

    /// Consume an optional leading `-` then a numeric-literal token (a type
    /// modifier), returning the possibly-signed source text — `NUMERIC`'s scale may
    /// be negative (`numeric(5,-2)` rounds to hundreds).
    fn expect_signed_number_text(&mut self) -> Result<String> {
        if self.ts.eat(&TokenKind::Minus)? {
            Ok(format!("-{}", self.expect_number_text()?))
        } else {
            self.expect_number_text()
        }
    }
}
