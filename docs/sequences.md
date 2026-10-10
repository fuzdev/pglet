# Sequences — the catalog record, ownership, and the dump

The sequence catalog record and every rule that reads it: the
start/counter/owner split, the ownership edges `DROP` and
`TRUNCATE … RESTART IDENTITY` follow, the identity-column link, the dump's
rendering of all of it, and what a rollback does and does not restore. The
rules are summarized in the repo `CLAUDE.md`; this file carries the full
statements. Sequence *naming* (minting, disambiguation, the relation
namespace) lives in [names.md](names.md).

## A sequence's `start` is not its counter, and `owner` is a third fact

Postgres's `(last_value, is_called)` pair plus `START WITH` plus the
`sequence → (table, attnum)` dependency edge do not compress into one `i64`: a
bare `ALTER SEQUENCE … RESTART` returns the counter to the *start*, which an
engine holding only the counter cannot compute — and a `START WITH` written
later moves that target, so tolerating that clause as a no-op while honouring
`RESTART` is silently wrong. Hence the catalog `Sequence` record (store magic
`PGMG`/v16) and the rule every write path follows: **a counter update moves
`last_value` and nothing else.** Overwriting the record — from `setval` or
`TRUNCATE … RESTART IDENTITY`, say — silently erases the start and the owner, and
no query that reads `nextval` can see it.

The dump splits accordingly, into three statements landing at three points:
`CREATE SEQUENCE … START WITH` in the schema block,
`ALTER SEQUENCE … OWNED BY` **after** the tables (it names one), and
`setval(…, last_value + 1, false)` after the data. The `is_called = false`
spelling is required, not stylistic: the `true` form would be
`setval(n, last_value, true)`, and for a never-advanced sequence `last_value`
is `start - 1`, which stock Postgres range-rejects (`22003`). **An identity
column's sequence takes a different split — two statements, and it shares
neither of the first two.** One `ALTER TABLE … ALTER COLUMN … ADD GENERATED …
AS IDENTITY (SEQUENCE NAME …)` after its table creates the sequence and records
the ownership edge at once, over the column its `CREATE TABLE` declared `NOT NULL`
(the clause is `55000` over one that is not), so a
`CREATE SEQUENCE` beside it is `42P07` on the restore and an
`ALTER SEQUENCE … OWNED BY` beside it is `0A000`; only the trailing `setval` is
shared. Suppressing the other two is therefore part of the rendering, not
tidiness.

**A sequence's name reaches that script in two grammars, and a name is only
correct when both spell the same relation.** Three of the five sites are
identifier positions (`CREATE SEQUENCE`, `ALTER SEQUENCE … OWNED BY`,
`SEQUENCE NAME`) and two are string literals inside a `regclass` cast
(`nextval('<n>'::regclass)`, `setval('<n>', …)`). A literal is read back by
`stringToQualifiedNameList` → `SplitIdentifierString`, which **downcases** an
unquoted part and rejects interior whitespace, so a name that is not a safe
bare identifier must carry its *identifier* quoting inside the string —
`nextval('"Odd_a_seq"'::regclass)`, pg_dump's own `fmtId` posture of quoting
only when needed. Emitting the raw name is not loud: an ordinary
`CREATE TABLE "Odd" (a SERIAL, v TEXT)` would dump to a script stock Postgres
**partly** refuses — the table and every row land and only the column default and
the counter go missing, so the identity silently renumbers from 1 into a collision —
while a reader taking the literal's bytes verbatim would restore it into pglet with
zero errors, since a DDL `DEFAULT nextval('x')` auto-creates `x` and the wrong name
simply mints a second sequence. The reader applies the same grammar (a quoted part unquoted,
`""` folded, an unquoted part **downcased**, a malformed name `42602`) — the
same fold the identifier position applies, so the two name positions cannot
disagree about which relation a name means. **Three deliberate divergences**
remain (listed on `reduce_name_literal`): no `NAMEDATALEN` truncation; `\v`
missing from the scanner's space set; an all-digit literal — **or a lone `-`**
— is a name here and an **OID** to `regclassin` there, whose `parseDashOrOid`
is named for both inputs and handles both. **Whether a part was quoted is
itself part of the name**: dropping it would let a quoted `"PUBLIC"` silently
resolve (`nextval` advancing, `setval` moving a counter, and a
`DEFAULT nextval('"PUBLIC".s2')` auto-creating a sequence) where 18.6 answers `42P01`
at all three, as pglet does (`sequences/default_nextval_quoted_schema_loud.sql`).

