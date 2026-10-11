# Declared column types — the two walkers

pglet computes its output column types with two different walkers, one per protocol, and
each one feeds a different client. The differential harness compares both against
Postgres. This file covers the seam between the walkers in full. The summary is in the
repo `CLAUDE.md` § SQL fixtures, and the fixture directives themselves are in
`tests/README.md`.

## Whether there is a descriptor at all comes first

Both walkers answer *what are the columns*. Whether the statement has a result set to
describe at all is a separate question. It's decided before either walker's answer is
read, and the column count doesn't answer it: a `SELECT *` over a relation whose last
column was dropped returns a result set with **zero** columns, while a `CREATE TABLE`, a
`COMMIT` or an `INSERT` with no `RETURNING` returns no result set at all. Postgres keeps
them apart structurally. `psrc->resultDesc` / `portal->tupDesc` is a null pointer for the
second case and a valid zero-attribute tuple descriptor for the first. pglet does the same
with `pglet_core::types::ResultShape`, which is either `NoResultSet` or `Columns(…)` with a
list that may be empty.

`Plan::output_columns_scoped` decides it per plan variant, next to the relations that walk
already decides, so there's **one** enumeration of the variants and no second one to drift
from it. The wire hosts read `Plan::result_shape` and `QueryResult::shape`.
`Plan::output_columns` / `QueryResult::columns()` are deliberately lossy reads for callers
that only want names and types. Three replies key on `ResultShape::is_result_set`, and
none of them may go back to keying on whether a list is empty:

- the **simple-`Query`** encoder sends a `RowDescription` (even with `fields=0`) plus the
  `DataRow`s for a result set, and neither without one;
- **`Describe`** (`row_description`) sends `NoData` for `NoResultSet` and otherwise a
  `RowDescription`, on the statement leg and the portal leg alike;
- the **aborted-block `Describe` gate** plays the part of Postgres's `resultDesc != NULL`.
  18.6 refuses a zero-column `SELECT *` there with `25P02` and describes a
  `RETURNING`-less `INSERT`; we checked both. Its reach here is wider than what we
  checked, and nothing bounds it against 18.6 (see below).

That criterion is the engine's single descriptor rule, not something any one record
defines. `row_description` answers `NoData` exactly when the statement returns **no result
set**, and a scan returns one whatever its `*` expands to. `EDGES.md` rows 29 and 59 both
record cases of that rule, and neither is derived from the other.

**"One enumeration" applies to the plan side, and there are two sides.**
`Plan::output_columns_scoped` is the single place a *plan* decides whether it has a result
set, but `QueryResult::shape` doesn't carry that answer forward. The executor builds it at
its own sites (`executor/query/mod.rs`, `executor/helpers.rs`, `executor/types.rs`,
`executor/sequence.rs`, `catalog/commands.rs`), and nothing checks that the two agree.
`SHOW` is the only disagreeing case we've found. The planner never sees the statement
(`runs_outside_the_planner` intercepts it), so `describe_result_shape` asks
`catalog::show_result_shape` first. That reader derives the shape from
`execute_show_command` instead of listing the columns again, so the two producers can't
disagree for this statement. Without it, the parse-failure branch answers `NoResultSet`
while the executor answers `Columns(…)`. The extended leg then describes `NoData` for
`SHOW server_version` and sends a `DataRow` that no field descriptor covers, which a
binary client decodes as a row with **no columns**, dropping the value. 18.6 describes
`RowDescription(1)`. The simple leg reads only the executor's answer. The aborted-block
gate reads the same fact: a `SHOW` prepared while the block was healthy is `25P02` at
either `Describe` leg, matching 18.6, while a `SET` is still described. The full statement
is on `runs_outside_the_planner` in `crates/pglet_server/src/handlers.rs`.

The broader **class** of disagreement stays open: the two producers are separate code and
nothing compares them.

