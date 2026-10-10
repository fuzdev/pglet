/**
 * Memory-footprint measurement — the two surfaces, one command.
 *
 * A **pglet-internal, longitudinal** metric — "did this change raise peak?" —
 * NOT a cross-engine ratio. A pglet/pglite memory comparison is meaningless:
 * pglite is whole-Postgres and WASM linear memory only ever grows, so the ratio
 * says nothing about pglet. "Rough is fine" (the discipline's word): peak
 * resident set / linear-memory high-water, no custom allocator hooks. Watch-tier
 * — print + results JSON, no gate (only `size:check` gates). Read a number
 * against a *prior local run* (its delta), never a committed absolute.
 *
 * Two surfaces, each in its natural mode:
 *   - **native embed engine** — peak RSS of the file-backed double-meta engine
 *     (CoW B+tree + LRU page cache). Measured by `pglet_bench memory`, which
 *     spawns a fresh child per workload and reads `/proc/self/status` VmHWM; this
 *     script just relays its `--emit-json` output. (Rust owns that half so it's
 *     also runnable bare via `cargo run -p pglet_bench -- memory`.)
 *   - **WASM in-memory engine** — linear-memory high-water
 *     (`memory.buffer.byteLength`) after a fixed load into `PgletBtree` (the
 *     in-memory CoW-btree the fuz_app wasm leg uses). Linear memory never
 *     shrinks, so each workload runs in a **fresh subprocess** (cold module) for
 *     a clean per-workload high-water.
 *
 * Workload (both surfaces, fixed + repeatable): the two schema axes
 * `indexfree` / `enforcing` (same DDL as the `commit`/`attribute` benches) ×
 * dataset `{10k, 100k}`, load-only. This script is the single writer of
 * `benches/memory_results.json` (native + wasm sections), print-by-default /
 * `--update`, mirroring `wasm_compare.ts`.
 *
 *   deno task bench:memory           # print both surfaces
 *   deno task bench:memory:update    # ...and refresh memory_results.json
 */
import {readFile, writeFile} from 'node:fs/promises';
import {git_commit, write_json} from './lib/results.ts';

// --- shared workload axes (mirror crates/pglet_bench BenchSchema + ROW_SIZES) ---
const ROW_SIZES = [10_000, 100_000];
const SCHEMAS = ['indexfree', 'enforcing'] as const;
type Schema = (typeof SCHEMAS)[number];

const TEXT_B = 'abcdefghijklmnopqrstuvwxyz012345'; // 32 chars, matches pglet_bench
const CHUNK = 2_000; // rows per BEGIN/COMMIT (matches PREPOPULATE_CHUNK)

const NATIVE_BIN = 'target/release/pglet_bench';
const PGLET_WASM = new URL('../crates/pglet_wasm/pkg/pglet_wasm_bg.wasm', import.meta.url);
const SELF = new URL(import.meta.url).pathname;

/** The `CREATE …` DDL for a schema axis — byte-identical to `BenchSchema::create_stmts`. */
function create_stmts(schema: Schema): string[] {
	return schema === 'enforcing'
		? [
				'CREATE TABLE bench (id BIGINT PRIMARY KEY, a BIGINT, b TEXT)',
				'CREATE INDEX bench_a_idx ON bench (a)',
			]
		: ['CREATE TABLE bench (id BIGINT, a BIGINT, b TEXT)'];
}

// Deterministic pseudo-random column `a` (filler; matches wasm_compare.ts).
function pseudo_a(id: number): number {
	let z = (id * 2654435761) >>> 0;
	z ^= z >>> 15;
	z = (z * 2246822519) >>> 0;
	z ^= z >>> 13;
	return z & 0x7fff_ffff;
}

