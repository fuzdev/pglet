//! Expression IR types and the shared function-call dispatch (`build_call`).

use super::common_type::{nullif_result_type, variadic_common_type_oid};
use super::plan::{LeafScope, PlannerScope, TypeScope};
use crate::catalog::{DEFAULT_SEARCH_PATH, Schema};
use crate::types::{ColumnDesc, TypeOid, Value};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Expression in a query plan.
//
// Walkers that recurse over an expression's direct scalar sub-expressions share one
// child-set source, so adding a variant updates that one helper rather than each
// walker:
//   - read-only folds (`collect_referenced_columns`, `validate_grouped_expr`,
//     `contains_aggregate`/`contains_window`/`contains_unnest`) go through
//     `Expr::for_each_child` / `try_for_each_child`;
//   - the owned type-resolution transform (`resolve_expr_types`) goes through
//     `Expr::map_children`.
// The remaining exhaustive walkers (`infer_type`/`expr_type_oid`, `output_name`,
// `expr_column_info`, `eval_expr_full`, `param_types::walk_expr`) are not uniform
// child-recursion — they map each variant to a label/type or carry extra scope/state —
// so they stay exhaustive and still need a new arm per variant.
//
// `remote = "Self"`: the derive writes the encoding as inherent functions, and the
// trait impls below wrap each level of it in the depth bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self")]
pub enum Expr {
    Literal(Literal),
    Column {
        /// Optional table qualifier (e.g., "users" in "users.id").
        table: Option<String>,
        name: String,
    },
    BinaryOp {
        left: Box<Self>,
        op: BinaryOp,
        right: Box<Self>,
    },
    /// Numeric negation (-x).
    Negate(Box<Self>),
    /// Boolean negation (NOT x).
    Not(Box<Self>),
    /// IS NULL check.
    IsNull(Box<Self>),
    /// IS NOT NULL check.
    IsNotNull(Box<Self>),
    /// DEFAULT keyword - use column's default value (or NULL).
    Default,
    /// Type cast (explicit CAST or implicit coercion).
    Cast {
        expr: Box<Self>,
        target_type: TypeOid,
    },
    /// Aliased expression (AS alias).
    Alias {
        expr: Box<Self>,
        alias: String,
    },
    /// Aggregate function call (COUNT, SUM, AVG, MIN, MAX).
    Aggregate {
        func: AggregateFunc,
        arg: Box<Self>,
        distinct: bool,
        /// The coercion pass's verdict on `arg`'s type, which every reader of this
        /// aggregate's result type reads ([`aggregate_type_of`]) rather than deriving
        /// one of its own.
        certainty: ArgCertainty,
    },
    /// Scalar subquery - returns a single value from a nested query.
    Subquery {
        plan: Box<super::Plan>,
    },
    /// IN subquery - `expr [NOT] IN (SELECT ...)`.
    InSubquery {
        expr: Box<Self>,
        plan: Box<super::Plan>,
        negated: bool,
    },
    /// `[NOT] EXISTS(SELECT ...)` — an existence test. Evaluates to a boolean:
    /// `true` iff the sub-plan returns at least one row (column count is irrelevant
    /// — only row presence matters), XOR'd with `negated` for the `NOT EXISTS`
    /// form. **Correlated** `EXISTS` (the inner query references an outer column) is
    /// supported: descending into the subquery pushes the outer scope onto the
    /// evaluation environment, and the correlated column resolves against it at
    /// eval time (see `resolve_column` / `EvalEnv` in `executor::expr`).
    Exists {
        plan: Box<super::Plan>,
        negated: bool,
    },
    /// IN list - `expr [NOT] IN (val1, val2, ...)`.
    InList {
        expr: Box<Self>,
        list: Vec<Self>,
        negated: bool,
    },
    /// Window function call (`ROW_NUMBER`, RANK, etc. OVER ...).
    Window {
        func: WindowFunc,
        /// PARTITION BY expressions.
        partition_by: Vec<Self>,
        /// ORDER BY sort keys within the window (each an [`OrderKey`] carrying its
        /// direction and NULL placement).
        order_by: Vec<OrderKey>,
    },
    /// Parameter placeholder ($1, $2, etc.) - 1-indexed.
    Parameter {
        index: usize,
    },
    /// A materialized value spliced into the plan. Parameters are **not** bound
    /// this way (an `Expr::Parameter` resolves from the `EvalEnv` at eval time);
    /// this variant carries an already-produced `Value` through a shared code path:
    /// `INSERT … SELECT` lowers each source row's values into `Bound` so they flow
    /// through the same insert loop as a `VALUES` literal, and `executor::dml`'s
    /// `substitute_nextval` splices drawn sequence values the same way (both
    /// transient — neither shape is persisted).
    ///
    /// **It is not only plan-internal, and reading it as such has cost a database's
    /// dump.** `parser::expr`'s `parse_interval_literal` produces one directly from
    /// SQL text (`INTERVAL '1 day'`), which is the *only* live such producer —
    /// `parser::ddl`'s `column_default_to_expr` constructs one for a
    /// `ColumnDefault::Literal` whose `Value` is outside the five its own classifier
    /// produces, dead by that range and confined to the already-probed `DEFAULT`
    /// site. `deparse::render_expr` declines this variant, having no source spelling to
    /// emit for an arbitrary `Value`. So an `Expr::Bound` reaching the catalog makes
    /// the **whole database** undumpable, and what keeps it out is a DDL-time render
    /// probe at each of the four sites an expression is persisted from
    /// (`executor::ddl`'s `cook_column_default` and `validate_stored_expr_renders`),
    /// not any property of this variant.
    Bound(Value),
    /// Niladic builtin scalar function (`now()`, `gen_random_uuid()`).
    Builtin(BuiltinFn),
    /// Array constructor (`ARRAY[a, b, c]`). Element type is inferred from the
    /// elements (or set by an enclosing cast); an empty array defaults to `text[]`.
    Array(Vec<Self>),
    /// `jsonb_build_object(k1, v1, k2, v2, …)` — a JSONB object from alternating
    /// key/value arguments. The `Vec` holds `[k1, v1, k2, v2, …]` (even length,
    /// enforced at conversion); each key must evaluate to text, and each value is
    /// coerced to a `JsonbValue` per Postgres `to_jsonb` semantics (a text value
    /// becomes a JSON string). Empty (`{}`) is valid.
    JsonbBuildObject(Vec<Self>),
    /// `expr op ANY(array)` / `expr op ALL(array)` — element-wise comparison
    /// against an array. `all = false` is ANY/SOME, `all = true` is ALL.
    AnyAll {
        left: Box<Self>,
        op: BinaryOp,
        right: Box<Self>,
        all: bool,
        /// The node was written `op ANY ((SELECT …))`, which 18.6 reads as its
        /// sub-select form — compare with each row — and pglet does not run. It is
        /// built with the scalar subquery as its array operand, and its evaluation
        /// refuses `0A000` once both operands are evaluated, so an error raised before
        /// then outranks the refusal (`EDGES.md` row 180). Not encoded: a stored
        /// expression refuses a subquery, so no catalog holds one.
        #[serde(skip)]
        sub_select: bool,
    },
    /// `unnest(<array-expr>)` as a **set-returning function in a SELECT
    /// projection** (e.g. `SELECT $1, unnest($2::text[])`). The inner expression
    /// evaluates to an array; the projection row-expands, one output row per
    /// element, with scalar siblings broadcast across them.
    ///
    /// Scoped to the no-FROM projection case the spine needs (`query_put_fact_refs`):
    /// a single `Unnest` per SELECT, no real FROM clause, no `WITH ORDINALITY` /
    /// LATERAL / LCM-zip of multiple unnests. The FROM-clause table-source form of
    /// `unnest` is a separate path (`Plan::FunctionScan`), unaffected by this variant.
    /// Lowered at plan time to `Plan::ProjectUnnest`; `eval_expr` never sees it as a
    /// scalar (a stray `Unnest` outside a projection fails loud).
    Unnest(Box<Self>),
    /// `LOWER(text)` — lowercase a text value (NULL → NULL). Modeled as its own
    /// scalar so it works both as an expression-index key part (`LOWER(email)`) and
    /// in ordinary queries. A non-text, non-null operand fails loud.
    Lower(Box<Self>),
    /// `COALESCE(a, b, …)` — the first non-NULL argument, or NULL if all are NULL.
    /// At least one argument (enforced at conversion). Used by the spine's scoped
    /// `role_grant` expression indexes (`COALESCE(scope_kind, 'GLOBAL')`).
    Coalesce(Vec<Self>),
    /// `CASE … WHEN … THEN … [ELSE …] END` — a conditional expression.
    ///
    /// `operand` is `None` for the searched form (each `when` is a boolean
    /// predicate) and `Some` for the simple form (each `when` is a value compared
    /// for equality against the operand). The first matching branch's result is
    /// returned; if none match, `else_result` (or NULL when absent).
    Case {
        operand: Option<Box<Self>>,
        /// `(condition_or_value, result)` pairs, in order.
        when_clauses: Vec<(Self, Self)>,
        else_result: Option<Box<Self>>,
    },
    /// A numeric math function — `abs` / `ceil` / `ceiling` / `floor` / `round` /
    /// `trunc`. `round`/`trunc` take an optional second *scale* argument (numeric
    /// only); the rest are unary. Exact over `NUMERIC` (Postgres semantics); a
    /// float argument computes in `float8` and an integer promotes to `NUMERIC`
    /// (matching Postgres's function resolution). A non-numeric operand fails loud.
    ///
    /// Appended last so existing postcard-encoded `Expr` IR (index-key / CHECK
    /// predicates) keeps its variant discriminants.
    MathFn {
        func: MathFunc,
        args: Vec<Self>,
    },
    /// A **strict-NULL** scalar string function — `upper` / `length` / `replace` /
    /// `substring` / `ltrim` / `rtrim` / `btrim` / `strpos` (`position`) / `lpad` /
    /// `rpad` / `split_part`. Any NULL argument makes the whole call NULL (the
    /// hoisted null check in `eval_string_fn`), exactly like [`Self::MathFn`]. All
    /// indexing is 1-based, and lengths/positions over `text` are **character**-based
    /// ([`StringFunc`] names the byte-counted members). An argument of a type the
    /// member has no overload for fails loud rather than silently coercing.
    ///
    /// Appended after `MathFn` so existing postcard-encoded `Expr` IR (index-key /
    /// CHECK predicates) keeps its variant discriminants.
    StringFn {
        func: StringFunc,
        args: Vec<Self>,
    },
    /// A **non-strict** variadic function whose NULL handling is per-function, so it
    /// cannot share [`Self::StringFn`]'s hoisted null check — `concat` / `concat_ws`
    /// (skip NULL args, all-NULL → `''`), `greatest` / `least` (skip NULLs, all-NULL
    /// → NULL, common-type comparison), and `nullif` (NULL if the two args are equal;
    /// a NULL argument returns the first). See `eval_variadic_fn`.
    ///
    /// Appended after `StringFn` for the same postcard-discriminant-stability reason.
    VariadicFn {
        func: VariadicFunc,
        args: Vec<Self>,
    },
    /// `EXTRACT(field FROM source)` / `extract(text, source)` / `date_part(text,
    /// source)` — pull a sub-field out of a temporal value. `field` evaluates to the
    /// field-name text (a literal from the `EXTRACT` grammar, or the text argument of
    /// the function forms), resolved to an [`ExtractField`] at eval time; `source` is
    /// the timestamp / date / time / interval. `retnumeric` selects the return type:
    /// `true` for `EXTRACT`/`extract` (**numeric**, with Postgres's exact per-field
    /// display scale — `second`/`epoch` → 6, `milliseconds` → 3, integer fields → 0),
    /// `false` for `date_part` (**float8**). An unrecognized field fails loud
    /// (`22023`); a recognized field wrong for the source type fails loud (`0A000`).
    ///
    /// Appended last so existing postcard-encoded `Expr` IR (index-key / CHECK
    /// predicates) keeps its variant discriminants.
    Extract {
        source: Box<Self>,
        field: Box<Self>,
        retnumeric: bool,
    },
    /// A sequence function — `nextval` / `currval` / `setval`. Its own variant (not
    /// a generic call) because evaluating one **mutates** sequence state (nextval /
    /// setval) or reads **per-session** state (currval), neither of which the
    /// read-only expression-eval path can do. It is therefore handled at the
    /// statement level (`execute_sequence_projection`), with `&mut Schema` +
    /// session access — only as a **top-level** no-FROM `SELECT` projection (and,
    /// for nextval/setval, an `INSERT … VALUES(…)` entry). A `SequenceFn` reaching
    /// the read-only eval path (a nested position — WHERE, a subquery, an arithmetic
    /// operand) fails loud.
    ///
    /// Appended last so existing postcard-encoded `Expr` IR keeps its variant
    /// discriminants (a projection `SequenceFn` is not persisted, but the ordering
    /// invariant is kept uniformly).
    SequenceFn {
        func: SequenceFunc,
        /// The bare sequence name — a schema-qualified `'public.s'` reduced to `s`
        /// at build time, keying the same `Schema.sequences` slot `CREATE SEQUENCE`
        /// and the `DEFAULT nextval(...)` path register under.
        seq: String,
        /// `setval`'s `n` (and optional `is_called`) arguments; empty for
        /// nextval/currval.
        args: Vec<Self>,
    },
    /// A cast whose target type carried a **type modifier** — `x::numeric(p,s)`,
    /// `x::varchar(n)`, `x::char(n)`. Distinct from [`Self::Cast`] (which drops any
    /// typmod) so the modifier is enforced at eval time: coerce the inner value to
    /// `type_oid`, then apply `typmod` via `executor::apply_typmod` with
    /// `is_explicit = true` (a `varchar`/`char` length over-run truncates silently,
    /// per Postgres explicit-cast semantics; a `numeric(p,s)` rounds to scale and
    /// range-checks identically to the column path).
    ///
    /// Appended **last** so existing postcard-encoded `Expr` IR (index-key / CHECK
    /// predicates that carry a plain `Expr::Cast`) keeps its variant discriminants.
    CastTypmod {
        expr: Box<Self>,
        type_oid: TypeOid,
        typmod: Typmod,
    },
    /// Unary `+` — a value-preserving wrapper that exists purely to keep `+x` from
    /// being *structurally* its operand.
    ///
    /// Postgres's grammar folds unary `-` over a numeric constant into the constant
    /// itself (`doNegate`, `gram.y`) but leaves `'+' a_expr` an ordinary operator
    /// node. That asymmetry is load-bearing for the SQL92 clause rules: because `+1`
    /// is not an `A_Const`, `findTargetlistEntrySQL92` never reads it as an ordinal,
    /// so `GROUP BY +1` is an ordinary (constant) grouping key — `42803` over a
    /// projected column — and `ORDER BY +1` sorts by a constant, i.e. preserves the
    /// input order, rather than sorting by output column 1.
    ///
    /// pglet's parser previously discarded unary `+` outright, making `+1` a bare
    /// [`Literal`] and thus an ordinal — silently the *inverse* of Postgres in both
    /// clauses. Keeping the node restores the distinction at the one place it is
    /// observable.
    ///
    /// Evaluation returns the operand unchanged and [`Self::infer_type`] is the
    /// operand's type, so the wrapper is semantically transparent; only
    /// [`Self::output_name`] differs (`?column?`, as Postgres names any operator
    /// expression). Two consequences worth knowing: a `+`-wrapped literal is not
    /// [row-independent](super::index_selection), so `WHERE indexed = +1` declines
    /// the index and runs a row filter (slower, and identical rows wherever the two
    /// spellings evaluate the same leaves — but a *non-matching* probe starves a
    /// `DEFAULT`/`$N` beside it, so over `ti(a INT, v TEXT)` indexed on `a` and holding
    /// `(1, 'x')`, `WHERE a = +2 AND DEFAULT IS NULL` is `42601` where
    /// `WHERE a = 2 AND DEFAULT IS NULL` answers `(0 rows)`; `EDGES.md` row 56); and
    /// pglet does **not** gate the operand's type, where Postgres
    /// rejects a non-numeric operand — in *two* different ways, both of which pglet
    /// answers with a bare `abc`:
    ///
    /// - `+'abc'::text` — the operand is *typed* `text`, so operator resolution finds
    ///   no `+ text` and raises `42883 operator does not exist: + text`.
    /// - `+'abc'` — the operand is an **unknown**-typed literal, which unary `+`
    ///   resolves to `float8`; the failure is then the *input conversion*, `22P02
    ///   invalid input syntax for type double precision: "abc"`, not `42883`.
    ///
    /// Neither is a regression — pglet's parser previously discarded the `+`
    /// entirely, so both spellings already returned `abc`. Keeping the node changes
    /// only the clause-ordinal behavior above, not the (absent) operand gate.
    ///
    /// Appended **last** so existing postcard-encoded `Expr` IR keeps its variant
    /// discriminants.
    UnaryPlus(Box<Self>),
    /// `x IS [NOT] TRUE / FALSE / UNKNOWN` — Postgres's `BooleanTest`.
    ///
    /// A **three-valued** test that is itself never NULL: the result is `boolean`
    /// for every input including NULL (`NULL IS TRUE` is `f`, `NULL IS UNKNOWN` is
    /// `t`, measured on 18.6). Kept as its own node rather than lowered to
    /// `IS [NOT] DISTINCT FROM true` / `IS [NULL|NOT NULL]` for two reasons, both
    /// observable: 18.6's `pg_get_constraintdef` renders a stored
    /// `CHECK (b IS TRUE)` as `CHECK ((b IS TRUE))`, so a lowered node would deparse
    /// to a different (if equivalent) constraint definition; and the operand gate's
    /// message names the spelling — `argument of IS NOT TRUE must be type boolean,
    /// not type integer` — which a lowered node no longer knows.
    ///
    /// The operand must be `boolean`: a non-boolean one is `42804`
    /// (`planner::legality`'s `check_boolean_operand` under `BoolContext::Test`), and an
    /// `unknown`-typed
    /// literal is coerced (`'t' IS TRUE` is `t`, `'x' IS TRUE` is `22P02`).
    ///
    /// Appended **last** so existing postcard-encoded `Expr` IR (index-key / CHECK
    /// predicates) keeps its variant discriminants.
    BooleanTest {
        expr: Box<Self>,
        test: BoolTest,
    },
    /// `pg_column_size(arg)` — the bytes 18.6 holds `arg`'s value in, which depends on
    /// where the value came from as well as on what it is: a value read from a heap row
    /// carries the one-byte varlena header `heap_form_tuple` packed it to (up to 126 data
    /// bytes), one built in memory a four-byte header, and a stored value 18.6's TOAST pass
    /// reached may be compressed or out of line. So the call carries what the coercion
    /// pass learned about its argument ([`ColumnSizeOrigin`]), and answers only where that
    /// settles the bytes — `0A000` otherwise, never a guess (`docs/compatibility.md`
    /// § `pg_column_size`).
    ///
    /// Appended **last** so existing postcard-encoded `Expr` IR keeps its variant
    /// discriminants.
    ColumnSize {
        arg: Box<Self>,
        origin: ColumnSizeOrigin,
    },
}

/// Where `pg_column_size`'s argument comes from ([`Expr::ColumnSize`]).
///
/// A constant is read off its spelling where the call is built ([`Self::written`]),
/// anything else by the coercion pass, which sees the argument's scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnSizeOrigin {
    /// Not read: what the call is built with for an argument that is not a constant, and
    /// what a clause the coercion pass does not reach keeps — a `CHECK`, whose operand
    /// 18.6 sizes before the row is formed. It answers only a NULL.
    Unclassified,
    /// A constant — a literal, a cast written over one, an `ARRAY[…]` of them — which 18.6
    /// folds to a datum built in memory. `unknown` is a bare string literal, which 18.6
    /// sizes as the C string `unknown` is (its bytes and a terminator).
    Constant { unknown: bool },
    /// A column of the base table `table` at the call's own query level, whose type is
    /// an enum when `is_enum` — a value read from the table's heap row.
    Stored { table: String, is_enum: bool },
    /// Anything else: an expression 18.6 computes, a derived table's or an enclosing
    /// level's column. It answers only a NULL.
    Unsupported,
}

impl ColumnSizeOrigin {
    /// The origin `arg`'s spelling alone settles: a constant, which 18.6 folds to a datum
    /// built in memory — a literal, a cast written over one (but not a length cast, which
    /// may be a `char(n)` whose padding pglet does not hold, `EDGES.md` row 95, nor a
    /// string's cast to `text`, which may be an enum member literal folded to it), a sign
    /// over a number literal, an `ARRAY[…]` of those — with a bare string literal (or
    /// `NULL`) 18.6's `unknown`. [`Self::Unclassified`] for anything else, which only
    /// the argument's scope can place.
    #[must_use]
    pub fn written(arg: &Expr) -> Self {
        fn constant(e: &Expr) -> bool {
            match e {
                Expr::Literal(_) => true,
                // an enum member literal is folded to exactly this cast, its type's name
                // dropped (`build_enum_cast`), so a string cast to `text` may be an enum
                // 18.6 stores as a four-byte OID
                Expr::Cast {
                    expr,
                    target_type: TypeOid::Text,
                } if matches!(**expr, Expr::Literal(Literal::String(_))) => false,
                Expr::Cast { expr, .. } => matches!(**expr, Expr::Literal(_)),
                Expr::CastTypmod { expr, typmod, .. } => {
                    !matches!(typmod, Typmod::Length(_)) && matches!(**expr, Expr::Literal(_))
                }
                Expr::Negate(inner) | Expr::UnaryPlus(inner) => matches!(
                    **inner,
                    Expr::Literal(Literal::Int(_) | Literal::Float(_) | Literal::Numeric(_))
                ),
                _ => false,
            }
        }
        match arg {
            Expr::Literal(Literal::String(_) | Literal::Null) => Self::Constant { unknown: true },
            Expr::Array(items) if items.iter().all(constant) => Self::Constant { unknown: false },
            e if constant(e) => Self::Constant { unknown: false },
            _ => Self::Unclassified,
        }
    }
}

/// Which truth value an [`Expr::BooleanTest`] tests for, and whether the test is
/// negated — the six spellings of `x IS [NOT] TRUE|FALSE|UNKNOWN`.
///
/// Persisted (a `CHECK` predicate may carry one), so the variant order is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoolTest {
    True,
    NotTrue,
    False,
    NotFalse,
    Unknown,
    NotUnknown,
}

impl BoolTest {
    /// The SQL spelling of the test, as `pg_get_expr` renders it (`IS NOT TRUE`, …)
    /// and as the `42804` operand-gate message names it.
    #[must_use]
    pub const fn sql(self) -> &'static str {
        match self {
            Self::True => "IS TRUE",
            Self::NotTrue => "IS NOT TRUE",
            Self::False => "IS FALSE",
            Self::NotFalse => "IS NOT FALSE",
            Self::Unknown => "IS UNKNOWN",
            Self::NotUnknown => "IS NOT UNKNOWN",
        }
    }

    /// Apply the test to a three-valued operand (`None` = NULL), yielding the
    /// boolean the test always produces.
    #[must_use]
    pub const fn apply(self, operand: Option<bool>) -> bool {
        match (self, operand) {
            (Self::True | Self::NotFalse, Some(b)) => b,
            (Self::True | Self::NotFalse, None) => matches!(self, Self::NotFalse),
            (Self::NotTrue | Self::False, Some(b)) => !b,
            (Self::NotTrue | Self::False, None) => matches!(self, Self::NotTrue),
            (Self::Unknown, operand) => operand.is_none(),
            (Self::NotUnknown, operand) => operand.is_some(),
        }
    }
}

/// A SQL construct that requires a `boolean` operand — the one enumeration behind
/// 18.6's `argument of <CTX> must be type boolean, not type <T>`.
///
/// 18.6 funnels every one of these through a single function, `coerce_to_boolean`
/// (`src/backend/parser/parse_coerce.c`), which attempts a `COERCION_ASSIGNMENT`
/// coercion to `boolean` and names the construct in the failure. This enum is the
/// construct half of that message, and it carries the SQL spelling so the sentence is
/// single-homed rather than reconstructed at each call site.
///
/// The members are the positions **pglet's grammar can reach**. 18.6 has more of them
/// — `COPY … WHERE`, an aggregate `FILTER (WHERE …)`, `MERGE`, a domain `CHECK`,
/// `CREATE RULE`, an RLS policy's `USING` / `WITH CHECK`, a trigger's `WHEN`, a
/// publication's row filter, `XMLPARSE` — and this parser refuses every one of those
/// spellings before any operand reaches a gate, measured on this build: `42601` from
/// the grammar, `0A000` from the `CREATE` dispatch, `42704` at `XMLPARSE`.
///
/// **`JOIN … USING` is not one of those**, and does not belong in that list: the clause
/// parses and runs here — `SELECT count(*) FROM t x JOIN t y USING (k)` answers rows.
/// It is outside this enum for a reason of its own, that pglet builds no `ON`
/// expression for a `USING` merge and so has no operand to gate. On 18.6 what
/// `coerce_to_boolean` is handed at that call site is the result of an `=` it has just
/// built, which is always `boolean` and can never raise; the `IN` call site is the same
/// shape, and it too parses and runs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolContext {
    /// Every `WHERE`: a `SELECT`'s, an `UPDATE`/`DELETE`'s, an
    /// `ON CONFLICT … DO UPDATE`'s action filter, and a partial index's predicate —
    /// 18.6 spells all five `"WHERE"`.
    Where,
    Having,
    JoinOn,
    /// A **searched** `CASE WHEN <cond>`. The simple form (`CASE x WHEN v`) is not a
    /// member: 18.6 expands it to `x = v` and coerces *that*, so the operand it gates
    /// is the comparison's `boolean` result and the label itself is judged by the `=`
    /// operator rule instead.
    CaseWhen,
    And,
    Or,
    Not,
    Check,
    /// The six `x IS [NOT] TRUE|FALSE|UNKNOWN` spellings, which name themselves.
    Test(BoolTest),
}

impl BoolContext {
    /// The SQL spelling 18.6's message names the construct by.
    #[must_use]
    pub const fn sql(self) -> &'static str {
        match self {
            Self::Where => "WHERE",
            Self::Having => "HAVING",
            Self::JoinOn => "JOIN/ON",
            Self::CaseWhen => "CASE/WHEN",
            Self::And => "AND",
            Self::Or => "OR",
            Self::Not => "NOT",
            Self::Check => "CHECK",
            Self::Test(test) => test.sql(),
        }
    }
}

/// One `ORDER BY` sort key — an expression, its direction, and its NULL placement.
///
/// Replaces the old `(Expr, bool)` tuple so the null-placement is a **named** field
/// that can't be silently dropped (the bug this closes: `NULLS FIRST/LAST` was
/// parsed then discarded, always applying the direction default). Shared by every
/// consumer of an `ORDER BY`: the [`Plan::Sort`](super::Plan::Sort) key list, a
/// window [`Expr::Window`] `ORDER BY`, `DISTINCT ON`'s effective sort, and a set-op
/// trailing `ORDER BY`.
///
/// Serde-derived because it rides inside [`Expr::Window`], which the catalog
/// persists via postcard for CHECK / partial-index predicates. A window function can
/// never reach a persisted `Expr` context: Postgres forbids window functions there
/// (`42P20`) and pglet rejects them at DDL time too — *and* rejects any **subquery**
/// (`0A000`) in those contexts, closing the one remaining path (a window nested
/// inside a subquery). So no persisted `Expr` ever carries an `OrderKey`; widening
/// this needs no store-magic bump.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderKey {
    /// The sort-key expression (or a bare integer literal — a positional
    /// `ORDER BY <n>` reference the executor resolves by output-column index).
    pub expr: Expr,
    /// `true` = `ASC`, `false` = `DESC`.
    pub asc: bool,
    /// Explicit NULL placement: `Some(true)` = `NULLS FIRST`, `Some(false)` =
    /// `NULLS LAST`, `None` = unspecified — resolved to Postgres's direction
    /// default at compare time via [`Self::nulls_first_resolved`].
    pub nulls_first: Option<bool>,
}

impl OrderKey {
    /// A key with **unspecified** NULL placement (`nulls_first = None`) — the
    /// constructor for planner-synthesized keys (`DISTINCT ON` ON-key padding, the
    /// no-FROM `DISTINCT ON` sort) that carry no explicit `NULLS` clause and so
    /// take the direction default.
    #[must_use]
    pub const fn new(expr: Expr, asc: bool) -> Self {
        Self {
            expr,
            asc,
            nulls_first: None,
        }
    }

    /// Resolve the NULL placement to a concrete `nulls_first` bool, applying
    /// Postgres's direction default when unspecified: `ASC` → NULLS LAST
    /// (`false`), `DESC` → NULLS FIRST (`true`). The single source of the default
    /// rule, shared by every comparator so they can't drift.
    #[must_use]
    pub const fn nulls_first_resolved(&self) -> bool {
        match self.nulls_first {
            Some(nf) => nf,
            None => !self.asc,
        }
    }
}

/// A type modifier captured on a cast target or a column declaration.
///
/// The `(p,s)` of `numeric(p,s)`, the character length `(n)` of `varchar(n)` /
/// `char(n)`, or the sub-second precision `(p)` of `timestamp(p)` / `timestamptz(p)`
/// / `time(p)` / `interval(p)`.
///
/// Persisted (via [`Expr::CastTypmod`] inside a CHECK / partial-index predicate),
/// so it derives serde and its variant order is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Typmod {
    /// `numeric(precision, scale)` — round to `scale`, precision-overflow → `22003`.
    /// Context-insensitive (column store and cast enforce it identically).
    Numeric { precision: u16, scale: i16 },
    /// `varchar(n)` / `char(n)` — a character-length cap. On a column store an
    /// over-length value truncates iff every excess character is a blank, else
    /// `22001`; on an explicit cast it always truncates silently.
    Length(u32),
    /// The sub-second precision of `timestamp(p)` / `timestamptz(p)` / `time(p)` /
    /// `interval(p)` — round the value's microseconds to `p` fractional digits, half
    /// away from zero (`pglet_core::types::round_micros_to_precision`).
    /// Context-insensitive, like [`Self::Numeric`]: 18.6 runs the same
    /// `Adjust*ForTypmod` for a column store and an explicit cast. Already clamped to
    /// `pglet_core::types::MAX_TEMPORAL_PRECISION` by
    /// [`resolve_column_type`](crate::executor::resolve_column_type), so `6` here is a
    /// *captured* modifier whose rounding is a no-op — distinct from a bare
    /// `timestamp`, which carries no `Typmod` at all.
    ///
    /// Appended **last** so existing postcard-encoded `Typmod`s keep their variant
    /// discriminants.
    Precision(u8),
}

/// A sequence-manipulation function, callable as a top-level `SELECT` projection
/// (nextval/currval/setval) or in `INSERT … VALUES(…)` (nextval/setval).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequenceFunc {
    /// `nextval(seq)` — advance the sequence by one and return the new value.
    Nextval,
    /// `currval(seq)` — the last value `nextval` returned for this sequence in this
    /// session (fails loud with `55000` if none).
    Currval,
    /// `setval(seq, n [, is_called])` — set the sequence's counter (persists,
    /// non-transactional).
    Setval,
}

impl SequenceFunc {
    /// The lowercased function name — the unaliased projection column name and the
    /// spelling used in error messages.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Nextval => "nextval",
            Self::Currval => "currval",
            Self::Setval => "setval",
        }
    }

    /// Whether calling this advances the sequence counter (nextval/setval), so a
    /// projection containing it must persist on the auto-commit write path.
    pub(crate) const fn advances(self) -> bool {
        matches!(self, Self::Nextval | Self::Setval)
    }
}

/// An `AND`/`OR` run at most this long is built as the left-deep chain of plain
/// [`BinaryOp::And`] / [`BinaryOp::Or`] nodes; a longer one is built
/// balanced from list nodes ([`Expr::bool_list`]).
const BOOL_LIST_LEFT_DEEP_MAX: usize = 64;

/// Each level of the catalog encoding takes the depth bounds
/// ([`crate::depth::EncodingLevel`]): a store written where the count or the stack was
/// larger is refused `54001` where it is not, rather than overflowing the reader.
impl Serialize for Expr {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let _level = crate::depth::EncodingLevel::enter().map_err(serde::ser::Error::custom)?;
        Self::serialize(self, serializer)
    }
}

/// See the `Serialize` impl.
impl<'de> Deserialize<'de> for Expr {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let _level = crate::depth::EncodingLevel::enter().map_err(serde::de::Error::custom)?;
        Self::deserialize(deserializer)
    }
}

impl Expr {
    /// Build an `AND` (`op` = [`BinaryOp::And`]) or `OR` list over `operands`, in order.
    ///
    /// 18.6's node for these is n-ary (`makeAndExpr` appends to one `BoolExpr` on
    /// sight), so it answers a chain of any length. A run of up to
    /// [`BOOL_LIST_LEFT_DEEP_MAX`] operands keeps pglet's left-deep binary shape; a
    /// longer one is built **balanced**, so its depth is logarithmic in its length and
    /// no walker recurses once per operand: a [`BinaryOp::AndList`] node at its root and
    /// [`BinaryOp::AndListInner`] nodes below it. The root marks where the list begins,
    /// so an operand that is itself a long list — one the user parenthesized — stays
    /// one operand to every reader that asks for a list's operands
    /// (`Self::bool_node_operands`). The shape is a function of the operand list
    /// alone, so two parses of the same list build equal trees. Both shapes mean the
    /// same thing: the connective is associative in three-valued logic, and an in-order
    /// walk of either visits the operands left to right, so evaluation reaches the same
    /// operands in the same order and stops at the same one: a subtree whose left operand
    /// decides it answers the deciding value to its parent (`connective_decided_by`).
    ///
    /// `operands` must be non-empty; one operand is returned unwrapped.
    #[must_use]
    pub fn bool_list(op: BinaryOp, mut operands: Vec<Self>) -> Self {
        let op = op.connective();
        if operands.len() <= BOOL_LIST_LEFT_DEEP_MAX {
            let mut iter = operands.into_iter();
            let first = iter.next().unwrap_or(Self::Literal(Literal::Null));
            return iter.fold(first, |left, right| Self::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            });
        }
        let inner = op.list_inner_form();
        // Pair adjacent nodes level by level — iterative, so building the list is
        // itself no deeper than the list.
        while operands.len() > 1 {
            let mut next = Vec::with_capacity(operands.len().div_ceil(2));
            let mut iter = operands.into_iter();
            while let Some(left) = iter.next() {
                match iter.next() {
                    Some(right) => next.push(Self::BinaryOp {
                        left: Box::new(left),
                        op: inner,
                        right: Box::new(right),
                    }),
                    None => next.push(left),
                }
            }
            operands = next;
        }
        match operands.pop() {
            // More than `BOOL_LIST_LEFT_DEEP_MAX` operands pair into at least one node,
            // and the last one built is the root.
            Some(Self::BinaryOp { left, right, .. }) => Self::BinaryOp {
                left,
                op: op.list_form(),
                right,
            },
            other => other.unwrap_or(Self::Literal(Literal::Null)),
        }
    }

    /// Append the operands of the `AND`/`OR` list `self` is to `out`, in order — `self`
    /// alone when it is not one. `self` is read as the **first** operand of a list, so
    /// a list there flattens into the enclosing one as `makeAndExpr` flattens it
    /// (`(a AND b) AND c` is a three-operand list), while an operand the user
    /// parenthesized anywhere else stays one operand.
    pub(crate) fn bool_operands<'a>(&'a self, op: BinaryOp, out: &mut Vec<&'a Self>) {
        match self {
            Self::BinaryOp {
                left,
                op: node_op,
                right,
            } if node_op.connective() == op.connective() && !node_op.is_list_inner() => {
                Self::bool_node_operands(left, *node_op, right, out);
            }
            other => out.push(other),
        }
    }

    /// Append to `out`, in order, the operands of the `AND`/`OR` list whose top node is
    /// `left op right`: a plain chain's left spine and its last operand, or every leaf
    /// under a long list's root reached through its [`BinaryOp::AndListInner`] nodes —
    /// never into an operand that is a list of its own.
    pub(crate) fn bool_node_operands<'a>(
        left: &'a Self,
        op: BinaryOp,
        right: &'a Self,
        out: &mut Vec<&'a Self>,
    ) {
        if op == op.connective().list_form() {
            let inner = op.connective().list_inner_form();
            left.list_leaves(inner, out);
            right.list_leaves(inner, out);
        } else {
            left.bool_operands(op, out);
            out.push(right);
        }
    }

    fn list_leaves<'a>(&'a self, inner: BinaryOp, out: &mut Vec<&'a Self>) {
        match self {
            Self::BinaryOp {
                left,
                op: node_op,
                right,
            } if *node_op == inner => {
                left.list_leaves(inner, out);
                right.list_leaves(inner, out);
            }
            other => out.push(other),
        }
    }
}

