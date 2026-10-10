/**
 * End-to-end smoke test for the assembled @fuzdev/pglet-wasm package: loads the built
 * Node entry (`pkg-npm/index.js` — sync `initSync` at import) and exercises the ergonomic
 * adapter + the value-coercion contract, the array recursion especially (the gap every
 * hand-rolled adapter had). Run via `deno task test:npm` (builds pkg-npm first).
 */
import {assert, assertEquals, assertInstanceOf} from '@std/assert';
// deno-lint-ignore no-import-prefix
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';

Deno.test('coercion contract — scalars + recursive array elements', async () => {
	const db = create_pglet_db();
	await db.query(
		`CREATE TABLE t (id BIGINT PRIMARY KEY, tags BIGINT[], ts TIMESTAMP, tsz TIMESTAMPTZ, d DATE, blob BYTEA, name TEXT)`,
	);
	await db.query(
		`INSERT INTO t VALUES (42, ARRAY[1, 2]::bigint[],` +
			` '2024-06-15 12:30:45.123', '2024-06-15 12:30:45.123+00', '2024-06-15', '\\xdeadbeef'::bytea, 'hi')`,
	);
	const {rows} = await db.query('SELECT id, tags, ts, tsz, d, blob, name FROM t');
	const r = rows[0] as Record<string, unknown>;

	// int8 → bigint (default) — the wasm returns BigInt for an int8 column regardless of magnitude
	assertEquals(r.id, 42n);
	// int8[] → bigint[] — the recursive-coercion contract (raw wasm gives BigInt elements;
	// the policy layer must reach INTO arrays, which the old hand-rolled coercers didn't)
	assertEquals(r.tags, [1n, 2n]);
	// timestamp / timestamptz → Date, parsed as UTC (pglet ≡ Postgres at TimeZone=UTC)
	assertInstanceOf(r.ts, Date);
	assertEquals((r.ts as Date).toISOString(), '2024-06-15T12:30:45.123Z');
	assertInstanceOf(r.tsz, Date);
	assertEquals((r.tsz as Date).toISOString(), '2024-06-15T12:30:45.123Z');
	// date → ISO string (default, TZ-proof)
	assertEquals(r.d, '2024-06-15');
	// bytea → Buffer (Node) or Uint8Array (Buffer is a Uint8Array subclass)
	assertInstanceOf(r.blob, Uint8Array);
	assertEquals([...(r.blob as Uint8Array)], [0xde, 0xad, 0xbe, 0xef]);
	assertEquals(r.name, 'hi');
	await db.close();
});

Deno.test('int8 knob (number) + rowMode:array recurse too', async () => {
	const db = create_pglet_db({coercion: {int8: 'number'}});
	await db.query('CREATE TABLE t (id BIGINT, tags BIGINT[])');
	await db.query('INSERT INTO t VALUES (42, ARRAY[1, 2]::bigint[])');
	const obj = await db.query('SELECT id, tags FROM t');
	assertEquals(obj.rows[0].id, 42); // number, not bigint
	assertEquals(obj.rows[0].tags, [1, 2]); // number[] — array elements honor the knob
	const arr = await db.query<unknown[]>('SELECT id, tags FROM t', [], {rowMode: 'array'});
	assertEquals(arr.rows[0], [42, [1, 2]]); // positional rows for drizzle
	await db.close();
});

// `avg()` of an integer and `sum()` of a `bigint` are `numeric`, so they arrive as the
// `numeric` string `pg` hands over — exact past 2^53 and past the `bigint` range — while
// `sum()` of an `integer` is a `bigint`, under the int8 knob like any other.
Deno.test('an integer avg and a bigint sum are numeric strings', async () => {
	for (const [int8, sum_i] of [['bigint', 6n], ['number', 6], ['string', '6']] as const) {
		const db = create_pglet_db({coercion: {int8}});
		await db.query('CREATE TABLE t (i INT, b BIGINT)');
		await db.query(
			'INSERT INTO t VALUES (1, 9223372036854775807), (2, 9223372036854775807), (3, 9223372036854775807)',
		);
		const {rows} = await db.query('SELECT avg(i) AS a, sum(b) AS s, avg(b) AS ab, sum(i) AS si FROM t');
		assertEquals(rows[0], {
			a: '2.0000000000000000',
			s: '27670116110564327421',
			ab: '9223372036854775807',
			si: sum_i,
		});
		const empty = await db.query('SELECT avg(i) AS a, sum(b) AS s FROM t WHERE false');
		assertEquals(empty.rows[0], {a: null, s: null});
		await db.close();
	}
});

Deno.test('bound $N params round-trip', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT, name TEXT)');
	await db.query('INSERT INTO t VALUES ($1, $2)', [7, 'seven']);
	const {rows} = await db.query('SELECT name FROM t WHERE id = $1', [7]);
	assertEquals(rows[0].name, 'seven');
	await db.close();
});

// The seeding idiom through the adapter, with its bound as a JS number: the bare `$1` is
// the `integer` the series' overload gives it, and a bound past the series' row limit is
// refused `54000` rather than allocated.
Deno.test('generate_series seeds a table with a bound parameter', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT PRIMARY KEY, label TEXT)');
	await db.query("INSERT INTO t SELECT g, 'row ' || g FROM generate_series(1, $1) g", [25]);
	const {rows} = await db.query('SELECT count(*) AS c, max(id) AS m FROM t');
	assertEquals([rows[0].c, rows[0].m], [25n, 25]);
	const err = await db
		.query('SELECT count(*) FROM generate_series(1, $1) g', [5_000_000])
		.then(() => null, (e) => e);
	assertEquals((err as {code?: string}).code, '54000');
	await db.close();
});

