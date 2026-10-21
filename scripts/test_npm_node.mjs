/**
 * Node-runtime smoke test of the built `@fuzdev/pglet-wasm` package.
 *
 * Runs under REAL Node (not Deno) on purpose — it validates the package in the runtime a
 * consumer actually uses: the `"node"` export condition and the top-level `readFileSync` +
 * `initSync` auto-init path in `index.js`. The rich Deno suite (`tests-web/npm/`) proves the
 * coercion/adapter contract, but runs under Deno's node-compat, so it can't prove the real
 * Node module-resolution + init path resolves and runs. This closes that gap — mirroring the
 * sibling wasm packages' `scripts/test_npm.*` pattern (tsv, blake3).
 *
 *   PKG_DIR=crates/pglet_wasm/pkg-npm node --test scripts/test_npm_node.mjs
 */
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {existsSync, readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';

const PKG_DIR = process.env.PKG_DIR ?? 'crates/pglet_wasm/pkg-npm';
const pkg = JSON.parse(readFileSync(resolve(PKG_DIR, 'package.json'), 'utf8'));

// The package declares the Node it supports (`engines`, from `scripts/npm_metadata.ts`), and a
// pass on a Node below that floor would vouch for a runtime the package does not support — so
// this leg refuses to run there instead. The floor is read from the built package, not
// restated, so the two cannot drift.
const engines_floor = /^>=(\d+)$/.exec(pkg.engines?.node ?? '');
if (!engines_floor) {
	console.error(`test_npm_node: package.json engines.node must be ">=<major>", got ${JSON.stringify(pkg.engines)}`);
	process.exit(1);
}
const node_major = Number(process.versions.node.split('.')[0]);
if (node_major < Number(engines_floor[1])) {
	console.error(
		`test_npm_node: Node ${process.versions.node} is below the package's engines floor (${pkg.engines.node}); ` +
			'run this leg on a supported Node',
	);
	process.exit(1);
}
// Importing the built entry runs the `node` condition's readFileSync + initSync at load.
const entry = await import(pathToFileURL(resolve(PKG_DIR, 'index.js')).href);
const {create_pglet_db} = entry;
// The raw binding is the adapter's internal file, not an export: these tests read it directly,
// after the entry's import initialized the one module both share.
const RAW_ENTRY = pathToFileURL(resolve(PKG_DIR, 'pglet_wasm.js')).href;
const {PgletBtree} = await import(RAW_ENTRY);

test('the entry exports the package API and nothing else', () => {
	assert.deepEqual(Object.keys(entry).sort(), ['create_pglet_db', 'init', 'init_sync']);
});

test('package shape: name, ESM, exports map, sideEffects, node10 fields, registry fields, files all present', () => {
	assert.equal(pkg.name, '@fuzdev/pglet-wasm');
	assert.equal(pkg.type, 'module');
	assert.ok(pkg.exports['.'], 'missing "." export');
	assert.equal(pkg.exports['.'].types, './index.d.ts');
	assert.ok(pkg.exports['./kysely'], 'missing "./kysely" subpath export');
	assert.deepEqual(pkg.sideEffects, ['./index.js']); // the tree-shaking-safety marker
	// `main`/`types` for resolvers that predate `exports`, naming the files the `.` export's
	// `node` and `types` conditions name — `exports` stays authoritative wherever it is read
	assert.equal(`./${pkg.main}`, pkg.exports['.'].node);
	assert.equal(`./${pkg.types}`, pkg.exports['.'].types);
	assert.equal(pkg.module, undefined);
	assert.deepEqual(pkg.engines, {node: '>=22'});
	assert.deepEqual(pkg.publishConfig, {access: 'public'});
	for (const f of pkg.files) assert.ok(existsSync(resolve(PKG_DIR, f)), `files[] entry missing: ${f}`);
});

test('the node entry auto-inits; a real query runs and coerces (int8→bigint default, arrays recurse)', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id BIGINT PRIMARY KEY, name TEXT, tags TEXT[])');
	await db.query('INSERT INTO t VALUES ($1, $2, $3)', [1n, 'ada', ['a', 'b']]);
	const {rows} = await db.query('SELECT id, name, tags FROM t');
	assert.equal(rows[0].id, 1n); // int8 → bigint by default (the documented public default)
	assert.equal(rows[0].name, 'ada');
	assert.deepEqual(rows[0].tags, ['a', 'b']);
	await db.close();
});

