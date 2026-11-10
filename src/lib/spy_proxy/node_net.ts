/// <reference types="node" />

/**
 * The spy proxy's Node-only half: loopback TCP and the capture file's appends,
 * on `node:net` and `node:fs`. gro's host modules have no TCP and no append, by
 * design, so this is the one module under `src/lib/` that imports `node:*`
 * (`src/test/node_imports.test.ts` holds that). Only the proxy's task root
 * imports it; the relay in `proxy.ts` takes what it builds as deps.
 *
 * Every socket here is bound or connected on `127.0.0.1`, never another
 * interface.
 *
 * @module
 */

import { open } from 'node:fs/promises';
import { connect, createServer, type Socket } from 'node:net';

import type { SpyConnection, SpyListener } from './proxy.ts';

/** The one address the proxy binds and connects on. */
export const SPY_LOOPBACK = '127.0.0.1';

/** A socket error that means the peer went away, which ends a direction quietly. */
const is_disconnect = (error: unknown): boolean => {
	const code = (error as { code?: unknown } | null)?.code;
	return (
		code === 'ECONNRESET' ||
		code === 'EPIPE' ||
		code === 'ERR_STREAM_DESTROYED' ||
		code === 'ERR_STREAM_PREMATURE_CLOSE'
	);
};

const to_spy_connection = (socket: Socket): SpyConnection => {
	let closed = false;
	// an error before anything reads the socket would otherwise be an uncaught 'error'
	// event; the reader still sees it, as the iteration of an errored stream
	socket.on('error', () => {});
	return {
		peer: `${socket.remoteAddress}:${socket.remotePort}`,
		chunks: {
			async *[Symbol.asyncIterator]() {
				try {
					// each chunk copied, so no reader holds a view into Node's buffer pool
					for await (const chunk of socket) yield new Uint8Array(chunk as Buffer);
				} catch (error) {
					if (closed || is_disconnect(error)) return;
					throw error;
				}
			}
		},
		write: (bytes) =>
			new Promise((resolve, reject) => {
				socket.write(bytes, (error) => {
					if (!error || is_disconnect(error)) resolve();
					else reject(error);
				});
			}),
		close: () => {
			closed = true;
			socket.destroy();
		}
	};
};

/** A loopback listener the caller can close, ending its connections' iteration. */
export interface SpyLoopbackListener extends SpyListener {
	/** The address it's bound to, as the kernel reports it. */
	readonly address: string;
	close: () => Promise<void>;
}

/**
 * Listens on `127.0.0.1:<port>`, port 0 for any free one.
 *
 * @throws Error - when the port can't be bound, like one in use
 */
export const listen_loopback = async (port: number): Promise<SpyLoopbackListener> => {
	const accepted: Array<Socket> = [];
	let wake: (() => void) | null = null;
	let ended = false;
	const server = createServer({ noDelay: true }, (socket) => {
		accepted.push(socket);
		wake?.();
	});
	await new Promise<void>((resolve, reject) => {
		server.once('error', reject);
		server.listen({ host: SPY_LOOPBACK, port }, () => {
			server.off('error', reject);
			resolve();
		});
	});
	server.on('close', () => {
		ended = true;
		wake?.();
	});
	const address = server.address();
	if (address === null || typeof address === 'string') throw new Error('not a TCP listener');
	return {
		address: address.address,
		port: address.port,
		connections: {
			async *[Symbol.asyncIterator]() {
				for (;;) {
					const socket = accepted.shift();
					if (socket) {
						yield to_spy_connection(socket);
						continue;
					}
					if (ended) return;
					await new Promise<void>((resolve) => (wake = resolve));
					wake = null;
				}
			}
		},
		close: () =>
			new Promise((resolve) => {
				server.close(() => resolve());
			})
	};
};

/**
 * Connects to `127.0.0.1:<port>`.
 *
 * @throws Error - when nothing accepts, like a refused connection
 */
export const connect_loopback = (port: number): Promise<SpyConnection> =>
	new Promise((resolve, reject) => {
		const socket = connect({ host: SPY_LOOPBACK, port, noDelay: true });
		socket.once('error', reject);
		socket.once('connect', () => {
			socket.off('error', reject);
			resolve(to_spy_connection(socket));
		});
	});

/** The capture file, opened for appending. */
export interface SpyCaptureFile {
	append: (line: string) => Promise<void>;
	close: () => Promise<void>;
}

/** Opens `path` for appending, creating it when missing; every write lands at the end. */
export const open_capture_file = async (path: string): Promise<SpyCaptureFile> => {
	const handle = await open(path, 'a');
	return {
		append: async (line) => {
			await handle.write(line);
		},
		close: () => handle.close()
	};
};
