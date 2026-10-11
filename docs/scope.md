# Column-reference scope and binding

How a column reference finds its relation, or fails loudly when it can't. This covers
eval-time resolution against the frame chain, the plan-time binding gate, and the
`RETURNING` namespace and which of its readers share one derivation. The repo `CLAUDE.md`
summarizes the rules and this file states them in full. The divergences that remain are
recorded in `EDGES.md`.

## Eval-time binding — the plan is never mutated to bind

Parameters and correlated columns resolve at eval time against an `EvalEnv`, which is a
`params` slice plus an outer-scope frame chain threaded through `execute_plan_node` /
`eval_expr_env`. That way a prepared statement holds one immutable cached plan, and a
correlated subquery pushes the outer row onto the env instead of deep-cloning the inner
plan for each row.

Resolution is **alias-aware**: `Plan::Scan` carries its FROM alias, so a qualified `c.id`
binds to the scope aliased `c`. It's also **level-aware**. Once a query level's own scope
exists, the plan-time pass re-runs over each sub-plan against a *level-chained* scope, so a
correlated subquery's literal coerces against the enclosing column. A set operation whose
side projects an enclosing column is unified there too, since that's the first place the
column's type is known (`planner::query`'s `OuterTyping`). Without that,
`DELETE FROM t WHERE EXISTS (SELECT i EXCEPT SELECT '7')` compared the `integer` `7` with
the `text` `'7'` and deleted the row 18.6 keeps. The result of that unification is then
**built into the statement again**, instead of being patched into the plan the readers
already saw (§ A pending set-op column's type is never read).

The innermost level wins, and cross-level ambiguity is impossible (Postgres's
`colNameToVar` rule). A relation the scope builder can't enumerate makes its level
uncertain, and resolution stops there instead of risking a binding past an unknown
relation. Uncertainty is a per-**relation** fact, not a per-level flag. That's what lets a
relation the scope can name but can't fully enumerate (a catalog view, where pglet models
a strict subset of 18.6's columns) answer its own qualifier while still stopping the walk.
It's also what lets a FROM-clause function (`unnest(…)`, `generate_series(…)`) be an
ordinary named relation, registered with exactly its declared columns. See
`planner/correlation.rs`.

The eval-time frame chain has its own version of this, and it is **one walk with a
four-way verdict**: `Found` / `Claimed` / `Ambiguous` / `Unclaimed`. A frame **owns** a
qualifier when it *is* that relation (`table == Some(t)`). A frame with no identity owns it
when it addresses it among several by a synthesized `"t.column"` name. That's the split
`SimpleColumns::resolve` uses to gate its own dotted tier, because a user's quoted `"a.b"`
column (or `AS "a.b"` alias) is byte-identical to that addressing. Without the gate, a
frame that knew it was `q1` answered for a relation named `a`, and
`DELETE FROM q1 WHERE EXISTS (SELECT 1 FROM q2 WHERE q2.yv = a.b)` **destroyed a row**.

Three rules follow, and **breaking any one of them causes a wrong write**:

- *The first owning frame stops the walk, whether or not it resolves the name.* It answers
  with its column, or else `42703 "t.name"`, and never rebinds further out. With `a(id)`
  outside and `b(v)` between, `DELETE FROM a WHERE EXISTS (SELECT 1 FROM b WHERE EXISTS
  (… k.k = a.v))` bound `b`'s `v` and removed **every** row, where 18.6 gives `42703`
  hinting `b.v`.
- *A qualifier no frame owns leaves the chain* instead of falling back to a bare match, so
  the inner resolver reports its own `42P01`. Without this rule, a plain typo destroys a
  row (`… u2b.yv = zzz.v`). An alias-hidden table name wrote `99` where the `SimpleColumns`
  version of the same `eref` rule already refused. And
  `ON CONFLICT … DO UPDATE SET v = zzz.v` / `SET v = "EXCLUDED".v` /
  `… WHERE zzz.v = <the excluded value>` each **stored the excluded value**, since the action
  pushes `excluded` innermost.
- *A bare name counts **exact** claimants per frame.* One resolves, two or more are
  `42702`, and none walks outward. That keeps a `USING`-merged `id` resolving (it's one
  exact claimant ahead of the raw `a.id`/`b.id`), while a derived table projecting
  `a AS x, b AS x` refuses instead of picking by select-list order.

The ambiguity is final at the level that claims it, including the **inner** scope. So
`resolve_column` returns the inner resolver's `AmbiguousColumn` and doesn't escalate.
Escalating deleted the outer row of `DELETE FROM o WHERE EXISTS (SELECT 1 FROM c JOIN d ON
… WHERE v = 1)`. It could also surface as a *different* loud error
(`42883 text = integer`) when the outer column's type differed, which looks like an
operand bug but is this one.

`column_declared_type` uses the same four-way match and returns `None` on the two raising
arms. The pairing is structural, not tested. A mutation that makes them disagree breaks no
test, because both are reached from walks that evaluate the value first, and its error
wins.

**A gate on the identity only works if the call sites pass the identity in, and a rule
written about the gate doesn't say that.** Both readers change behaviour on the same
`None`: the dotted tier above, and `SimpleColumns`'s loose last tier, which drops the
qualifier and matches the bare name. So a scope that *has* a relation but is handed `None`
doesn't just lose alias-awareness. It answers for relations the statement never named,
through whichever of the two tiers the spelling reaches. Two call sites hit this:

- The first is a **write**. With `execute_update` passing the statement's FROM *alias*, an
  unaliased `UPDATE u1 SET id = 99 WHERE EXISTS (SELECT 1 FROM u2 WHERE u2.yv = a.b)`
  updates a row over `u1(id, "a.b")`. The *aliased* spelling of the same statement is
  `42P01` on both engines, and that's the control showing it's about the identity and not
  the statement kind. Likewise `UPDATE ex1 SET v = 42 WHERE bogusrel.n = 1` writes `42`.
- Without a `Plan::FunctionScan` arm in `single_table_name`, `SELECT (SELECT count(*) FROM
  u2 WHERE u2.yv = a.b) FROM unnest(ARRAY['hit','miss']) AS v("a.b")` and
  `SELECT zzz.b FROM unnest(ARRAY['x','y']) AS v(b)` both return rows.

18.6 gives `42P01` for all four. The name to pass is Postgres's `eref`: the alias when one
is written, and the relation's own name when none is. That keeps the aliased spellings
refusing the hidden table name. Several walks derive a plan's relation name, and they
don't all cover the same FROM kinds. `single_table_name`'s doc says which ones have a
`FunctionScan` arm, and for which functions.

**A reader over a node's output names learns its level from the plan, so a qualifier the
level owns never reaches the chain.** `resolve_column` gives a qualified reference to the
inner resolver first only where `scope_has_table` holds. The resolvers that the `Sort`,
`Aggregate`, `Window` and `DistinctOn` nodes build over their input's names have no
relation identity, because those names are a projection's output, or a FROM pipeline's
evaluated by a node above it. So each one is told the effective names of its query
level's FROM items (`SimpleColumns::within_level`). Those are read from the plan by
`executor::query::scan`'s `level_relation_names`, which goes down through the nodes a
level stacks over its FROM clause, into both sides of a join, and stops at each FROM item.
That way a derived table's or a set operation's own names are never read as this level's.

Without this, when a sub-select's FROM item and an enclosing one both carried a qualifier,
the reference read the enclosing row. That happened in an aggregate's argument, a
`GROUP BY` key, `HAVING`, a grouped or windowed select list, and an `ORDER BY` or window
key. Over `t (y, n)` holding `10`, `20` and `ev (n)` holding `5`, `7`, `SELECT y,
(SELECT max(o.n) FROM ev o) FROM t o` returned `10`, `20` where 18.6 returns `7`, `7`. The
same binding changed which rows a `DELETE` removed and what an `INSERT … SELECT` stored
(`EDGES.md` row 323).

A qualifier the level owns binds a column of that relation. How it binds depends on the
level's structure, never on a column name's text. Where the node reads a join's rows, it
uses the input's `(relation, column)` pairs (`LevelScope::pairs`, from
`scan::plan_scope`). Otherwise it matches the bare name exactly. A suffix match would be
another relation's column: over `pg_class o JOIN w` it returned `w.k` for `max(o.k)`
where 18.6 refuses. And dots in a name don't make it a relation: a user's quoted `"o.z"`
column is an ordinary column of `o`.

The declared type and the enum order take the same route as the value, and all three
branch on `scope_has_table`. The aggregate and window nodes describe their output
(declared types, an aggregate's operand check, `MIN` / `MAX`'s enum order) through the
same resolver they evaluate with (`ProjectionScope::with_resolver`), so a dotted user
column such as `"o.n"` can't lend `o.n` its type. A sort above a projection reads output
names, which carry no relation, so an owned qualifier there binds whatever output column
has that name. For an ordinary table that's refused at plan time. For a catalog view it's
open (`EDGES.md` row 325).

