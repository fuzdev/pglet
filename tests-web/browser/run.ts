#!/usr/bin/env -S deno run --allow-all
/**
 * Browser test runner for pglet WASM.
 *
 * Uses Playwright to run tests in a real browser environment.
 * Tests verify WASM functionality — on the default build, the shipped shape — and OPFS
 * persistence, on the build with the crate's `opfs` feature on (`crates/pglet_wasm/pkg-opfs`).
 *
 * Usage:
 *   deno task test:browser          # Headless (CI)
 *   deno task test:browser:headed   # Visible browser (debugging)
 */

import { chromium, type Browser, type Page, type ConsoleMessage } from "playwright";

// Worker API response types
interface WorkerResponse {
  success?: boolean;
  error?: string;
  result?: {
    rows: Record<string, unknown>[];
    rowCount: number;
    fields: { name: string }[];
  };
}

// Extend Window type for our WASM exports
declare global {
  interface Window {
    __pglet_ready?: boolean;
    __pglet_error?: string;
    __pglet_btree_opfs_ready?: boolean;
    __pglet_btree_opfs_error?: string;
    PgletBtree: new () => {
      exec(sql: string): void;
      query(sql: string): { rowCount: number; rows: Record<string, unknown>[]; fields: { name: string }[] };
    };
    pgletBtreeWorker: {
      init(): Promise<WorkerResponse>;
      exec(sql: string): Promise<WorkerResponse>;
      query(sql: string): Promise<WorkerResponse>;
      sync(): Promise<WorkerResponse>;
      clear(): Promise<WorkerResponse>;
    };
  }
}
import { ensureWasm, WASM_DEFAULT, WASM_OPFS } from "../lib/build.ts";
import { PORTS } from "../lib/ports.ts";

const HTTP_PORT = PORTS.HTTP_TEST;

interface TestResult {
  name: string;
  passed: boolean;
  error?: string;
  duration: number;
}

// Parse args
const headed = Deno.args.includes("--headed");
const filterPattern = Deno.args.find((a) => !a.startsWith("-"));

// Global state
let browser: Browser | null = null;
let httpServer: Deno.HttpServer | null = null;

/**
 * Check if HTTP server is already running on the port.
 */
async function isHttpServerRunning(): Promise<boolean> {
  try {
    const resp = await fetch(`http://localhost:${HTTP_PORT}/crates/pglet_wasm/pkg/pglet_wasm.js`, {
      signal: AbortSignal.timeout(1000),
    });
    return resp.ok;
  } catch {
    return false;
  }
}

/**
 * Start HTTP server for serving WASM files and test page.
 * If a server is already running (e.g., from `deno task wasm:serve`), reuses it.
 */
async function startHttpServer(): Promise<void> {
  // Check if server already running (from wasm:serve)
  if (await isHttpServerRunning()) {
    console.log(`[browser] Reusing existing HTTP server on port ${HTTP_PORT}`);
    return;
  }

  console.log(`[browser] Starting HTTP server on port ${HTTP_PORT}...`);

  try {
    httpServer = Deno.serve(
      { port: HTTP_PORT, onListen: () => {} },
      async (req) => {
        const url = new URL(req.url);
        let path = url.pathname;

        // Default to test.html
        if (path === "/" || path === "/test.html") {
          path = "/tests-web/browser/test.html";
        }

        // Resolve file path
        const filePath = `.${path}`;

        try {
          const file = await Deno.readFile(filePath);
          const contentType = getContentType(filePath);
          return new Response(file, {
            headers: { "Content-Type": contentType },
          });
        } catch {
          return new Response("Not Found", { status: 404 });
        }
      },
    );

    // Wait for server to be ready
    await new Promise((r) => setTimeout(r, 100));
    console.log(`[browser] HTTP server ready on http://localhost:${HTTP_PORT}`);
  } catch (err) {
    if (err instanceof Deno.errors.AddrInUse) {
      // Another process grabbed the port between our check and bind
      // Try to reuse it
      if (await isHttpServerRunning()) {
        console.log(`[browser] Reusing existing HTTP server on port ${HTTP_PORT}`);
        return;
      }
    }
    throw err;
  }
}

