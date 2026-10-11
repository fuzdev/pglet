//! The two deep declared-type walkers, pinned side by side over one `Expr` corpus.
//!
//! pglet computes an output column's declared type twice, through two separate walks
//! that feed two different clients:
//!
//! - **W1** — [`expr_type_oid`](super::helpers) (reached here through its production
//!   boundary [`expr_column_info`]), resolving against a [`ProjectionScope`] of
//!   already-materialized descriptors. It is **total**: an operand it cannot type
//!   arrives at the next rule as `TypeOid::Text`. It feeds `QueryResult::shape` → the
//!   simple-query `RowDescription`, the column-type-aware text encoder in *both*
//!   protocols, and two accumulator selections in `executor::query::aggregate`.
//! - **W2** — [`Expr::infer_type`], resolving its leaves through a
//!   [`LeafScope`](crate::planner::LeafScope) — the planner's implementation being a
//!   `TypeScope` paired with the catalog `Schema`. It returns `Option`. It feeds
//!   `Plan::result_shape` → `Describe` → the extended-protocol `RowDescription`, the
//!   DDL static gate, and `$N` inference.
//!
//! # What these pins are for
//!
//! They are the **ratchet** for the walker-unification work, not a statement that
//! today's answers are right. Several pinned pairs are *divergences* — the two walks
//! answer differently for the same expression, so one wire protocol declares a
//! different OID from the other for one statement. Three have been measured reaching a
//! client on **both** legs from ordinary SQL, and each is recorded in `EDGES.md`: the `abs(…)`
//! family, the projection-position
//! `unnest(<non-array>)` split — whose direct-column and literal spellings are now
//! refused `42883` at plan time, leaving a *computed* argument as what reaches it (cell
//! F) — and the arithmetic-unknown family (`<text> + <int>`,
//! `<real> + NULL`, `NULL ^ <numeric>`). The rest are pinned as facts about the *arms*,
//! and reachability is a per-cell question each test answers only as far as it has been
//! measured. Silence about a cell here is not a
//! claim that nothing reaches it, and cell C is the measured cost of reading it as one:
//! it was recorded **dead** from the simple-query leg, and it was live wherever a
//! `sum`/`min`/`max` whose argument this walk answers `text` for — a genuinely `text`
//! argument included — ran over a FROM whose relations are **all zero-column**, which
//! ordinary SQL builds. The guard read `scope.columns.is_empty()`, a property of the
//! projecting node's *whole input*, so one columned relation anywhere in the FROM
//! defeated it: measured pre-deletion on the simple leg, `SELECT max('abc') FROM zc a,
//! zc b` declared `701` and `SELECT max('abc') FROM zc, t2` declared `25`.
//! It wrongly declared that aggregate's type *and* laundered the guess past an
//! enclosing aggregate's legality gate (see that cell's test). The arm is gone and the
//! cell is merged. A later change may alter a pinned answer only in a commit whose
//! subject **is** that cell, never as a
//! side effect of a refactor: the point of a refactoring change is that every assertion below
//! survives it unchanged.
//!
//! # Why the corpus is raw `Expr` values rather than SQL
//!
//! Several of these differences are **unreachable from SQL text**, because the planner's
//! coercion pass rewrites an untyped branch into an `Expr::Cast` *before* either walker
//! runs, so those SQL spellings arrive already typed: measured on both pglet legs and on
//! 18.6, `ARRAY[NULL,1]` is `int4[]` on all three, and `COALESCE(NULL,1)`,
//! `CASE WHEN true THEN NULL ELSE 1 END`, `greatest(NULL,1)` and `NULL + 1` are `int4`,
//! `least(NULL,b)` and `nullif(NULL,b)` over a `bigint` column `int8`. Building the
//! `Expr` directly is the only way to reach those arms — so a differential fixture is
//! the wrong instrument for them, and these pins move with the *pass* only insofar as it
//! changes the walkers.
//!
//! **The rewrite is a property of the branch set, not of the cell**, and reading it as
//! the cell's ("E/H/I/J are masked by the coercion pass") hid a family that was live on
//! both legs the whole time. The mechanism is what to carry, in two readers:
//!
//! - `COALESCE` / `CASE` / `ARRAY[…]` go through `planner::coercion`'s
//!   `unify_common_type_branches`, which asks `select_common_type` for a target and
//!   returns the branches **untouched** on `Ok(None)` — the answer when no branch is
//!   concrete to `concrete_branch_type`;
//! - `greatest`/`least` go through `unify_variadic_args` and `nullif` through
//!   `coerce_nullif_args`, both over `arg_known_type`, which likewise yields no target
//!   when no argument is known.
//!
//! **The two readers are not one walk**, and what separates them is an arm list rather
//! than a policy: over an otherwise identical `Column` / `Cast` / `CastTypmod` / literal
//! set, `concrete_branch_type` carries an `Aggregate` arm (its argument's type,
//! recursed), an `Array` arm (the elements' common type, one dimension up) and an
//! `Alias` arm that `arg_known_type` does not. So `COALESCE(NULL, sum(b))` and
//! `COALESCE(NULL, ARRAY[1,2])` are rewritten and were never in this family — `1700`
//! (`sum(bigint)`'s `numeric`) and `1007` on both legs, as on 18.6 — while `greatest(NULL, ARRAY[1,2])` and its
//! `least`/`nullif` spellings **were** in it, `25` on the simple leg against `Describe`'s
//! `1007` where 18.6 says `1007`. Those three are members this hook moved that the
//! fixtures do not name (no fixture builds an array beside a bare `NULL` in a variadic).
//!
//! The `Alias` arm is **reachable and moves no declared type**, which is a different
//! fact from the `Array` one beside it — and a narrower one than "carries no
//! divergence", which is what stood here. `set_config(<name>, <value>,
//! <is_local>)` lowers to an `Expr::Alias` wrapping `<value>` (`planner::expr`) — the one
//! production `Alias` mint that nests inside another expression, the others
//! (projection item, `VALUES` row, sort junk, `JOIN … USING` merged column,
//! `planner::query`'s `coerce_unknown_entry` — a set-op side's `unknown` output entry
//! rewritten to a `Cast` the alias names — and `carried_key`, which re-wraps a bare
//! `Int` literal lifted out of a projection entry so the sort key it becomes is not read
//! back as positional) all
//! sitting at a list's top level. `carried_key` is the member an enumeration keeps
//! dropping, because it mints on the **right of a `match` arm** rather than into a
//! `push` or a `return` — a sweep that reads `=>` lines as patterns rather than as
//! constructors never sees it. It replaces the whole `OrderKey.expr` and wraps nothing
//! else, so it moves no declared type either. Measured over
//! `(i INT, tx TEXT)`: `COALESCE(NULL, set_config('x', tx, false))` and its
//! `greatest`/`least`/`nullif` spellings are `25` on both legs, the `i` spelling `23` on
//! both — both walks reading through the alias to the inner expression either way. The
//! `i` spelling is pglet-only
//! (18.6 refuses `set_config(unknown, integer, boolean)` `42883`); over the `tx` one 18.6
//! declares `25` too, then fails at run time on the unknown GUC.
//!
//! **What the arm `arg_known_type` lacks moves is a gate verdict, not a declared type**,
//! and that is a consequence of this arm list the declared-type question cannot see. The
//! `42804` gate hands `concrete_branch_type` to all five constructs it covers —
//! `COALESCE`, `CASE`, `ARRAY[…]`, `greatest`, `least`; `nullif` is not in that match at
//! all and reaches the operand gate beside it instead — so over a
//! `greatest`/`least`/`nullif` the pass is the *blinder* of the two readers: it folds the
//! concrete sibling alone and wraps the argument it cannot see in a `Cast` to that
//! target, and the gate then reads that `Cast` at an agreeing type and never fires.
//! **That needs a concrete sibling to fold**: with none, `variadic_common_type` answers
//! `None`, the pass rewrites nothing and the gate types the raw pair itself —
//! `greatest(ARRAY[1,2], set_config('x', tx, false))` is `42804` on both engines. Over
//! `(i INT, tx TEXT, a INT[])` holding `(7, '7', {1,2})`, on both legs:
//! `greatest(ARRAY[1,2], tx)` answers `25`/`{1,2}`, `least` `25`/`7`, `nullif`
//! `25`/`{1,2}`, and `greatest`/`least`/`nullif` of `(set_config('x', tx, false), i)`
//! answer `23`/`7`, `23`/`7` and `23`/NULL — where 18.6 refuses each `42804` (`nullif`
//! `42883`). The same `int[]` through an arm **both** readers carry is refused as it
//! should be, `greatest(a, tx)` and `greatest(a::int[], tx)` each `42804` on both
//! engines, and the `COALESCE` family — where pass and gate read the one reader — is
//! untouched, `COALESCE(i, ARRAY[1,2])` and `COALESCE(set_config('x', tx, false), i)`
//! each `42804` on both. `EDGES.md` row 65's third sub-case carries the streams and the
//! bounds.
//!
//! What both readers *do* share is a `None` for every **computed** expression *outside*
//! the three arms just named and the numeric-category computed shapes both type at the
//! computed reach (`planner::coercion`'s `BranchReach`) — a string function but
//! `length`/`strpos`/`position`, temporal arithmetic, a subquery, a `$N`, a `DEFAULT`,
//! and any of those shapes over an operand whose type is not certain. The scope is the
//! point: `sum(b)` is a function
//! call and is **not** `None` to `concrete_branch_type`, which is the whole of the arm
//! list above. So a branch set of
//! `{a bare NULL} ∪ {a computed expression}` carries no target and reaches both walkers
//! un-rewritten. Measured live on 18.6 and both pglet legs before the fix, in every
//! projecting position tried: a plain projection, `RETURNING`, a `GROUP BY`/`ORDER BY`
//! ordinal, a derived table, a `VALUES` row, a scalar-subquery operand, an `unnest`
//! wrapper, a nested `COALESCE`, an empty relation and `LIMIT 0`. The fixtures named on
//! the multi-operand test are that family's guards; the pins keep the arm-level facts a
//! fixture cannot reach.
//!
//! **`Ok(None)` has a third consequence, and it is not a declared type at all.** The
//! branch-unification **gate** — `planner::coercion`'s `check_branch_unification_node`,
//! which stands in for 18.6's `42804` — reads that same `concrete_branch_type`, and
//! `select_common_type` folds only the branches it resolves, so an invisible branch is
//! never compared against a sibling **whether or not the set has a concrete member**.
//! These branch sets are the sub-case where nothing at all is compared — the fold is
//! `[None, None]`, the answer `Ok(None)`, and nothing is raised:
//! `COALESCE(length('ab'), upper('7'))` answers int4 `2` here and
//! `ARRAY[length('ab'), upper('7')]` answers `_int4` `{2,7}` where 18.6 refuses `42804`
//! at parse analysis. The walkers declare the first *inferable* branch while evaluation
//! returns the branch it takes, so the two can come from different branches over one
//! statement: `CASE WHEN false THEN length('ab') ELSE upper('x') END` declares `23` and
//! streams the single byte `58`. With a concrete sibling the pass casts instead, which
//! keeps the declaration and the value together and leaves only the missing refusal —
//! `COALESCE(i, upper(tx))` answers `7`. `COALESCE(i, tx)` is the control that says the
//! gate exists — it is `42804` on both. That is `EDGES.md` row 65, and it shares row
//! 54's fold band's root:
//! widening `concrete_branch_type`/`arg_known_type` past everything they answer `None`
//! for — the computed expressions and the `Expr::Bound` constants — closes both. The
//! numeric-category shapes are widened, and a set those type only in part keeps the
//! written reading this paragraph's members are measured over, so they are unmoved. It
//! closes a third as well: `arg_known_type` is also what the new
//! `unnest(<non-array>)` gate reads, so every argument that reader's `_` arm swallows —
//! a computed expression, and equally an `Expr::Bound` or an `Expr::Alias` — is a shape
//! that gate declines and cell F's split therefore still reaches (cell F).
//!
//! Every value below was measured from this code, not copied from a design note.

