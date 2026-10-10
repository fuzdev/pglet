# DDL gates, dependencies, and error precedence

The DDL-time correctness rules: what a `DROP` may take with it, why a failed
DDL statement must leave the catalog untouched, where a pglet-only gate may
run, how stored expressions are validated, and the error-order contract at
every scale. The rules are summarized in the repo `CLAUDE.md`; this file
carries the full statements. Naming and minting live in [names.md](names.md),
sequence ownership in [sequences.md](sequences.md).

## A `DROP` decides between auto-drop and refusal, never between silence and a brick

Postgres's `pg_depend` gives each dependent object an edge *kind*: an AUTO edge
means it dies with the target, a NORMAL edge means `RESTRICT` refuses (`2BP01`)
and `CASCADE` drops it too, and an INTERNAL edge refuses **with `CASCADE` as
without** (`findDependentObjects` raises it at the outermost recursion level
with no `DropBehavior` test in the path — so that check must sit *before* the
resolver's `cascade` short-circuit, not beside the NORMAL ones). pglet keeps no
`pg_depend`, so `executor/dependencies.rs` re-derives the kinds from the
catalog in **one** resolver `DROP TABLE`, `ALTER TABLE … DROP COLUMN`,
`DROP SEQUENCE` and `DROP TYPE` all consume — the last for the NORMAL edge from a
`CHECK` or column `DEFAULT` converting to the enum, which `CASCADE` drops or clears
(`docs/enums.md` § A cast to an enum converts; a column *declaring* the enum is the
edge it does not enforce, `EDGES.md` row 261). The rule is not "the table owns it" — a
*self-referencing* FK blocks its own table's referenced-column drop exactly as
an external one does, while dying silently with the whole table — and where one
constraint sits on both edges of the same column the AUTO edge wins. **Two
object classes ride it, and the second is where the edge kind stops being a
property of the dependent and becomes one of the *deletion set*.** A sequence a
table or column `OWNED BY`-owns is AUTO, so it goes with the drop; a column
default drawing from that sequence is NORMAL — but only if that column survives
the statement. So `CREATE TABLE t (a SERIAL); DROP TABLE t` succeeds (the
default dies with the table), a *second* table's default is `2BP01`, and
`CREATE TABLE q (id SERIAL, b INT DEFAULT nextval('q_id_seq')); ALTER TABLE q
DROP COLUMN id` is `2BP01` because a `DROP COLUMN`'s deletion set is one column
wide. The AUTO drop and that refusal are **one change and not two**: the leak
is precisely what kept the second table's default working, so shipping the drop
alone converts a leak into a broken default in a table nobody named. On the
`DROP SEQUENCE` side the two kinds split by the *owner column*: an identity
column's sequence is its INTERNAL implementation (`2BP01`, `CASCADE`
included), a `SERIAL`'s is the column's `pg_attrdef` NORMAL edge (`CASCADE`
sheds the default and keeps the column, its `NOT NULL` and its rows) — and
being merely `OWNED BY` an identity column is neither, since
`ALTER SEQUENCE … OWNED BY` records only the AUTO edge, so that sequence drops
normally on both engines. Between two NORMAL edges the order is **not**
observable (both `2BP01`, and Postgres reports every dependent in one message),
so which one pglet names decides only the message. That report's **order is the
reverse of the statement's own target list** (`reportDependentObjects` walks
its array back to front), not by kind and not by OID. A *single* target with two
dependents cannot tell the two apart — there the real rule and OID-descending
coincide — and neither can a list against its exact reverse, which coincide with the
two OID directions; a permutation does: with OIDs ascending `tp_a < tp_b < tp_c`,
`DROP TABLE tp_b, tp_a, tp_c` details `tc_c, tc_a, tc_b`, which is neither OID
direction.
`DROP INDEX` reads the same catalog fact from the other side: an index a
declared `PRIMARY KEY`/`UNIQUE` owns is refused outright, since dropping it
would un-enforce the key *and* erase it from the dump.

Two corollaries about **names**, both load-bearing and neither obvious: **a
constraint's name is not its identity** — the resolver identifies a foreign key
by `(table, position)`, because a name-keyed removal sheds a same-named sibling
along with its target. Note what does *and does not* justify that: the resolver
is right on its own terms, and it must be, because the same-named pair is
**not** unreachable on the strength of the `42710` gate. An incomplete corner of
the naming gate produces exactly that pair (a route that claims nothing, an auto-name
gated instead of disambiguated), and a name-keyed resolver turns each into silent
integrity loss instead of a duplicate row. And **a constraint's index is named after the constraint**, the
declared `CONSTRAINT <n>` when there is one, so nothing may re-derive that name
from the table's (`Schema::primary_key_index` asks the marker rather than guessing
`<table>_pkey`). Handing an introspecting client a
fabricated auto-name is the same silent-wrong class as handing it nothing, only
harder to detect.

## A failed DDL statement must leave the catalog untouched

Because the autocommit path cannot put it back. `run_planned_env` rolls back
**storage** on an error and leaves `self.schema` mutated in place — the server
snapshots the catalog only inside an explicit `BEGIN` — and the next successful
write serializes the whole schema. So a statement that mutates the catalog and
*then* fails leaves a **durable** phantom: the table is in
`information_schema.tables`, every read of it is `42P01` (storage was rolled
back), and its name is `42P07` forever. This is not a tidiness rule with a
correctness rationale bolted on; it is the only thing standing between a raised
error and a bricked name. `execute_create_table` therefore runs **everything
fallible before `schema.create_table`** — the FK resolution first, and the
PK/UNIQUE key columns, the multiple-primary-key count, and the constraint-index
name gate beside it (the last needing the table's own name *seeded* into the
relation namespace, since Postgres creates the heap before its indexes), and
last the **sequence names the statement itself registers**: a `SERIAL`'s
implicit one and a user's own `DEFAULT nextval('x')` are both claimed before
the constraint indexes are named, or the index is minted on top of the sequence
and only the post-mutation backstop catches it. Its `ALTER TABLE … ADD COLUMN`
twin is the worst instance of the class in the engine and worth stating
separately, because "phantom" undersells it: there the *column* lands in the
catalog while the widened rows roll back with storage, so the table keeps two
columns and one-value rows and **every `SELECT *` fails at the wire**
(`unexpected field count in "D" message`) for every client, permanently — and
it reproduces only on the B+tree backend, since the plain in-memory storage
double never rolls back and leaves the backfill applied, so a guard written
against that double sees nothing. The class is invisible to the differential
harness by construction: every fixture runs inside `BEGIN … ROLLBACK`, which
restores the snapshot, so its guards are unit tests (`crates/pglet`'s
`*_leaves_no_phantom`). Read the corollary before adding any DDL check:
**where** a check runs is a correctness decision on this path, not a style one.

**A `DO` statement cannot keep this rule by staging, so the executor puts it back
itself.** Its body is several statements, and one that fails comes after earlier ones
have finished their catalog writes — staging inside each of them does nothing for the
ones already done. So `execute_do_block` takes a storage savepoint and a `Schema` snapshot
before the block's first statement and, on every exit on which a statement of the block
raised — no handler, an unmatched one, a handler that raises — rolls storage back to the savepoint, restores
the catalog through `Schema::restore_from`, and re-derives the index specs over any enum
the restore moved, before the error returns. The sequence cells it restores are every one
a statement inside it replaced, a nested `DO`'s included: a `DO` that completes reports
the cells it replaced in its result, which an enclosing `DO` adds to its own list and the
host to the block's. Without the undo the body's earlier DDL stands in
the session and the next successful write persists it: a created table the bracket's
storage rollback discarded answers `42P07` to its own name and `42P01` to an `INSERT`,
a dropped one stays dropped, measured after a `kill -9` reopen too (`EDGES.md` row 249).
The guards are `crates/pglet/tests/do_block.rs`'s `a_failed_do_*` tests, over all three
backends and a reopen; the differential fixtures `ddl/do_failed_*_answers_*` and
`transaction/do_failed_in_block_*` pin only the error each failure answers, since the
harness stops at it, and what a failed `DO` put back is readable only where a caught
outer `DO`'s handler runs after it — `ddl/do_failed_inner_do_restores_its_cell_before_the_outer_handler`
reads the sequence cells that way. The snapshot and the level are copies every `DO`
pays for, a successful one included (`docs/storage.md` § Known bottlenecks / scaling
limits).

**The catalog is not the only mutable side, and staging it alone can point the same
desync the other way.** `CREATE TYPE` / `DROP TYPE` over an enum a live column declares
also rebuilds every storage index spec on that enum's tables, and that loop is a second
place a statement half-applies: staging `Schema.enums` back while the loop has
already moved the specs of the indexes it got through leaves the executor reading the old
order and storage enforcing the new one, and a unique partial index takes a duplicate 18.6
refuses — silently, `(2 rows)` where the same script without the failing statement raises
`23505`. The rule the catalog half states extends to it unchanged, and the loop meets it
by unwinding what it moved (`docs/enums.md` § A copy that outlives its statement). Ask of
a new DDL step not only *does every check run before the catalog write* but *what else
does this statement mutate, and can it put that back*.

