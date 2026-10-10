//! `::regclass` decides on the **canonical type name**, and the name it answers is
//! rendered for an identifier position.
//!
//! Two rules, one reader each, and the differential corpus can hold only part of either.
//!
//! **The cast target.** `parser::expr`'s `parse_cast_target` used to peek the identifier
//! `regclass` and discard what `parse_type_name` returned — a reader that also consumes a
//! trailing `[]` (one or more, with an optional dimension) and a `(…)` modifier — so
//! `'t'::regclass[]`, `'t'::regclass(3)` and `CAST('t' AS regclass[])` all built the node
//! `'t'::regclass` builds and **answered a relation name**. In a `WHERE` that wrote:
//! `DELETE FROM dw WHERE 't'::regclass[] = 't'` removed every row where 18.6 raises
//! `22P02` in parse analysis and keeps them.
//!
//! **A fixture cannot hold the headline spellings.** 18.6 raises `22P02` for the array
//! spelling and
//! `42601` for the modifier, pglet raises `42704` (the answer its *column* form already
//! gave for the same type spelling), and `tests/README.md`'s rule is that where Postgres
//! also errors the SQLSTATE must match — so a faithful `@expect-error` fixture for
//! `'t'::regclass[]` would be **red**. It is not that the harness is blind to the shape;
//! it is that the two engines refuse it under different codes. What the corpus *can*
//! hold is `catalog/regclass_array_target_is_loud.sql`, which drives an array
//! literal 18.6 **accepts** over a relation the swallowing reader also resolved, so "any
//! pglet error passes" is a live assertion there rather than a waiver. The SQLSTATE at
//! every spelling, and the rows, are pinned here.
//!
//! **The rendering.** 18.6's `regclassout` (`utils/adt/regproc.c`) ends in
//! `quote_qualified_identifier`, so a relation created as `"Odd"` prints as `"Odd"` —
//! quotes included. Answering the stored bytes gave a bare `Odd`, which is a *different*
//! relation's spelling: fed back to `'…'::regclass` it is `42P01 relation "odd" does not
//! exist` on **both** engines. `catalog/regclass_quotes_the_relation_name.sql` is the
//! differential guard for that; what is here is the polarity list a single fixture would
//! not carry.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface a setup failure loudly right
// here. `clippy::panic` has no test-allowance in this clippy (only unwrap/expect do).
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::common;

use common::open_database;
use pglet::Database;
use pglet_sql::types::Value;
use pglet_testing::block_io::MemBlockIo;

/// The SQLSTATE `sql` fails with, or `None` when it succeeds.
fn state(db: &mut Database, sql: &str) -> Option<&'static str> {
    db.execute(sql).err().map(|e| e.sqlstate())
}

/// The single text value `sql` answers.
fn text(db: &mut Database, sql: &str) -> String {
    let rows = db.execute(sql).expect("the query answers").remove(0).rows;
    match rows.first().and_then(|r| r.first()) {
        Some(Value::Text(s)) => s.clone(),
        other => panic!("expected one text value from `{sql}`, got {other:?}"),
    }
}

/// Seed the relations the probes below name: a bare-spelled table, one needing quotes,
/// and one whose *name* is the array literal `{qt}` — see
/// [`an_array_literal_naming_a_relation_is_refused_too`].
fn seed(db: &mut Database) {
    for sql in [
        "CREATE TABLE qt (a INT)",
        "CREATE TABLE \"Odd\" (a INT)",
        "CREATE TABLE \"{qt}\" (a INT)",
        "CREATE SEQUENCE qs",
    ] {
        db.execute(sql).expect("seeds");
    }
}

