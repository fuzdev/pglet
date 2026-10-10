/**
 * A vitest reporter for `exit_proof.ts`: at the end of the run it writes, as JSON, to the path in
 * `PGLET_EXITPROOF_FILE_TIMES`, the sum over the run's test files of each file's run time — the
 * file suite's `result.duration`, measured by vitest's runner from the file suite's start to its
 * end, so it holds every `beforeAll` / `afterAll` hook of the file as well as its tests, and not
 * the file's collection (import) or setup. That sum is what vitest's duration summary adds up as
 * its `tests` phase (vitest 5's `computeDurationBreakdown`); vitest 5's summary prints each phase
 * only as a percentage of the summed phases, which run in parallel workers — not a share of the
 * wall time, and not convertible to a time — and its JSON reporter spans each file by its tests
 * alone, leaving the hooks out. So the sum is read here.
 *
 * `phases` carries the other per-file sums vitest's duration breakdown is built from, so a run
 * can check `tests_ms` against the summary's percentage. Written in plain JS so vitest loads it by
 * absolute path without a transform.
 */
import {writeFileSync} from 'node:fs';

export default class FileTimesReporter {
	onTestRunEnd(test_modules) {
		const path = process.env.PGLET_EXITPROOF_FILE_TIMES;
		if (!path) return;
		const files = test_modules.map((m) => m.task);
		const sum = (read) => files.reduce((total, file) => total + (read(file) || 0), 0);
		writeFileSync(
			path,
			JSON.stringify({
				files: files.length,
				tests_ms: sum((f) => f.result?.duration),
				phases: {
					setup_ms: sum((f) => f.setupDuration),
					setup_fetch_ms: sum((f) => f.setupFetchDuration),
					collect_ms: sum((f) => f.collectDuration),
					collect_fetch_ms: sum((f) => f.collectFetchDuration),
					environment_ms: sum((f) => f.environmentLoad),
					prepare_ms: sum((f) => f.prepareDuration),
				},
			}),
		);
	}
}
