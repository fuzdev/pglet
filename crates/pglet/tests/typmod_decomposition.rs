//! A canonical type name decomposes once — base, modifier, array suffix — and each of
//! the three is decided by the *type*.
//!
//! What is here is the half the differential corpus cannot reach. The fixtures under
//! `tests/fixtures/types/typmod_not_allowed_*` and `tests/fixtures/array/typmod_*` hold
//! the refusals and the element-wise enforcement, comparing pglet against 18.6 through
//! the wire; three things sit outside them:
//!
//! - **The dump.** A modifier is the element's, so a `varchar(3)[]` column must dump as
//!   `varchar(3)[]` and reload as an array. Rendering it at the *column's* width emits a
//!   scalar `varchar(3)`, which reloads a different table — a break of the dump
//!   invariant's catalog part, that any catalog pglet's own accepted statements build
//!   dumps to something that loads back. No
//!   fixture compares pglet's dump text with the column shape it reloads to.
//! - **The persisted catalog.** The cap must survive a store round-trip on the
//!   **double-meta** path, not only in a live `Schema`.
//! - **The polarity list.** A fixture asserts one statement; the over-correction this
//!   rule invites — refusing a modifier a type does carry, or refusing an array that
//!   declares none — is a list, and it is cheaper to keep the list in one place.
//! - **The catalog a refusal must not leave.** A modifier the type itself refuses
//!   (`char(0)`, `numeric(5,2,3)`) takes the `CREATE TABLE` / `ADD COLUMN` down, and a
//!   fixture asserting that error is green whether or not a phantom table or a
//!   half-added column survived it — the error is all it can see. The absence is
//!   asserted here.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface a setup failure loudly right
// here. `clippy::panic` has no test-allowance in this clippy (only unwrap/expect do).
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::common;

use common::open_database;
use pglet::Database;
use pglet_testing::block_io::MemBlockIo;

/// The SQLSTATE `sql` fails with, or `None` when it succeeds.
fn state(db: &mut Database, sql: &str) -> Option<&'static str> {
    db.execute(sql).err().map(|e| e.sqlstate())
}

/// The single text value `sql` answers (an array renders in Postgres's `{…}` form).
fn text(db: &mut Database, sql: &str) -> String {
    let rows = db.execute(sql).expect("the query answers").remove(0).rows;
    let v = rows
        .first()
        .and_then(|r| r.first())
        .unwrap_or_else(|| panic!("expected one value from `{sql}`"));
    pglet_sql::types::value_to_text(v).unwrap_or_else(|| "NULL".to_string())
}

