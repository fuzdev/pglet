//! The `INTERVAL` value type — Postgres's 3-field span.
//!
//! `PgInterval` is a `(months, days, micros)` triple, faithful to Postgres's
//! `Interval` struct (which stores `month`/`day`/`time`, in that logical order).
//! It backs [`Value::Interval`](super::Value::Interval).
//!
//! # Why three fields
//!
//! A month has no fixed microsecond width (28–31 days; a year leap-varies) and a
//! calendar day is not always 24 h under DST — so Postgres keeps months, days,
//! and microseconds **separate** and resolves them against a concrete anchor only
//! during `timestamp ± interval` arithmetic. pglet mirrors that: a bare
//! `INTERVAL '1 month'` is `{months: 1, days: 0, micros: 0}`, distinct from
//! `INTERVAL '30 days'`, even though the two compare **equal**.
//!
//! # Comparison — the estimate
//!
//! Ordering, equality, hashing, and the B-tree index key all route through the
//! *estimate* `micros + (months*30 + days) * 86_400_000_000` (Postgres's
//! `interval_cmp_value`, computed in `i128` to avoid overflow). So `INTERVAL '30
//! days'` equals `INTERVAL '1 mon'` and a `UNIQUE` interval column dedups them —
//! matching Postgres. Because of this, [`PartialEq`] is **custom** (not derived,
//! like [`PgNumeric`](super::PgNumeric)); field-level round-trip tests must
//! destructure and compare fields explicitly.
//!
//! # Input / output
//!
//! [`PgInterval::parse`] accepts the three Postgres input grammars (verbose,
//! SQL-standard, ISO-8601) under the default `IntervalStyle = postgres`;
//! [`format_interval_text`] renders the default `postgres` output style. A
//! malformed literal is [`Error::InvalidIntervalSyntax`] (SQLSTATE `22007`); a
//! parse-time field overflow is [`Error::IntervalFieldOverflow`] (`22015`).

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::BinaryReader;
use crate::error::{Error, Result};

/// Postgres `DAYS_PER_MONTH` — the nominal day width of a month in the comparison
/// estimate (never in real calendar arithmetic).
const DAYS_PER_MONTH: i128 = 30;
/// Microseconds in a 24 h day (`USECS_PER_DAY`).
const USECS_PER_DAY: i128 = 86_400_000_000;
/// Months per year.
const MONTHS_PER_YEAR: i64 = 12;

/// An `INTERVAL` value: a `(months, days, micros)` span.
///
/// See the module docs for the field rationale and the estimate-based comparison.
/// Derives `Clone`/`Copy`/`Serialize`/`Deserialize` only — **not** `PartialEq`
/// (custom, over the estimate) nor `Ord`/`Hash`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PgInterval {
    /// Whole months (a year contributes 12); never folded into `days`.
    pub months: i32,
    /// Whole days; never folded into `months` (a day is not always 1/30 month).
    pub days: i32,
    /// Sub-day remainder, in microseconds.
    pub micros: i64,
}

impl PartialEq for PgInterval {
    fn eq(&self, other: &Self) -> bool {
        self.estimate() == other.estimate()
    }
}

impl PgInterval {
    /// A pure-microsecond interval (`months = days = 0`) — the shape produced by
    /// `timestamp - timestamp` before justification and `time - time`.
    #[must_use]
    pub const fn from_micros(micros: i64) -> Self {
        Self {
            months: 0,
            days: 0,
            micros,
        }
    }

    /// Construct from all three fields.
    #[must_use]
    pub const fn new(months: i32, days: i32, micros: i64) -> Self {
        Self {
            months,
            days,
            micros,
        }
    }

