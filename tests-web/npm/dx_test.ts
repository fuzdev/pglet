/**
 * Developer-experience contracts — the behaviors a consumer actually relies on, pinned so a
 * regression is caught: the `pg`-shaped result contract (`command`/`rowCount`/`oid`, measured
 * against node-postgres), param round-trips (incl. bigint/Date — the input side of the coercion
 * contract), error recovery, the transaction semantics (return value, rollback, savepoint
 * nesting under an open block, single-flight, and an outside `query()` queueing behind an open
 * transaction rather than joining it), close safety, and the headline **fork-per-test** pattern.
 */
import {assert, assertEquals, assertInstanceOf, assertRejects} from '@std/assert';
// deno-lint-ignore no-import-prefix
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';

Deno.test('params round-trip: bigint in AND out, Date → timestamp, arrays recurse', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (big BIGINT, ts TIMESTAMP, tags BIGINT[])');
	const big = 9007199254740993n; // > Number.MAX_SAFE_INTEGER
	const when = new Date('2024-06-15T12:30:45.123Z');
	await db.query('INSERT INTO t VALUES ($1, $2, $3)', [big, when, [1n, 2n]]);

	const {rows} = await db.query('SELECT big, ts, tags FROM t');
	assertEquals(rows[0].big, big); // bigint out
	assertInstanceOf(rows[0].ts, Date);
	assertEquals((rows[0].ts as Date).toISOString(), when.toISOString());
	assertEquals(rows[0].tags, [1n, 2n]);

	// the actual round-trip a consumer does: read a bigint/Date, pass it straight back as a param
	const back = await db.query('SELECT 1 FROM t WHERE big = $1 AND ts = $2', [rows[0].big, rows[0].ts]);
	assertEquals(back.rowCount, 1);
	await db.close();
});

// A `Date` in the engine's first century goes in with its four-digit year and comes back the
// same instant: the text is what the binder sends, the `Date`s read back are what the coercion
// answers (`Date.UTC` reads a year of 0 through 99 as 1900 through 1999).
Deno.test('params round-trip: a Date in years 0001–0099 reads back as the instant it was bound as', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE early (id INT, tsz TIMESTAMPTZ, ts TIMESTAMP, list TIMESTAMPTZ[])');
	const whens = ['0001-01-01T00:00:00.000Z', '0050-06-15T12:30:45.123Z', '0099-12-31T23:59:59.999Z', '0100-01-01T00:00:00.000Z'];
	for (const [id, iso] of whens.entries()) {
		const when = new Date(iso);
		await db.query('INSERT INTO early VALUES ($1, $2, $3, $4)', [id, when, when, [when, null]]);
		const {rows} = await db.query('SELECT tsz, ts, list, tsz::text AS text FROM early WHERE id = $1', [id]);
		assertEquals(rows[0].text, `${iso.replace('T', ' ').replace(/\.000Z$|Z$/, '')}+00`, iso);
		assertEquals((rows[0].tsz as Date).getTime(), when.getTime(), iso);
		assertEquals((rows[0].ts as Date).getTime(), when.getTime(), iso);
		assertEquals((rows[0].list as (Date | null)[]).map((d) => d && d.getTime()), [when.getTime(), null], iso);
		// read it, pass it straight back
		const back = await db.query('SELECT 1 FROM early WHERE tsz = $1 AND ts = $2 AND list = $3', [rows[0].tsz, rows[0].ts, rows[0].list]);
		assertEquals(back.rowCount, 1, iso);
	}
	await db.close();
});

Deno.test('params round-trip: a ±Infinity read from a date or timestamp binds back', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE ti (ts TIMESTAMPTZ, d DATE)');
	await db.query('INSERT INTO ti VALUES ($1, $2)', [Infinity, -Infinity]);
	const {rows} = await db.query('SELECT ts FROM ti');
	assertEquals(rows[0].ts, Infinity);
	const back = await db.query('SELECT 1 FROM ti WHERE ts = $1 AND d = $2', [rows[0].ts, -Infinity]);
	assertEquals(back.rowCount, 1);
	await db.close();
});

Deno.test('params: a non-finite number binds as each target type reads it', async () => {
	// A date or timestamp target reads `Infinity` as its infinity (the readers take the word
	// in any case); a float8 target keeps the number; a text target stores the spelling
	// node-postgres sends (`String(Infinity)`); and a jsonb target serializes the whole value
	// as `JSON.stringify` does, which writes a non-finite number as `null`.
	const db = create_pglet_db();
	await db.query('CREATE TABLE nf (d DATE, ds DATE[], f FLOAT8, t TEXT, j JSONB)');
	await db.query('INSERT INTO nf VALUES ($1, $2, $3, $4, $5)', [
		-Infinity,
		[Infinity, null],
		Infinity,
		Infinity,
		[1, Infinity, {a: -Infinity}],
	]);
	const {rows} = await db.query('SELECT d::text AS d, ds::text AS ds, f, t, j::text AS j FROM nf');
	assertEquals(rows[0], {d: '-infinity', ds: '{infinity,NULL}', f: Infinity, t: 'Infinity', j: '[1, null, {"a": null}]'});
	// A bare select-list parameter is `text`, as PostgreSQL resolves it, so it reads the
	// spelling too: `pg` against PostgreSQL returns the string `'-Infinity'` here.
	const bare = await db.query('SELECT $1 AS x', [-Infinity]);
	assertEquals(bare.rows[0].x, '-Infinity');
	// A parameter the statement types but this engine reports no type for is decoded by
	// its JS runtime type, so a non-finite number stays a float8 — the adapter does not
	// turn it into text first.
	const untyped = await db.query('SELECT $1 + 1.5::float8 AS x', [-Infinity]);
	assertEquals(untyped.rows[0].x, -Infinity);
	await db.close();
});

// The structured `ErrorResponse` fields under `pg`'s property names — `pg-protocol`
// maps `D→detail H→hint P→position s→schema t→table c→column n→constraint`, and pglite
// exposes the same shape, so an adapter written against either driver reads these off a
// pglet error unchanged. The wasm host and the native server derive them from **one**
// builder (`Error::wire_fields`), so this suite and the server's fixture harness are
// asserting the same derivation from two sides.
Deno.test('a database error exposes pg\'s structured field names', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t_err (id INT PRIMARY KEY, v TEXT)');
	await db.query("INSERT INTO t_err VALUES (1, 'x')");
	const err = await db.query("INSERT INTO t_err VALUES (1, 'y')").then(() => null, (e) => e);
	assert(err instanceof Error);
	const e = err as Error & Record<string, string | undefined>;
	assertEquals(e.code, '23505');
	assertEquals(e.severity, 'ERROR');
	assertEquals(e.message, 'duplicate key value violates unique constraint "t_err_pkey"');
	assertEquals(e.detail, 'Key (id)=(1) already exists.');
	assertEquals(e.schema, 'public');
	assertEquals(e.table, 't_err');
	assertEquals(e.constraint, 't_err_pkey');
	// A field this error does not carry is absent, not empty — omission is the
	// contract for anything pglet cannot fill truthfully.
	assertEquals(e.column, undefined);
	assertEquals(e.hint, undefined);
	assertEquals(e.position, undefined);
	await db.close();
});

Deno.test('a not-null violation exposes its column and failing row', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t_nn (id INT NOT NULL, name TEXT)');
	const err = await db.query("INSERT INTO t_nn (name) VALUES ('a')").then(() => null, (e) => e);
	assert(err instanceof Error);
	const e = err as Error & Record<string, string | undefined>;
	assertEquals(e.code, '23502');
	assertEquals(e.detail, 'Failing row contains (null, a).');
	assertEquals(e.table, 't_nn');
	assertEquals(e.column, 'id');
	assertEquals(e.constraint, undefined);
	await db.close();
});

Deno.test('a syntax error exposes position as a 1-based codepoint index, as a string', async () => {
	const db = create_pglet_db();
	const err = await db.query("SELECT 'ααααα' AS a, FROM t").then(() => null, (e) => e);
	assert(err instanceof Error);
	const e = err as Error & Record<string, string | undefined>;
	assertEquals(e.code, '42601');
	// Five two-byte characters advance the position by five, not ten; `pg` leaves the
	// value a string, so this one does too.
	assertEquals(e.position, '22');
	await db.close();
});

