/**
 * Publish `@fuzdev/pglet-wasm` to npm. USER-RUN — publishing is always the maintainer's call:
 * this is **dry-run by default**; pass `--wetrun` to actually publish. It never bumps the
 * version — the package version comes from `Cargo.toml [workspace.package].version`, which
 * you bump when you want a release — and it never tags or pushes: it prints the tag command.
 * Idempotent: a version already on npm is reported in the preflight and nothing past it runs.
 *
 * **Strict flags.** Anything but `--wetrun` is refused before any step runs, so a misspelled
 * `--wet-run` fails loud instead of running a green dry run that reads like a publish.
 *
 * **Preflight**, before any gate, so nothing that can be known up front is learned minutes in:
 *   - always refused: a missing tool (`node` below the package's `engines` floor, `wasm-pack`,
 *     `gzip`), an unreadable version, and a failed registry lookup;
 *   - a warning in a dry run, refused under `--wetrun` (collected, so one run names every
 *     reason): a dirty working tree, a branch other than `main` (a detached `HEAD` included), a
 *     `HEAD` that is not exactly `origin/main` (the published source must be the public
 *     source, and the tag must land on a commit `main` holds), an existing `v<version>` tag,
 *     locally or on `origin`, and a failing `npm whoami`.
 *
 * **Gates** (a dry run runs them too, so a dry run validates the release):
 *   1. `deno task check` — the `check` chain (clippy, the stripped and wasm-feature builds, fmt,
 *      docs, the release scripts' typecheck, the release-audit firewall)
 *   2. `deno task test:rust` and `deno task test:wasm` — the engine's own suites, natively and
 *      through the wasm binding; neither needs Postgres
 *   3. `deno task test:npm` — builds `pkg-npm` from an emptied directory (the build lives
 *      inside this task, so there is no separate build step, and no file an earlier build left
 *      there ships) and runs the package suite, ending in the packed-tarball smoke
 *      (`scripts/test_npm_pack.mjs`): `npm pack` of the same `pkg-npm` directory the publish
 *      step runs in, installed into a scratch project and imported by bare specifier. The task
 *      runs with `PKG_DIR` set to that directory, so an exported `PKG_DIR` cannot point its
 *      Node legs at another build, and nothing between it and the publish rebuilds `pkg-npm`.
 *   4. the built `package.json`'s `name@version` against the registry identity and `Cargo.toml`
 *   5. the size gate on the shipped `pkg-npm/pglet_wasm_bg.wasm`
 *
 * **Publish.** `npm publish --dry-run` in a dry run, `npm publish` under `--wetrun`, in
 * `pkg-npm`; the package's own `publishConfig` makes it public. A prerelease version
 * (`0.2.0-alpha.1`) publishes under the `next` dist-tag, which npm requires a prerelease to
 * name. npm's notice names the tarball's `name@version` and lists its files, and a dry run
 * fails unless it exits zero and reports exactly that `name@version`. The tag is
 * `v<version>`, printed as an annotated-tag command naming the commit this run built.
 *
 *   deno run --frozen --allow-read --allow-run=git,npm,deno,node,wasm-pack,gzip scripts/publish.ts [--wetrun]
 */

import {NPM_SHARED_METADATA} from './npm_metadata.ts';

const USAGE = 'usage: deno task publish:npm [--wetrun]';
const unknown_args = Deno.args.filter((arg) => arg !== '--wetrun');
if (unknown_args.length > 0) {
	console.error(`✗ unknown argument(s): ${unknown_args.join(' ')}\n${USAGE}`);
	Deno.exit(1);
}
const wetrun = Deno.args.includes('--wetrun');
const PKG = 'crates/pglet_wasm/pkg-npm';
const NAME = NPM_SHARED_METADATA.name;

const dec = new TextDecoder();

/** Run a command silently and return success + its output (stdout keeps its leading space, which
 * porcelain status lines open with); a missing binary fails. */
const capture = async (cmd: string, args: string[], cwd?: string) => {
	try {
		const {success, stdout, stderr} = await new Deno.Command(cmd, {args, cwd, stdout: 'piped', stderr: 'piped'})
			.output();
		return {success, out: dec.decode(stdout).trimEnd(), err: dec.decode(stderr).trim()};
	} catch (error) {
		if (!(error instanceof Deno.errors.NotFound)) throw error;
		return {success: false, out: '', err: `${cmd}: command not found`};
	}
};

/** Run a long task streaming its output; exit on failure. */
const task = async (label: string, cmd: string, args: string[], env?: Record<string, string>, cwd?: string) => {
	const {success, code} = await new Deno.Command(cmd, {
		args,
		env,
		cwd,
		stdin: 'inherit',
		stdout: 'inherit',
		stderr: 'inherit',
	}).output();
	if (!success) {
		console.error(`\n✗ ${label} failed (exit ${code})`);
		Deno.exit(1);
	}
};

