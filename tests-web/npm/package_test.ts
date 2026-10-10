/**
 * Package-shape unit test for the assembled pkg-npm (tsv's `test_npm.ts` pattern): the
 * `sideEffects`/`exports`/`files` contract, and that every referenced file actually exists.
 * A regression here (e.g. the `sideEffects: ['./index.js']` override lost, so a bundler
 * tree-shakes away the Node entry's top-level `initSync`) is a silent-breakage class, so
 * it's asserted, not assumed. Runs after `build:npm` (see `deno task test:npm`).
 */
import {assert, assertEquals} from '@std/assert';
import {NPM_SHARED_METADATA} from '../../scripts/npm_metadata.ts';

const PKG = 'crates/pglet_wasm/pkg-npm';
const exists = async (rel: string): Promise<boolean> => {
	try {
		await Deno.stat(`${PKG}/${rel}`);
		return true;
	} catch {
		return false;
	}
};

const pkg = JSON.parse(await Deno.readTextFile(`${PKG}/package.json`));

Deno.test('identity + module type', () => {
	assertEquals(pkg.name, '@fuzdev/pglet-wasm');
	assertEquals(pkg.type, 'module');
});

Deno.test('every shared registry field is the metadata module\'s, value for value', () => {
	for (const [key, value] of Object.entries(NPM_SHARED_METADATA)) {
		assertEquals(pkg[key], value, `package.json ${key}`);
	}
	// the two named in the release plan, spelled out so a metadata edit that drops either is red here too
	assertEquals(pkg.engines, {node: '>=22'});
	assertEquals(pkg.publishConfig, {access: 'public'});
});

Deno.test('sideEffects marks the Node entry (survives tree-shaking)', () => {
	assertEquals(pkg.sideEffects, ['./index.js']);
});

Deno.test('exports: `.` primary + `./kysely` subpath, all resolving; main/types name the `.` export\'s files', async () => {
	assertEquals(Object.keys(pkg.exports).sort(), ['.', './kysely', './package.json']);
	assertEquals(Object.keys(pkg.exports['.']), ['types', 'node', 'default']);
	for (const key of ['.', './kysely']) {
		for (const [cond, target] of Object.entries(pkg.exports[key])) {
			assert(await exists(target as string), `exports['${key}'].${cond} → missing ${target}`);
		}
	}
	// for resolvers that predate `exports`: the same files the `.` export's `node` and `types`
	// conditions name, so the two routes cannot load different code
	assertEquals(`./${pkg.main}`, pkg.exports['.'].node);
	assertEquals(`./${pkg.types}`, pkg.exports['.'].types);
	assert(!('module' in pkg), 'wasm-pack\'s `module` (the raw glue) must be deleted');
});

Deno.test('the shipped wasm carries no OPFS persistence layer (the crate\'s opt-in `opfs` feature)', async () => {
	// The raw glue and its declarations ship (`files[]`), so a class in the wasm is a class in
	// the package; `PgletBtree` is the control.
	for (const f of ['pglet_wasm.js', 'pglet_wasm.d.ts']) {
		const text = await Deno.readTextFile(`${PKG}/${f}`);
		assert(text.includes('class PgletBtree '), `${f}: no PgletBtree`);
		assert(!text.includes('PgletBtreePersistent'), `${f}: names PgletBtreePersistent`);
	}
	const wasm = await WebAssembly.compile(await Deno.readFile(`${PKG}/pglet_wasm_bg.wasm`));
	const exports = WebAssembly.Module.exports(wasm).map((e) => e.name);
	const imports = WebAssembly.Module.imports(wasm).map((e) => e.name);
	assert(exports.includes('pgletbtree_new'), exports.join(' '));
	assertEquals(exports.filter((n) => /persistent/i.test(n)), []);
	assertEquals(imports.filter((n) => /SyncAccessHandle|getDirectory|getFileHandle/.test(n)), []);
});

Deno.test('every files[] entry exists', async () => {
	assert(Array.isArray(pkg.files) && pkg.files.length > 0);
	for (const f of pkg.files) {
		assert(await exists(f), `files[] entry missing: ${f}`);
	}
});

Deno.test('the dep-free ergonomic layer + generated entries are present', async () => {
	for (const f of ['index.js', 'browser.js', 'db.js', 'coerce.js', 'index.d.ts', 'pglet_wasm_bg.wasm']) {
		assert(await exists(f), `missing ${f}`);
		assert(pkg.files.includes(f), `${f} not in files[]`);
	}
});
