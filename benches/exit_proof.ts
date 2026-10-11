/**
 * The "exit proof" — the headline real-consumer comparison.
 *
 * Drives fuz_app's `db` test matrix once per engine leg and reports how long the
 * *same* suite of database tests takes when the DB is backed by pglet (both
 * legs) versus pglite. This is the payoff measurement for the "fast test-fixture
 * engine" thesis: the micro-benches already prove pglet wins cold-start
 * decisively (`wasm_compare.ts`, ~11–14 ms vs pglite's ~1.1–1.2 s); this proves
 * it — or doesn't — on a real consumer's suite.
 *
 * ## Method (no fuz_app edit for the bench's sake)
 *
 * fuz_app's `--project db` runs every `*.db.test.ts` file; each DB backend is an
 * *additive* factory whose describe block is named `… (<driver>)` (pglite is
 * always on, the pglet/pg legs opt in via env). So a leg is isolated **by name**:
 * a separate `vitest` process per leg with `--testNamePattern '\(<driver>\)'`
 * runs only that driver's suites (identical bodies, full beforeAll/beforeEach/
 * afterAll lifecycle), on the **shared subset of files that bind all four legs**
 * (discovered below — the single-driver files are excluded so every leg does
 * identical work; that cuts *both* ways, since a pg-only file inflates the pg
 * leg alone — and a file mixing a single-driver suite into its four-leg ones is
 * refused before any leg runs). Wall-time is read from vitest's own Duration summary;
 * the `tests` bucket (engine work, excluding the fixed transform/import overhead) is the sum of
 * the test files' run times, each file's `beforeAll`/`afterAll` hooks included — fuz_app's `db`
 * project runs its files one at a time in one worker (`fileParallelism: false`), so the sum is
 * serial time, the run's wall less its transform/import/setup — written by a reporter of this
 * bench's (`lib/vitest_file_times.mjs`) — the cleaner engine number; total
 * wall is what a developer feels. A run that writes no such report fails its leg: no other
 * number is read in its place.
 *
 * ## Tiers — L1 and L2 never mixed
 *
 *   - **L1 (in-process):** pglet-wasm vs pglite — the apples-to-apples pair.
 *   - **L2 (out-of-process):** pglet native server (over TCP/IPC) and, if
 *     `PGLET_EXITPROOF_PG_URL` is set, real Postgres — production ceiling
 *     references, reported separately, never as an L1 peer to pglite.
 *
 * The run ends by computing the ratios the README quotes (`lib/exit_proof_ratios.ts`),
 * printed and recorded under `ratios`. One of them, pg over pglet-wasm, crosses the line on
 * purpose — the README's "faster than a real PostgreSQL server" is in-process pglet against
 * Postgres over TCP — and is marked `crosses_tiers` so it is never read as a same-tier pair.
 *
 * Watch-tier: prints by default; `--update` refreshes `exit_proof_results.json`.
 * Ratios (same run, same box) are the durable signal; absolute ms flap with the
 * box. Each leg runs `REPS` timed passes (+`WARMUP` discarded) interleaved so
 * thermal drift hits every leg evenly.
 *
 *   deno task bench:exit-proof            # print
 *   deno task bench:exit-proof:update     # ...and refresh the results JSON
 *
 * Knobs (env): PGLET_EXITPROOF_REPS (default 3), PGLET_EXITPROOF_WARMUP (1),
 * PGLET_EXITPROOF_LEGS (comma-separated subset; default all four),
 * PGLET_FUZ_APP_DIR (default ../fuz_app), PGLET_SERVER_BIN / PGLET_WASM_PKG
 * (default the built artifacts), PGLET_EXITPROOF_PG_URL (opt-in pg leg — the
 * named database is DROPPED and recreated before every pass; point it at a
 * dedicated scratch database, never at one you keep).
 *
 * That per-pass reset is defense in depth, not the load-bearing part, and the
 * distinction cost this leg several runs. The contamination it was written for
 * is *within* a pass: `create_pg_factory` hands every file one persistent
 * database and its `create()` drops only `schema_version`, so under
 * `isolate: false` one file's DDL is still standing when the next file's suite
 * runs — and `auth/account_schema.db.test.ts` asserts over the whole `public`
 * schema, so it reads the cell/fact tables and fails on the *first* pass. Only
 * a per-`create()` schema reset fixes that, and it lives in fuz_app's fixtures:
 * `src/test/create_db_fixture.ts`, through which `db_fixture.ts` and its
 * sibling four-leg fixtures are built, wraps the pg leg's `init_schema` in
 * `drop_auth_schema` as that helper's own guidance prescribes. Without that fix
 * this leg cannot go green no matter what this harness does between passes.
 */
