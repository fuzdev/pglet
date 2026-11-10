/**
 * Where this checkout is, found from a task module's own `import.meta.url`, so
 * a task never derives the repo root from the working directory.
 *
 * @module
 */

/** Where a task module sits in this checkout, `<checkout>/src/tasks/<name>.task.ts`. */
const TASK_PATH = /\/src\/tasks\/[^/]+\.task\.ts$/;

/**
 * This checkout's root, from a task module's own `import.meta.url`, so it names
 * the checkout wherever the task is run from.
 *
 * @param task_url - a task module's `import.meta.url`, in `<checkout>/src/tasks/`
 * @throws Error - when `task_url` isn't a file URL of a task module in `src/tasks/`
 */
export const to_task_checkout_path = (task_url: string): string => {
	const url = new URL(task_url);
	const path = decodeURIComponent(url.pathname);
	const task = TASK_PATH.exec(path);
	if (url.protocol !== 'file:' || task === null) {
		throw new Error(`not a task module in src/tasks/: ${task_url}`);
	}
	return path.slice(0, task.index) || '/';
};
