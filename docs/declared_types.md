# Declared column types — the two walkers

pglet computes its output column types through two different walkers, one per
protocol, feeding two different clients; the differential harness compares both
against Postgres. This file carries the walker seam in full — the summary is in
the repo `CLAUDE.md` § SQL fixtures, and the fixture directives themselves are
`tests/README.md`'s.

## Whether there is a descriptor at all comes first

Both walkers answer *what are the columns*. **Whether the statement has any
result set to describe is a different fact**, decided before either walker's
answer is read, and it is not the column count: a `SELECT *` over a relation
whose last column was dropped returns a result set of **zero** columns, and a
`CREATE TABLE`, a `COMMIT` or an `INSERT` with no `RETURNING` returns no result
set at all. Postgres holds them apart structurally — `psrc->resultDesc` /
`portal->tupDesc` is a null pointer for the second and a valid zero-attribute
tuple descriptor for the first — and pglet does too, as
`pglet_core::types::ResultShape`: `NoResultSet`, or `Columns(…)` whose list may
be empty.

`Plan::output_columns_scoped` decides it per plan variant, alongside the
relations that walk already decides there, so there is **one** enumeration of
the variants and not a second one to drift from; `Plan::result_shape` and
`QueryResult::shape` are what the wire hosts read, and `Plan::output_columns` /
`QueryResult::columns()` are the deliberately lossy reads for callers that only
want names and types. Three replies key on `ResultShape::is_result_set`, and
none of them may key on a list's emptiness again:

- the **simple-`Query`** encoder — a `RowDescription` (`fields=0` and all) plus
  the `DataRow`s for a result set, and neither without one;
- **`Describe`** (`row_description`) — `NoData` for `NoResultSet`, else a
  `RowDescription`, statement leg and portal leg alike;
- the **aborted-block `Describe` gate**, which stands in for Postgres's
  `resultDesc != NULL`: 18.6 refuses a zero-column `SELECT *` there with
  `25P02` and describes a `RETURNING`-less `INSERT`, both measured. Its reach
  here is wider than that measurement, and unbounded against 18.6 — see below.

That criterion is the engine's one descriptor rule and not any one record's:
`row_description` answers `NoData` exactly when the statement returns **no result set**,
and a scan returns one whatever its `*` expands to. `EDGES.md`'s rows 29 and 59 are two
records over that rule and neither is the other's source.

**"One enumeration" is a claim about the plan side, and there are two sides.**
`Plan::output_columns_scoped` is the single place a *plan* decides its
result-set-ness, but `QueryResult::shape` is not that answer carried forward:
the executor builds it at its own sites (`executor/query/mod.rs`,
`executor/helpers.rs`, `executor/types.rs`, `executor/sequence.rs`,
`catalog/commands.rs`), and nothing asserts the two agree. `SHOW` is the one
member measurement has found: the planner never sees the statement
(`runs_outside_the_planner` intercepts it), so `describe_result_shape` asks
`catalog::show_result_shape` first, and that reader derives the shape from
`execute_show_command` rather than re-listing the columns, so the two producers cannot
disagree for this statement. Without it, the parse-failure branch answers `NoResultSet`
against the executor's `Columns(…)`, and the extended leg describes `NoData` for
`SHOW server_version` and then sends a `DataRow` no field descriptor covers — which a
binary client decodes as a row with **no columns**, dropping the value — where 18.6
describes `RowDescription(1)`; the simple leg reads only the executor's answer. The
aborted-block gate reads the same fact: a `SHOW` prepared while the block was healthy is
`25P02` at either `Describe` leg, as 18.6 answers, while a `SET` is still described.
Stated in full on `runs_outside_the_planner` in `crates/pglet_server/src/handlers.rs`.

The **class** that disagreement belongs to stays open: the two producers are separate
code with nothing comparing them.

Keying those three on the column count is `EDGES.md` row 59: the simple leg drops the
descriptor **and the rows with it**, reporting `SELECT 2` and sending none, while the
extended leg says `NoData` and then sends the rows anyway. Rows 29, 61 and 62 are the
divergences the shape rule describes with an empty `RowDescription`: row 29's
missing-relation wildcard spellings, row 62's
two-to-zero-column re-`Describe`, and row 61's `SELECT *` over the zero-column
relation a `RETURNING`-less data-modifying CTE hands its reference. That is an
enumeration and not the rule "an empty expansion is described" — row 63's
`RETURNING *` over a zero-column relation still answers `NoData`, because
`lower_returning` has already collapsed the wildcard by the time the plan
decides. Two residuals sit beside them: rows 63 (18.6's two
at-least-one-column clauses, `RETURNING *` and `SELECT DISTINCT *`, accepted
empty here) and 64 (a zero-column relation is undumpable).

The **aborted-block gate's reach follows the same rule**, and no oracle comparison
bounds it. The gate refuses whatever `is_result_set` is true of, so it refuses every
spelling described with an empty `RowDescription` — not only the zero-column relation
row 59 measured. Prepared while the block was healthy and described after it aborted,
`SELECT * FROM nosuchtable_ab` and `DELETE FROM nosuchtable_ab RETURNING *` are
`25P02` here, while the `RETURNING`-less
`DELETE FROM nosuchtable_ab` is still described. 18.6 refuses all three at
`Parse` (`42P01`), so none of them ever reaches its aborted-block gate and there
is no 18.6 answer to compare against.

**The differential harness holds one leg of this and not the other**, which is
worth knowing before trusting a green run over it: a zero-column row is `{}` in
the recorded JSON, so both legs *express* the shape, but the binary leg decodes
`DataRow`s that followed a `NoData` exactly as it decodes ones that followed a
`RowDescription(0)` — a run with the descriptor missing is green on it. Only the live
text leg goes red. The `Describe` half has no fixture at all and is pinned by raw-frame tests
in `crates/pglet_server`'s `handlers::tests`.

## The two walkers, and what compares them

There is **no single "declared type"**:

- **executor-declared** — `expr_type_oid` via `QueryResult::shape`
  (`executor/helpers.rs`) → the **simple-query** `RowDescription`, plus the
  text rendering in *both* protocols.
