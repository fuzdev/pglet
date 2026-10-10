/**
 * Drives the JS a statement reaches that the caller or the page controls — a bound
 * parameter's getters and `toJSON`, a replaced `Date.now` — to throw, or
 * to call back into the module, and prints, as one JSON line, what each call answered
 * and what the module answers after it.
 *
 * Run by `pglet_wasm_test.ts` in a child Deno, so a module these leave poisoned cannot
 * take the rest of that suite with it.
 *
 * Usage: `deno run -A host_exceptions.ts <pkg dir URL>`.
 */

const [pkgUrl] = Deno.args;
const PKG = new URL(pkgUrl);
// deno-lint-ignore no-explicit-any
const mod: any = await import(new URL("pglet_wasm.js", PKG).href);
mod.initSync({ module: await Deno.readFile(new URL("pglet_wasm_bg.wasm", PKG)) });

/** What `f` throws — its `name`, `code` and `message`, or a thrown string — or `ok`. */
function outcome(f: () => unknown): string {
  try {
    f();
    return "ok";
  } catch (e) {
    if (typeof e === "string") return e;
    const err = e as { name?: string; code?: string; message?: string };
    return [err.name, err.code, err.message].filter((x) => x !== undefined).join(" ");
  }
}

// deno-lint-ignore no-explicit-any
const report: Record<string, any> = {};
const db = new mod.PgletBtree();
const thrower = (what: string) => () => {
  throw new Error(what);
};
// deno-lint-ignore no-explicit-any
const trap = (target: any, handler: ProxyHandler<any>) => new Proxy(target, handler);
// First, before this module has built any result: what a result is built with lives only as
// long as that result, so a page that replaces the global `Object` for one statement (a mock,
// a shim) — here the very first — sees that statement's stores, and none of a later one's once
// it restores it.
const RealObject = Object;
const seen: string[] = [];
// @ts-ignore a replaced global, restored below
globalThis.Object = function () {
  return new Proxy(new RealObject(), {
    set(t, k, v) {
      seen.push(`${String(k)}=${JSON.stringify(v)}`);
      t[k] = v;
      return true;
    },
  });
};
const replaced = outcome(() => db.query(`SELECT '{"a": 1}'::jsonb AS j`));
globalThis.Object = RealObject;
seen.length = 0;
const later = outcome(() => new mod.PgletBtree().query(`SELECT '{"secret": "s3cr3t"}'::jsonb AS j, 42 AS n`));
report["transient Object"] = { replaced, later, seen_after_restore: [...seen] };
const { proxy: revoked, revoke } = Proxy.revocable([1], {});
revoke();
const params: Record<string, [string, unknown]> = {
  "index getter": ["SELECT $1::int4[]", trap([1, 2], { get: (t, k) => (k === "0" ? thrower("index getter threw")() : t[k]) })],
  "length getter": ["SELECT $1::int4[]", trap([1, 2], { get: (t, k) => (k === "length" ? thrower("length getter threw")() : t[k]) })],
  "revoked proxy": ["SELECT $1::int4[]", revoked],
  "view getter": ["SELECT $1::bytea", Object.defineProperty(new Uint16Array(1), "byteLength", { get: thrower("byteLength getter threw") })],
  "toJSON": ["SELECT $1::jsonb", { toJSON: thrower("toJSON threw") }],
};
for (const [name, [sql, param]] of Object.entries(params)) {
  report[name] = {
    call: outcome(() => db.queryParams(sql, [param])),
    same: outcome(() => db.query("SELECT 1")),
    fresh: outcome(() => new mod.PgletBtree().query("SELECT 1")),
  };
}

// A replaced `Date.now` that calls back into the module, on another instance.
const other = new mod.PgletBtree();
const now = Date.now;
let inner = "";
Date.now = () => {
  inner = outcome(() => other.query("SELECT 2"));
  return now();
};
const reentered = outcome(() => db.query("SELECT now()"));
// And one that throws.
Date.now = thrower("Date.now threw");
const threw = outcome(() => db.query("SELECT now()"));
Date.now = now;
report["reentered"] = {
  call: reentered,
  inner,
  threw,
  same: outcome(() => db.query("SELECT 1")),
  other: outcome(() => other.query("SELECT 1")),
  fresh: outcome(() => new mod.PgletBtree().query("SELECT 1")),
};
// The JS **building a result** runs: the prototypes its objects and arrays inherit from. A
// field descriptor is an object literal, its keys defined, so an `Object.prototype` member named
// for one of them is never reached — a throwing setter, a read-only value, a setter that stores
// nothing, one that adds a key of its own or stores its own value: the statement answers and
// each field owns its seven keys, each holding pglet's value.
/** `fields[0]`'s own keys, each `key=value` and its writable/enumerable/configurable flags. */
const own_field = (f: Record<string, unknown>): string =>
  Reflect.ownKeys(f).map((k) => {
    const d = Object.getOwnPropertyDescriptor(f, k)!;
    return `${String(k)}=${"value" in d ? String(d.value) : "accessor"}:${d.writable}${d.enumerable}${d.configurable}`;
  }).join(" ");
