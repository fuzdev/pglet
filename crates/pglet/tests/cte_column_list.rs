//! A `WITH` query's `name(cols)` column list renames the CTE relation on **every**
//! reader — over the embed engine (parse → plan → execute) on the default CoW-B+tree
//! backend.
//!
//! **Why this sits beside the differential fixtures rather than inside them.** The
//! fixtures compare pglet against 18.6 on two of the three answers: the simple-query
//! `RowDescription` (the executor's `QueryResult::shape`) and `Describe`'s (the planner's
//! `Plan::result_shape`), each live. What no fixture can reach is the **executed
//! relation's own columns** — the third answer, and the one a rename stored only in the
//! parser's plan silently misses. `Plan::SubqueryScan` returns the executed subquery's
//! columns and `Plan::CteScan` returns the `Plan::With` materializer's, neither of which
//! is the plan-side list the binding gate and `Describe` read; that is `EDGES.md` row
//! 59's two-producer class with a *third* producer under it. So each test here asserts
//! the two producers **against each other** over the same statement, which is the
//! comparison nothing in the corpus makes.
//!
//! Both reference kinds are covered because they reach the rename by different routes: a
//! read-only CTE inlines (`Plan::SubqueryScan`, list carried on the node), a
//! data-modifying one materializes (`Plan::CteScan` plan-side, `MaterializedCte`
//! executed-side — two independent derivations of the same relation).

// A test harness: setup `.unwrap()`s, so a setup failure surfaces loudly here rather
// than as a confusing assertion mismatch.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pglet::Database;
use pglet_sql::types::Value;

/// The column names the **planner** answers for `sql` (what `Describe` sends), and the
/// ones the **executor** answers (what the simple-query `RowDescription` sends, and what
/// the rows are named by). Both come off the same `db`; the two must agree.
fn both_producers(db: &mut Database, sql: &str) -> (Vec<String>, Vec<String>) {
    let planned: Vec<String> = {
        let schema = db.schema();
        let plans = pglet_sql::parser::parse(sql, schema).expect("should plan");
        assert_eq!(plans.len(), 1, "expected one statement for {sql:?}");
        plans[0]
            .result_shape(schema)
            .into_columns()
            .into_iter()
            .map(|c| c.name)
            .collect()
    };
    let mut results = db.execute(sql).expect("should execute");
    let executed = results
        .remove(0)
        .shape
        .into_columns()
        .into_iter()
        .map(|c| c.name)
        .collect();
    (planned, executed)
}

fn seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ct (a INT, b TEXT)").unwrap();
    db.execute("INSERT INTO ct VALUES (1, 'x'), (2, 'y')")
        .unwrap();
    db.execute("CREATE TABLE cd (a INT)").unwrap();
    db.execute("INSERT INTO cd VALUES (7)").unwrap();
    db
}

/// A **read-only** CTE's list reaches both producers, and the executed rows are named by
/// it too. `SELECT *` is deliberate: the wildcard expands through the plan-time
/// namespace, a fourth reader of the same list.
#[test]
fn a_read_only_ctes_column_list_reaches_both_producers() {
    let mut db = seeded();
    let (planned, executed) =
        both_producers(&mut db, "WITH c AS (SELECT a, b FROM ct) SELECT * FROM c");
    assert_eq!(
        planned,
        vec!["a".to_string(), "b".to_string()],
        "the control: no list, the body's own names"
    );
    assert_eq!(planned, executed);

    let (planned, executed) = both_producers(
        &mut db,
        "WITH c(p, q) AS (SELECT a, b FROM ct) SELECT * FROM c",
    );
    assert_eq!(planned, vec!["p".to_string(), "q".to_string()]);
    assert_eq!(
        executed, planned,
        "the executed relation's columns are re-derived by executing the subquery — a \
         rename that reached only the plan would show up exactly here"
    );
}

/// A **short** list fills from the body, and the fill is positional: `analyzeCTE` takes
/// `ctecolnames[i]` while `i < numaliases` and `te->resname` beyond.
#[test]
fn a_short_column_list_fills_from_the_body_on_both_producers() {
    let mut db = seeded();
    let (planned, executed) = both_producers(
        &mut db,
        "WITH c(p) AS (SELECT a, b FROM ct) SELECT * FROM c",
    );
    assert_eq!(planned, vec!["p".to_string(), "b".to_string()]);
    assert_eq!(executed, planned);
}

