/**
 * pg (node-postgres) driver tests for pglet compatibility.
 *
 * Self-contained: starts a pglet server of its own for the file.
 *
 * Tests basic driver functionality with pg (node-postgres).
 * Complements integration tests by focusing on driver-specific features.
 *
 * Run: npm run test:pg
 *
 * NOTE: This is a simple test suite. Advanced features like binary vs text format
 * comparison, detailed type mapping, Pool vs Client behavior, and result metadata
 * are not yet covered here.
 */

import { test } from "vitest";
import { assert_equal } from "../../lib/assert.ts";
import pg from "pg";
import { start_suite_server } from "../../lib/server.ts";

const { Client, Pool } = pg;

// A server of this file's own, on a port the kernel picks, stopped when the file's tests end
const server = await start_suite_server();

const CONFIG = {
  host: server.host,
  port: server.port,
  user: "postgres",
  database: "postgres",
};

/**
 * Helper to create a fresh connection with pg.Client.
 */
async function withClient<T>(
  fn: (client: pg.Client) => Promise<T>,
): Promise<T> {
  const client = new Client(CONFIG);
  await client.connect();
  try {
    return await fn(client);
  } finally {
    await client.end();
  }
}

/**
 * Helper to create a fresh connection pool with pg.Pool.
 */
async function withPool<T>(
  fn: (pool: pg.Pool) => Promise<T>,
): Promise<T> {
  const pool = new Pool(CONFIG);
  try {
    return await fn(pool);
  } finally {
    await pool.end();
  }
}

// =============================================================================
// Basic Connection Tests
// =============================================================================

test("pg - basic connection and query", async () => {
  await withClient(async (client) => {
    const result = await client.query("SELECT 1 as num");
    assert_equal(result.rows.length, 1);
    assert_equal(result.rows[0].num, 1);
  });
});

// =============================================================================
// CREATE TABLE Tests
// =============================================================================

test("pg - create and drop table", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS test_table");
    await client.query("CREATE TABLE test_table (id INT, name TEXT)");
    await client.query("DROP TABLE test_table");
  });
});

// =============================================================================
// INSERT Tests
// =============================================================================

test("pg - insert and select", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS users");
    await client.query("CREATE TABLE users (id INT, name TEXT)");

    await client.query("INSERT INTO users (id, name) VALUES (1, 'Alice')");

    const result = await client.query("SELECT * FROM users");
    assert_equal(result.rows.length, 1);
    assert_equal(result.rows[0].id, 1);
    assert_equal(result.rows[0].name, "Alice");

    await client.query("DROP TABLE users");
  });
});

test("pg - insert multiple rows", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS users");
    await client.query("CREATE TABLE users (id INT, name TEXT)");

    await client.query(
      "INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')",
    );

    const result = await client.query("SELECT * FROM users ORDER BY id");
    assert_equal(result.rows.length, 3);
    assert_equal(result.rows[0].name, "Alice");
    assert_equal(result.rows[1].name, "Bob");
    assert_equal(result.rows[2].name, "Carol");

    await client.query("DROP TABLE users");
  });
});

// =============================================================================
// Parameterized Query Tests
// =============================================================================

test("pg - parameterized queries with $1, $2", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS users");
    await client.query("CREATE TABLE users (id INT, name TEXT)");

    await client.query("INSERT INTO users (id, name) VALUES ($1, $2)", [1, "Alice"]);

    const result = await client.query("SELECT * FROM users WHERE id = $1", [1]);
    assert_equal(result.rows.length, 1);
    assert_equal(result.rows[0].name, "Alice");

    await client.query("DROP TABLE users");
  });
});

test("pg - multiple parameters", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS products");
    await client.query("CREATE TABLE products (id INT, name TEXT, price FLOAT)");

    await client.query(
      "INSERT INTO products (id, name, price) VALUES ($1, $2, $3)",
      [1, "Widget", 9.99],
    );

    const result = await client.query(
      "SELECT * FROM products WHERE name = $1 AND price < $2",
      ["Widget", 20.0],
    );
    assert_equal(result.rows.length, 1);
    assert_equal(result.rows[0].id, 1);

    await client.query("DROP TABLE products");
  });
});

// =============================================================================
// UPDATE Tests
// =============================================================================

test("pg - update rows", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS users");
    await client.query("CREATE TABLE users (id INT, name TEXT)");

    await client.query("INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob')");
    await client.query("UPDATE users SET name = 'Charlie' WHERE id = 2");

    const result = await client.query("SELECT * FROM users WHERE id = 2");
    assert_equal(result.rows[0].name, "Charlie");

    await client.query("DROP TABLE users");
  });
});

// =============================================================================
// DELETE Tests
// =============================================================================

test("pg - delete rows", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS users");
    await client.query("CREATE TABLE users (id INT, name TEXT)");

    await client.query(
      "INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')",
    );
    await client.query("DELETE FROM users WHERE id = 2");

    const result = await client.query("SELECT * FROM users ORDER BY id");
    assert_equal(result.rows.length, 2);
    assert_equal(result.rows[0].id, 1);
    assert_equal(result.rows[1].id, 3);

    await client.query("DROP TABLE users");
  });
});

// =============================================================================
// Type Tests
// =============================================================================

