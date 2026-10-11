/**
 * Release `@fuzdev/pglet-wasm`: bump, gate, publish to npm, and commit, tag and push the release.
 * USER-RUN — publishing is always the maintainer's call: this is **dry-run by default** and
 * changes nothing; `--wetrun` releases.
 *
 *   deno task publish:npm --bump patch            # dry run: validate the release a wetrun makes
 *   deno task publish:npm --wetrun --bump patch   # release
 *   deno task publish:npm --wetrun                # resume an interrupted release
 *
 * **Flags** — anything else is refused before any step runs, so a misspelled `--wet-run` fails
 * loud instead of running a green dry run that reads like a release:
 *   - `--wetrun` — mutate: bump, publish, commit, tag, push
 *   - `--bump patch|minor|major` — the release's level, which must repeat the
 *     `<!-- bump: <level> -->` marker under `CHANGELOG.md`'s `## Unreleased` heading (the level
 *     stated twice, so a slip in either refuses); ignored, with a warning, by a resume
 *   - `--no-git` — stop after the publish and print the commit, tag and push instead
 *
 * **The version** is `Cargo.toml`'s `[workspace.package]` version, which wasm-pack stamps into
 * the package. A fresh release needs a non-empty `## Unreleased` section; the bump writes the
 * new version, both lockfiles' workspace members (the root's and `fuzz/`'s), and the changelog,
 * whose `## Unreleased` becomes `## <version>` under a fresh `## Unreleased` declaring a patch.
 * Those files, and only those, make the release commit (`scripts/release/bump.ts`).
 *
 * **Preflight**, before anything is mutated or built, so nothing that can be known up front is
 * learned minutes in. Always refused: a missing tool (`node` below the package's `engines` floor,
 * `wasm-pack`, `gzip`, `cargo`), an unreadable version, a failed registry lookup, and a fresh
 * release whose version is already on npm. A warning in a dry run and a refusal under
 * `--wetrun`, collected so one run names every reason: the release plan's problems (the
 * changelog and `--bump`, `scripts/release/plan.ts`), a dirty working tree, a branch other than
 * `main`, a `HEAD` that is not exactly `origin/main` (the published source must be the public
 * source), an existing `v<version>` tag, locally or on `origin`, and a failing `npm whoami`.
 *
 * **Gates** (a dry run runs them too, so a dry run validates the release):
 *   1. `deno task check` — clippy, the stripped and wasm-feature builds, fmt, docs, the release
 *      scripts' typecheck and unit tests, the release-audit firewall
 *   2. `deno task test:rust` and `deno task test:wasm` — the engine's own suites, natively and
 *      through the wasm binding; neither needs Postgres
 *   3. `deno task test:npm` — builds `pkg-npm` from an emptied directory and runs the package
 *      suite, ending in the packed-tarball smoke over that same directory. `PKG_DIR` is set to
 *      it, so an exported `PKG_DIR` cannot point the Node legs at another build, and nothing
 *      between it and the publish rebuilds `pkg-npm`.
 *   4. the built `package.json`'s `name@version` against the registry identity and the version
 *   5. the size gate on the shipped `pkg-npm/pglet_wasm_bg.wasm`
 * Under `--wetrun`, a checkout that changed while the gates ran refuses to publish.
 *
 * **Publish** in `pkg-npm`: `npm publish --dry-run` in a dry run, which must report exactly the
 * built `name@version` (or, when the dry run previews a bump, refuse only as a republish of the
 * unbumped version); `npm publish` under `--wetrun`, streamed, so npm's browser auth reaches the
 * terminal. The package's own `publishConfig` makes it public.
 *
 * **Git finalize**: commit the release files as `chore: release v<version>`, create the annotated
 * tag `v<version>`, and `git push --follow-tags origin main`.
 *
 * **Resume.** A wetrun writes `.publish-in-progress` (gitignored, `scripts/release/sentinel.ts`)
 * right after the bump and removes it only once the push lands. While it names the checkout's
 * version, a wetrun resumes that release: it skips the bump, accepts a tree dirty with only the
 * release files or one release commit ahead of `origin/main`, and repeats every later step, each
 * of which is idempotent. A resumed release whose version is already on npm skips the gates and
 * the publish and only finishes git.
 *
 *   deno run --frozen --allow-read --allow-write=Cargo.toml,CHANGELOG.md,.publish-in-progress \
 *     --allow-run=git,npm,deno,node,wasm-pack,gzip,cargo scripts/publish.ts [flags]
 */

