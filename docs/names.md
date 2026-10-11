# Object names — rendering, columns, constraints, indexes, sequences, row types

How pglet mints, disambiguates and refuses names: a relation's columns, constraints and
the indexes that enforce them, an index declared without a name, sequences, and a table's
row type. It also covers how pglet **renders** a name the catalog holds back into text
(§ A name written back out below). That section isn't limited to those kinds: it states
the rendering predicate and what an over-quote costs each kind of caller. The repo
`CLAUDE.md` summarizes these rules, and this file has the full statements. Identifier
case-folding, how SQL text becomes a name in the first place, is in
[parser.md](parser.md). The sequence *record* (start/counter/owner) and the name-literal
grammar are in [sequences.md](sequences.md).

## A relation's column names are a set, and the heap already owns some of them

A relation's column namespace has two rules, and pglet enforces both at the statement
that *adds* a name, not at a later statement that reads one. No two columns of one
relation share a name, and no column takes a name the heap itself owns: `ctid`, `xmin`,
`cmin`, `xmax`, `cmax`, `tableoid`. That's 18.6's `heap.c` `SysAtt[]`, in its own order.
`catalog::schema`'s `SYSTEM_COLUMN_NAMES` holds the list and `is_system_column_name` is
the predicate over it. `oid` is **not** on the list: it stopped being a per-row system
column, and it's an ordinary column name on both engines, tested at `CREATE TABLE` and at
`ALTER TABLE … ADD COLUMN`. Both refusals are `42701` with different messages (*column
"a" specified more than once* and *column name "ctid" conflicts with a system column
name*), so the order between them shows only in the message, which is what
`@expect-message` is for.

Both checks are plain byte comparisons, because folding has already happened
([parser.md](parser.md)). An unquoted `CTID` is refused and a quoted `"CTID"` is an
ordinary column, an unquoted `A` collides with `"a"`, and `"A"` beside `"a"` is two
columns. Each of these was checked on both engines.

**Each check runs where 18.6 runs it, not just somewhere early.** `CREATE TABLE` raises
the repeat from `MergeAttributes`, which walks `stmt->tableElts` after the parse-analysis
passes. It raises the system name one layer further in, from `CheckAttributeNamesTypes`
inside `heap_create_with_catalog`. So the repeat wins over the system name in either
written order: `CREATE TABLE d (ctid int, a int, a int)` is *specified more than once* on
18.6. **That's true even though the two checks are in the opposite order inside
`CheckAttributeNamesTypes` itself**, so reading that function's own order gets the pair
backwards. `ALTER TABLE … ADD COLUMN` calls one function,
`check_for_column_name_collision`. Its two arms are the user column's *already exists*
and the system name, which it answers for an `attnum <= 0`. Only the first arm can be
reached from an `IF NOT EXISTS` skip, so `ADD COLUMN IF NOT EXISTS ctid int` is `42701`
and not a no-op (tested). What each gate then loses to and beats is in [ddl.md](ddl.md)
§ Which error wins. The `CREATE TABLE` pair also has to run before `schema.create_table`.
That's the standing requirement of [ddl.md](ddl.md) § A failed DDL statement must leave
the catalog untouched, since a catalog write that fails afterwards leaves a durable
phantom.

**These gates guard the statements that add names; they don't check existing state.**
They leave the persisted shape alone, and nothing re-reads an existing relation's column
list, so a catalog written by a build without them can still hold a repeated name or a
system-owned one. That's the only reason the `ADD COLUMN` route's arm order can be
reached at all. The system-name check runs *before* the relation's own columns, matching
the arm order of 18.6's single lookup. So a legacy table that already holds a user column
named `ctid` answers *conflicts with a system column name* for a second `ctid`, not
*already exists*.

**These two are the only statements pglet has that add a column name. That's a fact
about `Plan`, not a claim that the list is complete.** 18.6 runs these checks wherever a
column name enters a relation's descriptor, and that includes statements pglet doesn't
parse: `ALTER TABLE … RENAME COLUMN`, `CREATE TABLE … AS`, a view's column list, a
composite type's. `Plan` has no variant that renames or adds a column for any of them.
The rename is `0A000` (a `Plan::AlterTableUnperformed`, refused once the relation is
found), `CREATE TYPE t AS (…)` is `0A000` at the parser, and there are no views and no
CTAS at all. `is_system_column_name`'s call sites are the two named above. So what would
make this list stale is a new `Plan` variant that puts a column name into a relation.
18.6 already says which kinds wouldn't want the system-column half:
`CheckAttributeNamesTypes` skips it for `RELKIND_VIEW` and `RELKIND_COMPOSITE_TYPE`,
neither of which has system attributes. The repeat half has no such exemption.

**`ALTER TABLE IF EXISTS` reaches the `ADD COLUMN` gates the same way the plain form
does.** Over a table that exists it performs the operation, so `ALTER TABLE IF EXISTS <a
table> ADD COLUMN ctid int` is `42701` *conflicts with a system column name*, and a
repeated name is `42701` *already exists*, on both engines (tested). Over a name no
relation holds it's 18.6's skip, and neither gate is reached, because 18.6 looks up the
relation before it analyses the operation (`EDGES.md` row 15 keeps the skip's
residuals).

## A statement's target column list is a set

The rule above is about the names a relation *has*. This one is about the names a
statement *assigns to*. A column is assigned at most once per statement that has a target
column list: an `INSERT`'s `(col, …)`, a `COPY`'s, an `UPDATE`'s `SET` list, and an
`ON CONFLICT DO UPDATE`'s `SET` list. That list is what `Plan` has, not a claim of
completeness. 18.6 runs the rule wherever a statement names its targets, and `MERGE`'s
actions go through the same rewriter call (`MERGE INTO … WHEN MATCHED THEN UPDATE SET b
= 1, b = 2` is `42601` there, tested). pglet doesn't parse `MERGE` at all.

There are two SQLSTATEs. The split comes from the *phase* 18.6 raises each one in, not
from a difference in the rule:

- The column lists are `42701 column "a" specified more than once`, raised during parse
  analysis by `checkInsertTargets` (`parser/parse_target.c`) for `INSERT` and
  `CopyGetAttnums` (`commands/copy.c`) for `COPY`. `CopyGetAttnums` serves both COPY
  directions, so `COPY t (a, a) TO STDOUT` is refused just like the `FROM STDIN` form.
