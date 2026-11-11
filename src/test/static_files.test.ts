/// <reference types="node" />

import { symlink } from 'node:fs/promises';

import { fs, type FsDir } from '@fuzdev/gro/fs.ts';
import { assert, describe, test } from 'vitest';

import {
	respond_static,
	STATIC_DEFAULT_CONTENT_TYPE,
	to_static_content_type,
	to_static_segments,
	type StaticMount
} from '#lib/static_files.ts';

/**
 * A served tree: `root/served/` is the one mount, under `/served/`, and `root/secret.txt`
 * sits beside it, outside the mount but inside the temp dir.
 */
const to_tree = async (dir: FsDir): Promise<Array<StaticMount>> => {
	await dir.write('secret.txt', 'secret');
	await dir.mkdir('served/sub', { recursive: true });
	await dir.write('served/page.html', '<p>page</p>');
	await dir.write('served/app.js', 'export {};');
	await dir.write('served/app_bg.wasm', new Uint8Array([0, 97, 115, 109]));
	await dir.write('served/data.json', '{}');
	await dir.write('served/style.css', 'p {}');
	await dir.write('served/notes.txt', 'notes');
	await dir.write('served/sub/index.html', '<p>index</p>');
	await dir.write('served/a b.js', 'export {};');
	return [{ prefix: 'served', open: () => dir.dir('served').readonly() }];
};

const text = (body: Uint8Array | string | null): string | null =>
	body === null ? null : typeof body === 'string' ? body : new TextDecoder().decode(body);

describe('to_static_segments', () => {
	test('splits and decodes a path, dropping the query and fragment', () => {
		assert.deepEqual(to_static_segments('/a/b.js'), ['a', 'b.js']);
		assert.deepEqual(to_static_segments('/a/b.js?v=1#x'), ['a', 'b.js']);
		assert.deepEqual(to_static_segments('/a/a%20b.js'), ['a', 'a b.js']);
		assert.deepEqual(to_static_segments('/a/'), ['a', '']);
	});

	test('refuses every spelling of a climb or an escape, never normalizing it', () => {
		for (const target of [
			'/a/../b',
			'/..',
			'/a/./b',
			'/a/%2e%2e/b',
			'/a/%2E%2E/b',
			'/a/.%2e/b',
			'/a/%2e/b',
			'/a/..%2fb',
			'/a/%2f/b',
			'/a/%5c..%5cb',
			'/a/..\\b',
			'/a/b%00.js',
			'//etc/passwd',
			'/a//b',
			'a/b',
			'',
			'http://127.0.0.1/a',
			'/a/%zz',
			'/a/%'
		]) {
			assert.isNull(to_static_segments(target), target);
		}
	});
});

describe('to_static_content_type', () => {
	test('names the types the pages load with, and a default for the rest', () => {
		assert.strictEqual(to_static_content_type('a/b.wasm'), 'application/wasm');
		assert.strictEqual(to_static_content_type('b.js'), 'text/javascript; charset=utf-8');
		assert.strictEqual(to_static_content_type('b.html'), 'text/html; charset=utf-8');
		assert.strictEqual(to_static_content_type('b.json'), 'application/json');
		assert.strictEqual(to_static_content_type('b.css'), 'text/css; charset=utf-8');
		assert.strictEqual(to_static_content_type('b.txt'), STATIC_DEFAULT_CONTENT_TYPE);
		assert.strictEqual(to_static_content_type('a.js/b'), STATIC_DEFAULT_CONTENT_TYPE);
	});
});