import {NPM_SHARED_METADATA} from './npm_metadata.ts';
import {
	bump_cargo_version,
	bump_changelog,
	bump_lockfiles,
	release_commit_message,
	RELEASE_PATHS,
} from './release/bump.ts';
import {CHANGELOG_PATH} from './release/changelog.ts';
import {command_capture, command_stream, type CommandOptions} from './release/command.ts';
import {
	git_checkout_fingerprint,
	git_commit_paths,
	git_current_branch,
	git_fetch_and_compare_origin_main,
	git_head_subject,
	git_local_tag_commit,
	git_push_main_with_tags,
	git_remote_tag_commit,
	git_rev,
	git_status_line_path,
	git_status_lines,
	git_tag_annotated,
} from './release/git.ts';
import {npm_error_is_republish, npm_is_published, npm_whoami} from './release/npm.ts';
import {release_plan} from './release/plan.ts';
import {sentinel_read, sentinel_remove, sentinel_write} from './release/sentinel.ts';
import {type BumpLevel, CARGO_PATH, cargo_workspace_version, is_bump_level, version_tag} from './release/version.ts';

const USAGE = 'usage: deno task publish:npm [--wetrun] [--bump patch|minor|major] [--no-git]';
const PKG = 'crates/pglet_wasm/pkg-npm';
const NAME = NPM_SHARED_METADATA.name;

function fail(message: string): never {
	console.error(`✗ ${message}`);
	Deno.exit(1);
}

let wetrun = false;
let no_git = false;
let bump: BumpLevel | null = null;
const unknown_args: Array<string> = [];
for (let i = 0; i < Deno.args.length; i++) {
	const arg = Deno.args[i];
	if (arg === '--wetrun') wetrun = true;
	else if (arg === '--no-git') no_git = true;
	else if (arg === '--bump') {
		const level = Deno.args[++i];
		if (!is_bump_level(level)) fail(`--bump takes patch, minor or major (got ${JSON.stringify(level ?? '')})\n${USAGE}`);
		bump = level;
	} else unknown_args.push(arg);
}
if (unknown_args.length > 0) fail(`unknown argument(s): ${unknown_args.join(' ')}\n${USAGE}`);

/** Set once the bump is written: from then on a failure names the resume. */
let bumped = false;
const resume_hint = () =>
	bumped ? '\n  the bump is in place — fix the cause and run `deno task publish:npm --wetrun` to resume' : '';

/** Run a long step streaming its output; exit on failure. */
const step = async (label: string, cmd: string, args: Array<string>, options?: CommandOptions) => {
	const {success, code} = await command_stream(cmd, args, options);
	if (!success) fail(`${label} failed (exit ${code})${resume_hint()}`);
};

// Plan. Everything a release decides from is read here, before anything is touched.

const cargo_version = cargo_workspace_version(await Deno.readTextFile(CARGO_PATH));
if (cargo_version === null) fail(`no [workspace.package] version in ${CARGO_PATH}`);
const changelog = await Deno.readTextFile(CHANGELOG_PATH).catch((error) => {
	if (error instanceof Deno.errors.NotFound) return null;
	throw error;
});
const plan = release_plan({version: cargo_version, bump, sentinel: await sentinel_read(), changelog});
/** The version this run releases; null only when a fresh plan has no target, which a wetrun
 * refuses. */
const release_version = plan.kind === 'resume' ? plan.version : plan.target;
const release_tag = release_version === null ? null : version_tag(release_version);