The shape is forced by `Column.identity` being a catalog fact **rows cannot
witness**: an identity column and a plain `nextval`-defaulted one hold identical
values, so a round-trip corpus comparing rows stays green across a dump that loses
`GENERATED ALWAYS`'s rejection of user-supplied ids and all three refusals that keep
the column tied to its sequence. The guard is accordingly a *refusal* set, not a row
set. And because an identity sequence is created **after** its table, not every
sequence exists before every table, so an inline `DEFAULT nextval(…)` in a
`CREATE TABLE` can name a sequence not yet created (a sibling column of the same
table, a column of a table earlier in name order); **every** `nextval` default is
therefore hoisted into an `ALTER TABLE … ALTER COLUMN … SET DEFAULT` after the whole
schema block, so no `CREATE TABLE` names a sequence at all. pg_dump reaches the same statement by a
different route: it prefers the inline form and falls back to this one when its
dependency sort finds the loop an owned sequence creates.

`owner` is recorded (by an explicit `OWNED BY` and by every implicit
`SERIAL`/identity sequence), dumped, and **read by every consumer of
ownership**: `executor/dependencies.rs` for the AUTO edge a `DROP TABLE` /
`DROP COLUMN` follows, and `Schema::truncation_targets` for
`RESTART IDENTITY`'s restart set (Postgres's `getOwnedSequences`) — which has
**three** readers: `execute_truncate`, which restarts exactly that set;
`Plan::mutates_catalog`, for which whether a truncate writes the catalog *is*
whether that set is empty; and `Plan::swapped_sequences`, which reads it to
record each restarted counter's pre-statement value, the one a rollback puts
back. Re-deriving the set independently in any of them lets it disagree with the
write: a syntactic reading of `TRUNCATE <a table owning no sequence> RESTART
IDENTITY` would record a restart that never happened. A column's `NextVal` default is the *other*
edge and answers a different question, and reading it for ownership is wrong
in **both** directions at once, in one statement: it would restart a sequence a
column merely *references* (which Postgres leaves alone, so
`ALTER SEQUENCE … OWNED BY NONE` followed by `TRUNCATE … RESTART IDENTITY` leaves
the counter where it is) and skip an **owned** sequence whose column carries no
default. That is why the two-way fixture — one owned sequence with no default
beside one defaulted-from sequence that is not owned — is the discriminating
one, and why the detach has a plain-SQL observable rather than only a dump
test. The restart *target* is the third fact in the same trap: `ResetSequence`
writes `start`, which a reset to a fixed `0` matches for exactly the default
`START WITH 1`, so a fixture built on the default cannot tell the two apart.

## A link Postgres cannot represent is a link pglet must forbid, and the resolver that reads it must not assume the forbidding

Postgres stores **no** `pg_attrdef` row for an identity column: the column's
tie to its sequence is one INTERNAL `pg_depend` edge and nothing else. pglet
has no `pg_depend`, so it models the same tie as *two* ordinary facts — the
column's `NextVal` default and the sequence's `owner` — and two ordinary facts
can be moved independently. Exempting every **identity** column from the
`pg_attrdef` NORMAL edge in `Dependents::resolve` is exactly right about Postgres
and false the moment the two halves come apart: the sequence is then nobody's
dependent, an ordinary `DROP SEQUENCE` removes it, and the column's default names a
relation the catalog lacks — every write `42P01` and, worse, a dump emitting
`DEFAULT nextval(…)` with **no** `CREATE SEQUENCE`, which stock Postgres refuses and
pglet's own restore accepts while silently renumbering the column from 1. Four
statements take the halves apart: three Postgres refuses and pglet's gates refuse
alike (`ALTER SEQUENCE … OWNED BY {NONE | another table's column | a plain sibling |
the identity column itself}` → `0A000`, `ALTER COLUMN … SET DEFAULT` / `DROP DEFAULT`
→ `42601`), and one through the sequence-owner AUTO edge itself (re-point the owner,
then `DROP TABLE` the new owner), which the resolver's narrowed exemption answers.

