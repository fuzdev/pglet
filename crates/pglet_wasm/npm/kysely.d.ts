import type {
	Dialect,
	DialectAdapter,
	DatabaseIntrospector,
	Driver,
	Kysely,
	QueryCompiler,
} from 'kysely';
import type {PgletDb} from './index.d.ts';

/** What a `PgletDialect` is built from. */
export interface PgletDialectConfig {
	/** The one in-process connection the dialect drives. */
	db: PgletDb;
}

/**
 * A kysely `Dialect` backed by one in-process `PgletDb`. Reuses kysely's stock Postgres
 * adapter/introspector/compiler (the SQL is transport-independent); only the driver is
 * pglet-specific. `kysely` is a peer dependency.
 *
 * ```ts
 * import {Kysely} from 'kysely';
 * import {create_pglet_db} from '@fuzdev/pglet-wasm';
 * import {PgletDialect} from '@fuzdev/pglet-wasm/kysely';
 * const db = new Kysely({dialect: new PgletDialect({db: create_pglet_db()})});
 * ```
 */
export declare class PgletDialect implements Dialect {
	constructor(config: PgletDialectConfig);
	createAdapter(): DialectAdapter;
	createDriver(): Driver;
	createIntrospector(db: Kysely<unknown>): DatabaseIntrospector;
	createQueryCompiler(): QueryCompiler;
}
