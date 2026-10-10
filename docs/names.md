# Object names — rendering, columns, constraints, indexes, sequences, row types

How pglet mints, disambiguates and refuses the names of a relation's columns, of
constraints, the indexes that enforce them, an index declared without a name, sequences and
a table's row type — and how
it **renders** a name the catalog holds back into text — § A name written back out below, which
is not scoped to those kinds: it states the one rendering predicate and what an
over-quote costs each class of caller. The rules are summarized in the repo
`CLAUDE.md`; this file carries the full statements. Identifier case-folding —
how SQL text becomes a name in the first place — lives in
[parser.md](parser.md); the sequence *record* (start/counter/owner) and the
name-literal grammar live in [sequences.md](sequences.md).

## A relation's column names are a set, and the heap already owns some of them

A relation's column namespace carries two rules, and pglet enforces both where a
statement *adds* a name to it rather than where a later statement reads one. No two
columns of one relation share a name, and no column takes one of the names the heap
itself owns: `ctid`, `xmin`, `cmin`, `xmax`, `cmax`, `tableoid` — 18.6's `heap.c`
`SysAtt[]`, in its own order, which `catalog::schema`'s `SYSTEM_COLUMN_NAMES` holds
and `is_system_column_name` is the one predicate over. `oid` is **not** one of them:
it stopped being a per-row system column, and is an ordinary column name on both
engines, measured at `CREATE TABLE` and at `ALTER TABLE … ADD COLUMN`. Both refusals
are `42701` under different sentences (*column "a" specified more than once* and
*column name "ctid" conflicts with a system column name*), so the order between them
is observable only through the message, which is what `@expect-message` exists for.

Both tests are plain byte comparisons, because the fold is already behind them
([parser.md](parser.md)): an unquoted `CTID` is refused and a quoted `"CTID"` is an
ordinary column, an unquoted `A` collides with `"a"`, and `"A"` beside `"a"` is two
columns — each measured on both engines.

**Where each runs is 18.6's position, not merely an early one.** `CREATE TABLE`
raises the repeat from `MergeAttributes`, which walks `stmt->tableElts` once the
parse-analysis passes are behind it, and the system name one layer further in, from
`CheckAttributeNamesTypes` inside `heap_create_with_catalog`. So the repeat outranks
the system name in either written order — `CREATE TABLE d (ctid int, a int, a int)`
is *specified more than once* on 18.6 — **even though the two tests sit the other way
up inside `CheckAttributeNamesTypes` itself**, which is why reading that function's
own order gets the pair backwards. `ALTER TABLE … ADD COLUMN` asks one function,
`check_for_column_name_collision`, whose two arms are the user column's *already
exists* and the system name it answers on an `attnum <= 0`; only the first is
reachable from an `IF NOT EXISTS` skip, so `ADD COLUMN IF NOT EXISTS ctid int` is
`42701` and not a no-op, measured. What each gate then loses to and beats is [ddl.md](ddl.md) § Which
error wins; that the `CREATE TABLE` pair also lands ahead of `schema.create_table` is
the standing requirement of [ddl.md](ddl.md) § A failed DDL statement must leave the
catalog untouched, since a catalog write that fails afterwards is a durable phantom.

**These gates close the producers, not the state.** They leave the persisted shape as
it is and nothing re-reads an existing relation's column list, so a catalog written by a
build without them can still hold a repeated name or a system-owned one. That is what makes
the `ADD COLUMN` route's arm order reachable at all: the system-name test runs *above*
the relation's own columns, matching the arm order of the one 18.6 lookup, so a legacy
table already holding a user column named `ctid` answers *conflicts with a system
column name* for a second `ctid` rather than *already exists*.

**The two enforcing statements are the two pglet has, which is a property of `Plan`
rather than a claim about completeness.** 18.6 runs these tests wherever a column
name enters a relation's descriptor, and that reaches statements pglet does not
parse: `ALTER TABLE … RENAME COLUMN`, `CREATE TABLE … AS`, a view's column list, a
composite type's. `Plan` carries a variant that renames or adds a column for none of
them — the rename is `0A000` (a `Plan::AlterTableUnperformed`, refused once the
relation is found), `CREATE TYPE t AS (…)` is `0A000` at the parser, and there is no
view and no CTAS at all
— and `is_system_column_name`'s call sites are the two named above. So what will
stale this enumeration is a new `Plan` variant that puts a column name into a
relation, and 18.6 already says which kind would not want the system-column half:
`CheckAttributeNamesTypes` skips it for `RELKIND_VIEW` and `RELKIND_COMPOSITE_TYPE`,
neither of which has system attributes. The repeat half has no such exemption.

**`ALTER TABLE IF EXISTS` reaches the `ADD COLUMN` gates as the plain spelling does.**
Over a table that exists it performs the operation, so `ALTER TABLE IF EXISTS <a table>
ADD COLUMN ctid int` is `42701` *conflicts with a system column name* and a repeated
name `42701` *already exists*, on both engines (measured); over a name no relation holds
it is 18.6's skip, and neither gate is reached — 18.6 looks the relation up before it
analyses the operation (`EDGES.md` row 15, which keeps the skip's residuals).

## A statement's target column list is a set

The rule above is about the names a relation *has*; this one is about the names a
statement *assigns to*. A column is assigned at most once per statement carrying a
target column list — an `INSERT`'s `(col, …)`, a `COPY`'s, an `UPDATE`'s `SET` list,
an `ON CONFLICT DO UPDATE`'s `SET` list. That enumeration is the one `Plan` has and not
a completeness claim: 18.6 runs the rule wherever a statement names its targets, and
`MERGE`'s actions go through the very same rewriter call
(`MERGE INTO … WHEN MATCHED THEN UPDATE SET b = 1, b = 2` is `42601` there, measured) —
pglet parses no `MERGE` at all.

Two SQLSTATEs, and the split is the *phase* 18.6 raises each from rather than a
difference in the invariant:

- The column lists are `42701 column "a" specified more than once`, raised during parse
  analysis — `checkInsertTargets` (`parser/parse_target.c`) for `INSERT`, `CopyGetAttnums`
  (`commands/copy.c`) for `COPY`. `CopyGetAttnums` serves both COPY directions, so
  `COPY t (a, a) TO STDOUT` is refused exactly as the `FROM STDIN` spelling is.
- The `SET` lists are `42601 multiple assignments to same column "a"`, raised from the
  **rewriter** — `process_matched_tle` (`rewrite/rewriteHandler.c`), which merges a
  repeated assignment where it can and refuses it where it cannot. pglet parses no
  assignment it could merge (`SET foo[2] = 42, foo[4] = 43` is a `42601` syntax error
  here), so every repeat is the refusal. The rule is *more than one assignment* and not
  *conflicting assignments*: `SET a = 1, a = 1` is refused too.

The claim is keyed on the **resolved column**, as 18.6's is — a `Bitmapset` of attnums on
the `INSERT` route, a `list_member_int` over collected attnums on the `COPY` one. What
that buys is not the quoted spelling, which the parser's fold already settled (`("a", a)`
is one column on both engines before either gate sees it); it is that a name which never
resolves never claims anything, so `(zz, zz)` is the `42703`. `claim_target_column` is
that one predicate.

