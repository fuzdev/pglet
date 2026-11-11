/**
 * The publish command, the body of `npm run publish:npm`: releases
 * `@fuzdev/pglet-wasm` by bumping, gating, publishing to npm, and committing,
 * tagging and pushing the release. Publishing is the maintainer's call, so a
 * run is a dry run that changes nothing unless `wetrun` is set.
 *
 * **The version** is `Cargo.toml`'s `[workspace.package]` version, which
 * wasm-pack stamps into the package. A fresh release needs a non-empty
 * `## Unreleased` section; the bump writes the new version, both lockfiles'
 * workspace members (the root's and `fuzz/`'s), and the changelog, whose
 * `## Unreleased` becomes `## <version>` under a fresh `## Unreleased`
 * declaring a patch. Those files, and only those, make the release commit
 * (`release/bump.ts`).
 *
 * **Preflight**, before anything is mutated or built, so nothing that can be
 * known up front is learned minutes in. Always refused: a missing tool (`node`
 * below the package's `engines` floor, `wasm-pack`, `gzip`, `cargo`), an
 * unreadable version, a failed registry lookup, and a fresh release whose
 * version is already on npm. A warning in a dry run and a refusal on a wetrun,
 * collected so one run names every reason: the release plan's problems (the
 * changelog and `bump`, `release/plan.ts`), a dirty working tree, a branch
 * other than `main`, a `HEAD` that isn't exactly `origin/main` (the published
 * source must be the public source), an existing `v<version>` tag, locally or
 * on `origin`, and a failing `npm whoami`.
 *
 * **Gates**, which a dry run runs too, so a dry run validates the release:
 *
 * 1. `npm run check`: clippy, the stripped and wasm-feature builds, fmt, docs,
 *    `gro check` over `src/`, the release-audit firewall
 * 2. `npm run test:rust` and `npm run test:wasm`: the engine's own suites,
 *    natively and through the wasm binding; neither needs Postgres
 * 3. `npm run test:npm`: builds `pkg-npm` from an emptied directory and runs
 *    the package suite, ending in the packed-tarball smoke over that same
 *    directory, with `PKG_DIR` naming it (the task root sets it on `npm`), so
 *    an exported `PKG_DIR` can't point the Node legs at another build, and
 *    nothing between it and the publish rebuilds `pkg-npm`
 * 4. the built `package.json`'s `name@version` against the registry identity
 *    and the version
 * 5. the size gate on the shipped `pkg-npm/pglet_wasm_bg.wasm`, its task run by `gro`
 *
 * On a wetrun, a checkout that changed while the gates ran refuses to publish.
 *
 * **Publish** in `pkg-npm`: `npm publish --dry-run` in a dry run, which must
 * report exactly the built `name@version` (or, when the dry run previews a
 * bump, refuse only as a republish of the unbumped version); `npm publish
 * --no-dry-run` on a wetrun, its mode stated whatever npm's config says, and
 * streamed, so npm's browser auth reaches the terminal. The package's own
 * `publishConfig` makes it public.
 *
 * **Git finalize**: commit the release files as `chore: release v<version>`,
 * create the annotated tag `v<version>`, and `git push --follow-tags origin main`.
 *
 * **Resume.** A wetrun writes `.publish-in-progress` (gitignored,
 * `release/sentinel.ts`) right after the bump and removes it only once the push
 * lands. While it names the checkout's version, a wetrun resumes that release:
 * it skips the bump, accepts a tree dirty with only the release files or one
 * release commit ahead of `origin/main`, and repeats every later step, each of
 * which is idempotent. A resumed release whose version is already on npm skips
 * the gates and the publish and only finishes git.
 *
 * @module
 */

import type { FsDir } from '@fuzdev/gro/fs.ts';
import { SilentError } from '@fuzdev/gro/task.ts';

