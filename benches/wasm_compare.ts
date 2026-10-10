/**
 * Cross-runtime, in-memory WASM comparison: **pglet vs pglite vs sqlite-wasm**.
 *
 * Isolated from storage on purpose — every engine runs in its no-persistence
 * mode (pglet `create_pglet_db()`, pglite ephemeral `new PGlite()`, sqlite
 * `:memory:`) so the numbers reflect engine startup + query CPU, not OPFS /
 * fsync / disk. Runs **headless in Deno, Node, or Bun** (no browser): the same
 * `.ts` file executes unmodified under all three (Deno/Bun native TS; Node ≥23.6
 * strips types). Run it under each runtime to give Deno's numbers cross-engine
 * (V8 vs JavaScriptCore) context.
 *
 * **The `pglet` row is the npm package as a consumer loads it**: the entry
 * `deno task build:npm` assembles (`crates/pglet_wasm/pkg-npm/index.js`, the file
 * each of the three runtimes resolves a bare `@fuzdev/pglet-wasm` import to, by
 * its `node` export condition), `create_pglet_db()`, and `await db.query(sql)`'s
 * `.rows` — the adapter's coerced object rows. The `pglet-raw` row is the same
 * package's raw `PgletBtree` binding (`db.query(sql).rows`), read from the package's
 * internal `pglet_wasm.js` (it is not an export), kept so the binding's own row path stays
 * visible. A process holds one instance
 * of the package's wasm module, so `pglet-raw` runs on the instance the `pglet`
 * row initialized and has no cold init of its own (recorded `null`). Its insert
 * and query figures are measured on a database of its own, but over an instance
 * already warm — memory grown, code compiled — which favours it on the one-shot
 * bulk insert. The package must be current: a missing one, or
 * one older than any file under `crates/`, `scripts/` or the workspace manifests,
 * stops the run before anything is measured.
 *
 * Phases, per engine:
 *   - cold init    : fresh module load → ready DB → first `SELECT 1` (one-shot;
 *                    this is the startup number, where pglite's full-Postgres
 *                    WASM is roughly two orders of magnitude slower than pglet's)
 *   - bulk insert  : load N rows (one-shot; reported total + per-row)
 *   - point lookup : `WHERE id = k` (median over samples; no index — a scan
 *                    filter for every engine, so it stays apples-to-apples)
 *   - full scan    : `SELECT *` returning all N rows (median; includes row
 *                    marshaling, which is what an app actually pays)
 *   - aggregate    : `SELECT count(*), sum(a)` (median)
 *
 * HONEST CAVEAT (printed in output): pglet's WASM is built `opt-level = "z"` +
 * LTO — tuned for *size*, which is its whole thesis, not query speed. Read a
 * slow pglet query number as "the small build is small," not "the engine is
 * slow." pglite is full Postgres compiled to WASM (startup is its weak spot,
 * query is real-PG); sqlite-wasm is the official SQLite build. Single-process,
 * in-memory, rough figures — not a rigorous throughput claim (that's the native
 * `pglet_bench` vs rusqlite lane).
 *
 *   deno task bench:wasm-compare           # build the package, then Deno, Node and Bun
 *   deno task bench:wasm-compare:deno      # Deno
 *   deno task bench:wasm-compare:node      # Node
 *   deno task bench:wasm-compare:bun       # Bun
 *   add --update to merge this runtime's slice into wasm_compare_results.json
 *   (any runtime; a run with a failed engine row exits 1 and writes nothing)
 */
import {readdirSync, statSync} from 'node:fs';
import {readFile} from 'node:fs/promises';
import {summarize, type Summary} from './lib/bench_stats.ts';
import {git_commit, write_json} from './lib/results.ts';

// --- knobs ----------------------------------------------------------------
const ROWS = num_env('PGLET_BENCH_ROWS', 10_000);
const WARMUP = num_env('PGLET_BENCH_WARMUP', 5);
const SAMPLES = num_env('PGLET_BENCH_SAMPLES', 50);
const INSERT_BATCH = 1_000; // rows per multi-VALUES INSERT statement
const TEXT_B = 'abcdefghijklmnopqrstuvwxyz012345'; // 32 chars, matches pglet_bench

