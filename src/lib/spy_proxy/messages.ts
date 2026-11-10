/**
 * The Postgres wire-protocol parsers behind the spy proxy: each turns one
 * message's bytes into the record a capture line holds. Pure, no I/O. The
 * payload keys (`paramTypeOids`, `tableOid`, …) are the capture format's, which
 * `tests/captures/*.jsonl` and the notes authored from them read, so they keep
 * their camelCase.
 *
 * @module
 */

/** Which side of the proxy sent a message. */
export type SpyDirection = 'client' | 'server';

/** One capture line, as `JSON.stringify` writes it, keys in this order. */
export interface SpyMessage {
	direction: SpyDirection;
	type: string;
	length: number;
	payload: unknown;
	timestamp: string;
}

/** A parsed message and how many bytes of the buffer it took. */
export interface SpyParsed {
	message: SpyMessage;
	consumed: number;
}

const CLIENT_TYPES: Record<number, string> = {
	0x51: 'Query', // 'Q'
	0x50: 'Parse', // 'P'
	0x42: 'Bind', // 'B'
	0x44: 'Describe', // 'D'
	0x45: 'Execute', // 'E'
	0x48: 'Flush', // 'H'
	0x53: 'Sync', // 'S'
	0x58: 'Terminate', // 'X'
	0x63: 'CopyDone', // 'c'
	0x64: 'CopyData', // 'd'
	0x66: 'CopyFail', // 'f'
	0x70: 'PasswordMessage' // 'p'
};

const SERVER_TYPES: Record<number, string> = {
	0x31: 'ParseComplete', // '1'
	0x32: 'BindComplete', // '2'
	0x33: 'CloseComplete', // '3'
	0x41: 'NotificationResponse', // 'A'
	0x43: 'CommandComplete', // 'C'
	0x44: 'DataRow', // 'D'
	0x45: 'ErrorResponse', // 'E'
	0x47: 'CopyInResponse', // 'G'
	0x48: 'CopyOutResponse', // 'H'
	0x49: 'EmptyQueryResponse', // 'I'
	0x4b: 'BackendKeyData', // 'K'
	0x4e: 'NoticeResponse', // 'N'
	0x52: 'AuthenticationOk', // 'R'
	0x53: 'ParameterStatus', // 'S'
	0x54: 'RowDescription', // 'T'
	0x5a: 'ReadyForQuery', // 'Z'
	0x6e: 'NoData', // 'n'
	0x73: 'PortalSuspended', // 's'
	0x74: 'ParameterDescription' // 't'
};

const ERROR_FIELD_NAMES: Record<number, string> = {
	0x53: 'severity', // S
	0x56: 'severity_v', // V
	0x43: 'code', // C
	0x4d: 'message', // M
	0x44: 'detail', // D
	0x48: 'hint', // H
	0x50: 'position', // P
	0x71: 'internal_position', // q
	0x57: 'where', // W
	0x73: 'schema', // s
	0x74: 'table', // t
	0x63: 'column', // c
	0x64: 'data_type', // d
	0x6e: 'constraint', // n
	0x46: 'file', // F
	0x4c: 'line', // L
	0x52: 'routine' // R
};

const AUTH_NAMES: Record<number, string> = {
	0: 'Ok',
	3: 'CleartextPassword',
	5: 'MD5Password',
	10: 'SASL',
	11: 'SASLContinue',
	12: 'SASLFinal'
};

const SSL_REQUEST_CODE = 80877103;
const CANCEL_REQUEST_CODE = 80877102;
const PROTOCOL_3_0 = 196608;

const view_of = (data: Uint8Array): DataView => new DataView(data.buffer, data.byteOffset);

const decode = (bytes: Uint8Array): string => new TextDecoder().decode(bytes);

/** A startup-phase message: an SSL request, a cancel request, or the startup message with its params. */
export const parse_startup_message = (
	data: Uint8Array
): { type: string; payload: unknown } | null => {
	if (data.length < 8) return null;
	const view = view_of(data);
	const length = view.getInt32(0);
	const version = view.getInt32(4);
	if (version === SSL_REQUEST_CODE) return { type: 'SSLRequest', payload: {} };
	if (version === CANCEL_REQUEST_CODE) {
		return {
			type: 'CancelRequest',
			payload: { pid: view.getInt32(8), secret: view.getInt32(12) }
		};
	}
	if (version === PROTOCOL_3_0) {
		const params: Record<string, string> = {};
		let offset = 8;
		while (offset < length - 1) {
			const key_end = data.indexOf(0, offset);
			if (key_end === -1 || key_end >= length) break;
			const key = decode(data.slice(offset, key_end));
			offset = key_end + 1;
			if (!key) break;
			const value_end = data.indexOf(0, offset);
			if (value_end === -1 || value_end >= length) break;
			const value = decode(data.slice(offset, value_end));
			offset = value_end + 1;
			params[key] = value;
		}
		return { type: 'StartupMessage', payload: { version: '3.0', params } };
	}
	return { type: 'UnknownStartup', payload: { version } };
};

