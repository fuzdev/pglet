# pglet_tests

Rust integration test utilities and binary format validation for pglet.

## The binaries

The test binaries live in `src/bin/`, each but the ad-hoc `run_sql` with a `deno task` wrapper
(see the repo `CLAUDE.md` § Commands): `test_fixtures` (the differential
harness; `--update`, patterns, `-v`, and `--profile-lock` — the stripped-build
regression lock with its two tag-completeness checks), `test_binary`
(documented below), `test_keywords`, `test_concurrent`, `test_copy`,
`test_pg_dump`, `test_dump`, `test_crash` (kill -9 + soak durability),
`test_power_loss` (root-only; skips without the dm-flakey device from
`tools/durability/power_loss_test.sh`; `PGLET_POWER_LOSS_N` sets the round
count), `test_soak` (opt-in large-dataset churn), and `run_sql` (ad-hoc;
autodiscovered rather than a `[[bin]]` stanza). `PGLET_NO_INDEX_ACCEL=1` makes
the harness build the spawned server `--no-default-features` (the stripped
profile) — it does not toggle this crate's own features.

## Test Binary

**Self-contained** binary format tests with tokio-postgres:

```bash
# Just run it - builds server, spawns on open port, tests, cleans up
cargo run -p pglet_tests --bin test_binary
```

