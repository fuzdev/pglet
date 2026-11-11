/// <reference types="node" />

import { connect, type Socket } from 'node:net';
import { networkInterfaces } from 'node:os';

import { fs } from '@fuzdev/gro/fs.ts';
import { assert, describe, test } from 'vitest';

import {
	start_static_server,
	STATIC_SERVER_HOST,
	StaticServerPortInUseError
} from '#lib/static_server.ts';

const to_mounts = async () => {
	const dir = await fs.temp_dir('pglet_static_server_');
	await dir.mkdir('pkg');
	await dir.write('pkg/a.wasm', new Uint8Array([0, 97, 115, 109]));
	await dir.write('secret.txt', 'secret');
	return { dir, mounts: [{ prefix: 'pkg', open: () => dir.dir('pkg').readonly() }] };
};

/** Whether a TCP connection to `host:port` is accepted. */
const accepts = (host: string, port: number): Promise<boolean> =>
	new Promise((resolve) => {
		const client = connect({ host, port });
		client.once('connect', () => {
			client.destroy();
			resolve(true);
		});
		client.once('error', () => resolve(false));
	});

describe('start_static_server', () => {
	test('binds 127.0.0.1 on a port the kernel picks, and answers through respond_static', async () => {
		const { dir, mounts } = await to_mounts();
		await using _dir = dir;
		const logged: Array<string> = [];
		await using server = await start_static_server(mounts, {
			log: (method, target, status) => logged.push(`${method} ${target} ${status}`)
		});
		assert.strictEqual(STATIC_SERVER_HOST, '127.0.0.1');
		assert.strictEqual(server.host, '127.0.0.1');
		assert.isAbove(server.port, 0);
		assert.strictEqual(server.origin, `http://127.0.0.1:${server.port}`);

		const wasm = await fetch(`${server.origin}/pkg/a.wasm`);
		assert.strictEqual(wasm.status, 200);
		assert.strictEqual(wasm.headers.get('content-type'), 'application/wasm');
		assert.deepEqual([...new Uint8Array(await wasm.arrayBuffer())], [0, 97, 115, 109]);

		// a raw target, as a client that doesn't normalize sends it
		const climb = await raw_get(server.port, '/pkg/%2e%2e/secret.txt');
		assert.match(climb, /^HTTP\/1\.1 400 /);
		assert.notInclude(climb, 'secret');

		assert.deepEqual(logged, ['GET /pkg/a.wasm 200', 'GET /pkg/%2e%2e/secret.txt 400']);
	});

	test('answers on no other address of this machine', async () => {
		const { dir, mounts } = await to_mounts();
		await using _dir = dir;
		await using server = await start_static_server(mounts);
		assert.isTrue(await accepts('127.0.0.1', server.port));
		const others = Object.values(networkInterfaces())
			.flat()
			.filter((i) => i !== undefined && !i.internal)
			.map((i) => i!.address);
		for (const address of [...others, '::1']) {
			assert.isFalse(await accepts(address, server.port), address);
		}
	});

	test('a port in use fails the start, naming the port', async () => {
		const { dir, mounts } = await to_mounts();
		await using _dir = dir;
		await using first = await start_static_server(mounts);
		let error: unknown;
		await start_static_server(mounts, { port: first.port }).catch((e: unknown) => (error = e));
		assert.instanceOf(error, StaticServerPortInUseError);
		assert.strictEqual((error as StaticServerPortInUseError).port, first.port);
		assert.include((error as Error).message, `port ${first.port} on 127.0.0.1 is in use`);
		assert.strictEqual(((error as Error).cause as { code?: string }).code, 'EADDRINUSE');
	});

	test('a listen error other than a port in use is passed on as it is', async (context) => {
		const { dir, mounts } = await to_mounts();
		await using _dir = dir;
		// port 1 is privileged: `EACCES` unless this process may bind it
		let error: unknown;
		const server = await start_static_server(mounts, { port: 1 }).catch((e: unknown) => {
			error = e;
			return null;
		});
		if (server !== null) {
			await server.close();
			context.skip('this process may bind port 1');
		}
		assert.notInstanceOf(error, StaticServerPortInUseError);
		assert.strictEqual((error as { code?: string }).code, 'EACCES');
	});

	test('two servers started from port 0 are two servers', async () => {
		const { dir, mounts } = await to_mounts();
		await using _dir = dir;
		await using a = await start_static_server(mounts);
		await using b = await start_static_server(mounts);
		assert.notStrictEqual(a.port, b.port);
	});

	test('close ends open connections, one mid-request included, and frees the port', async () => {
		const { dir, mounts } = await to_mounts();
		await using _dir = dir;
		const server = await start_static_server(mounts);
		const held = await new Promise<Socket>((resolve) => {
			const socket = connect({ host: '127.0.0.1', port: server.port }, () => resolve(socket));
		});
		// a request whose headers never end keeps the connection active, which a plain close waits on
		held.write('GET /pkg/a.wasm HTTP/1.1\r\nHost: 127.0.0.1\r\n');
		// long enough for the server to have read it; a connection it hasn't, a close ends as idle
		await new Promise((resolve) => setTimeout(resolve, 200));
		const ended = new Promise<void>((resolve) => held.once('close', () => resolve()));
		held.on('error', () => {});
		await server.close();
		await ended;
		assert.isFalse(await accepts('127.0.0.1', server.port));
		await server.close();
	});
});

/** Sends `GET <target>` as written and reads the whole response. */
const raw_get = (port: number, target: string): Promise<string> =>
	new Promise((resolve, reject) => {
		const socket = connect({ host: '127.0.0.1', port }, () => {
			socket.write(`GET ${target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n`);
		});
		let response = '';
		socket.on('data', (chunk) => (response += chunk.toString('utf8')));
		socket.on('end', () => resolve(response));
		socket.on('error', reject);
	});
