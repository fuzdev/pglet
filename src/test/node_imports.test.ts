/// <reference types="node" />

import { isBuiltin } from 'node:module';

import { fs_search } from '@fuzdev/gro/fs_search.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import ts from 'typescript';
import { assert, test } from 'vitest';

/**
 * The library modules on `node:*`: the spy proxy's sockets and capture file, and the static
 * server's HTTP listener.
 */
const NODE_ONLY = ['spy_proxy/node_net.ts', 'static_server.ts'];

/**
 * The benches on `node:*`, plain Node scripts whose timed regions carry no gro loader and
 * which Bun runs too (`wasm_compare.ts`): the scripts and the helpers that spawn, read files
 * or time. The rest of `benches/lib/` (the statistics, the ratios, the preflight) is pure.
 */
const BENCHES_NODE = [
	'exit_proof.ts',
	'index_accel.ts',
	'lib/gzip_size.ts',
	'lib/pglet_package.ts',
	'lib/pglite_install.ts',
	'lib/results.ts',
	'lib/vitest_file_times.mjs',
	'memory.ts',
	'size_compare.ts',
	'startup.ts',
	'wasm_compare.ts',
	'wasm_init.ts'
];

/** gro's host modules, the capabilities library code takes as deps. */
const HOST_MODULE = /^@fuzdev\/gro\/(fs|spawn|process|net|watch)\.ts$/;

interface ModuleImport {
	path: string;
	/** The module named, or null when it's computed, which only a Node-only module may do. */
	specifier: string | null;
	type_only: boolean;
}

/** A call's module argument: its text when it's a literal, else null. */
const to_call_specifier = (call: ts.CallExpression): string | null => {
	const arg = call.arguments[0];
	return arg !== undefined && ts.isStringLiteralLike(arg) ? arg.text : null;
};

/**
 * Every import, re-export, `require` and `process.getBuiltinModule` in the modules beneath
 * `dir` whose names end in one of `extensions`, parsed, so text inside a string or a comment
 * is no import. `node_modules` beneath it is skipped.
 */
const module_imports = async (
	dir: string,
	extensions: ReadonlyArray<string>
): Promise<Array<ModuleImport>> => {
	const root = fs.project.dir(dir).readonly();
	const imports: Array<ModuleImport> = [];
	for (const file of await fs_search(root)) {
		if (file.path.startsWith('node_modules/')) continue;
		if (!extensions.some((extension) => file.path.endsWith(extension))) continue;
		const source = ts.createSourceFile(
			file.path,
			await root.read_text(file.path),
			ts.ScriptTarget.Latest
		);
		const visit = (node: ts.Node): void => {
			if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) {
				const clause = node.importClause;
				const type_only =
					clause !== undefined &&
					(clause.isTypeOnly ||
						(clause.name === undefined &&
							clause.namedBindings !== undefined &&
							ts.isNamedImports(clause.namedBindings) &&
							clause.namedBindings.elements.every((e) => e.isTypeOnly)));
				imports.push({ path: file.path, specifier: node.moduleSpecifier.text, type_only });
			} else if (
				ts.isExportDeclaration(node) &&
				node.moduleSpecifier !== undefined &&
				ts.isStringLiteral(node.moduleSpecifier)
			) {
				const clause = node.exportClause;
				const type_only =
					node.isTypeOnly ||
					(clause !== undefined &&
						ts.isNamedExports(clause) &&
						clause.elements.every((e) => e.isTypeOnly));
				imports.push({ path: file.path, specifier: node.moduleSpecifier.text, type_only });
			} else if (
				ts.isCallExpression(node) &&
				(node.expression.kind === ts.SyntaxKind.ImportKeyword ||
					(ts.isIdentifier(node.expression) && node.expression.text === 'require') ||
					(ts.isPropertyAccessExpression(node.expression) &&
						node.expression.name.text === 'getBuiltinModule'))
			) {
				imports.push({ path: file.path, specifier: to_call_specifier(node), type_only: false });
			}
			ts.forEachChild(node, visit);
		};
		visit(source);
	}
	return imports;
};

/** The modules beneath `dir` that import one of Node's builtins, or a computed specifier. */
const node_importers = async (
	dir: string,
	extensions: ReadonlyArray<string>
): Promise<Array<string>> => {
	// `isBuiltin` matches a bare builtin specifier (`fs/promises`) as well as a `node:` one
	const importers = new Set(
		(await module_imports(dir, extensions))
			.filter((i) => i.specifier === null || isBuiltin(i.specifier))
			.map((i) => i.path)
	);
	return [...importers].sort();
};

test("library code imports Node's builtins only in the Node-only modules", async () => {
	assert.deepEqual(await node_importers('src/lib', ['.ts']), NODE_ONLY);
});

test("the benches import Node's builtins only in the scripts named for it", async () => {
	assert.deepEqual(await node_importers('benches', ['.ts', '.mjs']), BENCHES_NODE);
});

test("library code imports gro's host modules for their types only, taking the capabilities as deps", async () => {
	const imports = await module_imports('src/lib', ['.ts']);
	assert.isTrue(
		imports.some((i) => i.specifier !== null && HOST_MODULE.test(i.specifier)),
		'the check sees the type imports'
	);
	for (const i of imports) {
		if (i.specifier !== null && HOST_MODULE.test(i.specifier))
			assert.isTrue(i.type_only, `${i.path} imports ${i.specifier}`);
	}
});