    /// Whether every field is zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.months == 0 && self.days == 0 && self.micros == 0
    }

    /// The Postgres comparison *estimate* (`interval_cmp_value`), in microseconds:
    /// `micros + (months*30 + days) * USECS_PER_DAY`, computed in `i128` so no
    /// intermediate overflows. Two intervals compare equal iff their estimates are
    /// equal, so `30 days` collides `1 mon`.
    #[must_use]
    pub fn estimate(&self) -> i128 {
        let span_days = i128::from(self.months) * DAYS_PER_MONTH + i128::from(self.days);
        i128::from(self.micros) + span_days * USECS_PER_DAY
    }

    /// The order-preserving B-tree index key: the estimate as a sign-flipped
    /// big-endian `i128` (the span can be negative). Byte order then matches
    /// [`PgInterval::estimate`] order, so a B-tree index over an interval column
    /// sorts and dedups on the estimate.
    #[must_use]
    pub fn index_key_bytes(&self) -> [u8; 16] {
        ((self.estimate() as u128) ^ (1u128 << 127)).to_be_bytes()
    }

    /// Encode to the 16-byte Postgres binary wire form (`interval_send`): the
    /// **time (i64 µs) → day (i32) → month (i32)** field order, big-endian.
    #[must_use]
    pub fn to_wire_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..8].copy_from_slice(&self.micros.to_be_bytes());
        out[8..12].copy_from_slice(&self.days.to_be_bytes());
        out[12..16].copy_from_slice(&self.months.to_be_bytes());
        out
    }

    /// Read the Postgres binary wire form from `buf` the way `interval_recv` reads it:
    /// time (i64 µs), then day (i32), then month (i32), big-endian, each through the
    /// multi-byte message reader, leaving the cursor after the month. Bytes after it are
    /// not this reader's to refuse.
    ///
    /// 18.6 reads the all-maximum and all-minimum field triples as `infinity` and
    /// `-infinity`. `PgInterval` has neither, so both are refused with the error the text
    /// path gives the *words* `infinity` and `-infinity`, rather than stored as the finite
    /// interval the three fields would otherwise spell. The text path does not refuse the
    /// triples themselves: spelled as numbers, each is a finite interval (`EDGES.md` row 143).
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] (`08P01`) when a field runs past the buffer;
    /// [`Error::InvalidIntervalSyntax`] (`22007`) for either infinity sentinel.
    pub fn recv(buf: &mut BinaryReader<'_>) -> Result<Self> {
        let micros = buf.i64()?;
        let days = buf.i32()?;
        let months = buf.i32()?;
        // `INTERVAL_NOBEGIN` / `INTERVAL_NOEND`: all three fields at their minimum or
        // at their maximum.
        if (micros, days, months) == (i64::MAX, i32::MAX, i32::MAX) {
            return Err(Error::InvalidIntervalSyntax(String::from("infinity")));
        }
        if (micros, days, months) == (i64::MIN, i32::MIN, i32::MIN) {
            return Err(Error::InvalidIntervalSyntax(String::from("-infinity")));
        }
        Ok(Self {
            months,
            days,
            micros,
        })
    }

    /// Parse an `INTERVAL` literal body under the default `IntervalStyle = postgres`.
    ///
    /// Accepts the three Postgres input grammars: **verbose** (`1 year 2 mons 3
    /// days 04:05:06`, per-field signs, trailing `ago` negating all fields),
    /// **SQL-standard** (`1-2` year-month, `3 4:05:06` day + time), and
    /// **ISO-8601** (`P1Y2M3DT4H5M6S`, `M` before `T` = months / after `T` =
    /// minutes). Fractional units normalize exactly as Postgres does (years/decades
    /// round to whole months; weeks/months spill into days then microseconds;
    /// day/hour/minute/second spill into microseconds). `quarter`/`qtr` is rejected
    /// (Postgres rejects it in interval input).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidIntervalSyntax`] (SQLSTATE `22007`) for any spelling Postgres
    /// rejects; [`Error::IntervalFieldOverflow`] (`22015`) if a field magnitude
    /// overflows the representable range at parse/decode time.
    pub fn parse(input: &str) -> Result<Self> {
        let s = input.trim();
        if s.is_empty() {
            return Err(Error::InvalidIntervalSyntax(input.to_string()));
        }
        let mut itm = Itm::default();
        let mut ago = false;
        // A leading `P`/`p` is unambiguously ISO-8601 (no interval unit starts with
        // `p`), exactly as Postgres's `DecodeInterval` dispatches.
        let parsed = if s.as_bytes()[0].eq_ignore_ascii_case(&b'P') {
            parse_iso8601(s, input, &mut itm)
        } else {
            parse_verbose(s, input, &mut itm, &mut ago)
        };
        let decoded = parsed.and_then(|()| itm.to_interval(ago));
        // The deep decode helpers can't carry the original text, so fill it in here
        // for the `22015` message (the SQLSTATE is already correct).
        decoded.map_err(|e| match e {
            Error::IntervalFieldOverflow(_) => Error::IntervalFieldOverflow(input.to_string()),
            other => other,
        })
    }
}

/// Render a [`PgInterval`] as Postgres text under the default `postgres`
/// `IntervalStyle` (`EncodeInterval`, `INTSTYLE_POSTGRES`).
///
/// Emits `year`/`mon`/`day` groups (each omitted when zero, pluralized on `value
/// != 1` so a negative value is plural: `-1` → `-1 years`; `mon`/`mons`, never
/// `month`) then a `HH:MM:SS[.ffffff]` time group. A leading `+` is forced on a
/// positive group whose predecessor was negative; the zero interval renders
/// `00:00:00`. The single source for interval text (wire text, the `::text` cast,
/// the differential harness).
#[must_use]
pub fn format_interval_text(iv: &PgInterval) -> String {
    // Decompose to Postgres's `pg_itm` (interval2itm): year/mon from months, day
    // passthrough, then H:M:S.µs — every quotient truncates toward zero and carries
    // the sign of its source, so a negative interval prints all-negative groups.
    let year = iv.months / 12;
    let mon = iv.months % 12;
    let mday = iv.days;
    let mut t = iv.micros;
    let hour = t / 3_600_000_000;
    t %= 3_600_000_000;
    let min = t / 60_000_000;
    t %= 60_000_000;
    let sec = t / 1_000_000;
    let fsec = t % 1_000_000; // sub-second microseconds

    let mut out = String::new();
    let mut is_zero = true;
    let mut is_before = false;
    add_int_part(
        &mut out,
        i64::from(year),
        "year",
        &mut is_zero,
        &mut is_before,
    );
    add_int_part(
        &mut out,
        i64::from(mon),
        "mon",
        &mut is_zero,
        &mut is_before,
    );
    add_int_part(
        &mut out,
        i64::from(mday),
        "day",
        &mut is_zero,
        &mut is_before,
    );

    if is_zero || hour != 0 || min != 0 || sec != 0 || fsec != 0 {
        let minus = hour < 0 || min < 0 || sec < 0 || fsec < 0;
        if !is_zero {
            out.push(' ');
        }
        if minus {
            out.push('-');
        } else if is_before {
            out.push('+');
        }
        let _ = write!(out, "{:02}:{:02}:{:02}", hour.abs(), min.abs(), sec.abs());
        if fsec != 0 {
            let frac = format!("{:06}", fsec.abs());
            out.push('.');
            out.push_str(frac.trim_end_matches('0'));
        }
    }
    out
}