/// The shared child visitors — the single source of truth for an expression's
/// direct scalar sub-expressions, for both read-only walks (`for_each_child` /
/// `try_for_each_child`) and owned transforms (`map_children`). See the note on
/// [`Expr`].
impl Expr {
    /// Apply `f` to each **direct scalar sub-expression** of `self`.
    ///
    /// A self-contained sub-`Plan` (`Subquery`/`Exists`, and the plan side of
    /// `InSubquery`) is treated as a **leaf** — walkers never descend into it (its
    /// columns resolve internally). A `Window`'s `partition_by`/`order_by` operands
    /// and a `Case`'s operand/branches **are** children. A walker that needs a
    /// narrower or differently-rooted set (`contains_aggregate` treating `Window`
    /// as a leaf, `contains_unnest`'s narrower set) handles those variants
    /// explicitly *before* delegating here.
    pub(crate) fn for_each_child<'a>(&'a self, mut f: impl FnMut(&'a Self)) {
        self.for_each_child_ctx(|child, _| f(child));
    }

    /// Whether `verdict` answers `Some(true)` for this node or for one beneath it. A
    /// `Some(false)` is the answer for that node's whole subtree, which is not walked;
    /// `None` leaves it to the node's children ([`Self::for_each_child`]'s set).
    pub(crate) fn any_node(&self, verdict: &dyn Fn(&Self) -> Option<bool>) -> bool {
        if let Some(answer) = verdict(self) {
            return answer;
        }
        let mut found = false;
        self.for_each_child(|child| found = found || child.any_node(verdict));
        found
    }

    /// [`Self::for_each_child`] with each child's **boolean context** beside it: the
    /// [`BoolContext`] this node imposes on that operand, or `None` for an ordinary
    /// one.
    ///
    /// One arm list, not two. A walker that gates boolean operands needs the same
    /// child set and the same order as every other walker, and a second enumeration
    /// of "which children are boolean operands" would go stale against this one the
    /// first time a variant is added — the drift seam this engine keeps paying for.
    /// So the context rides the existing walk and [`Self::for_each_child`] drops it.
    ///
    /// The order is what makes an **interleaved** gate expressible: a caller that
    /// recurses into a child and then gates it reproduces 18.6's per-argument
    /// `transformExprRecurse`-then-`coerce_to_boolean` loop, which is why
    /// `CHECK (a AND b LIKE 'x')` is `42804` and the reverse spelling `42883`.
    pub fn for_each_child_ctx<'a>(&'a self, mut f: impl FnMut(&'a Self, Option<BoolContext>)) {
        // The chain-forming variants' children come from `chain_operands`, which is how
        // a walker recursing through them from its own frame reads the same set.
        if let Some(operands) = self.chain_operands() {
            f(operands.first, operands.ctx);
            if let Some(second) = operands.second {
                f(second, operands.ctx);
            }
            for item in operands.rest {
                f(item, None);
            }
            return;
        }
        match self {
            // Leaves: no scalar children (a self-contained `Plan` is a leaf) — and the
            // chain-forming nodes, taken by `chain_operands` above.
            Self::Negate(_)
            | Self::UnaryPlus(_)
            | Self::IsNull(_)
            | Self::IsNotNull(_)
            | Self::Cast { .. }
            | Self::CastTypmod { .. }
            | Self::BooleanTest { .. }
            | Self::Not(_)
            | Self::BinaryOp { .. }
            | Self::AnyAll { .. }
            | Self::InList { .. }
            | Self::InSubquery { .. }
            | Self::Literal(_)
            | Self::Column { .. }
            | Self::Default
            | Self::Parameter { .. }
            | Self::Bound(_)
            | Self::Builtin(_)
            | Self::Subquery { .. }
            | Self::Exists { .. } => {}
            // Single-child compounds (the `InSubquery` plan is a leaf).
            Self::Alias { expr: e, .. }
            | Self::Aggregate { arg: e, .. }
            | Self::Unnest(e)
            | Self::Lower(e)
            | Self::ColumnSize { arg: e, .. } => f(e, None),
            // `EXTRACT(field FROM source)` — the field-name expr, then the source
            // value. **Field first**, matching source order for *both* spellings that
            // build this node: Postgres's own parse node is field-first
            // (`gram.y`'s `extract_list: extract_arg FROM a_expr` →
            // `list_make2(makeStringConst($1), $3)`), and the `date_part(field,
            // source)` comma call reads field-first on the page. The struct's *field
            // declaration* order is the opposite and cannot be changed — postcard
            // encodes a stored `Expr` in declaration order — so the two orders differ
            // here on purpose. It is load-bearing: a walker that reports the first
            // failing child (`check_stored_expr_nodes`) must render Postgres's
            // positionally-first verdict, and `date_part` accepts an arbitrary
            // expression in either slot.
            Self::Extract { source, field, .. } => {
                f(field, None);
                f(source, None);
            }
            Self::Array(items)
            | Self::JsonbBuildObject(items)
            | Self::Coalesce(items)
            | Self::MathFn { args: items, .. }
            | Self::StringFn { args: items, .. }
            | Self::VariadicFn { args: items, .. }
            // A sequence function's only scalar children are its (setval) args.
            | Self::SequenceFn { args: items, .. } => {
                for item in items {
                    f(item, None);
                }
            }
            Self::Case {
                operand,
                when_clauses,
                else_result,
            } => {
                if let Some(op) = operand {
                    f(op, None);
                }
                // A **searched** `CASE`'s condition is the boolean context; a simple
                // one's label is the right-hand side of an `=` 18.6 builds first, so
                // the context is the comparison's, not this node's.
                let cond_ctx = operand.is_none().then_some(BoolContext::CaseWhen);
                for (cond, result) in when_clauses {
                    f(cond, cond_ctx);
                    f(result, None);
                }
                if let Some(else_expr) = else_result {
                    f(else_expr, None);
                }
            }
            Self::Window {
                partition_by,
                order_by,
                ..
            } => {
                for e in partition_by {
                    f(e, None);
                }
                for key in order_by {
                    f(&key.expr, None);
                }
            }
        }
    }

    /// The children of a **chain-forming** node, in [`Self::for_each_child_ctx`]'s
    /// order and with its contexts — a binary operator's two (a quantified `ANY`/`ALL`
    /// comparison's included), the one of a unary sign, `NOT`, a postfix test or a cast,
    /// and an `IN`'s left operand, then its list — and `None` for every other node.
    ///
    /// Those are the nodes a tree can be arbitrarily deep in without parser nesting —
    /// every node `parser::expr`'s operator loop builds around the chain so far but one —
    /// so a walker recurses through them from its own frame and hands only the rest to
    /// the visitor, whose closure layers cost frames of their own — three a level on wasm
    /// (`crate::depth`). The one is a `::regclass` target's string-function node, walked
    /// from the dispatch frames and charged `crate::depth::DISPATCH_LEVEL_WEIGHT` a level
    /// for it; `parser::operator_loop_arms` reads the loop's kinds from its builders and
    /// fails on any other outside this set. [`Self::for_each_child_ctx`] reads its own
    /// arms for these nodes from here, so the child set stays one.
    #[inline]
    pub(crate) fn chain_operands(&self) -> Option<ChainOperands<'_>> {
        match self {
            Self::BinaryOp { left, op, right } => Some(ChainOperands {
                first: left,
                second: Some(right),
                ctx: op.operand_context(),
                rest: &[],
            }),
            Self::AnyAll { left, right, .. } => Some(ChainOperands {
                first: left,
                second: Some(right),
                ctx: None,
                rest: &[],
            }),
            Self::Negate(e)
            | Self::UnaryPlus(e)
            | Self::IsNull(e)
            | Self::IsNotNull(e)
            | Self::Cast { expr: e, .. }
            | Self::CastTypmod { expr: e, .. }
            | Self::BooleanTest { expr: e, .. }
            // The `IN (subquery)` plan is a leaf.
            | Self::InSubquery { expr: e, .. } => Some(ChainOperands {
                first: e,
                second: None,
                ctx: None,
                rest: &[],
            }),
            Self::InList { expr, list, .. } => Some(ChainOperands {
                first: expr,
                second: None,
                ctx: None,
                rest: list,
            }),
            // `NOT x` — 18.6's `transformBoolExpr` coerces the one argument.
            Self::Not(e) => Some(ChainOperands {
                first: e,
                second: None,
                ctx: Some(BoolContext::Not),
                rest: &[],
            }),
            _ => None,
        }
    }

    /// Like [`Self::for_each_child`] but short-circuits: apply `f` to each direct
    /// child in order, returning the **first** `Err` (skipping the rest) or
    /// `Ok(())` when every child succeeds. Lets the `Result`-returning validators
    /// fold over the shared child set without re-listing every variant.
    pub(crate) fn try_for_each_child(&self, mut f: impl FnMut(&Self) -> Result<()>) -> Result<()> {
        self.try_for_each_child_ctx(|child, _| f(child))
    }

    /// [`Self::try_for_each_child`] over [`Self::for_each_child_ctx`] — each child with
    /// the [`BoolContext`] its parent imposes, short-circuiting on the first `Err`.
    pub(crate) fn try_for_each_child_ctx(
        &self,
        mut f: impl FnMut(&Self, Option<BoolContext>) -> Result<()>,
    ) -> Result<()> {
        let mut result = Ok(());
        self.for_each_child_ctx(|child, ctx| {
            if result.is_ok() {
                result = f(child, ctx);
            }
        });
        result
    }

    /// The owned counterpart of [`Self::for_each_child`]: rebuild `self`, replacing
    /// each **direct scalar sub-expression** with `f(child)` and carrying every
    /// non-`Expr` field (operators, target types, aliases, flags) through unchanged.
    /// A leaf (including a self-contained sub-`Plan`) is returned as-is. Children are
    /// visited in the same order as `for_each_child`. Lets an owned tree transform
    /// (`resolve_expr_types`) handle only the variants it rewrites and delegate the
    /// rest here, rather than re-listing every variant's rebuild.
    pub(crate) fn map_children(mut self, mut f: impl FnMut(Self) -> Self) -> Self {
        self.for_each_child_mut(&mut |slot: &mut Self| {
            let taken = std::mem::replace(slot, Self::Default);
            *slot = f(taken);
        });
        self
    }

    /// [`Self::map_children`] with a fallible `f`: the first `Err` is returned and no
    /// later child is visited.
    pub(crate) fn try_map_children(
        mut self,
        mut f: impl FnMut(Self) -> Result<Self>,
    ) -> Result<Self> {
        let mut result = Ok(());
        self.for_each_child_mut(&mut |slot: &mut Self| {
            if result.is_ok() {
                let taken = std::mem::replace(slot, Self::Default);
                match f(taken) {
                    Ok(mapped) => *slot = mapped,
                    Err(e) => result = Err(e),
                }
            }
        });
        result.map(|()| self)
    }

    /// In-place visitor over the same direct children, in the same order, as
    /// [`Self::map_children`] — the frame holds only references.
    pub(crate) fn for_each_child_mut<F: FnMut(&mut Self)>(&mut self, f: &mut F) {
        match self {
            Self::Literal(_)
            | Self::Column { .. }
            | Self::Default
            | Self::Parameter { .. }
            | Self::Bound(_)
            | Self::Builtin(_)
            | Self::Subquery { .. }
            | Self::Exists { .. } => {}
            Self::Negate(e)
            | Self::UnaryPlus(e)
            | Self::Not(e)
            | Self::IsNull(e)
            | Self::IsNotNull(e)
            | Self::Cast { expr: e, .. }
            | Self::CastTypmod { expr: e, .. }
            | Self::Alias { expr: e, .. }
            | Self::Aggregate { arg: e, .. }
            | Self::Unnest(e)
            | Self::Lower(e)
            | Self::ColumnSize { arg: e, .. }
            | Self::BooleanTest { expr: e, .. }
            | Self::InSubquery { expr: e, .. } => f(e),
            Self::BinaryOp { left, right, .. } | Self::AnyAll { left, right, .. } => {
                f(left);
                f(right);
            }
            Self::InList { expr, list, .. } => {
                f(expr);
                for e in list {
                    f(e);
                }
            }
            Self::Array(items)
            | Self::JsonbBuildObject(items)
            | Self::Coalesce(items)
            | Self::MathFn { args: items, .. }
            | Self::StringFn { args: items, .. }
            | Self::VariadicFn { args: items, .. }
            | Self::SequenceFn { args: items, .. } => {
                for e in items {
                    f(e);
                }
            }
            Self::Extract { source, field, .. } => {
                f(field);
                f(source);
            }
            Self::Case {
                operand,
                when_clauses,
                else_result,
            } => {
                if let Some(o) = operand {
                    f(o);
                }
                for (c, r) in when_clauses.iter_mut() {
                    f(c);
                    f(r);
                }
                if let Some(e) = else_result {
                    f(e);
                }
            }
            Self::Window {
                partition_by,
                order_by,
                ..
            } => {
                for e in partition_by {
                    f(e);
                }
                for k in order_by {
                    f(&mut k.expr);
                }
            }
        }
    }
}

/// A chain-forming node's children ([`Expr::chain_operands`]).
#[derive(Debug, Clone, Copy)]
pub struct ChainOperands<'a> {
    /// The first child.
    pub first: &'a Expr,
    /// The second, for a binary operator.
    pub second: Option<&'a Expr>,
    /// The boolean context both are in, if any.
    pub ctx: Option<BoolContext>,
    /// The children after those — an `IN` list's elements — in no boolean context.
    pub rest: &'a [Expr],
}

/// Niladic builtin scalar functions.
///
/// Impure (clock/RNG), so evaluated at execution time — `gen_random_uuid()` and
/// `clock_timestamp()` at each call, the rest of the clock family from the instant
/// the transaction or the statement started. Both legs are backed: native uses the
/// OS clock/RNG, wasm the JS host (`Date.now` / `Math.random`) — see `builtins`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinFn {
    /// `now()` → the transaction's start, in UTC (`timestamptz`).
    Now,
    /// `gen_random_uuid()` / `uuid_generate_v4()` → random v4 UUID.
    RandomUuid,
    /// `CURRENT_DATE` → the current UTC date (`date`).
    CurrentDate,
    /// `CURRENT_TIME` → the current UTC time-of-day. Postgres's `CURRENT_TIME` is
    /// `timetz`; pglet is UTC-only, so it returns a plain `time` (documented
    /// divergence). Kept distinct from `LocalTime` only so the output column name is
    /// `current_time`. Appended after `CurrentDate` for postcard-discriminant
    /// stability.
    CurrentTime,
    /// `LOCALTIME` → the current UTC time-of-day (plain `time`). Distinct from
    /// `CurrentTime` only for the `localtime` output column name. Appended last for
    /// postcard-discriminant stability.
    LocalTime,
    /// `LOCALTIMESTAMP` → the current timestamp **without** time zone (`timestamp`).
    /// The same instant/value as `Now` (`CURRENT_TIMESTAMP`), but typed `timestamp`
    /// rather than `timestamptz` — so its wire text renders without the `+00` offset.
    /// pglet is UTC-only, so both are the same `Value::Timestamp`; only the inferred
    /// `TypeOid` (and the `localtimestamp` output column name) differs. Appended last
    /// for postcard-discriminant stability.
    LocalTimestamp,
    /// `transaction_timestamp()` → `timestamptz`, the transaction's start as `Now` is.
    /// Distinct from `Now` for the `transaction_timestamp` output column name and the
    /// `column_default` / dump rendering. Appended for postcard-discriminant stability.
    TransactionTimestamp,
    /// `statement_timestamp()` → `timestamptz`, the statement's start. Appended for
    /// postcard-discriminant stability.
    StatementTimestamp,
    /// `clock_timestamp()` → `timestamptz`, re-read per call on both engines. The
    /// one member of this family whose column `DEFAULT` an `ADD COLUMN` backfill
    /// resolves **per row**. Appended for postcard-discriminant stability.
    ClockTimestamp,
    /// `CURRENT_TIMESTAMP` → `timestamptz`. The SQL value function (parenless special
    /// form), kept distinct from `Now` because `pg_get_expr` renders it as the bare
    /// keyword and its output column name is `current_timestamp`. Appended for
    /// postcard-discriminant stability.
    CurrentTimestamp,
    /// `CURRENT_USER` → `name`. The session's role
    /// ([`crate::catalog::SessionSettings::user`]).
    ///
    /// The four role spellings (`CURRENT_USER`, `SESSION_USER`, `USER`,
    /// `CURRENT_ROLE`) evaluate identically here and stay four variants because 18.6
    /// keeps them four: each is its own `SVFOP_*` and `pg_get_expr` renders a stored
    /// `DEFAULT` back as the **written** keyword — measured `CURRENT_USER`,
    /// `SESSION_USER`, `USER`, `CURRENT_ROLE`. Appended for postcard-discriminant
    /// stability.
    CurrentUser,
    /// `SESSION_USER` → `name`. See [`Self::CurrentUser`].
    SessionUser,
    /// `USER` → `name`. See [`Self::CurrentUser`].
    User,
    /// `CURRENT_ROLE` → `name`. See [`Self::CurrentUser`].
    CurrentRole,
    /// `CURRENT_CATALOG` → `name`. The session's database, and the parenless twin of
    /// [`Self::CurrentDatabase`] — a RESERVED_KEYWORD on 18.6 with no `pg_proc`
    /// entry, so `current_catalog()` is `42601` there and here.
    CurrentCatalog,
    /// `current_database()` → `name`. The session's database, spelled as a call.
    /// Distinct from [`Self::CurrentCatalog`] for its output column name
    /// (`current_database`) and its stored-`DEFAULT` rendering (`current_database()`
    /// against `CURRENT_CATALOG`).
    CurrentDatabase,
    /// `CURRENT_SCHEMA` → `name`, the parenless spelling. The first element of the
    /// search path pglet resolves to, which is the constant `public`
    /// ([`crate::catalog::DEFAULT_SEARCH_PATH`] never moves — `EDGES.md` row 66).
    CurrentSchema,
    /// `current_schema()` → `name`, the **called** spelling of [`Self::CurrentSchema`].
    ///
    /// `current_schema` is 18.6's one dual-form member — a TYPE_FUNC_NAME_KEYWORD
    /// that is both an `SVFOP_CURRENT_SCHEMA` and `pg_proc` 1402 — and the two
    /// spellings render differently in a stored `DEFAULT`: `CURRENT_SCHEMA` bare
    /// against `"current_schema"()` quoted, because an unquoted `current_schema()`
    /// would re-lex as the keyword. Both measured on 18.6.
    CurrentSchemaFn,
    /// `version()` → `text`. A fabricated build banner (`crate::builtins::version`)
    /// whose release must agree with `SHOW server_version`.
    Version,
    /// `random()` → `double precision` in `[0, 1)`, drawn afresh at each call — 18.6's
    /// `VOLATILE` function, so each row of a scan and each call in one expression
    /// answers its own draw. The bits come from the host (`crate::builtins::random`).
    /// Appended last for postcard-discriminant stability.
    Random,
}

impl BuiltinFn {
    /// Whether 18.6 marks the function `VOLATILE` (`pg_proc.provolatile = 'v'`): its value
    /// may change at each call, so it is evaluated once a row and never once for a scan.
    /// `random()`, `gen_random_uuid()` and `clock_timestamp()`; the `now` family, the
    /// session forms and `version()` are `STABLE` there.
    #[must_use]
    pub const fn is_volatile(self) -> bool {
        matches!(self, Self::Random | Self::RandomUuid | Self::ClockTimestamp)
    }
}

/// A SQL *value function* — one of Postgres's parenless special-form keywords
/// (`func_expr_common_subexpr`).
///
/// These take no argument list (with a precision variant `CURRENT_TIMESTAMP(0)`
/// pglet does not model) and are *not* ordinary functions (`now()` is) — with one
/// exception: `current_schema` is 18.6's one dual-form member and does route through
/// [`build_call`] when written with parens ([`Self::takes_no_argument_list`]).
/// Classified in one place so every parse position agrees on what these words mean;
/// a member fails loud rather than degrading to a column reference, keeping "pglet
/// accepts ⇔ Postgres accepts". Each lowers to an [`Expr::Builtin`], which is
/// **persistable** — that is what lets `DEFAULT CURRENT_USER` be stored (as a
/// `ColumnDefault::Expr`) and deparsed rather than refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlValueFunction {
    /// `CURRENT_DATE` → `date`. **Supported** (`BuiltinFn::CurrentDate`).
    CurrentDate,
    /// `CURRENT_TIMESTAMP` → `timestamptz`. **Supported** (≡ `BuiltinFn::Now`).
    CurrentTimestamp,
    /// `CURRENT_TIME` → `timetz` in Postgres; pglet returns a plain `time` (UTC-only).
    /// **Supported** (`BuiltinFn::CurrentTime`).
    CurrentTime,
    /// `LOCALTIME` → `time`. **Supported** (`BuiltinFn::LocalTime`).
    LocalTime,
    /// `LOCALTIMESTAMP` → `timestamp`. **Supported** (`BuiltinFn::LocalTimestamp` — the
    /// same instant as `CURRENT_TIMESTAMP`, typed without a time zone).
    LocalTimestamp,
    /// `CURRENT_USER` → `name`. **Supported** (`BuiltinFn::CurrentUser`).
    CurrentUser,
    /// `SESSION_USER` → `name`. **Supported** (`BuiltinFn::SessionUser`).
    SessionUser,
    /// `USER` → `name`. **Supported** (`BuiltinFn::User`).
    ///
    /// A RESERVED_KEYWORD on 18.6, so a bare `user` is *always* this and never a
    /// column — measured over a table holding a `"user"` column, where `SELECT user`
    /// answers the role and `SELECT "user"` the column. pglet's lexer still accepts
    /// `user` as an unquoted column *name* in DDL where 18.6 refuses it `42601`;
    /// that pre-existing gap is what makes such a column unreachable unquoted here,
    /// exactly as a quoted `"user"` column is on 18.6.
    User,
    /// `CURRENT_ROLE` → `name`. **Supported** (`BuiltinFn::CurrentRole`).
    CurrentRole,
    /// `CURRENT_CATALOG` → `name`. **Supported** (`BuiltinFn::CurrentCatalog`).
    CurrentCatalog,
    /// `CURRENT_SCHEMA` → `name`. **Supported** (`BuiltinFn::CurrentSchema`), and
    /// the one member with a call spelling too — see [`Self::takes_no_argument_list`].
    CurrentSchema,
}

impl SqlValueFunction {
    /// Classify a parsed name as a SQL value function; `None` for any other name (an
    /// ordinary column or function).
    ///
    /// The comparison is **exact**, because `name` arrives already downcased when the
    /// source wrote it unquoted. Each of these is a *keyword* to Postgres, so a
    /// quoted `"CURRENT_DATE"` is an ordinary identifier there — a column reference,
    /// not the value function — and matching it here would resolve a special form the
    /// source never wrote.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "current_date" => Some(Self::CurrentDate),
            "current_timestamp" => Some(Self::CurrentTimestamp),
            "current_time" => Some(Self::CurrentTime),
            "localtime" => Some(Self::LocalTime),
            "localtimestamp" => Some(Self::LocalTimestamp),
            "current_user" => Some(Self::CurrentUser),
            "session_user" => Some(Self::SessionUser),
            "user" => Some(Self::User),
            "current_role" => Some(Self::CurrentRole),
            "current_catalog" => Some(Self::CurrentCatalog),
            "current_schema" => Some(Self::CurrentSchema),
            _ => None,
        }
    }

    /// Whether a following `(` is a syntax error for this form.
    ///
    /// True for every member but [`Self::CurrentSchema`], which 18.6 also carries as
    /// `pg_proc` 1402. Measured on 18.6: `current_user()`, `session_user()`,
    /// `user()`, `current_role()` and `current_catalog()` are each
    /// `42601 syntax error at or near "("`, while `current_schema()` answers `public`
    /// and `current_schema(1)` is
    /// `42883 function current_schema(integer) does not exist`. So the false arm is
    /// not "accept anything": it hands the call to [`build_call`], which resolves the
    /// zero-argument signature and refuses every other arity the way 18.6 does.
    #[must_use]
    pub const fn takes_no_argument_list(self) -> bool {
        !matches!(self, Self::CurrentSchema)
    }

    /// The canonical lower-case spelling, for error messages.
    #[must_use]
    pub const fn sql_name(self) -> &'static str {
        match self {
            Self::CurrentDate => "current_date",
            Self::CurrentTimestamp => "current_timestamp",
            Self::CurrentTime => "current_time",
            Self::LocalTime => "localtime",
            Self::LocalTimestamp => "localtimestamp",
            Self::CurrentUser => "current_user",
            Self::SessionUser => "session_user",
            Self::User => "user",
            Self::CurrentRole => "current_role",
            Self::CurrentCatalog => "current_catalog",
            Self::CurrentSchema => "current_schema",
        }
    }

    /// The [`BuiltinFn`] this parenless form lowers to.
    ///
    /// One home for the mapping the parser used to spell inline, so a member added
    /// to this enum cannot be lowered in one parse position and missed in another.
    #[must_use]
    pub const fn builtin(self) -> BuiltinFn {
        match self {
            Self::CurrentDate => BuiltinFn::CurrentDate,
            // Kept distinct from `now()`: the same instant, but `pg_get_expr`
            // renders it as the bare keyword and its output column name is
            // `current_timestamp` (both measured on 18.6).
            Self::CurrentTimestamp => BuiltinFn::CurrentTimestamp,
            // `CURRENT_TIME` / `LOCALTIME` both yield the current UTC time-of-day
            // (a plain `time`); kept distinct only for their output column names.
            Self::CurrentTime => BuiltinFn::CurrentTime,
            Self::LocalTime => BuiltinFn::LocalTime,
            // `LOCALTIMESTAMP` — the same instant as `CURRENT_TIMESTAMP` (≡ `Now`),
            // but typed `timestamp` (no tz) rather than `timestamptz`, so its wire
            // text renders without the `+00` offset.
            Self::LocalTimestamp => BuiltinFn::LocalTimestamp,
            Self::CurrentUser => BuiltinFn::CurrentUser,
            Self::SessionUser => BuiltinFn::SessionUser,
            Self::User => BuiltinFn::User,
            Self::CurrentRole => BuiltinFn::CurrentRole,
            Self::CurrentCatalog => BuiltinFn::CurrentCatalog,
            Self::CurrentSchema => BuiltinFn::CurrentSchema,
        }
    }
}

/// Aggregate function types.
///
/// These are evaluated over groups of rows (or all rows if no GROUP BY).
/// NULL values are excluded from all aggregates except COUNT(*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateFunc {
    /// COUNT(*) or COUNT(expr) - counts rows (excludes NULLs for expr).
    Count,
    /// SUM(expr) - sum of numeric values.
    Sum,
    /// AVG(expr) - average of numeric or `interval` values; its result type is [`aggregate_result_type`]'s.
    Avg,
    /// MIN(expr) - minimum value.
    Min,
    /// MAX(expr) - maximum value.
    Max,
}

/// Whether an aggregate's argument is **certainly** of an integer type.
///
/// Certain is what this engine says the type is being what 18.6's parse analysis says — the
/// one fact that decides whether `avg` and `sum(bigint)` take 18.6's exact `numeric` rows
/// ([`aggregate_result_type`]).
///
/// Written once, by the coercion pass (`planner::coercion`'s `aggregate_arg_certainty`),
/// and read by every reader of the aggregate's result type through [`aggregate_type_of`]:
/// the planner's `Describe` walk, the executor's descriptor walk, the certain-type gate,
/// the coercion pass's own branch reader, and `compute_aggregate`'s choice of accumulator.
/// One producer is the point: readers that each derived the verdict from what they could
/// see disagreed, and a value was sent under another type's declaration. It is set while
/// the plan is **built** — the pass runs at plan construction, as its casts are written —
/// never at eval time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArgCertainty {
    /// Not decided: the parser's value, which the coercion pass replaces at every
    /// aggregate it visits. No reader may ask it — `aggregate_type_of` refuses it in a
    /// debug build — so an aggregate the pass has not reached yet, the inner one of a
    /// nested pair among them, has no result type to read.
    #[default]
    Undecided,
    /// Decided: the argument is not an integer whose type is certain — a guessed type,
    /// or a certain type that is not an integer.
    NotCertainInteger,
    /// Decided: the argument is certainly of this integer type.
    CertainInteger(TypeOid),
}

/// A numeric math function (see [`Expr::MathFn`]).
///
/// New variants are appended **last** so existing postcard-encoded `Expr` IR
/// (index-key / CHECK predicates carrying a `MathFn`) keeps its discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MathFunc {
    /// `abs(x)` — absolute value (preserves the argument's type + scale).
    Abs,
    /// `ceil(x)` / `ceiling(x)` — round toward +∞ to an integer.
    Ceil,
    /// `floor(x)` — round toward −∞ to an integer.
    Floor,
    /// `round(x [, s])` — round half-away-from-zero to `s` (default 0) places.
    Round,
    /// `trunc(x [, s])` — truncate toward zero to `s` (default 0) places.
    Trunc,
    /// `sqrt(x)` — square root. Only `(float8)`/`(numeric)` overloads, so an
    /// **integer** argument resolves to `float8` (unlike `ceil`/`floor`, which
    /// promote `int → numeric`); a negative argument fails loud (`2201F`). Appended
    /// last for postcard-discriminant stability.
    Sqrt,
    /// `sign(x)` — the sign (`-1`/`0`/`1`) with the argument's family: `(float8)`
    /// for an integer/float argument, `(numeric)` for a numeric one. `sign(NaN)`
    /// follows Postgres (`0` for `float8`, `NaN` for `numeric`). Appended last for
    /// postcard-discriminant stability.
    Sign,
}

/// A strict-NULL scalar string function (see [`Expr::StringFn`]).
///
/// The comment on each variant names its Postgres function, argument count, and
/// result type. Every string index is 1-based and every length/position over a `text`
/// value is counted in **characters**, matching Postgres under a multibyte (UTF-8)
/// encoding; over a `bytea` value, and for `octet_length`, it is counted in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StringFunc {
    /// `upper(text) → text` — Unicode upper-case (the mirror of `lower`).
    Upper,
    /// `length(text) → int4` — the character count.
    Length,
    /// `replace(str, from, to) → text` — every non-overlapping `from` occurrence
    /// (an empty `from` or `str` leaves `str` unchanged).
    Replace,
    /// `substring(str, start [, length]) → text` — 1-based; a negative `length`
    /// fails loud (`22011`); the clip math follows Postgres (`substring('12345',
    /// -2, 5)` = `'12'`). A `bytea` `str` answers `bytea`, its bytes clipped by the same
    /// math.
    Substring,
    /// `ltrim(str [, chars]) → text` — trim leading characters in the `chars` set
    /// (default `' '`).
    Ltrim,
    /// `rtrim(str [, chars]) → text` — trim trailing characters in the `chars` set.
    Rtrim,
    /// `btrim(str [, chars]) → text` — trim both ends (`TRIM`'s default direction).
    Btrim,
    /// `strpos(str, sub) → int4` — the 1-based character position of `sub` in `str`,
    /// 0 if absent, 1 for an empty `sub`. Backs `POSITION(sub IN str)` (arg-swapped).
    Strpos,
    /// `lpad(str, len [, fill]) → text` — left-pad/truncate to `len` characters
    /// (default fill `' '`; an empty fill disables padding).
    Lpad,
    /// `rpad(str, len [, fill]) → text` — right-pad/truncate to `len` characters.
    Rpad,
    /// `split_part(str, delim, n) → text` — the 1-based `n`-th field (a negative `n`
    /// counts from the end); `n = 0` fails loud (`22023`); an out-of-range `n`
    /// yields `''`.
    SplitPart,
    /// `POSITION(sub IN str) → int4` — the special-form spelling of `strpos(str,
    /// sub)`. Evaluates identically to [`Self::Strpos`] (the parser builds the same
    /// `[str, sub]` arg order), differing only in its output column name
    /// (`position`, matching Postgres, vs `strpos`). Appended last for
    /// postcard-discriminant stability.
    Position,
    /// `date_trunc(field, source) → source-type` — truncate a `timestamp` /
    /// `timestamptz` / `interval` (a `date` implicit-casts to `timestamp`) down to a
    /// field boundary, zeroing every finer field. Modeled as a strict-NULL string
    /// function (either NULL argument → NULL) though its result type is the **source**
    /// type, not the fixed `text`/`int4` the other members carry — so `infer_type` /
    /// `expr_type_oid` special-case it via [`date_trunc_result_type`] rather than
    /// [`string_fn_result_type`]. `field` accepts only the units tier
    /// ([`ExtractField::is_units`]); a special-only field (`dow`/`epoch`/…) is
    /// unrecognized (`22023`), matching Postgres. Appended last for
    /// postcard-discriminant stability.
    DateTrunc,
    /// `current_setting(name [, missing_ok]) → text` — the value of a runtime
    /// parameter, read from the **same** table `SHOW <name>` answers from
    /// ([`crate::catalog::setting_value`]), so the function and the statement cannot
    /// disagree about a value or about which names exist.
    ///
    /// Strict in **both** arguments on 18.6 (measured: `current_setting(NULL)` and
    /// `current_setting('search_path', NULL)` are each NULL), which is exactly this
    /// family's hoisted NULL rule. An unrecognized name is `42704` — the identical
    /// error `SHOW` raises — unless `missing_ok` is true, in which case NULL.
    /// Appended last for postcard-discriminant stability.
    CurrentSetting,
    /// `current_schemas(include_implicit) → name[]` — the search path as the list of
    /// schemas that exist ([`crate::catalog::current_schemas`]).
    ///
    /// Strict, and the one member of this family returning an **array**, so its
    /// result type is `name[]` rather than the fixed `text`/`int4` the others carry
    /// (`string_fn_result_type` answers it directly; no source argument is involved,
    /// unlike [`Self::DateTrunc`]). Appended last for postcard-discriminant
    /// stability.
    CurrentSchemas,
    /// `pg_get_constraintdef(oid) → text` — a constraint's SQL definition, the string
    /// the fuz spine's schema snapshot diffs across backends.
    ///
    /// A function **of the catalog**, so it reads the schema the executor lends the
    /// [`EvalEnv`](crate::executor::EvalEnv); in a context with none it fails loud
    /// (`0A000`) rather than answering NULL, which would be indistinguishable from
    /// 18.6's answer for an OID no constraint holds. That NULL *is* 18.6's behaviour for
    /// an unknown OID (measured: `pg_get_constraintdef(999999)` is NULL, not an error) —
    /// which is why the two cases must not share an answer. The one renderer is
    /// [`crate::catalog::constraint_def`]. Appended last for postcard-discriminant
    /// stability.
    PgGetConstraintdef,
    /// `<expr>::regclass` — a relation reference.
    ///
    /// Modelled as a strict scalar function rather than a cast target because pglet has
    /// no `regclass` *value*: 18.6's is an OID that **prints** as the relation name, and
    /// pglet can carry one or the other, not both (its text encoder has no catalog to
    /// resolve an OID against). What it yields is the relation's name **rendered for an
    /// identifier position** — through [`crate::lexer::quote_ident`], so a relation
    /// created as `"Odd"` answers `"Odd"` and not the stored `Odd` — carried as `text`.
    /// `executor::expr`'s `eval_regclass` is the full statement. What the choice costs is
    /// `EDGES.md` row 85: a bare `::regclass` is declared `text` (25) where 18.6 declares
    /// `regclass` (2205), and in the **binary** format 18.6 sends the OID's four bytes
    /// where pglet sends the name's (measured over `'"Odd"'::regclass`).
    /// Appended last for postcard-discriminant stability.
    Regclass,
    /// `isfinite(date | timestamp | timestamptz) → boolean` — false for `infinity` and
    /// `-infinity`. Strict, as 18.6's overloads are. `isfinite(interval)` is not modelled
    /// (`PgInterval` has no infinity): a non-NULL `interval` argument is `42883` where 18.6
    /// answers, and so is a non-NULL `time`, which 18.6 carries to that overload through
    /// its implicit `time → interval` cast. The argument's type is checked at eval, on a
    /// non-NULL value, so a NULL argument answers NULL and a call over no rows answers
    /// nothing, whatever the type. For `interval` and `time` that is 18.6's answer too —
    /// `isfinite(NULL::time)` is NULL there — so a plan-time type gate refusing either
    /// would over-refuse. 18.6 refuses the statement before any row is read where the call
    /// resolves to no single overload: `42883` for a type with no overload and no implicit
    /// cast to one (`int` and `text` measured), and `42725` for an `unknown` argument — a
    /// bare `NULL` or an untyped literal — which more than one overload could take
    /// (`EDGES.md` row 144).
    /// Appended last for postcard-discriminant stability.
    IsFinite,
    /// `col_description(oid, int4) → text` — a column's `COMMENT ON COLUMN` text.
    ///
    /// pglet models `COMMENT ON` as a no-op (`EDGES.md` row 15), so no column has a
    /// description and the answer is **always NULL** — the truthful one for this catalog,
    /// and 18.6's own for an OID or column number nothing describes (measured:
    /// `col_description(0, 0)` and `col_description(99999999, 1)` are NULL, not errors).
    /// What it cannot answer is a comment a `COMMENT ON COLUMN` wrote: NULL here, the
    /// text there (row 15). Strict, as 18.6's is. The argument checks are
    /// [`crate::executor`]'s `eval_col_description`. Appended last for
    /// postcard-discriminant stability.
    ColDescription,
    /// `pg_advisory_xact_lock(bigint)` / `pg_advisory_xact_lock(int4, int4) → void` — a
    /// transaction-scoped advisory lock, taken as **the writer**: every key maps to the
    /// one writer a block's first write takes, so the call waits (the native server,
    /// bounded by `lock_timeout` → `55P03`) or is refused `55P03` (the embed's other
    /// logical connection) exactly where a write would, and is released where the writer
    /// is — at the block's end, or at the statement's in autocommit.
    ///
    /// Answered only as a **top-level item of a `SELECT` with no `FROM`**
    /// ([`super::Plan::is_advisory_lock_projection`], which [`super::Plan::takes_writer`]
    /// reads), the spelling kysely's migration lock issues; anywhere else a non-NULL call
    /// is `0A000`, because nothing there took the writer. Strict: a NULL argument answers
    /// NULL (18.6 takes no lock for it; here the projection still took the writer). What
    /// the one-writer mapping costs against 18.6 — two keys contend, an ordinary write
    /// contends, a `ROLLBACK TO` or an error does not release — is `EDGES.md` row 250.
    /// Appended last for postcard-discriminant stability.
    AdvisoryXactLock,
    /// `quote_ident(text) → text` — the name rendered for an identifier position by 18.6's
    /// own `quote_identifier` rule and keyword table ([`crate::lexer::pg_quote`]), not by
    /// pglet's lexer. Strict. Appended last for postcard-discriminant stability.
    QuoteIdent,
    /// `pg_get_serial_sequence(table text, column text) → text` — the sequence a column
    /// owns (a `SERIAL`'s, an identity column's, or one attached by `OWNED BY`), as
    /// `public.<name>` quoted by [`Self::QuoteIdent`]'s rule; NULL for a column that owns
    /// none. The table is read as 18.6's `textToQualifiedNameList` reads it, the column
    /// verbatim. Reads the catalog; strict. `executor::expr`'s
    /// `eval_pg_get_serial_sequence` is the full statement. Appended last for
    /// postcard-discriminant stability.
    PgGetSerialSequence,
    /// `has_schema_privilege(schema text, privilege text) → boolean` — `true` for a schema
    /// pglet has and a privilege string 18.6 parses: pglet models one role, which holds
    /// every privilege. The role-taking and `oid` overloads are `0A000`. Strict.
    /// `executor::expr`'s `eval_has_schema_privilege` is the full statement. Appended last
    /// for postcard-discriminant stability.
    HasSchemaPrivilege,
    /// `<expr>::<enum>` — the conversion to a user enum, 18.6's `enum_in`: the label
    /// passes when the enum holds it byte for byte, and is `22P02 invalid input value for
    /// enum <name>: "<label>"` when it does not.
    ///
    /// Never written as a call: the parser builds it for a cast whose target resolves to
    /// an enum ([`build_enum_cast`]), and only where the conversion cannot be decided
    /// while parsing — an operand that is not a string literal, or a literal the enum does
    /// not hold. A member literal keeps the plain `Cast` to `text` it always had. The
    /// arguments are `[operand, '<enum name>', '<label>', …]`: the name for the message
    /// and the rendering, and the member labels **as the parser read them**, so the
    /// conversion is decidable at every evaluation — a `CHECK`, an index maintained in
    /// storage, a DDL probe — none of which lends the catalog. That makes the labels a copy
    /// of `Schema.enums[name]` that outlives its statement when the node is persisted.
    /// `DROP TYPE` is the one statement that moves a member list while a persisted
    /// expression survives it (`DROP SCHEMA` and a rollback move the tables with the
    /// types), and it treats a persisted node as a dependent — `2BP01`, or dropped with
    /// the type under `CASCADE` (`executor::dependencies`) — so the list cannot move
    /// under one. An index never holds the node over an operand not already the enum:
    /// `CREATE INDEX` refuses it `42P17`, as 18.6 does.
    ///
    /// The node is always the operand of a `Cast` to `text`, so every gate that reads a
    /// cast's identity reads what it read before (`EDGES.md` row 104's erasure). A string
    /// value is tested for membership; a value of any other type is 18.6's
    /// `42846 cannot cast type <T> to <enum>` (18.6 converts to an enum only from a string
    /// type's text); NULL answers NULL. `executor::expr`'s `eval_enum_in` is the
    /// evaluation, and `planner::legality`'s `enum_in_constant_legality` the decision for
    /// a string-literal operand, taken where 18.6 converts a constant — at plan time and
    /// in the DDL walks. Appended last for postcard-discriminant stability.
    EnumIn,
    /// `md5(text | bytea) → text` — the RFC 1321 digest of the argument's bytes (a
    /// `text` value's UTF-8, a `bytea` value's own) as 32 lower-case hex digits. Strict;
    /// any other argument type is `42883`. Appended last for postcard-discriminant
    /// stability.
    Md5,
    /// `left(str, n) → text` — the first `n` characters, or all but the last `|n|` for a
    /// negative `n`. `n` is an `int4` (an `int2` widens to it; an `int8` is `42883`, as
    /// 18.6 has no `left(text, bigint)`). Appended last for postcard-discriminant
    /// stability.
    Left,
    /// `right(str, n) → text` — the last `n` characters, or all but the first `|n|` for a
    /// negative `n`; [`Self::Left`]'s argument rule. Appended last for
    /// postcard-discriminant stability.
    Right,
    /// `substr(str, start [, length])` — [`Self::Substring`]'s evaluation under its
    /// other `pg_proc` name, which 18.6 keeps as the column name and the rendering. Both
    /// take a `bytea` first argument as well as `text`, and answer its type. Its
    /// `start` and `length` are `int4`s, [`Self::Left`]'s rule. Appended last for
    /// postcard-discriminant stability.
    Substr,
    /// `octet_length(text | bytea) → int4` — the byte count: a `text` value's UTF-8
    /// length, a `bytea` value's own. Appended last for postcard-discriminant stability.
    OctetLength,
}

