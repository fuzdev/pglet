/**
 * Spy Proxy for Postgres Wire Protocol
 *
 * Sits between a Postgres client and server, logging all messages.
 * Usage: deno run -A tools/spy_proxy/main.ts [listen_port] [pg_port] [output_file]
 *
 * Default: listens on 5434, forwards to 127.0.0.1:5432, logs to tests/captures/session.jsonl
 *
 * This tool does NOT read `PGLET_PG_URL` — the forward target is the second
 * positional argument, and the `spy:*` tasks in deno.json grant Deno's network
 * sandbox `127.0.0.1:5432,127.0.0.1:5434` literally. Forwarding anywhere else
 * needs both the argument and a wider `--allow-net`, or the sandbox refuses the
 * connection (which reads nothing like a connection error).
 *
 * Related files:
 * - tests/captures/orm-introspection.md - Captured ORM queries
 * - tests/fixtures/ - Differential test fixtures authored from captures
 *
 * Run: deno task spy:start
 */

const LISTEN_PORT = parseInt(Deno.args[0] ?? "5434");
const PG_HOST = "127.0.0.1";
const PG_PORT = parseInt(Deno.args[1] ?? "5432");
const OUTPUT_FILE = Deno.args[2] ?? "tests/captures/session.jsonl";

interface Message {
  direction: "client" | "server";
  type: string;
  length: number;
  payload: unknown;
  raw_hex?: string;
  timestamp: string;
}

// Postgres message types
const CLIENT_TYPES: Record<number, string> = {
  0x51: "Query", // 'Q'
  0x50: "Parse", // 'P'
  0x42: "Bind", // 'B'
  0x44: "Describe", // 'D'
  0x45: "Execute", // 'E'
  0x48: "Flush", // 'H'
  0x53: "Sync", // 'S'
  0x58: "Terminate", // 'X'
  0x63: "CopyDone", // 'c'
  0x64: "CopyData", // 'd'
  0x66: "CopyFail", // 'f'
  0x70: "PasswordMessage", // 'p'
};

const SERVER_TYPES: Record<number, string> = {
  0x31: "ParseComplete", // '1'
  0x32: "BindComplete", // '2'
  0x33: "CloseComplete", // '3'
  0x41: "NotificationResponse", // 'A'
  0x43: "CommandComplete", // 'C'
  0x44: "DataRow", // 'D'
  0x45: "ErrorResponse", // 'E'
  0x47: "CopyInResponse", // 'G'
  0x48: "CopyOutResponse", // 'H'
  0x49: "EmptyQueryResponse", // 'I'
  0x4b: "BackendKeyData", // 'K'
  0x4e: "NoticeResponse", // 'N'
  0x52: "AuthenticationOk", // 'R'
  0x53: "ParameterStatus", // 'S'
  0x54: "RowDescription", // 'T'
  0x5a: "ReadyForQuery", // 'Z'
  0x6e: "NoData", // 'n'
  0x73: "PortalSuspended", // 's'
  0x74: "ParameterDescription", // 't'
};

function parseStartupMessage(
  data: Uint8Array,
): { type: string; payload: unknown } | null {
  if (data.length < 8) return null;

  const view = new DataView(data.buffer, data.byteOffset);
  const length = view.getInt32(0);
  const version = view.getInt32(4);

  // SSL Request
  if (version === 80877103) {
    return { type: "SSLRequest", payload: {} };
  }

  // Cancel Request
  if (version === 80877102) {
    return {
      type: "CancelRequest",
      payload: {
        pid: view.getInt32(8),
        secret: view.getInt32(12),
      },
    };
  }

  // Startup Message (protocol 3.0 = 196608)
  if (version === 196608) {
    const params: Record<string, string> = {};
    let offset = 8;
    while (offset < length - 1) {
      const keyEnd = data.indexOf(0, offset);
      if (keyEnd === -1 || keyEnd >= length) break;
      const key = new TextDecoder().decode(data.slice(offset, keyEnd));
      offset = keyEnd + 1;
      if (!key) break;

      const valueEnd = data.indexOf(0, offset);
      if (valueEnd === -1 || valueEnd >= length) break;
      const value = new TextDecoder().decode(data.slice(offset, valueEnd));
      offset = valueEnd + 1;

      params[key] = value;
    }
    return { type: "StartupMessage", payload: { version: "3.0", params } };
  }

  return { type: "UnknownStartup", payload: { version } };
}

