# Sequences — the catalog record, ownership, and the dump

This doc covers the sequence catalog record and every rule that reads it: the
start/counter/owner split, the ownership edges `DROP` and
`TRUNCATE … RESTART IDENTITY` follow, the identity-column link, how the dump renders all
of it, and what a rollback does and doesn't restore. The repo `CLAUDE.md` summarizes the
rules, and this file has the full statements. Sequence *naming* (minting,
disambiguation, the relation namespace) is in [names.md](names.md).

## A sequence's `start` is not its counter, and `owner` is a third fact

Postgres's `(last_value, is_called)` pair, plus `START WITH`, plus the
`sequence → (table, attnum)` dependency edge, don't fit into one `i64`. A bare
`ALTER SEQUENCE … RESTART` returns the counter to the *start*, and an engine that holds
only the counter can't compute that. A `START WITH` written later moves that target, so
treating that clause as a no-op while honouring `RESTART` gives silently wrong results.
That's why there's a catalog `Sequence` record (store magic `PGMG`/v16), and why every
write path follows one rule: **a counter update moves `last_value` and nothing else.**
Overwriting the whole record (from `setval` or `TRUNCATE … RESTART IDENTITY`, say)
silently erases the start and the owner, and no query that reads `nextval` can see it.

The dump splits the same way, into three statements at three points:
`CREATE SEQUENCE … START WITH` in the schema block,
`ALTER SEQUENCE … OWNED BY` **after** the tables (it names one), and
`setval(…, last_value + 1, false)` after the data. The `is_called = false` form is
required. The `true` form would be `setval(n, last_value, true)`, and for a sequence
that has never advanced, `last_value` is `start - 1`, which stock Postgres rejects as out
of range (`22003`).

**An identity column's sequence is dumped differently: in two statements, and neither
is the `CREATE SEQUENCE` or the `ALTER SEQUENCE … OWNED BY` above.** One `ALTER TABLE … ALTER COLUMN … ADD GENERATED … AS
IDENTITY (SEQUENCE NAME …)` after its table creates the sequence and records the
ownership edge together. It runs over the column its `CREATE TABLE` declared `NOT NULL`
(the clause is `55000` over a column that isn't). So a `CREATE SEQUENCE` next to it is
`42P07` on the restore, and an `ALTER SEQUENCE … OWNED BY` next to it is `0A000`. Only
the trailing `setval` is shared. So leaving the other two out is part of getting the
rendering right, and isn't just tidiness.

**A sequence's name appears in that script in two grammars, and the name is only correct
when both spell the same relation.** Three of the five sites are identifier positions
(`CREATE SEQUENCE`, `ALTER SEQUENCE … OWNED BY`, `SEQUENCE NAME`), and two are string
literals inside a `regclass` cast (`nextval('<n>'::regclass)`, `setval('<n>', …)`).
18.6 reads a literal back with `stringToQualifiedNameList` → `SplitIdentifierString`,
which **downcases** an unquoted part and rejects interior whitespace. So a name that
isn't a safe bare identifier has to carry its *identifier* quoting inside the string:
`nextval('"Odd_a_seq"'::regclass)`. That's pg_dump's own `fmtId` approach of quoting only
when needed.

Emitting the raw name fails quietly. An ordinary `CREATE TABLE "Odd" (a SERIAL, v TEXT)`
would dump to a script that stock Postgres only **partly** refuses. The table and every
row load, and only the column default and the counter go missing, so the identity
silently renumbers from 1 into a collision. A reader that took the literal's bytes
verbatim would restore it into pglet with zero errors, because a DDL
`DEFAULT nextval('x')` auto-creates `x`, and the wrong name just mints a second sequence.
pglet's reader applies the same grammar: a quoted part is unquoted, `""` is folded, an
unquoted part is **downcased**, and a malformed name is `42602`. That's the same fold the
identifier position applies, so the two name positions can't disagree about which
relation a name means.

**Three deliberate divergences** remain, listed on `reduce_name_literal`. There's no
`NAMEDATALEN` truncation. `\v` is missing from the scanner's space set. And an all-digit
literal, **or a lone `-`**, is a name here but an **OID** to `regclassin` in 18.6, whose
`parseDashOrOid` is named for both inputs and handles both. **Whether a part was quoted is
also part of the name.** Dropping it would let a quoted `"PUBLIC"` silently resolve
(`nextval` advancing, `setval` moving a counter, and a `DEFAULT nextval('"PUBLIC".s2')`
auto-creating a sequence), where 18.6 answers `42P01` at all three, and so does pglet
(`sequences/default_nextval_quoted_schema_loud.sql`).

This design follows from `Column.identity` being a catalog fact that **rows can't show**.
An identity column and a plain `nextval`-defaulted one hold identical values. So a
round-trip corpus that compares rows stays green across a dump that loses
`GENERATED ALWAYS`'s rejection of user-supplied ids, and all three refusals that keep the
column tied to its sequence. That's why the guard is a set of refusals rather than a set
of rows.

And because an identity sequence is created **after** its table, not every sequence
exists before every table. An inline `DEFAULT nextval(…)` in a `CREATE TABLE` can name a
sequence that hasn't been created yet (a sibling column of the same table, or a column of
a table earlier in name order). So **every** `nextval` default is moved into an
`ALTER TABLE … ALTER COLUMN … SET DEFAULT` after the whole schema block, and no
`CREATE TABLE` names a sequence at all. pg_dump gets to the same statement a different
way: it prefers the inline form, and falls back to this one when its dependency sort
finds the loop an owned sequence creates.

