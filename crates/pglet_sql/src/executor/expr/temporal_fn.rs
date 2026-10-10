//! Temporal scalar functions: `EXTRACT`, `date_part` and `date_trunc`.

use crate::planner::ExtractField;
use crate::types::{Infinite, PgInterval, PgNumeric, Value, opt_type_oid_display_name};
use crate::{Error, Result};

// ============================================================================
// Temporal scalar functions (`EXTRACT` / `date_part` / `date_trunc`)
// ============================================================================

/// The Julian Day Number of the Unix epoch (1970-01-01), Postgres `UNIX_EPOCH_JDATE`.
/// A `date`'s epoch seconds are `(julian_day − this) × 86_400`.
const UNIX_EPOCH_JDN: i64 = 2_440_588;

/// Build the exact `PgNumeric` `value / 10^scale` carrying display scale `scale`
/// (Postgres `int64_div_fast_to_numeric`) — e.g. `numeric_scaled(6_789_000, 6)` →
/// `6.789000`, `numeric_scaled(0, 6)` → `0.000000`. `scale == 0` is the plain integer.
fn numeric_scaled(value: i128, scale: u32) -> Result<PgNumeric> {
    if scale == 0 {
        return Ok(PgNumeric::from_i128(value));
    }
    let neg = value < 0;
    let abs = value.unsigned_abs();
    let divisor = 10u128.pow(scale);
    let whole = abs / divisor;
    let frac = abs % divisor;
    let sign = if neg { "-" } else { "" };
    let width = scale as usize;
    // Always a well-formed decimal, so `parse` sets `dscale = scale` and never errors.
    PgNumeric::parse(&format!("{sign}{whole}.{frac:0width$}"))
}

/// `decade`/`century`/`millennium` for `EXTRACT` (Postgres `timestamp_part`): the
/// signed count with "no year 0" handling for BC years.
const fn extract_decade(year: i64) -> i64 {
    if year >= 0 {
        year / 10
    } else {
        -((8 - (year - 1)) / 10)
    }
}
const fn extract_century(year: i64) -> i64 {
    if year > 0 {
        (year + 99) / 100
    } else {
        -((99 - (year - 1)) / 100)
    }
}
const fn extract_millennium(year: i64) -> i64 {
    if year > 0 {
        (year + 999) / 1000
    } else {
        -((999 - (year - 1)) / 1000)
    }
}

/// The ISO-week-numbering year (`date2isoyear`), with Postgres's BC adjustment
/// (`isoyear <= 0` → `isoyear − 1`).
fn extract_isoyear(date: time::Date) -> i64 {
    let y = i64::from(date.to_iso_week_date().0);
    if y <= 0 { y - 1 } else { y }
}

/// Evaluate `EXTRACT`/`date_part`: resolve the (non-NULL) field text to an
/// [`ExtractField`], pull it from the temporal source, and package as `numeric`
/// (`retnumeric`) or `float8`. A NULL field/source propagates to NULL.
pub(super) fn eval_extract(field: &Value, source: &Value, retnumeric: bool) -> Result<Value> {
    if field.is_null() || source.is_null() {
        return Ok(Value::Null);
    }
    let fn_name = if retnumeric { "extract" } else { "date_part" };
    let Value::Text(field_name) = field else {
        return Err(Error::UndefinedFunction(format!(
            "function {fn_name} does not exist for field type {}",
            opt_type_oid_display_name(field.type_oid())
        )));
    };
    let Some(f) = ExtractField::from_name(field_name) else {
        return Err(Error::UnrecognizedTimeUnit(field_name.clone()));
    };
    if let Some(answer) = extract_from_infinity(f, source, field_name, retnumeric) {
        return answer;
    }
    let num = extract_field_numeric(f, source, field_name, retnumeric)?;
    if retnumeric {
        Ok(Value::Numeric(num))
    } else {
        Ok(Value::Float8(num.to_f64()))
    }
}

