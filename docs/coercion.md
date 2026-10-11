# Coercion and operand legality — correct-or-loud

This doc covers the operand-type gates, the cast-context split, and the assignment gate:
the places where a wrong-typed value fails loud instead of silently coercing, matching
zero rows, or storing a converted value Postgres refuses. The repo `CLAUDE.md` summarizes
the rules, and this file has the full statements. The remaining divergences are recorded
in `EDGES.md`, and `assignment_cast_verdicts.rs` guards the identity-guard residual.

## Correct-or-loud

An unsupported type (`42704`), a malformed value (`22P02`), or an eval error in a filter,
a join `ON` or a `SET` right-hand side each **fails loud** instead of silently coercing or
matching zero rows. The *SQL-text type sites* (CREATE TABLE, ALTER ADD COLUMN and CAST all
go through the shared `resolve_column_type`) have no silent-coercion path.

**Operand-type gating** covers comparisons (`=`/`<`/…, `IS [NOT] DISTINCT FROM`),
`LIKE`/`ILIKE`/`starts_with`, `||`, the jsonb operators (`->`/`->>`, `@>`/`<@`, …) and
`SUM`/`AVG`/`MIN`/`MAX`. `planner/legality.rs` is the list. A wrong-typed operand raises
`42883`, or `0A000` where Postgres has the operator and pglet doesn't (`bytea || bytea`,
`jsonb || jsonb`, array concat, `bytea LIKE bytea`). An untyped side of `||` next to an
operand whose type has its own `||` (a `bytea`, a `jsonb`, an array) reaches this rule as
`text`, where 18.6 reads it as that operand's type (`EDGES.md` row 293). The exception is
the jsonb family's `text` leniency: a `text` operand is parsed as JSON or as an array
literal, and the operator answers when it parses and fails with `22P02` or `42846` when it
doesn't (`EDGES.md` row 187).

Containment accepts only `jsonb @> jsonb` and two arrays of the **same** element type.
Postgres resolves `anyarray` per operand instead of unifying across them, so
`integer[] @> bigint[]` is `42883` on both. The jsonb key/path, `LIKE`-family,
`starts_with`, `||` and array-containment rules read pglet's `TypeOid`, so they see an
enum operand as `text` (the comparison gate reads the enum's identity, see
[enums.md](enums.md)). So `m || 1` and `ARRAY[m] @> ARRAY[t]` answer where 18.6 gives
`42883` (`EDGES.md` row 190). The legality rules live in **one** place
(`planner/legality.rs`), used by both the eval-time gates and the DDL-time static checker,
so the two can't drift.

**`unnest(<a non-array>)` lives in that same file but is deliberately *not* part of this
family.** It's **signature resolution**, not the operator table: 18.6 raises it in
`ParseFuncOrColumn`, not from `pg_operator`. `unnest_legality` has exactly **one**
consumer, the `Expr::Unnest` arm of `planner::coercion`'s `check_expr_node_semantics`. The
DDL-time static checker doesn't call it, so the "can't drift" guarantee above doesn't
cover it, and we've measured the drift. At all four stored-expression sites a non-array
`unnest` is `42883` on 18.6 and `0A000` on pglet, by two routes: `executor::ddl`'s blanket
`Expr::Unnest` arm for a column `DEFAULT`, and `validate_stored_expr`'s representative-row
probe, which surfaces the *query*-position scalar refusal for a `CHECK`, an index key
expression and an index predicate. `EDGES.md` row 55 has the cases and the two message
texts. The point here is just that one file isn't one rulebook when a rule inside it has
one caller.

The comparison operators (`=`/`<>`/`<`/`<=`/`>`/`>=`, and the `IS [NOT] DISTINCT FROM`
forms that resolve through `=`) use the same rulebook. A pair with no Postgres operator
(`text = integer`, `bytea = text`) raises `42883` instead of a permissive `false` or a
text ordering. The cross-type pairs Postgres does register compare normally: the numeric
family, the date/timestamp family, `text`/`varchar`, `name` against either
(`current_user = 'postgres'::text` and its `::varchar` and `<` spellings are `t` on both
engines), `oid` against `int2`/`int4`/`int8`, and `time` vs `interval` either way.
`name[]` against `text[]` is **not** one of them: it's
`42883 operator does not exist: name[] = text[]` on both.


## Where the plan-time coercion pass reaches

The gates above have a companion: the pass that types a **bare literal** against the
column it's compared to (`resolve_expr_types_gated`). The gate fires on the *resolved*
expression, so anywhere that pass doesn't reach, the gate judges an unresolved literal.
That's where its two remaining over-rejections are (`EDGES.md` row 3).

**Which operand a literal is typed against is also a fixed list, one entry per
construct.** Under a comparison operator, `coerce_comparison_operands` types it against a
column, an explicit cast (a declared `$N` included), a non-string literal, or an aggregate
call whose type is certain as written (below). In an `IN` list, `resolve_in_list_items`
types it against a column only: at the list's common type where the list is converted
(§ An `IN` list is compared at one type), and at the column's type otherwise. A simple
`CASE`'s label is typed against what `concrete_branch_type` types. Under `@>` / `<@`,
`coerce_containment_operands` types it against what `container_operand_type` types.
Arithmetic types it against no operand. Next to any other operand (a scalar subquery or a
function result, say, an arithmetic expression, or an aggregate whose type isn't certain)
and under arithmetic, the literal stays `text`. Where the other operand isn't text, the
gate then refuses what 18.6 answers: `now() > '2024-01-01'`, `a + '1'`, `1 IN ('1')`
(`EDGES.md` row 262).

**The aggregate anchor is decided from the operands as written, at the aggregate's own
level.** `certain_aggregate_comparison_type` reads the pair before the pass resolves
either side. It takes the aggregate's type from the computed-reach reader (§ A computed
branch is typed where its type is certain) along with the aggregate's recorded argument
certainty. `count` is `bigint`, `HAVING max(ts) > '2024-01-01'` reads a `timestamp`, and
`sum(i8) = '300'` a `numeric`. But `avg(x) = '0.3'` over an `unnest(ARRAY[…])` column,
whose type is the walkers' guess, keeps the literal `text`.

It declines where a column the argument names doesn't bind at the aggregate's own level
(the scope's binding verdict; a reference it can't judge declines too), or where the
argument holds a sub-plan. `count` reads no argument type, so this is what keeps
`count(nosuch) > 'x'` at its `42703`, `count(i) > 'x'` over two relations that both have
`i` at its `42702` (not a `USING`-merged `i` next to a third, `EDGES.md` row 45), and
`count(o.z) = '2'` over an enclosing query's `o` loud instead of returning `EDGES.md` row
303's wrong rows. It declines on a scope that chains an enclosing level. That's the
correlated re-run, which hands the pass the first run's resolved operands, whose fold
casts can name a guessed type as if it were certain, and under which an enclosing column
binds. It also declines where the aggregate refuses its argument (`max(boolean)`) or holds
another aggregate, so that refusal stays where it was, and on a `text` / `varchar` result,
which needs no conversion.

The conversion is the column anchor's cast, validated where that one is. A malformed
literal is refused at plan time over no group and next to a `NULL` aggregate, where 18.6
refuses it too (under the cast's own code; a `timestamp`'s is `EDGES.md` row 96's). A
spelling 18.6 reads and the cast doesn't is refused there, as it is next to a column. The
refusal takes every constant cast's statement position (`EDGES.md` row 104's residuals,
rows 154 and 302, row 60's plan-time-outranks-eval-time paragraph and row 45's ambiguous
column).

The same rulebook runs at DDL time over a stored expression, so a CHECK or index predicate
fails at `CREATE` instead of making the table unwritable. That validation also refuses a
leaf that can never bind there: `DEFAULT` (`42601`), a `$N` parameter (`42P02`), and a
column the table lacks (`42703`). All three checks run in one bottom-up, left-to-right
walk, so when an expression has more than one error, the one Postgres reports first is the
one raised. A **fourth** check, the render probe, runs outside that walk at the end of the
statement, because it stands in for no Postgres error at all ([ddl.md](ddl.md) § Gating a
stored expression is a DDL-time concern; `EDGES.md` row 49). Index-probe exactness, and
where the two build profiles diverge, are in [indexes.md](indexes.md) § Index probes are
exact-or-decline and § Build profiles.

**Which clauses the pass reaches is a fixed list too.** It covers the SELECT projection,
the top-level `WHERE`, the DML filters, `ON CONFLICT … WHERE`, and, resolved against the
same single `ColumnScope`, the **`JOIN … ON` conditions**, **`HAVING`**, **`GROUP BY`**,
**`ORDER BY`**, and the **`DISTINCT ON`** keys. It also reaches every position that
*writes* or *returns* a value: the **`INSERT … VALUES` cells**, the **`UPDATE … SET`**
right-hand sides, the **`ON CONFLICT … DO UPDATE SET`** right-hand sides, **`RETURNING`**
(all three DML statements), and the cells of a **FROM-clause `VALUES`**. A FROM-clause
`unnest`'s array is **not** on the list where its statement is lowered. Only a sub-plan's
is reached, on the correlated re-run, so an `ARRAY[…]` constructor there is evaluated as
written (`EDGES.md` row 308).

The write positions matter for a reason the read positions don't. On any statement that
touches rows, an eval-time gate looks the same as a plan-time one, so the shapes that tell
them apart are the ones that touch none: a
`DELETE … WHERE id = 999 RETURNING <ill-typed>`, and an upsert whose row doesn't conflict.
An eval-time gate lets those succeed silently. **This is about the operand *gate*.
Assignment coercion is a separate rulebook that runs next to it** (§ A conversion has a
*context* as well as a legality). Every assignment right-hand side is also checked against
its **target column's** type before the coercing cast is inserted, so
`UPDATE t SET <int col> = <text col>` is `42804` on both engines instead of a silently
coerced write.

What that gate does **not** cover is the *constant* half of the same clause's zero-row
test, because a bare constant is Postgres's `unknown` and is legal in the assignment.
`UPDATE t SET <int col> = 'notanint'` is coerced by a cast node the planner inserts but
doesn't fold, so `WHERE id = 999` reports `UPDATE 0` here and `22P02` on 18.6. Over a
**matching** row both say `22P02`. This is recorded, not fixed, as `EDGES.md` row 5. The
nearby constant positions do fold at plan time and agree at zero rows
(`UPDATE … WHERE id = 'notanint'`, `DELETE … WHERE id = 'notanint'`,
`INSERT … VALUES ('notanint', …)`), and so does the explicit-cast spelling
(`SET id = 'notanint'::int`).

**Why it's a per-clause pass at all.** Postgres reaches every clause structurally, by
resolving an unknown literal inside operator resolution instead of in a per-clause pass.
pglet's parser and planner are fused, so the equivalent is to run the pass at each clause.
The clauses that are compared to each other by structural equality (`GROUP
BY`/`HAVING`/projection, `DISTINCT ON`/`ORDER BY`) must be resolved **together against one
scope**. Coercing one side of such a pair makes two spellings of the same expression
unequal and raises a spurious `42803`/`42P10`.

**A correlated outer reference is reached**, even though the expression walkers treat a
sub-plan's expression as a leaf. Once a level's own scope exists, the pass re-runs over
each sub-plan against a *level-chained* scope. `ScopeRelation` carries a level, a bare
name resolves at the first level that claims it, and a qualified one at the first level
that names its relation. That's Postgres's `colNameToVar` rule: the innermost level wins
outright, and cross-level ambiguity can't happen (ambiguity is only ever raised *within* a
level). So `… WHERE EXISTS (SELECT 1 FROM b WHERE a.uid = '<uuid>')` resolves instead of
raising a spurious `42883`, in every clause position and at any nesting depth, for SELECT,
UPDATE, and DELETE. That chained scope is also where a nested level's **binding** gate
fires. Its scope when it's first lowered is only its FROM, so only this re-run sees the
whole namespace ([scope.md](scope.md)).

**A relation the scope builder can't enumerate makes its level uncertain.** Resolution
stops there instead of walking outward, because binding a name past an unknown relation
could silently attach it to the wrong column. That's a per-**relation** fact
(`ScopeRelation::exhaustive`), not a per-level flag. That's what lets a relation the scope
can *name* but can't fully enumerate (a catalog view) answer its own qualifier while still
stopping the walk. A FROM-clause `unnest(…)` isn't one of these. An unnest item is a named
relation whose columns are exactly the plan's output columns, so `push_level_relations`
registers it the way `plan_relations` does. Both the bare
`… FROM unnest(ARRAY[1.1::float4]) AS t(v) WHERE v = '1.1'` and the qualified
`… FROM unnest(…) AS t(q) WHERE ra.y = '<uuid>'` return a row on both engines
(`select/correlated_outer_ref_unnest_{level,qualified}_resolves.sql`). The control shows
it's the exactness that decides this, not a loosened walk: with the unnest column renamed
`w`, the bare `v` has no inner claimant and still resolves outward to the enclosing
`float8`, on both engines. An **`INSERT`'s `ON CONFLICT`** action is reached too. Its
namespace is complete by construction, so its scope is closed instead of chained.

