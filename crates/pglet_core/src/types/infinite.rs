//! `infinity` and `-infinity` — the two values a `date`, `timestamp` or `timestamptz`
//! holds beyond every calendar value.
//!
//! 18.6 spells them as reserved counts inside its own integer representation
//! (`DATE_NOBEGIN` / `DATE_NOEND` are `i32::MIN` / `i32::MAX`, `TIMESTAMP_NOBEGIN` /
//! `TIMESTAMP_NOEND` are `i64::MIN` / `i64::MAX`). pglet's calendar values are `time`'s
//! `Date` / `OffsetDateTime`, which have no such slot, so the sentinels are variants of
//! their own: [`Infinite`] wraps the finite value, and a site that reads calendar fields
//! has to take the `Finite` arm by name — nothing reads an infinity as a date.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::{Date, OffsetDateTime};

/// A calendar value, or one of the two infinities outside every calendar value.
///
/// The variant order is the value order — `NegInfinity < Finite(_) < Infinity` — and the
/// derived `Ord` compares the finite payloads by their own order, so ordering, `MIN` /
/// `MAX` and the index key all read one order.
///
/// A [`PgDate`] and a [`PgTimestamp`] are persisted as the count 18.6's binary format
/// carries — [`PgDate::to_pg_days`], [`PgTimestamp::to_pg_micros`] — sentinels included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Infinite<T> {
    /// `-infinity`, before every value.
    NegInfinity,
    /// A calendar value.
    Finite(T),
    /// `infinity`, after every value.
    Infinity,
}

/// A `date`: a calendar day, or `±infinity`.
pub type PgDate = Infinite<Date>;

/// A `timestamp` / `timestamptz`: an instant, or `±infinity`.
pub type PgTimestamp = Infinite<OffsetDateTime>;

impl<T> From<T> for Infinite<T> {
    fn from(value: T) -> Self {
        Self::Finite(value)
    }
}

impl<T> Infinite<T> {
    /// The calendar value, or `None` for an infinity.
    pub const fn finite(&self) -> Option<&T> {
        match self {
            Self::Finite(v) => Some(v),
            Self::NegInfinity | Self::Infinity => None,
        }
    }

    /// Carry the calendar value through `f`, leaving an infinity as it is — 18.6's
    /// "can't change infinity" at `date ± integer`, `± interval`, the casts and the
    /// truncations.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Infinite<U> {
        match self {
            Self::NegInfinity => Infinite::NegInfinity,
            Self::Finite(v) => Infinite::Finite(f(v)),
            Self::Infinity => Infinite::Infinity,
        }
    }

    /// [`Self::map`] through a fallible `f`.
    ///
    /// # Errors
    ///
    /// `f`'s error, for a finite value.
    pub fn try_map<U, E>(self, f: impl FnOnce(T) -> Result<U, E>) -> Result<Infinite<U>, E> {
        Ok(match self {
            Self::NegInfinity => Infinite::NegInfinity,
            Self::Finite(v) => Infinite::Finite(f(v)?),
            Self::Infinity => Infinite::Infinity,
        })
    }

    /// The text 18.6 renders for an infinity (`infinity` / `-infinity`), or `None` for a
    /// calendar value.
    pub const fn infinity_text(&self) -> Option<&'static str> {
        match self {
            Self::NegInfinity => Some("-infinity"),
            Self::Finite(_) => None,
            Self::Infinity => Some("infinity"),
        }
    }
}

/// Julian day of `2000-01-01`, day 0 of a Postgres `date`.
const PG_EPOCH_JULIAN_DAY: i32 = 2_451_545;

/// Microseconds from the Unix epoch to `2000-01-01`, instant 0 of a Postgres timestamp.
const PG_EPOCH_UNIX_MICROS: i128 = 946_684_800_000_000;

impl PgDate {
    /// Days since `2000-01-01` — `date_send`'s count — with `i32::MIN` / `i32::MAX` for
    /// `-infinity` / `infinity` (`DATE_NOBEGIN` / `DATE_NOEND`). Order-preserving.
    #[must_use]
    pub const fn to_pg_days(&self) -> i32 {
        match self {
            Self::NegInfinity => i32::MIN,
            Self::Finite(d) => d.to_julian_day() - PG_EPOCH_JULIAN_DAY,
            Self::Infinity => i32::MAX,
        }
    }

    /// The inverse of [`Self::to_pg_days`], or `None` for a count `time` cannot hold. The
    /// date domain is not judged here.
    #[must_use]
    pub fn from_pg_days(days: i32) -> Option<Self> {
        Some(match days {
            i32::MIN => Self::NegInfinity,
            i32::MAX => Self::Infinity,
            _ => Self::Finite(Date::from_julian_day(PG_EPOCH_JULIAN_DAY.checked_add(days)?).ok()?),
        })
    }
}

impl PgTimestamp {
    /// Microseconds since `2000-01-01 00:00:00` UTC — `timestamp_send`'s count — with
    /// `i64::MIN` / `i64::MAX` for `-infinity` / `infinity` (`TIMESTAMP_NOBEGIN` /
    /// `TIMESTAMP_NOEND`). Order-preserving; an instant's offset is not carried.
    #[must_use]
    pub const fn to_pg_micros(&self) -> i64 {
        match self {
            Self::NegInfinity => i64::MIN,
            // `time`'s instants all fit: ±9999 years is far inside `i64` microseconds.
            Self::Finite(ts) => (ts.unix_timestamp_nanos() / 1000 - PG_EPOCH_UNIX_MICROS) as i64,
            Self::Infinity => i64::MAX,
        }
    }

