/**
 * The browser entry's transaction-concurrency contract. `browser.js` hands the adapter no
 * async-context store — a browser has no `AsyncLocalStorage` — so a `transaction()`, `fork()` or
 * `close()` made while a callback runs cannot be told nested from concurrent, and all are refused
 * loudly. The Node entry (`dx_test.ts`) tells them apart and queues the concurrent ones; this pins
 * that the browser entry keeps the refusal rather than inheriting the queue, which would hang a
 * nested call.
 *
 * Runs the entry under Node, imported by path: what it proves is the entry's wiring, not a browser.
 */
import {readFile} from 'node:fs/promises';
import {assert, test} from 'vitest';
import {assert_equal} from '../lib/assert.ts';
import {create_pglet_db, init_sync} from '../../crates/pglet_wasm/pkg-npm/browser.js';

const PKG = new URL('../../crates/pglet_wasm/pkg-npm/', import.meta.url);
init_sync({wasm: await readFile(new URL('pglet_wasm_bg.wasm', PKG))});

/** `promise`, or a rejection naming `what` once `ms` pass; the timer is cleared either way. */
const within = <T,>(promise: Promise<T>, what: string, ms = 2000): Promise<T> => {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<never>((_, reject) => {
		timer = setTimeout(() => reject(new Error(`timed out: ${what}`)), ms);
	});
	return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
};

test('the browser entry imports no node: module', async () => {
	const source = await readFile(new URL('browser.js', PKG), 'utf8');
	assert(!source.includes('node:'), 'browser.js must not import a node: built-in');
});

test('browser entry: a concurrent top-level transaction() is refused, naming both readings', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	let release = () => {};
	const first_callback_running = new Promise<void>((r) => {
		release = r;
	});
	const first = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		release();
		await new Promise((r) => setTimeout(r, 10));
		await tx.query('INSERT INTO t VALUES (2)');
	});
	await first_callback_running;
	const second = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (3)');
	});
	const [a, b] = await within(Promise.allSettled([first, second]), 'both transactions');
	assert_equal(a.status, 'fulfilled');
	assert_equal(b.status, 'rejected');
	const err = (b as PromiseRejectedResult).reason;
	assert.instanceOf(err, Error);
	assert(err.message.includes('a transaction() callback is already running'), err.message);
	assert(err.message.includes('top-level'), err.message);
	assert(err.message.includes('AsyncLocalStorage'), err.message);
	assert_equal((await db.query<{id: number}>('SELECT id FROM t ORDER BY id')).rows.map((r) => r.id), [1, 2]);
	await db.close();
});

test('browser entry: a nested transaction() is refused, never deadlocks', async () => {
	const db = create_pglet_db();
	const err = await within(
		db.transaction(async () => db.transaction(async () => 1)).then(() => null, (e) => e),
		'the outer transaction',
	);
	assert.instanceOf(err, Error);
	assert(err.message.includes('a transaction() callback is already running'), err.message);
	await db.close();
});

test('browser entry: fork() and close() from outside a running callback are refused', async () => {
	const db = create_pglet_db();
	await db.query('CREATE TABLE t (id INT)');
	let release!: () => void;
	const gate = new Promise<void>((r) => (release = r));
	let entered!: () => void;
	const inside = new Promise<void>((r) => (entered = r));
	const running = db.transaction(async (tx) => {
		await tx.query('INSERT INTO t VALUES (1)');
		entered();
		await gate;
	});
	await inside;
	for (const call of [() => db.fork(), () => db.close()]) {
		const err = await call().then(() => null, (e) => e);
		assert.instanceOf(err, Error);
		assert(err.message.includes('cannot be told apart'), err.message);
		assert_equal((err as {code?: string}).code, '25001');
	}
	release();
	await within(running, 'the transaction');
	assert_equal((await db.query('SELECT count(*)::int AS c FROM t')).rows[0].c, 1); // still open
	await db.close();
});