Deno.test('an upsert that meets its own row twice is 21000 with 18.6\'s hint, and writes nothing', async () => {
	// One `ON CONFLICT DO UPDATE` statement whose rows propose one key twice: the second
	// proposal meets the row the first wrote, which 18.6 refuses and pglet used to update.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t_up (k INT PRIMARY KEY, v TEXT)');
	await db.query("INSERT INTO t_up VALUES (1, 'seed')");
	const sql = 'INSERT INTO t_up VALUES ($1, $2), ($3, $4) ON CONFLICT (k) DO UPDATE SET v = excluded.v';
	const err = await db.query(sql, [1, 'a', 1, 'b']).then(() => null, (e) => e);
	assert(err instanceof Error);
	const e = err as Error & Record<string, string | undefined>;
	assertEquals(e.code, '21000');
	assertEquals(e.message, 'ON CONFLICT DO UPDATE command cannot affect row a second time');
	assertEquals(
		e.hint,
		'Ensure that no rows proposed for insertion within the same command have duplicate constrained values.',
	);
	assertEquals(e.table, undefined);
	assertEquals((await db.query('SELECT k, v FROM t_up')).rows, [{k: 1, v: 'seed'}]);
	// the same statement over two keys upserts
	await db.query(sql, [1, 'a', 2, 'b']);
	assertEquals((await db.query('SELECT k, v FROM t_up ORDER BY k')).rows, [{k: 1, v: 'a'}, {k: 2, v: 'b'}]);
	await db.close();
});

Deno.test('a failed query does not wedge the db; error carries .code', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT PRIMARY KEY)');
	await db.query('INSERT INTO t VALUES (1)');
	const err = await db.query('INSERT INTO t VALUES (1)').then(() => null, (e) => e);
	assert(err instanceof Error);
	assertEquals((err as {code?: string}).code, '23505');
	// still fully usable after the error
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1);
	await db.close();
});

// pglet has one session zone (UTC), one date style, one interval style and one reading of a
// string literal. PostgreSQL acts on a `SET` that moves one; pglet cannot, and answering `SET`
// would leave it rendering as before. A value it cannot honor is refused, through `set_config` too, and a
// spelling of the value it already has is accepted — the statement a harness sends to pin a
// session.
Deno.test('a SET pglet cannot honor is refused (0A000); the value it already has is accepted', async () => {
	const db = create_pglet_db();
	const code = (sql: string) => db.query(sql).then(() => null, (e) => (e as {code?: string}).code);
	for (
		const sql of [
			`SET TIME ZONE 'America/New_York'`,
			`SET LOCAL TIME ZONE 'America/New_York'`,
			`SET timezone = 'America/New_York'`,
			`SELECT set_config('TimeZone', 'America/New_York', false)`,
			`SET DateStyle = 'German'`,
			`SET DateStyle = ISO, DMY`,
			`SET IntervalStyle = 'iso_8601'`,
			`SET standard_conforming_strings = off`,
		]
	) {
		assertEquals(await code(sql), '0A000', sql);
	}
	for (
		const sql of [
			`SET TIME ZONE 'UTC'`,
			`SET timezone TO utc`,
			`SET TIME ZONE DEFAULT`,
			`RESET TimeZone`,
			`SET DateStyle = 'ISO, MDY'`,
			`SET DateStyle = ISO, MDY`,
			`SET IntervalStyle = postgres`,
			`SET standard_conforming_strings = on`,
			`RESET ALL`,
		]
	) {
		assertEquals(await code(sql), null, sql);
	}
	assertEquals((await db.query(`SELECT set_config('timezone', 'utc', false) AS v`)).rows, [{v: 'UTC'}]);
	assertEquals((await db.query('SHOW TimeZone')).rows, [{TimeZone: 'UTC'}]);
	assertEquals((await db.query('SHOW DateStyle')).rows, [{DateStyle: 'ISO, MDY'}]);
	await db.close();
});

// Before the guard, a non-string reached wasm-bindgen's string copy and trapped inside the
// module's allocator (`RuntimeError: memory access out of bounds`, measured with a config
// object) — so the assertion that matters most is the error's CLASS, and after it the module
// answers on every instance. A config object whose `text` is not a string is one of them.
Deno.test('a non-string sql is a TypeError, and the module stays usable', async () => {
	const db = create_pglet_db();
	for (const [bad, why] of [
		[{sql: 'SELECT 1'}, 'takes the SQL text as a string'],
		[{text: 42}, 'takes the SQL text as a string'],
		[42, 'takes the SQL text as a string'],
		[null, 'null or undefined query'],
	] as const) {
		const err = await db.query(bad as unknown as string).then(() => null, (e) => e);
		assertInstanceOf(err, TypeError);
		assert((err as Error).message.includes(why), (err as Error).message);
	}
	assertEquals((await db.query('SELECT 1 AS x')).rows, [{x: 1}]);
	const fresh = create_pglet_db();
	assertEquals((await fresh.query('SELECT 2 AS x')).rows, [{x: 2}]);
	await fresh.close();
	await db.close();
});

// `pg`'s other call shape, `query(config, values)` — what drizzle's `node-postgres` driver
// sends. The second argument replaces `config.values` only when it is truthy (`pg` 8.16.3's
// `normalizeQueryConfig`), `rowMode` is honored, `name` is ignored, and `types` is honored over
// each cell's text form (the contract itself is `types_test.ts`); without `types` the values take
// the coercion contract (an int8 is a bigint, a timestamp a Date).
Deno.test('query takes a pg QueryConfig: text, values, rowMode, types; name ignored', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id BIGINT, ts TIMESTAMP, name TEXT)');
	await db.query({text: 'INSERT INTO t VALUES ($1, $2, $3)', values: [1n, new Date('2024-06-15T12:00:00Z'), 'a']});
	await db.query({text: 'INSERT INTO t VALUES ($1, $2, $3)', values: [9n, null, 'ignored']}, [2n, null, 'b']);
	const raw = {getTypeParser: () => (v: string) => `RAW(${v})`};
	const obj = await db.query({text: 'SELECT id, ts, name FROM t WHERE id = $1', name: 'q1', types: raw}, [1n]);
	assertEquals(obj.rows, [{id: 'RAW(1)', ts: 'RAW(2024-06-15 12:00:00)', name: 'RAW(a)'}]);
	const plain = await db.query({text: 'SELECT id, ts FROM t WHERE id = $1', name: 'q2'}, [1n]);
	assertEquals(plain.rows[0].id, 1n);
	assertInstanceOf(plain.rows[0].ts, Date);
	const arr = await db.query<unknown[]>({text: 'SELECT id, name FROM t ORDER BY id', rowMode: 'array'});
	assertEquals(arr.rows, [[1n, 'a'], [2n, 'b']]);
	// an empty array is truthy, so it replaces the config's values, as in `pg`
	const empty = await db.query({text: 'SELECT 1 AS x', values: ['unused']}, []);
	assertEquals(empty.rows, [{x: 1}]);
	// the transaction handle takes the same shape
	const inside = await db.transaction((tx) => tx.query({text: 'SELECT $1::int AS x'}, [7]));
	assertEquals(inside.rows, [{x: 7}]);
	await db.close();
});

// Every other shape `pg` 8.16.3's `Client.query` accepts is refused by name rather than read
// wrong: a callback anywhere (pg calls it; read as the parameter list, a 2-ary one bound two
// `undefined`s and the promise never called it back — measured before this), a parameter list
// that is not an array from either place (a string's characters bound one by one, measured),
// and a `Submittable` (pg runs it itself and answers through events; resolving it here left a
// cursor consumer waiting on events that never came). A refusal leaves the db usable.
Deno.test('query refuses a callback, a non-array values and a Submittable, as TypeErrors that say why', async () => {
	const db = create_pglet_db();
	const cb = (_err: unknown, _res: unknown) => {};
	const shapes: [string, () => Promise<unknown>, string][] = [
		['(config, callback)', () => (db.query as any)({text: 'SELECT $1::int AS a', values: [1]}, cb), 'callback'],
		['(config, values, callback)', () => (db.query as any)({text: 'SELECT $1::int AS a'}, [1], cb), 'callback'],
		['(sql, callback)', () => (db.query as any)('SELECT 1 AS a', cb), 'callback'],
		['config.callback', () => (db.query as any)({text: 'SELECT 1 AS a', callback: cb}), 'callback'],
		['(config, string)', () => (db.query as any)({text: 'SELECT $1::text AS a'}, 'xy'), 'values must be an array'],
		['(sql, string)', () => (db.query as any)('SELECT $1::text AS a', 'xy'), 'values must be an array'],
		['(sql, object)', () => (db.query as any)('SELECT $1::text AS a', {0: 'x'}), 'values must be an array'],
		['config.values string', () => (db.query as any)({text: 'SELECT $1::text AS a', values: 'xy'}), 'values must be an array'],
		['Submittable', () => (db.query as any)({text: 'SELECT 1 AS a', submit() {}}), 'Submittable'],
	];
	for (const [label, call, why] of shapes) {
		const err = await call().then(() => null, (e) => e);
		assertInstanceOf(err, TypeError, label);
		assert((err as Error).message.includes(why), `${label}: ${(err as Error).message}`);
	}
	// a null values is no values, as in pg
	assertEquals((await db.query({text: 'SELECT 1 AS a', values: null as unknown as unknown[]})).rows, [{a: 1}]);
	assertEquals((await db.query('SELECT 2 AS a')).rows, [{a: 2}]);
	await db.close();
});

