/// <reference types="node" />

import { fs, type FsDir } from '@fuzdev/gro/fs.ts';
import { is_silent_error } from '@fuzdev/gro/task.ts';
import { assert, describe, test } from 'vitest';

import {
	MATRIX_NPM_PACKAGE_DIR,
	MATRIX_RELEASE_DIR,
	run_matrix_env_command
} from '#lib/matrix_env.ts';

const run = async (setup: (checkout: FsDir) => Promise<void>) => {
	await using checkout = await fs.temp_dir('pglet_matrix_');
	await setup(checkout);
	const stdout: Array<string> = [];
	const stderr: Array<string> = [];
	let failed = false;
	try {
		await run_matrix_env_command({
			checkout_path: checkout.path,
			open_release_dir: async () =>
				(await fs.open_dir(`${checkout.path}/${MATRIX_RELEASE_DIR}`)).readonly(),
			open_npm_package_dir: async () =>
				(await fs.open_dir(`${checkout.path}/${MATRIX_NPM_PACKAGE_DIR}`)).readonly(),
			print: (text) => stdout.push(text),
			print_error: (text) => stderr.push(text)
		});
	} catch (error) {
		if (!is_silent_error(error)) throw error;
		failed = true;
	}
	return { stdout, stderr, failed, root: checkout.path };
};

const build = async (checkout: FsDir, files: Array<string>): Promise<void> => {
	for (const file of files) {
		await checkout.mkdir(file.slice(0, file.lastIndexOf('/')), { recursive: true });
		await checkout.write(file, '');
	}
};

const SERVER = `${MATRIX_RELEASE_DIR}/pglet_server`;
const WASM = `${MATRIX_NPM_PACKAGE_DIR}/pglet_wasm_bg.wasm`;
const INDEX = `${MATRIX_NPM_PACKAGE_DIR}/index.js`;

describe('run_matrix_env_command', () => {
	test('prints only comments and exports naming the built artifacts', async () => {
		const { stdout, stderr, failed, root } = await run((c) => build(c, [SERVER, WASM, INDEX]));
		assert.isFalse(failed);
		assert.deepEqual(stderr, []);
		assert.deepEqual(stdout, [
			'# pglet artifacts for the fuz_app `db` matrix — export these (bash), then run',
			'#   npx vitest run --project db        (in your fuz_app checkout; add TEST_DATABASE_URL for the pg leg)',
			`export PGLET_SERVER_BIN=${root}/target/release/pglet_server`,
			'# the wasm leg imports @fuzdev/pglet-wasm; to run this build, in your fuz_app checkout:',
			`#   rm -rf node_modules/@fuzdev/pglet-wasm && ln -s ${root}/crates/pglet_wasm/pkg-npm node_modules/@fuzdev/pglet-wasm`
		]);
		for (const line of stdout) assert.match(line, /^(#|export )/);
	});

	test('names every missing artifact, prints nothing to stdout, and fails', async () => {
		const none = await run(async () => {});
		assert.isTrue(none.failed);
		assert.deepEqual(none.stdout, []);
		assert.deepEqual(none.stderr, [
			`error: pglet matrix artifact(s) not built:\n  ${none.root}/target/release/pglet_server\n  ${none.root}/crates/pglet_wasm/pkg-npm/pglet_wasm_bg.wasm\n  ${none.root}/crates/pglet_wasm/pkg-npm/index.js (run: npm run build:npm)\nrun: npm run matrix:artifacts`
		]);
	});

	test('a raw wasm-pack dir without the entry is not the package', async () => {
		const raw = await run((c) => build(c, [SERVER, WASM]));
		assert.isTrue(raw.failed);
		assert.deepEqual(raw.stderr, [
			`error: pglet matrix artifact(s) not built:\n  ${raw.root}/crates/pglet_wasm/pkg-npm/index.js (run: npm run build:npm)\nrun: npm run matrix:artifacts`
		]);
	});

	test('a file where the release dir should be holds no server', async () => {
		const result = await run(async (c) => {
			await build(c, [WASM, INDEX]);
			await c.write('target', 'not a dir');
		});
		assert.isTrue(result.failed);
		assert.include(result.stderr[0]!, `${result.root}/target/release/pglet_server\n`);
	});

	test('an error other than a missing dir is not read as a missing artifact', async () => {
		let error: unknown;
		await run_matrix_env_command({
			checkout_path: '/nowhere',
			open_release_dir: async () => {
				throw new Error('permission denied, say');
			},
			open_npm_package_dir: async () => {
				throw new Error('unreached');
			},
			print: () => {},
			print_error: () => {}
		}).catch((e: unknown) => (error = e));
		assert(error instanceof Error);
		assert.strictEqual(error.message, 'permission denied, say');
	});

	test('a symlinked target dir is followed', async () => {
		await using elsewhere = await fs.temp_dir('pglet_target_');
		await build(elsewhere, ['release/pglet_server']);
		const result = await run(async (c) => {
			await build(c, [WASM, INDEX]);
			await fs_symlink(elsewhere.path, `${c.path}/target`);
		});
		assert.isFalse(result.failed, result.stderr.join('\n'));
	});
});

// gro's fs has no symlink op; the test makes its one link through node
const fs_symlink = async (target: string, path: string): Promise<void> => {
	const { symlink } = await import('node:fs/promises');
	await symlink(target, path);
};
