//! `json` is a type pglet does not model, so it answers `42704` — and the accept side
//! beside it does not move.
//!
//! **These are the discriminating guards for the rule; the fixtures are not.** The bare
//! type-name spellings — the ones the `types/json_*_loud.sql` fixtures mirror — are ones
//! 18.6 *accepts*, so under `tests/README.md`'s `@expect-error`
//! rule "where Postgres accepts, any pglet error passes" the differential corpus cannot
//! tell `42704` from `42601` from a panic. Those fixtures record the
//! shape; the SQLSTATE and the resolver's verdict are pinned here. The two **operator**
//! spellings below are the exception in the other direction — 18.6 refuses `json = json`
//! and `json @> json` under `42883`, so a fixture for those would be discriminating and
//! **red**, pglet's code being `42704` from one operand up. The `json(3)` rows further
//! down are a third case again: 18.6 is `42601` at each of them, so an `@expect-error`
//! fixture there would compare SQLSTATEs rather than waive them — green at the column and
//! `ADD COLUMN` spellings and red at `::`/`CAST`.
//!
//! 18.6's `json` keeps its input text verbatim — whitespace, key order and **duplicate
//! keys** — which no `jsonb` value can hold, which is why the spelling is refused rather
//! than mapped. Measured on 18.6, and written down here because the fixture that issues
//! it does not record it: `SELECT '{"b":1,  "a":2, "b":3}'::json` answers
//! `{"b":1,  "a":2, "b":3}` — the doubled space, the `b`-before-`a` order and the
//! repeated `b` all intact — where
//! the same literal through `::jsonb` answers `{"a": 2, "b": 3}`. That is
//! `tests/fixtures/types/json_cast_loud.sql`'s literal, and its recorded shape is the
//! harness's `serde_json` reparse of the answer rather than the answer, so the duplicate
//! is gone from the record and what is left is byte-identical to `::jsonb`'s.

// A test harness: `.expect()`/`.unwrap()` surface a setup failure loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use pglet::Database;
use pglet_sql::types::Value;

/// The SQLSTATE `sql` fails with, or `None` when it succeeds.
fn state(db: &mut Database, sql: &str) -> Option<&'static str> {
    db.execute(sql).err().map(|e| e.sqlstate())
}

