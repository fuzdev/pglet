// Public type surface for @fuzdev/pglet-wasm. Hand-written (the runtime is dep-free JS);
// the patch script copies this verbatim into the package as `index.d.ts`.

export type Int8Mode = 'bigint' | 'number' | 'string';
export type DateMode = 'string' | 'date';

/**
 * The two contested value-coercion knobs. Everything else follows pg's dominant JS
 * convention. Defaults: `int8: 'bigint'` (zero info loss, and the value the engine hands over
 * with no conversion), `date: 'string'` (TZ-proof).
 */
export interface CoercionOptions {
	int8?: Int8Mode;
	date?: DateMode;
}

/**
 * A column descriptor: `pg.FieldDef`'s keys, carrying what `pg` reads off pglet's native server
 * for the same column (both are built from one descriptor). Measured against `pg` 8.16.3 /
 * PostgreSQL 18.6:
 *
 * - `dataTypeID`, `dataTypeSize` and `format` are Postgres's except where pglet's declared type
 *   differs: an enum column (below), a `char(n)` column (`1043`, where Postgres sends `1042`),
 *   and `min`/`max` over a `varchar` (`1043`, where Postgres sends `25`). `format` is `'text'` on
 *   every field, as `pg` reports it for every query it sends, with or without `types`.
 * - `tableID` / `columnID` name the column's source table and attribute number, `0` / `0` for a
 *   computed column, as on Postgres; the table's OID is pglet's own catalog OID, as Postgres's is
 *   its own database's. A column after a dropped one is numbered by its position here, where
 *   Postgres keeps its original attribute number.
 * - `dataTypeModifier` is Postgres's `atttypmod` for a base column (`varchar(7)` is `11`,
 *   `numeric(6,2)` `393222`) and `-1` for a computed one, where Postgres sends a cast's modifier
 *   (`1.5::numeric(4,1)` is `262149` there).
 *
 * An **enum** column, which pglet stores as text, is declared `text` (`dataTypeID: 25`) where
 * `pg` against 18.6 sends the enum type's own catalog OID, so a driver branching on
 * `dataTypeID` decodes an enum column as text rather than as a custom type.
 */
export interface FieldDef {
	name: string;
	tableID: number;
	columnID: number;
	dataTypeID: number;
	dataTypeSize: number;
	dataTypeModifier: number;
	format: 'text';
}

/**
 * `pg`'s result shape, field for field — parsed from the engine's command tag by node-postgres's
 * own rule. Each tag-derived field is `null`, never `0`/`''`, when the tag carries no part for it,
 * and which tags those are differs per field: `rowCount` on any tag with no count, `oid` on
 * anything but an `INSERT`, and `command` only on an empty tag, which no statement this adapter
 * accepts produces.
 */
export interface QueryResult<T = Record<string, unknown>> {
	rows: T[];
	/**
	 * Rows the statement AFFECTED, read off the command tag — so an `INSERT`/`UPDATE`/`DELETE`
	 * without `RETURNING` reports its count over an empty `rows`. `null` for a tag carrying no
	 * count (`CREATE TABLE`, `BEGIN`, `DO`, `GRANT`, …), which `0` would be indistinguishable from.
	 */
	rowCount: number | null;
	/** One per output column, in order; `[]` for a statement that returns none (DDL, a write without `RETURNING`). */
	fields: FieldDef[];
	/**
	 * The command tag's **first word** — `CREATE` for `CREATE TABLE`, `TRUNCATE` for
	 * `TRUNCATE TABLE`, `ALTER` for `ALTER SEQUENCE`. `null` only for an empty tag.
	 */
	command: string | null;
	/** The `INSERT <oid> <rows>` middle number (`0` on any modern table); `null` otherwise. */
	oid: number | null;
}

export interface QueryOptions {
	/**
	 * `'array'` yields positional rows, every column kept — a repeated column name included — as
	 * `pg`'s are; default `'object'`, whose rows keep a repeated name's last value, as `pg`'s do.
	 */
	rowMode?: 'object' | 'array';
	/** `pg`-style type parsers, honored as in `QueryConfig`. */
	types?: TypeParsers;
}

/**
 * `pg`'s `types` hook: `getTypeParser(oid, 'text')` answers the function a column's cells are
 * parsed with. When a query carries one, each non-NULL cell reaches that function as its
 * **text form** — the text a Postgres text-protocol client receives — exactly as `pg` 8.16.3
 * applies it: asked once per result field with format `'text'`, a NULL cell left `null` without
 * calling it, and this package's coercion contract not applied. Where pglet's text differs from
 * Postgres's (a `char(n)` unpadded, an enum declared `text`, a float under `SET
 * extra_float_digits` below `1`), the parser sees pglet's.
 */