Three things generalize. **The gates and the resolver are both required, and
neither is the other's substitute:** the gates restore Postgres's accepted-DDL
surface, while narrowing the exemption to *this column's own INTERNAL sequence*
makes the resolver right on its own terms — the same argument `FkRef` makes for
keying on position rather than name ([ddl.md](ddl.md) § *A `DROP` decides*),
and the reason its guard has to be a hand-built catalog. What decides the hole is
what the resolver can *see*, not which sites remove a sequence; where a default names
a sequence the catalog lacks, `executor/dml.rs`'s
`a_missing_sequence_is_loud_rather_than_minted_at_one` holds the write loud (`42P01`)
rather than minting one. And **the dump reads this state**: a column default naming a
gone sequence is the mirror of the orphaned-owner state, and the dump corpus checks
both directions (`test_dump`).

## Recording a persisted cross-object reference turns every pre-existing leak of the referenced object into a rendering bug

The corollary of the record above: while nothing reads an orphaned sequence's owner,
an orphan is harmless; once `owner` is persisted and emitted, each orphan is an
`ALTER SEQUENCE … OWNED BY` naming a relation or column the catalog does not have:
refused by stock Postgres, and for the column routes by pglet's own restore. The
routes that would orphan one — a `DROP TABLE` / `DROP COLUMN` that left the sequence,
and an `ADD COLUMN … SERIAL` failing after its sequence was registered — are closed
(below), and the emitter still **skips** an unresolvable owner rather than failing
loud — a sequence whose owner is gone is semantically unowned, and an `Err`
there would brick the export of the whole database to preserve an edge pointing
at nothing.

The invariant is not "the emitter is faithful" but **the database state pglet's
own accepted statements build, through any door whose state a dump path reads,
must dump to something that loads into stock Postgres and back into pglet**.
`Database::dump_sql` is the dump path pglet exposes, reached in process and
through `pglet_server dump --data-dir` over a store a wire server wrote; a door
whose state no dump path reads is outside the invariant — `pglet_wasm` exposes
no dump of its own, and `pglet_server dump` takes only a `--data-dir`. The state
is the catalog and the rows, and the rows' part closes this section. The catalog part is **any
catalog pglet's own accepted statements build must dump to something that loads
into stock Postgres and back into pglet** — which also forces the `MINVALUE` a
below-1 counter needs, since pglet accepts `START WITH 0` / `setval(s, 0)` and
Postgres will not restore them bare (`EDGES.md` rows 32, 47, 51, 64 and 104, and row
133's catalog-text members, each record an accepted catalog whose dump does not replay,
into stock 18.6 or back into pglet: row 64's only back into pglet, and row 32's not at
all, its export failing. The invariant, not this list, decides which rows belong.) None
of the dangling-owner routes reaches the state: a `DROP TABLE` / `DROP COLUMN` takes the
sequence with it, and an `ADD COLUMN … SERIAL`'s sequence registration is **staged**
behind the backfill validation and lands only in the mutation block (`executor::ddl`'s
`staged_sequences`). Over such a route the guard's assertion — "no `OWNED BY` is emitted
for it" — holds trivially, so a guard seeded only by a route tests nothing and stays
green. The routes are therefore asserted to leave the sequence *gone*, and the dangling
owner itself is built on the catalog by hand in both guards (`dump`'s
`a_dangling_sequence_owner_is_not_emitted` and `test_dump`'s
`seed_sequence_edge_cases`): **a guard whose only seed is a defect stops testing anything
once that defect is fixed**, and goes green rather than red.