/// A non-strict variadic function (see [`Expr::VariadicFn`]) — each applies its own
/// NULL rule instead of propagating NULL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VariadicFunc {
    /// `concat(...) → text` — concatenate every argument's text form, **skipping**
    /// NULL arguments (all-NULL → `''`).
    Concat,
    /// `concat_ws(sep, ...) → text` — like `concat`, joining the surviving
    /// (non-NULL) values with `sep` between them. The separator (`sep`, the first
    /// argument) **is** strict — a NULL `sep` makes the whole call NULL.
    ConcatWs,
    /// `greatest(...) → common-type` — the largest non-NULL argument (all-NULL →
    /// NULL), ordered by `values_compare`.
    Greatest,
    /// `least(...) → common-type` — the smallest non-NULL argument (all-NULL → NULL).
    Least,
    /// `nullif(a, b) → type-of-a` — NULL when `a = b`, else `a`; a NULL argument
    /// returns `a` without comparing.
    NullIf,
}

/// A field name recognized by `EXTRACT` / `date_part` / `date_trunc` (see
/// [`Expr::Extract`] and [`StringFunc::DateTrunc`]).
///
/// Two-tier resolution, mirroring Postgres: [`Self::from_name`] matches the
/// (case-insensitive) field text against the **units** tier (Postgres `deltatktbl`
/// via `DecodeUnits`) and then the **special** tier (`datetktbl` via
/// `DecodeSpecial`). `EXTRACT`/`date_part` consult both tiers; `date_trunc` consults
/// only the units tier ([`Self::is_units`]), so a special-only field (`dow`/`doy`/
/// `isodow`/`isoyear`/`julian`/`epoch`) is *unrecognized* for truncation (`22023`) —
/// exactly Postgres's behavior, since `date_trunc` never calls `DecodeSpecial`. A
/// field that resolves but isn't defined for the source type fails loud (`0A000`) in
/// the per-source dispatch, not here.
///
/// Not serialized (the [`Expr::Extract`] node carries the field as a text expression,
/// resolved to this at eval time), so variant order is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractField {
    // --- units tier (Postgres `DecodeUnits` / `deltatktbl`) ---
    Microseconds,
    Milliseconds,
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
    Decade,
    Century,
    Millennium,
    Timezone,
    TimezoneHour,
    TimezoneMinute,
    // --- special tier (Postgres `DecodeSpecial` / `datetktbl`) ---
    Dow,
    Doy,
    IsoDow,
    IsoYear,
    Julian,
    Epoch,
}

impl ExtractField {
    /// Resolve a field name (case-insensitive), or `None` when neither Postgres token
    /// table recognizes it (the caller raises `22023`). Accepts the canonical spelling
    /// of each field plus the common Postgres aliases.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "microseconds" | "microsecond" | "us" | "usec" | "usecs" | "useconds" => {
                Self::Microseconds
            }
            "milliseconds" | "millisecond" | "ms" | "msec" | "msecs" | "mseconds" => {
                Self::Milliseconds
            }
            "second" | "seconds" | "sec" | "secs" | "s" => Self::Second,
            "minute" | "minutes" | "min" | "mins" | "m" => Self::Minute,
            "hour" | "hours" | "hr" | "hrs" | "h" => Self::Hour,
            "day" | "days" | "d" => Self::Day,
            "week" | "weeks" | "w" => Self::Week,
            "month" | "months" | "mon" | "mons" => Self::Month,
            "quarter" | "qtr" => Self::Quarter,
            "year" | "years" | "yr" | "yrs" | "y" => Self::Year,
            "decade" | "decades" | "dec" | "decs" => Self::Decade,
            "century" | "centuries" | "cent" | "c" => Self::Century,
            "millennium" | "millennia" | "mil" | "mils" => Self::Millennium,
            "timezone" => Self::Timezone,
            "timezone_hour" => Self::TimezoneHour,
            "timezone_minute" => Self::TimezoneMinute,
            "dow" => Self::Dow,
            "doy" => Self::Doy,
            "isodow" => Self::IsoDow,
            "isoyear" => Self::IsoYear,
            "julian" | "j" | "jd" => Self::Julian,
            "epoch" => Self::Epoch,
            _ => return None,
        })
    }

    /// Whether this field is in the **units** tier (Postgres `DecodeUnits`). Only
    /// units-tier fields are valid for `date_trunc`; a special-tier field there is
    /// unrecognized (`22023`), since `date_trunc` never consults `DecodeSpecial`.
    #[must_use]
    pub const fn is_units(self) -> bool {
        matches!(
            self,
            Self::Microseconds
                | Self::Milliseconds
                | Self::Second
                | Self::Minute
                | Self::Hour
                | Self::Day
                | Self::Week
                | Self::Month
                | Self::Quarter
                | Self::Year
                | Self::Decade
                | Self::Century
                | Self::Millennium
                | Self::Timezone
                | Self::TimezoneHour
                | Self::TimezoneMinute
        )
    }
}

/// Window function types.
///
/// These compute a value for each row based on a "window" of related rows,
/// defined by PARTITION BY and ORDER BY clauses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowFunc {
    /// `ROW_NUMBER()` - sequential number within partition (1, 2, 3, ...).
    RowNumber,
    /// `RANK()` - rank with gaps for ties (1, 2, 2, 4, ...).
    Rank,
    /// `DENSE_RANK()` - rank without gaps for ties (1, 2, 2, 3, ...).
    DenseRank,
}

/// Literal value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// A decimal or exponent literal (or an integer too wide for `i64`) — `NUMERIC`
    /// in Postgres. Parsed once from its source text so the display scale (trailing
    /// zeros: `1.50` → scale 2) survives, which an `f64` would lose.
    Numeric(crate::types::PgNumeric),
}

/// Binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
    /// String concatenation `||`. Renders both operands to their Postgres text
    /// form and concatenates; a NULL operand yields NULL (so `a || NULL` is NULL,
    /// matching Postgres without `concat()`'s NULL-skipping).
    Concat,
    /// LIKE pattern matching (% = any, _ = single char).
    Like,
    /// NOT LIKE pattern matching.
    NotLike,
    /// JSON(B) field access by key: -> returns JSONB.
    JsonGet,
    /// JSON(B) field access by key: ->> returns TEXT.
    JsonGetText,
    /// Containment `@>`: left contains right. Defined for arrays of the same
    /// element type (every element of right is in left) and for JSONB (Postgres's
    /// full `JsonbDeepContains` — object key/value, array subset, nested
    /// containers, and the top-level "an array may contain a primitive" rule). Any
    /// other operand pair resolves to no operator (`42883`).
    Containment,
    /// Containment `<@`: left is contained by right (the mirror of `@>`).
    ContainedBy,
    /// `starts_with(left, right)`: case-sensitive literal prefix match. Unlike
    /// `LIKE`, the right operand is matched verbatim (`%` / `_` are not
    /// wildcards).
    StartsWith,
    /// `left AT TIME ZONE right`: shift a timestamp into a named zone. pglet
    /// stores `Value::Timestamp` as UTC and has no distinct timestamptz, so only
    /// `'UTC'` is supported (identity for rendering); any other zone fails loud.
    AtTimeZone,
    /// `to_char(left, right)`: format the `left` timestamp through the Postgres
    /// `to_char` format string `right`. Scoped to the spine's ISO-8601 tokens
    /// (see `format_timestamp_to_char`); unknown tokens fail loud.
    ToChar,
    /// `left IS DISTINCT FROM right`: null-safe inequality. TRUE when exactly one
    /// operand is NULL or both are non-NULL and unequal; FALSE when both NULL or
    /// equal. Unlike `Ne`, this **never returns NULL** — that is its whole point.
    IsDistinctFrom,
    /// `left IS NOT DISTINCT FROM right`: null-safe equality (the negation of
    /// `IsDistinctFrom`). TRUE when both NULL or both non-NULL and equal; FALSE
    /// when exactly one is NULL or they are unequal. Unlike `Eq`, this **never
    /// returns NULL**.
    IsNotDistinctFrom,
    // Appended after the older variants for postcard-discriminant stability
    // (CHECK exprs persist this enum through the catalog).
    /// ILIKE pattern matching — `Like` with both operands case-folded through
    /// the same Unicode lowercasing `lower()` uses.
    Ilike,
    /// NOT ILIKE pattern matching.
    NotIlike,
    /// Modulo `%` — integer (`int2/4/8`, sign of the dividend) or `numeric`, with
    /// mixed integer widths promoting to the wider and an `int`/`numeric` mix casting
    /// to `numeric`. There is **no** float `%`: a float operand fails loud (`42883`).
    /// `x % 0` is `22012`. Appended after `NotIlike` for postcard-discriminant
    /// stability (CHECK exprs persist this enum through the catalog).
    Mod,
    /// Exponentiation `^` — `float8` (`f64::powf`) or `numeric` (integer exponent
    /// only). Result type follows Postgres resolution: `float8` wins over `numeric`
    /// and `int ^ int` is `float8` (no integer operator exists). Domain errors are
    /// `2201F` (`0 ^ negative`, negative base with a non-integer exponent); a
    /// `float8` overflow is `22003`; a positive-base fractional `numeric` exponent
    /// (the general ln/exp lift) is deferred (`0A000`). Appended after `Mod` for
    /// postcard-discriminant stability.
    Pow,
    /// JSONB key/element existence `?` (`jsonb ? text → bool`). True when the right
    /// text is a top-level object key, a `String` array element, or — Postgres's
    /// scalar quirk — a top-level scalar `String` equal to it. Appended after `Pow`
    /// for postcard-discriminant stability (CHECK exprs persist this enum).
    JsonExists,
    /// JSONB any-key existence `?|` (`jsonb ?| text[] → bool`). True when **any**
    /// non-NULL element of the array satisfies [`Self::JsonExists`]; a NULL array
    /// element is skipped, and an empty array is `false`. Appended after
    /// `JsonExists` for postcard-discriminant stability.
    JsonExistsAny,
    /// JSONB all-keys existence `?&` (`jsonb ?& text[] → bool`). True when **every**
    /// non-NULL element of the array satisfies [`Self::JsonExists`]; a NULL array
    /// element is skipped, and an empty array is `true`. Appended after
    /// `JsonExistsAny` for postcard-discriminant stability.
    JsonExistsAll,
    /// JSONB path extraction `#>` (`jsonb #> text[] → jsonb`). Walks the document
    /// down the path (object keys / array indices, a negative index counting from
    /// the end); any NULL path element or an empty path is handled per Postgres (see
    /// `executor::json_path_get`). Appended after `JsonExistsAll` for
    /// postcard-discriminant stability.
    JsonPathGet,
    /// JSONB path extraction as text `#>>` (`jsonb #>> text[] → text`). Like
    /// [`Self::JsonPathGet`] but the final value renders via the `->>` text rule (a
    /// JSON `null` target → SQL NULL). Appended after `JsonPathGet` for
    /// postcard-discriminant stability.
    JsonPathGetText,
    /// The **root** node of a long `AND` list built balanced — evaluated, typed and
    /// gated exactly as [`Self::And`] (every reader asks [`Self::connective`]), and
    /// distinguished only so the readers of a list's operands can tell where a list
    /// begins (`Expr::bool_node_operands`). Built only by [`Expr::bool_list`]; see it
    /// for why a long list is balanced. Appended for postcard-discriminant stability.
    AndList,
    /// [`Self::AndList`]'s `OR` twin.
    OrList,
    /// A node **below** a long `AND` list's [`Self::AndList`] root: its operands are the
    /// list's own, where an `AndList` below the root is an operand that is a list of
    /// its own.
    AndListInner,
    /// [`Self::AndListInner`]'s `OR` twin.
    OrListInner,
    /// POSIX regular-expression match `~` (`text ~ text → bool`), **narrowed**: a pattern
    /// of `^` followed by characters none of which is a regex metacharacter matches as
    /// 18.6 does — a prefix test — and every other pattern is refused `0A000`, decided
    /// on the pattern's value at evaluation (`executor::expr`'s `eval_regex`; `EDGES.md`
    /// row 251). Appended for postcard-discriminant stability.
    RegexMatch,
    /// [`Self::RegexMatch`]'s negation `!~`, narrowed the same way.
    NotRegexMatch,
    /// `mod(a, b)` — the `%` operator spelled as a call. 18.6 registers `mod` over the
    /// same four signatures as `%` (`int2`, `int4`, `int8`, `numeric`) with the same
    /// body, so it evaluates, types and is gated as [`Self::Mod`] (every reader asks
    /// [`Self::operator_form`]); it differs only in its output column name and its
    /// rendering. Appended for postcard-discriminant stability.
    ModFn,
    /// `power(a, b)` — the `^` operator spelled as a call (`float8` and `numeric`
    /// signatures, `dpow` / `numeric_power` as `^`'s), read as [`Self::Pow`] through
    /// [`Self::operator_form`]. Appended for postcard-discriminant stability.
    PowerFn,
    /// `pow(a, b)` — [`Self::PowerFn`] under its other `pg_proc` name, which 18.6 keeps
    /// as the column name and the rendering. Appended for postcard-discriminant
    /// stability.
    PowFn,
}

impl BinaryOp {
    /// The operator this one evaluates as: [`Self::Mod`] for `mod(…)`, [`Self::Pow`]
    /// for `power(…)` and `pow(…)`, itself otherwise. Every reader that decides what an
    /// arithmetic operator *does* — its value, its type, its operand gate — asks this;
    /// only the output column name and the rendering see the call spelling.
    #[must_use]
    pub const fn operator_form(self) -> Self {
        match self {
            Self::ModFn => Self::Mod,
            Self::PowerFn | Self::PowFn => Self::Pow,
            other => other,
        }
    }

    /// The function name a call-spelled operator was written with — `mod`, `power` or
    /// `pow` — and `None` for every operator spelled as one.
    #[must_use]
    pub const fn call_name(self) -> Option<&'static str> {
        match self {
            Self::ModFn => Some("mod"),
            Self::PowerFn => Some("power"),
            Self::PowFn => Some("pow"),
            _ => None,
        }
    }

    /// The connective this operator evaluates as: [`Self::And`] for `AndList`,
    /// [`Self::Or`] for `OrList`, itself otherwise. Every reader that decides what an
    /// operator *does* asks this; only the list builders and readers see the
    /// difference.
    #[must_use]
    pub const fn connective(self) -> Self {
        match self {
            Self::AndList | Self::AndListInner => Self::And,
            Self::OrList | Self::OrListInner => Self::Or,
            other => other,
        }
    }

    /// The boolean context each operand of this operator is in: `AND`/`OR` are the two
    /// operators whose operands are boolean contexts; every other one is judged by its
    /// own operator rule.
    #[must_use]
    pub const fn operand_context(self) -> Option<BoolContext> {
        match self.connective() {
            Self::And => Some(BoolContext::And),
            Self::Or => Some(BoolContext::Or),
            _ => None,
        }
    }

    /// The root form of a long `AND`/`OR` list ([`Self::AndList`] / [`Self::OrList`])
    /// for `And`/`Or`, itself for any other operator.
    #[must_use]
    pub const fn list_form(self) -> Self {
        match self {
            Self::And => Self::AndList,
            Self::Or => Self::OrList,
            other => other,
        }
    }

    /// The form of a node below a long list's root ([`Self::AndListInner`] /
    /// [`Self::OrListInner`]) for `And`/`Or`, itself for any other operator.
    #[must_use]
    pub const fn list_inner_form(self) -> Self {
        match self {
            Self::And => Self::AndListInner,
            Self::Or => Self::OrListInner,
            other => other,
        }
    }

    /// Whether this is a node below a long list's root.
    #[must_use]
    pub const fn is_list_inner(self) -> bool {
        matches!(self, Self::AndListInner | Self::OrListInner)
    }
}

/// Result type of additive temporal arithmetic (`timestamp`/`interval`/`date`).
///
/// Returns `None` when the operands aren't a temporal combination (the caller
/// falls back to the numeric rules). `timestamp ± interval → timestamp` (either
/// operand order for `+`, since `interval + timestamp` is commutative;
/// `interval - timestamp` is undefined), and `timestamp - timestamp → interval`;
/// the result keeps the timestamp operand's exact type (`timestamp` vs
/// `timestamptz`). Date arithmetic (Postgres `pg_operator.dat`):
/// `date ± int → date`, `date - date → int4`, and `date ± interval → timestamp`
/// (never `timestamptz` — a bare `date` carries no zone). The `date ± int` case
/// would already fall back to the (left) `date` type, but is listed explicitly so
/// the funnel is the single source of truth.
// Per-type arms are kept separate for the documentation table; several share a
// `Some(Interval)` / `Some(Timestamp)` body (e.g. `interval - interval` and `time
// - time` both yield `interval`).
#[allow(clippy::match_same_arms)]
pub fn temporal_arith_type(op: BinaryOp, left: TypeOid, right: TypeOid) -> Option<TypeOid> {
    let is_ts = |t| matches!(t, TypeOid::Timestamp | TypeOid::Timestamptz);
    let is_int = |t| matches!(t, TypeOid::Int2 | TypeOid::Int4);
    // A numeric multiplier/divisor for `interval × ÷ number` (the int/numeric
    // operand casts to `float8` at eval time).
    let is_num = |t| {
        matches!(
            t,
            TypeOid::Int2
                | TypeOid::Int4
                | TypeOid::Int8
                | TypeOid::Float4
                | TypeOid::Float8
                | TypeOid::Numeric
        )
    };
    match op {
        // `timestamp + interval` / commutative `interval + timestamp`; `date +
        // interval` / `interval + date → timestamp`; `date ± int → date`;
        // `time + interval` / commutative `interval + time → time`; `date + time` /
        // commutative `time + date → timestamp`; `interval + interval → interval`.
        BinaryOp::Add => match (left, right) {
            (t, TypeOid::Interval) | (TypeOid::Interval, t) if is_ts(t) => Some(t),
            (TypeOid::Interval, TypeOid::Interval) => Some(TypeOid::Interval),
            (TypeOid::Date, TypeOid::Interval) | (TypeOid::Interval, TypeOid::Date) => {
                Some(TypeOid::Timestamp)
            }
            (TypeOid::Date, t) | (t, TypeOid::Date) if is_int(t) => Some(TypeOid::Date),
            (TypeOid::Time, TypeOid::Interval) | (TypeOid::Interval, TypeOid::Time) => {
                Some(TypeOid::Time)
            }
            (TypeOid::Date, TypeOid::Time) | (TypeOid::Time, TypeOid::Date) => {
                Some(TypeOid::Timestamp)
            }
            _ => None,
        },
        // `timestamp - interval → timestamp`; `timestamp - timestamp → interval`;
        // `date - interval → timestamp`; `date - date → int4`; `date - int → date`;
        // `time - interval → time`; `time - time → interval`;
        // `interval - interval → interval`.
        BinaryOp::Sub => match (left, right) {
            (t, TypeOid::Interval) if is_ts(t) => Some(t),
            (l, r) if is_ts(l) && is_ts(r) => Some(TypeOid::Interval),
            (TypeOid::Interval, TypeOid::Interval) => Some(TypeOid::Interval),
            (TypeOid::Date, TypeOid::Interval) => Some(TypeOid::Timestamp),
            (TypeOid::Date, TypeOid::Date) => Some(TypeOid::Int4),
            (TypeOid::Date, t) if is_int(t) => Some(TypeOid::Date),
            (TypeOid::Time, TypeOid::Interval) => Some(TypeOid::Time),
            (TypeOid::Time, TypeOid::Time) => Some(TypeOid::Interval),
            _ => None,
        },
        // `interval × number` (commutative) and `interval ÷ number → interval`.
        BinaryOp::Mul => match (left, right) {
            (TypeOid::Interval, t) | (t, TypeOid::Interval) if is_num(t) => Some(TypeOid::Interval),
            _ => None,
        },
        BinaryOp::Div => match (left, right) {
            (TypeOid::Interval, t) if is_num(t) => Some(TypeOid::Interval),
            _ => None,
        },
        _ => None,
    }
}

/// Result type of arithmetic within the numeric family (Postgres promotion).
///
/// Returns `None` when neither operand is a float or `NUMERIC` — the **integer**
/// pairs it declines are answered by [`int_width_result`], and both are sequenced by
/// [`arith_result_type`], which is what callers should use. A `float8` operand wins; a
/// lone `float4` mixed with `numeric`/int promotes to `float8` (only `real ± real →
/// real`); otherwise a `NUMERIC` operand makes the result `NUMERIC`
/// (`int + numeric → numeric`). Applied after [`temporal_arith_type`], which takes
/// precedence.
///
/// Kept separate from [`int_width_result`] because `select_common_type`
/// (`planner::common_type`) consumes *this* rule alone — `COALESCE`/`CASE` resolve
/// their branches by a deliberately different rule from arithmetic (see `docs/compatibility.md`'s
/// `COALESCE` / `CASE` common types row), so widening this function to cover integers
/// would silently move that one too.
#[must_use]
pub fn numeric_family_result(l: TypeOid, r: TypeOid) -> Option<TypeOid> {
    let is_float = |t| matches!(t, TypeOid::Float4 | TypeOid::Float8);
    if l == TypeOid::Float8 || r == TypeOid::Float8 {
        Some(TypeOid::Float8)
    } else if is_float(l) || is_float(r) {
        // exactly one is float4 (float8 handled above)
        if l == TypeOid::Float4 && r == TypeOid::Float4 {
            Some(TypeOid::Float4)
        } else {
            Some(TypeOid::Float8)
        }
    } else if l == TypeOid::Numeric || r == TypeOid::Numeric {
        Some(TypeOid::Numeric)
    } else {
        None
    }
}

/// This integer type's arithmetic width rank, or `None` if it is not an integer.
const fn int_width_rank(t: TypeOid) -> Option<u8> {
    match t {
        TypeOid::Int2 => Some(0),
        TypeOid::Int4 => Some(1),
        TypeOid::Int8 => Some(2),
        _ => None,
    }
}

/// Result type of arithmetic over two **integers**: the wider operand wins, in either
/// order.
///
/// Postgres registers a distinct operator per ordered pair — `int24pl(int2,int4)` and
/// `int42pl(int4,int2)` both `prorettype => int4`, `int48*`/`int84*` both `int8`,
/// `int28*`/`int82*` both `int8` — so the result is the wider type regardless of which
/// side it is on. `%` has no mixed-width operator at all and resolves through the
/// implicit int2→int4→int8 casts, landing on the same answer.
///
/// The rule is not cosmetic. pglet's *value* path already widens (an `int2 + int4` of
/// `60000` is computed and sent as four bytes), so declaring the **left** operand's
/// type — which is what both walkers did before this function existed — declared a
/// two-byte type over a four-byte value, and `int4 + int8` a four-byte type over an
/// eight-byte one. That is the same undecodable-to-a-binary-client shape already
/// recorded for `float4` arithmetic, `sum(real)`, and `COALESCE(int4, int8)`.
#[must_use]
pub const fn int_width_result(l: TypeOid, r: TypeOid) -> Option<TypeOid> {
    match (int_width_rank(l), int_width_rank(r)) {
        (Some(lr), Some(rr)) => Some(if lr >= rr { l } else { r }),
        _ => None,
    }
}

/// Result type of `+ - * / %` over two **known** operand types.
///
/// The one place the arithmetic type chain lives: temporal first
/// ([`temporal_arith_type`] — `date - date`, `timestamp ± interval`, …), then the
/// float/`NUMERIC` promotion ([`numeric_family_result`]), then the integer width
/// ([`int_width_result`]). `None` means the pair is outside those families, and the
/// caller supplies its own unknown-operand fallback — the two walkers spell that
/// differently because they carry unknown-ness differently (the executor's
/// `expr_type_oid` is total and defaults an un-inferable operand to `text`, while the
/// planner's `Expr::infer_type` returns `Option`), and that is the *only* difference
/// they are entitled to.
///
/// Both walkers must call this rather than re-sequencing the chain: they previously
/// open-coded it, ending with two *different* wrong fallbacks that agreed only by
/// accident, and the missing integer step was invisible in both.
#[must_use]
pub fn arith_result_type(op: BinaryOp, l: TypeOid, r: TypeOid) -> Option<TypeOid> {
    temporal_arith_type(op, l, r)
        .or_else(|| numeric_family_result(l, r))
        .or_else(|| int_width_result(l, r))
}

/// Result type of `^`, given the numeric-family promotion of its operands.
///
/// Postgres registers exactly two exponentiation operators — `^(float8, float8)` and
/// `^(numeric, numeric)` — so the only way to get anything but `float8` is for both
/// operands to resolve to `numeric`. In particular `real ^ real` is **`double
/// precision`**: the arithmetic promotion says `real`, but there is no `real`
/// exponentiation for it to resolve to, and taking the promotion at face value
/// declared a 4-byte type over the 8-byte value `eval_power` returns.
#[must_use]
pub const fn pow_result_type(promoted: Option<TypeOid>) -> TypeOid {
    match promoted {
        Some(TypeOid::Numeric) => TypeOid::Numeric,
        _ => TypeOid::Float8,
    }
}

/// Result type of a numeric math function given its first argument's type and whether
/// a second (scale) argument is present.
///
/// `abs` preserves the argument's type. For the one-argument `ceil`/`floor`/`round`/
/// `trunc`, a `numeric` argument yields `numeric` while **int and float both yield
/// `float8`** — Postgres has only `(double precision)` and `(numeric)` overloads, and
/// an integer resolves to the `double precision` one (so `ceil(5)` is `double
/// precision`, `round(9)` is `double precision`, and an untyped `ceil(NULL)` is
/// `double precision`), *not* numeric. The **two-argument** `round(x, s)` / `trunc(x,
/// s)` have only a `(numeric, integer)` overload, so they always yield `numeric`
/// (`has_scale` selects this). `sqrt`/`sign` likewise have only `(float8)`/`(numeric)`
/// overloads: a `numeric` argument yields `numeric` and everything else (int **and**
/// float) yields `float8`. Shared by the planner's `infer_type` and the executor's
/// `expr_type_oid`; the executor's `Expr::MathFn` value must match this OID (an integer
/// input to a one-arg `ceil`/`floor`/`round`/`trunc` produces a `Value::Float8`).
///
/// **`None` means Postgres's `unknown`, and it resolves to `float8` for every member —
/// including `abs`, the one whose result rule otherwise passes its argument's type
/// through.** An `unknown` input picks the preferred type of the winning category
/// (`func_select_candidate`), and `typeconv.sgml` uses absolute value as its worked
/// example for exactly this: "one of these entries is for type float8, which is the
/// preferred type in the numeric category. Therefore PostgreSQL will use that entry when
/// faced with an unknown input." Measured on 18.6: `SELECT abs(NULL)` declares 701, as do
/// `ceil`/`floor`/`round`. A `numeric` default here was a *third* answer beside the two
/// walkers' — 1700 on `Describe` against 25 on the simple query — for one statement whose
/// value is NULL either way, so neither wrong OID is visible to a client except as the
/// OID itself. Both halves moved together (`types/math_fn_unknown_arg_declared_types.sql`);
/// this one alone leaves the simple-query leg on its own `Text` fallback.
///
/// The caller therefore owes this function a real `None` where the argument is untyped —
/// see `executor::helpers`'s `Expr::MathFn` arm, which pays it for a **bare `NULL`
/// literal** and nothing else. That is narrower than "an untyped constant": the arm routes
/// through `Literal::infer_type`, and `Literal::Null` is the only literal that answers
/// `None` there, so a bare string literal — Postgres's `unknown` too — keeps its `text`
/// and `abs('5')` is pglet's `0A000` against 18.6's `701` answering `5` (`docs/compatibility.md`'s
/// NUMERIC row).
///
/// **The general question of what an un-inferable operand should be stays open.** The
/// executor walk is total and answers `Text` where the other three `Expr` type walks
/// (`Expr::infer_type`, `certain_expr_type_oid`, `concrete_branch_type`) answer `None`,
/// and every shared rule that passes an operand's type through can tell the difference.
/// So this closes the `MathFn` cell **for a directly-`NULL` argument**, not for the cell:
/// an argument that is only *transitively* untyped still splits the two walkers, measured
/// on 18.6 — `abs(-NULL)`, `abs(CASE WHEN true THEN NULL END)` and
/// `abs(COALESCE(NULL, NULL))` each declare 25 on the simple query against `Describe`'s
/// 701, where 18.6 refuses all three (`42725 operator is not unique: - unknown`, then
/// `42883 function abs(text) does not exist` twice). Recorded in `EDGES.md`. The other rules are
/// unenumerated here on purpose, because a count
/// of them would go stale silently.
#[must_use]
pub fn math_fn_result_type(func: MathFunc, arg_type: Option<TypeOid>, has_scale: bool) -> TypeOid {
    match func {
        MathFunc::Abs => arg_type.unwrap_or(TypeOid::Float8),
        // Two-arg round/trunc(x, s) has only a numeric overload in Postgres.
        MathFunc::Round | MathFunc::Trunc if has_scale => TypeOid::Numeric,
        // One-arg ceil/floor/round/trunc and sqrt/sign share the same overload set: a
        // numeric argument yields numeric, while int and float both resolve to the
        // double-precision overload (so `ceil(5)` and `sqrt(5)` are each `float8`).
        _ => match arg_type {
            Some(TypeOid::Numeric) => TypeOid::Numeric,
            _ => TypeOid::Float8,
        },
    }
}

/// Result type of an aggregate ([`Expr::Aggregate`]) given its argument's type
/// (`None` when the argument type couldn't be resolved) and whether that type is a
/// **certain** integer ([`ArgCertainty`]).
///
/// 18.6's `pg_aggregate` rows: `count` is `int8`; `avg` over an integer or a
/// `numeric` is `numeric` (`int8_avg`, `numeric_avg`), over `interval` is `interval`
/// (`interval_avg`), and `float8` otherwise; `sum` widens `smallint`/`integer` to
/// `int8` and `bigint` to `numeric`, and passes every other argument type through, as
/// do `min`/`max`.
///
/// **An integer argument takes the `numeric` rows only where its type is certain.** A
/// guessed integer type — a `COALESCE` the coercion pass could not unify, declared by
/// its first branch over another branch's `float8` values, read directly or through a
/// derived table's column — would turn `avg` exact over values that are not integers.
/// There `avg` stays `float8` and `sum(bigint)` `int8`, the answers an argument of
/// unknown type gets, and the ones 18.6 gives where the guess hid a `float8`.
///
/// Readers do not call this with a certainty of their own: they call
/// [`aggregate_type_of`], which reads the one the coercion pass recorded on the node.
#[must_use]
pub const fn aggregate_result_type(
    func: AggregateFunc,
    arg_type: Option<TypeOid>,
    arg_certain: bool,
) -> Option<TypeOid> {
    match func {
        AggregateFunc::Count => Some(TypeOid::Int8),
        AggregateFunc::Avg => Some(match arg_type {
            Some(TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8) if arg_certain => TypeOid::Numeric,
            Some(TypeOid::Numeric) => TypeOid::Numeric,
            Some(TypeOid::Interval) => TypeOid::Interval,
            _ => TypeOid::Float8,
        }),
        AggregateFunc::Sum => match arg_type {
            Some(TypeOid::Int8) if arg_certain => Some(TypeOid::Numeric),
            Some(TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8) => Some(TypeOid::Int8),
            other => other,
        },
        AggregateFunc::Min | AggregateFunc::Max => arg_type,
    }
}