/// A **data-modifying** CTE's list reaches the plan-side `Plan::CteScan` and the
/// executed `MaterializedCte` — two derivations that share no code, which is why the
/// list is applied at both.
#[test]
fn a_data_modifying_ctes_column_list_reaches_both_producers() {
    let mut db = seeded();
    let (planned, executed) = both_producers(
        &mut db,
        "WITH dd(gone) AS (DELETE FROM cd RETURNING a) SELECT * FROM dd",
    );
    assert_eq!(planned, vec!["gone".to_string()]);
    assert_eq!(executed, planned);
}

/// The list renames — it does not *retype*. `analyzeCTE` reads the column types from
/// `exprType(te->expr)` and never from the list, so a renamed column keeps the body's
/// type and the body's value.
#[test]
fn a_column_list_renames_without_retyping() {
    let mut db = Database::with_btree();
    let mut results = db
        .execute("WITH c(v, w) AS (SELECT 'abc'::text, 5::int8) SELECT * FROM c")
        .unwrap();
    let result = results.remove(0);
    let columns = result.shape.into_columns();
    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["v", "w"]
    );
    assert_eq!(
        columns.iter().map(|c| c.type_oid).collect::<Vec<_>>(),
        vec![
            pglet_sql::types::TypeOid::Text,
            pglet_sql::types::TypeOid::Int8
        ]
    );
    assert_eq!(
        result.rows,
        vec![vec![Value::Text("abc".into()), Value::Int8(5)]]
    );
}

/// The body's own names are **gone** once a list renames them, on the reader that binds
/// a column reference at plan time. The reverse polarity of the rename, and the silent
/// half of the pre-close divergence: `SELECT a FROM c` used to answer rows.
#[test]
fn the_bodys_own_names_are_gone() {
    let mut db = seeded();
    assert!(
        db.execute("WITH c(p) AS (SELECT a FROM ct) SELECT a FROM c")
            .is_err(),
        "the body's name must not resolve through the renamed relation"
    );
    assert!(
        db.execute("WITH c(p) AS (SELECT a FROM ct) SELECT p FROM c")
            .is_ok(),
        "the control: the list's own name resolves"
    );
}

/// A **genuinely recursive** CTE's descriptors are the one list `recursive_cte_columns`
/// rebuilds — and a rebuild that cannot carry a fact drops it. `ColumnDesc::new` answers
/// `-1` for `type_modifier` and `None` for `enum_type`, so the seed's `VARCHAR(5)` came
/// back unbounded and its enum column came back as bare `text`.
///
/// **Why here.** No fixture can see the `type_modifier` half: the differential legs
/// compare each column's declared **`type_oid`** (`Expected::compare_declared_types` is
/// a bare `a.type_oid != b.type_oid`) and the harness compares no typmod anywhere. The
/// `enum_type` half is invisible for a second reason — it is a pglet-internal field, and
/// what does reach the wire is `text`, an enum's `type_oid` here — and what a fixture
/// *can* see of it is the behaviour it decides, which
/// `tests/fixtures/enum/recursive_cte_*` pins. This asserts the descriptors themselves,
/// on **both** producers, because the plan-side list feeds the gates and the executed
/// one feeds `MaterializedCte`'s comparator.
///
/// The **non**-recursive CTE beside each case is the control: it propagates rather than
/// rebuilds and is right either way, which is what makes the rebuild
/// the condition.
///
/// # And two fields the rebuild must keep dropping
///
/// `table_oid`/`column_id` are the reason this is a field-by-field decision rather than
/// a `..c` propagation. A recursive CTE's column is the set operation of its two terms,
/// so it has no single source column, and 18.6 sends `0`/`0` for it — measured on a raw
/// `RowDescription`, against the *non*-recursive CTE in the same session, where both
/// engines send the base table's oid and attnum. Propagating the whole descriptor is the
/// obvious repair and it is **wrong**: it would trade this divergence for a new one, on
/// a field no fixture compares either. So the control's half of this assertion is that
/// the provenance is **present** there and absent here.
/// One column's `(type_modifier, enum_type, has-provenance)`, per column of a result.
type Facts = Vec<(i32, Option<String>, bool)>;