/// A `varchar(n)[]` / `numeric(p,s)[]` column dumps with the modifier **inside** the
/// `[]`, and the dump reloads to the same column shape.
///
/// The reload check is the load-bearing half: a dump that spelled `varchar(3)` for the
/// array column would still be valid SQL, and a text assertion alone would not say that
/// the reloaded column is a *scalar*.
#[test]
fn an_array_columns_modifier_dumps_inside_the_suffix_and_reloads() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE d (a varchar(3)[], b numeric(4,1)[], c varchar[], e varchar(3))",
        "INSERT INTO d VALUES ('{ab,xy}', '{1.25,2.5}', '{abcdefg}', 'ab')",
    ] {
        db.execute(sql).expect("seed");
    }
    let dump = db.dump_sql().expect("dump");
    for spelling in [
        "a varchar(3)[]",
        "b numeric(4,1)[]",
        "c varchar[]",
        "e varchar(3)",
    ] {
        assert!(
            dump.contains(spelling),
            "dump must spell `{spelling}`:\n{dump}"
        );
    }

    // Reload into a fresh engine and re-derive the shape from what it then accepts and
    // refuses: an array column takes an array literal and refuses a bare scalar, and the
    // element cap is still enforced. The `COPY … FROM stdin` blocks carry the rows over a
    // wire this test has no client for and are dropped — the shape is what is under test.
    let mut in_copy = false;
    let ddl: String = dump
        .lines()
        .filter(|line| {
            if in_copy {
                in_copy = *line != "\\.";
                return false;
            }
            in_copy = line.starts_with("COPY ") && line.ends_with("FROM stdin;");
            !in_copy
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut reloaded = Database::with_btree();
    for stmt in ddl.split(";\n") {
        let stmt = stmt.trim().trim_end_matches(';');
        if stmt.is_empty() {
            continue;
        }
        reloaded
            .execute(stmt)
            .unwrap_or_else(|e| panic!("reload `{stmt}`: {e}"));
    }
    assert_eq!(
        state(&mut reloaded, "INSERT INTO d (a) VALUES ('abcdefg')"),
        Some("22P02"),
        "a reloaded `varchar(3)[]` column must still be an array",
    );
    assert_eq!(
        state(&mut reloaded, "INSERT INTO d (a) VALUES ('{abcdefg}')"),
        Some("22001"),
        "a reloaded `varchar(3)[]` column must still cap its elements",
    );
    assert_eq!(
        state(&mut reloaded, "INSERT INTO d (b) VALUES (1.25)"),
        Some("42804"),
        "a reloaded `numeric(4,1)[]` column must still be an array",
    );
    reloaded
        .execute("INSERT INTO d (b) VALUES ('{1.25}')")
        .expect("the array literal stores");
    assert_eq!(
        text(
            &mut reloaded,
            "SELECT b FROM d WHERE b IS NOT NULL ORDER BY b"
        ),
        "{1.3}",
        "a reloaded `numeric(4,1)[]` column must still round its elements",
    );
}

/// The element cap survives a **double-meta store** round-trip: it is a catalog field,
/// and a column whose cap lived only in the live `Schema` would enforce nothing after a
/// reopen.
#[test]
fn an_array_columns_element_cap_survives_a_reopen() {
    let io = MemBlockIo::new();
    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE p (a varchar(3)[], b numeric(4,1)[])")
            .expect("create");
        db.commit(0).expect("commit");
    }
    let mut db = open_database(Box::new(io));
    assert_eq!(
        state(&mut db, "INSERT INTO p (a) VALUES ('{abcdefg,xy}')"),
        Some("22001"),
    );
    db.execute("INSERT INTO p (b) VALUES ('{1.25,2.5}')")
        .expect("the array literal stores");
    assert_eq!(
        text(&mut db, "SELECT b FROM p WHERE b IS NOT NULL"),
        "{1.3,2.5}"
    );
}

/// A temporal column's declared precision is **enforced on store**, and it survives a
/// **double-meta store** round-trip.
///
/// The store-time arm is `executor::constraints::coerce_column_value`'s third
/// `apply_typmod` call, and it is the central write: every write path funnels
/// through it, so an unenforced precision writes a value the declared type does not
/// have and every later read is that value. Deleting the arm leaves `cargo test` over
/// `pglet`, `pglet_sql`, `pglet_core` and `pglet_storage` green without this test
/// (measured) — eight `precision_*` fixtures go red on it in `test:fixtures`, and both
/// `copy/` ones in `test:copy`, so the differential harness caught it all along and
/// `cargo test` did not.
///
/// The reopen half is the same claim [`an_array_columns_element_cap_survives_a_reopen`]
/// makes for a length: the precision is a catalog field, and a column whose precision
/// lived only in the live `Schema` would round nothing after a reopen. The array column
/// is here because `apply_typmod` reaches an array's elements one recursion down.
#[test]
fn a_temporal_columns_precision_is_enforced_on_store_and_survives_a_reopen() {
    let io = MemBlockIo::new();
    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute(
            "CREATE TABLE tp (a timestamp(3), b timestamptz(3), c time(3), d interval(3), \
             e timestamp(3)[])",
        )
        .expect("create");
        db.commit(0).expect("commit");
    }
    let mut db = open_database(Box::new(io));
    db.execute(
        "INSERT INTO tp VALUES ('2020-01-02 03:04:05.6789', '2020-01-02 03:04:05.6789', \
         '03:04:05.6789', '1.23456 secs', '{2020-01-02 03:04:05.6789}')",
    )
    .expect("the row stores");
    // Each value 18.6's, measured: the fourth fractional digit rounds half away from
    // zero and the column keeps three.
    assert_eq!(text(&mut db, "SELECT a FROM tp"), "2020-01-02 03:04:05.679");
    // `text` renders through the column-type-blind `value_to_text`, so the
    // `timestamptz` prints without the `+00` its wire encoder appends. What is asserted
    // here is the rounding, which is the same either way.
    assert_eq!(text(&mut db, "SELECT b FROM tp"), "2020-01-02 03:04:05.679");
    assert_eq!(text(&mut db, "SELECT c FROM tp"), "03:04:05.679");
    assert_eq!(text(&mut db, "SELECT d FROM tp"), "00:00:01.235");
    assert_eq!(
        text(&mut db, "SELECT e FROM tp"),
        "{\"2020-01-02 03:04:05.679\"}",
        "an array column rounds element-wise",
    );
}

