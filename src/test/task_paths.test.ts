import { fs } from '@fuzdev/gro/fs.ts';
import { assert, describe, test } from 'vitest';

import { to_task_checkout_path } from '#lib/task_paths.ts';

const tasks_url = new URL('../tasks/', import.meta.url);

describe('the tasks', () => {
	test('each that reaches the checkout opens it from its own location, never the working directory', async () => {
		const tasks = fs.project.dir('src/tasks').readonly();
		let rooted = 0;
		for await (const { name } of tasks.list()) {
			if (!name.endsWith('.task.ts')) continue;
			const source = await tasks.read_text(name);
			assert.notInclude(source, 'fs.project', name);
			assert.notInclude(source, 'process.cwd', name);
			if (!source.includes('fs.open_dir(')) continue;
			assert.include(source, 'fs.open_dir(to_task_checkout_path(import.meta.url))', name);
			rooted++;
		}
		assert.isAtLeast(rooted, 1);
	});

	test('each task module sits where the helper expects it', async () => {
		// this file is `<checkout>/src/test/`, so the checkout is two dirs up
		const checkout = decodeURIComponent(new URL('../..', import.meta.url).pathname).replace(
			/\/$/,
			''
		);
		for await (const { name } of fs.project.dir('src/tasks').list()) {
			assert.strictEqual(to_task_checkout_path(new URL(name, tasks_url).href), checkout, name);
		}
	});
});

describe('to_task_checkout_path', () => {
	test("the checkout, from a task module's URL", () => {
		assert.strictEqual(
			to_task_checkout_path('file:///home/u/dev/pglet/src/tasks/lint.task.ts'),
			'/home/u/dev/pglet'
		);
		assert.strictEqual(
			to_task_checkout_path('file:///w/my%20dev/pglet/src/tasks/a.task.ts'),
			'/w/my dev/pglet'
		);
		assert.strictEqual(to_task_checkout_path('file:///src/tasks/a.task.ts'), '/');
	});

	test("refuses a URL that isn't a task module's in src/tasks/", () => {
		for (const bad of [
			'https://x/pglet/src/tasks/lint.task.ts',
			'file:///home/u/dev/pglet/src/lib/lint.ts',
			'file:///home/u/dev/pglet/src/tasks/nested/lint.task.ts',
			'file:///home/u/dev/pglet/lint.task.ts'
		]) {
			assert.throws(() => to_task_checkout_path(bad), /not a task module/, bad);
		}
	});
});