/// `EXTRACT` / `date_part` over an infinite `date` or timestamp, or `None` for any other
/// source.
///
/// A field that grows with the value — `year`, `decade`, `century`, `millennium`,
/// `julian`, `isoyear`, `epoch` — is `±Infinity`, which `date_part`'s `float8` holds and
/// `extract`'s `numeric` does not here (`PgNumeric` has no infinity): that is a loud
/// `0A000` where 18.6 answers `Infinity`. Every other field is NULL
/// (`NonFiniteTimestampTzPart`), except under `EXTRACT(… FROM date)`, whose
/// `extract_date` refuses the time-of-day and zone fields for an infinity as it does for
/// a calendar day.
fn extract_from_infinity(
    f: ExtractField,
    source: &Value,
    field_name: &str,
    retnumeric: bool,
) -> Option<Result<Value>> {
    let (negative, is_date) = match source {
        Value::Date(Infinite::NegInfinity) => (true, true),
        Value::Date(Infinite::Infinity) => (false, true),
        Value::Timestamp(Infinite::NegInfinity) => (true, false),
        Value::Timestamp(Infinite::Infinity) => (false, false),
        _ => return None,
    };
    Some(match f {
        ExtractField::Year
        | ExtractField::Decade
        | ExtractField::Century
        | ExtractField::Millennium
        | ExtractField::Julian
        | ExtractField::IsoYear
        | ExtractField::Epoch => {
            if retnumeric {
                Err(Error::NotImplemented("numeric Infinity".into()))
            } else {
                Ok(Value::Float8(if negative {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }))
            }
        }
        ExtractField::Day
        | ExtractField::Month
        | ExtractField::Quarter
        | ExtractField::Week
        | ExtractField::Dow
        | ExtractField::IsoDow
        | ExtractField::Doy => Ok(Value::Null),
        _ if is_date && retnumeric => Err(Error::UnsupportedTimeUnit(field_name.to_string())),
        _ => Ok(Value::Null),
    })
}

/// Dispatch an extract to the per-source-type computation, each returning the exact
/// `PgNumeric` (with Postgres's per-field scale). A non-temporal source resolves to
/// no function (`42883`); a recognized-but-unsupported field fails loud (`0A000`).
fn extract_field_numeric(
    f: ExtractField,
    source: &Value,
    field_name: &str,
    retnumeric: bool,
) -> Result<PgNumeric> {
    match source {
        Value::Timestamp(Infinite::Finite(dt)) => extract_from_timestamp(f, *dt, field_name),
        // `date_part(text, date)` is a SQL wrapper that casts `date → timestamp`, so
        // its time-of-day fields succeed (`= 0`) rather than erroring; `EXTRACT(...
        // FROM date)` keeps the stricter `extract_date` path (`hour`/`minute`/… →
        // `0A000`). Reproduce the asymmetry: only the `date_part` (float8) form casts.
        Value::Date(Infinite::Finite(d)) if !retnumeric => extract_from_timestamp(
            f,
            d.with_time(time::Time::MIDNIGHT).assume_utc(),
            field_name,
        ),
        Value::Date(Infinite::Finite(d)) => extract_from_date(f, *d, field_name),
        Value::Time(t) => extract_from_time(f, *t, field_name),
        Value::Interval(iv) => extract_from_interval(f, iv, field_name),
        // A non-temporal source resolves to no `extract`/`date_part` signature — a
        // clean undefined-function message (42883, matching Postgres), spelling the
        // offending source type by its SQL name rather than debug-printing the OID.
        other => Err(Error::UndefinedFunction(format!(
            "function {}(text, {}) does not exist",
            if retnumeric { "extract" } else { "date_part" },
            other
                .type_oid()
                .and_then(crate::types::type_oid_to_sql_name)
                .unwrap_or("unknown"),
        ))),
    }
}