use super::helpers::{ProjectionScope, expr_column_info};
use crate::catalog::Schema;
use crate::parser::parse;
use crate::planner::{
    AggregateFunc, BinaryOp, BoolTest, BuiltinFn, Expr, LeafScope, Literal, MathFunc, Plan,
    PlannerScope, SequenceFunc, StringFunc, TypeScope, Typmod, VariadicFunc, WindowFunc,
};
use crate::types::{ColumnDesc, TypeOid, Value};

/// The executor walk's answer — total, so always a `TypeOid`.
fn w1(expr: &Expr, scope: ProjectionScope<'_>) -> TypeOid {
    expr_column_info(expr, scope).1
}

/// The planner walk's answer. The **one** call site of `Expr::infer_type` in this
/// module, so a change to that walk's signature touches one line here.
fn w2(expr: &Expr, schema: &Schema, scope: TypeScope<'_>) -> Option<TypeOid> {
    expr.infer_type(&PlannerScope::new(schema, scope))
}

/// `t (b BIGINT, tx TEXT, ts TIMESTAMPTZ, n NUMERIC, f4 REAL, f8 DOUBLE PRECISION)` — the
/// catalog half of the paired scopes.
///
/// The three numeric columns are here for cell K alone. `n` and `f4` are the two types
/// whose arithmetic promotion against an un-inferable operand is not the type itself,
/// which is what makes that cell's two live divergences reachable; `f8` is the control
/// that says `f4` reaches one of them for its *width* and not for being a float.
fn schema_t() -> Schema {
    let mut schema = Schema::new();
    let p = parse(
        "CREATE TABLE t (b BIGINT, tx TEXT, ts TIMESTAMPTZ, n NUMERIC, f4 REAL, f8 DOUBLE \
         PRECISION)",
        &schema,
    )
    .unwrap()
    .remove(0);
    crate::executor::execute(&p, &mut schema).unwrap();
    schema
}

/// The descriptor half of the paired scopes — the same columns `schema_t` declares, as a
/// projecting node's materialized input.
fn input_columns() -> [ColumnDesc; 6] {
    [
        ColumnDesc::new("b", TypeOid::Int8),
        ColumnDesc::new("tx", TypeOid::Text),
        ColumnDesc::new("ts", TypeOid::Timestamptz),
        ColumnDesc::new("n", TypeOid::Numeric),
        ColumnDesc::new("f4", TypeOid::Float4),
        ColumnDesc::new("f8", TypeOid::Float8),
    ]
}

const INPUT_NAMES: [&str; 6] = ["b", "tx", "ts", "n", "f4", "f8"];

fn lit_null() -> Expr {
    Expr::Literal(Literal::Null)
}

fn lit_int(n: i64) -> Expr {
    Expr::Literal(Literal::Int(n))
}

fn lit_str(s: &str) -> Expr {
    Expr::Literal(Literal::String(s.to_string()))
}

fn col(name: &str) -> Expr {
    Expr::Column {
        table: None,
        name: name.to_string(),
    }
}

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

/// Assert both walks over the no-input scopes (`ProjectionScope::empty()` /
/// `TypeScope::NONE`) — the shape of a no-FROM projection item.
fn assert_no_input(label: &str, expr: &Expr, want_w1: TypeOid, want_w2: Option<TypeOid>) {
    let schema = Schema::new();
    assert_eq!(
        w1(expr, ProjectionScope::empty()),
        want_w1,
        "executor walk over {label}"
    );
    assert_eq!(
        w2(expr, &schema, TypeScope::NONE),
        want_w2,
        "planner walk over {label}"
    );
}