function getContentType(path: string): string {
  if (path.endsWith(".html")) return "text/html";
  if (path.endsWith(".js")) return "application/javascript";
  if (path.endsWith(".wasm")) return "application/wasm";
  if (path.endsWith(".json")) return "application/json";
  return "application/octet-stream";
}

/**
 * Stop HTTP server.
 */
async function stopHttpServer(): Promise<void> {
  if (httpServer) {
    await httpServer.shutdown();
    httpServer = null;
  }
}

/**
 * Run a single test in the browser.
 */
async function runTest(
  page: Page,
  testName: string,
  testFn: (page: Page) => Promise<void>,
): Promise<TestResult> {
  const start = Date.now();
  try {
    await testFn(page);
    return {
      name: testName,
      passed: true,
      duration: Date.now() - start,
    };
  } catch (error) {
    return {
      name: testName,
      passed: false,
      error: String(error),
      duration: Date.now() - start,
    };
  }
}

// =============================================================================
// Tests
// =============================================================================

/**
 * Wait for WASM to be ready on the page.
 */
async function waitForWasm(page: Page, timeoutMs = 5000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const ready = await page.evaluate(() => {
      if (window.__pglet_error) throw new Error(window.__pglet_error);
      return window.__pglet_ready === true;
    });
    if (ready) return;
    await new Promise(r => setTimeout(r, 50));
  }
  throw new Error(`WASM did not load within ${timeoutMs}ms`);
}

/**
 * Wait for B+tree OPFS worker to be ready on the page.
 */
async function waitForBtreeOpfs(page: Page, timeoutMs = 10000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const result = await page.evaluate(() => {
      if (window.__pglet_btree_opfs_error) return { error: window.__pglet_btree_opfs_error };
      return { ready: window.__pglet_btree_opfs_ready === true };
    });
    if (result.error) throw new Error(`B+tree OPFS worker failed: ${result.error}`);
    if (result.ready) return;
    await new Promise(r => setTimeout(r, 50));
  }
  throw new Error(`B+tree OPFS worker did not become ready within ${timeoutMs}ms`);
}

async function testWasmLoads(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForWasm(page);
}

async function testCreateTable(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForWasm(page);

  const result = await page.evaluate(async () => {
    const db = new window.PgletBtree();
    db.exec("CREATE TABLE test_create (id INT, name TEXT)");
    db.exec("INSERT INTO test_create VALUES (1, 'Alice')");
    const res = db.query("SELECT * FROM test_create");
    // Convert BigInt to Number for serialization
    return {
      rowCount: Number(res.rowCount),
      id: Number(res.rows[0].id),
      name: String(res.rows[0].name)
    };
  });

  if (result.rowCount !== 1) {
    throw new Error(`Expected 1 row, got ${result.rowCount}`);
  }
  if (result.id !== 1 || result.name !== "Alice") {
    throw new Error(`Unexpected row data: id=${result.id}, name=${result.name}`);
  }
}

async function testQueryWithWhere(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForWasm(page);

  const result = await page.evaluate(async () => {
    const db = new window.PgletBtree();
    db.exec("CREATE TABLE test_where (id INT, val TEXT)");
    db.exec("INSERT INTO test_where VALUES (1, 'one'), (2, 'two'), (3, 'three')");
    const res = db.query("SELECT * FROM test_where WHERE id > 1 ORDER BY id");
    return { rowCount: Number(res.rowCount), rows: res.rows };
  });

  if (result.rowCount !== 2) {
    throw new Error(`Expected 2 rows, got ${result.rowCount}`);
  }
}

