/**
 * The `pglet_server` the server suites start: this checkout's debug build, run
 * through `#lib/pglet_server.ts` on a port the kernel picks. This module is the
 * suites' composition root for it: the one place a server suite's program is
 * made.
 *
 * @module
 */
import {fileURLToPath} from 'node:url';
import {fs} from '@fuzdev/gro/fs.ts';
import {spawn, type SpawnProgram} from '@fuzdev/gro/spawn.ts';
import {afterAll, onTestFinished} from 'vitest';
import {start_pglet_server, type PgletServer} from '#lib/pglet_server.ts';

/** This checkout, from this module's own location. */
const CHECKOUT = fileURLToPath(new URL('../../', import.meta.url));

/** The debug build, beneath the checkout: `npm run test:<suite>` builds it first. */
const SERVER = 'target/debug/pglet_server';

/**
 * This checkout's debug `pglet_server`, with gro's base environment and no more:
 * the server reads no variable.
 *
 * gro resolves a program name with a `/` against the working directory, so the
 * name is checked against this checkout's build, found from this module's own
 * location, and a run from another directory is refused rather than starting
 * another tree's server.
 *
 * @throws Error - when the build is missing, or the name resolves outside this checkout
 */
export const to_pglet_server_program = async (): Promise<SpawnProgram> => {
	const debug = await fs.open_dir(`${CHECKOUT}target/debug`).catch(() => null);
	if (debug === null || !(await debug.exists('pglet_server'))) {
		throw new Error(`no ${SERVER} in ${CHECKOUT}: build it with cargo build -p pglet_server`);
	}
	const expected = `${debug.path}/pglet_server`;
	const program = spawn.find(SERVER);
	if (program?.path !== expected) {
		throw new Error(
			`${SERVER} resolved to ${program?.path ?? 'nothing'}, not this checkout's ${expected}: ` +
				`run the suite from ${CHECKOUT}`,
		);
	}
	return program;
};

/**
 * Starts a server for the calling test file, and stops it when the file's tests
 * are done, passed or failed. Call it at the top level of a test file.
 */
export const start_suite_server = async (): Promise<PgletServer> => {
	const server = await start_pglet_server(await to_pglet_server_program());
	afterAll(() => server.stop());
	return server;
};

/**
 * Starts a server for the calling test, and stops it when the test ends,
 * passed or failed, unless the test stopped it first. Call it inside a test.
 */
export const start_test_server = async (): Promise<PgletServer> => {
	const server = await start_pglet_server(await to_pglet_server_program());
	onTestFinished(async () => {
		await server.stop();
	});
	return server;
};
