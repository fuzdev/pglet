/**
 * Writing a bump into the checkout: the version in `Cargo.toml`, the workspace
 * members' entries in both lockfiles, and the changelog's stamp. These are the
 * only files a release commit holds. Every write after the version is
 * idempotent, so a resumed release repeats them safely.
 *
 * @module
 */

import type { FsDir } from '@fuzdev/gro/fs.ts';

import { CHANGELOG_PATH, changelog_stamp, type ChangelogStampOutcome } from './changelog.ts';
import { release_capture, type ReleaseProgram, type ReleaseResult } from './programs.ts';
import { CARGO_PATH, cargo_workspace_version_set } from './version.ts';

/** The workspaces whose `Cargo.lock` records the bumped members: the root, and `fuzz/`, which path-depends on them. */
const LOCKFILES = [
	{ dir: '.', path: 'Cargo.lock' },
	{ dir: 'fuzz', path: 'fuzz/Cargo.lock' }
] as const;

/** Every file a bump writes, and so everything the release commit holds. */
export const RELEASE_PATHS: ReadonlyArray<string> = [
	CARGO_PATH,
	...LOCKFILES.map(({ path }) => path),
	CHANGELOG_PATH
];

/** The release commit's message, which a resume also reads to recognize its own commit. */
export const release_commit_message = (version: string): string => `chore: release v${version}`;

/** Writes `version` into `Cargo.toml`'s `[workspace.package]` table. */
export const bump_cargo_version = async (
	checkout: FsDir,
	version: string
): Promise<ReleaseResult> => {
	const next = cargo_workspace_version_set(await checkout.read_text(CARGO_PATH), version);
	if (next === null) return { ok: false, error: `no [workspace.package] version in ${CARGO_PATH}` };
	await checkout.write(CARGO_PATH, next);
	return { ok: true };
};

/**
 * Moves each lockfile's workspace members to the manifest's version. Offline: a
 * member's version is all that changes, and no registry crate may move with it.
 */
export const bump_lockfiles = async (
	cargo: ReleaseProgram,
	checkout: FsDir
): Promise<ReleaseResult> => {
	for (const { dir, path } of LOCKFILES) {
		const result = await release_capture(cargo, ['update', '--workspace', '--offline'], {
			cwd: checkout.dir(dir)
		});
		if (!result.success) {
			return { ok: false, error: `cargo update --workspace for ${path} failed:\n${result.err}` };
		}
	}
	return { ok: true };
};

/** Stamps the changelog's `## Unreleased` section as `version`, writing only when it changes. */
export const bump_changelog = async (
	checkout: FsDir,
	version: string
): Promise<ChangelogStampOutcome> => {
	const { outcome, text } = changelog_stamp(await checkout.read_text(CHANGELOG_PATH), version);
	if (outcome === 'stamped' || outcome === 'stamped_without_marker') {
		await checkout.write(CHANGELOG_PATH, text);
	}
	return outcome;
};
