import { fs, type FsDir, type FsReadDir } from '@fuzdev/gro/fs.ts';
import type { SpawnOutput, SpawnProgram, SpawnResult } from '@fuzdev/gro/spawn.ts';
import { is_silent_error } from '@fuzdev/gro/task.ts';

import type { ReleaseProgram } from '#lib/release/programs.ts';
import {
	run_publish_command,
	type PublishCommandArgs,
	type PublishCommandDeps
} from '#lib/release/publish.ts';

/** The programs a release runs, by the deps key the command takes them under. */
export const RELEASE_PROGRAM_KEYS = [
	'git_local',
	'git_remote',
	'npm',
	'deno',
	'gro',
	'cargo',
	'node',
	'wasm_pack',
	'gzip'
] as const;
export type ReleaseProgramKey = (typeof RELEASE_PROGRAM_KEYS)[number];

/** A call a fake program saw, in the order calls were made across programs. */
export interface ReleaseCall {
	program: ReleaseProgramKey;
	method: 'out' | 'run';
	args: Array<string>;
	/** The call's cwd, relative to the checkout (`.` for the checkout itself). */
	cwd: string;
	/** Whether the call's stdin was the terminal's. */
	stdin_inherited: boolean;
}

/** How a fake program answers a call; undefined exits 0 with no output. */
export interface ReleaseAnswer {
	code?: number;
	stdout?: string;
	stderr?: string;
}

/** The fake world's checkout and what the fakes do to it. */
export interface ReleaseWorld {
	checkout: FsDir;
	calls: Array<ReleaseCall>;
	stdout: Array<string>;
	stderr: Array<string>;
}

/** Answers each call, given the world so a fake can act on the checkout, as `npm run test:npm` builds `pkg-npm`. */
export type ReleaseScript = (
	call: ReleaseCall,
	world: ReleaseWorld
) => Promise<ReleaseAnswer | undefined> | ReleaseAnswer | undefined;

/** A call as one line, `program arg…`, with its cwd when not the checkout. */
export const to_call_line = (call: ReleaseCall): string =>
	[call.program, ...call.args].join(' ') + (call.cwd === '.' ? '' : ` (in ${call.cwd})`);

export const CARGO_TOML = `[workspace]
resolver = "2"

[workspace.package]
version = "0.4.7"
edition = "2024"
rust-version = "1.89"
`;

export const CHANGELOG = `# pglet changelog

## Unreleased
<!-- bump: patch -->

- fix: pending

## 0.4.7

- before
`;

/** The version the default world's fresh release bumps to. */
export const NEXT_VERSION = '0.4.8';

export const PKG_NAME = '@fuzdev/pglet-wasm';

const to_relative_cwd = (checkout: FsReadDir, cwd: FsReadDir | undefined): string => {
	if (cwd === undefined) return '<project>';
	if (cwd.path === checkout.path) return '.';
	return cwd.path.startsWith(checkout.path + '/')
		? cwd.path.slice(checkout.path.length + 1)
		: cwd.path;
};

/** The happy world: a clean `main` at `origin/main`, nothing on npm, every gate green. */
export const happy_release_script: ReleaseScript = async (call, world) => {
	const line = [call.program, ...call.args].join(' ');
	switch (call.program) {
		case 'node':
			return { stdout: 'v24.14.1\n' };
		case 'git_local':
		case 'git_remote':
			if (line.endsWith('branch --show-current')) return { stdout: 'main\n' };
			if (line.endsWith('rev-parse -q --verify HEAD^{commit}')) return { stdout: 'c0ffee\n' };
			if (line.includes('rev-parse -q --verify refs/tags/')) return { code: 1 };
			if (line.endsWith('rev-list --left-right --count origin/main...HEAD')) {
				return { stdout: '0\t0\n' };
			}
			if (line.endsWith('rev-parse HEAD')) return { stdout: 'c0ffee\n' };
			if (
				call.args.join(' ') ===
				'status --porcelain -- Cargo.toml Cargo.lock fuzz/Cargo.lock CHANGELOG.md'
			) {
				return { stdout: ' M Cargo.toml\n' };
			}
			return undefined;
		case 'npm':
			if (line === 'npm whoami') return { stdout: 'maintainer\n' };
			if (line.startsWith('npm view ')) return { code: 1, stderr: 'npm error code E404\n' };
			if (line === 'npm publish --dry-run') {
				const version = await read_version(world.checkout);
				return {
					stdout: `+ ${PKG_NAME}@${version}\n`,
					stderr: 'npm notice Tarball Contents\n'
				};
			}
			if (line === 'npm run test:npm') {
				// the gate builds pkg-npm, stamped with the checkout's version
				await world.checkout.mkdir('crates/pglet_wasm/pkg-npm', { recursive: true });
				await world.checkout.write(
					'crates/pglet_wasm/pkg-npm/package.json',
					JSON.stringify({ name: PKG_NAME, version: await read_version(world.checkout) })
				);
			}
			return undefined;
		default:
			return undefined;
	}
};