    /// The inverse of [`Self::to_pg_micros`], at UTC, or `None` for a count `time` cannot
    /// hold. The date domain is not judged here.
    #[must_use]
    pub fn from_pg_micros(micros: i64) -> Option<Self> {
        Some(match micros {
            i64::MIN => Self::NegInfinity,
            i64::MAX => Self::Infinity,
            _ => Self::Finite(
                OffsetDateTime::from_unix_timestamp_nanos(
                    (i128::from(micros) + PG_EPOCH_UNIX_MICROS) * 1000,
                )
                .ok()?,
            ),
        })
    }
}

impl Serialize for PgDate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i32(self.to_pg_days())
    }
}

impl<'de> Deserialize<'de> for PgDate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_pg_days(i32::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("date out of range"))
    }
}

impl Serialize for PgTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.to_pg_micros())
    }
}

impl<'de> Deserialize<'de> for PgTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_pg_micros(i64::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("timestamp out of range"))
    }
}

/// Read `s` as an `infinity` spelling, or `None` for the caller's own grammar to read.
///
/// The word, case-insensitive, with an optional `+` or `-` ahead of it — whitespace
/// allowed between the sign and the word — and whitespace around the whole.
///
/// This is the part of 18.6's `ParseDateTime` / `DecodeSpecial` route the date and
/// timestamp readers model: a lone `infinity` token (`DTK_LATE`) or a sign-prefixed one
/// (`DTK_EARLY` for `-`), where whitespace is C's `isspace` — space, tab, newline,
/// vertical tab, form feed, carriage return. It is not the whole route. `ParseDateTime`
/// also skips, as a field delimiter, an ASCII punctuation byte it gives no role of its own,
/// and accepts the token beside an `AD` / `BC` field, so 18.6 reads `infinity;`,
/// `(infinity)` and `-infinity ad` as the infinity and these readers refuse them;
/// `docs/temporal.md` § `infinity` and `-infinity` carries the measured spellings.
#[must_use]
pub fn parse_infinity<T>(s: &str) -> Option<Infinite<T>> {
    let negative = parse_infinity_sign(s.as_bytes())?;
    Some(if negative {
        Infinite::NegInfinity
    } else {
        Infinite::Infinity
    })
}

/// [`parse_infinity`]'s reader, not generic, so it is compiled once: `Some(true)` for
/// `-infinity`.
fn parse_infinity_sign(s: &[u8]) -> Option<bool> {
    let (negative, word) = match trim_c_space(s) {
        [b'-', rest @ ..] => (true, trim_c_space(rest)),
        [b'+', rest @ ..] => (false, trim_c_space(rest)),
        whole => (false, whole),
    };
    word.eq_ignore_ascii_case(b"infinity").then_some(negative)
}

/// `s` without C `isspace` bytes at either end.
fn trim_c_space(mut s: &[u8]) -> &[u8] {
    let space = |c: &u8| matches!(c, b' ' | b'\t'..=b'\r');
    while let [first, rest @ ..] = s
        && space(first)
    {
        s = rest;
    }
    while let [rest @ .., last] = s
        && space(last)
    {
        s = rest;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{Infinite, parse_infinity};
    use time::macros::date;

    #[test]
    fn the_infinities_order_outside_every_calendar_value() {
        let min = Infinite::Finite(date!(0001 - 01 - 01));
        let max = Infinite::Finite(date!(9999 - 12 - 31));
        assert!(Infinite::NegInfinity < min);
        assert!(max < Infinite::Infinity);
        assert!(Infinite::<time::Date>::NegInfinity < Infinite::Infinity);
        assert_eq!(Infinite::<time::Date>::Infinity, Infinite::Infinity);
    }

    /// The count conversions invert, sentinels included, and a count `time` cannot hold
    /// reads as nothing.
    #[test]
    fn the_counts_invert() {
        use super::{PgDate, PgTimestamp};
        use time::macros::datetime;
        for d in [
            PgDate::NegInfinity,
            PgDate::Finite(date!(0001 - 01 - 01)),
            PgDate::Infinity,
        ] {
            assert_eq!(PgDate::from_pg_days(d.to_pg_days()), Some(d));
        }
        assert_eq!(
            PgDate::from_pg_days(0),
            Some(PgDate::Finite(date!(2000 - 01 - 01)))
        );
        let local = PgTimestamp::Finite(datetime!(2026-06-15 22:00:00 -5));
        for ts in [PgTimestamp::NegInfinity, local, PgTimestamp::Infinity] {
            assert_eq!(PgTimestamp::from_pg_micros(ts.to_pg_micros()), Some(ts));
        }
        assert!(PgDate::from_pg_days(i32::MAX - 1).is_none());
        assert!(PgTimestamp::from_pg_micros(i64::MAX - 1).is_none());
    }

    #[test]
    fn an_infinity_spelling_is_the_word_with_an_optional_sign() {
        for (s, want) in [
            ("infinity", Some(Infinite::Infinity)),
            (" INFINITY ", Some(Infinite::Infinity)),
            ("+Infinity", Some(Infinite::Infinity)),
            ("+ infinity", Some(Infinite::Infinity)),
            ("-infinity", Some(Infinite::NegInfinity)),
            ("\t- infinity\n", Some(Infinite::NegInfinity)),
            ("\x0binfinity\x0c\r", Some(Infinite::Infinity)),
            ("\u{a0}infinity", None),
            ("infinit", None),
            ("infinityx", None),
            ("--infinity", None),
            ("-+infinity", None),
            ("infinity infinity", None),
            ("2000-01-01", None),
            ("", None),
        ] {
            assert_eq!(parse_infinity::<()>(s), want, "{s:?}");
        }
    }
}
