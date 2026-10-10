//! The `date` / `timestamp` / `timestamptz` **domain**: `0001-01-01` through
//! `9999-12-31`, a `timestamptz` measured in UTC.
//!
//! 18.6's own ranges are far wider — `4714-11-24 BC` up to `5874897-12-31` for a date,
//! up to `294276-12-31` for a timestamp — and it renders the years before 1 with a
//! ` BC` suffix. pglet models no BC and stores its dates in `time`'s types, whose
//! range ends at year 9999 (the `large-dates` feature is off). A finite value outside the
//! domain is therefore **refused**, never produced: `time` would represent a year 0 or
//! a negative year and every encoder would print it (`0000-12-31`), which no Postgres
//! input function reads back — a dump carrying one restores its table short, with
//! `psql` exiting `0`. `infinity` and `-infinity` ([`super::Infinite`]) are not finite
//! values, and pass.
//!
//! [`check_date_domain`] and [`check_timestamp_domain`] are the one predicate, and
//! each producer of a `Value::Date` / `Value::Timestamp` calls one of them on the value
//! it is about to return; `docs/temporal.md` § The date and timestamp domain lists the
//! producers. They answer `22008 date out of range` / `22008 timestamp out of range`
//! — 18.6's own sentences for a value leaving its range — and a producer where 18.6
//! refuses with a different sentence (a year-0 *field* in a text literal) raises that
//! one before it gets here.

use time::macros::date;
use time::{Date, OffsetDateTime, UtcOffset};

use super::{Infinite, TypeOid, Value};
use crate::{Error, Result};

/// The first day of the domain, `0001-01-01`.
pub const MIN_DOMAIN_DATE: Date = date!(0001 - 01 - 01);

/// The last day of the domain, `9999-12-31`.
pub const MAX_DOMAIN_DATE: Date = date!(9999 - 12 - 31);

/// Refuse a `date` outside `0001-01-01 ..= 9999-12-31`.
///
/// # Errors
///
/// [`Error::TemporalOutOfRange`] (`22008`, `date out of range`) outside the domain.
pub fn check_date_domain(d: Date) -> Result<Date> {
    if (MIN_DOMAIN_DATE..=MAX_DOMAIN_DATE).contains(&d) {
        Ok(d)
    } else {
        Err(Error::TemporalOutOfRange("date"))
    }
}

/// Refuse a `timestamp` / `timestamptz` whose **UTC** date is outside
/// `0001-01-01 ..= 9999-12-31`, returning the instant at the UTC offset.
///
/// The UTC conversion is `checked_to_offset`, never `to_offset`: an instant written
/// at a non-UTC offset can be representable where its UTC reading is not
/// (`9999-12-31 23:00:00-05` is year 10000 in UTC), and `to_offset` panics there —
/// which under `panic = "abort"` ends the process.
///
/// # Errors
///
/// [`Error::TemporalOutOfRange`] (`22008`, `timestamp out of range`) outside the domain,
/// or when the UTC reading is not representable at all.
pub fn check_timestamp_domain(ts: OffsetDateTime) -> Result<OffsetDateTime> {
    let oor = || Error::TemporalOutOfRange("timestamp");
    let utc = ts.checked_to_offset(UtcOffset::UTC).ok_or_else(oor)?;
    if (MIN_DOMAIN_DATE..=MAX_DOMAIN_DATE).contains(&utc.date()) {
        Ok(utc)
    } else {
        Err(oor())
    }
}

/// Refuse a [`Value`] that is, or holds as an array element, an out-of-domain `date` or
/// `timestamp`, and return it with every `timestamp` at the UTC offset and a whole
/// microsecond.
///
/// The check for a value that arrives **already built**, where no producer ran: a
/// `Value` an embedding caller binds through `Database::execute_bound`, and a value a
/// write is about to store. The normalisation is half the check. An instant's offset is
/// read by more than its UTC comparison: the text renderers print its fields at that
/// offset, and `+ interval`, `extract` and `date_trunc` take their calendar fields from
/// it. So a `timestamp` whose UTC reading is inside the domain can still render outside
/// it — `0000-12-31 22:00:00-05` is `0001-01-01 03:00:00` in UTC — and computes
/// differently from the same instant at UTC, unless it is stored at UTC, as every
/// producer builds one. (The `timestamp → date` cast reads the UTC date itself.) Its
/// nanoseconds below the microsecond are dropped too — truncated toward the earlier
/// instant, the rule `Value::Timestamp`'s documentation states and argues — since the
/// store persists a microsecond count and a value keeping them would answer differently
/// once the store is reopened.
///
/// It works in place: a value holding no `date` or `timestamp` is walked and handed back,
/// never rebuilt — every array element included, whatever the array's declared element
/// type (the store's cheaper form is [`normalize_stored_value_domain`]).
///
/// # Errors
///
/// The first out-of-domain element's [`check_date_domain`] / [`check_timestamp_domain`]
/// refusal.
pub fn normalize_value_domain(mut value: Value) -> Result<Value> {
    normalize_in_place(&mut value, ArrayWalk::Every)?;
    Ok(value)
}