- **planner-declared** — `Expr::infer_type` via `Plan::result_shape` →
  `Describe` → the **extended-protocol** `RowDescription`, which is what a
  **binary** client decodes against.

The harness compares **each column's declared type OID**, live
pglet-vs-Postgres, on **both** legs, and neither subsumes the other. On the
text leg every value is a string, so a column declared `text` over an `integer`
still sends `10` and still matches byte-for-byte — the `RowDescription` is the
only place the two differ, and it is what psql, node-postgres, and the wasm
adapter type their output by. On the extended leg a declared type incompatible
with the bytes sent already fails the client's decode, but a **wrong yet
width-compatible** one decodes cleanly and is otherwise invisible. Neither
comparison is recorded in `.expected.json`, so an `--update` run cannot freeze
a divergence into a snapshot.

## The walkers are separate code and have disagreed

The walkers are separate code, and a **scope** difference can split them even where their
rules agree, which rule-table sharing does not prevent: a walker that resolves a `Column`
leaf through the catalog by name types nothing over `FROM mw m`, since a FROM-clause alias
names no catalog table, and lets the arithmetic node answer from whatever it has. Measured
against 18.6, that reading declares `SELECT m.s + m.i FROM mw m` `text` on `Describe`
against `int4` on the simple query, and `SELECT m.b + 0 FROM mw m` `int4` over eight
`int8` bytes. Derived tables, CTE references and joins carry it too, and through the
scalar-subquery path — where the executor walker defers to `Plan::output_columns` rather
than re-deriving — it reaches the simple-query `RowDescription` as well. So both walkers
resolve a projected `Column` against
their node's **input descriptors** rather than the catalog
(`planner::TypeScope::Input` / `executor::ProjectionScope`);
`tests/fixtures/select/alias_*_declared_types.sql` spans the FROM shapes, and
`select/join_qualified_computed_declared_types.sql` pins the
qualifier-awareness itself — a *computed* expression over a qualified reference
into a join whose two sides carry a same-named column of different width, which
is the only shape that reaches `TypeScope::Input`'s own lookup and can tell it
from a flat by-name one (a passthrough column is routed straight to
`find_projected_column`, and a self-join carries identical types on both
sides).

## A scope lookup's cost is part of its correctness

That lookup carries a **cost** the catalog one does not, and not a constant factor: a
lookup that descends the plan node re-runs the inner `output_columns` per qualified
leaf, `(leaves + 1) ^ depth`, and a short-circuit keyed to a plan shape does not bound
it — a per-level `WHERE` (42 260 ms at depth 7) or a join nest with no wrapper node
(83 428 ms at depth 7) walks around it, against 18.4's ~30 ms (measured on 18.4, through
`Describe`: psql `\bind`, debug server, 8 qualified computed leaves per level, wall clock
including ~100 ms of psql startup). So each node's
`output_columns` returns the **relations it puts in scope** alongside its columns
(`RelationColumns`), built from its children's lists — one bottom-up pass, flat at
105–110 ms to depth 30. A cost invariant stated over one plan shape is not stated at
all, and the differential corpus cannot carry this one, since for a single-relation input
the flat by-name fallback returns the same descriptor and only the cost differs. Guards: `planner::tests`'
`describe_over_a_*_is_not_exponential`, one per shape, plus a structural test asserting
a `Filter` keeps its input's relations addressable; `Describe` latency on a shape no
deadline test names is unmeasured by any gate.

## A third `LeafScope`, and the gates that read one