`owner` is recorded (by an explicit `OWNED BY` and by every implicit `SERIAL`/identity
sequence), dumped, and **read by every consumer of ownership**. `executor/dependencies.rs`
reads it for the AUTO edge a `DROP TABLE` / `DROP COLUMN` follows, and
`Schema::truncation_targets` reads it for `RESTART IDENTITY`'s restart set (Postgres's
`getOwnedSequences`). That set has **three** readers:

- `execute_truncate`, which restarts exactly that set.
- `Plan::mutates_catalog`, for which a truncate writes the catalog exactly when that set
  is non-empty.
- `Plan::swapped_sequences`, which reads it to record each restarted counter's value from
  before the statement, the value a rollback puts back.

If any of them derived the set on its own, it could disagree with the write. A syntactic
reading of `TRUNCATE <a table owning no sequence> RESTART IDENTITY` would record a
restart that never happened.

A column's `NextVal` default is the *other* edge, and it answers a different question.
Reading it for ownership is wrong in **both** directions at once, in one statement. It
would restart a sequence a column only *references*, which Postgres leaves alone (so
`ALTER SEQUENCE … OWNED BY NONE` followed by `TRUNCATE … RESTART IDENTITY` leaves the
counter where it is). And it would skip an **owned** sequence whose column has no
default. That's why the two-way fixture is the one that tells them apart: one owned
sequence with no default, next to one defaulted-from sequence that isn't owned. It's also
why the detach has a check you can see in plain SQL, and not only a dump test. The
restart *target* is a third fact in the same trap. `ResetSequence` writes `start`, and a
reset to a fixed `0` only matches that for the default `START WITH 1`, so a fixture built
on the default can't tell the two apart.

## A link Postgres cannot represent is a link pglet must forbid, and the resolver that reads it must not assume the forbidding

Postgres stores **no** `pg_attrdef` row for an identity column. The column's tie to its
sequence is one INTERNAL `pg_depend` edge and nothing else. pglet has no `pg_depend`, so
it models the same tie as *two* ordinary facts: the column's `NextVal` default and the
sequence's `owner`. Two ordinary facts can be changed independently.

Exempting every **identity** column from the `pg_attrdef` NORMAL edge in
`Dependents::resolve` matches Postgres exactly, but it's wrong as soon as the two halves
come apart. The sequence is then nobody's dependent, an ordinary `DROP SEQUENCE` removes
it, and the column's default names a relation the catalog doesn't have. Every write is
then `42P01`. Worse, the dump emits `DEFAULT nextval(…)` with **no** `CREATE SEQUENCE`,
which stock Postgres refuses, and which pglet's own restore accepts while silently
renumbering the column from 1.