/// Every position that **resolves** a type name answers `42704` for `json`.
///
/// The positions are re-derived from the readers rather than from a remembered list.
/// The resolver has **two** entries onto one `resolve_type_name` — `resolve_column_type`
/// for `gram.y`'s `Typename` and `resolve_constant_type` for `ConstTypename`, differing
/// in the parenless-`char` fixed-length default and in nothing a `json` spelling can
/// see — so the grep is of the pair.
/// It finds four call sites outside the resolver's
/// own module and its tests: `executor::ddl`'s `execute_create_table` and
/// `execute_alter_table_add_column`, and `parser::expr`'s `resolve_type_with_typmod`
/// (reached from `parse_cast_target`, so from `::` and from `CAST(… AS …)`) and
/// `typed_constant_from` (`json '…'`, the one `resolve_constant_type` caller). The DDL column
/// path is **not** a parser reader — `parser::ddl`'s `parse_column` reads the type name
/// and stores the string on `ParsedColumn.type_name`, which rides the plan's `ColumnDef`
/// to the executor, where the resolve happens. `json[]` is a **member**, not a downstream
/// effect — the array spelling recurses through `type_name_to_oid` at the same call site —
/// so it is checked at the column and the cast positions both.
///
/// **Where a position writes a type name pglet accepts without resolving, it is outside
/// what this asserts and outside the refusal**: `CREATE`/`ALTER SEQUENCE … AS
/// <type>` and an identity column's `AS <type>`, both of which `parser::ddl` reads and
/// discards. Both accept `json` here — and `point`, and `nosuchtype`, so the gap is the
/// position's rather than this rule's. That is a statement about **pglet**: 18.6 resolves
/// at the first, and the three names split there — `json` and `point` are
/// `22023 sequence type must be smallint, integer, or bigint` (`init_params`' kind check,
/// after `typenameTypeId` has already resolved the name) and `nosuchtype` is
/// `42704 type "nosuchtype" does not exist` from that resolution. At the second all three
/// are `42601 conflicting or redundant options` and no name is resolved at all:
/// `generateSerialExtraStmts` (`parse_utilcmd.c`) prepends its own synthetic
/// `AS <the column's type>` — `lcons` and not `lappend`, its own comment giving the
/// reason, so that the redundant-options error points at the user's occurrence rather
/// than the synthetic one — which makes a user-written `AS` a duplicate option.
/// `errorConflictingDefElem` then fires **inside** `init_params` (`sequence.c`), in that
/// function's `foreach(option, options)` loop at the `strcmp(defel->defname, "as")` arm,
/// ahead of the `/* AS type */` block *later in the same function* that would call
/// `typenameTypeId`: one option scanner, two orderings, and 18.6's caret lands on the
/// user's `AS`. (`parse_utilcmd.c` calls `errorConflictingDefElem` too, but only for
/// `sequence_name` and `logged`/`unlogged` — never for `as`.) Each measured on both
/// engines.
#[test]
fn json_is_refused_at_every_type_name_position() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE jt (x INT)").expect("seeds");

    let mut wrong = Vec::new();
    for sql in [
        // column definition
        "CREATE TABLE j1 (a json)",
        "CREATE TABLE j2 (a json[])",
        "CREATE TABLE j3 (a json[][])",
        // ALTER TABLE … ADD COLUMN
        "ALTER TABLE jt ADD COLUMN a json",
        "ALTER TABLE jt ADD COLUMN b json[]",
        // `::` and `CAST(… AS …)`
        "SELECT '{\"a\":1}'::json",
        "SELECT CAST('{\"a\":1}' AS json)",
        "SELECT '{\"{}\"}'::json[]",
        "SELECT CAST('{\"{}\"}' AS json[])",
        // typed constant (`ConstTypename Sconst`)
        "SELECT json '{\"a\":1}'",
        // the fold is ASCII-downcasing at the parser, so case cannot smuggle it through
        "SELECT '{}'::JSON",
        "SELECT '{}'::JsOn",
        "CREATE TABLE j4 (a JSON)",
        // and the operator surface the alias used to answer through
        "SELECT '{\"a\":1}'::json = '{\"a\":1}'::json",
        "SELECT '{\"a\":1}'::json @> '{\"a\":1}'::json",
    ] {
        // Collected rather than asserted per iteration: a `?`-style stop reports only the
        // **first** position, which would leave every position after it — `json[]` among
        // them — untested by the run that fails, and so unproven by a revert.
        let observed = state(&mut db, sql);
        if observed != Some("42704") {
            wrong.push(format!("{sql} → {observed:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "`json` must be `42704` at every resolving type-name position; these were not: {wrong:#?}"
    );
}

/// The polarity: `jsonb` and its whole surface are untouched by the refusal above.
///
/// Green under the revert of the refusal itself (restoring `type_oid_from_name`'s `| "json"` arm)
/// by construction — it guards the **over-correction** direction, a fix that took `jsonb`
/// down with `json`.
#[test]
fn jsonb_is_untouched_at_the_same_positions() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE bt (a jsonb, b jsonb[])")
        .expect("a jsonb column and a jsonb[] column still declare");
    db.execute("ALTER TABLE bt ADD COLUMN c jsonb")
        .expect("ADD COLUMN jsonb still resolves");
    db.execute("INSERT INTO bt (a) VALUES ('{\"a\":1}')")
        .expect("a jsonb value still stores");

    let mut wrong = Vec::new();
    for sql in [
        "SELECT '{\"a\":1}'::jsonb",
        "SELECT CAST('{\"a\":1}' AS jsonb)",
        "SELECT jsonb '{\"a\":1}'",
        "SELECT '{\"a\":1}'::jsonb @> '{\"a\":1}'::jsonb",
        "SELECT '{\"a\":1}'::jsonb -> 'a'",
        "SELECT '{}'::JSONB",
    ] {
        if let Some(code) = state(&mut db, sql) {
            wrong.push(format!("{sql} → {code}"));
        }
    }
    assert!(wrong.is_empty(), "jsonb must still answer: {wrong:#?}");
}

/// A user `ENUM` named `json` still resolves to the enum, at the column and at the cast.
///
/// `executor::resolve_column_type` consults `Schema.enums` **before** the builtin table,
/// so the name is the enum's here. That shadowing is a **divergence pglet already had**,
/// not something this refusal created: 18.6 searches `pg_catalog` first, so the same
/// declaration is `pg_catalog.json` (atttypid 114) there and an `INSERT … VALUES ('x')`
/// is `22P02` (both measured). What this pins is only that closing the alias
/// did not move it — a fix placed in the parser instead of the resolver would have.
#[test]
fn an_enum_named_json_still_resolves_to_the_enum() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE json AS ENUM ('x', 'y')")
        .expect("an enum may be named json");
    db.execute("CREATE TABLE ej (a json)")
        .expect("the enum, not the builtin, answers the column's type name");
    db.execute("INSERT INTO ej VALUES ('x')")
        .expect("a member stores");

    let rows = db
        .execute("SELECT a FROM ej")
        .expect("the column reads")
        .remove(0)
        .rows;
    assert_eq!(rows, vec![vec![Value::Text("x".to_string())]]);

    // A non-member is refused by the enum's member set, which is the proof the column is
    // the enum and not a text passthrough.
    assert_eq!(
        state(&mut db, "INSERT INTO ej VALUES ('{\"a\":1}')"),
        Some("22P02")
    );
    // And the cast target reads the same catalog.
    assert_eq!(state(&mut db, "SELECT 'y'::json"), None);
}

/// `json` as an ordinary **column** name still works — the refusal is a type-name rule,
/// not a new lexer keyword.
///
/// The over-correction this rejects is real rather than hypothetical: `json` *is* a
/// `COL_NAME_KEYWORD` on 18.6 (`kwlist.h`), so promoting it to a pglet keyword looks like
/// conformance and would break the spelling 18.6 keeps working.
#[test]
fn json_is_still_an_ordinary_column_name() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE cn (json INT, jsonb INT)")
        .expect("json and jsonb are ordinary column names");
    db.execute("INSERT INTO cn VALUES (1, 2)").expect("stores");
    let rows = db
        .execute("SELECT json, jsonb FROM cn WHERE json = 1")
        .expect("reads")
        .remove(0)
        .rows;
    assert_eq!(rows, vec![vec![Value::Int4(1), Value::Int4(2)]]);
}

