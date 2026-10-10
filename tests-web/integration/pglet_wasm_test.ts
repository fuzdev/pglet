/**
 * Behavioral tests for the in-memory B+tree wasm bindings (`PgletBtree`).
 *
 * Covers the surface the native-server SQL fixtures can't: the wasm
 * **bound-parameter** path (`queryParams`) and **transactions** over the
 * in-memory CoW B+tree (real `BEGIN`/`COMMIT`/`ROLLBACK`). These back the
 * fuz_app in-process `db` matrix leg; the native server has its own coverage.
 *
 * Loads the `--target web` pkg the same way the benches do (read the `.wasm`
 * bytes + `initSync`), so it runs under `deno test` without a browser.
 */

import { assertEquals } from "@std/assert";

const PKG = new URL("../../crates/pglet_wasm/pkg/", import.meta.url);
// deno-lint-ignore no-explicit-any
const mod: any = await import(new URL("pglet_wasm.js", PKG).href);
mod.initSync({ module: await Deno.readFile(new URL("pglet_wasm_bg.wasm", PKG)) });

const num = (v: unknown): number => Number(v as bigint | number);

Deno.test("wasm default build - exports PgletBtree and carries no OPFS persistence layer", async () => {
  // The OPFS-backed `PgletBtreePersistent` is the crate's opt-in `opfs` feature; the default
  // build — this pkg, the npm package's shape, what the size gate measures — must carry
  // neither the class nor the OPFS imports its shim calls. Read three ways: the glue's
  // exports, and the module's own exports and imports.
  assertEquals(typeof mod.PgletBtree, "function");
  assertEquals("PgletBtreePersistent" in mod, false);
  const wasm = await WebAssembly.compile(await Deno.readFile(new URL("pglet_wasm_bg.wasm", PKG)));
  const exports = WebAssembly.Module.exports(wasm).map((e) => e.name);
  const imports = WebAssembly.Module.imports(wasm).map((e) => e.name);
  assertEquals(exports.includes("pgletbtree_new"), true, exports.join(" "));
  assertEquals(exports.filter((n) => /persistent/i.test(n)), []);
  assertEquals(imports.filter((n) => /SyncAccessHandle|getDirectory|getFileHandle/.test(n)), []);
});

Deno.test("wasm queryParams - typed binding (text/array/jsonb/uuid/null)", () => {
  const db = new mod.PgletBtree();
  db.exec(
    `CREATE TABLE t (id BIGSERIAL PRIMARY KEY, name TEXT, tags TEXT[], meta JSONB, uid UUID)`,
  );
  db.queryParams(`INSERT INTO t (name, tags, meta, uid) VALUES ($1, $2, $3, $4)`, [
    "alice",
    ["x", "y"],
    { kind: "doc", n: 3 },
    "00000000-0000-0000-0000-000000000001",
  ]);
  db.queryParams(`INSERT INTO t (name, tags, meta, uid) VALUES ($1, $2, $3, $4)`, [
    "bob",
    ["z"],
    { kind: "note" },
    null,
  ]);

  const row = db.queryParams(`SELECT id, name, tags, meta, uid FROM t WHERE name = $1`, [
    "alice",
  ]).rows[0];
  assertEquals(num(row.id), 1);
  assertEquals(row.name, "alice");
  assertEquals(row.tags, ["x", "y"]);
  assertEquals(row.meta, { kind: "doc", n: 3 });
  assertEquals(row.uid, "00000000-0000-0000-0000-000000000001");

  // array-containment with a JS array param
  const tagged = db.queryParams(`SELECT name FROM t WHERE tags @> $1 ORDER BY name`, [["y"]]);
  assertEquals(tagged.rows.map((r: { name: string }) => r.name), ["alice"]);

  // null-safe compare with a NULL param
  const nulls = db.queryParams(`SELECT name FROM t WHERE uid IS NOT DISTINCT FROM $1`, [null]);
  assertEquals(nulls.rows.map((r: { name: string }) => r.name), ["bob"]);
});

Deno.test("wasm queryParams - a jsonb and an array key hold, probe and order by the bound value", () => {
  // The ordinary idioms over a container key, through the bound-parameter door: a
  // `text[] UNIQUE`, a `jsonb` primary key, `ON CONFLICT` on each, and the composite
  // `UNIQUE (tenant, payload)`. A JS array binds as the array and a JS object as jsonb.
  const db = new mod.PgletBtree();
  const code = (run: () => unknown): string | undefined => {
    try {
      run();
    } catch (e) {
      return (e as { code?: string }).code;
    }
  };
  db.exec(`CREATE TABLE doc (n INT, tags TEXT[] UNIQUE, k JSONB PRIMARY KEY)`);
  db.exec(`CREATE TABLE ev (v TEXT, payload JSONB, tenant INT, UNIQUE (tenant, payload))`);
  db.queryParams(`INSERT INTO doc (k, tags, n) VALUES ($1, $2, $3)`, [{ id: 1 }, ["a", "b"], 1]);
  db.queryParams(`INSERT INTO doc (k, tags, n) VALUES ($1, $2, $3)`, [[1, 2], ["ab"], 2]);
  db.queryParams(`INSERT INTO doc (k, tags, n) VALUES ($1, $2, $3)`, [{ id: 2 }, null, 3]);

  // A duplicate of each key is refused; `{id: 1.0}` is the key `{id: 1}`.
  assertEquals(
    code(() => db.queryParams(`INSERT INTO doc (k, n) VALUES ($1::jsonb, 9)`, ['{"id":1.0}'])),
    "23505",
  );
  assertEquals(
    code(() => db.queryParams(`INSERT INTO doc (k, tags, n) VALUES ($1, $2, 9)`, [{ id: 9 }, ["a", "b"]])),
    "23505",
  );

  // `ON CONFLICT` arbitrates on the key, and a bound probe finds its row.
  db.queryParams(
    `INSERT INTO doc (k, tags, n) VALUES ($1, $2, 10) ON CONFLICT (tags) DO UPDATE SET n = doc.n + excluded.n`,
    [{ id: 10 }, ["a", "b"]],
  );
  db.queryParams(
    `INSERT INTO doc (k, n) VALUES ($1, 20) ON CONFLICT (k) DO UPDATE SET n = doc.n + excluded.n`,
    [[1, 2]],
  );
  assertEquals(
    db.queryParams(`SELECT n FROM doc WHERE tags = $1`, [["a", "b"]]).rows.map((r: { n: number }) => r.n),
    [11],
  );
  assertEquals(
    db.queryParams(`SELECT n FROM doc WHERE k = $1`, [[1, 2]]).rows.map((r: { n: number }) => r.n),
    [22],
  );
  // The primary key's order: an array below an object, objects by their pairs.
  assertEquals(
    db.query(`SELECT n FROM doc ORDER BY k`).rows.map((r: { n: number }) => r.n),
    [22, 11, 3],
  );

  // The one shape refused: a plain index on the array column alone. `USING gin` and a
  // second column both build.
  assertEquals(code(() => db.exec(`CREATE INDEX doc_tags ON doc (tags)`)), "0A000");
  db.exec(`CREATE INDEX doc_tags_g ON doc USING gin (tags)`);
  db.exec(`CREATE INDEX doc_tags_c ON doc (tags, n)`);
  assertEquals(
    db.queryParams(`SELECT n FROM doc WHERE tags @> $1`, [["b"]]).rows.map((r: { n: number }) => r.n),
    [11],
  );

  // The composite key: one payload per tenant.
  db.queryParams(`INSERT INTO ev (tenant, payload, v) VALUES ($1, $2, $3)`, [1, { k: 1 }, "a"]);
  db.queryParams(`INSERT INTO ev (tenant, payload, v) VALUES ($1, $2, $3)`, [2, { k: 1 }, "b"]);
  assertEquals(
    code(() => db.queryParams(`INSERT INTO ev (payload, v, tenant) VALUES ($1, $2, $3)`, [{ k: 1 }, "c", 1])),
    "23505",
  );
  assertEquals(num(db.query(`SELECT count(*) AS c FROM ev`).rows[0].c), 2);
});