/// The cases whose rule reads nothing but the node itself, so there is no unknown-ness
/// for the two walks to carry differently.
///
/// The polarity half of the corpus: a merge of the two walks that got any of these
/// wrong would be caught here rather than only by the differential harness, which
/// cannot reach several of them at all with an untyped operand.
///
/// It is a list of **cases, not of variants**, and several variants appear both here
/// and in the divergent groups below — `count(NULL)`'s result type is fixed where
/// `sum(NULL)`'s follows its argument; `Bound(Value::Int4)` carries a type where
/// `Bound(Value::Null)` has none; a typed literal answers where a bare `NULL` does not.
/// That is the point: what splits the two walks is a property of the **operand**, not
/// of the variant it sits under.
#[test]
fn variants_with_no_unknown_ness_to_carry_agree_on_both_walks() {
    let schema = schema_t();
    let scan = Plan::Scan {
        table: "t".to_string(),
        alias: None,
    };
    let cases: Vec<(&str, Expr, TypeOid)> = vec![
        ("an int literal", lit_int(1), TypeOid::Int4),
        ("a string literal", lit_str("x"), TypeOid::Text),
        ("negation", Expr::Negate(boxed(lit_int(1))), TypeOid::Int4),
        (
            "unary plus",
            Expr::UnaryPlus(boxed(lit_int(1))),
            TypeOid::Int4,
        ),
        (
            "NOT",
            Expr::Not(boxed(Expr::Literal(Literal::Bool(true)))),
            TypeOid::Bool,
        ),
        ("IS NULL", Expr::IsNull(boxed(lit_null())), TypeOid::Bool),
        (
            "IS NOT NULL",
            Expr::IsNotNull(boxed(lit_null())),
            TypeOid::Bool,
        ),
        (
            "IS TRUE over a NULL operand",
            Expr::BooleanTest {
                expr: boxed(lit_null()),
                test: BoolTest::True,
            },
            TypeOid::Bool,
        ),
        (
            "IS NOT UNKNOWN over a NULL operand",
            Expr::BooleanTest {
                expr: boxed(lit_null()),
                test: BoolTest::NotUnknown,
            },
            TypeOid::Bool,
        ),
        (
            "a cast",
            Expr::Cast {
                expr: boxed(lit_null()),
                target_type: TypeOid::Int4,
            },
            TypeOid::Int4,
        ),
        (
            "a typmod'd cast",
            Expr::CastTypmod {
                expr: boxed(lit_null()),
                type_oid: TypeOid::Varchar,
                typmod: Typmod::Length(7),
            },
            TypeOid::Varchar,
        ),
        ("now()", Expr::Builtin(BuiltinFn::Now), TypeOid::Timestamptz),
        (
            "CURRENT_DATE",
            Expr::Builtin(BuiltinFn::CurrentDate),
            TypeOid::Date,
        ),
        (
            "IN (list)",
            Expr::InList {
                expr: boxed(lit_int(1)),
                list: vec![lit_int(1)],
                negated: false,
            },
            TypeOid::Bool,
        ),
        (
            "IN (subquery)",
            Expr::InSubquery {
                expr: boxed(lit_int(1)),
                plan: Box::new(scan.clone()),
                negated: false,
            },
            TypeOid::Bool,
        ),
        (
            "EXISTS",
            Expr::Exists {
                plan: Box::new(scan),
                negated: false,
            },
            TypeOid::Bool,
        ),
        (
            "a window function",
            Expr::Window {
                func: WindowFunc::RowNumber,
                partition_by: Vec::new(),
                order_by: Vec::new(),
            },
            TypeOid::Int8,
        ),
        ("lower()", Expr::Lower(boxed(lit_null())), TypeOid::Text),
        (
            "jsonb_build_object()",
            Expr::JsonbBuildObject(vec![lit_str("k"), lit_int(1)]),
            TypeOid::Jsonb,
        ),
        (
            "= ANY (…)",
            Expr::AnyAll {
                left: boxed(lit_int(1)),
                op: BinaryOp::Eq,
                right: boxed(Expr::Array(vec![lit_int(1)])),
                all: false,
                sub_select: false,
            },
            TypeOid::Bool,
        ),
        (
            "nextval()",
            Expr::SequenceFn {
                func: SequenceFunc::Nextval,
                seq: "s".to_string(),
                args: Vec::new(),
            },
            TypeOid::Int8,
        ),
        (
            "EXTRACT",
            Expr::Extract {
                source: boxed(lit_null()),
                field: boxed(lit_str("year")),
                retnumeric: true,
            },
            TypeOid::Numeric,
        ),
        (
            "date_part",
            Expr::Extract {
                source: boxed(lit_null()),
                field: boxed(lit_str("year")),
                retnumeric: false,
            },
            TypeOid::Float8,
        ),
        (
            "upper()",
            Expr::StringFn {
                func: StringFunc::Upper,
                args: vec![lit_null()],
            },
            TypeOid::Text,
        ),
        (
            "length()",
            Expr::StringFn {
                func: StringFunc::Length,
                args: vec![lit_null()],
            },
            TypeOid::Int4,
        ),
        (
            "a comparison",
            Expr::BinaryOp {
                left: boxed(lit_null()),
                op: BinaryOp::Eq,
                right: boxed(lit_null()),
            },
            TypeOid::Bool,
        ),
        (
            "||",
            Expr::BinaryOp {
                left: boxed(lit_null()),
                op: BinaryOp::Concat,
                right: boxed(lit_null()),
            },
            TypeOid::Text,
        ),
        (
            "AT TIME ZONE",
            Expr::BinaryOp {
                left: boxed(lit_null()),
                op: BinaryOp::AtTimeZone,
                right: boxed(lit_str("utc")),
            },
            TypeOid::Timestamp,
        ),
        (
            "->",
            Expr::BinaryOp {
                left: boxed(lit_null()),
                op: BinaryOp::JsonGet,
                right: boxed(lit_str("k")),
            },
            TypeOid::Jsonb,
        ),
        (
            "count(NULL)",
            Expr::Aggregate {
                func: AggregateFunc::Count,
                arg: boxed(lit_null()),
                distinct: false,
                certainty: crate::planner::ArgCertainty::NotCertainInteger,
            },
            TypeOid::Int8,
        ),
        (
            "a bound non-null value",
            Expr::Bound(Value::Int4(1)),
            TypeOid::Int4,
        ),
    ];
    for (label, expr, want) in cases {
        assert_eq!(w1(&expr, ProjectionScope::empty()), want, "W1 over {label}");
        assert_eq!(
            w2(&expr, &schema, TypeScope::NONE),
            Some(want),
            "W2 over {label}"
        );
    }
}

/// **Cell A** (`Column`) and **cell D** (`Subquery`): the two leaves, and the only
/// cells whose difference is *which object* is consulted rather than what an
/// un-inferable operand is.
///
/// A resolvable column agrees — that is the whole point of the paired scopes, and it is
/// what the `select/alias_*_declared_types.sql` fixtures pin through SQL. An
/// **unresolvable** one does not, because W1 has nowhere to put "I could not tell".
///
/// D is a *layering* difference: W1 answers a scalar subquery from the planner's own
/// `Plan::output_columns` when its scope carries a catalog, and from a schema-blind
/// recursion (`infer_plan_output_type`) when it does not. The blind recursion has arms
/// for `Project` / `Aggregate` / `Filter` / `Sort` / `Limit` and nothing else, so a bare
/// `Plan::Scan` — and a no-FROM `SELECT 1`, which lowers to `Plan::Values` — both fall
/// through it to `Text`.
#[test]
fn a_column_and_a_subquery_leaf_resolve_through_each_walks_own_object() {
    let schema = schema_t();
    let columns = input_columns();
    let projection = ProjectionScope::new(&columns, &INPUT_NAMES, Some("t"));
    let catalog = TypeScope::Catalog(Some("t"));

    // A — the descriptor list and the catalog agree about `b`.
    assert_eq!(w1(&col("b"), projection), TypeOid::Int8);
    assert_eq!(w2(&col("b"), &schema, catalog), Some(TypeOid::Int8));
    // A — neither scope has `nosuch`, and only one can say so.
    assert_eq!(w1(&col("nosuch"), projection), TypeOid::Text);
    assert_eq!(w2(&col("nosuch"), &schema, catalog), None);

    // D — a bare `Scan`: the planner reads the table's first column, the executor's
    // blind recursion has no `Scan` arm and falls back.
    let scan = Expr::Subquery {
        plan: Box::new(Plan::Scan {
            table: "t".to_string(),
            alias: None,
        }),
    };
    assert_eq!(w1(&scan, ProjectionScope::empty()), TypeOid::Text);
    assert_eq!(
        w1(&scan, ProjectionScope::empty().with_schema(&schema)),
        TypeOid::Int8,
        "with a catalog attached W1 defers to the planner and the layering vanishes"
    );
    assert_eq!(w2(&scan, &schema, TypeScope::NONE), Some(TypeOid::Int8));

    // D — a no-FROM `SELECT 1` is a `Plan::Values`, another kind the blind recursion
    // does not walk.
    let values = Expr::Subquery {
        plan: Box::new(parse("SELECT 1", &schema).unwrap().remove(0)),
    };
    assert_eq!(w1(&values, ProjectionScope::empty()), TypeOid::Text);
    assert_eq!(w2(&values, &schema, TypeScope::NONE), Some(TypeOid::Int4));
}

