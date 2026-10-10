//! The `date` / `timestamp` / `timestamptz` domain, `0001-01-01 ..= 9999-12-31`
//! (`timestamptz` read in UTC), over the embed engine.
//!
//! The unit tests beside each reader and each arithmetic producer pin the edges one
//! function at a time; this file pins what only a whole statement shows:
//!
//! - **No input panics.** A `timestamptz` literal whose explicit offset carries its UTC
//!   reading past year 9999 once panicked inside `time`'s `to_offset`, which under the
//!   release profile's `panic = "abort"` ended the server process. It is an error here,
//!   at each spelling that reaches the reader.
//! - **No out-of-domain value is stored.** A year-0 date written through arithmetic was
//!   accepted, stored and dumped as `0000-12-31`, which 18.6's `COPY` refuses — and
//!   `psql -f` exits `0` with the table empty. Each write route is refused, the table
//!   keeps its in-domain rows, and the dump carries the edges and nothing past them.

// A test harness: `.expect()` / `panic!` surface a setup failure loudly right here.
#![allow(clippy::expect_used, clippy::panic)]

use pglet::Database;

/// The SQLSTATE and message `sql` fails with, or `None` when it succeeds.
fn refusal(db: &mut Database, sql: &str) -> Option<(&'static str, String)> {
    db.execute(sql).err().map(|e| (e.sqlstate(), e.to_string()))
}

/// The single text value `sql` answers.
fn text(db: &mut Database, sql: &str) -> String {
    let rows = db
        .execute(sql)
        .unwrap_or_else(|e| panic!("`{sql}` answers: {e}"))
        .remove(0)
        .rows;
    let v = rows
        .first()
        .and_then(|r| r.first())
        .unwrap_or_else(|| panic!("expected one value from `{sql}`"));
    pglet_sql::types::value_to_text(v).unwrap_or_else(|| "NULL".to_string())
}

fn timestamp_out_of_range() -> (&'static str, String) {
    ("22008", "timestamp out of range".to_string())
}

fn date_out_of_range() -> (&'static str, String) {
    ("22008", "date out of range".to_string())
}

/// The literal that aborted the release server, at each spelling that reaches the
/// `timestamptz` reader, and its lower-edge mirror; the same instants one microsecond
/// inside are values.
#[test]
fn an_offset_crossing_either_edge_is_an_error_and_not_a_panic() {
    let mut db = Database::with_btree();
    for sql in [
        "SELECT timestamptz '9999-12-31 23:00:00-05'",
        "SELECT '9999-12-31 23:00:00-05'::timestamptz",
        "SELECT CAST('9999-12-31T23:00:00-05:00' AS timestamptz)",
        "SELECT '9999-12-31 19:00:00-05'::timestamptz",
        "SELECT timestamptz '0001-01-01 03:00:00+05'",
        "SELECT '0001-01-01 04:59:59.999999+05'::timestamptz",
    ] {
        assert_eq!(
            refusal(&mut db, sql),
            Some(timestamp_out_of_range()),
            "{sql}"
        );
    }
    assert_eq!(
        text(
            &mut db,
            "SELECT '9999-12-31 18:59:59.999999-05'::timestamptz"
        ),
        "9999-12-31 23:59:59.999999"
    );
    assert_eq!(
        text(&mut db, "SELECT '0001-01-01 05:00:00+05'::timestamptz"),
        "0001-01-01 00:00:00"
    );
    // A `timestamp` drops the offset, so the same spelling is its written fields.
    assert_eq!(
        text(&mut db, "SELECT '9999-12-31 23:00:00-05'::timestamp"),
        "9999-12-31 23:00:00"
    );
}