// The hand-written idiom `WHERE status = $1::order_status`, with a test passing a status
// the enum does not hold: 18.6 runs `enum_in` on the bound value at Bind, so the query is
// `22P02` whatever the rows — over an empty table too — and a member binds as before.
Deno.test('a bound $N cast to an enum converts at bind, member or not', async () => {
	const db = create_pglet_db();
	await db.query(`CREATE TYPE order_status AS ENUM ('open', 'shipped')`);
	await db.query('CREATE TABLE orders (id INT, status order_status NOT NULL)');
	const refused = async (sql: string, params: unknown[]) => {
		const err = await db.query(sql, params).then(() => null, (e) => e);
		assert(err instanceof Error, `${sql}: accepted`);
		return (err as {code?: string}).code;
	};
	const sql = 'SELECT id FROM orders WHERE status = $1::order_status';
	assertEquals(await refused(sql, ['lost']), '22P02');
	await db.query(`INSERT INTO orders VALUES (1, 'open'), (2, 'shipped')`);
	assertEquals(await refused(sql, ['lost']), '22P02');
	// Labels are compared as written, never folded.
	assertEquals(await refused(sql, ['OPEN']), '22P02');
	const {rows} = await db.query(sql, ['shipped']);
	assertEquals(rows.map((r) => r.id), [2]);
	await db.close();
});

// …but only where the cast is the use that fixes `$1`'s type. 18.6 fixes an untyped
// parameter at its first use in parse-analysis order, so when an earlier use has made it
// `text` the cast converts a `text` value, and only where it is evaluated: the guarded
// idioms bind a value the enum lacks. The order pair first — the same two uses, swapped.
Deno.test('a bound $N cast to an enum after a text use converts on the value', async () => {
	const db = create_pglet_db();
	await db.query(`CREATE TYPE order_status AS ENUM ('open', 'shipped')`);
	await db.query('CREATE TABLE orders (id INT PRIMARY KEY, status order_status)');
	await db.query(`INSERT INTO orders VALUES (1, 'open')`);
	const code = (sql: string, params: unknown[]) =>
		db.query(sql, params).then(
			() => null,
			(e) => (e as {code?: string}).code ?? String(e),
		);
	assertEquals(
		await code('SELECT $1::order_status::text AS a, $1::text AS b FROM orders WHERE false', [
			'lost',
		]),
		'22P02',
	);
	assertEquals(
		await code('SELECT $1::text AS a, $1::order_status::text AS b FROM orders WHERE false', [
			'lost',
		]),
		null,
	);
	// A `WITH` query is analysed before the statement's own clauses, wherever it is read.
	assertEquals(
		await code(
			'WITH a AS (SELECT $1::text AS v) SELECT $1::order_status::text AS s, (SELECT v FROM a) AS v FROM orders WHERE false',
			['lost'],
		),
		null,
	);
	assertEquals(
		await code(
			'WITH a AS (SELECT $1::order_status::text AS v FROM orders WHERE false) SELECT $1::text AS s, (SELECT v FROM a) AS v FROM orders WHERE false',
			['lost'],
		),
		'22P02',
	);
	const {rows} = await db.query(
		`SELECT CASE WHEN $1 = '' THEN NULL ELSE $1::order_status END::text AS s`,
		[''],
	);
	assertEquals(rows, [{s: null}]);
	await db.query(
		`UPDATE orders SET status = CASE WHEN $1 = '' THEN NULL ELSE $1::order_status END WHERE id = 1`,
		[''],
	);
	await db.query('UPDATE orders SET status = $1::order_status WHERE $1 <> $2', ['lost', 'lost']);
	const after = await db.query('SELECT id, status::text AS s FROM orders ORDER BY id');
	assertEquals(after.rows, [{id: 1, s: null}]);
	// A row that reaches the conversion still refuses the value.
	assertEquals(
		await code('SELECT $1::text AS a, $1::order_status::text AS b FROM orders', ['lost']),
		'22P02',
	);
	await db.close();
});

// A parameter the statement itself gives no type: 18.6 refuses it at Parse (`42P18`, or
// `42P08` where a later use types it), whatever value is bound. The value's JS type must
// not stand in — a number, a string and a null are refused alike — and the spellings 18.6
// answers bind as before.
Deno.test('a $N the statement gives no type is refused, whatever JS value is bound', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE people (id INT PRIMARY KEY, name TEXT)');
	await db.query(`INSERT INTO people VALUES (1, 'ann'), (2, 'bo')`);
	const code = (sql: string, params: unknown[]) =>
		db.query(sql, params).then(
			() => null,
			(e) => (e as {code?: string}).code ?? String(e),
		);
	const first = 'SELECT count(*) AS c FROM people WHERE ($1 IS NULL OR name = $1)';
	for (const value of [null, 'ann', 5, true]) {
		assertEquals(await code('SELECT $1 IS NULL AS b', [value]), '42P18');
		assertEquals(await code('SELECT count($1) AS c FROM people', [value]), '42P18');
		assertEquals(await code(first, [value]), '42P08');
	}
	// The comparison first, or a cast: the parameter has its type before the null test.
	for (
		const sql of [
			'SELECT count(*)::int AS c FROM people WHERE (name = $1 OR $1 IS NULL)',
			'SELECT count(*)::int AS c FROM people WHERE ($1::text IS NULL OR name = $1)',
		]
	) {
		assertEquals((await db.query(sql, [null])).rows, [{c: 2}]);
		assertEquals((await db.query(sql, ['bo'])).rows, [{c: 1}]);
	}
	// A refused statement writes nothing and leaves the database serving.
	assertEquals(await code('DELETE FROM people WHERE $1 IS NULL', [null]), '42P18');
	assertEquals((await db.query('SELECT count(*)::int AS c FROM people')).rows, [{c: 2}]);
	// A bare select-list parameter is `text`, as 18.6 resolves it: a number comes back as
	// the string node-postgres would return.
	assertEquals((await db.query('SELECT $1 AS p', [5])).rows, [{p: '5'}]);
	await db.close();
});

