/**
 * pglet WASM init harness.
 *
 * Times the WASM/embedded startup path — the half of the <100ms thesis that
 * actually matters (the native server spawns in ~1 ms; the WASM module is the
 * heavy path, and where pglite is disqualifying at over a second). Phases:
 *   - cold init      : initSync({module: bytes})  — sync compile + instantiate
 *   - new PgletBtree(): engine/catalog construction (the shipped in-memory class)
 *   - first query    : first exec + query (V8 lazily compiling the functions it reaches)
 *   - full cold load : the three above, end to end (first-result latency)
 *   - cached inst.   : initSync({module: precompiled}) — instantiate only
 * plus a one-shot true-cold `WebAssembly.compile` of the bytes.
 *
 * Caveat: V8's native module cache serves byte-identical input the code it already
 * compiled, functions compiled lazily by an earlier sample's queries included, so with
 * it on every sample after the first is warm. The scripts run Node with
 * `--no-wasm-native-module-cache`, so each sample compiles afresh, as a fresh process
 * does; a hand-run without the flag measures the warm path, and `--update` refuses it.
 * The first compile is also reported on its own as `compile (cold, one-shot)`.
 * Watch-tier: prints + records, never gates.
 *
 * Every phase is timed in this process, a plain Node one with no loader of gro's: each
 * sample's module import happens before its clock starts.
 *
 *   npm run bench:wasm-init
 */
import {
	stats_cv,
	stats_mean,
	stats_median,
	stats_min_max,
	stats_percentile,
	stats_std_dev,
} from '@fuzdev/fuz_util/stats.ts';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {gzip_size} from './lib/gzip_size.ts';
import {git_commit, write_json} from './lib/results.ts';

const PKG = new URL('../crates/pglet_wasm/pkg/', import.meta.url);
const JS = new URL('pglet_wasm.js', PKG).href;
const WASM_PATH = new URL('pglet_wasm_bg.wasm', PKG);
const RESULTS = new URL('./wasm_init_results.json', import.meta.url);
const WARMUP = 3;
const SAMPLES = 25;
const TARGET_MS = 100;
/** The V8 flag that keeps the cold phases cold (see the caveat). */
const NO_MODULE_CACHE = '--no-wasm-native-module-cache';

const update = process.argv.includes('--update');
if (update && !process.execArgv.includes(NO_MODULE_CACHE)) {
	// a recorded run without it would file warm figures under the cold labels
	console.error(
		`error: --update needs \`node ${NO_MODULE_CACHE}\` (npm run bench:wasm-init:update)`,
	);
	process.exit(1);
}

const bytes = new Uint8Array(await readFile(WASM_PATH));

// init/initSync are idempotent per imported module instance, so each sample
// re-imports with a unique query string to get a fresh, uninitialized binding.
const fresh = (tag: string, n: number): Promise<any> => import(`${JS}?${tag}${n}`);

// genuine cold compile: the first compile of these bytes in this process.
const cc0 = performance.now();
const cached_module = await WebAssembly.compile(bytes);
const cold_compile_ms = performance.now() - cc0;

interface Phases {
	ci: number; // compile + instantiate
	np: number; // new PgletBtree()
	fq: number; // first query
	total: number;
}

const cold_sample = async (n: number): Promise<Phases> => {
	const m = await fresh('cold', n);
	const t0 = performance.now();
	m.initSync({module: bytes}); // sync compile + instantiate
	const t1 = performance.now();
	const db = new m.PgletBtree();
	const t2 = performance.now();
	db.exec('CREATE TABLE t (id INT)');
	const r = db.query('SELECT 1 AS one');
	const t3 = performance.now();
	if (r.rowCount !== 1) throw new Error('wasm sanity check failed');
	return {ci: t1 - t0, np: t2 - t1, fq: t3 - t2, total: t3 - t0};
};