const read_version = async (checkout: FsReadDir): Promise<string> =>
	/^version = "([^"]*)"/m.exec(await checkout.read_text('Cargo.toml'))![1]!;

const create_fake_program = (
	key: ReleaseProgramKey,
	world: ReleaseWorld,
	script: ReleaseScript
): SpawnProgram => {
	const answer = async (
		method: ReleaseCall['method'],
		args: ReadonlyArray<string>,
		cwd: FsReadDir | undefined,
		stdin_inherited: boolean
	): Promise<{ result: SpawnResult; stdout: string; stderr: string }> => {
		const call: ReleaseCall = {
			program: key,
			method,
			args: [...args],
			cwd: to_relative_cwd(world.checkout, cwd),
			stdin_inherited
		};
		world.calls.push(call);
		const a = (await script(call, world)) ?? {};
		return {
			result: { kind: 'exited', code: a.code ?? 0 },
			stdout: a.stdout ?? '',
			stderr: a.stderr ?? ''
		};
	};
	return {
		name: key,
		path: `/fake/${key}`,
		run: async (args, options) => (await answer('run', args, options?.cwd, true)).result,
		out: async (args, options): Promise<SpawnOutput> => {
			const { result, stdout, stderr } = await answer(
				'out',
				args,
				options?.cwd,
				options?.stdin === 'inherit'
			);
			return { ...result, stdout, stderr, truncated: false };
		},
		out_bytes: () => {
			throw new Error('the release never captures bytes');
		},
		start: () => {
			throw new Error('the release never starts a live child');
		}
	};
};

/** Options for one run in a fake world. */
export interface ReleaseRunOptions {
	script?: ReleaseScript;
	/** Programs that aren't installed. */
	missing?: Array<ReleaseProgramKey>;
	/** Files written into the checkout before the run, over the defaults. */
	files?: Record<string, string>;
}

/** The outcome of one run in a fake world. */
export interface ReleaseRun extends ReleaseWorld {
	/** Whether the command failed with a `SilentError`, its one way to fail. */
	failed: boolean;
	/** Reads a checkout file as the run left it, null when missing. */
	read: (path: string) => Promise<string | null>;
	lines: Array<string>;
}

/**
 * Runs the publish command once over a fresh temp checkout holding
 * `Cargo.toml`, `CHANGELOG.md`, both lockfiles and `fuzz/`, with every program
 * a fake answering from `script`.
 */
export const run_release = async (
	args: Partial<PublishCommandArgs>,
	options: ReleaseRunOptions = {}
): Promise<ReleaseRun> => {
	const { script = happy_release_script, missing = [], files = {} } = options;
	await using checkout = await fs.temp_dir('pglet_release_');
	await checkout.mkdir('fuzz');
	const initial: Record<string, string> = {
		'Cargo.toml': CARGO_TOML,
		'Cargo.lock': 'lock\n',
		'fuzz/Cargo.lock': 'fuzz lock\n',
		'CHANGELOG.md': CHANGELOG,
		...files
	};
	for (const [path, text] of Object.entries(initial)) await checkout.write(path, text);
	const world: ReleaseWorld = { checkout, calls: [], stdout: [], stderr: [] };
	const to_program = (key: ReleaseProgramKey): ReleaseProgram => ({
		name: key === 'wasm_pack' ? 'wasm-pack' : key.startsWith('git') ? 'git' : key,
		program: missing.includes(key) ? null : create_fake_program(key, world, script),
		cwd: checkout
	});
	const deps: PublishCommandDeps = {
		checkout,
		git_local: to_program('git_local'),
		git_remote: to_program('git_remote'),
		npm: to_program('npm'),
		deno: to_program('deno'),
		gro: to_program('gro'),
		cargo: to_program('cargo'),
		node: to_program('node'),
		wasm_pack: to_program('wasm_pack'),
		gzip: to_program('gzip'),
		print: (text) => world.stdout.push(text),
		print_error: (text) => world.stderr.push(text)
	};
	let failed = false;
	try {
		await run_publish_command(deps, { wetrun: false, git: true, stray: [], ...args });
	} catch (error) {
		if (!is_silent_error(error)) throw error;
		failed = true;
	}
	// read everything the assertions may ask for before the temp dir is removed
	const snapshot = new Map<string, string | null>();
	for (const path of [
		'Cargo.toml',
		'Cargo.lock',
		'fuzz/Cargo.lock',
		'CHANGELOG.md',
		'.publish-in-progress',
		'crates/pglet_wasm/pkg-npm/package.json'
	]) {
		snapshot.set(path, await checkout.read_text_if_exists(path));
	}
	return {
		...world,
		failed,
		read: async (path) => snapshot.get(path) ?? null,
		lines: world.calls.map(to_call_line)
	};
};
