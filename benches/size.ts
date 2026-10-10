/**
 * pglet size gate.
 *
 * Gates the **raw** WASM byte size — the size you build and directly control —
 * against a committed baseline plus a hard cap. The gzipped (over-the-wire) size
 * and the native binary are measured and reported for information only; they are
 * not gated. Raw bytes are near-deterministic for a fixed toolchain, so a
 * regression is a real code-size change, not compressor noise.
 *
 *   deno task size:check    # compare to baseline, exit 1 on regression
 *   deno task size:update   # rewrite the baseline from the current artifact
 *
 * Build the artifact first (the tasks chain `wasm:build`). gzip is shelled out
 * (`gzip -c`) for the reported wire-size figure, matching the ecosystem convention.
 */

import {git_commit, write_json} from './lib/results.ts';

// Default gates the plain `wasm:build` output (`pkg`); `--wasm <path>` overrides it so the
// publish flow can gate the actually-shipped `pkg-npm` artifact (byte-identical size, same
// build flags — only the embedded producers/name metadata differs).
const wasm_flag = Deno.args.indexOf('--wasm');
const WASM =
	wasm_flag >= 0 && Deno.args[wasm_flag + 1]
		? Deno.args[wasm_flag + 1]
		: 'crates/pglet_wasm/pkg/pglet_wasm_bg.wasm';
const NATIVE = 'target/release/pglet_server';
const BASELINE = 'benches/size_baseline.json';
// Raw-bytes hard backstop; the baseline +2% guard normally binds first. This is the
// source of truth — `--update` rewrites `hard_cap_bytes` in the baseline JSON from
// here, so editing the JSON alone is silently reverted by the next update.
//
// It is self-enforcing only because `size:check` runs second in `cargo xtask gates` —
// the cap was once breached and stayed breached while this gate sat in no automated
// set. Raising it is a deliberate decision, never a tripwire to move quietly.
const HARD_CAP_BYTES = 1650 * 1024; // 1.65 MB raw
const TOLERANCE_PCT = 2; // absorb toolchain noise before failing

interface Artifact {
	label: string;
	kind: 'wasm' | 'native';
	bytes: number;
	gzip_bytes: number;
}

const kb = (n: number): string => `${(n / 1024).toFixed(1)} KB`;

const gzip_size = async (path: string): Promise<number> => {
	const {success, stdout, stderr} = await new Deno.Command('gzip', {
		args: ['-c', path],
		stdout: 'piped',
		stderr: 'piped',
	}).output();
	if (!success) throw new Error(`gzip failed: ${new TextDecoder().decode(stderr)}`);
	return stdout.length;
};


const measure = async (label: string, kind: Artifact['kind'], path: string): Promise<Artifact> => ({
	label,
	kind,
	bytes: (await Deno.stat(path)).size,
	gzip_bytes: await gzip_size(path),
});

// The native binary is measured and reported for information only — never gated.
// Missing baselines are skipped, not fatal: if the release binary isn't built,
// note it and carry on (only `size:update` builds it first).
const measure_native = async (): Promise<Artifact | null> => {
	try {
		await Deno.stat(NATIVE);
	} catch {
		console.log(`note: native binary ${NATIVE} not built — skipping (reported, not gated)`);
		return null;
	}
	return measure('pglet_server', 'native', NATIVE);
};

const mode = Deno.args.includes('--update') ? 'update' : 'check';
const artifact = await measure('pglet_wasm_bg.wasm', 'wasm', WASM);
const native = await measure_native();

if (mode === 'update') {
	const baseline = {
		timestamp: new Date().toISOString(),
		git_commit: git_commit(),
		hard_cap_bytes: HARD_CAP_BYTES,
		tolerance_pct: TOLERANCE_PCT,
		artifacts: native ? [artifact, native] : [artifact],
	};
	await write_json(BASELINE, baseline);
	console.log(`baseline updated: ${artifact.label} ${kb(artifact.bytes)} raw / ${kb(artifact.gzip_bytes)} gzip`);
	if (native) {
		console.log(
			`  + ${native.label} ${kb(native.bytes)} raw / ${kb(native.gzip_bytes)} gzip (reported, not gated)`,
		);
	}
	Deno.exit(0);
}

let baseline: {hard_cap_bytes?: number; tolerance_pct?: number; artifacts: Artifact[]};
try {
	baseline = JSON.parse(await Deno.readTextFile(BASELINE));
} catch {
	console.error(`no baseline at ${BASELINE} — run \`deno task size:update\``);
	Deno.exit(1);
}

const base = baseline.artifacts.find((a) => a.label === artifact.label);
const hard_cap = baseline.hard_cap_bytes ?? HARD_CAP_BYTES;
const tolerance = baseline.tolerance_pct ?? TOLERANCE_PCT;
const ceiling = Math.min(
	base ? Math.floor(base.bytes * (1 + tolerance / 100)) : Infinity,
	hard_cap,
);

console.log(`${artifact.label}: ${kb(artifact.bytes)} raw (${kb(artifact.gzip_bytes)} gzip wire)`);
console.log(
	`  baseline ${base ? kb(base.bytes) : '—'} · ceiling ${kb(ceiling)} ` +
		`(min of baseline +${tolerance}% and hard cap ${kb(hard_cap)})`,
);

// Native binary: reported for information only — never gated, never exits.
if (native) {
	console.log(`${native.label}: ${kb(native.bytes)} raw (${kb(native.gzip_bytes)} gzip) — reported, not gated`);
	const base_native = baseline.artifacts.find((a) => a.kind === 'native');
	if (base_native) {
		const delta = native.bytes - base_native.bytes;
		console.log(`  baseline ${kb(base_native.bytes)} · delta ${delta >= 0 ? '+' : ''}${kb(delta)}`);
	}
}

if (artifact.bytes > ceiling) {
	console.error(`FAIL: raw size ${kb(artifact.bytes)} exceeds ceiling ${kb(ceiling)}`);
	Deno.exit(1);
}
console.log('OK');