/// `json(3)` stays a **parse** error at the two positions that resolve after parsing.
///
/// This is what `parser::type_name`'s `"json" => "json"` arm buys, and its own revert is
/// deleting that arm — not the `type_oid_from_name` one. `json` is a `COL_NAME_KEYWORD` on 18.6, so
/// `json(3)` is `42601` there at the column, `ADD COLUMN`, `::`, `CAST` and
/// typed-constant spellings alike; reading the name and leaving the `(` unread keeps
/// `42601` here on the DDL column path, where the pass-through would consume the modifier
/// and answer `42704`. The `::`/`CAST` positions resolve inline and answer
/// `42704` either way — a residual, recorded here so it is not read as intent.
///
/// **The typed-constant spelling is the fifth of 18.6's five and is not asserted below**,
/// because it never reaches the type-name reader: `json` is not an
/// `is_multiword_type_lead` word, so `try_typed_constant`'s two-token window sees
/// `json (` rather than `json '…'`, declines, and `SELECT json(3) '{}'` falls to the
/// function-call path for pglet's own `0A000 not implemented: function not supported:
/// json`. That answer belongs to the **shape** rather than to `json` — `point(3) '{}'`
/// and `nosuchfn(3) '{}'` read the same, and 18.6 is `42601`, `42601` and
/// `42704 type "nosuchfn" does not exist` across those three — so deleting the arm this
/// test guards would not move it, and a row here would pin something else's rule.
/// Measured on both engines.
#[test]
fn a_json_type_modifier_is_a_parse_error_on_the_column_path() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE mt (x INT)").expect("seeds");
    let mut wrong = Vec::new();
    for (sql, want) in [
        // Resolved after parsing, so the unread `(` is the verdict — 18.6's code.
        ("CREATE TABLE m1 (a json(3))", "42601"),
        ("ALTER TABLE mt ADD COLUMN a json(3)", "42601"),
        // Resolved inline, so the type name is the verdict — the residual.
        ("SELECT '{}'::json(3)", "42704"),
        ("SELECT CAST('{}' AS json(3))", "42704"),
    ] {
        let observed = state(&mut db, sql);
        if observed != Some(want) {
            wrong.push(format!("{sql} → {observed:?}, wanted {want}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// `--coerce-unknown-types` now reaches `json`, and degrades it to `TEXT`.
///
/// The flag's contract is exactly that — coerce an unsupported type to `TEXT` instead of
/// erroring — and `json` became one, so the flag answers `text` where it used to answer
/// `jsonb`. Nothing special-cases it; this pins the behaviour rather than blessing it.
/// The toggle is a thread-local, so it is reset before the assertions run.
#[test]
fn the_coerce_flag_degrades_json_to_text() {
    let mut db = Database::with_btree();
    db.set_coerce_unknown_types(true);
    let created = db.execute("CREATE TABLE cj (a json)");
    let declared = db.execute(
        "SELECT data_type FROM information_schema.columns WHERE table_name = 'cj' AND column_name = 'a'",
    );
    db.set_coerce_unknown_types(false);

    created.expect("under the coerce flag an unsupported type is accepted");
    let rows = declared.expect("the column reads back").remove(0).rows;
    assert_eq!(rows, vec![vec![Value::Text("text".to_string())]]);
}