## An `IN` list is compared at one type

18.6 doesn't compare `x IN (a, b, c)` as three `=`. `transformAExprIn`
(`parser/parse_expr.c`) splits the items by `contain_vars_of_level`. The ones holding no
column of the list's own query level are candidates for one array. When there are **two or
more** of them, it takes `select_common_type` over the left operand and those items (the
left operand first, so an all-`unknown` list takes its type), converts each of them to it,
and compares the left operand with the array through one operator. A single such item, an
item that names a column, and every item of a set with no common type are each an ordinary
`x = item`. `NOT IN` is the same tree under `<>`.

The two readings differ where the common type isn't the type a pair compares at.
`real_col = 0.1` resolves `float8 = float8` and doesn't match a stored `real` `0.1`, but
`real_col IN (0.1, 1.5)` converts both constants to `real` and matches it. On 18.6:
`x IN (0.1)` and `x IN (0.1, d)` (one constant, alone or next to a `float8` column) find
no `0.1` row. `x IN (0.1, 1.5)`, `x IN (0.1, NULL)` and `x IN (0.1, 7, d)` find it.
`x IN (0.1, 1.5::float8)` doesn't, because the common type is `float8`.
`x IN (16777217, 5)` finds a stored `16777216`. Over an `integer` column,
`i IN (16777217, 2.5::real)` doesn't find `16777217`, because the list is `real`. And
`i IN ('1.0', 2.5)` reads the string as `numeric`.

**pglet converts a list only where every operand's type is certain by construction, and
compares every other list item by item.** The conversion is only right if the list's type
matches 18.6's, and a type this engine infers is one 18.6 can read differently: a
parameter an earlier use has typed, arithmetic over a scalar subquery or an enclosing
query's column, a branch construct with a computed branch the branch readers don't type
(`COALESCE(i + 0, b + (SELECT 0))` is `bigint` there and reads as `integer` here), or a
derived table's column carrying any of those. Converted on a wrong reading, a list
silently matches rows that 18.6's doesn't. So the coercion pass reads the list **as the
statement writes it**, before it resolves either side (`certain_in_list_left_type`), and
converts it where:

- the left operand is a plain reference to a column of a **base table at the list's own
  query level** (`ColumnScope::base_column_type`). The scope builders that know a
  relation's kind mark a base table when they register it, so a derived table, a `WITH`
  query, a set operation, an `unnest`, a catalog view and an enclosing level's relation
  don't qualify. Neither does an `UPDATE … FROM` source, which `push_plan_relations`
  registers without its kind, so a base table there is compared item by item, in `WHERE`
  and in `RETURNING`. `excluded` carries the target's declared columns and does qualify.
- every item is a **constant the parser typed by its spelling**: a literal, or a cast
  written directly over one.

`in_list_array_type` then takes the common type over the column's declared type and the
items, with a string literal and a `NULL` counting as `unknown`. A string literal is read
as that type. An item of another type is cast to it where the type is `real` or
`double precision`, the conversions that round. Under any other common type no cast is
written. For the pairs we tested (an integer next to a wider integer or a `numeric`, a
`date` next to a `timestamp`, a `timestamp` next to a `timestamptz`, a `time` next to an
`interval`), the left operand compares the way 18.6 compares the converted item, and
writing no cast keeps a list whose items already have the column's type planned the same
as before.

**Everything else is one `=` per item, typed the way a scalar comparison is**: a string
literal next to a column is read as the column's type, and nothing else is touched. That
matches 18.6's own tree for a single item, an item naming a column of the list's level,
and a set with no common type. For the rest it's the recorded residual (`EDGES.md` row
280): a computed or constant left operand, a parameter anywhere in the list, a computed
item, a subquery, and every **stored expression**, which no coercion pass runs over. A
`$N` item is reported at the left operand's type (`planner::param_types`), which is the
type its comparison there is typed at. Casting each constant to the column's type
(`x IN ($1, 0.1::real)`) gives the list 18.6's answer in the cases we tested.
`tests/fixtures/in_list/` pins the converted lists, the controls, and the statements where
the item-by-item answer matches 18.6's own because its list is `float8`.

## A computed branch is typed where its type is certain

18.6 unifies a `COALESCE`, a `CASE`'s results, an `ARRAY[…]`'s elements and a
`greatest`/`least`'s arguments at the common type of **every** typed branch, computed ones
included. pglet's two branch readers (`concrete_branch_type`, `arg_known_type`) type a
computed branch where its result is in the numeric category and is certain once its
operands' types are (`BranchReach::Computed`). That means:

- `+ - * / %` and `^` over typed operands (`%` only over an exact pair, since 18.6
  registers no float `%`),
- unary `-`/`+`,
- a math function over a typed first argument,
- `length`/`strpos`/`position`, `EXTRACT`/`date_part`,
- a `date` minus a `date` (an `integer`),
- and a nested `COALESCE`/`CASE` whose every branch they type or is of unknown type.

The pass then resolves 18.6's target and casts every branch to it, and the declared-type
walkers read the casts. Checked against 18.6 on both legs: `COALESCE(i, f + 0)` is
`float8` and returns `2.5`, where before the computed branch was cast to its `integer`
sibling's type and returned `2`. `COALESCE(i + 0, f + 0)` is `float8`, where before it was
declared by its first branch. `COALESCE(i + 0, b + 0)` is `int8` on every row.
`COALESCE('7', length('ab'))` is the integer `7`. `COALESCE(i + 0, tx)` is `42804`.
`least(i, f + 0)` is `2.5`.

**A branch of unknown type takes its set's target; an operand of unknown type isn't
typed.** 18.6 holds a bare string literal, an untyped `NULL`, and a `$N` whose type no
cast and no `Parse` declares as `unknown`, and converts each to the type its position
chooses. A set reads each of them that way (`is_unknown_branch`). The unknown branch is
left out of target selection and cast to the target the others choose, which for a `$N` is
also what describes it. `COALESCE($1, f + 0)` is `double precision` and its parameter is a
`double precision` one. Before, the set was left as written, the parameter was described
with no type, and its text was sent under the other branch's declaration.

An unknown **operand** of `+ - * /` is different. 18.6 converts it to its sibling
operand's type, and nothing here does, so the operator returns whatever the bound value
makes it (a `numeric` times a `double precision` bound through the embed is a
`double precision`), and the branch readers don't type it. `max(f) + $1` is a branch no
reach types, cast to its fold's target like any other, and `avg(i + NULL)` is an `avg`
over no certain integer. A `$N` used more than once is unknown at each use, where 18.6
fixes its type at the first (`EDGES.md` row 54).

**A set the readers type only in part keeps the target it had before** (`set_reach`).
Typing one computed branch and not its sibling would choose the target from the typed one
and cast the other to it, which gives a wrong value where the untyped one is wider. Over a
column `g.y` whose type is a guess, `COALESCE(i + 0, f + g.y)` read at the computed reach
casts `3.5` to `4`. So it's read at the **written** reach instead (a column, a cast, a
literal, and the aggregate, array and alias arms over those), where it's left as it was
and returns `3.5`. The same written reach is what an `IN` list's conversion (§ An `IN`
list is compared at one type) and the `unnest` signature gate read, and their rules and
residuals are stated in terms of it.

**One kind of computed branch decides a written-reach set's target: a `double precision`
one.** Where the branches the written reach types are all in the numeric category and a
branch the computed reader types is `double precision`, the set's target is
`double precision` (`set_types`). 18.6 picks `double precision` for a set holding one,
whatever numeric types are next to it. So the target doesn't depend on which type the
written reading holds for a column it can't vouch for, or on whether any branch is typed
there at all, and each value converts to the float the way 18.6 converts one. A cast the
other way would round the float branch. Over `unnest(ARRAY[NULL::int]) u(x)`,
`COALESCE(u.x, t.f + 0)` is `double precision` `1.5`, where before the branch was cast to
the column's `integer` and returned `2`. `COALESCE(u.x * 1, t.f + 0)`, with neither branch
typed at the written reach, is `double precision`, where before the set was left
unresolved and declared `integer`.

The set must be in the numeric category. A branch neither reach types is read only for its
category, at the declared-type walkers' type, and one they read as `text`, a `boolean` or
a `date` leaves the set as written: `greatest(upper(tx), f + 0)` is 18.6's `42804` and
`nullif(upper(tx), f + 0)` its `42883`, with the gates reading the pair uncast. No other
type joins. A `real` branch doesn't decide the target, since 18.6's type for it depends on
the exact branch next to it. An exact computed branch doesn't join either: a guessed
`integer` column holding `3.5`, cast to a wider exact target, returns `4` (`EDGES.md` row
54).

**The computed reach builds only on certain types**, meaning the type this engine gives is
the type 18.6's parse analysis gives. A column operand is typed there only where its type
is certain (`ColumnScope::column_type_certain`): a base table's column, a derived
relation's column where the expression defining it is certain (below), or a `JOIN … USING`
column merged from same-typed columns. A derived column defined by an expression this
reader doesn't type carries the walkers' type for it, which can be a guess, and a target
chosen from a guess casts its sibling at the wrong type. Over a CTE `x` of
`COALESCE(i + 0, f + g.y)` holding `3.5`, with `g.y` a column whose type is a guess,
`COALESCE(x * 1, b * 1)` returned `4`. An `ARRAY[…]` branch is typed only where every
element is typed or is of unknown type. The no-op test before a cast and the
branch-unification gate read the reach the set's target was chosen at, so a set left at
the written reading is cast, and judged, the same as before this reach existed.

**A relation's column is certain where the expression defining it is.** The pass reads a
`SELECT`'s target-list entries and a `VALUES` list's cells (the expressions that define a
derived table's, a `WITH` query's, a set operation's and a `VALUES` list's columns) with
this reader, **as written**, before it resolves the expression (`certain_entry_type`), and
the plan node carries the type it read (`planner::CertainTypes`). The plan describes the
column as certain where that's the type it declares for it (`ColumnDesc::type_certain`). A
node that passes a column on passes that along with it, and the scope a query builds over
the relation reads it. So one reader decides a column at any depth of derivation:

- a derived table's or `WITH` query's column is certain where the pass typed its entry: a
  column of a certain type passed on, arithmetic over one, a cast, an aggregate;
- a `VALUES` column where it typed every cell, with an untyped cell (a string constant, a
  `NULL`) taking the column's type the way 18.6 gives it one;
- a set operation's column where both sides' are, with an untyped side taking the other's
  type, and a recursive `WITH` query's where both terms' are;
- a FROM-clause `unnest`'s where the array is one a cast converts;
- a column a scalar subquery defines, where the subquery's own column is certain. That's
  its sub-plan's record, read off the sub-plan (`certain_scalar_subquery_type`), which
  this level doesn't take again, wherever in the expression the subquery is.

The expression is read as written because its rewritten form can look typed where it
wasn't. A set left at the written reading casts the branch it couldn't type to a sibling's
type, and the pass types a branch subquery with a `Cast` to the type its column references
resolve to (§ A scalar subquery is a branch of its own type), which is a guess over a
sub-plan whose column is one. Any later reader sees either cast as a typed branch, so the
entry is read before both are written, and an aggregate's verdict likewise.

The pass resolves a fold with a reader that holds no catalog, and that reader types a
subquery only by a cast over it. So under a fold the pass writes that cast over **every**
subquery whose column is certain, a branch and an operand under a branch alike, and the
fold the entry reader types is one the pass's own reader types.
`COALESCE(i, f + (SELECT 1))` is `double precision`, and
`COALESCE(-(SELECT avg(x) FROM v), f + 0)` over a certain
`numeric` `avg` is 18.6's `double precision`. Before, the uncast operand left each set at
the written reading. And a certain column's values are of the type declared for it (the
casts the pass writes, the cast to a `VALUES` column's or a set operation's type, the cast
of a recursive query's rows). That's what lets an exact accumulator, and a target chosen
from the column, trust it.

**So the closure only covers certain columns.** A column whose declared type is right but
not certain (a FROM `unnest`'s over an `ARRAY[…]` constructor, a relation's column defined
by a window function) leaves its set at the written reading, and a computed sibling of an
exact type is cast to its type there. Over `unnest(ARRAY[NULL::int]) u(x)`,
`COALESCE(u.x, t.n * 1)` over a `numeric` `1.25` returns `1` where 18.6 returns `1.25`, as
`COALESCE(i, f + 0)` over a base column `i` did before the computed reach (`EDGES.md` row
54). A computed `double precision` sibling is the one that decides the target, as above.

**What this reader types is also the single definition of an aggregate's certainty.**
`avg` and `sum(bigint)` take 18.6's exact `numeric` rows only over an argument whose
integer type is certain. The coercion pass decides that once, from the argument as
written, by asking this reader (`aggregate_arg_certainty`), and records it on the
aggregate (`planner::ArgCertainty`) for every other reader of its result type to read
(`aggregate_type_of`). A `COALESCE`, `CASE` or `ARRAY[…]` this reader types is one whose
every branch it types, a set this pass reads at the computed reach and casts to its common
type. So a fold called certain is one the pass resolved. A fold left at the written
reading, over a branch with a column or a scalar subquery of a guessed type as an operand,
keeps the answer an argument of unknown type gets (`EDGES.md` row 2). The verdict is taken
on the aggregate's first run and kept. An argument over an enclosing query's column, which
a correlated sub-plan's own run can't type, is decided not certain there, before the
enclosing level reads the subquery's type to choose its own casts.

