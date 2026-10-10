/**
 * pglet WASM init harness.
 *
 * Times the WASM/embedded startup path — the half of the <100ms thesis that
 * actually matters (the native server is sub-ms; the WASM module is the heavy
 * path, and where pglite is disqualifying at over a second). Phases:
 *   - cold init      : initSync({module: bytes})  — sync compile + instantiate
 *   - new PgletBtree(): engine/catalog construction (the shipped in-memory class)
 *   - first query    : first exec + query (lazy parser/catalog warmup)
 *   - full cold load : the three above, end to end (first-result latency)
 *   - cached inst.   : initSync({module: precompiled}) — instantiate only
 * plus a one-shot true-cold `WebAssembly.compile` of the bytes.
 *
 * Caveat (honest framing): V8 may serve cached compiled code for byte-identical
 * input, so the per-sample "cold init" compile trends toward *warm* compile
 * after the first sample. The genuine first-compile cost is reported separately
 * as `compile (cold, one-shot)`; the delta `cold init − cached inst.` is the
 * warm compile cost. Watch-tier: prints + records, never gates.
 *
 *   deno task bench:wasm-init
 */
import {
	stats_cv,
	stats_mean,
	stats_median,
	stats_min_max,
	stats_percentile,
	stats_std_dev,
} from '@fuzdev/fuz_util/stats.ts';
import {git_commit, write_json} from './lib/results.ts';

const PKG = new URL('../crates/pglet_wasm/pkg/', import.meta.url);
const JS = new URL('pglet_wasm.js', PKG).href;
const WASM_PATH = new URL('pglet_wasm_bg.wasm', PKG);
const WARMUP = 3;
const SAMPLES = 25;
const TARGET_MS = 100;

const bytes = await Deno.readFile(WASM_PATH);

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

const gzip_size = async (path: URL): Promise<number> => {
	try {
		const {success, stdout} = await new Deno.Command('gzip', {
			args: ['-c', path.pathname],
			stdout: 'piped',
		}).output();
		return success ? stdout.length : -1;
	} catch {
		return -1;
	}
};


// Print by default; `--update` refreshes the recorded JSON — the idiom every
// other bench here follows, and which this one did not: it wrote on every run,
// so looking at the numbers overwrote them with whatever the box was doing.
if (Deno.args.includes('--update')) {
	await write_json('benches/wasm_init_results.json', {
		timestamp: new Date().toISOString(),
		git_commit: git_commit(),
		samples: SAMPLES,
		warmup: WARMUP,
		target_ms: TARGET_MS,
		note: 'per-iteration fresh module; cold-init compile trends warm after sample 1 (V8 code cache)',
		compile_cold_one_shot_ms: cold_compile_ms,
		wasm_bytes: bytes.length,
		wasm_gzip_bytes: await gzip_size(WASM_PATH),
		results: summaries,
	});
	console.error(`\nwrote benches/wasm_init_results.json`);
} else {
	console.error(`\n(results not written — pass --update to refresh wasm_init_results.json)`);
}