function num_env(name: string, fallback: number): number {
	const v = (globalThis as any).process?.env?.[name] ?? get_deno_env(name);
	const n = v == null ? NaN : Number(v);
	return Number.isFinite(n) ? n : fallback;
}
function get_deno_env(name: string): string | undefined {
	const d = (globalThis as any).Deno;
	try {
		return d?.env?.get?.(name);
	} catch {
		return undefined;
	}
}

const {RUNTIME_NAME, RUNTIME_VERSION} = (() => {
	const g = globalThis as any;
	if (typeof g.Deno !== 'undefined') return {RUNTIME_NAME: 'deno', RUNTIME_VERSION: g.Deno.version.deno};
	if (typeof g.Bun !== 'undefined') return {RUNTIME_NAME: 'bun', RUNTIME_VERSION: g.Bun.version};
	return {RUNTIME_NAME: 'node', RUNTIME_VERSION: String(g.process.version).replace(/^v/, '')};
})();
const RUNTIME = `${RUNTIME_NAME}-${RUNTIME_VERSION}`;

// --- the pglet package ----------------------------------------------------
const REPO = new URL('../', import.meta.url);
const PGLET_PKG = new URL('crates/pglet_wasm/pkg-npm/', REPO);
const PGLET_ENTRY = new URL('index.js', PGLET_PKG);
/** The package files a `create_pglet_db()` consumer loads; the oldest of them dates the build. */
const PGLET_PKG_LOADED = ['index.js', 'db.js', 'coerce.js', 'pglet_wasm.js', 'pglet_wasm_bg.wasm'];
/** Directories under `crates/` that hold build output rather than build input: every wasm-pack
 * out-dir (`pkg`, `pkg-npm`, `pkg-opfs`, …) by its prefix, and these. */
const NOT_INPUT_DIRS = new Set(['node_modules', 'target']);
const is_not_input_dir = (name: string): boolean =>
	NOT_INPUT_DIRS.has(name) || name === 'pkg' || name.startsWith('pkg-');

/**
 * The newest file under `dir`, by mtime. Every file counts — a source the package
 * does not depend on only makes the check stricter, which costs a rebuild; a list
 * of the inputs that do count is what would go stale.
 */
function newest_file(dir: URL, newest: {path: string; mtime: number}): void {
	for (const entry of readdirSync(dir, {withFileTypes: true})) {
		if (entry.isDirectory()) {
			if (!is_not_input_dir(entry.name) && !entry.name.startsWith('.')) {
				newest_file(new URL(`${entry.name}/`, dir), newest);
			}
			continue;
		}
		const url = new URL(entry.name, dir);
		const mtime = statSync(url).mtimeMs;
		if (mtime > newest.mtime) {
			newest.path = url.pathname;
			newest.mtime = mtime;
		}
	}
}

/**
 * Refuse a missing or stale npm package rather than measure an old build. Stale is
 * any file under `crates/` or `scripts/`, or a workspace manifest, newer than the
 * oldest package file the bench loads — `deno task build:npm` rebuilds all of them.
 */
function assert_pglet_package_current(): void {
	const rebuild = 'run `deno task build:npm` (the bench:wasm-compare tasks chain it)';
	let built = Infinity;
	for (const name of PGLET_PKG_LOADED) {
		const url = new URL(name, PGLET_PKG);
		let mtime: number;
		try {
			mtime = statSync(url).mtimeMs;
		} catch {
			throw new Error(`the pglet npm package is missing ${url.pathname} — ${rebuild}`);
		}
		built = Math.min(built, mtime);
	}
	const newest = {path: '', mtime: 0};
	newest_file(new URL('crates/', REPO), newest);
	newest_file(new URL('scripts/', REPO), newest);
	for (const name of ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']) {
		const url = new URL(name, REPO);
		const mtime = statSync(url).mtimeMs;
		if (mtime > newest.mtime) {
			newest.path = url.pathname;
			newest.mtime = mtime;
		}
	}
	if (newest.mtime > built) {
		throw new Error(
			`the pglet npm package in ${PGLET_PKG.pathname} is older than ${newest.path} — ${rebuild}`,
		);
	}
}

// --- engine adapters ------------------------------------------------------
// Uniform shape: `exec` runs one statement, `query` returns the row array.
// pglet-raw & sqlite-wasm are synchronous; pglet & pglite are async — all wrapped
// in a Promise so the harness is uniform.
interface Engine {
	exec(sql: string): Promise<void>;
	query(sql: string): Promise<unknown[]>;
}
interface EngineFactory {
	name: string;
	/**
	 * Whether `create` loads a module of its own, so its first query is a cold init.
	 * False for a row that reuses a module an earlier row loaded.
	 */
	cold: boolean;
	create(): Promise<Engine>;
}

