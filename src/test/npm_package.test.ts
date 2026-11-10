import { fs } from '@fuzdev/gro/fs.ts';
import { assert, describe, test } from 'vitest';

import { patch_npm_package, to_npm_package_json } from '#lib/npm_package.ts';

// what the patch wrote over wasm-pack's raw output, captured once from the build it replaced
const fixtures = fs.project.dir('src/test/fixtures/npm_package').readonly();
const WASM_PACK_JSON = await fixtures.read_text('wasm_pack_package.json.txt');
const EXPECTED_JSON = await fixtures.read_text('expected_package.json.txt');
const EXPECTED_INDEX = await fixtures.read_text('expected_index.js.txt');
const EXPECTED_BROWSER = await fixtures.read_text('expected_browser.js.txt');

const COPIED = ['db.js', 'coerce.js', 'index.d.ts', 'kysely.js', 'kysely.d.ts'];

/** A package dir as wasm-pack leaves it, a source dir, and a root, each a temp dir. */
const setup = async (options: { license?: boolean; readme?: boolean } = {}) => {
	const { license = true, readme = true } = options;
	const pkg = await fs.temp_dir('pglet_pkg_');
	const npm_src = await fs.temp_dir('pglet_npm_src_');
	const root = await fs.temp_dir('pglet_root_');
	await pkg.write('package.json', WASM_PACK_JSON);
	await pkg.write('pglet_wasm.js', 'raw glue\n');
	for (const name of COPIED) await npm_src.write(name, `source of ${name}\n`);
	if (readme) await npm_src.write('README.md', 'the readme\n');
	if (license) await root.write('LICENSE', 'the license\n');
	return {
		pkg,
		npm_src,
		root,
		[Symbol.asyncDispose]: async () => {
			await pkg[Symbol.asyncDispose]();
			await npm_src[Symbol.asyncDispose]();
			await root[Symbol.asyncDispose]();
		}
	};
};

describe('patch_npm_package', () => {
	test('writes the entries, copies the layer, LICENSE and README, and rewrites package.json', async () => {
		await using dirs = await setup();
		await patch_npm_package(dirs);
		const { pkg } = dirs;
		assert.strictEqual(await pkg.read_text('index.js'), EXPECTED_INDEX);
		assert.strictEqual(await pkg.read_text('browser.js'), EXPECTED_BROWSER);
		assert.strictEqual(await pkg.read_text('package.json'), EXPECTED_JSON);
		for (const name of COPIED) {
			assert.strictEqual(await pkg.read_text(name), `source of ${name}\n`, name);
		}
		assert.strictEqual(await pkg.read_text('LICENSE'), 'the license\n');
		assert.strictEqual(await pkg.read_text('README.md'), 'the readme\n');
		// wasm-pack's own output is left alone
		assert.strictEqual(await pkg.read_text('pglet_wasm.js'), 'raw glue\n');
	});

	test('refuses a checkout without a LICENSE', async () => {
		await using dirs = await setup({ license: false });
		let error: unknown;
		await patch_npm_package(dirs).catch((e: unknown) => (error = e));
		assert(error instanceof Error);
		assert.strictEqual(error.message, 'patch_npm_package: no LICENSE at the repo root');
		assert.strictEqual(await dirs.pkg.read_text('package.json'), WASM_PACK_JSON);
	});

	test('refuses a source dir without a README', async () => {
		await using dirs = await setup({ readme: false });
		let error: unknown;
		await patch_npm_package(dirs).catch((e: unknown) => (error = e));
		assert(error instanceof Error);
		assert.strictEqual(error.message, 'patch_npm_package: no crates/pglet_wasm/npm/README.md');
		assert.strictEqual(await dirs.pkg.read_text('package.json'), WASM_PACK_JSON);
	});
});

describe('to_npm_package_json', () => {
	test("keeps wasm-pack's other fields in place and drops `module`", () => {
		const json = JSON.parse(
			to_npm_package_json(
				JSON.stringify({ name: 'pglet_wasm', version: '1.2.3', module: 'x.js', extra: true })
			)
		);
		assert.deepEqual(Object.keys(json).slice(0, 4), ['name', 'version', 'extra', 'description']);
		assert.strictEqual(json.name, '@fuzdev/pglet-wasm');
		assert.strictEqual(json.version, '1.2.3');
		assert.isTrue(json.extra);
		assert.notProperty(json, 'module');
	});
});