test('int8:number knob + fork-per-test isolation', async () => {
	const base = create_pglet_db({coercion: {int8: 'number'}});
	await base.query('CREATE TABLE u (id BIGINT PRIMARY KEY)');
	await base.query('INSERT INTO u VALUES (1)');
	const f = await base.fork();
	await f.query('INSERT INTO u VALUES (2)');
	assert.equal((await f.query('SELECT count(*)::int AS c FROM u')).rows[0].c, 2);
	assert.equal((await base.query('SELECT count(*)::int AS c FROM u')).rows[0].c, 1); // base untouched by the fork
	assert.equal((await base.query('SELECT id FROM u')).rows[0].id, 1); // number, not 1n — the knob took
	await base.close();
});

test('the pg result shape survives the real-Node entry: first-word command, tag rowCount, null vs 0', async () => {
	// The Deno suite owns the full field-for-field table; this pins the three discriminating
	// answers in the runtime a consumer actually uses, since the shape is assembled in db.js and
	// the tag crosses the wasm boundary — a difference in either would be invisible under Deno's
	// node-compat alone.
	// `deepStrictEqual`, not `deepEqual`: the latter compares primitives with `==`, under which
	// `null == undefined` — so a build that simply dropped `oid`, or reported `undefined` where
	// `pg` reports `null`, would pass the assertion that exists to catch exactly that.
	const db = create_pglet_db();
	const ddl = await db.query('CREATE TABLE t (id INT, v TEXT)');
	assert.deepStrictEqual([ddl.command, ddl.rowCount, ddl.oid], ['CREATE', null, null]); // NOT 'CREATE TABLE', NOT 0
	const ins = await db.query("INSERT INTO t VALUES (1,'a'),(2,'b')");
	assert.deepStrictEqual([ins.command, ins.rowCount, ins.oid, ins.rows.length], ['INSERT', 2, 0, 0]); // 2 affected, 0 returned
	const upd = await db.query("UPDATE t SET v='z' WHERE v='nope'");
	assert.deepStrictEqual([upd.command, upd.rowCount], ['UPDATE', 0]); // a real 0, distinct from the DDL null
	const sel = await db.query('SELECT * FROM t');
	assert.deepStrictEqual([sel.command, sel.rowCount, sel.rows.length], ['SELECT', 2, 2]); // the over-correction polarity
	await db.close();
});

test('an object row is built as pg 8.16.3 builds it, a __proto__ or constructor column included', async () => {
	// `pg`'s `Result` assigns each field onto a `{}` template, spreads it, and assigns each cell
	// onto a spread of that (`addFields` / `parseRow`); under Node a `__proto__` name then goes
	// through `Object.prototype`'s setter at both steps. Every expectation below is what that
	// algorithm answers for the same fields and cells — which only real Node can show, since
	// Deno defines no `__proto__` accessor.
	const db = create_pglet_db();
	const plain = (await db.query('SELECT 1 AS "__proto__", 2 AS z')).rows[0];
	assert.deepEqual(Object.keys(plain), ['z']);
	assert.equal(Object.getPrototypeOf(plain), Object.prototype);
	assert.equal(typeof plain.hasOwnProperty, 'function');
	const json = (await db.query(`SELECT '{"p":1}'::jsonb AS "__proto__", 2 AS z`)).rows[0];
	assert.deepEqual(Object.keys(json), ['z']);
	assert.deepEqual(Object.getPrototypeOf(json), {p: 1});
	assert.equal(json.p, 1);
	const nul = (await db.query('SELECT NULL::int AS "__proto__", 2 AS z')).rows[0];
	assert.equal(Object.getPrototypeOf(nul), null);
	const ctor = (await db.query('SELECT 1 AS "constructor", 2 AS "hasOwnProperty"')).rows[0];
	assert.deepEqual(Object.keys(ctor), ['constructor', 'hasOwnProperty']);
	assert.equal(Object.getPrototypeOf(ctor), Object.prototype);
	assert.deepEqual([ctor.constructor, ctor.hasOwnProperty], [1, 2]);
	// the `types` branch builds its object rows through the same construction
	const typed = (await db.query({text: 'SELECT 1 AS "__proto__", 2 AS z', types: {getTypeParser: () => String}})).rows[0];
	assert.deepEqual(Object.keys(typed), ['z']);
	assert.equal(Object.getPrototypeOf(typed), Object.prototype);
	await db.close();
});