/// The executor scope's own [`LeafScope`] implementation, against the two readers it is
/// narrowed from.
///
/// What the trait carries differs by method, and so does what this test holds.
/// `subquery_type` is read through the trait by **both** walks, so an implementation
/// answering differently from the walk's own arm would merge the rule and unmerge the
/// answer — that assertion guards a live path. **This impl's** `column_type` (not the
/// same-named `ColumnResolver::column_type`, which has three live callers) has no
/// production caller at all: the executor walk's `Column` arm reads
/// `ProjectionScope::resolve` and
/// `Expr::infer_type` is never handed a `ProjectionScope`, so a differing
/// implementation would unmerge nothing and would redden only this test. What is
/// pinned there is a **boundary identity** between two readers, not a merge. Both
/// methods are asserted against their reader rather than inferred from the delegation,
/// because the delegation is exactly what a later change rewrites:
///
/// - `column_type` is `ProjectionScope`'s descriptor lookup narrowed to the type, so it
///   agrees with the executor walk's `Column` arm **under that walk's `Text`
///   substitution**. That is cell A's boundary-identity, measured here and deliberately
///   *not* merged — for one reason and not two: what an unresolvable column should
///   *declare* is the fenced unknown-ness question. The whole-`ColumnDesc` need (a
///   passthrough projection carries typmod and provenance, not just the type) is
///   `expr_output_column`'s, which takes that descriptor from this scope **directly**
///   on a passthrough branch entering neither walk — so it is no reason to hold the
///   two `Column` arms apart.
/// - `subquery_type` is the path the walk's `Subquery` arm has always taken — cell D,
///   which *is* merged — asserted with a catalog and without one, since the two routes
///   are what the arm's fallback chains.
#[test]
fn the_executor_scopes_leaf_scope_impl_answers_as_its_own_readers_do() {
    let schema = schema_t();
    let columns = input_columns();
    let projection = ProjectionScope::new(&columns, &INPUT_NAMES, Some("t"));

    for (table, name, want) in [
        (None, "b", Some(TypeOid::Int8)),
        (Some("t"), "b", Some(TypeOid::Int8)),
        (None, "tx", Some(TypeOid::Text)),
        // A name this scope lacks and a qualifier it does not answer to both fail to
        // resolve, and the trait can say so where the total walk cannot.
        (None, "nosuch", None),
        (Some("other"), "b", None),
    ] {
        let label = format!("{table:?}.{name}");
        assert_eq!(
            projection.column_type(table, name),
            want,
            "column_type over {label}"
        );
        let reference = Expr::Column {
            table: table.map(str::to_string),
            name: name.to_string(),
        };
        assert_eq!(
            w1(&reference, projection),
            want.unwrap_or(TypeOid::Text),
            "the executor walk's own Column arm over {label}"
        );
    }

    let scan = Plan::Scan {
        table: "t".to_string(),
        alias: None,
    };
    assert_eq!(
        ProjectionScope::empty().subquery_type(&scan),
        None,
        "no catalog and no arm in the blind recursion for a bare Scan"
    );
    assert_eq!(
        ProjectionScope::empty()
            .with_schema(&schema)
            .subquery_type(&scan),
        Some(TypeOid::Int8),
        "with a catalog attached, the planner's own answer"
    );
}

/// The un-inferable-operand seam itself: **cells B, K, L, N** plus the two
/// pass-through variants (`Literal`, `Alias`) that carry it.
///
/// W1 is total, so "no type" and "text" are the same answer to every rule above it;
/// W2 says `None`. This is the single question behind most of the corpus, and it is
/// deliberately *not* decided here — see `math_fn_result_type`'s doc, which
/// holds the policy open and says why it must be settled per rule against the oracle.
#[test]
fn an_un_inferable_operand_is_text_on_the_executor_walk_and_none_on_the_planners() {
    for (label, expr) in [
        // The root of the family: a bare `NULL` literal.
        ("a NULL literal", lit_null()),
        // An alias is transparent, so it inherits its operand's answer on both walks.
        (
            "an alias over NULL",
            Expr::Alias {
                expr: boxed(lit_null()),
                alias: "a".to_string(),
            },
        ),
        // B — a bare `DEFAULT`.
        ("DEFAULT", Expr::Default),
        // L — an unbound `$N`.
        ("a parameter", Expr::Parameter { index: 1 }),
        // N — a bound **NULL** value. Not in the seam map's A–M table, which reads
        // `Bound` as agreeing (both take the value's type): `Value::Null` has no type
        // oid, so the pair splits exactly as every other un-inferable operand does.
        ("a bound NULL", Expr::Bound(Value::Null)),
        // K — arithmetic where *neither* operand types. W1's fallback picks the left
        // (`Text`), W2's `lt.or(rt)` has nothing to pick.
        (
            "NULL + NULL",
            Expr::BinaryOp {
                left: boxed(lit_null()),
                op: BinaryOp::Add,
                right: boxed(lit_null()),
            },
        ),
    ] {
        assert_no_input(label, &expr, TypeOid::Text, None);
    }
}

/// **Cell C, now merged** — the `sum`/`min`/`max` arm over an argument neither walk can
/// type, asserted under **both** scopes because it used to answer differently under each.
///
/// The shape of the assertion is kept from when it was scope-keyed: the same expression
/// under an empty and a populated [`ProjectionScope`], beside the planner walk under
/// `TypeScope::NONE` and a catalog. What changed is the answer — W1 is `Text` under both
/// now, W2 `None` under both, which is the same totality boundary the four merged cells
/// take. `count` is outside this shape (its result type is fixed) and is pinned with the
/// agreeing variants above.
///
/// **The arm this replaces was recorded dead, and it was live.** It returned
/// `TypeOid::Float8` when `arg == Text ∧ scope.columns.is_empty() ∧ Sum|Min|Max`; its own
/// comment justified that guard by "a subquery without schema", and measured, that route
/// never fires it — `SELECT (SELECT max('abc') FROM zc)` declares `25` on both legs with
/// the guess in place. The caller that *did* reach it was `execute_aggregate` over a FROM
/// whose relations are **all zero-column**, which ordinary SQL builds by two measured
/// routes: `CREATE TABLE zc (x INT); INSERT INTO zc VALUES (1), (2); ALTER TABLE zc DROP
/// COLUMN x;` — both engines accept that one — and a `RETURNING`-less data-modifying CTE
/// the outer query references, which 18.6 refuses outright (`0A000`), so "both engines
/// accept it" bounds the first spelling and not the class (`EDGES.md` rows 58 and 61).
/// After the `zc` route, `SELECT max('abc') FROM zc` declared `701` on the simple-query
/// `RowDescription` over a `text` value, against `25` on `Describe` and `25` on both of
/// 18.6's legs. The *all* is the guard's own width, `scope.columns.is_empty()` over the
/// projecting node's whole input: measured pre-deletion, `FROM zc a, zc b` declared `701`
/// and `FROM zc, t2` declared `25`. It also
/// **wrote a wrong answer**: the guessed descriptor is what an enclosing aggregate's
/// operand gate reads, so `SELECT sum(m) FROM (SELECT max('abc') AS m FROM zc) s`
/// answered one NULL row where 18.6 raises `42883`, `aggregate_legality(Sum, Float8)`
/// accepting what `aggregate_legality(Sum, Text)` refuses. The differential corpus could
/// not see either: no fixture built a zero-column relation. The guards are
/// `select/zero_column_relation_aggregate_declared_types.sql` and
/// `select/zero_column_relation_sum_over_text_loud.sql`, and the record is `EDGES.md`.
#[test]
fn the_two_walks_answer_an_untypable_aggregate_argument_alike_under_either_scope() {
    let schema = schema_t();
    let columns = input_columns();
    let projection = ProjectionScope::new(&columns, &INPUT_NAMES, Some("t"));
    for func in [AggregateFunc::Sum, AggregateFunc::Min, AggregateFunc::Max] {
        let agg = Expr::Aggregate {
            func,
            arg: boxed(lit_null()),
            distinct: false,
            certainty: crate::planner::ArgCertainty::NotCertainInteger,
        };
        assert_eq!(
            w1(&agg, ProjectionScope::empty()),
            TypeOid::Text,
            "{func:?}: the executor walk under a no-input scope — where the guess fired"
        );
        assert_eq!(
            w1(&agg, projection),
            TypeOid::Text,
            "{func:?}: the executor walk under a populated scope"
        );
        assert_eq!(w2(&agg, &schema, TypeScope::NONE), None, "{func:?}");
        assert_eq!(
            w2(&agg, &schema, TypeScope::Catalog(Some("t"))),
            None,
            "{func:?}: the planner walk has no scope-dependent arm here either"
        );
    }
}

