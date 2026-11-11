import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { is_fs_error } from '@fuzdev/gro/fs_error.ts';
import { fs, type FsReadDir } from '@fuzdev/gro/fs.ts';
import { GIT_LOCAL_PASS_ENV } from '@fuzdev/gro/git.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
import type { Task } from '@fuzdev/gro/task.ts';

import { run_size_command, SIZE_NATIVE_DIR, SIZE_WASM_DEFAULT } from '#lib/size_gate.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({
	check: s.with_default(
		meta(s.boolean, { description: 'compare to the baseline, the default' }),
		false
	),
	update: s.with_default(
		meta(s.boolean, { description: 'rewrite the baseline from this build' }),
		false
	),
	wasm: s.with_default(
		meta(s.string, { description: 'the wasm to gate, relative to the checkout' }),
		SIZE_WASM_DEFAULT
	)
});
export type Args = s.Infer<typeof Args>;

/**
 * `npm run size:check` and `npm run size:update`, the composition root: it hands
 * `run_size_command` this checkout, found from this module's own location, never the
 * working directory. Its reach: the checkout read-only (the wasm artifact), `benches/`
 * (the baseline, written only by an update), the release target directory read-only,
 * followed through a symlink as `fs.open_dir` follows one, `gzip` with gro's base
 * environment, and `git` with git's own variables (`GIT_LOCAL_PASS_ENV`) for an
 * update's stamp.
 */
export const task: Task<Args> = {
	summary: 'gate the raw wasm size against benches/size_baseline.json (npm run size:check)',
	Args,
	run: async ({ args }) => {
		const checkout = await fs.open_dir(to_task_checkout_path(import.meta.url));
		let release: FsReadDir | null = null;
		try {
			release = (await fs.open_dir(`${checkout.path}/${SIZE_NATIVE_DIR}`)).readonly();
		} catch (error) {
			if (!is_fs_error(error, 'not_found')) throw error;
		}
		await run_size_command(
			{
				checkout: checkout.readonly(),
				benches: checkout.dir('benches'),
				release,
				gzip: spawn.program('gzip'),
				git: spawn.find('git', { pass_env: GIT_LOCAL_PASS_ENV }),
				now: () => new Date(),
				print: (text) => console.log(text),
				print_error: (text) => console.error(text)
			},
			{ mode: args.update ? 'update' : 'check', wasm: args.wasm }
		);
	}
};
