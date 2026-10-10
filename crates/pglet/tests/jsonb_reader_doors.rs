//! The jsonb reader's raw-NUL arm, at the doors that can deliver one.
//!
//! A NUL cannot reach the reader over the wire — a simple `Query` carries its SQL as a
//! NUL-terminated string, a bound `$N` and a COPY field go through the server-encoding
//! scan (`docs/encoding.md` § The doors that reach it), and each answers before the
//! value's input function runs. The doors measured to carry one to the reader are the
//! in-process ones, which take their SQL as a Rust `&str` (`EDGES.md` row 133) — so no
//! fixture can issue one, the harness reaching pglet over the wire, and the guards for
//! this arm are this file and `json.rs`'s own unit tests.
//!
//! **What the arm is.** `0x01`–`0x1F` raw inside a JSON string is
//! `22P02 invalid input syntax for type json`, which is 18.6's own answer
//! (`json_lex_string`'s `JSON_ESCAPING_REQUIRED`). A raw `0x00` is
//! `22021 invalid byte sequence for encoding "UTF8": 0x00` instead, which is not 18.6's
//! JSON lexer's answer but is the answer both engines were measured to give a NUL in
//! jsonb text at the routes measured — a text-format `$N` and a `COPY` field,
//! where 18.6 raises it from its encoding check (row 136's bullet). `docs/encoding.md`'s
//! rule is that text entering the engine answers `22021` wherever it enters, so the
//! reader answers the code a measured pair agrees on rather than minting `22P02` for it.
//!
//! **What refusing here bought, and what it did not.** Without this arm a NUL in any of
//! the four positions below decodes into the jsonb value and is stored, and the dump then
//! spells it `\u0000` — which `psql -f` loads into neither engine, so the table restores
//! with no rows at exit `0` (row 133). Closing the **reader** closes every route whose NUL
//! arrives as jsonb *text*, `parse_jsonb` being the one reader of it. It does not close
//! the routes that never read text at all, and those are not one door's: **a
//! `Value::Text` holding a NUL becomes a jsonb string where a jsonb-producing expression
//! *wraps* it rather than parsing it as jsonb text** — `jsonb_build_object` here, which
//! wraps a value ordinary SQL through `Database::execute` stored as readily as a bound
//! one, while the `::jsonb` cast and the `->` family's text-left arms parse and so answer
//! the reader. Producing a jsonb is not the predicate: `->` makes one from text too, and
//! refuses. A bound `Value::Jsonb` is such a string already.
//! `a_jsonb_expression_that_wraps_a_stored_text_reaches_the_nul_one_that_parses_does_not`
//! measures both sides rather than leaving the negative claim in prose, and pins what the
//! **dump** does with the result: `pglet::dump`'s `refuse_unreadable_values` refuses the
//! export (`22021`), the renderer's escape having hidden the NUL from the COPY payload's
//! own encoding gate.

// A test harness: `.expect()`/`.unwrap()` surface a setup failure loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use pglet::{BoundExecError, Database};
use pglet_sql::types::{TypeOid, Value};

/// The four jsonb positions a string can stand in, each spelled with `mark` in it.
fn positions(mark: &str) -> Vec<(&'static str, String)> {
    vec![
        ("a scalar string", format!("\"a{mark}b\"")),
        ("an object value", format!("{{\"k\": \"a{mark}b\"}}")),
        ("an object key", format!("{{\"a{mark}b\": 1}}")),
        ("an array element", format!("[\"a{mark}b\"]")),
    ]
}

/// The SQLSTATE `sql` fails with through `Database::execute`, or `None` when it succeeds.
fn execute_state(db: &mut Database, sql: &str) -> Option<&'static str> {
    db.execute(sql).err().map(|e| e.sqlstate())
}

/// The SQLSTATE `sql` fails with through `Database::execute_bound` with no parameters.
fn bound_sql_state(db: &mut Database, sql: &str) -> Option<&'static str> {
    match db.execute_bound(sql, 1, |_: &[Option<TypeOid>]| {
        Ok::<_, std::convert::Infallible>(Vec::new())
    }) {
        Ok(_) => None,
        Err(BoundExecError::Engine(e)) => Some(e.sqlstate()),
        Err(BoundExecError::Bind(e)) => match e {},
    }
}

/// The SQLSTATE a `Value::Text` bound as `$1` and cast to jsonb fails with.
fn bound_value_state(db: &mut Database, json: &str) -> Option<&'static str> {
    let text = json.to_string();
    match db.execute_bound("SELECT $1::jsonb", 1, move |_: &[Option<TypeOid>]| {
        Ok::<_, std::convert::Infallible>(vec![Some(Value::Text(text))])
    }) {
        Ok(_) => None,
        Err(BoundExecError::Engine(e)) => Some(e.sqlstate()),
        Err(BoundExecError::Bind(e)) => match e {},
    }
}

