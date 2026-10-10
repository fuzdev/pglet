//! A `$N` parse analysis leaves without a type, at the in-process host.
//!
//! The verdicts themselves are pinned against 18.6 by `tests/fixtures/params/undetermined_param_*`.
//! What sits outside that corpus is what the differential harness cannot bind or cannot
//! see:
//!
//! - **`Database::execute_bound` refuses before it binds** — the wasm bindings'
//!   `queryParams` runs through it, and a value's own runtime type must not stand in for
//!   a type the statement does not give its parameter.
//! - **The refusal fails an open block**, as a `Parse` error does on 18.6.
//! - **Accepted statements whose parameter this engine reports no OID for**, which a
//!   binary-protocol client cannot bind (`docs/param_binding.md` § Deferred / known gaps).
//! - **A missing relation's `42P01` wins** where it is certain, the parameter it would
//!   be bound through having no OID either.
//! - **A value bound to a parameter no reading types** is of its own runtime type — a
//!   whole JS number an `int8`, a fractional one a `float8` — so an operator over it
//!   returns that type's value, whatever its sibling operand's is. A fold over such an
//!   operator has to convert it to the type the fold declares.

// A test harness: `.expect()`/`.unwrap()`/`panic!` surface an unexpected verdict shape
// loudly right here.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::cell::Cell;

use pglet::{BoundExecError, Database};
use pglet_sql::types::{TypeOid, Value, value_to_text};

fn seeded() -> Database {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE t (id INT PRIMARY KEY, name TEXT)",
        "INSERT INTO t VALUES (1, 'x'), (2, 'y')",
    ] {
        db.execute(sql).expect("seeds");
    }
    db
}

/// Run `sql` with `$1` bound to `value`, answering the SQLSTATE of an engine refusal
/// and whether the binding callback ran.
fn bound_state(db: &mut Database, sql: &str, value: Option<Value>) -> (Option<String>, bool) {
    let bound = Cell::new(false);
    let outcome = db.execute_bound(sql, 0, |_| {
        bound.set(true);
        Ok::<_, ()>(vec![value])
    });
    let state = match outcome {
        Ok(_) => None,
        Err(BoundExecError::Engine(e)) => Some(e.sqlstate().to_string()),
        Err(BoundExecError::Bind(())) => panic!("the callback never fails"),
    };
    (state, bound.get())
}

#[test]
fn an_untyped_parameter_is_refused_before_any_value_is_bound() {
    let mut db = seeded();
    // The value's own type is irrelevant: a number, a string and a NULL are refused
    // alike, and the callback that would have decoded them is never asked.
    for value in [Some(Value::Int8(5)), Some(Value::Text("x".into())), None] {
        for (sql, state) in [
            ("SELECT $1 IS NULL", "42P18"),
            ("SELECT count($1) FROM t", "42P18"),
            (
                "SELECT count(*) FROM t WHERE ($1 IS NULL OR name = $1)",
                "42P08",
            ),
            ("DELETE FROM t WHERE $1 IS NULL", "42P18"),
        ] {
            assert_eq!(
                bound_state(&mut db, sql, value.clone()),
                (Some(state.to_string()), false),
                "{sql} bound {value:?}"
            );
        }
    }
    // Nothing was deleted by the refused `DELETE`.
    let rows = db.execute("SELECT count(*) FROM t").expect("count");
    assert_eq!(
        rows.last()
            .and_then(|r| r.rows.first())
            .and_then(|r| r.first()),
        Some(&Value::Int8(2))
    );
}

