//! In-process `COPY ... TO STDOUT` / `COPY ... FROM STDIN` (text format) over the
//! embed engine: round-trip fidelity, exact wire bytes, the fail-loud SQLSTATE
//! boundaries, and a dep-free no-panic fuzz over untrusted COPY-FROM input.
//!
//! COPY's byte stream rides outside the `QueryResult` channel, so it is driven
//! through `Database::copy_in`/`copy_out` (not `execute`); `execute("COPY …")`
//! itself fails loud, directing callers here.

// A test harness: setup/read helpers `.unwrap()` so a failure surfaces loudly right here.
#![allow(clippy::unwrap_used)]

use pglet::Database;
use pglet_sql::executor::copy_unescape_field;
use pglet_sql::types::Value;
use pglet_testing::rng::Xorshift;

/// The wide schema the round-trip + exact-bytes tests share: every text-real type
/// plus a NULL and a text value carrying a literal tab, newline, and backslash.
const WIDE_DDL: &str = "CREATE TABLE w (
    id INT,
    name TEXT,
    flag BOOL,
    tags TEXT[],
    ident UUID,
    ts TIMESTAMPTZ,
    data BYTEA,
    note TEXT
)";

const WIDE_SEED: &str = "INSERT INTO w (id, name, flag, tags, ident, ts, data, note) VALUES
    (1, 'alice', true, ARRAY['a','b c'], '00000000-0000-0000-0000-000000000001',
     '2024-06-15 12:30:45.123+00'::timestamptz, '\\xdeadbeef'::bytea, E'tab\\tnl\\nbs\\\\end'),
    (2, NULL, false, ARRAY[]::text[], '00000000-0000-0000-0000-000000000002',
     '2024-01-02 03:04:05+00'::timestamptz, '\\x'::bytea, NULL)";

fn select_all(db: &mut Database) -> Vec<Vec<Value>> {
    let mut res = db.execute("SELECT * FROM w ORDER BY id").unwrap();
    res.remove(0).rows
}

/// COPY a wide table out, COPY the bytes into a fresh identical table, and assert the
/// two tables' `SELECT *` match — every text-real type plus NULL and a control-char
/// text value round-trip byte-for-byte through the text codec.
#[test]
fn round_trip_wide_table() {
    let mut db = Database::in_memory();
    db.execute(WIDE_DDL).unwrap();
    db.execute(WIDE_SEED).unwrap();
    let before = select_all(&mut db);

    let bytes = db.copy_out("COPY w TO STDOUT").unwrap();

    // Re-ingest into a fresh, identically-shaped table.
    db.execute("DROP TABLE w").unwrap();
    db.execute(WIDE_DDL).unwrap();
    let n = db.copy_in("COPY w FROM STDIN", &bytes).unwrap();
    assert_eq!(n, 2, "two rows ingested");

    let after = select_all(&mut db);
    assert_eq!(
        before, after,
        "round-trip must preserve every value exactly"
    );
}

