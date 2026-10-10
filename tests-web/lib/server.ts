/**
 * Server lifecycle management for pglet tests.
 *
 * Provides utilities to spawn, detect, and manage pglet server instances.
 * Tests use `ensureServer()` to spawn a fresh server on the test port.
 */

import pg from "pg";
import { PORTS, PGLET_CONFIG } from "./ports.ts";
import { buildServer } from "./build.ts";

const { Client } = pg;

/** Connection config type */
interface DbConfig {
  host: string;
  port: number;
  user: string;
  database: string;
}

/** Handle to a spawned server process */
export interface ServerHandle {
  port: number;
  process: Deno.ChildProcess;
  kill: () => Promise<void>;
}

/** Global registry of spawned servers (for cleanup) */
const spawnedServers: Map<number, ServerHandle> = new Map();

/**
 * Check if a Postgres-compatiblesque server is healthy on the given config.
 */
export async function checkHealth(
  config: DbConfig,
  timeoutMs = 1000,
): Promise<boolean> {
  const client = new Client({ ...config, connectionTimeoutMillis: timeoutMs });
  try {
    await client.connect();
    await client.query("SELECT 1");
    await client.end();
    return true;
  } catch {
    return false;
  }
}

/**
 * Check if pglet server is running on the test port.
 */
export async function isPgletRunning(): Promise<boolean> {
  return checkHealth(PGLET_CONFIG);
}

/**
 * Wait for a port to become available with a server accepting connections.
 */
export async function waitForServer(
  port: number,
  timeoutMs = 10000,
  intervalMs = 100,
): Promise<boolean> {
  const config = { host: "127.0.0.1", port, user: "postgres", database: "postgres" };
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    if (await checkHealth(config, 500)) {
      return true;
    }
    await new Promise((r) => setTimeout(r, intervalMs));
  }

  return false;
}

/**
 * Spawn a pglet server on the given port.
 * Returns a handle to control the server.
 */
export async function spawnServer(port: number): Promise<ServerHandle> {
  // Build first
  if (!await buildServer()) {
    throw new Error("Failed to build pglet_server");
  }

  // Spawn server with null streams to avoid resource leaks
  const cmd = new Deno.Command("target/debug/pglet_server", {
    args: ["--port", String(port)],
    stdout: "null",
    stderr: "null",
    env: { ...Deno.env.toObject(), RUST_LOG: "error" },
  });
  const process = cmd.spawn();

  // Wait for server to be ready
  if (!await waitForServer(port, 10000)) {
    process.kill("SIGKILL");
    throw new Error(`Server failed to start on port ${port} within 10s`);
  }

  const handle: ServerHandle = {
    port,
    process,
    kill: async () => {
      try {
        process.kill("SIGTERM");
        await process.status;
      } catch {
        // Already dead
      }
      spawnedServers.delete(port);
    },
  };

  spawnedServers.set(port, handle);

  // Monitor for unexpected exits
  process.status.then((status) => {
    if (spawnedServers.has(port)) {
      console.log(`[test] Server on port ${port} exited with code ${status.code}`);
      spawnedServers.delete(port);
    }
  });

  return handle;
}

/**
 * Spawn a fresh pglet server, built from the current source, on the test port (5434).
 *
 * Something already answering there is refused, never reused: the port is also the spy
 * proxy's default (which forwards to real Postgres) and the first of the Rust harness's
 * dynamic range, so reusing whatever answers could run the suite against Postgres or a
 * stale pglet binary — and a spawn over an occupied port would read the occupant's health
 * as its own.
 *
 * Returns a cleanup function to call after tests.
 */
export async function ensureServer(): Promise<() => Promise<void>> {
  if (await isPgletRunning()) {
    throw new Error(
      `port ${PORTS.PGLET_TEST} is already in use; stop whatever is listening there ` +
        `(a spy proxy, a leftover test server) and rerun`,
    );
  }

  // Spawn new server on test port (dev server on 5433 is for manual testing)
  console.log(`[test] Spawning pglet server on port ${PORTS.PGLET_TEST}...`);
  const handle = await spawnServer(PORTS.PGLET_TEST);
  console.log(`[test] Server ready on port ${PORTS.PGLET_TEST}`);

  return handle.kill;
}

/**
 * Kill all spawned servers.
 * Call this in test cleanup or on process exit.
 */
export async function killAllServers(): Promise<void> {
  for (const [port, handle] of spawnedServers) {
    console.log(`[test] Killing server on port ${port}`);
    await handle.kill();
  }
}

// Cleanup on process exit
globalThis.addEventListener("unload", () => {
  killAllServers();
});