#[test]
fn a_parameter_typed_in_time_binds_and_runs() {
    let mut db = seeded();
    for sql in [
        // The comparison-first optional filter, and its cast spelling.
        "SELECT count(*) FROM t WHERE (name = $1 OR $1 IS NULL)",
        "SELECT count(*) FROM t WHERE ($1::text IS NULL OR name = $1)",
        // `DISTINCT` resolves an equality operator for `count`'s argument, and a simple
        // `CASE` resolves its operand before a branch is read: both type the parameter
        // on 18.6, and this engine reports no OID for either.
        "SELECT count(DISTINCT $1) FROM t",
        "SELECT CASE $1 WHEN 'x' THEN $1 IS NULL END",
    ] {
        assert_eq!(
            bound_state(&mut db, sql, Some(Value::Text("x".into()))),
            (None, true),
            "{sql}"
        );
    }
}

#[test]
fn the_refusal_fails_an_open_block() {
    let mut db = seeded();
    db.execute("BEGIN").expect("begin");
    db.execute("INSERT INTO t VALUES (3, 'z')").expect("insert");
    assert_eq!(
        bound_state(&mut db, "SELECT $1 IS NULL", None),
        (Some("42P18".to_string()), false)
    );
    let next = db.execute("SELECT 1").expect_err("the block is aborted");
    assert_eq!(next.sqlstate(), "25P02");
    db.execute("ROLLBACK").expect("rollback");
    let rows = db.execute("SELECT count(*) FROM t").expect("count");
    assert_eq!(
        rows.last()
            .and_then(|r| r.rows.first())
            .and_then(|r| r.first()),
        Some(&Value::Int8(2))
    );
}

#[test]
fn a_missing_relation_outranks_the_refusal() {
    let mut db = seeded();
    // 18.6 resolves the range table before it looks at a parameter, so each of these is
    // its `42P01`. Here the executor raises that, after the bind.
    for sql in [
        "SELECT id FROM nope WHERE $1 IS NULL",
        "SELECT id FROM nope WHERE ($1 IS NULL OR id = $1)",
        "UPDATE nope SET n = 1 WHERE $1 IS NULL",
        "DELETE FROM nope WHERE $1 IS NULL",
        "SELECT t.id FROM t JOIN nope n ON n.id = t.id WHERE $1 IS NULL",
    ] {
        assert_eq!(
            bound_state(&mut db, sql, None),
            (Some("42P01".to_string()), true),
            "{sql}"
        );
    }
    // A relation named only inside an expression's subquery is reported for a row that
    // evaluates it and for no other, so the refusal stands: the statement must not run
    // and answer.
    db.execute("CREATE TABLE empty (id INT)").expect("create");
    for sql in [
        "SELECT id FROM t WHERE $1 IS NULL AND id IN (SELECT id FROM nope)",
        "SELECT id FROM empty WHERE $1 IS NULL AND id IN (SELECT id FROM nope)",
    ] {
        assert_eq!(
            bound_state(&mut db, sql, None),
            (Some("42P18".to_string()), false),
            "{sql}"
        );
    }
}

#[test]
fn an_insert_select_entry_is_bound_as_its_target_column() {
    let mut db = seeded();
    db.execute("CREATE TABLE b (id INT, by BYTEA, n INT)")
        .expect("create");
    let mut described = Vec::new();
    db.execute_bound("INSERT INTO b (id, by, n) SELECT 1, $1, $2", 0, |oids| {
        described = oids.to_vec();
        Ok::<_, ()>(vec![Some(Value::Bytea(vec![1, 2])), Some(Value::Int4(7))])
    })
    .unwrap_or_else(|_| panic!("the insert runs"));
    assert_eq!(described, vec![Some(TypeOid::Bytea), Some(TypeOid::Int4)]);
    let rows = db.execute("SELECT by, n FROM b").expect("select");
    assert_eq!(
        rows.last().map(|r| r.rows.clone()),
        Some(vec![vec![Value::Bytea(vec![1, 2]), Value::Int4(7)]])
    );
}

