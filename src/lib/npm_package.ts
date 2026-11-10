/**
 * Assembles the publishable `@fuzdev/pglet-wasm` npm package from wasm-pack's
 * output, in place, the body of `npm run build:npm`'s last step. The package
 * dir is gitignored, so nothing here is tracked; the tracked source is
 * `crates/pglet_wasm/npm/` (the hand-written ergonomic layer) and this module.
 *
 * The entry files (`index.js`, `browser.js`) are generated here; the ergonomic
 * layer (`db.js`, `coerce.js`, the kysely dialect) and the hand-written
 * `index.d.ts` are copied verbatim (dep-free, no bundler). `package.json` gets
 * the `.` and `./kysely` exports, `sideEffects: ['./index.js']` (protecting the
 * Node entry's top-level `initSync` from tree-shaking), the registry identity
 * from `npm_metadata.ts`, and `main`/`types` pointing at the Node facade for
 * resolvers that predate `exports` (`exports` stays authoritative wherever it
 * is read).
 *
 * @module
 */

import type { FsDir, FsReadDir } from '@fuzdev/gro/fs.ts';

import { NPM_SHARED_METADATA } from './npm_metadata.ts';

/** The package dir `build:npm` builds and this patches, beneath the checkout. */
export const NPM_PACKAGE_DIR = 'crates/pglet_wasm/pkg-npm';

/** The tracked source of the copied files, beneath the checkout. */
export const NPM_PACKAGE_SOURCE_DIR = 'crates/pglet_wasm/npm';

const NAME = NPM_SHARED_METADATA.name;
const RAW_JS = 'pglet_wasm.js';
const RAW_DTS = 'pglet_wasm.d.ts';
const RAW_WASM = 'pglet_wasm_bg.wasm';
// the crate's `inline_js` snippets (a result field's object literal, `pglet_wasm`'s `result.rs`),
// which the raw glue imports by relative path: shipped with it, or the package can't load
const RAW_SNIPPETS = 'snippets/';

// The Node entry: sync `readFileSync` + `initSync` at import time (zero-config). It exports the
// ergonomic factory only — the raw `PgletBtree` binding and `wrap_pglet_db` are the adapter's
// internals, not the package's API. `init` / `init_sync` are the browser entry's, answered here
// as no-ops so code written for both entries runs on either.
//
// It is also the entry every server runtime resolves (Node, Deno and Bun all match the `node`
// export condition), so it is the one that hands the adapter an `AsyncLocalStorage`: that is what
// lets `transaction()` queue a concurrent top-level call and refuse only a nested one. It is ONE
// store for every instance — on Node 22's async-hooks implementation each store ever entered stays
// registered and costs on every promise, so a store per instance slows a long suite with each
// instance it creates (`db.js` tells instances apart by an owner token). The browser entry must
// never import `node:async_hooks`, so it passes none.
const INDEX_JS = `import { readFileSync } from 'node:fs';
import { AsyncLocalStorage } from 'node:async_hooks';
import { initSync, PgletBtree } from './${RAW_JS}';
import { wrap_pglet_db } from './db.js';

const wasm = readFileSync(new URL('./${RAW_WASM}', import.meta.url));
initSync({ module: wasm });

const context_store = new AsyncLocalStorage();

export async function init(_options) {}
export function init_sync(_options) {}
export const create_pglet_db = (options) => wrap_pglet_db(new PgletBtree(), options, context_store);
`;

// The browser/default entry: async `init()` guarded by `_ready`; `create_pglet_db` throws
// until initialized. Bundlers pick this via the `default` condition and resolve the
// `new URL('…_bg.wasm', import.meta.url)` the raw async init uses when no `wasm` is given.
// `init` takes the package's own `{wasm}` option and hands wasm-bindgen its
// `{module_or_path}` shape, so the generated glue's argument conventions are not the API.
const BROWSER_JS = `import { default as _init, initSync, PgletBtree } from './${RAW_JS}';
import { wrap_pglet_db } from './db.js';

let _ready = false;
// The one in-flight initialization: wasm-bindgen's own init only checks for a finished module, so a
// second load started before the first finished would replace the module under a live instance.
let _loading = null;
function _check() {
	if (!_ready) throw new Error('${NAME}: WASM not initialized. Call \`await init()\` first.');
}
export function init(options) {
	if (_ready) return Promise.resolve();
	_loading ??= _init(options?.wasm === undefined ? undefined : { module_or_path: options.wasm }).then(
		() => {
			_ready = true;
		},
		(e) => {
			_loading = null;
			throw e;
		},
	);
	return _loading;
}
export function init_sync(options) {
	if (_ready) return;
	if (_loading) throw new Error('${NAME}: init_sync() was called while init() is loading the module; await init() instead');
	if (options?.wasm === undefined) {
		throw new Error('${NAME}: init_sync() needs {wasm}: the module bytes or a compiled WebAssembly.Module');
	}
	initSync({ module: options.wasm });
	_ready = true;
}
export function create_pglet_db(options) {
	_check();
	return wrap_pglet_db(new PgletBtree(), options);
}
`;