/// A cast-target spelling whose canonical name is not the bare word `regclass` no longer
/// builds the `regclass` node; over the list below it is `42704` — the answer
/// `resolve_column_type` already gives the *column* form for the same spelling.
///
/// The `42704` is `resolve_column_type`'s answer rather than a refusal this branch mints,
/// so a canonical name that resolver *does* resolve is not refused here either — the
/// shadowing `json_type_refused.rs` records for `json`, whose guard
/// `an_enum_named_json_still_resolves_to_the_enum` is that class's. A user `ENUM`
/// quoted-named `regclass[]` does not reach it: the resolver decomposes the canonical name
/// into its base and suffix before the enum lookup, so `'a'::regclass[]` is `42704 type
/// "regclass" is not supported` here (measured) against 18.6's `22P02 malformed array
/// literal`, the enum shadowing the builtin on neither engine. Row 118 is the record for
/// what the branch cost and what it did not reach.
///
/// The list is the grammar `Parser::parse_type_name` accepts past the word, not a
/// remembered set of probes: one `[]`, several, a dimensioned one, a one-argument
/// modifier, a two-argument one, a signed one, a modifier **and** a suffix, at both
/// spellings of the cast (`::` and `CAST(… AS …)`), over both argument arms (a name and
/// an OID), and in upper case so the fold is exercised. 18.6 refuses each of them too and
/// under three different codes — `22P02` where the value fails to parse as an array,
/// `42601` for the modifier, `42846` for `1::regclass[]` — which is why this is a
/// SQLSTATE assertion here rather than a fixture.
#[test]
fn a_non_bare_regclass_cast_target_is_refused() {
    let mut db = Database::with_btree();
    seed(&mut db);

    let mut wrong = Vec::new();
    for sql in [
        "SELECT 'qt'::regclass[]",
        "SELECT 'qt'::regclass[][]",
        "SELECT 'qt'::regclass[3]",
        "SELECT 'qt'::regclass[3][4]",
        "SELECT 'qt'::regclass(3)",
        "SELECT 'qt'::regclass(5,2)",
        "SELECT 'qt'::regclass(-2)",
        "SELECT 'qt'::regclass(3)[]",
        "SELECT CAST('qt' AS regclass[])",
        "SELECT CAST('qt' AS regclass(3))",
        "SELECT 'qt'::REGCLASS[]",
        "SELECT 1::regclass[]",
        // The array spelling in the position that made it a data-loss edge rather than a
        // wrong answer. `an_array_cast_target_does_not_delete` asserts the rows; this
        // asserts that the refusal is the same one, at the same code, in a DML clause.
        "DELETE FROM qt WHERE 'qt'::regclass[] = 'qt'",
    ] {
        // Collected rather than asserted per iteration: a `?`-style stop reports only the
        // first spelling, leaving every one after it unproven by the run that fails.
        let observed = state(&mut db, sql);
        if observed != Some("42704") {
            wrong.push(format!("{sql} → {observed:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "a cast target whose canonical name is not `regclass` must be 42704; these were not: {wrong:#?}"
    );
}

/// The polarity: the bare spelling still lowers to the `regclass` node, in the forms
/// below.
///
/// This is the over-correction guard. A gate written as "refuse a cast target beginning
/// with `regclass`", or one that moved the intercept behind the type resolver, takes the
/// whole model down with the suffix — including the `nextval('…'::regclass)` column
/// `DEFAULT` pg_dump emits, which reaches a **different** reader (`parser::ddl`'s
/// `parse_nextval_arg`) and must not move either.
#[test]
fn the_bare_regclass_spellings_still_answer() {
    let mut db = Database::with_btree();
    seed(&mut db);

    assert_eq!(text(&mut db, "SELECT 'qt'::regclass"), "qt");
    assert_eq!(text(&mut db, "SELECT CAST('qt' AS regclass)"), "qt");
    assert_eq!(text(&mut db, "SELECT 'qt'::REGCLASS"), "qt");
    assert_eq!(text(&mut db, "SELECT 'qt'::regclass::text"), "qt");
    assert_eq!(text(&mut db, "SELECT 'public.qt'::regclass"), "qt");

    // `regclass` as a *column* name is not a cast target at all and never was; the pin is
    // cheap and the reader is one `peek` away from it.
    db.execute("CREATE TABLE rc (regclass INT)")
        .expect("a column may be named regclass");
    db.execute("INSERT INTO rc VALUES (7)").expect("it stores");
    assert_eq!(
        db.execute("SELECT regclass FROM rc")
            .expect("it reads")
            .remove(0)
            .rows,
        vec![vec![Value::Int4(7)]]
    );

    // pg_dump's serial-column default. `parser::ddl`'s classifier eats the cast token
    // without resolving the type, so this route never reaches `parse_cast_target` — which
    // is exactly why it needs its own pin here.
    db.execute("CREATE TABLE dseq (a INT DEFAULT nextval('qs'::regclass))")
        .expect("the pg_dump column DEFAULT spelling still loads");
    db.execute("INSERT INTO dseq DEFAULT VALUES")
        .expect("and the default draws from the sequence");
    assert_eq!(
        db.execute("SELECT a FROM dseq")
            .expect("reads")
            .remove(0)
            .rows,
        vec![vec![Value::Int4(1)]]
    );
}

/// The headline: the array cast target no longer removes rows, on **both** CoW-B+tree
/// shapes.
///
/// `Database::with_btree` is the in-memory tree the server runs without `--data-dir`;
/// `open_database` over a `MemBlockIo` is the double-meta store `--data-dir` selects,
/// with the block device in memory. The refusal is the parser's, so neither backend can
/// reach a write — which is the point: a repair placed at evaluation instead would leave
/// the `--storage memory` backend, whose failed DML has no autocommit rollback, able to
/// remove part of the range before raising.
#[test]
fn an_array_cast_target_does_not_delete() {
    for (label, mut db) in [
        ("with_btree", Database::with_btree()),
        ("double_meta", open_database(Box::new(MemBlockIo::new()))),
    ] {
        db.execute("CREATE TABLE qt (a INT)").expect("seeds");
        db.execute("INSERT INTO qt VALUES (1), (2)").expect("seeds");

        let observed = state(&mut db, "DELETE FROM qt WHERE 'qt'::regclass[] = 'qt'");
        assert_eq!(
            observed,
            Some("42704"),
            "{label}: the DELETE must be refused"
        );

        let rows = db
            .execute("SELECT a FROM qt ORDER BY a")
            .expect("reads")
            .remove(0)
            .rows;
        assert_eq!(
            rows,
            vec![vec![Value::Int4(1)], vec![Value::Int4(2)]],
            "{label}: both rows must survive the refused DELETE"
        );
    }
}

/// The one array spelling 18.6 **accepts** is refused here, and the reader that used to
/// swallow the suffix would have answered it.
///
/// `'{qt}'` is a well-formed one-element array literal on 18.6, whose element `qt` is a
/// relation — so `SELECT '{qt}'::regclass[]` is `{qt}` there. The swallowing reader threw
/// the `[]` away and looked the *whole* literal up as a name, which resolves against the
/// relation deliberately created as `"{qt}"` — so before this rule it answered a row.
/// That pair is what makes `catalog/regclass_array_target_is_loud.sql` a live guard
/// rather than an "any pglet error passes" waiver, and this is its unit half: the scalar
/// spelling below still answers, so the fixture's redness on a revert is the suffix's and
/// not the literal's.
#[test]
fn an_array_literal_naming_a_relation_is_refused_too() {
    let mut db = Database::with_btree();
    seed(&mut db);

    assert_eq!(text(&mut db, "SELECT '{qt}'::regclass"), "\"{qt}\"");
    assert_eq!(state(&mut db, "SELECT '{qt}'::regclass[]"), Some("42704"));
}

/// The answer is the relation's name **rendered for an identifier position**, from both
/// argument arms, and the digits fallback is not rendered that way.
///
/// `lexer::quote_ident` is the predicate — the one `fmtId`-style renderer this engine
/// has — so a name needing quotes gets them and a plain one does not. The **round trip**
/// is what the rule buys:
/// `'"Odd"'::regclass::text` fed back to `::regclass` names `Odd` again, where the bare
/// `Odd` the stored bytes gave is `42P01` on both engines.
///
/// Three over-corrections this rejects: quoting unconditionally (the `qt` rows), quoting
/// the numeric fallback (`99999`, which is not a bare identifier and would come back
/// `"99999"`), and quoting in the name arm only (the `pg_class` OID row).
#[test]
fn regclass_renders_the_relation_name_for_an_identifier_position() {
    let mut db = Database::with_btree();
    seed(&mut db);

    assert_eq!(text(&mut db, "SELECT 'qt'::regclass"), "qt");
    assert_eq!(text(&mut db, "SELECT '\"Odd\"'::regclass"), "\"Odd\"");
    assert_eq!(
        text(&mut db, "SELECT ('\"Odd\"'::regclass::text)::regclass"),
        "\"Odd\"",
        "the answer must re-resolve to the relation it names"
    );
    assert_eq!(
        state(&mut db, "SELECT 'Odd'::regclass"),
        Some("42P01"),
        "the unquoted spelling folds to `odd`, which is a different name — as it is on 18.6"
    );

    // The OID arm, reached through the catalog rather than through a literal.
    assert_eq!(
        text(
            &mut db,
            "SELECT c.oid::regclass::text FROM pg_class c WHERE c.relname = 'Odd'"
        ),
        "\"Odd\""
    );
    // …and its fallback, which `regclassout` prints as digits and `quote_ident` would
    // wrap. (18.6 prints `-` for `InvalidOid` rather than `0`; that arm is `EDGES.md`
    // row 85's (f) and is not this rule's.)
    assert_eq!(text(&mut db, "SELECT 99999::regclass"), "99999");
}
