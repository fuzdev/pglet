/**
 * Which pglite a bench loads, and the one rule for whether `benches/` holds the right one.
 *
 * Two installs of pglite feed recorded figures: `benches/node_modules` (the lockfile's,
 * which `wasm_compare.ts` runs and `size_compare.ts` weighs) and the fuz_app checkout's own
 * (which `exit_proof.ts`'s pglite leg runs, through fuz_app's suite). Each is found the way
 * Node resolves a bare `@electric-sql/pglite` import from the directory that imports it —
 * `resolve_package_dir` — so the version recorded is the one that was loaded.
 *
 * `assert_pglite_matches_lockfile` is the check both `benches/` consumers apply before they
 * measure, and that `cargo xtask measure`'s preflight asks (`deno eval`) before any timed
 * step runs, so a run cannot record one pglite and quote another.
 *
 * **Cross-runtime on purpose**, like `results.ts`: `wasm_compare.ts` runs under Deno, Node
 * and Bun, so this uses `node:` builtins only.
 *
 * @module
 */

import {existsSync, readFileSync} from 'node:fs';
import {dirname, join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {package_version} from './results.ts';

/** The package name every pglite figure is about. */
export const PGLITE = '@electric-sql/pglite';

/** `benches/`, whose `package-lock.json` pins the pglite the comparison benches run. */
const BENCHES = fileURLToPath(new URL('../', import.meta.url));

const INSTALL = 'run `deno task bench:wasm-compare:install`';

/**
 * The directory a bare `name` import from `from` resolves to: the first
 * `node_modules/<name>` holding a `package.json`, walking up from `from` as Node does,
 * or `null` when there is none.
 */
export const resolve_package_dir = (from: string, name: string): string | null => {
	for (let dir = from; ; ) {
		const candidate = join(dir, 'node_modules', name);
		if (existsSync(join(candidate, 'package.json'))) return candidate;
		const parent = dirname(dir);
		if (parent === dir) return null;
		dir = parent;
	}
};

/** pglite as `benches/` loads it. */
export interface PgliteInstall {
	/** The package directory, as `resolve_package_dir` finds it from `benches/`. */
	dir: string;
	version: string;
}

/**
 * pglite as `benches/` loads it, refused unless it is the version `benches/package-lock.json`
 * pins.
 *
 * @throws when no pglite resolves from `benches/`, the lockfile pins none, or the installed
 * version is not the pinned one
 */
export const assert_pglite_matches_lockfile = (): PgliteInstall => {
	const dir = resolve_package_dir(BENCHES, PGLITE);
	if (dir === null) throw new Error(`no ${PGLITE} resolves from ${BENCHES} — ${INSTALL}`);
	const version = package_version(dir);
	const lockfile = join(BENCHES, 'package-lock.json');
	const lock = JSON.parse(readFileSync(lockfile, 'utf8')) as {
		packages?: Record<string, {version?: unknown} | undefined>;
	};
	const locked = lock.packages?.[`node_modules/${PGLITE}`]?.version;
	if (typeof locked !== 'string') throw new Error(`${lockfile} pins no ${PGLITE} version`);
	if (version !== locked) {
		throw new Error(`${dir} holds pglite ${version} but ${lockfile} pins ${locked} — ${INSTALL}`);
	}
	return {dir, version};
};