import {resolve} from 'node:path';
import {
	stats_cv,
	stats_mean,
	stats_median,
	stats_min_max,
	stats_percentile,
	stats_std_dev,
} from '@fuzdev/fuz_util/stats.ts';
import {exit_proof_ratios} from './lib/exit_proof_ratios.ts';
import {PGLITE, resolve_package_dir} from './lib/pglite_install.ts';
import {git_commit, package_version, write_json} from './lib/results.ts';

// --- knobs ----------------------------------------------------------------
const num_env = (name: string, fallback: number): number => {
	const v = Deno.env.get(name);
	const n = v == null ? NaN : Number(v);
	return Number.isFinite(n) ? n : fallback;
};

const REPS = num_env('PGLET_EXITPROOF_REPS', 3);
const WARMUP = num_env('PGLET_EXITPROOF_WARMUP', 1);
const FUZ_APP_DIR = resolve(Deno.env.get('PGLET_FUZ_APP_DIR') ?? resolve(Deno.cwd(), '../fuz_app'));
const SERVER_BIN =
	Deno.env.get('PGLET_SERVER_BIN') ?? resolve(Deno.cwd(), 'target/release/pglet_server');
// fuz_app's wasm factory imports the npm adapter package (`create_pglet_db` from
// `index.js`), not the raw wasm-pack `pkg/` — pointing the default there made the
// leg fail instantly on every run.
const WASM_PKG = Deno.env.get('PGLET_WASM_PKG') ?? resolve(Deno.cwd(), 'crates/pglet_wasm/pkg-npm');
const PG_URL = Deno.env.get('PGLET_EXITPROOF_PG_URL'); // opt-in production-ceiling leg

// --- artifact / dir preflight (fail loud, never silently skip) ------------
const require_path = (path: string, hint: string): void => {
	try {
		Deno.statSync(path);
	} catch {
		console.error(`error: missing ${path}\n  ${hint}`);
		Deno.exit(1);
	}
};
require_path(FUZ_APP_DIR, 'set PGLET_FUZ_APP_DIR to the fuz_app checkout');
// The two pglet artifacts are checked below, once the leg filter has said which legs run: a leg
// left out needs no artifact.

// --- discover the shared (all-four-legs) test files -----------------------
// A `*.db.test.ts` file exercises all four legs iff its suites are bound to the
// four-leg factory set: it imports the four-leg `describe_db` (or `db_factories`) from a
// fixture module — `db_fixture.ts` or a sibling built by `create_db_fixture.ts`
// (`cell_db_fixture.ts`, `fact_db_fixture.ts`). Discovered, not hardcoded, so it tracks
// fuz_app as suites are added.
//
// Matching the *module name* alone is not enough. A file that imports only a single
// **named** factory from a fixture module (`pg_factory`, `default_db_factory`) is
// single-driver and excluded: the pg-only suites need connection-level concurrency
// pglite cannot give, and admitting one would add tests to the pg leg alone.
//
// A four-leg file whose every `create_describe_db` is not passed the imported `db_factories`
// is refused, before any leg runs, whichever legs are selected — "shared" is a claim about
// the tree, not about one run's legs. `create_describe_db` names each suite
// `<name> (<factory.name>)`, so a single factory passed to it yields suites only that
// driver's leg's `-t '\(<driver>\)'` selects — `pg_factory` grows the pg leg,
// `default_db_factory` (named `pglite`, as no leg here installs a substitute) or a local
// `create_pglite_factory(…)` the pglite leg — and the legs no longer run the same work.
// Excluding the file would drop its four-leg suites from every leg, and admitting it is
// unfair, so the fix is fuz_app's: the single-driver suite goes in its own file.
//
// The check is an allow-list over every *mention* of `create_describe_db` in a four-leg
// file, comments and strings read as code: a mention is allowed only as a specifier
// inside a braced `import` (not renamed away — `create_describe_db as …` is refused), or
// as a call whose first argument is exactly the imported `db_factories`' local name.
// Anything else is refused, naming the line — a mention in a comment, a type argument, an aliased import, a value reference or one passed
// in an argument list, a factory set it cannot see through (a local alias of `db_factories`,
// a `.filter(…)` of it) — loudly rather than guessed at; a comment mention is a trivial
// rewording. A fixture-module import this read cannot parse (a namespace import, a `}`
// inside the braces) beside a four-leg name is refused too, so a four-leg file is never
// silently dropped as single-driver. A single factory that never
// reaches `create_describe_db` (a `.create()` inside a plain `describe`) names no
// `(<driver>)` suite, so no leg selects it and it is not refused. The `⚠ FAIRNESS` check
// below is the backstop for what a static read of one file cannot see — a
// `create_describe_db` reached through a helper module, a hand-written `(<driver>)` name.
const FIXTURE_IMPORT = /import\s*\{([^}]*)\}\s*from\s*['"]([^'"]*db_fixture\.ts)['"]/g;
/** A fixture module named anywhere, parsed by `FIXTURE_IMPORT` or not. */
const FIXTURE_MODULE = /db_fixture\.ts['"]/g;
/** Any module's braced import — where a `create_describe_db` specifier may stand. */
const BRACED_IMPORT = /import\s*\{[^}]*\}\s*from\s*['"][^'"]*['"]/g;
/** The four-leg bindings; a named single factory (`pg_factory`, …) is not one. */
const FOUR_LEG_BINDINGS = ['describe_db', 'db_factories'];
const FOUR_LEG_NAME = /\b(?:describe_db|db_factories)\b/;
/**
 * Every mention of `create_describe_db`; group 1 is set where it stands as a specifier
 * (`,`/`}` follows) or a call with a bare identifier first argument, group 2 is that
 * argument.
 */
