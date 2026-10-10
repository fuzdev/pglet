//! The column-`DEFAULT` verdict surface — the half the differential fixtures
//! structurally cannot reach.
//!
//! `@expect-error` compares **SQLSTATE only**, so every pair of DEFAULT refusals
//! sharing `0A000` — subquery vs column reference, set-returning vs column reference
//! — is invisible to a fixture: one arrangement's fixture stays green with the
//! other's verdict. These assert the **full message text**, which is what separates
//! them, and they are the only guard for those rules.
//!
//! **They live here, over a `Database`, because the verdicts are the executor's.**
//! 18.6 raises every one of them in parse analysis, behind the statement-level gates
//! a parse-time refusal would jump in front of, so `executor::ddl`'s
//! `cook_column_default` is where they are decided — and reaching it needs a real
//! statement execution, not a parse.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface a setup failure or an
// unexpected verdict shape loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pglet::Database;
use pglet_sql::catalog::ColumnDefault;
use pglet_sql::types::Value;

/// A fresh database holding `t (a INT, b INT)` — the relation the `ALTER` route's
/// statements name, so a missing-relation `42P01` (which outranks every DEFAULT
/// verdict on 18.6, measured) never stands in for the verdict under test.
fn seeded() -> Database {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (a INT, b INT)").expect("seeds");
    db
}

/// The `(SQLSTATE, message)` a `CREATE TABLE` fails with, or `None` when it
/// succeeds. Deliberately **not** seeded: the statement creates `t` itself, and a
/// pre-existing one would answer `42P07` — which outranks the DEFAULT verdict on
/// both engines.
fn err(sql: &str) -> Option<(&'static str, String)> {
    Database::with_btree()
        .execute(sql)
        .err()
        .map(|e| (e.sqlstate(), e.to_string()))
}

/// [`err`]'s `ALTER` twin, over a database that already holds `t`.
fn alter_err(sql: &str) -> Option<(&'static str, String)> {
    seeded()
        .execute(sql)
        .err()
        .map(|e| (e.sqlstate(), e.to_string()))
}

/// The stored `DEFAULT` of the last column of a `CREATE TABLE`.
fn parsed_default(sql: &str) -> Option<ColumnDefault> {
    let mut db = Database::with_btree();
    db.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    db.schema().tables["t"].columns.last()?.default.clone()
}

/// The `Value::Numeric` a decimal literal stores, spelled as the source text so the
/// display scale (`1.5` vs `1.50`) is part of the assertion.
fn numeric(text: &str) -> Value {
    Value::Numeric(pglet_sql::types::PgNumeric::parse(text).expect("a numeric literal"))
}

/// A `CREATE TABLE` whose last column carries `default`.
///
/// The defaulted column is **`TEXT`**, and the type is load-bearing: `cook_column_default`
/// runs the target type's input function on an unknown literal at DDL time, as 18.6
/// does, so `DEFAULT 'x'` into an `INT` is `22P02` on both engines and could not be a
/// corpus entry here. `text` accepts every literal spelling below, which keeps these
/// tests about the verdict walk rather than about coercion.
fn create_with_default(default: &str) -> String {
    format!("CREATE TABLE t (a INT, b TEXT DEFAULT {default})")
}

