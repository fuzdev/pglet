/**
 * pglet index-acceleration bench — does query index acceleration still earn its keep?
 *
 * Runs a fixture-shaped load (a table with a PK + a non-unique b-tree index + a GIN
 * index; seed; then index-eligible reads: b-tree `=`, GIN `@>`, PK point) across table
 * sizes on the FULL (acceleration on) vs STRIPPED (`--no-default-features`, acceleration
 * off) wasm build, and reports the wasm-size delta alongside the read-latency ratio.
 *
 * This is the guard behind the stripped "testing" build profile: dropping index
 * acceleration is a *size* lever (~2.4% off the wasm), NOT a test-speed win — an index
 * pays for itself by ~200 rows, so `stripped ÷ full` should stay ≥ ~1 and grow with N.
 * A regression (ratio near 1 at large N) would mean acceleration stopped helping.
 *
 * Watch-tier: prints by default; `--update` records `benches/index_accel_results.json`.
 * Builds both profiles (stripped first, full last) so the working tree is left on the
 * default full build.
 *
 *   deno task bench:index-accel
 */

import {git_commit, write_json} from './lib/results.ts';

const PKG = new URL('../crates/pglet_wasm/pkg/', import.meta.url);
const JS = new URL('pglet_wasm.js', PKG).href;
const WASM_PATH = new URL('pglet_wasm_bg.wasm', PKG);
const SIZES = [20, 200, 2000];
const SAMPLES = 300;
const WARMUP = 40;
const CYCLE_REPS = 150;
const CYCLE_SEED = 25;

const pctl = (xs: number[], q: number): number => {
	const s = [...xs].sort((a, b) => a - b);
	return s[Math.min(s.length - 1, Math.floor(q * s.length))];
};

// ---- child mode (`--measure`): measure the currently-built pkg, emit JSON (last line) ----
const measure = async () => {
	const bytes = await Deno.readFile(WASM_PATH);
	const m: any = await import(JS);
	m.initSync({module: bytes});

	const schema = (db: any) => {
		db.exec(
			`CREATE TABLE fact (id BIGSERIAL PRIMARY KEY, kind TEXT NOT NULL, refs TEXT[] NOT NULL, data JSONB NOT NULL)`,
		);
		db.exec(`CREATE INDEX fact_kind ON fact (kind)`); // non-unique b-tree (accel-only)
		db.exec(`CREATE INDEX fact_refs ON fact USING gin (refs)`); // GIN (accel-only)
	};
	const seed = (db: any, n: number) => {
		const vals: string[] = [];
		for (let i = 0; i < n; i++) {
			vals.push(`('k${i % 8}', ARRAY['t${i % 5}','t${(i + 1) % 5}'], '{"n":${i}}'::jsonb)`);
		}
		db.exec(`INSERT INTO fact (kind, refs, data) VALUES ${vals.join(',')}`);
	};
	// three index-eligible reads: b-tree equality, GIN containment, PK point
	const batch = (db: any) => {
		db.query(`SELECT id FROM fact WHERE kind = 'k5'`);
		db.query(`SELECT id FROM fact WHERE refs @> ARRAY['t3']`);
		db.query(`SELECT id FROM fact WHERE id = 7`);
	};

	const rows = SIZES.map((n) => {
		const db = new m.PgletBtree();
		schema(db);
		seed(db, n);
		for (let i = 0; i < WARMUP; i++) batch(db);
		const t: number[] = [];
		for (let i = 0; i < SAMPLES; i++) {
			const s = performance.now();
			batch(db);
			t.push(performance.now() - s);
		}
		db.free();
		return {n, p50: pctl(t, 0.5), p95: pctl(t, 0.95)};
	});

	// full test cycle: fresh db + schema + seed(tiny) + query batches (the fixture pattern)
	for (let i = 0; i < 30; i++) {
		const db = new m.PgletBtree();
		schema(db);
		seed(db, CYCLE_SEED);
		for (let q = 0; q < 10; q++) batch(db);
		db.free();
	}
	const ct: number[] = [];
	for (let i = 0; i < CYCLE_REPS; i++) {
		const s = performance.now();
		const db = new m.PgletBtree();
		schema(db);
		seed(db, CYCLE_SEED);
		for (let q = 0; q < 10; q++) batch(db);
		db.free();
		ct.push(performance.now() - s);
	}
	console.log(
		JSON.stringify({wasm_bytes: bytes.length, rows, cycle: {p50: pctl(ct, 0.5), p95: pctl(ct, 0.95)}}),
	);
};

