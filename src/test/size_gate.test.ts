import { is_silent_error } from '@fuzdev/gro/task.ts';
import { fs, type FsDir } from '@fuzdev/gro/fs.ts';
import type {
	SpawnOutOptions,
	SpawnOutput,
	SpawnOutputBytes,
	SpawnProgram
} from '@fuzdev/gro/spawn.ts';
import { assert, describe, test } from 'vitest';

import {
	run_size_command,
	SIZE_HARD_CAP_BYTES,
	SIZE_WASM_DEFAULT,
	type SizeGateDeps
} from '#lib/size_gate.ts';

const unreached = () => {
	throw new Error('not reached');
};

/** A `gzip` whose output is `gzip_bytes` long per file, by the file's path, or that fails. */
const to_fake_gzip = (
	gzip_bytes: Record<string, number>,
	calls: Array<{ args: ReadonlyArray<string>; cwd: string | undefined }> = []
): SpawnProgram => ({
	name: 'gzip',
	path: '/fake/gzip',
	run: unreached,
	out: unreached,
	start: unreached,
	out_bytes: async (
		args: ReadonlyArray<string>,
		options?: SpawnOutOptions
	): Promise<SpawnOutputBytes> => {
		calls.push({ args, cwd: options?.cwd?.path });
		const n = gzip_bytes[args[1]!];
		if (n === undefined) {
			return {
				kind: 'exited',
				code: 1,
				stdout: new Uint8Array(),
				stderr: new TextEncoder().encode('gzip: no such file'),
				truncated: false
			};
		}
		return {
			kind: 'exited',
			code: 0,
			stdout: new Uint8Array(n),
			stderr: new Uint8Array(),
			truncated: false
		};
	}
});

/** A `git` at commit `abc1234` whose tree is clean. */
const fake_git: SpawnProgram = {
	name: 'git',
	path: '/fake/git',
	run: unreached,
	out_bytes: unreached,
	start: unreached,
	out: async (args: ReadonlyArray<string>): Promise<SpawnOutput> => ({
		kind: 'exited',
		code: 0,
		stdout: args[0] === 'rev-parse' ? 'abc1234\n' : '',
		stderr: '',
		truncated: false
	})
};

const NATIVE = 'target/release';

/** A checkout holding a wasm of `wasm_bytes` and, unless `null`, a native binary. */
const to_checkout = async (
	dir: FsDir,
	wasm_bytes: number,
	native_bytes: number | null,
	baseline?: unknown
): Promise<void> => {
	await dir.mkdir('crates/pglet_wasm/pkg', { recursive: true });
	await dir.write(SIZE_WASM_DEFAULT, new Uint8Array(wasm_bytes));
	await dir.mkdir('benches');
	if (baseline !== undefined) {
		await dir.write('benches/size_baseline.json', JSON.stringify(baseline));
	}
	if (native_bytes !== null) {
		await dir.mkdir(NATIVE, { recursive: true });
		await dir.write(`${NATIVE}/pglet_server`, new Uint8Array(native_bytes));
	}
};

const to_deps = (
	dir: FsDir,
	printed: Array<string>,
	gzip: SpawnProgram = to_fake_gzip({ [SIZE_WASM_DEFAULT]: 2048, pglet_server: 4096 })
): SizeGateDeps => ({
	checkout: dir.readonly(),
	benches: dir.dir('benches'),
	release: dir.exists_sync(`${NATIVE}/pglet_server`) ? dir.dir(NATIVE).readonly() : null,
	gzip,
	git: fake_git,
	now: () => new Date('2026-01-02T03:04:05.000Z'),
	print: (text) => printed.push(text),
	print_error: (text) => printed.push(`ERR ${text}`)
});

const baseline_of = (wasm_bytes: number, extra: Record<string, unknown> = {}) => ({
	hard_cap_bytes: SIZE_HARD_CAP_BYTES,
	tolerance_pct: 2,
	artifacts: [
		{ label: 'pglet_wasm_bg.wasm', kind: 'wasm', bytes: wasm_bytes, gzip_bytes: 1 },
		{ label: 'pglet_server', kind: 'native', bytes: 10 * 1024, gzip_bytes: 1 }
	],
	...extra
});

