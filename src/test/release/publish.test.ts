import { assert, describe, test } from 'vitest';

import { PUBLISH_USAGE, to_publish_stray_vars } from '#lib/release/publish.ts';

import {
	CARGO_TOML,
	CHANGELOG,
	happy_release_script,
	NEXT_VERSION,
	PKG_NAME,
	run_release,
	to_call_line,
	type ReleaseScript
} from './release_helpers.ts';

/** The preflight every run makes, in order, for a fresh release to `NEXT_VERSION`. */
const PREFLIGHT = [
	'node --version',
	'wasm_pack --version',
	'gzip --version',
	'cargo --version',
	'git_local status --porcelain',
	'git_local branch --show-current',
	'git_local rev-parse -q --verify HEAD^{commit}',
	'git_remote fetch origin main',
	'git_local rev-list --left-right --count origin/main...HEAD',
	`git_local rev-parse -q --verify refs/tags/v${NEXT_VERSION}^{commit}`,
	`git_remote ls-remote --tags origin refs/tags/v${NEXT_VERSION} refs/tags/v${NEXT_VERSION}^{}`,
	'npm whoami',
	`npm view ${PKG_NAME}@${NEXT_VERSION} version`
];

const FINGERPRINT = [
	'git_local rev-parse HEAD',
	'git_local status --porcelain --untracked-files=all',
	'git_local diff --binary HEAD'
];

/** The gates, in order, ending with the size gate on the shipped wasm. */
const GATES = [
	'npm run check',
	'npm run test:rust',
	'deno task test:wasm',
	'deno task test:npm',
	'deno run --frozen --allow-read --allow-run=gzip,git benches/size.ts --check --wasm crates/pglet_wasm/pkg-npm/pglet_wasm_bg.wasm'
];

const RELEASE_FILES = 'Cargo.toml Cargo.lock fuzz/Cargo.lock CHANGELOG.md';

const BUMP = ['cargo update --workspace --offline', 'cargo update --workspace --offline (in fuzz)'];

const FINALIZE = [
	`git_local status --porcelain -- ${RELEASE_FILES}`,
	`git_remote commit -m chore: release v${NEXT_VERSION} -- ${RELEASE_FILES}`,
	`git_local rev-parse -q --verify refs/tags/v${NEXT_VERSION}^{commit}`,
	`git_remote tag -m v${NEXT_VERSION} v${NEXT_VERSION}`,
	'git_remote push --follow-tags origin main'
];

/** The wetrun's publish, its mode stated on the command line. */
const PUBLISH_CALL = 'npm publish --no-dry-run (in crates/pglet_wasm/pkg-npm)';

const WETRUN_CALLS = [
	...PREFLIGHT,
	...BUMP,
	...FINGERPRINT,
	...GATES,
	...FINGERPRINT,
	PUBLISH_CALL,
	...FINALIZE
];

/** A script answering as the happy world but for the `index`th call, which fails. */
const failing_at =
	(index: number): ReleaseScript =>
	(call, world) =>
		world.calls.length - 1 === index
			? { code: 1, stderr: 'boom\n' }
			: happy_release_script(call, world);

const STAMPED_CHANGELOG = CHANGELOG.replace(
	'## Unreleased\n<!-- bump: patch -->\n',
	`## Unreleased\n<!-- bump: patch -->\n\n## ${NEXT_VERSION}\n`
);