test("pg - boolean type", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS test_bool");
    await client.query("CREATE TABLE test_bool (id INT, active BOOL)");

    await client.query("INSERT INTO test_bool VALUES (1, true), (2, false)");

    const result = await client.query("SELECT * FROM test_bool ORDER BY id");
    assert_equal(result.rows[0].active, true);
    assert_equal(result.rows[1].active, false);

    await client.query("DROP TABLE test_bool");
  });
});

test("pg - numeric types", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS test_numbers");
    await client.query("CREATE TABLE test_numbers (id INT, price FLOAT)");

    await client.query("INSERT INTO test_numbers VALUES (1, 99.99), (2, 42.5)");

    const result = await client.query("SELECT * FROM test_numbers WHERE id = 1");
    assert_equal(result.rows[0].price, 99.99);

    await client.query("DROP TABLE test_numbers");
  });
});

// =============================================================================
// WHERE Clause Tests
// =============================================================================

test("pg - where clause with comparison", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS products");
    await client.query("CREATE TABLE products (id INT, price FLOAT)");

    await client.query("INSERT INTO products VALUES (1, 10.0), (2, 20.0), (3, 30.0)");

    const result = await client.query("SELECT * FROM products WHERE price > 15.0 ORDER BY id");
    assert_equal(result.rows.length, 2);
    assert_equal(result.rows[0].id, 2);
    assert_equal(result.rows[1].id, 3);

    await client.query("DROP TABLE products");
  });
});

// =============================================================================
// Transaction Tests
// =============================================================================

test("pg - basic transaction commit", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS accounts");
    await client.query("CREATE TABLE accounts (id INT, balance FLOAT)");

    await client.query("BEGIN");
    await client.query("INSERT INTO accounts VALUES (1, 100.0)");
    await client.query("INSERT INTO accounts VALUES (2, 200.0)");
    await client.query("COMMIT");

    const result = await client.query("SELECT * FROM accounts ORDER BY id");
    assert_equal(result.rows.length, 2);
    assert_equal(result.rows[0].balance, 100.0);

    await client.query("DROP TABLE accounts");
  });
});

test("pg - transaction rollback", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS accounts");
    await client.query("CREATE TABLE accounts (id INT, balance FLOAT)");

    await client.query("INSERT INTO accounts VALUES (1, 100.0)");

    await client.query("BEGIN");
    await client.query("UPDATE accounts SET balance = 50.0 WHERE id = 1");
    await client.query("ROLLBACK");

    const result = await client.query("SELECT * FROM accounts WHERE id = 1");
    assert_equal(result.rows[0].balance, 100.0); // Should be unchanged

    await client.query("DROP TABLE accounts");
  });
});

// =============================================================================
// Pool Tests
// =============================================================================

test("pg - pool basic query", async () => {
  await withPool(async (pool) => {
    const result = await pool.query("SELECT 1 as num");
    assert_equal(result.rows.length, 1);
    assert_equal(result.rows[0].num, 1);
  });
});

// TODO: a `pg.Pool` over several connections at once — reuse, concurrent queries, and the
// writer's `55P03` wait between pooled clients (the Rust `test:concurrent` suite covers the
// engine side; nothing drives it through `pg`).

// =============================================================================
// Result Metadata Tests
// =============================================================================

test("pg - result metadata", async () => {
  await withClient(async (client) => {
    await client.query("DROP TABLE IF EXISTS users");
    await client.query("CREATE TABLE users (id INT, name TEXT)");
    await client.query("INSERT INTO users VALUES (1, 'Alice')");

    const result = await client.query("SELECT * FROM users");

    // Check rows
    assert_equal(result.rows.length, 1);

    // Check fields metadata
    assert_equal(result.fields.length, 2);
    assert_equal(result.fields[0].name, "id");
    assert_equal(result.fields[1].name, "name");
    // TODO: `dataTypeID`, `tableID` / `columnID` and `dataTypeModifier` as `pg` reads them off
    // the native server (the fixture harness compares declared type OIDs through its own Rust
    // client; the npm adapter's equivalents are pinned in `tests-web/npm/dx_test.ts`).

    await client.query("DROP TABLE users");
  });
});

/**
 * `pg`'s `options` connection setting, the startup packet's `options` string: a `search_path`
 * asked for there gets its `set_config`'s verdict. PostgreSQL 18.6 serves `-c
 * search_path=nosuch` (the schema may be created later); pglet has one schema, so it refuses
 * the connection (`FATAL 0A000`) rather than serve `public` to a client that asked for another,
 * and serves a path that names only `public`.
 */
test("pg - options: a startup search_path gets its set_config's verdict", async () => {
  const refused = new Client({ ...CONFIG, options: "-c search_path=nosuch" });
  let code: string | undefined;
  try {
    await refused.connect();
  } catch (e) {
    code = (e as { code?: string }).code;
  } finally {
    await refused.end().catch(() => {});
  }
  assert_equal(code, "0A000");

  const served = new Client({ ...CONFIG, options: "-c search_path=public" });
  await served.connect();
  try {
    const result = await served.query("SELECT current_schema() AS s");
    assert_equal(result.rows, [{ s: "public" }]);
  } finally {
    await served.end();
  }
});
