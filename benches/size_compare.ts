/**
 * pglet-vs-pglite payload size — the evidence behind the README's size ratio.
 *
 * Weighs the runtime payload a consumer downloads from each package, raw and gzipped:
 *
 *   - **pglet** — the wasm the npm package ships (`crates/pglet_wasm/pkg-npm/
 *     pglet_wasm_bg.wasm`, built by `deno task build:npm`), the file an
 *     `@fuzdev/pglet-wasm` consumer downloads. It is the same build `size.ts` gates
 *     (`size_baseline.json`, from `pkg`) up to embedded metadata, but the shipped file is
 *     the one the README's "~647 KB" describes, so it is the one weighed.
 *   - **pglite** — `pglite.wasm` + `pglite.data` + `initdb.wasm` from
 *     `benches/node_modules/@electric-sql/pglite/dist`, at the version
 *     `benches/package-lock.json` pins (the one `wasm_compare.ts` runs).
 *
 * JS glue is excluded from both sides. gzip is `lib/gzip_size.ts` — the measurement
 * `size.ts` reports — so every gzip figure in `benches/` is the same tool at the same level.
 *
 * Refuses (exit 1, nothing written) when the npm package is missing or older than its
 * sources (`lib/pglet_package.ts`), or when no pglite is installed under `benches/` or the
 * installed one is not the lockfile's (`lib/pglite_install.ts`) — the two rules
 * `wasm_compare.ts` applies too.
 *
 *   deno task bench:size-compare          # build the npm package, print
 *   deno task bench:size-compare:update   # ...and rewrite pglite_size_comparison.json
 *
 * @module
 */

import {fileURLToPath, pathToFileURL} from 'node:url';
import {gzip_size} from './lib/gzip_size.ts';
import {assert_pglet_package_current, PGLET_PKG} from './lib/pglet_package.ts';
import {assert_pglite_matches_lockfile} from './lib/pglite_install.ts';
import {git_commit, package_version, write_json} from './lib/results.ts';

const OUT = 'benches/pglite_size_comparison.json';

const refuse = (message: string): never => {
	console.error(`error: ${message}`);
	Deno.exit(1);
};

/** `fn`'s result, or a refusal carrying its error's message. */
const or_refuse = <T>(fn: () => T): T => {
	try {
		return fn();
	} catch (e) {
		return refuse(e instanceof Error ? e.message : String(e));
	}
};

interface Weighed {
	raw: number;
	gzip: number;
}

const weigh = async (url: URL): Promise<Weighed> => {
	let raw: number;
	try {
		raw = (await Deno.stat(url)).size;
	} catch {
		return refuse(`missing ${fileURLToPath(url)}`);
	}
	return {raw, gzip: await gzip_size(fileURLToPath(url))};
};

or_refuse(() => assert_pglet_package_current('the bench:size-compare tasks'));
const {dir: pglite_dir, version} = or_refuse(assert_pglite_matches_lockfile);

const pglet_wasm = await weigh(new URL('pglet_wasm_bg.wasm', PGLET_PKG));
const pglet_version = or_refuse(() => package_version(fileURLToPath(PGLET_PKG)));
const dist = pathToFileURL(`${pglite_dir}/dist/`);
const main_wasm = await weigh(new URL('pglite.wasm', dist));
const data = await weigh(new URL('pglite.data', dist));
const initdb_wasm = await weigh(new URL('initdb.wasm', dist));
const total_raw = main_wasm.raw + data.raw + initdb_wasm.raw;
const total_gzip = main_wasm.gzip + data.gzip + initdb_wasm.gzip;

// Recorded unrounded: a ratio rounded here and again where it is quoted can round the other way.
const doc = {
	schema: 'pglite_size_comparison',
	note:
		'Written by benches/size_compare.ts (`deno task bench:size-compare:update`, a step of ' +
		'`cargo xtask measure`). pglet: the wasm the npm package ships ' +
		'(crates/pglet_wasm/pkg-npm/pglet_wasm_bg.wasm, `deno task build:npm`) — the file a ' +
		'consumer downloads, the size_baseline.json build up to embedded metadata. pglite: ' +
		'pglite.wasm + pglite.data + initdb.wasm from benches/node_modules/@electric-sql/pglite/dist ' +
		'at the version benches/package-lock.json pins. gzip is `gzip -c` at its default level, as ' +
		'size.ts measures. Compares the runtime payload a browser/Node consumer downloads; JS glue ' +
		'is excluded from both sides.',
	timestamp: new Date().toISOString(),
	git_commit: git_commit(),
	pglet: {
		version: pglet_version,
		wasm_raw_bytes: pglet_wasm.raw,
		wasm_gzip_bytes: pglet_wasm.gzip,
	},
	pglite: {
		version,
		main_wasm_raw_bytes: main_wasm.raw,
		main_wasm_gzip_bytes: main_wasm.gzip,
		data_raw_bytes: data.raw,
		data_gzip_bytes: data.gzip,
		initdb_wasm_raw_bytes: initdb_wasm.raw,
		initdb_wasm_gzip_bytes: initdb_wasm.gzip,
		total_raw_bytes: total_raw,
		total_gzip_bytes: total_gzip,
	},
	ratios_pglite_over_pglet: {
		raw_total: total_raw / pglet_wasm.raw,
		gzip_total: total_gzip / pglet_wasm.gzip,
		raw_main_wasm_only: main_wasm.raw / pglet_wasm.raw,
		gzip_main_wasm_only: main_wasm.gzip / pglet_wasm.gzip,
	},
};

const kb = (n: number): string => `${(n / 1024).toFixed(1)} KB`;
const mb = (n: number): string => `${(n / 1024 / 1024).toFixed(2)} MB`;
console.log(`\npglet ${pglet_version} vs pglite ${version} — runtime payload, raw / gzip`);
console.log(`  pglet  wasm               ${kb(pglet_wasm.raw)} / ${kb(pglet_wasm.gzip)}`);
console.log(`  pglite wasm               ${mb(main_wasm.raw)} / ${mb(main_wasm.gzip)}`);
console.log(`  pglite data               ${mb(data.raw)} / ${mb(data.gzip)}`);
console.log(`  pglite initdb wasm        ${kb(initdb_wasm.raw)} / ${kb(initdb_wasm.gzip)}`);
console.log(`  pglite total              ${mb(total_raw)} / ${mb(total_gzip)}`);
console.log(`\nratios, pglite over pglet (×)`);
for (const [key, value] of Object.entries(doc.ratios_pglite_over_pglet)) {
	console.log(`  ${key.padEnd(24)} ${value.toFixed(3)}`);
}

if (Deno.args.includes('--update')) {
	await write_json(OUT, doc);
	console.error(`\nwrote ${OUT}`);
} else {
	console.error(`\n(results not written — pass --update to refresh ${OUT})`);
}