/// The exact COPY-text bytes of a known small table, asserted against the verified
/// Postgres framing: tab separators, `\n` row terminator, `t`/`f` bools, the canonical
/// array text, bytea's **doubled** leading backslash (`\\xdeadbeef`), and the raw `\N`
/// null marker. (Verified against live Postgres 18 `COPY ... TO STDOUT`.)
#[test]
fn exact_bytes_to_stdout() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE e (id INT, name TEXT, flag BOOL, tags TEXT[], data BYTEA, note TEXT)")
        .unwrap();
    db.execute(
        "INSERT INTO e VALUES (1, 'alice', true, ARRAY['a','b c'], '\\xdeadbeef'::bytea, NULL)",
    )
    .unwrap();

    let bytes = db.copy_out("COPY e TO STDOUT").unwrap();
    // id \t name \t flag \t {a,"b c"} \t \\xdeadbeef \t \N \n
    let expected = "1\talice\tt\t{a,\"b c\"}\t\\\\xdeadbeef\t\\N\n";
    assert_eq!(
        bytes,
        expected.as_bytes(),
        "got {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// A text value carrying a literal tab, newline, and backslash renders with the
/// COPY output escapes (`\t`, `\n`, `\\`), exact-bytes. (Verified against live PG.)
#[test]
fn exact_bytes_escapes_control_chars() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE c (id INT, v TEXT)").unwrap();
    db.execute("INSERT INTO c VALUES (2, E'a\\tb\\nc\\\\d')")
        .unwrap();

    let bytes = db.copy_out("COPY c TO STDOUT").unwrap();
    // 2 \t a\tb\nc\\d \n   (the value's tab/newline/backslash each escaped)
    let expected = "2\ta\\tb\\nc\\\\d\n";
    assert_eq!(
        bytes,
        expected.as_bytes(),
        "got {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// HEADER on TO emits a column-name line; on FROM it skips the first line. Round-trip
/// with a header preserves the rows.
#[test]
fn header_round_trip() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE h (id INT, name TEXT)").unwrap();
    db.execute("INSERT INTO h VALUES (1, 'a'), (2, 'b')")
        .unwrap();

    let bytes = db.copy_out("COPY h TO STDOUT WITH (HEADER true)").unwrap();
    assert!(
        bytes.starts_with(b"id\tname\n"),
        "header line present: {:?}",
        String::from_utf8_lossy(&bytes)
    );

    db.execute("DELETE FROM h").unwrap();
    let n = db
        .copy_in("COPY h FROM STDIN WITH (HEADER true)", &bytes)
        .unwrap();
    assert_eq!(n, 2);
    let rows = select_named(&mut db, "h");
    assert_eq!(
        rows,
        vec![
            vec![Value::Int4(1), Value::Text("a".into())],
            vec![Value::Int4(2), Value::Text("b".into())],
        ]
    );
}

/// A COPY column list ingests a subset of columns; the omitted ones take their
/// DEFAULT.
#[test]
fn from_with_column_list_fills_defaults() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE p (id INT, name TEXT, note TEXT DEFAULT 'def')")
        .unwrap();
    let n = db
        .copy_in("COPY p (id, name) FROM STDIN", b"1\talice\n2\tbob\n")
        .unwrap();
    assert_eq!(n, 2);
    let rows = select_named(&mut db, "p");
    assert_eq!(
        rows,
        vec![
            vec![
                Value::Int4(1),
                Value::Text("alice".into()),
                Value::Text("def".into())
            ],
            vec![
                Value::Int4(2),
                Value::Text("bob".into()),
                Value::Text("def".into())
            ],
        ]
    );
}

/// A lone `\.` line ends ingestion; data after it is ignored.
#[test]
fn end_of_data_marker_stops_ingest() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE d (id INT)").unwrap();
    let n = db
        .copy_in("COPY d FROM STDIN", b"1\n2\n\\.\n3\n4\n")
        .unwrap();
    assert_eq!(n, 2, "only rows before the \\. marker are ingested");
}

/// `Database::execute("COPY …")` fails loud (no byte channel), directing callers to
/// copy_in/copy_out rather than silently producing nothing.
#[test]
fn execute_copy_fails_loud() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE x (id INT)").unwrap();
    let err = db.execute("COPY x TO STDOUT").unwrap_err();
    assert_eq!(err.sqlstate(), "0A000", "{err}");
    let err = db.execute("COPY x FROM STDIN").unwrap_err();
    assert_eq!(err.sqlstate(), "0A000", "{err}");
}

// ----- error-path SQLSTATEs (verified vs Postgres) -----

fn from_err(ddl: &str, copy_sql: &str, data: &[u8]) -> pglet::Error {
    let mut db = Database::in_memory();
    db.execute(ddl).unwrap();
    db.copy_in(copy_sql, data).unwrap_err()
}

#[test]
fn missing_data_is_22p04() {
    // Two columns, a one-field row → missing data for the second column.
    let err = from_err("CREATE TABLE t (a INT, b INT)", "COPY t FROM STDIN", b"1\n");
    assert_eq!(err.sqlstate(), "22P04", "{err}");
}

#[test]
fn extra_data_is_22p04() {
    let err = from_err(
        "CREATE TABLE t (a INT, b INT)",
        "COPY t FROM STDIN",
        b"1\t2\t3\n",
    );
    assert_eq!(err.sqlstate(), "22P04", "{err}");
}