// ==========================================================================
// WASM child: one workload in a fresh process → JSON line on stdout.
// ==========================================================================
async function run_wasm_child(schema: Schema, rows: number): Promise<void> {
	const line = (obj: unknown) => console.log(JSON.stringify(obj));
	try {
		const mod: any = await import('../crates/pglet_wasm/pkg/pglet_wasm.js');
		const bytes = await readFile(PGLET_WASM);
		// `initSync` returns InitOutput whose `.memory` is the live WebAssembly.Memory;
		// `.buffer.byteLength` always reflects the current (monotonic) linear size.
		const out = mod.initSync({module: bytes});
		const mem: WebAssembly.Memory = out.memory;
		const initial_bytes = mem.buffer.byteLength;

		const db = new mod.PgletBtree(); // in-memory CoW B+tree; supports BEGIN/COMMIT
		for (const stmt of create_stmts(schema)) db.exec(stmt);

		// Chunked BEGIN/COMMIT load (bounds the per-commit working set, same shape
		// as the native prepopulate). One multi-row INSERT per chunk.
		for (let start = 0; start < rows; start += CHUNK) {
			const end = Math.min(start + CHUNK, rows);
			const values: string[] = [];
			for (let id = start; id < end; id++) values.push(`(${id}, ${pseudo_a(id)}, '${TEXT_B}')`);
			db.exec('BEGIN');
			db.exec(`INSERT INTO bench VALUES ${values.join(',')}`);
			db.exec('COMMIT');
		}

		// Sanity: the load actually landed.
		const n = Number((db.query('SELECT count(*) AS n FROM bench').rows[0] as any)?.n);
		if (n !== rows) throw new Error(`row count mismatch: got ${n}, want ${rows}`);

		const peak_bytes = mem.buffer.byteLength;
		line({
			engine: 'pglet',
			surface: 'wasm_inmemory',
			schema,
			rows,
			initial_bytes,
			peak_bytes,
			delta_bytes: peak_bytes - initial_bytes,
			units: 'bytes',
			higher_or_lower_is_better: 'lower',
		});
	} catch (err) {
		line({engine: 'pglet', surface: 'wasm_inmemory', schema, rows, error: (err as Error)?.message ?? String(err)});
	}
}

interface WasmRow {
	engine: string;
	surface: string;
	schema: string;
	rows: number;
	initial_bytes: number;
	peak_bytes: number;
	delta_bytes: number;
	units: string;
	higher_or_lower_is_better: string;
	error?: string;
}

/** Spawn a fresh Deno process for one WASM workload (cold module → clean high-water). */
async function measure_wasm_cell(schema: Schema, rows: number): Promise<WasmRow | null> {
	const cmd = new Deno.Command(Deno.execPath(), {
		args: ['run', '-A', SELF, '--wasm-child', schema, String(rows)],
		stdout: 'piped',
		stderr: 'inherit',
	});
	const {success, stdout} = await cmd.output();
	if (!success) return null;
	const text = new TextDecoder().decode(stdout).trim();
	const jsonLine = text.split('\n').findLast((l) => l.startsWith('{'));
	return jsonLine ? (JSON.parse(jsonLine) as WasmRow) : null;
}

// ==========================================================================
// Native surface: relay `pglet_bench memory --emit-json`.
// ==========================================================================
interface NativeRow {
	engine: string;
	surface: string;
	schema: string;
	rows: number;
	baseline_rss_kb: number;
	current_rss_kb: number;
	peak_rss_kb: number;
	delta_kb: number;
	file_size_kb: number;
	units: string;
	higher_or_lower_is_better: string;
}

async function measure_native(): Promise<NativeRow[]> {
	const cmd = new Deno.Command(NATIVE_BIN, {
		args: ['memory', '--emit-json'],
		stdout: 'piped',
		stderr: 'inherit',
	});
	const {success, stdout} = await cmd.output();
	if (!success) throw new Error(`${NATIVE_BIN} memory --emit-json failed (is it built? deno task bench:memory builds it)`);
	return JSON.parse(new TextDecoder().decode(stdout)) as NativeRow[];
}

// ==========================================================================
// Reporting.
// ==========================================================================
const pad = (s: string, n: number) => s.padEnd(n);
const padl = (s: string, n: number) => s.padStart(n);
const kb = (n: number) => (n / 1024).toFixed(2);

function print_native(rows: NativeRow[]): void {
	console.log(`\n=== native embed engine — peak RSS (MB; longitudinal, watch-tier) ===`);
	console.log(`file-backed double-meta (CoW B+tree + LRU page cache); load-only; peak = /proc VmHWM, baseline = VmRSS at start`);
	console.log(
		`${pad('schema', 12)}${padl('rows', 9)}${padl('baseline', 11)}${padl('current', 10)}${padl('peak', 10)}${padl('delta', 10)}${padl('file', 10)}`,
	);
	console.log('-'.repeat(72));
	for (const r of rows) {
		console.log(
			`${pad(r.schema, 12)}${padl(String(r.rows), 9)}${padl(kb(r.baseline_rss_kb), 11)}${padl(kb(r.current_rss_kb), 10)}${padl(kb(r.peak_rss_kb), 10)}${padl(kb(r.delta_kb), 10)}${padl(kb(r.file_size_kb), 10)}`,
		);
	}
	console.log('-'.repeat(72));
}