/// The `ColumnDefault` an `ALTER TABLE … ALTER COLUMN … SET DEFAULT <d>` records —
/// the second, `a_expr`-grammar caller of the DEFAULT parser.
fn set_default_of(default: &str) -> Option<ColumnDefault> {
    let mut db = seeded();
    let sql = format!("ALTER TABLE t ALTER COLUMN b SET DEFAULT {default}");
    db.execute(&sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    db.schema().tables["t"].columns[1].default.clone()
}

/// **Subquery and column reference share `0A000` and differ only in their
/// message**, so the positional rule between them is unpinnable by a fixture.
/// Measured on 18.6: `(SELECT 1) + a` is `cannot use subquery in DEFAULT
/// expression`, `a + (SELECT 1)` is `cannot use column reference in DEFAULT
/// expression`.
#[test]
fn subquery_and_column_ref_are_positional_and_differ_only_by_message() {
    assert_eq!(
        err(&create_with_default("((SELECT 1) + a)")),
        Some((
            "0A000",
            "cannot use subquery in DEFAULT expression".to_string()
        ))
    );
    assert_eq!(
        err(&create_with_default("(a + (SELECT 1))")),
        Some((
            "0A000",
            "cannot use column reference in DEFAULT expression".to_string()
        ))
    );
}

/// A subquery-bearing node is **opaque**: `transformSubLink` raises before it
/// transforms the sub-select *or* the `testexpr`, so `a IN (SELECT 1)` — whose
/// left operand is a column reference — is the subquery message, not the column
/// one. Measured on 18.6. Descending into `InSubquery`'s left operand (which the
/// shared child visitor does) reverses this.
#[test]
fn an_in_subquery_does_not_report_its_own_left_operand() {
    assert_eq!(
        err(&create_with_default("(a IN (SELECT 1))")),
        Some((
            "0A000",
            "cannot use subquery in DEFAULT expression".to_string()
        ))
    );
    assert_eq!(
        err(&create_with_default("EXISTS (SELECT 1)")),
        Some((
            "0A000",
            "cannot use subquery in DEFAULT expression".to_string()
        ))
    );
}

/// **Set-returning and column reference share `0A000` too.** `unnest` is the one
/// set-returning function pglet's expression grammar parses, so it is the only
/// reproduction of this rule (`generate_series` is a FROM item only, and in an
/// expression is `0A000 function not supported` here against 18.6's set-returning
/// `0A000` — recorded, not pinned). Measured on 18.6:
/// `unnest(ARRAY[1,2]) + a` is the set-returning message, `a + unnest(…)` the
/// column-reference one.
#[test]
fn set_returning_and_column_ref_are_positional_and_differ_only_by_message() {
    assert_eq!(
        err(&create_with_default("(unnest(ARRAY[1,2]) + a)")),
        Some((
            "0A000",
            "set-returning functions are not allowed in DEFAULT expressions".to_string()
        ))
    );
    assert_eq!(
        err(&create_with_default("(a + unnest(ARRAY[1,2]))")),
        Some((
            "0A000",
            "cannot use column reference in DEFAULT expression".to_string()
        ))
    );
    // The bare form, with no column anywhere, still reports set-returning.
    assert_eq!(
        err(&create_with_default("(unnest(ARRAY[1,2]))")),
        Some((
            "0A000",
            "set-returning functions are not allowed in DEFAULT expressions".to_string()
        ))
    );
}

/// `count(*)`'s star is spelled `Expr::Column { table: None, name: "*" }` in this
/// IR, so an unconditional column-reference verdict would fire on it and answer
/// `0A000` where 18.6 says `42803`. **The exemption belongs to the aggregate, not to
/// the star** — an `Expr::Aggregate` skips a direct bare-star argument and walks
/// every other one, so `count(t.*)`, `count(a)` and a bare `t.*` are all
/// `cannot use column reference` on 18.6 and here.
///
/// Keying it on the `Column` node instead — the shape this had before the `Expr`
/// catch-all existed — exempted the star **wherever it appeared**, which was
/// harmless only while every general DEFAULT expression was refused anyway. With the
/// catch-all it becomes an accept-then-brick: `DEFAULT "*"` and `DEFAULT ("*" + 1)`
/// would pass the walk and persist a column reference into the default, and every
/// write would then fail to resolve it. 18.6 answers `0A000 cannot use column
/// reference` to both (measured), and the last block below is the guard.
#[test]
fn only_the_bare_star_is_exempt_from_the_column_reference_verdict() {
    assert_eq!(
        err(&create_with_default("(count(*))")),
        Some((
            "42803",
            "aggregate functions are not allowed in DEFAULT expressions".to_string()
        ))
    );
    for spelling in ["(count(t.*))", "(t.*)", "(count(a))"] {
        assert_eq!(
            err(&create_with_default(spelling)),
            Some((
                "0A000",
                "cannot use column reference in DEFAULT expression".to_string()
            )),
            "{spelling}"
        );
    }
    // **A quoted `"*"` still forges the exemption where it is an aggregate's own
    // argument.** `Token::name` takes a quoted identifier verbatim, so a user's
    // `"*"` column is spelled `Expr::Column { table: None, name: "*" }` —
    // byte-identical to `count(*)`'s star. 18.6 answers `0A000 cannot use column
    // reference in DEFAULT expression` (measured); pglet answers the aggregate's
    // error. Loud, not silent, and the same quoted-forgery class as the `"a.b"` /
    // `"PUBLIC"` instances recorded in `docs/scope.md` / `docs/sequences.md`. Closing it needs a dedicated
    // star marker in the IR rather than a name — recorded in `EDGES.md`.
    assert_eq!(
        err(&create_with_default(r#"(count("*"))"#)),
        Some((
            "42803",
            "aggregate functions are not allowed in DEFAULT expressions".to_string()
        )),
        "18.6 says 0A000 cannot use column reference"
    );
    // Outside an aggregate's own argument the star is an ordinary column reference,
    // which is 18.6's answer to each of these (measured) — and the reason the
    // exemption cannot live on the `Column` node: each of these would otherwise be
    // **accepted**, persisting a column reference no write can resolve.
    for spelling in [r#"("*")"#, r#"("*" + 1)"#, r#"("*" + count(*))"#] {
        assert_eq!(
            err(&create_with_default(spelling)),
            Some((
                "0A000",
                "cannot use column reference in DEFAULT expression".to_string()
            )),
            "{spelling}"
        );
    }
}

/// The column-reference refusal never consults the catalog: a name no relation
/// holds is the same `0A000` as a real sibling, and there is no `42703` in this
/// context on either engine.
#[test]
fn the_column_reference_refusal_is_unconditional() {
    for spelling in ["a", "totally_bogus_name", "b", "t.a"] {
        assert_eq!(
            err(&create_with_default(spelling)),
            Some((
                "0A000",
                "cannot use column reference in DEFAULT expression".to_string()
            )),
            "{spelling}"
        );
    }
}

/// A window's `OVER` clause is not walked, so `PARTITION BY a` / `ORDER BY a`
/// keep the window verdict rather than answering for the column reference they
/// contain. The fixture `ddl/default_window_over_clause_is_not_walked.sql` pins
/// the SQLSTATE; this pins the message too, and the mirror below shows the
/// window is not simply always the answer.
#[test]
fn a_windows_over_clause_is_not_walked() {
    for spelling in [
        "(row_number() OVER (PARTITION BY a))",
        "(row_number() OVER (ORDER BY a))",
        "(row_number() OVER () + a)",
    ] {
        assert_eq!(
            err(&create_with_default(spelling)),
            Some((
                "42P20",
                "window functions are not allowed in DEFAULT expressions".to_string()
            )),
            "{spelling}"
        );
    }
    assert_eq!(
        err(&create_with_default("(a + row_number() OVER ())")),
        Some((
            "0A000",
            "cannot use column reference in DEFAULT expression".to_string()
        ))
    );
}

/// A `$N` in a DDL statement binds nothing (`42P02`), and a bare `DEFAULT`
/// keyword inside a DEFAULT expression is `42601` — both at their positional
/// place, both measured on 18.6.
#[test]
fn parameter_and_default_keyword_take_their_own_states() {
    assert_eq!(
        err(&create_with_default("($1 + a)")),
        Some(("42P02", "there is no parameter $1".to_string()))
    );
    assert_eq!(
        err(&create_with_default("(a + $1)")),
        Some((
            "0A000",
            "cannot use column reference in DEFAULT expression".to_string()
        ))
    );
    // 18.6's sentence is `DEFAULT is not allowed in this context`; pglet's
    // `Error::Syntax` renders every `42601` under a `syntax error: ` prefix, the
    // same shape the stored-expression binding check already produces for
    // `CHECK (a = DEFAULT)`. The SQLSTATE and the sentence match; the prefix is
    // pglet-wide and not this rule's to change.
    assert_eq!(
        alter_err("ALTER TABLE t ALTER COLUMN b SET DEFAULT DEFAULT"),
        Some((
            "42601",
            "syntax error: DEFAULT is not allowed in this context".to_string()
        ))
    );
}

/// A general expression **is** a column default now, and the walk still runs first.
///
/// The blanket `0A000 not implemented: unsupported DEFAULT expression` this replaced
/// stood in for **no** Postgres error — 18.6 evaluates a general DEFAULT expression
/// happily — which is why it had to run after every gate standing in for a real one.
/// It is gone; the arithmetic is stored unevaluated and resolves per row, as it does
/// there (`DEFAULT 1+1` is an operator expression in `pg_attrdef`, rendered `(1 + 1)`
/// by `pg_get_expr`, not the constant `2`). What remains of the ordering claim is
/// that the forbidden-construct walk still wins over acceptance.
#[test]
fn a_general_expression_is_stored_unevaluated_and_the_walk_still_runs_first() {
    let (state, message) = err(&create_with_default("(a + 1)")).expect("must fail");
    assert_eq!(state, "0A000");
    assert!(
        message.starts_with("cannot use column reference"),
        "the column reference must win over the acceptance path: {message}"
    );
    // No constant folding: the stored default is the expression, not its value.
    assert!(
        matches!(
            parsed_default(&create_with_default("(1 + 1)")),
            Some(ColumnDefault::Expr(_))
        ),
        "a general expression must store as `ColumnDefault::Expr`"
    );
    // And it evaluates at write time, through the target column's own coercion.
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE u (a INT, b INT DEFAULT (1 + 1))")
        .expect("creates");
    db.execute("INSERT INTO u (a) VALUES (7)").expect("inserts");
    let rows = db.execute("SELECT b FROM u").expect("selects");
    assert_eq!(rows[0].rows[0][0], Value::Int4(2));
}

/// Every recognized closed form still parses to exactly the `ColumnDefault` it
/// always did — the token-level classifier is unchanged and runs first, so a
/// spelling the general expression grammar cannot express (`nextval`'s
/// `::regclass` cast; `regclass` is not a pglet type) survives.
///
/// Two rows are *not* "as it always did", and each is its own decision. The five
/// `now`-family spellings each classify to their own arm, because 18.6 renders each
/// verbatim and only `clock_timestamp()` backfills per row. And `NULL` classifies
/// to **no default at all** — 18.6 stores no `pg_attrdef` row for it
/// ([`without_null_default`]).
#[test]
fn the_recognized_forms_keep_their_exact_column_default() {
    let cases: &[(&str, ColumnDefault)] = &[
        ("5", ColumnDefault::Literal(Value::Int8(5))),
        ("-7", ColumnDefault::Literal(Value::Int8(-7))),
        // Postgres's own constant typing: a decimal or exponent literal is a
        // `numeric` constant, not a float, so `1.50` keeps its display scale —
        // 18.6's `column_default` reads `1.50`, and `1e3` reads `'1000'::numeric`
        // (measured). Storing a `Float8` lost both.
        ("+1.5", ColumnDefault::Literal(numeric("1.5"))),
        ("1.50", ColumnDefault::Literal(numeric("1.50"))),
        ("1e3", ColumnDefault::Literal(numeric("1000"))),
        ("'x'", ColumnDefault::Literal(Value::Text("x".to_string()))),
        ("true", ColumnDefault::Literal(Value::Bool(true))),
        // The six ordinary functions, each in the only spelling that is a call.
        ("now()", ColumnDefault::Now),
        ("clock_timestamp()", ColumnDefault::ClockTimestamp),
        ("statement_timestamp()", ColumnDefault::StatementTimestamp),
        (
            "transaction_timestamp()",
            ColumnDefault::TransactionTimestamp,
        ),
        ("gen_random_uuid()", ColumnDefault::RandomUuid),
        ("uuid_generate_v4()", ColumnDefault::RandomUuid),
        // The parenless value functions, which take no guard.
        ("CURRENT_TIMESTAMP", ColumnDefault::CurrentTimestamp),
        ("CURRENT_DATE", ColumnDefault::CurrentDate),
        ("CURRENT_TIME", ColumnDefault::CurrentTime),
        ("nextval('s')", ColumnDefault::NextVal("s".to_string())),
        (
            "nextval('s'::regclass)",
            ColumnDefault::NextVal("s".to_string()),
        ),
    ];
    for (spelling, expected) in cases {
        assert_eq!(
            parsed_default(&create_with_default(spelling)).as_ref(),
            Some(expected),
            "{spelling}"
        );
    }
    // A SQL value function takes **no argument list**: `current_timestamp()` is
    // `42601 syntax error at or near ")"` on 18.6, measured on both the
    // column-constraint and the `SET DEFAULT` route. The classifier used to swallow
    // the empty parens and define the column, backfilling a real timestamp where
    // 18.6 defines no default at all — this clause's one silent member. It declines
    // that spelling now and the general grammar answers.
    assert_eq!(
        err(&create_with_default("current_timestamp()")).map(|(state, _)| state),
        Some("42601")
    );
    assert_eq!(
        alter_err("ALTER TABLE t ALTER COLUMN b SET DEFAULT current_timestamp()")
            .map(|(state, _)| state),
        Some("42601")
    );
}

/// `DEFAULT NULL` is **no default**, on both routes and in both the bare and the
/// parenthesized spelling.
///
/// 18.6 stores no `pg_attrdef` row for a NULL-constant default
/// (`AddRelationNewConstraints` skips `StoreAttrDefault`), so `CREATE TABLE t (c
/// INT DEFAULT NULL)` has zero rows there and `ALTER COLUMN … SET DEFAULT NULL`
/// *removes* an existing one (measured: 1 → 0). Both spellings arrive here as an
/// `Option`, and `None` already means what 18.6 means: no catalog default on the
/// column-definition route, and `DROP DEFAULT` on the `ALTER` route.
///
/// The `DEFAULT 7` row is the control — without it the assertion holds for a parser
/// that dropped every default on the floor.
#[test]
fn a_null_default_is_no_default_at_all() {
    for spelling in ["NULL", "null", "(NULL)", "((NULL))"] {
        assert_eq!(
            parsed_default(&create_with_default(spelling)),
            None,
            "CREATE TABLE … DEFAULT {spelling}"
        );
        assert_eq!(
            set_default_of(spelling),
            None,
            "ALTER TABLE … SET DEFAULT {spelling} must clear the default"
        );
    }
    assert_eq!(
        parsed_default(&create_with_default("7")),
        Some(ColumnDefault::Literal(Value::Int8(7)))
    );
    assert_eq!(
        set_default_of("7"),
        Some(ColumnDefault::Literal(Value::Int8(7)))
    );
}

/// The NULL normalization is a *normalization*, not a rejection, so it runs after
/// the positional verdict walk: a forbidden construct beside a `NULL` still reports
/// its own SQLSTATE rather than being reduced away.
#[test]
fn a_null_default_still_reports_a_forbidden_construct() {
    assert_eq!(
        err(&create_with_default("NULL::text || (SELECT 'x')"))
            .expect("must fail")
            .0,
        "0A000"
    );
    assert_eq!(
        err(&create_with_default("count(*)")).expect("must fail").0,
        "42803"
    );
}

/// **An ordinary function's parens are required.** A bare `now` is not the
/// function — it is a column reference, and a column `DEFAULT` refuses one
/// unconditionally, so 18.6 answers `0A000 cannot use column reference in
/// DEFAULT expression` to all six of these (measured). The token-level
/// classifier used to consume the bare identifier as the call, which was this
/// surface's one **silent** divergence: the statement was accepted and the
/// column backfilled with a real timestamp / UUID where 18.6 defines no table at
/// all — and it disagreed with pglet's own `DEFAULT (now)`, which takes the
/// general path.
///
/// `ddl/default_bare_now_is_a_column_ref.sql` pins the `now` row; the other five
/// answer the same SQLSTATE, so only a message-level assertion separates them
/// from any other `0A000` and they live here.
#[test]
fn a_bare_ordinary_function_name_is_a_column_reference() {
    for spelling in [
        "now",
        "clock_timestamp",
        "statement_timestamp",
        "transaction_timestamp",
        "gen_random_uuid",
        "uuid_generate_v4",
        // The parenthesized spelling of the same mistake, which already took the
        // general path — the two disagreed before the guard.
        "(now)",
        "(gen_random_uuid)",
    ] {
        assert_eq!(
            err(&create_with_default(spelling)),
            Some((
                "0A000",
                "cannot use column reference in DEFAULT expression".to_string()
            )),
            "{spelling}"
        );
    }
}

/// A redundantly parenthesized recognized form reduces onto the **same** closed
/// arm, which is what 18.6 does (it strips the parens in parse analysis). The
/// `nextval` row is the load-bearing one: falling through to a general
/// expression would lose the sequence's ownership edge and its dump hoisting.
///
/// `(NULL)` is deliberately absent: it reduces to **no default**, which
/// `a_null_default_is_no_default_at_all` asserts in both spellings.
#[test]
fn a_parenthesized_recognized_form_reduces_to_the_same_default() {
    let cases: &[(&str, ColumnDefault)] = &[
        ("(5)", ColumnDefault::Literal(Value::Int8(5))),
        ("((5))", ColumnDefault::Literal(Value::Int8(5))),
        ("(-7)", ColumnDefault::Literal(Value::Int8(-7))),
        ("(+1.5)", ColumnDefault::Literal(numeric("1.5"))),
        ("(1.50)", ColumnDefault::Literal(numeric("1.50"))),
        ("(1e3)", ColumnDefault::Literal(numeric("1000"))),
        (
            "('x')",
            ColumnDefault::Literal(Value::Text("x".to_string())),
        ),
        ("(true)", ColumnDefault::Literal(Value::Bool(true))),
        ("(now())", ColumnDefault::Now),
        ("(clock_timestamp())", ColumnDefault::ClockTimestamp),
        ("(CURRENT_TIMESTAMP)", ColumnDefault::CurrentTimestamp),
        ("(CURRENT_DATE)", ColumnDefault::CurrentDate),
        ("(gen_random_uuid())", ColumnDefault::RandomUuid),
        ("(nextval('s'))", ColumnDefault::NextVal("s".to_string())),
    ];
    for (spelling, expected) in cases {
        assert_eq!(
            parsed_default(&create_with_default(spelling)).as_ref(),
            Some(expected),
            "{spelling}"
        );
    }
}

/// **A niladic impure builtin refuses an argument list**, so `(now(1))` is
/// `42883` and no column is defined — agreeing with 18.6's `function now(integer)
/// does not exist` at this position and at every other (`SELECT now(1)` too).
///
/// The gate is engine-wide and lives at the one site a function call is built at
/// (`planner::build_call`'s `niladic_call`), so this clause inherits it rather
/// than carrying its own: the arms used to route `now` / `transaction_timestamp` /
/// `statement_timestamp` / `clock_timestamp` / `gen_random_uuid` /
/// `uuid_generate_v4` by **name alone**, discarding the parsed arguments, and the
/// column was defined with `column_default` rendering the argument-less spelling
/// of whichever builtin the name resolved to.
///
/// **The asymmetry with the sequence family is gone, and it was structural.**
/// `column_default_from_expr`'s `nextval` arm guards `args.is_empty()`, and
/// `build_call`'s neighbouring `nextval`/`currval`/`setval` arms arity-check
/// before that — `DEFAULT (nextval('s', 1))` is `42883` here, agreeing with 18.6.
/// The niladic arms could carry neither guard *downstream*, because they returned
/// an argument-less `Expr::Builtin` and by the time the normalizer ran the
/// arguments were gone; the check therefore had to be at the call site, which is
/// the shape the sequence arms in that same match already had.
#[test]
fn a_niladic_builtins_arguments_are_refused() {
    for spelling in [
        "(now(1))",
        "(now('zzz', 42))",
        "(now(NULL))",
        "(clock_timestamp(1))",
        "(statement_timestamp(1))",
        "(transaction_timestamp(1))",
        "(gen_random_uuid(1))",
        "(uuid_generate_v4(1))",
    ] {
        let (state, message) = err(&create_with_default(spelling)).expect("must fail");
        assert_eq!(state, "42883", "{spelling}: {message}");
        assert!(message.contains("expects 0 to 0 argument"), "{message}");
    }
    // The message names the **spelled** function, not the `BuiltinFn` two names
    // share: `uuid_generate_v4` and `gen_random_uuid` are one variant here.
    let (_, message) = err(&create_with_default("(uuid_generate_v4(1))")).expect("must fail");
    assert!(message.starts_with("uuid_generate_v4()"), "{message}");
    // The **bare** spelling is loud for a different reason and keeps its own code:
    // the token-level classifier's `eat_empty_call` demands an *empty* list, so its
    // own `expect(')')` answers ahead of `build_call`. 18.6 is `42883` here — the
    // code still diverges, and the direction does not.
    let (state, message) = err(&create_with_default("now(1)")).expect("must fail");
    assert_eq!(state, "42601");
    assert!(message.contains("expected ')'"), "{message}");
    // The contrast the gate was modeled on: the same statement shape over the
    // sequence family is arity-checked, and takes 18.6's own SQLSTATE.
    let (state, message) = err(&create_with_default("(nextval('s', 1))")).expect("must fail");
    assert_eq!(state, "42883");
    assert!(message.contains("expects 1 to 1 argument"), "{message}");
    // A wildcard argument list is *empty*, so the arity check above never sees it;
    // it takes 18.6's own separate verdict (`ParseFuncOrColumn`'s `agg_star` arm).
    let (state, message) = err(&create_with_default("(now(*))")).expect("must fail");
    assert_eq!(state, "42809", "{message}");
    assert!(
        message.contains("now(*) specified, but now is not an aggregate function"),
        "{message}"
    );
    // And the argument-less spellings still build the same defaults they did — the
    // gate refuses an argument list, not the builtins.
    for (spelling, expected) in [
        ("(now())", ColumnDefault::Now),
        ("(clock_timestamp())", ColumnDefault::ClockTimestamp),
        ("(gen_random_uuid())", ColumnDefault::RandomUuid),
    ] {
        assert_eq!(
            parsed_default(&create_with_default(spelling)).as_ref(),
            Some(&expected),
            "{spelling}"
        );
    }
}

/// `LOCALTIMESTAMP` / `LOCALTIME` are `Expr::Builtin` too and are deliberately **not**
/// normalized onto a closed `ColumnDefault` arm — they have none. They are stored as
/// the `Expr` catch-all instead, which is what closes the recorded `DEFAULT
/// LOCALTIMESTAMP` over-rejection: 18.6 accepts both (measured, and
/// `information_schema.columns.column_default` reads `LOCALTIMESTAMP` / `LOCALTIME`),
/// and so does this now.
///
/// The narrowness of the normalizer is the assertion: neither lands on
/// `ColumnDefault::Now` or `CurrentTime`, which would render the wrong spelling back
/// to an introspecting client and to the dump.
#[test]
fn localtimestamp_stores_as_the_expression_catch_all() {
    for spelling in ["LOCALTIMESTAMP", "(LOCALTIMESTAMP)", "LOCALTIME"] {
        assert!(
            matches!(
                parsed_default(&create_with_default(spelling)),
                Some(ColumnDefault::Expr(_))
            ),
            "{spelling} must store as the `Expr` catch-all, not a closed arm"
        );
    }
}

/// A `nextval` whose argument is not a string literal has already consumed its
/// call by the time the classifier declines, so it raises rather than resuming
/// the caller's parse mid-statement. Same `0A000` the general expression path's
/// own `sequence_name_from_arg` gives for the identical shape.
#[test]
fn a_non_literal_nextval_argument_is_loud_rather_than_resumed() {
    let (state, message) = err(&create_with_default("nextval(5)")).expect("must fail");
    assert_eq!(state, "0A000");
    assert!(
        message.contains("string-literal sequence name"),
        "{message}"
    );
}

/// Both callers of the DEFAULT parser get the walk. `ALTER COLUMN … SET DEFAULT`
/// is the second one, and it takes a different grammar (`a_expr`, not the column
/// constraint's `b_expr`) — the verdicts are the same either way.
#[test]
fn the_alter_column_route_takes_the_same_verdicts() {
    assert_eq!(
        alter_err("ALTER TABLE t ALTER COLUMN b SET DEFAULT count(*) + a"),
        Some((
            "42803",
            "aggregate functions are not allowed in DEFAULT expressions".to_string()
        ))
    );
    assert_eq!(
        alter_err("ALTER TABLE t ALTER COLUMN b SET DEFAULT a + count(*)"),
        Some((
            "0A000",
            "cannot use column reference in DEFAULT expression".to_string()
        ))
    );
    assert_eq!(
        alter_err("ALTER TABLE t ALTER COLUMN b SET DEFAULT (SELECT 1)"),
        Some((
            "0A000",
            "cannot use subquery in DEFAULT expression".to_string()
        ))
    );
}

/// A default that **cannot be written** must be refused at DDL, even where pglet's
/// SQLSTATE is not 18.6's.
///
/// These are the accept-then-brick shapes the cook's error family exists to catch:
/// each was accepted here and then failed *every* omitted-column `INSERT`, leaving a
/// column defined and unwritable — which is what 18.6 refuses at `CREATE TABLE`.
///
/// **Two of the six rows in the first loop below** cannot be a fixture, because
/// `@expect-error` compares SQLSTATE and pglet's differs there: `DEFAULT 5 + 'x'` is
/// `42804` here against 18.6's `22P02` (pglet's operand gate answers before any
/// unknown literal is coerced), and the `INTERVAL` row is pglet's own `0A000` where
/// 18.6 accepts. The other four agree at `42804` — measured on both engines, row by
/// row — so they are **fixture-able** and stay here for the reason the paragraph
/// below gives.
/// `DEFAULT 'x' || 'y'` into an `INT` column is one of the four: the assignment gate
/// reaching the cook (`executor::ddl`'s `cook_column_default`) answers it `42804`, as the
/// row asserts.
///
/// The `BOOL` rows are three of the four that **agree**, and they are here as the pair
/// that separates a rule from an accident. `int ↔ bool` is an *explicit-only* cast in
/// `pg_cast` (`castcontext = 'e'`, both directions, measured), so 18.6 answers
/// `42804 column "b" is of type boolean but default expression is of type …` to the
/// integer spellings and `42804 … is of type bigint but … is of type boolean` to the
/// reverse — while `SELECT 1::bool` and `SELECT true::int` answer there and here.
/// pglet's `coerce_value` is one table serving both contexts, so the assignment
/// direction was silently performed: `DEFAULT 1 + 1` on a `BOOL` column stored
/// `true`. `DEFAULT 1` was already loud, at `42846`, only because an integer literal
/// is an `Int8` for which no arm happened to exist — the accident standing in for the
/// rule, which is why the two spellings are asserted together.
#[test]
fn an_unwritable_default_is_refused_at_ddl() {
    for (sql, state) in [
        ("CREATE TABLE t (a INT, b INT DEFAULT 5 + 'x')", "42804"),
        // `'x' || 'y'` is two unknowns, which `select_common_type` types `text`, so
        // this is the same `42804` as its neighbours and **not** the `22P02` this row
        // asserted before the assignment gate reached the cook — that code was
        // pglet's own value coercion answering, and the row was pinning a divergence
        // as if it were agreement (18.6, measured: `column "b" is of type integer but
        // default expression is of type text`).
        ("CREATE TABLE t (a INT, b INT DEFAULT 'x' || 'y')", "42804"),
        ("CREATE TABLE t (a INT, b BOOL DEFAULT 1)", "42804"),
        ("CREATE TABLE t (a INT, b BOOL DEFAULT 1 + 1)", "42804"),
        ("CREATE TABLE t (a INT, b BIGINT DEFAULT true)", "42804"),
        // pglet's own unrenderable-expression gate, which runs **last** because it
        // stands in for no 18.6 error at all — 18.6 accepts this default. It is the
        // whole-database dump outage `pglet::dump`'s single `Result` would otherwise
        // produce. The column is `INTERVAL` so the coercion above it succeeds and
        // this gate is the one that answers.
        (
            "CREATE TABLE t (a INT, b INTERVAL DEFAULT interval '1 day')",
            "0A000",
        ),
    ] {
        assert_eq!(err(sql).map(|(state, _)| state), Some(state), "{sql}");
    }
    // The controls, and they are what make the assertions above about *writability*
    // rather than about strictness: the neighbouring shapes 18.6 accepts and raises
    // per row are accepted here too. A typmod is applied after the type's input
    // function, and an arithmetic or cast failure on a typed constant is the row's.
    for sql in [
        "CREATE TABLE t (a INT, b VARCHAR(3) DEFAULT 'abcdef')",
        "CREATE TABLE t (a INT, b NUMERIC(4,2) DEFAULT 12345.6)",
        "CREATE TABLE t (a INT, b INT DEFAULT 1/0)",
        "CREATE TABLE t (a INT, b INT DEFAULT 2147483648)",
        "CREATE TABLE t (a INT, b INT DEFAULT (2147483648)::int)",
        "CREATE TABLE t (a INT, b TEXT DEFAULT 5)",
    ] {
        assert_eq!(err(sql), None, "{sql}");
    }
    // **The explicit-cast control for the `BOOL` rows above**, and it is what makes
    // the refusal a *context* rule rather than a missing conversion. Deleting the
    // `int4 → bool` arm from the shared coercion table is green across the whole
    // fixture corpus and the whole workspace (measured) and takes these two with it —
    // no fixture casts an integer to a boolean, so the corpus cannot see the
    // direction. 18.6 answers `t` / `1` to both.
    let mut db = Database::with_btree();
    let rows = db
        .execute("SELECT 1::bool, 0::bool, true::int")
        .expect("18.6 answers all three");
    assert_eq!(
        rows[0].rows[0],
        vec![Value::Bool(true), Value::Bool(false), Value::Int4(1)]
    );
}