const DESCRIBE_DB_MENTION = /\bcreate_describe_db\b(\s*[,}]|\s*\(\s*([\w$]+)\s*[,)])?/g;

interface FixtureImport {
	/** the exported name */
	name: string;
	/** the local binding (`name as local`) */
	local: string;
}

const parse_fixture_imports = (text: string): FixtureImport[] => {
	const out: FixtureImport[] = [];
	for (const m of text.matchAll(FIXTURE_IMPORT)) {
		for (const spec of m[1].split(',')) {
			const [name, local] = spec
				.trim()
				.replace(/^type\s+/, '')
				.split(/\s+as\s+/)
				.map((s) => s.trim());
			if (name) out.push({name, local: local ?? name});
		}
	}
	return out;
};

/** The `[start, end)` spans of every match of `pattern` (a global regex) in `text`. */
const match_spans = (text: string, pattern: RegExp): Array<[number, number]> =>
	Array.from(text.matchAll(pattern), (m): [number, number] => [m.index, m.index + m[0].length]);

const within = (spans: Array<[number, number]>, index: number): boolean =>
	spans.some(([start, end]) => index >= start && index < end);

type DbTestVerdict =
	| {kind: 'shared'}
	| {kind: 'single_driver'}
	| {kind: 'refused'; offenses: string[]};

/**
 * Classifies one `*.db.test.ts` file's text: shared by all four legs, single-driver
 * (excluded), or refused — a four-leg name beside a fixture import this read cannot parse,
 * or four-leg bindings beside a `create_describe_db` mention the allow-list does not admit.
 */
const classify_db_test = (text: string): DbTestVerdict => {
	const lines = text.split('\n');
	const at = (index: number): string => {
		const line = text.slice(0, index).split('\n').length;
		return `line ${line}: \`${lines[line - 1].trim()}\``;
	};
	const offenses: string[] = [];
	if (FOUR_LEG_NAME.test(text)) {
		const parsed = match_spans(text, FIXTURE_IMPORT);
		for (const m of text.matchAll(FIXTURE_MODULE)) {
			if (within(parsed, m.index)) continue;
			offenses.push(
				`${at(m.index)} — a fixture import this read cannot parse, beside a four-leg name`,
			);
		}
	}
	const imports = parse_fixture_imports(text);
	if (!imports.some((i) => FOUR_LEG_BINDINGS.includes(i.name))) {
		return offenses.length ? {kind: 'refused', offenses} : {kind: 'single_driver'};
	}
	const factories = imports.find((i) => i.name === 'db_factories')?.local;
	const braced_imports = match_spans(text, BRACED_IMPORT);
	for (const m of text.matchAll(DESCRIBE_DB_MENTION)) {
		if (m[1] !== undefined) {
			if (m[2] === undefined ? within(braced_imports, m.index) : m[2] === factories) continue;
		}
		offenses.push(
			`${at(m.index)} — a four-leg file binds its suites through \`describe_db\` /` +
				` \`db_factories\` only`,
		);
	}
	return offenses.length ? {kind: 'refused', offenses} : {kind: 'shared'};
};

const walk_db_tests = async (dir: string, out: string[] = []): Promise<string[]> => {
	for await (const entry of Deno.readDir(dir)) {
		const path = `${dir}/${entry.name}`;
		if (entry.isDirectory) await walk_db_tests(path, out);
		else if (entry.name.endsWith('.db.test.ts')) out.push(path);
	}
	return out;
};