/** A simple query's text, up to its NUL. */
export const parse_query = (payload: Uint8Array): string => {
	const end = payload.indexOf(0);
	return decode(end >= 0 ? payload.slice(0, end) : payload);
};

/**
 * Parse (client): the statement name, the query text, and the client-declared
 * parameter type OIDs. For tokio-postgres these are usually all 0 ("server,
 * infer it"), which is itself the finding: the spine leans on server-side inference.
 */
export const parse_parse = (
	payload: Uint8Array
): { statement: string; query: string; paramTypeOids: Array<number> } => {
	const name_end = payload.indexOf(0);
	const statement = decode(payload.slice(0, name_end));
	const query_end = payload.indexOf(0, name_end + 1);
	const query = decode(payload.slice(name_end + 1, query_end));
	const view = view_of(payload);
	let offset = query_end + 1;
	const count = view.getInt16(offset);
	offset += 2;
	const paramTypeOids: Array<number> = [];
	for (let i = 0; i < count; i++) {
		paramTypeOids.push(view.getInt32(offset));
		offset += 4;
	}
	return { statement, query, paramTypeOids };
};

/**
 * ParameterDescription (server): the parameter type OIDs the server inferred,
 * the canonical answer to "what type is each `$N`" and the highest-value
 * capture for authoring spine fixtures.
 */
export const parse_parameter_description = (
	payload: Uint8Array
): { paramTypeOids: Array<number> } => {
	const view = view_of(payload);
	const count = view.getInt16(0);
	const paramTypeOids: Array<number> = [];
	let offset = 2;
	for (let i = 0; i < count; i++) {
		paramTypeOids.push(view.getInt32(offset));
		offset += 4;
	}
	return { paramTypeOids };
};

/**
 * Bind (client): the portal and statement names, the per-parameter format
 * codes, and the raw parameter values (hex; a -1 length is NULL), what the
 * driver actually sent over the wire and what pglet's decoder must accept.
 */
export const parse_bind = (
	payload: Uint8Array
): {
	portal: string;
	statement: string;
	paramFormats: Array<number>;
	params: Array<string | null>;
} => {
	const portal_end = payload.indexOf(0);
	const portal = decode(payload.slice(0, portal_end));
	const statement_end = payload.indexOf(0, portal_end + 1);
	const statement = decode(payload.slice(portal_end + 1, statement_end));
	const view = view_of(payload);
	let offset = statement_end + 1;

	const format_count = view.getInt16(offset);
	offset += 2;
	const paramFormats: Array<number> = [];
	for (let i = 0; i < format_count; i++) {
		paramFormats.push(view.getInt16(offset));
		offset += 2;
	}

	const param_count = view.getInt16(offset);
	offset += 2;
	const params: Array<string | null> = [];
	for (let i = 0; i < param_count; i++) {
		const len = view.getInt32(offset);
		offset += 4;
		if (len === -1) {
			params.push(null);
		} else {
			const bytes = payload.slice(offset, offset + len);
			offset += len;
			params.push(Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join(''));
		}
	}
	return { portal, statement, paramFormats, params };
};

/** An ErrorResponse's or NoticeResponse's fields, by name, an unknown field by its code letter. */
export const parse_error_or_notice = (payload: Uint8Array): Record<string, string> => {
	const fields: Record<string, string> = {};
	let offset = 0;
	while (offset < payload.length) {
		const field_type = payload[offset]!;
		if (field_type === 0) break;
		offset++;
		const value_end = payload.indexOf(0, offset);
		if (value_end === -1) break;
		const value = decode(payload.slice(offset, value_end));
		offset = value_end + 1;
		fields[ERROR_FIELD_NAMES[field_type] ?? String.fromCharCode(field_type)] = value;
	}
	return fields;
};