import { NPM_SHARED_METADATA } from '../npm_metadata.ts';
import {
	bump_cargo_version,
	bump_changelog,
	bump_lockfiles,
	release_commit_message,
	RELEASE_PATHS
} from './bump.ts';
import { CHANGELOG_PATH } from './changelog.ts';
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
	git_tag_annotated
} from './git.ts';
import { npm_error_is_republish, npm_is_published, npm_whoami } from './npm.ts';
import { release_plan } from './plan.ts';
import { release_capture, release_stream, type ReleaseProgram } from './programs.ts';
import { sentinel_read, sentinel_remove, sentinel_write } from './sentinel.ts';
import {
	type BumpLevel,
	CARGO_PATH,
	cargo_workspace_version,
	is_bump_level,
	version_tag
} from './version.ts';

/** The usage line every argument error ends with. */
export const PUBLISH_USAGE =
	'usage: npm run publish:npm -- [--wetrun] [--bump patch|minor|major] [--no-git]';

/**
 * The variables `npm run` sets when it takes one of the command's flags, or its
 * own `--dry-run`, written before its `--`: the command never sees such a flag,
 * so a run that went ahead would drop what the user wrote, like a cautious
 * `--dry-run` or `--no-git` before a wetrun.
 */
export const PUBLISH_NPM_FLAG_VARS = [
	'npm_config_wetrun',
	'npm_config_bump',
	'npm_config_git',
	'npm_config_dry_run'
] as const;

/** The values a `--git` or `--no-git` npm swallowed gives `npm_config_git`. */
const SWALLOWED_GIT_VALUES = ['true', 'false', ''];

/**
 * Which of `PUBLISH_NPM_FLAG_VARS` are set, as a swallowed flag sets them. npm's
 * own `git` config names the git program, so `npm_config_git` counts only with
 * a value a swallowed flag gives it: `true`, `false`, or empty, which a bare
 * `--git` or a negation in any other case (`--No-Git`) gives it.
 *
 * @param get_var - reads one variable, `undefined` when unset
 * @returns the names of the variables a swallowed flag set
 */
export const to_publish_stray_vars = (
	get_var: (name: string) => string | undefined
): Array<string> =>
	PUBLISH_NPM_FLAG_VARS.filter((name) => {
		const value = get_var(name);
		if (value === undefined) return false;
		return name !== 'npm_config_git' || SWALLOWED_GIT_VALUES.includes(value);
	});

/** The directory the package is built in and published from. */
export const PUBLISH_PKG_DIR = 'crates/pglet_wasm/pkg-npm';

/** The command's args, the task's `Args` after validation. */
export interface PublishCommandArgs {
	/** Bump, publish, commit, tag and push; a dry run without it. */
	wetrun: boolean;
	/** The release's level, which must repeat the changelog's marker; unchecked until the command reads it. */
	bump?: string | undefined;
	/** Commit, tag and push after the publish; without it, print those steps instead. */
	git: boolean;
	/**
	 * What was given but never reached the args, refusing the run when any is:
	 * each of `PUBLISH_NPM_FLAG_VARS` that's set, and the CLI args from a `--` on.
	 */
	stray: ReadonlyArray<string>;
}

/** What the command reaches, gathered by the task root. */
export interface PublishCommandDeps {
	/** This checkout, where the release files, the sentinel and `pkg-npm` are. */
	checkout: FsDir;
	/** `git` for the local reads: status, branch, rev-parse, log, diff. */
	git_local: ReleaseProgram;
	/** `git` for what reaches `origin` or writes history: fetch, ls-remote, commit, tag, push. */
	git_remote: ReleaseProgram;
	/** `npm`: the registry questions, the publish, and the gates `package.json` runs. */
	npm: ReleaseProgram;
	/** `gro`: runs the size gate's task on the shipped wasm. */
	gro: ReleaseProgram;
	/** `cargo`: probed, and the bump moves the lockfiles with it. */
	cargo: ReleaseProgram;
	/** `node`: probed against the package's `engines` floor. */
	node: ReleaseProgram;
	/** `wasm-pack`: probed, the gates build the package with it. */
	wasm_pack: ReleaseProgram;
	/** `gzip`: probed, the size gate measures with it. */
	gzip: ReleaseProgram;
	/** Prints a block to stdout, as `console.log`. */
	print: (text: string) => void;
	/** Prints a block to stderr, as `console.error`. */
	print_error: (text: string) => void;
}