/// A raw NUL inside a jsonb string is `22021` at every in-process door and position.
///
/// The wasm hosts' `exec` / `query` / `queryParams` are not a fifth door: every binding in
/// `pglet_wasm` runs its statement through `Database::execute_bound`, driven here.
#[test]
fn a_raw_nul_in_a_jsonb_string_is_refused_at_every_in_process_door() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE j (id INT, v JSONB)").unwrap();

    for (what, json) in positions("\0") {
        let select = format!("SELECT '{json}'::jsonb");
        assert_eq!(
            execute_state(&mut db, &select),
            Some("22021"),
            "execute, {what}"
        );
        assert_eq!(
            bound_sql_state(&mut db, &select),
            Some("22021"),
            "execute_bound's SQL, {what}"
        );
        assert_eq!(
            bound_value_state(&mut db, &json),
            Some("22021"),
            "a Value::Text bound through execute_bound, {what}"
        );
        // The write route, not only the read one: nothing may store the NUL.
        let insert = format!("INSERT INTO j VALUES (1, '{json}')");
        assert_eq!(
            execute_state(&mut db, &insert),
            Some("22021"),
            "INSERT through execute, {what}"
        );
    }

    // Nothing was written, so the store holds no NUL to dump.
    let counted = db
        .execute("SELECT count(*) FROM j")
        .unwrap()
        .pop()
        .expect("a result");
    assert_eq!(counted.rows, vec![vec![Value::Int8(0)]]);
}

/// A raw control character other than NUL is `22P02` at the same doors — 18.6's code.
#[test]
fn a_raw_control_character_in_a_jsonb_string_is_refused_as_18_6_refuses_it() {
    let mut db = Database::with_btree();
    // `0x01` and `0x1F` are the ends of the range; tab and LF have short escapes.
    for mark in ["\u{01}", "\t", "\n", "\u{1f}"] {
        for (what, json) in positions(mark) {
            let select = format!("SELECT '{json}'::jsonb");
            assert_eq!(
                execute_state(&mut db, &select),
                Some("22P02"),
                "{what}, {mark:?}"
            );
        }
    }
    // The accept side. `0x7F` is above the `31` 18.6's lexer tests and is taken by both
    // engines; the escaped spellings of the refused characters still decode.
    for (what, json) in positions("\u{7f}") {
        let select = format!("SELECT '{json}'::jsonb");
        assert_eq!(execute_state(&mut db, &select), None, "0x7F at {what}");
    }
    for (what, json) in positions("\\t") {
        let select = format!("SELECT '{json}'::jsonb");
        assert_eq!(
            execute_state(&mut db, &select),
            None,
            "\\t escape at {what}"
        );
    }
}

/// Which of a raw control character and a `\u0000` escape raises is decided by position.
///
/// 18.6's `json_lex_string` walks the token once, so the first of the two to be reached
/// is the one that answers; measured on 18.6 for both orders. A gate run over the string
/// body ahead of the escape decoding would answer `22P02` for both and break the order
/// the two engines already agreed on.
#[test]
fn the_first_of_a_raw_control_and_a_code_point_zero_escape_decides() {
    let mut db = Database::with_btree();
    assert_eq!(
        execute_state(&mut db, "SELECT '\"\\u0000a\tb\"'::jsonb"),
        Some("22P05"),
        "the escape stands first"
    );
    assert_eq!(
        execute_state(&mut db, "SELECT '\"a\tb\\u0000\"'::jsonb"),
        Some("22P02"),
        "the raw character stands first"
    );
}

