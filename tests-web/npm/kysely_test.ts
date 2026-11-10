/**
 * kysely on the in-process wasm leg via the `PgletDialect` (the `@fuzdev/pglet-wasm/kysely`
 * subpath). The existing `tests-web/orms/kysely/` covers the wire server; this covers the
 * embedded adapter — CRUD + a transaction through the query builder.
 *
 * The **affected-row counts** are asserted because nothing else here can see them: kysely
 * surfaces them as `numInsertedOrUpdatedRows` / `numUpdatedRows` / `numDeletedRows`, all
 * derived from the driver's `numAffectedRows`, and every one of them is `undefined` — silently,
 * with no error and correct `rows` — if the dialect's mapping is wrong. The suite's only other
 * count comes from a `countAll()` SELECT, which reads `rows` and never touches this path.
 */
import {assert, test} from 'vitest';
import {assert_equal, assert_rejects} from '../lib/assert.ts';
import {CompiledQuery, Kysely, sql} from 'kysely';
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';
// A real consumer resolves types via the `./kysely` export's `types` condition; importing the .js
// directly here, the checker reads the hand-written .d.ts beside it (which `implements Dialect`).
import {PgletDialect} from '../../crates/pglet_wasm/pkg-npm/kysely.js';

interface DB {
	users: {id: number; name: string};
}

test('kysely dialect — CRUD + transaction on the wasm leg', async () => {
	const pglet = create_pglet_db({coercion: {int8: 'number'}});
	const db = new Kysely<DB>({dialect: new PgletDialect({db: pglet})});

	await sql`CREATE TABLE users (id INT PRIMARY KEY, name TEXT)`.execute(db);

	const ins = await db.insertInto('users').values([{id: 1, name: 'ada'}, {id: 2, name: 'grace'}])
		.executeTakeFirst();
	assert_equal(ins.numInsertedOrUpdatedRows, 2n); // the affected-row count, over ZERO returned rows

	const all = await db.selectFrom('users').selectAll().orderBy('id').execute();
	assert_equal(all, [{id: 1, name: 'ada'}, {id: 2, name: 'grace'}]);

	const upd = await db.updateTable('users').set({name: 'ada lovelace'}).where('id', '=', 1)
		.executeTakeFirst();
	assert_equal(upd.numUpdatedRows, 1n);
	const one = await db.selectFrom('users').select('name').where('id', '=', 1).executeTakeFirst();
	assert_equal(one, {name: 'ada lovelace'});
	// A zero here is NOT discriminating and is kept only as an accept-polarity smoke: kysely
	// builds `UpdateResult(result.numAffectedRows ?? 0n)`, so a dialect answering `undefined`
	// is indistinguishable from a real zero at this layer. The non-zero counts above are what
	// catch a broken mapping; the zero is pinned one layer down, in the driver test below.
	const none = await db.updateTable('users').set({name: 'x'}).where('id', '=', 999)
		.executeTakeFirst();
	assert_equal(none.numUpdatedRows, 0n);

	// transaction (driver begin/commit) — commits on success
	await db.transaction().execute(async (trx) => {
		await trx.insertInto('users').values({id: 3, name: 'hopper'}).execute();
	});
	// transaction rolls back on throw
	await db
		.transaction()
		.execute(async (trx) => {
			await trx.insertInto('users').values({id: 4, name: 'nope'}).execute();
			throw new Error('boom');
		})
		.catch(() => {});

	const ids = (await db.selectFrom('users').select('id').orderBy('id').execute()).map((r) => r.id);
	assert_equal(ids, [1, 2, 3]); // 4 rolled back

	const del = await db.deleteFrom('users').where('id', '=', 2).executeTakeFirst();
	assert_equal(del.numDeletedRows, 1n);
	const count = await db.selectFrom('users').select(db.fn.countAll().as('c')).executeTakeFirst();
	assert_equal(Number(count?.c), 2);

	await db.destroy();
	await pglet.close();
});

test('the dialect driver reports numAffectedRows itself: a real 0n for a write, undefined for a SELECT', async () => {
	// Straight at the `DatabaseConnection` the dialect hands kysely, because kysely's own result
	// builders coerce `undefined` to `0n` and so cannot tell a zero count from a missing one.
	const pglet = create_pglet_db({coercion: {int8: 'number'}});
	const driver = new PgletDialect({db: pglet}).createDriver();
	await driver.init();
	const conn = await driver.acquireConnection();
	const run = (sql: string) => conn.executeQuery(CompiledQuery.raw(sql));

	await run('CREATE TABLE u (id INT PRIMARY KEY, name TEXT)');
	assert_equal((await run(`INSERT INTO u VALUES (1,'a'),(2,'b')`)).numAffectedRows, 2n);
	assert_equal((await run(`UPDATE u SET name='z' WHERE id = 999`)).numAffectedRows, 0n); // a real 0n
	assert_equal((await run(`DELETE FROM u WHERE id = 1`)).numAffectedRows, 1n);
	// a SELECT has no affected count on the stock Postgres dialect either — `undefined`, not `0n`
	const sel = await run('SELECT id FROM u');
	assert_equal(sel.numAffectedRows, undefined);
	assert_equal(sel.rows.length, 1);
	// DDL likewise: `command` is `CREATE`, which is not one of the four
	assert_equal((await run('CREATE TABLE v (id INT)')).numAffectedRows, undefined);

	await driver.releaseConnection(conn);
	await driver.destroy();
	await pglet.close();
});

test('kysely transaction settings reach the engine: an isolation level applies, READ ONLY is refused', async () => {
	const pglet = create_pglet_db({coercion: {int8: 'number'}});
	const db = new Kysely<DB>({dialect: new PgletDialect({db: pglet})});
	await sql`CREATE TABLE users (id INT PRIMARY KEY, name TEXT)`.execute(db);
	const level = await db.transaction().setIsolationLevel('serializable').execute(async (trx) => {
		await trx.insertInto('users').values({id: 1, name: 'ada'}).execute();
		return (await sql<{transaction_isolation: string}>`SHOW transaction_isolation`.execute(trx)).rows[0];
	});
	assert_equal(level, {transaction_isolation: 'serializable'}); // the plain `begin` would answer read committed
	assert_equal((await db.selectFrom('users').select('id').execute()).length, 1);
	// An access mode the engine does not model is refused there, not dropped by the dialect.
	const err = await assert_rejects(() =>
		db.transaction().setAccessMode('read only').execute(async (trx) => {
			await trx.insertInto('users').values({id: 2, name: 'nope'}).execute();
		})
	);
	assert_equal((err as {code?: string}).code, '0A000');
	assert_equal((await db.selectFrom('users').select('id').execute()).length, 1);
	await db.destroy();
	await pglet.close();
});
