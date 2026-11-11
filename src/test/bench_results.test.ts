import { fs } from '@fuzdev/gro/fs.ts';
import type { SpawnOutput, SpawnProgram } from '@fuzdev/gro/spawn.ts';
import { assert, describe, test } from 'vitest';

import {
	bench_status_is_dirty,
	format_bench_json,
	git_bench_stamp,
	to_bench_status_args
} from '#lib/bench_results.ts';

describe('bench_status_is_dirty', () => {
	test('this repo: a tracked change is dirty, staged or not', () => {
		assert.isTrue(bench_status_is_dirty(' M README.md\0', false));
		assert.isTrue(bench_status_is_dirty('M  crates/x.rs\0', false));
	});

	test('this repo: an untracked file counts only under the measured dirs', () => {
		assert.isFalse(bench_status_is_dirty('?? notes.txt\0?? docs/new.md\0', false));
		for (const path of ['benches/x.ts', 'crates/a/b.rs', 'scripts/s.mjs', 'src/lib/y.ts']) {
			assert.isTrue(bench_status_is_dirty(`?? ${path}\0`, false), path);
		}
	});

	test('this repo: a path is read verbatim, a space or a rename source included', () => {
		assert.isTrue(bench_status_is_dirty('?? src/a b.ts\0', false));
		assert.isTrue(bench_status_is_dirty('R  src/new.ts\0src/old.ts\0', false));
		assert.isFalse(bench_status_is_dirty('', false));
	});

	test('another checkout: every entry counts', () => {
		assert.isTrue(bench_status_is_dirty('?? notes.txt\0', true));
		assert.isFalse(bench_status_is_dirty('', true));
	});
});

describe('to_bench_status_args', () => {
	test('this repo leaves the recorded results documents out, from the top', () => {
		assert.deepEqual(to_bench_status_args(false), [
			'status',
			'--porcelain',
			'-z',
			'--untracked-files=all',
			'--',
			':(top)',
			':(top,glob,exclude)benches/*_results.json',
			':(top,glob,exclude)benches/size_baseline.json',
			':(top,glob,exclude)benches/pglite_size_comparison.json'
		]);
	});

	test('another checkout takes no pathspec', () => {
		assert.deepEqual(to_bench_status_args(true), [
			'status',
			'--porcelain',
			'-z',
			'--untracked-files=all'
		]);
	});
});

/** A `git` answering each subcommand by its first argument; `null` fails it. */
const to_fake_git = (
	answers: Record<string, string | null>,
	calls: Array<string> = []
): SpawnProgram => {
	const unreached = () => {
		throw new Error('only out is called');
	};
	return {
		name: 'git',
		path: '/fake/git',
		run: unreached,
		out_bytes: unreached,
		start: unreached,
		out: async (args: ReadonlyArray<string>, options): Promise<SpawnOutput> => {
			calls.push(`${options?.cwd?.path} ${args.join(' ')}`);
			const answer = answers[args[0]!];
			return answer === null || answer === undefined
				? { kind: 'exited', code: 128, stdout: '', stderr: 'fatal', truncated: false }
				: { kind: 'exited', code: 0, stdout: answer, stderr: '', truncated: false };
		}
	};
};

describe('git_bench_stamp', () => {
	test('a clean tree is its commit, read in the dir it is given', async () => {
		await using dir = await fs.temp_dir('pglet_bench_stamp_');
		const calls: Array<string> = [];
		const git = to_fake_git({ 'rev-parse': 'abc1234\n', status: '?? notes.txt\0' }, calls);
		assert.strictEqual(await git_bench_stamp(git, dir), 'abc1234');
		assert.deepEqual(calls, [
			`${dir.path} rev-parse --short HEAD`,
			`${dir.path} ${to_bench_status_args(false).join(' ')}`
		]);
	});

	test('a dirty tree is -dirty, another checkout by its own rule', async () => {
		await using dir = await fs.temp_dir('pglet_bench_stamp_');
		const git = to_fake_git({ 'rev-parse': 'abc1234\n', status: '?? notes.txt\0' });
		assert.strictEqual(await git_bench_stamp(git, dir, true), 'abc1234-dirty');
	});

	test('an unreadable status is -dirty?, an unreadable commit unknown', async () => {
		await using dir = await fs.temp_dir('pglet_bench_stamp_');
		assert.strictEqual(
			await git_bench_stamp(to_fake_git({ 'rev-parse': 'abc1234\n', status: null }), dir),
			'abc1234-dirty?'
		);
		assert.strictEqual(await git_bench_stamp(to_fake_git({ 'rev-parse': null }), dir), 'unknown');
	});

	test('a git that throws is unknown, never a thrown stamp', async () => {
		await using dir = await fs.temp_dir('pglet_bench_stamp_');
		const git: SpawnProgram = {
			...to_fake_git({}),
			out: async () => {
				throw new Error('launch failed');
			}
		};
		assert.strictEqual(await git_bench_stamp(git, dir), 'unknown');
	});
});

test('format_bench_json is tab-indented with a trailing newline', () => {
	assert.strictEqual(format_bench_json({ a: [1] }), '{\n\t"a": [\n\t\t1\n\t]\n}\n');
});