export interface TypeParsers {
	getTypeParser(oid: number, format?: string): (text: string) => unknown;
}

/**
 * `pg`'s `QueryConfig`, the other shape `query` takes — drizzle's `node-postgres` driver passes
 * one. `text` is the SQL and `rowMode` is honored; a truthy second argument to `query` replaces
 * `values`, as `pg` resolves them. `name` is ignored (statements are not named), and `types` is
 * honored as `pg` honors it (`TypeParsers`); without it, every value takes this package's
 * coercion contract.
 *
 * Refused with a `TypeError`, where `pg` would do something this promise-only API cannot: a
 * callback in any position (the second or third argument, or a config's `callback`), a `values`
 * that is not an array (from either place; `null` is no values), and a `pg` `Submittable` (an
 * object with a `submit` method).
 */
export interface QueryConfig {
	text: string;
	values?: unknown[];
	rowMode?: 'object' | 'array';
	name?: string;
	/** `pg`-style type parsers over each cell's text form — see `TypeParsers`. */
	types?: TypeParsers;
}

/**
 * The error a failed statement rejects with: an `Error` carrying `pg`'s `DatabaseError` field
 * names, which an ORM or a test reads (`err.code === '23505'`). A field pglet cannot fill
 * truthfully is absent rather than invented.
 *
 * The adapter's own `25001` refusals (an overlapping `BEGIN`; `fork()` or `close()` while a
 * `transaction()` callback runs) take this shape too. Not every rejection is one: the adapter's
 * other refusals (a malformed `query()` call, a closed `PgletDb`, a value the binding cannot
 * convert) carry no `code`, and neither do the module-wide refusals named `PgletModulePoisoned`
 * and `PgletReentrantCall` (see the README's *Behavior*).
 */
export interface PgletDatabaseError extends Error {
	/** The SQLSTATE, e.g. `'23505'`. */
	code: string;
	severity: 'ERROR';
	detail?: string;
	hint?: string;
	/** A 1-based character offset into the SQL, as a string (as `pg` hands it over). */
	position?: string;
	schema?: string;
	table?: string;
	column?: string;
	constraint?: string;
}

/**
 * The transaction handle: `query` only — the block's own BEGIN/COMMIT/ROLLBACK (or, for a nested
 * one, SAVEPOINT/RELEASE/ROLLBACK TO) are the wrapper's.
 *
 * This `query` runs **inside** the transaction, immediately; it is not `PgletDb.query`, which
 * queues behind the transaction. Once the transaction settles the handle is refused.
 */
export interface PgletTx {
	query<T = Record<string, unknown>>(
		sql: string,
		params?: unknown[],
		options?: QueryOptions,
	): Promise<QueryResult<T>>;
	query<T = Record<string, unknown>>(config: QueryConfig, values?: unknown[]): Promise<QueryResult<T>>;
}

/**
 * The ergonomic adapter — pg.Client-shaped `query` + PGlite-shaped `transaction`, plus
 * `fork`/`close`. Every method runs on the connection's one lane, in call order.
 *
 * **Single-connection contract.** A `PgletDb` drives ONE connection, so at most one *block* is
 * open at a time. A raw `query('BEGIN')` while one is already open **fails loud** with SQLSTATE
 * `25001` rather than silently joining it. For concurrent transactions, `fork()` an instance per
 * lane.
 *
 * **A failed statement aborts the block, as on Postgres.** Every statement after it rejects with
 * `25P02` until the block ends, and the `COMMIT` after it is an implicit rollback (it resolves,
 * reports `command: 'ROLLBACK'`, and publishes nothing). Savepoints work in SQL as on Postgres,
 * `ROLLBACK TO` included. The README's *Behavior* section has the whole contract.
 */