// The `pg` result contract, field for field. Every expectation below was MEASURED on
// node-postgres 8.16.3 — the version `deno.json` pins; 8.20.0's `lib/result.js` is
// byte-identical, diffed — against PostgreSQL 18.6 (a scratch database, same statement text),
// not derived from `pg`'s regex. The regex is in `db.js`; this is the oracle it must reproduce.
//
// All FOUR tag-derived answers are compared per row, not a chosen one, because each fails
// differently and cherry-picking hides three of them: the engine's raw tag is `INSERT 0 2` /
// `CREATE TABLE`, and reporting it verbatim with `rowCount = rows.length` got `command` wrong
// on EVERY statement (`SELECT` included), `rowCount` wrong on every write without `RETURNING`,
// `rowCount` `0` where `pg` says `null`, and omitted `oid` entirely.
//
// It is NOT the whole result object: the comparison is a hand-built projection, so `fields` and
// the row *values* sit outside it by construction. `fields` has its own test below; the row
// values are the coercion suite's subject.
interface PgShape {
	command: string | null;
	rowCount: number | null;
	oid: number | null;
	rows: number;
}

const SHAPES: Array<[label: string, sql: string, expected: PgShape]> = [
	// DDL — the case a guard written only for `INSERT` gets wrong and stays green on: the tag
	// is two words and carries no count, so `command` is the FIRST WORD and `rowCount` is
	// **null**, which `0` is not a substitute for (an `UPDATE` matching nothing is a real `0`).
	['ddl create', `CREATE TABLE t (id SERIAL PRIMARY KEY, v TEXT)`, {command: 'CREATE', rowCount: null, oid: null, rows: 0}],
	['ddl create 2nd', `CREATE TABLE t2 (a INT)`, {command: 'CREATE', rowCount: null, oid: null, rows: 0}],
	['ddl index', `CREATE INDEX t2_a_ix ON t2 (a)`, {command: 'CREATE', rowCount: null, oid: null, rows: 0}],
	['ddl alter', `ALTER TABLE t2 ADD COLUMN b INT`, {command: 'ALTER', rowCount: null, oid: null, rows: 0}],
	['ddl truncate', `TRUNCATE TABLE t2`, {command: 'TRUNCATE', rowCount: null, oid: null, rows: 0}],
	['ddl drop', `DROP TABLE t2`, {command: 'DROP', rowCount: null, oid: null, rows: 0}],
	['do block', `DO $$ BEGIN END $$`, {command: 'DO', rowCount: null, oid: null, rows: 0}],
	// INSERT — the `oid` field lives here and only here (`INSERT <oid> <rows>`), and the
	// no-RETURNING row is the headline: 2 affected, 0 returned.
	['insert 1', `INSERT INTO t (v) VALUES ('a')`, {command: 'INSERT', rowCount: 1, oid: 0, rows: 0}],
	['insert 2', `INSERT INTO t (v) VALUES ('c'),('d')`, {command: 'INSERT', rowCount: 2, oid: 0, rows: 0}],
	['insert returning', `INSERT INTO t (v) VALUES ('e'),('f') RETURNING id`, {command: 'INSERT', rowCount: 2, oid: 0, rows: 2}],
	// `INSERT 0 0`: both numeric parts present and the second one zero. A parse that tests the
	// captured group for truthiness after coercing it to a number (`if (+m[3])`) reads this as
	// `INSERT` with no count and answers `oid: null, rowCount: 0` — `pg` tests the raw capture,
	// a non-empty digit string, so `'0'` participates.
	['insert 0 rows', `INSERT INTO t (v) SELECT v FROM t WHERE false`, {command: 'INSERT', rowCount: 0, oid: 0, rows: 0}],
	// UPDATE / DELETE — `oid` must be null here, so a swap of the two captured numbers on the
	// INSERT rows above cannot be mistaken for a passing shape.
	['update 2', `UPDATE t SET v = 'z' WHERE v IN ('c','d')`, {command: 'UPDATE', rowCount: 2, oid: null, rows: 0}],
	['update returning', `UPDATE t SET v = 'y' WHERE v = 'z' RETURNING id`, {command: 'UPDATE', rowCount: 2, oid: null, rows: 2}],
	// A tag WITH a zero count: `rowCount` is `0`, not `null` — the other polarity of the DDL rows.
	['update none', `UPDATE t SET v = 'q' WHERE v = 'nope'`, {command: 'UPDATE', rowCount: 0, oid: null, rows: 0}],
	['delete 2', `DELETE FROM t WHERE v = 'y'`, {command: 'DELETE', rowCount: 2, oid: null, rows: 0}],
	['delete returning', `DELETE FROM t WHERE v = 'a' RETURNING id`, {command: 'DELETE', rowCount: 1, oid: null, rows: 1}],
	['delete none', `DELETE FROM t WHERE v = 'nope'`, {command: 'DELETE', rowCount: 0, oid: null, rows: 0}],
	// SELECT — the over-correction polarity. `rowCount` now comes from the tag for everything,
	// so a fix that reached the tag but mis-read the count reddens here rather than passing on
	// the writes alone.
	['select 2 rows', `SELECT * FROM t`, {command: 'SELECT', rowCount: 2, oid: null, rows: 2}],
	['select 3 rows', `SELECT * FROM (VALUES (1),(2),(3)) AS v(x)`, {command: 'SELECT', rowCount: 3, oid: null, rows: 3}],
	['select none', `SELECT * FROM t WHERE v = 'nope'`, {command: 'SELECT', rowCount: 0, oid: null, rows: 0}],
	// Transaction control — no count, so `null` again; and the tags the adapter's own
	// transaction state keys on, so a fix that rewrote the tag in place would show up here.
	['begin', `BEGIN`, {command: 'BEGIN', rowCount: null, oid: null, rows: 0}],
	['commit', `COMMIT`, {command: 'COMMIT', rowCount: null, oid: null, rows: 0}],
	['begin again', `BEGIN`, {command: 'BEGIN', rowCount: null, oid: null, rows: 0}],
	['rollback', `ROLLBACK`, {command: 'ROLLBACK', rowCount: null, oid: null, rows: 0}],
];

Deno.test('the result is pg-shaped field for field: command, rowCount, oid — measured vs pg 8.16.3 / PG 18.6', async () => {
	const db = create_pglet_db();
	for (const [label, sql, expected] of SHAPES) {
		const r = await db.query(sql);
		assertEquals(
			{command: r.command, rowCount: r.rowCount, oid: r.oid, rows: r.rows.length},
			expected,
			`${label}: ${sql}`,
		);
	}
	await db.close();
});

Deno.test('null and 0 are distinguishable, and `command` never carries the tag\'s tail', async () => {
	// The two discriminations the table above would still pass if `assertEquals` were loosened,
	// stated on their own so a failure names the rule rather than a row.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	const ddl = await db.query('CREATE TABLE t3 (id INT)');
	const zero = await db.query(`UPDATE t SET id = 1 WHERE id = 999`);
	assert(ddl.rowCount === null, `DDL rowCount must be null, got ${JSON.stringify(ddl.rowCount)}`);
	assert(zero.rowCount === 0, `a matched-nothing UPDATE must be 0, got ${JSON.stringify(zero.rowCount)}`);
	assert(ddl.rowCount !== zero.rowCount, 'null and 0 must not collapse into one value');
	// `command` is a single word for every tag pglet emits, including the multi-word ones.
	for (const [sql, word] of [
		['CREATE TABLE t4 (id INT)', 'CREATE'],
		['CREATE SEQUENCE s4', 'CREATE'],
		['ALTER SEQUENCE s4 RESTART WITH 5', 'ALTER'],
		['DROP SEQUENCE s4', 'DROP'],
		['TRUNCATE TABLE t4', 'TRUNCATE'],
		['DROP TABLE t4', 'DROP'],
	] as const) {
		const {command} = await db.query(sql);
		assertEquals(command, word, sql);
		assert(!command!.includes(' '), `command must be one word: ${JSON.stringify(command)}`);
	}
	await db.close();
});