test('a jsonb value reaches JS as JSON.parse of its text builds it, a __proto__ key an own key', async () => {
	// `pg` parses jsonb with `JSON.parse`, which defines every key as an own data property; under
	// Node a `__proto__` key assigned instead would replace the object's prototype, so a stored
	// `{"__proto__": {"isAdmin": true}}` would read back with `isAdmin` inherited.
	const db = create_pglet_db();
	const text = '{"y": 2, "__proto__": {"isAdmin": true}, "n": [{"__proto__": null}, {"constructor": 1, "toString": 2}]}';
	const {rows} = await db.query(`SELECT '${text}'::jsonb AS j, ARRAY['{"__proto__": 1}'::jsonb] AS a`);
	const [{j, a}] = rows;
	const expected = (await db.query({text: `SELECT '${text}'::jsonb AS j`, types: {getTypeParser: () => JSON.parse}})).rows[0].j;
	assert.deepStrictEqual(j, expected);
	assert.equal(Object.getPrototypeOf(j), Object.prototype);
	assert.equal(j.isAdmin, undefined);
	assert.ok(Object.hasOwn(j, '__proto__'));
	assert.deepStrictEqual(j.__proto__, {isAdmin: true});
	assert.equal(Object.getPrototypeOf(j.n[0]), Object.prototype);
	assert.ok(Object.hasOwn(j.n[0], '__proto__'));
	assert.deepStrictEqual(Object.keys(j.n[1]), ['toString', 'constructor']);
	assert.ok(Object.hasOwn(a[0], '__proto__'));
	assert.equal(Object.getPrototypeOf(a[0]), Object.prototype);
	await db.close();
	// the raw binding's object rows: a `__proto__` column is an own key there too
	const raw = new PgletBtree();
	const raw_row = raw.query('SELECT 1 AS "__proto__", 2 AS z').rows[0];
	assert.deepStrictEqual(Object.keys(raw_row), ['__proto__', 'z']);
	assert.equal(Object.getPrototypeOf(raw_row), Object.prototype);
	raw.free();
});

test('a Date.now pinned to a past date after a query under the real clock is the time now() answers', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE ev (id int, created_at timestamptz DEFAULT now())');
	await db.query('INSERT INTO ev (id) VALUES (0)');
	const real = Date.now;
	const pinned = Date.UTC(2024, 0, 1);
	Date.now = () => pinned;
	try {
		await db.query('INSERT INTO ev (id) VALUES (1)');
		assert.equal((await db.query('SELECT created_at FROM ev WHERE id = 1')).rows[0].created_at.getTime(), pinned);
		assert.equal((await db.query('SELECT now() AS at')).rows[0].at.getTime(), pinned);
		assert.equal((await db.query('SELECT CURRENT_DATE AS day')).rows[0].day, '2024-01-01');
		const other = create_pglet_db();
		assert.equal((await other.query('SELECT now() AS at')).rows[0].at.getTime(), pinned);
		await other.close();
	} finally {
		Date.now = real;
	}
	// The global `Date` replaced whole, which is how a fake-timer library pins the clock.
	const RealDate = globalThis.Date;
	const earlier = RealDate.UTC(2023, 5, 15);
	globalThis.Date = class extends RealDate {
		static now() {
			return earlier;
		}
	};
	try {
		await db.query('INSERT INTO ev (id) VALUES (2)');
		assert.equal((await db.query('SELECT created_at FROM ev WHERE id = 2')).rows[0].created_at.getTime(), earlier);
		assert.equal((await db.query('SELECT CURRENT_DATE AS day')).rows[0].day, '2023-06-15');
	} finally {
		globalThis.Date = RealDate;
	}
	const before = real();
	const at = (await db.query('SELECT now() AS at')).rows[0].at.getTime();
	assert.ok(at >= before && at <= real(), `now() answered ${at}, the clock ${before}`);
	await db.close();
});

