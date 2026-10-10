# Column-reference scope and binding

How a column reference finds (or loudly fails to find) its relation: eval-time
resolution against the frame chain, the plan-time binding gate, and the
`RETURNING` namespace and which of its readers share one derivation. The rules are
summarized in the repo `CLAUDE.md`; this file carries the full statements. The
divergences that remain are recorded in `EDGES.md`.

## Eval-time binding — the plan is never mutated to bind

Parameters and correlated columns resolve at eval time against an `EvalEnv` (a
`params` slice + an outer-scope frame chain threaded through
`execute_plan_node` / `eval_expr_env`), so a prepared statement holds one
immutable cached plan and a correlated subquery pushes the outer row onto the
env instead of deep-cloning the inner plan per row. Resolution is
**alias-aware** (`Plan::Scan` carries its FROM alias, so a qualified `c.id`
binds to the scope aliased `c`) and **level-aware**: once a query level's own
scope exists, the plan-time pass re-runs over each sub-plan against a
*level-chained* scope, so a correlated subquery's literal coerces against the
enclosing column — and a set operation whose side projects an enclosing column is
unified there, the first place that column's type is known (`planner::query`'s
`OuterTyping`: `DELETE FROM t WHERE EXISTS (SELECT i EXCEPT SELECT '7')` compared
the `integer` `7` with the `text` `'7'` and deleted the row 18.6 keeps). What that
unification produces is then **built into the statement again** rather than patched into
the plan the readers already saw — § A pending set-op column's type is never read. Innermost level wins and cross-level ambiguity is impossible
(Postgres's `colNameToVar` rule); a relation the scope builder cannot enumerate
makes its level uncertain — resolution stops rather than risk binding a name
past an unknown relation. That is a per-**relation** fact and not a per-level
flag, which is what lets a relation the scope can name and cannot fully
enumerate (a catalog view, where pglet models a strict subset of 18.6's
columns) answer its own qualifier while still stopping the walk — and what lets a
FROM-clause function (`unnest(…)`, `generate_series(…)`) be an ordinary named relation,
registered with exactly its declared columns. See `planner/correlation.rs`.

The eval-time frame chain has its own statement of that, and it is **one walk
with a four-way verdict**: `Found` / `Claimed` / `Ambiguous` / `Unclaimed`. A
frame **owns** a qualifier when it *is* that relation (`table == Some(t)`) or,
having no identity, when it addresses it among several by a synthesized
`"t.column"` name — the split `SimpleColumns::resolve` gates its own dotted
tier by, because a user's quoted `"a.b"` column (or `AS "a.b"` alias) is
byte-identical to that addressing. Ungated, a frame that knew it was `q1`
answered for a relation named `a` and
`DELETE FROM q1 WHERE EXISTS (SELECT 1 FROM q2 WHERE q2.yv = a.b)` **destroyed
a row**. Three rules follow, and **each of the three, broken, writes**. *The first
owning frame stops the walk, resolved or not* — its column,
else `42703 "t.name"`, never a rebinding further out: with `a(id)` outside and
`b(v)` between, `DELETE FROM a WHERE EXISTS (SELECT 1 FROM b WHERE EXISTS
(… k.k = a.v))` bound `b`'s `v` and removed **every** row, against 18.6's
`42703` hinting `b.v`. *A qualifier no frame owns leaves the chain* rather than
falling to a bare match, so the inner resolver reports its own `42P01`: a plain
typo destroyed a row (`… u2b.yv = zzz.v`), an alias-hidden table name wrote
`99` where the `SimpleColumns` spelling of the same `eref` rule already
refused, and `ON CONFLICT … DO UPDATE SET v = zzz.v` / `SET v = "EXCLUDED".v` /
`… WHERE zzz.v = <the excluded value>` each **stored the excluded value**,
since the action pushes `excluded` innermost. *A bare name counts **exact**
claimants per frame* — one resolves, two or more are `42702`, none walks
outward — which keeps a `USING`-merged `id` resolving (it is one exact claimant
ahead of the raw `a.id`/`b.id`) while a derived table projecting
`a AS x, b AS x` refuses instead of taking select-list order. The ambiguity is
final at the level that claims it, the **inner** scope included, so
`resolve_column` returns the inner resolver's `AmbiguousColumn` rather than
escalating — that escalation deleted the outer row of `DELETE FROM o WHERE
EXISTS (SELECT 1 FROM c JOIN d ON … WHERE v = 1)`, and could surface as a
*different* loud error (`42883 text = integer`) when the outer column's type
differed, which reads like an operand bug and is this one.
`column_declared_type` takes the same four-way match and answers `None` on the
two raising arms; the pairing is structural rather than tested — a mutation making them disagree reddens nothing, because
both are reached from walks that evaluate the value first and its error wins.

**But a gate on the identity is only worth what the call sites put there, and
that is the half a rule written about the gate does not state.** Both readers
switch behaviour on the same `None` — the dotted tier above, and
`SimpleColumns`'s loose last tier, which drops the qualifier and matches the
bare name — so a scope that *has* a relation and is handed `None` does not lose
alias-awareness, it answers for relations the statement never named, through
whichever of the two the spelling reaches. Two call sites are where that bites, and
the first is a **write**: with `execute_update` passing the statement's FROM *alias*,
an unaliased `UPDATE u1 SET id = 99 WHERE EXISTS (SELECT 1 FROM u2 WHERE u2.yv = a.b)`
updates a row over `u1(id, "a.b")` — while the *aliased* spelling of the identical
statement is `42P01` on both, the control that identifies the identity rather than the
statement kind — and `UPDATE ex1 SET v = 42 WHERE bogusrel.n = 1` writes `42`; without
a `Plan::FunctionScan` arm in `single_table_name`, `SELECT (SELECT count(*) FROM u2 WHERE
u2.yv = a.b) FROM unnest(ARRAY['hit','miss']) AS v("a.b")` and
`SELECT zzz.b FROM unnest(ARRAY['x','y']) AS v(b)` both answer rows. 18.6 is
`42P01` to all four. The name to pass is Postgres's `eref` — the alias when one
is written, the relation's own name when none is — so the aliased spellings
keep refusing the hidden table name. Several walks derive a plan's relation
name and they do not cover the same FROM kinds; `single_table_name`'s doc says
which carry a `FunctionScan` arm, and for which functions.

**A reader over a node's output names learns its level from the plan, so a qualifier the
level owns never reaches the chain.** `resolve_column` hands a qualified reference to the
inner resolver first only where `scope_has_table` holds, and the resolvers the `Sort`,
`Aggregate`, `Window` and `DistinctOn` nodes build over their input's names have no relation
identity — those names are a projection's output, or a FROM pipeline's evaluated by a node
above it. Each is told the effective names of its query level's FROM items
(`SimpleColumns::within_level`), read from the plan by `executor::query::scan`'s
`level_relation_names`: down through the nodes a level stacks over its FROM clause, into
both sides of a join, and stopping at each FROM item, so a derived table's or a set
operation's own names are never read as this level's. Without them, a qualifier both a
sub-select's FROM item and an enclosing one carried read the enclosing row in an aggregate's
argument, a `GROUP BY` key, `HAVING`, a grouped or windowed select list, and an `ORDER BY`
or window key: over `t (y, n)` holding `10`, `20` and `ev (n)` holding `5`, `7`, `SELECT y,
(SELECT max(o.n) FROM ev o) FROM t o` answered `10`, `20` against 18.6's `7`, `7`, and the
same binding moved a `DELETE`'s rows and an `INSERT … SELECT`'s stored values (`EDGES.md`
row 323). A qualifier the level owns binds a column of that relation, and what decides how
is the level's structure, never a column name's text: where the node reads a join's rows,
the input's `(relation, column)` pairs (`LevelScope::pairs`, from `scan::plan_scope`);
otherwise the bare name, exactly. A suffix match is another relation's column — over
`pg_class o JOIN w` it answered `w.k` for `max(o.k)` where 18.6 refuses — and a name's dots
are no relation: a user's quoted `"o.z"` column is an ordinary column of `o`. The declared
type and the enum order take the same route as the value, all three branching on
`scope_has_table`, and the aggregate and window nodes describe their output — declared
types, an aggregate's operand check, `MIN` / `MAX`'s enum order — through the resolver they
evaluate with (`ProjectionScope::with_resolver`), so a dotted user column such as `"o.n"`
cannot lend `o.n` its type. A sort above a projection reads output names, which carry no relation, so
an owned qualifier there binds whatever output column carries the name — refused at plan
time for an ordinary table, and open for a catalog view (`EDGES.md` row 325).
`eval_expr_env` takes no column names, so these nodes build their resolver themselves rather
than one per row with no level. A `DISTINCT ON` key reaches its dedup re-tied to an output
name, so no statement measured reaches that node's resolver with a qualifier; it is told its
level all the same, so the four nodes build their resolver one way. Guarded by the
`select/inner_alias_shadows_outer_*` fixtures, both polarities, and `executor::expr`'s
`level_names_tests`.

**The whole chain is at eval, which bounds every rule above it**: a statement
matching no row never reaches it, and a `RETURNING`-position reference is
projected after the writes. That is why the executor's rules are only half of
it — the plan-time binding gate below is the other half, and neither is
optional. The rules here are guarded by
`crates/pglet/tests/scope_relation_identity.rs` for the row survival
(`@expect-error` compares SQLSTATEs and is green for a refusal raised *after*
the writes) beside the `correlated_outer_ref_*` fixtures for the SQLSTATEs,
each red on the revert of its own rule. A frame carries its scope's
`(relation, column)` pairs — derived from the plan, so no spelling forges one: an
identity-less frame owning a qualifier by a `"t."` prefix is forged by a quoted
`"a.c"` column and answers a bare `c` over `f3i("a.c", "b.c")` `42702`, where both
engines answer the row — `"a.c"` and `"b.c"` being two ordinary column names.

## A grouping query's aggregates are values the scalar walk reads by node

A `Plan::Aggregate` evaluates its `HAVING` predicate and its select list — the junk
columns the planner adds to it for `ORDER BY` keys among them — through the **ordinary
scalar walk** (`eval_expr_full`), over the group's
first row, and an `Expr::Aggregate` the walk reaches answers the value the grouping
computed for **that node** over the group — read through the `SubqueryExec::aggregate`
lookup the grouping lends the walk (`executor::query::aggregate`). It is the eval-time
binding rule above with one more kind of leaf: the plan is never rewritten to carry a
value. The lookup is keyed by the node's identity, so `sum(a)` written twice is two
entries, and a node the grouping computed nothing for is refused `0A000` rather than
answered from another node's value. A sub-plan's evaluation builds its own
`SubqueryExec`, so an inner query's aggregate is never read from an outer query's group;
and a position no grouping lends the lookup — a `RETURNING` aggregate (`EDGES.md` row 4)
— keeps the refusal. Because the walk is the scalar one, every node it evaluates carries
an aggregate below it with no list of its own: `round(avg(x), 2)`, `CASE WHEN count(*) >
0 THEN … END`, `greatest(max(a), 1)`, `jsonb_build_object('n', count(*))`, and the
operand gates — `max(conrelid) + 1` is `42883` as `conrelid + 1` is. An arm that
evaluated a child through a context without the lookup would refuse an aggregate below it
again; `crates/pglet/tests/aggregate_walker_arms.rs` runs an aggregate under each `Expr`
variant the walk evaluates an operand of, in the projection, an `ORDER BY` key and
`HAVING`.

**Which aggregates a group computes is not "the ones the walk reaches".** 18.6 computes
every aggregate that survives its planner's constant folding (`eval_const_expressions`)
for every group, whether or not the expression above it reads the value: `CASE WHEN
count(*) > 0 THEN max(a) ELSE sum(1/b) END` raises the division's `22012` over a `b = 0`
row, while `CASE WHEN false THEN sum(1/b) END` answers `NULL`, its aggregate dropped with
the branch. So the grouping first collects the aggregates under each expression
(`Collector`), computes them, and only then runs the walk.

**A branch is dropped only under a decider that is certain by its shape, and every other
aggregate is computed.** Whether 18.6's planner folds a decider is a property of every
function, operator, cast and type under it — its volatility, its collation, the session
— and the two ways of modelling that wrong are not alike: an aggregate kept where 18.6
dropped it can only raise where 18.6 answers, and one dropped where 18.6 kept it can
**answer where 18.6 raises**. So the collection takes for a decider only what is a
constant by construction, whatever stands around it:

- the literals `true`, `false` and `NULL`;
- a comparison (`=`, `<>`, `<`, `<=`, `>`, `>=`) of two integer or `numeric` literals;
- `NOT`, `AND`, `OR`, `IS [NOT] NULL` and `IS [NOT] TRUE | FALSE | UNKNOWN` over those.

What a certain decider drops is a branch the scalar walk does not evaluate either: the
right operand of an `AND` / `OR` its **left** operand decides; a `CASE` arm whose
condition is certainly not true, and every arm and the `ELSE` after one whose condition
certainly is — a simple `CASE` likewise where its operand and the arm's value are both
integer or `numeric` literals; and the arguments of a `COALESCE` after an integer,
`numeric` or boolean literal, bare or under the cast the coercion pass brings it to the
`COALESCE`'s type by. Nothing else decides anything here — not a string comparison, a
bound `$N`, arithmetic over constants, a function call, a cast, an `IN` list or a strict
operator over a `NULL` — and where 18.6 folds one of those, a failing aggregate in the
branch it dropped raises here: loud, the class `EDGES.md` row 89's residuals measure.

A kept branch is kept whole. A `CASE` a certain condition reduces to one arm does not
make that arm lazy: `CASE WHEN true THEN (CASE WHEN a > 0 THEN 1 ELSE sum(1/b) END) END`
still computes the inner `ELSE`'s `sum`, as 18.6 does — evaluating an arm only as far
as the walk reads it would answer where 18.6 raises.

**Dropping an aggregate drops its computation, and not its operand's type check where
the operand is a column or a literal.** 18.6 resolves an aggregate against its argument's
type in parse analysis, before its planner folds anything, so `true OR sum(text_col) > 0`
is `42883` there whatever the rows. The collection therefore walks a dropped branch too,
collecting nothing, and refuses each aggregate it meets that has no form for its
operand's type — kept or dropped, and before any group is formed — where it reads that
type without evaluating anything: a column's **declared** type, and a literal's. A
dropped aggregate over a cast, which is what a typed constant and a declared `$N` are,
takes the verdict over the cast's target, nothing evaluating its argument. A kept one
evaluates its argument per row, and its verdict waits for `compute_aggregate`: alone in
the statement, a refusal of the cast itself is then raised first, as 18.6 raises it
(`max(int_col::jsonb)` is `42846` on both); beside another aggregate the collection
refuses, that refusal comes first here (`EDGES.md` row 304).

What the check does not type is judged where the aggregate is computed, so under a
dropped branch, or over no group, not at all: an argument whose type or refusal only
evaluation shows (`sum(upper(int_col))`), a bare `$N`, a `DEFAULT`, and a kept aggregate
over a cast. And the check trusts the declared type: where a column's declaration is a
guess — a derived column that is a correlated scalar subquery, an outer-level aggregate's
argument — the refusal is one too, and it has no form for a `time` operand of `sum` /
`avg`, which 18.6 resolves through an implicit cast (`EDGES.md` rows 89, 60 and 307).

**The collection evaluates nothing but those deciders.** A constant in the select list
or `HAVING` is evaluated by the scalar walk where the walk reaches it, as in a query that
groups nothing: `CASE WHEN length('abc') = 3 THEN 1 ELSE 1/0 END … GROUP BY a` answers
`1`, and a constant in a branch no group takes, or in a query that forms no group, is not
evaluated. Where such a constant fails, that is 18.6's answer if its planner folded the
branch away or never planned the expression, and an answer where 18.6 raises if its
planner folded the constant (`EDGES.md` row 60 owns that class, grouped or not). A
failing constant the walk does reach is raised after the group's aggregates are
computed, so a failing aggregate outranks it (row 304).

**The order within a group is pglet's.** A group's `HAVING` aggregates are computed and
the predicate evaluated before its select-list aggregates are computed, so a group
`HAVING` discards computes none of the select list's — not 18.6's order, which advances
every aggregate over each input row before evaluating anything above one (`EDGES.md`
row 304).

## A pending set-op column's type is never read

A set operation is built as its sides are parsed, and a side's entry naming a column of
an **enclosing** query level has no type at that point: inside an expression subquery the
enclosing level's `FROM` may not even have been read yet. Such a column is left
**pending** with a provisional type (`planner::query`'s `OuterTyping::Deferring`), and
the correlated re-walk unifies it once the level-chained scope exists.

Between those two moments the node is an ordinary plan, and everything that asks it for
its columns is handed the provisional type: an enclosing set operation's own
unification, a derived table's column, a scalar subquery's common type in a `COALESCE`,
an `INSERT … SELECT` source's assignment types, both declared-type walkers. That reader
list is not enumerable — it is every caller of `Plan::output_columns` on a plan whose
rows flow from the node — so **no reader is made to wait**. What the re-walk produces is
built into the statement again instead: each unified column's enclosing-level reference
types, enum identity included, are recorded on the node (`SetOpOuterTypes`, a handle the
parser holds too), and `Parser::parse_statement` **parses the statement a second time**
from the same token checkpoint, building each such node complete with those types
(`SetOpTyping::Resolved`). The plan the second parse *returns* holds no pending column,
so no reader of it can see a provisional type — a node still pending there is refused
instead (below); the first parse's plan is dropped.

The consequences worth stating:

- **A column the re-walk never unified has no recorded type**, so the second parse leaves
  it pending again — and that plan is *refused* (`0A000`) rather than returned. The
  measured route is a `VALUES` cell, which no re-walk visits:
  `INSERT INTO t VALUES ((SELECT zz EXCEPT SELECT 1))` is that refusal here against 18.6's
  `42703` for the name.
- **A node the re-walk never *reached* takes the hints of one built equal to it, and the
  record is no part of a plan's identity.** The re-walk visits the plan, so an expression
  a later rewrite discards is never unified — `SELECT DISTINCT (SELECT i EXCEPT SELECT
  '7') FROM t ORDER BY (SELECT i EXCEPT SELECT '7')` matches the sort key to the target
  entry it equals and keeps only the entry. Two things make that statement answer. The
  records are excluded from `Plan` equality, so the two occurrences are one expression to
  the match that compares them; and the key's own record, left with nothing, takes the
  entry's hints — the two nodes were built equal, which is *why* the match took one for
  the other, and building them differently in the second parse is what breaks it. Equality
  is of the nodes **as built**, before the re-walk wrote its unified types into them.

  **What an unhinted record takes is not the refusal above.** With the borrowing removed,
  measured, the statement above is
  `42P10 for SELECT DISTINCT, ORDER BY expressions must appear in select list`: the second
  parse builds the key unlike the entry, the match misses, the key survives into the plan
  and `SELECT DISTINCT`'s own gate raises — an error `parse_statement` returns from
  `parse_statement_once` **before** it reaches the `unresolved` check at all. So what the
  borrowing moves is that `42P10`, not the pending-column `0A000`, and nothing about its
  scope or its soundness follows from that refusal.

  **And which record is borrowed from is order-dependent**, because `Plan` equality is
  blind to the enclosing level a correlated name resolves at: two nodes spelled alike whose
  bare column binds at different levels compare equal here, and `hinted.iter().find` takes
  the **first** of them in build order. Over `pp(i INT)` and `qq(i TEXT)`,
  `SELECT DISTINCT (SELECT i EXCEPT SELECT '7') AS z, (SELECT (SELECT i EXCEPT SELECT '7')
  FROM qq) AS y FROM pp ORDER BY (SELECT i EXCEPT SELECT '7')` answers as 18.6 does, and the
  same two entries **swapped** is `42P10` here where 18.6 still answers — the key's orphan
  borrows the nested entry's hints rather than its own, and the second parse builds it
  unlike the entry it must match. So the equality does not ask whether two nodes are the
  same expression; it asks whether they were built alike, which is the weaker question.
  Every measured wrong borrow costs a refusal rather than an answer, and nothing here
  bounds it to that. That gap is open, and the measured pair is `EDGES.md` row 153 —
  which also says what a repair has to preserve: 18.6 answers **both** orders, so refusing
  the answering one as well would make the pair consistent and both halves wrong.
- **The first parse's errors are the statement's errors.** A reader that raises on the
  provisional type raises before the re-walk can correct it, and there is no second parse
  to reach: over a `numeric` `n`,
  `EXISTS (SELECT 1 FROM (SELECT n EXCEPT SELECT 1) s WHERE n = '1.25')` is `22P02` here
  where 18.6 answers the row. That is the loud residual of this design, recorded in
  [compatibility.md](compatibility.md)'s set-operation row.
- **A clone reports to the same record.** A read-only CTE is inlined at each reference, so
  two plans can share one node's record; a second unification that disagrees with the
  first is recorded as a conflict, and a conflicted record hints nothing — so the
  statement takes the refusal above rather than building on either answer.

## A column reference binds at plan time, and the gate closes only over a namespace it can prove whole

The eval-time rules above cannot be the whole story, because an error they
raise per row is not raised at all when there is no row: `SELECT 1 FROM <empty>
WHERE zzz.id = 1`, `UPDATE t SET v = zzz.v WHERE 1 = 0` and `DELETE FROM t
WHERE EXISTS (SELECT 1 FROM <empty> WHERE u.k = zzz.v)` all report success under
the eval-time rules alone, where 18.6 refuses in parse analysis. And a whole family of
readers has no identity to gate with at all — the post-projection resolvers behind
`ORDER BY` / `GROUP BY` / `HAVING` / window keys and a join's projection — where a
bogus qualifier would answer off the loose bare/suffix tier, and **delete a row** when
a write reaches one through a subquery. The planner knows the FROM
namespace where those readers do not, so `ColumnScope::check_binding` closes
the class once rather than per site (`EDGES.md`). The gate judges a reference and does not
route it: a qualified column that a sub-select's own FROM item and an enclosing level's both
carry passes it, and which of the two the value comes from is the eval-time resolver's
(§ Eval-time binding, *a reader over a node's output names learns its level*).

**What makes it safe is that "whole namespace" is computed, not assumed.** A
level's scope is complete at its own lowering exactly when no *expression*
subquery encloses it — a depth the parser counts around `(SELECT …)` /
`EXISTS (…)` / `IN (SELECT …)` and **resets** for a derived table or CTE, whose
FROM cannot see the enclosing query; a level inside one stays provisional until
`resolve_correlated_subplans` re-walks it against the level-chained scope.
Closing regardless makes an ordinary two-level correlated reference `42P01`,
because at the middle query's own lowering the outermost relation does not
exist yet. And the verdict is **three-way**, not two: `Bound` / an error /
**`Uncertain`**, because a relation the scope cannot name or cannot enumerate
might carry the reference — which is why the catalog relations pglet models only in
part register non-exhaustively and answer `(0 rows)` where an exhaustive gate would
invent a `42703`, `pg_constraint` and `pg_index` excepted: those two register
exhaustively although modelled in part, as the relations modelled whole do, answering
`42703` for an unmodelled column rather than a fabricated one. `CatalogTable::columns_complete` is
the one fact that decides which side a relation is on. **But the
set of kinds that get that treatment is a claim about what the builder *cannot*
produce, not about what it has not been taught to walk, and three builders
derive it separately.** A FROM-clause function item (`unnest(…)`,
`generate_series(…)`) is a function RTE with exactly its declared columns, and all three — `push_level_relations` (the correlated
sub-plan's scope), `push_plan_relations` (an `UPDATE … FROM`'s) and
`build_column_scope` (the one the gate closes over) — register it that way, through
one `push_partial_relation` → `push_relation`. A builder registering it as a *subset*
answers by route: `UPDATE pt SET v='x' FROM unnest(…) u(x) WHERE bogus = 1` is 18.6's
`42703` one level down and `(0 rows)` at the statement's own level — through `SELECT`,
`GROUP BY`, `ORDER BY`, `HAVING`, a nested sort key (rows, sorted, against a refusal)
and an unreferenced CTE's `DELETE`, which removes every row. A neighbouring shape is
not this bit's to close: two FROM items sharing an alias, which 18.6 refuses `42712`
and pglet does not gate at all, answer a loud wrong code for the spellings the
collapsed column map leaves unserved and still answer rows for the spellings the
surviving relation's columns serve — the repair there is the `42712` gate, not this
bit.

Two more rules are Postgres's own rather than caution: only a **bare
single-field** `ORDER BY` / `DISTINCT ON` key may name an output column
(`findTargetlistEntrySQL92`), so such a key is judged against the level's
**target list** first and falls through to the range table only when no entry
carries the name — measured, dropping that claimant set (a bare key judged
against the FROM relations alone) reddens the output-alias keys under `select/order_by_*`, `select/window_*` and
`select/distinct_on_alias_key`, the `types/coerce_*` / `types/group_by_*`
clause-pairing set, `select/join_dotted_column_*`, and two `spine/auth`
queries. Those keys are resolved **after** the projection, which is where that
list finally is — `transformSelectStmt`'s own position for the sort clause, and
what makes the target-list match possible at all — and a `JOIN … USING (c)`
merges `c` into one column, a fact that shifts outward with its level so a
correlated subquery sees it too.

## A sort key is bound where its claimant set exists, and that is a different site in each clause position

An `ORDER BY` / `DISTINCT ON` key is the one reference whose namespace is not
the FROM clause alone: Postgres's `findTargetlistEntrySQL92` lets a **bare
single-field** key denote a target-list entry no relation carries. So the gate
over such a key cannot run where the rest of the level's references are judged
— at a level's own lowering the target list does not exist yet — and a gate
that cannot see the claimant set can only decline to judge every bare name.
A blanket decline on `ColumnScope` would leave every level the correlated
re-walk (`planner/correlation.rs`'s `key_scope`) never reaches with a key binding to
nothing past plan time — **and a statement matching no row never reaches the
eval-time resolver either**, so nothing would judge it at all. So there is no such
decline: each position a key can be written in is gated where its own claimant set
is whole — a level's own lowering, a set operation's trailing clause, a no-FROM
select's tail, and, for a level inside an expression subquery, the correlated
re-walk that already carried the set. They are separate sites, not one, and a
repair at any of them leaves the others as they were.

**At a level's own lowering, after the projection.** The two calls sit just
after the `WHERE` and `HAVING`, `ORDER BY` first, which is `transformSelectStmt`'s
own order (FROM, target list, `WHERE`, `HAVING`, sort, group, distinct), and
they read `correlation::target_list_entries` — the sibling of the re-walk's
derivation over the same `entry_identity`, so the two phases cannot drift about
what makes two entries one claimant. Each key's binding **interleaves with its
positional range check** in source order, which is a second decision and a
visible one: `ORDER BY bogus, 9` is `42703` where a check-then-bind order gives
`42P10`, `ORDER BY 9, bogusrel.x` is `42P10`, and `DISTINCT ON (bogusdon, 9)`
is `42703`, with `ORDER BY 9, bogus` and `DISTINCT ON (9, bogusdon)` staying
`42P10` as the other half of each pair.

Binding the keys behind the clauses `transformSelectStmt` transforms first is
what puts these error-precedence pairs in 18.6's order, over `ep(a INT, s TEXT)`:
`SELECT bogusproj FROM ep ORDER BY bogusrel.a` is `42703`,
`SELECT a FROM ep WHERE bogusw = 1 ORDER BY bogusrel.a` is `42703`, and
`SELECT s = 5 FROM ep ORDER BY bogusrel.a` is `42883` — where a key bound first
answers its own `42P01`. `GROUP BY` is the control that fixes the position rather than
merely moving it:
`SELECT count(*) FROM ep GROUP BY bogusg ORDER BY bogusrel.a` is `42P01` on
both engines, in either binding order. Two more move only the *message*, so no fixture
holds them: `GROUP BY bogusg ORDER BY bogus` names the sort key as 18.6 does rather
than the grouping key, and
`DISTINCT ON (bogusdon) … ORDER BY bogusord` names the sort key.

What it costs is a widening rather than a new class. `check_binding`'s recorded
causes of refusing two entries 18.6 calls `equal()` are reached at a level's own
lowering as well as from a *nested* level, and independently of the row count: over an
empty `ja2(c, x)` and a two-row one, each is `42702` at either row count, against
18.6's rows throughout. No accept shape measured moves with the binding site: a sweep
over output aliases, dropped base columns, `USING` merges, positional keys,
aggregate/window/`DISTINCT`/`DISTINCT ON` shapes, derived tables, CTEs, correlated
subqueries, `unnest`, a set op and a catalog view answers byte-identically either way.
`EDGES.md` carries the causes; retiring them is their own item.

**On a set operation's trailing clause, at that clause's own lowering.**
`planner/query.rs`'s `plan_set_op_order_limit` closes hard, because that
namespace is complete by construction: 18.6 restricts a set-op's trailing keys
to the set-op's own output names, refusing a correlated reference and any
computed expression alike with `0A000 invalid UNION/INTERSECT/EXCEPT ORDER BY
clause`, so there is no enclosing level to wait for. The correlated re-walk's
`Plan::SetOp { .. } => scope.push_opaque_relation()` stays — it protects names
in the set-op's *arms*, and removing it was only ever measured as a probe. A
whole key that **is** an `Expr::Column` is judged against an empty closed scope
carrying the output names as claimants, with no expression identity on any of
them (18.6 calls `a AS x, a AS x … ORDER BY x` `42702` even though the entries
are identical), interleaved with the positional range check in source order
(`ORDER BY bogus, 99` is `42703` on both, `ORDER BY 99, bogus` is `42P10`). A
key that is *not* a bare column routes its leaves the way `scope_for_key`
routes a whole key, which is what makes `-a` bind and `bogus+1` refuse:
`transformSetOperationStmt` adds the output names to the namespace as a join
RTE precisely "so that `ORDER BY upper(foo)` will draw the right error message
rather than `foo not found`".

The gate is keyed on the **tree** being a set op rather than on the parser arm,
because the paren-leading arm also serves a parenthesized plain select whose
sort clause 18.6 folds into the inner query —
`WITH c AS ((SELECT a FROM pt) ORDER BY b) DELETE FROM n8`
answers on both engines and a per-arm gate would
refuse it. A `Plan::Sort` over the tree is peeled before that test, because a
parenthesized operand may carry its own `ORDER BY`; the peel cannot reach the
plain-select case, since nothing under a `Plan::Sort` over a plain select is a
`Plan::SetOp` and the walk stops on its own.

**On a no-FROM select's trailing tail.** The tail routes through `plan_no_from_order_limit`, which binds its keys as well as
applying them, and the arm makes the `parse_for_locking()` call its siblings
make. Routing alone does not judge a key: a `Plan::Sort` over a `Plan::Values`
judges its key only where a row reaches the eval-time resolver, so the keys are
bound at plan time, by the machinery the no-FROM `DISTINCT ON` gate uses — and,
as there, **conditionally on scope completeness**:
`SELECT (SELECT 1 AS q ORDER BY ct.a) AS r FROM ct` answers on both engines,
so an unconditional close would refuse it.

The same arm decides the **quantifier**: `SELECT DISTINCT` builds a node as
`DISTINCT ON` builds its `Plan::DistinctOn` — building none would be sound over the
one-row `Plan::Values` the arm was written for and a complete no-op over
`Plan::ProjectUnnest`, which row-expands.
The node is built unconditionally rather than only over a row-expanding
projection — a shape test would be a second rule to keep in step with
`plan_select_no_from`'s — and it is built in the **leaf**, so both routes out of
it carry the quantifier: the single select and `finalize_leaf`'s set-op operand.
The one exclusion is a **sequence projection**, which the executor dispatches on
`Plan::is_sequence_projection` ahead of the tail; there the elision is sound for
the quantifier (a dedup over a single row returns that row) and open for the
bound (`EDGES.md` row 30).

## A level's scope is closed by a reader, and a FROM item is judged by a re-walk that has to reach it

A query level's scope is complete at its own lowering exactly when no
*expression* subquery encloses it, and a level inside one stays **provisional**
until `resolve_correlated_subplans` re-walks it against the level-chained
scope. A statement's own level also judges each sub-plan, on a copy, where its binding
gate reaches the sub-plan, so the verdict keeps that level's walk order
(`correlation::judge_nested_level`). Each re-walk also records, on the level's projecting
node, the identity of every target-list reference an enclosing level binds
(`EnclosingColumns`, shared with that copy), which is what the level's output descriptor
reads for it (`EDGES.md` row 159). A `DELETE`'s namespace is its target and an `UPDATE`'s its target plus
its `FROM`, so both close outright; a **no-FROM** select runs the pass over its
projection's sub-plans because nothing else would; and the pass always rewrites
a nested level, an enclosing level with no relations included — precisely the one
whose bogus qualifiers have nowhere left to hide. Two things follow that are easy to
state and each writes when it fails: *someone* must make that re-walk call for every
leaf, and a level left provisional must be reachable by some re-walk that can decide
it.

**The call.** `parser::select`'s `finalize_leaf` calls `resolve_correlated_subplans`
on a `SelectLeaf::NoFrom` operand, as its `parse_select_inner` twin does on the
identical leaf. Without it a level inside such a leaf keeps the open scope its own
lowering gave it — an expression subquery's scope is provisional by design, since it
may still name the enclosing query's relations — and no second reader ever closes it,
so a write through it destroys every row (`EDGES.md` row 30). The call belongs in
the shared function rather than at one caller: the leaf on the **left** of a
set op reaches `finalize_leaf` through `parse_select_inner`'s own call site and
not `parse_setop_operand`'s, and writes identically without it. It is handed the same
`scope_is_complete()` its twin passes, which is what keeps the pass from
inventing a refusal where the leaf may legitimately name an enclosing relation
(`SELECT a FROM ct WHERE EXISTS (SELECT 1 AS z UNION ALL SELECT ct.a)` answers
on both engines). The rule is independent of the trailing-tail routing above: a
set-op *operand* carries no tail of its own.

**The reachability.** `Parser::in_from_item` resets `expr_subquery_depth` to
`0` for a derived table's or a CTE's body, on the reasoning that a
`LATERAL`-less FROM item cannot see the enclosing query's relations. That
reasoning is **half of Postgres's rule**: such an item is outside its **own
level's** namespace — a sibling FROM relation is an error, not a binding —
while the enclosing *levels* stay visible exactly as they are to an expression
subquery. So a FROM item nested inside an `EXISTS`/`IN`/scalar subquery is
handed a closed scope that is missing every enclosing relation, and any gate
reading it invents a `42P01` over ordinary correlated SQL. That over-rejection
is live, with its members in `EDGES.md`.

The repair direction is that such an item must stay *provisional* — the
enclosing chain is not whole yet — so the gate declines at that item's own
lowering and the re-walk judges its keys once the chain exists. Spelled as a
one-line change to `in_from_item`, that is *reset only when the depth is
already `0`*. It does not reopen a row-destroying silent write through an
**unreferenced CTE** — whose body never runs, so no eval-time resolver ever
sees its keys — because a top-level CTE body is still at depth `0` and still
resets. What refutes the one-line change is that, probed adversarially, it turns 18.6-matching refusals into silently
accepted wrong answers, eight of them writes 18.6 refuses. Two routes carry
that, an `INSERT`'s clauses re-walked by nobody and a chain holding a
non-exhaustively-registered relation answering `Uncertain` — the re-walk
declines, and the provisional item is judged by nobody either. So the rule is
not *reset only at depth `0`* but: **a FROM item may be left provisional only
where some `resolve_correlated_subplans` re-walk will reach it with a complete
chain and a decidable namespace.** Which of the two routes is closed and what
the other still writes are `EDGES.md`'s.

Both ends of that probe were measured. On the accept side, the spelling the change
actually makes provisional stays refused with its rows intact —
`WITH c AS (SELECT (SELECT 1 FROM (SELECT 1 AS q ORDER BY zzz) x) AS z) DELETE FROM n8`
is `42703` under it, the enclosing level's re-walk reaching the item. On the refuting
side, the worst two of the eight writes are a `DELETE` and an
`UPDATE` driven through a
`NOT EXISTS` over a derived table beneath an `information_schema` level, each
destroying or overwriting every row where 18.6 refuses the statement outright.

## Which readers of a level's uncertainty move when the bit does

The readers of the columns-exhaustive bit split on whether a relation kind's
registration flip can change their answer, and the split is a fact about this
scope rather than about their rules — so it is worth re-deriving from the
field's callers rather than inherited from here. `binding_qualified`,
`binding_bare` and `ColumnScope::sole_relation_carrying` are the movers — the
last declines outright on an uncertain level, so no `ORDER BY` key over such a
level can be bound to its relation when `output_name_binding` decides whether
two same-named target entries are one expression or two. `bare_candidates` and
`qualified` consult `level_columns_uncertain` only to decide whether to keep
walking **outward**, and `build_column_scope` has nowhere outward to go:
`push_scope_relation` stamps every relation level `0`, and the only production
`nested_in` (`planner::correlation`'s `resolve_level`) chains a scope
`push_level_relations` built. `levels()` is therefore `0..=0` in every scope
`build_column_scope` produces, and both readers answer identically either way.
The reading to distrust is *a reader consults the field, therefore the field's flip
moves it*.

## A synthesized `relation.column` name is not a pair, and three alternatives that pass only some of the rows

A join scope that *encodes* each column's `(relation, column)` pair in the column's
own name, for every consumer to re-parse, is forgeable: a user's quoted `"a.b"` column
— or an `AS "a.b"` alias — is byte-identical to such a synthesized name, so the
forgery runs both ways: a relation the query never named becomes addressable, while
the pair the scan's own alias spells is never registered at all. The executor's pairs
come from the plan (`executor::query::plan_scope`), which no spelling can forge, and the scope
carries them as pairs rather than as strings: `register_input_columns` files
each input column under its plan-derived relation, `SimpleColumns` and the
correlated `OuterFrame` resolve through the pairs, and a `RETURNING` frame is
handed the same namespace its clause already resolved through.

**The correlated outer frame is the flat-string half, and it has two readers
and two producers.** A frame is `(col_names, table, row)`, and a *dotted* entry
in `col_names` arrives two ways: `returning_input_columns` renames a
**multi**-relation `RETURNING` row's descriptors to `relation.column`, and a
**user** quotes a dot into a column name or into an `AS` alias — which lands in
*any* frame, a single-relation one that carries its relation identity included.
`EvalEnv::resolve_outer_slot` reads those names on both of its paths: the
**qualified** one (an exact `"t.name"` match) and the **unqualified** one (an
exact bare match). The dotted tier answers a qualified reference only against a frame
that does not know its relation, gated on `f.table.is_none()` — the gate
`SimpleColumns::resolve` carries on its own dotted tier. Two resolvers agreeing on
tier *order* and not on the gate is not alignment: the gate is the half whose absence
destroys rows.

**A frame lacking an identity is not the same fact as a frame being
multi-relation.** A single-relation scope can still be handed `table: None` by its
call site — two were measured, on statements both engines accept as SQL and 18.6
answers `42P01` — so the call sites, not the gate, are what give a single-relation
frame its identity, and they are above.

**What the pair derivation is not**, kept because each alternative passes some of
the measured rows and none passes all. `JoinColumnMap` is pair-keyed already, so
nothing is closed by giving it pair keying — and the `ON`-position statements
resolve through it, so a change confined to `SimpleColumns` cannot reach
them. Nor is it "stop splitting": for a left-deep `Join(Join(a, b), c)` the
inner join's output columns arrive at the outer level already named
`a.account_id` and `get_table_name_from_plan` answers `"joined"` for a `Join`,
so the split is what keeps a qualified reference two joins deep addressable. Nor
is splitting at the first dot given a known join scope: a relation aliased
`"a.b"` then splits at the wrong place, which
`select/join_dotted_alias_relation.sql` pins.

**Those scopes' identity decides error precedence, and the clause where it agrees
with 18.6 is the `WHERE`.** A bogus qualifier in the `WHERE` answers `42P01` ahead of
— each answering instead with the identity reverted — a malformed `SET` value
(`22P02`), a unique violation (`23505`), a missing relation in `RETURNING` (`42P01`
naming *that* relation), a missing relation in a `SET` right-hand side (`0A000`), and
a second bogus qualifier in the `SET` right-hand side (`UPDATE r1 SET v = bogusrel.v
WHERE zzz.id = 1`, which reverted silently succeeds). At an unnest
level it outranks a neighbour **written after it in the `WHERE`**, in two
spellings: a missing column — `SELECT b FROM unnest(ARRAY['x']) AS v(b) WHERE
zzz.b = 'x' AND v.nosuch = 'y'` is `42P01 "zzz"` on both engines and `42703
"v.nosuch"` with the identity reverted — and an ill-typed one, `… WHERE zzz.b =
'x' AND b + 1 > 0`, likewise `42P01 "zzz"` on both and `42804` reverted. The
**position in the clause is part of each pair**: write the neighbour first and
all three builds answer the neighbour instead — `… WHERE v.nosuch = 'y' AND
zzz.b = 'x'` is `42703` on both engines, and `… WHERE b + 1 > 0 AND zzz.b =
'x'` is `42804` here against 18.6's `42883`. The target-list direction does not
agree and is `EDGES.md`'s.

## A FROM name is resolved by the item's kind, never by asking the catalog first

A FROM item's effective name **replaces** the namespace entry for that name
across the whole query level: `addRangeTableEntry` registers the derived
table's, CTE's or function's eref and nothing else, so a base table that
happens to share the spelling is not in scope at all. `SELECT z FROM (SELECT 1
AS z) AS t` answers `1` on 18.6 even when a table `t` exists, and `SELECT a
FROM (SELECT 1 AS z) AS t` is `42703` even when that table has an `a`. A CTE or
derived table named for the table it derives from is ordinary SQL, so this is
not a corner.

Anything that turns a FROM name into a relation must therefore dispatch on the
item's **kind**. Those that match on the plan node cannot mis-resolve this way —
`plan_relations`/`push_plan_relations` and `push_level_relations`, plus
`collect_nonbase_into`, which builds the map the name-keyed readers consult. The readers keyed on the *name* are `build_column_scope`
and `expand_qualified_wildcard`; the first consults the
non-base maps ahead of `schema.get_table`, which answers the wrong relation whenever
the spellings collide. `expand_qualified_wildcard` asks the non-base map first and so
is right for the kinds that map holds — but it is not a plan-node reader, and the kind
that map omits is exactly where it mis-resolves; its bullet is below with the live
defect. (`validate_grouped_columns` reads the FROM name through the catalog too and
stays there, its verdict measured rather than assumed — see the residuals.) The list
is worth re-deriving from the callers of `Schema::get_table` rather than inherited
from here. **A reader's own *call sites* need the same re-derivation**:
`expand_qualified_wildcard` has two, and its defect reads as an `alias.*` one unless
the second — a bare `*` — is derived from `expand_wildcards` rather than from the
bullet below.

- **`build_column_scope`**, the scope the plan-time binding gate, the coercion
  pass and the static comparison gate all read. Its two non-base maps
  (`function_scan_columns`, `collect_nonbase_columns`) are keyed by effective FROM
  name and hold exactly the FROM items of those kinds — neither recurses into a
  `SubqueryScan`'s body, so their keys are this level's items and no other's —
  which makes a hit in one *proof* of the kind, where a catalog hit proves only
  a shared spelling. The one thing that can forge such a hit is two FROM items
  sharing a name, which 18.6 refuses `42712` and pglet does not gate; the last
  paragraph below is what that costs. The lookup is ordered function item →
  derived/CTE/catalog-view → base table → unenumerable.
- **`expand_qualified_wildcard`**, which expands `<name>.*` — and, through a
  second call site in `expand_wildcards`'s per-FROM-item loop, a bare `*` over a
  FROM clause with no join **whenever that `*` is not the projection's only
  item**. A *sole* `*` there reaches neither call site: `expand_wildcards`
  returns it untouched for the `Plan::Project` fast path — which is why the
  defect below reads as an `alias.*`-only one and is not. It reads
  `nonbase` before `schema.get_table`, so
  the derived-table, CTE and catalog-view kinds are *proved* — each measured
  with a base table of that name in the catalog: `SELECT d.* FROM (SELECT 1 AS
  z) AS d` and `WITH d AS (SELECT 1 AS z) SELECT d.* FROM d` are `z` = `1` on
  both engines beside a `d(a)` holding `9`, and `SELECT t.* FROM
  information_schema.tables t` beside a base `t(zzcol)` expands the view's
  columns rather than `zzcol` (that it emits four of them where 18.6 emits
  twelve is the separate modeled-subset divergence). A FROM-clause
  `generate_series(…)` is proved the same way: `collect_nonbase_into` has its arm, and
  `SELECT g.*, *, 1 FROM generate_series(1, 2) g` beside a base `g(q, r)` expands the
  series' one column, as 18.6 does. `collect_nonbase_into` has no arm for an `unnest`
  item, though — deliberately, since widening it would move
  `SELECT u.* FROM unnest(…) AS u`, a divergence of its own, as a side effect of
  a scope fix — so a FROM-clause `unnest(…)` is the one kind whose name reaches
  the catalog here. The expansion then emits the *base table's* column names for
  the unnest's relation, so the polarity is decided by whether those names are a
  subset of the alias list's: a subset is silent (one column answered where 18.6
  sends two, or the unnest's values under the table's labels and order), a
  non-subset `42703`. Both live, and **both repeat verbatim at the bare-`*` call
  site**: beside a base `uu(q INT)`, `SELECT *, 1 FROM unnest(ARRAY[1,2],
  ARRAY[3,4]) AS uu(q,r)` answers `q | ?column?` where 18.6 sends `q | r |
  ?column?`, while the sole-`*` spelling of the same statement is correct on both
  engines. `EDGES.md`'s *addressable by its alias in a
  value position and not* bullet carries the members, the controls and the one
  spelling the kind dispatch moves; the fix is the `unnest` arm that bullet already
  names, not a reordering of this reader, and one arm closes both call sites.

**Measured members, 18.6 first.** The accept polarity — `1` there, `42703`
here — reached the bare name, the qualified name, `alias.*`, `ORDER BY z`,
`GROUP BY z`, `WHERE z = 1`, and a correlated inner shadow, through both the
derived-table and the CTE spelling.

**A base table carrying the same name at a different type separates the
binding from the typing, and only that spelling reaches `coercion_type`.**
Where the base table lacks the column, a mis-resolution fails the binding gate
and the `42703` above is the whole story. Where it carries it — `shadow(z
TEXT)` shadowed by `(SELECT 5 AS z)` — the reference *binds*, to the wrong
relation, and `ColumnScope::coercion_type` hands the predicate's `'5'` a `text`
target instead of `int4`: `SELECT z FROM (SELECT 5 AS z) AS shadow WHERE z =
'5'` is, catalog-first, `42883 operator does not exist: integer = text` (the eval-time
rulebook judging the derived table's int4 row value against a literal the
plan-time pass typed off the base table) against 18.6's row. A fixture group
written only against the binding gate cannot reach that polarity;
`select/derived_table_alias_shadow_comparison_coerces.sql` is the one that
does.

The **silent** polarity needs an empty derived table, because the base table's
columns *were* bound and only the executor's row resolver would have refused
them: `SELECT a FROM (SELECT a AS z FROM t WHERE false) AS t` and its `t.a`
spelling answer `(0 rows)` catalog-first against 18.6's `42703`. The `unnest` alias
members are the same defect (`SELECT z FROM unnest(ARRAY[1,2]) rv_u(z)`
`42703` catalog-first against two rows there, and `SELECT rv_u.x FROM
unnest(ARRAY[]::int[]) rv_u(z)` `(0 rows)` against `42703` there) and are
closed by the same dispatch.

**What makes a catalog-first lookup a wrong refusal rather than a quiet
mis-resolution** is the plan-time `unnest(<non-array>)` gate, which reads its
argument's type from this same scope: catalog-first, `SELECT unnest(a) FROM (SELECT
arr AS a FROM t) AS t` is `42883 function unnest(integer) does not exist` — the
shadowed table's `a INT` — where 18.6 answers, and the suppression polarity pglet's
own `0A000` where 18.6 raises that `42883`. A gate is only as good as the type it is handed.

**What it does not reach**, each measured rather than reasoned about. A
projection-position `unnest(…)` over a
FROM clause is `0A000` here whatever its argument, unchanged by any of this,
which is why the accept-polarity `unnest` fixtures carry an empty table. A
shadowing derived table's `z + 1` is `42804`, not 18.6's `42883` — the general
operator-code class, which answers `42804` for `'a'::text + 1` with no shadow
anywhere. And `validate_grouped_columns` reads the FROM name through the catalog too
and stays there, because across every probed spelling its verdict is the one it
gives without a collision: `SELECT z, w FROM (SELECT 1 AS z, 2 AS w) AS <name>
GROUP BY z` answers here and is `42803` there whether or not `<name>` is a
table, and the shadowed spellings that *do* raise `42803` here raise it there
too. The reason to expect that, rather than a proof of it: pglet's
functional-dependency relaxation only ever widens acceptance, and 18.6 grants a
derived table no dependency to relax by — so the collision can move this gate
toward 18.6's answer or toward the leniency already recorded for a
non-colliding derived table, but not into a refusal 18.6 does not share.

**Two FROM items sharing a name are 18.6's `42712`, a gate pglet does not
have**, and the kind dispatch moves which of its spellings are silent rather than
closing any. Over `FROM t, (SELECT 1 AS z) AS t` the **qualified-column** pair
trades against a catalog-first lookup: `SELECT t.z` answers `1` where catalog-first is
`42703`, `SELECT t.a` is `42703` where catalog-first answers `1`, and `SELECT z`/
`SELECT a` swap `42703` and `42702` between the two — a silent member for a silent
member across those four spellings, with `SELECT 1` answering a row under either
lookup as the fifth. **Those five are not the class**, and the sentence is bounded to
them. Those five repeat
verdict-for-verdict across all three FROM forms — comma-with-derived-table,
`JOIN`-with-derived-table, and comma-with-`unnest` — and three further spellings
are recorded beside them, each over the derived-table form and each measured under
both lookups: `SELECT count(*)` answers `1`, `SELECT *` is `42703`
naming a different column each time, and **`SELECT t.*` splits by the
second item's kind**, which is why a count is the wrong instrument. Of the three,
only `t.*` breaks the across-forms repetition, and it breaks it on the
**verdict**; what the other two show is that a repeating verdict is not a
repeating *value* — re-measured across the forms, the unnest form's two rows make
`count(*)` answer `2` there against the derived table's `1` (and `SELECT t.z`
answer `1, 2`, and `SELECT 1` answer two rows), while every one of those
verdicts is unchanged. Over a
derived table `t.*` answers the derived table's `1` where catalog-first is `42703` — a
silent member the kind dispatch **adds** with nothing traded for it; over an `unnest`
(`FROM t, unnest(ARRAY[1,2]) AS t(z)`) it runs the other way, `42703` where
catalog-first silently answers the base table's `9` — a silent member the dispatch
**removes**. Every one of them is a wrong answer or a wrong code to a statement
18.6 refuses outright. As with the same class under the unnest arm, the repair is
the `42712` gate, which closes every spelling at once.

## A clause's namespace is one object, and every reader of that clause reads it

A `RETURNING` item is read by the plan-time binding gate and the wildcard
expansion (`lower_returning`), the planner's declared-type walk
(`Plan::returning_output_columns`), the executor's projection and the evaluator
under it, the prepared-statement parameter inference (`Inference::walk_returning`),
and the correlated re-walk (`correlation::walk_level`) — which for an `INSERT` reads
the shared derivation through `insert_returning_relations` and for an
`UPDATE`/`DELETE` hands the items that level's own scope. Each can hold its own idea
of what the clause can name: a reader on the catalog scope mistypes a `$N`
compared with a source column (`int8` for 18.6's `int4` over `dt(id, b BIGINT)` /
`ds(id, b INT) s`, and OID **0** for a source column the target lacks, which a
binary-protocol client's introspection trips over). Re-derive the reader list from
every site that reads a DML plan's `returning` items — the callers of
`returning_relations` and `insert_returning_relations` are only the readers that share
the derivation — rather than inheriting it; a *count* is the form of it that goes stale
silently. A catalog scope is not a namespace at all: `TypeScope::Catalog(Some(<target>))`
looks a qualifier up **in the whole database** and falls back to the target only when
nothing matches, so a `RETURNING` reader on it accepts `DELETE FROM ra WHERE id = 999
RETURNING rb.big + 0` (18.6: `42P01`) declaring `rb`'s real `int8` on one leg and
`int4` on the other, and `UPDATE ra o … RETURNING ra.b` resolves where an alias
hides the table name. All three are invisible until a statement produces **no
row**, because until then the evaluator's `42P01` covers for the other two.

The namespace is Postgres's own — **the DML target under its effective name
plus the relations its `FROM` introduces, in FROM order** — and the planner-side
readers above read it as one derivation (`returning_relations`). Known still to
derive it separately: the executor's projection and the evaluator under it
(`project_returning`, off the materialized rows — it has no `Schema`-side list to
read), pinned to the planner's by fixture, and the correlated re-walk of
an `UPDATE`/`DELETE` level, which hands its `RETURNING` items that level's own scope
(`correlation::level_scope`). Measured against
18.6 with target `amb_t(id, v)` and source `amb_s(id, v, k)` aliased `s`:
`RETURNING *` is five columns (`id | v | id | v | k`, the target's then the
source's), `RETURNING s.*` the source's three, `RETURNING s.k` and bare
`RETURNING k` are `7`, `RETURNING v` is `42702`, `RETURNING amb_s.k` is
`42P01` (the alias hides the table name) and `RETURNING s.nosuch` is `42703` —
each answer identical on both engines, and each identical whether or not the
statement matches a row. A `Describe` on a statement matching nothing still
describes all five columns, on both.

**The namespace is the executor's before it is the planner's.**
`collect_updated_from` builds `target ++ from`, evaluates `WHERE` and the `SET` list
against it, and keeps the source half beside the collected targets, so
`execute_update` hands the projection the two relations themselves. **A namespace
addressed by synthesized `relation.column` strings cannot also hold verbatim user
column names.** The executor's flat multi-relation convention (the one `Plan::Project`
over a `Plan::Join` uses) has `SimpleColumns` match a qualified reference against
`"{qualifier}.{name}"` and a bare one exactly-then-by-suffix, and a column a user
quotes into existence with a dot in it is byte-identical to such a synthesized
name, so the exact tier runs first and wins: on that convention, with target
`wt(id, v)` and source `sd s(id, "wt.v")`, `RETURNING "wt.v"` answers the *target's*
`v` where 18.6 answers the source's column, and with target `qt(id, "s.k")` and source
`ws s(id, v, k)`, `RETURNING "s.k"` answers the *source's* `k` where 18.6
answers the target's. No error on either engine, on statements both accept; the
qualified spellings are correct on both either way, which is what identifies
the resolution rather than the column name. The second polarity also declares
the wrong *type* (`int4` on the simple-query leg against `text` on `Describe`
and on 18.6), so the binary leg puts four raw `int4` bytes under a `text` OID.
Resolution keys on the `(relation, column)` **pair**
(`returning_column_map`, the `JoinColumnMap` the same statement's
`WHERE`/`SET` clauses already use), which no string can forge; the synthesized
names stay on the descriptors, and the correlated `OuterFrame` is handed the same
pairs, so a bare dotted reference inside a `RETURNING` subquery answers the right
relation's column. A `Plan::Join` would wear the same *spelling* by a second
mechanism — `register_input_columns` re-parsing each input column's name for its
pair, which manufactures a relation the query never named — and takes its pairs from
the plan (`executor::query::plan_scope`) instead, so the two are one derivation. The
near-miss alternative is worth naming because it passes both statements above:
dropping the exact tier and matching `.{name}` gets `RETURNING c` wrong over a target
carrying `"b.c"`, whose name is a suffix match too, where 18.6 resolves the source's
own `c`. Measured as a mutation, it reddens
`update/returning_from_dotted_column_is_not_a_suffix_match.sql` and
`update/returning_from_dotted_alias_and_dotted_column.sql` and leaves the two
headline fixtures green, which is why those two exist. The planner and the executor
also **agree on what a source is called**: the executor's `get_table_name_from_plan`
carries a `FunctionScan` arm, without which a FROM-clause `unnest(…)` goes in as
`"unknown"` and `WHERE u.u = 'x'` is `42P01` on a statement 18.6 answers — and
`RETURNING u.u` resolves through the same key. **That one arm reaches further than
the clause**, because `execute_join`'s column map derives its side names the same way;
without it, measured on both engines,
`SELECT u.q FROM jn JOIN unnest(ARRAY[1,2]) AS u(q) ON true` over a `jn`
carrying its own `q` answers `9, 9` against 18.6's `1, 2`, and
`SELECT 1 FROM jn, unnest(ARRAY[1,2]) AS t(q) WHERE t.q = 1` answers zero rows
against 18.6's one. The *wildcard* spelling `SELECT v.* FROM unnest(…) AS v` is
`0A000` **where no base table is spelled `v`**; it reads a different walk
(`expand_qualified_wildcard`, keyed on the name and reading a `nonbase` map with
no arm for an `unnest` item), which is why the arm does not reach it — and where
a base table does carry the name, that walk falls through to the catalog and
answers *its* columns instead, the silent polarity of the same gap.

Two corollaries. **A binding check cannot be
a pass of its own**: Postgres resolves a leaf when it reaches it, so over a
non-text `b`, `RETURNING bogus > 0 AND b LIKE 'x'` is `42703` and the reverse
spelling is `42883`, and no ordering of one whole-tree binding pass against one
whole-tree operand pass yields both — it is a per-node arm of the walk that
already decides operand legality. (Give `b` a `text` type and both spellings
are `42703` on both engines; the pair discriminates only while the `LIKE`
operand is illegal.) The same rule puts the `42702` in `check_binding` rather
than beside it: `RETURNING v LIKE 5` is 18.6's `42702`, and pglet's.
**And a claimant count is only as wide as the walk that collects it.** The
relation walk (`plan_relations`) has arms for the kinds `parse_table_factor`
produces — base `Scan`, `VirtualScan`, `CteScan`, `SubqueryScan`, `FunctionScan` — and a
source lowering to a kind it lacks contributes **no** claimant, so a bare name looks
unambiguous: without the `VirtualScan` and `FunctionScan` arms, over targets that carry the
name — `rvt(table_name TEXT, x INT)` and `run_t(id INT, v TEXT)` —
`FROM information_schema.tables c … RETURNING table_name` and
`FROM unnest(ARRAY['x','y']) AS v … RETURNING v` both answer the target's
column where 18.6 says `42702`. A source shape the parser gains later needs an arm
added; the `unnest` arm rests on the *column name* being 18.6's
(`chooseScalarFunctionAlias` names a single-function RTE's column after the
alias). A `Scan` naming a table the catalog lacks still registers **nothing**,
deliberately: the relation does not exist, so an empty registration would
answer `42703` at plan time in front of pglet's own execute-time `42P01` for
the source, which is what 18.6 reports. *And covering the kind is not counting
the claimants.* A claimant is a name the source would **resolve** here, so for
a synthesized catalog view the count is over the columns pglet emits — a strict
subset of 18.6's for the relations it models in part (`information_schema.columns`
17 of 44, `pg_class` 9 of 34, `pg_attribute` 12 of 25, `pg_type` 10 of 32,
`information_schema.tables` 4 of 12, `table_constraints` 9 of 11, `pg_namespace` 3 of
4, `pg_constraint` 22 of 28, `pg_index` 15 of 21). All but the last two bind
non-exhaustively; `pg_constraint`, `pg_index` and the relations modelled whole
bind **exhaustively**, which is a different fact from claiming
(`CatalogTable::columns_complete`). The gap stands: a name 18.6's
view declares and pglet's does not is a claimant there and not here, so a bare
`RETURNING <that name>` over a target carrying it answers the target's column where
18.6 says `42702`, while `RETURNING table_name` — a name pglet's view does emit — is
`42702` on both, the control that identifies the list rather than the gate. The
measurements, and the qualified spelling's, are `EDGES.md` row 7.

Divergences measured on this clause and left in place. An aggregate is
row-count-dependent here where 18.6 is uniformly `42803`. A source aliased to
the target's own name is 18.6's `42712 table name "s" specified more than
once`, and here the statement runs — but only for the spellings the **first**
same-named relation can serve, because the namespace holds two relations
under one name and a lookup takes the first: measured with `UPDATE wt s SET
v=s.v FROM ws s WHERE s.id=1`, `RETURNING s.v` answers where 18.6 refuses,
while `RETURNING s.k` and `RETURNING *` are `42703 column "s.k" does not exist`
— loud, so no wrong answer, and no fixture pins it because a faithful one would
be red until the `42712` gate exists. And a **bare** outer reference to a
source column inside a correlated `RETURNING` subquery is `42703` here where
18.6 answers — the executor addresses a multi-relation outer scope by qualified
name only, which is the answer a plain `SELECT` over a join already gave
(`SELECT (SELECT count(*) FROM y WHERE y.yk = k) FROM t JOIN s ON …`, measured
on both); the qualified spelling `s.k` answers here.

## A relation's column names are one object too, and a rename is applied where each reader reads them

A `WITH c(x, y) AS (…)` column list is `analyzeCTE`'s positional rename of the CTE
relation: the list's names replace the body's, a short list fills from the body
(`ctecolnames[i]` while `i < numaliases`, `te->resname` beyond), a long one is
`42P10 WITH query "c" has N columns available but M columns specified`, and the body's
own names are gone. It never changes a type — `analyzeCTE` reads those from
`exprType(te->expr)`. Duplicates are accepted (`gram.y`'s `opt_name_list` is a bare
`name_list` with no uniqueness rule) and refused `42702` only on reference.

What makes the rename more than a feature is the trap an obvious implementation
falls into: **a CTE's two reference kinds both re-derive the relation's columns at
execution.** A
read-only CTE inlines as a `Plan::SubqueryScan`, whose executor arm returns the columns
the *executed subquery* produced; a data-modifying one resolves to a `Plan::CteScan`,
whose executor arm returns the columns of the `MaterializedCte` that `Plan::With` built
from the DML's own `RETURNING` rows — **not** the `Plan::CteScan.columns` the parser
filled. So a rename applied to the stored body plan reaches the plan-time binding gate
and `Describe`, and does not reach the rows. That is `EDGES.md` row 59's two-producer
class with a third producer under it, and the differential harness compares only two of
the three (`RowDescription` on the simple leg, `Describe` on the extended) — a rename
that missed the executed relation would show as a *green* fixture run and wrong rows.

So the list is stored **beside** the plan and applied by one derivation,
`planner::alias_relation_columns`, at every reader. The reader set is
compiler-enforced: `Plan::SubqueryScan` carries `column_aliases` as a field, and the
node's column-deriving readers all destructure it exhaustively, so the field names
them —

- `Plan::output_columns_scoped`'s arm (the plan-side shape, what `Describe` sends),
- `plan_relations`'s arm (the `UPDATE … FROM` / `RETURNING` scope),
- `collect_nonbase_into` (`planner/query.rs` — the plan-time binding gate's namespace and
  `alias.*` expansion),
- `push_level_relations` (`planner/correlation.rs` — the correlated outer-scope chain),
- the executor's `Plan::SubqueryScan` arm (the executed relation's own columns).

**Two readers the compiler did not name**, and the ones worth re-deriving rather than
inheriting: `Plan::With`'s materializer in `executor/mod.rs`, which reads `CtePlan` by
field access, and the plan-side `Plan::CteScan.columns` application in `parser/select.rs`
(the reference site). They are the executed and the plan sides of the *data-modifying*
route — two independent derivations of the same relation, so the list is applied at
both. Each of these readers is pinned by a fixture or a unit test that reddens when
its application is removed — `plan_relations` and `push_level_relations` by fixtures
written for them, over an `UPDATE … FROM` a renamed CTE and a correlated subquery over
one, since a reader the compiler names is not thereby a reader the corpus reaches.

**A second `WITH`-clause gate, and it cannot share that home.** Two CTEs declared under
one name is `42712 WITH query name "c" specified more than once`
(`transformWithClause`'s collision scan, which runs over the raw list ahead of every
`analyzeCTE`). Under a plain `WITH` the check sits in
`parse_with_clause`'s comma list and **not** in `register_cte`, because `register_cte`
is also what both `WITH RECURSIVE` degrade arms call — and by then that same declaration
has already put its own placeholder in `ctes`, so a `contains_key` test there refuses
every list-bearing `WITH RECURSIVE` as its own duplicate (measured: fixtures and a
unit test redden). Under `WITH RECURSIVE` the list's token pre-scan (`parser::with_recursive`)
reads every name before any body parses, and finds it there.

**Its verdict is recorded, not raised, and so is the `42P10` `register_cte` produces.**
Postgres runs the collision scan after the statement's raw parse and before all
analysis; pglet has no such split — `parse_cte_body` parses and plans in one act — so the
closest expressible ordering is to record both verdicts as the list is walked and fire
them once it ends, duplicate ahead of arity. That reproduces both halves pglet *can*
reach: a `42712` outranks an earlier declaration's `42P10` (`WITH c(a,b) AS (SELECT 1), c
AS (SELECT 2)`), and a body's own **syntax** error outranks both, because it is raised
while the list is still being walked, exactly where Postgres's raw parse raises it.

**`register_cte` is not the clause's only arity check, and the other one is eager.**
`recursive_cte_columns` checks the alias list against the *seed* term's width, and it
runs inside `parse_recursive_cte_body` with a `?` — before the recursive term is parsed,
and before `register_cte` is reached at all. Under a `WITH RECURSIVE` list the pre-scan
decides which items reach it: only one that names itself does (every item, where the
pre-scan cannot read the list), and a `UNION` body with no self-reference is an ordinary CTE whose arity is deferred like any other.

**Under `WITH RECURSIVE` the eager site does not decide the statement's error, and a
plain `WITH` keeps the walk.** When resolving a `WITH RECURSIVE` list fails, the
recorded duplicate, then an arity verdict an item resolved before the failing one
recorded, outrank the failure, and the list's bodies and the statement body are parsed
again in declaration order so that the first **syntax** error that re-parse reaches
outranks all of it — the raw parse's precedence, reproduced by re-parsing rather than by a token scan, which cannot
tell a well-formed body from a malformed one: hoisting the duplicate scan into a token
pre-pass alone was measured to answer `42712` to `WITH c AS (SELECT FROM FROM), c AS
(SELECT 1)`, where 18.6 is `42601`. A plain `WITH` keeps the declaration-order walk, so a
body error pglet raises at *plan* time there still pre-empts a later duplicate. What is
left on both paths is `EDGES.md` row 80.

The arity check lives at registration (`Parser::register_cte`), not at the reference
site, because `analyzeCTE` raises while the `WITH` clause is analysed — an **unreferenced**
CTE with an over-long list still raises. One registration function serves all three routes
that produce an ordinary CTE: the plain `WITH … AS (…)` arm and both `WITH RECURSIVE`
**degrade** arms (a body with no combinator, and a `UNION` body with no self-reference,
each of which Postgres silently treats as an ordinary CTE) — reached under a `WITH
RECURSIVE` list only where its pre-scan read a self-reference the parse then did not
find, or could not read the list at all.

**What the placement cannot reproduce.** 18.6 analyses a CTE body *before* the arity
check, so `WITH c(w,x) AS (SELECT 1 FROM nosuchtbl) SELECT 1` is `42P01` there and this
`42P10` here. pglet does not analyse an unreferenced CTE body at all — the same
gap that makes `WITH c AS (SELECT 1 FROM nosuchtbl) SELECT 1` a silent
success — so the ordering follows that gap rather than causing it, and closing it is that
gap's job.

**The same shape sits one level up, on the FROM-item alias, and is not closed.** Only
`( VALUES … ) AS k(a, b)` and `unnest(…) AS k(a, b)` parse a column-alias list at all; a
base table, a derived table or a CTE reference takes the bare alias, so `FROM t AS k(y)`
is `42601`. `Plan::SubqueryScan`'s field is exactly what a derived table and a CTE
reference would need, but `Plan::Scan` has no counterpart and a base table is the
spelling a reader would try first — closing two of the three would trade one
inconsistency for a worse one. `EDGES.md` carries it, with the precedence artefact that
hides half of it (the plan-time binding gate runs before the end-of-input check, so the
`42601` is masked by `42703` whenever the projection also names a missing column).