Deno.test('the same shape on the two alternate call paths: bound $N params, and rowMode array', async () => {
	// `run_query` picks `queryParams` over `query` when params are bound, and pivots `rows` for
	// `rowMode: 'array'` — two branches the table above never takes, and a shape assembled per
	// branch would diverge on exactly the fields that branch does not touch. Measured on
	// node-postgres 8.16.3 / PG 18.6: binding and row mode change neither `command` nor `rowCount`.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT, v TEXT)');
	const ins = await db.query('INSERT INTO t VALUES ($1, $2), ($3, $4)', [1, 'a', 2, 'b']);
	assertEquals({command: ins.command, rowCount: ins.rowCount, oid: ins.oid, rows: ins.rows.length}, {
		command: 'INSERT',
		rowCount: 2,
		oid: 0,
		rows: 0,
	});
	const upd = await db.query('UPDATE t SET v = $1 WHERE id = $2', ['z', 1]);
	assertEquals({command: upd.command, rowCount: upd.rowCount, oid: upd.oid}, {command: 'UPDATE', rowCount: 1, oid: null});
	const obj = await db.query('SELECT id, v FROM t ORDER BY id');
	const arr = await db.query<unknown[]>('SELECT id, v FROM t ORDER BY id', [], {rowMode: 'array'});
	assertEquals(
		{command: arr.command, rowCount: arr.rowCount, oid: arr.oid},
		{command: obj.command, rowCount: obj.rowCount, oid: obj.oid},
	);
	assertEquals(arr.rowCount, 2); // and it is the right number, not merely the same one
	await db.close();
});

Deno.test('a repeated column name: array rows are positional, object rows keep the last value, as pg', async () => {
	// each expected row is what `pg` 8.16.3 answers over PostgreSQL 18.6 for the same query, but
	// for an int8 cell, which this package hands back as a `bigint` where `pg` hands a string
	const db = create_pglet_db();
	assertEquals((await db.query<unknown[]>('SELECT 1 AS x, 2 AS x, 3 AS y', [], {rowMode: 'array'})).rows, [[1, 2, 3]]);
	assertEquals((await db.query<unknown[]>({text: 'SELECT 1 AS x, 2 AS x, 3 AS y', rowMode: 'array'})).rows, [[1, 2, 3]]);
	assertEquals((await db.query('SELECT 1 AS x, NULL::int AS x, 2 AS y, 3 AS y')).rows, [{x: null, y: 3}]);
	assertEquals((await db.query<unknown[]>('SELECT 1 AS x, NULL::int AS x, 2 AS y, 3 AS y', [], {rowMode: 'array'})).rows, [
		[1, null, 2, 3],
	]);
	// each cell is coerced by its own column's type, not by another column of the same name
	const ts = "'2024-06-15 12:30:45+00'::timestamptz";
	assertEquals((await db.query(`SELECT ${ts} AS x, 'ab' AS x`)).rows, [{x: 'ab'}]);
	const [[first, second]] = (await db.query<unknown[]>(`SELECT 'ab' AS x, ${ts} AS x`, [], {rowMode: 'array'})).rows;
	assertEquals(first, 'ab');
	assertEquals(second, (await db.query(`SELECT ${ts} AS x`)).rows[0].x);
	await db.close();
});

Deno.test('fields: one descriptor per output column, [] when there are none, three keys and no more', async () => {
	// `fields` had no assertion anywhere in the npm suites before this test — not in the pg-shape
	// table above (a hand-built projection of the four tag-derived answers), not in the coercion
	// suite (which reads values), not in the Node smoke run. It is load-bearing twice over:
	// `coerce_rows` keys the value policy off `dataTypeID`, and `rowMode: 'array'` pivots each row
	// by `fields` order, so a wrong descriptor is a wrong VALUE at the consumer with no error.
	//
	// Measured on node-postgres 8.16.3 / PostgreSQL 18.6: `name` and `dataTypeID` agree exactly,
	// and a statement returning no columns reports `[]` on both — not `undefined`, not a
	// placeholder. What `pg` carries and this does not (`tableID`, `columnID`, `dataTypeSize`,
	// `dataTypeModifier`, `format`) is asserted ABSENT, so `index.d.ts`'s "pg.FieldDef-shaped"
	// stays a claim with a list attached rather than an impression.
	const db = create_pglet_db();
	const ddl = await db.query('CREATE TABLE t (id INT, name TEXT, big BIGINT, ts TIMESTAMPTZ, tags TEXT[])');
	assertEquals(ddl.fields, []);
	assertEquals((await db.query(`INSERT INTO t VALUES (1,'a',2,'2024-01-01T00:00:00Z',ARRAY['x'])`)).fields, []);
	const sel = await db.query('SELECT id, name, big, ts, tags FROM t');
	assertEquals(sel.fields.map((f) => [f.name, f.dataTypeID]), [
		['id', 23],
		['name', 25],
		['big', 20],
		['ts', 1184],
		['tags', 1009],
	]);
	// `pg`'s `FieldDef` keys, in its order; the table OID is pglet's own
	const table_id = sel.fields[0].tableID;
	assert(table_id > 0);
	assertEquals(Object.keys(sel.fields[0]), [
		'name', 'tableID', 'columnID', 'dataTypeID', 'dataTypeSize', 'dataTypeModifier', 'format',
	]);
	assertEquals(sel.fields[0], {
		name: 'id', tableID: table_id, columnID: 1, dataTypeID: 23, dataTypeSize: 4, dataTypeModifier: -1,
		format: 'text',
	});
	// every value below is what `pg` reads off PostgreSQL 18.6 for the same table and query
	// (its `tableID` aside): a typmod'd column's modifier, a computed column's `0` / `0`
	await db.query('CREATE TABLE fd (a int4, b varchar(7), c numeric(6,2))');
	const fd = await db.query('SELECT a, b, c, a + 1 AS e, now() AS h FROM fd');
	const fd_id = fd.fields[0].tableID;
	assertEquals(
		fd.fields.map((f) => [f.name, f.tableID, f.columnID, f.dataTypeID, f.dataTypeSize, f.dataTypeModifier, f.format]),
		[
			['a', fd_id, 1, 23, 4, -1, 'text'],
			['b', fd_id, 2, 1043, -1, 11, 'text'],
			['c', fd_id, 3, 1700, -1, 393222, 'text'],
			['e', 0, 0, 23, 4, -1, 'text'],
			['h', 0, 0, 1184, 8, -1, 'text'],
		],
	);
	// with `types` the descriptors are the same objects' shape
	assertEquals((await db.query({text: 'SELECT a FROM fd', types: {getTypeParser: () => String}})).fields, [fd.fields[0]]);
	// an alias renames the descriptor — which is what an ORM keys its row objects by
	assertEquals((await db.query('SELECT id AS the_id FROM t')).fields.map((f) => f.name), ['the_id']);
	// `rowMode: 'array'` pivots the ROWS and leaves `fields` object-shaped: it is the pivot's key,
	// so a descriptor list that changed shape here would silently reorder or drop row elements
	const arr = await db.query<unknown[]>('SELECT id, name FROM t', [], {rowMode: 'array'});
	assertEquals(arr.fields.map((f) => f.name), ['id', 'name']);
	assertEquals(arr.rows[0], [1, 'a']);
	await db.close();
});

Deno.test('transaction: returns the callback value; rolls back on throw + stays usable', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	const got = await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		return 'done';
	});
	assertEquals(got, 'done');
	await assertRejects(() =>
		db.transaction(async (tx) => {
			await tx.query('INSERT INTO t VALUES (2)');
			throw new Error('boom');
		}));
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1); // 2 rolled back
	await db.close();
});

