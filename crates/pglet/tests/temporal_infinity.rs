//! `infinity` / `-infinity` for `date`, `timestamp` and `timestamptz` over the embed engine:
//! the file-backed reopen of a store holding them, the embed door, and the refusal codes
//! the differential fixtures cannot hold because 18.6 answers those statements.
//!
//! The fixtures under `tests/fixtures/types/infinity_*.sql` assert the SQL-visible
//! behaviour against a live Postgres; nothing there closes and reopens a store.

// A test harness: `.unwrap()`/`.expect()` surface a setup failure loudly here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::types::{Infinite, Value};

fn texts(db: &mut Database, sql: &str) -> Vec<Vec<Option<String>>> {
    db.execute(sql)
        .expect(sql)
        .remove(0)
        .rows
        .iter()
        .map(|row| row.iter().map(pglet_sql::types::value_to_text).collect())
        .collect()
}

fn code(db: &mut Database, sql: &str) -> (String, String) {
    let e = db.execute(sql).expect_err(sql);
    (e.sqlstate().to_string(), e.to_string())
}

/// A store holding both infinities in scalar, array and indexed columns reopens with the
/// same rows, the same order, and an index that still answers a range read.
#[test]
fn infinities_survive_a_reopen_with_their_index() {
    let dir = ScratchDir::new("temporal_infinity_reopen");
    let read = "SELECT id, d, ts, tz, ds FROM inf WHERE d >= date '9999-12-31' OR tz < timestamptz '0001-01-01 00:00:00+00' ORDER BY d, id";
    let before = {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute(
            "CREATE TABLE inf (id INT PRIMARY KEY, d DATE UNIQUE, ts TIMESTAMP, tz TIMESTAMPTZ, ds DATE[]); \
             CREATE INDEX inf_tz ON inf (tz); \
             INSERT INTO inf VALUES (1, 'infinity', '-infinity', '-infinity', '{infinity,-infinity}'), \
             (2, '-infinity', 'infinity', 'infinity', NULL), (3, '9999-12-31', NULL, '2000-01-01 00:00:00+00', '{2000-01-01}')",
        )
        .expect("seed");
        texts(&mut db, read)
    };
    assert_eq!(
        before,
        vec![
            vec![
                Some("3".into()),
                Some("9999-12-31".into()),
                None,
                Some("2000-01-01 00:00:00".into()),
                Some("{2000-01-01}".into()),
            ],
            vec![
                Some("1".into()),
                Some("infinity".into()),
                Some("-infinity".into()),
                Some("-infinity".into()),
                Some("{infinity,-infinity}".into()),
            ],
        ]
    );
    let mut db = Database::open(&dir.path).expect("reopen");
    assert_eq!(texts(&mut db, read), before);
    let (sqlstate, _) = code(&mut db, "INSERT INTO inf (id, d) VALUES (9, 'infinity')");
    assert_eq!(sqlstate, "23505");
}

/// The refusals 18.6 does not share — an infinite interval, and `EXTRACT`'s numeric
/// `Infinity` — are `0A000`, and the ones it does share carry its code and sentence.
#[test]
fn the_refusals_carry_their_codes() {
    let mut db = Database::with_btree();
    for (sql, want) in [
        (
            "SELECT timestamp 'infinity' - timestamp '2000-01-01'",
            ("0A000", "not implemented: infinite interval"),
        ),
        (
            "SELECT timestamp '2000-01-01' - timestamptz '-infinity'",
            ("0A000", "not implemented: infinite interval"),
        ),
        (
            "SELECT timestamp 'infinity' - timestamp '-infinity'",
            ("0A000", "not implemented: infinite interval"),
        ),
        (
            "SELECT extract(epoch FROM date 'infinity')",
            ("0A000", "not implemented: numeric Infinity"),
        ),
        (
            "SELECT extract(year FROM timestamp '-infinity')",
            ("0A000", "not implemented: numeric Infinity"),
        ),
        (
            "SELECT timestamp '-infinity' - timestamp '-infinity'",
            ("22008", "interval out of range"),
        ),
        (
            "SELECT date 'infinity' - date '2000-01-01'",
            ("22008", "cannot subtract infinite dates"),
        ),
        (
            "SELECT date '2000-01-01' - date '-infinity'",
            ("22008", "cannot subtract infinite dates"),
        ),
    ] {
        let (sqlstate, message) = code(&mut db, sql);
        assert_eq!((sqlstate.as_str(), message.as_str()), want, "{sql}");
    }
}

/// A `Value` bound through `Database::execute_bound` may hold an infinity, and it stores,
/// compares and renders as the one a literal builds.
#[test]
fn an_infinity_bound_through_the_embed_door_stores() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE b (d DATE, tz TIMESTAMPTZ)")
        .expect("create");
    db.execute_bound("INSERT INTO b VALUES ($1, $2)", 0, |_| {
        Ok::<_, ()>(vec![
            Some(Value::Date(Infinite::Infinity)),
            Some(Value::Timestamp(Infinite::NegInfinity)),
        ])
    })
    .expect("bound insert");
    assert_eq!(
        texts(
            &mut db,
            "SELECT d, tz, d = date 'infinity', tz < timestamptz '0001-01-01 00:00:00+00' FROM b"
        ),
        vec![vec![
            Some("infinity".into()),
            Some("-infinity".into()),
            Some("t".into()),
            Some("t".into()),
        ]]
    );
}