/// The result type of `func` over an argument whose recorded certainty is `certainty`
/// and whose type the caller's own walk reads as `walked`.
///
/// The one call every reader of an aggregate's result type makes: the planner's
/// `infer_type` (the extended protocol's `Describe`), the executor's `expr_type_oid` (the
/// simple protocol's `RowDescription`), the certain-type gate `certain_expr_type_oid`, the
/// coercion pass's `concrete_branch_type`, and `compute_aggregate`, whose exact
/// `sum`/`avg` accumulators are chosen by the type this answers, so the value they produce
/// is the type the descriptor advertises.
///
/// A certain integer argument is read at the type the coercion pass recorded, so no
/// reader's walk can move the answer; every other argument at `walked`, under the
/// uncertain rule.
#[must_use]
pub const fn aggregate_type_of(
    func: AggregateFunc,
    certainty: ArgCertainty,
    walked: Option<TypeOid>,
) -> Option<TypeOid> {
    debug_assert!(
        !matches!(certainty, ArgCertainty::Undecided),
        "an aggregate's result type read before the coercion pass decided its argument"
    );
    match (func, certainty) {
        (AggregateFunc::Avg | AggregateFunc::Sum, ArgCertainty::CertainInteger(t)) => {
            aggregate_result_type(func, Some(t), true)
        }
        _ => aggregate_result_type(func, walked, false),
    }
}

/// Result type of a niladic builtin ([`Expr::Builtin`]).
///
/// One mapping read by **both** type walkers — the planner's [`Expr::infer_type`] and
/// the executor's `expr_type_oid` — the same shape [`string_fn_result_type`] gives the
/// [`StringFunc`] members. It was two hand-duplicated matches over all nineteen
/// variants; they agreed, which is the failure mode that hides — `docs/declared_types.md`
/// § *The seam: shared rules and open-coded ones* states it, that duplicated rules do not
/// only drift apart, they also agree on being wrong.
#[must_use]
// arms grouped by result type for documentation; several groups share a body
#[allow(clippy::match_same_arms)]
pub const fn builtin_fn_result_type(func: BuiltinFn) -> TypeOid {
    match func {
        BuiltinFn::Now
        | BuiltinFn::TransactionTimestamp
        | BuiltinFn::StatementTimestamp
        | BuiltinFn::ClockTimestamp
        | BuiltinFn::CurrentTimestamp => TypeOid::Timestamptz,
        BuiltinFn::RandomUuid => TypeOid::Uuid,
        BuiltinFn::CurrentDate => TypeOid::Date,
        // `CURRENT_TIME` is `timetz` on 18.6; pglet is UTC-only and returns a plain
        // `time` (a recorded divergence), which `LOCALTIME` shares.
        BuiltinFn::CurrentTime | BuiltinFn::LocalTime => TypeOid::Time,
        // `LOCALTIMESTAMP` — the `Now` value typed without a time zone.
        BuiltinFn::LocalTimestamp => TypeOid::Timestamp,
        // The session-information forms — `name` (19) on 18.6, measured on both legs.
        BuiltinFn::CurrentUser
        | BuiltinFn::SessionUser
        | BuiltinFn::User
        | BuiltinFn::CurrentRole
        | BuiltinFn::CurrentCatalog
        | BuiltinFn::CurrentDatabase
        | BuiltinFn::CurrentSchema
        | BuiltinFn::CurrentSchemaFn => TypeOid::Name,
        BuiltinFn::Version => TypeOid::Text,
        BuiltinFn::Random => TypeOid::Float8,
    }
}

/// Result type of a strict-NULL string function ([`Expr::StringFn`]).
///
/// `length`/`strpos`/`octet_length` yield `int4`, `current_schemas` yields `name[]`,
/// and every other member yields `text`. Depends only on the function, so both the
/// planner's `infer_type` and the executor's `expr_type_oid` share this one mapping —
/// save the members whose result follows an argument (`date_trunc`, and `substring` /
/// `substr` over a `bytea`), which both walks answer through [`shared_leaf_cell_type`]
/// instead.
#[must_use]
// arms kept per member for documentation; `current_setting` shares `text` with the
// string members and is spelled separately because its reason is unrelated to theirs
#[allow(clippy::match_same_arms)]
pub const fn string_fn_result_type(func: StringFunc) -> TypeOid {
    match func {
        StringFunc::Length
        | StringFunc::Strpos
        | StringFunc::Position
        | StringFunc::OctetLength => TypeOid::Int4,
        StringFunc::Upper
        | StringFunc::Replace
        | StringFunc::Substring
        | StringFunc::Ltrim
        | StringFunc::Rtrim
        | StringFunc::Btrim
        | StringFunc::Lpad
        | StringFunc::Rpad
        | StringFunc::SplitPart => TypeOid::Text,
        // `date_trunc`'s result type is the *source* type, not a fixed one; the
        // type-walkers special-case it via `date_trunc_result_type` (with the source
        // arg's type). This fallback covers a caller that ignores the source (none do).
        StringFunc::DateTrunc => TypeOid::Timestamp,
        // A GUC's value is text on 18.6 whatever the setting is, `missing_ok` or not.
        StringFunc::CurrentSetting => TypeOid::Text,
        // The one array-returning member: the search path is a list of identifiers.
        StringFunc::CurrentSchemas => TypeOid::NameArray,
        StringFunc::PgGetConstraintdef | StringFunc::Regclass => TypeOid::Text,
        StringFunc::IsFinite => TypeOid::Bool,
        StringFunc::ColDescription => TypeOid::Text,
        StringFunc::AdvisoryXactLock => TypeOid::Void,
        StringFunc::QuoteIdent | StringFunc::PgGetSerialSequence => TypeOid::Text,
        StringFunc::HasSchemaPrivilege => TypeOid::Bool,
        // The label, as `text` — an enum value's `TypeOid` here (`docs/enums.md`).
        StringFunc::EnumIn => TypeOid::Text,
        StringFunc::Md5 | StringFunc::Left | StringFunc::Right => TypeOid::Text,
        // The `text` overload's type; a `bytea` argument's is `substring_result_type`'s,
        // which the type walkers read through `shared_leaf_cell_type`.
        StringFunc::Substr => TypeOid::Text,
    }
}

/// The node a cast to the enum `enum_name` parses to, over the member list `labels`.
///
/// Where the parser can decide the conversion it does, and builds what it always built:
/// a string literal the enum holds, and a `NULL`, are a plain `Cast` to `text` — an
/// enum value is its label (`docs/enums.md`). Everything else is the conversion
/// [`StringFunc::EnumIn`] carries, under that same `Cast`: an operand whose value is not
/// known yet (a column, a parameter, `'q'::text`), decided at evaluation or at `Bind`, and
/// a literal the enum does not hold, decided where 18.6 converts a constant
/// ([`crate::planner::enum_in_constant_legality`]). The parser raises nothing here, so a
/// refusal takes the statement position of every other constant cast's rather than a
/// parse-time one ahead of it.
#[must_use]
pub fn build_enum_cast(inner: Expr, enum_name: &str, labels: &[String]) -> Expr {
    let decided = match &inner {
        Expr::Literal(Literal::Null) => true,
        Expr::Literal(Literal::String(label)) => labels.iter().any(|l| l == label),
        _ => false,
    };
    let converted = if decided {
        inner
    } else {
        let mut args = Vec::with_capacity(labels.len() + 2);
        args.push(inner);
        args.push(Expr::Literal(Literal::String(enum_name.to_string())));
        args.extend(
            labels
                .iter()
                .map(|l| Expr::Literal(Literal::String(l.clone()))),
        );
        Expr::StringFn {
            func: StringFunc::EnumIn,
            args,
        }
    };
    Expr::Cast {
        expr: Box::new(converted),
        target_type: TypeOid::Text,
    }
}

/// A [`StringFunc::EnumIn`] node's arguments read back: the operand, the enum's name,
/// and its member labels. `None` for any other node, or for an argument list the parser
/// never builds.
#[must_use]
pub fn enum_in_parts(expr: &Expr) -> Option<(&Expr, &str, &[Expr])> {
    let Expr::StringFn {
        func: StringFunc::EnumIn,
        args,
    } = expr
    else {
        return None;
    };
    enum_in_arg_parts(args)
}

/// [`enum_in_parts`] over the argument list alone, for a caller already inside the
/// node's `StringFn` arm.
#[must_use]
pub const fn enum_in_arg_parts(args: &[Expr]) -> Option<(&Expr, &str, &[Expr])> {
    match args {
        [operand, Expr::Literal(Literal::String(name)), labels @ ..] => {
            Some((operand, name.as_str(), labels))
        }
        _ => None,
    }
}

/// Whether the member list of an [`enum_in_parts`] node holds `label` — exact byte
/// equality, a label being a string literal that nothing folds (`docs/enums.md` § An
/// enum's labels are a set).
#[must_use]
pub fn enum_in_holds(labels: &[Expr], label: &str) -> bool {
    labels
        .iter()
        .any(|l| matches!(l, Expr::Literal(Literal::String(m)) if m == label))
}

/// Result type of `substring(x, …)` / `substr(x, …)` ([`StringFunc::Substring`],
/// [`StringFunc::Substr`]): `bytea` for a `bytea` `x`, and `text` for every other —
/// an argument of unknown type included, which 18.6 resolves to the `text` overload
/// (`substr(NULL, 1)` is `text` there, measured).
#[must_use]
pub const fn substring_result_type(source_type: Option<TypeOid>) -> TypeOid {
    match source_type {
        Some(TypeOid::Bytea) => TypeOid::Bytea,
        _ => TypeOid::Text,
    }
}

/// Result type of `date_trunc(field, source)` ([`StringFunc::DateTrunc`]): the
/// **source** argument's type.
///
/// `timestamptz` → `timestamptz`, `interval` → `interval`, `date` → **`timestamptz`**
/// (Postgres resolves `date_trunc(text, date)` via the `date → timestamptz` cast, not
/// `timestamp` — so the result renders with the UTC `+00` offset), everything else
/// (`timestamp` or an unknown source) → `timestamp`. Load-bearing: the returned value
/// is a `Value::Timestamp` / `Value::Interval`, and the text encoder is
/// column-type-aware, so a `timestamptz` result must be declared as such or the offset
/// is dropped. Shared by the planner's `infer_type` and the executor's `expr_type_oid`.
#[must_use]
pub const fn date_trunc_result_type(source_type: Option<TypeOid>) -> TypeOid {
    match source_type {
        Some(TypeOid::Timestamptz | TypeOid::Date) => TypeOid::Timestamptz,
        Some(TypeOid::Interval) => TypeOid::Interval,
        _ => TypeOid::Timestamp,
    }
}

/// Result type of a non-strict variadic function ([`Expr::VariadicFn`]) given each
/// argument's inferable type (`None` for an unresolvable/NULL arg).
///
/// `concat`/`concat_ws` are always `text`. `greatest`/`least` return the **common
/// type** of their arguments ([`variadic_common_type_oid`]) — so `greatest(int4, int8)` is
/// `int8` — falling back to the first known type (then `text`) when no common type
/// resolves (the loud-at-eval case).
///
/// **`nullif` does not.** It follows the operator-resolution rulebook
/// ([`nullif_result_type`]): the result is argument one's type as the `=` resolution
/// left it, so `nullif(int4_col, int8_col)` is `int4` where `greatest` of the same pair
/// is `int8`. Sharing the common-type rule with its IR neighbours is what made pglet
/// declare the widened type.
///
/// Shared by the planner's `infer_type` and the executor's `expr_type_oid`, so the
/// declared type can't drift from the returned value's type.
#[must_use]
pub fn variadic_fn_result_type(func: VariadicFunc, arg_types: &[Option<TypeOid>]) -> TypeOid {
    match func {
        VariadicFunc::Concat | VariadicFunc::ConcatWs => TypeOid::Text,
        // The fallback is **argument one's own type**, not `text`. `nullif_result_type`
        // answers `None` only for a pair with no `=` operator at all — which the operand
        // gate rejects (`42883`) before this matters — and for two unknown arguments,
        // where `text` is Postgres's answer. Falling back to `text` unconditionally
        // meant a `nullif(time_col, timestamp_col)` that slipped past the gate declared
        // `text` (OID 25) over eight bytes of `time` microseconds; falling back to
        // argument one keeps the declaration equal to what `eval_nullif` returns, so the
        // worst case is a wrong but well-formed type rather than a mis-decode.
        VariadicFunc::NullIf => {
            let first = arg_types.first().copied().flatten();
            let second = arg_types.get(1).copied().flatten();
            nullif_result_type(first, second)
                .or(first)
                .unwrap_or(TypeOid::Text)
        }
        VariadicFunc::Greatest | VariadicFunc::Least => {
            let known: Vec<TypeOid> = arg_types.iter().flatten().copied().collect();
            // The declared-type walker answers the **wire** OID, which for an enum is
            // `text` either way, so it folds the plain form of the rulebook — and it is
            // the only fold that does; every gate reads the identity-bearing one.
            variadic_common_type_oid(&known)
                .or_else(|| known.first().copied())
                .unwrap_or(TypeOid::Text)
        }
    }
}

/// The **set quantifier** written on a function call — `gram.y`'s `func_application`
/// productions `func_name '(' DISTINCT func_arg_list opt_sort_clause ')'` and its
/// `ALL` twin.
///
/// Three states rather than a `bool`, because the two spellings are not each other's
/// negation. `ALL` is the **no-op** quantifier: 18.6 accepts it on any call that
/// resolves, aggregate or not (`gram.y`'s own comment on that production says it would
/// like to mark the node "must be an aggregate" and has nowhere to put the bit), so
/// `upper(ALL 'a')` answers `A` there. `DISTINCT` is aggregate-only. A `bool` can spell
/// neither "written, and a no-op" nor "not written" — and dropping `ALL` in the parser
/// instead is a **new silent over-acceptance**, because the names 18.6 owns in its own
/// grammar refuse *either* keyword: `coalesce(ALL 1, 2)` is `42601` there
/// (measured), and a dropped `ALL` answers `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SetQuantifier {
    /// No quantifier — `f(args)`.
    #[default]
    None,
    /// `f(DISTINCT args)` — legal only over an aggregate.
    Distinct,
    /// `f(ALL args)` — legal wherever the call itself is, and semantically nothing.
    All,
}

impl SetQuantifier {
    /// The keyword as SQL spells it, or `None` when the call carries no quantifier —
    /// so a caller that needs the spelling and the written-ness gets both from one
    /// match rather than pairing a predicate with a lookup.
    #[must_use]
    pub const fn keyword(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Distinct => Some("DISTINCT"),
            Self::All => Some("ALL"),
        }
    }

    /// 18.6's `agg_distinct` refusal, for a call whose name resolved to an ordinary
    /// function: `42809 DISTINCT specified, but <name> is not an aggregate function`
    /// (`ParseFuncOrColumn`, the `if` immediately below the `agg_star` arm
    /// [`niladic_call`] implements). `ALL` passes — it is the no-op quantifier.
    ///
    /// The rule is here and its **positions** are its callers': [`build_call`]'s tail,
    /// the two arms that carry it inside [`dispatch_call`],
    /// [`refuse_from_unnest_quantifier`], and — outside the planner —
    /// `parser::ddl`'s `parse_nextval_arg`, the DDL `DEFAULT` classifier's `nextval`
    /// arm, which parses its own argument list and so cannot reach any of the others.
    /// Every one of them is the same relative position, which is the whole of this
    /// function's contract, and it has **two** halves above it rather than one: it runs
    /// after 18.6's argument transform ([`transform_call_args`] — a constant cast in any
    /// argument is `22P02` there, ahead of this) and after every check standing in for
    /// `func_get_detail`'s signature resolution (an argument count — `42883` there, also
    /// ahead of this), and ahead of every check standing in for something 18.6 raises
    /// later. Stating only the second half is what let the first be implemented on one
    /// arm: `upper(DISTINCT 'a'::int)` answered `42809` here against `22P02` on 18.6
    /// while `nextval(DISTINCT 'sq'::int)` agreed.
    /// The classifier reaches this position with **both** of those halves missing rather
    /// than reordered — it runs no argument transform and resolves no signature — which
    /// is two residuals recorded on `parse_nextval_arg`, not a second position.
    /// That is an **enumeration over `dispatch_call`'s error sites**, not a property of
    /// this function: every arity helper it reaches (`math_call`, `string_call`,
    /// `variadic_call`, `binary_call`, `single_arg`, `niladic_call` and the inline
    /// `try_from` arms) checks an argument count and nothing else, and the two sites
    /// that do more — the `jsonb_build_object` parity check and [`sequence_call`]'s name
    /// reduction — carry the call themselves. A new arm doing post-arity work has to be
    /// asked the same question.
    ///
    /// # Errors
    ///
    /// `Error::NotAnAggregate` when `self` is [`Self::Distinct`].
    pub(crate) fn refuse_distinct(self, name: &str) -> Result<()> {
        if self == Self::Distinct {
            return Err(Error::NotAnAggregate {
                modifier: "DISTINCT".to_string(),
                name: name.to_string(),
            });
        }
        Ok(())
    }
}

/// Whether a set quantifier written on `name` is a **syntax** error (`42601`) here,
/// rather than reaching [`SetQuantifier::refuse_distinct`]'s `42809` or — for `ALL` —
/// being dropped as the no-op it is.
///
/// **The set is derived per member from one catalog fact, and the derivation is a query
/// rather than an analogy.** Sweep every name `dispatch_call` matches against 18.6's
/// `pg_proc` and exactly these are absent: `coalesce`, `greatest`, `least`, `nullif`,
/// `trim`, `uuid_generate_v4`. A name 18.6 cannot resolve is one whose every call it
/// **refuses**, so a spelling of it that reaches [`build_call`] and *answers* is a
/// silent wrong — and the members are the five where that can happen:
///
/// - `coalesce` / `greatest` / `least` / `nullif` are `func_expr_common_subexpr`
///   productions, which no `func_application` rule reaches, so 18.6 answers `42601` to
///   the bare spelling — measured for both keywords apiece (`coalesce(DISTINCT 1, 2)`,
///   `greatest(DISTINCT 1)`, `least(DISTINCT 1, 2)`, `nullif(DISTINCT 1)` and their
///   `ALL` twins). pglet routes the bare spelling here, so without this test
///   `coalesce(ALL 1, 2)` answers `1`.
/// - `trim` is a `gram.y` production there too, but pglet owns its bare spelling in
///   `parser::parse_trim`, which refuses a quantifier `42601` before this function is
///   reached (measured on both engines) — so this test is **dead for the bare spelling**
///   and exists for the *quoted* one, which leaves that route and lands on the
///   `"btrim" | "trim"` arm below. `pg_proc` holds `btrim` and not `trim`, so 18.6
///   answers `42883 function trim(unknown) does not exist` to `"trim"(ALL ' x ')` where
///   this engine answered `x` — measured, and the reason the set cannot be read off
///   18.6's grammar list.
/// - `uuid_generate_v4` is absent from `pg_proc` for an unrelated reason (the oracle has
///   no `uuid-ossp`) and is **not** a member, because it is niladic: `niladic_call`'s
///   arity check refuses every quantified spelling that reaches [`build_call`], at
///   18.6's own code. `uuid_generate_v4(ALL 1)` and `"uuid_generate_v4"(DISTINCT 1)` are
///   `42883` on both engines; adding it here would replace those two agreements with
///   `42601`. (Its third quantified spelling, `uuid_generate_v4(ALL)`, is `42601` on both
///   either way — the grammar bound answers in the parser, ahead of this function, so it
///   is not evidence for the membership question in either direction.)
///
/// The member that fixes the **position** is `nullif(DISTINCT 1)`: its argument count is
/// also wrong, and 18.6 still answers the syntax error, which is why this test runs
/// ahead of [`build_call`]'s arity checks rather than beside its `refuse_distinct`
/// position. It runs ahead of [`transform_call_args`] for the same reason and by a
/// separate measurement, since a bison refusal precedes the argument transform too:
/// `coalesce(DISTINCT 'a'::int, 1)` is `42601` on 18.6 where the unquantified
/// `coalesce('a'::int, 1)` is `22P02`, and that pair is what identifies which of the two
/// gates decides.
///
/// `substring` is the control that identifies the rule as `pg_proc` membership rather
/// than grammar membership — it is a grammar production on 18.6 **and** a `pg_proc`
/// entry, so its quoted spelling resolves there and this engine's ordinary verdicts
/// already agree: `"substring"(ALL 'abcdef', 2)` answers `bcdef` on both and
/// `"substring"(DISTINCT 'abcdef', 2)` is `42809` on both. Adding it here would break
/// the first. `position` / `extract` / `cast` cannot be members at all — `dispatch_call`
/// models none of them, so no spelling reaches this function.
///
/// The divergence every member keeps: a **quoted** spelling is a `pg_proc` lookup on
/// 18.6, so `"coalesce"(DISTINCT 1, 2)` and `"trim"(ALL ' x ')` are `42883` there
/// against `42601` here. That is the recorded quoted-name class — pglet's function table
/// does not record which of its names `pg_proc` holds — and it is loud on both sides,
/// where the spellings it replaces answered a row.
///
/// A name added to `dispatch_call` re-runs the sweep. The question to ask of it is
/// whether 18.6's `pg_proc` holds the name, not whether its grammar does.
fn grammar_only_name(name: &str) -> bool {
    matches!(name, "coalesce" | "greatest" | "least" | "nullif" | "trim")
}

/// 18.6's **argument transform** — the pass every verdict about the *call* stands behind
/// — run **only for a call that wrote a set quantifier**.
///
/// `transformFuncCall` runs `transformExprRecurse` over each argument before it hands the
/// list to `ParseFuncOrColumn`, so an error an **argument** raises outranks the signature
/// resolution (`42883`), the `agg_star` arm and the `agg_distinct` arm alike. The one such
/// error pglet can raise this early is the **constant cast**: 18.6 coerces a
/// `'literal'::type` constant during that transform (`coerce_type` → `stringTypeDatum`,
/// `parse_coerce.c`), so `SELECT upper(DISTINCT 'a'::int)` is `22P02` there and not the
/// `42809` the modifier alone would reach.
///
/// **The gate is the whole of why this is here.** pglet raises a quantifier's verdict in
/// its *parser*, which runs ahead of every executor-time statement gate, so a transform
/// run for an **unquantified** call pre-empts errors — and successes — that 18.6 decides
/// first. Measured against 18.6 and against the build before this gate, with the transform ungated:
/// `CREATE TABLE IF NOT EXISTS existing (b TEXT DEFAULT upper('a'::int))` — the
/// idempotent-migration idiom, a **skip** on 18.6 (`transformCreateStmt` returns before
/// anything is transformed, `parse_utilcmd.c`) and a skip here before this gate — was
/// `22P02`, and a client's `prepare()` failed at `Parse`; twelve more unquantified DDL
/// statements moved the same way (`42704` / `42P07` / `42P16` / `42703` / `42P01` /
/// `42601` → `22P02`) beside four more `IF NOT EXISTS` skips, because 18.6 cooks a
/// column `DEFAULT` and a `CHECK` in
/// `AddRelationNewConstraints`, *after* `heap_create_with_catalog`, where pglet's
/// equivalents are `executor::ddl`'s. So this transform runs where 18.6's *does* run
/// ahead of the statement — for a quantified call, whose verdict is already in the parser
/// — and nowhere else. The invariant it keeps is that **no unquantified statement this
/// transform can reach changes
/// phase or answer relative to the build before this gate**: every ordinary expression position was
/// measured unchanged (`22P02` on 18.6, on that build and here, at `upper`,
/// `abs`, `length`, `concat`, `to_char`, `date_part`, `starts_with`, `count`,
/// `jsonb_build_object`, `nextval`, `setval`, `unnest`, `trim`, `substring`, in a select
/// list and in `INSERT`/`UPDATE`/`DELETE`), because the plan-time
/// [`crate::planner::validate_const_casts`] pass already answers there — except the two
/// sequence names, whose name argument never reaches that pass and which [`sequence_call`]
/// answers with a check of its own. The scope is this transform and not the whole
/// set-quantifier close: one unquantified class *did* move, at the route this call cannot
/// reach — `parser::ddl`'s `parse_nextval_arg` defers its name-literal reduction past the
/// closing paren now, so a bad token inside the call is `42601` where it was pglet's
/// `42602`, toward 18.6 at all three `DEFAULT` positions. That doc and `docs/compatibility.md` carry the
/// members and the control.
///
/// What the gate leaves open is recorded rather than fixed here, because each is a
/// *site's* missing check rather than this one's, and there are two such sites.
/// `set_config('x'::int, 'y', false)` answers `y` where 18.6 is `22P02` — the arm
/// **discards** its `setting` argument, so nothing downstream holds the cast, and no table
/// is involved. The other is the **stored expression** `validate_stored_expr`'s probe
/// judges, where the boundary is the *row count* and not the route: every route reaching
/// that probe accepts the cast while its table holds no row — measured at all five CHECK
/// spellings and both index positions — leaving a table no
/// `INSERT` can satisfy; each route that can meet a row is `22P02` here once the table has
/// one, so the loudness is the back-fill's and not the gate's. A column `DEFAULT` is **not** in that
/// list — `CREATE TABLE t (c TEXT DEFAULT upper('a'::int))` is `22P02` here over an empty
/// table, because [`crate::planner::validate_const_casts`] answers a cooked default — which
/// is the control saying the gap is the probe's filter rather than persistence
/// (`validate_stored_expr`'s
/// representative-row probe evaluates the cast and propagates only
/// `UndefinedFunction`/`NotImplemented`, so the coercion error is discarded by *kind*).
/// Both sites predate this gate, both silent, both in `EDGES.md` — with the routes, the
/// row-count condition, and the dump the
/// stored-expression half costs: `pglet_server dump` emits those expressions and stock
/// 18.6 refuses each `22P02`, loading the tables and rows without the index or the
/// constraints.
///
/// Its position within a quantified call is measured against a gate on each side and
/// neither follows from the
/// other. **Below** [`grammar_only_name`], because 18.6 refuses those names in bison,
/// ahead of any transform: `coalesce(DISTINCT 'a'::int, 1)` is `42601` there while
/// `coalesce('a'::int, 1)` is `22P02`. **Above** the dispatch, so it outranks every arity
/// check `dispatch_call` makes: `setval(DISTINCT 'sq'::int, 1, 2, 3)` — wrong on both
/// counts — is `22P02` on 18.6, where the arity check answers `42883`;
/// `setval(DISTINCT 'sq', 1, 2, 3)`, the same arity with no cast, is `42883` on both and
/// is the control that identifies which of the two decides.
///
/// The routes an argument list reaches are [`build_call`],
/// [`refuse_from_unnest_quantifier`], `parser::select`'s FROM-clause `generate_series`
/// and — outside the planner — `parser::ddl`'s
/// `parse_nextval_arg`, which **cannot** reach this; its doc says why.
///
/// # Errors
///
/// The coercion error of the first unparseable constant string cast — the eval-time
/// cast's own, since [`crate::planner::validate_arg_const_casts`] calls the same
/// `coerce_value`.
pub fn transform_call_args(quantifier: SetQuantifier, args: &[Expr]) -> Result<()> {
    if quantifier == SetQuantifier::None {
        return Ok(());
    }
    args.iter()
        .try_for_each(crate::planner::validate_arg_const_casts)
}

/// 18.6's verdict on a set quantifier written on a **FROM-clause** `unnest(…)`, which is
/// `transformRangeFunction`'s rule before it is `ParseFuncOrColumn`'s.
///
/// A multi-argument FROM-clause `unnest` is expanded into one call per argument
/// (`parse_clause.c` — "a kluge, for sure, but it's less nasty than other ways of
/// implementing the SQL-standard UNNEST() syntax"), and the expansion is skipped for a
/// call carrying **any** decoration, `!fc->agg_distinct` among its guards. So the two
/// keywords part company here in a way they do not at [`build_call`]: `ALL` leaves the
/// expansion alone and answers exactly as the unquantified spelling does — measured,
/// `FROM unnest(ALL ARRAY[1,2], ARRAY[3,4]) AS u(x, y)` answers `1|3` and `2|4` on both
/// engines — while `DISTINCT` suppresses it and leaves an ordinary `unnest(…)` call for
/// `ParseFuncOrColumn`. Over one array that call resolves and the quantifier is refused
/// (`42809 DISTINCT specified, but unnest is not an aggregate function`); over two, no
/// signature exists and 18.6 answers
/// `42883 function unnest(integer[], integer[]) does not exist` **ahead** of the
/// quantifier. Both refusals measured on 18.6.
///
/// **Where the caller puts this call is a second, independent contract, and stating the
/// arity-before-quantifier ordering above as "the precedence [`build_call`]'s tail
/// position reproduces" hid it.** There are two orderings here, not one: this function
/// against the argument list (above), and this function against the call's closing
/// paren. `parse_function_call` renders `build_call` *after* `self.ts.expect(RParen)`,
/// and `parse_unnest_table` rendered this one *before* it — so the arity ordering
/// matched while the position did not, and a syntax error standing **where the closing
/// paren should be** lost to the verdict (a bad token earlier inside the parens never
/// did, since `parse_expr_bp` reaches it first):
/// `FROM unnest(DISTINCT ARRAY[1, 2] junk)` was `42809` here against 18.6's
/// `42601 syntax error at or near "junk"`, its multi-array spelling `42883`, and the
/// unterminated `FROM unnest(DISTINCT ARRAY[1, 2] AS u(x)` the same. Both routes
/// now render the verdict after the paren, and both keep the grammar bound before it —
/// which is a choice about keeping the two routes identical rather than a second
/// correctness rule, since the bound fires only over an empty argument list and the list
/// is empty only when the next token *is* `)`, so moving it below the paren changes no
/// SQLSTATE and no fixture (measured).
/// `functions/set_quantifier_distinct_from_unnest_paren_beats_verdict_loud.sql` guards
/// it. What that does **not** reach is a syntax error after the whole call
/// (`… AS u(x) WHERE`), which is `42601` on 18.6 against `42809` here — the recorded
/// later-syntax-error class, whose fix is moving the verdict out of the parser.
///
/// A **quantified** zero-array call cannot arrive: the grammar bound
/// (`Parser::bound_func_set_quantifier`) refuses a quantifier beside an empty argument
/// list before this is called. The unquantified `FROM unnest()` does arrive, and this
/// function deliberately says nothing about it — a pre-existing divergence of a
/// different class, measured `42883 function unnest() does not exist` on 18.6 against a
/// zero-row, zero-column relation here, which the quantifier rules neither opened nor
/// touch; it is recorded in `EDGES.md` beside `concat()`,
/// the same unchecked-empty-argument-list shape at the route that does reach
/// [`build_call`].
///
/// **Both verdicts stand behind [`transform_call_args`]**, which is this route's half of
/// the rule that arm carries for every other name: 18.6 transforms the arrays before
/// `transformRangeFunction` looks at the decoration at all, so
/// `FROM unnest(DISTINCT ARRAY['a'::int]) AS u(x)` is `22P02` there — as are its
/// two-array spelling (`22P02`, not the `42883` the same statement without a cast gets)
/// and its `ALL` and unquantified twins. All four measured on 18.6, and all four `22P02`
/// here; the **unquantified** one is not this call's doing and never was — that transform
/// is gated on a written quantifier, and the plan-time
/// [`crate::planner::validate_const_casts`] pass answers there, before this gate as now.
///
/// # Errors
///
/// The first argument's constant-cast coercion error, else `Error::UndefinedFunction` for
/// a `DISTINCT` over more than one array and `Error::NotAnAggregate` for one over exactly
/// one.
pub fn refuse_from_unnest_quantifier(quantifier: SetQuantifier, arrays: &[Expr]) -> Result<()> {
    transform_call_args(quantifier, arrays)?;
    if quantifier == SetQuantifier::Distinct && arrays.len() > 1 {
        return Err(arity_error("unnest", &(1..=1)));
    }
    quantifier.refuse_distinct("unnest")
}

/// 18.6's verdict on the argument list of a call written with `OVER` — the window route,
/// which never reaches [`build_call`].
///
/// pglet's three window functions are zero-argument ones, so an argument list either is
/// empty (or a `*`, which contributes no argument: `row_number(*) OVER ()` answers on
/// both engines) or resolves something else, by the name: `row_number` has no other
/// signature (`42883 function row_number(integer) does not exist`), while `rank` and
/// `dense_rank` resolve the hypothetical-set **ordered-set aggregates** of the same
/// names for any non-empty list and are refused for the missing `WITHIN GROUP`
/// ([`Error::WithinGroupRequired`], `42809`) — each measured on 18.6 over one argument
/// and over two. A set quantifier decides nothing here, as on 18.6, where both of its
/// spellings resolve by the argument; it only brings [`transform_call_args`] ahead of the
/// verdict, as it does at [`build_call`].
///
/// # Errors
///
/// The first argument's constant-cast coercion error under a written quantifier, else
/// `Error::UndefinedFunction` for `row_number` and `Error::WithinGroupRequired` for
/// `rank` / `dense_rank` over a non-empty argument list.
pub fn refuse_window_arguments(
    name: &str,
    func: WindowFunc,
    quantifier: SetQuantifier,
    args: &[Expr],
) -> Result<()> {
    transform_call_args(quantifier, args)?;
    if args.is_empty() {
        return Ok(());
    }
    match func {
        WindowFunc::RowNumber => Err(arity_error(name, &(0..=0))),
        WindowFunc::Rank | WindowFunc::DenseRank => Err(Error::WithinGroupRequired {
            name: name.to_string(),
        }),
    }
}

/// Dispatch a function call by name to the typed [`Expr`] variant, given its
/// already-parsed positional arguments and its [`SetQuantifier`].
///
/// The single source of truth for pglet's scalar/aggregate function surface,
/// called by the native parser (`parser::parse_function_call`). Window functions
/// are handled by their callers (they need the `OVER` clause). Covers the niladic
/// impure builtins
/// (`now()` / `gen_random_uuid()`), the binary-modeled builtins
/// (`starts_with`/`to_char`), the variadic `jsonb_build_object`/`coalesce`, the
/// single-arg `unnest`/`lower`, and the aggregates; everything else fails loud.
///
/// **The quantifier's verdict is 18.6's own and it is positional.** `gram.y` bounds it
/// first — a quantifier must be followed by a non-empty argument list, which is the
/// *parser's* half (`parse_function_call`) and deliberately not restated here, so this
/// function never sees a quantifier beside `wildcard`. Then a grammar-only name refuses
/// it outright ([`grammar_only_name`]), ahead of everything below because 18.6's
/// refusal there is a parse error. Then [`transform_call_args`] runs — for a **quantified**
/// call only, which is that function's own contract and not a shortcut — because 18.6
/// transforms a call's arguments before it judges the call, so a constant cast in **any**
/// argument of **any** name outranks both the signature resolution and the modifier
/// (`upper(DISTINCT 'a'::int)` is `22P02` there). Then the dispatch runs, so an argument
/// list matching no signature is refused **ahead of** the quantifier — measured,
/// `upper(DISTINCT 'a', 'b')` is `42883 function upper(unknown, unknown) does not exist`
/// on 18.6 while `upper(DISTINCT 'a')` is `42809` — and the *ordering* is what every arm
/// named below reproduces, whatever code it spells the refusal with.
/// **The refusal's SQLSTATE, though, is the arm's own rather than uniformly 18.6's
/// `42883`**, and reading the ordering as if it fixed the code is how this sentence came
/// to assert one pglet does not have. The arms that build [`Error::UndefinedFunction`]
/// agree with 18.6 — [`string_call`] (`upper(DISTINCT 'a','b')`), [`variadic_call`],
/// [`arity_error`]'s callers (`setval(DISTINCT 'sq',1,2,3)`, `count(DISTINCT a, b)`) and,
/// of `dispatch_call`'s own inline arms, `date_part` and `nullif` — while [`math_call`],
/// [`single_arg`], [`binary_call`] and the inline `set_config` arm build
/// [`Error::NotImplemented`] and answer `0A000`
/// (`set_config(DISTINCT 'x','y')` is `0A000` against 18.6's `42883`, same as its
/// unquantified twin; the other inline `NotImplemented` arm — `coalesce`'s empty list —
/// sits behind the grammar-name gate and the grammar bound, so no quantifier reaches it):
/// `abs(DISTINCT 1,2)`, `round(DISTINCT 1.5,1,2)`, `lower(DISTINCT 'a','b')`, a
/// select-list `unnest(DISTINCT ARRAY[1], ARRAY[2])`, `starts_with(DISTINCT 'ab','a','c')`
/// and `to_char(DISTINCT 1.5,'9','x')`, each `42883` on 18.6 and each answering the same
/// `0A000` for its **unquantified** spelling before this gate and here — so it is
/// [`arity_error`]'s recorded spelling split, not this gate's. One name carries both codes
/// by *route*: `FROM unnest(DISTINCT ARRAY[1], ARRAY[2])` refuses through
/// [`refuse_from_unnest_quantifier`]'s [`arity_error`] and is `42883`, where the
/// select-list spelling of the same call is `0A000`. Then
/// [`SetQuantifier::refuse_distinct`] runs over the built node, testing what the name
/// resolved *to* — `ParseFuncOrColumn`'s `fdresult` guard, of whose three admitted
/// values only `FUNCDETAIL_NORMAL` is reachable here (`Error::NotAnAggregate`'s doc has
/// the other two and the measured pair that identifies them), which is "the dispatch did
/// not build an [`Expr::Aggregate`]".
///
/// Two arms carry the verdict **inside** the dispatch instead, because they do work
/// after their arity check that 18.6 does later than parse analysis: the odd-argument
/// `jsonb_build_object` parity error (18.6: `22023`, at execution) and
/// [`sequence_call`]'s name-literal reduction (`42602`/`42601`, where 18.6 resolves the
/// `regclass` cast after `ParseFuncOrColumn`). Each was measured: 18.6 answers `42809`
/// to `jsonb_build_object(DISTINCT 'a')` and to `nextval(DISTINCT 'bad name')`.
///
/// # Errors
///
/// Returns `Error::NotImplemented` for an unknown function or wrong arity,
/// `Error::UndefinedFunction` for an argument list a modeled function has no
/// signature for, `Error::ParameterlessAggregateWithoutStar` for `count()`,
/// `Error::NotAnAggregate` for a `f(*)` over a name carrying a
/// zero-argument signature (the niladic builtins and `jsonb_build_object`) and for a
/// `DISTINCT` over a non-aggregate, `Error::Syntax` for either quantifier over a
/// grammar-only name, and `Error::Type` for an odd `jsonb_build_object` argument
/// count.
pub fn build_call(
    name: &str,
    args: Vec<Expr>,
    quantifier: SetQuantifier,
    wildcard: bool,
) -> Result<Expr> {
    if let Some(keyword) = quantifier.keyword()
        && grammar_only_name(name)
    {
        return Err(Error::syntax(format!(
            "{keyword} is not allowed in the {name} expression"
        )));
    }
    transform_call_args(quantifier, &args)?;
    let expr = dispatch_call(name, args, quantifier, wildcard)?;
    if !matches!(expr, Expr::Aggregate { .. }) {
        quantifier.refuse_distinct(name)?;
    }
    Ok(expr)
}

