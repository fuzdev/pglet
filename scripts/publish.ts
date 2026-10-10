/**
 * Publish `@fuzdev/pglet-wasm` to npm. USER-RUN — publishing is always the maintainer's call:
 * this is **dry-run by default**; pass `--wetrun` to actually publish. It never bumps the
 * version — the package version comes from `Cargo.toml [workspace.package].version`, which
 * you bump when you want a release — and it never tags or pushes: it prints the tag command.
 * Idempotent: a version already on npm is reported and nothing past that check runs.
 *
 * **Strict flags.** Anything but `--wetrun` is refused before any step runs, so a misspelled
 * `--wet-run` fails loud instead of running a green dry run that reads like a publish.
 *
 * **Preflight.** A dirty working tree, a branch other than `main` (a detached `HEAD` included)
 * and a failing `npm whoami` are each a warning in a dry run, so it still validates without
 * blocking; under `--wetrun` they are collected and refused together, before any gate runs —
 * a release does not ship uncommitted or off-`main` state, or start a publish it cannot finish.
 *
 * **Gates** (a dry run runs them too, so a dry run validates the release):
 *   1. `deno task check` — the `check` chain (clippy, the stripped and wasm-feature builds, fmt,
 *      docs, the release-audit firewall)
 *   2. `deno task test:npm` — builds `pkg-npm` from an emptied directory (the build lives
 *      inside this task, so there is no separate build step, and no file an earlier build left
 *      there ships) and runs the package suite, ending in the packed-tarball smoke
 *      (`scripts/test_npm_pack.mjs`): `npm pack` of the same `pkg-npm` directory the publish
 *      step runs in, installed into a scratch project and imported by bare specifier. The task
 *      runs with `PKG_DIR` set to that directory, so an exported `PKG_DIR` cannot point its
 *      Node legs at another build, and nothing between it and the publish rebuilds `pkg-npm`.
 *   3. the size gate on the shipped `pkg-npm/pglet_wasm_bg.wasm`
 *
 * **Publish.** `npm publish --dry-run` in a dry run, `npm publish` under `--wetrun`, in
 * `pkg-npm`. npm's notice names the tarball's `name@version` and lists its files, and a dry
 * run fails unless it exits zero and reports exactly the built `package.json`'s `name@version`.
 * The tag is `v<version>`, printed as an annotated-tag command naming the commit this run built.
 *
 *   deno run --allow-read --allow-run scripts/publish.ts [--wetrun]
 */

const USAGE = 'usage: deno task publish:npm [--wetrun]';
const unknown_args = Deno.args.filter((arg) => arg !== '--wetrun');
if (unknown_args.length > 0) {
	console.error(`✗ unknown argument(s): ${unknown_args.join(' ')}\n${USAGE}`);
	Deno.exit(1);
}
const wetrun = Deno.args.includes('--wetrun');
const PKG = 'crates/pglet_wasm/pkg-npm';

const dec = new TextDecoder();

