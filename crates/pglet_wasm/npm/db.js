// The ergonomic pg/PGlite-shaped adapter over a raw `PgletBtree` instance. Gives a
// `{query, transaction, fork, close}` surface — `query(sql, params, {rowMode})`, PGlite's call
// shape — so a consumer's `Db` DI plugs in with zero hand-rolled coercion/transaction glue.
//
// Wasm-agnostic by construction: it takes a `PgletBtree` *instance*, never importing the
// wasm module (blake3's `make_stream_functions(Hasher)` DI pattern), so the generated
// entry (index.js/browser.js) owns wasm init + `new PgletBtree()` and this stays testable.

import {coerce_params, coerce_rows} from './coerce.js';

// The `pg` (node-postgres) `CommandComplete` rule, reproduced from its `lib/result.js`
// rather than approximated — `{command, rowCount, oid}` is the adapter's `pg`-shaped
// promise, and a consumer branching on it (kysely's stock Postgres driver does, on
// `command === 'INSERT' | 'UPDATE' | 'DELETE' | 'MERGE'`) reads a *different* field set
// if this drifts. Measured against pg **8.16.3** — the version this repo pins in `deno.json` —
// and PostgreSQL 18.6. (`pg` 8.20.0's `lib/result.js` is byte-identical to 8.16.3's, diffed, so
// the rule is the same one either version applies; cite the pin, not a version nothing here
// resolves.) Quoted from that file rather than paraphrased:
//
//   const matchRegexp = /^([A-Za-z]+)(?: (\d+))?(?: (\d+))?/
//   if (match) {
//     this.command = match[1]
//     if (match[3]) {                        // COMMAND OID ROWS
//       this.oid = parseInt(match[2], 10)
//       this.rowCount = parseInt(match[3], 10)
//     } else if (match[2]) {                 // COMMAND ROWS
//       this.rowCount = parseInt(match[2], 10)
//     }
//   }
//
// with the constructor's `command = rowCount = oid = null` standing wherever a group does
// not participate. Four consequences, each measured on both engines and none of them a
// detail — the engine's raw tag is `INSERT 0 1` / `SELECT 3` / `CREATE TABLE`, and reporting
// it verbatim (with `rowCount` read off `rows.length`) got all four wrong:
//
//   * `command` is the tag's FIRST WORD, so `CREATE TABLE` → `CREATE`, `TRUNCATE TABLE` →
//     `TRUNCATE`, `ALTER SEQUENCE` → `ALTER`. The divergence was universal, `SELECT` included.
//   * `rowCount` comes from the TAG, never from `rows.length` — that is the whole point for a
//     write without `RETURNING`, where the tag says `UPDATE 3` and `rows` is empty.
//   * a tag carrying no trailing number leaves `rowCount` **null**, not 0: `CREATE TABLE`,
//     `BEGIN`, `DO`, `GRANT` all report `null` on `pg`, and `0` is a legitimate count that a
//     consumer may compare against.
//   * `oid` is a field of the shape (the middle number of `INSERT <oid> <rows>`; `0` on any
//     modern table, `null` for every other command).
//
// `match[N]` is either `undefined` or a `(\d+)` capture, i.e. a non-empty digit string, and
// every non-empty string is truthy — so the `!== undefined` tests below are exactly pg's
// truthiness tests, `INSERT 0 0` included.
const COMMAND_TAG_RE = /^([A-Za-z]+)(?: (\d+))?(?: (\d+))?/;

/**
 * Split an engine command tag into `pg`'s `{command, rowCount, oid}`.
 * @param {string} tag the raw `CommandComplete` tag (`'INSERT 0 2'`, `'SELECT 3'`, `'CREATE TABLE'`)
 */
const parse_command_tag = (tag) => {
	const m = COMMAND_TAG_RE.exec(tag);
	// No match means no leading letter — an empty tag. `pg` leaves all three null there, and
	// so does this: no statement the adapter accepts produces one (a comment-only or empty
	// SQL string is `42601 expected 1 statement, got 0` before it could), but the shape is
	// what is promised, not the reachability argument.
	if (!m) return {command: null, rowCount: null, oid: null};
	if (m[3] !== undefined) return {command: m[1], rowCount: Number(m[3]), oid: Number(m[2])};
	if (m[2] !== undefined) return {command: m[1], rowCount: Number(m[2]), oid: null};
	return {command: m[1], rowCount: null, oid: null};
};