console.log(`=== ${NAME} release ${wetrun ? '(wetrun)' : '(dry run)'} ===`);
if (plan.kind === 'resume') {
	console.log(`resuming the interrupted release of ${NAME}@${plan.version}`);
	for (const note of plan.notes) console.warn(`⚠ ${note}`);
} else {
	console.log(`${NAME}: ${plan.version} → ${plan.target ?? '?'}${plan.level ? ` (${plan.level})` : ''}`);
	if (plan.stale_sentinel !== null) {
		console.warn(`⚠ a sentinel names v${plan.stale_sentinel}, not this checkout's version — a wetrun removes it`);
	}
}

// Preflight. Tools, the registry lookup and a republish refuse in both modes; the release-state
// checks are a warning in a dry run and a refusal under `--wetrun`.

const failures: Array<string> = [];
const refusals: Array<string> = [];
/** How many refusals a dry run downgraded to warnings, so its last line can say a wetrun
 * would refuse. */
let warnings = 0;
const refuse_or_warn = (message: string) => {
	if (wetrun) refusals.push(message);
	else {
		warnings++;
		console.warn(`⚠ ${message}`);
	}
};

if (plan.kind === 'fresh') for (const problem of plan.problems) refuse_or_warn(problem);

const engines_floor = /^>=(\d+)$/.exec(NPM_SHARED_METADATA.engines.node);
const node = await command_capture('node', ['--version']);
const node_major = Number(/^v(\d+)\./.exec(node.out)?.[1]);
if (!node.success) failures.push('node not found — test:npm runs the package under it');
else if (!engines_floor || !(node_major >= Number(engines_floor[1]))) {
	failures.push(`node ${node.out} is below the package's engines floor (${NPM_SHARED_METADATA.engines.node})`);
}
for (const [tool, why] of [
	['wasm-pack', 'builds the package (`cargo install wasm-pack`)'],
	['gzip', 'the size gate measures the wire size with it'],
	['cargo', 'the bump moves the lockfiles with it'],
] as const) {
	if (!(await command_capture(tool, ['--version'])).success) failures.push(`${tool} not found — ${why}`);
}

// A fresh release starts from a clean tree; a resume's tree may hold its own bump.
const status = await git_status_lines();
if (!status.ok) refuse_or_warn(status.error);
else {
	const stray =
		plan.kind === 'fresh'
			? status.lines
			: status.lines.filter((line) => !RELEASE_PATHS.includes(git_status_line_path(line)));
	if (stray.length > 0) {
		const allowed = plan.kind === 'fresh' ? '' : ` outside the release files (${RELEASE_PATHS.join(', ')})`;
		refuse_or_warn(`working tree has changes${allowed} — commit or stash them first:\n${stray.join('\n')}`);
	}
}

const branch = await git_current_branch();
if (branch === null) refuse_or_warn('git branch failed');
else if (branch !== 'main') refuse_or_warn(`${branch ? `on branch "${branch}"` : 'HEAD is detached'} — releases publish from main`);
const head = await git_rev('HEAD');
if (head === null) refuse_or_warn('git rev-parse HEAD failed');

// The published source must be the public source. A resume may sit one commit ahead, when that
// commit is its own release commit and only the push failed.
const compared = await git_fetch_and_compare_origin_main();
if (!compared.ok) refuse_or_warn(`${compared.error} — cannot compare HEAD with origin`);
else {
	const {ahead, behind} = compared;
	const own_release_commit =
		plan.kind === 'resume' &&
		behind === 0 &&
		ahead === 1 &&
		(await git_head_subject()) === release_commit_message(plan.version);
	if ((behind > 0 || ahead > 0) && !own_release_commit) {
		refuse_or_warn(`HEAD is ${ahead} ahead / ${behind} behind origin/main — release from exactly origin/main`);
	}
}

// A fresh release's tag must not exist yet; a resume's may, when it is on the release commit.
if (release_tag !== null) {
	const local = await git_local_tag_commit(release_tag);
	const remote = await git_remote_tag_commit(release_tag);
	if (!remote.ok) refuse_or_warn(remote.error);
	for (const [where, commit] of [
		['locally', local],
		['on origin', remote.ok ? remote.commit : null],
	] as const) {
		if (commit === null) continue;
		if (plan.kind === 'fresh') refuse_or_warn(`tag ${release_tag} already exists ${where}`);
		else if (commit !== head) refuse_or_warn(`tag ${release_tag} exists ${where} on ${commit}, not on HEAD`);
	}
}

