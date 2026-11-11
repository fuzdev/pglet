/**
 * The dev server: `pglet_server` on its default port, rebuilt and restarted when a `.rs`
 * file beneath `crates/` changes. The loop is a function over its deps (the `crates/`
 * handle it watches, the checkout cargo runs in, the cargo program, the watch, a timer and
 * the output), so it's tested over fakes; `npm run server` is its root.
 *
 * @module
 */

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import type { SpawnProgram, SpawnResult } from '@fuzdev/gro/spawn.ts';
import { spawn_restartable, type SpawnRestartable } from '@fuzdev/gro/spawn_restartable.ts';
import type { Watcher, WatchDirOptions, WatchEvent } from '@fuzdev/gro/watch.ts';

import { CARGO_PASS_ENV } from './cargo_env.ts';

/** How long a rebuild waits after the last change that calls for it, in milliseconds. */
export const DEV_SERVER_DEBOUNCE_MS = 500;

/** The build that must pass before the server starts: the server and what it links. */
export const DEV_SERVER_BUILD_ARGS: ReadonlyArray<string> = ['build', '-p', 'pglet_server'];

/** The server, run through cargo on its default port, 5433. */
export const DEV_SERVER_RUN_ARGS: ReadonlyArray<string> = ['run', '-p', 'pglet_server'];

/**
 * The host variables the cargo program is given beside gro's base environment: cargo's own
 * (`CARGO_PASS_ENV`), `RUST_LIBC_UNSTABLE_*`, the one family a build script in the server's
 * graph reruns on that the list doesn't hold (`libc`'s), so the dev server's builds and the
 * npm scripts' share their output, and `RUST_BACKTRACE`, which the server reads when it
 * panics.
 */
export const DEV_SERVER_PASS_ENV: ReadonlyArray<string> = [
	...CARGO_PASS_ENV,
	'RUST_LIBC_UNSTABLE_*',
	'RUST_BACKTRACE'
];

/** What `run_dev_server` reaches. */
export interface DevServerDeps {
	/** The checkout, cargo's working directory. */
	checkout: FsReadDir;
	/** The checkout's `crates/`, watched recursively. */
	crates: FsReadDir;
	/** `cargo`, its output inherited, which builds and then runs the server. */
	cargo: SpawnProgram;
	/** `watch.dir` of gro's watch host module. */
	watch_dir: (dir: FsReadDir, options?: WatchDirOptions) => Promise<Watcher>;
	/** Calls `callback` once after `ms`, returning what cancels it. */
	set_timeout: (callback: () => void, ms: number) => () => void;
	print: (text: string) => void;
}

/** How the dev server runs. */
export interface DevServerOptions {
	/** How long to wait after the last change that calls for a rebuild. */
	debounce_ms?: number;
	/** Ends the dev server: the watch, any build, and the server. */
	signal?: AbortSignal;
}

/**
 * Whether a watch event calls for a rebuild: a `.rs` entry added, updated or deleted, or a
 * `rescan`, after which a change may have been missed. A directory's own events don't, so a
 * build output directory removed beneath `crates/` rebuilds nothing.
 */
export const is_dev_server_trigger = (event: WatchEvent): boolean =>
	event.kind === 'rescan' || (!event.is_directory && event.path.endsWith('.rs'));

const to_event_text = (event: WatchEvent): string =>
	event.kind === 'rescan'
		? `rescan of crates/${event.path}`
		: `${event.kind} of crates/${event.path}`;

const to_result_text = (result: SpawnResult): string => {
	switch (result.kind) {
		case 'exited':
			return `code ${result.code}`;
		case 'signaled':
			return `signal ${result.signal}`;
		case 'timed_out':
			return 'a timeout';
		case 'aborted':
			return 'an abort';
	}
};