The declaration-only operand gates (`arithmetic_legality`, `negate_legality`,
`math_fn_arg_legality` — the rules that refuse `oid` / `"char"` arithmetic, whose
bodies are `docs/coercion.md`'s) do **not** hold a reader of their own.
Each asks the planner's `Expr::infer_type` — one of the two walkers above, and the
one `Describe` reports — through whichever `LeafScope` its evaluating context can
build. `impl LeafScope for` is the list of implementations — re-derive it by grep rather
than from a count. The ones these gates use are the planner's
`PlannerScope` and `executor::expr`'s `EvalLeafScope`; the second exists because those
gates run inside `eval_expr_full` — a grouping query's expressions included — where the
first is not in hand — its `column_type` is the
resolver-plus-outer-frame pairing `column_declared_type` owns, and its
`subquery_type` is `Plan::output_columns` over whatever catalog the evaluation was
lent (`None` where it was lent none).

A shallow reader in that position misses shapes: one that answers `None` for an
aggregate and for a scalar subquery lets `max(conrelid) + 1` and
`(SELECT conrelid …) + 1` answer integers where 18.6 raises `42883`, and drops the `+00`
offset from `(SELECT max(tz) …)::text`; the grouping executor's `certain_expr_type_oid`
has the aggregate arm and not the subquery one. **Gating on the complete walk is safe here
and is not safe in
general**: this walk substitutes a guess where it cannot resolve (`arith_result_type`
falls back to an operand, the multi-operand cells take the first that types), and a
guessed `text` would refuse legitimate queries at the comparison and `LIKE` rules —
which is why `certain_operand_type` still serves those (`certain_expr_type_oid`'s one
external caller is the grouping executor's aggregate operand check).
The declaration-only gate is exempt for a measured reason rather than a hopeful one:
`oid`, `"char"` and `timestamptz` each enter this walk only at a **leaf** (a column,
a cast target, a subquery's output column), so no guessing arm can invent one from
operands that carry none.

What is **not** closed, and it is two halves rather than one. **Zero rows:** the gate
runs at eval time, so a statement whose evaluation never reaches the node still
answers — `SELECT max(conrelid) + 1 FROM pg_constraint WHERE false` errors here only
because an aggregate always produces its row, while `SELECT conrelid + 1 FROM
pg_constraint WHERE false` is `(0 rows)`. **Precedence:** an eval-time gate loses to
every error the evaluation raises first, so `SELECT max(conrelid) + (1/0) FROM
pg_constraint` is `22012 division by zero` here against 18.6's `42883`, and so is
`SELECT -max(conrelid) FROM pg_constraint WHERE 1/0 = 1`, whose two errors are not
even in the same clause. 18.6 refuses all of them in parse analysis. Moving the gate
to plan time closes both halves and is the change that would; it has not been made.

**The type is derived once per node, never once per enclosing node.** The gate reads
its *operand's* type, so a reader that re-walks the operand subtree from each enclosing
node makes the read quadratic in expression depth — measured 6.4× a linear shallow
reader's cost at depth 32 over 163 840 rows, against a chain of ordinary `int` additions.
The eval walk instead carries a node's declared type up
beside its value (`executor::expr`'s `OperandType`), and each node derives its own from
the ones its operands derived. It is still the one walk that derives it:
`Expr::infer_type_with` is `Expr::infer_type`'s body with its recursion supplied by the
caller, and `binary_op_result_type` is that body's `BinaryOp` cell lifted out — the same
shape `shared_leaf_cell_type` and `shared_multi_operand_type` already have, and the
reason this is not a fourth reader. The asymmetry inside that: the `BinaryOp` sites call
`binary_op_result_type` **directly** where the deriving arms beside them reach their rule
through `infer_type_with`, so what keeps the two routes equal is an invariant stated on
that function — its arm must stay a bare call to it. A node no gate above it reads
derives nothing: the walk is handed no slot to report into, so an ungated `a > 0` pays
nothing for it.

## The seam: shared rules and open-coded ones

A sweep of expressions that reports no disagreement is bounded by its own families,
and they are **expression** families; the scope class varies the FROM clause instead,
so a zero there says nothing about it. The two walkers hold their own open-coded rule
tables over the *same* `Expr` variants, less the ones merged into a shared rule —
`Default`, `Parameter`, `Subquery`,
`Array`, `VariadicFn`, `Coalesce`, `Case` — and
`StringFn`'s `DateTrunc` **member**, whose variant still carries an arm on each
walk for its other members; the structural defence against drift is that the
rules they share live in shared functions (`arith_result_type`,
`numeric_family_result`, `aggregate_result_type`, `math_fn_result_type`,
`temporal_arith_type`, …) rather than being open-coded twice. **A shared rule is not
enough where it reads a fact each walk would derive from its own scope**: an aggregate's
result type turns on whether its argument's integer type is certain, and walks deriving
that from their own descriptors disagreed — the planner's grouped-column descriptor
carried no base table where the executor's did, and the two legs declared different types
over one value (`EDGES.md` row 2). So that fact is not derived by either walk: the
coercion pass records it on the aggregate (`planner::ArgCertainty`), and both walks read
it through `aggregate_type_of`, as do the gate, the pass's branch reader and the
accumulator. A relation's **column** is certain by a record of the same kind and the same
producer — the pass reads the column's defining expression as written and the plan node
carries the type it read (`planner::CertainTypes`, `ColumnDesc::type_certain`) — and
only the planner reads it: no descriptor the executor builds carries it. That a `Column`
leaf is a scope lookup on both sides in the *projection* clauses is a shared
**shape**, not a defence — the two walks still reach it through separate code,
as the next sentences state.
The two leaf questions are one object — a `LeafScope` (`column_type`,
`subquery_type`) the planner's `TypeScope`+`Schema` pair and the executor's
`ProjectionScope` each implement — but **only the `Subquery` leaf is read
through it by both walks**: the planner's `Column` arm calls
`scope.column_type`, while the executor walk's still reads the inherent
`ProjectionScope::resolve`, and the executor impl's `column_type` has
exactly one caller in the tree, `walker_pins`'s scope test. No answer
differs — both bottom out in the same `column_desc` — but the trait is not
yet a structural defence at that leaf: an edit to `column_type` alone would
not move `expr_type_oid`, and what would catch the split is that test's
boundary-identity assertion rather than the shape of the code. There are two
merged rules. `shared_leaf_cell_type` holds four cells over **three** `Expr`
variants (a bare `DEFAULT`, an unbound `$N`, a scalar subquery) plus
`StringFn`'s `date_trunc` **member**, whose variant keeps an arm on each walk
for its other members; `shared_multi_operand_type` holds the four that pick one
operand's type out of several (`ARRAY[…]`, `greatest`/`least`/`nullif`,
`COALESCE`, `CASE`). The line between merged and not is the *second* half
of the split: the executor walk is total, substituting `TypeOid::Text` where
the planner's answers `None`, so a variant whose answer moves with that
substitution is a decision against the oracle rather than a refactor's side
effect. **But "the two arms differ" conflates two things and only one of them
is a decision**: the variadic functions, `COALESCE`, `CASE` and `ARRAY[…]` differ
*only* through the recursion, so one merged arm reading each walk's **own** walk as its
operand hook reproduces both answers exactly but one, at the `array_of()` step (see
below). Those four cells' answer is a separate decision: the executor walk's operand
hook (`executor::helpers`'s
`multi_operand_type`) declines for the four **typeless leaves** — a bare `NULL`
literal, a bound NULL, an unbound `$N`, a bare `DEFAULT` — so the shared rule
sees a real `None` and takes the first inferable operand. That is 18.6's answer
wherever exactly **one** branch type is known **to either walk**; it is not 18.6's
*rule*, which folds the common type of all the known branches, and where two computed
branches carry different types both walks are wrong together (`EDGES.md` row 54's
agree-wrong band). The qualifier is load-bearing in the other direction as well,
because 18.6's notion of *known* is not this one's: a bare **string literal** is
`unknown` there and `text` to both walks here, so `COALESCE('7', length('ab'))`
is `25` on both legs against 18.6's `23`, `COALESCE('abc', length('ab'))`
silently answers `abc` as `text` where 18.6 is
`22P02 invalid input syntax for type integer: "abc"`, and the polarity flips at
the variadic cells — `greatest('7', length('ab'))` is refused `42804` here where
18.6 answers `7`. That is the same literal-as-text policy [compatibility.md](compatibility.md)'s
`COALESCE`/`CASE` row records for stored expressions, reaching a branch set with
no concrete member; it sits beside the fold band in `EDGES.md` row 54, outside this
hook's reach (routing a literal through
`Literal::infer_type` is what keeps a string typing as `text`). An unresolvable
`Column` is neither shape — a
leaf reads no operand, so its two arms differ through the **scope**, which the
trait gives one *shape* — though, as above, only the planner's walk actually
reads its `Column` leaf through it — and merging it is `column_type` under the
same totality boundary the others take; what holds it open is the fenced
unknown-ness question alone (what an unresolvable column should *declare*), the
whole-`ColumnDesc` need belonging to `expr_output_column`, which takes that
descriptor from the scope directly on a passthrough branch entering neither
walk.

Four arms still hold a rule the other walk has no counterpart for, **re-derived** arm by
arm rather than inherited:

- `unnest`'s fallback to the inner type — the **executor** walk's, pinned at
  `unnest(1)`. It stays a member beside the plan-time `unnest(<non-array>)`
  gate (`planner::legality`'s `unnest_legality`, which raises 18.6's own
  `42883` where signature resolution sits): that gate refuses the *directly
  typed* arguments — a column, a cast, a concrete literal — but reads the
  narrow `arg_known_type` at its written reach, so an argument that reader's `_` arm swallows still
  reaches both arms. Measured over an **empty**
  `mw`, all at `(0 rows)` against 18.6's `42883` and all declaring 23 on the
  simple leg against `Describe`'s 25: `unnest(i + 0)`, `unnest(length(tx))`,
  `unnest(abs(i))`, `unnest(COALESCE(i, 1))`, `unnest((SELECT i FROM mw))`,
  `unnest(CASE WHEN true THEN i ELSE i END)`, `unnest(NULLIF(i, 1))`,
  `unnest(GREATEST(i, 1))`, `unnest(LEAST(i, 1))` and `unnest(-i)`, with
  `unnest(b + 0)` measuring 20 against the same 25.
  `unnest(set_config('x', tx, false))` is a member of the same reader class — an
  `Expr::Alias`, which `certain_operand_type` types and this reader does not, so
  "computed" is the wrong name for what the class shares — at the same
  `(0 rows)` against `42883`, but measured 25 on both legs rather than a split,
  as `unnest(tx || 'a')` is. **The silence is the empty
  table's**: with a row every one of them, *spelled bare like this*, is `0A000` — the
  scalar-position refusal, which fires where evaluation reaches the leaf and not
  where a row exists. So what these two arms declare is observable wherever a row
  is kept **away from** the leaf, an untaken branch included: over a one-row
  `mw`, `SELECT CASE WHEN false THEN unnest(i + 0) ELSE 'x' END FROM mw` answers
  `x` here against 18.6's `42883`, declaring `23` on the simple leg against
  `Describe`'s `25` — and that leg's `DataRow` is the text `x` under the `23`,
  the declared-vs-produced shape, while the extended leg's `25` and its binary
  payload agree. The member is recorded with row 55's residual in `EDGES.md`.
  Refusing a caller is not merging an arm;
- `abs`'s bare-literal carve-out — the executor walk's, pinned at `abs(NULL)`;
- arithmetic's `text`-left divert — the executor walk's, pinned at `NULL + 1`;
- `^`'s short-circuit of any half-unknown operand pair to `float8` — the
  **planner** walk's, where the executor walk always folds through
  `numeric_family_result`; pinned at `NULL ^ n`, and it is row 54's third
  member.

`executor::walker_pins` pins the current answers on both sides of that line, merged
and unmerged alike.

**The executor `Aggregate` arm carries no `float8` guess.** A `sum`/`min`/`max` whose
argument the walk answers `text` for, under a projection scope with no input columns, is
reached through `execute_aggregate` over a FROM whose relations are **all** zero-column
(`ALTER TABLE … DROP COLUMN` to nothing; a `RETURNING`-less data-modifying CTE, which
18.6 refuses `0A000`); a `float8` guess there declares `701` over a `text` value and
feeds an enclosing aggregate's gate. `EDGES.md` rows 58 and 61
carry the measurements and the residual (`sum(NULL)`/`avg(NULL)` against `42725`);
`select/zero_column_relation_aggregate_declared_types.sql` and
`select/zero_column_relation_sum_over_text_loud.sql` pin it.

`RETURNING` reads `TypeScope::Input` over `returning_relations`, the same scope object
every other projecting node uses ([scope.md](scope.md)). A rule open-coded in both walkers
carries a hazard the pair-comparison cannot see: duplicated rules do not only drift apart,
they also agree on being wrong (the integer-width note in [compatibility.md](compatibility.md)'s
Arithmetic row is a measured instance).

**And "the two walkers" undercounts the seam, which hides *three*-way
disagreements.** At least four more `Expr` walks answer the same question
with an `Option`: `executor::helpers`'s `certain_expr_type_oid` (deliberately
shallow, feeding the gates), `planner::coercion`'s `concrete_branch_type`
(feeding the common-type folds), and its `certain_operand_type` and `arg_known_type`
(the comparison/`LIKE` and `unnest` gates) — re-derive the set by grep rather than
inherit this list. What divides all of them is **what an
un-inferable operand is** — the executor walk is *total* and answers
`TypeOid::Text`, the other three answer `None` — and a shared rule handed
`Some(Text)` cannot tell "this is text" from "I could not tell".
`SELECT abs(NULL)` is the measured instance: the total walk declares `text` and the
planner's `numeric`, where 18.6 declares `float8` (an unknown argument takes the numeric
category's preferred type — `typeconv.sgml` uses absolute value as its worked example),
and the value being NULL, a wrong answer shows only as the OID. The shared rule is
handed a real `None` for a **bare `NULL` literal** argument — not for the `MathFn` cell —
so both legs declare `float8`. The
arm routes through `Literal::infer_type`, so `Literal::Null` is the only
literal that reaches it, and an argument that is merely *transitively* untyped
still splits the walkers: `abs(-NULL)`, `abs(CASE WHEN true THEN NULL END)` and
`abs(COALESCE(NULL, NULL))` each declare 25 on the simple query against
`Describe`'s 701, where 18.6 refuses all three (`42725` then `42883` twice) —
measured, and recorded in `EDGES.md`. The general
policy is open and belongs decided per rule against the oracle, since unifying
the walks first would only make them agree faster.

Two more rules at the same seam concern a FROM-clause `unnest(…)`, and both are about
*what the executor may derive from a value*: its column type comes from the array
**expression** through one `function_scan_column_types` both walkers read, because the evaluated
`Value::Array`'s `element_type` cannot carry the tz/no-tz bit — derived from the value,
an `unnest(ARRAY[…::timestamptz])` column declares 1114 on the simple leg against
`Describe`'s 1184, and the column-type-aware text encoder drops the UTC `+00` offset. And
the *planner's* coercion scope registers an unnest's columns, so `COALESCE` branch
unification does not defer over one. **What deferring costs is not "every branch
untypable"** — one untypable branch is enough for `unify_common_type_branches` to defer,
after which the executor's total walk decides by its own rule (a `COALESCE`'s first
argument's type), right or wrong by accident of which branch leads: measured against an
empty map, `COALESCE(NULL, v)` declares 25 and its sibling `COALESCE(v, 0)` 23 on the same
simple-query `RowDescription`, where 18.6 says 23 to both. The second is the fixture's
polarity pin. Registering the unnest's columns decides two further verdicts — see the
`unnest` bullet in `EDGES.md`.

**The coercion pass defers a second way, and the deferral is a property of the
*branch set*, not of the cell.** The `ARRAY` / `greatest` / `COALESCE` / `CASE` splits
are masked from SQL by the coercion pass only for some branch sets, and the mechanism is
two readers:

- `COALESCE` / `CASE` / `ARRAY[…]` go through `unify_common_type_branches`,
  which asks `select_common_type` for a target and returns the branches
  **untouched** on `Ok(None)` — its answer when *no* branch is concrete to
  `concrete_branch_type`;
- `greatest`/`least` go through `unify_variadic_args` and `nullif` through
  `coerce_nullif_args`, both over `arg_known_type`, which yields no target the
  same way.

**The two readers are not one walk**, and the difference is an arm list rather
than a policy: over an otherwise identical `Column` / `Cast` / `CastTypmod` /
literal set, `concrete_branch_type` carries an `Array` arm (the elements' common type,
one dimension up) and an `Alias` arm that `arg_known_type` does not; the `Aggregate`
arm (its argument's type, recursed) both carry, `arg_known_type` delegating it.
Measured, that is exactly where the family's boundary runs: `COALESCE(NULL, sum(b))` and
`COALESCE(NULL, ARRAY[1,2])` are rewritten and fall outside it (1700 and 1007 on both
legs, as on 18.6), while `greatest(NULL, ARRAY[1,2])` and its `least`/`nullif` spellings
fall **inside** it: raw, they split 25 on the simple leg against `Describe`'s 1007, where
18.6 says 1007, and the executor walk's operand hook (below) is what decides them. No
fixture names them, because none builds an array beside a bare `NULL` in a variadic. The
`Alias` arm is **reachable and moves no declared type**, which is the weaker fact of the
two: `set_config(<name>, <value>, <is_local>)`
lowers to an `Expr::Alias` wrapping `<value>` — the one production `Alias` mint
that nests inside another expression, the others (projection item, `VALUES`
row, sort junk, `JOIN … USING` merged column, `planner::query`'s
`coerce_unknown_entry` — a set-op side's `unknown` output entry rewritten to a
`Cast` the alias names — and `carried_key`, which re-wraps a bare `Int` literal lifted
out of a projection entry so the sort key it becomes is not read back as positional) all sitting at a
list's top level. `carried_key` is the member an enumeration easily drops,
because it mints on the **right of a `match` arm** rather than into a `push` or a
`return`, so a sweep that reads `=>` lines as patterns rather than constructors
never sees it; it replaces the whole `OrderKey.expr` and wraps nothing else, so it
moves no declared type either. Over `(i INT, tx TEXT)`,
`COALESCE(NULL, set_config('x', tx, false))` and its `greatest`/`least`/`nullif`
spellings are 25 on both legs, the `i` spelling 23 on both, since both walks read
through the alias to the inner expression either way.

**What the arm `arg_known_type` lacks moves is a gate verdict**, which is a
consequence of this arm list no declared-type comparison can see. The `42804`
branch-unification gate reads through `branch_type_with_subqueries` (and, for a set
holding an enum, `complete_resolved`) — `concrete_branch_type` plus the scalar-subquery arm ([coercion.md](coercion.md) § What
the gate reads) — for all five constructs it covers — `COALESCE`, `CASE`, `ARRAY[…]`, `greatest`, `least`; `nullif` is not in
that match at all and reaches the operand gate beside it instead — so over a
`greatest`/`least`/`nullif` the *pass* is the blinder of the two readers: it folds
the concrete sibling alone and wraps the argument it cannot see in a `Cast` to
that target, and the gate then reads that `Cast` at an agreeing type and never
fires. **That needs a concrete sibling to fold**: with none,
`variadic_common_type` answers `None`, the pass rewrites nothing and the gate
types the raw pair itself — `greatest(ARRAY[1,2], set_config('x', tx, false))` is
`42804` on both engines. Over `(i INT, tx TEXT, a INT[])` holding
`(7, '7', {1,2})`, on both legs, `greatest(ARRAY[1,2], tx)` answers 25 over
`{1,2}`, `least` 25 over `7`, `nullif` 25 over `{1,2}`, and the
`(set_config('x', tx, false), i)` spellings answer 23 over `7`, `7` and NULL —
where 18.6 refuses each `42804` (`nullif` `42883`). The same `int[]` through an
arm both readers carry is refused as it should be (`greatest(a, tx)`,
`greatest(a::int[], tx)`, `42804` on both engines), and the `COALESCE` family,
where pass and gate read the one reader, is untouched (`COALESCE(i, ARRAY[1,2])`,
`COALESCE(set_config('x', tx, false), i)`, `42804` on both). `EDGES.md` row 65's
third sub-case carries the streams and the bounds.

What both readers *do* share is a `None` for every **computed** expression
*outside* the arms just named and the numeric-category computed shapes both now
type (`docs/coercion.md` § A computed branch is typed where its type is certain) — a
string function but `length`/`strpos`/`position`, temporal arithmetic, a subquery, an
`Expr::Bound`, a `$N`, a `DEFAULT`. The scope is the point:
`sum(b)` is a function call and is **not** `None` to `concrete_branch_type`,
which is the whole of the arm list above. So a
branch set of `{a bare NULL} ∪ {a computed expression}` carries no target and reaches
both walkers raw; the masked spellings all have a concrete literal branch
(`COALESCE(NULL,1)`, `ARRAY[NULL,1]`, …). Raw, the walkers split in every projecting
position measured against 18.6 — a plain projection, `RETURNING`, a `GROUP BY`/`ORDER BY`
ordinal, a derived table, a `VALUES` row, a scalar-subquery operand, an `unnest` wrapper,
a nested `COALESCE`, an empty relation and `LIMIT 0` — with the planner walk right at
every one: `COALESCE(NULL, length('ab'))` declares 25 on the simple leg against 18.6's
23, and `COALESCE(NULL, date_trunc('day', ts))` reaches a **value** — the
column-type-aware text encoder sends `2024-01-15 00:00:00` where 18.6 sends
`2024-01-15 00:00:00+00`, and the table-free `COALESCE(NULL, now())` is the same
corrupted value with no setup at all. **Not every consequence of the split is silent**:
the executor walk's answer becomes the projecting node's `ColumnDesc`, which an enclosing
aggregate's operand gate reads (row 58's mechanism), so a split
`SELECT sum(c) FROM (SELECT COALESCE(NULL, length('ab')) AS c) s` is
`42883 function sum(text) does not exist` where 18.6 answers `2` — a refused statement
rather than a mislabelled column. Closed by the executor
walk's operand hook (see § The
seam: shared rules and open-coded ones); the guards are
`select/{coalesce,case,variadic,array}_untyped_first_*`,
`select/coalesce_untyped_first_branch_timestamptz.sql`, and
`select/coalesce_bound_param_null_aggregate_operand.sql` — the bound-`$N` leaf's,
which holds that loud consequence over the extended protocol because the leaf's
own OID-and-value divergence is invisible to a binary-format `Bind` (the
`timestamptz` corruption is text-format-only, and `@param` requests binary). The
residual is the
`abs(-NULL)` family's: an operand that is only *transitively* untyped keeps the
total answer, so `COALESCE(-NULL, length('ab'))`,
`COALESCE(CASE WHEN true THEN NULL END, length('ab'))` and
`COALESCE(COALESCE(NULL, NULL), length('ab'))` still declare 25 against
`Describe`'s 23 — and 18.6 refuses all three rather than answering the other
branch (`42725` for the first, `42804` for the two whose untyped branch resolves
to `text`), so the honest close is a refusal and not the other branch's type.
The all-untyped shapes are the controls:
`COALESCE(NULL, NULL)`, `greatest(NULL, NULL)`, `nullif(NULL, NULL)` and
`CASE WHEN true THEN NULL END` are 25 on both legs and on 18.6, and
`ARRAY[NULL, NULL]` is 1009 on all four.