Keying those three replies on the column count is `EDGES.md` row 59. The simple leg drops
the descriptor **and the rows with it**, reporting `SELECT 2` and sending none, while the
extended leg says `NoData` and then sends the rows anyway. Rows 29, 61 and 62 are the
divergences where the shape rule gives an empty `RowDescription`: row 29's
missing-relation wildcard spellings, row 62's two-to-zero-column re-`Describe`, and row
61's `SELECT *` over the zero-column relation that a `RETURNING`-less data-modifying CTE
hands to its reference. That's a list of cases, not a rule that "an empty expansion is
described". Row 63's `RETURNING *` over a zero-column relation still answers `NoData`,
because `lower_returning` has already collapsed the wildcard by the time the plan decides.
Two residuals sit next to them: row 63 (18.6's two at-least-one-column clauses,
`RETURNING *` and `SELECT DISTINCT *`, are accepted empty here) and row 64 (a zero-column
relation can't be dumped).

The **aborted-block gate's reach follows the same rule**, and no comparison against the
oracle bounds it. The gate refuses anything `is_result_set` is true of, so it refuses
every spelling described with an empty `RowDescription`, not only the zero-column relation
row 59 measured. Prepared while the block was healthy and described after it aborted,
`SELECT * FROM nosuchtable_ab` and `DELETE FROM nosuchtable_ab RETURNING *` are `25P02`
here, while the `RETURNING`-less `DELETE FROM nosuchtable_ab` is still described. 18.6
refuses all three at `Parse` (`42P01`), so none of them reaches its aborted-block gate and
there's no 18.6 answer to compare against.

**The differential harness covers one leg of this and not the other**, which is worth
knowing before you trust a green run here. A zero-column row is `{}` in the recorded JSON,
so both legs can *express* the shape. But the binary leg decodes `DataRow`s that followed
a `NoData` exactly the same as ones that followed a `RowDescription(0)`, so a run with the
descriptor missing is green on that leg. Only the live text leg goes red. The `Describe`
half has no fixture at all; raw-frame tests in `crates/pglet_server`'s `handlers::tests`
pin it.

## The two walkers, and what compares them

There is **no single "declared type"**:

- **executor-declared**: `expr_type_oid` via `QueryResult::shape`
  (`executor/helpers.rs`) → the **simple-query** `RowDescription`, plus the text
  rendering in *both* protocols.
- **planner-declared**: `Expr::infer_type` via `Plan::result_shape` → `Describe` → the
  **extended-protocol** `RowDescription`, which is what a **binary** client decodes
  against.

The harness compares **each column's declared type OID**, live pglet-vs-Postgres, on
**both** legs, and neither leg covers the other. On the text leg every value is a string,
so a column declared `text` over an `integer` still sends `10` and still matches
byte-for-byte. The `RowDescription` is the only place the two differ, and it's what psql,
node-postgres and the wasm adapter use to type their output. On the extended leg, a
declared type that doesn't fit the bytes sent already fails the client's decode, but one
that's **wrong yet width-compatible** decodes cleanly and is otherwise invisible. Neither
comparison is recorded in `.expected.json`, so an `--update` run can't freeze a divergence
into a snapshot.

## The walkers are separate code and have disagreed

The walkers are separate code, and a difference in **scope** can split them even where
their rules agree. Sharing rule tables doesn't prevent that. A walker that resolves a
`Column` leaf by name through the catalog types nothing over `FROM mw m`, since a
FROM-clause alias names no catalog table, and lets the arithmetic node answer from
whatever it has. Checked against 18.6, that reading declares `SELECT m.s + m.i FROM mw m`
as `text` on `Describe` but `int4` on the simple query, and `SELECT m.b + 0 FROM mw m` as
`int4` over eight `int8` bytes. Derived tables, CTE references and joins have the same
problem. Through the scalar-subquery path, where the executor walker defers to
`Plan::output_columns` instead of deriving the type again, it reaches the simple-query
`RowDescription` too.

So both walkers resolve a projected `Column` against their node's **input descriptors**
instead of the catalog (`planner::TypeScope::Input` / `executor::ProjectionScope`).
`tests/fixtures/select/alias_*_declared_types.sql` covers the FROM shapes, and
`select/join_qualified_computed_declared_types.sql` pins the qualifier-awareness itself.
That fixture is a *computed* expression over a qualified reference into a join whose two
sides have a same-named column of different width. It's the only shape that reaches
`TypeScope::Input`'s own lookup and can tell it apart from a flat by-name lookup: a
passthrough column goes straight to `find_projected_column`, and a self-join has
identical types on both sides.

## A scope lookup's cost is part of its correctness

That lookup has a **cost** the catalog lookup doesn't, and it isn't a constant factor. A
lookup that descends the plan node re-runs the inner `output_columns` for every qualified
leaf, `(leaves + 1) ^ depth`, and a short-circuit keyed to one plan shape doesn't bound
it. A per-level `WHERE` (42 260 ms at depth 7) or a join nest with no wrapper node
(83 428 ms at depth 7) gets around it, against 18.4's ~30 ms (measured on 18.4, through
`Describe`: psql `\bind`, debug server, 8 qualified computed leaves per level, wall clock
including ~100 ms of psql startup).

So each node's `output_columns` returns the **relations it puts in scope** along with its
columns (`RelationColumns`), built from its children's lists. That's one bottom-up pass,
flat at 105–110 ms up to depth 30. A cost invariant stated for one plan shape isn't stated
at all. The differential corpus can't catch this one either: for a single-relation input,
the flat by-name fallback returns the same descriptor and only the cost differs. The
guards are `planner::tests`' `describe_over_a_*_is_not_exponential`, one per shape, plus a
structural test checking that a `Filter` keeps its input's relations addressable. No gate
measures `Describe` latency on a shape that no deadline test names.

## A third `LeafScope`, and the gates that read one

The declaration-only operand gates (`arithmetic_legality`, `negate_legality`,
`math_fn_arg_legality`, the rules that refuse `oid` / `"char"` arithmetic; their bodies
are in `docs/coercion.md`) **don't** have a reader of their own. Each asks the planner's
`Expr::infer_type` (one of the two walkers above, and the one `Describe` reports) through
whatever `LeafScope` its evaluating context can build. Grep for `impl LeafScope for` to
get the list of implementations instead of trusting a count. The ones these gates use are
the planner's `PlannerScope` and `executor::expr`'s `EvalLeafScope`. The second exists
because those gates run inside `eval_expr_full` (a grouping query's expressions
included), where the first isn't available. Its `column_type` is the
resolver-plus-outer-frame pairing that `column_declared_type` owns, and its
`subquery_type` is `Plan::output_columns` over whatever catalog the evaluation was lent
(`None` where it was lent none).

A shallow reader in that position misses shapes. One that answers `None` for an aggregate
and for a scalar subquery lets `max(conrelid) + 1` and `(SELECT conrelid …) + 1` return
integers where 18.6 raises `42883`, and drops the `+00` offset from
`(SELECT max(tz) …)::text`. The grouping executor's `certain_expr_type_oid` has the
aggregate arm but not the subquery one. **Gating on the complete walk is safe here, but
not in general.** This walk substitutes a guess where it can't resolve
(`arith_result_type` falls back to an operand, the multi-operand cells take the first
operand that types), and a guessed `text` would refuse legitimate queries at the
comparison and `LIKE` rules. That's why `certain_operand_type` still serves those
(`certain_expr_type_oid`'s only external caller is the grouping executor's aggregate
operand check). The declaration-only gate is exempt for a reason we measured, not a
hopeful one: `oid`, `"char"` and `timestamptz` each enter this walk only at a **leaf** (a
column, a cast target, a subquery's output column), so no guessing arm can invent one
from operands that don't have one.

Two things are **not** closed. **Zero rows:** the gate runs at eval time, so a statement
whose evaluation never reaches the node still answers. `SELECT max(conrelid) + 1 FROM
pg_constraint WHERE false` errors here only because an aggregate always produces its row,
while `SELECT conrelid + 1 FROM pg_constraint WHERE false` is `(0 rows)`. **Precedence:**
an eval-time gate loses to every error the evaluation raises first. So
`SELECT max(conrelid) + (1/0) FROM pg_constraint` is `22012 division by zero` here against
18.6's `42883`, and so is `SELECT -max(conrelid) FROM pg_constraint WHERE 1/0 = 1`, whose
two errors aren't even in the same clause. 18.6 refuses all of them in parse analysis.
Moving the gate to plan time would close both; that change hasn't been made.

**The type is derived once per node, not once per enclosing node.** The gate reads its
*operand's* type, so a reader that re-walks the operand subtree from each enclosing node
makes the read quadratic in expression depth. We measured 6.4× a linear shallow reader's
cost at depth 32 over 163 840 rows, on a chain of ordinary `int` additions. Instead, the
eval walk carries each node's declared type up alongside its value (`executor::expr`'s
`OperandType`), and each node derives its own type from the ones its operands derived.
It's still the same single walk doing the deriving: `Expr::infer_type_with` is
`Expr::infer_type`'s body with its recursion supplied by the caller, and
`binary_op_result_type` is that body's `BinaryOp` cell pulled out. That's the same shape
`shared_leaf_cell_type` and `shared_multi_operand_type` already have, and it's why this
isn't a fourth reader. There's an asymmetry inside it: the `BinaryOp` sites call
`binary_op_result_type` **directly**, while the deriving arms next to them reach their
rule through `infer_type_with`. What keeps the two routes equal is an invariant stated on
that function: its arm must stay a bare call to it. A node that no gate above it reads
derives nothing. The walk gets no slot to report into, so an ungated `a > 0` pays nothing
for it.

