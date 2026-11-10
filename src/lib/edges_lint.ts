/**
 * Structural lint for `EDGES.md`, the divergence ledger's own invariants.
 *
 * The ledger's table is an index into its bullets, and these ways of breaking
 * that index are invisible to every other check in the repo, because nothing
 * else reads prose:
 *
 * 1. a row that doesn't parse as seven cells: an unescaped `|` inside a code
 *    span splits the row and displaces every cell after it
 * 2. a `find it` phrase that doesn't occur in the row's own bullet, which is the
 *    only navigation from a row to its record
 * 3. a row with no bullet at all
 * 4. a top-level bullet carrying no row number, which the table can't reach
 * 5. a cell longer than `EDGES_MAX_CELL_CHARS`: the preamble's rule is that a
 *    cell aims at one screen-line, bounded here, and members, mechanism and the
 *    why-no-fixture argument go in the bullet
 *
 * It also reports the row-number sequence, since the preamble's rule is that a
 * closed edge keeps its row and a gap is therefore a defect rather than a
 * closure.
 *
 * Run by `npm run lint:edges`. Not wired into `check` or the gate set: it reads
 * one document, and a contributor runs it when editing that document.
 *
 * @module
 */

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import { is_fs_error } from '@fuzdev/gro/fs_error.ts';
import { SilentError } from '@fuzdev/gro/task.ts';

/** A cell aims at one screen-line; past this bound its material belongs in the row's bullet. */
export const EDGES_MAX_CELL_CHARS = 300;

/** The ledger's path in the checkout, linted when no other is named. */
export const EDGES_PATH = 'EDGES.md';

const ROW = /^\|\s*(~~)?(\d+)(~~)?\s*\|/;
// the number group only, stopping at the `·`: a greedy class reads "63 · 18" out of
// "- **63 · 18.6's …" and files that bullet under row 18 too
const BULLET = /^- (?:~~)?\*\*(?:~~)?\s*(\d+(?:\s*\/\s*\d+)*)\s*(?:~~)?\s*·/;

const CELL_NAMES = ['#', 'shape', 'pglet', '18.6', 'silent?', 'guard', 'find it'];

/** What the lint found in one ledger. */
export interface EdgesLintResult {
	rows: number;
	/** How many row numbers have a bullet. */
	numbered_bullets: number;
	struck: number;
	/** Each problem, as a line naming the row or line it's on. */
	problems: Array<string>;
}

interface EdgesRow {
	n: number;
	line: number;
	cells: Array<string>;
}

const to_cells = (line: string): Array<string> => {
	let inner = line.trim();
	if (inner.startsWith('|')) inner = inner.slice(1);
	if (inner.endsWith('|') && !inner.endsWith('\\|')) inner = inner.slice(0, -1);
	return inner.split(/(?<!\\)\|/).map((c) => c.trim());
};

