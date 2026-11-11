/**
 * pglet's size gate, the body of `npm run size:check` and `npm run size:update`.
 *
 * Gates the **raw** wasm byte size, the size you build and directly control, against the
 * recorded baseline plus a hard cap. The gzipped (over-the-wire) size and the native
 * binary are measured and reported for information only; they are not gated. Raw bytes
 * are near-deterministic for a fixed toolchain, so a regression is a real code-size
 * change, not compressor noise.
 *
 * The scripts build the artifact first (they chain `wasm:build`). gzip is shelled out
 * (`gzip -c`, `to_gzip_args`) for the reported wire size, the measurement every
 * recorded gzip figure uses.
 *
 * @module
 */

import type { FsDir, FsReadDir } from '@fuzdev/gro/fs.ts';
import type { SpawnProgram } from '@fuzdev/gro/spawn.ts';
import { SilentError } from '@fuzdev/gro/task.ts';

import {
	BENCH_STAMP_UNKNOWN,
	format_bench_json,
	git_bench_stamp,
	to_gzip_args
} from './bench_results.ts';

/** The artifact gated by default, the plain `wasm:build` output, relative to the checkout. */
export const SIZE_WASM_DEFAULT = 'crates/pglet_wasm/pkg/pglet_wasm_bg.wasm';

/** The native binary reported beside it, in the release target directory. */
export const SIZE_NATIVE_NAME = 'pglet_server';

/** The release target directory, relative to the checkout, for display. */
export const SIZE_NATIVE_DIR = 'target/release';

/** The recorded baseline, in `benches/`. */
export const SIZE_BASELINE_NAME = 'size_baseline.json';

/**
 * The raw-bytes hard backstop; the baseline +2% guard normally binds first. This is the
 * source of truth: an update rewrites `hard_cap_bytes` in the baseline from here, so
 * editing the JSON alone is silently reverted by the next update.
 *
 * It is self-enforcing only because `size:check` runs second in `cargo xtask gates`:
 * the cap was once breached and stayed breached while this gate sat in no automated
 * set. Raising it is a deliberate decision, never a tripwire to move quietly.
 */
export const SIZE_HARD_CAP_BYTES = 1650 * 1024;

/** The growth over the baseline absorbed as toolchain noise before failing, in percent. */
export const SIZE_TOLERANCE_PCT = 2;

/** One measured artifact, as the baseline records it. */
export interface SizeArtifact {
	label: string;
	kind: 'wasm' | 'native';
	bytes: number;
	gzip_bytes: number;
}

/** What `run_size_command` reaches. */
export interface SizeGateDeps {
	/** The checkout, read-only: the wasm artifact is read through it. */
	checkout: FsReadDir;
	/** `benches/`, where the baseline is read, and written by an update. */
	benches: FsDir;
	/** The release target directory, or `null` when there is none. */
	release: FsReadDir | null;
	/** `gzip`, run on each artifact. */
	gzip: SpawnProgram;
	/** `git`, for an update's stamp, or `null` when there is none (the stamp is then `unknown`). */
	git: SpawnProgram | null;
	/** The clock an update's timestamp reads. */
	now: () => Date;
	print: (text: string) => void;
	print_error: (text: string) => void;
}

/** What a run does. */
export interface SizeGateOptions {
	/** `check` compares to the baseline; `update` rewrites it from this build. */
	mode: 'check' | 'update';
	/**
	 * The wasm to gate, relative to the checkout; the publish flow gates the shipped
	 * `pkg-npm` artifact (the same size and build flags as `pkg`, but for embedded
	 * metadata).
	 */
	wasm: string;
}

interface SizeBaseline {
	hard_cap_bytes?: number;
	tolerance_pct?: number;
	artifacts: Array<SizeArtifact>;
}

const kb = (n: number): string => `${(n / 1024).toFixed(1)} KB`;

/**
 * The byte count `gzip -c` writes for `path` in `dir`.
 *
 * @throws Error - when gzip fails, since a gate must not proceed on a figure it did not measure
 */
const gzip_size = async (gzip: SpawnProgram, dir: FsReadDir, path: string): Promise<number> => {
	const result = await gzip.out_bytes(to_gzip_args(path), { cwd: dir });
	if (result.kind !== 'exited' || result.code !== 0 || result.truncated) {
		throw new Error(`gzip failed: ${new TextDecoder().decode(result.stderr)}`);
	}
	return result.stdout.length;
};

