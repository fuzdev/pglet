/// <reference types="node" />

import { isBuiltin } from 'node:module';

import { fs_search } from '@fuzdev/gro/fs_search.ts';
import { fs } from '@fuzdev/gro/fs.ts';
import ts from 'typescript';
import { assert, test } from 'vitest';

/** The one library module on `node:*`, the spy proxy's sockets and capture file. */
const NODE_ONLY = ['spy_proxy/node_net.ts'];

/** gro's host modules, the capabilities library code takes as deps. */
const HOST_MODULE = /^@fuzdev\/gro\/(fs|spawn|process|net|watch)\.ts$/;

interface LibImport {
	path: string;
	/** The module named, or null when it's computed, which only the Node-only module may do. */
	specifier: string | null;
	type_only: boolean;
}

/** A call's module argument: its text when it's a literal, else null. */
const to_call_specifier = (call: ts.CallExpression): string | null => {
	const arg = call.arguments[0];
	return arg !== undefined && ts.isStringLiteralLike(arg) ? arg.text : null;
};

/**
 * Every import, re-export, `require` and `process.getBuiltinModule` in `src/lib`,
 * parsed, so text inside a string or a comment is no import.
 */
const lib_imports = async (): Promise<Array<LibImport>> => {
	const lib = fs.project.dir('src/lib').readonly();
	const imports: Array<LibImport> = [];
	for (const file of await fs_search(lib)) {
		if (!file.path.endsWith('.ts')) continue;
		const source = ts.createSourceFile(
			file.path,
			await lib.read_text(file.path),
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

test("library code imports Node's builtins only in the one Node-only module", async () => {
	// `isBuiltin` matches a bare builtin specifier (`fs/promises`) as well as a `node:` one
	const importers = new Set(
		(await lib_imports())
			.filter((i) => i.specifier === null || isBuiltin(i.specifier))
			.map((i) => i.path)
	);
	assert.deepEqual([...importers].sort(), NODE_ONLY);
});

test("library code imports gro's host modules for their types only, taking the capabilities as deps", async () => {
	const imports = await lib_imports();
	assert.isTrue(
		imports.some((i) => i.specifier !== null && HOST_MODULE.test(i.specifier)),
		'the check sees the type imports'
	);
	for (const i of imports) {
		if (i.specifier !== null && HOST_MODULE.test(i.specifier))
			assert.isTrue(i.type_only, `${i.path} imports ${i.specifier}`);
	}
});