/// [`normalize_value_domain`] over a borrowed value, for the store's finalizer.
///
/// A value holding no `date` or `timestamp` is neither moved nor rebuilt, and an array is
/// walked only when its element type is `date`, `timestamp` or `timestamptz`.
///
/// **The element-type gate rests on where the finalizer's arrays come from**, which is
/// why it is this function's and not [`normalize_value_domain`]'s — reasoned from the
/// producers, not enforced by the type: the readers and the array constructor build an
/// array's elements at its declared element type, and `Database::execute_bound`, where a
/// caller can hand over an `ArrayValue` whose declared element type does not match its
/// elements, walks every element before any write sees it. A store written without the
/// domain could hold such an array, and this gate would let a write copy it — but such a
/// store is older than the `PGMO` store magic, which reopen refuses; the dump still refuses
/// the copy.
///
/// # Errors
///
/// As [`normalize_value_domain`].
#[inline]
pub fn normalize_stored_value_domain(value: &mut Value) -> Result<()> {
    normalize_in_place(value, ArrayWalk::TemporalElementTypes)
}

/// Which arrays [`normalize_in_place`] walks.
#[derive(Clone, Copy)]
enum ArrayWalk {
    /// Every array — a value no coercion has typed.
    Every,
    /// Only an array whose declared element type is a `date` or a `timestamp`.
    TemporalElementTypes,
}