/// `(type_modifier, enum_type, has-provenance)` per column, from **each** producer, for
/// one statement — the plan-side list `Describe` sends and the executed one the
/// simple-query `RowDescription` and `MaterializedCte`'s comparator read.
fn descriptor_facts(db: &mut Database, sql: &str) -> (Facts, Facts) {
    let read = |c: pglet_sql::executor::ColumnDesc| {
        // `table_oid` and `column_id` move together — a `0` oid means there is no
        // source column to number — so one flag is the whole fact, and asserting it
        // as a flag keeps the test off the catalog's oid allocation.
        assert_eq!(
            c.table_oid == 0,
            c.column_id == 0,
            "table_oid and column_id must agree about whether there is a source column"
        );
        (c.type_modifier, c.enum_type, c.table_oid != 0)
    };
    let planned: Facts = {
        let schema = db.schema();
        let plans = pglet_sql::parser::parse(sql, schema).expect("should plan");
        assert_eq!(plans.len(), 1, "expected one statement for {sql:?}");
        plans[0]
            .result_shape(schema)
            .into_columns()
            .into_iter()
            .map(read)
            .collect()
    };
    let executed = db
        .execute(sql)
        .expect("should execute")
        .remove(0)
        .shape
        .into_columns()
        .into_iter()
        .map(read)
        .collect();
    (planned, executed)
}

/// `descriptor_facts` for a statement 18.6 **refuses**, where pglet may answer: the
/// facts both producers declare when pglet answers — asserted equal — and `None` when
/// both refuse it with 18.6's own `42804`.
///
/// The refusal is admitted on purpose. Every statement passed here is one 18.6 raises
/// `42804 … in non-recursive term but type … overall` over, and pglet's over-acceptance of
/// it is recorded (`EDGES.md` row 155 and `docs/declared_types.md`) rather than required.
/// A guard that asserted the answer would redden the day that refusal lands, which is the
/// correct direction; so what these assert is the property both outcomes satisfy — pglet
/// **either** refuses as 18.6 does **or** declares a modifier its rows satisfy.
fn facts_unless_refused(db: &mut Database, sql: &str) -> Option<Facts> {
    let refused = {
        let schema = db.schema();
        pglet_sql::parser::parse(sql, schema)
            .err()
            .map(|e| e.sqlstate().to_string())
    };
    if let Some(code) = refused {
        assert_eq!(
            code, "42804",
            "{sql}: the only refusal admitted is 18.6's own"
        );
        let err = db
            .execute(sql)
            .expect_err("the executor must refuse what the planner refused");
        assert_eq!(err.sqlstate(), "42804", "{sql}");
        return None;
    }
    let (planned, executed) = descriptor_facts(db, sql);
    assert_eq!(executed, planned, "{sql}: both producers are one list here");
    Some(planned)
}

/// **A conforming recursive CTE's rebuilt descriptors carry the seed's typmod and enum.**
/// The recursive term here reads the working-table columns back, so both terms declare the
/// seed's modifier and the set operation keeps it — which is the condition under which the
/// seed's modifier is the answer at all. Where the terms disagree it is not:
/// `a_cast_typed_recursive_terms_declared_modifier_is_one_its_rows_satisfy` and
/// `a_column_typed_recursive_term_folds_what_pglet_reads_exactly` pin that half.
#[test]
fn a_conforming_recursive_ctes_descriptors_carry_the_seeds_typmod_and_enum() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')",
        "CREATE TABLE rt (v VARCHAR(5) NOT NULL, n NUMERIC(6,2) NOT NULL, m mood NOT NULL)",
        "INSERT INTO rt VALUES ('ab', 1.25, 'ok')",
    ] {
        db.execute(sql).unwrap();
    }

    let facts = descriptor_facts;

    // `varchar(5)` is atttypmod 9 and `numeric(6,2)` is 393222 — the `+ 4` and
    // `((p << 16) | s) + 4` encodings a width-deriving client reads.
    let recursive = vec![
        (9, None, false),
        (393_222, None, false),
        (-1, Some("mood".to_string()), false),
    ];
    // The control differs in exactly one field, and it is the one that must *not* carry.
    let non_recursive: Facts = recursive
        .iter()
        .cloned()
        .map(|(m, e, _)| (m, e, true))
        .collect();

    let (planned, executed) = facts(
        &mut db,
        "WITH RECURSIVE r(a, b, z) AS (\
           SELECT v, n, m FROM rt UNION ALL SELECT a, b, z FROM r WHERE false\
         ) SELECT a, b, z FROM r",
    );
    assert_eq!(planned, recursive, "the plan-side list the gates read");
    assert_eq!(
        executed, recursive,
        "and the executed one the comparator reads — they are one list here, and an \
         assertion on only one of them would not say so"
    );

    let (planned, executed) = facts(
        &mut db,
        "WITH r(a, b, z) AS (SELECT v, n, m FROM rt) SELECT a, b, z FROM r",
    );
    assert_eq!(
        planned, non_recursive,
        "the control: a non-recursive CTE propagates the descriptor whole, provenance \
         included — which is what makes the recursion, and not the CTE, the condition"
    );
    assert_eq!(executed, non_recursive);
}

