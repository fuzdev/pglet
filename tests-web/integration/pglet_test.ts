/**
 * Behavioral tests for pglet that can't be captured by SQL fixtures.
 *
 * Self-contained: Automatically spawns pglet server if not running.
 *
 * SQL fixtures (tests/fixtures/) cover query correctness.
 * These tests cover connection behavior, session state, and edge cases.
 *
 * Note: State is shared across connections. Tests use DROP TABLE IF EXISTS
 * to be idempotent when running in parallel. Each test uses unique table
 * names to avoid conflicts.
 */

import pg from "pg";
import { assertEquals, assertRejects } from "@std/assert";
import { PORTS } from "../lib/ports.ts";
import { ensureServer } from "../lib/server.ts";

const { Client } = pg;

const CONFIG = {
  host: "127.0.0.1",
  port: PORTS.PGLET_TEST,
  user: "postgres",
  database: "postgres",
};

// Ensure server is running before tests
const cleanupServer = await ensureServer();

// Register cleanup
globalThis.addEventListener("unload", () => {
  cleanupServer();
});

Deno.test("connection - basic connect and disconnect", async () => {
  const client = new Client(CONFIG);
  await client.connect();
  await client.end();
});

Deno.test("connection - multiple sequential connections", async () => {
  // Each connection should work independently
  for (let i = 0; i < 3; i++) {
    const client = new Client(CONFIG);
    await client.connect();
    const result = await client.query("SELECT 1");
    assertEquals(result.rows.length, 1);
    await client.end();
  }
});

Deno.test("session - state persists within connection", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  // Cleanup and create table
  await client.query("DROP TABLE IF EXISTS session_test");
  await client.query("CREATE TABLE session_test (id INT, val TEXT)");

  // Insert data
  await client.query("INSERT INTO session_test VALUES (1, 'first')");
  await client.query("INSERT INTO session_test VALUES (2, 'second')");

  // Query should see both rows
  const result = await client.query("SELECT * FROM session_test ORDER BY id");
  assertEquals(result.rows.length, 2);
  assertEquals(result.rows[0].val, "first");
  assertEquals(result.rows[1].val, "second");

  // Cleanup
  await client.query("DROP TABLE session_test");
  await client.end();
});

Deno.test("session - state shared between connections", async () => {
  // First connection creates a table
  const client1 = new Client(CONFIG);
  await client1.connect();
  await client1.query("DROP TABLE IF EXISTS shared_test");
  await client1.query("CREATE TABLE shared_test (id INT)");
  await client1.query("INSERT INTO shared_test VALUES (1)");
  await client1.end();

  // Second connection SHOULD see the table (shared state)
  const client2 = new Client(CONFIG);
  await client2.connect();

  const result = await client2.query("SELECT * FROM shared_test");
  assertEquals(result.rows.length, 1);
  assertEquals(result.rows[0].id, 1);

  // Cleanup
  await client2.query("DROP TABLE shared_test");
  await client2.end();
});

Deno.test("rowCount - INSERT returns correct count", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS count_test");
  await client.query("CREATE TABLE count_test (id INT)");

  const r1 = await client.query("INSERT INTO count_test VALUES (1)");
  assertEquals(r1.rowCount, 1);

  const r2 = await client.query("INSERT INTO count_test VALUES (2), (3), (4)");
  assertEquals(r2.rowCount, 3);

  // Cleanup
  await client.query("DROP TABLE count_test");
  await client.end();
});

Deno.test("rowCount - UPDATE returns correct count", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS update_count");
  await client.query("CREATE TABLE update_count (id INT, active BOOL)");
  await client.query(
    "INSERT INTO update_count VALUES (1, false), (2, false), (3, true)",
  );

  const r1 = await client.query(
    "UPDATE update_count SET active = true WHERE active = false",
  );
  assertEquals(r1.rowCount, 2);

  const r2 = await client.query("UPDATE update_count SET active = false");
  assertEquals(r2.rowCount, 3);

  // Cleanup
  await client.query("DROP TABLE update_count");
  await client.end();
});