Four statements separate the halves. Postgres refuses three of them, and pglet's gates
refuse them too: `ALTER SEQUENCE … OWNED BY {NONE | another table's column | a plain
sibling | the identity column itself}` is `0A000`, and `ALTER COLUMN … SET DEFAULT` /
`DROP DEFAULT` is `42601`. The fourth goes through the sequence-owner AUTO edge itself
(re-point the owner, then `DROP TABLE` the new owner), and the resolver's narrowed
exemption handles that one.

Three lessons carry over to other cases. First, **the gates and the resolver are both
required, and neither replaces the other.** The gates restore the DDL surface Postgres
accepts. Narrowing the exemption to *this column's own INTERNAL sequence* makes the
resolver correct on its own terms. That's the same argument `FkRef` makes for keying on
position instead of name ([ddl.md](ddl.md) § *A `DROP` decides*), and it's why its guard
has to be a hand-built catalog. Second, what decides the hole is what the resolver can
*see*, not which sites remove a sequence. Where a default names a sequence the catalog
lacks, `executor/dml.rs`'s `a_missing_sequence_is_loud_rather_than_minted_at_one` keeps
the write loud (`42P01`) instead of minting one. Third, **the dump reads this state**. A
column default naming a sequence that's gone is the mirror of the orphaned-owner state,
and the dump corpus checks both directions (`test_dump`).

## Recording a persisted cross-object reference turns every pre-existing leak of the referenced object into a rendering bug

This follows from the record above. While nothing reads an orphaned sequence's owner, an
orphan is harmless. Once `owner` is persisted and emitted, each orphan becomes an
`ALTER SEQUENCE … OWNED BY` naming a relation or column the catalog doesn't have. Stock
Postgres refuses that, and so does pglet's own restore for the column routes. The routes
that would orphan one are closed (see below): a `DROP TABLE` / `DROP COLUMN` that left the
sequence behind, and an `ADD COLUMN … SERIAL` that failed after its sequence was
registered. The emitter still **skips** an unresolvable owner instead of failing loud. A
sequence whose owner is gone is semantically unowned, and an `Err` there would break the
export of the whole database just to keep an edge that points at nothing.

The invariant is broader than "the emitter is faithful". It's this: **the database state
pglet's own accepted statements build, through any door whose state a dump path reads,
must dump to something that loads into stock Postgres and back into pglet**.
`Database::dump_sql` is the dump path pglet exposes. It's reached in process, and through
`pglet_server dump --data-dir` over a store a wire server wrote. A door whose state no
dump path reads is outside the invariant: `pglet_wasm` exposes no dump of its own, and
`pglet_server dump` only takes a `--data-dir`. The state is the catalog and the rows, and
the rows' part is at the end of this section.

The catalog part is **any catalog pglet's own accepted statements build must dump to
something that loads into stock Postgres and back into pglet**. That also forces the
`MINVALUE` a below-1 counter needs, since pglet accepts `START WITH 0` / `setval(s, 0)`
and Postgres won't restore them bare. (`EDGES.md` rows 32, 47, 51, 64 and 104, and row
133's catalog-text members, each record an accepted catalog whose dump doesn't replay,
into stock 18.6 or back into pglet. Row 64's fails to replay only back into pglet, and
row 32's doesn't replay at all, because its export fails. The invariant decides which
rows belong, not this list.)

None of the dangling-owner routes reaches that state. A `DROP TABLE` / `DROP COLUMN` takes
the sequence with it, and an `ADD COLUMN … SERIAL`'s sequence registration is **staged**
behind the backfill validation and only lands in the mutation block (`executor::ddl`'s
`staged_sequences`). For such a route, the guard's assertion ("no `OWNED BY` is emitted
for it") holds trivially, so a guard seeded only by a route tests nothing and stays
green. So the tests assert that the routes leave the sequence *gone*, and both guards
build the dangling owner on the catalog by hand (`dump`'s
`a_dangling_sequence_owner_is_not_emitted` and `test_dump`'s
`seed_sequence_edge_cases`). **A guard whose only seed is a defect stops testing anything
once that defect is fixed**, and it goes green instead of red.