// The guard idioms: `AND` / `OR` stop at a left operand that decides them, so a conversion
// the guard keeps away — of the empty string, which the enum does not hold — is never
// evaluated, bound or written into the statement.
// A bare `$N` entry of an `INSERT … SELECT` source is the column's type, as PostgreSQL
// coerces it — so a `Buffer`, an array and an object go in as they do through `VALUES`.
Deno.test('an INSERT … SELECT parameter is bound as its target column', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE ty (id INT PRIMARY KEY, by BYTEA, ta TEXT[], ia INT[], j JSONB, n INT)');
	const row = async (sql: string, params: unknown[]) =>
		(await db.query(sql, params, {rowMode: 'array'})).rows[0] as unknown as unknown[];
	const by = await row('INSERT INTO ty (id, by) SELECT 1, $1 RETURNING by', [Buffer.from('abc')]);
	assertEquals([...by[0] as Uint8Array], [97, 98, 99]);
	assertEquals(await row('INSERT INTO ty (id, ta, ia) SELECT 2, $1, $2 RETURNING ta, ia', [['a', 'b'], [1, 2]]), [
		['a', 'b'],
		[1, 2],
	]);
	assertEquals(await row('INSERT INTO ty (id, j, n) SELECT $1, $2, $3 RETURNING id, j, n', [3, {a: 1}, 5]), [
		3,
		{a: 1},
		5,
	]);
	// The target types the parameter before `RETURNING` reads it.
	assertEquals(await row('INSERT INTO ty (id, n) SELECT 4, $1 RETURNING $1 IS NULL, $1', [7]), [false, 7]);
	await db.close();
});

// A `text` parameter cannot refuse a spelling, so it must be handed the one node-postgres
// sends: a `Buffer` is its bytes read as text, an array is an array literal with each
// string, number and boolean element quoted, a number is JS's own spelling of it up to 2^53. Each expected string is what `pg`
// against PostgreSQL 18 returns for the same call.
Deno.test('a text parameter reads a JS value as node-postgres sends it', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE notes (id INT PRIMARY KEY, t TEXT, vc VARCHAR(40))');
	const text = async (sql: string, value: unknown) =>
		(await db.query(sql, [value], {rowMode: 'array'})).rows[0][0];
	const cases: Array<[unknown, string]> = [
		[Buffer.from('abc'), 'abc'],
		[new Uint8Array(0), ''],
		[[1, 2], '{"1","2"}'],
		[['a', 'b'], '{"a","b"}'],
		[[[1, 2], [3, 4]], '{{"1","2"},{"3","4"}}'],
		[['a"b\\c', null, 'NULL', '', '{z}', 'a,b'], '{"a\\"b\\\\c",NULL,"NULL","","{z}","a,b"}'],
		[[true, false], '{"true","false"}'],
		[[{a: 1}], '{"{\\"a\\":1}"}'],
		[[10n], '{"10"}'],
		[[], '{}'],
		[[undefined], '{NULL}'],
		[[Buffer.from('abc')], '{\\\\x616263}'],
		[{a: 1}, '{"a":1}'],
		[10n, '10'],
		[true, 'true'],
		[1.5, '1.5'],
		[1e21, '1e+21'],
		[1e-7, '1e-7'],
		[-1.5e-7, '-1.5e-7'],
		[123456789012345680000, '123456789012345680000'],
		[2 ** 53, '9007199254740992'],
		// Between 2^53 and 2^63 every digit is spelled, where `pg` sends JS's seventeen: `1152921504606847000`.
		[2 ** 60, '1152921504606846976'],
		[-0, '0'],
		[0.000001, '0.000001'],
	];
	for (const [value, expected] of cases) {
		for (
			const sql of [
				'SELECT $1 AS x',
				'SELECT $1::text AS x',
				'INSERT INTO notes (id, t) VALUES (1, $1) RETURNING t',
				'INSERT INTO notes (id, vc) VALUES (2, $1) RETURNING vc',
			]
		) {
			assertEquals(await text(sql, value), expected, `${sql} <- ${Deno.inspect(value)}`);
			await db.query('DELETE FROM notes');
		}
	}
	// Bytes the server encoding does not hold are refused, as PostgreSQL refuses them.
	const code = (value: unknown) =>
		db.query('SELECT $1::text AS x', [value]).then(() => null, (e) => (e as {code?: string}).code);
	assertEquals(await code(Buffer.from([0xff, 0x61])), '22021');
	assertEquals(await code(Buffer.from('a\0b')), '22021');
	assertEquals(await code(['a\0b']), '22021');
	// The other targets keep their own readers: bytes for `bytea`, elements for an array,
	// JSON for `jsonb`.
	assertEquals([...(await text('SELECT $1::bytea AS x', Buffer.from('ab'))) as Uint8Array], [97, 98]);
	assertEquals(await text('SELECT $1::text[] AS x', ['a', 'b']), ['a', 'b']);
	assertEquals(await text('SELECT $1::jsonb AS x', [1, 2]), [1, 2]);
	await db.close();
});

// A JS string bound to a `bytea` parameter is text for the type's input function, as it is
// when node-postgres sends it: hex behind `\x`, the escape format otherwise. A `Buffer` or a
// `Uint8Array` is its bytes and is read by nothing. Each expectation is `pg` against
// PostgreSQL 18.
Deno.test('a string bound to a bytea parameter is read as PostgreSQL reads it', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE blobs (id INT PRIMARY KEY, b BYTEA)');
	const bytes_of = (v: unknown) => v === null ? null : [...v as Uint8Array];
	const utf8 = (s: string) => [...new TextEncoder().encode(s)];
	const cases: Array<[unknown, Array<number>]> = [
		['abcd', utf8('abcd')],
		['abc', utf8('abc')],
		['hello', utf8('hello')],
		[' ', [0x20]],
		['', []],
		[' abcd ', utf8(' abcd ')],
		['hé', [0x68, 0xc3, 0xa9]],
		['\\141\\\\', [0x61, 0x5c]],
		['\\000\\377', [0, 255]],
		['\\x6162', [0x61, 0x62]],
		['\\x6162 63', [0x61, 0x62, 0x63]],
		['\\x', []],
		[Buffer.from('abcd'), utf8('abcd')],
		[Buffer.from('\\x6162'), utf8('\\x6162')],
		[Buffer.from('a\\b'), utf8('a\\b')],
		[new Uint8Array([0, 255, 0x5c, 0x58]), [0, 255, 0x5c, 0x58]],
	];
	for (const [value, expected] of cases) {
		for (
			const sql of [
				'SELECT $1::bytea AS x',
				'INSERT INTO blobs VALUES (1, $1) RETURNING b',
				'INSERT INTO blobs VALUES (1, NULL) ON CONFLICT (id) DO UPDATE SET b = $1 RETURNING b',
			]
		) {
			const {rows} = await db.query(sql, [value], {rowMode: 'array'});
			assertEquals(bytes_of(rows[0][0]), expected, `${sql} <- ${Deno.inspect(value)}`);
		}
		await db.query('DELETE FROM blobs');
	}
	const code = (value: unknown) =>
		db.query('SELECT $1::bytea AS x', [value]).then(() => null, (e) => (e as {code?: string}).code);
	assertEquals(await code('\\X6162'), '22P02');
	assertEquals(await code(' \\x6162'), '22P02');
	assertEquals(await code('a\\b'), '22P02');
	assertEquals(await code('\\400'), '22P02');
	assertEquals(await code('\\x616'), '22023');
	assertEquals(await code('\\x6 162'), '22023');
	// A comparison reads its parameter the same way: the string finds the row holding its
	// characters, not the one holding its hex reading.
	await db.query("INSERT INTO blobs VALUES (1, '\\x63616665'), (2, '\\xcafe')");
	const ids = async (value: unknown) =>
		(await db.query('SELECT id FROM blobs WHERE b = $1', [value], {rowMode: 'array'})).rows.map((r) => r[0]);
	assertEquals(await ids('cafe'), [1]);
	assertEquals(await ids('\\xcafe'), [2]);
	assertEquals(await ids(Buffer.from('cafe')), [1]);
	// An array's string elements are read by the same function, a `Buffer` element is bytes.
	const {rows} = await db.query('SELECT $1::bytea[] AS x', [['abcd', '\\x6162', Buffer.from('abcd'), null]], {
		rowMode: 'array',
	});
	assertEquals((rows[0][0] as Array<unknown>).map(bytes_of), [utf8('abcd'), [0x61, 0x62], utf8('abcd'), null]);
	await db.close();
});