Deno.test("rowCount - DELETE returns correct count", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS delete_count");
  await client.query("CREATE TABLE delete_count (id INT)");
  await client.query("INSERT INTO delete_count VALUES (1), (2), (3), (4), (5)");

  const r1 = await client.query("DELETE FROM delete_count WHERE id > 3");
  assertEquals(r1.rowCount, 2);

  const r2 = await client.query("DELETE FROM delete_count");
  assertEquals(r2.rowCount, 3);

  // Cleanup
  await client.query("DROP TABLE delete_count");
  await client.end();
});

Deno.test("concurrent - multiple simultaneous connections", async () => {
  // Create multiple clients and connect them all at once
  const clients = await Promise.all(
    Array.from({ length: 5 }, async () => {
      const client = new Client(CONFIG);
      await client.connect();
      return client;
    }),
  );

  // Cleanup any leftover tables from previous runs
  await Promise.all(
    clients.map(async (client, i) => {
      await client.query(`DROP TABLE IF EXISTS concurrent_${i}`);
    }),
  );

  // Each client creates its own table
  await Promise.all(
    clients.map(async (client, i) => {
      await client.query(`CREATE TABLE concurrent_${i} (id INT, val TEXT)`);
      await client.query(
        `INSERT INTO concurrent_${i} VALUES (${i}, 'client${i}')`,
      );
    }),
  );

  // Each client queries its own table
  const results = await Promise.all(
    clients.map(async (client, i) => {
      const result = await client.query(`SELECT * FROM concurrent_${i}`);
      return result.rows[0];
    }),
  );

  // Verify each client got its own data
  results.forEach((row, i) => {
    assertEquals(row.id, i);
    assertEquals(row.val, `client${i}`);
  });

  // Cleanup
  await Promise.all(
    clients.map(async (client, i) => {
      await client.query(`DROP TABLE concurrent_${i}`);
    }),
  );
  await Promise.all(clients.map((client) => client.end()));
});

Deno.test("concurrent - interleaved queries", async () => {
  // Two clients running queries that interleave
  const client1 = new Client(CONFIG);
  const client2 = new Client(CONFIG);
  await Promise.all([client1.connect(), client2.connect()]);

  // Cleanup and setup tables
  await client1.query("DROP TABLE IF EXISTS interleave1");
  await client2.query("DROP TABLE IF EXISTS interleave2");
  await client1.query("CREATE TABLE interleave1 (id INT)");
  await client2.query("CREATE TABLE interleave2 (id INT)");

  // Interleave inserts
  await client1.query("INSERT INTO interleave1 VALUES (1)");
  await client2.query("INSERT INTO interleave2 VALUES (2)");
  await client1.query("INSERT INTO interleave1 VALUES (3)");
  await client2.query("INSERT INTO interleave2 VALUES (4)");

  // Verify data
  const r1 = await client1.query("SELECT * FROM interleave1 ORDER BY id");
  const r2 = await client2.query("SELECT * FROM interleave2 ORDER BY id");

  assertEquals(r1.rows.length, 2);
  assertEquals(r1.rows[0].id, 1);
  assertEquals(r1.rows[1].id, 3);

  assertEquals(r2.rows.length, 2);
  assertEquals(r2.rows[0].id, 2);
  assertEquals(r2.rows[1].id, 4);

  // Cleanup
  await client1.query("DROP TABLE interleave1");
  await client2.query("DROP TABLE interleave2");
  await Promise.all([client1.end(), client2.end()]);
});

Deno.test("error - invalid SQL syntax", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await assertRejects(
    async () => {
      await client.query("SELEKT * FROM nowhere");
    },
    Error,
  );

  await client.end();
});

Deno.test("error - table not found", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await assertRejects(
    async () => {
      await client.query("SELECT * FROM nonexistent_table_xyz");
    },
    Error,
  );

  await client.end();
});

Deno.test("error - column not found", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS col_test");
  await client.query("CREATE TABLE col_test (id INT)");

  await assertRejects(
    async () => {
      await client.query("SELECT nonexistent_column FROM col_test");
    },
    Error,
  );

  // Cleanup
  await client.query("DROP TABLE col_test");
  await client.end();
});

