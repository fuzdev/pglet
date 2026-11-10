/**
 * A `pglet_server` for one test file: started on a port the kernel picks, its
 * port read from the line the server prints once it's bound, and stopped when
 * the caller is done, or when it fails to start. The program is a dep, so the
 * helper is tested over fake servers; the suites under `tests-web/` hand in the
 * debug build.
 *
 * Port 0 is what lets suites run in parallel: no two servers ask for one port,
 * and nothing that already listens on a fixed one (a spy proxy, another
 * checkout's server) can be mistaken for the server a suite started.
 *
 * @module
 */

import type { SpawnProgram, SpawnResult } from '@fuzdev/gro/spawn.ts';

/** What the server prints once it's bound, followed by the address it's bound to. */
export const PGLET_LISTENING_PREFIX = 'pglet listening on ';

/** The one address the server binds, and so the one the helper accepts. */
export const PGLET_SERVER_HOST = '127.0.0.1';

/** How long a start waits for the listening line by default, in milliseconds. */
export const PGLET_READY_TIMEOUT_MS = 30_000;

/** The most of the server's stderr, in characters, a failed start reports. */
const STDERR_TAIL_CHARS = 4096;

/** The most stdout lines before the listening line a failed start reports. */
const STDOUT_HEAD_LINES = 20;

/** A running server. */
export interface PgletServer extends AsyncDisposable {
	/** Always `PGLET_SERVER_HOST`. */
	readonly host: string;
	/** The port the server is bound to. */
	readonly port: number;
	/** The server's process id, for display. */
	readonly pid: number;
	/** Resolves when the server has ended. Never rejects. */
	readonly closed: Promise<SpawnResult>;
	/**
	 * Stops the server, `SIGTERM` then `SIGKILL` after gro's grace period, and resolves
	 * once it has ended and its output is read. Calling it again returns the same result.
	 */
	stop: () => Promise<SpawnResult>;
}

/** How a start waits. */
export interface StartPgletServerOptions {
	/** How long to wait for the listening line before stopping the server and failing. */
	ready_timeout_ms?: number;
}

/**
 * Starts `program` as `pglet_server --port 0` and resolves once it prints the
 * loopback address it's bound to. Its stdin is `/dev/null` and its output is
 * piped and read (its stdout past the listening line discarded), so a chatty
 * server never blocks on a full pipe. With no inherited stream, gro makes it
 * the leader of its own process group, which a stop, gro's exit hook and the
 * signals that end this process reach.
 *
 * A server that exits first, prints no listening line in time, or names an
 * address other than `PGLET_SERVER_HOST` is stopped before the start rejects.
 *
 * @param program - the `pglet_server` executable, with the environment its root chose
 * @throws Error - when the server doesn't come up listening on loopback; the message carries
 * its exit and the end of its stderr
 */
export const start_pglet_server = async (
	program: SpawnProgram,
	options: StartPgletServerOptions = {}
): Promise<PgletServer> => {
	const { ready_timeout_ms = PGLET_READY_TIMEOUT_MS } = options;
	const child = await program.start(['--port', '0'], { stdio: ['ignore', 'pipe', 'pipe'] });
	const stderr = read_tail(child.stderr!);
	const stdout = read_listening_line(child.stdout!);

	let stopping: Promise<SpawnResult> | null = null;
	const stop = (): Promise<SpawnResult> =>
		(stopping ??= (async () => {
			await child[Symbol.asyncDispose]();
			await Promise.all([stdout.done, stderr.done]);
			return child.closed;
		})());

	const fail = async (reason: string): Promise<never> => {
		const result = await stop();
		const lines = stdout.before.length ? `\nstdout:\n${stdout.before.join('\n')}` : '';
		const tail = stderr.text();
		throw new Error(
			`pglet_server (${program.path}) ${reason}; it ended ${format_result(result)}${lines}` +
				(tail ? `\nstderr:\n${tail}` : '')
		);
	};

	let timer: ReturnType<typeof setTimeout> | undefined;
	const ready = await Promise.race([
		stdout.address.then((address) => ({ kind: 'listening' as const, address })),
		child.closed.then((result) => ({ kind: 'ended' as const, result })),
		new Promise<{ kind: 'timeout' }>((resolve) => {
			timer = setTimeout(() => resolve({ kind: 'timeout' }), ready_timeout_ms);
		})
	]).finally(() => clearTimeout(timer));

	if (ready.kind === 'ended') return fail('exited before printing its listening line');
	if (ready.kind === 'timeout') {
		return fail(`printed no listening line within ${ready_timeout_ms}ms`);
	}
	const port = to_loopback_port(ready.address);
	if (port === null) {
		return fail(`is listening on ${ready.address}, not on ${PGLET_SERVER_HOST} with a port`);
	}
	return {
		host: PGLET_SERVER_HOST,
		port,
		pid: child.pid,
		closed: child.closed,
		stop,
		[Symbol.asyncDispose]: async () => {
			await stop();
		}
	};
};

/** The port of a `PGLET_SERVER_HOST:<port>` address, or `null` for any other address. */
const to_loopback_port = (address: string): number | null => {
	const match = /^127\.0\.0\.1:([1-9]\d{0,4})$/.exec(address);
	const port = match ? Number(match[1]) : 0;
	return port >= 1 && port <= 65535 ? port : null;
};

const format_result = (result: SpawnResult): string =>
	result.kind === 'exited'
		? `with exit code ${result.code}`
		: result.kind === 'signaled'
			? `by ${result.signal}`
			: `${result.kind}, by ${result.signal ?? `exit code ${result.code}`}`;

/**
 * Reads `chunks` until the first line that starts with `PGLET_LISTENING_PREFIX`,
 * resolving `address` with what follows it; gro discards what comes after. The
 * lines before it are kept, a bounded number, for a failed start's message.
 */
const read_listening_line = (
	chunks: AsyncIterable<Uint8Array>
): { address: Promise<string>; before: Array<string>; done: Promise<void> } => {
	let found: (address: string) => void = () => {};
	const address = new Promise<string>((resolve) => (found = resolve));
	const before: Array<string> = [];
	const done = (async () => {
		const decoder = new TextDecoder();
		let pending = '';
		try {
			for await (const chunk of chunks) {
				pending += decoder.decode(chunk, { stream: true });
				let end: number;
				while ((end = pending.indexOf('\n')) !== -1) {
					const line = pending.slice(0, end);
					pending = pending.slice(end + 1);
					if (line.startsWith(PGLET_LISTENING_PREFIX)) {
						found(line.slice(PGLET_LISTENING_PREFIX.length));
						// leaving the loop, gro reads and discards the rest
						return;
					}
					if (before.length < STDOUT_HEAD_LINES) before.push(line);
				}
			}
		} catch {
			// a stream that fails ends like one that closes; `closed` reports the exit
		}
	})();
	return { address, before, done };
};

/** Reads `chunks` to the end, keeping the last `STDERR_TAIL_CHARS` characters. */
const read_tail = (
	chunks: AsyncIterable<Uint8Array>
): { text: () => string; done: Promise<void> } => {
	let tail = '';
	const decoder = new TextDecoder();
	const done = (async () => {
		try {
			for await (const chunk of chunks) {
				tail = (tail + decoder.decode(chunk, { stream: true })).slice(-STDERR_TAIL_CHARS);
			}
		} catch {
			// as for stdout
		}
	})();
	return { text: () => tail.trimEnd(), done };
};