- The `SET` lists are `42601 multiple assignments to same column "a"`, raised from the
  **rewriter** by `process_matched_tle` (`rewrite/rewriteHandler.c`). It merges a
  repeated assignment where it can and refuses it where it can't. pglet parses no
  assignment it could merge (`SET foo[2] = 42, foo[4] = 43` is a `42601` syntax error
  here), so every repeat is refused. The rule is *more than one assignment*, not
  *conflicting assignments*: `SET a = 1, a = 1` is refused too.

The check is keyed on the **resolved column**, as 18.6's is: a `Bitmapset` of attnums on
the `INSERT` route, and a `list_member_int` over collected attnums on the `COPY` one.
Keying on the resolved column means a name that never resolves never claims anything, so
`(zz, zz)` is the `42703`. (The quoted spelling doesn't come into it: the parser's fold
already settled that, and `("a", a)` is one column on both engines before either gate
sees it.) `claim_target_column` is that predicate.

**pglet uses one `Vec<usize>` where 18.6 uses two bitmapsets, and that works only because
of what this parser accepts.** `checkInsertTargets` keeps `wholecols` *and*
`partialcols`, and branches on the target's shape. A whole column is refused if its
attnum is in **either** set, and a partial one (`a[1]`) only if its attnum is in
`wholecols`, so two partial claims on one attnum are allowed on purpose. Tested over
`ar (a INT[], b INT)`: `INSERT INTO ar (a[1], a[2])` and `INSERT INTO ar (a[1], a[1])`
are accepted on 18.6, while `INSERT INTO ar (a, a[1])` and `INSERT INTO ar (a[1], a)` are
`42701`. Nothing diverges here because this parser reads no indirection target at all
(all four are `42601`). That's the same premise the `SET` bullet above relies on, and a
parser that gains `a[1]` targets would have to revisit both gates together.

The `SET` lists key on the folded name instead. That's the same key over the same
objects: a relation's column names are a set (above), so folded names and columns
correspond one to one, and the name was resolved against that set before the check runs.

### Where each gate runs, and why the families disagree about it

**The column lists interleave their two checks, one item at a time.** Both
`checkInsertTargets` and `CopyGetAttnums` resolve one name and test it against the
columns already claimed before reading the next. So a repeat at item 2 wins over an
unknown name at item 3, while an unknown name at item 1 wins over a repeat after it. On
18.6, `INSERT INTO t (a, a, zz)` and `COPY t (a, a, zz) TO STDOUT` are `42701`, and
`INSERT INTO t (zz, a, a)` and `COPY t (zz, a, a) TO STDOUT` are `42703`.

**A `SET` list doesn't.** `transformUpdateTargetList` resolves every target name during
parse analysis, and the duplicate check is a rewriter check a whole phase later, so the
unknown name wins from either position: `UPDATE t SET a = 1, a = 2, zz = 3` is `42703`,
and so is `SET zz = 1, a = 1, a = 2`. That statement is the sharpest control in the
family. One shared interleaved helper used for all of them would answer `42701` there,
which is wrong. So `claim_target_column` carries no order at all. Each caller's loop owns
the order (`resolve_insert_columns` for the `INSERT` routes, `executor::resolve_columns`
for `COPY`), and the `SET` routes use a separate predicate,
`refuse_duplicate_assignments`, over a whole list.

The phases then decide the rest. Each boundary was checked on 18.6:

- A column list resolves in parse analysis, which puts it **ahead of the row source's own
  work**: the arity comparison and every cell's conversion. So
  `INSERT INTO t (a, a) VALUES (1)`, `… VALUES (1, 'zz')` and `… SELECT 1` are all
  `42701`, not the arity error or the `22P02`. pglet resolves the list before the
  `INSERT … SELECT` arity comparison, so a merely *unknown* name wins over it too:
  `INSERT INTO t (a, zz) SELECT 1` is `42703` on both engines.
- It also puts it ahead of the `GENERATED ALWAYS` identity guard, a rewriter check:
  `INSERT INTO g (id, id) VALUES (1, 2)` is `42701`, not the guard's `428C9`.
- A `SET` list's rewriter position puts it after its statement's clauses (the `FROM`, the
  `WHERE`, the right-hand sides, `RETURNING`). It also comes after a correlated sub-plan
  inside one of them, whose own binding gate closes in a re-walk that runs after every
  clause is lowered and before this gate. `SET a = 1, a = 2 RETURNING (SELECT b FROM et
  ORDER BY zzz)` is `42703` on both engines, over a row and over none, and so are the
  `WHERE (SELECT b FROM et ORDER BY zzz) = 1` and `SET a = (SELECT b FROM et ORDER BY
  zzz), a = 2` forms. **That position orders the gate only against the errors pglet
  raises while it lowers the statement. Outside that set, the answer depends on whether
  the statement reaches the gate at all, which is a property of the call site, not of the
  error.** The call site reaches the gate only after every error the same lowering raises
  before it, only with the target relation in the catalog, and only when its own
  missing-relation skip doesn't fire. None of those conditions looks at what pglet
  would answer later, or at what 18.6 answers at all. § What the refusals do not carry
  has the tests. The gate runs before the identity guard next to it, because
  `rewriteTargetListIU` merges every target entry in its first loop and only walks the
  relation's identity columns in its second: `UPDATE g SET id = 1, id = 2` and
  `SET b = 1, b = 2, id = 3` are both `42601`.
- The `ON CONFLICT DO UPDATE` list is rewritten by that same function, in a separate call
  that `RewriteQuery` makes **after** the `INSERT`'s own target list, and unconditionally
  for a `DO UPDATE` action. Both parts matter. Unconditionally means a free key is
  refused just like a conflicting one: `INSERT INTO u VALUES (5, 1) ON CONFLICT (a) DO
  UPDATE SET b = 2, b = 3` is `42601`, not an **insert**, and that's what keeps the gate
  off the conflict path. After the `INSERT`'s list means whatever that list's own rewrite
  decides answers first. We tested that at the identity guard, where both engines agree:
  `INSERT INTO g (id, b) VALUES (1, 2) ON CONFLICT (id) DO UPDATE SET b = 1, b = 2` is
  `428C9` on both. **Which of the row source's errors pglet reaches before this gate is a
  separate question from where the gate sits**, and the identity guard isn't a sample of
  the answer. A cell's own conversion and a cell's own bare name are reached here only at
  execution, after every plan-time gate, and § What the refusals do not carry records
  what that costs. Since this is the rewriter, it also still comes after `RETURNING`, and
  before the arbiter inference 18.6 does later still in the *planner*: `ON CONFLICT (c)
  DO UPDATE SET b = 1, b = 2` over a `c` with no unique index is `42601`, not the
  arbiter's `42P10`.

### What the refusals do not carry

