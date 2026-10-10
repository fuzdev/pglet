use time::{Date, Month, OffsetDateTime};

use super::{Infinite, PgDate, PgTimestamp, TypeOid, Value};
use crate::error::{Error, Result};

/// Format UUID bytes as the canonical `8-4-4-4-12` lowercase-hex string.
///
/// The single source for UUID text rendering, shared by the wire text format
/// (server `value_to_bytes`), the `uuid::text` cast, and array elements.
#[must_use]
pub fn format_uuid(bytes: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

/// Parse a UUID string (dashed or bare hex) to bytes, or `None` if malformed.
///
/// The inverse of `format_uuid`: trims, drops dashes, and requires exactly 32
/// hex digits. The hex-digit guard also keeps the byte slicing on ASCII char
/// boundaries, so non-ASCII input is rejected rather than panicking.
#[must_use]
pub fn parse_uuid_bytes(s: &str) -> Option<[u8; 16]> {
    let s = s.trim().replace('-', "");
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

/// Format bytes as a Postgres `bytea` text literal (`\xdeadbeef`, `\x` for empty).
///
/// The single source for `bytea` text rendering, shared by the wire text format
/// (server `value_to_bytes`), the `bytea::text` cast, and array elements.
#[must_use]
pub fn format_bytea_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("\\x");
    push_lower_hex(&mut s, bytes);
    s
}

/// Append two lower-case hex digits per byte of `bytes` to `out` — the one hex
/// encoder behind `bytea`'s text form and `md5()`'s answer.
pub fn push_lower_hex(out: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 15)]));
    }
}

/// Render a timestamp as Postgres text: `YYYY-MM-DD HH:MM:SS[.ffffff]`, or
/// `infinity` / `-infinity`.
///
/// Space-separated (not RFC3339's `T`/`Z`), with fractional seconds trimmed of
/// trailing zeros and omitted entirely when zero — matching Postgres's `timestamp`
/// output. The single source for timestamp text (wire format + the `::text` cast).
#[must_use]
pub fn format_timestamp_text(ts: &PgTimestamp) -> String {
    match ts {
        Infinite::Finite(ts) => format_finite_timestamp_text(ts),
        _ => infinity_string(ts),
    }
}

/// The text of an infinity, for a value [`Infinite::infinity_text`] names.
fn infinity_string<T>(v: &Infinite<T>) -> String {
    v.infinity_text().unwrap_or_default().to_string()
}

/// `YYYY-MM-DD` for a calendar date's fields, the year as `time`'s `[year]` prints it.
///
/// Hand-written rather than `time`'s `format_description!`, whose formatting machinery
/// cost ~9.5 KB of wasm for these two call sites (`TRADEOFFS.md` § Release profile). It
/// matches `[year]` for every year `time` holds — `large-dates` is off, so -9999
/// through 9999: four digits zero-padded and a `-` before a negative year. A year below
/// 1 never reaches it from a value the engine builds (the date domain, `temporal_domain`),
/// so the negative arm keeps `time`'s `[year]` rendering, not a case the engine produces.
fn calendar_date_text(year: i32, month: Month, day: u8) -> String {
    let sign = if year < 0 { "-" } else { "" };
    format!(
        "{sign}{:04}-{:02}-{day:02}",
        year.unsigned_abs(),
        u8::from(month)
    )
}

fn format_finite_timestamp_text(ts: &OffsetDateTime) -> String {
    let base = format!(
        "{} {:02}:{:02}:{:02}",
        calendar_date_text(ts.year(), ts.month(), ts.day()),
        ts.hour(),
        ts.minute(),
        ts.second()
    );
    let micros = ts.microsecond();
    if micros == 0 {
        base
    } else {
        let frac = format!("{micros:06}");
        format!("{base}.{}", frac.trim_end_matches('0'))
    }
}

/// Render a `timestamptz` as Postgres text: the UTC wall-clock plus a `+00` offset,
/// or `infinity` / `-infinity`, which carry no offset.
///
/// pglet has no session time zone — every `timestamptz` is stored and rendered in
/// UTC — so the offset is always `+00`, matching Postgres run with `TimeZone=UTC`.
/// The trailing offset is load-bearing: a client driver (node-postgres, the fuz_app
/// matrix) parses an *offset-less* timestamptz text value in its own local zone,
/// silently shifting the instant; the explicit `+00` pins it to UTC. The single
/// source for `timestamptz` wire text (vs `format_timestamp_text`, which is the
/// offset-less form for `timestamp without time zone`).
#[must_use]
pub fn format_timestamptz_text(ts: &PgTimestamp) -> String {
    match ts {
        Infinite::Finite(ts) => format!("{}+00", format_finite_timestamp_text(ts)),
        _ => infinity_string(ts),
    }
}

/// Parse a `timestamptz` string, applying any explicit offset (normalized to UTC).
///
/// The inverse of `format_timestamptz_text`, and the input twin of `timestamptz_in`: the
/// spelling is split into fields as 18.6's `ParseDateTime` splits it — any whitespace or
/// otherwise unclaimed ASCII punctuation between fields, `t` in either case between the
/// date and the time — and read as its `DecodeDateTime` reads the field kinds modelled
/// (`decode_timestamp_fields`): a date, a time of day, a numeric offset
/// (`decode_timezone`: `+05`, `+5`, `-08:30`, `+05:30:15`, `+0530`), the word `z`, and a
/// POSIX zone with no daylight-saving part (`utc+16`). No offset assumes UTC, and a date
/// alone is midnight. An explicit offset **converts** the instant to UTC (`12:00:00+05` →
/// `07:00:00`) — pglet stores every `timestamptz` as UTC.
///
/// The tz-aware half of the pair; use `parse_timestamp_naive` for a plain
/// `timestamp without time zone` (which drops the offset instead of applying it),
/// mirroring Postgres's `timestamptz_in` vs `timestamp_in` split.
///
/// # Errors
///
/// Returns `Error::InvalidInputSyntax` for a spelling 18.6 refuses as malformed or reads
/// through a field kind not modelled here, `Error::TimeZoneDisplacementOutOfRange`
/// (`22009`) for a numeric offset outside `DecodeTimezone`'s ranges,
/// `Error::DatetimeFieldValueOutOfRange` (`22008`) for a time of day or a date field out
/// of range — each where 18.6's field loop or its date check raises it — and
/// `Error::TemporalOutOfRange` (`22008`) when the instant, after the offset, is outside
/// the date domain. An `infinity` spelling ([`super::parse_infinity`]) is read first.
pub fn parse_timestamp(s: &str) -> Result<PgTimestamp> {
    parse_timestamp_inner(s, true)
}

/// Parse a plain `timestamp` string, **dropping** any explicit offset.
///
/// The tz-naive twin of `parse_timestamp`, and the input mirror of Postgres's
/// `timestamp_in`: it keeps the literal wall-clock fields exactly as written and
/// ignores any trailing offset (`12:00:00+05` → `12:00:00`, not `07:00:00`) — a
/// plain `timestamp without time zone` carries no zone, so an offset in the input
/// is discarded rather than applied, once it has been read: an offset
/// `parse_timestamp` refuses is refused here too, as `timestamp_in` refuses it. Naive
/// and date-only spellings behave identically to `parse_timestamp`.
///
/// This is the input-side mirror of the offset-aware output asymmetry
/// (`format_timestamptz_text` renders a `+00`, `format_timestamp_text` renders
/// none): a plain-`timestamp` target must not silently shift the instant.
///
/// # Errors
///
/// As [`parse_timestamp`], the domain judged on the written fields the offset leaves.
pub fn parse_timestamp_naive(s: &str) -> Result<PgTimestamp> {
    parse_timestamp_inner(s, false)
}

/// Shared parser for `parse_timestamp` (tz-aware) and `parse_timestamp_naive`
/// (tz-naive). `apply_offset` selects how an explicit offset is resolved: `true`
/// converts the instant to UTC (Postgres `timestamptz_in`), `false` drops the
/// offset and keeps the wall-clock fields as UTC (Postgres `timestamp_in`). Naive
/// and date-only inputs carry no offset, so both modes assume UTC there.
///
/// The spelling is split into fields by [`split_datetime_fields`] and the fields are
/// read by [`decode_timestamp_fields`], the two halves of 18.6's `ParseDateTime` and
/// `DecodeDateTime` for the field kinds this reader models. The result is inside the
/// domain (`super::check_timestamp_domain`), judged on the instant after the offset.
fn parse_timestamp_inner(s: &str, apply_offset: bool) -> Result<PgTimestamp> {
    if let Some(inf) = super::parse_infinity(s) {
        return Ok(inf);
    }
    let micros = split_datetime_fields(s)
        .ok_or(DtError::BadFormat)
        .and_then(|fields| decode_timestamp_fields(&fields, apply_offset))
        .map_err(|e| match e {
            DtError::BadFormat => Error::InvalidInputSyntax {
                type_name: "timestamp",
                value: s.trim().to_string(),
            },
            DtError::FieldOverflow => Error::DatetimeFieldValueOutOfRange(s.to_string()),
            DtError::ZoneOverflow => Error::TimeZoneDisplacementOutOfRange(s.to_string()),
        })?;
    micros
        .and_then(PgTimestamp::from_pg_micros)
        .and_then(|ts| match ts {
            Infinite::Finite(ts) => super::check_timestamp_domain(ts).ok(),
            _ => None,
        })
        .map(Infinite::Finite)
        .ok_or(Error::TemporalOutOfRange("timestamp"))
}

/// The kind 18.6's `ParseDateTime` gives a field: `DTK_NUMBER`, `DTK_DATE`, `DTK_TIME`,
/// `DTK_STRING`, `DTK_SPECIAL` and `DTK_TZ`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DtField {
    Number,
    Date,
    Time,
    Text,
    Special,
    Zone,
}

/// Why a date/time field was refused, as 18.6's `DTERR_*` codes: `DTERR_BAD_FORMAT`
/// (`22007` on 18.6, this reader's `22P02` — `EDGES.md` row 96), `DTERR_FIELD_OVERFLOW`
/// and `DTERR_MD_FIELD_OVERFLOW` (`22008`), and `DTERR_TZDISP_OVERFLOW` (`22009`).
#[derive(Debug, PartialEq, Eq)]
enum DtError {
    BadFormat,
    FieldOverflow,
    ZoneOverflow,
}