/// [`build_call`]'s name dispatch — everything but the two quantifier gates that bracket
/// it there. Two *more* quantifier calls sit inside this function, on the arms whose
/// post-arity work 18.6 does later than parse analysis; [`build_call`]'s doc has the
/// measured ordering.
fn dispatch_call(
    name: &str,
    args: Vec<Expr>,
    quantifier: SetQuantifier,
    wildcard: bool,
) -> Result<Expr> {
    // Only `DISTINCT` reaches an `Expr::Aggregate`'s own flag; `ALL` is the explicit
    // spelling of the default and dedups nothing.
    let distinct = quantifier == SetQuantifier::Distinct;
    // `name` is matched **exactly**: it arrives from `parse_identifier_primary`,
    // already downcased when the source wrote it unquoted. A quoted spelling is a
    // different function name on Postgres too — `SELECT "NOW"()` and
    // `SELECT "COUNT"(*)` are each `42883 function … does not exist` on 18.6 —
    // so it must fall through to the loud arm rather than fold into these.
    //
    // The exactness is what a quoted *upper-case* spelling needs, and it is not
    // enough for a quoted *lower-case* one, because 18.6 resolves a quoted name
    // against `pg_proc` and several names below are grammar rather than functions
    // there. Measured: `SELECT "coalesce"(1,2)`, `"nullif"(1,2)`, `"greatest"(1,2)`,
    // `"least"(1,2)` and `"trim"(' x ')` each answer here and are
    // `42883 function … does not exist` on 18.6, while `"substring"('abcdef',2,3)`,
    // `"count"(*)`, `"now"()`, `"upper"('a')`, `"lower"('A')` and
    // `"gen_random_uuid"()` agree with 18.6 because a real `pg_proc` entry carries
    // each of those names. Separating the two would need this table to record which
    // names are `pg_proc` entries; recorded in `docs/compatibility.md`'s *Identifier case-folding*
    // row and in `docs/parser.md`. (`uuid_generate_v4` is not in that set: it
    // diverges unquoted as well, because the oracle has no `uuid-ossp` extension.)
    // Niladic impure builtin *functions*, evaluated at execution time.
    // `CURRENT_TIMESTAMP`/`CURRENT_DATE` are deliberately absent — those are
    // SQL value functions (parenless special forms, `SqlValueFunction`) handled in
    // the parser, not functions, so `current_timestamp()` fails loud here as
    // Postgres rejects it too.
    // Each `now`-family spelling keeps its own variant. The spelling decides which
    // clock is read (`crate::builtins`), the output column name, the `column_default`
    // / dump rendering, and — for `clock_timestamp` alone — whether an `ADD COLUMN`
    // backfill resolves the default per row.
    //
    // Each routes through `niladic_call`, which refuses an argument list rather than
    // discarding it: these arms used to match on the name alone, so `SELECT now(1)`,
    // `now('zzz', 42)` and `gen_random_uuid(1)` each answered an ordinary value where
    // 18.6 resolves by name *and* argument types and says `42883 function now(integer)
    // does not exist`. Because `build_call` is the one site a function call is built
    // at, that reached every expression position, a parenthesized column `DEFAULT`
    // included.
    match name {
        "now" => return niladic_call(name, &args, wildcard, BuiltinFn::Now),
        "transaction_timestamp" => {
            return niladic_call(name, &args, wildcard, BuiltinFn::TransactionTimestamp);
        }
        "statement_timestamp" => {
            return niladic_call(name, &args, wildcard, BuiltinFn::StatementTimestamp);
        }
        "clock_timestamp" => return niladic_call(name, &args, wildcard, BuiltinFn::ClockTimestamp),
        "gen_random_uuid" | "uuid_generate_v4" => {
            return niladic_call(name, &args, wildcard, BuiltinFn::RandomUuid);
        }
        // The session-information calls. These are ordinary `pg_proc` entries on
        // 18.6 (89 / 861 / 1402) rather than keywords, so a *bare* `version` /
        // `current_database` is `42703` there and here — they are deliberately absent
        // from `SqlValueFunction`. `current_schema` is the one that is **both**, and
        // it reaches this arm only through the parenthesized spelling.
        "version" => return niladic_call(name, &args, wildcard, BuiltinFn::Version),
        "random" => return niladic_call(name, &args, wildcard, BuiltinFn::Random),
        "current_database" => {
            return niladic_call(name, &args, wildcard, BuiltinFn::CurrentDatabase);
        }
        "current_schema" => return niladic_call(name, &args, wildcard, BuiltinFn::CurrentSchemaFn),
        _ => {}
    }
    // `COUNT(*)` is the only valid wildcard-argument call; the two arms under it are
    // refusals that split by **whether the name resolves to a zero-argument
    // signature**, the same rule `niladic_call` above reads off `ParseFuncOrColumn`.
    if wildcard {
        if name == "count" {
            return Ok(Expr::Aggregate {
                func: AggregateFunc::Count,
                arg: Box::new(Expr::Column {
                    table: None,
                    name: "*".to_string(),
                }),
                // Always `false` in practice: `gram.y` combines no quantifier with
                // `'*'`, and `parse_function_call` reproduces that bound, so
                // `count(DISTINCT *)` is `42601` before it reaches here (`42601` on
                // 18.6 too — it used to answer a count). Threaded rather than
                // hard-coded so the bound stays the parser's one statement of itself.
                distinct,
                certainty: ArgCertainty::Undecided,
            });
        }
        // `jsonb_build_object` has a zero-argument `pg_proc` entry — `SELECT
        // jsonb_build_object()` answers `{}` on both engines — so a `*`, which
        // contributes no argument, resolves it and reaches the `agg_star` arm just as
        // the niladic builtins do: measured `42809 jsonb_build_object(*) specified,
        // but jsonb_build_object is not an aggregate function` on 18.6.
        //
        // It is the only name **this match** models below the niladic block that 18.6
        // carries such a signature for — measured over every one of them: the rest are
        // `42883 function f() does not exist` (`upper(*)`, `sum(*)`, `concat(*)`,
        // `concat_ws(*)`, `nextval(*)`, `abs(*)`, …, the variadics included, since
        // `concat` takes a minimum of one argument) or, for the names that are grammar
        // rather than functions there, `42601` (`coalesce(*)`, `greatest(*)`,
        // `least(*)`, `nullif(*)`, `substring(*)`, `trim(*)`). Those all take the generic arm
        // below, a recorded loud divergence (`EDGES.md`,
        // *Wrong-SQLSTATE-class*) — so **a name added to this match later needs its own
        // arm here whenever 18.6 gives it a zero-argument signature**; the enumeration
        // is what makes the sentence true, not the count.
        if name == "jsonb_build_object" {
            return Err(star_not_an_aggregate(name));
        }
        return Err(Error::NotImplemented(format!("{name}(*) not supported")));
    }
    match name {
        // `starts_with(text, prefix)` / `to_char(value, format)` — modeled as
        // binary predicates so they ride the existing `BinaryOp` walkers.
        "starts_with" => binary_call(args, BinaryOp::StartsWith, "starts_with"),
        "to_char" => binary_call(args, BinaryOp::ToChar, "to_char"),
        // `mod` / `power` / `pow` — the `%` and `^` operators spelled as calls.
        "mod" => operator_call(name, args, BinaryOp::ModFn),
        "power" => operator_call(name, args, BinaryOp::PowerFn),
        "pow" => operator_call(name, args, BinaryOp::PowFn),
        // `jsonb_build_object(k1, v1, …)` — variadic, even arity (empty is `{}`).
        //
        // The quantifier is judged **before** the parity check, which is the one
        // position in this arm 18.6's own order can be read off: its signature is
        // `VARIADIC "any"`, so any argument count resolves it and the parity is an
        // *execution*-time `22023` there — measured, `jsonb_build_object(DISTINCT 'a')`
        // is `42809` on 18.6 against the `42804` this arm would otherwise answer.
        "jsonb_build_object" => {
            quantifier.refuse_distinct(name)?;
            if !args.len().is_multiple_of(2) {
                return Err(Error::Type(
                    "argument list must have even number of elements".into(),
                ));
            }
            Ok(Expr::JsonbBuildObject(args))
        }
        "unnest" => Ok(Expr::Unnest(Box::new(single_arg(args, "unnest")?))),
        "lower" => Ok(Expr::Lower(Box::new(single_arg(args, "lower")?))),
        // `pg_column_size("any")`: its origin is read by the coercion pass, which sees the
        // argument's scope (`coercion::classify_column_size`).
        "pg_column_size" => {
            let [arg] = <[Expr; 1]>::try_from(args).map_err(|args| {
                Error::UndefinedFunction(format!(
                    "pg_column_size expects 1 argument, got {}",
                    args.len()
                ))
            })?;
            Ok(Expr::ColumnSize {
                origin: ColumnSizeOrigin::written(&arg),
                arg: Box::new(arg),
            })
        }
        // Numeric math functions. `abs`/`ceil`/`ceiling`/`floor` are unary;
        // `round`/`trunc` accept an optional second scale argument.
        "abs" => math_call(MathFunc::Abs, args, 1..=1),
        "ceil" | "ceiling" => math_call(MathFunc::Ceil, args, 1..=1),
        "floor" => math_call(MathFunc::Floor, args, 1..=1),
        "round" => math_call(MathFunc::Round, args, 1..=2),
        "trunc" => math_call(MathFunc::Trunc, args, 1..=2),
        "sqrt" => math_call(MathFunc::Sqrt, args, 1..=1),
        "sign" => math_call(MathFunc::Sign, args, 1..=1),
        // Strict-NULL string functions. Indexing is 1-based and character-based;
        // arity is checked here (an out-of-range count fails loud). `substring`,
        // `trim`, and `position` also have special SQL grammars in the parser that
        // converge on these same `StringFunc` nodes; `strpos` backs `POSITION`.
        "upper" => string_call(StringFunc::Upper, args, 1..=1),
        "length" => string_call(StringFunc::Length, args, 1..=1),
        "replace" => string_call(StringFunc::Replace, args, 3..=3),
        "substring" => string_call(StringFunc::Substring, args, 2..=3),
        "ltrim" => string_call(StringFunc::Ltrim, args, 1..=2),
        "rtrim" => string_call(StringFunc::Rtrim, args, 1..=2),
        "btrim" | "trim" => string_call(StringFunc::Btrim, args, 1..=2),
        "strpos" => string_call(StringFunc::Strpos, args, 2..=2),
        "lpad" => string_call(StringFunc::Lpad, args, 2..=3),
        "rpad" => string_call(StringFunc::Rpad, args, 2..=3),
        "split_part" => string_call(StringFunc::SplitPart, args, 3..=3),
        // Temporal scalar functions. `date_trunc` is an ordinary 2-arg call (no
        // special grammar). `date_part(field, source)` is the float8-returning twin of
        // `EXTRACT` (which has its own `field FROM source` grammar in the parser and
        // returns numeric) — both build an `Expr::Extract`, differing only in
        // `retnumeric`.
        "date_trunc" => string_call(StringFunc::DateTrunc, args, 2..=2),
        "isfinite" => string_call(StringFunc::IsFinite, args, 1..=1),
        // `current_setting(name [, missing_ok])` and `current_schemas(include_implicit)`
        // — strict scalar calls, so they ride `Expr::StringFn`'s machinery (which is
        // "strict function of N arguments", not "returns text": `date_trunc` and
        // `current_schemas` both answer something else).
        "current_setting" => string_call(StringFunc::CurrentSetting, args, 1..=2),
        "current_schemas" => string_call(StringFunc::CurrentSchemas, args, 1..=1),
        "pg_get_constraintdef" => string_call(StringFunc::PgGetConstraintdef, args, 1..=1),
        "col_description" => string_call(StringFunc::ColDescription, args, 2..=2),
        // Both overloads, `(bigint)` and `(int4, int4)`: one lock, the writer, either way.
        // The other advisory functions stay the default arm's `0A000` — the session-level
        // ones hold past the block, which the writer cannot, and a `try` form answers
        // without waiting, which no host's writer acquisition can.
        "pg_advisory_xact_lock" => string_call(StringFunc::AdvisoryXactLock, args, 1..=2),
        "quote_ident" => string_call(StringFunc::QuoteIdent, args, 1..=1),
        "md5" => string_call(StringFunc::Md5, args, 1..=1),
        "left" => string_call(StringFunc::Left, int4_positions(args), 2..=2),
        "right" => string_call(StringFunc::Right, int4_positions(args), 2..=2),
        "substr" => string_call(StringFunc::Substr, int4_positions(args), 2..=3),
        "octet_length" => string_call(StringFunc::OctetLength, args, 1..=1),
        "pg_get_serial_sequence" => string_call(StringFunc::PgGetSerialSequence, args, 2..=2),
        // 18.6 also has the role-taking three-argument overloads; pglet models no role
        // catalog to resolve a role name against, so those are refused rather than
        // answered for a role that may not exist.
        "has_schema_privilege" if args.len() == 3 => Err(Error::NotImplemented(
            "has_schema_privilege(role, schema, privilege) is not supported".into(),
        )),
        "has_schema_privilege" => string_call(StringFunc::HasSchemaPrivilege, args, 2..=2),
        "date_part" => {
            let [field, source] = <[Expr; 2]>::try_from(args).map_err(|_| {
                Error::UndefinedFunction("date_part expects exactly 2 arguments".into())
            })?;
            Ok(Expr::Extract {
                source: Box::new(source),
                field: Box::new(field),
                retnumeric: false,
            })
        }
        // Non-strict variadic functions (each applies its own NULL rule). `concat` is
        // `VARIADIC "any"` with a one-argument minimum on 18.6, so `concat()` resolves
        // nothing there (`42883`) rather than answering the empty string.
        "concat" => variadic_call(VariadicFunc::Concat, args, 1),
        "concat_ws" => variadic_call(VariadicFunc::ConcatWs, args, 1),
        "greatest" => variadic_call(VariadicFunc::Greatest, args, 1),
        "least" => variadic_call(VariadicFunc::Least, args, 1),
        "nullif" => {
            let [a, b] = <[Expr; 2]>::try_from(args).map_err(|_| {
                Error::UndefinedFunction("nullif expects exactly 2 arguments".into())
            })?;
            Ok(Expr::VariadicFn {
                func: VariadicFunc::NullIf,
                args: vec![a, b],
            })
        }
        // `set_config(setting, value, is_local)` (pg_dump preamble): pglet models no
        // GUCs, so this is a no-op that echoes the `value` argument as text. Aliased
        // so the output column is named `set_config`, matching Postgres.
        //
        // **`search_path` is checked here rather than at the `SET` seam** — a GUC's
        // value is a property of the *request*, not of the statement that spelled it,
        // and `catalog::check_search_path_elements` is the one decider both routes
        // reach. This spelling is the one modern `pg_dump` emits (`SELECT
        // pg_catalog.set_config('search_path', '', false);`) and the CVE-2018-1058
        // hardening idiom, so it is the route a real consumer takes; it used to walk
        // straight past the gate and leave the session believing it was pointed
        // somewhere pglet cannot point.
        //
        // The check reads the setting name as a **literal**, which is what makes
        // "every route is checked" true rather than nearly true: a *computed* name
        // (`'search' || '_path'`, `lower('SEARCH_PATH')`, a `$N`) is refused outright
        // rather than echoed, because pglet does not constant-fold before lowering and
        // an un-foldable name is one this gate cannot prove is not `search_path`.
        // Measured: 18.6 accepts those spellings, so the refusal is pglet-stricter and
        // loud — the direction the engine's correct-or-loud rule takes when it cannot
        // see what it is being asked. Every `pg_dump` preamble line and every
        // `set_config('x', <computed>, false)` shape the walker pins depend on has a
        // literal *name*, so nothing measured moves.
        //
        // A `NULL` value is a **reset** on 18.6, which returns the restored value
        // rather than NULL (`set_config('search_path', NULL, false)` answers
        // `"$user", public` there, measured). pglet's `search_path` is a fixed string,
        // so that answer is representable and is given, as it is for a parameter whose
        // value pglet holds fixed (`catalog::fixed_guc_value`); for every other GUC the
        // restored default is not modelled and the NULL is echoed — `EDGES.md` row 69.
        "set_config" => {
            let [setting, value, _is_local] = <[Expr; 3]>::try_from(args).map_err(|_| {
                Error::NotImplemented("set_config expects exactly 3 arguments".into())
            })?;
            let alias = |expr| {
                Ok(Expr::Alias {
                    expr: Box::new(expr),
                    alias: "set_config".to_string(),
                })
            };
            // A cast around a literal is still a literal name. The parser folds
            // redundant parentheses away, so `set_config(('search_path'), …)` already
            // arrived here as one; `'search_path'::text` and `CAST(… AS text)` did not,
            // and the two spellings of one valid request disagreed. Peeling is
            // 18.6-faithful because the cast decides the argument's *type* and this gate
            // reads its *value*: what the name resolves to is the same string either way.
            let mut setting_literal = &setting;
            while let Expr::Cast { expr, .. } | Expr::CastTypmod { expr, .. } = setting_literal {
                setting_literal = expr;
            }
            let Expr::Literal(name) = setting_literal else {
                return Err(Error::NotImplemented(
                    "set_config requires a literal setting name".into(),
                ));
            };
            let Literal::String(name) = name else {
                if matches!(name, Literal::Null) {
                    return Err(Error::SetRequiresParameterName);
                }
                return Err(Error::NotImplemented(
                    "set_config requires a literal setting name".into(),
                ));
            };
            let name = name.trim().to_ascii_lowercase();
            if name == "search_path" {
                return match &value {
                    Expr::Literal(Literal::Null) => alias(Expr::Literal(Literal::String(
                        DEFAULT_SEARCH_PATH.to_string(),
                    ))),
                    Expr::Literal(Literal::String(list)) => {
                        crate::catalog::check_search_path_value(list)?;
                        alias(value)
                    }
                    _ => Err(Error::UnsupportedSearchPath {
                        requested: "<a value the planner cannot read>".into(),
                    }),
                };
            }
            // Every other GUC whose value pglet decides (`catalog::guc_value_is_checked`) —
            // goes through the same one decider the `SET` statement reaches. A value
            // this cannot read is refused for those and echoed for the rest, which is
            // what keeps `set_config('x', <a column>, false)` (an ordinary modelled
            // expression) working while `set_config('client_encoding', <a column>, …)`
            // cannot slip a value past the gate.
            // The isolation levels are acted on through `SET` alone: this spelling lowers
            // to an echo the session never sees, so a level it would move a block to is
            // refused rather than answered as set — the downgrade the statement spelling
            // no longer makes. `application_name` and `lock_timeout` are the session's
            // other two values, and this spelling cannot move them either.
            let isolation = matches!(
                name.as_str(),
                "transaction_isolation" | "default_transaction_isolation"
            );
            if isolation || matches!(name.as_str(), "application_name" | "lock_timeout") {
                let level = match &value {
                    Expr::Literal(Literal::String(v)) if isolation => {
                        Some(crate::catalog::parse_isolation_setting(&name, v)?)
                    }
                    _ => None,
                };
                if level.is_none_or(crate::executor::IsolationLevel::uses_transaction_snapshot) {
                    return Err(Error::NotImplemented(format!(
                        "set_config cannot set {name}; use SET"
                    )));
                }
            }
            match &value {
                Expr::Literal(Literal::Null) => crate::catalog::check_guc_value(&name, None)?,
                Expr::Literal(Literal::String(v)) => {
                    crate::catalog::check_guc_value(&name, Some(v))?;
                }
                _ if crate::catalog::guc_value_is_checked(&name) => {
                    return Err(Error::NotImplemented(format!(
                        "set_config requires a literal value for {name}"
                    )));
                }
                _ => {}
            }
            // A parameter whose value is fixed answers that value: 18.6's `set_config`
            // returns the setting as it now stands (`UTC` for `'utc'`), not its argument.
            if let Some(fixed) = crate::catalog::fixed_guc_value(&name) {
                return alias(Expr::Literal(Literal::String(fixed.to_string())));
            }
            alias(value)
        }
        // Sequence functions — `nextval`/`currval`/`setval`. Each takes a
        // string-literal sequence name (reduced to its bare key) plus, for setval,
        // the value + optional `is_called` flag. Evaluated at the statement level
        // (they mutate sequence / read per-session state); see `Expr::SequenceFn`.
        "nextval" => sequence_call(SequenceFunc::Nextval, args, 1..=1, quantifier),
        "currval" => sequence_call(SequenceFunc::Currval, args, 1..=1, quantifier),
        "setval" => sequence_call(SequenceFunc::Setval, args, 2..=3, quantifier),
        // `coalesce(a, b, …)` — the first non-NULL argument (1+ args).
        "coalesce" => {
            if args.is_empty() {
                return Err(Error::NotImplemented(
                    "coalesce requires at least one argument".into(),
                ));
            }
            Ok(Expr::Coalesce(args))
        }
        "count" | "sum" | "avg" | "min" | "max" => {
            let func = match name {
                "count" => AggregateFunc::Count,
                "sum" => AggregateFunc::Sum,
                "avg" => AggregateFunc::Avg,
                "min" => AggregateFunc::Min,
                _ => AggregateFunc::Max,
            };
            // Each of these resolves a one-argument signature only, so any other count
            // fails resolution (`42883`) rather than dropping the extra arguments — save
            // `count()`, which resolves the zero-argument aggregate `count(*)` is and
            // is refused for its spelling (`42809`).
            let [arg] = <[Expr; 1]>::try_from(args).map_err(|args| {
                if args.is_empty() && func == AggregateFunc::Count {
                    Error::ParameterlessAggregateWithoutStar {
                        name: name.to_string(),
                    }
                } else {
                    arity_error(name, &(1..=1))
                }
            })?;
            Ok(Expr::Aggregate {
                func,
                arg: Box::new(arg),
                distinct,
                certainty: ArgCertainty::Undecided,
            })
        }
        _ => Err(Error::NotImplemented(format!(
            "function not supported: {name}"
        ))),
    }
}

/// 18.6's `agg_star` refusal — the sibling of [`SetQuantifier::refuse_distinct`] in the
/// same `ParseFuncOrColumn` block, spelled with the modifier Postgres's `errmsg` puts
/// there (`<name>(*)`, the name twice).
fn star_not_an_aggregate(name: &str) -> Error {
    Error::NotAnAggregate {
        modifier: format!("{name}(*)"),
        name: name.to_string(),
    }
}

/// Require exactly one argument (for single-arg scalars like `lower`/`unnest`).
fn single_arg(args: Vec<Expr>, label: &str) -> Result<Expr> {
    let [arg] = <[Expr; 1]>::try_from(args)
        .map_err(|_| Error::NotImplemented(format!("{label} expects exactly 1 argument")))?;
    Ok(arg)
}

/// Build a niladic impure builtin ([`BuiltinFn`]), refusing any argument list.
///
/// **Two checks, and the wildcard one is not covered by the other**: `f(*)` parses
/// to `wildcard = true` with an *empty* `args`, so an arity-only guard passes it
/// through. Their order is 18.6's own, read off `ParseFuncOrColumn`: `func_get_detail`
/// resolves the signature first and a `*` contributes no argument, so a name carrying
/// a zero-argument entry reaches the `agg_star` arm (`42809`, [`Error::NotAnAggregate`])
/// while a non-empty argument list fails resolution ahead of it (`42883`). Measured on
/// both: `now(*)` and `gen_random_uuid(*)` are `42809`, `now(DISTINCT 1)` is `42883`
/// (resolution fails before the aggregate-ness check the `DISTINCT` would reach), and
/// `upper(*)` is `42883 function upper() does not exist` — there is no `upper()`, which
/// is why the generic wildcard arm in [`build_call`] is *not* this error. That test is
/// on the **signature and not on this family**, so `jsonb_build_object(*)` takes
/// [`Error::NotAnAggregate`] from its own arm in that wildcard block; the enumeration of
/// which names qualify lives there and on the error.
///
/// `name` is the **spelled** name, so `uuid_generate_v4(1)` names `uuid_generate_v4`
/// rather than the `gen_random_uuid` its `BuiltinFn` is shared with.
fn niladic_call(name: &str, args: &[Expr], wildcard: bool, func: BuiltinFn) -> Result<Expr> {
    if wildcard {
        return Err(star_not_an_aggregate(name));
    }
    if !args.is_empty() {
        return Err(arity_error(name, &(0..=0)));
    }
    Ok(Expr::Builtin(func))
}

/// A shared spelling of the argument-count refusal; its call sites are the enumeration
/// ([`niladic_call`], [`sequence_call`], `dispatch_call`'s aggregate arm,
/// [`refuse_from_unnest_quantifier`] and [`refuse_window_arguments`] among them). It is
/// **not** the only such spelling in this file: [`string_call`], [`math_call`],
/// [`variadic_call`], [`single_arg`] and [`binary_call`] each carry their own older one,
/// and `dispatch_call` carries four more inline (`date_part` and `nullif` build
/// [`Error::UndefinedFunction`] and agree; `set_config` and `coalesce` build
/// [`Error::NotImplemented`] — of those, `set_config`'s is the one a written quantifier
/// can reach: `set_config(DISTINCT 'x', 'y')` is `0A000` here against 18.6's `42883`,
/// same as its unquantified twin). [`string_call`] and [`variadic_call`]
/// build [`Error::UndefinedFunction`] as this does and so agree with 18.6; [`math_call`],
/// [`single_arg`] and [`binary_call`] build [`Error::NotImplemented`] and raise `0A000`
/// where 18.6 is uniformly `42883` — a
/// recorded divergence (`EDGES.md` row 45, the *Wrong-SQLSTATE-class* row), and one that splits a
/// single name by route: `unnest(DISTINCT ARRAY[1], ARRAY[2])` refuses through this helper
/// (`42883`) in the FROM clause, where the select-list spelling refuses through
/// [`single_arg`] (`0A000`); both are `42883` on 18.6, and the unquantified and `ALL` FROM
/// spellings answer two columns on both engines rather than refusing at all. Measured.
/// Unifying them onto
/// this helper is a behavior change per family and separate work.
///
/// `Error::UndefinedFunction` carries 18.6's own `42883`, and the sentence is pglet's
/// own rather than a reproduction — 18.6 names the argument *types* it could not
/// resolve (`function nextval(unknown, integer) does not exist`) where this names the
/// arity it wanted. Two callers spelled it independently before, which is the drift
/// seam this workspace keeps paying for: the two messages differ only in the numbers,
/// so a divergence between them would read as a deliberate distinction.
fn arity_error(name: &str, arity: &std::ops::RangeInclusive<usize>) -> Error {
    Error::UndefinedFunction(format!(
        "{name}() expects {} to {} argument(s)",
        arity.start(),
        arity.end(),
    ))
}

/// Build an [`Expr::SequenceFn`]: check arity, extract the sequence name from the
/// first (string-literal) argument, and keep the remaining arguments (setval's
/// value + optional `is_called`).
///
/// The quantifier's two neighbours here are ordered against 18.6, each half measured:
/// the arity is `func_get_detail`'s (`setval(DISTINCT 'nosuch', 1, 2, 3)` is `42883`
/// there) and the name reduction stands in for a `regclass` resolution 18.6 performs
/// *after* `ParseFuncOrColumn` (`nextval(DISTINCT 'bad name')` is `42809` there against
/// this function's `42602`), so the modifier sits between them.
///
/// A **third** check sits above both, and it is this arm's own rather than the
/// quantifier's: the name argument is reduced to a `String` and its expression
/// discarded, so a constant cast on it is raised here or **not at all** —
/// `nextval('s'::int)` looked through the cast and **advanced the sequence** where 18.6
/// evaluates `'s'::int` and raises `22P02` (measured, and the same statement is a
/// `setval` write one line over). [`build_call`]'s [`transform_call_args`] does not
/// stand in for it and cannot: that pass runs only for a call carrying a set quantifier,
/// which is the ordinary `nextval('s'::int)` spelling's whole problem. Deleting this as
/// redundant of an *ungated* transform is exactly how the write came back — measured
/// `OK` here against 18.6's `22P02` on both sequence names, with the gate applied and
/// this check still absent.
fn sequence_call(
    func: SequenceFunc,
    args: Vec<Expr>,
    arity: std::ops::RangeInclusive<usize>,
    quantifier: SetQuantifier,
) -> Result<Expr> {
    if !arity.contains(&args.len()) {
        return Err(arity_error(func.name(), &arity));
    }
    let mut it = args.into_iter();
    let name_arg = it.next().ok_or_else(|| {
        Error::UndefinedFunction(format!("{}() expects a sequence name", func.name()))
    })?;
    crate::planner::validate_const_casts(&name_arg)?;
    quantifier.refuse_distinct(func.name())?;
    let seq = sequence_name_from_arg(&name_arg, func)?;
    Ok(Expr::SequenceFn {
        func,
        seq,
        args: it.collect(),
    })
}

/// Extract the bare sequence name from a `nextval`/`currval`/`setval` name argument.
///
/// The argument must be a string literal (`'s'`), optionally wrapped in a cast
/// (`'public.s'::text`); the name is read through the **same**
/// `reduce_name_literal` path `CREATE SEQUENCE`/`DEFAULT nextval(...)` use — so `s`,
/// `public.s` and `"s"` all key the `s` slot, while a non-`public` schema or a 3-part
/// name **fails loud** (`42P01`/`42601`) rather than silently truncating to the last
/// segment (which would advance the wrong sequence), and a malformed name (`'s x'`,
/// an unterminated `"`) is `42602`. A non-literal name (a dynamic regclass, a
/// parameter) fails loud — pglet resolves sequences by literal name only.
///
/// **A bare `::regclass`-wrapped name is refused, and it is *this function* that refuses
/// it** — which is the one thing this look-through does not buy.
/// `parser::parse_cast_target` branches on the canonical type name `parse_type_name`
/// returned — `regclass` exactly — and lowers `'s'::regclass` to an `Expr::StringFn`, so
/// the `Expr::Cast` arm below does not see through it and the `_` arm answers
/// `0A000 not implemented: nextval() requires a string-literal sequence name`, where
/// 18.6 answers `1` over a freshly created sequence (measured on both). A
/// **schema-qualified** one takes the other branch, because `parse_type_name` reads the
/// *qualifier* as its type word and returns `pg_catalog`, which is not `regclass`, so the
/// type resolver refuses it first: `nextval('s1'::pg_catalog.regclass)` is
/// `42704 type "pg_catalog" is not supported` here where 18.6 draws from the sequence,
/// measured.
///
/// The spelling pg_dump emits is nonetheless accepted, because a column `DEFAULT` is
/// parsed by `parser::ddl`'s own `parse_nextval_arg`, which eats the cast token without
/// resolving the type. What
/// *does* reach the look-through is `'s'::text` / `'s'::varchar`, and there it agrees
/// with Postgres, which coerces those to `regclass` at run time.
///
/// The cast expression is **dropped** once the name is extracted, so the caller
/// validates its constant coercion first (see [`sequence_call`]); otherwise
/// `nextval('s'::int)` silently advanced a sequence where Postgres raises `22P02`.
fn sequence_name_from_arg(arg: &Expr, func: SequenceFunc) -> Result<String> {
    // Look through a `'s'::text`-style cast to the underlying literal.
    let inner = match arg {
        Expr::Cast { expr, .. } => expr.as_ref(),
        other => other,
    };
    match inner {
        Expr::Literal(Literal::String(s)) => crate::parser::reduce_name_literal(s),
        _ => Err(Error::NotImplemented(format!(
            "{}() requires a string-literal sequence name",
            func.name()
        ))),
    }
}

/// Build an [`Expr::MathFn`], checking the argument count against `arity`.
fn math_call(
    func: MathFunc,
    args: Vec<Expr>,
    arity: std::ops::RangeInclusive<usize>,
) -> Result<Expr> {
    if !arity.contains(&args.len()) {
        return Err(Error::NotImplemented(format!(
            "{func:?} expects {}–{} argument(s), got {}",
            arity.start(),
            arity.end(),
            args.len()
        )));
    }
    Ok(Expr::MathFn { func, args })
}

/// Build an [`Expr::StringFn`], checking the argument count against `arity`.
///
/// A wrong count is [`Error::UndefinedFunction`] (`42883`), matching Postgres —
/// which resolves `upper('a', 'b')` to no `upper/2` signature — rather than a
/// generic `NotImplemented`.
fn string_call(
    func: StringFunc,
    args: Vec<Expr>,
    arity: std::ops::RangeInclusive<usize>,
) -> Result<Expr> {
    if !arity.contains(&args.len()) {
        return Err(Error::UndefinedFunction(format!(
            "{} expects {}–{} argument(s), got {}",
            // The **SQL** name, not the variant's: `Debug` spells `SplitPart` and
            // `CurrentSchemas` where the user wrote `split_part` and
            // `current_schemas`, and this is a message a client reads.
            crate::deparse::string_fn_name(func),
            arity.start(),
            arity.end(),
            args.len()
        )));
    }
    Ok(Expr::StringFn { func, args })
}

/// The arguments of `left`, `right` or `substr` with a bare string literal past the
/// first resolved to `int4`: every overload of each takes an `int4` there, so 18.6's
/// resolution gives an *unknown* literal that type whatever the first argument is
/// (`left('abc', '2')` is `ab`), and a spelling `int4` cannot read is its `22P02`.
fn int4_positions(args: Vec<Expr>) -> Vec<Expr> {
    args.into_iter()
        .enumerate()
        .map(|(i, arg)| match arg {
            Expr::Literal(Literal::String(_)) if i > 0 => Expr::Cast {
                expr: Box::new(arg),
                target_type: TypeOid::Int4,
            },
            other => other,
        })
        .collect()
}

/// Build an [`Expr::VariadicFn`], requiring at least `min` arguments. A shortfall
/// is [`Error::UndefinedFunction`] (`42883`), as in [`string_call`].
fn variadic_call(func: VariadicFunc, args: Vec<Expr>, min: usize) -> Result<Expr> {
    if args.len() < min {
        return Err(Error::UndefinedFunction(format!(
            "{func:?} expects at least {min} argument(s), got {}",
            args.len()
        )));
    }
    Ok(Expr::VariadicFn { func, args })
}

/// Require exactly two arguments and build a `BinaryOp` (for `starts_with` /
/// `to_char`, which the IR models as binary predicates).
fn binary_call(args: Vec<Expr>, op: BinaryOp, label: &str) -> Result<Expr> {
    let [left, right] = <[Expr; 2]>::try_from(args)
        .map_err(|_| Error::NotImplemented(format!("{label} expects exactly 2 arguments")))?;
    Ok(Expr::BinaryOp {
        left: Box::new(left),
        op,
        right: Box::new(right),
    })
}