**The rows' part is three properties**, stated for a dump whose catalog
statements load. A row lost because the catalog part failed is the catalog
part's loss — as at row 32, whose export fails and takes every table's rows with
it; row 104, where a refused `CREATE TABLE` takes its own `COPY` block down and
desynchronises the framing of the `COPY`s after it, for as far as the data carries it
(row 47 measures the desync ending at a data line that starts `\N`); and row 133's catalog-text
members, whose open quote can run over other tables' `COPY` blocks. Each
property is given with the records that break it:

- **A. Rendering.** Over the values 18.6's column type can hold (B's scope,
  below), the dump's data text — the `COPY`
  statement that frames a table's rows, and each value's text inside it — is
  read by 18.6 and by pglet as the rows stored. PostgreSQL's own documentation
  asks the value half of it of a type's input and output functions
  (`doc/src/sgml/xtypes.sgml`): make them inverses of each other, or dumping
  data into a file and reading it back in has severe problems. `EDGES.md` row 64
  breaks it at the framing, on its 18.6 leg: a zero-column
  relation dumps as `COPY zc () FROM stdin;`, which 18.6 refuses at the `)`
  after accepting the `CREATE TABLE`, so the table restores without its rows. No
  live record breaks it at a *value's* text; the half that reads back is what
  every renderer a dump path reaches is held to, jsonb's object keys among them.