async function testAggregates(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForWasm(page);

  const result = await page.evaluate(async () => {
    const db = new window.PgletBtree();
    db.exec("CREATE TABLE test_agg (val INT)");
    db.exec("INSERT INTO test_agg VALUES (10), (20), (30)");
    const res = db.query("SELECT COUNT(*) as cnt, SUM(val) as total FROM test_agg");
    // Convert BigInt to Number for serialization
    return {
      cnt: Number(res.rows[0].cnt),
      total: Number(res.rows[0].total)
    };
  });

  if (result.cnt !== 3) {
    throw new Error(`Expected count 3, got ${result.cnt}`);
  }
  if (result.total !== 60) {
    throw new Error(`Expected sum 60, got ${result.total}`);
  }
}

// =============================================================================
// B+tree OPFS Tests
// =============================================================================

async function testBtreePersistence(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForBtreeOpfs(page);

  // Create data in first session (DROP IF EXISTS handles cleanup)
  const created = await page.evaluate(async () => {
    const dropRes = await window.pgletBtreeWorker.exec("DROP TABLE IF EXISTS btree_persist_test");
    if (dropRes.error) return { error: `DROP failed: ${dropRes.error}` };

    const createRes = await window.pgletBtreeWorker.exec("CREATE TABLE btree_persist_test (id INT, val TEXT)");
    if (createRes.error) return { error: `CREATE failed: ${createRes.error}` };

    const insertRes = await window.pgletBtreeWorker.exec("INSERT INTO btree_persist_test VALUES (42, 'btree-persisted')");
    if (insertRes.error) return { error: `INSERT failed: ${insertRes.error}` };

    // No explicit sync(): each autocommit write is durable on its own (the meta-slot
    // flip happens per statement, like the native server), so the row must survive a
    // reload without a manual flush.
    return { success: true };
  });

  if (created.error) {
    throw new Error(`Failed to create data: ${created.error}`);
  }

  // Reload page to simulate new session
  await page.reload();
  await waitForBtreeOpfs(page);

  // Verify data persisted
  const verified = await page.evaluate(async () => {
    const queryRes = await window.pgletBtreeWorker.query("SELECT * FROM btree_persist_test");
    if (queryRes.error) return { error: queryRes.error };
    return queryRes.result;
  });

  if ("error" in verified && verified.error) {
    throw new Error(`Failed to verify persistence: ${verified.error}`);
  }

  if (verified.rowCount !== 1) {
    throw new Error(`Expected 1 row after reload, got ${verified.rowCount}`);
  }

  const row = verified.rows[0];
  if (row.id !== 42 || row.val !== "btree-persisted") {
    throw new Error(`Unexpected data after reload: ${JSON.stringify(row)}`);
  }
}

// Reopen an OPFS database whose table carries a UNIQUE index, then prove the index
// is re-registered after reload: a duplicate-key insert must still be rejected, and
// a point lookup must find the row. Without `reattach_persisted_indexes` on the wasm
// open path, the in-memory index def map is empty after reload, so the duplicate
// would be wrongly accepted (silent index drift) and probes would error.
async function testBtreeIndexReopen(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForBtreeOpfs(page);

  const created = await page.evaluate(async () => {
    const drop = await window.pgletBtreeWorker.exec("DROP TABLE IF EXISTS btree_index_test");
    if (drop.error) return { error: `DROP failed: ${drop.error}` };
    const create = await window.pgletBtreeWorker.exec(
      "CREATE TABLE btree_index_test (id INT PRIMARY KEY, email TEXT UNIQUE)",
    );
    if (create.error) return { error: `CREATE failed: ${create.error}` };
    const insert = await window.pgletBtreeWorker.exec(
      "INSERT INTO btree_index_test VALUES (1, 'a@x.io'), (2, 'b@x.io')",
    );
    if (insert.error) return { error: `INSERT failed: ${insert.error}` };
    return { success: true };
  });
  if (created.error) throw new Error(`Failed to seed indexed table: ${created.error}`);

  // Reload to force a fresh open() over the persisted OPFS file.
  await page.reload();
  await waitForBtreeOpfs(page);

  const result = await page.evaluate(async () => {
    // Duplicate UNIQUE value must be rejected — proves the index def was reattached
    // and is enforced after reopen (not silently accepted).
    const dup = await window.pgletBtreeWorker.exec("INSERT INTO btree_index_test VALUES (3, 'a@x.io')");
    const dupRejected = Boolean(dup.error);
    // A point lookup on the unique column must find the row (probe works post-reopen).
    const lookup = await window.pgletBtreeWorker.query(
      "SELECT id FROM btree_index_test WHERE email = 'b@x.io'",
    );
    if (lookup.error) return { error: `lookup failed: ${lookup.error}` };
    return { dupRejected, lookupCount: lookup.result?.rowCount ?? 0, lookupId: lookup.result?.rows?.[0]?.id };
  });

  if ("error" in result && result.error) throw new Error(result.error);
  if (!result.dupRejected) {
    throw new Error("duplicate UNIQUE value was accepted after reopen — index not reattached");
  }
  if (result.lookupCount !== 1 || result.lookupId !== 2) {
    throw new Error(`post-reopen unique lookup wrong: ${JSON.stringify(result)}`);
  }
}