const pglet_factory: EngineFactory = {
	name: 'pglet',
	cold: true,
	async create() {
		// the Node entry reads the wasm and runs `initSync` as it loads
		const {create_pglet_db} = await import(PGLET_ENTRY.href);
		const db = create_pglet_db();
		return {
			exec: async (sql) => void (await db.query(sql)),
			query: async (sql) => (await db.query(sql)).rows,
		};
	},
};

const pglet_raw_factory: EngineFactory = {
	name: 'pglet-raw',
	cold: false,
	async create() {
		// the module the `pglet` row loaded — one wasm instance per process, as a consumer has
		const {PgletBtree} = await import(new URL('pglet_wasm.js', PGLET_PKG).href);
		const db = new PgletBtree();
		return {
			exec: async (sql) => void db.exec(sql),
			query: async (sql) => db.query(sql).rows,
		};
	},
};

const pglite_factory: EngineFactory = {
	name: 'pglite',
	cold: true,
	async create() {
		const {PGlite} = await import('@electric-sql/pglite');
		const db = new PGlite(); // ephemeral in-memory; readiness resolves on first query
		return {
			exec: async (sql) => void (await db.exec(sql)),
			query: async (sql) => (await db.query(sql)).rows,
		};
	},
};

const sqlite_factory: EngineFactory = {
	name: 'sqlite-wasm',
	cold: true,
	async create() {
		const init = (await import('@sqlite.org/sqlite-wasm')).default;
		const sqlite3: any = await init();
		const db = new sqlite3.oo1.DB(':memory:');
		return {
			exec: async (sql) => void db.exec(sql),
			query: async (sql) => {
				const rows: unknown[] = [];
				db.exec({sql, rowMode: 'object', resultRows: rows});
				return rows;
			},
		};
	},
};

// `pglet` first: it is the row that loads the package, so the only one whose cold init is cold.
const FACTORIES = [pglet_factory, pglet_raw_factory, pglite_factory, sqlite_factory];

// --- workload -------------------------------------------------------------
// Deterministic pseudo-random column `a` so every engine sees identical data.
function pseudo_a(id: number): number {
	// 32-bit xorshift-ish; values fit INT, deterministic across runtimes.
	let z = (id * 2654435761) >>> 0;
	z ^= z >>> 15;
	z = (z * 2246822519) >>> 0;
	z ^= z >>> 13;
	return z & 0x7fff_ffff;
}

// Fixed point-lookup id stream (same for every engine), derived deterministically.
const LOOKUP_IDS = Array.from({length: SAMPLES}, (_, i) => pseudo_a(i + 1) % ROWS);

async function bulk_insert(eng: Engine): Promise<void> {
	for (let start = 0; start < ROWS; start += INSERT_BATCH) {
		const end = Math.min(start + INSERT_BATCH, ROWS);
		const values: string[] = [];
		for (let id = start; id < end; id++) values.push(`(${id}, ${pseudo_a(id)}, '${TEXT_B}')`);
		await eng.exec(`INSERT INTO bench VALUES ${values.join(',')}`);
	}
}

async function timed(fn: () => Promise<unknown>): Promise<number> {
	const t0 = performance.now();
	await fn();
	return performance.now() - t0;
}

async function sample(fn: () => Promise<unknown>): Promise<number[]> {
	for (let i = 0; i < WARMUP; i++) await fn();
	const out: number[] = [];
	for (let i = 0; i < SAMPLES; i++) out.push(await timed(fn));
	return out;
}

interface EngineResult {
	engine: string;
	/** `null` for a row whose module an earlier row loaded (`EngineFactory.cold`). */
	cold_init_ms: number | null;
	bulk_insert_ms: number;
	bulk_insert_per_row_us: number;
	point_lookup: Summary;
	full_scan: Summary;
	aggregate: Summary;
	error?: string;
}