Deno.test("wasm query - a timestamptz[] element keeps its +00 and an out-of-range offset is refused", () => {
  // A JS array element is a string the caller reparses, so a `timestamptz[]` element
  // needs its `+00` as a scalar does — including one the `ARRAY[…]` constructor built,
  // whose value records no tz bit. A `timestamp[]` element carries none.
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE t (a TIMESTAMPTZ[], b TIMESTAMP[])`);
  db.queryParams(`INSERT INTO t VALUES ($1, $2)`, [
    ["2000-01-01 00:00:00+05:30:15", null],
    ["2000-01-01 00:00:00+05"],
  ]);
  const row = db.query(
    `SELECT a, b, ARRAY[timestamptz '2000-01-01 00:00:00+05'] AS c, a::text AS d FROM t`,
  ).rows[0];
  assertEquals([row.a, row.b, row.c, row.d], [
    ["1999-12-31 18:29:45+00", null],
    ["2000-01-01 00:00:00"],
    ["1999-12-31 19:00:00+00"],
    '{"1999-12-31 18:29:45+00",NULL}',
  ]);
  let refused = "";
  try {
    db.queryParams(`SELECT $1::timestamptz`, ["2000-01-01 00:00:00+16:00"]);
  } catch (e) {
    const err = e as { code?: string; message?: string };
    refused = `${err.code} ${err.message}`;
  }
  assertEquals(refused, '22009 time zone displacement out of range: "2000-01-01 00:00:00+16:00"');
});

Deno.test("wasm queryParams - date and timestamp infinity bind, store, index, order and render", () => {
  // The wasm host renders a temporal value through the shared text encoders and binds a
  // JS string through the text readers, so both infinities cross the boundary as the
  // words 18.6 prints (`infinity`, no `+00` on a `timestamptz`).
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE t (id INT, d DATE PRIMARY KEY, tz TIMESTAMPTZ)`);
  db.exec(`CREATE INDEX t_tz ON t (tz)`);
  db.queryParams(`INSERT INTO t VALUES ($1, $2, $3)`, [1, "infinity", "-infinity"]);
  db.queryParams(`INSERT INTO t VALUES ($1, $2, $3)`, [2, "-infinity", "infinity"]);
  db.queryParams(`INSERT INTO t VALUES ($1, $2, $3)`, [3, "2000-01-01", "2000-01-01 00:00:00+00"]);
  const rows = db.queryParams(`SELECT id, d, tz FROM t WHERE d > $1 ORDER BY d`, ["1999-01-01"]).rows;
  assertEquals(rows.map((r: { id: number; d: string; tz: string }) => [r.id, r.d, r.tz]), [
    [3, "2000-01-01", "2000-01-01 00:00:00+00"],
    [1, "infinity", "-infinity"],
  ]);
  const later = db.queryParams(`SELECT id FROM t WHERE tz > $1 ORDER BY tz`, ["-infinity"]).rows;
  assertEquals(later.map((r: { id: number }) => r.id), [3, 2]);
  const fin = db.query(`SELECT isfinite(d) AS f, d + 1 AS n FROM t WHERE id = 1`).rows[0];
  assertEquals([fin.f, fin.n], [false, "infinity"]);
});

Deno.test("wasm queryParams - a non-finite JS number binds as each target type reads it", () => {
  // The binder spells a non-finite number as JS's `String` does (`Infinity`), which the
  // date and timestamp readers take as their infinity and a float8 reader as the number,
  // and which a text target stores as node-postgres would send it. A jsonb target goes
  // through `JSON.stringify`, which writes `null`.
  const db = new mod.PgletBtree();
  const row = db.queryParams(
    `SELECT $1::date::text AS d, $2::timestamptz::text AS tz, $3::float8 AS f, $4::text AS t, $5::jsonb::text AS j, $6::date[]::text AS ds`,
    [Infinity, -Infinity, -Infinity, Infinity, [Infinity], [-Infinity]],
  ).rows[0];
  assertEquals([row.d, row.tz, row.f, row.t, row.j, row.ds], [
    "infinity",
    "-infinity",
    -Infinity,
    "Infinity",
    "[null]",
    "{-infinity}",
  ]);
});

Deno.test("wasm queryParams - bound text is validated in the server encoding, whatever the type", () => {
  // Each NUL is built at run time so this file carries none. 18.6 answers each
  // `22021` below for the corresponding parameter bound over the wire in text format,
  // an array spelled as `{…}` text (measured): `exec_bind_message` validates a
  // parameter's text before its input function.
  const NUL = String.fromCharCode(0);
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE t (id INT, a TEXT)`);
  const outcome = (sql: string, params: unknown[]): string => {
    try {
      db.queryParams(sql, params);
    } catch (e) {
      const err = e as { code?: string; message?: string };
      return `${err.code} ${err.message}`;
    }
    return "accepted";
  };
  const refused = `22021 invalid byte sequence for encoding "UTF8": 0x00`;

  // First, because it is the one that separates a check on the whole parameter from a
  // check per element: the first element fails `int4`'s input before the NUL is read,
  // and 18.6 answers `{x,1<NUL>}` with `22021`.
  assertEquals(outcome(`SELECT $1::int4[] AS v`, [["x", "1" + NUL]]), refused);
  assertEquals(outcome(`SELECT $1::int4 AS v`, ["1" + NUL]), refused);
  assertEquals(outcome(`SELECT length($1::text) AS n`, ["x" + NUL + "y"]), refused);
  assertEquals(outcome(`SELECT $1 AS v`, ["x" + NUL]), refused);
  assertEquals(outcome(`SELECT $1::text[] AS v`, [["a" + NUL]]), refused);
  assertEquals(outcome(`SELECT $1::jsonb AS v`, ['"a' + NUL + '"']), refused);
  assertEquals(outcome(`INSERT INTO t VALUES (2, $1)`, ["x" + NUL + "y"]), refused);
  assertEquals(num(db.query(`SELECT count(*)::int8 AS c FROM t`).rows[0].c), 0);

  // The order is per parameter, as on 18.6: `$1` fails its input before `$2` is read.
  assertEquals(outcome(`SELECT $1::int4 AS a, $2::text AS b`, ["x", "a" + NUL]).slice(0, 5), "22P02");
  // A jsonb object is serialized first, and `JSON.stringify` writes the NUL as an
  // escape, which jsonb refuses as its own `22P05`.
  assertEquals(outcome(`SELECT $1::jsonb AS v`, [{ k: "a" + NUL }]).slice(0, 5), "22P05");
  // So is a JS array bound to a jsonb target: its string items are not scanned as text.
  assertEquals(outcome(`SELECT $1::jsonb AS v`, [["a" + NUL]]).slice(0, 5), "22P05");
  // The accept side: a multi-byte character binds.
  const accented = "x" + String.fromCharCode(0xe9) + "y";
  assertEquals(num(db.queryParams(`SELECT length($1::text) AS n`, [accented]).rows[0].n), 3);
});

Deno.test("wasm queryParams - a bound value at an enum position is converted, member or not", () => {
  const db = new mod.PgletBtree();
  db.exec(`CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')`);
  db.exec(`CREATE TABLE t (id INT, m mood NOT NULL)`);
  db.exec(`INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy')`);
  db.exec(`CREATE TABLE tx (v TEXT)`);

  // 18.6 runs `enum_in` on the bound value at Bind, so a non-member refuses before
  // anything executes — a read, a write, and a predicate no row reaches alike. This host
  // reaches the gate through `Database::execute_bound`, which the native server does not,
  // so a green fixture corpus says nothing about it.
  for (
    const sql of [
      `SELECT m > $1 FROM t`,
      `SELECT greatest(m, $1) FROM t`,
      `SELECT m IN ($1) FROM t`,
      `DELETE FROM t WHERE m < $1`,
      `INSERT INTO tx SELECT greatest(m, $1) FROM t`,
      `SELECT m > $1 FROM t WHERE false`,
      // A cast of the parameter to the enum is the same conversion at the same point.
      `SELECT $1::mood::text AS v FROM t WHERE false`,
      `DELETE FROM t WHERE m = $1::mood`,
    ]
  ) {
    let message = "";
    try {
      db.queryParams(sql, ["zzz"]);
    } catch (e) {
      message = String(e);
    }
    assertEquals(
      message.includes(`invalid input value for enum mood: "zzz"`),
      true,
      `${sql}: ${message || "accepted"}`,
    );
  }

  // The rows a refused write leaves, and the accept polarity beside it: a member binds
  // and orders by the enum's member positions, so only `happy` is greater than `ok`.
  assertEquals(num(db.query(`SELECT count(*) AS n FROM t`).rows[0].n), 3);
  assertEquals(num(db.query(`SELECT count(*) AS n FROM tx`).rows[0].n), 0);
  assertEquals(
    db.queryParams(`SELECT m::text AS label FROM t WHERE m > $1 ORDER BY id`, ["ok"])
      .rows.map((r: { label: string }) => r.label),
    ["happy"],
  );
});

