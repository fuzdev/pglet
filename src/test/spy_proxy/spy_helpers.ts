import type { SpyConnection } from '#lib/spy_proxy/proxy.ts';

const encoder = new TextEncoder();

/** A part of a wire payload: a NUL-terminated string, a big-endian int, or raw bytes. */
export type PayloadPart =
	{ str: string } | { i16: number } | { i32: number } | { bytes: Array<number> };

/** Builds a payload from its parts, as the wire lays them out. */
export const to_payload = (parts: Array<PayloadPart>): Uint8Array => {
	const out: Array<number> = [];
	for (const p of parts) {
		if ('str' in p) {
			out.push(...encoder.encode(p.str), 0);
		} else if ('i16' in p) {
			out.push((p.i16 >> 8) & 0xff, p.i16 & 0xff);
		} else if ('i32' in p) {
			out.push((p.i32 >>> 24) & 0xff, (p.i32 >>> 16) & 0xff, (p.i32 >>> 8) & 0xff, p.i32 & 0xff);
		} else {
			out.push(...p.bytes);
		}
	}
	return new Uint8Array(out);
};

/** A typed message: its type byte, a length counting itself, and the payload. */
export const to_message = (type: string, parts: Array<PayloadPart>): Uint8Array => {
	const payload = to_payload(parts);
	return new Uint8Array([
		...to_payload([{ bytes: [type.charCodeAt(0)] }, { i32: payload.length + 4 }]),
		...payload
	]);
};

/** A client's startup message for protocol 3.0 with `params`. */
export const to_startup_message = (params: Record<string, string>): Uint8Array => {
	const body = to_payload([
		{ i32: 196608 },
		...Object.entries(params).flatMap(([k, v]) => [{ str: k }, { str: v }]),
		{ bytes: [0] }
	]);
	return new Uint8Array([...to_payload([{ i32: body.length + 4 }]), ...body]);
};

/** A client's SSL request. */
export const SSL_REQUEST = to_payload([{ i32: 8 }, { i32: 80877103 }]);

/** One end of an in-memory connection, its peer's sends readable as `chunks`. */
export interface FakeConnection extends SpyConnection {
	/** Delivers bytes as the peer sending them. */
	send: (bytes: Uint8Array) => void;
	/** Ends the chunks, as the peer closing. */
	end: () => void;
	/** Everything written to the peer, in order. */
	written: Array<Uint8Array>;
	closed: boolean;
}

/** An in-memory connection to `peer`. */
export const create_fake_connection = (peer: string): FakeConnection => {
	const queue: Array<Uint8Array> = [];
	let ended = false;
	let wake: (() => void) | null = null;
	const connection: FakeConnection = {
		peer,
		written: [],
		closed: false,
		send: (bytes) => {
			queue.push(bytes);
			wake?.();
		},
		end: () => {
			ended = true;
			wake?.();
		},
		chunks: {
			async *[Symbol.asyncIterator]() {
				for (;;) {
					const chunk = queue.shift();
					if (chunk) {
						yield chunk;
						continue;
					}
					if (ended) return;
					await new Promise<void>((resolve) => (wake = resolve));
					wake = null;
				}
			}
		},
		write: async (bytes) => {
			connection.written.push(bytes);
		},
		close: () => {
			connection.closed = true;
			connection.end();
		}
	};
	return connection;
};

/** Waits for pending microtasks and timers, so a relay catches up with what was sent. */
export const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));