/// Build a call-spelled arithmetic operator — `mod(a, b)`, `power(a, b)`, `pow(a, b)` —
/// over exactly two arguments, resolving a bare string literal among them as 18.6's
/// function resolution does where the other argument's type is known from its spelling.
///
/// An *unknown* argument (a string literal, a bare `NULL`) takes the type the call
/// resolves to: beside an integer or `numeric` literal, that literal's own type for
/// `mod` and the `numeric`-or-`float8` choice for `power` (an integer resolving to
/// `float8`, the numeric category's preferred type); beside another unknown, `float8`
/// for `power` and no single signature for `mod` (`42725`). The literal is then a cast
/// to that type, so a spelling the type cannot read is that type's `22P02`.
///
/// Beside any other argument — a column, a parameter, an expression — the literal is
/// left as written, and the operator's own value gate answers for it (`EDGES.md` row
/// 262: it stays `text` beside an operand the plan-time pass does not type).
fn operator_call(name: &str, args: Vec<Expr>, op: BinaryOp) -> Result<Expr> {
    let [left, right] = <[Expr; 2]>::try_from(args).map_err(|_| arity_error(name, &(2..=2)))?;
    let is_mod = op.operator_form() == BinaryOp::Mod;
    let ambiguous = || Error::AmbiguousFunction(format!("{name}(unknown, unknown)"));
    if is_mod
        && matches!(unknown_operand_peer(&left), UnknownPeer::Unknown)
        && matches!(unknown_operand_peer(&right), UnknownPeer::Unknown)
    {
        return Err(ambiguous());
    }
    // Each argument is resolved against the other **as written**, so two unknowns both
    // take the resolved type rather than the second reading the first's new cast.
    let target = |this: &Expr, other: &Expr| -> Option<TypeOid> {
        if !matches!(this, Expr::Literal(Literal::String(_))) {
            return None;
        }
        match (is_mod, unknown_operand_peer(other)) {
            (_, UnknownPeer::Other) => None,
            (true, UnknownPeer::Typed(t)) => Some(t),
            // Two unknowns are refused for `mod` above; `power`'s resolve to `float8`.
            (_, UnknownPeer::Typed(TypeOid::Numeric)) => Some(TypeOid::Numeric),
            _ => Some(TypeOid::Float8),
        }
    };
    let cast = |e: Expr, t: Option<TypeOid>| match t {
        Some(target_type) => Expr::Cast {
            expr: Box::new(e),
            target_type,
        },
        None => e,
    };
    let (left_target, right_target) = (target(&left, &right), target(&right, &left));
    Ok(Expr::BinaryOp {
        left: Box::new(cast(left, left_target)),
        op,
        right: Box::new(cast(right, right_target)),
    })
}

/// What [`operator_call`] can read off the *other* argument beside an unknown one.
enum UnknownPeer {
    /// An unknown argument too: a string literal or a bare `NULL`.
    Unknown,
    /// A number literal, whose type its spelling decides.
    Typed(TypeOid),
    /// Anything else, whose type is not known while the call is built.
    Other,
}

/// [`UnknownPeer`]'s reading of `expr`.
fn unknown_operand_peer(expr: &Expr) -> UnknownPeer {
    match expr {
        Expr::Literal(Literal::String(_) | Literal::Null) => UnknownPeer::Unknown,
        Expr::Literal(Literal::Int(i)) => UnknownPeer::Typed(if i32::try_from(*i).is_ok() {
            TypeOid::Int4
        } else {
            TypeOid::Int8
        }),
        Expr::Literal(Literal::Numeric(_)) => UnknownPeer::Typed(TypeOid::Numeric),
        _ => UnknownPeer::Other,
    }
}

/// Whether `expr`'s scalar tree contains a subquery-bearing node anywhere.
///
/// An [`Expr::Subquery`], [`Expr::Exists`], or [`Expr::InSubquery`] — the only `Expr`
/// variants carrying a sub-[`Plan`](super::Plan). Recurses the scalar children
/// (`for_each_child` treats a subquery as an opaque leaf, so the node itself is what's
/// detected — the self-contained sub-plan need not be descended).
///
/// One derivation, two consumers, deliberately: `parser::ddl`'s
/// `reject_persisted_expr_constructs` refuses one at DDL time (`0A000`) and
/// [`serialize_expr`] refuses one at the store boundary. They stand in for two
/// different things — a Postgres error and no Postgres error at all — so a second copy
/// of this walk would be a drift seam between a conformance rule and a corruption
/// backstop.
#[must_use]
pub fn expr_contains_subquery(expr: &Expr) -> bool {
    expr.any_node(&|node| {
        matches!(
            node,
            Expr::Subquery { .. } | Expr::Exists { .. } | Expr::InSubquery { .. }
        )
        .then_some(true)
    })
}

/// Encode an `Expr` to the planner-IR wire bytes (postcard), **persistence-agnostic**.
///
/// The single owner of the format, and deliberately ungated: its second caller is
/// `planner::coercion`'s `structural_identity`, which encodes a target-list entry as
/// Postgres's `equal()` over two entries and never writes a byte anywhere.
/// A plan-bearing entry is ordinary there — 18.6 accepts
/// `SELECT (SELECT 1) AS q, (SELECT 1) AS q … ORDER BY q` because the two `SubLink`s
/// are `equal()`, and rejects the `(SELECT 1)`/`(SELECT 2)` pair with `42702`; both
/// verdicts here are these bytes compared, so refusing to encode a plan would turn the
/// first into a spurious `42702`.
///
/// [`serialize_expr`] is the persistence-bound wrapper and is where the refusal lives.
/// This one is deliberately **not** re-exported from `planner`: `coercion` is a sibling
/// module and reaches it directly, so no crate outside the planner can encode past the
/// gate.
///
/// # Errors
///
/// Returns `Error::Execution` if postcard serialization fails.
pub fn encode_expr(expr: &Expr) -> Result<Vec<u8>> {
    postcard::to_allocvec(expr).map_err(|e| {
        crate::depth::encoding_error(Error::Execution(format!("expr serialize failed: {e}")))
    })
}

/// Serialize an `Expr` to the catalog/storage wire bytes (postcard).
///
/// The catalog persists `Expr` trees inline (typed) inside the `Schema`, but the
/// storage `IndexSpec` — which lives in the `pglet_core` leaf crate and so cannot name
/// `Expr` — carries them as these opaque bytes, deserialized back via
/// [`deserialize_expr`] by the storage index-maintenance layer (which depends on
/// `pglet_sql`).
///
/// **Refuses a plan-bearing `Expr`** ([`expr_contains_subquery`]), because
/// [`Plan`](super::Plan) does not decode: bytes carrying one would be a write nothing
/// could ever read back. It stands in for no Postgres error — the statement that must
/// never reach it has already been refused at DDL time by `parser::ddl`'s
/// `reject_persisted_expr_constructs` (`0A000`) or `executor::ddl`'s
/// `check_default_expr_nodes`.
///
/// **Read its reach exactly: one persistence site, not the class.** It is reached only
/// through `executor::index_spec_from_catalog`, so it covers the storage `IndexSpec`
/// copy of an index key expression or predicate — at `CREATE INDEX` ahead of the catalog
/// write, and again on every open of a persisted database. The catalog's own copy of the
/// same expression, and CHECK and DEFAULT entirely, go through
/// `catalog::serialize_schema` ungated (it runs on every commit, so it carries no walk)
/// and rest on their two DDL-time refusals — the ranked/positional node walk and the
/// stored-expression cook's probe eval — plus [`Plan`](super::Plan)'s non-decode.
///
/// # Errors
///
/// Returns `Error::Execution` if the tree carries a sub-plan, or if postcard
/// serialization fails.
pub fn serialize_expr(expr: &Expr) -> Result<Vec<u8>> {
    if expr_contains_subquery(expr) {
        return Err(Error::Execution(
            "expr serialize failed: a persisted Expr cannot carry a sub-plan".to_string(),
        ));
    }
    encode_expr(expr)
}

/// Deserialize an `Expr` from the bytes [`serialize_expr`] produced.
///
/// A **plan-bearing** tree is not among them: [`Plan`](super::Plan)'s `Deserialize`
/// always fails, so bytes claiming one fail here too — as ordinary corruption, since
/// postcard's `Error` drops the custom message.
///
/// # Errors
///
/// Returns `Error::Execution` if the bytes are not a valid serialized `Expr`, a
/// plan-bearing one included.
pub fn deserialize_expr(bytes: &[u8]) -> Result<Expr> {
    postcard::from_bytes(bytes).map_err(|e| {
        crate::depth::encoding_error(Error::Execution(format!("expr deserialize failed: {e}")))
    })
}

/// The cells the two declared-type walks answer **identically**, one rule each.
///
/// They are three `Expr` variants plus the `StringFn` **members** whose result follows
/// an argument (`DateTrunc`, `Substring`, `Substr`), each read by both walks instead of
/// two rules.
///
/// pglet declares an output column's type twice: [`Expr::infer_type`] feeds
/// `Plan::result_shape` → `Describe` → the extended-protocol `RowDescription`, and
/// `executor::helpers`'s `expr_type_oid` feeds `QueryResult::shape` → the
/// simple-query `RowDescription`, the column-type-aware text encoder in *both*
/// protocols, and two accumulator selections. They are separate code for two reasons:
/// they resolve their leaves against different objects, and they answer an un-inferable
/// operand differently — the executor walk is **total**, substituting `TypeOid::Text`
/// where this one answers `None`. [`LeafScope`] gives the first half one *shape* —
/// both scopes implement it — but be exact about how far that reaches: the only leaf
/// **both** walks read through the trait is `Subquery`, via this function. This walk's
/// `Column` arm calls `LeafScope::column_type`; `executor::helpers`'s `expr_type_oid`
/// still calls the inherent `ProjectionScope::resolve`, and [`Expr::infer_type`] is
/// never handed a `ProjectionScope` at all. The two bottom out in the same
/// `ProjectionScope::column_desc` and so answer alike, but that is an identity
/// `executor::walker_pins`'s scope test holds, not one the trait enforces. This closes
/// the second half for the cells where it makes no difference:
///
/// | cell | rule |
/// |---|---|
/// | `Default` | no type — a bare `DEFAULT` takes the target column's |
/// | `Parameter` | no type — an unbound `$N` |
/// | `Subquery` | the plan's first output column, through the scope |
/// | `StringFn` / `DateTrunc` | [`date_trunc_result_type`] of the **source** argument |
/// | `StringFn` / `Substring`, `Substr` | [`substring_result_type`] of the first argument |
///
/// The first three answer `None`, or whatever the scope answers, so the executor walk's
/// `Text` substitution — applied where it *consumes* this, not here — reproduces its
/// previous arms exactly. The `StringFn` members answer through tables that each map
/// `Some(TypeOid::Text)` and `None` to one result — `timestamp` for `date_trunc`, `text`
/// for `substring` / `substr` — so the two walks' different source answers land on one
/// result.
///
/// **That is the whole safety argument, and it is why the other cells are not here.**
/// Every remaining difference between the two walks (an unresolvable `Column`, `Unnest` /
/// `MathFn`, the arithmetic fallback, a bare `NULL` `Literal` and its `Bound` NULL twin)
/// has an answer that *moves* with the unknown-ness policy, so each is a decision
/// against the oracle — its own change, with its own measurement — rather than a
/// refactor's side effect. `math_fn_result_type`'s doc holds that policy open and says
/// why; `executor::walker_pins` pins today's answers on both sides of the seam, merged
/// and unmerged alike.
///
/// **"Its answer moves with the policy" is not the same as "its code cannot be merged",
/// and the difference was measured rather than reasoned.** A cell whose two arms differ
/// only through the *recursion* — where each walk reads its operand through its own hook
/// — reproduces both answers from one merged arm. The variadic functions, `COALESCE`,
/// `CASE` and `ARRAY[…]` were each in that shape and are merged now, into
/// [`shared_multi_operand_type`]; what the merge of `ARRAY` cost is stated there.
///
/// An unresolvable `Column` reaches the same conclusion by a **third** route, and it is
/// not the recursion's: a leaf reads no operand, so the two arms differ through neither
/// the rule nor the recursion but through the **scope**, which [`LeafScope`] gives one
/// shape — though, as above, this walk's `Column` arm is the only one of the two that
/// actually goes through it. Its merged form is `column_type` under the same totality
/// boundary the cells above take, and that the two answers agree under that
/// substitution is what `executor::walker_pins`'s scope test pins, case by case. What
/// holds it open is the fenced unknown-ness question alone — what an unresolvable
/// column should *declare*.
/// The whole-`ColumnDesc` reason (a passthrough projection carries the source column's
/// typmod and provenance, not just its type) is about a different function:
/// `executor::helpers`'s `expr_output_column` takes that descriptor from the scope
/// **directly**, on a passthrough branch that enters neither walk.
///
/// The ones that genuinely cannot be merged without deciding their answer are the ones
/// holding a rule the other walk has no counterpart for, and the list is **re-derived**
/// arm by arm rather than decremented — merging `ARRAY` removed one member and the
/// re-derivation found a fourth the old list had folded into "arithmetic":
///
/// - `unnest`'s fallback to the inner type (`array_element().unwrap_or(type_oid)`) — the
///   executor walk's, and pinned at `unnest(1)`. Still four members: the plan-time
///   `unnest(<non-array>)` gate ([`super::legality::unnest_legality`]) refuses the
///   *directly typed* arguments this arm used to answer for, but it reads
///   [`super::coercion`]'s narrow `arg_known_type`, so every argument that reader's `_`
///   arm swallows still reaches both arms. `unnest(i + 0)`, `unnest(length(tx))` and the
///   `CASE`/`NULLIF`/`GREATEST`/`LEAST`/unary-`-` spellings over an `INT` argument split
///   them 23-against-25, and `unnest(b + 0)` 20-against-25. The `Expr::Alias`
///   `unnest(set_config('x', tx, false))` is a member of that reader class too, measured
///   25 on both — as is `unnest(tx || 'a')`. A refusal is not a merge, and the arm was
///   not removed;
///
/// - the bare-literal carve-out in `abs` — the executor walk's, pinned at `abs(NULL)`;
/// - arithmetic's `text`-left divert (`if left == Text { right }`) — the executor walk's,
///   pinned at `NULL + 1`;
/// - `^`'s short-circuit of any half-unknown operand pair to `float8` — the **planner**
///   walk's, where the executor walk always folds through [`numeric_family_result`];
///   pinned at `NULL ^ n` (cell K's third member).
///
/// Each **reddens its own pin** when written in the merged form, which is how the two
/// halves were told apart rather than argued apart.
///
/// It was **five**, and the fifth is the correction worth carrying: the executor walk's
/// `Aggregate` arm held a `float8` guess for an unresolved `sum`/`min`/`max` argument,
/// guarded on the projection scope having no input columns. Its own comment justified
/// that guard by "a subquery without schema", and that is why the arm was recorded dead
/// — measured, the scalar-subquery route never fires it (`SELECT (SELECT max('abc') FROM
/// zc)` and siblings declare `25` on both legs) — while the caller that actually reached
/// it was `execute_aggregate` over a FROM whose relations are **all zero-column**. Two
/// measured routes build one: `ALTER TABLE … DROP COLUMN` down to nothing, which both
/// engines accept, and a `RETURNING`-less data-modifying CTE the outer query references,
/// which 18.6 refuses outright (`0A000`) — so "which both engines accept" bounds the
/// first spelling and not the class (`EDGES.md` rows 58 and 61). The *all* is
/// the guard's own width — `scope.columns.is_empty()` reads the projecting node's whole
/// input, so one columned relation anywhere in the FROM defeats it: measured
/// pre-deletion on the simple leg, `SELECT max('abc') FROM zc a, zc b` declared `701`
/// and `SELECT max('abc') FROM zc, t2` declared `25`. Deleting it moved 10 of 13
/// measured shapes over such a FROM onto 18.6's answer — seven to byte-and-OID-identical
/// on both legs, and three (`sum`/`avg` over the guessed column, in a derived table and
/// in a CTE) from a silent NULL row to 18.6's own `42883`. The three that did not move
/// **onto 18.6's answer** are unrelated to the guess: `sum(NULL)`/`avg(NULL)` still
/// answer a NULL row where 18.6 is `42725` (the fenced unknown-argument leniency,
/// identical over an ordinary table), and `sum('abc')` is `42883` here against that same
/// `42725`. Only the *row* is unmoved for the first: `SELECT sum(NULL) FROM zc` did move
/// at the OID, `701` → `25` on the simple leg, the guess having typed it, while
/// `avg(NULL)` is `701` on both sides for an independent reason
/// (`aggregate_result_type(Avg, Some(Text))` is `Float8`). `select/zero_column_relation_*`
/// are the fixtures.
///
/// `operand_type` is the **caller's own walk**, and it is a parameter precisely because
/// it is what still differs: the executor walk hands back `Some(TypeOid::Text)` for an
/// un-inferable source where this one hands back `None`. Only the `date_trunc` rule
/// reads an operand.
///
/// Returns `None` for every other variant, which is not a rule about them but "no shared
/// rule owns this". It is unreachable from the two call sites — each is a single match
/// arm naming exactly the four patterns above — and a caller that widened its arm
/// without widening this match would get the conservative answer (the planner walk's own
/// unknown, the executor walk's `Text`) rather than a confidently wrong type.
///
/// **The four patterns are written in three places and only two of them are guarded**,
/// measured one deleted pattern at a time. At either call site, dropping `Default`,
/// `Parameter` or `Subquery` leaves that walk's own match non-exhaustive (`E0004`), and
/// dropping `DateTrunc` compiles — `StringFn`'s other members keep an arm there — and
/// reddens the cell-M pin. In **this** match only the two arms carrying a rule are
/// guarded that way: deleting the `Subquery` or the `DateTrunc` arm reddens a pin, while
/// its `Default` and `Parameter` arms are **documentation** — deleting either compiles
/// and leaves the pin corpus green, because the `_ => None` fallback answers identically.
/// They are spelled out because the classification is what the call sites' patterns are
/// documented against, not because anything checks them.
// `match_same_arms`: the two `None` rules and the wildcard say different things — "this
// variant has no type" against "no shared rule owns this variant" — and collapsing them
// would delete the classification the two call sites' patterns are documented against.
#[allow(clippy::match_same_arms)]
#[must_use]
pub fn shared_leaf_cell_type(
    expr: &Expr,
    scope: &dyn LeafScope,
    operand_type: &dyn Fn(&Expr) -> Option<TypeOid>,
) -> Option<TypeOid> {
    match expr {
        // A bare `DEFAULT` has no type of its own: it takes the target column's, which
        // is the assignment's business rather than the expression's.
        Expr::Default => None,
        // An unbound `$N` has no type until it is bound — a bound one is `Expr::Bound`,
        // which carries its value's type and is deliberately not merged here (a bound
        // NULL has no type either, and *that* is the fenced unknown-ness question).
        Expr::Parameter { .. } => None,
        // A scalar subquery is its plan's first output column — the scope's business,
        // because only it knows what that plan resolves against.
        Expr::Subquery { plan } => scope.subquery_type(plan),
        // `date_trunc`'s result type is its **source** (arg 1), unlike the other string
        // functions' fixed result type.
        Expr::StringFn {
            func: StringFunc::DateTrunc,
            args,
        } => Some(date_trunc_result_type(args.get(1).and_then(operand_type))),
        // `substring` / `substr` answer their first argument's type: `bytea` over a
        // `bytea`, `text` otherwise.
        Expr::StringFn {
            func: StringFunc::Substring | StringFunc::Substr,
            args,
        } => Some(substring_result_type(args.first().and_then(operand_type))),
        _ => None,
    }
}

/// The rule both walks apply to a node that picks **one operand's type out of several** —
/// `ARRAY[…]`, `greatest`/`least`/`nullif`, `COALESCE` and `CASE`.
///
/// These four differ from [`shared_leaf_cell_type`]'s cells in what made them mergeable:
/// not that the unknown-ness cannot show, but that the two arms differed **only through
/// the recursion**. Each walk passes its own operand reader as `operand_type`, so the
/// merged rule reproduces each walk's former answer from its own hook — measured one
/// merge at a time against `executor::walker_pins`, not argued.
///
/// | cell | rule |
/// |---|---|
/// | `Array` | the first operand that types, `text` if none, one dimension up |
/// | `VariadicFn` | [`variadic_fn_result_type`] over every operand's answer |
/// | `Coalesce` | the first operand that types |
/// | `Case` | the first `WHEN` result that types, then the `ELSE` |
///
/// **The merge moved exactly one answer, and it is unreachable through SQL.** The
/// executor walk's `Array` arm carried its `TypeOid::Text` substitution *inside* the
/// `array_of()` step (`elem.array_of().unwrap_or(TypeOid::TextArray)`) rather than at
/// the totality boundary its caller applies, so an element type with no array type
/// answered `text[]` where the boundary answers `text`. The only element type pglet has
/// that reaches it is an array — `array_of()` is total over every scalar — and a nested
/// `ARRAY[ARRAY[…]]` is `0A000 multidimensional arrays are not supported` on both legs
/// before either walker's answer is read (18.6 answers `int4[]`). `walker_pins` records
/// the new answer with that reasoning; no fixture can.
///
/// Returns `None` for every other variant — "no shared rule owns this", the same
/// convention [`shared_leaf_cell_type`] uses, and unreachable from the two call sites,
/// each of which is a single match arm naming exactly these four patterns.
///
/// **That sentence is the sibling's, and so is the guardedness analysis it needs**, which
/// comes out differently here: the four patterns are written in three places and the two
/// *call sites* are guarded while this match is guarded by the pins. Neither walk's own
/// match carries a wildcard, so dropping any of the four at either call site is `E0004`
/// — measured for `Array` at `executor::helpers`'s and for `Case` at `Expr::infer_type`'s.
/// Dropping an arm from **this** match compiles, because the `_ => None` fallback answers
/// it, and reddens the corresponding pin — measured, deleting the `Array` arm fails cell
/// E's assertion (`executor walk over ARRAY[NULL,1]`, `Text` against `Int4Array`) and
/// deleting the `Coalesce` arm fails cell I's. So every arm here carries a rule and none
/// is documentation, which is the half that differs from [`shared_leaf_cell_type`]'s.
#[must_use]
pub fn shared_multi_operand_type(
    expr: &Expr,
    operand_type: &dyn Fn(&Expr) -> Option<TypeOid>,
) -> Option<TypeOid> {
    match expr {
        // Array type = array_of(the first element that types), default text[].
        Expr::Array(elems) => elems
            .iter()
            .find_map(operand_type)
            .unwrap_or(TypeOid::Text)
            .array_of(),
        // Variadic functions: concat/concat_ws are text; greatest/least/nullif fold
        // their arguments. An operand the hook declines is `None` here, which the fold
        // skips rather than counting as a `text` candidate.
        Expr::VariadicFn { func, args } => {
            let types: Vec<Option<TypeOid>> = args.iter().map(operand_type).collect();
            Some(variadic_fn_result_type(*func, &types))
        }
        // `COALESCE(a, b, …)` takes the first argument that types (all branches are the
        // same type in well-formed SQL).
        Expr::Coalesce(args) => args.iter().find_map(operand_type),
        // `CASE` takes the first result branch that types — the `WHEN` results in order,
        // then the `ELSE`.
        Expr::Case {
            when_clauses,
            else_result,
            ..
        } => when_clauses
            .iter()
            .map(|(_, result)| result)
            .chain(else_result.as_deref())
            .find_map(operand_type),
        _ => None,
    }
}

/// The `BinaryOp` cell of the declared-type walk — an operator's result type from its
/// operands' types, and the one place that rule lives.
///
/// Extracted from [`Expr::infer_type_with`] for the reason
/// [`shared_multi_operand_type`] was: a second walk reads it. The executor's eval walk
/// derives a `BinaryOp` node's type from operand types it already holds, per row, and
/// reaching this arm through the whole-`Expr` match cost about as much again as the arm
/// itself (measured: the same depth-32 chain 303 ms through the match, 274 ms calling
/// here, 256 ms deriving nothing at all).
///
/// `lt` / `rt` are the operands' declared types (`None` where the walk cannot answer);
/// the operators that ignore them ignore them here too.
///
/// **The invariant: [`Expr::infer_type_with`]'s `BinaryOp` arm must stay a *bare* call to
/// this function.** The eval walks reach a node's rule through `derive_node_type` →
/// [`Expr::infer_type_with`] where they can, but their `BinaryOp` sites call here
/// **directly** — three sites, which are this function's callers other than
/// [`Expr::infer_type_with`]'s own arm (`executor::expr`'s scalar walk, two in
/// `executor::query::aggregate`; re-derive them there) — so a rule
/// added to that arm would reach the planner's walk and not theirs. Either keep the arm
/// bare, or move those sites onto `derive_node_type` in the same change.
// arms grouped by operator family for documentation; several share a body (the
// boolean-valued and the text-valued operators) but are kept separate intentionally
#[allow(clippy::match_same_arms)]
#[must_use]
pub fn binary_op_result_type(
    op: BinaryOp,
    lt: Option<TypeOid>,
    rt: Option<TypeOid>,
) -> Option<TypeOid> {
    // `mod(…)` / `power(…)` / `pow(…)` are typed as the operator each is.
    let op = op.operator_form();
    match op {
        // Comparison operators always return Bool
        BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge
        | BinaryOp::And
        | BinaryOp::Or
        | BinaryOp::AndList
        | BinaryOp::OrList
        | BinaryOp::AndListInner
        | BinaryOp::OrListInner
        | BinaryOp::Like
        | BinaryOp::NotLike
        | BinaryOp::Ilike
        | BinaryOp::NotIlike
        | BinaryOp::StartsWith
        | BinaryOp::RegexMatch
        | BinaryOp::NotRegexMatch
        | BinaryOp::IsDistinctFrom
        | BinaryOp::IsNotDistinctFrom => Some(TypeOid::Bool),
        // Arithmetic: `timestamp ± interval → timestamp` and
        // `timestamp - timestamp → interval` when both operand types
        // are known; otherwise follow the left operand (falling back
        // to the right when the left is unknown). `%` (`Mod`) rides the
        // same numeric-family rule (`int % int → int`, `int % numeric →
        // numeric`); it has no temporal form.
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::Mod
        | BinaryOp::ModFn => match (lt, rt) {
            (Some(l), Some(r)) => arith_result_type(op, l, r).or(Some(l)),
            _ => lt.or(rt),
        },
        // `^` (exponentiation): `numeric ^ numeric → numeric`, but every
        // other combination (incl. `int ^ int` and **`real ^ real`**)
        // resolves to `float8` — Postgres registers only
        // `^(float8,float8)` and `^(numeric,numeric)`, so there is no
        // `real` exponentiation to keep the width.
        BinaryOp::Pow | BinaryOp::PowerFn | BinaryOp::PowFn => Some(match (lt, rt) {
            (Some(l), Some(r)) => pow_result_type(numeric_family_result(l, r)),
            _ => TypeOid::Float8,
        }),
        // String concatenation always yields text.
        BinaryOp::Concat => Some(TypeOid::Text),
        // JSON operators
        BinaryOp::JsonGet => Some(TypeOid::Jsonb),
        BinaryOp::JsonGetText => Some(TypeOid::Text),
        // JSON key/path operators: `?`/`?|`/`?&` → bool, `#>` → jsonb,
        // `#>>` → text.
        BinaryOp::JsonExists | BinaryOp::JsonExistsAny | BinaryOp::JsonExistsAll => {
            Some(TypeOid::Bool)
        }
        BinaryOp::JsonPathGet => Some(TypeOid::Jsonb),
        BinaryOp::JsonPathGetText => Some(TypeOid::Text),
        // Containment (`@>` / `<@`) returns boolean
        BinaryOp::Containment | BinaryOp::ContainedBy => Some(TypeOid::Bool),
        // `ts AT TIME ZONE 'UTC'` yields a timestamp; `to_char` yields text.
        BinaryOp::AtTimeZone => Some(TypeOid::Timestamp),
        BinaryOp::ToChar => Some(TypeOid::Text),
    }
}

/// Expression type inference for `Plan::output_columns()`.
impl Expr {
    /// Infer the output type of this expression.
    ///
    /// Used by `Plan::output_columns()` to determine column types without execution.
    /// Returns `None` for types that can't be determined statically (e.g., NULL literals).
    ///
    /// The walk itself reads no catalog: its two outward-facing leaves — a `Column` and
    /// a scalar `Subquery` — go through [`LeafScope`], and **which scope the caller
    /// builds is a correctness decision, not a convenience one** (see [`TypeScope`],
    /// which the planner's implementation carries). A projecting plan node passes
    /// [`TypeScope::Input`]; an expression with no FROM clause to resolve against (a
    /// stored `CHECK`, a `RETURNING` item, a no-FROM `VALUES` row) passes
    /// [`TypeScope::Catalog`].
    ///
    /// This is [`Expr::infer_type_with`] driven by its own recursion — the whole walk,
    /// which is what every caller outside a per-row loop wants.
    pub fn infer_type(&self, scope: &dyn LeafScope) -> Option<TypeOid> {
        self.infer_type_with(scope, &|e| e.infer_type(scope))
    }

    /// [`Self::infer_type`] for an expression the coercion pass has **not** resolved yet
    /// — what a parse-time error message names an operand's type from.
    ///
    /// An aggregate the pass has not visited carries no recorded certainty
    /// ([`ArgCertainty::Undecided`]), which [`aggregate_type_of`] refuses to read, so
    /// this walk types one from its argument instead: `count` is `bigint`, and every
    /// other aggregate is typed only where its argument is — an argument of unknown
    /// type leaves the aggregate unknown rather than guessing `avg`'s `double
    /// precision`. A decided aggregate, and every other node, is read as
    /// [`Self::infer_type`] reads it.
    pub fn infer_type_unresolved(&self, scope: &dyn LeafScope) -> Option<TypeOid> {
        if let Self::Aggregate {
            func,
            arg,
            certainty: ArgCertainty::Undecided,
            ..
        } = self
        {
            return match (func, arg.infer_type_unresolved(scope)) {
                (AggregateFunc::Count, _) => Some(TypeOid::Int8),
                (_, None) => None,
                (func, known) => aggregate_result_type(*func, known, true),
            };
        }
        self.infer_type_with(scope, &|e| e.infer_type_unresolved(scope))
    }

    /// The walk above with its **recursion supplied by the caller**: `operand` answers
    /// each direct sub-expression's type instead of `infer_type` re-deriving it.
    ///
    /// One body, so there is no second rule to drift: [`Expr::infer_type`] passes its
    /// own recursion here and is the whole walk. The hook exists for a caller that
    /// already holds its children's types — the executor's eval walk, which derives a
    /// node's type from the ones it derived for that node's operands. Reading the
    /// operand type by re-walking the operand subtree from every enclosing node made
    /// that read quadratic in expression depth (measured 6.4× at depth 32 over 163 840
    /// rows); this makes it one derivation per node.
    ///
    /// A hook that answers something other than the sub-expression's type answers for
    /// the whole node, so it is the caller's business that it does not.
    // arms grouped by `Expr` variant for documentation; several variants share a
    // body (e.g. boolean-typed expressions return `Some(TypeOid::Bool)`) but are
    // kept separate intentionally
    #[allow(clippy::match_same_arms)]
    pub fn infer_type_with<F: Fn(&Self) -> Option<TypeOid>>(
        &self,
        scope: &dyn LeafScope,
        operand: &F,
    ) -> Option<TypeOid> {
        match self {
            Self::Literal(lit) => lit.infer_type(),
            Self::Column { table, name } => scope.column_type(table.as_deref(), name),
            // One cell, two walks — see [`binary_op_result_type`]. The operands are
            // typed up front rather than per branch: the executor's eval walk holds
            // both already, and reaching the cell through the whole-`Expr` match is
            // what it exists to skip. Keep this arm a **bare** call to that function:
            // the eval walks call it directly, so a rule added here would not reach
            // them — the invariant is stated on the function.
            Self::BinaryOp { left, op, right } => {
                binary_op_result_type(*op, operand(left), operand(right))
            }
            Self::Negate(inner) | Self::UnaryPlus(inner) => operand(inner),
            Self::Not(_) => Some(TypeOid::Bool),
            Self::IsNull(_) | Self::IsNotNull(_) => Some(TypeOid::Bool),
            // Never NULL and never anything but `boolean`, for every operand
            // including a NULL one — the whole point of the three-valued test.
            Self::BooleanTest { .. } => Some(TypeOid::Bool),
            // The four cells the executor's twin walk answers identically, from the
            // one rule both read (see [`shared_leaf_cell_type`]): a bare `DEFAULT` and
            // an unbound `$N` (no type), a scalar subquery (the scope's business), and
            // `date_trunc` / `substring` / `substr` (an argument's type). This arm's patterns are the
            // classification that rule is documented against, so widening one without
            // the other is what would break it.
            Self::Default
            | Self::Parameter { .. }
            | Self::Subquery { .. }
            | Self::StringFn {
                func: StringFunc::DateTrunc | StringFunc::Substring | StringFunc::Substr,
                ..
            } => shared_leaf_cell_type(self, scope, operand),
            // The four cells that pick one operand's type out of several, from the one
            // rule both walks read (see [`shared_multi_operand_type`]) — handed this
            // walk's own operand hook.
            Self::Array(_) | Self::VariadicFn { .. } | Self::Coalesce(_) | Self::Case { .. } => {
                shared_multi_operand_type(self, operand)
            }
            Self::Cast { target_type, .. } => Some(*target_type),
            // A typmod'd cast's output type is its target (the modifier changes the
            // value, not the type).
            Self::CastTypmod { type_oid, .. } => Some(*type_oid),
            Self::Alias { expr, .. } => operand(expr),
            Self::Aggregate {
                func,
                arg,
                certainty,
                ..
            } => aggregate_type_of(*func, *certainty, operand(arg)),
            Self::MathFn { func, args } => {
                let arg_type = args.first().and_then(operand);
                Some(math_fn_result_type(*func, arg_type, args.len() >= 2))
            }
            // Strict-NULL string functions: the result type depends only on the
            // function (`string_fn_result_type`). The members whose result follows an
            // argument — `date_trunc`, `substring`, `substr` — are answered by the
            // shared arm above.
            Self::StringFn { func, .. } => Some(string_fn_result_type(*func)),
            // `EXTRACT`/`extract` yield numeric; `date_part` yields float8.
            Self::Extract { retnumeric, .. } => Some(if *retnumeric {
                TypeOid::Numeric
            } else {
                TypeOid::Float8
            }),
            Self::InSubquery { .. } | Self::InList { .. } => Some(TypeOid::Bool),
            // `[NOT] EXISTS(...)` is a boolean predicate.
            Self::Exists { .. } => Some(TypeOid::Bool),
            Self::Window { .. } => Some(TypeOid::Int8), // Window functions return Int8
            Self::Bound(v) => v.type_oid(),             // Bound param: type of its value
            // Depends only on the builtin, so both walkers read the one mapping.
            Self::Builtin(func) => Some(builtin_fn_result_type(*func)),
            // `jsonb_build_object(...)` always yields jsonb.
            Self::JsonbBuildObject(_) => Some(TypeOid::Jsonb),
            // ANY/ALL comparisons return boolean.
            Self::AnyAll { .. } => Some(TypeOid::Bool),
            // `unnest(arr)` yields the array's element type.
            Self::Unnest(inner) => operand(inner).and_then(TypeOid::array_element),
            // `LOWER(text)` is text.
            Self::Lower(_) => Some(TypeOid::Text),
            Self::ColumnSize { .. } => Some(TypeOid::Int4),
            // Every sequence function returns bigint.
            Self::SequenceFn { .. } => Some(TypeOid::Int8),
        }
    }

    /// Whether this node is itself a call 18.6 marks `VOLATILE` — a volatile
    /// [`BuiltinFn`], a sequence function, or `pg_advisory_xact_lock` — whatever its
    /// children are. The reading of volatility its readers share — the GIN probe and guard
    /// selection (`planner::index_selection`), an `ADD COLUMN` default's backfill
    /// (`executor::ddl`), and through [`Self::contains_volatile_call`] a projection's
    /// `unnest` siblings (`executor::query`) — each of which must not evaluate a call once
    /// in place of once a row.
    #[must_use]
    pub const fn is_volatile_call(&self) -> bool {
        match self {
            Self::Builtin(func) => func.is_volatile(),
            Self::SequenceFn { .. }
            | Self::StringFn {
                func: StringFunc::AdvisoryXactLock,
                ..
            } => true,
            _ => false,
        }
    }