/// Each SQL producer, one step past each edge it can cross, beside the same step
/// landing on the edge. The answer is `22008` with the producer's range sentence,
/// whether 18.6 refuses the statement too or answers a `BC` / year-10000 value.
#[test]
fn each_producer_refuses_one_step_past_the_edge_and_keeps_the_edge() {
    let mut db = Database::with_btree();
    let on_edge = [
        ("SELECT '0001-01-02'::date - 1", "0001-01-01"),
        ("SELECT 1 + '9999-12-30'::date", "9999-12-31"),
        (
            "SELECT date '0001-01-02' - interval '1 day'",
            "0001-01-01 00:00:00",
        ),
        (
            "SELECT timestamp '9999-12-31 23:59:59.999998' + interval '1 microsecond'",
            "9999-12-31 23:59:59.999999",
        ),
        (
            "SELECT timestamptz '0001-01-01 00:00:01+00' - interval '1 second'",
            "0001-01-01 00:00:00",
        ),
        (
            "SELECT date '9999-12-31' + time '23:59:59.999999'",
            "9999-12-31 23:59:59.999999",
        ),
        (
            "SELECT date_trunc('decade', timestamp '0010-06-01')",
            "0010-01-01 00:00:00",
        ),
        (
            "SELECT '0001-01-01'::date::timestamptz - interval '0 days'",
            "0001-01-01 00:00:00",
        ),
    ];
    for (sql, value) in on_edge {
        assert_eq!(text(&mut db, sql), value, "{sql}");
    }
    let dates = [
        "SELECT '0001-01-01'::date - 1",
        "SELECT '0001-01-01'::date + (-1)",
        "SELECT '9999-12-31'::date + 1",
        "SELECT '0001-01-01'::date - 800000",
        "SELECT '0001-01-01'::date - 2147483647",
        "SELECT '10000-01-01'::date",
    ];
    for sql in dates {
        assert_eq!(refusal(&mut db, sql), Some(date_out_of_range()), "{sql}");
    }
    let timestamps = [
        "SELECT '0001-01-01'::date - interval '1 day'",
        "SELECT interval '-1 day' + '0001-01-01'::date",
        "SELECT timestamp '0001-01-01 00:00:00' - interval '1 microsecond'",
        "SELECT timestamptz '0001-01-01 00:00:00+00' - interval '1 second'",
        "SELECT timestamp '0001-01-01 00:00:00' + interval '-1 month'",
        "SELECT timestamp '0001-01-01 00:00:00' - interval '10000 years'",
        "SELECT timestamp '9999-12-31 23:59:59.999999' + interval '1 microsecond'",
        "SELECT date '9999-12-31' + time '24:00'",
        "SELECT date_trunc('decade', timestamp '0005-06-01')",
        "SELECT date_trunc('decade', date '0009-12-31')",
        "SELECT '9999-12-31 23:59:59.9999995'::timestamp",
    ];
    for sql in timestamps {
        assert_eq!(
            refusal(&mut db, sql),
            Some(timestamp_out_of_range()),
            "{sql}"
        );
    }
    // A year-0 field in a literal is 18.6's own refusal, quoting what was written.
    for (sql, literal) in [
        ("SELECT '0000-12-31'::date", "0000-12-31"),
        ("SELECT date '0000-01-01'", "0000-01-01"),
        (
            "SELECT '0000-06-15 12:00:00'::timestamp",
            "0000-06-15 12:00:00",
        ),
        (
            "SELECT '0000-06-15 12:00:00+00'::timestamptz",
            "0000-06-15 12:00:00+00",
        ),
    ] {
        assert_eq!(
            refusal(&mut db, sql),
            Some((
                "22008",
                format!("date/time field value out of range: \"{literal}\"")
            )),
            "{sql}"
        );
    }
}

