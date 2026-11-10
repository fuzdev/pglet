/**
 * The spy proxy: it sits between a Postgres client and server on loopback,
 * forwarding every byte unchanged and logging each message it parses, one JSON
 * line per message, to a capture file. The sockets and the file are deps, so
 * the relay is tested over in-memory connections; `node_net.ts` is the Node
 * implementation the task root hands in.
 *
 * @module
 */

import { parse_spy_message, type SpyDirection, type SpyMessage } from './messages.ts';

/** One side of a proxied connection. */
export interface SpyConnection {
	/** The peer, for the log, as `host:port`. */
	readonly peer: string;
	/** The bytes the peer sends, each chunk once, ending when the peer closes. */
	readonly chunks: AsyncIterable<Uint8Array>;
	/** Sends bytes to the peer. */
	write: (bytes: Uint8Array) => Promise<void>;
	/** Closes the connection; the chunks then end. */
	close: () => void;
}

/** A listening socket. */
export interface SpyListener {
	/** The port it's bound to, the one asked for or the kernel's pick for port 0. */
	readonly port: number;
	/** Each accepted client. */
	readonly connections: AsyncIterable<SpyConnection>;
}

/** What one proxied connection reaches. */
export interface SpyRelayDeps {
	/** Connects to the server being spied on. */
	connect: () => Promise<SpyConnection>;
	/** Appends one line to the capture. */
	append: (line: string) => Promise<void>;
	/** The capture's timestamp, an ISO string. */
	now: () => string;
	/** Prints a line to stdout, as `console.log`. */
	print: (text: string) => void;
	/** Prints a line to stderr, as `console.error`. */
	print_error: (text: string) => void;
}

/** What the proxy reaches, gathered by the task root. */
export interface SpyProxyDeps extends SpyRelayDeps {
	/** Listens on loopback on `port`, 0 for any free one. */
	listen: (port: number) => Promise<SpyListener>;
}

/** Where the proxy listens and forwards, and where it logs. */
export interface SpyProxyOptions {
	listen_port: number;
	server_host: string;
	server_port: number;
	/** The capture file, for the startup lines. */
	output: string;
}

/** The capture dir beneath the checkout, the one place the proxy writes. */
export const SPY_CAPTURE_DIR = 'tests/captures/';

/** What the positional args name when they're left out. */
export const SPY_DEFAULTS = {
	listen_port: 5434,
	server_port: 5432,
	output: `${SPY_CAPTURE_DIR}session.jsonl`
} as const;

/** The usage line every argument error ends with. */
export const SPY_USAGE =
	'usage: npm run spy:start -- [listen_port] [server_port] [tests/captures/<file>.jsonl]';

/** The proxy's positional args, checked. */
export interface SpyProxyArgs {
	listen_port: number;
	server_port: number;
	/** The capture file beneath the checkout, inside `SPY_CAPTURE_DIR`. */
	output: string;
}

const to_port = (text: string, min: number): number | null => {
	if (!/^\d{1,5}$/.test(text)) return null;
	const port = Number(text);
	return port >= min && port <= 65535 ? port : null;
};

/**
 * Checks the proxy's positional args, `[listen_port] [server_port] [output]`,
 * each defaulting to `SPY_DEFAULTS`. The listen port may be 0, for any free
 * one; the output is a path beneath the checkout inside `SPY_CAPTURE_DIR`,
 * relative and without `.` or `..` segments.
 */
export const parse_spy_proxy_args = (
	positionals: ReadonlyArray<string>
): { ok: true; args: SpyProxyArgs } | { ok: false; error: string } => {
	if (positionals.length > 3) {
		return { ok: false, error: `too many arguments: ${positionals.join(' ')}\n${SPY_USAGE}` };
	}
	const [listen = String(SPY_DEFAULTS.listen_port), server = String(SPY_DEFAULTS.server_port)] =
		positionals;
	const output = positionals[2] ?? SPY_DEFAULTS.output;
	const listen_port = to_port(listen, 0);
	if (listen_port === null) {
		return { ok: false, error: `listen_port must be 0 to 65535, got "${listen}"\n${SPY_USAGE}` };
	}
	const server_port = to_port(server, 1);
	if (server_port === null) {
		return { ok: false, error: `server_port must be 1 to 65535, got "${server}"\n${SPY_USAGE}` };
	}
	const segments = output.split('/');
	if (
		!output.startsWith(SPY_CAPTURE_DIR) ||
		output.length === SPY_CAPTURE_DIR.length ||
		segments.some((s) => s === '' || s === '.' || s === '..')
	) {
		return {
			ok: false,
			error: `the capture goes in ${SPY_CAPTURE_DIR}, got "${output}"\n${SPY_USAGE}`
		};
	}
	return { ok: true, args: { listen_port, server_port, output } };
};

