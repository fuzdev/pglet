/**
 * Vitest for the suites under `tests-web/`, which `npm run test:<suite>` runs by file; `gro check`
 * runs only `src/`'s tests and never reads this.
 *
 * Each test file runs in a process of its own (`forks`): the coercion suite re-zones its process
 * through `TZ`, which a worker thread's environment copy doesn't do. No test has a deadline
 * shorter than a slow machine needs: the suites ran with none before.
 *
 * @module
 */
import {defineConfig} from 'vitest/config';

export default defineConfig({
	root: new URL('.', import.meta.url).pathname,
	test: {
		include: ['npm/*_test.ts'],
		pool: 'forks',
		testTimeout: 600_000,
		hookTimeout: 600_000,
	},
});
