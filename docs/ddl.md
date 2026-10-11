# DDL gates, dependencies, and error precedence

These are the DDL-time correctness rules: what a `DROP` may take with it, why a
failed DDL statement must leave the catalog untouched, where a pglet-only gate
may run, how stored expressions are validated, and the error-order contract at
every scale. The repo `CLAUDE.md` summarizes them, and this file has the full
statements. Naming and minting are in [names.md](names.md), and sequence
ownership is in [sequences.md](sequences.md).

## A `DROP` decides between auto-drop and refusal, never between silence and a brick

Postgres's `pg_depend` gives each dependent object an edge *kind*. An AUTO edge
means the dependent is dropped with the target. A NORMAL edge means `RESTRICT`
refuses (`2BP01`) and `CASCADE` drops the dependent too. An INTERNAL edge refuses
**with or without `CASCADE`**: `findDependentObjects` raises it at the outermost
recursion level with no `DropBehavior` test in the path, so that check has to come
*before* the resolver's `cascade` short-circuit, not next to the NORMAL ones.

pglet keeps no `pg_depend`, so `executor/dependencies.rs` re-derives the kinds from
the catalog in **one** resolver. `DROP TABLE`, `ALTER TABLE … DROP COLUMN`,
`DROP SEQUENCE` and `DROP TYPE` all use it. `DROP TYPE` uses it for the NORMAL edge
from a `CHECK` or column `DEFAULT` that converts to the enum, which `CASCADE` drops
or clears (`docs/enums.md` § A cast to an enum converts). A column *declaring* the
enum is an edge the resolver doesn't enforce (`EDGES.md` row 261).

The rule isn't "the table owns it." A *self-referencing* FK blocks a drop of its own
table's referenced column exactly like an external FK does, but it's dropped
silently with the whole table. And where one constraint sits on both edges of the
same column, the AUTO edge wins.

**Two object classes go through the resolver, and with the second, the edge kind
depends on the *deletion set*, not just on the dependent.** A sequence that a table
or column owns through `OWNED BY` is AUTO, so it goes with the drop. A column
default drawing from that sequence is NORMAL, but only if that column survives the
statement. So `CREATE TABLE t (a SERIAL); DROP TABLE t` succeeds (the default goes
with the table), a *second* table's default is `2BP01`, and
`CREATE TABLE q (id SERIAL, b INT DEFAULT nextval('q_id_seq')); ALTER TABLE q
DROP COLUMN id` is `2BP01` because a `DROP COLUMN`'s deletion set is one column
wide. The AUTO drop and that refusal have to land as **one change**. The leak is
exactly what kept the second table's default working, so shipping the drop alone
turns a leak into a broken default in a table nobody named.

On the `DROP SEQUENCE` side, the two kinds split by the *owner column*. An identity
column's sequence is its INTERNAL implementation (`2BP01`, even with `CASCADE`). A
`SERIAL`'s sequence is the column's `pg_attrdef` NORMAL edge (`CASCADE` removes the
default and keeps the column, its `NOT NULL` and its rows). A sequence that's only
`OWNED BY` an identity column is neither, because `ALTER SEQUENCE … OWNED BY`
records only the AUTO edge, so that sequence drops normally on both engines.

Between two NORMAL edges, the order **can't** be observed (both are `2BP01`, and
Postgres reports every dependent in one message), so which one pglet names only
affects the message. The report lists dependents **in reverse order of the
statement's own target list** (`reportDependentObjects` walks its array back to
front), not by kind and not by OID. A *single* target with two dependents can't
tell these apart, because there the real rule and OID-descending give the same
order. Neither can a list against its exact reverse, which matches the two OID
directions. A permutation does tell them apart: with OIDs ascending
`tp_a < tp_b < tp_c`, `DROP TABLE tp_b, tp_a, tp_c` details `tc_c, tc_a, tc_b`,
which is neither OID direction.

`DROP INDEX` reads the same catalog fact from the other side. An index owned by a
declared `PRIMARY KEY`/`UNIQUE` is refused outright, since dropping it would stop
enforcing the key *and* erase it from the dump.

Two consequences about **names** matter here, and neither is obvious.

First, **a constraint's name is not its identity.** The resolver identifies a
foreign key by `(table, position)`, because removing by name would also remove a
sibling with the same name. The resolver has to be right on its own terms, because
the `42710` gate doesn't make a same-named pair unreachable. Incomplete corners of
the naming gate produce exactly that pair (a route that claims nothing, an
auto-name gated instead of disambiguated), and a name-keyed resolver would turn
each one into silent integrity loss instead of a duplicate row.

Second, **a constraint's index is named after the constraint**, using the declared
`CONSTRAINT <n>` when there is one, so nothing may re-derive that name from the
table's name (`Schema::primary_key_index` asks the marker instead of guessing
`<table>_pkey`). Giving an introspecting client a made-up auto-name is the same
kind of silent wrong answer as giving it nothing, only harder to notice.

## A failed DDL statement must leave the catalog untouched

This is because the autocommit path can't put it back. `run_planned_env` rolls back
**storage** on an error but leaves `self.schema` changed in place (the server
snapshots the catalog only inside an explicit `BEGIN`), and the next successful
write serializes the whole schema. So a statement that changes the catalog and
*then* fails leaves a **durable** phantom: the table shows up in
`information_schema.tables`, every read of it is `42P01` (storage was rolled
back), and its name is `42P07` forever. This rule is about correctness, not
tidiness. It's the only thing keeping a raised error from bricking a name.

So `execute_create_table` runs **everything fallible before
`schema.create_table`**. The FK resolution goes first. Next to it are the PK/UNIQUE
key columns, the multiple-primary-key count, and the constraint-index name gate,
which needs the table's own name *seeded* into the relation namespace, since
Postgres creates the heap before its indexes. Last come the **sequence names the
statement itself registers**: a `SERIAL`'s implicit sequence and a user's own
`DEFAULT nextval('x')` are both claimed before the constraint indexes are named.
Otherwise the index gets minted on top of the sequence, and only the post-mutation
backstop catches it.

The `ALTER TABLE … ADD COLUMN` version is the worst case of this in the engine, and
"phantom" undersells it. There the *column* lands in the catalog while the widened
rows roll back with storage, so the table keeps two columns and one-value rows, and
**every `SELECT *` fails at the wire** (`unexpected field count in "D" message`)
for every client, permanently. It reproduces only on the B+tree backend. The plain
in-memory storage double never rolls back and leaves the backfill applied, so a
guard written against that double sees nothing.

The differential harness can't see this class at all: every fixture runs inside
`BEGIN … ROLLBACK`, which restores the snapshot. So the guards are unit tests
(`crates/pglet`'s `*_leaves_no_phantom`). Keep this in mind before adding any DDL
check: on this path, **where** a check runs is a correctness decision, not a style
one.

**A `DO` statement can't follow this rule by staging, so the executor puts the
catalog back itself.** Its body is several statements, and a failing one comes
after earlier ones have finished their catalog writes. Staging inside each
statement does nothing for the ones already done. So `execute_do_block` takes a
storage savepoint and a `Schema` snapshot before the block's first statement. On
every exit where a statement in the block raised (no handler, an unmatched one, or
a handler that raises), it rolls storage back to the savepoint, restores the
catalog through `Schema::restore_from`, and re-derives the index specs over any
enum the restore moved, all before the error returns.

The sequence cells it restores include every one that a statement inside it
replaced, a nested `DO`'s included. A `DO` that completes reports the cells it
replaced in its result, an enclosing `DO` adds those to its own list, and the host
adds them to the block's.

Without the undo, the body's earlier DDL stays in the session and the next
successful write persists it. A created table that the bracket's storage rollback
discarded answers `42P07` to its own name and `42P01` to an `INSERT`, and a dropped
one stays dropped. We saw this after a `kill -9` reopen too (`EDGES.md` row 249).

The guards are `crates/pglet/tests/do_block.rs`'s `a_failed_do_*` tests, over all
three backends and a reopen. The differential fixtures `ddl/do_failed_*_answers_*`
and `transaction/do_failed_in_block_*` pin only the error each failure gives, since
the harness stops there. What a failed `DO` put back can only be read where a
caught outer `DO`'s handler runs after it, and
`ddl/do_failed_inner_do_restores_its_cell_before_the_outer_handler` reads the
sequence cells that way. Every `DO` pays for the snapshot and level copies, even a
successful one (`docs/storage.md` § Known bottlenecks / scaling limits).

**The catalog isn't the only thing a statement changes, and staging only the
catalog can produce the same mismatch in the other direction.** `CREATE TYPE` /
`DROP TYPE` over an enum that a live column declares also rebuilds every storage
index spec on that enum's tables, and that loop is a second place a statement can
half-apply. If `Schema.enums` is staged back after the loop has already moved the
specs of the indexes it got through, the executor reads the old order while storage
enforces the new one. Then a unique partial index accepts a duplicate that 18.6
refuses. It's silent: `(2 rows)`, where the same script without the failing
statement raises `23505`. The same rule applies here as for the catalog, and the
loop follows it by unwinding what it moved (`docs/enums.md` § A copy that outlives
its statement). For a new DDL step, don't only ask *does every check run before the
catalog write*. Also ask *what else does this statement change, and can it put that
back*.

**A walk of the whole catalog isn't the statement's own work, so it's refused
before the statement runs instead of inside it.** A write over a persisting backend
encodes the catalog next to its data. That's the B+tree, on file or in memory,
unless it was built for one connection; memory storage encodes none. A `BEGIN`, a
`SAVEPOINT` and a block's first write clone the catalog, and a rollback drops one.
A stored expression adds a level to each of those walks. So a catalog built where
the thread's depth bounds were larger (another thread, a limit since lowered, a
budget since narrowed) is one this thread can't walk. The `Clone` and the
`Drop` can't fail at all, so on such a catalog they would abort the process, and
the encoding would refuse with `54001` only after the statement had done its work.