const concat = (a: Uint8Array, b: Uint8Array): Uint8Array => {
	const out = new Uint8Array(a.length + b.length);
	out.set(a);
	out.set(b, a.length);
	return out;
};

/**
 * Proxies one client until either side closes, logging every message each side
 * sends. A client's SSL request is answered by the server with one byte, logged
 * as `SSLResponse`, after which the client sends its startup message again. An
 * error other than the peer going away is printed, and ends the connection.
 */
export const relay_spy_connection = async (
	deps: SpyRelayDeps,
	client: SpyConnection
): Promise<void> => {
	const { append, now, print, print_error } = deps;
	print(`[CONNECT] Client ${client.peer}`);

	let server: SpyConnection;
	try {
		server = await deps.connect();
	} catch (error) {
		print_error(`[ERROR] Failed to connect to Postgres: ${error}`);
		client.close();
		return;
	}

	const log = async (message: SpyMessage): Promise<void> => {
		print(
			`[${message.direction.toUpperCase()}] ${message.type} ${JSON.stringify(message.payload)}`
		);
		await append(JSON.stringify(message) + '\n');
	};

	let client_startup = true;
	let awaiting_ssl_response = false;

	const pump = async (direction: SpyDirection): Promise<void> => {
		const [from, to] = direction === 'client' ? [client, server] : [server, client];
		let pending: Uint8Array = new Uint8Array(0);
		try {
			for await (const chunk of from.chunks) {
				pending = concat(pending, chunk);
				// the server answers an SSL request with one byte, 'N' or 'S'
				if (direction === 'server' && awaiting_ssl_response && pending.length >= 1) {
					await log({
						direction: 'server',
						type: 'SSLResponse',
						length: 1,
						payload: { response: String.fromCharCode(pending[0]!) === 'N' ? 'NoSSL' : 'SSL' },
						timestamp: now()
					});
					awaiting_ssl_response = false;
					pending = pending.slice(1);
				}
				while (pending.length > 0) {
					const parsed = parse_spy_message(
						pending,
						direction,
						direction === 'client' && client_startup,
						now
					);
					if (!parsed) break;
					await log(parsed.message);
					if (parsed.message.type === 'SSLRequest') awaiting_ssl_response = true;
					else if (parsed.message.type === 'StartupMessage') client_startup = false;
					pending = pending.slice(parsed.consumed);
				}
				await to.write(chunk);
			}
		} catch (error) {
			print_error(`[ERROR] ${direction === 'client' ? 'Client' : 'Server'} read: ${error}`);
		}
	};

	await Promise.race([pump('client'), pump('server')]);
	client.close();
	server.close();
	print(`[DISCONNECT] Client ${client.peer}`);
};

/**
 * Listens on loopback and proxies each client to the server, until the
 * listener ends. Prints the bound port, so port 0 can be asked for and read.
 */
export const run_spy_proxy = async (
	deps: SpyProxyDeps,
	options: SpyProxyOptions
): Promise<void> => {
	const listener = await deps.listen(options.listen_port);
	deps.print(`Spy proxy listening on 127.0.0.1:${listener.port}`);
	deps.print(`Forwarding to ${options.server_host}:${options.server_port}`);
	deps.print(`Logging to ${options.output}`);
	// the relays in flight, each removed when it ends, so a long session holds only the open ones
	const relays: Set<Promise<void>> = new Set();
	for await (const client of listener.connections) {
		const relay = relay_spy_connection(deps, client).finally(() => relays.delete(relay));
		relays.add(relay);
	}
	await Promise.all(relays);
};