/**
 * Measures `path` in `dir`.
 *
 * @throws Error - when the file is missing or gzip fails
 */
const measure = async (
	deps: SizeGateDeps,
	dir: FsReadDir,
	label: string,
	kind: SizeArtifact['kind'],
	path: string
): Promise<SizeArtifact> => {
	const info = await dir.stat(path);
	if (info === null) throw new Error(`no artifact at ${path}`);
	return { label, kind, bytes: info.size, gzip_bytes: await gzip_size(deps.gzip, dir, path) };
};

/**
 * Measures the native binary, reported for information only and never gated; a missing
 * one is a note, not a failure (only `size:update` builds it first).
 */
const measure_native = async (deps: SizeGateDeps): Promise<SizeArtifact | null> => {
	const native_path = `${SIZE_NATIVE_DIR}/${SIZE_NATIVE_NAME}`;
	if (deps.release === null || (await deps.release.stat(SIZE_NATIVE_NAME)) === null) {
		deps.print(`note: native binary ${native_path} not built — skipping (reported, not gated)`);
		return null;
	}
	return measure(deps, deps.release, SIZE_NATIVE_NAME, 'native', SIZE_NATIVE_NAME);
};

/**
 * Checks the wasm against the baseline, or with `mode: 'update'` records this build as
 * the baseline.
 *
 * @throws SilentError - exit 1 after naming the failure: the wasm is over its ceiling, or
 * there is no readable baseline
 * @throws Error - when an artifact can't be measured
 */
export const run_size_command = async (
	deps: SizeGateDeps,
	options: SizeGateOptions
): Promise<void> => {
	const { benches, print, print_error } = deps;
	const artifact = await measure(deps, deps.checkout, 'pglet_wasm_bg.wasm', 'wasm', options.wasm);
	const native = await measure_native(deps);

	if (options.mode === 'update') {
		const baseline = {
			timestamp: deps.now().toISOString(),
			git_commit: deps.git ? await git_bench_stamp(deps.git, deps.checkout) : BENCH_STAMP_UNKNOWN,
			hard_cap_bytes: SIZE_HARD_CAP_BYTES,
			tolerance_pct: SIZE_TOLERANCE_PCT,
			artifacts: native ? [artifact, native] : [artifact]
		};
		await benches.write(SIZE_BASELINE_NAME, format_bench_json(baseline));
		print(
			`baseline updated: ${artifact.label} ${kb(artifact.bytes)} raw / ${kb(artifact.gzip_bytes)} gzip`
		);
		if (native) {
			print(
				`  + ${native.label} ${kb(native.bytes)} raw / ${kb(native.gzip_bytes)} gzip (reported, not gated)`
			);
		}
		return;
	}

	let baseline: SizeBaseline;
	try {
		baseline = JSON.parse(await benches.read_text(SIZE_BASELINE_NAME));
	} catch {
		print_error(`no baseline at benches/${SIZE_BASELINE_NAME} — run \`npm run size:update\``);
		throw new SilentError();
	}

	const base = baseline.artifacts.find((a) => a.label === artifact.label);
	const hard_cap = baseline.hard_cap_bytes ?? SIZE_HARD_CAP_BYTES;
	const tolerance = baseline.tolerance_pct ?? SIZE_TOLERANCE_PCT;
	const ceiling = Math.min(
		base ? Math.floor(base.bytes * (1 + tolerance / 100)) : Infinity,
		hard_cap
	);

	print(`${artifact.label}: ${kb(artifact.bytes)} raw (${kb(artifact.gzip_bytes)} gzip wire)`);
	print(
		`  baseline ${base ? kb(base.bytes) : '—'} · ceiling ${kb(ceiling)} ` +
			`(min of baseline +${tolerance}% and hard cap ${kb(hard_cap)})`
	);

	// the native binary: reported for information only, never gated
	if (native) {
		print(
			`${native.label}: ${kb(native.bytes)} raw (${kb(native.gzip_bytes)} gzip) — reported, not gated`
		);
		const base_native = baseline.artifacts.find((a) => a.kind === 'native');
		if (base_native) {
			const delta = native.bytes - base_native.bytes;
			print(`  baseline ${kb(base_native.bytes)} · delta ${delta >= 0 ? '+' : ''}${kb(delta)}`);
		}
	}

	if (artifact.bytes > ceiling) {
		print_error(`FAIL: raw size ${kb(artifact.bytes)} exceeds ceiling ${kb(ceiling)}`);
		throw new SilentError();
	}
	print('OK');
};
