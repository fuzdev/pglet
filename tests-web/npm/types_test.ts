/**
 * `pg`'s `types` hook honored as `pg` 8.16.3 honors it: each non-NULL cell's **text form** — the
 * text a Postgres text-protocol client receives — through `types.getTypeParser(dataTypeID,
 * 'text')`, asked once per field per result, a NULL left `null` without calling it, and this
 * package's coercion contract not also applied. Every expected text below is what `pg` hands the
 * same parser over PostgreSQL 18.6 for the same query (its session at `TimeZone = 'UTC'`),
 * measured side by side.
 */
import {assertEquals} from '@std/assert';
// deno-lint-ignore no-import-prefix
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';

const Q = `SELECT 9223372036854775807::int8 AS i8, 1.50::numeric AS n,
	'2024-06-15 12:30:45.123456+00'::timestamptz AS tz, '{1,NULL,3}'::int4[] AS ai,
	'{"b": [1, 2.5], "a": "x"}'::jsonb AS j, NULL::int8 AS nul, 7::int4 AS i4, true AS b,
	'\\x00ff'::bytea AS by, '2024-06-15'::date AS d, '1 day 02:00:00'::interval AS iv,
	'{"2024-06-15 12:30:45+00"}'::timestamptz[] AS atz`;

/** A parser that tags each cell with its OID, recording each `getTypeParser` call. */
const tagging = () => {
	const calls: string[] = [];
	const cells: string[] = [];
	return {
		calls,
		cells,
		types: {
			getTypeParser: (oid: number, format?: string) => {
				calls.push(`${oid}/${format}`);
				return (text: string) => (cells.push(text), `${oid}:${text}`);
			},
		},
	};
};

const EXPECTED = {
	i8: '20:9223372036854775807',
	n: '1700:1.50',
	tz: '1184:2024-06-15 12:30:45.123456+00',
	ai: '1007:{1,NULL,3}',
	j: '3802:{"a": "x", "b": [1, 2.5]}',
	nul: null,
	i4: '23:7',
	b: '16:t',
	by: '17:\\x00ff',
	d: '1082:2024-06-15',
	iv: '1186:1 day 02:00:00',
	atz: '1185:{"2024-06-15 12:30:45+00"}',
};

Deno.test('types: every non-NULL cell is its text through getTypeParser(oid, "text"); NULL skips it', async () => {
	const db = create_pglet_db();
	const t = tagging();
	const {rows} = await db.query({text: Q, types: t.types});
	assertEquals(rows, [EXPECTED]);
	// once per field, format 'text', the NULL column's parser asked for but never called
	assertEquals(t.calls, ['20/text', '1700/text', '1184/text', '1007/text', '3802/text', '20/text', '23/text', '16/text', '17/text', '1082/text', '1186/text', '1185/text']);
	assertEquals(t.cells.length, 11);
	await db.close();
});

Deno.test("types with rowMode 'array', and the string form's options", async () => {
	const db = create_pglet_db();
	const arr = await db.query<unknown[]>({text: Q, types: tagging().types, rowMode: 'array'});
	assertEquals(arr.rows, [Object.values(EXPECTED)]);
	const opt = await db.query('SELECT 1::int8 AS x', [], {types: tagging().types});
	assertEquals(opt.rows, [{x: '20:1'}]);
	await db.close();
});

Deno.test('types over no rows still asks for each parser once; without types the coercion contract stands', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (x int8, y text)');
	const t = tagging();
	assertEquals((await db.query({text: 'SELECT x, y FROM t', types: t.types})).rows, []);
	assertEquals(t.calls, ['20/text', '25/text']);
	await db.query("INSERT INTO t VALUES (5, 'a')");
	assertEquals((await db.query({text: 'SELECT x FROM t'})).rows, [{x: 5n}]);
	await db.close();
});

Deno.test("a float's text is Postgres's, and so is the number the coercion reads from it", async () => {
	const db = create_pglet_db();
	// each is a value whose shortest spelling in JS or Rust differs from `float4out`/`float8out`'s
	const q = `SELECT 155.390625::float4 AS tie, (-84197376)::float4 AS wide, 1e20::float8 AS e20,
		1e-10::float8 AS em10, ARRAY[155.390625::float4] AS arr`;
	const t = tagging();
	assertEquals((await db.query({text: q, types: t.types})).rows, [
		{tie: '700:155.39062', wide: '700:-8.4197376e+07', e20: '701:1e+20', em10: '701:1e-10', arr: '1021:{155.39062}'},
	]);
	assertEquals((await db.query(q)).rows, [{tie: 155.39062, wide: -84197376, e20: 1e20, em10: 1e-10, arr: [155.39062]}]);
	await db.close();
});

Deno.test('a repeated column name: object rows keep its last value, array rows keep every value', async () => {
	// each expected row is what `pg` 8.16.3 answers over PostgreSQL 18.6 for the same query
	const db = create_pglet_db();
	const q = 'SELECT 1 AS x, NULL::int AS x, 2 AS y, 3 AS y';
	assertEquals((await db.query({text: q, types: tagging().types})).rows, [{x: null, y: '23:3'}]);
	assertEquals((await db.query<unknown[]>({text: q, types: tagging().types, rowMode: 'array'})).rows, [
		['23:1', null, '23:2', '23:3'],
	]);
	await db.close();
});
