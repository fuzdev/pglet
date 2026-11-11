/**
 * What a cargo child reads from the host for a build. A task's cargo child gets gro's base
 * environment and a list, while an npm script's inherits the whole environment, and cargo
 * fingerprints what a build reads, so for each crate both builds compile the second finds
 * the first's output fresh only when the list holds every variable that crate's build reads:
 * cargo's and rustc's (`CARGO_PASS_ENV`), and each one a build script in the build's graph
 * reruns on (its `cargo:rerun-if-env-changed` lines), which the user of the list adds for the
 * graph it builds.
 *
 * @module
 */

/**
 * The host variables a cargo child is given beside gro's base environment:
 *
 * - `CARGO_*` - cargo's own (`CARGO_HOME`, `CARGO_TARGET_DIR`, `CARGO_ENCODED_RUSTFLAGS`,
 *   `CARGO_INCREMENTAL`, `CARGO_CACHE_RUSTC_INFO`, `CARGO_LOG`, `CARGO`) and every
 *   configuration value set as one (`CARGO_BUILD_*`, `CARGO_PROFILE_*`, `CARGO_TARGET_*`)
 * - `RUSTUP_*` - the toolchain rustup picks (`RUSTUP_TOOLCHAIN`, `RUSTUP_HOME`)
 * - `RUSTC`, `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER`, `RUSTFLAGS` - the compiler cargo
 *   runs, what wraps it, and the flags it passes
 * - `RUSTC_BOOTSTRAP` - read by rustc, and by a build script in the workspace's graph
 * - `HTTPS_PROXY`, `https_proxy`, `http_proxy`, `HTTP_TIMEOUT` - how cargo fetches a crate
 *   it lacks
 *
 * Not given: `RUSTDOC`, `RUSTDOCFLAGS`, `RUSTFMT` and `BROWSER`, which only `cargo doc` and
 * `cargo fmt` read.
 */
export const CARGO_PASS_ENV: ReadonlyArray<string> = [
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
	'HTTP_TIMEOUT'
];