/// **Over a cast-typed recursive term, a recursive CTE declares no type modifier its own
/// rows need not satisfy.** A term whose column is read through a window query or a
/// qualified `unnest` column is the other route to the fold, and
/// `a_recursive_terms_column_read_through_a_window_or_an_unnest_folds_its_own_modifier`
/// pins it.
///
/// The rebuilt descriptors start as the seed's, and taking the seed's *modifier* whole
/// is wrong the moment the recursive term can produce a value outside it: over a
/// `VARCHAR(5)` seed, `UNION ALL SELECT 'abcdefgh'::varchar(8)` answered both rows under
/// a declared `character varying(5)` — a width-deriving client reading `5` off a
/// `RowDescription` whose next `DataRow` is eight characters. 18.6 never has to answer
/// that question because it refuses the pair (`42804 … has type character varying(5) in
/// non-recursive term but type character varying overall`, measured at every spelling
/// below); pglet accepts it, so what it declares has to be honest.
///
/// So the modifier is **resolved** by `select_common_typmod` — the seed's while both
/// terms declare it, `-1` otherwise — and this pins both directions. The conforming case
/// is the one that must **keep** its modifier, and it is asserted here as well as in the
/// test above, because a repair that simply stopped carrying the modifier would satisfy
/// the widening half alone.
///
/// # Why this is a Rust test
///
/// No fixture can see a type modifier: both differential legs compare name + OID only
/// (`tests/README.md` § Declared column types), so every assertion here is invisible to
/// the whole corpus. And the widening statement is one 18.6 **refuses**, so a faithful
/// fixture over it is red whatever pglet declares.
///
/// # And pglet does not refuse
///
/// Raising 18.6's `42804` on the modifier was implemented and measured against 18.6
/// first: it refused `a::varchar(5)`, `'xy'::varchar(5)` and `1.5::numeric(6,2)` against
/// a seed of the same width, and `CASE`/`COALESCE`/`GREATEST`/`(SELECT a)` over the
/// working-table column — each of which 18.6 **answers**, because pglet reads `-1` for a
/// modifier 18.6 knows exactly (`EDGES.md` row 155). The members are
/// `a_modifier_comparison_would_wrongly_refuse_these`'s table.
///
/// The disagreeing statements go through `facts_unless_refused`, which admits 18.6's own
/// refusal as well as the fold's `-1`: 18.6 refuses each of them, and a future pglet that
/// refuses them too has closed a divergence, not broken this rule. The two statements 18.6
/// answers — the conforming one and the undeclared seed — must execute.
///
/// # What this cannot see
///
/// Every recursive term below is a **cast**, and pglet reads a cast's modifier as `-1`
/// (row 155) — so the fold here only ever meets a `-1` on the recursive side. A fold that
/// kept the seed's modifier *unless the recursive term reads `-1`* passes every assertion
/// in this test. `a_column_typed_recursive_term_folds_what_pglet_reads_exactly` is the
/// guard against that fold, over terms whose modifier pglet reads exactly.
#[test]
fn a_cast_typed_recursive_terms_declared_modifier_is_one_its_rows_satisfy() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE rm (v VARCHAR(5) NOT NULL, n NUMERIC(6,2) NOT NULL, t TEXT NOT NULL)",
        "INSERT INTO rm VALUES ('ab', 1.25, 'ab')",
    ] {
        db.execute(sql).unwrap();
    }
    let unbounded: Facts = vec![(-1, None, false)];

    // The **widening** direction, and the one the narrowing-only example hid: the
    // recursive term's value is outside the seed's declared width.
    let widening = "WITH RECURSIVE r(a) AS (\
         SELECT v FROM rm UNION ALL SELECT 'abcdefgh'::varchar(8) FROM r WHERE a = 'ab'\
       ) SELECT a FROM r";
    if let Some(planned) = facts_unless_refused(&mut db, widening) {
        assert_eq!(
            planned, unbounded,
            "a recursive term that can widen the column must not leave the seed's modifier \
             declared — the plan-side list is what `Describe` sends"
        );
        // The rows are what make the declared modifier a claim rather than a formality.
        let rows = db.execute(widening).expect("should execute").remove(0).rows;
        assert_eq!(
            rows,
            vec![
                vec![Value::Text("ab".into())],
                vec![Value::Text("abcdefgh".into())]
            ],
            "the eight-character row is the one a declared `varchar(5)` would have lied about"
        );
    }

    // The **narrowing** direction — the fold is symmetric, so this is `-1` too, and it
    // is the direction whose value fits and therefore shows nothing on its own.
    let narrowing = facts_unless_refused(
        &mut db,
        "WITH RECURSIVE r(a) AS (\
           SELECT v FROM rm UNION ALL SELECT 'xy'::varchar(3) FROM r WHERE false\
         ) SELECT a FROM r",
    );
    assert!(
        narrowing.as_ref().is_none_or(|f| f == &unbounded),
        "{narrowing:?}"
    );

    // A `numeric`'s modifier is a scale as well as a precision, and the recursive term
    // carries five fractional digits against the seed's two.
    let numeric = "WITH RECURSIVE r(a) AS (\
         SELECT n FROM rm UNION ALL SELECT 1.23456::numeric(10,5) FROM r WHERE a = 1.25\
       ) SELECT a FROM r";
    if let Some(planned) = facts_unless_refused(&mut db, numeric) {
        assert_eq!(planned, unbounded);
        let rows = db.execute(numeric).expect("should execute").remove(0).rows;
        assert_eq!(
            rows.len(),
            2,
            "both terms' rows flow under the declared type"
        );
    }

    // **The conforming case keeps its modifier.** Reverting the carry entirely — the
    // obvious way to satisfy every assertion above — reddens exactly here.
    let (planned, executed) = descriptor_facts(
        &mut db,
        "WITH RECURSIVE r(a) AS (\
           SELECT v FROM rm UNION ALL SELECT a FROM r WHERE false\
         ) SELECT a FROM r",
    );
    assert_eq!(
        planned,
        vec![(9, None, false)],
        "both terms declare `varchar(5)`, so the set operation's modifier is `9` — what \
         18.6 sends for this statement"
    );
    assert_eq!(executed, planned);

    // And a seed that declares **no** modifier cannot acquire one from the recursive
    // term: the fold's `-1` is the seed's own, which is 18.6's answer here too.
    let (planned, executed) = descriptor_facts(
        &mut db,
        "WITH RECURSIVE r(a) AS (\
           SELECT t FROM rm UNION ALL SELECT 'xy'::varchar(3) FROM r WHERE false\
         ) SELECT a FROM r",
    );
    assert_eq!(planned, unbounded);
    assert_eq!(executed, planned);
}