const assert_silent_failure = async (run: Promise<void>): Promise<void> => {
	let error: unknown = null;
	try {
		await run;
	} catch (e) {
		error = e;
	}
	assert.isTrue(is_silent_error(error), `expected a SilentError, got ${String(error)}`);
};

describe('run_size_command check', () => {
	test('passes under the ceiling, reporting the native binary against its baseline', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 1000 * 1024, 12 * 1024, baseline_of(1000 * 1024));
		const printed: Array<string> = [];
		await run_size_command(to_deps(dir, printed), { mode: 'check', wasm: SIZE_WASM_DEFAULT });
		assert.deepEqual(printed, [
			'pglet_wasm_bg.wasm: 1000.0 KB raw (2.0 KB gzip wire)',
			'  baseline 1000.0 KB · ceiling 1020.0 KB (min of baseline +2% and hard cap 1650.0 KB)',
			'pglet_server: 12.0 KB raw (4.0 KB gzip) — reported, not gated',
			'  baseline 10.0 KB · delta +2.0 KB',
			'OK'
		]);
	});

	test('passes at exactly the ceiling, and notes a native binary that is not built', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		// floor(1000 * 1.02) = 1020: the ceiling itself passes
		await to_checkout(dir, 1020, null, baseline_of(1000));
		const printed: Array<string> = [];
		await run_size_command(to_deps(dir, printed), { mode: 'check', wasm: SIZE_WASM_DEFAULT });
		assert.deepEqual(printed, [
			'note: native binary target/release/pglet_server not built — skipping (reported, not gated)',
			'pglet_wasm_bg.wasm: 1.0 KB raw (2.0 KB gzip wire)',
			'  baseline 1.0 KB · ceiling 1.0 KB (min of baseline +2% and hard cap 1650.0 KB)',
			'OK'
		]);
	});

	test('rounds the ceiling down: a fractional +2% grants no extra byte', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		// 1001 * 1.02 = 1021.02, so the ceiling is 1021 and 1022 fails
		await to_checkout(dir, 1022, null, baseline_of(1001));
		const printed: Array<string> = [];
		await assert_silent_failure(
			run_size_command(to_deps(dir, printed), { mode: 'check', wasm: SIZE_WASM_DEFAULT })
		);
		assert.notInclude(printed, 'OK');
	});

	test('throws on a truncated gzip output rather than gate on a short count', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 1000, null, baseline_of(1000));
		const gzip = to_fake_gzip({ [SIZE_WASM_DEFAULT]: 10 });
		const truncated: SpawnProgram = {
			...gzip,
			out_bytes: async (args, options) => ({
				...(await gzip.out_bytes(args, options)),
				truncated: true
			})
		};
		let error: unknown = null;
		try {
			await run_size_command(to_deps(dir, [], truncated), {
				mode: 'check',
				wasm: SIZE_WASM_DEFAULT
			});
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, Error);
		assert.isFalse(is_silent_error(error));
	});

	test('fails one byte over the baseline +2%', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 1021, null, baseline_of(1000));
		const printed: Array<string> = [];
		await assert_silent_failure(
			run_size_command(to_deps(dir, printed), { mode: 'check', wasm: SIZE_WASM_DEFAULT })
		);
		assert.strictEqual(printed.at(-1), 'ERR FAIL: raw size 1.0 KB exceeds ceiling 1.0 KB');
		assert.notInclude(printed, 'OK');
	});

	test('fails over the hard cap when the baseline +2% would allow it', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, SIZE_HARD_CAP_BYTES + 1, null, baseline_of(SIZE_HARD_CAP_BYTES));
		const printed: Array<string> = [];
		await assert_silent_failure(
			run_size_command(to_deps(dir, printed), { mode: 'check', wasm: SIZE_WASM_DEFAULT })
		);
		assert.deepEqual(printed.slice(-2), [
			'  baseline 1650.0 KB · ceiling 1650.0 KB (min of baseline +2% and hard cap 1650.0 KB)',
			'ERR FAIL: raw size 1650.0 KB exceeds ceiling 1650.0 KB'
		]);
	});

	test("reads the cap and tolerance the baseline records over the module's", async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(
			dir,
			1100,
			null,
			baseline_of(1000, { hard_cap_bytes: 2000, tolerance_pct: 10 })
		);
		const printed: Array<string> = [];
		await run_size_command(to_deps(dir, printed), { mode: 'check', wasm: SIZE_WASM_DEFAULT });
		assert.include(
			printed,
			'  baseline 1.0 KB · ceiling 1.1 KB (min of baseline +10% and hard cap 2.0 KB)'
		);
		assert.strictEqual(printed.at(-1), 'OK');
	});

	test('fails naming the update script when there is no baseline', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 1000, null);
		const printed: Array<string> = [];
		await assert_silent_failure(
			run_size_command(to_deps(dir, printed), { mode: 'check', wasm: SIZE_WASM_DEFAULT })
		);
		assert.strictEqual(
			printed.at(-1),
			'ERR no baseline at benches/size_baseline.json — run `npm run size:update`'
		);
	});

	test('gates the wasm it is pointed at, through gzip -c in the checkout', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 1000, null, baseline_of(1000));
		const shipped = 'crates/pglet_wasm/pkg-npm/pglet_wasm_bg.wasm';
		await dir.mkdir('crates/pglet_wasm/pkg-npm');
		await dir.write(shipped, new Uint8Array(5000));
		const calls: Array<{ args: ReadonlyArray<string>; cwd: string | undefined }> = [];
		const printed: Array<string> = [];
		await assert_silent_failure(
			run_size_command(to_deps(dir, printed, to_fake_gzip({ [shipped]: 1 }, calls)), {
				mode: 'check',
				wasm: shipped
			})
		);
		assert.deepEqual(calls, [{ args: ['-c', shipped], cwd: dir.path }]);
		assert.strictEqual(printed.at(-1), 'ERR FAIL: raw size 4.9 KB exceeds ceiling 1.0 KB');
	});

	test('throws when gzip fails rather than gate on a figure it did not measure', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 1000, null, baseline_of(1000));
		const printed: Array<string> = [];
		let error: unknown = null;
		try {
			await run_size_command(to_deps(dir, printed, to_fake_gzip({})), {
				mode: 'check',
				wasm: SIZE_WASM_DEFAULT
			});
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, Error);
		assert.isFalse(is_silent_error(error));
		assert.strictEqual((error as Error).message, 'gzip failed: gzip: no such file');
		assert.notInclude(printed, 'OK');
	});
});