function parseQuery(payload: Uint8Array): string {
  // Query payload is null-terminated string
  const end = payload.indexOf(0);
  const text = end >= 0 ? payload.slice(0, end) : payload;
  return new TextDecoder().decode(text);
}

// Parse (client): statement name, query text, and the client-declared parameter
// type OIDs. For tokio-postgres these OIDs are usually all 0 ("server, infer
// it") — which is itself the finding: the spine leans on server-side inference.
export function parseParse(
  payload: Uint8Array,
): { statement: string; query: string; paramTypeOids: number[] } {
  const dec = new TextDecoder();
  const nameEnd = payload.indexOf(0);
  const statement = dec.decode(payload.slice(0, nameEnd));
  const queryEnd = payload.indexOf(0, nameEnd + 1);
  const query = dec.decode(payload.slice(nameEnd + 1, queryEnd));
  const view = new DataView(payload.buffer, payload.byteOffset);
  let offset = queryEnd + 1;
  const count = view.getInt16(offset);
  offset += 2;
  const paramTypeOids: number[] = [];
  for (let i = 0; i < count; i++) {
    paramTypeOids.push(view.getInt32(offset));
    offset += 4;
  }
  return { statement, query, paramTypeOids };
}

// ParameterDescription (server): the parameter type OIDs the server INFERRED.
// This is the canonical answer to "what type is each $N" and the highest-value
// capture for authoring spine fixtures.
export function parseParameterDescription(payload: Uint8Array): { paramTypeOids: number[] } {
  const view = new DataView(payload.buffer, payload.byteOffset);
  const count = view.getInt16(0);
  const paramTypeOids: number[] = [];
  let offset = 2;
  for (let i = 0; i < count; i++) {
    paramTypeOids.push(view.getInt32(offset));
    offset += 4;
  }
  return { paramTypeOids };
}

// Bind (client): portal/statement names, the per-parameter format codes, and the
// raw parameter values (hex; -1 length is NULL) — what the driver actually sent
// over the wire and what pglet's decoder must accept.
export function parseBind(
  payload: Uint8Array,
): {
  portal: string;
  statement: string;
  paramFormats: number[];
  params: (string | null)[];
} {
  const dec = new TextDecoder();
  const portalEnd = payload.indexOf(0);
  const portal = dec.decode(payload.slice(0, portalEnd));
  const stmtEnd = payload.indexOf(0, portalEnd + 1);
  const statement = dec.decode(payload.slice(portalEnd + 1, stmtEnd));
  const view = new DataView(payload.buffer, payload.byteOffset);
  let offset = stmtEnd + 1;

  const formatCount = view.getInt16(offset);
  offset += 2;
  const paramFormats: number[] = [];
  for (let i = 0; i < formatCount; i++) {
    paramFormats.push(view.getInt16(offset));
    offset += 2;
  }

  const paramCount = view.getInt16(offset);
  offset += 2;
  const params: (string | null)[] = [];
  for (let i = 0; i < paramCount; i++) {
    const len = view.getInt32(offset);
    offset += 4;
    if (len === -1) {
      params.push(null);
    } else {
      const bytes = payload.slice(offset, offset + len);
      offset += len;
      params.push(
        Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(""),
      );
    }
  }
  return { portal, statement, paramFormats, params };
}

function parseErrorOrNotice(
  payload: Uint8Array,
): Record<string, string> {
  const fields: Record<string, string> = {};
  let offset = 0;
  while (offset < payload.length) {
    const fieldType = payload[offset];
    if (fieldType === 0) break;
    offset++;
    const valueEnd = payload.indexOf(0, offset);
    if (valueEnd === -1) break;
    const value = new TextDecoder().decode(payload.slice(offset, valueEnd));
    offset = valueEnd + 1;

    const fieldNames: Record<number, string> = {
      0x53: "severity", // S
      0x56: "severity_v", // V
      0x43: "code", // C
      0x4d: "message", // M
      0x44: "detail", // D
      0x48: "hint", // H
      0x50: "position", // P
      0x71: "internal_position", // q
      0x57: "where", // W
      0x73: "schema", // s
      0x74: "table", // t
      0x63: "column", // c
      0x64: "data_type", // d
      0x6e: "constraint", // n
      0x46: "file", // F
      0x4c: "line", // L
      0x52: "routine", // R
    };
    fields[fieldNames[fieldType] ?? String.fromCharCode(fieldType)] = value;
  }
  return fields;
}

