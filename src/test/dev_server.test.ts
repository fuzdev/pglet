/// <reference types="node" />

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import type {
	SpawnCallOptions,
	SpawnChild,
	SpawnProgram,
	SpawnResult,
	SpawnStartOptions
} from '@fuzdev/gro/spawn.ts';
import { SpawnError } from '@fuzdev/gro/spawn_error.ts';
import type { Watcher, WatchDirOptions, WatchEvent } from '@fuzdev/gro/watch.ts';
import { assert, describe, test } from 'vitest';

import {
	DEV_SERVER_DEBOUNCE_MS,
	DEV_SERVER_PASS_ENV,
	is_dev_server_trigger,
	run_dev_server
} from '#lib/dev_server.ts';

/** Lets every pending callback and promise chain run. */
const settle = async (): Promise<void> => {
	for (let i = 0; i < 5; i++) await new Promise((resolve) => setImmediate(resolve));
};

const handle = (path: string): FsReadDir => ({ path }) as unknown as FsReadDir;

const CHECKOUT = handle('/fake/checkout');
const CRATES = handle('/fake/checkout/crates');

const change = (
	kind: 'add' | 'update' | 'delete',
	path: string,
	is_directory = false
): WatchEvent => ({ kind, path, is_directory }) as WatchEvent;

/** A watch whose events the test pushes, ending when the signal it was given aborts. */
const to_fake_watch = () => {
	const queue: Array<WatchEvent> = [];
	let wake: (() => void) | null = null;
	let closed = false;
	let failure: Error | null = null;
	const calls: Array<{ dir: string; recursive: boolean | undefined }> = [];
	const notify = () => {
		wake?.();
		wake = null;
	};
	const watcher: Watcher = {
		async *[Symbol.asyncIterator]() {
			for (;;) {
				while (queue.length > 0) yield queue.shift()!;
				if (failure) throw failure;
				if (closed) return;
				await new Promise<void>((resolve) => {
					wake = resolve;
				});
			}
		},
		close: async () => {
			closed = true;
			notify();
		},
		[Symbol.asyncDispose]: async () => {
			closed = true;
			notify();
		}
	};
	return {
		calls,
		push: (...events: Array<WatchEvent>) => {
			queue.push(...events);
			notify();
		},
		fail: (error: Error) => {
			failure = error;
			notify();
		},
		get closed() {
			return closed;
		},
		watch_dir: async (dir: FsReadDir, options?: WatchDirOptions): Promise<Watcher> => {
			calls.push({ dir: dir.path, recursive: options?.recursive });
			options?.signal?.addEventListener('abort', () => void watcher.close(), { once: true });
			return watcher;
		}
	};
};

/** Timers the test fires by hand, all at once or by moving a clock of its own. */
const to_fake_timers = () => {
	const timers: Array<{ callback: () => void; ms: number; due: number; cancelled: boolean }> = [];
	let now = 0;
	return {
		timers,
		/** The timers not yet fired or cancelled. */
		live: () => timers.filter((t) => !t.cancelled),
		/** Fires every live timer. */
		fire: () => {
			for (const t of timers) {
				if (t.cancelled) continue;
				t.cancelled = true;
				t.callback();
			}
		},
		/** Moves the clock on by `ms`, firing each live timer it reaches. */
		advance: (ms: number) => {
			now += ms;
			for (const t of timers) {
				if (t.cancelled || t.due > now) continue;
				t.cancelled = true;
				t.callback();
			}
		},
		set_timeout: (callback: () => void, ms: number) => {
			const timer = { callback, ms, due: now + ms, cancelled: false };
			timers.push(timer);
			return () => {
				timer.cancelled = true;
			};
		}
	};
};

interface FakeServer {
	args: ReadonlyArray<string>;
	cwd: string | undefined;
	/** Whether it was stopped by its owner. */
	killed: boolean;
	/** Whether it has ended, stopped or on its own. */
	ended: boolean;
	/** Ends it on its own with `result`. */
	exit: (result: SpawnResult) => void;
	closed: Promise<SpawnResult>;
}

/**
 * A cargo whose builds the test ends one at a time (`finish_build`), and whose
 * runs are fake servers that end when they're stopped or told to exit.
 */