/** A ParameterStatus's name and value. */
export const parse_parameter_status = (payload: Uint8Array): { name: string; value: string } => {
	const sep = payload.indexOf(0);
	const name = decode(payload.slice(0, sep));
	const value_end = payload.indexOf(0, sep + 1);
	const value = decode(payload.slice(sep + 1, value_end));
	return { name, value };
};

/** A RowDescription's fields. */
export const parse_row_description = (payload: Uint8Array): Array<unknown> => {
	const view = view_of(payload);
	const field_count = view.getInt16(0);
	const fields: Array<unknown> = [];
	let offset = 2;
	for (let i = 0; i < field_count; i++) {
		const name_end = payload.indexOf(0, offset);
		const name = decode(payload.slice(offset, name_end));
		offset = name_end + 1;
		const tableOid = view.getInt32(offset);
		offset += 4;
		const columnAttr = view.getInt16(offset);
		offset += 2;
		const typeOid = view.getInt32(offset);
		offset += 4;
		const typeLen = view.getInt16(offset);
		offset += 2;
		const typeMod = view.getInt32(offset);
		offset += 4;
		const format = view.getInt16(offset);
		offset += 2;
		fields.push({ name, tableOid, columnAttr, typeOid, typeLen, typeMod, format });
	}
	return fields;
};

/** A DataRow's values as text, NULL as null. */
export const parse_data_row = (payload: Uint8Array): Array<string | null> => {
	const view = view_of(payload);
	const column_count = view.getInt16(0);
	const values: Array<string | null> = [];
	let offset = 2;
	for (let i = 0; i < column_count; i++) {
		const length = view.getInt32(offset);
		offset += 4;
		if (length === -1) {
			values.push(null);
		} else {
			values.push(decode(payload.slice(offset, offset + length)));
			offset += length;
		}
	}
	return values;
};

const to_payload = (type_name: string, payload: Uint8Array): unknown => {
	switch (type_name) {
		case 'Query':
			return { query: parse_query(payload) };
		case 'Parse':
			return parse_parse(payload);
		case 'Bind':
			return parse_bind(payload);
		case 'ParameterDescription':
			return parse_parameter_description(payload);
		case 'ErrorResponse':
		case 'NoticeResponse':
			return parse_error_or_notice(payload);
		case 'ParameterStatus':
			return parse_parameter_status(payload);
		case 'RowDescription':
			return { fields: parse_row_description(payload) };
		case 'DataRow':
			return { values: parse_data_row(payload) };
		case 'CommandComplete':
			return { tag: decode(payload.slice(0, payload.indexOf(0))) };
		case 'ReadyForQuery':
			return { status: String.fromCharCode(payload[0]!) };
		case 'AuthenticationOk': {
			const auth_type = view_of(payload).getInt32(0);
			return { auth: AUTH_NAMES[auth_type] ?? auth_type };
		}
		case 'BackendKeyData':
			return { pid: view_of(payload).getInt32(0), secret: view_of(payload).getInt32(4) };
		default:
			return { raw_length: payload.length };
	}
};

/**
 * The first whole message in `data`, or null when `data` doesn't hold one yet.
 * A client in its startup phase sends length-prefixed messages with no type
 * byte; everything else is a type byte, a length that counts itself, and the
 * payload.
 *
 * @param now - the capture's timestamp, an ISO string
 */
export const parse_spy_message = (
	data: Uint8Array,
	direction: SpyDirection,
	is_startup: boolean,
	now: () => string
): SpyParsed | null => {
	if (is_startup && direction === 'client') {
		if (data.length < 4) return null;
		const length = view_of(data).getInt32(0);
		if (data.length < length) return null;
		const startup = parse_startup_message(data.slice(0, length));
		if (!startup) return null;
		return {
			message: {
				direction,
				type: startup.type,
				length,
				payload: startup.payload,
				timestamp: now()
			},
			consumed: length
		};
	}

	if (data.length < 5) return null;
	const msg_type = data[0]!;
	const length = view_of(data).getInt32(1); // counts itself
	const total = 1 + length;
	if (data.length < total) return null;

	const types = direction === 'client' ? CLIENT_TYPES : SERVER_TYPES;
	const type = types[msg_type] ?? `Unknown(0x${msg_type.toString(16)})`;
	return {
		message: {
			direction,
			type,
			length: total,
			payload: to_payload(type, data.slice(5, total)),
			timestamp: now()
		},
		consumed: total
	};
};