    /// Whether a volatile call ([`Self::is_volatile_call`]) stands anywhere in this
    /// node's scalar tree. A sub-query is a leaf here, as `for_each_child` reads it: an
    /// uncorrelated one is evaluated once on 18.6 too, its `InitPlan` running once —
    /// pglet re-evaluates one for each outer row elsewhere (`EDGES.md` row 319), but a
    /// projection `unnest`'s scalar slot that is not volatile is evaluated once, which is
    /// 18.6's count.
    #[must_use]
    pub fn contains_volatile_call(&self) -> bool {
        if self.is_volatile_call() {
            return true;
        }
        let mut found = false;
        self.for_each_child(|child| found = found || child.contains_volatile_call());
        found
    }

    /// Whether this is a bare `*` projection wildcard
    /// (`Expr::Column { table: None, name: "*" }`). The single definition of the
    /// wildcard test, shared by the parser's no-FROM guard and the planner's
    /// wildcard expansion so they can't drift.
    pub fn is_bare_star(&self) -> bool {
        matches!(self, Self::Column { table: None, name } if name == "*")
    }

    /// If this is a **top-level** sequence function — a bare [`Self::SequenceFn`] or
    /// one wrapped only in an [`Self::Alias`] (`nextval('s') AS x`) — return its
    /// parts. A sequence function nested any deeper (an arithmetic operand, a cast, a
    /// subquery) yields `None`, so it is not treated as a top-level projection and
    /// fails loud when the read-only eval path reaches it.
    pub(crate) fn as_sequence_fn(&self) -> Option<(SequenceFunc, &str, &[Self])> {
        match self {
            Self::SequenceFn { func, seq, args } => Some((*func, seq.as_str(), args)),
            Self::Alias { expr, .. } => expr.as_sequence_fn(),
            _ => None,
        }
    }

    /// The arguments of a **top-level** `pg_advisory_xact_lock` call — a bare
    /// [`StringFunc::AdvisoryXactLock`] or one wrapped only in an [`Self::Alias`] — as
    /// [`Self::as_sequence_fn`] finds a top-level sequence function. A call nested any
    /// deeper yields `None`.
    pub(crate) fn as_advisory_lock(&self) -> Option<&[Self]> {
        match self {
            Self::StringFn {
                func: StringFunc::AdvisoryXactLock,
                args,
            } => Some(args),
            Self::Alias { expr, .. } => expr.as_advisory_lock(),
            _ => None,
        }
    }

    /// Get the output column name for this expression.
    // arms grouped by `Expr` variant for documentation; several variants share a
    // body (the `"?column?"` fallback name) but are kept separate intentionally
    #[allow(clippy::match_same_arms)]
    pub fn output_name(&self) -> String {
        match self {
            Self::Literal(_) => "?column?".to_string(),
            Self::Column { name, .. } => name.clone(),
            // Postgres names these projections after the function/operator when
            // unaliased (`to_char(...)` → "to_char", `... AT TIME ZONE ...` →
            // "timezone"); every other binary op stays "?column?".
            Self::BinaryOp {
                op: BinaryOp::ToChar,
                ..
            } => "to_char".to_string(),
            Self::BinaryOp {
                op: BinaryOp::AtTimeZone,
                ..
            } => "timezone".to_string(),
            // `mod(…)` / `power(…)` / `pow(…)` are named after the function written.
            Self::BinaryOp { op, .. } if op.call_name().is_some() => {
                op.call_name().unwrap_or("?column?").to_string()
            }
            Self::BinaryOp { .. } => "?column?".to_string(),
            // Postgres names unary operator/predicate projections `?column?`
            // (`NOT x`, `-x`, `x IS [NOT] NULL`), but looks *through* a cast to the
            // argument's name (`id::text` → "id"), verified against real Postgres.
            // `?column?` for the unary-plus wrapper too: Postgres names any operator
            // expression that way, so `SELECT +x` is `?column?`, not `x`.
            Self::Negate(_) | Self::UnaryPlus(_) | Self::Not(_) => "?column?".to_string(),
            Self::IsNull(_) | Self::IsNotNull(_) => "?column?".to_string(),
            Self::BooleanTest { .. } => "?column?".to_string(),
            Self::Default => "?column?".to_string(),
            // One cast over a named argument keeps that name (`id::varchar(3)` → "id");
            // over a nameless one it takes the target type's canonical internal name
            // (`1::int4` → "int4", `'a'::varchar(5)` → "varchar"). `cast_output_name`
            // holds the rule and where it parts from 18.6's.
            Self::Cast { expr, target_type } => cast_output_name(expr, *target_type),
            Self::CastTypmod { expr, type_oid, .. } => cast_output_name(expr, *type_oid),
            Self::Alias { alias, .. } => alias.clone(),
            Self::Aggregate { func, .. } => {
                let func_name = match func {
                    AggregateFunc::Count => "count",
                    AggregateFunc::Sum => "sum",
                    AggregateFunc::Avg => "avg",
                    AggregateFunc::Min => "min",
                    AggregateFunc::Max => "max",
                };
                func_name.to_string()
            }
            Self::Subquery { .. } => "?column?".to_string(),
            Self::InSubquery { .. } | Self::InList { .. } => "?column?".to_string(),
            // Postgres names an unaliased `EXISTS(...)` projection `exists`.
            Self::Exists { .. } => "exists".to_string(),
            Self::Window { func, .. } => match func {
                WindowFunc::RowNumber => "row_number".to_string(),
                WindowFunc::Rank => "rank".to_string(),
                WindowFunc::DenseRank => "dense_rank".to_string(),
            },
            Self::Parameter { index } => format!("${index}"),
            Self::Bound(_) => "?column?".to_string(),
            Self::Builtin(BuiltinFn::Now) => "now".to_string(),
            // Postgres names each `now`-family projection after the spelling written,
            // not after the value's provenance (measured on 18.6: `SELECT
            // clock_timestamp()` is column `clock_timestamp`).
            Self::Builtin(BuiltinFn::TransactionTimestamp) => "transaction_timestamp".to_string(),
            Self::Builtin(BuiltinFn::StatementTimestamp) => "statement_timestamp".to_string(),
            Self::Builtin(BuiltinFn::ClockTimestamp) => "clock_timestamp".to_string(),
            Self::Builtin(BuiltinFn::CurrentTimestamp) => "current_timestamp".to_string(),
            Self::Builtin(BuiltinFn::CurrentDate) => "current_date".to_string(),
            Self::Builtin(BuiltinFn::CurrentTime) => "current_time".to_string(),
            Self::Builtin(BuiltinFn::LocalTime) => "localtime".to_string(),
            Self::Builtin(BuiltinFn::LocalTimestamp) => "localtimestamp".to_string(),
            Self::Builtin(BuiltinFn::RandomUuid) => "gen_random_uuid".to_string(),
            // Each session form's column name is the **written** spelling on 18.6,
            // which is why `current_catalog` and `current_database()` are two
            // variants rather than one (measured: the columns are named
            // `current_catalog` and `current_database`).
            Self::Builtin(BuiltinFn::CurrentUser) => "current_user".to_string(),
            Self::Builtin(BuiltinFn::SessionUser) => "session_user".to_string(),
            Self::Builtin(BuiltinFn::User) => "user".to_string(),
            Self::Builtin(BuiltinFn::CurrentRole) => "current_role".to_string(),
            Self::Builtin(BuiltinFn::CurrentCatalog) => "current_catalog".to_string(),
            Self::Builtin(BuiltinFn::CurrentDatabase) => "current_database".to_string(),
            Self::Builtin(BuiltinFn::CurrentSchema | BuiltinFn::CurrentSchemaFn) => {
                "current_schema".to_string()
            }
            Self::Builtin(BuiltinFn::Version) => "version".to_string(),
            Self::Builtin(BuiltinFn::Random) => "random".to_string(),
            Self::Array(_) => "array".to_string(),
            Self::JsonbBuildObject(_) => "jsonb_build_object".to_string(),
            Self::AnyAll { .. } => "?column?".to_string(),
            // Postgres names an unaliased `unnest(...)` projection `unnest`.
            Self::Unnest(_) => "unnest".to_string(),
            // Postgres names these projections after the function when unaliased.
            Self::Lower(_) => "lower".to_string(),
            Self::ColumnSize { .. } => "pg_column_size".to_string(),
            Self::MathFn { func, .. } => crate::deparse::math_fn_name(*func).to_string(),
            // Postgres names these projections after the function when unaliased.
            // `POSITION(…)` keeps its own `StringFunc::Position` variant rather than
            // desugaring to `strpos`, so an unaliased one is named `position` on both
            // engines (measured) — and the deparser renders it back as `POSITION`,
            // which the round-trip invariant needs (see `deparse::render_expr`).
            // `::regclass` is a **cast**, and 18.6 keys a cast on the *strength* its
            // argument returns (`FigureColnameInternal`, `parse_target.c`): at strength 2
            // (a column reference) it passes that name through, so `conrelid::regclass` is
            // `conrelid`; at strength ≤ 1 it **substitutes its own type name** and returns
            // 1, so a cast above substitutes in turn and `'parent'::regclass::text` is
            // `text`. Modelling this cast as a `StringFn` leaked the function's name into
            // both; passing the argument's name straight through — the nameless `?column?`
            // included — makes this node transparent, so the argument's own name decides.
            //
            // What that leaves is not this arm's to close. A bare `'x'::regclass` is
            // `?column?` here and `regclass` there — the same expression whose *value*
            // already diverges (a name here, an OID there), recorded with it in
            // `EDGES.md` row 85's (a). And a **second** cast above the first disagrees:
            // `'zz'::regclass::text::varchar` is `text` here and `varchar` there,
            // measured — `cast_output_name`'s divergence rather than this arm's, and
            // `EDGES.md` row 117.
            // `::<enum>` is a cast too, and transparent for the same reason: the operand's
            // own name decides, and the enclosing `Cast` to `text` substitutes `text` for
            // a nameless one exactly as it did before this node existed.
            Self::StringFn {
                func: StringFunc::Regclass | StringFunc::EnumIn,
                args,
            } => args
                .first()
                .map_or_else(|| "regclass".to_string(), Self::output_name),
            // The deparser's spelling: `position` for the special form, as 18.6 names it.
            Self::StringFn { func, .. } => crate::deparse::string_fn_name(*func).to_string(),
            // Postgres names an unaliased `EXTRACT(...)`/`extract(...)` projection
            // `extract` and a `date_part(...)` one `date_part`.
            Self::Extract { retnumeric, .. } => {
                if *retnumeric { "extract" } else { "date_part" }.to_string()
            }
            Self::VariadicFn { func, .. } => crate::deparse::variadic_fn_name(*func).to_string(),
            Self::Coalesce(_) => "coalesce".to_string(),
            // Postgres names an unaliased `CASE … END` projection `case`.
            Self::Case { .. } => "case".to_string(),
            // Postgres names an unaliased sequence-function projection after the
            // function (`nextval`/`currval`/`setval`).
            Self::SequenceFn { func, .. } => func.name().to_string(),
        }
    }

    /// Convert this expression to a `ColumnDesc`.
    ///
    /// Builds a fresh descriptor with no wire metadata (the schema-based inference
    /// path used for a computed expression / RETURNING item); a passthrough
    /// base-table column's typmod + provenance is instead propagated by the
    /// projection walk (`project_output_columns` / `expr_column_descs`).
    ///
    /// This is [`Self::infer_type`]'s **boundary** — the `unwrap_or` that turns the
    /// walk's `Option` into a descriptor's total `TypeOid` — so it, not the walk,
    /// is where the catalog and the scope are paired into a [`PlannerScope`].
    ///
    /// # The enum identity is derived here, not left to the passthrough
    ///
    /// `ColumnDesc::enum_type` is how a gate reads a *derived relation*'s column, and the
    /// namespaces built **here** are a derived table's and a plain CTE's scope entries,
    /// the relations `Plan::output_columns_scoped` hands up, an `UPDATE … FROM` source's,
    /// `executor::subquery_enum_name`, and a `JOIN … USING` merge folding two of those.
    /// A **base table**'s entry comes from the catalog and always carried the identity; a
    /// derived one is built here, and for a *computed* projection this used to derive
    /// nothing. Those readers took that `None` for *not an enum*, so `t.m = v.z` over
    /// `(SELECT min(m) …) v` and every neighbouring spelling refused SQL 18.6 answers.
    /// Deriving it where the descriptor is built is what keeps the repair from having to
    /// reach each of those namespaces separately (`EDGES.md` rows 107 and 114).
    ///
    /// **It is not every namespace that holds one, and a sentence here saying so was
    /// false.** A *genuinely recursive* CTE's relation is built by
    /// `parser::select`'s `recursive_cte_columns`, which rebuilds the seed's descriptors
    /// to apply the `name(cols)` list — no derivation to skip, and nothing this function
    /// can reach. It carries the seed's identity on its own account; the rule that binds
    /// the two producers is `ColumnDesc`'s, not this one's. Asking *who builds this
    /// namespace* is the question that finds the next one.
    ///
    /// The walk is skipped outright for a catalog holding no enum type, which keeps it
    /// off every plan in a database where its answer could only be `None`. Its **subquery
    /// leaf declines**, because a descending one would re-enter this function beside the
    /// descent [`Self::infer_type`] is already making — measured flat against measured
    /// exponential in nesting depth, at [`PlannerScope::expr_enum_shallow`].
    ///
    /// **So a projection that *is* a scalar subquery carries no identity — `EDGES.md` row
    /// 107's residual.** `(SELECT (SELECT min(m) FROM u) FROM u) > 'sad'` is `f` here against
    /// `t` on 18.6, and its no-FROM spelling, a `Plan::Values`, likewise: that arm derives an
    /// identity only from a column its record of enclosing references holds
    /// (`EnclosingColumns`), so `SELECT id, (SELECT m) > 'sad' FROM u` answers. The
    /// derived-table spelling of the nesting (`FROM (SELECT (SELECT min(m) FROM u) AS z) v`)
    /// answers, both readers of it being reached.
    pub fn to_column_desc(&self, schema: &Schema, scope: TypeScope<'_>) -> ColumnDesc {
        let planner_scope = PlannerScope::new(schema, scope);
        let mut desc = ColumnDesc::new(
            self.output_name(),
            self.infer_type(&planner_scope).unwrap_or(TypeOid::Text),
        );
        if !schema.enums.is_empty() {
            desc.enum_type = planner_scope.expr_enum_shallow(self);
        }
        desc
    }
}

/// pglet's stand-in for 18.6's `FigureColname` cast rule: look *through* the cast to
/// the inner expression's name, but when that is the nameless `?column?` placeholder (a
/// literal / bare operator), fall back to the target type's canonical name.
///
/// **The stand-in is the `?column?` test**, and it is where the two part. 18.6 keys on
/// a *strength* (`FigureColnameInternal`, `parse_target.c`): a cast over a strength-≤ 1
/// argument substitutes its own type name and **returns 1**, so the cast above it
/// substitutes in turn and the outermost one wins. A name this function has already
/// substituted reads back as an ordinary name, so a stack of casts is named after its
/// **innermost** cast here — `'1'::int::text` is `int4` here against `text` there,
/// measured, and `EDGES.md` row 117. The strength-2 arm agrees: a named operand keeps
/// its name under a stack (`a::int::text::varchar` is `a` on both, measured).
///
/// Inlined into [`Expr::output_name`], so a stack of casts is one frame a level
/// (`crate::depth`).
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn cast_output_name(inner: &Expr, target: TypeOid) -> String {
    let name = inner.output_name();
    if name == "?column?" {
        cast_type_display_name(target).to_string()
    } else {
        name
    }
}

/// The canonical Postgres internal type name (`pg_type.typname`) a cast column
/// takes its name from — `int4`/`float8`/`varchar`/… (the spelling
/// `strVal(llast(typeName->names))` yields), not the DDL alias
/// `integer`/`double precision`. An **array** target uses its element type's name
/// (`'{1}'::int[]` → "int4"), matching Postgres's `llast(names)` (which ignores
/// the array bounds). Distinct from `type_oid_to_sql_name`, which spells the
/// round-trippable DDL form.
const fn cast_type_display_name(oid: TypeOid) -> &'static str {
    match oid {
        TypeOid::Bool | TypeOid::BoolArray => "bool",
        TypeOid::Int2 | TypeOid::Int2Array => "int2",
        TypeOid::Int4 | TypeOid::Int4Array => "int4",
        TypeOid::Int8 | TypeOid::Int8Array => "int8",
        TypeOid::Float4 | TypeOid::Float4Array => "float4",
        TypeOid::Float8 | TypeOid::Float8Array => "float8",
        TypeOid::Text | TypeOid::TextArray => "text",
        TypeOid::Varchar | TypeOid::VarcharArray => "varchar",
        TypeOid::Bytea | TypeOid::ByteaArray => "bytea",
        TypeOid::Timestamp | TypeOid::TimestampArray => "timestamp",
        TypeOid::Timestamptz | TypeOid::TimestamptzArray => "timestamptz",
        TypeOid::Date | TypeOid::DateArray => "date",
        TypeOid::Time | TypeOid::TimeArray => "time",
        TypeOid::Interval | TypeOid::IntervalArray => "interval",
        TypeOid::Numeric | TypeOid::NumericArray => "numeric",
        TypeOid::Uuid | TypeOid::UuidArray => "uuid",
        TypeOid::Jsonb | TypeOid::JsonbArray => "jsonb",
        // Unreachable: a cast target is read through `type_oid_from_name`, which
        // maps no spelling to `name` / `oid` / `"char"` / `void`, so `::name` is `42704`
        // before it gets here (the four are *declaration* types only).
        TypeOid::Name | TypeOid::NameArray => "name",
        TypeOid::Oid => "oid",
        TypeOid::Char => "char",
        TypeOid::Void => "void",
    }
}

impl Literal {
    /// Infer the type of this literal.
    ///
    /// Integer literals are typed based on value (matches Postgres behavior):
    /// - Values in i32 range → Int4
    /// - Values outside i32 range → Int8
    pub fn infer_type(&self) -> Option<TypeOid> {
        match self {
            Self::Null => None,
            Self::Bool(_) => Some(TypeOid::Bool),
            Self::Int(n) => {
                // Match literal_to_value behavior: Int4 if fits, else Int8
                if i32::try_from(*n).is_ok() {
                    Some(TypeOid::Int4)
                } else {
                    Some(TypeOid::Int8)
                }
            }
            Self::Float(_) => Some(TypeOid::Float8),
            Self::String(_) => Some(TypeOid::Text),
            Self::Numeric(_) => Some(TypeOid::Numeric),
        }
    }
}

#[cfg(test)]
mod arith_result_type_tests {
    use super::{BinaryOp, arith_result_type, int_width_result};
    use crate::types::TypeOid;

    /// Every ordered integer pair takes the **wider** operand's type.
    ///
    /// Postgres registers the mixed-width operators in both directions with the same
    /// `prorettype` (`int24pl`/`int42pl` → `int4`; `int48*`/`int84*`, `int28*`/`int82*`
    /// → `int8`), so this is symmetric. The asymmetric answer is the bug this replaced:
    /// taking the left operand declared `int2` for `int2 + int4`, a two-byte type over
    /// the four-byte value the eval path already produced, which tokio-postgres refuses
    /// with `error deserializing column`.
    #[test]
    fn integer_arithmetic_takes_the_wider_operand_in_both_orders() {
        use TypeOid::{Int2, Int4, Int8};
        for (l, r, want) in [
            (Int2, Int4, Int4),
            (Int4, Int2, Int4),
            (Int4, Int8, Int8),
            (Int8, Int4, Int8),
            (Int2, Int8, Int8),
            (Int8, Int2, Int8),
            (Int2, Int2, Int2),
            (Int4, Int4, Int4),
            (Int8, Int8, Int8),
        ] {
            assert_eq!(int_width_result(l, r), Some(want), "{l:?} vs {r:?}");
            for op in [
                BinaryOp::Add,
                BinaryOp::Sub,
                BinaryOp::Mul,
                BinaryOp::Div,
                BinaryOp::Mod,
            ] {
                assert_eq!(
                    arith_result_type(op, l, r),
                    Some(want),
                    "{l:?} {op:?} {r:?}"
                );
            }
        }
    }

    /// The integer step runs **after** the float/`NUMERIC` promotion, so a mixed pair
    /// never reaches it — `int4 + numeric` is `numeric`, not `int4`.
    #[test]
    fn the_integer_step_does_not_preempt_the_numeric_family() {
        assert_eq!(int_width_result(TypeOid::Int4, TypeOid::Numeric), None);
        assert_eq!(
            arith_result_type(BinaryOp::Add, TypeOid::Int4, TypeOid::Numeric),
            Some(TypeOid::Numeric)
        );
        assert_eq!(
            arith_result_type(BinaryOp::Add, TypeOid::Int8, TypeOid::Float4),
            Some(TypeOid::Float8)
        );
    }

    /// A non-arithmetic operand pair still declines, so each walker keeps applying its
    /// own unknown-operand fallback rather than being handed a wrong concrete type.
    #[test]
    fn a_non_numeric_pair_still_declines() {
        assert_eq!(int_width_result(TypeOid::Text, TypeOid::Int4), None);
        assert_eq!(
            arith_result_type(BinaryOp::Add, TypeOid::Text, TypeOid::Text),
            None
        );
    }

    /// Temporal arithmetic keeps precedence over both later steps.
    #[test]
    fn temporal_arithmetic_still_wins() {
        assert_eq!(
            arith_result_type(BinaryOp::Sub, TypeOid::Date, TypeOid::Date),
            Some(TypeOid::Int4)
        );
        assert_eq!(
            arith_result_type(BinaryOp::Add, TypeOid::Date, TypeOid::Int4),
            Some(TypeOid::Date)
        );
    }
}

#[cfg(test)]
mod child_visitor_tests {
    use super::{AggregateFunc, BinaryOp, BuiltinFn, Expr, Literal, OrderKey, WindowFunc};
    use crate::planner::Plan;
    use crate::types::{TypeOid, Value};

    fn col(name: &str) -> Expr {
        Expr::Column {
            table: None,
            name: name.to_string(),
        }
    }

    fn dummy_plan() -> Box<Plan> {
        Box::new(Plan::Values {
            rows: vec![],
            certain: crate::planner::CertainTypes::default(),
            enclosing: crate::planner::EnclosingColumns::default(),
        })
    }

    /// The comma-joined names of the direct children `for_each_child` yields. Every
    /// child is built as a uniquely-named `Column`, so the join captures both the set
    /// and the order.
    fn children(expr: &Expr) -> String {
        let mut out = Vec::new();
        expr.for_each_child(|c| match c {
            Expr::Column { name, .. } => out.push(name.clone()),
            other => panic!("test built a non-column child: {other:?}"),
        });
        out.join(",")
    }

    #[test]
    fn leaves_have_no_children() {
        for leaf in [
            Expr::Literal(Literal::Null),
            col("x"),
            Expr::Default,
            Expr::Parameter { index: 1 },
            Expr::Bound(Value::Null),
            Expr::Builtin(BuiltinFn::Now),
            // A self-contained sub-`Plan` is a leaf — never descended into.
            Expr::Subquery { plan: dummy_plan() },
            Expr::Exists {
                plan: dummy_plan(),
                negated: false,
            },
        ] {
            assert_eq!(children(&leaf), "", "{leaf:?} should be a leaf");
        }
    }

    #[test]
    fn single_child_compounds_yield_their_one_child() {
        // `InSubquery` yields only its scalar `expr`, never the plan side.
        let cases = [
            Expr::Negate(Box::new(col("a"))),
            Expr::Not(Box::new(col("a"))),
            Expr::IsNull(Box::new(col("a"))),
            Expr::IsNotNull(Box::new(col("a"))),
            Expr::Cast {
                expr: Box::new(col("a")),
                target_type: TypeOid::Text,
            },
            Expr::Alias {
                expr: Box::new(col("a")),
                alias: "z".to_string(),
            },
            Expr::Aggregate {
                func: AggregateFunc::Count,
                arg: Box::new(col("a")),
                distinct: false,
                certainty: crate::planner::ArgCertainty::NotCertainInteger,
            },
            Expr::Unnest(Box::new(col("a"))),
            Expr::Lower(Box::new(col("a"))),
            Expr::InSubquery {
                expr: Box::new(col("a")),
                plan: dummy_plan(),
                negated: false,
            },
        ];
        for e in cases {
            assert_eq!(children(&e), "a", "{e:?}");
        }
    }

    #[test]
    fn two_child_compounds_yield_left_then_right() {
        let bin = Expr::BinaryOp {
            left: Box::new(col("l")),
            op: BinaryOp::Eq,
            right: Box::new(col("r")),
        };
        assert_eq!(children(&bin), "l,r");
        let any_all = Expr::AnyAll {
            left: Box::new(col("l")),
            op: BinaryOp::Eq,
            right: Box::new(col("r")),
            all: false,
            sub_select: false,
        };
        assert_eq!(children(&any_all), "l,r");
    }

    #[test]
    fn extract_yields_field_then_source() {
        // Source order, **against** the struct's declaration order (`source` is
        // declared first, and cannot be moved without changing the postcard encoding
        // of a stored `Expr`). Both spellings that build this node read field-first —
        // `EXTRACT(field FROM source)` and `date_part(field, source)` — and Postgres's
        // own parse node is field-first too (`gram.y`'s `extract_list`).
        //
        // Load-bearing, not cosmetic: `executor::ddl`'s stored-expression walk reports
        // the *positionally first* failing child, and `date_part` accepts an arbitrary
        // expression in either slot, so the wrong order flips the reported SQLSTATE in
        // both directions (`tests/fixtures/ddl/check_date_part_*`).
        for retnumeric in [false, true] {
            let extract = Expr::Extract {
                source: Box::new(col("src")),
                field: Box::new(col("fld")),
                retnumeric,
            };
            assert_eq!(children(&extract), "fld,src");
            // `map_children` must visit in the same order, or the two traversals
            // disagree about what "first" means.
            assert_eq!(children(&extract.map_children(bang)), "fld!,src!");
        }
    }

    #[test]
    fn list_compounds_yield_every_element() {
        let in_list = Expr::InList {
            expr: Box::new(col("e")),
            list: vec![col("a"), col("b")],
            negated: false,
        };
        assert_eq!(children(&in_list), "e,a,b");
        assert_eq!(children(&Expr::Array(vec![col("a"), col("b")])), "a,b");
        assert_eq!(
            children(&Expr::JsonbBuildObject(vec![col("k"), col("v")])),
            "k,v"
        );
        assert_eq!(children(&Expr::Coalesce(vec![col("a"), col("b")])), "a,b");
    }

    #[test]
    fn case_yields_operand_then_branches_then_else() {
        let case = Expr::Case {
            operand: Some(Box::new(col("op"))),
            when_clauses: vec![(col("c1"), col("r1")), (col("c2"), col("r2"))],
            else_result: Some(Box::new(col("el"))),
        };
        assert_eq!(children(&case), "op,c1,r1,c2,r2,el");
        // Searched form (no operand) and no ELSE.
        let searched = Expr::Case {
            operand: None,
            when_clauses: vec![(col("c1"), col("r1"))],
            else_result: None,
        };
        assert_eq!(children(&searched), "c1,r1");
    }

    #[test]
    fn window_yields_partition_then_order() {
        let win = Expr::Window {
            func: WindowFunc::RowNumber,
            partition_by: vec![col("p1"), col("p2")],
            order_by: vec![
                OrderKey::new(col("o1"), true),
                OrderKey::new(col("o2"), false),
            ],
        };
        assert_eq!(children(&win), "p1,p2,o1,o2");
    }

    /// A child transform appending `!` to a column's name; leaves everything else.
    /// A `fn` item (not a closure) so it stays `Copy` and is reusable across calls.
    fn bang(e: Expr) -> Expr {
        match e {
            Expr::Column { table, name } => Expr::Column {
                table,
                name: format!("{name}!"),
            },
            other => other,
        }
    }

    #[test]
    fn map_children_identity_is_a_structural_round_trip() {
        // Mapping every direct child through the identity rebuilds an equal tree,
        // proving each variant's non-`Expr` fields and children survive the rebuild.
        let nested = Expr::Case {
            operand: Some(Box::new(col("op"))),
            when_clauses: vec![(
                Expr::BinaryOp {
                    left: Box::new(col("a")),
                    op: BinaryOp::Eq,
                    right: Box::new(col("b")),
                },
                Expr::Coalesce(vec![col("c"), col("d")]),
            )],
            else_result: Some(Box::new(Expr::Array(vec![col("e")]))),
        };
        assert_eq!(nested.clone().map_children(|c| c), nested);
    }

    #[test]
    fn map_children_applies_f_to_every_child_in_order() {
        // Each direct child is mapped (here, renamed) in `for_each_child` order.
        let case = Expr::Case {
            operand: Some(Box::new(col("op"))),
            when_clauses: vec![(col("c1"), col("r1")), (col("c2"), col("r2"))],
            else_result: Some(Box::new(col("el"))),
        };
        assert_eq!(
            children(&case.map_children(bang)),
            "op!,c1!,r1!,c2!,r2!,el!"
        );
        let win = Expr::Window {
            func: WindowFunc::RowNumber,
            partition_by: vec![col("p1")],
            order_by: vec![OrderKey::new(col("o1"), true)],
        };
        assert_eq!(children(&win.map_children(bang)), "p1!,o1!");
        // A leaf is returned unchanged.
        assert_eq!(Expr::Default.map_children(bang), Expr::Default);
    }
}

#[cfg(test)]
mod cast_output_name_tests {
    use super::{Expr, Literal, Typmod};
    use crate::types::TypeOid;

    // Returns boxed because every `Expr` child slot is a `Box<Expr>`; unboxing here
    // would just move the `Box::new` to each call site.
    #[allow(clippy::unnecessary_box_returns)]
    fn lit_int(n: i64) -> Box<Expr> {
        Box::new(Expr::Literal(Literal::Int(n)))
    }

    #[test]
    fn cast_of_nameless_literal_takes_target_type_name() {
        // Postgres FigureColname: an unaliased cast of a nameless operand is named
        // for the target type's canonical internal name, not "?column?".
        let cases = [
            (TypeOid::Int4, "int4"),
            (TypeOid::Int8, "int8"),
            (TypeOid::Float8, "float8"),
            (TypeOid::Text, "text"),
            (TypeOid::Numeric, "numeric"),
            (TypeOid::Varchar, "varchar"),
            (TypeOid::Bool, "bool"),
        ];
        for (target, name) in cases {
            let cast = Expr::Cast {
                expr: lit_int(1),
                target_type: target,
            };
            assert_eq!(cast.output_name(), name, "{target:?}");
        }
    }

    #[test]
    fn cast_typmod_of_nameless_literal_takes_target_type_name() {
        // `9::numeric(10,2)` → "numeric", `'a'::varchar(5)` → "varchar".
        let numeric = Expr::CastTypmod {
            expr: lit_int(9),
            type_oid: TypeOid::Numeric,
            typmod: Typmod::Numeric {
                precision: 10,
                scale: 2,
            },
        };
        assert_eq!(numeric.output_name(), "numeric");
        let varchar = Expr::CastTypmod {
            expr: Box::new(Expr::Literal(Literal::String("a".into()))),
            type_oid: TypeOid::Varchar,
            typmod: Typmod::Length(5),
        };
        assert_eq!(varchar.output_name(), "varchar");
    }

    #[test]
    fn cast_of_named_column_keeps_the_column_name() {
        // A cast of a NAMED operand looks *through* the cast to the column's name.
        let cast = Expr::Cast {
            expr: Box::new(Expr::Column {
                table: None,
                name: "x".into(),
            }),
            target_type: TypeOid::Int4,
        };
        assert_eq!(cast.output_name(), "x");
    }

    #[test]
    fn array_cast_of_nameless_literal_takes_element_type_name() {
        // A NAMELESS operand cast to an array type takes the element type's name
        // (`'{1}'::int[]` → "int4"); `strVal(llast(names))` ignores the array
        // bounds. (A named `ARRAY[…]` constructor instead keeps "array", looked
        // through — matching `SELECT ARRAY[1]::int[]` → "array" in Postgres.)
        let cast = Expr::Cast {
            expr: Box::new(Expr::Literal(Literal::String("{1}".into()))),
            target_type: TypeOid::Int4Array,
        };
        assert_eq!(cast.output_name(), "int4");
    }
}

#[cfg(test)]
mod plan_is_not_persistable {
    use super::{Expr, Literal, deserialize_expr, encode_expr, serialize_expr};
    use crate::planner::Plan;

    /// The three `Expr` variants that box a [`Plan`], each over a trivial sub-plan —
    /// plus one **nested** row, where the plan-bearing node is not the tree's root.
    ///
    /// The nested row is what makes the guards below about the *walk* rather than about
    /// a top-level `matches!`: with `expr_contains_subquery`'s recursion replaced by
    /// `false`, a top-level-only corpus leaves every guard here green and only
    /// `parser::ddl`'s own gate tests redden — so the backstop's guard would be
    /// inheriting its coverage from the conformance gate it is supposed to stand behind.
    fn plan_bearing() -> Vec<(&'static str, Expr)> {
        let plan = || {
            Box::new(Plan::Values {
                rows: vec![vec![]],
                certain: crate::planner::CertainTypes::default(),
                enclosing: crate::planner::EnclosingColumns::default(),
            })
        };
        vec![
            ("Subquery", Expr::Subquery { plan: plan() }),
            (
                "InSubquery",
                Expr::InSubquery {
                    expr: Box::new(Expr::Literal(Literal::Int(1))),
                    plan: plan(),
                    negated: false,
                },
            ),
            (
                "Exists",
                Expr::Exists {
                    plan: plan(),
                    negated: true,
                },
            ),
            (
                "Not(Exists)",
                Expr::Not(Box::new(Expr::Exists {
                    plan: plan(),
                    negated: false,
                })),
            ),
        ]
    }

    /// [`serialize_expr`] — the persistence-bound encoder — **refuses** a plan-bearing
    /// `Expr`: each of the three plan-boxing variants at the root, and one nested.
    ///
    /// The write half of "a `Plan` is never persisted". It is a backstop, not the
    /// defence: every DDL route that persists an `Expr` refuses a subquery **at least
    /// twice** before the first catalog write — the node walk with Postgres's own `0A000` (the
    /// fixtures under `tests/fixtures/ddl/` pin those), then the stored-expression
    /// cook's probe eval. This fires only if both go missing, and it stands in for
    /// **no** Postgres error, so its message is pglet's.
    ///
    /// What it does not look at: whether a *reader* would refuse the bytes (that is
    /// `plan_bearing_expr_bytes_are_not_decodable`), and how much of the persistence
    /// class this reaches — one site, the storage `IndexSpec` copy, since
    /// `executor::index_spec_from_catalog` is all that reaches it. CHECK, DEFAULT and
    /// every route's catalog copy have no write-side refusal at all.
    #[test]
    fn serialize_expr_refuses_a_plan_bearing_expr() {
        for (name, expr) in plan_bearing() {
            let err = serialize_expr(&expr)
                .expect_err(&format!("Expr::{name} must not serialize into the store"));
            let message = err.to_string();
            assert!(
                message.contains("sub-plan"),
                "the refusal must name why, got: {message}"
            );
        }
    }

    /// A plan-bearing `Expr` does not survive a **decode**, even from bytes that were
    /// legitimately produced.
    ///
    /// The read half, and the one that closes the class by construction: `Plan`'s
    /// `Deserialize` always fails, so every persisted-bytes reader inherits the refusal
    /// — [`deserialize_expr`] here, and the `postcard::from_bytes::<Schema>` sites in
    /// `pglet`, `pglet_wasm`, `pglet_bench` and `pglet_testing`, since a `Schema`
    /// reaches `Expr` through stored `CHECK` / `DEFAULT` / index trees. The bytes are
    /// made with [`encode_expr`], the ungated encoder `structural_identity` uses, which
    /// is the only way such bytes can exist at all.
    ///
    /// What it does not look at: which reader is asking. That is the point — the refusal
    /// is on the type, so there is no reader enumeration to keep current. Nor does it
    /// look at the *reason* the message gives — it cannot: postcard's `Error` has no
    /// payload, so it renders every `serde::de::Error::custom` as the same
    /// "Serde Deserialization Error" and `PLAN_NOT_PERSISTED`'s text never reaches a
    /// caller. A store carrying plan bytes therefore reports as ordinary corruption,
    /// which is what it is.
    #[test]
    fn plan_bearing_expr_bytes_are_not_decodable() {
        for (name, expr) in plan_bearing() {
            let bytes = encode_expr(&expr).expect("the ungated encoder accepts a plan");
            let err = deserialize_expr(&bytes)
                .expect_err(&format!("Expr::{name} must not decode back out of a store"));
            let message = err.to_string();
            assert!(
                message.contains("expr deserialize failed"),
                "the refusal must report a failed decode, got: {message}"
            );
        }
    }