function print_wasm(rows: WasmRow[]): void {
	console.log(`\n=== WASM in-memory engine — linear-memory high-water (MB; watch-tier) ===`);
	console.log(`PgletBtree (in-memory CoW B+tree); load-only; high-water = memory.buffer.byteLength; fresh module per workload`);
	console.log(`${pad('schema', 12)}${padl('rows', 9)}${padl('initial', 11)}${padl('peak', 11)}${padl('delta', 11)}`);
	console.log('-'.repeat(56));
	for (const r of rows) {
		if (r.error) {
			console.log(`${pad(r.schema, 12)}${padl(String(r.rows), 9)}${padl('SKIPPED: ' + r.error, 36)}`);
			continue;
		}
		console.log(
			`${pad(r.schema, 12)}${padl(String(r.rows), 9)}${padl(kb(r.initial_bytes / 1024), 11)}${padl(kb(r.peak_bytes / 1024), 11)}${padl(kb(r.delta_bytes / 1024), 11)}`,
		);
	}
	console.log('-'.repeat(56));
}


// ==========================================================================
// Entry.
// ==========================================================================
const args = Deno.args;
const child_i = args.indexOf('--wasm-child');
if (child_i !== -1) {
	// Child mode: one WASM workload, JSON to stdout, exit.
	const schema = (args[child_i + 1] ?? 'indexfree') as Schema;
	const rows = Number(args[child_i + 2] ?? ROW_SIZES[0]);
	await run_wasm_child(schema, rows);
} else {
	const update = args.includes('--update');

	console.error('  native (pglet_bench memory) ...');
	const native = await measure_native();

	const wasm: WasmRow[] = [];
	for (const rows of ROW_SIZES) {
		for (const schema of SCHEMAS) {
			console.error(`  wasm schema=${schema} rows=${rows} ...`);
			const r = await measure_wasm_cell(schema, rows);
			if (r) wasm.push(r);
		}
	}

	console.log(`\npglet memory footprint — pglet-internal longitudinal (NOT a cross-engine ratio)`);
	console.log(`workload: load-only; schemas [${SCHEMAS.join(', ')}] × rows [${ROW_SIZES.join(', ')}]`);
	print_native(native);
	print_wasm(wasm);
	console.log(
		`\nNOTE: pglet-internal tripwire — read each number against a PRIOR LOCAL run's delta,` +
			`\n  never a committed absolute (perf memory isn't a gated baseline; only size:check gates).` +
			`\n  No cross-engine ratio: pglite is whole-Postgres and WASM linear memory only grows.` +
			`\n  Native peak = /proc VmHWM (glibc retains freed pages, so current≈peak); WASM = linear-memory high-water.`,
	);

	if (update) {
		const doc = {
			schema: 'memory',
			note: 'pglet-internal longitudinal peak; NOT a cross-engine ratio (pglite is whole-Postgres; WASM linear memory only grows). Watch-tier — compare to a prior local run, never a committed absolute; only size:check gates.',
			meta: {
				timestamp: new Date().toISOString(),
				git_commit: git_commit(),
				workload: `load-only; schemas [${SCHEMAS.join(', ')}] × rows [${ROW_SIZES.join(', ')}]`,
				native_method: 'peak RSS via /proc/self/status VmHWM; fresh child process per workload; file-backed double-meta (CoW B+tree + LRU page cache, default 2000 pages)',
				wasm_method: 'linear-memory high-water (WebAssembly.Memory.buffer.byteLength); fresh module (subprocess) per workload; in-memory PgletBtree',
				units: {native: 'kilobytes', wasm: 'bytes'},
				higher_or_lower_is_better: 'lower',
			},
			native,
			wasm,
		};
		const outUrl = new URL('./memory_results.json', import.meta.url);
		await write_json(outUrl, doc);
		console.error(`\nwrote ${outUrl.pathname}`);
	} else {
		console.error(`\n(results not written — pass --update to refresh benches/memory_results.json)`);
	}
}
