/**
 * The packed-tarball smoke test of `@fuzdev/pglet-wasm`: what a consumer's `npm install` gets.
 *
 * `npm pack`s the assembled package (`PKG_DIR`, default `crates/pglet_wasm/pkg-npm`), installs the
 * `.tgz` into a scratch project, and there — under real Node, by **bare specifier**, so resolution
 * runs through the installed `package.json`'s `exports` — imports `@fuzdev/pglet-wasm` and
 * `@fuzdev/pglet-wasm/kysely` and runs a query through each. The files an entry imports at runtime
 * reach the consumer only if `files[]` lists them, and the entries resolve only through what
 * `exports` names, so a file dropped from either reddens this test and nothing else in `test:npm`:
 * the other package tests read the build directory, where every file is present whatever
 * `files[]` says. The type declarations `exports`, `main` and `types` name are checked to be in the
 * installed package too, and so is each relative import inside them.
 *
 * **The directory that publishes.** `scripts/publish.ts` runs this (through `test:npm`) with
 * `PKG_DIR` set to the `pkg-npm` its publish step runs in, after `build:npm` has rebuilt that
 * directory from empty, and nothing rebuilds it in between — so the tarball smoked here is packed
 * from the same files as the one published.
 *
 * **Offline.** `npm pack` and `npm install --offline` of a dependency-free tarball reach no
 * registry. `./kysely` imports `kysely`, an optional peer the scratch project does not install
 * from a registry: it links the copy Deno already holds for the pinned version (`deno.json`'s
 * `kysely` import, which `test:npm`'s Deno suite loads first), found through `deno info --json`.
 * What that does not prove: that npm itself installs or resolves `kysely` as a peer, and that the
 * subpath works with any `kysely` but the pinned one.
 *
 * npm's cache and logs go to the scratch directory (`npm_config_cache`), so nothing is written
 * outside the OS temp dir; the scratch directory is removed afterwards.
 *
 *   PKG_DIR=crates/pglet_wasm/pkg-npm node --test scripts/test_npm_pack.mjs
 */
import {after, before, test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join, resolve} from 'node:path';

const PKG_DIR = resolve(process.env.PKG_DIR ?? 'crates/pglet_wasm/pkg-npm');
// read from the built package, so the bare specifier is whatever name it was assembled under
const NAME = JSON.parse(readFileSync(join(PKG_DIR, 'package.json'), 'utf8')).name;

let scratch;
let installed;

/** `npm <args>` in `cwd`, offline, its cache and logs kept in the scratch directory. */
const npm = (args, cwd) =>
	execFileSync('npm', [...args, '--offline', '--no-audit', '--no-fund', '--ignore-scripts', '--loglevel=error'], {
		cwd,
		encoding: 'utf8',
		env: {...process.env, npm_config_cache: join(scratch, 'npm-cache'), npm_config_update_notifier: 'false'},
	});

/** The pinned `kysely` in Deno's npm cache, the copy `test:npm`'s Deno suite loaded. */
const deno_kysely = () => {
	const deno_json = JSON.parse(readFileSync(resolve('deno.json'), 'utf8'));
	const version = /^npm:kysely@(.+)$/.exec(deno_json.imports?.kysely ?? '')?.[1];
	assert.ok(version, 'deno.json pins no `npm:kysely@<version>` import');
	const {npmCache} = JSON.parse(execFileSync('deno', ['info', '--json'], {encoding: 'utf8'}));
	const dir = join(npmCache, 'registry.npmjs.org', 'kysely', version);
	assert.ok(
		existsSync(join(dir, 'package.json')),
		`kysely ${version} is not in Deno's npm cache (${dir}); run the Deno half of \`deno task test:npm\` first`,
	);
	return dir;
};

/** Run `body` as an ES module in the scratch project under this Node, answering its stdout. */
const run_in_project = (body) =>
	execFileSync(process.execPath, ['--input-type=module', '-e', body], {cwd: scratch, encoding: 'utf8'});

before(() => {
	scratch = mkdtempSync(join(tmpdir(), 'pglet_npm_pack_'));
	const [{filename: tgz}] = JSON.parse(npm(['pack', PKG_DIR, '--pack-destination', scratch, '--json'], scratch));
	writeFileSync(join(scratch, 'package.json'), JSON.stringify({name: 'pglet-pack-smoke', private: true, type: 'module'}));
	npm(['install', join(scratch, tgz)], scratch);
	installed = join(scratch, 'node_modules', ...NAME.split('/'));
	mkdirSync(join(scratch, 'node_modules'), {recursive: true});
	symlinkSync(deno_kysely(), join(scratch, 'node_modules', 'kysely'), 'dir');
});

after(() => {
	if (scratch) rmSync(scratch, {recursive: true, force: true});
});

test('the root entry, by bare specifier from the installed tarball, runs a query', () => {
	const out = run_in_project(
		`import {create_pglet_db} from '${NAME}';
		const db = create_pglet_db();
		await db.query('CREATE TABLE t (id BIGINT PRIMARY KEY, name TEXT)');
		await db.query('INSERT INTO t VALUES ($1, $2)', [1n, 'ada']);
		const {rows} = await db.query('SELECT id, name FROM t');
		console.log(JSON.stringify(rows, (_k, v) => (typeof v === 'bigint' ? v + 'n' : v)));
		await db.close();`,
	);
	assert.equal(out.trim(), '[{"id":"1n","name":"ada"}]');
});

test('the ./kysely subpath, by bare specifier from the installed tarball, runs a query', () => {
	const out = run_in_project(
		`import {Kysely, sql} from 'kysely';
		import {create_pglet_db} from '${NAME}';
		import {PgletDialect} from '${NAME}/kysely';
		const db = new Kysely({dialect: new PgletDialect({db: create_pglet_db({coercion: {int8: 'number'}})})});
		const {rows} = await sql\`SELECT 1 + 1 AS two\`.execute(db);
		console.log(JSON.stringify(rows));
		await db.destroy();`,
	);
	assert.equal(out.trim(), '[{"two":2}]');
});

test('every file exports, main and types name is in the installed package, and so is every relative import of a declaration', () => {
	const pkg = JSON.parse(readFileSync(join(installed, 'package.json'), 'utf8'));
	const named = new Set([pkg.main, pkg.types]);
	for (const target of Object.values(pkg.exports)) {
		for (const file of typeof target === 'string' ? [target] : Object.values(target)) named.add(file);
	}
	for (const file of named) assert.ok(existsSync(join(installed, file)), `${file} is named but not installed`);
	for (const dts of readdirSync(installed).filter((f) => f.endsWith('.d.ts'))) {
		const text = readFileSync(join(installed, dts), 'utf8');
		for (const [, spec] of text.matchAll(/from\s+'(\.[^']+)'/g)) {
			// a declaration's `./x.js` is resolved by TypeScript to `./x.d.ts`
			const declared = spec.replace(/\.js$/, '.d.ts');
			assert.ok(existsSync(resolve(dirname(join(installed, dts)), declared)), `${dts} imports ${spec}, whose ${declared} is not installed`);
		}
	}
});