// Refuses (exit 1) when any file would make the legs run different tests — a four-leg
// file that also binds a single-driver suite — or carries a fixture import it cannot
// read, naming each — before any leg runs, whichever legs `PGLET_EXITPROOF_LEGS` selects
// and whether or not `PGLET_EXITPROOF_PG_URL` is set, since the shared set is a claim about fuz_app's tree, not
// about this run's legs.
const discover_shared_files = async (): Promise<string[]> => {
	const root = `${FUZ_APP_DIR}/src/test`;
	const all = (await walk_db_tests(root)).sort();
	const shared: string[] = [];
	const refused: string[] = [];
	for (const abs of all) {
		const rel = abs.slice(FUZ_APP_DIR.length + 1); // relative to fuz_app cwd
		const verdict = classify_db_test(await Deno.readTextFile(abs));
		if (verdict.kind === 'shared') shared.push(rel);
		else if (verdict.kind === 'refused') {
			refused.push(`  ${rel}\n${verdict.offenses.map((o) => `    ${o}`).join('\n')}`);
		}
	}
	if (refused.length) {
		console.error(
			`error: fuz_app file(s) the shared-set discovery refuses — each would make the legs run` +
				` different tests, or its fixture import cannot be read:\n` +
				refused.join('\n') +
				`\n  hint: split a single-driver suite into its own *.db.test.ts file; write a fixture` +
					` import as \`import {…} from '…db_fixture.ts'\``,
		);
		Deno.exit(1);
	}
	return shared;
};

// Discovered now, ahead of the artifact checks and every leg, so a refusal costs nothing.
const shared_files = await discover_shared_files();

// --- legs -----------------------------------------------------------------
interface Leg {
	/** display + JSON key */
	name: string;
	/** L1 in-process, or L2 out-of-process (never compared across the line). */
	tier: 'L1' | 'L2';
	/** the `(driver)` describe suffix vitest filters on. */
	driver: string;
	/** leg-specific env layered onto the minimal base. */
	env: Record<string, string>;
	skip?: string;
}

// Start every pass from an empty database. This is the coarse half of the
// isolation and by itself it is not enough — the assertions that read the whole
// `public` schema are tripped by DDL from an *earlier file in the same pass*,
// which only fuz_app's per-`create()` schema reset can prevent (see the module
// doc). Kept because it costs nothing: it runs outside the timed window (timing
// is read from vitest's own report — the Duration line's total and the file-times
// reporter's per-file times), and it means a pass cannot inherit
// whatever a previous run, or a hand-run query, left in the scratch database.
const reset_pg_database = async (): Promise<void> => {
	if (!PG_URL) return;
	const url = new URL(PG_URL);
	const dbname = decodeURIComponent(url.pathname.slice(1));
	const admin = new URL(PG_URL);
	admin.pathname = '/postgres';
	for (const sql of [`DROP DATABASE IF EXISTS "${dbname}"`, `CREATE DATABASE "${dbname}"`]) {
		const {code, stderr} = await new Deno.Command('psql', {
			args: [admin.toString(), '-v', 'ON_ERROR_STOP=1', '-Atc', sql],
			stdout: 'null',
			stderr: 'piped',
		}).output();
		if (code !== 0) {
			console.error(`error: pg-leg database reset failed: ${new TextDecoder().decode(stderr)}`);
			Deno.exit(1);
		}
	}
};

const legs: Leg[] = [
	{name: 'pglite', tier: 'L1', driver: 'pglite', env: {}},
	{name: 'pglet-wasm', tier: 'L1', driver: 'pglet-wasm', env: {PGLET_WASM_PKG: WASM_PKG}},
	{name: 'pglet-server', tier: 'L2', driver: 'pglet', env: {PGLET_SERVER_BIN: SERVER_BIN}},
	{
		name: 'pg',
		tier: 'L2',
		driver: 'pg',
		env: PG_URL ? {TEST_DATABASE_URL: PG_URL} : {},
		skip: PG_URL ? undefined : 'PGLET_EXITPROOF_PG_URL not set',
	},
];

// Optional leg filter, comma-separated (`PGLET_EXITPROOF_LEGS=pglite,pglet-wasm,pg`): a run of
// only the legs a change touches. An unknown name is a hard error rather than a silent no-op,
// so a typo cannot quietly measure a different set than the one asked for.
const LEGS_FILTER = Deno.env.get('PGLET_EXITPROOF_LEGS');
if (LEGS_FILTER) {
	const wanted = new Set(
		LEGS_FILTER.split(',')
			.map((n) => n.trim())
			.filter(Boolean),
	);
	const unknown = [...wanted].filter((n) => !legs.some((l) => l.name === n));
	if (unknown.length) {
		console.error(
			`error: PGLET_EXITPROOF_LEGS names unknown leg(s): ${unknown.join(', ')}` +
				`\n  known legs: ${legs.map((l) => l.name).join(', ')}`,
		);
		Deno.exit(1);
	}
	for (const leg of legs) if (!wanted.has(leg.name)) leg.skip = 'not in PGLET_EXITPROOF_LEGS';
}
const runs_leg = (name: string): boolean => legs.some((l) => l.name === name && !l.skip);
if (runs_leg('pglet-server')) {
	require_path(SERVER_BIN, 'run `deno task matrix:artifacts` to build the release server');
}
if (runs_leg('pglet-wasm')) {
	require_path(`${WASM_PKG}/pglet_wasm_bg.wasm`, 'run `deno task build:npm` to build the adapter pkg');
	// The wasm leg loads the npm adapter (`pkg-npm`), never the raw wasm-pack `pkg` —
	// so `deno task build:npm` is what freshens it. `bench:exit-proof` chains that one
	// for exactly this reason; running this file directly measures whatever `pkg-npm`
	// happens to hold, which can be an older engine than the server binary beside it.
	require_path(`${WASM_PKG}/index.js`, 'run `deno task build:npm` — the wasm leg needs the npm adapter, not the raw wasm-pack pkg');
}