/// Emit one signed `year`/`mon`/`day` group (`AddPostgresIntPart`): nothing when
/// `value == 0`, else a space separator (unless first), a forced `+` when the
/// previous group was negative and this one is positive, the value with its own
/// sign, the unit, and a trailing `s` when `value != 1`.
fn add_int_part(
    out: &mut String,
    value: i64,
    unit: &str,
    is_zero: &mut bool,
    is_before: &mut bool,
) {
    if value == 0 {
        return;
    }
    if !*is_zero {
        out.push(' ');
    }
    if *is_before && value > 0 {
        out.push('+');
    }
    out.push_str(&value.to_string());
    out.push(' ');
    out.push_str(unit);
    if value != 1 {
        out.push('s');
    }
    *is_zero = false;
    *is_before = value < 0;
}

/// Microseconds in a 24 h day (`USECS_PER_DAY`), the day → µs cascade factor.
const USECS_PER_DAY_I64: i64 = 86_400_000_000;

/// A Postgres `pg_itm_in`-shaped decode accumulator (`DecodeInterval` builds one).
/// All time accumulates into a single `usec` field (as Postgres's `tm_usec` does),
/// so the whole-number part of every unit stays **exact** (no `f64` precision loss);
/// only the sub-unit fraction is computed in `f64`. `mon`/`year` are `i64` so
/// intermediate sums can't overflow before the final range-check in `to_interval`.
#[derive(Default)]
struct Itm {
    usec: i64,
    mday: i64,
    mon: i64,
    year: i64,
}

impl Itm {
    /// Combine into a [`PgInterval`] (`itmin2interval`): `months = year*12 + mon`,
    /// `days = mday`, `micros = usec`. `negate` applies the trailing `ago`. A
    /// parse-time field overflow → `22015` (filled with the original text at the
    /// `PgInterval::parse` boundary).
    fn to_interval(&self, negate: bool) -> Result<PgInterval> {
        let sign: i64 = if negate { -1 } else { 1 };
        // **Assembly overflow is `22008`, not `22015`.** A field the user wrote that
        // does not fit its own range is `interval field value out of range` (checked
        // where each field is applied); a field that fits and whose *assembled* value
        // does not is `interval out of range`. 18.6 splits them exactly there:
        // `INTERVAL '2147483648 months'` is `22015` (the field), while
        // `'178956971 years'` is `22008` — the year count fits `int32` and only the
        // ×12 product does not.
        let res = interval_result_oor;
        let total_months = self
            .year
            .checked_mul(MONTHS_PER_YEAR)
            .and_then(|m| m.checked_add(self.mon))
            .and_then(|m| m.checked_mul(sign))
            .ok_or_else(res)?;
        let months = i32::try_from(total_months).map_err(|_| res())?;
        let days_signed = self.mday.checked_mul(sign).ok_or_else(res)?;
        let days = i32::try_from(days_signed).map_err(|_| res())?;
        let micros = self.usec.checked_mul(sign).ok_or_else(res)?;
        Ok(PgInterval {
            months,
            days,
            micros,
        })
    }
}

/// The interval units, each mapping a number to a field with a scale.
#[derive(Clone, Copy)]
enum Unit {
    Microsecond,
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    /// Year-family: the value multiplies by `year_scale` (year 1, decade 10, …).
    Year(i64),
}

/// Map a lowercased unit word to a [`Unit`], or `None` for an unrecognized or
/// deliberately-rejected word (`quarter`/`qtr`, which Postgres rejects in interval
/// input even though it is valid for `EXTRACT`/`date_trunc`).
fn unit_from_word(w: &str) -> Option<Unit> {
    Some(match w {
        "microsecond" | "microseconds" | "usec" | "usecs" | "useconds" | "us" => Unit::Microsecond,
        "millisecond" | "milliseconds" | "msec" | "msecs" | "mseconds" | "ms" => Unit::Millisecond,
        "second" | "seconds" | "sec" | "secs" | "s" => Unit::Second,
        "minute" | "minutes" | "min" | "mins" | "m" => Unit::Minute,
        "hour" | "hours" | "hr" | "hrs" | "h" => Unit::Hour,
        "day" | "days" | "d" => Unit::Day,
        "week" | "weeks" | "w" => Unit::Week,
        "month" | "months" | "mon" | "mons" => Unit::Month,
        "year" | "years" | "yr" | "yrs" | "y" => Unit::Year(1),
        "decade" | "decades" | "dec" | "decs" => Unit::Year(10),
        "century" | "centuries" | "cent" | "c" => Unit::Year(100),
        "millennium" | "millennia" | "mil" | "mils" => Unit::Year(1000),
        _ => return None,
    })
}

