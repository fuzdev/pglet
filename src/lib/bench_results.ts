/**
 * The rules every recorded `benches/*.json` shares, in one place for its two
 * kinds of writer: the plain Node scripts under `benches/` (`benches/lib/results.ts`,
 * which Bun runs too, so it reaches git through `node:child_process`) and the size
 * gate (`size_gate.ts`, a gro task's library, reaching git and gzip through
 * `SpawnProgram`s). Each does its own I/O; what the I/O means is decided here, so
 * a stamp or a gzip figure is produced the same way whichever wrote it.
 *
 * Pure but for `git_bench_stamp`, which takes its program as a dep, and free of
 * runtime imports, so a plain script under Node or Bun imports it as it is.
 *
 * @module
 */

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import type { SpawnProgram } from '@fuzdev/gro/spawn.ts';

/**
 * The recorded results documents the benches write. A run that writes one leaves the
 * tree modified, and a later step of the same `cargo xtask measure` run must not read
 * its predecessor's output as the tree having moved, so these never make a stamp dirty.
 * Git pathspecs, relative to the repo root.
 */
export const BENCH_RECORDED_OUTPUTS: ReadonlyArray<string> = [
	'benches/*_results.json',
	'benches/size_baseline.json',
	'benches/pglite_size_comparison.json'
];

/**
 * Untracked files count toward a dirty stamp only under these: the directories whose
 * files build the measured artifacts (`crates/`, `src/`, the npm package's patch step
 * and the size gate, and `scripts/`) or are the measuring code itself (`benches/`). An
 * untracked file anywhere else, a scratch note or an editor file, changes nothing a
 * bench measures.
 */
export const BENCH_UNTRACKED_COUNTS_UNDER: ReadonlyArray<string> = [
	'benches/',
	'crates/',
	'scripts/',
	'src/'
];

/** The git arguments that print the short commit a stamp names. */
export const BENCH_COMMIT_ARGS: ReadonlyArray<string> = ['rev-parse', '--short', 'HEAD'];

/**
 * The git arguments whose output `bench_status_is_dirty` reads.
 *
 * `-z` gives each entry NUL-terminated with its path verbatim: without it git quotes a
 * path holding a space or a non-ASCII byte, and the prefix test reads the quote. Paths
 * are relative to the repo root wherever git runs in it, which is what the prefixes
 * compare against, and `:(top)` anchors the pathspecs there likewise.
 *
 * @param other_checkout - whether git runs in another checkout (`exit_proof.ts` stamps the
 * fuz_app it ran), where every change counts and no pathspec applies
 */
export const to_bench_status_args = (other_checkout: boolean): Array<string> =>
	other_checkout
		? ['status', '--porcelain', '-z', '--untracked-files=all']
		: [
				'status',
				'--porcelain',
				'-z',
				'--untracked-files=all',
				'--',
				':(top)',
				...BENCH_RECORDED_OUTPUTS.map((p) => `:(top,glob,exclude)${p}`)
			];

/**
 * Whether `git status` output (from `to_bench_status_args`) shows the tree differs from
 * HEAD. For this repo, any tracked file modified (staged or not), or an untracked file
 * under `BENCH_UNTRACKED_COUNTS_UNDER`, the recorded results documents left out by the
 * pathspecs; for another checkout, any entry, since this repo's layout says nothing about
 * which of its files a bench depends on. Gitignored files never appear.
 *
 * @param status - the NUL-separated porcelain output
 * @param other_checkout - as given to `to_bench_status_args`
 */
export const bench_status_is_dirty = (status: string, other_checkout: boolean): boolean => {
	// an entry is `XY path`; a rename or copy carries its source path as one more
	// NUL-terminated field after it, which is never read, since a tracked entry decides
	// before it is reached
	for (const entry of status.split('\0')) {
		if (!entry) continue;
		if (other_checkout || !entry.startsWith('?? ')) return true;
		const path = entry.slice(3);
		if (BENCH_UNTRACKED_COUNTS_UNDER.some((dir) => path.startsWith(dir))) return true;
	}
	return false;
};

/**
 * The stamp a results document records: the short commit, suffixed `-dirty` when the
 * tree differs from it, `-dirty?` when that could not be read (`null`).
 */
export const to_bench_stamp = (commit: string, dirty: boolean | null): string =>
	dirty === null ? `${commit}-dirty?` : dirty ? `${commit}-dirty` : commit;

/** The stamp of a checkout git can't read: outside a git checkout, or without git. */
export const BENCH_STAMP_UNKNOWN = 'unknown';

/**
 * The stamp `git` gives the checkout `dir`, by the rules above. Never throws: a bench
 * that has measured must not lose it to a failure in the stamping, so a failed commit
 * read is `BENCH_STAMP_UNKNOWN` and a failed dirty check alone `-dirty?`, never a clean
 * tree it could not see.
 *
 * @param git - git, with the environment its root chose
 * @param dir - the checkout git runs in
 * @param other_checkout - whether `dir` is another checkout than this repo
 */
export const git_bench_stamp = async (
	git: SpawnProgram,
	dir: FsReadDir,
	other_checkout = false
): Promise<string> => {
	const read = async (args: ReadonlyArray<string>): Promise<string | null> => {
		try {
			const result = await git.out(args, { cwd: dir });
			return result.kind === 'exited' && result.code === 0 && !result.truncated
				? result.stdout
				: null;
		} catch {
			return null;
		}
	};
	const commit = (await read(BENCH_COMMIT_ARGS))?.trim();
	if (!commit) return BENCH_STAMP_UNKNOWN;
	const status = await read(to_bench_status_args(other_checkout));
	return to_bench_stamp(
		commit,
		status === null ? null : bench_status_is_dirty(status, other_checkout)
	);
};

/**
 * A results document's text: tab-indented JSON with a trailing newline, the format every
 * `benches/*.json` is in.
 */
export const format_bench_json = (doc: unknown): string => JSON.stringify(doc, null, '\t') + '\n';

/**
 * The `gzip` arguments behind every recorded gzip figure: `gzip -c`, gzip's default level,
 * the byte count of what it writes being the figure.
 */
export const to_gzip_args = (path: string): Array<string> => ['-c', path];