/// **The fold is over what pglet reads, and a base column projected directly is read
/// exactly** — and so is one read through a window query or a qualified `unnest` column,
/// which is the next test's half.
///
/// The guard the cast-typed test above cannot be: a recursive term that reads a **base
/// column** carries that column's own modifier into `parser::select`'s `check_recursive_shape` fold, so
/// the fold meets two *known* modifiers and has to decide between them. Over
/// `rk(v VARCHAR(5), v8 VARCHAR(8), c3 VARCHAR(3), n NUMERIC(6,2), n10 NUMERIC(10,5),
/// tz TIMESTAMPTZ(3), ts TIMESTAMP(3))`, measured over the raw wire on both legs of the
/// native server against 18.6:
///
/// - `v` seed beside `v8` (**widening**), `c3` (**narrowing**), and `n` beside `n10`
///   (scale widening) — 18.6 refuses each `42804 … has type character varying(5) in
///   non-recursive term but type character varying overall` (`numeric(6,2)` for the
///   third); pglet answers and declares `-1`, which every row satisfies. The widening one
///   is the shape that matters: its second row is eight characters long.
/// - `tz` seed beside `ts` — the modifiers agree as *numbers* (`3`, `3`) over two types,
///   which is `select_common_typmod`'s `oid != common` arm; 18.6 refuses, pglet declares
///   `-1`.
/// - `v` beside `v` — the conforming control, `9` on both engines.
///
/// Each disagreeing statement goes through `facts_unless_refused` for the reason that
/// helper states. Each of three plausible wrong folds reddens here, measured one at a
/// time: keeping the seed's modifier **unless the recursive term reads `-1`** (declares
/// `9` over the eight-character row — green in the cast-typed test above, which is why
/// this test exists), keeping the seed's modifier **always** (the same `9`, and red above
/// too), and dropping `select_common_typmod`'s `oid != common` arm (declares `3` for the
/// timestamp pair). The conforming control reddens a fold that always answers `-1`.
#[test]
fn a_column_typed_recursive_term_folds_what_pglet_reads_exactly() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE rk (v VARCHAR(5) NOT NULL, v8 VARCHAR(8) NOT NULL, c3 VARCHAR(3) NOT NULL, \
         n NUMERIC(6,2) NOT NULL, n10 NUMERIC(10,5) NOT NULL, \
         tz TIMESTAMPTZ(3) NOT NULL, ts TIMESTAMP(3) NOT NULL)",
        "INSERT INTO rk VALUES ('ab', 'abcdefgh', 'xyz', 1.25, 1.23456, \
         '2020-01-01 00:00:00.123+00', '2020-01-01 00:00:00.456')",
    ] {
        db.execute(sql).unwrap();
    }
    let unbounded: Facts = vec![(-1, None, false)];
    let honest = |db: &mut Database, seed: &str, term: &str, until: &str| {
        let sql = format!(
            "WITH RECURSIVE r(a) AS (SELECT {seed} FROM rk UNION ALL SELECT {term} FROM rk, r WHERE {until}) SELECT a FROM r"
        );
        let facts = facts_unless_refused(db, &sql);
        assert!(
            facts.as_ref().is_none_or(|f| f == &unbounded),
            "{seed} beside {term}: refuse as 18.6 does or declare no modifier — got {facts:?}"
        );
        facts.is_some()
    };

    // The widening pair first, and its rows: the second is eight characters long, which is
    // what a declared `varchar(5)` would misdescribe.
    if honest(&mut db, "v", "v8", "a = 'ab'") {
        let rows = db
            .execute(
                "WITH RECURSIVE r(a) AS (SELECT v FROM rk UNION ALL SELECT v8 FROM rk, r WHERE a = 'ab') SELECT a FROM r",
            )
            .expect("should execute")
            .remove(0)
            .rows;
        assert_eq!(
            rows,
            vec![
                vec![Value::Text("ab".into())],
                vec![Value::Text("abcdefgh".into())]
            ]
        );
    }
    honest(&mut db, "v", "c3", "a = 'ab'");
    honest(&mut db, "n", "n10", "a = 1.25");
    honest(&mut db, "tz", "ts", "false");

    // The conforming control: both terms read the same `varchar(5)` column, and 18.6
    // answers with `9`.
    let (planned, executed) = descriptor_facts(
        &mut db,
        "WITH RECURSIVE r(a) AS (SELECT v FROM rk UNION ALL SELECT v FROM rk, r WHERE false) SELECT a FROM r",
    );
    assert_eq!(planned, vec![(9, None, false)]);
    assert_eq!(executed, planned);
}