The numeric category is a scope, not a measured bound. A computed branch of another
category (a string function other than the three above, temporal arithmetic, `now()`)
stays untyped, and a set holding one is in row 54's open band (`EDGES.md`), as is a set
holding a `$N`.

## A scalar subquery is a branch of its own type, where that type is resolved

`COALESCE`, a `CASE` result, an `ARRAY[…]` element and a `greatest`/`least`/`nullif`
argument unify through two readers, `concrete_branch_type` and `arg_known_type`. Neither
holds a catalog, so neither can see a scalar subquery. An invisible branch next to a
concrete one is **cast to that sibling's type** (`EDGES.md` row 65). For a subquery that's
a wrong answer, not just a missing refusal, wherever 18.6 unifies the pair. Left bare,
`COALESCE((SELECT 1.25), 2)` returns `integer` `1` against 18.6's `numeric` `1.25`, and
`DELETE FROM t WHERE COALESCE((SELECT t.n), 2) = 1` removes a row 18.6 keeps. We checked
this at `COALESCE`, a `CASE` result, an `ARRAY[…]` element, `greatest`, `least` and
`nullif`, over a subquery reading a table and over one naming the enclosing row.

So the branch is typed before the pass runs. `type_scalar_subquery_branches` wraps each
such subquery in a `Cast` to its own output column's type. Both readers already type that
shape, and it converts nothing at that type. Where the sub-plan's column is **certain**,
the type is the sub-plan's own record of it (`ColumnDesc::type_certain`), and the cast is
written over every such subquery under the fold, an operand under a branch as well as a
branch (§ A computed branch is typed where its type is certain). Where it isn't certain, a
subquery that is a branch is cast to the type it **resolves** to, or isn't wrapped. It
resolves when it's:

- a set operation with no column pending an enclosing level, whose `column_types` are
  unified (`docs/scope.md` § A pending set-op column's type is never read);
- or a target-list entry whose every column reference resolves (in the subquery's own
  input where it binds there, since a qualifier naming none of its relations belongs to an
  enclosing level; otherwise in an enclosing subquery's input; otherwise in the clause's
  own namespace), whose every scalar subquery inside is resolved in turn, over an input no
  pending set operation feeds, and naming no enum through an enclosing level, which a cast
  would erase;
- or **no** target-list entry at all, because the projection is a wildcard. The column is
  then whatever that wildcard covers: a **scan**'s, which `output_columns` reads from the
  catalog instead of guessing, or a **derived table**'s, which is that table's own query's
  column and gets the same question asked of it.

Otherwise a column that isn't certain isn't read off `Plan::output_columns`, because that
answers for a reference it can't resolve (`text`) and for a pending column (its
provisional type) instead of declining. The resolving reader asks about a nested subquery
more than once per level and derives it once, reading it back at the other asks
(`EDGES.md` row 310). The certain reading is one descent of the nest, and it's asked
first. A subquery left **bare** counts as untyped to `is_untyped_column`, so the
unification defers on it exactly as it defers on a column the scope can't type yet. That
keeps the pass idempotent, so the correlated re-walk over a level-chained scope can type
what this level couldn't.

**Resolved isn't the same as certain.** The type a resolved entry is read at is the
declared-type walkers' type, and that's a guess over a sub-plan whose own column is one: a
fold its pass left at the written reading, an `ARRAY[…]` constructor's `unnest`, an
aggregate over an enclosing query's column. The cast types the branch for its fold either
way, which keeps the fold's declaration and its values together. Whether the column the
fold defines is *certain* is read off the sub-plan before the cast is written (§ A
computed branch is typed where its type is certain).

The residual is the reading that goes **through** a guess instead of around one. A derived
table read by a *named* entry (`(SELECT z FROM (SELECT o.n AS z) w)`) resolves `z` against
the derived table's own output columns, and `output_columns` guessed `text` for those,
since `o` is a level that plan can't resolve. So the branch set is `42804` here where 18.6
returns the row. That's loud, and only pglet does it. The wildcard spelling of the same
derived table (`(SELECT * FROM (SELECT o.n AS z) w)`) goes around the guess and agrees.

An enum-typed subquery and the `name` type are excluded from the **wrap**. A `Cast` erases
an enum's identity (`EDGES.md` rows 104 and 107), and `name` isn't a target of any cast,
so those branches stay bare and the pass defers on them. The **gate** doesn't defer on
them (§ What the gate reads).

### What the gate reads, which is not what the pass can wrap

**A branch set the gate can type is judged the way 18.6 judges it. One it can't type is
left with per-branch runtime types under one declared type.** The second half is the
residual, not the design. If the pass's deferral were the whole rule, it would apply
everywhere: a bare subquery would defer the **gate** too (permanently, for a type no
`Cast` carries), and the set would be neither unified nor judged. Then
`COALESCE((SELECT current_user), i, d)` would return `postgres` where 18.6 refuses `42804`
over `i` and `d`, a pair the subquery has nothing to do with. And
`COALESCE((SELECT * FROM num1 WHERE c > k.id), 2)` would declare `numeric` while sending a
four-byte `int4` on the row that took the constant branch, which a binary client can't
decode.

So `check_branch_unification_node` reads a bare subquery through its own reader
(`branch_type_with_subqueries` / `is_untyped_branch`), which is `concrete_branch_type`
plus the one arm that needs a catalog. It answers wherever the type is **resolved**,
including the types the pass can't wrap, and defers only where a wider scope could still
type the subquery. The gate compares types and casts nothing, which is why the uncastable
pair is safe there and not in the pass.

**Answering isn't refusing**, and treating it as refusing would be a loud over-refusal.
What the gate then *does* with the type it read is up to `select_common_type`, not the
reader. On 18.6 `name` is `typcategory` `S`, with implicit `pg_cast` rows both ways
against `text` (and `varchar → name` one way). So `COALESCE((SELECT current_user), tx)`
resolves to `name` (19) as on 18.6, and the `WHERE` spelling of it keeps its rows. What
the **fold** refuses is every sibling whose type it reads as outside the string category:
`common_type_step` returns `None` on a category mismatch and the caller raises. We checked
this against 18.6 at each of `int`, `numeric`, `date`, `timestamp`, `timestamptz`, `time`,
`interval`, `boolean`, `uuid`, `jsonb`, `bytea` and `text[]`: `42804` on both engines,
with pglet's message naming the same two types after its `type error:` prefix. It's about
the type the fold *reads*, not the type it *is*: an enum column's `type_oid` is `text`
too, and the identity next to it is what the fold reads (see below). What a gate
downstream of the fold refuses is a separate question:
`ARRAY[(SELECT current_schemas(true)), ta]` is pglet's `0A000` against a two-dimensional
`name[]` on 18.6, which is the nested-array gap, not this one.

**The enum sibling next to it is refused, and one rule gives all three of 18.6's
answers.** The pair `name` joins by entering the string category (an enum operand next to
a string one) would unify if a reader that decides a common type were given a bare
`TypeOid`, since an enum's is `text`. Instead it's given `planner::common_type`'s
`ResolvedType`, the OID **and** the identity next to it, and what the rulebook models is
the `typcategory`. An enum is `E` where `text`/`varchar`/`name` are `S`, so the pair has
no common type at all: `42804 COALESCE types name and emood cannot be matched`,
`42804 <construct> types mood and text cannot be matched`, and at the comparison positions
`42883 operator does not exist: mood > text`, each byte for byte the same as 18.6. Two
**enums** share `E`, so the fold gets past the category check and `coerce_to_common_type`
is what fails: `42846 GREATEST could not convert type m3 to mood`. An enum **array** is
`A` like every other array, with its identity at the element, so that's a same-category
conversion failure again (`42846 could not convert type text[] to mood[]`). Refusing on
the type *identity* instead of the category would collapse all three into one code. The
category is what separates them. `EDGES.md` rows 114 and 115 have the measured statements.

**Every door that decides a common type takes the same `ResolvedType`, and so does every
comparison position.** Among the operator rules outside those, `concat_legality`,
`like_legality`, `starts_with_legality`, `json_get_legality`, `json_operator_legality` and
`containment_legality` still get a bare `TypeOid`, just as a string function's argument
reader gets the value. So an enum is the `text` of its label there, and `m LIKE 'a'`,
`j ? m`, `upper(m)`, `m || 1` and `ARRAY[m] @> ARRAY[t]` answer where 18.6 gives `42883`
(`EDGES.md` row 190). The branch readers (`concrete_branch_type` and
`branch_subquery_type`, the bare-scalar-subquery one) pair their OID with the identity the
exhaustive `expr_enum_with` walk derives. The doors that aren't branch readers (the set
operation's fold, a FROM-clause `VALUES` list's, a CTE's, the recursive-CTE check in the
parser, the two `JOIN … USING` merge readers, and the DDL-time static check over a stored
expression) resolve from a `ColumnDesc` and go through `ResolvedType::of_column`. A
clause's namespace carries the identity for the same reason: `ColumnScope`'s relations
hold `ResolvedType`s, not bare OIDs. So `SELECT current_user UNION ALL SELECT m FROM em`
gives that `42804` here as on 18.6, the `DELETE … WHERE tx IN (…)` over it leaves the row
18.6 leaves, and
`CREATE TABLE dch3 (m emood CHECK (COALESCE(current_user, m) IS NOT NULL))` is refused
instead of being recorded and then dumped to an `ALTER TABLE … ADD CONSTRAINT` that stock
18.6 rejects.

**The identity has a third state, and the rule over-refuses without it.**
`EnumIdentity::Undecided` is what an operand carries when this engine can't say whether
it's an enum. That happens in three cases:

- a cast to `text`, because a written `x::mood` reaches the planner as that node (the
  parser drops the enum name, `EDGES.md` row 104). A member literal's cast is that node
  alone, and any other operand's has the membership test `StringFunc::EnumIn` under it
  (`docs/enums.md` § A cast to an enum converts), which this identity doesn't read. So the
  gate reads `t::mood` the same as `t::text`;
- a bare string or `NULL` literal, because that's 18.6's `unknown`, which *adopts* the
  position's type, enum included;
- a spliced `Bound` text value, for the first reason.

Such a pair falls back to the OID rule, so `greatest(m, t::mood)`, `m = 'ok'` and
`COALESCE(m, 'sad')` answer as on 18.6. The cost is the under-refusal row 104 already
records (`m = t::text` stays silent). Deleting the state turns the
`enum/*_member_position` family red. An over-refusal here is a worse defect than the one
the rule fixes.

**A gate's reach is a separate question from its identity, and the answer is the complete
walk behind the shallow reader.** `certain_operand_type` and `concrete_branch_type`
deliberately return `None` for a computed operand, because a gate must not reject on a
guess. What that costs over a non-enum pair is `EDGES.md` row 65.
(`concrete_branch_type`'s exceptions are the numeric-category shapes whose type is certain
once their operands' are, § A computed branch is typed where its type is certain.) But the
identity is derived by a walk that *is* exhaustive. So a shallow reader alone reads a
`mood` column's identity next to `lower(s)` and then throws it away with the `None` its
OID comes back as. That's `EDGES.md` row 114's whole family, one call in.
`complete_resolved` sits behind each reader (`unknown_aware_type` over a
`ClauseLeafScope`, the same complete walk the boolean-context gate takes), and
`operand_pair` / `check_branch_unification_node` use it **only where the widened pair or
branch set names an enum**.

What that leaves unchanged differs by construct. A comparison pair is widened only where
the pair itself names an enum, so a pair with no enum keeps its answer, and
`enum/computed_operand_non_enum_pair_accepts.sql` pins that accept side. A branch set is
widened where the *set* names one, and then every branch the shallow reader can't type is
read through the complete walk, so a non-enum pair inside it is judged too:
`COALESCE(n + 0, lower(s), COALESCE(m, 'ok'))` and `greatest(n + 0, lower(s), m)` are
`42804 … types integer and text cannot be matched`, as 18.6 raises. Every verdict the
widening decides matches 18.6's, an `UPDATE` predicate among them (`EDGES.md` rows 65 and
114). Replacing the readers outright passes the whole fixture corpus and fixes most of the
row-65 shapes we tested against it. That's the fix for row 65, not part of this rule.

**An array's identity is the unification of its *typed* elements' identities, and an
`unknown` element contributes nothing to it.** That's `transformArrayExpr`'s rule (it
folds the elements through `select_common_type` before anything outside the constructor
sees the array), and `executor::expr`'s walk has the same arm. So `erases_enum_identity`'s
array arm excludes an unresolved literal instead of letting one erase the whole array. On
both engines: `ARRAY[m] < ARRAY['ok']` is `42883 … mood[] < text[]`, not the `t, f, f` an
erasing literal returns. `m = ANY(ARRAY[s,'ok'])`, `ARRAY[s,NULL]` and `ARRAY[y,'ok']`
give the element's `… mood = text` / `… mood = m3`. An erasing literal makes them return
rows, in a `DELETE` and an `UPDATE` predicate, and in a `CHECK` and a partial index that
record.