function fail(message: string): never {
	console.error(`✗ ${message}`);
	Deno.exit(1);
}

// Preflight (modeled on tsv/blake3). Tools and the version are needed by a dry run too, so
// they refuse in both modes; the release-state checks are a warning in a dry run and a
// refusal under `--wetrun` — collected, so one run names every reason at once.
const failures: string[] = [];
const refusals: string[] = [];
const refuse_or_warn = (message: string) => {
	if (wetrun) refusals.push(message);
	else console.warn(`⚠ ${message}`);
};

// The version, from the one place it is declared. `[^[]*?` bounds the scan to the section, and
// `^version` (multiline) skips `rust-version`.
const cargo_match = /\[workspace\.package\][^[]*?^version\s*=\s*"([^"]+)"/m.exec(await Deno.readTextFile('Cargo.toml'));
if (!cargo_match) fail('no [workspace.package] version in Cargo.toml');
const version = cargo_match[1];
const id = `${NAME}@${version}`;
const tag = `v${version}`;
// npm refuses a prerelease without a dist-tag, and `latest` must never point at one.
const dist_tag = version.includes('-') ? 'next' : null;
console.log(`package: ${id}${dist_tag ? ` (prerelease, dist-tag ${dist_tag})` : ''}`);

// The tools the gates reach, asked here so a missing one fails before `check` spends minutes.
const engines_floor = /^>=(\d+)$/.exec(NPM_SHARED_METADATA.engines.node);
const node = await capture('node', ['--version']);
const node_major = Number(/^v(\d+)\./.exec(node.out)?.[1]);
if (!node.success) failures.push(`node not found — test:npm runs the package under it`);
else if (!engines_floor || !(node_major >= Number(engines_floor[1]))) {
	failures.push(`node ${node.out} is below the package's engines floor (${NPM_SHARED_METADATA.engines.node})`);
}
for (const [tool, args, why] of [
	['wasm-pack', ['--version'], 'builds the package (`cargo install wasm-pack`)'],
	['gzip', ['--version'], 'the size gate measures the wire size with it'],
] as const) {
	if (!(await capture(tool, [...args])).success) failures.push(`${tool} not found — ${why}`);
}

const status = await capture('git', ['status', '--porcelain']);
if (!status.success) refuse_or_warn(`git status failed: ${status.err}`);
else if (status.out) refuse_or_warn(`working tree not clean — commit or stash first:\n${status.out}`);
const branch = await capture('git', ['branch', '--show-current']);
if (!branch.success) refuse_or_warn(`git branch failed: ${branch.err}`);
else if (branch.out !== 'main') {
	refuse_or_warn(`${branch.out ? `on branch "${branch.out}"` : 'HEAD is detached'} — releases publish from main`);
}
const head = await capture('git', ['rev-parse', 'HEAD']);
if (!head.success) refuse_or_warn(`git rev-parse HEAD failed: ${head.err}`);

// The published source must be the public source: a commit not yet pushed, or one main has
// moved past, publishes what no one else can build, and the tag it prints lands off `main`.
const fetched = await capture('git', ['fetch', 'origin', 'main']);
if (!fetched.success) refuse_or_warn(`git fetch origin main failed — cannot compare HEAD with origin: ${fetched.err}`);
else {
	const counts = await capture('git', ['rev-list', '--left-right', '--count', 'origin/main...HEAD']);
	const [behind, ahead] = counts.out.split(/\s+/).map(Number);
	if (!counts.success || Number.isNaN(behind) || Number.isNaN(ahead)) {
		refuse_or_warn(`could not compare HEAD with origin/main: ${counts.err}`);
	} else if (behind > 0 || ahead > 0) {
		refuse_or_warn(
			`HEAD is ${ahead} ahead / ${behind} behind origin/main — publish a commit that is exactly origin/main`,
		);
	}
}

// The tag the run prints must be creatable: one that already exists, here or on origin, would
// fail that command after the publish, when the release can no longer be redone.
if ((await capture('git', ['rev-parse', '-q', '--verify', `refs/tags/${tag}`])).success) {
	refuse_or_warn(`tag ${tag} already exists locally`);
}
const remote_tag = await capture('git', ['ls-remote', '--tags', 'origin', `refs/tags/${tag}`]);
if (!remote_tag.success) refuse_or_warn(`git ls-remote origin failed: ${remote_tag.err}`);
else if (remote_tag.out) refuse_or_warn(`tag ${tag} already exists on origin`);

