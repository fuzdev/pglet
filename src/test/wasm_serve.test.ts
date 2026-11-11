/// <reference types="node" />

import { realpath } from 'node:fs/promises';

import { fs } from '@fuzdev/gro/fs.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
import { assert, test } from 'vitest';

import {
	start_static_server,
	StaticServerPortInUseError,
	type StaticServer
} from '#lib/static_server.ts';

/** The port `wasm:serve` binds, as its task and the docs state it. */
const WASM_SERVE_PORT = 8001;

test('wasm:serve on a port in use exits 1 with one line naming the port', async () => {
	// held by this test, or already by something else: taken either way
	let held: StaticServer | null = null;
	try {
		held = await start_static_server([], { port: WASM_SERVE_PORT });
	} catch (error) {
		if (!(error instanceof StaticServerPortInUseError)) throw error;
	}
	try {
		const node = spawn.find('node');
		const expected = await realpath(process.execPath);
		assert(node !== null && node.path === expected, `node on PATH is not ${expected}`);
		const project = fs.project.readonly();
		const out = await node.out(
			[
				`${project.path}/node_modules/@fuzdev/gro/dist/gro.js`,
				'task',
				'src/tasks/wasm_serve.task.ts'
			],
			{ cwd: project, timeout_ms: 30_000 }
		);
		assert.deepEqual(
			{ kind: out.kind, code: 'code' in out ? out.code : null },
			{ kind: 'exited', code: 1 },
			`stdout:\n${out.stdout}\nstderr:\n${out.stderr}`
		);
		// beside gro's lines naming the task, the failure is one line, with no stack and no
		// advice to reinstall
		const output = `${out.stdout}\n${out.stderr}`;
		assert.deepEqual(
			output.split('\n').filter((line) => line.includes('error')),
			[
				`🞩error🞩 [src/tasks/wasm_serve.task.ts] port ${WASM_SERVE_PORT} on 127.0.0.1 is in use: ` +
					'stop what is listening there (`ss -ltnp` names it) and start again'
			],
			output
		);
		assert.notMatch(output, /^\s+at /m, output);
		assert.notInclude(output, 'Unexpected error', output);
	} finally {
		await held?.close();
	}
});