async function run_engine(factory: EngineFactory): Promise<EngineResult | null> {
	try {
		// cold init: fresh module load → ready DB → first query (the startup number).
		const t0 = performance.now();
		const eng = await factory.create();
		await eng.query('SELECT 1 AS one');
		const cold_init_ms = factory.cold ? performance.now() - t0 : null;

		await eng.exec('CREATE TABLE bench (id INT, a INT, b TEXT)');
		const bulk_insert_ms = await timed(() => bulk_insert(eng));

		// sanity: row count matches what we loaded.
		const count_rows = await eng.query('SELECT count(*) AS n FROM bench');
		const n = Number((count_rows[0] as any)?.n);
		if (n !== ROWS) throw new Error(`row count mismatch: got ${n}, want ${ROWS}`);

		let lookup_i = 0;
		const point_lookup = summarize(
			await sample(() =>
				eng.query(`SELECT * FROM bench WHERE id = ${LOOKUP_IDS[lookup_i++ % LOOKUP_IDS.length]}`),
			),
		);
		const full_scan = summarize(await sample(() => eng.query('SELECT * FROM bench')));
		const aggregate = summarize(
			await sample(() => eng.query('SELECT count(*) AS n, sum(a) AS s FROM bench')),
		);

		return {
			engine: factory.name,
			cold_init_ms,
			bulk_insert_ms,
			bulk_insert_per_row_us: (bulk_insert_ms * 1000) / ROWS,
			point_lookup,
			full_scan,
			aggregate,
		};
	} catch (err) {
		return {
			engine: factory.name,
			cold_init_ms: NaN,
			bulk_insert_ms: NaN,
			bulk_insert_per_row_us: NaN,
			point_lookup: empty_summary(),
			full_scan: empty_summary(),
			aggregate: empty_summary(),
			error: (err as Error)?.message ?? String(err),
		};
	}
}

const empty_summary = (): Summary => ({p50: 0, p95: 0, mean: 0, min: 0, max: 0, cv: 0});

// --- reporting ------------------------------------------------------------
const ms = (n: number | null) => (n != null && Number.isFinite(n) ? n.toFixed(2) : '—');
const pad = (s: string, n: number) => s.padEnd(n);
const padl = (s: string, n: number) => s.padStart(n);

function print_summary_table(title: string, results: EngineResult[], pick: (r: EngineResult) => Summary) {
	console.log(`\n${title}  (ms; ${SAMPLES} samples, +${WARMUP} warmup)`);
	console.log(
		`${pad('engine', 14)}${padl('p50', 10)}${padl('p95', 10)}${padl('mean', 10)}${padl('min', 10)}${padl('max', 10)}${padl('cv', 8)}`,
	);
	console.log('-'.repeat(72));
	for (const r of results) {
		if (r.error) {
			console.log(`${pad(r.engine, 14)}${padl('SKIPPED: ' + r.error, 58)}`);
			continue;
		}
		const s = pick(r);
		console.log(
			`${pad(r.engine, 14)}${padl(ms(s.p50), 10)}${padl(ms(s.p95), 10)}${padl(ms(s.mean), 10)}${padl(ms(s.min), 10)}${padl(ms(s.max), 10)}${padl((s.cv * 100).toFixed(1) + '%', 8)}`,
		);
	}
}

async function dep_version(pkg: string): Promise<string> {
	try {
		const url = new URL(`./node_modules/${pkg}/package.json`, import.meta.url);
		return JSON.parse(new TextDecoder().decode(await readFile(url))).version ?? 'unknown';
	} catch {
		return 'unknown';
	}
}

// --- main -----------------------------------------------------------------
const update = (globalThis as any).Deno?.args?.includes('--update') ?? process.argv.includes('--update');

assert_pglet_package_current();

console.log(`\npglet WASM comparison — in-memory, headless  ·  runtime: ${RUNTIME}`);
console.log(`rows=${ROWS}  ·  row=(id INT, a INT, b TEXT[${TEXT_B.length}])  ·  no indexes (scan-based filters)`);

const results: EngineResult[] = [];
for (const f of FACTORIES) {
	console.error(`  running ${f.name} ...`);
	const r = await run_engine(f);
	if (r) results.push(r);
}

