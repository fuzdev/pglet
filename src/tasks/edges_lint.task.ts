import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import type { Task } from '@fuzdev/gro/task.ts';

import { run_edges_lint_command } from '#lib/edges_lint.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), {
			description: 'the ledger to lint, beneath the checkout, EDGES.md when none is named'
		}),
		() => []
	)
});
export type Args = s.Infer<typeof Args>;

/**
 * `npm run lint:edges`, the composition root: it hands `run_edges_lint_command`
 * this checkout read-only, found from this module's own location, never the
 * working directory. It reads the ledger and nothing else.
 */
export const task: Task<Args> = {
	summary: "check EDGES.md's structural invariants (npm run lint:edges)",
	Args,
	run: async ({ args }) => {
		const checkout = await fs.open_dir(to_task_checkout_path(import.meta.url));
		await run_edges_lint_command(
			{
				checkout: checkout.readonly(),
				print: (text) => console.log(text),
				print_error: (text) => console.error(text)
			},
			args
		);
	}
};
