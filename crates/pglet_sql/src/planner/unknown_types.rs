//! Opt-in toggle for coercing unsupported SQL types to `TEXT`.
//!
//! By default the type resolvers (the native parser's `CAST`/`::`
//! path and `executor::type_resolution::type_name_to_oid` for DDL) fail loud
//! with `Error::UnsupportedType` for a type name they do not model, rather than
//! silently storing the value as
//! `TEXT`: silent coercion violates the
//! "correct-or-loud, never silently-wrong" principle directly. Which names those are is
//! `docs/compatibility.md`'s and `PLAN.md` § Non-goals' to carry, and not
//! this module's: a second copy of the set goes stale the moment a type lands, and
//! nothing reads this one, so nothing catches that.
//!
//! Setting the flag selects the lenient behavior instead: an unknown type name coerces to
//! `TEXT`.
//!
//! # What the flag does **not** reach: a `(…)` modifier
//!
//! The flag decides what an unrecognized type **name** resolves to. It says nothing
//! about a modifier's legality, which `executor::resolve_column_type` decides from the
//! **resolved** type — and the type this flag resolves to is `text`, which carries
//! none. So a modifier the **resolver still sees** on an unknown name is refused with
//! the flag on as with it off — but under a *different code* each way, and the code is
//! the half a summary drops: `42601 type modifier is not allowed for type "…"` with the
//! flag on, and the name's own `42704 type "…" is not supported` with it off, resolution
//! failing before the gate is reached. `varchar2(10)`, `nvarchar(10)`, `nosuchtype(3)`
//! and `regclass(3)` read that way at a cast and at a column alike (measured, both codes
//! at both positions), while a
//! modifier-less `varchar2` / `nvarchar` / `nosuchtype` / `regclass` **column** still
//! resolves to `text` and still loads (measured). At a *cast* the bare `regclass` is a
//! different route entirely — the relation-name intercept of `EDGES.md` row 118 — and
//! reaches this flag not at all.
//!
//! **"Still sees" is the whole qualification, and one kind of spelling escapes it.** A
//! modifier the parser cannot read never reaches the gate — `geometry(Point,4326)` is
//! `42601 syntax error: expected a number, found ','` with the flag on or off.
//!
//! There **was** a second kind — a modifier the parser read and *discarded* — and
//! `timetz` was its only member: the reader handed the resolver a bare
//! `time with time zone`, so `'01:02:03'::timetz(3)` and a `timetz(3)` column were
//! `42704` with the flag off and **accepted as `text`** with it on. `EDGES.md` row 123
//! closed that: the four temporal precisions are kept in the canonical name now, so
//! `timetz(3)` reaches the gate like any other unmodelled carrier and answers this
//! `42601` under the flag (measured at a cast, a typed constant, the
//! `time(3) with time zone` spelling and a column). The modifier-less `timetz` still
//! coerces to `text` under the flag, `data_type` reading `text` and the `SELECT`
//! answering `01:02:03` against 18.6's `01:02:03+00`.
//!
//! **That kind has no members left**, and the check is the reader's own arms rather
//! than a list: every `(…)` a `parser::type_name` arm *reads* it either keeps in the
//! canonical name or spends on deciding which type the name is — `float(n)` was the
//! last that did neither, and its modifier now selects `float4`/`float8`
//! (`EDGES.md` row 116). An arm that returns before the `(` leaves it **unread**, which
//! is the first kind above and not this one; `json`'s arm is where that route is stated,
//! because for `json` leaving it unread is a decision rather than a default.
//!
//! **That is a decision, not a leftover**, and the alternative was live: exempting the
//! lenient arm would restore the flag's old lenience for a legacy `varchar2(10)`. It is
//! declined on four grounds, three of them measured against the exemption built and
//! run (`mutate` the gate to skip an unrecognized base, then read what it does).
//!
//! - It answers 18.6's own `42601 type modifier is not allowed for type "…"` — code
//!   *and* sentence — at `name(3)`, `oid(3)`, `money(3)`, `xml(3)`, `point(3)` and
//!   `regclass(3)`, where the default-off route answers `42704` and where the exemption
//!   would build a `text` column 18.6 refuses outright. **Six spellings, and not a
//!   class**: all six have `typmodin` 0 on 18.6, and the unmodelled types whose
//!   `typmodin` is non-zero go the other way. There are exactly four of those — 18.6
//!   has ten scalar types with a non-zero `typmodin` (`bit`, `bpchar`, `interval`,
//!   `numeric`, `time`, `timestamp`, `timestamptz`, `timetz`, `varbit`, `varchar`,
//!   measured off `pg_type`), and pglet models six of them. `bit`, `varbit` and
//!   `bpchar` are `is_multiword_type_lead` words whose `passthrough_type` keeps the
//!   modifier, so all three reach the gate and take this `42601` under the flag — at a
//!   cast (`1::bit(3)`, `'1'::varbit(3)`, `'ab'::bpchar(3)`, where 18.6 answers `001`,
//!   `1` and `ab `), at a typed constant (`bit(3) '1'`, `varbit(3) '1'`,
//!   `bpchar(3) 'ab'`, where it answers `100`, `1` and `ab `) and at a column, where it
//!   builds the column (all measured). **`timetz` is the fourth and joined them** when
//!   `EDGES.md` row 123 stopped discarding a temporal precision: `timetz(3)` at a cast,
//!   a typed constant and a column is this same `42601` under the flag, against 18.6's
//!   `01:02:03+00` and its built `timetz` column. So the flag route gains agreement at
//!   six spellings and loses it at four, and at the four the sentence is affirmatively
//!   wrong about 18.6: it reports a modifier forbidden that 18.6 requires. What is left
//!   of this ground is only that neither posture is right — the exemption builds a
//!   silently wrong *type* where the gate returns a loud wrong *verdict*, and
//!   `Correct-or-loud` takes the loud one. The decision rests on the row-destroying
//!   write and the session toggle below.
//!   (Under the flag `json(3)` raises a bare syntax error on both engines by the other
//!   route: pglet's `json` arm returns before the `(`, the flag coerces the bare `json`
//!   to `text`, and the stray `(` is then what fails — where with the flag off the
//!   name's own `42704` comes first. Where 18.6 lacks the type as well, both engines are
//!   loud and their codes differ — `42601` here against `42704` there.)
//! - It closes a **row-destroying write**. With the exemption, `'t'::regclass(3)` is
//!   `'t'::text`, so `DELETE FROM dw WHERE 't'::regclass(3) = 't'` deletes every row
//!   against 18.6's `42601`; refusing leaves them (measured, `count(*)` stays 2). The
//!   modifier-less `'t'::nosuchtype` still deletes them — the flag's declared posture,
//!   and `EDGES.md` row 118's standing residual.
//! - The exemption would make one gate's verdict depend on a **session toggle** rather
//!   than on the resolved type, which is the invariant `docs/coercion.md` § A canonical
//!   type name decomposes once exists to hold; and it would need
//!   `type_oid_from_parts` to leak which arm it took, since `text` and a coerced
//!   unknown are the same `TypeOid` afterwards.
//! - Its benefit is narrower than it looks: a modifier the parser never hands the
//!   resolver cannot be exempted by a rule about the resolver — `geometry(Point,4326)`
//!   goes unread, the escape above. (`timetz(3)` was the second such escape and is one
//!   no longer; the exemption would now reach it, and would build a `text` column where
//!   18.6 builds a `timetz` one.)
//!
//! What it costs is loud and nameable, and was measured on the exemption rather than
//! argued: a legacy dump spelling `varchar2(10)` no longer loads under the flag, where
//! with the exemption it builds a `text` column whose
//! `information_schema.columns.character_maximum_length` is NULL — the declared length
//! dropped silently.
//!
//! # Mechanism
//!
//! The flag is **thread-local**, set once per connection. The engine is sync and
//! thread-per-connection (no async, no work-stealing), so a thread-local is the
//! per-connection setting with zero plumbing churn: one set-site (the embed
//! `Database` / the server connection) and one read-site — `unsupported_type_oid`,
//! the single decision point both type-resolution paths funnel through — with
//! nothing threaded through the recursive expression walk or the executor
//! dispatch in between. The default is `false` (fail loud) and resets to the
//! default at thread start, so a fresh connection never inherits a stale toggle.