// --- engine provenance ----------------------------------------------------
// What each leg ran, recorded under `engines`: the pglite leg runs fuz_app's own pglite —
// resolved from the fuz_app checkout as its suite's `@electric-sql/pglite` import is, a
// different install from `benches/`' — so the exit-proof multiples' pglite version is read
// there, never assumed to be the one the size comparison weighs and the head-to-head cold
// inits run. Read before the run, so an unresolvable one refuses now rather than after it;
// `null` for a leg not run.
const fail = (message: string): never => {
	console.error(`error: ${message}`);
	Deno.exit(1);
};
const engine_version = (dir: string): string => {
	try {
		return package_version(dir);
	} catch (e) {
		return fail(e instanceof Error ? e.message : String(e));
	}
};
const FUZ_APP_PGLITE = runs_leg('pglite') ? resolve_package_dir(FUZ_APP_DIR, PGLITE) : null;
if (runs_leg('pglite') && FUZ_APP_PGLITE === null) {
	fail(`no ${PGLITE} resolves from ${FUZ_APP_DIR} — install fuz_app's dependencies`);
}
/** The pglite the pglite leg loads: fuz_app's install. */
const PGLITE_VERSION = FUZ_APP_PGLITE === null ? null : engine_version(FUZ_APP_PGLITE);
/** The npm package the pglet-wasm leg loads. */
const PGLET_WASM_VERSION = runs_leg('pglet-wasm') ? engine_version(WASM_PKG) : null;
/** The fuz_app checkout every leg runs, `-dirty` by `git_commit`'s rule for another checkout. */
const FUZ_APP_COMMIT = git_commit(FUZ_APP_DIR);

// --- one vitest run -------------------------------------------------------
interface RunResult {
	/** The run's wall time, vitest's Duration line. */
	total_ms: number;
	/**
	 * The engine bucket: the sum of the test files' run times, each file's hooks included
	 * (`lib/vitest_file_times.mjs`); `NaN` when the run wrote no report.
	 */
	tests_ms: number;
	passed: number;
	failed: number;
	files: number;
	/** vitest's exit status; non-zero fails the leg. */
	exit_code: number;
}