/// No write route stores a value past either edge, and what the table does hold — the
/// edges themselves — dumps as text both engines read back. The `COPY` payload the
/// dump emits is re-ingested into a fresh engine and compared row for row.
#[test]
fn no_write_route_stores_past_an_edge_and_the_edges_dump_and_reload() {
    const DDL: &str = "CREATE TABLE d (id INT PRIMARY KEY, d DATE, t TIMESTAMP, tz TIMESTAMPTZ)";
    const ALL: &str = "SELECT id, d::text, t::text, tz::text FROM d ORDER BY id";
    let mut db = Database::with_btree();
    db.execute(DDL).expect("create");
    db.execute(
        "INSERT INTO d VALUES \
         (1, '0001-01-01', '0001-01-01 00:00:00', '0001-01-01 00:00:00+00'), \
         (2, '9999-12-31', '9999-12-31 23:59:59.999999', '9999-12-31 23:59:59.999999+00')",
    )
    .expect("the edges are in the domain");
    let before = db.execute(ALL).expect("read").remove(0).rows;

    for sql in [
        "INSERT INTO d VALUES (3, '0001-01-01'::date - 1, NULL, NULL)",
        "INSERT INTO d VALUES (3, '0000-12-31', NULL, NULL)",
        "INSERT INTO d VALUES (3, NULL, timestamp '0001-01-01' - interval '1 second', NULL)",
        "INSERT INTO d VALUES (3, NULL, NULL, '9999-12-31 23:00:00-05')",
        "INSERT INTO d SELECT 3, d - 1, t, tz FROM d WHERE id = 1",
        "UPDATE d SET d = d - 1 WHERE id = 1",
        "UPDATE d SET t = t + interval '1 microsecond' WHERE id = 2",
        "UPDATE d SET tz = tz - interval '1 day' WHERE id = 1",
        "INSERT INTO d VALUES (1, '0001-01-01', NULL, NULL) \
         ON CONFLICT (id) DO UPDATE SET d = excluded.d - 1",
    ] {
        let (state, _) = refusal(&mut db, sql).unwrap_or_else(|| panic!("`{sql}` is refused"));
        assert_eq!(state, "22008", "{sql}");
    }
    let copied = db.copy_in("COPY d FROM STDIN", b"3\t0000-12-31\t\\N\t\\N\n");
    assert_eq!(copied.expect_err("COPY refuses year 0").sqlstate(), "22008");

    let after = db.execute(ALL).expect("read").remove(0).rows;
    assert_eq!(before, after, "no refused write moved a row");

    let dump = db.dump_sql().expect("dump");
    for edge in [
        "1\t0001-01-01\t0001-01-01 00:00:00\t0001-01-01 00:00:00+00",
        "2\t9999-12-31\t9999-12-31 23:59:59.999999\t9999-12-31 23:59:59.999999+00",
    ] {
        assert!(dump.contains(edge), "the dump carries `{edge}`:\n{dump}");
    }
    assert!(!dump.contains("0000-"), "no year 0 in the dump:\n{dump}");

    // Reload: the `COPY` block's payload through `copy_in`, into the same DDL.
    let payload: String = dump
        .lines()
        .skip_while(|line| !line.starts_with("COPY d "))
        .skip(1)
        .take_while(|line| *line != "\\.")
        .flat_map(|line| [line, "\n"])
        .collect();
    let mut reloaded = Database::with_btree();
    reloaded.execute(DDL).expect("create");
    reloaded
        .copy_in("COPY d FROM STDIN", payload.as_bytes())
        .expect("the dumped edges reload");
    assert_eq!(reloaded.execute(ALL).expect("read").remove(0).rows, before);
}