/// **The surviving `42804` names each type's width, as 18.6 does.**
///
/// The modifier fold raises nothing, but the recursive CTE's *type*-mismatch `42804` is
/// still reachable with a seed that declares a modifier, and 18.6 spells **both** halves
/// of that sentence through `format_type_with_typemod`. Measured on the oracle over
/// `mg(n NUMERIC(6,2), f FLOAT8)`:
/// `recursive query "r" column 1 has type numeric(6,2) in non-recursive term but type
/// double precision overall`. pglet's renderer used to print a bare `numeric` at the
/// first half, which made the sentence name a type it does not have.
///
/// This pins the **composition** — that the message actually reads the renderer — which
/// `a_type_and_its_atttypmod_spell_as_18_6_spells_them` cannot see, it testing the
/// renderer alone. No fixture can pin it either: pglet's `M` carries an `Error::Type`
/// `type error: ` prefix 18.6 does not send, so `@expect-message` over this statement is
/// red for a reason that has nothing to do with the width.
#[test]
fn a_recursive_ctes_type_mismatch_names_the_width() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE mg (n NUMERIC(6,2) NOT NULL, f FLOAT8 NOT NULL)",
        "INSERT INTO mg VALUES (1.25, 1.5)",
    ] {
        db.execute(sql).unwrap();
    }
    let err = db
        .execute(
            "WITH RECURSIVE r(a) AS (SELECT n FROM mg UNION ALL SELECT f FROM mg, r WHERE false) SELECT a FROM r",
        )
        .expect_err("the recursive term's type is not the seed's");
    let message = err.to_string();
    assert!(
        message.contains(concat!(
            "recursive query \"r\" column 1 has type numeric(6,2) ",
            "in non-recursive term but type double precision overall"
        )),
        "the seed's half must carry its width, as 18.6's does — got {message:?}"
    );
}

