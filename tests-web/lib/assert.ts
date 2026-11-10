/**
 * Assertions the suites share beyond vitest's `assert`.
 *
 * @module
 */
import {assert, expect} from 'vitest';

/**
 * Asserts that `actual` deep-equals `expected`, prototypes included: a class instance never
 * equals a plain object, nor a `Buffer` a `Uint8Array`, nor an array hole an `undefined` element.
 * `assert.deepEqual` compares objects by their type tag alone, so it would pass a result row that
 * stopped being a plain object.
 *
 * @param actual - the value under test
 * @param expected - the value it must equal
 * @param message - prefixes the failure message
 */
export const assert_equal = (actual: unknown, expected: unknown, message?: string): void => {
	expect(actual, message).toStrictEqual(expected);
};

/**
 * Asserts that `fn` returns a promise that rejects, and returns the rejection. A function that
 * throws instead of returning a promise fails, as does a promise that resolves.
 *
 * @param fn - called once; must return a thenable
 * @param ErrorClass - when given, the rejection must be an `Error` and an instance of it
 * @param message_includes - when given, the rejection's message must include it
 * @param message - prefixes the failure message
 * @returns the rejection
 */
export async function assert_rejects(fn: () => PromiseLike<unknown>, message?: string): Promise<unknown>;
export async function assert_rejects<E extends Error = Error>(
	fn: () => PromiseLike<unknown>,
	ErrorClass: new (...args: Array<any>) => E,
	message_includes?: string,
	message?: string,
): Promise<E>;
export async function assert_rejects(
	fn: () => PromiseLike<unknown>,
	class_or_message?: (new (...args: Array<any>) => Error) | string,
	message_includes?: string,
	message?: string,
): Promise<unknown> {
	const ErrorClass = typeof class_or_message === 'string' ? undefined : class_or_message;
	const prefix = (typeof class_or_message === 'string' ? class_or_message : message) ?? '';
	const failure = (text: string): string => (prefix ? `${prefix}: ${text}` : text);
	let promise: PromiseLike<unknown>;
	try {
		promise = fn();
	} catch (error) {
		assert.fail(failure(`the function threw instead of returning a promise: ${String(error)}`));
	}
	assert(
		typeof (promise as {then?: unknown} | null)?.then === 'function',
		failure('the function returned no promise'),
	);
	try {
		await promise;
	} catch (error) {
		if (ErrorClass) {
			assert.instanceOf(error, Error, failure('the rejection is not an Error'));
			assert.instanceOf(error, ErrorClass, failure(`the rejection is not a ${ErrorClass.name}`));
		}
		if (message_includes !== undefined) {
			assert.include((error as Error).message, message_includes, failure('the rejection message'));
		}
		return error;
	}
	assert.fail(failure('the promise resolved; expected it to reject'));
}