// An element of a JS array bound to an array parameter is read by node-postgres's rule
// for an array element, which is not the rule for the same value bound on its own: a
// `Buffer` element is `\\x` and its hex whatever the element type, and a nested array is
// a second dimension, which pglet does not have. Each expectation is `pg` against
// PostgreSQL 18, except the refusal.
Deno.test('an array parameter reads its elements as node-postgres sends them', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE notes (id INT PRIMARY KEY, t TEXT, vc VARCHAR(40))');
	await db.query("INSERT INTO notes VALUES (1, 'abc', 'abc'), (2, '\\x616263', '\\x616263')");
	const first = async (sql: string, value: unknown) =>
		(await db.query(sql, [value], {rowMode: 'array'})).rows.map((r) => r[0]);
	// The bytes of `abc` are not the text `abc` inside an array.
	for (
		const sql of [
			'SELECT id FROM notes WHERE t = ANY($1) ORDER BY id',
			'SELECT id FROM notes WHERE vc = ANY($1) ORDER BY id',
			'SELECT id FROM notes WHERE t = ANY($1::text[]) ORDER BY id',
			'SELECT id FROM notes WHERE t IN (SELECT unnest($1::text[])) ORDER BY id',
		]
	) {
		assertEquals(await first(sql, [Buffer.from('abc')]), [2], sql);
		assertEquals(await first(sql, [new Uint8Array([0xff, 0])]), [], sql);
		assertEquals(await first(sql, [Buffer.from('abc'), 'abc']), [1, 2], sql);
	}
	assertEquals(await first('SELECT $1::text[] AS x', [Buffer.from('abc'), 'abc', null]), [[
		'\\x616263',
		'abc',
		null,
	]]);
	assertEquals(await first('SELECT $1::varchar[] AS x', [new Uint8Array([0xff, 0])]), [['\\xff00']]);
	assertEquals(await first('SELECT $1::text[] AS x', [new Uint8Array(0)]), [['\\x']]);
	// A `bytea` element is the bytes themselves.
	const bytes = (await first('SELECT $1::bytea[] AS x', [Buffer.from('ab')]))[0] as Uint8Array[];
	assertEquals([...bytes[0]], [97, 98]);
	const code = (sql: string, value: unknown) =>
		db.query(sql, [value]).then(() => null, (e) => (e as {code?: string}).code);
	// The other element types refuse the spelling, as PostgreSQL refuses it.
	assertEquals(await code('SELECT $1::int[] AS x', [Buffer.from('1')]), '22P02');
	assertEquals(await code('SELECT $1::jsonb[] AS x', [Buffer.from('1')]), '22P02');
	// A nested array is a second dimension there, and is refused here whatever the element
	// type: left to the element's reader, `bytea`, `jsonb` and the timestamp types would
	// take the inner array's JSON spelling as one element.
	await db.query('CREATE TABLE blobs (id INT PRIMARY KEY, b BYTEA[])');
	const element_types = [
		'text',
		'varchar',
		'bytea',
		'jsonb',
		'timestamp',
		'timestamptz',
		'date',
		'time',
		'interval',
		'int2',
		'int4',
		'int8',
		'numeric',
		'float4',
		'float8',
		'bool',
		'uuid',
	];
	for (
		const nested of [
			[[1, 2], [3, 4]],
			[['a']],
			[['a', 'b']],
			[[]],
			[[Buffer.from('abc')]],
			[['2024-01-01'], ['2024-01-02']],
			[[null]],
		]
	) {
		for (
			const sql of [
				...element_types.map((t) => `SELECT $1::${t}[] AS x`),
				'SELECT id FROM notes WHERE t = ANY($1)',
				'INSERT INTO blobs VALUES (1, $1)',
			]
		) {
			assertEquals(await code(sql, nested), '0A000', `${sql} <- ${Deno.inspect(nested)}`);
		}
	}
	// An element ahead of the nested array is read first; one its type takes leaves the
	// refusal to the nested array.
	for (const t of ['text', 'varchar', 'bytea']) {
		assertEquals(await code(`SELECT $1::${t}[] AS x`, ['a', ['b']]), '0A000', t);
		assertEquals(await code(`SELECT $1::${t}[] AS x`, [Buffer.from('a'), [Buffer.from('b')]]), '0A000', t);
	}
	assertEquals((await db.query('SELECT count(*)::int AS n FROM blobs')).rows[0].n, 0);
	await db.close();
});

