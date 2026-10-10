// A kysely Dialect backed by one in-process PgletDb (subpath `@fuzdev/pglet-wasm/kysely`).
// The SQL text kysely compiles is transport-independent, so the stock Postgres
// adapter/introspector/compiler are reused verbatim — only the Driver is pglet-specific
// (kysely's dialect-decomposition seam). `kysely` is a peer dep.

import {PostgresAdapter, PostgresIntrospector, PostgresQueryCompiler} from 'kysely';

// The row-count mapping is kysely's OWN, copied from its stock `PostgresDriver`
// (`dialect/postgres/postgres-driver.js` @ 0.28.8) rather than re-derived, so this dialect
// answers `numAffectedRows` exactly as the `pg`-backed one does:
//
//   numAffectedRows: command === 'INSERT' || command === 'UPDATE' ||
//                    command === 'DELETE' || command === 'MERGE' ? BigInt(rowCount) : undefined
//
// It reads `rowCount`, which is why it can be the stock rule: `PgletDb.query` reports `pg`'s
// tag-derived count, and every tag whose first word is one of those four carries one
// (`INSERT <oid> <n>` / `UPDATE <n>` / `DELETE <n>`, with or without `RETURNING`), so the
// `BigInt(null)` the guard is missing is unreachable. What stood here instead was a
// `/(\d+)\s*$/` scrape of `command` — written when `command` was the engine's raw
// `INSERT 0 5` tag and `rowCount` was `rows.length`, i.e. a workaround for the very defect
// that is now fixed. Left in place it would find no digits in a first-word `command` and
// return `undefined` for every write, silently, across the whole dialect.
const affected = (command, rowCount) =>
	command === 'INSERT' || command === 'UPDATE' || command === 'DELETE' || command === 'MERGE'
		? BigInt(rowCount)
		: undefined;

class PgletConnection {
	/** @param {import('./index.d.ts').PgletDb} db */
	constructor(db) {
		this._db = db;
	}
	async executeQuery(compiledQuery) {
		const {rows, command, rowCount} = await this._db.query(
			compiledQuery.sql,
			[...compiledQuery.parameters],
		);
		return {rows, numAffectedRows: affected(command, rowCount)};
	}
	// deno-lint-ignore require-yield
	async *streamQuery() {
		throw new Error('@fuzdev/pglet-wasm: kysely streamQuery is not supported (single in-process connection)');
	}
}

// Transactions are the stock Postgres driver's shape — raw `BEGIN`/`COMMIT`/`ROLLBACK` through
// the same `query()` the statements use — and one consequence is worth stating where a reader
// will look for it: `PgletDb.query`'s queue-behind-an-open-transaction protection is
// `PgletDb.transaction()`'s, and these are not that. To the adapter, a statement kysely runs
// inside this transaction and one the surrounding program runs outside it are the same call on
// the same lane, with no boundary to queue against — so an outside write issued while a kysely
// transaction is open joins it and is lost with its `ROLLBACK`. `fork()` a separate `PgletDb`
// for work that must survive. Recorded in `EDGES.md`.
//
// TODO: kysely's optional savepoint hooks (`savepoint` / `rollbackToSavepoint` /
// `releaseSavepoint`) — the engine has savepoints; and holding the lane across a kysely
// transaction (begin/commit/rollback are explicit callbacks here, so this driver could keep
// an outside write out of it, as `PgletDb.transaction()` does).
class PgletDriver {
	/** @param {import('./index.d.ts').PgletDb} db */
	constructor(db) {
		this._conn = new PgletConnection(db);
	}
	async init() {}
	async acquireConnection() {
		return this._conn;
	}
	// kysely's stock `PostgresDriver.beginTransaction` (0.28.8), so its `TransactionSettings` reach
	// the engine as they reach Postgres: an isolation level is applied, and an access mode the
	// engine does not model (`read only`) is refused there (`0A000`) rather than dropped here.
	async beginTransaction(conn, settings) {
		if (settings?.isolationLevel || settings?.accessMode) {
			let sql = 'start transaction';
			if (settings.isolationLevel) sql += ` isolation level ${settings.isolationLevel}`;
			if (settings.accessMode) sql += ` ${settings.accessMode}`;
			await conn._db.query(sql);
		} else {
			await conn._db.query('begin');
		}
	}
	async commitTransaction(conn) {
		await conn._db.query('commit');
	}
	async rollbackTransaction(conn) {
		await conn._db.query('rollback');
	}
	async releaseConnection() {}
	async destroy() {}
}

/**
 * A kysely `Dialect` over one `PgletDb`. pglet is single-connection, so the one connection is
 * shared (kysely's per-instance mutex serializes access).
 */
export class PgletDialect {
	/** @param {import('./kysely.d.ts').PgletDialectConfig} config */
	constructor(config) {
		if (!config || typeof config.db?.query !== 'function') {
			throw new TypeError('@fuzdev/pglet-wasm: PgletDialect takes {db}, a PgletDb');
		}
		this._db = config.db;
	}
	createAdapter() {
		return new PostgresAdapter();
	}
	createIntrospector(db) {
		return new PostgresIntrospector(db);
	}
	createQueryCompiler() {
		return new PostgresQueryCompiler();
	}
	createDriver() {
		return new PgletDriver(this._db);
	}
}