#[test]
fn bad_value_is_22p02() {
    let err = from_err("CREATE TABLE t (a INT)", "COPY t FROM STDIN", b"notanint\n");
    assert_eq!(err.sqlstate(), "22P02", "{err}");
}

#[test]
fn null_into_not_null_is_23502() {
    let err = from_err(
        "CREATE TABLE t (a INT NOT NULL)",
        "COPY t FROM STDIN",
        b"\\N\n",
    );
    assert_eq!(err.sqlstate(), "23502", "{err}");
}

#[test]
fn corrupt_end_marker_is_22p04() {
    let err = from_err("CREATE TABLE t (a INT)", "COPY t FROM STDIN", b"\\.oops\n");
    assert_eq!(err.sqlstate(), "22P04", "{err}");
    // 18.6's sentence for `\.x`, measured over the wire; the harness compares only the
    // SQLSTATE, so the message is pinned here.
    assert_eq!(
        err.to_string(),
        "end-of-copy marker is not alone on its line"
    );
}

#[test]
fn unknown_option_is_42601() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE t (a INT)").unwrap();
    // Unknown option in the parenthesized form is a parse error (42601).
    let err = db
        .copy_in("COPY t FROM STDIN (BOGUS 'x')", b"")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "42601", "{err}");
}

#[test]
fn csv_format_is_0a000() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE t (a INT)").unwrap();
    let err = db
        .copy_in("COPY t FROM STDIN (FORMAT csv)", b"1\n")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "0A000", "{err}");
    let err = db.copy_out("COPY t TO STDOUT (FORMAT binary)").unwrap_err();
    assert_eq!(err.sqlstate(), "0A000", "{err}");
}

#[test]
fn bad_delimiter_is_0a000() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE t (a INT)").unwrap();
    let err = db
        .copy_in("COPY t FROM STDIN (DELIMITER '||')", b"1\n")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "0A000", "{err}");
}

// ----- dep-free no-panic fuzz (COPY-FROM ingests untrusted input) -----

/// Build a pseudo-random byte buffer: a mix of raw noise, valid-ish rows, embedded
/// escapes (incl. truncated trailing `\`), `\N` / `\.` markers, delimiters, and huge
/// field counts — the shapes that must never panic `copy_unescape_field` /
/// `execute_copy_from`.
fn fuzz_buf(rng: &mut Xorshift) -> Vec<u8> {
    // A small alphabet weighted toward the bytes that matter for COPY framing.
    const ALPHABET: &[u8] = b"\\.Nntr0123456789xX\t\n\r| ab{}\",";
    let len = (rng.below(140) + 1) as usize;
    let mut buf = Vec::with_capacity(len);
    for _ in 0..len {
        let pick = rng.below(10);
        if pick < 7 {
            // Mostly the COPY-significant alphabet.
            let idx = rng.below(ALPHABET.len() as u64) as usize;
            buf.push(ALPHABET[idx]);
        } else {
            // Occasionally a fully-random byte (incl. high bytes / NUL).
            buf.push((rng.below(256)) as u8);
        }
    }
    buf
}

