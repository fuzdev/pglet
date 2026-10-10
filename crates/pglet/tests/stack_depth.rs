//! The statement-depth bounds (`pglet_sql::depth`), driven to their edge on threads of
//! known stack — the instrument behind the constants there.
//!
//! Every shape the parser can grow deep is found at its **largest answered size** on a
//! caller's thread, and then driven past it, far past it, and through everything that
//! walks it: parse and plan, execution over a row, the catalog render and the portable
//! dump, and — for a stored expression — a file-backed reopen that decodes it and
//! evaluates it again. Past the edge a statement must be refused (`54001`, or the
//! parser's nesting wall `42601`), and the process must still be here to say so.
//!
//! **A bound too loose does not fail an assertion here: it overflows, and the test
//! binary aborts. That abort is the red.** Two halves are bounded: the byte check
//! (`depth::check_stack`) stops every fallible walk short of the thread's reserve, and
//! the count (`depth::max_depth`) keeps every infallible one — `Clone`, `Drop`, the
//! type walks — inside that reserve. Remove the byte check and the nesting shapes
//! overflow a debug build's 2 MiB thread (`abs(…)` from about 100 levels) and the CTE
//! chain a release build's; the edges each shape reaches here are the numbers
//! `pglet_sql::depth`'s docs quote.
//!
//! `cargo test -p pglet --test stack_depth` measures the debug constants, and
//! `cargo test -p pglet --release --test stack_depth` the release ones.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::format_push_string
)]

use pglet::Database;
use pglet_testing::db::ScratchDir;

/// The stack Rust spawns a thread with by default, and libtest's — the one
/// `DEFAULT_MAX_DEPTH` is sized for.
const SMALL_STACK: usize = 2 * 1024 * 1024;
/// A main thread's usual stack, where every shape must answer at least as deep.
const LARGE_STACK: usize = 8 * 1024 * 1024;

/// One deep-tree shape: its setup statements, and the statement grown to size `n`.
struct Shape {
    name: &'static str,
    setup: &'static [&'static str],
    grow: fn(usize) -> String,
    /// Whether the grown statement stores an expression, so a file-backed reopen
    /// (decode, then evaluate on insert) is part of its pipeline.
    stored: bool,
    /// Whether each repetition is a parser *nesting* level, so the parser's own
    /// nesting wall (`42601` past 128) can come first.
    nested: bool,
    /// The size this shape must answer on a 2 MiB thread, `[release, debug]`: what a
    /// build before any bound answered there, where it was measured — `1` where it was
    /// not. A debug build's nested calls answer below that build's 96 (`EDGES.md`
    /// row 171), so that floor is `1`.
    floor: [usize; 2],
}

impl Shape {
    fn floor(&self) -> usize {
        self.floor[usize::from(cfg!(debug_assertions))]
    }
}

fn rep(piece: &str, n: usize) -> String {
    piece.repeat(n)
}

fn join_chain(n: usize, sep: &str) -> String {
    let mut sql = String::from("SELECT count(*) FROM (VALUES (1)) t0(a)");
    for i in 1..n {
        sql.push_str(&format!(
            "{sep}(VALUES (1)) t{i}(a){}",
            if sep == ", " { "" } else { " ON true" }
        ));
    }
    sql
}

fn cte_chain(n: usize) -> String {
    let mut sql = String::from("WITH c0 AS (SELECT 1 AS x)");
    for i in 1..n {
        sql.push_str(&format!(", c{i} AS (SELECT x FROM c{})", i - 1));
    }
    sql.push_str(&format!(" SELECT x FROM c{}", n.saturating_sub(1)));
    sql
}

/// `n` nested `EXISTS`, each correlated with the level above it.
fn correlated_exists(n: usize) -> String {
    let mut sql = String::from("SELECT count(*) FROM t t0 WHERE ");
    for i in 1..=n {
        sql.push_str(&format!(
            "EXISTS (SELECT 1 FROM t t{i} WHERE t{i}.a = t{}.a AND ",
            i - 1
        ));
    }
    sql.push_str("true");
    sql.push_str(&rep(")", n));
    sql
}

/// A `WHERE` of `n` `AND`ed comparisons — each distinct, so no two are one term.
fn where_and(n: usize) -> String {
    let mut sql = String::from("SELECT count(*) FROM t WHERE a = 1");
    for i in 0..n {
        sql.push_str(&format!(" AND a <> {}", i + 2));
    }
    sql
}

const TABLE: &[&str] = &["CREATE TABLE t (a INT)", "INSERT INTO t VALUES (1)"];