Deno.test("wasm transactions - a savepoint unwinds its own level, and an unmodeled end statement is loud", () => {
  // The wasm binding applies the transaction lifecycle from a signal the executed
  // statement carries, never from its command tag. Both halves of that rule are here,
  // and each was a real bug in turn. Deciding it from the SQL text matched `ROLLBACK TO
  // SAVEPOINT sp` on the bare `ROLLBACK` prefix and discarded the whole transaction —
  // success, tag `ROLLBACK`, every write since `BEGIN` gone. Deciding it from the *tag*
  // is the same bug one layer along, because `ROLLBACK TO` reports tag `ROLLBACK` on
  // Postgres too, so it survives any amount of care about which spellings the parser
  // accepts.
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE t (id INT)`);
  db.exec(`INSERT INTO t VALUES (1)`);
  const count = () => num(db.query(`SELECT count(*)::int8 AS c FROM t`).rows[0].c);

  db.exec("BEGIN");
  db.exec(`INSERT INTO t VALUES (2)`);
  db.exec("SAVEPOINT sp");
  db.exec(`INSERT INTO t VALUES (3)`);
  assertEquals(count(), 3);
  // Only the savepoint's level unwinds — the block and its earlier write stand, which
  // is what a tag-keyed dispatch gets wrong while reporting success.
  db.exec("ROLLBACK TO SAVEPOINT sp");
  assertEquals(count(), 2, "ROLLBACK TO must not discard the whole block");
  db.exec("RELEASE sp");
  db.exec("COMMIT");
  assertEquals(count(), 2, "the surviving level commits");

  // A spelling the parser does not model is loud, and — like any raising statement —
  // aborts the block, so the `COMMIT` after it converts to a rollback and publishes
  // nothing. A *readable* count after the refusal only holds on an engine that does
  // not track the aborted block, so it is not assertable here.
  db.exec("BEGIN");
  db.exec(`INSERT INTO t VALUES (9)`);
  let threw = false;
  try {
    db.exec("COMMIT AND CHAIN");
  } catch {
    threw = true;
  }
  assertEquals(threw, true, "COMMIT AND CHAIN must fail loud");
  assertEquals(db.query("COMMIT").command, "ROLLBACK", "the aborted block converts");
  assertEquals(count(), 2, "the aborted block publishes nothing");
});

Deno.test("wasm transactions - a spelling the parser accepts opens a real transaction", () => {
  // The other direction of the same rule. The first two returned tag `BEGIN` under
  // the text match while opening nothing (measured), so the write that followed
  // autocommitted and the `ROLLBACK` after it reverted nothing; `BEGIN WORK` is the
  // control, a spelling that text match did route.
  for (const begin of ["/* c */ BEGIN", "START\nTRANSACTION", "BEGIN WORK"]) {
    const db = new mod.PgletBtree();
    db.exec(`CREATE TABLE t (id INT)`);
    db.exec(`INSERT INTO t VALUES (1)`);
    db.exec(begin);
    db.exec(`INSERT INTO t VALUES (2)`);
    db.exec("ROLLBACK");
    assertEquals(
      num(db.query(`SELECT count(*)::int8 AS c FROM t`).rows[0].c),
      1,
      `${JSON.stringify(begin)} must open a transaction the ROLLBACK undoes`,
    );
  }
});

Deno.test("wasm transactions - ROLLBACK undoes, COMMIT persists", () => {
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE t (id BIGSERIAL PRIMARY KEY, name TEXT)`);
  db.queryParams(`INSERT INTO t (name) VALUES ($1)`, ["seed"]);

  const count = () => num(db.query(`SELECT count(*)::int8 AS c FROM t`).rows[0].c);

  db.exec("BEGIN");
  db.queryParams(`INSERT INTO t (name) VALUES ($1)`, ["temp"]);
  assertEquals(count(), 2);
  db.exec("ROLLBACK");
  assertEquals(count(), 1);

  db.exec("BEGIN");
  db.queryParams(`INSERT INTO t (name) VALUES ($1)`, ["kept"]);
  db.exec("COMMIT");
  assertEquals(count(), 2);
});

Deno.test("wasm DO - a failed DO leaves the database as it found it", () => {
  // A `DO` is one statement, so one that fails keeps nothing of its body: the table and
  // type it made are gone and the table it dropped is back with its rows. The binding
  // runs the statement through the same `Database` bracket, which rolls back only the
  // staged storage writes; the catalog half is the block's own undo.
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE keep (id INT PRIMARY KEY)`);
  db.exec(`INSERT INTO keep VALUES (1), (2)`);
  let code: string | undefined;
  try {
    db.exec(`DO $$ BEGIN CREATE TABLE ft (a INT); CREATE TYPE fe AS ENUM ('a'); DROP TABLE keep; INSERT INTO nosuch VALUES (1); END $$`);
  } catch (e) {
    code = (e as { code?: string }).code;
  }
  assertEquals(code, "42P01", "the body's own error");
  db.exec(`CREATE TABLE ft (a INT)`);
  db.exec(`INSERT INTO ft VALUES (1)`);
  db.exec(`CREATE TYPE fe AS ENUM ('z')`);
  assertEquals(num(db.query(`SELECT count(*)::int8 AS c FROM keep`).rows[0].c), 2);
});

Deno.test("wasm queryParams - a text parameter takes a byte view as text and an array as an array literal", () => {
  // The spellings node-postgres sends, which a `text` target cannot refuse: a
  // `Uint8Array`'s bytes are the text, a JS array is `{"a","b"}`. Other targets keep
  // their own readers.
  const db = new mod.PgletBtree();
  const one = (sql: string, value: unknown) => db.queryParams(sql, [value]).rows[0].x;
  assertEquals(one(`SELECT $1::text AS x`, new Uint8Array([97, 98])), "ab");
  assertEquals(one(`SELECT $1 AS x`, ["a", null, [1]]), `{"a",NULL,{"1"}}`);
  assertEquals(one(`SELECT $1::text AS x`, [new Uint8Array([1, 171])]), `{\\\\x01ab}`);
  assertEquals(one(`SELECT $1::text AS x`, 1e21), "1e+21");
  assertEquals([...one(`SELECT $1::bytea AS x`, new Uint8Array([97, 98])) as Uint8Array], [97, 98]);
  assertEquals(one(`SELECT $1::text[] AS x`, ["a", "b"]), ["a", "b"]);
  let code: string | undefined;
  try {
    db.queryParams(`SELECT $1::text AS x`, [new Uint8Array([255])]);
  } catch (e) {
    code = (e as { code?: string }).code;
  }
  assertEquals(code, "22021");
});

Deno.test("wasm queryParams - a $N the statement gives no type is refused before it binds", () => {
  // 18.6 refuses these in parse analysis; the binding's decoder, which would otherwise
  // read an untyped argument by its JS type, is never reached. Inside a block the
  // refusal fails the block, as an error at Parse does.
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE up (id INT PRIMARY KEY, name TEXT)`);
  db.exec(`INSERT INTO up VALUES (1, 'x')`);
  const code = (sql: string, params: unknown[]) => {
    try {
      db.queryParams(sql, params);
      return null;
    } catch (e) {
      return (e as { code?: string }).code ?? String(e);
    }
  };
  for (const value of [null, 5, "x", true]) {
    assertEquals(code(`SELECT $1 IS NULL AS b`, [value]), "42P18");
    assertEquals(code(`SELECT count(*) FROM up WHERE ($1 IS NULL OR name = $1)`, [value]), "42P08");
  }
  assertEquals(code(`SELECT count(*) FROM up WHERE (name = $1 OR $1 IS NULL)`, [null]), null);
  db.exec(`BEGIN`);
  db.exec(`INSERT INTO up VALUES (2, 'y')`);
  assertEquals(code(`DELETE FROM up WHERE $1 IS NULL`, [null]), "42P18");
  assertEquals(code(`SELECT 1`, []), "25P02");
  db.exec(`ROLLBACK`);
  assertEquals(num(db.query(`SELECT count(*)::int8 AS c FROM up`).rows[0].c), 1);
});

