/**
 * The wasm-pack builds the browser pages load, made when missing or older than
 * the newest `.rs` file of a crate they link. The browser tests' runner calls it
 * before it serves the pages; the program and the crates directory are deps.
 *
 * @module
 */

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import { fs_search } from '@fuzdev/gro/fs_search.ts';
import type { SpawnProgram } from '@fuzdev/gro/spawn.ts';

/**
 * A wasm-pack build of `crates/pglet_wasm`: its output directory beneath the crate, and
 * the cargo features it adds to the crate's defaults.
 */
export interface WasmPackage {
	out_dir: string;
	features: Array<string>;
}

/** The default build, the shipped artifact's shape (`npm run wasm:build`). */
export const WASM_DEFAULT: WasmPackage = { out_dir: 'pkg', features: [] };

/**
 * The build with the `opfs` feature on, which alone exports `PgletBtreePersistent`
 * (`npm run wasm:build:opfs`).
 */
export const WASM_OPFS: WasmPackage = { out_dir: 'pkg-opfs', features: ['opfs'] };

/** The source directories, beneath `crates/`, of every crate the wasm build links. */
export const WASM_SOURCE_DIRS: ReadonlyArray<string> = [
	'pglet_wasm/src',
	'pglet/src',
	'pglet_core/src',
	'pglet_sql/src',
	'pglet_storage/src'
];

/**
 * The host variables wasm-pack's child is given beside gro's base environment: what cargo,
 * rustc, rustup, wasm-pack and wasm-bindgen read for a build, so the build of a `pkg` here
 * and `npm run wasm:build`'s, which inherits the whole environment, compile with the same
 * flags and tools, and cargo, which fingerprints the flags, finds the other's output fresh.
 *
 * - `CARGO_*` - cargo's own (`CARGO_HOME`, `CARGO_TARGET_DIR`, `CARGO_ENCODED_RUSTFLAGS`,
 *   `CARGO_INCREMENTAL`, `CARGO_CACHE_RUSTC_INFO`, `CARGO_LOG`, `CARGO`) and every
 *   configuration value set as one (`CARGO_BUILD_*`, `CARGO_PROFILE_*`, `CARGO_TARGET_*`)
 * - `RUSTUP_*` - the toolchain rustup picks (`RUSTUP_TOOLCHAIN`, `RUSTUP_HOME`)
 * - `RUSTC`, `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER`, `RUSTFLAGS` - the compiler cargo
 *   runs, what wraps it, and the flags it passes
 * - `RUSTC_BOOTSTRAP` - read by rustc, and by a build script in the wasm build's graph
 * - `HTTPS_PROXY`, `https_proxy`, `http_proxy`, `HTTP_TIMEOUT` - how cargo fetches a crate
 *   it lacks
 * - `WASM_PACK_*` - wasm-pack's (`WASM_PACK_CACHE`, where its tools are kept)
 * - `WASM_BINDGEN_*` - wasm-bindgen's, which change the glue it emits
 *
 * Not given: `RUSTDOC`, `RUSTDOCFLAGS`, `RUSTFMT` and `BROWSER`, which only `cargo doc` and
 * `cargo fmt` read.
 */
export const WASM_PACK_PASS_ENV: ReadonlyArray<string> = [
	'CARGO_*',
	'RUSTUP_*',
	'RUSTC',
	'RUSTC_WRAPPER',
	'RUSTC_WORKSPACE_WRAPPER',
	'RUSTFLAGS',
	'RUSTC_BOOTSTRAP',
	'HTTPS_PROXY',
	'https_proxy',
	'http_proxy',
	'HTTP_TIMEOUT',
	'WASM_PACK_*',
	'WASM_BINDGEN_*'
];

/** What `ensure_wasm_package` reaches. */
export interface EnsureWasmDeps {
	/** This checkout's `crates/` directory. */
	crates: FsReadDir;
	/** `wasm-pack`, run in the checkout, its output inherited. */
	wasm_pack: SpawnProgram;
	/** The checkout, `wasm_pack`'s working directory. */
	checkout: FsReadDir;
	print: (text: string) => void;
}

/** The wasm-pack arguments that build `pkg`, from the checkout. */
export const to_wasm_pack_args = (pkg: WasmPackage): Array<string> => [
	'build',
	'crates/pglet_wasm',
	'--target',
	'web',
	'--out-dir',
	pkg.out_dir,
	...(pkg.features.length > 0 ? ['--features', pkg.features.join(',')] : [])
];

/**
 * Builds `pkg` unless its `pglet_wasm.js` is at least as new as every `.rs` file of
 * `WASM_SOURCE_DIRS`.
 *
 * @throws Error - when wasm-pack fails; the message names the script that builds it
 */
export const ensure_wasm_package = async (
	deps: EnsureWasmDeps,
	pkg: WasmPackage
): Promise<void> => {
	const { crates, wasm_pack, checkout, print } = deps;
	const built = await crates.stat(`pglet_wasm/${pkg.out_dir}/pglet_wasm.js`);
	let reason = 'not found';
	if (built !== null) {
		const newest = await newest_rust_mtime(crates);
		if (newest <= built.mtime_ms) {
			print(`[build] WASM package ${pkg.out_dir} already exists`);
			return;
		}
		reason = 'stale';
	}
	print(`[build] WASM package ${pkg.out_dir} ${reason}, building...`);
	const args = to_wasm_pack_args(pkg);
	print(`[build] wasm-pack ${args.join(' ')}...`);
	const result = await wasm_pack.run(args, { cwd: checkout });
	if (result.kind !== 'exited' || result.code !== 0) {
		throw new Error(
			`Failed to build WASM package ${pkg.out_dir}\n` +
				`Fix: npm run ${pkg.features.length > 0 ? 'wasm:build:opfs' : 'wasm:build'}`
		);
	}
};

/** The newest modification time of a `.rs` file beneath `WASM_SOURCE_DIRS`, `0` for none. */
const newest_rust_mtime = async (crates: FsReadDir): Promise<number> => {
	let newest = 0;
	for (const dir of WASM_SOURCE_DIRS) {
		for (const file of await fs_search(crates, { path: dir, sort: false })) {
			if (!file.path.endsWith('.rs')) continue;
			const info = await crates.stat(`${dir}/${file.path}`);
			if (info !== null && info.mtime_ms > newest) newest = info.mtime_ms;
		}
	}
	return newest;
};