#[inline]
fn normalize_in_place(value: &mut Value, walk: ArrayWalk) -> Result<()> {
    match value {
        // The common case answered from the year alone — the domain is whole years — so a
        // store of an in-domain value pays two compares and no conversion.
        // An infinity is inside what may be stored: it has no calendar fields to leave
        // the domain with, and no offset to render at.
        Value::Date(Infinite::Finite(d)) if DOMAIN_YEARS.contains(&d.year()) => {}
        Value::Date(Infinite::Finite(d)) => {
            check_date_domain(*d)?;
        }
        Value::Timestamp(Infinite::Finite(ts))
            if ts.offset().is_utc()
                && ts.nanosecond() % 1_000 == 0
                && DOMAIN_YEARS.contains(&ts.year()) => {}
        // The sub-second field is never negative, so dropping its nanoseconds below the
        // microsecond truncates toward the earlier instant, as `truncate_timestamp_to_micros`
        // does. That cannot carry an in-domain instant out of the domain, whose edges are
        // whole microseconds, so the order of the two is free.
        Value::Timestamp(Infinite::Finite(ts)) => {
            let utc = check_timestamp_domain(*ts)?;
            *ts = utc
                .replace_nanosecond(utc.nanosecond() / 1_000 * 1_000)
                .unwrap_or(utc);
        }
        Value::Array(arr)
            if matches!(walk, ArrayWalk::Every)
                || matches!(
                    arr.element_type,
                    TypeOid::Date | TypeOid::Timestamp | TypeOid::Timestamptz
                ) =>
        {
            for element in &mut arr.elements {
                normalize_in_place(element, walk)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The domain's years, `MIN_DOMAIN_DATE.year() ..= MAX_DOMAIN_DATE.year()`: the domain runs
/// from a year's first day to a year's last, so a year inside this range is a date inside
/// the domain.
const DOMAIN_YEARS: std::ops::RangeInclusive<i32> = 1..=9999;

#[cfg(test)]
mod tests {
    use super::{
        check_date_domain, check_timestamp_domain, normalize_stored_value_domain,
        normalize_value_domain,
    };
    use crate::types::{ArrayValue, Infinite, TypeOid, Value};
    use time::macros::{date, datetime};

    #[test]
    fn a_date_is_accepted_at_both_edges_and_refused_below_the_lower() {
        for d in [
            date!(0001 - 01 - 01),
            date!(9999 - 12 - 31),
            date!(2000 - 01 - 01),
        ] {
            assert_eq!(check_date_domain(d).expect("in domain"), d);
        }
        for d in [
            date!(0000 - 12 - 31),
            date!(-0001 - 06 - 15),
            date!(-9999 - 01 - 01),
        ] {
            let e = check_date_domain(d).expect_err("out of domain");
            assert_eq!(e.sqlstate(), "22008");
            assert_eq!(e.to_string(), "date out of range");
        }
    }

    #[test]
    fn a_timestamp_is_accepted_at_both_edges_and_refused_one_microsecond_past_the_lower() {
        for ts in [
            datetime!(0001-01-01 00:00:00 UTC),
            datetime!(9999-12-31 23:59:59.999999 UTC),
        ] {
            assert_eq!(check_timestamp_domain(ts).expect("in domain"), ts);
        }
        let e = check_timestamp_domain(datetime!(0000-12-31 23:59:59.999999 UTC))
            .expect_err("one microsecond below the domain");
        assert_eq!(e.sqlstate(), "22008");
        assert_eq!(e.to_string(), "timestamp out of range");
    }

    /// A value built outside every producer is judged the same way, inside an array too,
    /// and comes back at UTC — including one whose UTC reading is in the domain while its
    /// own fields are not.
    #[test]
    fn a_built_value_is_judged_through_arrays_and_normalised_to_utc() {
        assert_eq!(
            normalize_value_domain(Value::date(date!(0001 - 01 - 01))).unwrap(),
            Value::date(date!(0001 - 01 - 01))
        );
        assert_eq!(
            normalize_value_domain(Value::Int4(0)).unwrap(),
            Value::Int4(0)
        );
        let e = normalize_value_domain(Value::date(date!(0000 - 12 - 31))).expect_err("year 0");
        assert_eq!(e.to_string(), "date out of range");
        let arr = |elements| {
            Value::Array(ArrayValue {
                element_type: TypeOid::Timestamp,
                elements,
            })
        };
        let e = normalize_value_domain(arr(vec![
            Value::timestamp(datetime!(2000-01-01 00:00:00 UTC)),
            Value::timestamp(datetime!(0000-12-31 23:59:59 UTC)),
        ]))
        .expect_err("an element in year 0");
        assert_eq!(e.to_string(), "timestamp out of range");
        let local = datetime!(0000-12-31 22:00:00 -5);
        let utc = datetime!(0001-01-01 03:00:00 UTC);
        for (built, normalised) in [
            (Value::timestamp(local), Value::timestamp(utc)),
            (
                arr(vec![Value::timestamp(local)]),
                arr(vec![Value::timestamp(utc)]),
            ),
        ] {
            let got = normalize_value_domain(built).expect("in domain in UTC");
            assert_eq!(got, normalised);
            let Value::Timestamp(Infinite::Finite(ts)) = (match got {
                Value::Array(a) => a.elements.into_iter().next().expect("one element"),
                v => v,
            }) else {
                panic!("a timestamp")
            };
            assert!(ts.offset().is_utc(), "stored and read at UTC");
            assert_eq!(ts.year(), 1);
        }
    }

    /// The store's form walks an array by its declared element type: a `timestamp[]` is
    /// brought to UTC element by element, while an array declared as another type is not
    /// walked — the gate the full form does not have.
    #[test]
    fn the_stored_form_walks_temporal_arrays_only() {
        let local = datetime!(2026-06-15 22:00:00 -5);
        let mut ts_arr = Value::Array(ArrayValue {
            element_type: TypeOid::Timestamptz,
            elements: vec![Value::timestamp(local)],
        });
        normalize_stored_value_domain(&mut ts_arr).expect("in domain");
        assert_eq!(
            ts_arr,
            Value::Array(ArrayValue {
                element_type: TypeOid::Timestamptz,
                elements: vec![Value::timestamp(datetime!(2026-06-16 03:00:00 UTC))],
            })
        );
        let Value::Array(arr) = &ts_arr else {
            unreachable!()
        };
        let Value::Timestamp(Infinite::Finite(ts)) = arr.elements[0] else {
            unreachable!()
        };
        assert!(ts.offset().is_utc());
        let mut mislabelled = Value::Array(ArrayValue {
            element_type: TypeOid::Int4,
            elements: vec![Value::date(date!(0000 - 12 - 31))],
        });
        assert!(normalize_stored_value_domain(&mut mislabelled).is_ok());
        assert!(normalize_value_domain(mislabelled).is_err());
    }

    /// An infinity is inside what may be stored, in both forms and inside an array.
    #[test]
    fn an_infinity_is_inside_the_domain() {
        for v in [
            Value::Date(Infinite::Infinity),
            Value::Date(Infinite::NegInfinity),
            Value::Timestamp(Infinite::Infinity),
            Value::Array(ArrayValue {
                element_type: TypeOid::Timestamptz,
                elements: vec![Value::Timestamp(Infinite::NegInfinity)],
            }),
        ] {
            let mut stored = v.clone();
            normalize_stored_value_domain(&mut stored).expect("stored");
            assert_eq!(stored, v);
            assert_eq!(normalize_value_domain(v.clone()).expect("bound"), v);
        }
    }

    /// The domain is read in UTC: an offset moves an instant across either edge, and
    /// the upper crossing is one `to_offset` would have panicked on.
    #[test]
    fn a_timestamp_is_judged_by_its_utc_reading_and_never_panics() {
        // Written in year 1, UTC in year 0.
        assert!(check_timestamp_domain(datetime!(0001-01-01 04:59:59.999999 +5)).is_err());
        assert_eq!(
            check_timestamp_domain(datetime!(0001-01-01 05:00:00 +5)).expect("in domain"),
            datetime!(0001-01-01 00:00:00 UTC)
        );
        // Written in year 9999, UTC in year 10000 — unrepresentable, refused.
        assert!(check_timestamp_domain(datetime!(9999-12-31 19:00:00 -5)).is_err());
        assert_eq!(
            check_timestamp_domain(datetime!(9999-12-31 18:59:59.999999 -5)).expect("in domain"),
            datetime!(9999-12-31 23:59:59.999999 UTC)
        );
    }
}