**A walk of the whole catalog is not the statement's own work, and it is refused
before the statement rather than inside it.** A write over a persisting backend encodes
the catalog beside its data (the B+tree, on file or in memory, unless it was built for
one connection; memory storage encodes none), a `BEGIN`, a `SAVEPOINT` and a block's first write clone it, a rollback drops
one — and a stored
expression costs each of those walks a level. A catalog built where the thread's depth
bounds were larger (another thread, a limit since lowered, a budget since narrowed) is
therefore one this thread cannot walk: the `Clone` and the `Drop` cannot fail at all, so
on such a catalog they would abort the process, and the encoding would refuse `54001` only
after the statement had done its work.
So every host asks one question at each statement's **entry** — can this thread walk this
catalog (`pglet_sql::depth::check_catalog_walks`, over `pglet_sql::depth::CatalogDepth`)
— and answers `54001` before anything of the statement runs, leaving the session, the
store and the open block as they were. The same question is asked where an expression is
stored (`pglet_sql::depth::check_stored_expr`), from deeper in the stack than any entry,
so a thread that accepts an expression can still walk its catalog afterwards. What is
left inside the statement is ordering: the encoding still runs before the storage commit
publishes anything (`pglet::autocommit_publish_bytes`), so a failure there discards the
staged writes rather than answering an error for a write that stood.

**And the same route's *sequence* is the case where "run every check before the
mutation" is not enough on its own, because the mutation is an input to a
check.** An `ADD COLUMN`'s `SERIAL` / identity / `DEFAULT nextval('x')` has to
exist for the backfill to draw from, and the backfill is what the `23502` gate,
the index build's checks and the CHECK and FK validations judge — so registering it first puts a
catalog write ahead of three violations (`23514` / `23505` / `23503`), each of
which then raises over a **durable phantom sequence carrying the failed
backfill's own advances**: two rows and `ADD COLUMN c SERIAL CHECK (c > 100)`
answer `23514`, then `nextval('t_c_seq')` answers `3` and the name is
`42P07` forever, where 18.6 answers `42P01` and leaves it free. The answer is
to **stage** it — a local `Sequence` the backfill advances, inserted into
`schema.sequences` only inside the mutation block (`executor::ddl`'s
`staged_sequences`) — and the general form is worth carrying: when a check
needs a catalog fact the statement is creating, compute the fact locally rather
than moving the check after the write. The `23502` gate is in that list as a
gate and not as a violation: a sequence-bearing default resolves to an `int8`,
which no coercion turns into NULL, so `ADD COLUMN c SERIAL NOT NULL`
**succeeds** on both engines and a type that cannot take the value dies as a
cast error instead. The `23505` arm needs an exotic spelling to reach —
`NUMERIC(3,-1) DEFAULT nextval(…) UNIQUE`, whose backfilled `1, 2, 3` all round
to `0` — measured on both. Unlike the phantom column this half is
**backend-independent** (a catalog residue with no storage counterpart, so the
memory double reproduces it identically), and one asymmetry in it is
load-bearing: a sequence the statement did *not* create is advanced **in
place**, because `nextval` is non-transactional, and staging a copy of a live
counter would walk it back over ids already issued.

### A failed index build

**The fallible step a `CREATE INDEX` cannot put ahead of its catalog write is the
storage build, so the catalog write goes after the build instead.** This is the same
rule read from the other end: `execute_create_table` moves its *checks* above the
mutation, and `CREATE INDEX` cannot, because the build is not a check — it is the
statement's work, it is fallible, and `IndexSpec` is keyed by the OID, so it cannot even
be assembled before a number is chosen. `execute_create_index` and `create_unique_index`
therefore read the number with `next_object_oid` (which does not advance the counter),
build the spec, call `storage.create_index`, and write `schema.next_oid` and
`schema.create_index` only once that has returned. `execute_create_type` reads the
counter the same way, for the same reason. What that counter write buys is **not**
intra-statement distinctness — `next_object_oid` re-seeds from the live catalog max, so
`create_unique_index`'s two looping call sites read past the index the previous
iteration registered either way — but monotonicity across a `DROP`, where the max no
longer holds the number and a reused index OID puts two logical indexes on one storage
band.

**What the build can still raise.** The refusals that stand in for an 18.6 error are
raised ahead of the storage build, by the executor's own scan of the rows the build
would enter (`constraints::scan_index_build_sizes` and the duplicate check between its
halves): 18.6's three `54000`s, the build's `23505`, and any error a key expression or
a partial predicate raises on a row — `CREATE TABLE w (a INT); INSERT INTO w VALUES
(0),(1); CREATE INDEX w_ix ON w ((10 / a))` is `22012` on both engines, raised by the
entry sizing, which evaluates the key for each row. A key **type** refuses nothing: every
value has a key (`docs/indexes.md` § A float key, § A container key), and the one shape
pglet refuses for its type — a non-unique B-tree over a single bare container column —
is `validate_btree_key_type`'s, a gate ahead of the build. A partial index's predicate
is not another axis: the predicate decides *membership*, and it is the admitted rows'
**key** that is encoded. What is left inside the storage build is its own bound and the
device: a key longer than a B+tree page carries, which the executor's scan reads for a
B-tree key and a GIN array element and does not read for a `jsonb` GIN **token**
(`CREATE INDEX … USING gin (j)` over a row holding a string of 5 000 characters is
`54000` from the back-fill, where 18.6 hashes the token and builds — `EDGES.md` row 192),
and a write the device refuses.

**Ordering the catalog write after the build is only sound because
`Storage::create_index` is all-or-nothing** — a contract the trait states and both
backends meet by registering last (`MemoryStorage` and `BtreeStorageWrapper`, which
derives its def, back-fills, applies the batch, and only then inserts into `index_defs`
/ `table_indexes`, as its own `rebuild_index` does). Without that half, a catalog write
after the build would shed the visible residue and keep the invisible one. The
two halves together are what a failed build would otherwise leave:

- **Catalog.** The index is in `pg_indexes` and its name `42P07` forever. Via
  `create_unique_index` it is **constraint-marked**, so `DROP INDEX` refuses it `2BP01`
  and pglet has no `ALTER TABLE … DROP CONSTRAINT` — unrecoverable short of
  `DROP TABLE`. Not live-only either: one later ordinary statement serializes the whole
  catalog, so it survives restart on a `--data-dir` store, and the store's own dump then
  emits the failing statement — a database that reloads into stock Postgres and is
  refused by pglet.
- **Storage.** On the B+tree backend the def stands and the table becomes unwritable:
  an `INSERT`, `UPDATE` or `DELETE` whose row reaches the encoder raises. On
  `--storage memory`, which registers last, only the catalog half exists, and it leaks
  `22000 execution error: index not found: <oid>` from the accelerated reads and from
  every write that probes a unique index.
- **The OID.** Burned on every build failure. The allocation also sits below the
  catalog checks and the `IF NOT EXISTS` skip, so neither of those burns one.

Guards: `crates/pglet`'s `a_failed_create_index_build_leaves_no_phantom`,
`a_failed_create_index_build_inside_a_block_leaves_nothing_to_roll_back` and
`a_rejected_create_index_leaves_the_oid_counter_alone` for the catalog and counter
halves (with `an_index_oid_is_never_reused_after_its_index_is_dropped` for what the
counter write itself buys), `index_registry_hygiene.rs`'s three for the storage registry,
the reopen, and the one question the registry cannot answer —
`a_failed_back_fill_write_leaves_no_readable_entry_in_the_band`, which drives an injected
I/O error through both halves of `apply_batch`'s write tail and reads the band back
through a later index that takes the same id — and
`tests/fixtures/ddl/failed_gin_index_build_leaves_the_table_writable.sql` for the storage
half from SQL (`failed_expression_index_build_leaves_the_table_writable.sql` holds the
statement-level property only: its `22012` is raised by the build's entry sizing, ahead of
the storage build). Those fixtures need `-- @no-txn`, because pglet defers index
maintenance inside a transaction block and the harness's own wrapper makes them green
with the defect present; their `CREATE INDEX` must also be swallowed by a
`DO $$ … EXCEPTION WHEN others … $$`, since `run_setup` propagates the first error — and
that handler sheds the catalog half, which is why the catalog guards are unit tests.

**The SQL guards of the storage build ride one trigger, and one route has none.** The statement that
fails inside the storage build is the `jsonb` GIN token above, and
`the_gin_token_refusal_is_the_storage_builds_own` (`crates/pglet`) holds that it is the
build that refuses it — by an order only the build's position gives, against the render
probe that is the last gate ahead of it — so the guards using it do not go blind the day
an earlier gate learns to refuse the token. It reaches `execute_create_index` only. A
**declared constraint's** index (`create_unique_index`: an inline `UNIQUE` /
`PRIMARY KEY`, `ADD COLUMN … UNIQUE`, `ADD CONSTRAINT`) is a B-tree over bare columns,
every refusal of one is the executor's, and no statement was found that fails inside its
storage build; on that route the catalog-after-build order is held by a device failure
instead (`index_registry_hygiene.rs`'s
`a_failed_constraint_index_storage_build_writes_no_catalog`, over `ADD CONSTRAINT … UNIQUE`
and `ADD PRIMARY KEY`).
`a_refused_constraint_index_leaves_nothing_on_the_add_column_and_pkey_routes` and
`a_refused_add_column_unique_index_leaves_the_table_whole` pin the position of the
refusals **ahead** of that route's build, by pglet's own page bound over an `int[]` key.

**What this does not change** is pglet's own refusals of DDL 18.6 accepts — the page
bound (`EDGES.md` row 192) and the bare container B-tree (row 25). Each fails cleanly.

## A gate standing in for *no* Postgres error must run after every gate that stands in for one