## The seam: shared rules and open-coded ones

A sweep of expressions that finds no disagreement only covers its own families, and those
are **expression** families. The scope class varies the FROM clause instead, so a clean
sweep says nothing about it. The two walkers keep their own open-coded rule tables over
the *same* `Expr` variants, minus the ones merged into a shared rule: `Default`,
`Parameter`, `Subquery`, `Array`, `VariadicFn`, `Coalesce`, `Case`, and `StringFn`'s
`DateTrunc` **member** (that variant still has an arm on each walk for its other
members). The structural defence against drift is that the rules they share live in
shared functions (`arith_result_type`, `numeric_family_result`, `aggregate_result_type`,
`math_fn_result_type`, `temporal_arith_type`, …) instead of being open-coded twice.

**A shared rule isn't enough when it reads a fact each walk would derive from its own
scope.** An aggregate's result type depends on whether its argument's integer type is
certain, and walks deriving that from their own descriptors disagreed. The planner's
grouped-column descriptor had no base table where the executor's did, and the two legs
declared different types for one value (`EDGES.md` row 2). So neither walk derives that
fact. The coercion pass records it on the aggregate (`planner::ArgCertainty`), and both
walks read it through `aggregate_type_of`, as do the gate, the pass's branch reader and
the accumulator. A relation's **column** is certain by the same kind of record from the
same producer: the pass reads the column's defining expression as written, and the plan
node keeps the type it read (`planner::CertainTypes`, `ColumnDesc::type_certain`). Only
the planner reads it; no descriptor the executor builds carries it. Both sides treating a
`Column` leaf as a scope lookup in the *projection* clauses is a shared **shape**, not a
defence. The two walks still reach it through separate code, as the next part explains.

The two leaf questions are one object, a `LeafScope` (`column_type`, `subquery_type`),
which the planner's `TypeScope`+`Schema` pair and the executor's `ProjectionScope` each
implement. But **only the `Subquery` leaf is read through it by both walks**. The
planner's `Column` arm calls `scope.column_type`, while the executor walk still reads the
inherent `ProjectionScope::resolve`, and the executor impl's `column_type` has exactly one
caller in the tree, `walker_pins`'s scope test. No answer differs, since both end up in
the same `column_desc`. But the trait isn't yet a structural defence at that leaf: an
edit to `column_type` alone wouldn't move `expr_type_oid`, and what would catch the split
is that test's boundary-identity assertion, not the shape of the code.

There are two merged rules. `shared_leaf_cell_type` holds four cells over **three** `Expr`
variants (a bare `DEFAULT`, an unbound `$N`, a scalar subquery) plus `StringFn`'s
`date_trunc` **member**, whose variant keeps an arm on each walk for its other members.
`shared_multi_operand_type` holds the four that pick one operand's type out of several
(`ARRAY[…]`, `greatest`/`least`/`nullif`, `COALESCE`, `CASE`). Where the line between
merged and unmerged falls is the *second* half of the split. The executor walk is total,
substituting `TypeOid::Text` where the planner's answers `None`, so merging a variant
whose answer moves with that substitution is a decision to make against the oracle, not a
side effect of a refactor.

**But "the two arms differ" mixes up two things, and only one of them is a decision.**
The variadic functions, `COALESCE`, `CASE` and `ARRAY[…]` differ *only* through the
recursion. So one merged arm that uses each walk's **own** walk as its operand hook
reproduces both answers exactly, with one exception at the `array_of()` step (see below).
What those four cells answer is a separate decision. The executor walk's operand hook
(`executor::helpers`'s `multi_operand_type`) declines for the four **typeless leaves** (a
bare `NULL` literal, a bound NULL, an unbound `$N`, a bare `DEFAULT`), so the shared rule
sees a real `None` and takes the first inferable operand. That matches 18.6's answer
wherever exactly **one** branch type is known **to either walk**. It isn't 18.6's *rule*,
which folds the common type of all the known branches, and where two computed branches
have different types both walks are wrong together (`EDGES.md` row 54's agree-wrong
band). The qualifier matters in the other direction too, because what 18.6 counts as
*known* differs from what this counts. A bare **string literal** is `unknown` there and
`text` to both walks here. So `COALESCE('7', length('ab'))` is `25` on both legs against
18.6's `23`, `COALESCE('abc', length('ab'))` silently answers `abc` as `text` where 18.6
gives `22P02 invalid input syntax for type integer: "abc"`, and the polarity flips at the
variadic cells: `greatest('7', length('ab'))` is refused `42804` here where 18.6 answers
`7`. That's the same literal-as-text policy that [compatibility.md](compatibility.md)'s
`COALESCE`/`CASE` row records for stored expressions, here reaching a branch set with no
concrete member. It sits next to the fold band in `EDGES.md` row 54, out of this hook's
reach (routing a literal through `Literal::infer_type` is what keeps a string typed as
`text`).