**`Ok(None)` has a third consequence, and it is not a declared type at all.** The
branch-unification **gate** — `planner::coercion`'s
`check_branch_unification_node`, which stands in for 18.6's `42804` — reads that
same `concrete_branch_type` (through `branch_type_with_subqueries`), and `select_common_type` folds only the branches it
resolves, so an invisible branch is never compared against a sibling **whether or
not the set has a concrete member** (`COALESCE(i, upper(tx))` answers `7` where
18.6 refuses). These branch sets are the sub-case where nothing at all is
compared: the fold is `[None, None]`, the answer `Ok(None)`, and nothing is
raised. pglet then *answers* where
18.6 refuses: `COALESCE(length('ab'), upper('7'))` is int4 `2` here,
`ARRAY[length('ab'), upper('7')]` is `_int4` `{2,7}`, and
`COALESCE(upper('x'), length('ab'))` is text `X` — the answer decided by whichever
branch leads, over a pair 18.6 will not match at all. What leads is two facts and
not one, and a short-circuiting cell parts them: the walkers declare the first
*inferable* branch while evaluation returns the branch it **takes**, so
`CASE WHEN false THEN length('ab') ELSE upper('x') END` declares 23 and streams
the single byte `58` — a declared type over a value of another type, which is
`EDGES.md` row 36's shape reached with no `$N` and no NULL.
`COALESCE(i, tx)` is the
control that says the gate exists: `42804` on both engines. This is a **wrong
answer**, not a wrong label, so it is its own row (`EDGES.md` row 65) rather than
a residual of this one — and it shares the fold band's root, since widening
`concrete_branch_type`/`arg_known_type` past everything they answer `None` for (the
computed expressions, and the `Expr::Bound` constants such as an `INTERVAL` literal) is
what
closes both.

