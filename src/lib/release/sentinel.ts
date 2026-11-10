/**
 * The resume sentinel: a gitignored file naming the version a wetrun bumped to,
 * written right after the bump and removed only once the release commit, tag
 * and push have landed. While it names the checkout's version, a wetrun resumes
 * that release instead of starting a new one.
 *
 * @module
 */

import type { FsDir, FsReadDir } from '@fuzdev/gro/fs.ts';

/** The sentinel's path in the checkout. */
export const SENTINEL_PATH = '.publish-in-progress';

/** The version the sentinel names, or null when there is none. */
export const sentinel_read = async (checkout: FsReadDir): Promise<string | null> =>
	(await checkout.read_text_if_exists(SENTINEL_PATH))?.trim() ?? null;

/** Writes the sentinel naming `version`. */
export const sentinel_write = (checkout: FsDir, version: string): Promise<void> =>
	checkout.write(SENTINEL_PATH, `${version}\n`);

/** Removes the sentinel; a missing one isn't an error. */
export const sentinel_remove = (checkout: FsDir): Promise<void> =>
	checkout.remove(SENTINEL_PATH, { missing_ok: true });