/** The files copied verbatim from `NPM_PACKAGE_SOURCE_DIR`, in order. */
const COPIED_SOURCES = ['db.js', 'coerce.js', 'index.d.ts', 'kysely.js', 'kysely.d.ts'] as const;

/** What the patch reads and writes. */
export interface NpmPackagePatchDeps {
	/** The package dir wasm-pack built, patched in place. */
	pkg: FsDir;
	/** The tracked ergonomic layer and the package's README, `NPM_PACKAGE_SOURCE_DIR`. */
	npm_src: FsReadDir;
	/** The checkout's root, for its `LICENSE`. */
	root: FsReadDir;
}

/**
 * Assembles the package in `deps.pkg`: writes the generated entries, copies
 * the ergonomic layer, the `LICENSE` and the README, then rewrites
 * `package.json`. A package without a `LICENSE` or README is refused rather
 * than assembled, after the entries and the layer are written.
 *
 * @throws Error - when the checkout has no `LICENSE` or the source dir no `README.md`
 */
export const patch_npm_package = async (deps: NpmPackagePatchDeps): Promise<void> => {
	const { pkg, npm_src, root } = deps;

	// 1. generated entries
	await pkg.write('index.js', INDEX_JS);
	await pkg.write('browser.js', BROWSER_JS);

	// 2. the copied ergonomic layer, hand-written types, and the kysely dialect (an optional
	// peer dep, the `./kysely` subpath)
	for (const name of COPIED_SOURCES) await pkg.write(name, await npm_src.read(name));

	// 3. a package without a LICENSE or README is refused rather than assembled
	const license = await root.read_if_exists('LICENSE');
	if (license === null) throw new Error('patch_npm_package: no LICENSE at the repo root');
	await pkg.write('LICENSE', license);
	const readme = await npm_src.read_if_exists('README.md');
	if (readme === null) throw new Error(`patch_npm_package: no ${NPM_PACKAGE_SOURCE_DIR}/README.md`);
	await pkg.write('README.md', readme);

	// 4. package.json
	await pkg.write('package.json', to_npm_package_json(await pkg.read_text('package.json')));
};

/**
 * The published `package.json`, from the one wasm-pack wrote: every field
 * wasm-pack set is kept unless replaced here, in its place.
 */
export const to_npm_package_json = (wasm_pack_json: string): string => {
	const pkg = JSON.parse(wasm_pack_json);
	pkg.description = 'fast and small embeddable postgres for speeding up your tests';
	pkg.type = 'module';
	pkg.exports = {
		'./package.json': './package.json',
		'.': { types: './index.d.ts', node: './index.js', default: './browser.js' },
		// ORM subpath, a real kysely Dialect
		'./kysely': { types: './kysely.d.ts', default: './kysely.js' }
	};
	// wasm-pack emits `sideEffects: ["./snippets/*"]`, declaring index.js side-effect-free, but
	// its top-level readFileSync + initSync is the side effect: without this a tree-shaking
	// bundler on the `node` condition can bypass the re-export facade and skip init
	pkg.sideEffects = ['./index.js'];
	pkg.files = [
		'index.js',
		'index.d.ts',
		'browser.js',
		'db.js',
		'coerce.js',
		'kysely.js',
		'kysely.d.ts',
		RAW_JS,
		RAW_DTS,
		RAW_WASM,
		RAW_SNIPPETS,
		'README.md',
		'LICENSE'
	];
	// kysely is an optional peer dep: only the `./kysely` subpath imports it, the core adapter is dep-free
	pkg.peerDependencies = { kysely: '>=0.27 <1' };
	pkg.peerDependenciesMeta = { kysely: { optional: true } };
	pkg.keywords = [
		'postgres',
		'postgresql',
		'pglet',
		'wasm',
		'webassembly',
		'database',
		'sql',
		'test-fixtures',
		'embedded'
	];
	Object.assign(pkg, NPM_SHARED_METADATA);
	// wasm-pack's web-target fields point at the raw glue; `exports` supersedes them wherever
	// it is read, so they're replaced with the Node facade for resolvers that predate `exports`
	// (a `moduleResolution: node10` consumer, tooling that reads `main` directly)
	pkg.main = 'index.js';
	pkg.types = 'index.d.ts';
	delete pkg.module;
	return JSON.stringify(pkg, null, '\t') + '\n';
};