The `42701`'s **position** field is left empty. 18.6 points it at the repeat on the
`INSERT` route (`CopyGetAttnums` fills none on either COPY direction), but pglet's column
lists are `Vec<String>` with no spans, so there's no offset to compute. That's the
omission [errors.md](errors.md) requires for a field pglet can't fill truthfully, and the
same *no span reaches the gate* situation as the `PRIMARY KEY`/`UNIQUE` key-list repeat.
18.6 sends no position for the `42601` on either `SET` route, so that one agrees
outright.

A **missing relation** is the one case the `UPDATE` gate's wide skip gives back, and what
the skip leaves behind is silence. A `WHERE` or `RETURNING` naming a relation the catalog
lacks turns the gate off, and pglet raises its own `42P01` for that relation at
execution, per target row. So `UPDATE t SET a = 1, a = 2 WHERE EXISTS (SELECT 1 FROM
nosuchrel)` and its `RETURNING (SELECT 1 FROM nosuchrel)` form are `42P01` on both
engines over a matching row, and `UPDATE 0` here against `42P01` over none. In that
zero-row corner the rule goes quiet: a column is assigned twice and nothing says so. The
silence is the reachability hole `EDGES.md` row 29 records, not this gate's: the same
statement with a single assignment is just as silent. That's what makes the skip
acceptable, since what it declines to refuse is a statement that writes nothing. The
**right-hand sides** aren't covered by the skip, and there the gate wins instead:
`UPDATE t SET a = (SELECT 1 FROM nosuchrel), a = 2` is `42P01` on 18.6 and `42601` here,
at either row count. That's loud and wrong rather than silent, and no agreement elsewhere
pays for it.

**The skip belongs to that call site, not to the whole family**, which shows the outcome
is a property of the call site and not of the error. The `ON CONFLICT DO UPDATE` list's
call site has no such skip. It does share the *catalog* one, since both routes gate on
`lowers_clauses`, so `INSERT INTO nosuchtbl … ON CONFLICT (a) DO UPDATE SET b = 1, b = 2`
is `42P01` on both engines. But the clause that turns the `UPDATE`'s gate off leaves the
`ON CONFLICT` one running. `RETURNING (SELECT 1 FROM nosuchrel)` beside a repeat is
`42P01` on 18.6 in both statements. Here, the `UPDATE` form is pglet's own `42P01` over a
matching row and `UPDATE 0` over none, while `INSERT … ON CONFLICT (a) DO UPDATE SET
b = 1, b = 2 RETURNING (SELECT 1 FROM nosuchrel)` is `42601` at a conflicting key and a
free one alike. That statement's control without the repeat is `42P01` on both engines
at either key.

The same `42601` answers that statement's `WHERE EXISTS (SELECT 1 FROM nosuchrel)` form
on the action, but *its* repeat-free control doesn't agree, and what that control
measures isn't this gate. pglet's own `0A000 … requires execution context` answers at a
conflicting key. We tested it at the `WHERE`'s `EXISTS` over a relation the catalog lacks
and over one it holds (`EXISTS (SELECT 1 FROM u)`), at a scalar subquery in that same
`WHERE`, and at one in a right-hand side. That shows it's the gate answering, not the
lookup, and it's `EDGES.md` row 89's class reached through this clause. At a **free** key
the action isn't evaluated, so that control **inserts the row** where 18.6 is `42P01` at
parse: silent, and it writes. Neither answer comes from this gate (a build without the
gate gives both), and the silence is `EDGES.md` row 29's reachability hole again, reached
through an action no conflict evaluates. **Not every subquery inside the action reaches
that `0A000`.** One the statement's own lowering refuses first answers at **either** key
instead: `SET b = 1 WHERE (SELECT b FROM et ORDER BY zzz) = 1`, `SET b = (SELECT b FROM
et ORDER BY zzz)` and `SET b = 1 WHERE EXISTS (SELECT nosuchcol FROM u)` are `42703` on
both engines at a conflicting key and a free one alike.

The residuals below are loud. The last one diverges from an error pglet raises
**nowhere**. The `ON CONFLICT` action's *own* identity guard runs inside the clause's
lowering here and after the merge on 18.6, so `DO UPDATE SET id = 1, id = 2` over a
`GENERATED ALWAYS` column is `428C9` here against `42601` there. That's the identity
guards' known position, not this gate's.

