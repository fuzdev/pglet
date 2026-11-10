import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { to_raw_rest_args } from '@fuzdev/gro/args.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import { GIT_LOCAL_PASS_ENV } from '@fuzdev/gro/git.ts';
import { process } from '@fuzdev/gro/process.ts';
import { spawn, type SpawnProgramOptions } from '@fuzdev/gro/spawn.ts';
import type { Task } from '@fuzdev/gro/task.ts';

import {
	PUBLISH_NPM_FLAG_VARS,
	PUBLISH_PKG_DIR,
	run_publish_command,
	to_publish_stray_vars
} from '#lib/release/publish.ts';
import type { ReleaseProgram } from '#lib/release/programs.ts';
import { to_task_checkout_path } from '#lib/task_paths.ts';

/** @nodocs */
export const Args = s.object({
	wetrun: s.with_default(
		meta(s.boolean, { description: 'bump, publish, commit, tag and push; a dry run without it' }),
		false
	),
	bump: s.optional(
		meta(s.string, {
			description: "patch, minor or major, repeating the changelog's `<!-- bump: … -->` marker"
		})
	),
	git: s.with_default(meta(s.boolean, { description: 'dual of no-git' }), true),
	'no-git': s.with_default(
		meta(s.boolean, { description: 'stop after the publish and print the commit, tag and push' }),
		false
	)
});
export type Args = s.Infer<typeof Args>;

/**
 * `npm run publish:npm`, the composition root: it gathers the host modules and
 * hands them to `run_publish_command`. Its reach:
 *
 * - this checkout, found from this module's own location, never the working
 *   directory: it reads the release files and `pkg-npm`, and a wetrun writes
 *   `Cargo.toml`, `CHANGELOG.md` and `.publish-in-progress`; every program runs
 *   in it
 * - `git` twice: for the local reads with git's own variables
 *   (`GIT_LOCAL_PASS_ENV`), and for the fetch, the remote tag lookup, the
 *   commit, the tag and the push with the whole environment, since credentials,
 *   signing and hooks are the user's setup
 * - `npm` with the whole environment and `PKG_DIR` set to `pkg-npm`, which only
 *   `test:npm`'s Node legs read: the registry's auth and config are the user's,
 *   and it runs the `package.json` gates, cargo and wasm-pack beneath
 * - `deno` with the whole environment: it runs the `deno.json` gate and the size
 *   gate
 * - `cargo` with its own and rustup's variables, for its probe and the offline
 *   lockfile update; `node`, `wasm-pack` and `gzip` with gro's base environment,
 *   for their probes
 * - the network, through `git` and `npm` alone: `origin`, and the npm registry
 * - the CLI args and the `PUBLISH_NPM_FLAG_VARS` variables, read only to refuse
 *   a flag that never reached `Args`
 */
export const task: Task<Args> = {
	summary:
		'release @fuzdev/pglet-wasm, a dry run unless --wetrun (npm run publish:npm -- --wetrun --bump patch)',
	Args,
	run: async ({ args }) => {
		const env = process.env.narrow(PUBLISH_NPM_FLAG_VARS);
		const stray = [
			...to_publish_stray_vars((name) => env.get(name)),
			...to_raw_rest_args(process.args)
		];
		const checkout = await fs.open_dir(to_task_checkout_path(import.meta.url));
		const to_program = (name: string, options?: SpawnProgramOptions): ReleaseProgram => ({
			name,
			program: spawn.find(name, options),
			cwd: checkout
		});
		await run_publish_command(
			{
				checkout,
				git_local: to_program('git', { pass_env: GIT_LOCAL_PASS_ENV }),
				git_remote: to_program('git', { pass_env: 'all' }),
				npm: to_program('npm', { pass_env: 'all', env: { PKG_DIR: PUBLISH_PKG_DIR } }),
				deno: to_program('deno', { pass_env: 'all' }),
				cargo: to_program('cargo', { pass_env: ['CARGO_*', 'RUSTUP_*'] }),
				node: to_program('node'),
				wasm_pack: to_program('wasm-pack'),
				gzip: to_program('gzip'),
				print: (text) => console.log(text),
				print_error: (text) => console.error(text)
			},
			{ ...args, stray }
		);
	}
};