const who = await npm_whoami();
if (who.ok) console.log(`npm user: ${who.user}`);
else refuse_or_warn(`npm whoami failed — not logged in to npm (run \`npm login\`): ${who.error}`);

/** Whether a resumed release's package is already out, leaving only git to finish. */
let already_published = false;
if (release_version !== null) {
	const published = await npm_is_published(NAME, release_version);
	if (!published.ok) failures.push(published.error);
	else if (published.published) {
		if (plan.kind === 'fresh') failures.push(`${NAME}@${release_version} is already on npm — release past it`);
		else already_published = true;
	}
}

if (failures.length > 0 || refusals.length > 0) {
	for (const message of [...failures, ...refusals]) console.error(`✗ ${message}`);
	fail(`refusing to ${wetrun ? 'release' : 'run'} — nothing was changed, built or published`);
}

if (already_published) {
	console.log(`${NAME}@${release_version} is already on npm — only the release commit, tag and push remain`);
	if (!wetrun) {
		console.log('\nDRY RUN — nothing changed. `deno task publish:npm --wetrun` finishes them.');
		Deno.exit(0);
	}
}

// Bump. A fresh wetrun writes the version, then the sentinel, at once, so any later failure
// resumes; the lockfiles and the changelog are idempotent and repeat on a resume.

/** The version the gates build: the bumped one under `--wetrun`, the checkout's in a dry run. */
let build_version: string = cargo_version;
if (wetrun) {
	if (release_version === null) fail('no release version — the plan should have refused');
	if (plan.kind === 'fresh') {
		if (plan.stale_sentinel !== null) await sentinel_remove();
		const written = await bump_cargo_version(release_version);
		if (!written.ok) fail(written.error);
		await sentinel_write(release_version);
		console.log(`\nbumped ${CARGO_PATH}: ${plan.version} → ${release_version}`);
	}
	bumped = true;
	build_version = release_version;
	const locked = await bump_lockfiles();
	if (!locked.ok) fail(`${locked.error}${resume_hint()}`);
	const stamped = await bump_changelog(release_version);
	if (stamped === 'no_unreleased') fail(`no "## Unreleased" or "## ${release_version}" in ${CHANGELOG_PATH}${resume_hint()}`);
	console.log(`${CHANGELOG_PATH}: ## ${release_version} ${stamped === 'already_stamped' ? 'already stamped' : 'stamped'}`);
}
const id = `${NAME}@${build_version}`;

// Gates and publish, unless a resume finds the package already out.