/// **Cells E, F, H, I, J** — the arms that pick *an* operand's type from several.
///
/// **E, H, I and J agree now**, on the rule *and* on the answer: both walks read
/// `shared_multi_operand_type` and both take the first **inferable** operand, which is
/// 18.6's answer wherever exactly one branch type is known **to either walk** — and
/// *not* its rule, which folds the common type of every known branch, so two
/// differently-typed computed branches leave both walks wrong together (`EDGES.md` row
/// 54's agree-wrong band, which predates this and is untouched). The qualifier is
/// load-bearing in the *other* direction too, because 18.6's notion of *known* is not
/// this one's: a bare string literal is `unknown` there and `text` to both walks here,
/// so `COALESCE('7', length('ab'))` is `25` on both legs against 18.6's `23`,
/// `greatest('7', length('ab'))` is refused `42804` here where 18.6 answers `7`, and
/// `COALESCE('abc', length('ab'))` answers `abc` as `text` here where 18.6 is `22P02`
/// (row 54's literal-as-text band — pre-existing, and untouched by this hook, which
/// routes a literal through `Literal::infer_type` precisely so a string keeps typing as
/// `text`). W1 used to take the first operand outright,
/// its totality leaving it no way to skip an un-inferable one, and the four pairs below
/// that now agree were the divergence — pinned here as pairs (`ARRAY[NULL,1]` against
/// `ARRAY[1,NULL]`, and so on) because the **polarity control** is what catches an
/// over-correction that decides from the last operand instead of the first.
/// The controls are enumerated rather than asserted as a habit, since the sentence
/// claiming all five had one was written while H and J had none: `ARRAY[1,NULL]` (E),
/// `unnest(ARRAY[1,2])` (F), `greatest(1,NULL)` (H), `COALESCE(1,NULL)` (I), and
/// `CASE WHEN true THEN 1 ELSE NULL END` (J).
///
/// What closed them is `executor::helpers`'s `multi_operand_type` — this walk's operand
/// hook, declining for the four **typeless leaves** (a bare `NULL` literal, a bound
/// NULL, an unbound `$N`, a bare `DEFAULT`) so the shared rule sees a real `None`. The
/// convergence is **not** a merge's side effect: the merge alone left every answer
/// where it was, and this changed six of them. The residual is the `abs(-NULL)`
/// family's — an operand that is only *transitively* untyped keeps the total answer, so
/// `COALESCE(-NULL, length('ab'))` still declares 25 against `Describe`'s 23, where 18.6
/// refuses it `42725` rather than answering the other branch (`EDGES.md`).
///
/// **Which of the four leaves are live is a claim about the callers, so it is measured
/// rather than read off the arm list**, and they are not alike:
///
/// - **a bare `NULL` literal** — live, and the only one whose *declared type* a fixture
///   can assert on both legs (the `*_untyped_first_*` fixtures).
/// - **a bound `$N`** — live, and it is the `Parameter` arm rather than the `Bound` one:
///   the plan is never mutated to bind, so the leaf survives the bind and declines
///   whatever value arrives. Guarded on the extended leg by
///   `select/coalesce_bound_param_null_aggregate_operand.sql`.
/// - **a bare `DEFAULT`** — live, and the rule is **short-circuit, not position**: the
///   leaf survives wherever a cell operand goes unevaluated, so it reaches this hook from
///   any of them, leading or trailing. `SELECT CASE WHEN false THEN DEFAULT ELSE
///   length('ab') END` declared 25 against `Describe`'s 23 pre-fix and the `now()`
///   spelling reached the dropped-offset value. Measured now — every spelling in this
///   bullet `42601` on 18.6, and each pglet answer identical on the two legs — so are the
///   `WHEN false THEN DEFAULT WHEN true THEN length('ab')` spelling, a **trailing**
///   `COALESCE` operand (`COALESCE(length('ab'), DEFAULT)`, and the same with a third
///   operand behind it), a simple-form `CASE 1 WHEN 2 THEN DEFAULT ELSE length('ab') END`,
///   a `WHEN` **condition** past the taken arm
///   (`CASE WHEN true THEN 1 WHEN DEFAULT THEN 2 ELSE 3 END`), a whole nested cell
///   (`COALESCE(length('ab'), COALESCE(DEFAULT, 1))`), an aggregate argument inside the
///   untaken branch (`CASE WHEN false THEN count(DEFAULT) ELSE 1 END`) and an `ELSE`-less
///   `CASE WHEN false THEN DEFAULT END`. What each *declares* is the surviving branch's
///   rule and not this leaf's — `23` for the first five, `20` for the `count` one, and
///   `25` over a NULL row for the `ELSE`-less one, whose every branch is untyped.
///   **Which of them is loud is a property of the data**, not of the spelling: over
///   `dt(i)` holding `(7)` and `(NULL)`, `SELECT COALESCE(i, DEFAULT) FROM dt` answers
///   `7` filtered to the non-NULL row, `42601` filtered to the NULL one, `(0 rows)` over
///   `WHERE false` and `42601` over both — one statement, three verdicts. That axis is
///   row 56's rule (`EDGES.md`), whose *evaluation never reaches the leaf* mechanism owns
///   this leaf's reachability entirely; nothing here is a second criterion. A cell that
///   evaluates **every** operand buys the leaf no position of its own — `ARRAY[…,
///   DEFAULT]`, `greatest`/`least`/`nullif` either way round, `COALESCE(DEFAULT, …)`
///   reached first, and a projection-position bare `DEFAULT` are `42601` here as on 18.6
///   — but only *while the cell is evaluated at all*: measured, each of those over
///   `… FROM t WHERE false`, and `CASE WHEN false THEN greatest(length('ab'), DEFAULT)
///   ELSE 1 END`, is silent again. Starving the cell is row 56's rule one level out, and
///   it outranks the cell's own arms. Unfixturable —
///   the accepted spellings are row 56's over-acceptance, which 18.6 refuses `42601`.
/// - **a bound NULL** (`Expr::Bound` carrying `Value::Null`) — **no route found** into a
///   multi-operand cell, which is a statement about the production `Expr::Bound` mints,
///   and they are four rather than the three this bullet used to name.
///   **Each mint is disqualified by its own property, and nesting is not the shared
///   one** — two of the four nest. `parser::ddl`'s `column_default_to_expr` maps
///   `Value::Null` to `Expr::Literal(Literal::Null)` and only reaches `Expr::Bound` for
///   the values it has no `Literal` spelling for. The `INSERT` source-row substitution
///   replaces a whole column slot rather than nesting. `executor::dml`'s
///   `substitute_nextval` **does** nest — it rewrites through `map_children`, so
///   `id INT DEFAULT COALESCE(nextval('s'), 1)` mints an `Expr::Bound` *inside* the
///   `COALESCE`, and must (`executor::expr` refuses a nested `Expr::SequenceFn`, so a
///   value could come from nowhere else); measured over a sequence set to 100, two
///   inserts store `101` and `102`, and the `greatest(nextval('s'), 5)` spelling `201`
///   and `202` over one set to 200. What disqualifies it is its **type**: both arms of
///   `SequenceDraw` yield `Value::Int8`, never a `Value::Null`. And `parser::expr`'s
///   `parse_interval_literal` mints one for an `INTERVAL 'x'` literal, nests too, and
///   carries a `Value::Interval`. So the conclusion stands on a wider base than it did,
///   not a narrower one. `parse_interval_literal`'s mint has a consequence
///   one arm over: an `INTERVAL` literal is an `Expr::Bound`, so `concrete_branch_type`
///   answers `None` for it and `COALESCE(INTERVAL '1 day', length('ab'))` is a member of
///   the skipped-`42804` family — `1186` here and `42804` on 18.6 (`EDGES.md` row 65).
///   "No route found", not "no route" — the arm is classification, and deleting it would
///   be a silent widening.
///
/// The split's consequences are not all silent, and the loud one is what the executor
/// walk's answer becomes **downstream**: it is the projecting node's `ColumnDesc`, which
/// an enclosing aggregate's operand gate then reads (row 58's mechanism). Measured
/// pre-fix, `SELECT sum(c) FROM (SELECT COALESCE(NULL, length('ab')) AS c) s` was
/// `42883 function sum(text) does not exist` on both legs where 18.6 answers `2` — a
/// refused statement, not a mislabelled column.
///
/// Cell E keeps a **second** half with no control, because it has no polarity: the two
/// arms still disagree at the `array_of()` boundary, where an element type that has no
/// array type leaves the step answering `None` — W1 substitutes at its totality boundary
/// and W2 keeps the `None`. The only element type pglet has that reaches it is an
/// array — and a nested `ARRAY[ARRAY[…]]` is `0A000 multidimensional arrays are not
/// supported` on **both** legs (measured; 18.6 answers `int4[]`), so neither walker's
/// answer reaches a client. It is pinned as a fact about the arms, not about a
/// reachable statement — and it is the one answer the multi-operand merge moved, from
/// `text[]` to `text`, because the substitution used to sit inside the arm.
///
/// The E/H/I/J spellings pinned here are the ones the coercion pass **does** rewrite —
/// each has a concrete literal branch — so they say what the arms do and not what a
/// client sees. The spellings a client saw are the ones with a *computed* branch, which
/// carry no target and reach the walkers raw (module doc); their guards are
/// `select/{coalesce,case,variadic,array}_untyped_first_*`,
/// `select/coalesce_untyped_first_branch_timestamptz.sql` — the one that reached a
/// **value**, the column-type-aware text encoder dropping a `timestamptz`'s `+00` offset
/// under the wrong declaration — and
/// `select/coalesce_bound_param_null_aggregate_operand.sql`, the bound-`$N` leaf's, which
/// holds the loud consequence rather than the OID.
///
/// **F is live, and what reaches it has changed shape rather than gone away.** The
/// spelling this cell was written on — over `mw (s SMALLINT, i INT, b BIGINT, n NUMERIC,
/// f4 REAL, tx TEXT)` and an **empty** table, `SELECT unnest(<column>)` answering zero
/// rows under a `RowDescription` declaring the column's *own* type (21, 23, 20, 1700,
/// 700) against `Describe`'s 25 — is now refused at plan time, `42883 function
/// unnest(<type>) does not exist`, the same code and the same protocol phase (`Parse`,
/// measured over raw v3 on both engines) 18.6 answers. `planner::legality`'s
/// `unnest_legality` is the gate and `planner::coercion`'s `Expr::Unnest` arm its one
/// caller; the nine `select/unnest_*non_array*` fixtures hold it.
///
/// **What still reaches these two arms is an argument the gate's reader does not type**,
/// because that gate reads `arg_known_type` and that reader is narrow: an argument whose
/// type would be a *guess* defers rather than be refused on one, and the walkers then
/// type it anyway. Measured post-gate over an **empty** `mw`, each at `(0 rows)` against
/// 18.6's `42883` and each declaring 23 on the simple leg against `Describe`'s 25:
/// `SELECT unnest(i + 0) FROM mw`, `unnest(length(tx))`, `unnest(abs(i))`,
/// `unnest(COALESCE(i, 1))`, `unnest((SELECT i FROM mw))`,
/// `unnest(CASE WHEN true THEN i ELSE i END)`, `unnest(NULLIF(i, 1))`,
/// `unnest(GREATEST(i, 1))`, `unnest(LEAST(i, 1))` and `unnest(-i)`, with
/// `unnest(b + 0)` measuring 20 against the same 25. The list is re-derived from the
/// `Expr` variants `arg_known_type`'s `_` arm swallows, which is also why "computed" is
/// the wrong name for it: `set_config` lowers to an `Expr::Alias`, a shape
/// `certain_operand_type` types perfectly well, and `unnest(set_config('x', tx, false))`
/// is a member with the *same* `(0 rows)` against `42883` — but not a split, measured 25
/// on both legs, as `unnest(tx || 'a')` is. **The `(0 rows)` is the empty table's, not
/// the statement's**: with a
/// row every one of them, spelled bare as above, is `0A000` — these arms are read where
/// a value is produced, and the refusal fires where **evaluation reaches the leaf**
/// rather than where a row exists. Those are not the same bound, so the split is
/// observable with a full table wherever the leaf is not evaluated: over a one-row `mw`,
/// `SELECT CASE WHEN false THEN unnest(i + 0) ELSE 'x' END FROM mw` answers `x` against
/// 18.6's `42883`, at 23 on the simple leg against `Describe`'s 25 — and that leg sends
/// the text `x` under its own 23, while the extended leg's 25 and binary payload agree —
/// one spelling that is this residual, a row-36 declared-vs-produced
/// route and this cell's split at once (`EDGES.md` row 55's residual carries it, with the
/// taken-branch, `WHERE false` and cast controls). So the two arms are **kept, not
/// merged**: neither answer is right, there is
/// nothing to merge them onto, and the right close is the reader's — widening
/// `arg_known_type` past those arms, which is `EDGES.md` row 65's close and would take
/// this residual with it.
///
/// The `tx` column is still the spelling that shows what is being measured: W1's
/// fallback is `text` too, so before the gate the two legs *agreed* at 25 over a
/// statement 18.6 refuses — an agreeing pair is not a conforming one, and
/// `select/unnest_projection_non_array_text_column_loud.sql` is that member's fixture.
///
/// F's residual is un-fixturable for a different reason from cell K's below: there each
/// pair is red on exactly one *declared-type leg* (measured), while here 18.6 refuses
/// the statement outright and pglet answers rows, so the fixture is red before any
/// declared type is compared and `@expect-error` cannot hold it either.
/// `select/unnest_projection_int_declared_type.sql` is the array case this module names
/// as its differential control — the shape where both arms must keep agreeing. It is
/// not the only fixture carrying that polarity (its own header names the five others),
/// so a green corpus is not evidence that the control is doing work.
#[test]
fn the_multi_operand_arms_take_the_first_inferable_operand_on_both_walks() {
    let schema = schema_t();
    let columns = input_columns();
    let projection = ProjectionScope::new(&columns, &INPUT_NAMES, Some("t"));
    let catalog = TypeScope::Catalog(Some("t"));

    // E — `ARRAY[NULL, 1]`: int4[] on both walks now (was text[] on W1).
    let array_null_first = Expr::Array(vec![lit_null(), lit_int(1)]);
    assert_no_input(
        "ARRAY[NULL,1]",
        &array_null_first,
        TypeOid::Int4Array,
        Some(TypeOid::Int4Array),
    );
    assert_no_input(
        "ARRAY[1,NULL] (control)",
        &Expr::Array(vec![lit_int(1), lit_null()]),
        TypeOid::Int4Array,
        Some(TypeOid::Int4Array),
    );
    // E, second half — the `array_of()` boundary, and **the one answer the multi-operand
    // merge moved**. `int4[]` has no array type here, so the step answers `None`; W1's
    // substitution used to sit *inside* the arm (`.unwrap_or(TypeOid::TextArray)`) and
    // now sits at the totality boundary its caller applies, so `text[]` became `text`.
    // W2 is unmoved. Nothing reaches either answer: `ARRAY[ARRAY[1,2]]` is
    // `0A000 multidimensional arrays are not supported` on **both** pglet legs (measured
    // — simple query and `Describe` alike) before a declared type is read, where 18.6
    // answers `{{1,2}}` as `int4[]`. Recorded here because no fixture can hold it.
    assert_no_input(
        "ARRAY[ARRAY[1,2]]",
        &Expr::Array(vec![Expr::Array(vec![lit_int(1), lit_int(2)])]),
        TypeOid::Text,
        None,
    );

    // F — `unnest(<non-array>)`: the inner type itself against nothing. Both spellings
    // are now refused at plan time as SQL (`42883`), so these are facts about the
    // **arms**; the shapes that still reach them from SQL are the arguments the gate's
    // `arg_known_type` reader does not type — `unnest(i + 0)`, `unnest(length(tx))`,
    // `unnest(abs(i))`, `unnest(COALESCE(i, 1))`, `unnest((SELECT i FROM mw))` and the
    // `CASE`/`NULLIF`/`GREATEST`/`LEAST`/unary-`-` spellings, each measured 23 against
    // `Describe`'s 25 over 18.6's `42883` and each `(0 rows)` only while the table is
    // empty. `unnest(set_config('x', tx, false))` (an `Expr::Alias`) is a member of the
    // same reader class with the same `(0 rows)`, measured 25 on both legs rather than a
    // split. An `Expr::BinaryOp` argument is the closest of those to build here, and it
    // takes the same two arms this pair does.
    assert_no_input(
        "unnest(1)",
        &Expr::Unnest(boxed(lit_int(1))),
        TypeOid::Int4,
        None,
    );
    let unnest_col = Expr::Unnest(boxed(col("b")));
    assert_eq!(w1(&unnest_col, projection), TypeOid::Int8);
    assert_eq!(w2(&unnest_col, &schema, catalog), None);
    // The reachable-from-SQL member, in its `Expr` form: `unnest(b + 1)`.
    let unnest_computed = Expr::Unnest(boxed(Expr::BinaryOp {
        left: boxed(col("b")),
        op: BinaryOp::Add,
        right: boxed(lit_int(1)),
    }));
    assert_eq!(w1(&unnest_computed, projection), TypeOid::Int8);
    assert_eq!(w2(&unnest_computed, &schema, catalog), None);
    assert_no_input(
        "unnest(ARRAY[1,2]) (control)",
        &Expr::Unnest(boxed(Expr::Array(vec![lit_int(1), lit_int(2)]))),
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );

    // H — the variadic arm used to wrap every argument in `Some`, so an unknown one
    // reached the shared rule as `Some(Text)` and joined the common-type fold instead of
    // being skipped: `greatest(text, int4)` has no common type, and the rule fell back to
    // argument one. The hook declines for the typeless leaf now, so the fold sees one
    // known type.
    assert_no_input(
        "greatest(NULL,1)",
        &Expr::VariadicFn {
            func: VariadicFunc::Greatest,
            args: vec![lit_null(), lit_int(1)],
        },
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );
    assert_no_input(
        "greatest(1,NULL) (control)",
        &Expr::VariadicFn {
            func: VariadicFunc::Greatest,
            args: vec![lit_int(1), lit_null()],
        },
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );
    let least = Expr::VariadicFn {
        func: VariadicFunc::Least,
        args: vec![lit_null(), col("b")],
    };
    assert_eq!(w1(&least, projection), TypeOid::Int8);
    assert_eq!(w2(&least, &schema, catalog), Some(TypeOid::Int8));
    // `nullif` follows a different result rule (operator resolution, not common type)
    // and used to split the same way, which is what said the split was the operand
    // policy and not either result rule — and is why one hook closed both.
    let nullif = Expr::VariadicFn {
        func: VariadicFunc::NullIf,
        args: vec![lit_null(), col("b")],
    };
    assert_eq!(w1(&nullif, projection), TypeOid::Int8);
    assert_eq!(w2(&nullif, &schema, catalog), Some(TypeOid::Int8));

    // I — `COALESCE`.
    assert_no_input(
        "COALESCE(NULL,1)",
        &Expr::Coalesce(vec![lit_null(), lit_int(1)]),
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );
    assert_no_input(
        "COALESCE(1,NULL) (control)",
        &Expr::Coalesce(vec![lit_int(1), lit_null()]),
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );

    // J — `CASE`. W1 used to read the first WHEN's result and consult the ELSE only when
    // there was no WHEN at all; both walks now chain the WHEN results and the ELSE and
    // take the first that types.
    assert_no_input(
        "CASE WHEN true THEN NULL ELSE 1 END",
        &Expr::Case {
            operand: None,
            when_clauses: vec![(Expr::Literal(Literal::Bool(true)), lit_null())],
            else_result: Some(boxed(lit_int(1))),
        },
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );
    assert_no_input(
        "CASE WHEN true THEN 1 ELSE NULL END (control)",
        &Expr::Case {
            operand: None,
            when_clauses: vec![(Expr::Literal(Literal::Bool(true)), lit_int(1))],
            else_result: Some(boxed(lit_null())),
        },
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );
}