The `INSERT … SELECT` **source query** is the clause boundary the column list doesn't get
ahead of. 18.6 runs `checkInsertTargets` before the source's own `transformStmt`. pglet
lowers the source `SELECT` first, so the source's error wins: `INSERT INTO t (a, a)
SELECT 1, nosuchcol FROM t` is `42701` there and `42703` here, `… SELECT COALESCE(1,
'x'::text), 2` is `42701` there and `42804` here, and `… SELECT 1, 2 ORDER BY nosuchcol`
is `42701` there and `42703` here. It's a wrong code, loud on both sides. The boundary is
the source *query*, not its relations: a relation the catalog lacks is pglet's `42P01` at
execution, after the list, so `INSERT INTO t (a, a) SELECT 1 FROM nosuchrel` is `42701`
on both.

**Where 18.6 decides a cell in parse analysis and pglet decides it at execution, the gate
answers first.** pglet reaches an implicit assignment conversion, and a `VALUES` cell's
bare name, only per row at execution, while 18.6 does both before the rewriter. So a
repeat in the `SET` list takes the statement instead. Tested over
`u (a INT UNIQUE, b INT, bo BOOL)` holding `(1, 1, true)`, at a free key and a
conflicting one alike: `INSERT INTO u VALUES (5, 'notanint', true) ON CONFLICT (a) DO
UPDATE SET b = 1, b = 2` is `22P02` on 18.6 and `42601` here, and its
`nosuchcol`-in-the-cell form is `42703` there and `42601` here. The `INSERT … SELECT`
route's cell cast gives that first pair at either row count, and the `UPDATE` route has
it in its own right-hand side (`UPDATE t SET a = 1, a = 'notanint'`).

**The `ON CONFLICT` action's `WHERE` belongs to this same class, not a second one.** The
boolean-context gate ([coercion.md](coercion.md) § A boolean context is that same
conversion) sits at the same place in the clause's lowering, and the same call-site
property decides what it pre-empts. Tested over `bt (k INT, u INT UNIQUE)`:
`INSERT INTO bt (u) VALUES ('zz') ON CONFLICT (u) DO UPDATE SET k = 1 WHERE bt.k` is
`22P02` on 18.6 and `42804` here, and its `nosuchcol`-in-the-cell form is `42703` there
and `42804` here. Those are the two cases above with the boolean gate's code in place of
the `SET` list's. Everything else about them is the same: both engines error and the
SQLSTATEs differ, so no fixture can hold either; without the gate the engines agree; and
the repeat-free control agrees. The **arbiter**'s `WHERE` isn't in this class, because on
18.6 it isn't a boolean position at all.

**Without this gate pglet answers those as 18.6 does, wherever a row is evaluated.** It
gives `22P02` where the cell is an ill-typed constant and `42703` where it's an unknown
name, at both keys, and `22P02` at the one-row `INSERT … SELECT` and `UPDATE` forms,
while it silently accepts their zero-row forms (row 5's timing). So the gate costs an
agreement here rather than exposing a divergence. The control is the same statement with
the repeat removed, which gives 18.6's own code on both engines wherever a row is
evaluated: `insert/on_conflict_single_set_target_yields_to_the_cell_cast_loud.sql` and
its `_name_` sibling. A fixture can't express the divergent cases themselves. Both
engines error with different SQLSTATEs, which `@expect-error` scores red, and since pglet
errors, a value fixture is ruled out too.

**Moving the gate after those errors is ruled out, for a reason we tested.** 18.6 raises
the `SET` list's `42601` from the rewriter, before any row is evaluated, so a row source
yielding **zero rows** is refused too: `INSERT INTO u SELECT a, b FROM src WHERE false ON
CONFLICT (a) DO UPDATE SET b = 1, b = 2` is `42601` on 18.6 and here, while a build
without the gate reports `INSERT 0 0`. pglet's cell errors are per row, so a gate placed
after them wouldn't fire on that statement at all. Closing the residual that way would
fix the cases above and lose this agreement.
`insert/on_conflict_duplicate_set_target_over_a_zero_row_source_loud.sql` pins it, so a
later change that moves the gate goes red instead of finding this out later. Closing the
residual *properly* means moving the cell errors themselves to plan time: `EDGES.md`
row 5's constant-conversion timing and row 13's statement-level error order, neither of
which belongs to this gate.

**The two engines don't always read the same set, and these gates aren't the cause.**
18.6 truncates an identifier to `NAMEDATALEN - 1` = 63 bytes where the parser reads it,
so two 64-byte names that differ only in their last byte are **one** column there and two
unresolvable names here. Tested over `tt (<63 × 'c'> INT, x INT)` with those two names:
18.6 answers `42701` for the `INSERT` column list, `42601` for the `SET` list and `42701`
for `COPY`, while all three are `42703` here. With a single name, 18.6 truncates it (a
`42622` **notice**, not an error) where pglet gives `42703`. This belongs to the parser,
not this family. But it decides whether a target column list is a set of the *same*
things on both engines, so it's this section's residual as much as
[parser.md](parser.md)'s parse-level one.

**And where pglet raises an error *nowhere*, the gate has nothing to be ordered
against.** A non-boolean `WHERE` is `42804` on 18.6 and is accepted here:
`UPDATE t SET a = 1 WHERE a` runs and matches no row, tested over a row holding `9` and
over one holding `0` alike. So `UPDATE t SET a = 1, a = 2 WHERE a` is `42804` there and
`42601` here, and the fix belongs to the acceptance, not to this gate's position. An
aggregate or a window function in that same clause is in between: pglet answers its own
`0A000` at execution over a matching row and nothing at all over none, so the repeat's
`42601` comes before 18.6's `42803` / `42P20` at either row count. Read both as
**examples** of the ordering rule above, not as its full list. The set of things 18.6
decides in parse analysis and pglet decides somewhere other than its lowering isn't
listed here, and no bound is claimed for it.

## A name written back out

Folding is one direction. The other is rendering a name the catalog holds back into
text. The shared implementation is `lexer::quote_ident`, which follows pg_dump's `fmtId`
and quotes only when needed, with `is_safe_bare_ident` as its predicate. It's shared
rather than local to one caller because its callers aren't all the same kind of thing. A
second renderer is used where the string is a value a client reads and 18.6's own
function defines the rule: `lexer::pg_quote::quote_identifier`, which is 18.6's
`quote_identifier` over 18.6's keyword table. It's behind the SQL function `quote_ident`
and `pg_get_serial_sequence`'s result (`functions/quote_ident_every_keyword.sql` compares
it over every 18.6 keyword). **What an over-quote costs depends on the caller, not on the
predicate**, and the four kinds of caller don't all pay the same:

- **SQL text a reload must accept**: the statements `pglet::dump` emits. The call sites
  are the real list, and a list here would go stale. Over-quoting is **free**: both
  engines take `"action"` wherever they take `action`. Under-quoting is the defect here,
  and it doesn't cost one fixed thing.
  `CREATE INDEX Odd Ix ON public.Odd Tbl USING btree (a)`, the string the catalog
  rendering below produces, is `42601` on each engine, while the same rendering over a
  `"OddTbl"` parses on each and builds the index on the folded `oddtbl` instead (tested;
  `EDGES.md` row 122 has the per-case table). One dump position doesn't use this
  predicate. A **type** name, at `CREATE TYPE` and at an enum column's type, goes through
  `deparse::bare_type_ident`, because pglet's type-name grammar reads a bare identifier
  and rejects a quoted one. So quoting isn't free there, and the emitter *refuses* an
  unsafe name instead of writing text its own parser would reject.
- **A name written inside a string literal**: `nextval('<n>'::regclass)`,
  `setval('<n>', …)`. Same approach, same cost, one level further in. The full statement
  is [sequences.md](sequences.md) § A sequence's name reaches that script in two
  grammars.
- **Catalog text a client reads**: `pg_indexes.indexdef`, `pg_get_constraintdef`,
  `information_schema.check_constraints.check_clause`. pglet re-parses none of it, and a
  schema differ compares it as text. So an over-quote isn't free here. It's a **wrong
  value**, tested over a column named `action`: `indexdef` reads `(parent_id, "action")`
  here against 18.6's bare `(parent_id, action)`, and carries the same quoting into an
  expression key and a partial predicate (`(("action" + 1)) WHERE ("action" > 5)` against
  `((action + 1)) WHERE (action > 5)`). `pg_get_constraintdef` reads
  `CHECK (("action" > 0))` against `CHECK ((action > 0))`, and `check_clause` reads
  `("action" > 0)` against `(action > 0)`. A column named `position` agrees on both,
  which makes the difference easy to miss: `position` is a `COL_NAME_KEYWORD`, which 18.6
  quotes too.
- **A value a client reads back**: `<expr>::regclass`. Same wrong-value cost. The client
  may pass the value to `'…'::regclass` again, and it resolves on whichever engine
  printed it, so the cost here is the different *spelling*, not a broken round trip.
  That's why row 85 (f) files it as a residual and not as a defect.

The two disagree on the **keyword half** of the predicate, in both directions. 18.6's
`quote_identifier` quotes a word whose `ScanKeywordCategories` entry isn't
`UNRESERVED_KEYWORD`, while `is_safe_bare_ident` asks whether *pglet's own lexer* reads
the word as an `Identifier`. Tested at one word each way through `::regclass`:
`'"action"'::regclass::text` is `"action"` here and `action` there, and
`'"numeric"'::regclass::text` is `numeric` here and `"numeric"` there. `EDGES.md` row 85
(f) owns that rule: its tested cases, its `pg_get_keywords()` sweep, and the argument for
moving the predicate to a keyword-category table instead of refining it. This section
deliberately has no list of cases of its own, since one would go stale against both the
oracle's keyword table and pglet's lexer.

**A caller can also skip the predicate entirely.** `pg_indexes.indexdef` sends its key
columns through `quote_ident`, then interpolates the index's own name and its table's
name raw. So there the names that diverge are exactly the ones **18.6's** predicate would quote,
not the ones this one would, and the bare rendering costs different things for different names: a parse error
for some, a *different* relation for others, and for others only the spelling
(`EDGES.md` row 122). The dump's `emit_create_index` builds the same statement and quotes
both (tested), which is why the dump invariant survives it. Two renderers of one
statement disagreeing is the pattern to check for whenever a name reaches text by a
second route.

## A constraint name lives in two spaces, and a generated one is disambiguated rather than refused

Postgres puts every constraint in `pg_constraint (conrelid, conname)`, which is **one**
unique index over CHECK, FOREIGN KEY, PRIMARY KEY and UNIQUE, so the kinds share a name
pool (`42710`). It also puts a PK/UNIQUE's *index* in the `pg_class` relation namespace
alongside tables and other indexes (`42P07`, checked first). Reading only half of that
rejects too little one way and too much the other. Per-kind pools let every cross-kind
duplicate through, while gating the *auto*-generated name breaks the ordinary
`CREATE TABLE t (a INT UNIQUE, UNIQUE (a))`. `executor/ddl.rs`'s `ConstraintNames` is the
single gate that reads both spaces, as `catalog ∪ claimed-by-this-statement`, so a
`CREATE TABLE`'s in-flight names are visible to its next constraint (Postgres's
`CommandCounterIncrement`). The asymmetry is intended. A **declared** name is the user's
to get wrong. An **auto**-name is the engine's and gets a digit appended until it fits
(`ChooseRelationName`). Two specifications describing the same index are **merged**, not
rejected (`transformIndexConstraints`). The exception is two `PRIMARY KEY`s, which are
`42P16` even when identical, because the multiplicity check runs per constraint before
the merge loop. That last one matters beyond the error: `Schema::primary_key_index` is a
`find`, so a second primary key would make FK default-key resolution and `GROUP BY`
functional dependency depend on index-name order.

## A constraint's name is minted once, at execute time, and is the same fact everywhere

The declared/auto asymmetry above isn't specific to PK/UNIQUE. It's the rule for all four
kinds, and the way to get it wrong is to decide the name anywhere but the single namer.
Deciding it at **parse** time means declared-vs-auto can't be represented (a plain
`String` can't say "the engine chose this"), which breaks both halves at once: an FK's
auto-name gets gated where Postgres disambiguates, and a declared name on an inline
`REFERENCES` gets silently dropped. Deciding it **lazily at display time** is worse,
because each reader makes up its own. The violation message, `information_schema` and
the dump agree on nothing, `information_schema` can leave the constraint out entirely,
and the dump can emit two `ADD CONSTRAINT` statements under one name, which stock
Postgres and pglet's own restore both refuse.

So the split is by **side**, not by kind. The parse side says declared-or-nothing
(`planner::ForeignKeyDef`, `planner::CheckConstraintDef`). The catalog side is **total**
(`ForeignKey`, `CheckConstraint`, a plain `String`). `executor/ddl.rs`'s
`ConstraintNames` is the only bridge between them: one claimed-name pool with **two**
minters, so anyone re-deriving the mint sites has to walk both. `mint_constraint` covers
CHECK and FK, and is reached only through `mint_check` (`execute_create_table`,
`execute_alter_table_add_column`, `execute_alter_table_add_check`) and `mint_foreign_key`
(`execute_create_table`, `execute_alter_table_add_column`, `add_foreign_key`).
`mint_constraint_index` covers PK and UNIQUE, and its minted name is the index's too
(`execute_create_table`, `execute_alter_table_add_column`, `add_primary_key`,
`add_unique`). The minted name is **persisted**. Neither minter formats a name itself:
each passes the table's name, a second name and a label to the shared chooser (§ An
unnamed index's name below), which is where the name is cut to 63 bytes and where the
digit goes. The catalog name isn't an `Option` whose `None` means "reconstruct it". A
reconstruction can only search the table's own earlier checks, while the namer searches
the whole schema across all four kinds. So `CREATE TABLE q (a INT, CONSTRAINT r_a_check
CHECK (a > 0)); CREATE TABLE r (a INT, CHECK (a > 0));` would give `r_a_check1` from the
namer and `r_a_check` from the reconstruction: one database, two dumps, depending on
which build wrote it. A compatibility branch like that answers differently from the code
it replaces, so it isn't compatible.

The two sides search different scopes, and the asymmetry is deliberate (`pg_constraint.c`:
"deliberately not the same test"). A **declared** name is checked per relation
(`ConstraintNameIsUsed`), so the same `CONSTRAINT positive_amount` on two different
tables is ordinary, valid schema. That direction needs asserting as explicitly as the
other: widening the gate to the auto scope breaks ordinary hand-written DDL while turning
exactly one fixture red (`ddl/check_name_may_match_plain_index.sql`'s second table, which
is why its header says not to trim it). An **auto** name is disambiguated **schema-wide**
over all four kinds (`ChooseConstraintName`'s `(conname, connamespace)` scan), and reads
*only* `pg_constraint`. So a plain `CREATE INDEX t_a_fkey` doesn't push an FK's auto-name
along, while a PK/UNIQUE auto-name has to clear the relation namespace too, because its
index carries it (`ChooseRelationName`). Every write path mints through this gate:
`CREATE TABLE` (column- and table-level, all four kinds), `ALTER TABLE … ADD CONSTRAINT`,
`ADD CHECK`, and `ADD COLUMN`'s inline PK/UNIQUE/CHECK/REFERENCES.

Because the name is minted from a *position in a list*, the list's order is part of the
contract. A `CREATE TABLE`'s elements are processed in **written** order across the
column-level and table-level forms alike (Postgres walks `stmt->tableElts` once, routing
both onto the same `cxt.ck`/`ix`/`fkconstraints` lists). Two sinks concatenated after the
fact would pair a persisted auto-name with the wrong specification. A name-only query
can't see that, but it shows the moment the two constraints differ in anything else. And
*which kind* gets minted first differs by route. `CREATE TABLE` names its CHECKs, then
its indexes, then its FKs (`DefineRelation`), while `ADD COLUMN` names its **index
first**, then the CHECKs, then the FK. That second order is a *reschedule*, not the pass
numbering, and reading it off the enum gets it backwards: `ALTER TABLE`'s passes run in
ascending order, and `AT_PASS_ADD_CONSTR` (6) comes **before** `AT_PASS_ADD_INDEX` (9).
Nothing final happens in pass 6. `transformAlterTableStmt` schedules the inline `UNIQUE`
straight into pass 9 as an `AT_AddIndex`, while the CHECK and FK arrive in pass 6 as
`AT_AddConstraint`s that `ATParseTransformCmd` only re-files into
`AT_PASS_ADD_OTHERCONSTR` (10). Both orders are observable, and they point opposite ways
on the same column definition.

## The same mint rule governs a sequence's name, over a *different* namespace — and a sequence is a relation

A `SERIAL` / identity column's `<table>_<column>_seq` is the engine's name, so it's
disambiguated (`t_a_seq`, `t_a_seq1`, …) instead of refused, by the same
`ChooseRelationName` shape the constraint indexes use (and cut to 63 bytes as theirs is,
§ An unnamed index's name below). For the same reason, it can't be decided at parse time.
The parse side says *implicit-sequence* (`planner::ColumnDef.implicit_sequence`, with
`default: None`), and `executor/ddl.rs`'s `resolve_implicit_sequence_name` is the only
bridge, at every route (`CREATE TABLE`, `ADD COLUMN`, `ALTER COLUMN … ADD GENERATED`).
The user's own `DEFAULT nextval('x')` is never rewritten. **The declared half follows the
same rule.** An identity column's `SEQUENCE NAME <n>` is the *user's* name, so it's taken
verbatim and refused (`42P07`) when the relation namespace already holds it. Implementing
only the auto half and discarding the declared input leaves the rule half-true in a way
that bites. pg_dump emits `SEQUENCE NAME` on every identity column, and the re-minted
`<t>_<c>_seq` *coincides* with it in the default case. So the restore's `setval` names a
sequence the catalog lacks, the identity counter silently restarts at 1 on top of the
restored rows, and only a corpus using a non-default name can see it.

Three asymmetries matter here, and none of them is obvious. First, the candidate must
clear the **relation** namespace. Postgres's `pg_class` holds tables, indexes **and
sequences** in one pool, so `Schema::relation_name_taken` reads all three, and
`CREATE SEQUENCE ns1; CREATE TABLE ns1 (…)` is `42P07` in either order. Second, it must
**not** consult `pg_constraint`, because `transformColumnDefinition` passes
`isconstraint = false`: a CHECK named `t_a_seq` leaves the column's sequence on
`t_a_seq`. Third, the *order* against the constraint indexes is observable. The serial
sequences go on `transformCreateStmt`'s **before**-list, so they're created ahead of the
heap and its indexes. `CREATE TABLE b1 (a SERIAL, CONSTRAINT b1_a_seq PRIMARY KEY (a))`
is `42P07` because the sequence took the name first, and the names minted for them are
seeded into `ConstraintNames`. That before-list also puts the implicit sequences ahead of
the table's **own** `42P07` and after parse analysis. So a colliding declared name beats
an existing table of the statement's name and a duplicate CHECK name, and loses to
`IF NOT EXISTS`, to the column type (`42704`) and to the key columns (`42703`). Those are
four more positions in the same contract, each tested.

**The exception is when the name collided with is the table's own.** That's a fifth
position, and none of the other four predicts it. The sequence is created successfully
and `heap_create_with_catalog` is what finds the clash, so the error is the *table's*
`42P07` (`CREATE TABLE a1 (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME a1))`,
`IF NOT EXISTS` included, since its skip only fires when the table already exists). A
later column's collision with an already-taken name still wins over it. That one can't be
gated in the naming loop at all, which is the reason [ddl.md](ddl.md) § *A gate that
reads the catalog is blind inside the statement that is building it* generalizes. The
auto-name, though, reads the **committed catalog only** and never the claimed set. That's
the one place it differs from `ConstraintNames`' `catalog ∪ claimed`.
`ChooseRelationName` runs while `transformCreateStmt` is still *appending* to the
before-list, so there's nothing there to see yet. `CREATE TABLE zf (b INT GENERATED …
(SEQUENCE NAME zf_a_seq), a SERIAL)` is `42P07` on both engines, where consulting the
claimed set would silently give the SERIAL `zf_a_seq1`.

`ALTER SEQUENCE`'s own precedence is a third instance of the error-order contract
([ddl.md](ddl.md) § *Which error wins*), and it doesn't follow from the others.
`validate_relation_kind` runs **before** the `IF EXISTS` skip, so
`ALTER SEQUENCE IF EXISTS <a table>` is `42809` and not a no-op, while a name no relation
holds is `42P01`. Both win over the `OWNED BY` column's `42703`. On `CREATE SEQUENCE` the
same three run the other way round: the `IF NOT EXISTS` skip first (so a bad `OWNED BY`
is never even resolved), then `42P07`, then the owner. And the kind check has to be asked
of **every operand**, not once per statement. One `ALTER SEQUENCE … OWNED BY t.c` has two
relation operands with two different wrong-kind *messages* (`validate_relation_kind`'s
`cannot open relation "x"` for the target, `process_owned_by`'s
`sequence cannot be owned by relation "x"` for the owner), and a third kind, a sequence,
is wrong for the owner but right for the target. The same `42809` reaches beyond either
statement: `nextval`/`currval`/`setval` all open their relation through `init_sequence`,
so a table name there is `42809`, not the `42P01` a lookup in the sequences map alone
would report.

**A `DEFAULT nextval('x')` naming a table is `42809` at DDL time in pglet, deliberately
stricter than Postgres**, which resolves `'x'::regclass` at parse analysis and defers the
kind check to the first read. pglet's `nextval` argument is a **name literal**, not a
`regclass` reference: `parser::ddl`'s `parse_nextval_arg` consumes the cast token without
resolving the type, and reads the string through `reduce_name_literal`. A DDL
`DEFAULT nextval('q1')` also *auto-creates* `q1` (tested: the column then draws `1`). So
deferring wouldn't defer the error. It would put one name in `tables` and `sequences` at
once, the state `relation_name_taken` declares impossible, and the database would dump to
a script with a `CREATE SEQUENCE q1` and a `CREATE TABLE q1` that neither engine reloads.
The invariant holds because every registration site checks it, not only
`CREATE SEQUENCE`.

Three consequences follow across the rest of the statement surface.

**The wrong kind wins over a *skip*, not just over an error.** Postgres decides the
relkind on a name that *resolves*, so only a name no relation holds reaches
`DropErrorMsgNonExistent`. `DROP TABLE IF EXISTS`, `DROP INDEX IF EXISTS` and
`CREATE INDEX IF NOT EXISTS <a taken name> ON` are `42809` over **any** wrong-kind
target, and `ALTER TABLE IF EXISTS <a sequence> ADD COLUMN …` is `42809` too: its skip,
like theirs, reads the whole relation namespace. Seen the other way round, these are the
*quiet* cases of the class: the statement reports success, and the next one to name that
relation insists it already exists. That's exactly the
contradiction the `42809` exists to remove, only silent.

**The kind is a property of the (found, expected) pair, so the check has to read the
whole relation namespace, not one map.** `pg_class` holds tables, indexes and sequences
in one pool, so *every* ordered pair of kinds can happen. A check written against
`schema.sequences` alone is right about a third of the class and silently wrong about the
rest. The `IF EXISTS` forms are where it goes silent: such a check lets
`DROP TABLE IF EXISTS <an index>` and `DROP INDEX IF EXISTS <a table>` skip and report
success, while the other index-side shapes answer `42P01`/`42704`. A list of the **bare**
forms misses exactly the silent ones. `Schema::relation_kind` is the single enumeration
and `Error::WrongObjectType` the single error, in both directions, so a fourth kind can't
be added to only half of it.

**The lookup that decides is the function's first one, not the shared one.** A gate
routed through `get_table_or_err` only reaches the call sites with no open-coded lookup
before it: an earlier `get_table` in the same function answers `42P01` first. When a gate
moves into a shared helper, the question isn't "does this function call the helper" but
"does anything resolve the relation *before* it".

## An unnamed index's name is chosen as a sequence's is, and both are cut to 63 bytes

`CREATE [UNIQUE] INDEX [CONCURRENTLY] ON t (…)` names no index, so the engine chooses the
name. It's the declared/auto asymmetry again. A declared index name is the user's, and a
taken one is `42P07` (or the `IF NOT EXISTS` skip). An absent one is chosen, and steps
around a taken name instead of failing on it. The parse side says declared-or-nothing
(`Plan::CreateIndex`'s `name` is an `Option`), and `execute_create_index` is the bridge.
It chooses the name at the point where it would test a declared one: after the gates that
come before that test (a missing table is `42P01` and a missing key column `42703`, as
with a declared name) and before the build. The chosen name is persisted, so the dump
writes the name the catalog holds. The grammar still requires the name after
`IF NOT EXISTS` (`gram.y`'s `IndexStmt` is two productions, with `opt_single_name` in the
plain one), so there's no unnamed index to skip over: `CREATE INDEX IF NOT EXISTS ON t
(a)` is `42601` on both engines.

The name is 18.6's `ChooseIndexName` for an index that isn't a constraint's, in three
steps that `executor::ddl`'s `choose_index_name` takes in 18.6's order:

- **Each key part gets a name** (`ChooseIndexColumnNames`). A bare column uses its own
  name. An expression gets the name a `SELECT` would give it as an output column
  (`FigureIndexColname`, here `Expr::output_name`): `lower(t)` is `lower`, `(a)::bigint`
  is `a`, `coalesce(a, 0)` is `coalesce`, and it's `expr` where a `SELECT` says
  `?column?`. A part whose name an earlier part of the same key already has gets a digit
  of its own: `(a, b, a, a)` is `a`, `b`, `a1`, `a2`, and two operator parts are `expr`,
  `expr1`. Where pglet names an output column differently from 18.6, the index name
  differs too (`EDGES.md` row 277's residual).
- **The parts are joined by `_`** and used as the second name beside the table's, under
  the label `idx`, for a `UNIQUE` index as well as a plain one. `key` is the label of a
  `UNIQUE` **constraint**'s index, and `CREATE UNIQUE INDEX` declares no constraint: on
  one table, `b INT UNIQUE` is `t_b_key` and `CREATE UNIQUE INDEX ON t (b)` is `t_b_idx`
  (tested).
- **`ChooseRelationName` finishes it**, with `isconstraint = false`. That's the routine a
  `SERIAL` or identity column's sequence name goes through, and one function here
  (`choose_relation_name`). It builds `<table>_<parts>_<label>` through `makeObjectName`,
  which cuts the result to `NAMEDATALEN - 1` = 63 bytes. The label and the two
  underscores are never cut. The two names share what's left, with the **longer** one
  giving up a byte at a time until they fit, and each is then clipped back to a character
  boundary (so a multibyte name can come out shorter than 63). A taken name puts a digit
  on the **label** (`idx1`, `idx2`) and the name is cut again around it, so a stepped
  name is no longer than an unstepped one. It steps around the relation namespace only
  (`Schema::relation_name_taken`: tables, indexes, sequences): a `CHECK` named `t_a_idx`
  and an enum named `t_a_idx` both leave the index on `t_a_idx` (tested), just as a
  `CHECK` named `t_a_seq` leaves a sequence on its name.

The name is chosen against the catalog as the statement finds it, and nothing is claimed
before the build. A build that fails (`23505` over duplicate rows, an expression key that
raises on a row) leaves the name free for the next statement, a rolled-back block gives
its names back, and dropping `t_a_idx` lets the next unnamed statement take it again
instead of counting up from the highest digit handed out.

**Every name the engine chooses goes through one cut, and every name it mints through one
digit loop.** `catalog::make_object_name` is the cut and `executor::ddl`'s `choose_name`
is the loop: build the candidate, ask the caller's namespace whether it's taken, put a
digit on the label and cut again. A sequence name goes through them exactly as an index
name does, and so does each constraint's. Over 40-byte table and column names,
`pg_get_serial_sequence` gives 18.6's 63-byte name, and two `SERIAL` columns whose names
collide after the cut get the `42P07` 18.6 gives. The constraint kinds differ in three
things, each matching 18.6:

- **The second name.** A `PRIMARY KEY` has none, whatever its columns (`<table>_pkey`,
  `ChooseIndexName`'s `primary` arm), and neither does a `CHECK` that reads no single
  column (`<table>_check`). `make_object_name` treats an empty second name as that absent
  one, with one underscore and the whole budget going to the table's name. A `UNIQUE`'s
  second name is its key columns joined by `_` in **key** order
  (`ChooseIndexNameAddition`), a `FOREIGN KEY`'s is its referencing columns the same way
  (`ChooseForeignKeyConstraintNameAddition`), and a single-column `CHECK`'s is that
  column.
- **The label** (`pkey`, `key`, `fkey`, `check`), which is never cut and is where the
  digit goes: `…_key1`, never `…_key` with a digit after the 63rd byte. So two names that
  differ in full but match once cut are told apart the same way as two that matched in
  full, and each still fits. That's what keeps the dump loadable: 18.6 truncates an
  identifier it reads to 63 bytes, so two such names written in full would arrive there
  as one.
- **The namespace the digit steps around**, which the cut doesn't change and which is
  stated above. A `PRIMARY KEY`'s and `UNIQUE`'s candidate clears the relation namespace
  and every constraint name in the schema (`ChooseRelationName`, `isconstraint = true`),
  and a `CHECK`'s and `FOREIGN KEY`'s clears the schema's constraint names only
  (`ChooseConstraintName`), each as `catalog ∪ claimed`.

A name that fits in 63 bytes isn't cut: it's `<table>_<second name>_<label>` in full.

**A `NOT NULL` is the one name that is rendered but never minted.** pglet stores a
`NOT NULL` as a column bit, so `catalog/constraints_view.rs` builds its `pg_constraint`
name when it reads the row. It uses the same cut, so `<table>_<column>_not_null` over
long names is 18.6's 63-byte name, but it has nowhere to keep a digit. 18.6 mints that
name through `ChooseConstraintName` like a `CHECK`'s, so there it steps around a name
already taken. Two `NOT NULL` columns whose names collide after the cut are `…_not_null`
and `…_not_null1` there, and the same name twice here. The same goes for a column `a` of
a table declaring `CONSTRAINT t_a_not_null CHECK (…)`, and for the second of two 60-byte
tables that differ in their last byte. Nothing mints, drops or dumps that name, so the
long-name cases only cost a catalog read: rows of one table sharing a `conname`, which a
join keyed on the name multiplies. The declared `t_a_not_null` case also dumps a `CHECK`
that 18.6 refuses (`EDGES.md` row 278's residual).

**The cut applies to newly chosen names, not to existing state, and it belongs to the
chooser, not the parser.** A store written by a build whose minters don't cut holds
full-length names, and they stay that way. Nothing re-derives a persisted name, so such a
store opens, enforces and reports each constraint under the long name, drops its table,
and dumps those names in full. And a **declared** name is taken as written at any length,
while 18.6 truncates every identifier to 63 bytes as it reads it ([parser.md](parser.md)
§ Known lexical / parse divergences). So two declared constraint names that share their
first 63 bytes are two constraints here and one there (the second is `42710`, or `42P07`
where both are a `UNIQUE`'s), and a dump holding such a pair still restores short into
18.6 (`EDGES.md` row 278's residual).

## A table's name is a type name too

18.6 gives every table a **row type** with the table's own name, so `pg_type` holds it
beside each enum. A new name is checked against that namespace by the routine that
creates the object, not by one shared "is this name free?" check.
`heap_create_with_catalog` checks it for every relation it creates, after the relation
name's own `42P07`, and a sequence is one of those even though it gets no row type. So a
table or a sequence over an enum's name is `42710 type "…" already exists`, with the hint
`A relation has an associated type of the same name, …`. `DefineEnum` checks it for an
enum, so an enum over a table's name is `42710` with no hint. `index_create` never
checks, and neither an index nor a sequence has a row type, so an index over an enum's
name and an enum over an index's or a sequence's name are all accepted. We tested this
with `reltype <> 0` over `pg_class`, which is true for a table and a view and false for
an index and a sequence.

`Schema::type_name_taken` is that namespace in pglet: the enums and the tables' names.
Two consequences aren't obvious. The **minted** `<table>_<column>_seq` of a `SERIAL` or
identity column is checked too, and is refused instead of disambiguated.
`ChooseRelationName` steps around `pg_class` only, so over an enum `h_a_seq` 18.6 answers
`CREATE TABLE h (a SERIAL)` with `42710` and never picks `h_a_seq1`
(`resolve_implicit_sequence_name`, at all three routes). And the `IF NOT EXISTS` skips of
`CREATE TABLE` and `CREATE SEQUENCE` read the relation namespace only, on both engines,
so over an enum they give the `42710`, not the skip. Where the table's own check sits was
tested from both sides: after the column types (`42704`) and the repeated column
(`42701`), and before a missing foreign-key parent (`42P01`), a constraint index name a
relation holds (`42P07`) and a default's cook (`22P02`).

The row type is also what a `DROP TYPE <a table>` finds:
`2BP01 cannot drop type t because table t requires it`, with `IF EXISTS` and `CASCADE`
included. It's found by the name as written, before the enum lookup's folded key, so
`DROP TYPE "Rd"` beside a table `"Rd"` and an enum `rd` names the row type, as on 18.6.
Getting this wrong breaks the dump invariant: a table and an enum with the same name dump
to a script whose `CREATE TABLE` 18.6 refuses, losing the table and its rows while `psql`
exits `0`. A store written by a build without these gates can still hold such a pair,
and it opens. Whichever of the two `CREATE`s a restore runs second is refused on both
engines, so the dump refuses that store (`0A000`, naming the table) until the pair is
separated. You can separate it by dropping the enum where no column declares it. Under a
column of that type, `DROP TYPE` is accepted here where 18.6 answers `2BP01`, and the
store then dumps short, silently (`EDGES.md` row 248).

The asymmetry leaves some pairs legal (an enum beside an index or a sequence with its
name), and of those, the sequence is the one that constrains the dump's order: both
engines accept the enum only once the sequence exists. So the dump emits every
`CREATE SEQUENCE` before every `CREATE TYPE`, the only order both reload. It refuses
(`0A000`) a database whose **identity** sequence shares an enum's name. That sequence is
created by an `ADD GENERATED` after its table, which may declare the enum, so the dump's
order doesn't reload it (`EDGES.md` row 248, which also keeps the residuals: a quoted
enum name against its folded key, and a row type pglet doesn't otherwise model).