/// The all-letter tokens of 18.6's `datetktbl`. `ParseDateTime` looks a word up here when
/// a digit or `+` follows it: a word in the table stays a word (`t` in
/// `2000-01-01T12:00:00`, and `july` in `2000-01-01 00:00:00 july5`, which is then
/// malformed), and any other word runs on into one field, which after a date is read as a
/// time zone name (`utc+16`). The lookup is a membership test, so the entries' order
/// decides nothing.
const DATE_TOKENS: &[&str] = &[
    "ad",
    "allballs",
    "am",
    "apr",
    "april",
    "at",
    "aug",
    "august",
    "bc",
    "d",
    "dec",
    "december",
    "dow",
    "doy",
    "dst",
    "epoch",
    "feb",
    "february",
    "fri",
    "friday",
    "h",
    "infinity",
    "isodow",
    "isoyear",
    "j",
    "jan",
    "january",
    "jd",
    "jul",
    "july",
    "julian",
    "jun",
    "june",
    "m",
    "mar",
    "march",
    "may",
    "mm",
    "mon",
    "monday",
    "nov",
    "november",
    "now",
    "oct",
    "october",
    "on",
    "pm",
    "s",
    "sat",
    "saturday",
    "sep",
    "sept",
    "september",
    "sun",
    "sunday",
    "t",
    "thu",
    "thur",
    "thurs",
    "thursday",
    "today",
    "tomorrow",
    "tue",
    "tues",
    "tuesday",
    "wed",
    "wednesday",
    "weds",
    "y",
    "yesterday",
];

/// Split a date/time spelling into fields as 18.6's `ParseDateTime` does, or `None`
/// where it answers `DTERR_BAD_FORMAT`.
///
/// C `isspace` separates fields and is dropped; a field starting with a digit is a time
/// (`12:34:56.5`, digits then `:` and every following digit, `:` and `.`), a date (digits
/// then `-`, `/` or `.` and more digits, the same delimiter repeated — `2000-01-01`), or
/// a number; a word is lowercased, and runs on into a date-kind field when a `-`, `/` or
/// `.` follows it, or a digit or `+` does and the word is not in [`DATE_TOKENS`]; a sign
/// starts an offset (whitespace after it skipped, then a digit and every following
/// digit, `:`, `.` and `-`) or a signed word. Any other ASCII punctuation separates
/// fields and is dropped, so `2000-01-01/12:00:00` and `2000-01-01_12:00:00` are two
/// fields; a control or non-ASCII byte is malformed. 18.6 writes the fields into a
/// buffer of `MAXDATELEN + MAXDATEFIELDS` bytes and allows `MAXDATEFIELDS` fields, and
/// an input past either is malformed there too.
fn split_datetime_fields(input: &str) -> Option<Vec<(DtField, Vec<u8>)>> {
    const MAX_FIELDS: usize = 25;
    const BUF_LEN: usize = 128 + MAX_FIELDS;
    let bytes = input.as_bytes();
    let at = |pos: usize| bytes.get(pos).copied().unwrap_or(0);
    let space = |lead: u8| matches!(lead, b' ' | b'\t'..=b'\r');
    let mut fields: Vec<(DtField, Vec<u8>)> = Vec::new();
    let mut used = 0;
    let mut pos = 0;
    while pos < bytes.len() {
        let lead = bytes[pos];
        if space(lead) {
            pos += 1;
            continue;
        }
        if fields.len() >= MAX_FIELDS {
            return None;
        }
        let mut field = Vec::new();
        // `APPEND_CHAR`: the byte and the field's terminator must fit the buffer.
        let push = |field: &mut Vec<u8>, byte: u8| {
            (used + field.len() + 1 < BUF_LEN).then(|| field.push(byte.to_ascii_lowercase()))
        };
        let kind = if lead.is_ascii_digit() {
            while at(pos).is_ascii_digit() {
                push(&mut field, at(pos))?;
                pos += 1;
            }
            match at(pos) {
                b':' => {
                    while at(pos).is_ascii_digit() || matches!(at(pos), b':' | b'.') {
                        push(&mut field, at(pos))?;
                        pos += 1;
                    }
                    DtField::Time
                }
                delim @ (b'-' | b'/' | b'.') => {
                    push(&mut field, delim)?;
                    pos += 1;
                    if at(pos).is_ascii_digit() {
                        let mut kind = if delim == b'.' {
                            DtField::Number
                        } else {
                            DtField::Date
                        };
                        while at(pos).is_ascii_digit() {
                            push(&mut field, at(pos))?;
                            pos += 1;
                        }
                        if at(pos) == delim {
                            kind = DtField::Date;
                            while at(pos).is_ascii_digit() || at(pos) == delim {
                                push(&mut field, at(pos))?;
                                pos += 1;
                            }
                        }
                        kind
                    } else {
                        while at(pos).is_ascii_alphanumeric() || at(pos) == delim {
                            push(&mut field, at(pos))?;
                            pos += 1;
                        }
                        DtField::Date
                    }
                }
                _ => DtField::Number,
            }
        } else if lead == b'.' {
            push(&mut field, lead)?;
            pos += 1;
            while at(pos).is_ascii_digit() {
                push(&mut field, at(pos))?;
                pos += 1;
            }
            DtField::Number
        } else if lead.is_ascii_alphabetic() {
            while at(pos).is_ascii_alphabetic() {
                push(&mut field, at(pos))?;
                pos += 1;
            }
            let runs_on = matches!(at(pos), b'-' | b'/' | b'.')
                || ((at(pos) == b'+' || at(pos).is_ascii_digit())
                    && std::str::from_utf8(&field).is_ok_and(|w| !DATE_TOKENS.contains(&w)));
            if runs_on {
                while matches!(at(pos), b'+' | b'-' | b'/' | b'_' | b'.' | b':')
                    || at(pos).is_ascii_alphanumeric()
                {
                    push(&mut field, at(pos))?;
                    pos += 1;
                }
                DtField::Date
            } else {
                DtField::Text
            }
        } else if lead == b'+' || lead == b'-' {
            push(&mut field, lead)?;
            pos += 1;
            while space(at(pos)) {
                pos += 1;
            }
            if at(pos).is_ascii_digit() {
                while at(pos).is_ascii_digit() || matches!(at(pos), b':' | b'.' | b'-') {
                    push(&mut field, at(pos))?;
                    pos += 1;
                }
                DtField::Zone
            } else if at(pos).is_ascii_alphabetic() {
                while at(pos).is_ascii_alphabetic() {
                    push(&mut field, at(pos))?;
                    pos += 1;
                }
                DtField::Special
            } else {
                return None;
            }
        } else if lead.is_ascii_punctuation() {
            pos += 1;
            continue;
        } else {
            return None;
        };
        used += field.len() + 1;
        fields.push((kind, field));
    }
    Some(fields)
}

/// Read the fields of a `timestamp` / `timestamptz` spelling as 18.6's `DecodeDateTime`
/// does, returning microseconds since `2000-01-01` UTC — `None` for a date 18.6's
/// `IS_VALID_JULIAN` refuses or a count past `i64`, each `timestamp out of range` there.
/// The date domain is the caller's to judge.
///
/// The kinds modelled, each at most once and in any order save a date after the time of
/// day, which `DecodeDate` refuses: a `y-m-d` date whose year field has at least three
/// digits; a time of day (`DecodeTime`); a numeric offset
/// (`decode_timezone`), the word `z`, or — once the date is in hand — a POSIX zone with
/// no daylight-saving part (`posix_zone_seconds`: `utc+16`, `x5`); and a `t` between the
/// date and the time. Every other field is malformed here: a month name, a
/// day-of-year, a run-together `hhmmss`, a named zone, an `am`/`pm` or `at`. Each field
/// answers where 18.6's field loop does, so a time of day out of range answers before a
/// later offset out of range, and the date's ranges are judged after the loop.
///
/// The seconds field may be `60` and the hour `24`, as `time_overflows` allows, so long
/// as the time of day is at most `24:00:00`: `12:30:60.5` is `12:31:00.5`, and
/// `23:59:60.5` is out of range.
fn decode_timestamp_fields(
    fields: &[(DtField, Vec<u8>)],
    apply_offset: bool,
) -> std::result::Result<Option<i64>, DtError> {
    let mut date = None;
    let mut time = None;
    let mut zone = None;
    let mut iso_t = false;
    for (kind, f) in fields {
        match kind {
            DtField::Date if date.is_some() => {
                // A date-kind field after the date is a zone: a run-together time with
                // an offset when it starts with a digit, a zone name otherwise.
                if iso_t || f[0].is_ascii_digit() {
                    return Err(DtError::BadFormat);
                }
                let seconds = posix_zone_seconds(f).ok_or(DtError::BadFormat)?;
                if zone.replace(seconds).is_some() {
                    return Err(DtError::BadFormat);
                }
            }
            DtField::Date => {
                // `DecodeDate` judges its fields against every field already read, so a
                // date after the time of day is malformed; an offset is not counted.
                let ymd = decode_date(f)?;
                if time.is_some() {
                    return Err(DtError::BadFormat);
                }
                date = Some(ymd);
            }
            DtField::Time => {
                iso_t = false;
                let micros = decode_time(f)?;
                if time.replace(micros).is_some() {
                    return Err(DtError::BadFormat);
                }
            }
            DtField::Zone => {
                let seconds = decode_timezone(f).map(i64::from)?;
                if zone.replace(seconds).is_some() {
                    return Err(DtError::BadFormat);
                }
            }
            DtField::Text if f == b"t" => {
                if date.is_none() || iso_t {
                    return Err(DtError::BadFormat);
                }
                iso_t = true;
            }
            DtField::Text if f == b"z" => {
                if zone.replace(0).is_some() {
                    return Err(DtError::BadFormat);
                }
            }
            _ => return Err(DtError::BadFormat),
        }
    }
    let Some((year, month, day)) = date.filter(|_| !iso_t) else {
        return Err(DtError::BadFormat);
    };
    // `ValidateDate`: the year, then the month and day ranges, then the month's length.
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year <= 0
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || day > month_days[(month - 1) as usize]
    {
        return Err(DtError::FieldOverflow);
    }
    // `tm2timestamp`: `IS_VALID_JULIAN`, `date2j`, then the offset.
    if year > 5_874_898 || (year == 5_874_898 && month >= 6) {
        return Ok(None);
    }
    let (y, m) = if month > 2 {
        (year + 4800, month + 1)
    } else {
        (year + 4799, month + 13)
    };
    let century = y / 100;
    let julian = y * 365 - 32167 + y / 4 - century + century / 4 + 7834 * m / 256 + day;
    let offset = if apply_offset { zone.unwrap_or(0) } else { 0 };
    Ok((julian - 2_451_545)
        .checked_mul(86_400_000_000)
        .and_then(|us| us.checked_add(time.unwrap_or(0)))
        .and_then(|us| us.checked_sub(offset * 1_000_000)))
}