/// The value the wasm bindings bind for a JS number: decoded as the inferred type where
/// the statement gives its parameter one, and by the number's own kind where it does
/// not — whole, an `int8`; fractional, a `float8`.
fn js_number(n: f64, inferred: Option<TypeOid>) -> Value {
    let whole = n.fract() == 0.0;
    #[allow(clippy::cast_possible_truncation)] // the test binds small whole numbers
    match inferred {
        Some(TypeOid::Int2) if whole => Value::Int2(n as i16),
        Some(TypeOid::Int4) if whole => Value::Int4(n as i32),
        Some(TypeOid::Int8) | None if whole => Value::Int8(n as i64),
        Some(TypeOid::Float4) => Value::Float4(n as f32),
        Some(TypeOid::Numeric) => {
            Value::Numeric(pglet_sql::types::PgNumeric::parse(&n.to_string()).expect("a numeric"))
        }
        Some(TypeOid::Float8) | None => Value::Float8(n),
        Some(other) => panic!("no JS number binds as {other:?}"),
    }
}

/// The first column of `sql`'s rows with `$1` bound as [`js_number`] binds `number`
/// (`None`: no parameter), each beside the type the result declares for it — or the
/// SQLSTATE the statement is refused with.
fn declared_and_values(
    db: &mut Database,
    sql: &str,
    number: Option<f64>,
) -> Result<(TypeOid, Vec<Value>), String> {
    let outcome = db.execute_bound(sql, 0, |inferred| {
        Ok::<_, ()>(
            number
                .map(|n| vec![Some(js_number(n, inferred[0]))])
                .unwrap_or_default(),
        )
    });
    match outcome {
        Ok(result) => Ok((
            result.columns()[0].type_oid,
            result.rows.iter().map(|row| row[0].clone()).collect(),
        )),
        Err(BoundExecError::Engine(e)) => Err(e.sqlstate().to_string()),
        Err(BoundExecError::Bind(())) => panic!("the callback never fails"),
    }
}