Deno.test('a transaction() a throw rolls back leaves each later statement answering the catalog it restored', async () => {
	// A rollback that changed no catalog keeps the statements the database holds parsed, and one
	// that took a catalog change back holds none parsed against it — here an index swap, which a
	// held plan would probe after the rollback dropped it. The hit itself is seen by the native
	// suite (`plan_cache.rs`); here, the answers on both sides of each rollback.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	await db.query('CREATE INDEX t_id ON t (id)');
	await db.query('INSERT INTO t VALUES (1)');
	const bound = 'SELECT id FROM t WHERE id = $1';
	const probe = 'SELECT id FROM t WHERE id = 1';
	const inside: unknown[] = [];
	for (const id of [2, 3]) {
		await assertRejects(() =>
			db.transaction(async (tx) => {
				await tx.query('INSERT INTO t VALUES ($1)', [id]);
				inside.push((await tx.query(bound, [id])).rows);
				throw new Error('boom');
			}));
		assertEquals((await db.query(bound, [id])).rows, []);
	}
	await assertRejects(() =>
		db.transaction(async (tx) => {
			await tx.query('DROP INDEX t_id');
			await tx.query('CREATE INDEX t_id2 ON t (id)');
			inside.push((await tx.query(probe)).rows);
			throw new Error('boom');
		}));
	assertEquals(inside, [[{id: 2}], [{id: 3}], [{id: 1}]]);
	assertEquals((await db.query(probe)).rows, [{id: 1}]);
	await db.close();
});

Deno.test('a failed statement aborts the block: the rest is 25P02 and the COMMIT keeps nothing', async () => {
	// The headline transaction-abort divergence, on the leg that had it. This package
	// used to run every statement after the failure normally and answer the `COMMIT` with tag
	// `COMMIT`, committing `1, 2, 3`; the native server and Postgres 18.6 answer `25P02` and
	// convert the `COMMIT`, leaving no rows. So the same npm package committed different data
	// depending on which leg you drove.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT PRIMARY KEY)');
	await db.query('BEGIN');
	await db.query('INSERT INTO t VALUES (1)');
	await db.query('INSERT INTO t VALUES (2)');

	const dup = await db.query('INSERT INTO t VALUES (1)').then(() => null, (e) => e);
	assertInstanceOf(dup, Error);
	assertEquals((dup as Error & {code?: string}).code, '23505');

	// Everything after it — a read and a write alike — is `25P02` until the block ends.
	for (const sql of ['SELECT 1', 'INSERT INTO t VALUES (3)']) {
		const e = await db.query(sql).then(() => null, (err) => err);
		assertInstanceOf(e, Error, sql);
		assertEquals((e as Error & {code?: string}).code, '25P02', sql);
	}

	// The COMMIT is Postgres's implicit rollback: no error, command tag `ROLLBACK`.
	assertEquals((await db.query('COMMIT')).command, 'ROLLBACK');
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0);
	// The adapter's own `in_tx` mirror followed the tag, so the next BEGIN is accepted rather
	// than a spurious 25001 — the over-correction polarity.
	assertEquals((await db.query('BEGIN')).command, 'BEGIN');
	await db.query('ROLLBACK');
	await db.close();
});

Deno.test('a transaction() callback that swallows its own error commits nothing', async () => {
	// The shape the README used to warn about the other way round. `transaction(fn)` rolls back
	// only when `fn` throws, so a callback that catches its own error and returns normally
	// reaches `run_query('COMMIT')` — which, inside an aborted block, is the implicit rollback.
	// Before the block aborted at all, that COMMIT published the surviving writes **and** the
	// partial effects of the failed statement.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT PRIMARY KEY)');
	await db.query('INSERT INTO t VALUES (1)');

	const got = await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (2)');
		try {
			await tx.query('INSERT INTO t VALUES (1)'); // 23505 — aborts the block
		} catch {
			// swallowed on purpose: the callback returns normally
		}
		return 'swallowed';
	});
	assertEquals(got, 'swallowed');
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1);
	await db.close();
});

Deno.test('a comment-prefixed exit ends an aborted block instead of wedging the connection', async () => {
	// The exit classifier reads the lexer, so the comment is stripped before anything
	// classifies. Under the `starts_with` text match each of these answered `25P02` **and so did
	// every statement after it, `BEGIN` included**, until a bare exit arrived — a wedged
	// connection reached from an ordinary query-tagging client.
	for (const exit of ['/* trace */ COMMIT', '-- trace\nCOMMIT', '/* trace */ ROLLBACK', '-- trace\nROLLBACK']) {
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		await db.query('BEGIN');
		await db.query('INSERT INTO t VALUES (1)');
		await db.query('SELECT * FROM no_such_table').then(() => null, () => null);

		assertEquals((await db.query(exit)).command, 'ROLLBACK', JSON.stringify(exit));
		// Usable, not wedged: an ordinary statement answers and a fresh block opens.
		assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0);
		assertEquals((await db.query('BEGIN')).command, 'BEGIN', JSON.stringify(exit));
		await db.query('ROLLBACK');
		await db.close();
	}
});

Deno.test('an outside query() during a transaction outlives its ROLLBACK (the audit-log shape)', async () => {
	// The lost-write half. `transaction()` holds the connection's one lane; a plain `db.query()`
	// issued from outside used to run IMMEDIATELY on that same connection, so it silently joined
	// the open transaction and was reverted with it. This is the `auth/audit_log_rollback` shape
	// a real app protects — the audit entry is written precisely because the business
	// transaction may fail, and it must survive that failure. PGlite keeps it (queued behind its
	// per-instance mutex — measured on 0.4.5 and 0.5.4) and so does a `pg` pool, on a second
	// connection.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	await db.query('CREATE TABLE audit (m TEXT)');
	const order: string[] = [];
	const err = await db.transaction(async (tx) => {
		order.push('tx:start');
		await tx.query('INSERT INTO t VALUES (1)');
		order.push('tx:wrote');
		// deliberately NOT awaited — awaiting it inside the callback deadlocks (documented), and
		// fire-and-forget is the shape an audit/telemetry write actually takes
		db.query(`INSERT INTO audit VALUES ('kept')`).then(
			() => order.push('outside:ok'),
			(e) => order.push(`outside:err:${(e as Error).message}`),
		);
		throw new Error('boom');
	}).then(() => null, (e) => e);
	assertInstanceOf(err, Error);
	assertEquals(err.message, 'boom');
	await new Promise((r) => setTimeout(r, 20)); // let the queued outside write drain

	assertEquals((await db.query('SELECT count(*)::int AS c FROM audit')).rows[0].c, 1); // survived
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0); // txn reverted
	// It ran AFTER the transaction ended, not inside it — the ordering is the mechanism, and a
	// fix that merely made the write survive (e.g. by suppressing the ROLLBACK) would fail here.
	assertEquals(order, ['tx:start', 'tx:wrote', 'outside:ok']);
	await db.close();
});

Deno.test('an outside query() during a COMMITTING transaction still lands after it, in call order', async () => {
	// The accept polarity of the guard above: the queue must not reorder or drop anything when
	// the transaction succeeds either, and two outside writes must keep their relative order.
	//
	// A committing transaction cannot tell joined from queued by its ROWS — both end with the
	// outside writes committed — so the discriminating assertion is made from INSIDE the
	// callback, where the two are opposite: queued, the transaction sees none of them.
	const db = create_pglet_db();
	await db.query('CREATE TABLE log (n INT)');
	const settled: Array<Promise<unknown>> = [];
	await db.transaction(async (tx) => {
		await tx.query('CREATE TABLE inner_t (n INT)');
		await tx.query('INSERT INTO inner_t VALUES (1)');
		settled.push(db.query('INSERT INTO log VALUES (1)'));
		settled.push(db.query('INSERT INTO log VALUES (2)'));
		// the `await` above and this one both yield the microtask queue, which is all an
		// unqueued `query()` needs to have already landed inside this transaction
		assertEquals((await tx.query('SELECT count(*)::int AS c FROM log')).rows[0].c, 0);
	});
	await Promise.all(settled);
	assertEquals((await db.query<{n: number}>('SELECT n FROM log')).rows.map((r) => r.n), [1, 2]);
	assertEquals((await db.query('SELECT count(*)::int AS c FROM inner_t')).rows[0].c, 1);
	await db.close();
});

Deno.test('the tx handle is NOT the queued query — its statements run inside the transaction', async () => {
	// The other side of the split: `tx.query` must bypass the queue the transaction is holding,
	// or the callback deadlocks against itself. Two awaited statements inside one callback prove
	// it, and the rollback proves they were really inside the transaction.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		await tx.query('INSERT INTO t VALUES (2)');
		assertEquals((await tx.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 2); // reads its own writes
	});
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 2);
	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (3)');
		throw new Error('boom');
	}).catch(() => undefined);
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 2); // 3 rolled back
	await db.close();
});