describe('a dry run', () => {
	test('asks, gates and previews the publish, in order, and writes nothing', async () => {
		const run = await run_release({ bump: 'patch' });
		assert.isFalse(run.failed, run.stderr.join('\n'));
		assert.deepEqual(run.lines, [
			...PREFLIGHT,
			...FINGERPRINT,
			...GATES,
			...FINGERPRINT,
			'npm publish --dry-run (in crates/pglet_wasm/pkg-npm)'
		]);
		assert.strictEqual(await run.read('Cargo.toml'), CARGO_TOML);
		assert.strictEqual(await run.read('CHANGELOG.md'), CHANGELOG);
		assert.isNull(await run.read('.publish-in-progress'));
		assert.deepEqual(run.stderr, []);
		assert.strictEqual(
			run.stdout.at(-1),
			`\nDRY RUN — nothing changed or published. \`npm run publish:npm -- --wetrun --bump patch\` releases ${PKG_NAME}@${NEXT_VERSION}.`
		);
	});

	test('every captured call keeps the terminal as stdin, and every program runs in the checkout', async () => {
		const run = await run_release({ bump: 'patch' });
		for (const call of run.calls) {
			assert.isTrue(call.stdin_inherited, to_call_line(call));
			assert.notStrictEqual(call.cwd, '<project>', to_call_line(call));
		}
	});

	test('warns about every release-state problem in one run and says a wetrun would refuse', async () => {
		const run = await run_release(
			{},
			{
				script: (call, world) => {
					const line = [call.program, ...call.args].join(' ');
					if (line === 'git_local status --porcelain') return { stdout: '?? stray.txt\n' };
					if (line === 'git_local branch --show-current') return { stdout: 'topic\n' };
					return happy_release_script(call, world);
				}
			}
		);
		assert.isFalse(run.failed);
		assert.deepEqual(run.stderr, [
			'⚠ no --bump — pass the level the changelog marker declares',
			'⚠ working tree has changes — commit or stash them first:\n?? stray.txt',
			'⚠ on branch "topic" — releases publish from main'
		]);
		assert.strictEqual(
			run.stdout.at(-1),
			'\nDRY RUN — nothing changed or published. A wetrun refuses until every ⚠ above is resolved.'
		);
	});
});