/// 18.6's `DecodeDate` over a date-kind field, for the `y-m-d` order: the digit runs
/// between separators, the first of at least three digits (a shorter one is read in
/// `DateStyle` order, which is not modelled), the second not a three-digit day of year.
/// One separator after the last run is dropped, as `DecodeDate` drops it.
fn decode_date(f: &[u8]) -> std::result::Result<(i64, i64, i64), DtError> {
    let mut parts = [0_i64; 3];
    let mut count = 0;
    let mut i = 0;
    while i < f.len() {
        while i < f.len() && !f[i].is_ascii_alphanumeric() {
            i += 1;
        }
        let start = i;
        while i < f.len() && f[i].is_ascii_digit() {
            i += 1;
        }
        let len = i - start;
        if len == 0 || count == 3 || (count == 0 && len < 3) {
            // End of string after a separator, a month name, a fourth field or a
            // `DateStyle`-ordered one: malformed, or not modelled.
            return Err(DtError::BadFormat);
        }
        let value = strtoint(f, start).ok_or(DtError::FieldOverflow)?.0;
        if count == 1 && len == 3 && (1..=366).contains(&value) {
            // A day of year (`2000-060`), which `DecodeNumber` reads ahead of a month.
            return Err(DtError::BadFormat);
        }
        parts[count] = i64::from(value);
        count += 1;
        i += 1;
    }
    if count < 3 {
        return Err(DtError::BadFormat);
    }
    Ok(parts.into())
}

/// The digits after a time field's `.`, through [`parse_frac_micros`]: `.` alone is `0`,
/// and anything but digits after it is malformed.
fn fraction_micros(digits: &[u8]) -> std::result::Result<i64, DtError> {
    std::str::from_utf8(digits)
        .ok()
        .and_then(parse_frac_micros)
        .ok_or(DtError::BadFormat)
}

/// 18.6's `DecodeTime` and `time_overflows` over a time field, returning microseconds
/// since midnight.
///
/// `hour:minute`, `hour:minute:second` and `minute:second.fraction` (a fraction after
/// two fields reads them as minutes and seconds, so `12:30.5` is `00:12:30.5`); an empty
/// field is `0` (`12::30` is `12:00:30`). The fraction is `rint(strtod(fraction) *
/// 1000000)`, rounded as a C `double`, so `.0001255` is `.000125` and `.0001265` is
/// `.000127` — not a decimal rounding. Each field is range-checked on its own (a minute
/// to `59`, a second to `60`), then the whole time of day to `24:00:00`.
fn decode_time(f: &[u8]) -> std::result::Result<i64, DtError> {
    // `strtoi64`: the field starts with a digit, and a count past `i64` is `ERANGE`.
    let mut cp = f.iter().take_while(|b| b.is_ascii_digit()).count();
    let hour = f[..cp]
        .iter()
        .try_fold(0_i64, |n, &b| {
            n.checked_mul(10)?.checked_add(i64::from(b - b'0'))
        })
        .ok_or(DtError::FieldOverflow)?;
    let (minute, end) = strtoint(f, cp + 1).ok_or(DtError::FieldOverflow)?;
    cp = end;
    let (mut h, mut m, mut s, mut us) = (hour, i64::from(minute), 0, 0);
    match f.get(cp) {
        None => {}
        Some(b'.') => {
            us = fraction_micros(&f[cp + 1..])?;
            (h, m, s) = (0, hour, i64::from(minute));
        }
        Some(b':') => {
            let (second, end) = strtoint(f, cp + 1).ok_or(DtError::FieldOverflow)?;
            s = i64::from(second);
            match f.get(end) {
                None => {}
                Some(b'.') => us = fraction_micros(&f[end + 1..])?,
                Some(_) => return Err(DtError::BadFormat),
            }
        }
        Some(_) => return Err(DtError::BadFormat),
    }
    let total = ((h.min(25) * 60 + m.min(60)) * 60 + s) * 1_000_000 + us;
    if h > 24 || !(0..60).contains(&m) || !(0..=60).contains(&s) || total > 86_400_000_000 {
        return Err(DtError::FieldOverflow);
    }
    Ok(total)
}

/// A zone name 18.6's `pg_tzset` reads as a POSIX time zone with no daylight-saving part,
/// returning its seconds east of UTC: a name (everything up to a digit, `,`, `+` or `-`),
/// then an offset `[+|-]hh[:mm[:ss]]` with the hour at most `167`, the minute `59` and
/// the second `60`, ending the field. POSIX counts the offset **west**, so `utc+16` is
/// sixteen hours behind UTC. `None` for anything else — a named zone, a daylight-saving
/// rule, an offset out of those ranges — which 18.6 reads, or refuses as
/// `22023 time zone "…" not recognized`.
fn posix_zone_seconds(f: &[u8]) -> Option<i64> {
    let mut i = f
        .iter()
        .position(|&b| b.is_ascii_digit() || matches!(b, b',' | b'+' | b'-'))?;
    let west = match f[i] {
        b'-' => {
            i += 1;
            -1
        }
        b'+' => {
            i += 1;
            1
        }
        _ => 1,
    };
    let mut seconds = 0;
    for (max, scale) in [(167, 3600), (59, 60), (60, 1)] {
        if scale != 3600 {
            if f.get(i) != Some(&b':') {
                break;
            }
            i += 1;
        }
        let digits = f[i..].iter().take_while(|b| b.is_ascii_digit()).count();
        let num = f[i..i + digits]
            .iter()
            .try_fold(0_i64, |n, &b| {
                Some(n * 10 + i64::from(b - b'0')).filter(|&n| n <= max)
            })
            .filter(|_| digits > 0)?;
        seconds += num * scale;
        i += digits;
    }
    (i == f.len()).then_some(-west * seconds)
}

/// Read a numeric UTC offset as 18.6's `DecodeTimezone` does, returning its seconds
/// east of UTC.
///
/// `field` is the `DTK_TZ` field [`split_datetime_fields`] builds: a sign, then a digit
/// and every following digit, `:`, `.` and `-`. It is an hour, then `:minute` and
/// `:second` — or, with no `:` and a field longer than three bytes counting the sign, the
/// digits run together as `hhmm` (`+0530`, `+155` is `01:55`) — each through `strtoint`,
/// so a field with no digits is `0` and a `-` inside one is a sign. The range checks
/// (`hour ..= 15`, `minute` and `second` `0 ..= 59`, and an `int` overflow) answer
/// before the check that the whole field was read, so `+16.5` is out of range while
/// `+05.5` is malformed.
fn decode_timezone(field: &[u8]) -> std::result::Result<i32, DtError> {
    let (mut hour, mut cp) = strtoint(field, 1).ok_or(DtError::ZoneOverflow)?;
    let mut minute = 0;
    let mut second = 0;
    if field.get(cp) == Some(&b':') {
        (minute, cp) = strtoint(field, cp + 1).ok_or(DtError::ZoneOverflow)?;
        if field.get(cp) == Some(&b':') {
            (second, cp) = strtoint(field, cp + 1).ok_or(DtError::ZoneOverflow)?;
        }
    } else if cp == field.len() && field.len() > 3 {
        minute = hour % 100;
        hour /= 100;
    }
    if !(0..=15).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
        return Err(DtError::ZoneOverflow);
    }
    if cp != field.len() {
        return Err(DtError::BadFormat);
    }
    let seconds = (hour * 60 + minute) * 60 + second;
    Ok(if field[0] == b'-' { -seconds } else { seconds })
}

/// C's `strtoint(field + start, &end, 10)` over a field holding only digits, `:`, `.`
/// and `-`: an optional `-`, then digits. Returns the value and the index reading
/// stopped at — `start` itself when no digit follows, as `strtol` leaves `endptr` —
/// or `None` where `strtoint` sets `ERANGE`.
fn strtoint(field: &[u8], start: usize) -> Option<(i32, usize)> {
    let mut i = start;
    let negative = field.get(i) == Some(&b'-');
    if negative {
        i += 1;
    }
    let digits = i;
    let mut value: i64 = 0;
    while let Some(&b) = field.get(i)
        && b.is_ascii_digit()
    {
        value = (value * 10 + i64::from(b - b'0')).min(i64::from(i32::MAX) + 2);
        i += 1;
    }
    if i == digits {
        return Some((0, start));
    }
    let value = i32::try_from(if negative { -value } else { value }).ok()?;
    Some((value, i))
}

/// Render a `date` as Postgres text: `YYYY-MM-DD` (zero-padded, no time zone), or
/// `infinity` / `-infinity`.
///
/// The single source for date text (wire format + the `::text` cast), the
/// date-only twin of `format_timestamp_text`.
#[must_use]
pub fn format_date_text(d: &PgDate) -> String {
    match d {
        Infinite::Finite(d) => calendar_date_text(d.year(), d.month(), d.day()),
        _ => infinity_string(d),
    }
}

/// Parse a `YYYY-MM-DD` string as a `date`.
///
/// The inverse of `format_date_text`, and the single date parser shared by the
/// `CAST`/`::` coercion path and bound `date` parameters. Deliberately strict:
/// only the ISO `YYYY-MM-DD` calendar spelling (Postgres's default output form),
/// not the many `DateStyle` input variants — a differential engine matches
/// Postgres's *output*, and every date pglet emits is this form.
///
/// The month/day fields may be one **or** two digits (`2026-1-1` parses like
/// `2026-01-01`, matching Postgres's lenient field padding); a leading `-` (a BC /
/// negative year pglet does not model) is rejected.
///
/// # Errors
///
/// An unparseable spelling (wrong shape, non-digit field, leading `-`) returns
/// `Error::InvalidDatetimeFormat` (`22007`); a year of `0` returns
/// `Error::DatetimeFieldValueOutOfRange` (`22008`, 18.6's sentence quoting the input),
/// judged ahead of the month and day; any other well-formed value with an out-of-range
/// field (`2026-13-01`, `2026-02-30`, a year past 9999) returns
/// `Error::TemporalOutOfRange` (`22008`), matching Postgres's split between
/// `invalid_datetime_format` and `datetime_field_overflow`. An `infinity` spelling
/// ([`super::parse_infinity`]) is read first.
pub fn parse_date(s: &str) -> Result<PgDate> {
    if let Some(inf) = super::parse_infinity(s) {
        return Ok(inf);
    }
    let trimmed = s.trim();
    let format_err = || Error::InvalidDatetimeFormat {
        type_name: "date",
        value: s.to_string(),
    };
    // A leading `-` would be a BC / negative year — the `time` crate would silently
    // parse the sign, but pglet models no BC dates, so reject it loudly.
    if trimmed.starts_with('-') {
        return Err(format_err());
    }
    // Exactly three `-`-separated all-digit groups (month/day 1-or-2 digits). Parsing
    // the fields manually — rather than a fixed `[month]-[day]` format — accepts the
    // unpadded spelling Postgres accepts while still emitting the padded output form.
    let mut parts = trimmed.split('-');
    let (Some(y), Some(m), Some(d), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(format_err());
    };
    let group = |g: &str| -> Option<i64> {
        if g.is_empty() || !g.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        g.parse::<i64>().ok()
    };
    let (Some(year), Some(month), Some(day)) = (group(y), group(m), group(d)) else {
        return Err(format_err());
    };
    // A year of `0` is 18.6's `ValidateDate` refusal, which it checks ahead of the
    // month and day and words as a field overflow quoting the input.
    if year == 0 {
        return Err(Error::DatetimeFieldValueOutOfRange(s.to_string()));
    }
    // The fields are well-formed digits; an out-of-range value is `22008`.
    let range_err = || Error::TemporalOutOfRange("date");
    let year = i32::try_from(year).map_err(|_| range_err())?;
    let month = u8::try_from(month)
        .ok()
        .and_then(|m| Month::try_from(m).ok())
        .ok_or_else(range_err)?;
    let day = u8::try_from(day).map_err(|_| range_err())?;
    Date::from_calendar_date(year, month, day)
        .map_err(|_| range_err())
        .and_then(super::check_date_domain)
        .map(Infinite::Finite)
}