/// A `Value` an embedding caller binds through `Database::execute_bound` was built
/// outside every producer, so the door judges it: a year-0 date is refused at a store
/// and at a read alike, inside an array too, and an in-domain one is kept.
#[test]
fn a_value_bound_through_the_embed_door_is_judged_too() {
    use pglet_sql::types::{ArrayValue, TypeOid, Value};
    let first = pglet_sql::types::parse_date("0001-01-01").expect("in domain");
    let year_zero = first.map(|d| d.previous_day().expect("time holds year 0"));
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE d (d DATE, ds DATE[])")
        .expect("create");
    let bind = |sql: &str, db: &mut Database, v: Value| {
        db.execute_bound(sql, 0, move |_| Ok::<_, ()>(vec![Some(v)]))
            .map(|r| r.rows.len())
            .map_err(|e| match e {
                pglet::BoundExecError::Engine(e) => (e.sqlstate(), e.to_string()),
                pglet::BoundExecError::Bind(()) => panic!("the binder does not fail"),
            })
    };
    for (sql, v) in [
        ("INSERT INTO d (d) VALUES ($1)", Value::Date(year_zero)),
        ("SELECT $1::date", Value::Date(year_zero)),
        (
            "INSERT INTO d (ds) VALUES ($1)",
            Value::Array(ArrayValue {
                element_type: TypeOid::Date,
                elements: vec![Value::Date(first), Value::Date(year_zero)],
            }),
        ),
        // The door walks every element, whatever element type the array declares.
        (
            "SELECT $1::text",
            Value::Array(ArrayValue {
                element_type: TypeOid::Int4,
                elements: vec![Value::Date(year_zero)],
            }),
        ),
    ] {
        assert_eq!(bind(sql, &mut db, v), Err(date_out_of_range()), "{sql}");
    }
    assert_eq!(
        bind("INSERT INTO d (d) VALUES ($1)", &mut db, Value::Date(first)),
        Ok(0)
    );
    assert_eq!(text(&mut db, "SELECT count(*) FROM d"), "1");
    assert!(!db.dump_sql().expect("dump").contains("0000-"));
}

/// A bound `timestamp` at a non-UTC offset is judged, stored and rendered at UTC. The
/// instant below is `0001-01-01 03:00:00` in UTC and `0000-12-31 22:00:00` at its own
/// offset. The text renderers print an instant's fields at the offset it carries, so a
/// door that only judged the UTC reading let this value render as year 0 in text, in
/// three stored columns and in the dump; the door brings it to UTC. (`$1::date` reads the
/// UTC date either way.)
#[test]
fn a_bound_timestamp_at_another_offset_is_stored_and_rendered_at_utc() {
    use pglet_sql::types::{ArrayValue, TypeOid, Value};
    use time::macros::datetime;
    let local = Value::timestamp(datetime!(0000-12-31 22:00:00 -5));
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE d (id INT, t TIMESTAMP, tz TIMESTAMPTZ, d DATE, ts TIMESTAMP[])")
        .expect("create");
    let one = |db: &mut Database, sql: &str, v: Vec<Value>| {
        let mut r = db
            .execute_bound(sql, 0, move |_| {
                Ok::<_, ()>(v.into_iter().map(Some).collect())
            })
            .unwrap_or_else(|_| panic!("`{sql}` answers"));
        r.rows
            .pop()
            .and_then(|row| row.into_iter().next())
            .map(|v| pglet_sql::types::value_to_text(&v).unwrap_or_else(|| "NULL".into()))
    };
    assert_eq!(
        one(&mut db, "SELECT $1::text", vec![local.clone()]).as_deref(),
        Some("0001-01-01 03:00:00")
    );
    assert_eq!(
        one(&mut db, "SELECT $1::date", vec![local.clone()]).as_deref(),
        Some("0001-01-01")
    );
    let arr = Value::Array(ArrayValue {
        element_type: TypeOid::Timestamp,
        elements: vec![local.clone()],
    });
    one(
        &mut db,
        "INSERT INTO d VALUES (1, $1, $2, $3, $4)",
        vec![local.clone(), local.clone(), local, arr],
    );
    assert_eq!(
        text(
            &mut db,
            "SELECT t::text || ' ' || tz::text || ' ' || d::text || ' ' || ts::text FROM d"
        ),
        "0001-01-01 03:00:00 0001-01-01 03:00:00+00 0001-01-01 {\"0001-01-01 03:00:00\"}"
    );
    let dump = db.dump_sql().expect("the stored values are in the domain");
    assert!(!dump.contains("0000-"), "no year 0 in the dump:\n{dump}");
}