/// **The statements a modifier *comparison* would refuse, and 18.6 answers.**
///
/// The obvious repair for the row above is 18.6's own: compare the resolved modifier
/// against the seed's and raise `42804` when they differ. It was implemented and measured
/// against the oracle before the fold was chosen, and it refused every statement here —
/// each of which 18.6 executes, declaring the seed's own modifier. The cause is
/// `EDGES.md` row 155: pglet reads
/// `-1` for a `::` cast's modifier and for a value constructor's propagated one, so the
/// comparison would be over a value this engine cannot compute.
///
/// So this is the **accept** polarity of that decision, and the guard that reddens the day
/// someone adds the comparison. What it asserts is what the decision owns: each statement
/// executes, both producers declare one list, and the declared modifier is `-1` — which
/// pglet declares today because it reads `-1` for the term (row 155's under-declaration)
/// — or the seed's own, which is what 18.6 declares for each of these. It deliberately does
/// **not** pin the `-1`: a repair of row 155 that makes these declare the seed's modifier
/// is 18.6's answer, not a regression, and this test stays green under it.
#[test]
fn a_modifier_comparison_would_wrongly_refuse_these() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE rc (v VARCHAR(5) NOT NULL, n NUMERIC(6,2) NOT NULL)",
        "INSERT INTO rc VALUES ('ab', 1.25)",
    ] {
        db.execute(sql).unwrap();
    }
    // Each is `<seed column>` beside a recursive term whose modifier 18.6 knows and
    // pglet reads as `-1` — a same-width cast, or a constructor over the working-table
    // column whose arms agree.
    for (seed, term) in [
        ("v", "a::varchar(5)"),
        ("v", "'xy'::varchar(5)"),
        ("n", "1.5::numeric(6,2)"),
        ("v", "CASE WHEN true THEN a ELSE a END"),
        ("v", "COALESCE(a, a)"),
        ("v", "GREATEST(a, a)"),
        ("v", "(SELECT a)"),
        ("n", "COALESCE(a, a)"),
        // `NULLIF` carries its **first argument's** modifier where the result type
        // survives (`exprTypmod`'s `T_NullIfExpr` arm): `NULLIF(n, n)` over a
        // `numeric(6,2)` is `393222` on 18.6. Its `varchar` spelling is *not* a member,
        // because 18.6 refuses it: `42804 recursive query "r" column 1 has type character
        // varying(5) in non-recursive term but type character varying overall` — both
        // halves `character varying`, so the refusal is on the **modifier**, the term's
        // being `-1` there. pglet answers it, which is the recursive CTE's recorded
        // over-acceptance (`docs/declared_types.md`), not a statement this table admits.
        ("n", "NULLIF(a, a)"),
    ] {
        let sql = format!(
            "WITH RECURSIVE r(a) AS (               SELECT {seed} FROM rc UNION ALL SELECT {term} FROM r WHERE false             ) SELECT a FROM r"
        );
        let (planned, executed) = descriptor_facts(&mut db, &sql);
        assert_eq!(executed, planned, "{term}: both producers declare one list");
        let seeds = if seed == "v" { 9 } else { 393_222 };
        assert!(
            matches!(planned.as_slice(), [(m, None, false)] if *m == -1 || *m == seeds),
            "{term}: the declared modifier is `-1` or the seed's own `{seeds}` — got {planned:?}"
        );
    }
}