// A rejected promise reaches the host — Node's `unhandledRejection`, the browser's and Deno's
// `unhandledrejection` event — only while NOTHING has handled it (the spec's
// HostPromiseRejectionTracker). `enqueue` below must attach a handler to every entry to keep the
// lane's tail settled, and that attachment is itself what silences the caller's copy: the promise
// a fire-and-forget `db.query(...)` returns is already "handled" by the tail before the caller
// ever declines to handle it.
//
// That silence would land on the exact write the queue exists to save. The shape this adapter
// recommends for an audit/telemetry write beside an open transaction is fire-and-forget (awaiting
// it deadlocks — see `query` below), so without the re-derivation the ordinary failure of the
// ordinary recommended call reports nothing at all: measured on the built package under Node,
// `db.query('INSERT INTO no_such_table …')` with no handler printed nothing and exited **0**,
// where the same ignored call reports `code: '42P01'` on stderr and exits **1** once the caller's
// promise is its own. Correct-or-loud is the engine's rule and the adapter does not get to opt
// out of it at its own boundary.
//
// So the caller is handed a FRESH promise, rejecting with the same reason and handled by nobody.
// Two things about that are load-bearing and both look deletable:
//
//   * it is not an identity `.then` that could be simplified to `return promise` — the new
//     promise OBJECT is the whole mechanism, and collapsing it restores the silence;
//   * the near-miss `tx_chain = result; return result` (drop the tail handler instead of adding
//     this one) is silently wrong too, and worse, because it PASSES the single-statement
//     reproduction: the next `enqueue` does `tx_chain.then(run, run)`, which handles the caller's
//     own promise, so the report disappears as soon as any statement follows. Measured — with a
//     following `db.query('SELECT 1')` that shape exits 0 again.
//
// A caller who DOES attach a handler is unaffected and never double-reports: the tail
// (`result.then(noop, noop)`) swallows and therefore never rejects, which leaves the returned
// promise the only one whose handledness the host is tracking.
/**
 * Re-derive `promise` as one nothing has handled — see the note above. **Not** a no-op.
 * @param {Promise<unknown>} promise
 */
const derive_unhandled = (promise) =>
	promise.then(undefined, (e) => {
		throw e;
	});

/**
 * A thrown value as an `Error`. The binding throws an `Error` carrying the SQLSTATE for every
 * engine failure, but a few of its own refusals (a parameter it cannot convert, a result value
 * it cannot hand over) throw a bare string; this gives those a message and a stack, and no
 * `.code`, since none is known for them.
 * @param {unknown} e
 */
const as_error = (e) => (e instanceof Error ? e : new Error(`@fuzdev/pglet-wasm: ${String(e)}`));

/**
 * Wrap a `PgletBtree` instance as an ergonomic `PgletDb`.
 * @param {any} btree a constructed `PgletBtree` (or `PgletBtree.fork()` result)
 * @param {{coercion?: import('./coerce.js').CoercionOptions}} [options]
 * @returns {import('./index.d.ts').PgletDb}
 */