describe('respond_static', () => {
	test('serves a file beneath a mount by its exact path, with its content type', async () => {
		await using dir = await fs.temp_dir('pglet_static_');
		const mounts = await to_tree(dir);
		for (const [target, type, body] of [
			['/served/page.html', 'text/html; charset=utf-8', '<p>page</p>'],
			['/served/app.js?v=2', 'text/javascript; charset=utf-8', 'export {};'],
			['/served/data.json', 'application/json', '{}'],
			['/served/style.css', 'text/css; charset=utf-8', 'p {}'],
			['/served/notes.txt', STATIC_DEFAULT_CONTENT_TYPE, 'notes'],
			['/served/a%20b.js', 'text/javascript; charset=utf-8', 'export {};'],
			['/served/sub/', 'text/html; charset=utf-8', '<p>index</p>']
		] as const) {
			const answer = await respond_static(mounts, 'GET', target);
			assert.strictEqual(answer.status, 200, target);
			assert.deepEqual(answer.headers, { 'Content-Type': type }, target);
			assert.strictEqual(text(answer.body), body, target);
		}
		const wasm = await respond_static(mounts, 'GET', '/served/app_bg.wasm');
		assert.strictEqual(wasm.headers['Content-Type'], 'application/wasm');
		assert.deepEqual([...(wasm.body as Uint8Array)], [0, 97, 115, 109]);
	});

	test('HEAD answers the headers without the body', async () => {
		await using dir = await fs.temp_dir('pglet_static_');
		const answer = await respond_static(await to_tree(dir), 'HEAD', '/served/app.js');
		assert.strictEqual(answer.status, 200);
		assert.strictEqual(answer.headers['Content-Type'], 'text/javascript; charset=utf-8');
		assert.isNull(answer.body);
	});

	test('refuses a climb out of the mount, written or encoded, before reading anything', async () => {
		await using dir = await fs.temp_dir('pglet_static_');
		let opened = 0;
		const [mount] = await to_tree(dir);
		const mounts = [{ prefix: mount!.prefix, open: () => (opened++, mount!.open()) }];
		for (const target of [
			'/served/../secret.txt',
			'/served/%2e%2e/secret.txt',
			'/served/%2e%2e%2fsecret.txt',
			'/served/..%5csecret.txt',
			'/served//secret.txt',
			'/served/sub/%2e%2e/page.html'
		]) {
			const answer = await respond_static(mounts, 'GET', target);
			assert.strictEqual(answer.status, 400, target);
			assert.notInclude(text(answer.body) ?? '', 'secret', target);
		}
		assert.strictEqual(opened, 0);
	});

	test('refuses a symlink out of the mount, absolute or climbing, and follows one inside it', async () => {
		await using dir = await fs.temp_dir('pglet_static_');
		const mounts = await to_tree(dir);
		await symlink(`${dir.path}/secret.txt`, `${dir.path}/served/absolute.txt`);
		await symlink('../secret.txt', `${dir.path}/served/climbing.txt`);
		await symlink('..', `${dir.path}/served/up`);
		await symlink('page.html', `${dir.path}/served/inside.html`);
		for (const target of [
			'/served/absolute.txt',
			'/served/climbing.txt',
			'/served/up/secret.txt'
		]) {
			const answer = await respond_static(mounts, 'GET', target);
			assert.strictEqual(answer.status, 403, target);
			assert.notInclude(text(answer.body) ?? '', 'secret', target);
		}
		const inside = await respond_static(mounts, 'GET', '/served/inside.html');
		assert.strictEqual(inside.status, 200);
		assert.strictEqual(text(inside.body), '<p>page</p>');
	});

	test('a path under no mount, a missing file or mount, or a bare directory is 404', async () => {
		await using dir = await fs.temp_dir('pglet_static_');
		const mounts = [
			...(await to_tree(dir)),
			{ prefix: 'missing/dir', open: () => dir.dir('missing/dir').readonly() }
		];
		for (const target of [
			'/',
			'/secret.txt',
			'/served',
			'/served/',
			'/served/sub',
			'/served/nope.js',
			'/served/page.html/',
			'/servedx/page.html',
			'/missing/dir/a.js'
		]) {
			assert.strictEqual((await respond_static(mounts, 'GET', target)).status, 404, target);
		}
	});

	test('only GET and HEAD are answered', async () => {
		await using dir = await fs.temp_dir('pglet_static_');
		const mounts = await to_tree(dir);
		for (const method of ['POST', 'PUT', 'DELETE', 'OPTIONS', 'get']) {
			const answer = await respond_static(mounts, method, '/served/page.html');
			assert.strictEqual(answer.status, 405, method);
			assert.strictEqual(answer.headers.Allow, 'GET, HEAD');
		}
	});

	test('a mount opened per request serves a directory made after the first request', async () => {
		await using dir = await fs.temp_dir('pglet_static_');
		const mounts = [{ prefix: 'late', open: () => dir.dir('late').readonly() }];
		assert.strictEqual((await respond_static(mounts, 'GET', '/late/a.js')).status, 404);
		await dir.mkdir('late');
		await dir.write('late/a.js', 'export {};');
		assert.strictEqual((await respond_static(mounts, 'GET', '/late/a.js')).status, 200);
	});
});