`eval_expr_env` takes no column names, so these nodes build their resolver themselves,
instead of one per row with no level. A `DISTINCT ON` key reaches its dedup already tied
back to an output name, so no statement we tested reaches that node's resolver with a
qualifier. It's told its level anyway, so all four nodes build their resolver the same
way. This is covered by the `select/inner_alias_shadows_outer_*` fixtures, in both
polarities, and by `executor::expr`'s `level_names_tests`.

**The whole chain runs at eval time, and that limits every rule above.** A statement that
matches no row never reaches it, and a reference in `RETURNING` is projected after the
writes. So the executor's rules are only half the story. The plan-time binding gate below
is the other half, and both are required. The rules here are covered by
`crates/pglet/tests/scope_relation_identity.rs` for row survival, since `@expect-error`
compares SQLSTATEs and passes for a refusal raised *after* the writes. The
`correlated_outer_ref_*` fixtures cover the SQLSTATEs. Each test fails when its own rule
is reverted.

A frame carries its scope's `(relation, column)` pairs, derived from the plan, so no
spelling can forge one. A frame that owned a qualifier by a `"t."` prefix
instead would be forged by a quoted `"a.c"` column, and would answer a bare `c` over
`f3i("a.c", "b.c")` with `42702`, where both engines return the row, since `"a.c"` and `"b.c"` are just two
ordinary column names.

## A grouping query's aggregates are values the scalar walk reads by node

A `Plan::Aggregate` evaluates its `HAVING` predicate and its select list through the
**ordinary scalar walk** (`eval_expr_full`), over the group's first row. The select list
includes the junk columns the planner adds for `ORDER BY` keys. When the walk reaches an
`Expr::Aggregate`, it returns the value the grouping computed for **that node** over the
group. It reads that value through the `SubqueryExec::aggregate` lookup the grouping lends
the walk (`executor::query::aggregate`). This is the eval-time binding rule above with one
more kind of leaf: the plan is never rewritten to carry a value.

The lookup is keyed by the node's identity, so `sum(a)` written twice is two entries. A
node the grouping computed nothing for is refused with `0A000`, instead of being answered
from another node's value. A sub-plan's evaluation builds its own `SubqueryExec`, so an
inner query's aggregate is never read from an outer query's group. A position where no
grouping lends the lookup, such as a `RETURNING` aggregate (`EDGES.md` row 4), keeps the
refusal.

Because it's the scalar walk, every node it evaluates can have an aggregate below it
without needing a list of its own: `round(avg(x), 2)`, `CASE WHEN count(*) > 0 THEN …
END`, `greatest(max(a), 1)`, `jsonb_build_object('n', count(*))`, and the operand gates
(`max(conrelid) + 1` is `42883`, just as `conrelid + 1` is). An arm that evaluated a child
through a context without the lookup would refuse an aggregate below it again.
`crates/pglet/tests/aggregate_walker_arms.rs` runs an aggregate under each `Expr` variant
the walk evaluates an operand of, in the projection, an `ORDER BY` key and `HAVING`.

**A group can compute aggregates the walk never reaches.** 18.6 computes every aggregate
that survives its planner's constant folding (`eval_const_expressions`) for every group,
whether or not the expression above it reads the value. `CASE WHEN count(*) > 0 THEN
max(a) ELSE sum(1/b) END` raises the division's `22012` over a `b = 0` row, while
`CASE WHEN false THEN sum(1/b) END` returns `NULL`, because its aggregate was dropped with
the branch. So the grouping first collects the aggregates under each expression
(`Collector`), computes them, and only then runs the walk.

**A branch is dropped only under a decider that is certain by its shape, and every other
aggregate is computed.** Whether 18.6's planner folds a decider depends on every function,
operator, cast and type under it: its volatility, its collation, the session. Getting that
wrong in each direction has different costs. An aggregate kept where 18.6 dropped it can
only raise where 18.6 answers. One dropped where 18.6 kept it can **answer where 18.6
raises**. So the collection only treats as a decider what is a constant by construction,
whatever is around it:

- the literals `true`, `false` and `NULL`;
- a comparison (`=`, `<>`, `<`, `<=`, `>`, `>=`) of two integer or `numeric` literals;
- `NOT`, `AND`, `OR`, `IS [NOT] NULL` and `IS [NOT] TRUE | FALSE | UNKNOWN` over those.

What a certain decider drops is a branch the scalar walk doesn't evaluate either:

- the right operand of an `AND` / `OR` that its **left** operand decides;
- a `CASE` arm whose condition is certainly not true, and every arm and the `ELSE` after
  one whose condition certainly is. A simple `CASE` works the same way where its operand
  and the arm's value are both integer or `numeric` literals;
- the arguments of a `COALESCE` after an integer, `numeric` or boolean literal, either bare
  or under the cast the coercion pass uses to bring it to the `COALESCE`'s type.

Nothing else decides anything here: not a string comparison, a bound `$N`, arithmetic over
constants, a function call, a cast, an `IN` list, or a strict operator over a `NULL`.
Where 18.6 folds one of those, a failing aggregate in the branch it dropped raises here.
That's loud, and it's the class `EDGES.md` row 89's residuals measure.

A kept branch is kept whole. A `CASE` that a certain condition reduces to one arm doesn't
make that arm lazy: `CASE WHEN true THEN (CASE WHEN a > 0 THEN 1 ELSE sum(1/b) END) END`
still computes the inner `ELSE`'s `sum`, as 18.6 does. Evaluating an arm only as far as
the walk reads it would answer where 18.6 raises.

**Dropping an aggregate drops its computation, but not its operand's type check where the
operand is a column or a literal.** 18.6 resolves an aggregate against its argument's type
in parse analysis, before its planner folds anything, so `true OR sum(text_col) > 0` is
`42883` there no matter the rows. So the collection walks a dropped branch too, collecting
nothing. It refuses each aggregate it meets that has no form for its operand's type,
kept or dropped, and before any group is formed. It does this only where it can read the
type without evaluating anything: a column's **declared** type, and a literal's.

A dropped aggregate over a cast (which is what a typed constant and a declared `$N` are)
gets its verdict from the cast's target, and nothing evaluates its argument. A kept one
evaluates its argument per row, and its verdict waits for `compute_aggregate`. If it's
alone in the statement, a refusal of the cast itself is raised first, as 18.6 raises it
(`max(int_col::jsonb)` is `42846` on both). If it's beside another aggregate the
collection refuses, that refusal comes first here (`EDGES.md` row 304).