Including the ones that decide a Postgres *success*. The error-order contract's
scales below are all about reproducing Postgres's own precedence; this is the
case where pglet has a check Postgres does not have at all, and the rule is not
"put it where the analogous check lives" but "put it last". `CREATE INDEX` has
one: pglet refuses a non-unique B-tree over a single bare container column (`EDGES.md`
row 25), while Postgres indexes `jsonb` and arrays happily (`jsonb_ops`, `array_ops` are
default B-tree opclasses).
Sequenced with the *real* method checks — `get_am_oid` and `amcanunique`, which
`DefineIndex` genuinely raises ahead of `index_create`'s name conflict — it
inherited their precedence and turned
`CREATE INDEX IF NOT EXISTS <a taken name> ON t (jsonb_col)`, the canonical
idempotent-migration idiom and a Postgres **success**, into a `0A000`. So
`validate_index_method` (Postgres's half, before the name) and
`validate_btree_key_type` (pglet's own, after the name and the skip) are two
functions, and the split is by *whose error it is*, not by what it inspects —
both read the key column's type.

### The mirror: a gate that *does* stand in for a Postgres error goes where 18.6 raises it

The case is `CREATE [UNIQUE] INDEX`'s **build**.
18.6's `index_create` builds the index as its last act, and a duplicate key
among the existing rows raises `23505 could not create unique index "…"` with
`DETAIL: Key (b)=(7) is duplicated.` — a different error from the per-row
`already exists` one, on the same SQLSTATE. An engine that registers the index and
back-fills it without asking creates an index that is simply never held.

**That is not a catalog divergence, it is a row one.** 18.6 creates no index, so
the *next* duplicate row writes there; an engine holding an index it should never
have built refuses it `23505`. Two engines, the same statements, different
table contents — measured, `nd6` holding three rows there and two here.

Three things the build gate has to get right, and the first two are this page's own
rules read in the other direction:

- **Where.** Between the name conflict (with its `IF NOT EXISTS` skip, which
  `index_create` owns too — `CREATE UNIQUE INDEX IF NOT EXISTS <a taken name>`
  over duplicate rows reports success on **both** engines) and
  `validate_btree_key_type`, which stands in for no 18.6 error at all. Over a
  `jsonb` key column with duplicate rows the ordering is observable: `23505` on
  both engines, where the pglet-only gate first would have said `0A000`.
- **Before the first catalog write.** The counter write is a catalog mutation
  the autocommit path cannot undo, and `schema.create_index` a durable phantom
  whose name is `42P07` forever. The `Index` value is therefore assembled
  *ahead* of both, with a placeholder OID, so the gate can read the real thing
  without anything being written. (Both catalog writes also sit *below* the
  storage build — § A failed index build — so the gate is ahead of a statement whose
  whole mutating half is at the end; being ahead of the build is what this bullet
  asks for, since the build is where 18.6 raises the `23505`.) That placeholder is kept out of storage by the **type**: `PreparedIndex.index_id` is an `Option<u32>`
  which the placeholder `0` maps to `None` (no real OID is below 16384), so the
  one reader that probes storage with it names the case and refuses loud
  (`PreparedIndex::probe_index_id`, called from `occupancy_of`). A
  probe against index `0` would answer "no rows", and a uniqueness check reads
  "no rows" as "no conflict" and writes.
- **Through the write path's own key preparation.**
  `executor::constraints::validate_index_build_unique` calls
  `prepare_one_index` + `prepared_unique_index_key` — the same two
  functions `enforce_index_entries` calls per row — so a partial index's predicate,
  `NULLS [NOT] DISTINCT`, an expression key and a composite key are **prepared** at
  build time exactly as a later write prepares them, and those cannot drift. The cost
  is one extra full scan of the table plus a map holding every distinct key, paid once
  per `CREATE UNIQUE INDEX` (as `ADD CONSTRAINT UNIQUE` / `ADD PRIMARY KEY` pay
  it), and nothing for a non-unique index, which runs no such scan.

**The build's *occupancy* question reads committed rows plus its own.** Key preparation
is shared with the write path; the reading is the scan's, and it asks nothing about
other connections — which it need not, since no other connection's block holds a
staged write while the build runs: `CREATE UNIQUE INDEX` takes the writer, so a build
over a duplicate another connection's open block staged waits for that block and then
refuses `23505 could not create unique index "g8_a"`, as 18.6 does after blocking on the
relation lock (`EDGES.md` row 103, closed) — on the native server; the embedded
`Database`, which cannot wait, refuses the build `55P03` (row 229).

**And *which equality* the scan uses is a second decision.** A validation scan asks
"would this index hold two entries it calls equal?", and the index's equality is its
**key encoding**'s, not the `Value`'s structural one. `Value::encode_index_key` is
order-preserving and so normalises what ordering cannot distinguish; `postcard` does
not. A scan comparing postcard bytes lets a `numeric` column holding `1.5` and `1.50`
take `CREATE UNIQUE INDEX`, `ADD CONSTRAINT UNIQUE` and `ADD PRIMARY KEY` where 18.6
refuses all three (`Key (n)=(1.5) is duplicated.`) — after which the engine's **own**
write path refuses the row already sitting in the table, the engine disagreeing with itself.
`constraints::index_key_identity` is the one equality all three validation scans use,
keyed by `encode_index_key` under `encode_probe_key`'s presence tagging (a NULL cannot
collide with a present value whose encoding is empty). Two further consequences:
the scans agree with each other, and the `DETAIL` names the **first**
row in scan order rather than the colliding one, which is what 18.6 names and which
only differs when the equality calls two spellings equal.

**Every key type is compared by its key**, `jsonb` and arrays included — postcard
bytes are finer than 18.6's equality, so a postcard comparison would pass `jsonb '1'`
beside `'1.0'`. That pair and `{"a": 1}` beside `{"a": 1.0}` are the build's `23505`
on both engines
(`container_key/jsonb_create_unique_index_over_scale_duplicates_loud.sql`,
`container_key/jsonb_add_constraint_unique_over_scale_duplicates_loud.sql`), as
`'0.0'::float8` beside `'-0.0'::float8` and a NaN beside a NaN of the other sign are
(`float_key/unique_build_negative_zero_loud.sql`,
`float_key/add_unique_constraint_nan_loud.sql`).

The accept side is the half a build gate gets wrong: distinct keys, a partial
unique index whose duplicates all fail its predicate, and two NULL keys under
the default `NULLS DISTINCT` are each **created** on 18.6, and each is pinned
beside the refusal. The one field pglet does not reproduce is an expression
key's `D` — `Key (lower(s))=(a) is duplicated.` there, omitted here, the same
truthfulness rule `enforce_index_entries` follows for a key it cannot name.

**A second case is the identity clause's two last checks.** `ALTER COLUMN … ADD
GENERATED … AS IDENTITY` over a column not declared `NOT NULL` is 18.6's `55000 column
"c" of relation "t" must be declared NOT NULL before identity can be added`, and over a
column that has a default its `55000 column "c" of relation "t" already has a default
value` (`ATExecAddIdentity`). Each stands in for a row divergence. The clause reads no
row, so accepting it and setting the flag would declare `NOT NULL` over whatever the
column holds, and an ordered index read would then leave a NULL row out
([indexes.md](indexes.md) § Index probes are exact-or-decline). And writing the
identity's `nextval` over a default would drop it with no word: over a `SERIAL` holding
`1`, `2` the next row would draw `1` again. The first gate reads the declaration alone,
so it refuses over rows holding a NULL, rows holding none and an empty table alike; the
second reads whether the column has a default — a constant's, an expression's, a
`nextval`'s, a `SERIAL`'s, one given by `SET DEFAULT` — and `DEFAULT NULL`, a dropped
default and one set to NULL are none, each measured on 18.6. `SET NOT NULL` and `DROP
DEFAULT` are the ways in. The same two things decide where the gates go:

- **Where.** Behind the sequence's refusals, the declaration's ahead of the default's.
  18.6 creates the identity's sequence before `ATExecAddIdentity` looks at the column,
  so over a column that fails either gate the type's `22023`, a taken `SEQUENCE NAME`'s
  `42P07` (a table's, an index's or a sequence's) and a type's name's `42710` each win;
  over a column that is neither declared nor without a default the declaration's
  refusal is the one raised; and an identity column, whose own `nextval` is a default
  in the catalog, is refused as already an identity column and not for that default.
  Each pair is measured on both engines (`ddl/add_identity_nullable_column_*`,
  `ddl/add_identity_default_column_*`,
  `ddl/add_identity_over_an_identity_column_is_already_identity_loud.sql`,
  `crates/pglet/tests/identity_declaration.rs`), behind the relation's `42P01` and the
  column's `42703`.
- **Before the first catalog write**, which is the sequence's registration: a refused
  statement leaves no sequence under the name it would have minted, on a file store
  across a reopen as in memory.

What reaches the first gate is the declaration both statements that make a column
give it: a `SERIAL` column is declared `NOT NULL` by `ADD COLUMN` as by `CREATE TABLE`
(`parser::ddl`'s `ParsedColumn::nullable`, one derivation for the two), each row the
table already holds drawing its value before the declaration is written. So the
statements that turn a serial column into an identity one — `DROP DEFAULT`, then the
clause — are accepted over an added column as over a created one
(`ddl/add_identity_over_an_added_serial_column.sql`).

What that statement's order does not reproduce stands beside the gates. A column that is
already an identity column is `55000` here ahead of a taken
`SEQUENCE NAME`, where 18.6 raises the name's `42P07` or `42710` first. The sequence
options 18.6 judges while creating the sequence (`START WITH` outside
`MINVALUE`…`MAXVALUE`, `INCREMENT BY 0`, each `22023`; a repeated option or an empty
list, `42601`) are not judged here (`EDGES.md` rows 215 and 315), so over a column that
fails a gate such a statement is the gate's `55000` against the option's refusal, and
over one that passes both it is accepted. A system column (`ctid`, `xmin`) is the
type's `22023` there and the column's `42703` here. A default 18.6 keeps as a cast of
NULL to another type than the column's (`c bigint DEFAULT NULL::int`) is no default
here, so the clause over it is accepted where 18.6 refuses it (`EDGES.md` row 47); a
NULL cast again to the type it already has (`c int DEFAULT NULL::int::int`), which 18.6
keeps no default for, is a default here, so the clause over it is refused where 18.6
accepts it (the same row). And
18.6 accepts the clause and the declaration in one statement — `ALTER TABLE t ALTER
COLUMN c ADD GENERATED ALWAYS AS IDENTITY, ALTER COLUMN c SET NOT NULL`, whose `SET NOT
NULL` it performs first — which is `42601` here.

## A gate that reads the catalog is blind inside the statement that is building it

And the `CREATE TABLE` path is where every sequence gate is called. The rule
above and the declared-name rule ([names.md](names.md)) share a hole of exactly one
shape: `resolve_implicit_sequence_name` and `ensure_sequence_relation` decide by
reading `Schema`, and a `CREATE TABLE`'s own table is not in `Schema` yet —
nothing is mutated until every check passes, which is itself a hard requirement
(the phantom rule above). Ungated, `CREATE TABLE dz (id INT GENERATED ALWAYS AS
IDENTITY (SEQUENCE NAME dz))` and `CREATE TABLE dz (id INT DEFAULT
nextval('dz'), b INT)` both put `dz` in `tables` and `sequences` at once — after
which `nextval('dz')` answers `42809` while `INSERT … VALUES (DEFAULT)` answers `1`,
and the database's **dump silently loses the table and every row it held**,
identically in stock Postgres and in pglet's own restore (`relation "dz" already
exists` → `sequence cannot be owned by relation "dz"` → `column "id" of relation "dz"
does not exist`, and the `COPY` has nowhere to land). The constraint-index namer
answers this shape with a seed (`ConstraintNames::new` puts the table's own name in
`claimed_relations`); the sequence family answers it with the gates below.

Three things those gates have to get right, none of them a matter of taste. The
two halves take **different SQLSTATEs** — the declared name is `42P07` (the
collision is the table's, per the fifth position in [names.md](names.md)) and
the user default is `42809` (Postgres accepts that `CREATE TABLE` and raises
`42809` at the first write, so that is the consistent spelling of pglet's
documented DDL-time over-rejection). The declared half must be tested at the
table's own `42P07` and **not** by seeding the name into the loop, or the wrong
relation is named and at the wrong position. And the user-default half must run
in its **own pass, after** the table's `42P07` — Postgres cooks a default once
the heap exists, so an existing table of the statement's name and a colliding
`SEQUENCE NAME` on any column both outrank it, and the same pass split is what
stops a sibling `DEFAULT nextval('<a SERIAL column's sequence>')` from
re-registering a bare record over the one that carries the column's
`START WITH` and its ownership edge (`CREATE TABLE k (b INT DEFAULT
nextval('k_a_seq'), a INT GENERATED ALWAYS AS IDENTITY (START WITH 100))` is
accepted by both engines and answers `100`, where a re-registered bare record answers
`1`).

The general question to ask of any new gate on this path: *which of the names
this statement is creating can it not see?* — and it has to be asked of
**every** gate. Asked of the table name alone it leaves the same hole a layer over: a
user-default sequence gated for kind but never **claimed** is invisible to the
constraint-index namer too, and `CREATE TABLE t1 (a INT DEFAULT nextval('t1_pkey'),
b INT PRIMARY KEY)` would mint the index straight onto the sequence and brick `t1`
from the post-mutation backstop. The sequence is claimed, so both routes
disambiguate to `<name>1`, byte-identical to Postgres with that sequence
pre-created. Two rules follow. The name a statement will register belongs in
the in-flight namespace at the point Postgres's own catalog would hold it (for
a cooked default, `AddRelationNewConstraints`). And a **deliberate
over-rejection must be the last check**, because it stands in for an error
Postgres raises later — placed earlier, pglet's DDL-time `42809` would outrank a
duplicate CHECK name, an ill-typed CHECK, a taken constraint-index name and a missing
FK parent, all of which Postgres does raise on that statement; the gate runs after
the whole constraint block on both routes, and fixtures pin the pairs.

## Gating a stored expression is a DDL-time concern

An operator gate is not purely additive when the engine *persists* expressions:
a CHECK constraint, index predicate, or index key expression that the gate
rejects at maintenance time would turn an accepted DDL into an **unwritable
table**. `CREATE INDEX` / `CREATE TABLE` / `ALTER TABLE` therefore validate
stored expressions up front (`executor/ddl.rs`), so the failure lands where
Postgres puts it. The checks run in **one bottom-up, left-to-right walk**,
because Postgres resolves in source order during parse analysis and two
separate whole-tree passes would let whichever ran first win regardless of
*where* each error sits — over a table whose `a` is not text,
`CHECK (a LIKE 'x' AND bogus > 0)` is `42883` while
`CHECK (bogus > 0 AND a LIKE 'x')` is `42703`, and no pass ordering yields
both. (The non-text half is the example's load-bearing half: give `a` a `text`
type and `~~` accepts it, so both spellings are `42703` on both engines and the
pair discriminates nothing — measured on 18.6 in the `RETURNING` spelling of
the same pair.) The walk drives (1) **binding** — a leaf that can never resolve
here: `DEFAULT` (`42601`), a `$N` parameter (`42P02`), or a column the table
lacks (`42703`), each of which, accepted, would fail *every* write; (2) **static operand typing** over `Expr::infer_type`
(`planner/legality.rs`, which stays a pure per-node rulebook — the traversal is
the caller's); (3) the **boolean-context gate** on each child its parent puts in
one, applied the moment that child's own subtree is done rather than after the
parent's — 18.6's `transformBoolExpr` coerces one argument before transforming
the next, so `CHECK (a AND b LIKE 'x')` is `42804 argument of AND` and
`CHECK (b LIKE 'x' AND a)` is `42883`, and no pair of whole-subtree passes
yields both; and (4) a **representative-row probe**. Together a strong
filter, **not** a proof: a value-dependent error can still surface at write
time, as it does in Postgres. And it has a known hole that is *not* value-dependent: a
typed constant whose input conversion fails (`'1e39'::real`, `'zzz'::int`) is refused by
18.6's DDL, but the probe keeps only operator-resolution outcomes, so the conversion's
error is discarded and the DDL is accepted — the unwritable table this section exists to
prevent, and a dump whose restore drops the constraint (`EDGES.md` row 301). Above the walk, once it and the probe have
passed, a stored *predicate* takes the clause-root gate its own `BoolContext`
names — `validate_stored_predicate`, `Check` for a CHECK and `Where` for a
partial index's — which is where 18.6's `cookConstraint` and `transformIndexStmt`
put `coerce_to_boolean`: last, after the whole expression is transformed. So
`CHECK (a)` over an `INT` column is `42804 argument of CHECK` at DDL on both
engines. It also has a known false *rejection*, shared by
every gated family: a stored expression is checked **as parsed**, without the
planner's literal-coercion pass, so a bare literal types as `text` instead of
staying Postgres's `unknown` — `CHECK (n = '5')` and `CHECK (d @> '{"k":1}')`
are rejected here and accepted there, and the fix is the explicit cast.
Treating a literal as unknown would also stop rejecting
`CHECK (data->'k' LIKE 'x%')`, the NULL-intermediate shape the probe
structurally cannot see and the whole reason the static check exists; closing
it properly means resolving the stored expression against its table's scope
before recording it (`tests/fixtures/ddl/check_containment_*`).

**And one more check belongs beside those, because the persistence has a
second reader: the expression must *render*.** `pglet::dump` renders every
persisted expression through one `Result`, so an expression the deparser
declines does not make that one table undumpable — it makes the **whole
database** undumpable. All four persisted-expression sites run that probe
(`crate::deparse::render_expr` — last in `cook_column_default` for a column
`DEFAULT`, and `validate_stored_expr_renders` for a CHECK, an index predicate
and an index key part), which is what keeps
`DEFAULT now() - interval '1 day'` and `CHECK (t > now() - interval '1 day')`
DDL-time `0A000`s rather than accepted databases that cannot be exported — the
parser carries an interval literal as an `Expr::Bound`, which the deparser has
no source spelling for, and `parse_interval_literal` is its only live SQL-text
producer (`parser::ddl`'s `column_default_to_expr` holds a second constructor,
dead by its own classifier's range and confined to the already-probed `DEFAULT`
site). Without the probe the gap is the ordinary shape rather than an exotic one:
`CREATE TABLE ck1 (t TIMESTAMPTZ, CHECK (t > now() - interval '1 day'))` —
accepted by 18.6 — makes `pglet_server dump` exit 1 for the whole database, and the
same interval shape reaches every CHECK spelling (three executor call sites serve
them) and both index positions, measured.

**The invariant is *every accepted* expression renders, and that is a claim
about the paths reaching the probe rather than about the probe.** At the
`DEFAULT` site the probe is the cook's last step, so it is skipped by every
early return above it, and an arm returning there accepts what the probe never sees:
an expression whose probe *evaluation* fails with a deferred code (`22012`, `22003`)
has no value to coerce, and returning on it would **accept**
`DEFAULT now() + interval '1 day' * (1/0)` and persist the very `Expr::Bound` the
paragraph above says is refused — after which `pglet_server dump` exits 1 for the
whole database, every other table's schema and rows gone from the export, and
`information_schema.columns.column_default` reads SQL NULL for the column, the same
defect wearing a silent face because that view has no error channel. The neighbouring
interval spelling that *evaluates* is refused either way, which is what hides such a
hole: a guard written on it is green with the bug. Both readers of a persisted default carry a "no
accepted DDL reaches the declining arm" sentence, and the probe is the only
thing that makes either true — so a *reader*-side claim of unreachability has
to name the writer-side gate it depends on, or it is a claim nobody owns.

**And *where* the render probe runs follows from whose error it stands in for,
which is why it is a separate function from the checks above rather than their
last step — and why it is a whole-*statement* pass rather than a
per-constraint one.** Those reproduce 18.6 errors raised in parse analysis,
ahead of `index_create`'s name conflict, so they run where they do; each is
positioned *against* a Postgres error, so each has a position. The render probe
reproduces **nothing** — 18.6 accepts every expression it declines — so it has
no position to reproduce, and the "gate standing in for no Postgres error runs
last" rule is the whole of its placement, *including against the gates that
decide a Postgres success*. It runs at the end of every statement that persists
a stored expression, immediately before that statement's first catalog write
(`validate_check_exprs_render` on the three CHECK routes,
`validate_index_exprs_render` after the index name conflict and the
`IF NOT EXISTS` skip), the phantom rule bounding it from the other side. One
deliberate carve-out on the `ADD COLUMN` route: a **live** (pre-existing)
sequence the discarded backfill drew from keeps its advances, written back
ahead of the probe — the in-place `nextval` rule ([sequences.md](sequences.md);
walking a live counter back re-issues ids, and 18.6's counter moves on its
equivalent statement too) — so "before the first catalog write" is exact for
every catalog fact except a live counter cell; the same statement's *staged*
sequence is discarded whole.

**Both halves are measured against the placement each rules out.** Folded into
`validate_stored_expr` on the index routes, the probe turns
`CREATE INDEX IF NOT EXISTS <a taken name> ON t ((iv + interval '1 day'))` —
the idempotent-migration idiom, a **success** on 18.6 — into a `0A000`, the
same trap `validate_btree_key_type` was split out of `validate_index_method`
for. Riding inside the per-constraint cook on the CHECK routes, it pre-empts
that block's *errors*, in the measured arrangements across the three
routes: a taken constraint-index name (`42P07`), a missing FK parent (`42P01`),
a later duplicate CHECK name (`42710`), an `ADD CONSTRAINT`'s existing-row
violation and an `ADD COLUMN`'s backfill violation (`23514`) — each `0A000`
there against 18.6's own code, each agreeing at the whole-statement position.
**Whether source order is the variable depends on the gate.** The duplicate name
shares the cook-then-name loop, so source order really does decide it; the
constraint-index name and the FK parent are resolved by *later blocks*, with the
whole CHECK loop standing ahead of them, so a per-constraint probe diverges for those
in **both** orders and no arrangement of them is a control. *Which* gate a divergence
is measured against decides whether order is even the variable — a fact about the call
graph, not about the SQL. On the `DEFAULT` twin (`EDGES.md` row 47) the
probe is still the cook's last step and the residual stands there alone; the
same three kinds are measured diverging for it, and the same move closes it.

The gate closes the *producers*, and that is not the same as closing the
*state*: the gate leaves the persisted shape as it is, so no store magic marks it,
and a database written by a build without the gate can hold an interval CHECK that
reopens, reads and writes — and cannot dump. It fails **loud** there
rather than emitting a guessed spelling or silently dropping the constraint,
which is the property
`a_legacy_stored_expression_that_cannot_render_fails_the_dump_loudly` pins over
a hand-built catalog. Rendering the interval literal — closing the
over-rejection from the other end — is the real repair and is separate work.

## A constraint's key list is validated by the constraint, not by the index it builds

Four independent facts, and none of them follows from the others — the way to get
this area wrong is to notice one and generalize it into a rule about name lists:

- a `PRIMARY KEY` / `UNIQUE` key list **refuses** a repeated column, `42701`, under
  the constraint's own sentence (*column "a" appears twice in primary key
  constraint*, or *unique* for the other kind);
- the index that constraint builds carries no such rule, so `CREATE INDEX t_i ON t
  (a, a)` is accepted by both engines, rows inserted afterwards are indexed under
  both segments rather than refused, and `pg_indexes.indexdef` reads both back;
- a foreign key's **referenced** list refuses a repeat, `42830` — the SQL standard
  forbids it, and 18.6's `transformFkeyCheckAttrs` adds the implementation reason:
  the list-to-index match would be ambiguous about which opclass pairs with which
  key;
- a foreign key's **referencing** list accepts one, so `FOREIGN KEY (a, a)
  REFERENCES p (x, y)` builds and then enforces the pair `(a, a)` against the
  parent's `(x, y)`.

`validate_key_columns` owns the first, and the second holds because `CREATE INDEX`'s
path never calls it; `validate_referenced_columns_distinct` owns the third, and is
handed the parent's list alone — the referencing list reaches it by no route. The two
acceptances are guarded as **accept-side controls**
(`ddl/create_index_key_may_repeat_a_column.sql`,
`ddl/foreign_key_referencing_list_may_repeat_a_column.sql`), because a gate written
over "a list of column names" rather than over the constraint reddens there and
nowhere else.

**What the referenced-list refusal outranks, and what it never reaches.** It runs
ahead of the covering-key lookup, so `FOREIGN KEY (a, b) REFERENCES p (y, y)` over
a parent with no key on `y` is *must not contain duplicates* rather than *there is
no unique constraint matching given keys* — both `42830`, so the position decides
the sentence and not the code. It runs over a **derived** list too: an omitted
`REFERENCES p` resolves to the parent's primary key, and the check reads the same
`parent_columns` whether that list was written or derived. A derived one comes from
a key list the first rule above refuses a repeat in, so only a store written
by a build without that gate can supply one — which is the reason neither route is
exempted from the call rather than a reason to exempt the derived one. What it does *not* reach is the
unequal-arity spelling: `FOREIGN KEY (a) REFERENCES p (x, x)` is refused a layer
earlier by the parser's own arity check, which sits ahead of every execute-time
verdict including the parent's own — measured, `CREATE TABLE c (a INT, FOREIGN KEY
(a) REFERENCES <no such table> (x, x))` is `42830` here and `42P01` on 18.6 — a
divergence this gate neither causes nor closes (`EDGES.md` row 275).

**The PK/UNIQUE refusal's position differs by route, and the difference is whose
pass the key *resolution* belongs to.** 18.6 tests each key against the keys
already accepted, inside `transformIndexConstraint`'s per-key loop, with the
resolution above it in the same body — but skipped entirely when `cxt->isalter`.
So on `CREATE TABLE` the two verdicts interleave (`PRIMARY KEY (a, a, nosuch)` is
`42701`, `PRIMARY KEY (a, nosuch, a)` is `42703`) while on `ALTER TABLE … ADD
CONSTRAINT` the repeat wins even where no key resolves at all
(`PRIMARY KEY (nosuch, nosuch)` is `42701`), the key column's `42703` arriving
later from `DefineIndex`. pglet reproduces that by taking the column list as an
`Option` — `Some` on the `CREATE TABLE` route, `None` on the `ALTER` one, whose
caller supplies its own `42703` after the call returns. § Which error wins carries
the rest of the arrangement, and `EDGES.md` row 39 the measured precedence pairs.

## Which error wins is part of the contract, at three scales

Postgres resolves a statement in source order during parse analysis, so when
two things are independently wrong the *positionally first* one is reported —
and pglet reproduces that at each scale separately, because none of
them follows from the others. **Within an expression**, one bottom-up
left-to-right walk decides operand legality and branch unification per node
(`planner/coercion.rs`, and the same shape in `executor/ddl.rs` for stored
expressions); two whole-tree passes cannot, since whichever runs first wins
wherever its error sits. The literal conversions are such a pass: they run over the
whole expression ahead of its name binding (`EDGES.md` row 327). **Between the clauses of a statement**, each DML
lowering visits its clauses in `analyze.c`'s order — INSERT resolves its row
source, then `ON CONFLICT`, then `RETURNING`; UPDATE resolves `WHERE`, then
`RETURNING`, then the `SET` list; a missing target relation short-circuits all
of it with `42P01` — which is why `ON CONFLICT` is lowered in the planner
rather than where it is parsed. **Below that order sits a later tier**, for a gate
standing in for a check 18.6 makes in the *rewriter* (`rewriteTargetListIU`) rather
than in parse analysis: it runs after every clause of its statement is lowered **and**
below the correlated re-walk that closes a nested sub-plan's binding gate, so a clause
error raised at either of those places outranks it. **What that tier orders is
pglet's own pipeline**: the verdicts pglet reaches while it lowers, and nothing else. An
error 18.6 raises in parse analysis and pglet raises at *execution* — an unresolvable
relation's `42P01` — is not in that set, and which of the two answers a statement that
carries a repeat is decided by whether that statement reaches the gate at all, which is
the gate's call site's property and not the error's: the same
`RETURNING (SELECT 1 FROM nosuchrel)` leaves the `UPDATE` `SET` list's gate skipped, so
pglet's own `42P01` answers over a matching row and nothing answers over none, and runs
the `ON CONFLICT DO UPDATE` list's, whose call site carries no such skip, so its `42601`
answers. Which statements reach which gate is not enumerated, and no bound is claimed
for it.
The duplicate-target refusal on an `UPDATE`'s `SET` list is one gate in that tier,
above the `GENERATED ALWAYS` identity guard beside it because that is
the order `rewriteTargetListIU`'s own two loops take; the one on an
`ON CONFLICT DO UPDATE`'s list is another, below everything the `INSERT`'s own target
list earns. Which gates are and are not in the tier here, and the residuals that
leaves, is [names.md](names.md) § A statement's target column list is a set.
**Between protocol phases**, the extended
protocol's `Parse` raises a plan error rather than deferring it, as Postgres
does — but only the errors the *planner* raises. A **missing relation is not
one of them**, in most of the spellings that reach it: pglet resolves the target
at execute time, so `Parse` and `Describe` both succeed on
`SELECT * FROM no_such_table` — `Describe` answering an **empty
`RowDescription`** (`fields=0`), a fabricated descriptor for a table that does
not exist — and the
`42P01` lands at `Execute`, where Postgres raises it at `Parse` and fails the
client's `prepare` outright. Which spellings defer is measured membership rather
than relation-ness, over four outcomes and not two — a schema-qualified or
star-qualified one raises here at `Parse` too, a `HAVING` one takes pglet's own
`0A000` at `Execute` in place of the lookup, and a reachability-silenced one
raises nowhere at all — and the enumeration is `EDGES.md` row 29's. What
decides `NoData` from a
`RowDescription` is not that enumeration but the engine's one descriptor rule
(`row_description`: **`NoData` exactly when the statement returns no result
set** — DDL, transaction control, a DML with no `RETURNING`, and the branches
that produce no plan at all), and a *scan* always returns one, so the
`SELECT *` above is a `RowDescription` however narrow its expansion. The
criterion is result-set-ness and never the column count: reading the count
answers a zero-column `SELECT *` `NoData` and drops its rows on the simple
leg, which is `EDGES.md` row 59 (**closed**); rows 29 and 59 are two records
over that one rule, neither the other's source. **The descriptor rule is not the only deferral:** the eval-time gates enumerated here defer the same way, measured
over an ordinary table with
no missing relation in it (`sum(<text>)` and `avg(<text>)` at `42883`,
`<text> + <int>` at `42804`, an aggregate in a `WHERE` at `0A000`, a
per-row `1/0` at `22012` — pglet answering `1 t T 2 T E Z` where 18.6 answers a
bare `E Z`, or `1 t T E Z` for the `1/0`, which *its* planner constant-folds,
landing that refusal one message later, at `Bind`), while the parser's and the
plan-time binder's own refusals (`42601`, `22P02`, `42703`, `42702`, `42704`,
`42803`) do land at `Parse` on both engines. **"Defers" is itself too wide**:
where evaluation never reaches the gate the refusal is not late but **absent** —
`1 t T 2 T C Z` — or the same with `D` rows before the `C` where a row is
projected — no error on either leg. Which spellings reach it is an
**enumeration**, not a rule this doc restates: the governing statement is `EDGES.md` row 56's *evaluation
reaches the expression, or it does not*. The measured silencers, the controls
that bound them (`LIMIT 0` silences nothing; the relation lookup survives every
row-count silencer), and the **third** direction — where pglet is the louder
engine — are `EDGES.md` row 60.

The trap: a clause with no gate is invisibly "correct"
because its neighbour's error wins by default, so *adding* a gate is a change
to statement-level error precedence every time. **Between two guards on one
statement** it bites the same way: `ALTER TABLE … DROP COLUMN` checks FK
dependents *before* the index position-drift guard, because a referenced column
always carries a covering unique index and the guard would otherwise always
answer first — with `0A000` where Postgres says `2BP01` and names the
constraint. **Between the constraints of one `CREATE TABLE`** it bites again,
and the shape is not obvious from the SQL: Postgres attaches the `CHECK`s as
the heap is created, builds the constraint indexes, then adds the foreign keys
— each with its *name* checked before its parent relation is resolved. So a
duplicate name beats a missing parent on the same FK (`42710`), a missing
parent on an *earlier* FK beats a duplicate name on a later one (`42P01`), and
a CHECK written *after* an FK still takes the name first. Three fixtures pin
the three arrangements; none follows from the others. **Inside one foreign key** the
order is `ATAddForeignKeyConstraint`'s: the referenced relation (`42P01`), each list's
columns (`42703`), the referenced list's repeat and the covering key, then the arity
(`42830`), and the column pairs' types (`42804`) last — so a key's `42804` outranks a
*later* key's duplicate name or missing parent and loses to the same two errors on its
own. pglet's parser refuses a *written* list's arity mismatch once it has read the key's
clause — whose own `MATCH` and `ON DELETE` / `ON UPDATE` refusals (`MATCH FULL`'s and `SET
DEFAULT`'s `0A000`, a malformed action's `42601`) come first — so ahead of all of these, of
anything the parser would refuse after that clause, of every verdict past the parse and of
an `IF [NOT] EXISTS` skip: `FOREIGN KEY (a, b) REFERENCES nosuch (k)` is `42830` here and `42P01` there (`EDGES.md` row 275); the arity
the resolver checks is an omitted list's, resolved to the parent's primary key. On `ADD
COLUMN` the test's place beside the column's own index build depends on whether 18.6
rewrites the table: the key is added in the pass after the index's, whose build runs in
that pass without a rewrite and is deferred into the rewrite with one — so the test sits
below the build for a constant default, and above the build and the rewrite's `NOT NULL`
and `CHECK` verdicts for a per-row-volatile one (`EDGES.md` row 195).
**Between one constraint and the next** it bites once more, a scale below that:
`AddRelationNewConstraints` cooks each constraint and *then* names it, per
constraint, so constraint 2's duplicate name beats constraint 3's ill-typed
expression (`42710`) while a duplicate name on constraint 2 *itself* loses to
constraint 2's own bad expression (`42883`). No arrangement of one whole-list
type pass and one whole-list name pass yields both — it has to be one
interleaved loop, the same shape the expression walk needs one scale further
down. **Between parse analysis and execution** it bites hardest, because the
split is invisible in the statement text: `CREATE TABLE`'s index constraints
are resolved in `transformIndexConstraints`, so a second `PRIMARY KEY`
(`42P16`) outranks the duplicate CHECK name, the missing FK parent, the
ill-typed CHECK, *and* the already-existing table in the very same statement —
while `IF NOT EXISTS`, tested at the top of the same pass, outranks it in turn.
On the `ALTER` paths the same check sits inside `DefineIndex` **before**
`index_create`, so there `42P16` beats the name checks but the column
resolution beats it. `EDGES.md` records what still
sits outside the order. **Within `CREATE TABLE`'s own parse analysis** it
splits once more, and this is the layer the `42P16`-beats-execution rule does
*not* determine: column types resolve in the `tableElts` walk
(`transformColumnDefinition` → `transformColumnType`), so `42704` outranks
`42P16` and the key-column `42703` alike — and the modifier refusal
(`42601 type modifier is not allowed for type "…"`) sits with it, both being
verdicts of the one `resolve_column_type` call that walk makes per column, left
to right; then `transformIndexConstraints` runs
**per constraint**, testing the multiplicity at the top of
`transformIndexConstraint` and resolving that constraint's key columns further
down in the same call — so `PRIMARY KEY (nosuch), PRIMARY KEY (a)` is `42703`
while `PRIMARY KEY (a), PRIMARY KEY (nosuch)` is `42P16`, and no arrangement of
one whole-list count and one whole-list column check yields both — the same
interleave shape as the CHECK cook-then-name loop.

**And once more inside one key list** — the scale `validate_key_columns`'s own
doc comment is written at: the repeat test sits *below* each key's own
resolution in the same loop body, so `PRIMARY KEY (a, a, nosuch)` is `42701` and
`PRIMARY KEY (a, nosuch, a)` is `42703`. Whose pass the resolution belongs to is
what makes the two routes disagree rather than an ordering choice:
`transformIndexConstraint` skips it entirely under `!cxt->isalter`, so on
`ALTER TABLE … ADD CONSTRAINT` both spellings are `42701` and the key column's
`42703` arrives later, from `DefineIndex`. Three positions down the same
statement, none of them derivable from the one above it.

**`ADD COLUMN` puts the type the other way round, and the asymmetry is 18.6's.**
There `check_for_column_name_collision` runs *ahead* of the
`BuildDescForRelation` that resolves the type, so the relation's `42P01`, the
column's `42701` (in **both** of that function's arms — the user column's
*already exists* and a **system** column name's own sentence, which carries no
`if_not_exists` test at all) and the `IF NOT EXISTS` skip each outrank the
verdicts *that resolution* reaches — `42704`, the modifier `42601`, the identity `22023`, and the
**carrier's** own `22023` beside the `22003`/`22P02` of the `int4` coercion that
precedes it (`ADD COLUMN b char(0)` / `b numeric(0)` / `b numeric(1.5)` all lose to a
missing relation and to the skip, measured on both engines) — where on `CREATE TABLE`
the `tableElts` walk outranks even the `42P07`. Both
measured on both engines; pglet mirrors the asymmetry rather than normalising it
(`EDGES.md` rows 119's `ADD COLUMN` table and 124).

**A type-name verdict the *grammar* raises sits outside that resolution and beats
all three**, because bison reduces the production before either the collision
check or the type resolution runs. `opt_float`'s `22023` is one such — `gram.y`'s
`opt_float` action `ereport`s it in the rule body — and it lands ahead of the
column's `42701`, ahead of the relation's `42P01`, and ahead of the
`IF NOT EXISTS` skip, a Postgres **success**: measured on both engines,
`ALTER TABLE t ADD COLUMN IF NOT EXISTS b float(0)` over an existing `b` is
`22023 precision for type float must be at least 1 bit`, and
`ALTER TABLE nosuchtable ADD COLUMN b float(0)` is that same `22023` rather than
`42P01`. It is not alone in the class: the temporal modifier grammar's `42601`s
land the same way, `ALTER TABLE nosuchtable ADD COLUMN b timestamp(3,1)` and
`… b interval(-1)` each being `42601` on both engines — **and so does the character
length**, read by that same one-`Iconst` reader: `ALTER TABLE nosuchtable ADD COLUMN
b char(3.5)` and `ALTER TABLE t ADD COLUMN IF NOT EXISTS b char(2147483648)` over an
existing `b` are `42601` on both engines, each with its own fixture
(`types/refused_typmod_grammar_length_beats_missing_table_loud.sql`,
`types/refused_typmod_grammar_length_beats_add_column_skip.sql`), since inheriting the
order is not inheriting the evidence. The contrast is a spelling
whose refusal *does* need the resolved type — `int4(3)` loses to all three, and
`ALTER TABLE t ADD COLUMN IF NOT EXISTS b int4(3)` over an existing `b`
**succeeds** on both engines (18.6 raising a `skipping` notice), because
`typenameTypeMod` runs after them. Same shape, opposite winner at each spelling,
decided by which phase the refusal lives in — `docs/coercion.md` § A canonical
type name decomposes once states the rule, `EDGES.md` rows 116 and 124 the record.

**And a gate that lives in the wrong *phase* of pglet's own pipeline cannot be
placed at all**, which is why a column `DEFAULT`'s verdicts are raised in the
executor. Every refusal 18.6 raises for one — a subquery, an aggregate, a window, a
set-returning function, a column reference, a `$N`, a bare `DEFAULT` — is a
parse-analysis error there, *behind* whatever 18.6 decides earlier in the
statement — the column type, the key-list pass, the relation's column-name set,
the table's own `42P07`, an `ALTER`'s `42P01`/`42703`; § A column `DEFAULT` is
an expression the executor cooks carries the measured members and the route
split — and *ahead* of the constraint naming
block. Raised in pglet's **parser** they would sit above all
of those, and no reordering inside the parser could fix it: the
statement-level gates run in the executor. So the verdict is `executor::ddl`'s
`cook_column_default`, reached because `ColumnDefault` *carries* a general expression
to the executor. What is still raised while parsing is enumerated rather than
counted: pglet's absent-function refusal
pre-empts every positional verdict (`DEFAULT (count(*) + nosuchfunc(1))` is
18.6's `42803` and pglet's `0A000`), and the **set-quantifier** verdict
(`EDGES.md` row 51) at both of this site's parsers
(`DEFAULT (upper(DISTINCT 'a'))` and `DEFAULT nextval(DISTINCT 's')` are
`42809` here, where 18.6 cooks a default behind the column type's `42704`, the
table's own `42P07` and the rest of the parse). That second member **carries
18.6's argument transform with it** — a quantified call's constant cast is
coerced in the parser too, so `DEFAULT upper(DISTINCT 'a'::int)` is `22P02`
here ahead of the same gates — and the quantifier is what bounds it: run for an
*unquantified* call the transform pre-empts measured DDL statements' own error codes
and `IF NOT EXISTS` skips, the worst of them a **success**
(`CREATE TABLE IF NOT EXISTS <existing> (b TEXT DEFAULT upper('a'::int))`, a
skip on 18.6 and here, `22P02` with the transform unquantified). A member of this
enumeration is a *refusal pglet raises early*, so widening one of them is joining the
enumeration, not implementing it. A plain **later syntax error**, by contrast, agrees:
`CREATE TABLE k (a INT, b INT DEFAULT now,)` is `42601` on both, because a gate
outside the parser is ordered against the parser's *own* errors as well as against
the executor's.

**The grammar a clause takes is the same kind of fact, and it is a *mode*
rather than a binding power.** A column constraint's `DEFAULT` takes `gram.y`'s
restricted `b_expr` while `ALTER COLUMN … SET DEFAULT` takes full `a_expr`. The
excluded predicate band and `AT TIME ZONE` bind *tighter* than the comparisons
`b_expr` includes, and `NOT` is a prefix production that no `min_bp` gates at
all — so a binding-power threshold standing in for it diverges in **both** directions
at once (`DEFAULT 5 = 4` refused where 18.6 accepts, `DEFAULT NOT true`
accepted-shaped where 18.6 refuses). A scoped flag is consulted by every arm
reading a production `b_expr` lacks, and **that list is the contract, not a
count**: the `NOT` prefix, the `IS NULL` family (`IS [NOT] DISTINCT FROM`
stays), the predicate band (`IN` / `BETWEEN` / `LIKE` / `ILIKE`),
`AT TIME ZONE`, and the **quantified comparison** (`= ANY (…)` / `ALL` /
`SOME`, `gram.y`'s `a_expr subquery_Op sub_type …`, which has no `b_expr`
twin). A count hides an omission — a reader checking a number cannot notice that a
production is missing from it — so the enumeration is spelled and re-derived from
`gram.y`'s `b_expr` block rather than carried forward. What makes the mode correct rather than merely stricter is that a `(`
**resets** it to `a_expr` and the reset is *restored* on the way out, so
`1 + (NOT true)::int` and `(5 = ANY(ARRAY[1,2]))` are accepted while
`(now()) AT TIME ZONE 'utc'` and `(5 IS NULL) IS NULL` are `42601` — measured
on both engines. The reset lives in `parse_primary`, which is the same
enumeration as the bracketing constructs rather than a list of them that can
fall behind one.

**`COLLATE` is on `b_expr`'s exclusion list and is not this mode's to
enforce**, which is worth stating because the list invites the inference:
`DEFAULT 'x' COLLATE "C"` is **accepted** on 18.6 in the column-constraint
position (measured; `column_default` reads `'x'::text`), because the `COLLATE`
is a `ColConstraint` element of its own sitting *beside* the `DEFAULT`, not
part of its expression. pglet refuses it `42601`, and the control that
identifies which gap that is: the bare `v TEXT COLLATE "C"`, with no `DEFAULT`
anywhere, is the same refusal. It is the no-`COLLATE` gap, not the grammar mode.


## A column `DEFAULT` is an expression the executor cooks

`ColumnDefault` carries an `Expr` catch-all beside its recognized closed forms, so `DEFAULT <any expression>` is a capability rather than a refusal: the
tree is stored **unevaluated** and evaluated per row, which is 18.6's own model
(`pg_attrdef` holds the post-parse-analysis tree, so `DEFAULT 1+1` reads back
`(1 + 1)` rather than `2`, and `DEFAULT nextval('s') + 1` draws a fresh value on
every insert). The whole verdict — the constructs Postgres forbids, the DDL-time
coercion, and pglet's own gates — lives in `executor::ddl`'s
`cook_column_default`, at the position `AddRelationNewConstraints` cooks a
default, rather than in the parser; carrying a general expression to the executor is
what makes that possible. What still diverges is `EDGES.md` row 47's.

**The statement-level precedence is Postgres's, measured from both sides — and
the two halves below are not a partition.** What outranks a bad DEFAULT is
whatever 18.6 decides earlier in the same statement, so the list grows with
every gate put ahead of the cook and is read as measured members rather than as
a closed set. On `CREATE TABLE` it **loses** to the column type's `42704`, the
`transformIndexConstraints` pass (`42P16`, a key column's `42703`, a repeated
key column's `42701`), a declared identity `SEQUENCE NAME` collision (`42P07`),
the relation's column-name set (a duplicate and a system-owned name, each
`42701`) and the table's own `42P07`; on the `ALTER` routes, to the target
relation's `42P01`, to both names one collision gate answers — a column the
table already has and a system-owned one, each `42701` — and to the column's
`42703`. It **beats** the constraint *naming* block sitting behind the cook: a
duplicate CHECK name (`42710`, in either written order), an
ill-typed CHECK (`42883`), a taken constraint-index name (`42P07`), a missing FK
parent (`42P01`), a repeated column in a foreign key's *referenced* list
(`42830`), and on `ADD COLUMN` the inline `PRIMARY KEY` multiplicity
(`42P16`) — which is the same `42P16` the `CREATE TABLE` route puts above the
cook, so a member of one route's list is not a member of the other's by
inheritance. "The constraint block" names that naming block and not every
constraint gate: a `PRIMARY KEY`/`UNIQUE` key list is validated in the pass
above, so `CREATE TABLE p (a INT DEFAULT 'zz', PRIMARY KEY (a, a))` is `42701`
and not `22P02` on both engines, while its FK-referenced twin
`… FOREIGN KEY (a, b) REFERENCES pp (x, x)` is the default's `22P02`. The
`ALTER` spelling of that first pair needs two actions in one statement and
pglet's parser takes one, so the comma is `42601` there before either gate is
reached — measured, on the plain two-`ADD COLUMN` control too, which is what
says it is the action list and not the pair.
Between two bad defaults, **declared column order** wins — pinned by
a *cross-code* pair (`22P02` vs `22007`, swapping with the column order), which
is what an `@expect-error` fixture can hold on the SQLSTATE alone: a same-code
pair is satisfied by an engine that answers that code to everything, and is told
apart by `@expect-message`, which locks the sentence instead. The duplicate-column
row of that contract **agrees**: 18.6 reports `42701` for a duplicate column ahead of
the cook, and so does
pglet — measured off-corpus, no fixture's `CREATE TABLE` both repeating a
column name and carrying a `DEFAULT`.

**A plain syntax error later in the statement wins on both engines.** bison scans
the whole statement before parse analysis, so a syntax error after the default beats
the DEFAULT verdict on 18.6; pglet's parser has no separate scan pass, and agrees
because the verdict is not the parser's: measured on both engines,
`CREATE TABLE k3 (a INT, b INT DEFAULT now,)` and
`CREATE TABLE k5 (a INT, b INT DEFAULT now) GARBAGE` are `42601` on each, and
the well-formed control `CREATE TABLE k6 (a INT, b INT DEFAULT now)` is
`0A000 cannot use column reference` on each. A gate outside the parser is ordered
against the parser's own errors as well as against the executor's. What is **not**
closed is
every refusal pglet still raises while parsing — the enumeration in § Which
error wins, of which a **niladic builtin called with arguments** is a member
this clause inherits rather than decides: the walk itself sees an `Expr::Builtin`
with nothing left to object to, and the refusal is upstream of it, at the one
site a call is built (`build_call`), which is why `DEFAULT (now(1))` is `42883`
here without the walk gaining an arm — and, being a parse-time refusal, one more
member of that enumeration rather than a class of its own
(`CREATE TABLE t2 (a INT DEFAULT (now(1)), b nosuchtype)` is 18.6's `42704` and
pglet's `42883`).

**The render probe reaches this site and the three persisted-expression sites
beside it**, which is § Gating a stored expression is a DDL-time concern's rule
seen from the `DEFAULT` end: `executor::ddl`'s `validate_stored_expr_renders`
covers a CHECK, an index predicate and an index key part — without it
`CHECK (t > now() - interval '1 day')` is accepted at every CHECK spelling
(over three executor call sites) and `CREATE INDEX … ((iv + interval '1 day'))` /
`… WHERE iv > interval '1 day'` at both index positions, each making
`pglet_server dump` exit 1 for the whole database. At the `DEFAULT` site the
probe is still the cook's last step rather than a whole-statement pass, which is
`EDGES.md` row 47's residual.

**The `b_expr` mode's own arms, measured at this site.** Beside the members
§ Which error wins enumerates: `DEFAULT 5 = 4` and `DEFAULT 5 IS DISTINCT FROM 4`
are accepted, `DEFAULT NOT true` and `DEFAULT now() AT TIME ZONE 'utc'` are `42601`,
and `IS NULL` / `AND` / `OR` / `IN` /
`BETWEEN` / `LIKE` stay `42601`. The `SET DEFAULT` spelling of each is accepted
on both engines and is the control that identifies the mode rather than the
construct. The quantified comparison is `a_expr subquery_Op sub_type …` with no
`b_expr` twin, so 18.6 answers `42601 syntax error at or near "ANY"`, and so does
pglet, on `CREATE TABLE` and on `ADD COLUMN` alike: it is gated at the quantifier
keyword before its operand is read, the shape the `NOT` arm has, and pinned by `ddl/default_b_expr_excludes_a_quantified_comparison*`,
`ddl/add_column_default_b_expr_excludes_a_quantified_comparison`,
`ddl/default_quantified_comparison_is_admitted_parenthesized` and
`ddl/alter_column_set_default_a_expr_admits_a_quantified_comparison`.

**`ADD COLUMN` evaluates its default; the other two routes only store it.** 18.6
computes the added column's `attmissingval` once at the `ALTER`
(`StoreAttrMissingVal`), so the whole *value-dependent* family is a DDL error on
that route — and over an **empty** table, which is what separates it from a
backfill failing per row. Measured on 18.6 over a table with no rows:
`ADD COLUMN a INT DEFAULT 1/0` is `22012`, `(2147483647 + 1)` is `22003`,
`VARCHAR(3) DEFAULT 'abcdef'` is `22001`, `NUMERIC(4,2) DEFAULT 12345.6` is
`22003` — and the identical four on `CREATE TABLE` and on `SET DEFAULT` are all
accepted, there and here. pglet takes a `DefaultTiming` at the one call, because
the position needs nothing reordered:
measured on the same route, the deferred family sits exactly where the cook-time
family does — behind `42P01` / `42704` / `42701`, ahead of the inline
`PRIMARY KEY` multiplicity, a taken constraint-index name, a duplicate inline
CHECK name, an ill-typed inline CHECK and a missing inline `REFERENCES` parent
(`ADD COLUMN e INT
NOT NULL DEFAULT 1/0` is `22012` and not `23502`; `… DEFAULT 1/0 CHECK (c > 100)`
is `22012` and not `23514`). **A
sequence-bearing default is excluded from the eager arm on purpose**: the cook's
`nextval` stand-in is `1` whatever the counter holds, so over a sequence at 5,
`ADD COLUMN c INT DEFAULT 100 / (nextval('s') - 1)` would be refused here for a
division 18.6 never performs — an eager verdict about a value the statement does
not produce. Pinned by `ddl/add_column_default_*_is_eager` and the
`create_table_` / `alter_column_set_default_` deferred controls, which are what
make the refusals a statement about the *route*.

**A column carries one default, and a `SERIAL` type name is one of the two ways
to spell it.** `transformColumnDefinition`'s `saw_default` is set by the implicit
`nextval` a `SERIAL` synthesizes, so `a SERIAL DEFAULT 5` is the same
`42601 multiple default values specified for column "a" of table "t"` as
`a INT DEFAULT 5 DEFAULT 7`. `GENERATED … AS IDENTITY` takes its own sentence
(`both default and identity specified`), in either written order. All measured;
a last-wins assignment would silently keep the second clause and, for the `SERIAL`
spelling, drop the implicit sequence the type name asked for.

**A SQL value function takes no argument list, so `DEFAULT
current_timestamp()` is `42601`.** The token-level classifier's `eat_empty_call`
declines that spelling — taking it would define the column and backfill a real
timestamp where 18.6 defines no default at all — and the general grammar answers with
the message the parenthesized spelling gives.

**A `DEFAULT` whose cooked expression is a NULL constant is no default**, and
the test is 18.6's own: `coerce_type` folds an `unknown` literal into a typed
`Const` in place, so `NULL::int`, `(NULL)::int` and `CAST(NULL AS int)` each
store **zero** `pg_attrdef` rows while `NULL::text::int` — a second cast, a real
conversion rather than a constant — **is** stored. `1 + NULL` is stored too, and
is the control that identifies the rule as *a constant* rather than *contains a
NULL*. All measured; the reduction looks through exactly one cast over a bare
`NULL`.

**A decimal literal is a `numeric` constant.** `DEFAULT 1.50` keeps its display
scale (18.6's `column_default` reads `1.50`, where an `f64` hop reads `1.5` —
silently, the stored value agreeing and the reported default not), and an
out-of-`i64` integer stays exact. And **`DEFAULT LOCALTIMESTAMP` / `LOCALTIME` are
accepted**, through the catch-all rather than a closed arm, and `column_default`
reads `LOCALTIMESTAMP` / `LOCALTIME` on both engines.

## Extensions are a name check, not a catalog

`CREATE EXTENSION [IF NOT EXISTS] name [WITH] [SCHEMA s] [VERSION v] [CASCADE]`
and `DROP EXTENSION [IF EXISTS] name [, …] [CASCADE|RESTRICT]` parse with their
full option tails and lower to a `Plan::Noop` carrying the statement's command
tag. **pglet models no extension catalog**, and the reason is that the two
extensions a ported migration actually asks for are already *in* the engine:
`pgcrypto`'s `gen_random_uuid()` and `uuid-ossp`'s `uuid_generate_v4()` are
ordinary builtins, resolvable with or without the statement, and `plpgsql` is
what the `DO $$ … $$` block is written in. There is no installation state to
record, so the statement has nothing to do.

What is left are the decisions that stay knowable without state, and they are
knowable exactly. The first is the **name**: is it one this engine provides
(`parser::ddl`'s `KNOWN_EXTENSIONS`)? An unknown one takes 18.6's SQLSTATE for
an extension it can find no control file for — `0A000`, the code that server
gives under `IF NOT EXISTS` too (measured, and the reason the name check runs
ahead of the existence question here as well) — while a `DROP` naming one
without `IF EXISTS` is `42704`, and with it a skip. The names are compared
**exactly**, against a name the parser has already folded, so an unquoted
`PGCRYPTO` matches and a quoted `"UUID-OSSP"` does not, both as 18.6 answers
them. The second is the `SCHEMA` option: pglet models exactly one schema, so
`CREATE EXTENSION pgcrypto SCHEMA nosuchschema` is `3F000` here as there,
with no extension state needed to say so.

**The message deliberately does not copy 18.6's.** That server says
*is not available*, and for a name it also lacks the two engines then agree —
but 18.6 *ships* `citext`, `hstore`, `btree_gin`, `pg_trgm` and the rest, and
answering "not available" for those would be a false claim about the system
rather than a true one about this engine. The set here is what **pglet**
provides, so the message says so (`is not provided by pglet`) and only the
SQLSTATE is borrowed.

**Most of the residual is the shape a persisted set would close**, and it is an
over-acceptance in one direction: without installation state a *bare repeat*
(`CREATE EXTENSION pgcrypto` twice) succeeds here where 18.6 answers
`42710 extension "pgcrypto" already exists`, and so does a bare
`CREATE EXTENSION plpgsql`, which 18.6 refuses from the very first statement
because it ships pre-installed. `DROP EXTENSION pgcrypto` with nothing created
succeeds here against 18.6's `42704`. And a `DROP SCHEMA public` without `CASCADE`
over a `public` whose only object is an extension succeeds here where 18.6 answers
`2BP01` (`extension pgcrypto depends on schema public`): the refusal counts the
tables, enums and sequences `public` holds, and there is no extension to count
(`EDGES.md` row 72). The `IF NOT EXISTS` spelling — what every migration writes —
agrees on both engines, which is what the trade is for: a persisted set costs a store-magic bump and a `docs/storage.md` ledger
line for a fact no query reads, since `pg_extension` is absent here in either
model.

**One residual a persisted set would *not* close**, so it is recorded
separately rather than folded into the paragraph above: the `VERSION` option is
parsed and dropped, so `CREATE EXTENSION pgcrypto VERSION '99.9'` succeeds here
where 18.6 answers `22023 … has no installation script nor update path for
version "99.9"`. Knowing that needs a per-extension version list, which is a
different fact from *whether* an extension is installed.