/// Microseconds in a full day — the inclusive upper bound of a `TIME` value's
/// range (`24:00:00`, Postgres's `USECS_PER_DAY`).
const USECS_PER_DAY: i64 = 86_400_000_000;

/// Render a `time` as Postgres text: `HH:MM:SS[.ffffff]` (zero-padded, no tz).
///
/// The single source for time text (wire format + the `::text` cast), the
/// time-of-day twin of `format_date_text`. `t` is microseconds since midnight in
/// `[0, 86_400_000_000]`; the `24:00:00` endpoint renders as `24:00:00`. Fractional
/// seconds are trimmed of trailing zeros and omitted entirely when zero, matching
/// Postgres's `EncodeTimeOnly`.
#[must_use]
pub fn format_time_text(t: i64) -> String {
    let micros_of_sec = t.rem_euclid(1_000_000);
    let total_secs = t.div_euclid(1_000_000);
    let secs = total_secs % 60;
    let total_mins = total_secs / 60;
    let mins = total_mins % 60;
    let hours = total_mins / 60;
    let base = format!("{hours:02}:{mins:02}:{secs:02}");
    if micros_of_sec == 0 {
        base
    } else {
        let frac = format!("{micros_of_sec:06}");
        format!("{base}.{}", frac.trim_end_matches('0'))
    }
}

/// Parse a Postgres `time` string into microseconds since midnight.
///
/// The inverse of `format_time_text`, and the single time parser shared by the
/// `CAST`/`::` coercion path and bound `time` parameters. Accepts the colon forms
/// (`HH:MM:SS.ffffff`, `HH:MM:SS`, `HH:MM`), the all-digit ISO basic forms
/// (`HHMMSS`, `HHMM`), and an optional 12-hour `AM`/`PM` suffix — applied *after*
/// field parsing, as Postgres does. Fractional seconds beyond 6 digits round to the
/// nearest microsecond (ties to even, C `rint`). The `24:00:00` endpoint is
/// accepted; any value past it is rejected.
///
/// A trailing time-zone offset (`-08:00`, `+05`, `Z`) is **rejected** (fail loud):
/// pglet is UTC-only with no time-zone database, so it is deliberately stricter than
/// Postgres (which accepts-and-ignores the offset for a plain `time`).
///
/// # Errors
///
/// An unrecognized `time` spelling returns `Error::InvalidDatetimeFormat` (`22007`); a
/// well-formed value with an out-of-range field (`25:00:00`, `12:99`) returns
/// `Error::TemporalOutOfRange` (`22008`), matching Postgres's `invalid_datetime_format`
/// vs `datetime_field_overflow` split.
pub fn parse_time(s: &str) -> Result<i64> {
    // An unparseable shape → 22007 (`invalid_datetime_format`); a well-formed field
    // out of range (`25:00:00`, `12:99`) → 22008 (`datetime_field_overflow`), matching
    // Postgres's split. (A tz suffix is a deliberately-stricter format rejection.)
    let format_err = || Error::InvalidDatetimeFormat {
        type_name: "time",
        value: s.to_string(),
    };
    let range_err = || Error::TemporalOutOfRange("time");

    // Split off an optional `AM`/`PM` meridian marker (case-insensitive, with or
    // without a leading space): `04:05 PM`, `4:00PM`, `12:00 AM`.
    let upper = s.trim().to_ascii_uppercase();
    let is_pm = if upper.ends_with("AM") {
        Some(false)
    } else if upper.ends_with("PM") {
        Some(true)
    } else {
        None
    };
    // `AM`/`PM` are ASCII, so trimming the two trailing bytes stays on a char boundary.
    let body = if is_pm.is_some() {
        upper[..upper.len() - 2].trim_end()
    } else {
        upper.as_str()
    };

    // A time-zone offset/zone marker in the body is rejected (pglet is UTC-only).
    if body.contains('+') || body.contains('-') || body.contains('Z') {
        return Err(format_err());
    }

    // Field extraction: colon form (`HH:MM[:SS[.f]]`) or all-digit form
    // (`HHMMSS[.f]` / `HHMM[.f]`). The `.fraction` suffix is split off first, so
    // the all-digit length check sees only the integer part (`134506.5` →
    // `13:45:06.5`, matching Postgres's `DecodeNumberField`).
    let (int_body, all_digit_frac) = body.split_once('.').map_or((body, ""), |(i, f)| (i, f));
    let (hours, minutes, sec_field) = if body.contains(':') {
        let parts: Vec<&str> = body.split(':').collect();
        match parts.as_slice() {
            [h, m] => (parse_u64_field(h)?, parse_u64_field(m)?, "0".to_string()),
            [h, m, s] => (parse_u64_field(h)?, parse_u64_field(m)?, (*s).to_string()),
            _ => return Err(format_err()),
        }
    } else {
        if !int_body.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format_err());
        }
        // Re-attach the fraction (if any) to the seconds field for the shared
        // seconds+fraction split below.
        let with_frac = |sec: &str| {
            if all_digit_frac.is_empty() {
                sec.to_string()
            } else {
                format!("{sec}.{all_digit_frac}")
            }
        };
        match int_body.len() {
            6 => (
                parse_u64_field(&int_body[0..2])?,
                parse_u64_field(&int_body[2..4])?,
                with_frac(&int_body[4..6]),
            ),
            4 => (
                parse_u64_field(&int_body[0..2])?,
                parse_u64_field(&int_body[2..4])?,
                with_frac("0"),
            ),
            _ => return Err(format_err()),
        }
    };

    // Seconds + fractional microseconds.
    let (sec_str, frac_str) = sec_field
        .split_once('.')
        .unwrap_or((sec_field.as_str(), ""));
    let seconds = parse_u64_field(sec_str)?;
    let micros = parse_frac_micros(frac_str).ok_or_else(format_err)?;

    // Meridian adjustment is applied after parsing the fields, matching Postgres.
    let mut hours = hours;
    if let Some(pm) = is_pm {
        if hours > 12 {
            return Err(range_err());
        }
        if pm {
            if hours != 12 {
                hours += 12; // 1–11 PM → 13–23; 12 PM stays noon.
            }
        } else if hours == 12 {
            hours = 0; // 12 AM → midnight.
        }
    }

    // Minutes/seconds must be within a clock field; hours are bounded by the final
    // range check (so `24:00:00` passes but `25:00:00` / `24:30:00` do not).
    if minutes >= 60 || seconds >= 60 {
        return Err(range_err());
    }
    // Compute in i128 so an unbounded colon-form hour field (e.g. `2600000000:00`)
    // can't overflow the multiply — it fails loud via the range check below rather
    // than panicking (debug) or wrapping (release).
    let total = i128::from(hours) * 3_600_000_000
        + i128::from(minutes) * 60_000_000
        + i128::from(seconds) * 1_000_000
        + i128::from(micros);
    if !(0..=i128::from(USECS_PER_DAY)).contains(&total) {
        return Err(range_err());
    }
    // In range → fits i64.
    Ok(total as i64)
}

/// Parse a decimal field (hours/minutes/seconds) as a `u64`. A non-digit field is an
/// unparseable shape → `22007` (`InvalidDatetimeFormat`); a numeric-but-out-of-range
/// field value (a huge hour like `9999999999`, past `u32` but a valid number) is
/// caught downstream by the range check → `22008`, not mislabeled a format error.
fn parse_u64_field(s: &str) -> Result<u64> {
    s.parse::<u64>().map_err(|_| Error::InvalidDatetimeFormat {
        type_name: "time",
        value: s.to_string(),
    })
}

/// Convert a fractional-seconds digit string to microseconds as 18.6's
/// `ParseFractionalSecond` does: `rint(strtod("." digits) * 1000000)`, the product a C
/// `double` and the tie going to the even microsecond. `None` if the string holds a
/// non-digit; an empty string is zero microseconds.
///
/// The rounding is the `double`'s, not the decimal's: `.0001255` is `.000125` and
/// `.0001265` is `.000127` on 18.6, because neither decimal is a tie once it is a
/// `double`, while `.0000015` is `.000002` and `.0000025` is `.000002`. It may answer
/// `1_000_000` (`.9999995`), and the caller carries it into the seconds: the `time`
/// reader here, and `decode_time` for the timestamp readers.
fn parse_frac_micros(frac: &str) -> Option<i64> {
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let fraction: f64 = format!("0.{frac}").parse().ok()?;
    // In `0.0 ..= 1_000_000.0`, so the cast is exact.
    #[allow(clippy::cast_possible_truncation)]
    Some((fraction * 1_000_000.0).round_ties_even() as i64)
}

