#!/usr/bin/env -S deno run --allow-read
/**
 * Print the environment exports that enable pglet's two legs (native server +
 * in-process wasm) in the fuz_app `db` test matrix.
 *
 * Run after the artifacts are built — `deno task matrix:artifacts` builds both
 * then runs this; `deno task matrix:env` just re-prints (no rebuild). Output is
 * bash and eval-safe (only `#` comments and `export` lines):
 *
 *   eval "$(deno task matrix:env)"               # in this repo
 *   (cd <your fuz_app checkout> && npx vitest run --project db)
 *
 * Fails loud if an artifact is missing rather than printing a path the consumer
 * would silently skip on.
 */

const root = Deno.cwd();
const server = `${root}/target/release/pglet_server`;
// The assembled npm package (`pkg-npm`, via `deno task build:npm`) — NOT the raw `pkg`.
// It carries the ergonomic `create_pglet_db` entry the fuz_app wasm leg consumes, so the
// matrix exercises the SHIPPED adapter (coercion, single-flight, fork), not a hand-rolled shim.
const wasm_pkg = `${root}/crates/pglet_wasm/pkg-npm`;

const exists = (path: string): boolean => {
	try {
		Deno.statSync(path);
		return true;
	} catch {
		return false;
	}
};

const missing: Array<string> = [];
if (!exists(server)) missing.push(server);
// `PGLET_WASM_PKG` is the pkg-npm directory; check the wasm blob AND the `index.js` entry the
// consumer imports are both present (a raw-`pkg` dir has the wasm but not the ergonomic entry).
if (!exists(`${wasm_pkg}/pglet_wasm_bg.wasm`)) missing.push(`${wasm_pkg}/pglet_wasm_bg.wasm`);
if (!exists(`${wasm_pkg}/index.js`)) missing.push(`${wasm_pkg}/index.js (run: deno task build:npm)`);

if (missing.length > 0) {
	console.error(
		`error: pglet matrix artifact(s) not built:\n  ${
			missing.join("\n  ")
		}\nrun: deno task matrix:artifacts`,
	);
	Deno.exit(1);
}

console.log(`# pglet artifacts for the fuz_app \`db\` matrix — export these (bash), then run`);
console.log(`#   npx vitest run --project db        (in your fuz_app checkout; add TEST_DATABASE_URL for the pg leg)`);
console.log(`export PGLET_SERVER_BIN=${server}`);
console.log(`export PGLET_WASM_PKG=${wasm_pkg}`);