/// An **array** column's `atttypmod` is its *element's*, and the `interval` arm is the
/// one that has to ask which.
///
/// `Column::pg_type_modifier`'s temporal arm takes `type_oid.array_element()` before
/// choosing an encoding, because an `interval(3)[]` column's `TypeOid` is
/// `IntervalArray` and only the element's OID says the `INTERVAL_FULL_RANGE` packing
/// applies. Reading the column's own OID instead answers a bare `3` there, which is
/// wrong on the wire and in `pg_attribute` alike — and leaves that same four-crate
/// `cargo test` green without this test (measured).
///
/// Every value below is 18.6's over the same declarations, measured.
#[test]
fn an_array_columns_type_modifier_is_its_elements() {
    let mut db = Database::with_btree();
    db.execute(
        "CREATE TABLE am (a varchar(3)[], b timestamp(3)[], c interval(3)[], \
         d timestamptz(2)[], e time(1)[], f varchar(3), g interval(3))",
    )
    .expect("create");
    for (col, want) in [
        ("a", "7"),
        ("b", "3"),
        ("c", "2147418115"),
        ("d", "2"),
        ("e", "1"),
        ("f", "7"),
        ("g", "2147418115"),
    ] {
        assert_eq!(
            text(
                &mut db,
                &format!(
                    "SELECT atttypmod FROM pg_attribute WHERE attrelid = \
                     (SELECT oid FROM pg_class WHERE relname = 'am') AND attname = '{col}'"
                ),
            ),
            want,
            "`am.{col}` reports its element's modifier",
        );
    }
}