/// **Cell G** — the three `abs(…)` spellings recorded in `EDGES.md`, whose walker split had
/// nothing pinning it until now.
///
/// A **bare** `NULL` argument is routed through `Literal::infer_type` on both walks, so
/// it reaches `math_fn_result_type` as `None` and both answer `float8` — 18.6's answer,
/// and the control that says the 1a close still holds. An argument that is only
/// *transitively* untyped keeps W1's total `Text`, and `abs`'s rule passes an argument
/// type through, so the two legs declare 25 against 701 for one statement whose value is
/// NULL either way. 18.6 refuses all three (`42725`, then `42883` twice), so neither
/// answer is right and the right close is a refusal — its own rung.
#[test]
fn the_recorded_abs_residual_family_splits_the_two_walks() {
    let abs = |arg: Expr| Expr::MathFn {
        func: MathFunc::Abs,
        args: vec![arg],
    };
    assert_no_input(
        "abs(NULL) (control)",
        &abs(lit_null()),
        TypeOid::Float8,
        Some(TypeOid::Float8),
    );
    for (label, arg) in [
        ("abs(-NULL)", Expr::Negate(boxed(lit_null()))),
        (
            "abs(CASE WHEN true THEN NULL END)",
            Expr::Case {
                operand: None,
                when_clauses: vec![(Expr::Literal(Literal::Bool(true)), lit_null())],
                else_result: None,
            },
        ),
        (
            "abs(COALESCE(NULL, NULL))",
            Expr::Coalesce(vec![lit_null(), lit_null()]),
        ),
    ] {
        assert_no_input(label, &abs(arg), TypeOid::Text, Some(TypeOid::Float8));
    }
}