/**
 * Runs one release, a dry run unless `args.wetrun`; see the module doc for the
 * steps. A refusal or a failed step prints `✗ <reason>` and fails the task.
 *
 * @throws SilentError - exit 1 after printing the reason
 */
export const run_publish_command = async (
	deps: PublishCommandDeps,
	args: PublishCommandArgs
): Promise<void> => {
	const { checkout, git_local, git_remote, npm, gro, cargo, node, print, print_error } = deps;
	const { wetrun } = args;
	const name = NPM_SHARED_METADATA.name;

	const fail = (message: string): never => {
		print_error(`✗ ${message}`);
		throw new SilentError();
	};

	if (args.stray.length > 0) {
		fail(
			`refusing: ${args.stray.join(' ')} never reached the command; write its flags after npm's \`--\` and nothing after a second one\n${PUBLISH_USAGE}`
		);
	}

	let bump: BumpLevel | null = null;
	if (args.bump !== undefined) {
		if (!is_bump_level(args.bump)) {
			fail(
				`--bump takes patch, minor or major (got ${JSON.stringify(args.bump)})\n${PUBLISH_USAGE}`
			);
		}
		bump = args.bump as BumpLevel;
	}

	/** Set once the bump is written: from then on a failure names the resume. */
	let bumped = false;
	const resume_hint = (): string =>
		bumped
			? '\n  the bump is in place — fix the cause and run `npm run publish:npm -- --wetrun` to resume'
			: '';

	/** Runs a long step streaming its output, failing the run when it fails. */
	const step = async (
		label: string,
		program: ReleaseProgram,
		step_args: Array<string>
	): Promise<void> => {
		const { success, ended } = await release_stream(program, step_args, print_error);
		if (!success) fail(`${label} failed (${ended})${resume_hint()}`);
	};

	// Plan. Everything a release decides from is read here, before anything is touched.

	const cargo_version = cargo_workspace_version(await checkout.read_text(CARGO_PATH));
	if (cargo_version === null) return fail(`no [workspace.package] version in ${CARGO_PATH}`);
	const changelog = await checkout.read_text_if_exists(CHANGELOG_PATH);
	const plan = release_plan({
		version: cargo_version,
		bump,
		sentinel: await sentinel_read(checkout),
		changelog
	});
	// the version this run releases; null only when a fresh plan has no target, which a wetrun refuses
	const release_version = plan.kind === 'resume' ? plan.version : plan.target;
	const release_tag = release_version === null ? null : version_tag(release_version);

	print(`=== ${name} release ${wetrun ? '(wetrun)' : '(dry run)'} ===`);
	if (plan.kind === 'resume') {
		print(`resuming the interrupted release of ${name}@${plan.version}`);
		for (const note of plan.notes) print_error(`⚠ ${note}`);
	} else {
		print(
			`${name}: ${plan.version} → ${plan.target ?? '?'}${plan.level ? ` (${plan.level})` : ''}`
		);
		if (plan.stale_sentinel !== null) {
			print_error(
				`⚠ a sentinel names v${plan.stale_sentinel}, not this checkout's version — a wetrun removes it`
			);
		}
	}

	// Preflight. Tools, the registry lookup and a republish refuse in both modes; the
	// release-state checks are a warning in a dry run and a refusal on a wetrun.

	const failures: Array<string> = [];
	const refusals: Array<string> = [];
	// how many refusals a dry run downgraded to warnings, so its last line can say a wetrun would refuse
	let warnings = 0;
	const refuse_or_warn = (message: string): void => {
		if (wetrun) refusals.push(message);
		else {
			warnings++;
			print_error(`⚠ ${message}`);
		}
	};

	if (plan.kind === 'fresh') for (const problem of plan.problems) refuse_or_warn(problem);

	const engines_floor = /^>=(\d+)$/.exec(NPM_SHARED_METADATA.engines.node);
	const node_version = await release_capture(node, ['--version']);
	const node_major = Number(/^v(\d+)\./.exec(node_version.out)?.[1]);
	if (!node_version.success) failures.push('node not found — test:npm runs the package under it');
	else if (!engines_floor || !(node_major >= Number(engines_floor[1]))) {
		failures.push(
			`node ${node_version.out} is below the package's engines floor (${NPM_SHARED_METADATA.engines.node})`
		);
	}
	for (const [tool, why] of [
		[deps.wasm_pack, 'builds the package (`cargo install wasm-pack`)'],
		[deps.gzip, 'the size gate measures the wire size with it'],
		[cargo, 'the bump moves the lockfiles with it']
	] as const) {
		if (!(await release_capture(tool, ['--version'])).success) {
			failures.push(`${tool.name} not found — ${why}`);
		}
	}

	// a fresh release starts from a clean tree; a resume's tree may hold its own bump
	const status = await git_status_lines(git_local);
	if (!status.ok) refuse_or_warn(status.error);
	else {
		const stray =
			plan.kind === 'fresh'
				? status.lines
				: status.lines.filter((line) => !RELEASE_PATHS.includes(git_status_line_path(line)));
		if (stray.length > 0) {
			const allowed =
				plan.kind === 'fresh' ? '' : ` outside the release files (${RELEASE_PATHS.join(', ')})`;
			refuse_or_warn(
				`working tree has changes${allowed} — commit or stash them first:\n${stray.join('\n')}`
			);
		}
	}

	const branch = await git_current_branch(git_local);
	if (branch === null) refuse_or_warn('git branch failed');
	else if (branch !== 'main') {
		refuse_or_warn(
			`${branch ? `on branch "${branch}"` : 'HEAD is detached'} — releases publish from main`
		);
	}
	const head = await git_rev(git_local, 'HEAD');
	if (head === null) refuse_or_warn('git rev-parse HEAD failed');

	// The published source must be the public source. A resume may sit one commit ahead,
	// when that commit is its own release commit and only the push failed.
	const compared = await git_fetch_and_compare_origin_main(git_remote, git_local);
	if (!compared.ok) refuse_or_warn(`${compared.error} — cannot compare HEAD with origin`);
	else {
		const { ahead, behind } = compared;
		const own_release_commit =
			plan.kind === 'resume' &&
			behind === 0 &&
			ahead === 1 &&
			(await git_head_subject(git_local)) === release_commit_message(plan.version);
		if ((behind > 0 || ahead > 0) && !own_release_commit) {
			refuse_or_warn(
				`HEAD is ${ahead} ahead / ${behind} behind origin/main — release from exactly origin/main`
			);
		}
	}

	// a fresh release's tag must not exist yet; a resume's may, when it's on the release commit
	if (release_tag !== null) {
		const local = await git_local_tag_commit(git_local, release_tag);
		const remote = await git_remote_tag_commit(git_remote, release_tag);
		if (!remote.ok) refuse_or_warn(remote.error);
		for (const [where, commit] of [
			['locally', local],
			['on origin', remote.ok ? remote.commit : null]
		] as const) {
			if (commit === null) continue;
			if (plan.kind === 'fresh') refuse_or_warn(`tag ${release_tag} already exists ${where}`);
			else if (commit !== head) {
				refuse_or_warn(`tag ${release_tag} exists ${where} on ${commit}, not on HEAD`);
			}
		}
	}

	const who = await npm_whoami(npm);
	if (who.ok) print(`npm user: ${who.user}`);
	else refuse_or_warn(`npm whoami failed — not logged in to npm (run \`npm login\`): ${who.error}`);

	// whether a resumed release's package is already out, leaving only git to finish
	let already_published = false;
	if (release_version !== null) {
		const published = await npm_is_published(npm, name, release_version);
		if (!published.ok) failures.push(published.error);
		else if (published.published) {
			if (plan.kind === 'fresh') {
				failures.push(`${name}@${release_version} is already on npm — release past it`);
			} else already_published = true;
		}
	}

	if (failures.length > 0 || refusals.length > 0) {
		for (const message of [...failures, ...refusals]) print_error(`✗ ${message}`);
		fail(`refusing to ${wetrun ? 'release' : 'run'} — nothing was changed, built or published`);
	}

	if (already_published) {
		print(
			`${name}@${release_version} is already on npm — only the release commit, tag and push remain`
		);
		if (!wetrun) {
			print('\nDRY RUN — nothing changed. `npm run publish:npm -- --wetrun` finishes them.');
			return;
		}
	}

	// Bump. A fresh wetrun writes the version, then the sentinel, at once, so any later
	// failure resumes; the lockfiles and the changelog are idempotent and repeat on a resume.

	// the version the gates build: the bumped one on a wetrun, the checkout's in a dry run
	let build_version: string = cargo_version;
	if (wetrun) {
		if (release_version === null) return fail('no release version — the plan should have refused');
		if (plan.kind === 'fresh') {
			if (plan.stale_sentinel !== null) await sentinel_remove(checkout);
			const written = await bump_cargo_version(checkout, release_version);
			if (!written.ok) fail(written.error);
			await sentinel_write(checkout, release_version);
			print(`\nbumped ${CARGO_PATH}: ${plan.version} → ${release_version}`);
		}
		bumped = true;
		build_version = release_version;
		const locked = await bump_lockfiles(cargo, checkout);
		if (!locked.ok) fail(`${locked.error}${resume_hint()}`);
		const stamped = await bump_changelog(checkout, release_version);
		if (stamped === 'no_unreleased') {
			fail(`no "## Unreleased" or "## ${release_version}" in ${CHANGELOG_PATH}${resume_hint()}`);
		}
		print(
			`${CHANGELOG_PATH}: ## ${release_version} ${stamped === 'already_stamped' ? 'already stamped' : 'stamped'}`
		);
	}
	const id = `${name}@${build_version}`;

	// Gates and publish, unless a resume finds the package already out.

	if (!already_published) {
		const before = await git_checkout_fingerprint(git_local);

		// `test:npm` builds `pkg-npm`, so it also assembles the package, and runs last of the
		// suites so nothing after it rebuilds what ships
		print('\nrunning checks …');
		await step('npm run check', npm, ['run', 'check']);
		print('\ntesting the engine (native + wasm) …');
		await step('npm run test:rust', npm, ['run', 'test:rust']);
		await step('npm run test:wasm', npm, ['run', 'test:wasm']);
		print(`\nbuilding + testing ${name} …`);
		await step('npm run test:npm', npm, ['run', 'test:npm']);

		// wasm-pack stamps the version from Cargo.toml and the patch task the name; assert both
		// rather than trust the chain, since this is the file npm publishes from
		const pkg = JSON.parse(await checkout.read_text(`${PUBLISH_PKG_DIR}/package.json`));
		if (`${pkg.name}@${pkg.version}` !== id) {
			fail(
				`${PUBLISH_PKG_DIR} built as ${pkg.name}@${pkg.version}, expected ${id}${resume_hint()}`
			);
		}

		// the size of the artifact that ships, not the sibling `pkg` build `size:check` measures
		print('\ngating the shipped wasm size …');
		await step('size gate (pkg-npm)', gro, [
			'task',
			'src/tasks/size.task.ts',
			'--check',
			'--wasm',
			`${PUBLISH_PKG_DIR}/pglet_wasm_bg.wasm`
		]);

		// `pkg-npm` holds what the checkout held while `test:npm` built it: a checkout that
		// moved since refuses on a wetrun rather than publish a build no commit describes
		const after = await git_checkout_fingerprint(git_local);
		if (!before.ok || !after.ok || before.fingerprint !== after.fingerprint) {
			const moved =
				'the checkout changed while the gates ran — the built package may match no commit';
			if (wetrun) fail(`${moved}; refusing to publish${resume_hint()}`);
			warnings++;
			print_error(`⚠ ${moved}`);
		}

		const pkg_dir = checkout.dir(PUBLISH_PKG_DIR);

		// npm writes the tarball notice (name@version, the file list, the registry) to stderr
		// and `+ name@version` to stdout
		if (!wetrun) {
			const dry_args = ['publish', '--dry-run'];
			print(`\n(cd ${PUBLISH_PKG_DIR} && npm ${dry_args.join(' ')})`);
			const dry = await release_capture(npm, dry_args, { cwd: pkg_dir });
			if (dry.err) print(dry.err);
			if (dry.out) print(dry.out);
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
						? `\`npm run publish:npm -- --wetrun\` resumes the release of ${id}.`
						: `\`npm run publish:npm -- --wetrun --bump ${plan.level}\` releases ${name}@${plan.target}.`;
			print(`\nDRY RUN — nothing changed or published. ${next}`);
			return;
		}

		// the mode is stated, so a `dry-run` npm reads from the environment (in any spelling)
		// or a config file never turns the publish into a dry run that succeeds
		const publish_args = ['publish', '--no-dry-run'];
		print(`\n(cd ${PUBLISH_PKG_DIR} && npm ${publish_args.join(' ')})`);
		const published = await release_stream(npm, publish_args, print_error, { cwd: pkg_dir });
		if (!published.success) fail(`npm publish failed (${published.ended})${resume_hint()}`);
		print(`\n✓ published ${id}`);
	}

	// Git finalize, reached only on a wetrun. Past the publish the package is public, so
	// every failure here says so and how to finish.

	// every dry-run path returns above; this guards a future edit that falls through
	if (!wetrun) fail('a dry run reached the git finalize — refusing to commit, tag or push');

	const tag = version_tag(build_version);
	const message = release_commit_message(build_version);
	const manual_finalize = [
		`  git add ${RELEASE_PATHS.join(' ')}`,
		`  git commit -m "${message}"`,
		`  git tag -m ${tag} ${tag}`,
		'  git push --follow-tags origin main'
	].join('\n');

	const git_fail = (error: string): never =>
		fail(
			`${error}\n  ${id} is PUBLISHED — only the release commit, tag and push are missing.\n` +
				'  `npm run publish:npm -- --wetrun` resumes from here (it skips the gates and the publish),\n' +
				`  or finish by hand:\n${manual_finalize}`
		);

	if (!args.git) {
		print(
			`\n--no-git: finish the release by hand (only the release files; an annotated tag):\n${manual_finalize}`
		);
	} else {
		print('\nfinalizing git …');
		const committed = await git_commit_paths(git_remote, git_local, RELEASE_PATHS, message);
		if (!committed.ok) git_fail(committed.error);
		else print(committed.committed ? `committed "${message}"` : 'release commit already made');
		if ((await git_local_tag_commit(git_local, tag)) === null) {
			const tagged = await git_tag_annotated(git_remote, tag);
			if (!tagged.ok) git_fail(tagged.error);
			print(`tagged ${tag}`);
		} else print(`tag ${tag} already exists`);
		if (!(await git_push_main_with_tags(git_remote, print_error))) {
			git_fail('git push --follow-tags origin main failed');
		}
	}

	// last, so a failed finalize stays resumable
	await sentinel_remove(checkout);
	print(`\n✓ released ${id}${args.git ? ` — ${tag} pushed` : ' (git finalize left to you)'}`);
};
