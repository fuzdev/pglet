/**
 * postgres.js driver tests for pglet compatibility.
 *
 * Self-contained: Automatically spawns pglet server if not running.
 *
 * Tests basic driver functionality with postgres.js (modern alternative to node-postgres).
 * Run: deno task test:postgres
 *
 * NOTE: This is a simple test suite. Advanced features like binary vs text format
 * comparison, detailed type mapping, and connection pooling are not yet covered here.
 */

import { assertEquals } from "@std/assert";
import postgres from "postgres";
import { PORTS } from "../../lib/ports.ts";
import { ensureServer } from "../../lib/server.ts";

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

/**
 * Helper to create a fresh connection with postgres.js.
 */
async function withSql<T>(
  fn: (sql: ReturnType<typeof postgres>) => Promise<T>,
): Promise<T> {
  const sql = postgres(CONFIG);
  try {
    return await fn(sql);
  } finally {
    await sql.end();
  }
}

// =============================================================================
// Basic Connection Tests
// =============================================================================

Deno.test("postgres.js - basic connection and query", async () => {
  await withSql(async (sql) => {
    const result = await sql`SELECT 1 as num`;
    assertEquals(result.length, 1);
    assertEquals(result[0].num, 1);
  });
});

// =============================================================================
// CREATE TABLE Tests
// =============================================================================

Deno.test("postgres.js - create and drop table", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS test_table`;
    await sql`CREATE TABLE test_table (id INT, name TEXT)`;
    await sql`DROP TABLE test_table`;
  });
});

// =============================================================================
// INSERT Tests
// =============================================================================

Deno.test("postgres.js - insert and select", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS users`;
    await sql`CREATE TABLE users (id INT, name TEXT)`;

    await sql`INSERT INTO users (id, name) VALUES (1, 'Alice')`;

    const result = await sql`SELECT * FROM users`;
    assertEquals(result.length, 1);
    assertEquals(result[0].id, 1);
    assertEquals(result[0].name, "Alice");

    await sql`DROP TABLE users`;
  });
});

Deno.test("postgres.js - insert multiple rows", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS users`;
    await sql`CREATE TABLE users (id INT, name TEXT)`;

    await sql`INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')`;

    const result = await sql`SELECT * FROM users ORDER BY id`;
    assertEquals(result.length, 3);
    assertEquals(result[0].name, "Alice");
    assertEquals(result[1].name, "Bob");
    assertEquals(result[2].name, "Carol");

    await sql`DROP TABLE users`;
  });
});

// =============================================================================
// Parameterized Query Tests
// =============================================================================

Deno.test("postgres.js - parameterized queries", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS users`;
    await sql`CREATE TABLE users (id INT, name TEXT)`;

    const name = "Alice";
    const id = 1;
    await sql`INSERT INTO users (id, name) VALUES (${id}, ${name})`;

    const result = await sql`SELECT * FROM users WHERE id = ${id}`;
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Alice");

    await sql`DROP TABLE users`;
  });
});

// =============================================================================
// UPDATE Tests
// =============================================================================

Deno.test("postgres.js - update rows", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS users`;
    await sql`CREATE TABLE users (id INT, name TEXT)`;

    await sql`INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob')`;
    await sql`UPDATE users SET name = 'Charlie' WHERE id = 2`;

    const result = await sql`SELECT * FROM users WHERE id = 2`;
    assertEquals(result[0].name, "Charlie");

    await sql`DROP TABLE users`;
  });
});

// =============================================================================
// DELETE Tests
// =============================================================================

Deno.test("postgres.js - delete rows", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS users`;
    await sql`CREATE TABLE users (id INT, name TEXT)`;

    await sql`INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')`;
    await sql`DELETE FROM users WHERE id = 2`;

    const result = await sql`SELECT * FROM users ORDER BY id`;
    assertEquals(result.length, 2);
    assertEquals(result[0].id, 1);
    assertEquals(result[1].id, 3);

    await sql`DROP TABLE users`;
  });
});

// =============================================================================
// Type Tests
// =============================================================================

Deno.test("postgres.js - boolean type", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS test_bool`;
    await sql`CREATE TABLE test_bool (id INT, active BOOL)`;

    await sql`INSERT INTO test_bool VALUES (1, true), (2, false)`;

    const result = await sql`SELECT * FROM test_bool ORDER BY id`;
    assertEquals(result[0].active, true);
    assertEquals(result[1].active, false);

    await sql`DROP TABLE test_bool`;
  });
});

Deno.test("postgres.js - numeric types", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS test_numbers`;
    await sql`CREATE TABLE test_numbers (id INT, price FLOAT)`;

    await sql`INSERT INTO test_numbers VALUES (1, 99.99), (2, 42.5)`;

    const result = await sql`SELECT * FROM test_numbers WHERE id = 1`;
    assertEquals(result[0].price, 99.99);

    await sql`DROP TABLE test_numbers`;
  });
});

// =============================================================================
// WHERE Clause Tests
// =============================================================================

Deno.test("postgres.js - where clause with comparison", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS products`;
    await sql`CREATE TABLE products (id INT, price FLOAT)`;

    await sql`INSERT INTO products VALUES (1, 10.0), (2, 20.0), (3, 30.0)`;

    const result = await sql`SELECT * FROM products WHERE price > 15.0 ORDER BY id`;
    assertEquals(result.length, 2);
    assertEquals(result[0].id, 2);
    assertEquals(result[1].id, 3);

    await sql`DROP TABLE products`;
  });
});

// =============================================================================
// Transaction Tests
// =============================================================================

Deno.test("postgres.js - basic transaction commit", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS accounts`;
    await sql`CREATE TABLE accounts (id INT, balance FLOAT)`;

    await sql.begin(async (sql) => {
      await sql`INSERT INTO accounts VALUES (1, 100.0)`;
      await sql`INSERT INTO accounts VALUES (2, 200.0)`;
    });

    const result = await sql`SELECT * FROM accounts ORDER BY id`;
    assertEquals(result.length, 2);
    assertEquals(result[0].balance, 100.0);

    await sql`DROP TABLE accounts`;
  });
});

Deno.test("postgres.js - transaction rollback", async () => {
  await withSql(async (sql) => {
    await sql`DROP TABLE IF EXISTS accounts`;
    await sql`CREATE TABLE accounts (id INT, balance FLOAT)`;

    await sql`INSERT INTO accounts VALUES (1, 100.0)`;

    try {
      await sql.begin(async (sql) => {
        await sql`UPDATE accounts SET balance = 50.0 WHERE id = 1`;
        throw new Error("Rollback test");
      });
    } catch (e) {
      // Expected error
    }

    const result = await sql`SELECT * FROM accounts WHERE id = 1`;
    assertEquals(result[0].balance, 100.0); // Should be unchanged

    await sql`DROP TABLE accounts`;
  });
});
