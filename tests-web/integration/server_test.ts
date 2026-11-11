/**
 * Server lifecycle tests for pglet.
 *
 * These tests spawn their own server instance to test:
 * - Server startup and accepts connections
 * - What a stop does to open connections and the port
 * - What connections to one server share
 *
 * Uses shared utilities from tests-web/lib/ but spawns fresh servers
 * per-test to test lifecycle behavior, each on a port the kernel picks.
 *
 * Run: npm run test:integration:server
 *
 * Note: These tests are slower as they spawn/kill server processes.
 */

import pg from "pg";
import { assert, onTestFinished, test } from "vitest";
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

test("server - stopping it ends its open connections and frees its port", async () => {
  const server = await start_test_server();

  const client = new Client({
    host: "127.0.0.1",
    port: server.port,
    user: "postgres",
    database: "postgres",
  });
  // pg reports a connection it didn't end as an `error` event, which would be uncaught
  const errors: Array<Error> = [];
  client.on("error", (error) => errors.push(error));
  const ended = new Promise<"ended">((resolve) => client.once("end", () => resolve("ended")));
  await client.connect();
  await client.query("CREATE TABLE shutdown_test (id INT)");

  await server.stop();

  // the server has no shutdown of its own: the signal ends it, and the kernel closes its sockets
  let timer: ReturnType<typeof setTimeout> | undefined;
  const outcome = await Promise.race([
    ended,
    new Promise<"open">((resolve) => (timer = setTimeout(() => resolve("open"), 10_000))),
  ]).finally(() => clearTimeout(timer));
  assert_equal(outcome, "ended", "the client's connection ended");
  assert.include(
    errors.map((e) => e.message),
    "Connection terminated unexpectedly",
    "pg saw the connection end without its own end()",
  );

  assert_equal(await isPortInUse(server.port), false);
});

test("server - connections share one database, a block's rows visible to others once committed", async () => {
  const server = await start_test_server();
  const connect = async (): Promise<pg.Client> => {
    const client = new Client({
      host: "127.0.0.1",
      port: server.port,
      user: "postgres",
      database: "postgres",
    });
    await client.connect();
    onTestFinished(() => client.end());
    return client;
  };

  // A table one connection commits is there for every connection after it
  const client1 = await connect();
  await client1.query("CREATE TABLE server_test (id INT)");
  await client1.query("INSERT INTO server_test VALUES (42)");
  await client1.end();

  const client2 = await connect();
  const ids = async (): Promise<Array<number>> =>
    (await client2.query("SELECT id FROM server_test ORDER BY id")).rows.map((r) => r.id);
  assert_equal(await ids(), [42]);

  // Another connection's open block's rows are invisible until it commits (its DDL is not:
  // the catalog is shared and live, EDGES.md row 221)
  const client3 = await connect();
  await client3.query("BEGIN");
  await client3.query("INSERT INTO server_test VALUES (43)");
  assert_equal(await ids(), [42]);
  await client3.query("COMMIT");
  assert_equal(await ids(), [42, 43]);
});