**One `Vec<usize>` stands in for a pair of bitmapsets, and it does so on a premise this
parser supplies.** `checkInsertTargets` keeps `wholecols` *and* `partialcols`, and
branches on the target's shape: a whole column refuses an attnum in **either** set, a
partial one (`a[1]`) refuses only an attnum in `wholecols` — so two partial claims on one
attnum are deliberately allowed. Measured over `ar (a INT[], b INT)`:
`INSERT INTO ar (a[1], a[2])` and `INSERT INTO ar (a[1], a[1])` are accepted on 18.6,
while `INSERT INTO ar (a, a[1])` and
`INSERT INTO ar (a[1], a)` are `42701`. Nothing diverges here because this parser reads no
indirection target at all — all four are `42601` — which is the same premise the `SET`
route states in the bullet above, and it is what a parser gaining `a[1]` targets would
have to revisit at both gates together.

The `SET` lists key on the folded name instead, which is the same key
over the same objects: a relation's column names are a set (above), so the map from a
folded name to a column is injective, and the name was resolved against that set before
the check runs.

### Where each gate runs, and why the families disagree about it

**The column lists interleave their two verdicts, one item at a time.** Both
`checkInsertTargets` and `CopyGetAttnums` resolve one name and test it against the
columns already claimed before reading the next, so a repeat at item 2 outranks an
unknown name at item 3 while an unknown name at item 1 outranks a repeat behind it.
Measured on 18.6: `INSERT INTO t (a, a, zz)` and `COPY t (a, a, zz) TO STDOUT` are
`42701`, `INSERT INTO t (zz, a, a)` and `COPY t (zz, a, a) TO STDOUT` are `42703`.

**A `SET` list does not.** `transformUpdateTargetList` resolves every target name during
parse analysis and the duplicate check is a rewriter check a whole phase later, so the
unknown name wins from either position — `UPDATE t SET a = 1, a = 2, zz = 3` is `42703`,
and so is the `SET zz = 1, a = 1, a = 2` spelling. That statement is the sharpest control
in the family: one shared interleaved helper applied to all of them answers `42701` there
and is wrong. `claim_target_column` therefore carries no order at all: each caller's loop
owns it — `resolve_insert_columns` for the `INSERT` routes, `executor::resolve_columns`
for `COPY` — and the `SET` routes reach a separate predicate,
`refuse_duplicate_assignments`, over a whole list.

The phases then decide the rest, each boundary measured on 18.6:

- A column list resolving in parse analysis puts it **ahead of the row source's own
  work** — the arity comparison and every cell's conversion — so
  `INSERT INTO t (a, a) VALUES (1)`, `… VALUES (1, 'zz')` and `… SELECT 1` are all the
  `42701` rather than the arity error or the `22P02`. pglet's resolution sits above
  the `INSERT … SELECT` arity comparison, which carries a merely *unknown* name with
  it: `INSERT INTO t (a, zz) SELECT 1` is `42703` on both engines.
- It also puts it ahead of the `GENERATED ALWAYS` identity guard, a rewriter check:
  `INSERT INTO g (id, id) VALUES (1, 2)` is `42701`, not the guard's `428C9`.
- A `SET` list's rewriter position puts it behind its statement's clauses — the `FROM`,
  the `WHERE`, the right-hand sides, `RETURNING` — and behind a correlated sub-plan
  inside one of them, whose own binding gate closes in a re-walk that runs below every
  clause lowering and above this gate: `SET a = 1, a = 2 RETURNING (SELECT b FROM et
  ORDER BY zzz)` is `42703` on both engines, over a row and over none, as are its
  `WHERE (SELECT b FROM et ORDER BY zzz) = 1` and `SET a = (SELECT b FROM et ORDER BY
  zzz), a = 2` spellings. **That position orders the gate against the verdicts pglet
  reaches while it lowers, and against nothing else — and outside that set, what answers
  is decided by whether the statement reaches the gate at all, which is the call site's
  property and not the verdict's.** The call site reaches it only below every verdict the
  same lowering raises above it, only with the target relation in the catalog, and only
  with its own missing-relation skip left on; what pglet would answer later, and what 18.6
  answers at all, are read at none of them. § What the refusals do not carry has the
  measurements. It sits above the identity guard beside it,
  because `rewriteTargetListIU` merges every target entry in its first loop and only
  walks the relation's identity columns in its second: `UPDATE g SET id = 1, id = 2` and
  `SET b = 1, b = 2, id = 3` are both `42601`.
- The `ON CONFLICT DO UPDATE` list is rewritten by that same function, on a separate
  call `RewriteQuery` makes **after** the `INSERT`'s own target list and unconditionally
  for a `DO UPDATE` action. Both halves are load-bearing. Unconditionally, so a free key
  is refused as a conflicting one is — `INSERT INTO u VALUES (5, 1) ON CONFLICT (a) DO
  UPDATE SET b = 2, b = 3` is `42601` rather than an **insert**, which is what fixes the
  gate off the conflict path. After the `INSERT`'s list, so what that list's own rewrite
  decides answers first — measured at the identity guard, which is where both engines
  agree: `INSERT INTO g (id, b) VALUES (1, 2) ON CONFLICT (id) DO UPDATE SET b = 1,
  b = 2` is `428C9` on both. **Which of the row source's verdicts
  pglet reaches before this gate is a separate question from where the gate sits**, and
  the identity guard is not a sample of the answer: a cell's own conversion and a cell's
  own bare name are reached here only at execution, below every plan-time gate, and
  § What the refusals do not carry records what that costs. And being the rewriter it is
  still behind `RETURNING`,
  and ahead of the arbiter inference 18.6 does later still in the *planner*: `ON CONFLICT
  (c) DO UPDATE SET b = 1, b = 2` over a `c` with no unique index is `42601`, not the
  arbiter's `42P10`.

### What the refusals do not carry