- **B. Domain.** pglet stores no value that 18.6's column type cannot hold. Row 133's
  jsonb-string member breaks it at the store: a NUL inside a jsonb string is written
  wherever a jsonb-producing expression *wraps* a `Value::Text` rather than parsing jsonb
  text (`jsonb_build_object`, reached through ordinary SQL at `Database::execute`), and
  wherever a bound `Value::Jsonb` carries one. The route and its producer probe, to be re-run rather than
  inherited, are `docs/encoding.md` § A stored `Value::Text` becomes a jsonb string. The
  export refuses such a value `22021` (`pglet::dump`'s `refuse_unreadable_values`), so B
  stands broken at the *store* and loud at the *export*, which is the preference below
  rather than a close.
- **C. Consistency.** The stored rows satisfy the catalog the dump recreates.
  Row 38's legacy store breaks it: a store in which an earlier build accepted
  `ALTER COLUMN … ADD GENERATED … AS IDENTITY` over a column holding a NULL — the
  clause is `55000` over a column not declared `NOT NULL` here, as on 18.6 —
  holds that NULL under a column the dump recreates `NOT NULL`, and its `COPY`
  fails `23502` into 18.6 and back into pglet alike, the table restoring with
  none of its rows — psql exiting `0` on the 18.6 restore.

Where a faithful dump is not possible, **the export should refuse loudly**
rather than emit SQL that restores short with exit `0`, as `pglet::dump`
already does for a NUL stored in a `text` column and for one stored inside a
jsonb string (`22021` at both, row 133) — and, at the catalog, for an identity
sequence that shares an enum's name, which the dump's statement order does not reload, and
for a table that does, which a store written by a build without the type-namespace gates can hold
and no order of the two `CREATE`s reloads (`0A000` at both, row 248). The two NUL gates are one rule and two
readers: a value's text reaches the COPY payload as itself, so the payload's
own encoding scan finds the NUL, while a jsonb string is escaped on the way out
and only a walk over the *value* can. Which reader a rendering needs is a
question to ask of every renderer a dump path reaches, not a property of the
NUL. The owner
skip at the top of this section does not stand against that: a sequence whose
owner is gone is semantically unowned, and the skip drops only an edge pointing
at nothing.

## A sequence counter is not rolled back, and the exception is not a type of statement but a replaced *cell*

`nextval` and `setval` write the sequence's current counter cell in place and
are non-transactional, so a `ROLLBACK`, a `ROLLBACK TO SAVEPOINT`, a caught
`DO … EXCEPTION` and a failed `DO` all leave them where they are — restoring them is a silent
wrong that walks the counter back over ids already issued, and the next insert
collides on a key. `ALTER SEQUENCE … RESTART`, `TRUNCATE … RESTART IDENTITY`,
`DROP SEQUENCE`, and a `DROP TABLE`, `ALTER TABLE … DROP COLUMN` or `DROP SCHEMA
public CASCADE` that drops a sequence with its owner or its schema are the other
kind: they give the sequence a **new** cell,
and the rollback puts the old one back *as the in-place advances left it* —
which is neither the live value nor the snapshot's, and is why a per-statement
`Plan::swapped_sequences` reads the outgoing value **before** the statement
runs. Measured on 18.6: `BEGIN; nextval → 1; nextval → 2; ALTER SEQUENCE …
RESTART WITH 77; nextval → 77; ROLLBACK` then answers **3**.

Those statements are the routes `Plan::swapped_sequences` lists. An owner drop's
list is read through `owned_sequences`, the derivation `Dependents::resolve` drops
through, so the cells a rollback restores are the cells the drop removes — and it is
the owner edge that decides, not a column's `NextVal` default: a sequence the dropped
table's default only references survives the drop and keeps the block's later draws,
while one `OWNED BY` a column that draws nothing from it is dropped and put back.
Measured on 18.6, over a table holding ids `1`–`3`: `BEGIN; DROP TABLE t; CREATE TABLE
t (id SERIAL …); INSERT …; ROLLBACK` then inserts `4`, the reset idiom under the same
block inserts `4`, and a block that draws `4` and then drops the table inserts `5` —
pinned, with the column, identity, savepoint, cascade and caught-`DO` forms, by
differential fixtures in `tests/fixtures/transaction/`.

A `DO` statement lists nothing there, since which of its statements run, and against
which catalog, is decided as it runs. `execute_do_block` asks each of its statements
for its list as it runs them, and a `DO` that **completes** reports the cells it
replaced in its result (`QueryResult::swapped_sequences`): the host hands them to the
block's log beside the list it read ahead (`TransactionSession::record_statement`), and
an enclosing `DO` adds them to its own. A `DO` that **fails** restores its replaced
cells itself before its error returns and reports none. So a replacement anywhere inside
a `DO` — a `RESTART`, a `DROP SEQUENCE` with or without a `CREATE SEQUENCE` after it, a
`TRUNCATE … RESTART IDENTITY`, an owner drop, in its body, its handler or a `DO` nested
in either — is put back by whichever rollback takes the `DO` back: the block's, a
savepoint's, an enclosing `DO`'s undo, or its own. Measured over a sequence at `3`, or
a serial table holding ids `1`–`3`, each against 18.6 on the native server (in-memory
B+tree, `--data-dir`, memory storage) and the embed's three backends: a block whose `DO`
restarts the sequence in its body, in its handler or in a nested `DO`, drops and
re-creates it, truncates the table `RESTART IDENTITY`, drops and re-creates the table,
or runs `DROP SCHEMA public CASCADE` and re-creates it, answers `4` after its `ROLLBACK`;
one that draws `4` and then drops the sequence or the column answers `5`; a failing
outer `DO` whose nested `DO` completed a `RESTART`, a drop and re-create, or a `TRUNCATE
… RESTART IDENTITY` answers `4`; a failing `DO` in a block that draws `4` first answers
`5` after the `ROLLBACK`. And a reopen after the rollback answers the same, the
rollback's counter save and the restored catalog agreeing (`EDGES.md` row 213, closed).

18.6 gives a sequence a new cell for more `ALTER SEQUENCE` options than `RESTART`: its
`init_params` sets `need_seq_rewrite` for `AS`, `INCREMENT`, `START`, `MINVALUE`,
`MAXVALUE`, `CACHE` and `CYCLE` as well, and not for `OWNED BY`. So a rollback of one of
those returns the old cell there, while pglet, whose list names `RESTART` alone among
them, carries the live counter forward. Measured over a sequence that has
drawn `1` and `2`: `BEGIN`, a draw of `3`, `ALTER SEQUENCE s START WITH 5`, draws of `4` and
`5`, `ROLLBACK` — the next draw is `4` on 18.6, handing the block's `4` out again, and `6`
here (`EDGES.md` row 222). An identity column's `ALTER TABLE … ALTER COLUMN … SET INCREMENT
BY` is an `ALTER SEQUENCE` of the column's sequence there, and diverges the same way.

A counter a rollback keeps must also be durable once the rollback returns. A value
`nextval` handed out in a block that then rolls back would otherwise stand only in memory,
and a reopen before the next save by any connection would hand it out again — after a
clean close as after a crash; 18.6 logs a `nextval`'s advance ahead of use, and a clean
shutdown flushes it. So on a store with durable state, a rollback that ends a block saves
the counters it leaves ahead of the durable ones (`TransactionSession::counter_save`), and a
`Database` dropped with a block open rolls the block back. What it writes is the **durable**
catalog with each such counter raised to the live counter of the same name — nothing else
the live catalog holds — not the rolled-back block's own DDL, nor a host's change no save
wrote. Three choices make that the right write:

- **Compared with the durable counter, not the one the block took the writer on.** Where
  a save has run since the block took the writer — the block's own `sync`, on a host
  that has one — the durable counter is the one that save wrote, and it is compared
  directly. (With no save since, the rollback writes only where some counter passed the
  value it had when the block took the writer, so a durable counter already behind the
  live one then is raised only by a rollback that writes for a draw of its own. That lag
  is a failed autocommit statement's draw, which no connection was handed — or a value a
  block handed out whose own rollback wrote nothing, by the third rule below, which a
  reopen hands out again until a save writes it (`EDGES.md` row 214).)
- **Only raised.** A counter below the durable one — a `setval` moved back — is not
  written. Raised by name, a live cell under a sequence's name is carried only where it
  stands above the durable counter — a reopen that skips ahead, never one that
  re-issues. The one route measured to reach it is that `setval`: after the rollback a reopen answers
  the counter past the durable one where the connections answer past the `setval` (`EDGES.md`
  row 214). It prevents no measured re-issue; a durable counter can
  stand above a correct live one only where a save wrote a value the rollback took back.
- **Not written where the live catalog holds a structure no save has written** — a
  host's change through `Database::schema_mut` outside any block. (Another connection's
  open DDL cannot be one: that block holds the writer, so a block's draw waits for it on
  the native server, and on the embedded `Database`, which runs one block at a time,
  another connection's draw is refused `55P03`.) The save writes the store's current tree
  beside the catalog, and a structure no save has written can leave a footprint there the
  older catalog does not account for. Two were measured with the check removed, both where
  another connection's open block held the structure — the route the writer keeps closed:
  an open `CREATE INDEX`'s entries, under a catalog that lacked the index, gave a reopen's
  next index its OID and wrong rows, a footprint that reads nothing because a reopen seeds
  its OID counter past every index number the tree holds an entry under; and an enum
  dropped and created again with its labels reordered rebuilt, in the tree at once, a
  unique partial index whose predicate reads the enum, and the durable catalog's order
  written beside those entries put a committed row back inside the predicate with no
  entry for it, so a reopen accepted a duplicate under the unique index. The counters then
  wait for the next save that writes the live catalog.

"Ahead" is only right because every draw is `+1`: pglet parses and ignores `INCREMENT BY`,
`MINVALUE` / `MAXVALUE` and `CYCLE` (`EDGES.md` row 215). Where a reopen can still hand an
issued value out again — a kill with the block open, a `COMMIT` the device refused, a
rollback whose save fails the store or finds such a structure — is `EDGES.md` row 214.

The three
rollbacks — a `ROLLBACK`, a `ROLLBACK TO SAVEPOINT`, and a `DO` block's undo (a caught
`DO … EXCEPTION`'s body, a failed `DO`) — share one implementation (`Schema::restore_from`), which also carries
the monotonic `generation`/`next_oid` stamps. A restore that took the counters back with
the rest of the catalog would answer `1` twice to `BEGIN; nextval; CREATE TABLE …;
ROLLBACK; nextval`.
