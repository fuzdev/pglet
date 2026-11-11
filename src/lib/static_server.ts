/// <reference types="node" />

/**
 * The static server's Node-only half: an HTTP listener on `node:http`, bound to
 * `127.0.0.1` alone, answering each request through `respond_static`. gro's host
 * modules have no listening socket, by design, so this is one of the modules
 * under `src/lib/` that imports `node:*` (`src/test/node_imports.test.ts` holds
 * the list). The browser tests' runner and the `wasm:serve` task import it.
 *
 * @module
 */

import { createServer, type Server } from 'node:http';

import { respond_static, type StaticMount } from './static_files.ts';

/** The one address the server binds. */
export const STATIC_SERVER_HOST = '127.0.0.1';

/** A listening server. */
export interface StaticServer extends AsyncDisposable {
	/** Always `STATIC_SERVER_HOST`. */
	readonly host: string;
	/** The port it's bound to, the kernel's pick for port `0`. */
	readonly port: number;
	/** `http://127.0.0.1:<port>`, with no trailing `/`. */
	readonly origin: string;
	/** Stops listening and ends every open connection; resolves once the server is closed. */
	close: () => Promise<void>;
}

/** The start's failure when its port is taken: the port is the person's to free. */
export class StaticServerPortInUseError extends Error {
	/** The port that was taken. */
	readonly port: number;

	constructor(port: number, options?: ErrorOptions) {
		super(
			`port ${port} on ${STATIC_SERVER_HOST} is in use: stop what is listening there ` +
				'(`ss -ltnp` names it) and start again',
			options
		);
		this.name = 'StaticServerPortInUseError';
		this.port = port;
	}
}

/** How a server starts. */
export interface StartStaticServerOptions {
	/** The port to bind; `0`, the default, lets the kernel pick a free one. */
	port?: number;
	/** Called once per answered request, with its method, target and status. */
	log?: (method: string, target: string, status: number) => void;
}

/**
 * Starts serving `mounts` on `127.0.0.1`, and resolves once the port is bound.
 *
 * @param mounts - the directories served, as `respond_static` reads them
 * @throws StaticServerPortInUseError - when the port is taken
 * @throws Error - when the port can't be bound for another reason
 */
export const start_static_server = async (
	mounts: ReadonlyArray<StaticMount>,
	options: StartStaticServerOptions = {}
): Promise<StaticServer> => {
	const { port = 0, log } = options;
	const server = createServer((request, response) => {
		const method = request.method ?? 'GET';
		const target = request.url ?? '/';
		respond_static(mounts, method, target).then(
			(answer) => {
				response.writeHead(answer.status, answer.headers);
				response.end(answer.body ?? undefined);
				log?.(method, target, answer.status);
			},
			(error: unknown) => {
				response.writeHead(500, { 'Content-Type': 'text/plain; charset=utf-8' });
				response.end('Internal Server Error');
				log?.(method, target, 500);
				console.error(error);
			}
		);
	});
	await listen(server, port);
	const address = server.address();
	if (address === null || typeof address === 'string') {
		server.close();
		throw new Error(`the static server bound no TCP port on ${STATIC_SERVER_HOST}`);
	}
	let closing: Promise<void> | null = null;
	const close = (): Promise<void> =>
		(closing ??= new Promise<void>((resolve) => {
			server.close(() => resolve());
			server.closeAllConnections();
		}));
	return {
		host: STATIC_SERVER_HOST,
		port: address.port,
		origin: `http://${STATIC_SERVER_HOST}:${address.port}`,
		close,
		[Symbol.asyncDispose]: close
	};
};

const listen = (server: Server, port: number): Promise<void> =>
	new Promise((resolve, reject) => {
		const on_error = (error: NodeJS.ErrnoException): void => {
			reject(
				error.code === 'EADDRINUSE' ? new StaticServerPortInUseError(port, { cause: error }) : error
			);
		};
		server.once('error', on_error);
		server.listen({ port, host: STATIC_SERVER_HOST, exclusive: true }, () => {
			server.off('error', on_error);
			resolve();
		});
	});
