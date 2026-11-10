/**
 * Package-shape unit test for the assembled pkg-npm (tsv's `test_npm.ts` pattern): the
 * `sideEffects`/`exports`/`files` contract, and that every referenced file actually exists.
 * A regression here (e.g. the `sideEffects: ['./index.js']` override lost, so a bundler
 * tree-shakes away the Node entry's top-level `initSync`) is a silent-breakage class, so
 * it's asserted, not assumed. Runs after `build:npm` (see `npm run test:npm`).
 */
import {readFile, stat} from 'node:fs/promises';
import {assert, test} from 'vitest';
import {assert_equal} from '../lib/assert.ts';
import {NPM_SHARED_METADATA} from '#lib/npm_metadata.ts';

const PKG = new URL('../../crates/pglet_wasm/pkg-npm/', import.meta.url);
const exists = async (rel: string): Promise<boolean> => {
	try {
		await stat(new URL(rel, PKG));
		return true;
	} catch {
		return false;
	}
};

const pkg = JSON.parse(await readFile(new URL('package.json', PKG), 'utf8'));

test('identity + module type', () => {
	assert_equal(pkg.name, '@fuzdev/pglet-wasm');
	assert_equal(pkg.type, 'module');
});

test('every shared registry field is the metadata module\'s, value for value', () => {
	for (const [key, value] of Object.entries(NPM_SHARED_METADATA)) {
		assert_equal(pkg[key], value, `package.json ${key}`);
	}
	// the two named in the release plan, spelled out so a metadata edit that drops either is red here too
	assert_equal(pkg.engines, {node: '>=22'});
	assert_equal(pkg.publishConfig, {access: 'public'});
});

test('sideEffects marks the Node entry (survives tree-shaking)', () => {
	assert_equal(pkg.sideEffects, ['./index.js']);
});

test('exports: `.` primary + `./kysely` subpath, all resolving; main/types name the `.` export\'s files', async () => {
	assert_equal(Object.keys(pkg.exports).sort(), ['.', './kysely', './package.json']);
	assert_equal(Object.keys(pkg.exports['.']), ['types', 'node', 'default']);
	for (const key of ['.', './kysely']) {
		for (const [cond, target] of Object.entries(pkg.exports[key])) {
			assert(await exists(target as string), `exports['${key}'].${cond} → missing ${target}`);
		}
	}
	// for resolvers that predate `exports`: the same files the `.` export's `node` and `types`
	// conditions name, so the two routes cannot load different code
	assert_equal(`./${pkg.main}`, pkg.exports['.'].node);
	assert_equal(`./${pkg.types}`, pkg.exports['.'].types);
	assert(!('module' in pkg), 'wasm-pack\'s `module` (the raw glue) must be deleted');
});

test('the shipped wasm carries no OPFS persistence layer (the crate\'s opt-in `opfs` feature)', async () => {
	// The raw glue and its declarations ship (`files[]`), so a class in the wasm is a class in
	// the package; `PgletBtree` is the control.
	for (const f of ['pglet_wasm.js', 'pglet_wasm.d.ts']) {
		const text = await readFile(new URL(f, PKG), 'utf8');
		assert(text.includes('class PgletBtree '), `${f}: no PgletBtree`);
		assert(!text.includes('PgletBtreePersistent'), `${f}: names PgletBtreePersistent`);
	}
	const wasm = await WebAssembly.compile(await readFile(new URL('pglet_wasm_bg.wasm', PKG)));
	const exports = WebAssembly.Module.exports(wasm).map((e) => e.name);
	const imports = WebAssembly.Module.imports(wasm).map((e) => e.name);
	assert(exports.includes('pgletbtree_new'), exports.join(' '));
	assert_equal(exports.filter((n) => /persistent/i.test(n)), []);
	assert_equal(imports.filter((n) => /SyncAccessHandle|getDirectory|getFileHandle/.test(n)), []);
});

test('every files[] entry exists', async () => {
	assert(Array.isArray(pkg.files) && pkg.files.length > 0);
	for (const f of pkg.files) {
		assert(await exists(f), `files[] entry missing: ${f}`);
	}
});

test('the dep-free ergonomic layer + generated entries are present', async () => {
	for (const f of ['index.js', 'browser.js', 'db.js', 'coerce.js', 'index.d.ts', 'pglet_wasm_bg.wasm']) {
		assert(await exists(f), `missing ${f}`);
		assert(pkg.files.includes(f), `${f} not in files[]`);
	}
});