**The rows' part is three properties**, stated for a dump whose catalog statements load.
A row lost because the catalog part failed counts against the catalog part. Row 32 is one
case: its export fails and takes every table's rows with it. Row 104 is another: a
refused `CREATE TABLE` takes its own `COPY` block down and throws off the framing of the
`COPY`s after it, for as far as the data carries it (row 47 measures the desync ending at
a data line that starts `\N`). Row 133's catalog-text members are a third: their open
quote can run over other tables' `COPY` blocks. Each property below lists the records
that break it:

- **A. Rendering.** Over the values 18.6's column type can hold (B's scope, below), 18.6
  and pglet both read the dump's data text as the rows stored. The data text is the
  `COPY` statement that frames a table's rows, and each value's text inside it.
  PostgreSQL's own documentation asks the value half of this of a type's input and output
  functions (`doc/src/sgml/xtypes.sgml`): make them inverses of each other, or dumping
  data into a file and reading it back in has severe problems. `EDGES.md` row 64 breaks it
  at the framing, on its 18.6 leg. A zero-column relation dumps as
  `COPY zc () FROM stdin;`, which 18.6 refuses at the `)` after accepting the
  `CREATE TABLE`, so the table restores without its rows. No live record breaks it at a
  *value's* text. Every renderer a dump path reaches is held to reading back as the value
  stored, jsonb's object keys included.
- **B. Domain.** pglet stores no value that 18.6's column type can't hold. Row 133's
  jsonb-string member breaks it at the store. A NUL inside a jsonb string gets written
  wherever a jsonb-producing expression *wraps* a `Value::Text` instead of parsing jsonb
  text (`jsonb_build_object`, reached through ordinary SQL at `Database::execute`), and
  wherever a bound `Value::Jsonb` carries one. The route and its producer probe, which
  should be re-run rather than trusted, are in `docs/encoding.md` § A stored `Value::Text`
  becomes a jsonb string. The export refuses such a value with `22021` (`pglet::dump`'s
  `refuse_unreadable_values`). So B is broken at the *store* and loud at the *export*.
  That follows the preference below, but it doesn't fix the break.
- **C. Consistency.** The stored rows satisfy the catalog the dump recreates. Row 38's
  legacy store breaks it. That's a store where an earlier build accepted
  `ALTER COLUMN … ADD GENERATED … AS IDENTITY` over a column holding a NULL (the clause is
  `55000` over a column not declared `NOT NULL` here, as on 18.6). The store holds that
  NULL under a column the dump recreates as `NOT NULL`, so its `COPY` fails `23502` into
  18.6 and back into pglet alike, and the table restores with none of its rows. psql exits
  `0` on the 18.6 restore.

Where a faithful dump isn't possible, **the export should refuse loudly** instead of
emitting SQL that restores short with exit `0`. `pglet::dump` already does that for a NUL
stored in a `text` column and for one stored inside a jsonb string (`22021` at both, row
133). At the catalog, it does it for an identity sequence that shares an enum's name,
which the dump's statement order doesn't reload. It also does it for a table that shares
an enum's name, which a store written by a build without the type-namespace gates can
hold, and which no order of the two `CREATE`s reloads (`0A000` at both, row 248).

The two NUL gates are one rule with two readers. A value's text reaches the COPY payload
as itself, so the payload's own encoding scan finds the NUL. A jsonb string is escaped on
the way out, so only a walk over the *value* can find it. Which reader a rendering needs
is a question to ask of every renderer a dump path reaches, and the answer doesn't come
from the NUL. The owner skip at the top of this section doesn't conflict with any of
this: a sequence whose owner is gone is semantically unowned, and the skip only drops an
edge that points at nothing.

## A sequence counter is not rolled back, and the exception is not a type of statement but a replaced *cell*

`nextval` and `setval` write the sequence's current counter cell in place, and they
aren't transactional. So a `ROLLBACK`, a `ROLLBACK TO SAVEPOINT`, a caught
`DO … EXCEPTION` and a failed `DO` all leave them where they are. Restoring them would be
silently wrong: it walks the counter back over ids already issued, and the next insert
collides on a key.