/// A store can already hold a value outside the domain — one a build without it wrote —
/// and the dump refuses to export it rather than emit a `COPY` line 18.6 refuses, whose
/// restore loses the table while `psql -f` exits `0`. The value is put into storage
/// directly, standing in for such a store, and is judged in a `date`, a `timestamp`, a
/// `timestamptz`, an array element, a `timestamp` whose fields at its own offset render
/// year 0 while its UTC reading is year 1, and the reverse — in the domain at its own
/// offset and out of it in UTC. Each of those answers the range refusal, which is what
/// separates the two halves from the offset refusal below. What the same store's reads
/// and writes do is pinned beside it: the value renders as stored, a cast or a write that
/// would build a new row from it is refused, and deleting the row makes the dump succeed.
#[test]
fn a_stored_value_outside_the_domain_refuses_the_dump() {
    use pglet_core::Storage;
    use pglet_sql::types::{ArrayValue, TypeOid, Value};
    use time::macros::{date, datetime};
    let year_zero = |column: usize, value: Value| {
        let db = Database::with_btree();
        let mut db = db;
        db.execute("CREATE TABLE d (id INT, d DATE, t TIMESTAMP, tz TIMESTAMPTZ, ds DATE[])")
            .expect("create");
        db.execute("INSERT INTO d (id) VALUES (1)")
            .expect("a clean row first");
        let (schema, mut storage) = db.into_parts();
        let mut row = vec![
            Value::Int4(2),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        ];
        row[column] = value;
        storage
            .insert("d", row)
            .expect("storage holds what it is handed");
        Database::with_storage(storage, schema)
    };
    let date_zero = Value::date(date!(0000 - 12 - 31));
    for (what, column, value, sentence) in [
        ("date", 1, date_zero.clone(), "date out of range"),
        (
            "timestamp",
            2,
            Value::timestamp(datetime!(0000-12-31 23:59:59 UTC)),
            "timestamp out of range",
        ),
        (
            "timestamptz",
            3,
            Value::timestamp(datetime!(0000-12-31 00:00:00 UTC)),
            "timestamp out of range",
        ),
        (
            "an array element",
            4,
            Value::Array(ArrayValue {
                element_type: TypeOid::Date,
                elements: vec![Value::date(date!(2000 - 01 - 01)), date_zero.clone()],
            }),
            "date out of range",
        ),
        (
            "fields at a non-UTC offset",
            2,
            Value::timestamp(datetime!(0000-12-31 22:00:00 -5)),
            "timestamp out of range",
        ),
        (
            "an instant in UTC at a non-UTC offset",
            3,
            Value::timestamp(datetime!(0001-01-01 03:00:00 +5)),
            "timestamp out of range",
        ),
    ] {
        let db = year_zero(column, value);
        let e = db.dump_sql().expect_err(what);
        assert_eq!(
            (e.sqlstate(), e.to_string().as_str()),
            ("22008", sentence),
            "{what}"
        );
    }

    let mut db = year_zero(1, date_zero);
    assert_eq!(
        text(&mut db, "SELECT d::text FROM d WHERE id = 2"),
        "0000-12-31"
    );
    for sql in [
        "SELECT d::timestamp FROM d WHERE id = 2",
        "INSERT INTO d (id, d) SELECT 3, d FROM d WHERE id = 2",
        "UPDATE d SET d = d WHERE id = 2",
    ] {
        assert_eq!(refusal(&mut db, sql), Some(date_out_of_range()), "{sql}");
    }
    // An `UPDATE` that does not assign the column rewrites the row with the stored value
    // as it is: the finalizer runs over the assigned columns only.
    db.execute("UPDATE d SET id = 4 WHERE id = 2")
        .expect("an unassigned column does not reach the finalizer");
    assert!(db.dump_sql().is_err());
    db.execute("DELETE FROM d WHERE id = 4").expect("delete");
    assert!(
        db.dump_sql()
            .expect("the store is in the domain again")
            .contains("COPY d ")
    );
}

