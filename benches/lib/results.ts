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
 * **Plain Node on purpose.** `wasm_compare.ts` runs under Node *and* Bun, so these
 * use `node:` builtins and no gro host module (Bun is not a gro runtime). What a stamp
 * and the JSON envelope mean is `src/lib/bench_results.ts`'s, a module with no runtime
 * imports, which the size gate's gro task follows too: one rule, two ways to reach git.
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
import {fileURLToPath} from 'node:url';
import {
	BENCH_COMMIT_ARGS,
	BENCH_STAMP_UNKNOWN,
	bench_status_is_dirty,
	format_bench_json,
	to_bench_stamp,
	to_bench_status_args,
} from '../../src/lib/bench_results.ts';

/** This checkout, found from this file, which every stamp of this repo is taken in. */
const REPO = fileURLToPath(new URL('../../', import.meta.url));

/**
 * The short commit a result was measured at, suffixed `-dirty` when the tree differs
 * from it, or `unknown` outside a git checkout (or without git on PATH): the rule is
 * `src/lib/bench_results.ts`'s, which the size gate's stamp follows too.
 *
 * Never throws: a bench that has already done its measuring must not lose it to
 * a failure in the stamping. A failure of the dirty check alone keeps the commit
 * and stamps it `-dirty?`, rather than claiming a clean tree it could not see.
 *
 * @param cwd - another checkout to stamp (`exit_proof.ts` stamps the fuz_app it ran);
 * omitted, this repo, found from this file rather than the working directory
 */
export const git_commit = (cwd?: string): string => {
	const other_checkout = cwd !== undefined;
	const dir = cwd ?? REPO;
	let commit: string;
	try {
		commit = git(BENCH_COMMIT_ARGS, dir).trim();
	} catch {
		return BENCH_STAMP_UNKNOWN;
	}
	if (!commit) return BENCH_STAMP_UNKNOWN;
	let dirty: boolean | null;
	try {
		dirty = bench_status_is_dirty(git(to_bench_status_args(other_checkout), dir), other_checkout);
	} catch {
		dirty = null;
	}
	return to_bench_stamp(commit, dirty);
};

const git = (args: ReadonlyArray<string>, cwd: string): string =>
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
	await writeFile(path, format_bench_json(doc));
};