So every host checks at each statement's **entry** whether this thread can walk
this catalog (`pglet_sql::depth::check_catalog_walks`, over
`pglet_sql::depth::CatalogDepth`), and answers `54001` before any of the statement
runs, leaving the session, the store and the open block as they were. The same
check runs where an expression is stored (`pglet_sql::depth::check_stored_expr`),
from deeper in the stack than any entry, so a thread that accepts an expression can
still walk its catalog afterwards. Inside the statement, what's left is ordering:
the encoding still runs before the storage commit publishes anything
(`pglet::autocommit_publish_bytes`), so a failure there discards the staged writes
instead of reporting an error for a write that went through.

**The same route's *sequence* is the case where "run every check before the
mutation" isn't enough, because the mutation is an input to a check.** An
`ADD COLUMN`'s `SERIAL` / identity / `DEFAULT nextval('x')` has to exist for the
backfill to draw from, and the backfill is what the `23502` gate, the index build's
checks and the CHECK and FK validations judge. So registering it first puts a
catalog write ahead of three violations (`23514` / `23505` / `23503`), and each of
them then raises over a **durable phantom sequence that keeps the failed backfill's
own advances**. With two rows, `ADD COLUMN c SERIAL CHECK (c > 100)` answers
`23514`, then `nextval('t_c_seq')` answers `3` and the name is `42P07` forever,
where 18.6 answers `42P01` and leaves the name free.

The fix is to **stage** it: a local `Sequence` that the backfill advances, inserted
into `schema.sequences` only inside the mutation block (`executor::ddl`'s
`staged_sequences`). The general form is worth remembering: when a check needs a
catalog fact the statement is creating, compute the fact locally instead of moving
the check after the write.

The `23502` gate is in that list as a gate, not as a violation. A sequence-bearing
default resolves to an `int8`, which no coercion turns into NULL, so
`ADD COLUMN c SERIAL NOT NULL` **succeeds** on both engines, and a type that can't
take the value fails with a cast error instead. Reaching the `23505` arm takes an
unusual spelling, `NUMERIC(3,-1) DEFAULT nextval(…) UNIQUE`, whose backfilled
`1, 2, 3` all round to `0` (checked on both engines).

Unlike the phantom column, this half is **backend-independent**. It's a catalog
residue with no storage counterpart, so the memory double reproduces it exactly.
One asymmetry in it matters: a sequence the statement did *not* create is advanced
**in place**, because `nextval` is non-transactional, and staging a copy of a live
counter would walk it back over ids already issued.

### A failed index build

**A `CREATE INDEX` can't put its storage build ahead of its catalog write, so the
catalog write goes after the build instead.** This is the same rule from the other
end. `execute_create_table` moves its *checks* above the mutation, but `CREATE INDEX`
can't, because the build isn't a check. It's the statement's actual work, it can
fail, and `IndexSpec` is keyed by the OID, so the spec can't even be assembled before
a number is chosen. So `execute_create_index` and `create_unique_index` read the
number with `next_object_oid` (which doesn't advance the counter), build the spec,
call `storage.create_index`, and write `schema.next_oid` and `schema.create_index`
only once that has returned. `execute_create_type` reads the counter the same way,
for the same reason.

The counter write isn't there for distinctness within a statement.
`next_object_oid` re-seeds from the live catalog max, so `create_unique_index`'s two
looping call sites read past the index the previous iteration registered either way.
What it gives is monotonicity across a `DROP`, where the max no longer holds the
number, and a reused index OID would put two logical indexes on one storage band.

**What the build can still raise.** The refusals that match an 18.6 error are raised
before the storage build, by the executor's own scan of the rows the build would
enter (`constraints::scan_index_build_sizes` and the duplicate check between its
halves). These are 18.6's three `54000`s, the build's `23505`, and any error a key
expression or a partial predicate raises on a row. For example,
`CREATE TABLE w (a INT); INSERT INTO w VALUES (0),(1); CREATE INDEX w_ix ON w
((10 / a))` is `22012` on both engines, raised by the entry sizing, which evaluates
the key for each row.

A key **type** refuses nothing, because every value has a key (`docs/indexes.md`
§ A float key, § A container key). The one shape pglet refuses for its type, a
non-unique B-tree over a single bare container column, is refused by
`validate_btree_key_type`, a gate before the build. A partial index's predicate
doesn't add another case: the predicate decides *membership*, and it's the admitted
rows' **key** that gets encoded.

What's left inside the storage build is its own bound and the device. The bound is a
key longer than a B+tree page can hold. The executor's scan checks that for a B-tree
key and a GIN array element, but not for a `jsonb` GIN **token**:
`CREATE INDEX … USING gin (j)` over a row holding a string of 5 000 characters is
`54000` from the back-fill, where 18.6 hashes the token and builds (`EDGES.md`
row 192). The device can also refuse a write.

**Putting the catalog write after the build only works because
`Storage::create_index` is all-or-nothing.** The trait states that contract, and both
backends meet it by registering last: `MemoryStorage`, and `BtreeStorageWrapper`,
which derives its def, back-fills, applies the batch, and only then inserts into
`index_defs` / `table_indexes`, like its own `rebuild_index` does. Without that half,
writing the catalog after the build would remove the visible residue and keep the
invisible one. Here's what the two halves together prevent a failed build from
leaving behind:

- **Catalog.** The index is in `pg_indexes` and its name is `42P07` forever. Through
  `create_unique_index` it's **constraint-marked**, so `DROP INDEX` refuses it with
  `2BP01`, and pglet has no `ALTER TABLE … DROP CONSTRAINT`, so it can't be fixed
  short of `DROP TABLE`. It isn't only in memory either: one later ordinary statement
  serializes the whole catalog, so it survives a restart on a `--data-dir` store. The
  store's own dump then emits the failing statement, giving a database that reloads
  into stock Postgres but that pglet refuses.
- **Storage.** On the B+tree backend the def stays and the table becomes unwritable:
  an `INSERT`, `UPDATE` or `DELETE` whose row reaches the encoder raises. On
  `--storage memory`, which registers last, only the catalog half exists, and it leaks
  `22000 execution error: index not found: <oid>` from the accelerated reads and from
  every write that probes a unique index.
- **The OID.** Burned on every build failure. The allocation also comes after the
  catalog checks and the `IF NOT EXISTS` skip, so neither of those burns one.

The guards:

- `crates/pglet`'s `a_failed_create_index_build_leaves_no_phantom`,
  `a_failed_create_index_build_inside_a_block_leaves_nothing_to_roll_back` and
  `a_rejected_create_index_leaves_the_oid_counter_alone` cover the catalog and counter
  halves, with `an_index_oid_is_never_reused_after_its_index_is_dropped` for what the
  counter write itself gives.
- `index_registry_hygiene.rs`'s three cover the storage registry, the reopen, and the
  one question the registry can't answer.
  `a_failed_back_fill_write_leaves_no_readable_entry_in_the_band` drives an injected
  I/O error through both halves of `apply_batch`'s write tail and reads the band back
  through a later index that takes the same id.
- `tests/fixtures/ddl/failed_gin_index_build_leaves_the_table_writable.sql` covers the
  storage half from SQL. `failed_expression_index_build_leaves_the_table_writable.sql`
  holds only the statement-level property, since its `22012` is raised by the build's
  entry sizing, before the storage build.

Those fixtures need `-- @no-txn`, because pglet defers index maintenance inside a
transaction block, and the harness's own wrapper makes them pass with the defect
present. Their `CREATE INDEX` also has to be caught by a
`DO $$ … EXCEPTION WHEN others … $$`, since `run_setup` propagates the first error.
That handler removes the catalog half, which is why the catalog guards are unit tests.

**The SQL guards of the storage build all depend on one trigger, and one route has
none.** The statement that fails inside the storage build is the `jsonb` GIN token
above. `the_gin_token_refusal_is_the_storage_builds_own` (`crates/pglet`) checks that
it's the build that refuses it, using an order that only the build's position gives,
against the render probe, which is the last gate before it. That way the guards using
it don't go blind on the day an earlier gate learns to refuse the token. It only
reaches `execute_create_index`.

A **declared constraint's** index (`create_unique_index`: an inline `UNIQUE` /
`PRIMARY KEY`, `ADD COLUMN … UNIQUE`, `ADD CONSTRAINT`) is a B-tree over bare columns.
Every refusal of one comes from the executor, and we found no statement that fails
inside its storage build. On that route a device failure checks the
catalog-after-build order instead (`index_registry_hygiene.rs`'s
`a_failed_constraint_index_storage_build_writes_no_catalog`, over
`ADD CONSTRAINT … UNIQUE` and `ADD PRIMARY KEY`).
`a_refused_constraint_index_leaves_nothing_on_the_add_column_and_pkey_routes` and
`a_refused_add_column_unique_index_leaves_the_table_whole` pin the refusals' position
**before** that route's build, using pglet's own page bound over an `int[]` key.

**This doesn't change** pglet's own refusals of DDL that 18.6 accepts: the page bound
(`EDGES.md` row 192) and the bare container B-tree (row 25). Each fails cleanly.

## A gate standing in for *no* Postgres error must run after every gate that stands in for one

That includes the gates that decide a Postgres *success*. The error-order scales
below are all about reproducing Postgres's own precedence. This is the case where
pglet has a check Postgres doesn't have at all, and the rule is "put it last," not
"put it where the similar check lives." `CREATE INDEX` has one: pglet refuses a
non-unique B-tree over a single bare container column (`EDGES.md` row 25), while
Postgres indexes `jsonb` and arrays happily (`jsonb_ops`, `array_ops` are default
B-tree opclasses).