/// Thousands of random/structured buffers thrown at the COPY-FROM ingest path and the
/// field unescaper must **never panic** — every outcome is a clean `Ok` or `Err`.
/// This is the untrusted-bulk-input hardening bar.
#[test]
fn fuzz_copy_from_never_panics() {
    const ITERATIONS: usize = 20_000;
    let seed = std::env::var("PGLET_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0xC0FF_EE15_900D_u64);
    let mut rng = Xorshift::new(seed);

    for _ in 0..ITERATIONS {
        let buf = fuzz_buf(&mut rng);

        // (1) The field unescaper directly, over the raw bytes — any of them.
        let _ = copy_unescape_field(&buf);

        // (2) The full ingest over a simple seeded table — random buffer as the data
        // stream. A fresh DB per buffer keeps each independent.
        let mut db = Database::in_memory();
        db.execute("CREATE TABLE f (a TEXT, b INT)").unwrap();
        // Every outcome must be Ok or a clean Err (the assert is "did not panic").
        let _ = db.copy_in("COPY f FROM STDIN", &buf);

        // (3) A single-column table exercises the no-delimiter / huge-field paths.
        let mut db1 = Database::in_memory();
        db1.execute("CREATE TABLE g (a TEXT)").unwrap();
        let _ = db1.copy_in("COPY g FROM STDIN", &buf);
    }
}

/// Read a named table's rows ordered by the first column.
fn select_named(db: &mut Database, table: &str) -> Vec<Vec<Value>> {
    let sql = format!("SELECT * FROM {table} ORDER BY 1");
    let mut res = db.execute(&sql).unwrap();
    res.remove(0).rows
}

// ----- the column list is a set (verified vs Postgres) -----

/// A repeated column in a `COPY … FROM STDIN` list is `42701`, and **no row lands**.
///
/// The `TO STDOUT` direction of the rule is pinned by differential fixtures
/// (`tests/fixtures/copy/duplicate_column_*`); this direction cannot be, because the
/// refusal arrives *before* the `CopyInResponse` and the harness's client leaves the
/// connection unusable when it does — against the real Postgres oracle as much as
/// against pglet, so it is the client's limit and not a divergence. The state
/// assertion is the load-bearing half either way: pglet used to accept the list and
/// ingest a row under it.
#[test]
fn from_duplicate_column_is_42701_and_writes_nothing() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE cd (a INT, b INT, c INT)").unwrap();
    let err = db
        .copy_in("COPY cd (a, a) FROM STDIN", b"1\t2\n")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "42701", "{err}");
    assert!(
        select_named(&mut db, "cd").is_empty(),
        "a refused COPY must leave the table empty"
    );
}

/// The same in the `TO STDOUT` direction, in process — `CopyGetAttnums` serves both,
/// and so does the one resolver here.
#[test]
fn to_duplicate_column_is_42701() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE ct (a INT, b INT)").unwrap();
    db.execute("INSERT INTO ct VALUES (1, 2)").unwrap();
    let err = db.copy_out("COPY ct (a, a) TO STDOUT").unwrap_err();
    assert_eq!(err.sqlstate(), "42701", "{err}");
}

/// The interleave, in the direction the fixtures cannot reach: a repeat at item 2
/// outranks an unknown name at item 3, and an unknown name at item 1 outranks the
/// repeat behind it. `CopyGetAttnums` resolves one name and tests it against the
/// attributes it has collected before reading the next.
#[test]
fn from_column_list_verdicts_interleave() {
    let mut db = Database::in_memory();
    db.execute("CREATE TABLE ci (a INT, b INT)").unwrap();

    let dup_first = db
        .copy_in("COPY ci (a, a, zz) FROM STDIN", b"1\t2\t3\n")
        .unwrap_err();
    assert_eq!(dup_first.sqlstate(), "42701", "{dup_first}");

    let unknown_first = db
        .copy_in("COPY ci (zz, a, a) FROM STDIN", b"1\t2\t3\n")
        .unwrap_err();
    assert_eq!(unknown_first.sqlstate(), "42703", "{unknown_first}");

    assert!(
        select_named(&mut db, "ci").is_empty(),
        "neither refusal may write"
    );
}

/// A name the relation lacks, written twice, is the unknown name: the claim is keyed
/// on the resolved attribute, and this one never resolves.
#[test]
fn from_unknown_column_twice_is_42703() {
    let err = from_err(
        "CREATE TABLE cu (a INT, b INT)",
        "COPY cu (zz, zz) FROM STDIN",
        b"1\t2\n",
    );
    assert_eq!(err.sqlstate(), "42703", "{err}");
}

// ----- the server encoding: raw input and de-escaped fields (verified vs 18.6) -----

/// The `22021` sentence for `bytes`, as 18.6 renders it.
fn encoding_message(bytes: &str) -> String {
    format!("invalid byte sequence for encoding \"UTF8\": {bytes}")
}

/// Assert a text-column `COPY` refuses `data` with `22021` naming exactly `bytes`.
fn assert_encoding_refusal(ddl: &str, copy_sql: &str, data: &[u8], bytes: &str) {
    let err = from_err(ddl, copy_sql, data);
    assert_eq!(err.sqlstate(), "22021", "{data:?}: {err}");
    assert_eq!(err.to_string(), encoding_message(bytes), "{data:?}");
}

const TEXT_DDL: &str = "CREATE TABLE t (a TEXT)";
const COPY_T: &str = "COPY t FROM STDIN";

