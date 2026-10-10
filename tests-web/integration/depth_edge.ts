/**
 * Drives deep-tree shapes to the wasm depth limit and past it, and prints one JSON line
 * per shape: the largest size answered, and every refusal seen.
 *
 * Run by `pglet_wasm_test.ts` under each of V8's tiers — the default (Liftoff first) and
 * `--v8-flags=--no-liftoff`, which compiles every function with the optimizing tier up
 * front. The limit (`pglet_sql::depth`'s wasm `DEFAULT_MAX_DEPTH`) must hold under both:
 * a host-stack `RangeError` here is the failure this file exists to see, and it leaves
 * the module refusing every later call (`PgletModulePoisoned`), so each shape after it
 * reports an error too.
 *
 * The shapes drive every node kind `parser::expr`'s operator loop builds around the chain
 * so far — a tree grows one level a repetition through each without parser nesting — each
 * in a `SELECT` and in a stored `CHECK` (which adds the catalog encoding, the render and
 * the DDL gates), and the plan chains and stored doors beside them. That the `ARMS` below
 * reach every such kind is not this file's claim: `pglet_sql`'s `operator_loop_arms` test
 * reads the kinds from the builders' source and fails when a piece repeated here builds
 * none of one. An arm whose chain pglet refuses by type answers that SQLSTATE below the
 * limit; any answer but a host error counts as answered, and past the limit the refusal
 * must be `54001`.
 *
 * Usage: `deno run -A [--v8-flags=--no-liftoff] depth_edge.ts <pkg dir URL>`.
 */

const [pkgUrl] = Deno.args;
const PKG = new URL(pkgUrl);
// deno-lint-ignore no-explicit-any
const mod: any = await import(new URL("pglet_wasm.js", PKG).href);
mod.initSync({ module: await Deno.readFile(new URL("pglet_wasm_bg.wasm", PKG)) });

const rep = (s: string, k: number) => s.repeat(k);
const TABLE = ["CREATE TABLE t (a INT)", "INSERT INTO t VALUES (1)"];
const cte = (k: number) => {
  let s = "WITH c0 AS (SELECT 1 AS x)";
  for (let i = 1; i < k; i++) s += `, c${i} AS (SELECT x FROM c${i - 1})`;
  return s + ` SELECT x FROM c${k - 1}`;
};

/**
 * The operator loop's arms (`Parser::operator_loop`), as the chain an arm grows at `n`
 * repetitions, and what one repetition counts against the limit. A comparison is not
 * associative, so its chain alternates with `IS TRUE`, two levels a repetition. A
 * `::regclass` target builds a string-function node, not a cast — walked from each
 * walker's big dispatch frame, so charged `DISPATCH_LEVEL_WEIGHT` (three) a level.
 */
export const ARMS: Record<string, [(n: number) => string, number]> = {
  "::": [(n) => `1${rep("::int", n)}`, 1],
  ":: with a typmod": [(n) => `'a'${rep("::varchar(3)", n)}`, 1],
  "::regclass": [(n) => `'t'${rep("::regclass", n)}`, 3],
  "AT TIME ZONE": [(n) => `now()${rep(" AT TIME ZONE 'UTC'", n)}`, 1],
  "IS NULL": [(n) => `1${rep(" IS NULL", n)}`, 1],
  "IS NOT NULL": [(n) => `1${rep(" IS NOT NULL", n)}`, 1],
  "IS TRUE": [(n) => `true${rep(" IS TRUE", n)}`, 1],
  "IS NOT UNKNOWN": [(n) => `true${rep(" IS NOT UNKNOWN", n)}`, 1],
  "IS DISTINCT FROM": [(n) => `true${rep(" IS DISTINCT FROM true", n)}`, 1],
  "IS NOT DISTINCT FROM": [(n) => `true${rep(" IS NOT DISTINCT FROM true", n)}`, 1],
  "IN list": [(n) => `true${rep(" IN (true)", n)}`, 1],
  "NOT IN list": [(n) => `true${rep(" NOT IN (false, NULL)", n)}`, 1],
  "IN subquery": [(n) => `true${rep(" IN (SELECT true)", n)}`, 1],
  "NOT IN subquery": [(n) => `true${rep(" NOT IN (SELECT false)", n)}`, 1],
  "LIKE": [(n) => `'a'${rep(" LIKE 'a'", n)}`, 1],
  "NOT ILIKE": [(n) => `'a'${rep(" NOT ILIKE 'b'", n)}`, 1],
  "comparison": [(n) => `true${rep(" = true IS TRUE", n)}`, 2],
  "ANY": [(n) => `true${rep(" = ANY(ARRAY[true]) IS TRUE", n)}`, 2],
  "ALL": [(n) => `true${rep(" <> ALL(ARRAY[false]) IS TRUE", n)}`, 2],
  "+": [(n) => `1${rep("+1", n)}`, 1],
  "*": [(n) => `1${rep("*1", n)}`, 1],
  "/": [(n) => `1${rep("/1", n)}`, 1],
  "%": [(n) => `1${rep("%2", n)}`, 1],
  "^": [(n) => `1.0::float8${rep("^1", n)}`, 1],
  "numeric +": [(n) => `1.5${rep("+1.5", n)}`, 1],
  "|| text": [(n) => `'a'${rep("||'a'", n)}`, 1],
  "|| jsonb": [(n) => `'{}'::jsonb${rep("||'{}'::jsonb", n)}`, 1],
  "->": [(n) => `'{}'::jsonb${rep("->'a'", n)}`, 1],
  "#>": [(n) => `'{}'::jsonb${rep("#>'{a}'", n)}`, 1],
  "@>": [(n) => `'{}'::jsonb${rep(" @> '{}' IS TRUE", n)}`, 2],
};