An unresolvable `Column` is neither kind. A leaf reads no operand, so its two arms differ
through the **scope**, which the trait gives one *shape* (though, as above, only the
planner's walk reads its `Column` leaf through it). Merging it means `column_type` under
the same totality boundary the others take. The only thing holding it open is the fenced
question of unknown-ness (what an unresolvable column should *declare*). The need for a
whole `ColumnDesc` belongs to `expr_output_column`, which takes that descriptor straight
from the scope on a passthrough branch that enters neither walk.

Four arms still hold a rule the other walk has no counterpart for. Each was
**re-derived** arm by arm, not inherited:

- `unnest`'s fallback to the inner type, in the **executor** walk, pinned at `unnest(1)`.
  It stays on this list next to the plan-time `unnest(<non-array>)` gate
  (`planner::legality`'s `unnest_legality`, which raises 18.6's own `42883` where
  signature resolution happens). That gate refuses the *directly typed* arguments (a
  column, a cast, a concrete literal), but it reads the narrow `arg_known_type` at its
  written reach, so an argument that falls into that reader's `_` arm still reaches both
  arms. We measured these over an **empty** `mw`, all giving `(0 rows)` against 18.6's
  `42883` and all declaring 23 on the simple leg against `Describe`'s 25:
  `unnest(i + 0)`, `unnest(length(tx))`, `unnest(abs(i))`, `unnest(COALESCE(i, 1))`,
  `unnest((SELECT i FROM mw))`, `unnest(CASE WHEN true THEN i ELSE i END)`,
  `unnest(NULLIF(i, 1))`, `unnest(GREATEST(i, 1))`, `unnest(LEAST(i, 1))` and
  `unnest(-i)`. `unnest(b + 0)` measured 20 against the same 25.
  `unnest(set_config('x', tx, false))` belongs to the same reader class. It's an
  `Expr::Alias`, which `certain_operand_type` types and this reader doesn't, so
  "computed" is the wrong name for what the class has in common. It gives the same
  `(0 rows)` against `42883`, but measured 25 on both legs instead of a split, as
  `unnest(tx || 'a')` does. **The silence comes from the empty table.** With a row, every
  one of them, *written bare like this*, is `0A000`: the scalar-position refusal, which
  fires where evaluation reaches the leaf, not wherever a row exists. So what these two
  arms declare is visible wherever a row is kept **away from** the leaf, an untaken
  branch included. Over a one-row `mw`,
  `SELECT CASE WHEN false THEN unnest(i + 0) ELSE 'x' END FROM mw` answers `x` here
  against 18.6's `42883`, declaring `23` on the simple leg against `Describe`'s `25`.
  That leg's `DataRow` is the text `x` under the `23` (the declared-vs-produced shape),
  while the extended leg's `25` and its binary payload agree. The case is recorded with
  row 55's residual in `EDGES.md`. Refusing a caller isn't the same as merging an arm;
- `abs`'s bare-literal carve-out, in the executor walk, pinned at `abs(NULL)`;
- arithmetic's `text`-left divert, in the executor walk, pinned at `NULL + 1`;
- `^`'s short-circuit of any half-unknown operand pair to `float8`, in the **planner**
  walk, where the executor walk always folds through `numeric_family_result`. It's pinned
  at `NULL ^ n`, and it's row 54's third member.

`executor::walker_pins` pins the current answers on both sides of that line, merged and
unmerged alike.

**The executor `Aggregate` arm doesn't guess `float8`.** A `sum`/`min`/`max` whose
argument the walk types as `text`, under a projection scope with no input columns, is
reached through `execute_aggregate` over a FROM whose relations are **all** zero-column
(`ALTER TABLE … DROP COLUMN` down to nothing, or a `RETURNING`-less data-modifying CTE,
which 18.6 refuses `0A000`). A `float8` guess there would declare `701` over a `text`
value and feed an enclosing aggregate's gate. `EDGES.md` rows 58 and 61 have the
measurements and the residual (`sum(NULL)`/`avg(NULL)` against `42725`).
`select/zero_column_relation_aggregate_declared_types.sql` and
`select/zero_column_relation_sum_over_text_loud.sql` pin it.

`RETURNING` reads `TypeScope::Input` over `returning_relations`, the same scope object
every other projecting node uses ([scope.md](scope.md)). A rule open-coded in both
walkers has a hazard the pair comparison can't see: duplicated rules don't only drift
apart, they can also agree on the wrong answer (the integer-width note in
[compatibility.md](compatibility.md)'s Arithmetic row is a measured example).

**Calling them "the two walkers" undercounts the seam, and that hides *three*-way
disagreements.** At least four more `Expr` walks answer the same question with an
`Option`: `executor::helpers`'s `certain_expr_type_oid` (deliberately shallow, feeding the
gates), `planner::coercion`'s `concrete_branch_type` (feeding the common-type folds), and
its `certain_operand_type` and `arg_known_type` (the comparison/`LIKE` and `unnest`
gates). Grep to re-derive the set instead of trusting this list. What divides all of them
is **what to do with an operand that can't be inferred**. The executor walk is *total* and
answers `TypeOid::Text`, the other three answer `None`, and a shared rule handed
`Some(Text)` can't tell "this is text" from "I couldn't tell". `SELECT abs(NULL)` is the
measured example. The total walk declares `text` and the planner's declares `numeric`,
where 18.6 declares `float8` (an unknown argument takes the numeric category's preferred
type; `typeconv.sgml` uses absolute value as its worked example). Since the value is NULL,
the wrong answer only shows up in the OID. The shared rule gets a real `None` for a
**bare `NULL` literal** argument (the argument, not the `MathFn` cell), so both legs
declare `float8`. The arm routes through `Literal::infer_type`, so `Literal::Null` is the
only literal that reaches it, and an argument that's only *transitively* untyped still
splits the walkers. `abs(-NULL)`, `abs(CASE WHEN true THEN NULL END)` and
`abs(COALESCE(NULL, NULL))` each declare 25 on the simple query against `Describe`'s 701,
where 18.6 refuses all three (`42725` then `42883` twice). That's measured and recorded
in `EDGES.md`. The general policy is open, and should be decided per rule against the
oracle, since unifying the walks first would only make them agree faster.

Two more rules at the same seam concern a FROM-clause `unnest(…)`, and both are about
*what the executor may derive from a value*. First, its column type comes from the array
**expression**, through a single `function_scan_column_types` that both walkers read,
because the evaluated `Value::Array`'s `element_type` can't carry the tz/no-tz bit.
Derived from the value, an `unnest(ARRAY[…::timestamptz])` column declares 1114 on the
simple leg against `Describe`'s 1184, and the column-type-aware text encoder drops the
UTC `+00` offset. Second, the *planner's* coercion scope registers an unnest's columns, so
`COALESCE` branch unification doesn't defer over one. **Deferring doesn't need every
branch to be untypable.** One untypable branch is enough for `unify_common_type_branches`
to defer, and then the executor's total walk decides by its own rule (a `COALESCE`'s
first argument's type), right or wrong depending on which branch happens to lead.
Measured against an empty map, `COALESCE(NULL, v)` declares 25 and its sibling
`COALESCE(v, 0)` 23 on the same simple-query `RowDescription`, where 18.6 says 23 for
both. The second is the fixture's polarity pin. Registering the unnest's columns decides
two more verdicts; see the `unnest` bullet in `EDGES.md`.

**The coercion pass defers a second way, and the deferral depends on the *branch set*, not
on the cell.** The coercion pass hides the `ARRAY` / `greatest` / `COALESCE` / `CASE`
splits from SQL only for some branch sets, and the mechanism is two readers:

- `COALESCE` / `CASE` / `ARRAY[…]` go through `unify_common_type_branches`, which asks
  `select_common_type` for a target and returns the branches **untouched** on
  `Ok(None)`. That's its answer when *no* branch is concrete to `concrete_branch_type`.
- `greatest`/`least` go through `unify_variadic_args` and `nullif` through
  `coerce_nullif_args`, both over `arg_known_type`, which yields no target the same way.

**The two readers aren't one walk**, and the difference is a list of arms, not a policy.
Over an otherwise identical `Column` / `Cast` / `CastTypmod` / literal set,
`concrete_branch_type` has an `Array` arm (the elements' common type, one dimension up)
and an `Alias` arm that `arg_known_type` lacks. Both have the `Aggregate` arm (its
argument's type, recursed), with `arg_known_type` delegating it. We measured that this is
exactly where the family's boundary runs. `COALESCE(NULL, sum(b))` and
`COALESCE(NULL, ARRAY[1,2])` are rewritten and fall outside it (1700 and 1007 on both
legs, as on 18.6). `greatest(NULL, ARRAY[1,2])` and its `least`/`nullif` spellings fall
**inside** it: raw, they split 25 on the simple leg against `Describe`'s 1007, where 18.6
says 1007, and the executor walk's operand hook (below) decides them. No fixture names
them, because none builds an array next to a bare `NULL` in a variadic.

The `Alias` arm is **reachable but moves no declared type**, which is the weaker of the
two facts. `set_config(<name>, <value>, <is_local>)` lowers to an `Expr::Alias` wrapping
`<value>`. It's the only production `Alias` mint that nests inside another expression.
The others all sit at a list's top level: projection item, `VALUES` row, sort junk,
`JOIN … USING` merged column, `planner::query`'s `coerce_unknown_entry` (a set-op side's
`unknown` output entry rewritten to a `Cast` the alias names), and `carried_key`, which
re-wraps a bare `Int` literal lifted out of a projection entry so the sort key it becomes
isn't read back as positional. `carried_key` is the one an enumeration easily misses,
because it mints on the **right side of a `match` arm** instead of into a `push` or a
`return`, so a sweep that reads `=>` lines as patterns rather than constructors never
sees it. It replaces the whole `OrderKey.expr` and wraps nothing else, so it moves no
declared type either. Over `(i INT, tx TEXT)`, `COALESCE(NULL, set_config('x', tx, false))`
and its `greatest`/`least`/`nullif` spellings are 25 on both legs and the `i` spelling is
23 on both, since both walks read through the alias to the inner expression either way.

**What the arm missing from `arg_known_type` changes is a gate verdict**, a consequence of
this arm list that no declared-type comparison can see. The `42804` branch-unification
gate reads through `branch_type_with_subqueries` (and, for a set holding an enum,
`complete_resolved`), which is `concrete_branch_type` plus the scalar-subquery arm
([coercion.md](coercion.md) § What the gate reads), for all five constructs it covers:
`COALESCE`, `CASE`, `ARRAY[…]`, `greatest`, `least`. `nullif` isn't in that match at all
and reaches the operand gate next to it instead. So over a `greatest`/`least`/`nullif`,
the *pass* is the blinder of the two readers. It folds the concrete sibling alone and
wraps the argument it can't see in a `Cast` to that target, and the gate then reads that
`Cast` at an agreeing type and never fires. **That needs a concrete sibling to fold.**
With none, `variadic_common_type` answers `None`, the pass rewrites nothing, and the gate
types the raw pair itself: `greatest(ARRAY[1,2], set_config('x', tx, false))` is `42804`
on both engines. Over `(i INT, tx TEXT, a INT[])` holding `(7, '7', {1,2})`, on both
legs, `greatest(ARRAY[1,2], tx)` answers 25 over `{1,2}`, `least` 25 over `7`, `nullif`
25 over `{1,2}`, and the `(set_config('x', tx, false), i)` spellings answer 23 over `7`,
`7` and NULL, where 18.6 refuses each with `42804` (`nullif` with `42883`). The same
`int[]` through an arm both readers have is refused as it should be (`greatest(a, tx)`,
`greatest(a::int[], tx)`, `42804` on both engines). The `COALESCE` family, where pass and
gate use the same reader, is unaffected (`COALESCE(i, ARRAY[1,2])`,
`COALESCE(set_config('x', tx, false), i)`, `42804` on both). `EDGES.md` row 65's third
sub-case has the streams and the bounds.

What both readers *do* share is a `None` for every **computed** expression *outside* the
arms just named and the numeric-category computed shapes both now type
(`docs/coercion.md` § A computed branch is typed where its type is certain): a string
function other than `length`/`strpos`/`position`, temporal arithmetic, a subquery, an
`Expr::Bound`, a `$N`, a `DEFAULT`. The scope matters here: `sum(b)` is a function call
and is **not** `None` to `concrete_branch_type`, which is the whole of the arm list above.
So a branch set of `{a bare NULL} ∪ {a computed expression}` has no target and reaches
both walkers raw. The spellings that do get hidden all have a concrete literal branch
(`COALESCE(NULL,1)`, `ARRAY[NULL,1]`, …). Raw, the walkers split in every projecting
position we checked against 18.6 (a plain projection, `RETURNING`, a `GROUP BY`/`ORDER BY`
ordinal, a derived table, a `VALUES` row, a scalar-subquery operand, an `unnest` wrapper,
a nested `COALESCE`, an empty relation and `LIMIT 0`), with the planner walk right at
every one. `COALESCE(NULL, length('ab'))` declares 25 on the simple leg against 18.6's
23. `COALESCE(NULL, date_trunc('day', ts))` reaches a **value**: the column-type-aware
text encoder sends `2024-01-15 00:00:00` where 18.6 sends `2024-01-15 00:00:00+00`, and
the table-free `COALESCE(NULL, now())` gives the same corrupted value with no setup at
all.

**Not every consequence of the split is silent.** The executor walk's answer becomes the
projecting node's `ColumnDesc`, which an enclosing aggregate's operand gate reads (row
58's mechanism). So a split `SELECT sum(c) FROM (SELECT COALESCE(NULL, length('ab')) AS c) s`
is `42883 function sum(text) does not exist` where 18.6 answers `2`, a refused statement
instead of a mislabelled column. This is closed by the executor walk's operand hook (see
§ The seam: shared rules and open-coded ones). The guards are
`select/{coalesce,case,variadic,array}_untyped_first_*`,
`select/coalesce_untyped_first_branch_timestamptz.sql`, and
`select/coalesce_bound_param_null_aggregate_operand.sql`. The last is the bound-`$N`
leaf's guard. It holds that loud consequence over the extended protocol, because the
leaf's own OID-and-value divergence is invisible to a binary-format `Bind` (the
`timestamptz` corruption only happens in text format, and `@param` requests binary).

The residual is the same as the `abs(-NULL)` family's: an operand that's only
*transitively* untyped keeps the total answer. So `COALESCE(-NULL, length('ab'))`,
`COALESCE(CASE WHEN true THEN NULL END, length('ab'))` and
`COALESCE(COALESCE(NULL, NULL), length('ab'))` still declare 25 against `Describe`'s 23.
18.6 refuses all three instead of answering with the other branch (`42725` for the first,
`42804` for the two whose untyped branch resolves to `text`), so the right fix is a
refusal, not the other branch's type. The all-untyped shapes are the controls:
`COALESCE(NULL, NULL)`, `greatest(NULL, NULL)`, `nullif(NULL, NULL)` and
`CASE WHEN true THEN NULL END` are 25 on both legs and on 18.6, and `ARRAY[NULL, NULL]`
is 1009 on all four.

**`Ok(None)` has a third consequence, and it isn't about a declared type at all.** The
branch-unification **gate**, `planner::coercion`'s `check_branch_unification_node`
(pglet's counterpart of 18.6's `42804`), reads the same `concrete_branch_type` (through
`branch_type_with_subqueries`), and `select_common_type` folds only the branches it
resolves. So an invisible branch is never compared against a sibling, **whether or not
the set has a concrete member** (`COALESCE(i, upper(tx))` answers `7` where 18.6
refuses). These branch sets are the sub-case where nothing gets compared at all: the fold
is `[None, None]`, the answer is `Ok(None)`, and nothing is raised. pglet then *answers*
where 18.6 refuses. `COALESCE(length('ab'), upper('7'))` is int4 `2` here,
`ARRAY[length('ab'), upper('7')]` is `_int4` `{2,7}`, and
`COALESCE(upper('x'), length('ab'))` is text `X`. The answer is decided by whichever
branch leads, over a pair 18.6 won't match at all. "Leads" means two different things, and
a short-circuiting cell separates them: the walkers declare the first *inferable* branch,
while evaluation returns the branch it **takes**. So
`CASE WHEN false THEN length('ab') ELSE upper('x') END` declares 23 and streams the single
byte `58`, a declared type over a value of another type. That's `EDGES.md` row 36's
shape, reached with no `$N` and no NULL. `COALESCE(i, tx)` is the control showing the
gate exists: `42804` on both engines. This is a **wrong answer**, not a wrong label, so it
has its own row (`EDGES.md` row 65) instead of being a residual of this one. It shares
the fold band's root cause: widening `concrete_branch_type`/`arg_known_type` past
everything they answer `None` for (the computed expressions, and the `Expr::Bound`
constants such as an `INTERVAL` literal) is what closes both.

## What each walker is handed: one resolver, one coercion pass

The executor's descriptor walk resolves through the same `SimpleColumns` resolver that
binds the row's values, so a declared type can't name a different column than the one the
value came from. It types a scalar subquery through the planner's schema-aware walk
instead of a second, schema-blind one. Upstream of both, the coercion pass rewrites a
branch into an `Expr::Cast` only where it can resolve a target, so a branch set it
resolves nothing for reaches the two walkers raw. Every split recorded in `EDGES.md` rows
54 and 55 is measured under that condition.

The `abs` carve-out above is narrow on purpose. Every other shape keeps the total walk,
because the shallow `certain_expr_type_oid` would answer `None` for an ordinary computed
argument and turn `abs(v + 1)` over an `int` column into `float8`.

## The four typeless leaves, and the routes that do not reach them

The shared multi-operand rule gets a real `None` for four **typeless leaves**: a bare
`NULL` literal, a bound NULL, an unbound `$N`, and a bare `DEFAULT`. It's the same
carve-out shape `abs` uses. The `$N` leaf is an **undeclared** parameter. A parameter
whose type the client declared at `Parse` reaches both walks as a `Cast` over the leaf, so
it's typed and isn't one of the four (`docs/param_binding.md` § A declared type is part
of parse analysis). Which of the four ordinary SQL can reach, and by what route, varies.

The `Literal(Null)` and `Parameter` leaves are live from ordinary SQL. The bare `DEFAULT`
is live wherever a **short-circuiting** cell operand goes unevaluated: a `CASE` result
branch (raw, `SELECT CASE WHEN false THEN DEFAULT ELSE length('ab') END` declares `25`
against `Describe`'s `23`, and the `now()` spelling reaches the dropped-offset value),
and equally a trailing `COALESCE` operand, a `WHEN` condition past the taken arm, a
simple-form `CASE` and a whole nested cell, with the row's own values deciding which
(row 56's rule and its short-circuit entries; it can't be fixtured, since that acceptance is
row 56's). A NULL-valued `Expr::Bound` has **no route we've found** into a multi-operand
cell. There are four production mints, and each is ruled out by its **own** property, not
a shared one:

- `parser::ddl`'s `column_default_to_expr` reaches `Expr::Bound` only for values with no
  `Literal` spelling.
- The `INSERT` source-row substitution replaces a whole column slot instead of nesting.
- `executor::dml`'s `substitute_nextval` *does* nest. It rewrites through `map_children`,
  so `DEFAULT COALESCE(nextval('s'), 1)` mints a `Bound` inside the cell (measured: a
  sequence set to 100 stores `101` then `102`, and `greatest(nextval('s'), 5)` over one
  set to 200 stores `201` then `202`). But both `SequenceDraw` arms yield a
  `Value::Int8`.
- `parser::expr`'s `parse_interval_literal` nests too, and carries an interval.

So nesting is **not** what rules any of them out.

## A query-position `unnest` is refused at plan time

pglet raises **18.6's `42883` where signature resolution happens**:
`planner::legality`'s `unnest_legality`, called from `check_expr_node_semantics`'s single
bottom-up, left-to-right plan-time walk. Everything below follows from where the check
runs, not from the rule itself:

- **It doesn't depend on rows, because it reads a declared type.** Left to evaluation, the
  verdict depends on whether evaluation reaches the leaf. It's silent over an empty table
  and otherwise fails with one of *two* wrong codes (`0A000 set-returning unnest() is only
  supported in a SELECT projection`, the scalar-position refusal a row-bearing
  `Plan::Project` reaches; `0A000 unnest requires an array argument, got Int4(1)` from
  `execute_project_unnest`, where a no-FROM projection's single row reaches it). That's
  row 56's mechanism, and the gate takes this leaf out of it entirely. Measured: `42883`
  over an empty table, over rows, over `WHERE false`, inside an untaken `CASE` branch and
  inside a short-circuited `COALESCE`.
- **It isn't a row-60 case.** Checked over raw v3 against both engines, the refusal
  arrives at `Parse` (`E Z`, with no `ParseComplete`) on pglet as on 18.6, so the phase
  agrees as well as the code. That's the reason for putting it at plan time instead of in
  the executor.
- **The position that counts is the expression's, not the projection clause's.**
  `RETURNING`, a sort key, a `WHERE`, an `UPDATE … SET` right-hand side, a set-op arm, a
  CTE and an `INSERT … SELECT` source each reach the same walk, and each is `42883` on
  both engines (each measured). Left to evaluation, the last two accept the statement
  (`INSERT 0 0` / `UPDATE 0`) **only where evaluation doesn't reach the leaf**, and that's
  a different emptiness for each. `INSERT INTO dst SELECT unnest(i) FROM mw` is
  `INSERT 0 0` over an empty `mw` and `0A000` with a row in it. The CTE spelling
  `WITH c AS (SELECT unnest(i) AS q FROM mw) UPDATE dst SET q = 1 WHERE q IN (SELECT q
  FROM c)` is `UPDATE 0` whether or not `mw` holds a row, because reaching the CTE takes a
  row in **`dst`**; with one, it's `0A000` too. Both are row 56's mechanism, and the
  answer is *quiet* only for the empty shape.
- **Which error wins, at two scales, both checked on 18.6 first.** Within the expression
  the walk is bottom-up, so the argument's own binding failure beats the signature refusal
  built on it (`unnest(nosuchcol)` is `42703`). Between clauses, the whole target list is
  analysed before the `WHERE`. pglet reproduces that because `lower_select_level` lowers
  the projection before the predicate, item by item in source order:
  `SELECT unnest(i) FROM t WHERE nosuchcol = 1` and `SELECT unnest(i), nosuchcol FROM t`
  are `42883`, while `SELECT nosuchcol, unnest(i) FROM t` is `42703`. Two neighbouring
  cases also match 18.6: `SELECT unnest(i), 'abc'::int FROM t` is `42883`, not `22P02`
  (the coercion pass runs per item, so item one's gate fires before item two is folded),
  and `… HAVING count(*) > 0` is `42883`, not `42803`.

## Coverage and limits

Only the text leg needed building. tokio-postgres throws away the simple query's
`RowDescription`, so a hand-rolled probe reads it, while the extended leg's OIDs come for
free from `Statement::columns` on the prepared statement the binary path already sends.
The two legs also cover **different corpora**: `@param` and `@no-txn` are extended-only
(the simple query can't bind), and `@text` is text-only. So the extended leg adds
coverage even where the walkers agree. (The text leg excludes `@no-txn` fixtures for a
recorded reason, in `has_text_leg`'s doc in `test_fixtures.rs`: that leg's types come
from a *second* connection replaying the setup, which a fixture driving its own
transactions can't be assumed to survive. Lifting the exclusion hasn't been measured
under the current cross-connection rule, where that replaying connection's write next to
a block that has written waits for it, and gets `55P03` past the default `1s`
`lock_timeout`.) Together the legs reach **roughly two thirds of the corpus**. The run
prints the exact per-leg split instead of us fixing a number here, since it moves with
every fixture added. The rest is gated out on purpose (`@expect-error` on both legs, the
per-leg exclusions above, and a `@no-declared-types:` opt-out that waives **both** legs by
design, since a per-leg waiver would hide the internal split) or has no columns to
compare.

Two known limits apply to both legs. The first: **typmod isn't compared**, so a column can
be declared the right type with the wrong precision/length and still pass. We tried
turning it on, and what it found is a **class**, not a count. An **explicit cast's**
typmod is `-1` here where 18.6 sends the modifier's encoding. Measured over the wire with
node-postgres's `dataTypeModifier`, `SELECT '…'::timestamp(3), 'abcd'::varchar(7)` reads
`[-1, -1]` here against `[3, 11]` there. `ColumnDesc::new` defaults the field to `-1`, and
the only sites that write anything else are the two base-table-column ones
(`planner::plan`'s `table_columns_desc`, `executor::helpers`'s `table_column_descs`)
**plus the recursive-CTE pair below**. So every *computed* column has the default no
matter what the cast says. How many fixtures that turns red changes with the corpus, so
it's deliberately not given here. A value constructor is in the same class and costs more
than a wrong width. 18.6's `exprTypmod` gives a `CASE`, `COALESCE` or `GREATEST` its
arms' common modifier when they agree, and pglet gives it `-1`, so the two engines
disagree about a modifier **18.6 knows exactly**. The same goes for a column that two
sources unify or that a subquery passes up (a set operation, a `JOIN … USING` merge, a
scalar subquery) when the sources agree: 18.6 keeps their modifier
(`SELECT v … UNION ALL SELECT v …` over a `varchar(5)` is `9` there) and pglet sends `-1`
on both legs. All three are `EDGES.md` row 155.

### A recursive CTE's declared modifier

A recursive CTE that does recurse has its columns rebuilt instead of propagated
(`parser::select`'s `recursive_cte_columns`). That makes the modifier a decision rather
than something inherited, and the two candidate answers aren't equally safe. Taking the
**seed's** modifier is what 18.6 always reports, but that's because 18.6 *refuses* every
pair whose two terms disagree (`42804 … has type character varying(5) in non-recursive
term but type character varying overall`, comparing `exprTypmod` against the set
operation's resolved column). pglet doesn't refuse that pair, so declaring the seed's
`character varying(5)` over a `UNION ALL SELECT 'abcdefgh'::varchar(8)` gives a width
that a `DataRow` two frames later violates.

So the modifier is **resolved**, by `planner::common_type`'s `select_common_typmod`.
That's 18.6's own function, and it's asymmetric in a way worth remembering: it keeps the
pair's modifier only while both sides agree *and* both types are the common one. So a
seed declaring none can never gain one, while a seed declaring a width loses it to any
term pglet doesn't read as declaring the same. `check_recursive_shape` returns that answer
per column, and the caller writes it over **both** readers of the list: the
`Plan::CteScan` the body resolves against (`Describe`) and the `CteBody::Recursive` the
executor materializes (the simple-query `RowDescription`). Writing it over only one would
be a two-producer split, which `crates/pglet/tests/cte_column_list.rs` catches.

pglet does **not** raise 18.6's `42804` on the modifier, and the reason is this document's
subject: it can't compute `exprTypmod`. We measured that raising without it refuses a
whole family of statements 18.6 answers: a cast to the seed's *own* width
(`a::varchar(5)`, `'xy'::varchar(5)`, `1.5::numeric(6,2)`) and a value constructor over
the working-table column (`CASE`, `COALESCE`, `GREATEST`, `NULLIF`, a scalar subquery).
The cases are kept as a table in `crates/pglet/tests/cte_column_list.rs` instead of being
counted in prose.

**What's left at the pairs 18.6 refuses is over-acceptance (`42804` there, an answer
here), and it splits into two subsets with different fixes:**

- **A term pglet reads as `-1` where 18.6 knows a modifier**: a cast, a value constructor
  (`EDGES.md` row 155). pglet declares `-1` and answers. The same read gives the
  neighbouring family that 18.6 *answers* (the cast to the seed's own width, the
  constructors) a declared `-1` where 18.6 declares the seed's modifier, which is an
  under-declaration. This subset and that neighbour wait on row 155: no comparison can
  match 18.6 while one side reads a default.
- **A term pglet reads exactly, with a different modifier**: a base column projected
  directly. Examples are a `VARCHAR(8)` or `VARCHAR(3)` column next to a `VARCHAR(5)`
  seed, a `NUMERIC(10,5)` next to a `NUMERIC(6,2)`, and a `TIMESTAMP(3)` column next to a
  `TIMESTAMPTZ(3)` seed, whose modifiers agree as numbers over two types. pglet reads
  both sides here, declares `-1`, and doesn't need row 155 to see the disagreement.
  18.6's own comparison (the set operation's resolved modifier against the seed's), made
  only where both sides' modifiers are *read*, could close this subset without touching
  the first.

**Any such refusal would rest on the premise that a read modifier is the column's own.**
A non-negative modifier is minted only at the base-column sites (`planner::plan`'s
`table_columns_desc`, `executor::helpers`'s `table_column_descs`) and by the recursive
CTE's own pair. Every other site mints `-1` or copies an existing descriptor. A copy that
*picks* its descriptor by name is where the premise can break, and on the planner's side
the picking is `project_output_columns`' lookup through `find_projected_column`, which
the `Window` arm shares. Two routes could take another relation's same-named column
there, and both are closed (`EDGES.md` row 158): a window query's own lookup by bare
name, and a qualified reference to a FROM-clause `unnest` that puts no relation in scope
for the qualifier to find. If either were open, a term next to a `VARCHAR(5)` seed would
read `9` for a `varchar(8)` or `text` column, which the fold keeps, so the CTE would
declare `character varying(5)` over a row of eight characters. That was measured on both
legs of the native server against 18.6, which refuses that statement `42804`. Next to a
`VARCHAR(8)` seed the window term reads `12` and the CTE declares `12`, as 18.6 does.

`find_projected_column` reads a reference the input doesn't bind as an **enclosing**
level's, using the node's record of the column it binds to (`EnclosingColumns`, modifier
`-1`), and declines one the record doesn't hold (row 159). We didn't find that route
reaching this fold: a `WITH` inside a scalar subquery is `0A000` here and a `LATERAL`
derived table is `42601`, so no recursive term we measured has an enclosing level to
correlate to.

No fixture can see any of this, since neither leg compares a modifier, so what's pinned
is pinned in Rust.

The *column* half is the one that does agree, at the width 18.6 sends. Over
`wtm (a timestamp(3), b timestamptz(2), c time(1), d interval(4), e varchar(7),
f timestamp, g timestamp(3)[])`, both engines send `[3, 2, 1, 2147418116, 11, -1, 3]`,
including the interval's packed range mask and the array column's *element* width. That
half is pinned on the catalog side by `catalog/precision_atttypmod.sql` and
`array/typmod_column_atttypmod.sql`. Over the **wire** nothing pins it, because
`pglet_tests`' `parse_row_description` skips past the field. The other limit is that
duplicate output column names collapse in the JSON keying, so give such a `@test`
distinct `AS` labels. A third, bigger caveat isn't a limit of the check but of the corpus:
it only sees SQL that some fixture issues, so a green run says as much about coverage as
about correctness.