// A year of 0 through 99 is the year written, not 1900 through 1999 (what `Date.UTC` and the
// `Date` constructor make of it), under Node's own zone handling: a timestamp is its UTC
// instant in any zone, the `date: 'date'` knob the zone's local midnight.
test('a timestamp or a date in years 0001–0099 reads back as the year written, in any local zone', async () => {
	const db = create_pglet_db();
	const local = create_pglet_db({coercion: {date: 'date'}});
	const prior = process.env.TZ;
	try {
		for (const [tz, midnight] of [['UTC', '00:00:00'], ['America/New_York', '04:56:02'], ['America/Sao_Paulo', '03:06:28']]) {
			process.env.TZ = tz;
			assert.equal(new Intl.DateTimeFormat().resolvedOptions().timeZone, tz);
			const {rows} = await db.query(
				`SELECT '0001-01-01 00:00:00+00'::timestamptz AS first, '0099-12-31 23:59:59.999999'::timestamp AS last,
					ARRAY['0085-11-02 00:00:00+00'::timestamptz] AS list, '0100-01-01 00:00:00+00'::timestamptz AS after`,
			);
			assert.equal(rows[0].first.toISOString(), '0001-01-01T00:00:00.000Z', tz);
			assert.equal(rows[0].last.toISOString(), '0099-12-31T23:59:59.999Z', tz);
			assert.equal(rows[0].list[0].toISOString(), '0085-11-02T00:00:00.000Z', tz);
			assert.equal(rows[0].after.toISOString(), '0100-01-01T00:00:00.000Z', tz);
			// 0085-11-02: São Paulo skipped this midnight in 1985, not in 0085
			for (const day of ['0001-01-01', '0085-11-02', '0099-12-31']) {
				const d = (await local.query(`SELECT '${day}'::date AS d, ARRAY['${day}'::date] AS list`)).rows[0];
				assert.equal(d.d.toISOString(), `${day}T${midnight}.000Z`, `${tz} ${day}`);
				assert.equal(d.list[0].getTime(), d.d.getTime(), `${tz} ${day}`);
				assert.deepEqual([d.d.getFullYear(), d.d.getMonth() + 1, d.d.getDate(), d.d.getHours()], [+day.slice(0, 4), +day.slice(5, 7), +day.slice(8), 0], `${tz} ${day}`);
			}
			const when = new Date('0050-06-15T12:30:45.123Z');
			const bound = (await db.query('SELECT $1::timestamptz AS at, $1::timestamptz::text AS text', [when])).rows[0];
			assert.equal(bound.text, '0050-06-15 12:30:45.123+00', tz);
			assert.equal(bound.at.getTime(), when.getTime(), tz);
		}
	} finally {
		if (prior === undefined) delete process.env.TZ;
		else process.env.TZ = prior;
		await db.close();
		await local.close();
	}
});

test('an outside query() during a transaction outlives its rollback (the audit-log shape)', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	await db.query('CREATE TABLE audit (m TEXT)');
	await db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		db.query("INSERT INTO audit VALUES ('kept')"); // fire-and-forget; queues behind the txn
		throw new Error('boom');
	}).catch(() => undefined);
	await new Promise((r) => setTimeout(r, 20));
	assert.equal((await db.query('SELECT count(*)::int AS c FROM audit')).rows[0].c, 1); // survived
	assert.equal((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 0); // txn reverted
	await db.close();
});

// Node's DEFAULT policy on an unhandled rejection is to crash, and that default is the whole
// observable here — so this runs in child processes with no `process.on('unhandledRejection')`
// listener anywhere. Installing one in THIS process would both suppress the crash it is meant to
// observe and, being process-global and asynchronous, let a rejection from any other test in the
// file be counted as this one's.
const ENTRY = pathToFileURL(resolve(PKG_DIR, 'index.js')).href;
const run_child = (body) =>
	spawnSync(process.execPath, ['--input-type=module', '-e', body], {encoding: 'utf8', timeout: 60_000});