// A JS array bound to a scalar `bytea` parameter is the array literal node-postgres
// sends, read as `bytea`'s escape format — not the array's JSON spelling.
Deno.test('a JS array bound to a bytea parameter is the text node-postgres sends', async () => {
	const db = create_pglet_db();
	const text_of = async (value: unknown) =>
		new TextDecoder().decode(
			(await db.query('SELECT $1::bytea AS x', [value], {rowMode: 'array'})).rows[0][0] as Uint8Array,
		);
	assertEquals(await text_of(['a', 'b']), '{"a","b"}');
	assertEquals(await text_of([1, 2]), '{"1","2"}');
	assertEquals(await text_of([]), '{}');
	assertEquals(await text_of([null, true]), '{NULL,"true"}');
	assertEquals(await text_of([['a']]), '{{"a"}}');
	await db.close();
});

// What a JS value bound to a `bytea` parameter stores is decided by the value's kind, never
// by `bytea`'s reader taking whatever text the value was turned into. A byte view — any
// `ArrayBuffer.isView` value — is the bytes of its window, as node-postgres sends it; each
// expectation here is `pg` against PostgreSQL 18.
Deno.test('a byte view bound to a bytea parameter is the bytes of its window', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE img (id INT PRIMARY KEY, px BYTEA, parts BYTEA[])');
	const bytes_of = (v: unknown) => [...v as Uint8Array];
	const shared = new SharedArrayBuffer(4);
	new Uint8Array(shared).set([1, 2, 3, 4]);
	const of = (...bytes: number[]) => new Uint8Array(bytes).buffer;
	const views: Array<[string, () => ArrayBufferView, number[]]> = [
		['Uint8Array', () => new Uint8Array([1, 2]), [1, 2]],
		['Buffer', () => Buffer.from('ab'), [0x61, 0x62]],
		['Uint8ClampedArray', () => new Uint8ClampedArray([1, 2]), [1, 2]],
		['Int8Array', () => new Int8Array([1, -1]), [1, 255]],
		['Uint16Array', () => new Uint16Array([1, 2]), [1, 0, 2, 0]],
		['Int16Array', () => new Int16Array([-2]), [0xfe, 0xff]],
		['Uint32Array', () => new Uint32Array([1]), [1, 0, 0, 0]],
		['Int32Array', () => new Int32Array([-1]), [255, 255, 255, 255]],
		['Float32Array', () => new Float32Array([1]), [0, 0, 0x80, 0x3f]],
		['Float64Array', () => new Float64Array([1]), [0, 0, 0, 0, 0, 0, 0xf0, 0x3f]],
		['BigInt64Array', () => new BigInt64Array([1n]), [1, 0, 0, 0, 0, 0, 0, 0]],
		['BigUint64Array', () => new BigUint64Array([2n]), [2, 0, 0, 0, 0, 0, 0, 0]],
		['DataView', () => new DataView(of(1, 2)), [1, 2]],
		// The window, not the buffer under it.
		['Uint8Array window', () => new Uint8Array(of(9, 8, 7, 6), 1, 2), [8, 7]],
		['Uint16Array window', () => new Uint16Array(of(9, 9, 1, 0, 9, 9, 9, 9), 2, 1), [1, 0]],
		['DataView window', () => new DataView(of(9, 8, 7, 6), 1, 2), [8, 7]],
		['Buffer window', () => Buffer.from(of(9, 8, 7, 6), 1, 2), [8, 7]],
		['Int16Array over a SharedArrayBuffer', () => new Int16Array(shared, 2, 1), [3, 4]],
		['empty Float32Array', () => new Float32Array(0), []],
		// Bytes that spell `bytea`'s hex format are those bytes, not their hex reading.
		['Uint16Array spelling \\x41', () => new Uint16Array([0x785c, 0x3134]), [0x5c, 0x78, 0x34, 0x31]],
	];
	for (const [name, view, expected] of views) {
		const scalar = await db.query('SELECT $1::bytea AS x', [view()], {rowMode: 'array'});
		assertEquals(bytes_of(scalar.rows[0][0]), expected, `scalar ${name}`);
		const stored = await db.query('INSERT INTO img VALUES (1, $1, $2) RETURNING px, parts', [view(), [
			'a',
			view(),
			null,
		]], {rowMode: 'array'});
		assertEquals(bytes_of(stored.rows[0][0]), expected, `stored ${name}`);
		const parts = stored.rows[0][1] as Array<unknown>;
		assertEquals([bytes_of(parts[0]), bytes_of(parts[1]), parts[2]], [[0x61], expected, null], `element ${name}`);
		const found = await db.query('SELECT id FROM img WHERE px = $1', [view()], {rowMode: 'array'});
		assertEquals(found.rows as unknown, [[1]], `comparison ${name}`);
		await db.query('DELETE FROM img');
	}
	// A view that is an element of a JS array bound to the scalar is `\x` and its hex inside
	// the array literal node-postgres sends, which `bytea`'s escape format reads.
	const literal = await db.query('SELECT $1::bytea AS x', [[new Uint8ClampedArray([1, 2]), new DataView(of(3))]], {
		rowMode: 'array',
	});
	assertEquals(new TextDecoder().decode(literal.rows[0][0] as Uint8Array), '{\\x0102,\\x03}');
	await db.close();
});