const who = await capture('npm', ['whoami']);
if (who.success) console.log(`npm user: ${who.out}`);
else refuse_or_warn(`npm whoami failed — not logged in to npm (run \`npm login\`): ${who.err.split('\n')[0]}`);

// Idempotency: never republish an existing version. `npm view pkg@ver version` returns the
// version on success, empty on a package that exists without this version, and E404 when the
// package is absent entirely — all "safe to publish" except a genuine lookup failure, which
// aborts (no publish decision is safe without an answer).
const view = await capture('npm', ['view', id, 'version']);
if (view.success && view.out === version) {
	console.log(`${id} is already published — nothing to do`);
	Deno.exit(0);
}
if (!view.success && !view.err.includes('E404')) failures.push(`npm view ${id} failed:\n${view.err}`);

if (failures.length > 0 || refusals.length > 0) {
	for (const message of [...failures, ...refusals]) console.error(`✗ ${message}`);
	console.error(`✗ refusing to ${wetrun ? 'publish' : 'run'} — nothing was built or published`);
	Deno.exit(1);
}

// Validate before publishing. `test:npm` builds `pkg-npm`, so it also assembles the package,
// and runs last of the suites so nothing after it rebuilds what ships.
console.log('\nrunning checks …');
await task('deno task check', 'deno', ['task', 'check']);
console.log('\ntesting the engine (native + wasm) …');
await task('deno task test:rust', 'deno', ['task', 'test:rust']);
await task('deno task test:wasm', 'deno', ['task', 'test:wasm']);
console.log('\nbuilding + testing @fuzdev/pglet-wasm …');
await task('deno task test:npm', 'deno', ['task', 'test:npm'], {PKG_DIR: PKG});

// wasm-pack stamps the version from Cargo.toml and the patch script the name; assert both
// rather than trust the chain, since this is the file npm publishes from.
const pkg = JSON.parse(await Deno.readTextFile(`${PKG}/package.json`));
if (`${pkg.name}@${pkg.version}` !== id) fail(`${PKG} built as ${pkg.name}@${pkg.version}, expected ${id}`);

// Gate the size of the ACTUALLY-SHIPPED artifact (`pkg-npm`, just built by test:npm) against
// the committed baseline — the plain `size:check` task gates the sibling `pkg` build instead.
console.log('\ngating the shipped wasm size …');
await task('size gate (pkg-npm)', 'deno', [
	'run',
	'--frozen',
	'--allow-read',
	'--allow-run=gzip,git',
	'benches/size.ts',
	'--check',
	'--wasm',
	`${PKG}/pglet_wasm_bg.wasm`,
]);

// The gates take minutes, and `pkg-npm` holds what the checkout held while `test:npm` built it:
// under `--wetrun`, a checkout that moved since the preflight refuses rather than publish a
// build no commit describes.
const status_now = await capture('git', ['status', '--porcelain']);
const head_now = await capture('git', ['rev-parse', 'HEAD']);
if (status_now.out !== status.out || head_now.out !== head.out) {
	const moved = 'the checkout changed while the gates ran — the built package may match no commit';
	if (wetrun) fail(`${moved}; refusing to publish`);
	console.warn(`⚠ ${moved}`);
}

// annotated, and on the commit this run built, so a commit made after the run cannot move the
// tag off the published source
const tag_command = `git tag -m ${tag} ${tag} ${head.out} && git push origin ${tag}`;
const publish_args = ['publish', ...(dist_tag ? ['--tag', dist_tag] : [])];

// npm writes the tarball notice (name@version, the file list, the target registry) to stderr
// and `+ name@version` to stdout.
if (!wetrun) {
	const dry_args = [...publish_args, '--dry-run'];
	console.log(`\n(cd ${PKG} && npm ${dry_args.join(' ')})`);
	const dry = await capture('npm', dry_args, PKG);
	if (dry.err) console.log(dry.err);
	if (dry.out) console.log(dry.out);
	if (!dry.success) fail(`npm ${dry_args.join(' ')} failed`);
	const reported = dry.out.split('\n').at(-1);
	if (reported !== `+ ${id}`) fail(`npm publish --dry-run reported ${JSON.stringify(reported)}, expected "+ ${id}"`);
	console.log(`\nDRY RUN — nothing published. Re-run with --wetrun to publish ${id}, then tag it:`);
	console.log(`  ${tag_command}`);
	Deno.exit(0);
}

// the real publish streams, so npm's notice and any interactive auth prompt reach the terminal
console.log(`\n(cd ${PKG} && npm ${publish_args.join(' ')})`);
await task('npm publish', 'npm', publish_args, undefined, PKG);
console.log(`\n✓ published ${id}\nnext — the tag is yours to create and push:\n  ${tag_command}`);