const to_fake_cargo = (launch_error: Error | null = null) => {
	const builds: Array<{
		args: ReadonlyArray<string>;
		cwd: string | undefined;
		aborted: boolean;
		finish: (code: number) => void;
	}> = [];
	const servers: Array<FakeServer> = [];
	const unreached = () => {
		throw new Error('the dev server only runs and starts cargo');
	};
	const program: SpawnProgram = {
		name: 'cargo',
		path: '/fake/cargo',
		run: (args: ReadonlyArray<string>, options?: SpawnCallOptions) =>
			new Promise<SpawnResult>((resolve, reject) => {
				if (launch_error) {
					reject(launch_error);
					return;
				}
				const build = {
					args,
					cwd: options?.cwd?.path,
					aborted: false,
					finish: (code: number) => resolve({ kind: 'exited', code })
				};
				options?.signal?.addEventListener(
					'abort',
					() => {
						build.aborted = true;
						resolve({ kind: 'aborted', code: null, signal: 'SIGTERM' } as SpawnResult);
					},
					{ once: true }
				);
				builds.push(build);
			}),
		out: unreached,
		out_bytes: unreached,
		start: async (args: ReadonlyArray<string>, options?: SpawnStartOptions) => {
			let resolve_closed!: (result: SpawnResult) => void;
			const closed = new Promise<SpawnResult>((resolve) => {
				resolve_closed = resolve;
			});
			const end = (result: SpawnResult) => {
				server.ended = true;
				resolve_closed(result);
			};
			const server: FakeServer = {
				args,
				cwd: options?.cwd?.path,
				killed: false,
				ended: false,
				exit: (result) => end(result),
				closed
			};
			servers.push(server);
			const child: SpawnChild = {
				pid: 1000 + servers.length,
				closed,
				stdout: null,
				stderr: null,
				stdin: null,
				kill: () => {
					server.killed = true;
					end({ kind: 'signaled', signal: 'SIGTERM' });
					return true;
				},
				[Symbol.asyncDispose]: async () => {
					server.killed = true;
					end({ kind: 'signaled', signal: 'SIGTERM' });
					await closed;
				}
			};
			return child;
		}
	};
	return {
		program,
		builds,
		servers,
		/** Ends the build in flight, the newest one, with `code`. */
		finish_build: async (code = 0) => {
			const build = builds.at(-1);
			assert.ok(build, 'a build is running');
			build.finish(code);
			await settle();
		}
	};
};

/**
 * Starts the dev server over fakes, its cargo failing to launch with `launch_error` when
 * given; `stop` aborts it and waits for it to resolve.
 */
const start = async (launch_error: Error | null = null) => {
	const watch = to_fake_watch();
	const timers = to_fake_timers();
	const cargo = to_fake_cargo(launch_error);
	const printed: Array<string> = [];
	const controller = new AbortController();
	const done = run_dev_server(
		{
			checkout: CHECKOUT,
			crates: CRATES,
			cargo: cargo.program,
			watch_dir: watch.watch_dir,
			set_timeout: timers.set_timeout,
			print: (text) => printed.push(text)
		},
		{ signal: controller.signal }
	);
	// a rejection the test awaits later isn't unhandled while `settle` runs
	done.catch(() => {});
	await settle();
	return {
		watch,
		timers,
		cargo,
		printed,
		done,
		/** Pushes `events` and lets the loop read them. */
		push: async (...events: Array<WatchEvent>) => {
			watch.push(...events);
			await settle();
		},
		/** Fires the debounce timer and lets the loop act on it. */
		fire: async () => {
			timers.fire();
			await settle();
		},
		/** Moves the timers' clock on by `ms` and lets the loop act on what fired. */
		advance: async (ms: number) => {
			timers.advance(ms);
			await settle();
		},
		stop: async () => {
			controller.abort();
			await done;
		}
	};
};

/** The number of builds cargo was asked for. */
const build_count = (dev: Awaited<ReturnType<typeof start>>): number => dev.cargo.builds.length;

/** How many servers are up: started and neither stopped nor ended on their own. */
const servers_up = (dev: Awaited<ReturnType<typeof start>>): number =>
	dev.cargo.servers.filter((server) => !server.ended).length;