// The kinds that are not a byte view. A string, a number and a boolean are the text
// node-postgres sends, which `bytea`'s reader takes as PostgreSQL 18's does. An object that
// is neither a view nor an array is refused: node-postgres sends its JSON — `{}` for an
// `ArrayBuffer` — and PostgreSQL stores that text's bytes.
Deno.test('an object that is not a byte view is refused for a bytea parameter', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE img (id INT PRIMARY KEY, px BYTEA, parts BYTEA[])');
	const text_of = async (value: unknown) =>
		new TextDecoder().decode(
			(await db.query('SELECT $1::bytea AS x', [value], {rowMode: 'array'})).rows[0][0] as Uint8Array,
		);
	assertEquals(await text_of(5), '5');
	assertEquals(await text_of(1.5), '1.5');
	assertEquals(await text_of(true), 'true');
	assertEquals(await text_of(10n), '10');
	const code = (sql: string, params: unknown[]) =>
		db.query(sql, params).then(() => null, (e) => (e as {code?: string}).code);
	const refused: Array<[string, () => unknown]> = [
		['ArrayBuffer', () => new Uint8Array([1, 2]).buffer],
		['SharedArrayBuffer', () => new SharedArrayBuffer(2)],
		['plain object', () => ({a: 1})],
		['empty object', () => ({})],
		['a Buffer that went through JSON', () => JSON.parse(JSON.stringify(Buffer.from('ab')))],
		['Map', () => new Map([[1, 2]])],
		['Set', () => new Set([1])],
		['toPostgres', () => ({toPostgres: () => 'zz'})],
		['toJSON', () => ({toJSON: () => 'q'})],
		['Proxy of a Uint8Array', () => new Proxy(new Uint8Array([1, 2]), {})],
	];
	for (const [name, value] of refused) {
		assertEquals(await code('SELECT $1::bytea AS x', [value()]), '22023', `scalar ${name}`);
		assertEquals(await code('SELECT $1::bytea[] AS x', [[value()]]), '22023', `element ${name}`);
		assertEquals(await code('SELECT $1::bytea[] AS x', [['a', value()]]), '22023', `second element ${name}`);
		assertEquals(await code('INSERT INTO img VALUES (1, $1, NULL)', [value()]), '22023', `stored ${name}`);
		assertEquals(await code('INSERT INTO img VALUES (1, NULL, $1)', [[value()]]), '22023', `stored element ${name}`);
		assertEquals(await code('SELECT id FROM img WHERE px = $1', [value()]), '22023', `comparison ${name}`);
	}
	assertEquals((await db.query('SELECT count(*)::int AS n FROM img')).rows[0].n, 0);
	await db.close();
});

// A byte view bound to another target. node-postgres sends a view's bytes whatever the
// parameter's type: a `text` or `varchar` target, and an untyped parameter, read them as
// characters; an array's element is `\x` and the hex; any other scalar type would read them
// as its own binary layout, and is refused here.
Deno.test('a byte view bound to a parameter that is not bytea', async () => {
	const db = create_pglet_db();
	const first = async (sql: string, params: unknown[]) =>
		(await db.query(sql, params, {rowMode: 'array'})).rows[0][0];
	const code = (sql: string, params: unknown[]) =>
		db.query(sql, params).then(() => null, (e) => (e as {code?: string}).code);
	const hi = (): Array<[string, ArrayBufferView]> => [
		['Uint8Array', new Uint8Array([0x68, 0x69])],
		['Uint16Array', new Uint16Array([0x6968])],
		['Uint8ClampedArray', new Uint8ClampedArray([0x68, 0x69])],
		['DataView', new DataView(new Uint8Array([0x68, 0x69]).buffer)],
		['window', new Uint8ClampedArray(new Uint8Array([0x7a, 0x68, 0x69, 0x7a]).buffer, 1, 2)],
	];
	for (
		const sql of [
			'SELECT $1::text AS x',
			'SELECT $1::varchar AS x',
			'SELECT $1 AS x',
			'SELECT coalesce($1, $2) AS x',
		]
	) {
		const params = (view: unknown) => sql.includes('$2') ? [view, null] : [view];
		for (const [name, view] of hi()) {
			assertEquals(await first(sql, params(view)), 'hi', `${sql} <- ${name}`);
		}
		// Bytes the server encoding does not hold.
		assertEquals(await code(sql, params(new Uint16Array([1, 0xff00]))), '22021', sql);
		assertEquals(await code(sql, params(new Float64Array([1]))), '22021', sql);
	}
	// An element of an array target, whatever the element type.
	assertEquals(await first('SELECT $1::text[] AS x', [[new Uint16Array([1, 2]), new DataView(new ArrayBuffer(1))]]), [
		'\\x01000200',
		'\\x00',
	]);
	assertEquals(await code('SELECT $1::jsonb[] AS x', [[new Uint8ClampedArray([1])]]), '22P02');
	assertEquals(await code('SELECT $1::int4[] AS x', [[new DataView(new ArrayBuffer(1))]]), '22P02');
	// Any other scalar target refuses a view rather than read its JSON spelling.
	for (const t of ['jsonb', 'int4', 'int8', 'numeric', 'float8', 'bool', 'uuid', 'date', 'timestamptz', 'interval']) {
		for (const [name, view] of [...hi(), ['Buffer', Buffer.from('1')] as [string, ArrayBufferView]]) {
			assertEquals(await code(`SELECT $1::${t} AS x`, [view]), '22023', `${t} <- ${name}`);
		}
	}
	// Inside a JSON value a view is still that value's JSON.
	assertEquals(await first('SELECT $1::jsonb AS x', [{a: new Uint8ClampedArray([1])}]), {a: {0: 1}});
	await db.close();
});

// A JS number past the `bigint` range is bound as the number it is — PostgreSQL's answer
// through `pg` — rather than clamped to the largest `bigint`.
Deno.test('a JS number past the bigint range is not clamped', async () => {
	const db = create_pglet_db();
	const one = async (sql: string, value: unknown) =>
		(await db.query(sql, [value], {rowMode: 'array'})).rows[0][0];
	assertEquals(await one('SELECT $1::float8 AS x', 1e21), 1e21);
	assertEquals(await one('SELECT $1::float8 AS x', -1.5e300), -1.5e300);
	assertEquals(await one('SELECT $1::float8 AS x', 1e-7), 1e-7);
	assertEquals(await one('SELECT $1::numeric AS x', 1e21), '1000000000000000000000');
	assertEquals(await one('SELECT $1::numeric AS x', 123456789012345680000), '123456789012345680000');
	const code = (sql: string, value: unknown) =>
		db.query(sql, [value]).then(() => null, (e) => (e as {code?: string}).code);
	// `1e+21` is not a `bigint` spelling on PostgreSQL either.
	assertEquals(await code('SELECT $1::int8 AS x', 1e21), '22P02');
	// A parameter this engine reports no type for is read by its JS type: a `float8`.
	assertEquals(await one('SELECT $1 + 1.5::float8 AS x', 1e21), 1e21);
	await db.close();
});

