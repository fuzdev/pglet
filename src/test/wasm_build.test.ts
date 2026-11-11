/// <reference types="node" />

import { utimes } from 'node:fs/promises';

import { fs, type FsDir } from '@fuzdev/gro/fs.ts';
import type { SpawnCallOptions, SpawnProgram, SpawnResult } from '@fuzdev/gro/spawn.ts';
import { assert, describe, test } from 'vitest';

import {
	ensure_wasm_package,
	to_wasm_pack_args,
	WASM_DEFAULT,
	WASM_OPFS,
	WASM_PACK_PASS_ENV,
	WASM_SOURCE_DIRS
} from '#lib/wasm_build.ts';

interface Call {
	args: ReadonlyArray<string>;
	cwd: string | undefined;
}

/** A `wasm-pack` that records its calls and ends with `result`; only `run` is reached. */
const to_fake_wasm_pack = (
	result: SpawnResult = { kind: 'exited', code: 0 }
): { program: SpawnProgram; calls: Array<Call> } => {
	const calls: Array<Call> = [];
	const unreached = () => {
		throw new Error('only run is called');
	};
	return {
		calls,
		program: {
			name: 'wasm-pack',
			path: '/fake/wasm-pack',
			run: async (args: ReadonlyArray<string>, options?: SpawnCallOptions) => {
				calls.push({ args, cwd: options?.cwd?.path });
				return result;
			},
			out: unreached,
			out_bytes: unreached,
			start: unreached
		}
	};
};

/** A checkout whose every wasm source dir holds one `.rs` file, all at `rs_seconds`. */
const to_checkout = async (dir: FsDir, rs_seconds: number): Promise<void> => {
	for (const src of WASM_SOURCE_DIRS) {
		await dir.mkdir(`crates/${src}/nested`, { recursive: true });
		await dir.write(`crates/${src}/nested/lib.rs`, '');
		await dir.write(`crates/${src}/notes.md`, '');
		await utimes(`${dir.path}/crates/${src}/nested/lib.rs`, rs_seconds, rs_seconds);
		await utimes(`${dir.path}/crates/${src}/notes.md`, rs_seconds + 1000, rs_seconds + 1000);
	}
};

const build = async (dir: FsDir, out_dir: string, seconds: number): Promise<void> => {
	await dir.mkdir(`crates/pglet_wasm/${out_dir}`, { recursive: true });
	await dir.write(`crates/pglet_wasm/${out_dir}/pglet_wasm.js`, '');
	await utimes(`${dir.path}/crates/pglet_wasm/${out_dir}/pglet_wasm.js`, seconds, seconds);
};

const to_deps = (dir: FsDir, program: SpawnProgram, printed: Array<string>) => ({
	crates: dir.dir('crates').readonly(),
	checkout: dir.readonly(),
	wasm_pack: program,
	print: (text: string) => printed.push(text)
});

describe('ensure_wasm_package', () => {
	test('builds a missing package from the checkout, with its out dir and features', async () => {
		await using dir = await fs.temp_dir('pglet_wasm_build_');
		await to_checkout(dir, 1_000_000);
		const { program, calls } = to_fake_wasm_pack();
		const printed: Array<string> = [];
		await ensure_wasm_package(to_deps(dir, program, printed), WASM_OPFS);
		assert.deepEqual(calls, [
			{
				args: [
					'build',
					'crates/pglet_wasm',
					'--target',
					'web',
					'--out-dir',
					'pkg-opfs',
					'--features',
					'opfs'
				],
				cwd: dir.path
			}
		]);
		assert.include(printed[0], 'pkg-opfs not found, building');
	});

	test('leaves a package alone that is as new as every .rs file, and rebuilds an older one', async () => {
		await using dir = await fs.temp_dir('pglet_wasm_build_');
		await to_checkout(dir, 1_000_000);
		const { program, calls } = to_fake_wasm_pack();
		const printed: Array<string> = [];
		// newer than every .rs, older than the .md beside them, which doesn't count
		await build(dir, 'pkg', 1_000_000);
		await ensure_wasm_package(to_deps(dir, program, printed), WASM_DEFAULT);
		assert.deepEqual(calls, []);
		assert.deepEqual(printed, ['[build] WASM package pkg already exists']);

		await utimes(`${dir.path}/crates/pglet_storage/src/nested/lib.rs`, 1_000_001, 1_000_001);
		await ensure_wasm_package(to_deps(dir, program, printed), WASM_DEFAULT);
		assert.deepEqual(
			calls.map((c) => c.args),
			[to_wasm_pack_args(WASM_DEFAULT)]
		);
		assert.include(printed[1], 'pkg stale, building');
	});

	test('a failed build throws, naming the script that makes that package', async () => {
		await using dir = await fs.temp_dir('pglet_wasm_build_');
		await to_checkout(dir, 1_000_000);
		for (const [result, pkg, script] of [
			[{ kind: 'exited', code: 1 }, WASM_DEFAULT, 'npm run wasm:build'],
			[{ kind: 'signaled', signal: 'SIGKILL' }, WASM_OPFS, 'npm run wasm:build:opfs']
		] as const) {
			const { program } = to_fake_wasm_pack(result);
			let error: unknown;
			await ensure_wasm_package(to_deps(dir, program, []), pkg).catch((e: unknown) => (error = e));
			assert.instanceOf(error, Error);
			assert.include((error as Error).message, `Fix: ${script}`);
		}
	});
});

describe('WASM_PACK_PASS_ENV', () => {
	/** Whether a `pass_env` entry passes `name`: an exact name, or a prefix ending in `*`. */
	const passes = (name: string): boolean =>
		WASM_PACK_PASS_ENV.some((entry) =>
			entry.endsWith('*') ? name.startsWith(entry.slice(0, -1)) : name === entry
		);

	test('passes what cargo, rustc, rustup, wasm-pack and wasm-bindgen read for a build', () => {
		for (const name of [
			'RUSTFLAGS',
			'CARGO_ENCODED_RUSTFLAGS',
			'RUSTC',
			'RUSTC_WRAPPER',
			'RUSTC_WORKSPACE_WRAPPER',
			'RUSTC_BOOTSTRAP',
			'CARGO_HOME',
			'CARGO_TARGET_DIR',
			'CARGO_INCREMENTAL',
			'CARGO_PROFILE_RELEASE_OPT_LEVEL',
			'RUSTUP_TOOLCHAIN',
			'HTTPS_PROXY',
			'https_proxy',
			'http_proxy',
			'HTTP_TIMEOUT',
			'WASM_PACK_CACHE',
			'WASM_BINDGEN_EXTERNREF'
		]) {
			assert.ok(passes(name), name);
		}
	});

	test('leaves out what only `cargo doc` and `cargo fmt` read, and the rest of the host', () => {
		for (const name of [
			'RUSTDOC',
			'RUSTDOCFLAGS',
			'RUSTFMT',
			'BROWSER',
			'NPM_TOKEN',
			'SSH_AUTH_SOCK'
		]) {
			assert.ok(!passes(name), name);
		}
	});
});