async function testBtreeTransactionRollback(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForBtreeOpfs(page);

  // DROP IF EXISTS handles cleanup
  const result = await page.evaluate(async () => {
    const dropRes = await window.pgletBtreeWorker.exec("DROP TABLE IF EXISTS btree_rollback_test");
    if (dropRes.error) return { error: `DROP failed: ${dropRes.error}` };

    const createRes = await window.pgletBtreeWorker.exec("CREATE TABLE btree_rollback_test (id INT)");
    if (createRes.error) return { error: `CREATE failed: ${createRes.error}` };

    const syncRes = await window.pgletBtreeWorker.sync();
    if (syncRes.error) return { error: `SYNC failed: ${syncRes.error}` };

    // Start transaction, insert, then rollback
    const beginRes = await window.pgletBtreeWorker.exec("BEGIN");
    if (beginRes.error) return { error: `BEGIN failed: ${beginRes.error}` };

    const insertRes = await window.pgletBtreeWorker.exec("INSERT INTO btree_rollback_test VALUES (999)");
    if (insertRes.error) return { error: `INSERT failed: ${insertRes.error}` };

    const rollbackRes = await window.pgletBtreeWorker.exec("ROLLBACK");
    if (rollbackRes.error) return { error: `ROLLBACK failed: ${rollbackRes.error}` };

    const syncRes2 = await window.pgletBtreeWorker.sync();
    if (syncRes2.error) return { error: `SYNC2 failed: ${syncRes2.error}` };

    // Should have no rows
    const queryRes = await window.pgletBtreeWorker.query("SELECT * FROM btree_rollback_test");
    if (queryRes.error) return { error: queryRes.error };
    return { rowCount: queryRes.result?.rowCount ?? 0 };
  });

  if ("error" in result && result.error) {
    throw new Error(`Rollback test failed: ${result.error}`);
  }

  if (result.rowCount !== 0) {
    throw new Error(`Expected 0 rows after rollback, got ${result.rowCount}`);
  }
}

