# pglet Browser Example

A working demonstration of pglet running entirely in the browser via WebAssembly.

## What This Demonstrates

pglet executing SQL entirely client-side, with no server in the path: `CREATE` / `INSERT`
/ `SELECT` / `UPDATE` / `DELETE`, `WHERE` filtering, `ORDER BY` and `LIMIT`, aggregates,
transactions (`BEGIN` / `COMMIT` / `ROLLBACK`, and `SAVEPOINT` / `ROLLBACK TO` / `RELEASE`
— a savepoint rolls back only its own level and recovers an aborted block), and data held
in browser memory or, on the persistent page, in OPFS across a reload. `test.ts` drives the
in-memory CRUD, filter, aggregate and ordering paths; a rollback and OPFS persistence across
a reload are covered by `deno task test:browser`, savepoints by `deno task test:wasm`.

## Storage Modes

The wasm bindings (`crates/pglet_wasm`) have two classes. **`PgletBtree`** — the
in-memory CoW B+tree, the same storage the native server runs, and the class the npm
package's `create_pglet_db` wraps (the package does not export it) — is in every build;
this example's editor page runs it from the default build (`pkg/`). **`PgletBtreePersistent`**
(OPFS-backed) is an opt-in layer: only a build with the crate's `opfs` feature on exports
it, and this example's persistent page loads that build (`pkg-opfs/`). The npm package
ships the default build, without it.

### In-Memory Mode (Default)

```javascript
// import path as used by this example's own files:
import init, { PgletBtree } from '../../crates/pglet_wasm/pkg/pglet_wasm.js';

await init();
const db = new PgletBtree();
db.exec("CREATE TABLE users (id INT, name TEXT)");
db.exec("INSERT INTO users VALUES (1, 'Alice')");
const result = db.query("SELECT * FROM users");
// result: { rows, rowCount, command, fields }
// Bound positional parameters:
const one = db.queryParams("SELECT * FROM users WHERE id = $1", [1]);
// A cheap CoW branch of committed state (in-memory only; throws inside an
// open transaction block) — the per-test-database primitive:
const branch = db.fork();
// db.transactionDepth() counts BEGIN…COMMIT plus one per live savepoint —
// needed because ROLLBACK TO SAVEPOINT reports tag ROLLBACK.
// Data is lost on page refresh
```

### Persistent Mode (Web Worker Required)

For persistent storage using OPFS (Origin Private File System), run pglet in a
Web Worker, from a build with the `opfs` feature on (`deno task wasm:build:opfs`):

```javascript
// worker.js — import path as used by this example's own worker:
import init, { PgletBtreePersistent } from '../../crates/pglet_wasm/pkg-opfs/pglet_wasm.js';

await init();
const db = await PgletBtreePersistent.open();
db.exec("CREATE TABLE users (id INT, name TEXT)");
db.exec("INSERT INTO users VALUES (1, 'Alice')");
db.sync(); // Ensure data is flushed to disk
// Data persists across page reloads!
```

**Why Web Worker?** The OPFS sync API (`FileSystemSyncAccessHandle`) is only
available in Web Workers. This allows for efficient synchronous file operations
without blocking the main thread.

## Running the Example

From the project root:

```bash
# Build the WASM packages (required first time): the default build for the
# editor page, the `opfs` feature's build for the persistent page
deno task wasm:build
deno task wasm:build:opfs

# Start the development server
deno task wasm:serve
```

Then open http://localhost:8001/examples/browser/ in your browser.

### Test persistence:

Open http://localhost:8001/examples/browser/persistent.html to test OPFS
persistence. This page uses a Web Worker and lets you verify data survives
page reloads.

### Run automated tests:

```bash
deno task wasm:test
```

## Files

- `index.html` - Interactive SQL editor UI (in-memory mode)
- `persistent.html` - Persistence test page (Web Worker + OPFS)
- `main.js` - JS code for in-memory mode
- `worker.js` - Web Worker for persistent mode (loads the `opfs` build)
- `serve.ts` - Simple HTTP server for local development
- `test.ts` - Automated test suite

## Limitations

- **Single instance**: Only one `PgletBtreePersistent` can be open at a time per origin.
  OPFS sync access handles are exclusive - opening a second instance will fail.
  This is fine for typical single-tab usage. (`PgletBtree` has no such limit,
  and `fork()` gives multiple independent in-memory instances; the persistent
  class does not fork.)

- **Web Worker required for persistence**: The OPFS sync API is only available in
  Web Workers. The in-memory `PgletBtree` works on the main thread.

- **B+tree persistence**: Uses CoW B+tree storage for efficient persistence
  with low write amplification.

## Binary Size

The release WASM build applies `opt-level = "z"` + LTO + symbol stripping +
`wasm-opt` (`-Oz`). The current gated figure is `benches/size_baseline.json`;
the budget and what has been cut to hold it are `TRADEOFFS.md`.

## Browser Compatibility

Requires modern browser with:
- WebAssembly support
- ES6 modules
- Bulk memory operations (standard in modern browsers)

For persistence mode:
- OPFS with synchronous access handles (`FileSystemSyncAccessHandle`)
- Must run in a Web Worker for sync access

The automated browser tests (`deno task test:browser`) run in Chromium under Playwright.