#[test]
fn a_nul_is_refused_on_the_raw_route_and_the_escape_route() {
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\0y\n", "0x00");
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\\000y\n", "0x00");
}

#[test]
fn the_raw_route_cuts_its_byte_list_from_the_whole_input_and_the_escape_route_from_the_field() {
    // `min(pg_utf_mblen(0xe9) = 3, remaining)`: the raw check verifies the input buffer,
    // so the line terminator — and the next line — are in what remains; the escape check
    // verifies the de-escaped field, which ends at the `y`.
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\xe9y\n", "0xe9 0x79 0x0a");
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\xe9\ny\n", "0xe9 0x0a 0x79");
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\xe9\r\n", "0xe9 0x0d 0x0a");
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\\xe9y\n", "0xe9 0x79");
    // An incomplete sequence at the very end of the input.
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\xc3", "0xc3");
}

#[test]
fn the_raw_check_runs_ahead_of_the_header_skip_and_the_field_count() {
    // First, a header-only input: with no line after the header, a check placed behind
    // the header skip reaches no line and the copy loads zero rows. The case below it,
    // a header followed by a data line, cannot show that placement on its own once the
    // loop's `debug_assert!` is compiled out, since the check then fires on the data
    // line with the same byte list.
    assert_encoding_refusal(
        TEXT_DDL,
        "COPY t FROM STDIN (HEADER true)",
        b"h\0d\n",
        "0x00",
    );
    assert_encoding_refusal(
        TEXT_DDL,
        "COPY t FROM STDIN (HEADER true)",
        b"a\xe9\nok\n",
        "0xe9 0x0a 0x6f",
    );
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\xe9\ty\tz\n", "0xe9 0x09 0x79");
}

#[test]
fn every_field_is_validated_before_the_field_count_and_before_any_value_is_typed() {
    // An escaped NUL outranks `extra data after last expected column` (`22P04`) — in
    // the first field and in the extra one — and a bad value in an *earlier* column.
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\\000\ty\tz\n", "0x00");
    assert_encoding_refusal(TEXT_DDL, COPY_T, b"x\ty\tz\\000\n", "0x00");
    // ...and `missing data for column`, the other count verdict.
    assert_encoding_refusal(
        "CREATE TABLE t (a INT, b TEXT)",
        COPY_T,
        b"x\\000\n",
        "0x00",
    );
    assert_encoding_refusal(
        "CREATE TABLE t (a INT, b TEXT)",
        COPY_T,
        b"notint\tx\\000\n",
        "0x00",
    );
}

#[test]
fn a_failure_on_a_later_line_waits_for_that_line() {
    // Line 1's bad value is reached first; the raw failure on line 2 is not raised early.
    let err = from_err("CREATE TABLE t (a INT)", COPY_T, b"notint\n1\xe9\n");
    assert_eq!(err.sqlstate(), "22P02", "{err}");
}

#[test]
fn what_the_encoding_checks_never_read_is_accepted() {
    let mut db = Database::in_memory();
    db.execute(TEXT_DDL).unwrap();
    // An illegal byte after the end-of-data marker.
    assert_eq!(db.copy_in(COPY_T, b"ok\n\\.\nx\xe9y\n").unwrap(), 1);
    // An escaped NUL on a header line, which is skipped without being de-escaped.
    assert_eq!(
        db.copy_in("COPY t FROM STDIN (HEADER true)", b"a\\000\nok\n")
            .unwrap(),
        1
    );
    // The accept side of both routes: a valid multi-byte character, raw and escaped.
    assert_eq!(db.copy_in(COPY_T, "x\u{e9}y\n".as_bytes()).unwrap(), 1);
    assert_eq!(db.copy_in(COPY_T, b"x\\303\\251y\n").unwrap(), 1);
    let rows = db
        .execute("SELECT a FROM t ORDER BY a")
        .unwrap()
        .remove(0)
        .rows;
    assert_eq!(
        rows,
        vec![
            vec![Value::Text("ok".into())],
            vec![Value::Text("ok".into())],
            vec![Value::Text("x\u{e9}y".into())],
            vec![Value::Text("x\u{e9}y".into())],
        ]
    );
}