/// A stored infinity is not an unreadable value: the dump exports it as the word, which both
/// engines read back (`test_dump` restores it into each).
#[test]
fn a_stored_infinity_dumps_as_the_word() {
    let mut db = Database::with_btree();
    db.execute(
        "CREATE TABLE dmp (d DATE, ts TIMESTAMP, tz TIMESTAMPTZ, ds DATE[]); \
         INSERT INTO dmp VALUES ('infinity', '-infinity', 'infinity', '{-infinity,infinity}')",
    )
    .expect("seed");
    let dump = db.dump_sql().expect("a stored infinity dumps");
    assert!(
        dump.contains("infinity\t-infinity\tinfinity\t{-infinity,infinity}"),
        "{dump}"
    );
}

/// A `timestamp` bound through `Database::execute_bound` with nanoseconds below the
/// microsecond — what `OffsetDateTime::now_utc()` builds — answers alike before and after
/// the store is closed and reopened, because the door makes it a microsecond value, as
/// every value 18.6 holds is. The door **truncates toward the earlier instant**, as a
/// clock read (`now()`) does, so a pre-1970 instant goes back a microsecond rather than
/// forward to the Unix epoch: the persisted count once divided the nanoseconds away
/// toward zero at the Unix epoch while the in-memory value kept them, so `=` against the
/// microsecond went from no rows to one across a reopen, and the pre-1970 instant read
/// back one microsecond later.
#[test]
fn a_sub_microsecond_timestamp_bound_through_the_embed_door_answers_alike_across_a_reopen() {
    use time::macros::datetime;
    let dir = ScratchDir::new("temporal_infinity_sub_micro");
    let read = "SELECT id, ts, tz, ts = timestamp '2026-01-01 00:00:00', \
                ts > timestamp '2026-01-01 00:00:00', \
                tz = timestamptz '1969-12-31 23:59:59.999999+00', \
                (SELECT count(*) FROM s WHERE tz = timestamptz '2026-01-01 00:00:00.000001+00') \
                FROM s ORDER BY id";
    let before = {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute(
            "CREATE TABLE s (id INT PRIMARY KEY, ts TIMESTAMP, tz TIMESTAMPTZ); \
             CREATE INDEX s_tz ON s (tz)",
        )
        .expect("create");
        for (id, instant) in [
            // Just below a microsecond: truncation keeps the earlier one, rounding
            // would not.
            (1, datetime!(2026-01-01 00:00:00.000000999 UTC)),
            // Half a microsecond before the Unix epoch: truncation toward the earlier
            // instant answers `…59.999999`; toward zero, or rounding half to even,
            // answers the epoch.
            (2, datetime!(1969-12-31 23:59:59.9999995 UTC)),
            (3, datetime!(2026-01-01 00:00:00.0000019 UTC)),
        ] {
            db.execute_bound("INSERT INTO s VALUES ($1, $2, $3)", 0, move |_| {
                Ok::<_, ()>(vec![
                    Some(Value::Int4(id)),
                    Some(Value::timestamp(instant)),
                    Some(Value::timestamp(instant)),
                ])
            })
            .expect("bound insert");
        }
        let bound_compare = db
            .execute_bound("SELECT $1 = timestamp '2026-01-01 00:00:00'", 0, |_| {
                Ok::<_, ()>(vec![Some(Value::timestamp(datetime!(
                    2026-01-01 00:00:00.0000005 UTC
                )))])
            })
            .expect("bound compare");
        assert_eq!(
            pglet_sql::types::value_to_text(&bound_compare.rows[0][0]).as_deref(),
            Some("t")
        );
        texts(&mut db, read)
    };
    let row = |id: &str, ts: &str, tz: &str, eq: &str, gt: &str, eq_neg: &str| {
        vec![
            Some(id.into()),
            Some(ts.into()),
            Some(tz.into()),
            Some(eq.into()),
            Some(gt.into()),
            Some(eq_neg.into()),
            Some("1".into()),
        ]
    };
    assert_eq!(
        before,
        vec![
            row(
                "1",
                "2026-01-01 00:00:00",
                "2026-01-01 00:00:00",
                "t",
                "f",
                "f"
            ),
            row(
                "2",
                "1969-12-31 23:59:59.999999",
                "1969-12-31 23:59:59.999999",
                "f",
                "f",
                "t"
            ),
            row(
                "3",
                "2026-01-01 00:00:00.000001",
                "2026-01-01 00:00:00.000001",
                "f",
                "t",
                "f"
            ),
        ]
    );
    let mut db = Database::open(&dir.path).expect("reopen");
    assert_eq!(texts(&mut db, read), before);
}