/// Extract from a `timestamp` / `timestamptz` (pglet is UTC-only, so the two coincide
/// except that timezone fields — unsupported here — would be 0 on a `timestamptz`).
fn extract_from_timestamp(
    f: ExtractField,
    dt: time::OffsetDateTime,
    field_name: &str,
) -> Result<PgNumeric> {
    let unsupported = || Error::UnsupportedTimeUnit(field_name.to_string());
    let date = dt.date();
    let year = i64::from(date.year());
    let mon = i64::from(u8::from(date.month()));
    let mday = i64::from(date.day());
    let hour = i64::from(dt.hour());
    let min = i64::from(dt.minute());
    let sec = i64::from(dt.second());
    let usec = i64::from(dt.microsecond());
    // Postgres `tm_sec * 1e6 + fsec` — the sub-minute microseconds.
    let sec_usecs = sec * 1_000_000 + usec;
    match f {
        ExtractField::Microseconds => Ok(PgNumeric::from_i128(i128::from(sec_usecs))),
        ExtractField::Milliseconds => numeric_scaled(i128::from(sec_usecs), 3),
        ExtractField::Second => numeric_scaled(i128::from(sec_usecs), 6),
        ExtractField::Minute => Ok(PgNumeric::from_i128(i128::from(min))),
        ExtractField::Hour => Ok(PgNumeric::from_i128(i128::from(hour))),
        ExtractField::Day => Ok(PgNumeric::from_i128(i128::from(mday))),
        ExtractField::Month => Ok(PgNumeric::from_i128(i128::from(mon))),
        ExtractField::Quarter => Ok(PgNumeric::from_i128(i128::from((mon - 1) / 3 + 1))),
        ExtractField::Week => Ok(PgNumeric::from_i128(i128::from(date.iso_week()))),
        ExtractField::Year => Ok(PgNumeric::from_i128(i128::from(if year > 0 {
            year
        } else {
            year - 1
        }))),
        ExtractField::Decade => Ok(PgNumeric::from_i128(i128::from(extract_decade(year)))),
        ExtractField::Century => Ok(PgNumeric::from_i128(i128::from(extract_century(year)))),
        ExtractField::Millennium => Ok(PgNumeric::from_i128(i128::from(extract_millennium(year)))),
        ExtractField::IsoYear => Ok(PgNumeric::from_i128(i128::from(extract_isoyear(date)))),
        ExtractField::Dow => Ok(PgNumeric::from_i128(i128::from(
            date.weekday().number_days_from_sunday(),
        ))),
        ExtractField::IsoDow => Ok(PgNumeric::from_i128(i128::from(
            date.weekday().number_from_monday(),
        ))),
        ExtractField::Doy => Ok(PgNumeric::from_i128(i128::from(date.ordinal()))),
        ExtractField::Julian => {
            // date2j + fractional-day, at Postgres's numeric-division default scale.
            let jdn = i128::from(date.to_julian_day());
            let micros_of_day = ((hour * 60 + min) * 60 + sec) * 1_000_000 + usec;
            let frac = PgNumeric::from_i128(i128::from(micros_of_day))
                .div(&PgNumeric::from_i128(86_400_000_000))?;
            PgNumeric::from_i128(jdn).add(&frac)
        }
        ExtractField::Epoch => {
            let micros = i128::from(dt.unix_timestamp()) * 1_000_000 + i128::from(dt.microsecond());
            numeric_scaled(micros, 6)
        }
        ExtractField::Timezone | ExtractField::TimezoneHour | ExtractField::TimezoneMinute => {
            Err(unsupported())
        }
    }
}