describe('a wetrun', () => {
	test('asks, bumps, gates, publishes and finalizes git, in order', async () => {
		const run = await run_release({ wetrun: true, bump: 'patch' });
		assert.isFalse(run.failed, run.stderr.join('\n'));
		assert.deepEqual(run.lines, WETRUN_CALLS);
		assert.strictEqual(
			await run.read('Cargo.toml'),
			CARGO_TOML.replace('version = "0.4.7"', `version = "${NEXT_VERSION}"`)
		);
		assert.strictEqual(await run.read('CHANGELOG.md'), STAMPED_CHANGELOG);
		assert.isNull(await run.read('.publish-in-progress'));
		assert.strictEqual(
			run.stdout.at(-1),
			`\n✓ released ${PKG_NAME}@${NEXT_VERSION} — v${NEXT_VERSION} pushed`
		);
	});

	test('refuses a dirty tree before anything is written', async () => {
		const run = await run_release(
			{ wetrun: true, bump: 'patch' },
			{
				script: (call, world) =>
					[call.program, ...call.args].join(' ') === 'git_local status --porcelain'
						? { stdout: ' M src/x.ts\n' }
						: happy_release_script(call, world)
			}
		);
		assert.isTrue(run.failed);
		assert.deepEqual(run.lines, PREFLIGHT);
		assert.deepEqual(run.stderr, [
			'✗ working tree has changes — commit or stash them first:\n M src/x.ts',
			'✗ refusing to release — nothing was changed, built or published'
		]);
		assert.strictEqual(await run.read('Cargo.toml'), CARGO_TOML);
		assert.isNull(await run.read('.publish-in-progress'));
	});

	test('with --no-git, publishes and prints the finalize instead of running it', async () => {
		const run = await run_release({ wetrun: true, bump: 'patch', git: false });
		assert.isFalse(run.failed);
		assert.deepEqual(run.lines, WETRUN_CALLS.slice(0, -FINALIZE.length));
		assert.isNull(await run.read('.publish-in-progress'));
		assert.include(run.stdout.join('\n'), `  git commit -m "chore: release v${NEXT_VERSION}"`);
		assert.strictEqual(
			run.stdout.at(-1),
			`\n✓ released ${PKG_NAME}@${NEXT_VERSION} (git finalize left to you)`
		);
	});

	// what a failure of each call leaves: refused before any write, failed with the bump in
	// place and nothing published, failed after the publish, or absorbed (a failed tag lookup
	// reads as no tag)
	const OUTCOMES: Record<string, 'refused' | 'bumped' | 'published' | 'absorbed'> = {};
	WETRUN_CALLS.forEach((line, index) => {
		const key = `${index} ${line}`;
		if (index < PREFLIGHT.length) {
			OUTCOMES[key] = line.startsWith('git_local rev-parse -q --verify refs/tags/')
				? 'absorbed'
				: 'refused';
		} else if (index < WETRUN_CALLS.length - FINALIZE.length) OUTCOMES[key] = 'bumped';
		else {
			OUTCOMES[key] = line.startsWith('git_local rev-parse -q --verify refs/tags/')
				? 'absorbed'
				: 'published';
		}
	});

	for (const [key, outcome] of Object.entries(OUTCOMES)) {
		const index = Number(key.split(' ')[0]);
		test(`a failure of call ${key} leaves the release ${outcome}`, async () => {
			const run = await run_release({ wetrun: true, bump: 'patch' }, { script: failing_at(index) });
			const bumped_cargo = CARGO_TOML.replace('version = "0.4.7"', `version = "${NEXT_VERSION}"`);
			const publish_index = WETRUN_CALLS.indexOf(PUBLISH_CALL);
			switch (outcome) {
				case 'absorbed':
					assert.isFalse(run.failed);
					assert.deepEqual(run.lines, WETRUN_CALLS);
					assert.isNull(await run.read('.publish-in-progress'));
					break;
				case 'refused':
					assert.isTrue(run.failed);
					// the preflight runs out, collecting every reason; a failed fetch has nothing to count
					assert.deepEqual(
						run.lines,
						WETRUN_CALLS[index] === 'git_remote fetch origin main'
							? PREFLIGHT.filter((line) => !line.includes(' rev-list '))
							: PREFLIGHT
					);
					assert.strictEqual(await run.read('Cargo.toml'), CARGO_TOML);
					assert.strictEqual(await run.read('CHANGELOG.md'), CHANGELOG);
					assert.isNull(await run.read('.publish-in-progress'));
					assert.strictEqual(
						run.stderr.at(-1),
						'✗ refusing to release — nothing was changed, built or published'
					);
					break;
				case 'bumped': {
					assert.isTrue(run.failed);
					assert.strictEqual(await run.read('Cargo.toml'), bumped_cargo);
					assert.strictEqual(await run.read('.publish-in-progress'), `${NEXT_VERSION}\n`);
					// a lockfile failure stops before the changelog's stamp; everything later has it
					const stamped = index >= PREFLIGHT.length + BUMP.length;
					assert.strictEqual(
						await run.read('CHANGELOG.md'),
						stamped ? STAMPED_CHANGELOG : CHANGELOG
					);
					// a fingerprint failure surfaces only once the gates have run
					const last = FINGERPRINT.includes(WETRUN_CALLS[index]!) ? publish_index - 1 : index;
					assert.deepEqual(run.lines, WETRUN_CALLS.slice(0, last + 1));
					assert.match(
						run.stderr.at(-1)!,
						/\n {2}the bump is in place — fix the cause and run `npm run publish:npm -- --wetrun` to resume$/
					);
					break;
				}
				case 'published':
					assert.isTrue(run.failed);
					assert.include(run.lines, PUBLISH_CALL);
					assert.deepEqual(run.lines, WETRUN_CALLS.slice(0, index + 1));
					assert.strictEqual(await run.read('Cargo.toml'), bumped_cargo);
					assert.strictEqual(await run.read('.publish-in-progress'), `${NEXT_VERSION}\n`);
					assert.include(run.stderr.at(-1)!, `${PKG_NAME}@${NEXT_VERSION} is PUBLISHED`);
					assert.include(run.stderr.at(-1)!, '`npm run publish:npm -- --wetrun` resumes from here');
					break;
			}
		});
	}

	test('a resume whose package is already out only finishes git', async () => {
		const run = await run_release(
			{ wetrun: true, bump: 'minor' },
			{
				files: {
					'Cargo.toml': CARGO_TOML.replace('version = "0.4.7"', `version = "${NEXT_VERSION}"`),
					'.publish-in-progress': `${NEXT_VERSION}\n`
				},
				script: (call, world) =>
					[call.program, ...call.args].join(' ') === `npm view ${PKG_NAME}@${NEXT_VERSION} version`
						? { stdout: `${NEXT_VERSION}\n` }
						: happy_release_script(call, world)
			}
		);
		assert.isFalse(run.failed, run.stderr.join('\n'));
		assert.deepEqual(run.lines, [...PREFLIGHT, ...BUMP, ...FINALIZE]);
		assert.deepEqual(run.stderr, [
			`⚠ --bump minor is ignored — resuming the interrupted v${NEXT_VERSION} release`
		]);
		assert.isNull(await run.read('.publish-in-progress'));
	});
});