/// **A `(n)` the type refuses is still a spelled modifier, not the parenless
/// spelling** — so it takes no fixed-length default, at a typed constant and at a
/// declared position alike; it takes the statement down at both.
///
/// `gram.y` installs the `bpchar(1)` default on `CharacterWithoutLength` and nowhere
/// else, and `ConstCharacter` clears that one arm; `CharacterWithLength` hands its
/// `Iconst` straight to `typenameTypeMod`, which hands it to `anychar_typmodin`. The
/// keying has been wrong in both directions and each was worse on a write than on a
/// read: reading the refusal as "no modifier was spelled" collected the default, so
/// `'ab'::char(0)` truncated to `a` and `CREATE TABLE … (c CHAR(0))` built a column
/// that refused `'ab'` with `22001` — a cap nobody spelled; reading it as "no cap"
/// built an unconstrained column that **stored** `'abcdef'`, where 18.6 has no table
/// to hold the row (`EDGES.md` row 124).
///
/// **What this compares, and what it never looks at.** Every assertion puts a pglet
/// answer against a literal, so there is no second derivation for a common move to
/// hide behind; the resolver-level pair in
/// `executor::type_resolution`'s `bare_char_carries_no_length_at_a_constant_position`
/// is the one whose two sides both route through `resolve_type_name`, and it is blind
/// to anything that moves them together. What it never looks at is **18.6's** reply —
/// now because it does not need to rather than because it could not: each verdict below
/// was measured on 18.6 as well, and the `types/refused_typmod_*` fixtures compare the
/// class live, so what is left here is the half a fixture cannot hold. That half is the **catalog**: a
/// refused `CREATE TABLE` must leave no table and a refused `ADD COLUMN` no column, and
/// a fixture that asserted the error would be green over either phantom.
#[test]
fn a_refused_length_is_not_the_parenless_spelling() {
    let mut db = Database::with_btree();
    // The split still holds, at the spelling it is actually keyed on: parenless.
    for (sql, want) in [
        ("SELECT char 'ab'", "ab"),
        ("SELECT character 'ab'", "ab"),
        ("SELECT 'ab'::char", "a"),
        ("SELECT CAST('ab' AS character)", "a"),
        // A modifier inside the window caps at both positions, at both its ends.
        ("SELECT char(1) 'ab'", "a"),
        ("SELECT 'ab'::char(1)", "a"),
        ("SELECT char(3) 'abcde'", "abc"),
        ("SELECT 'abcde'::char(3)", "abc"),
        ("SELECT 'ab'::varchar(10485760)", "ab"),
        ("SELECT varchar(10485760) 'ab'", "ab"),
    ] {
        assert_eq!(text(&mut db, sql), want, "`{sql}`");
    }
    // …and one outside it is refused at both, under the same sentence — not capped at
    // `1` on one side and unbounded on the other.
    for (sql, want) in [
        (
            "SELECT char(0) 'ab'",
            "length for type char must be at least 1",
        ),
        (
            "SELECT 'ab'::char(0)",
            "length for type char must be at least 1",
        ),
        (
            "SELECT varchar(0) 'ab'",
            "length for type varchar must be at least 1",
        ),
        (
            "SELECT 'ab'::varchar(0)",
            "length for type varchar must be at least 1",
        ),
        (
            "SELECT char(20000000) 'ab'",
            "length for type char cannot exceed 10485760",
        ),
        (
            "SELECT 'ab'::char(20000000)",
            "length for type char cannot exceed 10485760",
        ),
    ] {
        let err = db.execute(sql).expect_err("the length is refused");
        assert_eq!(
            (err.sqlstate(), err.to_string()),
            ("22023", want.to_string()),
            "`{sql}`"
        );
    }
    // **The catalog half**, which is this test's own: a refused length must leave no
    // table and no column behind, so the statement is the `22023` *and* nothing of it
    // is durable. A phantom `cz` would make the retry `42P07` and a phantom column
    // would widen every `SELECT *` over `cp`.
    assert_eq!(state(&mut db, "CREATE TABLE cz (c CHAR(0))"), Some("22023"));
    assert_eq!(
        text(
            &mut db,
            "SELECT count(*) FROM information_schema.tables WHERE table_name = 'cz'"
        ),
        "0",
    );
    db.execute("CREATE TABLE cp (c CHAR)").expect("create");
    assert_eq!(
        state(&mut db, "ALTER TABLE cp ADD COLUMN d VARCHAR(0)"),
        Some("22023")
    );
    assert_eq!(
        text(
            &mut db,
            "SELECT count(*) FROM information_schema.columns WHERE table_name = 'cp'"
        ),
        "1",
    );
    // The polarity control on the same surface: a **parenless** `char` column is still
    // `char(1)`, and still refuses the value 18.6 refuses (`22001` on both engines,
    // naming different types — `EDGES.md` row 95).
    assert_eq!(
        state(&mut db, "INSERT INTO cp VALUES ('ab')"),
        Some("22001")
    );
    assert_eq!(
        text(
            &mut db,
            "SELECT character_maximum_length FROM information_schema.columns \
             WHERE table_name = 'cp'"
        ),
        "1",
    );
}