/// Extract from a `date` — every field is an integer (scale 0), including `epoch`
/// (whole seconds) and `julian` (`extract_date`). Time-of-day fields (`hour`/`minute`
/// /`second`/`ms`/`us`/timezone) are unsupported for a bare date (`0A000`).
fn extract_from_date(f: ExtractField, d: time::Date, field_name: &str) -> Result<PgNumeric> {
    let unsupported = || Error::UnsupportedTimeUnit(field_name.to_string());
    let year = i64::from(d.year());
    let mon = i64::from(u8::from(d.month()));
    let mday = i64::from(d.day());
    let jdn = i64::from(d.to_julian_day());
    let intresult = match f {
        ExtractField::Day => mday,
        ExtractField::Month => mon,
        ExtractField::Quarter => (mon - 1) / 3 + 1,
        ExtractField::Week => i64::from(d.iso_week()),
        ExtractField::Year => {
            if year > 0 {
                year
            } else {
                year - 1
            }
        }
        ExtractField::Decade => extract_decade(year),
        ExtractField::Century => extract_century(year),
        ExtractField::Millennium => extract_millennium(year),
        ExtractField::Julian => jdn,
        ExtractField::IsoYear => extract_isoyear(d),
        ExtractField::Dow => i64::from(d.weekday().number_days_from_sunday()),
        ExtractField::IsoDow => i64::from(d.weekday().number_from_monday()),
        ExtractField::Doy => i64::from(d.ordinal()),
        ExtractField::Epoch => (jdn - UNIX_EPOCH_JDN) * 86_400,
        _ => return Err(unsupported()),
    };
    Ok(PgNumeric::from_i128(i128::from(intresult)))
}

/// Extract from a `time` (µs since midnight): hour/minute/second/ms/µs and `epoch`
/// (seconds since midnight). Date/zone fields are unsupported (`0A000`).
fn extract_from_time(f: ExtractField, t: i64, field_name: &str) -> Result<PgNumeric> {
    let unsupported = || Error::UnsupportedTimeUnit(field_name.to_string());
    let hour = t / 3_600_000_000;
    let min = (t / 60_000_000) % 60;
    let sec_usecs = t % 60_000_000; // Postgres `tm_sec * 1e6 + fsec`
    match f {
        ExtractField::Microseconds => Ok(PgNumeric::from_i128(i128::from(sec_usecs))),
        ExtractField::Milliseconds => numeric_scaled(i128::from(sec_usecs), 3),
        ExtractField::Second => numeric_scaled(i128::from(sec_usecs), 6),
        ExtractField::Minute => Ok(PgNumeric::from_i128(i128::from(min))),
        ExtractField::Hour => Ok(PgNumeric::from_i128(i128::from(hour))),
        ExtractField::Epoch => numeric_scaled(i128::from(t), 6),
        _ => Err(unsupported()),
    }
}

/// Extract from an `interval`, over Postgres `interval2itm` (months → year/mon, day
/// raw, micros → h/m/s/µs with an **unclamped** hour magnitude). `dow`/`doy`/`isodow`
/// /`isoyear`/`julian`/timezone are undefined for an interval (`0A000`).
fn extract_from_interval(f: ExtractField, iv: &PgInterval, field_name: &str) -> Result<PgNumeric> {
    let unsupported = || Error::UnsupportedTimeUnit(field_name.to_string());
    let months = i64::from(iv.months);
    let tm_year = months / 12;
    let tm_mon = months % 12;
    let tm_mday = i64::from(iv.days);
    let time = iv.micros;
    let tm_hour = time / 3_600_000_000;
    let tm_min = (time / 60_000_000) % 60;
    let sec_usecs = time % 60_000_000; // `tm_sec * 1e6 + tm_usec`
    match f {
        ExtractField::Microseconds => Ok(PgNumeric::from_i128(i128::from(sec_usecs))),
        ExtractField::Milliseconds => numeric_scaled(i128::from(sec_usecs), 3),
        ExtractField::Second => numeric_scaled(i128::from(sec_usecs), 6),
        ExtractField::Minute => Ok(PgNumeric::from_i128(i128::from(tm_min))),
        ExtractField::Hour => Ok(PgNumeric::from_i128(i128::from(tm_hour))),
        ExtractField::Day => Ok(PgNumeric::from_i128(i128::from(tm_mday))),
        ExtractField::Week => Ok(PgNumeric::from_i128(i128::from(tm_mday / 7))),
        ExtractField::Month => Ok(PgNumeric::from_i128(i128::from(tm_mon))),
        ExtractField::Quarter => {
            // Keep sign-symmetry with the negated interval (Postgres works from months).
            let q = if months >= 0 {
                tm_mon / 3 + 1
            } else {
                -((-months % 12) / 3 + 1)
            };
            Ok(PgNumeric::from_i128(i128::from(q)))
        }
        ExtractField::Year => Ok(PgNumeric::from_i128(i128::from(tm_year))),
        ExtractField::Decade => Ok(PgNumeric::from_i128(i128::from(tm_year / 10))),
        ExtractField::Century => Ok(PgNumeric::from_i128(i128::from(tm_year / 100))),
        ExtractField::Millennium => Ok(PgNumeric::from_i128(i128::from(tm_year / 1000))),
        ExtractField::Epoch => interval_epoch_numeric(iv),
        _ => Err(unsupported()),
    }
}