/// The routes the reader gate does **not** close, what separates them from the ones it
/// does, and the door that is not their bound.
///
/// `EDGES.md` row 133 records this shape as the dump invariant's property-B breach for
/// the rows — the store holds a NUL inside a jsonb string. Three facts are pinned here
/// rather than carried in prose. **Which routes**: a jsonb-producing expression that
/// *wraps* a `Value::Text` instead of parsing it as jsonb text, which `Database::execute`
/// feeds with no bound `Value` anywhere (the NUL arriving as SQL text, row 133's own
/// door), as well as `execute_bound`; and a bound `Value::Jsonb`, a jsonb string already.
/// **What separates them**: the routes measured here that parse the same stored text hand
/// it to `parse_jsonb` and so meet the reader — the `::jsonb` cast and the text-left arms
/// of `->`, `->>`, `#>`, `#>>`, `?`, `?|` and `?&`. Producing a jsonb is not what decides,
/// and the control below shows why: `->` over plain text yields a jsonb made from that
/// text. **What the dump does**: it refuses. The renderer spells the NUL `\u0000`, so the
/// COPY payload holds none for `emit_copy`'s encoding gate to find, and
/// `refuse_unreadable_values` walks the stored
/// value instead. Before that walk the export succeeded and each `psql -f` restore into a
/// fresh 18.6 exited `0` with the table's `COPY` refused `22P05` and `count(*)` `0`.
#[test]
fn a_jsonb_expression_that_wraps_a_stored_text_reaches_the_nul_one_that_parses_does_not() {
    use pglet_sql::types::JsonbValue;

    let mut db = Database::with_btree();
    db.execute("CREATE TABLE j (id INT, v JSONB)").unwrap();
    // `a` is the bare text the wrapping route takes; `jt` spells the same NUL inside
    // jsonb *text*, which is what the parsing routes below are handed — a route that
    // parsed `a` would answer `22P02` at the `x`, which would measure nothing.
    db.execute("CREATE TABLE src (id INT, a TEXT, jt TEXT)")
        .unwrap();

    // The NUL enters as SQL text through `Database::execute` — no bound `Value`.
    db.execute("INSERT INTO src VALUES (1, 'x\0y', '{\"k\": \"x\0y\"}')")
        .expect("a NUL in SQL text through an in-process door is stored (row 133)");

    // The routes that *parse* that stored text meet the reader, so the gate answers and
    // nothing is built. Only the SQLSTATE is read: a route that succeeded would fail the
    // comparison rather than be scored on what it returned, and neither the message nor
    // the returned rows are looked at.
    for (what, sql) in [
        ("the ::jsonb cast", "SELECT jt::jsonb FROM src"),
        ("->", "SELECT jt -> 'k' FROM src"),
        ("->>", "SELECT jt ->> 'k' FROM src"),
        ("#>", "SELECT jt #> '{k}' FROM src"),
        ("#>>", "SELECT jt #>> '{k}' FROM src"),
        ("?", "SELECT jt ? 'k' FROM src"),
        ("?|", "SELECT jt ?| ARRAY['k'] FROM src"),
        ("?&", "SELECT jt ?& ARRAY['k'] FROM src"),
    ] {
        assert_eq!(
            execute_state(&mut db, sql),
            Some("22021"),
            "{what} over src.jt"
        );
    }

    // The control that makes the distinction parse-versus-wrap and not jsonb-versus-not:
    // `->` over plain text *makes* a jsonb, so what it refused above is the parse and not
    // an inability to produce one. Only the value's kind is read, never its content.
    let made = db
        .execute("SELECT ('{\"a\":1}'::text) -> 'a'")
        .expect("-> over plain text")
        .pop()
        .expect("a result")
        .rows;
    assert!(
        matches!(made.first().and_then(|r| r.first()), Some(Value::Jsonb(_))),
        "{made:?}"
    );

    // ...while `jsonb_build_object` wraps rather than parses, so it makes a jsonb string
    // of the NUL, at the value and at the key.
    db.execute("INSERT INTO j SELECT 1, jsonb_build_object('k', a) FROM src")
        .expect("jsonb_build_object's value position, through execute");
    db.execute("INSERT INTO j SELECT 2, jsonb_build_object(a, 1) FROM src")
        .expect("jsonb_build_object's key position, through execute");

    let built = db.execute_bound(
        // Cast: a bare `$1` here is one 18.6 cannot type (`42P18`), and so is refused.
        "INSERT INTO j VALUES (3, jsonb_build_object('k', $1::text))",
        1,
        |_: &[Option<TypeOid>]| {
            Ok::<_, std::convert::Infallible>(vec![Some(Value::Text("x\0y".into()))])
        },
    );
    assert!(built.is_ok(), "jsonb_build_object over a bound Value::Text");

    let direct = db.execute_bound(
        "INSERT INTO j VALUES (4, $1)",
        1,
        |_: &[Option<TypeOid>]| {
            Ok::<_, std::convert::Infallible>(vec![Some(Value::Jsonb(JsonbValue::String(
                "x\0y".into(),
            )))])
        },
    );
    assert!(direct.is_ok(), "a bound Value::Jsonb holding a NUL");

    // Each is in the store, spelled with jsonb's escape for U+0000 where it is read back.
    let rendered = db
        .execute("SELECT v::text FROM j ORDER BY id")
        .unwrap()
        .pop()
        .expect("a result")
        .rows;
    let escaped = format!("x{}u0000y", char::from(92u8));
    assert_eq!(rendered.len(), 4, "{rendered:?}");
    for row in &rendered {
        let Some(Value::Text(text)) = row.first() else {
            panic!("{row:?}")
        };
        assert!(text.contains(&escaped), "{text}");
    }

    // The export is loud rather than short. `src`'s own raw NUL would refuse the export
    // too, at `emit_copy`'s payload gate, so it is dropped first — leaving the jsonb
    // value as the only thing the refusal below can be coming from.
    db.execute("DROP TABLE src").unwrap();
    let err = db
        .dump_sql()
        .expect_err("the dump refuses a stored jsonb NUL");
    assert_eq!(err.sqlstate(), "22021", "{err}");
    assert_eq!(
        err.to_string(),
        "invalid byte sequence for encoding \"UTF8\": 0x00"
    );
}
