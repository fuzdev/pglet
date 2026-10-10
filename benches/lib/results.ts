/**
 * The two things every bench in this directory does with its results, in one
 * place instead of seven.
 *
 * `git_commit()` was copied into all seven bench scripts — the same `git
 * rev-parse --short HEAD`, the same "unknown" on failure — and the envelope
 * around it (tab-indented JSON, trailing newline) was open-coded beside each
 * one. That is a small duplication with a real cost: every recorded
 * `benches/*_results.json` is read as a comparable artifact, so their common
 * fields have to be produced the same way, and seven copies is seven chances to
 * stamp a different one. (The statistics had the same shape of drift and it had
 * already gone wrong — see `crates/pglet_bench/src/stats.rs`.)
 *
 * `package_version` sits beside them for the same reason: an engine's version is the
 * other provenance a document records, read one way wherever it is recorded.
 *
 * **Cross-runtime on purpose.** `wasm_compare.ts` runs under Deno, Node *and*
 * Bun, so these use `node:` builtins rather than `Deno.*` — every runtime this
 * directory targets supports them, while `Deno.Command` and
 * `Deno.writeTextFile` would shut two of them out and leave the duplicate in
 * place for exactly the file that needs it most.
 *
 * This module deliberately does **not** own the document shape. Each bench
 * writes its own keys in its own order, because the recorded files are data a
 * reader diffs across commits and a reordering churns every line of them.
 *
 * @module
 */

import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {writeFile} from 'node:fs/promises';
import {join} from 'node:path';

/**
 * The recorded results documents the benches write. A run that writes one leaves the
 * tree modified, and a later step of the same `cargo xtask measure` run must not read
 * its predecessor's output as the tree having moved — so these never make a stamp dirty.
 * Git pathspecs, relative to the repo root.
 */
const RECORDED_OUTPUTS = [
	'benches/*_results.json',
	'benches/size_baseline.json',
	'benches/pglite_size_comparison.json',
];

/**
 * Untracked files count toward a dirty stamp only under these: the directories whose
 * files build the measured artifacts (`crates/`, `scripts/` — the npm package's patch
 * step) or are the measuring code itself (`benches/`). An untracked file anywhere else —
 * a scratch note, an editor file — changes nothing a bench measures.
 */
const UNTRACKED_COUNTS_UNDER = ['benches/', 'crates/', 'scripts/'];

/**
 * The short commit a result was measured at, suffixed `-dirty` when the tree differs
 * from it, or `unknown` outside a git checkout (or without git on PATH).
 *
 * Dirty, for this repo, is any tracked file modified (staged or not), or an untracked file
 * under `UNTRACKED_COUNTS_UNDER`, leaving out the recorded results documents
 * (`RECORDED_OUTPUTS`); for another checkout (`cwd`), it is any change `git status` shows,
 * since this repo's layout says nothing about which of its files a bench depends on.
 * Gitignored files never count — build output, `node_modules`.
 *
 * Never throws: a bench that has already done its measuring must not lose it to
 * a failure in the stamping. A failure of the dirty check alone keeps the commit
 * and stamps it `-dirty?`, rather than claiming a clean tree it could not see.
 *
 * @param cwd - another checkout to stamp (`exit_proof.ts` stamps the fuz_app it ran);
 * omitted, this repo, which every bench runs from
 */
export const git_commit = (cwd?: string): string => {
	let commit: string;
	try {
		// `execFileSync`, not `execSync`: the latter runs the command through a shell,
		// so the program Deno's sandbox is asked to spawn is `sh` rather than `git` —
		// and `benches/size.ts` and `benches/size_compare.ts` run under a narrow
		// `--allow-run=gzip,git`. The catch below then swallowed the permission denial
		// and stamped `"unknown"` into `size_baseline.json`, which is a *provenance*
		// field: the baseline recorded which build it measured, and silently stopped.
		// The consumers under `-A` never saw it. (`--allow-env` is needed beside it:
		// `node:child_process` reads the environment, and without it this stamps
		// `"unknown"` the same silent way — both tasks pass it.)
		commit = git(['rev-parse', '--short', 'HEAD'], cwd).trim();
	} catch {
		return 'unknown';
	}
	try {
		return git_tree_is_dirty(cwd) ? `${commit}-dirty` : commit;
	} catch {
		return `${commit}-dirty?`;
	}
};

/**
 * Whether the working tree differs from HEAD by `git_commit`'s rule.
 *
 * @param cwd - another checkout, judged by any change; omitted, this repo
 * @throws when git fails
 */
export const git_tree_is_dirty = (cwd?: string): boolean => {
	// `-z`: each entry NUL-terminated and its path verbatim — without it git quotes a path
	// holding a space or a non-ASCII byte, and the prefix test below reads the quote.
	// Paths are relative to the repo root wherever this runs from, which is what the
	// prefixes compare against; `:(top)` anchors the pathspecs there likewise.
	const status = git(
		cwd === undefined
			? [
					'status',
					'--porcelain',
					'-z',
					'--untracked-files=all',
					'--',
					':(top)',
					...RECORDED_OUTPUTS.map((p) => `:(top,glob,exclude)${p}`),
				]
			: ['status', '--porcelain', '-z', '--untracked-files=all'],
		cwd,
	);
	// An entry is `XY path`; a rename or copy carries its source path as one more NUL-terminated
	// field after it, which is never read — a tracked entry decides before it is reached.
	for (const entry of status.split('\0')) {
		if (!entry) continue;
		if (cwd !== undefined || !entry.startsWith('?? ')) return true;
		const path = entry.slice(3);
		if (UNTRACKED_COUNTS_UNDER.some((dir) => path.startsWith(dir))) return true;
	}
	return false;
};

const git = (args: string[], cwd?: string): string =>
	execFileSync('git', args, {cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore']});

/**
 * The `version` a package's `package.json` records — provenance a results document carries
 * beside its commit.
 *
 * @param dir - the package directory
 * @throws when the `package.json` is unreadable or records no version
 */
export const package_version = (dir: string): string => {
	const manifest = join(dir, 'package.json');
	const {version} = JSON.parse(readFileSync(manifest, 'utf8')) as {version?: unknown};
	if (typeof version !== 'string') throw new Error(`${manifest} records no version`);
	return version;
};

/**
 * Write a results document: tab-indented JSON with a trailing newline, the
 * format every `benches/*_results.json` is already in.
 */
export const write_json = async (path: string | URL, doc: unknown): Promise<void> => {
	await writeFile(path, JSON.stringify(doc, null, '\t') + '\n');
};