const cached_instantiate_sample = async (n: number): Promise<number> => {
	const m = await fresh('cached', n);
	const t0 = performance.now();
	m.initSync({module: cached_module}); // instantiate only — no recompile
	return performance.now() - t0;
};

const cold: Phases[] = [];
for (let i = 0; i < WARMUP + SAMPLES; i++) {
	const p = await cold_sample(i);
	if (i >= WARMUP) cold.push(p);
}
const cached: number[] = [];
for (let i = 0; i < WARMUP + SAMPLES; i++) {
	const t = await cached_instantiate_sample(i);
	if (i >= WARMUP) cached.push(t);
}

interface Summary {
	scenario: string;
	p50: number;
	p95: number;
	mean: number;
	min: number;
	max: number;
	cv: number;
}

const summarize = (scenario: string, samples: number[]): Summary => {
	const mean = stats_mean(samples);
	const {min, max} = stats_min_max(samples);
	return {
		scenario,
		p50: stats_median(samples),
		p95: stats_percentile(samples, 0.95),
		mean,
		min,
		max,
		cv: stats_cv(mean, stats_std_dev(samples, mean)),
	};
};

const summaries: Summary[] = [
	summarize('cold init (compile+inst)', cold.map((p) => p.ci)),
	summarize('new PgletBtree()', cold.map((p) => p.np)),
	summarize('first query', cold.map((p) => p.fq)),
	summarize('full cold load', cold.map((p) => p.total)),
	summarize('cached instantiate', cached),
];

const ms = (n: number): string => n.toFixed(2);
const pad = (s: string, n: number) => s.padEnd(n);
const padl = (s: string, n: number) => s.padStart(n);

console.log(
	`pglet WASM init — ${SAMPLES} samples (+${WARMUP} warmup), per-iteration fresh module\n`,
);
console.log(`compile (cold, one-shot): ${ms(cold_compile_ms)} ms  ·  artifact ${(bytes.length / 1024).toFixed(0)} KB raw\n`);
console.log(
	`${pad('phase', 26)}${padl('p50', 9)}${padl('p95', 9)}${padl('mean', 9)}${padl('min', 9)}${padl('max', 9)}${padl('cv', 8)}`,
);
console.log('-'.repeat(78));
for (const r of summaries) {
	const flag = r.scenario === 'full cold load' ? (r.p50 <= TARGET_MS ? '  ✓ under target' : '  ⚠ over target') : '';
	console.log(
		`${pad(r.scenario, 26)}${padl(ms(r.p50), 9)}${padl(ms(r.p95), 9)}${padl(ms(r.mean), 9)}${padl(ms(r.min), 9)}${padl(ms(r.max), 9)}${padl((r.cv * 100).toFixed(1) + '%', 8)}${flag}`,
	);
}
console.log(`\n(all values in ms; target = ${TARGET_MS}ms for full cold load)`);

// Print by default; `--update` refreshes the recorded JSON — the idiom every
// other bench here follows, and which this one did not: it wrote on every run,
// so looking at the numbers overwrote them with whatever the box was doing.
if (update) {
	await write_json(RESULTS, {
		timestamp: new Date().toISOString(),
		git_commit: git_commit(),
		samples: SAMPLES,
		warmup: WARMUP,
		target_ms: TARGET_MS,
		note:
			`per-iteration fresh module, under node ${NO_MODULE_CACHE} so each sample compiles ` +
			'afresh; first query includes the lazy compile of the functions it reaches',
		compile_cold_one_shot_ms: cold_compile_ms,
		wasm_bytes: bytes.length,
		// reported beside the timings, never their gate: a gzip failure records -1 rather than
		// losing the run
		wasm_gzip_bytes: await gzip_size(fileURLToPath(WASM_PATH)).catch(() => -1),
		results: summaries,
	});
	console.error(`\nwrote benches/wasm_init_results.json`);
} else {
	console.error(`\n(results not written — pass --update to refresh wasm_init_results.json)`);
}