// cold init + bulk insert (one-shot) table
console.log(`\nstartup + bulk load  (one-shot)`);
console.log(
	`${pad('engine', 14)}${padl('cold init ms', 14)}${padl('insert ' + ROWS, 14)}${padl('per-row us', 12)}`,
);
console.log('-'.repeat(54));
for (const r of results) {
	if (r.error) {
		console.log(`${pad(r.engine, 14)}${padl('SKIPPED: ' + r.error, 40)}`);
		continue;
	}
	console.log(
		`${pad(r.engine, 14)}${padl(ms(r.cold_init_ms), 14)}${padl(ms(r.bulk_insert_ms), 14)}${padl(r.bulk_insert_per_row_us.toFixed(2), 12)}`,
	);
}
for (const f of FACTORIES) {
	if (!f.cold) console.log(`(${f.name}: no cold init, and its one-shot insert runs warm — it reuses the module an earlier row loaded)`);
}

print_summary_table('point lookup (WHERE id = k)', results, (r) => r.point_lookup);
print_summary_table('full scan (SELECT *)', results, (r) => r.full_scan);
print_summary_table('aggregate (count + sum)', results, (r) => r.aggregate);

console.log(
	`\nNOTE: pglet is the npm package (pkg-npm/index.js): create_pglet_db().query, the` +
		`\n  adapter's coerced object rows; pglet-raw is its raw PgletBtree binding.` +
		`\n  pglet WASM is opt-level="z"+LTO (size-tuned, not speed-tuned) — a slow` +
		`\n  query figure means "the build is small," not "the engine is slow." pglite is` +
		`\n  full Postgres-in-WASM (startup-heavy); sqlite-wasm is the official build.` +
		`\n  Single-process, in-memory, rough figures — rigorous throughput is the native` +
		`\n  pglet_bench-vs-rusqlite lane.`,
);

const failed = results.filter((r) => r.error);
if (failed.length > 0) {
	console.error(`\nFAILED: ${failed.map((r) => r.engine).join(', ')} — nothing written`);
} else if (update) {
	// This-runtime slice. The recorded JSON keys each runtime (deno/node/bun)
	// under `runtimes`, so a run only refreshes its own slice — `--update` merges
	// into the existing doc rather than clobbering the other runtimes' numbers.
	// Built only here, so a print-only run never spawns git / reads package.json.
	const slice = {
		runtime_version: RUNTIME_VERSION,
		timestamp: new Date().toISOString(),
		git_commit: git_commit(),
		engine_versions: {
			pglite: await dep_version('@electric-sql/pglite'),
			'sqlite-wasm': await dep_version('@sqlite.org/sqlite-wasm'),
		},
		rows: ROWS,
		samples: SAMPLES,
		warmup: WARMUP,
		results,
	};

	const out = new URL('./wasm_compare_results.json', import.meta.url);
	let doc: {schema: string; note: string; runtimes: Record<string, unknown>} = {
		schema: 'wasm_compare',
		note: 'in-memory, single-process, rough figures; pglet wasm is size-tuned (opt-level z+LTO), not speed-tuned. Each runtime keyed separately for cross-runtime (V8 vs JavaScriptCore) context. pglet is the npm package as a consumer loads it (the build:npm entry, create_pglet_db().query, object rows); pglet-raw is the same package\'s raw PgletBtree binding (query().rows, read from its internal pglet_wasm.js), run on the wasm instance the pglet row loaded, so its cold_init_ms is null and its bulk_insert_ms is measured warm: the two rows\' bulk_insert_ms are not comparable.',
		runtimes: {},
	};
	try {
		const existing = JSON.parse(new TextDecoder().decode(await readFile(out)));
		if (existing?.runtimes) doc = {...doc, runtimes: existing.runtimes};
	} catch {
		// no existing file — start fresh
	}
	doc.runtimes[RUNTIME_NAME] = slice;
	await write_json(out, doc);
	console.error(`\nwrote ${out.pathname} (runtime slice: ${RUNTIME_NAME})`);
} else {
	console.error(`\n(results not written — pass --update to refresh wasm_compare_results.json)`);
}

// pglite's wasm teardown sets `process.exitCode = 99` under bun on a *fully
// successful* run (measured: node and deno exit 0, a trivial bun script exits 0,
// and a bare `PGlite.create()` + one query reproduces it with or without
// `close()`). Left alone it makes every green `bench:wasm-compare` run report
// failure to its caller, which is worse than cosmetic: the caller's warning then
// fires unconditionally and a *real* failure is indistinguishable from it.
// Reset it rather than calling `exit()`, which can truncate a piped stdout — to
// the run's own verdict, so a failed engine row still exits 1.
if (typeof process !== 'undefined') process.exitCode = failed.length > 0 ? 1 : 0;