const ANSI = /\x1b\[[0-9;]*m/g;
/** Parse a vitest duration token like `65.15s` / `402ms` into ms. */
const dur_ms = (raw: string): number => {
	const m = /([\d.]+)\s*(ms|s)/.exec(raw);
	if (!m) return NaN;
	return m[2] === 's' ? Number(m[1]) * 1000 : Number(m[1]);
};

/** `tests_ms` from the file-times report's JSON; `NaN` when it is missing or unreadable. */
const read_tests_ms = (report_json: string | null): number => {
	if (report_json == null) return NaN;
	try {
		const tests_ms = (JSON.parse(report_json) as {tests_ms: unknown}).tests_ms;
		return typeof tests_ms === 'number' ? tests_ms : NaN;
	} catch {
		return NaN;
	}
};

const parse_summary = (out: string, report_json: string | null): Omit<RunResult, 'exit_code'> => {
	const text = out.replace(ANSI, '');
	const grab = (re: RegExp): string => re.exec(text)?.[1] ?? '';
	// Only the Duration line's total is read from the summary. Its bucket list is not: vitest 5
	// (fuz_app's) prints `Duration  4.87s (tests 49%, transform 36%, …)`, percentages of the summed
	// phases, which run in parallel workers — not a share of the wall time and not convertible to
	// one. The engine bucket comes from the file-times reporter, whatever the version.
	return {
		total_ms: dur_ms(/Duration\s+([\d.]+\s*(?:ms|s))/.exec(text)?.[1] ?? ''),
		tests_ms: read_tests_ms(report_json),
		passed: Number(grab(/Tests\s+(\d+)\s+passed/) || 0),
		failed: Number(grab(/Tests\s+.*?(\d+)\s+failed/) || 0),
		files: Number(grab(/Test Files\s+(\d+)\s+passed/) || 0),
	};
};

/** The reporter that writes each run's engine bucket (`lib/vitest_file_times.mjs`). */
const FILE_TIMES_REPORTER = new URL('./lib/vitest_file_times.mjs', import.meta.url).pathname;

const run_leg_once = async (leg: Leg, files: string[]): Promise<{r: RunResult; out: string}> => {
	// Minimal env (mirrors the validated `env -i PATH HOME`) + leg vars, so a
	// pglet var never leaks into the pglite leg.
	const base: Record<string, string> = {
		PATH: Deno.env.get('PATH') ?? '',
		HOME: Deno.env.get('HOME') ?? '',
	};
	// The file-times reporter, beside the dot reporter, writes the engine bucket here — outside
	// the fuz_app checkout, removed after.
	const report_path = await Deno.makeTempFile({prefix: 'pglet_exit_proof_', suffix: '.json'});
	const cmd = new Deno.Command('npx', {
		args: [
			'vitest',
			'run',
			'--project',
			'db',
			'--reporter=dot',
			`--reporter=${FILE_TIMES_REPORTER}`,
			// A failing leg isn't comparable anyway; `--bail=1` aborts it on the first
			// failure (fast) instead of grinding through per-test timeouts. Green legs
			// have zero failures, so bail never fires — timing is unaffected.
			'--bail=1',
			...files,
			'-t',
			`\\(${leg.driver}\\)`,
		],
		cwd: FUZ_APP_DIR,
		env: {...base, ...leg.env, PGLET_EXITPROOF_FILE_TIMES: report_path},
		stdout: 'piped',
		stderr: 'piped',
	});
	const {code, stdout, stderr} = await cmd.output();
	const out = new TextDecoder().decode(stdout) + new TextDecoder().decode(stderr);
	let report_json: string | null = null;
	try {
		report_json = await Deno.readTextFile(report_path);
	} catch {
		// no report: `tests_ms` is `NaN`, and the leg fails below
	}
	await Deno.remove(report_path).catch(() => {});
	return {r: {...parse_summary(out, report_json || null), exit_code: code}, out};
};

// --- measure --------------------------------------------------------------
interface Summary {
	name: string;
	tier: 'L1' | 'L2';
	passed: number;
	failed: number;
	files: number;
	leg_failed: boolean;
	total: Stat;
	tests: Stat;
}
interface Stat {
	p50: number;
	p95: number;
	mean: number;
	min: number;
	max: number;
	cv: number;
}

const summarize = (xs: number[]): Stat => {
	if (xs.length === 0) return {p50: NaN, p95: NaN, mean: NaN, min: NaN, max: NaN, cv: NaN};
	const mean = stats_mean(xs);
	const {min, max} = stats_min_max(xs);
	return {
		p50: stats_median(xs),
		p95: stats_percentile(xs, 0.95),
		mean,
		min,
		max,
		cv: stats_cv(mean, stats_std_dev(xs, mean)),
	};
};

const active_legs = legs.filter((l) => !l.skip);
const skipped_legs = legs.filter((l) => l.skip);

console.log(`\npglet exit proof — fuz_app \`db\` suite wall-time, pglet (both legs) vs pglite`);
console.log(
	`\nshared files (all four legs bind these): ${shared_files.length}` +
		`  ·  reps=${REPS} (+${WARMUP} warmup)  ·  legs: ${active_legs.map((l) => l.name).join(', ')}` +
		(skipped_legs.length ? `  ·  skipped: ${skipped_legs.map((l) => `${l.name} (${l.skip})`).join(', ')}` : ''),
);
console.log(`server=${SERVER_BIN}\nwasm=${WASM_PKG}\nfuz_app=${FUZ_APP_DIR}`);
console.log(
	`engines: pglite ${PGLITE_VERSION ?? '—'} (fuz_app's)  ·  pglet-wasm ${PGLET_WASM_VERSION ?? '—'}` +
		`  ·  fuz_app ${FUZ_APP_COMMIT}\n`,
);

// rep-major, leg-minor: interleave so drift hits every leg evenly.
const enc = new TextEncoder();
const log_err = (s: string): void => void Deno.stderr.writeSync(enc.encode(s));
interface LegState {
	total: number[];
	tests: number[];
	last: RunResult | null;
	failed: boolean;
}
const samples: Record<string, LegState> = {};
for (const leg of active_legs) samples[leg.name] = {total: [], tests: [], last: null, failed: false};

for (let rep = 0; rep < WARMUP + REPS; rep++) {
	const phase = rep < WARMUP ? `warmup ${rep + 1}/${WARMUP}` : `rep ${rep - WARMUP + 1}/${REPS}`;
	for (const leg of active_legs) {
		const st = samples[leg.name];
		if (st.failed) {
			log_err(`  [${phase}] ${leg.name} … skipped (leg failed earlier)\n`);
			continue;
		}
		log_err(`  [${phase}] ${leg.name} … `);
		if (leg.name === 'pg') await reset_pg_database();
		const {r, out} = await run_leg_once(leg, shared_files);
		// A leg with any failure (or zero passes) isn't comparable — mark it and
		// stop repeating it (saves grinding a broken leg through every rep). Nor is a run
		// whose engine bucket could not be read: no other number stands in for it.
		const no_bucket = !Number.isFinite(r.tests_ms);
		const bad = r.failed > 0 || r.passed === 0 || no_bucket || r.exit_code !== 0;
		log_err(
			`${(r.total_ms / 1000).toFixed(2)}s total / ${(r.tests_ms / 1000).toFixed(2)}s tests` +
				` (${r.passed} passed${r.failed ? `, ${r.failed} FAILED` : ''})` +
				(no_bucket ? '  ⚠ no file-times report — the tests bucket was not measured' : '') +
				(r.exit_code !== 0 ? `  ⚠ vitest exited ${r.exit_code}` : '') +
				`${bad ? '  ⚠ leg failed — skipping remaining reps' : ''}\n`,
		);
		st.last = r;
		if (bad) {
			st.failed = true;
			// The parsed counters can't say *what* failed — surface the run's own
			// tail so a failed leg is diagnosable without re-running it.
			const tail = out.replace(ANSI, '').trimEnd().split('\n').slice(-30).join('\n');
			log_err(`  ── ${leg.name} failure output (tail) ──\n${tail}\n  ── end ${leg.name} tail ──\n`);
			continue;
		}
		if (rep >= WARMUP) {
			st.total.push(r.total_ms);
			st.tests.push(r.tests_ms);
		}
	}
}

const summaries: Summary[] = active_legs.map((leg) => {
	const s = samples[leg.name];
	return {
		name: leg.name,
		tier: leg.tier,
		passed: s.last?.passed ?? 0,
		failed: s.last?.failed ?? 0,
		files: s.last?.files ?? 0,
		leg_failed: s.failed || s.total.length === 0,
		total: summarize(s.total),
		tests: summarize(s.tests),
	};
});

// --- report ---------------------------------------------------------------
const s2 = (n: number): string => (Number.isFinite(n) ? (n / 1000).toFixed(2) : '—');
const pad = (s: string, n: number): string => s.padEnd(n);
const padl = (s: string, n: number): string => s.padStart(n);

// Fairness sanity: every *green* leg must run the same passed-count.
const green = summaries.filter((r) => !r.leg_failed);
const counts = new Set(green.map((r) => r.passed));
const any_failed = summaries.some((r) => r.leg_failed);
const pct = (r: Summary): string => (r.leg_failed || !Number.isFinite(r.tests.cv) ? '—' : (r.tests.cv * 100).toFixed(1) + '%');
console.log(`\nper-leg wall-time  (seconds; p50 of ${REPS} reps)`);
console.log(
	`${pad('leg', 14)}${pad('tier', 6)}${padl('total p50', 11)}${padl('tests p50', 11)}${padl('tests cv', 10)}${padl('passed', 9)}`,
);
console.log('-'.repeat(71));
for (const r of summaries) {
	console.log(
		`${pad(r.name, 14)}${pad(r.tier, 6)}${padl(s2(r.total.p50), 11)}${padl(s2(r.tests.p50), 11)}` +
			`${padl(pct(r), 10)}${padl(String(r.passed) + (r.failed ? `/${r.failed}✗` : ''), 9)}`,
	);
}

// The figures the README quotes, computed once and both printed and recorded — never a
// second, separately-divided set. A ratio over a leg that failed or was not run, or over two
// legs that passed different test counts, is `null` with its reason.
const ratios = exit_proof_ratios(summaries);
console.log(`\nratios  (× ; numerator p50 / denominator p50 — >1 = the denominator is faster)`);
console.log(`${pad('key', 31)}${pad('bucket', 8)}${padl('×', 7)}  pair`);
console.log('-'.repeat(86));
for (const [key, ratio] of Object.entries(ratios)) {
	const pair = `${ratio.numerator} / ${ratio.denominator}`;
	const value =
		ratio.value === null
			? `${padl('n/a', 7)}  ${pair} — ${ratio.null_reason}`
			: `${padl(ratio.value.toFixed(3), 7)}  ${pair}`;
	const note = ratio.crosses_tiers ? ' — crosses tiers: in-process vs over TCP, quote it labelled' : '';
	console.log(`${pad(key, 31)}${pad(ratio.bucket, 8)}${value}${note}`);
}

console.log(
	`\ncomponent split (p50, one leg): total = transform + import + tests(+env). The` +
		`\n  \`tests\` bucket is engine work (beforeAll migrations + per-test TRUNCATE +` +
		`\n  query bodies + one amortized engine boot over ${shared_files.length} files); transform+import` +
		`\n  (~${s2(summaries[0]?.total.p50 - summaries[0]?.tests.p50)}s) is fixed vitest overhead shared by every leg.`,
);
console.log(
	`\nNOTE: fuz_app shares one engine instance across the whole \`db\` run (isolate:false,` +
		`\n  fileParallelism:false), so each engine's cold boot is paid once and amortized —` +
		`\n  the per-file spin-up win pglet holds decisively (wasm_compare cold-init ~80–100× vs` +
		`\n  pglite, runtime-dependent — see wasm_compare_results.json) shows here only as its` +
		`\n  ~1/${shared_files.length} share. This suite measures steady-state throughput on a real` +
		`\n  consumer, the complement to wasm_compare's cold-start number.`,
);
if (counts.size > 1) {
	console.log(
		`\n⚠ FAIRNESS: green legs ran different passed-counts (${green.map((r) => `${r.name}:${r.passed}`).join(', ')})` +
			` — the comparison is only valid on an identical test set, so each ratio over two of them is` +
			` recorded null and this run is FATAL (exit 1 below).` +
			`\n  A leg running extra tests is a single-driver suite discovery did not catch reaching the` +
			`\n  shared set (see \`classify_db_test\`); fix that rather than the ratio.`,
	);
}
if (any_failed) {
	console.log(
		`\n⚠ FAILURES: ${summaries.filter((r) => r.leg_failed).map((r) => r.name).join(', ')} could not complete the shared suite` +
			` (see passed/✗ above) — no comparable wall-time for that leg, so this run is FATAL (exit 1 below).` +
			`\n  The recorded JSON still holds whatever the other legs measured; a green L1 pair (pglite, pglet-wasm)` +
			`\n  stands on its own if that is all you needed.`,
	);
}

// --- results JSON ---------------------------------------------------------
const update = Deno.args.includes('--update');
if (update) {
	const doc = {
		schema: 'exit_proof',
		note:
			'fuz_app `db` suite wall-time per engine leg, isolated by testNamePattern over the ' +
			'shared four-leg files. L1 (pglite, pglet-wasm) in-process; L2 (pglet-server, pg) ' +
			'out-of-process — never compared across the line but by a `ratios` entry marked ' +
			'`crosses_tiers`, which is quoted only labelled as in-process vs over TCP. `ratios` holds ' +
			'the figures the README quotes, each a numerator leg p50 over a denominator leg p50 in one ' +
			'bucket, `null` with a `null_reason` where either leg failed or was not run or the two ' +
			'passed different test counts. `engines` records what the legs ran: `pglite` is the ' +
			"pglite fuz_app's install resolves (not benches/'), `pglet_wasm` the npm package's " +
			'version, `fuz_app` the checkout\'s commit. Ratios are the durable signal; ' +
			'absolute seconds flap with the box. Engine boot is amortized over the file set (see ' +
			'wasm_compare_results.json for the cold-start number).',
		timestamp: new Date().toISOString(),
		git_commit: git_commit(),
		reps: REPS,
		warmup: WARMUP,
		shared_files: shared_files.length,
		results: summaries,
		ratios,
		engines: {
			pglite: PGLITE_VERSION,
			pglet_wasm: PGLET_WASM_VERSION,
			fuz_app: FUZ_APP_COMMIT,
		},
	};
	await write_json('benches/exit_proof_results.json', doc);
	console.error(`\nwrote benches/exit_proof_results.json`);
} else {
	console.error(`\n(results not written — pass --update to refresh exit_proof_results.json)`);
}

// Fatal *after* the write: the counts are recorded in the JSON either way, and a
// 25-minute run should not have to be repeated to see what it measured. A leg that
// was asked for and could not complete is fatal too — a run whose whole purpose is
// one leg's number should not exit 0 without it, which is how the pg leg stayed
// broken across several sessions.
const fatal: string[] = [];
if (counts.size > 1) {
	fatal.push(
		`green legs ran different passed-counts (${green.map((r) => `${r.name}:${r.passed}`).join(', ')})` +
			` — the ratios over two of them are recorded null`,
	);
}
const failed_legs = summaries.filter((r) => r.leg_failed).map((r) => r.name);
if (failed_legs.length) fatal.push(`leg(s) failed to complete: ${failed_legs.join(', ')}`);
if (fatal.length) {
	console.error(`\nerror: ${fatal.join('\n       ')}`);
	Deno.exit(1);
}