/// C `rint` (round to nearest, ties to even) into an `i64` — Postgres's default
/// float rounding mode, used for the *year* fraction (`AdjustFractYears`) and the
/// `HH:MM:SS.fff` time-field sub-second (`ParseFractionalSecond`). The **unit**
/// sub-µs fraction uses truncate-then-bump instead (`AdjustFractMicroseconds`).
fn rint(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Parse a numeric field into its **exact** integer part and `f64` fractional part,
/// matching Postgres's `strtoi64` + `ParseFraction`: the integer part is parsed
/// exactly (out-of-range → `None`, never a silent `f64` saturation), and only the
/// `.frac` tail is `f64`. An **exponent form** (`1e3`) is rejected (Postgres's
/// `strtoi64` stops at `e`, leaving trailing input → bad format). Returns `None`
/// (→ `22007`) for any spelling Postgres rejects. The fraction carries the sign of
/// the integer part.
fn parse_number(tok: &str) -> Option<(i64, f64)> {
    // Reject exponent forms — Postgres never reads a `float` here.
    if tok.bytes().any(|b| matches!(b, b'e' | b'E')) {
        return None;
    }
    let (int_str, frac_str) = tok
        .split_once('.')
        .map_or((tok, None), |(a, b)| (a, Some(b)));
    let negative = int_str.starts_with('-');
    // The integer part is parsed exactly; an empty or lone-sign int part (`.5`,
    // `-.5`) is zero, and any out-of-`i64` magnitude fails loud rather than saturate.
    let int_val: i64 = if int_str.is_empty() || int_str == "-" || int_str == "+" {
        0
    } else {
        int_str.parse().ok()?
    };
    let frac = match frac_str {
        None | Some("") => 0.0,
        Some(f) => {
            if !f.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mag: f64 = format!("0.{f}").parse().ok()?;
            if negative { -mag } else { mag }
        }
    };
    Some((int_val, frac))
}

/// Apply a number (exact integer part `iv` + `f64` fraction `frac`) under `unit` to
/// the accumulator, matching `DecodeInterval`'s per-unit cases exactly. Overflow →
/// `22015`.
fn apply_number(itm: &mut Itm, iv: i64, frac: f64, unit: Unit) -> Result<()> {
    match unit {
        Unit::Microsecond => adjust_micros(itm, iv, frac, 1),
        Unit::Millisecond => adjust_micros(itm, iv, frac, 1_000),
        Unit::Second => adjust_micros(itm, iv, frac, 1_000_000),
        Unit::Minute => adjust_micros(itm, iv, frac, 60_000_000),
        Unit::Hour => adjust_micros(itm, iv, frac, 3_600_000_000),
        Unit::Day => {
            field_fits_i32(iv)?;
            adjust_days(itm, iv, 1)?;
            adjust_fract_micros(itm, frac, USECS_PER_DAY_I64)
        }
        Unit::Week => {
            field_fits_i32(iv)?;
            adjust_days(itm, iv, 7)?;
            adjust_fract_days(itm, frac, 7)
        }
        Unit::Month => {
            field_fits_i32(iv)?;
            itm.mon = itm.mon.checked_add(iv).ok_or_else(interval_oor)?;
            adjust_fract_days(itm, frac, 30)
        }
        Unit::Year(scale) => {
            // The **field** is the year (or decade/century/…) count the user wrote,
            // and it is checked against `int32` here; the ×`scale` product is an
            // assembly concern and overflows as `22008` in `Itm::to_interval`.
            field_fits_i32(iv)?;
            let add = iv.checked_mul(scale).ok_or_else(interval_oor)?;
            itm.year = itm.year.checked_add(add).ok_or_else(interval_oor)?;
            adjust_fract_years(itm, frac, scale)
        }
    }
}

/// Refuse a written field that does not fit `int32` — Postgres's per-field range for
/// the year / month / day counters, checked where the field is applied so it carries
/// `22015` rather than the assembly's `22008`.
///
/// # Errors
///
/// [`Error::IntervalFieldOverflow`] (`22015`) when `value` is outside `i32`.
fn field_fits_i32(value: i64) -> Result<()> {
    i32::try_from(value).map(|_| ()).map_err(|_| interval_oor())
}

/// An `interval out of range` error (`22008`), for an **assembled** value that does
/// not fit though every written field did. 18.6 carries no value in this message.
const fn interval_result_oor() -> Error {
    Error::TemporalOutOfRange("interval")
}

/// An `interval field value out of range` error (`22015`), for a field magnitude that
/// overflows at parse/decode time. The original text is filled in at the [`PgInterval::parse`]
/// boundary — these deep decode helpers don't carry it — so the value is left empty here.
const fn interval_oor() -> Error {
    Error::IntervalFieldOverflow(String::new())
}

/// `AdjustMicroseconds`: add the exact integer `val * scale` µs, then the fraction.
fn adjust_micros(itm: &mut Itm, val: i64, frac: f64, scale: i64) -> Result<()> {
    let add = val.checked_mul(scale).ok_or_else(interval_oor)?;
    itm.usec = itm.usec.checked_add(add).ok_or_else(interval_oor)?;
    adjust_fract_micros(itm, frac, scale)
}

/// `AdjustFractMicroseconds`: `usec = (int64)(frac*scale)` (truncate toward zero),
/// then **bump** by one only if the leftover magnitude strictly exceeds ½ µs
/// (`> 0.5` / `< -0.5`) — Postgres's rounding here is truncate-then-bump, **not**
/// ties-to-even, so an exact ½ µs truncates toward zero.
fn adjust_fract_micros(itm: &mut Itm, frac: f64, scale: i64) -> Result<()> {
    if frac == 0.0 {
        return Ok(());
    }
    let scaled = frac * scale as f64;
    let mut usec = scaled as i64; // (int64) truncates toward zero
    let rem = scaled - usec as f64;
    if rem > 0.5 {
        usec += 1;
    } else if rem < -0.5 {
        usec -= 1;
    }
    itm.usec = itm.usec.checked_add(usec).ok_or_else(interval_oor)?;
    Ok(())
}

/// `AdjustDays`: add the exact integer `val * scale` days.
fn adjust_days(itm: &mut Itm, val: i64, scale: i64) -> Result<()> {
    let add = val.checked_mul(scale).ok_or_else(interval_oor)?;
    itm.mday = itm.mday.checked_add(add).ok_or_else(interval_oor)?;
    Ok(())
}

/// `AdjustFractDays`: spill a week/month fraction into whole days then microseconds
/// (assuming a 24 h day).
fn adjust_fract_days(itm: &mut Itm, frac: f64, scale: i64) -> Result<()> {
    if frac == 0.0 {
        return Ok(());
    }
    let scaled = frac * scale as f64;
    let extra_days = scaled as i64; // (int) truncates toward zero
    itm.mday = itm.mday.checked_add(extra_days).ok_or_else(interval_oor)?;
    let rem = scaled - extra_days as f64;
    adjust_fract_micros(itm, rem, USECS_PER_DAY_I64)
}

/// `AdjustFractYears`: a year fraction becomes `rint(frac * scale * 12)` whole
/// months (ties-to-even, unlike the microsecond fraction's truncate-then-bump).
fn adjust_fract_years(itm: &mut Itm, frac: f64, scale: i64) -> Result<()> {
    if frac == 0.0 {
        return Ok(());
    }
    let extra_months = rint(frac * scale as f64 * MONTHS_PER_YEAR as f64);
    itm.mon = itm.mon.checked_add(extra_months).ok_or_else(interval_oor)?;
    Ok(())
}

/// Parse the verbose / SQL-standard grammar (whitespace-separated fields).
fn parse_verbose(s: &str, orig: &str, itm: &mut Itm, ago: &mut bool) -> Result<()> {
    let err = || Error::InvalidIntervalSyntax(orig.to_string());
    // A number awaiting its unit; a bare number followed by a `HH:MM:SS` field is
    // days (the SQL-standard `3 4:05:06`), otherwise it defaults to seconds.
    let mut pending: Option<(i64, f64)> = None;
    let mut consumed = false;
    for tok in s.split_whitespace() {
        if tok == "@" {
            continue; // optional noise marker
        }
        let lower = tok.to_ascii_lowercase();
        if lower == "ago" {
            if let Some((iv, frac)) = pending.take() {
                apply_number(itm, iv, frac, Unit::Second)?;
            }
            *ago = true;
            consumed = true;
            continue;
        }
        if tok.contains(':') {
            if let Some((iv, frac)) = pending.take() {
                apply_number(itm, iv, frac, Unit::Day)?; // bare number before time → days
            }
            parse_time_field(tok, orig, itm)?;
            consumed = true;
            continue;
        }
        if let Some((y, m)) = split_year_month(tok) {
            if pending.take().is_some() {
                return Err(err());
            }
            itm.year = itm.year.checked_add(y).ok_or_else(interval_oor)?;
            itm.mon = itm.mon.checked_add(m).ok_or_else(interval_oor)?;
            consumed = true;
            continue;
        }
        if let Some(num) = parse_number(tok) {
            // **At most one unlabelled number may be pending.** A second one is
            // `22007` on 18.6, not a defaulted seconds field: `INTERVAL '1 2'`,
            // `'1 2 3'`, `'1.5 2.5'` and `'2 1 day'` are all refused there, while
            // `'1'`, `'1 day 2'`, `'1-2 3'` and `'1 2:30'` — the shapes with a single
            // pending number, or none — are values. Summing the displaced one as
            // seconds accepted every refused shape with a **wrong value**
            // (`'1 2'` → `00:00:03`, `'2 1 day'` → `1 day 00:00:02`), and a
            // single-field interval qualifier then masked that wrong value into a
            // different one (`INTERVAL '1 2' DAY` → `00:00:00`).
            //
            // Postgres reads a run of unlabelled numbers positionally instead, the
            // rightmost taking the qualifier's field and each earlier one the next
            // coarser (`datetime.c`'s `DecodeInterval`, the `DTK_NUMBER` case under
            // `IGNORE_DTF`), which is why `INTERVAL '1 2' HOUR` is `1 day 02:00:00`
            // there. That positional decode is not modelled here, so the shape is
            // refused rather than answered — the one spelling this refusal costs.
            if pending.is_some() {
                return Err(err());
            }
            pending = Some(num);
            continue;
        }
        // A unit word consumes the pending number.
        let unit = unit_from_word(&lower).ok_or_else(err)?;
        let (iv, frac) = pending.take().ok_or_else(err)?;
        apply_number(itm, iv, frac, unit)?;
        consumed = true;
    }
    if let Some((iv, frac)) = pending.take() {
        apply_number(itm, iv, frac, Unit::Second)?; // trailing bare number → seconds
        consumed = true;
    }
    if consumed { Ok(()) } else { Err(err()) }
}

/// Split a SQL-standard `Y-M` year-month field (`1-2` → 1 year 2 mons, `-1-2` →
/// -1 year -2 mons). Returns `None` unless the token is `[+-]?digits-digits`; a
/// leading sign applies to **both** parts (Postgres treats `-1-2` as -14 months).
/// The month part must be `0..=11`.
fn split_year_month(tok: &str) -> Option<(i64, i64)> {
    // Strip an optional leading sign, then require an internal `-` (the year-month
    // separator) with digits on both sides.
    let negative = tok.starts_with('-');
    let unsigned = tok.strip_prefix(['+', '-']).unwrap_or(tok);
    let dash = unsigned.find('-')?;
    let (y_str, m_str) = (&unsigned[..dash], &unsigned[dash + 1..]);
    if y_str.is_empty()
        || !y_str.bytes().all(|b| b.is_ascii_digit())
        || m_str.is_empty()
        || !m_str.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let y: i64 = y_str.parse().ok()?;
    let m: i64 = m_str.parse().ok()?;
    if !(0..=11).contains(&m) {
        return None;
    }
    if negative {
        Some((-y, -m))
    } else {
        Some((y, m))
    }
}

/// Parse a `HH:MM[:SS[.ffffff]]` time field (with an optional leading sign) into
/// the accumulator's `usec`, applying the sign to every component. The seconds
/// fraction rounds ties-to-even (`ParseFractionalSecond`), unlike the unit-based
/// sub-µs fraction's truncate-then-bump.
fn parse_time_field(tok: &str, orig: &str, itm: &mut Itm) -> Result<()> {
    let err = || Error::InvalidIntervalSyntax(orig.to_string());
    let (neg, body) = tok.strip_prefix('-').map_or_else(
        || (false, tok.strip_prefix('+').unwrap_or(tok)),
        |r| (true, r),
    );
    let parse_int = |p: &str| -> Result<i64> {
        if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
            p.parse::<i64>().map_err(|_| err())
        } else {
            Err(err())
        }
    };
    let parts: Vec<&str> = body.split(':').collect();
    // Accumulate the field's magnitude in µs, overflow-checked, then apply the sign.
    let (h, m, s_str) = match parts.as_slice() {
        [h, m] => (parse_int(h)?, parse_int(m)?, "0"),
        [h, m, s] => (parse_int(h)?, parse_int(m)?, *s),
        _ => return Err(err()),
    };
    let (sec_int, sec_frac) = parse_number(s_str).ok_or_else(err)?;
    if sec_int < 0 || sec_frac < 0.0 {
        return Err(err()); // a bare negative inside the field is malformed
    }
    let whole_secs = h
        .checked_mul(3600)
        .and_then(|x| m.checked_mul(60).and_then(|y| x.checked_add(y)))
        .and_then(|x| x.checked_add(sec_int))
        .ok_or_else(err)?;
    let usec = whole_secs
        .checked_mul(1_000_000)
        .and_then(|x| x.checked_add(rint(sec_frac * 1_000_000.0)))
        .ok_or_else(err)?;
    let signed = if neg { -usec } else { usec };
    itm.usec = itm.usec.checked_add(signed).ok_or_else(err)?;
    Ok(())
}

