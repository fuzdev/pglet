import * as s from '@fuzdev/fuz_schema/schema.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import { TaskError, type Task } from '@fuzdev/gro/task.ts';

import type { StaticMount } from '#lib/static_files.ts';
import { start_static_server, StaticServerPortInUseError } from '#lib/static_server.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({});
export type Args = s.Infer<typeof Args>;

/** The port a person opens: fixed, so the example's URL is stable. */
const WASM_SERVE_PORT = 8001;

/** The directories the example pages load, each served under its own path. */
const WASM_SERVE_MOUNT_DIRS: ReadonlyArray<string> = [
	'examples/browser',
	'crates/pglet_wasm/pkg',
	'crates/pglet_wasm/pkg-opfs'
];

/**
 * `npm run wasm:serve`, the composition root: serves `examples/browser/` and the two
 * wasm builds it loads from this checkout, found from this module's own location, never
 * the working directory, on `127.0.0.1:8001` until the process is stopped. A port in use
 * fails the task with one line naming the port, and no stack: the port is the person's to
 * free.
 */
export const task: Task<Args> = {
	summary: 'serve examples/browser and the wasm builds on 127.0.0.1:8001 (npm run wasm:serve)',
	Args,
	run: async () => {
		const checkout = (await fs.open_dir(to_task_checkout_path(import.meta.url))).readonly();
		const mounts: Array<StaticMount> = WASM_SERVE_MOUNT_DIRS.map((prefix) => ({
			prefix,
			open: () => checkout.dir(prefix)
		}));
		const server = await start_static_server(mounts, {
			port: WASM_SERVE_PORT,
			log: (method, target, status) => console.log(`${method} ${target} ${status}`)
		}).catch((error: unknown) => {
			if (error instanceof StaticServerPortInUseError) throw new TaskError(error.message);
			throw error;
		});
		console.log(`Serving on ${server.origin}`);
		console.log(`Open ${server.origin}/examples/browser/ in your browser`);
		// the listening socket keeps the process alive until it is stopped
	}
};