Besides the `NULL` below, the rule doesn't reach a `NULL::varchar(3)` element. That's a
`CastTypmod`, which the exclusion doesn't match, so `m = ANY(ARRAY[s, NULL::varchar(3)])`
answers against 18.6's `42883 … mood = text`, while its enum-element twin is
`42804 ARRAY types mood and character varying cannot be matched` on both. A bare
**column** element isn't one of the arm's producers (a text-shaped cast and a text-shaped
bound value), so the pair spellings that carry one refuse on both engines:
`ARRAY[m] < ARRAY[s]` is `42883 … mood[] < text[]` and `ARRAY[m] < ARRAY[y]` is
`… mood[] < m3[]`. The *branch* shape one constructor up
(`COALESCE(ARRAY[m], ARRAY[t])`) refuses too, and so does the literal spelling
(`COALESCE(ARRAY[m], ARRAY['ok'])`).

The rule does **not** reach a `NULL` next to a typed element, which this engine's own
`unify_common_type_branches` has already cast to the fold's target. That's the same node a
written `NULL::text` parses to, which 18.6 types `text` instead of `unknown`. Both are
read as the pass's output, so `ARRAY[s, NULL]` refuses as on 18.6, and
`ARRAY[m, NULL::text]` answers where 18.6 raises `42804`. That's row 104's under-refusal,
which that spelling carries from the cast anyway.

**Which positions the gates ask about is decided by the grammar, not by the arm list.** A
construct with no arm in a gate is silent for a reason upstream of any identity: the
comparison it implies is never put to `comparison_legality` at all. And because the
*reach* rule above uses the complete walk only where the pair names an enum, a missing arm
is a missing verdict, not a wrong one. The list to check the arm lists against is 18.6's
own `transformExpr` dispatch over the positions that resolve a comparison operator:
`transformAExprOp` (`Expr::BinaryOp`, which `IS [NOT] DISTINCT FROM` and the desugared
`BETWEEN` also reach), `transformAExprIn` (`Expr::InList`), `transformAExprNullIf`,
`transformCaseExpr`'s simple form, `transformAExprOpAny`/`transformAExprOpAll`
(`Expr::AnyAll`), and `transformSubLink`'s ALL/ANY/ROWCOMPARE branch, which rewrites a
bare `IN (subquery)` into `= ANY (subquery)` and resolves the operator through
`make_row_comparison_op` (`Expr::InSubquery`). Both gates cover every one of those that a
pglet `Expr` can reach. `make_row_comparison_op`'s **multi**-column route isn't one, since
a row constructor is `42601` in this parser.

**The last two arms take a one-sided rule, because their right-hand operand isn't a
sibling `Expr`.** A sub-plan's output column is read by `branch_subquery_type`, and an
array's element by `array_element_resolved_type`. Both sit where `complete_resolved` sits,
so `one_sided_operand_pair` falls the right side back to `None` (not to a shallower
answer) where the widened pair names no enum. That's the same discipline `operand_pair`
keeps, with the same consequence for rows 60 and 65. And the **conversion** runs one way.
At both positions 18.6 has already resolved the right-hand side to a concrete type before
the operator is built: a `PARAM_SUBLINK` carries its target-list entry's `exprType`, and
an `A_ArrayExpr` with no target type has folded its elements through `select_common_type`.
So neither can still be an `unknown` for the operator to adopt. `m IN (SELECT 'ok')` and
`m = ANY(ARRAY['ok'])` are `42883 operator does not exist: mood = text` on 18.6, where
`m IN ('ok')` over the same member label is an ordinary membership test, while
`'zzz' IN (SELECT m FROM w)` and `'zzz' = ANY(ARRAY[m])` give its `22P02`.

**The element's identity is the array's, and `array_element_resolved_type` folds
nothing.** The fold is one layer down, in the array's own identity (above), so this reader
takes one array suffix off the type and passes the identity through. Keyed on the node
kind instead, it would refuse `m > ANY(ARRAY['ok'::mood])`, which 18.6 answers. That array
is an `Expr::Array` too, and its identity is `Undecided` for row 104's reason, the cast
parsing to a cast to `text`. `enum/any_all_cast_element_array_accepts.sql` is the fixture
that turns red for the wider reading.

The sub-plan reader's answer has three values for the same reason every identity does. A
`ColumnDesc::enum_type` of `None` can mean two things, *not an enum* and *no identity
derived*, and reading it as the first refuses `m IN (SELECT 'ok'::mood)`,
`m IN (SELECT min(m) FROM w)`, a `GROUP BY` key and the value constructors, all of which
18.6 answers. The descriptor derives an identity for a projected **expression**
(`EDGES.md` row 107), but a **cast** still leaves that `Option` meaning two things (row
104), so the reader stays three-valued. The identity is derived from the projection
instead, by the same exhaustive walk, under the namespaces the type leaf reaches. An
unresolved **literal** entry is decided instead of deferred, because `transformTargetList`
has resolved it to `text` before the column leaves the subquery, so `m IN (SELECT 'ok')`
keeps refusing.

Both readers are still **certain-or-defer** on the *type*, and a deferral isn't a verdict.
So the residual each leaves is a right-hand side it can't type where 18.6 can:
`m IN (SELECT $1)`, `m IN (SELECT NULL)` and `m IN (SELECT * FROM unnest(…))` answer
against 18.6's `42883`, and the first one writes. The opposite residual, a **loud**
over-refusal, is left wherever one of this engine's own passes has cast an `unknown`
branch to the unified type. That writes the same node a written `'ok'::mood` parses to,
and the reader can't tell the two apart. We hit this at a set operation's branch column
and at a derived table's wildcard column:
`m IN (SELECT 'ok'::mood UNION SELECT 'sad'::mood)` and
`m IN (SELECT * FROM (VALUES ('ok'::mood)) v)` refuse where 18.6 answers. That's the one
direction that never leaves a write silent. Deferring there would answer
`m IN (SELECT 'ok' UNION SELECT 'sad')` and
`DELETE … WHERE m IN (SELECT * FROM (VALUES ('sad')) v)` with rows 18.6 refuses.
`EDGES.md` rows 114, 115 and 111 have those and the rest.

**An unknown literal that resolves to an enum is converted, not left alone.** 18.6's
`coerce_type` runs the target's input function on the `Const`, so a label the type lacks
is `22P02` in parse analysis, whatever the rows and whichever branch runs.
`planner::enum_literal_legality` is that `enum_in`. Since pglet holds an enum value as a
`Value::Text`, the conversion *is* the membership test. It's reached from the comparison
positions (`comparison_enum_literal_legality`, after the operator's own verdict, in 18.6's
order), from `branch_unification_legality` after its fold resolves a target, from the
set-operation door's `coerce_unknown_entry` (where a cast to `text`, 18.6's *other* type,
would prove nothing), and from the recursive-CTE term's fold
(`unknown_entry_enum_legality`, the same test over a term whose plan it leaves as is).
`EDGES.md` row 114 has the measurements, including the persisted `CHECK` whose dump
restored into stock 18.6 with `pg_constraint` empty and `psql` exiting `0`.

**A value the client binds is converted too, by the same predicate.** 18.6 runs the same
`enum_in` at `Bind` for an undeclared `$N` it resolved to an enum, so both doors go
through one membership test, `enum_label_legality`. The literal door is
`enum_literal_legality` above, and the bound door is `bound_param_enum_legality`, over the
enum target `infer_param_targets` recorded next to each parameter's OID
(`ParamTargets::enums`). Each host calls it at its own decode (the native server's `Bind`
and `Database::execute_bound`), and never at eval time, since 18.6 refuses over an empty
table. The target is a property of the statement, not of the transport, and a gate on only
one host would leave the other writing rows the dump can't restore. A `NULL` parameter is
skipped, and a client-**declared** parameter is planned as a cast and has no enum target
in the first place. Where a **plan** instead of a sibling `Expr` decides the target (the
`IN (SELECT …)` left-hand side, a set-operation branch), no target is recorded, and that
residual writes (`EDGES.md` row 114).

**A cast to an enum is the same conversion, at the positions 18.6 gives it.** The node a
cast of anything but a member literal builds (`planner::build_enum_cast`) carries the
enum's member list, and it's decided by the rule of the operand it converts. A string
literal is decided where every constant cast is (`validate_const_casts` at plan time and
the DDL walk over a stored expression, through `enum_in_constant_legality`). An undeclared
`$1` is decided by the bound door above, whose target the cast records too. Anything else
is decided on the value. `docs/enums.md` § A cast to an enum converts states it, and
`EDGES.md` row 241 has the measurements.

Two more residuals remain, recorded as `EDGES.md` row 152. A set the gate types and the
pass leaves bare takes its **declared** type from the walkers' first inferable branch, not
from 18.6's fold, which for `CASE` puts the `ELSE` first:
`CASE WHEN … THEN (SELECT current_user) ELSE <text> END` is `19` here against 18.6's `25`.
`name` and `text` share their text and binary encoding, so a **scalar** member's value
bytes are the same either way. An **array** member's aren't, because its binary payload
carries the element OID, so the `name[]`/`text[]` spellings of that `CASE` differ in their
bytes as well as their descriptor. And a branch the gate can't type at all (a wildcard
over a function scan whose column isn't certain, such as an `ARRAY[…]` constructor's
`unnest`, which is `wildcard_column_type`'s `_` arm) leaves the set neither cast nor
refused. For a binary client that's a raw `int4` under a `numeric` descriptor.

## A declaration-only type has no arithmetic, and two walks have to say so

`oid` and `"char"` are catalog **declaration** types with no value shape of their own. An
`oid` *is* a `Value::Int4`, and a `"char"` *is* a `Value::Text`. So `eval_arithmetic`,
which decides on the value pair, can't tell `conrelid + 1` from `int + int`, and would
return an integer where 18.6 raises `42883 operator does not exist: oid + integer`. The
gate is `planner::legality`'s `arithmetic_legality` / `negate_legality` /
`math_fn_arg_legality`. It's narrow on purpose: it refuses an `oid` / `"char"` operand and
says nothing about any other pair.

**Each of the three has its own list of callers, and that list is the part that goes
stale, so re-derive it with grep instead of trusting it here.** As of the last check,
`arithmetic_legality` and `negate_legality` are called from two sites:
`check_expr_node_operand_types` (DDL time) and `eval_expr_full` (the scalar eval walk).
`math_fn_arg_legality` is called from the scalar eval walk **only**, since
`check_expr_node_operand_types` has no `MathFn` arm, so this is the one rule that can't
reach a stored expression by any route. A grouping query's expressions also go through the
scalar walk, with the grouping lending it its aggregates' values (`docs/scope.md` § A
grouping query's aggregates are values the scalar walk reads by node). So the gate reaches
an operator above an aggregate the same as anywhere else: `max(conrelid) + 1`,
`- max(conrelid)` and `abs(max(conrelid))` are `42883` as on 18.6.

**The other half is what each caller reads the operand type *through***, and that's
`docs/declared_types.md`'s § A third `LeafScope`. The eval-time caller takes the complete
declared-type walk over its own scope, not a reader that enumerates `Expr` kinds.

The residuals are measured. The gate runs at **eval time**, so a statement whose
evaluation never reaches the node still answers, where 18.6 refuses in parse analysis. And
the *contexts* an `oid` reaches that aren't arithmetic (the common-type fold, the
assignment cast, an aggregate's result type) diverge the other way, with pglet stricter,
recorded as `EDGES.md` row 87.

## A canonical type name decomposes once

**A canonical type name is split once, into base, modifier and array suffix, and each of
the three is then decided by the *type*, never by the spelling.** Whether a `(…)` is legal
is a fact about the base type. A modifier on a type that carries none is refused, never
dropped. **A modifier the type does carry but refuses is a refusal too**: the `(…)` is a
*value the type validates*, not a hint it can ignore. And a `[]` makes the modifier the
**element's**. It's never a reason to lose either the modifier or the array.

`executor::type_resolution`'s `decompose_type_name` is the single place that splits it,
and the order inside it matters. `parser::parse_type_name` appends the suffix to a scalar
name that may already carry its modifier, so the canonical spelling is `varchar(3)[]` and
the `[]` has to come off **before** the `(`. Cutting on `(` first leaves the suffix inside
the discarded tail, and the name then resolves to the *scalar* element with the cap lost
too. A `varchar(3)[]` column becomes an unconstrained scalar `varchar` that takes
`'{abcdefg,xy}'` whole, where 18.6 answers `22001`.