The `42701`'s **position** is left unfilled. 18.6 points it at the repeat on the
`INSERT` route (`CopyGetAttnums` fills none on either COPY direction), and pglet's
column lists are `Vec<String>` with no spans, so there is no offset to compute from —
the omission [errors.md](errors.md) requires of a field it cannot fill truthfully, and
the same *no span reaches the gate* the `PRIMARY KEY`/`UNIQUE` key-list repeat gives.
18.6 sends no position for the `42601` on either `SET` route, so that one agrees
outright.

A **missing relation** is the one subject the `UPDATE` gate's wide skip hands back, and
what the skip leaves is a silence. A `WHERE` or `RETURNING` naming a relation the
catalog lacks turns the gate off, and pglet raises its own `42P01` for that relation at
execution, per target row — so `UPDATE t SET a = 1, a = 2 WHERE EXISTS (SELECT 1 FROM
nosuchrel)` and its `RETURNING (SELECT 1 FROM nosuchrel)` spelling are `42P01` on both
engines over a matching row, and `UPDATE 0` here against `42P01` over none. That
zero-row corner is where the invariant goes quiet — a column assigned twice, and nothing
says so. The silence is the reachability hole `EDGES.md` row 29 records rather than this
gate's: the same statement with a single assignment is exactly as silent. That is what
makes the skip payable — what it declines to refuse is a statement that writes nothing.
The **right-hand sides** are outside the skip, and there the gate wins instead: `UPDATE
t SET a = (SELECT 1 FROM nosuchrel), a = 2` is `42P01` on 18.6 and `42601` here, at
either row count — loud-wrong rather than silent, and with no agreement standing behind
it.