/// A refused `numeric(…)` leaves no catalog behind either, and the **surplus** third
/// component is the member a range check would have missed.
///
/// `1.5::numeric(5,2,3)` applied the scale `2` and ignored the `3`, so the silence was
/// not "an out-of-range modifier dropped" but "a spelling 18.6 refuses as a whole,
/// half-honoured". The per-member verdicts and their order are
/// `executor::type_resolution`'s `a_numeric_modifier_is_refused_in_18_6s_order`; what is
/// here is the write side — the value a surplus component used to round, and the catalog
/// a refused column definition must not leave.
#[test]
fn a_refused_numeric_modifier_leaves_no_value_and_no_catalog() {
    let mut db = Database::with_btree();
    for (sql, want) in [
        ("SELECT 1.5::numeric(5,2,3)", "22023"),
        ("SELECT numeric(5,2,3) '1.5'", "22023"),
        ("SELECT 1.5::numeric(0)", "22023"),
        ("SELECT 1.5::numeric(5,1001)", "22023"),
        ("SELECT 1.5::numeric(2147483648)", "22003"),
        ("SELECT 1.5::numeric(1.5)", "22P02"),
    ] {
        assert_eq!(state(&mut db, sql), Some(want), "`{sql}`");
    }
    // The controls, which is what keeps the arity gate from reading as "two components
    // are suspect": one and two components inside both windows still apply.
    assert_eq!(text(&mut db, "SELECT 1.5::numeric(5,2)"), "1.50");
    assert_eq!(text(&mut db, "SELECT 1.5::numeric(1000)"), "2");
    assert_eq!(text(&mut db, "SELECT 1.5::numeric(5,-1000)"), "0");
    // The catalog half.
    assert_eq!(
        state(&mut db, "CREATE TABLE nz (c NUMERIC(5,2,3))"),
        Some("22023")
    );
    assert_eq!(
        text(
            &mut db,
            "SELECT count(*) FROM information_schema.tables WHERE table_name = 'nz'"
        ),
        "0",
    );
}

/// The polarity list: what the refusal must **not** reach, and what must not gain a cap.
///
/// The over-correction this rule invites is refusing a modifier a type does carry, or
/// treating an array as a reason to refuse. Each spelling below is accepted on 18.6
/// (measured), and the last two pin that no cap is invented where none was declared.
///
/// **Now that a carrier refuses as well, the window's own ends are on this list**, and
/// so is the lexer's decoded payload. An inclusive bound written exclusive refuses a
/// spelling 18.6 accepts at exactly one value per end — `char(1)`, `varchar(10485760)`,
/// `numeric(1)`, `numeric(1000)`, `numeric(5,±1000)` — and the `int4` coercion the
/// refusal now runs reads the number a **second** time after `opt_precision` did, so
/// `char(0x3)` and `char(03)` stand here to pin that the decoded payload and the leading
/// zero both survive it.
///
/// **`float(3)` is here on a different route and is named rather than left to read as a
/// carrier.** Its modifier *selects* the type — `real` for 1–24 — so
/// `parser::type_name`'s `float_type_name` spends it and hands the resolver the bare
/// `real`, with no `(…)` for `type_oid_carries_modifier` to judge (`EDGES.md` row 116).
/// Refusing it would be the same over-correction by a route this gate cannot see, which
/// is why it belongs on the polarity list and not in the carriers above it.
#[test]
fn what_the_refusal_must_not_reach_is_accepted_and_gains_no_cap() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE ok (a varchar(3), b numeric(4,1), c char(3), d interval(3))",
        "CREATE TABLE oka (a varchar(3)[], b numeric(4,1)[], c char(3)[], d interval(3)[])",
        "CREATE TABLE plain (a varchar[], b numeric[], c int4[], d interval[])",
        "CREATE TABLE ts (a timestamp(3), b timestamp(3)[], c time(3)[], d timestamptz(2))",
        // The selector's route, not a carrier's — see the doc above.
        "CREATE TABLE sel (a float(3), b float(3)[])",
        // Both windows at both ends, inclusive.
        "CREATE TABLE ends (a char(1), b varchar(10485760), c numeric(1), d numeric(1000))",
        "CREATE TABLE sends (a numeric(5,1000), b numeric(5,-1000), c numeric(5,0))",
    ] {
        assert_eq!(state(&mut db, sql), None, "`{sql}` must be accepted");
    }
    for sql in [
        "SELECT 'abcd'::varchar(3)",
        "SELECT 1.25::numeric(4,1)",
        "SELECT '{abcd}'::varchar(3)[]",
        "SELECT '{1.25}'::numeric(4,1)[]",
        "SELECT '{1,2}'::int4[]",
        "SELECT '10:00'::time(3)",
        // The selector's route again, at a cast.
        "SELECT 1::float(3)",
        // The ends again, at a cast and at a typed constant.
        "SELECT 'ab'::varchar(10485760)",
        "SELECT 1.5::numeric(1000)",
        "SELECT varchar(10485760) 'ab'",
        "SELECT numeric(1000) '1.5'",
    ] {
        assert_eq!(state(&mut db, sql), None, "`{sql}` must be accepted");
    }
    // The lexer's decoded payload and a leading zero both survive the second read the
    // refusal does: `0x3` and `03` are the length `3`, not a malformed component.
    assert_eq!(text(&mut db, "SELECT 'abcd'::char(0x3)"), "abc");
    assert_eq!(text(&mut db, "SELECT 'abcd'::char(03)"), "abc");
    assert_eq!(text(&mut db, "SELECT char(0x3) 'abcd'"), "abc");
    // No cap where none was declared — an unconstrained `varchar[]` stores whole, and
    // an unconstrained `numeric[]` keeps its scale.
    db.execute("INSERT INTO plain (a, b) VALUES ('{abcdefg}', '{1.25}')")
        .expect("store");
    assert_eq!(text(&mut db, "SELECT a FROM plain"), "{abcdefg}");
    assert_eq!(text(&mut db, "SELECT b FROM plain"), "{1.25}");
}