describe('run_dev_server', () => {
	test('watches crates/ recursively, builds the server, then runs it from the checkout', async () => {
		const dev = await start();
		try {
			assert.deepEqual(dev.watch.calls, [{ dir: CRATES.path, recursive: true }]);
			assert.strictEqual(build_count(dev), 1, 'the first build runs with no event');
			assert.deepEqual(dev.cargo.builds[0]!.args, ['build', '-p', 'pglet_server']);
			assert.strictEqual(dev.cargo.builds[0]!.cwd, CHECKOUT.path);
			assert.strictEqual(dev.cargo.servers.length, 0, 'no server before the build passes');
			await dev.cargo.finish_build(0);
			assert.strictEqual(dev.cargo.servers.length, 1);
			assert.deepEqual(dev.cargo.servers[0]!.args, ['run', '-p', 'pglet_server']);
			assert.strictEqual(dev.cargo.servers[0]!.cwd, CHECKOUT.path);
		} finally {
			await dev.stop();
		}
	});

	test('a burst of changes rebuilds once, after the debounce', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push(
				change('update', 'pglet/src/lib.rs'),
				change('add', 'pglet_sql/src/new.rs'),
				change('update', 'pglet/src/lib.rs')
			);
			assert.strictEqual(build_count(dev), 1, 'nothing rebuilds before the debounce');
			assert.strictEqual(dev.timers.live().length, 1, 'each change restarts the one timer');
			assert.strictEqual(dev.timers.live()[0]!.ms, DEV_SERVER_DEBOUNCE_MS);
			await dev.fire();
			assert.strictEqual(build_count(dev), 2);
			assert.isTrue(dev.cargo.servers[0]!.killed, 'the server stops before the rebuild');
			await dev.cargo.finish_build(0);
			assert.strictEqual(dev.cargo.servers.length, 2);
			assert.strictEqual(servers_up(dev), 1);
			assert.ok(dev.printed.includes('[server] update of crates/pglet/src/lib.rs'));
		} finally {
			await dev.stop();
		}
	});

	test('each change restarts the wait, so a burst builds once, a debounce after its last change', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			const step = DEV_SERVER_DEBOUNCE_MS * 0.8;
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.advance(step);
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.advance(step);
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.advance(step);
			assert.strictEqual(build_count(dev), 1, 'no build while the changes keep coming');
			await dev.advance(DEV_SERVER_DEBOUNCE_MS - step);
			assert.strictEqual(build_count(dev), 2, 'one build, a debounce after the last change');
			await dev.cargo.finish_build(0);
			await dev.advance(DEV_SERVER_DEBOUNCE_MS * 10);
			assert.strictEqual(build_count(dev), 2);
		} finally {
			await dev.stop();
		}
	});

	test('an editor save by rename, a delete then an add of one file, rebuilds once', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push(
				change('add', 'pglet/src/.lib.rs.swp'),
				change('delete', 'pglet/src/lib.rs'),
				change('add', 'pglet/src/lib.rs'),
				change('delete', 'pglet/src/.lib.rs.swp')
			);
			await dev.fire();
			assert.strictEqual(build_count(dev), 2);
			await dev.cargo.finish_build(0);
			assert.strictEqual(build_count(dev), 2);
		} finally {
			await dev.stop();
		}
	});

	test('a change during a build reruns it once after it finishes, however many arrive', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.fire();
			assert.strictEqual(build_count(dev), 2);
			// mid-build: two debounced bursts
			await dev.push(change('update', 'pglet/src/a.rs'));
			await dev.fire();
			await dev.push(change('update', 'pglet/src/b.rs'));
			await dev.fire();
			assert.strictEqual(build_count(dev), 2, 'no second build while one runs');
			await dev.cargo.finish_build(0);
			assert.strictEqual(build_count(dev), 3, 'exactly one more build');
			assert.strictEqual(dev.cargo.servers.length, 1, 'no server from the overtaken build');
			await dev.cargo.finish_build(0);
			assert.strictEqual(build_count(dev), 3);
			assert.strictEqual(dev.cargo.servers.length, 2);
			assert.strictEqual(servers_up(dev), 1);
		} finally {
			await dev.stop();
		}
	});

	test('a change during the first build is kept', async () => {
		const dev = await start();
		try {
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.fire();
			await dev.cargo.finish_build(0);
			assert.strictEqual(build_count(dev), 2);
			await dev.cargo.finish_build(0);
			assert.strictEqual(dev.cargo.servers.length, 1);
		} finally {
			await dev.stop();
		}
	});

	test('deleting a .rs file rebuilds', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push(change('delete', 'pglet_sql/src/gone.rs'));
			await dev.fire();
			assert.strictEqual(build_count(dev), 2);
		} finally {
			await dev.stop();
		}
	});

	test('a rescan rebuilds', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push({ kind: 'rescan', path: '' } as WatchEvent);
			await dev.fire();
			assert.strictEqual(build_count(dev), 2);
		} finally {
			await dev.stop();
		}
	});

	test('a change to anything but a .rs file rebuilds nothing', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push(
				change('update', 'pglet/Cargo.toml'),
				change('add', 'pglet_wasm/pkg/pglet_wasm.js'),
				change('delete', 'pglet_wasm/pkg', true),
				change('add', 'pglet/src/lib.rs.orig'),
				change('add', 'pglet/src/new_module', true)
			);
			assert.strictEqual(dev.timers.timers.length, 0);
			await dev.fire();
			assert.strictEqual(build_count(dev), 1);
		} finally {
			await dev.stop();
		}
	});

	test('a failed build leaves no server running until a build passes', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.fire();
			await dev.cargo.finish_build(101);
			assert.strictEqual(dev.cargo.servers.length, 1, 'no server after a failed build');
			assert.strictEqual(servers_up(dev), 0, 'the last good server was stopped');
			assert.ok(dev.printed.includes('[server] build failed, waiting for a .rs change'));
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.fire();
			await dev.cargo.finish_build(0);
			assert.strictEqual(dev.cargo.servers.length, 2);
			assert.strictEqual(servers_up(dev), 1);
		} finally {
			await dev.stop();
		}
	});

	test('a failed first build starts no server', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(101);
			assert.strictEqual(dev.cargo.servers.length, 0);
		} finally {
			await dev.stop();
		}
	});

	test('a server that exits on its own is reported, not restarted', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			dev.cargo.servers[0]!.exit({ kind: 'exited', code: 1 });
			await settle();
			assert.ok(
				dev.printed.includes(
					'[server] pglet_server exited with code 1; a .rs change rebuilds and starts it'
				),
				dev.printed.join('\n')
			);
			assert.strictEqual(build_count(dev), 1);
			assert.strictEqual(dev.cargo.servers.length, 1);
			assert.strictEqual(dev.timers.timers.length, 0);
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.fire();
			await dev.cargo.finish_build(0);
			assert.strictEqual(dev.cargo.servers.length, 2);
		} finally {
			await dev.stop();
		}
	});

	test('a server stopped for a rebuild is not reported as exited', async () => {
		const dev = await start();
		try {
			await dev.cargo.finish_build(0);
			await dev.push(change('update', 'pglet/src/lib.rs'));
			await dev.fire();
			await dev.cargo.finish_build(0);
			await dev.stop();
			assert.isFalse(dev.printed.some((line) => line.includes('exited')));
		} finally {
			await dev.stop();
		}
	});

	test('stopping ends the watch and stops the server', async () => {
		const dev = await start();
		await dev.cargo.finish_build(0);
		await dev.stop();
		assert.isTrue(dev.watch.closed);
		assert.isTrue(dev.cargo.servers[0]!.killed);
		assert.strictEqual(servers_up(dev), 0);
	});

	test('stopping mid-build aborts the build, cancels a pending rebuild, and starts nothing', async () => {
		const dev = await start();
		await dev.push(change('update', 'pglet/src/lib.rs'));
		assert.strictEqual(dev.timers.live().length, 1);
		await dev.stop();
		assert.isTrue(dev.cargo.builds[0]!.aborted);
		assert.strictEqual(dev.timers.live().length, 0);
		assert.strictEqual(build_count(dev), 1);
		assert.strictEqual(dev.cargo.servers.length, 0);
	});

	test('cargo failing to launch ends the watch and rejects with its error', async () => {
		const error = new SpawnError('launch_failed', 'cargo', 'cargo: not found');
		const dev = await start(error);
		try {
			await dev.done;
			assert.fail('expected a rejection');
		} catch (caught) {
			assert.strictEqual(caught, error);
		}
		assert.isTrue(dev.watch.closed);
		assert.strictEqual(dev.cargo.servers.length, 0);
	});

	test('a watch that fails stops the server and rejects with its error', async () => {
		const dev = await start();
		await dev.cargo.finish_build(0);
		const error = new Error('crates/ is gone');
		dev.watch.fail(error);
		try {
			await dev.done;
			assert.fail('expected a rejection');
		} catch (caught) {
			assert.strictEqual(caught, error);
		}
		assert.isTrue(dev.cargo.servers[0]!.killed);
	});
});

