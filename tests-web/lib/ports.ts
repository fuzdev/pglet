/**
 * Port allocation for pglet tests.
 *
 * Tests use dedicated ports to avoid conflicts with development servers.
 */

export const PORTS = {
  /** Real Postgres (system service) for fixture generation */
  POSTGRES: 5432,

  /** pglet dev server (`deno task server`) - tests avoid this port */
  PGLET_DEV: 5433,

  /** pglet test server - auto-spawned by tests */
  PGLET_TEST: 5434,

  /** HTTP server for browser/WASM tests */
  HTTP_TEST: 8001,
} as const;

/** Default connection config for pglet (test port) */
export const PGLET_CONFIG = {
  host: "127.0.0.1",
  port: PORTS.PGLET_TEST,
  user: "postgres",
  database: "postgres",
} as const;