export interface PgletDb {
	/**
	 * Run one statement. It **queues behind an open `transaction()`** (PGlite's per-instance
	 * mutex) and runs once that transaction ends, so an outside write is not lost with somebody
	 * else's rollback. A transaction driven by a raw `query('BEGIN')` (kysely's and drizzle's way)
	 * has no boundary this method can see, so an outside write issued inside one joins it.
	 *
	 * A `transaction()` callback that **awaits** `db.query()` deadlocks — the queue is held by the
	 * transaction it is inside — and the symptom can be a silent exit 0 (the README's *What the
	 * deadlock looks like*). Use the `tx` handle for work that belongs to the transaction. An
	 * ignored rejection still reaches the host as an unhandled rejection.
	 *
	 * It also takes `pg`'s `(config, values)` shape (`QueryConfig`), honoring its `types`;
	 * anything that is neither shape is a `TypeError`.
	 */
	query<T = Record<string, unknown>>(
		sql: string,
		params?: unknown[],
		options?: QueryOptions,
	): Promise<QueryResult<T>>;
	query<T = Record<string, unknown>>(config: QueryConfig, values?: unknown[]): Promise<QueryResult<T>>;
	/**
	 * Run `fn` inside BEGIN/COMMIT (ROLLBACK on throw); resolves to `fn`'s return value. An
	 * outside `query()` serializes behind it.
	 *
	 * **Under an already-open block it is a savepoint** — SAVEPOINT/RELEASE/ROLLBACK TO — so only
	 * its own level unwinds on a throw; a nested level's success merges into the enclosing block,
	 * which can still discard it. That is the rollback-per-test harness idiom.
	 *
	 * A `transaction()` called while another one's callback is running **queues** behind it and
	 * runs as its own block once that one ends. One called from **inside** a running callback
	 * (through awaits, timers and helpers the callback calls) is rejected instead, since it would
	 * wait on its own transaction: use the `tx` handle there. Telling the two apart needs
	 * `AsyncLocalStorage`, which the Node entry (Node, Deno, Bun) has; the browser entry does not,
	 * so there any `transaction()` called while a callback is running is rejected, unless it was
	 * called before that callback began.
	 */
	transaction<T>(fn: (tx: PgletTx) => Promise<T>): Promise<T>;
	/**
	 * A cheap copy-on-write branch of the committed state — the per-test-fixture primitive
	 * (in-memory). It runs on the lane after every statement issued before it, so it carries them.
	 *
	 * Rejects `25001` while a block is open on this instance (a branch of committed state cannot
	 * carry a block's catalog writes over a heap without its rows), and when called from inside a
	 * running `transaction()` callback, where waiting for the lane would deadlock. Called from
	 * outside one it branches after that transaction ends — except on the browser entry, which
	 * cannot tell the two apart and rejects both.
	 */
	fork(): Promise<PgletDb>;
	/**
	 * Free the underlying wasm instance once everything queued before this call has run;
	 * resolves when it is freed. A call made after it (`query`, `transaction`, `fork`) rejects as
	 * closed — except, outside the browser entry, a `query()` made by a `transaction()` callback
	 * that was already running, which is queued and drained before the free, so its
	 * fire-and-forget write lands rather than rejecting unhandled. Rejects `25001` when called
	 * from inside a running `transaction()` callback (on the browser entry, while any callback is
	 * running), as `fork()` does, and leaves the instance as it was; every other call answers the
	 * same promise.
	 */
	close(): Promise<void>;
	// The runtime also sets `[Symbol.asyncDispose]` to `close` where the runtime has the symbol.
	// It is not declared here: the declaration needs TypeScript's `esnext.disposable` lib, and a
	// consumer compiling without it would get an error from this file.
}

export interface CreatePgletDbOptions {
	coercion?: CoercionOptions;
}

/** The wasm a browser-side `init` loads, when not the module next to the entry. */
export interface InitOptions {
	/** A URL or `Response` to fetch it from, its bytes, or a compiled module. */
	wasm?: string | URL | Request | Response | BufferSource | WebAssembly.Module;
}

/** The wasm `init_sync` instantiates: its bytes, or a compiled module. */
export interface InitSyncOptions {
	wasm: BufferSource | WebAssembly.Module;
}

/**
 * Construct a fresh in-memory `PgletDb`. On Node the wasm is initialized at import; in a browser,
 * `await init()` first.
 */
export function create_pglet_db(options?: CreatePgletDbOptions): PgletDb;

/**
 * Initialize the wasm (a browser, or any non-Node runtime the bundler targets). With no `wasm`,
 * the module next to the entry is fetched. Calls made while a load is in flight share it — the
 * first call's `wasm` decides what loads — and once the module is ready a call resolves at once,
 * its argument unused; after a failed load, the next call starts a new one. On Node it is a no-op:
 * the Node entry initializes at import.
 */
export function init(options?: InitOptions): Promise<void>;
/**
 * Initialize the wasm synchronously from its bytes or a compiled module; a no-op once initialized,
 * and refused while an `init()` is in flight. A no-op on Node.
 */
export function init_sync(options: InitSyncOptions): void;