/** Run a command silently and return success + trimmed stdout/stderr. */
const capture = async (cmd: string, args: string[], cwd?: string) => {
	const {success, stdout, stderr} = await new Deno.Command(cmd, {args, cwd, stdout: 'piped', stderr: 'piped'})
		.output();
	return {success, out: dec.decode(stdout).trim(), err: dec.decode(stderr).trim()};
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

// Preflight (modeled on tsv/blake3): a warning in a dry run, a refusal under `--wetrun` —
// collected, so one run names every reason at once.
const refusals: string[] = [];
const refuse_or_warn = (message: string) => {
	if (wetrun) refusals.push(message);
	else console.warn(`⚠ ${message}`);
};
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
const who = await capture('npm', ['whoami']);
if (who.success) console.log(`npm user: ${who.out}`);
else refuse_or_warn(`npm whoami failed — not logged in to npm (run \`npm login\`): ${who.err.split('\n')[0]}`);
if (refusals.length > 0) {
	for (const refusal of refusals) console.error(`✗ ${refusal}`);
	console.error('✗ refusing to publish — nothing was built or published');
	Deno.exit(1);
}

// Validate before publishing. `test:npm` builds `pkg-npm`, so it also assembles the package.
console.log('\nrunning checks …');
await task('deno task check', 'deno', ['task', 'check']);
console.log('\nbuilding + testing @fuzdev/pglet-wasm …');
await task('deno task test:npm', 'deno', ['task', 'test:npm'], {PKG_DIR: PKG});

// Gate the size of the ACTUALLY-SHIPPED artifact (`pkg-npm`, just built by test:npm) against
// the committed baseline — the plain `size:check` task gates the sibling `pkg` build instead.
console.log('\ngating the shipped wasm size …');
await task('size gate (pkg-npm)', 'deno', [
	'run',
	'--allow-read',
	'--allow-run=gzip,git',
	'benches/size.ts',
	'--check',
	'--wasm',
	`${PKG}/pglet_wasm_bg.wasm`,
]);

const pkg = JSON.parse(await Deno.readTextFile(`${PKG}/package.json`));
const id = `${pkg.name}@${pkg.version}`;
console.log(`\npackage: ${id}`);

// Idempotency: never republish an existing version. `npm view pkg@ver version` returns the
// version on success, empty on a package that exists without this version, and E404 when the
// package is absent entirely — all "safe to publish" except a genuine lookup failure, which
// aborts (no publish decision is safe without an answer).
const view = await capture('npm', ['view', id, 'version']);
if (view.success && view.out === pkg.version) {
	console.log(`${id} is already published — nothing to do`);
	Deno.exit(0);
}
if (!view.success && !view.err.includes('E404')) {
	console.error(`✗ npm view ${id} failed:\n${view.err}`);
	Deno.exit(1);
}

// The gates take minutes, and `pkg-npm` holds what the checkout held while `test:npm` built it:
// under `--wetrun`, a checkout that moved since the preflight refuses rather than publish a
// build no commit describes.
const status_now = await capture('git', ['status', '--porcelain']);
const head_now = await capture('git', ['rev-parse', 'HEAD']);
if (status_now.out !== status.out || head_now.out !== head.out) {
	const moved = 'the checkout changed while the gates ran — the built package may match no commit';
	if (wetrun) {
		console.error(`✗ ${moved}; refusing to publish`);
		Deno.exit(1);
	}
	console.warn(`⚠ ${moved}`);
}

const tag = `v${pkg.version}`;
// annotated, and on the commit this run built, so a commit made after the run cannot move the
// tag off the published source
const tag_command = `git tag -m ${tag} ${tag} ${head.out} && git push origin ${tag}`;

// npm writes the tarball notice (name@version, the file list, the target registry) to stderr
// and `+ name@version` to stdout.
if (!wetrun) {
	const dry_args = ['publish', '--dry-run', '--access', 'public'];
	console.log(`\n(cd ${PKG} && npm ${dry_args.join(' ')})`);
	const dry = await capture('npm', dry_args, PKG);
	if (dry.err) console.log(dry.err);
	if (dry.out) console.log(dry.out);
	if (!dry.success) {
		console.error(`✗ npm ${dry_args.join(' ')} failed`);
		Deno.exit(1);
	}
	const reported = dry.out.split('\n').at(-1);
	if (reported !== `+ ${id}`) {
		console.error(`✗ npm publish --dry-run reported ${JSON.stringify(reported)}, expected "+ ${id}"`);
		Deno.exit(1);
	}
	console.log(`\nDRY RUN — nothing published. Re-run with --wetrun to publish ${id}, then tag it:`);
	console.log(`  ${tag_command}`);
	Deno.exit(0);
}

// the real publish streams, so npm's notice and any interactive auth prompt reach the terminal
console.log(`\n(cd ${PKG} && npm publish --access public)`);
await task('npm publish', 'npm', ['publish', '--access', 'public'], undefined, PKG);
console.log(`\n✓ published ${id}\nnext — the tag is yours to create and push:\n  ${tag_command}`);
