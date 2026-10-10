/**
 * The one gzip measurement behind every recorded gzip figure in this directory.
 *
 * `size.ts` (the `size_baseline.json` wire size), `size_compare.ts` (both halves of
 * `pglite_size_comparison.json`) and `wasm_init.ts` (the size it reports beside its
 * timings) read a gzip byte count through this, so two gzip figures from two files are
 * the same tool at the same level: `gzip -c`, gzip's default level, shelled out — under
 * `size.ts`'s narrow `--allow-run=gzip,git` as well as `-A`.
 *
 * @module
 */

/**
 * The byte count `gzip -c` writes for the file at `path`.
 *
 * @throws when gzip fails — a size gate or a comparison must not proceed on a figure it
 * did not measure
 */
export const gzip_size = async (path: string): Promise<number> => {
	const {success, stdout, stderr} = await new Deno.Command('gzip', {
		args: ['-c', path],
		stdout: 'piped',
		stderr: 'piped',
	}).output();
	if (!success) throw new Error(`gzip failed: ${new TextDecoder().decode(stderr)}`);
	return stdout.length;
};
