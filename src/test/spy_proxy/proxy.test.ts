import { assert, describe, test } from 'vitest';

import {
	parse_spy_proxy_args,
	relay_spy_connection,
	run_spy_proxy,
	SPY_DEFAULTS,
	SPY_USAGE,
	type SpyConnection,
	type SpyRelayDeps
} from '#lib/spy_proxy/proxy.ts';

import {
	create_fake_connection,
	settle,
	SSL_REQUEST,
	to_message,
	to_startup_message,
	type FakeConnection
} from './spy_helpers.ts';

const NOW = '2026-01-02T03:04:05.000Z';

/** A relay between a fake client and a fake server, its capture lines and output collected. */
const start_relay = (options: { connect_fails?: boolean } = {}) => {
	const client = create_fake_connection('127.0.0.1:50000');
	const server = create_fake_connection('127.0.0.1:5432');
	const lines: Array<string> = [];
	const stdout: Array<string> = [];
	const stderr: Array<string> = [];
	const deps: SpyRelayDeps = {
		connect: async () => {
			if (options.connect_fails) throw new Error('connection refused');
			return server;
		},
		append: async (line) => {
			lines.push(line);
		},
		now: () => NOW,
		print: (text) => stdout.push(text),
		print_error: (text) => stderr.push(text)
	};
	const done = relay_spy_connection(deps, client);
	return { client, server, lines, stdout, stderr, done };
};

const types = (lines: Array<string>): Array<string> =>
	lines.map((line) => {
		const { direction, type } = JSON.parse(line);
		return `${direction} ${type}`;
	});

const concat_written = (connection: FakeConnection): Array<number> =>
	connection.written.flatMap((chunk) => [...chunk]);

describe('relay_spy_connection', () => {
	test('forwards every byte unchanged both ways and logs each message once whole', async () => {
		const relay = start_relay();
		const startup = to_startup_message({ user: 'u' });
		const query = to_message('Q', [{ str: 'SELECT 1' }]);
		// the startup message and a query split across three reads
		relay.client.send(startup.slice(0, 3));
		relay.client.send(new Uint8Array([...startup.slice(3), ...query.slice(0, 6)]));
		relay.client.send(query.slice(6));
		await settle();
		const ready = to_message('Z', [{ bytes: [0x49] }]);
		relay.server.send(new Uint8Array([...to_message('R', [{ i32: 0 }]), ...ready]));
		await settle();
		relay.client.end();
		await relay.done;

		assert.deepEqual(concat_written(relay.server), [...startup, ...query]);
		assert.deepEqual(concat_written(relay.client), [...to_message('R', [{ i32: 0 }]), ...ready]);
		assert.deepEqual(types(relay.lines), [
			'client StartupMessage',
			'client Query',
			'server AuthenticationOk',
			'server ReadyForQuery'
		]);
		for (const line of relay.lines) assert.isTrue(line.endsWith(`"timestamp":"${NOW}"}\n`), line);
		assert.deepEqual(JSON.parse(relay.lines[1]!), {
			direction: 'client',
			type: 'Query',
			length: query.length,
			payload: { query: 'SELECT 1' },
			timestamp: NOW
		});
		assert.deepEqual(relay.stdout, [
			'[CONNECT] Client 127.0.0.1:50000',
			'[CLIENT] StartupMessage {"version":"3.0","params":{"user":"u"}}',
			'[CLIENT] Query {"query":"SELECT 1"}',
			'[SERVER] AuthenticationOk {"auth":"Ok"}',
			'[SERVER] ReadyForQuery {"status":"I"}',
			'[DISCONNECT] Client 127.0.0.1:50000'
		]);
		assert.isTrue(relay.client.closed);
		assert.isTrue(relay.server.closed);
	});

	test("an SSL request's one-byte answer is logged, and the client's next message is its startup", async () => {
		const relay = start_relay();
		relay.client.send(SSL_REQUEST);
		await settle();
		relay.server.send(new Uint8Array([0x4e])); // 'N'
		await settle();
		relay.client.send(to_startup_message({ user: 'u' }));
		await settle();
		relay.server.end();
		await relay.done;
		assert.deepEqual(types(relay.lines), [
			'client SSLRequest',
			'server SSLResponse',
			'client StartupMessage'
		]);
		assert.deepEqual(JSON.parse(relay.lines[1]!).payload, { response: 'NoSSL' });
	});

	test('a startup message sent before the SSL answer ends the startup phase for good', async () => {
		const relay = start_relay();
		relay.client.send(new Uint8Array([...SSL_REQUEST, ...to_startup_message({ user: 'u' })]));
		await settle();
		relay.server.send(new Uint8Array([0x4e])); // 'N'
		await settle();
		relay.client.send(to_message('Q', [{ str: 'SELECT 1' }]));
		await settle();
		relay.server.end();
		await relay.done;
		assert.deepEqual(types(relay.lines), [
			'client SSLRequest',
			'client StartupMessage',
			'server SSLResponse',
			'client Query'
		]);
	});

	test('a server that refuses is logged, and the client is closed', async () => {
		const relay = start_relay({ connect_fails: true });
		await relay.done;
		assert.deepEqual(relay.stderr, [
			'[ERROR] Failed to connect to Postgres: Error: connection refused'
		]);
		assert.isTrue(relay.client.closed);
		assert.deepEqual(relay.lines, []);
	});

	test('a read error ends the connection and is printed', async () => {
		const relay = start_relay();
		const failing: SpyConnection = {
			...relay.server,
			chunks: {
				async *[Symbol.asyncIterator]() {
					throw new Error('kaboom');
				}
			}
		};
		const stderr: Array<string> = [];
		await relay_spy_connection(
			{
				connect: async () => failing,
				append: async () => {},
				now: () => NOW,
				print: () => {},
				print_error: (text) => stderr.push(text)
			},
			relay.client
		);
		assert.deepEqual(stderr, ['[ERROR] Server read: Error: kaboom']);
		assert.isTrue(relay.client.closed);
	});
});