/// Render a timestamp through a Postgres `to_char` format string.
///
/// Deliberately minimal: this backs exactly the spine's ISO-8601 projection,
/// `to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')`, so only the
/// field tokens that string uses are recognized — `YYYY`, `MM`, `DD`, `HH24`,
/// `MI`, `SS`. Double-quoted runs (`"T"`, `"Z"`) are passed through verbatim
/// with the quotes stripped, and non-letter separators (`-`, `:`, space) are
/// literal passthrough — matching Postgres, which treats only letter runs as
/// format patterns. An unrecognized *letter* run (e.g. `US`, `FF`) fails loud
/// with `Error::NotImplemented` rather than silently emitting the literal text
/// or a guessed field; silent-wrong output is the cardinal sin for a
/// differential engine. No fractional seconds or offset text are emitted (the
/// spine's format asks for neither).
///
/// `ts` is the underlying UTC value (pglet has no distinct timestamptz; the
/// caller has already applied `AT TIME ZONE 'UTC'`, which is identity here).
///
/// # Errors
///
/// Returns `Error::NotImplemented` for any unrecognized letter-run format token
/// or an unterminated double-quoted literal.
pub fn format_timestamp_to_char(ts: &OffsetDateTime, pg_format: &str) -> Result<String> {
    let mut out = String::with_capacity(pg_format.len());
    let chars: Vec<char> = pg_format.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            // Double-quoted literal run: copy verbatim, strip the quotes.
            let mut j = i + 1;
            let mut closed = false;
            while j < chars.len() {
                if chars[j] == '"' {
                    closed = true;
                    break;
                }
                out.push(chars[j]);
                j += 1;
            }
            if !closed {
                return Err(Error::NotImplemented(format!(
                    "to_char: unterminated quoted literal in format {pg_format:?}"
                )));
            }
            i = j + 1;
            continue;
        }
        // Non-letter characters (`-`, `:`, space, …) are literal separators.
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        // A letter run is a format token. Match longest-first (HH24 before the
        // MM/DD/MI two-char tokens); an unrecognized run fails loud.
        let rest: String = chars[i..].iter().collect();
        let (token, value) = if rest.starts_with("YYYY") {
            ("YYYY", format!("{:04}", ts.year()))
        } else if rest.starts_with("HH24") {
            ("HH24", format!("{:02}", ts.hour()))
        } else if rest.starts_with("MM") {
            ("MM", format!("{:02}", u8::from(ts.month())))
        } else if rest.starts_with("DD") {
            ("DD", format!("{:02}", ts.day()))
        } else if rest.starts_with("MI") {
            ("MI", format!("{:02}", ts.minute()))
        } else if rest.starts_with("SS") {
            ("SS", format!("{:02}", ts.second()))
        } else {
            return Err(Error::NotImplemented(format!(
                "to_char: unsupported format token at {rest:?} in {pg_format:?}"
            )));
        };
        out.push_str(&value);
        i += token.chars().count();
    }
    Ok(out)
}

/// The Postgres text representation of a scalar `Value`, or `None` for NULL.
///
/// The single source for a value's text form (notably `t`/`f` for booleans):
/// the server's text-format wire encoder, the `::text` casts, and array elements
/// all render through this. Array elements additionally pass the result through
/// `quote_array_element`; the unquoted text is identical to a top-level value's,
/// matching Postgres (the element output is the type's output function in both).
// A float's text is 18.6's `float4out` / `float8out` (`super::format_float4_text`). jsonb
// numbers are exact `PgNumeric`, not `f64`, so a number renders with its own spelling,
// and object keys are stored in Postgres's canonical order (`JsonbValue::canonical`),
// so a document's rendered key order does not depend on the order it was written in. A
// jsonb string's control chars escape to valid JSON via `escape_json_string`, and an
// object key goes through that same one predicate.
#[must_use]
pub fn value_to_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "t" } else { "f" }.to_string()),
        Value::Int2(n) => Some(n.to_string()),
        Value::Int4(n) => Some(n.to_string()),
        Value::Int8(n) => Some(n.to_string()),
        Value::Float4(n) => Some(super::format_float4_text(*n)),
        Value::Float8(n) => Some(super::format_float8_text(*n)),
        Value::Text(s) => Some(s.clone()),
        Value::Bytea(b) => Some(format_bytea_hex(b)),
        Value::Timestamp(ts) => Some(format_timestamp_text(ts)),
        Value::Date(d) => Some(format_date_text(d)),
        Value::Time(t) => Some(format_time_text(*t)),
        Value::Numeric(nn) => Some(nn.to_display_string()),
        Value::Jsonb(j) => Some(j.to_json_string()),
        Value::Uuid(u) => Some(format_uuid(u)),
        // Nested arrays are not constructed by pglet, but render defensively.
        Value::Array(inner) => Some(inner.to_text_format()),
        // Postgres `postgres`-style interval output (`1 year 2 mons 3 days …`).
        Value::Interval(iv) => Some(super::format_interval_text(iv)),
    }
}

/// Render a value as Postgres text, honoring the result column's declared type.
///
/// The runtime `Value` cannot distinguish `timestamptz` from `timestamp` — both
/// are `Value::Timestamp` (pglet has no distinct timestamptz value; it stores UTC).
/// The *column* type carries that distinction, so the wire encoders pass the
/// declared `TypeOid` here: a `Timestamp` value in a `Timestamptz` column renders
/// with the `+00` offset, and so does each element of an array in a `timestamptz[]`
/// column, whatever element type the array value itself records; everything else
/// defers to `value_to_text`. This is the column-aware entry point for wire text output
/// (the native server's simple/extended text path, `COPY TO` and the dump through it);
/// plain `value_to_text` stays the value-only renderer (JSONB coercion among its callers).
#[must_use]
pub fn value_to_text_with_oid(v: &Value, oid: TypeOid) -> Option<String> {
    match (v, oid) {
        (Value::Timestamp(ts), TypeOid::Timestamptz) => Some(format_timestamptz_text(ts)),
        (Value::Array(arr), _) if oid.is_array() => {
            Some(arr.to_text_format_as(oid.array_element().unwrap_or(arr.element_type)))
        }
        _ => value_to_text(v),
    }
}

