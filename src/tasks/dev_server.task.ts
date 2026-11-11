import * as s from '@fuzdev/fuz_schema/schema.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
import type { Task } from '@fuzdev/gro/task.ts';
import { watch } from '@fuzdev/gro/watch.ts';

import { DEV_SERVER_PASS_ENV, run_dev_server } from '#lib/dev_server.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({});
export type Args = s.Infer<typeof Args>;

/**
 * `npm run server`, the composition root: it hands `run_dev_server` this checkout, found
 * from this module's own location, never the working directory. Its reach: the checkout
 * read-only (cargo's working directory), its `crates/` watched read-only, and `cargo` with
 * cargo's variables (`DEV_SERVER_PASS_ENV`). It runs until the process is stopped.
 */
export const task: Task<Args> = {
	summary: 'run pglet_server on :5433, rebuilt and restarted on a .rs change (npm run server)',
	Args,
	run: async () => {
		const checkout = (await fs.open_dir(to_task_checkout_path(import.meta.url))).readonly();
		await run_dev_server({
			checkout,
			crates: checkout.dir('crates'),
			cargo: spawn.program('cargo', { pass_env: DEV_SERVER_PASS_ENV }),
			watch_dir: (dir, options) => watch.dir(dir, options),
			set_timeout: (callback, ms) => {
				const timer = setTimeout(callback, ms);
				return () => clearTimeout(timer);
			},
			print: (text) => console.log(text)
		});
	}
};
