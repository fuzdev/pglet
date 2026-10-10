// Unit tests for the extended-protocol message parsers (Parse / Bind /
// ParameterDescription). Hand-crafted wire payloads (the message body after the
// 1-byte type tag and 4-byte length) keep these deterministic — no proxy.

import { assertEquals } from "jsr:@std/assert";
import { parseBind, parseParameterDescription, parseParse } from "./main.ts";

const enc = new TextEncoder();

/** Build a payload from a sequence of bytes / strings (NUL-terminated) / i16 / i32. */
function payload(
  parts: Array<{ str: string } | { i16: number } | { i32: number } | { bytes: number[] }>,
): Uint8Array {
  const out: number[] = [];
  for (const p of parts) {
    if ("str" in p) {
      out.push(...enc.encode(p.str), 0);
    } else if ("i16" in p) {
      out.push((p.i16 >> 8) & 0xff, p.i16 & 0xff);
    } else if ("i32" in p) {
      out.push((p.i32 >>> 24) & 0xff, (p.i32 >>> 16) & 0xff, (p.i32 >>> 8) & 0xff, p.i32 & 0xff);
    } else {
      out.push(...p.bytes);
    }
  }
  return new Uint8Array(out);
}

Deno.test("parseParse: name, query, and untyped params", () => {
  // tokio-postgres sends empty param-type OIDs (server infers).
  const p = parseParse(payload([
    { str: "s1" },
    { str: "SELECT $1::int4" },
    { i16: 0 },
  ]));
  assertEquals(p.statement, "s1");
  assertEquals(p.query, "SELECT $1::int4");
  assertEquals(p.paramTypeOids, []);
});

Deno.test("parseParse: declared param OIDs", () => {
  const p = parseParse(payload([
    { str: "" },
    { str: "SELECT $1" },
    { i16: 2 },
    { i32: 23 }, // int4
    { i32: 25 }, // text
  ]));
  assertEquals(p.paramTypeOids, [23, 25]);
});

Deno.test("parseParameterDescription: inferred OIDs", () => {
  // The server's inferred types: uuid[] then text.
  const d = parseParameterDescription(payload([
    { i16: 2 },
    { i32: 2951 }, // uuid[]
    { i32: 25 }, // text
  ]));
  assertEquals(d.paramTypeOids, [2951, 25]);
});

Deno.test("parseBind: formats, hex values, and NULL", () => {
  const b = parseBind(payload([
    { str: "" }, // portal
    { str: "s1" }, // statement
    { i16: 1 }, // one format code...
    { i16: 1 }, // ...binary
    { i16: 2 }, // two params
    { i32: 4 }, // param 1 length
    { bytes: [0x00, 0x00, 0x00, 0x2a] }, // 42 (int4)
    { i32: -1 }, // param 2 is NULL
  ]));
  assertEquals(b.statement, "s1");
  assertEquals(b.paramFormats, [1]);
  assertEquals(b.params, ["0000002a", null]);
});
