import { fs } from '@fuzdev/gro/fs.ts';
import { is_silent_error } from '@fuzdev/gro/task.ts';
import { assert, describe, test } from 'vitest';

import { EDGES_MAX_CELL_CHARS, lint_edges, run_edges_lint_command } from '#lib/edges_lint.ts';

/** A table row with seven cells, `find` its find-it phrase. */
const row = (n: number | string, find: string, shape = 'a shape'): string =>
	`| ${n} | ${shape} | x | y | no | fixture | ${find} |`;

const LEDGER = [
	'# EDGES',
	'',
	'| # | shape | pglet | 18.6 | silent? | guard | find it |',
	'|---|---|---|---|---|---|---|',
	row(1, 'the first edge'),
	row('~~2~~', '`the second` edge'),
	row(3, 'shared edge'),
	row(4, 'shared edge'),
	'',
	'- **1 · Title.** About the first edge.',
	'- ~~**2 · Closed.**~~ About **the second** edge, now closed.',
	'- **3 / 4 · Two rows.** One shared edge record.',
	''
].join('\n');

describe('lint_edges', () => {
	test('a sound ledger has no problems, and counts its rows, numbered bullets and struck rows', () => {
		assert.deepEqual(lint_edges(LEDGER), {
			rows: 4,
			numbered_bullets: 4,
			struck: 1,
			problems: []
		});
	});

	test('a row split by an unescaped pipe is not seven cells, and an escaped one is fine', () => {
		const ledger = LEDGER.replace(row(1, 'the first edge'), row(1, 'the first edge', '`a | b`'));
		assert.deepEqual(lint_edges(ledger).problems, [
			'row 1 (line 5): 8 cells, not 7 — a literal | inside a code span must be written \\|'
		]);
		const escaped = LEDGER.replace(row(1, 'the first edge'), row(1, 'the first edge', '`a \\| b`'));
		assert.deepEqual(lint_edges(escaped).problems, []);
	});

	test('a cell over the bound is named, and one at it is not', () => {
		const at = LEDGER.replace(
			row(3, 'shared edge'),
			row(3, 'shared edge', 'x'.repeat(EDGES_MAX_CELL_CHARS))
		);
		assert.deepEqual(lint_edges(at).problems, []);
		const over = LEDGER.replace(
			row(3, 'shared edge'),
			row(3, 'shared edge', 'x'.repeat(EDGES_MAX_CELL_CHARS + 1))
		);
		assert.deepEqual(lint_edges(over).problems, [
			`row 3 (line 7): the \`shape\` cell is ${EDGES_MAX_CELL_CHARS + 1} characters, over ${EDGES_MAX_CELL_CHARS} — move members and mechanism into the bullet`
		]);
	});

	test('a row with no bullet, and a find-it phrase missing from or empty in its bullet', () => {
		const ledger = LEDGER.replace('- **1 · Title.** About the first edge.\n', '')
			.replace(row(3, 'shared edge'), row(3, 'nowhere at all'))
			.replace(row(4, 'shared edge'), row(4, ' '));
		assert.deepEqual(lint_edges(ledger).problems, [
			'row 1 (line 5): no bullet — every row is reachable from its find-it phrase',
			'row 3 (line 7): find-it phrase does not occur in its bullet — nowhere at all',
			'row 4 (line 8): empty find-it cell'
		]);
	});

	test('a top-level bullet with no row number is unreachable', () => {
		const ledger = LEDGER + '- **Unnumbered · title** body\n';
		assert.deepEqual(lint_edges(ledger).problems, [
			'line 13: top-level bullet with no row number, unreachable from the table — - **Unnumbered · title** body'
		]);
	});

	test("a bullet's number stops at its `·`, so `63 · 18.6's` files under 63 alone", () => {
		const ledger = [
			row(18, 'eighteen'),
			row(63, 'sixty three'),
			"- **63 · 18.6's sixty three**",
			''
		].join('\n');
		assert.deepEqual(lint_edges(ledger).problems, [
			'row 18 (line 1): no bullet — every row is reachable from its find-it phrase',
			'row-number gaps (a closed edge keeps its row): before 63'
		]);
	});

	test('duplicate row numbers and gaps in the sequence', () => {
		const ledger = LEDGER.replace(
			row(4, 'shared edge'),
			`${row(4, 'shared edge')}\n${row(4, 'shared edge')}\n${row(9, 'shared edge')}`
		).replace('- **3 / 4 ·', '- **3 / 4 / 9 ·');
		assert.deepEqual(lint_edges(ledger).problems, [
			'duplicate row numbers: 4',
			'row-number gaps (a closed edge keeps its row): before 9'
		]);
	});
});

describe('run_edges_lint_command', () => {
	const run = async (files: Record<string, string>, args: Array<string>) => {
		await using dir = await fs.temp_dir('pglet_edges_');
		for (const [path, text] of Object.entries(files)) await dir.write(path, text);
		const stdout: Array<string> = [];
		const stderr: Array<string> = [];
		let failed = false;
		try {
			await run_edges_lint_command(
				{
					checkout: dir.readonly(),
					print: (text) => stdout.push(text),
					print_error: (text) => stderr.push(text)
				},
				{ _: args }
			);
		} catch (error) {
			if (!is_silent_error(error)) throw error;
			failed = true;
		}
		return { stdout, stderr, failed, path: dir.path };
	};

	test('lints EDGES.md when no path is named', async () => {
		const result = await run({ 'EDGES.md': LEDGER }, []);
		assert.deepEqual(result, {
			stdout: [
				'EDGES.md: 4 rows, 4 numbered bullets, 1 struck',
				'ok — every row parses, has a bullet, and its find-it phrase resolves'
			],
			stderr: [],
			failed: false,
			path: result.path
		});
	});

	test('prints each problem and their count, and fails', async () => {
		const result = await run({ 'other.md': LEDGER + '- **x · y**\n' }, ['other.md']);
		assert.isTrue(result.failed);
		assert.deepEqual(result.stdout, ['other.md: 4 rows, 4 numbered bullets, 1 struck']);
		assert.deepEqual(result.stderr, [
			'  line 13: top-level bullet with no row number, unreachable from the table — - **x · y**',
			'\n1 problem(s)'
		]);
	});

	test('refuses a missing ledger and a second path', async () => {
		const missing = await run({}, []);
		assert.isTrue(missing.failed);
		assert.deepEqual(missing.stderr, [`EDGES.md: no such file in ${missing.path}`]);
		const two = await run({ 'EDGES.md': LEDGER }, ['EDGES.md', 'b.md']);
		assert.isTrue(two.failed);
		assert.deepEqual(two.stdout, []);
		assert.deepEqual(two.stderr, ['lint:edges lints one ledger, got 2: EDGES.md b.md']);
	});
});