test('an ignored failing query() crashes the process (fire-and-forget stays loud); a handled one does not', () => {
	// The queue's tail handler marks the caller's promise handled, so without `derive_unhandled`
	// an ignored `db.query()` failure reports NOTHING — measured: this exact child exited 0,
	// printed `CHILD_COMPLETED`, and named neither the SQLSTATE nor the table. That is the shape
	// the adapter recommends beside an open transaction (awaiting deadlocks), so the silence
	// would land on the very write the queue exists to save. The pre-queue adapter, where
	// `query()` ran unqueued, exits 1 with `code: '42P01'` — also measured, on a build with the
	// queue removed, which is what makes this a regression the pass introduced rather than an
	// absence it inherited.
	//
	// The `SELECT 1` is load-bearing, not filler: the near-miss repair `tx_chain = result; return
	// result` passes a single-statement version of this and fails with a following entry, because
	// the next `enqueue` chains onto the caller's own promise and launders the rejection.
	const ignored = run_child(
		`const {create_pglet_db} = await import(${JSON.stringify(ENTRY)});
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		db.query('INSERT INTO ff_sentinel_ignored VALUES (1)');
		await db.query('SELECT 1');
		await new Promise((r) => setTimeout(r, 50));
		console.log('CHILD_COMPLETED');`,
	);
	assert.equal(ignored.signal, null, 'the child must crash on its own, not be killed by the timeout');
	assert.notEqual(ignored.status, 0, `expected a non-zero exit; stderr was: ${ignored.stderr}`);
	assert.match(ignored.stderr, /ff_sentinel_ignored/); // it names the statement that failed
	assert.match(ignored.stderr, /42P01/); // ...and carries the SQLSTATE a driver error would
	assert.ok(!ignored.stdout.includes('CHILD_COMPLETED'), 'it must die at the rejection, not run on');

	// The other polarity: a caller that DOES handle must not be double-reported. The tail
	// (`result.then(noop, noop)`) swallows and never rejects, so the returned promise is the only
	// one the host tracks — a repair that re-raised instead would crash this child.
	const handled = run_child(
		`const {create_pglet_db} = await import(${JSON.stringify(ENTRY)});
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id INT)');
		await db.query('INSERT INTO ff_sentinel_catch VALUES (1)').catch(() => undefined);
		try { await db.query('INSERT INTO ff_sentinel_await VALUES (1)'); } catch { /* handled */ }
		await db.transaction(async (tx) => {
			await tx.query('INSERT INTO ff_sentinel_txn VALUES (1)');
		}).catch(() => undefined);
		await db.query('SELECT 1');
		await new Promise((r) => setTimeout(r, 50));
		console.log('CHILD_COMPLETED');`,
	);
	assert.equal(handled.signal, null);
	assert.equal(handled.status, 0, `handled rejections must not crash; stderr was: ${handled.stderr}`);
	assert.ok(handled.stdout.includes('CHILD_COMPLETED'));
	assert.ok(!handled.stderr.includes('ff_sentinel'), `nothing may be reported; stderr was: ${handled.stderr}`);
});

test('a frozen Object.prototype drops no key: every data key is defined, not set', () => {
	// Freezing `Object.prototype` is the usual prototype-pollution hardening; once it is frozen, a
	// plain set of a key naming one of its members (`toString`, `constructor`, …) returns `false`
	// and stores nothing. So each data key is defined. Run in a child, each way a host freezes
	// it, so the test runner's own realm stays unfrozen.
	const body = `const {create_pglet_db} = await import(${JSON.stringify(ENTRY)});
		const {PgletBtree} = await import(${JSON.stringify(RAW_ENTRY)});
		const db = create_pglet_db();
		const doc = '{"toString": 1, "constructor": 2, "valueOf": 3, "hasOwnProperty": 4, "a": 5, "__proto__": 6}';
		const [row] = (await db.query(\`SELECT '\${doc}'::jsonb AS j, '\${doc}'::jsonb::text AS t\`)).rows;
		const raw = new PgletBtree();
		const raw_row = raw.query('SELECT 1 AS "toString", 2 AS "constructor", 3 AS z').rows[0];
		console.log(JSON.stringify({
			frozen: Object.isFrozen(Object.prototype),
			j: Reflect.ownKeys(row.j), parsed: Reflect.ownKeys(JSON.parse(row.t)),
			values: Object.values(row.j), proto: Object.getPrototypeOf(row.j) === Object.prototype,
			raw: Reflect.ownKeys(raw_row), raw_values: Object.values(raw_row),
		}));`;
	for (const args of [['--frozen-intrinsics'], []]) {
		const script = args.length ? body : `Object.freeze(Object.prototype);\n${body}`;
		const child = spawnSync(process.execPath, [...args, '--input-type=module', '-e', script], {
			encoding: 'utf8',
			timeout: 60_000,
		});
		assert.equal(child.status, 0, `${args}: ${child.stderr}`);
		const out = JSON.parse(child.stdout.trim().split('\n').at(-1));
		assert.equal(out.frozen, true, `${args}`);
		assert.deepEqual(out.j, out.parsed, `${args}`);
		assert.deepEqual(out.j, ['a', 'valueOf', 'toString', '__proto__', 'constructor', 'hasOwnProperty']);
		assert.deepEqual(out.values, [5, 3, 1, 6, 2, 4]);
		assert.equal(out.proto, true);
		assert.deepEqual(out.raw, ['toString', 'constructor', 'z']);
		assert.deepEqual(out.raw_values, [1, 2, 3]);
	}
});