describe('run_spy_proxy', () => {
	test('prints the bound port, not the one asked for, and relays each client', async () => {
		const client = create_fake_connection('127.0.0.1:50001');
		const stdout: Array<string> = [];
		const run = run_spy_proxy(
			{
				listen: async (port) => {
					assert.strictEqual(port, 0);
					return {
						port: 40123,
						connections: {
							async *[Symbol.asyncIterator]() {
								yield client;
							}
						}
					};
				},
				connect: async () => create_fake_connection('127.0.0.1:5432'),
				append: async () => {},
				now: () => NOW,
				print: (text) => stdout.push(text),
				print_error: () => {}
			},
			{
				listen_port: 0,
				server_host: '127.0.0.1',
				server_port: 5432,
				output: 'tests/captures/x.jsonl'
			}
		);
		await settle();
		client.end();
		await run;
		assert.deepEqual(stdout.slice(0, 4), [
			'Spy proxy listening on 127.0.0.1:40123',
			'Forwarding to 127.0.0.1:5432',
			'Logging to tests/captures/x.jsonl',
			'[CONNECT] Client 127.0.0.1:50001'
		]);
	});
});

describe('parse_spy_proxy_args', () => {
	test('defaults, and the three positionals', () => {
		assert.deepEqual(parse_spy_proxy_args([]), { ok: true, args: { ...SPY_DEFAULTS } });
		assert.deepEqual(parse_spy_proxy_args(['0', '5433', 'tests/captures/a/b.jsonl']), {
			ok: true,
			args: { listen_port: 0, server_port: 5433, output: 'tests/captures/a/b.jsonl' }
		});
	});

	test('refuses ports out of range or not numbers, and a server port of 0', () => {
		for (const [args, error] of [
			[['65536'], 'listen_port must be 0 to 65535, got "65536"'],
			[['x'], 'listen_port must be 0 to 65535, got "x"'],
			[['-1'], 'listen_port must be 0 to 65535, got "-1"'],
			[['5434', '0'], 'server_port must be 1 to 65535, got "0"'],
			[['5434', '5432.5'], 'server_port must be 1 to 65535, got "5432.5"']
		] as const) {
			assert.deepEqual(parse_spy_proxy_args(args), { ok: false, error: `${error}\n${SPY_USAGE}` });
		}
	});

	test('refuses a capture outside tests/captures/, and a fourth argument', () => {
		for (const output of [
			'session.jsonl',
			'/tmp/x.jsonl',
			'tests/captures/',
			'tests/captures/../../x.jsonl',
			'tests/captures/./x.jsonl',
			'tests/captures//x.jsonl',
			'tests/capturesx.jsonl'
		]) {
			assert.deepEqual(
				parse_spy_proxy_args(['5434', '5432', output]),
				{
					ok: false,
					error: `the capture goes in tests/captures/, got "${output}"\n${SPY_USAGE}`
				},
				output
			);
		}
		assert.isFalse(parse_spy_proxy_args(['1', '2', 'tests/captures/a', 'd']).ok);
	});
});