Deno.test('an ignored failing query() still reports its rejection — fire-and-forget is not silenced', async () => {
	// `enqueue` must attach a handler to every entry to keep the lane's tail settled, and that
	// attachment is what marks the CALLER's promise handled under the spec's
	// HostPromiseRejectionTracker. Left there, the ordinary failure of the very shape this adapter
	// recommends beside an open transaction — fire-and-forget, because awaiting deadlocks —
	// reports nothing anywhere: measured on the built package, an ignored `db.query()` against a
	// missing table printed nothing and Node exited **0**, where before the queue landed it
	// crashed the process with `code: '42P01'`. `derive_unhandled` hands the caller a fresh
	// promise so the host still sees it.
	//
	// Both polarities in one listener, filtered by the sentinel table NAME rather than counted:
	// `unhandledrejection` is global and asynchronous, so a bare count passes for the wrong reason
	// the moment any other rejection lands in the window — and a rejection this listener does not
	// claim is left to the runner, which is what keeps the test honest about its own scope.
	//
	// The near-miss it rejects: dropping the tail handler (`tx_chain = result; return result`)
	// rather than adding the fresh promise. That passes a single-statement reproduction and fails
	// here, because the NEXT enqueued entry chains onto the caller's own promise and launders it —
	// which is why the `SELECT 1` below is part of the reproduction and not decoration.
	//
	// What it cannot see: the browser's own `unhandledrejection`. Deno implements the same web
	// event and Node's `unhandledRejection` is the same host hook (`test_npm_node.mjs` pins Node's
	// consumer-visible form — a non-zero exit with the SQLSTATE on stderr); no browser runs here.
	const seen: string[] = [];
	const on_rejection = (e: Event) => {
		const {reason} = e as PromiseRejectionEvent;
		const message = (reason as Error | undefined)?.message ?? '';
		const sentinel = message.match(/unhandled_probe_\w+/)?.[0];
		if (!sentinel) return; // not ours — leave it to the test runner
		e.preventDefault();
		seen.push(`${(reason as {code?: string}).code}:${sentinel}`);
	};
	globalThis.addEventListener('unhandledrejection', on_rejection);
	try {
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		// HANDLED — must NOT report. The no-double-report polarity: the tail swallows and so never
		// rejects, leaving the returned promise the only one the host tracks.
		await db.query('INSERT INTO unhandled_probe_handled VALUES (1)').catch(() => undefined);
		await db.transaction(async (tx) => {
			await tx.query('INSERT INTO unhandled_probe_tx_handled VALUES (1)');
		}).catch(() => undefined);
		// IGNORED — must report. Both spellings go through the one `enqueue`, and `transaction()`
		// has worn this shape since before the queue existed, so the fix covers the older twin too.
		db.query('INSERT INTO unhandled_probe_ignored VALUES (1)');
		db.transaction(async (tx) => {
			await tx.query('INSERT INTO unhandled_probe_tx_ignored VALUES (1)');
		});
		// a following entry: the lane must still advance past two failures...
		assertEquals((await db.query('SELECT 1 AS x')).rows[0].x, 1);
		await new Promise((r) => setTimeout(r, 30));
		await db.close();
	} finally {
		globalThis.removeEventListener('unhandledrejection', on_rejection);
	}
	// ...and chaining it must not have laundered either ignored rejection
	assertEquals(seen.sort(), ['42P01:unhandled_probe_ignored', '42P01:unhandled_probe_tx_ignored']);
});

Deno.test('a raw BEGIN gets NO outside-query protection — the transaction() guarantee does not reach it', async () => {
	// The narrowed half of the audit-log class, pinned so the green `transaction()` guards above
	// are not read as the class being closed. kysely's shipped dialect and drizzle both drive
	// transactions with raw BEGIN/COMMIT/ROLLBACK through `query()`, and at that layer the
	// transaction's own statements and an outside one are the same call on the same lane: there is
	// no boundary to queue against, so the outside write joins the transaction and dies with its
	// ROLLBACK.
	//
	// The reference is a connection POOL, not "pg and PGlite": measured, PGlite loses this write on
	// the same raw-BEGIN spelling (0.4.5 and 0.5.4) and so does a single `pg.Client`, while a `pg`
	// Pool keeps it by running it on a second connection. PGlite's boundary is `transaction()`'s
	// too, which is the same boundary this adapter has.
	//
	// This asserts the LOSS, and the `transaction()` spelling beside it is the control that
	// identifies the boundary rather than the statements. Invert it if the kysely dialect is ever
	// reworked to hold the lane across begin/commit/rollback — that closes the class for kysely
	// only, needs no new `PgletDb` API (see `kysely.js`), and would leave this raw-`query()`
	// spelling exactly as it is, so read the inversion narrowly. A guard that merely vanished
	// would leave `EDGES.md` row 16's record with nothing pointing at it.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	await db.query('CREATE TABLE audit (m TEXT)');
	await db.query('BEGIN');
	await db.query('INSERT INTO t VALUES (1)');
	await db.query(`INSERT INTO audit VALUES ('outside')`); // indistinguishable from an inside one
	await db.query('ROLLBACK');
	assertEquals((await db.query('SELECT count(*)::int AS c FROM audit')).rows[0].c, 0); // LOST
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0);
	// the control — the same two writes through `transaction()`, where the boundary exists
	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		db.query(`INSERT INTO audit VALUES ('kept')`); // fire-and-forget; queues behind the txn
		throw new Error('boom');
	}).catch(() => undefined);
	await new Promise((r) => setTimeout(r, 20));
	assertEquals((await db.query('SELECT count(*)::int AS c FROM audit')).rows[0].c, 1);
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0);
	await db.close();
});

Deno.test('transaction: nested db.transaction throws, never deadlocks', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	await assertRejects(
		() => db.transaction(async () => db.transaction(async () => 1)),
		Error,
		'a transaction() callback is already running',
	);
	await db.close();
});

Deno.test('transaction: two calls made before either callback begins serialize (single-flight)', async () => {
	// The scope of this test is in its name and is narrower than "overlapping top-level calls
	// serialize", which is not the guarantee: BOTH calls happen in one synchronous turn here, so
	// `in_managed_tx` is still false when each is made and both are queued. The later-turn
	// spelling is the test below, and it is rejected rather than serialized.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	// a yields the event loop mid-transaction; single-flight must keep b out until a commits
	const a = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		await new Promise((r) => setTimeout(r, 5));
		await tx.query('INSERT INTO t VALUES (2)');
	});
	const b = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (3)');
	});
	await Promise.all([a, b]);
	const ids = (await db.query<{id: number}>('SELECT id FROM t ORDER BY id')).rows.map((r) => r.id);
	assertEquals(ids, [1, 2, 3]); // no interleave, no lost write
	await db.close();
});

Deno.test('a concurrent top-level transaction() is REJECTED as nested — the divergence, not a queue', async () => {
	// The other spelling of the test above, and the realistic one: the second call is made in a
	// later turn, after the first callback has begun. `in_managed_tx` is a plain flag read at call
	// time, so every `transaction()` from that moment on takes the nested rejection — a genuinely
	// top-level one included — and that caller's work does not run at all. A `pg` pool would give
	// it its own connection; PGlite serializes both on its one connection (measured: two
	// concurrent top-level `transaction()` calls both fulfil, on 0.4.5 and 0.5.4).
	//
	// The undecidability is inherent — separating a lexically nested call from a concurrent
	// top-level one needs an async-context store (`AsyncLocalStorage`), which browsers do not have
	// — but the resolution is a CHOICE, and PGlite is the proof: same single connection, same
	// missing store, and it serializes instead, hanging on a genuinely nested call rather than on
	// this one. pglet refuses loudly because that hang is largely silent (an ordinary `main()`
	// entry exits 0 with no output; see the deadlock note on `PgletDb.query`). Pinned as a
	// divergence — invert this if the rejection is ever narrowed to real nesting.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	let release = () => {};
	const first_callback_running = new Promise<void>((r) => {
		release = r;
	});
	const first = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		release();
		await new Promise((r) => setTimeout(r, 10)); // hold the lane across a macrotask
		await tx.query('INSERT INTO t VALUES (2)');
	});
	await first_callback_running; // a later turn — this call is top-level by every measure
	const second = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (3)');
	});
	const [a, b] = await Promise.allSettled([first, second]);
	assertEquals(a.status, 'fulfilled');
	assertEquals(b.status, 'rejected');
	const err = (b as PromiseRejectedResult).reason;
	assertInstanceOf(err, Error);
	assert(err.message.includes('a transaction() callback is already running'), err.message);
	// and the message must name the top-level reading too, or it sends this caller looking for a
	// callback that exists nowhere in its scope
	assert(err.message.includes('top-level'), err.message);
	// the second caller's write is simply gone — loud, but gone
	assertEquals((await db.query<{id: number}>('SELECT id FROM t ORDER BY id')).rows.map((r) => r.id), [1, 2]);
	await db.close();
});

