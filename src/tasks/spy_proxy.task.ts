import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import { TaskError, type Task } from '@fuzdev/gro/task.ts';

import {
	connect_loopback,
	listen_loopback,
	open_capture_file,
	SPY_LOOPBACK
} from '#lib/spy_proxy/node_net.ts';
import { parse_spy_proxy_args, run_spy_proxy } from '#lib/spy_proxy/proxy.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), {
			description:
				'[listen_port] [server_port] [output]: 5434 (0 for any free port, printed), 5432, tests/captures/session.jsonl'
		}),
		() => []
	)
});
export type Args = s.Infer<typeof Args>;

/**
 * `npm run spy:start`, the composition root: it hands `run_spy_proxy` the
 * Node-only sockets and the capture file. Its reach:
 *
 * - a listener on `127.0.0.1` at the port it's given, and connections to the
 *   server it spies on at `127.0.0.1:<server_port>`, never another host
 * - one file, appended to, inside this checkout's `tests/captures/` (the
 *   checkout found from this module's own location, never the working directory)
 *
 * It doesn't read `PGLET_PG_URL`: the server is the second positional argument.
 */
export const task: Task<Args> = {
	summary:
		'log the Postgres wire protocol between a client and a server on loopback (npm run spy:start -- 5434 5432)',
	Args,
	run: async ({ args }) => {
		const parsed = parse_spy_proxy_args(args._);
		if (!parsed.ok) throw new TaskError(parsed.error);
		const { listen_port, server_port, output } = parsed.args;
		const checkout = await fs.open_dir(to_task_checkout_path(import.meta.url));
		const capture = await open_capture_file(`${checkout.path}/${output}`);
		await run_spy_proxy(
			{
				listen: listen_loopback,
				connect: () => connect_loopback(server_port),
				append: capture.append,
				now: () => new Date().toISOString(),
				print: (text) => console.log(text),
				print_error: (text) => console.error(text)
			},
			{ listen_port, server_host: SPY_LOOPBACK, server_port, output }
		);
	}
};
