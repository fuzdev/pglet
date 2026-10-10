/**
 * pglet native startup harness.
 *
 * Measures wall time from spawn(pglet_server) to the "pglet listening on …"
 * ready line, across storage modes, reporting p50/p95 via fuz_util's stats.
 * This is the first real measurement of the <100ms startup claim.
 *
 * Scope: this measures *warm-binary cold-process* startup — the binary is in
 * the page cache, each process is fresh. That's the steady-state CLI case
 * (spawn-many-short-lived-processes). True page-cache-cold startup needs
 * privileges (drop_caches) and is not measured here. Informational
 * (watch-tier), not a gate: startup variance is real; the numbers inform.
 *
 *   deno task bench:startup
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

const BIN = 'target/release/pglet_server';
const MARKER = 'pglet listening on';
const WARMUP = 3;
const SAMPLES = 25;
const READY_TIMEOUT_MS = 10_000;
const TARGET_MS = 100;

let next_port = 6000;

/** Spawn the server with `args`, time spawn→ready, then kill it. Returns ms. */
const spawn_to_ready = async (args: string[]): Promise<number> => {
	const full = [...args, '--port', String(next_port++)];
	const t0 = performance.now();
	const child = new Deno.Command(BIN, {
		args: full,
		stdout: 'piped',
		stderr: 'null',
		stdin: 'null',
	}).spawn();

	const reader = child.stdout.getReader();
	const decoder = new TextDecoder();
	let buf = '';
	// `ReturnType<typeof setTimeout>` rather than `number`: this file's graph now
	// reaches `node:` builtins (through `./lib/results.ts`), and under those
	// typings `setTimeout` returns a `Timeout` object rather than a handle id.
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<never>((_resolve, reject) => {
		timer = setTimeout(() => reject(new Error('timeout waiting for ready')), READY_TIMEOUT_MS);
	});
	const read_loop = (async (): Promise<number> => {
		for (;;) {
			const {value, done} = await reader.read();
			if (done) throw new Error('server exited before ready');
			buf += decoder.decode(value, {stream: true});
			if (buf.includes(MARKER)) return performance.now();
		}
	})();

	try {
		const t1 = await Promise.race([read_loop, timeout]);
		return t1 - t0;
	} finally {
		clearTimeout(timer);
		await reader.cancel().catch(() => {});
		try {
			child.kill('SIGKILL');
		} catch {
			// already exited
		}
		await child.status;
	}
};

/** Each scenario yields fresh args per spawn (so "fresh dir" stays honest). */
interface Scenario {
	name: string;
	make_args: () => Promise<{args: string[]; dispose?: () => Promise<void>}>;
	teardown?: () => Promise<void>;
}

const scenarios: Scenario[] = [
	{name: 'memory', make_args: async () => ({args: ['--storage', 'memory']})},
	{name: 'btree (in-mem)', make_args: async () => ({args: []})},
	{
		// new empty data dir each spawn → measures first-open + file create
		name: 'btree (file, fresh)',
		make_args: async () => {
			const dir = await Deno.makeTempDir({prefix: 'pglet_bench_fresh_'});
			return {
				args: ['--data-dir', dir],
				dispose: () => Deno.remove(dir, {recursive: true}).catch(() => {}),
			};
		},
	},
	(() => {
		// one dir, initialized once, reused → measures reopen (load existing)
		let dir: string | undefined;
		return {
			name: 'btree (file, reopen)',
			make_args: async () => {
				if (dir === undefined) {
					dir = await Deno.makeTempDir({prefix: 'pglet_bench_reopen_'});
					await spawn_to_ready(['--data-dir', dir]); // initialize the files
				}
				return {args: ['--data-dir', dir]};
			},
			teardown: async () => {
				if (dir !== undefined) await Deno.remove(dir, {recursive: true}).catch(() => {});
			},
		};
	})(),
];

const measure = async (s: Scenario): Promise<number[]> => {
	const samples: number[] = [];
	try {
		for (let i = 0; i < WARMUP + SAMPLES; i++) {
			const {args, dispose} = await s.make_args();
			try {
				const ms = await spawn_to_ready(args);
				if (i >= WARMUP) samples.push(ms);
			} finally {
				await dispose?.();
			}
		}
	} finally {
		await s.teardown?.();
	}
	return samples;
};

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

const ms = (n: number): string => n.toFixed(2);

console.log(
	`pglet native startup — spawn→ready, ${SAMPLES} samples (+${WARMUP} warmup), warm binary / cold process\n`,
);

const summaries: Summary[] = [];
for (const s of scenarios) {
	const samples = await measure(s);
	summaries.push(summarize(s.name, samples));
}

const pad = (s: string, n: number) => s.padEnd(n);
const padl = (s: string, n: number) => s.padStart(n);
console.log(
	`${pad('scenario', 22)}${padl('p50', 9)}${padl('p95', 9)}${padl('mean', 9)}${padl('min', 9)}${padl('max', 9)}${padl('cv', 8)}`,
);
console.log('-'.repeat(75));
for (const r of summaries) {
	const flag = r.p50 <= TARGET_MS ? '' : '  ⚠ over target';
	console.log(
		`${pad(r.scenario, 22)}${padl(ms(r.p50), 9)}${padl(ms(r.p95), 9)}${padl(ms(r.mean), 9)}${padl(ms(r.min), 9)}${padl(ms(r.max), 9)}${padl((r.cv * 100).toFixed(1) + '%', 8)}${flag}`,
	);
}
console.log(`\n(all values in ms; target = ${TARGET_MS}ms p50)`);


// Print by default; `--update` refreshes the recorded JSON — the idiom every
// other bench here follows, and which this one did not: it wrote on every run,
// so looking at the numbers overwrote them with whatever the box was doing.
if (Deno.args.includes('--update')) {
	await write_json('benches/startup_results.json', {
		timestamp: new Date().toISOString(),
		git_commit: git_commit(),
		samples: SAMPLES,
		warmup: WARMUP,
		target_ms: TARGET_MS,
		note: 'warm-binary cold-process; true page-cache-cold not measured',
		results: summaries,
	});
	console.error(`\nwrote benches/startup_results.json`);
} else {
	console.error(`\n(results not written — pass --update to refresh startup_results.json)`);
}
