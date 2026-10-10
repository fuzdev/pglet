/**
 * drizzle over its `node-postgres` driver, wrapping a `PgletDb` — the package README's recipe,
 * gated. The driver calls `query({name, text, rowMode, types}, params)`; the adapter takes that
 * `pg` shape and honors its `types` over each cell's text form, as `pg` does. Covered: CRUD, a
 * transaction that commits and one that throws and rolls back, and a bound parameter of each
 * coerced JS type — `bigint`, `Date`, a `'YYYY-MM-DD'` string, a boolean, a `text[]`, a
 * `bigint[]`, a `Date[]`, a JSON value — each read back, and each used as a `WHERE` parameter;
 * and the string-mode timestamps, read back as drizzle over node-postgres reads them.
 */
import {assert, assertEquals, assertInstanceOf, assertRejects} from '@std/assert';
import {and, eq, sql} from 'drizzle-orm';
import {drizzle} from 'drizzle-orm/node-postgres';
import {bigint, boolean, date, integer, jsonb, pgTable, real, text, timestamp} from 'drizzle-orm/pg-core';
// deno-lint-ignore no-import-prefix
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';

const people = pgTable('people', {
	id: integer('id').primaryKey(),
	name: text('name').notNull(),
	big: bigint('big', {mode: 'bigint'}),
	seen: timestamp('seen'),
	seen_tz: timestamp('seen_tz', {withTimezone: true}),
	born: date('born'),
	active: boolean('active'),
	tags: text('tags').array(),
	meta: jsonb('meta'),
	bigs: bigint('bigs', {mode: 'bigint'}).array(),
	seens: timestamp('seens').array(),
	score: real('score'),
});

const DDL = sql`CREATE TABLE people (id INT PRIMARY KEY, name TEXT NOT NULL, big BIGINT, seen TIMESTAMP,
	seen_tz TIMESTAMPTZ, born DATE, active BOOLEAN, tags TEXT[], meta JSONB, bigs BIGINT[], seens TIMESTAMP[],
	score REAL)`;

const ada = {
	id: 1,
	name: 'ada',
	big: 9007199254740993n,
	seen: new Date('2024-06-15T12:30:45.123Z'),
	seen_tz: new Date('2024-06-15T12:30:45.123Z'),
	born: '1815-12-10',
	active: true,
	tags: ['math', 'engines'],
	meta: {notes: [1, 2]},
	bigs: [1n, 9007199254740993n],
	seens: [new Date('2024-06-15T12:30:45.123Z'), new Date('1999-12-31T23:59:59.999Z')],
	score: 1.1,
};

// deno-lint-ignore no-explicit-any
const open = () => drizzle(create_pglet_db() as any);

Deno.test('drizzle over node-postgres: CRUD, and every coerced JS type bound and read back', async () => {
	const db = open();
	await db.execute(DDL);
	await db.insert(people).values([ada, {id: 2, name: 'grace'}]);
	// every value reads back as written — `score`, a `real`, as `1.1` (the number its text reads
	// as), which is what drizzle over node-postgres reads

	const [row] = await db.select().from(people).where(eq(people.id, 1));
	assertEquals(row, ada);
	assertInstanceOf(row.seen, Date);

	// each coerced type as a WHERE parameter
	const hit = async (cond: ReturnType<typeof eq>) => (await db.select({id: people.id}).from(people).where(cond)).map((r) => r.id);
	assertEquals(await hit(eq(people.big, 9007199254740993n)), [1]);
	assertEquals(await hit(eq(people.seen, ada.seen)), [1]);
	assertEquals(await hit(eq(people.seen_tz, ada.seen_tz)), [1]);
	assertEquals(await hit(eq(people.born, '1815-12-10')), [1]);
	assertEquals(await hit(eq(people.active, true)), [1]);
	assertEquals(await hit(eq(people.tags, ['math', 'engines'])), [1]);
	assertEquals(await hit(eq(people.bigs, [1n, 9007199254740993n])), [1]);
	assertEquals(await hit(eq(people.seens, ada.seens)), [1]);
	assertEquals(await hit(sql`${people.meta} @> ${JSON.stringify({notes: [1]})}::jsonb`), [1]);

	const updated = await db.update(people).set({name: 'grace hopper', big: 7n}).where(eq(people.id, 2)).returning({
		id: people.id,
		big: people.big,
	});
	assertEquals(updated, [{id: 2, big: 7n}]);
	await db.delete(people).where(and(eq(people.id, 2), eq(people.name, 'grace hopper')));
	assertEquals((await db.select({id: people.id}).from(people)).map((r) => r.id), [1]);
});