Deno.test("wasm generate_series - seeds a table, takes a bound parameter, and refuses past its bound", () => {
  // The seeding idiom on the npm package's store, and a bound as a parameter: a whole JS
  // number binds as the `integer` the overload gives a bare `$N`, a fractional one is
  // refused as that type's input refuses it, and a series longer than the bound is
  // refused before a row is built.
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE gs (id INT PRIMARY KEY, label TEXT)`);
  db.exec(`INSERT INTO gs SELECT g, 'row ' || g FROM generate_series(1, 1000) g`);
  const seeded = db.query(`SELECT count(*) AS c, sum(id) AS s FROM gs`).rows[0];
  assertEquals([num(seeded.c), num(seeded.s)], [1000, 500500]);
  const sql = `SELECT count(*) AS c, max(g) AS m FROM generate_series(1, $1) g`;
  const run = (params: unknown[]) => {
    try {
      const row = db.queryParams(sql, params).rows[0];
      return [num(row.c), row.m === null ? null : num(row.m)];
    } catch (e) {
      return (e as { code?: string }).code ?? String(e);
    }
  };
  assertEquals(run([3]), [3, 3]);
  assertEquals(run([5]), [5, 5]);
  assertEquals(run([null]), [0, null]);
  assertEquals(run([3.5]), "22P02");
  assertEquals(run([2_000_000]), "54000");
  assertEquals(run([0]), [0, null]);
  const big = db.queryParams(`SELECT max(g) AS m FROM generate_series(1::bigint, $1) g`, [4]).rows[0];
  assertEquals(num(big.m), 4);
});

Deno.test("wasm DO - a block's rollback restores a cell a completed DO replaced", () => {
  // A `DO` that completes hands the counter cells it replaced to the block with its
  // result, so the block's `ROLLBACK` puts the sequence back on its own cell: `4` after
  // three draws, as on 18.6 — handed nothing, the rollback left the restarted `100`.
  const db = new mod.PgletBtree();
  db.exec(`CREATE SEQUENCE s`);
  db.exec(`SELECT nextval('s'), nextval('s'), nextval('s')`);
  db.exec(`BEGIN`);
  db.exec(`DO $$ BEGIN ALTER SEQUENCE s RESTART WITH 100; END $$`);
  db.exec(`ROLLBACK`);
  assertEquals(num(db.query(`SELECT nextval('s') AS n`).rows[0].n), 4);
});

Deno.test("wasm now() - the step is a microsecond, so the values it separates render apart", () => {
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE ev (id INT, at TIMESTAMPTZ)`);
  // `Date.now()` is whole milliseconds; held at one, these three statements' starts are
  // separated only by `wall_clock`'s step.
  with_js_clock(() => {
    db.exec(`INSERT INTO ev VALUES (1, now())`);
    db.exec(`INSERT INTO ev VALUES (2, now())`);
    db.exec(`INSERT INTO ev VALUES (3, now())`);
  });

  const distinct_values = num(
    db.query(`SELECT count(*)::int8 AS c FROM (SELECT DISTINCT at FROM ev) d`).rows[0].c,
  );
  const distinct_renderings = num(
    db.query(`SELECT count(*)::int8 AS c FROM (SELECT DISTINCT at::text FROM ev) d`).rows[0].c,
  );

  // The point of the test is the *equality*, not either number: a nanosecond step was
  // invisible to all three encoders, so these were 3 and 1 — three
  // rows printing one identical string, and a `UNIQUE` column raising `23505` for
  // two values `WHERE a = b` called different. With a microsecond step the value
  // the executor compares and the value every encoder can carry are the same one.
  assertEquals(distinct_values, distinct_renderings);
  assertEquals(distinct_values, 3);
});

/** Run `body` with `Date.now` replaced by a counter over a clock the test moves. */
function with_js_clock<T>(body: (clock: { ms: number; reads: number }) => T): T {
  const real = Date.now;
  const clock = { ms: real(), reads: 0 };
  Date.now = () => {
    clock.reads += 1;
    return clock.ms;
  };
  try {
    return body(clock);
  } finally {
    Date.now = real;
  }
}

Deno.test("wasm clock - a statement reads the JS clock at most once, whatever it evaluates", () => {
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE d (x INT)`);
  db.exec(`INSERT INTO d VALUES (0), (1), (2), (3), (4), (5), (6), (7)`);
  db.exec(
    `CREATE TABLE ev (id INT, created TIMESTAMPTZ DEFAULT now(), updated TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
       day DATE DEFAULT CURRENT_DATE, stmt TIMESTAMPTZ DEFAULT statement_timestamp(), written TIMESTAMPTZ)`,
  );
  with_js_clock((clock) => {
    // 64 rows, four clock-family defaults and one written `now()` each: one read.
    db.exec(`INSERT INTO ev (id, written) SELECT a.x * 8 + b.x, now() FROM d a, d b`);
    assertEquals(clock.reads, 1, "a multi-row INSERT under now() defaults");

    clock.reads = 0;
    const row = db.query(
      `SELECT now() = now() AS same, now() = statement_timestamp() AS stmt,
              now() - now() = INTERVAL '0' AS zero, LOCALTIMESTAMP = LOCALTIMESTAMP AS local,
              CURRENT_DATE = CURRENT_DATE AS day, LOCALTIME = LOCALTIME AS tod`,
    ).rows[0];
    assertEquals(clock.reads, 1, "a SELECT reading the family eleven times");
    assertEquals(row, { same: true, stmt: true, zero: true, local: true, day: true, tod: true });

    clock.reads = 0;
    db.queryParams(
      `UPDATE ev SET written = now(), created = DEFAULT, updated = CURRENT_TIMESTAMP WHERE id >= $1`,
      [0],
    );
    assertEquals(clock.reads, 1, "a bound UPDATE over every row");

    // A statement that evaluates none of the family does not call out at all, and a
    // `BEGIN` reads the block's start.
    clock.reads = 0;
    db.exec(`SELECT count(*) FROM ev`);
    assertEquals(clock.reads, 0, "a statement reading no clock");
    db.exec(`BEGIN`);
    assertEquals(clock.reads, 1, "BEGIN");
    db.exec(`SELECT now(), now()`);
    db.exec(`COMMIT`);
    assertEquals(clock.reads, 1, "a block's statement reads the block's start");

    // The mirror: `clock_timestamp()` is a read of its own at each call, after the
    // statement's.
    clock.reads = 0;
    const wall = db.query(
      `SELECT clock_timestamp() > now() AS later, clock_timestamp() <> clock_timestamp() AS moves`,
    ).rows[0];
    assertEquals(clock.reads, 4, "the statement's read and three calls");
    assertEquals(wall, { later: true, moves: true });
  });
  const stamps = db.query(
    `SELECT count(DISTINCT created)::int4 AS created, count(DISTINCT stmt)::int4 AS stmt,
            sum(CASE WHEN created = updated AND created = written THEN 0 ELSE 1 END)::int4 AS differing
     FROM ev`,
  ).rows[0];
  assertEquals(stamps, { created: 1, stmt: 1, differing: 0 });
});

Deno.test("wasm clock - a block reads its start, a statement its own, the next transaction a later one", () => {
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE ev (id INT, tx TIMESTAMPTZ DEFAULT now(), stmt TIMESTAMPTZ DEFAULT statement_timestamp())`);
  with_js_clock((clock) => {
    db.exec(`BEGIN`);
    clock.ms += 5;
    db.exec(`INSERT INTO ev (id) VALUES (1)`);
    clock.ms += 5;
    db.queryParams(`INSERT INTO ev (id) VALUES ($1)`, [2]);
    db.exec(`SAVEPOINT s`);
    clock.ms += 5;
    db.exec(`ROLLBACK TO s`);
    db.exec(`INSERT INTO ev (id) VALUES (3)`);
    db.exec(`COMMIT`);
    clock.ms += 5;
    db.exec(`INSERT INTO ev (id) VALUES (4)`);
    // No time passes on the JS clock here: the step alone separates the two statements.
    db.exec(`INSERT INTO ev (id) VALUES (5)`);
    db.exec(`BEGIN`);
    db.exec(`INSERT INTO ev (id) VALUES (6)`);
    db.exec(`ROLLBACK`);
  });
  const row = db.query(
    `SELECT (SELECT count(DISTINCT tx) FROM ev WHERE id <= 3)::int4 AS block_starts,
            (SELECT count(DISTINCT stmt) FROM ev WHERE id <= 3)::int4 AS block_statements,
            (SELECT min(stmt) - min(tx) FROM ev WHERE id <= 3) > INTERVAL '4 milliseconds' AS first_statement_later,
            (SELECT count(*) FROM ev WHERE id IN (4, 5) AND tx = stmt)::int4 AS autocommit_agree,
            (SELECT count(DISTINCT tx) FROM ev WHERE id IN (4, 5))::int4 AS autocommit_starts,
            (SELECT max(stmt) FROM ev WHERE id <= 3) < (SELECT min(tx) FROM ev WHERE id > 3) AS next_is_later,
            (SELECT count(*) FROM ev)::int4 AS kept`,
  ).rows[0];
  assertEquals(row, {
    block_starts: 1,
    block_statements: 3,
    first_statement_later: true,
    autocommit_agree: 2,
    autocommit_starts: 2,
    next_is_later: true,
    kept: 5,
  });
});

