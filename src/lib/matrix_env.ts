/**
 * Prints what points fuz_app's `db` test matrix at this checkout's pglet: the
 * export that enables the native server leg, and, as a comment since it isn't
 * an environment variable, the command that links this build's npm package
 * over the `@fuzdev/pglet-wasm` fuz_app installed, which its in-process wasm
 * leg imports.
 *
 * Run after the artifacts are built: `npm run matrix:artifacts` builds both and
 * then prints this, `npm run matrix:env` only prints it. The output is bash and
 * eval-safe, only `#` comments and `export` lines:
 *
 *     eval "$(npm run --silent matrix:env)"   # in this repo
 *     (cd <your fuz_app checkout> && npx vitest run --project db)
 *
 * A missing artifact fails loudly rather than printing a path the consumer
 * would silently skip on.
 *
 * @module
 */

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import { is_fs_error } from '@fuzdev/gro/fs_error.ts';
import { SilentError } from '@fuzdev/gro/task.ts';

/** The release server's dir beneath the checkout. */
export const MATRIX_RELEASE_DIR = 'target/release';

/**
 * The assembled npm package beneath the checkout, not the raw `pkg`: it carries
 * the ergonomic `create_pglet_db` entry the fuz_app wasm leg consumes, so the
 * matrix exercises the shipped adapter (coercion, single-flight, fork), not a
 * hand-rolled shim.
 */
export const MATRIX_NPM_PACKAGE_DIR = 'crates/pglet_wasm/pkg-npm';

/** What the command reaches, gathered by the task root. */
export interface MatrixEnvCommandDeps {
	/** The checkout's absolute path, which the printed lines name. */
	checkout_path: string;
	/**
	 * Opens `MATRIX_RELEASE_DIR` beneath the checkout, read-only, following
	 * symlinks, since a cargo target dir is often one.
	 */
	open_release_dir: () => Promise<FsReadDir>;
	/** Opens `MATRIX_NPM_PACKAGE_DIR` beneath the checkout, read-only. */
	open_npm_package_dir: () => Promise<FsReadDir>;
	/** Prints a line to stdout, as `console.log`. */
	print: (text: string) => void;
	/** Prints a block to stderr, as `console.error`. */
	print_error: (text: string) => void;
}

/** A dir that's absent, or a file where it should be, holds none of the artifacts. */
const open_or_null = async (open: () => Promise<FsReadDir>): Promise<FsReadDir | null> => {
	try {
		return await open();
	} catch (error) {
		if (is_fs_error(error, 'not_found') || is_fs_error(error, 'not_a_directory')) return null;
		throw error;
	}
};

/**
 * Prints the matrix setup, or each missing artifact and how to build it.
 *
 * @throws SilentError - exit 1 after naming the missing artifacts
 */
export const run_matrix_env_command = async (deps: MatrixEnvCommandDeps): Promise<void> => {
	const { checkout_path, print, print_error } = deps;
	const server = `${checkout_path}/${MATRIX_RELEASE_DIR}/pglet_server`;
	const wasm_pkg = `${checkout_path}/${MATRIX_NPM_PACKAGE_DIR}`;

	const release_dir = await open_or_null(deps.open_release_dir);
	const pkg_dir = await open_or_null(deps.open_npm_package_dir);
	const missing: Array<string> = [];
	if (!(await release_dir?.exists('pglet_server'))) missing.push(server);
	// the pkg-npm dir is what the link names: check that the wasm blob and the `index.js` entry
	// the consumer imports are both present (a raw `pkg` dir has the wasm but not the entry)
	if (!(await pkg_dir?.exists('pglet_wasm_bg.wasm')))
		missing.push(`${wasm_pkg}/pglet_wasm_bg.wasm`);
	if (!(await pkg_dir?.exists('index.js'))) {
		missing.push(`${wasm_pkg}/index.js (run: npm run build:npm)`);
	}

	if (missing.length > 0) {
		print_error(
			`error: pglet matrix artifact(s) not built:\n  ${missing.join('\n  ')}\nrun: npm run matrix:artifacts`
		);
		throw new SilentError();
	}

	print('# pglet artifacts for the fuz_app `db` matrix — export these (bash), then run');
	print(
		'#   npx vitest run --project db        (in your fuz_app checkout; add TEST_DATABASE_URL for the pg leg)'
	);
	print(`export PGLET_SERVER_BIN=${server}`);
	print('# the wasm leg imports @fuzdev/pglet-wasm; to run this build, in your fuz_app checkout:');
	print(
		`#   rm -rf node_modules/@fuzdev/pglet-wasm && ln -s ${wasm_pkg} node_modules/@fuzdev/pglet-wasm`
	);
};