// A table that does not exist, in a FROM clause or as the statement's target, is `42P01`
// whatever the statement's parameters — PostgreSQL resolves the tables first.
Deno.test('a missing table is reported ahead of an untyped parameter', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE people (id INT PRIMARY KEY, name TEXT)');
	await db.query(`INSERT INTO people VALUES (1, 'ann')`);
	const code = (sql: string) =>
		db.query(sql, [null]).then(() => null, (e) => (e as {code?: string}).code ?? String(e));
	for (
		const sql of [
			'SELECT id FROM nope WHERE $1 IS NULL',
			'SELECT id FROM nope WHERE ($1 IS NULL OR id = $1)',
			'UPDATE nope SET n = 1 WHERE $1 IS NULL',
			'DELETE FROM nope WHERE $1 IS NULL',
			'INSERT INTO nope (a) VALUES (1) RETURNING $1 IS NULL',
			'SELECT p.id FROM people p JOIN nope n ON n.id = p.id WHERE $1 IS NULL',
		]
	) {
		assertEquals(await code(sql), '42P01', sql);
	}
	assertEquals(await code('SELECT id FROM people WHERE $1 IS NULL'), '42P18');
	// Inside a subquery of an expression the missing table is noticed only by a row that
	// evaluates it, so the parameter's refusal stands rather than the statement answering.
	assertEquals(await code('SELECT id FROM people WHERE id IN (SELECT id FROM nope) AND $1 IS NULL'), '42P18');
	assertEquals(await code('SELECT id FROM people WHERE false AND id IN (SELECT id FROM nope) AND $1 IS NULL'), '42P18');
	await db.close();
});

Deno.test('a guarded conversion to an enum is not evaluated past a deciding AND / OR', async () => {
	const db = create_pglet_db();
	await db.query(`CREATE TYPE status AS ENUM ('open', 'closed')`);
	await db.query('CREATE TABLE tasks (id INT, st status, s TEXT)');
	await db.query(`INSERT INTO tasks VALUES (1, 'open', '')`);
	const count = async (sql: string, params: unknown[] = []) =>
		(await db.query(sql, params)).rows[0].n;
	assertEquals(await count(`SELECT count(*)::int AS n FROM tasks WHERE (s <> '' AND st = s::status)`), 0);
	assertEquals(await count(`SELECT count(*)::int AS n FROM tasks WHERE (s = '' OR st = s::status)`), 1);
	assertEquals(
		await count(`SELECT count(*)::int AS n FROM tasks WHERE ($1 = '' OR st = $1::status)`, ['']),
		1,
	);
	assertEquals(
		await count(`SELECT count(*)::int AS n FROM tasks WHERE ($1 = '' OR st = $1::status)`, [
			'open',
		]),
		1,
	);
	// A NULL left operand decides nothing, so the conversion is reached and refused.
	const err = await db
		.query(`SELECT count(*) FROM tasks WHERE (NULL::bool OR st = s::status)`)
		.then(() => null, (e) => (e as {code?: string}).code);
	assertEquals(err, '22P02');
	await db.close();
});

Deno.test('transaction commit / rollback-on-throw + fork isolation', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	const count = async (d = db) => (await d.query('SELECT count(*)::int AS c FROM t')).rows[0].c;

	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
	});
	assertEquals(await count(), 1);

	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (2)');
		throw new Error('boom');
	}).catch(() => {});
	assertEquals(await count(), 1); // rolled back

	const forked = await db.fork();
	await forked.query('INSERT INTO t VALUES (3)');
	assertEquals(await count(forked), 2); // fork sees committed base + its own write
	assertEquals(await count(), 1); // base isolated from the fork
	await forked.close();
	await db.close();
});

Deno.test('SQLSTATE surfaces as error.code', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT PRIMARY KEY)');
	await db.query('INSERT INTO t VALUES (1)');
	const err = await db.query('INSERT INTO t VALUES (1)').then(() => null, (e) => e);
	assert(err instanceof Error);
	assertEquals((err as {code?: string}).code, '23505'); // unique_violation
	await db.close();
});

Deno.test('a Date.now pinned after a query under the real clock is the time, back or forward', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE ev (id INT PRIMARY KEY, created_at TIMESTAMPTZ DEFAULT now(), day DATE DEFAULT CURRENT_DATE)');
	await db.query('INSERT INTO ev (id) VALUES (0)'); // stamped by the real clock
	const real = Date.now;
	try {
		// What a test runner's fake timer does, to a past date and then a future one.
		for (const [id, pinned, day] of [[1, Date.UTC(2024, 0, 1), '2024-01-01'], [2, Date.UTC(2099, 11, 31), '2099-12-31']] as const) {
			Date.now = () => pinned;
			await db.query('INSERT INTO ev (id) VALUES ($1)', [id]);
			const row = (await db.query('SELECT created_at, day FROM ev WHERE id = $1', [id])).rows[0];
			assertEquals((row.created_at as Date).getTime(), pinned);
			assertEquals(row.day, day);
			assertEquals(((await db.query('SELECT now() AS at')).rows[0].at as Date).getTime(), pinned);
			assertEquals((await db.query('SELECT CURRENT_DATE AS day')).rows[0].day, day);
			// A database made under the pinned clock, and a fork, read the same clock.
			const other = create_pglet_db();
			assertEquals(((await other.query('SELECT now() AS at')).rows[0].at as Date).getTime(), pinned);
			await other.close();
			const fork = await db.fork();
			assertEquals(((await fork.query('SELECT now() AS at')).rows[0].at as Date).getTime(), pinned);
			await fork.close();
			// Statements in the pinned millisecond are still in order.
			await db.query('UPDATE ev SET created_at = now() WHERE id = 0');
			assertEquals((await db.query('SELECT (SELECT created_at FROM ev WHERE id = 0) > (SELECT created_at FROM ev WHERE id = $1) AS later', [id])).rows[0].later, true);
		}
	} finally {
		Date.now = real;
	}
	// The real clock again.
	const before = real();
	const at = ((await db.query('SELECT now() AS at')).rows[0].at as Date).getTime();
	assert(at >= before && at <= real(), `now() answered ${at}, the clock ${before}`);
	await db.close();
});