use crate::types::TypeOid;
use crate::{Error, Result};
use std::cell::Cell;

thread_local! {
    /// Whether the current thread coerces unsupported types to `TEXT` instead of
    /// erroring. Default `false` (fail loud).
    static COERCE_UNKNOWN_TYPES: Cell<bool> = const { Cell::new(false) };
}

/// Set the current thread's coerce-unknown-types policy.
///
/// `true` restores the legacy lenient behavior (unsupported types degrade to
/// `TEXT`); `false` (the default) fails loud with `Error::UnsupportedType`. A
/// host sets this once when wiring up a connection's thread.
pub fn set_coerce_unknown_types(coerce: bool) {
    COERCE_UNKNOWN_TYPES.with(|c| c.set(coerce));
}

/// Whether the current thread coerces unsupported types to `TEXT`.
#[must_use]
pub fn coerce_unknown_types() -> bool {
    COERCE_UNKNOWN_TYPES.with(Cell::get)
}

/// Resolve an unsupported/unrecognized type name: `TypeOid::Text` when the
/// thread's `coerce_unknown_types` flag is set, else `Error::UnsupportedType`.
///
/// The single decision point for both type-resolution paths (the native parser's
/// `CAST`/`::` path and the DDL `type_name_to_oid` path), so the
/// fail-loud-vs-coerce policy can't drift between them.
///
/// # Errors
///
/// Returns `Error::UnsupportedType` when coercion is disabled (the default).
pub fn unsupported_type_oid(type_name: &str) -> Result<TypeOid> {
    if coerce_unknown_types() {
        Ok(TypeOid::Text)
    } else {
        Err(Error::UnsupportedType {
            type_name: type_name.to_string(),
        })
    }
}