const field_hook = (key: string, descriptor: PropertyDescriptor) => {
  Object.defineProperty(Object.prototype, key, { configurable: true, ...descriptor });
  let own = "none";
  const call = outcome(() => {
    own = own_field((db.query("SELECT 1 AS a") as { fields: Record<string, unknown>[] }).fields[0]);
  });
  // @ts-ignore removing the properties this test added
  delete (Object.prototype as Record<string, unknown>)[key];
  // @ts-ignore removing the properties this test added
  delete (Object.prototype as Record<string, unknown>)["zzz"];
  return { call, own };
};
const define_on = (key: string, value: unknown) =>
  function (this: object) {
    Object.defineProperty(this, key, { value, writable: true, enumerable: true, configurable: true });
  };
report["field key throwing setter"] = field_hook("tableID", { set: thrower("setter threw") });
report["field key read-only"] = field_hook("format", { writable: false, value: "binary" });
report["field key swallowed"] = field_hook("dataTypeID", { set(_v: unknown) {}, get: () => 0 });
report["field key adds another"] = field_hook("format", { set: define_on("zzz", 1), get: () => "binary" });
report["field key own value"] = field_hook("dataTypeID", { set: define_on("dataTypeID", 25) });
// A row's column name and a JSONB object's key are data, **defined** on the object as
// `JSON.parse` defines a key, so a page's setter of the same name is never reached: the
// statement answers, and the key is the row's own.
Object.defineProperty(Object.prototype, "poisoned", { configurable: true, set: thrower("setter threw") });
let column_setter: string;
try {
  const rows = db.query(`SELECT 1 AS poisoned, '{"poisoned": 2}'::jsonb AS j`).rows as Record<string, unknown>[];
  const j = rows[0].j as Record<string, unknown>;
  column_setter = Object.hasOwn(rows[0], "poisoned") && Object.hasOwn(j, "poisoned") ? "ok" : "missing";
} catch (e) {
  column_setter = String(e);
}
// @ts-ignore removing the property this test added
delete (Object.prototype as Record<string, unknown>)["poisoned"];
report["column setter"] = { call: column_setter };
// An array element — a row, a cell, a field, a JSONB or SQL array's element — is an own
// element whatever `Array.prototype` holds (defined once a page put an index on the chain), so
// neither an index setter a page puts there nor a read-only inherited index is reached: the
// statement answers with every element.
const elements = (): string => {
  const r = db.query(`SELECT 1 AS a, '[7,8]'::jsonb AS j, ARRAY[4,5] AS arr`) as {
    rows: Record<string, unknown>[];
    fields: unknown[];
  };
  return JSON.stringify([r.rows, r.fields.length]);
};
Object.defineProperty(Array.prototype, "0", { configurable: true, set: thrower("index setter threw") });
const index_setter = outcome(() => (report["index setter elements"] = elements()));
// @ts-ignore removing the property this test added
delete (Array.prototype as unknown as Record<string, unknown>)["0"];
report["index setter"] = {
  call: index_setter,
  same: outcome(() => db.query("SELECT 1")),
  fresh: outcome(() => new mod.PgletBtree().query("SELECT 1")),
};
Object.defineProperty(Array.prototype, "0", { configurable: true, writable: false, value: "inherited" });
const read_only_index = outcome(() => (report["read-only index elements"] = elements()));
// @ts-ignore removing the property this test added
delete (Array.prototype as unknown as Record<string, unknown>)["0"];
report["read-only index"] = { call: read_only_index };
// A `bytea` value's bytes are copied by a **caught** `Uint8Array.prototype.set`, so a page
// that replaced it fails the statement and leaves the module usable.
const typed_set = Uint8Array.prototype.set;
Uint8Array.prototype.set = thrower("bytea set threw") as typeof typed_set;
const bytea_set = outcome(() => db.query(`SELECT '\\x00ff'::bytea AS b`));
Uint8Array.prototype.set = typed_set;
report["bytea set"] = {
  call: bytea_set,
  same: outcome(() => db.query(`SELECT '\\x00ff'::bytea AS b`)),
  fresh: outcome(() => new mod.PgletBtree().query("SELECT 1")),
};
// The chain is checked once per result, and page code the build itself calls can move it after
// the check — here a replaced `Uint8Array.prototype.set`, reached through the caught `bytea` copy,
// that copies and then puts a read-only index on `Array.prototype`: the next element store answers
// `false`, and that is the statement's error, never a dropped row. The index cases above left
// `Array.prototype.length` at 1 (deleting an index keeps the length), which alone keeps the store
// defining; it is put back so the set path is the one measured.
(Array.prototype as unknown as unknown[]).length = 0;
const copying_set = Uint8Array.prototype.set;
Uint8Array.prototype.set = function (this: Uint8Array, source: ArrayLike<number>, offset?: number) {
  copying_set.call(this, source, offset);
  Object.defineProperty(Array.prototype, "0", { configurable: true, writable: false, value: "inherited" });
} as typeof copying_set;
let late_rows = "none";
const late_read_only = outcome(() => {
  late_rows = String((db.query(`SELECT ARRAY[1,2] AS a, '\\x01ff'::bytea AS b`) as { rows: unknown[] }).rows.length);
});
Uint8Array.prototype.set = copying_set;
// @ts-ignore removing the property this test added
delete (Array.prototype as unknown as Record<string, unknown>)["0"];
report["late read-only index"] = { call: late_read_only, rows: late_rows };
// The same late step with a **non-throwing** index setter, which answers a set `true` and stores
// nothing: the store checks the array's own `length` moved past the element, so this too is the
// statement's error. Reached once through the `bytea` copy and once through the glue's
// `TextDecoder.prototype.decode`, which every string of the result passes through. And with a
// **throwing** one, which the element store's `Reflect.set` meets after the check: the import
// catches, so the throw is the statement's error and the module answers on.
const late_swallow = (install_through: "set" | "decode", hook: "swallow" | "throw" = "swallow") => {
  (Array.prototype as unknown as unknown[]).length = 0;
  const swallow = () =>
    Object.defineProperty(Array.prototype, "0", {
      configurable: true,
      set: hook === "swallow" ? (_v: unknown) => {} : thrower("late index setter threw"),
      get: () => undefined,
    });
  const set = Uint8Array.prototype.set;
  const decode = TextDecoder.prototype.decode;
  let armed = true;
  if (install_through === "set") {
    Uint8Array.prototype.set = function (this: Uint8Array, source: ArrayLike<number>, offset?: number) {
      set.call(this, source, offset);
      if (armed) (armed = false), swallow();
    } as typeof set;
  } else {
    let seen = 0;
    TextDecoder.prototype.decode = function (this: TextDecoder, ...args: Parameters<typeof decode>) {
      const text = decode.apply(this, args);
      if (armed && ++seen === 2) (armed = false), swallow();
      return text;
    };
  }
  let rows = "none";
  const call = outcome(() => {
    rows = String((db.query(`SELECT ARRAY[1,2] AS a, '\\x01ff'::bytea AS b, 'x' AS c`) as { rows: unknown[] }).rows.length);
  });
  Uint8Array.prototype.set = set;
  TextDecoder.prototype.decode = decode;
  // @ts-ignore removing the property this test added
  delete (Array.prototype as unknown as Record<string, unknown>)["0"];
  (Array.prototype as unknown as unknown[]).length = 0;
  return {
    call,
    rows,
    same: outcome(() => db.query(`SELECT ARRAY[1,2] AS a`)),
    fresh: outcome(() => new mod.PgletBtree().query(`SELECT ARRAY[1,2] AS a`)),
  };
};
report["late swallowing index (set)"] = late_swallow("set");
report["late swallowing index (decode)"] = late_swallow("decode");
report["late throwing index (decode)"] = late_swallow("decode", "throw");
// A bound `bytea` parameter is copied into the module through a caught binding too, so a page's
// replaced `Uint8Array.prototype.set` fails that statement and the module answers on.
Uint8Array.prototype.set = thrower("param set threw") as typeof copying_set;
const param_set = outcome(() => db.queryParams("SELECT $1::bytea AS b", [new Uint8Array([1, 2])]));
Uint8Array.prototype.set = copying_set;
report["param bytea set"] = {
  call: param_set,
  same: outcome(() => db.queryParams("SELECT $1::bytea AS b", [new Uint8Array([1, 2])])),
  fresh: outcome(() => new mod.PgletBtree().query("SELECT 1")),
};
// A replaced `Array.prototype.push` is the other half, and it is **not** the module's to
// catch: pglet stores a row at the array's own length now (`result::append`), but
// wasm-bindgen's generated glue grows its own object heap with `heap.push`, so a
// throwing one unwinds a call from inside the glue. Last, because it poisons the module.
const push = Array.prototype.push;
Array.prototype.push = thrower("push threw") as typeof push;
const pushed = outcome(() => db.query("SELECT 1 AS a, ARRAY[1,2] AS arr"));
Array.prototype.push = push;
report["glue push"] = {
  call: pushed,
  same: outcome(() => db.query("SELECT 1")),
  fresh: outcome(() => new mod.PgletBtree().query("SELECT 1")),
};
console.log(JSON.stringify(report));