function parseParameterStatus(
  payload: Uint8Array,
): { name: string; value: string } {
  const sep = payload.indexOf(0);
  const name = new TextDecoder().decode(payload.slice(0, sep));
  const valueEnd = payload.indexOf(0, sep + 1);
  const value = new TextDecoder().decode(payload.slice(sep + 1, valueEnd));
  return { name, value };
}

function parseRowDescription(payload: Uint8Array): unknown[] {
  const view = new DataView(payload.buffer, payload.byteOffset);
  const fieldCount = view.getInt16(0);
  const fields: unknown[] = [];
  let offset = 2;

  for (let i = 0; i < fieldCount; i++) {
    const nameEnd = payload.indexOf(0, offset);
    const name = new TextDecoder().decode(payload.slice(offset, nameEnd));
    offset = nameEnd + 1;

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

    fields.push({
      name,
      tableOid,
      columnAttr,
      typeOid,
      typeLen,
      typeMod,
      format,
    });
  }
  return fields;
}

function parseDataRow(payload: Uint8Array): (string | null)[] {
  const view = new DataView(payload.buffer, payload.byteOffset);
  const columnCount = view.getInt16(0);
  const values: (string | null)[] = [];
  let offset = 2;

  for (let i = 0; i < columnCount; i++) {
    const length = view.getInt32(offset);
    offset += 4;
    if (length === -1) {
      values.push(null);
    } else {
      values.push(
        new TextDecoder().decode(payload.slice(offset, offset + length)),
      );
      offset += length;
    }
  }
  return values;
}

function parseMessage(
  data: Uint8Array,
  direction: "client" | "server",
  isStartup: boolean,
): { message: Message; consumed: number } | null {
  if (isStartup && direction === "client") {
    if (data.length < 4) return null;
    const view = new DataView(data.buffer, data.byteOffset);
    const length = view.getInt32(0);
    if (data.length < length) return null;

    const startup = parseStartupMessage(data.slice(0, length));
    if (!startup) return null;

    return {
      message: {
        direction,
        type: startup.type,
        length,
        payload: startup.payload,
        timestamp: new Date().toISOString(),
      },
      consumed: length,
    };
  }

  // Regular message: 1 byte type + 4 byte length + payload
  if (data.length < 5) return null;

  const msgType = data[0];
  const view = new DataView(data.buffer, data.byteOffset);
  const length = view.getInt32(1); // includes length field itself
  const totalLen = 1 + length;

  if (data.length < totalLen) return null;

  const payload = data.slice(5, totalLen);
  const types = direction === "client" ? CLIENT_TYPES : SERVER_TYPES;
  const typeName = types[msgType] ?? `Unknown(0x${msgType.toString(16)})`;

  let parsedPayload: unknown;
  switch (typeName) {
    case "Query":
      parsedPayload = { query: parseQuery(payload) };
      break;
    case "Parse":
      parsedPayload = parseParse(payload);
      break;
    case "Bind":
      parsedPayload = parseBind(payload);
      break;
    case "ParameterDescription":
      parsedPayload = parseParameterDescription(payload);
      break;
    case "ErrorResponse":
    case "NoticeResponse":
      parsedPayload = parseErrorOrNotice(payload);
      break;
    case "ParameterStatus":
      parsedPayload = parseParameterStatus(payload);
      break;
    case "RowDescription":
      parsedPayload = { fields: parseRowDescription(payload) };
      break;
    case "DataRow":
      parsedPayload = { values: parseDataRow(payload) };
      break;
    case "CommandComplete":
      parsedPayload = {
        tag: new TextDecoder().decode(payload.slice(0, payload.indexOf(0))),
      };
      break;
    case "ReadyForQuery":
      parsedPayload = { status: String.fromCharCode(payload[0]) };
      break;
    case "AuthenticationOk": {
      const authType = new DataView(payload.buffer, payload.byteOffset)
        .getInt32(0);
      const authNames: Record<number, string> = {
        0: "Ok",
        3: "CleartextPassword",
        5: "MD5Password",
        10: "SASL",
        11: "SASLContinue",
        12: "SASLFinal",
      };
      parsedPayload = { auth: authNames[authType] ?? authType };
      break;
    }
    case "BackendKeyData":
      parsedPayload = {
        pid: new DataView(payload.buffer, payload.byteOffset).getInt32(0),
        secret: new DataView(payload.buffer, payload.byteOffset).getInt32(4),
      };
      break;
    default:
      // For unknown messages, include hex dump
      parsedPayload = { raw_length: payload.length };
  }

  return {
    message: {
      direction,
      type: typeName,
      length: totalLen,
      payload: parsedPayload,
      timestamp: new Date().toISOString(),
    },
    consumed: totalLen,
  };
}