if (Deno.args.includes('--measure')) {
	await measure();
	Deno.exit(0);
}

// ---- orchestrator: build each profile, measure in a clean subprocess, compare ----
const build = async (stripped: boolean) => {
	const args = ['build', 'crates/pglet_wasm', '--target', 'web'];
	if (stripped) args.push('--', '--no-default-features', '--features', 'console_error_panic_hook');
	const {success} = await new Deno.Command('wasm-pack', {args, stdout: 'null', stderr: 'null'}).output();
	if (!success) throw new Error(`wasm-pack build failed (stripped=${stripped})`);
};

const run_measure = async (): Promise<{wasm_bytes: number; rows: {n: number; p50: number; p95: number}[]; cycle: {p50: number; p95: number}}> => {
	const {success, stdout} = await new Deno.Command(Deno.execPath(), {
		args: ['run', '-A', new URL(import.meta.url).pathname, '--measure'],
		stdout: 'piped',
		stderr: 'inherit',
	}).output();
	if (!success) throw new Error('measure subprocess failed');
	const last = new TextDecoder().decode(stdout).trim().split('\n').at(-1) ?? '';
	return JSON.parse(last);
};

console.error('building stripped (accel off)…');
await build(true);
const stripped = await run_measure();
console.error('building full (accel on)…');
await build(false);
const full = await run_measure();

const fmt = (n: number) => n.toFixed(4);
const padl = (s: string, n: number) => s.padStart(n);
console.log(`\npglet index-acceleration — 3-query batch p50 (ms), full (accel) vs stripped\n`);
const size_pct = ((1 - stripped.wasm_bytes / full.wasm_bytes) * 100).toFixed(1);
console.log(`wasm: full ${full.wasm_bytes} B · stripped ${stripped.wasm_bytes} B  (stripped −${size_pct}%)\n`);
console.log(`${padl('rows', 8)}${padl('full', 12)}${padl('stripped', 12)}${padl('stripped÷full', 16)}`);
console.log('-'.repeat(48));
for (let i = 0; i < full.rows.length; i++) {
	const f = full.rows[i];
	const s = stripped.rows[i];
	console.log(
		`${padl(String(f.n), 8)}${padl(fmt(f.p50), 12)}${padl(fmt(s.p50), 12)}${padl((s.p50 / f.p50).toFixed(2) + '×', 16)}`,
	);
}
console.log(`\nfull test cycle (fresh db + schema + seed ${CYCLE_SEED} + 10 batches), p50 ms:`);
console.log(
	`  full ${full.cycle.p50.toFixed(3)}  ·  stripped ${stripped.cycle.p50.toFixed(3)}  (${(stripped.cycle.p50 / full.cycle.p50).toFixed(2)}×)`,
);
console.log(
	`\nAcceleration is a size lever, not a speed one: stripping drops ~${size_pct}% of the wasm but\n` +
		`makes reads slower (stripped ÷ full ≥ ~1, growing with N — an index pays for itself by ~200 rows).`,
);

if (Deno.args.includes('--update')) {
	await write_json('benches/index_accel_results.json', {
				timestamp: new Date().toISOString(),
				git_commit: git_commit(),
				samples: SAMPLES,
				warmup: WARMUP,
				cycle_reps: CYCLE_REPS,
				cycle_seed: CYCLE_SEED,
				note: 'fixture-shaped load; ratio = stripped(accel off) ÷ full(accel on); >1 means acceleration helps',
		full,
		stripped,
	});
}