/**
 * Builds the server, starts `pglet_server` through cargo once the build passes, and
 * watches `crates/`, rebuilding and restarting on each burst of `.rs` changes once
 * `debounce_ms` passes without another. A rebuild stops the server first, so a failed build
 * leaves no server running until a later one passes. A change during a build is kept: the
 * build finishes, then one more runs, however many changes arrived, and the server starts
 * only after a build no change overtook. A server that exits on its own is reported and
 * stays down until the next change.
 *
 * Resolves once `signal` aborts, after the watch has ended, any build has been stopped, and
 * the server has exited. Without a signal it runs until the process ends, where gro's spawn
 * module stops the children (cargo and the server run in the terminal's group, so Ctrl-C
 * reaches them too).
 *
 * @throws `FsError` - from the watch, when `crates/` can't be watched or goes away
 * @throws `SpawnError` - `launch_failed`, when cargo can't be started
 */
export const run_dev_server = async (
	deps: DevServerDeps,
	options: DevServerOptions = {}
): Promise<void> => {
	const { checkout, crates, cargo, watch_dir, set_timeout, print } = deps;
	const { debounce_ms = DEV_SERVER_DEBOUNCE_MS, signal } = options;

	// ends everything: the caller's signal, or a failure of the loop's own
	const ending = new AbortController();
	const end_signal = signal ? AbortSignal.any([signal, ending.signal]) : ending.signal;
	const ended = (): boolean => end_signal.aborted;

	let server: SpawnRestartable | null = null;
	let cancel_timer = null as (() => void) | null;
	// the first change of the burst the timer waits on, for the log line
	let reason: string | null = null;
	let cycle: Promise<void> | null = null;
	// a change arrived while a cycle was building, so it builds once more
	let pending = false;
	let failure = null as { error: unknown } | null;

	// reports an exit nobody asked for, which a restart or a stop sees as a replaced child or `active` false
	const report_exit = (restartable: SpawnRestartable): void => {
		const { child } = restartable;
		void restartable.closed.then((result) => {
			if (ended() || restartable.child !== child || !restartable.active) return;
			print(
				`[server] pglet_server exited with ${to_result_text(result)}; ` +
					'a .rs change rebuilds and starts it'
			);
		});
	};

	const stop_server = async (): Promise<void> => {
		if (server === null || !server.active) return;
		print('[server] stopping pglet_server');
		await server.stop();
	};

	const start_server = async (): Promise<void> => {
		print('[server] starting pglet_server on :5433');
		if (server === null) {
			server = await spawn_restartable(cargo, DEV_SERVER_RUN_ARGS, { cwd: checkout });
		} else {
			await server.restart();
		}
		report_exit(server);
	};

	const build = async (): Promise<boolean> => {
		print('[server] building');
		const result = await cargo.run(DEV_SERVER_BUILD_ARGS, { cwd: checkout, signal: end_signal });
		return result.kind === 'exited' && result.code === 0;
	};

	const run_cycle = async (): Promise<void> => {
		do {
			pending = false;
			await stop_server();
			if (ended()) return;
			const passed = await build();
			if (ended()) return;
			if (!passed) {
				print('[server] build failed, waiting for a .rs change');
			} else if (!pending) {
				await start_server();
			}
		} while (pending && !ended());
	};

	const request_cycle = (): void => {
		if (ended()) return;
		if (cycle !== null) {
			pending = true;
			return;
		}
		cycle = run_cycle()
			.catch((error: unknown) => {
				failure ??= { error };
				ending.abort();
			})
			.finally(() => {
				cycle = null;
			});
	};

	const schedule_cycle = (event: WatchEvent): void => {
		reason ??= to_event_text(event);
		cancel_timer?.();
		cancel_timer = set_timeout(() => {
			cancel_timer = null;
			print(`[server] ${reason}${cycle === null ? '' : ', rebuilding after this build'}`);
			reason = null;
			request_cycle();
		}, debounce_ms);
	};

	// watched before the first build, so a change during it is kept like any other
	const watcher = await watch_dir(crates, { recursive: true, signal: end_signal });
	try {
		print('[server] watching crates/ for .rs changes');
		request_cycle();
		for await (const event of watcher) {
			if (is_dev_server_trigger(event)) schedule_cycle(event);
		}
	} finally {
		ending.abort();
		cancel_timer?.();
		cancel_timer = null;
		pending = false;
		await watcher.close();
		await cycle;
		await stop_server();
	}
	if (failure !== null) throw failure.error;
};