/// Every construct the parser builds a tree from one level per repetition of —
/// loops (an operator chain, a postfix chain, a set-operation, join or CTE chain) and
/// nesting (a call, a `CASE`, a subquery) — plus the stored-expression doors.
const SHAPES: &[Shape] = &[
    Shape {
        name: "+ chain",
        setup: &[],
        grow: |n| format!("SELECT 1{}", rep("+1", n)),
        stored: false,
        nested: false,
        floor: [2170, 96],
    },
    Shape {
        name: "|| chain",
        setup: &[],
        grow: |n| format!("SELECT 'a'{}", rep("||'a'", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    // Operands the planner coerces — a mixed-type chain and unknown-typed literals —
    // so a pass that inserts a cast per level after the parser measured the tree is
    // driven too.
    Shape {
        name: "mixed-type + chain",
        setup: &[],
        grow: |n| format!("SELECT 1{}", rep("+1.5+1", n / 2 + 1)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "unknown-literal || chain",
        setup: TABLE,
        grow: |n| format!("SELECT a{} FROM t", rep("||'x'", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: ":: chain",
        setup: &[],
        grow: |n| format!("SELECT 1{}", rep("::int", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "IS NULL chain",
        setup: &[],
        grow: |n| format!("SELECT 1{}", rep(" IS NULL", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "IS NOT NULL chain",
        setup: &[],
        grow: |n| format!("SELECT 1{}", rep(" IS NOT NULL", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "IS TRUE chain",
        setup: &[],
        grow: |n| format!("SELECT true{}", rep(" IS TRUE", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "-> chain",
        setup: &[],
        grow: |n| format!("SELECT '{{}}'::jsonb{}", rep("->'a'", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    // The rest of the parser's operator-loop arms (`parser::expr`'s `operator_loop`): each
    // builds its node around the chain so far, so a tree grows one level a repetition
    // through it with no parser nesting.
    // `parse_cast_target` builds a string-function node for a `regclass` target, not a
    // cast: every walker takes it in its big dispatch frame.
    Shape {
        name: "::regclass chain",
        setup: TABLE,
        grow: |n| format!("SELECT 't'{}", rep("::regclass", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: ":: typmod chain",
        setup: &[],
        grow: |n| format!("SELECT 'a'{}", rep("::varchar(3)", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "AT TIME ZONE chain",
        setup: &[],
        grow: |n| format!("SELECT now(){}", rep(" AT TIME ZONE 'UTC'", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "IS DISTINCT FROM chain",
        setup: &[],
        grow: |n| format!("SELECT true{}", rep(" IS DISTINCT FROM true", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "IN list chain",
        setup: TABLE,
        grow: |n| {
            format!(
                "SELECT count(*) FROM t WHERE ((a = 1){}) IS NOT NULL",
                rep(" IN (true, a = a)", n)
            )
        },
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "NOT IN list chain",
        setup: &[],
        grow: |n| format!("SELECT true{}", rep(" NOT IN (false, NULL)", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "IN subquery chain",
        setup: TABLE,
        grow: |n| {
            format!(
                "SELECT count(*) FROM t WHERE (a = 1){}",
                rep(" IN (SELECT true)", n)
            )
        },
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "ANY / ALL chain",
        setup: &[],
        grow: |n| {
            format!(
                "SELECT true{}",
                rep(
                    " = ANY(ARRAY[true]) IS TRUE <> ALL(ARRAY[false]) IS TRUE",
                    n / 4 + 1
                )
            )
        },
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    // No member of the list is a column: the catalog rendering spells `x IN (…, c)` as
    // `x = … OR x = c`, the left operand once per such member, as 18.6's
    // `transformAExprIn` builds it — so nested, the rendered text doubles a level
    // (`EDGES.md`).
    Shape {
        name: "CHECK over an IN chain",
        setup: &[],
        grow: |n| {
            format!(
                "CREATE TABLE c (a INT CHECK (((a > 0){}) IS NOT NULL))",
                rep(" IN (true, false)", n)
            )
        },
        stored: true,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "CHECK over an ANY chain",
        setup: &[],
        grow: |n| {
            format!(
                "CREATE TABLE c (a INT CHECK ((a > 0 IS TRUE{}) IS NOT NULL))",
                rep(" = ANY(ARRAY[true]) IS TRUE", n / 2 + 1)
            )
        },
        stored: true,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "nested call",
        setup: &[],
        grow: |n| format!("SELECT {}1{}", rep("abs(", n), rep(")", n)),
        stored: false,
        nested: true,
        floor: [126, 1],
    },
    Shape {
        name: "nested CASE",
        setup: &[],
        grow: |n| {
            format!(
                "SELECT {}1{}",
                rep("CASE WHEN true THEN ", n),
                rep(" END", n)
            )
        },
        stored: false,
        nested: true,
        floor: [1, 1],
    },
    Shape {
        name: "NOT chain",
        setup: &[],
        grow: |n| format!("SELECT {}true", rep("NOT ", n)),
        stored: false,
        nested: true,
        floor: [1, 1],
    },
    Shape {
        name: "parens around a chain",
        setup: &[],
        grow: |n| format!("SELECT {}1{}", rep("(", n), rep("+1)", n)),
        stored: false,
        nested: true,
        floor: [1, 1],
    },
    Shape {
        name: "nested scalar subquery",
        setup: &[],
        grow: |n| format!("SELECT {}1{}", rep("(SELECT ", n), rep(")", n)),
        stored: false,
        nested: true,
        floor: [1, 1],
    },
    Shape {
        name: "nested EXISTS",
        setup: &[],
        grow: |n| format!("SELECT {}1{}", rep("EXISTS (SELECT ", n), rep(")", n)),
        stored: false,
        nested: true,
        floor: [1, 1],
    },
    Shape {
        name: "correlated EXISTS",
        setup: TABLE,
        grow: correlated_exists,
        stored: false,
        nested: true,
        floor: [42, 25],
    },
    Shape {
        name: "nested FROM subquery",
        setup: &[],
        grow: |n| {
            format!(
                "SELECT * FROM {}(VALUES (1)) v(a){}",
                rep("(SELECT * FROM ", n),
                rep(") s", n)
            )
        },
        stored: false,
        nested: true,
        floor: [1, 1],
    },
    Shape {
        name: "UNION ALL chain",
        setup: &[],
        grow: |n| format!("SELECT 1{}", rep(" UNION ALL SELECT 1", n)),
        stored: false,
        nested: false,
        floor: [1328, 121],
    },
    Shape {
        name: "INTERSECT chain",
        setup: &[],
        grow: |n| format!("SELECT 1{}", rep(" INTERSECT SELECT 1", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    // A plan chain under an expression: every level of it is charged
    // `EMBEDDED_PLAN_LEVEL_WEIGHT`, since cloning the expression clones it.
    Shape {
        name: "UNION ALL chain under EXISTS",
        setup: TABLE,
        grow: |n| {
            format!(
                "SELECT count(*) FROM t WHERE EXISTS (SELECT 1{})",
                rep(" UNION ALL SELECT 1", n)
            )
        },
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "JOIN chain",
        setup: &[],
        grow: |n| join_chain(n + 1, " JOIN "),
        stored: false,
        nested: false,
        floor: [787, 90],
    },
    Shape {
        name: "comma FROM",
        setup: &[],
        grow: |n| join_chain(n + 1, ", "),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "CTE chain",
        setup: &[],
        grow: |n| cte_chain(n + 1),
        stored: false,
        nested: false,
        floor: [663, 59],
    },
    Shape {
        name: "UNION ALL chain in a CTE",
        setup: &[],
        grow: |n| {
            format!(
                "WITH c AS (SELECT 1 AS x{}) SELECT count(*) FROM c",
                rep(" UNION ALL SELECT 1", n)
            )
        },
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "WHERE over a row",
        setup: TABLE,
        grow: |n| format!("SELECT count(*) FROM t WHERE a{} > 0", rep("+1", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "ORDER BY",
        setup: TABLE,
        grow: |n| format!("SELECT a FROM t ORDER BY a{}", rep("+1", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "GROUP BY / HAVING",
        setup: TABLE,
        grow: |n| {
            format!(
                "SELECT a{0} FROM t GROUP BY a{0} HAVING count(*){0} > 0",
                rep("+1", n)
            )
        },
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "UPDATE SET … RETURNING",
        setup: TABLE,
        grow: |n| format!("UPDATE t SET a = a{0} RETURNING a{0}", rep("*1", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "INSERT VALUES",
        setup: TABLE,
        grow: |n| format!("INSERT INTO t VALUES (1{})", rep("*1", n)),
        stored: false,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "CHECK",
        setup: &[],
        grow: |n| format!("CREATE TABLE c (a INT CHECK (a{} > 0))", rep("+1", n)),
        stored: true,
        nested: false,
        floor: [1495, 96],
    },
    Shape {
        name: "CHECK over a cast chain",
        setup: &[],
        grow: |n| format!("CREATE TABLE c (a INT CHECK (a{} > 0))", rep("::int", n)),
        stored: true,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "DEFAULT",
        setup: &[],
        grow: |n| format!("CREATE TABLE c (a INT DEFAULT 1{})", rep("+1", n)),
        stored: true,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "index expression",
        setup: &["CREATE TABLE c (a INT)"],
        grow: |n| format!("CREATE INDEX ci ON c ((a{}))", rep("+1", n)),
        stored: true,
        nested: false,
        floor: [1, 1],
    },
    Shape {
        name: "partial index predicate",
        setup: &["CREATE TABLE c (a INT)"],
        grow: |n| format!("CREATE INDEX ci ON c (a) WHERE a{} > 0", rep("+1", n)),
        stored: true,
        nested: false,
        floor: [1, 1],
    },
];

/// Run `setup` then the grown statement on a fresh in-memory database: `Ok` when all
/// succeed, else the failing statement's SQLSTATE.
fn run(shape: &Shape, n: usize) -> Result<Database, &'static str> {
    let mut db = Database::with_btree();
    for sql in shape.setup {
        db.execute(sql).expect("setup");
    }
    db.execute(&(shape.grow)(n)).map_err(|e| e.sqlstate())?;
    db.dump_sql().map_err(|e| e.sqlstate())?;
    Ok(db)
}

/// The SQLSTATEs a shape may be refused with past its edge.
const fn walls(shape: &Shape) -> &'static [&'static str] {
    if shape.nested {
        &["54001", "42601"]
    } else {
        &["54001"]
    }
}

/// The largest `n` the bounds admit on this thread: found by doubling, then
/// bisection. Each probe is a full run, and one that overflows is the failure this file
/// exists to see.
fn largest_answered(shape: &Shape) -> usize {
    assert!(
        run(shape, 1).is_ok(),
        "{}: one repetition must answer",
        shape.name
    );
    let mut lo = 1;
    let mut hi = 2;
    while run(shape, hi).is_ok() {
        lo = hi;
        hi *= 2;
        assert!(
            hi < 1 << 20,
            "{}: never refused — the shape is not bounded",
            shape.name
        );
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if run(shape, mid).is_ok() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The stored-expression leg: create at `n` in a file-backed database, insert a row
/// (evaluating what was stored), reopen — decoding it from the catalog — insert
/// again, render it and dump. A host must read back what it wrote.
fn stored_leg(shape: &Shape, n: usize) {
    let dir = ScratchDir::new("stack_depth");
    {
        let mut db = Database::open(&dir.path).expect("open");
        for sql in shape.setup {
            db.execute(sql).expect("setup");
        }
        db.execute(&(shape.grow)(n))
            .expect("the answered size answers file-backed");
        db.execute("INSERT INTO c VALUES (1)")
            .expect("insert evaluates the stored expression");
    }
    let mut db = Database::open(&dir.path).expect("the host reopens what it stored");
    db.execute("INSERT INTO c VALUES (2)")
        .expect("insert after reopen");
    db.execute("SELECT pg_get_constraintdef(oid) FROM pg_constraint")
        .expect("catalog render");
    db.execute("SELECT indexdef FROM pg_indexes")
        .expect("catalog render");
    db.execute("SELECT column_default FROM information_schema.columns")
        .expect("catalog render");
    db.dump_sql().expect("dump");
}

/// Every shape to its edge on this thread, then past it: returns each shape's edge.
fn drive_every_shape() -> Vec<usize> {
    let limit = pglet_sql::depth::max_depth();
    SHAPES
        .iter()
        .map(|shape| {
            let n = largest_answered(shape);
            eprintln!("{}: answers through {n}", shape.name);
            // Past the edge — one past it, a little past, and far past — the refusal is
            // one of the walls, and the tree never reaches a walker that cannot refuse,
            // `Drop` included. Near an edge the byte check sets, one size may still
            // answer where the one before it did not: a frame's layout is not a
            // function of `n` alone.
            for m in [n + 1, n + 2, n + 8, 2 * n, 4 * n, limit * 20] {
                match run(shape, m) {
                    Ok(_) => assert!(m <= n + 8, "{}: {m} answered past {n}", shape.name),
                    Err(code) => assert!(
                        walls(shape).contains(&code),
                        "{}: {m} refused {code}",
                        shape.name
                    ),
                }
            }
            if shape.stored {
                stored_leg(shape, n);
            }
            n
        })
        .collect()
}

/// Run `f` on a thread of `stack` bytes, which the engine measures with its fallback
/// budget: only a declared stack and a Linux main thread's are known to it.
fn on_thread<T: Send + 'static>(stack: usize, f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("no panic")
}

/// Run `f` on a thread of `stack` bytes, declared to the engine (`pglet::set_thread_stack`).
fn on_declared_thread<T: Send + 'static>(
    stack: usize,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    on_thread(stack, move || {
        pglet::set_thread_stack(stack);
        f()
    })
}

/// Every deep-tree shape to its edge and past it on a 2 MiB thread — the smallest a
/// caller may run on — and on an 8 MiB one, both declared, where each answers at least
/// as deep. Each answers at least what base answered on the small one.
#[test]
fn every_shape_answers_or_refuses_on_a_small_and_a_large_stack() {
    let small = on_declared_thread(SMALL_STACK, drive_every_shape);
    let large = on_declared_thread(LARGE_STACK, drive_every_shape);
    for ((shape, small), large) in SHAPES.iter().zip(small).zip(large) {
        assert!(
            small >= shape.floor(),
            "{}: answers through {small} on 2 MiB, below base's {}",
            shape.name,
            shape.floor()
        );
        assert!(
            large >= small,
            "{}: {large} on 8 MiB, {small} on 2 MiB",
            shape.name
        );
    }
}

/// A thread whose stack the engine does not know — any but a declared one or a Linux
/// main thread, since `/proc/self/maps` cannot say whose a thread mapping's low end is —
/// is measured against the fallback budget below the facade entry, which fits the
/// smallest stack a thread has by default on the target: every shape answers or is
/// refused there, never overflowing, and none answers deeper than on the same stack
/// declared.
#[test]
fn every_shape_answers_or_refuses_on_a_stack_the_engine_does_not_know() {
    let undeclared = on_thread(SMALL_STACK, drive_every_shape);
    let declared = on_declared_thread(SMALL_STACK, drive_every_shape);
    for ((shape, undeclared), declared) in SHAPES.iter().zip(undeclared).zip(declared) {
        assert!(
            undeclared <= declared + 8,
            "{}: {undeclared} undeclared, {declared} declared",
            shape.name
        );
    }
}

/// The fallback's arithmetic (`pglet_sql::depth::DEFAULT_STACK_BUDGET`): the budget, the
/// slop and the count's infallible walks fit the smallest stack a thread has by default
/// on this target, above 128 KiB of the caller's own frames.
#[test]
fn the_fallback_fits_the_smallest_default_stack() {
    use pglet_sql::depth::{
        DEFAULT_STACK_BUDGET, FALLBACK_MAX_DEPTH, INFALLIBLE_LEVEL_BYTES, STACK_SLOP,
    };
    // On Windows, a 1 MiB main thread less std's 20 KiB stack-overflow guarantee and a
    // 4 KiB guard page.
    let smallest = if cfg!(windows) {
        1024 * 1024 - 24 * 1024
    } else {
        2 * 1024 * 1024
    };
    let needed = 128 * 1024
        + DEFAULT_STACK_BUDGET
        + STACK_SLOP
        + FALLBACK_MAX_DEPTH * INFALLIBLE_LEVEL_BYTES;
    assert!(needed <= smallest, "{needed} B needed of {smallest}");
}

/// `AND` and `OR` are not bounded at all: 18.6 flattens either into one n-ary node and
/// answers a chain of 100 000 terms, and pglet builds a long one balanced. A chain far
/// past the limit answers, stores, renders, dumps and reloads.
#[test]
fn and_or_lists_far_past_the_limit_answer() {
    on_thread(SMALL_STACK, || {
        const TERMS: usize = 100_000;
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (a INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1), (2)").unwrap();
        for (sql, want) in [
            (
                format!(
                    "SELECT count(*) FROM t WHERE a = 1{}",
                    rep(" AND a = 1", TERMS)
                ),
                1,
            ),
            (
                format!(
                    "SELECT count(*) FROM t WHERE a = 0{}",
                    rep(" OR a = 2", TERMS)
                ),
                1,
            ),
        ] {
            let rows = &db.execute(&sql).expect("a long list answers")[0].rows;
            assert_eq!(rows[0][0], pglet_sql::types::Value::Int8(want));
        }
        // Distinct terms too — the shape a generated `WHERE` takes.
        for terms in [96, TERMS] {
            let rows = &db.execute(&where_and(terms)).expect("a long list answers")[0].rows;
            assert_eq!(rows[0][0], pglet_sql::types::Value::Int8(1));
        }
        let dir = ScratchDir::new("stack_depth_and");
        {
            let mut db = Database::open(&dir.path).unwrap();
            db.execute(&format!(
                "CREATE TABLE c (a INT CHECK (a > 0{}))",
                rep(" AND a > 0", 20_000)
            ))
            .expect("a long CHECK list stores");
            db.execute("INSERT INTO c VALUES (1)").unwrap();
            assert_eq!(
                db.execute("INSERT INTO c VALUES (0)")
                    .err()
                    .map(|e| e.sqlstate()),
                Some("23514")
            );
        }
        let mut db = Database::open(&dir.path).expect("reopen");
        db.execute("INSERT INTO c VALUES (2)").unwrap();
        let def = &db
            .execute("SELECT pg_get_constraintdef(oid) FROM pg_constraint")
            .unwrap()[0]
            .rows;
        let pglet_sql::types::Value::Text(def) = &def[0][0] else {
            panic!("text")
        };
        // One flat list, as 18.6 prints a `BoolExpr`: no operand is parenthesized as a
        // nested `AND`.
        assert!(
            def.starts_with("CHECK (((a > 0) AND (a > 0) AND"),
            "{}",
            &def[..60]
        );
        assert!(
            !def.contains("((a > 0) AND (a > 0)) AND"),
            "a nested pair in the render"
        );
        // The dump is psql input: its schema half, ahead of the first `COPY … FROM
        // stdin` and without psql's meta-commands, reloads through this host.
        let dump = db.dump_sql().unwrap();
        let schema_half = dump.split("\nCOPY ").next().unwrap();
        let sql: String = schema_half
            .lines()
            .filter(|l| !l.starts_with('\\'))
            .collect::<Vec<_>>()
            .join("\n");
        let mut restored = Database::with_btree();
        restored.execute(&sql).expect("the dump reloads");
        restored.execute("INSERT INTO c VALUES (3)").unwrap();
    });
}

/// The bound-parameter door (`Database::execute_bound`, which the wasm `queryParams`
/// and a driver's extended protocol reach) takes the same bounds, its parameter
/// inference included: a `$1` chain answers up to its edge and is refused past it,
/// never aborting.
#[test]
fn a_bound_parameter_chain_takes_the_same_bounds() {
    on_declared_thread(SMALL_STACK, || {
        let chain = |n: usize| format!("SELECT $1{}", rep("+$1", n));
        let bind = |_: &[Option<pglet_sql::types::TypeOid>]| {
            Ok::<_, String>(vec![Some(pglet_sql::types::Value::Int4(1))])
        };
        let mut db = Database::with_btree();
        let mut outcome = |sql: &str| match db.execute_bound(sql, 0, bind) {
            Ok(_) => None,
            Err(pglet::BoundExecError::Engine(e)) => Some(e.sqlstate()),
            Err(pglet::BoundExecError::Bind(e)) => panic!("bind: {e}"),
        };
        let mut lo = 1;
        let mut hi = 1 << 16;
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            match outcome(&chain(mid)) {
                None => lo = mid,
                Some(code) => {
                    assert_eq!(code, "54001", "{mid}");
                    hi = mid;
                }
            }
        }
        assert!(
            lo >= [2000, 90][usize::from(cfg!(debug_assertions))],
            "answers through {lo}"
        );
        assert_eq!(outcome(&chain(20_000)), Some("54001"));
        // The statement count ahead of the parse counts what 18.6's raw parse counts: a
        // trailing comment is no second statement (`54001` on 18.6 over an extended
        // `Parse`, measured), and a second statement still is.
        let long = chain(20_000);
        assert_eq!(outcome(&format!("{long}; -- c")), Some("54001"));
        assert_eq!(outcome(&format!("{long}; /* c */")), Some("54001"));
        assert_eq!(outcome("SELECT $1; -- c"), None);
        assert_eq!(outcome(&format!("{long}; SELECT 1")), Some("42601"));
    });
}

/// A store written where the bounds are larger — a server's connection thread — is
/// refused `54001` by a smaller opener, never overflowing it: every stored expression
/// is decoded under the opener's own bounds. A thread as large as the writer's opens it.
#[test]
fn a_store_too_deep_for_its_opener_is_refused_not_overflowed() {
    let dir = ScratchDir::new("stack_depth_carried");
    let path = dir.path.clone();
    let terms = pglet_sql::depth::DEFAULT_MAX_DEPTH + 100;
    let server = move |f: Box<dyn FnOnce() + Send>| {
        on_thread(pglet_sql::depth::SERVER_THREAD_STACK, move || {
            pglet_sql::depth::set_max_depth(pglet_sql::depth::SERVER_MAX_DEPTH);
            pglet_sql::depth::set_thread_stack(pglet_sql::depth::SERVER_THREAD_STACK);
            f();
        });
    };
    let written = path.clone();
    server(Box::new(move || {
        let mut db = Database::open(&written).unwrap();
        db.execute(&format!(
            "CREATE TABLE c (a INT CHECK (a{} > 0))",
            rep("+1", terms)
        ))
        .expect("the server's bounds store it");
    }));
    let opened = path.clone();
    let refusal = on_declared_thread(SMALL_STACK, move || {
        Database::open(&opened)
            .err()
            .and_then(std::io::Error::into_inner)
            .and_then(|e| e.downcast::<pglet::StoreTooDeep>().ok())
    })
    .expect("refused as StoreTooDeep");
    assert_eq!(refusal.sqlstate(), "54001");
    assert_eq!(refusal.max_depth(), pglet_sql::depth::DEFAULT_MAX_DEPTH);
    // The refusal names the limit and both ways out, and each works.
    let message = refusal.to_string();
    for part in [
        &format!("at most {} levels", pglet_sql::depth::DEFAULT_MAX_DEPTH),
        "Open and use it",
        "pglet::set_thread_stack",
        "pglet::set_thread_max_depth",
        "every statement on a thread that could not walk the catalog",
        "pglet_server dump --data-dir",
    ] {
        assert!(message.contains(part), "{message}");
    }
    let reopened = path.clone();
    server(Box::new(move || {
        let mut db = Database::open(&reopened).expect("a server-sized opener reads it");
        db.execute("INSERT INTO c VALUES (1)").unwrap();
    }));
    // The facade's own setters, on a thread as large as the server's: what the refusal
    // tells a caller to do.
    let facade = path;
    on_thread(pglet_sql::depth::SERVER_THREAD_STACK, move || {
        pglet::set_thread_stack(pglet_sql::depth::SERVER_THREAD_STACK);
        pglet::set_thread_max_depth(pglet_sql::depth::SERVER_MAX_DEPTH);
        let mut db = Database::open(&facade).expect("a declared, raised opener reads it");
        db.execute("INSERT INTO c VALUES (2)").unwrap();
    });
}

/// A caller that bounds the stack itself (`Database::set_stack_budget`) is held to its
/// budget, and a budget cannot widen a stack the engine knows.
#[test]
fn a_stack_budget_bounds_a_statement_and_cannot_widen_the_stack() {
    on_declared_thread(SMALL_STACK, || {
        let chain = |n: usize| format!("SELECT 1{}", rep("+1", n));
        let answers = |db: &mut Database, n: usize| db.execute(&chain(n)).is_ok();
        let mut db = Database::with_btree();
        let n = [400, 20][usize::from(cfg!(debug_assertions))];
        assert!(answers(&mut db, n));
        db.set_stack_budget(Some(64 * 1024));
        assert_eq!(
            db.execute(&chain(n)).err().map(|e| e.sqlstate()),
            Some("54001"),
            "a 64 KiB budget refuses what the thread answers"
        );
        assert!(answers(&mut db, 2), "a short statement fits any budget");
        // Far past the thread's own stack: the learned bound still holds, so the
        // statement is refused rather than overflowing.
        db.set_stack_budget(Some(1 << 40));
        assert_eq!(
            db.execute(&chain(1 << 16)).err().map(|e| e.sqlstate()),
            Some("54001")
        );
        db.set_stack_budget(None);
        assert!(answers(&mut db, n));
    });
}

/// A plan under an expression is cloned wherever the expression is — no site can check
/// its room — so each of its levels is charged `EMBEDDED_PLAN_LEVEL_WEIGHT` against the
/// limit. A `GROUP BY` over a scalar subquery clones its `UNION ALL` chain: charged one
/// level a plan level, a 2 MiB release thread overflowed at 2 500 arms.
#[test]
fn a_plan_under_an_expression_is_charged_what_cloning_it_costs() {
    on_thread(SMALL_STACK, || {
        let limit = pglet_sql::depth::max_depth();
        // The band that overflowed sits just inside the limit, so the sweep is fine.
        for arms in (1..=42).map(|k| limit * k / 40) {
            let mut db = Database::with_btree();
            for sql in TABLE {
                db.execute(sql).expect("setup");
            }
            let sql = format!(
                "SELECT (SELECT x FROM (SELECT 1 AS x{}) s LIMIT 1) AS y FROM t GROUP BY y",
                rep(" UNION ALL SELECT 1", arms)
            );
            // Answered or refused — `0A000` is pglet's own for the grouping — but here.
            let _ = db.execute(&sql);
        }
    });
}

/// Rows `sql`'s single count answers.
fn count(db: &mut Database, sql: &str) -> i64 {
    match db
        .execute(sql)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .remove(0)
        .rows[0][0]
    {
        pglet_sql::types::Value::Int8(n) => n,
        ref other => panic!("{sql}: {other:?}"),
    }
}

/// The outcome of each statement a database takes, run where it cannot walk its
/// catalog: every one is refused `54001` at the facade's entry, before anything of it
/// runs — reads included, since the refusal is the catalog's and not the statement's.
///
/// **The discriminating assertion is the first**: `SELECT 1` walks no stored expression
/// and publishes nothing, so it is refused only by the entry check — a check left to the
/// walks themselves answers it, and refuses the writes after their effects.
fn every_statement_is_refused(db: &mut Database) {
    let refused =
        |r: Result<Vec<pglet::ExecuteResult>, pglet::Error>| r.err().map(|e| e.sqlstate());
    for sql in [
        "SELECT 1",
        "INSERT INTO p VALUES (2)",
        "CREATE TABLE o (x INT)",
        "DROP TABLE p",
        "SELECT nextval('s')",
        "BEGIN",
        "SAVEPOINT a",
        "COMMIT",
        "ROLLBACK",
        "INSERT INTO p VALUES (2); SELECT 1",
    ] {
        assert_eq!(refused(db.execute(sql)), Some("54001"), "{sql}");
    }
    let code = |r: Result<(), pglet::Error>| r.err().map(|e| e.sqlstate());
    assert_eq!(
        code(db.copy_in("COPY p FROM STDIN", b"3\n").map(drop)),
        Some("54001")
    );
    assert_eq!(
        code(db.copy_out("COPY p TO STDOUT").map(drop)),
        Some("54001")
    );
    assert_eq!(code(db.begin(0)), Some("54001"));
    assert_eq!(code(db.commit(0)), Some("54001"));
    assert_eq!(code(db.rollback(0)), Some("54001"));
    assert_eq!(code(db.sync()), Some("54001"));
    assert_eq!(code(db.dump_sql().map(drop)), Some("54001"));
    assert_eq!(code(db.fork().map(drop)), Some("54001"));
    assert_eq!(
        db.execute_bound("INSERT INTO p VALUES ($1)", 0, |_| {
            Ok::<_, ()>(vec![Some(pglet_sql::types::Value::Int4(4))])
        })
        .err()
        .map(|e| match e {
            pglet::BoundExecError::Engine(e) => e.sqlstate(),
            pglet::BoundExecError::Bind(()) => "bind",
        }),
        Some("54001")
    );
}

/// What a database at `path` holds, read on a server-sized thread: `p`'s rows, the
/// relations named `o`, `s`'s counter, and — through `db` when it is the session —
/// the block's status.
fn holdings(db: &mut Database) -> (i64, i64, i64, pglet::TransactionStatus) {
    (
        count(db, "SELECT count(*) FROM p"),
        count(db, "SELECT count(*) FROM pg_class WHERE relname = 'o'"),
        db.schema().sequences["s"].last_value,
        db.transaction_status(),
    )
}

/// Run `f` on a thread as large as a server connection's, its stack declared and its
/// limit raised to the server's.
fn on_server_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    on_declared_thread(pglet_sql::depth::SERVER_THREAD_STACK, move || {
        pglet::set_thread_max_depth(pglet_sql::depth::SERVER_MAX_DEPTH);
        f()
    })
}

/// The session's holdings and then the disk's, both read on a server-sized thread.
fn session_and_disk(
    db: Database,
    path: std::path::PathBuf,
) -> [(i64, i64, i64, pglet::TransactionStatus); 2] {
    on_server_thread(move || {
        let mut db = db;
        let session = holdings(&mut db);
        drop(db);
        let mut reopened = Database::open(&path).expect("reopen");
        [session, holdings(&mut reopened)]
    })
}

/// What seeding leaves, in session and on disk: `p`'s one row, no `o`, `s` advanced once.
const SEEDED: (i64, i64, i64, pglet::TransactionStatus) = (1, 0, 1, pglet::TransactionStatus::Idle);

/// The stored expression's depth in these tests: inside the thread's default limit.
const REFUSED_TERMS: usize = if cfg!(debug_assertions) { 100 } else { 1000 };

/// The two shapes a stored expression can carry the catalog's depth in, and what each
/// level of one costs a walk of it.
///
/// **A nesting cannot be deep.** `parser::MAX_DEPTH` refuses one past 128 levels, so a
/// catalog deeper than that holds a *chain* — the cheapest shape a level can be — and a
/// seed grown to a thousand levels has no choice about which it drives. The two together
/// are the range of `depth::ENCODE_LEVEL_BYTES`'s product, cheap-and-deep and
/// dear-and-shallow, and the edge sweep runs over both.
#[derive(Clone, Copy)]
enum DeepShape {
    /// An operator chain, `a+1+1+…` — 188 B a level to encode, and reachable at any
    /// depth the thread's limit admits.
    Chain,
    /// A `coalesce` nesting, whose argument list is a frame of its own — the dearest
    /// level measured (240 B release), and the shape `ENCODE_LEVEL_BYTES` is rounded up
    /// from. Reachable only to the parser's nesting wall.
    Nesting,
}

/// The deepest a [`DeepShape::Nesting`] seed goes: inside `parser::MAX_DEPTH` (128) with
/// room for the `CHECK`'s own levels above it.
const DEEPEST_NESTING: usize = 120;

impl DeepShape {
    /// The `CHECK` body of this shape at `levels` levels, as a predicate over `a`.
    fn check_body(self, levels: usize) -> String {
        match self {
            Self::Chain => format!("a{} > 0", rep("+1", levels)),
            Self::Nesting => format!("{}a{} > 0", rep("coalesce(", levels), rep(", 0)", levels)),
        }
    }
}

/// A file-backed database at `path` holding a `CHECK` of `terms` chain levels, `p` with
/// one row, and a sequence `s` advanced once.
fn seed_deep_catalog(path: &std::path::Path, terms: usize) -> Database {
    seed_deep_catalog_shaped(path, DeepShape::Chain, terms)
}

/// [`seed_deep_catalog`] over a chosen shape.
fn seed_deep_catalog_shaped(path: &std::path::Path, shape: DeepShape, levels: usize) -> Database {
    let mut db = Database::open(path).expect("open");
    db.execute(&format!(
        "CREATE TABLE c (a INT CHECK ({})); CREATE TABLE p (x INT); \
         INSERT INTO p VALUES (1); CREATE SEQUENCE s; SELECT nextval('s')",
        shape.check_body(levels)
    ))
    .expect("seed");
    db
}

/// Recurse `kib` KiB down the stack, then run `f` there: a caller already partway down
/// its own stack when it calls the facade.
fn burn(kib: usize, f: &mut dyn FnMut()) {
    let frame = [0_u8; 1024];
    std::hint::black_box(&frame);
    if kib == 0 { f() } else { burn(kib - 1, f) }
}

// A statement can walk the whole catalog, and most of those walks cannot fail — a
// `BEGIN`'s snapshot is a `Clone` of it — so a database is refused every statement, up
// front, on a thread that could not walk its catalog: the count lowered after the DDL, a
// narrower budget, a smaller thread. Each route runs twice, from the thread's top and
// from a caller partway down its stack; each refused statement leaves no effect in the
// session or on disk. The abort a missing check produces is also a red.

/// How far down a caller is on each route's second run, in KiB.
const PARTWAY: [usize; 2] = [0, 256];

/// The count, lowered below the stored expression's depth after it was stored.
#[test]
fn a_database_is_refused_every_statement_under_a_lowered_count() {
    for partway in PARTWAY {
        let dir = ScratchDir::new("stack_depth_refused_count");
        let path = dir.path.clone();
        let db = on_declared_thread(LARGE_STACK, move || {
            let mut db = seed_deep_catalog(&path, REFUSED_TERMS);
            let previous = pglet::set_thread_max_depth(REFUSED_TERMS / 2);
            burn(partway, &mut || every_statement_is_refused(&mut db));
            pglet::set_thread_max_depth(previous);
            db
        });
        assert_eq!(
            session_and_disk(db, dir.path.clone()),
            [SEEDED; 2],
            "{partway} KiB down"
        );
    }
}

/// A stack budget narrowed below the room the catalog's encoding takes.
#[test]
fn a_database_is_refused_every_statement_under_a_narrowed_budget() {
    for partway in PARTWAY {
        let dir = ScratchDir::new("stack_depth_refused_budget");
        let path = dir.path.clone();
        let db = on_declared_thread(LARGE_STACK, move || {
            let mut db = seed_deep_catalog(&path, REFUSED_TERMS);
            db.set_stack_budget(Some([64, 256][usize::from(cfg!(debug_assertions))] * 1024));
            burn(partway, &mut || every_statement_is_refused(&mut db));
            db.set_stack_budget(None);
            db
        });
        assert_eq!(
            session_and_disk(db, dir.path.clone()),
            [SEEDED; 2],
            "{partway} KiB down"
        );
    }
}

/// Opened and written on a server-sized thread, then used on a small one: a stored
/// expression past the small thread's limit refuses everything there, a `BEGIN` among
/// them — whose snapshot of the catalog overflowed a 2 MiB thread and aborted the
/// process before this check.
#[test]
fn a_database_is_refused_every_statement_on_a_smaller_thread_than_its_catalog() {
    for partway in PARTWAY {
        let dir = ScratchDir::new("stack_depth_refused_moved");
        let path = dir.path.clone();
        let db = on_server_thread(move || {
            seed_deep_catalog(&path, pglet_sql::depth::DEFAULT_MAX_DEPTH + 100)
        });
        let db = on_declared_thread(SMALL_STACK, move || {
            let mut db = db;
            burn(partway, &mut || every_statement_is_refused(&mut db));
            db
        });
        assert_eq!(
            session_and_disk(db, dir.path.clone()),
            [SEEDED; 2],
            "{partway} KiB down"
        );
    }
}

/// A caller far enough down a small thread that the catalog's encoding would not fit
/// below it, though its depth is inside the thread's limit: the room half of the check
/// refuses what the count half admits.
#[test]
fn a_database_is_refused_every_statement_where_its_encoding_would_not_fit() {
    let dir = ScratchDir::new("stack_depth_refused_room");
    let path = dir.path.clone();
    let db = on_server_thread(move || seed_deep_catalog(&path, REFUSED_TERMS));
    let db = on_declared_thread(SMALL_STACK, move || {
        let mut db = db;
        // Deep enough that the frame is within the encoding's room of the limit, and not
        // so deep that it is past the limit itself.
        let room = REFUSED_TERMS * pglet_sql::depth::ENCODE_LEVEL_BYTES
            + pglet_sql::depth::ENCODE_ENTRY_BYTES;
        let above_limit = SMALL_STACK - 64 * 1024 - pglet_sql::depth::stack_reserve();
        assert!(room < above_limit, "the room fits the thread from its top");
        burn((above_limit - room / 2) / 1024 - 16, &mut || {
            every_statement_is_refused(&mut db);
        });
        db
    });
    assert_eq!(session_and_disk(db, dir.path.clone()), [SEEDED; 2]);
}

/// A block opened where the catalog could be walked, then used where it cannot: every
/// statement is refused, and the block is left as it was — not aborted, not ended — so
/// the same `COMMIT` made again from a thread that walks the catalog commits it.
#[test]
fn a_refused_statement_leaves_an_open_block_as_it_was() {
    let dir = ScratchDir::new("stack_depth_refused_block");
    let path = dir.path.clone();
    let db = on_server_thread(move || {
        let mut db = seed_deep_catalog(&path, pglet_sql::depth::DEFAULT_MAX_DEPTH + 100);
        db.execute("BEGIN; INSERT INTO p VALUES (7); SAVEPOINT a")
            .expect("the block");
        db
    });
    let db = on_declared_thread(SMALL_STACK, move || {
        let mut db = db;
        every_statement_is_refused(&mut db);
        assert_eq!(
            db.transaction_status(),
            pglet::TransactionStatus::InTransaction
        );
        db
    });
    let path = dir.path.clone();
    let db = on_server_thread(move || {
        let mut db = db;
        assert_eq!(
            count(&mut db, "SELECT count(*) FROM p"),
            2,
            "the block's row, pending"
        );
        db.execute("COMMIT")
            .expect("the refused COMMIT, made again");
        db
    });
    assert_eq!(
        session_and_disk(db, path),
        [(2, 0, 1, pglet::TransactionStatus::Idle); 2]
    );
}

/// Across the check's edge, every statement answers alike: at each budget either all of
/// them run — and what they did is on disk — or all are refused. A statement refused
/// where `SELECT 1` runs is one the entry admitted and a walk inside it refused, which is
/// the refusal falling after a write: a room constant set below what the encoding takes
/// does exactly that.
///
/// Run over **both** [`DeepShape`]s — for shape coverage, not for a tighter bound on
/// `ENCODE_LEVEL_BYTES`. Bisecting that constant one leg at a time (release) puts the
/// chain leg's crossing between 150 and 200 and the nesting leg's between 80 and 120, so
/// the *cheap* shape is the sharper probe: the check reserves
/// `depth × ENCODE_LEVEL_BYTES + ENCODE_ENTRY_BYTES`, and a shape the parser's nesting
/// wall keeps to 120 levels has 8× less leverage on the per-level term than a 1 000-level
/// chain. What the nesting leg adds is that the sweep's all-or-nothing property is
/// asserted over a `coalesce` tree's clone, restore and encode as well as a chain's.
#[test]
fn every_statement_answers_alike_across_the_edge() {
    for (shape, levels) in [
        (DeepShape::Chain, REFUSED_TERMS),
        (DeepShape::Nesting, DEEPEST_NESTING),
    ] {
        every_statement_answers_alike_for(shape, levels);
    }
}

fn every_statement_answers_alike_for(shape: DeepShape, levels: usize) {
    let dir = ScratchDir::new("stack_depth_edge");
    let path = dir.path.clone();
    on_declared_thread(LARGE_STACK, move || {
        let mut db = seed_deep_catalog_shaped(&path, shape, levels);
        let room =
            levels * pglet_sql::depth::ENCODE_LEVEL_BYTES + pglet_sql::depth::ENCODE_ENTRY_BYTES;
        let step = [2, 8][usize::from(cfg!(debug_assertions))] * 1024;
        let (mut ran, mut refused) = (0, 0);
        let mut budget = room / 2;
        let mut rows = 1;
        while budget < room + 16 * step {
            db.set_stack_budget(Some(budget));
            let answers: Vec<Option<&str>> = [
                "SELECT 1",
                "INSERT INTO p VALUES (2)",
                "CREATE TABLE o (x INT CHECK (x > 0)); DROP TABLE o",
                "BEGIN; SAVEPOINT a; INSERT INTO p VALUES (3); COMMIT",
                "SELECT nextval('s')",
            ]
            .iter()
            .map(|sql| db.execute(sql).err().map(|e| e.sqlstate()))
            .collect();
            let first = answers[0];
            assert!(
                answers.iter().all(|a| *a == first),
                "{budget} B: {answers:?}"
            );
            match first {
                None => {
                    ran += 1;
                    rows += 2;
                }
                Some("54001") => refused += 1,
                Some(other) => panic!("{budget} B: {other}"),
            }
            db.set_stack_budget(None);
            assert_eq!(count(&mut db, "SELECT count(*) FROM p"), rows, "{budget} B");
            budget += step;
        }
        assert!(
            ran > 0 && refused > 0,
            "the sweep crosses the edge: {ran} ran, {refused} refused"
        );
        drop(db);
        let mut reopened = Database::open(&path).expect("reopen");
        assert_eq!(count(&mut reopened, "SELECT count(*) FROM p"), rows);
    });
}

/// A declared stack is held to what the kernel shows of it: where the mapping the
/// thread runs in starts above the declared low end, the declaration overstates the
/// stack, and the mapping's start is the low end. A 2 MiB thread declared 4 MiB aborted
/// on a CTE chain at 2 048 (`SIGSEGV`) — the reserve measured from a low end 2 MiB
/// below the real one. Linux only: elsewhere no mapping is read, and the declaration is
/// the caller's to get right.
#[cfg(target_os = "linux")]
#[test]
fn a_stack_declared_larger_than_its_mapping_is_held_to_the_mapping() {
    let over = on_thread(SMALL_STACK, || {
        pglet::set_thread_stack(2 * SMALL_STACK);
        drive_every_shape()
    });
    // Past its edge every shape was refused, not overflowed — the abort is the red. And
    // it answers about what a true declaration does, not what twice the stack would: the
    // mapping's start sits a little below a true declaration's low end, which assumes
    // `TOP_OF_STACK` above the entry frame.
    let declared = on_declared_thread(SMALL_STACK, drive_every_shape);
    for ((shape, over), declared) in SHAPES.iter().zip(over).zip(declared) {
        assert!(
            over <= declared + declared / 5 + 8,
            "{}: {over} over-declared, {declared} declared truly",
            shape.name
        );
    }
}