export const wrap_pglet_db = (btree, options = {}) => {
	const coercion = options.coercion ?? {};
	// Two flags, because `close()` drains the lane before it frees: `closing` refuses new public
	// work from the moment `close()` is called, `freed` is the wasm handle gone. Work queued before
	// the `close()` still runs, and so does the `tx` handle of a transaction already running.
	let closing = false;
	let freed = false;
	/** @type {Promise<void> | null} */
	let close_promise = null;
	const closed_error = () => new Error('@fuzdev/pglet-wasm: this PgletDb is closed');
	// Using a freed wasm handle otherwise surfaces as a cryptic `null pointer passed to rust`;
	// guard with a clear message instead.
	const ensure_open = () => {
		if (freed) throw closed_error();
	};

	// The single connection's transaction state. **`in_tx` is read from the engine, never
	// mirrored** — `btree.transactionDepth()` is `0` idle, `1` inside a `BEGIN … COMMIT`, and
	// one more per live savepoint — because a mirror maintained from command tags cannot be
	// right: `ROLLBACK TO SAVEPOINT` reports tag `ROLLBACK`, exactly as a whole-block rollback
	// does. The mirror this replaces cleared its flag on one and then let the next `BEGIN` open
	// a second block over the still-open first, whose `COMMIT` published the outer block's rows.
	// The same collision is why the binding itself dispatches the lifecycle on a carried signal
	// rather than the tag; reading its count is the adapter's version of that fix.
	//
	// `in_managed_tx`: currently inside a `transaction()` callback (for its distinct
	// nested-call message). `tx_chain` is the connection's single lane — the tail of every
	// `enqueue`d entry, which is every `transaction()` and every public `query()`; the `tx`
	// handle's own `query` is the one thing deliberately outside it.
	let tx_chain = Promise.resolve();
	/**
	 * `query`'s call shapes, resolved as `pg` 8.16.3's `Client.query` resolves them where
	 * pglet can answer the same way, and refused with a `TypeError` naming why where it
	 * cannot — so no shape `pg` accepts resolves to something else here, and none hangs.
	 *
	 * - `(sql, params?, options?)` — the SQL text as a string.
	 * - `(config, values?)` with `config` a pg `QueryConfig` (`{text, values, rowMode, name,
	 *   types}`, the shape drizzle's `node-postgres` driver passes): `text` is the SQL, a truthy
	 *   second argument replaces `config.values` (`pg`'s `normalizeQueryConfig`), `rowMode` is
	 *   honored, `name` is ignored (statements are not named), and `types` is honored as
	 *   `pg` honors it: each cell's **text form** through
	 *   `types.getTypeParser(oid, 'text')` (`parse_text_rows`), in place of the coercion
	 *   contract. A `types` in the string form's options is honored the same way.
	 *
	 * Refused, each a `TypeError`: a `null` / `undefined` query (`pg`'s own refusal); a `pg`
	 * `Submittable` — an object with a `submit` method, which `pg` runs itself and answers
	 * through events (`pg-cursor`, `pg-query-stream`), and which no promise can stand in for; a
	 * callback in any position (`(sql, cb)`, `(config, cb)`, `(config, values, cb)`, a config's
	 * `callback`) — this API is promise-only, and a function read as the parameter list would
	 * bind its arity's worth of `undefined`s; a parameter list that is not an array, from either
	 * place (`pg`'s `Query values must be an array`) — a string's characters would otherwise
	 * bind one by one; and anything else that is not a string or a config with a string
	 * `text`. Each is refused before the wasm: its string copy does not check its argument, and
	 * a non-string traps inside the module's allocator (`RuntimeError: memory access out of
	 * bounds`, measured).
	 */
	const normalize_query_args = (sql_or_config, params, opts) => {
		const refuse = (why) => {
			throw new TypeError(`@fuzdev/pglet-wasm: query() ${why}`);
		};
		if (sql_or_config === null || sql_or_config === undefined) {
			refuse('was passed a null or undefined query');
		}
		if (typeof sql_or_config?.submit === 'function') {
			refuse('does not run a pg Submittable (an object with submit(), as pg-cursor and pg-query-stream pass)');
		}
		if (typeof params === 'function' || typeof opts === 'function' || typeof sql_or_config?.callback === 'function') {
			refuse('takes no callback; await the promise it returns');
		}
		let sql;
		let values;
		let options = opts;
		if (typeof sql_or_config === 'string') {
			sql = sql_or_config;
			values = params;
		} else if (typeof sql_or_config === 'object' && typeof sql_or_config.text === 'string') {
			sql = sql_or_config.text;
			values = params ? params : sql_or_config.values;
			const rowMode = sql_or_config.rowMode ?? opts?.rowMode;
			const types = sql_or_config.types ?? opts?.types;
			if (rowMode || types) options = {...opts, rowMode, types};
		} else {
			refuse(
				'takes the SQL text as a string, or a pg QueryConfig whose `text` is a string; got ' +
					(typeof sql_or_config === 'object' ? 'an object without a string `text`' : typeof sql_or_config),
			);
		}
		if (values !== undefined && values !== null && !Array.isArray(values)) {
			refuse('values must be an array');
		}
		if (options?.types != null && typeof options.types.getTypeParser !== 'function') {
			refuse('types must have a getTypeParser(oid, format) function, as pg\'s do');
		}
		return [sql, values, options];
	};
	/**
	 * Rows of text cells through the caller's `pg`-style `types`, as `pg` 8.16.3's `Result` does
	 * it: one parser per field, `types.getTypeParser(dataTypeID, 'text')`, asked once per result
	 * whether or not it has rows; each non-NULL cell's text through its column's parser, a NULL
	 * left `null` with the parser not called. Positional; `to_object_rows` makes object rows.
	 * The coercion contract does not also run.
	 */
	const parse_text_rows = (rows, fields, types) => {
		const parsers = fields.map((f) => types.getTypeParser(f.dataTypeID, 'text'));
		return rows.map((cells) => cells.map((c, i) => (c === null ? null : parsers[i](c))));
	};
	/**
	 * Positional rows as `pg` 8.16.3's `Result` hands object rows back, built as it builds them:
	 * a template assigned `null` for every field in order, copied once by spread
	 * (`addFields`), and each row a spread of that copy with every cell assigned in field order
	 * (`parseRow`) — so a later field of a repeated name wins, and a column named `__proto__`
	 * goes through `Object.prototype`'s setter exactly where `pg`'s does (under Node: no own
	 * key, and the cell, when an object or `null`, becomes the row's prototype).
	 * `rowMode: 'array'` skips this and keeps every cell, as `pg` does.
	 */
	const to_object_rows = (rows, fields) => {
		const template = {};
		for (const f of fields) template[f.name] = null;
		const prebuilt = {...template};
		return rows.map((cells) => {
			const row = {...prebuilt};
			for (let i = 0; i < cells.length; i++) row[fields[i].name] = cells[i];
			return row;
		});
	};
	let in_managed_tx = false;
	/** Levels open on the connection right now, straight from the engine. */
	const tx_depth = () => btree.transactionDepth();
	/** Distinct savepoint names for the managed nested `transaction()`. */
	let savepoint_seq = 0;

	/**
	 * Synchronous under the hood (pglet does no I/O); the rows arrive positional, are coerced
	 * (or parsed by `types`), and are made objects unless `rowMode` is `'array'`.
	 * A `BEGIN` while a transaction is already open **fails loud** (`25001`): pglet drives one
	 * connection, so an overlapping/nested transaction would corrupt shared state — the
	 * single-connection contract is correct-or-loud, not silently-joined-then-clobbered.
	 * @param {string} sql @param {unknown[]} [params] @param {{rowMode?: 'object'|'array'}} [opts]
	 */
	const run_query = (sql_or_config, params_arg, opts_arg) => {
		const [sql, params, opts] = normalize_query_args(sql_or_config, params_arg, opts_arg);
		const bound = coerce_params(params); // bigint/Date/array param serialization (the input side)
		// Read before the statement runs: the overlap guard below asks whether a block was
		// *already* open, which the depth afterwards can no longer answer (the engine's second
		// `BEGIN` is a no-op, so the count is 1 either way).
		const depth_before = tx_depth();
		// The rows come back positional — every cell of a repeated column name kept — as each
		// cell's text form with `types` (what a text-protocol client receives, for the caller's
		// parsers), otherwise as JS values for the coercion contract.
		const text_rows = opts?.types != null;
		let result;
		try {
			result =
				bound && bound.length
					? btree.queryParams(sql, bound, text_rows, true)
					: btree.query(sql, text_rows, true);
		} catch (e) {
			throw as_error(e);
		}
		// Whether a *second* block was just opened over an open one, which the engine treats as
		// a no-op and this adapter refuses. Two things decide it and neither can be dropped.
		//
		// The **tag**, never a match on the SQL text: a text match cannot track a tokenizer, and
		// `/^\s*begin\b/i` misses `/* c */ BEGIN` and `-- c\nBEGIN`, both of which the engine
		// opens a real transaction for — so the guard never fired and a managed `transaction()`
		// then rolled the caller's in-transaction write back with its own, or committed it and
		// left the caller's later `ROLLBACK` reporting success while reverting nothing.
		//
		// The **depth before**, because after the statement the engine's count is 1 whether this
		// `BEGIN` opened the block or was the redundant second one. It is read from the engine
		// rather than from a flag this file maintains for the reason given where `tx_depth` is
		// defined: `ROLLBACK TO SAVEPOINT` reports tag `ROLLBACK` and a mirror cannot tell it
		// from the whole-block rollback it shares a tag with.
		//
		// Checking after the statement is sound because the engine's second `BEGIN` returns
		// early instead of re-snapshotting the catalog, so nothing has changed when this throws.
		const tag = result.command;
		if (tag === 'BEGIN' && depth_before > 0) {
			const err = new Error(
				'@fuzdev/pglet-wasm: a transaction is already open on this connection. pglet drives a ' +
					'single connection, so an overlapping or nested transaction corrupts shared state — ' +
					'run one transaction at a time, use SAVEPOINT inside the open one, or fork() a ' +
					'separate instance for a concurrent one.',
			);
			// @ts-expect-error augment with the Postgres SQLSTATE, like a real driver error
			err.code = '25001'; // active_sql_transaction ("there is already a transaction in progress")
			// @ts-expect-error the severity an engine error carries
			err.severity = 'ERROR';
			throw err;
		}
		const fields = result.fields;
		let rows = result.rows;
		rows = text_rows ? parse_text_rows(rows, fields, opts.types) : coerce_rows(rows, fields, coercion);
		if (opts?.rowMode !== 'array') rows = to_object_rows(rows, fields);
		// `rowCount`/`oid`/`command` come from the tag, never from `rows` or the binding's own
		// `rowCount` getter (which is `rows.length` — right for the raw binding's contract,
		// wrong for this one: a write without `RETURNING` returns no rows and still affected n).
		const {command, rowCount, oid} = parse_command_tag(tag);
		return {rows, rowCount, oid, fields, command};
	};

	// Every PgletBtree call is synchronous, but `query`/`transaction` return Promises to
	// match the pg/PGlite/`Db` async contract, and `.then` normalizes a synchronous throw
	// (JS `Error` with `.code` = SQLSTATE) into a rejected Promise.

	const noop = () => undefined;
	/**
	 * Put `run` on the single-flight queue and keep the queue's tail settled either way.
	 * The queue is the connection's ONE lane: pglet drives one connection with global
	 * BEGIN/COMMIT/ROLLBACK, so anything that must not land inside somebody else's open
	 * transaction goes through here.
	 * @param {() => unknown} run
	 * @returns {Promise<any>} a promise nothing has handled, so an ignored rejection still
	 * reaches the host — `derive_unhandled`, which is required and not decoration.
	 */
	const enqueue = (run) => {
		const result = tx_chain.then(run, run); // chain regardless of the prior entry's outcome
		tx_chain = result.then(noop, noop); // ...which also marks `result` handled, hence the next line
		return derive_unhandled(result);
	};

	/**
	 * The `tx` handle one `transaction()` callback receives: the IMMEDIATE, unqueued execution. It
	 * must bypass the queue — the transaction is what currently holds it, so chaining here would
	 * deadlock the callback against its own transaction. It works until that transaction
	 * settles and is refused after — called then, it would run outside any transaction and ahead
	 * of the queue. Checked at the call and again when the statement runs.
	 * @returns {{handle: import('./index.d.ts').PgletTx, settle: () => void}}
	 */
	const make_tx_handle = () => {
		let settled = false;
		const ended = () => new Error('@fuzdev/pglet-wasm: this transaction has ended; its tx handle is no longer usable');
		const handle = {
			query: (sql, params, opts) =>
				settled
					? Promise.reject(ended())
					: Promise.resolve().then(() => {
							if (settled) throw ended();
							ensure_open();
							return run_query(sql, params, opts);
						}),
		};
		return {
			handle,
			settle: () => {
				settled = true;
			},
		};
	};

	// The public `query` QUEUES — PGlite's per-instance mutex, and the reason is a lost write,
	// not tidiness. It used to run immediately on the one connection, so a `query()` issued
	// while a `transaction()` was open silently JOINED that transaction and was rolled back
	// with it. Measured: `db.transaction(async (tx) => { await tx.query(<write>);
	// db.query(<audit write>); throw })` left the audit table empty here, where PGlite keeps the
	// audit row (its per-instance mutex — measured on 0.4.5 and 0.5.4) and a `pg` pool keeps it on
	// a second connection — the `auth/audit_log_rollback` shape, whose whole point is that the
	// audit entry outlives the failed transaction. Queued, it runs after the transaction ends and
	// survives.
	//
	// The queue does NOT reach a transaction driven by a raw `query('BEGIN')` — kysely's shipped
	// dialect and drizzle both drive them that way — and nothing in THIS file can make it: the
	// in-transaction statements and the outside one are literally the same `query()` on the same
	// lane, with no boundary to queue against, so an outside write issued between such a `BEGIN`
	// and its `ROLLBACK` still joins the transaction and is lost with it (measured).
	//
	// Two things not to over-read there. It is not a pglet quirk: **PGlite loses it on the
	// identical spelling** (measured on 0.4.5 and 0.5.4) and so does a single `pg.Client`; of the
	// four configurations measured the only one that keeps it is a `pg` POOL, which keeps it by
	// running it on a second connection — the thing a one-connection engine does not have by
	// definition rather than by oversight. PGlite's boundary
	// is `transaction()`'s too, exactly as this adapter's now is (its managed `transaction()` keeps
	// the outside write, measured on both versions). And it is not "no layer can": kysely's driver
	// seam receives explicit begin/commit/rollback callbacks and CAN hold the lane across them
	// using nothing but today's public surface — `db.transaction()` is a lane-holding primitive
	// once a deferred promise converts its callback scope into a controlled one, prototyped and
	// measured against the public package. `transaction()` is the spelling that has the protection
	// today.
	//
	// The cost is PGlite's too, and it is the reason this cannot instead be a loud refusal:
	// a callback that AWAITS an outside `db.query()` deadlocks (the queue is held by the
	// transaction the callback is inside), because nothing at call time can tell that
	// fire-and-forget audit write — the shape being rescued — from an awaited one. Use the
	// `tx` handle for work that belongs to the transaction. What CAN be told apart at call time is
	// only that SOME `transaction()` callback is running — not whose — so a `db.transaction()` made
	// then is rejected below rather than queued, and a concurrent top-level one is rejected with it
	// (see the note above `transaction`).
	//
	// What that deadlock LOOKS like, since the docs owe a consumer the symptom. No runtime warning
	// is armed for it — a timer cannot tell a deadlocked transaction from a slow healthy one
	// holding a fire-and-forget write, which is the recommended shape — so the measured shapes are
	// the whole lookup. Measured on node 24 / deno 2.8 / bun 1.3, and the one that matters most is
	// the quiet one, which is also the ordinary Node entry idiom:
	//
	//   * `main()` called at the bottom of the module, no top-level `await`, no live handle: the
	//     process **exits 0 with no output whatsoever** — measured on node, deno and bun. It does
	//     not hang; the program stops mid-way, every statement after the deadlock goes unrun, and
	//     the shell reads success. A consumer in this shape has no symptom to match against at all.
	//   * a top-level `await`: node warns "Detected unsettled top-level await" naming the
	//     `db.transaction(` line and exits 13; deno errors "Top-level await promise never resolved"
	//     and exits 1; **bun hangs** with no output, with no live handle anywhere in the program.
	//   * a live handle open (a server, a poller): hangs with no output (measured on node).
	//   * under a test runner: `deno test` fails it in ~10 ms with "Promise resolution is still
	//     pending but the event loop has already resolved"; `node --test` cancels it at the test
	//     timeout.
	//
	// Nothing in any of those names pglet as the cause, and the quiet ones name nothing at all —
	// this comment and `PgletDb.query`'s doc are what a consumer has to find. Do NOT re-close this
	// list with an "only": the version this replaces said only a live handle hangs, which bun's
	// top-level await falsifies outright, and it had no entry for the silent exit-0 idiom, which is
	// the shape a consumer is likeliest to be in. State the measured members and stop.
	//
	// The returned promise is `derive_unhandled`'d (see the top of this file): the fire-and-forget
	// call above is a recommendation, so its rejection must still reach the host rather than being
	// swallowed by the queue's own bookkeeping.
	/** @type {import('./index.d.ts').PgletDb['query']} */
	const query = (sql, params, opts) =>
		closing
			? Promise.reject(closed_error())
			: enqueue(() => {
					ensure_open();
					return run_query(sql, params, opts);
				});

	// Single-flight: an outside `query()` SERIALIZES behind an open `transaction()` through the
	// queue above (cheap; the engine is synchronous), matching PGlite's per-instance mutex. The
	// callback's own transaction control routes through `run_query` (unqueued, since this entry
	// already holds the queue), so a raw `BEGIN` that overlaps a managed txn (or vice-versa)
	// hits the loud guard above.
	//
	// A second `transaction()` serializes only when it is called BEFORE the first one's callback
	// begins — in practice the same synchronous turn, since `in_managed_tx` is set as soon as the
	// queued entry runs its `BEGIN`. That is measurably narrower than "overlapping top-level calls
	// serialize": with the first callback already running, a
	// genuinely top-level `db.transaction()` from elsewhere in the program is REJECTED below as
	// nested and its work does not run at all.
	//
	// A plain flag read at call time cannot separate a lexically nested call from a concurrent
	// top-level one; that needs an async-context store (`AsyncLocalStorage`), which browsers do not
	// have. What follows is that the two readings are indistinguishable — NOT that rejecting is the
	// only available resolution. PGlite is the
	// counter-example and it has the same constraints (one connection, browser-targeted, no
	// `AsyncLocalStorage`): measured on 0.4.5 and 0.5.4, two concurrent top-level `transaction()`
	// calls are both FULFILLED with both rows landing, while a genuinely nested one does not settle
	// at all. `query()` one function above takes that same trade inside pglet — it queues, and
	// accepts the deadlock. So this is a choice between two undecidable readings, made this way
	// because the deadlock's symptom set is the bad one: the ordinary `main()`-at-module-bottom
	// entry exits 0 having silently done nothing (the enumeration above `query`). A loud refusal
	// naming both readings beats that, and the message names both rather than assuming the nested
	// one.
	/** @type {import('./index.d.ts').PgletDb['transaction']} */
	const transaction = (fn) => {
		if (closing) return Promise.reject(closed_error());
		// A `db.transaction()` made while another `transaction()` CALLBACK is running would
		// deadlock the single-flight queue, so it is rejected loudly. A concurrent TOP-LEVEL call
		// reaches this same branch (see above) and is rejected with it — a divergence from a `pg`
		// pool, which would give it its own connection, and from PGlite, which serializes both on
		// its one connection (measured).
		//
		// This is **not** the "a block is already open" case, which savepoints made decidable: see
		// the note in `run` below for why that one runs as a savepoint and this one still cannot.
		if (in_managed_tx) {
			return Promise.reject(
				new Error(
					'@fuzdev/pglet-wasm: a transaction() callback is already running on this ' +
						'connection. If this call is inside that callback, use the tx handle it was ' +
						'passed; if it is a concurrent top-level transaction(), start it before the ' +
						'other one begins or fork() a separate instance for it (the two cannot be told ' +
						'apart at call time).',
				),
			);
		}
		const run = async () => {
			// **A `transaction()` under an already-open block becomes a savepoint**, which is the
			// rollback-per-test harness idiom: the harness holds a raw `BEGIN` around each test and
			// discards it afterwards, and the code under test calls `transaction()` normally. Before
			// savepoints existed this threw `25001` and the test could not run at all.
			//
			// The depth is read from the engine rather than from a flag, so it is true of a block
			// opened any way at all — a raw `query('BEGIN')`, a kysely or drizzle driver's own
			// begin, a `/* c */ BEGIN`.
			//
			// **This is not the `in_managed_tx` case above and must not be conflated with it.**
			// That one is rejected because a lexically nested call and a concurrent top-level one
			// are indistinguishable at call time, and turning a *concurrent top-level* transaction
			// into a savepoint inside somebody else's would be a silent wrong: its `RELEASE` would
			// report success while the outer block still decided whether the rows lived. Here no
			// callback is running, so there is no second reading to be wrong about.
			const nested = tx_depth() > 0;
			const savepoint = nested ? `pglet_tx_${(savepoint_seq += 1)}` : null;
			// `BEGIN` throws loud (25001) if a block is somehow already open; the savepoint path
			// cannot collide, since the name carries a counter no user name here reproduces.
			run_query(savepoint ? `SAVEPOINT ${savepoint}` : 'BEGIN');
			in_managed_tx = true;
			const tx = make_tx_handle();
			try {
				const out = await fn(tx.handle);
				// A callback that CATCHES its own statement error and returns normally reaches this
				// `COMMIT` inside an aborted block, where the engine converts it to a rollback (tag
				// `ROLLBACK`) and publishes nothing — so `out` is returned over a transaction that
				// committed no rows, exactly as `pg` behaves. Before the engine tracked the aborted
				// block this committed the surviving writes AND the failed statement's partial ones.
				//
				// The savepoint spelling of that case is `RELEASE`, which an aborted block refuses
				// with `25P02` — so it is preceded by nothing and simply throws, which is right:
				// under an outer block a failed level must not report success.
				run_query(savepoint ? `RELEASE ${savepoint}` : 'COMMIT');
				return out;
			} catch (e) {
				try {
					// Unwind only this level. `ROLLBACK TO` is also the one statement an aborted
					// block admits, so it works whether the callback threw a plain error or left the
					// block aborted — where a savepoint-less `ROLLBACK` here would have discarded
					// the enclosing block's writes too.
					if (savepoint) {
						if (tx_depth() > 1) {
							run_query(`ROLLBACK TO ${savepoint}`);
							// `ROLLBACK TO` keeps the savepoint it names (it is repeatable), so the
							// level is popped explicitly — the same pair a `pg` driver issues. Without
							// it the depth stays raised and every later `transaction()` on this
							// connection nests one level deeper for no reason.
							run_query(`RELEASE ${savepoint}`);
						}
					} else if (tx_depth() > 0) {
						run_query('ROLLBACK');
					}
				} catch {
					// already unwound — the original error is what matters
				}
				throw e;
			} finally {
				tx.settle();
				in_managed_tx = false;
			}
		};
		return enqueue(run);
	};

	/**
	 * An error for a call made while a `transaction()` callback is running, which would wait on the
	 * lane that callback holds. Made inside the callback, awaiting it would deadlock — the failure
	 * `query()`'s docs enumerate; made from outside, it cannot be told apart from that (the same
	 * undecidability `transaction()` refuses on), so both readings are refused and named.
	 * @param {string} what
	 */
	const in_callback_error = (what) => {
		const err = new Error(
			`@fuzdev/pglet-wasm: ${what} was called while a transaction() callback is running. If it ` +
				'is inside that callback, it would wait for the transaction to end; if it is outside ' +
				'it, await the transaction first (the two cannot be told apart at call time).',
		);
		// @ts-expect-error augment with the Postgres SQLSTATE, like a real driver error
		err.code = '25001'; // active_sql_transaction: the callback's block is open
		// @ts-expect-error the severity an engine error carries
		err.severity = 'ERROR';
		return err;
	};

	/** @type {import('./index.d.ts').PgletDb['fork']} */
	const fork = () => {
		if (closing) return Promise.reject(closed_error());
		if (in_managed_tx) return Promise.reject(in_callback_error('fork()'));
		// Otherwise queued, so a fork sees every statement issued before it — `db.query(<write>);
		// await db.fork()` carries the write — and a block opened by a raw `BEGIN` is refused
		// `25001` by the engine (a fork branches committed state).
		return enqueue(() => {
			ensure_open();
			let branch;
			try {
				branch = btree.fork();
			} catch (e) {
				throw as_error(e);
			}
			return wrap_pglet_db(branch, options);
		});
	};

	/** @type {import('./index.d.ts').PgletDb['close']} */
	const close = () => {
		// Checked first: a call made while a callback runs must not be handed a pending drain, which
		// waits on that callback's own transaction — awaited inside it, that never settles.
		if (in_managed_tx) return Promise.reject(in_callback_error('close()'));
		if (close_promise) return close_promise; // idempotent: every other call answers the one drain
		closing = true;
		// Drain the lane, then free: whatever was queued before this call runs first, so
		// `db.query(<write>); await db.close();` keeps the write. The tail never rejects.
		close_promise = tx_chain.then(() => {
			freed = true;
			btree.free();
		});
		return close_promise;
	};

	/** @type {import('./index.d.ts').PgletDb} */
	const db = {query, transaction, fork, close};
	// `await using db = create_pglet_db()` where the runtime has explicit resource management.
	if (typeof Symbol.asyncDispose === 'symbol') db[Symbol.asyncDispose] = close;
	return db;
};
