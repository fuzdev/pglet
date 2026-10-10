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
import {writeFile} from 'node:fs/promises';

/**
 * The short commit a result was measured at, or `unknown` outside a git
 * checkout (or without git on PATH).
 *
 * Never throws: a bench that has already done its measuring must not lose it to
 * a failure in the stamping.
 */
export const git_commit = (): string => {
	try {
		// `execFileSync`, not `execSync`: the latter runs the command through a shell,
		// so the program Deno's sandbox is asked to spawn is `sh` rather than `git` —
		// and `benches/size.ts` is the one consumer here that runs under a narrow
		// `--allow-run=gzip,git`. The catch below then swallowed the permission denial
		// and stamped `"unknown"` into `size_baseline.json`, which is a *provenance*
		// field: the baseline recorded which build it measured, and silently stopped.
		// The other four consumers run under `-A` and never saw it.
		return execFileSync('git', ['rev-parse', '--short', 'HEAD'], {
			encoding: 'utf8',
			stdio: ['ignore', 'pipe', 'ignore'],
		}).trim();
	} catch {
		return 'unknown';
	}
};

/**
 * Write a results document: tab-indented JSON with a trailing newline, the
 * format every `benches/*_results.json` is already in.
 */
export const write_json = async (path: string | URL, doc: unknown): Promise<void> => {
	await writeFile(path, JSON.stringify(doc, null, '\t') + '\n');
};