async function testBtreeMultipleRows(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForBtreeOpfs(page);

  // Test multiple rows with various operations
  const result = await page.evaluate(async () => {
    const dropRes = await window.pgletBtreeWorker.exec("DROP TABLE IF EXISTS btree_multi_test");
    if (dropRes.error) return { error: `DROP failed: ${dropRes.error}` };

    const createRes = await window.pgletBtreeWorker.exec("CREATE TABLE btree_multi_test (id INT, name TEXT)");
    if (createRes.error) return { error: `CREATE failed: ${createRes.error}` };

    // Insert multiple rows
    const insertRes = await window.pgletBtreeWorker.exec(
      "INSERT INTO btree_multi_test VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')"
    );
    if (insertRes.error) return { error: `INSERT failed: ${insertRes.error}` };

    const syncRes = await window.pgletBtreeWorker.sync();
    if (syncRes.error) return { error: `SYNC failed: ${syncRes.error}` };

    // Query with WHERE
    const queryRes = await window.pgletBtreeWorker.query("SELECT * FROM btree_multi_test WHERE id >= 2 ORDER BY id");
    if (queryRes.error) return { error: queryRes.error };

    return {
      rowCount: queryRes.result?.rowCount ?? 0,
      rows: queryRes.result?.rows ?? []
    };
  });

  if ("error" in result && result.error) {
    throw new Error(`Multi-row test failed: ${result.error}`);
  }

  if (result.rowCount !== 2) {
    throw new Error(`Expected 2 rows with id >= 2, got ${result.rowCount}`);
  }

  if (result.rows[0].name !== "Bob" || result.rows[1].name !== "Carol") {
    throw new Error(`Unexpected row data: ${JSON.stringify(result.rows)}`);
  }
}

async function testBtreeCommitDurability(page: Page): Promise<void> {
  await page.goto(`http://localhost:${HTTP_PORT}/`);
  await waitForBtreeOpfs(page);

  // Create table and sync (this saves metadata with schema)
  const setup = await page.evaluate(async () => {
    const dropRes = await window.pgletBtreeWorker.exec("DROP TABLE IF EXISTS btree_commit_test");
    if (dropRes.error) return { error: `DROP failed: ${dropRes.error}` };

    const createRes = await window.pgletBtreeWorker.exec("CREATE TABLE btree_commit_test (id INT, val TEXT)");
    if (createRes.error) return { error: `CREATE failed: ${createRes.error}` };

    // Sync to save schema/table metadata
    const syncRes = await window.pgletBtreeWorker.sync();
    if (syncRes.error) return { error: `SYNC failed: ${syncRes.error}` };

    return { success: true };
  });

  if (setup.error) {
    throw new Error(`Setup failed: ${setup.error}`);
  }

  // Insert data in an explicit transaction. COMMIT is the durable point:
  // it flips the alternate meta slot (double-meta crash safety, no WAL). We do
  // NOT call sync() afterward — the meta flip already made the commit durable.
  const inserted = await page.evaluate(async () => {
    const beginRes = await window.pgletBtreeWorker.exec("BEGIN");
    if (beginRes.error) return { error: `BEGIN failed: ${beginRes.error}` };

    const insertRes = await window.pgletBtreeWorker.exec(
      "INSERT INTO btree_commit_test VALUES (123, 'committed')"
    );
    if (insertRes.error) return { error: `INSERT failed: ${insertRes.error}` };

    // COMMIT writes the new pages, fsyncs them, then flips the alternate meta
    // slot and fsyncs again — the durable commit point.
    const commitRes = await window.pgletBtreeWorker.exec("COMMIT");
    if (commitRes.error) return { error: `COMMIT failed: ${commitRes.error}` };

    // Verify data is there before reload.
    const queryRes = await window.pgletBtreeWorker.query("SELECT * FROM btree_commit_test");
    if (queryRes.error) return { error: queryRes.error };

    // Note: NOT calling sync() — the meta flip on COMMIT is the durable point.
    return {
      success: true,
      rowCount: queryRes.result?.rowCount ?? 0
    };
  });

  if (inserted.error) {
    throw new Error(`Insert failed: ${inserted.error}`);
  }

  if (inserted.rowCount !== 1) {
    throw new Error(`Expected 1 row before reload, got ${inserted.rowCount}`);
  }

  // Reload page - simulates restart. The committed row must survive via the
  // meta-slot flip (double-meta provides crash safety, not WAL replay).
  await page.reload();
  await waitForBtreeOpfs(page);

  // Verify the committed data survived the reopen.
  const recovered = await page.evaluate(async () => {
    const queryRes = await window.pgletBtreeWorker.query("SELECT * FROM btree_commit_test");
    if (queryRes.error) return { error: queryRes.error };
    return queryRes.result;
  });

  if ("error" in recovered && recovered.error) {
    throw new Error(`Reopen query failed: ${recovered.error}`);
  }

  if (recovered.rowCount !== 1) {
    throw new Error(`Expected 1 row after reopen, got ${recovered.rowCount}`);
  }

  const row = recovered.rows[0];
  if (row.id !== 123 || row.val !== "committed") {
    throw new Error(`Unexpected data after reopen: ${JSON.stringify(row)}`);
  }
}