/** Each shape: setup, the statement at size `n`, what runs after it, and levels a size. */
type Shape = (n: number) => [string[], string, string[]];
const shapes: Record<string, [Shape, number]> = {};
for (const [arm, [grow, levels]] of Object.entries(ARMS)) {
  shapes[`${arm}, SELECT`] = [(n) => [[], `SELECT ${grow(n)}`, []], levels];
  shapes[`${arm}, CHECK`] = [(n) => [[], `CREATE TABLE c (a INT CHECK ((${grow(n)}) IS NOT NULL))`, [
    "INSERT INTO c VALUES (1)",
    "SELECT pg_get_constraintdef(oid) FROM pg_constraint",
  ]], levels];
}
Object.assign(shapes, {
  "DEFAULT chain": [(n: number) => [[], `CREATE TABLE c (a INT DEFAULT 1${rep("+1", n)})`, [
    "INSERT INTO c DEFAULT VALUES",
  ]], 1],
  "GROUP BY / HAVING": [(n: number) => [
    TABLE,
    `SELECT a${rep("+1", n)} FROM t GROUP BY a${rep("+1", n)} HAVING count(*)${rep("+1", n)} > 0`,
    [],
  ], 1],
  // A grouping query's expressions are walked once more than a plain one's — by the
  // collection of the aggregates each group computes — and then by the scalar walk,
  // over a chain above an aggregate: in the select list, an `ORDER BY` key and
  // `HAVING`, and through an arm that is not an arithmetic operator.
  "aggregate chain, select list": [(n: number) => [
    TABLE,
    `SELECT count(*)${rep("+1", n)} FROM t`,
    [],
  ], 1],
  "aggregate chain, ORDER BY": [(n: number) => [
    TABLE,
    `SELECT a FROM t GROUP BY a ORDER BY count(*)${rep("+1", n)}`,
    [],
  ], 1],
  "aggregate IN chain": [(n: number) => [
    TABLE,
    `SELECT (count(*) > 0)${rep(" IN (true)", n)} FROM t`,
    [],
  ], 1],
  "aggregate cast chain, HAVING": [(n: number) => [
    TABLE,
    `SELECT a FROM t GROUP BY a HAVING count(*)${rep("::int8", n)} > 0`,
    [],
  ], 1],
  "index expression": [(n: number) => [["CREATE TABLE c (a INT)"], `CREATE INDEX ci ON c ((a${rep("+1", n)}))`, [
    "INSERT INTO c VALUES (1)",
  ]], 1],
  "WHERE over an IN chain": [(n: number) => [
    TABLE,
    `SELECT count(*) FROM t WHERE (a${rep(" IN (a)", n)}) IS NOT NULL`,
    [],
  ], 1],
  "CTE chain": [(n: number) => [[], cte(n + 1), []], 2],
  "UNION ALL chain under EXISTS": [(n: number) => [
    TABLE,
    `SELECT count(*)::int4 FROM t WHERE EXISTS (SELECT 1${rep(" UNION ALL SELECT 1", n)})`,
    [],
  ], 5],
  "$N chain": [(n: number) => [[], `SELECT $1::int4${rep("+$1::int4", n)}`, []], 1],
} as Record<string, [Shape, number]>);

/** `ok`, or the SQLSTATE the statements answered; a non-SQL error is thrown. */
function outcome(build: Shape, n: number): string {
  const [setup, sql, after] = build(n);
  const db = new mod.PgletBtree();
  for (const s of setup) db.exec(s);
  try {
    if (sql.includes("$1")) db.queryParams(sql, [1]);
    else db.query(sql);
    for (const s of after) db.exec(s);
  } catch (e) {
    const code = (e as { code?: string }).code;
    if (code === undefined) throw e;
    // The instance that refused answers the next statement.
    db.query("SELECT 1");
    return code;
  }
  db.query("SELECT 1");
  return "ok";
}

for (const [name, [build, levels]] of Object.entries(shapes)) {
  const refusals = new Set<string>();
  // Anything but the depth refusal is an answer: an arm pglet refuses by type answers
  // that SQLSTATE below the limit, and must still answer it rather than overflow.
  const answered = (n: number) => {
    const got = outcome(build, n);
    if (got === "54001") refusals.add(got);
    return got !== "54001";
  };
  let lo = 1;
  let hi = 8192;
  try {
    while (hi - lo > 1) {
      const mid = Math.floor((lo + hi) / 2);
      if (answered(mid)) lo = mid;
      else hi = mid;
    }
    for (const n of [lo + 1, lo + 8, 2 * lo, 20_000]) {
      if (answered(n)) refusals.add(`answered ${n}`);
    }
    console.log(JSON.stringify({ name, answered: lo, levels: lo * levels, refusals: [...refusals] }));
  } catch (e) {
    console.log(JSON.stringify({ name, error: String(e) }));
  }
}