/// **Cell K** — the arithmetic unknown-operand fallback. The seam map recorded it as
/// "coincides on every constructed shape"; that is false, and it was false of shapes
/// both engines accept and answer.
///
/// Three spellings split, by three different mechanisms, and the mechanism is what says
/// which operand type reaches each. Two covering sentences stood here in place of that
/// and both were false: the doc's "the two fallbacks coincide whenever exactly one
/// operand is un-inferable" (members 2 and 3 are exactly that shape and split), and the
/// test's own name, `…_coincide_except_over_a_real_text_operand`, which asserted that
/// member 1 was the only one.
///
/// 1. **A genuinely `text` left operand** (`+ - * / %`). W1 cannot tell a real `text`
///    apart from its own un-inferable placeholder, so its `if left == Text { right }`
///    fallback hands the answer to the right operand; W2 keeps the left. Live: over
///    `t(b BIGINT, tx TEXT)` **empty**, `SELECT tx + 1 FROM t` answers zero rows
///    declaring `int4` on the simple-query `RowDescription` against `Describe`'s `text`,
///    and `SELECT tx + b FROM t` `int8` against `text`, where 18.6 refuses
///    `42883 operator does not exist: text + integer` / `text + bigint`. With a row
///    pglet raises its own `42804` (loud, wrong code), so this member is bounded to the
///    empty table. The **order** is the control: `1 + tx` and `b + tx` agree at `int4`
///    and `int8` on both walks, because W1's fallback only diverts when the *left* is
///    `Text`.
/// 2. **A `float4` operand beside an un-inferable one** (`+ - * / %`). Nothing to do
///    with `text`: W1 reaches `arith_result_type`, whose `numeric_family_result`
///    promotes `float4` mixed with anything else to `float8`, and its placeholder
///    `Text` is that "anything else"; W2's `(Some, None) ⇒ lt` keeps `float4`. Live and
///    **not** bounded to an empty table — over `t(f4 REAL)` with a row,
///    `SELECT f4 + NULL FROM t` declares `701` on the simple leg against `Describe`'s
///    `700`, where 18.6 says `700` on both. `float4` is the only type this reaches,
///    which the controls say — each `701`, `1700`, `20`, `700` on both walks here and on
///    both legs of 18.6: `f8 + NULL`, `n + NULL`, `b + NULL`, `f4 + f4`.
/// 3. **`^` against a `numeric` operand.** A third spelling again: W1 always folds
///    through `numeric_family_result` — and `numeric_family_result(Text, Numeric)` is
///    `numeric`, so W1 is **accidentally right** — while W2 short-circuits any
///    `(None, Some)` or `(Some, None)` pair to `float8`. Live and unbounded: over
///    `t(n NUMERIC)` with a row, `SELECT NULL ^ n FROM t` and `SELECT n ^ NULL FROM t`
///    both declare `1700` on the simple leg against `Describe`'s `701`, where 18.6 says
///    `1700` on both. `numeric` is the only type this reaches, because
///    `pow_result_type` answers `float8` to every other promotion: `NULL ^ 1` and
///    `f4 ^ NULL` are `701` on both walks and both engines.
///
/// Members 2 and 3 point **opposite ways** — the executor walk is wrong in one and the
/// planner walk in the other — which is the load-bearing fact for the ladder: no rung
/// may adopt one walker's cell-K rule wholesale, because either choice ships a
/// divergence that is correct today on the other leg.
///
/// None of the three can be pinned by a green fixture, and members 2 and 3 were measured
/// rather than argued: a candidate `SELECT f4 + NULL` fixture is red on the **text**
/// declared leg (`Postgres oid=700 vs pglet oid=701`) and a candidate `SELECT NULL ^ n`
/// one is red on the **extended** leg (`Postgres oid=1700 vs pglet oid=701`), each green
/// on the other leg; a `@no-declared-types:` opt-out waives both legs by design, so it
/// would assert nothing about the OIDs at all. Member 1 is out for a different reason,
/// the same one cell F is out for: 18.6 refuses the statement and pglet answers rows, so
/// the fixture is red on the values before a declared type is compared, and
/// `@expect-error` cannot hold it either. All three are recorded in `EDGES.md`. The right close
/// for each is 18.6's own answer — a `42883` for
/// member 1, the surviving operand's type for members 2 and 3 — which is this cell's own
/// rung.
#[test]
fn arithmetic_over_an_unknown_operand_splits_on_a_text_left_a_float4_and_a_numeric_pow() {
    let schema = schema_t();
    let columns = input_columns();
    let projection = ProjectionScope::new(&columns, &INPUT_NAMES, Some("t"));
    let catalog = TypeScope::Catalog(Some("t"));
    let arith = |left: Expr, op: BinaryOp, right: Expr| Expr::BinaryOp {
        left: boxed(left),
        op,
        right: boxed(right),
    };

    // The agreeing shapes: with no real operand type in play, the two fallbacks land on
    // the same answer.
    assert_no_input(
        "NULL + 1",
        &arith(lit_null(), BinaryOp::Add, lit_int(1)),
        TypeOid::Int4,
        Some(TypeOid::Int4),
    );
    assert_no_input(
        "NULL ^ 1",
        &arith(lit_null(), BinaryOp::Pow, lit_int(1)),
        TypeOid::Float8,
        Some(TypeOid::Float8),
    );

    // 1 — a real `text` left operand, and the reverse order as its control.
    let text_plus_int = arith(col("tx"), BinaryOp::Add, lit_int(1));
    assert_eq!(w1(&text_plus_int, projection), TypeOid::Int4);
    assert_eq!(w2(&text_plus_int, &schema, catalog), Some(TypeOid::Text));
    let text_plus_bigint = arith(col("tx"), BinaryOp::Add, col("b"));
    assert_eq!(w1(&text_plus_bigint, projection), TypeOid::Int8);
    assert_eq!(w2(&text_plus_bigint, &schema, catalog), Some(TypeOid::Text));
    let int_plus_text = arith(lit_int(1), BinaryOp::Add, col("tx"));
    assert_eq!(w1(&int_plus_text, projection), TypeOid::Int4);
    assert_eq!(w2(&int_plus_text, &schema, catalog), Some(TypeOid::Int4));
    let bigint_plus_text = arith(col("b"), BinaryOp::Add, col("tx"));
    assert_eq!(w1(&bigint_plus_text, projection), TypeOid::Int8);
    assert_eq!(w2(&bigint_plus_text, &schema, catalog), Some(TypeOid::Int8));

    // 2 — `float4` beside an un-inferable operand, in both orders, with the rest of the
    // numeric family as the controls that say `float4` is the only type reaching it.
    for (label, expr) in [
        ("f4 + NULL", arith(col("f4"), BinaryOp::Add, lit_null())),
        ("NULL + f4", arith(lit_null(), BinaryOp::Add, col("f4"))),
        ("f4 * NULL", arith(col("f4"), BinaryOp::Mul, lit_null())),
    ] {
        assert_eq!(w1(&expr, projection), TypeOid::Float8, "W1 over {label}");
        assert_eq!(
            w2(&expr, &schema, catalog),
            Some(TypeOid::Float4),
            "W2 over {label}"
        );
    }
    for (label, expr, want) in [
        (
            "n + NULL (control)",
            arith(col("n"), BinaryOp::Add, lit_null()),
            TypeOid::Numeric,
        ),
        (
            "b + NULL (control)",
            arith(col("b"), BinaryOp::Add, lit_null()),
            TypeOid::Int8,
        ),
        (
            "f4 + f4 (control)",
            arith(col("f4"), BinaryOp::Add, col("f4")),
            TypeOid::Float4,
        ),
        (
            "f8 + NULL (control)",
            arith(col("f8"), BinaryOp::Add, lit_null()),
            TypeOid::Float8,
        ),
    ] {
        assert_eq!(w1(&expr, projection), want, "W1 over {label}");
        assert_eq!(w2(&expr, &schema, catalog), Some(want), "W2 over {label}");
    }

    // 3 — `^` against `numeric`, in both orders, with `f4 ^ NULL` as the control that
    // says `numeric` is the only promotion `pow_result_type` does not flatten.
    for (label, expr) in [
        ("NULL ^ n", arith(lit_null(), BinaryOp::Pow, col("n"))),
        ("n ^ NULL", arith(col("n"), BinaryOp::Pow, lit_null())),
    ] {
        assert_eq!(w1(&expr, projection), TypeOid::Numeric, "W1 over {label}");
        assert_eq!(
            w2(&expr, &schema, catalog),
            Some(TypeOid::Float8),
            "W2 over {label}"
        );
    }
    let f4_pow_null = arith(col("f4"), BinaryOp::Pow, lit_null());
    assert_eq!(w1(&f4_pow_null, projection), TypeOid::Float8);
    assert_eq!(w2(&f4_pow_null, &schema, catalog), Some(TypeOid::Float8));
}

