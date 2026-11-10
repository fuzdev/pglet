import { assert, describe, test } from 'vitest';

import {
	parse_bind,
	parse_parameter_description,
	parse_parse,
	parse_spy_message
} from '#lib/spy_proxy/messages.ts';

import { SSL_REQUEST, to_message, to_payload, to_startup_message } from './spy_helpers.ts';

const NOW = (): string => '2026-01-02T03:04:05.000Z';

// the extended-protocol parsers, over hand-crafted payloads (the body after the type byte and length)
describe('the extended-protocol parsers', () => {
	test('parse_parse: name, query, and untyped params', () => {
		// tokio-postgres sends empty param-type OIDs (server infers)
		const p = parse_parse(to_payload([{ str: 's1' }, { str: 'SELECT $1::int4' }, { i16: 0 }]));
		assert.deepEqual(p.statement, 's1');
		assert.deepEqual(p.query, 'SELECT $1::int4');
		assert.deepEqual(p.paramTypeOids, []);
	});

	test('parse_parse: declared param OIDs', () => {
		const p = parse_parse(
			to_payload([
				{ str: '' },
				{ str: 'SELECT $1' },
				{ i16: 2 },
				{ i32: 23 }, // int4
				{ i32: 25 } // text
			])
		);
		assert.deepEqual(p.paramTypeOids, [23, 25]);
	});

	test('parse_parameter_description: inferred OIDs', () => {
		// the server's inferred types: uuid[] then text
		const d = parse_parameter_description(
			to_payload([
				{ i16: 2 },
				{ i32: 2951 }, // uuid[]
				{ i32: 25 } // text
			])
		);
		assert.deepEqual(d.paramTypeOids, [2951, 25]);
	});

	test('parse_bind: formats, hex values, and NULL', () => {
		const b = parse_bind(
			to_payload([
				{ str: '' }, // portal
				{ str: 's1' }, // statement
				{ i16: 1 }, // one format code...
				{ i16: 1 }, // ...binary
				{ i16: 2 }, // two params
				{ i32: 4 }, // param 1 length
				{ bytes: [0x00, 0x00, 0x00, 0x2a] }, // 42 (int4)
				{ i32: -1 } // param 2 is NULL
			])
		);
		assert.deepEqual(b.statement, 's1');
		assert.deepEqual(b.paramFormats, [1]);
		assert.deepEqual(b.params, ['0000002a', null]);
	});
});

describe('parse_spy_message', () => {
	test("a client's startup message, with its params and its length", () => {
		const bytes = to_startup_message({ user: 'postgres', database: 'db' });
		const parsed = parse_spy_message(bytes, 'client', true, NOW);
		assert.deepEqual(parsed, {
			message: {
				direction: 'client',
				type: 'StartupMessage',
				length: bytes.length,
				payload: { version: '3.0', params: { user: 'postgres', database: 'db' } },
				timestamp: NOW()
			},
			consumed: bytes.length
		});
	});

	test('an SSL request in the startup phase, and the same bytes outside it', () => {
		assert.strictEqual(
			parse_spy_message(SSL_REQUEST, 'client', true, NOW)?.message.type,
			'SSLRequest'
		);
		// outside the startup phase the first byte is a type: 0x00 is no client message, and the
		// 4-byte length read from bytes 1..5 is past the buffer, so nothing parses yet
		assert.isNull(parse_spy_message(SSL_REQUEST, 'client', false, NOW));
	});

	test('a message split across reads parses only once whole, and consumes only itself', () => {
		const query = to_message('Q', [{ str: 'SELECT 1' }]);
		const sync = to_message('S', []);
		assert.isNull(parse_spy_message(query.slice(0, 4), 'client', false, NOW));
		assert.isNull(parse_spy_message(query.slice(0, query.length - 1), 'client', false, NOW));
		const both = new Uint8Array([...query, ...sync]);
		const parsed = parse_spy_message(both, 'client', false, NOW)!;
		assert.deepEqual(parsed.message.payload, { query: 'SELECT 1' });
		assert.strictEqual(parsed.message.length, query.length);
		assert.strictEqual(parsed.consumed, query.length);
	});

	test('the server messages a capture reads', () => {
		const server = (type: string, parts: Parameters<typeof to_message>[1]) =>
			parse_spy_message(to_message(type, parts), 'server', false, NOW)!.message;
		assert.deepEqual(server('R', [{ i32: 0 }]).payload, { auth: 'Ok' });
		assert.deepEqual(server('R', [{ i32: 7 }]).payload, { auth: 7 });
		assert.deepEqual(server('K', [{ i32: 42 }, { i32: -7 }]).payload, { pid: 42, secret: -7 });
		assert.deepEqual(server('S', [{ str: 'TimeZone' }, { str: 'UTC' }]).payload, {
			name: 'TimeZone',
			value: 'UTC'
		});
		assert.deepEqual(server('Z', [{ bytes: [0x49] }]).payload, { status: 'I' });
		assert.deepEqual(server('C', [{ str: 'SELECT 1' }]).payload, { tag: 'SELECT 1' });
		assert.deepEqual(
			server('T', [
				{ i16: 1 },
				{ str: 'id' },
				{ i32: 16384 },
				{ i16: 1 },
				{ i32: 23 },
				{ i16: 4 },
				{ i32: -1 },
				{ i16: 0 }
			]).payload,
			{
				fields: [
					{
						name: 'id',
						tableOid: 16384,
						columnAttr: 1,
						typeOid: 23,
						typeLen: 4,
						typeMod: -1,
						format: 0
					}
				]
			}
		);
		assert.deepEqual(
			server('D', [{ i16: 2 }, { i32: 1 }, { bytes: [0x37] }, { i32: -1 }]).payload,
			{ values: ['7', null] }
		);
		assert.deepEqual(
			server('E', [
				{ bytes: [0x53] },
				{ str: 'ERROR' },
				{ bytes: [0x43] },
				{ str: '42P01' },
				{ bytes: [0x7a] },
				{ str: 'odd' },
				{ bytes: [0] }
			]).payload,
			{ severity: 'ERROR', code: '42P01', z: 'odd' }
		);
		assert.deepEqual(server('2', []), {
			direction: 'server',
			type: 'BindComplete',
			length: 5,
			payload: { raw_length: 0 },
			timestamp: NOW()
		});
		assert.strictEqual(server('?', []).type, 'Unknown(0x3f)');
	});
});
