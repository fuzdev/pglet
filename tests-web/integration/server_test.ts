/**
 * Server lifecycle tests for pglet.
 *
 * These tests spawn their own server instance to test:
 * - Server startup and accepts connections
 * - Graceful shutdown
 * - Port binding behavior
 *
 * Uses shared utilities from tests-web/lib/ but spawns fresh servers
 * per-test to test lifecycle behavior, each on a port the kernel picks.
 *
 * Run: npm run test:integration:server
 *
 * Note: These tests are slower as they spawn/kill server processes.
 */

import pg from "pg";
import { test } from "vitest";
import { assert_equal } from "../lib/assert.ts";
import { start_test_server } from "../lib/server.ts";

const { Client } = pg;

/**
 * Check if a port has a server accepting connections
 */
async function isPortInUse(port: number): Promise<boolean> {
  const client = new Client({
    host: "127.0.0.1",
    port,
    user: "postgres",
    database: "postgres",
    connectionTimeoutMillis: 500,
  });
  try {
    await client.connect();
    await client.query("SELECT 1");
    await client.end();
    return true;
  } catch {
    return false;
  }
}

test("server - starts and accepts connection", async () => {
  const server = await start_test_server();

  try {
    const client = new Client({
      host: "127.0.0.1",
      port: server.port,
      user: "postgres",
      database: "postgres",
    });
    await client.connect();

    // Create table and query to verify server works
    await client.query("CREATE TABLE startup_test (id INT)");
    await client.query("INSERT INTO startup_test VALUES (42)");
    const result = await client.query("SELECT id FROM startup_test");
    assert_equal(result.rows[0].id, 42);

    await client.end();
  } finally {
    await server.stop();
  }
});

test("server - handles multiple sequential starts", async () => {
  // Start server, use it, stop it, start again
  for (let i = 0; i < 2; i++) {
    const server = await start_test_server();

    try {
      const client = new Client({
        host: "127.0.0.1",
        port: server.port,
        user: "postgres",
        database: "postgres",
      });
      await client.connect();

      // Create table and verify we can use it
      await client.query(`CREATE TABLE restart_test_${i} (val INT)`);
      await client.query(`INSERT INTO restart_test_${i} VALUES (${i})`);
      const result = await client.query(`SELECT val FROM restart_test_${i}`);
      assert_equal(result.rows[0].val, i);

      await client.end();
    } finally {
      await server.stop();
    }

    // Verify port is released
    await new Promise((r) => setTimeout(r, 100));
    assert_equal(await isPortInUse(server.port), false);
  }
});

test("server - graceful shutdown closes connections", async () => {
  const server = await start_test_server();

  const client = new Client({
    host: "127.0.0.1",
    port: server.port,
    user: "postgres",
    database: "postgres",
  });
  await client.connect();

  // Verify connection works
  await client.query("CREATE TABLE shutdown_test (id INT)");

  // Kill server
  await server.stop();

  // Wait a moment for shutdown
  await new Promise((r) => setTimeout(r, 100));

  // Port should be free
  assert_equal(await isPortInUse(server.port), false);
});

test("server - per-connection state isolation", async () => {
  const server = await start_test_server();

  try {
    // First connection creates a table
    const client1 = new Client({
      host: "127.0.0.1",
      port: server.port,
      user: "postgres",
      database: "postgres",
    });
    await client1.connect();
    await client1.query("CREATE TABLE server_test (id INT)");
    await client1.query("INSERT INTO server_test VALUES (42)");
    const r1 = await client1.query("SELECT * FROM server_test");
    assert_equal(r1.rows[0].id, 42);
    await client1.end();

    // Second connection should NOT see the table (no --data-dir = isolated state)
    const client2 = new Client({
      host: "127.0.0.1",
      port: server.port,
      user: "postgres",
      database: "postgres",
    });
    await client2.connect();

    let errorThrown = false;
    try {
      await client2.query("SELECT * FROM server_test");
    } catch {
      errorThrown = true;
    }
    assert_equal(errorThrown, true);

    await client2.end();
  } finally {
    await server.stop();
  }
});