const strip = (s: string): string =>
	s
		.replace(/[*_`]|~~/g, '')
		.replace(/\s+/g, ' ')
		.trim();

/** Lints a ledger's text. */
export const lint_edges = (text: string): EdgesLintResult => {
	const lines = text.split('\n');

	const rows: Array<EdgesRow> = [];
	const bullets: Map<number, Array<string>> = new Map();
	const unnumbered: Array<{ line: number; head: string }> = [];

	lines.forEach((line, i) => {
		const m = ROW.exec(line);
		if (m) rows.push({ n: Number(m[2]), line: i + 1, cells: to_cells(line) });
	});

	const starts: Array<{ i: number; nums: Array<number> }> = [];
	lines.forEach((line, i) => {
		if (!line.startsWith('- **') && !line.startsWith('- ~~**')) return;
		const m = BULLET.exec(line);
		const nums = m ? [...m[1]!.matchAll(/\d+/g)].map((x) => Number(x[0])) : [];
		starts.push({ i, nums });
		if (nums.length === 0) unnumbered.push({ line: i + 1, head: line.slice(0, 90) });
	});
	starts.forEach(({ i, nums }, k) => {
		const end = k + 1 < starts.length ? starts[k + 1]!.i : lines.length;
		const body = lines.slice(i, end).join('\n');
		for (const n of nums) bullets.set(n, [...(bullets.get(n) ?? []), body]);
	});

	const problems: Array<string> = [];

	for (const { n, line, cells } of rows) {
		if (cells.length !== 7) {
			problems.push(
				`row ${n} (line ${line}): ${cells.length} cells, not 7 — a literal | inside a code span must be written \\|`
			);
			continue;
		}
		cells.forEach((cell, k) => {
			if (cell.trim().length > EDGES_MAX_CELL_CHARS) {
				problems.push(
					`row ${n} (line ${line}): the \`${CELL_NAMES[k]}\` cell is ${cell.trim().length} characters, over ${EDGES_MAX_CELL_CHARS} — move members and mechanism into the bullet`
				);
			}
		});
		const body = bullets.get(n);
		if (!body) {
			problems.push(
				`row ${n} (line ${line}): no bullet — every row is reachable from its find-it phrase`
			);
			continue;
		}
		const phrase = strip(cells[6]!);
		const hay = strip(body.join('\n'));
		if (!phrase) problems.push(`row ${n} (line ${line}): empty find-it cell`);
		else if (!hay.includes(phrase)) {
			problems.push(
				`row ${n} (line ${line}): find-it phrase does not occur in its bullet — ${phrase.slice(0, 70)}`
			);
		}
	}
	for (const { line, head } of unnumbered) {
		problems.push(
			`line ${line}: top-level bullet with no row number, unreachable from the table — ${head}`
		);
	}

	const numbers = rows.map((r) => r.n);
	const gaps = numbers.filter(
		(n, i) => i > 0 && n !== numbers[i - 1]! + 1 && !numbers.includes(n - 1)
	);
	const dupes = numbers.filter((n, i) => numbers.indexOf(n) !== i);
	if (dupes.length) problems.push(`duplicate row numbers: ${[...new Set(dupes)].join(', ')}`);
	if (gaps.length) {
		problems.push(`row-number gaps (a closed edge keeps its row): before ${gaps.join(', ')}`);
	}

	return {
		rows: rows.length,
		numbered_bullets: bullets.size,
		struck: rows.filter((r) => r.cells[0]!.includes('~~')).length,
		problems
	};
};

/** The command's args, the task's `Args` after validation. */
export interface EdgesLintCommandArgs {
	/** The ledger to lint, beneath the checkout, `EDGES.md` when empty; at most one. */
	_: Array<string>;
}

/** What the command reaches, gathered by the task root. */
export interface EdgesLintCommandDeps {
	/** This checkout, read-only, where the ledger is. */
	checkout: FsReadDir;
	/** Prints a block to stdout, as `console.log`. */
	print: (text: string) => void;
	/** Prints a block to stderr, as `console.error`. */
	print_error: (text: string) => void;
}

/**
 * Lints the ledger and prints its counts, then an ok line or each problem and
 * their count.
 *
 * @throws SilentError - exit 1 after printing the problems, or that the ledger is missing
 */
export const run_edges_lint_command = async (
	deps: EdgesLintCommandDeps,
	args: EdgesLintCommandArgs
): Promise<void> => {
	const { checkout, print, print_error } = deps;
	if (args._.length > 1) {
		print_error(`lint:edges lints one ledger, got ${args._.length}: ${args._.join(' ')}`);
		throw new SilentError();
	}
	const path = args._[0] ?? EDGES_PATH;
	let text: string;
	try {
		text = await checkout.read_text(path);
	} catch (error) {
		if (!is_fs_error(error, 'not_found')) throw error;
		print_error(`${path}: no such file in ${checkout.path}`);
		throw new SilentError();
	}
	const result = lint_edges(text);
	print(
		`${path}: ${result.rows} rows, ${result.numbered_bullets} numbered bullets, ${result.struck} struck`
	);
	if (result.problems.length === 0) {
		print('ok — every row parses, has a bullet, and its find-it phrase resolves');
		return;
	}
	for (const p of result.problems) print_error(`  ${p}`);
	print_error(`\n${result.problems.length} problem(s)`);
	throw new SilentError();
};
