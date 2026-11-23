/**
 * The engine calls kysely's `Migrator` makes, on the wasm leg. kysely 0.28's
 * `PostgresAdapter.acquireMigrationLock` sends, on the connection the migration runs on and
 * outside its transaction, `with set_timeout as (select set_config('lock_timeout',
 * '3600000', true) as config_val) select pg_advisory_lock(3853314791062309107) from
 * set_timeout`, and its `releaseMigrationLock` `select pg_advisory_unlock(…)` after the
 * transaction; its introspector's column query reads `col_description`. The wasm instance
 * is one connection, so a lock — the writer — is granted every time; what is asserted is
 * the answer each reports, as 18.6 reports them: `void` for a taken lock, `true` for an
 * unlock of a held one and `false` for one not held, NULL for a strict call over a NULL,
 * and NULL for a column description. The `Migrator` as a whole is
 * `kysely_migrator_test.ts`.
 */
import {assert, test} from 'vitest';
import {assert_equal, assert_rejects} from '../lib/assert.ts';
import {Kysely, PostgresAdapter, sql} from 'kysely';
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';
import {PgletDialect} from '../../crates/pglet_wasm/pkg-npm/kysely.js';

/** 18.6's `void` type OID, which a taken lock's column is declared as. */
const VOID_OID = 2278;
/** `boolean`'s type OID, an unlock's answer. */
const BOOL_OID = 16;

test('pg_advisory_xact_lock answers void, in autocommit and inside a transaction', async () => {
	const db = create_pglet_db();
	const auto = await db.query('SELECT pg_advisory_xact_lock(3853314791062309107)');
	assert_equal(auto.rows, [{pg_advisory_xact_lock: ''}]);
	assert_equal(auto.fields.map((f) => f.dataTypeID), [VOID_OID]);
	const inside = await db.transaction(async (tx) => {
		const first = await tx.query('SELECT pg_advisory_xact_lock($1)', [7]);
		// the holder takes it again at once
		const again = await tx.query('SELECT pg_advisory_xact_lock(7, 8) AS k');
		return [first.rows, again.rows];
	});
	assert_equal(inside, [[{pg_advisory_xact_lock: ''}], [{k: ''}]]);
	// strict: a NULL argument answers NULL, not the empty text a taken lock answers
	assert_equal((await db.query('SELECT pg_advisory_xact_lock(NULL) AS k')).rows, [{k: null}]);
	// answered as a top-level item of the statement's own projection — once per row over a
	// FROM — and refused under a LIMIT, whose rows are not the projection's
	await db.query('CREATE TABLE t (a INT)');
	await db.query('INSERT INTO t VALUES (1), (2)');
	assert_equal((await db.query('SELECT pg_advisory_xact_lock(a) AS k FROM t')).rows, [{k: ''}, {k: ''}]);
	await assert_rejects(() => db.query('SELECT pg_advisory_xact_lock(a) FROM t LIMIT 1'), Error, 'top-level');
	await db.close();
});

test("kysely's migration lock and unlock, as its Migrator issues them around a transaction", async () => {
	const pglet = create_pglet_db();
	const db = new Kysely<Record<string, never>>({dialect: new PgletDialect({db: pglet})});
	const adapter = new PostgresAdapter();
	const options = {lockTable: 'kysely_migration_lock', lockRowId: 'migration_lock'};
	const answer = await db.connection().execute(async (conn) => {
		// exactly the calls `Migrator` makes, the lock and the unlock outside the transaction
		await adapter.acquireMigrationLock(conn, options);
		const inside = await conn.transaction().execute(async (trx) => {
			// the local `lock_timeout` lasted for the lock's statement alone
			const shown = (await sql<{lock_timeout: string}>`SHOW lock_timeout`.execute(trx)).rows;
			return [shown, (await sql<{n: number}>`SELECT 1 AS n`.execute(trx)).rows];
		});
		await adapter.releaseMigrationLock(conn, options);
		return inside;
	});
	assert_equal(answer, [[{lock_timeout: '1s'}], [{n: 1}]]);
	await db.destroy();
	await pglet.close();
});

test('pg_advisory_lock holds stack past a transaction; pg_advisory_unlock gives one back each', async () => {
	const db = create_pglet_db();
	const key = '3853314791062309107';
	const locked = await db.query(`SELECT pg_advisory_lock(${key})`);
	assert_equal(locked.rows, [{pg_advisory_lock: ''}]);
	assert_equal(locked.fields.map((f) => f.dataTypeID), [VOID_OID]);
	await db.transaction(async (tx) => {
		await tx.query('SELECT pg_advisory_lock($1)', [key]);
	});
	const unlock = `SELECT pg_advisory_unlock(${key}) AS released`;
	// two holds: two unlocks answer `true`, the third `false` (18.6 also warns)
	for (const released of [true, true, false]) {
		assert_equal((await db.query(unlock)).rows, [{released}]);
	}
	assert_equal((await db.query(unlock)).fields.map((f) => f.dataTypeID), [BOOL_OID]);
	assert_equal((await db.query('SELECT pg_advisory_unlock(NULL) AS released')).rows, [{released: null}]);
	assert_equal((await db.query('SELECT pg_advisory_unlock_all() AS k')).rows, [{k: ''}]);
	// set_config moves lock_timeout, and answers it as SHOW renders it
	assert_equal((await db.query("SELECT set_config('lock_timeout', '3600000', false) AS v")).rows, [{v: '1h'}]);
	assert_equal((await db.query('SHOW lock_timeout')).rows, [{lock_timeout: '1h'}]);
	// bound: `$2` is inferred `boolean`, as 18.6 infers `is_local`
	assert_equal((await db.query("SELECT set_config('lock_timeout', $1, $2) AS v", ['6s', false])).rows, [{v: '6s'}]);
	await db.close();
});

test('col_description is NULL over pg_attribute (no column carries a comment)', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE people (id INT, name TEXT)');
	const {rows, fields} = await db.query(
		`SELECT a.attname, col_description(a.attrelid, a.attnum) AS d
		 FROM pg_catalog.pg_attribute AS a JOIN pg_catalog.pg_class AS c ON a.attrelid = c.oid
		 WHERE c.relname = 'people' AND a.attnum > 0 ORDER BY a.attnum`,
	);
	assert_equal(rows, [{attname: 'id', d: null}, {attname: 'name', d: null}]);
	assert_equal(fields.map((f) => f.dataTypeID), [19, 25]);
	await db.close();
});