**Which types carry a modifier is decided by `pg_type.typmodin <> 0`, checked on 18.6
instead of guessed from the spellings.** `bpchar`, `varchar`, `numeric`, `time`, `timetz`,
`timestamp`, `timestamptz` and `interval` carry a modifier. `bool`, `bytea`, `"char"`,
`date`, `float4`, `float8`, `int2`, `int4`, `int8`, `json`, `jsonb`, `money`, `name`,
`oid`, `regclass`, `text`, `uuid`, `xml` and every enum (`typtype = 'e'`) carry none.
`pglet_core`'s `type_oid_carries_modifier` is that list. It's exhaustive over `TypeOid`,
so a new type has to decide instead of defaulting into the refusing half. An **array
answers at its element's width**, which is 18.6's own shape and not a convenience:
`LookupTypeName` resolves `varchar(3)[]` to `_varchar` before `typenameTypeMod` reads the
modifier, and `_varchar.typmodin` is `varchartypmodin` where `_int4`'s is 0.

**The refusal comes from `resolve_column_type`, at the point where `typenameTypeMod`
raises it.** That's after the name resolves, so `nosuchtype(3)` still answers `42704`. It
uses 18.6's own message, `type modifier is not allowed for type "…"`. The name in it is
the spelling, plus one `[]` for an array of any number of dimensions. The exception is the
`SERIAL` family, which 18.6's grammar has already rewritten by then (`serial(3)` names
`integer`). pglet reads that off the resolved `TypeOid` instead of keeping a second list.

It has four call sites outside the resolver: the `::`/`CAST(… AS …)` pair, a column
definition, `ADD COLUMN` and a typed constant. The first three go through
`resolve_column_type` and the last through `resolve_constant_type`, two entry points onto
the same `resolve_type_name` that raises this, so one gate still covers them. The
typed-constant position is the one that can't *reach* the refusal **on the default
route**, since `int4(3) '1'` isn't an `is_multiword_type_lead` window and falls to the
function-call path's own `0A000` (`EDGES.md` row 121's shape). Under
`--coerce-unknown-types`, though, three windows that *are* one and whose base pglet
doesn't model (`bpchar(3) 'ab'`, `bit(3) '1'`, `varbit(3) '1'`) resolve to `text` and do
reach it. That's the coerce fork's own divergence (`planner::unknown_types`, `EDGES.md`
row 119). An **enum** is covered by keying the catalog lookup on the *base*: `moodg(3)`
finds the type its modifier is attached to, instead of asking about a name nobody wrote.

### A modifier the type refuses

**A carrier validates its modifier, and a value it refuses fails the statement.**
`typenameTypeMod` first refuses on its own account and then **delegates**. It tests
`typmodin == InvalidOid` and raises the `42601` above, and *then* calls the typmodin,
which can refuse what it's given. Its own half covers more than that one `42601`, and how
many arms it has isn't part of the rule. We measured two more, each under
`LOCATION: typenameTypeMod` on the oracle:

- A `+`-signed or computed component is
  `42601 type modifiers must be simple constants or identifiers` there
  (`parse_type.c:407`), where pglet answers its own `42601` from the modifier reader:
  `expected a number, found number` at `numeric(+5)`, and `expected ')', found '+'` at
  `numeric(1+1)` (both checked on pglet).
- A modifier on a **shell** type is
  `42601 type modifier cannot be specified for shell type "zshell"` there
  (`parse_type.c:352`, after `CREATE TYPE zshell`). pglet has nothing that reaches that
  arm: it models no shell type at all, and the statement that creates one is refused.
  `CREATE TYPE zshell` is `42601 syntax error: expected 'as', found end of input` here,
  as we checked.

pglet's `resolve_type_name` runs both halves in that order. The delegated half is the one
a reader can lose. If a `parse_length_typmod` or `parse_numeric_typmod` refusal is read as
**no cap**, `'ab'::char(0)` becomes `ab`, a `CREATE TABLE q (c CHAR(0))` column gets built
that stores `'abcdef'` where 18.6 has no table to hold the row, and the scale `2` from
`1.5::numeric(5,2,3)` gets applied, a spelling 18.6 refuses entirely. "No modifier was
spelled" is `TypeNameParts::modifier`, a different `Option` one step earlier, and the rule
is to keep the two apart (`EDGES.md` row 124).

**The class belongs to the `(…)`, not to `char`.** It isn't a *character length* problem.
The numeric modifier has the same site, the same silence and the same mechanism, and it
has a case no length can spell (a negative precision) along with spellings the `char`
grammar refuses before any type reads them. The cases, grouped by the layer that decides
each, every cell checked on 18.6 and on pglet. The **verdict** column is pglet's message
(byte-identical to 18.6's at every `22023`, but not at the grammar `42601`; see the
message residual below):

| member | verdict | layer |
| --- | --- | --- |
| `char(0)` / `character(0)` — zero is the only sub-1 length the grammar admits (`char(00)` is the same value, `char(-1)` never reaches the type) | `22023 length for type char must be at least 1` | `anychar_typmodin` |
| `varchar(0)` / `character varying(0)` | `22023 length for type varchar must be at least 1` | `anychar_typmodin` |
| either family, `n > 10485760` | `22023 length for type {char,varchar} cannot exceed 10485760` | `anychar_typmodin` |
| `char(3.5)`, `char(2147483648)`, `char(99999999999999999999)` | `42601 syntax error: type modifier must be an integer that fits int4, found "…"` (18.6: `syntax error at or near "…"`) | the grammar's `Iconst` |
| `char(-1)`, `char(+3)` — either sign, either family | `42601 syntax error: expected a number, found number` (18.6 quotes the sign: `syntax error at or near "-"` / `"+"`) | the grammar's `Iconst` |
| `numeric(p)`, `p` outside `1 ..= 1000` (a negative `p` included) | `22023 NUMERIC precision <p> must be between 1 and 1000` | `numerictypmodin`'s `make_numeric_typmod_safe` |
| `numeric(p,s)`, `s` outside `-1000 ..= 1000` | `22023 NUMERIC scale <s> must be between -1000 and 1000` | `numerictypmodin`'s `make_numeric_typmod_safe` |
| `numeric(a,b,c)` and longer | `22023 invalid NUMERIC type modifier` | `numerictypmodin`'s arity count |
| a `numeric` component outside `int32` | `22003` | `ArrayGetIntegerTypmods` |
| a `numeric` component that is not an integer (`numeric(1.5)`) | `22P02 invalid input syntax for type integer: "1.5"` | `ArrayGetIntegerTypmods` |

**There are two grammar layers, and which one answers depends on the type's *spelling*,
not the type.** A character length **as the `character` keyword production spells it** is
`CharacterWithLength: character '(' Iconst ')'`, and an `ICONST` is an unsigned integer
that fits `int32`. So at those spellings a fractional or out-of-`int32` length is refused
in the grammar and never reaches a modifier reader. That's why `parser::type_name` reads
it through the same `opt_precision` the temporal and `opt_float` windows already use
(`docs/parser.md`). A looser window would let the `22023` fire on a number nobody could
have written, with a message naming a bound that has nothing to do with it. A `numeric`
modifier has no such production (`opt_type_modifiers: '(' expr_list ')'`), so its
components reach `typenameTypeMod` as text and are coerced to `int4` there. That's why
that family gives a `22003` and a `22P02` at all, and why a **negative** precision is a
range verdict here but a syntax error at `char(-1)`.

**On 18.6 that `expr_list` window isn't only `numeric`'s, but in pglet only `numeric`'s
spelling reaches it.** The `Iconst` productions (`CharacterWithLength`, `ConstDatetime`,
`ConstInterval` and `opt_float`) narrow a *named* set of spellings, not a set of types.
`GenericType: type_function_name opt_type_modifiers` hands the `expr_list` to any type
name none of them captures, and `BitWithLength` is a keyword production that is an
`expr_list` itself. A double-quoted type name is an `IDENT` and so a `type_function_name`,
so by the grammar **any** carrier has a spelling that reaches the coercion. That last step
follows from the productions' shape, not from a sweep. What we measured on the oracle is
the set below, each `(1.5)` answering
`22P02 invalid input syntax for type integer: "1.5"`: `bpchar`, `timestamptz` and `varbit`
unquoted; `"varchar"`, `"bpchar"`, `"timestamp"`, `"time"`, `"timetz"`, `"interval"` and
`"numeric"` quoted; and `bit varying(1.5)` through `BitWithLength`. The `22003` and the
`22023` arrive the same way: `'ab'::bpchar(2147483648)` is
`value "2147483648" is out of range for type integer`, and `'ab'::bpchar(0)` is
`anychar_typmodin`'s `length for type char must be at least 1`, the same verdict the
keyword `char(0)` gets, here reached through a spelling no `Iconst` narrows.

pglet reaches none of those, through three different refusals: `42704` for a type it
doesn't model (`bpchar`, `bit`, `varbit`), `42601` for a quoted type name (the rule is in
the doc for `parser::type_name`'s `expect_type_word`, since that reader only takes a bare
`TokenKind::Identifier`), and its own `42601` where `opt_precision` does read the spelling
(`timestamptz(1.5)`, `timetz(1.5)`). So **within pglet**, `numeric`'s is the only modifier
window whose components reach the coercion. Every other *carrier*'s is one `Iconst` that
its reader has already proved is an `int32`. That's a fact about pglet's grammar, and 18.6
doesn't share it. `EDGES.md` row 124 has the divergence.

**The order inside one modifier reader is part of the contract**, measured rather than
inferred. On 18.6 it's all **one function**, `numerictypmodin`: the arity `22023`'s
`LOCATION` is that function, and the two range checks' is the `make_numeric_typmod_safe`
it calls. `numeric(0)` and `numeric(0,1001)` name the precision from the same line, so
both arity branches share the one check. It coerces **every** component to `int4` first,
then the count *selects* the branch, and then the range check tests the precision before
the scale (the one-component branch has no scale to reach). So `numeric(5,2,1.5)` is the
coercion's `22P02` instead of the arity's `22023`, `numeric(0,1.5)` is the coercion's
instead of the precision's, and `numeric(0,1001)` names the precision instead of the
scale. That's three pairs, each separating one step from the next. Reordering the whole
pass keeps every single refusal right but answers the wrong one at all three.

**Where it runs needed no decision, and that's the point of putting it there.** Both
refusals come from `resolve_type_name`, so the `22023` inherits the `42601`'s measured
error order instead of needing its own. We re-checked each pair below on both engines for
the `22023`, and each has its own fixture: `CREATE TABLE`'s left-to-right column walk
(`CREATE TABLE g (a char(0), b nosuchtype)` gives the length error, and the mirror is
`42704`), before the `42P07` for an existing relation, after the `IF NOT EXISTS` skip, and
on `ADD COLUMN` after the relation lookup, the column-existence check and that statement's
own skip. It also inherits the residual. At a **cast**, pglet decides the declared type
before 18.6 reads the relation, so `SELECT 'ab'::char(0) FROM nosuchtbl` is the `22023`
here against `42P01` there. That's the same position where
`SELECT 'ab'::nosuchtype FROM nosuchtbl` and `SELECT 1::int4(3) FROM nosuchtable` already
diverge, not a new class (`EDGES.md` row 119's first residual).

**Residuals.** Each case is in `EDGES.md` row 124's bullet. The `22003`'s message and the
grammar `42601`'s message are pglet's own under the same SQLSTATE. The wider integer-input
class (`'2147483648'::int` `22P02` against `22003`) is `EDGES.md` row 324. At a typed
constant, a refused number falls through the speculative read to the function-call path,
which is loud and, at every spelling but `character varying`, uses the wrong code
([parser.md](parser.md) § Known lexical / parse divergences, "A number the window refuses
parts the two positions").

**One gate covers every position that *resolves* a declared type, which is fewer than
every position that spells one.** `CREATE`/`ALTER SEQUENCE … AS <type>` and a column
identity clause's `( AS <type> )` call `parse_type_name` and **discard** what it returns,
so `resolve_column_type` never runs there. `CREATE SEQUENCE sq1 AS int4(3)`,
`… AS varchar(3)` and `… AS nosuchtype(3)` are each accepted here, against 18.6's `42601`
/ `22023 sequence type must be smallint, integer, or bigint` / `42704`. `AS varchar(0)` is
accepted against its `22023 length for type varchar must be at least 1`. An identity
column's `(AS char(0))` and each of five identity spellings we tested (`int4`, `bigint`,
`int4(3)`, `varchar(3)`, `nosuchtype`) are accepted against its blanket
`42601 conflicting or redundant options`. All of these were checked on both engines.
That's the dropped-`AS` clause's divergence, not this rule's. No `EDGES.md` row owns it
(row 116 records its in-range `FLOAT(n)` spellings), and none of it is pinned.

**A captured modifier belongs to the element everywhere downstream**, and the readers
split on whether their question is about the element or about the column.
`ResolvedColumnType`/`Column`'s `numeric_typmod`, `varchar_length` and
`temporal_precision` hold the element's. `coerce_column_value` and `Expr::CastTypmod` both
go through one `coerce::apply_typmod`, whose array arm walks the elements under the
caller's `is_explicit` context (18.6's `ArrayCoerceExpr`). So
`'{abcdefg,xy}'::varchar(3)[]` truncates to `{abc,xy}`, and the same literal stored into
such a column is `22001`. `Column::pg_type_modifier` reports the element's modifier as is,
which is what 18.6's `atttypmod` holds for an array column (`7` for a `varchar(3)[]`, `3`
for a `timestamp(3)[]`, `2147418115` for an `interval(3)[]`, each measured). How that
reader *encodes* it depends on the type too: a length adds `VARHDRSZ`, a `numeric(p,s)`
packs both and adds it, a `timestamp`/`timestamptz`/`time` precision is the bare `p`, and
an `interval`'s packs `INTERVAL_FULL_RANGE` next to it (`(0x7FFF << 16) | p`).

