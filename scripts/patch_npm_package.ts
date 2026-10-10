/**
 * Assemble the publishable @fuzdev/pglet-wasm npm package from wasm-pack's `pkg/` output.
 *
 * Run right after `wasm-pack build crates/pglet_wasm --target web` (see `deno task
 * build:npm`). Operates in place on the given pkg dir (default `crates/pglet_wasm/pkg`),
 * which is gitignored — nothing here is tracked; the tracked source is `crates/pglet_wasm/
 * npm/*` (the hand-written ergonomic layer) + this script.
 *
 * Follows the tsv/blake3 pattern:
 * the entry files (index.js/browser.js) are GENERATED here; the ergonomic layer
 * (db.js/coerce.js) + hand-written index.d.ts are COPIED verbatim (dep-free, no bundler);
 * package.json gets the `.` and `./kysely` exports, `sideEffects: ['./index.js']` (protects the
 * Node entry's top-level `initSync` from tree-shaking — tsv's fix), the registry identity from
 * `scripts/npm_metadata.ts`, and `main`/`types` pointing at the Node facade for resolvers that
 * predate `exports` (`exports` stays authoritative wherever it is read).
 *
 *   deno run --allow-read --allow-write scripts/patch_npm_package.ts [pkg-dir]
 */

import {NPM_SHARED_METADATA} from './npm_metadata.ts';

const pkg_dir = Deno.args[0] ?? 'crates/pglet_wasm/pkg';
const npm_src = 'crates/pglet_wasm/npm';

const NAME = NPM_SHARED_METADATA.name;
const RAW_JS = 'pglet_wasm.js';
const RAW_DTS = 'pglet_wasm.d.ts';
const RAW_WASM = 'pglet_wasm_bg.wasm';
// The crate's `inline_js` snippets (a result field's object literal, `pglet_wasm`'s `result.rs`),
// which the raw glue imports by relative path — shipped with it, or the package cannot load.
const RAW_SNIPPETS = 'snippets/';

// The Node entry: sync `readFileSync` + `initSync` at import time (zero-config). It exports the
// ergonomic factory only — the raw `PgletBtree` binding and `wrap_pglet_db` are the adapter's
// internals, not the package's API. `init` / `init_sync` are the browser entry's, answered here
// as no-ops so code written for both entries runs on either.
const index_js = `import { readFileSync } from 'node:fs';
import { initSync, PgletBtree } from './${RAW_JS}';
import { wrap_pglet_db } from './db.js';

const wasm = readFileSync(new URL('./${RAW_WASM}', import.meta.url));
initSync({ module: wasm });

export async function init(_options) {}
export function init_sync(_options) {}
export const create_pglet_db = (options) => wrap_pglet_db(new PgletBtree(), options);
`;

// The browser/default entry: async `init()` guarded by `_ready`; `create_pglet_db` throws
// until initialized. Bundlers pick this via the `default` condition and resolve the
// `new URL('…_bg.wasm', import.meta.url)` the raw async init uses when no `wasm` is given.
// `init` takes the package's own `{wasm}` option and hands wasm-bindgen its
// `{module_or_path}` shape, so the generated glue's argument conventions are not the API.
const browser_js = `import { default as _init, initSync, PgletBtree } from './${RAW_JS}';
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

const at = (name: string) => `${pkg_dir}/${name}`;
const copy = async (from: string, to: string): Promise<boolean> => {
	try {
		await Deno.copyFile(from, at(to));
		return true;
	} catch {
		return false;
	}
};

// 1. Generated entries.
await Deno.writeTextFile(at('index.js'), index_js);
await Deno.writeTextFile(at('browser.js'), browser_js);

// 2. Copied ergonomic layer + hand-written types.
await Deno.copyFile(`${npm_src}/db.js`, at('db.js'));
await Deno.copyFile(`${npm_src}/coerce.js`, at('coerce.js'));
await Deno.copyFile(`${npm_src}/index.d.ts`, at('index.d.ts'));
// The kysely dialect (optional peer dep; the `./kysely` subpath).
await Deno.copyFile(`${npm_src}/kysely.js`, at('kysely.js'));
await Deno.copyFile(`${npm_src}/kysely.d.ts`, at('kysely.d.ts'));

// 3. LICENSE / README: a package without either is refused rather than assembled.
if (!(await copy('LICENSE', 'LICENSE'))) throw new Error('patch_npm_package: no LICENSE at the repo root');
if (!(await copy(`${npm_src}/README.md`, 'README.md'))) {
	throw new Error(`patch_npm_package: no ${npm_src}/README.md`);
}

// 4. Patch package.json.
const pkg = JSON.parse(await Deno.readTextFile(at('package.json')));
pkg.description = 'fast and small embeddable postgres for speeding up your tests';
pkg.type = 'module';
pkg.exports = {
	'./package.json': './package.json',
	'.': {types: './index.d.ts', node: './index.js', default: './browser.js'},
	// ORM subpath — pglet-specific (no blake3/tsv precedent); a real kysely Dialect.
	'./kysely': {types: './kysely.d.ts', default: './kysely.js'},
};
// wasm-pack emits `sideEffects: ["./snippets/*"]`, declaring index.js side-effect-free —
// but its top-level readFileSync + initSync IS the side effect. Without this a tree-shaking
// bundler on the `node` condition can bypass the re-export facade and skip init (tsv's fix).
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
	'LICENSE',
];
// kysely is an OPTIONAL peer dep — only the `./kysely` subpath imports it; the core adapter is dep-free.
pkg.peerDependencies = {kysely: '>=0.27 <1'};
pkg.peerDependenciesMeta = {kysely: {optional: true}};
pkg.keywords = ['postgres', 'postgresql', 'pglet', 'wasm', 'webassembly', 'database', 'sql', 'test-fixtures', 'embedded'];
Object.assign(pkg, NPM_SHARED_METADATA);
// wasm-pack's web-target fields point at the raw glue; `exports` supersedes them wherever it is
// read, so they are replaced with the Node facade for resolvers that predate `exports` (a
// `moduleResolution: node10` consumer, tooling that reads `main` directly) — tsv's shape.
pkg.main = 'index.js';
pkg.types = 'index.d.ts';
delete pkg.module;

await Deno.writeTextFile(at('package.json'), JSON.stringify(pkg, null, '\t') + '\n');

console.log(`patched ${pkg_dir} → ${NAME}`);