/** `promise`, or a rejection naming `what` once `ms` pass, so a deadlock fails rather than hangs. */
const within = (promise, what, ms = 2000) => {
	let timer;
	const timeout = new Promise((_, reject) => {
		timer = setTimeout(() => reject(new Error(`timed out: ${what}`)), ms);
	});
	return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
};

test('concurrent top-level transaction() calls queue on the Node entry, and all land in call order', async () => {
	// Every call after the first is made in a later turn, while an earlier callback is running, from
	// an async context that is not that callback's — the Node entry's AsyncLocalStorage tells it
	// apart from a nested call, so it waits its turn rather than being refused.
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	const calls = [];
	for (let i = 1; i <= 8; i++) {
		calls.push(
			db.transaction(async (tx) => {
				await new Promise((r) => setTimeout(r, 1));
				await tx.query('INSERT INTO t VALUES ($1)', [i]);
				return i;
			}),
		);
		await new Promise((r) => setTimeout(r, 0));
	}
	assert.deepEqual(await within(Promise.all(calls), 'every transaction'), [1, 2, 3, 4, 5, 6, 7, 8]);
	assert.deepEqual((await db.query('SELECT id FROM t ORDER BY id')).rows.map((r) => r.id), [1, 2, 3, 4, 5, 6, 7, 8]);
	await db.close();
});

test('a nested transaction() on the Node entry is refused as nested, never deadlocks', async () => {
	const db = create_pglet_db();
	const err = await within(
		db.transaction(async () => db.transaction(async () => 1)).then(() => null, (e) => e),
		'the outer transaction',
	);
	assert.ok(err instanceof Error);
	assert.match(err.message, /inside a running transaction\(\) callback/);
	assert.match(err.message, /tx handle/);
	await db.close();
});

const STORE_TAX_TEST = 'instances that ran a transaction and closed leave no per-promise cost behind';
// The `AsyncLocalStorage` implementation this process runs. Node 24+ defaults to the
// async-context-frame one and selects the async-hooks one with `--no-async-context-frame`; Node 22
// defaults to async hooks and opts into frames with `--experimental-async-context-frame`. Node
// expands every option it was started with into `execArgv`, the test runner's per-file child
// included, so that is where the choice is read.
const NO_FRAME_FLAG = '--no-async-context-frame';
const on_async_hooks_store = () =>
	process.execArgv.includes(NO_FRAME_FLAG) ||
	(!process.allowedNodeEnvironmentFlags.has(NO_FRAME_FLAG) &&
		!process.execArgv.includes('--experimental-async-context-frame'));