**The skip is that call site's and not the family's**, which is what shows the outcome to
be the call site's property rather than the verdict's. The `ON CONFLICT DO UPDATE` list's
call site carries no such skip — it does share the *catalog* one, both routes gating on
`lowers_clauses`, so `INSERT INTO nosuchtbl … ON CONFLICT (a) DO UPDATE SET b = 1, b = 2`
is `42P01` on both engines — but the clause that turns the `UPDATE`'s gate off leaves the
`ON CONFLICT` one running. `RETURNING (SELECT 1 FROM nosuchrel)` beside a repeat is
`42P01` on 18.6 under both statements; here the `UPDATE` spelling is pglet's own `42P01`
over a matching row and `UPDATE 0` over none, while `INSERT … ON CONFLICT (a) DO UPDATE
SET b = 1, b = 2 RETURNING (SELECT 1 FROM nosuchrel)` is `42601`, at a conflicting key and
a free one alike — and that statement's repeat-free control is the `42P01` on both engines
at either key. The same `42601` answers that statement's `WHERE EXISTS (SELECT 1 FROM
nosuchrel)` spelling on the action, but *its* repeat-free control does not agree, and what
that control measures is not this gate: pglet's own `0A000 … requires execution context`
answers at a conflicting key — measured at the `WHERE`'s `EXISTS` over a relation the
catalog lacks and over one it holds (`EXISTS (SELECT 1 FROM u)`), at a scalar subquery in
that same `WHERE`, and at one in a right-hand side — which identifies the gate rather than
the lookup, and is `EDGES.md` row 89's class reached through this clause. At a **free**
key the action is not evaluated, so that control **inserts the row** where 18.6 is `42P01`
at parse: silent, and it writes. Neither answer is this gate's — a build without the gate
gives both — and the silence is `EDGES.md` row 29's reachability hole again, reached through an
action no conflict evaluates. **Not every subquery inside the action reaches that
`0A000`**: one the statement's own lowering refuses first answers at **either** key
instead — `SET b = 1 WHERE (SELECT b FROM et ORDER BY zzz) = 1`, `SET b = (SELECT b FROM
et ORDER BY zzz)` and `SET b = 1 WHERE EXISTS (SELECT nosuchcol FROM u)` are `42703` on
both engines at a conflicting key and a free one alike.

The residuals below are loud — the last of them at a gate standing in front of nothing,
since the verdict it diverges from is one pglet raises **nowhere**. The `ON CONFLICT`
action's *own* identity guard runs inside the clause's lowering here and after the merge
on 18.6, so `DO UPDATE SET id = 1, id = 2` over a `GENERATED ALWAYS` column is `428C9`
here against `42601` there — the identity guards' known position rather than this gate's.

The `INSERT … SELECT` **source query** is the clause boundary the column list does not
clear. 18.6 runs `checkInsertTargets` ahead of the source's own `transformStmt`; pglet
lowers the source `SELECT` first, so the source's error wins — `INSERT INTO t (a, a)
SELECT 1, nosuchcol FROM t` is `42701` there and `42703` here, `… SELECT COALESCE(1,
'x'::text), 2` is `42701` there and `42804` here, and `… SELECT 1, 2 ORDER BY nosuchcol`
is `42701` there and `42703` here. A wrong code, loud on both sides. The boundary is the
source *query* and not its relations: a relation the catalog lacks is pglet's `42P01` at
execution, downstream of the list, so `INSERT INTO t (a, a) SELECT 1 FROM nosuchrel` is
`42701` on both.

**Where 18.6 decides a cell in parse analysis and pglet decides it at execution, the gate
answers first.** pglet reaches an implicit assignment conversion, and a `VALUES` cell's
bare name, only per row at execution, where 18.6 performs both ahead of the rewriter — so
a repeat in the `SET` list takes the statement instead. Measured over
`u (a INT UNIQUE, b INT, bo BOOL)` holding `(1, 1, true)`, at a free key and a conflicting
one alike: `INSERT INTO u VALUES (5, 'notanint', true) ON CONFLICT (a) DO UPDATE SET
b = 1, b = 2` is `22P02` on 18.6 and `42601` here, its `nosuchcol`-in-the-cell spelling is
`42703` there and `42601` here, the `INSERT … SELECT` route's cell cast answers that first
pair at either row count, and the `UPDATE` route carries it in its own right-hand side
(`UPDATE t SET a = 1, a = 'notanint'`).

**The `ON CONFLICT` action's `WHERE` is a member of this class rather than a second
one.** The boolean-context gate ([coercion.md](coercion.md) § A boolean context is that
same conversion) sits at the same place in the clause's lowering, and the same call-site
property decides what it pre-empts: measured over `bt (k INT, u INT UNIQUE)`,
`INSERT INTO bt (u) VALUES ('zz') ON CONFLICT (u) DO UPDATE SET k = 1 WHERE bt.k` is
`22P02` on 18.6 and `42804` here, and its `nosuchcol`-in-the-cell spelling is `42703`
there and `42804` here — the two members above with the boolean gate's code substituted
for the `SET` list's. Everything else about them is the same: both engines error, the
SQLSTATEs differ, so no fixture can hold either; without the gate the answer agrees on
both; and the repeat-free control agrees. The **arbiter**'s `WHERE` is not a member, being no
boolean position on 18.6 at all.

**Without this gate pglet answers those as 18.6 does, wherever a row is evaluated** —
`22P02` where the cell is an ill-typed constant and `42703` where it is an unknown name,
at both keys, and `22P02` at the one-row `INSERT … SELECT` and `UPDATE` spellings, while
it accepts their zero-row spellings silently (row 5's timing). So this is an agreement
the gate costs rather than a divergence it exposes. The control is the same statement with the
repeat removed, which answers 18.6's own code on both engines wherever a row is evaluated
— `insert/on_conflict_single_set_target_yields_to_the_cell_cast_loud.sql` and its `_name_`
sibling. A fixture cannot express the divergent members themselves: both engines error and
the SQLSTATEs differ, which `@expect-error` scores red, and pglet erroring rules out a
value fixture.

**Moving the gate below those verdicts is refused, for a measured reason.** 18.6 raises
the `SET` list's `42601` from the rewriter, before any row is evaluated, so a row source
yielding **zero rows** is refused too — `INSERT INTO u SELECT a, b FROM src WHERE false ON
CONFLICT (a) DO UPDATE SET b = 1, b = 2` is `42601` on 18.6 and here, where a build
without the gate reports `INSERT 0 0`. pglet's cell verdicts are per-row, so a gate placed
after them would not fire on that statement at all: closing the residual that way trades
the members above for this agreement.
`insert/on_conflict_duplicate_set_target_over_a_zero_row_source_loud.sql` pins it, so a
later mover reddens rather than discovers it. Closing the residual *properly* means moving
the cell verdicts themselves to plan time — `EDGES.md` row 5's constant-conversion timing
and row 13's statement-level error order, neither of them this gate's.

**The two engines do not always read the same set, and that is no doing of these
gates.** 18.6 truncates an identifier to `NAMEDATALEN - 1` = 63 bytes where the parser
reads it, so two 64-byte names differing only in their last byte are **one** column there
and two unresolvable names here. Measured over `tt (<63 × 'c'> INT, x INT)` with those two
names: 18.6 answers `42701` for the `INSERT` column list, `42601` for the `SET` list and
`42701` for `COPY`, while all three are `42703` here, the single-name spelling being
18.6's truncation (a `42622` **notice**, not an error) against pglet's `42703`. It is the
parser's, not this family's — but it is what decides
whether a target column list is a set of the *same* things on the two engines, which makes
it this section's residual as much as [parser.md](parser.md)'s parse-level one.

**And a verdict pglet reaches *nowhere* leaves the gate standing in front of nothing.** A
non-boolean `WHERE` is 18.6's `42804` and is accepted here — `UPDATE t SET a = 1 WHERE a`
runs and matches no row, measured over a row holding `9` and over one holding `0` alike —
so `UPDATE t SET a = 1, a = 2 WHERE a` is `42804` there and `42601` here, and what to
repair is the acceptance rather than this gate's position. An aggregate or a window
function in that same clause is the polarity between: pglet answers its own `0A000` at
execution over a matching row and nothing at all over none, so the repeat's `42601` stands
ahead of 18.6's `42803` / `42P20` at either row count. Read both as **instances** of the
ordering rule above rather than as its membership: what 18.6 decides in parse analysis and
pglet decides somewhere other than its lowering is a set not enumerated here, and no
bound is claimed for it.

## A name written back out

Folding is one direction; the other is rendering a name the catalog holds back
into text. The shared implementation is `lexer::quote_ident`, a pg_dump-`fmtId`
posture that quotes only when needed, with `is_safe_bare_ident` as its predicate
— and the reason it is shared rather than local to any caller is that its callers
are not one kind of thing. A second renderer answers where the string is a value a
client reads and 18.6's own function names the rule: `lexer::pg_quote::quote_identifier`,
18.6's `quote_identifier` over 18.6's keyword table, behind the SQL function
`quote_ident` and `pg_get_serial_sequence`'s answer (`functions/quote_ident_every_keyword.sql`
compares it over every 18.6 keyword). **What an over-quote costs is the caller's property,
not the predicate's**, and the four classes do not all charge the same:

- **SQL text a reload must accept** — the statements `pglet::dump` emits; the
  call sites are the enumeration, and a list here would go stale. Over-quoting is **free**: both engines take
  `"action"` wherever they take `action`. Under-quoting is the defect here, and
  what it costs is **not one thing** —
  `CREATE INDEX Odd Ix ON public.Odd Tbl USING btree (a)`, the string the catalog
  rendering below actually produces, is `42601` on each engine, while the same
  rendering over a `"OddTbl"` parses on each and builds the index on the folded
  `oddtbl` instead (measured; `EDGES.md` row 122 has the per-member table). One
  dump position is **not** this predicate's: a **type** name, at `CREATE TYPE` and
  at an enum column's type, goes through `deparse::bare_type_ident`, since pglet's
  type-name grammar reads a bare identifier and rejects a quoted one — so quoting
  is not free there, and the emitter *refuses* an unsafe name rather than write
  text its own parser would reject.
- **A name written inside a string literal** — `nextval('<n>'::regclass)`,
  `setval('<n>', …)`. Same posture, same cost, one indirection further in; the
  full statement is [sequences.md](sequences.md) § A sequence's name reaches that
  script in two grammars.
- **Catalog text a client reads** — `pg_indexes.indexdef`,
  `pg_get_constraintdef`, `information_schema.check_constraints.check_clause`.
  pglet re-parses none of it; a schema differ compares it as text. So an over-quote
  is not free here, it is a **wrong value**, measured over a column named `action`: `indexdef`
  reads `(parent_id, "action")` here against 18.6's bare `(parent_id, action)` and
  carries the same quoting into an expression key and a partial predicate
  (`(("action" + 1)) WHERE ("action" > 5)` against `((action + 1)) WHERE
  (action > 5)`), `pg_get_constraintdef` reads `CHECK (("action" > 0))` against
  `CHECK ((action > 0))`, and `check_clause` reads `("action" > 0)` against
  `(action > 0)`. A column named `position` agrees on both, which is what makes
  the divergence easy to miss: `position` is a `COL_NAME_KEYWORD`, which 18.6
  quotes too.
- **A value a client reads back** — `<expr>::regclass`. Same wrong-value cost. The
  client may feed the value to `'…'::regclass` again, and it resolves on whichever
  engine printed it, so what costs here is the differing *spelling* rather than a
  broken round trip — which is why row 85 (f) files it as a residual and not as a
  defect.

The disagreement is the **keyword half** of the predicate, in both directions:
18.6's `quote_identifier` quotes a word whose `ScanKeywordCategories` entry is not
`UNRESERVED_KEYWORD`, while `is_safe_bare_ident` asks whether *pglet's own lexer*
reads the word as an `Identifier`. Measured at one word each way through
`::regclass`: `'"action"'::regclass::text` is `"action"` here and `action` there,
`'"numeric"'::regclass::text` is `numeric` here and `"numeric"` there.
`EDGES.md` row 85 (f) owns that rule — its
measured members, its `pg_get_keywords()` sweep instrument, and the argument for
moving the predicate to a keyword-category table rather than refining it — and
this section deliberately carries no member list of its own, since one would go
stale against both the oracle's keyword table and pglet's lexer.

**And a caller can skip the predicate outright.** `pg_indexes.indexdef` sends its
key columns through `quote_ident` and then interpolates the index's own name and
its table's raw — so there the divergent set is **18.6's** predicate's, not this
one's, and what the bare rendering costs is not one thing: a parse error
at some names, a *different* relation at others, and at others only the spelling
(`EDGES.md` row 122). The dump's `emit_create_index` builds
the same statement and quotes both — measured, and the reason the dump-invariant
survives it. Two renderers of one statement disagreeing is the shape to check for
whenever a name reaches text by a second route.

## A constraint name lives in two spaces, and a generated one is disambiguated rather than refused

Postgres puts every constraint in `pg_constraint (conrelid, conname)` — **one**
unique index over CHECK, FOREIGN KEY, PRIMARY KEY and UNIQUE, so the kinds share
a name pool (`42710`) — and additionally puts a PK/UNIQUE's *index* in the
`pg_class` relation namespace alongside tables and other indexes (`42P07`,
tested first). Reading only half of that under-rejects in one direction and
over-rejects in the other: per-kind pools let every cross-kind duplicate through,
while gating the *auto*-generated name breaks the ordinary
`CREATE TABLE t (a INT UNIQUE, UNIQUE (a))`. `executor/ddl.rs`'s
`ConstraintNames` is the one gate that reads both spaces, as
`catalog ∪ claimed-by-this-statement` so a `CREATE TABLE`'s in-flight names are
visible to its next constraint (Postgres's `CommandCounterIncrement`). The
asymmetry is the point: a **declared** name is the user's to get wrong, an
**auto**-name is the engine's and gets a digit appended until it fits
(`ChooseRelationName`), and two specifications describing the same index are
**merged**, not rejected (`transformIndexConstraints`) — except two
`PRIMARY KEY`s, which are `42P16` even when identical, because the multiplicity
check runs per constraint before the merge loop. That last one is not cosmetic:
`Schema::primary_key_index` is a `find`, so a second primary key would make FK
default-key resolution and `GROUP BY` functional dependency depend on
index-name order.

## A constraint's name is minted once, at execute time, and is the same fact everywhere

The declared/auto asymmetry above is not a PK/UNIQUE rule — it is *the* rule,
for all four kinds, and the way to get it wrong is to decide the name anywhere
but the one namer. Deciding it at **parse** time makes declared-vs-auto
structurally unrepresentable (a plain `String` cannot say "the engine chose
this"), which costs both halves at once: an FK's auto-name gated where
Postgres disambiguates, and a declared name on an inline `REFERENCES`
silently dropped. Deciding it **lazily at display time** costs worse, because each
reader synthesizes its own: the violation message, `information_schema` and the dump
agree on nothing, `information_schema` can omit the constraint entirely, and the dump
can emit two `ADD CONSTRAINT` statements under one name, which stock Postgres and
pglet's own restore both refuse. So the split is by **side**, not by kind: the parse
side says declared-or-nothing (`planner::ForeignKeyDef`,
`planner::CheckConstraintDef`), the catalog side is **total** (`ForeignKey`,
`CheckConstraint` — a plain `String`), `executor/ddl.rs`'s `ConstraintNames` is
the only bridge — one claimed-name pool with **two** minters, so a reader
re-deriving the mint sites must walk both: `mint_constraint` for CHECK and FK,
reached only through `mint_check` (`execute_create_table`,
`execute_alter_table_add_column`, `execute_alter_table_add_check`) and
`mint_foreign_key` (`execute_create_table`, `execute_alter_table_add_column`,
`add_foreign_key`), and `mint_constraint_index` for PK and UNIQUE, whose minted
name is the index's too (`execute_create_table`,
`execute_alter_table_add_column`, `add_primary_key`, `add_unique`) — and the
minted name is **persisted**. Neither minter formats a name: each hands the table's
name, a second name and a label to the one chooser (§ An unnamed index's name below),
which is where the name is cut to 63 bytes and where the digit goes. The catalog name
is not an `Option` whose `None` means "reconstruct it": a reconstruction can only search
the table's own preceding checks where the namer searches the whole schema across all
four kinds, so `CREATE TABLE q (a INT, CONSTRAINT r_a_check CHECK (a > 0)); CREATE TABLE
r (a INT, CHECK (a > 0));` would yield `r_a_check1` from the namer and `r_a_check` from
the reconstruction — one database, two dumps, decided by which build wrote it. A
compatibility branch that answers differently from the code it stands in for is not
compatibility.

The two sides search different scopes and the asymmetry is deliberate
(`pg_constraint.c`: "deliberately not the same test"): a **declared** name is
checked per-relation (`ConstraintNameIsUsed`) — so the same
`CONSTRAINT positive_amount` on two different tables is ordinary, valid schema,
and that direction needs asserting as explicitly as the other, since widening
the gate to the auto scope bricks ordinary hand-written DDL while reddening
exactly one fixture (`ddl/check_name_may_match_plain_index.sql`'s second table,
which is why its header says not to trim it) — while an **auto** name is
disambiguated **schema-wide** over all four kinds (`ChooseConstraintName`'s
`(conname, connamespace)` scan) — and reads *only* `pg_constraint`, so a plain
`CREATE INDEX t_a_fkey` does not push an FK's auto-name, where a PK/UNIQUE
auto-name must clear the relation namespace too because its index carries it
(`ChooseRelationName`). Every write path mints through the one gate —
`CREATE TABLE` (column- and table-level, all four kinds),
`ALTER TABLE … ADD CONSTRAINT`, `ADD CHECK`, `ADD COLUMN`'s inline
PK/UNIQUE/CHECK/REFERENCES.

Because the name is minted from a *position in a list*, the list's order is
part of the contract: a `CREATE TABLE`'s elements are folded in **written**
order across the column-level and table-level spellings alike (Postgres walks
`stmt->tableElts` once, routing both onto the same `cxt.ck`/`ix`/`fkconstraints`
lists), so two sinks concatenated after the fact pair a persisted auto-name
with the wrong specification — invisible to a name-only query, and visible the
moment the two constraints differ in anything else. And *which kind* it mints
first differs by route: `CREATE TABLE` names its CHECKs, then its indexes, then
its FKs (`DefineRelation`), while `ADD COLUMN` names its **index first**, then
the CHECKs, then the FK. That second order is a *reschedule*, not the pass
numbering, and reading it off the enum gets it backwards: `ALTER TABLE`'s
passes run ascending and `AT_PASS_ADD_CONSTR` (6) is **ahead** of
`AT_PASS_ADD_INDEX` (9). Nothing final happens in pass 6 —
`transformAlterTableStmt` schedules the inline `UNIQUE` straight into pass 9 as
an `AT_AddIndex`, while the CHECK and FK arrive in pass 6 as
`AT_AddConstraint`s that `ATParseTransformCmd` only re-files into
`AT_PASS_ADD_OTHERCONSTR` (10). Both orders are observable and they point
opposite ways on the same column definition.

## The same mint rule governs a sequence's name, over a *different* namespace — and a sequence is a relation

A `SERIAL` / identity column's `<table>_<column>_seq` is the engine's name, so
it is disambiguated (`t_a_seq`, `t_a_seq1`, …) rather than refused, by the same
`ChooseRelationName` shape the constraint indexes use (cut to 63 bytes as theirs is —
§ An unnamed index's name below) — and for the same reason
it cannot be decided at parse time: the parse side says *implicit-sequence*
(`planner::ColumnDef.implicit_sequence`, with `default: None`) and
`executor/ddl.rs`'s `resolve_implicit_sequence_name` is the only bridge, at
every route (`CREATE TABLE`, `ADD COLUMN`, `ALTER COLUMN … ADD GENERATED`). The
user's own `DEFAULT nextval('x')` is never rewritten. **And the declared half
is the same rule, not a footnote:** an identity column's `SEQUENCE NAME <n>` is
the *user's* name, so it is taken verbatim and refused (`42P07`) when the
relation namespace holds it — implementing only the auto half and discarding
the declared input leaves the rule half-true in the way that bites, because
pg_dump emits `SEQUENCE NAME` on every identity column and the re-minted
`<t>_<c>_seq` *coincides* with it in the default case: the restore's `setval`
then names a sequence the catalog lacks and the identity counter silently
restarts at 1 on top of the restored rows, and only a corpus using a
non-default name can see it.

Three asymmetries are load-bearing and none is guessable. The candidate must
clear the **relation** namespace — Postgres's `pg_class` holds tables, indexes
**and sequences** in one pool, so `Schema::relation_name_taken` reads all three
and `CREATE SEQUENCE ns1; CREATE TABLE ns1 (…)` is `42P07` in either polarity —
but it must **not** consult `pg_constraint`, because `transformColumnDefinition`
passes `isconstraint = false`; a CHECK named `t_a_seq` leaves the column's
sequence on `t_a_seq`. And the *order* against the constraint indexes is
observable: the serial sequences ride `transformCreateStmt`'s **before**-list,
so they are created ahead of the heap and its indexes — `CREATE TABLE b1
(a SERIAL, CONSTRAINT b1_a_seq PRIMARY KEY (a))` is `42P07` because the
sequence took the name first, and the names minted for them are seeded into
`ConstraintNames`. That before-list also puts the implicit sequences ahead of
the table's **own** `42P07` and behind parse analysis, so a colliding declared
name beats an existing table of the statement's name and a duplicate CHECK
name, and loses to `IF NOT EXISTS`, to the column type (`42704`) and to the key
columns (`42703`) — four more positions of the one contract, each measured.
**Except when the name collided with is the table's own**, which is a fifth
position and reads off none of the other four: the sequence is created
successfully and `heap_create_with_catalog` is what finds the clash, so the
error is the *table's* `42P07` (`CREATE TABLE a1 (id INT GENERATED ALWAYS AS
IDENTITY (SEQUENCE NAME a1))`, `IF NOT EXISTS` included — its skip only fires
when the table already exists) and a later column's collision with an
already-taken name still outranks it. That one cannot be gated in the naming
loop at all — the reason [ddl.md](ddl.md) § *A gate that reads the catalog is
blind inside the statement that is building it* generalizes. The auto-name,
though, reads the **committed catalog only** and never the claimed set, which
is the one place it diverges from `ConstraintNames`' `catalog ∪ claimed`:
`ChooseRelationName` runs while `transformCreateStmt` is still *appending* to
the before-list, so nothing is there to see yet, and `CREATE TABLE zf (b INT
GENERATED … (SEQUENCE NAME zf_a_seq), a SERIAL)` is `42P07` on both engines
where consulting the claimed set would silently hand the SERIAL `zf_a_seq1`.

`ALTER SEQUENCE`'s own precedence is a third instance of the error-order
contract ([ddl.md](ddl.md) § *Which error wins*) and does not follow from the
others: `validate_relation_kind` runs **before** the `IF EXISTS` skip, so
`ALTER SEQUENCE IF EXISTS <a table>` is `42809` and not a no-op, while a name
no relation holds is `42P01`; both outrank the `OWNED BY` column's `42703`. On
`CREATE SEQUENCE` the same three sit the other way up — the `IF NOT EXISTS`
skip first (so a bad `OWNED BY` is never even resolved), then `42P07`, then the
owner. And the kind check has to be asked of **every operand, not the
statement**. One
`ALTER SEQUENCE … OWNED BY t.c` carries two relation operands with two
different wrong-kind *sentences* (`validate_relation_kind`'s
`cannot open relation "x"` for the target, `process_owned_by`'s
`sequence cannot be owned by relation "x"` for the owner), and a third kind — a
sequence — is wrong for the owner while being right for the target. The same
`42809` reaches further than either statement: `nextval`/`currval`/`setval` all
open their relation through `init_sequence`, so a table name there is `42809`
and not the `42P01` a sequences-map-only lookup reports. **And a
`DEFAULT nextval('x')` naming a table is `42809` at DDL time in pglet,
deliberately stricter than Postgres**, which resolves `'x'::regclass` at parse
analysis and defers the kind check to the first read. pglet's `nextval` argument
is a **name literal** and not a `regclass` reference — `parser::ddl`'s
`parse_nextval_arg` eats the cast token without resolving the type, and reads the
string through `reduce_name_literal` — and a DDL `DEFAULT nextval('q1')`
*auto-creates* `q1` (measured: the column then draws `1`), so deferring would not
defer the error — it
would hold one name in `tables` and `sequences` at once, the state
`relation_name_taken` declares impossible, and the database would
dump to a script carrying a `CREATE SEQUENCE q1` and a `CREATE TABLE q1` that
neither engine reloads. The invariant holds because every registration site asks it,
not only the `CREATE SEQUENCE` door.

Three corollaries across the rest of the statement surface, and none is a detail. **The wrong kind outranks a *skip*, not just an error** —
Postgres decides the relkind on a name that *resolves*, so only a name no
relation holds reaches `DropErrorMsgNonExistent`; `DROP TABLE IF EXISTS`,
`DROP INDEX IF EXISTS` and `CREATE INDEX IF NOT EXISTS <a taken name> ON` are
`42809` over **any** wrong-kind target, and `ALTER TABLE IF EXISTS <a sequence> ADD
COLUMN …` is `42809` too — its skip, like theirs, reads the whole relation namespace. Read the other way round these are the
*non-loud* members of the class: the statement reports success, and the next
one to name that relation insists it already exists — the exact contradiction
the `42809` exists to remove, restated as a silent one. **And the kind
is a property of the (found, expected) pair, so the check must read the whole
relation namespace and not one map.**
`pg_class` holds tables, indexes and sequences in one pool, so *every* ordered
pair of kinds is reachable; a check written against `schema.sequences` alone is
therefore right about a third of the class and silently wrong about the rest.
The `IF EXISTS` spellings are where it goes silent: such a check lets
`DROP TABLE IF EXISTS <an index>` and `DROP INDEX IF EXISTS <a table>` skip and report
success while the other index-side shapes answer `42P01`/`42704` — and an enumeration
of the **bare** forms misses exactly the silent ones. `Schema::relation_kind` is the
one enumeration and `Error::WrongObjectType` the one error, both directions, so a
fourth kind cannot be added to half of it. **And the lookup that decides is a
function's first, not its shared one**: a gate routed through `get_table_or_err`
reaches only the call sites with no open-coded lookup ahead of it — an earlier
`get_table` in the same function answers `42P01` first. When a gate moves into a
shared helper,
the question is not "does this function call the helper" but "does anything
resolve the relation *before* it".

## An unnamed index's name is chosen as a sequence's is, and both are cut to 63 bytes

`CREATE [UNIQUE] INDEX [CONCURRENTLY] ON t (…)` names no index, and the name is the
engine's to choose — so it is the declared/auto asymmetry again: a declared index
name is the user's and a taken one is `42P07` (or the `IF NOT EXISTS` skip), an absent one
is chosen and steps around a taken name instead of failing on it. The parse side says
declared-or-nothing (`Plan::CreateIndex`'s `name` is an `Option`), and
`execute_create_index` is the bridge: it chooses the name where it would test a declared
one — behind the gates that stand ahead of that test (a missing table is `42P01` and a
missing key column `42703`, as with a name) and ahead of the build — and the chosen name
is persisted, so the dump writes the name the catalog holds. The grammar keeps the name required behind `IF NOT EXISTS`
(`gram.y`'s `IndexStmt` is two productions, `opt_single_name` in the plain one), so there is
no unnamed index to skip over: `CREATE INDEX IF NOT EXISTS ON t (a)` is `42601` on both
engines.

The name is 18.6's `ChooseIndexName` for an index that is no constraint's, in three steps
that `executor::ddl`'s `choose_index_name` takes in 18.6's order:

- **Each key part gets a name** (`ChooseIndexColumnNames`). A bare column's is its own. An
  expression's is the name a `SELECT` would give it as an output column
  (`FigureIndexColname`, here `Expr::output_name`) — `lower(t)` is `lower`, `(a)::bigint` is
  `a`, `coalesce(a, 0)` is `coalesce` — with `expr` where a `SELECT` says `?column?`. A part
  whose name an earlier part of the same key already holds takes a digit of its own:
  `(a, b, a, a)` is `a`, `b`, `a1`, `a2`, and two operator parts are `expr`, `expr1`. Where pglet names an output column differently from 18.6, the index
  name differs with it (`EDGES.md` row 277's residual).
- **The parts are joined by `_`** and stand as the second name beside the table's, under the
  label `idx` — for a `UNIQUE` index as for a plain one. `key` is the label of a `UNIQUE`
  **constraint**'s index, and `CREATE UNIQUE INDEX` declares no constraint: over one table
  `b INT UNIQUE` is `t_b_key` and `CREATE UNIQUE INDEX ON t (b)` is `t_b_idx`, measured.
- **`ChooseRelationName` finishes it**, with `isconstraint = false` — the routine a `SERIAL`
  or identity column's sequence name goes through, and one function here
  (`choose_relation_name`). It builds `<table>_<parts>_<label>` through `makeObjectName`,
  which cuts the result to `NAMEDATALEN - 1` = 63 bytes: the label and the two underscores
  are never cut, the two names share what is left, the **longer** giving up a byte at a time
  until they fit, and each is then clipped back to a character boundary (so a multibyte name
  can come out shorter than 63). A taken name moves a digit onto the **label** — `idx1`,
  `idx2` — and the name is cut again around it, so a stepped name is no longer than an
  unstepped one. What it steps around is the relation namespace alone
  (`Schema::relation_name_taken`: tables, indexes, sequences): a `CHECK` named `t_a_idx` and
  an enum named `t_a_idx` both leave the index on `t_a_idx`, measured, as a `CHECK` named
  `t_a_seq` leaves a sequence on its name.

The name is chosen against the catalog as the statement finds it, and nothing is claimed
ahead of the build: a build that fails (`23505` over duplicate rows, an expression key that
raises on a row) leaves the name free for the next statement, a rolled-back block gives its
names back, and dropping `t_a_idx` lets the next unnamed statement take it again rather than
counting on from the highest digit handed out.

**One cut stands behind every name the engine chooses, and one digit loop behind every
one it mints.**
`catalog::make_object_name` is the cut and `executor::ddl`'s `choose_name` the loop — build
the candidate, ask the caller's namespace whether it is taken, move a digit onto the label
and cut again. A sequence name goes through them exactly as an index name does — over
40-byte table and column names `pg_get_serial_sequence` answers 18.6's 63-byte name, and two
`SERIAL` columns whose names meet after the cut are the `42P07` 18.6 answers — and so does
each constraint's. What differs between the constraint kinds is three things, each of them
18.6's:

- **The second name.** A `PRIMARY KEY` has none, whatever its columns (`<table>_pkey` —
  `ChooseIndexName`'s `primary` arm), and neither has a `CHECK` that reads no single column
  (`<table>_check`); `make_object_name` reads an empty second name as that absent one, with
  one underscore and the whole budget the table's. A `UNIQUE`'s is its key columns joined by
  `_` in **key** order (`ChooseIndexNameAddition`), a `FOREIGN KEY`'s its referencing columns
  likewise (`ChooseForeignKeyConstraintNameAddition`), a single-column `CHECK`'s that column.
- **The label** — `pkey`, `key`, `fkey`, `check` — never cut, and where the digit goes:
  `…_key1`, never `…_key` with a digit after the 63rd byte. So two names that are different
  whole and equal once cut are told apart as two that were equal whole are, and each still
  fits. It is what keeps the dump loadable: 18.6 truncates an identifier it reads to 63
bytes, so two such names written whole would arrive there as one.
- **The namespace the digit steps around**, unchanged by the cut and stated above: a
  `PRIMARY KEY`'s and `UNIQUE`'s candidate clears the relation namespace and every constraint
  name in the schema (`ChooseRelationName`, `isconstraint = true`), a `CHECK`'s and `FOREIGN
  KEY`'s the schema's constraint names alone (`ChooseConstraintName`), each as `catalog ∪
  claimed`.

A name that fits in 63 bytes is not cut: it is `<table>_<second name>_<label>` whole.

**A `NOT NULL` is the one name that is rendered and never minted.** pglet stores a `NOT
NULL` as a column bit, so `catalog/constraints_view.rs` builds its `pg_constraint` name where
it reads the row — through the same cut, so `<table>_<column>_not_null` over long names is
18.6's 63-byte name — and has nowhere to keep a digit. 18.6 mints that name through
`ChooseConstraintName` like a `CHECK`'s, so there it steps around a name already held: two
`NOT NULL` columns whose names meet after the cut are `…_not_null` and `…_not_null1` there
and one name twice here, as are a column `a` of a table declaring `CONSTRAINT t_a_not_null
CHECK (…)` and the second of two 60-byte tables that differ in their last byte. Nothing
mints, drops or dumps that name, so the long-name cases cost a catalog read — rows of one
table sharing a `conname`, which a join keyed on the name multiplies — while the declared
`t_a_not_null` case also dumps a `CHECK` that 18.6 refuses (`EDGES.md` row 278's residual).

**The cut closes the producers, not the state, and it is the chooser's — not the
parser's.** A store written by a build whose minters do not cut holds whole names, and
they stay what they are: nothing re-derives a persisted name, so such a store opens,
enforces and reports each constraint under the long name, drops its table, and dumps
those names whole. And a **declared** name is taken as written at any length, where 18.6 truncates
every identifier to 63 bytes as it reads it ([parser.md](parser.md) § Known lexical / parse
divergences): two declared constraint names that share their first 63 bytes are two
constraints here and one there — the second is `42710`, or `42P07` where both are a
`UNIQUE`'s — so a dump holding such a pair still restores short into 18.6 (`EDGES.md` row
278's residual).

## A table's name is a type name too

18.6 gives every table a **row type** of the table's own name, so `pg_type` holds it
beside each enum, and a new name is asked of that namespace by the routine that
creates the object — not by one shared "is this name free?". `heap_create_with_catalog`
asks it of every relation it creates, behind the relation name's own `42P07`, and a
sequence is one of those although it gets no row type: a table or a sequence over an
enum's name is `42710 type "…" already exists`, with the hint `A relation has an
associated type of the same name, …`. `DefineEnum` asks it of an enum, so an enum over
a table's name is `42710` with no hint. `index_create` never asks, and neither an index
nor a sequence has a row type, so an index over an enum's name and an enum over an
index's or a sequence's are all accepted — measured with `reltype <> 0` over `pg_class`,
true of a table and a view and false of an index and a sequence.

`Schema::type_name_taken` is that namespace in pglet: the enums and the tables' names.
Two consequences are not guessable. The **minted** `<table>_<column>_seq` of a `SERIAL`
or identity column is asked too, and is refused rather than disambiguated —
`ChooseRelationName` steps around `pg_class` alone, so over an enum `h_a_seq` 18.6
answers `CREATE TABLE h (a SERIAL)` with `42710` and never picks `h_a_seq1`
(`resolve_implicit_sequence_name`, at all three routes). And the `IF NOT EXISTS` skips of
`CREATE TABLE` and `CREATE SEQUENCE` read the relation namespace only, on both engines,
so over an enum they are the `42710`, not the skip. Where the table's own check sits is
measured from both sides: behind the column types (`42704`) and the repeated column
(`42701`), ahead of a missing foreign-key parent (`42P01`), a constraint index name a
relation holds (`42P07`) and a default's cook (`22P02`). The row type is also what a
`DROP TYPE <a table>` finds: `2BP01 cannot drop type t because table t requires it`,
`IF EXISTS` and `CASCADE` included — found by the name as written, ahead of the enum
lookup's folded key, so `DROP TYPE "Rd"` beside a table `"Rd"` and an enum `rd` names
the row type, as on 18.6. What missing it costs is the dump invariant: a table and an
enum of one name dump to a script whose `CREATE TABLE` 18.6 refuses, losing the table
and its rows with `psql` exiting `0`. A store written by a build without these gates can
still hold such a pair, and opens; whichever of the two `CREATE`s a restore runs second is refused
on both engines, so the dump refuses that store (`0A000`, naming the table) until the pair is parted — by dropping
the enum where no column declares it; under a column of it `DROP TYPE` is accepted here, where 18.6 answers `2BP01`, and
the store then dumps short, silently (`EDGES.md` row 248).

The asymmetry leaves pairs legal — an enum beside an index or a sequence of its name —
and of those the sequence is the one that constrains the dump's order: both engines
accept the enum only once the sequence exists. So the dump emits every `CREATE SEQUENCE`
ahead of every `CREATE TYPE`, the only order both reload, and refuses (`0A000`) a
database whose **identity** sequence
shares an enum's name — that sequence is created by an `ADD GENERATED` after its table,
which may declare the enum, so the dump's order does not reload it (`EDGES.md` row 248, which also keeps
the residuals — a quoted enum name against its folded key, and a row type pglet does not
otherwise model).

