import * as s from '@fuzdev/fuz_schema/schema.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import type { Task } from '@fuzdev/gro/task.ts';

import { NPM_PACKAGE_DIR, NPM_PACKAGE_SOURCE_DIR, patch_npm_package } from '#lib/npm_package.ts';
import { NPM_SHARED_METADATA } from '#lib/npm_metadata.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({});
export type Args = s.Infer<typeof Args>;

/**
 * The last step of `npm run build:npm`, the composition root: it hands
 * `patch_npm_package` this checkout's handles. Its reach, beneath the checkout
 * found from this module's own location: `crates/pglet_wasm/pkg-npm/`, written
 * in place, and read-only the root's `LICENSE` and `crates/pglet_wasm/npm/`.
 * It patches `pkg-npm` and no other dir, since patching the plain `wasm:build`
 * output (`pkg`) would rewrite the build `size:check` and `test:wasm` read.
 */
export const task: Task<Args> = {
	summary: 'assemble @fuzdev/pglet-wasm in crates/pglet_wasm/pkg-npm from wasm-pack output',
	Args,
	run: async () => {
		const checkout = await fs.open_dir(to_task_checkout_path(import.meta.url));
		await patch_npm_package({
			pkg: checkout.dir(NPM_PACKAGE_DIR),
			npm_src: checkout.readonly().dir(NPM_PACKAGE_SOURCE_DIR),
			root: checkout.readonly()
		});
		console.log(`patched ${NPM_PACKAGE_DIR} → ${NPM_SHARED_METADATA.name}`);
	}
};