Deno.test('raw BEGIN/COMMIT (the kysely/drizzle path) works sequentially', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	// kysely/drizzle drivers manage transactions with raw BEGIN/COMMIT via query(), not transaction()
	await db.query('BEGIN');
	await db.query('INSERT INTO t VALUES (1)');
	await db.query('COMMIT');
	await db.query('BEGIN');
	await db.query('INSERT INTO t VALUES (2)');
	await db.query('ROLLBACK');
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1); // commit kept, rollback dropped
	await db.close();
});

Deno.test('overlapping raw-BEGIN transactions fail loud (25001), never silently corrupt', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	// The exact corruption shape: two logically-concurrent transactions sharing ONE instance,
	// each managing its own txn with raw BEGIN/COMMIT (as kysely/drizzle do). On a single
	// connection this used to silently interleave and lose the "committed" row; it must now
	// reject the second BEGIN loudly instead.
	const txA = (async () => {
		await db.query('BEGIN');
		await db.query('INSERT INTO t VALUES (1)');
		await new Promise((r) => setTimeout(r, 5)); // yield mid-transaction
		await db.query('COMMIT');
	})();
	const txB = (async () => {
		await db.query('BEGIN'); // overlaps txA's open transaction → must fail loud
		await db.query('INSERT INTO t VALUES (2)');
		await db.query('ROLLBACK');
	})();
	const results = await Promise.allSettled([txA, txB]);
	const rejected = results.filter((r) => r.status === 'rejected');
	assertEquals(rejected.length, 1); // exactly one lane rejected — the overlapping BEGIN
	const err = (rejected[0] as PromiseRejectedResult).reason;
	assertInstanceOf(err, Error);
	assertEquals((err as {code?: string}).code, '25001');
	// the surviving transaction committed cleanly — no lost write, no phantom
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1);
	await db.close();
});

// The spellings the engine opens a transaction for — measured tags, all `BEGIN`, and each an
// ordinary thing for a query-tagging or raw-SQL consumer to send. Which of them a text pre-check
// saw depends on the matcher, so no split of this list is a property of the list: the deleted JS
// regex `/^\s*(?:begin|start\s+transaction)\b/i` missed the two comment-prefixed spellings and
// matched `START\nTRANSACTION`, since `\s+` spans a newline, while the Rust classifier it was
// modeled on tested `trim_start().to_ascii_uppercase().starts_with("START TRANSACTION")` — a
// literal space — and missed that one too. The regex was evaluated against all six; the Rust
// side's newline miss is what `apply_transaction_command`'s doc comment records as measured.
const OPENERS = ['BEGIN', 'begin work', 'START TRANSACTION', '/* c */ BEGIN', '-- c\nBEGIN', 'START\nTRANSACTION'];

Deno.test('a managed transaction() under a caller\'s open block is a savepoint, in every opener spelling', async () => {
	// The lost-row shape, and the property it was protecting is unchanged — the caller's
	// in-transaction write must survive the managed transaction's failure. What changed is how:
	// this used to be a loud `25001` refusal because pglet had no savepoints, so the managed call
	// could not run at all; now it runs as a savepoint and only ITS OWN write is discarded. That is
	// the rollback-per-test harness idiom — the harness holds the outer block, the code under test
	// calls `transaction()` normally.
	//
	// The opener spellings are still the point. The adapter used to decide its transaction state by
	// matching the SQL text (`/^\s*begin\b/i`), which cannot see past a leading comment, so under
	// `/* c */ BEGIN` it believed there was no transaction; today it asks the engine for its depth,
	// which is true of every spelling the engine accepts. Under the old mirror this shape lost the
	// caller's row outright — the managed ROLLBACK took it down with its own.
	for (const opener of OPENERS) {
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		await db.query(opener);
		await db.query('INSERT INTO t VALUES (2)'); // the caller's in-transaction write
		const err = await db.transaction(async (tx) => {
			await tx.query('INSERT INTO t VALUES (3)');
			throw new Error('boom');
		}).then(() => null, (e) => e);
		// The callback's own error surfaces — not a refusal from the adapter.
		assertInstanceOf(err, Error);
		assertEquals(err.message, 'boom', `the callback's error must surface: ${JSON.stringify(opener)}`);
		// Only the savepoint's level was discarded: the caller's row stands and `3` is gone.
		assertEquals(
			(await db.query<{id: number}>('SELECT id FROM t ORDER BY id')).rows.map((r) => r.id),
			[2],
			`only the managed level unwinds: ${JSON.stringify(opener)}`,
		);
		// …and the caller's block is still open, so it still owns the decision.
		assertEquals((await db.query('ROLLBACK')).command, 'ROLLBACK');
		assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0);
		await db.close();
	}
});

Deno.test('a SUCCEEDING managed transaction() under a caller\'s block still leaves the caller the decision', async () => {
	// The succeeding half of the test above, and the one that pins what the savepoint's `RELEASE`
	// must NOT do: merge into the enclosing block, never commit out from under it. Both rows are
	// therefore still pending, and the caller's own ROLLBACK reverts both.
	//
	// The desync this replaces was the worse half of the pair, because nothing anywhere reported an
	// error: the managed COMMIT committed the caller's earlier write, and the caller's later
	// ROLLBACK then reported success while reverting nothing.
	for (const opener of OPENERS) {
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		await db.query(opener);
		await db.query('INSERT INTO t VALUES (2)');
		await db.transaction(async (tx) => {
			await tx.query('INSERT INTO t VALUES (3)');
		});
		// Both rows are visible inside the still-open block…
		assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 2);
		assertEquals((await db.query('ROLLBACK')).command, 'ROLLBACK');
		assertEquals(
			(await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c,
			0,
			`the caller's ROLLBACK must actually revert: ${JSON.stringify(opener)}`,
		);
		await db.close();
	}
});

Deno.test('an overlapping BEGIN is 25001 in every spelling, not just the ones a regex sees', async () => {
	// The other direction, and the one a surviving text pre-check would still miss: the OVERLAPPING
	// statement is the comment-prefixed one. Postgres answers a nested BEGIN with a WARNING and a
	// no-op; this adapter is deliberately stricter (single connection, no savepoints — see the
	// single-connection contract on `PgletDb`), and that strictness has to hold for every spelling
	// the engine accepts or it is not a contract.
	for (const overlapping of OPENERS) {
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		await db.query('BEGIN');
		await db.query('INSERT INTO t VALUES (1)');
		const err = await db.query(overlapping).then(() => null, (e) => e);
		assertInstanceOf(err, Error, `must reject: ${JSON.stringify(overlapping)}`);
		assertEquals((err as {code?: string}).code, '25001', JSON.stringify(overlapping));
		// The first transaction is intact: its write is still pending and still revertible.
		assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1);
		await db.query('ROLLBACK');
		assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0);
		await db.close();
	}
});

Deno.test('a rejected overlapping BEGIN costs the caller nothing, DDL included', async () => {
	// The coupling between the guard's position and the engine. The overlap check runs AFTER the
	// statement, so the rejected BEGIN really does reach the engine — and that is only free
	// because a second BEGIN is a no-op there. A binding that re-snapshotted the catalog on it
	// would leave the caller's transaction holding a snapshot taken AFTER its own DDL, and the
	// caller's ROLLBACK would then restore storage to before the table and the catalog to after
	// it: `SELECT` 42P01 and the name 42P07 forever, from a statement that reported an error and
	// was supposed to change nothing.
	for (const overlapping of ['BEGIN', '/* c */ BEGIN']) {
		const db = create_pglet_db();
		await db.query('BEGIN');
		await db.query('CREATE TABLE ddl_in_tx (x INT)'); // the caller's in-transaction DDL
		await db.query(overlapping).then(() => null, () => undefined); // rejected 25001
		await db.query('ROLLBACK');
		// The rolled-back name must be free again, not a phantom.
		assertEquals(
			(await db.query('CREATE TABLE ddl_in_tx (x INT)')).command,
			'CREATE', // the tag's first word — `pg`'s rule; see the pg-shape test above
			`the rejected overlap must not disturb the caller's snapshot: ${JSON.stringify(overlapping)}`,
		);
		await db.close();
	}
});

