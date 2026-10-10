/**
 * Two engine calls kysely's `Migrator` makes, on the wasm leg: `pg_advisory_xact_lock`
 * (kysely's `PostgresAdapter.acquireMigrationLock`, issued inside the migration's transaction)
 * and `col_description` (in its introspector's column query). The wasm instance is one connection, so the lock — the writer — is granted every
 * time; what is asserted is the answer each reports: `void` for a taken lock, NULL for a
 * strict call over a NULL, and NULL for a column description, as 18.6 reports them. The
 * `Migrator` as a whole is `kysely_migrator_test.ts`.
 */
import {assertEquals, assertRejects} from '@std/assert';
import {Kysely, PostgresAdapter, sql} from 'kysely';
// deno-lint-ignore no-import-prefix
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';
// @ts-types="../../crates/pglet_wasm/pkg-npm/kysely.d.ts"
import {PgletDialect} from '../../crates/pglet_wasm/pkg-npm/kysely.js';

/** 18.6's `void` type OID, which a taken lock's column is declared as. */
const VOID_OID = 2278;

Deno.test('pg_advisory_xact_lock answers void, in autocommit and inside a transaction', async () => {
	const db = create_pglet_db();
	const auto = await db.query('SELECT pg_advisory_xact_lock(3853314791062309107)');
	assertEquals(auto.rows, [{pg_advisory_xact_lock: ''}]);
	assertEquals(auto.fields.map((f) => f.dataTypeID), [VOID_OID]);
	const inside = await db.transaction(async (tx) => {
		const first = await tx.query('SELECT pg_advisory_xact_lock($1)', [7]);
		// the holder takes it again at once
		const again = await tx.query('SELECT pg_advisory_xact_lock(7, 8) AS k');
		return [first.rows, again.rows];
	});
	assertEquals(inside, [[{pg_advisory_xact_lock: ''}], [{k: ''}]]);
	// strict: a NULL argument answers NULL, not the empty text a taken lock answers
	assertEquals((await db.query('SELECT pg_advisory_xact_lock(NULL) AS k')).rows, [{k: null}]);
	// answered only as a top-level item of a SELECT with no FROM
	await db.query('CREATE TABLE t (a INT)');
	await db.query('INSERT INTO t VALUES (1)');
	await assertRejects(() => db.query('SELECT pg_advisory_xact_lock(a) FROM t'), Error, 'top-level');
	await db.close();
});

Deno.test("kysely's migration lock runs inside a kysely transaction", async () => {
	const pglet = create_pglet_db();
	const db = new Kysely<Record<string, never>>({dialect: new PgletDialect({db: pglet})});
	const answer = await db.transaction().execute(async (trx) => {
		// exactly the call `Migrator` makes: `select pg_advisory_xact_lock(<literal>)`
		await new PostgresAdapter().acquireMigrationLock(trx, {
			lockTable: 'kysely_migration_lock',
			lockRowId: 'migration_lock',
		});
		return (await sql<{n: number}>`SELECT 1 AS n`.execute(trx)).rows;
	});
	assertEquals(answer, [{n: 1}]);
	await db.destroy();
	await pglet.close();
});

Deno.test('col_description is NULL over pg_attribute (no column carries a comment)', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE people (id INT, name TEXT)');
	const {rows, fields} = await db.query(
		`SELECT a.attname, col_description(a.attrelid, a.attnum) AS d
		 FROM pg_catalog.pg_attribute AS a JOIN pg_catalog.pg_class AS c ON a.attrelid = c.oid
		 WHERE c.relname = 'people' AND a.attnum > 0 ORDER BY a.attnum`,
	);
	assertEquals(rows, [{attname: 'id', d: null}, {attname: 'name', d: null}]);
	assertEquals(fields.map((f) => f.dataTypeID), [19, 25]);
	await db.close();
});
