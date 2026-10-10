/**
 * The registry identity of `@fuzdev/pglet-wasm` — the `package.json` fields that say what the
 * package is and who answers for it, shared by every script that writes or checks them:
 * `scripts/patch_npm_package.ts` assembles the published `package.json` from this module, and
 * `tests-web/npm/package_test.ts` and `scripts/test_npm_node.mjs` assert the assembled file
 * carries it.
 *
 * `engines` is the load-bearing field. Its floor is Node 22: Node 20 reached end of life in
 * 2026-04, and the sibling wasm packages (`@fuzdev/tsv-wasm`) already sit at 22.
 * `scripts/test_npm_node.mjs` reads this floor and refuses to run on a Node below it, so the
 * Node leg of `test:npm` cannot pass on a runtime the package does not support.
 *
 * Per-package fields (`description`, `keywords`, `files`, `exports`, `sideEffects`,
 * `peerDependencies`) stay in the patch script beside the files they describe.
 */
export const NPM_SHARED_METADATA = {
	name: '@fuzdev/pglet-wasm',
	license: 'MIT',
	homepage: 'https://github.com/fuzdev/pglet',
	author: {
		name: 'Ryan Atkinson',
		email: 'mail@ryanatkn.com',
		url: 'https://www.ryanatkn.com/',
	},
	repository: {
		type: 'git',
		url: 'git+https://github.com/fuzdev/pglet.git',
	},
	bugs: 'https://github.com/fuzdev/pglet/issues',
	funding: 'https://www.ryanatkn.com/funding',
	engines: {node: '>=22'},
	// structural, so a hand `npm publish` of a scoped package can never land restricted
	publishConfig: {access: 'public'},
} as const;