/// The refusal reaches every position that resolves a declared type, at the array
/// spelling as well as the scalar one, and does **not** reach a spelling whose parser
/// arm returns before the `(` — those stay their own `42601`/`42704`, unchanged.
#[test]
fn the_refusal_reaches_every_declared_type_position() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (x INT)").expect("seed");
    db.execute("CREATE TYPE moodq AS ENUM ('ok')")
        .expect("seed");
    for sql in [
        "SELECT 1::int4(3)",
        "SELECT CAST(1 AS int4(3))",
        "CREATE TABLE r1 (a bool(3))",
        "ALTER TABLE t ADD COLUMN y date(3)",
        "SELECT 1::int4(3)[]",
        "CREATE TABLE r2 (a int4(3)[])",
        "ALTER TABLE t ADD COLUMN z bool(3)[]",
        "SELECT 'ok'::moodq(3)",
        "CREATE TABLE r3 (a moodq(3))",
        "CREATE TABLE r4 (a serial(3))",
        "INSERT INTO t VALUES (1::int4(3))",
        "UPDATE t SET x = 1::int4(3)",
        "CREATE INDEX ri ON t ((x::int4(3)))",
        "CREATE TABLE r5 (a INT CHECK (a::int4(3) > 0))",
        "CREATE TABLE r6 (a INT DEFAULT 1::int4(3))",
        // A `WHERE` and a `HAVING`, each over a real `FROM`: the `FROM`-less
        // `SELECT 1 WHERE …` spelling is pglet's own `42601` whatever the cast target,
        // so asserting through *that* one would be vacuous under the gate's removal.
        "SELECT x FROM t WHERE x = 1::int4(3)",
        "SELECT x FROM t GROUP BY x HAVING count(*) > 1::int4(3)",
    ] {
        assert_eq!(state(&mut db, sql), Some("42601"), "`{sql}`");
    }
    // An unrecognized base still answers `42704`: the name resolves before the modifier
    // is judged, which is 18.6's order (`LookupTypeName` then `typenameTypeMod`).
    assert_eq!(state(&mut db, "SELECT 1::nosuchtype(3)"), Some("42704"));
    assert_eq!(state(&mut db, "SELECT 1::nosuchtype(3)[]"), Some("42704"));
}