async function handleConnection(
  clientConn: Deno.TcpConn,
  outputFile: Deno.FsFile,
) {
  const clientAddr = clientConn.remoteAddr as Deno.NetAddr;
  console.log(`[CONNECT] Client ${clientAddr.hostname}:${clientAddr.port}`);

  let pgConn: Deno.TcpConn;
  try {
    pgConn = await Deno.connect({ hostname: PG_HOST, port: PG_PORT });
  } catch (e) {
    console.error(`[ERROR] Failed to connect to Postgres: ${e}`);
    clientConn.close();
    return;
  }

  const logMessage = async (msg: Message) => {
    const line = JSON.stringify(msg) + "\n";
    console.log(
      `[${msg.direction.toUpperCase()}] ${msg.type}`,
      JSON.stringify(msg.payload),
    );
    await outputFile.write(new TextEncoder().encode(line));
  };

  let clientStartup = true;
  let awaitingSslResponse = false;

  // Proxy client -> postgres
  const clientToPg = async () => {
    const buffer = new Uint8Array(65536);
    let pending = new Uint8Array(0);

    try {
      while (true) {
        const n = await clientConn.read(buffer);
        if (n === null) break;

        const newPending = new Uint8Array(pending.length + n);
        newPending.set(pending);
        newPending.set(buffer.slice(0, n), pending.length);
        pending = newPending;

        // Parse and log messages
        while (pending.length > 0) {
          const result = parseMessage(pending, "client", clientStartup);
          if (!result) break;

          await logMessage(result.message);

          if (result.message.type === "SSLRequest") {
            awaitingSslResponse = true;
          } else if (result.message.type === "StartupMessage") {
            clientStartup = false;
          }

          pending = pending.slice(result.consumed);
        }

        // Forward raw data to postgres
        await pgConn.write(buffer.slice(0, n));
      }
    } catch (e) {
      if (
        !(e instanceof Deno.errors.BrokenPipe ||
          e instanceof Deno.errors.ConnectionReset)
      ) {
        console.error(`[ERROR] Client read: ${e}`);
      }
    }
  };

  // Proxy postgres -> client
  const pgToClient = async () => {
    const buffer = new Uint8Array(65536);
    let pending = new Uint8Array(0);

    try {
      while (true) {
        const n = await pgConn.read(buffer);
        if (n === null) break;

        const newPending = new Uint8Array(pending.length + n);
        newPending.set(pending);
        newPending.set(buffer.slice(0, n), pending.length);
        pending = newPending;

        // Handle SSL response (single byte 'N' or 'S')
        if (awaitingSslResponse && pending.length >= 1) {
          const sslResponse = String.fromCharCode(pending[0]);
          await logMessage({
            direction: "server",
            type: "SSLResponse",
            length: 1,
            payload: { response: sslResponse === "N" ? "NoSSL" : "SSL" },
            timestamp: new Date().toISOString(),
          });
          awaitingSslResponse = false;
          clientStartup = true; // Client will send StartupMessage next
          pending = pending.slice(1);
        }

        // Parse and log messages
        while (pending.length > 0) {
          const result = parseMessage(pending, "server", false);
          if (!result) break;

          await logMessage(result.message);
          pending = pending.slice(result.consumed);
        }

        // Forward raw data to client
        await clientConn.write(buffer.slice(0, n));
      }
    } catch (e) {
      if (
        !(e instanceof Deno.errors.BrokenPipe ||
          e instanceof Deno.errors.ConnectionReset)
      ) {
        console.error(`[ERROR] Server read: ${e}`);
      }
    }
  };

  await Promise.race([clientToPg(), pgToClient()]);

  try {
    clientConn.close();
  } catch { /* ignore */ }
  try {
    pgConn.close();
  } catch { /* ignore */ }

  console.log(`[DISCONNECT] Client ${clientAddr.hostname}:${clientAddr.port}`);
}

async function main() {
  console.log(`Spy proxy listening on 127.0.0.1:${LISTEN_PORT}`);
  console.log(`Forwarding to ${PG_HOST}:${PG_PORT}`);
  console.log(`Logging to ${OUTPUT_FILE}`);

  const outputFile = await Deno.open(OUTPUT_FILE, {
    write: true,
    create: true,
    append: true,
  });

  const listener = Deno.listen({ hostname: "127.0.0.1", port: LISTEN_PORT });

  for await (const conn of listener) {
    handleConnection(conn, outputFile);
  }
}

if (import.meta.main) {
  main();
}