Deno.test('a comment-prefixed COMMIT/ROLLBACK ends the transaction, so the next BEGIN is accepted', async () => {
	// The accept polarity — an over-correction reddens here. Reading the exit from the tag and the
	// open from the text (or vice versa) leaves `in_tx` stuck true after `/* c */ COMMIT`, and the
	// next perfectly ordinary BEGIN is then a spurious 25001. Measured tags: COMMIT / ROLLBACK.
	for (const [exit, expected] of [
		['COMMIT', 1],
		['/* c */ COMMIT', 1],
		['-- c\nCOMMIT', 1],
		['ROLLBACK', 0],
		['/* c */ ROLLBACK', 0],
		['-- c\nROLLBACK', 0],
	] as const) {
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		await db.query('BEGIN');
		await db.query('INSERT INTO t VALUES (1)');
		await db.query(exit);
		assertEquals(
			(await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c,
			expected,
			`${JSON.stringify(exit)} must ${expected ? 'keep' : 'drop'} the write`,
		);
		// The next transaction must open cleanly — no leftover `in_tx`.
		assertEquals((await db.query('BEGIN')).command, 'BEGIN', JSON.stringify(exit));
		await db.query('ROLLBACK');
		await db.close();
	}
});

Deno.test('close is idempotent; use-after-close is a clear error, not a wasm panic', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	const first = db.close();
	assert(db.close() === first, 'every close() answers the one drain');
	await first;
	await db.close(); // idempotent, no throw
	await assertRejects(() => db.query('SELECT 1'), Error, 'this PgletDb is closed');
	await assertRejects(() => db.transaction(async () => 1), Error, 'this PgletDb is closed');
	await assertRejects(() => db.fork(), Error, 'this PgletDb is closed');
});

Deno.test('close() drains the lane: a fire-and-forget write issued before it lands', async () => {
	// `db.query(<write>); await db.close();` in one block: the write was queued before the close,
	// so it runs first. A database that lost it would answer this from a fork taken in between —
	// so the fork is queued too, and sees the row.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	db.query('INSERT INTO t VALUES (1)'); // fire-and-forget
	const forked = db.fork(); // queued behind the write
	const closed = db.close(); // queued behind both
	await assertRejects(() => db.query('SELECT 1'), Error, 'this PgletDb is closed'); // refused at once
	const f = await forked;
	await closed;
	assertEquals((await f.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1);
	await f.close();
});

Deno.test('fork() and close() inside a transaction() callback reject rather than deadlock', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		const fork_err = await assertRejects(() => db.fork(), Error, 'while a transaction() callback is running');
		assertEquals((fork_err as {code?: string}).code, '25001');
		await assertRejects(() => db.close(), Error, 'while a transaction() callback is running');
	});
	// the rejected close() did not close it
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1);
	await db.close();
});

Deno.test('a close() pending before a callback starts still refuses a close() made inside it, not hangs', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	let inner: unknown;
	const done = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		inner = await db.close().then(() => 'settled', (e) => (e as {code?: string}).code);
	});
	const outer = db.close(); // accepted: the transaction is queued, its callback not yet running
	await done;
	await outer;
	assertEquals(inner, '25001');
});

Deno.test('close() from outside a running transaction() callback is refused, the db left open', async () => {
	// The undecidable case: a call made while a callback runs cannot be told from one inside it.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	let release!: () => void;
	const gate = new Promise<void>((r) => (release = r));
	let entered!: () => void;
	const inside = new Promise<void>((r) => (entered = r));
	const running = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		entered();
		await gate;
	});
	await inside;
	const err = await assertRejects(() => db.close(), Error, 'cannot be told apart');
	assertEquals((err as {code?: string; severity?: string}).code, '25001');
	assertEquals((err as {code?: string; severity?: string}).severity, 'ERROR');
	await assertRejects(() => db.fork(), Error, 'cannot be told apart');
	release();
	await running;
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1); // still open
	await db.close();
});

Deno.test('a transaction queued before close() runs whole, its tx handle working, before the free', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	// Same synchronous turn: the transaction is queued, its callback not yet running, so the close
	// is accepted and drains behind it.
	const done = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		await new Promise((r) => setTimeout(r, 5)); // the close was accepted before this callback began
		return (await tx.query('SELECT count(*)::int AS c FROM t')).rows[0].c;
	});
	const forked = db.fork(); // queued behind the transaction, ahead of the close
	const closed = db.close();
	assertEquals(await done, 1);
	const f = await forked;
	await closed;
	assertEquals((await f.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1); // committed before the fork
	await f.close();
});

Deno.test('[Symbol.asyncDispose] is close()', async () => {
	const db = create_pglet_db();
	const dispose = (db as unknown as Record<symbol, () => Promise<void>>)[Symbol.asyncDispose];
	assert(dispose === db.close);
	await dispose.call(db);
	await assertRejects(() => db.query('SELECT 1'), Error, 'this PgletDb is closed');
});

Deno.test('fork() rejects 25001 under a block opened by a raw BEGIN', async () => {
	const db = create_pglet_db();
	await db.query('BEGIN');
	const err = await assertRejects(() => db.fork(), Error);
	assertEquals((err as {code?: string}).code, '25001');
	await db.query('ROLLBACK');
	await (await db.fork()).close(); // forkable again once the block ends
	await db.close();
});

Deno.test('the tx handle is refused once its transaction settles', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	let leaked: {query: (sql: string) => Promise<unknown>} | undefined;
	await db.transaction(async (tx) => {
		leaked = tx;
		await tx.query('INSERT INTO t VALUES (1)');
	});
	await assertRejects(() => leaked!.query('INSERT INTO t VALUES (2)'), Error, 'transaction has ended');
	// and after a rolled-back one
	await assertRejects(() =>
		db.transaction(async (tx) => {
			leaked = tx;
			throw new Error('boom');
		}),
	);
	await assertRejects(() => leaked!.query('SELECT 1'), Error, 'transaction has ended');
	assertEquals((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1);
	await db.close();
});

Deno.test('a value the binding cannot convert rejects with an Error, not a bare string', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (a INT[])');
	const circular: Record<string, unknown> = {};
	circular.self = circular;
	for (const [sql, params] of [
		['INSERT INTO t VALUES ($1)', [{not: 'an array'}]],
		['SELECT $1::jsonb', [circular]],
	] as const) {
		let caught: unknown;
		try {
			await db.query(sql, [...params]);
		} catch (e) {
			caught = e;
		}
		assert(caught instanceof Error, `${sql}: ${typeof caught} ${String(caught)}`);
	}
	await db.close();
});

Deno.test('fork-per-test: the headline pattern — seed once, fork isolated copies', async () => {
	// seed a base ONCE (the migrate-once/fork-per-test idiom the test-fixture audience uses)
	const base = create_pglet_db();
	await base.query('CREATE TABLE u (id INT PRIMARY KEY, name TEXT)');
	await base.query(`INSERT INTO u VALUES (1, 'seed')`);
	const count = async (d: ReturnType<typeof create_pglet_db>) =>
		(await d.query('SELECT count(*)::int AS c FROM u')).rows[0].c;

	// each fork sees the seed, writes are isolated from the base AND from sibling forks
	const f1 = await base.fork();
	const f2 = await base.fork();
	await f1.query(`INSERT INTO u VALUES (2, 'f1')`);
	await f2.query(`INSERT INTO u VALUES (3, 'f2')`);
	assertEquals(await count(f1), 2);
	assertEquals(await count(f2), 2);
	assertEquals(await count(base), 1); // base untouched by either fork

	// a fork of a fork inherits its parent's uncommitted-to-base writes
	const ff = await f1.fork();
	await ff.query(`INSERT INTO u VALUES (4, 'ff')`);
	assertEquals(await count(ff), 3); // seed + f1's row + ff's row
	assertEquals(await count(f1), 2); // f1 untouched by its child

	// closing a fork doesn't disturb the base — still re-forkable
	await f1.close();
	await f2.close();
	await ff.close();
	assertEquals(await count(base), 1);
	const f3 = await base.fork();
	assertEquals(await count(f3), 1);
	await f3.close();
	await base.close();
});