Deno.test('a frozen Date.now is the time to the millisecond at any read count, across instances and a fork', async () => {
	const db = create_pglet_db();
	const table = `CREATE TABLE ev (id INT PRIMARY KEY, created_at TIMESTAMPTZ DEFAULT now(),
		t3 TIMESTAMPTZ(3) DEFAULT now(), p3 TIMESTAMP(3) DEFAULT now())`;
	await db.query(table);
	const other = create_pglet_db();
	await other.query(table);
	const fork = await db.fork();
	const lanes = [db, other, fork];
	const real = Date.now;
	const pinned = Date.UTC(2024, 0, 1);
	let ms = pinned - 1;
	Date.now = () => ms;
	try {
		await db.query('SELECT now()');
		ms = pinned;
		// Well over a thousand statements in one millisecond of the JS clock, in one module.
		const read: Array<number> = [];
		for (let i = 0; i < 2400; i++) {
			const lane = lanes[i % 3];
			await lane.query('INSERT INTO ev (id) VALUES ($1)', [i]);
			read.push(((await lane.query('SELECT now() AS at')).rows[0].at as Date).getTime());
		}
		// Every `now()` read back as a `Date` is the pinned `Date`, to the last statement.
		assertEquals(read.filter((at) => at !== pinned).length, 0);
		const js_now = new Date(Date.now());
		for (const lane of lanes) {
			const row = (await lane.query(
				`SELECT count(*)::int AS rows,
					sum(CASE WHEN created_at >= $1 AND created_at < $1::timestamptz + INTERVAL '1 millisecond' THEN 1 ELSE 0 END)::int AS inside
				FROM ev`,
				[js_now],
			)).rows[0];
			assertEquals(row, {rows: 800, inside: 800});
			const decreasing = (await lane.query(
				'SELECT count(*)::int AS n FROM ev x JOIN ev y ON y.id = x.id + 3 WHERE y.created_at < x.created_at',
			)).rows[0].n;
			assertEquals(decreasing, 0);
			// Read back as `Date`s, whatever the column's precision: all rows, each column.
			const rows = (await lane.query('SELECT created_at, t3, p3 FROM ev')).rows;
			for (const column of ['created_at', 't3', 'p3']) {
				assertEquals(rows.filter((r) => (r[column] as Date).getTime() !== pinned).length, 0, column);
			}
			// A precision-3 column against the JS time: equal, and before the next millisecond.
			const rounded = (await lane.query(
				`SELECT sum(CASE WHEN t3 = $1 THEN 1 ELSE 0 END)::int AS equal,
					sum(CASE WHEN t3 < $1::timestamptz + INTERVAL '1 millisecond' THEN 1 ELSE 0 END)::int AS before_next
				FROM ev`,
				[js_now],
			)).rows[0];
			assertEquals(rounded, {equal: 800, before_next: 800});
			assertEquals(((await lane.query('SELECT now()::timestamptz(3) AS at')).rows[0].at as Date).getTime(), pinned);
		}
		// The clock advanced by exactly one millisecond is that millisecond.
		ms = pinned + 1;
		// The first read of it in SQL is the JS time exactly, and a `Date` at every read.
		assertEquals((await db.query('SELECT now() = $1::timestamptz AS exact', [new Date(ms)])).rows[0].exact, true);
		assertEquals(((await other.query('SELECT now() AS at')).rows[0].at as Date).getTime(), pinned + 1);
		// Pinned again to the same instant after the long run: still that instant.
		ms = pinned;
		for (const lane of lanes) {
			assertEquals(((await lane.query('SELECT now() AS at')).rows[0].at as Date).getTime(), pinned);
		}
	} finally {
		Date.now = real;
	}
	await fork.close();
	await other.close();
	await db.close();
});

Deno.test('the now() family is one time per transaction, statement_timestamp() per statement', async () => {
	const db = create_pglet_db();
	await db.query(
		`CREATE TABLE ev (id INT PRIMARY KEY, created_at TIMESTAMPTZ DEFAULT now(),
			updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP, st TIMESTAMPTZ DEFAULT statement_timestamp(), w TIMESTAMPTZ)`,
	);
	// A fresh row's two defaulted columns agree, read back as `Date`s and compared in SQL.
	await db.query('INSERT INTO ev (id, w) VALUES ($1, now())', [1]);
	const fresh = (await db.query('SELECT created_at, updated_at, created_at = w AS same FROM ev')).rows[0];
	assertEquals((fresh.created_at as Date).getTime(), (fresh.updated_at as Date).getTime());
	assertEquals(fresh.same, true);
	assertEquals((await db.query('SELECT now() = now() AS same')).rows[0].same, true);

	// A multi-row INSERT and an UPDATE over every row write one stamp.
	await db.query('INSERT INTO ev (id, w) VALUES (2, now()), (3, now()), (4, now())');
	await db.query('UPDATE ev SET updated_at = now(), w = now()');
	const one = (await db.query(
		`SELECT (SELECT count(DISTINCT updated_at)::int FROM ev) AS updated,
			(SELECT count(DISTINCT w)::int FROM ev) AS w,
			(SELECT count(DISTINCT created_at)::int FROM ev WHERE id > 1) AS created`,
	)).rows[0];
	assertEquals(one, {updated: 1, w: 1, created: 1});

	// The statements of one transaction share its time; each has its own statement time,
	// and the transaction after it has a later one.
	await db.query('DELETE FROM ev');
	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO ev (id) VALUES (1)');
		await tx.query('INSERT INTO ev (id) VALUES ($1)', [2]);
		await tx.query('INSERT INTO ev (id, w) VALUES (3, transaction_timestamp())');
	});
	await db.query('INSERT INTO ev (id) VALUES (4)');
	const block = (await db.query(
		`SELECT (SELECT count(DISTINCT created_at)::int FROM ev WHERE id <= 3) AS times,
			(SELECT count(DISTINCT st)::int FROM ev WHERE id <= 3) AS statement_times,
			(SELECT max(st) FROM ev WHERE id <= 3) < (SELECT created_at FROM ev WHERE id = 4) AS next_is_later,
			(SELECT w = created_at FROM ev WHERE id = 3) AS spelled_out`,
	)).rows[0];
	assertEquals(block, {times: 1, statement_times: 3, next_is_later: true, spelled_out: true});
	// The mirror: clock_timestamp() is the wall clock at each call.
	assertEquals(
		(await db.query('SELECT clock_timestamp() >= now() AS later, clock_timestamp() < clock_timestamp() AS moves')).rows[0],
		{later: true, moves: true},
	);
	await db.close();
});