test(STORE_TAX_TEST, async (t) => {
	// The Node entry shares ONE `AsyncLocalStorage` across instances. On the async-hooks
	// implementation (Node 22's default, the `engines` floor) every store ever entered stays
	// registered and is paid for on every promise, so a store per instance made each closed instance
	// a permanent tax on every later `await` in the process. Measured on that implementation with
	// a store per instance, the loop below took ~1.9 s after these instances against ~4 ms before
	// them; with the shared store it stays at the baseline. The bound sits far from both, so neither
	// a loaded machine nor a fast one makes it flaky. The async-context-frame implementation never
	// had the cost, so on it the check proves nothing, and it runs on async hooks or not at all.
	if (!on_async_hooks_store()) {
		if (!process.allowedNodeEnvironmentFlags.has(NO_FRAME_FLAG)) {
			t.skip('this Node has neither the async-hooks store nor the flag to select it');
			return;
		}
		// This one test again, in a child on the async-hooks store, which must report it run and passed.
		const child = spawnSync(
			process.execPath,
			[
				NO_FRAME_FLAG,
				'--test',
				'--test-reporter=tap',
				`--test-name-pattern=^${STORE_TAX_TEST.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`,
				fileURLToPath(import.meta.url),
			],
			// without the runner's marker for a file it spawned, which would make the child's runner
			// refuse to run files as a recursive call
			{encoding: 'utf8', timeout: 120_000, env: {...process.env, NODE_TEST_CONTEXT: undefined}},
		);
		const output = `${child.stdout}\n${child.stderr}`;
		assert.equal(child.error, undefined, output);
		assert.equal(child.status, 0, output);
		assert.match(child.stdout, /^# tests 1$/m, output);
		assert.match(child.stdout, /^# pass 1$/m, output);
		assert.match(child.stdout, /^# skipped 0$/m, output);
		return;
	}
	for (let i = 0; i < 300; i++) {
		const db = create_pglet_db();
		await db.transaction(async (tx) => tx.query('SELECT 1'));
		await db.close();
	}
	const start = performance.now();
	for (let i = 0; i < 100_000; i++) await null;
	const elapsed = performance.now() - start;
	assert.ok(elapsed < 500, `100k awaits took ${elapsed.toFixed(0)} ms after 300 closed instances`);
});

test('overlapping raw BEGIN (the kysely/drizzle path) fails loud (25001), never silently corrupts', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	const a = (async () => {
		await db.query('BEGIN');
		await db.query('INSERT INTO t VALUES (1)');
		await new Promise((r) => setTimeout(r, 5)); // yield mid-transaction
		await db.query('COMMIT');
	})();
	const b = (async () => {
		await db.query('BEGIN'); // overlaps a's open transaction → must reject loudly
		await db.query('INSERT INTO t VALUES (2)');
		await db.query('ROLLBACK');
	})();
	const settled = await Promise.allSettled([a, b]);
	const rejected = settled.filter((s) => s.status === 'rejected');
	assert.equal(rejected.length, 1);
	assert.equal(rejected[0].reason.code, '25001');
	assert.equal((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1); // no lost write
	await db.close();
});

test('the browser entry: concurrent init() loads once, init_sync() is refused while it loads, exports the API', () => {
	// In a child: this process already loaded the module through the Node entry, and the browser
	// entry shares it. Bytes stand in for the URL fetch a bundler build makes.
	const BROWSER = pathToFileURL(resolve(PKG_DIR, 'browser.js')).href;
	const WASM = resolve(PKG_DIR, 'pglet_wasm_bg.wasm');
	const child = spawnSync(
		process.execPath,
		[
			'--input-type=module',
			'-e',
			`import {readFileSync} from 'node:fs';
			const entry = await import(${JSON.stringify(BROWSER)});
			const bytes = readFileSync(${JSON.stringify(WASM)});
			const out = {keys: Object.keys(entry).sort()};
			try { entry.create_pglet_db(); out.before = 'no throw'; } catch (e) { out.before = e.message; }
			const first = entry.init({wasm: bytes});
			const second = entry.init({wasm: bytes});
			out.same = first === second;
			try { entry.init_sync({wasm: bytes}); out.sync = 'no throw'; } catch (e) { out.sync = e.message; }
			await first;
			const db = entry.create_pglet_db();
			await db.query('CREATE TABLE t (id INT)');
			await second;
			await entry.init({wasm: bytes});
			entry.init_sync({wasm: bytes}); // a no-op once initialized
			await db.query('INSERT INTO t VALUES (1)');
			out.rows = (await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c;
			await db.close();
			console.log(JSON.stringify(out));`,
		],
		{encoding: 'utf8', timeout: 60_000},
	);
	assert.equal(child.status, 0, child.stderr);
	const out = JSON.parse(child.stdout.trim().split('\n').at(-1));
	assert.deepEqual(out.keys, ['create_pglet_db', 'init', 'init_sync']);
	assert.match(out.before, /not initialized/);
	assert.equal(out.same, true);
	assert.match(out.sync, /while init\(\) is loading/);
	assert.equal(out.rows, 1);
});