describe('refusals', () => {
	test('a flag that never reached the args refuses before anything runs', async () => {
		const run = await run_release({
			wetrun: true,
			bump: 'patch',
			stray: ['npm_config_dry_run', '--', '--no-git']
		});
		assert.isTrue(run.failed);
		assert.deepEqual(run.calls, []);
		assert.strictEqual(await run.read('Cargo.toml'), CARGO_TOML);
		assert.include(run.stderr[0], 'npm_config_dry_run -- --no-git never reached the command');
	});

	test('a variable counts as a swallowed flag only with the values npm gives the flag', () => {
		const vars: Record<string, string> = {
			npm_config_wetrun: 'true',
			npm_config_dry_run: 'true',
			npm_config_git: '/usr/bin/git'
		};
		assert.deepEqual(
			to_publish_stray_vars((name) => vars[name]),
			['npm_config_wetrun', 'npm_config_dry_run']
		);
		for (const value of ['true', 'false', '']) {
			assert.deepEqual(
				to_publish_stray_vars((name) => (name === 'npm_config_git' ? value : undefined)),
				['npm_config_git']
			);
		}
		assert.deepEqual(
			to_publish_stray_vars(() => undefined),
			[]
		);
	});

	test('a --bump that is no level fails before anything runs', async () => {
		const run = await run_release({ bump: 'huge' });
		assert.isTrue(run.failed);
		assert.deepEqual(run.calls, []);
		assert.deepEqual(run.stderr, [
			`✗ --bump takes patch, minor or major (got "huge")\n${PUBLISH_USAGE}`
		]);
	});

	test('missing tools refuse in a dry run too, each named', async () => {
		const run = await run_release(
			{ bump: 'patch' },
			{ missing: ['node', 'wasm_pack', 'gzip', 'cargo'] }
		);
		assert.isTrue(run.failed);
		assert.deepEqual(run.stderr, [
			'✗ node not found — test:npm runs the package under it',
			'✗ wasm-pack not found — builds the package (`cargo install wasm-pack`)',
			'✗ gzip not found — the size gate measures the wire size with it',
			'✗ cargo not found — the bump moves the lockfiles with it',
			'✗ refusing to run — nothing was changed, built or published'
		]);
	});

	test('a node below the engines floor refuses', async () => {
		const run = await run_release(
			{ bump: 'patch' },
			{
				script: (call, world) =>
					call.program === 'node' ? { stdout: 'v20.1.0\n' } : happy_release_script(call, world)
			}
		);
		assert.isTrue(run.failed);
		assert.strictEqual(run.stderr[0], "✗ node v20.1.0 is below the package's engines floor (>=22)");
	});

	test('a missing git refuses on a wetrun, and a missing npm fails the lookup', async () => {
		const run = await run_release(
			{ wetrun: true, bump: 'patch' },
			{ missing: ['git_local', 'git_remote', 'npm'] }
		);
		assert.isTrue(run.failed);
		assert.deepEqual(run.stderr, [
			`✗ npm view ${PKG_NAME}@${NEXT_VERSION} failed:\nnpm: command not found`,
			'✗ git status failed: git: command not found',
			'✗ git branch failed',
			'✗ git rev-parse HEAD failed',
			'✗ git fetch origin main failed: git: command not found — cannot compare HEAD with origin',
			'✗ git ls-remote origin failed: git: command not found',
			'✗ npm whoami failed — not logged in to npm (run `npm login`): npm: command not found',
			'✗ refusing to release — nothing was changed, built or published'
		]);
		assert.strictEqual(await run.read('Cargo.toml'), CARGO_TOML);
	});

	test('a fresh release of a version already on npm refuses', async () => {
		const run = await run_release(
			{ bump: 'patch' },
			{
				script: (call, world) =>
					call.args[0] === 'view'
						? { stdout: `${NEXT_VERSION}\n` }
						: happy_release_script(call, world)
			}
		);
		assert.isTrue(run.failed);
		assert.strictEqual(
			run.stderr[0],
			`✗ ${PKG_NAME}@${NEXT_VERSION} is already on npm — release past it`
		);
	});
});
