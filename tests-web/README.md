# pglet TypeScript/Web Test Suite

Tests auto-spawn the pglet servers they need. The differential suites also need a
**disposable** PostgreSQL on port 5432: `deno task test` runs `test:fixtures`,
`test:fixtures:profile-lock`, `test:copy` and `test:dump`, which reset that database's `public`
schema, and `test:pg-dump`, which drops tables of fixed names in it — read `CONTRIBUTING.md`
before running it against a Postgres you keep.

## Quick Start

```bash
deno task test                 # The chained suites (Rust + TypeScript + Browser) — not every test:* task
deno task test:rust            # Rust unit tests only
deno task test:fixtures        # SQL fixture tests (in tests/fixtures/)
deno task test:web             # The driver/ORM/npm suites (spy-proxy parsers, integration, drizzle, kysely, postgres, pg, npm)
deno task test:integration     # Behavioral tests
deno task test:drizzle         # Drizzle ORM tests
deno task test:kysely          # Kysely ORM tests
deno task test:pg              # pg (node-postgres) driver tests
deno task test:postgres        # postgres.js driver tests
deno task test:browser         # Playwright browser tests (WASM + OPFS)
```

## Test Categories

| Category | Command | What it tests |
|----------|---------|---------------|
| Rust unit | `cargo test` | Core logic; some integration tests build and spawn a pglet server |
| SQL fixtures | `test:fixtures` | Query correctness vs real Postgres |
| Web/TS tests | `test:web` | All TypeScript driver/ORM/integration tests |
| Integration | `test:integration` | Connection behavior, sessions |
| Drizzle ORM | `test:drizzle` | ORM compatibility |
| Kysely ORM | `test:kysely` | ORM compatibility |
| pg driver | `test:pg` | node-postgres driver compatibility |
| postgres.js | `test:postgres` | postgres.js driver compatibility |
| npm adapter | `test:npm` | The `@fuzdev/pglet-wasm` adapter suite (coercion contract, shape, DX, kysely) + a Node smoke run — builds via `build:npm` first, needs Node |
| Browser/WASM | `test:browser` | WASM in real browser (the default build) + OPFS persistence (the `opfs` feature's build) |
| Profile lock | `test:fixtures:profile-lock` | The `@profile-lock` fixture subset against the stripped (`--no-default-features`) server |
| Server lifecycle | `test:integration:server` | Start/stop, port binding |

## Port Allocation

`tests-web/lib/ports.ts` is the TypeScript mirror of the Rust constants, and
`crates/pglet_tests/README.md` § Port Conventions is where the allocation is stated —
including the dynamic range a spawned server takes. What matters here: a test server never lands on the dev server's
port, and the browser harness's HTTP port is its own.

## Directory Structure

```
tests-web/              # TypeScript interface tests (this directory)
├── lib/                # Shared test utilities
│   ├── ports.ts        # Port constants (5432, 5433, 5434, 8001)
│   ├── server.ts       # ensureServer(), spawnServer(), checkHealth()
│   └── build.ts        # ensureWasm(), buildServer()
│
├── orms/               # ORM compatibility tests
│   ├── drizzle/        # Drizzle ORM tests (+ schema.ts)
│   └── kysely/         # Kysely ORM tests (+ types.ts, capture.ts)
│
├── drivers/            # Driver compatibility tests
│   ├── pg/             # node-postgres driver tests
│   └── postgres/       # postgres.js driver tests
│
├── integration/        # Behavioral tests (pglet_test.ts, server_test.ts, pglet_wasm_test.ts + its child scripts; see its README)
│
├── npm/                # @fuzdev/pglet-wasm adapter suite (coercion, dx, kysely, package, adapter)
│
└── browser/            # Playwright browser tests
    ├── run.ts          # Test runner
    ├── test.html       # Test page (loads WASM + spawns worker)
    └── test-worker-btree.js  # Web Worker for B+tree OPFS tests

tests/                  # Rust-related test data
├── captures/           # Wire protocol captures (documentation)
└── fixtures/           # SQL fixtures with expected outputs

crates/pglet_tests/      # Rust test binaries (see crates/pglet_tests/README.md)
└── src/
    ├── lib.rs          # Server management, connection helpers, JSON type mapping
    └── bin/            # test_fixtures (counts in tests/gates_baseline.txt), test_binary, test_keywords,
                        #   test_concurrent, test_copy, test_pg_dump, test_dump,
                        #   test_crash, test_power_loss, test_soak, run_sql
```

## How Self-Contained Tests Work

When you run `deno task test:fixtures`:

1. The Rust fixture runner builds the server binary from current source and
   **always spawns a fresh server** on a dynamically-chosen open port (5434+) —
   never reusing a running one, which could be a stale binary or carry leaked state
2. Validates expected files against Postgres (catches stale snapshots)
3. Runs queries against pglet and compares to expected
4. Server is killed on exit

The pglet side needs no setup; the Postgres side is the disposable oracle above.

## SQL Fixture Format

The fixture format and its directives are documented canonically in
[`tests/README.md`](../tests/README.md) — a fixture runs inside an isolating
`BEGIN … ROLLBACK` unless it opts out with `@no-txn`, so no `DROP TABLE IF EXISTS`
boilerplate is needed. Common commands:

```bash
deno task test:fixtures:update [pattern]                      # regenerate expected from Postgres
deno task test:fixtures <pattern>                             # run a subset (path filter)
cargo run -p pglet_tests --bin test_fixtures -- -v <pattern>  # verbose
```

## Browser Tests

Browser tests use Playwright to run WASM in a real Chromium browser.

**First-time setup:**

```bash
deno task test:browser:install  # Downloads Chromium (~200MB)
```

**Run tests:**

```bash
deno task test:browser          # Headless (CI)
deno task test:browser:headed   # Visible browser (debugging)
```

**What's tested**: WASM loads; CREATE/INSERT/SELECT with WHERE;
aggregates (COUNT, SUM); OPFS persistence (data survives reload); an index
surviving reopen; transaction rollback; multi-row inserts; commit durability.

### OPFS Architecture

OPFS persistence requires `createSyncAccessHandle()` which only works in Web
Workers (not the main thread). The test architecture handles this:

```
test.html
├── Main thread: Loads the default build (pkg/), exposes the in-memory PgletBtree class
└── Web Worker (test-worker-btree.js): Loads the `opfs` build (pkg-opfs/), runs PgletBtreePersistent
    └── window.pgletBtreeWorker.{exec, query, sync, clear}
```

`PgletBtreePersistent` is the `pglet_wasm` crate's opt-in `opfs` feature: the default
build — the npm package's shape — exports no such class, so the worker loads the build
`deno task wasm:build:opfs` makes; the runner builds either one that is missing or older
than the Rust sources (`tests-web/lib/build.ts`).

Tests call `window.pgletBtreeWorker.*` methods which message the worker. This works
in both headed and headless modes.

## Shared Test Utilities

`tests-web/lib/` holds the three a web test reaches for: `ports.ts` (the port constants),
`server.ts` (`checkHealth` / `ensureServer` / `spawnServer` — health-check, a fresh spawn
that refuses an occupied test port, with a cleanup function, and a handle-returning spawn) and `build.ts` (`buildServer` /
`ensureWasm`, each building only when the artifact is stale). Signatures live in the files;
they are short enough to read, and a copy here goes stale silently.

## Running All Tests

```bash
deno task test
```

This chains, in order: `test:rust` (cargo unit tests) → `test:fixtures` →
`test:fixtures:profile-lock` → `test:keywords` → `test:concurrent` → `test:copy` →
`test:pg-dump` → `test:dump` → `test:crash` → `test:web` (integration + ORMs +
drivers + npm) → `test:wasm` → `test:browser`. Opt-in and not chained: `test:binary`,
`test:integration:server`, `test:power-loss` (root-only) and `test:soak` / `test:soak:long`
(long-running).

## Parallel Execution

| Test Type | Parallel Safe | Notes |
|-----------|---------------|-------|
| Rust unit | Yes | Independent tests |
| SQL fixtures | No — one run per oracle | Each run resets the oracle's `public` schema before its first fixture; fixtures within a run are isolated by `BEGIN … ROLLBACK` |
| Integration | Yes | Unique table names per test |
| Browser | No | The tests run serially on one shared page/context (OPFS is one exclusive handle per origin) |

## Prerequisites

| Test Type | Prerequisites | Auto-Built |
|-----------|---------------|------------|
| Rust unit | None | N/A |
| SQL fixtures | Postgres on 5432 | pglet binary: Yes |
| Integration | None | pglet binary: Yes |
| Drizzle | None | pglet binary: Yes |
| npm adapter | Node (`test:npm` runs `node --test`) | npm package: Yes (`build:npm`) |
| Browser | Playwright Chromium | WASM package: Yes |

Most suites fail fast with a message naming a missing prerequisite; some skip and exit 0 instead — `test:copy`, `test:dump` and `test:pg-dump` without Postgres or its client tools, `test:power-loss` without its device.

## Manual Testing with psql

```bash
# Terminal 1: Run pglet server
deno task server

# Terminal 2: Test with psql
psql -h 127.0.0.1 -p 5433 -U postgres << 'EOF'
CREATE TABLE users (id INT, name TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob');
SELECT * FROM users ORDER BY name;
EOF
```

Use `127.0.0.1` (not `localhost`) to avoid IPv6 issues.

## See Also

- `tests/README.md` - The fixture format and its directives (canonical)
- `crates/pglet_tests/README.md` - Rust test binaries (binary format, fixtures)
- `CLAUDE.md` - Project overview and commands
