/**
 * The gzip measurement behind the recorded gzip figures this directory's scripts write.
 *
 * `size_compare.ts` (both halves of `pglite_size_comparison.json`) and `wasm_init.ts` (the
 * size it reports beside its timings) read a gzip byte count through this, and the size
 * gate (`src/lib/size_gate.ts`, the `size_baseline.json` wire size) runs the same
 * arguments (`to_gzip_args` in `src/lib/bench_results.ts`), so two gzip figures from two
 * files are the same tool at the same level: `gzip -c`, gzip's default level, shelled out.
 *
 * @module
 */

import {execFile} from 'node:child_process';
import {to_gzip_args} from '../../src/lib/bench_results.ts';

/** Room for gzip's output of any artifact measured here; past it the measurement fails. */
const MAX_BYTES = 256 * 1024 * 1024;

/**
 * The byte count `gzip -c` writes for the file at `path`.
 *
 * @throws when gzip fails, since a size gate or a comparison must not proceed on a figure
 * it did not measure
 */
export const gzip_size = (path: string): Promise<number> =>
	new Promise((resolve, reject) => {
		execFile(
			'gzip',
			to_gzip_args(path),
			{encoding: 'buffer', maxBuffer: MAX_BYTES},
			(error, stdout, stderr) => {
				if (error) reject(new Error(`gzip failed: ${stderr.toString('utf8') || error.message}`));
				else resolve(stdout.length);
			},
		);
	});