describe('is_dev_server_trigger', () => {
	test('a .rs file added, updated or deleted, and a rescan', () => {
		assert.isTrue(is_dev_server_trigger(change('add', 'a/src/x.rs')));
		assert.isTrue(is_dev_server_trigger(change('update', 'a/src/x.rs')));
		assert.isTrue(is_dev_server_trigger(change('delete', 'a/src/x.rs')));
		assert.isTrue(is_dev_server_trigger({ kind: 'rescan', path: 'a' } as WatchEvent));
		assert.isFalse(is_dev_server_trigger(change('update', 'a/Cargo.toml')));
		assert.isFalse(is_dev_server_trigger(change('add', 'a/src/x.rs~')));
		assert.isFalse(is_dev_server_trigger(change('delete', 'a/src', true)));
	});
});

describe('DEV_SERVER_PASS_ENV', () => {
	test("passes cargo's variables, libc's build script's and the server's backtrace switch, not the whole environment", () => {
		for (const name of [
			'CARGO_*',
			'RUSTUP_*',
			'RUSTFLAGS',
			'RUSTC_WRAPPER',
			'RUST_LIBC_UNSTABLE_*',
			'RUST_BACKTRACE'
		]) {
			assert.include(DEV_SERVER_PASS_ENV, name);
		}
		assert.notInclude(DEV_SERVER_PASS_ENV, '*');
	});
});