/// `EXTRACT(EPOCH FROM interval)` (Postgres `interval_part`): total seconds treating
/// a year as 365.25 days and a remainder-month as 30 days, scale 6. The `× 4 … ÷ 4`
/// trick keeps `DAYS_PER_YEAR = 365.25` in integer arithmetic (`4 × 365.25 = 1461`).
fn interval_epoch_numeric(iv: &PgInterval) -> Result<PgNumeric> {
    let month = i128::from(iv.months);
    let day = i128::from(iv.days);
    let secs_from_day_month = (1461 * (month / 12) + 120 * (month % 12) + 4 * day) * 21_600;
    let val = secs_from_day_month * 1_000_000 + i128::from(iv.micros);
    numeric_scaled(val, 6)
}

/// Evaluate `date_trunc(field, source)` — field/source are non-NULL (the hoisted
/// check in `eval_string_fn` already handled NULL). Only the units tier is valid;
/// a special-only field is unrecognized (`22023`). A `date` source implicit-casts to
/// `timestamp` (result `timestamp`, not `date`); `time`/non-temporal → `42883`.
pub(super) fn eval_date_trunc(field: &Value, source: &Value) -> Result<Value> {
    let Value::Text(field_name) = field else {
        return Err(Error::UndefinedFunction(format!(
            "function date_trunc does not exist for field type {}",
            opt_type_oid_display_name(field.type_oid())
        )));
    };
    let Some(f) = ExtractField::from_name(field_name) else {
        return Err(Error::UnrecognizedTimeUnit(field_name.clone()));
    };
    // `date_trunc` consults only the units tier (Postgres never calls `DecodeSpecial`
    // here), so a special-only field (`dow`/`epoch`/…) is unrecognized, not merely
    // unsupported.
    if !f.is_units() {
        return Err(Error::UnrecognizedTimeUnit(field_name.clone()));
    }
    match source {
        Value::Timestamp(dt) => date_trunc_timestamp(f, *dt, field_name).map(Value::Timestamp),
        Value::Date(d) => {
            let dt = d.map(|d| d.with_time(time::Time::MIDNIGHT).assume_utc());
            date_trunc_timestamp(f, dt, field_name).map(Value::Timestamp)
        }
        Value::Interval(iv) => date_trunc_interval(f, iv, field_name).map(Value::Interval),
        other => Err(Error::UndefinedFunction(format!(
            "function date_trunc does not exist for source type {}",
            opt_type_oid_display_name(other.type_oid())
        ))),
    }
}