    /// The other polarity: a **plan-free** `Expr` still serializes and round-trips
    /// through the persistence path unchanged.
    ///
    /// Without this, a `serialize_expr` that refused everything would pass the two tests
    /// above. The wide version of this — the whole value corpus, byte-stable across a
    /// re-encode — is `serialize_expr_determinism`'s
    /// `serialize_expr_survives_reserialization_byte_stable`; this is the narrow
    /// statement that the gate itself lets an ordinary tree past.
    #[test]
    fn a_plan_free_expr_still_round_trips() {
        let expr = Expr::Not(Box::new(Expr::Column {
            table: Some("t".to_string()),
            name: "c".to_string(),
        }));
        let bytes = serialize_expr(&expr).expect("a plan-free Expr serializes");
        assert_eq!(
            deserialize_expr(&bytes).expect("a plan-free Expr deserializes"),
            expr
        );
    }
}

#[cfg(test)]
mod postcard_layout {
    use super::{
        AggregateFunc, BinaryOp, BoolTest, BuiltinFn, Expr, Literal, MathFunc, SequenceFunc,
        StringFunc, Typmod, VariadicFunc, WindowFunc,
    };
    use crate::types::TypeOid;

    /// A `date` / `timestamp` value persists as the Postgres binary format's count, the
    /// infinity sentinels included, and reads back as the value written — an instant
    /// written at another offset reads back at UTC. The store magic moved to `PGMO` with
    /// this encoding; `double_meta`'s `stale_format_fails_loud` refuses a `PGMN` store.
    #[test]
    fn temporal_values_persist_as_their_binary_count() {
        use crate::types::{Infinite, PgDate, PgTimestamp, Value};
        use time::macros::{date, datetime};
        for d in [
            PgDate::NegInfinity,
            PgDate::Finite(date!(0001 - 01 - 01)),
            PgDate::Finite(date!(2000 - 01 - 01)),
            PgDate::Infinity,
        ] {
            let bytes = postcard::to_allocvec(&d).expect("serializes");
            assert_eq!(bytes, postcard::to_allocvec(&d.to_pg_days()).expect("i32"));
            assert_eq!(postcard::from_bytes::<PgDate>(&bytes).expect("reads"), d);
        }
        let local = PgTimestamp::Finite(datetime!(2026-06-15 22:00:00 -5));
        for ts in [PgTimestamp::NegInfinity, local, PgTimestamp::Infinity] {
            let bytes = postcard::to_allocvec(&Value::Timestamp(ts)).expect("serializes");
            let Value::Timestamp(back) = postcard::from_bytes::<Value>(&bytes).expect("reads")
            else {
                panic!("a timestamp")
            };
            assert_eq!(back, ts);
            if let Infinite::Finite(t) = back {
                assert!(t.offset().is_utc());
            }
        }
        // A count `time` cannot hold is a decode error, never a value.
        let unreadable = postcard::to_allocvec(&(i32::MAX - 1)).expect("i32");
        assert!(postcard::from_bytes::<PgDate>(&unreadable).is_err());
    }

    /// The postcard variant tag `expr` encodes to — its leading byte.
    fn tag(expr: &Expr) -> u8 {
        postcard::to_allocvec(expr).expect("Expr serializes")[0]
    }

    /// Pins the postcard **variant discriminants** of [`Expr`].
    ///
    /// `Expr` is persisted into the catalog — CHECK predicates, index key
    /// expressions, index partial predicates, all postcard-encoded — and a plain
    /// serde-derived enum encodes its variant as a **positional** index. So the tags
    /// are on-disk format, not an implementation detail: inserting a variant anywhere
    /// but the end shifts every later variant's tag by one, and an existing store
    /// then decodes each persisted expression as *the wrong variant* — silently,
    /// where the shifted variant's payload happens to parse.
    ///
    /// Six variants' comments already say "appended last so existing postcard-encoded
    /// `Expr` IR keeps its variant discriminants"; until this test there was nothing
    /// enforcing it. The last variant's tag is the load-bearing assertion — a
    /// mid-enum insertion *anywhere* shifts it — and the anchors localize the break.
    /// So **appending a variant means moving that row onto the new last one**, not just
    /// adding an anchor: left pointing at the old last variant it still passes, and the
    /// slot between the two goes unguarded.
    ///
    /// **If this test fails**, you inserted (or removed, or reordered) an `Expr`
    /// variant somewhere other than the end. Move it to the end of the enum and
    /// update the `UnaryPlus` expectation below to the new final index. Do **not**
    /// just edit the numbers to match: every pre-existing database on disk carries
    /// the old tags, and pglet has no `Expr`-level migration — a store written before
    /// the shift would silently misdecode. (The store magic in `catalog::schema` is
    /// the escape hatch: bumping it makes old stores fail loud on reopen instead.)
    ///
    /// [`crate::types::TypeOid`] deliberately does *not* work this way — it keys its
    /// encoding on the numeric OID precisely so it can be reordered freely; see
    /// `typeoid_postcard_encodes_as_oid`. `Expr` cannot borrow that trick (its
    /// variants have no external numbering), so it gets this pin instead.
    #[test]
    fn expr_postcard_variant_tags_are_pinned() {
        let lit = Expr::Literal(Literal::Int(1));

        // Anchors across the enum — first, and a spread of mid-enum variants that
        // together localize where an insertion happened.
        assert_eq!(tag(&lit), 0, "Expr::Literal must stay the first variant");
        assert_eq!(tag(&Expr::Default), 7, "Expr::Default tag shifted");
        assert_eq!(
            tag(&Expr::Cast {
                expr: Box::new(lit.clone()),
                target_type: TypeOid::Int4,
            }),
            8,
            "Expr::Cast tag shifted"
        );
        // The three plan-bearing variants. Nothing can persist one any more (see
        // `plan_is_not_persistable`), so their *own* tags carry no store, but they sit
        // mid-enum: moving them — the shape a "just relocate the subquery variants"
        // change would take — shifts every later variant's tag and misdecodes every
        // stored CHECK and index expression.
        assert_eq!(
            tag(&Expr::Subquery {
                plan: Box::new(crate::planner::Plan::Values {
                    rows: vec![],
                    certain: crate::planner::CertainTypes::default(),
                    enclosing: crate::planner::EnclosingColumns::default(),
                }),
            }),
            11,
            "Expr::Subquery tag shifted"
        );
        assert_eq!(tag(&Expr::Array(vec![])), 19, "Expr::Array tag shifted");
        assert_eq!(
            tag(&Expr::Case {
                operand: None,
                when_clauses: vec![],
                else_result: None,
            }),
            25,
            "Expr::Case tag shifted"
        );

        assert_eq!(
            tag(&Expr::UnaryPlus(Box::new(lit.clone()))),
            32,
            "Expr::UnaryPlus tag shifted"
        );

        // The guard that catches *every* mid-enum insertion: the final variant's
        // tag is one less than the variant count, so it moves whenever anything is
        // added ahead of it. Keep this row on whatever the last variant is — an
        // appended variant that leaves it pointing at the previous last one still
        // passes, and opens a one-slot window where an insertion goes unseen.
        assert_eq!(
            tag(&Expr::BooleanTest {
                expr: Box::new(lit.clone()),
                test: BoolTest::True,
            }),
            33,
            "Expr::BooleanTest tag shifted"
        );
        assert_eq!(
            tag(&Expr::ColumnSize {
                arg: Box::new(lit.clone()),
                origin: super::ColumnSizeOrigin::Unclassified,
            }),
            34,
            "Expr::ColumnSize is no longer the last variant (or a variant was \
             inserted mid-enum) — see this test's doc comment before changing this \
             number; persisted CHECK/index expressions decode by position"
        );

        // And the tags actually round-trip, so the pin is over real encode/decode
        // and not just a leading-byte coincidence.
        for expr in [lit.clone(), Expr::Default, Expr::UnaryPlus(Box::new(lit))] {
            let bytes = postcard::to_allocvec(&expr).expect("Expr serializes");
            assert_eq!(
                postcard::from_bytes::<Expr>(&bytes).expect("Expr deserializes"),
                expr
            );
        }
    }

    /// One enum's every-variant tag pin: each listed value must encode with the listed
    /// leading byte.
    fn pin_tags<T: serde::Serialize + core::fmt::Debug>(enum_name: &str, pins: &[(u8, T)]) {
        for (expected, value) in pins {
            let bytes = postcard::to_allocvec(value).expect("variant serializes");
            assert_eq!(
                bytes[0], *expected,
                "{enum_name}::{value:?} no longer encodes as postcard variant {expected} \
                 — see this test's doc comment; persisted CHECK, DEFAULT and index \
                 expressions decode by position"
            );
        }
    }

    /// Pins the postcard variant discriminants of **every enum reachable from a
    /// persisted [`Expr`]**, each variant listed, plus a compile-time length guard.
    ///
    /// [`expr_postcard_variant_tags_are_pinned`] pins `Expr`'s own tags and sees nothing
    /// below them: every member of a *payload* enum encodes under one `Expr` tag, so
    /// exchanging two neighbours in [`BinaryOp`] shifts nothing that test measures and
    /// turns a stored `CHECK (a > 5)` into `CHECK (a < 5)` — with the store magic
    /// unmoved and every gate green. [`BuiltinFn`] had the only such pin; this is the
    /// rest of the set.
    ///
    /// **The set is derived, not inherited.** It is every enum the persisted `Expr`
    /// type graph reaches — which is not the same as every enum declared beside it.
    /// [`ExtractField`], [`SqlValueFunction`] and [`SetQuantifier`] are *not* in it and
    /// must not be added: none derives `Serialize`, so none can appear in the bytes
    /// (`Expr::Extract` carries its field as a text expression resolved at eval time,
    /// and the other two never reach an `Expr` field at all). Conversely the set leaves
    /// this crate — `Value`, `JsonbValue` and `PgNumeric` live in `pglet_core` and are
    /// reached through `Expr::Bound` and `Literal::Numeric`. Two members are pinned
    /// elsewhere and are deliberately absent here: `BuiltinFn` (above) and
    /// [`crate::types::TypeOid`], which keys its encoding on the numeric OID rather than
    /// on position and is pinned by `catalog::schema`'s `typeoid_postcard_encodes_as_oid`
    /// — the one member of the graph that *is* free to be reordered.
    ///
    /// **What this test does not look at:** a *struct's* field order, which is on-disk
    /// format by the same argument and has no pin — `OrderKey`, `ArrayValue`,
    /// `PgInterval`, `NumericVar` and `Expr`'s own struct-shaped variants all encode
    /// their fields positionally, and reordering two same-typed fields would misdecode
    /// silently. Nothing here would notice.
    ///
    /// **If this test fails**, you reordered or removed a variant. Move it back; a new
    /// one goes at the end with a new row. Do not renumber to match — every store on
    /// disk carries the old tags — bump the store magic instead.
    #[test]
    fn expr_sub_enum_postcard_variant_tags_are_pinned() {
        use crate::types::{ArrayValue, JsonbValue, PgInterval, PgNumeric, Value};
        use time::macros::{date, datetime};

        let num = || PgNumeric::parse("1.5").expect("numeric parses");

        pin_tags(
            "BoolTest",
            &[
                (0, BoolTest::True),
                (1, BoolTest::NotTrue),
                (2, BoolTest::False),
                (3, BoolTest::NotFalse),
                (4, BoolTest::Unknown),
                (5, BoolTest::NotUnknown),
            ],
        );
        pin_tags(
            "Typmod",
            &[
                (
                    0,
                    Typmod::Numeric {
                        precision: 1,
                        scale: 0,
                    },
                ),
                (1, Typmod::Length(1)),
                (2, Typmod::Precision(3)),
            ],
        );
        pin_tags(
            "SequenceFunc",
            &[
                (0, SequenceFunc::Nextval),
                (1, SequenceFunc::Currval),
                (2, SequenceFunc::Setval),
            ],
        );
        pin_tags(
            "AggregateFunc",
            &[
                (0, AggregateFunc::Count),
                (1, AggregateFunc::Sum),
                (2, AggregateFunc::Avg),
                (3, AggregateFunc::Min),
                (4, AggregateFunc::Max),
            ],
        );
        pin_tags(
            "MathFunc",
            &[
                (0, MathFunc::Abs),
                (1, MathFunc::Ceil),
                (2, MathFunc::Floor),
                (3, MathFunc::Round),
                (4, MathFunc::Trunc),
                (5, MathFunc::Sqrt),
                (6, MathFunc::Sign),
            ],
        );
        pin_tags(
            "StringFunc",
            &[
                (0, StringFunc::Upper),
                (1, StringFunc::Length),
                (2, StringFunc::Replace),
                (3, StringFunc::Substring),
                (4, StringFunc::Ltrim),
                (5, StringFunc::Rtrim),
                (6, StringFunc::Btrim),
                (7, StringFunc::Strpos),
                (8, StringFunc::Lpad),
                (9, StringFunc::Rpad),
                (10, StringFunc::SplitPart),
                (11, StringFunc::Position),
                (12, StringFunc::DateTrunc),
                (13, StringFunc::CurrentSetting),
                (14, StringFunc::CurrentSchemas),
                (15, StringFunc::PgGetConstraintdef),
                (16, StringFunc::Regclass),
                (17, StringFunc::IsFinite),
                (18, StringFunc::ColDescription),
                (19, StringFunc::AdvisoryXactLock),
                (20, StringFunc::QuoteIdent),
                (21, StringFunc::PgGetSerialSequence),
                (22, StringFunc::HasSchemaPrivilege),
                (23, StringFunc::EnumIn),
                (24, StringFunc::Md5),
                (25, StringFunc::Left),
                (26, StringFunc::Right),
                (27, StringFunc::Substr),
                (28, StringFunc::OctetLength),
            ],
        );
        pin_tags(
            "VariadicFunc",
            &[
                (0, VariadicFunc::Concat),
                (1, VariadicFunc::ConcatWs),
                (2, VariadicFunc::Greatest),
                (3, VariadicFunc::Least),
                (4, VariadicFunc::NullIf),
            ],
        );
        pin_tags(
            "WindowFunc",
            &[
                (0, WindowFunc::RowNumber),
                (1, WindowFunc::Rank),
                (2, WindowFunc::DenseRank),
            ],
        );
        pin_tags(
            "Literal",
            &[
                (0, Literal::Null),
                (1, Literal::Bool(true)),
                (2, Literal::Int(1)),
                (3, Literal::Float(1.0)),
                (4, Literal::String("s".to_string())),
                (5, Literal::Numeric(num())),
            ],
        );
        pin_tags(
            "BinaryOp",
            &[
                (0, BinaryOp::Eq),
                (1, BinaryOp::Ne),
                (2, BinaryOp::Lt),
                (3, BinaryOp::Le),
                (4, BinaryOp::Gt),
                (5, BinaryOp::Ge),
                (6, BinaryOp::And),
                (7, BinaryOp::Or),
                (8, BinaryOp::Add),
                (9, BinaryOp::Sub),
                (10, BinaryOp::Mul),
                (11, BinaryOp::Div),
                (12, BinaryOp::Concat),
                (13, BinaryOp::Like),
                (14, BinaryOp::NotLike),
                (15, BinaryOp::JsonGet),
                (16, BinaryOp::JsonGetText),
                (17, BinaryOp::Containment),
                (18, BinaryOp::ContainedBy),
                (19, BinaryOp::StartsWith),
                (20, BinaryOp::AtTimeZone),
                (21, BinaryOp::ToChar),
                (22, BinaryOp::IsDistinctFrom),
                (23, BinaryOp::IsNotDistinctFrom),
                (24, BinaryOp::Ilike),
                (25, BinaryOp::NotIlike),
                (26, BinaryOp::Mod),
                (27, BinaryOp::Pow),
                (28, BinaryOp::JsonExists),
                (29, BinaryOp::JsonExistsAny),
                (30, BinaryOp::JsonExistsAll),
                (31, BinaryOp::JsonPathGet),
                (32, BinaryOp::JsonPathGetText),
                (33, BinaryOp::AndList),
                (34, BinaryOp::OrList),
                (35, BinaryOp::AndListInner),
                (36, BinaryOp::OrListInner),
                (37, BinaryOp::RegexMatch),
                (38, BinaryOp::NotRegexMatch),
                (39, BinaryOp::ModFn),
                (40, BinaryOp::PowerFn),
                (41, BinaryOp::PowFn),
            ],
        );

        // The `pglet_core` half of the graph, reached through `Expr::Bound` and
        // `Literal::Numeric`.
        pin_tags(
            "Value",
            &[
                (0, Value::Null),
                (1, Value::Bool(true)),
                (2, Value::Int2(1)),
                (3, Value::Int4(1)),
                (4, Value::Int8(1)),
                (5, Value::Float4(1.0)),
                (6, Value::Float8(1.0)),
                (7, Value::Text("s".to_string())),
                (8, Value::Bytea(vec![0])),
                (9, Value::timestamp(datetime!(2024-03-04 05:06:07 UTC))),
                (10, Value::Jsonb(JsonbValue::Null)),
                (11, Value::Uuid([0; 16])),
                (12, Value::Array(ArrayValue::new(TypeOid::Text, vec![]))),
                (
                    13,
                    Value::Interval(PgInterval {
                        months: 1,
                        days: 1,
                        micros: 1,
                    }),
                ),
                (14, Value::date(date!(2024 - 03 - 04))),
                (15, Value::Numeric(num())),
                (16, Value::Time(1)),
            ],
        );
        pin_tags(
            "JsonbValue",
            &[
                (0, JsonbValue::Null),
                (1, JsonbValue::Bool(true)),
                (2, JsonbValue::Number(num())),
                (3, JsonbValue::String("s".to_string())),
                (4, JsonbValue::Array(vec![])),
                (5, JsonbValue::Object(vec![])),
            ],
        );
        pin_tags("PgNumeric", &[(0, PgNumeric::NaN), (1, num())]);
    }

    /// The **length** half of [`expr_sub_enum_postcard_variant_tags_are_pinned`], as a
    /// compile error rather than an assertion.
    ///
    /// A tag list cannot notice a variant *appended* without a row — the rows it has
    /// still pass. These matches are exhaustive, so adding a variant to any enum in the
    /// set fails to compile here, and the fix is to add its row above. `BuiltinFn` is in
    /// the matches below though not in this module's shared pin list: its tags are
    /// pinned by its own test, whose trailing length anchor was deleted as a restatement
    /// of its last row rather than an append guard.
    ///
    /// What this buys is not the append itself — postcard tolerates that and no store
    /// carries the new tag — but the one-slot window it would otherwise leave open
    /// against a **later** mid-enum insertion, which is
    /// [`expr_postcard_variant_tags_are_pinned`]'s last-variant argument one level down.
    #[test]
    #[allow(clippy::match_same_arms)]
    fn every_pinned_sub_enum_variant_has_a_row() {
        use crate::types::{JsonbValue, PgNumeric, Value};

        // Each match names every variant; a new one is a compile error, not a green run.
        fn _bool_test(v: BoolTest) {
            match v {
                BoolTest::True
                | BoolTest::NotTrue
                | BoolTest::False
                | BoolTest::NotFalse
                | BoolTest::Unknown
                | BoolTest::NotUnknown => (),
            }
        }
        fn _builtin_fn(v: BuiltinFn) {
            match v {
                BuiltinFn::Now
                | BuiltinFn::RandomUuid
                | BuiltinFn::CurrentDate
                | BuiltinFn::CurrentTime
                | BuiltinFn::LocalTime
                | BuiltinFn::LocalTimestamp
                | BuiltinFn::TransactionTimestamp
                | BuiltinFn::StatementTimestamp
                | BuiltinFn::ClockTimestamp
                | BuiltinFn::CurrentTimestamp
                | BuiltinFn::CurrentUser
                | BuiltinFn::SessionUser
                | BuiltinFn::User
                | BuiltinFn::CurrentRole
                | BuiltinFn::CurrentCatalog
                | BuiltinFn::CurrentDatabase
                | BuiltinFn::CurrentSchema
                | BuiltinFn::CurrentSchemaFn
                | BuiltinFn::Version
                | BuiltinFn::Random => (),
            }
        }
        fn _typmod(v: Typmod) {
            match v {
                Typmod::Numeric { .. } | Typmod::Length(_) | Typmod::Precision(_) => (),
            }
        }
        fn _sequence_func(v: SequenceFunc) {
            match v {
                SequenceFunc::Nextval | SequenceFunc::Currval | SequenceFunc::Setval => (),
            }
        }
        fn _aggregate_func(v: AggregateFunc) {
            match v {
                AggregateFunc::Count
                | AggregateFunc::Sum
                | AggregateFunc::Avg
                | AggregateFunc::Min
                | AggregateFunc::Max => (),
            }
        }
        fn _math_func(v: MathFunc) {
            match v {
                MathFunc::Abs
                | MathFunc::Ceil
                | MathFunc::Floor
                | MathFunc::Round
                | MathFunc::Trunc
                | MathFunc::Sqrt
                | MathFunc::Sign => (),
            }
        }
        fn _string_func(v: StringFunc) {
            match v {
                StringFunc::Upper
                | StringFunc::Length
                | StringFunc::Replace
                | StringFunc::Substring
                | StringFunc::Ltrim
                | StringFunc::Rtrim
                | StringFunc::Btrim
                | StringFunc::Strpos
                | StringFunc::Lpad
                | StringFunc::Rpad
                | StringFunc::SplitPart
                | StringFunc::Position
                | StringFunc::DateTrunc
                | StringFunc::CurrentSetting
                | StringFunc::CurrentSchemas
                | StringFunc::PgGetConstraintdef
                | StringFunc::Regclass
                | StringFunc::IsFinite
                | StringFunc::ColDescription
                | StringFunc::AdvisoryXactLock
                | StringFunc::QuoteIdent
                | StringFunc::PgGetSerialSequence
                | StringFunc::HasSchemaPrivilege
                | StringFunc::EnumIn
                | StringFunc::Md5
                | StringFunc::Left
                | StringFunc::Right
                | StringFunc::Substr
                | StringFunc::OctetLength => (),
            }
        }
        fn _variadic_func(v: VariadicFunc) {
            match v {
                VariadicFunc::Concat
                | VariadicFunc::ConcatWs
                | VariadicFunc::Greatest
                | VariadicFunc::Least
                | VariadicFunc::NullIf => (),
            }
        }
        fn _window_func(v: WindowFunc) {
            match v {
                WindowFunc::RowNumber | WindowFunc::Rank | WindowFunc::DenseRank => (),
            }
        }
        fn _literal(v: &Literal) {
            match v {
                Literal::Null
                | Literal::Bool(_)
                | Literal::Int(_)
                | Literal::Float(_)
                | Literal::String(_)
                | Literal::Numeric(_) => (),
            }
        }
        fn _binary_op(v: BinaryOp) {
            match v {
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Concat
                | BinaryOp::Like
                | BinaryOp::NotLike
                | BinaryOp::JsonGet
                | BinaryOp::JsonGetText
                | BinaryOp::Containment
                | BinaryOp::ContainedBy
                | BinaryOp::StartsWith
                | BinaryOp::AtTimeZone
                | BinaryOp::ToChar
                | BinaryOp::IsDistinctFrom
                | BinaryOp::IsNotDistinctFrom
                | BinaryOp::Ilike
                | BinaryOp::NotIlike
                | BinaryOp::Mod
                | BinaryOp::Pow
                | BinaryOp::JsonExists
                | BinaryOp::JsonExistsAny
                | BinaryOp::JsonExistsAll
                | BinaryOp::JsonPathGet
                | BinaryOp::JsonPathGetText
                | BinaryOp::AndList
                | BinaryOp::OrList
                | BinaryOp::AndListInner
                | BinaryOp::OrListInner
                | BinaryOp::RegexMatch
                | BinaryOp::NotRegexMatch
                | BinaryOp::ModFn
                | BinaryOp::PowerFn
                | BinaryOp::PowFn => (),
            }
        }
        fn _value(v: &Value) {
            match v {
                Value::Null
                | Value::Bool(_)
                | Value::Int2(_)
                | Value::Int4(_)
                | Value::Int8(_)
                | Value::Float4(_)
                | Value::Float8(_)
                | Value::Text(_)
                | Value::Bytea(_)
                | Value::Timestamp(_)
                | Value::Jsonb(_)
                | Value::Uuid(_)
                | Value::Array(_)
                | Value::Interval(_)
                | Value::Date(_)
                | Value::Numeric(_)
                | Value::Time(_) => (),
            }
        }
        fn _jsonb(v: &JsonbValue) {
            match v {
                JsonbValue::Null
                | JsonbValue::Bool(_)
                | JsonbValue::Number(_)
                | JsonbValue::String(_)
                | JsonbValue::Array(_)
                | JsonbValue::Object(_) => (),
            }
        }
        fn _numeric(v: &PgNumeric) {
            match v {
                PgNumeric::NaN | PgNumeric::Num(_) => (),
            }
        }
    }

    /// Pins the postcard variant discriminants of [`BuiltinFn`] — the **payload** of
    /// `Expr::Builtin`, which the test above cannot see.
    ///
    /// `tag(&Expr::Builtin(_))` reads `Expr`'s own leading byte, so every member of
    /// this enum encodes under one `Expr` tag and reordering them shifts nothing the
    /// pin above measures. They are on-disk format all the same: a persisted CHECK
    /// predicate or index expression naming `now()` decodes by this position, so a
    /// mid-enum insertion would silently turn a stored `now()` into a `CURRENT_DATE`.
    ///
    /// **Every** variant is pinned, not a first/last/anchor sample. Sampling pins the
    /// enum's *length* and leaves its interior permutable: exchanging two unsampled
    /// neighbours — `CurrentDate` and `CurrentTime`, say — shifts no sampled tag, and a
    /// persisted `CURRENT_DATE` in a CHECK predicate would then decode as
    /// `CURRENT_TIME` with the magic unmoved and every gate green.
    ///
    /// **If this test fails**, move the new variant to the end and append its row
    /// below; do not renumber, bump the store magic.
    #[test]
    fn builtin_fn_postcard_variant_tags_are_pinned() {
        let tag = |func: BuiltinFn| {
            postcard::to_allocvec(&Expr::Builtin(func)).expect("Expr serializes")[1]
        };
        // Every variant, in declaration order, with the tag it must keep forever.
        let pins: [(u8, BuiltinFn); 20] = [
            (0, BuiltinFn::Now),
            (1, BuiltinFn::RandomUuid),
            (2, BuiltinFn::CurrentDate),
            (3, BuiltinFn::CurrentTime),
            (4, BuiltinFn::LocalTime),
            (5, BuiltinFn::LocalTimestamp),
            (6, BuiltinFn::TransactionTimestamp),
            (7, BuiltinFn::StatementTimestamp),
            (8, BuiltinFn::ClockTimestamp),
            (9, BuiltinFn::CurrentTimestamp),
            (10, BuiltinFn::CurrentUser),
            (11, BuiltinFn::SessionUser),
            (12, BuiltinFn::User),
            (13, BuiltinFn::CurrentRole),
            (14, BuiltinFn::CurrentCatalog),
            (15, BuiltinFn::CurrentDatabase),
            (16, BuiltinFn::CurrentSchema),
            (17, BuiltinFn::CurrentSchemaFn),
            (18, BuiltinFn::Version),
            (19, BuiltinFn::Random),
        ];
        for (expected, func) in pins {
            assert_eq!(
                tag(func),
                expected,
                "{func:?} no longer encodes as postcard variant {expected} — see this \
                 test's doc comment; persisted CHECK and index expressions decode by \
                 position"
            );
        }
        // No trailing length anchor: over an every-variant list it would restate the
        // loop's own last row. An appended variant is caught by
        // `every_pinned_sub_enum_variant_has_a_row`'s exhaustive match instead.
        for (_, func) in pins {
            let expr = Expr::Builtin(func);
            let bytes = postcard::to_allocvec(&expr).expect("Expr serializes");
            assert_eq!(
                postcard::from_bytes::<Expr>(&bytes).expect("Expr deserializes"),
                expr
            );
        }
    }
}

#[cfg(test)]
mod serialize_expr_determinism {
    use super::{
        BinaryOp, Expr, Literal, OrderKey, WindowFunc, deserialize_expr, encode_expr,
        serialize_expr,
    };
    use crate::planner::Plan;
    use crate::types::{ArrayValue, JsonbValue, PgInterval, PgNumeric, TypeOid, Value};
    use time::macros::{date, datetime};

    /// Values whose encodings are **not** plain derived scalars, so a determinism
    /// claim over the `Expr` closure has to reach them: `time`'s own `Serialize` for
    /// `Timestamp`/`Date`, the fixed `[u8; 16]` behind `Uuid`, a `PgNumeric` carrying
    /// a display scale its digits do not (`1.50`), a three-field `PgInterval`, a
    /// multi-key `JsonbValue::Object`, and a typed array holding a NULL. A `Time`, a
    /// `NaN` and a `Bytea` ride along as ordinary derived shapes.
    fn values() -> Vec<Value> {
        vec![
            Value::timestamp(datetime!(2024-03-04 05:06:07.891011 UTC)),
            Value::date(date!(2024 - 03 - 04)),
            Value::Time(45_296_000_001),
            Value::Uuid([
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
                0xff, 0x00,
            ]),
            Value::Numeric(PgNumeric::parse("1.50").expect("numeric parses")),
            Value::Numeric(PgNumeric::NaN),
            Value::Interval(PgInterval {
                months: 14,
                days: -3,
                micros: 4_567_890,
            }),
            Value::Jsonb(JsonbValue::Object(vec![
                ("b".to_string(), JsonbValue::Bool(true)),
                ("c".to_string(), JsonbValue::Null),
                (
                    "aa".to_string(),
                    JsonbValue::Array(vec![
                        JsonbValue::Number(PgNumeric::parse("2.00").expect("numeric parses")),
                        JsonbValue::String("s".to_string()),
                    ]),
                ),
            ])),
            Value::Array(ArrayValue::new(
                TypeOid::Text,
                vec![Value::Text("a".to_string()), Value::Null],
            )),
            Value::Bytea(vec![0, 1, 2, 255]),
        ]
    }

    /// A representative `Expr` corpus: every `values()` entry as an `Expr::Bound`, the
    /// literal and column leaves, the collection-bearing nodes, and — the one that drags
    /// the widest closure in — an `Expr::Subquery` carrying a `Plan` whose own
    /// projection, filter and sort keys hold the same values.
    fn corpus() -> Vec<Expr> {
        let mut exprs: Vec<Expr> = values().into_iter().map(Expr::Bound).collect();
        exprs.push(Expr::Literal(Literal::Numeric(
            PgNumeric::parse("1.50").expect("numeric parses"),
        )));
        exprs.push(Expr::Literal(Literal::Float(-0.5)));
        exprs.push(Expr::Column {
            table: Some("t".to_string()),
            name: "c".to_string(),
        });
        exprs.push(Expr::Negate(Box::new(Expr::Literal(Literal::Int(7)))));
        exprs.push(Expr::Cast {
            expr: Box::new(Expr::Literal(Literal::String("t".to_string()))),
            target_type: TypeOid::Bool,
        });
        exprs.push(Expr::Array(
            values().into_iter().map(Expr::Bound).collect::<Vec<_>>(),
        ));
        exprs.push(Expr::InList {
            expr: Box::new(Expr::Column {
                table: None,
                name: "c".to_string(),
            }),
            list: values().into_iter().map(Expr::Bound).collect(),
            negated: true,
        });
        exprs.push(Expr::Window {
            func: WindowFunc::RowNumber,
            partition_by: values().into_iter().map(Expr::Bound).collect(),
            order_by: vec![OrderKey {
                expr: Expr::Bound(Value::Interval(PgInterval {
                    months: 1,
                    days: 2,
                    micros: 3,
                })),
                asc: false,
                nulls_first: Some(true),
            }],
        });
        exprs
    }

    /// The plan-bearing half of the corpus — all three `Expr` variants that box a
    /// [`Plan`], each over a sub-plan carrying the value corpus at three depths.
    ///
    /// Split out because these are **encodable but not decodable**: a `Plan`'s
    /// `Deserialize` always fails, so the round-trip half of the determinism guard
    /// cannot run over them. They still need the determinism claim — `structural_identity`
    /// encodes exactly these — so they take the stronger of the two checks below
    /// instead of the weaker one.
    fn plan_bearing_corpus() -> Vec<Expr> {
        vec![
            Expr::Subquery {
                plan: Box::new(subplan()),
            },
            Expr::Exists {
                plan: Box::new(subplan()),
                negated: true,
            },
            Expr::InSubquery {
                expr: Box::new(Expr::Bound(Value::Numeric(
                    PgNumeric::parse("1.50").expect("numeric parses"),
                ))),
                plan: Box::new(subplan()),
                negated: false,
            },
        ]
    }

    /// A `Plan` deep enough to carry the value corpus at three different depths —
    /// `Values` rows, a `Filter` predicate and a `Sort` key — under a `Project`.
    fn subplan() -> Plan {
        Plan::Project {
            input: Box::new(Plan::Sort {
                resolved_targets: crate::planner::plan::ResolvedTargets::default(),
                input: Box::new(Plan::Filter {
                    input: Box::new(Plan::Values {
                        rows: vec![values().into_iter().map(Expr::Bound).collect()],
                        certain: crate::planner::CertainTypes::default(),
                        enclosing: crate::planner::EnclosingColumns::default(),
                    }),
                    predicate: Expr::BinaryOp {
                        left: Box::new(Expr::Column {
                            table: Some("v".to_string()),
                            name: "column1".to_string(),
                        }),
                        op: BinaryOp::Eq,
                        right: Box::new(Expr::Bound(Value::Numeric(
                            PgNumeric::parse("1.50").expect("numeric parses"),
                        ))),
                    },
                }),
                order_by: vec![OrderKey {
                    expr: Expr::Bound(Value::Jsonb(JsonbValue::Object(vec![
                        ("b".to_string(), JsonbValue::Null),
                        ("aa".to_string(), JsonbValue::Bool(false)),
                    ]))),
                    asc: true,
                    nulls_first: None,
                }],
            }),
            exprs: values().into_iter().map(Expr::Bound).collect(),
            certain: crate::planner::CertainTypes::default(),
            enclosing: crate::planner::EnclosingColumns::default(),
        }
    }

    /// The planner-IR encoding is **deterministic**: re-encoding a decoded tree
    /// reproduces the bytes exactly.
    ///
    /// `planner::coercion`'s `structural_identity` uses those bytes as Postgres's
    /// `equal()` over two target-list entries, and a `42702` that should fire is lost
    /// (measured: rows destroyed) if two structurally identical trees can encode
    /// differently. The argument that they cannot is that nothing reachable from an
    /// `Expr` — `Plan` included, and the `pglet_core` value types under it — is a map,
    /// whose iteration order is the thing that would vary. This is the guard on that
    /// argument, and the analogue of `catalog::schema`'s
    /// `serialize_schema_survives_reserialization_byte_stable` one layer over: a
    /// `HashMap` added to any shape `corpus` reaches fails here, because deserializing
    /// rebuilds the map and re-serializing walks it in a fresh order — measured, by
    /// giving `ArrayValue` a twelve-entry `HashMap` field (which `corpus` reaches
    /// through its `Expr::Bound` array) and watching this assertion fail on the two
    /// re-ordered encodings.
    ///
    /// Read its scope exactly. It is a **necessary** condition, not the property: it
    /// shows a tree re-encodes to its own bytes, not that two independently built equal
    /// trees encode alike. And it is only as wide as `corpus` — a variant the corpus
    /// does not name is untested here, so extend the corpus when the closure grows. Its
    /// one structural blind spot is the plan-bearing half, which cannot decode at all;
    /// [`plan_bearing_trees_encode_identically`] covers that half by the stronger route.
    #[test]
    fn serialize_expr_survives_reserialization_byte_stable() {
        for expr in corpus() {
            let first = serialize_expr(&expr).expect("expr serializes");
            let back = deserialize_expr(&first).expect("expr deserializes");
            let second = serialize_expr(&back).expect("expr reserializes");
            assert_eq!(
                first, second,
                "serialize→deserialize→serialize must be byte-stable for {expr:?}"
            );
        }
    }

    /// The determinism claim for the half the round-trip cannot reach: two
    /// **independently built** equal plan-bearing trees encode to identical bytes.
    ///
    /// This is the property `structural_identity` actually needs, not the weaker
    /// necessary condition above — the entries it compares are two separately parsed
    /// target-list expressions, never one tree fed back through a decoder. A `HashMap`
    /// anywhere in the plan closure fails here for the same reason as above and by a
    /// tighter route: two maps built by separate `HashMap::new()` calls carry different
    /// hash seeds, so they iterate differently without any decode step to blame.
    ///
    /// What it does **not** look at: whether the bytes decode (they cannot, by
    /// construction — that is
    /// [`super::super::plan_bearing_expr_bytes_are_not_decodable`]'s subject), and
    /// whether two trees that *differ* encode differently (injectivity, which the
    /// derived field-by-field encoding gives and no test here asserts).
    #[test]
    fn plan_bearing_trees_encode_identically() {
        for (left, right) in plan_bearing_corpus().into_iter().zip(plan_bearing_corpus()) {
            assert_eq!(
                encode_expr(&left).expect("plan-bearing expr encodes"),
                encode_expr(&right).expect("plan-bearing expr encodes"),
                "two independently built equal trees must encode alike for {left:?}"
            );
        }
    }
}