/// A store written through `execute_bound` before that door normalised can hold an
/// in-domain `timestamp` at another offset. Its text renders the fields at that offset
/// with `+00`, which reloads as a different instant — `2026-06-15 22:00:00-05` as
/// `2026-06-15 22:00:00+00`, five hours off — so the dump refuses it, in a scalar column
/// and in an array alike. A write that stores such a value — `INSERT … SELECT` here —
/// stores it at UTC, which is what the dump then exports.
#[test]
fn a_stored_timestamp_at_another_offset_refuses_the_dump_and_a_write_stores_it_at_utc() {
    use pglet_core::Storage;
    use pglet_sql::types::{ArrayValue, TypeOid, Value};
    use time::macros::datetime;
    let shifted = datetime!(2026-06-15 22:00:00 -5);
    for (what, column, value) in [
        ("timestamptz", 1, Value::timestamp(shifted)),
        (
            "a timestamp[] element",
            2,
            Value::Array(ArrayValue {
                element_type: TypeOid::Timestamp,
                elements: vec![Value::timestamp(shifted)],
            }),
        ),
    ] {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE n (id INT, tz TIMESTAMPTZ, ts TIMESTAMP[])")
            .expect("create");
        let (schema, mut storage) = db.into_parts();
        let mut row = vec![Value::Int4(1), Value::Null, Value::Null];
        row[column] = value;
        storage
            .insert("n", row)
            .expect("storage holds what it is handed");
        let mut db = Database::with_storage(storage, schema);

        let e = db.dump_sql().expect_err(what);
        assert_eq!(e.sqlstate(), "22000", "{what}");
        assert_eq!(
            e.to_string(),
            "execution error: cannot dump a timestamp stored at UTC offset -05:00:00: \
             its text would reload as a different instant",
            "{what}"
        );

        db.execute("INSERT INTO n SELECT 2, tz, ts FROM n WHERE id = 1")
            .expect("the copy is in the domain");
        db.execute("DELETE FROM n WHERE id = 1").expect("delete");
        assert_eq!(
            text(&mut db, "SELECT coalesce(tz::text, ts::text) FROM n"),
            if column == 1 {
                "2026-06-16 03:00:00+00"
            } else {
                "{\"2026-06-16 03:00:00\"}"
            },
            "{what}: the write stored the instant at UTC"
        );
        let dump = db.dump_sql().expect("the copy dumps");
        assert!(dump.contains("2026-06-16 03:00:00"), "{what}:\n{dump}");
    }
}

/// The dump asks each stored value two things in one row walk, and the jsonb verdict
/// anywhere in a table answers ahead of a temporal one anywhere in it — the order the
/// two walks had before they were folded into one. The year-0 row is scanned first, so
/// a walk that answered its first failure of either kind would report the date.
#[test]
fn a_jsonb_refusal_answers_ahead_of_a_temporal_one_in_the_dump() {
    use pglet_core::Storage;
    use pglet_sql::types::{JsonbValue, Value};
    use time::macros::date;
    let db = Database::with_btree();
    let mut db = db;
    db.execute("CREATE TABLE m (id INT, d DATE, j JSONB)")
        .expect("create");
    let (schema, mut storage) = db.into_parts();
    storage
        .insert(
            "m",
            vec![
                Value::Int4(1),
                Value::date(date!(0000 - 12 - 31)),
                Value::Null,
            ],
        )
        .expect("insert");
    storage
        .insert(
            "m",
            vec![
                Value::Int4(2),
                Value::Null,
                Value::Jsonb(JsonbValue::String("x\0y".to_string())),
            ],
        )
        .expect("insert");
    let db = Database::with_storage(storage, schema);
    assert_eq!(db.dump_sql().expect_err("refused").sqlstate(), "22021");
}