Deno.test('drizzle transactions: a commit keeps its writes, a throw rolls them back', async () => {
	const db = open();
	await db.execute(DDL);
	const kept = await db.transaction(async (tx) => {
		await tx.insert(people).values({id: 10, name: 'kept'});
		return 'done';
	});
	assertEquals(kept, 'done');
	await assertRejects(() =>
		db.transaction(async (tx) => {
			await tx.insert(people).values({id: 11, name: 'lost'});
			throw new Error('boom');
		}), Error, 'boom');
	assertEquals((await db.select({id: people.id}).from(people)).map((r) => r.id), [10]);
});

// drizzle's `date({mode: 'date'})` column sends a `Date` as `toISOString()`
// (`'1815-12-10T00:00:00.000Z'`), which 18.6's date input reads and pglet's refuses
// (`EDGES.md` row 96) — loud, so the README's recipe keeps to the default `date` mode, whose
// value is the `'YYYY-MM-DD'` string the coercion contract uses.
Deno.test("drizzle date({mode: 'date'}) fails loud on pglet (22007), where 18.6 reads the value", async () => {
	const db = open();
	await db.execute(sql`CREATE TABLE days (id INT PRIMARY KEY, d DATE)`);
	const days = pgTable('days', {id: integer('id').primaryKey(), d: date('d', {mode: 'date'})});
	const err = await db.insert(days).values({id: 1, d: new Date('1815-12-10T00:00:00Z')}).then(() => null, (e) => e);
	assert(err instanceof Error);
	assertEquals((err as {cause?: {code?: string}}).cause?.code, '22007');
});

// The string-mode timestamps read back exactly as drizzle over node-postgres reads them off
// 18.6 (its session at `TimeZone = 'UTC'`, which is pglet's), in any process zone: drizzle hands
// these columns `types` parsers that keep the server's text, and the adapter honors `types` with
// each cell's text form. Before it did, the first read `…45.000` and the second named the UTC
// digits under the process's local offset (`…12:30:45.123-04` in a UTC−4 process).
Deno.test('drizzle string-mode timestamps read back the server text, whatever the process zone', async () => {
	const db = open();
	await db.execute(sql`CREATE TABLE m (s TIMESTAMP, z TIMESTAMPTZ)`);
	await db.execute(sql`INSERT INTO m VALUES ('2024-06-15 12:30:45', '2024-06-15 12:30:45.123456+00')`);
	const m = pgTable('m', {
		s: timestamp('s', {mode: 'string'}),
		z: timestamp('z', {mode: 'string', withTimezone: true}),
	});
	const [row] = await db.select().from(m);
	assertEquals(row, {s: '2024-06-15 12:30:45', z: '2024-06-15 12:30:45.123456+00'});
});

// The bulk-upsert idiom over a seed array that repeats a key: one `INSERT … ON CONFLICT DO
// UPDATE` whose second proposal meets the row its first wrote. 18.6 refuses it `21000` and keeps
// nothing it wrote; this package used to apply both proposals and answer as though it had
// inserted two rows.
Deno.test('drizzle onConflictDoUpdate over a repeated key is refused 21000 and writes nothing', async () => {
	const db = open();
	await db.execute(sql`CREATE TABLE kv (k INT PRIMARY KEY, v TEXT)`);
	const kv = pgTable('kv', {k: integer('k').primaryKey(), v: text('v')});
	await db.insert(kv).values({k: 1, v: 'seed'});
	const err = await db
		.insert(kv)
		.values([{k: 1, v: 'a'}, {k: 2, v: 'n'}, {k: 1, v: 'b'}])
		.onConflictDoUpdate({target: kv.k, set: {v: sql`excluded.v`}})
		.then(() => null, (e) => e);
	assertInstanceOf(err, Error);
	assertEquals((err as {cause?: {code?: string}}).cause?.code, '21000');
	assertEquals(await db.select().from(kv), [{k: 1, v: 'seed'}]);
	// distinct keys upsert as before
	await db
		.insert(kv)
		.values([{k: 1, v: 'a'}, {k: 2, v: 'n'}])
		.onConflictDoUpdate({target: kv.k, set: {v: sql`excluded.v`}});
	assertEquals(await db.select().from(kv).orderBy(kv.k), [{k: 1, v: 'a'}, {k: 2, v: 'n'}]);
});