`ALTER SEQUENCE … RESTART`, `TRUNCATE … RESTART IDENTITY`, `DROP SEQUENCE`, and a
`DROP TABLE`, `ALTER TABLE … DROP COLUMN` or `DROP SCHEMA public CASCADE` that drops a
sequence along with its owner or its schema are the other kind. They give the sequence a
**new** cell, and the rollback puts the old one back *as the in-place advances left it*.
That's neither the live value nor the snapshot's, which is why a per-statement
`Plan::swapped_sequences` reads the outgoing value **before** the statement runs. Checked
on 18.6: `BEGIN; nextval → 1; nextval → 2; ALTER SEQUENCE … RESTART WITH 77;
nextval → 77; ROLLBACK` then answers **3**.

Those statements are the routes `Plan::swapped_sequences` lists. For an owner drop, the
list is read through `owned_sequences`, the same derivation `Dependents::resolve` drops
through, so the cells a rollback restores are the cells the drop removes. The owner edge
decides this, and a column's `NextVal` default doesn't. A sequence that the dropped
table's default only references survives the drop and keeps the block's later draws,
while one `OWNED BY` a column that draws nothing from it is dropped and put back. Checked
on 18.6, over a table holding ids `1`–`3`: `BEGIN; DROP TABLE t; CREATE TABLE
t (id SERIAL …); INSERT …; ROLLBACK` then inserts `4`, the reset idiom in the same block
inserts `4`, and a block that draws `4` and then drops the table inserts `5`.
Differential fixtures in `tests/fixtures/transaction/` pin these, along with the column,
identity, savepoint, cascade and caught-`DO` forms.

A `DO` statement lists nothing there, because which of its statements run, and against
which catalog, is only decided as it runs. `execute_do_block` asks each of its statements
for its list as it runs them. A `DO` that **completes** reports the cells it replaced in
its result (`QueryResult::swapped_sequences`). The host passes them to the block's log
next to the list it read ahead (`TransactionSession::record_statement`), and an enclosing
`DO` adds them to its own. A `DO` that **fails** restores its replaced cells itself before
its error returns, and reports none. So a replacement anywhere inside a `DO` is put back
by whichever rollback undoes the `DO`: the block's, a savepoint's, an enclosing `DO`'s
undo, or its own. That covers a `RESTART`, a `DROP SEQUENCE` with or without a
`CREATE SEQUENCE` after it, a `TRUNCATE … RESTART IDENTITY`, or an owner drop, in the
`DO`'s body, its handler, or a `DO` nested in either.

We tested this over a sequence at `3`, or a serial table holding ids `1`–`3`, each
against 18.6, on the native server (in-memory B+tree, `--data-dir`, memory storage) and on
the embed's three backends:

- A block whose `DO` does any of these answers `4` after its `ROLLBACK`: restarts the
  sequence in its body, in its handler or in a nested `DO`; drops and re-creates it;
  truncates the table `RESTART IDENTITY`; drops and re-creates the table; or runs
  `DROP SCHEMA public CASCADE` and re-creates it.
- One that draws `4` and then drops the sequence or the column answers `5`.
- A failing outer `DO` whose nested `DO` completed a `RESTART`, a drop and re-create, or a
  `TRUNCATE … RESTART IDENTITY` answers `4`.
- A failing `DO` in a block that draws `4` first answers `5` after the `ROLLBACK`.

A reopen after the rollback gives the same answers, with the rollback's counter save and
the restored catalog in agreement (`EDGES.md` row 213, closed).