**Two renderers spell a type name with a modifier, and both put it inside the suffix.**
One is `dump`'s `column_type_sql`, for a *column*'s declared type (or the export reloads a
scalar column). The other is `deparse`'s `cast_type_spelling`, reached from
`render_cast`, for an `Expr::CastTypmod`'s *target*, in both `RenderMode`s at once. A list
of readers built by grepping `numeric_typmod` / `varchar_length` misses the second, which
reads neither: it reads the `Typmod` the cast node carries, which
`ResolvedColumnType::typmod` produced from those two fields one step earlier. A modifier
after the suffix is one wrong spelling per `RenderMode`: `varchar[](3)` at the dump's
statement kinds (`Portable`'s name function is `type_oid_sql`), and
`character varying[](3)` at the catalog reader positions (`catalog_type_name`):
`pg_get_constraintdef`, `check_clause`, `column_default` and `indexdef`'s key and
predicate. Neither is valid SQL on either engine (`42601` on each, measured).

`information_schema.columns.character_maximum_length` is the reader whose question is
about the *column*, so it gates on the scalar `TypeOid::Varchar` and returns NULL for an
array. That matches 18.6, next to the very `atttypmod` that isn't NULL. That reader is
part of the rule, not a side effect of it, since it reads the same field.
`datetime_precision` is the same reader one type family over and uses the same gate, with
one twist the length reader doesn't have: it's a fact about the **type**, not about
whether a modifier was spelled. So a bare `timestamp` returns `6`, and a `date`, which
carries no modifier at all, returns `0` (both measured), next to the NULL an array column
and every non-temporal column return.

**The rule's third arm: a modifier the type *does* carry is kept, clamped by the type's
own maximum, and enforced, never discarded.** Six types carry one here. `varchar`/`char`
carry a length and `numeric` a `(p,s)`. `timestamp`, `timestamptz`, `time` and `interval`
carry a **sub-second precision**, captured as `ResolvedColumnType::temporal_precision` and
enforced through the same `coerce::apply_typmod` the other two use. So
`'…05.6789'::timestamp(3)` rounds to `…05.679` at a cast, at a column store, and
element-wise under a `[]`, as on 18.6. The rounding rule itself (half away from zero about
the value's *internal* zero, which for a timestamp is 2000-01-01 and not the wall clock)
is in `docs/temporal.md` § Sub-second precision.

Three things about that arm depend on the type, stated in [temporal.md](temporal.md) §
Sub-second precision. A `p` above 6 is clamped to a captured `6` (`atttypmod` `6` against
a bare type's `-1`). The grammar is one `Iconst`, so `timestamp(3,1)`, `timestamp(3.5)`,
`timestamp(-1)`, `timestamp(99999999999999)` and their four `interval` spellings are
`42601` on both engines. And the canonical name carries the precision last
(`time with time zone(3)`), so `decompose_type_name`'s first-`(` cut can't read a tz-time
as a `time`. 18.6's mid-name spelling is a **rendering**, reproduced only in `deparse`'s
`Catalog` mode.

`timetz`'s share of the arm has no effect: the type is `42704` here whatever the
modifier.

**The rule's fourth arm: a modifier that *selects* the type is used up by the choice, not
carried.** `FLOAT(n)` is the only case, and it isn't a narrowing modifier at all. 18.6's
`opt_float` reduces `FLOAT '(' Iconst ')'` to `SystemTypeName("float4")` for `n` in 1–24
and to `("float8")` for 25–53, raising
`22023 precision for type float must be {at least 1 bit, less than 54 bits}` outside that.
`parser::type_name`'s `float_type_name` does the same and returns the **selected** type's
canonical spelling, `real` or `double precision`, the same two strings its own `real` and
`double precision` arms already emit. So no canonical name this reader emits for a `FLOAT`
spelling has a `(…)` at all: the `type_oid_carries_modifier` gate above is never reached
for one, and `decompose_type_name`'s first-`(` cut has nothing to read. A bare `FLOAT`
becomes `double precision` too, as `opt_float`'s `/*EMPTY*/` production becomes `float8`.
Without the selection, the divergence **writes**: a `FLOAT(1)` column mapped to `float8`
stores `0.1234567890123456` whole where 18.6 stores a `real`'s `0.12345679`, at an
`INSERT`, an `ADD COLUMN` backfill, a `DEFAULT`, and out through `COPY … TO STDOUT`
(`EDGES.md` row 116).

**Where the refusal runs is the arm's second decision**, and it isn't this section's
`typenameTypeMod` placement. 18.6 raises `opt_float`'s `22023` *in the grammar*, before
any name resolution, so pglet raises it in the parser. That's what makes the two positions
that resolve no type name at all agree: `CREATE SEQUENCE s AS FLOAT(0)` and an identity
column's `(AS FLOAT(0))`, both of which read a type name and discard it. Their in-range
spellings still differ, under two *different* codes because of the position
(`22023 sequence type must be smallint, integer, or bigint` and
`42601 conflicting or redundant options`). That split is the position's gap, not this
arm's. The refusal carries 18.6's `P` (the number's own 1-based codepoint index,
`parser_errposition(@2)`), equal at each spelling we tried.

**Being in the grammar also changes statement-level error precedence**, and it wins where
this section's `42601` loses. `ALTER TABLE t ADD COLUMN IF NOT EXISTS b float(0)` over an
existing `b` is the `22023` on both engines, ahead of the column-existence check *and*
ahead of the `IF NOT EXISTS` skip, which is a Postgres **success**. An `int4(3)` at that
position loses to both, because `typenameTypeMod` runs after them. Without `IF NOT EXISTS`
the statement is `42701 column "b" of relation "zex" already exists` on both engines, and
with it the statement **succeeds** on both (18.6 raising a `skipping` notice). Same shape,
opposite winner at each spelling, decided by which phase the refusal lives in.

**The typed-constant reader commits on `float (`**, so its speculative restore doesn't
swallow the grammar-phase refusal ([parser.md](parser.md) § Known lexical / parse
divergences, the speculation's "third outcome"; `commits_on_open_paren`'s doc names the
lead words that must never join it).

**What the placement costs** is `EDGES.md` row 119's *first* residual, and it's one
statement wide (the row's second residual, the types pglet doesn't model, is a different
thing and isn't affected). 18.6 resolves the relation first and reads the modifier in
parse analysis, so wherever pglet decides a declared type earlier, the refusal outranks a
`42P01` that 18.6 answers first. `SELECT 1::int4(3) FROM nosuchtable` is `42601` here
against `42P01` there, as its `42704` twin `SELECT 1::nosuchtype FROM nosuchtable` is.
`ADD COLUMN` isn't affected: 18.6's `check_for_column_name_collision` runs before the
`BuildDescForRelation` that resolves the type, so pglet resolves there too. The relation's
`42P01`, the column's `42701` and the `IF NOT EXISTS` skip each outrank the verdict
`resolve_column_type` raises, though not the grammar-phase `22023` above, which beats all
three.

**The two statements behave differently on 18.6, and pglet copies that.** `CREATE TABLE`'s
`tableElts` walk comes before even the `42P07`, so `CREATE TABLE ex (a int4(3))` over an
existing `ex` is `42601` on both engines, while `ALTER TABLE ex ADD COLUMN a int4(3)` over
an existing `a` is `42701` on both. What still travels together is the `42704`/`42601`
pair itself. 18.6 raises them from one `LookupTypeName`-then-`typenameTypeMod` sequence,
so a placement change moves both, or it splits a pair 18.6 keeps together.

## A conversion has a *context* as well as a legality

If one table serves both casts and stores, every write runs an explicit-only cast.
`pg_cast.castcontext` splits implicit / assignment / explicit, and pglet's
`executor::coerce`'s `coerce_value` is both the cast implementation *and* the store-time
conversion. So without a guard, a pair Postgres marks `'e'` is performed silently when a
value is stored. For example, `int ↔ bool` is `'e'` in both directions, so 18.6 answers
`SELECT 1::bool` → `t` and `SELECT true::int` → `1`, but refuses
`INSERT INTO t (boolcol) VALUES (1)` and its mirror with
`42804 column "…" is of type … but expression is of type …`, where the unguarded table
stores `true`. `constraints::coerce_column_value`, the finalizer every assignment of a
non-NULL value goes through (`docs/numeric.md`), refuses those pairs up front, using the
same `is_explicit` split the `varchar(n)` typmod two lines below it already uses.
**Deleting the arm from the shared table instead passes every test and is wrong.** We
checked both ways: the whole fixture corpus and the whole workspace pass without it (no
fixture casts an integer to a boolean), and `SELECT 1::bool` then fails with `42846`.

**The write routes that finalizer can't see are closed one layer up, and the class they
belong to is wider than `pg_cast`.** The planner's `coerce_to_type` wraps an assignment
right-hand side in an `Expr::Cast` to the column type, so the evaluator converts under
explicit-cast rules and the finalizer never sees the source type at all. Without a gate,
`INSERT INTO tb (b) VALUES (1)` stores `true` and `UPDATE ti SET a = true` stores `1`. The
gate for that is at **lowering**, before the wrap, at each of the four assignment sites
(`INSERT … VALUES`, `INSERT … SELECT`, `UPDATE … SET`, `ON CONFLICT … DO UPDATE SET`). It
uses one rule, `planner::legality`'s `assignment_legality`, reached by two routes. Three
of the sites go through `planner::coercion`'s `check_assignment` (`lower_insert_rows`,
`lower_on_conflict`, `lower_update`). `INSERT … SELECT`, whose right-hand side is a plan's
output column instead of an expression to wrap, calls `assignment_legality` directly from
`check_insert_select_assignments`.

The gate couldn't go anywhere else. The finalizer reads a `Value`, and a legal unknown
literal and an illegal typed `text` are the same `Value::Text`, while narrowing the cast
table takes `SELECT 1::bool` down with it. And `transformAssignedExpr` refuses more than
the explicit-only list: a pair with no `pg_cast` row at all is refused too. So
`text → integer`, `jsonb → integer`, `uuid ← text`, `time ← date` and `uuid[] ← text[]`
are each `42804`, like `integer → boolean`
(`tests/fixtures/{insert,update}/assignment_cast_*` pins the four sites, the
unknown-literal carve-out and the error order).

What it accepts comes down to four rules: the same type, an `'i'`/`'a'` `pg_cast` row, I/O
coercion into a string-category **target** (which is why every type assigns into
`text`/`varchar` and the reverse never does), and element-wise for two arrays. Those rules
were read off a measured accept matrix over every pair of the type spellings pglet models,
and they're locked as that matrix, not as the rules
(`the_four_rules_reproduce_the_measured_matrix`). The finalizer's own pair list is
narrower and hands both its verdict *and* its message to the same rulebook. It covers the
write paths with no plan-time gate above them (`COPY`'s decoded cells, `resolve_default`'s
stored defaults, and the `ADD COLUMN` back-fill probe).

**A fifth position uses the same rulebook, and it isn't a lowering site.**
`executor::ddl`'s `cook_column_default` sits where 18.6's `cookDefault` does: the DDL-time
cook of a column `DEFAULT`, on the `CREATE TABLE`, `ALTER … ADD COLUMN` and
`ALTER … SET DEFAULT` routes alike. It reaches `assignment_legality` by **two** routes of
its own: `check_assignment` for the expression arm, and `assignment_legality` directly for
the bare-constant arm the parser has already folded to a `Value`. That arm skips the
`Text` and `Null` members, because those *are* Postgres's unknowns. Re-derive the caller
list by grepping both names instead of trusting a count here.

**Its statement-level order matches 18.6, and neither a plain "before the probe" nor an
"after the probe" placement gets it right.** The gate is a type verdict 18.6 reaches
without evaluating. So it **loses** to the column type's `42704` and to the input-function
family the target type runs on an unknown literal (`INT DEFAULT 'zzz'::date` is `22007` on
both engines, `INT DEFAULT 'x'` is `22P02`). It **beats** the value-dependent family
(`22012` / `22003`: `int DEFAULT (2147483647 + 1)::text` is `42804`, not `22003`) and the
constraint naming block. It's raised *inside* the cook, so the cook's own losers are also
its losers. A gate that returns before `cook_column_default` is called is ahead of every
step in it, and `CREATE TABLE k (a INT DEFAULT true, PRIMARY KEY (a, a))` and
`CREATE TABLE k2 (ctid INT, b INT DEFAULT true)` are `42701` on both engines, against the
`42804` the gate gives alone ([ddl.md](ddl.md) § A column `DEFAULT` is an expression the
executor cooks has that list). So the probe's value error is *held* and re-raised after
the gate: `ADD COLUMN c INT DEFAULT (1/0)::text` is `42804`, 18.6's answer on that route,
instead of `22012`.

**Two of the measured residuals, recorded but not fixed** (`EDGES.md` row 47 has the
sweep's others). The engine-valued `ColumnDefault` arms (the `now()` family,
`CURRENT_DATE` / `CURRENT_TIME`, `gen_random_uuid()`, `nextval`) return before the probe
and the cast, so they get no gate at all, and `int DEFAULT now()` is accepted here against
18.6's `42804`. And an **enum** target is outside the gate for a reason in the rulebook,
not the cook: pglet types an enum value as `text`, so `assignment_legality` is asked about
a `text` target and accepts. `e vis DEFAULT 5` is `42804` on 18.6 and accepted here.


## A boolean context is that same conversion with `boolean` as the target

**A boolean context resolves its operand to `boolean` where the operand is read, and only
two things resolve: `boolean` itself, and an unresolved literal.** Every other type is
`42804 argument of <CTX> must be type boolean, not type <T>`, raised at the position 18.6
raises it. An unresolved literal is *converted* instead of refused, through the boolean
input function. So `WHERE 't'` is a row, `WHERE 'f'` is none, and `WHERE 'abc'` is
`22P02`. No position substitutes a truth value for a non-boolean: not FALSE, not TRUE, not
a skipped write, not `0A000`.

It goes next to the section above because it **is** that section's rule with one target
filled in. 18.6 sends every boolean context through a single function, `coerce_to_boolean`
(`src/backend/parser/parse_coerce.c` in the PostgreSQL source), which attempts a
**`COERCION_ASSIGNMENT`** coercion to `boolean`. `pg_cast` has exactly two rows into
`boolean`, `integer→boolean` and `jsonb→boolean`, and **both are explicit-context**
(measured). That's why the rule is complete and short, and why pglet asks
`planner::legality`'s `assignment_compatible` with `TypeOid::Bool` as the target instead
of adding a second predicate. The same measured accept matrix that locks the assignment
gate locks this one.

**Substituting a truth value is wrong in both directions, and one statement can hit
both.** If a non-boolean operand is read as FALSE at a filter and as TRUE at a
connective's catch-all arm, then `WHERE a` over a row holding `9` matches no row, and
`WHERE a AND true` matches it. One operand gives opposite answers depending on whether a
connective is above it. A `CHECK` that only fails on an exact `Bool(false)` is a third
polarity, admitting **every** row under a non-boolean predicate. The accept side matters
too, which makes this a data-loss rule and not just a strictness one: with the literal
unconverted, `UPDATE u1 SET a = 3 FROM u2 WHERE 't'` silently leaves the row alone on a
statement both engines accept.

### One gate, one resolver, and an interleave

- **Plan time**: `planner::legality`'s `boolean_context_legality` over
  `assignment_compatible(x, Bool)`, plus `boolean_context_literal` for the conversion the
  gate lets through. `check_boolean_operand` is the pair, so no caller can take only half
  of it. The construct name is carried on `BoolContext` (`planner::expr`), the
  generalization of `BoolTest::sql()`, so the message is defined in one place instead of
  each call site passing a `&str`.
- **The operand is read through the *complete* type walk.** `unknown_aware_type` is
  `Expr::infer_type` with Postgres's `unknown` answered as `None`, generic in its
  `LeafScope`, and a clause's `ColumnScope` becomes one through `ClauseLeafScope`
  (`planner::coercion`). The narrow readers next to it, `certain_operand_type` and
  `arg_known_type`, deliberately return `None` for a scalar subquery, and for an operator
  result or a function call outside `arg_known_type`'s numeric-category shapes, because
  *their* rules must not reject on a guess. This one must, because `WHERE k + 1` is
  `42804` on 18.6 whatever the row holds.
- **Eval time**: `executor::expr`'s `boolean_context_operand`, which returns a
  three-valued answer, with `predicate_holds` as the two-valued question a filter asks. It
  decides an expression's truth at the sites we found by asking, for each position 18.6
  sends through `coerce_to_boolean`, *where does pglet evaluate that clause*. Those are:
  the `Plan::Filter` walk, the three DML filters (`scan_matching_rows`,
  `collect_updated_from`, the `ON CONFLICT` action), `HAVING`, `JOIN … ON`, `CHECK`,
  **both** partial-index predicates (one per crate, see below), the GIN guard (which
  leaves its failure to the recheck `Filter`), the searched `CASE WHEN`, `AND`/`OR`,
  `NOT`, and `IS [NOT] TRUE`. A `Value::Text` goes through `text_to_bool` at each.

  `AND`/`OR` resolve the left operand first and stop where it decides the answer
  (`connective_decided_by`), as the `EEOP_BOOL_AND_STEP`/`EEOP_BOOL_OR_STEP` steps of
  18.6's `ExecInterpExpr` do. So the right operand (its resolution and any error in it) is
  only reached where the left leaves the answer open, a NULL left operand included.

  Treat that as the list the derivation found, not a claim that it's complete. **Re-derive
  it by asking that question, not by grepping the datum.** Grepping also finds five sites
  that are *not* members: the `USING` merge's equality, an index range scan's own
  comparison, the simple `CASE`'s `=`, an `IN` list's `=` and its result. Each of those is
  an operator result pglet built, not an operand a user wrote.
- **Both halves are needed.** The plan-time reader returns `None` wherever it can't be
  certain (an unbound `$N`, a subquery a scope couldn't type) and defers. The eval-time
  one never reaches a filter over an empty table, which is why the unresolved-literal
  *conversion* happens at plan time (`WHERE 'abc'` over an empty table is `22P02` on
  18.6).
- **The connective contexts are interleaved.** 18.6's `transformBoolExpr` and
  `transformCaseExpr` transform one argument and coerce it before touching the next, so
  `AND`/`OR`/`NOT`/searched `CASE WHEN` gate per argument: `CHECK (a AND b LIKE 'x')` is
  `42804` and `CHECK (b LIKE 'x' AND a)` is `42883`, and `CASE WHEN 1 THEN nosuchcol END`
  is `42804` instead of the `THEN` arm's `42703`. No pair of whole-subtree passes gets
  either right. `IS [NOT] TRUE` is **not** interleaved (`transformBooleanTest` transforms
  the whole argument and then coerces), so it stays an ordinary post-order node check. The
  interleave uses `Expr::for_each_child_ctx`, which is `for_each_child` with each child's
  context next to it. There's one arm list, so "which children are boolean operands" can't
  go stale against "which children are there".
- **A parameter resolves too, at the positions the child walk carries a context for.** A
  `$N` at a clause root, or at a node position `Expr::for_each_child_ctx` marks, infers
  `boolean` at `PRIO_CONTEXT`, so an explicit `$1::int` still wins the type, and then
  fails the gate, which is what 18.6 does too. `IS [NOT] TRUE` is the measured residual on
  the other side of that shared list. Its gate is an ordinary post-order node check, which
  carries no child context, so the operand's `$N` stays OID 0 where 18.6 describes
  `boolean`. The gate still answers, but the inference doesn't
  ([param_binding.md](param_binding.md) § Design: infer at Parse, decode at Bind, resolve
  at Execute).

### Where each clause-root gate sits, and why the placements disagree

A **node** position is attached to an `Expr`. A **clause root** isn't attached to
anything, so each lowering site places its own, where that statement's `transformStmt`
calls `transformWhereClause`. The placements come from measurement, not reasoning, and
they don't agree with each other:

- **`JOIN/ON` outranks the target list.** `transformFromClause` runs first, so
  `SELECT nosuchcol FROM a JOIN c ON a.k` is `42804`, where the `ON a.b` spelling is
  `42703`.
- **`HAVING` doesn't.** 18.6's order is FROM → target list → `WHERE` → `HAVING`, so
  `SELECT nosuchcol FROM t HAVING max(k)` is `42703`. pglet's `HAVING` walk sits below the
  projection and the `WHERE`, instead of the gate being placed apart from it. That also
  gives `SELECT nosuchcol FROM t HAVING max(nosuch2) > 0` 18.6's `nosuchcol` error, and
  `… GROUP BY k HAVING b LIKE 'x'` 18.6's `42703` instead of `42883`. Nothing between the
  two positions reads `having` except to check whether it's empty.
- **`UPDATE`/`DELETE`'s `WHERE` outranks the `SET` list and `RETURNING`**, following
  `transformUpdateStmt`'s clause order, and pglet's lowering does the same.
- **A partial index's predicate outranks its own key columns *and* its access method**,
  because 18.6 analyses the whole `IndexStmt` in `transformIndexStmt` before `DefineIndex`
  resolves a bare key column or an opclass. `validate_index_exprs` runs before
  `validate_columns_exist` and `validate_index_method` to match, so
  `CREATE INDEX z ON t (nosuchcol) WHERE k` is 18.6's `42804` and
  `CREATE INDEX z ON t USING gin (intcol) WHERE txt LIKE 1` its `42883`, not the later
  gates' `42703` and `0A000`.

  A **mixed key** is the residual that order leaves:
  `CREATE INDEX z ON t (nosuchcol, (txt LIKE 1))` is `42883` on 18.6 and
  `42703 column "nosuchcol" does not exist` here. A mixed key folds to `IndexKey::Exprs`,
  and `index_stored_exprs` hands the binding check *every* part of one, bare-column parts
  included, where `transformIndexStmt` only transforms an `IndexElem` that has an `expr`
  and leaves a bare name to `DefineIndex`. Narrowing that list would move a second gate's
  position, which is a separate decision. `executor/ddl.rs`'s `execute_create_index` says
  the same thing at the call site.
- **The `ON CONFLICT … DO UPDATE … WHERE`** is `"WHERE"` too, and it comes after the
  action's own `SET` list.
- **The arbiter's inference `WHERE` isn't a boolean position at all.**
  `transformOnConflictArbiter` uses a bare `transformExpr` with no boolean coercion, so
  `ON CONFLICT (u) WHERE k DO NOTHING` is accepted on 18.6 (measured), and there's no gate
  there.

The **message** is 18.6's text after `Error::Type`'s own `type error: ` prefix, so it's
byte-equal from `argument of` onward but not before. That's why the two clause-order pairs
that would otherwise need `@expect-message` to tell a `WHERE` from a `HAVING` are written
**cross-code** instead, and why nothing in this family uses that directive. Every `42804`
raised through `Error::Type` has that prefix (the binary-array header mismatch,
`Error::BinaryArrayElementTypeMismatch`, is a `42804` without it).

A gate the statement never reaches answers nothing, which is the same call-site rule the
duplicate-name gates follow ([names.md](names.md) § What the refusals do not carry): a
target the catalog lacks is `42P01` on both engines wherever the gate would have been.

A gate the statement **does** reach can answer before a token 18.6 never gets as far as
analysing. Parse and plan are fused here, so a boolean gate's verdict lands while the
statement's trailing junk is still unread, where 18.6's raw-parse phase refuses the whole
statement first: `SELECT k FROM t WHERE k JUNK` is this gate's `42804` against 18.6's
`42601`. It isn't the clause root that causes this. The node gates run in
`check_expr_semantics`, which the lowering reaches at positions no clause root covers, so
`SELECT NOT k FROM t x JUNK` (with no clause root in it at all) is `42804` the same way.
The gates are members of that class, not its cause. Its measured members, its non-members
and the other readers that reach it are in `EDGES.md` row 128.

### A stored predicate, and what its acceptance persisted

A `CHECK` and a partial index predicate take the root gate at DDL time, after
`validate_stored_expr`'s node walk and probe. That's 18.6's own order, since
`coerce_to_boolean` is the last thing `cookConstraint` does to the expression. It's also
the order that keeps the probe's recorded `0A000` shapes answering as they do, instead of
being pre-empted by a `42804` on a result type 18.6 never reaches. Without the gate, a
predicate accepted here is one stock Postgres refuses to load, which breaks the invariant
a persisted expression relies on ([sequences.md](sequences.md) § Recording a persisted
cross-object reference). The gate is on **new** DDL. A predicate a store already holds
(persisted by a build without the gate) is decoded straight from the store and isn't
re-checked, so such a database answers `42804` at the writes the predicate guards instead
of admitting the row.

`enforce_checks` is the finalizer they all go through, which makes the answer belong to
the predicate and not to any one statement. Its callers go beyond the `INSERT` and the
`UPDATE` to a `COPY FROM` row, an `ON CONFLICT DO UPDATE`'s post-update image, and the
foreign-key cascade images. Re-derive that caller list instead of trusting it. The
`ALTER TABLE` scans that also call `enforce_checks` are outside the set: each validates
the *statement's own* new predicate over existing rows, never a predicate the table
already had. This is loud, not silent, and recorded, not migrated.
`boolean_context_verdicts.rs` plants such a catalog directly, which no fixture can do.

### The input function is a prefix grammar

`boolin` and `parse_bool_with_len` (`src/backend/utils/adt/bool.c`) accept any
case-insensitive **non-empty prefix** of `true` / `false` / `yes` / `no`. `on`/`off`
compare over `max(len, 2)` characters, so `'of'` is FALSE and a bare `'o'` is `22P02`. `1`
and `0` are accepted at length **exactly** one, which is what refuses `'01'`. pglet's
`parse_bool` is the same prefix grammar, so `'tr'` · `'fals'` · `'ye'` · `'of'` are values
on both engines, where a fixed list of spellings would refuse them with `22P02`.

The trim set is C `isspace` in the `C` locale: space, `\t`, `\n`, `\v`, `\f`, `\r`.
`str::trim` strips every Unicode `White_Space` code point, so it would trim a no-break
space that 18.6 keeps. `u8::is_ascii_whitespace` is the same set minus `\v`, so it isn't
right either. The message names the **untrimmed** input, as `boolin`'s `errmsg` does.
`\v`, `\x0B`, `\013` and `\u000B` each decode to U+000B on both engines, so the `\v` part
of the trim set can be tested differentially with any of them. The trim fixture spells it
`\x0B`.

### Two things this rule is not

It's **not** a constant-folding pass. 18.6 stores a stored expression after parse
analysis, so over `CHECK ('t')`, `CHECK ('f')` and the two index predicates spelled the
same way, `pg_get_constraintdef` reads back `CHECK (true)` / `CHECK (false)`,
`information_schema.check_constraints.check_clause` reads `true` / `false`, and `indexdef`
has `WHERE false` for the second while **dropping** the first's `WHERE` entirely. pglet
renders each as written. That's a separate mechanism, the `Catalog` `RenderMode`
([deparse.md](deparse.md)), independent of this gate. The literal half is accepted at DDL
time with the gate and without it, and all four renderings are **byte-identical** either
way. The gate decides what those constraints *do*, not how they read.

And it says nothing about **which** index answers a query. A clause root that answers at
its statement's own lowering answers above index selection, so the two build profiles
reach the same verdict there. We checked this at the top level and at an uncorrelated
nested level, over the same `AND` and the same index that make the deferring spelling
below diverge. And the GIN route's two readers of one predicate, the scan-independent
guard and the recheck `Filter`, follow the rule together, and not for symmetry's sake. A
row admitted only by the guard matches no GIN probe and never reaches the `Filter`, so
widening only the `Filter` silently loses rows, on the accelerated profile only
([indexes.md](indexes.md) § Build profiles).

**Where a clause root *defers*, the plan answers instead, and there the two profiles
disagree.** A correlated reference types as `None`, so a nested level's
`check_boolean_clause` judges nothing. The walk that does judge it is
`planner/correlation.rs`'s `walk_level`, which runs *after* `plan_filter_or_index_scan`
has lifted a conjunct into a probe. With one `CREATE INDEX` and nothing else changed, the
accelerated build reports `argument of WHERE` where the stripped build and 18.6 report
`argument of AND` (`EDGES.md` row 126). The statement is refused either way under the same
SQLSTATE, so what the index changes is the **message**, not a result. That same deferral
also puts a correlated nested level on the non-member side of `EDGES.md` row 128: a clause
root that types nothing raises nothing, so trailing junk reaches the end-of-input check
and answers `42601`, where the uncorrelated spelling of the same statement gives this
gate's `42804`.


## Which coercion runs at DDL time is a precise rule

Only the target type's **input function on an unknown literal** runs in parse analysis. So
`DEFAULT 'x'` into `INT` is `22P02` at `CREATE TABLE` and `'notats'` into `TIMESTAMP` is
`22007`, while `1/0` (`22012`), `2147483648` into `INT` (`22003`), `'abcdef'` into
`VARCHAR(3)` (`22001`) and `12345.6` into `NUMERIC(4,2)` (`22003`) are all **accepted**
and raise per row. The length and precision typmods are applied after the input function,
not by it. Each half is pinned, and the accepting half has to be a **success** fixture,
not an `@expect-error` one, for the same same-code reason the cross-code precedence pairs
have ([ddl.md](ddl.md) § A column `DEFAULT` is an expression the executor cooks). pglet
reaches the input function by **evaluating** the default, which only reaches the branches
evaluation takes. So a cast over an unknown literal in a branch it doesn't take
(`CASE WHEN false THEN 'x'::int ELSE 1 END`) is accepted here and refused at DDL on 18.6
(`EDGES.md` row 47).

**The split is "resolution vs value", not "coercion vs the rest".** The narrower reading
produces two shapes (we tested both) that are accepted and then break the table:
`DEFAULT 5 + 'x'` and `DEFAULT 1` into a `BOOL` column. Each is accepted and then fails
*every* omitted-column `INSERT` (`42804` / `42846`), leaving a column that's defined but
unwritable, which is exactly what 18.6's DDL refusal prevents. The cook propagates the
type-resolution codes along with the input-function ones.

**The cook's assignment cast reads the expression's *declared type*, because a value
coercion alone answers the mismatch position four different ways.** 18.6's `cookDefault`
(`src/backend/catalog/heap.c` in the PostgreSQL source) runs `coerce_to_target_type` under
`COERCION_ASSIGNMENT` and raises one code for every bad pair:
`42804 column "x" is of type integer but default expression is of type text`. A *value*
coercion answers `42846` (a typed value with no cast), `22P02` (a `text`-typed value whose
text the target's input function rejects), `42804` (the explicit-only pairs
`reject_explicit_only_assignment` lists), and **nothing at all** where the text happens to
parse, as at `int DEFAULT '5'::text`. `planner::coercion::check_assignment` runs at the
cook, which is where 18.6 runs it too. Sweeps of column types × defaults show the
remaining divergences from 18.6 on the `CREATE TABLE` and `ADD COLUMN` routes, a few of
them specific to `ADD COLUMN`. The `ADD COLUMN` and `ALTER COLUMN … SET DEFAULT` routes
agree at every case of the smaller matrices run over both, which bounds what the matrices
covered, not what the routes do. `EDGES.md` row 47 records the cases.

**An `INSERT … SELECT` source through a wrapper follows the claimant rule** (§ `unknown`
is a shape, and what resolves it is a claimant set). The source-type walk descends the
nodes that resolve nothing and marks per position at the nodes that do. So
`SELECT 't' LIMIT 1` / `GROUP BY <a source column>` / `HAVING` / a window are accepted,
and `ORDER BY 1` / `GROUP BY 1` / `DISTINCT` / a set operation / a derived table are
`42804`, each matching 18.6 and each pinned. That includes the **sort-below-projection**
layout, where the `Sort` sits under the `Project` and the positions are read from
`Plan::ResolvedTargets` instead of from the rewritten key.

## `unknown` is a shape, and what resolves it is a claimant set

There are three consequences worth keeping in mind, each measured. First, Postgres's
**`unknown`** has to be read off the expression's *shape*, not its inferred type. A bare
`'5'` and a `'5'::text` both infer `text` here, while 18.6 assigns the first into an `INT`
column and refuses the second. And the carve-out stops at the bare constant:
`CASE WHEN true THEN 't' ELSE 'f' END`, `COALESCE('5','6')` and `ARRAY['1','2']` each
resolve the unknown to `text` and are `42804` into a non-text column on both engines.

**But *what* resolves an unknown is a set of claimants, not a node kind.** For an
`INSERT … SELECT` source, it's tempting to judge the plan's immediate node and let
everything else fall back to `Plan::output_columns`, which substitutes `text`. Judged that
way, these spellings of `INSERT INTO t (b) SELECT 't' …` are refused, and 18.6 accepts
every one: `LIMIT 1`, `OFFSET 0`, `LIMIT ALL`, `FROM s ORDER BY n LIMIT 1`,
`FROM s GROUP BY n ORDER BY n`, `FROM s GROUP BY n ORDER BY count(*)`,
`DISTINCT ON (n) … FROM s`, and a window projection with `ORDER BY n`. (A bare
`GROUP BY n`, a `HAVING` and a bare window projection are accepted under that approach,
since each already has a target-list node on top.)

18.6's resolvers are `findTargetlistEntrySQL92`'s claimants (an `ORDER BY` / `GROUP BY` /
`DISTINCT ON` key naming an output entry, by position or by alias) and
`findTargetlistEntrySQL99`'s, the first entry a key *equals*. Those are every window
`PARTITION BY` / `ORDER BY` key
(`INSERT INTO t (b, n) SELECT 't', row_number() OVER (PARTITION BY 't') FROM s` is
`42804`), and a sort key that is the entry's own `$N`. Then there are the whole-list ones:
plain `DISTINCT`, a set operation, a derived table. A `LIMIT`, an `OFFSET`, a `HAVING`, a
`WHERE`, a `GROUP BY` or `ORDER BY` on a **source** column, and a `DISTINCT ON` over one
all resolve nothing. So `INSERT INTO t (b) SELECT 't' LIMIT 1` is ordinary SQL on 18.6,
while `… ORDER BY 1` is `42804`, the pair that separates them. `planner::dml`'s
`target_list_source_types` walks that, with a three-way answer (`Typed` / `Unknown` /
`Opaque`), because a `$N` and a column of a relation the catalog lacks have to survive a
resolving node unresolved. For the `$N` that's pglet's choice, not 18.6's: a claimed
parameter is resolved there too, so `INSERT INTO t (b) SELECT $1 FROM s ORDER BY 1` (`$1`
undeclared, bound `t`) is `42804` at `Parse` on 18.6 and writes its row here (`EDGES.md`
row 150).

**And it's per *position*, which the finished plan can't answer.**
`SELECT 't', 5 ORDER BY 2` is accepted because no key names the first entry. Both claiming
clauses rewrite the key into the entry it named, so over two *equal* entries the position
is lost: `SELECT 't', 't' FROM s GROUP BY 1` and its sort-below twin
`SELECT 't' AS p, 't' AS q FROM s ORDER BY p, n` are accepted into `(text, bool)` and
`42804` into `(bool, text)`. No comparison of the rewritten key against the entries gets
both right (marking every equal entry refuses the first, and marking the first writes the
row 18.6 refuses in the second). So the positions are recorded while they're still known,
in `Plan::ResolvedTargets` on `Plan::Sort` and `Plan::Aggregate`, written by `build_sort`
/ `resolve_group_by_keys`.

A `Sort` carries the **enclosing** target list's positions, because `build_sort` puts it
*under* the projection whenever a key reads a base column the output drops
(`ORDER BY 1, n`). A walk that reads only the projection's entries never sees that layout,
and so stores rows 18.6 refuses. That layout carries a naming key down as the entry's own
expression, and a bare integer literal is *also* how the plan spells a positional
reference. Without protection, `SELECT 't', 5 … ORDER BY 2, n` reads base column 5
(`42P10` over a narrow source), and `SELECT 2 AS z, b FROM s ORDER BY 1, a` comes back in
the wrong row order with no error on either engine. The carried entry is protected
(`carried_key`).

## Four statement-level contracts the gate's placement follows

Each is measured. The identity refusal is a rewriter check on 18.6, so it runs *after* the
whole `SET` list, not at the top of `lower_update`, where it would also beat the `WHERE`'s
and `RETURNING`'s errors. The `UPDATE` target-name check sits in the lowering next to the
type verdict, because `transformUpdateTargetList` interleaves them per target
(`SET b = 1, nosuch = 1` is `42804` and the mirror is `42703`).

**A relation the catalog lacks turns the gate off for the whole statement.** 18.6 resolves
the range table before it coerces anything, and pglet raises that `42P01` at *execution*,
after every plan-time verdict. *Which clauses* that covers depends on the statement kind,
not on a general rule. `transformUpdateStmt` runs FROM, `WHERE`, `RETURNING` and only then
the target list, so all three outrank the cast there. `transformInsertStmt` runs its row
source **first**, so `INSERT … VALUES (1) RETURNING (SELECT 1 FROM nosuchrel)` and its
`ON CONFLICT … DO UPDATE SET` twin are 18.6's `42804` and get no such skip. A gate keyed
on the `FROM` alone would let the other two clauses pre-empt `42P01` with a `42804`.

**The identity guards take a narrower half of that skip, and the line is drawn by when
pglet's own `42P01` fires, not by which clause 18.6 resolves first.** Only the `FROM` is a
plan node the executor runs whatever the row count. A `WHERE` predicate and a `RETURNING`
item are evaluated per target row, so over an empty table neither is reached.
`UPDATE gi SET id = 5 WHERE EXISTS (SELECT 1 FROM nosuchrel)` and its `RETURNING` spelling
both report `UPDATE 0` here against 18.6's `42P01`, while `… FROM nosuchrel` is `42P01`
here either way. Skipping on the per-row clauses is acceptable for the assignment cast
(the same statement with a well-typed right-hand side reports `UPDATE 0` too, a zero-row
hole of its own, which the gate can't close with the wrong code). It's not acceptable for
the identity guard, whose skip *removes a refusal the statement has*: those two spellings
are `428C9` without the skip. So the guard takes only the `FROM` half and prefers a loud
wrong code to a silent success. The cost is in the row-matching spellings, which answer
`428C9` where a wide skip would give 18.6's `42P01`.