/// `year`/`century`/… truncation of the year field for `date_trunc` (Postgres
/// `timestamp_trunc`): round *down* to the decade/century/millennium start, "no year
/// 0" for BC. Distinct from `extract_*` (which return the field *number*, not a year).
const fn trunc_decade(year: i64) -> i64 {
    if year > 0 {
        (year / 10) * 10
    } else {
        -((8 - (year - 1)) / 10) * 10
    }
}
const fn trunc_century(year: i64) -> i64 {
    if year > 0 {
        ((year + 99) / 100) * 100 - 99
    } else {
        -((99 - (year - 1)) / 100) * 100 + 1
    }
}
const fn trunc_millennium(year: i64) -> i64 {
    if year > 0 {
        ((year + 999) / 1000) * 1000 - 999
    } else {
        -((999 - (year - 1)) / 1000) * 1000 + 1
    }
}

/// Truncate a timestamp down to the `f` boundary, zeroing every finer field
/// (Postgres `timestamp_trunc`'s cascade). `week` → Monday of the ISO week. A
/// timezone field is unsupported for a timestamp (`0A000`), an infinity included; any
/// other field leaves an infinity as it is.
fn date_trunc_timestamp(
    f: ExtractField,
    dt: crate::types::PgTimestamp,
    field_name: &str,
) -> Result<crate::types::PgTimestamp> {
    let unsupported = || Error::UnsupportedTimeUnit(field_name.to_string());
    let oor = || Error::TemporalOutOfRange("timestamp");
    let Infinite::Finite(dt) = dt else {
        return if matches!(
            f,
            ExtractField::Timezone | ExtractField::TimezoneHour | ExtractField::TimezoneMinute
        ) {
            Err(unsupported())
        } else {
            Ok(dt)
        };
    };
    let date = dt.date();
    let year = i64::from(date.year());
    let mut ny = year;
    let mut nmon = i64::from(u8::from(date.month()));
    let mut nday = i64::from(date.day());
    let mut nhour = i64::from(dt.hour());
    let mut nmin = i64::from(dt.minute());
    let mut nsec = i64::from(dt.second());
    let mut nusec = i64::from(dt.microsecond());
    // `week` replaces the whole date with the ISO-week Monday (computed directly).
    let mut week_date: Option<time::Date> = None;
    match f {
        ExtractField::Microseconds => {}
        ExtractField::Milliseconds => nusec = (nusec / 1000) * 1000,
        ExtractField::Second => nusec = 0,
        ExtractField::Minute => {
            nsec = 0;
            nusec = 0;
        }
        ExtractField::Hour => {
            nmin = 0;
            nsec = 0;
            nusec = 0;
        }
        ExtractField::Day => {
            nhour = 0;
            nmin = 0;
            nsec = 0;
            nusec = 0;
        }
        ExtractField::Week => {
            let back = i32::from(date.weekday().number_from_monday() - 1);
            let monday =
                time::Date::from_julian_day(date.to_julian_day() - back).map_err(|_| oor())?;
            week_date = Some(monday);
            nhour = 0;
            nmin = 0;
            nsec = 0;
            nusec = 0;
        }
        ExtractField::Month => {
            nday = 1;
            nhour = 0;
            nmin = 0;
            nsec = 0;
            nusec = 0;
        }
        ExtractField::Quarter => {
            nmon = 3 * ((nmon - 1) / 3) + 1;
            nday = 1;
            nhour = 0;
            nmin = 0;
            nsec = 0;
            nusec = 0;
        }
        ExtractField::Year => {
            nmon = 1;
            nday = 1;
            nhour = 0;
            nmin = 0;
            nsec = 0;
            nusec = 0;
        }
        ExtractField::Decade | ExtractField::Century | ExtractField::Millennium => {
            ny = match f {
                ExtractField::Decade => trunc_decade(year),
                ExtractField::Century => trunc_century(year),
                _ => trunc_millennium(year),
            };
            nmon = 1;
            nday = 1;
            nhour = 0;
            nmin = 0;
            nsec = 0;
            nusec = 0;
        }
        // timezone / timezone_hour / timezone_minute — unsupported for a timestamp.
        _ => return Err(unsupported()),
    }
    let result_date = if let Some(d) = week_date {
        d
    } else {
        let month =
            time::Month::try_from(u8::try_from(nmon).map_err(|_| oor())?).map_err(|_| oor())?;
        time::Date::from_calendar_date(
            i32::try_from(ny).map_err(|_| oor())?,
            month,
            u8::try_from(nday).map_err(|_| oor())?,
        )
        .map_err(|_| oor())?
    };
    let ntime = time::Time::from_hms_micro(
        u8::try_from(nhour).map_err(|_| oor())?,
        u8::try_from(nmin).map_err(|_| oor())?,
        u8::try_from(nsec).map_err(|_| oor())?,
        u32::try_from(nusec).map_err(|_| oor())?,
    )
    .map_err(|_| oor())?;
    crate::types::check_timestamp_domain(result_date.with_time(ntime).assume_utc())
        .map(Infinite::Finite)
}