What the check doesn't type is judged where the aggregate is computed. So under a dropped
branch, or over no group, it isn't judged at all. That covers an argument whose type or
refusal only evaluation shows (`sum(upper(int_col))`), a bare `$N`, a `DEFAULT`, and a
kept aggregate over a cast. The check also trusts the declared type. Where a column's
declaration is a guess (a derived column that is a correlated scalar subquery, or an
outer-level aggregate's argument), the refusal is a guess too. It has no form for a
`time` operand of `sum` / `avg`, which 18.6 resolves through an implicit cast (`EDGES.md`
rows 89, 60 and 307).

**The collection evaluates nothing but those deciders.** A constant in the select list or
`HAVING` is evaluated by the scalar walk where the walk reaches it, as in a query that
groups nothing. `CASE WHEN length('abc') = 3 THEN 1 ELSE 1/0 END … GROUP BY a` returns
`1`, and a constant in a branch no group takes, or in a query that forms no group, isn't
evaluated. When such a constant fails, that matches 18.6 if its planner folded the branch
away or never planned the expression. It's an answer where 18.6 raises if its planner
folded the constant (`EDGES.md` row 60 covers that class, grouped or not). A failing
constant the walk does reach is raised after the group's aggregates are computed, so a
failing aggregate takes precedence over it (row 304).

**The order within a group is pglet's own.** A group's `HAVING` aggregates are computed and
the predicate evaluated before its select-list aggregates are computed, so a group that
`HAVING` discards computes none of the select list's. That's not 18.6's order, which
advances every aggregate over each input row before evaluating anything above one
(`EDGES.md` row 304).

## A pending set-op column's type is never read

A set operation is built as its sides are parsed. A side's entry that names a column of an
**enclosing** query level has no type at that point, since inside an expression subquery
the enclosing level's `FROM` may not even have been read yet. Such a column is left
**pending** with a provisional type (`planner::query`'s `OuterTyping::Deferring`), and the
correlated re-walk unifies it once the level-chained scope exists.

Between those two moments the node is an ordinary plan, and anything that asks it for its
columns gets the provisional type. That includes an enclosing set operation's own
unification, a derived table's column, a scalar subquery's common type in a `COALESCE`,
an `INSERT … SELECT` source's assignment types, and both declared-type walkers. We can't
list those readers, because it's every caller of `Plan::output_columns` on a plan whose
rows flow from the node. So **no reader is made to wait**. Instead, the re-walk's result
is built into the statement again. Each unified column's enclosing-level reference types,
enum identity included, are recorded on the node (`SetOpOuterTypes`, a handle the parser
also holds). Then `Parser::parse_statement` **parses the statement a second time** from the
same token checkpoint, building each such node complete with those types
(`SetOpTyping::Resolved`). The plan the second parse *returns* holds no pending column, so
no reader of it can see a provisional type. A node that's still pending there is refused
instead (below). The first parse's plan is dropped.

The consequences:

- **A column the re-walk never unified has no recorded type**, so the second parse leaves
  it pending again, and that plan is *refused* (`0A000`) instead of returned. The route we
  found is a `VALUES` cell, which no re-walk visits:
  `INSERT INTO t VALUES ((SELECT zz EXCEPT SELECT 1))` gets that refusal here, where 18.6
  gives `42703` for the name.
- **A node the re-walk never *reached* takes the hints of one built equal to it, and the
  record isn't part of a plan's identity.** The re-walk visits the plan, so an expression
  a later rewrite discards is never unified. `SELECT DISTINCT (SELECT i EXCEPT SELECT
  '7') FROM t ORDER BY (SELECT i EXCEPT SELECT '7')` matches the sort key to the target
  entry it equals and keeps only the entry. Two things make that statement work. First,
  the records are excluded from `Plan` equality, so the two occurrences are one expression
  to the match that compares them. Second, the key's own record, left with nothing, takes
  the entry's hints. The two nodes were built equal, which is *why* the match took one for
  the other, and building them differently in the second parse would break it. Equality is
  of the nodes **as built**, before the re-walk wrote its unified types into them.

  **An unhinted record doesn't lead to the refusal above.** We tested with the borrowing
  removed, and the statement above is
  `42P10 for SELECT DISTINCT, ORDER BY expressions must appear in select list`. The second
  parse builds the key unlike the entry, the match misses, the key survives into the plan,
  and `SELECT DISTINCT`'s own gate raises. `parse_statement` returns that error from
  `parse_statement_once` **before** it reaches the `unresolved` check at all. So the
  borrowing affects that `42P10`, not the pending-column `0A000`, and the refusal tells us
  nothing about the borrowing's scope or soundness.

  **Which record gets borrowed from depends on order**, because `Plan` equality doesn't see
  the enclosing level a correlated name resolves at. Two nodes spelled alike whose bare
  column binds at different levels compare equal here, and `hinted.iter().find` takes the
  **first** of them in build order. Over `pp(i INT)` and `qq(i TEXT)`,
  `SELECT DISTINCT (SELECT i EXCEPT SELECT '7') AS z, (SELECT (SELECT i EXCEPT SELECT '7')
  FROM qq) AS y FROM pp ORDER BY (SELECT i EXCEPT SELECT '7')` answers as 18.6 does. With
  the same two entries **swapped**, it's `42P10` here while 18.6 still answers. The key's
  orphan borrows the nested entry's hints instead of its own, and the second parse builds
  it unlike the entry it has to match. So the equality doesn't check whether two nodes are
  the same expression, only whether they were built alike, which is weaker. Every wrong
  borrow we found costs a refusal instead of a wrong answer, but nothing here guarantees
  that. The gap is open, and the pair we tested is `EDGES.md` row 153. That row also says
  what a repair has to preserve: 18.6 answers **both** orders, so refusing the working one
  too would make the pair consistent and both halves wrong.
- **The first parse's errors are the statement's errors.** A reader that raises on the
  provisional type raises before the re-walk can correct it, and there's no second parse.
  Over a `numeric` `n`,
  `EXISTS (SELECT 1 FROM (SELECT n EXCEPT SELECT 1) s WHERE n = '1.25')` is `22P02` here
  where 18.6 returns the row. That's the loud residual of this design, recorded in
  [compatibility.md](compatibility.md)'s set-operation row.
- **A clone reports to the same record.** A read-only CTE is inlined at each reference, so
  two plans can share one node's record. A second unification that disagrees with the
  first is recorded as a conflict, and a conflicted record hints nothing. So the statement
  gets the refusal above instead of building on either answer.

## A column reference binds at plan time, and the gate closes only over a namespace it can prove whole

The eval-time rules above can't be the whole story, because an error they raise per row
isn't raised at all when there's no row. `SELECT 1 FROM <empty> WHERE zzz.id = 1`,
`UPDATE t SET v = zzz.v WHERE 1 = 0` and `DELETE FROM t WHERE EXISTS (SELECT 1 FROM
<empty> WHERE u.k = zzz.v)` all report success under the eval-time rules alone, where 18.6
refuses them in parse analysis. And a whole family of readers has no identity to gate
with at all: the post-projection resolvers behind `ORDER BY` / `GROUP BY` / `HAVING` /
window keys and a join's projection. There a bogus qualifier would be answered by the
loose bare/suffix tier, and would **delete a row** when a write reaches one through a
subquery. The planner knows the FROM namespace where those readers don't, so
`ColumnScope::check_binding` closes the whole class at once instead of per site
(`EDGES.md`).

The gate judges a reference but doesn't route it. A qualified column that both a
sub-select's own FROM item and an enclosing level carry passes the gate, and which of the
two supplies the value is up to the eval-time resolver (§ Eval-time binding, *a reader
over a node's output names learns its level*).

**It's safe because "whole namespace" is computed, not assumed.** A level's scope is
complete at its own lowering exactly when no *expression* subquery encloses it. The
parser counts that depth around `(SELECT …)` / `EXISTS (…)` / `IN (SELECT …)`, and
**resets** it for a derived table or CTE, whose FROM can't see the enclosing query. A
level inside an expression subquery stays provisional until `resolve_correlated_subplans`
re-walks it against the level-chained scope. Closing it anyway makes an ordinary
two-level correlated reference `42P01`, because at the middle query's own lowering the
outermost relation doesn't exist yet.

The verdict has **three** outcomes: `Bound`, an error, or **`Uncertain`**. That's because
a relation the scope can't name or can't enumerate might carry the reference. It's why
the catalog relations pglet models only in part register non-exhaustively, and return
`(0 rows)` where an exhaustive gate would invent a `42703`. `pg_constraint` and
`pg_index` are the exceptions. Although they're modelled in part, they register
exhaustively like the relations modelled whole, and give `42703` for an unmodelled column
instead of a fabricated one. `CatalogTable::columns_complete` is the single fact that
decides which side a relation is on.

**But which kinds get that treatment is a claim about what the builder *can't* produce,
not about what it hasn't been taught to walk, and three builders derive it separately.**
A FROM-clause function item (`unnest(…)`, `generate_series(…)`) is a function RTE with
exactly its declared columns. All three builders register it that way, through one
`push_partial_relation` → `push_relation`: `push_level_relations` (the correlated
sub-plan's scope), `push_plan_relations` (an `UPDATE … FROM`'s), and `build_column_scope`
(the one the gate closes over). A builder that registered it as a *subset* would answer
differently by route. `UPDATE pt SET v='x' FROM unnest(…) u(x) WHERE bogus = 1` would
match 18.6's `42703` one level down and give `(0 rows)` at the statement's own level. The
same happens through `SELECT`, `GROUP BY`, `ORDER BY`, `HAVING`, a nested sort key (rows,
sorted, instead of a refusal), and an unreferenced CTE's `DELETE`, which removes every
row.

A nearby shape isn't this bit's to fix. Two FROM items sharing an alias are refused
`42712` by 18.6, and pglet doesn't gate that at all. They give a wrong loud code for the
spellings the collapsed column map can't serve, and still return rows for the spellings
the surviving relation's columns serve. The fix there is the `42712` gate, not this bit.

Two more rules come from Postgres itself, not from caution. Only a **bare single-field**
`ORDER BY` / `DISTINCT ON` key may name an output column (`findTargetlistEntrySQL92`). So
such a key is judged against the level's **target list** first, and falls through to the
range table only when no entry has the name. We tested dropping that claimant set (judging
a bare key against the FROM relations alone), and it breaks the output-alias keys under
`select/order_by_*`, `select/window_*` and `select/distinct_on_alias_key`, the
`types/coerce_*` / `types/group_by_*` clause-pairing set, `select/join_dotted_column_*`,
and two `spine/auth` queries. Those keys are resolved **after** the projection, which is
where that list finally exists. That's `transformSelectStmt`'s own position for the sort
clause, and it's what makes the target-list match possible at all. The second rule: a
`JOIN … USING (c)` merges `c` into one column, and that fact shifts outward with its
level, so a correlated subquery sees it too.

## A sort key is bound where its claimant set exists, and that is a different site in each clause position

An `ORDER BY` / `DISTINCT ON` key is the only reference whose namespace isn't just the
FROM clause. Postgres's `findTargetlistEntrySQL92` lets a **bare single-field** key name a
target-list entry that no relation carries. So the gate for such a key can't run where the
rest of the level's references are judged, because at a level's own lowering the target
list doesn't exist yet. A gate that can't see the claimant set can only decline to judge
every bare name.

A blanket decline on `ColumnScope` would leave a key binding to nothing past plan time at
every level the correlated re-walk (`planner/correlation.rs`'s `key_scope`) never reaches.
**And a statement matching no row never reaches the eval-time resolver either**, so
nothing would judge it at all. So there's no such decline. Each position a key can be
written in is gated where its own claimant set is whole:

- a level's own lowering,
- a set operation's trailing clause,
- a no-FROM select's tail,
- and, for a level inside an expression subquery, the correlated re-walk that already
  has the set.

These are separate sites, and fixing one leaves the others unchanged.

**At a level's own lowering, after the projection.** The two calls come right after the
`WHERE` and `HAVING`, `ORDER BY` first, which matches `transformSelectStmt`'s order (FROM,
target list, `WHERE`, `HAVING`, sort, group, distinct). They read
`correlation::target_list_entries`, the sibling of the re-walk's derivation over the same
`entry_identity`, so the two phases can't drift on what makes two entries one claimant.

Each key's binding **interleaves with its positional range check** in source order. That's
a second decision, and a visible one. `ORDER BY bogus, 9` is `42703`, where checking all
positions before binding would give `42P10`. `ORDER BY 9, bogusrel.x` is `42P10`, and
`DISTINCT ON (bogusdon, 9)` is `42703`. `ORDER BY 9, bogus` and
`DISTINCT ON (9, bogusdon)` stay `42P10`, as the other half of each pair.

Binding the keys after the clauses `transformSelectStmt` transforms first is what puts
these error-precedence pairs in 18.6's order. Over `ep(a INT, s TEXT)`:

- `SELECT bogusproj FROM ep ORDER BY bogusrel.a` is `42703`,
- `SELECT a FROM ep WHERE bogusw = 1 ORDER BY bogusrel.a` is `42703`,
- `SELECT s = 5 FROM ep ORDER BY bogusrel.a` is `42883`.

If the key were bound first, each would give the key's own `42P01`. `GROUP BY` is the
control showing the position is right and not just different:
`SELECT count(*) FROM ep GROUP BY bogusg ORDER BY bogusrel.a` is `42P01` on both engines,
in either binding order. Two more cases change only the *message*, so no fixture covers
them: `GROUP BY bogusg ORDER BY bogus` names the sort key as 18.6 does instead of the
grouping key, and `DISTINCT ON (bogusdon) … ORDER BY bogusord` names the sort key.

The cost is a widening of an existing class, not a new one. `check_binding` has recorded
causes for refusing two entries that 18.6 calls `equal()`. These are reached at a
level's own lowering as well as from a *nested* level, and regardless of row count. Over
an empty `ja2(c, x)` and a two-row one, each is `42702` at either row count, where 18.6
returns rows throughout. No accept shape we tested changes with the binding site. A sweep
over output aliases, dropped base columns, `USING` merges, positional keys,
aggregate/window/`DISTINCT`/`DISTINCT ON` shapes, derived tables, CTEs, correlated
subqueries, `unnest`, a set op and a catalog view gives byte-identical answers either way.
`EDGES.md` has the causes, and retiring them is a separate item.

**On a set operation's trailing clause, at that clause's own lowering.**
`planner/query.rs`'s `plan_set_op_order_limit` closes hard, because that namespace is
complete by construction. 18.6 restricts a set-op's trailing keys to the set-op's own
output names, and refuses a correlated reference and any computed expression alike with
`0A000 invalid UNION/INTERSECT/EXCEPT ORDER BY clause`, so there's no enclosing level to
wait for. The correlated re-walk's `Plan::SetOp { .. } => scope.push_opaque_relation()`
stays. It protects names in the set-op's *arms*, and removing it was only ever tried as a
probe.

A whole key that **is** an `Expr::Column` is judged against an empty closed scope that has
the output names as claimants, with no expression identity on any of them. (18.6 calls
`a AS x, a AS x … ORDER BY x` `42702` even though the entries are identical.) It's
interleaved with the positional range check in source order: `ORDER BY bogus, 99` is
`42703` on both, and `ORDER BY 99, bogus` is `42P10`. A key that *isn't* a bare column
routes its leaves the way `scope_for_key` routes a whole key, which is what makes `-a`
bind and `bogus+1` refuse. `transformSetOperationStmt` adds the output names to the
namespace as a join RTE precisely "so that `ORDER BY upper(foo)` will draw the right error
message rather than `foo not found`".

The gate is keyed on the **tree** being a set op, not on the parser arm. The paren-leading
arm also handles a parenthesized plain select whose sort clause 18.6 folds into the inner
query: `WITH c AS ((SELECT a FROM pt) ORDER BY b) DELETE FROM n8` works on both engines,
and a per-arm gate would refuse it. A `Plan::Sort` over the tree is peeled off before that
test, because a parenthesized operand may carry its own `ORDER BY`. The peel can't reach
the plain-select case, since nothing under a `Plan::Sort` over a plain select is a
`Plan::SetOp`, and the walk stops on its own.

**On a no-FROM select's trailing tail.** The tail goes through `plan_no_from_order_limit`,
which binds its keys as well as applying them, and the arm makes the `parse_for_locking()`
call its siblings make. Routing alone doesn't judge a key: a `Plan::Sort` over a
`Plan::Values` judges its key only where a row reaches the eval-time resolver. So the keys
are bound at plan time, using the same machinery as the no-FROM `DISTINCT ON` gate, and,
as there, **only when the scope is complete**.
`SELECT (SELECT 1 AS q ORDER BY ct.a) AS r FROM ct` works on both engines, so closing
unconditionally would refuse it.

The same arm handles the **quantifier**. `SELECT DISTINCT` builds a node the way
`DISTINCT ON` builds its `Plan::DistinctOn`. Building none would be fine over the one-row
`Plan::Values` the arm was written for, but would do nothing over `Plan::ProjectUnnest`,
which expands rows. The node is built unconditionally, not only over a row-expanding
projection, because a shape test would be a second rule to keep in step with
`plan_select_no_from`'s. And it's built in the **leaf**, so both routes out of it carry the
quantifier: the single select and `finalize_leaf`'s set-op operand. The one exclusion is a
**sequence projection**, which the executor dispatches on `Plan::is_sequence_projection`
before the tail. There, skipping it is fine for the quantifier (a dedup over a single row
returns that row) and open for the bound (`EDGES.md` row 30).

## A level's scope is closed by a reader, and a FROM item is judged by a re-walk that has to reach it

A query level's scope is complete at its own lowering exactly when no *expression*
subquery encloses it. A level inside one stays **provisional** until
`resolve_correlated_subplans` re-walks it against the level-chained scope. A statement's
own level also judges each sub-plan, on a copy, where its binding gate reaches the
sub-plan, so the verdict keeps that level's walk order
(`correlation::judge_nested_level`). Each re-walk also records, on the level's projecting
node, the identity of every target-list reference an enclosing level binds
(`EnclosingColumns`, shared with that copy). The level's output descriptor reads that
record (`EDGES.md` row 159).

A `DELETE`'s namespace is its target, and an `UPDATE`'s is its target plus its `FROM`, so
both close outright. A **no-FROM** select runs the pass over its projection's sub-plans,
because nothing else would. And the pass always rewrites a nested level, including an
enclosing level with no relations, which is exactly the one whose bogus qualifiers have
nowhere left to hide. Two things follow, and each one causes a wrong write when it fails:
*something* has to make that re-walk call for every leaf, and a level left provisional has
to be reachable by some re-walk that can decide it.

**The call.** `parser::select`'s `finalize_leaf` calls `resolve_correlated_subplans` on a
`SelectLeaf::NoFrom` operand, as its `parse_select_inner` twin does on the identical leaf.
Without it, a level inside such a leaf keeps the open scope its own lowering gave it. (An
expression subquery's scope is provisional by design, since it may still name the
enclosing query's relations.) No second reader ever closes it, so a write through it
destroys every row (`EDGES.md` row 30).

The call belongs in the shared function, not at one caller. The leaf on the **left** of a
set op reaches `finalize_leaf` through `parse_select_inner`'s own call site, not
`parse_setop_operand`'s, and it causes the same wrong writes without the call. It's passed
the same `scope_is_complete()` its twin passes, which keeps the pass from inventing a
refusal where the leaf may legitimately name an enclosing relation
(`SELECT a FROM ct WHERE EXISTS (SELECT 1 AS z UNION ALL SELECT ct.a)` works on both
engines). This rule is independent of the trailing-tail routing above, since a set-op
*operand* has no tail of its own.

**The reachability.** `Parser::in_from_item` resets `expr_subquery_depth` to `0` for a
derived table's or a CTE's body, on the reasoning that a `LATERAL`-less FROM item can't
see the enclosing query's relations. That reasoning is **only half of Postgres's rule**.
Such an item is outside its **own level's** namespace (a sibling FROM relation is an
error, not a binding), but the enclosing *levels* stay visible, just as they are to an
expression subquery. So a FROM item nested inside an `EXISTS`/`IN`/scalar subquery is
given a closed scope that's missing every enclosing relation, and any gate reading it
invents a `42P01` over ordinary correlated SQL. That over-rejection is live, and its
members are in `EDGES.md`.

The fix direction is that such an item has to stay *provisional*, since the enclosing
chain isn't whole yet. Then the gate declines at that item's own lowering, and the re-walk
judges its keys once the chain exists. As a one-line change to `in_from_item`, that's
*reset only when the depth is already `0`*. That change doesn't reopen a row-destroying
silent write through an **unreferenced CTE** (whose body never runs, so no eval-time
resolver ever sees its keys), because a top-level CTE body is still at depth `0` and still
resets.

What rules out the one-line change is that, when we probed it adversarially, it turned
refusals that match 18.6 into silently accepted wrong answers, eight of them writes 18.6
refuses. Two routes cause that: an `INSERT`'s clauses, which nobody re-walks, and a chain
holding a non-exhaustively-registered relation that answers `Uncertain`, where the re-walk
declines and nobody judges the provisional item either. So the rule isn't *reset only at
depth `0`*. It's this: **a FROM item may be left provisional only where some
`resolve_correlated_subplans` re-walk will reach it with a complete chain and a decidable
namespace.** Which of the two routes is closed, and what the other still writes, are in
`EDGES.md`.

We tested both ends of that probe. On the accept side, the spelling the change
makes provisional stays refused with its rows intact:
`WITH c AS (SELECT (SELECT 1 FROM (SELECT 1 AS q ORDER BY zzz) x) AS z) DELETE FROM n8`
is `42703` under it, because the enclosing level's re-walk reaches the item. On the
refuting side, the worst two of the eight writes are a `DELETE` and an `UPDATE` driven
through a `NOT EXISTS` over a derived table beneath an `information_schema` level. Each
destroys or overwrites every row where 18.6 refuses the statement outright.

## Which readers of a level's uncertainty move when the bit does

The readers of the columns-exhaustive bit split on whether flipping a relation kind's
registration can change their answer. The split is a fact about this scope, not about
their rules, so re-derive it from the field's callers instead of taking it from here.

`binding_qualified`, `binding_bare` and `ColumnScope::sole_relation_carrying` are the ones
that move. The last one declines outright on an uncertain level, so no `ORDER BY` key over
such a level can be bound to its relation when `output_name_binding` decides whether two
same-named target entries are one expression or two.

`bare_candidates` and `qualified` read `level_columns_uncertain` only to decide whether to
keep walking **outward**, and `build_column_scope` has nowhere outward to go.
`push_scope_relation` stamps every relation level `0`, and the only production `nested_in`
(`planner::correlation`'s `resolve_level`) chains a scope that `push_level_relations`
built. So `levels()` is `0..=0` in every scope `build_column_scope` produces, and both
readers answer the same either way. Don't assume that because a reader consults the
field, flipping the field changes its answer.

## A synthesized `relation.column` name is not a pair, and three alternatives that pass only some of the rows

A join scope that *encodes* each column's `(relation, column)` pair in the column's own
name, for every consumer to re-parse, can be forged. A user's quoted `"a.b"` column, or an
`AS "a.b"` alias, is byte-identical to such a synthesized name. So the forgery works both
ways: a relation the query never named becomes addressable, while the pair the scan's own
alias spells is never registered at all. The executor's pairs come from the plan
(`executor::query::plan_scope`), which no spelling can forge, and the scope carries them
as pairs, not strings. `register_input_columns` files each input column under its
plan-derived relation, `SimpleColumns` and the correlated `OuterFrame` resolve through the
pairs, and a `RETURNING` frame gets the same namespace its clause already resolved
through.

**The correlated outer frame is the flat-string half, and it has two readers and two
producers.** A frame is `(col_names, table, row)`, and a *dotted* entry in `col_names`
arrives two ways. `returning_input_columns` renames a **multi**-relation `RETURNING` row's
descriptors to `relation.column`. And a **user** can quote a dot into a column name or an
`AS` alias, which lands in *any* frame, including a single-relation one that carries its
relation identity. `EvalEnv::resolve_outer_slot` reads those names on both of its paths:
the **qualified** one (an exact `"t.name"` match) and the **unqualified** one (an exact
bare match). The dotted tier answers a qualified reference only against a frame that
doesn't know its relation, gated on `f.table.is_none()`. That's the same gate
`SimpleColumns::resolve` has on its own dotted tier. Two resolvers that agree on tier
*order* but not on the gate aren't aligned, because the gate is the part whose absence
destroys rows.

**A frame lacking an identity isn't the same fact as a frame being multi-relation.** A
single-relation scope can still be handed `table: None` by its call site. We found two,
on statements both engines accept as SQL and 18.6 answers `42P01`. So it's the call
sites, not the gate, that give a single-relation frame its identity, and they're covered
above.

**What the pair derivation is not.** We keep these because each alternative passes some
of the tested rows and none passes all of them.

- It isn't giving `JoinColumnMap` pair keying. `JoinColumnMap` is pair-keyed already, so
  that closes nothing. And the `ON`-position statements resolve through it, so a change
  confined to `SimpleColumns` can't reach them.
- It isn't "stop splitting". For a left-deep `Join(Join(a, b), c)`, the inner join's
  output columns arrive at the outer level already named `a.account_id`, and
  `get_table_name_from_plan` returns `"joined"` for a `Join`. So the split is what keeps a
  qualified reference two joins deep addressable.
- It isn't splitting at the first dot given a known join scope. A relation aliased
  `"a.b"` then splits in the wrong place, which `select/join_dotted_alias_relation.sql`
  covers.

**Those scopes' identity decides error precedence, and the `WHERE` is where it matches
18.6.** A bogus qualifier in the `WHERE` gives `42P01` ahead of each of these (and with
the identity reverted, each one is answered instead): a malformed `SET` value (`22P02`),
a unique violation (`23505`), a missing relation in `RETURNING` (`42P01` naming *that*
relation), a missing relation in a `SET` right-hand side (`0A000`), and a second bogus
qualifier in the `SET` right-hand side (`UPDATE r1 SET v = bogusrel.v WHERE zzz.id = 1`,
which silently succeeds when reverted).

At an unnest level it takes precedence over a neighbour **written after it in the
`WHERE`**, in two spellings. One is a missing column: `SELECT b FROM unnest(ARRAY['x']) AS
v(b) WHERE zzz.b = 'x' AND v.nosuch = 'y'` is `42P01 "zzz"` on both engines, and `42703
"v.nosuch"` with the identity reverted. The other is an ill-typed one: `… WHERE zzz.b =
'x' AND b + 1 > 0` is likewise `42P01 "zzz"` on both, and `42804` reverted. The
**position in the clause is part of each pair**. Write the neighbour first and all three
builds answer the neighbour instead: `… WHERE v.nosuch = 'y' AND zzz.b = 'x'` is `42703`
on both engines, and `… WHERE b + 1 > 0 AND zzz.b = 'x'` is `42804` here where 18.6 gives
`42883`. The target-list direction doesn't match, and that's in `EDGES.md`.

## A FROM name is resolved by the item's kind, never by asking the catalog first

A FROM item's effective name **replaces** the namespace entry for that name across the
whole query level. `addRangeTableEntry` registers the derived table's, CTE's or function's
eref and nothing else, so a base table that happens to have the same name isn't in scope
at all. `SELECT z FROM (SELECT 1 AS z) AS t` returns `1` on 18.6 even when a table `t`
exists, and `SELECT a FROM (SELECT 1 AS z) AS t` is `42703` even when that table has an
`a`. Naming a CTE or derived table after the table it comes from is ordinary SQL, so this
isn't a corner case.

So anything that turns a FROM name into a relation has to dispatch on the item's
**kind**. The readers that match on the plan node can't mis-resolve this way:
`plan_relations`/`push_plan_relations` and `push_level_relations`, plus
`collect_nonbase_into`, which builds the map the name-keyed readers consult. The readers
keyed on the *name* are `build_column_scope` and `expand_qualified_wildcard`.
`build_column_scope` consults the non-base maps before `schema.get_table`, which would
answer with the wrong relation whenever the names collide. `expand_qualified_wildcard`
asks the non-base map first, so it's right for the kinds that map holds. But it isn't a
plan-node reader, and it mis-resolves exactly on the kind that map leaves out. Its bullet
below has the live defect. (`validate_grouped_columns` also reads the FROM name through
the catalog and stays that way, its verdict tested rather than assumed; see the
residuals.)

Re-derive this list from the callers of `Schema::get_table` instead of taking it from
here. **A reader's own *call sites* need the same re-derivation.**
`expand_qualified_wildcard` has two, and its defect looks like an `alias.*` problem unless
you derive the second one (a bare `*`) from `expand_wildcards` instead of from the bullet
below.

- **`build_column_scope`** is the scope the plan-time binding gate, the coercion pass and
  the static comparison gate all read. Its two non-base maps (`function_scan_columns`,
  `collect_nonbase_columns`) are keyed by effective FROM name and hold exactly the FROM
  items of those kinds. Neither recurses into a `SubqueryScan`'s body, so their keys are
  this level's items and no other's. That makes a hit in one a *proof* of the kind, where
  a catalog hit only proves the names match. The one thing that can forge such a hit is
  two FROM items sharing a name, which 18.6 refuses `42712` and pglet doesn't gate. The
  last paragraph below covers what that costs. The lookup order is function item →
  derived/CTE/catalog-view → base table → unenumerable.
- **`expand_qualified_wildcard`** expands `<name>.*`. Through a second call site in
  `expand_wildcards`'s per-FROM-item loop, it also expands a bare `*` over a FROM clause
  with no join, **whenever that `*` isn't the projection's only item**. A *sole* `*` there
  reaches neither call site, because `expand_wildcards` returns it untouched for the
  `Plan::Project` fast path. That's why the defect below looks like it's only about
  `alias.*`, though it isn't.

  It reads `nonbase` before `schema.get_table`, so the derived-table, CTE and catalog-view
  kinds are *proved*. We tested each with a base table of that name in the catalog:
  `SELECT d.* FROM (SELECT 1 AS z) AS d` and `WITH d AS (SELECT 1 AS z) SELECT d.* FROM d`
  give `z` = `1` on both engines beside a `d(a)` holding `9`, and `SELECT t.* FROM
  information_schema.tables t` beside a base `t(zzcol)` expands the view's columns
  instead of `zzcol`. (That it emits four of them where 18.6 emits twelve is the separate
  modeled-subset divergence.) A FROM-clause `generate_series(…)` is proved the same way:
  `collect_nonbase_into` has its arm, and `SELECT g.*, *, 1 FROM generate_series(1, 2) g`
  beside a base `g(q, r)` expands the series' one column, as 18.6 does.

  `collect_nonbase_into` has no arm for an `unnest` item, though. That's deliberate, since
  widening it would change `SELECT u.* FROM unnest(…) AS u`, a divergence of its own, as a
  side effect of a scope fix. So a FROM-clause `unnest(…)` is the one kind whose name
  reaches the catalog here. The expansion then emits the *base table's* column names for
  the unnest's relation, so the outcome depends on whether those names are a subset of
  the alias list's. A subset is silent (one column returned where 18.6 sends two, or the
  unnest's values under the table's labels and order). A non-subset is `42703`. Both are
  live, and **both repeat exactly at the bare-`*` call site**. Beside a base `uu(q INT)`,
  `SELECT *, 1 FROM unnest(ARRAY[1,2], ARRAY[3,4]) AS uu(q,r)` returns `q | ?column?`
  where 18.6 sends `q | r | ?column?`, while the sole-`*` spelling of the same statement
  is correct on both engines. `EDGES.md`'s *addressable by its alias in a value position
  and not* bullet has the members, the controls, and the one spelling the kind dispatch
  changes. The fix is the `unnest` arm that bullet already names, not a reordering of this
  reader, and one arm fixes both call sites.

**Tested members, 18.6 first.** The accept polarity (`1` there, `42703` here) reached the
bare name, the qualified name, `alias.*`, `ORDER BY z`, `GROUP BY z`, `WHERE z = 1`, and a
correlated inner shadow, through both the derived-table and the CTE spelling.

**A base table with the same name and a column of a different type separates binding from
typing, and only that spelling reaches `coercion_type`.** Where the base table lacks the
column, a mis-resolution fails the binding gate and the `42703` above is the whole story.
Where it has it (`shadow(z TEXT)` shadowed by `(SELECT 5 AS z)`), the reference *binds*,
to the wrong relation. Then `ColumnScope::coercion_type` gives the predicate's `'5'` a
`text` target instead of `int4`. With a catalog-first lookup, `SELECT z FROM (SELECT 5 AS
z) AS shadow WHERE z = '5'` is `42883 operator does not exist: integer = text`, where 18.6
returns the row. (That's the eval-time rulebook judging the derived table's int4 row value
against a literal the plan-time pass typed from the base table.) A fixture group written
only against the binding gate can't reach that case.
`select/derived_table_alias_shadow_comparison_coerces.sql` is the one that does.

The **silent** polarity needs an empty derived table, because the base table's columns
*were* bound and only the executor's row resolver would have refused them.
`SELECT a FROM (SELECT a AS z FROM t WHERE false) AS t` and its `t.a` spelling give
`(0 rows)` catalog-first, where 18.6 gives `42703`. The `unnest` alias members are the same
defect and are fixed by the same dispatch: `SELECT z FROM unnest(ARRAY[1,2]) rv_u(z)` is
`42703` catalog-first where 18.6 returns two rows, and `SELECT rv_u.x FROM
unnest(ARRAY[]::int[]) rv_u(z)` is `(0 rows)` where 18.6 gives `42703`.

**What turns a catalog-first lookup into a wrong refusal instead of a quiet
mis-resolution** is the plan-time `unnest(<non-array>)` gate, which reads its argument's
type from this same scope. With a catalog-first lookup, `SELECT unnest(a) FROM (SELECT arr
AS a FROM t) AS t` is `42883 function unnest(integer) does not exist` (from the shadowed
table's `a INT`) where 18.6 answers. The suppression polarity is pglet's own `0A000` where
18.6 raises that `42883`. A gate is only as good as the type it's given.

**What it doesn't reach**, each tested rather than reasoned about:

- A projection-position `unnest(…)` over a FROM clause is `0A000` here whatever its
  argument, and none of this changes that. That's why the accept-polarity `unnest`
  fixtures use an empty table.
- A shadowing derived table's `z + 1` is `42804`, not 18.6's `42883`. That's the general
  operator-code class, which gives `42804` for `'a'::text + 1` with no shadow anywhere.
- `validate_grouped_columns` also reads the FROM name through the catalog and stays that
  way, because across every spelling we probed, its verdict is the same one it gives
  without a collision. `SELECT z, w FROM (SELECT 1 AS z, 2 AS w) AS <name> GROUP BY z`
  answers here and is `42803` there whether or not `<name>` is a table, and the shadowed
  spellings that *do* raise `42803` here raise it there too. Here's why we expect that,
  though it isn't a proof: pglet's functional-dependency relaxation only ever widens
  acceptance, and 18.6 grants a derived table no dependency to relax by. So the collision
  can move this gate toward 18.6's answer, or toward the leniency already recorded for a
  non-colliding derived table, but not into a refusal 18.6 doesn't share.

**Two FROM items sharing a name get 18.6's `42712`, a gate pglet doesn't have.** The kind
dispatch changes which of its spellings are silent, but doesn't close any. Over
`FROM t, (SELECT 1 AS z) AS t`, the **qualified-column** pair trades against a
catalog-first lookup. `SELECT t.z` returns `1` where catalog-first gives `42703`, and
`SELECT t.a` is `42703` where catalog-first returns `1`. `SELECT z`/`SELECT a` swap
`42703` and `42702` between the two. That's one silent member traded for another across
those four spellings, with `SELECT 1` returning a row under either lookup as the fifth.

**Those five aren't the whole class**, and what's said here applies only to them. They
repeat verdict-for-verdict across all three FROM forms: comma-with-derived-table,
`JOIN`-with-derived-table, and comma-with-`unnest`. Three more spellings are recorded
beside them, each over the derived-table form and each tested under both lookups.
`SELECT count(*)` returns `1`, `SELECT *` is `42703` naming a different column each time,
and **`SELECT t.*` splits by the second item's kind**, which is why a count is the wrong
tool here. Of the three, only `t.*` breaks the across-forms repetition, and it breaks it
on the **verdict**. What the other two show is that a repeating verdict isn't a repeating
*value*. Re-tested across the forms, the unnest form's two rows make `count(*)` return `2`
there against the derived table's `1` (and `SELECT t.z` return `1, 2`, and `SELECT 1`
return two rows), while every one of those verdicts is unchanged.

Over a derived table, `t.*` returns the derived table's `1` where catalog-first gives
`42703`. That's a silent member the kind dispatch **adds**, with nothing traded for it.
Over an `unnest` (`FROM t, unnest(ARRAY[1,2]) AS t(z)`) it goes the other way: `42703`
where catalog-first silently returns the base table's `9`, which is a silent member the
dispatch **removes**. Every one of them is a wrong answer or a wrong code for a statement
18.6 refuses outright. As with the same class under the unnest arm, the fix is the
`42712` gate, which closes every spelling at once.

## A clause's namespace is one object, and every reader of that clause reads it

A `RETURNING` item is read by:

- the plan-time binding gate and the wildcard expansion (`lower_returning`),
- the planner's declared-type walk (`Plan::returning_output_columns`),
- the executor's projection and the evaluator under it,
- the prepared-statement parameter inference (`Inference::walk_returning`),
- and the correlated re-walk (`correlation::walk_level`). For an `INSERT` that reads the
  shared derivation through `insert_returning_relations`, and for an `UPDATE`/`DELETE` it
  hands the items that level's own scope.

Each of these can have its own idea of what the clause can name. A reader on the catalog
scope mistypes a `$N` compared with a source column: `int8` where 18.6 has `int4` over
`dt(id, b BIGINT)` / `ds(id, b INT) s`, and OID **0** for a source column the target
lacks, which trips up a binary-protocol client's introspection. Re-derive the reader list
from every site that reads a DML plan's `returning` items, instead of taking it from here.
The callers of `returning_relations` and `insert_returning_relations` are only the readers
that share the derivation. A *count* of readers is the version of this that goes stale
without anyone noticing.

A catalog scope isn't a namespace at all. `TypeScope::Catalog(Some(<target>))` looks a
qualifier up **in the whole database** and falls back to the target only when nothing
matches. So a `RETURNING` reader on it accepts `DELETE FROM ra WHERE id = 999 RETURNING
rb.big + 0` (18.6: `42P01`), declaring `rb`'s real `int8` on one leg and `int4` on the
other. And `UPDATE ra o … RETURNING ra.b` resolves where an alias hides the table name.
All three problems stay hidden until a statement produces **no row**, because until then
the evaluator's `42P01` covers for the other two.

The namespace is Postgres's own: **the DML target under its effective name plus the
relations its `FROM` introduces, in FROM order**. The planner-side readers above read it
as one derivation (`returning_relations`). The readers known to still derive it
separately: one is the executor's projection and the evaluator under it
(`project_returning`, working from the materialized rows, since it has no `Schema`-side
list to read), which fixtures keep in line with the planner's. The other is the
correlated re-walk of an `UPDATE`/`DELETE` level, which hands its `RETURNING` items that
level's own scope (`correlation::level_scope`).

Tested against 18.6 with target `amb_t(id, v)` and source `amb_s(id, v, k)` aliased `s`:

- `RETURNING *` is five columns (`id | v | id | v | k`, the target's then the source's),
- `RETURNING s.*` is the source's three,
- `RETURNING s.k` and bare `RETURNING k` are `7`,
- `RETURNING v` is `42702`,
- `RETURNING amb_s.k` is `42P01` (the alias hides the table name),
- and `RETURNING s.nosuch` is `42703`.

Each answer is identical on both engines, and identical whether or not the statement
matches a row. A `Describe` on a statement matching nothing still describes all five
columns, on both.

**The namespace belongs to the executor before the planner.** `collect_updated_from`
builds `target ++ from`, evaluates `WHERE` and the `SET` list against it, and keeps the
source half beside the collected targets, so `execute_update` hands the projection the two
relations themselves.

**A namespace addressed by synthesized `relation.column` strings can't also hold verbatim
user column names.** In the executor's flat multi-relation convention (the one
`Plan::Project` over a `Plan::Join` uses), `SimpleColumns` matches a qualified reference
against `"{qualifier}.{name}"`, and a bare one exactly and then by suffix. A column a user
quotes into existence with a dot in it is byte-identical to such a synthesized name, so
the exact tier runs first and wins. Under that convention, with target `wt(id, v)` and
source `sd s(id, "wt.v")`, `RETURNING "wt.v"` returns the *target's* `v` where 18.6
returns the source's column. With target `qt(id, "s.k")` and source `ws s(id, v, k)`,
`RETURNING "s.k"` returns the *source's* `k` where 18.6 returns the target's. Neither
engine raises an error, and both accept the statements. The qualified spellings are
correct on both either way, which shows it's the resolution at fault, not the column
name. The second case also declares the wrong *type* (`int4` on the simple-query leg,
against `text` on `Describe` and on 18.6), so the binary leg puts four raw `int4` bytes
under a `text` OID.

Resolution keys on the `(relation, column)` **pair** (`returning_column_map`, the
`JoinColumnMap` the same statement's `WHERE`/`SET` clauses already use), which no string
can forge. The synthesized names stay on the descriptors. The correlated `OuterFrame` gets
the same pairs, so a bare dotted reference inside a `RETURNING` subquery returns the right
relation's column. A `Plan::Join` would end up with the same *spelling* by a second
mechanism: `register_input_columns` re-parsing each input column's name for its pair,
which invents a relation the query never named. So it takes its pairs from the plan
(`executor::query::plan_scope`) instead, and the two are one derivation.

The near-miss alternative is worth naming, because it passes both statements above.
Dropping the exact tier and matching `.{name}` gets `RETURNING c` wrong over a target with
a `"b.c"` column, whose name is also a suffix match, where 18.6 resolves the source's own
`c`. We tried it as a mutation: it breaks
`update/returning_from_dotted_column_is_not_a_suffix_match.sql` and
`update/returning_from_dotted_alias_and_dotted_column.sql` and leaves the two headline
fixtures passing, which is why those two exist.

The planner and the executor also **agree on what a source is called**. The executor's
`get_table_name_from_plan` has a `FunctionScan` arm. Without it, a FROM-clause
`unnest(…)` goes in as `"unknown"`, and `WHERE u.u = 'x'` is `42P01` on a statement 18.6
answers. `RETURNING u.u` resolves through the same key. **That one arm reaches beyond
the clause**, because `execute_join`'s column map derives its side names the same way.
Without it, tested on both engines,
`SELECT u.q FROM jn JOIN unnest(ARRAY[1,2]) AS u(q) ON true` over a `jn` with its own
`q` returns `9, 9` where 18.6 returns `1, 2`, and
`SELECT 1 FROM jn, unnest(ARRAY[1,2]) AS t(q) WHERE t.q = 1` returns zero rows where 18.6
returns one.

The *wildcard* spelling `SELECT v.* FROM unnest(…) AS v` is `0A000` **where no base table
is named `v`**. It reads a different walk (`expand_qualified_wildcard`, keyed on the name
and reading a `nonbase` map with no arm for an `unnest` item), which is why the arm doesn't
reach it. Where a base table does have the name, that walk falls through to the catalog
and returns *its* columns instead, which is the silent polarity of the same gap.

Two corollaries follow.

**A binding check can't be a pass of its own.** Postgres resolves a leaf when it reaches
it. So over a non-text `b`, `RETURNING bogus > 0 AND b LIKE 'x'` is `42703` and the
reverse spelling is `42883`, and no ordering of one whole-tree binding pass against one
whole-tree operand pass gives both. It has to be a per-node arm of the walk that already
decides operand legality. (Give `b` a `text` type and both spellings are `42703` on both
engines. The pair only discriminates while the `LIKE` operand is illegal.) The same rule
puts the `42702` inside `check_binding` instead of beside it: `RETURNING v LIKE 5` is
`42702` on 18.6, and on pglet.

**And a claimant count is only as wide as the walk that collects it.** The relation walk
(`plan_relations`) has arms for the kinds `parse_table_factor` produces: base `Scan`,
`VirtualScan`, `CteScan`, `SubqueryScan`, `FunctionScan`. A source that lowers to a kind
it lacks contributes **no** claimant, so a bare name looks unambiguous. Without the
`VirtualScan` and `FunctionScan` arms, over targets that have the name
(`rvt(table_name TEXT, x INT)` and `run_t(id INT, v TEXT)`),
`FROM information_schema.tables c … RETURNING table_name` and
`FROM unnest(ARRAY['x','y']) AS v … RETURNING v` both return the target's column where
18.6 says `42702`. A source shape the parser gains later needs an arm added. The `unnest`
arm depends on the *column name* matching 18.6's (`chooseScalarFunctionAlias` names a
single-function RTE's column after the alias).

A `Scan` naming a table the catalog lacks still registers **nothing**, on purpose. The
relation doesn't exist, so an empty registration would give `42703` at plan time ahead of
pglet's own execute-time `42P01` for the source, which is what 18.6 reports.

*And covering the kind isn't the same as counting the claimants.* A claimant is a name
the source would **resolve** here, so for a synthesized catalog view the count is over
the columns pglet emits. For the relations it models in part, that's a strict subset of
18.6's (`information_schema.columns` 17 of 44, `pg_class` 9 of 34, `pg_attribute` 12 of
25, `pg_type` 10 of 32, `information_schema.tables` 4 of 12, `table_constraints` 9 of 11,
`pg_namespace` 3 of 4, `pg_constraint` 22 of 28, `pg_index` 15 of 21). All but the last
two bind non-exhaustively. `pg_constraint`, `pg_index` and the relations modelled whole
bind **exhaustively**, which is a different fact from claiming
(`CatalogTable::columns_complete`). The gap remains: a name 18.6's view declares and
pglet's doesn't is a claimant there and not here. So a bare `RETURNING <that name>` over a
target that has it returns the target's column where 18.6 says `42702`. Meanwhile
`RETURNING table_name`, a name pglet's view does emit, is `42702` on both, and that's the
control showing it's the column list at fault, not the gate. The measurements, including
the qualified spelling's, are `EDGES.md` row 7.

Divergences we found on this clause and left in place:

- An aggregate depends on row count here, where 18.6 is uniformly `42803`.
- A source aliased to the target's own name gets 18.6's `42712 table name "s" specified
  more than once`, and here the statement runs. But only for the spellings the **first**
  same-named relation can serve, because the namespace holds two relations under one name
  and a lookup takes the first. Tested with `UPDATE wt s SET v=s.v FROM ws s WHERE
  s.id=1`: `RETURNING s.v` answers where 18.6 refuses, while `RETURNING s.k` and
  `RETURNING *` are `42703 column "s.k" does not exist`. That's loud, so there's no wrong
  answer, and no fixture covers it because a faithful one would fail until the `42712`
  gate exists.
- A **bare** outer reference to a source column inside a correlated `RETURNING` subquery
  is `42703` here where 18.6 answers. The executor addresses a multi-relation outer scope
  by qualified name only, which is what a plain `SELECT` over a join already gives
  (`SELECT (SELECT count(*) FROM y WHERE y.yk = k) FROM t JOIN s ON …`, tested on both).
  The qualified spelling `s.k` works here.

## A relation's column names are one object too, and a rename is applied where each reader reads them

A `WITH c(x, y) AS (…)` column list is `analyzeCTE`'s positional rename of the CTE
relation. The list's names replace the body's. A short list is filled in from the body
(`ctecolnames[i]` while `i < numaliases`, `te->resname` beyond), and a long one is
`42P10 WITH query "c" has N columns available but M columns specified`. The body's own
names are gone. It never changes a type, since `analyzeCTE` reads types from
`exprType(te->expr)`. Duplicates are accepted (`gram.y`'s `opt_name_list` is a bare
`name_list` with no uniqueness rule) and refused `42702` only when referenced.

The rename matters because of a trap an obvious implementation falls into: **both kinds
of CTE reference re-derive the relation's columns at execution.** A read-only CTE inlines
as a `Plan::SubqueryScan`, whose executor arm returns the columns the *executed subquery*
produced. A data-modifying one resolves to a `Plan::CteScan`, whose executor arm returns
the columns of the `MaterializedCte` that `Plan::With` built from the DML's own
`RETURNING` rows, **not** the `Plan::CteScan.columns` the parser filled in. So a rename
applied to the stored body plan reaches the plan-time binding gate and `Describe`, but
not the rows. That's `EDGES.md` row 59's two-producer class with a third producer under
it. The differential harness compares only two of the three (`RowDescription` on the
simple leg, `Describe` on the extended), so a rename that missed the executed relation
would show up as a *passing* fixture run with wrong rows.

So the list is stored **beside** the plan and applied by one derivation,
`planner::alias_relation_columns`, at every reader. The compiler enforces the reader set:
`Plan::SubqueryScan` carries `column_aliases` as a field, and the node's column-deriving
readers all destructure it exhaustively, so the field names them:

- `Plan::output_columns_scoped`'s arm (the plan-side shape, what `Describe` sends),
- `plan_relations`'s arm (the `UPDATE … FROM` / `RETURNING` scope),
- `collect_nonbase_into` (`planner/query.rs` — the plan-time binding gate's namespace and
  `alias.*` expansion),
- `push_level_relations` (`planner/correlation.rs` — the correlated outer-scope chain),
- the executor's `Plan::SubqueryScan` arm (the executed relation's own columns).

**Two readers the compiler didn't name**, and these are the ones to re-derive instead of
taking from here: `Plan::With`'s materializer in `executor/mod.rs`, which reads `CtePlan`
by field access, and the plan-side `Plan::CteScan.columns` application in
`parser/select.rs` (the reference site). They're the executed side and the plan side of
the *data-modifying* route, two independent derivations of the same relation, so the list
is applied at both. Each of these readers is covered by a fixture or a unit test that
fails when its application is removed. `plan_relations` and `push_level_relations` are
covered by fixtures written for them, over an `UPDATE … FROM` a renamed CTE and a
correlated subquery over one, since a reader the compiler names isn't necessarily one the
corpus reaches.

**A second `WITH`-clause gate, which can't share that function.** Two CTEs declared under
one name is `42712 WITH query name "c" specified more than once` (`transformWithClause`'s
collision scan, which runs over the raw list before any `analyzeCTE`). Under a plain
`WITH` the check is in `parse_with_clause`'s comma list and **not** in `register_cte`.
That's because `register_cte` is also what both `WITH RECURSIVE` degrade arms call, and by
then that same declaration has already put its own placeholder in `ctes`. A
`contains_key` test there would refuse every list-bearing `WITH RECURSIVE` as its own
duplicate (we tried it, and fixtures and a unit test fail). Under `WITH RECURSIVE`, the
list's token pre-scan (`parser::with_recursive`) reads every name before any body parses,
and finds the duplicate there.

**Its verdict is recorded, not raised, and so is the `42P10` that `register_cte`
produces.** Postgres runs the collision scan after the statement's raw parse and before
all analysis. pglet has no such split, since `parse_cte_body` parses and plans in one
step. So the closest ordering we can express is to record both verdicts as the list is
walked and raise them once it ends, duplicate before arity. That reproduces both halves
pglet *can* reach. A `42712` takes precedence over an earlier declaration's `42P10`
(`WITH c(a,b) AS (SELECT 1), c AS (SELECT 2)`). And a body's own **syntax** error takes
precedence over both, because it's raised while the list is still being walked, exactly
where Postgres's raw parse raises it.

**`register_cte` isn't the clause's only arity check, and the other one is eager.**
`recursive_cte_columns` checks the alias list against the *seed* term's width. It runs
inside `parse_recursive_cte_body` with a `?`, before the recursive term is parsed and
before `register_cte` is reached at all. Under a `WITH RECURSIVE` list, the pre-scan
decides which items reach it. Only an item that names itself does (or every item, where
the pre-scan can't read the list). A `UNION` body with no self-reference is an ordinary
CTE, and its arity is deferred like any other.

**Under `WITH RECURSIVE` the eager site doesn't decide the statement's error, and a plain
`WITH` keeps the walk.** When resolving a `WITH RECURSIVE` list fails, two recorded
verdicts take precedence over the failure: first the recorded duplicate, then an arity
verdict recorded by an item resolved before the failing one. Then the list's bodies and
the statement body are parsed again in declaration order, so that the first **syntax**
error that re-parse reaches takes precedence over all of it. That reproduces the raw
parse's precedence by re-parsing, not by a token scan, because a token scan can't tell a
well-formed body from a malformed one. Hoisting the duplicate scan into a token pre-pass
alone gave `42712` for `WITH c AS (SELECT FROM FROM), c AS (SELECT 1)` when we tried it,
where 18.6 gives `42601`. A plain `WITH` keeps the declaration-order walk, so a body error
pglet raises at *plan* time there still pre-empts a later duplicate. What's left on both
paths is `EDGES.md` row 80.

The arity check lives at registration (`Parser::register_cte`), not at the reference site,
because `analyzeCTE` raises while the `WITH` clause is analysed, so an **unreferenced**
CTE with an over-long list still raises. One registration function serves all three
routes that produce an ordinary CTE: the plain `WITH … AS (…)` arm and both `WITH
RECURSIVE` **degrade** arms (a body with no combinator, and a `UNION` body with no
self-reference, each of which Postgres silently treats as an ordinary CTE). Under a `WITH
RECURSIVE` list, the degrade arms are reached only where the pre-scan read a
self-reference that the parse then didn't find, or couldn't read the list at all.

**What the placement can't reproduce.** 18.6 analyses a CTE body *before* the arity
check, so `WITH c(w,x) AS (SELECT 1 FROM nosuchtbl) SELECT 1` is `42P01` there and this
`42P10` here. pglet doesn't analyse an unreferenced CTE body at all. That's the same gap
that makes `WITH c AS (SELECT 1 FROM nosuchtbl) SELECT 1` a silent success. So the
ordering follows from that gap rather than causing it, and closing it is that gap's job.

**The same shape exists one level up, on the FROM-item alias, and isn't closed.** Only
`( VALUES … ) AS k(a, b)` and `unnest(…) AS k(a, b)` parse a column-alias list at all. A
base table, a derived table or a CTE reference takes only the bare alias, so
`FROM t AS k(y)` is `42601`. `Plan::SubqueryScan`'s field is exactly what a derived table
and a CTE reference would need, but `Plan::Scan` has no counterpart, and a base table is
the spelling someone would try first. Closing two of the three would trade one inconsistency
for a worse one. `EDGES.md` has it, along with the precedence artefact that hides half of
it: the plan-time binding gate runs before the end-of-input check, so the `42601` is
masked by `42703` whenever the projection also names a missing column.