describe('run_size_command update', () => {
	test('records this build, the native binary beside it, stamped and capped from the module', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 3000, 5000, baseline_of(1, { hard_cap_bytes: 7 }));
		const printed: Array<string> = [];
		await run_size_command(to_deps(dir, printed), { mode: 'update', wasm: SIZE_WASM_DEFAULT });
		assert.strictEqual(
			await dir.read_text('benches/size_baseline.json'),
			JSON.stringify(
				{
					timestamp: '2026-01-02T03:04:05.000Z',
					git_commit: 'abc1234',
					hard_cap_bytes: SIZE_HARD_CAP_BYTES,
					tolerance_pct: 2,
					artifacts: [
						{ label: 'pglet_wasm_bg.wasm', kind: 'wasm', bytes: 3000, gzip_bytes: 2048 },
						{ label: 'pglet_server', kind: 'native', bytes: 5000, gzip_bytes: 4096 }
					]
				},
				null,
				'\t'
			) + '\n'
		);
		assert.deepEqual(printed, [
			'baseline updated: pglet_wasm_bg.wasm 2.9 KB raw / 2.0 KB gzip',
			'  + pglet_server 4.9 KB raw / 4.0 KB gzip (reported, not gated)'
		]);
	});

	test('records the wasm alone without a native binary, unknown without git', async () => {
		await using dir = await fs.temp_dir('pglet_size_gate_');
		await to_checkout(dir, 3000, null);
		const printed: Array<string> = [];
		await run_size_command(
			{ ...to_deps(dir, printed), git: null },
			{
				mode: 'update',
				wasm: SIZE_WASM_DEFAULT
			}
		);
		const written = JSON.parse(await dir.read_text('benches/size_baseline.json'));
		assert.strictEqual(written.git_commit, 'unknown');
		assert.deepEqual(
			written.artifacts.map((a: { label: string }) => a.label),
			['pglet_wasm_bg.wasm']
		);
		assert.deepEqual(printed, [
			'note: native binary target/release/pglet_server not built — skipping (reported, not gated)',
			'baseline updated: pglet_wasm_bg.wasm 2.9 KB raw / 2.0 KB gzip'
		]);
	});
});