/// Parse the ISO-8601 `P…T…` grammar (`P1Y2M3DT4H5M6S`): `Y`/`M`/`W`/`D` before
/// the `T` boundary, `H`/`M`/`S` after — so `M` is months before `T`, minutes
/// after. Only the designator form is accepted (the alternate `P0001-02-03`
/// spellings fail loud — out of scope).
fn parse_iso8601(s: &str, orig: &str, itm: &mut Itm) -> Result<()> {
    let err = || Error::InvalidIntervalSyntax(orig.to_string());
    let rest = &s[1..]; // after the leading `P`
    if rest.is_empty() {
        return Err(err());
    }
    let mut in_time = false;
    let mut num = String::new();
    let mut consumed = false;
    for ch in rest.chars() {
        if ch == 'T' || ch == 't' {
            if !num.is_empty() {
                return Err(err());
            }
            in_time = true;
            continue;
        }
        if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+' {
            num.push(ch);
            continue;
        }
        if ch.is_ascii_alphabetic() {
            let (iv, frac) = parse_number(&num).ok_or_else(err)?;
            num.clear();
            let unit = match (ch.to_ascii_uppercase(), in_time) {
                ('Y', false) => Unit::Year(1),
                ('M', false) => Unit::Month,
                ('W', false) => Unit::Week,
                ('D', false) => Unit::Day,
                ('H', true) => Unit::Hour,
                ('M', true) => Unit::Minute,
                ('S', true) => Unit::Second,
                _ => return Err(err()),
            };
            apply_number(itm, iv, frac, unit)?;
            consumed = true;
            continue;
        }
        return Err(err()); // any other character (e.g. the alternate `-`/`:` forms)
    }
    if !num.is_empty() || !consumed {
        return Err(err());
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{PgInterval, format_interval_text};

    /// Assert an interval's fields explicitly — estimate-equality makes `==`
    /// collapse `30 days` and `1 mon`, so field-level tests must destructure.
    fn assert_fields(iv: &PgInterval, months: i32, days: i32, micros: i64) {
        assert_eq!(
            (iv.months, iv.days, iv.micros),
            (months, days, micros),
            "interval fields"
        );
    }

    fn p(s: &str) -> PgInterval {
        PgInterval::parse(s).unwrap()
    }

    #[test]
    fn verbose_multi_unit_and_fractional_canonical() {
        // The canonical Postgres normalization example.
        let iv = p("2 years 15 months 100 weeks 99 hours 123456789 milliseconds");
        // months: 2*12 + 15 = 39; days: 100*7 = 700; µs: 99h + 123456789ms.
        let micros = 99 * 3_600_000_000 + 123_456_789 * 1000;
        assert_fields(&iv, 39, 700, micros);
        assert_eq!(
            format_interval_text(&iv),
            "3 years 3 mons 700 days 133:17:36.789"
        );
    }

    #[test]
    fn fractional_years_round_to_months() {
        assert_fields(&p("1.5 years"), 18, 0, 0); // 1 year 6 mons
        assert_eq!(format_interval_text(&p("1.5 years")), "1 year 6 mons");
    }

    #[test]
    fn fractional_months_spill_to_days_and_time() {
        // 1.75 months → 1 mon + 0.75*30 = 22.5 days → 22 days + 0.5 day = 12:00:00.
        assert_fields(&p("1.75 months"), 1, 22, 12 * 3_600_000_000);
        assert_eq!(
            format_interval_text(&p("1.75 months")),
            "1 mon 22 days 12:00:00"
        );
    }

    #[test]
    fn sql_standard_forms() {
        assert_fields(&p("1-2"), 14, 0, 0); // 1 year 2 mons
        assert_eq!(format_interval_text(&p("1-2")), "1 year 2 mons");
        // `3 4:05:06` → 3 days + time; the bare 3 becomes days.
        assert_fields(
            &p("3 4:05:06"),
            0,
            3,
            4 * 3_600_000_000 + 5 * 60_000_000 + 6_000_000,
        );
        // Leading negative day, positive time (`-1 2:03:04`).
        let iv = p("-1 2:03:04");
        assert_fields(&iv, 0, -1, 2 * 3_600_000_000 + 3 * 60_000_000 + 4_000_000);
        assert_eq!(format_interval_text(&iv), "-1 days +02:03:04");
    }

    #[test]
    fn iso8601_month_vs_minute_boundary() {
        // `M` before T is months, after T is minutes.
        let iv = p("P1Y2M3DT4H5M6S");
        assert_fields(&iv, 14, 3, 4 * 3_600_000_000 + 5 * 60_000_000 + 6_000_000);
        assert_eq!(format_interval_text(&iv), "1 year 2 mons 3 days 04:05:06");
    }

    #[test]
    fn ago_negates_all_fields() {
        let iv = p("1 year 2 mons 3 days ago");
        assert_fields(&iv, -14, -3, 0);
    }

    #[test]
    fn negative_field_pluralizes_and_forces_sign() {
        // `-14 months -3 days -4:05:06` → `-1 years -2 mons -3 days -04:05:06`.
        let iv = PgInterval::new(-14, -3, -(4 * 3_600_000_000 + 5 * 60_000_000 + 6_000_000));
        assert_eq!(
            format_interval_text(&iv),
            "-1 years -2 mons -3 days -04:05:06"
        );
        // Mixed sign forces a `+` on the positive day after negative months.
        let mixed = PgInterval::new(-14, 3, -(4 * 3_600_000_000 + 5 * 60_000_000 + 6_000_000));
        assert_eq!(
            format_interval_text(&mixed),
            "-1 years -2 mons +3 days -04:05:06"
        );
    }

    #[test]
    fn zero_renders_time_group() {
        assert_eq!(format_interval_text(&PgInterval::new(0, 0, 0)), "00:00:00");
        assert_eq!(format_interval_text(&p("0")), "00:00:00");
    }

    #[test]
    fn estimate_collapses_days_and_months() {
        // 30 days == 1 mon under the estimate (so `==` is true), but fields differ.
        let a = p("30 days");
        let b = p("1 mon");
        assert_eq!(a, b);
        assert_fields(&a, 0, 30, 0);
        assert_fields(&b, 1, 0, 0);
    }

    #[test]
    fn wire_round_trip_preserves_fields() {
        let iv = PgInterval::new(14, 3, 4 * 3_600_000_000 + 5 * 60_000_000 + 6_000_000);
        let bytes = iv.to_wire_bytes();
        let back = PgInterval::recv(&mut crate::types::BinaryReader::new(&bytes)).unwrap();
        assert_fields(&back, iv.months, iv.days, iv.micros);
        // Wire order is time (i64) → day (i32) → month (i32).
        assert_eq!(&bytes[0..8], &iv.micros.to_be_bytes());
        assert_eq!(&bytes[8..12], &iv.days.to_be_bytes());
        assert_eq!(&bytes[12..16], &iv.months.to_be_bytes());
    }

    #[test]
    fn index_key_orders_by_estimate() {
        let neg = p("-1 day");
        let zero = p("0");
        let pos = p("1 mon"); // estimate 30 days > 1 day
        assert!(neg.index_key_bytes() < zero.index_key_bytes());
        assert!(zero.index_key_bytes() < pos.index_key_bytes());
    }

    #[test]
    fn malformed_and_rejected_units_fail_loud() {
        for bad in ["", "  ", "quarter", "1 quarter", "3 blah", "P", "PT", "1-"] {
            let e = PgInterval::parse(bad);
            assert!(e.is_err(), "{bad:?} must fail to parse");
        }
        // The malformed-syntax error is SQLSTATE 22007 (interval_in), not 22P02.
        assert_eq!(
            PgInterval::parse("1 quarter").unwrap_err().sqlstate(),
            "22007"
        );
    }

    #[test]
    fn integer_field_is_exact_above_f64_precision() {
        // The whole-number part parses via i64 (exact), not f64 — `9007199254740993`
        // survives (an f64 would round it to …992). Matches Postgres.
        assert_fields(
            &p("9007199254740993 microseconds"),
            0,
            0,
            9_007_199_254_740_993,
        );
    }

    #[test]
    fn exponent_forms_and_out_of_range_integers_fail_loud() {
        // Postgres's `strtoi64` stops at `e`, leaving trailing input → bad format.
        for bad in ["1e3 days", "1e300 microseconds", "1E3 hours"] {
            let e = PgInterval::parse(bad).unwrap_err();
            assert_eq!(
                e.sqlstate(),
                "22007",
                "{bad:?} must be a 22007 syntax error"
            );
        }
        // An integer past i64 fails loud (never silently saturates to i64::MAX).
        assert!(PgInterval::parse("99999999999999999999999 microseconds").is_err());
    }

    #[test]
    fn signed_sql_standard_year_month() {
        // A leading sign on `Y-M` applies to both parts (Postgres `-1-2` = -14 months).
        assert_fields(&p("-1-2"), -14, 0, 0);
        assert_fields(&p("+1-2"), 14, 0, 0);
        assert_eq!(format_interval_text(&p("-1-2")), "-1 years -2 mons");
    }

    #[test]
    fn mils_is_millennium() {
        // The `mils` abbreviation maps to millennium (2000 years).
        assert_fields(&p("2 mils"), 24_000, 0, 0);
        assert_eq!(format_interval_text(&p("2 mils")), "2000 years");
    }

    #[test]
    fn sub_microsecond_fraction_truncates_then_bumps() {
        // Postgres's `AdjustFractMicroseconds` truncates toward zero then bumps only
        // if the leftover magnitude strictly exceeds ½ µs — NOT ties-to-even. So an
        // exact ½ µs truncates (`0.0000005 s` → 0) while `0.0000015 s` → 1 µs.
        assert_fields(&p("0.0000005 seconds"), 0, 0, 0);
        assert_fields(&p("0.0000015 seconds"), 0, 0, 1);
        assert_fields(&p("0.0000025 seconds"), 0, 0, 2);
        // The year fraction still rounds ties-to-even (`rint`): `1.5 years` → 18 mons.
        assert_fields(&p("1.5 years"), 18, 0, 0);
    }

    #[test]
    fn parse_overflow_is_loud_22015() {
        // A parse-time field overflow fails loud with `22015` (interval_field_overflow),
        // not `22008` and not a silent wrap. A year count that overflows the month field
        // and a plain oversized year both route through the decode overflow path.
        for bad in ["9223372036854775807 years", "999999999999 years"] {
            let e = PgInterval::parse(bad).unwrap_err();
            assert_eq!(e.sqlstate(), "22015", "{bad:?} must be a 22015 overflow");
        }
    }
}
