/**
 * Drives a host `RangeError` through a statement and prints, as one JSON line, what
 * the module answers afterwards: the error the statement threw, then what a new
 * instance, the instance that threw, and an instance created before it each answer.
 *
 * Run by `pglet_wasm_test.ts` in a child `node` with V8's stack shrunk (`--stack-size=…`), so
 * that a statement well inside the depth limit exhausts it — the one way to reach the
 * `RangeError` the limit otherwise keeps every statement from.
 *
 * Usage: `node --stack-size=<KiB> module_poison.ts <pkg dir URL>`.
 */

import { readFile } from "node:fs/promises";

const PKG = new URL(process.argv[2]);
const mod: any = await import(new URL("pglet_wasm.js", PKG).href);
mod.initSync({ module: await readFile(new URL("pglet_wasm_bg.wasm", PKG)) });

/** What `f` throws — its `name` and `code` — or `ok`. */
function outcome(f: () => unknown): string {
  try {
    f();
    return "ok";
  } catch (e) {
    const err = e as { name?: string; code?: string };
    return `${err.name}${err.code ? ` ${err.code}` : ""}`;
  }
}

const before = new mod.PgletBtree();
before.exec("CREATE TABLE t (a INT)");
const ran = new mod.PgletBtree();
const statement = outcome(() => ran.query("SELECT 1" + "+1".repeat(1200)));
const after: string[] = [];
// Past the six unwinds that once walked the shadow stack below zero.
for (let i = 0; i < 10; i++) {
  after.push(outcome(() => new mod.PgletBtree().exec("CREATE TABLE z (a INT)")));
}
console.log(JSON.stringify({
  statement,
  after,
  ran: outcome(() => ran.query("SELECT 1")),
  before: outcome(() => before.query("SELECT a FROM t")),
}));