Deno.test("wasm clock - a JS clock set back is followed, in every database of the module", () => {
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE ev (id INT, created TIMESTAMPTZ DEFAULT now(), day DATE DEFAULT CURRENT_DATE)`);
  const text = (on: any, sql: string) => on.query(sql).rows[0].x as string;
  with_js_clock((clock) => {
    // A read under today's clock, then a test runner's fake timer pinned to a past date.
    db.exec(`INSERT INTO ev (id) VALUES (0)`);
    clock.ms = Date.UTC(2024, 0, 1);
    db.exec(`INSERT INTO ev (id) VALUES (1)`);
    // The first read after the set-back is the pinned millisecond itself, and each read
    // after it in that millisecond one microsecond on — from the pinned instant, in this
    // database and in one made after the set-back: the module has one clock.
    assertEquals(text(db, `SELECT created::text AS x FROM ev WHERE id = 1`), "2024-01-01 00:00:00+00");
    assertEquals(text(db, `SELECT day::text AS x FROM ev WHERE id = 1`), "2024-01-01");
    assertEquals(text(db, `SELECT now()::text AS x`), "2024-01-01 00:00:00.000001+00");
    assertEquals(text(db, `SELECT CURRENT_DATE::text AS x`), "2024-01-01");
    const second = new mod.PgletBtree();
    assertEquals(text(second, `SELECT now()::text AS x`), "2024-01-01 00:00:00.000003+00");
    assertEquals(text(db, `SELECT now()::text AS x`), "2024-01-01 00:00:00.000004+00");
    // Back again, to a millisecond between the two: followed, to the millisecond.
    clock.ms = Date.UTC(2025, 5, 15, 12);
    assertEquals(text(db, `SELECT now()::text AS x`), "2025-06-15 12:00:00+00");
    clock.ms = Date.UTC(2024, 0, 1);
    assertEquals(text(second, `SELECT now()::text AS x`), "2024-01-01 00:00:00+00");
  });
});

Deno.test("wasm clock - a JS clock pinned forward is followed, and so is the real clock once it is restored", () => {
  const db = new mod.PgletBtree();
  const text = (sql: string) => db.query(sql).rows[0].x as string;
  with_js_clock((clock) => {
    db.exec(`SELECT now()`);
    clock.ms = Date.UTC(2099, 0, 1);
    assertEquals(text(`SELECT now()::text AS x`), "2099-01-01 00:00:00+00");
    assertEquals(text(`SELECT CURRENT_DATE::text AS x`), "2099-01-01");
  });
  // `Date.now` is the real one again: a read is between two readings of it taken around
  // the statement, where a step kept from 2099 would answer 2099 until the real clock reached it.
  const before = new Date(Date.now()).toISOString();
  const row = db.queryParams(
    `SELECT now() >= $1::timestamptz AS not_earlier, now() < $1::timestamptz + INTERVAL '1 minute' AS not_later`,
    [before],
  ).rows[0];
  assertEquals(row, { not_earlier: true, not_later: true });
});

Deno.test("wasm clock - a frozen JS clock is answered inside its millisecond at any read count, across databases", () => {
  const a = new mod.PgletBtree();
  const b = new mod.PgletBtree();
  for (const db of [a, b]) db.exec(`CREATE TABLE ev (id INT, at TIMESTAMPTZ DEFAULT now())`);
  const frozen = Date.UTC(2024, 0, 1);
  const stamps: string[] = [];
  const now = (db: any) => db.query(`SELECT now()::text AS x`).rows[0].x as string;
  with_js_clock((clock) => {
    clock.ms = frozen - 1;
    a.exec(`SELECT now()`);
    clock.ms = frozen;
    // 2500 statements in one millisecond of the JS clock, alternating between two
    // databases. The first five hundred starts are a microsecond apart, in statement
    // order; the five-hundredth is the tie point, 499 microseconds in, and every start
    // after it is that microsecond again — a tie, where one more step would round to the
    // next millisecond in a `timestamptz(3)` column.
    for (let i = 0; i < 2500; i++) {
      const db = i % 2 === 0 ? a : b;
      db.exec(`INSERT INTO ev (id) VALUES (${i})`);
      stamps.push(db.query(`SELECT at::text AS x FROM ev WHERE id = ${i}`).rows[0].x);
    }
    assertEquals(stamps[0], "2024-01-01 00:00:00+00");
    assertEquals(stamps[1], "2024-01-01 00:00:00.000001+00");
    assertEquals(stamps[498], "2024-01-01 00:00:00.000498+00");
    assertEquals(stamps[499], "2024-01-01 00:00:00.000499+00");
    assertEquals(stamps[500], "2024-01-01 00:00:00.000499+00");
    assertEquals(new Set(stamps.slice(0, 500)).size, 500);
    assertEquals(stamps.every((at, i) => i === 0 || (i < 500 ? stamps[i - 1] < at : stamps[i - 1] === at)), true);
    // What a tie is inside one statement: the wall clock equal to the statement's start,
    // and to itself — never earlier.
    assertEquals(
      a.query(
        `SELECT clock_timestamp() = now() AS tied, clock_timestamp() < now() AS inverted,
                clock_timestamp() = clock_timestamp() AS still, now()::text AS at`,
      ).rows[0],
      { tied: true, inverted: false, still: true, at: "2024-01-01 00:00:00.000499+00" },
    );
    // Pinned again to the same millisecond — a new `Date.now`, no other read between — and
    // in a database made under it: still inside the millisecond.
    const pinned = Date.now;
    Date.now = () => pinned();
    assertEquals(now(a), "2024-01-01 00:00:00.000499+00");
    assertEquals(now(new mod.PgletBtree()), "2024-01-01 00:00:00.000499+00");
    Date.now = pinned;
    // The JS clock moves one millisecond: the next start is that millisecond, exactly,
    // and later than every start before it.
    clock.ms = frozen + 1;
    assertEquals(now(a), "2024-01-01 00:00:00.001+00");
    assertEquals(now(b), "2024-01-01 00:00:00.001001+00");
    // Back to the spent millisecond: its start again.
    clock.ms = frozen;
    assertEquals(now(a), "2024-01-01 00:00:00+00");
    clock.ms = frozen + 3;
    assertEquals(now(a), "2024-01-01 00:00:00.003+00");
  });
});

Deno.test("wasm clock - a frozen clock's value is its millisecond in a column of precision 3 or finer, at any read count", () => {
  // A `timestamptz(3)` rounds half away from the value's zero, 2000-01-01: one pinned
  // millisecond after it, one between it and 1970, one before 1970.
  for (const [frozen, at] of [
    [Date.UTC(2024, 0, 1, 0, 0, 0, 678), "2024-01-01 00:00:00.678"],
    [Date.UTC(1985, 10, 5, 0, 0, 0, 678), "1985-11-05 00:00:00.678"],
    [Date.UTC(1960, 5, 30, 0, 0, 0, 678), "1960-06-30 00:00:00.678"],
  ] as const) {
    const db = new mod.PgletBtree();
    db.exec(
      `CREATE TABLE ev (id INT, t3 TIMESTAMPTZ(3) DEFAULT now(), p3 TIMESTAMP(3) DEFAULT now(),
         w3 TIMESTAMPTZ(3) DEFAULT clock_timestamp(), t4 TIMESTAMPTZ(4) DEFAULT now(),
         t5 TIMESTAMPTZ(5) DEFAULT now(), t6 TIMESTAMPTZ DEFAULT now())`,
    );
    with_js_clock((clock) => {
      clock.ms = frozen - 1;
      db.exec(`SELECT now()`);
      clock.ms = frozen;
      for (let i = 0; i < 1200; i++) db.exec(`INSERT INTO ev (id) VALUES (${i})`);
      const row = db.query(
        `SELECT count(*)::int4 AS rows,
                sum(CASE WHEN t3::text = '${at}+00' THEN 1 ELSE 0 END)::int4 AS t3,
                sum(CASE WHEN p3::text = '${at}' THEN 1 ELSE 0 END)::int4 AS p3,
                sum(CASE WHEN w3::text = '${at}+00' THEN 1 ELSE 0 END)::int4 AS w3,
                sum(CASE WHEN t4::text LIKE '${at}%' THEN 1 ELSE 0 END)::int4 AS t4,
                sum(CASE WHEN t5::text LIKE '${at}%' THEN 1 ELSE 0 END)::int4 AS t5,
                sum(CASE WHEN t6::text LIKE '${at}%' THEN 1 ELSE 0 END)::int4 AS t6,
                sum(CASE WHEN t3 < '${at}+00'::timestamptz + INTERVAL '1 millisecond' THEN 1 ELSE 0 END)::int4 AS before_next
         FROM ev`,
      ).rows[0];
      assertEquals(row, { rows: 1200, t3: 1200, p3: 1200, w3: 1200, t4: 1200, t5: 1200, t6: 1200, before_next: 1200 }, at);
      // Past the tie point, a cast rounds the same way.
      assertEquals(db.query(`SELECT now()::timestamptz(3)::text AS x`).rows[0].x, `${at}+00`, at);
      assertEquals(db.query(`SELECT clock_timestamp()::timestamp(3)::text AS x`).rows[0].x, at, at);
    });
  }
});

Deno.test("wasm clock - a block keeps its start when the JS clock is set back under it", () => {
  const db = new mod.PgletBtree();
  with_js_clock((clock) => {
    clock.ms = Date.UTC(2025, 0, 1);
    db.exec(`SELECT now()`);
    clock.ms = Date.UTC(2025, 0, 2);
    db.exec(`BEGIN`);
    clock.ms = Date.UTC(2024, 0, 1);
    // The block's start is an instant already taken; the statement's start and the wall
    // clock are read after the set-back and are earlier than it, as 18.6's are when the
    // system clock is set back inside a transaction.
    const row = db.query(
      `SELECT now()::text AS tx, transaction_timestamp() = now() AS same, CURRENT_DATE::text AS day,
              statement_timestamp()::text AS stmt, clock_timestamp()::text AS wall`,
    ).rows[0];
    assertEquals(row, {
      tx: "2025-01-02 00:00:00+00",
      same: true,
      day: "2025-01-02",
      stmt: "2024-01-01 00:00:00+00",
      wall: "2024-01-01 00:00:00.000001+00",
    });
    db.exec(`COMMIT`);
    // The next transaction starts on the clock as it now reads.
    assertEquals(db.query(`SELECT now()::text AS x`).rows[0].x, "2024-01-01 00:00:00.000002+00");
  });
});

Deno.test("wasm clock - a Date.now or Math.random answering NaN is refused, not read as zero", () => {
  const db = new mod.PgletBtree();
  const message = (sql: string) => {
    try {
      db.exec(sql);
      return null;
    } catch (e) {
      return String((e as Error).message);
    }
  };
  db.exec(`SELECT now()`);
  const now = Date.now;
  const random = Math.random;
  try {
    // `NaN` is a JS number; read as an integer it is zero — the first of January 1970.
    Date.now = () => NaN;
    assertEquals(message(`SELECT now()`)?.includes("Date.now() did not answer a number"), true);
    Math.random = () => NaN;
    assertEquals(message(`SELECT gen_random_uuid()`)?.includes("Math.random() did not answer a number"), true);
  } finally {
    Date.now = now;
    Math.random = random;
  }
  assertEquals(db.query(`SELECT now() > '2020-01-01'::timestamptz AS x, gen_random_uuid() IS NOT NULL AS u`).rows[0], {
    x: true,
    u: true,
  });
});

Deno.test("wasm uuid - gen_random_uuid is a v4 uuid of three Math.random draws, whatever the draws answer", () => {
  const db = new mod.PgletBtree();
  const uuid = () => db.query(`SELECT gen_random_uuid()::text AS u`).rows[0].u as string;
  const v4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
  const random = Math.random;
  let calls = 0;
  try {
    Math.random = () => {
      calls++;
      return random();
    };
    uuid();
    // The top 48 bits of each draw's mantissa: red at one draw a byte.
    assertEquals(calls, 3);
    const seen = new Set<string>();
    // Which values each of the 128 bit positions (0 = the text's first) has taken.
    const took_zero = new Array<boolean>(128).fill(false);
    const took_one = new Array<boolean>(128).fill(false);
    for (let i = 0; i < 500; i++) {
      const u = uuid();
      assertEquals(v4.test(u), true, u);
      seen.add(u);
      const hex = u.replaceAll("-", "");
      for (let bit = 0; bit < 128; bit++) {
        const set = (parseInt(hex[bit >> 2], 16) >> (3 - (bit & 3))) & 1;
        (set ? took_one : took_zero)[bit] = true;
      }
    }
    assertEquals(seen.size, 500);
    // The version nibble (bits 48–51) and the variant's two bits (64–65) are fixed; every
    // other position took both values. Over 500 draws a position that is random fails
    // this with probability 2^-499, so a mutation that leaves one constant — draws OR'd
    // unshifted, say — is red every run, not by chance. A shift that ORs two random bits
    // into one position leaves none constant, and this does not see it.
    const fixed = new Set([48, 49, 50, 51, 64, 65]);
    for (let bit = 0; bit < 128; bit++) {
      assertEquals(took_zero[bit] && took_one[bit], !fixed.has(bit), `bit ${bit}`);
    }
    // A page's own `Math.random` answering outside `[0, 1)` cannot trap the cast — below
    // 0 or from 2^16 up it saturates, in between a draw's bits past the 48th land in the
    // previous draw's place — and the version and variant bits are stamped over either.
    for (const answer of [0, 1, 1.5, 65535.9, 65536, 5, -1, Infinity, -Infinity, 1 - Number.EPSILON]) {
      Math.random = () => answer;
      assertEquals(v4.test(uuid()), true, String(answer));
    }
    Math.random = () => 0;
    assertEquals(uuid(), "00000000-0000-4000-8000-000000000000");
  } finally {
    Math.random = random;
  }
});

Deno.test("wasm random - random() is a float8 in [0, 1) of one Math.random draw a call, whatever the draw answers", () => {
  const db = new mod.PgletBtree();
  const random = Math.random;
  let calls = 0;
  try {
    Math.random = () => {
      calls++;
      return random();
    };
    // One draw a call, and a call per row: two calls in one row, eight rows of one.
    db.query(`SELECT random() AS a, random() AS b`);
    assertEquals(calls, 2);
    db.exec(`CREATE TABLE r (id INT)`);
    db.exec(`INSERT INTO r VALUES (1), (2), (3), (4), (5), (6), (7), (8)`);
    calls = 0;
    const rows = db.query(`SELECT random() AS v FROM r`).rows as { v: number }[];
    assertEquals(calls, 8);
    assertEquals(rows.every((r) => typeof r.v === "number" && r.v >= 0 && r.v < 1), true);
    assertEquals(new Set(rows.map((r) => r.v)).size > 1, true, "every row drew one value");
    // A page's own `Math.random` answering outside `[0, 1)` still yields a value in range.
    for (const answer of [0, 0.5, 1, 1.5, -1, 65536, Infinity, -Infinity, 1 - Number.EPSILON]) {
      Math.random = () => answer;
      const v = db.query(`SELECT random() AS v`).rows[0].v as number;
      assertEquals(v >= 0 && v < 1, true, `${answer} answered ${v}`);
    }
    Math.random = () => 0.5;
    assertEquals(db.query(`SELECT random() AS v`).rows[0].v, 0.5);
    Math.random = () => NaN;
    let refused = "";
    try {
      db.query(`SELECT random()`);
    } catch (e) {
      refused = String(e);
    }
    assertEquals(refused.includes("Math.random() did not answer a number"), true, refused);
  } finally {
    Math.random = random;
  }
});

Deno.test("wasm parsed statements - a held text binds by the catalog it runs against, rolled back or not", () => {
  // A statement's text is parsed once and held while the catalog stands. A bound `'01'`
  // is the integer 1 against an `INT` column and the text `01` against a `TEXT` one, so
  // which column type the binding read shows which catalog the held plan was parsed for.
  const db = new mod.PgletBtree();
  const sql = `SELECT a::text AS v FROM t WHERE a = $1`;
  const matches = () => db.queryParams(sql, ["01"]).rows.length;
  db.exec(`CREATE TABLE t (a INT)`);
  db.exec(`INSERT INTO t VALUES (1)`);
  assertEquals(matches(), 1, "INT");
  assertEquals(matches(), 1, "INT, held");
  const as_text = () => {
    db.exec(`DROP TABLE t`);
    db.exec(`CREATE TABLE t (a TEXT)`);
    db.exec(`INSERT INTO t VALUES ('1')`);
  };
  db.exec(`BEGIN`);
  as_text();
  assertEquals(matches(), 0, "TEXT, in the block");
  db.exec(`ROLLBACK`);
  assertEquals(matches(), 1, "INT, after the rollback");
  db.exec(`BEGIN`);
  db.exec(`SAVEPOINT s`);
  as_text();
  assertEquals(matches(), 0, "TEXT, past the savepoint");
  db.exec(`ROLLBACK TO s`);
  assertEquals(matches(), 1, "INT, rolled back to the savepoint");
  db.exec(`COMMIT`);
  as_text();
  assertEquals(matches(), 0, "TEXT, committed");
});

Deno.test("wasm parsed statements - a held text reads the block it runs in", () => {
  // A held statement's session is installed as any statement's is, so `now()` is the
  // start of the block it runs in, and after a block an autocommit statement's own.
  const db = new mod.PgletBtree();
  const now = () => String(db.query(`SELECT now()::text AS v`).rows[0].v);
  with_js_clock((clock) => {
    db.exec(`BEGIN`);
    const first = now();
    db.exec(`COMMIT`);
    clock.ms += 5;
    db.exec(`BEGIN`);
    const second = now();
    clock.ms += 5;
    assertEquals(now(), second, "one block, one start");
    db.exec(`COMMIT`);
    assertEquals(first === second, false, "a later block's start");
    const agree = `SELECT now() = statement_timestamp() AS v`;
    db.exec(`BEGIN`);
    db.query(agree);
    db.exec(`COMMIT`);
    clock.ms += 5;
    assertEquals(db.query(agree).rows[0].v, true);
  });
});

Deno.test("wasm clock - a JS clock read past the year 9999 is refused, and followed once it is put back", () => {
  const db = new mod.PgletBtree();
  with_js_clock((clock) => {
    const code = (sql: string) => {
      try {
        db.exec(sql);
        return null;
      } catch (e) {
        return (e as { code?: string }).code ?? String(e);
      }
    };
    clock.ms = Date.UTC(2024, 0, 1);
    assertEquals(code(`SELECT now()`), null);
    // Refused, with whichever code the clock's reader gives a year a timestamp cannot
    // hold; what this test is for is the read after it.
    clock.ms = Date.UTC(20000, 0, 1);
    assertEquals(code(`SELECT now()`) !== null, true);
    clock.ms = Date.UTC(2024, 0, 2);
    assertEquals(db.query(`SELECT now()::text AS x`).rows[0].x, "2024-01-02 00:00:00+00");
  });
});

/**
 * Every deep-tree shape at the depth limit and past it, run by `depth_edge.ts` in a child
 * Deno under each V8 tier: `flags` empty is the default (Liftoff first), and
 * `--no-liftoff` compiles every function optimized up front, whose frames are the larger.
 * The shapes are each arm of the parser's operator loop, in a `SELECT` and in a stored
 * `CHECK`, and the plan chains and stored doors beside them.
 */
async function depthEdges(flags: string[]): Promise<{ name: string; answered?: number; levels?: number; refusals?: string[]; error?: string }[]> {
  const out = await new Deno.Command(Deno.execPath(), {
    args: [
      "run",
      "-A",
      ...(flags.length ? [`--v8-flags=${flags.join(",")}`] : []),
      new URL("depth_edge.ts", import.meta.url).pathname,
      PKG.href,
    ],
    stdout: "piped",
    stderr: "inherit",
  }).output();
  assertEquals(out.code, 0, "depth_edge.ts exited");
  return new TextDecoder().decode(out.stdout).trim().split("\n").map((l) => JSON.parse(l));
}

for (const [tier, flags] of [["default tier", []], ["optimizing tier (--no-liftoff)", ["--no-liftoff"]]] as const) {
  Deno.test(`wasm - the depth limit holds at its edge under the ${tier}`, async () => {
    const edges = await depthEdges([...flags]);
    // 30 operator-loop arms in two contexts each, and 11 other shapes.
    assertEquals(edges.length, 30 * 2 + 11, "every shape reported");
    for (const edge of edges) {
      assertEquals(edge.error, undefined, `${edge.name}: ${edge.error}`);
      // Each shape answers to the limit, less at most one repetition's plan weight: the
      // limit is a count, so the same under both tiers, and a shape short of it is
      // refused by something that is not the limit. Past it, `54001` and nothing else.
      assertEquals((edge.levels ?? 0) >= 1990, true, `${edge.name} answered ${edge.levels} levels`);
      assertEquals(edge.refusals, ["54001"], edge.name);
    }
  });
}

Deno.test("wasm - a host exception that unwinds a call leaves every later call refused, not trapping", async () => {
  // V8's stack shrunk until a statement well inside the depth limit exhausts it: the
  // `RangeError` the limit keeps every statement from reaching on a default stack.
  // Unwound, the call left the shadow-stack pointer short of its frames, and the
  // module's memory as the call left it; new instances went on answering over it, and
  // a few more such errors walked the stack below zero and trapped
  // `memory access out of bounds` on a one-line `CREATE TABLE`.
  const out = await new Deno.Command(Deno.execPath(), {
    args: [
      "run",
      "-A",
      "--v8-flags=--stack-size=200",
      new URL("module_poison.ts", import.meta.url).pathname,
      PKG.href,
    ],
    stdout: "piped",
    stderr: "inherit",
  }).output();
  assertEquals(out.code, 0, "module_poison.ts exited");
  const r = JSON.parse(new TextDecoder().decode(out.stdout));
  assertEquals(r.statement, "RangeError");
  // Every later call is refused by name, on every instance: a new one, and one created
  // before it. The one that ran the statement is still borrowed by the unwound call, so
  // wasm-bindgen refuses it first (`recursive use of an object …`) — loud either way.
  assertEquals(r.after, Array(10).fill("PgletModulePoisoned"));
  assertEquals(r.before, "PgletModulePoisoned");
  assertEquals(r.ran, "Error");
});

Deno.test("wasm - the caller's own JS throwing, or calling back in, is an error and not poison", async () => {
  const out = await new Deno.Command(Deno.execPath(), {
    args: ["run", "-A", new URL("host_exceptions.ts", import.meta.url).pathname, PKG.href],
    stdout: "piped",
    stderr: "inherit",
  }).output();
  assertEquals(out.code, 0, "host_exceptions.ts exited");
  const r = JSON.parse(new TextDecoder().decode(out.stdout));
  // A parameter's own exception is the call's error, unchanged, and the module answers
  // on — the instance and a new one. Before, the index getter's unwound the call and
  // every later call on every instance was `PgletModulePoisoned`.
  for (const [name, message] of [
    ["index getter", "Error index getter threw"],
    ["length getter", "Error length getter threw"],
    ["view getter", "Error byteLength getter threw"],
  ]) {
    assertEquals(r[name], { call: message, same: "ok", fresh: "ok" }, name);
  }
  assertEquals(r["revoked proxy"].call.startsWith("TypeError"), true, r["revoked proxy"].call);
  assertEquals(r["toJSON"].call, "cannot JSON-serialize parameter");
  for (const name of ["revoked proxy", "toJSON"]) {
    assertEquals([r[name].same, r[name].fresh], ["ok", "ok"], name);
  }
  // A call from inside a running one is refused by its own name, the running call
  // answers, and nothing is poisoned. A `Date.now` that throws fails the statement with a
  // SQLSTATE, like any engine error.
  const re = r["reentered"];
  assertEquals(re.call, "ok");
  assertEquals(re.inner.startsWith("PgletReentrantCall"), true, re.inner);
  assertEquals(re.threw.startsWith("Error 22000"), true, re.threw);
  assertEquals([re.same, re.other, re.fresh], ["ok", "ok", "ok"]);
  // Building the result reaches the page's prototypes too — but not through a field
  // descriptor: it is an object literal, its keys defined, so no `Object.prototype` member
  // named for a key is reached (a throwing setter, a read-only value, a setter that stores
  // nothing, adds a key or stores its own value), and each field owns its seven keys holding
  // pglet's values.
  const field_own = "name=a:truetruetrue tableID=0:truetruetrue columnID=0:truetruetrue " +
    "dataTypeID=23:truetruetrue dataTypeSize=4:truetruetrue dataTypeModifier=-1:truetruetrue " +
    "format=text:truetruetrue";
  for (const hook of ["throwing setter", "read-only", "swallowed", "adds another", "own value"]) {
    assertEquals(r[`field key ${hook}`], { call: "ok", own: field_own }, hook);
  }
  // A result's descriptor lives only as long as the result: a page's `Object` replaced for one
  // statement sees nothing of a later one once restored.
  assertEquals(r["transient Object"].later, "ok");
  assertEquals(r["transient Object"].seen_after_restore, []);
  // A column name or a JSONB key is defined, not set, so a setter of that name is not
  // reached at all: the statement answers and each is an own key.
  assertEquals(r["column setter"].call, "ok");
  // An array element is an own element whatever `Array.prototype` holds — defined when a page
  // put an index on the chain — so an index setter or a read-only inherited index is not
  // reached: every element is there.
  const all_elements = JSON.stringify([[{ a: 1, j: [7, 8], arr: [4, 5] }], 3]);
  assertEquals(r["index setter"], { call: "ok", same: "ok", fresh: "ok" });
  assertEquals(r["index setter elements"], all_elements);
  assertEquals(r["read-only index"].call, "ok");
  assertEquals(r["read-only index elements"], all_elements);
  // A `bytea` value's bytes are copied through a caught `Uint8Array.prototype.set`: a replaced
  // one fails the statement and the module answers on.
  assertEquals(r["bytea set"], { call: "Error bytea set threw", same: "ok", fresh: "ok" });
  // Page code the build calls can put a read-only index on `Array.prototype` after the once-per-result
  // check; the element store's `false` is then the statement's error, never a dropped row.
  assertEquals(r["late read-only index"], { call: "pglet: could not store a result element", rows: "none" });
  // A non-throwing index setter put there the same way answers a set `true` and stores nothing;
  // the store's own-`length` check makes that the statement's error too, through either door.
  for (const door of ["set", "decode"]) {
    assertEquals(
      r[`late swallowing index (${door})`],
      { call: "pglet: could not store a result element", rows: "none", same: "ok", fresh: "ok" },
      door,
    );
  }
  // A throwing one is met by the element store's caught `Reflect.set`: the statement's error,
  // and the module answers on.
  assertEquals(r["late throwing index (decode)"], {
    call: "Error late index setter threw",
    rows: "none",
    same: "ok",
    fresh: "ok",
  });
  // A bound `bytea` parameter's copy is caught: a replaced `Uint8Array.prototype.set` fails that
  // statement, and the module answers on.
  assertEquals(r["param bytea set"], { call: "Error param set threw", same: "ok", fresh: "ok" });
  // The half it does not own: wasm-bindgen's glue grows its own object heap with
  // `heap.push`, so a replaced `Array.prototype.push` throws from inside the glue and
  // unwinds the call — the module notices and refuses every call after it, which is the
  // loud answer, not a wrong one. pglet's own row and field stores no longer call `push`.
  const gp = r["glue push"];
  assertEquals(gp.call, "Error push threw", gp.call);
  assertEquals(gp.fresh.startsWith("PgletModulePoisoned"), true, gp.fresh);
});

Deno.test("wasm - a statement past the depth limit is 54001 and every instance lives", () => {
  // Before the limit a `+` chain of 2 842+ terms trapped `memory access out of bounds`
  // and poisoned the whole module (every `PgletBtree`, old and new), and from ~4 805 a
  // host-stack `RangeError` poisoned the object that ran it. Each length below sat in
  // one of those bands, or past both.
  const outcome = (db: { query(sql: string): unknown }, sql: string): string => {
    try {
      db.query(sql);
    } catch (e) {
      const err = e as { code?: string; message?: string };
      return `${err.code} ${err.message}`;
    }
    return "accepted";
  };
  const before = new mod.PgletBtree();
  before.exec(`CREATE TABLE t (a INT)`);
  before.exec(`INSERT INTO t VALUES (7)`);
  const db = new mod.PgletBtree();
  for (const terms of [3000, 5000, 100000]) {
    assertEquals(outcome(db, "SELECT 1" + "+1".repeat(terms - 1)), "54001 stack depth limit exceeded");
    // The instance that ran it, one created before it, and one created after, all answer.
    assertEquals(num(db.query(`SELECT 1 AS x`).rows[0].x), 1);
    assertEquals(num(before.query(`SELECT a FROM t`).rows[0].a), 7);
    assertEquals(num(new mod.PgletBtree().query(`SELECT 2 AS x`).rows[0].x), 2);
  }
  // At the edge of the limit the dearest shapes still answer: the shadow stack
  // (`crates/pglet_wasm/build.rs`) must not be what binds — at rustc's 1 MiB a `JOIN`
  // chain trapped from 1 227 relations — and the host stack must hold the limit.
  const join = (n: number) =>
    "SELECT count(*)::int4 AS c FROM (VALUES (1)) t0(a)" +
    Array.from({ length: n - 1 }, (_, i) => ` JOIN (VALUES (1)) t${i + 1}(a) ON true`).join("");
  // The innermost `VALUES` row's item is an alias node (`t0(a)` names it), which no
  // walker takes in its lean frame and wasm charges `DISPATCH_LEVEL_WEIGHT` (three
  // levels), so the chain's edge sits two relations below the count.
  assertEquals(num(db.query(join(1994)).rows[0].c), 1);
  assertEquals(outcome(db, join(1995)), "54001 stack depth limit exceeded");
  const checked = new mod.PgletBtree();
  checked.exec(`CREATE TABLE c (a INT CHECK (a${"+1".repeat(1997)} > 0))`);
  checked.exec(`INSERT INTO c VALUES (1)`);
  // `AND`/`OR` lists are not deep: 18.6 answers these at 100 000 terms, and so does this.
  const v = `SELECT count(*)::int4 AS c FROM (VALUES (1), (2)) v(a)`;
  assertEquals(num(db.query(`${v} WHERE a = 1` + " AND a = 1".repeat(99_999)).rows[0].c), 1);
  assertEquals(num(db.query(`${v} WHERE a = 0` + " OR a = 2".repeat(99_999)).rows[0].c), 1);
});

Deno.test("wasm - a deeply parenthesized IN / ANY body is the nesting refusal and the instance lives", () => {
  // Whether an `IN (…)` or `ANY (…)` body is a sub-select is decided by a look-ahead over its
  // run of leading parentheses, ahead of the parser's own nesting bound. A look-ahead recursing
  // a host frame a parenthesis would exhaust V8's stack (from about 11 000 of them under Node
  // 24) and the `RangeError` would poison the instance; the bound must hold on wasm, which has
  // no byte check.
  const outcome = (db: { query(sql: string): unknown }, sql: string): string => {
    try {
      db.query(sql);
    } catch (e) {
      const err = e as { code?: string; message?: string };
      return `${err.code} ${err.message}`;
    }
    return "accepted";
  };
  const db = new mod.PgletBtree();
  for (const n of [20_000, 100_000]) {
    const [open, close] = ["(".repeat(n), ")".repeat(n)];
    for (
      const sql of [
        `SELECT 1 IN (${open}SELECT 1${close})`,
        `SELECT 1 IN (${open}1${close})`,
        `SELECT 1 = ANY (${open}SELECT 1${close})`,
      ]
    ) {
      const answer = outcome(db, sql);
      assertEquals(answer.startsWith("42601 ") && answer.includes("nesting too deep"), true, answer);
      assertEquals(num(db.query(`SELECT 42 AS x`).rows[0].x), 42);
    }
  }
});

Deno.test("wasm - a row longer than a page is stored out of line and reads back whole", () => {
  // The in-memory B+tree stores a value too long to keep inline in an overflow chain,
  // and refuses a page that would not fit rather than holding it — the same page
  // store the OPFS build writes through. Rows of 5 000 characters and of a mebibyte,
  // bound and inlined, replaced long-to-long and deleted, beside an indexed column
  // whose values sit inside 18.6's index-row limit.
  const db = new mod.PgletBtree();
  db.exec(`CREATE TABLE big (id INT, body TEXT, tag TEXT)`);
  db.exec(`CREATE INDEX big_tag ON big (tag)`);
  let x = 0x2545f491;
  const letters = (len: number): string => {
    let out = "";
    for (let i = 0; i < len; i++) {
      x ^= x << 13;
      x ^= x >>> 17;
      x ^= x << 5;
      out += String.fromCharCode(97 + ((x >>> 0) % 26));
    }
    return out;
  };
  const five = letters(5_000);
  const mib = letters(1 << 20);
  const tag = letters(2_000);
  db.queryParams(`INSERT INTO big VALUES ($1, $2, $3)`, [1, five, tag]);
  db.exec(`INSERT INTO big VALUES (2, '${mib}', 'b')`);
  assertEquals(db.queryParams(`SELECT body FROM big WHERE id = $1`, [1]).rows[0].body, five);
  assertEquals(db.query(`SELECT body FROM big WHERE id = 2`).rows[0].body, mib);
  assertEquals(num(db.queryParams(`SELECT id FROM big WHERE tag = $1`, [tag]).rows[0].id), 1);
  const other = letters(300_000);
  db.queryParams(`UPDATE big SET body = $1 WHERE id = 2`, [other]);
  assertEquals(db.query(`SELECT body FROM big WHERE id = 2`).rows[0].body, other);
  db.exec(`DELETE FROM big WHERE id = 1`);
  assertEquals(num(db.query(`SELECT count(*)::int8 AS c FROM big`).rows[0].c), 1);
});