When that check was sequenced with the *real* method checks (`get_am_oid` and
`amcanunique`, which `DefineIndex` does raise before `index_create`'s name
conflict), it inherited their precedence. That turned
`CREATE INDEX IF NOT EXISTS <a taken name> ON t (jsonb_col)`, the standard
idempotent-migration idiom and a Postgres **success**, into a `0A000`. So
`validate_index_method` (Postgres's half, before the name) and
`validate_btree_key_type` (pglet's own, after the name and the skip) are two
functions. They're split by *whose error it is*, not by what they inspect, since
both read the key column's type.

### The mirror: a gate that *does* stand in for a Postgres error goes where 18.6 raises it

The case is `CREATE [UNIQUE] INDEX`'s **build**. 18.6's `index_create` builds the
index as its last step, and a duplicate key among the existing rows raises
`23505 could not create unique index "…"` with `DETAIL: Key (b)=(7) is duplicated.`
That's a different error from the per-row `already exists` one, with the same
SQLSTATE. An engine that registers the index and back-fills it without checking
creates an index that's never enforced.

**This is a divergence in the rows, not in the catalog.** 18.6 creates no
index, so the *next* duplicate row is written there, while an engine holding an
index it should never have built refuses that row with `23505`. Same statements on
two engines, different table contents: we saw `nd6` holding three rows in 18.6 and
two in pglet.

The build gate has to get three things right, and the first two are this page's
own rules read in the other direction:

- **Where.** Between the name conflict and `validate_btree_key_type`. The name
  conflict comes with its `IF NOT EXISTS` skip, which `index_create` owns too, so
  `CREATE UNIQUE INDEX IF NOT EXISTS <a taken name>` over duplicate rows reports
  success on **both** engines. `validate_btree_key_type` matches no 18.6 error at
  all. Over a `jsonb` key column with duplicate rows the order is visible: `23505` on
  both engines, where running the pglet-only gate first would have said `0A000`.
- **Before the first catalog write.** The counter write is a catalog change the
  autocommit path can't undo, and `schema.create_index` would be a durable phantom
  whose name is `42P07` forever. So the `Index` value is assembled *before* both,
  with a placeholder OID, so the gate can read the real thing without anything being
  written. (Both catalog writes also come *after* the storage build, see § A failed
  index build, so the gate runs ahead of a statement whose whole mutating half is at
  the end. Being ahead of the build is what this bullet needs, since the build is
  where 18.6 raises the `23505`.) The **type** keeps that placeholder out of
  storage: `PreparedIndex.index_id` is an `Option<u32>`, and the placeholder `0` maps
  to `None` (no real OID is below 16384). So the one reader that probes storage with
  it names the case and refuses loudly (`PreparedIndex::probe_index_id`, called from
  `occupancy_of`). A probe against index `0` would answer "no rows," and a
  uniqueness check reads "no rows" as "no conflict" and writes.
- **Through the write path's own key preparation.**
  `executor::constraints::validate_index_build_unique` calls `prepare_one_index` +
  `prepared_unique_index_key`, the same two functions `enforce_index_entries` calls
  per row. So a partial index's predicate, `NULLS [NOT] DISTINCT`, an expression key
  and a composite key are **prepared** at build time exactly as a later write
  prepares them, and the two can't drift apart. The cost is one extra full scan of
  the table plus a map holding every distinct key, paid once per
  `CREATE UNIQUE INDEX` (as `ADD CONSTRAINT UNIQUE` / `ADD PRIMARY KEY` pay it), and
  nothing for a non-unique index, which runs no such scan.

**The build's *occupancy* check reads committed rows plus its own.** Key
preparation is shared with the write path. The reading is the scan's own, and it
doesn't look at other connections. It doesn't need to, since no other
connection's block holds a staged write while the build runs: `CREATE UNIQUE
INDEX` takes the writer. On the native server, a build over a duplicate that
another connection's open block staged waits for that block and then refuses with
`23505 could not create unique index "g8_a"`, as 18.6 does after blocking on the
relation lock (`EDGES.md` row 103, closed). The embedded `Database` can't wait, so
it refuses the build with `55P03` (row 229).

**Which *equality* the scan uses is a second decision.** A validation scan asks
"would this index hold two entries it considers equal?", and the index's equality
comes from its **key encoding**, not from the `Value`'s structural equality.
`Value::encode_index_key` is order-preserving, so it normalizes what ordering can't
tell apart, and `postcard` doesn't. A scan comparing postcard bytes lets a
`numeric` column holding `1.5` and `1.50` pass `CREATE UNIQUE INDEX`,
`ADD CONSTRAINT UNIQUE` and `ADD PRIMARY KEY`, where 18.6 refuses all three
(`Key (n)=(1.5) is duplicated.`). After that, the engine's **own** write path
refuses the row already in the table, so the engine disagrees with itself.

`constraints::index_key_identity` is the one equality all three validation scans
use. It's keyed by `encode_index_key` under `encode_probe_key`'s presence tagging,
so a NULL can't collide with a present value whose encoding is empty. Two more
things follow: the scans agree with each other, and the `DETAIL` names the
**first** row in scan order, not the colliding one. That's what 18.6 names, and it
only differs when the equality treats two spellings as equal.

**Every key type is compared by its key**, `jsonb` and arrays included. Postcard
bytes are finer than 18.6's equality, so a postcard comparison would accept
`jsonb '1'` next to `'1.0'`. That pair, and `{"a": 1}` next to `{"a": 1.0}`, give
the build's `23505` on both engines
(`container_key/jsonb_create_unique_index_over_scale_duplicates_loud.sql`,
`container_key/jsonb_add_constraint_unique_over_scale_duplicates_loud.sql`), as do
`'0.0'::float8` next to `'-0.0'::float8` and a NaN next to a NaN of the other sign
(`float_key/unique_build_negative_zero_loud.sql`,
`float_key/add_unique_constraint_nan_loud.sql`).

The accept side is the half a build gate tends to get wrong. Distinct keys, a
partial unique index whose duplicates all fail its predicate, and two NULL keys
under the default `NULLS DISTINCT` are each **created** on 18.6, and each is pinned
next to the refusal. The one field pglet doesn't reproduce is an expression key's
`D`: 18.6 says `Key (lower(s))=(a) is duplicated.`, and pglet omits it, following
the same rule `enforce_index_entries` follows for a key it can't name truthfully.

**A second case is the identity clause's last two checks.** `ALTER COLUMN … ADD
GENERATED … AS IDENTITY` over a column not declared `NOT NULL` is 18.6's
`55000 column "c" of relation "t" must be declared NOT NULL before identity can be
added`, and over a column that has a default it's `55000 column "c" of relation "t"
already has a default value` (`ATExecAddIdentity`). Each of these prevents a row
divergence. The clause reads no rows, so accepting it and setting the flag would
declare `NOT NULL` over whatever the column holds, and an ordered index read would
then leave a NULL row out ([indexes.md](indexes.md) § Index probes are
exact-or-decline). And writing the identity's `nextval` over a default would drop
the default without a word: over a `SERIAL` holding `1`, `2`, the next row would
draw `1` again.

The first gate reads only the declaration, so it refuses over rows holding a NULL,
rows holding none, and an empty table alike. The second reads whether the column
has a default: a constant's, an expression's, a `nextval`'s, a `SERIAL`'s, or one
given by `SET DEFAULT`. `DEFAULT NULL`, a dropped default and one set to NULL count
as none, each checked on 18.6. `SET NOT NULL` and `DROP DEFAULT` are how you get
past them. The same two things as before decide where the gates go:

- **Where.** After the sequence's refusals, with the declaration's gate before the
  default's. 18.6 creates the identity's sequence before `ATExecAddIdentity` looks at
  the column. So over a column that fails either gate, the type's `22023`, a taken
  `SEQUENCE NAME`'s `42P07` (a table's, an index's or a sequence's) and a type
  name's `42710` each win. Over a column that isn't declared `NOT NULL` and has a default,
  the declaration's refusal is the one raised. An identity column, whose own
  `nextval` is a default in the catalog, is refused as already being an identity
  column, not for that default. Each pair is checked on both engines
  (`ddl/add_identity_nullable_column_*`, `ddl/add_identity_default_column_*`,
  `ddl/add_identity_over_an_identity_column_is_already_identity_loud.sql`,
  `crates/pglet/tests/identity_declaration.rs`), after the relation's `42P01` and
  the column's `42703`.
- **Before the first catalog write**, which is the sequence's registration. A
  refused statement leaves no sequence under the name it would have minted, on a
  file store across a reopen as well as in memory.

What reaches the first gate is the declaration that both column-creating
statements give a column. A `SERIAL` column is declared `NOT NULL` by `ADD COLUMN`
just as by `CREATE TABLE` (`parser::ddl`'s `ParsedColumn::nullable`, one derivation
for both), and each row the table already holds draws its value before the
declaration is written. So the statements that turn a serial column into an
identity one (`DROP DEFAULT`, then the clause) are accepted over an added column
just as over a created one (`ddl/add_identity_over_an_added_serial_column.sql`).

Some things this statement's order doesn't reproduce:

- A column that's already an identity column is `55000` here before a taken
  `SEQUENCE NAME`, where 18.6 raises the name's `42P07` or `42710` first.
- The sequence options 18.6 judges while creating the sequence (`START WITH` outside
  `MINVALUE`…`MAXVALUE`, `INCREMENT BY 0`, each `22023`; a repeated option or an
  empty list, `42601`) aren't judged here (`EDGES.md` rows 215 and 315). So over a
  column that fails a gate, such a statement gets the gate's `55000` instead of the
  option's refusal, and over one that passes both it's accepted.
- A system column (`ctid`, `xmin`) is the type's `22023` in 18.6 and the column's
  `42703` here.
- A default that 18.6 keeps as a cast of NULL to a type other than the column's
  (`c bigint DEFAULT NULL::int`) is no default here, so the clause over it is
  accepted where 18.6 refuses it (`EDGES.md` row 47). A NULL cast again to the type
  it already has (`c int DEFAULT NULL::int::int`), which 18.6 keeps no default for,
  is a default here, so the clause over it is refused where 18.6 accepts it (the
  same row).
- 18.6 accepts the clause and the declaration in one statement,
  `ALTER TABLE t ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY, ALTER COLUMN c SET
  NOT NULL`, performing the `SET NOT NULL` first. That's `42601` here.

## A gate that reads the catalog is blind inside the statement that is building it

The `CREATE TABLE` path is where every sequence gate is called. The rule above and
the declared-name rule ([names.md](names.md)) share a hole of exactly one shape.
`resolve_implicit_sequence_name` and `ensure_sequence_relation` decide by reading
`Schema`, and a `CREATE TABLE`'s own table isn't in `Schema` yet, because nothing is
changed until every check passes (itself a hard requirement, from the phantom rule
above). Without a gate, `CREATE TABLE dz (id INT GENERATED ALWAYS AS IDENTITY
(SEQUENCE NAME dz))` and `CREATE TABLE dz (id INT DEFAULT nextval('dz'), b INT)` both
put `dz` in `tables` and `sequences` at once. After that, `nextval('dz')` answers
`42809` while `INSERT … VALUES (DEFAULT)` answers `1`, and the database's **dump
silently loses the table and every row it held**. That happens the same way in stock
Postgres and in pglet's own restore (`relation "dz" already exists` →
`sequence cannot be owned by relation "dz"` →
`column "id" of relation "dz" does not exist`, and the `COPY` has nowhere to land).
The constraint-index namer handles this shape with a seed (`ConstraintNames::new`
puts the table's own name in `claimed_relations`). The sequence family handles it
with the gates below.

Those gates have to get three things right, and none of them is a matter of taste:

- The two halves use **different SQLSTATEs**. The declared name is `42P07` (the
  collision is the table's, per the fifth position in [names.md](names.md)), and the
  user default is `42809`. Postgres accepts that `CREATE TABLE` and raises `42809` at
  the first write, so `42809` is the consistent code for pglet's documented DDL-time
  over-rejection.
- The declared half has to be tested at the table's own `42P07`, **not** by seeding
  the name into the loop. Otherwise the wrong relation is named, at the wrong
  position.
- The user-default half has to run in its **own pass, after** the table's `42P07`.
  Postgres cooks a default once the heap exists, so an existing table with the
  statement's name and a colliding `SEQUENCE NAME` on any column both outrank it. The
  same pass split also stops a sibling `DEFAULT nextval('<a SERIAL column's
  sequence>')` from re-registering a bare record over the one that carries the
  column's `START WITH` and its ownership edge. `CREATE TABLE k (b INT DEFAULT
  nextval('k_a_seq'), a INT GENERATED ALWAYS AS IDENTITY (START WITH 100))` is
  accepted by both engines and answers `100`, where a re-registered bare record
  would answer `1`.

The general question for any new gate on this path is *which of the names this
statement is creating can it not see?*, and you have to ask it of **every** gate.
Asking it only of the table name leaves the same hole one layer over. A
user-default sequence that's gated for kind but never **claimed** is invisible to
the constraint-index namer too, and `CREATE TABLE t1 (a INT DEFAULT
nextval('t1_pkey'), b INT PRIMARY KEY)` would mint the index right onto the
sequence and brick `t1` from the post-mutation backstop. Since the sequence is
claimed, both routes disambiguate to `<name>1`, byte-identical to Postgres with that
sequence created beforehand.

Two rules follow. First, the name a statement will register belongs in the
in-flight namespace at the point Postgres's own catalog would hold it (for a cooked
default, `AddRelationNewConstraints`). Second, a **deliberate over-rejection has to
be the last check**, because it stands in for an error Postgres raises later. If it
ran earlier, pglet's DDL-time `42809` would outrank a duplicate CHECK name, an
ill-typed CHECK, a taken constraint-index name and a missing FK parent, all of which
Postgres does raise on that statement. The gate runs after the whole constraint
block on both routes, and fixtures pin the pairs.

## Gating a stored expression is a DDL-time concern

An operator gate isn't purely additive when the engine *persists* expressions. A
CHECK constraint, index predicate, or index key expression that the gate rejects at
maintenance time would turn an accepted DDL into an **unwritable table**. So
`CREATE INDEX` / `CREATE TABLE` / `ALTER TABLE` validate stored expressions up front
(`executor/ddl.rs`), and the failure lands where Postgres puts it.

The checks run in **one bottom-up, left-to-right walk**. Postgres resolves in source
order during parse analysis, and two separate whole-tree passes would let whichever
ran first win no matter *where* each error is. Over a table whose `a` isn't text,
`CHECK (a LIKE 'x' AND bogus > 0)` is `42883` while
`CHECK (bogus > 0 AND a LIKE 'x')` is `42703`, and no pass order gives both. (The
example depends on `a` not being text: give `a` a `text` type and `~~` accepts it,
so both spellings are `42703` on both engines and the pair tells you nothing. We
checked this on 18.6 with the `RETURNING` spelling of the same pair.)

The walk runs four checks:

1. **Binding.** A leaf that can never resolve here: `DEFAULT` (`42601`), a `$N`
   parameter (`42P02`), or a column the table lacks (`42703`). If accepted, each of
   these would fail *every* write.
2. **Static operand typing** over `Expr::infer_type` (`planner/legality.rs`, which
   stays a pure per-node rulebook; the caller does the traversal).
3. The **boolean-context gate** on each child its parent puts in a boolean context,
   applied as soon as that child's own subtree is done, not after the parent's.
   18.6's `transformBoolExpr` coerces one argument before transforming the next, so
   `CHECK (a AND b LIKE 'x')` is `42804 argument of AND` and
   `CHECK (b LIKE 'x' AND a)` is `42883`, and no pair of whole-subtree passes gives
   both.
4. A **representative-row probe**.

Together these are a strong filter, **not** a proof. A value-dependent error can
still show up at write time, as it does in Postgres. There's also a known hole that
*isn't* value-dependent: a typed constant whose input conversion fails
(`'1e39'::real`, `'zzz'::int`) is refused by 18.6's DDL, but the probe keeps only
operator-resolution outcomes, so the conversion's error is thrown away and the DDL
is accepted. That's the unwritable table this section exists to prevent, plus a dump
whose restore drops the constraint (`EDGES.md` row 301).

Above the walk, once it and the probe have passed, a stored *predicate* goes through
the clause-root gate named by its own `BoolContext`: `validate_stored_predicate`,
with `Check` for a CHECK and `Where` for a partial index's predicate. That's where
18.6's `cookConstraint` and `transformIndexStmt` put `coerce_to_boolean`: last,
after the whole expression is transformed. So `CHECK (a)` over an `INT` column is
`42804 argument of CHECK` at DDL on both engines.

There's also a known false *rejection*, shared by every gated family. A stored
expression is checked **as parsed**, without the planner's literal-coercion pass, so
a bare literal types as `text` instead of staying Postgres's `unknown`.
`CHECK (n = '5')` and `CHECK (d @> '{"k":1}')` are rejected here and accepted
there, and the fix is an explicit cast. Treating a literal as unknown would also
stop rejecting `CHECK (data->'k' LIKE 'x%')`, the NULL-intermediate shape the probe
can never see, given how it works, and the whole reason the static check exists. Fixing it
properly means resolving the stored expression against its table's scope before
recording it (`tests/fixtures/ddl/check_containment_*`).

**One more check belongs with those, because persisted expressions have a second
reader: the expression must *render*.** `pglet::dump` renders every persisted
expression through one `Result`, so an expression the deparser declines doesn't
just make its own table undumpable. It makes the **whole database** undumpable. All
four persisted-expression sites run that probe (`crate::deparse::render_expr`, last
in `cook_column_default` for a column `DEFAULT`, and `validate_stored_expr_renders`
for a CHECK, an index predicate and an index key part). That's what keeps
`DEFAULT now() - interval '1 day'` and `CHECK (t > now() - interval '1 day')` as
DDL-time `0A000`s instead of accepted databases that can't be exported.

The reason is that the parser carries an interval literal as an `Expr::Bound`, which
the deparser has no source spelling for, and `parse_interval_literal` is its only
live SQL-text producer. (`parser::ddl`'s `column_default_to_expr` has a second
constructor, but it's dead given its own classifier's range, and it's confined to
the already-probed `DEFAULT` site.) Without the probe, the gap is a common shape,
not an exotic one: `CREATE TABLE ck1 (t TIMESTAMPTZ, CHECK (t > now() - interval
'1 day'))`, which 18.6 accepts, makes `pglet_server dump` exit 1 for the whole
database. We found the same interval shape reaches every CHECK spelling (three
executor call sites serve them) and both index positions.

**The invariant is that *every accepted* expression renders, and that's a claim
about which paths reach the probe, not about the probe itself.** At the `DEFAULT`
site the probe is the cook's last step, so every early return above it skips the
probe, and an arm that returns there accepts something the probe never sees. For
example, an expression whose probe *evaluation* fails with a deferred code (`22012`,
`22003`) has no value to coerce. Returning on it would **accept**
`DEFAULT now() + interval '1 day' * (1/0)` and persist the very `Expr::Bound` the
paragraph above says is refused. Then `pglet_server dump` exits 1 for the whole
database, with every other table's schema and rows missing from the export, and
`information_schema.columns.column_default` reads SQL NULL for the column. That's
the same defect, just silent, because that view has no error channel.

The neighboring interval spelling that *does* evaluate is refused either way, which
is what hides a hole like this: a guard written against it passes with the bug
present. Both readers of a persisted default say that no accepted DDL reaches the
declining arm, and the probe is the only thing that makes either statement true. So
a *reader*-side claim of unreachability has to name the writer-side gate it depends
on, or nobody owns the claim.

**Where the render probe runs follows from which error it stands in for. That's
why it's a separate function from the checks above instead of their last step, and
why it's a whole-*statement* pass instead of a per-constraint one.** Those checks
reproduce 18.6 errors raised in parse analysis, before `index_create`'s name
conflict, so they run where they do. Each is positioned *against* a Postgres error,
so each has a position. The render probe reproduces **nothing**, since 18.6 accepts
every expression it declines, so it has no position to reproduce. Its placement
comes entirely from the "a gate standing in for no Postgres error runs last" rule,
*including against the gates that decide a Postgres success*.

It runs at the end of every statement that persists a stored expression, right
before that statement's first catalog write (`validate_check_exprs_render` on the
three CHECK routes, `validate_index_exprs_render` after the index name conflict and
the `IF NOT EXISTS` skip). The phantom rule bounds it from the other side.

There's one deliberate exception on the `ADD COLUMN` route. A **live**
(pre-existing) sequence that the discarded backfill drew from keeps its advances,
written back before the probe. That's the in-place `nextval` rule
([sequences.md](sequences.md)): walking a live counter back re-issues ids, and
18.6's counter moves on its equivalent statement too. So "before the first catalog
write" holds exactly for every catalog fact except a live counter cell. The same
statement's *staged* sequence is discarded whole.

**We tested both halves against the placement each one rules out.** If the probe is
folded into `validate_stored_expr` on the index routes, it turns
`CREATE INDEX IF NOT EXISTS <a taken name> ON t ((iv + interval '1 day'))`, the
idempotent-migration idiom and a **success** on 18.6, into a `0A000`. That's the
same trap `validate_btree_key_type` was split out of `validate_index_method` to
avoid. If it runs inside the per-constraint cook on the CHECK routes, it pre-empts
that block's *errors* in the arrangements we tested across the three routes: a taken
constraint-index name (`42P07`), a missing FK parent (`42P01`), a later duplicate
CHECK name (`42710`), an `ADD CONSTRAINT`'s existing-row violation and an
`ADD COLUMN`'s backfill violation (`23514`). Each is `0A000` there against 18.6's
own code, and each agrees at the whole-statement position.

**Whether source order matters depends on the gate.** The duplicate name shares the
cook-then-name loop, so source order does decide it. The constraint-index name and
the FK parent are resolved by *later blocks*, with the whole CHECK loop ahead of
them, so a per-constraint probe diverges for those in **both** orders, and no
arrangement of them works as a control. Which gate you test a divergence against
decides whether order is even the variable, and that's a fact about the call graph,
not about the SQL. On the `DEFAULT` twin (`EDGES.md` row 47), the probe is still the
cook's last step and the residual remains there on its own. The same three kinds
diverge for it in our tests, and the same move would close it.

The gate closes off the *producers*, which isn't the same as fixing existing
*state*. The gate leaves the persisted shape as it is, so no store magic marks it,
and a database written by a build without the gate can hold an interval CHECK that
reopens, reads and writes, but can't dump. It fails **loudly** there instead of
emitting a guessed spelling or silently dropping the constraint, which is what
`a_legacy_stored_expression_that_cannot_render_fails_the_dump_loudly` pins over a
hand-built catalog. Rendering the interval literal, which fixes the over-rejection
from the other end, is the real repair and is separate work.

## A constraint's key list is validated by the constraint, not by the index it builds

There are four independent facts here, and none of them follows from the others.
The easy mistake is to notice one and generalize it into a rule about name lists:

- A `PRIMARY KEY` / `UNIQUE` key list **refuses** a repeated column with `42701`,
  using the constraint's own message (*column "a" appears twice in primary key
  constraint*, or *unique* for the other kind).
- The index that constraint builds has no such rule, so `CREATE INDEX t_i ON t
  (a, a)` is accepted by both engines, rows inserted afterwards are indexed under
  both segments instead of refused, and `pg_indexes.indexdef` reads both back.
- A foreign key's **referenced** list refuses a repeat with `42830`. The SQL standard
  forbids it, and 18.6's `transformFkeyCheckAttrs` adds the implementation reason:
  matching the list to an index would be ambiguous about which opclass pairs with
  which key.
- A foreign key's **referencing** list accepts a repeat, so `FOREIGN KEY (a, a)
  REFERENCES p (x, y)` builds and then enforces the pair `(a, a)` against the
  parent's `(x, y)`.

`validate_key_columns` handles the first, and the second holds because the
`CREATE INDEX` path never calls it. `validate_referenced_columns_distinct` handles
the third, and it's given only the parent's list, so the referencing list can't
reach it. The two acceptances are guarded as **accept-side controls**
(`ddl/create_index_key_may_repeat_a_column.sql`,
`ddl/foreign_key_referencing_list_may_repeat_a_column.sql`), because a gate written
over "a list of column names" instead of over the constraint fails there and
nowhere else.

**What the referenced-list refusal outranks, and what it never reaches.** It runs
before the covering-key lookup, so `FOREIGN KEY (a, b) REFERENCES p (y, y)` over a
parent with no key on `y` gets *must not contain duplicates* instead of *there is no
unique constraint matching given keys*. Both are `42830`, so the position decides
the message, not the code.

It also runs over a **derived** list. An omitted `REFERENCES p` resolves to the
parent's primary key, and the check reads the same `parent_columns` whether that
list was written or derived. A derived list comes from a key list in which the
first rule above refuses a repeat, so only a store written by a build without that
gate can supply one. That's a reason to keep the check on both routes, not a
reason to exempt the derived one.

What it does *not* reach is the unequal-arity spelling. `FOREIGN KEY (a) REFERENCES
p (x, x)` is refused one layer earlier by the parser's own arity check, which runs
before every execute-time verdict, including the parent's own. For example,
`CREATE TABLE c (a INT, FOREIGN KEY (a) REFERENCES <no such table> (x, x))` is
`42830` here and `42P01` on 18.6. This gate neither causes nor closes that
divergence (`EDGES.md` row 275).

**The PK/UNIQUE refusal's position differs by route, depending on which pass the key
*resolution* belongs to.** 18.6 tests each key against the keys already accepted,
inside `transformIndexConstraint`'s per-key loop, with the resolution above it in
the same body, but it skips this entirely when `cxt->isalter`. So on `CREATE TABLE`
the two verdicts interleave (`PRIMARY KEY (a, a, nosuch)` is `42701`,
`PRIMARY KEY (a, nosuch, a)` is `42703`), while on `ALTER TABLE … ADD CONSTRAINT`
the repeat wins even where no key resolves at all (`PRIMARY KEY (nosuch, nosuch)` is
`42701`), and the key column's `42703` comes later from `DefineIndex`. pglet
reproduces that by taking the column list as an `Option`: `Some` on the
`CREATE TABLE` route, `None` on the `ALTER` one, whose caller supplies its own
`42703` after the call returns. § Which error wins has the rest of the arrangement,
and `EDGES.md` row 39 has the precedence pairs we tested.

## Which error wins is part of the contract, at three scales

Postgres resolves a statement in source order during parse analysis, so when two
things are independently wrong, the *first by position* is reported. pglet
reproduces that at each scale separately, because none of them follows from the
others.

**Within an expression**, one bottom-up left-to-right walk decides operand legality
and branch unification per node (`planner/coercion.rs`, and the same shape in
`executor/ddl.rs` for stored expressions). Two whole-tree passes can't do this,
since whichever runs first wins no matter where its error is. The literal
conversions are such a pass: they run over the whole expression before its name
binding (`EDGES.md` row 327).

**Between the clauses of a statement**, each DML lowering visits its clauses in
`analyze.c`'s order. INSERT resolves its row source, then `ON CONFLICT`, then
`RETURNING`. UPDATE resolves `WHERE`, then `RETURNING`, then the `SET` list. A
missing target relation short-circuits all of it with `42P01`. That's why
`ON CONFLICT` is lowered in the planner instead of where it's parsed.

**Below that order there's a later tier**, for a gate that stands in for a check
18.6 makes in the *rewriter* (`rewriteTargetListIU`) instead of in parse analysis.
It runs after every clause of its statement is lowered **and** after the correlated
re-walk that closes a nested sub-plan's binding gate, so a clause error raised at
either of those places outranks it.

**That tier only orders pglet's own pipeline**: the verdicts pglet reaches while it
lowers, and nothing else. An error that 18.6 raises in parse analysis and pglet
raises at *execution*, like an unresolvable relation's `42P01`, isn't in that set.
Which of the two answers a statement that has a repeat depends on whether that
statement reaches the gate at all, and that's a property of the gate's call site,
not of the error. The same `RETURNING (SELECT 1 FROM nosuchrel)` skips the `UPDATE`
`SET` list's gate, so pglet's own `42P01` answers over a matching row and nothing
answers over none. But it runs the `ON CONFLICT DO UPDATE` list's gate, whose call
site has no such skip, so its `42601` answers. We haven't enumerated which
statements reach which gate, and we don't claim a bound for it.

The duplicate-target refusal on an `UPDATE`'s `SET` list is one gate in that tier.
It comes before the `GENERATED ALWAYS` identity guard next to it, because that's the
order of `rewriteTargetListIU`'s own two loops. The one on an
`ON CONFLICT DO UPDATE`'s list is another, after everything the `INSERT`'s own
target list checks. Which gates are and aren't in the tier here, and the residuals
that leaves, are in [names.md](names.md) § A statement's target column list is a
set.

**Between protocol phases**, the extended protocol's `Parse` raises a plan error
instead of deferring it, as Postgres does, but only for errors the *planner* raises.
In most of the spellings that reach it, a **missing relation isn't one of them**.
pglet resolves the target at execute time, so `Parse` and `Describe` both succeed on
`SELECT * FROM no_such_table`. `Describe` answers with an **empty
`RowDescription`** (`fields=0`), a made-up descriptor for a table that doesn't
exist, and the `42P01` lands at `Execute`, where Postgres raises it at `Parse` and
fails the client's `prepare` outright.

Which spellings defer is a matter of tested membership, not of being a relation,
and there are four outcomes, not two. A schema-qualified or star-qualified one
raises here at `Parse` too. A `HAVING` one gets pglet's own `0A000` at `Execute`
instead of the lookup. And a reachability-silenced one raises nowhere at all. The
list is in `EDGES.md` row 29.

What decides `NoData` versus a `RowDescription` isn't that list but the engine's
one descriptor rule (`row_description`: **`NoData` exactly when the statement
returns no result set**, meaning DDL, transaction control, a DML with no
`RETURNING`, and the branches that produce no plan at all). A *scan* always returns
one, so the `SELECT *` above gets a `RowDescription` however narrow its expansion.
The test is whether there's a result set, never the column count. Reading the count
would answer a zero-column `SELECT *` with `NoData` and drop its rows on the simple
leg, which is `EDGES.md` row 59 (**closed**). Rows 29 and 59 are two records of that
one rule, and neither is the other's source.

**The descriptor rule isn't the only deferral.** The eval-time gates listed here
defer the same way, tested over an ordinary table with no missing relation in it:
`sum(<text>)` and `avg(<text>)` at `42883`, `<text> + <int>` at `42804`, an
aggregate in a `WHERE` at `0A000`, and a per-row `1/0` at `22012`. pglet answers
`1 t T 2 T E Z` where 18.6 answers a bare `E Z`, or `1 t T E Z` for the `1/0`, which
*its* planner constant-folds, so that refusal lands one message later, at `Bind`.
The parser's and the plan-time binder's own refusals (`42601`, `22P02`, `42703`,
`42702`, `42704`, `42803`) do land at `Parse` on both engines.

**"Defers" is itself too broad.** Where evaluation never reaches the gate, the
refusal isn't late but **absent**: `1 t T 2 T C Z`, or the same with `D` rows before
the `C` where a row is projected, with no error on either leg. Which spellings reach
it is an **enumeration**, not a rule this doc restates. The governing statement is
`EDGES.md` row 56's *evaluation reaches the expression, or it does not*. The
silencers we found, the controls that bound them (`LIMIT 0` silences nothing; the
relation lookup survives every row-count silencer), and the **third** direction,
where pglet is the louder engine, are in `EDGES.md` row 60.

Here's the trap. A clause with no gate looks "correct" because its neighbor's
error wins by default, so *adding* a gate changes statement-level error precedence
every time.

**Between two guards on one statement** it works the same way.
`ALTER TABLE … DROP COLUMN` checks FK dependents *before* the index position-drift
guard. A referenced column always has a covering unique index, so otherwise the
guard would always answer first, with `0A000` where Postgres says `2BP01` and names
the constraint.

**Between the constraints of one `CREATE TABLE`** it comes up again, and the shape
isn't obvious from the SQL. Postgres attaches the `CHECK`s as the heap is created,
builds the constraint indexes, then adds the foreign keys, each with its *name*
checked before its parent relation is resolved. So a duplicate name beats a missing
parent on the same FK (`42710`), a missing parent on an *earlier* FK beats a
duplicate name on a later one (`42P01`), and a CHECK written *after* an FK still
gets its name checked first. Three fixtures pin the three arrangements, and none
follows from the others.

**Inside one foreign key**, the order is `ATAddForeignKeyConstraint`'s: the
referenced relation (`42P01`), each list's columns (`42703`), the referenced list's
repeat and the covering key, then the arity (`42830`), and the column pairs' types
(`42804`) last. So a key's `42804` outranks a *later* key's duplicate name or
missing parent, and loses to those same two errors on its own key.

pglet's parser refuses a *written* list's arity mismatch once it has read the key's
clause. That clause's own `MATCH` and `ON DELETE` / `ON UPDATE` refusals come first
(`MATCH FULL`'s and `SET DEFAULT`'s `0A000`, a malformed action's `42601`). So the
arity refusal comes before all of the errors above, before anything the parser
would refuse after that clause, before every verdict past the parse, and before an
`IF [NOT] EXISTS` skip: `FOREIGN KEY (a, b) REFERENCES nosuch (k)` is `42830` here
and `42P01` there (`EDGES.md` row 275). The arity the resolver checks is an omitted
list's, resolved to the parent's primary key.

On `ADD COLUMN`, where the test goes relative to the column's own index build
depends on whether 18.6 rewrites the table. The key is added in the pass after the
index's. The index build runs in that pass without a rewrite, and is deferred into
the rewrite with one. So the test comes after the build for a constant default, and
before the build and the rewrite's `NOT NULL` and `CHECK` verdicts for a
per-row-volatile one (`EDGES.md` row 195).

**Between one constraint and the next**, one scale smaller, it comes up again.
`AddRelationNewConstraints` cooks each constraint and *then* names it, one
constraint at a time. So constraint 2's duplicate name beats constraint 3's
ill-typed expression (`42710`), while a duplicate name on constraint 2 *itself*
loses to constraint 2's own bad expression (`42883`). No arrangement of one
whole-list type pass and one whole-list name pass gives both. It has to be one
interleaved loop, the same shape the expression walk needs one scale further down.

**Between parse analysis and execution** it matters most, because the split can't
be seen in the statement text. `CREATE TABLE`'s index constraints are resolved in
`transformIndexConstraints`, so a second `PRIMARY KEY` (`42P16`) outranks the
duplicate CHECK name, the missing FK parent, the ill-typed CHECK, *and* the
already-existing table in the same statement. `IF NOT EXISTS`, tested at the top of
the same pass, outranks it in turn. On the `ALTER` paths the same check is inside
`DefineIndex`, **before** `index_create`, so there `42P16` beats the name checks but
the column resolution beats it. `EDGES.md` records what's still outside the order.

**Within `CREATE TABLE`'s own parse analysis** it splits once more, and the
`42P16`-beats-execution rule does *not* decide this layer. Column types resolve in
the `tableElts` walk (`transformColumnDefinition` → `transformColumnType`), so
`42704` outranks both `42P16` and the key-column `42703`. The modifier refusal
(`42601 type modifier is not allowed for type "…"`) goes with it, since both are
verdicts of the one `resolve_column_type` call that walk makes per column, left to
right. Then `transformIndexConstraints` runs **per constraint**, testing the
multiplicity at the top of `transformIndexConstraint` and resolving that
constraint's key columns further down in the same call. So
`PRIMARY KEY (nosuch), PRIMARY KEY (a)` is `42703` while
`PRIMARY KEY (a), PRIMARY KEY (nosuch)` is `42P16`, and no arrangement of one
whole-list count and one whole-list column check gives both. It's the same
interleaved shape as the CHECK cook-then-name loop.

**And once more inside one key list**, the scale `validate_key_columns`'s own doc
comment is written at. The repeat test comes *after* each key's own resolution in
the same loop body, so `PRIMARY KEY (a, a, nosuch)` is `42701` and
`PRIMARY KEY (a, nosuch, a)` is `42703`. The two routes disagree because of which
pass the resolution belongs to, not because of an ordering choice:
`transformIndexConstraint` skips it entirely under `!cxt->isalter`, so on
`ALTER TABLE … ADD CONSTRAINT` both spellings are `42701` and the key column's
`42703` comes later, from `DefineIndex`. That's three positions down the same
statement, and none of them can be derived from the one above it.

**`ADD COLUMN` orders the type the other way around, and that asymmetry comes from
18.6.** There `check_for_column_name_collision` runs *before* the
`BuildDescForRelation` that resolves the type. So the relation's `42P01`, the
column's `42701` and the `IF NOT EXISTS` skip each outrank the verdicts that *type
resolution* reaches. The `42701` applies in **both** of that function's arms: the
user column's *already exists*, and a **system** column name's own message, which
has no `if_not_exists` test at all. The type-resolution verdicts are `42704`, the
modifier `42601`, the identity `22023`, and the **carrier's** own `22023` next to
the `22003`/`22P02` of the `int4` coercion that comes before it
(`ADD COLUMN b char(0)` / `b numeric(0)` / `b numeric(1.5)` all lose to a missing
relation and to the skip, checked on both engines). On `CREATE TABLE`, by contrast,
the `tableElts` walk outranks even the `42P07`. We checked both orders on both
engines, and pglet mirrors the asymmetry instead of normalizing it (`EDGES.md` rows
119's `ADD COLUMN` table and 124).

**A type-name verdict that the *grammar* raises sits outside that resolution and
beats all three**, because bison reduces the production before either the collision
check or the type resolution runs. `opt_float`'s `22023` is one (`gram.y`'s
`opt_float` action `ereport`s it in the rule body), and it lands before the column's
`42701`, before the relation's `42P01`, and before the `IF NOT EXISTS` skip, which
is a Postgres **success**. On both engines,
`ALTER TABLE t ADD COLUMN IF NOT EXISTS b float(0)` over an existing `b` is
`22023 precision for type float must be at least 1 bit`, and
`ALTER TABLE nosuchtable ADD COLUMN b float(0)` is that same `22023` instead of
`42P01`.

It isn't the only one. The temporal modifier grammar's `42601`s land the same way:
`ALTER TABLE nosuchtable ADD COLUMN b timestamp(3,1)` and `… b interval(-1)` are
each `42601` on both engines. **So does the character length**, read by the same
one-`Iconst` reader: `ALTER TABLE nosuchtable ADD COLUMN b char(3.5)` and
`ALTER TABLE t ADD COLUMN IF NOT EXISTS b char(2147483648)` over an existing `b` are
`42601` on both engines. Each has its own fixture
(`types/refused_typmod_grammar_length_beats_missing_table_loud.sql`,
`types/refused_typmod_grammar_length_beats_add_column_skip.sql`), since sharing the
order doesn't mean sharing the evidence.

Compare a spelling whose refusal *does* need the resolved type: `int4(3)` loses to
all three, and `ALTER TABLE t ADD COLUMN IF NOT EXISTS b int4(3)` over an existing
`b` **succeeds** on both engines (18.6 raising a `skipping` notice), because
`typenameTypeMod` runs after them. Same shape, opposite winner for each spelling,
decided by which phase the refusal lives in. `docs/coercion.md` § A canonical type
name decomposes once states the rule, and `EDGES.md` rows 116 and 124 are the
record.

**A gate in the wrong *phase* of pglet's own pipeline can't be placed correctly at
all**, which is why a column `DEFAULT`'s verdicts are raised in the executor. Every
refusal 18.6 raises for one (a subquery, an aggregate, a window, a set-returning
function, a column reference, a `$N`, a bare `DEFAULT`) is a parse-analysis error
there. It comes *after* whatever 18.6 decides earlier in the statement (the column
type, the key-list pass, the relation's column-name set, the table's own `42P07`,
an `ALTER`'s `42P01`/`42703`) and *before* the constraint naming block. § A column
`DEFAULT` is an expression the executor cooks has the cases we tested and the route
split. Raised in pglet's **parser**, these would come before all of those, and no
reordering inside the parser could fix it, because the statement-level gates run in
the executor. So the verdict is `executor::ddl`'s `cook_column_default`, reached
because `ColumnDefault` *carries* a general expression to the executor.

What's still raised while parsing is listed rather than counted:

- pglet's absent-function refusal pre-empts every positional verdict
  (`DEFAULT (count(*) + nosuchfunc(1))` is 18.6's `42803` and pglet's `0A000`).
- The **set-quantifier** verdict (`EDGES.md` row 51) at both of this site's parsers.
  `DEFAULT (upper(DISTINCT 'a'))` and `DEFAULT nextval(DISTINCT 's')` are `42809`
  here, where 18.6 cooks a default after the column type's `42704`, the table's own
  `42P07` and the rest of the parse.

That second one **brings 18.6's argument transform with it**. A quantified call's
constant cast is coerced in the parser too, so `DEFAULT upper(DISTINCT 'a'::int)` is
`22P02` here, before the same gates. The quantifier is what limits it. When we ran
the transform for an *unquantified* call, it pre-empted tested DDL statements' own
error codes and `IF NOT EXISTS` skips, the worst of them a **success**
(`CREATE TABLE IF NOT EXISTS <existing> (b TEXT DEFAULT upper('a'::int))` is a skip
on 18.6 and here, but `22P02` with the transform unquantified). Each item in this
list is a *refusal pglet raises early*, so widening one of them just adds another
early refusal to the list; it doesn't implement 18.6's behavior.

A plain **later syntax error**, on the other hand, agrees:
`CREATE TABLE k (a INT, b INT DEFAULT now,)` is `42601` on both, because a gate
outside the parser is ordered against the parser's *own* errors as well as the
executor's.

**The grammar a clause uses is the same kind of fact, and it's a *mode*, not a
binding power.** A column constraint's `DEFAULT` uses `gram.y`'s restricted
`b_expr`, while `ALTER COLUMN … SET DEFAULT` uses full `a_expr`. The excluded
predicate band and `AT TIME ZONE` bind *tighter* than the comparisons `b_expr`
includes, and `NOT` is a prefix production that no `min_bp` gates at all. So a
binding-power threshold used in its place diverges in **both** directions at once
(`DEFAULT 5 = 4` refused where 18.6 accepts, `DEFAULT NOT true` accepted-shaped where
18.6 refuses).

Instead, a scoped flag is checked by every arm that reads a production `b_expr`
lacks, and **that list is the contract, not a count**: the `NOT` prefix, the
`IS NULL` family (`IS [NOT] DISTINCT FROM` stays), the predicate band (`IN` /
`BETWEEN` / `LIKE` / `ILIKE`), `AT TIME ZONE`, and the **quantified comparison**
(`= ANY (…)` / `ALL` / `SOME`, `gram.y`'s `a_expr subquery_Op sub_type …`, which has
no `b_expr` twin). A count hides an omission, since someone checking a number can't
notice a production missing from it. So the list is spelled out and re-derived from
`gram.y`'s `b_expr` block instead of carried forward.

What makes the mode correct and not just stricter is that a `(` **resets** it to
`a_expr`, and the mode is *restored* on the way out. So `1 + (NOT true)::int` and
`(5 = ANY(ARRAY[1,2]))` are accepted while `(now()) AT TIME ZONE 'utc'` and
`(5 IS NULL) IS NULL` are `42601`, checked on both engines. The reset lives in
`parse_primary`, which covers the same set as the bracketing constructs, instead of
a separate list of them that could fall behind.

**`COLLATE` is on `b_expr`'s exclusion list, but this mode doesn't enforce it.**
That's worth saying because the list invites the assumption. `DEFAULT 'x' COLLATE
"C"` is **accepted** on 18.6 in the column-constraint position (checked;
`column_default` reads `'x'::text`), because the `COLLATE` is its own
`ColConstraint` element next to the `DEFAULT`, not part of its expression. pglet
refuses it with `42601`, and the control shows which gap that is: the bare
`v TEXT COLLATE "C"`, with no `DEFAULT` anywhere, gets the same refusal. It's the
missing-`COLLATE` gap, not the grammar mode.


## A column `DEFAULT` is an expression the executor cooks

`ColumnDefault` carries an `Expr` catch-all next to its recognized closed forms, so
`DEFAULT <any expression>` is supported instead of refused. The tree is stored
**unevaluated** and evaluated per row, which matches 18.6's own model: `pg_attrdef`
holds the post-parse-analysis tree, so `DEFAULT 1+1` reads back `(1 + 1)` instead
of `2`, and `DEFAULT nextval('s') + 1` draws a fresh value on every insert. The
whole verdict (the constructs Postgres forbids, the DDL-time coercion, and pglet's
own gates) lives in `executor::ddl`'s `cook_column_default`, at the position where
`AddRelationNewConstraints` cooks a default, instead of in the parser. Carrying a
general expression to the executor is what makes that possible. What still diverges
is in `EDGES.md` row 47.

**The statement-level precedence matches Postgres, tested from both sides, and the
two lists below don't cover every case.** What outranks a bad DEFAULT is whatever
18.6 decides earlier in the same statement, so the list grows with every gate put
before the cook, and it should be read as the cases we tested, not as a closed set.

On `CREATE TABLE` it **loses** to the column type's `42704`, the
`transformIndexConstraints` pass (`42P16`, a key column's `42703`, a repeated key
column's `42701`), a declared identity `SEQUENCE NAME` collision (`42P07`), the
relation's column-name set (a duplicate and a system-owned name, each `42701`) and
the table's own `42P07`. On the `ALTER` routes it loses to the target relation's
`42P01`, to both names one collision gate answers (a column the table already has
and a system-owned one, each `42701`), and to the column's `42703`.

It **beats** the constraint *naming* block that comes after the cook: a duplicate
CHECK name (`42710`, in either written order), an ill-typed CHECK (`42883`), a taken
constraint-index name (`42P07`), a missing FK parent (`42P01`), a repeated column in
a foreign key's *referenced* list (`42830`), and on `ADD COLUMN` the inline
`PRIMARY KEY` multiplicity (`42P16`). That's the same `42P16` the `CREATE TABLE`
route puts before the cook, so being on one route's list doesn't put a case on the
other's.

"The constraint block" means that naming block, not every constraint gate. A
`PRIMARY KEY`/`UNIQUE` key list is validated in the earlier pass, so
`CREATE TABLE p (a INT DEFAULT 'zz', PRIMARY KEY (a, a))` is `42701` and not
`22P02` on both engines, while its FK-referenced twin
`… FOREIGN KEY (a, b) REFERENCES pp (x, x)` gets the default's `22P02`. The `ALTER`
spelling of that first pair needs two actions in one statement, and pglet's parser
takes one, so the comma is `42601` there before either gate is reached. We checked
this on the plain two-`ADD COLUMN` control too, which shows it's the action list and
not the pair.

Between two bad defaults, **declared column order** wins. That's pinned by a
*cross-code* pair (`22P02` vs `22007`, swapping with the column order), which an
`@expect-error` fixture can hold on the SQLSTATE alone. A same-code pair would be
satisfied by an engine that answers that code to everything, and needs
`@expect-message`, which locks the message instead. The duplicate-column row of that
contract **agrees**: 18.6 reports `42701` for a duplicate column before the cook,
and so does pglet. We checked that outside the corpus, since no fixture's
`CREATE TABLE` both repeats a column name and has a `DEFAULT`.

**A plain syntax error later in the statement wins on both engines.** bison scans
the whole statement before parse analysis, so a syntax error after the default beats
the DEFAULT verdict on 18.6. pglet's parser has no separate scan pass, and agrees
because the verdict isn't the parser's. On both engines,
`CREATE TABLE k3 (a INT, b INT DEFAULT now,)` and
`CREATE TABLE k5 (a INT, b INT DEFAULT now) GARBAGE` are `42601`, and the
well-formed control `CREATE TABLE k6 (a INT, b INT DEFAULT now)` is
`0A000 cannot use column reference`. A gate outside the parser is ordered against
the parser's own errors as well as the executor's.

What's **not** fixed is every refusal pglet still raises while parsing, the list in
§ Which error wins. A **niladic builtin called with arguments** is on that list, and
this clause inherits it rather than deciding it. The walk itself sees an
`Expr::Builtin` with nothing left to object to. The refusal happens upstream, at the
one place a call is built (`build_call`), which is why `DEFAULT (now(1))` is `42883`
here without the walk needing an arm. Being a parse-time refusal, it's one more item
on that list, not a class of its own
(`CREATE TABLE t2 (a INT DEFAULT (now(1)), b nosuchtype)` is 18.6's `42704` and
pglet's `42883`).

**The render probe reaches this site and the three persisted-expression sites next
to it.** This is § Gating a stored expression is a DDL-time concern's rule seen from
the `DEFAULT` end. `executor::ddl`'s `validate_stored_expr_renders` covers a CHECK,
an index predicate and an index key part. Without it,
`CHECK (t > now() - interval '1 day')` is accepted at every CHECK spelling (over
three executor call sites), and `CREATE INDEX … ((iv + interval '1 day'))` /
`… WHERE iv > interval '1 day'` at both index positions, each making
`pglet_server dump` exit 1 for the whole database. At the `DEFAULT` site, the probe
is still the cook's last step instead of a whole-statement pass, which is
`EDGES.md` row 47's residual.

**The `b_expr` mode's own arms, tested at this site.** On top of the cases § Which
error wins lists: `DEFAULT 5 = 4` and `DEFAULT 5 IS DISTINCT FROM 4` are accepted,
`DEFAULT NOT true` and `DEFAULT now() AT TIME ZONE 'utc'` are `42601`, and
`IS NULL` / `AND` / `OR` / `IN` / `BETWEEN` / `LIKE` stay `42601`. The `SET DEFAULT`
spelling of each is accepted on both engines, and that's the control showing it's
the mode and not the construct.

The quantified comparison is `a_expr subquery_Op sub_type …` with no `b_expr` twin,
so 18.6 answers `42601 syntax error at or near "ANY"`, and so does pglet, on both
`CREATE TABLE` and `ADD COLUMN`. It's gated at the quantifier keyword before its
operand is read, the same shape as the `NOT` arm, and pinned by
`ddl/default_b_expr_excludes_a_quantified_comparison*`,
`ddl/add_column_default_b_expr_excludes_a_quantified_comparison`,
`ddl/default_quantified_comparison_is_admitted_parenthesized` and
`ddl/alter_column_set_default_a_expr_admits_a_quantified_comparison`.

**`ADD COLUMN` evaluates its default; the other two routes only store it.** 18.6
computes the added column's `attmissingval` once at the `ALTER`
(`StoreAttrMissingVal`), so the whole *value-dependent* family is a DDL error on that
route, even over an **empty** table, which is what separates it from a backfill
failing per row. On 18.6 over a table with no rows:
`ADD COLUMN a INT DEFAULT 1/0` is `22012`, `(2147483647 + 1)` is `22003`,
`VARCHAR(3) DEFAULT 'abcdef'` is `22001`, and `NUMERIC(4,2) DEFAULT 12345.6` is
`22003`. The same four on `CREATE TABLE` and on `SET DEFAULT` are all accepted, in
18.6 and here.

pglet passes a `DefaultTiming` at the one call, because the position doesn't need
anything reordered. Tested on the same route, the deferred family sits exactly where
the cook-time family does: after `42P01` / `42704` / `42701`, and before the inline
`PRIMARY KEY` multiplicity, a taken constraint-index name, a duplicate inline CHECK
name, an ill-typed inline CHECK and a missing inline `REFERENCES` parent
(`ADD COLUMN e INT NOT NULL DEFAULT 1/0` is `22012` and not `23502`;
`… DEFAULT 1/0 CHECK (c > 100)` is `22012` and not `23514`).

**A sequence-bearing default is left out of the eager arm on purpose.** The cook's
`nextval` stand-in is `1` whatever the counter holds, so over a sequence at 5,
`ADD COLUMN c INT DEFAULT 100 / (nextval('s') - 1)` would be refused here for a
division 18.6 never performs. That would be an eager verdict about a value the
statement doesn't produce. This is pinned by `ddl/add_column_default_*_is_eager` and
the `create_table_` / `alter_column_set_default_` deferred controls, which are what
make the refusals a statement about the *route*.

**A column has one default, and a `SERIAL` type name is one of the two ways to write
it.** `transformColumnDefinition`'s `saw_default` is set by the implicit `nextval` a
`SERIAL` synthesizes, so `a SERIAL DEFAULT 5` gets the same
`42601 multiple default values specified for column "a" of table "t"` as
`a INT DEFAULT 5 DEFAULT 7`. `GENERATED … AS IDENTITY` gets its own message
(`both default and identity specified`), in either written order. All checked on
both engines. A last-wins assignment would silently keep the second clause and, for
the `SERIAL` spelling, drop the implicit sequence the type name asked for.

**A SQL value function takes no argument list, so `DEFAULT current_timestamp()` is
`42601`.** The token-level classifier's `eat_empty_call` declines that spelling.
Accepting it would define the column and backfill a real timestamp where 18.6
defines no default at all. The general grammar then answers with the same message
the parenthesized spelling gets.

**A `DEFAULT` whose cooked expression is a NULL constant is no default**, and the
test matches 18.6's. `coerce_type` folds an `unknown` literal into a typed `Const` in
place, so `NULL::int`, `(NULL)::int` and `CAST(NULL AS int)` each store **zero**
`pg_attrdef` rows, while `NULL::text::int` (a second cast, a real conversion and not
a constant) **is** stored. `1 + NULL` is stored too, and it's the control showing the
rule is about *a constant*, not about *containing a NULL*. All tested. The reduction looks through exactly one cast over a bare `NULL`.

**A decimal literal is a `numeric` constant.** `DEFAULT 1.50` keeps its display
scale (18.6's `column_default` reads `1.50`, where going through `f64` would read
`1.5`, silently, with the stored value agreeing but the reported default not), and
an integer outside `i64` stays exact. And **`DEFAULT LOCALTIMESTAMP` / `LOCALTIME`
are accepted**, through the catch-all instead of a closed arm, and `column_default`
reads `LOCALTIMESTAMP` / `LOCALTIME` on both engines.

## Extensions are a name check, not a catalog

`CREATE EXTENSION [IF NOT EXISTS] name [WITH] [SCHEMA s] [VERSION v] [CASCADE]`
and `DROP EXTENSION [IF EXISTS] name [, …] [CASCADE|RESTRICT]` parse with their
full option tails and lower to a `Plan::Noop` carrying the statement's command tag.
**pglet models no extension catalog.** The reason is that the two extensions a
ported migration usually asks for are already *in* the engine: `pgcrypto`'s
`gen_random_uuid()` and `uuid-ossp`'s `uuid_generate_v4()` are ordinary builtins,
available with or without the statement, and `plpgsql` is what the `DO $$ … $$`
block is written in. There's no installation state to record, so the statement has
nothing to do.

What's left are the decisions that can be made without state, and those can be made
exactly. The first is the **name**: is it one this engine provides
(`parser::ddl`'s `KNOWN_EXTENSIONS`)? An unknown one gets 18.6's SQLSTATE for an
extension it can't find a control file for, `0A000`. That server gives the same code
under `IF NOT EXISTS` too (we checked, and that's why the name check runs before the
existence question here as well). A `DROP` naming an unknown one without `IF EXISTS`
is `42704`, and with it, a skip. The names are compared **exactly**, against a name
the parser has already folded, so an unquoted `PGCRYPTO` matches and a quoted
`"UUID-OSSP"` doesn't, both as 18.6 answers them. The second is the `SCHEMA` option.
pglet models exactly one schema, so `CREATE EXTENSION pgcrypto SCHEMA nosuchschema`
is `3F000` here as in 18.6, with no extension state needed to say so.

**The message deliberately doesn't copy 18.6's.** That server says *is not
available*, and for a name it also lacks, the two engines then agree. But 18.6
*ships* `citext`, `hstore`, `btree_gin`, `pg_trgm` and the rest, and answering "not
available" for those would be a false claim about the system instead of a true one
about this engine. The set here is what **pglet** provides, so the message says that
(`is not provided by pglet`), and only the SQLSTATE is borrowed.

**Most of what remains would be fixed by a persisted set**, and it's over-acceptance
in one direction. Without installation state, a *bare repeat*
(`CREATE EXTENSION pgcrypto` twice) succeeds here where 18.6 answers
`42710 extension "pgcrypto" already exists`. So does a bare
`CREATE EXTENSION plpgsql`, which 18.6 refuses from the very first statement because
it ships pre-installed. `DROP EXTENSION pgcrypto` with nothing created succeeds here
against 18.6's `42704`. And a `DROP SCHEMA public` without `CASCADE`, over a
`public` whose only object is an extension, succeeds here where 18.6 answers `2BP01`
(`extension pgcrypto depends on schema public`). The refusal counts the tables,
enums and sequences `public` holds, and there's no extension to count (`EDGES.md`
row 72). The `IF NOT EXISTS` spelling, which is what every migration writes, agrees
on both engines, and that's what the trade is for. A persisted set would cost a
store-magic bump and a `docs/storage.md` ledger line for a fact no query reads,
since `pg_extension` is absent here either way.

**One remaining gap a persisted set would *not* fix**, so it's recorded separately
from the paragraph above: the `VERSION` option is parsed and dropped, so
`CREATE EXTENSION pgcrypto VERSION '99.9'` succeeds here where 18.6 answers
`22023 … has no installation script nor update path for version "99.9"`. Knowing
that needs a per-extension version list, which is a different fact from *whether*
an extension is installed.