/// **Cell M** — `date_trunc`'s source argument, one of the four cells merged into
/// `shared_leaf_cell_type`.
///
/// The two walks no longer hold an arm each, and they still read the source
/// differently: each passes its **own** walk as the shared rule's recursion, so W1 hands
/// back `Some(Text)` for an untyped source where W2 hands back `None`, and
/// `date_trunc_result_type` maps both to `timestamp`. The merge is safe *because of that
/// table*, not by construction — which is what this pins. A future rule distinguishing
/// `text` from unknown must redden this rather than silently drop a `+00` offset.
///
/// The first case's SQL is one 18.6 **refuses**: `SELECT date_trunc('day', NULL)` is
/// `42725 function date_trunc(unknown, unknown) is not unique` there and answers a NULL
/// row declaring `timestamp` on both of pglet's legs here. So this pins today's answer
/// and not its rightness; the over-acceptance is recorded on `docs/compatibility.md`'s `date_trunc`
/// row, whose control is the same call over a **typed** source — the second case's
/// shape, which both engines resolve to `timestamptz`.
#[test]
fn date_trunc_answers_identically_though_the_two_walks_read_the_source_differently() {
    let schema = schema_t();
    let columns = input_columns();
    let projection = ProjectionScope::new(&columns, &INPUT_NAMES, Some("t"));

    assert_no_input(
        "date_trunc('day', NULL)",
        &Expr::StringFn {
            func: StringFunc::DateTrunc,
            args: vec![lit_str("day"), lit_null()],
        },
        TypeOid::Timestamp,
        Some(TypeOid::Timestamp),
    );
    let over_tz = Expr::StringFn {
        func: StringFunc::DateTrunc,
        args: vec![lit_str("day"), col("ts")],
    };
    assert_eq!(w1(&over_tz, projection), TypeOid::Timestamptz);
    assert_eq!(
        w2(&over_tz, &schema, TypeScope::Catalog(Some("t"))),
        Some(TypeOid::Timestamptz)
    );
}
