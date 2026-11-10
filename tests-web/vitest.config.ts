/**
 * Vitest for the suites under `tests-web/`, which `npm run test:<suite>` runs
 * by file; `gro check` runs only `src/`'s tests and never reads this.
 *
 * Each test file runs in a process of its own (`forks`): the coercion suite
 * re-zones its process through `TZ`, which a worker thread's copy of the
 * environment doesn't do, and a server suite's `pglet_server` is a child of
 * that process, stopped by gro's exit and signal hooks if the file never gets
 * to stop it. Files run in parallel: every server suite starts its own server
 * on a port the kernel picks.
 *
 * The deadlines are only a backstop for a hang: the suites ran with none before,
 * and a test on a loaded machine can take minutes.
 *
 * @module
 */
import {fileURLToPath} from 'node:url';
import {defineConfig} from 'vitest/config';

export default defineConfig({
	root: fileURLToPath(new URL('.', import.meta.url)),
	test: {
		include: [
			'npm/*_test.ts',
			'drivers/**/*_test.ts',
			'orms/**/*_test.ts',
			'integration/pglet_test.ts',
			'integration/server_test.ts',
		],
		pool: 'forks',
		testTimeout: 600_000,
		hookTimeout: 600_000,
	},
});
