/**
 * What `cargo xtask measure`'s preflight asks of the benches' own rules, so the Rust side
 * keeps no second copy of them. Prints one answer to stdout:
 *
 *   node benches/lib/measure_preflight.ts pglite   # the version benches/ loads, refused
 *                                                  # (exit 1, the reason on stderr) unless
 *                                                  # it is the lockfile's (`pglite_install.ts`)
 *   node benches/lib/measure_preflight.ts stamp    # the stamp a results file written now
 *                                                  # carries (`results.ts`'s `git_commit`)
 *
 * Any other question is exit 2.
 *
 * @module
 */

import {assert_pglite_matches_lockfile} from './pglite_install.ts';
import {git_commit} from './results.ts';

const question = process.argv[2];
if (question === 'pglite') {
	try {
		console.log(assert_pglite_matches_lockfile().version);
	} catch (e) {
		console.error(e instanceof Error ? e.message : String(e));
		process.exitCode = 1;
	}
} else if (question === 'stamp') {
	console.log(git_commit());
} else {
	console.error(`usage: node benches/lib/measure_preflight.ts pglite|stamp (got ${String(question)})`);
	process.exitCode = 2;
}