Deno.test("edge - empty result set", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS empty_test");
  await client.query("CREATE TABLE empty_test (id INT)");

  const result = await client.query("SELECT * FROM empty_test");
  assertEquals(result.rows.length, 0);
  assertEquals(result.rowCount, 0);

  // Cleanup
  await client.query("DROP TABLE empty_test");
  await client.end();
});

Deno.test("edge - large result set", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS large_test");
  await client.query("CREATE TABLE large_test (id INT, val TEXT)");

  // Insert 200 rows (well inside what the server's 8 MB connection stack takes)
  const values = Array.from({ length: 200 }, (_, i) => `(${i}, 'row${i}')`);
  await client.query(`INSERT INTO large_test VALUES ${values.join(", ")}`);

  const result = await client.query("SELECT * FROM large_test ORDER BY id");
  assertEquals(result.rows.length, 200);
  assertEquals(result.rows[0].id, 0);
  assertEquals(result.rows[199].id, 199);

  // Cleanup
  await client.query("DROP TABLE large_test");
  await client.end();
});

Deno.test("edge - wide row with many columns", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS wide_test");

  // Create table with 20 columns
  const cols = Array.from({ length: 20 }, (_, i) => `col${i} INT`);
  await client.query(`CREATE TABLE wide_test (${cols.join(", ")})`);

  // Insert a row
  const vals = Array.from({ length: 20 }, (_, i) => i);
  await client.query(`INSERT INTO wide_test VALUES (${vals.join(", ")})`);

  const result = await client.query("SELECT * FROM wide_test");
  assertEquals(result.rows.length, 1);
  assertEquals(Object.keys(result.rows[0]).length, 20);
  assertEquals(result.rows[0].col0, 0);
  assertEquals(result.rows[0].col19, 19);

  // Cleanup
  await client.query("DROP TABLE wide_test");
  await client.end();
});

Deno.test("edge - null values", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS null_test");
  await client.query("CREATE TABLE null_test (id INT, val TEXT)");
  await client.query("INSERT INTO null_test VALUES (1, NULL), (NULL, 'text')");

  const result = await client.query("SELECT * FROM null_test ORDER BY id");
  // NULL sorts LAST in ASC (Postgres: NULLs are treated as larger than any value)
  assertEquals(result.rows[0].id, 1);
  assertEquals(result.rows[0].val, null);
  assertEquals(result.rows[1].id, null);
  assertEquals(result.rows[1].val, "text");

  // Cleanup
  await client.query("DROP TABLE null_test");
  await client.end();
});

Deno.test("simple query - multi-statement returns every result set", async () => {
  // A single simple-Query message with multiple statements must return EVERY
  // statement's result set, not just the last — node-postgres surfaces this as an
  // array of results. Before the fix the server dropped the earlier result sets
  // (silent data loss for a driver batching statements).
  const client = new Client(CONFIG);
  await client.connect();

  const results = await client.query("SELECT 'first' AS a; SELECT 'second' AS b");

  // Multiple statements => node-postgres returns an array of Result objects.
  if (!Array.isArray(results)) {
    throw new Error(
      `expected an array of results for a multi-statement query, got a single result: ${JSON.stringify(results)}`,
    );
  }
  assertEquals(results.length, 2);
  assertEquals(results[0].rows[0].a, "first");
  assertEquals(results[1].rows[0].b, "second");

  await client.end();
});

Deno.test("edge - special characters in text", async () => {
  const client = new Client(CONFIG);
  await client.connect();

  await client.query("DROP TABLE IF EXISTS special_test");
  await client.query("CREATE TABLE special_test (val TEXT)");

  // Test various special characters
  const testCases = [
    "hello'world", // single quote
    'hello"world', // double quote
    "hello\\world", // backslash
    "hello\nworld", // newline
    "hello\tworld", // tab
    "émojis: 🎉🚀", // unicode
    "", // empty string
  ];

  for (const val of testCases) {
    await client.query("DELETE FROM special_test");
    await client.query({
      text: "INSERT INTO special_test VALUES ($1)",
      values: [val],
    });
    const result = await client.query("SELECT * FROM special_test");
    assertEquals(result.rows[0].val, val);
  }

  // Cleanup
  await client.query("DROP TABLE special_test");
  await client.end();
});
