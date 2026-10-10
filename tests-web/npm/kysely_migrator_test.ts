/**
 * kysely's `Migrator` end to end on the wasm leg, unstubbed: its introspector's
 * `getTables` query (`col_description`, `pg_get_serial_sequence` over `quote_ident`-built
 * names, `has_schema_privilege`, `nspname !~ $1`) and its migration lock
 * (`pg_advisory_xact_lock`) run on the engine as kysely issues them.
 */
import {assertEquals} from '@std/assert';
import {Kysely, type Migration, Migrator, sql} from 'kysely';
// deno-lint-ignore no-import-prefix
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';
// @ts-types="../../crates/pglet_wasm/pkg-npm/kysely.d.ts"
import {PgletDialect} from '../../crates/pglet_wasm/pkg-npm/kysely.js';

const init: Migration = {
	up: async (db) => {
		await db.schema.createTable('person').addColumn('id', 'serial', (c) => c.primaryKey())
			.addColumn('name', 'text', (c) => c.notNull()).execute();
		await db.insertInto('person').values({name: 'ada'}).execute();
	},
	down: async (db) => {
		await db.schema.dropTable('person').execute();
	},
};

/** A migration that writes the catalog and a row, then fails. */
const failing: Migration = {
	up: async (db) => {
		await db.schema.createTable('pet').addColumn('id', 'serial', (c) => c.primaryKey()).execute();
		await db.insertInto('person').values({name: 'grace'}).execute();
		await sql`SELECT 1 / 0`.execute(db);
	},
};

const table_names = async (db: Kysely<any>) =>
	(await db.introspection.getTables()).map((t) => t.name).sort();

Deno.test("kysely's Migrator: migrate, no-op rerun, a failing migration rolls back whole, migrateDown", async () => {
	const pglet = create_pglet_db({coercion: {int8: 'number'}});
	const db = new Kysely<any>({dialect: new PgletDialect({db: pglet})});
	let migrations: Record<string, Migration> = {'001_init': init};
	const migrator = new Migrator({db, provider: {getMigrations: async () => migrations}});

	const first = await migrator.migrateToLatest();
	assertEquals(first.error, undefined);
	assertEquals(first.results?.map((r) => [r.migrationName, r.direction, r.status]), [['001_init', 'Up', 'Success']]);
	assertEquals(await table_names(db), ['person']);
	// the introspector reads the serial column as auto-incrementing, through
	// `pg_get_serial_sequence`
	const person = (await db.introspection.getTables()).find((t) => t.name === 'person');
	assertEquals(person?.columns.map((c) => [c.name, c.isAutoIncrementing, c.hasDefaultValue]), [
		['id', true, true],
		['name', false, false],
	]);

	const again = await migrator.migrateToLatest();
	assertEquals([again.error, again.results], [undefined, []]);

	migrations = {'001_init': init, '002_failing': failing};
	const failed = await migrator.migrateToLatest();
	assertEquals((failed.error as {code?: string} | undefined)?.code, '22012');
	assertEquals(failed.results?.map((r) => [r.migrationName, r.status]), [['002_failing', 'Error']]);
	// nothing of 002 survives: not its table, not its row, not its migration record
	assertEquals(await table_names(db), ['person']);
	assertEquals((await db.selectFrom('person').select('name').execute()).map((r) => r.name), ['ada']);
	assertEquals((await db.selectFrom('kysely_migration').select('name').execute()).map((r) => r.name), ['001_init']);

	migrations = {'001_init': init};
	const down = await migrator.migrateDown();
	assertEquals(down.error, undefined);
	assertEquals(down.results?.map((r) => [r.migrationName, r.direction, r.status]), [['001_init', 'Down', 'Success']]);
	assertEquals(await table_names(db), []);
	assertEquals((await db.selectFrom('kysely_migration').select('name').execute()).length, 0);

	await db.destroy();
	await pglet.close();
});
