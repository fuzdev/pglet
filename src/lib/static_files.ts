/**
 * The static files the browser pages load, answered from directory handles: the
 * request half of the static server the browser tests and `wasm:serve` run
 * (`static_server.ts` is its Node-only listener). Pure over its deps: the
 * mounts are `FsReadDir` openers a root builds, so the answers are tested over
 * temp dirs without a socket.
 *
 * A request reaches only a file beneath one mount, by its exact path. A path
 * with a `.` or `..` segment, an empty segment, a backslash or a NUL, written
 * or percent-encoded, is refused before any mount is asked, and the mount's
 * handle resolves what is left beneath its own directory, refusing an absolute
 * symlink and one that climbs above it.
 *
 * @module
 */

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import { is_fs_error } from '@fuzdev/gro/fs_error.ts';

/** One directory served under a URL prefix. */
export interface StaticMount {
	/** The URL path's leading segments that select this mount, like `crates/pglet_wasm/pkg`. */
	prefix: string;
	/**
	 * Opens the mount's directory, once per request, so a directory built while the server
	 * runs is served. A `not_found` throw is answered `404`.
	 */
	open: () => FsReadDir;
}

/** What the server writes back. */
export interface StaticResponse {
	status: number;
	headers: Record<string, string>;
	/** The body, or `null` for a `HEAD` request. */
	body: Uint8Array | string | null;
}

/**
 * The content types the pages need, by extension: a module script and a module worker
 * load only with a JavaScript type, and `WebAssembly.instantiateStreaming`, which the
 * wasm-bindgen glue's `init()` takes, only with `application/wasm`.
 */
export const STATIC_CONTENT_TYPES: Readonly<Record<string, string>> = {
	'.html': 'text/html; charset=utf-8',
	'.js': 'text/javascript; charset=utf-8',
	'.wasm': 'application/wasm',
	'.json': 'application/json',
	'.css': 'text/css; charset=utf-8'
};

/** The type of a file no entry of `STATIC_CONTENT_TYPES` names. */
export const STATIC_DEFAULT_CONTENT_TYPE = 'application/octet-stream';

/** The file a request for a directory, a path ending in `/`, is answered with. */
export const STATIC_INDEX = 'index.html';

/** The content type of `path`, by its extension. */
export const to_static_content_type = (path: string): string => {
	const slash = path.lastIndexOf('/');
	const dot = path.lastIndexOf('.');
	return (dot > slash && STATIC_CONTENT_TYPES[path.slice(dot)]) || STATIC_DEFAULT_CONTENT_TYPE;
};

/**
 * A request target's path as segments, decoded, or `null` when it's refused: not
 * starting with `/`, malformed percent-encoding, or a segment that is empty (but the
 * last, for a directory), `.` or `..`, or holds a `/`, a `\` or a NUL once decoded.
 * The query and fragment are dropped. Nothing is normalized: a refused segment is
 * refused, never resolved.
 *
 * @param target - the request line's target, like `/crates/pglet_wasm/pkg/pglet_wasm.js?v=1`
 * @returns the segments, the last `''` when the path ends in `/`
 */
export const to_static_segments = (target: string): Array<string> | null => {
	const end = target.search(/[?#]/);
	const path = end === -1 ? target : target.slice(0, end);
	if (!path.startsWith('/')) return null;
	const raw = path.slice(1).split('/');
	const segments: Array<string> = [];
	for (let i = 0; i < raw.length; i++) {
		let segment: string;
		try {
			segment = decodeURIComponent(raw[i]!);
		} catch {
			return null;
		}
		const last = i === raw.length - 1;
		if (segment === '' && !last) return null;
		if (segment === '.' || segment === '..' || /[/\\\0]/.test(segment)) return null;
		segments.push(segment);
	}
	return segments;
};

const text_response = (status: number, text: string, head: boolean): StaticResponse => ({
	status,
	headers: { 'Content-Type': 'text/plain; charset=utf-8' },
	body: head ? null : text
});

/**
 * Answers one request from `mounts`: `GET` and `HEAD` of a file beneath a mount, by its
 * exact path, with the content type its extension names. A path ending in `/` asks for
 * that directory's `STATIC_INDEX`.
 *
 * - `400` for a target `to_static_segments` refuses
 * - `403` for a path the mount's handle refuses as outside it (a symlink out)
 * - `404` for a path under no mount, a missing file or mount directory, and a directory
 *   asked for without its trailing `/`
 * - `405` for any other method
 *
 * @param mounts - the directories served, the first whose prefix matches answering
 * @param method - the request's method
 * @param target - the request line's target
 * @throws Error - an `FsError` other than those above (a permission or I/O failure)
 */
export const respond_static = async (
	mounts: ReadonlyArray<StaticMount>,
	method: string,
	target: string
): Promise<StaticResponse> => {
	const head = method === 'HEAD';
	if (method !== 'GET' && !head) {
		return {
			...text_response(405, 'Method Not Allowed', head),
			headers: {
				'Content-Type': 'text/plain; charset=utf-8',
				Allow: 'GET, HEAD'
			}
		};
	}
	const segments = to_static_segments(target);
	if (segments === null) return text_response(400, 'Bad Request', head);
	for (const mount of mounts) {
		const prefix = mount.prefix.split('/');
		if (segments.length <= prefix.length || prefix.some((p, i) => segments[i] !== p)) continue;
		const rest = segments.slice(prefix.length);
		const path =
			rest.at(-1) === '' ? [...rest.slice(0, -1), STATIC_INDEX].join('/') : rest.join('/');
		try {
			const dir = mount.open();
			const info = await dir.stat(path);
			if (info === null || info.kind !== 'file') return text_response(404, 'Not Found', head);
			const body = await dir.read(path);
			return {
				status: 200,
				headers: { 'Content-Type': to_static_content_type(path) },
				body: head ? null : body
			};
		} catch (error) {
			if (is_fs_error(error, 'outside_root')) return text_response(403, 'Forbidden', head);
			if (is_fs_error(error, 'not_found') || is_fs_error(error, 'not_a_directory')) {
				return text_response(404, 'Not Found', head);
			}
			throw error;
		}
	}
	return text_response(404, 'Not Found', head);
};
