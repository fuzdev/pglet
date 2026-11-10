/**
 * Port allocation for pglet tests.
 *
 * Tests use dedicated ports to avoid conflicts with development servers; a server suite's
 * pglet takes a port the kernel picks (`tests-web/lib/server.ts`).
 */

export const PORTS = {
  /** Real Postgres (system service) for fixture generation */
  POSTGRES: 5432,

  /** pglet dev server (`deno task server`) - tests avoid this port */
  PGLET_DEV: 5433,

  /** HTTP server for browser/WASM tests */
  HTTP_TEST: 8001,
} as const;
