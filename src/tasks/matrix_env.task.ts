import * as s from '@fuzdev/fuz_schema/schema.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import type { Task } from '@fuzdev/gro/task.ts';

import {
	MATRIX_NPM_PACKAGE_DIR,
	MATRIX_RELEASE_DIR,
	run_matrix_env_command
} from '#lib/matrix_env.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({});
export type Args = s.Infer<typeof Args>;

/**
 * `npm run matrix:env`, the composition root: it hands `run_matrix_env_command`
 * this checkout's path, found from this module's own location, never the
 * working directory, and read-only openers for the two dirs whose artifacts it
 * checks, each followed through symlinks as `fs.open_dir` follows them. It
 * reads nothing else.
 */
export const task: Task<Args> = {
	summary: "print the fuz_app db matrix's setup for this checkout's build (npm run matrix:env)",
	Args,
	run: async () => {
		const checkout_path = (await fs.open_dir(to_task_checkout_path(import.meta.url))).path;
		await run_matrix_env_command({
			checkout_path,
			open_release_dir: async () =>
				(await fs.open_dir(`${checkout_path}/${MATRIX_RELEASE_DIR}`)).readonly(),
			open_npm_package_dir: async () =>
				(await fs.open_dir(`${checkout_path}/${MATRIX_NPM_PACKAGE_DIR}`)).readonly(),
			print: (text) => console.log(text),
			print_error: (text) => console.error(text)
		});
	}
};
