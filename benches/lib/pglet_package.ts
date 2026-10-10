/**
 * The pglet npm package a bench measures, and the one rule for whether it is current.
 *
 * Every bench that reads `crates/pglet_wasm/pkg-npm` — `wasm_compare.ts` loads it,
 * `size_compare.ts` weighs its wasm — refuses a missing or stale package through
 * `assert_pglet_package_current`, so "current" means the same thing for each figure
 * that comes out of it.
 *
 * **Cross-runtime on purpose**, like `results.ts`: `wasm_compare.ts` runs under Deno,
 * Node and Bun, so this uses `node:` builtins only.
 *
 * @module
 */

import {readdirSync, statSync} from 'node:fs';

const REPO = new URL('../../', import.meta.url);

/** The npm package `deno task build:npm` assembles. */
export const PGLET_PKG = new URL('crates/pglet_wasm/pkg-npm/', REPO);

/** The package files a `create_pglet_db()` consumer loads; the oldest of them dates the build. */
const PGLET_PKG_LOADED = ['index.js', 'db.js', 'coerce.js', 'pglet_wasm.js', 'pglet_wasm_bg.wasm'];

/** Directories under `crates/` that hold build output rather than build input: every wasm-pack
 * out-dir (`pkg`, `pkg-npm`, `pkg-opfs`, …) by its prefix, and these. */
const NOT_INPUT_DIRS = new Set(['node_modules', 'target']);
const is_not_input_dir = (name: string): boolean =>
	NOT_INPUT_DIRS.has(name) || name === 'pkg' || name.startsWith('pkg-');

/**
 * The newest file under `dir`, by mtime. Every file counts — a source the package
 * does not depend on only makes the check stricter, which costs a rebuild; a list
 * of the inputs that do count is what would go stale.
 */
const newest_file = (dir: URL, newest: {path: string; mtime: number}): void => {
	for (const entry of readdirSync(dir, {withFileTypes: true})) {
		if (entry.isDirectory()) {
			if (!is_not_input_dir(entry.name) && !entry.name.startsWith('.')) {
				newest_file(new URL(`${entry.name}/`, dir), newest);
			}
			continue;
		}
		const url = new URL(entry.name, dir);
		const mtime = statSync(url).mtimeMs;
		if (mtime > newest.mtime) {
			newest.path = url.pathname;
			newest.mtime = mtime;
		}
	}
};

/**
 * Refuse a missing or stale npm package rather than measure an old build. Stale is
 * any file under `crates/` or `scripts/`, or a workspace manifest, newer than the
 * oldest package file a consumer loads — `deno task build:npm` rebuilds all of them.
 *
 * @param chained_by - the tasks that chain `build:npm`, named in the refusal
 * @throws when the package is missing a file or is older than its newest input
 */
export const assert_pglet_package_current = (chained_by: string): void => {
	const rebuild = `run \`deno task build:npm\` (${chained_by} chain it)`;
	let built = Infinity;
	for (const name of PGLET_PKG_LOADED) {
		const url = new URL(name, PGLET_PKG);
		let mtime: number;
		try {
			mtime = statSync(url).mtimeMs;
		} catch {
			throw new Error(`the pglet npm package is missing ${url.pathname} — ${rebuild}`);
		}
		built = Math.min(built, mtime);
	}
	const newest = {path: '', mtime: 0};
	newest_file(new URL('crates/', REPO), newest);
	newest_file(new URL('scripts/', REPO), newest);
	for (const name of ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']) {
		const url = new URL(name, REPO);
		const mtime = statSync(url).mtimeMs;
		if (mtime > newest.mtime) {
			newest.path = url.pathname;
			newest.mtime = mtime;
		}
	}
	if (newest.mtime > built) {
		throw new Error(
			`the pglet npm package in ${PGLET_PKG.pathname} is older than ${newest.path} — ${rebuild}`,
		);
	}
};