/// **`--coerce-unknown-types` does not exempt the modifier gate**, and that is the
/// decision rather than an oversight — `crates/pglet_sql/src/planner/unknown_types.rs`'s
/// module doc argues it in full.
///
/// The flag decides what an unrecognized type *name* resolves to (`text`); a modifier's
/// legality is the **resolved** type's fact, and `text` carries none. So the refusal
/// stands with the flag on, and the row-destroying half of `EDGES.md` row 118's second
/// residual is closed on this route: `'t'::regclass(3)` no longer becomes `'t'::text`.
///
/// Both polarities are pinned, because the exemption would flip exactly them: the
/// modifier-less spelling must still coerce and still write (the flag's declared
/// posture, and row 118's surviving residual), and the modifier-bearing one must refuse
/// and leave the rows alone.
///
/// What the refusal is **not** is agreement with 18.6 — see the list's own comment. A
/// spelling 18.6 gives a modifier to (`bit(3)`, `timetz(3)`) is refused here just the
/// same, so the guard reads as the decision it is rather than as conformance.
///
/// The toggle is a thread-local, so every answer is taken before it is reset.
#[test]
fn the_coerce_flag_does_not_exempt_the_modifier_gate() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE dw (a INT)",
        "INSERT INTO dw VALUES (1), (2)",
        "CREATE TABLE dw2 (a INT)",
        "INSERT INTO dw2 VALUES (1), (2)",
    ] {
        db.execute(sql).expect("seed");
    }

    db.set_coerce_unknown_types(true);
    // Refused at a column and at a cast alike — but **not** "whether or not 18.6 has
    // the type", which is the tempting and wrong summary: the verdict follows the
    // *resolved* type (`text`), so it is the same whatever 18.6 answers — and 18.6's
    // answers split three ways across these five spellings. `point(3)` is that same
    // `42601` there; `varchar2(10)` / `nosuchtype(3)` its `42704`; and `bit(3)` — an
    // unmodelled type whose 18.6 `typmodin` is **non-zero** — is a *value* there
    // (`001`), so the fifth entry pins the divergent polarity beside the agreeing ones
    // rather than letting the list read as agreement. Why that divergence is accepted
    // is `planner::unknown_types`'s module doc.
    let refused: Vec<Option<&str>> = [
        "CREATE TABLE ct5 (a varchar2(10))",
        "CREATE TABLE ct3 (a point(3))",
        "SELECT 1::nosuchtype(3)",
        "SELECT 1::regclass(3)",
        "SELECT 1::bit(3)",
    ]
    .into_iter()
    .map(|sql| state(&mut db, sql))
    .collect();
    // `timetz(3)` is the fourth member of the "unmodelled here, modifier-carrying
    // there" set (`bit`, `varbit`, `bpchar` are the other three): the parser now keeps
    // the precision, so the canonical `time with time zone(3)` reaches the gate, the
    // flag resolves its unrecognized base to `text`, and `text` carries no modifier.
    // 18.6 answers the value `01:02:03+00` (measured, `PGTZ=UTC`), so this is a **loud
    // wrong verdict standing in for a silently wrong value** — with the flag off the
    // same spelling is `42704 type "time with time zone" is not supported`, which is
    // pglet's own posture and unaffected.
    let unmodelled_carrier = state(&mut db, "SELECT '01:02:03'::timetz(3)");
    // The row-destroying write, and its still-open modifier-less twin.
    let modified_delete = state(&mut db, "DELETE FROM dw WHERE 't'::regclass(3) = 't'");
    let bare_delete = state(&mut db, "DELETE FROM dw2 WHERE 't'::nosuchtype = 't'");
    // The polarity control: a modifier-less unknown name still coerces to `text`.
    let coerced = state(&mut db, "CREATE TABLE ct8 (a varchar2)");
    let survivors = text(&mut db, "SELECT count(*) FROM dw");
    let deleted = text(&mut db, "SELECT count(*) FROM dw2");
    db.set_coerce_unknown_types(false);

    assert_eq!(refused, vec![Some("42601"); 5], "the flag exempts nothing");
    assert_eq!(
        unmodelled_carrier,
        Some("42601"),
        "an unmodelled type's modifier is refused on the resolved `text`",
    );
    assert_eq!(modified_delete, Some("42601"));
    assert_eq!(survivors, "2", "a refused cast must not destroy rows");
    assert_eq!(coerced, None, "a modifier-less unknown name still coerces");
    assert_eq!(
        text(
            &mut db,
            "SELECT data_type FROM information_schema.columns \
             WHERE table_name = 'ct8' AND column_name = 'a'"
        ),
        "text",
    );
    assert_eq!(bare_delete, None, "row 118's surviving residual");
    assert_eq!(deleted, "0", "…which still deletes both rows");
}
