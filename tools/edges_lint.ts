/**
 * Structural lint for `EDGES.md` — the divergence ledger's own invariants.
 *
 * The ledger's table is an index into its bullets, and these ways of breaking that index
 * are invisible to every other check in the repo, because nothing else reads prose:
 *
 * 1. a row that does not parse as seven cells — an unescaped `|` inside a code span
 *    splits the row and displaces every cell after it;
 * 2. a `find it` phrase that does not occur in the row's own bullet, which is the only
 *    navigation from a row to its record;
 * 3. a row with no bullet at all;
 * 4. a top-level bullet carrying no row number, which the table cannot reach;
 * 5. a cell longer than `MAX_CELL_CHARS` — the preamble's rule is that a cell aims at one
 *    screen-line, bounded here, and members, mechanism and the why-no-fixture argument go
 *    in the bullet.
 *
 * It also reports the row-number sequence, since the preamble's rule is that a closed
 * edge keeps its row and a gap is therefore a defect rather than a closure.
 *
 * Run: `deno task lint:edges`. Not wired into `check` or the gate set — it reads one
 * document, and a contributor runs it when editing that document.
 */

const path = Deno.args[0] ?? 'EDGES.md';
const text = await Deno.readTextFile(path);
const lines = text.split('\n');

/** A cell aims at one screen-line; past this bound its material belongs in the row's bullet. */
const MAX_CELL_CHARS = 300;

const row_re = /^\|\s*(~~)?(\d+)(~~)?\s*\|/;
// the number group only, stopping at the `·`: a greedy class reads "63 · 18" out of
// "- **63 · 18.6's …" and files that bullet under row 18 too
const bullet_re = /^- (?:~~)?\*\*(?:~~)?\s*(\d+(?:\s*\/\s*\d+)*)\s*(?:~~)?\s*·/;

const cells_of = (line: string): Array<string> => {
	let inner = line.trim();
	if (inner.startsWith('|')) inner = inner.slice(1);
	if (inner.endsWith('|') && !inner.endsWith('\\|')) inner = inner.slice(0, -1);
	return inner.split(/(?<!\\)\|/).map((c) => c.trim());
};

interface Row {
	n: number;
	line: number;
	cells: Array<string>;
}

const rows: Array<Row> = [];
const bullets = new Map<number, Array<string>>();
const unnumbered: Array<{line: number; head: string}> = [];

lines.forEach((line, i) => {
	const m = row_re.exec(line);
	if (m) rows.push({n: Number(m[2]), line: i + 1, cells: cells_of(line)});
});

const starts: Array<{i: number; nums: Array<number>}> = [];
lines.forEach((line, i) => {
	if (!line.startsWith('- **') && !line.startsWith('- ~~**')) return;
	const m = bullet_re.exec(line);
	const nums = m ? [...m[1].matchAll(/\d+/g)].map((x) => Number(x[0])) : [];
	starts.push({i, nums});
	if (nums.length === 0) unnumbered.push({line: i + 1, head: line.slice(0, 90)});
});
starts.forEach(({i, nums}, k) => {
	const end = k + 1 < starts.length ? starts[k + 1].i : lines.length;
	const body = lines.slice(i, end).join('\n');
	for (const n of nums) bullets.set(n, [...(bullets.get(n) ?? []), body]);
});

const strip = (s: string): string => s.replace(/[*_`]|~~/g, '').replace(/\s+/g, ' ').trim();
const problems: Array<string> = [];

for (const {n, line, cells} of rows) {
	if (cells.length !== 7) {
		problems.push(
			`row ${n} (line ${line}): ${cells.length} cells, not 7 — a literal | inside a code span must be written \\|`,
		);
		continue;
	}
	const names = ['#', 'shape', 'pglet', '18.6', 'silent?', 'guard', 'find it'];
	cells.forEach((cell, k) => {
		if (cell.trim().length > MAX_CELL_CHARS) {
			problems.push(
				`row ${n} (line ${line}): the \`${names[k]}\` cell is ${cell.trim().length} characters, over ${MAX_CELL_CHARS} — move members and mechanism into the bullet`,
			);
		}
	});
	const body = bullets.get(n);
	if (!body) {
		problems.push(`row ${n} (line ${line}): no bullet — every row is reachable from its find-it phrase`);
		continue;
	}
	const phrase = strip(cells[6]);
	const hay = strip(body.join('\n'));
	if (!phrase) problems.push(`row ${n} (line ${line}): empty find-it cell`);
	else if (!hay.includes(phrase)) {
		problems.push(`row ${n} (line ${line}): find-it phrase does not occur in its bullet — ${phrase.slice(0, 70)}`);
	}
}
for (const {line, head} of unnumbered) {
	problems.push(`line ${line}: top-level bullet with no row number, unreachable from the table — ${head}`);
}

const numbers = rows.map((r) => r.n);
const gaps = numbers.filter((n, i) => i > 0 && n !== numbers[i - 1] + 1 && !numbers.includes(n - 1));
const dupes = numbers.filter((n, i) => numbers.indexOf(n) !== i);
if (dupes.length) problems.push(`duplicate row numbers: ${[...new Set(dupes)].join(', ')}`);
if (gaps.length) problems.push(`row-number gaps (a closed edge keeps its row): before ${gaps.join(', ')}`);

console.log(
	`${path}: ${rows.length} rows, ${bullets.size} numbered bullets, ` +
		`${rows.filter((r) => r.cells[0].includes('~~')).length} struck`,
);
if (problems.length === 0) {
	console.log('ok — every row parses, has a bullet, and its find-it phrase resolves');
	Deno.exit(0);
}
for (const p of problems) console.error(`  ${p}`);
console.error(`\n${problems.length} problem(s)`);
Deno.exit(1);