// =============================================================================
// Main
// =============================================================================

const tests: Array<{ name: string; fn: (page: Page) => Promise<void>; skip?: boolean }> = [
  { name: "wasm-loads", fn: testWasmLoads },
  { name: "create-table", fn: testCreateTable },
  { name: "query-with-where", fn: testQueryWithWhere },
  { name: "aggregates", fn: testAggregates },
  // B+tree OPFS tests
  { name: "btree-persistence", fn: testBtreePersistence },
  { name: "btree-index-reopen", fn: testBtreeIndexReopen },
  { name: "btree-transaction-rollback", fn: testBtreeTransactionRollback },
  { name: "btree-multiple-rows", fn: testBtreeMultipleRows },
  { name: "btree-commit-durability", fn: testBtreeCommitDurability },
];

async function main() {
  console.log("pglet Browser Tests");
  console.log("=".repeat(50));

  // Ensure both WASM builds are current: the page's in-memory tests load the default
  // build (the shipped shape), the OPFS worker the `opfs` feature's build, the only one
  // that exports `PgletBtreePersistent`.
  console.log("\n[browser] Checking WASM builds...");
  await ensureWasm(WASM_DEFAULT);
  await ensureWasm(WASM_OPFS);

  // Start HTTP server
  await startHttpServer();

  // Launch browser
  console.log(`\n[browser] Launching browser (headed: ${headed})...`);
  try {
    browser = await chromium.launch({ headless: !headed });
  } catch (err) {
    if (String(err).includes("Executable doesn't exist")) {
      console.error("\n[browser] Playwright browsers not installed.");
      console.error("[browser] Run: npx playwright install chromium");
      console.error("[browser] Or:  deno run -A npm:playwright install chromium\n");
      Deno.exit(1);
    }
    throw err;
  }
  const context = await browser.newContext();
  const page = await context.newPage();

  // Enable console logging from browser
  page.on("console", (msg: ConsoleMessage) => {
    if (msg.type() === "error") {
      console.error(`[browser console] ${msg.text()}`);
    }
  });

  // Run tests
  const results: TestResult[] = [];
  const filteredTests = filterPattern
    ? tests.filter((t) => t.name.includes(filterPattern))
    : tests;

  console.log(`\n[browser] Running ${filteredTests.length} tests...\n`);

  for (const test of filteredTests) {
    const encoder = new TextEncoder();
    await Deno.stdout.write(encoder.encode(`  ${test.name}... `));

    if (test.skip) {
      console.log(`\x1b[33mSKIP\x1b[0m (requires Web Worker)`);
      continue;
    }

    const result = await runTest(page, test.name, test.fn);
    results.push(result);

    if (result.passed) {
      console.log(`\x1b[32mPASS\x1b[0m (${result.duration}ms)`);
    } else {
      console.log(`\x1b[31mFAIL\x1b[0m (${result.duration}ms)`);
      console.log(`    Error: ${result.error}`);
    }
  }

  // Summary
  const passed = results.filter((r) => r.passed).length;
  const failed = results.filter((r) => !r.passed).length;

  console.log("\n" + "=".repeat(50));
  console.log(`Results: ${passed} passed, ${failed} failed`);

  // Cleanup
  await browser.close();
  await stopHttpServer();

  // Exit with error if any tests failed
  if (failed > 0) {
    Deno.exit(1);
  }
}

main().catch((err) => {
  console.error("Fatal error:", err);
  if (browser) browser.close();
  if (httpServer) stopHttpServer();
  Deno.exit(1);
});