/// Quote an array element for the `{...}` text format if it would be ambiguous.
pub(super) fn quote_array_element(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.eq_ignore_ascii_case("null")
        || s.chars()
            .any(|c| matches!(c, '{' | '}' | ',' | '"' | '\\') || c.is_whitespace());
    if needs_quote {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DATE_TOKENS, format_date_text, format_time_text, format_timestamp_text,
        format_timestamp_to_char, format_timestamptz_text, format_uuid, parse_date, parse_time,
        parse_timestamp, parse_timestamp_naive, parse_uuid_bytes, value_to_text,
        value_to_text_with_oid,
    };
    use crate::types::{TypeOid, Value};

    const SPINE_FORMAT: &str = "YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"";

    #[test]
    fn date_text_is_zero_padded_iso_and_round_trips() {
        use time::macros::date;
        // `YYYY-MM-DD`, zero-padded month/day (matches Postgres `date` output).
        assert_eq!(
            format_date_text(&date!(2024 - 06 - 15).into()),
            "2024-06-15"
        );
        assert_eq!(
            format_date_text(&date!(2024 - 01 - 02).into()),
            "2024-01-02"
        );
        // `value_to_text` routes a `Value::Date` through the same renderer.
        assert_eq!(
            value_to_text(&Value::date(date!(2024 - 12 - 31))).as_deref(),
            Some("2024-12-31")
        );
        // parse ∘ format is identity across a spread of dates.
        for d in [
            date!(1999 - 12 - 31),
            date!(2000 - 01 - 01),
            date!(2024 - 02 - 29),
        ] {
            assert_eq!(parse_date(&format_date_text(&d.into())).unwrap(), d.into());
        }
    }

    /// The date and timestamp renderers print a year as `time`'s `[year]` component does,
    /// across every year `time` holds (-9999 through 9999, `large-dates` off): four digits
    /// zero-padded, a `-` before a negative year. The domain keeps a year below 1 out of
    /// every value the engine builds; this holds the rendering for the ones it does not.
    #[test]
    fn calendar_text_pads_every_year_time_holds() {
        use time::macros::{date, datetime};
        for (d, want) in [
            (date!(-9999 - 01 - 01), "-9999-01-01"),
            (date!(-0001 - 06 - 15), "-0001-06-15"),
            (date!(0000 - 12 - 31), "0000-12-31"),
            (date!(0001 - 01 - 01), "0001-01-01"),
            (date!(0009 - 02 - 03), "0009-02-03"),
            (date!(0099 - 10 - 11), "0099-10-11"),
            (date!(0999 - 09 - 09), "0999-09-09"),
            (date!(9999 - 12 - 31), "9999-12-31"),
        ] {
            assert_eq!(format_date_text(&d.into()), want);
        }
        for (ts, want) in [
            (datetime!(-9999-01-01 00:00:00 UTC), "-9999-01-01 00:00:00"),
            (
                datetime!(-0001-06-15 01:02:03.5 UTC),
                "-0001-06-15 01:02:03.5",
            ),
            (
                datetime!(0000-12-31 23:59:59.999999 UTC),
                "0000-12-31 23:59:59.999999",
            ),
            (datetime!(0042-07-08 09:10:11 UTC), "0042-07-08 09:10:11"),
            (
                datetime!(9999-12-31 23:59:59.000001 UTC),
                "9999-12-31 23:59:59.000001",
            ),
        ] {
            assert_eq!(format_timestamp_text(&ts.into()), want);
            assert_eq!(format_timestamptz_text(&ts.into()), format!("{want}+00"));
        }
        // The fields are printed at the offset the instant carries.
        assert_eq!(
            format_timestamp_text(&datetime!(0001-01-01 03:00:00 +05).into()),
            "0001-01-01 03:00:00"
        );
    }

    #[test]
    fn time_text_is_zero_padded_and_trims_fraction() {
        // Zero-padded HH:MM:SS, no fractional part when µs == 0.
        assert_eq!(format_time_text(0), "00:00:00");
        assert_eq!(
            format_time_text(13 * 3_600_000_000 + 45 * 60_000_000),
            "13:45:00"
        );
        // The 24:00:00 endpoint renders as itself.
        assert_eq!(format_time_text(86_400_000_000), "24:00:00");
        // Trailing zeros trimmed: .100000 → .1, .120000 → .12; full µs kept.
        assert_eq!(format_time_text(100_000), "00:00:00.1");
        assert_eq!(format_time_text(120_000), "00:00:00.12");
        assert_eq!(format_time_text(123_456), "00:00:00.123456");
        // `value_to_text` routes a `Value::Time` through the same renderer.
        assert_eq!(
            value_to_text(&Value::Time(45_015_000_000)).as_deref(),
            Some("12:30:15")
        );
    }

    #[test]
    fn parse_time_accepts_all_forms_and_round_trips() {
        // Colon forms.
        assert_eq!(
            parse_time("13:45:06").unwrap(),
            13 * 3_600_000_000 + 45 * 60_000_000 + 6_000_000
        );
        assert_eq!(
            parse_time("13:45").unwrap(),
            13 * 3_600_000_000 + 45 * 60_000_000
        );
        assert_eq!(parse_time("00:00:00.123456").unwrap(), 123_456);
        // All-digit ISO basic forms, with and without a fractional suffix.
        assert_eq!(
            parse_time("134506").unwrap(),
            parse_time("13:45:06").unwrap()
        );
        assert_eq!(parse_time("1345").unwrap(), parse_time("13:45").unwrap());
        assert_eq!(
            parse_time("134506.5").unwrap(),
            parse_time("13:45:06.5").unwrap()
        );
        assert_eq!(
            parse_time("1345.5").unwrap(),
            parse_time("13:45:00.5").unwrap()
        );
        // 12-hour AM/PM (applied after field parse): 12 AM = midnight, 12 PM = noon.
        assert_eq!(parse_time("12:00 AM").unwrap(), 0);
        assert_eq!(parse_time("12:00 PM").unwrap(), 12 * 3_600_000_000);
        assert_eq!(parse_time("4:00 PM").unwrap(), 16 * 3_600_000_000);
        assert_eq!(parse_time("4:00PM").unwrap(), 16 * 3_600_000_000);
        // The 24:00:00 endpoint parses.
        assert_eq!(parse_time("24:00:00").unwrap(), 86_400_000_000);
        // Fractional beyond 6 digits rounds to nearest µs (ties to even).
        assert_eq!(parse_time("00:00:00.1234565").unwrap(), 123_456);
        assert_eq!(parse_time("00:00:00.1234575").unwrap(), 123_458);
        assert_eq!(parse_time("00:00:00.1234567").unwrap(), 123_457);
        // format ∘ parse is identity across a spread of times.
        for t in [
            "00:00:00",
            "01:02:03",
            "23:59:59.999999",
            "12:30:15.5",
            "24:00:00",
        ] {
            assert_eq!(
                format_time_text(parse_time(t).unwrap()),
                reference_render(t)
            );
        }
    }

    /// The canonical rendering a spelling should round-trip to (fraction trimmed).
    fn reference_render(t: &str) -> String {
        format_time_text(parse_time(t).unwrap())
    }

    #[test]
    fn parse_time_rejects_malformed_and_out_of_range() {
        // A well-formed value with an out-of-range field → 22008 (datetime_field_overflow).
        for out_of_range in [
            "12:60:00",        // minutes ≥ 60
            "12:00:60",        // seconds ≥ 60
            "24:00:00.000001", // just past the endpoint
            "24:30:00",        // past the endpoint
            "25:00:00",        // past the endpoint
            "13:00 PM",        // hour > 12 with a meridian
        ] {
            let e = parse_time(out_of_range).unwrap_err();
            assert_eq!(e.sqlstate(), "22008", "{out_of_range:?} is a range error");
        }
        // An unparseable shape → 22007 (invalid_datetime_format).
        for bad_format in [
            "not-a-time",
            "12345",    // 5 digits — not a valid all-digit form
            "1234567",  // 7 digits — not a valid all-digit form
            "13:00 XM", // bogus meridian degrades to a bad field
            "",
        ] {
            let e = parse_time(bad_format).unwrap_err();
            assert_eq!(e.sqlstate(), "22007", "{bad_format:?} is a format error");
        }
        // A tz suffix is rejected as a format error (pglet is UTC-only, deliberately
        // stricter than PG, which accepts-and-drops it).
        for tz in ["04:05:06-08:00", "04:05:06+05", "04:05:06Z"] {
            assert_eq!(parse_time(tz).unwrap_err().sqlstate(), "22007");
        }
        // A huge but numeric hour field is a range error (`22008`), not a format one:
        // it must NOT overflow-panic the i64 multiply, and a value past `u32::MAX`
        // (`9999999999`) must reach the range check rather than failing to parse as a
        // narrower integer (regression guard for both bounds).
        for huge_hour in ["2600000000:00:00", "9999999999:00:00"] {
            assert_eq!(
                parse_time(huge_hour).unwrap_err().sqlstate(),
                "22008",
                "{huge_hour:?} is a numeric field out of range"
            );
        }
        // Unpadded fields parse like Postgres (`12:5:3` = 12:05:03).
        assert_eq!(format_time_text(parse_time("12:5:3").unwrap()), "12:05:03");
    }

    #[test]
    fn parse_date_rejects_malformed() {
        // A value with the ISO shape but an out-of-range field → 22008.
        for out_of_range in ["2024-13-01", "2024-02-30", "2024-00-01"] {
            let e = parse_date(out_of_range).unwrap_err();
            assert_eq!(e.sqlstate(), "22008", "{out_of_range:?} is a range error");
        }
        // An unparseable / non-ISO spelling → 22007 (never a silent misparse). A
        // leading `-` or `+` (a signed year pglet doesn't model) is a format error too.
        for bad_format in [
            "not-a-date",
            "2024/06/15",
            "2024-06-15 12:00",
            "-2026-06-15",
            "+2026-06-15",
            "",
        ] {
            let e = parse_date(bad_format).unwrap_err();
            assert_eq!(e.sqlstate(), "22007", "{bad_format:?} is a format error");
        }
        // Unpadded month/day parse like Postgres (`2026-1-1` = 2026-01-01), and both
        // forms round-trip to the padded output.
        for (input, expected) in [("2026-1-1", "2026-01-01"), ("2026-01-01", "2026-01-01")] {
            assert_eq!(format_date_text(&parse_date(input).unwrap()), expected);
        }
        // Surrounding whitespace is tolerated (trimmed), like the other parsers.
        assert!(parse_date("  2024-06-15  ").is_ok());
    }

    /// The date reader's domain edges: `0001-01-01` and `9999-12-31` accepted, a year-0
    /// field refused with 18.6's field sentence quoting the input as written (ahead of a
    /// bad month or day, as `ValidateDate` orders it), and a year past 9999 refused with
    /// the bare range sentence 18.6 does not share.
    #[test]
    fn parse_date_holds_the_domain_edges() {
        use time::macros::date;
        assert_eq!(
            parse_date("0001-01-01").unwrap(),
            date!(0001 - 01 - 01).into()
        );
        assert_eq!(
            parse_date("9999-12-31").unwrap(),
            date!(9999 - 12 - 31).into()
        );
        for (input, quoted) in [
            ("0000-12-31", "0000-12-31"),
            ("0000-01-01", "0000-01-01"),
            ("  0000-12-31", "  0000-12-31"),
            ("0000-13-01", "0000-13-01"),
            ("0000-02-30", "0000-02-30"),
        ] {
            let e = parse_date(input).unwrap_err();
            assert_eq!(e.sqlstate(), "22008", "{input:?}");
            assert_eq!(
                e.to_string(),
                format!("date/time field value out of range: \"{quoted}\""),
                "{input:?}"
            );
        }
        let e = parse_date("10000-01-01").unwrap_err();
        assert_eq!(
            (e.sqlstate(), e.to_string().as_str()),
            ("22008", "date out of range")
        );
    }

    /// The timestamp readers' domain edges, both readers, both polarities — including
    /// the explicit offset that moves a `timestamptz` instant across either edge, whose
    /// upper crossing once panicked in `to_offset` (a process abort under
    /// `panic = "abort"`).
    #[test]
    fn parse_timestamp_holds_the_domain_edges_and_never_panics() {
        use time::macros::datetime;
        for parse in [parse_timestamp, parse_timestamp_naive] {
            assert_eq!(
                parse("0001-01-01 00:00:00").unwrap(),
                datetime!(0001-01-01 00:00:00 UTC).into()
            );
            assert_eq!(
                parse("9999-12-31 23:59:59.999999").unwrap(),
                datetime!(9999-12-31 23:59:59.999999 UTC).into()
            );
            assert_eq!(
                parse("0001-01-01").unwrap(),
                datetime!(0001-01-01 00:00:00 UTC).into()
            );
            // A year-0 field is 18.6's field refusal, quoting the input, at every spelling.
            for input in [
                "0000-06-15 12:00:00",
                "0000-06-15 12:00:00+00",
                "0000-06-15T12:00:00Z",
                "0000-06-15",
                "0000-12-31 23:00:00-05",
                "0000-12-31 12:00:00.9999999",
            ] {
                let e = parse(input).unwrap_err();
                assert_eq!(e.sqlstate(), "22008", "{input:?}");
                assert_eq!(
                    e.to_string(),
                    format!("date/time field value out of range: \"{input}\""),
                    "{input:?}"
                );
            }
            // A signed year is malformed, as it is for `date`.
            for input in [
                "-0001-01-01 00:00:00",
                "-0001-01-01 00:00:00+00",
                "+0001-01-01 12:00:00",
                "+0001-01-01 12:00:00+00",
                " +0001-01-01",
                "+0000-12-31 12:00:00",
            ] {
                assert_eq!(parse(input).unwrap_err().sqlstate(), "22P02", "{input:?}");
            }
            // A rounding carry past year 9999 is refused, not kept unrounded.
            let e = parse("9999-12-31 23:59:59.9999995").unwrap_err();
            assert_eq!(
                (e.sqlstate(), e.to_string().as_str()),
                ("22008", "timestamp out of range")
            );
        }

        // `timestamptz`: the domain is the UTC reading, one microsecond each side.
        for (input, utc) in [
            ("0001-01-01 05:00:00+05", datetime!(0001-01-01 00:00:00 UTC)),
            // The rounding runs before the domain is judged, so a UTC reading below the
            // edge that rounds onto it is in.
            (
                "0001-01-01 04:59:59.9999995+05",
                datetime!(0001-01-01 00:00:00 UTC),
            ),
            (
                "9999-12-31 18:59:59.999999-05",
                datetime!(9999-12-31 23:59:59.999999 UTC),
            ),
            (
                "9999-12-31T18:59:59.999999-05:00",
                datetime!(9999-12-31 23:59:59.999999 UTC),
            ),
        ] {
            assert_eq!(parse_timestamp(input).unwrap(), utc.into(), "{input:?}");
        }
        for input in [
            "0001-01-01 04:59:59.999999+05",
            "0001-01-01 03:00:00+05",
            "0001-01-01 04:59:59.9999994+05",
            "9999-12-31 18:59:59.9999995-05",
            "9999-12-31 19:00:00-05",
            "9999-12-31 23:00:00-05",
            "9999-12-31T23:00:00-05:00",
        ] {
            let e = parse_timestamp(input).unwrap_err();
            assert_eq!(
                (e.sqlstate(), e.to_string().as_str()),
                ("22008", "timestamp out of range"),
                "{input:?}"
            );
        }
        // `timestamp` drops the offset, so the same spellings are the written fields.
        assert_eq!(
            parse_timestamp_naive("9999-12-31 23:00:00-05").unwrap(),
            datetime!(9999-12-31 23:00:00 UTC).into()
        );
        assert_eq!(
            parse_timestamp_naive("0001-01-01 03:00:00+05").unwrap(),
            datetime!(0001-01-01 03:00:00 UTC).into()
        );
    }

    #[test]
    fn timestamptz_text_carries_utc_offset() {
        use time::macros::datetime;
        // Fractional seconds preserved, `+00` appended.
        assert_eq!(
            format_timestamptz_text(&datetime!(2024-06-15 12:30:45.123 UTC).into()),
            "2024-06-15 12:30:45.123+00"
        );
        // Whole second: no fractional part, still `+00`.
        assert_eq!(
            format_timestamptz_text(&datetime!(2024-01-02 03:04:05 UTC).into()),
            "2024-01-02 03:04:05+00"
        );
    }

    #[test]
    fn value_to_text_with_oid_distinguishes_tz_from_plain() {
        use time::macros::datetime;
        let ts = Value::timestamp(datetime!(2024-06-15 12:30:45.123 UTC));
        // A `timestamptz` column gets the offset...
        assert_eq!(
            value_to_text_with_oid(&ts, TypeOid::Timestamptz).as_deref(),
            Some("2024-06-15 12:30:45.123+00")
        );
        // ...while a `timestamp without time zone` column does not.
        assert_eq!(
            value_to_text_with_oid(&ts, TypeOid::Timestamp).as_deref(),
            Some("2024-06-15 12:30:45.123")
        );
        // Non-timestamp values are unaffected by the OID and match `value_to_text`.
        let n = Value::Int4(42);
        assert_eq!(
            value_to_text_with_oid(&n, TypeOid::Timestamptz),
            value_to_text(&n)
        );
    }

    #[test]
    fn to_char_renders_the_spine_iso8601_format() {
        use time::macros::datetime;
        // Whole second.
        assert_eq!(
            format_timestamp_to_char(&datetime!(2026-06-19 12:30:45 UTC), SPINE_FORMAT).unwrap(),
            "2026-06-19T12:30:45Z"
        );
        // Sub-second is truncated (the format asks for no fractional digits).
        assert_eq!(
            format_timestamp_to_char(&datetime!(2026-06-19 12:30:45.123456 UTC), SPINE_FORMAT)
                .unwrap(),
            "2026-06-19T12:30:45Z"
        );
        // Midnight, single-digit month/day zero-padded.
        assert_eq!(
            format_timestamp_to_char(&datetime!(2026-01-05 00:00:00 UTC), SPINE_FORMAT).unwrap(),
            "2026-01-05T00:00:00Z"
        );
    }

    #[test]
    fn to_char_fails_loud_on_unknown_token() {
        use time::macros::datetime;
        // `US` (microseconds) is a real PG token but unsupported here — must error,
        // not silently emit the literal text.
        let err = format_timestamp_to_char(&datetime!(2026-06-19 12:30:45 UTC), "YYYY-US");
        assert!(err.is_err());
        // Unterminated quoted literal also fails loud.
        let err = format_timestamp_to_char(&datetime!(2026-06-19 12:30:45 UTC), "YYYY\"T");
        assert!(err.is_err());
    }

    #[test]
    fn timestamp_text_is_space_separated_pg_form() {
        use time::macros::datetime;
        // Space separator, no `T`/`Z`; fractional omitted when zero.
        assert_eq!(
            format_timestamp_text(&datetime!(2024-06-15 12:30:45 UTC).into()),
            "2024-06-15 12:30:45"
        );
        // Fractional seconds present: trailing zeros trimmed.
        assert_eq!(
            format_timestamp_text(&datetime!(2024-06-15 12:30:45.123 UTC).into()),
            "2024-06-15 12:30:45.123"
        );
        assert_eq!(
            format_timestamp_text(&datetime!(2024-06-15 12:30:45.5 UTC).into()),
            "2024-06-15 12:30:45.5"
        );
    }

    #[test]
    fn float_text_spells_infinity_like_pg() {
        assert_eq!(value_to_text(&Value::Float8(1.5)).as_deref(), Some("1.5"));
        assert_eq!(value_to_text(&Value::Float4(-2.5)).as_deref(), Some("-2.5"));
        assert_eq!(
            value_to_text(&Value::Float8(f64::INFINITY)).as_deref(),
            Some("Infinity")
        );
        assert_eq!(
            value_to_text(&Value::Float8(f64::NEG_INFINITY)).as_deref(),
            Some("-Infinity")
        );
        assert_eq!(
            value_to_text(&Value::Float4(f32::INFINITY)).as_deref(),
            Some("Infinity")
        );
        assert_eq!(
            value_to_text(&Value::Float8(f64::NAN)).as_deref(),
            Some("NaN")
        );
    }

    #[test]
    fn uuid_round_trips_dashed_and_bare() {
        let bytes = [
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ];
        let dashed = format_uuid(&bytes);
        assert_eq!(dashed, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(parse_uuid_bytes(&dashed), Some(bytes));
        // Bare (undashed) and surrounding whitespace also parse.
        assert_eq!(
            parse_uuid_bytes("  550e8400e29b41d4a716446655440000  "),
            Some(bytes)
        );
    }

    #[test]
    fn parse_uuid_bytes_rejects_malformed_without_panicking() {
        assert_eq!(parse_uuid_bytes("not-a-uuid"), None);
        assert_eq!(parse_uuid_bytes(""), None);
        // 32 hex digits minus one is too short.
        assert_eq!(parse_uuid_bytes("550e8400e29b41d4a71644665544000"), None);
        // 32 *bytes* of non-ASCII (8 four-byte chars) must reject, not slice
        // mid-codepoint and panic — the bug the shared parser closes.
        assert_eq!(parse_uuid_bytes("🎉🎉🎉🎉🎉🎉🎉🎉"), None);
    }

    /// 18.6's `DecodeTimezone` over the field `ParseDateTime` builds, measured on 18.6
    /// member by member: the accepted spellings with their seconds east of UTC, and the
    /// two refusals — out of range (`22009`) checked before malformed (`22007` there).
    #[test]
    fn decode_timezone_reads_the_offset_as_18_6_does() {
        use super::{DtError, decode_timezone};
        for (tail, seconds) in [
            ("+15:59:59", 15 * 3600 + 59 * 60 + 59),
            ("-15:59:59", -(15 * 3600 + 59 * 60 + 59)),
            ("-05:30:15", -(5 * 3600 + 30 * 60 + 15)),
            ("+1559", 15 * 3600 + 59 * 60),
            ("+155", 3600 + 55 * 60),
            ("+005", 5 * 60),
            ("+00000005", 5 * 60),
            ("+5", 5 * 3600),
            ("+05:3", 5 * 3600 + 3 * 60),
            ("+15:", 15 * 3600),
            ("+15::", 15 * 3600),
            ("+15:-0", 15 * 3600),
            ("-0", 0),
            ("+1:2:3", 3600 + 2 * 60 + 3),
        ] {
            assert_eq!(decode_timezone(tail.as_bytes()), Ok(seconds), "{tail:?}");
        }
        for tail in [
            "+16",
            "-16:00",
            "+23:59",
            "+15:60",
            "+15:59:60",
            "+1600",
            "+165",
            "+155959",
            "+99",
            "+00:60",
            "+15:-5",
            "+16.5",
            "+16:",
            "+16:00:00:00",
            "+2147483648",
            "+99999999999999999999999",
        ] {
            assert_eq!(
                decode_timezone(tail.as_bytes()),
                Err(DtError::ZoneOverflow),
                "{tail:?}"
            );
        }
        for tail in ["+05.5", "+15:59:59.5", "+5-3", "+05:00:00:00"] {
            assert_eq!(
                decode_timezone(tail.as_bytes()),
                Err(DtError::BadFormat),
                "{tail:?}"
            );
        }
    }

    /// Where an offset sits in a timestamp reader's error order: after a spelling it can
    /// split, before the year-0 field, a month out of range and the domain — measured on
    /// 18.6, whose `DecodeDateTime` decodes the offset in its field loop and validates the
    /// date after it — and a `timestamp` validates the offset it then drops.
    #[test]
    fn a_timestamp_offset_is_judged_before_the_date_it_moves() {
        use time::macros::datetime;
        for parse in [parse_timestamp, parse_timestamp_naive] {
            for input in [
                "2000-01-01 00:00:00+16:00",
                "  2000-01-01 00:00:00+16  ",
                "0000-12-31 23:00:00+16",
                "2000-13-01 00:00:00+16",
                "2000-02-30 00:00:00+16",
                "0001-01-01T00:00:00+23:59",
                "9999-12-31 23:00:00-16",
                "2000-01-01t00:00:00+16:00",
            ] {
                let e = parse(input).unwrap_err();
                assert_eq!(
                    (e.sqlstate(), e.to_string()),
                    (
                        "22009",
                        format!("time zone displacement out of range: \"{input}\"")
                    ),
                    "{input:?}"
                );
            }
            // A time of day 18.6 refuses in its field loop answers ahead of the offset
            // after it, and an offset after an empty fraction or a first offset is still
            // read and judged.
            for (input, code) in [
                ("2000-01-01 25:00:00+16", "22008"),
                ("2000-01-01 00:60:00+16", "22008"),
                ("2000-01-01 00:00:00.+16", "22009"),
                ("2000-01-01 00:00:00+05+16", "22009"),
                ("zzz 00:00:00+16", "22P02"),
            ] {
                assert_eq!(parse(input).unwrap_err().sqlstate(), code, "{input:?}");
            }
            // A malformed offset beside a year 0 is malformed.
            assert_eq!(
                parse("0000-12-31 23:00:00+05.5").unwrap_err().sqlstate(),
                "22P02"
            );
        }
        assert_eq!(
            parse_timestamp("2000-01-01 00:00:00-15:59:59").unwrap(),
            datetime!(2000-01-01 15:59:59 UTC).into()
        );
        assert_eq!(
            parse_timestamp("2000-01-01 12:00:00.5 +05:30:15").unwrap(),
            datetime!(2000-01-01 06:29:45.5 UTC).into()
        );
        assert_eq!(
            parse_timestamp_naive("2000-01-01 00:00:00-15:59:59").unwrap(),
            datetime!(2000-01-01 00:00:00 UTC).into()
        );
        // A sign inside a zone name makes a POSIX zone, whose offset counts west.
        assert_eq!(
            parse_timestamp("2000-01-01 00:00:00 UTC+16").unwrap(),
            datetime!(2000-01-01 16:00:00 UTC).into()
        );
        assert_eq!(
            parse_timestamp_naive("2000-01-01 00:00:00 UTC+16").unwrap(),
            datetime!(2000-01-01 00:00:00 UTC).into()
        );
    }

    /// The fields a timestamp spelling splits into and the order they may come in, each
    /// member measured on 18.6: ASCII punctuation the tokenizer gives no role, or whitespace,
    /// between the date and the time, `t` in either case, a `+` after the date starting an offset, a word that
    /// runs into a POSIX zone, and a date written after the time of day refused.
    #[test]
    fn a_timestamp_spelling_splits_into_18_6s_fields() {
        use time::macros::datetime;
        for input in [
            "2000-01-01 12:34:56",
            "2000-01-01T12:34:56",
            "2000-01-01t12:34:56",
            "2000-01-01/12:34:56",
            "2000-01-01_12:34:56",
            "2000-01-01:12:34:56",
            "2000-01-01#12:34:56",
            "2000-01-01  12:34:56",
            "\t2000-01-01 12:34:56\n",
            "2000/01/01 12:34:56",
            "2000.01.01 12:34:56",
            "2000-01-001 12:34:56",
            "2000-01-01- 12:34:56",
        ] {
            for parse in [parse_timestamp, parse_timestamp_naive] {
                assert_eq!(
                    parse(input).unwrap(),
                    datetime!(2000-01-01 12:34:56 UTC).into(),
                    "{input:?}"
                );
            }
        }
        for (input, utc) in [
            ("2000-01-01+12:34:56", datetime!(1999-12-31 11:25:04 UTC)),
            (
                "+05 2000-01-01 00:00:00",
                datetime!(1999-12-31 19:00:00 UTC),
            ),
            ("z 2000-01-01", datetime!(2000-01-01 00:00:00 UTC)),
            (
                "2000-01-01T00:00:00Z+05",
                datetime!(2000-01-01 05:00:00 UTC),
            ),
            ("2000-01-01 x5", datetime!(2000-01-01 05:00:00 UTC)),
            (
                "2000-01-01 00:00:00 abc-167",
                datetime!(1999-12-25 01:00:00 UTC),
            ),
            ("200-01-01 00:00:00", datetime!(0200-01-01 00:00:00 UTC)),
            (
                "10000-01-01 00:00:00+05",
                datetime!(9999-12-31 19:00:00 UTC),
            ),
        ] {
            assert_eq!(parse_timestamp(input).unwrap(), utc.into(), "{input:?}");
        }
        assert_eq!(
            parse_timestamp_naive("2000-01-01+12:34:56").unwrap(),
            datetime!(2000-01-01 00:00:00 UTC).into()
        );
        // Malformed here, where 18.6 answers `22007` or reads by a rule not modelled here
        // (a value, `22008` or `22023`): a `-` or `.` joining the date to the time, a
        // letter that starts a zone name with a fraction or an offset after it, a control
        // or non-ASCII byte, a date after the time of day, a daylight-saving zone, a zone
        // hour past 167, a day-of-year and a two-digit year.
        for input in [
            "2000-01-01-12:34:56",
            "2000-01-01.12:34:56",
            "2000-01-01x12:34:56.5",
            "2000-01-01x12:34:56+05",
            "2000-01-01\u{1}12:34:56",
            "2000-01-01\u{e9}12:34:56",
            "00:00:00 2000-01-01",
            "2000-01-01 00:00:00 x5z",
            "2000-01-01 00:00:00 abc168",
            "2000-060-01",
            "20-01-01 00:00:00",
            "2000-01-01 00:00:00 00:00:00",
            "2000-01-01 12:00:00 t",
        ] {
            for parse in [parse_timestamp, parse_timestamp_naive] {
                assert_eq!(parse(input).unwrap_err().sqlstate(), "22P02", "{input:?}");
            }
        }
        let e = parse_timestamp_naive("2000-01-01012:34:56").unwrap_err();
        assert_eq!(
            (e.sqlstate(), e.to_string()),
            (
                "22008",
                "date/time field value out of range: \"2000-01-01012:34:56\"".to_string()
            )
        );
    }

    /// A word of 18.6's `datetktbl` followed by a digit or `+` stays a word rather than
    /// running on into a time zone name, so a month name, a unit or a day name written
    /// straight before digits is malformed here, as 18.6 refuses it (`22007` there, `22P02`
    /// here — `EDGES.md` row 96). The loop is over the table itself, so a lookup that misses
    /// one of its entries — as a binary search over entries out of order did for `july` —
    /// reddens at that entry; each named `july` spelling answered a value under that lookup.
    #[test]
    fn a_token_word_before_digits_stays_a_word() {
        let mut inputs = vec![
            "2000-01-01july12:00:00".to_string(),
            "2000-01-01 july5 00:00:00".to_string(),
            "2000-01-01 00:00:00 july+05:30".to_string(),
            "2000-01-01 00:00:00 JULY0".to_string(),
        ];
        for word in DATE_TOKENS {
            for suffix in ["5", "0", "05:30"] {
                inputs.push(format!("2000-01-01 00:00:00 {word}{suffix}"));
                inputs.push(format!(
                    "2000-01-01 00:00:00 {}{suffix}",
                    word.to_ascii_uppercase()
                ));
            }
        }
        for input in &inputs {
            for parse in [parse_timestamp, parse_timestamp_naive] {
                assert_eq!(
                    parse(input).map_err(|e| e.sqlstate()),
                    Err("22P02"),
                    "{input:?}"
                );
            }
        }
    }

    /// A seconds field of 60 and an hour of 24 are read as 18.6's `time_overflows` reads
    /// them — each field on its own, then the time of day as a whole at most `24:00:00` —
    /// and carry into the next minute or day; past that edge is a field refusal, and a
    /// carry out of the date domain is the domain's refusal, not a silent year 10000.
    #[test]
    fn second_60_and_hour_24_are_judged_on_the_whole_time_of_day() {
        use time::macros::datetime;
        for (input, utc) in [
            ("2000-06-30 23:59:60", datetime!(2000-07-01 00:00:00 UTC)),
            ("2000-06-29 23:59:60", datetime!(2000-06-30 00:00:00 UTC)),
            (
                "2000-01-01 12:30:60.5",
                datetime!(2000-01-01 12:31:00.5 UTC),
            ),
            ("2000-01-01 24:00:00", datetime!(2000-01-02 00:00:00 UTC)),
            ("2000-01-01 12:30.5", datetime!(2000-01-01 00:12:30.5 UTC)),
            ("2000-01-01 12::30", datetime!(2000-01-01 12:00:30 UTC)),
            ("2000-01-01 12:", datetime!(2000-01-01 12:00:00 UTC)),
        ] {
            for parse in [parse_timestamp, parse_timestamp_naive] {
                assert_eq!(parse(input).unwrap(), utc.into(), "{input:?}");
            }
        }
        for input in [
            "2000-06-30 23:59:60.5",
            "2000-01-01 23:59:60.000001",
            "2000-01-01 24:00:00.5",
            "2000-01-01 24:00:01",
            "2000-01-01 00:60:00",
            "2000-01-01 00:59:61",
            "2000-01-01 99999999999999999999:00:00.5.5",
            "2000-13-01 25:00:00",
            "0000-13-01 00:00:00",
            "2000-02-30 00:00:00",
        ] {
            for parse in [parse_timestamp, parse_timestamp_naive] {
                let e = parse(input).unwrap_err();
                assert_eq!(
                    (e.sqlstate(), e.to_string()),
                    (
                        "22008",
                        format!("date/time field value out of range: \"{input}\"")
                    ),
                    "{input:?}"
                );
            }
        }
        for input in ["9999-12-31 23:59:60", "9999-12-31 24:00:00"] {
            let e = parse_timestamp_naive(input).unwrap_err();
            assert_eq!(
                (e.sqlstate(), e.to_string().as_str()),
                ("22008", "timestamp out of range"),
                "{input:?}"
            );
        }
    }

    /// A fraction past six digits is `rint(strtod(fraction) * 1000000)` over a C
    /// `double`, measured on 18.6 for the timestamp and `time` readers: `.0001255` and
    /// `.0001265` are not ties once they are `double`s, where `.0000015` and `.0000025`
    /// are and go to the even microsecond.
    #[test]
    fn a_fraction_rounds_as_a_c_double() {
        use time::macros::datetime;
        for (digits, micros) in [
            ("0001255", 125),
            ("0001265", 127),
            ("0000015", 2),
            ("0000025", 2),
            ("12345650000000000001", 123_456),
            ("", 0),
        ] {
            assert_eq!(
                parse_timestamp(&format!("2000-01-01 00:00:00.{digits}")).unwrap(),
                datetime!(2000-01-01 00:00:00 UTC)
                    .replace_microsecond(micros)
                    .unwrap()
                    .into(),
                "{digits:?}"
            );
            assert_eq!(
                parse_time(&format!("12:00:00.{digits}")).unwrap(),
                43_200_000_000 + i64::from(micros),
                "{digits:?}"
            );
        }
    }

    /// A `timestamptz[]` column renders each element with `+00` whatever element type the
    /// array value records — `ARRAY[timestamptz '…']` builds one recording `timestamp` —
    /// and a `timestamp[]` column renders none whatever the value records.
    #[test]
    fn value_to_text_with_oid_renders_an_array_at_its_declared_element_type() {
        use crate::types::ArrayValue;
        use time::macros::datetime;
        let ts = Value::Timestamp(datetime!(2000-01-01 00:00:00 UTC).into());
        for recorded in [TypeOid::Timestamp, TypeOid::Timestamptz] {
            let arr = Value::Array(ArrayValue::new(
                recorded,
                vec![
                    ts.clone(),
                    Value::Null,
                    Value::Timestamp(crate::types::Infinite::Infinity),
                ],
            ));
            assert_eq!(
                value_to_text_with_oid(&arr, TypeOid::TimestamptzArray).as_deref(),
                Some(r#"{"2000-01-01 00:00:00+00",NULL,infinity}"#),
                "{recorded:?}"
            );
            assert_eq!(
                value_to_text_with_oid(&arr, TypeOid::TimestampArray).as_deref(),
                Some(r#"{"2000-01-01 00:00:00",NULL,infinity}"#),
                "{recorded:?}"
            );
        }
    }

    #[test]
    fn parse_timestamp_applies_offset_naive_drops_it() {
        use time::macros::datetime;

        // Offset-aware input: the tz-aware parser (`timestamptz_in`) converts to UTC;
        // the tz-naive parser (`timestamp_in`) drops the offset, keeping wall-clock.
        assert_eq!(
            parse_timestamp("2024-06-15 12:00:00+05").unwrap(),
            datetime!(2024-06-15 07:00:00 UTC).into()
        );
        assert_eq!(
            parse_timestamp_naive("2024-06-15 12:00:00+05").unwrap(),
            datetime!(2024-06-15 12:00:00 UTC).into()
        );

        // A fractional hour-minute offset (`-08:30`).
        assert_eq!(
            parse_timestamp("2024-06-15 12:00:00-08:30").unwrap(),
            datetime!(2024-06-15 20:30:00 UTC).into()
        );
        assert_eq!(
            parse_timestamp_naive("2024-06-15 12:00:00-08:30").unwrap(),
            datetime!(2024-06-15 12:00:00 UTC).into()
        );

        // Fractional seconds survive the offset drop.
        assert_eq!(
            parse_timestamp_naive("2024-06-15 12:00:00.123456+05").unwrap(),
            datetime!(2024-06-15 12:00:00.123456 UTC).into()
        );

        // Naive input (no offset) → assume UTC, identical for both.
        for parse in [parse_timestamp, parse_timestamp_naive] {
            assert_eq!(
                parse("2024-06-15 12:00:00").unwrap(),
                datetime!(2024-06-15 12:00:00 UTC).into()
            );
            // Date-only → midnight UTC, identical for both.
            assert_eq!(
                parse("2024-06-15").unwrap(),
                datetime!(2024-06-15 00:00:00 UTC).into()
            );
        }

        // A malformed spelling still fails loud (unchanged) on both.
        assert!(parse_timestamp("not-a-timestamp").is_err());
        assert!(parse_timestamp_naive("not-a-timestamp").is_err());
    }
}
