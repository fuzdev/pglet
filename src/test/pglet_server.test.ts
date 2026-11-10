/// <reference types="node" />

import { fs, type FsDir } from '@fuzdev/gro/fs.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
import { assert, describe, test } from 'vitest';

import { PGLET_SERVER_HOST, start_pglet_server } from '#lib/pglet_server.ts';

const FAKE = 'src/test/fixtures/pglet_server/fake_pglet_server.sh';

/** The fake server in `mode`, recording its pid and arguments in `dir`. */
const to_fake = (mode: string, dir: FsDir) =>
	spawn.program(FAKE, { env: { FAKE_MODE: mode, FAKE_DIR: dir.path } });

/** Whether any process of the group `pid` leads is left, waiting up to `ms` for it to go. */
const group_alive = async (pid: number, ms = 3000): Promise<boolean> => {
	const deadline = Date.now() + ms;
	for (;;) {
		try {
			process.kill(-pid, 0);
		} catch (error) {
			if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false;
			throw error;
		}
		if (Date.now() > deadline) return true;
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
};

const recorded_pid = async (dir: FsDir): Promise<number> =>
	Number((await dir.read_text('pid')).trim());

/** The rejection of `promise`, failing when it resolves. */
const rejection = async (promise: Promise<unknown>): Promise<Error> => {
	try {
		await promise;
	} catch (error) {
		assert.instanceOf(error, Error);
		return error;
	}
	assert.fail('expected a rejection');
};

describe('start_pglet_server', () => {
	test('asks for port 0 and reads the port the listening line names', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		const server = await start_pglet_server(to_fake('ready', dir));
		try {
			assert.strictEqual(server.host, PGLET_SERVER_HOST);
			assert.strictEqual(server.port, 45678);
			assert.strictEqual(await dir.read_text('args'), '--port 0\n');
			assert.strictEqual(server.pid, await recorded_pid(dir));
		} finally {
			await server.stop();
		}
	});

	test('a listening line split across writes is read whole', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		await using server = await start_pglet_server(to_fake('split', dir));
		assert.strictEqual(server.port, 45679);
	});

	test('stop ends the server and its group, and a second stop returns the same result', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		const server = await start_pglet_server(to_fake('ready', dir));
		const result = await server.stop();
		assert.deepEqual(result, { kind: 'signaled', signal: 'SIGTERM' });
		assert.deepEqual(await server.closed, result);
		assert.strictEqual(await server.stop(), result);
		assert.isFalse(await group_alive(server.pid, 0));
	});

	test('a server that ignores SIGTERM is killed', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		const server = await start_pglet_server(to_fake('stubborn', dir));
		assert.deepEqual(await server.stop(), { kind: 'signaled', signal: 'SIGKILL' });
		assert.isFalse(await group_alive(server.pid));
	});

	test('disposing the server stops it', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		let pid = 0;
		{
			await using server = await start_pglet_server(to_fake('ready', dir));
			pid = server.pid;
			assert.isTrue(await group_alive(pid, 0));
		}
		assert.isFalse(await group_alive(pid, 0));
	});

	test('output past the listening line is read, so the server never blocks writing it', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		await using server = await start_pglet_server(to_fake('noisy', dir));
		assert.strictEqual(server.port, 45680);
		const deadline = Date.now() + 10_000;
		while (!(await dir.exists('drained'))) {
			assert.isBelow(Date.now(), deadline, 'the server is blocked writing its output');
			await new Promise((resolve) => setTimeout(resolve, 20));
		}
	});

	test('a server that exits first is reported with its exit and stderr', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		const error = await rejection(start_pglet_server(to_fake('exit', dir)));
		assert.include(error.message, 'exited before printing its listening line');
		assert.include(error.message, 'with exit code 3');
		assert.include(error.message, 'pglet_server: --port: invalid port "x"');
		assert.isFalse(await group_alive(await recorded_pid(dir), 0));
	});

	test('a server that never prints its port is stopped and reported', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		const error = await rejection(
			start_pglet_server(to_fake('silent', dir), { ready_timeout_ms: 300 })
		);
		assert.include(error.message, 'printed no listening line within 300ms');
		assert.include(error.message, 'by SIGTERM');
		assert.isFalse(await group_alive(await recorded_pid(dir), 0));
	});

	test('a server listening beyond loopback is stopped and reported', async () => {
		await using dir = await fs.temp_dir('pglet_server_');
		const error = await rejection(start_pglet_server(to_fake('wildcard', dir)));
		assert.include(error.message, 'is listening on 0.0.0.0:45678, not on 127.0.0.1');
		assert.isFalse(await group_alive(await recorded_pid(dir), 0));
	});
});