/// **A recursive term that reads a column through a window query or a FROM-clause
/// `unnest` folds that column's own modifier.**
///
/// The fold is only as good as what the planner reads for each term, and two readers
/// handed it another relation's column where the FROM repeated a name: a window query's
/// projected `b.v` took the first input column named `v`, and a qualified `u.v` over an
/// `unnest` fell through to the flat column list. Over `t(v VARCHAR(5))` and
/// `t3(v VARCHAR(8))`, each declared `character varying(5)` over the eight-character row
/// here — where 18.6 refuses the statement `42804` — because the fold saw `9` on both
/// sides.
///
/// Both polarities are pinned, and the second is what separates the repair from a fold
/// that merely stops trusting a derived column: beside a `VARCHAR(8)` seed the same window
/// term declares `12`, which is 18.6's answer, and a fold that declared `-1` for any term
/// read through a derived table is red there.
#[test]
fn a_recursive_terms_column_read_through_a_window_or_an_unnest_folds_its_own_modifier() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE t (v VARCHAR(5) NOT NULL)",
        "CREATE TABLE t3 (v VARCHAR(8) NOT NULL)",
        "INSERT INTO t VALUES ('ab')",
        "INSERT INTO t3 VALUES ('abcdefgh')",
    ] {
        db.execute(sql).unwrap();
    }
    let unbounded: Facts = vec![(-1, None, false)];
    let window_term =
        "SELECT x.v FROM (SELECT b.v, row_number() OVER () AS rn FROM t a, t3 b) x, r";
    let unnest_term = "SELECT u.v FROM t, unnest(ARRAY['abcdefgh']) AS u(v), r";
    for term in [window_term, unnest_term] {
        let sql = format!(
            "WITH RECURSIVE r(a) AS (SELECT v FROM t UNION ALL {term} WHERE r.a = 'ab') SELECT a FROM r"
        );
        let facts = facts_unless_refused(&mut db, &sql);
        assert!(
            facts.as_ref().is_none_or(|f| f == &unbounded),
            "{term}: refuse as 18.6 does or declare no modifier — got {facts:?}"
        );
        if facts.is_some() {
            let rows = db.execute(&sql).expect("should execute").remove(0).rows;
            assert!(
                rows.contains(&vec![Value::Text("abcdefgh".into())]),
                "{term}: the eight-character row is what a declared `varchar(5)` misdescribes"
            );
        }
    }

    // The conforming polarity: a `VARCHAR(8)` seed beside the window term reading `t3.v`.
    let (planned, executed) = descriptor_facts(
        &mut db,
        &format!(
            "WITH RECURSIVE r(a) AS (SELECT v FROM t3 UNION ALL {window_term} WHERE false) SELECT a FROM r"
        ),
    );
    assert_eq!(planned, vec![(12, None, false)], "18.6 declares `12`");
    assert_eq!(executed, planned);

    // A **seed** spelled through the `unnest`: 18.6 types it `text`, so the column declares
    // `text` with no modifier; the flat fallback typed it as `t.v`, `varchar(5)`.
    let sql = "WITH RECURSIVE r(a) AS (SELECT u.v FROM t, unnest(ARRAY['abcdefgh']) AS u(v) \
               UNION ALL SELECT v FROM t, r WHERE r.a = 'zz') SELECT a FROM r";
    let (planned, executed) = descriptor_facts(&mut db, sql);
    assert_eq!(planned, unbounded);
    assert_eq!(executed, planned);
    let schema = db.schema();
    let plans = pglet_sql::parser::parse(sql, schema).expect("should plan");
    let declared: Vec<_> = plans[0]
        .result_shape(schema)
        .into_columns()
        .into_iter()
        .map(|c| c.type_oid)
        .collect();
    assert_eq!(declared, vec![pglet_sql::types::TypeOid::Text]);
}
