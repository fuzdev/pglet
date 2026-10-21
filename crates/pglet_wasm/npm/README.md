# @fuzdev/pglet-wasm

> fast and small embeddable postgres for speeding up your tests

[pglet](https://github.com/fuzdev/pglet) — a small Postgres-compatible-ish embedded engine
in Rust — as an in-process WebAssembly database with a `pg`/PGlite-shaped adapter. Small
(~647 KB gzip) and fast to start, it's built for **test fixtures**: spin up a real
Postgres-dialect DB per test, seed it, `fork()` it, throw it away. Measured on a real
production app's test suite: **~7.5× faster than pglite** on the suite's engine work,
at **~1/8.2 the gzipped size** (~647 KB vs pglite 0.5.8's ~5.2 MB wasm + data).

> Early-maturity: great for tests and scratch DBs, **not yet a store of record**. pglet's
> internals are largely LLM-generated from PostgreSQL's documented behavior, held to a
> differential harness that compares every feature against a live PostgreSQL — the
> [project README](https://github.com/fuzdev/pglet) has the full background and
> status, and whatever isn't implemented aims to fail loud rather than approximate.

## Install

```
npm i @fuzdev/pglet-wasm
```

Node 22 or later (`engines`), or a browser through a bundler.

## Use

```js
import {create_pglet_db} from '@fuzdev/pglet-wasm';

const db = create_pglet_db();
await db.query('CREATE TABLE users (id BIGINT PRIMARY KEY, name TEXT, tags TEXT[])');
await db.query('INSERT INTO users VALUES ($1, $2, $3)', [1, 'ada', ['a', 'b']]);

const {rows} = await db.query('SELECT * FROM users WHERE id = $1', [1]);
// rows[0] = { id: 1n, name: 'ada', tags: ['a', 'b'] }

await db.transaction(async (tx) => {
	await tx.query('UPDATE users SET name = $1 WHERE id = $2', ['ada lovelace', 1]);
});

const forked = await db.fork(); // cheap CoW branch of the committed state — the per-test primitive
await db.close();
```

On the **browser** (a bundler picks the `default` entry) initialize the wasm first; on
**Node** it auto-initializes at import, so `create_pglet_db()` just works:

```js
import {init, create_pglet_db} from '@fuzdev/pglet-wasm';
await init();
const db = create_pglet_db();
```

`init()` fetches `pglet_wasm_bg.wasm` from beside the entry (`new URL(…, import.meta.url)`).
Vite and webpack 5 emit that file with the bundle; esbuild does not — copy
`pglet_wasm_bg.wasm` next to the bundle, or pass its URL as `init({wasm})`.

## Test fixtures — fork per test

**`fork()`** is a cheap copy-on-write branch of the committed
state, so migrate/seed a base **once** and hand each test an isolated copy — no re-running
schema SQL, no cross-test leakage.

```js
import {create_pglet_db} from '@fuzdev/pglet-wasm';

// once, in setup
const base = create_pglet_db();
await base.query('CREATE TABLE users (id BIGINT PRIMARY KEY, name TEXT)');
await base.query(`INSERT INTO users VALUES (1, 'seed')`);

// per test
const db = await base.fork(); // sees the seed; writes are isolated from base and siblings
await db.query(`INSERT INTO users VALUES (2, 'test')`);
// ... assertions ...
await db.close(); // base is untouched, still forkable
```

A fork can be forked, and forks are independent; `close()`ing one never disturbs the base.
`fork()` runs on the instance's lane after every statement issued before it, awaited or not, so it carries them.
Called from inside a running `transaction()` callback it rejects `25001` instead (see *Behavior*).

`fork()` branches **committed** state, so it rejects `25001` while a block is open on the
instance — commit or roll back the seed first. Serving it instead would hand the fork the
block's catalog writes over a heap without its rows: a pending `DROP TABLE` would hide a
committed table, a pending `ADD COLUMN` would widen the descriptor past the rows, and a
whole suite of fixtures would come out well-formed and empty.

## Value coercion

The wasm boundary hands JS the natural shapes; the adapter applies **one tested policy**,
recursing into array elements. A query that carries `pg`
type parsers takes those instead (next section):

| pglet type | JS value | note |
|---|---|---|
| bool | `boolean` | |
| int2 / int4 | `number` | |
| **int8** | **`bigint`** | configurable — `{coercion: {int8: 'number' \| 'string'}}`; default `bigint` (no precision loss, and the value the engine hands over with no conversion) |
| float4 / float8 | `number` | a float4 is the number its text reads as (`1.1`, not the widened `1.100000023841858`), as node-postgres reads it; the text is Postgres's at the default `extra_float_digits` |
| numeric | `string` | its text, so no digit is lost, as node-postgres hands it over — `avg()` of an integer and `sum()` of a `bigint` are `numeric` too (`'1.5000000000000000'`) where pglet is sure of the argument's integer type — mostly a column (a table's, a `VALUES` list's, a derived table's or a `UNION`'s) or arithmetic over columns; the exceptions are below |
| text / varchar / uuid / enum | `string` | |
| bytea | `Buffer` / `Uint8Array` | `Buffer` when available |
| timestamp / timestamptz | `Date` | parsed as UTC (pglet ≡ Postgres at `TimeZone=UTC`); `infinity` / `-infinity` are the numbers `Infinity` / `-Infinity`, as node-postgres answers them |
| **date** | **ISO `string`** | configurable — `{coercion: {date: 'date'}}` for a local-midnight `Date`, under which `infinity` / `-infinity` are `Infinity` / `-Infinity` |
| time / interval | `string` | Postgres's text (`'12:34:56'`, `'1 day 02:00:00'`); node-postgres parses an interval into an object |
| jsonb | parsed value | |
| arrays | element-wise | the same policy applied to each element |

### `pg` type parsers

A query carrying `pg`'s `types` hook — `query({text, types}, values)` (the `QueryConfig` shape), or `types` in the
string form's options — gets `pg` 8.16.3's contract instead of the table above: each non-NULL cell's **text form**,
the text a Postgres text-protocol client receives, goes through `types.getTypeParser(dataTypeID, 'text')`, asked
once per result field; a NULL stays `null` without the parser being called, and the coercion above does not also
run. The text is pglet's, which is Postgres's except where it is recorded otherwise: a `char(n)` unpadded, an enum
column declared `text` (OID 25) rather than its own type, and a `float4` / `float8` under `SET extra_float_digits` below
`1`. Dates, timestamps and intervals are ISO, UTC and `postgres`-style text: a `SET DateStyle`,
`IntervalStyle` or `TimeZone` to anything else is refused (`0A000`) rather than applied.

**Params go the other way too:** pass a `bigint` for an int8 column (so a value read back as a
`bigint` round-trips), a `Date` for a timestamp (sent as UTC), or arrays of them — the same
policy in reverse. For a `date` column, pass a `'YYYY-MM-DD'` string. A string param that holds
a NUL character (`'\0'`) is refused with a `22021` error, since Postgres text cannot hold one.

A **byte view** — a `Buffer`, a `Uint8Array` or any other typed array, a `DataView` — is bound as
its bytes, the ones its `byteOffset` and `byteLength` select, as `pg` sends it: a `bytea` parameter
stores them, a `text` or `varchar` parameter (and one with no type of its own, `SELECT $1`) reads
them as text (`22021` if they are not UTF-8), and as an element of an array parameter it is `\x`
and their hex — the bytes themselves for a `bytea[]`. A scalar parameter of any other type refuses a
view (`22023`) — an enum or `char(n)` one aside, which reads it as text — and an array parameter bound to a
bare view is an error with no code. A `bytea` parameter also takes a string, read as PostgreSQL reads one (`'\x6162'`, or
the escape format), a number or a boolean (the bytes of its text) and a JS array (the bytes of the
array literal below); any other object is refused (`22023`) — see *Known differences*.

A `text` or `varchar` parameter takes whatever it is given, so it reads a value as the text `pg`
sends for it: a JS
array is an array literal with each string, number and boolean element quoted (`[1, 2]` →
`{"1","2"}`), an object is its JSON, a number is `String(n)` — except a whole number between 2^53
and 2^63 in magnitude, which is spelled with all its digits (`2**60` is `1152921504606846976` here and
`1152921504606847000` through `pg`). The values that differ — a `Date` among them — are under
*Known differences*.

## API

- `create_pglet_db(options?)` → `PgletDb` — `options.coercion` sets the two knobs above
- `PgletDb.query<T>(sql, params?, {rowMode?, types?})` → `Promise<{rows, rowCount, oid, fields, command}>` (`rowMode: 'array'` → positional rows, a repeated column name's every value kept; object rows keep its last, as `pg`'s do; `types` → `pg` type parsers); also `query(config, values?)` with a `pg` `QueryConfig` (`text`, `values`, `rowMode`, `types`; `name` ignored)
- `PgletDb.transaction(fn)` — BEGIN/COMMIT, ROLLBACK on throw; `fn` gets a `tx` handle whose `query` runs inside the transaction; an outside `query()` serializes behind it (see Behavior)
- `PgletDb.fork()` → `Promise<PgletDb>` — cheap CoW branch of the committed state
- `PgletDb.close()` → `Promise<void>` — runs what is queued, then frees the wasm instance; the object also carries `[Symbol.asyncDispose]` (the same function) where the runtime has the symbol — undeclared in the types, so `await using` type-checks only with your own declaration
- `init(options?)` → `Promise<void>` — initialize the wasm outside Node (Node initializes at import, and `init` is a no-op there); `options.wasm` is a URL, `Response`, bytes or a compiled module to load instead of the file next to the entry. Calls made while a load is in flight share it (the first call's `wasm` decides), a call once it is ready resolves at once, and a call after a failed load starts a new one
- `init_sync({wasm})` — the synchronous form, from bytes or a compiled module; refused while an `init()` is loading
- `PgletDatabaseError` (type) — the shape a failed statement rejects with: `code` (the SQLSTATE), `severity`, and, where known, `detail`, `hint`, `position`, `schema`, `table`, `column`, `constraint` — `pg`'s field names

Every method runs on the instance's one lane, in call order.

## ORMs

**kysely** — a ready `Dialect` ships as a subpath (`kysely` is an optional peer dep):

```js
import {Kysely} from 'kysely';
import {create_pglet_db} from '@fuzdev/pglet-wasm';
import {PgletDialect} from '@fuzdev/pglet-wasm/kysely';

const db = new Kysely({dialect: new PgletDialect({db: create_pglet_db()})});
```

int8 columns come back as `bigint` (the default above), where kysely's Postgres typings and
kysely-codegen expect `string`; pass `create_pglet_db({coercion: {int8: 'string'}})` to match them.
A transaction's isolation level reaches the engine as it reaches Postgres, and an access mode
pglet does not model (`READ ONLY`) is refused (`0A000`). Savepoints through kysely's API are not
implemented yet.

The package's tests drive kysely's CRUD queries, its transactions and its `Migrator` through the dialect —
migrating to the latest, a rerun that does nothing, a failing migration rolled back whole, and `migrateDown`.
The `Migrator` locks with `pg_advisory_xact_lock`, which on one in-process connection is always granted.

**drizzle** — its `node-postgres` driver wraps a `PgletDb` directly, no pglet-side code:

```js
import {drizzle} from 'drizzle-orm/node-postgres';
import {create_pglet_db} from '@fuzdev/pglet-wasm';

const db = drizzle(create_pglet_db());
```

The driver calls `query` with `pg`'s `QueryConfig` (`{name, text, rowMode, types}`), and `PgletDb.query`
honors its `types` as `pg` does (see *`pg` type parsers* above), so drizzle reads each column as it reads it over
node-postgres. Measured against drizzle over node-postgres on PostgreSQL 18.6 (its session at `TimeZone = 'UTC'`,
which is pglet's), over the `pg-core` column types and modes below — the integers, serials, `bigint` in
both modes, `real`, `double precision`, `numeric` in its three modes, `boolean`, `text`, `varchar`, `uuid`, `jsonb`,
`date`, `time`, `timestamp` in its date and string modes with and without a time zone, `interval`, an enum, and
arrays of them — every value, type and instant is identical in a process running in UTC and in one that is not; the
two that differ are listed after.
The package's tests drive CRUD, a committed and a rolled-back transaction, and a bound parameter of each coerced
type (`bigint`, `Date`, a `'YYYY-MM-DD'` string, a boolean, `text[]`, `bigint[]`, `Date[]`, a JSON value).

What still differs:

- **`char(n)`** reads back unpadded (`'ab'` where Postgres answers `'ab  '`): pglet stores `char(n)` as `varchar`.
- **`date({mode: 'date'})`** sends its `Date` as an ISO timestamp (`'…T00:00:00.000Z'`), which pglet's `date` input
  refuses (`22007`) where Postgres reads it — keep `date` columns in the default string mode.
- **Its migrator** starts with `CREATE SCHEMA IF NOT EXISTS "drizzle"`, and pglet has only the `public` schema, so
  `migrate(db, {migrationsFolder})` fails at that first statement (`0A000`). Pass `migrationsSchema: 'public'`:
  `migrate(db, {migrationsFolder, migrationsSchema: 'public'})` migrates, and a rerun applies nothing.

## Behavior

- **Result shape** — `{rows, rowCount, oid, fields, command}`, parsed from the command tag by
  node-postgres's own rule. The three tag-derived fields each take `null` on a *different* set of
  tags, so read them one at a time rather than as a group (`fields` is not one of them — it is a
  list, empty rather than null):
  - `command` — the tag's **first word** (`CREATE TABLE` → `CREATE`, `ALTER SEQUENCE` → `ALTER`).
    Never `null` for a statement this adapter accepts.
  - `rowCount` — the rows the statement **affected**, so an `INSERT`/`UPDATE`/`DELETE` without
    `RETURNING` reports its count over an empty `rows`. **`null`**, not `0`, for a tag that carries
    no count at all (`CREATE TABLE`, `BEGIN`, `DO`) — `0` is a real answer an `UPDATE` matching
    nothing gives.
  - `oid` — the `INSERT <oid> <rows>` middle number (`0` on any modern table); `null` for every
    other command.
  - `fields` — one per output column, `[]` when there are none: `pg`'s `FieldDef` keys (`name`,
    `tableID`, `columnID`, `dataTypeID`, `dataTypeSize`, `dataTypeModifier`, `format`) with the
    values `pg` reads off Postgres for the same column — `format` is always `'text'`, and the
    table OID is pglet's own. A computed column's `dataTypeModifier` is `-1`
    where Postgres sends a cast's (`1.5::numeric(4,1)` is `262149` there), and an enum column is
    declared `text` (see `index.d.ts`).
- **Transactions** — `transaction(fn)` runs `fn` inside BEGIN/COMMIT, ROLLBACK on throw, and
  resolves to `fn`'s return value. While one is open, a plain `query()` issued from outside it
  **queues** and runs *after* that transaction ends rather than joining it, so an audit/telemetry
  write survives a rollback (PGlite's per-instance mutex keeps it too; on a `pg` pool the outside
  statement gets its own connection). The cost is PGlite's too: a callback that **awaits** an
  outside `db.query()` deadlocks — use the `tx` handle for work that belongs to the transaction.
  Fire-and-forget is the
  spelling that works, and an ignored failure is still reported to the host (an
  `unhandledRejection` / `unhandledrejection`), not swallowed.
- **What the deadlock looks like** — nothing warns at runtime, deliberately (no timer can tell a
  deadlocked transaction from a slow healthy one holding a fire-and-forget write), so these are the
  measured symptoms, and the first one is the one to know:
  - `main()` called at the bottom of a module, no top-level `await`, no live handle: the process
    **exits 0 with no output at all**, on node, deno and bun. It does not hang — it stops mid-way,
    every statement after the deadlock goes unrun, and the shell reads success.
  - a top-level `await`: node warns *"Detected unsettled top-level await"* and exits 13; deno errors
    *"Top-level await promise never resolved"* and exits 1; bun hangs silently.
  - a live handle open (a server, a poller): hangs with no output (measured on node).
  - under a test runner: `deno test` fails in ~10 ms (*"Promise resolution is still pending but the
    event loop has already resolved"*); `node --test` cancels at the test timeout.
- **Deadlocks no refusal can see** — the same symptoms follow two more shapes:
  - a callback that awaits work which runs `db.transaction()` on the same instance from a context
    set up outside it — a job queue, a shared in-flight promise, a listener registered earlier, a
    request to your own server — waits forever, as on PGlite; no refusal can see it.
  - across two instances, A's callback awaiting `B.transaction()` while B's callback awaits
    `A.transaction()` waits forever, the lock-order deadlock of any two locks.
- **That queueing is `transaction()`'s only** — a transaction driven by a raw `query('BEGIN')`,
  the way kysely and drizzle drive them, presents no boundary `query()` can see: its statements
  and an outside one are the same `query()` on the same lane. An outside write issued between such a
  `BEGIN` and its `ROLLBACK` joins the transaction and is lost with it. That is the one-connection
  shape rather than a pglet one — PGlite loses it on the same spelling and keeps it on
  `transaction()`, the same boundary, and a single `pg.Client` loses it too; a `pg` **pool** keeps
  it by running it on a second connection. Use `transaction()` where an outside write must survive,
  or `fork()` a separate instance for it.
- **Concurrent `transaction()` calls queue; a nested one is refused** — a `db.transaction()`
  made while another one's callback is running waits for it and then runs, its own block after
  the first one's commit or rollback, as on PGlite or a `pg` pool — so concurrent request handlers
  that each run a transaction all succeed, in call order. One made from **inside** a running
  callback (through awaits, timers and helpers the callback calls) is refused loudly instead, since
  it would wait for the transaction it is part of and never run: use the `tx` handle for that work
  (`tx.query('SAVEPOINT …')` for a nested level), or a separate instance forked beforehand. Telling
  the two apart takes `AsyncLocalStorage`, which the Node entry — the one Node, Deno and Bun
  resolve — provides. **In a browser** there is none, so there a `transaction()` made while any
  callback runs is refused, nested or not, unless it was called before that callback began (in
  practice, the same synchronous turn); pglet refuses rather than guess, since a nested call that
  queued would hang silently. Start concurrent lanes up front there, or `fork()` an instance per
  lane. Which behaviour you get follows the `node` export condition: a bundle resolved without it
  (esbuild `--platform=browser`/`neutral`, a Workers target) gets the browser entry's refusal even
  where `AsyncLocalStorage` exists.
- **Single-connection contract** — one connection, so **one open block at a time**. A raw
  `query('BEGIN')` while a block is already open — the way kysely and drizzle drive transactions —
  **fails loud** with SQLSTATE `25001` instead of silently joining and clobbering shared state.
  For concurrent transactions, `fork()` a separate instance per lane (the per-test
  primitive above).
- **`transaction()` under an open block is a savepoint** — it issues `SAVEPOINT` / `RELEASE` /
  `ROLLBACK TO` rather than `BEGIN` / `COMMIT` / `ROLLBACK`, so a throw unwinds only its own level
  and the enclosing block keeps its writes and its decision. This is the rollback-per-test harness
  idiom: hold a `BEGIN` around each test, let the code under test call `transaction()` normally,
  and discard the block afterwards. A nested level's success is a `RELEASE`, which **merges** into
  the enclosing block rather than committing — the outer block can still discard all of it.
  Whether a block is open is read from the engine, so this holds for any spelling of it,
  comment-prefixed ones included.
- **Savepoints in SQL** — `SAVEPOINT n`, `ROLLBACK TO [SAVEPOINT] n` and `RELEASE [SAVEPOINT] n`
  work as on Postgres. Two rules: `ROLLBACK TO` is **repeatable** (it keeps the
  savepoint it names), and it is the one statement an **aborted** block still admits — so a failed
  statement can be rewound and the block carried on, instead of losing the whole thing.
- **Refusals that are not SQL errors** — a statement whose tree is deeper than the wasm
  limit is not one of them: it is refused `54001 stack depth limit exceeded`, an ordinary
  SQL error, and the db stays usable. But if a JS exception unwinds a call from outside the
  engine's reach — the JS engine's own stack overflowing, a trap such as running out of
  memory, or page code replacing a built-in the generated glue calls
  (`Array.prototype.push`) — the module refuses every later call, on every instance, with an
  `Error` named `PgletModulePoisoned` and no `.code`. Importing the package again does not
  recover it: on Node a second `import()` answers the module already loaded, poisoned instance
  and all. Restart the process — or run pglet in a `Worker` you can replace, since a Worker
  loads its own instance. A call made
  back into the module from JS a statement is running (a replaced `Date.now`, a parameter's
  getter) is refused with an `Error` named `PgletReentrantCall`.
- **Errors** — a failed query rejects with an `Error` whose `.code` is the Postgres SQLSTATE
  (e.g. `'23505'`; the `PgletDatabaseError` type); the db stays usable afterwards. **Inside a transaction the block aborts, as
  on Postgres:** every statement after the failure rejects with `25P02` (*current transaction is
  aborted, commands ignored until end of transaction block*) until the block ends, and the
  `COMMIT` after it is an implicit rollback — it resolves without error and reports
  `command: 'ROLLBACK'`, publishing nothing. So `BEGIN; INSERT …; <a duplicate-key INSERT>;
  SELECT 1; COMMIT;` leaves 0 rows, matching Postgres. `transaction(fn)` rolls back explicitly
  when `fn` throws; a callback that **catches** its own error and returns normally still commits
  nothing, because its `COMMIT` lands in the aborted block and converts. Recover with any exit
  statement — `ROLLBACK`, `COMMIT`, or either with a leading comment (`/* trace */ ROLLBACK`
  works: the classifier reads the lexer, not the raw text).
- **Time** — `now()`, `CURRENT_DATE` and the rest of the clock family read `Date.now()` when a
  statement runs, so a pinned clock — a `Date.now` or a global `Date` replaced, as a test
  runner's fake timers replace them — is the database's time, set before or after the first
  query, in every instance and fork, and the real clock again once it is restored. A read of the
  clock is in the first half of the millisecond `Date.now()` reports at that read, so a
  `timestamptz` or `timestamp` read back as a `Date` is that millisecond, however many statements
  ran, in a column of any precision from `(3)` up (`(0)` to `(2)` round the time, as they round any).
  `Date.now()` is whole milliseconds, so inside one millisecond each of the first five hundred
  reads — a statement reading the clock makes one, and one more at each `clock_timestamp()` — is
  a microsecond after the one before, and those statements keep their order: the first read of a
  pinned time answers it exactly and the second `…00.000001+00`, already later than a
  `new Date()` bound as a parameter (`created_at <= $1` matches the first row only; compare with
  `<` against `$1::timestamptz + interval '1 millisecond'`). The microseconds are counted across
  every instance and fork in the process, and from the five-hundredth read of one millisecond
  the reads tie (`…00.000499+00`) until a statement reads the clock in another millisecond —
  restoring the real clock restarts nothing until one does, so tests that pin the same instant
  share the five hundred. Rows stamped past it have equal times: order them by a second column,
  and a unique key over the time refuses them (`23505`). On the real clock that is reached by a
  `clock_timestamp()` evaluated for each row of one large statement, whose values then repeat —
  as PostgreSQL's own do. An open transaction keeps the time it started at when the clock is
  moved under it.
- **`close()`** — runs everything queued before it, then frees the wasm instance, and resolves
  once it is freed; `db.query(sql); await db.close();` keeps the write. It is idempotent, and any
  call after it rejects with a clear error (not a wasm panic). Called from outside a running
  `transaction()` callback it drains behind that transaction, as `fork()` branches after it. Called
  from inside one it rejects `25001` and leaves the instance as it was, as `fork()` does, since
  waiting for the lane there would deadlock. A callback still running when an outside `close()`
  arrives may go on calling `db.query()` — its fire-and-forget write is queued and lands before
  the free, rather than rejecting with nobody to catch it. On the browser entry, a `close()`
  accepted before a callback began gives that callback no such exemption: its write rejects as
  closed, reported as an `unhandledrejection` — so there, await a running transaction before
  closing. In a browser the two cannot be told apart, so there both
  reject while any callback is running.
- **The `tx` handle ends with its transaction** — called after the transaction settles, its
  `query` rejects rather than running outside any transaction.
- **Async shape** — every method returns a Promise to match `pg`/PGlite, though the engine is
  synchronous (no I/O).

## Known differences from PostgreSQL

pglet aims to answer as PostgreSQL 18 does and to refuse loudly where it can't. These are the differences a test
suite is likely to meet: answers that differ with no error, and statements refused that PostgreSQL accepts.
The [ledger](https://github.com/fuzdev/pglet/blob/main/EDGES.md) records the rest, row by row.

**Answers that differ, with no error:**

- **Text sorts byte by byte**, like PostgreSQL's `C` collation: `ORDER BY name` puts `B` before `a`, and `'a' < 'B'`
  is false, where a database created with a locale such as `en_US.UTF-8` sorts `a, A, b, B`.
- **Some errors wait for a row.** A table name that doesn't exist, and some type errors — an operator or function
  applied to the wrong type (`text_col + 1`, `lower(int_col)`), a bad constant in `UPDATE … SET`, an aggregate in
  `RETURNING` — are reported only when pglet evaluates them. So a statement over an empty table, or a subquery,
  `WITH` query or `CASE` branch that never runs, or the second half of an `AND` / `OR` that the first half already
  decides, can succeed where PostgreSQL rejects it: a misspelled table inside `EXISTS (…)` passes a test that runs it
  against empty tables, and `WHERE false AND text_col + 1 > 0` passes whatever the table holds.
- **A sequence's step, range and type options have no effect.** `INCREMENT BY`, `MINVALUE`, `MAXVALUE`, `CYCLE`
  and a sequence's `AS smallint` / `AS integer` are accepted (an identity column's `INCREMENT BY` too), and every
  sequence counts up by 1 from its `START WITH` (1 by default, whatever `MINVALUE` says), past any declared maximum.
- **Enum values work as text in string functions and `LIKE`**: `upper(status)`, `length(status)` and
  `status LIKE 'a%'` answer on an enum column, where PostgreSQL requires `status::text`.
- **`jsonb || '…'` and `jsonb || $1` join as text.** With the other side an uncast string literal or a parameter,
  `j || '{"x": 1}'` answers the text `{"k": 1}{"x": 1}`, where PostgreSQL reads that side as `jsonb` and merges the
  objects (`{"k": 1, "x": 1}`); either order. Compared with a literal or a parameter in a `WHERE`, it is the joined
  text that is compared, so `WHERE j || '{"x": 1}' = '{"k": 1, "x": 1}'` matches no row, and an `UPDATE` or `DELETE`
  filtered that way writes accordingly. Writing the result to a `jsonb` column is refused (`42804`), and with the
  other side cast (`'{"x": 1}'::jsonb`) the whole `||` is refused (`0A000`, *Not supported yet*, below). A `bytea`
  beside an uncast literal or a parameter is joined as text the same way: `'abcd'::bytea || 'ef'` is the text
  `\x61626364ef`, where PostgreSQL answers the `bytea` `\x616263646566`.
- **`avg()` of an integer and `sum()` of a `bigint` keep `double precision` and `bigint` where pglet cannot be sure
  of the argument's integer type.** Over a column a derived table defines by a window function, `unnest` of an
  `ARRAY[…]` constructor (of a cast array in the `FROM` clause it is sure: `FROM unnest($1::int[])`), `unnest` in a
  select list, a `JOIN … USING` column merged from an `integer` and a `bigint`, a recursive `WITH` query whose
  recursive step selects an untyped `NULL`, a `RETURNING` column of a data-modifying `WITH` query, a catalog view's
  column (`information_schema`, `pg_catalog`), an outer query's column inside a correlated subquery, a column a
  scalar subquery over one of those defines, arithmetic over an uncast parameter or an untyped `NULL`
  (`avg(qty + $1)`; `avg(qty + $1::int)` is sure), or a `COALESCE`, `CASE`, `greatest`/`least`/`nullif` or
  `ARRAY[…]` holding one of those,
  `avg()` is a `number` computed in floating point and `sum()` of a `bigint` a JS `bigint` (refused `22003` past its
  range), where PostgreSQL answers a `numeric` string. A cast (`avg(x::int)`) gives PostgreSQL's answer.
- **A `COALESCE`, `CASE`, `greatest`/`least` or `ARRAY[…]` over an exact `avg()` or `sum()` and a float expression
  pglet cannot type can return a `numeric` string where PostgreSQL returns a float.** The float side is arithmetic
  over one of the columns above or over a parameter: `COALESCE(avg(x), max(f) + $1)` is a `numeric` string
  (`'2.5000000000000000'`, the float side kept to fifteen digits), where PostgreSQL answers the `number` `2.5`. Cast
  the float side (`(max(f) + $1)::float8`) for PostgreSQL's answer.
- **The session time zone is always UTC, and a statement that would change it is refused.** `SET TIME ZONE` to a
  zone other than `UTC`, and `SET DateStyle`, `SET IntervalStyle` or `SET standard_conforming_strings` to a value
  other than the default (`ISO, MDY`, `postgres`, `on`), fail with `0A000` — through `set_config` too; the
  `INTERVAL '…' HOUR TO MINUTE` spelling of an offset with `42601` — where PostgreSQL applies them; so do a zero-offset zone under another name (`'GMT'`, `0`) and other spellings of the
  default date style (`'US'`, `'MDY, ISO'`). `SET TIME ZONE 'UTC'`, `SET DateStyle = 'ISO, MDY'` and a `RESET` are accepted. So a
  `timestamptz` converts to a date and renders as text in UTC; a test that depends on another zone needs a
  real PostgreSQL.
- **`real_col IN ($1, 0.1)` can miss a row.** Given two or more constants, PostgreSQL compares the list as `real`,
  and a row holding `0.1` matches. pglet does the same for a plain column against written constants
  (`real_col IN (0.1, 1.5)`). With a parameter, an expression or a subquery in the list, or anything but a table's own
  column on the left (`abs(real_col) IN (…)`, a subquery's or `WITH` query's column, a column of the `FROM` table in an `UPDATE … FROM`), and inside a `CHECK` constraint
  or a partial index's `WHERE`, it compares each item as `real_col = 0.1` does, which is false: the row is missed,
  `NOT IN` keeps it, an `UPDATE` or `DELETE` filtered this way writes accordingly, the `CHECK` fails with `23514`.
  Cast every constant (`real_col IN ($1, 0.1::real)`); a `double precision` column is not affected.
- **The read-only settings are accepted and not applied.** After `SET default_transaction_read_only = on`, or
  `SET transaction_read_only = on` inside a transaction, an `INSERT` succeeds where PostgreSQL refuses it (`25006`).
  `BEGIN READ ONLY` and `SET TRANSACTION READ ONLY` are refused (`0A000`).
- **Outside a transaction, `statement_timestamp()` equals `now()` in a query with parameters.** Through `pg`,
  PostgreSQL starts such a query's transaction a moment before it stamps the statement, so its `now()` is the earlier
  of the two and a row defaulting one column to each gets two times; pglet gives both the same time. `now() <=
  statement_timestamp()` holds on both while the clock is not set back inside an open transaction, and inside a
  `BEGIN` the two differ on both — on pglet, until the reads of one millisecond tie (**Time**, above).
- **`char(n)` behaves as `varchar(n)`**: values are not blank-padded, and trailing spaces count in comparisons and
  unique keys, so `'ab'` and `'ab '` are two different keys.
- **A single `UPDATE` shifting a unique column** (`SET position = position + 1`) can succeed where PostgreSQL
  reports a duplicate key, or the reverse. PostgreSQL's outcome follows its physical row order, which an earlier
  update of one of those rows changes — whatever column it set, `INSERT … ON CONFLICT DO UPDATE` included — and
  pglet's does not.
- **Identifiers longer than 63 bytes are kept whole.** PostgreSQL truncates a name to 63 bytes, so two names that
  differ only past the 63rd byte are one name there (the second `CREATE TABLE` is `42P07`) and two objects here, and
  a long ORM-generated index or constraint name reads back longer than PostgreSQL's.
- **`lower`, `upper` and `ILIKE` fold some non-ASCII letters differently.** pglet uses Unicode's full case mapping:
  `lower('İ')` is two code points, so `'İstanbul' ILIKE 'istanbul'` is false, `upper('ß')` is `SS` and a final sigma
  lowers to `ς` — where PostgreSQL answers true, `ß` and `σ`. ASCII letters, and letters like `Ä`, fold the same.
- **A bare carriage return does not end a `--` comment.** PostgreSQL ends the comment at a lone `\r`; pglet runs it
  to the next `\n`, so `DELETE FROM t -- note` + `\r` + `WHERE a = 1` deletes every row. Only CR-only line breaks
  reach it; a `\r\n` ends the comment on both.
- **A `Date` bound to a text or `bytea` parameter is spelled differently.** It arrives as UTC with
  no zone (`2024-01-02 03:04:05.678`), where `pg` sends the process's local time with its offset
  (`2024-01-01T22:04:05.678-05:00`); inside an array bound to such a parameter likewise, and a
  `bytea` stores that text's bytes. An object with a `toPostgres` method bound to a text parameter
  is sent as its JSON (`pg` calls the method). Pass the string you mean.
- **A JS number of 2^63 or more in arithmetic is read as a float.** `SELECT n + $1` or `SELECT $1 + 1` bound to
  `2 ** 63` answers `9.223372036854776e+18`, where PostgreSQL refuses the value for an integer (`22003`) or adds it
  exactly to a `numeric`. Cast the parameter (`$1::numeric`).
- **`nullif(col, $1)` with a fractional JS number returns a JS number.** `nullif(price, $1)` and `nullif(price,
  price * $1)` return a `numeric` column's value as a `number` (an `UPDATE` stores it so), where PostgreSQL
  returns the column's own value; cast the parameter (`$1::numeric`).
- **A fractional JS number multiplied into a `numeric` column is computed as a float, and beside an integer
  constant the product — a whole JS number's too — is rounded to a whole number.** `UPDATE t SET price = price *
  $1` bound `0.9` stores fifteen significant digits of the product; `COALESCE(price * $1, 0)` returns — and an
  `UPDATE` stores — a whole number (`19.99 * 0.9` becomes `18`), as `greatest(price * $1, 0)` and `CASE WHEN …
  THEN price * $1 ELSE 0 END` return one, where PostgreSQL computes in `numeric`. Cast the parameter
  (`$1::numeric`).
- **`unnest(…)` in `FROM` cannot read a table listed before it.** `FROM t, unnest(t.tags) AS u` is refused
  (`42703`). Inside a subquery, `FROM t, unnest(tags) AS u` reads the outer query's `tags` if it has a column of
  that name — and `unnest(t.tags)` does too if the outer query also reads `t` under that name — where PostgreSQL
  reads the subquery's `t`.
- **An unaliased scalar subquery, parameter or `NOT EXISTS` can get another column name.** `SELECT (SELECT
  count(*) FROM t)` is named `?column?` (PostgreSQL names it after the subquery's own column, `count`), `SELECT $1`
  and `SELECT $1::int` are named `$1` (PostgreSQL: `?column?` and `int4`), and `SELECT NOT EXISTS (…)` is named
  `exists` (PostgreSQL: `?column?`), so the row object's key differs; give the column an alias (`AS n`).

**Refused where PostgreSQL accepts:**

- **A byte view bound to a scalar parameter that is not `bytea` or text** is refused (`22023`): `pg` sends a
  view's bytes in the binary format, which PostgreSQL reads as that type's own binary layout (four bytes are an
  `int4`) or refuses (a `jsonb`). Inside a JSON value a view is still serialized with it
  (`{a: new Uint8Array([1])}` is `{"a": {"0": 1}}`, on both).
- **An object bound to a `bytea` parameter that is not a byte view or an array** — an `ArrayBuffer`, a plain
  object, a `Map`, an object with a `toPostgres` or `toJSON` method — is refused (`22023`), as a `bytea[]` element
  too. `pg` sends such a value's JSON (what the method returns, for a `toPostgres` object), and PostgreSQL stores
  that text's bytes: `{}` for an `ArrayBuffer`. Wrap a
  buffer in a view (`new Uint8Array(buffer)`).
- **One statement per `query()` call.** `db.query('SELECT 1; SELECT 2')` is refused (`42601`) where `pg` runs both;
  split a migration or seed file into its statements.
- **Arrays have one dimension.** A nested JS array bound to an array parameter is refused (`0A000`) whatever the
  element type, where `pg` sends a two-dimensional array — a `jsonb[]` parameter included: pass each JSON array as
  its JSON text (`[JSON.stringify([1, 2])]`), which both read as one element. A two-dimensional literal
  (`'{{a,b}}'::text[]`, or that string bound to an array parameter) is refused the same way, and so is one with
  explicit bounds (`'[1:2]={a,b}'`, `22P02`).
- **Only the `public` schema.** `CREATE SCHEMA` of any other name is refused (`0A000`) — for drizzle's migrator, see
  *ORMs* above.
- **A plain index on one `jsonb` or array column.** `CREATE INDEX ON t (tags)` over a `text[]` or `jsonb` column is
  refused (`0A000`), where PostgreSQL builds a B-tree. `USING gin` works, as do a `UNIQUE` or `PRIMARY KEY` on the
  column and an index that names a second column with it.
- **A string literal beside a computed value needs a cast.** A quoted literal compared with a subquery, function or
  arithmetic expression whose result is not text — `WHERE (SELECT max(created_at) FROM t) > '2024-01-01'`,
  `WHERE now() > '2024-01-01'`, `abs(n) = '1'` — or used in arithmetic (`n + '1'`) is refused (`42883`, or `42804` in
  arithmetic), where PostgreSQL reads the literal as the other side's type. Cast the literal
  (`'2024-01-01'::timestamptz`, `'1'::int`); comparing a literal with a plain column in a query (`WHERE n = '1'`,
  `n IN ('1', '2')`) or with an aggregate of one (`HAVING max(n) = '1'`) works, but not inside a `CHECK` constraint or a partial index's `WHERE` (`42883`), nor as an
  `= ANY('{1,2}')` array (`0A000`) — cast the literal there too.
- **A parameter on the left of `IN` needs its real type.** `$1 IN (1, 2)` with `$1` passed as the string `'1'` is
  refused (`42883`), where PostgreSQL reads the parameter as an `integer` from the list; pass a JS `number` — a JS
  `bigint` is refused the same way, as is a string in `$1 IN (SELECT …)` or `$1 = ANY (ARRAY[…])`.
  (`col IN ($1, $2)` with strings works.)
- **A comparison with a subquery's `unnest(arr)` over the outer row's array needs a cast.** `(SELECT
  count(*) FROM unnest(arr) AS u WHERE u = 30)` is refused (`42883`), the column being reported as `text`; cast
  the array (`unnest(arr::int[])`). The outer table's own name works too (`unnest(outer_table.arr)`), an alias
  of it does not (`FROM outer_table o` with `unnest(o.arr)`).

- **`generate_series` in `FROM` answers its `integer` and `bigint` series, up to 1 000 000 rows.**
  `INSERT INTO t SELECT g, … FROM generate_series(1, $1) g` answers as PostgreSQL does, a bound parameter included; a
  series longer than 1 000 000 rows is refused (`54000`) — seed in batches. Its arguments are constants and
  parameters: an argument naming a column — of an earlier table in the same `FROM`, which PostgreSQL reads row by
  row, or of an outer query — or holding a subquery is refused (`0A000`), and so is a parameter under arithmetic
  (`$1 * 10`; write `$1::int * 10`). A zero step is refused (`22023`) even
  under `WHERE false` or `LIMIT 0`, where PostgreSQL does not run the series. A parameter beside the series' column
  takes no type from it: in `WHERE g > $1` a JS number is compared as itself (`3.5` included, which PostgreSQL
  refuses as an `integer`) and a string is refused (`42883`), and `g + $1` answers JS `bigint`s — or, bound `1.5`,
  `2.5, 3.5, …` — under a field declared `integer`; write `$1::int`.

## Not supported yet

PostgreSQL features pglet has not built, and refuses rather than approximates. These are the ones a test suite, a
migration or a seed file is likely to meet; the list is not complete, and the
[compatibility table](https://github.com/fuzdev/pglet/blob/main/docs/compatibility.md) and the
[ledger](https://github.com/fuzdev/pglet/blob/main/EDGES.md) carry the rest. `0A000` is *feature not supported*;
syntax pglet does not parse is a syntax error, `42601`. Each refusal below comes before any row is read, so a
statement over an empty table is refused too, except where an item says otherwise.

**Queries:**

- **`generate_series` beyond its integer form in `FROM`** — in the select list (`SELECT generate_series(1, 3)`),
  `0A000`; the `numeric` and timestamp series, `0A000`; `WITH ORDINALITY`, `LATERAL` and `ROWS FROM (…)`, `42601`.
  `FROM generate_series(1, 3) AS g` answers (*Known differences*, above), and a recursive `WITH` counts in other
  types: `WITH RECURSIVE d(t) AS (SELECT '2024-01-01'::timestamp UNION ALL SELECT t + interval '1 day' FROM d WHERE
  t < '2024-01-03') SELECT t FROM d`.
- **An aggregate's `FILTER (WHERE …)`** — `42601`. Move the condition into a `CASE`:
  `count(CASE WHEN n > 10 THEN 1 END)`.
- **`array_agg`, `string_agg`, `json_agg`, `jsonb_agg`, `jsonb_object_agg`** — `0A000`.
- **`bool_and`, `bool_or`, `every`** — `0A000`. `min(b::int) = 1` and `max(b::int) = 1` answer as `bool_and(b)` and
  `bool_or(b)` do, `NULL` over no rows included.
- **Window functions other than `row_number`, `rank` and `dense_rank`** — `lag`, `lead`, `first_value`, `ntile`
  and an aggregate with `OVER` (`sum(n) OVER (…)`, `count(*) OVER ()`) are `0A000`, a frame (`ROWS BETWEEN …`) is
  `42601`. A correlated subquery answers the running total: `(SELECT sum(n) FROM t t2 WHERE t2.id <= t.id)`.
- **Array subscripts and slices** (`a[1]`, `a[1:2]`) and `ARRAY(SELECT …)` — `42601`. The array functions
  (`array_length`, `cardinality`, `array_append`, `array_position`, `array_to_string`, `string_to_array`) and `||`
  on an array are `0A000` (`42883` beside an uncast literal such as `a || '{9}'`, unless the array is a
  `text[]`), raised only when a row is evaluated if one side is an `ARRAY[…]` constructor; written to an array
  column, such a constructor's `||` is `42804`, before any row.
  Membership tests work: `x = ANY (a)`, `a @> ARRAY[x]`.
- **Row values** — `(a, b) = (1, 2)` and `(a, b) IN ((1, 2))` are `42601`, `ROW(1, 2)` is `0A000`. Compare column
  by column.
- **`= ANY (SELECT …)` and `> ALL (SELECT …)`** — `42601`. `IN (SELECT …)` works.
- **`LATERAL`** — `42601`.
- **`DELETE … USING`** — `42601`. `DELETE … WHERE id IN (SELECT …)` works.
- **An alias on a `DELETE`'s table**, and `DELETE FROM ONLY t` — `DELETE FROM t AS x WHERE x.id = 1`, `DELETE
  FROM t x` — `42601`. Name the table itself (`DELETE FROM t WHERE t.id = 1`); `UPDATE t AS x SET …` works.
- **An aggregate in a `SELECT` with no `FROM`** — `SELECT count(*)`, `INSERT INTO t SELECT max(…)` — `0A000`,
  raised when it is evaluated. Add `FROM (SELECT 1) AS one`.
- **`SELECT … WHERE` with no `FROM`**, as in `INSERT INTO t SELECT … WHERE NOT EXISTS (…)` — `42601`. Add
  `FROM (SELECT 1) AS one`, or, where a unique key covers the condition, insert with `ON CONFLICT DO NOTHING`.
- **`VALUES (1), (2)` as a statement of its own** — `0A000`. `SELECT * FROM (VALUES (1), (2)) AS v(x)` works.
- **`FETCH FIRST n ROWS ONLY`** — `42601`. `LIMIT` works.
- **`COLLATE`** — `42601`. Text already compares byte by byte, as `COLLATE "C"` does (*Known differences*, above).
- **`FOR UPDATE SKIP LOCKED` and `FOR UPDATE NOWAIT`** — `0A000`. A plain `FOR UPDATE` is accepted.
- **Regular expressions** — `~` and `!~` take only a pattern of `^` followed by literal characters
  (`s ~ '^ab'`); any other pattern is `0A000`, raised when a row is compared. `~*` and `!~*` are `42883`,
  `SIMILAR TO` is `42601`, `regexp_replace` and `regexp_matches` are `0A000`. `LIKE` and `ILIKE` work.

**Functions** — each `0A000` unless noted:

- **Strings:** `format`, `encode`, `repeat`.
- **Numbers:** `div`, `setseed`; `random(min, max)` is `42883` (`random()` works); a `numeric` raised to a
  non-integer `numeric` power — `2.0 ^ 0.5`, `power(2.0, 0.5)` — is `0A000`, raised when it is evaluated.
- **Time:** `to_timestamp`, `age`; `AT TIME ZONE` with a zone other than `'UTC'`, raised when a row is evaluated.
- **JSON:** `jsonb_set`, `jsonb_array_elements`, `jsonb_typeof`, `jsonb_array_length`, `jsonb_build_array`,
  `to_jsonb`, `row_to_json`, `json_build_object`; `jsonb || jsonb`; `jsonb - 'key'` (`42804`, raised when a row is
  evaluated). `->`, `->>`, `#>`, `#>>`, `?`, `@>` and `jsonb_build_object` work. Beside an uncast string or a
  parameter, `jsonb ||` is not refused but joins as text (*Known differences*, above).
- **Server:** `pg_sleep`, `pg_notify`, `pg_advisory_lock` (`pg_advisory_xact_lock` works), `txid_current`.

**Schema:**

- **`CREATE TABLE … AS SELECT`** and **`SELECT … INTO`** — `42601` (`SELECT * INTO`'s `0A000`). Create the table, then
  `INSERT INTO … SELECT`.
- **Views, materialized views, domains, functions, procedures, triggers, roles, temporary and unlogged tables** —
  each `CREATE` is `0A000`.
- **`DO` blocks** run plain SQL statements only: PL/pgSQL's `IF` and `PERFORM` are `42601`, `DECLARE` is `0A000`.
- **Renames** — `ALTER TABLE … RENAME` of a table, a column or a constraint is `0A000`; `ALTER INDEX … RENAME` is
  `42601`.
- **`ALTER COLUMN … TYPE` and `ALTER COLUMN … DROP NOT NULL`** — `0A000`. `SET NOT NULL`, `SET DEFAULT`,
  `ADD COLUMN` and `DROP COLUMN` work.
- **Generated columns** (`GENERATED ALWAYS AS (…) STORED`) — `0A000`. Identity columns work.
- **`PARTITION BY`, `(LIKE other)`, `EXCLUDE`, a constraint's `DEFERRABLE` or `NOT VALID`, an index's
  `INCLUDE (…)`** — `42601`; an index `USING hash` is `0A000`.
- **Composite types** (`CREATE TYPE … AS (…)`) — `0A000`. Enum types work.
- **The `json`, `citext`, `inet`, `money`, `xml`, range (`int4range`) and geometric (`point`) types** — `42704`.
  `jsonb` works. Full-text search (`to_tsvector`) is `0A000`.

**Statements:**

- **`EXPLAIN`, `ANALYZE`, `VACUUM`, `LISTEN`, `NOTIFY`, `PREPARE`, `EXECUTE`, `DECLARE … CURSOR`, `LOCK TABLE`,
  `MERGE`** — `42601`. Bound `$1` parameters work, and so does `INSERT … ON CONFLICT`.
- **`COPY … FROM STDIN` and `COPY … TO STDOUT`** — `0A000`: the adapter has no copy stream.

## Caveats

- In-memory, single connection (one writer). `fork()` is in-memory only.
- `gen_random_uuid()` and `random()` draw their bits from `Math.random()`, which is not a cryptographic source:
  fine for test ids and seed data, not for secrets.
- A Postgres-*dialect* subset — see *Not supported yet*, above, and the
  [pglet compatibility table](https://github.com/fuzdev/pglet/blob/main/docs/compatibility.md).

## License

0BSD — the package's `LICENSE` file.
