/**
 * The git questions and steps a release asks of the checkout, all against
 * `origin` and its `main`, the one branch a release publishes from. Each takes
 * the `git` program as its first argument, so the caller decides its
 * environment: the task root hands the local reads a `git` with git's own
 * variables, and the fetch, the remote lookup, the commit, the tag and the push
 * a `git` with the user's whole environment, for their credentials, signing and
 * hooks.
 *
 * @module
 */

import {
	release_capture,
	release_stream,
	type ReleaseProgram,
	type ReleaseResult
} from './programs.ts';

/** The working tree's `git status --porcelain` lines. */
export const git_status_lines = async (
	git: ReleaseProgram
): Promise<ReleaseResult<{ lines: Array<string> }>> => {
	const status = await release_capture(git, ['status', '--porcelain']);
	if (!status.success) return { ok: false, error: `git status failed: ${status.err}` };
	return { ok: true, lines: status.out ? status.out.split('\n') : [] };
};

/** The path a porcelain status line names (the destination of a rename). */
export const git_status_line_path = (line: string): string =>
	line.slice(3).split(' -> ').at(-1) ?? '';

/**
 * A digest of the checkout: `HEAD`, the status lines (naming every untracked
 * file), and the content of every tracked change against `HEAD`. Two equal
 * fingerprints mean the tracked checkout did not move between them, including a
 * further edit to a file that was already modified, which the status lines
 * alone can't see; an untracked file's edit isn't seen.
 */
export const git_checkout_fingerprint = async (
	git: ReleaseProgram
): Promise<ReleaseResult<{ fingerprint: string }>> => {
	const parts = await Promise.all([
		release_capture(git, ['rev-parse', 'HEAD']),
		release_capture(git, ['status', '--porcelain', '--untracked-files=all']),
		release_capture(git, ['diff', '--binary', 'HEAD'])
	]);
	const failed = parts.find((part) => !part.success);
	if (failed) return { ok: false, error: `could not read the checkout: ${failed.err}` };
	const bytes = new TextEncoder().encode(parts.map((part) => part.out).join('\0'));
	const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
	return {
		ok: true,
		fingerprint: Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('')
	};
};

/** The checked-out branch, `''` when `HEAD` is detached, or null when git fails. */
export const git_current_branch = async (git: ReleaseProgram): Promise<string | null> => {
	const branch = await release_capture(git, ['branch', '--show-current']);
	return branch.success ? branch.out : null;
};

/** The commit `rev` names, or null when it names none. */
export const git_rev = async (git: ReleaseProgram, rev: string): Promise<string | null> => {
	const result = await release_capture(git, ['rev-parse', '-q', '--verify', `${rev}^{commit}`]);
	return result.success ? result.out : null;
};

/** The subject line of `HEAD`'s commit, or null when git fails. */
export const git_head_subject = async (git: ReleaseProgram): Promise<string | null> => {
	const result = await release_capture(git, ['log', '-1', '--format=%s']);
	return result.success ? result.out : null;
};

/**
 * Fetches `origin/main` with `git_remote`, then counts with `git_local` the
 * commits `HEAD` is ahead of it and behind it.
 */
export const git_fetch_and_compare_origin_main = async (
	git_remote: ReleaseProgram,
	git_local: ReleaseProgram
): Promise<ReleaseResult<{ ahead: number; behind: number }>> => {
	const fetched = await release_capture(git_remote, ['fetch', 'origin', 'main']);
	if (!fetched.success) return { ok: false, error: `git fetch origin main failed: ${fetched.err}` };
	const counts = await release_capture(git_local, [
		'rev-list',
		'--left-right',
		'--count',
		'origin/main...HEAD'
	]);
	const failed = {
		ok: false,
		error: `could not compare HEAD with origin/main: ${counts.err}`
	} as const;
	if (!counts.success) return failed;
	const [behind, ahead] = counts.out.split(/\s+/).map(Number);
	if (behind === undefined || ahead === undefined || Number.isNaN(behind) || Number.isNaN(ahead)) {
		return failed;
	}
	return { ok: true, ahead, behind };
};

/** The commit a local tag points at, or null when there is no such tag. */
export const git_local_tag_commit = (git: ReleaseProgram, tag: string): Promise<string | null> =>
	git_rev(git, `refs/tags/${tag}`);

/**
 * The commit a tag on `origin` points at, peeled so an annotated tag names its
 * commit; `commit` is null when `origin` has no such tag.
 */
export const git_remote_tag_commit = async (
	git: ReleaseProgram,
	tag: string
): Promise<ReleaseResult<{ commit: string | null }>> => {
	const ref = `refs/tags/${tag}`;
	const result = await release_capture(git, ['ls-remote', '--tags', 'origin', ref, `${ref}^{}`]);
	if (!result.success) return { ok: false, error: `git ls-remote origin failed: ${result.err}` };
	const rows = result.out ? result.out.split('\n').map((row) => row.split('\t')) : [];
	// an annotated tag lists its own object under `ref` and its commit under `ref^{}`
	const row =
		rows.find(([, name]) => name === `${ref}^{}`) ?? rows.find(([, name]) => name === ref);
	return { ok: true, commit: row?.[0] ?? null };
};

/**
 * Commits exactly `paths`, which must be tracked, with `message` when any of
 * them has changes; `committed` says whether a commit was made. A pathspec
 * commit takes those paths' working-tree content and nothing else, so a stray
 * change can't ride into it. `git_local` asks whether they changed and
 * `git_remote` commits, since a commit may sign and run hooks.
 */
export const git_commit_paths = async (
	git_remote: ReleaseProgram,
	git_local: ReleaseProgram,
	paths: ReadonlyArray<string>,
	message: string
): Promise<ReleaseResult<{ committed: boolean }>> => {
	const status = await release_capture(git_local, ['status', '--porcelain', '--', ...paths]);
	if (!status.success) return { ok: false, error: `git status failed: ${status.err}` };
	if (!status.out) return { ok: true, committed: false };
	const committed = await release_capture(git_remote, ['commit', '-m', message, '--', ...paths]);
	if (!committed.success) {
		return { ok: false, error: `git commit failed: ${committed.err || committed.out}` };
	}
	return { ok: true, committed: true };
};

/** Creates the annotated tag `tag` on `HEAD`; annotated, because `git push --follow-tags` skips a lightweight tag. */
export const git_tag_annotated = async (
	git: ReleaseProgram,
	tag: string
): Promise<ReleaseResult> => {
	const result = await release_capture(git, ['tag', '-m', tag, tag]);
	return result.success ? { ok: true } : { ok: false, error: `git tag failed: ${result.err}` };
};

/** Pushes `main` and the annotated tags it reaches to `origin`, streaming git's output. */
export const git_push_main_with_tags = async (
	git: ReleaseProgram,
	print_error: (text: string) => void
): Promise<boolean> =>
	(await release_stream(git, ['push', '--follow-tags', 'origin', 'main'], print_error)).success;