## What each walker is handed: one resolver, one coercion pass

The executor's descriptor walk resolves through the same `SimpleColumns` resolver that
binds the row's values — so a declared type cannot name a different column than the value
came from — and it types a scalar subquery through the planner's schema-aware walk rather
than a second, schema-blind one. Upstream of both, the coercion pass rewrites a branch
into an `Expr::Cast` only where it can resolve a target, so a branch set it resolves
nothing for reaches the two walkers raw; that is the condition every split recorded in
`EDGES.md` rows 54 and 55 is measured under.

The `abs` carve-out above is narrow on purpose: every other shape keeps the total walk,
because the shallow `certain_expr_type_oid` would answer `None` for an ordinary computed
argument and turn `abs(v + 1)` over an `int` column into `float8`.

## The four typeless leaves, and the routes that do not reach them

The shared multi-operand rule is handed a real `None` for four **typeless leaves** — a
bare `NULL` literal, a bound NULL, an unbound `$N`, a bare `DEFAULT` — the same carve-out
shape `abs`'s takes. The `$N` leaf is an **undeclared** parameter: one whose type the client
declared at `Parse` reaches both walks as a `Cast` over the leaf, so it is typed and is not
one of the four (`docs/param_binding.md` § A declared type is part of parse analysis).
Which of them ordinary SQL reaches, and by what route, is not
uniform. Of the four leaves,
the `Literal(Null)` and `Parameter` ones are live from ordinary SQL, the bare `DEFAULT`
is live wherever a **short-circuiting** cell operand goes unevaluated — a `CASE` result
branch (raw, `SELECT CASE WHEN false THEN DEFAULT ELSE length('ab') END` declares `25`
against `Describe`'s `23`, and the `now()` spelling reaches the dropped-offset value),
and equally a trailing `COALESCE` operand, a `WHEN` condition
past the taken arm, a simple-form `CASE` and a whole nested cell, with the row's own
values deciding which (row 56's rule and its short-circuit entries; unfixturable, since
that acceptance is row 56's) — and a NULL-valued `Expr::Bound` has **no route found**
into a multi-operand cell, over four production mints, each disqualified by its **own**
property rather than by a
shared one: `parser::ddl`'s `column_default_to_expr` reaches `Expr::Bound` only for
values with no `Literal` spelling, the `INSERT` source-row substitution replaces a
whole column slot rather than nesting, `executor::dml`'s `substitute_nextval` *does*
nest (it rewrites through `map_children`, so `DEFAULT COALESCE(nextval('s'), 1)` mints
a `Bound` inside the cell — measured, a sequence set to 100 stores `101` then `102`,
and `greatest(nextval('s'), 5)` over one set to 200 stores `201` then `202`) but both
`SequenceDraw` arms yield a `Value::Int8`, and `parser::expr`'s
`parse_interval_literal` nests too and carries an interval. Nesting is therefore
**not** what rules any of them out.

## A query-position `unnest` is refused at plan time

**The close is 18.6's `42883`, raised where signature resolution sits** —
`planner::legality`'s `unnest_legality`, called from `check_expr_node_semantics`'s one
bottom-up, left-to-right plan-time walk. Everything that follows is a property of that
placement rather than of the rule:

- **Row-independent, because it reads a declared type.** Left to evaluation, the verdict
  turns on whether evaluation reaches the leaf — silent over an empty table, loud under one of
  *two* wrong codes otherwise (`0A000 set-returning unnest() is only supported in a
  SELECT projection`, the scalar-position refusal a row-bearing `Plan::Project` reaches;
  `0A000 unnest requires an array argument, got Int4(1)` from `execute_project_unnest`
  where a no-FROM projection's single row reaches it). That is row 56's mechanism, and
  the gate takes this leaf out of it entirely: measured, `42883` over an empty table,
  over rows, over `WHERE false`, inside an untaken `CASE` branch and inside a
  short-circuited `COALESCE`.
- **It is not a row-60 member.** Measured over raw v3 against both engines, the refusal
  arrives at `Parse` — `E Z`, with no `ParseComplete` — on pglet as on 18.6, so the
  phase agrees as well as the code. That is the point of putting it at plan time rather
  than in the executor.
- **The position is the expression's, not the projection clause's.** `RETURNING`, a
  sort key, a `WHERE`, an `UPDATE … SET` right-hand side, a set-op arm, a CTE and an
  `INSERT … SELECT` source each reach the same walk and each is `42883` on both engines
  (each measured). Left to evaluation, the last two accept the statement — `INSERT 0 0` /
  `UPDATE 0` — **only where evaluation does not reach the leaf**, which is not the same
  emptiness for the two: `INSERT INTO dst SELECT unnest(i) FROM mw` is `INSERT 0 0` over
  an empty `mw` and `0A000` with a row in it, while the CTE spelling
  `WITH c AS (SELECT unnest(i) AS q FROM mw) UPDATE dst SET q = 1 WHERE q IN (SELECT q
  FROM c)` is `UPDATE 0` whether or not `mw` holds a row, because reaching the CTE takes
  a row in **`dst`** — with one, it too is `0A000`. Row 56's mechanism at both, and a
  *quiet* answer only over the empty shape.
- **Which error wins, at two scales, both measured on 18.6 first.** Within the
  expression the walk is bottom-up, so the argument's own binding failure outranks the
  signature refusal built on it (`unnest(nosuchcol)` is `42703`). Between clauses the
  whole target list is analysed before the `WHERE`, and pglet reproduces that because
  `lower_select_level` lowers the projection ahead of the predicate, item by item in
  source order: `SELECT unnest(i) FROM t WHERE nosuchcol = 1` and
  `SELECT unnest(i), nosuchcol FROM t` are `42883`, while
  `SELECT nosuchcol, unnest(i) FROM t` is `42703`. Two neighbours follow, both as 18.6
  answers: `SELECT unnest(i), 'abc'::int FROM t` is `42883`, not `22P02` (the coercion
  pass runs per item, so item one's gate fires before item two is folded), and
  `… HAVING count(*) > 0` is `42883`, not `42803`.

## Coverage and limits

Only the text leg needed building: tokio-postgres discards the simple query's
`RowDescription`, so a hand-rolled probe reads it, whereas the extended leg's
OIDs ride free on `Statement::columns` off the prepared statement the binary
path already sends. The two also cover **different corpora** — `@param` and
`@no-txn` are extended-only (the simple query cannot bind), `@text` is
text-only — so the extended leg is a coverage gain even where the walkers
agree. (The text leg excludes `@no-txn` fixtures for a recorded reason — `has_text_leg`'s doc
in `test_fixtures.rs`: that leg's types come from a *second* connection replaying the
setup, which a fixture driving its own transactions cannot be assumed to survive. Lifting
the exclusion is unmeasured under the current cross-connection rule, where that replaying
connection's write beside a block that has written waits for it — `55P03` past the
default `1s` `lock_timeout`.) Together they reach **roughly two
thirds of the corpus**; the run prints the exact per-leg split rather than
fixing a number here, since it moves with every fixture added. The rest is
gated out deliberately (`@expect-error` on both legs, the per-leg exclusions
above, a `@no-declared-types:` opt-out that waives **both** legs by design — a
per-leg waiver would hide the internal split) or has no columns to compare. Two
known limits, shared by both legs: **typmod is not compared** — a column can be
declared the right type with the wrong precision/length and still pass. Turning
it on was measured, and the finding is a **class** rather than a count: an
**explicit cast's** typmod is `-1` here where 18.6 sends the modifier's encoding —
measured over the wire with node-postgres's `dataTypeModifier`, `SELECT
'…'::timestamp(3), 'abcd'::varchar(7)` reading `[-1, -1]` here against `[3, 11]`
there. `ColumnDesc::new` defaults the field to `-1`, and the sites that write anything else are
the two base-table-column ones (`planner::plan`'s `table_columns_desc`,
`executor::helpers`'s `table_column_descs`) **plus the recursive-CTE pair below** —
so every *computed* column carries the default whatever the cast says. How many fixtures
that reddens moves with the corpus and is deliberately not fixed here. A value
constructor is in the same class and costs more than a wrong width: 18.6's `exprTypmod`
gives a `CASE`, `COALESCE` or `GREATEST` its arms' common modifier when they agree, and
pglet gives it `-1`, so the two engines disagree about a modifier **18.6 knows exactly**.
So does a column that two sources unify or a subquery passes up — a set operation, a
`JOIN … USING` merge, a scalar subquery — whose sources agree: 18.6 keeps their modifier
(`SELECT v … UNION ALL SELECT v …` over a `varchar(5)` is `9` there) and pglet sends `-1`
on both legs. All three are `EDGES.md` row 155.

### A recursive CTE's declared modifier

A genuinely-recursive CTE's columns are rebuilt rather than propagated
(`parser::select`'s `recursive_cte_columns`), which makes the modifier a decision rather
than an inheritance, and the two candidate answers are not equally safe. Taking the
**seed's** is what 18.6 always reports — because 18.6 *refuses* every pair whose two
terms disagree (`42804 … has type character varying(5) in non-recursive term but type
character varying overall`, and the comparison is `exprTypmod` against the set
operation's resolved column). pglet does not refuse that pair, so taking the seed's
declared `character varying(5)` over a `UNION ALL SELECT 'abcdefgh'::varchar(8)` is a
width a `DataRow` two frames later violates.

So the modifier is **resolved**, by `planner::common_type`'s `select_common_typmod` —
18.6's own function, and asymmetric in a way worth carrying: it keeps the pair's modifier
only while both sides agree *and* both types are the common one, so a seed declaring none
can never acquire one, while a seed declaring a width loses it to any term pglet does not
read as declaring the same. `check_recursive_shape` returns that answer per column and the caller
writes it over **both** readers of the list — the `Plan::CteScan` the body resolves
against (`Describe`) and the `CteBody::Recursive` the executor materializes (the
simple-query `RowDescription`). Writing it over one is a two-producer split, which
`crates/pglet/tests/cte_column_list.rs` catches.

pglet does **not** raise 18.6's `42804` on the modifier, and the reason is this document's
subject: it cannot compute `exprTypmod`. Raising without it is measured to refuse a whole
family of statements 18.6 answers — a cast to the seed's *own* width
(`a::varchar(5)`, `'xy'::varchar(5)`, `1.5::numeric(6,2)`) and a value constructor over
the working-table column (`CASE`, `COALESCE`, `GREATEST`, `NULLIF`, a scalar subquery).
The members are kept as a table in `crates/pglet/tests/cte_column_list.rs` rather than
counted in prose.

**What is left at the pairs 18.6 refuses is an over-acceptance — `42804` there, an answer
here — and it has two subsets with different repairs:**

- **A term pglet reads as `-1` where 18.6 knows a modifier** — a cast, a value
  constructor (`EDGES.md` row 155). pglet declares `-1` and answers. The same read gives
  the neighbouring family 18.6 *answers* (the cast to the seed's own width, the
  constructors) a declared `-1` against the seed's modifier 18.6 declares — an
  under-declaration. This subset and that neighbour wait on row 155: no comparison can be
  18.6's while one side reads a default.
- **A term pglet reads exactly, with a different modifier** — a base column projected
  directly: a `VARCHAR(8)` or `VARCHAR(3)` column beside a `VARCHAR(5)` seed, a
  `NUMERIC(10,5)` beside a `NUMERIC(6,2)`, and a `TIMESTAMP(3)` column beside a
  `TIMESTAMPTZ(3)` seed, whose modifiers agree as numbers over two types. pglet reads
  both sides here, declares `-1`, and is not waiting on row 155 to see the disagreement.
  18.6's own comparison — the set operation's resolved modifier against the seed's —
  made only where both sides' modifiers are *read*, could close this subset without
  touching the first.

**Whether a read modifier is the column's own is the premise such a refusal would rest
on.** A non-negative modifier is minted only at the base-column sites
(`planner::plan`'s `table_columns_desc`, `executor::helpers`'s `table_column_descs`) and
by the recursive CTE's own pair; every other site mints `-1` or copies an existing
descriptor. A copy that *chooses* its descriptor by name is where the premise can break,
and on the planner's side the choosing is `project_output_columns`' lookup through
`find_projected_column`, which the `Window` arm shares. Two routes can take another
relation's same-named column there, and both are closed (`EDGES.md` row 158): a window
query's own lookup by bare name, and a qualified reference to a FROM-clause `unnest` that
puts no relation in scope for the qualifier to find. Open, either lets a term beside a
`VARCHAR(5)` seed read `9` for a `varchar(8)` or `text` column, which the fold keeps, so
the CTE declares `character varying(5)` over a row of eight characters — measured on both
legs of the native server against 18.6, which refuses that statement `42804`. Beside a
`VARCHAR(8)` seed the window term reads `12` and the CTE declares `12`, as 18.6 does.

`find_projected_column` reads a reference the input does not bind as an **enclosing**
level's, off the node's record of the column it binds to (`EnclosingColumns`, modifier
`-1`), and declines one the record does not hold (row 159). That route was not found to
reach this fold: a `WITH` inside a scalar subquery is `0A000` here and a `LATERAL` derived
table `42601`, so no recursive term measured has an enclosing level to correlate to.

No fixture can see any of it — neither leg compares a modifier — so what is pinned is
pinned in Rust.

The *column* half is the one that does agree, and it agrees at the width 18.6 sends:
over `wtm (a timestamp(3), b timestamptz(2), c time(1), d interval(4), e varchar(7),
f timestamp, g timestamp(3)[])`, both engines send
`[3, 2, 1, 2147418116, 11, -1, 3]` — the interval's packed range mask and the array
column's *element* width included. That half is pinned on the catalog side by
`catalog/precision_atttypmod.sql` and `array/typmod_column_atttypmod.sql`; over the
**wire** it is pinned by nothing, `pglet_tests`' `parse_row_description` stepping past
the field. The other limit is that duplicate output
column names collapse in the JSON keying, so give such a `@test` distinct `AS`
labels. A third, bigger caveat is not a limit of the check but of the corpus:
it only sees SQL some fixture issues, so a green run is a statement about
coverage as much as about correctness.