**What it tests:**
- the scalar types in binary format (INT2–INT8, FLOAT4/8, BOOL, TEXT, BYTEA, TIMESTAMP/TZ, JSONB, UUID; VARCHAR shares TEXT's wire path). DATE, TIME, INTERVAL, and NUMERIC — real column types — are validated in binary via the differential fixtures (`tests/fixtures/types/date_*`, `time_*`, `interval_*`, `numeric_*`), not here.
- NULL value handling for all types
- Parameterized queries with binary parameters
- Extended query protocol (Parse/Bind/Execute)

**How it works:**
1. Runs `cargo build -p pglet_server`
2. Finds open port starting from 5434
3. Spawns pglet_server
4. Runs all tests
5. Kills server on exit

This design allows parallel test runs without port conflicts.

## Library Utilities

The `pglet_tests` library provides utilities for testing (the lower-level
spawn/kill substrate lives in the dev-only `pglet_testing` crate):

```rust
use pglet_tests::{connect, pglet_config, spawn_test_server};

// Build + spawn a fresh pglet server on a dynamically-chosen open port (5434+);
// killed on drop
let server = spawn_test_server().await?;

// Connect and run queries
let client = connect(&pglet_config(server.port)).await?;
client.simple_query("SELECT 1").await?;
```

## Port Conventions

| Constant | Port | Usage |
|----------|------|-------|
| `POSTGRES_PORT` | 5432 | Real Postgres, unless `PGLET_PG_URL` names another |
| `PGLET_TEST_PORT` | 5434+ | pglet test server (dynamic, finds an open port up to `DYNAMIC_PORT_END` = 5500) |

Dev server runs on 5433 via `deno task server`.

## Environment

| Variable | Default | Effect |
|----------|---------|--------|
| `PGLET_PG_URL` | unset | Names the Postgres oracle — a libpq `postgres://` URL or a `key=value` conninfo string. Unset means `host=127.0.0.1 port=5432 user=postgres dbname=postgres`, no password. A malformed value fails loud naming the variable rather than falling back to that default. |
| `PGLET_NO_INDEX_ACCEL` | unset | Set to anything (the check is presence, not value) to make the harness build the spawned server `--no-default-features` — the stripped profile. |
| `PGLET_POWER_LOSS_N` | 200 | Acked-commit count for `test_power_loss`'s commit phase (`--n` also sets it). |

**What this variable points at gets its `public` schema dropped.** `test_fixtures`,
`test_dump` and `test_copy` each issue `DROP SCHEMA public CASCADE; CREATE SCHEMA public`
against the database they connect to, before every run — so an override names a database
you are willing to lose, and the contributor-facing form of that warning is
`CONTRIBUTING.md` § Before you run the tests.

`PGLET_PG_URL` is the one oracle knob, and the reach is worth stating per ring
rather than generalizing over "the harness":

- **Routed.** The `tokio-postgres` clients (`connect` / `check_health`, hence every
  Postgres gate), the hand-rolled `SimpleProbe` that reads the simple-query
  declared types — which needs the fields split back out of the URL — and the two
  oracle-targeting `psql` / `pg_dump` shell-outs, which take the conninfo
  **positionally** (libpq ranks a positional conninfo above the `PG*` environment,
  so an inherited `PGHOST` cannot redirect them). Every value is rendered through
  libpq's `key = value` quoting rules — quoted when empty or when it holds any
  Unicode whitespace, `'` or `\` (wider than libpq's ASCII `isspace`, and safe in
  that direction), bare otherwise, so the default target's conninfo is unchanged —
  and a password or a database name holding a space therefore reaches each
  consumer as the value that was parsed in. An *unquoted* empty value would not
  merely truncate: libpq reads the next keyword as the value, so `password=
  dbname=x` lands on the database libpq defaults to, the user's name.
- **Deliberately not routed.** The pglet servers the tests spawn — a dynamically
  chosen loopback port, any credentials accepted; pointing them elsewhere would
  compare the oracle against someone else's server. And `cargo xtask measure`,
  whose quiet-box measurement set creates and drops its own scratch database
  against `127.0.0.1:5432`: `xtask` depends on `pglet_core` alone, so routing it
  would mean a second copy of the parse rather than one reader.
- **A second URL knob, and not the oracle's.** `benches/exit_proof.ts` reads
  `PGLET_EXITPROOF_PG_URL` — opt-in, naming the real Postgres its
  production-ceiling leg runs against, whose database it drops and creates. Its
  one other appearance is `cargo xtask measure`, which *sets* it to that run's own
  scratch database for the exit-proof step; no Rust code reads it (`measure.rs`
  only writes it), and the two knobs never resolve each other.
- **Not reached at all, and not the harness's.** `tools/spy_proxy`'s forward target
  (an argument), and `deno.json`'s `--allow-net=127.0.0.1:5432` literals on the
  `spy:*` tasks, which are Deno *sandbox* permissions: an override pointing
  elsewhere is refused by the sandbox, a different failure from a connection error.

The override moves the host, the port and the cluster freely. Two things it does not move
freely — the session identity (fixtures that snapshot `postgres`/`postgres`; `--update`
refused when either name moves) and a password, which `SimpleProbe` does not speak — are
`CONTRIBUTING.md` § Before you run the tests and § The Postgres oracle respectively.

`.github/workflows/check.yml` runs the three tiers a contributor can — hermetic
(`deno task check` + `deno task test:rust`), package (`test:npm`,
`size:check` and the `opfs` build's export check), differential (the Postgres gates under `cargo xtask gates
--check`, against a `postgres:18.6` service with `POSTGRES_HOST_AUTH_METHOD: trust`
— the probe's ceiling — named by `PGLET_PG_URL`). It runs the `deno task` bodies
rather than re-enumerating them, and the differential job both installs the
`psql` / `pg_dump` clients explicitly and relies on `--check`'s missing-counter
rule, so neither a missing client nor the gate skip it causes can pass for a green
run. The service's default user and database names are what the job names, because
moving either costs the two session-identity fixtures above.

## SQL Fixture Tests

**Self-contained** fixture tests comparing pglet output to Postgres:

```bash
cargo run -p pglet_tests --bin test_fixtures              # Run all fixtures
cargo run -p pglet_tests --bin test_fixtures -- --update  # Regenerate expected
cargo run -p pglet_tests --bin test_fixtures -- select/   # Filter by pattern
cargo run -p pglet_tests --bin test_fixtures -- -v        # Verbose mode
cargo run -p pglet_tests --bin test_fixtures -- --profile-lock  # @profile-lock subset (stripped server)
```

**What it does:**
1. Builds pglet_server if needed
2. Validates expected files against Postgres (catches stale snapshots)
3. Runs fixtures against pglet and compares to expected
4. Supports `@expect-error` directive for error case testing

**Protocol:**
- Setup statements run over the simple query protocol (multi-statement)
- The `@test` runs in both wire formats — extended-protocol binary against
  `.expected.json`, simple-query text live against Postgres — unless `@text` / `@param`
  narrows it to one (`tests/README.md` § Fixtures)
- Type-aware JSON conversion matches node-postgres behavior

**JSON Type Mapping:**

| SQL Type | JSON Format | Why |
|----------|-------------|-----|
| INT2, INT4 | Number | Fits in JS Number |
| INT8 (BIGINT) | String | JS can't represent integers > 2^53 precisely |
| FLOAT4, FLOAT8 | Number | IEEE 754 |
| BOOL | Boolean | Native JSON |
| TEXT, VARCHAR (also BPCHAR, NAME) | String | Native JSON |
| TIMESTAMP | String | no-zone text, millisecond-truncated |
| TIMESTAMPTZ | String | RFC3339 (a distinct decode arm from TIMESTAMP) |
| DATE | String | `YYYY-MM-DD` (explicit decode arm — the generic fallback would null it) |
| TIME | String | `HH:MM:SS[.ffffff]` via a raw-µs shim (`24:00:00` representable) |
| INTERVAL | String | `postgres`-style text, decoded from the 16-byte wire form |
| NUMERIC | String | precision-preserving (node-postgres does the same) |
| JSONB, JSON | Object/Array | Native JSON |
| UUID | String | Hyphenated format |
| BYTEA | Object | `{"type": "Buffer", "data": [...]}` |
| Arrays | Array | element-decoded (int8[]/uuid[] elements as strings) |
| NULL | null | Native JSON |

This matches node-postgres behavior, ensuring compatibility with TypeScript ORM tests.

## See Also

- `tests/README.md` - Fixture format documentation
- `tests-web/README.md` - Full test architecture
- `PLAN.md` - roadmap; `docs/compatibility.md` - per-feature status; `EDGES.md` - the known-divergence ledger