/// Every value an operator over an **unknown operand** hands a fold is of the type the
/// fold declares. No reading types the unknown — an untyped `$N`, an untyped `NULL` — so
/// nothing converts it to its sibling operand's type, and the operator returns what the
/// bound value makes it: a `numeric` times a bound `0.1` is a `double precision`. Read
/// at the sibling's type, such a branch matched its fold's target and was left uncast —
/// a `double precision` under a `numeric` declaration, an `int8` under an `integer`, a
/// `double precision` under a `real`, an `integer` under a `date` — which this host hands
/// its caller as it is.
#[test]
fn a_fold_over_an_unknown_operand_declares_the_type_its_values_have() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE p (id INT, s2 SMALLINT, i4 INT, i8 BIGINT, price NUMERIC, w REAL, \
         f8 FLOAT8, d DATE)",
        "INSERT INTO p VALUES (1, 3, 3, 3, 1.5, 0.1, 0.1, '2020-01-05'), \
         (2, 7, 7, 7, 19.99, 2.5, 2.5, '2020-01-03')",
    ] {
        db.execute(sql).expect("seeds");
    }
    let folds = [
        "COALESCE({X}, {C})",
        "COALESCE({C}, {X})",
        "CASE WHEN id > 1 THEN {X} ELSE {C} END",
        "greatest({X}, {C})",
        // the operator as `nullif`'s **first** argument, the one it returns. As its second
        // it is only compared, and what `nullif` returns beside a bound `0.1` is its first
        // argument at the comparison's `double precision` under that argument's own
        // declaration — a different defect, and an older one
        "nullif({X}, {C})",
        "ARRAY[{X}, {C}]",
    ];
    // each statement whose values are not all of the type it declares
    let mut parted: Vec<String> = Vec::new();
    let mut check = |sql: &str, declared: TypeOid, values: &[Value]| {
        let agrees = values.iter().all(|value| match value {
            Value::Null => true,
            Value::Array(array) => {
                array.element_type.array_of() == Some(declared)
                    && array
                        .elements
                        .iter()
                        .all(|e| e.is_null() || e.type_oid() == Some(array.element_type))
            }
            scalar => scalar.type_oid() == Some(declared),
        });
        if !agrees {
            parted.push(format!("{sql}: declared {declared:?}, values {values:?}"));
        }
    };
    // an untyped bound number, whole and fractional, beside each numeric type
    let mut answered = 0;
    for column in ["s2", "i4", "i8", "price", "w", "f8"] {
        for operator in ["{C} * $1", "$1 + {C}", "{C} - $1", "{C} / $1"] {
            for fold in folds {
                for number in [2.0, 0.1] {
                    let sql = format!(
                        "SELECT {} FROM p ORDER BY id",
                        fold.replace("{X}", operator).replace("{C}", column)
                    );
                    // a refusal is loud, and not this test's
                    if let Ok((declared, values)) = declared_and_values(&mut db, &sql, Some(number))
                    {
                        check(&sql, declared, &values);
                        answered += 1;
                    }
                }
            }
        }
    }
    assert!(answered > 0, "every statement was refused");
    // an untyped `NULL` operand beside a `date` and a `real`
    for (operand, sibling) in [("d - NULL", "0"), ("w + NULL", "w"), ("NULL * w", "w")] {
        for fold in folds {
            let sql = format!(
                "SELECT {} FROM p ORDER BY id",
                fold.replace("{X}", operand).replace("{C}", sibling)
            );
            if let Ok((declared, values)) = declared_and_values(&mut db, &sql, None) {
                check(&sql, declared, &values);
            }
        }
    }
    assert!(parted.is_empty(), "{}", parted.join("\n"));
    // the fold's declaration is its typed branch's, and the value is converted to it:
    // the product of a `numeric` and a bound `0.1` is an exact `numeric` again
    let text =
        |values: &[Value]| -> Vec<Option<String>> { values.iter().map(value_to_text).collect() };
    let (declared, values) = declared_and_values(
        &mut db,
        "SELECT COALESCE(price * $1, price) FROM p ORDER BY id",
        Some(0.1),
    )
    .expect("answers");
    assert_eq!(declared, TypeOid::Numeric);
    assert_eq!(text(&values), [Some("0.15".into()), Some("1.999".into())]);
    let (declared, values) = declared_and_values(
        &mut db,
        "SELECT COALESCE(i4 * $1, i4) FROM p ORDER BY id",
        Some(2.0),
    )
    .expect("answers");
    assert_eq!(declared, TypeOid::Int4);
    assert_eq!(values, [Value::Int4(6), Value::Int4(14)]);
    let (declared, values) = declared_and_values(
        &mut db,
        "SELECT COALESCE(w * $1, w) FROM p ORDER BY id",
        Some(2.0),
    )
    .expect("answers");
    assert_eq!(declared, TypeOid::Float4);
    assert_eq!(values, [Value::Float4(0.2), Value::Float4(5.0)]);
    let (declared, values) = declared_and_values(
        &mut db,
        "SELECT COALESCE(d - NULL, 0) FROM p ORDER BY id",
        None,
    )
    .expect("answers");
    assert_eq!(declared, TypeOid::Int4);
    assert_eq!(values, [Value::Int4(0), Value::Int4(0)]);
    // read by a filter and by an aggregate, the converted value is the one compared and
    // the one summed
    let (_, values) = declared_and_values(
        &mut db,
        "SELECT id FROM p WHERE COALESCE(price * $1, price) = 0.15",
        Some(0.1),
    )
    .expect("answers");
    assert_eq!(values, [Value::Int4(1)]);
    let (declared, values) = declared_and_values(
        &mut db,
        "SELECT sum(x) FROM (SELECT COALESCE(price * $1, price) AS x FROM p) q",
        Some(0.1),
    )
    .expect("answers");
    assert_eq!(declared, TypeOid::Numeric);
    assert_eq!(text(&values), [Some("2.149".into())]);
}