18.6 gives a sequence a new cell for more `ALTER SEQUENCE` options than `RESTART`. Its
`init_params` sets `need_seq_rewrite` for `AS`, `INCREMENT`, `START`, `MINVALUE`,
`MAXVALUE`, `CACHE` and `CYCLE` as well, but not for `OWNED BY`. So rolling back one of
those returns the old cell in 18.6, while pglet, whose list names only `RESTART` among
them, carries the live counter forward. Over a sequence that has drawn `1` and `2`:
`BEGIN`, a draw of `3`, `ALTER SEQUENCE s START WITH 5`, draws of `4` and `5`,
`ROLLBACK`. The next draw is `4` on 18.6, handing out the block's `4` again, and `6` in
pglet (`EDGES.md` row 222). An identity column's `ALTER TABLE … ALTER COLUMN … SET
INCREMENT BY` is an `ALTER SEQUENCE` of the column's sequence in 18.6, and diverges the
same way.

A counter a rollback keeps also has to be durable once the rollback returns. Otherwise a
value `nextval` handed out in a block that then rolls back would exist only in memory,
and a reopen before the next save by any connection would hand it out again. That's true
after a clean close as well as after a crash. 18.6 logs a `nextval`'s advance before it's
used, and a clean shutdown flushes it. So on a store with durable state, a rollback that
ends a block saves the counters it leaves ahead of the durable ones
(`TransactionSession::counter_save`), and a `Database` dropped with a block open rolls the
block back. What it writes is the **durable** catalog with each such counter raised to the
live counter of the same name. It writes nothing else from the live catalog: not the
rolled-back block's own DDL, and not a host's change that no save wrote. Three choices
make that the right write:

- **Compared with the durable counter, not the one the block took the writer on.** If a
  save has run since the block took the writer (the block's own `sync`, on a host that
  has one), the durable counter is the one that save wrote, and it's compared directly.
  (With no save since, the rollback only writes where some counter passed the value it had
  when the block took the writer. So a durable counter that was already behind the live
  one at that point is only raised by a rollback that writes for a draw of its own. That
  lag comes from a failed autocommit statement's draw, which no connection was handed, or
  from a value a block handed out whose own rollback wrote nothing (by the third rule
  below), which a reopen hands out again until a save writes it (`EDGES.md` row 214).)
- **Only raised.** A counter below the durable one (a `setval` moved back) isn't written.
  Because counters are raised by name, a live cell under a sequence's name is only carried
  over where it's above the durable counter. A reopen can skip ahead, but never re-issues.
  The only route we found that reaches this is that `setval`: after the rollback, a reopen
  answers with the counter past the durable one, where the connections answer past the
  `setval` (`EDGES.md` row 214). This rule prevents no re-issue we've measured. A durable
  counter can only be above a correct live one where a save wrote a value the rollback
  took back.
- **Not written where the live catalog holds a structure no save has written**, such as a
  host's change through `Database::schema_mut` outside any block. (Another connection's
  open DDL can't be one. That block holds the writer, so on the native server a block's
  draw waits for it, and on the embedded `Database`, which runs one block at a time,
  another connection's draw is refused `55P03`.) The save writes the store's current tree
  next to the catalog, and a structure no save has written can leave a footprint in the
  tree that the older catalog doesn't account for. We measured two with the check removed,
  both where another connection's open block held the structure (the route the writer
  keeps closed). In the first, an open `CREATE INDEX`'s entries, under a catalog that
  lacked the index, gave a reopen's next index its OID and wrong rows. A reopen now seeds its OID counter
  past every index number the tree holds an entry under, so that footprint reads nothing. In the second, an enum dropped and created again with its labels
  reordered rebuilt, in the tree at once, a unique partial index whose predicate reads the
  enum. The durable catalog's order, written next to those entries, put a committed row
  back inside the predicate with no entry for it, so a reopen accepted a duplicate under
  the unique index. Where the check finds such a structure, the counters wait for the next
  save that writes the live catalog.

"Ahead" is only right because every draw is `+1`: pglet parses and ignores
`INCREMENT BY`, `MINVALUE` / `MAXVALUE` and `CYCLE` (`EDGES.md` row 215). The cases where a
reopen can still hand out an issued value again are `EDGES.md` row 214: a kill with the
block open, a `COMMIT` the device refused, and a rollback whose save fails the store or
finds such a structure.

The three rollbacks — a `ROLLBACK`, a `ROLLBACK TO SAVEPOINT`, and a `DO` block's undo (a
caught `DO … EXCEPTION`'s body, a failed `DO`) — share one implementation
(`Schema::restore_from`), which also carries the monotonic `generation`/`next_oid`
stamps. A restore that took the counters back with the rest of the catalog would answer
`1` twice to `BEGIN; nextval; CREATE TABLE …; ROLLBACK; nextval`.
