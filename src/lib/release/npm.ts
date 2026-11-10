/**
 * The npm registry questions a release asks, each through the `npm` program
 * the caller hands in.
 *
 * @module
 */

import { release_capture, type ReleaseProgram, type ReleaseResult } from './programs.ts';

/** The logged-in npm user. */
export const npm_whoami = async (npm: ReleaseProgram): Promise<ReleaseResult<{ user: string }>> => {
	const who = await release_capture(npm, ['whoami']);
	if (who.success) return { ok: true, user: who.out };
	return { ok: false, error: who.err.split('\n')[0] ?? '' };
};

/**
 * Whether `name@version` is on the registry. `npm view` prints the version when
 * it is, nothing when the package exists without it, and E404 when the package
 * doesn't exist; anything else is a lookup failure, which no publish decision
 * may be made without.
 */
export const npm_is_published = async (
	npm: ReleaseProgram,
	name: string,
	version: string
): Promise<ReleaseResult<{ published: boolean }>> => {
	const view = await release_capture(npm, ['view', `${name}@${version}`, 'version']);
	if (view.success) return { ok: true, published: view.out === version };
	if (view.err.includes('E404')) return { ok: true, published: false };
	return { ok: false, error: `npm view ${name}@${version} failed:\n${view.err}` };
};

/**
 * Whether a failed `npm publish --dry-run` failed only because the version is
 * already on the registry, which a dry run of a release that hasn't bumped yet
 * always is.
 */
export const npm_error_is_republish = (stderr: string): boolean =>
	/cannot publish over the previously published versions?/i.test(stderr);
