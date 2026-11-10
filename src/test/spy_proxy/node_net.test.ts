import { fs } from '@fuzdev/gro/fs.ts';
import { assert, describe, test } from 'vitest';

import {
	connect_loopback,
	listen_loopback,
	open_capture_file,
	SPY_LOOPBACK
} from '#lib/spy_proxy/node_net.ts';
import { relay_spy_connection, type SpyConnection } from '#lib/spy_proxy/proxy.ts';

import { to_message, to_startup_message } from './spy_helpers.ts';

const read_all = async (connection: SpyConnection): Promise<Array<number>> => {
	const bytes: Array<number> = [];
	for await (const chunk of connection.chunks) bytes.push(...chunk);
	return bytes;
};

describe('the loopback sockets', () => {
	test('port 0 binds a free loopback port, and connections carry bytes both ways', async () => {
		const listener = await listen_loopback(0);
		try {
			assert.isAbove(listener.port, 0);
			const accepted = (async () => {
				for await (const connection of listener.connections) return connection;
				throw new Error('no connection');
			})();
			const client = await connect_loopback(listener.port);
			const server = await accepted;
			assert.match(server.peer, /^127\.0\.0\.1:\d+$/);
			await client.write(new Uint8Array([1, 2, 3]));
			await server.write(new Uint8Array([9]));
			client.close();
			assert.deepEqual(await read_all(server), [1, 2, 3]);
			server.close();
		} finally {
			await listener.close();
		}
	});

	test("listening on a port that's taken fails, and connecting where nothing listens fails", async () => {
		const listener = await listen_loopback(0);
		try {
			let error: unknown;
			await listen_loopback(listener.port).catch((e: unknown) => (error = e));
			assert.strictEqual((error as { code?: string }).code, 'EADDRINUSE');
		} finally {
			await listener.close();
		}
		let error: unknown;
		await connect_loopback(listener.port).catch((e: unknown) => (error = e));
		assert.strictEqual((error as { code?: string }).code, 'ECONNREFUSED');
	});

	test('only the loopback address is bound', async () => {
		assert.strictEqual(SPY_LOOPBACK, '127.0.0.1');
		const listener = await listen_loopback(0);
		try {
			assert.strictEqual(listener.address, '127.0.0.1');
		} finally {
			await listener.close();
		}
	});

	test('a relay over real sockets forwards and captures, appending to the file', async () => {
		await using dir = await fs.temp_dir('pglet_spy_');
		await dir.write('capture.jsonl', 'earlier line\n');
		const capture = await open_capture_file(`${dir.path}/capture.jsonl`);
		const upstream = await listen_loopback(0);
		const proxy = await listen_loopback(0);
		try {
			const relayed = (async () => {
				for await (const client of proxy.connections) {
					await relay_spy_connection(
						{
							connect: () => connect_loopback(upstream.port),
							append: capture.append,
							now: () => 'T',
							print: () => {},
							print_error: () => {}
						},
						client
					);
					return;
				}
			})();
			const upstream_side = (async () => {
				for await (const connection of upstream.connections) return connection;
				throw new Error('no upstream connection');
			})();
			const client = await connect_loopback(proxy.port);
			const startup = to_startup_message({ user: 'u' });
			await client.write(startup);
			const server = await upstream_side;
			const ready = to_message('Z', [{ bytes: [0x49] }]);
			await server.write(ready);
			const received: Array<number> = [];
			for await (const chunk of client.chunks) {
				received.push(...chunk);
				if (received.length >= ready.length) break;
			}
			assert.deepEqual(received, [...ready]);
			client.close();
			await relayed;
			assert.deepEqual(await read_all(server), [...startup]);
			server.close();
		} finally {
			await capture.close();
			await proxy.close();
			await upstream.close();
		}
		const lines = (await dir.read_text('capture.jsonl')).split('\n');
		assert.strictEqual(lines[0], 'earlier line');
		assert.deepEqual(
			lines.slice(1, -1).map((line) => JSON.parse(line).type),
			['StartupMessage', 'ReadyForQuery']
		);
	});
});