if (!already_published) {
	const before = await git_checkout_fingerprint();

	// `test:npm` builds `pkg-npm`, so it also assembles the package, and runs last of the suites
	// so nothing after it rebuilds what ships.
	console.log('\nrunning checks …');
	await step('deno task check', 'deno', ['task', 'check']);
	console.log('\ntesting the engine (native + wasm) …');
	await step('deno task test:rust', 'deno', ['task', 'test:rust']);
	await step('deno task test:wasm', 'deno', ['task', 'test:wasm']);
	console.log(`\nbuilding + testing ${NAME} …`);
	await step('deno task test:npm', 'deno', ['task', 'test:npm'], {env: {PKG_DIR: PKG}});

	// wasm-pack stamps the version from Cargo.toml and the patch script the name; assert both
	// rather than trust the chain, since this is the file npm publishes from.
	const pkg = JSON.parse(await Deno.readTextFile(`${PKG}/package.json`));
	if (`${pkg.name}@${pkg.version}` !== id) fail(`${PKG} built as ${pkg.name}@${pkg.version}, expected ${id}${resume_hint()}`);

	// the size of the artifact that ships, not the sibling `pkg` build `size:check` measures
	console.log('\ngating the shipped wasm size …');
	await step('size gate (pkg-npm)', 'deno', [
		'run',
		'--frozen',
		'--allow-read',
		'--allow-run=gzip,git',
		'benches/size.ts',
		'--check',
		'--wasm',
		`${PKG}/pglet_wasm_bg.wasm`,
	]);

	// `pkg-npm` holds what the checkout held while `test:npm` built it: a checkout that moved
	// since refuses under `--wetrun` rather than publish a build no commit describes.
	const after = await git_checkout_fingerprint();
	if (!before.ok || !after.ok || before.fingerprint !== after.fingerprint) {
		const moved = 'the checkout changed while the gates ran — the built package may match no commit';
		if (wetrun) fail(`${moved}; refusing to publish${resume_hint()}`);
		warnings++;
		console.warn(`⚠ ${moved}`);
	}

	// npm writes the tarball notice (name@version, the file list, the registry) to stderr and
	// `+ name@version` to stdout
	if (!wetrun) {
		const dry_args = ['publish', '--dry-run'];
		console.log(`\n(cd ${PKG} && npm ${dry_args.join(' ')})`);
		const dry = await command_capture('npm', dry_args, {cwd: PKG});
		if (dry.err) console.log(dry.err);
		if (dry.out) console.log(dry.out);
		// a dry run never bumps, so previewing a fresh release packs the version already on npm
		const republish_preview = plan.kind === 'fresh' && npm_error_is_republish(dry.err);
		if (!dry.success && !republish_preview) fail(`npm ${dry_args.join(' ')} failed`);
		const reported = dry.out.split('\n').at(-1);
		if (dry.success && reported !== `+ ${id}`) {
			fail(`npm publish --dry-run reported ${JSON.stringify(reported)}, expected "+ ${id}"`);
		}
		const next =
			warnings > 0
				? 'A wetrun refuses until every ⚠ above is resolved.'
				: plan.kind === 'resume'
					? `\`deno task publish:npm --wetrun\` resumes the release of ${id}.`
					: `\`deno task publish:npm --wetrun --bump ${plan.level}\` releases ${NAME}@${plan.target}.`;
		console.log(`\nDRY RUN — nothing changed or published. ${next}`);
		Deno.exit(0);
	}

	console.log(`\n(cd ${PKG} && npm publish)`);
	await step('npm publish', 'npm', ['publish'], {cwd: PKG});
	console.log(`\n✓ published ${id}`);
}

// Git finalize, reached only under `--wetrun`. Past the publish the package is public, so every
// failure here says so and how to finish.

// every dry-run path exits above; this guards a future edit that falls through
if (!wetrun) fail('a dry run reached the git finalize — refusing to commit, tag or push');

const tag = version_tag(build_version);
const message = release_commit_message(build_version);
const manual_finalize = [
	`  git add ${RELEASE_PATHS.join(' ')}`,
	`  git commit -m "${message}"`,
	`  git tag -m ${tag} ${tag}`,
	'  git push --follow-tags origin main',
].join('\n');

function git_fail(error: string): never {
	fail(
		`${error}\n  ${id} is PUBLISHED — only the release commit, tag and push are missing.\n` +
			'  `deno task publish:npm --wetrun` resumes from here (it skips the gates and the publish),\n' +
			`  or finish by hand:\n${manual_finalize}`,
	);
}

if (no_git) {
	console.log(`\n--no-git: finish the release by hand (only the release files; an annotated tag):\n${manual_finalize}`);
} else {
	console.log('\nfinalizing git …');
	const committed = await git_commit_paths(RELEASE_PATHS, message);
	if (!committed.ok) git_fail(committed.error);
	console.log(committed.committed ? `committed "${message}"` : 'release commit already made');
	if ((await git_local_tag_commit(tag)) === null) {
		const tagged = await git_tag_annotated(tag);
		if (!tagged.ok) git_fail(tagged.error);
		console.log(`tagged ${tag}`);
	} else console.log(`tag ${tag} already exists`);
	if (!(await git_push_main_with_tags())) git_fail('git push --follow-tags origin main failed');
}

// last, so a failed finalize stays resumable
await sentinel_remove();
console.log(`\n✓ released ${id}${no_git ? ' (git finalize left to you)' : ` — ${tag} pushed`}`);