/// Truncate an interval down to the `f` boundary (Postgres `interval_trunc` over
/// `interval2itm` → cascade → `itm2interval`). `week` is unsupported for an interval
/// (`0A000`, "months usually have fractional weeks").
fn date_trunc_interval(f: ExtractField, iv: &PgInterval, field_name: &str) -> Result<PgInterval> {
    let unsupported = || Error::UnsupportedTimeUnit(field_name.to_string());
    let oor = || Error::TemporalOutOfRange("interval");
    let months = i64::from(iv.months);
    let mut tm_year = months / 12;
    let mut tm_mon = months % 12;
    let mut tm_mday = i64::from(iv.days);
    let time = iv.micros;
    let mut tm_hour = time / 3_600_000_000;
    let mut tm_min = (time / 60_000_000) % 60;
    let mut tm_sec = (time / 1_000_000) % 60;
    let mut tm_usec = time % 1_000_000;
    match f {
        ExtractField::Millennium | ExtractField::Century | ExtractField::Decade => {
            tm_year = match f {
                ExtractField::Millennium => (tm_year / 1000) * 1000,
                ExtractField::Century => (tm_year / 100) * 100,
                _ => (tm_year / 10) * 10,
            };
            tm_mon = 0;
            tm_mday = 0;
            tm_hour = 0;
            tm_min = 0;
            tm_sec = 0;
            tm_usec = 0;
        }
        ExtractField::Year => {
            tm_mon = 0;
            tm_mday = 0;
            tm_hour = 0;
            tm_min = 0;
            tm_sec = 0;
            tm_usec = 0;
        }
        ExtractField::Quarter => {
            tm_mon = 3 * (tm_mon / 3);
            tm_mday = 0;
            tm_hour = 0;
            tm_min = 0;
            tm_sec = 0;
            tm_usec = 0;
        }
        ExtractField::Month => {
            tm_mday = 0;
            tm_hour = 0;
            tm_min = 0;
            tm_sec = 0;
            tm_usec = 0;
        }
        ExtractField::Day => {
            tm_hour = 0;
            tm_min = 0;
            tm_sec = 0;
            tm_usec = 0;
        }
        ExtractField::Hour => {
            tm_min = 0;
            tm_sec = 0;
            tm_usec = 0;
        }
        ExtractField::Minute => {
            tm_sec = 0;
            tm_usec = 0;
        }
        ExtractField::Second => tm_usec = 0,
        ExtractField::Milliseconds => tm_usec = (tm_usec / 1000) * 1000,
        ExtractField::Microseconds => {}
        // week / timezone* — unsupported for an interval.
        _ => return Err(unsupported()),
    }
    let new_months = tm_year * 12 + tm_mon;
    let new_micros = tm_hour * 3_600_000_000 + tm_min * 60_000_000 + tm_sec * 1_000_000 + tm_usec;
    Ok(PgInterval {
        months: i32::try_from(new_months).map_err(|_| oor())?,
        days: i32::try_from(tm_mday).map_err(|_| oor())?,
        micros: new_micros,
    })
}
